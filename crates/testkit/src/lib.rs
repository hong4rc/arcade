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
    AttachRequest, ClientMessage, CreateMode, Decoder, PROTOCOL_VERSION, ServerMessage, encode,
};
use arcade_term::{Event, Term, TermConfig};

/// Memory a process really holds, in KiB: on macOS the system's "physical footprint", which
/// (unlike the resident size) drops when memory is compressed or given back; elsewhere the
/// resident size.
pub fn footprint_kib(pid: u32) -> u64 {
    #[cfg(target_os = "macos")]
    {
        // SAFETY: proc_pid_rusage fills the struct we pass for the flavor we ask for.
        let mut info: libc::rusage_info_v2 = unsafe { std::mem::zeroed() };
        // SAFETY: as above; info is a live local of the size the flavor expects.
        let rc = unsafe {
            libc::proc_pid_rusage(
                pid as i32,
                libc::RUSAGE_INFO_V2,
                (&raw mut info).cast::<libc::rusage_info_t>(),
            )
        };
        if rc == 0 {
            return info.ri_phys_footprint / 1024;
        }
    }
    let out = std::process::Command::new("ps")
        .args(["-o", "rss=", "-p", &pid.to_string()])
        .output()
        .expect("ps");
    String::from_utf8_lossy(&out.stdout)
        .trim()
        .parse()
        .unwrap_or(0)
}

/// The processor time this process has used so far (user and system), all threads together.
pub fn cpu_time() -> Duration {
    // SAFETY: getrusage fills the struct we pass.
    let mut usage: libc::rusage = unsafe { std::mem::zeroed() };
    // SAFETY: as above; usage is a live local.
    unsafe { libc::getrusage(libc::RUSAGE_SELF, &mut usage) };
    let of = |t: libc::timeval| Duration::new(t.tv_sec as u64, (t.tv_usec as u32) * 1000);
    of(usage.ru_utime) + of(usage.ru_stime)
}

/// How long the tests wait for something that should happen right away.
pub const WAIT: Duration = Duration::from_secs(10);

