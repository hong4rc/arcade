//! The Arcade benchmark suite (decision 003).
//!
//! It plays the host terminal: it opens a terminal it owns, runs the client of the subject in
//! it, sends keys and reads what the client writes. The same measurements run against "direct"
//! (no multiplexer), tmux and Arcade, on identical data from `arcade-workload`.
//!
//! Run with `cargo bench-run [--quick] [--subjects direct,tmux,arcade] [--tests a,b]`.
//! Everything uses private servers (tmux: its own socket name and no configuration file; Arcade:
//! its own socket in a temporary directory). All of them are stopped on exit, on Ctrl-C and on
//! termination.

mod sys;

use std::error::Error;
use std::fs;
use std::io::{self, Read};
use std::os::unix::fs::DirBuilderExt;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::atomic::{AtomicBool, AtomicI32, AtomicU64, Ordering};
use std::sync::{Arc, Mutex};
use std::thread;
use std::time::{Duration, Instant};

use arcade_workload::{COLS, ROWS, Workload, generate, scrollback_fill};
use serde_json::{Value, json};
use sys::{Pty, now_ns};

type R<T> = Result<T, Box<dyn Error>>;

// ---------------------------------------------------------------- cleanup

enum Cleanup {
    Pid(i32),
    Tmux(String),
    Arcade { exe: PathBuf, socket: PathBuf },
    Dir(PathBuf),
}

static CLEANUPS: Mutex<Vec<Cleanup>> = Mutex::new(Vec::new());
static SIGNAL_PIPE: AtomicI32 = AtomicI32::new(-1);

fn register(item: Cleanup) {
    CLEANUPS.lock().unwrap().push(item);
}

fn unregister_pid(pid: i32) {
    CLEANUPS
        .lock()
        .unwrap()
        .retain(|c| !matches!(c, Cleanup::Pid(p) if *p == pid));
}

fn clean_env(command: &mut Command) -> &mut Command {
    command
        .env_remove("TMUX")
        .env_remove("TMUX_PANE")
        .env_remove("ARCADE")
        .env_remove("ARCADE_SOCKET")
}

/// Stop everything this run started: clients first, then servers, then the directories.
fn cleanup_all() {
    let items: Vec<Cleanup> = std::mem::take(&mut *CLEANUPS.lock().unwrap());
    for item in &items {
        if let Cleanup::Pid(pid) = item {
            // SAFETY: plain kill(2) of a process this run started and has not reaped.
            unsafe { libc::kill(*pid, libc::SIGKILL) };
        }
    }
    for item in &items {
        match item {
            Cleanup::Tmux(name) => {
                let _ = clean_env(Command::new("tmux").args([
                    "-L",
                    name,
                    "-f",
                    "/dev/null",
                    "kill-server",
                ]))
                .output();
                let tmpdir = std::env::var("TMUX_TMPDIR").unwrap_or_else(|_| "/tmp".into());
                // SAFETY: getuid cannot fail.
                let uid = unsafe { libc::getuid() };
                let _ = fs::remove_file(Path::new(&tmpdir).join(format!("tmux-{uid}")).join(name));
            }
            Cleanup::Arcade { exe, socket } => {
                let _ = clean_env(
                    Command::new(exe)
                        .arg("--socket")
                        .arg(socket)
                        .arg("kill-server"),
                )
                .output();
            }
            _ => {}
        }
    }
    for item in &items {
        if let Cleanup::Dir(dir) = item {
            let _ = fs::remove_dir_all(dir);
        }
    }
}

extern "C" fn on_signal(_: libc::c_int) {
    let fd = SIGNAL_PIPE.load(Ordering::Relaxed);
    // SAFETY: write(2) is async-signal-safe; the byte lives on the stack.
    unsafe { libc::write(fd, b"x".as_ptr().cast(), 1) };
}

/// Run `cleanup_all` and exit when the run is interrupted or terminated.
fn install_signal_cleanup() {
    let mut fds = [0; 2];
    // SAFETY: pipe fills the two descriptors.
    if unsafe { libc::pipe(fds.as_mut_ptr()) } != 0 {
        return;
    }
    SIGNAL_PIPE.store(fds[1], Ordering::Relaxed);
    // SAFETY: the handler only calls write(2).
    unsafe {
        libc::signal(libc::SIGINT, on_signal as *const () as libc::sighandler_t);
        libc::signal(libc::SIGTERM, on_signal as *const () as libc::sighandler_t);
    }
    let read_fd = fds[0];
    thread::spawn(move || {
        let mut byte = [0u8; 1];
        // SAFETY: reading one byte into a live buffer.
        unsafe { libc::read(read_fd, byte.as_mut_ptr().cast(), 1) };
        cleanup_all();
        // SAFETY: leave without running destructors of other threads.
        unsafe { libc::_exit(130) };
    });
}

struct Guard;
impl Drop for Guard {
    fn drop(&mut self) {
        cleanup_all();
    }
}

// ---------------------------------------------------------------- terminal clients

/// A program running in a terminal this run owns; stopped and forgotten when dropped.
struct Client {
    pty: Pty,
}

impl Client {
    fn open(argv: &[String]) -> R<Client> {
        let pty = sys::spawn_pty(argv, COLS, ROWS)?;
        register(Cleanup::Pid(pty.pid));
        Ok(Client { pty })
    }
}

impl Drop for Client {
    fn drop(&mut self) {
        self.pty.kill();
        unregister_pid(self.pty.pid);
    }
}

/// Reads everything a client writes, like a host terminal would, and counts it.
struct Drain {
    stop: Arc<AtomicBool>,
    bytes: Arc<AtomicU64>,
    reads: Arc<AtomicU64>,
    frames: Arc<AtomicU64>,
    handle: Option<thread::JoinHandle<()>>,
}

const SYNC_START: &[u8] = b"\x1b[?2026h";

impl Drain {
    fn start(client: &Client) -> R<Drain> {
        let mut master = client.pty.master.try_clone()?;
        let (stop, bytes, reads, frames) = (
            Arc::new(AtomicBool::new(false)),
            Arc::new(AtomicU64::new(0)),
            Arc::new(AtomicU64::new(0)),
            Arc::new(AtomicU64::new(0)),
        );
        let (s, b, r, f) = (stop.clone(), bytes.clone(), reads.clone(), frames.clone());
        let handle = thread::spawn(move || {
            let mut buf = vec![0u8; 1 << 20];
            let mut tail: Vec<u8> = Vec::new();
            while !s.load(Ordering::Relaxed) {
                let mut pfd = libc::pollfd {
                    fd: std::os::fd::AsRawFd::as_raw_fd(&master),
                    events: libc::POLLIN,
                    revents: 0,
                };
                // SAFETY: poll on one live pollfd.
                if unsafe { libc::poll(&mut pfd, 1, 100) } <= 0 {
                    continue;
                }
                match master.read(&mut buf) {
                    Ok(0) | Err(_) => return,
                    Ok(n) => {
                        b.fetch_add(n as u64, Ordering::Relaxed);
                        r.fetch_add(1, Ordering::Relaxed);
                        let mut chunk = std::mem::take(&mut tail);
                        let carried = chunk.len();
                        chunk.extend_from_slice(&buf[..n]);
                        let mut found = 0;
                        let mut i = 0;
                        while i + SYNC_START.len() <= chunk.len() {
                            if chunk[i] == 0x1b && chunk[i..].starts_with(SYNC_START) {
                                found += 1;
                                i += SYNC_START.len();
                            } else {
                                i += 1;
                            }
                        }
                        f.fetch_add(found, Ordering::Relaxed);
                        let keep = (SYNC_START.len() - 1).min(chunk.len());
                        let _ = carried;
                        tail = chunk[chunk.len() - keep..].to_vec();
                    }
                }
            }
        });
        Ok(Drain {
            stop,
            bytes,
            reads,
            frames,
            handle: Some(handle),
        })
    }

