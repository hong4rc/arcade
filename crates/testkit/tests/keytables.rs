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
    run(&daemon, "window.rename", &["old"]);
    status_has(&mut client, " old ");
    client.send(PREFIX);
    client.send(b",");
    // The prompt starts with the name in use; three Backspaces clear it.
    status_has(&mut client, "Window name: old");
    client.send(b"\x7f\x7f\x7fbuildx\x7f");
    status_has(&mut client, "Window name: build");
    client.send(b"\r");
    status_has(&mut client, " build ");
    // Esc leaves the name as it was.
    client.send(PREFIX);
    client.send(b",");
    status_has(&mut client, "Window name: build");
    client.send(b"other");
    client.send(b"\x1b[27u");
    status_lacks(&mut client, "Window name");
    assert!(bar(&client).contains(" build "));
}

#[test]
fn the_session_rename_prompt_starts_with_the_session_name() {
    let daemon = TestDaemon::start();
    let mut client = daemon
        .attach(arcade_testkit::AttachOptions::new().named("alpha"))
        .unwrap();
    client.wait_for_prompt().unwrap();
    client.send(PREFIX);
    client.send(b"$");
    status_has(&mut client, "Session name: alpha");
    // Adding to the old name is one small edit.
    client.send(b"2\r");
    status_has(&mut client, " alpha2 ");
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
    status_has(client, " F  6 ");
}

/// Ctrl+h, Ctrl+Shift+h and Ctrl+Shift+`.` as a terminal with the keyboard protocol sends them
/// while Ctrl is still down after the prefix.
const CTRL_H: &[u8] = b"\x1b[104;5u";
const CTRL_SHIFT_H: &[u8] = b"\x1b[104;6u";
const CTRL_SHIFT_DOT: &[u8] = b"\x1b[46;6u";

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
    // Ctrl is let go (the hint on the bar goes with it) so that the tabs can be read.
    client.send(b"\x1b[27u");
    let want = format!(" A  {} ", 1 + forms.len());
    status_has(&mut client, &want);
}

/// Three panes side by side, the focus on the right one.
fn three_in_a_row(daemon: &TestDaemon) -> TestClient {
    let mut client = daemon.attach_shell();
    client.wait_for_prompt().unwrap();
    run(daemon, "pane.split-right", &[]);
    run(daemon, "pane.split-right", &[]);
    wait_panes(&mut client, daemon, 3);
    client
}

/// Which of the three panes of `three_in_a_row` has the focus, told by where the cursor is
/// (asking the pane would type into it).
fn focus_of_three(client: &TestClient) -> usize {
    match client.term().cursor_position().0 {
        0..=49 => 0,
        50..=75 => 1,
        _ => 2,
    }
}

fn focus_goes_to(client: &mut TestClient, pane: usize) {
    let ok = client.wait_until(WAIT, |c| focus_of_three(c) == pane);
    assert!(
        ok,
        "the focus is in pane {}, not {pane}:\n{}",
        focus_of_three(client),
        client.screen_text()
    );
}

#[test]
fn a_bare_key_after_the_prefix_runs_once_and_the_next_keys_are_typing() {
    let daemon = TestDaemon::start();
    let mut client = daemon.attach_shell();
    client.wait_for_prompt().unwrap();
    run(&daemon, "pane.split-right", &[]);
    wait_panes(&mut client, &daemon, 2);
    let right = client.focused_pane_id();
    client.send(PREFIX);
    client.send(b"h");
    // The prefix table closed at once: `h`, `e`, `c` and `x` are letters for the pane.
    client.send(b"echo hexc-typed\r");
    screen_has(&mut client, "hexc-typed");
    assert_ne!(client.focused_pane_id(), right);
    assert!(
        !client.status_row().contains("Close this pane"),
        "{}",
        client.status_row()
    );
    assert_eq!(pane_count(&daemon), 2);
}

#[test]
fn a_second_bare_key_is_typing_even_when_the_first_one_repeated_before() {
    // Before, `>` twice moved a window twice; now the second `>` is a character for the pane.
    move_with_prefix(&[b">"], Duration::from_millis(100));
    let daemon = TestDaemon::start();
    let mut client = raw_pane(&daemon);
    client.send(PREFIX);
    client.send(b">");
    client.send(b">");
    wait_shown(&mut client, ">");
}

