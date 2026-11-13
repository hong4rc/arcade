//! A fake client for tests: start a private daemon, attach to it, type keys, and read the screen
//! back.
//!
//! The screen the test reads is a terminal emulation (`arcade-term`) fed with the bytes the
//! daemon sends, so what a test sees is what a real host terminal would show. Every wait has a
//! deadline and reports the screen when it runs out. A daemon started here is stopped when the
//! `TestDaemon` is dropped, also when the test fails, and its socket lives in its own temporary
//! directory, never at the default path.

use std::io::{Read, Write};
use std::os::unix::net::UnixStream;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU32, Ordering};
use std::thread::JoinHandle;
use std::time::{Duration, Instant};

use arcade_daemon::DaemonConfig;
use arcade_proto::{
    AttachRequest, ClientMessage, Decoder, PROTOCOL_VERSION, ServerMessage, encode,
};
use arcade_term::{Term, TermConfig};

/// How long the tests wait for something that should happen right away.
pub const WAIT: Duration = Duration::from_secs(10);

static COUNTER: AtomicU32 = AtomicU32::new(0);

pub struct TestDaemon {
    dir: PathBuf,
    socket: PathBuf,
    handle: Option<JoinHandle<std::io::Result<()>>>,
}

impl TestDaemon {
    pub fn start() -> TestDaemon {
        Self::start_with(|_| {})
    }

    pub fn start_with(adjust: impl FnOnce(&mut DaemonConfig)) -> TestDaemon {
        // A short path: socket paths are limited to about 100 bytes.
        let dir = PathBuf::from(format!(
            "/tmp/arc-{}-{}",
            std::process::id(),
            COUNTER.fetch_add(1, Ordering::Relaxed)
        ));
        std::fs::create_dir_all(&dir).expect("create test directory");
        let socket = dir.join("s");
        let mut config = DaemonConfig::new(&socket);
        adjust(&mut config);
        let handle = std::thread::spawn(move || arcade_daemon::run(config));
        let mut daemon = TestDaemon {
            dir,
            socket,
            handle: Some(handle),
        };
        let start = Instant::now();
        while UnixStream::connect(&daemon.socket).is_err() {
            assert!(start.elapsed() < WAIT, "daemon did not start listening");
            assert!(
                !daemon.handle.as_ref().is_some_and(JoinHandle::is_finished),
                "daemon stopped while starting: {:?}",
                daemon.handle.take().map(|h| h.join())
            );
            std::thread::sleep(Duration::from_millis(2));
        }
        daemon
    }

    pub fn socket(&self) -> &Path {
        &self.socket
    }

    /// A directory that belongs to this test, usable as home and working directory.
    pub fn dir(&self) -> &Path {
        &self.dir
    }

    /// Attach with these options (waits for the daemon's answer).
    pub fn attach(&self, options: AttachOptions) -> Result<TestClient, String> {
        let options = options.with_defaults(&self.dir);
        TestClient::attach_to(&self.socket, options)
    }

    /// Attach with a plain `/bin/sh` in a 100 by 30 terminal.
    pub fn attach_shell(&self) -> TestClient {
        self.attach(AttachOptions::new()).expect("attach")
    }

    /// Run a named command on a fresh connection.
    pub fn command(&self, name: &str, args: &[&str]) -> Result<(bool, String), String> {
        let mut client = TestClient::connect(&self.socket)?;
        client.send_message(&ClientMessage::Command {
            name: name.to_string(),
            args: args.iter().map(|s| s.to_string()).collect(),
        })?;
        let deadline = Instant::now() + WAIT;
        loop {
            for m in &client.messages {
                if let ServerMessage::CommandResult { ok, output } = m {
                    return Ok((*ok, output.clone()));
                }
            }
            if client.closed || Instant::now() >= deadline {
                return Err("no answer to the command".to_string());
            }
            client.pump(deadline);
        }
    }

    /// Whether the daemon has stopped.
    pub fn is_stopped(&self) -> bool {
        self.handle.as_ref().is_none_or(JoinHandle::is_finished)
    }

    /// Wait for the daemon to stop by itself; returns what `run` returned.
    pub fn wait_stopped(&mut self, timeout: Duration) -> Result<(), String> {
        let deadline = Instant::now() + timeout;
        while !self.is_stopped() {
            if Instant::now() >= deadline {
                return Err("daemon is still running".to_string());
            }
            std::thread::sleep(Duration::from_millis(5));
        }
        match self.handle.take() {
            Some(handle) => match handle.join() {
                Ok(Ok(())) => Ok(()),
                Ok(Err(e)) => Err(format!("daemon failed: {e}")),
                Err(_) => Err("daemon thread panicked".to_string()),
            },
            None => Ok(()),
        }
    }

    pub fn socket_exists(&self) -> bool {
        self.socket.exists()
    }
}

