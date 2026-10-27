use std::fs::File;
use std::io::{Read, Write};
use std::os::fd::AsRawFd;
use std::os::unix::process::ExitStatusExt;
use std::path::PathBuf;
use std::process::ExitStatus;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::{self, Sender};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use alacritty_terminal::event::{Event, OnResize, WindowSize};
use alacritty_terminal::sync::FairMutex;
use alacritty_terminal::tty;
use alacritty_terminal::vte::ansi::Processor;

use crate::images::{self, Images};
use crate::tap::{Mark, Tap};
use crate::term::{Listener, Term, UserEvent};

/// Hyperlink id prefix (and URI) marking prompt cells; the id ends in the prompt's number.
pub const PROMPT_TAG: &str = "noobsbot-prompt-"; // no `:`, it separates OSC 8 params

/// How long the reader may wait for output before a synchronized update (DEC 2026)
/// times out; `None` outside one. An app that begins one and dies before ending it
/// would otherwise leave the pane frozen until its next output.
fn sync_wait(parser: &Processor) -> Option<Duration> {
    parser.sync_timeout().sync_timeout().map(|at| at.saturating_duration_since(Instant::now()))
}

/// The synchronized update is past its deadline: show what it buffered.
fn sync_expired(parser: &Processor) -> bool {
    parser.sync_timeout().sync_timeout().is_some_and(|at| at <= Instant::now())
}

/// How the shell (or `-e` program) exited, once its output is done: it may take a
/// moment longer to be gone (up to 100 ms, given its pidfd). Left for `Pty`'s drop to reap.
fn exit_status(pid: u32, pidfd: i32) -> Option<ExitStatus> {
    if pidfd >= 0 {
        let mut fd = libc::pollfd { fd: pidfd, events: libc::POLLIN, revents: 0 };
        unsafe { libc::poll(&mut fd, 1, 100) };
    }
    let mut info: libc::siginfo_t = unsafe { std::mem::zeroed() };
    let found = unsafe { libc::waitid(libc::P_PID, pid, &mut info, libc::WEXITED | libc::WNOHANG | libc::WNOWAIT) } == 0;
    if !found || unsafe { info.si_pid() } == 0 {
        return None;
    }
    // Back to a wait status: the code in the second byte, or the signal in the first.
    let status = unsafe { info.si_status() };
    Some(ExitStatus::from_raw(if info.si_code == libc::CLD_EXITED { (status & 0xff) << 8 } else { status & 0x7f }))
}

pub struct Pty {
    // Dropping this hangs up the shell.
    pty: tty::Pty,
    writer: Sender<Vec<u8>>,
    /// A wakeup is on its way to the app: the reader sends no more until the app lets
    /// it, once a frame has passed (`about_to_wait`).
    pub woken: Arc<AtomicBool>,
}

