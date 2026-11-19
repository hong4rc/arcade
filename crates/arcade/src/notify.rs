//! `arcade notify`: telling the daemon that a pane needs attention. It is made to be called from
//! the hooks of other programs, so it never waits long, never starts a daemon, and by default
//! neither prints nor fails: whatever goes wrong, it ends quietly with the exit code 0. Only
//! `--strict` says what went wrong (on the error output) and exits with 1.

use std::path::Path;
use std::time::{Duration, Instant};

use arcade_proto::{ClientMessage, PROTOCOL_VERSION, ServerMessage};

use crate::connection::{ConnectError, Handshake, connect, handshake};

/// The whole time `notify` and `notify clear` may take to connect, send and hear back.
const BUDGET: Duration = Duration::from_millis(200);
/// The time the commands a person runs by hand may take.
const MANUAL_BUDGET: Duration = Duration::from_secs(3);
/// The version of the contract of the signal.
const SIGNAL_VERSION: u32 = 1;

#[derive(Default)]
struct Flags {
    class: Option<String>,
    title: Option<String>,
    message: Option<String>,
    target: Option<String>,
    source: Option<String>,
    strict: bool,
    all: bool,
    json: bool,
}

fn parse(args: &[String]) -> Result<Flags, String> {
    let mut flags = Flags::default();
    let mut words = args.iter();
    while let Some(word) = words.next() {
        let (name, inline) = match word.split_once('=') {
            Some((n, v)) if n.starts_with("--") => (n, Some(v.to_string())),
            _ => (word.as_str(), None),
        };
        match name {
            "--strict" => flags.strict = true,
            "--all" => flags.all = true,
            "--json" => flags.json = true,
            "--class" | "--title" | "--message" | "--target" | "--source" => {
                let value = match inline {
                    Some(v) => v,
                    None => words
                        .next()
                        .cloned()
                        .ok_or_else(|| format!("{name} needs a value"))?,
                };
                let slot = match name {
                    "--class" => &mut flags.class,
                    "--title" => &mut flags.title,
                    "--message" => &mut flags.message,
                    "--target" => &mut flags.target,
                    _ => &mut flags.source,
                };
                *slot = Some(value);
            }
            other => return Err(format!("unknown option {other}")),
        }
    }
    Ok(flags)
}

/// The pane to tell: the one named, else the one this program runs in. The second word says
/// where it came from.
fn target(flags: &Flags) -> Option<(String, &'static str)> {
    if let Some(t) = &flags.target {
        return Some((t.clone(), "the --target option"));
    }
    std::env::var("ARCADE_PANE")
        .ok()
        .filter(|p| !p.is_empty())
        .map(|p| (p, "the ARCADE_PANE variable"))
}

/// Connect, say hello, send one named command and wait for its answer, all within `budget`.
fn exchange(
    socket: &Path,
    name: &str,
    args: Vec<String>,
    budget: Duration,
) -> Result<(bool, String), String> {
    let start = Instant::now();
    let left = || {
        budget
            .checked_sub(start.elapsed())
            .filter(|d| !d.is_zero())
            .ok_or_else(|| "timed out".to_string())
    };
    let stream = connect(socket).map_err(|e| match e {
        ConnectError::NoServer => format!("no daemon is listening on {}", socket.display()),
        ConnectError::Other(m) => m,
    })?;
    let timeouts = |s: &std::os::unix::net::UnixStream, d: Duration| {
        s.set_read_timeout(Some(d))
            .and_then(|()| s.set_write_timeout(Some(d)))
            .map_err(|e| e.to_string())
    };
    timeouts(&stream, left()?)?;
    let mut connection = match handshake(stream)? {
        Handshake::Ok(c) => c,
        Handshake::Mismatch { daemon, client, .. } => {
            return Err(format!(
                "the daemon speaks protocol {daemon}, this arcade {client}"
            ));
        }
    };
    timeouts(&connection.stream, left()?)?;
    connection.send(&ClientMessage::Command {
        name: name.to_string(),
        args,
    })?;
    loop {
        timeouts(&connection.stream, left()?)?;
        match connection.receive()? {
            Some(ServerMessage::CommandResult { ok, output }) => return Ok((ok, output)),
            Some(_) => {}
            None => return Err("the daemon closed the connection".to_string()),
        }
    }
}

