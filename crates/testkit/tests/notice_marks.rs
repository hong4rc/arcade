//! Where the mark of a notification shows, and that it is one cell each time: on the tab of the
//! window, at the top right of a pane out of focus (only in a window of several panes), and in a
//! count at the right end of the status bar that is for the other sessions.

use arcade_daemon::config::StatusCount;
use arcade_testkit::{AttachOptions, Screened, TestClient, TestDaemon, WAIT};

const PREFIX: &[u8] = b"\x1b[109;5u";
const INPUT_MARK: char = '◆';

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

fn start(daemon: &TestDaemon) -> TestClient {
    let mut client = daemon
        .attach(AttachOptions::new().size(100, 30).named("work"))
        .unwrap();
    client.wait_for_prompt().unwrap();
    client
}

/// A window of two panes with the focus in the second; the number of the first is returned.
fn two_panes(client: &mut TestClient) -> u64 {
    let first = client.focused_pane_id();
    split(client);
    first
}

/// Split without asking the pane for its number (that types into it, which clears a
/// notification of the class input).
fn split(client: &mut TestClient) {
    client.send(PREFIX);
    client.send(b"|");
    assert!(client.wait_until(WAIT, |c| c.screen_text().contains('│')));
}

fn marks_in(text: &str) -> usize {
    text.chars().filter(|c| *c == INPUT_MARK).count()
}

fn top_row(client: &TestClient) -> String {
    client.screen_rows().into_iter().next().unwrap_or_default()
}

/// The tab and the pane each wear one cell, and the count at the right end does not say it a
/// third time.
#[test]
fn a_notification_is_one_cell_on_the_tab_and_one_on_the_pane_and_not_counted_again() {
    let daemon = TestDaemon::start();
    let mut client = start(&daemon);
    let first = two_panes(&mut client);
    notify(&daemon, first, "input");
    assert!(
        client.wait_until(WAIT, |c| marks_in(&c.status_row()) == 1),
        "{}",
        client.status_row()
    );
    let bar = client.status_row();
    assert!(bar.contains(&format!(" {INPUT_MARK}")), "{bar}");
    assert!(!bar.contains(&format!("{INPUT_MARK}1")), "{bar}");
    assert!(
        client.wait_until(WAIT, |c| marks_in(&top_row(c)) == 1),
        "{}",
        top_row(&client)
    );
}

/// What the tabs cannot show, a notification in another session, is counted at the right end.
#[test]
fn the_count_at_the_right_end_is_for_the_other_sessions() {
    let daemon = TestDaemon::start();
    let mut other = daemon
        .attach(AttachOptions::new().size(100, 30).named("other"))
        .unwrap();
    other.wait_for_prompt().unwrap();
    let far = other.focused_pane_id();
    other.detach();
    assert!(other.wait_closed(WAIT));
    let mut client = start(&daemon);
    // Nothing yet: nothing is drawn.
    assert_eq!(marks_in(&client.status_row()), 0, "{}", client.status_row());
    let near = two_panes(&mut client);
    notify(&daemon, near, "input");
    assert!(
        client.wait_until(WAIT, |c| marks_in(&c.status_row()) == 1),
        "{}",
        client.status_row()
    );
    assert!(
        !client.status_row().contains(&format!("{INPUT_MARK}1")),
        "{}",
        client.status_row()
    );
    notify(&daemon, far, "input");
    assert!(
        client.wait_until(WAIT, |c| c
            .status_row()
            .trim_end()
            .ends_with(&format!("{INPUT_MARK}1"))),
        "{}",
        client.status_row()
    );
    // The tab, and the count.
    assert_eq!(marks_in(&client.status_row()), 2, "{}", client.status_row());
}

#[test]
fn a_single_pane_and_the_pane_in_focus_wear_no_mark_of_their_own() {
    let daemon = TestDaemon::start();
    let mut client = start(&daemon);
    let only = client.focused_pane_id();
    notify(&daemon, only, "input");
    assert!(
        client.wait_until(WAIT, |c| marks_in(&c.status_row()) == 1),
        "{}",
        client.status_row()
    );
    assert_eq!(marks_in(&top_row(&client)), 0, "{}", top_row(&client));
    // With two panes the pane out of focus (the first, whose notification stands) wears its
    // mark, and the one in focus, with a notification of its own, wears none.
    split(&mut client);
    assert!(
        client.wait_until(WAIT, |c| marks_in(&top_row(c)) == 1),
        "{}",
        top_row(&client)
    );
    let second = client.focused_pane_id();
    notify(&daemon, second, "input");
    client.wait_until(std::time::Duration::from_millis(500), |_| false);
    assert_eq!(marks_in(&top_row(&client)), 1, "{}", top_row(&client));
}

#[test]
fn each_setting_turns_its_place_off() {
    // The tab.
    let daemon = TestDaemon::start_with(|c| c.config.notification.tab_mark = false);
    let mut client = start(&daemon);
    let first = two_panes(&mut client);
    notify(&daemon, first, "input");
    assert!(client.wait_until(WAIT, |c| marks_in(&top_row(c)) == 1));
    assert_eq!(marks_in(&client.status_row()), 0, "{}", client.status_row());
    drop(client);
    drop(daemon);

    // The pane.
    let daemon = TestDaemon::start_with(|c| c.config.notification.pane_mark = false);
    let mut client = start(&daemon);
    let first = two_panes(&mut client);
    notify(&daemon, first, "input");
    assert!(client.wait_until(WAIT, |c| marks_in(&c.status_row()) == 1));
    assert_eq!(marks_in(&top_row(&client)), 0, "{}", top_row(&client));
    drop(client);
    drop(daemon);

    // The count: on for all, off for none.
    for (mode, expected) in [(StatusCount::All, 2), (StatusCount::Off, 1)] {
        let daemon = TestDaemon::start_with(move |c| c.config.notification.status_count = mode);
        let mut client = start(&daemon);
        let first = two_panes(&mut client);
        notify(&daemon, first, "input");
        assert!(
            client.wait_until(WAIT, |c| marks_in(&c.status_row()) == expected),
            "{mode:?}: {}",
            client.status_row()
        );
    }
}
