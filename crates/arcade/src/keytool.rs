//! `arcade keys list`, `arcade keys test`, and the warnings `arcade config check` adds: the
//! tools that show what the key tables hold and what the host terminal really sends.

use std::io::Write;
use std::path::PathBuf;
use std::time::{Duration, Instant};

use arcade_daemon::config::{self, Config};
use arcade_daemon::keytools::{self, HOST_QUERY, Host, Reply};
use arcade_daemon::{Input, KeyReader, Keymap, normalize, typed_name};
use arcade_term::{Code, Key};

use crate::host::{RawMode, Signals, is_tty};

/// How long to wait for the host terminal to answer the question of its keyboard.
const PROBE_WAIT: Duration = Duration::from_millis(300);

const KEYS_USAGE: &str =
    "usage: keys list [--host kitty|legacy] [<file>]\n       keys test [<file>]";

/// One line on the standard output. A reader that went away (`| head`) is not an error.
fn say(line: &str) {
    let _ = writeln!(std::io::stdout(), "{line}");
}

/// The configuration at `file`, or the usual place, or the defaults when there is no file.
/// A file with mistakes is told to the user and gives the exit code to leave with.
pub fn load(
    file: Option<PathBuf>,
    usual: Option<PathBuf>,
) -> Result<(Option<PathBuf>, Config), u8> {
    let Some(path) = file.or(usual) else {
        return Ok((None, Config::default()));
    };
    match config::load(&path) {
        Ok(config) => Ok((Some(path), config)),
        Err(errors) => {
            for e in errors {
                eprintln!("{}: {e}", path.display());
            }
            Err(1)
        }
    }
}

/// `--host kitty` or `--host legacy`, taken out of `args`.
pub fn take_host_flag(args: &mut Vec<String>) -> Result<Option<Host>, String> {
    let Some(at) = args.iter().position(|a| a == "--host") else {
        return Ok(None);
    };
    let value = args
        .get(at + 1)
        .cloned()
        .ok_or_else(|| "--host needs kitty or legacy".to_string())?;
    args.drain(at..=at + 1);
    match value.as_str() {
        "kitty" => Ok(Some(Host::Kitty)),
        "legacy" => Ok(Some(Host::Legacy)),
        other => Err(format!("--host needs kitty or legacy, not {other}")),
    }
}

/// Print the warnings for `config`, on the host given or the one this terminal turns out to be.
/// Where nothing is known about the host, say so instead of claiming the keys are fine.
pub fn print_warnings(config: &Config, flag: Option<Host>) {
    let host = flag.or_else(|| {
        if is_tty(0) && is_tty(1) {
            probe_host()
        } else {
            None
        }
    });
    for w in keytools::warnings(config, host) {
        say(&format!("warning: {w}"));
    }
    if host.is_none() {
        say(
            "note: the host terminal is not known, so keys it may not send are not checked (--host legacy checks them)",
        );
    }
}

/// Ask the terminal this process runs in whether it has the kitty keyboard protocol.
/// `None` when it cannot be asked or does not answer in time.
pub fn probe_host() -> Option<Host> {
    let raw = RawMode::enter(0).ok()?;
    let mut out = std::io::stdout();
    out.write_all(HOST_QUERY).ok()?;
    out.flush().ok()?;
    let mut reader = KeyReader::new();
    let mut kitty = false;
    let mut found = None;
    let deadline = Instant::now() + PROBE_WAIT;
    'wait: while let Some(left) = deadline.checked_duration_since(Instant::now()) {
        let Some(bytes) = read_input(0, left.min(Duration::from_millis(50))) else {
            continue;
        };
        for input in reader.feed(&bytes, Instant::now()) {
            if let Input::Raw(sequence) = input {
                match keytools::read_reply(&sequence) {
                    Some(Reply::Keyboard) => kitty = true,
                    Some(Reply::Attributes) => {
                        found = Some(if kitty { Host::Kitty } else { Host::Legacy });
                        break 'wait;
                    }
                    None => {}
                }
            }
        }
    }
    // SAFETY: tcflush only discards the input queued on the descriptor.
    unsafe {
        libc::tcflush(0, libc::TCIFLUSH);
    }
    drop(raw);
    found
}

/// Wait up to `wait` for bytes on `fd` and read what is there. `None` when nothing came.
fn read_input(fd: i32, wait: Duration) -> Option<Vec<u8>> {
    let mut pfd = libc::pollfd {
        fd,
        events: libc::POLLIN,
        revents: 0,
    };
    let millis = i32::try_from(wait.as_millis()).unwrap_or(i32::MAX);
    // SAFETY: poll on one live pollfd.
    let ready = unsafe { libc::poll(&mut pfd, 1, millis) };
    if ready <= 0 {
        return None;
    }
    let mut buf = [0u8; 256];
    // SAFETY: read into a live local buffer of the stated length.
    let n = unsafe { libc::read(fd, buf.as_mut_ptr().cast(), buf.len()) };
    (n > 0).then(|| buf[..n as usize].to_vec())
}

pub fn run(rest: &[String], usual: Option<PathBuf>) -> u8 {
    let usage = |message: &str| {
        eprintln!("arcade: {message}\n{KEYS_USAGE}");
        2
    };
    let mut args: Vec<String> = rest.to_vec();
    let Some(word) = args.first().cloned() else {
        return usage("keys needs list or test");
    };
    args.remove(0);
    match word.as_str() {
        "list" => {
            let flag = match take_host_flag(&mut args) {
                Ok(flag) => flag,
                Err(e) => return usage(&e),
            };
            let file = match args.as_slice() {
                [] => None,
                [file] => Some(PathBuf::from(file)),
                _ => return usage("keys list takes one file at most"),
            };
            list(file, usual, flag)
        }
        "test" => {
            let file = match args.as_slice() {
                [] => None,
                [file] => Some(PathBuf::from(file)),
                _ => return usage("keys test takes one file at most"),
            };
            test(file, usual)
        }
        other => usage(&format!("unknown keys command {other}")),
    }
}

