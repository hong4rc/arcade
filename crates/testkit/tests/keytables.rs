//! The key tables at work: the direct layer, the prefix and its repeat window, the lock, the
//! prompt line, and the message line. Keys are sent the way a host terminal sends them.

use std::time::Duration;

use arcade_daemon::parse_key;
use arcade_testkit::{Screened, TestClient, TestDaemon, WAIT};

const PREFIX: &[u8] = b"\x1b[109;5u";
const CTRL_ALT_LEFT: &[u8] = b"\x1b[1;7D";
const CTRL_ALT_RIGHT: &[u8] = b"\x1b[1;7C";
const CTRL_ALT_G: &[u8] = b"\x1b[103;7u";

fn run(daemon: &TestDaemon, name: &str, args: &[&str]) {
    let (ok, output) = daemon.command(name, args).unwrap();
    assert!(ok, "{name} {args:?}: {output}");
}

/// How many panes the session `session.list` shows first has.
fn pane_count(daemon: &TestDaemon) -> usize {
    let (_, output) = daemon.command("session.list", &[]).unwrap();
    output
        .lines()
        .next()
        .and_then(|l| l.split('\t').nth(4))
        .and_then(|n| n.parse().ok())
        .unwrap_or(0)
}

fn wait_panes(client: &mut TestClient, daemon: &TestDaemon, count: usize) {
    let ok = client.wait_until(WAIT, |_| pane_count(daemon) == count);
    assert!(ok, "expected {count} panes, have {}", pane_count(daemon));
}

/// The status bar's text with a space after it: reading the screen cuts trailing blanks.
fn bar(client: &TestClient) -> String {
    format!("{} ", client.status_row())
}

fn screen_has(client: &mut TestClient, text: &str) {
    let ok = client.wait_until(WAIT, |c| c.screen_text().contains(text));
    assert!(ok, "the screen lacks {text:?}:\n{}", client.screen_text());
}

fn status_has(client: &mut TestClient, text: &str) {
    let ok = client.wait_until(WAIT, |c| bar(c).contains(text));
    assert!(
        ok,
        "the status bar lacks {text:?}: {:?}",
        client.status_row()
    );
}

fn status_lacks(client: &mut TestClient, text: &str) {
    let ok = client.wait_until(WAIT, |c| !bar(c).contains(text));
    assert!(
        ok,
        "the status bar still has {text:?}: {:?}",
        client.status_row()
    );
}

/// A pane whose program shows each byte it gets, as `cat -vt` writes it.
fn raw_pane(daemon: &TestDaemon) -> TestClient {
    let mut client = daemon.attach_shell();
    client.wait_for_prompt().unwrap();
    client.type_line("stty raw -echo; echo ready; cat -vt");
    client.wait_for_row("ready").unwrap();
    client
}

fn wait_shown(client: &mut TestClient, text: &str) {
    let ok = client.wait_until(WAIT, |c| {
        c.content_rows().iter().any(|r| r.trim_start() == text)
    });
    assert!(
        ok,
        "expected {text:?} on the screen:\n{}",
        client.screen_text()
    );
}

#[test]
fn direct_keys_move_focus_and_windows_with_one_press() {
    let daemon = TestDaemon::start();
    let mut client = daemon.attach_shell();
    client.wait_for_prompt().unwrap();
    client.type_line("echo mark-one");
    client.wait_for_row("mark-one").unwrap();
    run(&daemon, "pane.split-right", &[]);
    wait_panes(&mut client, &daemon, 2);
    let right = client.focused_pane_id();
    client.send(CTRL_ALT_LEFT);
    let left = {
        let mut id = right;
        let ok = client.wait_until(WAIT, |c| {
            id = c.focused_pane_id();
            id != right
        });
        assert!(ok);
        id
    };
    assert_ne!(left, right);
    client.send(CTRL_ALT_RIGHT);
    assert!(client.wait_until(Duration::from_secs(5), |c| c.focused_pane_id() == right));
    // Windows by number: a second window, then back to the first with Ctrl+Alt+1.
    run(&daemon, "window.new", &[]);
    assert!(client.wait_until(WAIT, |c| !c.screen_text().contains("mark-one")));
    client.send(b"\x1b[49;7u");
    screen_has(&mut client, "mark-one");
}

#[test]
fn the_direct_modifier_and_movement_keys_come_from_the_settings() {
    let daemon = TestDaemon::start_with(|c| {
        c.config.keys.direct_modifier = parse_key("ctrl+alt+x").map(|k| Some(k.mods)).unwrap();
        c.config.keys.nav = ['h', 'n', 'e', 'i'];
    });
    let mut client = daemon.attach_shell();
    client.wait_for_prompt().unwrap();
    run(&daemon, "pane.split-right", &[]);
    wait_panes(&mut client, &daemon, 2);
    let right = client.focused_pane_id();
    // Ctrl+Alt+h moves left; plain Alt+h is no longer bound, so the shell gets it.
    client.send(b"\x1b[104;7u");
    assert!(client.wait_until(Duration::from_secs(5), |c| c.focused_pane_id() != right));
    client.send(b"\x1b[105;7u");
    assert!(client.wait_until(Duration::from_secs(5), |c| c.focused_pane_id() == right));
}

