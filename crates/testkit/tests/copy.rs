//! Copy mode and the clipboard, through the daemon: moving over the scrollback, selecting,
//! copying (as an OSC 52 sequence for the client, or through a command), and pasting.
//!
//! No test touches the real clipboard: OSC 52 is read from what the test client was sent, and
//! the command mode writes to a file in the temporary directory.

use std::path::PathBuf;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::time::Duration;

use arcade_daemon::config::ClipboardMode;
use arcade_testkit::{AttachOptions, Screened, TestClient, TestDaemon, WAIT};

const CTRL_A: &[u8] = b"\x01";
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

fn in_copy_mode(client: &mut TestClient) -> bool {
    client.wait_until(WAIT, |c| bar(c).contains(" COPY "))
}

fn left_copy_mode(client: &mut TestClient) -> bool {
    client.wait_until(WAIT, |c| !bar(c).contains(" COPY "))
}

fn enter(client: &mut TestClient) {
    client.send(CTRL_A);
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
    // Copying ended copy mode, and the bar says what was sent and does not say "copied".
    assert!(left_copy_mode(&mut client));
    message_has(&mut client, "sent 11 characters");
    assert!(!bar(&client).contains("copied"));
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
    let ok = client.wait_until(WAIT, |_| {
        std::fs::read_to_string(&file).is_ok_and(|t| t == "50")
    });
    assert!(ok, "{:?}", std::fs::read_to_string(&file));
    assert!(client.clipboard_writes.is_empty());
    message_has(&mut client, "sent 2 characters");
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
    client.send(CTRL_A);
    client.send(b"|");
    client.wait_until(WAIT, |c| c.screen_text().contains('│'));
    client.type_line("sleep 1; echo OTHER-PANE-LINE");
    client.send(CTRL_A);
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
    message_has(&mut client, "sent 9 characters");
    client.send(b"k");
    assert!(in_copy_mode(&mut client), "{}", bar(&client));
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
    client.send(CTRL_A);
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
    client.send(CTRL_A);
    client.send(b"]");
    client.wait_for_text("^[[200~marked-text^[[201~").unwrap();
}

#[test]
fn pasting_with_nothing_copied_says_so() {
    let daemon = TestDaemon::start();
    let mut client = client(&daemon);
    client.send(CTRL_A);
    client.send(b"]");
    message_has(&mut client, "nothing has been copied");
}

#[test]
fn a_program_writing_the_clipboard_is_heard_only_when_the_settings_allow_it() {
    let ask = r"printf '\033]52;c;%s\a' $(printf from-pane | base64)";
    // Off by default: nothing is sent, and the buffer stays empty.
    let daemon = TestDaemon::start();
    let mut client = client(&daemon);
    client.type_line(ask);
    client.wait_for_prompt().unwrap();
    std::thread::sleep(Duration::from_millis(200));
    client.read_more(std::time::Instant::now() + Duration::from_millis(50));
    assert!(client.clipboard_writes.is_empty());
    client.send(CTRL_A);
    client.send(b"]");
    message_has(&mut client, "nothing has been copied");
    // Allowed: it is sent the way a copy is.
    let daemon = TestDaemon::start_with(|c| c.config.clipboard.allow_pane_write = true);
    let mut client = self::client(&daemon);
    client.type_line(ask);
    wait_clipboard(&mut client, 1);
    assert_eq!(client.clipboard_writes[0], b"from-pane".to_vec());
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