    fn snapshot(&self) -> (u64, u64, u64) {
        (
            self.bytes.load(Ordering::Relaxed),
            self.reads.load(Ordering::Relaxed),
            self.frames.load(Ordering::Relaxed),
        )
    }

    fn finish(mut self) {
        self.stop.store(true, Ordering::Relaxed);
        if let Some(handle) = self.handle.take() {
            let _ = handle.join();
        }
    }
}

/// Remove escape sequences and line breaks, so a marker can be found even when the
/// multiplexer drew it with cursor movements in between.
fn strip_escapes(input: &[u8]) -> Vec<u8> {
    let mut out = Vec::with_capacity(input.len());
    let mut i = 0;
    while i < input.len() {
        match input[i] {
            0x1b => {
                i += 1;
                match input.get(i) {
                    Some(b'[') => {
                        i += 1;
                        while i < input.len() && !(0x40..=0x7e).contains(&input[i]) {
                            i += 1;
                        }
                        i += 1;
                    }
                    Some(b']') => {
                        i += 1;
                        while i < input.len() && input[i] != 0x07 && input[i] != 0x1b {
                            i += 1;
                        }
                        if input.get(i) == Some(&0x1b) {
                            i += 1;
                        }
                        i += 1;
                    }
                    Some(b'(' | b')') => i += 2,
                    Some(_) => i += 1,
                    None => {}
                }
            }
            b'\r' | b'\n' | 0x0e | 0x0f => i += 1,
            byte => {
                out.push(byte);
                i += 1;
            }
        }
    }
    out
}

fn contains(haystack: &[u8], needle: &[u8]) -> bool {
    haystack.windows(needle.len()).any(|w| w == needle)
}

/// Read from the client until `marker` shows in the text with escape sequences removed.
/// Returns the time the chunk holding it arrived and the bytes read so far.
fn read_until(
    client: &mut Client,
    marker: &[u8],
    timeout: Duration,
    mut buffer: Vec<u8>,
) -> R<(u64, Vec<u8>)> {
    let deadline = Instant::now() + timeout;
    let mut chunk = vec![0u8; 1 << 16];
    loop {
        if contains(&strip_escapes(&buffer), marker) {
            return Ok((now_ns(), buffer));
        }
        let left = deadline.saturating_duration_since(Instant::now());
        if left.is_zero() {
            return Err(format!(
                "marker {:?} never reached the client",
                String::from_utf8_lossy(marker)
            )
            .into());
        }
        let n = client
            .pty
            .read_timeout(&mut chunk, left.as_millis().clamp(1, 1000) as i32)?;
        buffer.extend_from_slice(&chunk[..n]);
        if n > 0 && contains(&strip_escapes(&buffer), marker) {
            return Ok((now_ns(), buffer));
        }
    }
}

/// Read and throw away output until the client has been quiet for `quiet`.
fn drain_quiet(client: &mut Client, quiet: Duration) -> R<()> {
    let mut chunk = vec![0u8; 1 << 16];
    let ms = quiet.as_millis() as i32;
    while client.pty.read_timeout(&mut chunk, ms)? > 0 {}
    Ok(())
}

/// Read and throw away output for a fixed time, however busy the client is.
fn drain_for(client: &mut Client, duration: Duration) -> R<()> {
    let mut chunk = vec![0u8; 1 << 16];
    let deadline = Instant::now() + duration;
    loop {
        let left = deadline.saturating_duration_since(Instant::now());
        if left.is_zero() {
            return Ok(());
        }
        client
            .pty
            .read_timeout(&mut chunk, left.as_millis().max(1) as i32)?;
    }
}

/// Wait until `condition` holds, polling every millisecond, or fail at the deadline.
fn wait_for(what: &str, timeout: Duration, mut condition: impl FnMut() -> bool) -> R<()> {
    let deadline = Instant::now() + timeout;
    while !condition() {
        if Instant::now() >= deadline {
            return Err(format!("timed out waiting for {what}").into());
        }
        thread::sleep(Duration::from_millis(1));
    }
    Ok(())
}

// ---------------------------------------------------------------- subjects

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum Kind {
    Direct,
    Tmux,
    Arcade,
}

impl Kind {
    fn name(self) -> &'static str {
        match self {
            Kind::Direct => "direct",
            Kind::Tmux => "tmux",
            Kind::Arcade => "arcade",
        }
    }
}

static SUBJECT_COUNT: AtomicU64 = AtomicU64::new(0);

/// One private server of tmux or Arcade (or nothing, for the direct baseline).
struct Subject {
    kind: Kind,
    /// tmux socket name.
    name: String,
    /// Arcade socket path.
    socket: PathBuf,
    exe: PathBuf,
    /// Windows or sessions created so far, to give each one a unique name.
    extra: std::cell::Cell<u32>,
}

impl Subject {
    fn new(kind: Kind, arcade_exe: &Path, dir: &Path) -> Subject {
        let n = SUBJECT_COUNT.fetch_add(1, Ordering::Relaxed);
        let name = format!("arcade-bench-{}-{n}", std::process::id());
        let socket = dir.join(format!("s{n}"));
        match kind {
            Kind::Tmux => register(Cleanup::Tmux(name.clone())),
            Kind::Arcade => register(Cleanup::Arcade {
                exe: arcade_exe.to_path_buf(),
                socket: socket.clone(),
            }),
            Kind::Direct => {}
        }
        Subject {
            kind,
            name,
            socket,
            exe: arcade_exe.to_path_buf(),
            extra: std::cell::Cell::new(0),
        }
    }

    fn tmux(&self, args: &[&str]) -> R<String> {
        let out = clean_env(
            Command::new("tmux")
                .args(["-L", &self.name, "-f", "/dev/null"])
                .args(args),
        )
        .output()?;
        if !out.status.success() {
            return Err(format!("tmux {args:?}: {}", String::from_utf8_lossy(&out.stderr)).into());
        }
        Ok(String::from_utf8_lossy(&out.stdout).trim().to_string())
    }

    fn arcade(&self, args: &[&str]) -> R<String> {
        let out = clean_env(
            Command::new(&self.exe)
                .arg("--socket")
                .arg(&self.socket)
                .args(args),
        )
        .output()?;
        if !out.status.success() {
            return Err(
                format!("arcade {args:?}: {}", String::from_utf8_lossy(&out.stderr)).into(),
            );
        }
        Ok(String::from_utf8_lossy(&out.stdout).trim().to_string())
    }

    /// Start the server with a session whose pane runs `script` under `sh -c`, and no client
    /// attached. The session is called `bench`; the pane has 50,000 lines of scrollback.
    fn start(&self, script: &str) -> R<()> {
        match self.kind {
            Kind::Direct => Ok(()),
            Kind::Tmux => {
                // The history limit only applies to panes made after it is set, so the first
                // window is a placeholder and the pane under test is the second.
                self.tmux(&[
                    "new-session",
                    "-d",
                    "-s",
                    "bench",
                    "-x",
                    "120",
                    "-y",
                    "40",
                    "exec cat",
                ])?;
                self.tmux(&["set-option", "-g", "history-limit", "50000"])?;
                self.tmux(&["new-window", "-t", "bench", script])?;
                Ok(())
            }
            Kind::Arcade => self.arcade_session("bench", script),
        }
    }

