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
    // Every pane on the screen counts as looked at while the host terminal has the focus, so
    // the person looks away: what the pane left behind then stands until the focus is back.
    client.send(FOCUS_OUT);
    std::thread::sleep(Duration::from_millis(100));
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
    // The symbol stays on the tab here: the alternation has tests of its own.
    let daemon = TestDaemon::start_with(|c| {
        c.config.notification.status_count = arcade_daemon::config::StatusCount::All;
        c.config.state_look.alternate = false;
    });
    let mut client = start(&daemon);
    signal_from_a_pane_left_behind(&mut client, "\\a");
    assert!(mark_shown(&mut client, "●1"), "{}", bar(&client));
    // The tab of the window wears the mark, too, in place of its number.
    assert!(bar(&client).contains(" ● "), "{}", bar(&client));
    // Looking at the window again is looking at every pane it shows: an info notification goes
    // away at once, also in the pane beside the one in focus.
    client.send(FOCUS_IN);
    assert!(
        client.wait_until(WAIT, |c| !bar(c).contains("●")),
        "{}",
        bar(&client)
    );
}

#[test]
fn a_bell_in_the_pane_that_is_looked_at_leaves_nothing() {
    let daemon = TestDaemon::start_counting_all();
    let mut client = start(&daemon);
    client.type_line("printf '\\a'; echo rang");
    client.wait_for_row("rang").unwrap();
    assert!(
        nothing_shown_for(&mut client, "●", Duration::from_millis(400)),
        "{}",
        bar(&client)
    );
}

#[test]
fn the_two_notification_escape_sequences_make_one_each_and_the_others_make_none() {
    let daemon = TestDaemon::start_counting_all();
    let mut client = start(&daemon);
    // A progress report and the kitty protocol are not notifications of ours.
    signal_from_a_pane_left_behind(
        &mut client,
        "\\033]9;4;1;50\\007\\033]99;i=1:d=0;Hello\\033\\\\\\\\",
    );
    assert!(
        nothing_shown_for(&mut client, "●", Duration::from_millis(1500)),
        "{}",
        bar(&client)
    );
    // A message in OSC 9, and a title and body in OSC 777, are.
    client.send(FOCUS_IN);
    client.send(PREFIX);
    client.send(b"h");
    signal_from_a_pane_left_behind(&mut client, "\\033]9;build done\\007");
    assert!(mark_shown(&mut client, "●1"), "{}", bar(&client));
    client.send(FOCUS_IN);
    assert!(client.wait_until(WAIT, |c| !bar(c).contains("●")));
    signal_from_a_pane_left_behind(&mut client, "\\033]777;notify;Title;Body\\007");
    assert!(mark_shown(&mut client, "●1"), "{}", bar(&client));
}

#[test]
fn the_counter_counts_panes_and_not_signals() {
    let daemon = TestDaemon::start_counting_all();
    let mut client = start(&daemon);
    // Three bells from one pane are one pane.
    signal_from_a_pane_left_behind(&mut client, "\\a\\a\\a");
    assert!(mark_shown(&mut client, "●1"), "{}", bar(&client));
    // A second pane that rang makes two.
    client.send(PREFIX);
    client.send(b"l");
    signal_from_a_pane_left_behind(&mut client, "\\a");
    assert!(mark_shown(&mut client, "●2"), "{}", bar(&client));
}

#[test]
fn a_pane_whose_program_ends_takes_its_notification_with_it() {
    let daemon = TestDaemon::start_counting_all();
    let mut client = start(&daemon);
    client.type_line("sleep 1; printf '\\a'; sleep 2; exit");
    client.send(PREFIX);
    client.send(b"|");
    assert!(client.wait_until(WAIT, |c| c.screen_text().contains('│')));
    client.send(FOCUS_OUT);
    assert!(mark_shown(&mut client, "●1"), "{}", bar(&client));
    // The first pane's shell ends, the pane closes, and the counter goes with it.
    assert!(
        client.wait_until(WAIT, |c| !bar(c).contains("●")
            && !c.screen_text().contains('│')),
        "{}\n{}",
        client.screen_text(),
        bar(&client)
    );
}

#[test]
fn a_notification_is_kept_when_the_client_detaches_and_attaches_again() {
    let daemon = TestDaemon::start_counting_all();
    let mut client = start(&daemon);
    signal_from_a_pane_left_behind(&mut client, "\\a");
    assert!(mark_shown(&mut client, "●1"), "{}", bar(&client));
    // Zoomed on the pane in focus, so the pane that rang is not drawn when the client comes back.
    client.send(PREFIX);
    client.send(b"z");
    std::thread::sleep(Duration::from_millis(200));
    client.detach();
    assert!(client.wait_closed(WAIT));
    let mut again = daemon
        .attach(AttachOptions::new().size(100, 30).existing(Some("work")))
        .unwrap();
    assert!(mark_shown(&mut again, "●1"), "{}", bar(&again));
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
    client.split_right();
    let second = client.focused_pane_id();
    client.split_right();
    let third = client.focused_pane_id();
    (client, [first, second, third])
}

