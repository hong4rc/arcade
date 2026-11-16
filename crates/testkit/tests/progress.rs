//! A program's progress report (`OSC 9;4`) reaches the host terminal while the pane that made it
//! is the focused one of the window on show, and only then; it is never a notification.

use std::time::Duration;

use arcade_term::Progress;
use arcade_testkit::{AttachOptions, Screened, TestClient, TestDaemon, WAIT};

const PREFIX: &[u8] = b"\x1b[109;5u";

fn p(state: u8, value: u8) -> Progress {
    Progress { state, value }
}

fn start(daemon: &TestDaemon) -> TestClient {
    let mut client = daemon
        .attach(AttachOptions::new().size(100, 30).named("prog"))
        .unwrap();
    client.wait_for_prompt().unwrap();
    client
}

fn last(client: &TestClient) -> Option<Progress> {
    client.host_progress.last().copied()
}

fn shows(client: &mut TestClient, report: Progress) -> bool {
    client.wait_until(WAIT, |c| last(c) == Some(report))
}

fn report(state: u8, value: u8) -> String {
    format!("printf '\\033]9;4;{state};{value}\\007'")
}

/// Split, so that the pane that was in focus is on the left and a fresh one has the focus.
fn split(client: &mut TestClient) {
    client.send(PREFIX);
    client.send(b"|");
    assert!(client.wait_until(WAIT, |c| c.screen_text().contains('│')));
    client.wait_for_prompt().ok();
}

fn focus(client: &mut TestClient, key: &[u8]) {
    client.send(PREFIX);
    client.send(key);
}

#[test]
fn a_report_in_the_focused_pane_reaches_the_host_and_a_repeat_does_too() {
    let daemon = TestDaemon::start();
    let mut client = start(&daemon);
    client.type_line(&report(1, 50));
    assert!(shows(&mut client, p(1, 50)));
    let before = client.host_progress.len();
    // Programs repeat the same report to keep the bar shown: it is passed on again.
    client.type_line(&report(1, 50));
    assert!(client.wait_until(WAIT, |c| c.host_progress.len() > before));
    client.type_line(&report(2, 80));
    assert!(shows(&mut client, p(2, 80)));
    client.type_line(&report(3, 0));
    assert!(shows(&mut client, p(3, 0)));
    client.type_line(&report(0, 0));
    assert!(shows(&mut client, p(0, 0)));
}

#[test]
fn a_report_in_another_pane_is_held_back_until_that_pane_has_the_focus() {
    let daemon = TestDaemon::start();
    let mut client = start(&daemon);
    client.type_line(&format!("(sleep 1; {}) &", report(1, 30)));
    split(&mut client);
    std::thread::sleep(Duration::from_millis(2000));
    client.wait_until(Duration::from_millis(200), |_| false);
    assert!(
        !client.host_progress.contains(&p(1, 30)),
        "{:?}",
        client.host_progress
    );
    focus(&mut client, b"h");
    assert!(shows(&mut client, p(1, 30)));
    // Moving away takes the bar away.
    focus(&mut client, b"l");
    assert!(shows(&mut client, p(0, 0)));
}

#[test]
fn the_end_of_the_program_and_a_detach_take_the_bar_away() {
    let daemon = TestDaemon::start();
    let mut client = start(&daemon);
    split(&mut client);
    focus(&mut client, b"h");
    client.type_line(&format!("{}; sleep 1; exit", report(1, 60)));
    assert!(shows(&mut client, p(1, 60)));
    // The pane ends, the focus goes to the other one, which has none.
    assert!(shows(&mut client, p(0, 0)));

    let mut other = daemon
        .attach(AttachOptions::new().size(100, 30).named("second"))
        .unwrap();
    other.wait_for_prompt().unwrap();
    other.type_line(&report(1, 10));
    assert!(shows(&mut other, p(1, 10)));
    other.detach();
    assert!(other.wait_until(WAIT, |c| last(c) == Some(p(0, 0))));
}

#[test]
fn a_report_that_is_not_valid_is_not_passed_on_and_none_is_a_notification() {
    let daemon = TestDaemon::start();
    let mut client = start(&daemon);
    client.type_line(&report(1, 40));
    assert!(shows(&mut client, p(1, 40)));
    let count = client.host_progress.len();
    client.type_line(
        "printf '\\033]9;4;7;10\\007\\033]9;4;1;101\\007\\033]9;4;1;5;5\\007'; echo checked",
    );
    client.wait_for_row("checked").unwrap();
    client.wait_until(Duration::from_millis(300), |_| false);
    assert_eq!(client.host_progress.len(), count);
    assert!(
        !client.status_row().contains("●"),
        "{}",
        client.status_row()
    );
    // Nothing of the program's other bytes goes along.
    assert!(client.host_notifications.is_empty());
}

#[test]
fn the_tab_of_a_window_shows_the_progress_of_its_panes_until_it_is_removed() {
    let daemon = TestDaemon::start();
    let mut client = start(&daemon);
    client.type_line(&report(1, 42));
    assert!(client.wait_until(WAIT, |c| c.screen_text().contains("42%")));
    client.type_line(&report(2, 0));
    let bar_ends_in_x = |c: &mut TestClient| c.screen_text().trim_end().ends_with(" err");
    assert!(client.wait_until(WAIT, bar_ends_in_x));
    client.type_line(&report(0, 0));
    assert!(client.wait_until(WAIT, |c| {
        let text = c.screen_text();
        !text.contains("42%") && !text.trim_end().ends_with(" err")
    }));
}

/// The bytes of screen output, not counting the progress sequences passed on to the host.
fn screen_bytes(client: &TestClient) -> usize {
    let raw = String::from_utf8_lossy(&client.raw_output).into_owned();
    raw.replace("\x1b]9;4;1;42\x1b\\", "").len()
}

#[test]
fn a_report_that_says_the_same_again_does_not_redraw_the_bar() {
    let daemon = TestDaemon::start();
    let mut client = start(&daemon);
    client.type_line(&report(1, 42));
    assert!(client.wait_until(WAIT, |c| c.screen_text().contains("42%")));
    let lap = |client: &mut TestClient, body: &str, marker: &str| {
        let before = screen_bytes(client);
        client.type_line(&format!(
            "i=0; while [ $i -lt 40 ]; do {body}; sleep 0.02; i=$((i+1)); done; echo {marker}"
        ));
        client.wait_for_text(marker).unwrap();
        client.wait_until(Duration::from_millis(300), |_| false);
        screen_bytes(client) - before
    };
    let with_reports = lap(&mut client, &report(1, 42)[..], "lap-one");
    let without = lap(&mut client, "true", "lap-two");
    // The two laps type about the same text; 40 reports must not add a bar's worth of cells each.
    assert!(
        with_reports < without + 400,
        "with {with_reports} bytes, without {without}"
    );
}
