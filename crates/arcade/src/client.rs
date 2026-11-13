//! The attached client: the keyboard goes up, the screen comes down.
//!
//! The client does four things: it puts the host terminal in raw mode, passes the bytes from the
//! keyboard to the daemon untouched, writes the bytes the daemon sends to the host terminal
//! untouched, and reports the terminal's size when it changes.

use std::io::Read;
use std::os::fd::AsRawFd;

use arcade_proto::{AttachRequest, ClientMessage, ServerMessage};

use crate::connection::Connection;
use crate::host::{ENTER_SEQUENCE, RESTORE_SEQUENCE, RawMode, Signals, window_size};

pub enum Outcome {
    /// Left on purpose or because the session ended; the text is a line for the user.
    Left(String),
    /// The server went away or sent something wrong.
    Lost(String),
    /// The daemon refused the request.
    Refused(String),
}

/// Ask the daemon to attach, then run until we are detached. Returns the exit code.
pub fn attach(mut connection: Connection, mut request: AttachRequest) -> i32 {
    let stdin_fd = 0;
    let stdout_fd = 1;
    let (cols, rows) = window_size(stdout_fd);
    request.cols = cols;
    request.rows = rows;
    if let Err(e) = connection.send(&ClientMessage::Attach(request)) {
        eprintln!("arcade: {e}");
        return 1;
    }
    // The daemon answers either with Attached (frames follow) or with a refusal.
    loop {
        match connection.receive() {
            Ok(Some(ServerMessage::Attached { .. })) => break,
            Ok(Some(ServerMessage::CommandResult { ok: false, output })) => {
                eprintln!("arcade: {output}");
                return 1;
            }
            Ok(Some(_)) => {}
            Ok(None) => {
                eprintln!("arcade: the server closed the connection");
                return 1;
            }
            Err(e) => {
                eprintln!("arcade: {e}");
                return 1;
            }
        }
    }

    let signals = match Signals::install() {
        Ok(s) => s,
        Err(e) => {
            eprintln!("arcade: cannot set up signals: {e}");
            return 1;
        }
    };
    let raw = match RawMode::enter(stdin_fd) {
        Ok(raw) => raw,
        Err(e) => {
            eprintln!("arcade: cannot set the terminal to raw mode: {e}");
            return 1;
        }
    };
    let _ = write_all(stdout_fd, ENTER_SEQUENCE);
    let outcome = run(&mut connection, &signals, stdin_fd, stdout_fd);
    // Back to the terminal the way it was, before saying anything.
    let _ = write_all(stdout_fd, RESTORE_SEQUENCE);
    drop(raw);
    drop(signals);
    match outcome {
        Outcome::Left(line) => {
            println!("{line}");
            0
        }
        Outcome::Lost(line) => {
            eprintln!("{line}");
            1
        }
        Outcome::Refused(line) => {
            eprintln!("arcade: {line}");
            1
        }
    }
}

fn write_all(fd: i32, mut bytes: &[u8]) -> std::io::Result<()> {
    while !bytes.is_empty() {
        // SAFETY: write from a live slice of the stated length.
        let n = unsafe { libc::write(fd, bytes.as_ptr().cast(), bytes.len()) };
        if n < 0 {
            let e = std::io::Error::last_os_error();
            match e.kind() {
                std::io::ErrorKind::Interrupted => continue,
                std::io::ErrorKind::WouldBlock => {
                    wait_writable(fd);
                    continue;
                }
                _ => return Err(e),
            }
        }
        bytes = &bytes[n as usize..];
    }
    Ok(())
}

fn wait_writable(fd: i32) {
    let mut pfd = libc::pollfd {
        fd,
        events: libc::POLLOUT,
        revents: 0,
    };
    // SAFETY: poll on one live pollfd.
    unsafe {
        libc::poll(&mut pfd, 1, -1);
    }
}