fn list(file: Option<PathBuf>, usual: Option<PathBuf>, flag: Option<Host>) -> u8 {
    let (_, config) = match load(file, usual) {
        Ok(loaded) => loaded,
        Err(code) => return code,
    };
    for line in keytools::list_lines(&config) {
        say(&line);
    }
    say("");
    print_warnings(&config, flag);
    0
}

/// What a key does in each table, as a line of words.
fn what_it_does(map: &Keymap, prefix: Key, key: Key) -> String {
    let key = normalize(key);
    let mut parts = Vec::new();
    if key == normalize(prefix) {
        parts.push("the prefix key".to_string());
    }
    for (name, table) in [
        ("direct", &map.direct),
        ("after the prefix", &map.prefix),
        ("copy mode", &map.copy),
        ("lists", &map.list),
    ] {
        if let Some(b) = Keymap::get(table, key) {
            let mut text = b.command.clone();
            if !b.args.is_empty() {
                text.push(' ');
                text.push_str(&b.args.join(" "));
            }
            parts.push(format!("{name}: {text}"));
        }
    }
    if parts.is_empty() {
        "no binding: it goes to the program in the pane".to_string()
    } else {
        parts.join("; ")
    }
}

fn hex(bytes: &[u8]) -> String {
    bytes
        .iter()
        .map(|b| format!("{b:02x}"))
        .collect::<Vec<_>>()
        .join(" ")
}

/// Puts the keyboard back the way it was, whatever the way out.
struct KeyboardGuard;

impl Drop for KeyboardGuard {
    fn drop(&mut self) {
        let mut out = std::io::stdout();
        let _ = out.write_all(b"\x1b[<u");
        let _ = out.flush();
    }
}

fn test(file: Option<PathBuf>, usual: Option<PathBuf>) -> u8 {
    let (_, config) = match load(file, usual) {
        Ok(loaded) => loaded,
        Err(code) => return code,
    };
    if !is_tty(0) || !is_tty(1) {
        eprintln!("arcade: keys test needs a terminal");
        return 1;
    }
    let map = config.keymap();
    let prefix = config.keys.prefix;
    let signals = match Signals::install() {
        Ok(s) => s,
        Err(e) => {
            eprintln!("arcade: cannot set up signals: {e}");
            return 1;
        }
    };
    let raw = match RawMode::enter(0) {
        Ok(raw) => raw,
        Err(e) => {
            eprintln!("arcade: cannot set the terminal to raw mode: {e}");
            return 1;
        }
    };
    let guard = KeyboardGuard;
    let mut out = std::io::stdout();
    // The protocol as the attached client asks for it, then the question of what the terminal has.
    let _ = out.write_all(b"\x1b[>1u");
    let _ = out.write_all(HOST_QUERY);
    let _ = write!(
        out,
        "arcade keys test: press keys to see what Arcade receives. Ctrl+C quits.\r\nA key that shows nothing was taken by the system, the window manager or the terminal before it reached Arcade.\r\n"
    );
    let _ = out.flush();
    let mut reader = KeyReader::new();
    let mut kitty = false;
    'outer: loop {
        let wait = reader
            .deadline()
            .map(|t| t.saturating_duration_since(Instant::now()))
            .unwrap_or(Duration::from_millis(200));
        if let Some(signal) = signals.take().into_iter().find(|s| *s != libc::SIGWINCH) {
            let _ = signal;
            break;
        }
        let mut inputs = match read_input(0, wait.min(Duration::from_millis(200))) {
            Some(bytes) => reader.feed(&bytes, Instant::now()),
            None => Vec::new(),
        };
        inputs.extend(reader.expire(Instant::now()));
        for input in inputs {
            let line = match input {
                Input::Key { key, raw } => {
                    if key.code == Code::Char('c') && key.mods.ctrl && !key.mods.alt {
                        break 'outer;
                    }
                    format!(
                        "{:<24} bytes {:<28} {}",
                        typed_name(key),
                        hex(&raw),
                        what_it_does(&map, prefix, key)
                    )
                }
                Input::Raw(sequence) => match keytools::read_reply(&sequence) {
                    Some(Reply::Keyboard) => {
                        kitty = true;
                        continue;
                    }
                    Some(Reply::Attributes) => {
                        if kitty {
                            "host terminal: has the keyboard protocol, so every combination can be told apart".to_string()
                        } else {
                            "host terminal: old key codes only, so some combinations cannot be sent (arcade keys list --host legacy names them)".to_string()
                        }
                    }
                    None => format!("{:<24} bytes {}", "(not a key)", hex(&sequence)),
                },
                Input::Focus(true) => "(the terminal got the focus)".to_string(),
                Input::Focus(false) => "(the terminal lost the focus)".to_string(),
                Input::Paste(_) | Input::Mouse(_) => continue,
            };
            let mut out = std::io::stdout();
            let _ = write!(out, "{line}\r\n");
            let _ = out.flush();
        }
    }
    drop(guard);
    drop(raw);
    drop(signals);
    say("arcade keys test: done");
    0
}
