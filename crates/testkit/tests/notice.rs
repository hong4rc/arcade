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