impl Drop for TestDaemon {
    fn drop(&mut self) {
        if self.handle.is_some() && !self.is_stopped() {
            let _ = self.command("server.kill", &[]);
            let _ = self.wait_stopped(Duration::from_secs(8));
        }
        let _ = std::fs::remove_dir_all(&self.dir);
    }
}

#[derive(Clone, Debug, Default)]
pub struct AttachOptions {
    pub session: Option<String>,
    pub create: bool,
    pub new_name: Option<String>,
    pub command: Vec<String>,
    pub cwd: Option<String>,
    pub env: Option<Vec<String>>,
    pub cols: u16,
    pub rows: u16,
}

impl AttachOptions {
    /// Create a session with `/bin/sh` and a 100 by 30 screen.
    pub fn new() -> Self {
        AttachOptions {
            create: true,
            command: vec!["/bin/sh".to_string()],
            cols: 100,
            rows: 30,
            ..Default::default()
        }
    }

    pub fn size(mut self, cols: u16, rows: u16) -> Self {
        self.cols = cols;
        self.rows = rows;
        self
    }

    pub fn command(mut self, argv: &[&str]) -> Self {
        self.command = argv.iter().map(|s| s.to_string()).collect();
        self
    }

    pub fn named(mut self, name: &str) -> Self {
        self.new_name = Some(name.to_string());
        self
    }

    /// Attach to an existing session instead of creating one.
    pub fn existing(mut self, name: Option<&str>) -> Self {
        self.create = false;
        self.session = name.map(str::to_string);
        self.command.clear();
        self
    }

    fn with_defaults(mut self, dir: &Path) -> Self {
        if self.cwd.is_none() {
            self.cwd = Some(dir.display().to_string());
        }
        if self.env.is_none() {
            // A small, fixed environment so that a shell starts the same on every machine.
            self.env = Some(vec![
                "PATH=/usr/bin:/bin:/usr/sbin:/sbin".to_string(),
                format!("HOME={}", dir.display()),
                "PS1=$ ".to_string(),
                "LANG=en_US.UTF-8".to_string(),
                "SHELL=/bin/sh".to_string(),
            ]);
        }
        self
    }
}

pub struct TestClient {
    stream: UnixStream,
    decoder: Decoder<ServerMessage>,
    /// What the host terminal would show.
    pub screen: Term,
    /// Every message except screen output, in order of arrival.
    pub messages: Vec<ServerMessage>,
    /// The daemon closed the connection.
    pub closed: bool,
    /// Number of `Output` messages received.
    pub output_messages: usize,
    /// Number of bytes of screen output received.
    pub output_bytes: usize,
}

impl TestClient {
    /// Connect and do the handshake, without attaching.
    pub fn connect(socket: &Path) -> Result<TestClient, String> {
        let stream = UnixStream::connect(socket).map_err(|e| format!("connect: {e}"))?;
        let mut client = TestClient {
            stream,
            decoder: Decoder::new(),
            screen: Term::new(TermConfig::new(80, 24)).map_err(|e| e.to_string())?,
            messages: Vec::new(),
            closed: false,
            output_messages: 0,
            output_bytes: 0,
        };
        client.send_message(&ClientMessage::Hello {
            version: PROTOCOL_VERSION,
        })?;
        client.wait_message(|m| matches!(m, ServerMessage::HelloOk { .. }), WAIT)?;
        Ok(client)
    }

    pub fn attach_to(socket: &Path, options: AttachOptions) -> Result<TestClient, String> {
        let mut client = Self::connect(socket)?;
        client.screen =
            Term::new(TermConfig::new(options.cols, options.rows)).map_err(|e| e.to_string())?;
        client.send_message(&ClientMessage::Attach(AttachRequest {
            session: options.session,
            create: options.create,
            new_name: options.new_name,
            command: options.command,
            cwd: options.cwd.unwrap_or_default(),
            env: options.env.unwrap_or_default(),
            cols: options.cols,
            rows: options.rows,
        }))?;
        let deadline = Instant::now() + WAIT;
        loop {
            for m in &client.messages {
                match m {
                    ServerMessage::Attached { .. } => return Ok(client),
                    ServerMessage::CommandResult { ok: false, output } => {
                        return Err(output.clone());
                    }
                    _ => {}
                }
            }
            if client.closed || Instant::now() >= deadline {
                return Err("daemon did not accept the attach".to_string());
            }
            client.pump(deadline);
        }
    }

    pub fn send_message(&mut self, msg: &ClientMessage) -> Result<(), String> {
        let mut bytes = Vec::new();
        encode(msg, &mut bytes).map_err(|e| e.to_string())?;
        self.stream
            .write_all(&bytes)
            .map_err(|e| format!("write: {e}"))
    }

    /// Send keyboard bytes.
    pub fn send(&mut self, bytes: &[u8]) {
        self.send_message(&ClientMessage::Input(bytes.to_vec()))
            .expect("send keys");
    }

