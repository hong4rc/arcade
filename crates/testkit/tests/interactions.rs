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
