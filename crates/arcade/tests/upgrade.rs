//! `arcade upgrade`: the daemon continues as another program in the same process and keeps its
//! sessions, the programs in the panes and the attached screens. These run the real `arcade`
//! (a copy of it plays the new program) on a private socket in a private directory.

use std::os::unix::fs::PermissionsExt;
use std::path::{Path, PathBuf};
use std::process::{Command, Output, Stdio};
use std::sync::atomic::{AtomicU32, Ordering};
use std::time::{Duration, Instant};

use arcade_proto::{DetachReason, ServerMessage};
use arcade_testkit::{AttachOptions, PtyHost, Screened, TestClient, WAIT};

const ARCADE: &str = env!("CARGO_BIN_EXE_arcade");

static COUNTER: AtomicU32 = AtomicU32::new(0);

struct Env {
    dir: PathBuf,
    socket: PathBuf,
}

impl Env {
    fn new() -> Env {
        let dir = PathBuf::from(format!(
            "/tmp/arc-up-{}-{}",
            std::process::id(),
            COUNTER.fetch_add(1, Ordering::Relaxed)
        ));
        std::fs::create_dir_all(dir.join(".config/arcade")).unwrap();
        let bin_dir = Path::new(ARCADE).parent().unwrap().display().to_string();
        std::fs::write(
            dir.join(".profile"),
            format!("PS1='$ '\nPATH={bin_dir}:$PATH\nexport PATH\n"),
        )
        .unwrap();
        std::fs::write(
            dir.join(".config/arcade/config.toml"),
            "[power]\nprevent-sleep = false\n",
        )
        .unwrap();
        let socket = dir.join("s");
        Env { dir, socket }
    }

    fn env(&self) -> Vec<String> {
        let bin_dir = Path::new(ARCADE).parent().unwrap().display().to_string();
        vec![
            format!("PATH={bin_dir}:/usr/bin:/bin:/usr/sbin:/sbin"),
            format!("HOME={}", self.dir.display()),
            "PS1=$ ".to_string(),
            "LANG=en_US.UTF-8".to_string(),
            "SHELL=/bin/sh".to_string(),
            "TERM=xterm-256color".to_string(),
            format!("ARCADE_SOCKET={}", self.socket.display()),
        ]
    }

    fn host(&self, program: &Path, args: &[&str]) -> PtyHost {
        PtyHost::spawn(program, args, self.env(), &self.dir, 80, 24)
    }

    fn host_with(&self, program: &Path, args: &[&str], extra: &[&str]) -> PtyHost {
        let mut env = self.env();
        env.extend(extra.iter().map(|e| e.to_string()));
        PtyHost::spawn(program, args, env, &self.dir, 80, 24)
    }

    /// Run a command with no terminal and extra environment (a test version, for one).
    fn plain_env(&self, program: &Path, args: &[&str], extra: &[&str]) -> Output {
        let mut command = Command::new(program);
        command.args(args).env_clear().stdin(Stdio::null());
        for entry in self
            .env()
            .iter()
            .map(String::as_str)
            .chain(extra.iter().copied())
        {
            let (k, v) = entry.split_once('=').unwrap();
            command.env(k, v);
        }
        command.output().expect("run arcade")
    }

    fn set_config(&self, text: &str) {
        std::fs::write(
            self.dir.join(".config/arcade/config.toml"),
            format!("[power]\nprevent-sleep = false\n{text}"),
        )
        .unwrap();
    }

    fn replaced(&self) -> bool {
        self.log().contains("daemon replaced its program")
    }

    fn plain_with(&self, program: &Path, args: &[&str]) -> Output {
        let mut command = Command::new(program);
        command.args(args).env_clear().stdin(Stdio::null());
        for entry in self.env() {
            let (k, v) = entry.split_once('=').unwrap();
            command.env(k, v);
        }
        command.output().expect("run arcade")
    }

