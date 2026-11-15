//! Copy mode and the clipboard, through the daemon: moving over the scrollback, selecting,
//! copying (as an OSC 52 sequence for the client, or through a command), and pasting.
//!
//! No test touches the real clipboard: OSC 52 is read from what the test client was sent, and
//! the command mode writes to a file in the temporary directory.

use std::path::PathBuf;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::time::Duration;

use arcade_daemon::config::ClipboardMode;
use arcade_testkit::{AttachOptions, Screened, TestClient, TestDaemon, WAIT, wait_for_file};

const PREFIX: &[u8] = b"\x1b[109;5u";
const ESC: &[u8] = b"\x1b[27u";

static COUNTER: AtomicUsize = AtomicUsize::new(0);

fn scratch() -> PathBuf {
    let dir = std::env::temp_dir().join(format!(
        "arc-copy-{}-{}",
        std::process::id(),
        COUNTER.fetch_add(1, Ordering::Relaxed)
    ));
    std::fs::create_dir_all(&dir).unwrap();
    dir
}

fn client(daemon: &TestDaemon) -> TestClient {
    let mut client = daemon
        .attach(AttachOptions::new().size(100, 30).named("copy"))
        .unwrap();
    client.wait_for_prompt().unwrap();
    client
}

fn bar(client: &TestClient) -> String {
    format!("{} ", client.status_row())
}

/// The word on the bar that says the pane is in copy mode: `COPY`, or `VISUAL` and `V-LINE`
/// while a selection is active, or `JUMP` while a jump is open.
fn copy_word(bar: &str) -> Option<&'static str> {
    ["COPY", "VISUAL", "V-LINE", "JUMP"]
        .into_iter()
        .find(|w| bar.contains(&format!(" {w} ")) || bar.trim_end().ends_with(&format!(" {w}")))
}

fn in_copy_mode(client: &mut TestClient) -> bool {
    client.wait_until(WAIT, |c| copy_word(&bar(c)).is_some())
}

fn left_copy_mode(client: &mut TestClient) -> bool {
    client.wait_until(WAIT, |c| copy_word(&bar(c)).is_none())
}

fn enter(client: &mut TestClient) {
    client.send(PREFIX);
    client.send(b"[");
    assert!(in_copy_mode(client), "{}", client.screen_text());
}

fn search(client: &mut TestClient, forward: bool, text: &str) {
    client.send(if forward { b"/" } else { b"?" });
    client.send(text.as_bytes());
    client.send(b"\r");
}

fn message_has(client: &mut TestClient, text: &str) {
    let ok = client.wait_until(WAIT, |c| bar(c).contains(text));
    assert!(ok, "the bar lacks {text:?}: {:?}", bar(client));
}

/// The screen row (counting from the top of the terminal) whose text is exactly `text`.
fn row_of(client: &TestClient, text: &str) -> Option<usize> {
    client.screen_rows().iter().position(|r| r == text)
}

fn wait_clipboard(client: &mut TestClient, count: usize) {
    let ok = client.wait_until(WAIT, |c| c.clipboard_writes.len() >= count);
    assert!(
        ok,
        "the client was sent {} clipboard writes, not {count}",
        client.clipboard_writes.len()
    );
}

#[test]
fn three_lines_far_up_the_scrollback_reach_the_client_as_one_osc52_sequence() {
    let daemon = TestDaemon::start();
    let mut client = client(&daemon);
    client.type_line("seq 1 500");
    client.wait_for_row("500").unwrap();
    enter(&mut client);
    // Far above the screen: the screen shows 29 rows, line 250 is about 250 rows up.
    search(&mut client, true, "250");
    client.wait_for_row("250").unwrap();
    client.send(b"V");
    client.send(b"jj");
    client.send(b"y");
    wait_clipboard(&mut client, 1);
    assert_eq!(client.clipboard_writes, vec![b"250\n251\n252".to_vec()]);
    // Copying ended copy mode, and the bar says how much was copied.
    assert!(left_copy_mode(&mut client));
    message_has(&mut client, "copied 11 characters");
    // And there was one sequence, not two.
    std::thread::sleep(Duration::from_millis(100));
    client.read_more(std::time::Instant::now() + Duration::from_millis(50));
    assert_eq!(client.clipboard_writes.len(), 1);
}