    /// Create an Arcade session through a client and let the client go: the session stays.
    fn arcade_session(&self, name: &str, script: &str) -> R<()> {
        let mut argv = self.arcade_argv(&["new", "-s", name]);
        argv.extend(["sh".into(), "-c".into(), script.into()]);
        let client = Client::open(&argv)?;
        wait_for("the session to exist", Duration::from_secs(20), || {
            self.arcade(&["ls"])
                .map(|text| text.contains(name))
                .unwrap_or(false)
        })?;
        drop(client);
        Ok(())
    }

    fn arcade_argv(&self, args: &[&str]) -> Vec<String> {
        let mut argv = vec![
            self.exe.to_string_lossy().into_owned(),
            "--socket".into(),
            self.socket.to_string_lossy().into_owned(),
        ];
        argv.extend(args.iter().map(|s| s.to_string()));
        argv
    }

    fn attach_argv(&self) -> Vec<String> {
        match self.kind {
            Kind::Tmux => [
                "tmux",
                "-L",
                &self.name,
                "-f",
                "/dev/null",
                "attach-session",
                "-t",
                "bench",
            ]
            .map(String::from)
            .to_vec(),
            Kind::Arcade => self.arcade_argv(&["attach", "-t", "bench"]),
            Kind::Direct => unreachable!("the direct baseline has nothing to attach to"),
        }
    }

    fn server_pid(&self) -> R<i32> {
        match self.kind {
            Kind::Tmux => Ok(self.tmux(&["display-message", "-p", "#{pid}"])?.parse()?),
            Kind::Arcade => {
                let socket = self.socket.to_string_lossy().into_owned();
                sys::find_pid(&[&socket, "__daemon"])
                    .ok_or_else(|| "the Arcade daemon is not running".into())
            }
            Kind::Direct => Err("no server".into()),
        }
    }

    /// Another pane that only waits, for the memory test of many idle panes.
    fn add_idle_pane(&self) -> R<()> {
        let n = self.extra.get() + 1;
        self.extra.set(n);
        match self.kind {
            Kind::Tmux => self
                .tmux(&["new-window", "-d", "-t", "bench", "exec cat"])
                .map(|_| ()),
            Kind::Arcade => self.arcade_session(&format!("idle{n}"), "exec cat"),
            Kind::Direct => Ok(()),
        }
    }

    /// A short command sent to the running server, timed by the caller.
    fn command_argv(&self) -> Vec<String> {
        match self.kind {
            Kind::Tmux => [
                "tmux",
                "-L",
                &self.name,
                "-f",
                "/dev/null",
                "display-message",
                "-p",
                "x",
            ]
            .map(String::from)
            .to_vec(),
            _ => self.arcade_argv(&["ls"]),
        }
    }

    fn stop(&self) {
        match self.kind {
            Kind::Tmux => {
                let _ = self.tmux(&["kill-server"]);
            }
            Kind::Arcade => {
                let _ = self.arcade(&["kill-server"]);
                let _ = wait_for("the daemon to leave", Duration::from_secs(10), || {
                    sys::find_pid(&[&self.socket.to_string_lossy(), "__daemon"]).is_none()
                });
            }
            Kind::Direct => {}
        }
    }
}

// ---------------------------------------------------------------- setup

struct Config {
    quick: bool,
    subjects: Vec<Kind>,
    tests: Vec<String>,
    arcade_exe: PathBuf,
    dir: PathBuf,
    data_dir: PathBuf,
    out_dir: PathBuf,
}

impl Config {
    fn runs(&self, test: &str) -> bool {
        self.tests.is_empty() || self.tests.iter().any(|t| t == test)
    }
    fn has(&self, kind: Kind) -> bool {
        self.subjects.contains(&kind)
    }
}

fn shell_quote(path: &Path) -> String {
    format!("'{}'", path.to_string_lossy().replace('\'', "'\\''"))
}

fn percentile(sorted: &[u64], p: f64) -> u64 {
    sorted[((p / 100.0) * (sorted.len() - 1) as f64).round() as usize]
}

fn summary_us(values_ns: &[u64]) -> Value {
    let mut v = values_ns.to_vec();
    v.sort_unstable();
    let us = |x: u64| (x as f64 / 100.0).round() / 10.0;
    json!({
        "p50_us": us(percentile(&v, 50.0)),
        "p99_us": us(percentile(&v, 99.0)),
        "max_us": us(*v.last().unwrap()),
        "samples": v.len(),
    })
}

fn round1(x: f64) -> f64 {
    (x * 10.0).round() / 10.0
}

fn workload_files(config: &Config) -> R<Vec<(String, PathBuf, u64)>> {
    fs::create_dir_all(&config.data_dir)?;
    let mut files = Vec::new();
    for workload in Workload::ALL {
        let megabytes = if config.quick {
            32
        } else if workload == Workload::Plain {
            1000
        } else {
            256
        };
        let path = config
            .data_dir
            .join(format!("{}-{megabytes}.bin", workload.name()));
        if !path.exists() {
            eprintln!("generating {}", path.display());
            fs::write(&path, generate(workload, megabytes * 1_000_000, 1))?;
        }
        files.push((
            workload.name().to_string(),
            path.clone(),
            fs::metadata(&path)?.len(),
        ));
    }
    Ok(files)
}

fn fill_file(config: &Config) -> R<PathBuf> {
    fs::create_dir_all(&config.data_dir)?;
    let path = config.data_dir.join("scrollback-fill-50000.bin");
    if !path.exists() {
        fs::write(&path, scrollback_fill(50_000, COLS as usize, 1))?;
    }
    Ok(path)
}

// ---------------------------------------------------------------- throughput

fn throughput_direct(path: &Path, size: u64) -> R<Value> {
    let script = format!("stty -opost; cat {}", shell_quote(path));
    let mut client = Client::open(&["sh".into(), "-c".into(), script])?;
    let mut buf = vec![0u8; 1 << 20];
    let start = now_ns();
    while client.pty.read_timeout(&mut buf, 1000).is_ok() {}
    let seconds = (now_ns() - start) as f64 / 1e9;
    Ok(
        json!({ "seconds": round1(seconds * 10.0) / 10.0, "mb_per_s": round1(size as f64 / 1e6 / seconds) }),
    )
}

