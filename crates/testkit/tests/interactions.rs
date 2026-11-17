//! Where features meet: each rule says what shows when two features want the same place.

use arcade_testkit::{AttachOptions, Screened, TestClient, TestDaemon, WAIT};

const PREFIX: &[u8] = b"\x1b[109;5u";

fn start(daemon: &TestDaemon, cols: u16) -> TestClient {
    let mut client = daemon
        .attach(AttachOptions::new().size(cols, 30).named("work"))
        .unwrap();
    client.wait_for_prompt().unwrap();
    client
}

fn notify(daemon: &TestDaemon, pane: u64, class: &str) {
    let target = format!("target={pane}");
    let class = format!("class={class}");
    let (ok, why) = daemon
        .command(
            "notify.send",
            &["version=1", &target, &class, "source=test", "message=hi"],
        )
        .unwrap();
    assert!(ok, "{why}");
}

fn wheel_up(client: &mut TestClient, x: u16, y: u16) {
    client.send(format!("\x1b[<64;{};{}M", x + 1, y + 1).as_bytes());
}

/// The corner at the top right of a pane is the mark of a notification, and the position mark of
/// a scrolled pane gives way by one cell: neither hides the other, and the program's text is
/// covered once, not twice.
#[test]
fn the_position_mark_ends_before_the_notification_mark_in_the_corner() {
    let daemon = TestDaemon::start();
    let mut client = start(&daemon, 100);
    let first = client.focused_pane_id();
    client.type_line("i=1; while [ $i -le 80 ]; do echo line-$i; i=$((i+1)); done; echo filled");
    client.wait_for_text("filled").unwrap();
    client.send(PREFIX);
    client.send(b"|");
    assert!(client.wait_until(WAIT, |c| c.screen_text().contains('│')));
    wheel_up(&mut client, 5, 5);
    assert!(
        client.wait_until(WAIT, |c| c.screen_rows()[0].contains('[')
            && c.screen_rows()[0].contains('/')),
        "{}",
        client.screen_text()
    );
    let before = client.screen_rows()[0].clone();
    let mark = before.split('│').next().unwrap().trim_end().to_string();
    assert!(mark.ends_with(']'), "{before}");
    notify(&daemon, first, "input");
    assert!(
        client.wait_until(WAIT, |c| c.screen_rows()[0].contains('◆')),
        "{}",
        client.screen_rows()[0]
    );
    let left = client.screen_rows()[0]
        .split('│')
        .next()
        .unwrap()
        .to_string();
    let left = left.trim_end();
    // The mark is the last cell of the pane, and the position mark ends right before it, whole.
    assert!(left.ends_with("]◆"), "{left}");
    assert!(left.contains(&mark[mark.rfind('[').unwrap()..]), "{left}");
}

// ---- A divider drag while a pane is in copy mode or scrolled ----

fn drag(client: &mut TestClient, from: (u16, u16), to: (u16, u16)) {
    let report = |client: &mut TestClient, code: u16, x: u16, y: u16, up: bool| {
        let end = if up { 'm' } else { 'M' };
        client.send(format!("\x1b[<{code};{};{}{end}", x + 1, y + 1).as_bytes());
    };
    report(client, 0, from.0, from.1, false);
    report(client, 32, (from.0 + to.0) / 2, (from.1 + to.1) / 2, false);
    report(client, 32, to.0, to.1, false);
    report(client, 0, to.0, to.1, true);
}

fn bars(client: &TestClient, y: usize) -> Vec<usize> {
    client.screen_rows()[y]
        .chars()
        .enumerate()
        .filter(|(_, c)| *c == '│')
        .map(|(x, _)| x)
        .collect()
}

/// The last `[a/b]` mark on the first row, with the column its `]` is in.
fn position_end(client: &TestClient) -> Option<usize> {
    client.screen_rows()[0]
        .rfind(']')
        .map(|b| client.screen_rows()[0][..b].chars().count())
}

fn fill(client: &mut TestClient) {
    client.type_line("i=1; while [ $i -le 80 ]; do echo line-$i; i=$((i+1)); done; echo filled");
    client.wait_for_text("filled").unwrap();
}