/// Wait until a file exists, for at most `timeout`. A program in a pane writes one when it has
/// done what a test waits for, so the test needs no fixed sleep.
pub fn wait_for_file(path: &std::path::Path, timeout: Duration) -> bool {
    let deadline = std::time::Instant::now() + timeout;
    while !path.exists() {
        if std::time::Instant::now() >= deadline {
            return false;
        }
        std::thread::sleep(Duration::from_millis(5));
    }
    true
}

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
        // A pane made after the first one runs a login shell, which reads the system's profile
        // and then this one (the test's home is this directory): it puts the prompt back.
        std::fs::write(dir.join(".profile"), "PS1='$ '\n").expect("write the test profile");
        let socket = dir.join("s");
        let mut config = DaemonConfig::new(&socket);
        // Hundreds of tests start a daemon: none of them should start a process that holds the
        // machine awake. The tests of that feature turn it on.
        config.config.prevent_sleep = false;
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
        self.command_from(None, name, args)
    }

    /// Run a named command as `arcade run` does inside a pane: on the session and window of the
    /// pane (the number in that pane's `ARCADE_PANE` variable). Without a pane it acts on the
    /// session whose client is attached.
    pub fn command_from(
        &self,
        pane: Option<u64>,
        name: &str,
        args: &[&str],
    ) -> Result<(bool, String), String> {
        let mut client = TestClient::connect(&self.socket)?;
        let args: Vec<String> = args.iter().map(|s| s.to_string()).collect();
        client.send_message(&match pane {
            Some(pane) => ClientMessage::PaneCommand {
                pane,
                name: name.to_string(),
                args,
            },
            None => ClientMessage::Command {
                name: name.to_string(),
                args,
            },
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
            client.read_more(deadline);
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

#[derive(Clone, Debug)]
pub struct AttachOptions {
    pub session: Option<String>,
    pub create: CreateMode,
    pub new_name: Option<String>,
    pub command: Vec<String>,
    pub cwd: Option<String>,
    pub env: Option<Vec<String>>,
    pub cols: u16,
    pub rows: u16,
}

impl Default for AttachOptions {
    fn default() -> Self {
        Self::new()
    }
}

impl AttachOptions {
    /// Create a session with `/bin/sh` and a 100 by 30 screen.
    pub fn new() -> Self {
        AttachOptions {
            session: None,
            create: CreateMode::Always,
            new_name: None,
            command: vec!["/bin/sh".to_string()],
            cwd: None,
            env: None,
            cols: 100,
            rows: 30,
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
        self.create = CreateMode::Never;
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

/// Something with a screen that fills as bytes arrive: the fake client and the fake host
/// terminal. Waiting helpers all work the same way: read until the condition holds or the
/// deadline passes, and say what the screen shows when it does not hold.
pub trait Screened: Sized {
    fn term(&self) -> &Term;
    /// Read what has arrived, waiting no longer than `deadline`.
    fn pump(&mut self, deadline: Instant) -> bool;
    /// The other side is gone.
    fn is_closed(&self) -> bool;

    /// The text of every row of the screen, trailing blanks removed.
    fn screen_rows(&self) -> Vec<String> {
        let term = self.term();
        (0..term.rows()).map(|r| term.row_text(r)).collect()
    }

    /// The rows the panes show: every row except the last, which is the status bar.
    fn content_rows(&self) -> Vec<String> {
        let mut rows = self.screen_rows();
        rows.pop();
        rows
    }

    /// The text of the status bar (the last row).
    fn status_row(&self) -> String {
        self.screen_rows().pop().unwrap_or_default()
    }

    /// The screen as one string, rows joined by newlines, empty rows at the end dropped.
    fn screen_text(&self) -> String {
        let mut rows = self.screen_rows();
        while rows.last().is_some_and(String::is_empty) {
            rows.pop();
        }
        rows.join("\n")
    }

    /// Keep reading until `done` is true or the time is up. Returns whether `done` became true.
    fn wait_until(&mut self, timeout: Duration, mut done: impl FnMut(&mut Self) -> bool) -> bool {
        let deadline = Instant::now() + timeout;
        loop {
            if done(self) {
                return true;
            }
            if self.is_closed() || Instant::now() >= deadline {
                return done(self);
            }
            self.pump(deadline);
        }
    }

    /// Wait until some row of the screen contains `needle`.
    fn wait_for_text(&mut self, needle: &str) -> Result<(), String> {
        self.wait_for_text_within(needle, WAIT)
    }

    fn wait_for_text_within(&mut self, needle: &str, timeout: Duration) -> Result<(), String> {
        let found = self.wait_until(timeout, |c| {
            c.screen_rows().iter().any(|r| r.contains(needle))
        });
        if found {
            Ok(())
        } else {
            Err(format!(
                "did not see {needle:?} within {timeout:?}{}; the screen shows:\n{}",
                if self.is_closed() { " (closed)" } else { "" },
                self.screen_text()
            ))
        }
    }

    /// Wait until the shell's prompt (`$`, set by the test environment) is the last thing on
    /// the screen: the shell is ready for the next command.
    fn wait_for_prompt(&mut self) -> Result<(), String> {
        let ok = self.wait_until(WAIT, |c| {
            c.content_rows()
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

    /// Wait until some row of the screen is exactly `row`.
    fn wait_for_row(&mut self, row: &str) -> Result<(), String> {
        let ok = self.wait_until(WAIT, |c| c.screen_rows().iter().any(|r| r == row));
        if ok {
            Ok(())
        } else {
            Err(format!(
                "no row {row:?}; the screen shows:\n{}",
                self.screen_text()
            ))
        }
    }
}

impl Screened for TestClient {
    fn term(&self) -> &Term {
        &self.screen
    }
    fn pump(&mut self, deadline: Instant) -> bool {
        self.read_more(deadline)
    }
    fn is_closed(&self) -> bool {
        self.closed
    }
}

/// The text of some cells: a blank cell is a space, and trailing blanks are cut.
pub fn cells_text(cells: &[arcade_term::Cell]) -> String {
    let text: String = cells
        .iter()
        .map(|c| {
            if c.text.is_empty() {
                " ".to_string()
            } else {
                c.text.clone()
            }
        })
        .collect();
    text.trim_end().to_string()
}

static MARKER: AtomicU32 = AtomicU32::new(0);

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
    /// What the host terminal was asked to put on its clipboard (OSC 52), decoded, in order.
    pub clipboard_writes: Vec<Vec<u8>>,
    /// How many bells the daemon rang the host terminal with, and the notifications it asked for
    /// (title and body) in the order they came.
    pub bells: usize,
    pub host_notifications: Vec<(String, String)>,
    /// The titles the host terminal's window was given (OSC 0 or 2), in order.
    pub titles: Vec<String>,
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
            clipboard_writes: Vec::new(),
            bells: 0,
            host_notifications: Vec::new(),
            titles: Vec::new(),
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
            client.read_more(deadline);
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

    /// The text of a rectangle of the screen, one string per row, trailing blanks cut. Only
    /// meant for screens without wide characters.
    pub fn region(&mut self, x: u16, y: u16, w: u16, h: u16) -> Vec<String> {
        let grid = self.screen.screen().expect("read the screen");
        grid.iter()
            .skip(usize::from(y))
            .take(usize::from(h))
            .map(|row| {
                let end = (usize::from(x) + usize::from(w)).min(row.len());
                cells_text(&row[usize::from(x).min(end)..end])
            })
            .collect()
    }

    /// The style of one cell of the screen.
    pub fn style_at(&self, row: u16, col: u16) -> arcade_term::Style {
        self.screen.style_at(row, col)
    }

    /// The number of the pane that has the focus (its `ARCADE_PANE` variable), asked of the
    /// shell running in it.
    /// How many panes show nothing but a shell prompt on a row of their own (a pane that waits
    /// for a command shows one).
    fn idle_prompts(&self) -> usize {
        self.content_rows()
            .iter()
            .map(|r| r.split('│').filter(|part| part.trim() == "$").count())
            .sum()
    }

    /// Split the pane that has the focus into left and right with the default prefix, and wait
    /// until the shell in the new pane has drawn its prompt. A line typed before that is echoed
    /// by the terminal first and the prompt comes after it, which puts the lines of the two
    /// panes on different rows.
    pub fn split_right(&mut self) {
        let before = self.idle_prompts();
        self.send(b"\x1b[109;5u");
        self.send(b"|");
        let ok = self.wait_until(WAIT, |c| {
            c.screen_text().contains('│') && c.idle_prompts() > before
        });
        assert!(ok, "the new pane shows no prompt:\n{}", self.screen_text());
    }

    pub fn focused_pane_id(&mut self) -> u64 {
        let marker = format!("pane-id{}=", MARKER.fetch_add(1, Ordering::Relaxed));
        self.type_line(&format!("echo {marker}$ARCADE_PANE."));
        let mut found = None;
        let ok = self.wait_until(WAIT, |c| {
            for row in c.screen_rows() {
                let mut from = 0;
                while let Some(at) = row[from..].find(&marker) {
                    let rest = &row[from + at + marker.len()..];
                    let digits: String = rest.chars().take_while(char::is_ascii_digit).collect();
                    if !digits.is_empty() && rest[digits.len()..].starts_with('.') {
                        found = digits.parse().ok();
                        return true;
                    }
                    from += at + marker.len();
                }
            }
            false
        });
        assert!(
            ok,
            "no answer to the pane id question:\n{}",
            self.screen_text()
        );
        found.expect("pane id")
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
    pub fn read_more(&mut self, deadline: Instant) -> bool {
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
                            // The virtual screen's own replies go nowhere; what it was asked to
                            // put on the clipboard is kept.
                            for event in self.screen.drain_events() {
                                match event {
                                    Event::ClipboardWrite(data) => self.clipboard_writes.push(data),
                                    Event::Title(title) => self.titles.push(title),
                                    Event::Bell => self.bells += 1,
                                    Event::Notification { title, body } => {
                                        self.host_notifications.push((title, body));
                                    }
                                    _ => {}
                                }
                            }
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

// ---- A fake host terminal ----

/// The terminal state a program can change: what `arcade` must put back when it leaves.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct TermSettings {
    pub input_flags: u64,
    pub output_flags: u64,
    pub control_flags: u64,
    pub local_flags: u64,
    pub control_chars: Vec<u8>,
}

/// A program running in a real pseudo terminal, with a virtual screen showing what it draws.
/// This is how the tests run the real `arcade` command the way a person would.
pub struct PtyHost {
    master: std::fs::File,
    child: std::process::Child,
    pub pid: i32,
    pub screen: Term,
    closed: bool,
    initial: TermSettings,
    /// Every byte the program wrote to the terminal, in order.
    pub raw: Vec<u8>,
    /// The titles the terminal's window was given, in order.
    pub titles: Vec<String>,
}

impl PtyHost {
    /// Run `program` with `args` in a new terminal of the given size. `env` is its whole
    /// environment.
    pub fn spawn(
        program: &Path,
        args: &[&str],
        env: Vec<String>,
        cwd: &Path,
        cols: u16,
        rows: u16,
    ) -> PtyHost {
        let mut argv = vec![program.display().to_string()];
        argv.extend(args.iter().map(|a| a.to_string()));
        let spawned = arcade_daemon::pty::spawn(&arcade_daemon::pty::SpawnSpec {
            argv,
            cwd: cwd.display().to_string(),
            env,
            cols,
            rows,
        })
        .expect("start the program in a terminal");
        let mut host = PtyHost {
            master: spawned.master,
            child: spawned.child,
            pid: spawned.pid,
            screen: Term::new(TermConfig::new(cols, rows)).expect("screen"),
            closed: false,
            initial: TermSettings {
                input_flags: 0,
                output_flags: 0,
                control_flags: 0,
                local_flags: 0,
                control_chars: Vec::new(),
            },
            raw: Vec::new(),
            titles: Vec::new(),
        };
        host.initial = host.settings();
        host
    }

    /// The terminal's settings right now.
    pub fn settings(&self) -> TermSettings {
        use std::os::fd::AsRawFd;
        // SAFETY: tcgetattr fills a termios struct we own.
        let mut t: libc::termios = unsafe { std::mem::zeroed() };
        // SAFETY: as above; t is a live local.
        let rc = unsafe { libc::tcgetattr(self.master.as_raw_fd(), &mut t) };
        assert_eq!(rc, 0, "tcgetattr: {}", std::io::Error::last_os_error());
        TermSettings {
            input_flags: t.c_iflag as u64,
            output_flags: t.c_oflag as u64,
            control_flags: t.c_cflag as u64,
            local_flags: t.c_lflag as u64,
            control_chars: t.c_cc.to_vec(),
        }
    }

    /// The settings the terminal had before the program ran.
    pub fn initial_settings(&self) -> &TermSettings {
        &self.initial
    }

    pub fn send(&mut self, bytes: &[u8]) {
        let mut rest = bytes;
        while !rest.is_empty() {
            match self.master.write(rest) {
                Ok(n) => rest = &rest[n..],
                Err(e) if e.kind() == std::io::ErrorKind::WouldBlock => {
                    std::thread::sleep(Duration::from_millis(1));
                }
                Err(e) => panic!("write to the terminal: {e}"),
            }
        }
    }

    pub fn type_line(&mut self, line: &str) {
        let mut bytes = line.as_bytes().to_vec();
        bytes.push(b'\r');
        self.send(&bytes);
    }

    /// Resize the terminal; the program gets the window size signal.
    pub fn resize(&mut self, cols: u16, rows: u16) {
        arcade_daemon::pty::set_window_size(&self.master, cols, rows).expect("resize");
        self.screen.resize(cols, rows).expect("resize screen");
    }

    /// Wait for the program to end and return its exit code.
    pub fn wait_exit(&mut self, timeout: Duration) -> Option<i32> {
        let deadline = Instant::now() + timeout;
        loop {
            // Keep the screen up to date while waiting.
            self.pump(Instant::now() + Duration::from_millis(5));
            match self.child.try_wait() {
                Ok(Some(status)) => {
                    // Take what it wrote last.
                    self.pump(Instant::now() + Duration::from_millis(5));
                    use std::os::unix::process::ExitStatusExt;
                    return Some(
                        status
                            .code()
                            .unwrap_or_else(|| 128 + status.signal().unwrap_or(0)),
                    );
                }
                Ok(None) => {}
                Err(_) => return None,
            }
            if Instant::now() >= deadline {
                return None;
            }
        }
    }

    /// Send a signal to the program.
    pub fn signal(&self, signal: i32) {
        // SAFETY: kill with plain integers.
        unsafe {
            libc::kill(self.pid, signal);
        }
    }
}

impl Screened for PtyHost {
    fn term(&self) -> &Term {
        &self.screen
    }

    fn pump(&mut self, deadline: Instant) -> bool {
        use std::os::fd::AsRawFd;
        if self.closed {
            return false;
        }
        let remaining = deadline.saturating_duration_since(Instant::now());
        let mut pfd = libc::pollfd {
            fd: self.master.as_raw_fd(),
            events: libc::POLLIN,
            revents: 0,
        };
        // SAFETY: poll on one live pollfd.
        let rc = unsafe { libc::poll(&mut pfd, 1, remaining.as_millis().clamp(1, 1000) as i32) };
        if rc <= 0 {
            return false;
        }
        let mut buf = vec![0u8; 64 * 1024];
        match self.master.read(&mut buf) {
            Ok(0) => {
                self.closed = true;
                false
            }
            Ok(n) => {
                self.raw.extend_from_slice(&buf[..n]);
                self.screen.feed(&buf[..n]);
                // Answer the program's queries the way a real terminal would.
                for event in self.screen.drain_events() {
                    match event {
                        arcade_term::Event::Reply(bytes) => {
                            let _ = self.master.write_all(&bytes);
                        }
                        arcade_term::Event::Title(title) => self.titles.push(title),
                        _ => {}
                    }
                }
                true
            }
            Err(e) if e.kind() == std::io::ErrorKind::WouldBlock => false,
            Err(_) => {
                self.closed = true;
                false
            }
        }
    }

    fn is_closed(&self) -> bool {
        self.closed
    }
}

impl Drop for PtyHost {
    fn drop(&mut self) {
        let _ = self.child.kill();
        let _ = self.child.wait();
    }
}