fn throughput_subject(config: &Config, kind: Kind, path: &Path, size: u64) -> R<Value> {
    let subject = Subject::new(kind, &config.arcade_exe, &config.dir);
    let go = config.dir.join(format!("go-{}", subject.name));
    let done = config.dir.join(format!("done-{}", subject.name));
    let script = format!(
        "while [ ! -e {go} ]; do sleep 0.005; done; stty -opost; cat {file}; touch {done}; exec cat",
        go = shell_quote(&go),
        file = shell_quote(path),
        done = shell_quote(&done)
    );
    subject.start(&script)?;
    let mut client = Client::open(&subject.attach_argv())?;
    let drain = Drain::start(&client)?;
    let _ = &mut client;
    wait_for("the first frame", Duration::from_secs(20), || {
        drain.snapshot().0 > 0
    })?;
    thread::sleep(Duration::from_millis(300));
    let before = drain.snapshot();
    let start = now_ns();
    fs::write(&go, b"")?;
    wait_for("the data to be printed", Duration::from_secs(900), || {
        done.exists()
    })?;
    let seconds = (now_ns() - start) as f64 / 1e9;
    thread::sleep(Duration::from_millis(500));
    let after = drain.snapshot();
    let pid = subject.server_pid()?;
    let result = json!({
        "megabytes": (size as f64 / 1e6).round(),
        "seconds": (seconds * 1000.0).round() / 1000.0,
        "mb_per_s": round1(size as f64 / 1e6 / seconds),
        "client_received_mb": round1((after.0 - before.0) as f64 / 1e6),
        "client_writes": after.1 - before.1,
        "frames_per_second_during_run": round1((after.2 - before.2) as f64 / seconds),
        "server_footprint_mb_after": round1(sys::footprint_mb(pid)),
        "server_rss_mb_after": round1(sys::rss_mb(pid)),
    });
    drain.finish();
    drop(client);
    subject.stop();
    let _ = fs::remove_file(&go);
    let _ = fs::remove_file(&done);
    Ok(result)
}

fn run_throughput(config: &Config) -> R<Value> {
    let files = workload_files(config)?;
    let mut out = serde_json::Map::new();
    for kind in &config.subjects {
        let mut per = serde_json::Map::new();
        for (name, path, size) in &files {
            let value = match kind {
                Kind::Direct => throughput_direct(path, *size)?,
                _ => throughput_subject(config, *kind, path, *size)?,
            };
            println!(
                "  {:<7} {:<13} {:>8} MB/s",
                kind.name(),
                name,
                value["mb_per_s"]
            );
            per.insert(name.clone(), value);
        }
        out.insert(kind.name().into(), Value::Object(per));
    }
    Ok(Value::Object(out))
}

// ---------------------------------------------------------------- latency

/// The program that runs inside the pane for the latency test: for every byte it reads it stamps
/// the arrival, writes a numbered token, and stamps the write. Run as `arcade-bench probe <log>`.
fn probe_main(log: &str) -> R<()> {
    // SAFETY: tcgetattr/cfmakeraw/tcsetattr on the terminal this process is attached to.
    unsafe {
        let mut t: libc::termios = std::mem::zeroed();
        if libc::tcgetattr(0, &mut t) == 0 {
            libc::cfmakeraw(&mut t);
            libc::tcsetattr(0, libc::TCSANOW, &t);
        }
    }
    let mut records: Vec<(u64, u64, u64)> = Vec::new();
    let mut n: u64 = 0;
    loop {
        let mut byte = [0u8; 1];
        // SAFETY: one byte into a live buffer.
        let got = unsafe { libc::read(0, byte.as_mut_ptr().cast(), 1) };
        let t1 = now_ns();
        if got <= 0 || byte[0] == b'Q' {
            break;
        }
        let token = format!("<{n:05}>");
        let t2 = now_ns();
        // SAFETY: writing a live buffer to stdout.
        unsafe { libc::write(1, token.as_ptr().cast(), token.len()) };
        records.push((n, t1, t2));
        n += 1;
        if n.is_multiple_of(12) {
            // SAFETY: as above.
            unsafe { libc::write(1, b"\r\n".as_ptr().cast(), 2) };
        }
    }
    let text: String = records
        .iter()
        .map(|(n, a, b)| format!("{n} {a} {b}\n"))
        .collect();
    fs::write(log, text)?;
    Ok(())
}

fn probe_script(config: &Config, log: &Path) -> String {
    let exe = std::env::current_exe().unwrap_or_default();
    let _ = config;
    format!(
        "printf PROBE-READY; exec {} probe {}",
        shell_quote(&exe),
        shell_quote(log)
    )
}

fn latency_run(client: &mut Client, log: &Path, samples: usize) -> R<Value> {
    read_until(client, b"PROBE-READY", Duration::from_secs(20), Vec::new())?;
    latency_measure(client, log, samples)
}

/// The measurement itself, once the probe is known to be running in the pane that has focus.
fn latency_measure(client: &mut Client, log: &Path, samples: usize) -> R<Value> {
    Ok(latency_samples(client, log, samples)?.0)
}

/// The measurement, with the time every key was written and every echo seen (nanoseconds on the
/// monotonic clock), for the stage-timing run to line its marks up with.
fn latency_samples(
    client: &mut Client,
    log: &Path,
    samples: usize,
) -> R<(Value, Vec<u64>, Vec<u64>)> {
    drain_for(client, Duration::from_millis(300))?;
    let mut sent = Vec::with_capacity(samples);
    let mut seen = Vec::with_capacity(samples);
    let mut buffer: Vec<u8> = Vec::new();
    for n in 0..samples {
        let t0 = now_ns();
        client.pty.write_all(b"a")?;
        let token = format!("<{n:05}>");
        let (t3, rest) = read_until(
            client,
            token.as_bytes(),
            Duration::from_secs(10),
            std::mem::take(&mut buffer),
        )?;
        sent.push(t0);
        seen.push(t3);
        buffer = rest[rest.len().saturating_sub(256)..].to_vec();
        thread::sleep(Duration::from_micros(4000 + (n as u64 % 7) * 1000));
    }
    client.pty.write_all(b"Q")?;
    wait_for("the probe log", Duration::from_secs(10), || {
        fs::metadata(log).map(|m| m.len() > 0).unwrap_or(false)
    })?;
    let text = fs::read_to_string(log)?;
    let rows: Vec<(u64, u64, u64)> = text
        .lines()
        .filter_map(|l| {
            let mut p = l.split_whitespace().map(|x| x.parse::<u64>().ok());
            Some((p.next()??, p.next()??, p.next()??))
        })
        .collect();
    if rows.len() < samples {
        return Err(format!("probe logged {} of {samples} keys", rows.len()).into());
    }
    let key_in: Vec<u64> = (0..samples)
        .map(|n| rows[n].1.saturating_sub(sent[n]))
        .collect();
    let echo_out: Vec<u64> = (0..samples)
        .map(|n| seen[n].saturating_sub(rows[n].2))
        .collect();
    let round_trip: Vec<u64> = (0..samples).map(|n| seen[n] - sent[n]).collect();
    let summary = json!({
        "key_to_pane": summary_us(&key_in),
        "pane_to_client": summary_us(&echo_out),
        "round_trip": summary_us(&round_trip),
    });
    Ok((summary, sent, seen))
}

