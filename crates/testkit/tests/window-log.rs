//! `arcade window log`: the latest changes of the window shown, each with its reason, and the
//! host terminal's focus changes, newest first.

use std::time::Duration;

use arcade_testkit::{AttachOptions, Screened, TestClient, TestDaemon, WAIT};

const PREFIX: &[u8] = b"\x1b[109;5u";
const FOCUS_IN: &[u8] = b"\x1b[I";
const FOCUS_OUT: &[u8] = b"\x1b[O";
/// Ctrl+Alt+a, the key of `notification.next`.
const NEXT_NOTIFICATION: &[u8] = b"\x1b[97;7u";

fn start(daemon: &TestDaemon) -> arcade_testkit::TestClient {
    let mut client = daemon
        .attach(AttachOptions::new().size(100, 30).named("work"))
        .unwrap();
    client.wait_for_prompt().unwrap();
    client
}

fn log(daemon: &TestDaemon) -> Vec<String> {
    let (ok, output) = daemon.command("window.log", &[]).unwrap();
    assert!(ok, "{output}");
    output.lines().map(str::to_string).collect()
}

#[test]
fn a_key_with_the_prefix_is_the_reason_of_a_window_change() {
    let daemon = TestDaemon::start();
    let mut client = start(&daemon);
    // Window 2 is made (prefix then c), which shows it; then prefix then 1 shows window 1.
    client.send(PREFIX);
    client.send(b"c");
    client.wait_for_prompt().unwrap();
    client.send(PREFIX);
    client.send(b"1");
    assert!(client.wait_until(WAIT, |c| c.status_row().contains(" 1 ")));
    let lines = log(&daemon);
    let to_first = lines
        .iter()
        .position(|l| l.contains("window 2 -> 1") && l.ends_with("prefix then 1"))
        .unwrap_or_else(|| panic!("{lines:?}"));
    assert!(
        lines[to_first].contains("work  window 2 -> 1  prefix then 1"),
        "{lines:?}"
    );
}

#[test]
fn a_command_run_from_outside_names_itself_as_the_reason() {
    let daemon = TestDaemon::start();
    let _client = start(&daemon);
    run(&daemon, "window.new", &[]);
    run(&daemon, "window.select", &["1"]);
    let lines = log(&daemon);
    assert!(
        lines
            .iter()
            .any(|l| l.ends_with("window 2 -> 1  command window.select")),
        "{lines:?}"
    );
}

#[test]
fn the_log_keeps_the_latest_twenty_changes_of_each_kind_and_focus_does_not_push_out_window_changes()
{
    let daemon = TestDaemon::start();
    let mut client = start(&daemon);
    run(&daemon, "window.new", &[]);
    for n in 0..25 {
        run(
            &daemon,
            "window.select",
            &[if n % 2 == 0 { "1" } else { "2" }],
        );
    }
    // Many focus changes after the window changes must not take the window changes out.
    for _ in 0..25 {
        client.send(FOCUS_OUT);
        client.send(FOCUS_IN);
    }
    client.wait_until(Duration::from_millis(300), |_| false);
    let lines = log(&daemon);
    let windows = lines.iter().filter(|l| l.contains("  window ")).count();
    let focus = lines.iter().filter(|l| l.contains("host focus")).count();
    assert_eq!(windows, 20, "{lines:?}");
    assert_eq!(focus, 20, "{lines:?}");
}

#[test]
fn the_host_terminal_losing_and_gaining_the_focus_is_logged_in_order() {
    let daemon = TestDaemon::start();
    let mut client = start(&daemon);
    client.send(b"\x1b[O");
    client.wait_until(Duration::from_millis(200), |_| false);
    client.send(b"\x1b[I");
    client.wait_until(Duration::from_millis(200), |_| false);
    let lines = log(&daemon);
    let gained = lines
        .iter()
        .position(|l| l.ends_with("host focus gained"))
        .unwrap_or_else(|| panic!("{lines:?}"));
    let lost = lines
        .iter()
        .position(|l| l.ends_with("host focus lost"))
        .unwrap_or_else(|| panic!("{lines:?}"));
    // Newest first: the gain was after the loss.
    assert!(gained < lost, "{lines:?}");
}

fn run(daemon: &TestDaemon, name: &str, args: &[&str]) {
    let (ok, output) = daemon.command(name, args).unwrap();
    assert!(ok, "{name} {args:?}: {output}");
}

/// The lines of the log that record a change from window 2 to window 1.
fn to_window_one(lines: &[String]) -> Vec<&String> {
    lines
        .iter()
        .filter(|l| l.contains("window 2 -> 1"))
        .collect()
}

