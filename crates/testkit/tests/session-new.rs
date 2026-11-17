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

/// The session made by the last key press: the one that is not `alpha` or in `known`.
fn automatic_name(daemon: &TestDaemon, known: &[&str]) -> String {
    let names = sessions(daemon);
    names
        .into_iter()
        .find(|n| !known.contains(&n.as_str()))
        .expect("a new session")
}

fn bar_has(client: &mut TestClient, text: &str) {
    let ok = client.wait_until(WAIT, |c| c.status_row().contains(text));
    assert!(ok, "the bar lacks {text:?}: {:?}", client.status_row());
}

fn bar_lacks(client: &mut TestClient, text: &str) {
    let ok = client.wait_until(WAIT, |c| !c.status_row().contains(text));
    assert!(ok, "the bar still has {text:?}: {:?}", client.status_row());
}

fn screen_has(client: &mut TestClient, text: &str) {
    let ok = client.wait_until(WAIT, |c| c.screen_text().contains(text));
    assert!(ok, "the screen lacks {text:?}:\n{}", client.screen_text());
}

/// Prefix `C`: the new session is made and shown, and its name is asked for on the bar.
fn press_c(client: &mut TestClient) {
    client.send(PREFIX);
    client.send(b"C");
    bar_has(client, "Session name: ");
}

/// Where the first line with `name` is on the screen, so that a list's order can be checked.
fn at(client: &TestClient, name: &str) -> usize {
    client
        .screen_text()
        .find(name)
        .unwrap_or_else(|| panic!("{name} is not on the screen:\n{}", client.screen_text()))
}

#[test]
fn the_key_makes_the_session_and_shows_it_before_any_name_is_asked() {
    let daemon = TestDaemon::start();
    let mut client = start(&daemon);
    client.type_line("mkdir -p inner && cd inner && echo here-$((1+1))");
    client.wait_for_row("here-2").unwrap();
    press_c(&mut client);
    // The session exists and is on the screen while the prompt is open.
    let automatic = automatic_name(&daemon, &["alpha"]);
    assert_eq!(automatic, "inner");
    bar_has(&mut client, &automatic);
    assert_eq!(sessions(&daemon), ["alpha", automatic.as_str()]);
    // A name renames it, and the shell still starts in the directory the pane was in.
    client.send(b"fresh");
    client.send(b"\r");
    bar_has(&mut client, "fresh");
    bar_lacks(&mut client, "Session name: ");
    assert_eq!(sessions(&daemon), ["alpha", "fresh"]);
    client.wait_for_prompt().unwrap();
    client.type_line("echo dir-$(basename \"$PWD\")");
    client.wait_for_row("dir-inner").unwrap();
}

#[test]
fn an_empty_name_keeps_the_automatic_name_and_escape_keeps_the_session() {
    let daemon = TestDaemon::start();
    let mut client = start(&daemon);
    press_c(&mut client);
    let automatic = automatic_name(&daemon, &["alpha"]);
    client.send(b"\x1b");
    bar_lacks(&mut client, "Session name: ");
    assert_eq!(sessions(&daemon), ["alpha", automatic.as_str()]);
    bar_has(&mut client, &automatic);
    // Enter on an empty prompt keeps the name too.
    press_c(&mut client);
    client.send(b"\r");
    bar_lacks(&mut client, "Session name: ");
    let names = sessions(&daemon);
    assert_eq!(names.len(), 3, "{names:?}");
    assert!(names.contains(&automatic));
}

#[test]
fn a_name_in_use_keeps_the_prompt_open_and_says_so() {
    let daemon = TestDaemon::start();
    let mut client = start(&daemon);
    press_c(&mut client);
    let automatic = automatic_name(&daemon, &["alpha"]);
    client.send(b"alpha");
    client.send(b"\r");
    bar_has(&mut client, "Name in use: alpha");
    // Still asking, with what was typed; the session keeps its automatic name.
    assert_eq!(sessions(&daemon), ["alpha", automatic.as_str()]);
    // A different name then works from the same prompt.
    client.send(b"\x15");
    client.send(b"other");
    client.send(b"\r");
    bar_has(&mut client, "other");
    assert_eq!(sessions(&daemon), ["alpha", "other"]);
}

#[test]
fn a_session_may_keep_the_name_it_got_automatically() {
    let daemon = TestDaemon::start();
    let mut client = start(&daemon);
    press_c(&mut client);
    let automatic = automatic_name(&daemon, &["alpha"]);
    // Typing the session's own name is not a clash with another session.
    client.send(automatic.as_bytes());
    client.send(b"\r");
    bar_lacks(&mut client, "Session name: ");
    assert_eq!(sessions(&daemon), ["alpha", automatic.as_str()]);
}

#[test]
fn the_session_list_line_makes_the_session_on_enter_and_asks_for_its_name_there() {
    let daemon = TestDaemon::start();
    let mut client = start(&daemon);
    client.send(PREFIX);
    client.send(b"s");
    screen_has(&mut client, "+ new session");
    screen_has(&mut client, "go or new session");
    // With a filter that matches no session the line is still there. Enter on it makes the
    // session, shows it, and asks for the name on its prompt, starting from the filter.
    client.send(b"zzz");
    screen_has(&mut client, "+ new session");
    client.send(b"\r");
    bar_has(&mut client, "Session name: zzz");
    let automatic = automatic_name(&daemon, &["alpha", "zzz"]);
    assert_eq!(sessions(&daemon), ["alpha", automatic.as_str()]);
    client.send(b"\r");
    bar_has(&mut client, "zzz");
    assert_eq!(sessions(&daemon), ["alpha", "zzz"]);
}

#[test]
fn the_new_session_line_without_a_filter_opens_an_empty_prompt_on_a_new_session() {
    let daemon = TestDaemon::start();
    let mut client = start(&daemon);
    client.send(PREFIX);
    client.send(b"s");
    screen_has(&mut client, "+ new session");
    // Down from the only session to the last line.
    client.send(b"\x1b[B");
    client.send(b"\r");
    bar_has(&mut client, "Session name: ");
    assert_eq!(sessions(&daemon).len(), 2);
    client.send(b"\x1b");
    bar_lacks(&mut client, "Session name: ");
    assert_eq!(sessions(&daemon).len(), 2);
}

#[test]
fn a_new_session_comes_last_in_the_session_list_and_in_the_tree() {
    let daemon = TestDaemon::start();
    let mut client = start(&daemon);
    // Names that are not in the order they were made: "zulu" first, then "bravo".
    let (ok, output) = daemon.command("session.new", &["zulu"]).unwrap();
    assert!(ok, "{output}");
    let (ok, output) = daemon.command("session.new", &["bravo"]).unwrap();
    assert!(ok, "{output}");
    client.send(PREFIX);
    client.send(b"s");
    screen_has(&mut client, "bravo");
    assert!(at(&client, "alpha") < at(&client, "zulu"));
    assert!(at(&client, "zulu") < at(&client, "bravo"));
    client.send(b"\x1b");
    client.send(PREFIX);
    client.send(b"t");
    screen_has(&mut client, "bravo");
    assert!(at(&client, "alpha") < at(&client, "zulu"));
    assert!(at(&client, "zulu") < at(&client, "bravo"));
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