#[test]
fn holding_ctrl_after_the_prefix_moves_the_focus_again_and_again_then_typing_goes_to_the_pane() {
    let daemon = TestDaemon::start();
    let mut client = three_in_a_row(&daemon);
    focus_goes_to(&mut client, 2);
    client.send(PREFIX);
    client.send(CTRL_H);
    focus_goes_to(&mut client, 1);
    client.send(CTRL_H);
    focus_goes_to(&mut client, 0);
    // At the left edge a third press has nowhere to go and the table stays open.
    client.send(CTRL_H);
    std::thread::sleep(Duration::from_millis(100));
    assert_eq!(focus_of_three(&client), 0);
    // Ctrl was let go: `x` is a letter for the pane, not the close key, and is not lost.
    client.send(b"xyz-typed\r");
    screen_has(&mut client, "xyz-typed");
    assert!(
        !client.status_row().contains("Close this pane"),
        "{}",
        client.status_row()
    );
    assert_eq!(pane_count(&daemon), 3);
    assert_eq!(focus_of_three(&client), 0);
}

#[test]
fn holding_ctrl_and_shift_resizes_again_and_again() {
    let daemon = TestDaemon::start();
    let mut client = daemon.attach_shell();
    client.wait_for_prompt().unwrap();
    run(&daemon, "pane.split-right", &[]);
    wait_panes(&mut client, &daemon, 2);
    let divider = |c: &TestClient| {
        c.content_rows()
            .first()
            .and_then(|r| r.chars().position(|ch| ch == '│'))
    };
    assert!(client.wait_until(WAIT, |c| divider(c).is_some()));
    client.send(PREFIX);
    let mut last = divider(&client);
    for _ in 0..3 {
        client.send(CTRL_SHIFT_H);
        let before = last;
        let ok = client.wait_until(WAIT, |c| divider(c) != before);
        assert!(ok, "the divider stayed at {before:?}");
        last = divider(&client);
        assert!(last < before, "{before:?} then {last:?}");
    }
}

#[test]
fn holding_goes_on_for_as_long_as_the_key_is_held() {
    // Two seconds between two presses, and the second one still counts.
    move_with_prefix(
        &[CTRL_SHIFT_DOT, CTRL_SHIFT_DOT],
        Duration::from_millis(2000),
    );
    // Five presses in a row, as the shifted symbol and as the base key with Shift.
    move_with_prefix(&[CTRL_SHIFT_DOT; 5], Duration::from_millis(30));
}

#[test]
fn a_key_that_means_nothing_while_holding_is_passed_over_and_the_table_stays_open() {
    let daemon = TestDaemon::start();
    let mut client = three_in_a_row(&daemon);
    focus_goes_to(&mut client, 2);
    client.send(PREFIX);
    client.send(CTRL_H);
    focus_goes_to(&mut client, 1);
    // Ctrl+F2 is in no table.
    client.send(b"\x1b[1;5Q");
    client.send(CTRL_H);
    focus_goes_to(&mut client, 0);
}

#[test]
fn the_hint_shows_at_once_while_holding_and_goes_when_ctrl_is_let_go() {
    let daemon = hint_daemon();
    let mut client = three_in_a_row(&daemon);
    client.send(PREFIX);
    client.send(CTRL_H);
    // No half second to wait for: the hint is there as soon as the holding starts.
    assert!(
        client.wait_until(Duration::from_millis(400), |c| hint_is_shown(c)),
        "{}",
        client.status_row()
    );
    std::thread::sleep(Duration::from_millis(700));
    assert!(hint_is_shown(&client), "{}", client.status_row());
    client.send(b"q");
    assert!(
        client.wait_until(WAIT, |c| !hint_is_shown(c)),
        "{}",
        client.status_row()
    );
}

#[test]
fn an_arrow_after_the_prefix_runs_once_and_the_next_arrow_goes_to_the_pane() {
    let daemon = TestDaemon::start();
    let mut client = raw_pane(&daemon);
    run(&daemon, "pane.split-right", &[]);
    wait_panes(&mut client, &daemon, 2);
    client.send(PREFIX);
    client.send(b"\x1b[D");
    // The raw pane is the left one: it gets the second arrow, as the key came.
    client.send(b"\x1b[D");
    screen_has(&mut client, "^[[D");
}

