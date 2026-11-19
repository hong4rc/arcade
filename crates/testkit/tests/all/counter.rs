//! The counter at the right end of the bar says which session holds what it counts, a click on it
//! opens the list of notifications on the most urgent one of the other sessions, and Enter there
//! goes to that pane, switching session.

use arcade_testkit::{AttachOptions, Screened, TestClient, TestDaemon, WAIT};

fn notify(daemon: &TestDaemon, pane: u64, class: &str) {
    let target = format!("target={pane}");
    let class = format!("class={class}");
    let (ok, why) = daemon
        .command(
            "notify.send",
            &[
                "version=1",
                &target,
                &class,
                "source=agent",
                "message=hello",
            ],
        )
        .unwrap();
    assert!(ok, "{why}");
}

/// A detached session called `away`, and a client in `work`; the pane numbers of both.
fn two_sessions(daemon: &TestDaemon) -> (TestClient, u64, u64) {
    let mut away = daemon
        .attach(AttachOptions::new().size(100, 30).named("away"))
        .unwrap();
    away.wait_for_prompt().unwrap();
    let far = away.focused_pane_id();
    away.detach();
    assert!(away.wait_closed(WAIT));
    let mut client = daemon
        .attach(AttachOptions::new().size(100, 30).named("work"))
        .unwrap();
    client.wait_for_prompt().unwrap();
    let near = client.focused_pane_id();
    (client, near, far)
}

fn click(client: &mut TestClient, x: u16, y: u16) {
    client.send(format!("\x1b[<0;{};{}M\x1b[<0;{};{}m", x + 1, y + 1, x + 1, y + 1).as_bytes());
}

/// The column of the first character of `needle` on the bar.
fn column_of(client: &TestClient, needle: &str) -> Option<u16> {
    let bar = client.status_row();
    bar.find(needle).map(|at| bar[..at].chars().count() as u16)
}

fn bar_y(client: &TestClient) -> u16 {
    (client.screen_rows().len() - 1) as u16
}

/// The row of the list that is chosen (it starts with the mark `> `).
fn chosen_row(client: &TestClient) -> Option<String> {
    client
        .screen_rows()
        .into_iter()
        .find(|r| r.contains("> ") && r.contains("waited"))
}

#[test]
fn the_counter_names_the_session_and_a_click_opens_the_list_on_its_row() {
    let daemon = TestDaemon::start();
    let (mut client, near, far) = two_sessions(&daemon);
    // Nothing waits elsewhere: no counter.
    assert!(!client.status_row().contains("away"));
    // A notification in this session that is more urgent than the one in the other session, so
    // the first row of the list is not the one the click is for.
    notify(&daemon, near, "approval");
    notify(&daemon, far, "info");
    assert!(
        client.wait_until(WAIT, |c| c.status_row().contains("away \u{25cf}1")),
        "{}",
        client.status_row()
    );
    let x = column_of(&client, "away").unwrap();
    let y = bar_y(&client);
    click(&mut client, x, y);
    assert!(
        client.wait_until(WAIT, |c| c.screen_text().contains("Notifications")),
        "{}",
        client.screen_text()
    );
    let chosen = chosen_row(&client).unwrap_or_default();
    assert!(chosen.contains("away"), "the row chosen is {chosen:?}");
    // Every row says where it is: session, window, pane.
    let text = client.screen_text();
    assert!(
        text.contains("work") && text.contains("away"),
        "both sessions are named in the list:\n{text}"
    );
    // Enter goes to the pane in the other session.
    client.send(b"\r");
    assert!(
        client.wait_until(WAIT, |c| c.focused_pane_id() == far),
        "still in the pane {}",
        client.focused_pane_id()
    );
    assert!(
        client.wait_until(WAIT, |c| !c.screen_text().contains("Notifications")),
        "{}",
        client.screen_text()
    );
}

#[test]
fn several_sessions_are_named_by_the_one_with_the_most_urgent_notification() {
    let daemon = TestDaemon::start();
    let (mut client, _near, far) = two_sessions(&daemon);
    // A third session with a notification that is less urgent than the one of `away`.
    let mut third = daemon
        .attach(AttachOptions::new().size(100, 30).named("third"))
        .unwrap();
    third.wait_for_prompt().unwrap();
    let third_pane = third.focused_pane_id();
    third.detach();
    assert!(third.wait_closed(WAIT));
    notify(&daemon, third_pane, "info");
    notify(&daemon, far, "input");
    assert!(
        client.wait_until(WAIT, |c| c.status_row().contains("away +1 ")),
        "{}",
        client.status_row()
    );
    let bar = client.status_row();
    assert!(
        bar.contains('\u{25c6}') && bar.contains('\u{25cf}'),
        "{bar}"
    );
}

#[test]
fn a_click_elsewhere_on_the_bar_does_not_open_the_list() {
    let daemon = TestDaemon::start();
    let (mut client, _near, far) = two_sessions(&daemon);
    notify(&daemon, far, "info");
    assert!(client.wait_until(WAIT, |c| c.status_row().contains("away \u{25cf}1")));
    let y = bar_y(&client);
    click(&mut client, 50, y);
    client.wait_until(std::time::Duration::from_millis(400), |_| false);
    assert!(!client.screen_text().contains("Notifications"));
}
