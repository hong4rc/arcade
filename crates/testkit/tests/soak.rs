//! A soak test: a real daemon driven by a long run of random operations, with the daemon's
//! books and the screens looked over every fifty operations. It finds what tests of one feature
//! at a time do not: the order and the mix.
//!
//! The run is fixed by a seed. A failure prints the seed and the last operations; set
//! `ARCADE_SOAK_SEED` to run the same sequence again, `ARCADE_SOAK_OPS` for its length.
//!
//! One client is attached at a time (a second client that attaches to a session takes it over),
//! so the screen the client built from updates is compared with the screen a client that takes
//! the session over gets from a full redraw, cell by cell.

use std::time::{Duration, Instant};

use arcade_proto::ServerMessage;
use arcade_testkit::{AttachOptions, Screened, TestClient, TestDaemon, footprint_kib};

const PREFIX: &[u8] = b"\x1b[109;5u";
const ESCAPE: &[u8] = b"\x1b[27u";
const CHECK_EVERY: usize = 50;
/// The most the daemon (which runs inside this process) may grow by, in KiB.
const MEMORY_GROWTH_LIMIT_KIB: u64 = 600 * 1024;

/// A small, fast, seedable generator (splitmix64); the soak must not depend on a library whose
/// sequence could change.
struct Rng(u64);

impl Rng {
    fn next(&mut self) -> u64 {
        self.0 = self.0.wrapping_add(0x9E37_79B9_7F4A_7C15);
        let mut z = self.0;
        z = (z ^ (z >> 30)).wrapping_mul(0xBF58_476D_1CE4_E5B9);
        z = (z ^ (z >> 27)).wrapping_mul(0x94D0_49BB_1331_11EB);
        z ^ (z >> 31)
    }

    fn below(&mut self, n: usize) -> usize {
        (self.next() % n as u64) as usize
    }

    fn pick<'a, T>(&mut self, items: &'a [T]) -> &'a T {
        &items[self.below(items.len())]
    }
}

const SIZES: &[(u16, u16)] = &[
    (5, 2),
    (1, 1),
    (12, 4),
    (20, 5),
    (40, 12),
    (80, 24),
    (100, 30),
    (132, 43),
    (200, 60),
    (400, 100),
];

const NAMES: &[&str] = &[
    "work",
    "漢字",
    "a b",
    "x",
    "long-name-that-goes-on-and-on",
    "é",
];

const DIRECTIONS: &[&str] = &["left", "down", "up", "right"];

const COPY_COMMANDS: &[&str] = &[
    "copy.up",
    "copy.down",
    "copy.left",
    "copy.right",
    "copy.page-up",
    "copy.page-down",
    "copy.half-page-up",
    "copy.top",
    "copy.bottom",
    "copy.word-next",
    "copy.word-back",
    "copy.line-start",
    "copy.line-end",
    "copy.select-char",
    "copy.select-line",
    "copy.yank",
    "copy.yank-stay",
    "copy.search-next",
    "copy.search-previous",
    "copy.cancel",
    "copy.exit",
];

/// Lines a person might type into a shell, with wide characters and escape sequences.
const LINES: &[&str] = &[
    "echo hello",
    "echo 漢字ｗｉｄｅ and é",
    "echo 👍🏽 family 👨‍👩‍👧",
    "printf '\\033[1;31mred\\033[0m \\033[38;2;1;2;3mrgb\\033[0m\\n'",
    "printf '\\033[2J\\033[Hcleared\\n'",
    "printf 'a\\tb\\tc\\n'",
    "seq 1 60",
    "seq 1 400",
    "printf '\\033]0;a title\\007\\033]52;c;aGk=\\007ok\\n'",
    "printf '\\033[?1049h\\033[Halt screen\\033[?1049l'",
    "printf 'x\\033[10Cy\\033[3Dz\\n'",
    "echo $((6*7))",
    "stty size",
    "printf '\\033[4mu\\033[0m \\033[9ms\\033[0m \\033[3mi\\033[0m\\n'",
];