    /// Type a line and press Enter.
    pub fn type_line(&mut self, line: &str) {
        let mut bytes = line.as_bytes().to_vec();
        bytes.push(b'\r');
        self.send(&bytes);
    }

    pub fn resize(&mut self, cols: u16, rows: u16) {
        self.screen.resize(cols, rows).expect("resize screen");
        self.send_message(&ClientMessage::Resize { cols, rows })
            .expect("send resize");
    }

    pub fn detach(&mut self) {
        self.send_message(&ClientMessage::Detach).expect("detach");
    }

    /// Read what has arrived, waiting no longer than `deadline`. Returns whether anything came.
    pub fn pump(&mut self, deadline: Instant) -> bool {
        let remaining = deadline.saturating_duration_since(Instant::now());
        let remaining = remaining.max(Duration::from_millis(1));
        let _ = self.stream.set_read_timeout(Some(remaining));
        let mut buf = vec![0u8; 64 * 1024];
        match self.stream.read(&mut buf) {
            Ok(0) => {
                self.closed = true;
                false
            }
            Ok(n) => {
                self.decoder.push(&buf[..n]);
                loop {
                    match self.decoder.next_message() {
                        Ok(Some(ServerMessage::Output(bytes))) => {
                            self.output_messages += 1;
                            self.output_bytes += bytes.len();
                            self.screen.feed(&bytes);
                            // The virtual screen's own replies go nowhere.
                            let _ = self.screen.drain_events();
                        }
                        Ok(Some(other)) => self.messages.push(other),
                        Ok(None) => break,
                        Err(e) => panic!("daemon sent a bad message: {e}"),
                    }
                }
                true
            }
            Err(e)
                if matches!(
                    e.kind(),
                    std::io::ErrorKind::WouldBlock | std::io::ErrorKind::TimedOut
                ) =>
            {
                false
            }
            Err(_) => {
                self.closed = true;
                false
            }
        }
    }

    /// Keep reading until `done` is true or the time is up. Returns whether `done` became true.
    pub fn wait_until(
        &mut self,
        timeout: Duration,
        mut done: impl FnMut(&mut TestClient) -> bool,
    ) -> bool {
        let deadline = Instant::now() + timeout;
        loop {
            if done(self) {
                return true;
            }
            if self.closed || Instant::now() >= deadline {
                return done(self);
            }
            self.pump(deadline);
        }
    }

    /// The text of every row of the screen, trailing blanks removed.
    pub fn screen_rows(&self) -> Vec<String> {
        (0..self.screen.rows())
            .map(|r| self.screen.row_text(r))
            .collect()
    }

    /// The screen as one string, rows joined by newlines, empty rows at the end dropped.
    pub fn screen_text(&self) -> String {
        let mut rows = self.screen_rows();
        while rows.last().is_some_and(String::is_empty) {
            rows.pop();
        }
        rows.join("\n")
    }

    /// Wait until some row of the screen contains `needle`.
    pub fn wait_for_text(&mut self, needle: &str) -> Result<(), String> {
        self.wait_for_text_within(needle, WAIT)
    }

    pub fn wait_for_text_within(&mut self, needle: &str, timeout: Duration) -> Result<(), String> {
        let found = self.wait_until(timeout, |c| {
            c.screen_rows().iter().any(|r| r.contains(needle))
        });
        if found {
            Ok(())
        } else {
            Err(format!(
                "did not see {needle:?} within {timeout:?}{}; the screen shows:\n{}",
                if self.closed {
                    " (connection closed)"
                } else {
                    ""
                },
                self.screen_text()
            ))
        }
    }

    /// Wait until the shell's prompt (`$`, set by the test environment) is the last thing on
    /// the screen: the shell is ready for the next command.
    pub fn wait_for_prompt(&mut self) -> Result<(), String> {
        let ok = self.wait_until(WAIT, |c| {
            c.screen_rows()
                .iter()
                .rev()
                .find(|r| !r.is_empty())
                .is_some_and(|r| r == "$")
        });
        if ok {
            Ok(())
        } else {
            Err(format!(
                "no shell prompt; the screen shows:\n{}",
                self.screen_text()
            ))
        }
    }

    /// Wait for a message that is not screen output.
    pub fn wait_message(
        &mut self,
        pred: impl Fn(&ServerMessage) -> bool,
        timeout: Duration,
    ) -> Result<ServerMessage, String> {
        let mut found = None;
        let ok = self.wait_until(timeout, |c| {
            found = c.messages.iter().find(|m| pred(m)).cloned();
            found.is_some()
        });
        match (ok, found) {
            (true, Some(m)) => Ok(m),
            _ => Err(format!(
                "expected message did not arrive; got {:?}; screen:\n{}",
                self.messages,
                self.screen_text()
            )),
        }
    }

    /// Wait until the daemon closes the connection.
    pub fn wait_closed(&mut self, timeout: Duration) -> bool {
        self.wait_until(timeout, |c| c.closed)
    }
}