#[test]
fn the_command_mode_feeds_the_command_and_the_client_gets_no_sequence() {
    let dir = scratch();
    let file = dir.join("clipboard.txt");
    let target = file.clone();
    let daemon = TestDaemon::start_with(move |c| {
        c.config.clipboard.mode = ClipboardMode::Command;
        c.config.clipboard.command = Some(format!("cat > '{}'", target.display()));
    });
    let mut client = client(&daemon);
    client.type_line("seq 1 100");
    client.wait_for_row("100").unwrap();
    enter(&mut client);
    search(&mut client, true, "50");
    client.wait_for_row("50").unwrap();
    client.send(b"Vy");
    // The message stays for three seconds only, so it is looked at first.
    message_has(&mut client, "copied 2 characters");
    let start = std::time::Instant::now();
    while std::fs::read_to_string(&file).map_or(true, |t| t != "50") {
        assert!(
            start.elapsed() < WAIT,
            "{:?}",
            std::fs::read_to_string(&file)
        );
        std::thread::sleep(Duration::from_millis(10));
    }
    assert!(client.clipboard_writes.is_empty());
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn the_mode_off_only_keeps_the_buffer() {
    let daemon = TestDaemon::start_with(|c| c.config.clipboard.mode = ClipboardMode::Off);
    let mut client = client(&daemon);
    client.type_line("echo keep-this");
    client.wait_for_row("keep-this").unwrap();
    enter(&mut client);
    search(&mut client, false, "keep-this");
    client.wait_for_row("keep-this").unwrap();
    client.send(b"Vy");
    message_has(&mut client, "kept 9 characters");
    assert!(client.clipboard_writes.is_empty());
}

#[test]
fn a_very_large_copy_comes_with_a_warning() {
    let daemon = TestDaemon::start();
    let mut client = client(&daemon);
    let line = "a".repeat(90);
    client.type_line(&format!("yes {line} | head -1000"));
    client.wait_until(WAIT, |c| {
        c.screen_rows().iter().filter(|r| **r == line).count() > 20
    });
    client.wait_for_prompt().unwrap();
    enter(&mut client);
    client.send(b"gVGy");
    wait_clipboard(&mut client, 1);
    assert!(
        client.clipboard_writes[0].len() > 90_000,
        "{}",
        client.clipboard_writes[0].len()
    );
    message_has(&mut client, "may have ignored");
}

#[test]
fn the_screen_stands_still_in_copy_mode_while_the_program_goes_on_printing() {
    let daemon = TestDaemon::start();
    let mut client = client(&daemon);
    client.type_line("(sleep 1; echo LATE-$((40+2))) &");
    client.wait_for_prompt().unwrap();
    enter(&mut client);
    std::thread::sleep(Duration::from_millis(1500));
    client.read_more(std::time::Instant::now() + Duration::from_millis(50));
    assert!(!client.screen_text().contains("LATE-42"));
    // A key that is not a copy mode key does not reach the shell.
    client.send(b"x");
    std::thread::sleep(Duration::from_millis(100));
    client.send(b"q");
    assert!(left_copy_mode(&mut client));
    // Now the newest output is on the screen, and nothing was typed at the prompt.
    client.wait_for_text("LATE-42").unwrap();
    assert!(
        !client.screen_text().lines().any(|r| r.starts_with("$ x")),
        "{}",
        client.screen_text()
    );
}

#[test]
fn another_pane_goes_on_updating_while_this_one_is_in_copy_mode() {
    let daemon = TestDaemon::start();
    let mut client = client(&daemon);
    client.send(PREFIX);
    client.send(b"|");
    client.wait_until(WAIT, |c| c.screen_text().contains('│'));
    client.type_line("sleep 1; echo OTHER-PANE-LINE");
    client.send(PREFIX);
    client.send(b"h");
    enter(&mut client);
    client.wait_for_text("OTHER-PANE-LINE").unwrap();
    assert!(bar(&client).contains(" COPY "));
}

#[test]
fn escape_drops_the_selection_first_and_leaves_the_second_time() {
    let daemon = TestDaemon::start();
    let mut client = client(&daemon);
    client.type_line("echo select-me");
    client.wait_for_row("select-me").unwrap();
    enter(&mut client);
    // The cursor starts on the prompt under the output; one up is the output's row.
    client.send(b"k0");
    let y = row_of(&client, "select-me").unwrap() as u16;
    let plain = client.style_at(y, 0);
    client.send(b"v");
    client.send(b"llll");
    // The selected cells look different from the others.
    let selected = client.wait_until(WAIT, |c| c.style_at(y, 3) != plain);
    assert!(selected);
    assert_eq!(client.style_at(y, 5), plain);
    client.send(ESC);
    assert!(client.wait_until(WAIT, |c| c.style_at(y, 3) == plain));
    assert!(bar(&client).contains(" COPY "));
    client.send(ESC);
    assert!(left_copy_mode(&mut client));
}

#[test]
fn the_capital_y_copies_and_stays_in_copy_mode() {
    let daemon = TestDaemon::start();
    let mut client = client(&daemon);
    client.type_line("echo stay-here");
    client.wait_for_row("stay-here").unwrap();
    enter(&mut client);
    search(&mut client, false, "stay-here");
    client.wait_for_row("stay-here").unwrap();
    client.send(b"VY");
    wait_clipboard(&mut client, 1);
    assert_eq!(client.clipboard_writes[0], b"stay-here".to_vec());
    // A message takes the place of the bar until the next key; that key is a copy mode key, and
    // the bar then shows copy mode again.
    message_has(&mut client, "copied 9 characters");
    client.send(b"k");
    assert!(in_copy_mode(&mut client), "{}", bar(&client));
}

#[test]
fn enter_does_not_copy_and_the_selection_stays() {
    let daemon = TestDaemon::start();
    let mut client = client(&daemon);
    client.type_line("echo no-enter-copy");
    client.wait_for_row("no-enter-copy").unwrap();
    enter(&mut client);
    search(&mut client, false, "no-enter-copy");
    client.wait_for_row("no-enter-copy").unwrap();
    client.send(b"V\r");
    std::thread::sleep(Duration::from_millis(300));
    client.read_more(std::time::Instant::now() + Duration::from_millis(100));
    assert!(client.clipboard_writes.is_empty());
    assert!(in_copy_mode(&mut client), "{}", bar(&client));
    // The selection is still there: the copy key copies it.
    client.send(b"y");
    wait_clipboard(&mut client, 1);
    assert_eq!(client.clipboard_writes[0], b"no-enter-copy".to_vec());
}

#[test]
fn lines_after_characters_take_the_rows_from_the_anchor_to_the_cursor() {
    let daemon = TestDaemon::start();
    let mut client = client(&daemon);
    client.type_line("printf 'alpha one\\nbeta two\\ngamma three\\ndelta four\\n'");
    client.wait_for_row("delta four").unwrap();
    enter(&mut client);
    search(&mut client, false, "delta four");
    client.wait_for_row("delta four").unwrap();
    // Characters from the middle of the last row up to the middle of the first, then lines.
    client.send(b"wvkkk");
    client.send(b"V");
    client.send(b"y");
    wait_clipboard(&mut client, 1);
    assert_eq!(
        client.clipboard_writes[0],
        b"alpha one\nbeta two\ngamma three\ndelta four".to_vec()
    );
}

#[test]
fn the_paste_key_writes_what_was_copied_into_the_pane() {
    let daemon = TestDaemon::start();
    let mut client = client(&daemon);
    client.type_line("echo hello-paste");
    client.wait_for_row("hello-paste").unwrap();
    enter(&mut client);
    search(&mut client, false, "hello-paste");
    client.wait_for_row("hello-paste").unwrap();
    client.send(b"Vy");
    wait_clipboard(&mut client, 1);
    assert!(left_copy_mode(&mut client));
    client.send(PREFIX);
    client.send(b"]");
    // The shell has not seen Enter, so the text stands at its prompt.
    let ok = client.wait_until(WAIT, |c| {
        c.screen_rows().iter().any(|r| r == "$ hello-paste")
    });
    assert!(ok, "{}", client.screen_text());
}

#[test]
fn a_program_that_asked_for_bracketed_paste_gets_the_markers_and_others_do_not() {
    let daemon = TestDaemon::start();
    let mut client = client(&daemon);
    client.type_line("echo marked-text");
    client.wait_for_row("marked-text").unwrap();
    enter(&mut client);
    search(&mut client, false, "marked-text");
    client.wait_for_row("marked-text").unwrap();
    client.send(b"Vy");
    assert!(left_copy_mode(&mut client));
    client.type_line(r"printf '\033[?2004h'; stty raw -echo; cat -v");
    client.wait_for_text("cat -v").unwrap();
    std::thread::sleep(Duration::from_millis(200));
    client.send(PREFIX);
    client.send(b"]");
    client.wait_for_text("^[[200~marked-text^[[201~").unwrap();
}

#[test]
fn pasting_with_nothing_copied_says_so() {
    let daemon = TestDaemon::start();
    let mut client = client(&daemon);
    client.send(PREFIX);
    client.send(b"]");
    message_has(&mut client, "nothing has been copied");
}

#[test]
fn a_program_writing_the_clipboard_is_heard_by_default_and_not_when_the_settings_forbid_it() {
    let ask = r"printf '\033]52;c;%s\a' $(printf from-pane | base64)";
    // On by default: the client is sent exactly one sequence.
    let daemon = TestDaemon::start();
    let mut client = client(&daemon);
    client.type_line(ask);
    wait_clipboard(&mut client, 1);
    std::thread::sleep(Duration::from_millis(200));
    client.read_more(std::time::Instant::now() + Duration::from_millis(50));
    assert_eq!(client.clipboard_writes, vec![b"from-pane".to_vec()]);
    // Forbidden by `allow-pane-write = false`: nothing is sent, and the buffer stays empty.
    let daemon = TestDaemon::start_with(|c| c.config.clipboard.allow_pane_write = false);
    let mut client = self::client(&daemon);
    client.type_line(ask);
    client.wait_for_prompt().unwrap();
    std::thread::sleep(Duration::from_millis(200));
    client.read_more(std::time::Instant::now() + Duration::from_millis(50));
    assert!(client.clipboard_writes.is_empty());
    client.send(PREFIX);
    client.send(b"]");
    message_has(&mut client, "nothing has been copied");
}

#[test]
fn a_thousand_clipboard_writes_in_one_burst_reach_the_client_as_few_and_the_last_is_the_last() {
    let daemon = TestDaemon::start();
    let mut client = client(&daemon);
    let dir = std::env::temp_dir().join(format!("arc-osc52-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let file = dir.join("burst");
    // 999 writes of "A", then one of "B", all in a file that `cat` puts out at once.
    client.type_line(&format!(
        r#"{{ yes "$(printf '\033]52;c;QQ==\007')" | head -n 999 | tr -d '\n'; printf '\033]52;c;Qg==\007'; }} > {f} && cat {f}"#,
        f = file.display()
    ));
    let ok = client.wait_until(WAIT, |c| c.clipboard_writes.last() == Some(&b"B".to_vec()));
    assert!(ok, "{:?}", client.clipboard_writes.len());
    std::thread::sleep(Duration::from_millis(200));
    client.read_more(std::time::Instant::now() + Duration::from_millis(50));
    assert!(
        client.clipboard_writes.len() <= 10,
        "{} sequences reached the client",
        client.clipboard_writes.len()
    );
    assert_eq!(client.clipboard_writes.last(), Some(&b"B".to_vec()));
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn a_clipboard_write_from_a_pane_that_never_stops_printing_still_reaches_the_client() {
    let daemon = TestDaemon::start();
    let mut client = client(&daemon);
    // "X" in base64 is "WA==".
    client.type_line(r"printf '\033]52;c;WA==\007'; yes");
    let ok = client.wait_until(Duration::from_secs(1), |c| {
        c.clipboard_writes.last() == Some(&b"X".to_vec())
    });
    assert!(ok, "{:?}", client.clipboard_writes);
}

#[test]
fn the_commands_work_from_inside_a_pane_too() {
    let daemon = TestDaemon::start();
    let mut client = client(&daemon);
    let pane = client.focused_pane_id();
    let (ok, _) = daemon.command_from(Some(pane), "copy.enter", &[]).unwrap();
    assert!(ok);
    assert!(in_copy_mode(&mut client));
    let (ok, _) = daemon.command_from(Some(pane), "copy.up", &[]).unwrap();
    assert!(ok);
    let (ok, _) = daemon.command_from(Some(pane), "copy.exit", &[]).unwrap();
    assert!(ok);
    assert!(left_copy_mode(&mut client));
    // Outside copy mode the movement commands are refused.
    let (ok, output) = daemon.command_from(Some(pane), "copy.up", &[]).unwrap();
    assert!(!ok);
    assert!(output.contains("not in copy mode"), "{output}");
}

#[test]
fn resizing_the_terminal_leaves_copy_mode() {
    let daemon = TestDaemon::start();
    let mut client = client(&daemon);
    enter(&mut client);
    client.resize(90, 25);
    assert!(left_copy_mode(&mut client));
}

#[test]
fn the_authors_movement_keys_and_a_rebound_key_work_in_copy_mode() {
    let dir = scratch();
    let file = dir.join("config.toml");
    std::fs::write(
        &file,
        "[keys]\nnav = \"hnei\"\n\n[keys.copy]\n\"x\" = \"copy.exit\"\n",
    )
    .unwrap();
    let path = file.clone();
    let daemon = TestDaemon::start_with(move |c| c.config_path = Some(path));
    let mut client = client(&daemon);
    client.type_line("printf 'a\\nfoo\\nb\\nfoo\\nc\\n'");
    client.wait_for_row("c").unwrap();
    enter(&mut client);
    let rows = client.screen_rows();
    let foos: Vec<usize> = rows
        .iter()
        .enumerate()
        .filter(|(_, r)| *r == "foo")
        .map(|(i, _)| i)
        .collect();
    assert_eq!(foos.len(), 2, "{}", client.screen_text());
    let cursor_row = |c: &TestClient| usize::from(c.screen.cursor_position().1);
    // Search upwards: the nearer `foo`; `k` goes on in the same direction; `K` goes back.
    search(&mut client, false, "foo");
    assert!(client.wait_until(WAIT, |c| cursor_row(c) == foos[1]));
    client.send(b"k");
    assert!(client.wait_until(WAIT, |c| cursor_row(c) == foos[0]));
    client.send(b"K");
    assert!(client.wait_until(WAIT, |c| cursor_row(c) == foos[1]));
    // `n` is down and `e` is up.
    client.send(b"n");
    assert!(client.wait_until(WAIT, |c| cursor_row(c) == foos[1] + 1));
    client.send(b"e");
    assert!(client.wait_until(WAIT, |c| cursor_row(c) == foos[1]));
    // The old keys for down and up do nothing but what they were given: `j` is the end of a word.
    // The key bound in the file leaves copy mode.
    client.send(b"x");
    assert!(left_copy_mode(&mut client));
    let _ = std::fs::remove_dir_all(&dir);
}

/// A pane with a scrollback of 200 lines that has printed 1 to 300, and a background job that
/// prints `more` further lines (numbered from 5001) a moment from now, while copy mode is open,
/// and then writes the returned file. The test waits for that file instead of sleeping.
///
/// The job is started from a subshell: an interactive shell prints `[1] <process number>` for a
/// job it starts itself, and the search for "250" finds that line (the nearest match going up)
/// whenever the process number happens to contain 250. A subshell prints nothing.
fn full_scrollback_with_late_output(daemon: &TestDaemon, more: usize) -> (TestClient, PathBuf) {
    let dir = scratch();
    let marker = dir.join("printed");
    let mut client = client(daemon);
    client.type_line(&format!(
        "seq 1 300; ( (sleep 1; seq 5001 {}; touch '{}') & )",
        5000 + more,
        marker.display()
    ));
    client.wait_for_row("300").unwrap();
    assert!(
        client.screen_rows().iter().all(|r| !r.starts_with("[1]")),
        "the shell printed a job line, which a search for a number can find:\n{}",
        client.screen_text()
    );
    enter(&mut client);
    search(&mut client, false, "250");
    client.wait_for_row("250").unwrap();
    (client, marker)
}

/// Wait for the late output to be printed, and a moment for the daemon to take it in.
fn late_output_taken_in(client: &mut TestClient, marker: &std::path::Path) {
    assert!(
        wait_for_file(marker, WAIT),
        "the late output was never printed"
    );
    client.read_more(std::time::Instant::now() + Duration::from_millis(300));
}

#[test]
fn the_selection_stays_on_its_line_when_old_lines_are_cut_from_a_full_scrollback() {
    let daemon = TestDaemon::start_with(|c| c.scrollback_lines = 200);
    let (mut client, marker) = full_scrollback_with_late_output(&daemon, 100);
    client.send(b"V");
    // Lines are cut off the top of the scrollback while the selection is held.
    late_output_taken_in(&mut client, &marker);
    assert!(row_of(&client, "250").is_some(), "{}", client.screen_text());
    client.send(b"y");
    wait_clipboard(&mut client, 1);
    assert_eq!(client.clipboard_writes, vec![b"250".to_vec()]);
}

#[test]
fn a_selected_line_that_was_cut_off_is_dropped_with_a_message() {
    let daemon = TestDaemon::start_with(|c| c.scrollback_lines = 200);
    let (mut client, marker) = full_scrollback_with_late_output(&daemon, 1000);
    client.send(b"V");
    late_output_taken_in(&mut client, &marker);
    client.send(b"y");
    // Nothing is copied: the line is gone, and the bar says so.
    message_has(&mut client, "no longer in the scrollback");
    std::thread::sleep(Duration::from_millis(300));
    client.read_more(std::time::Instant::now() + Duration::from_millis(100));
    assert!(
        client.clipboard_writes.is_empty(),
        "{:?}",
        client.clipboard_writes
    );
}

/// Identical lines printed while copy mode is open: when old lines are cut off the top, the
/// cursor stays on its line, a fixed distance under the one line that is different.
#[test]
fn the_cursor_stays_on_its_line_when_identical_lines_are_cut_from_a_full_scrollback() {
    let daemon = TestDaemon::start_with(|c| c.scrollback_lines = 200);
    let dir = scratch();
    let marker = dir.join("printed");
    // A wide terminal makes the library cut off few lines at a time.
    let mut client = daemon
        .attach(AttachOptions::new().size(400, 30).named("copy"))
        .unwrap();
    client.wait_for_prompt().unwrap();
    client.type_line(&format!(
        "{{ yes same | head -n 600; echo MARK; yes same | head -n 12; }}; (sleep 1; yes same | head -n 200; touch '{}') &",
        marker.display()
    ));
    client.wait_until(WAIT, |c| row_of(c, "MARK").is_some());
    enter(&mut client);
    // Three lines under the line that is different.
    search(&mut client, false, "MARK");
    client.wait_until(WAIT, |c| row_of(c, "MARK").is_some());
    for _ in 0..3 {
        client.send(b"j");
    }
    let cursor_row = |c: &TestClient| usize::from(c.screen.cursor_position().1);
    assert!(client.wait_until(WAIT, |c| {
        row_of(c, "MARK").is_some_and(|m| cursor_row(c) == m + 3)
    }));
    late_output_taken_in(&mut client, &marker);
    // The next key is the first that looks again.
    client.send(b"j");
    assert!(
        client.wait_until(WAIT, |c| {
            row_of(c, "MARK").is_some_and(|m| cursor_row(c) == m + 4)
        }),
        "mark at {:?}, cursor at {}\n{}",
        row_of(&client, "MARK"),
        cursor_row(&client),
        client.screen_text()
    );
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn the_bar_tells_copy_visual_v_line_and_jump_apart() {
    let daemon = TestDaemon::start();
    let mut client = client(&daemon);
    client.type_line("echo state-check");
    client.wait_for_row("state-check").unwrap();
    enter(&mut client);
    let word = |client: &mut TestClient, want: &str| {
        let ok = client.wait_until(WAIT, |c| copy_word(&bar(c)) == Some(want));
        assert!(ok, "wanted {want}: {}", bar(client));
    };
    word(&mut client, "COPY");
    client.send(b"v");
    word(&mut client, "VISUAL");
    client.send(b"V");
    word(&mut client, "V-LINE");
    client.send(b"V");
    word(&mut client, "COPY");
    client.send(b"s");
    word(&mut client, "JUMP");
    client.send(b"st");
    assert!(client.wait_until(WAIT, |c| bar(c).contains("JUMP st")));
    client.send(ESC);
    word(&mut client, "COPY");
}

#[test]
fn a_jump_label_moves_the_cursor_and_a_selection_follows_it_to_the_copy() {
    let daemon = TestDaemon::start();
    let mut client = client(&daemon);
    client.type_line("printf 'first mark\\nsecond line\\nthird mark here\\n'");
    client.wait_for_row("third mark here").unwrap();
    enter(&mut client);
    // Select from the cursor's row, then jump to `third`: the jump extends the selection.
    search(&mut client, false, "first mark");
    client.wait_for_row("first mark").unwrap();
    client.send(b"V");
    client.send(b"sthird");
    // The label stands right after the match, in its own colour; read it off the screen.
    let label = {
        let mut found = None;
        let ok = client.wait_until(WAIT, |c| {
            let row = c.screen_rows().iter().position(|r| r.starts_with("third"));
            if let Some(y) = row {
                let text = c.screen_rows()[y].clone();
                let at = text.find("third").unwrap() + "third".len();
                found = text[at..].chars().next();
            }
            found.is_some_and(|ch| ch.is_ascii_lowercase() && ch != ' ')
        });
        assert!(ok, "{}", client.screen_text());
        found.unwrap()
    };
    client.send(label.to_string().as_bytes());
    assert!(client.wait_until(WAIT, |c| copy_word(&bar(c)) == Some("V-LINE")));
    client.send(b"y");
    wait_clipboard(&mut client, 1);
    assert_eq!(
        client.clipboard_writes[0],
        b"first mark\nsecond line\nthird mark here".to_vec()
    );
}