const KEYS: &[&[u8]] = &[
    b"\x1b[A",
    b"\x1b[B",
    b"\x1b[C",
    b"\x1b[D",
    b"ab",
    b"\x7f",
    b"\x1b[27u",
    b"\x15",
];

struct Soak {
    seed: u64,
    daemon: TestDaemon,
    client: TestClient,
    size: (u16, u16),
    rng: Rng,
    counter: usize,
    done: usize,
    recent: Vec<String>,
    memory_at_start: u64,
}

fn ended(client: &TestClient) -> bool {
    client.closed
        || client.messages.iter().any(|m| {
            matches!(
                m,
                ServerMessage::Detached { .. } | ServerMessage::SessionEnded { .. }
            )
        })
}

/// Read until nothing has come for a while.
fn settle(client: &mut TestClient) {
    let start = Instant::now();
    let mut last = Instant::now();
    while last.elapsed() < Duration::from_millis(150) && start.elapsed() < Duration::from_secs(8) {
        if client.read_more(Instant::now() + Duration::from_millis(40)) {
            last = Instant::now();
        }
        if client.closed {
            return;
        }
    }
}

fn rows_of(client: &mut TestClient) -> Vec<String> {
    client
        .screen
        .screen()
        .expect("read the screen")
        .iter()
        .map(|row| format!("{row:?}"))
        .collect()
}

fn start_daemon() -> TestDaemon {
    TestDaemon::start_with(|c| {
        c.log_path = Some(c.socket_path.with_extension("log"));
        c.scrollback_lines = 500;
    })
}

impl Soak {
    fn new(seed: u64) -> Soak {
        let daemon = start_daemon();
        let size = (100, 30);
        let client = daemon
            .attach(AttachOptions::new().size(size.0, size.1).named("soak-0"))
            .expect("first attach");
        Soak {
            seed,
            daemon,
            client,
            size,
            rng: Rng(seed),
            counter: 0,
            done: 0,
            recent: Vec::new(),
            memory_at_start: footprint_kib(std::process::id()),
        }
    }

    fn note(&mut self, what: String) {
        self.recent.push(format!("{}: {what}", self.done));
        if self.recent.len() > 40 {
            self.recent.remove(0);
        }
    }

    fn fail(&mut self, why: &str) -> ! {
        let screen = self.client.screen_text();
        panic!(
            "soak failed (seed {seed}, after {done} operations; run again with \
             ARCADE_SOAK_SEED={seed}): {why}\n\nthe last operations:\n{ops}\n\nthe client's screen:\n{screen}",
            seed = self.seed,
            done = self.done,
            ops = self.recent.join("\n"),
        );
    }

    fn keys(&mut self, bytes: &[u8]) {
        if !ended(&self.client) {
            let _ = self
                .client
                .send_message(&arcade_proto::ClientMessage::Input(bytes.to_vec()));
        }
    }

    fn command(&mut self, name: &str, args: &[&str]) {
        self.note(format!("command {name} {args:?}"));
        if self.daemon.is_stopped() {
            // Stopped because the last session ended, which `reattach` checks, or crashed,
            // which it reports.
            return;
        }
        // A refusal (nothing to close, no room) is an ordinary answer; no answer is not.
        if let Err(e) = self.daemon.command(name, args) {
            self.fail(&format!("{name}: {e}"));
        }
    }

    fn attached_name(&self) -> Option<String> {
        let (_, list) = self.daemon.command("session.list", &[]).ok()?;
        list.lines().find_map(|l| {
            let f: Vec<&str> = l.split('\t').collect();
            (f.get(2) == Some(&"1")).then(|| f[0].to_string())
        })
    }

    fn session_names(&self) -> Vec<String> {
        let list = self
            .daemon
            .command("session.list", &[])
            .map(|(_, t)| t)
            .unwrap_or_default();
        list.lines()
            .filter_map(|l| l.split('\t').next().map(str::to_string))
            .collect()
    }

