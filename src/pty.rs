use std::fs::File;
use std::io::{Read, Write};
use std::os::fd::AsRawFd;
use std::path::PathBuf;
use std::sync::Arc;
use std::sync::mpsc::{self, Sender};

use alacritty_terminal::event::{Event, OnResize, WindowSize};
use alacritty_terminal::sync::FairMutex;
use alacritty_terminal::tty;
use alacritty_terminal::vte::ansi::Processor;

use crate::term::{Listener, Term, UserEvent};

pub struct Pty {
    // Dropping this hangs up the shell.
    pty: tty::Pty,
    writer: Sender<Vec<u8>>,
}

impl Pty {
    pub fn spawn(term: Arc<FairMutex<Term>>, size: WindowSize, listener: Listener, options: &tty::Options) -> std::io::Result<Self> {
        let Listener { proxy, pane } = listener;
        let pty = tty::new(options, size, 0)?;
        let mut file: File = pty.file().try_clone().expect("clone pty fd");
        let mut reader = pty.file().try_clone().expect("clone pty fd");
        // alacritty sets O_NONBLOCK for its mio loop; we use a plain blocking thread.
        // That makes writes blocking too (flags are per open file), so they get their own thread:
        // a big paste then never stalls the UI while the shell is busy.
        unsafe {
            let fd = reader.as_raw_fd();
            libc::fcntl(fd, libc::F_SETFL, libc::fcntl(fd, libc::F_GETFL) & !libc::O_NONBLOCK);
        }

        std::thread::Builder::new()
            .name("pty-reader".into())
            .spawn(move || {
                let mut parser: Processor = Processor::new();
                let mut buf = vec![0u8; 64 * 1024];
                loop {
                    let n = match reader.read(&mut buf) {
                        Ok(0) | Err(_) => break, // EIO once the shell exits
                        Ok(n) => n,
                    };
                    parser.advance(&mut *term.lock(), &buf[..n]);
                    let _ = proxy.send_event(UserEvent::Term(pane, Event::Wakeup));
                }
                let _ = proxy.send_event(UserEvent::Term(pane, Event::Exit));
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

        Ok(Self { pty, writer })
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