#[test]
fn next_goes_to_the_most_urgent_pane_then_the_next_and_skips_the_pane_it_is_in() {
    let daemon = TestDaemon::start_counting_all();
    let (mut client, [p1, p2, p3]) = three_panes(&daemon);
    let _ = p3;
    notify(&daemon, p1, "error", "a", "old");
    notify(&daemon, p2, "approval", "b", "newer");
    assert!(mark_shown(&mut client, "▲1"), "{}", bar(&client));
    // The direct key (Ctrl+Alt+a) goes to the approval; every key press after it leaves the pane it
    // is in for the other one, so four presses end in the error pane. (Looking at the pane with
    // a typed command would clear what waits there, so the focus is asked for only at the end.)
    client.send(b"\x1b[97;7u");
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
    let daemon = TestDaemon::start_counting_all();
    let (mut client, [p1, p2, _]) = three_panes(&daemon);
    notify(&daemon, p1, "input", "a", "first");
    std::thread::sleep(Duration::from_millis(50));
    notify(&daemon, p2, "input", "b", "second");
    assert!(mark_shown(&mut client, "◆2"), "{}", bar(&client));
    let (ok, _) = daemon.command("notification.next", &[]).unwrap();
    assert!(ok);
    assert_eq!(client.focused_pane_id(), p1);
}

#[test]
fn next_with_nothing_waiting_says_so() {
    let daemon = TestDaemon::start_counting_all();
    let _client = start(&daemon);
    let (ok, why) = daemon.command("notification.next", &[]).unwrap();
    assert!(!ok);
    assert!(why.contains("no notification"), "{why}");
}

#[test]
fn the_list_shows_what_waits_and_enter_goes_to_the_chosen_pane() {
    let daemon = TestDaemon::start_counting_all();
    let (mut client, [p1, p2, _]) = three_panes(&daemon);
    notify(&daemon, p1, "error", "build", "failed");
    notify(&daemon, p2, "approval", "claude", "may I");
    assert!(mark_shown(&mut client, "▲1"), "{}", bar(&client));
    client.send(PREFIX);
    client.send(b"a");
    assert!(
        client.wait_until(WAIT, |c| c.screen_text().contains("Notifications")),
        "{}",
        client.screen_text()
    );
    let text = client.screen_text();
    assert!(text.contains("claude") && text.contains("may I"), "{text}");
    assert!(text.contains("build") && text.contains("failed"), "{text}");
    assert!(text.contains("waited"), "{text}");
    // Where the pane is, as the tree path: session, window number and name, pane.
    assert!(text.contains("work \u{203a} 1 "), "{text}");
    // The most urgent is first and chosen: moving down picks the error one.
    client.send(DOWN);
    client.send(ENTER);
    assert!(client.wait_until(WAIT, |c| !c.screen_text().contains("Notifications")));
    assert_eq!(client.focused_pane_id(), p1);
}

#[test]
fn delete_in_the_list_clears_the_chosen_line_and_the_list_closes_when_empty() {
    let daemon = TestDaemon::start_counting_all();
    let (mut client, [p1, p2, _]) = three_panes(&daemon);
    notify(&daemon, p1, "error", "build", "failed");
    notify(&daemon, p2, "approval", "claude", "may I");
    assert!(mark_shown(&mut client, "▲1"), "{}", bar(&client));
    client.send(PREFIX);
    client.send(b"a");
    assert!(client.wait_until(WAIT, |c| c.screen_text().contains("Notifications")));
    client.send(DELETE);
    assert!(
        client.wait_until(WAIT, |c| !c.screen_text().contains("may I")
            && c.screen_text().contains("failed")),
        "{}",
        client.screen_text()
    );
    assert!(gone(&mut client, "▲"), "{}", bar(&client));
    client.send(DELETE);
    assert!(
        client.wait_until(WAIT, |c| !c.screen_text().contains("Notifications")),
        "{}",
        client.screen_text()
    );
    assert!(gone(&mut client, "■"), "{}", bar(&client));
}

fn gone(client: &mut TestClient, mark: &str) -> bool {
    client.wait_until(WAIT, |c| !bar(c).contains(mark))
}

