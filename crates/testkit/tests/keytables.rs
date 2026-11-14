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
fn a_key_that_does_not_repeat_is_typing_inside_the_repeat_window() {
    let daemon = TestDaemon::start();
    let mut client = daemon.attach_shell();
    client.wait_for_prompt().unwrap();
    run(&daemon, "pane.split-right", &[]);
    wait_panes(&mut client, &daemon, 2);
    client.send(PREFIX);
    client.send(b"h");
    // `c` is a prefix key (new window) that does not repeat: inside the window it is a letter
    // for the pane, not a command.
    client.send(b"cd /; echo typed-c-e");
    client.send(b"\r");
    // The whole line reached the pane, with its first `c`, and no window was made.
    let ok = client.wait_until(WAIT, |c| {
        c.content_rows()
            .iter()
            .any(|r| r.trim_start().starts_with("typed-c-e"))
    });
    assert!(ok, "{}", client.screen_text());
    assert!(
        !client.status_row().contains(" 2 "),
        "a window was made: {}",
        client.status_row()
    );
    // A key that repeats still works inside the window.
    client.send(PREFIX);
    client.send(b"h");
    client.send(b"l");
    assert!(client.wait_until(WAIT, |c| c.status_row().contains("PREFIX")));
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

fn the_keys_list_is_open(client: &TestClient) -> bool {
    client.screen_text().contains("Keys")
}

fn step_down_until(client: &mut TestClient, text: &str) {
    for _ in 0..200 {
        if client.screen_text().contains(text) {
            return;
        }
        client.send(b"\x1b[B");
        client.read_more(std::time::Instant::now() + Duration::from_millis(20));
    }
    panic!("{text:?} is not in the list:\n{}", client.screen_text());
}

#[test]
fn the_question_mark_after_the_prefix_lists_the_keys_with_their_commands() {
    let daemon = TestDaemon::start();
    let mut client = raw_pane(&daemon);
    client.send(PREFIX);
    // `?` as the new protocol reports it: Shift and `/`.
    client.send(b"\x1b[47;2u");
    assert!(client.wait_until(WAIT, |c| the_keys_list_is_open(c)));
    for wanted in [
        "ctrl+m | ",
        "pane.split-right",
        "ctrl+m - ",
        "pane.split-down",
        "ctrl+m c ",
        "window.new",
        "ctrl+m d ",
        "session.detach",
    ] {
        step_down_until(&mut client, wanted);
    }
    // Escape closes it and nothing reached the pane.
    client.send(b"\x1b");
    assert!(client.wait_until(WAIT, |c| !the_keys_list_is_open(c)));
}

#[test]
fn the_command_names_in_the_keys_list_start_in_one_column_whatever_the_key_name_length() {
    let daemon = TestDaemon::start();
    let mut client = raw_pane(&daemon);
    client.send(PREFIX);
    client.send(b"\x1b[47;2u");
    assert!(client.wait_until(WAIT, |c| the_keys_list_is_open(c)));
    let column_of = |client: &mut TestClient, command: &str, key: &str| -> usize {
        step_down_until(client, key);
        let row = client
            .content_rows()
            .into_iter()
            .find(|r| r.contains(key))
            .unwrap();
        assert!(row.contains(command), "{row:?} lacks {command:?}");
        row.find(command).unwrap()
    };
    // A short key name and a long one.
    let short = column_of(&mut client, "window.new", "ctrl+m c ");
    let long = column_of(&mut client, "pane.focus-left", "ctrl+alt+left");
    assert_eq!(short, long, "the command names do not line up");
}

#[test]
fn enter_in_the_keys_list_runs_the_chosen_command() {
    let daemon = TestDaemon::start();
    let mut client = daemon.attach_shell();
    client.wait_for_prompt().unwrap();
    client.send(PREFIX);
    client.send(b"?");
    assert!(client.wait_until(WAIT, |c| the_keys_list_is_open(c)));
    // The lines go by command name: `arcade.lock`, `arcade.redraw`, `arcade.reload-config`,
    // `command.list`, then `copy.enter`.
    client.send(b"\x1b[B\x1b[B\x1b[B\x1b[B");
    client.read_more(std::time::Instant::now() + Duration::from_millis(100));
    client.send(b"\r");
    assert!(client.wait_until(WAIT, |c| bar(c).contains(" COPY ")));
    assert!(!the_keys_list_is_open(&client));
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

fn hint_is_shown(client: &TestClient) -> bool {
    client.screen_text().contains("After ctrl+m")
}

#[test]
fn the_prefix_with_no_key_after_it_shows_the_hint_and_a_key_closes_it_and_runs() {
    let daemon = TestDaemon::start();
    let mut client = raw_pane(&daemon);
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
    let text = client.screen_text();
    assert!(
        text.contains("pane.split-right") && text.contains("window.new"),
        "{text}"
    );
    let row_of = |needle: &str| {
        text.lines()
            .find(|l| l.contains(needle))
            .unwrap()
            .to_string()
    };
    assert!(row_of("pane.split-right").contains("| "), "{text}");
    assert!(row_of("window.new").contains("c "), "{text}");
    // The nine numbered window keys are one line, and the whole box is on the screen.
    assert!(row_of("window.select").contains("1-9"), "{text}");
    assert_eq!(text.matches("window.select").count(), 1, "{text}");
    assert!(text.contains('└'), "{text}");
    // A key closes it and its action runs as it always does.
    client.send(b"c");
    assert!(
        client.wait_until(WAIT, |c| !hint_is_shown(c)),
        "{}",
        client.screen_text()
    );
    let (_, windows) = daemon.command("window.list", &[]).unwrap();
    assert_eq!(windows.lines().count(), 2, "{windows}");
}

#[test]
fn a_key_that_follows_the_prefix_quickly_never_shows_the_hint() {
    let daemon = TestDaemon::start();
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

#[test]
fn the_hint_does_not_show_in_the_repeat_window() {
    let daemon = TestDaemon::start();
    let mut client = raw_pane(&daemon);
    run(&daemon, "pane.split-right", &[]);
    // `H` is repeatable: after it the table stays open, and no hint comes of that.
    client.send(PREFIX);
    client.send(b"H");
    std::thread::sleep(Duration::from_millis(300));
    let deadline = std::time::Instant::now() + Duration::from_millis(500);
    while std::time::Instant::now() < deadline {
        client.read_more(std::time::Instant::now() + Duration::from_millis(50));
        assert!(!hint_is_shown(&client), "{}", client.screen_text());
    }
}
