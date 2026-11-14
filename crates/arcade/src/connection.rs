//! Talking to the daemon: connecting, starting it when needed, the handshake.

use std::io::{Read, Write};
use std::os::unix::net::UnixStream;
use std::os::unix::process::CommandExt;
use std::path::Path;
use std::process::{Command, Stdio};
use std::time::{Duration, Instant};

use arcade_daemon::stage_count;
#[cfg(feature = "stage-timing")]
use arcade_daemon::stage_mark;
use arcade_proto::{ClientMessage, Decoder, PROTOCOL_VERSION, ServerMessage, encode};

pub enum ConnectError {
    /// Nothing listens on the socket.
    NoServer,
    Other(String),
}

pub struct Connection {
    pub stream: UnixStream,
    pub decoder: Decoder<ServerMessage>,
}

pub enum Handshake {
    Ok(Connection),
    /// The daemon speaks another protocol version. The connection can still carry `server.kill`.
    Mismatch {
        daemon: u32,
        client: u32,
        connection: Connection,
    },
}

pub fn connect(socket: &Path) -> Result<UnixStream, ConnectError> {
    UnixStream::connect(socket).map_err(|e| match e.kind() {
        std::io::ErrorKind::NotFound | std::io::ErrorKind::ConnectionRefused => {
            ConnectError::NoServer
        }
        _ => ConnectError::Other(format!("cannot connect to {}: {e}", socket.display())),
    })
}

impl Connection {
    pub fn send(&mut self, message: &ClientMessage) -> Result<(), String> {
        let mut bytes = Vec::new();
        encode(message, &mut bytes).map_err(|e| e.to_string())?;
        stage_count!(Write);
        let sent = self
            .stream
            .write_all(&bytes)
            .map_err(|e| format!("lost the server: {e}"));
        #[cfg(feature = "stage-timing")]
        if matches!(message, ClientMessage::Input(_)) {
            stage_mark!(ClientKeySent);
        }
        sent
    }

    /// Wait for the next message. `Ok(None)` means the daemon closed the connection.
    pub fn receive(&mut self) -> Result<Option<ServerMessage>, String> {
        let mut buf = [0u8; 64 * 1024];
        loop {
            if let Some(message) = self.decoder.next_message().map_err(|e| e.to_string())? {
                return Ok(Some(message));
            }
            match self.stream.read(&mut buf) {
                Ok(0) => return Ok(None),
                Ok(n) => self.decoder.push(&buf[..n]),
                Err(e) if e.kind() == std::io::ErrorKind::Interrupted => {}
                Err(e) => return Err(format!("lost the server: {e}")),
            }
        }
    }
}

pub fn handshake(stream: UnixStream) -> Result<Handshake, String> {
    let mut connection = Connection {
        stream,
        decoder: Decoder::new(),
    };
    connection.send(&ClientMessage::Hello {
        version: PROTOCOL_VERSION,
    })?;
    match connection.receive()? {
        Some(ServerMessage::HelloOk { .. }) => Ok(Handshake::Ok(connection)),
        Some(ServerMessage::VersionMismatch { daemon, client }) => Ok(Handshake::Mismatch {
            daemon,
            client,
            connection,
        }),
        Some(other) => Err(format!("unexpected answer from the server: {other:?}")),
        None => Err("the server closed the connection".to_string()),
    }
}

/// The text shown when the client and the daemon disagree on the protocol version.
pub fn mismatch_message(daemon: u32, client: u32) -> String {
    format!(
        "arcade: version mismatch: this client speaks protocol version {client}, \
         the running daemon speaks {daemon}.\n\
         The daemon was started by another version of arcade. To use this version, stop the old \
         daemon with `arcade kill-server` (this ends every session it holds), then run arcade again."
    )
}

/// Start the daemon in the background, detached from this terminal, and wait until it listens.
pub fn start_daemon(socket: &Path) -> Result<(), String> {
    let exe = std::env::current_exe().map_err(|e| format!("cannot find myself: {e}"))?;
    let mut command = Command::new(exe);
    command
        .arg("--socket")
        .arg(socket)
        .arg("__daemon")
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null());
    // SAFETY: only setsid runs between fork and exec, and it is async-signal-safe. It makes the
    // daemon a session leader with no terminal, so closing this terminal does not touch it.
    unsafe {
        command.pre_exec(|| {
            if libc::setsid() < 0 {
                return Err(std::io::Error::last_os_error());
            }
            Ok(())
        });
    }
    let mut child = command
        .spawn()
        .map_err(|e| format!("cannot start the daemon: {e}"))?;
    let deadline = Instant::now() + Duration::from_secs(5);
    loop {
        if UnixStream::connect(socket).is_ok() {
            return Ok(());
        }
        if let Ok(Some(status)) = child.try_wait() {
            // Another client may have started a daemon at the same moment and won.
            if UnixStream::connect(socket).is_ok() {
                return Ok(());
            }
            return Err(format!("the daemon stopped right away ({status})"));
        }
        if Instant::now() >= deadline {
            return Err("the daemon did not start in time".to_string());
        }
        std::thread::sleep(Duration::from_millis(5));
    }
}