#[test]
fn the_prefix_runs_prefix_commands_and_shows_on_the_status_bar() {
    let daemon = TestDaemon::start();
    let mut client = daemon.attach_shell();
    client.wait_for_prompt().unwrap();
    client.send(PREFIX);
    status_has(&mut client, "PREFIX");
    client.send(b"|");
    status_lacks(&mut client, "PREFIX");
    wait_panes(&mut client, &daemon, 2);
    client.send(PREFIX);
    client.send(b"-");
    wait_panes(&mut client, &daemon, 3);
    // A new window, then back to the first by number.
    client.send(PREFIX);
    client.send(b"c");
    assert!(client.wait_until(WAIT, |c| c.status_row().contains(" 2 ")));
    client.send(PREFIX);
    client.send(b"1");
    assert!(client.wait_until(WAIT, |_| pane_count(&daemon) == 4));
}

#[test]
fn closing_a_pane_asks_first() {
    let daemon = TestDaemon::start();
    let mut client = daemon.attach_shell();
    client.wait_for_prompt().unwrap();
    run(&daemon, "pane.split-right", &[]);
    wait_panes(&mut client, &daemon, 2);
    client.send(PREFIX);
    client.send(b"x");
    status_has(&mut client, "Close this pane? (y/n)");
    client.send(b"n");
    status_lacks(&mut client, "Close this pane");
    assert_eq!(pane_count(&daemon), 2);
    client.send(PREFIX);
    client.send(b"x");
    status_has(&mut client, "Close this pane? (y/n)");
    // A key that is neither answer changes nothing.
    client.send(b"q");
    status_has(&mut client, "Close this pane? (y/n)");
    client.send(b"y");
    wait_panes(&mut client, &daemon, 1);
}

#[test]
fn renaming_a_window_is_typed_on_the_prompt_line() {
    let daemon = TestDaemon::start();
    let mut client = daemon.attach_shell();
    client.wait_for_prompt().unwrap();
    client.send(PREFIX);
    client.send(b",");
    status_has(&mut client, "Window name: ");
    client.send(b"buildx\x7f");
    status_has(&mut client, "Window name: build");
    client.send(b"\r");
    status_has(&mut client, " build ");
    // Esc leaves the name as it was.
    client.send(PREFIX);
    client.send(b",");
    status_has(&mut client, "Window name: ");
    client.send(b"other");
    client.send(b"\x1b[27u");
    status_lacks(&mut client, "Window name");
    assert!(bar(&client).contains(" build "));
}

/// Six windows named A to F with A the current one, in the order they were made.
fn six_windows(daemon: &TestDaemon, client: &mut TestClient) {
    client.wait_for_prompt().unwrap();
    for (index, name) in ["A", "B", "C", "D", "E", "F"].into_iter().enumerate() {
        if index > 0 {
            run(daemon, "window.new", &[]);
        }
        run(daemon, "window.rename", &[name]);
    }
    run(daemon, "window.select", &["1"]);
    status_has(client, " 6 F ");
}

fn move_with_prefix(forms: &[&[u8]], gap: Duration) {
    let daemon = TestDaemon::start();
    let mut client = daemon
        .attach(
            arcade_testkit::AttachOptions::new()
                .size(100, 30)
                .named("w"),
        )
        .unwrap();
    six_windows(&daemon, &mut client);
    client.send(PREFIX);
    for form in forms {
        client.send(form);
        std::thread::sleep(gap);
    }
    let want = format!(" {} A ", 1 + forms.len());
    status_has(&mut client, &want);
}

#[test]
fn the_prefix_twice_in_its_window_moves_a_window_two_places() {
    move_with_prefix(&[b">", b">"], Duration::from_millis(100));
}

#[test]
fn holding_the_key_moves_a_window_five_places_in_either_form() {
    // As a character, the way a host sends the shifted symbol ...
    move_with_prefix(&[b">", b">", b">", b">", b">"], Duration::from_millis(50));
    // ... and as the base key with Shift.
    move_with_prefix(
        &[
            b"\x1b[46;2u",
            b"\x1b[46;2u",
            b"\x1b[46;2u",
            b"\x1b[46;2u",
            b"\x1b[46;2u",
        ],
        Duration::from_millis(50),
    );
}