    fn plain(&self, args: &[&str]) -> Output {
        self.plain_with(Path::new(ARCADE), args)
    }

    /// A copy of the program under test: the "new" program of an upgrade.
    fn copy_of_program(&self, name: &str) -> PathBuf {
        let path = self.dir.join(name);
        std::fs::copy(ARCADE, &path).unwrap();
        std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o755)).unwrap();
        path
    }

    fn upgrade_to(&self, program: &Path) -> Output {
        self.plain(&["upgrade", "--force", program.to_str().unwrap()])
    }

    fn sessions(&self) -> String {
        String::from_utf8_lossy(&self.plain(&["ls"]).stdout).to_string()
    }

    fn log(&self) -> String {
        std::fs::read_to_string(self.socket.with_extension("log")).unwrap_or_default()
    }

    /// The process number the daemon says it runs as, each time it says so.
    fn daemon_pids(&self) -> Vec<u32> {
        self.log()
            .lines()
            .filter(|l| l.contains("daemon "))
            .filter_map(|l| {
                let at = l.find("process ")?;
                l[at + 8..]
                    .split(|c: char| !c.is_ascii_digit())
                    .next()?
                    .parse()
                    .ok()
            })
            .collect()
    }

    fn client(&self, session: &str, cols: u16, rows: u16) -> TestClient {
        let mut options = AttachOptions::new()
            .existing(Some(session))
            .size(cols, rows);
        options.cwd = Some(self.dir.display().to_string());
        options.env = Some(self.env());
        TestClient::attach_to(&self.socket, options).expect("attach")
    }
}

impl Drop for Env {
    fn drop(&mut self) {
        let _ = self.plain(&["kill-server"]);
        let start = Instant::now();
        while self.socket.exists() && start.elapsed() < Duration::from_secs(8) {
            std::thread::sleep(Duration::from_millis(10));
        }
        let _ = std::fs::remove_dir_all(&self.dir);
    }
}

fn text(out: &Output) -> String {
    format!(
        "{}{}",
        String::from_utf8_lossy(&out.stdout),
        String::from_utf8_lossy(&out.stderr)
    )
}

/// Type a line until a row starting with `prefix` shows: keys typed in the moment between the old
/// screen leaving and the new one attaching are lost, as they are for a person.
fn type_until_row_starting(host: &mut PtyHost, line: &str, prefix: &str) {
    for _ in 0..6 {
        host.type_line(line);
        if host.wait_until(Duration::from_secs(2), |h| {
            h.screen_text().lines().any(|l| l.starts_with(prefix))
        }) {
            return;
        }
    }
    panic!("no row starts with {prefix:?}:\n{}", host.screen_text());
}

/// Wait until a row of the screen starts with `prefix` (a row is matched whole by `wait_for_row`).
fn wait_for_row_starting(host: &mut PtyHost, prefix: &str) {
    let found = host.wait_until(WAIT, |h| {
        h.screen_text().lines().any(|l| l.starts_with(prefix))
    });
    assert!(
        found,
        "no row starts with {prefix:?}:\n{}",
        host.screen_text()
    );
}