#[test]
fn the_keys_clear_the_pane_in_focus_or_every_pane() {
    let daemon = TestDaemon::start_counting_all();
    let (mut client, [p1, p2, p3]) = three_panes(&daemon);
    notify(&daemon, p3, "approval", "a", "here");
    notify(&daemon, p1, "error", "a", "there");
    assert!(mark_shown(&mut client, "▲1"), "{}", bar(&client));
    client.send(PREFIX);
    client.send(b"q");
    assert!(gone(&mut client, "▲"), "{}", bar(&client));
    assert!(bar(&client).contains("■1"), "{}", bar(&client));
    notify(&daemon, p2, "input", "a", "x");
    assert!(mark_shown(&mut client, "◆1"), "{}", bar(&client));
    client.send(PREFIX);
    client.send(b"A");
    assert!(
        client.wait_until(WAIT, |c| !bar(c).contains("◆") && !bar(c).contains("■")),
        "{}",
        bar(&client)
    );
}

#[test]
fn the_tree_marks_panes_windows_and_sessions_and_the_marks_follow_clearing() {
    let daemon = TestDaemon::start_counting_all();
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
            .is_some_and(|r| r.contains("■"))),
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
            c.screen_rows().iter().filter(|r| r.contains("■")).count() >= 3
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
        client.wait_until(WAIT, |c| !c.screen_text().contains("■")),
        "{}",
        client.screen_text()
    );
}

// ---- Out to the host terminal: the bell, the notification of its own, and the focus ----

const FOCUS_IN: &[u8] = b"\x1b[I";
const FOCUS_OUT: &[u8] = b"\x1b[O";

/// Let the daemon's output reach the client's virtual terminal.
fn settle(client: &mut TestClient, wait: Duration) {
    std::thread::sleep(wait);
    client.read_more(Instant::now() + Duration::from_millis(100));
}

use std::time::Instant;

#[test]
fn a_class_that_waits_for_you_rings_and_asks_for_a_notification_when_the_pane_is_not_looked_at() {
    let daemon = TestDaemon::start_counting_all();
    let (mut client, [first, _, _]) = three_panes(&daemon);
    notify(&daemon, first, "approval", "claude", "may I");
    assert!(mark_shown(&mut client, "▲1"), "{}", bar(&client));
    settle(&mut client, Duration::from_millis(200));
    assert_eq!(client.bells, 1);
    // A notification of the host terminal's own, from a command: OSC 777, the source for the
    // title.
    assert_eq!(
        client.host_notifications,
        vec![("claude".to_string(), "may I".to_string())]
    );
    let (_, log) = daemon.command("notify.log", &[]).unwrap();
    assert!(log.contains("sent out"), "{log}");
}

#[test]
fn nothing_goes_out_for_the_pane_that_is_looked_at() {
    let daemon = TestDaemon::start_counting_all();
    let (mut client, [_, _, third]) = three_panes(&daemon);
    // The third pane has the focus and the terminal is looked at.
    notify(&daemon, third, "approval", "claude", "here");
    assert!(mark_shown(&mut client, "▲1"), "{}", bar(&client));
    settle(&mut client, Duration::from_millis(300));
    assert_eq!(client.bells, 0);
    assert!(client.host_notifications.is_empty());
    let (_, log) = daemon.command("notify.log", &[]).unwrap();
    assert!(log.contains("looked at"), "{log}");
}

#[test]
fn an_info_notification_asks_for_a_notification_but_does_not_ring() {
    let daemon = TestDaemon::start_counting_all();
    let (mut client, [first, _, _]) = three_panes(&daemon);
    client.send(FOCUS_OUT);
    settle(&mut client, Duration::from_millis(100));
    notify(&daemon, first, "info", "build", "done");
    assert!(mark_shown(&mut client, "●1"), "{}", bar(&client));
    settle(&mut client, Duration::from_millis(200));
    assert_eq!(client.bells, 0);
    assert_eq!(client.host_notifications.len(), 1);
}

#[test]
fn an_escape_sequence_notification_goes_out_as_one_new_sequence_with_the_cleaned_texts() {
    let daemon = TestDaemon::start_counting_all();
    let mut client = start(&daemon);
    signal_from_a_pane_left_behind(&mut client, "\\033]777;notify;Title;Body\\007");
    assert!(mark_shown(&mut client, "●1"), "{}", bar(&client));
    settle(&mut client, Duration::from_millis(200));
    assert_eq!(
        client.host_notifications,
        vec![("Title".to_string(), "Body".to_string())]
    );
    assert_eq!(client.bells, 0);
}