fn run_latency(config: &Config) -> R<Value> {
    let samples = if config.quick { 200 } else { 1000 };
    let log = config.dir.join("probe.log");
    let script = probe_script(config, &log);
    let mut out = serde_json::Map::new();
    let flood_file = config.data_dir.join(if config.quick {
        "plain-32.bin"
    } else {
        "plain-1000.bin"
    });

    for kind in &config.subjects {
        let _ = fs::remove_file(&log);
        match kind {
            Kind::Direct => {
                let mut client = Client::open(&["sh".into(), "-c".into(), script.clone()])?;
                out.insert("direct".into(), latency_run(&mut client, &log, samples)?);
            }
            _ => {
                let subject = Subject::new(*kind, &config.arcade_exe, &config.dir);
                subject.start(&script)?;
                let mut client = Client::open(&subject.attach_argv())?;
                out.insert(
                    format!("{}_idle", kind.name()),
                    latency_run(&mut client, &log, samples)?,
                );
                drop(client);
                subject.stop();

                if *kind == Kind::Tmux {
                    // Same measurement while a second, visible pane floods output without pause.
                    let _ = fs::remove_file(&log);
                    let subject = Subject::new(*kind, &config.arcade_exe, &config.dir);
                    subject.start(&script)?;
                    if !flood_file.exists() {
                        workload_files(config)?;
                    }
                    let flood = format!(
                        "stty -opost; while :; do cat {}; done",
                        shell_quote(&flood_file)
                    );
                    subject.tmux(&["split-window", "-d", "-t", "bench", &flood])?;
                    let mut client = Client::open(&subject.attach_argv())?;
                    out.insert(
                        "tmux_other_pane_flooding".into(),
                        latency_run(&mut client, &log, samples)?,
                    );
                    drop(client);
                    subject.stop();
                }

                if *kind == Kind::Arcade {
                    // The same, with a second, visible pane that floods output without pause.
                    // The pane is made after the client attached (Arcade has no way to give
                    // the new pane a command), the flood is typed into it, and the focus goes
                    // back to the pane with the probe.
                    let _ = fs::remove_file(&log);
                    let subject = Subject::new(*kind, &config.arcade_exe, &config.dir);
                    subject.start(&script)?;
                    if !flood_file.exists() {
                        workload_files(config)?;
                    }
                    let mut client = Client::open(&subject.attach_argv())?;
                    read_until(
                        &mut client,
                        b"PROBE-READY",
                        Duration::from_secs(20),
                        Vec::new(),
                    )?;
                    subject.arcade(&["run", "pane.split-down"])?;
                    subject.arcade(&["run", "pane.focus-down"])?;
                    let flood = format!(
                        "stty -opost; while :; do cat {}; done\r",
                        shell_quote(&flood_file)
                    );
                    // A terminal always reads what it is sent.
                    drain_for(&mut client, Duration::from_millis(1500))?;
                    client.pty.write_all(flood.as_bytes())?;
                    drain_for(&mut client, Duration::from_millis(1500))?;
                    subject.arcade(&["run", "pane.focus-up"])?;
                    out.insert(
                        "arcade_other_pane_flooding".into(),
                        latency_measure(&mut client, &log, samples)?,
                    );
                    drop(client);
                    subject.stop();
                }
            }
        }
    }
    for (name, value) in &out {
        println!(
            "  {name:<26} round trip p50 {} us, p99 {} us",
            value["round_trip"]["p50_us"], value["round_trip"]["p99_us"]
        );
    }
    Ok(Value::Object(out))
}

// ---------------------------------------------------------------- stages

/// One mark of the stage-timing build: point, nanoseconds, then reads, writes and polls so far.
#[derive(Clone, Copy)]
struct Mark {
    point: u8,
    ns: u64,
    calls: [u32; 3],
}

/// The points, as numbered in the daemon's `stage::Point`.
const CLIENT_KEY_READ: u8 = 1;
const CLIENT_KEY_SENT: u8 = 2;
const DAEMON_WAKE: u8 = 3;
const DAEMON_SOCKET_READ: u8 = 4;
const DAEMON_DECODED: u8 = 5;
const DAEMON_KEY_HANDLED: u8 = 6;
const DAEMON_PTY_WRITTEN: u8 = 7;
const DAEMON_PTY_READ: u8 = 8;
const DAEMON_FED: u8 = 9;
const DAEMON_SNAPSHOT: u8 = 10;
const DAEMON_COMPOSED: u8 = 11;
const DAEMON_ENCODED: u8 = 12;
const DAEMON_SOCKET_WRITTEN: u8 = 13;
const CLIENT_WAKE: u8 = 14;
const CLIENT_STDOUT_WRITTEN: u8 = 15;
const DAEMON_FRAME_START: u8 = 16;
const DAEMON_BEFORE_READ: u8 = 17;
const CLIENT_BEFORE_WRITE: u8 = 18;

fn read_marks(path: &Path) -> Vec<Mark> {
    let Ok(text) = fs::read_to_string(path) else {
        return Vec::new();
    };
    text.lines()
        .filter_map(|line| {
            let mut p = line.split_whitespace().map(|x| x.parse::<u64>().ok());
            Some(Mark {
                point: p.next()?? as u8,
                ns: p.next()??,
                calls: [p.next()?? as u32, p.next()?? as u32, p.next()?? as u32],
            })
        })
        .collect()
}

/// The first mark of `point` at or after `from`.
fn first_after(marks: &[Mark], point: u8, from: u64) -> Option<Mark> {
    marks
        .iter()
        .copied()
        .find(|m| m.point == point && m.ns >= from)
}

/// The last mark of `point` at or before `until` and not before `from`.
fn last_between(marks: &[Mark], point: u8, from: u64, until: u64) -> Option<Mark> {
    marks
        .iter()
        .copied()
        .rev()
        .find(|m| m.point == point && m.ns <= until && m.ns >= from)
}

/// The stretches between points that a key and its echo pass, in order.
const STRETCHES: [(&str, &str); 19] = [
    (
        "bench_write_to_client_read",
        "operating system: the test's terminal to the client's read",
    ),
    (
        "client_read_to_write_call",
        "client: key read, message encoded, up to the write call",
    ),
    ("client_write_call", "client: the write call to the socket"),
    (
        "sent_to_daemon_wake",
        "operating system: socket write to the daemon's wakeup",
    ),
    (
        "daemon_wake_to_read_call",
        "daemon: wakeup to the first read call (events, tables, checks)",
    ),
    ("daemon_read_call", "daemon: the read call(s) on the socket"),
    ("socket_read_to_decoded", "daemon: message decoded"),
    ("decoded_to_key_handled", "daemon: key found in the tables"),
    (
        "key_handled_to_pty_written",
        "daemon: key written to the pane's terminal",
    ),
    (
        "pane_program",
        "pane program (not Arcade): read the key, wrote the echo, wakeup",
    ),
    ("wake_to_pty_read", "daemon: wakeup to read of the echo"),
    (
        "pty_read_to_fed",
        "daemon: terminal emulation takes the echo in",
    ),
    (
        "fed_to_frame_start",
        "daemon: rest of the turn before the frame is begun",
    ),
    ("frame_start_to_snapshot", "daemon: panes' screens read out"),
    ("snapshot_to_composed", "daemon: grid composed"),
    (
        "composed_to_encoded",
        "daemon: difference found and frame encoded",
    ),
    (
        "encoded_to_client_wake",
        "daemon write plus operating system: to the client's wakeup",
    ),
    (
        "client_wake_to_stdout",
        "client: frame written to its terminal",
    ),
    (
        "stdout_to_bench_read",
        "operating system: the client's terminal to the test's read",
    ),
];