fn run(connection: &mut Connection, signals: &Signals, stdin_fd: i32, stdout_fd: i32) -> Outcome {
    let socket_fd = connection.stream.as_raw_fd();
    let mut key_buf = vec![0u8; 16 * 1024];
    let mut net_buf = vec![0u8; 64 * 1024];
    // Messages that came in the same read as `Attached` are already in the decoder; the socket
    // will not report them again, so they are handled before waiting.
    if let Some(outcome) = handle_messages(connection, stdout_fd) {
        return outcome;
    }
    loop {
        let mut fds = [
            libc::pollfd {
                fd: stdin_fd,
                events: libc::POLLIN,
                revents: 0,
            },
            libc::pollfd {
                fd: socket_fd,
                events: libc::POLLIN,
                revents: 0,
            },
            libc::pollfd {
                fd: signals.read_fd,
                events: libc::POLLIN,
                revents: 0,
            },
        ];
        // SAFETY: poll on three live pollfds, waiting with no timeout.
        let rc = unsafe { libc::poll(fds.as_mut_ptr(), 3, -1) };
        if rc < 0 {
            if std::io::Error::last_os_error().kind() == std::io::ErrorKind::Interrupted {
                continue;
            }
            return Outcome::Lost("[lost terminal]".to_string());
        }

        // Signals first: a hangup or termination ends the client at once (the session lives on).
        if fds[2].revents != 0 {
            for signal in signals.take() {
                match signal {
                    libc::SIGWINCH => {
                        let (cols, rows) = window_size(stdout_fd);
                        if connection
                            .send(&ClientMessage::Resize { cols, rows })
                            .is_err()
                        {
                            return Outcome::Lost("[lost server]".to_string());
                        }
                    }
                    _ => {
                        let _ = connection.send(&ClientMessage::Detach);
                        return Outcome::Left("[detached: terminal closed]".to_string());
                    }
                }
            }
        }

        // The keyboard.
        if fds[0].revents & (libc::POLLIN | libc::POLLHUP | libc::POLLERR) != 0 {
            // SAFETY: read into a live buffer of the stated length.
            let n = unsafe { libc::read(stdin_fd, key_buf.as_mut_ptr().cast(), key_buf.len()) };
            if n == 0 {
                return Outcome::Left("[detached: terminal closed]".to_string());
            }
            if n < 0 {
                let kind = std::io::Error::last_os_error().kind();
                if kind != std::io::ErrorKind::Interrupted && kind != std::io::ErrorKind::WouldBlock
                {
                    return Outcome::Left("[detached: terminal closed]".to_string());
                }
            } else if connection
                .send(&ClientMessage::Input(key_buf[..n as usize].to_vec()))
                .is_err()
            {
                return Outcome::Lost("[lost server]".to_string());
            }
        }

        // The daemon.
        if fds[1].revents & (libc::POLLIN | libc::POLLHUP | libc::POLLERR) != 0 {
            match connection.stream.read(&mut net_buf) {
                Ok(0) => return Outcome::Lost("[lost server]".to_string()),
                Ok(n) => connection.decoder.push(&net_buf[..n]),
                Err(e) if e.kind() == std::io::ErrorKind::Interrupted => {}
                Err(_) => return Outcome::Lost("[lost server]".to_string()),
            }
            if let Some(outcome) = handle_messages(connection, stdout_fd) {
                return outcome;
            }
        }
    }
}

/// Handle every complete message the decoder holds. `Some` ends the client.
fn handle_messages(connection: &mut Connection, stdout_fd: i32) -> Option<Outcome> {
    loop {
        match connection.decoder.next_message() {
            Ok(Some(ServerMessage::Output(bytes))) => {
                if write_all(stdout_fd, &bytes).is_err() {
                    return Some(Outcome::Left("[detached: terminal closed]".to_string()));
                }
            }
            Ok(Some(ServerMessage::Detached { message, .. })) => {
                return Some(Outcome::Left(format!("[{message}]")));
            }
            Ok(Some(ServerMessage::SessionEnded { message })) => {
                return Some(Outcome::Left(message));
            }
            Ok(Some(ServerMessage::CommandResult { output, .. })) => {
                return Some(Outcome::Refused(output));
            }
            Ok(Some(_)) => {}
            Ok(None) => return None,
            Err(e) => {
                return Some(Outcome::Lost(format!(
                    "arcade: bad message from the server: {e}"
                )));
            }
        }
    }
}
