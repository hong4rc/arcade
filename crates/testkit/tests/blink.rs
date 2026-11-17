//! A state that blinks: the listed states change between their colour and a dim one on the status
//! bar, all in the same step, with nothing woken when nothing blinks. Every file lives in a
//! directory of its own under the temporary directory.

use std::path::PathBuf;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::time::{Duration, Instant};

/// A configuration that turns blinking off (the default blinks the states that wait for you).
const NO_BLINK: &str = "[status]\nblink = []\n";

use arcade_testkit::{AttachOptions, Screened, TestClient, TestDaemon, WAIT};

static COUNTER: AtomicUsize = AtomicUsize::new(0);
const PREFIX: &[u8] = b"\x1b[109;5u";

fn config_file(body: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!(
        "arc-blink-{}-{}",
        std::process::id(),
        COUNTER.fetch_add(1, Ordering::Relaxed)
    ));
    std::fs::create_dir_all(&dir).unwrap();
    let file = dir.join("config.toml");
    std::fs::write(&file, body).unwrap();
    file
}

fn daemon_with(file: &std::path::Path) -> TestDaemon {
    let path = file.to_path_buf();
    TestDaemon::start_with(move |c| c.config_path = Some(path))
}

fn notify(daemon: &TestDaemon, pane: u64, class: &str) {
    let target = format!("target={pane}");
    let class = format!("class={class}");
    let (ok, why) = daemon
        .command(
            "notify.send",
            &["version=1", &target, &class, "source=test", "message=hi"],
        )
        .unwrap();
    assert!(ok, "{why}");
}

/// Three windows with the third in front; the numbers of the panes of the first two.
fn three_windows(daemon: &TestDaemon) -> (TestClient, u64, u64) {
    let mut client = daemon
        .attach(AttachOptions::new().size(100, 30).named("work"))
        .unwrap();
    client.wait_for_prompt().unwrap();
    let first = client.focused_pane_id();
    client.send(PREFIX);
    client.send(b"c");
    client.wait_for_prompt().unwrap();
    let second = client.focused_pane_id();
    client.send(PREFIX);
    client.send(b"c");
    client.wait_for_prompt().unwrap();
    (client, first, second)
}

/// The place of the first cell that shows `symbol` on the bar (the last row).
fn find(client: &TestClient, symbol: char) -> Option<(u16, u16)> {
    let rows = client.screen_rows();
    let y = rows.len() - 1;
    let x = rows[y].chars().position(|c| c == symbol)?;
    Some((y as u16, x as u16))
}

/// Look at the cell for `time`, every time the client has read something: the colours it had.
fn colours_of(client: &mut TestClient, at: (u16, u16), time: Duration) -> Vec<arcade_term::Color> {
    let mut seen = Vec::new();
    let end = Instant::now() + time;
    while Instant::now() < end {
        client.read_more(Instant::now() + Duration::from_millis(20));
        let fg = client.style_at(at.0, at.1).fg;
        if seen.last() != Some(&fg) {
            seen.push(fg);
        }
    }
    seen
}

/// How many times the client was sent something during `time`.
fn updates_in(client: &mut TestClient, time: Duration) -> usize {
    let end = Instant::now() + time;
    let mut n = 0;
    while Instant::now() < end {
        if client.read_more(end) {
            n += 1;
        }
    }
    n
}

const BLINKING: &str = "[status]\nblink = [\"input\"]\nblink-interval = 200\n";

#[test]
fn only_the_listed_states_blink_and_the_bar_keeps_its_width() {
    let file = config_file(BLINKING);
    let daemon = daemon_with(&file);
    let (mut client, first, second) = three_windows(&daemon);
    notify(&daemon, first, "input");
    notify(&daemon, second, "error");
    assert!(client.wait_until(WAIT, |c| find(c, '◆').is_some() && find(c, '■').is_some()));
    let width = client.status_row().chars().count();
    let input = find(&client, '◆').unwrap();
    let error = find(&client, '■').unwrap();
    // The terminal being somewhere else changes nothing.
    client.send(b"\x1b[O");
    let seen = colours_of(&mut client, input, Duration::from_millis(1500));
    assert!(seen.len() >= 4, "input did not blink: {seen:?}");
    let mut distinct: Vec<arcade_term::Color> = Vec::new();
    for colour in &seen {
        if !distinct.contains(colour) {
            distinct.push(*colour);
        }
    }
    assert_eq!(distinct.len(), 2, "{seen:?}");
    let steady = colours_of(&mut client, error, Duration::from_millis(800));
    assert_eq!(steady.len(), 1, "error blinked: {steady:?}");
    // The same symbol in the same place, the bar as wide.
    assert_eq!(find(&client, '◆'), Some(input));
    assert_eq!(client.status_row().chars().count(), width);
}