/// What a person sees: a shell in a session; the daemon continues as the new program; the
/// screen comes back by itself in the same terminal with the same shell, and what was typed
/// before is still there.
#[test]
fn an_upgrade_keeps_the_shell_and_the_screen_comes_back_by_itself() {
    let env = Env::new();
    let new = env.copy_of_program("arcade-new");
    let mut host = env.host(Path::new(ARCADE), &["new", "-s", "main"]);
    host.wait_for_prompt().unwrap();
    host.type_line("echo shell-$$");
    wait_for_row_starting(&mut host, "shell-");
    let before = host.screen_text();
    let pid_before: String = before
        .lines()
        .rev()
        .find_map(|l| l.trim().strip_prefix("shell-").map(str::to_string))
        .filter(|p| p.chars().all(|c| c.is_ascii_digit()) && !p.is_empty())
        .expect("the shell's number");
    let pids = env.daemon_pids();

    let out = env.upgrade_to(&new);
    assert!(out.status.success(), "{}", text(&out));
    assert!(text(&out).contains("upgraded"), "{}", text(&out));

    // The same terminal shows the session again, with what it showed, and the shell answers.
    type_until_row_starting(&mut host, "echo again-$$", "again-");
    let after = host.screen_text();
    assert!(after.contains(&format!("shell-{pid_before}")), "{after}");
    assert!(after.contains(&format!("again-{pid_before}")), "{after}");

    // One session still, one daemon process throughout, which now runs the new program.
    assert!(env.sessions().contains("main"), "{}", env.sessions());
    let after_pids = env.daemon_pids();
    assert!(after_pids.len() > pids.len(), "{:?}", env.log());
    assert!(
        after_pids.iter().all(|p| *p == pids[0]),
        "the daemon's process changed: {after_pids:?}"
    );
    assert!(env.log().contains("daemon replaced its program"));
    // The saved copy of the old program is gone once the new one is ready.
    let leftovers: Vec<_> = std::fs::read_dir(&env.dir)
        .unwrap()
        .filter_map(|e| e.ok())
        .filter(|e| {
            e.file_name()
                .to_string_lossy()
                .starts_with("arcade-previous")
        })
        .collect();
    assert!(leftovers.is_empty(), "{leftovers:?}");
}

/// A client that does not know how to start the new program is told, and the session is there
/// for the next attach.
#[test]
fn an_attached_client_is_told_which_program_and_session_to_attach_again() {
    let env = Env::new();
    let new = env.copy_of_program("arcade-new");
    assert!(
        env.plain(&["new", "-d", "-s", "work", "/bin/sh"])
            .status
            .success()
    );
    let mut client = env.client("work", 100, 30);
    client.wait_for_prompt().unwrap();
    let out = env.upgrade_to(&new);
    assert!(out.status.success(), "{}", text(&out));
    let told = client.wait_until(WAIT, |c| {
        c.messages.iter().any(|m| {
            matches!(
                m,
                ServerMessage::Detached {
                    reason: DetachReason::Upgraded,
                    ..
                }
            )
        })
    });
    assert!(told, "{:?}", client.messages.last());
    let message = client
        .messages
        .iter()
        .find_map(|m| match m {
            ServerMessage::Detached {
                reason: DetachReason::Upgraded,
                message,
            } => Some(message.clone()),
            _ => None,
        })
        .unwrap();
    let (program, session) = message.split_once('\0').expect("program and session");
    assert_eq!(session, "work");
    assert!(program.ends_with("arcade-new"), "{program}");
    // Attaching again, as the client does, finds the shell where it was.
    let mut again = env.client("work", 100, 30);
    again.type_line("echo back");
    again.wait_for_text("back").unwrap();
}

/// What the new program refuses or fails at, before and after the point of no return, never
/// costs a session.
#[test]
fn refusals_change_nothing() {
    let env = Env::new();
    assert!(
        env.plain(&["new", "-d", "-s", "keep", "/bin/sh"])
            .status
            .success()
    );
    // The same program and version: nothing to do, unless forced.
    let out = env.plain(&["upgrade", ARCADE]);
    assert!(!out.status.success());
    assert!(text(&out).contains("nothing to upgrade"), "{}", text(&out));
    // A file that is not a program.
    let junk = env.dir.join("junk");
    std::fs::write(&junk, "not a program").unwrap();
    std::fs::set_permissions(&junk, std::fs::Permissions::from_mode(0o755)).unwrap();
    let out = env.plain(&["upgrade", "--force", junk.to_str().unwrap()]);
    assert!(!out.status.success());
    assert!(text(&out).contains("cannot take over"), "{}", text(&out));
    // A program anybody can write to is not trusted.
    let open = env.copy_of_program("arcade-open");
    std::fs::set_permissions(&open, std::fs::Permissions::from_mode(0o777)).unwrap();
    let out = env.plain(&["upgrade", "--force", open.to_str().unwrap()]);
    assert!(!out.status.success());
    assert!(text(&out).contains("written by others"), "{}", text(&out));
    // A rehearsal that fails.
    let new = env.copy_of_program("arcade-new");
    std::fs::write(env.socket.with_extension("fault"), "rehearsal-error").unwrap();
    let out = env.upgrade_to(&new);
    assert!(!out.status.success());
    assert!(text(&out).contains("cannot take over"), "{}", text(&out));
    assert!(env.sessions().contains("keep"), "{}", env.sessions());
    assert!(env.log().lines().filter(|l| l.contains("started")).count() == 1);
}

