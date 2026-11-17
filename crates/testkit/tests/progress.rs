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
    // The job says when it has sent the report (the sum is computed, so that the typed line
    // itself does not show the word): the pane is then known to have had it, however slow the
    // machine is, and the check below is not made before the report exists.
    client.type_line(&format!(
        "(sleep 1; {}; echo SENT-$((40+2))) &",
        report(1, 30)
    ));
    split(&mut client);
    client.wait_for_text("SENT-42").unwrap();
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
    assert!(client.wait_until(WAIT, |c| c.screen_text().contains('\u{2584}')));
    client.type_line(&report(2, 0));
    let bar_ends_in_x = |c: &mut TestClient| c.status_row().contains('\u{2717}');
    assert!(client.wait_until(WAIT, bar_ends_in_x));
    client.type_line(&report(0, 0));
    assert!(client.wait_until(WAIT, |c| {
        let text = c.screen_text();
        !text.contains('\u{2584}') && !c.status_row().contains('\u{2717}')
    }));
}

/// How many times a frame addressed the status bar's row (the last of the 30 rows) from `from` on.
/// The renderer moves to a row with an absolute position, and a bar that did not change is not
/// written at all.
fn bar_writes_since(client: &TestClient, from: usize) -> usize {
    let raw = String::from_utf8_lossy(&client.raw_output[from..]).into_owned();
    raw.matches("\x1b[30;").count()
}

#[test]
fn a_report_that_says_the_same_again_does_not_redraw_the_bar() {
    let daemon = TestDaemon::start();
    let mut client = start(&daemon);
    client.type_line(&report(1, 42));
    assert!(client.wait_until(WAIT, |c| c.screen_text().contains('\u{2584}')));
    // The count sees a write of the bar: showing the first report wrote it.
    assert!(bar_writes_since(&client, 0) >= 1);
    let lap = |client: &mut TestClient, body: &str, marker: &str| {
        let from = client.raw_output.len();
        client.type_line(&format!(
            "i=0; while [ $i -lt 40 ]; do {body}; sleep 0.02; i=$((i+1)); done; echo {marker}"
        ));
        client.wait_for_text(marker).unwrap();
        client.wait_until(Duration::from_millis(300), |_| false);
        bar_writes_since(client, from)
    };
    // The bar is written when it changes (a new report for it to show), so the lap with
    // reports repeats the one that is already shown.
    let with_reports = lap(&mut client, &report(1, 42)[..], "lap-one");
    let without = lap(&mut client, "true", "lap-two");
    // Forty identical reports must not write the bar again: the bar is written when something
    // on it changes (the same as in the lap without reports), not once for each report.
    assert!(
        with_reports <= without + 2,
        "the bar's row was written {with_reports} times with reports and {without} without"
    );
}

// ---- A report that nothing takes away ----

fn with_expiry(seconds: u64) -> TestDaemon {
    TestDaemon::start_with(move |c| {
        c.config.progress_expires = Duration::from_secs(seconds);
    })
}

fn tab_says_running(client: &mut TestClient) -> bool {
    client.status_row().contains('\u{25b8}')
}

/// A program that says "working" once and nothing more, as one that was ended or killed does:
/// the report goes after `progress-expires` seconds, on the tab and on the host terminal's bar.
#[test]
fn a_report_that_nothing_repeats_expires_on_the_tab_and_on_the_host_bar() {
    let daemon = with_expiry(1);
    let mut client = start(&daemon);
    client.type_line(&report(3, 0));
    assert!(shows(&mut client, p(3, 0)));
    assert!(client.wait_until(WAIT, tab_says_running));
    // It goes by itself (a deadline wake-up, no key and no output needed), and the host's own
    // bar is cleared with it.
    assert!(
        client.wait_until(WAIT, |c| !tab_says_running(c)),
        "{}",
        client.status_row()
    );
    assert!(client.wait_until(WAIT, |c| last(c) == Some(p(0, 0))));
}

/// A program that repeats its report keeps it shown for as long as it repeats; when it stops
/// the report goes.
#[test]
fn a_report_that_is_repeated_is_kept_and_goes_when_the_repeats_stop() {
    let daemon = with_expiry(1);
    let mut client = start(&daemon);
    client.type_line(&format!(
        "i=0; while [ $i -lt 12 ]; do {}; sleep 0.25; i=$((i+1)); done; echo repeats-over",
        report(3, 0)
    ));
    assert!(client.wait_until(WAIT, tab_says_running));
    // Three seconds of repeats, three times the limit: still shown.
    client.wait_for_text("repeats-over").ok();
    client.read_more(std::time::Instant::now() + Duration::from_millis(100));
    assert!(tab_says_running(&mut client), "{}", client.status_row());
    assert!(
        client.wait_until(WAIT, |c| !tab_says_running(c)),
        "{}",
        client.status_row()
    );
}