#[test]
fn a_notification_typed_in_the_pane_that_is_looked_at_sends_nothing_out() {
    let daemon = TestDaemon::start_counting_all();
    let mut client = start(&daemon);
    client.type_line("printf '\\033]777;notify;T;B\\007'; echo sent");
    client.wait_for_row("sent").unwrap();
    settle(&mut client, Duration::from_millis(300));
    assert!(client.host_notifications.is_empty());
    assert_eq!(client.bells, 0);
}

#[test]
fn when_the_terminal_is_not_looked_at_the_pane_in_focus_is_told_about_and_looking_again_clears_info()
 {
    let daemon = TestDaemon::start_counting_all();
    let (mut client, [_, _, third]) = three_panes(&daemon);
    client.send(FOCUS_OUT);
    notify(&daemon, third, "info", "build", "done");
    assert!(mark_shown(&mut client, "●1"), "{}", bar(&client));
    settle(&mut client, Duration::from_millis(200));
    assert_eq!(client.host_notifications.len(), 1);
    // The terminal gets the focus back: the pane is looked at, and info goes.
    client.send(FOCUS_IN);
    assert!(gone(&mut client, "●"), "{}", bar(&client));
}

#[test]
fn a_pane_sends_out_at_most_once_in_five_seconds() {
    let daemon = TestDaemon::start_counting_all();
    let (mut client, [first, _, _]) = three_panes(&daemon);
    notify(&daemon, first, "input", "a", "one");
    notify(&daemon, first, "input", "a", "two");
    assert!(mark_shown(&mut client, "◆1"), "{}", bar(&client));
    settle(&mut client, Duration::from_millis(300));
    assert_eq!(client.host_notifications.len(), 1);
    let (_, log) = daemon.command("notify.log", &[]).unwrap();
    assert!(log.contains("a moment ago"), "{log}");
}

#[test]
fn ten_panes_in_a_second_make_ten_marks_one_bell_at_once_and_one_more_together_later() {
    let daemon = TestDaemon::start_counting_all();
    let mut client = start(&daemon);
    let mut panes = vec![client.focused_pane_id()];
    for _ in 0..9 {
        client.send(PREFIX);
        client.send(b"c");
        client.wait_for_prompt().unwrap();
        panes.push(client.focused_pane_id());
    }
    // The terminal is somewhere else, so even the pane in focus is told about.
    client.send(FOCUS_OUT);
    for pane in &panes {
        notify(&daemon, *pane, "approval", "agent", "go");
    }
    assert!(mark_shown(&mut client, "▲10"), "{}", bar(&client));
    settle(&mut client, Duration::from_millis(300));
    assert_eq!(client.bells, 1, "one bell at once");
    assert_eq!(client.host_notifications.len(), 10);
    // The bells that waited ring once together when the interval ends.
    let end = Instant::now() + Duration::from_millis(2500);
    while Instant::now() < end {
        client.read_more(end);
    }
    assert_eq!(client.bells, 2, "and one more for all that waited");
}

#[test]
fn the_focus_reports_of_the_host_terminal_do_not_reach_the_program_in_the_pane() {
    let daemon = TestDaemon::start_counting_all();
    let mut client = start(&daemon);
    client.type_line("cat -v");
    client.send(FOCUS_OUT);
    client.send(FOCUS_IN);
    client.type_line("typed");
    assert!(client.wait_until(WAIT, |c| {
        c.screen_rows()
            .iter()
            .filter(|r| r.contains("typed"))
            .count()
            >= 2
    }));
    assert!(
        !client.screen_text().contains("^[[I"),
        "{}",
        client.screen_text()
    );
    assert!(
        !client.screen_text().contains("^[[O"),
        "{}",
        client.screen_text()
    );
}

// ---- Settings, going to the pane when the terminal gets the focus back ----

static FILES: std::sync::atomic::AtomicU32 = std::sync::atomic::AtomicU32::new(0);

fn daemon_with(settings: &str) -> TestDaemon {
    let path = std::path::PathBuf::from(format!(
        "/tmp/arc-notice-{}-{}.toml",
        std::process::id(),
        FILES.fetch_add(1, std::sync::atomic::Ordering::Relaxed)
    ));
    // The counting at the right end is the subject of these tests: count every pane.
    let settings = match settings.strip_prefix("[notification]\n") {
        Some(rest) => format!("[notification]\nstatus-count = \"all\"\n{rest}"),
        None => format!("[notification]\nstatus-count = \"all\"\n{settings}"),
    };
    std::fs::write(&path, settings).unwrap();
    TestDaemon::start_with(move |c| c.config_path = Some(path))
}

const BACK: &[u8] = b"\x1b[111;7u";