/// A new program that fails after the exec — by an error, a panic, a crash or by never saying
/// it is ready — is replaced by the old one, which restores the sessions and says so.
#[test]
fn a_new_program_that_fails_to_come_up_leaves_everything_on_the_old_one() {
    for fault in ["error", "panic", "crash", "hang"] {
        let env = Env::new();
        let new = env.copy_of_program("arcade-new");
        let mut host = env.host(Path::new(ARCADE), &["new", "-s", "main"]);
        host.wait_for_prompt().unwrap();
        host.type_line("echo shell-$$");
        wait_for_row_starting(&mut host, "shell-");
        std::fs::write(env.socket.with_extension("fault"), fault).unwrap();
        let out = env.upgrade_to(&new);
        assert!(!out.status.success(), "{fault}: {}", text(&out));
        assert!(
            text(&out).contains("the upgrade was undone"),
            "{fault}: {}",
            text(&out)
        );
        // The attached screen never left; the shell is the same one.
        type_until_row_starting(&mut host, "echo still-$$", "still-");
        let screen = host.screen_text();
        assert!(screen.contains("shell-"), "{fault}: {screen}");
        assert!(env.sessions().contains("main"), "{fault}");
        assert!(env.log().contains("undone"), "{fault}: {}", env.log());
        let leftovers = std::fs::read_dir(&env.dir)
            .unwrap()
            .filter_map(|e| e.ok())
            .filter(|e| {
                e.file_name()
                    .to_string_lossy()
                    .starts_with("arcade-previous")
            })
            .count();
        assert_eq!(leftovers, 0, "{fault}");
        // And the daemon can still be upgraded afterwards.
        let out = env.upgrade_to(&new);
        assert!(out.status.success(), "{fault}: {}", text(&out));
    }
}

/// A pane with a fast writer loses nothing and repeats nothing across an upgrade.
#[test]
fn a_counting_loop_is_neither_cut_nor_repeated() {
    let env = Env::new();
    let new = env.copy_of_program("arcade-new");
    let script = "i=0; while [ $i -lt 1500 ]; do echo n$i; i=$((i+1)); sleep 0.002; done; echo finished; sleep 1000";
    assert!(
        env.plain(&["new", "-d", "-s", "count", "/bin/sh", "-c", script])
            .status
            .success()
    );
    let mut client = env.client("count", 100, 30);
    client.wait_for_text("n300").unwrap();
    let out = env.upgrade_to(&new);
    assert!(out.status.success(), "{}", text(&out));
    // The whole history on one screen: every number once, in order.
    let mut all = env.client("count", 100, 2200);
    all.wait_for_text("finished").unwrap();
    let rows = all.screen_rows();
    let numbers: Vec<u32> = rows
        .iter()
        .filter_map(|r| r.trim().strip_prefix('n')?.parse().ok())
        .collect();
    let expected: Vec<u32> = (0..1500).collect();
    assert_eq!(numbers.len(), expected.len(), "{} numbers", numbers.len());
    assert_eq!(numbers, expected);
}

