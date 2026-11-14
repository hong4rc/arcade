//! The floating list, switching between sessions, and moving a window to another session.

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
    // The shell draws its next prompt a moment after the output; the screen is steady only then.
    let prompt_back = alpha.wait_until(WAIT, |c| c.screen_text().contains("only-in-alpha\n$"));
    assert!(
        prompt_back,
        "no prompt after the echo:\n{}",
        alpha.screen_text()
    );
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
    client.send(b"\x1b[B");
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
    let same = client.wait_until(WAIT, |c| cells(c) == before);
    let after = cells(&mut client);
    assert!(
        same,
        "the screen is not as it was.\nmessages: {:?}\nnow:\n{}\nbefore: {before}\nafter: {after}",
        client.messages,
        client.screen_text(),
    );
    // The keys went to the list while it was open, not to the shell.
    assert!(!client.screen_text().contains("Sessions"));
}

#[test]
fn escape_after_filtering_leaves_the_screen_as_it_was() {
    let daemon = TestDaemon::start();
    let mut client = two_sessions(&daemon);
    let before = cells(&mut client);
    client.send(PREFIX);
    client.send(b"s");
    screen_has(&mut client, "Sessions");
    client.send(b"alp");
    screen_has(&mut client, "/ alp");
    screen_lacks(&mut client, "beta");
    client.send(b"\x1b[27u");
    screen_lacks(&mut client, "Sessions");
    let same = client.wait_until(WAIT, |c| cells(c) == before);
    assert!(
        same,
        "the screen is not as it was:\n{}",
        client.screen_text()
    );
}

#[test]
fn letters_go_into_the_filter_and_the_arrows_and_ctrl_n_and_ctrl_p_move() {
    // The movement letters of the settings do not move in a list: it has a filter box.
    let daemon = TestDaemon::start_with(|c| c.config.keys.nav = ['h', 'n', 'e', 'i']);
    let mut client = two_sessions(&daemon);
    client.send(PREFIX);
    client.send(b"s");
    screen_has(&mut client, "> alpha");
    client.send(b"\x0e");
    screen_has(&mut client, "> beta");
    client.send(b"\x10");
    screen_has(&mut client, "> alpha");
    client.send(b"\x1b[B");
    screen_has(&mut client, "> beta");
    client.send(b"\x1b[A");
    screen_has(&mut client, "> alpha");
    // `n` is typed: only the sessions with an `n` in the name are left, and there is none.
    client.send(b"n");
    screen_has(&mut client, "no match");
    screen_has(&mut client, "/ n");
    client.send(b"\x1b");
    screen_lacks(&mut client, "Sessions");
}

/// Sessions `a`, `ab` and `b`; the client is attached to `a`.
fn a_ab_b(daemon: &TestDaemon) -> TestClient {
    for name in ["b", "ab"] {
        let mut other = daemon
            .attach(AttachOptions::new().size(100, 30).named(name))
            .unwrap();
        other.wait_for_prompt().unwrap();
        other.detach();
        assert!(other.wait_closed(WAIT));
    }
    let mut client = daemon
        .attach(AttachOptions::new().size(100, 30).named("a"))
        .unwrap();
    client.wait_for_prompt().unwrap();
    client
}

/// The names on the lines of the open list, in order.
fn lines_of_the_list(client: &mut TestClient) -> Vec<String> {
    client
        .content_rows()
        .into_iter()
        .filter(|r| r.contains('│'))
        .filter_map(|r| {
            let inside = r.split('│').nth(1)?.trim().to_string();
            (!inside.is_empty() && !inside.starts_with('/')).then_some(inside)
        })
        .collect()
}

fn names_in_list(client: &mut TestClient, wanted: &[&str]) {
    let ok = client.wait_until(WAIT, |c| {
        let lines = lines_of_the_list(c);
        lines.len() == wanted.len()
            && lines
                .iter()
                .zip(wanted)
                .all(|(l, w)| l.trim_start_matches("> ").starts_with(w))
    });
    assert!(ok, "the list is not {wanted:?}:\n{}", client.screen_text());
}

#[test]
fn typing_filters_the_sessions_and_backspace_brings_them_back() {
    let daemon = TestDaemon::start();
    let mut client = a_ab_b(&daemon);
    client.send(PREFIX);
    client.send(b"s");
    names_in_list(&mut client, &["a", "ab", "b"]);
    client.send(b"a");
    names_in_list(&mut client, &["a", "ab"]);
    client.send(b"b");
    names_in_list(&mut client, &["ab"]);
    client.send(b"\x7f\x7f");
    names_in_list(&mut client, &["a", "ab", "b"]);
    // The line chosen is the first of what is left; Enter goes to it.
    client.send(b"b");
    client.send(b"\r");
    screen_lacks(&mut client, "Sessions");
    let (_, output) = run(&daemon, "session.list", &[]);
    // The columns are the name, the number, whether a client is attached, and more.
    let attached: Vec<&str> = output
        .lines()
        .filter(|l| l.split('\t').nth(2) == Some("1"))
        .collect();
    assert_eq!(attached.len(), 1, "{output}");
    assert!(attached[0].starts_with("ab\t"), "{output}");
}

#[test]
fn ctrl_u_empties_the_filter() {
    let daemon = TestDaemon::start();
    let mut client = a_ab_b(&daemon);
    client.send(PREFIX);
    client.send(b"s");
    client.send(b"ab");
    names_in_list(&mut client, &["ab"]);
    client.send(b"\x15");
    names_in_list(&mut client, &["a", "ab", "b"]);
}