#[test]
fn when_the_terminal_gets_the_focus_back_the_client_goes_to_the_pane_that_sent_a_notification_out_and_the_return_key_goes_back()
 {
    let daemon = daemon_with("[notification]\nauto-jump = true\n");
    let (mut client, [first, _, third]) = three_panes(&daemon);
    client.send(FOCUS_OUT);
    notify(&daemon, first, "approval", "claude", "may I");
    assert!(mark_shown(&mut client, "▲1"), "{}", bar(&client));
    settle(&mut client, Duration::from_millis(200));
    assert_eq!(client.host_notifications.len(), 1);
    client.send(FOCUS_IN);
    assert_eq!(client.focused_pane_id(), first);
    // The return key (Ctrl+Alt+o) goes back to where the person was.
    client.send(BACK);
    std::thread::sleep(Duration::from_millis(200));
    assert_eq!(client.focused_pane_id(), third);
}

#[test]
fn no_jump_when_the_notification_is_gone_or_the_terminal_was_looked_at_all_the_time() {
    let daemon = daemon_with("[notification]\nauto-jump = true\n");
    let (mut client, [first, _, third]) = three_panes(&daemon);
    client.send(FOCUS_OUT);
    notify(&daemon, first, "approval", "claude", "may I");
    assert!(mark_shown(&mut client, "▲1"), "{}", bar(&client));
    let (ok, _) = daemon
        .command("notify.clear", &[&format!("target={first}")])
        .unwrap();
    assert!(ok);
    client.send(FOCUS_IN);
    assert_eq!(client.focused_pane_id(), third);
    // A notification sent out while the terminal was looked at does not make a jump later.
    notify(&daemon, first, "input", "claude", "again");
    assert!(mark_shown(&mut client, "◆1"), "{}", bar(&client));
    client.send(FOCUS_OUT);
    client.send(FOCUS_IN);
    assert_eq!(client.focused_pane_id(), third);
}

/// With the default settings the jump is off: a notification sent out from another window while
/// the host terminal had no focus does not change the window or the pane when the focus comes
/// back, and the window log has the two focus changes and no window change.
#[test]
fn coming_back_with_the_default_settings_changes_neither_window_nor_pane() {
    let daemon = daemon_with("");
    let (mut client, [first, _, _]) = three_panes(&daemon);
    let (ok, why) = daemon.command("window.new", &[]).unwrap();
    assert!(ok, "{why}");
    assert!(
        client.wait_until(WAIT, |c| bar(c).contains(" 2 ")),
        "{}",
        bar(&client)
    );
    let shown = client.focused_pane_id();
    let window_changes = |daemon: &TestDaemon| {
        let (ok, log) = daemon.command("window.log", &[]).unwrap();
        assert!(ok, "{log}");
        log.lines().filter(|l| l.contains("  window ")).count()
    };
    let before = window_changes(&daemon);
    client.send(FOCUS_OUT);
    notify(&daemon, first, "approval", "claude", "may I");
    assert!(mark_shown(&mut client, "▲"), "{}", bar(&client));
    settle(&mut client, Duration::from_millis(200));
    client.send(FOCUS_IN);
    settle(&mut client, Duration::from_millis(300));
    assert_eq!(client.focused_pane_id(), shown);
    assert_eq!(window_changes(&daemon), before);
    let (_, log) = daemon.command("window.log", &[]).unwrap();
    assert!(
        log.contains("host focus lost") && log.contains("host focus gained"),
        "{log}"
    );
}

#[test]
fn the_jump_can_be_turned_off() {
    let daemon = daemon_with("[notification]\nauto-jump = false\n");
    let (mut client, [first, _, third]) = three_panes(&daemon);
    client.send(FOCUS_OUT);
    notify(&daemon, first, "approval", "claude", "may I");
    assert!(mark_shown(&mut client, "▲1"), "{}", bar(&client));
    client.send(FOCUS_IN);
    assert_eq!(client.focused_pane_id(), third);
}

#[test]
fn a_muted_source_makes_nothing() {
    let daemon = daemon_with("[notification]\nmute-sources = [\"noisy\"]\n");
    let (mut client, [first, _, _]) = three_panes(&daemon);
    notify(&daemon, first, "approval", "noisy", "spam");
    settle(&mut client, Duration::from_millis(300));
    assert!(!bar(&client).contains("▲"), "{}", bar(&client));
    assert!(client.host_notifications.is_empty());
    let (_, log) = daemon.command("notify.log", &[]).unwrap();
    assert!(log.contains("muted"), "{log}");
    notify(&daemon, first, "approval", "other", "real");
    assert!(mark_shown(&mut client, "▲1"), "{}", bar(&client));
}

