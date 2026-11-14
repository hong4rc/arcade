//! Notifications through the daemon: what a program in a pane can ask for with a bell or an
//! escape sequence, what shows of it, and what takes it away.
//!
//! A pane that is looked at does not keep a signal of the class that goes away at a glance, so
//! most tests make the signal come from a pane that is not in focus: a command is started in
//! the background of a pane, and the pane is left by splitting the window.

use std::time::Duration;

use arcade_testkit::{AttachOptions, Screened, TestClient, TestDaemon, WAIT};

const PREFIX: &[u8] = b"\x1b[109;5u";

fn bar(client: &TestClient) -> String {
    client.status_row()
}

fn start(daemon: &TestDaemon) -> TestClient {
    let mut client = daemon
        .attach(AttachOptions::new().size(100, 30).named("work"))
        .unwrap();
    client.wait_for_prompt().unwrap();
    client
}

/// In the pane that has the focus, run `command` after a second in the background, then leave
/// the pane for a new one on its right, so that the output comes from a pane out of focus.
fn signal_from_a_pane_left_behind(client: &mut TestClient, printf_argument: &str) {
    client.type_line(&format!("(sleep 1; printf '{printf_argument}') &"));
    client.send(PREFIX);
    client.send(b"|");
    assert!(client.wait_until(WAIT, |c| c.screen_text().contains('│')));
}

fn mark_shown(client: &mut TestClient, mark: &str) -> bool {
    client.wait_until(WAIT, |c| bar(c).contains(mark))
}

fn nothing_shown_for(client: &mut TestClient, mark: &str, wait: Duration) -> bool {
    std::thread::sleep(wait);
    client.read_more(std::time::Instant::now() + Duration::from_millis(50));
    !bar(client).contains(mark)
}

#[test]
fn a_bell_in_a_pane_out_of_focus_shows_on_its_tab_and_in_the_counter_and_looking_clears_it() {
    let daemon = TestDaemon::start();
    let mut client = start(&daemon);
    signal_from_a_pane_left_behind(&mut client, "\\a");
    assert!(mark_shown(&mut client, "[i]1"), "{}", bar(&client));
    // The tab of the window wears the mark, too.
    assert!(
        bar(&client).contains(" 1 ") && bar(&client).contains("[i] "),
        "{}",
        bar(&client)
    );
    // Going to the pane that rang is looking at it: an info notification goes away at once.
    client.send(PREFIX);
    client.send(b"h");
    assert!(
        client.wait_until(WAIT, |c| !bar(c).contains("[i]")),
        "{}",
        bar(&client)
    );
}

#[test]
fn a_bell_in_the_pane_that_is_looked_at_leaves_nothing() {
    let daemon = TestDaemon::start();
    let mut client = start(&daemon);
    client.type_line("printf '\\a'; echo rang");
    client.wait_for_row("rang").unwrap();
    assert!(
        nothing_shown_for(&mut client, "[i]", Duration::from_millis(400)),
        "{}",
        bar(&client)
    );
}

#[test]
fn the_two_notification_escape_sequences_make_one_each_and_the_others_make_none() {
    let daemon = TestDaemon::start();
    let mut client = start(&daemon);
    // A progress report and the kitty protocol are not notifications of ours.
    signal_from_a_pane_left_behind(
        &mut client,
        "\\033]9;4;1;50\\007\\033]99;i=1:d=0;Hello\\033\\\\\\\\",
    );
    assert!(
        nothing_shown_for(&mut client, "[i]", Duration::from_millis(1500)),
        "{}",
        bar(&client)
    );
    // A message in OSC 9, and a title and body in OSC 777, are.
    client.send(PREFIX);
    client.send(b"h");
    signal_from_a_pane_left_behind(&mut client, "\\033]9;build done\\007");
    assert!(mark_shown(&mut client, "[i]1"), "{}", bar(&client));
    client.send(PREFIX);
    client.send(b"h");
    assert!(client.wait_until(WAIT, |c| !bar(c).contains("[i]")));
    signal_from_a_pane_left_behind(&mut client, "\\033]777;notify;Title;Body\\007");
    assert!(mark_shown(&mut client, "[i]1"), "{}", bar(&client));
}

#[test]
fn the_counter_counts_panes_and_not_signals() {
    let daemon = TestDaemon::start();
    let mut client = start(&daemon);
    // Three bells from one pane are one pane.
    signal_from_a_pane_left_behind(&mut client, "\\a\\a\\a");
    assert!(mark_shown(&mut client, "[i]1"), "{}", bar(&client));
    // A second pane that rang makes two.
    client.send(PREFIX);
    client.send(b"l");
    signal_from_a_pane_left_behind(&mut client, "\\a");
    assert!(mark_shown(&mut client, "[i]2"), "{}", bar(&client));
}

