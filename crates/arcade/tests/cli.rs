//! The real `arcade` command, run the way a person runs it: in a terminal (a pseudo terminal the
//! test owns, with a virtual screen) on a private socket. These cover core scenarios 1, 2, 8 and
//! 9 and the behaviour of the host terminal around them.

use std::os::unix::net::UnixListener;
use std::path::{Path, PathBuf};
use std::process::{Command, Output, Stdio};
use std::sync::atomic::{AtomicU32, Ordering};
use std::time::{Duration, Instant};

use arcade_proto::{ClientMessage, Decoder, PROTOCOL_VERSION, ServerMessage, encode};
use arcade_testkit::{PtyHost, Screened, WAIT};

const ARCADE: &str = env!("CARGO_BIN_EXE_arcade");

static COUNTER: AtomicU32 = AtomicU32::new(0);

/// A private directory and socket for one test. Stops the daemon when dropped.
struct Env {
    dir: PathBuf,
    socket: PathBuf,
}

impl Env {
    fn new() -> Env {
        let dir = PathBuf::from(format!(
            "/tmp/arc-cli-{}-{}",
            std::process::id(),
            COUNTER.fetch_add(1, Ordering::Relaxed)
        ));
        std::fs::create_dir_all(&dir).unwrap();
        // The pane's shell is a login shell, which reads this file after the system's own
        // profile; it makes the prompt the same on every machine, and puts the directory of the
        // `arcade` under test back on the path, where some systems' profiles replace the path.
        let bin_dir = Path::new(ARCADE).parent().unwrap().display().to_string();
        std::fs::write(
            dir.join(".profile"),
            format!("PS1='$ '\nPATH={bin_dir}:$PATH\nexport PATH\n"),
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

    /// Run `arcade` in a terminal of 80 by 24.
    fn host(&self, args: &[&str]) -> PtyHost {
        self.host_sized(args, 80, 24)
    }

    fn host_sized(&self, args: &[&str], cols: u16, rows: u16) -> PtyHost {
        PtyHost::spawn(Path::new(ARCADE), args, self.env(), &self.dir, cols, rows)
    }

    /// Run `arcade` with no terminal and collect what it prints.
    fn plain(&self, args: &[&str]) -> Output {
        let mut command = Command::new(ARCADE);
        command.args(args).env_clear().stdin(Stdio::null());
        for entry in self.env() {
            let (k, v) = entry.split_once('=').unwrap();
            command.env(k, v);
        }
        command.output().expect("run arcade")
    }

    fn sessions(&self) -> String {
        let out = self.plain(&["ls"]);
        String::from_utf8_lossy(&out.stdout).to_string()
    }

    fn wait_no_daemon(&self) {
        let start = Instant::now();
        while self.socket.exists() {
            assert!(start.elapsed() < WAIT, "the daemon is still there");
            std::thread::sleep(Duration::from_millis(10));
        }
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

fn stdout(out: &Output) -> String {
    String::from_utf8_lossy(&out.stdout).to_string()
}

fn stderr(out: &Output) -> String {
    String::from_utf8_lossy(&out.stderr).to_string()
}

/// Scenario 1: `arcade` on a machine with no daemon starts one, with a session and a pane that
/// runs a shell, and a typed command shows its result.
#[test]
fn running_arcade_starts_the_daemon_and_shows_a_shell() {
    let env = Env::new();
    assert!(!env.socket.exists());
    let mut host = env.host(&[]);
    host.wait_for_prompt().unwrap();
    assert!(env.socket.exists(), "no daemon socket");

    host.type_line("echo sum-is-$((40+2))");
    host.wait_for_row("sum-is-42").unwrap();

    let ls = env.sessions();
    assert_eq!(ls.lines().count(), 1, "{ls:?}");
    assert!(ls.contains("1 pane (attached)"), "{ls:?}");
}

#[test]
fn the_host_terminal_shows_the_alternate_screen_while_attached() {
    let env = Env::new();
    // Something on the normal screen first, which must come back afterwards.
    let mut host = env.host(&["new", "-s", "main"]);
    host.wait_for_prompt().unwrap();
    host.type_line("echo inside");
    host.wait_for_row("inside").unwrap();
    assert_eq!(host.screen.mode_alt_screen(), Some(true));
    host.type_line("exit");
    assert_eq!(host.wait_exit(WAIT), Some(0), "{}", host.screen_text());
    assert_eq!(host.screen.mode_alt_screen(), Some(false));
}

/// Leaving the client puts the terminal back as it was, by every way of leaving.
#[test]
fn the_terminal_settings_come_back_after_the_client_exits() {
    let env = Env::new();
    let mut host = env.host(&["new", "-s", "a"]);
    host.wait_for_prompt().unwrap();
    // In raw mode the terminal differs from what it was.
    assert_ne!(&host.settings(), host.initial_settings());
    host.type_line("exit");
    assert_eq!(host.wait_exit(WAIT), Some(0));
    assert_eq!(&host.settings(), host.initial_settings());
}

/// The pane turns on key modes; the host terminal shares them while attached and has them off
/// again once the client has left.
#[test]
fn key_modes_are_shared_while_attached_and_reset_on_leaving() {
    use arcade_term::Mode;
    let env = Env::new();
    let mut host = env.host(&["new", "-s", "modes"]);
    host.wait_for_prompt().unwrap();
    host.type_line("printf '\\033[?1h\\033[?2004h'; echo modes-set");
    host.wait_for_row("modes-set").unwrap();
    let on = host.wait_until(WAIT, |h| {
        h.screen.mode(Mode::AppCursorKeys) == Some(true)
            && h.screen.mode(Mode::BracketedPaste) == Some(true)
    });
    assert!(
        on,
        "the host terminal did not get the modes:\n{}",
        host.screen_text()
    );
    host.type_line("arcade run session.detach");
    assert_eq!(host.wait_exit(WAIT), Some(0));
    assert_eq!(host.screen.mode(Mode::AppCursorKeys), Some(false));
    assert_eq!(host.screen.mode(Mode::BracketedPaste), Some(false));
}

#[test]
fn the_terminal_settings_come_back_after_a_detach_command() {
    let env = Env::new();
    let mut host = env.host(&["new", "-s", "a"]);
    host.wait_for_prompt().unwrap();
    host.type_line("arcade run session.detach");
    assert_eq!(host.wait_exit(WAIT), Some(0), "{}", host.screen_text());
    assert_eq!(&host.settings(), host.initial_settings());
    host.wait_for_text("[detached]").unwrap();
    // Cursor and screen are back to normal too.
    assert_eq!(host.screen.mode_alt_screen(), Some(false));
}

#[test]
fn the_terminal_settings_come_back_after_a_termination_signal() {
    for signal in [libc::SIGTERM, libc::SIGHUP, libc::SIGINT] {
        let env = Env::new();
        let mut host = env.host(&["new", "-s", "a"]);
        host.wait_for_prompt().unwrap();
        host.signal(signal);
        assert_eq!(
            host.wait_exit(WAIT),
            Some(0),
            "signal {signal}: {}",
            host.screen_text()
        );
        assert_eq!(&host.settings(), host.initial_settings(), "signal {signal}");
        // The session lives on.
        assert!(env.sessions().contains("a: 1 pane"), "signal {signal}");
    }
}

/// Scenario 2: the host terminal goes away in the middle of a command; attaching again shows the
/// command carried on, and what it wrote while nobody watched is in the scrollback.
#[test]
fn a_command_keeps_running_while_the_terminal_is_gone() {
    let env = Env::new();
    let mut first = env.host_sized(&["new", "-s", "work"], 80, 24);
    first.wait_for_prompt().unwrap();
    // Wait a moment, then write far more lines than the screen holds, then a final line.
    first.type_line("sleep 1; seq 1 200; echo finished-while-away");
    first.wait_for_text("sleep 1").unwrap();
    // The host terminal closes: the program in it gets a hangup.
    drop(first);

    let mut second = env.host_sized(&["attach", "-t", "work"], 80, 24);
    // The typed command line contains the same words, so wait for the row that is exactly the
    // output.
    let done = second.wait_until(Duration::from_secs(15), |h| {
        h.screen_rows().iter().any(|r| r == "finished-while-away")
    });
    assert!(done, "{}", second.screen_text());
    assert!(
        second.screen_rows().iter().any(|r| r == "200"),
        "{}",
        second.screen_text()
    );
    // Line 1 scrolled off long ago; it is not on this screen...
    assert!(!second.screen_rows().iter().any(|r| r == "1"));
    // ...but it is in the scrollback: a taller terminal pulls it back into view.
    second.resize(80, 260);
    let pulled = second.wait_until(WAIT, |h| {
        let rows = h.screen_rows();
        ["1", "2", "100", "199"]
            .iter()
            .all(|want| rows.iter().any(|r| r == want))
    });
    assert!(
        pulled,
        "the scrollback did not come back:\n{}",
        second.screen_text()
    );
}

#[test]
fn detach_then_attach_shows_the_session_as_it_was() {
    let env = Env::new();
    let mut host = env.host(&["new", "-s", "keep"]);
    host.wait_for_prompt().unwrap();
    host.type_line("echo marker-one");
    host.wait_for_row("marker-one").unwrap();
    host.type_line("arcade run session.detach");
    assert_eq!(host.wait_exit(WAIT), Some(0));
    assert!(
        env.sessions().contains("keep: 1 pane"),
        "{}",
        env.sessions()
    );
    assert!(!env.sessions().contains("(attached)"));

    let mut again = env.host(&["attach", "-t", "keep"]);
    again.wait_for_row("marker-one").unwrap();
    again.type_line("echo marker-two");
    again.wait_for_row("marker-two").unwrap();
}

/// Scenario 8: a second client attaches to the same session; the first is detached with a
/// line telling why, and the second works.
#[test]
fn a_second_client_replaces_the_first() {
    let env = Env::new();
    let mut first = env.host(&["new", "-s", "shared"]);
    first.wait_for_prompt().unwrap();
    first.type_line("echo from-first");
    first.wait_for_row("from-first").unwrap();

    let mut second = env.host(&["attach", "-t", "shared"]);
    second.wait_for_row("from-first").unwrap();

    assert_eq!(first.wait_exit(WAIT), Some(0), "{}", first.screen_text());
    first.wait_for_text("another client attached").unwrap();
    assert_eq!(&first.settings(), first.initial_settings());

    second.type_line("echo from-second");
    second.wait_for_row("from-second").unwrap();
}

#[test]
fn the_default_command_attaches_to_the_session_used_last() {
    let env = Env::new();
    let mut a = env.host(&["new", "-s", "older"]);
    a.wait_for_prompt().unwrap();
    a.type_line("echo i-am-older");
    a.wait_for_row("i-am-older").unwrap();
    a.type_line("arcade run session.detach");
    a.wait_exit(WAIT).unwrap();

    let mut b = env.host(&["new", "-s", "newer"]);
    b.wait_for_prompt().unwrap();
    b.type_line("echo i-am-newer");
    b.wait_for_row("i-am-newer").unwrap();
    b.type_line("arcade run session.detach");
    b.wait_exit(WAIT).unwrap();

    let mut c = env.host(&[]);
    c.wait_for_row("i-am-newer").unwrap();
    assert!(!c.screen_rows().iter().any(|r| r == "i-am-older"));
}

#[test]
fn new_always_creates_another_session() {
    let env = Env::new();
    let mut a = env.host(&["new", "-s", "one"]);
    a.wait_for_prompt().unwrap();
    let mut b = env.host(&["new"]);
    b.wait_for_prompt().unwrap();
    let ls = env.sessions();
    assert_eq!(ls.lines().count(), 2, "{ls}");
    assert!(ls.contains("one: 1 pane (attached)"));
}

#[test]
fn new_with_a_command_runs_it_and_the_session_ends_with_it() {
    let env = Env::new();
    let mut host = env.host(&["new", "-s", "job", "sh", "-c", "echo ran-directly; read x"]);
    host.wait_for_row("ran-directly").unwrap();
    // The command is the pane's first process: when it ends, the session and the daemon end.
    host.send(b"\r");
    assert_eq!(host.wait_exit(WAIT), Some(0));
    // Leaving puts the normal screen back, and the one line says what happened.
    host.wait_for_text("[job: exited]").unwrap();
    env.wait_no_daemon();
}

#[test]
fn a_duplicate_session_name_is_refused() {
    let env = Env::new();
    let mut a = env.host(&["new", "-s", "same"]);
    a.wait_for_prompt().unwrap();
    let mut b = env.host(&["new", "-s", "same"]);
    assert_eq!(b.wait_exit(WAIT), Some(1));
    b.wait_for_text("duplicate session: same").unwrap();
}

#[test]
fn attaching_to_a_missing_session_fails_with_a_message() {
    let env = Env::new();
    let mut a = env.host(&["new", "-s", "real"]);
    a.wait_for_prompt().unwrap();
    let mut b = env.host(&["attach", "-t", "imaginary"]);
    assert_eq!(b.wait_exit(WAIT), Some(1));
    b.wait_for_text("no such session: imaginary").unwrap();
}

#[test]
fn without_a_daemon_attach_ls_and_kill_say_so() {
    let env = Env::new();
    for args in [
        &["attach"][..],
        &["ls"],
        &["kill-server"],
        &["kill-session"],
        &["run", "session.detach"],
    ] {
        let out = env.plain(args);
        // `attach` needs a terminal as well; either message is about not being able to start.
        assert_eq!(out.status.code(), Some(1), "{args:?}: {}", stderr(&out));
        assert!(!env.socket.exists(), "{args:?} must not start a daemon");
    }
    let out = env.plain(&["ls"]);
    assert!(
        stderr(&out).contains("no server running"),
        "{}",
        stderr(&out)
    );
    let mut host = env.host(&["attach"]);
    assert_eq!(host.wait_exit(WAIT), Some(1));
    host.wait_for_text("no server running").unwrap();
    assert!(!env.socket.exists());
}

#[test]
fn arcade_needs_a_terminal_to_attach() {
    let env = Env::new();
    let out = env.plain(&[]);
    assert_eq!(out.status.code(), Some(1));
    assert!(stderr(&out).contains("terminal"), "{}", stderr(&out));
}

#[test]
fn kill_session_ends_one_session_and_kill_server_ends_everything() {
    let env = Env::new();
    let mut a = env.host(&["new", "-s", "first"]);
    a.wait_for_prompt().unwrap();
    let mut b = env.host(&["new", "-s", "second"]);
    b.wait_for_prompt().unwrap();

    let out = env.plain(&["kill-session", "-t", "first"]);
    assert_eq!(out.status.code(), Some(0), "{}", stderr(&out));
    assert_eq!(a.wait_exit(WAIT), Some(0));
    let ls = env.sessions();
    assert!(!ls.contains("first"), "{ls}");
    assert!(ls.contains("second"), "{ls}");

    let out = env.plain(&["kill-server"]);
    assert_eq!(out.status.code(), Some(0), "{}", stderr(&out));
    assert_eq!(b.wait_exit(WAIT), Some(0));
    env.wait_no_daemon();
}

#[test]
fn the_window_size_follows_the_host_terminal() {
    let env = Env::new();
    let mut host = env.host_sized(&["new", "-s", "size"], 90, 28);
    host.wait_for_prompt().unwrap();
    host.type_line("stty size");
    host.wait_for_row("28 90").unwrap();
    host.wait_for_prompt().unwrap();
    host.resize(120, 40);
    host.type_line("stty size");
    host.wait_for_row("40 120").unwrap();
}

#[test]
fn the_pane_can_run_a_command_back_on_its_own_daemon() {
    let env = Env::new();
    let mut host = env.host(&["new", "-s", "self"]);
    host.wait_for_prompt().unwrap();
    // `arcade` finds its daemon through the ARCADE variable the daemon put in the pane.
    host.type_line("arcade ls");
    host.wait_for_row("self: 1 pane (attached)").unwrap();
}

#[test]
fn unknown_commands_print_usage_and_fail() {
    let env = Env::new();
    let out = env.plain(&["frobnicate"]);
    assert_eq!(out.status.code(), Some(2));
    assert!(stderr(&out).contains("usage"), "{}", stderr(&out));
    let out = env.plain(&["--help"]);
    assert_eq!(out.status.code(), Some(0));
    assert!(stdout(&out).contains("kill-server"));
}

// ---- Scenario 9: a daemon of another version ----

/// A pretend daemon that answers every Hello with a mismatch, and remembers a `server.kill`.
fn fake_old_daemon(socket: &Path, daemon_version: u32) -> std::thread::JoinHandle<Vec<String>> {
    let listener = UnixListener::bind(socket).unwrap();
    let path = socket.to_path_buf();
    std::thread::spawn(move || {
        let mut commands = Vec::new();
        // Serve a few connections, then stop.
        listener.set_nonblocking(false).unwrap();
        for _ in 0..4 {
            // After a kill request there is nothing more to serve.
            if !commands.is_empty() {
                break;
            }
            let Ok((mut stream, _)) = listener.accept() else {
                break;
            };
            stream.set_read_timeout(Some(Duration::from_secs(5))).ok();
            let mut decoder = Decoder::<ClientMessage>::new();
            let mut buf = [0u8; 4096];
            'conn: loop {
                let n = match std::io::Read::read(&mut stream, &mut buf) {
                    Ok(0) | Err(_) => break,
                    Ok(n) => n,
                };
                decoder.push(&buf[..n]);
                while let Ok(Some(message)) = decoder.next_message() {
                    let mut out = Vec::new();
                    match message {
                        ClientMessage::Hello { version } => {
                            encode(
                                &ServerMessage::VersionMismatch {
                                    daemon: daemon_version,
                                    client: version,
                                },
                                &mut out,
                            )
                            .unwrap();
                        }
                        ClientMessage::Command { name, .. } => {
                            commands.push(name);
                            encode(
                                &ServerMessage::CommandResult {
                                    ok: true,
                                    output: String::new(),
                                },
                                &mut out,
                            )
                            .unwrap();
                        }
                        _ => {}
                    }
                    std::io::Write::write_all(&mut stream, &out).unwrap();
                    if !commands.is_empty() {
                        break 'conn;
                    }
                }
            }
        }
        let _ = std::fs::remove_file(&path);
        commands
    })
}

#[test]
fn a_daemon_of_another_version_is_reported_with_both_versions() {
    let env = Env::new();
    let _daemon = fake_old_daemon(&env.socket, 77);
    let mut host = env.host(&[]);
    assert_eq!(host.wait_exit(WAIT), Some(1), "{}", host.screen_text());
    let text = host.screen_text();
    assert!(text.contains("77"), "daemon version missing: {text}");
    assert!(
        text.contains(&PROTOCOL_VERSION.to_string()),
        "client version missing: {text}"
    );
    assert!(text.contains("kill-server"), "no hint what to do: {text}");
    // Nothing was left changed on the terminal.
    assert_eq!(&host.settings(), host.initial_settings());
    assert_eq!(host.screen.mode_alt_screen(), Some(false));
}

#[test]
fn other_commands_report_the_mismatch_too() {
    let env = Env::new();
    let _daemon = fake_old_daemon(&env.socket, 77);
    let out = env.plain(&["ls"]);
    assert_eq!(out.status.code(), Some(1));
    let text = stderr(&out);
    assert!(
        text.contains("77") && text.contains("kill-server"),
        "{text}"
    );
}

#[test]
fn kill_server_still_reaches_a_daemon_of_another_version() {
    let env = Env::new();
    let daemon = fake_old_daemon(&env.socket, 77);
    let out = env.plain(&["kill-server"]);
    assert_eq!(out.status.code(), Some(0), "{}", stderr(&out));
    assert_eq!(daemon.join().unwrap(), vec!["server.kill".to_string()]);
}

/// Not part of the normal run, because it depends on the machine: starts `arcade` with the
/// user's own shell and configuration and prints what the screen shows. Run it with
/// `cargo test -p arcade --test cli real_shell -- --ignored --nocapture`.
#[test]
#[ignore = "uses the user's own shell and home directory"]
fn real_shell_smoke() {
    let env = Env::new();
    let home = std::env::var("HOME").unwrap();
    let shell = std::env::var("SHELL").unwrap_or_else(|_| "/bin/sh".to_string());
    let mut vars = env.env();
    vars.retain(|v| !v.starts_with("HOME=") && !v.starts_with("SHELL=") && !v.starts_with("PS1="));
    vars.push(format!("HOME={home}"));
    vars.push(format!("SHELL={shell}"));
    let mut host = PtyHost::spawn(
        Path::new(ARCADE),
        &["new", "-s", "smoke"],
        vars,
        Path::new(&home),
        100,
        30,
    );
    // Give a slow shell start-up time, then run a command and show the screen.
    std::thread::sleep(Duration::from_secs(3));
    host.type_line("echo smoke-$((6*7)); printf 'tone: Tie\\xcc\\x82\\xcc\\x81ng 汉字\\n'");
    host.wait_until(Duration::from_secs(10), |h| {
        h.screen_rows().iter().any(|r| r == "smoke-42")
    });
    let show = |h: &PtyHost| {
        h.screen_rows()
            .iter()
            .enumerate()
            .filter(|(_, r)| !r.is_empty())
            .map(|(i, r)| format!("{i:>2}| {r}"))
            .collect::<Vec<_>>()
            .join("\n")
    };
    // The screen is drawn in one piece, but it arrives in several reads; let it settle.
    host.wait_until(Duration::from_millis(700), |_| false);
    println!(
        "--- through arcade ({shell}) ---\n{}\ncursor {:?}",
        show(&host),
        host.screen.cursor_position()
    );
    // The same shell, run directly in a terminal of the same size, for comparison.
    let mut direct_env = env.env();
    direct_env
        .retain(|v| !v.starts_with("HOME=") && !v.starts_with("SHELL=") && !v.starts_with("PS1="));
    direct_env.push(format!("HOME={home}"));
    let mut direct = PtyHost::spawn(
        Path::new(&shell),
        &["-l"],
        direct_env,
        Path::new(&home),
        100,
        30,
    );
    std::thread::sleep(Duration::from_secs(3));
    direct.type_line("echo smoke-$((6*7)); printf 'tone: Tie\\xcc\\x82\\xcc\\x81ng 汉字\\n'");
    direct.wait_until(Duration::from_secs(10), |h| {
        h.screen_rows().iter().any(|r| r == "smoke-42")
    });
    println!(
        "--- directly ---\n{}\ncursor {:?}",
        show(&direct),
        direct.screen.cursor_position()
    );
    host.type_line("exit");
    println!("exit code {:?}", host.wait_exit(WAIT));
}
