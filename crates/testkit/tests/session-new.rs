//! Making a session from the keyboard (prefix `C`), from the session list (the last line) and
//! from a script (`arcade run session.new`).

use arcade_testkit::{AttachOptions, Screened, TestClient, TestDaemon, WAIT};

const PREFIX: &[u8] = b"\x1b[109;5u";

fn start(daemon: &TestDaemon) -> TestClient {
    let mut client = daemon
        .attach(AttachOptions::new().size(120, 30).named("alpha"))
        .unwrap();
    client.wait_for_prompt().unwrap();
    client
}

fn sessions(daemon: &TestDaemon) -> Vec<String> {
    let (ok, output) = daemon.command("session.list", &[]).unwrap();
    assert!(ok, "{output}");
    let mut names: Vec<String> = output
        .lines()
        .filter_map(|l| l.split_whitespace().next())
        .map(|w| w.trim_start_matches('*').to_string())
        .filter(|w| !w.is_empty())
        .collect();
    names.sort();
    names
}

fn bar_has(client: &mut TestClient, text: &str) {
    let ok = client.wait_until(WAIT, |c| c.status_row().contains(text));
    assert!(ok, "the bar lacks {text:?}: {:?}", client.status_row());
}

fn screen_has(client: &mut TestClient, text: &str) {
    let ok = client.wait_until(WAIT, |c| c.screen_text().contains(text));
    assert!(ok, "the screen lacks {text:?}:\n{}", client.screen_text());
}

fn open_prompt(client: &mut TestClient) {
    client.send(PREFIX);
    client.send(b"C");
    bar_has(client, "Session name: ");
}

#[test]
fn the_key_asks_for_a_name_makes_the_session_in_the_directory_of_the_pane_and_shows_it() {
    let daemon = TestDaemon::start();
    let mut client = start(&daemon);
    client.type_line("mkdir -p inner && cd inner");
    client.type_line("echo here-$((1+1))");
    client.wait_for_row("here-2").unwrap();
    open_prompt(&mut client);
    // Nothing is made before the name is entered.
    assert_eq!(sessions(&daemon), ["alpha"]);
    client.send(b"fresh");
    client.send(b"\r");
    bar_has(&mut client, "fresh");
    assert_eq!(sessions(&daemon), ["alpha", "fresh"]);
    // The screen is the new session's, and its shell starts in the directory the pane was in.
    client.wait_for_prompt().unwrap();
    client.type_line("echo dir-$(basename \"$PWD\")");
    client.wait_for_row("dir-inner").unwrap();
}

#[test]
fn an_empty_name_gives_the_automatic_name_and_escape_makes_nothing() {
    let daemon = TestDaemon::start();
    let mut client = start(&daemon);
    open_prompt(&mut client);
    client.send(b"\x1b");
    assert!(client.wait_until(WAIT, |c| !c.status_row().contains("Session name: ")));
    assert_eq!(sessions(&daemon), ["alpha"]);
    open_prompt(&mut client);
    client.send(b"\r");
    assert!(client.wait_until(WAIT, |c| !c.status_row().contains("Session name: ")));
    let names = sessions(&daemon);
    assert_eq!(names.len(), 2, "{names:?}");
    let automatic = names.iter().find(|n| *n != "alpha").unwrap();
    assert!(client.wait_until(WAIT, |c| c.status_row().contains(automatic.as_str())));
}

#[test]
fn a_name_in_use_keeps_the_prompt_open_and_says_so() {
    let daemon = TestDaemon::start();
    let mut client = start(&daemon);
    open_prompt(&mut client);
    client.send(b"alpha");
    client.send(b"\r");
    bar_has(&mut client, "Name in use: alpha");
    // Still asking, with what was typed, and nothing was made.
    assert_eq!(sessions(&daemon), ["alpha"]);
    // A different name then works from the same prompt.
    client.send(b"\x15");
    client.send(b"other");
    client.send(b"\r");
    bar_has(&mut client, "other");
    assert_eq!(sessions(&daemon), ["alpha", "other"]);
}

#[test]
fn the_session_list_ends_with_a_new_session_line_that_asks_for_the_name_first() {
    let daemon = TestDaemon::start();
    let mut client = start(&daemon);
    client.send(PREFIX);
    client.send(b"s");
    screen_has(&mut client, "+ new session");
    screen_has(&mut client, "go or new session");
    // With a filter that matches no session the line is still there, and a first Enter on it
    // opens the prompt with the filter as the name; nothing is made yet.
    client.send(b"zzz");
    screen_has(&mut client, "+ new session");
    client.send(b"\r");
    bar_has(&mut client, "Session name: zzz");
    assert_eq!(sessions(&daemon), ["alpha"]);
    client.send(b"\r");
    bar_has(&mut client, "zzz");
    assert_eq!(sessions(&daemon), ["alpha", "zzz"]);
}

#[test]
fn the_new_session_line_without_a_filter_opens_an_empty_prompt() {
    let daemon = TestDaemon::start();
    let mut client = start(&daemon);
    client.send(PREFIX);
    client.send(b"s");
    screen_has(&mut client, "+ new session");
    // Down from the only session to the last line.
    client.send(b"\x1b[B");
    client.send(b"\r");
    bar_has(&mut client, "Session name: ");
    assert!(
        !client.status_row().contains("Session name: a"),
        "{:?}",
        client.status_row()
    );
    assert_eq!(sessions(&daemon), ["alpha"]);
    client.send(b"\x1b");
    assert_eq!(sessions(&daemon), ["alpha"]);
}

#[test]
fn run_makes_a_session_and_shows_it_only_when_it_comes_from_a_pane() {
    let daemon = TestDaemon::start();
    let mut client = start(&daemon);
    // From a script with no pane: made, and the screen stays where it is.
    let (ok, output) = daemon.command("session.new", &["scripted"]).unwrap();
    assert!(ok, "{output}");
    assert_eq!(sessions(&daemon), ["alpha", "scripted"]);
    client.wait_until(std::time::Duration::from_millis(500), |_| false);
    assert!(
        !client.status_row().contains("scripted"),
        "{:?}",
        client.status_row()
    );
    // The same name again is refused.
    let (ok, output) = daemon.command("session.new", &["scripted"]).unwrap();
    assert!(!ok && output.contains("exists"), "{output}");
    // From a pane: the screen of the pane goes to the new session.
    let pane = client.focused_pane_id();
    let (ok, output) = daemon
        .command_from(Some(pane), "session.new", &["from-pane"])
        .unwrap();
    assert!(ok, "{output}");
    bar_has(&mut client, "from-pane");
    assert_eq!(sessions(&daemon), ["alpha", "from-pane", "scripted"]);
}