#[test]
fn a_pane_whose_program_ends_takes_its_notification_with_it() {
    let daemon = TestDaemon::start();
    let mut client = start(&daemon);
    client.type_line("sleep 1; printf '\\a'; sleep 2; exit");
    client.send(PREFIX);
    client.send(b"|");
    assert!(client.wait_until(WAIT, |c| c.screen_text().contains('│')));
    assert!(mark_shown(&mut client, "[i]1"), "{}", bar(&client));
    // The first pane's shell ends, the pane closes, and the counter goes with it.
    assert!(
        client.wait_until(WAIT, |c| !bar(c).contains("[i]")
            && !c.screen_text().contains('│')),
        "{}\n{}",
        client.screen_text(),
        bar(&client)
    );
}

#[test]
fn a_notification_is_kept_when_the_client_detaches_and_attaches_again() {
    let daemon = TestDaemon::start();
    let mut client = start(&daemon);
    signal_from_a_pane_left_behind(&mut client, "\\a");
    assert!(mark_shown(&mut client, "[i]1"), "{}", bar(&client));
    client.detach();
    assert!(client.wait_closed(WAIT));
    let mut again = daemon
        .attach(AttachOptions::new().size(100, 30).existing(Some("work")))
        .unwrap();
    assert!(mark_shown(&mut again, "[i]1"), "{}", bar(&again));
}

// ---- Going to the pane that needs you, the list, clearing, and the marks in the tree ----

const ENTER: &[u8] = b"\r";
const DOWN: &[u8] = b"\x1b[B";
const DELETE: &[u8] = b"\x1b[3~";
const RIGHT: &[u8] = b"\x1b[C";

fn notify(daemon: &TestDaemon, pane: u64, class: &str, source: &str, text: &str) {
    let target = format!("target={pane}");
    let class = format!("class={class}");
    let source = format!("source={source}");
    let message = format!("message={text}");
    let (ok, why) = daemon
        .command(
            "notify.send",
            &["version=1", &target, &class, &source, &message],
        )
        .unwrap();
    assert!(ok, "{why}");
}

/// Three panes in a row, the focus in the last. Returns the three numbers in order.
fn three_panes(daemon: &TestDaemon) -> (TestClient, [u64; 3]) {
    let mut client = start(daemon);
    let first = client.focused_pane_id();
    client.send(PREFIX);
    client.send(b"|");
    assert!(client.wait_until(WAIT, |c| c.screen_text().contains('│')));
    let second = client.focused_pane_id();
    client.send(PREFIX);
    client.send(b"|");
    assert!(client.wait_until(WAIT, |c| c.screen_text().matches('│').count() >= 2));
    let third = client.focused_pane_id();
    (client, [first, second, third])
}

#[test]
fn next_goes_to_the_most_urgent_pane_then_the_next_and_skips_the_pane_it_is_in() {
    let daemon = TestDaemon::start();
    let (mut client, [p1, p2, p3]) = three_panes(&daemon);
    let _ = p3;
    notify(&daemon, p1, "error", "a", "old");
    notify(&daemon, p2, "approval", "b", "newer");
    assert!(mark_shown(&mut client, "[!]1"), "{}", bar(&client));
    // The key after the prefix goes to the approval; every key press after it leaves the pane it
    // is in for the other one, so four presses end in the error pane. (Looking at the pane with
    // a typed command would clear what waits there, so the focus is asked for only at the end.)
    client.send(PREFIX);
    client.send(b"a");
    std::thread::sleep(Duration::from_millis(300));
    for _ in 0..3 {
        let (ok, _) = daemon.command("notification.next", &[]).unwrap();
        assert!(ok);
    }
    assert!(client.wait_until(WAIT, |c| c.screen_text().contains('│')));
    assert_eq!(client.focused_pane_id(), p1);
}

#[test]
fn within_a_class_the_pane_that_waited_longest_comes_first() {
    let daemon = TestDaemon::start();
    let (mut client, [p1, p2, _]) = three_panes(&daemon);
    notify(&daemon, p1, "input", "a", "first");
    std::thread::sleep(Duration::from_millis(50));
    notify(&daemon, p2, "input", "b", "second");
    assert!(mark_shown(&mut client, "[?]2"), "{}", bar(&client));
    let (ok, _) = daemon.command("notification.next", &[]).unwrap();
    assert!(ok);
    assert_eq!(client.focused_pane_id(), p1);
}

#[test]
fn next_with_nothing_waiting_says_so() {
    let daemon = TestDaemon::start();
    let _client = start(&daemon);
    let (ok, why) = daemon.command("notification.next", &[]).unwrap();
    assert!(!ok);
    assert!(why.contains("no notification"), "{why}");
}

