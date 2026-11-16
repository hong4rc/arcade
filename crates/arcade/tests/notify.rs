//! `arcade notify`, the real command, against a daemon with a client attached: what it sends,
//! what shows of it, and how it keeps quiet when something is wrong (it is made for hooks).

use std::path::Path;
use std::process::{Command, Output, Stdio};
use std::time::{Duration, Instant};

use arcade_testkit::{AttachOptions, Screened, TestClient, TestDaemon, WAIT};

const ARCADE: &str = env!("CARGO_BIN_EXE_arcade");
const PREFIX: &[u8] = b"\x1b[109;5u";

fn run(socket: &Path, pane: Option<u64>, args: &[&str]) -> Output {
    let mut command = Command::new(ARCADE);
    command
        .args(args)
        .env_clear()
        .env("ARCADE_SOCKET", socket)
        .env("HOME", "/nonexistent")
        .stdin(Stdio::null());
    if let Some(pane) = pane {
        command.env("ARCADE_PANE", pane.to_string());
    }
    command.output().expect("run arcade")
}

fn out(o: &Output) -> String {
    String::from_utf8_lossy(&o.stdout).to_string()
}

fn err(o: &Output) -> String {
    String::from_utf8_lossy(&o.stderr).to_string()
}

fn bar(c: &TestClient) -> String {
    c.status_row()
}

/// A client whose window has two panes: the focus is in the second, and the number of the first
/// (out of focus) is returned.
fn two_panes(daemon: &TestDaemon) -> (TestClient, u64) {
    let mut client = daemon
        .attach(AttachOptions::new().size(100, 30).named("work"))
        .unwrap();
    client.wait_for_prompt().unwrap();
    let first = client.focused_pane_id();
    client.send(PREFIX);
    client.send(b"|");
    assert!(client.wait_until(WAIT, |c| c.screen_text().contains('│')));
    (client, first)
}

fn shown(client: &mut TestClient, mark: &str) -> bool {
    client.wait_until(WAIT, |c| bar(c).contains(mark))
}

fn gone(client: &mut TestClient, mark: &str) -> bool {
    client.wait_until(WAIT, |c| !bar(c).contains(mark))
}

#[test]
fn a_notification_from_the_command_shows_on_the_pane_it_names() {
    let daemon = TestDaemon::start_counting_all();
    let (mut client, first) = two_panes(&daemon);
    let o = run(
        daemon.socket(),
        None,
        &["notify", "--target", &first.to_string(), "--message", "hi"],
    );
    assert_eq!(o.status.code(), Some(0));
    assert!(out(&o).is_empty() && err(&o).is_empty());
    assert!(shown(&mut client, "●1"), "{}", bar(&client));
}

#[test]
fn the_pane_comes_from_the_environment_when_no_target_is_given() {
    let daemon = TestDaemon::start_counting_all();
    let (mut client, first) = two_panes(&daemon);
    let o = run(
        daemon.socket(),
        Some(first),
        &["notify", "--class", "approval"],
    );
    assert_eq!(o.status.code(), Some(0));
    assert!(shown(&mut client, "▲1"), "{}", bar(&client));
}

#[test]
fn a_class_that_waits_for_the_user_stays_until_the_user_types_into_the_pane() {
    let daemon = TestDaemon::start_counting_all();
    let (mut client, first) = two_panes(&daemon);
    let o = run(
        daemon.socket(),
        Some(first),
        &["notify", "--class", "input"],
    );
    assert_eq!(o.status.code(), Some(0));
    assert!(shown(&mut client, "◆1"), "{}", bar(&client));
    // Looking at the pane does not take it away.
    client.send(PREFIX);
    client.send(b"h");
    std::thread::sleep(Duration::from_millis(300));
    client.read_more(Instant::now() + Duration::from_millis(50));
    assert!(bar(&client).contains("◆1"), "{}", bar(&client));
    // Typing into it does.
    client.send(b"x");
    assert!(gone(&mut client, "◆"), "{}", bar(&client));
}