/// How a hook-safe command ends: quietly, unless `--strict`.
fn finish(strict: bool, result: Result<(bool, String), String>) -> u8 {
    let problem = match result {
        Ok((true, _)) => return 0,
        Ok((false, why)) => why,
        Err(why) => why,
    };
    if strict {
        eprintln!("arcade: notify: {}", problem.trim_end());
        1
    } else {
        0
    }
}

pub fn run(socket: &Path, rest: &[String]) -> u8 {
    let (word, tail) = match rest.split_first() {
        Some((w, t)) if matches!(w.as_str(), "clear" | "check" | "log" | "list") => (w.as_str(), t),
        _ => ("send", rest),
    };
    let flags = match parse(tail) {
        Ok(f) => f,
        Err(why) => {
            // A hook with a wrong option must not break the program that called it.
            return if tail.iter().any(|a| a == "--strict")
                || matches!(word, "check" | "log" | "list")
            {
                eprintln!("arcade: notify: {why}");
                1
            } else {
                0
            };
        }
    };
    match word {
        "clear" => clear(socket, &flags),
        "check" => check(socket, &flags),
        "log" => log(socket),
        "list" => list(socket, flags.json),
        _ => send(socket, &flags),
    }
}

fn send(socket: &Path, flags: &Flags) -> u8 {
    let Some((pane, _)) = target(flags) else {
        return finish(
            flags.strict,
            Err("no pane to tell: give --target, or run inside an Arcade pane".to_string()),
        );
    };
    let mut args = vec![
        format!("version={SIGNAL_VERSION}"),
        format!("target={pane}"),
    ];
    for (key, value) in [
        ("class", &flags.class),
        ("title", &flags.title),
        ("message", &flags.message),
        ("source", &flags.source),
    ] {
        if let Some(v) = value {
            args.push(format!("{key}={v}"));
        }
    }
    finish(flags.strict, exchange(socket, "notify.send", args, BUDGET))
}

fn clear(socket: &Path, flags: &Flags) -> u8 {
    let args =
        if flags.all {
            vec!["all=1".to_string()]
        } else {
            match target(flags) {
                Some((pane, _)) => vec![format!("target={pane}")],
                None => {
                    return finish(
                    flags.strict,
                    Err("no pane to clear: give --target or --all, or run inside an Arcade pane"
                        .to_string()),
                );
                }
            }
        };
    finish(flags.strict, exchange(socket, "notify.clear", args, BUDGET))
}

/// For a person to run by hand, not in a hook: say what would be used, and send a test.
fn check(socket: &Path, flags: &Flags) -> u8 {
    let mut failed = false;
    let pane = match target(flags) {
        Some((pane, origin)) => {
            println!("pane: {pane} (from {origin})");
            Some(pane)
        }
        None => {
            println!("pane: none: give --target, or run inside an Arcade pane");
            failed = true;
            None
        }
    };
    println!("socket: {}", socket.display());
    match exchange(socket, "server.ping", Vec::new(), MANUAL_BUDGET) {
        Ok((true, _)) => println!("daemon: reachable, protocol version {PROTOCOL_VERSION}"),
        Ok((false, why)) | Err(why) => {
            println!("daemon: not reachable: {}", why.trim_end());
            return 1;
        }
    }
    if let Some(pane) = pane {
        let args = vec![
            format!("version={SIGNAL_VERSION}"),
            format!("target={pane}"),
            "class=info".to_string(),
            "source=check".to_string(),
            "title=arcade notify check".to_string(),
        ];
        match exchange(socket, "notify.send", args, MANUAL_BUDGET) {
            Ok((true, result)) => println!("test notification: {result}"),
            Ok((false, why)) | Err(why) => {
                println!("test notification: failed: {}", why.trim_end());
                failed = true;
            }
        }
    }
    u8::from(failed)
}