fn daemon_with_auto_jump(name: &str, settings: &str) -> TestDaemon {
    let path =
        std::env::temp_dir().join(format!("arc-window-log-{name}-{}.toml", std::process::id()));
    std::fs::write(&path, settings).unwrap();
    TestDaemon::start_with(move |c| c.config_path = Some(path))
}

fn notify(daemon: &TestDaemon, pane: u64) {
    let target = format!("target={pane}");
    let (ok, why) = daemon
        .command(
            "notify.send",
            &[
                "version=1",
                &target,
                "class=approval",
                "source=test",
                "message=may I",
            ],
        )
        .unwrap();
    assert!(ok, "{why}");
}

/// Window 1 (named A) has the pane `first`; window 2 (named B) is shown.
fn two_windows(daemon: &TestDaemon) -> (TestClient, u64) {
    let mut client = daemon
        .attach(AttachOptions::new().size(100, 30).named("work"))
        .unwrap();
    client.wait_for_prompt().unwrap();
    run(daemon, "window.rename", &["A"]);
    let first = client.focused_pane_id();
    run(daemon, "window.new", &[]);
    run(daemon, "window.rename", &["B"]);
    assert!(client.wait_until(WAIT, |c| c.focused_pane_id() != first));
    (client, first)
}

#[test]
fn the_focus_coming_back_names_the_notification_jump_as_the_reason_when_it_jumps() {
    let daemon = daemon_with_auto_jump("jump", "[notification]\nauto-jump = true\n");
    let (mut client, first) = two_windows(&daemon);
    client.send(FOCUS_OUT);
    notify(&daemon, first);
    assert!(client.wait_until(WAIT, |c| c.status_row().contains('▲')));
    client.send(FOCUS_IN);
    assert!(client.wait_until(WAIT, |c| c.focused_pane_id() == first));
    let (_, log) = daemon.command("window.log", &[]).unwrap();
    let lines: Vec<String> = log.lines().map(str::to_string).collect();
    let change = to_window_one(&lines);
    assert_eq!(change.len(), 1, "{lines:?}");
    assert!(
        change[0].ends_with("a notification jump, when the host terminal got the focus back"),
        "{lines:?}"
    );
}

#[test]
fn a_left_press_on_a_tab_names_the_click_and_its_column() {
    let daemon = TestDaemon::start();
    let (mut client, first) = two_windows(&daemon);
    let rows = client.screen_rows();
    let (y, bar) = (rows.len() - 1, rows[rows.len() - 1].clone());
    let at = bar.find("A  1").unwrap_or_else(|| panic!("{bar:?}"));
    let x = bar[..at].chars().count() as u16;
    client.send(format!("\x1b[<0;{};{}M\x1b[<0;{};{}m", x + 1, y + 1, x + 1, y + 1).as_bytes());
    assert!(client.wait_until(WAIT, |c| c.focused_pane_id() == first));
    let (_, log) = daemon.command("window.log", &[]).unwrap();
    let lines: Vec<String> = log.lines().map(str::to_string).collect();
    let change = to_window_one(&lines);
    assert_eq!(change.len(), 1, "{lines:?}");
    assert!(
        change[0].ends_with(&format!("a click on the bar, column {x}")),
        "{lines:?}"
    );
}

#[test]
fn notification_next_by_its_key_names_the_key() {
    let daemon = TestDaemon::start();
    let (mut client, first) = two_windows(&daemon);
    notify(&daemon, first);
    assert!(client.wait_until(WAIT, |c| c.status_row().contains('▲')));
    client.send(NEXT_NOTIFICATION);
    assert!(client.wait_until(WAIT, |c| c.focused_pane_id() == first));
    let (_, log) = daemon.command("window.log", &[]).unwrap();
    let lines: Vec<String> = log.lines().map(str::to_string).collect();
    let change = to_window_one(&lines);
    assert_eq!(change.len(), 1, "{lines:?}");
    assert!(change[0].ends_with("key ctrl+alt+a"), "{lines:?}");
}

#[test]
fn choosing_a_row_in_the_notification_list_names_the_list() {
    let daemon = TestDaemon::start();
    let (mut client, first) = two_windows(&daemon);
    notify(&daemon, first);
    assert!(client.wait_until(WAIT, |c| c.status_row().contains('▲')));
    client.send(PREFIX);
    client.send(b"a");
    assert!(client.wait_until(WAIT, |c| c.screen_text().contains("Notifications")));
    client.send(b"\r");
    assert!(client.wait_until(WAIT, |c| c.focused_pane_id() == first));
    let (_, log) = daemon.command("window.log", &[]).unwrap();
    let lines: Vec<String> = log.lines().map(str::to_string).collect();
    let change = to_window_one(&lines);
    assert_eq!(change.len(), 1, "{lines:?}");
    assert!(change[0].ends_with("the notification list"), "{lines:?}");
}