/// A program that ends while the daemon replaces itself is still reaped, and its pane closes.
#[test]
fn a_program_that_ends_during_upgrades_is_reaped_and_its_pane_closes() {
    let env = Env::new();
    let new = env.copy_of_program("arcade-new");
    assert!(
        env.plain(&["new", "-d", "-s", "stay", "/bin/sh"])
            .status
            .success()
    );
    assert!(
        env.plain(&[
            "new",
            "-d",
            "-s",
            "short",
            "/bin/sh",
            "-c",
            "sleep 0.4; exit 0"
        ])
        .status
        .success()
    );
    let start = Instant::now();
    let mut upgrades = 0;
    while env.sessions().contains("short") && start.elapsed() < Duration::from_secs(10) {
        let out = env.upgrade_to(&new);
        assert!(out.status.success(), "{}", text(&out));
        upgrades += 1;
    }
    assert!(!env.sessions().contains("short"), "{}", env.sessions());
    assert!(env.sessions().contains("stay"));
    assert!(upgrades >= 1);
    // No process of ours is left as a zombie.
    let zombies = Command::new("ps")
        .args(["-axo", "stat=,command="])
        .output()
        .unwrap();
    let zombies = String::from_utf8_lossy(&zombies.stdout);
    assert!(
        !zombies
            .lines()
            .any(|l| l.trim_start().starts_with('Z') && l.contains("sleep 0.4")),
        "{zombies}"
    );
}

/// Makes the snapshot kept under `crates/daemon/tests/snapshots/`: a real daemon with two
/// windows, a split, history, links, a title, a progress report and a full-screen program. Run
/// by hand when a release changes the format:
/// `ARCADE_FIXTURE_OUT=<file> cargo test -p arcade --test upgrade -- --ignored write_the_snapshot`.
/// The snapshot holds the environment and directories the session was started with, so this one
/// starts from a directory and an environment that name nothing of the machine.
#[test]
#[ignore = "writes a file by hand"]
fn write_the_snapshot_fixture() {
    let out = std::env::var("ARCADE_FIXTURE_OUT").expect("ARCADE_FIXTURE_OUT names the file");
    let env = Env::new();
    let old = env.copy_of_program("arcade-old");
    let new = env.copy_of_program("arcade-new");
    let clean_env: Vec<String> = vec![
        "PATH=/usr/bin:/bin".to_string(),
        format!("HOME={}", env.dir.display()),
        "PS1=$ ".to_string(),
        "LANG=en_US.UTF-8".to_string(),
        "SHELL=/bin/sh".to_string(),
    ];
    let mut command = Command::new(&old);
    command
        .args([
            "--socket",
            env.socket.to_str().unwrap(),
            "new",
            "-d",
            "-s",
            "work",
            "/bin/sh",
        ])
        .env_clear()
        .current_dir("/")
        .stdin(Stdio::null());
    for entry in &clean_env {
        let (k, v) = entry.split_once('=').unwrap();
        command.env(k, v);
    }
    assert!(command.output().unwrap().status.success());
    let mut client = env.client("work", 100, 30);
    client.wait_for_prompt().unwrap();
    for line in [
        "i=0; while [ $i -lt 150 ]; do printf '\\033[1;3%dmhistory line %d\\033[0m\\n' $((i%7)) $i; i=$((i+1)); done".to_string(),
        "printf '\\033]0;a title\\007\\033]8;;https://example.com\\033\\\\a link\\033]8;;\\033\\\\\\n'".to_string(),
        "printf '\\033]9;4;1;40\\007\\033[5 q'".to_string(),
    ] {
        client.type_line(&line);
        std::thread::sleep(Duration::from_millis(300));
    }
    assert!(env.plain(&["run", "pane.split-right"]).status.success());
    client.type_line("printf '\\033[?1049h\\033[2J\\033[3;3Hfull screen program'");
    client.wait_for_text("full screen program").unwrap();
    assert!(env.plain(&["run", "window.new"]).status.success());
    std::thread::sleep(Duration::from_millis(300));
    std::fs::write(env.socket.with_extension("fault"), "keep-snapshot").unwrap();
    let result = env.upgrade_to(&new);
    assert!(result.status.success(), "{}", text(&result));
    std::fs::copy(env.socket.with_extension("snapshot"), &out).unwrap();
}

