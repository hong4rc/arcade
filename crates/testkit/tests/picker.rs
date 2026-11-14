//! The floating list, switching between sessions, and moving a window to another session.

use std::time::Duration;

use arcade_proto::{DetachReason, ServerMessage};
use arcade_testkit::{AttachOptions, Screened, TestClient, TestDaemon, WAIT};

const PREFIX: &[u8] = b"\x1b[109;5u";

fn run(daemon: &TestDaemon, name: &str, args: &[&str]) -> (bool, String) {
    daemon.command(name, args).unwrap()
}

/// Two sessions, `alpha` and `beta`, each with a line of its own on the screen. The returned
/// client is attached to `alpha`; `beta` has no client.
fn two_sessions(daemon: &TestDaemon) -> TestClient {
    let mut beta = daemon
        .attach(AttachOptions::new().size(100, 30).named("beta"))
        .unwrap();
    beta.wait_for_prompt().unwrap();
    beta.type_line("echo only-in-beta");
    beta.wait_for_row("only-in-beta").unwrap();
    beta.detach();
    assert!(beta.wait_closed(WAIT));
    let mut alpha = daemon
        .attach(AttachOptions::new().size(100, 30).named("alpha"))
        .unwrap();
    alpha.wait_for_prompt().unwrap();
    alpha.type_line("echo only-in-alpha");
    alpha.wait_for_row("only-in-alpha").unwrap();
    alpha
}

fn screen_has(client: &mut TestClient, text: &str) {
    let ok = client.wait_until(WAIT, |c| c.screen_text().contains(text));
    assert!(ok, "the screen lacks {text:?}:\n{}", client.screen_text());
}

fn screen_lacks(client: &mut TestClient, text: &str) {
    let ok = client.wait_until(WAIT, |c| !c.screen_text().contains(text));
    assert!(
        ok,
        "the screen still has {text:?}:\n{}",
        client.screen_text()
    );
}

/// Every cell of the screen, for comparing two moments.
fn cells(client: &mut TestClient) -> String {
    format!("{:?}", client.screen.screen().unwrap())
}

#[test]
fn the_session_list_shows_every_session_and_choosing_one_switches_to_it() {
    let daemon = TestDaemon::start();
    let mut client = two_sessions(&daemon);
    client.send(PREFIX);
    client.send(b"s");
    screen_has(&mut client, "Sessions");
    screen_has(&mut client, "> alpha");
    screen_has(&mut client, "beta");
    assert!(
        client.screen_text().contains("alpha  1 window *"),
        "{}",
        client.screen_text()
    );
    client.send(b"j");
    screen_has(&mut client, "> beta");
    client.send(b"\r");
    // The screen is the other session's, the bar says its name, and the client stayed.
    screen_has(&mut client, "only-in-beta");
    screen_lacks(&mut client, "Sessions");
    screen_lacks(&mut client, "only-in-alpha");
    assert!(client.wait_until(WAIT, |c| c.status_row().contains(" beta ")));
    assert!(!client.closed);
    assert!(
        !client.messages.iter().any(|m| matches!(
            m,
            ServerMessage::Detached { .. } | ServerMessage::SessionEnded { .. }
        )),
        "{:?}",
        client.messages
    );
    // Typing goes to the session now shown.
    client.type_line("echo typed-after-switch");
    screen_has(&mut client, "typed-after-switch");
}

#[test]
fn escape_closes_the_list_and_the_screen_is_as_it_was() {
    let daemon = TestDaemon::start();
    let mut client = two_sessions(&daemon);
    let before = cells(&mut client);
    client.send(PREFIX);
    client.send(b"s");
    screen_has(&mut client, "Sessions");
    assert_ne!(cells(&mut client), before);
    client.send(b"\x1b[27u");
    screen_lacks(&mut client, "Sessions");
    assert!(client.wait_until(WAIT, |c| cells(c) == before));
    // The keys went to the list while it was open, not to the shell.
    assert!(!client.screen_text().contains("Sessions"));
}

#[test]
fn the_list_follows_the_movement_keys_of_the_settings() {
    let daemon = TestDaemon::start_with(|c| c.config.keys.nav = ['h', 'n', 'e', 'i']);
    let mut client = two_sessions(&daemon);
    client.send(PREFIX);
    client.send(b"s");
    screen_has(&mut client, "> alpha");
    client.send(b"n");
    screen_has(&mut client, "> beta");
    client.send(b"e");
    screen_has(&mut client, "> alpha");
    // The old movement key does nothing now.
    client.send(b"j");
    std::thread::sleep(Duration::from_millis(50));
    assert!(client.screen_text().contains("> alpha"));
    client.send(b"q");
    screen_lacks(&mut client, "Sessions");
}