#[test]
fn the_list_shows_what_waits_and_enter_goes_to_the_chosen_pane() {
    let daemon = TestDaemon::start();
    let (mut client, [p1, p2, _]) = three_panes(&daemon);
    notify(&daemon, p1, "error", "build", "failed");
    notify(&daemon, p2, "approval", "claude", "may I");
    assert!(mark_shown(&mut client, "[!]1"), "{}", bar(&client));
    client.send(PREFIX);
    client.send(b"A");
    assert!(
        client.wait_until(WAIT, |c| c.screen_text().contains("Notifications")),
        "{}",
        client.screen_text()
    );
    let text = client.screen_text();
    assert!(text.contains("claude") && text.contains("may I"), "{text}");
    assert!(text.contains("build") && text.contains("failed"), "{text}");
    assert!(text.contains("waited"), "{text}");
    // The most urgent is first and chosen: moving down picks the error one.
    client.send(DOWN);
    client.send(ENTER);
    assert!(client.wait_until(WAIT, |c| !c.screen_text().contains("Notifications")));
    assert_eq!(client.focused_pane_id(), p1);
}

#[test]
fn delete_in_the_list_clears_the_chosen_line_and_the_list_closes_when_empty() {
    let daemon = TestDaemon::start();
    let (mut client, [p1, p2, _]) = three_panes(&daemon);
    notify(&daemon, p1, "error", "build", "failed");
    notify(&daemon, p2, "approval", "claude", "may I");
    assert!(mark_shown(&mut client, "[!]1"), "{}", bar(&client));
    client.send(PREFIX);
    client.send(b"A");
    assert!(client.wait_until(WAIT, |c| c.screen_text().contains("Notifications")));
    client.send(DELETE);
    assert!(
        client.wait_until(WAIT, |c| !c.screen_text().contains("may I")
            && c.screen_text().contains("failed")),
        "{}",
        client.screen_text()
    );
    assert!(gone(&mut client, "[!]"), "{}", bar(&client));
    client.send(DELETE);
    assert!(
        client.wait_until(WAIT, |c| !c.screen_text().contains("Notifications")),
        "{}",
        client.screen_text()
    );
    assert!(gone(&mut client, "[x]"), "{}", bar(&client));
}

fn gone(client: &mut TestClient, mark: &str) -> bool {
    client.wait_until(WAIT, |c| !bar(c).contains(mark))
}

#[test]
fn the_keys_clear_the_pane_in_focus_or_every_pane() {
    let daemon = TestDaemon::start();
    let (mut client, [p1, p2, p3]) = three_panes(&daemon);
    notify(&daemon, p3, "approval", "a", "here");
    notify(&daemon, p1, "error", "a", "there");
    assert!(mark_shown(&mut client, "[!]1"), "{}", bar(&client));
    client.send(PREFIX);
    client.send(b"q");
    assert!(gone(&mut client, "[!]"), "{}", bar(&client));
    assert!(bar(&client).contains("[x]1"), "{}", bar(&client));
    notify(&daemon, p2, "input", "a", "x");
    assert!(mark_shown(&mut client, "[?]1"), "{}", bar(&client));
    client.send(PREFIX);
    client.send(b"Q");
    assert!(
        client.wait_until(WAIT, |c| !bar(c).contains("[?]") && !bar(c).contains("[x]")),
        "{}",
        bar(&client)
    );
}

#[test]
fn the_tree_marks_panes_windows_and_sessions_and_the_marks_follow_clearing() {
    let daemon = TestDaemon::start();
    let mut other = daemon
        .attach(AttachOptions::new().size(100, 30).named("other"))
        .unwrap();
    other.wait_for_prompt().unwrap();
    let far = other.focused_pane_id();
    other.detach();
    assert!(other.wait_closed(WAIT));
    let mut client = start(&daemon);
    notify(&daemon, far, "error", "build", "boom");
    client.send(PREFIX);
    client.send(b"t");
    // The session of the pane is closed in the tree, and its line already carries the mark.
    let row_of = |c: &TestClient, word: &str| -> Option<String> {
        c.screen_rows().into_iter().find(|r| r.contains(word))
    };
    assert!(
        client.wait_until(WAIT, |c| row_of(c, "other")
            .is_some_and(|r| r.contains("[x]"))),
        "{}",
        client.screen_text()
    );
    // Opening the branch shows the pane line with its own mark.
    for _ in 0..3 {
        client.send(b"\x1b[A");
    }
    client.send(RIGHT);
    client.send(RIGHT);
    client.send(RIGHT);
    assert!(
        client.wait_until(WAIT, |c| {
            c.screen_rows().iter().filter(|r| r.contains("[x]")).count() >= 3
        }),
        "{}",
        client.screen_text()
    );
    // Clearing takes every mark away at the next drawing.
    let (ok, _) = daemon
        .command("notify.clear", &[&format!("target={far}")])
        .unwrap();
    assert!(ok);
    assert!(
        client.wait_until(WAIT, |c| !c.screen_text().contains("[x]")),
        "{}",
        client.screen_text()
    );
}
