use std::fs::File;
use std::io::{Read, Write};
use std::os::fd::AsRawFd;
use std::sync::Arc;

use alacritty_terminal::event::{Event, OnResize, WindowSize};
use alacritty_terminal::sync::FairMutex;
use alacritty_terminal::tty;
use alacritty_terminal::vte::ansi::Processor;
use winit::event_loop::EventLoopProxy;

use crate::term::Term;

pub struct Pty {
    // Dropping this hangs up the shell.
    pty: tty::Pty,
    writer: File,
}

impl Pty {
    pub fn spawn(term: Arc<FairMutex<Term>>, size: WindowSize, proxy: EventLoopProxy<Event>) -> Self {
        let pty = tty::new(&tty::Options::default(), size, 0).expect("spawn shell");
        let writer = pty.file().try_clone().expect("clone pty fd");
        let mut reader = pty.file().try_clone().expect("clone pty fd");
        // alacritty sets O_NONBLOCK for its mio loop; we use a plain blocking thread.
        // ponytail: blocking writes too, since flags are per open file. Fine until a paste stalls the UI.
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
                    let _ = proxy.send_event(Event::Wakeup);
                }
                let _ = proxy.send_event(Event::Exit);
            })
            .expect("spawn reader thread");

        Self { pty, writer }
    }

    pub fn write(&mut self, bytes: &[u8]) {
        if let Err(e) = self.writer.write_all(bytes) {
            log::warn!("pty write failed: {e}");
        }
    }

    pub fn resize(&mut self, size: WindowSize) {
        self.pty.on_resize(size);
    }
}