#[test]
fn a_prefix_without_a_modifier_runs_one_command_each_time() {
    let daemon = TestDaemon::start_with(|c| c.config.keys.prefix = parse_key("f12").unwrap());
    let mut client = three_in_a_row(&daemon);
    let start = client.focused_pane_id();
    client.send(b"\x1b[24~");
    client.send(b"h");
    client.send(b"echo f12-typed\r");
    screen_has(&mut client, "f12-typed");
    assert_ne!(client.focused_pane_id(), start);
    // A key with Ctrl after it is no holding here: it means what it means in the table.
    client.send(b"\x1b[24~");
    client.send(CTRL_H);
    status_has(&mut client, "no binding for ctrl+h");
}

#[test]
fn a_terminal_with_old_key_codes_gets_one_command_a_prefix_and_loses_no_key() {
    let daemon = TestDaemon::start();
    let mut client = three_in_a_row(&daemon);
    // The terminal says it has old key codes only.
    client.send(b"\x1b[?62;22c");
    status_has(&mut client, "cannot work here");
    let start = client.focused_pane_id();
    client.send(PREFIX);
    client.send(b"h");
    client.send(b"echo legacy-typed\r");
    screen_has(&mut client, "legacy-typed");
    assert_ne!(client.focused_pane_id(), start);
    // Ctrl and `h` after the prefix is not a way to hold on: nothing repeats, nothing is eaten.
    client.send(PREFIX);
    client.send(CTRL_H);
    client.send(b"echo legacy-again\r");
    screen_has(&mut client, "legacy-again");
}