#[test]
fn switching_to_a_session_that_has_a_client_detaches_that_client() {
    let daemon = TestDaemon::start();
    let mut other = daemon
        .attach(AttachOptions::new().size(100, 30).named("beta"))
        .unwrap();
    other.wait_for_prompt().unwrap();
    let mut client = daemon
        .attach(AttachOptions::new().size(100, 30).named("alpha"))
        .unwrap();
    client.wait_for_prompt().unwrap();
    assert!(run(&daemon, "session.switch", &["beta"]).0);
    let detached = other.wait_message(
        |m| {
            matches!(
                m,
                ServerMessage::Detached {
                    reason: DetachReason::Replaced,
                    ..
                }
            )
        },
        WAIT,
    );
    assert!(detached.is_ok(), "{:?}", other.messages);
    assert!(client.wait_until(WAIT, |c| c.status_row().contains(" beta ")));
    assert!(!client.closed);
}

#[test]
fn the_command_from_a_pane_switches_like_the_key() {
    let daemon = TestDaemon::start();
    let mut client = two_sessions(&daemon);
    let pane = client.focused_pane_id();
    let (ok, message) = daemon
        .command_from(Some(pane), "session.switch", &["beta"])
        .unwrap();
    assert!(ok, "{message}");
    screen_has(&mut client, "only-in-beta");
    assert!(client.wait_until(WAIT, |c| c.status_row().contains(" beta ")));
    // A name that does not exist is an error and changes nothing.
    let pane = client.focused_pane_id();
    let (ok, message) = daemon
        .command_from(Some(pane), "session.switch", &["nowhere"])
        .unwrap();
    assert!(!ok);
    assert!(message.contains("no such session"), "{message}");
}

#[test]
fn a_window_moves_to_another_session_and_its_program_keeps_running() {
    let daemon = TestDaemon::start();
    let mut client = two_sessions(&daemon);
    run(&daemon, "window.new", &[]);
    assert!(client.wait_until(WAIT, |c| c.status_row().contains(" 2 ")));
    client.type_line("echo before-move; sleep 1; echo after-move");
    screen_has(&mut client, "before-move");
    let (ok, message) = run(&daemon, "window.move-to-session", &["beta"]);
    assert!(ok, "{message}");
    // The client stays in `alpha`, which has one window again.
    assert!(client.wait_until(WAIT, |c| !c.status_row().contains(" 2 ")));
    screen_has(&mut client, "only-in-alpha");
    let (_, list) = run(&daemon, "session.list", &[]);
    let counts: Vec<(String, String)> = list
        .lines()
        .map(|l| {
            let f: Vec<&str> = l.split('\t').collect();
            (f[0].to_string(), f[3].to_string())
        })
        .collect();
    assert!(
        counts.contains(&("alpha".to_string(), "1".to_string())),
        "{list}"
    );
    assert!(
        counts.contains(&("beta".to_string(), "2".to_string())),
        "{list}"
    );
    // In `beta` the moved window shows what the program went on to print.
    assert!(run(&daemon, "session.switch", &["beta"]).0);
    assert!(client.wait_until(WAIT, |c| c.status_row().contains(" 2 ")));
    assert!(run(&daemon, "window.select", &["2"]).0);
    screen_has(&mut client, "after-move");
}

#[test]
fn the_last_window_of_a_session_cannot_be_moved() {
    let daemon = TestDaemon::start();
    let mut client = two_sessions(&daemon);
    let (ok, message) = run(&daemon, "window.move-to-session", &["beta"]);
    assert!(!ok);
    assert!(message.contains("last window"), "{message}");
    // From the keys, the same refusal shows on the status bar and no list opens.
    client.send(PREFIX);
    client.send(b"m");
    assert!(client.wait_until(WAIT, |c| c.status_row().contains("last window")));
    assert!(!client.screen_text().contains("Move window to"));
}

#[test]
fn the_move_list_leaves_out_the_current_session_and_moves_the_window_on_enter() {
    let daemon = TestDaemon::start();
    let mut client = two_sessions(&daemon);
    run(&daemon, "window.new", &[]);
    assert!(client.wait_until(WAIT, |c| c.status_row().contains(" 2 ")));
    client.send(PREFIX);
    client.send(b"m");
    screen_has(&mut client, "Move window to");
    assert!(
        !client.screen_text().contains("alpha  2 windows"),
        "{}",
        client.screen_text()
    );
    screen_has(&mut client, "> beta");
    client.send(b"\r");
    assert!(client.wait_until(WAIT, |c| !c.status_row().contains(" 2 ")));
    let (_, list) = run(&daemon, "session.list", &[]);
    assert!(
        list.lines()
            .any(|l| l.starts_with("beta\t") && l.split('\t').nth(3) == Some("2")),
        "{list}"
    );
}