/// `arcade notify list`: the notifications that wait, one per line (or a JSON list), and nothing
/// when there are none. It only reads.
fn list(socket: &Path, json: bool) -> u8 {
    match exchange(socket, "notify.list", Vec::new(), MANUAL_BUDGET) {
        Ok((true, output)) => {
            let rows: Vec<Vec<&str>> = output
                .lines()
                .map(|line| line.splitn(10, '\t').collect::<Vec<&str>>())
                .filter(|f| f.len() == 10)
                .collect();
            if json {
                let items: Vec<String> = rows
                    .iter()
                    .map(|f| {
                        format!(
                            "{{\"session\":{},\"window\":{},\"window_name\":{},\"pane\":{},\"pane_label\":{},\"class\":{},\"source\":{},\"waited_seconds\":{},\"title\":{},\"message\":{}}}",
                            json_text(f[0]),
                            f[1].parse::<u64>().unwrap_or(0),
                            json_text(f[2]),
                            f[3].parse::<u64>().unwrap_or(0),
                            json_text(f[4]),
                            json_text(f[5]),
                            json_text(f[6]),
                            f[7].parse::<u64>().unwrap_or(0),
                            json_text(f[8]),
                            json_text(f[9]),
                        )
                    })
                    .collect();
                println!("[{}]", items.join(","));
            } else {
                for f in rows {
                    let what = match (f[8].is_empty(), f[9].is_empty()) {
                        (false, false) => format!("{}: {}", f[8], f[9]),
                        (false, true) => f[8].to_string(),
                        _ => f[9].to_string(),
                    };
                    println!(
                        "{}\t{} {}\tpane {} {}\t{}\t{}\t{}s\t{}",
                        f[0], f[1], f[2], f[3], f[4], f[5], f[6], f[7], what
                    );
                }
            }
            0
        }
        Ok((false, why)) | Err(why) => {
            eprintln!("arcade: notify list: {}", why.trim_end());
            1
        }
    }
}

/// A text as a JSON string.
fn json_text(text: &str) -> String {
    let mut out = String::from("\"");
    for c in text.chars() {
        match c {
            '"' => out.push_str("\\\""),
            '\\' => out.push_str("\\\\"),
            '\n' => out.push_str("\\n"),
            '\t' => out.push_str("\\t"),
            c if (c as u32) < 0x20 => out.push_str(&format!("\\u{:04x}", c as u32)),
            c => out.push(c),
        }
    }
    out.push('"');
    out
}

fn log(socket: &Path) -> u8 {
    match exchange(socket, "notify.log", Vec::new(), MANUAL_BUDGET) {
        Ok((true, output)) => {
            if output.is_empty() {
                println!("no signals yet");
            }
            for line in output.lines() {
                let f: Vec<&str> = line.splitn(6, '\t').collect();
                if let [age, channel, class, source, pane, result] = f[..] {
                    println!(
                        "{age:>4}s ago  {channel:<8} {class:<9} {source:<12} pane {pane:<4} {result}"
                    );
                }
            }
            0
        }
        Ok((false, why)) | Err(why) => {
            eprintln!("arcade: notify log: {}", why.trim_end());
            1
        }
    }
}

/// `arcade window log`: the latest changes of the window shown in each session, and of the host
/// terminal's focus, newest first, each with the reason the daemon recorded.
pub fn window_log(socket: &Path) -> u8 {
    match exchange(socket, "window.log", Vec::new(), MANUAL_BUDGET) {
        Ok((true, output)) => {
            if output.is_empty() {
                println!("no window change since the daemon started");
                return 0;
            }
            println!(
                "the latest changes, newest first (seconds since the daemon started, session, window, reason):"
            );
            for line in output.lines() {
                println!("  {line}");
            }
            0
        }
        Ok((false, why)) | Err(why) => {
            eprintln!("arcade: window log: {}", why.trim_end());
            1
        }
    }
}

/// `arcade keys stray`: the key bytes the daemon wrote to a pane that no key stands for.
pub fn stray(socket: &Path) -> u8 {
    match exchange(socket, "keys.stray", Vec::new(), MANUAL_BUDGET) {
        Ok((true, output)) => {
            let mut lines = output.lines();
            let count = lines.next().unwrap_or("0");
            if count == "0" {
                println!("no stray key bytes since the daemon started");
                return 0;
            }
            println!(
                "{count} stray key bytes since the daemon started; the latest (seconds since it started, pane, keyboard flags of the pane, the bytes the host sent for that key, the bytes written, in hexadecimal):"
            );
            for line in lines.filter(|l| !l.is_empty()) {
                println!("  {line}");
            }
            0
        }
        Ok((false, why)) | Err(why) => {
            eprintln!("arcade: keys stray: {}", why.trim_end());
            1
        }
    }
}