/// A client that speaks another protocol version cannot attach, but can still ask the daemon to
/// continue as its program: the daemon becomes that program, so the two protocols need not agree
/// for anything but the first message. After it, that program's own client attaches.
#[test]
fn a_client_of_another_protocol_can_ask_for_the_upgrade() {
    use arcade_proto::{ClientMessage, Decoder, PROTOCOL_VERSION, encode};
    use std::io::{Read, Write};
    let env = Env::new();
    let new = env.copy_of_program("arcade-new");
    assert!(
        env.plain(&["new", "-d", "-s", "old", "/bin/sh"])
            .status
            .success()
    );
    let mut stream = std::os::unix::net::UnixStream::connect(&env.socket).unwrap();
    stream.set_read_timeout(Some(WAIT)).unwrap();
    let send = |stream: &mut std::os::unix::net::UnixStream, m: &ClientMessage| {
        let mut buf = Vec::new();
        encode(m, &mut buf).unwrap();
        stream.write_all(&buf).unwrap();
    };
    send(
        &mut stream,
        &ClientMessage::Hello {
            version: PROTOCOL_VERSION + 7,
        },
    );
    let mut decoder: Decoder<ServerMessage> = Decoder::new();
    let mut wait_for = |stream: &mut std::os::unix::net::UnixStream,
                        want: &dyn Fn(&ServerMessage) -> bool|
     -> ServerMessage {
        let mut buf = [0u8; 4096];
        loop {
            while let Some(m) = decoder.next_message().unwrap() {
                if want(&m) {
                    return m;
                }
            }
            let n = stream.read(&mut buf).expect("the daemon answers");
            assert!(n > 0, "the daemon closed the connection");
            decoder.push(&buf[..n]);
        }
    };
    wait_for(&mut stream, &|m| {
        matches!(m, ServerMessage::VersionMismatch { .. })
    });
    send(
        &mut stream,
        &ClientMessage::Command {
            name: "server.upgrade".to_string(),
            args: vec![new.display().to_string(), "--force".to_string()],
        },
    );
    let answer = wait_for(&mut stream, &|m| {
        matches!(m, ServerMessage::CommandResult { .. })
    });
    match answer {
        ServerMessage::CommandResult { ok, output } => {
            assert!(ok && output.contains("upgraded"), "{output}");
        }
        _ => unreachable!(),
    }
    let mut client = env.client("old", 100, 30);
    client.type_line("echo back-again");
    client.wait_for_text("back-again").unwrap();
    assert!(env.log().contains("daemon replaced its program"));
}

/// Installing a newer arcade and attaching is all it takes: the attach finds the daemon older than
/// itself, asks it to continue as this program, and attaches to the same session.
#[test]
fn attaching_with_a_newer_program_upgrades_the_daemon_first() {
    let env = Env::new();
    let new = env.copy_of_program("arcade-new");
    // The daemon says it is 0.0.1; the program that attaches says 9.9.9.
    assert!(
        env.plain_env(
            Path::new(ARCADE),
            &["new", "-d", "-s", "main", "/bin/sh"],
            &["ARCADE_FAKE_VERSION=0.0.1"]
        )
        .status
        .success()
    );
    assert!(!env.replaced());
    let mut host = env.host_with(
        &new,
        &["attach", "-t", "main"],
        &["ARCADE_FAKE_VERSION=9.9.9"],
    );
    host.type_line("echo here-$$");
    type_until_row_starting(&mut host, "echo here-$$", "here-");
    assert!(env.replaced(), "{}", env.log());
    assert!(env.log().contains("upgrade to"), "{}", env.log());
}