#[test]
fn an_info_notification_goes_away_when_the_pane_is_looked_at() {
    let daemon = TestDaemon::start_counting_all();
    let (mut client, first) = two_panes(&daemon);
    run(daemon.socket(), Some(first), &["notify"]);
    assert!(shown(&mut client, "●1"), "{}", bar(&client));
    client.send(PREFIX);
    client.send(b"h");
    assert!(gone(&mut client, "●"), "{}", bar(&client));
}

#[test]
fn another_source_with_a_lower_class_does_not_replace_and_the_same_source_does() {
    let daemon = TestDaemon::start_counting_all();
    let (mut client, first) = two_panes(&daemon);
    let pane = first.to_string();
    run(
        daemon.socket(),
        None,
        &[
            "notify", "--target", &pane, "--class", "approval", "--source", "a",
        ],
    );
    assert!(shown(&mut client, "▲1"), "{}", bar(&client));
    run(
        daemon.socket(),
        None,
        &[
            "notify", "--target", &pane, "--class", "info", "--source", "b",
        ],
    );
    let (_, log) = daemon.command("notify.log", &[]).unwrap();
    assert!(log.contains("dropped"), "{log}");
    assert!(bar(&client).contains("▲1"), "{}", bar(&client));
    run(
        daemon.socket(),
        None,
        &[
            "notify", "--target", &pane, "--class", "error", "--source", "a",
        ],
    );
    assert!(shown(&mut client, "■1"), "{}", bar(&client));
    assert!(!bar(&client).contains("▲"), "{}", bar(&client));
}

#[test]
fn clear_takes_one_notification_or_all_of_them_away() {
    let daemon = TestDaemon::start_counting_all();
    let (mut client, first) = two_panes(&daemon);
    let pane = first.to_string();
    run(
        daemon.socket(),
        None,
        &["notify", "--target", &pane, "--class", "error"],
    );
    assert!(shown(&mut client, "■1"), "{}", bar(&client));
    let o = run(
        daemon.socket(),
        None,
        &["notify", "clear", "--target", &pane],
    );
    assert_eq!(o.status.code(), Some(0));
    assert!(gone(&mut client, "■"), "{}", bar(&client));
    run(
        daemon.socket(),
        None,
        &["notify", "--target", &pane, "--class", "input"],
    );
    assert!(shown(&mut client, "◆1"), "{}", bar(&client));
    let o = run(daemon.socket(), None, &["notify", "clear", "--all"]);
    assert_eq!(o.status.code(), Some(0));
    assert!(gone(&mut client, "◆"), "{}", bar(&client));
}

#[test]
fn control_characters_in_the_text_are_not_kept_and_do_not_break_the_screen() {
    let daemon = TestDaemon::start_counting_all();
    let (mut client, first) = two_panes(&daemon);
    let o = run(
        daemon.socket(),
        Some(first),
        &[
            "notify",
            "--class",
            "error",
            "--title",
            "bad\x1b[2Jtitle",
            "--message",
            "a\x07b\x1b]0;owned\x07",
        ],
    );
    assert_eq!(o.status.code(), Some(0));
    assert!(shown(&mut client, "■1"), "{}", bar(&client));
    assert!(!client.screen_text().contains("owned"));
}

#[test]
fn an_unknown_class_is_refused_quietly_unless_strict() {
    let daemon = TestDaemon::start_counting_all();
    let (mut client, first) = two_panes(&daemon);
    let pane = first.to_string();
    let quiet = run(
        daemon.socket(),
        None,
        &["notify", "--target", &pane, "--class", "loud"],
    );
    assert_eq!(quiet.status.code(), Some(0));
    assert!(out(&quiet).is_empty() && err(&quiet).is_empty());
    let strict = run(
        daemon.socket(),
        None,
        &["notify", "--target", &pane, "--class", "loud", "--strict"],
    );
    assert_eq!(strict.status.code(), Some(1));
    assert!(out(&strict).is_empty());
    assert!(err(&strict).contains("unknown class"), "{}", err(&strict));
    std::thread::sleep(Duration::from_millis(200));
    client.read_more(Instant::now() + Duration::from_millis(50));
    assert!(!bar(&client).contains("●"), "{}", bar(&client));
}