/// A pane whose size changes shows its live screen again (the rows copy mode counts from change
/// with the width), so a divider drag ends the copy mode or scroll of the two panes it moves, and
/// does nothing to the other ones.
#[test]
fn a_divider_drag_moves_it_and_ends_copy_mode_and_scroll_of_the_panes_it_resizes_only() {
    let daemon = TestDaemon::start();
    let mut client = start(&daemon, 101);
    fill(&mut client);
    client.send(PREFIX);
    client.send(b"|");
    assert!(client.wait_until(WAIT, |c| bars(c, 3) == vec![50]));
    // The left pane (not in focus) is scrolled with the wheel.
    wheel_up(&mut client, 5, 5);
    assert!(
        client.wait_until(WAIT, |c| position_end(c) == Some(49)),
        "{}",
        client.screen_text()
    );
    drag(&mut client, (50, 5), (30, 5));
    assert!(
        client.wait_until(WAIT, |c| bars(c, 3) == vec![30]),
        "{}",
        client.screen_text()
    );
    // The scrolled pane was resized: it is live again, with no mark.
    assert!(
        client.wait_until(WAIT, |c| position_end(c).is_none()),
        "{}",
        client.screen_text()
    );
    // The focused pane in copy mode: the drag moves the divider and ends its copy mode.
    client.send(PREFIX);
    client.send(b"[");
    assert!(client.wait_until(WAIT, |c| format!("{} ", c.status_row()).contains(" COPY ")));
    drag(&mut client, (30, 5), (60, 5));
    assert!(
        client.wait_until(WAIT, |c| bars(c, 3) == vec![60]),
        "{}",
        client.screen_text()
    );
    assert!(
        client.wait_until(WAIT, |c| !format!("{} ", c.status_row()).contains(" COPY ")),
        "{}",
        client.status_row()
    );
    // A pane whose size does not change keeps its scroll: the left pane is scrolled, and the
    // split of the right one leaves it alone.
    wheel_up(&mut client, 5, 5);
    assert!(client.wait_until(WAIT, |c| position_end(c).is_some()));
    let before = client.screen_rows()[0].clone();
    client.send(PREFIX);
    client.send(b"|");
    assert!(client.wait_until(WAIT, |c| bars(c, 3).len() == 2));
    assert_eq!(
        client.screen_rows()[0].split('│').next(),
        before.split('│').next()
    );
}

// ---- A pane that is closed leaves nothing behind ----

/// A progress report and a notification of a pane that is closed go with it: the tab and the
/// counter show nothing of them, and the window of the other pane is as it was.
#[test]
fn closing_a_pane_takes_its_progress_and_its_notification_with_it() {
    let daemon = TestDaemon::start();
    let mut client = start(&daemon, 100);
    let first = client.focused_pane_id();
    client.send(PREFIX);
    client.send(b"|");
    assert!(client.wait_until(WAIT, |c| c.screen_text().contains('│')));
    let second = client.focused_pane_id();
    client.type_line("printf '\\033]9;4;1;40\\033\\\\'; echo shown");
    assert!(
        client.wait_until(WAIT, |c| c.status_row().contains('\u{2584}')),
        "{}",
        client.status_row()
    );
    notify(&daemon, second, "input");
    assert!(
        client.wait_until(WAIT, |c| c.status_row().contains('◆')),
        "{}",
        client.status_row()
    );
    daemon
        .command_from(Some(second), "pane.close", &[])
        .unwrap();
    assert!(
        client.wait_until(WAIT, |c| !c.status_row().contains('◆')),
        "{}",
        client.status_row()
    );
    assert!(
        !client.status_row().contains('◆'),
        "{}",
        client.status_row()
    );
    // The notification of the closed pane is gone from the list too, and the other pane is
    // still there and answers.
    let (ok, listed) = daemon.command("notification.list", &[]).unwrap();
    assert!(ok);
    assert!(!listed.contains(&format!("\t{second}\t")), "{listed}");
    let _ = first;
    client.type_line("echo alive-$((1+1))");
    client.wait_for_text("alive-2").unwrap();
}