#[test]
fn with_the_setting_off_the_attach_does_not_upgrade() {
    let env = Env::new();
    env.set_config("[upgrade]\nauto = false\n");
    let new = env.copy_of_program("arcade-new");
    assert!(
        env.plain_env(
            Path::new(ARCADE),
            &["new", "-d", "-s", "main", "/bin/sh"],
            &["ARCADE_FAKE_VERSION=0.0.1"]
        )
        .status
        .success()
    );
    let mut host = env.host_with(
        &new,
        &["attach", "-t", "main"],
        &["ARCADE_FAKE_VERSION=9.9.9"],
    );
    type_until_row_starting(&mut host, "echo here-$$", "here-");
    assert!(!env.replaced(), "{}", env.log());
}

#[test]
fn the_same_version_does_nothing() {
    let env = Env::new();
    assert!(
        env.plain(&["new", "-d", "-s", "main", "/bin/sh"])
            .status
            .success()
    );
    let mut host = env.host(Path::new(ARCADE), &["attach", "-t", "main"]);
    type_until_row_starting(&mut host, "echo here-$$", "here-");
    assert!(!env.replaced());
    assert!(!env.log().contains("upgrade to"), "{}", env.log());
}

/// When the daemon cannot take the program over, the attach goes on to the old daemon, and the
/// same program is not tried again on every attach.
#[test]
fn a_failed_rehearsal_attaches_to_the_old_daemon_and_is_not_tried_again() {
    let env = Env::new();
    let new = env.copy_of_program("arcade-new");
    assert!(
        env.plain_env(
            Path::new(ARCADE),
            &["new", "-d", "-s", "main", "/bin/sh"],
            &["ARCADE_FAKE_VERSION=0.0.1"]
        )
        .status
        .success()
    );
    // The rehearsal fails once (the file is used up by it); a second try would work.
    std::fs::write(env.socket.with_extension("fault"), "rehearsal-error").unwrap();
    let mut host = env.host_with(
        &new,
        &["attach", "-t", "main"],
        &["ARCADE_FAKE_VERSION=9.9.9"],
    );
    type_until_row_starting(&mut host, "echo one-$$", "one-");
    assert!(!env.replaced(), "{}", env.log());
    assert!(
        env.log().contains("upgrade to"),
        "it was tried: {}",
        env.log()
    );
    host.type_line("arcade run session.detach");
    assert_eq!(host.wait_exit(WAIT), Some(0));
    let mut host = env.host_with(
        &new,
        &["attach", "-t", "main"],
        &["ARCADE_FAKE_VERSION=9.9.9"],
    );
    type_until_row_starting(&mut host, "echo two-$$", "two-");
    assert!(!env.replaced(), "tried again: {}", env.log());
    assert_eq!(env.log().matches("upgrade to").count(), 1, "{}", env.log());
}

/// An older program never replaces a newer daemon.
#[test]
fn an_older_program_never_replaces_a_newer_daemon() {
    let env = Env::new();
    let old = env.copy_of_program("arcade-old");
    assert!(
        env.plain_env(
            Path::new(ARCADE),
            &["new", "-d", "-s", "main", "/bin/sh"],
            &["ARCADE_FAKE_VERSION=9.9.9"]
        )
        .status
        .success()
    );
    let mut host = env.host_with(
        &old,
        &["attach", "-t", "main"],
        &["ARCADE_FAKE_VERSION=0.0.1"],
    );
    type_until_row_starting(&mut host, "echo here-$$", "here-");
    assert!(!env.replaced(), "{}", env.log());
}