#[test]
fn a_class_can_go_without_an_indicator_without_a_bell_or_without_a_notification() {
    let daemon = daemon_with(
        "[notification.error]\nindicator = false\n[notification.input]\nbell = false\n[notification.approval]\nosc = false\n",
    );
    let (mut client, [first, second, third]) = three_panes(&daemon);
    notify(&daemon, first, "error", "a", "e");
    settle(&mut client, Duration::from_millis(300));
    assert!(!bar(&client).contains("■"), "{}", bar(&client));
    assert_eq!(
        client.host_notifications.len(),
        1,
        "no mark but it goes out"
    );
    assert_eq!(client.bells, 1);
    // Input: the notification goes out, the bell does not.
    notify(&daemon, second, "input", "a", "i");
    assert!(mark_shown(&mut client, "◆1"), "{}", bar(&client));
    settle(&mut client, Duration::from_millis(300));
    assert_eq!(client.host_notifications.len(), 2);
    assert_eq!(client.bells, 1);
    // Approval: the bell only. The third pane has the focus; leave it first.
    client.send(PREFIX);
    client.send(b"h");
    notify(&daemon, third, "approval", "a", "p");
    assert!(mark_shown(&mut client, "▲1"), "{}", bar(&client));
    // The bell of the interval waits; the notification of its own does not come.
    assert_eq!(client.host_notifications.len(), 2);
}

#[test]
fn the_terminal_looked_at_can_keep_everything_in_but_the_indicator() {
    let daemon = daemon_with("[notification]\npush-when-host-focused = false\n");
    let (mut client, [first, second, _]) = three_panes(&daemon);
    notify(&daemon, first, "approval", "a", "quiet");
    assert!(mark_shown(&mut client, "▲1"), "{}", bar(&client));
    settle(&mut client, Duration::from_millis(300));
    assert!(client.host_notifications.is_empty());
    assert_eq!(client.bells, 0);
    // With the terminal not looked at it goes out.
    client.send(FOCUS_OUT);
    notify(&daemon, second, "approval", "a", "loud");
    settle(&mut client, Duration::from_millis(300));
    assert_eq!(client.host_notifications.len(), 1);
}

#[test]
fn a_command_notification_can_go_out_as_one_message() {
    let daemon = daemon_with("[notification]\nosc = \"9\"\n");
    let (mut client, [first, _, _]) = three_panes(&daemon);
    notify(&daemon, first, "approval", "claude", "may I");
    assert!(mark_shown(&mut client, "▲1"), "{}", bar(&client));
    settle(&mut client, Duration::from_millis(300));
    assert_eq!(
        client.host_notifications,
        vec![(String::new(), "may I".to_string())]
    );
}

#[test]
fn a_class_can_be_made_to_go_away_when_looked_at() {
    let daemon = daemon_with("[notification.approval]\nclear-on-focus = true\n");
    let (mut client, [first, _, _]) = three_panes(&daemon);
    client.send(FOCUS_OUT);
    settle(&mut client, Duration::from_millis(100));
    notify(&daemon, first, "approval", "a", "x");
    assert!(mark_shown(&mut client, "▲1"), "{}", bar(&client));
    client.send(FOCUS_IN);
    assert!(gone(&mut client, "▲"), "{}", bar(&client));
}

#[test]
fn the_intervals_come_from_the_settings() {
    let daemon = daemon_with("[notification]\npush-interval = 0\nbell-interval = 0\n");
    let (mut client, [first, _, _]) = three_panes(&daemon);
    notify(&daemon, first, "input", "a", "one");
    notify(&daemon, first, "input", "a", "two");
    assert!(mark_shown(&mut client, "◆1"), "{}", bar(&client));
    settle(&mut client, Duration::from_millis(300));
    assert_eq!(client.host_notifications.len(), 2);
    assert_eq!(client.bells, 2);
}

// ---- An info notification that nobody looked at goes away by itself, when asked ----

fn counting_all_with_expiry(after: Option<Duration>) -> TestDaemon {
    TestDaemon::start_with(move |c| {
        c.config.notification.status_count = arcade_daemon::config::StatusCount::All;
        c.config.notification.expire_info_after = after;
        c.config.state_look.alternate = false;
    })
}

#[test]
fn an_unlooked_at_info_notification_goes_away_after_the_expiry_and_an_input_one_stays() {
    let daemon = counting_all_with_expiry(Some(Duration::from_secs(1)));
    let (mut client, [p1, p2, _]) = three_panes(&daemon);
    client.send(FOCUS_OUT);
    settle(&mut client, Duration::from_millis(100));
    notify(&daemon, p1, "info", "a", "done");
    notify(&daemon, p2, "input", "b", "waiting");
    assert!(mark_shown(&mut client, "●1"), "{}", bar(&client));
    assert!(
        client.wait_until(WAIT, |c| !bar(c).contains("●")),
        "the info mark stayed: {}",
        bar(&client)
    );
    assert!(bar(&client).contains("◆1"), "{}", bar(&client));
    // Nothing of the info notification is left to go to.
    let (ok, _) = daemon.command("notification.next", &[]).unwrap();
    assert!(ok);
    assert_eq!(client.focused_pane_id(), p2);
}