/// Zero turns the expiry off: the report stays until the program takes it away.
#[test]
fn zero_turns_the_expiry_off() {
    let daemon = with_expiry(0);
    let mut client = start(&daemon);
    client.type_line(&report(3, 0));
    assert!(client.wait_until(WAIT, tab_says_running));
    client.wait_until(Duration::from_millis(2500), |_| false);
    assert!(tab_says_running(&mut client), "{}", client.status_row());
    client.type_line(&report(0, 0));
    assert!(client.wait_until(WAIT, |c| !tab_says_running(c)));
}

/// The program that sent a report is gone when the shell shows its next prompt (mark A) or the
/// command ends (mark D): the report goes then, with the expiry off.
#[test]
fn a_new_prompt_or_the_end_of_a_command_takes_the_report_away() {
    for mark in ["A", "D;0"] {
        let daemon = with_expiry(0);
        let mut client = start(&daemon);
        client.type_line(&report(3, 0));
        assert!(client.wait_until(WAIT, tab_says_running), "{mark}");
        client.type_line(&format!("printf '\\033]133;{mark}\\007'"));
        assert!(
            client.wait_until(WAIT, |c| !tab_says_running(c)),
            "{mark}: {}",
            client.status_row()
        );
        assert!(
            client.wait_until(WAIT, |c| last(c) == Some(p(0, 0))),
            "{mark}"
        );
    }
}

/// A report made inside the alternate screen is still there after the program leaves it without
/// taking the report away (nothing about the screen switch clears it); the expiry does.
#[test]
fn a_report_made_in_the_alternate_screen_outlives_it_until_it_expires() {
    let daemon = with_expiry(2);
    let mut client = start(&daemon);
    client.type_line(&format!(
        "printf '\\033[?1049h'; {}; sleep 0.3; printf '\\033[?1049l'; echo left-$((1+1))",
        report(3, 0)
    ));
    assert!(
        client.wait_until(WAIT, tab_says_running),
        "never shown: {}",
        client.status_row()
    );
    client.wait_for_text("left-2").unwrap();
    assert!(
        tab_says_running(&mut client),
        "gone on leaving: {}",
        client.status_row()
    );
    assert!(
        client.wait_until(WAIT, |c| !tab_says_running(c)),
        "{}",
        client.status_row()
    );
}

/// A window moved to another session takes its report with it: the other session shows it, and
/// the session it left does not.
#[test]
fn a_report_moves_with_its_window_and_is_not_left_behind() {
    let daemon = TestDaemon::start();
    let mut alpha = daemon
        .attach(AttachOptions::new().size(100, 30).named("alpha"))
        .unwrap();
    alpha.wait_for_prompt().unwrap();
    let mut beta = daemon
        .attach(AttachOptions::new().size(100, 30).named("beta"))
        .unwrap();
    beta.wait_for_prompt().unwrap();
    assert!(daemon.command("window.new", &[]).is_ok());
    // The window that was made has the focus of alpha or beta; ask each client where it stands.
    let before = alpha.status_row();
    let _ = before;
    let (ok, why) = daemon
        .command_from(Some(alpha.focused_pane_id()), "window.new", &[])
        .unwrap();
    assert!(ok, "{why}");
    alpha.type_line(&report(3, 0));
    assert!(alpha.wait_until(WAIT, tab_says_running));
    let pane = alpha.focused_pane_id();
    let (ok, why) = daemon
        .command_from(Some(pane), "window.move-to-session", &["beta"])
        .unwrap();
    assert!(ok, "{why}");
    // The window is gone from alpha, and so is its mark.
    assert!(
        alpha.wait_until(WAIT, |c| !tab_says_running(c)),
        "{}",
        alpha.status_row()
    );
    // Beta has the window with the report once, not twice.
    assert!(
        beta.wait_until(WAIT, tab_says_running),
        "{}",
        beta.status_row()
    );
    assert_eq!(
        beta.status_row().matches('\u{25b8}').count(),
        1,
        "{}",
        beta.status_row()
    );
}

/// With no prompt marks and the expiry turned off, a report goes at once when the program that
/// made it ends and the shell is back, and stays for as long as that program is in front.
#[test]
fn a_report_goes_when_the_program_that_made_it_ends_and_stays_while_it_runs() {
    let daemon = with_expiry(0);
    let mut client = start(&daemon);
    client.type_line(&format!(
        "sh -c \"{}; sleep 2\"; echo back-$((1+1))",
        report(3, 0)
    ));
    assert!(
        client.wait_until(WAIT, tab_says_running),
        "{}",
        client.status_row()
    );
    // Still in front after a second: the report is still there.
    client.wait_until(Duration::from_millis(1000), |_| false);
    assert!(tab_says_running(&mut client), "{}", client.status_row());
    // Gone when the program ends, without any prompt mark and with the expiry off.
    assert!(
        client.wait_until(WAIT, |c| !tab_says_running(c)),
        "{}",
        client.status_row()
    );
    assert!(client.wait_until(WAIT, |c| last(c) == Some(p(0, 0))));
}