/// A daemon of a release from before `arcade upgrade` cannot hand its sessions over. The restart it
/// is offered keeps the session's name, its number of windows and the directories of its panes,
/// and runs against the real released programs (`scripts/fetch-old-releases.sh` puts them under
/// `.tools/old/`; without them the test says so and passes).
#[test]
fn a_daemon_from_before_the_feature_is_restarted_with_its_sessions_made_again() {
    for version in ["0.1.3", "0.1.4"] {
        let old = Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../../.tools/old")
            .join(format!("arcade-{version}"));
        // A program fetched for another system (the directory is shared with a Linux container)
        // cannot run here.
        let runs = Command::new(&old)
            .arg("--version")
            .output()
            .is_ok_and(|o| o.status.success());
        if !old.exists() || !runs {
            eprintln!(
                "skipped {version}: run scripts/fetch-old-releases.sh to fetch {}",
                old.display()
            );
            continue;
        }
        let env = Env::new();
        let start = env.dir.join("start-here");
        std::fs::create_dir_all(&start).unwrap();
        let mut command = Command::new(&old);
        command
            .args(["new", "-d", "-s", "work", "/bin/sh"])
            .env_clear()
            .current_dir(&start)
            .stdin(Stdio::null());
        for entry in env.env() {
            let (k, v) = entry.split_once('=').unwrap();
            command.env(k, v);
        }
        assert!(command.output().unwrap().status.success(), "{version}");
        // A second window, with its shell sent to another directory.
        let mut client = env.client("work", 100, 30);
        client
            .send_message(&arcade_proto::ClientMessage::Command {
                name: "window.new".to_string(),
                args: Vec::new(),
            })
            .unwrap();
        std::thread::sleep(Duration::from_millis(500));
        client.type_line("cd /usr");
        std::thread::sleep(Duration::from_millis(500));
        drop(client);
        // Without a yes (not a terminal, no --restart) nothing changes.
        let out = env.plain(&["upgrade"]);
        assert!(!out.status.success(), "{version}: {}", text(&out));
        assert!(
            text(&out).contains("2 window(s)"),
            "{version}: {}",
            text(&out)
        );
        assert!(text(&out).contains("/usr"), "{version}: {}", text(&out));
        assert!(
            env.sessions().contains("work: 2 windows"),
            "{version}: {}",
            env.sessions()
        );
        // With --restart the old daemon ends and the session is made again.
        let out = env.plain(&["upgrade", "--restart"]);
        assert!(out.status.success(), "{version}: {}", text(&out));
        assert!(
            text(&out).contains("made again with 2 window(s)"),
            "{version}: {}",
            text(&out)
        );
        assert!(
            env.sessions().contains("work: 2 windows"),
            "{version}: {}",
            env.sessions()
        );
        assert!(
            env.log().contains(env!("CARGO_PKG_VERSION")),
            "{version}: {}",
            env.log()
        );
        // The shells of the new daemon: one in each directory the old panes had.
        let daemon = *env.daemon_pids().last().unwrap();
        let table = Command::new("ps")
            .args(["-axo", "pid=,ppid="])
            .output()
            .unwrap();
        let mut pids: Vec<i32> = String::from_utf8_lossy(&table.stdout)
            .lines()
            .filter_map(|l| {
                let mut f = l.split_whitespace().map(|n| n.parse::<i32>().ok());
                let (pid, ppid) = (f.next()??, f.next()??);
                (ppid == daemon as i32).then_some(pid)
            })
            .collect();
        pids.sort_unstable();
        let dirs: Vec<String> = pids
            .iter()
            .map(|pid| {
                if let Ok(p) = std::fs::read_link(format!("/proc/{pid}/cwd")) {
                    return p.display().to_string();
                }
                let out = Command::new("lsof")
                    .args(["-a", "-p", &pid.to_string(), "-d", "cwd", "-Fn"])
                    .output()
                    .unwrap();
                String::from_utf8_lossy(&out.stdout)
                    .lines()
                    .find_map(|l| l.strip_prefix('n').map(str::to_string))
                    .unwrap_or_default()
            })
            .collect();
        assert_eq!(dirs.len(), 2, "{version}: {dirs:?}");
        assert!(dirs[0].ends_with("start-here"), "{version}: {dirs:?}");
        assert_eq!(dirs[1], "/usr", "{version}: {dirs:?}");
    }
}
