//! `arcade window log`: the latest changes of the window shown, each with its reason, and the
//! host terminal's focus changes, newest first.

use std::time::Duration;

use arcade_testkit::{AttachOptions, Screened, TestDaemon, WAIT};

const PREFIX: &[u8] = b"\x1b[109;5u";

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
fn the_log_keeps_the_latest_twenty_changes_of_a_session() {
    let daemon = TestDaemon::start();
    let _client = start(&daemon);
    run(&daemon, "window.new", &[]);
    for n in 0..25 {
        run(
            &daemon,
            "window.select",
            &[if n % 2 == 0 { "1" } else { "2" }],
        );
    }
    let changes = log(&daemon)
        .iter()
        .filter(|l| l.contains("  window "))
        .count();
    assert_eq!(changes, 20);
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