#[test]
fn with_the_expiry_off_an_unlooked_at_info_notification_stays() {
    let daemon = counting_all_with_expiry(None);
    let (mut client, [p1, _, _]) = three_panes(&daemon);
    client.send(FOCUS_OUT);
    settle(&mut client, Duration::from_millis(100));
    notify(&daemon, p1, "info", "a", "done");
    assert!(mark_shown(&mut client, "●1"), "{}", bar(&client));
    assert!(
        !nothing_shown_for(&mut client, "●1", Duration::from_millis(2500)),
        "{}",
        bar(&client)
    );
}

// ---- A glance at a window is a glance at every pane it shows ----

/// Two panes side by side, the focus in the second. Returns the client and both numbers.
fn two_panes(daemon: &TestDaemon) -> (TestClient, u64, u64) {
    let mut client = start(daemon);
    let first = client.focused_pane_id();
    client.split_right();
    let second = client.focused_pane_id();
    (client, first, second)
}

fn standing(daemon: &TestDaemon) -> String {
    daemon.command("notify.list", &[]).unwrap().1
}

#[test]
fn a_glance_at_the_window_clears_the_info_of_a_visible_pane_that_has_not_the_focus() {
    let daemon = counting_all_with_expiry(None);
    let (mut client, first, _) = two_panes(&daemon);
    // The host terminal is not looked at, so the signal stands.
    client.send(FOCUS_OUT);
    settle(&mut client, Duration::from_millis(200));
    notify(&daemon, first, "info", "a", "done");
    assert!(mark_shown(&mut client, "●1"), "{}", bar(&client));
    assert!(!standing(&daemon).trim().is_empty());
    // Looking at the window again: the pane beside the one in focus is looked at, too.
    client.send(FOCUS_IN);
    assert!(gone(&mut client, "●"), "{}", bar(&client));
    assert!(!standing(&daemon).contains("done"), "{}", standing(&daemon));
}

#[test]
fn a_new_info_signal_for_a_visible_pane_without_the_focus_is_blocked() {
    let daemon = counting_all_with_expiry(None);
    let (mut client, first, _) = two_panes(&daemon);
    notify(&daemon, first, "info", "a", "done");
    assert!(
        nothing_shown_for(&mut client, "●", Duration::from_millis(400)),
        "{}",
        bar(&client)
    );
    let (_, log) = daemon.command("notify.log", &[]).unwrap();
    assert!(log.contains("blocked"), "{log}");
}

#[test]
fn in_a_zoomed_window_the_hidden_pane_keeps_its_info() {
    let daemon = counting_all_with_expiry(None);
    let (mut client, first, _) = two_panes(&daemon);
    client.send(PREFIX);
    client.send(b"z");
    settle(&mut client, Duration::from_millis(200));
    notify(&daemon, first, "info", "a", "done");
    assert!(mark_shown(&mut client, "●1"), "{}", bar(&client));
    assert!(
        !nothing_shown_for(&mut client, "●1", Duration::from_millis(500)),
        "{}",
        bar(&client)
    );
    // Zoom off: the pane is drawn again, so it is looked at.
    client.send(PREFIX);
    client.send(b"z");
    assert!(gone(&mut client, "●"), "{}", bar(&client));
}

#[test]
fn an_input_notification_on_a_visible_pane_without_the_focus_stays() {
    let daemon = counting_all_with_expiry(None);
    let (mut client, first, _) = two_panes(&daemon);
    notify(&daemon, first, "input", "a", "waiting");
    assert!(mark_shown(&mut client, "◆1"), "{}", bar(&client));
    settle(&mut client, Duration::from_millis(500));
    assert!(bar(&client).contains("◆1"), "{}", bar(&client));
}

#[test]
fn with_the_host_terminal_out_of_focus_the_info_of_a_visible_pane_stays() {
    let daemon = counting_all_with_expiry(None);
    let (mut client, first, _) = two_panes(&daemon);
    client.send(FOCUS_OUT);
    settle(&mut client, Duration::from_millis(200));
    notify(&daemon, first, "info", "a", "done");
    assert!(mark_shown(&mut client, "●1"), "{}", bar(&client));
    settle(&mut client, Duration::from_millis(600));
    assert!(bar(&client).contains("●1"), "{}", bar(&client));
}

// ---- The lists and the tree show the same state symbols ----