#[test]
fn a_message_on_the_status_bar_goes_away_by_itself_after_three_seconds() {
    let daemon = TestDaemon::start();
    let mut client = daemon.attach_shell();
    client.wait_for_prompt().unwrap();
    client.send(PREFIX);
    client.send(b"b");
    assert!(client.wait_until(WAIT, |c| c.status_row().contains("no binding for b")));
    let shown = std::time::Instant::now();
    // Not before three seconds, and back with the tabs in four.
    let gone = client.wait_until(Duration::from_secs(4), |c| {
        !c.status_row().contains("no binding")
    });
    assert!(gone, "{}", client.status_row());
    assert!(
        shown.elapsed() >= Duration::from_millis(2900),
        "{:?}",
        shown.elapsed()
    );
    assert!(
        client.status_row().contains(" 1 "),
        "{}",
        client.status_row()
    );
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
    status_has(&mut client, " A  2 ");
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
fn the_lock_says_which_key_unlocks() {
    let daemon = TestDaemon::start();
    let mut client = raw_pane(&daemon);
    client.send(CTRL_ALT_G);
    status_has(&mut client, "LOCK \u{b7} ctrl+alt+g to unlock");
    client.send(CTRL_ALT_G);
    status_lacks(&mut client, "to unlock");
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
    status_has(&mut client, "no binding for ~");
    client.send(b"|");
    wait_shown(&mut client, "|");
    assert!(!client.screen_text().contains('~'));
}

#[test]
fn an_unbound_shifted_key_after_the_prefix_is_named_as_typed_and_reaches_no_pane() {
    let daemon = TestDaemon::start();
    let mut client = raw_pane(&daemon);
    // Shift and a backtick as the new protocol reports it: the message says `~`, not `shift+``.
    client.send(PREFIX);
    client.send(b"\x1b[96;2u");
    status_has(&mut client, "no binding for ~");
    // The pane got nothing: what is typed next is the first thing it shows.
    client.send(b"z");
    wait_shown(&mut client, "z");
    assert!(client.content_rows().iter().all(|r| !r.contains('~')));
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

#[test]
fn the_default_direct_layer_uses_ctrl_alt_and_plain_alt_goes_to_the_pane() {
    let daemon = TestDaemon::start();
    let mut client = daemon.attach_shell();
    client.wait_for_prompt().unwrap();
    client.type_line("echo mark-one");
    client.wait_for_row("mark-one").unwrap();
    run(&daemon, "window.new", &[]);
    assert!(client.wait_until(WAIT, |c| !c.screen_text().contains("mark-one")));
    // Ctrl+Alt+, is the previous window.
    client.send(b"\x1b[44;7u");
    screen_has(&mut client, "mark-one");
    // Plain Alt+Left is nobody's binding: the program gets it whole.
    let mut raw = raw_pane(&daemon);
    raw.send(b"\x1b[1;3D");
    wait_shown(&mut raw, "^[[1;3D");
}

#[test]
fn a_multi_line_bracketed_paste_never_opens_the_prefix() {
    let daemon = TestDaemon::start();
    let mut client = raw_pane(&daemon);
    client.send(b"\x1b[200~one\rtwo\rthree\r\x1b[201~");
    wait_shown(&mut client, "one^Mtwo^Mthree^M");
    assert!(!client.status_row().contains("PREFIX"));
}

#[test]
fn a_prefix_set_to_ctrl_b_works_with_the_old_byte() {
    let daemon = TestDaemon::start_with(|c| c.config.keys.prefix = parse_key("ctrl+b").unwrap());
    let mut client = daemon.attach_shell();
    client.wait_for_prompt().unwrap();
    client.send(b"\x02");
    status_has(&mut client, "PREFIX");
    client.send(b"|");
    wait_panes(&mut client, &daemon, 2);
}

/// A daemon with the key hint turned on (it is off unless `[keys] hint = true`).
fn hint_daemon() -> TestDaemon {
    TestDaemon::start_with(|c| c.config.keys.hint = true)
}

#[test]
fn by_default_the_prefix_and_a_held_modifier_never_change_the_status_bar() {
    let daemon = TestDaemon::start();
    let mut client = daemon.attach_shell();
    six_windows(&daemon, &mut client);
    let before = client.status_row();
    assert!(
        before.contains("A  1") && before.contains("F  6"),
        "{before}"
    );
    // The prefix, then longer than the hint's delay.
    client.send(PREFIX);
    std::thread::sleep(Duration::from_millis(900));
    client.read_more(std::time::Instant::now() + Duration::from_millis(100));
    let during = client.status_row();
    assert!(!hint_is_shown(&client), "{during}");
    assert!(
        during.contains("A  1") && during.contains("F  6"),
        "{during}"
    );
    // Holding the modifier after the prefix.
    client.send(CTRL_H);
    std::thread::sleep(Duration::from_millis(700));
    client.read_more(std::time::Instant::now() + Duration::from_millis(100));
    let held = client.status_row();
    assert!(!hint_is_shown(&client), "{held}");
    assert!(held.contains("A  1") && held.contains("F  6"), "{held}");
}

#[test]
fn hint_is_a_boolean_setting_of_keys() {
    let ok = arcade_daemon::config::parse("[keys]\nhint = true\n").unwrap();
    assert!(ok.keys.hint);
    assert!(!arcade_daemon::config::Config::default().keys.hint);
    assert!(arcade_daemon::config::parse("[keys]\nhint = \"yes\"\n").is_err());
}

fn hint_is_shown(client: &TestClient) -> bool {
    client.status_row().contains("all keys")
}

#[test]
fn the_prefix_with_no_key_after_it_puts_the_hint_on_the_status_bar_and_a_key_restores_it() {
    let daemon = hint_daemon();
    let mut client = raw_pane(&daemon);
    let panes_before = client.content_rows();
    client.send(PREFIX);
    // Not at once: the hint waits for half a second.
    std::thread::sleep(Duration::from_millis(100));
    client.read_more(std::time::Instant::now() + Duration::from_millis(50));
    assert!(!hint_is_shown(&client), "{}", client.screen_text());
    std::thread::sleep(Duration::from_millis(500));
    assert!(
        client.wait_until(WAIT, |c| hint_is_shown(c)),
        "{}",
        client.screen_text()
    );
    let line = client.status_row();
    assert!(
        line.contains("| - split") && line.contains("c new") && line.contains("1-9 win"),
        "{line}"
    );
    assert!(line.trim_end().ends_with("? all keys"), "{line}");
    // Nothing is drawn over the panes: every cell of that area is as it was.
    assert_eq!(client.content_rows(), panes_before);
    // A key brings the bar back and its action runs as it always does.
    client.send(b"c");
    assert!(
        client.wait_until(WAIT, |c| !hint_is_shown(c)),
        "{}",
        client.screen_text()
    );
    assert!(
        client.status_row().contains("2 "),
        "{}",
        client.status_row()
    );
    let (_, windows) = daemon.command("window.list", &[]).unwrap();
    assert_eq!(windows.lines().count(), 2, "{windows}");
}

#[test]
fn the_hint_fits_a_narrow_terminal_and_still_ends_with_the_key_list() {
    let daemon = hint_daemon();
    let mut client = daemon
        .attach(
            arcade_testkit::AttachOptions::new()
                .size(40, 12)
                .named("narrow"),
        )
        .unwrap();
    client.wait_for_prompt().unwrap();
    client.send(PREFIX);
    std::thread::sleep(Duration::from_millis(560));
    assert!(
        client.wait_until(WAIT, |c| hint_is_shown(c)),
        "{}",
        client.screen_text()
    );
    let line = client.status_row();
    assert!(line.chars().count() <= 40, "{line:?}");
    assert!(line.trim_end().ends_with("? all keys"), "{line}");
}

#[test]
fn rebinding_a_key_changes_the_hint() {
    let dir = std::env::temp_dir().join(format!("arc-hint-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let file = dir.join("config.toml");
    std::fs::write(
        &file,
        "[keys]\nhint = true\n[keys.prefix]\n\"b\" = \"pane.zoom\"\n\"z\" = false\n",
    )
    .unwrap();
    let path = file.clone();
    let daemon = TestDaemon::start_with(move |c| c.config_path = Some(path));
    let mut client = daemon.attach_shell();
    client.wait_for_prompt().unwrap();
    client.send(PREFIX);
    std::thread::sleep(Duration::from_millis(560));
    assert!(
        client.wait_until(WAIT, |c| hint_is_shown(c)),
        "{}",
        client.screen_text()
    );
    let line = client.status_row();
    assert!(
        line.contains("b zoom") && !line.contains("z zoom"),
        "{line}"
    );
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn a_key_that_follows_the_prefix_quickly_never_shows_the_hint() {
    let daemon = hint_daemon();
    let mut client = raw_pane(&daemon);
    client.send(PREFIX);
    std::thread::sleep(Duration::from_millis(100));
    client.send(b"c");
    // Look for longer than the delay: the hint must not come.
    let deadline = std::time::Instant::now() + Duration::from_millis(900);
    while std::time::Instant::now() < deadline {
        client.read_more(std::time::Instant::now() + Duration::from_millis(50));
        assert!(!hint_is_shown(&client), "{}", client.screen_text());
    }
    let (_, windows) = daemon.command("window.list", &[]).unwrap();
    assert_eq!(windows.lines().count(), 2, "{windows}");
}

/// After holding Ctrl, whatever is typed reaches the program: no letter is lost, whatever was
/// held before it.
#[test]
fn no_typed_key_is_lost_after_holding() {
    const HELD: &[&[u8]] = &[
        CTRL_H,
        b"\x1b[106;5u",
        b"\x1b[107;5u",
        b"\x1b[108;5u",
        CTRL_SHIFT_H,
        b"\x1b[108;6u",
        b"\x1b[115;6u",
        b"\x1b[1;5D",
        b"\x1b[1;5C",
        b"\x1b[1;5Q",
        b"\x1b[114;5u",
    ];
    let daemon = TestDaemon::start();
    let mut client = daemon.attach_shell();
    client.wait_for_prompt().unwrap();
    let mut seed: u64 = 0x2545_f491_4f6c_dd1d;
    let mut next = |n: usize| {
        seed ^= seed << 13;
        seed ^= seed >> 7;
        seed ^= seed << 17;
        (seed % n as u64) as usize
    };
    for round in 0..40 {
        client.send(PREFIX);
        for _ in 0..1 + next(5) {
            client.send(HELD[next(HELD.len())]);
        }
        // Letters that are keys of the prefix table (`h`, `x`, `c`, `z`, `s`, `t`) are in it on
        // purpose: once Ctrl is up they are typing.
        let token: String = (0..4)
            .map(|_| ["h", "x", "c", "z", "s", "t", "q", "w"][next(8)])
            .collect::<String>()
            + &round.to_string();
        client.send(token.as_bytes());
        let ok = client.wait_until(WAIT, |c| c.screen_text().contains(&token));
        assert!(
            ok,
            "round {round}: {token} did not reach the pane:\n{}",
            client.screen_text()
        );
        client.send(b"\r");
        assert_eq!(pane_count(&daemon), 1);
    }
}