#[test]
fn the_keys_list_is_filtered_by_what_the_command_is_called_and_enter_runs_it() {
    let daemon = TestDaemon::start();
    let mut client = daemon.attach_shell();
    client.wait_for_prompt().unwrap();
    client.send(b"\x1b[109;5u");
    client.send(b"?");
    screen_has(&mut client, "Keys");
    client.send(b"copy.enter");
    let ok = client.wait_until(WAIT, |c| {
        let lines = lines_of_the_list(c);
        lines.len() == 1 && lines[0].contains("copy.enter")
    });
    assert!(ok, "{}", client.screen_text());
    client.send(b"\r");
    screen_lacks(&mut client, "Keys");
    screen_has(&mut client, "COPY");
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

/// A session of three windows named `one`, `two` and `three`, with the first one shown.
fn three_windows(daemon: &TestDaemon) -> TestClient {
    let mut client = daemon.attach_shell();
    client.wait_for_prompt().unwrap();
    run(daemon, "window.rename", &["one"]);
    for name in ["two", "three"] {
        run(daemon, "window.new", &[]);
        run(daemon, "window.rename", &[name]);
    }
    run(daemon, "window.select", &["1"]);
    client
}

#[test]
fn the_window_list_shows_the_windows_and_choosing_one_by_filter_goes_to_it() {
    let daemon = TestDaemon::start();
    let mut client = three_windows(&daemon);
    client.send(PREFIX);
    client.send(b"w");
    names_in_list(&mut client, &["one", "two", "three"]);
    screen_has(&mut client, "> one");
    client.send(b"thr");
    names_in_list(&mut client, &["three"]);
    client.send(b"\r");
    screen_lacks(&mut client, "Windows");
    let (_, output) = run(&daemon, "window.list", &[]);
    assert_eq!(output, "1\tone\t0\n2\ttwo\t0\n3\tthree\t1", "{output}");
}

#[test]
fn the_window_list_starts_on_the_current_window_and_escape_goes_nowhere() {
    let daemon = TestDaemon::start();
    let mut client = three_windows(&daemon);
    run(&daemon, "window.select", &["2"]);
    client.send(PREFIX);
    client.send(b"w");
    screen_has(&mut client, "> two");
    client.send(b"\x1b[27u");
    screen_lacks(&mut client, "Windows");
    let (_, output) = run(&daemon, "window.list", &[]);
    assert!(output.contains("2\ttwo\t1"), "{output}");
}

/// `:` as the new keyboard protocol reports it: Shift and `;`.
const COLON: &[u8] = b"\x1b[59;2u";

#[test]
fn the_command_palette_shows_the_keys_of_a_command_and_runs_it_on_enter() {
    let daemon = TestDaemon::start();
    let mut client = daemon.attach_shell();
    client.wait_for_prompt().unwrap();
    run(&daemon, "pane.split-right", &[]);
    client.send(PREFIX);
    client.send(COLON);
    screen_has(&mut client, "Commands");
    client.send(b"zoom");
    let ok = client.wait_until(WAIT, |c| {
        let lines = lines_of_the_list(c);
        lines.len() == 1 && lines[0].contains("pane.zoom") && lines[0].contains("ctrl+m z")
    });
    assert!(ok, "{}", client.screen_text());
    assert!(!client.screen_text().contains("[Z]"));
    client.send(b"\r");
    screen_lacks(&mut client, "Commands");
    screen_has(&mut client, "[Z]");
}

#[test]
fn a_command_with_no_key_that_needs_a_number_asks_on_the_prompt_line_and_runs() {
    let daemon = TestDaemon::start();
    let mut client = three_windows(&daemon);
    client.send(PREFIX);
    client.send(COLON);
    client.send(b"window.select");
    names_in_list(&mut client, &["window.select"]);
    client.send(b"\r");
    screen_has(&mut client, "Window number:");
    client.send(b"3\r");
    let ok = client.wait_until(WAIT, |_| {
        let (_, output) = run(&daemon, "window.list", &[]);
        output.contains("3\tthree\t1")
    });
    assert!(ok);
}

#[test]
fn the_palette_asks_for_a_session_name_too() {
    let daemon = TestDaemon::start();
    let mut client = a_ab_b(&daemon);
    client.send(PREFIX);
    client.send(COLON);
    client.send(b"session.switch");
    client.send(b"\r");
    screen_has(&mut client, "Switch to session:");
    client.send(b"b\r");
    let ok = client.wait_until(WAIT, |_| {
        let (_, output) = run(&daemon, "session.list", &[]);
        output
            .lines()
            .any(|l| l.starts_with("b\t") && l.split('\t').nth(2) == Some("1"))
    });
    assert!(ok);
}

#[test]
fn the_palette_leaves_out_the_commands_of_the_lists_and_lists_the_rest_in_order() {
    let daemon = TestDaemon::start();
    let (_, output) = run(&daemon, "command.list", &[]);
    let names: Vec<&str> = output
        .lines()
        .map(|l| l.split('\t').next().unwrap())
        .collect();
    assert!(names.contains(&"pane.zoom") && names.contains(&"window.select"));
    assert!(!names.iter().any(|n| n.starts_with("list.")));
    let mut sorted = names.clone();
    sorted.sort_unstable();
    assert_eq!(names, sorted);
    assert!(output.contains("pane.zoom\tctrl+m z"), "{output}");
}
