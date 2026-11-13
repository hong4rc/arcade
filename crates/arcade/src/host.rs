//! The host terminal: raw mode, its size, and getting it back the way it was.

use std::io;
use std::os::fd::RawFd;
use std::sync::atomic::{AtomicI32, Ordering};

/// What to write to the host terminal when the client leaves, whatever the way it leaves:
/// show the cursor, restore its shape and the colors, end synchronized output, and go back to the
/// normal screen.
pub const RESTORE_SEQUENCE: &[u8] = b"\x1b[0m\x1b[0 q\x1b[?25h\x1b[?2026l\x1b[?1049l";

/// Switch to the alternate screen, where the multiplexer draws.
pub const ENTER_SEQUENCE: &[u8] = b"\x1b[?1049h\x1b[0m\x1b[H\x1b[2J";

pub fn is_tty(fd: RawFd) -> bool {
    // SAFETY: isatty only inspects the descriptor number.
    unsafe { libc::isatty(fd) == 1 }
}

/// Columns and rows of the terminal on `fd`; 80 by 24 when it has none.
pub fn window_size(fd: RawFd) -> (u16, u16) {
    let mut ws = libc::winsize {
        ws_row: 0,
        ws_col: 0,
        ws_xpixel: 0,
        ws_ypixel: 0,
    };
    // SAFETY: TIOCGWINSZ writes a winsize struct, which points to a live local.
    let rc = unsafe { libc::ioctl(fd, libc::TIOCGWINSZ as _, &mut ws) };
    if rc < 0 || ws.ws_col == 0 || ws.ws_row == 0 {
        (80, 24)
    } else {
        (ws.ws_col, ws.ws_row)
    }
}

/// Puts the terminal on `fd` in raw mode and puts it back when dropped.
pub struct RawMode {
    fd: RawFd,
    saved: libc::termios,
}

impl RawMode {
    pub fn enter(fd: RawFd) -> io::Result<RawMode> {
        // SAFETY: tcgetattr fills the termios struct we hand it.
        let mut saved: libc::termios = unsafe { std::mem::zeroed() };
        // SAFETY: as above; saved is a live local.
        if unsafe { libc::tcgetattr(fd, &mut saved) } != 0 {
            return Err(io::Error::last_os_error());
        }
        let mut raw = saved;
        // SAFETY: cfmakeraw only edits the struct it is given.
        unsafe { libc::cfmakeraw(&mut raw) };
        // SAFETY: tcsetattr reads the struct, which is a live local.
        if unsafe { libc::tcsetattr(fd, libc::TCSANOW, &raw) } != 0 {
            return Err(io::Error::last_os_error());
        }
        Ok(RawMode { fd, saved })
    }
}

impl Drop for RawMode {
    fn drop(&mut self) {
        // SAFETY: tcsetattr reads the struct, which we own; failure leaves nothing to do.
        unsafe {
            libc::tcsetattr(self.fd, libc::TCSADRAIN, &self.saved);
        }
    }
}

// ---- Signals ----
//
// A signal handler may only do very little, so each one writes its number to a pipe, and the main
// loop reads the pipe along with the keyboard and the socket.

static SIGNAL_PIPE_WRITE: AtomicI32 = AtomicI32::new(-1);

extern "C" fn on_signal(signal: libc::c_int) {
    let fd = SIGNAL_PIPE_WRITE.load(Ordering::Relaxed);
    if fd >= 0 {
        let byte = signal as u8;
        // SAFETY: write is async-signal-safe; the buffer is a live local.
        unsafe {
            libc::write(fd, (&raw const byte).cast(), 1);
        }
    }
}

pub struct Signals {
    pub read_fd: RawFd,
    write_fd: RawFd,
}

impl Signals {
    pub fn install() -> io::Result<Signals> {
        let mut fds = [0; 2];
        // SAFETY: pipe fills the two-element array.
        if unsafe { libc::pipe(fds.as_mut_ptr()) } != 0 {
            return Err(io::Error::last_os_error());
        }
        for fd in fds {
            // SAFETY: fcntl on descriptors we just made.
            unsafe {
                let flags = libc::fcntl(fd, libc::F_GETFL);
                libc::fcntl(fd, libc::F_SETFL, flags | libc::O_NONBLOCK);
                libc::fcntl(fd, libc::F_SETFD, libc::FD_CLOEXEC);
            }
        }
        SIGNAL_PIPE_WRITE.store(fds[1], Ordering::Relaxed);
        for signal in [libc::SIGWINCH, libc::SIGTERM, libc::SIGHUP, libc::SIGINT] {
            // SAFETY: the handler only calls write; sigaction gets a fully set up struct.
            unsafe {
                let mut action: libc::sigaction = std::mem::zeroed();
                action.sa_sigaction = on_signal as extern "C" fn(libc::c_int) as usize;
                action.sa_flags = libc::SA_RESTART;
                libc::sigemptyset(&mut action.sa_mask);
                libc::sigaction(signal, &action, std::ptr::null_mut());
            }
        }
        Ok(Signals {
            read_fd: fds[0],
            write_fd: fds[1],
        })
    }

    /// The signals that arrived since the last call.
    pub fn take(&self) -> Vec<i32> {
        let mut out = Vec::new();
        let mut buf = [0u8; 64];
        loop {
            // SAFETY: read into a live local buffer of the stated length.
            let n = unsafe { libc::read(self.read_fd, buf.as_mut_ptr().cast(), buf.len()) };
            if n <= 0 {
                break;
            }
            out.extend(buf[..n as usize].iter().map(|b| i32::from(*b)));
        }
        out
    }
}

impl Drop for Signals {
    fn drop(&mut self) {
        SIGNAL_PIPE_WRITE.store(-1, Ordering::Relaxed);
        for signal in [libc::SIGWINCH, libc::SIGTERM, libc::SIGHUP, libc::SIGINT] {
            // SAFETY: back to the default action.
            unsafe {
                libc::signal(signal, libc::SIG_DFL);
            }
        }
        // SAFETY: closing descriptors we own.
        unsafe {
            libc::close(self.read_fd);
            libc::close(self.write_fd);
        }
    }
}