    fn fresh_name(&mut self) -> String {
        self.counter += 1;
        format!("soak-{}", self.counter)
    }

    /// The session ended or the client was detached: attach to what is left, or to a new one.
    fn reattach(&mut self) {
        let (c, r) = self.size;
        // With no session left the daemon stops by itself (after the clients took their last
        // messages); it must do that cleanly and soon. A new one takes its place.
        if self.daemon.is_stopped() || self.session_names().is_empty() {
            if let Err(e) = self.daemon.wait_stopped(Duration::from_secs(5)) {
                self.fail(&format!(
                    "no session is left but the daemon did not stop cleanly: {e}"
                ));
            }
            self.note("the last session ended; a new daemon starts".to_string());
            self.daemon = start_daemon();
        }
        let names = self.session_names();
        let options = if let Some(name) = names.first() {
            AttachOptions::new().size(c, r).existing(Some(name))
        } else {
            let name = self.fresh_name();
            AttachOptions::new().size(c, r).named(&name)
        };
        self.note("attach again".to_string());
        match self.daemon.attach(options) {
            Ok(client) => self.client = client,
            Err(e) => self.fail(&format!("could not attach again: {e}")),
        }
    }

    fn drain(&mut self) {
        for _ in 0..3 {
            self.client
                .read_more(Instant::now() + Duration::from_millis(2));
        }
    }

    fn one_operation(&mut self) {
        let n = self.rng.below(100);
        match n {
            0..=13 => {
                let name = *self.rng.pick(&["pane.split-right", "pane.split-down"]);
                self.command(name, &[]);
            }
            14..=20 => self.command("pane.close", &[]),
            21..=25 => self.command("window.new", &[]),
            26..=32 => {
                let which = self.rng.below(12);
                match which {
                    0 => {
                        let number = (self.rng.below(5) + 1).to_string();
                        self.command("window.select", &[&number]);
                    }
                    1 => self.command("window.next", &[]),
                    2 => self.command("window.previous", &[]),
                    3 => self.command("window.last", &[]),
                    4 => self.command("window.move-left", &[]),
                    5 => self.command("window.move-right", &[]),
                    6 => {
                        let d = *self.rng.pick(DIRECTIONS);
                        self.command(&format!("pane.swap-{d}"), &[]);
                    }
                    7 => self.command("pane.break", &[]),
                    8 | 9 => self.command("window.layout-next", &[]),
                    10 => self.command("window.layout-even", &[]),
                    _ => self.command("pane.focus-last", &[]),
                }
            }
            33..=36 => {
                let name = *self.rng.pick(NAMES);
                let which = *self.rng.pick(&["window.rename", "session.rename"]);
                self.command(which, &[name]);
            }
            37..=46 => {
                let d = *self.rng.pick(DIRECTIONS);
                self.command(&format!("pane.focus-{d}"), &[]);
            }
            47..=54 => {
                let d = *self.rng.pick(DIRECTIONS);
                self.command(&format!("pane.resize-{d}"), &[]);
            }
            55..=57 => self.command("pane.zoom", &[]),
            58..=68 => self.copy_mode(),
            69..=73 => self.picker(),
            74..=76 => self.second_session(),
            77..=79 => self.leave_and_come_back(),
            80..=84 => {
                let (c, r) = *self.rng.pick(SIZES);
                self.note(format!("client resizes to {c}x{r}"));
                if !ended(&self.client) {
                    self.client.resize(c, r);
                }
                self.size = (c, r);
            }
            85..=93 => {
                if self.rng.below(4) == 0 {
                    let k = *self.rng.pick(KEYS);
                    self.note(format!("keys {k:?}"));
                    self.keys(k);
                } else {
                    let line = *self.rng.pick(LINES);
                    self.note(format!("types {line:?}"));
                    let mut bytes = line.as_bytes().to_vec();
                    bytes.push(b'\r');
                    self.keys(&bytes);
                }
            }
            94..=96 => {
                self.note("types a run of output".to_string());
                self.keys(b"yes 'soak line' | head -n 700\r");
            }
            _ => self.command("arcade.reload-config", &[]),
        }
        self.drain();
        if ended(&self.client) {
            self.reattach();
        }
    }