/// Cut the files of a stage-timing run into the stretches of every key and print the medians.
fn stages_table(dir: &Path, socket_name: &str, sent: &[u64], seen: &[u64]) -> R<Value> {
    let keys = sent.len();
    let mut daemon = Vec::new();
    let mut client = Vec::new();
    for entry in fs::read_dir(dir)? {
        let path = entry?.path();
        let name = path
            .file_name()
            .map(|n| n.to_string_lossy().to_string())
            .unwrap_or_default();
        if name == format!("{socket_name}.stages-daemon") {
            daemon = read_marks(&path);
        } else if name.starts_with(&format!("{socket_name}.stages-client-")) {
            let marks = read_marks(&path);
            // The client that attached is the one that read keys.
            if marks.iter().any(|m| m.point == CLIENT_KEY_READ) {
                client = marks;
            }
        }
    }
    if daemon.is_empty() || client.is_empty() {
        let listing: Vec<String> = fs::read_dir(dir)?
            .filter_map(|e| e.ok().map(|e| e.file_name().to_string_lossy().to_string()))
            .collect();
        return Err(format!(
            "the stage-timing files are missing (daemon {} marks, client {} marks; the directory holds {listing:?}): is the build made with the feature?",
            daemon.len(),
            client.len()
        )
        .into());
    }
    let key_reads: Vec<Mark> = client
        .iter()
        .copied()
        .filter(|m| m.point == CLIENT_KEY_READ)
        .collect();
    let mut columns: Vec<Vec<u64>> = vec![Vec::new(); STRETCHES.len()];
    let mut calls: [Vec<u64>; 6] = Default::default();
    let mut used = 0;
    for (i, k1) in key_reads.iter().enumerate().take(keys) {
        let end = key_reads.get(i + 1).map_or(u64::MAX, |m| m.ns);
        let chain = (|| {
            let kw = first_after(&client, CLIENT_BEFORE_WRITE, k1.ns)?;
            let k2 = first_after(&client, CLIENT_KEY_SENT, kw.ns)?;
            let k4 = first_after(&daemon, DAEMON_SOCKET_READ, k1.ns)?;
            let kr = last_between(&daemon, DAEMON_BEFORE_READ, k1.ns, k4.ns)?;
            let k3 = last_between(&daemon, DAEMON_WAKE, k1.ns, kr.ns)?;
            let k5 = first_after(&daemon, DAEMON_DECODED, k4.ns)?;
            let k6 = first_after(&daemon, DAEMON_KEY_HANDLED, k5.ns)?;
            let k7 = first_after(&daemon, DAEMON_PTY_WRITTEN, k6.ns)?;
            let o2 = first_after(&daemon, DAEMON_PTY_READ, k7.ns)?;
            let w2 = last_between(&daemon, DAEMON_WAKE, k7.ns, o2.ns)?;
            let o3 = first_after(&daemon, DAEMON_FED, o2.ns)?;
            let o3b = first_after(&daemon, DAEMON_FRAME_START, o3.ns)?;
            let o4 = first_after(&daemon, DAEMON_SNAPSHOT, o3b.ns)?;
            let o5 = first_after(&daemon, DAEMON_COMPOSED, o4.ns)?;
            let o6 = first_after(&daemon, DAEMON_ENCODED, o5.ns)?;
            let o7 = first_after(&daemon, DAEMON_SOCKET_WRITTEN, o6.ns)?;
            let c1 = first_after(&client, CLIENT_WAKE, o6.ns)?;
            let c2 = first_after(&client, CLIENT_STDOUT_WRITTEN, c1.ns)?;
            if c2.ns >= end {
                return None;
            }
            Some((
                kw, kr, k2, k3, k4, k5, k6, k7, w2, o2, o3, o3b, o4, o5, o6, o7, c1, c2,
            ))
        })();
        let Some((kw, kr, k2, k3, k4, k5, k6, k7, w2, o2, o3, o3b, o4, o5, o6, o7, c1, c2)) = chain
        else {
            continue;
        };
        used += 1;
        let d = |a: u64, b: u64| b.saturating_sub(a);
        let values = [
            d(sent[i], k1.ns),
            d(k1.ns, kw.ns),
            d(kw.ns, k2.ns),
            d(k2.ns, k3.ns),
            d(k3.ns, kr.ns),
            d(kr.ns, k4.ns),
            d(k4.ns, k5.ns),
            d(k5.ns, k6.ns),
            d(k6.ns, k7.ns),
            d(k7.ns, w2.ns),
            d(w2.ns, o2.ns),
            d(o2.ns, o3.ns),
            d(o3.ns, o3b.ns),
            d(o3b.ns, o4.ns),
            d(o4.ns, o5.ns),
            d(o5.ns, o6.ns),
            d(o6.ns, c1.ns),
            d(c1.ns, c2.ns),
            d(c2.ns, seen[i]),
        ];
        for (column, value) in columns.iter_mut().zip(values) {
            column.push(value);
        }
        // System calls of this key: the client from the key read to the echo on its terminal,
        // the daemon from the wakeup that took the key to the write of the frame.
        let diff = |a: &Mark, b: &Mark, n: usize| u64::from(b.calls[n].saturating_sub(a.calls[n]));
        for n in 0..3 {
            calls[n].push(diff(k1, &c2, n));
            calls[3 + n].push(diff(&k3, &o7, n));
        }
    }
    if used == 0 {
        return Err("no key could be followed through all the points".into());
    }
    let names = ["read", "write", "poll"];
    let mut stretches = serde_json::Map::new();
    println!("  {used} of {keys} keys followed through every point");
    println!("  {:<28} {:>8} {:>8}", "stretch", "p50 us", "p90 us");
    let mut arcade_total = 0.0;
    let mut total_with_pane = 0.0;
    for ((key, what), column) in STRETCHES.iter().zip(&mut columns) {
        column.sort_unstable();
        let p50 = percentile(column, 50.0) as f64 / 1000.0;
        let p90 = percentile(column, 90.0) as f64 / 1000.0;
        println!("  {key:<28} {p50:>8.1} {p90:>8.1}   {what}");
        total_with_pane += p50;
        if *key != "pane_program" {
            arcade_total += p50;
        }
        stretches.insert(
            (*key).into(),
            json!({"p50_us": round1(p50), "p90_us": round1(p90), "what": what}),
        );
    }
    println!("  sum of the stretches except the pane program: {arcade_total:.1} us");
    println!("  sum of all the stretches: {total_with_pane:.1} us");
    let mut per_key = serde_json::Map::new();
    for (n, name) in names.iter().enumerate() {
        calls[n].sort_unstable();
        calls[3 + n].sort_unstable();
        let (c, dm) = (percentile(&calls[n], 50.0), percentile(&calls[3 + n], 50.0));
        println!("  per key, system calls: client {name} {c}, daemon {name} {dm}");
        per_key.insert(format!("client_{name}"), json!(c));
        per_key.insert(format!("daemon_{name}"), json!(dm));
    }
    Ok(json!({
        "keys_followed": used,
        "stretches": stretches,
        "sum_without_pane_program_us": round1(arcade_total),
        "sum_of_all_us": round1(total_with_pane),
        "system_calls_per_key": per_key,
    }))
}

