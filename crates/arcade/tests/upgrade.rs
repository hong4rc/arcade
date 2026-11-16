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
    host.type_line("echo again-$$");
    wait_for_row_starting(&mut host, "again-");
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
        host.type_line("echo still-$$");
        wait_for_row_starting(&mut host, "still-");
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