    fn copy_mode(&mut self) {
        self.command("copy.enter", &[]);
        let steps = 2 + self.rng.below(7);
        for _ in 0..steps {
            let c = *self.rng.pick(COPY_COMMANDS);
            self.command(c, &[]);
        }
        if self.rng.below(3) == 0 {
            self.note("searches in copy mode".to_string());
            self.keys(b"/line\r");
        }
        if self.rng.below(2) == 0 {
            self.command("copy.exit", &[]);
        }
    }

    fn picker(&mut self) {
        // Which list opens: sessions, keys, windows, the command palette, or none of them but the
        // hint of the prefix keys (the prefix, then a wait past the hint's delay).
        let which = self.rng.below(5);
        let name = ["session", "keys", "window", "command", "hint"][which];
        self.note(format!("opens the {name} list"));
        self.keys(PREFIX);
        if which == 4 {
            std::thread::sleep(Duration::from_millis(560));
            self.drain();
            // A key closes the hint: Escape does nothing else, a letter runs its action.
            let k: &[u8] = if self.rng.below(2) == 0 { ESCAPE } else { b"c" };
            self.keys(k);
            return;
        }
        let opening: &[u8] = match which {
            0 => b"s",
            1 => b"\x1b[47;2u",
            2 => b"w",
            _ => b"\x1b[59;2u",
        };
        self.keys(opening);
        // Typing into the filter and taking it back.
        if self.rng.below(2) == 0 {
            let text = *self
                .rng
                .pick(&["a", "z", "pane", "w", "ctrl", "no-such-name"]);
            self.note(format!("filters with {text:?}"));
            self.keys(text.as_bytes());
            match self.rng.below(3) {
                0 => self.keys(b"\x7f"),
                1 => self.keys(b"\x15"),
                _ => {}
            }
        }
        for _ in 0..self.rng.below(4) {
            let k: &[u8] = match self.rng.below(4) {
                0 => b"\x1b[B",
                1 => b"\x1b[A",
                2 => b"\x0e",
                _ => b"\x10",
            };
            self.keys(k);
        }
        // Closing a list is safe; choosing from it runs a command, which is fair too, but only
        // half the time.
        if self.rng.below(2) == 0 {
            self.keys(ESCAPE);
        } else {
            self.keys(b"\r");
        }
    }

    fn second_session(&mut self) {
        let names = self.session_names();
        if names.len() >= 3 {
            let name = self.rng.pick(&names).clone();
            self.command("session.close", &[&name]);
            return;
        }
        let name = self.fresh_name();
        self.note(format!("makes session {name}"));
        // A second client creates it and lets go; this client then moves over to it.
        match self
            .daemon
            .attach(AttachOptions::new().size(80, 24).named(&name))
        {
            Ok(mut other) => other.detach(),
            Err(e) => self.fail(&format!("could not make a second session: {e}")),
        }
        std::thread::sleep(Duration::from_millis(5));
        if self.rng.below(2) == 0 {
            self.command("session.switch", &[&name]);
        }
    }

    fn leave_and_come_back(&mut self) {
        let Some(name) = self.attached_name() else {
            return;
        };
        self.note(format!("client leaves {name} and comes back"));
        self.client.detach();
        self.client.wait_closed(Duration::from_secs(5));
        let (c, r) = if self.rng.below(2) == 0 {
            self.size
        } else {
            *self.rng.pick(SIZES)
        };
        self.size = (c, r);
        match self
            .daemon
            .attach(AttachOptions::new().size(c, r).existing(Some(&name)))
        {
            Ok(client) => self.client = client,
            Err(e) => self.fail(&format!("could not attach to {name} again: {e}")),
        }
    }