#[test]
fn no_pane_or_a_missing_pane_is_an_error_that_only_strict_reports() {
    let daemon = TestDaemon::start_counting_all();
    let quiet = run(daemon.socket(), None, &["notify"]);
    assert_eq!(quiet.status.code(), Some(0));
    assert!(out(&quiet).is_empty() && err(&quiet).is_empty());
    let strict = run(daemon.socket(), None, &["notify", "--strict"]);
    assert_eq!(strict.status.code(), Some(1));
    assert!(err(&strict).contains("no pane"), "{}", err(&strict));
    let missing = run(daemon.socket(), None, &["notify", "--target", "9999"]);
    assert_eq!(missing.status.code(), Some(0));
    let missing = run(
        daemon.socket(),
        None,
        &["notify", "--target", "9999", "--strict"],
    );
    assert_eq!(missing.status.code(), Some(1));
    assert!(err(&missing).contains("no such pane"), "{}", err(&missing));
}

#[test]
fn without_a_daemon_it_is_silent_quick_and_starts_nothing() {
    let dir = std::env::temp_dir().join(format!("arc-notify-none-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let socket = dir.join("none");
    let start = Instant::now();
    let o = run(&socket, Some(1), &["notify", "--message", "x"]);
    let took = start.elapsed();
    assert_eq!(o.status.code(), Some(0));
    assert!(out(&o).is_empty() && err(&o).is_empty());
    assert!(took < Duration::from_millis(400), "{took:?}");
    assert!(!socket.exists(), "a daemon was started");
    let strict = run(&socket, Some(1), &["notify", "--strict"]);
    assert_eq!(strict.status.code(), Some(1));
    assert!(err(&strict).contains("no daemon"), "{}", err(&strict));
    let check = run(&socket, Some(1), &["notify", "check"]);
    assert_ne!(check.status.code(), Some(0));
    assert!(out(&check).contains("not reachable"), "{}", out(&check));
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn a_version_newer_than_the_daemon_knows_is_refused_with_the_supported_one() {
    let daemon = TestDaemon::start_counting_all();
    let (ok, why) = daemon
        .command("notify.send", &["version=2", "target=1"])
        .unwrap();
    assert!(!ok);
    assert!(why.contains("supports version 1"), "{why}");
    // A field that is not known is ignored.
    let (_client, first) = two_panes(&daemon);
    let target = format!("target={first}");
    let (ok, _) = daemon
        .command("notify.send", &["version=1", &target, "colour=red"])
        .unwrap();
    assert!(ok);
}

#[test]
fn check_says_which_pane_and_socket_it_uses_and_sends_a_test() {
    let daemon = TestDaemon::start_counting_all();
    let (mut client, first) = two_panes(&daemon);
    let o = run(daemon.socket(), Some(first), &["notify", "check"]);
    assert_eq!(o.status.code(), Some(0), "{}", out(&o));
    let text = out(&o);
    assert!(text.contains(&format!("pane: {first}")), "{text}");
    assert!(text.contains("ARCADE_PANE"), "{text}");
    assert!(
        text.contains(&daemon.socket().display().to_string()),
        "{text}"
    );
    assert!(shown(&mut client, "●1"), "{}", bar(&client));
    let none = run(daemon.socket(), None, &["notify", "check"]);
    assert_ne!(none.status.code(), Some(0));
}

#[test]
fn the_log_lists_what_became_of_the_signals_and_keeps_no_text() {
    let daemon = TestDaemon::start_counting_all();
    let (_client, first) = two_panes(&daemon);
    run(
        daemon.socket(),
        Some(first),
        &[
            "notify",
            "--class",
            "error",
            "--title",
            "secret-title",
            "--message",
            "secret-body",
        ],
    );
    let o = run(daemon.socket(), None, &["notify", "log"]);
    assert_eq!(o.status.code(), Some(0));
    let text = out(&o);
    assert!(text.contains("cli") && text.contains("error"), "{text}");
    assert!(!text.contains("secret"), "{text}");
}