#[test]
fn two_blinking_tabs_change_in_the_same_update() {
    let file = config_file(BLINKING);
    let daemon = daemon_with(&file);
    let (mut client, first, second) = three_windows(&daemon);
    notify(&daemon, first, "input");
    notify(&daemon, second, "input");
    assert!(client.wait_until(WAIT, |c| { c.status_row().matches('◆').count() == 2 }));
    let row = client.screen_rows().len() as u16 - 1;
    let line: Vec<char> = client.status_row().chars().collect();
    let cols: Vec<u16> = line
        .iter()
        .enumerate()
        .filter(|(_, c)| **c == '◆')
        .map(|(i, _)| i as u16)
        .collect();
    let end = Instant::now() + Duration::from_millis(1500);
    let mut changes = 0;
    let mut last = None;
    while Instant::now() < end {
        client.read_more(Instant::now() + Duration::from_millis(10));
        let a = client.style_at(row, cols[0]).fg;
        let b = client.style_at(row, cols[1]).fg;
        assert_eq!(a, b, "the two symbols were not in the same step");
        if last != Some(a) {
            changes += 1;
            last = Some(a);
        }
    }
    assert!(changes >= 4, "{changes}");
}

#[test]
fn nothing_is_woken_when_nothing_blinks() {
    // The list names a state that is not on the bar: no symbol blinks, so no update comes.
    let file = config_file(BLINKING);
    let daemon = daemon_with(&file);
    let (mut client, first, _) = three_windows(&daemon);
    notify(&daemon, first, "info");
    assert!(client.wait_until(WAIT, |c| find(c, '●').is_some()));
    updates_in(&mut client, Duration::from_millis(300));
    assert_eq!(updates_in(&mut client, Duration::from_secs(2)), 0);
    // With one that blinks the same two seconds bring about ten.
    notify(&daemon, first, "input");
    assert!(client.wait_until(WAIT, |c| find(c, '◆').is_some()));
    let n = updates_in(&mut client, Duration::from_secs(2));
    assert!(n >= 6, "{n}");
}

#[test]
fn a_reload_turns_blinking_on_and_off() {
    let file = config_file(NO_BLINK);
    let daemon = daemon_with(&file);
    let (mut client, first, _) = three_windows(&daemon);
    notify(&daemon, first, "input");
    assert!(client.wait_until(WAIT, |c| find(c, '◆').is_some()));
    updates_in(&mut client, Duration::from_millis(1500));
    assert_eq!(updates_in(&mut client, Duration::from_millis(1200)), 0);
    std::fs::write(&file, BLINKING).unwrap();
    assert!(daemon.command("arcade.reload-config", &[]).unwrap().0);
    let at = find(&client, '◆').unwrap();
    assert!(colours_of(&mut client, at, Duration::from_millis(1500)).len() >= 4);
    std::fs::write(&file, NO_BLINK).unwrap();
    assert!(daemon.command("arcade.reload-config", &[]).unwrap().0);
    updates_in(&mut client, Duration::from_millis(1500));
    assert_eq!(updates_in(&mut client, Duration::from_millis(1200)), 0);
}

#[test]
fn a_wrong_blink_setting_is_refused_with_its_line() {
    let file = config_file("");
    let daemon = daemon_with(&file);
    let _client = daemon
        .attach(AttachOptions::new().size(100, 30).named("work"))
        .unwrap();
    for (body, expect) in [
        ("[status]\nblink = [\"loud\"]\n", "line 2"),
        ("[status]\nblink = \"input\"\n", "line 2"),
        ("[status]\nblink-interval = 100\n", "line 2"),
    ] {
        std::fs::write(&file, body).unwrap();
        let (ok, message) = daemon.command("arcade.reload-config", &[]).unwrap();
        assert!(!ok, "{body}");
        assert!(message.contains(expect), "{body}: {message}");
    }
}

/// A step of a blink sends only the cells that changed: the symbol's own cell(s), not the bar.
#[test]
fn a_step_of_a_blink_sends_only_the_cells_that_changed() {
    let file = config_file(BLINKING);
    let daemon = daemon_with(&file);
    let (mut client, first, _) = three_windows(&daemon);
    notify(&daemon, first, "input");
    assert!(client.wait_until(WAIT, |c| find(c, '◆').is_some()));
    updates_in(&mut client, Duration::from_millis(600));
    let (bytes, messages) = (client.output_bytes, client.output_messages);
    let n = updates_in(&mut client, Duration::from_millis(2000));
    let per_step = (client.output_bytes - bytes) / (client.output_messages - messages).max(1);
    eprintln!("blink: {n} updates in 2 s, {per_step} bytes each");
    assert!(n >= 6, "{n}");
    assert!(per_step < 150, "{per_step} bytes for one step of a blink");
}