impl Pty {
    pub fn spawn(
        term: Arc<FairMutex<Term>>,
        images: Arc<Mutex<Images>>,
        size: WindowSize,
        listener: Listener,
        options: &tty::Options,
    ) -> std::io::Result<Self> {
        let Listener { proxy, pane } = listener;
        let pty = tty::new(options, size, 0)?;
        let mut file: File = pty.file().try_clone()?;
        let mut reader = pty.file().try_clone()?;
        // alacritty sets O_NONBLOCK for its mio loop; we use a plain blocking thread.
        // That makes writes blocking too (flags are per open file), so they get their own thread:
        // a big paste then never stalls the UI while the shell is busy.
        unsafe {
            let fd = reader.as_raw_fd();
            libc::fcntl(fd, libc::F_SETFL, libc::fcntl(fd, libc::F_GETFL) & !libc::O_NONBLOCK);
        }

        // The shell's exit, seen even while a background job it started keeps
        // the terminal open (`sleep 600 &`, then `exit`). -1 on kernels before 5.3.
        let pid = pty.child().id();
        let shell_exit = unsafe { libc::syscall(libc::SYS_pidfd_open, pid, 0) } as i32;
        let woken = Arc::new(AtomicBool::new(false));
        let woken_reader = woken.clone();
        std::thread::Builder::new()
            .name("pty-reader".into())
            .spawn(move || {
                let mut parser: Processor = Processor::new();
                let mut buf = vec![0u8; 64 * 1024];
                let mut tap = Tap::default();
                let mut marks = Vec::new();
                let mut prompt = 0;
                loop {
                    let mut fds = [
                        libc::pollfd { fd: reader.as_raw_fd(), events: libc::POLLIN, revents: 0 },
                        libc::pollfd { fd: shell_exit, events: libc::POLLIN, revents: 0 },
                    ];
                    // During a synchronized update, wait no longer than its deadline.
                    let wait = sync_wait(&parser).map_or(-1, |d| d.as_micros().div_ceil(1000).min(i32::MAX as u128) as i32);
                    unsafe { libc::poll(fds.as_mut_ptr(), if shell_exit < 0 { 1 } else { 2 }, wait) };
                    if fds[1].revents != 0 && fds[0].revents & libc::POLLIN == 0 {
                        break; // the shell is gone and so is its output
                    }
                    if fds[0].revents == 0 {
                        if sync_expired(&parser) {
                            parser.stop_sync(&mut *term.lock());
                            if !woken_reader.swap(true, Ordering::AcqRel) {
                                let _ = proxy.send_event(UserEvent::Term(pane, Event::Wakeup));
                            }
                        }
                        continue; // interrupted, or the update timed out
                    }
                    let n = match reader.read(&mut buf) {
                        Ok(0) | Err(_) => break, // EIO once the shell exits
                        Ok(n) => n,
                    };
                    tap.scan(&buf[..n], &mut marks);
                    let mut term = term.lock();
                    let mut done = 0;
                    for (end, mark) in marks.drain(..) {
                        parser.advance(&mut *term, &buf[done..end]);
                        done = end;
                        match mark {
                            // Tag the prompt's cells with a hidden hyperlink: the tag moves
                            // with the text through scrolling and reflow, and is gone when
                            // the text is, so blocks can be found on the grid later.
                            Mark::PromptStart => {
                                prompt += 1;
                                parser.advance(&mut *term, format!("\x1b]8;id={PROMPT_TAG}{prompt};{PROMPT_TAG}\x1b\\").as_bytes());
                            }
                            Mark::PromptEnd => parser.advance(&mut *term, b"\x1b]8;;\x1b\\"),
                            // Images get a block of blank cells tagged the same way.
                            Mark::Graphics(_) | Mark::Sixel(_) => {
                                let mut images = images.lock().unwrap();
                                let col = images::cursor_col(&term);
                                let (place, reply) = match &mark {
                                    Mark::Graphics(cmd) => images.kitty(cmd, col),
                                    Mark::Sixel(dcs) => (images.sixel(dcs, col), None),
                                    _ => unreachable!(),
                                };
                                if let Some(reply) = reply {
                                    let _ = proxy.send_event(UserEvent::Term(pane, Event::PtyWrite(reply)));
                                }
                                if let Some(place) = place {
                                    let room = images::room(&term, &place);
                                    parser.advance(&mut *term, room.as_bytes());
                                    images.evict(&term);
                                }
                            }
                            // Name and version, so apps go on to ask about features (Claude Code then
                            // checks DEC 2026 and uses synchronized output instead of flickering).
                            Mark::XtVersion => {
                                let reply = format!("\x1bP>|noobsbot {}\x1b\\", env!("CARGO_PKG_VERSION"));
                                let _ = proxy.send_event(UserEvent::Term(pane, Event::PtyWrite(reply)));
                            }
                            mark => {
                                let _ = proxy.send_event(UserEvent::Shell(pane, prompt, mark));
                            }
                        }
                    }
                    parser.advance(&mut *term, &buf[done..n]);
                    drop(term);
                    if !woken_reader.swap(true, Ordering::AcqRel) {
                        let _ = proxy.send_event(UserEvent::Term(pane, Event::Wakeup));
                    }
                }
                let status = exit_status(pid, shell_exit);
                if shell_exit >= 0 {
                    unsafe { libc::close(shell_exit) };
                }
                let _ = proxy.send_event(UserEvent::Term(pane, status.map_or(Event::Exit, Event::ChildExit)));
            })
            .expect("spawn reader thread");

        let (writer, rx) = mpsc::channel::<Vec<u8>>();
        std::thread::Builder::new()
            .name("pty-writer".into())
            .spawn(move || {
                for bytes in rx {
                    if let Err(e) = file.write_all(&bytes) {
                        log::warn!("pty write failed: {e}");
                        break;
                    }
                }
            })
            .expect("spawn writer thread");

        Ok(Self { pty, writer, woken })
    }

    pub fn write(&mut self, bytes: &[u8]) {
        let _ = self.writer.send(bytes.to_vec());
    }

    pub fn fd(&self) -> std::os::fd::RawFd {
        self.pty.file().as_raw_fd()
    }

    pub fn shell_pid(&self) -> u32 {
        self.pty.child().id()
    }

    /// The shell's working directory, for opening a new pane next to it.
    pub fn shell_cwd(&self) -> Option<PathBuf> {
        std::fs::read_link(format!("/proc/{}/cwd", self.shell_pid())).ok()
    }

    pub fn resize(&mut self, size: WindowSize) {
        self.pty.on_resize(size);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use alacritty_terminal::event::VoidListener;
    use alacritty_terminal::index::{Column, Line};
    use alacritty_terminal::term::Config;
    use alacritty_terminal::term::test::TermSize;

    #[test]
    fn exit_status_is_read_and_left_to_reap() {
        let child = |script: &str| std::process::Command::new("sh").args(["-c", script]).spawn().unwrap();
        for (script, code, signal) in [("exit 3", Some(3), None), ("true", Some(0), None), ("kill -9 $$", None, Some(9))] {
            let mut c = child(script);
            let pidfd = unsafe { libc::syscall(libc::SYS_pidfd_open, c.id(), 0) } as i32;
            let status = exit_status(c.id(), pidfd).expect(script);
            assert_eq!((status.code(), status.signal()), (code, signal), "{script}");
            assert_eq!(c.wait().unwrap(), status, "still there to reap");
            unsafe { libc::close(pidfd) };
        }
        let mut sleeping = child("sleep 5");
        assert_eq!(exit_status(sleeping.id(), -1), None);
        sleeping.kill().unwrap();
        sleeping.wait().unwrap();
    }

    #[test]
    fn unfinished_sync_update_times_out() {
        let mut term = alacritty_terminal::Term::new(Config::default(), &TermSize::new(20, 5), VoidListener);
        let mut parser: Processor = Processor::new();
        assert_eq!(sync_wait(&parser), None); // outside an update the reader may block
        // An app begins an update, draws, and dies before ending it.
        parser.advance(&mut term, b"\x1b[?2026hhi");
        assert_eq!(term.grid()[Line(0)][Column(0)].c, ' ', "held back until the update ends");
        let wait = sync_wait(&parser).expect("a deadline while the update is open");
        assert!(wait <= Duration::from_millis(150));
        assert!(!sync_expired(&parser));
        std::thread::sleep(wait + Duration::from_millis(5));
        assert!(sync_expired(&parser));
        parser.stop_sync(&mut term);
        assert_eq!(term.grid()[Line(0)][Column(0)].c, 'h');
        assert_eq!(sync_wait(&parser), None);
    }
}