#[test]
fn the_repeat_window_ends_and_typing_after_it_goes_to_the_pane() {
    let daemon = TestDaemon::start();
    let mut client = daemon.attach_shell();
    client.wait_for_prompt().unwrap();
    run(&daemon, "pane.split-right", &[]);
    wait_panes(&mut client, &daemon, 2);
    client.send(PREFIX);
    client.send(b"h");
    status_has(&mut client, "PREFIX");
    // The window ends by itself, without a key.
    status_lacks(&mut client, "PREFIX");
    // A key typed inside the window that is not in the table is typing.
    client.send(PREFIX);
    client.send(b"h");
    client.send(b"echo typed-after");
    client.send(b"\r");
    screen_has(&mut client, "typed-after");
}

#[test]
fn the_command_run_from_a_pane_gives_the_same_result_as_the_key() {
    let daemon = TestDaemon::start();
    let mut client = daemon
        .attach(
            arcade_testkit::AttachOptions::new()
                .size(100, 30)
                .named("w"),
        )
        .unwrap();
    six_windows(&daemon, &mut client);
    run(&daemon, "window.move-right", &[]);
    status_has(&mut client, " 2 A ");
}

#[test]
fn the_prefix_twice_sends_the_prefix_to_the_program_once() {
    let daemon = TestDaemon::start();
    let mut client = raw_pane(&daemon);
    client.send(PREFIX);
    client.send(PREFIX);
    client.send(b"|");
    wait_shown(&mut client, "^M|");
}

#[test]
fn a_key_that_is_in_no_table_reaches_the_program_exactly_once() {
    let daemon = TestDaemon::start();
    let mut client = raw_pane(&daemon);
    client.send(b"Q");
    client.send(b"|");
    wait_shown(&mut client, "Q|");
}

#[test]
fn an_old_style_enter_never_starts_a_prefix_of_ctrl_m() {
    let daemon = TestDaemon::start();
    let mut client = raw_pane(&daemon);
    client.send(b"\r");
    client.send(b"|");
    wait_shown(&mut client, "^M|");
    assert!(!client.status_row().contains("PREFIX"));
    // The same key as the new protocol reports it does start the prefix.
    client.send(b"\x1b[109;5u");
    status_has(&mut client, "PREFIX");
}

#[test]
fn the_lock_sends_every_key_to_the_program_but_the_unlock_key() {
    let daemon = TestDaemon::start();
    let mut client = raw_pane(&daemon);
    client.send(CTRL_ALT_G);
    status_has(&mut client, "LOCK");
    client.send(PREFIX);
    client.send(CTRL_ALT_LEFT);
    client.send(b"|");
    wait_shown(&mut client, "^M^[[1;7D|");
    assert_eq!(pane_count(&daemon), 1);
    client.send(CTRL_ALT_G);
    status_lacks(&mut client, "LOCK");
    // Unlocked, the prefix works again.
    client.send(PREFIX);
    client.send(b"|");
    wait_panes(&mut client, &daemon, 2);
}

#[test]
fn the_lock_is_refused_when_nothing_could_unlock_it() {
    let daemon = TestDaemon::start_with(|c| c.config.keys.direct_modifier = None);
    let mut client = daemon.attach_shell();
    client.wait_for_prompt().unwrap();
    client.send(PREFIX);
    client.send(b"g");
    status_has(&mut client, "no key would unlock");
    assert!(!client.status_row().contains("LOCK"));
    // The message goes away with the next key.
    client.send(b"x");
    status_lacks(&mut client, "no key would unlock");
}

#[test]
fn a_key_after_the_prefix_that_means_nothing_says_so_and_is_not_passed_on() {
    let daemon = TestDaemon::start();
    let mut client = raw_pane(&daemon);
    client.send(PREFIX);
    client.send(b"~");
    status_has(&mut client, "no key");
    client.send(b"|");
    wait_shown(&mut client, "|");
    assert!(!client.screen_text().contains('~'));
}

#[test]
fn the_lock_command_from_a_pane_toggles_the_lock_too() {
    let daemon = TestDaemon::start();
    let mut client = daemon.attach_shell();
    client.wait_for_prompt().unwrap();
    run(&daemon, "arcade.lock", &[]);
    status_has(&mut client, "LOCK");
    run(&daemon, "arcade.lock", &[]);
    status_lacks(&mut client, "LOCK");
}

#[test]
fn shifted_characters_reach_a_pane_with_the_new_protocol_as_they_were_typed() {
    let daemon = TestDaemon::start();
    let mut client = daemon.attach_shell();
    client.wait_for_prompt().unwrap();
    client.type_line("stty raw -echo; echo ready; printf '\\033[>1u'; cat -vt");
    client.wait_for_row("ready").unwrap();
    client.send(b"x");
    wait_shown(&mut client, "x");
    client.send(b":A>|~");
    wait_shown(&mut client, "x:A>|~");
}
