//! The attached client: the keyboard goes up, the screen comes down.
//!
//! The client does four things: it puts the host terminal in raw mode, passes the bytes from the
//! keyboard to the daemon untouched, writes the bytes the daemon sends to the host terminal
//! untouched, and reports the terminal's size when it changes.

use std::io::Read;
use std::os::fd::AsRawFd;

use arcade_daemon::{stage_count, stage_mark};
use arcade_proto::{AttachRequest, ClientMessage, ServerMessage};

use crate::connection::Connection;
use crate::host::{ENTER_SEQUENCE, RESTORE_SEQUENCE, RawMode, Signals, window_size};
use crate::wait::{Ready, Waiter};

pub enum Outcome {
    /// Left on purpose or because the session ended; the text is a line for the user.
    Left(String),
    /// The server went away or sent something wrong.
    Lost(String),
    /// The daemon refused the request.
    Refused(String),
    /// The daemon replaced its program and keeps the session: run this program's `attach` for
    /// that session in this terminal.
    Upgrade { program: String, session: String },
}

/// Ask the daemon to attach, then run until we are detached. Returns the exit code.
pub fn attach(
    mut connection: Connection,
    mut request: AttachRequest,
    socket: &std::path::Path,
) -> i32 {
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
    #[cfg(feature = "stage-timing")]
    let stage_file = stage_file();
    let outcome = run(&mut connection, &signals, stdin_fd, stdout_fd);
    #[cfg(feature = "stage-timing")]
    if let Some(path) = stage_file {
        arcade_daemon::stage::dump(&path);
    }
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
        Outcome::Upgrade { program, session } => reattach_with(&program, &session, socket),
    }
}

/// Become the new program's `attach` for the session, in this terminal. A program that cannot be
/// started yet (an install may be finishing) is tried again for a few seconds.
fn reattach_with(program: &str, session: &str, socket: &std::path::Path) -> i32 {
    use std::os::unix::process::CommandExt;
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(5);
    loop {
        let error = std::process::Command::new(program)
            .arg("--socket")
            .arg(socket)
            .arg("attach")
            .arg("-t")
            .arg(session)
            .exec();
        if std::time::Instant::now() >= deadline {
            eprintln!(
                "arcade: the daemon now runs a newer arcade and keeps your session, but this screen could not start it ({error}); type: {program} --socket {} attach -t {session}",
                socket.display()
            );
            return 1;
        }
        std::thread::sleep(std::time::Duration::from_millis(200));
    }
}

/// Where this process writes its stage-timing marks: next to the daemon's socket, in a file of
/// its own. The socket is the one given with `--socket` (the measurement always gives one).
#[cfg(feature = "stage-timing")]
fn stage_file() -> Option<std::path::PathBuf> {
    let args: Vec<String> = std::env::args().collect();
    let at = args.iter().position(|a| a == "--socket")?;
    let socket = std::path::PathBuf::from(args.get(at + 1)?);
    Some(socket.with_extension(format!("stages-client-{}", std::process::id())))
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
    let waiter = Waiter::new([stdin_fd, socket_fd, signals.read_fd]);
    loop {
        let ready = match &waiter {
            Some(waiter) => match waiter.wait() {
                Ok(Some(ready)) => ready,
                Ok(None) => continue,
                Err(_) => return Outcome::Lost("[lost terminal]".to_string()),
            },
            None => match poll_ready(stdin_fd, socket_fd, signals.read_fd) {
                Ok(Some(ready)) => ready,
                Ok(None) => continue,
                Err(_) => return Outcome::Lost("[lost terminal]".to_string()),
            },
        };
        stage_mark!(ClientWake);
        stage_count!(Poll);

        // Signals first: a hangup or termination ends the client at once (the session lives on).
        if ready[2] {
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
        if ready[0] {
            // SAFETY: read into a live buffer of the stated length.
            let n = unsafe { libc::read(stdin_fd, key_buf.as_mut_ptr().cast(), key_buf.len()) };
            stage_count!(Read);
            stage_mark!(ClientKeyRead);
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
        if ready[1] {
            stage_count!(Read);
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

/// Wait with `poll` for the keyboard, the socket and the signal pipe: for terminals the event
/// queue of `wait::Waiter` cannot take. `Ok(None)` when a signal interrupted the wait.
fn poll_ready(stdin_fd: i32, socket_fd: i32, signal_fd: i32) -> std::io::Result<Option<Ready>> {
    let mut fds = [stdin_fd, socket_fd, signal_fd].map(|fd| libc::pollfd {
        fd,
        events: libc::POLLIN,
        revents: 0,
    });
    // SAFETY: poll on three live pollfds, waiting with no timeout.
    let rc = unsafe { libc::poll(fds.as_mut_ptr(), 3, -1) };
    if rc < 0 {
        let e = std::io::Error::last_os_error();
        return if e.kind() == std::io::ErrorKind::Interrupted {
            Ok(None)
        } else {
            Err(e)
        };
    }
    let any = libc::POLLIN | libc::POLLHUP | libc::POLLERR;
    Ok(Some([
        fds[0].revents & any != 0,
        fds[1].revents & any != 0,
        fds[2].revents != 0,
    ]))
}

/// Handle every complete message the decoder holds. `Some` ends the client.
fn handle_messages(connection: &mut Connection, stdout_fd: i32) -> Option<Outcome> {
    loop {
        match connection.decoder.next_message() {
            Ok(Some(ServerMessage::Output(bytes))) => {
                stage_count!(Write);
                if write_all(stdout_fd, &bytes).is_err() {
                    return Some(Outcome::Left("[detached: terminal closed]".to_string()));
                }
                stage_mark!(ClientStdoutWritten);
            }
            Ok(Some(ServerMessage::Detached {
                reason: arcade_proto::DetachReason::Upgraded,
                message,
            })) => {
                return Some(match message.split_once('\0') {
                    Some((program, session)) => Outcome::Upgrade {
                        program: program.to_string(),
                        session: session.to_string(),
                    },
                    None => Outcome::Left(format!("[{message}]")),
                });
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