/// Build the stage-timing binary apart from the normal one, run the latency probe against it,
/// and cut the marks it wrote into the stretches of every key.
fn run_stages(config: &Config) -> R<Value> {
    let root = find_root();
    let status = Command::new("cargo")
        .args([
            "build",
            "--release",
            "-q",
            "-p",
            "arcade",
            "--features",
            "stage-timing",
            "--target-dir",
        ])
        .arg(root.join("target/stage-timing"))
        .current_dir(&root)
        .status()?;
    if !status.success() {
        return Err("cannot build the stage-timing binary of arcade".into());
    }
    let exe = root.join("target/stage-timing/release/arcade");
    let samples = if config.quick { 200 } else { 1000 };
    let log = config.dir.join("probe.log");
    let script = probe_script(config, &log);
    let _ = fs::remove_file(&log);
    let subject = Subject::new(Kind::Arcade, &exe, &config.dir);
    subject.start(&script)?;
    let mut client = Client::open(&subject.attach_argv())?;
    read_until(
        &mut client,
        b"PROBE-READY",
        Duration::from_secs(20),
        Vec::new(),
    )?;
    let (latency, sent, seen) = latency_samples(&mut client, &log, samples)?;
    let socket_name = subject
        .socket
        .file_name()
        .map(|n| n.to_string_lossy().to_string())
        .unwrap_or_default();
    // The probe ended the session, so the client leaves by itself and writes its marks; wait
    // for them before letting the client go.
    let prefix = format!("{socket_name}.stages-client-");
    wait_for("the client's marks", Duration::from_secs(10), || {
        fs::read_dir(&config.dir).is_ok_and(|mut d| {
            d.any(|e| e.is_ok_and(|e| e.file_name().to_string_lossy().starts_with(&prefix)))
        })
    })?;
    drop(client);
    subject.stop();
    let table = stages_table(&config.dir, &socket_name, &sent, &seen)?;
    println!(
        "  the latency test of the same run: round trip p50 {} us (key to pane {}, pane to client {})",
        latency["round_trip"]["p50_us"],
        latency["key_to_pane"]["p50_us"],
        latency["pane_to_client"]["p50_us"]
    );
    Ok(json!({"latency": latency, "stages": table}))
}

// ---------------------------------------------------------------- idle

fn run_idle(config: &Config) -> R<Value> {
    let seconds = if config.quick { 10 } else { 60 };
    let mut out = serde_json::Map::new();
    for kind in config.subjects.iter().filter(|k| **k != Kind::Direct) {
        let subject = Subject::new(*kind, &config.arcade_exe, &config.dir);
        subject.start("exec cat")?;
        let client = Client::open(&subject.attach_argv())?;
        let drain = Drain::start(&client)?;
        let server = subject.server_pid()?;
        wait_for("the first frame", Duration::from_secs(20), || {
            drain.snapshot().0 > 0
        })?;
        thread::sleep(Duration::from_secs(2));
        let (cpu_s0, cpu_c0) = (sys::cpu_seconds(server), sys::cpu_seconds(client.pty.pid));
        let (csw_s0, csw_c0) = (
            sys::context_switches(server),
            sys::context_switches(client.pty.pid),
        );
        let b0 = drain.snapshot();
        thread::sleep(Duration::from_secs(seconds));
        let b1 = drain.snapshot();
        let (cpu_s1, cpu_c1) = (sys::cpu_seconds(server), sys::cpu_seconds(client.pty.pid));
        let (csw_s1, csw_c1) = (
            sys::context_switches(server),
            sys::context_switches(client.pty.pid),
        );
        let diff = |a: Option<u64>, b: Option<u64>| a.zip(b).map(|(a, b)| b.saturating_sub(a));
        let value = json!({
            "window_seconds": seconds,
            "server_cpu_seconds": round1((cpu_s1 - cpu_s0) * 100.0) / 100.0,
            "client_cpu_seconds": round1((cpu_c1 - cpu_c0) * 100.0) / 100.0,
            "server_context_switches": diff(csw_s0, csw_s1),
            "client_context_switches": diff(csw_c0, csw_c1),
            "bytes_sent_to_host_terminal": b1.0 - b0.0,
            "writes_to_host_terminal": b1.1 - b0.1,
        });
        println!("  {:<7} {value}", kind.name());
        out.insert(kind.name().into(), value);
        drain.finish();
        drop(client);
        subject.stop();
    }
    Ok(Value::Object(out))
}

// ---------------------------------------------------------------- memory

fn mem_sample(pid: i32) -> Value {
    json!({ "footprint_mb": round1(sys::footprint_mb(pid)), "rss_mb": round1(sys::rss_mb(pid)) })
}

fn run_memory(config: &Config) -> R<Value> {
    let fill = fill_file(config)?;
    let mut out = serde_json::Map::new();
    for kind in config.subjects.iter().filter(|k| **k != Kind::Direct) {
        let mut result = serde_json::Map::new();

        let subject = Subject::new(*kind, &config.arcade_exe, &config.dir);
        subject.start("exec cat")?;
        let pid = subject.server_pid()?;
        thread::sleep(Duration::from_millis(500));
        result.insert("one_idle_pane".into(), mem_sample(pid));
        subject.stop();

        let subject = Subject::new(*kind, &config.arcade_exe, &config.dir);
        let go = config.dir.join(format!("go-{}", subject.name));
        let done = config.dir.join(format!("done-{}", subject.name));
        let script = format!(
            "while [ ! -e {go} ]; do sleep 0.005; done; stty -opost; cat {file}; touch {done}; exec cat",
            go = shell_quote(&go),
            file = shell_quote(&fill),
            done = shell_quote(&done)
        );
        subject.start(&script)?;
        let pid = subject.server_pid()?;
        fs::write(&go, b"")?;
        wait_for("the scrollback to fill", Duration::from_secs(300), || {
            done.exists()
        })?;
        thread::sleep(Duration::from_millis(500));
        result.insert("after_50000_lines_just_after".into(), mem_sample(pid));
        thread::sleep(Duration::from_secs(3));
        result.insert("after_50000_lines_3s_idle".into(), mem_sample(pid));
        if *kind == Kind::Tmux {
            let lines =
                subject.tmux(&["display-message", "-p", "-t", "bench", "#{history_size}"])?;
            result.insert(
                "history_lines_kept".into(),
                json!(lines.parse::<u64>().unwrap_or(0)),
            );
        }
        subject.stop();
        let _ = fs::remove_file(&go);
        let _ = fs::remove_file(&done);

        let subject = Subject::new(*kind, &config.arcade_exe, &config.dir);
        subject.start("exec cat")?;
        for _ in 0..19 {
            subject.add_idle_pane()?;
        }
        thread::sleep(Duration::from_secs(1));
        result.insert(
            "twenty_idle_panes".into(),
            mem_sample(subject.server_pid()?),
        );
        subject.stop();

        println!("  {:<7} {}", kind.name(), Value::Object(result.clone()));
        out.insert(kind.name().into(), Value::Object(result));
    }
    Ok(Value::Object(out))
}

// ---------------------------------------------------------------- startup