/// The rows of the screen that contain `needle`.
fn rows_with(client: &TestClient, needle: &str) -> Vec<String> {
    client
        .screen_text()
        .lines()
        .filter(|l| l.contains(needle))
        .map(str::to_string)
        .collect()
}

#[test]
fn the_session_list_marks_a_session_that_holds_an_input_notification_and_only_that_one() {
    let daemon = counting_all_with_expiry(None);
    let (mut client, first, _) = two_panes(&daemon);
    let (ok, why) = daemon.command("session.new", &["plain"]).unwrap();
    assert!(ok, "{why}");
    notify(&daemon, first, "input", "a", "waiting");
    assert!(mark_shown(&mut client, "◆"), "{}", bar(&client));
    client.send(PREFIX);
    client.send(b"s");
    assert!(
        client.wait_until(WAIT, |c| !rows_with(c, "plain").is_empty()),
        "{}",
        client.screen_text()
    );
    let input = rows_with(&client, "work");
    let input = input
        .iter()
        .find(|l| l.contains("window"))
        .expect("work row");
    assert!(input.contains("work ◆  "), "{input}");
    let quiet = rows_with(&client, "plain");
    let quiet = quiet
        .iter()
        .find(|l| l.contains("window"))
        .expect("plain row");
    assert!(!quiet.contains('◆'), "{quiet}");
    assert!(quiet.contains("plain  1 window"), "{quiet}");
}

#[test]
fn the_session_list_names_the_windows_with_a_state_most_urgent_first() {
    let daemon = counting_all_with_expiry(None);
    let mut client = start(&daemon);
    let first = client.focused_pane_id();
    let (ok, why) = daemon.command("window.new", &[]).unwrap();
    assert!(ok, "{why}");
    assert!(
        client.wait_until(WAIT, |c| bar(c).contains(" 2 ")),
        "{}",
        bar(&client)
    );
    let second = client.focused_pane_id();
    let (ok, why) = daemon.command("window.new", &[]).unwrap();
    assert!(ok, "{why}");
    assert!(
        client.wait_until(WAIT, |c| bar(c).contains(" 3 ")),
        "{}",
        bar(&client)
    );
    notify(&daemon, first, "error", "a", "broke");
    notify(&daemon, second, "input", "a", "waiting");
    assert!(mark_shown(&mut client, "◆"), "{}", bar(&client));
    client.send(PREFIX);
    client.send(b"s");
    assert!(
        client.wait_until(WAIT, |c| !rows_with(c, "3 windows").is_empty()),
        "{}",
        client.screen_text()
    );
    let row = rows_with(&client, "3 windows");
    assert!(row[0].contains("work ◆  3 windows · 2◆ 1■"), "{}", row[0]);
    // Window 3 holds nothing, so it is not named.
    assert!(
        !row[0].contains("3◆") && !row[0].contains("3■"),
        "{}",
        row[0]
    );
}

#[test]
fn the_window_list_marks_a_window_whose_pane_has_an_error() {
    let daemon = counting_all_with_expiry(None);
    let (mut client, first, _) = two_panes(&daemon);
    notify(&daemon, first, "error", "a", "broken");
    assert!(mark_shown(&mut client, "■"), "{}", bar(&client));
    client.send(PREFIX);
    client.send(b"w");
    assert!(
        client.wait_until(WAIT, |c| rows_with(c, "#1").iter().any(|l| l.contains('■'))),
        "{}",
        client.screen_text()
    );
}

#[test]
fn the_pane_you_are_in_has_the_current_mark_in_the_tree_and_a_dot_means_only_info() {
    let daemon = counting_all_with_expiry(None);
    let (mut client, first, _) = two_panes(&daemon);
    client.send(FOCUS_OUT);
    settle(&mut client, Duration::from_millis(100));
    notify(&daemon, first, "info", "a", "done");
    assert!(mark_shown(&mut client, "●1"), "{}", bar(&client));
    client.send(PREFIX);
    client.send(b"t");
    assert!(
        client.wait_until(WAIT, |c| c.screen_text().contains("Tree")),
        "{}",
        client.screen_text()
    );
    // Two pane rows: the one in the pane that has the info carries the dot and no current mark;
    // the other is the pane the client is in and carries the current mark and no dot.
    let pane_rows: Vec<String> = client
        .screen_text()
        .lines()
        .filter(|l| l.contains(" *") && l.contains('│'))
        .map(str::to_string)
        .collect();
    let all = client.screen_text();
    assert!(all.matches('●').count() >= 1, "{all}");
    for row in pane_rows {
        assert!(
            !row.contains('●') || !row.trim_end().ends_with('*'),
            "{row}"
        );
    }
}