    /// Everything that must hold, looked at when the daemon is quiet.
    fn check(&mut self) {
        // Quiet: close lists and prompts, end the repeat window, clear any message.
        self.keys(ESCAPE);
        self.keys(ESCAPE);
        settle(&mut self.client);
        if ended(&self.client) {
            self.reattach();
            settle(&mut self.client);
        }

        // 1. The daemon is alive and answers.
        if self.daemon.is_stopped() {
            self.fail("the daemon stopped");
        }
        match self.daemon.command("session.list", &[]) {
            Ok((true, _)) => {}
            other => self.fail(&format!("session.list gave {other:?}")),
        }
        // 2. No panic in the log.
        let log =
            std::fs::read_to_string(self.daemon.socket().with_extension("log")).unwrap_or_default();
        if let Some(line) = log.lines().find(|l| l.to_lowercase().contains("panic")) {
            self.fail(&format!("the log has a panic: {line}"));
        }
        // 5. The daemon's own books add up.
        match self.daemon.command("server.check", &[]) {
            Ok((true, _)) => {}
            Ok((false, why)) => self.fail(&format!("server.check: {why}")),
            Err(e) => self.fail(&format!("server.check: {e}")),
        }
        // 6. Memory stays under a ceiling.
        let now = footprint_kib(std::process::id());
        if now > self.memory_at_start + MEMORY_GROWTH_LIMIT_KIB {
            self.fail(&format!(
                "memory grew from {} to {} KiB",
                self.memory_at_start, now
            ));
        }

        // 3 and 4. A client that takes the session over, of the same size, gets a full redraw;
        // it must show exactly what this client built up from the updates.
        let Some(name) = self.attached_name() else {
            self.fail("no session has a client");
        };
        let before = rows_of(&mut self.client);
        let (c, r) = self.size;
        let mut fresh = match self
            .daemon
            .attach(AttachOptions::new().size(c, r).existing(Some(&name)))
        {
            Ok(client) => client,
            Err(e) => self.fail(&format!("the takeover could not attach: {e}")),
        };
        settle(&mut fresh);
        let after = rows_of(&mut fresh);
        if before != after {
            let row = before
                .iter()
                .zip(&after)
                .position(|(a, b)| a != b)
                .unwrap_or(before.len().min(after.len()));
            let text = |rows: &Vec<String>| rows.get(row).cloned().unwrap_or_default();
            self.fail(&format!(
                "the screen built from updates differs from a full redraw at row {row} \
                 ({} rows against {}):\nfrom updates: {}\nfull redraw:  {}\n\
                 the redraw's screen:\n{}",
                before.len(),
                after.len(),
                text(&before),
                text(&after),
                fresh.screen_text()
            ));
        }
        self.client = fresh;
    }

    fn run(&mut self, operations: usize) {
        while self.done < operations {
            self.one_operation();
            self.done += 1;
            if self.done.is_multiple_of(CHECK_EVERY) {
                self.check();
            }
        }
        self.check();
    }
}

fn seed_from_env(default: u64) -> u64 {
    std::env::var("ARCADE_SOAK_SEED")
        .ok()
        .and_then(|v| v.parse().ok())
        .unwrap_or(default)
}

fn operations_from_env(default: usize) -> usize {
    std::env::var("ARCADE_SOAK_OPS")
        .ok()
        .and_then(|v| v.parse().ok())
        .unwrap_or(default)
}

#[test]
fn a_short_random_run_keeps_every_invariant() {
    let seed = seed_from_env(7);
    let mut soak = Soak::new(seed);
    soak.run(operations_from_env(500));
}

#[test]
#[ignore = "long soak"]
fn a_long_random_run_keeps_every_invariant() {
    let seeds: Vec<u64> = match std::env::var("ARCADE_SOAK_SEED") {
        Ok(v) => vec![v.parse().expect("ARCADE_SOAK_SEED is a number")],
        Err(_) => vec![101, 202, 303],
    };
    for seed in seeds {
        let mut soak = Soak::new(seed);
        soak.run(operations_from_env(20_000));
    }
}