fn run_startup(config: &Config) -> R<Value> {
    let rounds = if config.quick { 10 } else { 40 };
    let mut out = serde_json::Map::new();
    for kind in config.subjects.iter().filter(|k| **k != Kind::Direct) {
        let mut result = serde_json::Map::new();
        let subject = Subject::new(*kind, &config.arcade_exe, &config.dir);
        subject.start("printf READY-MARK; exec cat")?;
        thread::sleep(Duration::from_millis(500));

        let mut times = Vec::new();
        for _ in 0..rounds {
            let t0 = now_ns();
            let mut client = Client::open(&subject.attach_argv())?;
            let (t1, _) = read_until(
                &mut client,
                b"READY-MARK",
                Duration::from_secs(10),
                Vec::new(),
            )?;
            times.push(t1 - t0);
            drop(client);
            thread::sleep(Duration::from_millis(50));
        }
        result.insert("attach_to_first_frame".into(), summary_us(&times));

        let mut times = Vec::new();
        for _ in 0..rounds {
            let argv = subject.command_argv();
            let t0 = now_ns();
            let _ = clean_env(Command::new(&argv[0]).args(&argv[1..])).output()?;
            times.push(now_ns() - t0);
        }
        result.insert("command_line_round_trip".into(), summary_us(&times));

        if *kind == Kind::Tmux {
            // Leave exactly two windows, each showing its own text: drop the placeholder.
            subject.tmux(&["kill-window", "-t", "bench:0"])?;
            subject.tmux(&[
                "new-window",
                "-d",
                "-t",
                "bench",
                "printf SECOND-WINDOW; exec cat",
            ])?;
            let mut client = Client::open(&subject.attach_argv())?;
            let mut times = Vec::new();
            for index in 0..rounds {
                let marker: &[u8] = if index % 2 == 0 {
                    b"SECOND-WINDOW"
                } else {
                    b"READY-MARK"
                };
                drain_quiet(&mut client, Duration::from_millis(50))?;
                let t0 = now_ns();
                client.pty.write_all(b"\x02n")?;
                let (t1, _) = read_until(&mut client, marker, Duration::from_secs(10), Vec::new())?;
                times.push(t1 - t0);
            }
            result.insert("switch_window_to_frame".into(), summary_us(&times));
        }
        subject.stop();

        let mut times = Vec::new();
        for _ in 0..rounds {
            let fresh = Subject::new(*kind, &config.arcade_exe, &config.dir);
            let argv = match kind {
                Kind::Tmux => [
                    "tmux",
                    "-L",
                    &fresh.name,
                    "-f",
                    "/dev/null",
                    "new-session",
                    "printf READY-MARK; exec cat",
                ]
                .map(String::from)
                .to_vec(),
                _ => fresh.arcade_argv(&[
                    "new",
                    "-s",
                    "bench",
                    "sh",
                    "-c",
                    "printf READY-MARK; exec cat",
                ]),
            };
            let t0 = now_ns();
            let mut client = Client::open(&argv)?;
            let (t1, _) = read_until(
                &mut client,
                b"READY-MARK",
                Duration::from_secs(10),
                Vec::new(),
            )?;
            times.push(t1 - t0);
            drop(client);
            fresh.stop();
        }
        result.insert(
            "start_server_and_first_session_to_first_frame".into(),
            summary_us(&times),
        );

        println!("  {:<7} {}", kind.name(), Value::Object(result.clone()));
        out.insert(kind.name().into(), Value::Object(result));
    }
    Ok(Value::Object(out))
}

// ---------------------------------------------------------------- main

fn text_of(command: &str, args: &[&str]) -> String {
    Command::new(command)
        .args(args)
        .output()
        .map(|o| String::from_utf8_lossy(&o.stdout).trim().to_string())
        .unwrap_or_default()
}

fn find_root() -> PathBuf {
    let mut dir = std::env::current_dir().unwrap_or_default();
    loop {
        if dir.join("Cargo.toml").exists() && dir.join("crates").is_dir() {
            return dir;
        }
        if !dir.pop() {
            return std::env::current_dir().unwrap_or_default();
        }
    }
}

fn ensure_arcade(root: &Path) -> R<PathBuf> {
    let status = Command::new("cargo")
        .args(["build", "--release", "-q", "-p", "arcade"])
        .current_dir(root)
        .status()?;
    if !status.success() {
        return Err("cannot build the release binary of arcade".into());
    }
    let exe = root.join("target/release/arcade");
    if !exe.exists() {
        return Err(format!("{} does not exist", exe.display()).into());
    }
    Ok(exe)
}

fn run() -> R<()> {
    let args: Vec<String> = std::env::args().skip(1).collect();
    if args.first().map(String::as_str) == Some("probe") {
        return probe_main(args.get(1).ok_or("probe needs a log path")?);
    }
    let mut quick = false;
    let mut subjects = vec![Kind::Direct, Kind::Tmux, Kind::Arcade];
    let mut tests: Vec<String> = Vec::new();
    let mut iter = args.iter();
    while let Some(arg) = iter.next() {
        match arg.as_str() {
            "--quick" => quick = true,
            "--subjects" => {
                let list = iter.next().ok_or("--subjects needs a list")?;
                subjects = list
                    .split(',')
                    .map(|s| match s {
                        "direct" => Ok(Kind::Direct),
                        "tmux" => Ok(Kind::Tmux),
                        "arcade" => Ok(Kind::Arcade),
                        other => Err(format!("unknown subject {other}")),
                    })
                    .collect::<Result<_, _>>()?;
            }
            "--tests" => {
                tests = iter
                    .next()
                    .ok_or("--tests needs a list")?
                    .split(',')
                    .map(String::from)
                    .collect();
            }
            other => return Err(format!("unknown argument {other}").into()),
        }
    }

    let root = find_root();
    let out_dir = root.join("bench-results");
    fs::create_dir_all(&out_dir)?;
    let dir = std::env::temp_dir().join(format!("arcade-bench-{}", std::process::id()));
    fs::DirBuilder::new()
        .recursive(true)
        .mode(0o700)
        .create(&dir)?;
    register(Cleanup::Dir(dir.clone()));
    let arcade_exe = if subjects.contains(&Kind::Arcade) {
        ensure_arcade(&root)?
    } else {
        PathBuf::new()
    };
    let config = Config {
        quick,
        subjects,
        tests,
        arcade_exe,
        dir,
        data_dir: out_dir.join("workloads"),
        out_dir,
    };

    let mut result = serde_json::Map::new();
    result.insert("quick".into(), json!(quick));
    result.insert("terminal_size".into(), json!(format!("{COLS}x{ROWS}")));
    result.insert("tmux_version".into(), json!(text_of("tmux", &["-V"])));
    result.insert(
        "machine".into(),
        json!({
            "cpu": text_of("sysctl", &["-n", "machdep.cpu.brand_string"]),
            "memory_bytes": text_of("sysctl", &["-n", "hw.memsize"]),
            "os": text_of("uname", &["-sr"]),
        }),
    );
    if config.has(Kind::Arcade) {
        result.insert(
            "arcade_commit".into(),
            json!(text_of(
                "git",
                &[
                    "-C",
                    &root.to_string_lossy(),
                    "rev-parse",
                    "--short",
                    "HEAD"
                ]
            )),
        );
    }

    type Test = fn(&Config) -> R<Value>;
    let tests: [(&str, &str, Test); 6] = [
        ("throughput", "Throughput", run_throughput),
        ("latency", "Key latency", run_latency),
        ("idle", "Idle", run_idle),
        ("memory", "Memory", run_memory),
        ("startup", "Startup", run_startup),
        ("stages", "Stages of a key and its echo", run_stages),
    ];
    for (key, title, test) in tests {
        // The stage-timing run is made only when asked for by name.
        let asked = config.tests.iter().any(|t| t == key);
        if !config.runs(key) || (key == "stages" && !asked) {
            continue;
        }
        println!("{title}:");
        result.insert(key.into(), test(&config)?);
    }

    let path = config.out_dir.join(if quick {
        "arcade-milestone-1-quick.json"
    } else {
        "arcade-milestone-1.json"
    });
    fs::write(&path, serde_json::to_string_pretty(&Value::Object(result))?)?;
    println!("\nResults written to {}", path.display());
    Ok(())
}

fn main() -> std::process::ExitCode {
    install_signal_cleanup();
    let guard = Guard;
    let code = match run() {
        Ok(()) => std::process::ExitCode::SUCCESS,
        Err(error) => {
            eprintln!("bench: {error}");
            std::process::ExitCode::FAILURE
        }
    };
    drop(guard);
    let _: io::Result<()> = Ok(());
    code
}
