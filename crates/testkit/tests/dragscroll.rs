//! A selection dragged to the edge of its pane scrolls the view by itself, also while the pointer
//! stands still, and the selection grows with it. Places are counted from 0 and sent from 1.

use std::time::Duration;

use arcade_testkit::{AttachOptions, Screened, TestClient, TestDaemon, WAIT};

const PREFIX: &[u8] = b"\x1b[109;5u";

fn report(client: &mut TestClient, code: u16, x: u16, y: u16, up: bool) {
    let end = if up { 'm' } else { 'M' };
    client.send(format!("\x1b[<{code};{};{}{end}", x + 1, y + 1).as_bytes());
}
fn press(client: &mut TestClient, x: u16, y: u16) {
    report(client, 0, x, y, false);
}
fn motion(client: &mut TestClient, x: u16, y: u16) {
    report(client, 32, x, y, false);
}
fn release(client: &mut TestClient, x: u16, y: u16) {
    report(client, 0, x, y, true);
}

fn start(daemon: &TestDaemon) -> TestClient {
    let mut client = daemon
        .attach(AttachOptions::new().size(100, 30).named("drag"))
        .unwrap();
    client.wait_for_prompt().unwrap();
    client
}

fn fill(client: &mut TestClient, lines: usize) {
    client.type_line(&format!(
        "i=1; while [ $i -le {lines} ]; do echo line-$i; i=$((i+1)); done; echo filled-$((1+1))"
    ));
    client.wait_for_text("filled-2").unwrap();
}

/// The `[above/history]` of the first row, within the first `cols` columns.
fn mark(client: &TestClient, cols: usize) -> Option<(usize, usize)> {
    let row: String = client.screen_rows()[0].chars().take(cols).collect();
    let end = row.rfind(']')?;
    let start = row[..end].rfind('[')?;
    let (a, b) = row[start + 1..end].split_once('/')?;
    Some((a.parse().ok()?, b.parse().ok()?))
}

fn above(client: &TestClient) -> usize {
    mark(client, 100).map_or(0, |m| m.0)
}

fn copied(client: &mut TestClient) -> Vec<String> {
    assert!(client.wait_until(WAIT, |c| !c.clipboard_writes.is_empty()));
    String::from_utf8_lossy(client.clipboard_writes.last().unwrap())
        .lines()
        .map(|l| l.trim_end().to_string())
        .collect()
}

fn numbers(lines: &[String]) -> Vec<usize> {
    lines
        .iter()
        .filter_map(|l| l.strip_prefix("line-").and_then(|n| n.parse().ok()))
        .collect()
}

/// The pointer stands still on the first row: the view goes up into the history by itself, the
/// selection ends up longer than a screen, and what `y` copies is the lines in order. Then the
/// pointer goes to the last row and the view comes down again.
#[test]
fn a_still_pointer_at_the_top_edge_scrolls_up_and_the_copy_is_the_lines_in_order() {
    let daemon = TestDaemon::start();
    let mut client = start(&daemon);
    fill(&mut client, 200);
    // The press is on a line of the visible screen; remember which.
    let (row, text) = client
        .screen_rows()
        .into_iter()
        .enumerate()
        .find(|(y, r)| *y == 20 && r.starts_with("line-"))
        .unwrap_or_else(|| panic!("{}", client.screen_text()));
    let anchor: usize = text
        .trim_end()
        .strip_prefix("line-")
        .unwrap()
        .parse()
        .unwrap();
    press(&mut client, 30, row as u16);
    motion(&mut client, 4, 10);
    motion(&mut client, 4, 0);
    // No more motion: only the timer scrolls.
    assert!(
        client.wait_until(Duration::from_secs(5), |c| above(c) >= 60),
        "{}",
        client.screen_text()
    );
    // Down again with the pointer at the last row of the pane.
    motion(&mut client, 4, 28);
    let now = above(&client);
    assert!(
        client.wait_until(Duration::from_secs(5), |c| above(c) < now),
        "{}",
        client.screen_text()
    );
    // Back to the top, to the end of the history.
    motion(&mut client, 4, 0);
    assert!(
        client.wait_until(Duration::from_secs(15), |c| {
            mark(c, 100).is_some_and(|(a, b)| a == b)
        }),
        "{}",
        client.screen_text()
    );
    release(&mut client, 4, 0);
    // The selection is kept after the release: copy it.
    client.send(b"y");
    let lines = copied(&mut client);
    let found = numbers(&lines);
    assert!(found.len() > 100, "{} lines: {lines:?}", found.len());
    assert_eq!(
        found.last().copied(),
        Some(anchor),
        "{:?}",
        &found[found.len() - 3..]
    );
    for pair in found.windows(2) {
        assert_eq!(pair[1], pair[0] + 1, "{pair:?}");
    }
}

/// With the pointer on the first row again the view stays at the top of the history: no more
/// scrolling and no error; and the release ends everything (the view stands still after it).
#[test]
fn the_release_stops_the_scrolling() {
    let daemon = TestDaemon::start();
    let mut client = start(&daemon);
    fill(&mut client, 200);
    press(&mut client, 0, 20);
    motion(&mut client, 4, 10);
    motion(&mut client, 4, 0);
    assert!(client.wait_until(Duration::from_secs(5), |c| above(c) >= 10));
    release(&mut client, 4, 0);
    client.wait_until(Duration::from_millis(300), |_| false);
    let at = above(&client);
    client.wait_until(Duration::from_millis(600), |_| false);
    assert_eq!(above(&client), at);
}

/// The pointer may go into the neighbour pane, over the divider or the bar: the drag is still the
/// first pane's, the selection never spills over, and the column is kept inside the pane.
#[test]
fn a_pointer_in_a_neighbour_pane_keeps_the_drag_in_its_own_pane() {
    let daemon = TestDaemon::start();
    let mut client = start(&daemon);
    fill(&mut client, 200);
    client.send(PREFIX);
    client.send(b"|");
    assert!(client.wait_until(WAIT, |c| c.screen_text().contains('│')));
    // The right pane holds its own text, to see whether anything spills.
    client.type_line("echo right-pane-text");
    client.wait_for_text("right-pane-text").unwrap();
    // A drag in the left pane that ends far into the right one, on its last row.
    press(&mut client, 2, 10);
    motion(&mut client, 30, 20);
    motion(&mut client, 80, 28);
    client.wait_until(Duration::from_millis(500), |_| false);
    client.send(b"y");
    let lines = copied(&mut client);
    assert!(
        lines.iter().all(|l| !l.contains("right-pane-text")),
        "{lines:?}"
    );
    release(&mut client, 80, 28);
}

/// The pane is closed in the middle of a drag at the edge: nothing breaks, the other pane works.
#[test]
fn closing_the_pane_in_the_middle_of_a_drag_ends_it() {
    let daemon = TestDaemon::start();
    let mut client = start(&daemon);
    fill(&mut client, 200);
    client.send(PREFIX);
    client.send(b"|");
    assert!(client.wait_until(WAIT, |c| c.screen_text().contains('│')));
    let right = client.focused_pane_id();
    // Drag in the right pane (the focus is there), then close it while the pointer waits.
    press(&mut client, 70, 20);
    motion(&mut client, 70, 10);
    motion(&mut client, 70, 0);
    daemon.command_from(Some(right), "pane.close", &[]).unwrap();
    client.wait_until(Duration::from_millis(500), |_| false);
    release(&mut client, 70, 0);
    client.type_line("echo alive-$((2+3))");
    client.wait_for_text("alive-5").unwrap();
}

/// A program that asked for the mouse still gets it, with no scrolling of Arcade's.
#[test]
fn a_program_that_asked_for_the_mouse_is_unchanged() {
    let daemon = TestDaemon::start();
    let mut client = start(&daemon);
    client.type_line("stty raw -echo; printf '\\033[?1002h\\033[?1006h'; echo re''ady; cat -vt");
    assert!(client.wait_until(WAIT, |c| {
        c.screen_rows().iter().any(|r| r.ends_with("ready"))
    }));
    press(&mut client, 5, 10);
    motion(&mut client, 5, 0);
    client.wait_until(Duration::from_millis(600), |_| false);
    assert!(
        client.screen_text().contains("^[[<"),
        "{}",
        client.screen_text()
    );
    assert_eq!(mark(&client, 100), None);
    release(&mut client, 5, 0);
}

/// Another window shown, or the pane resized, in the middle of a drag at the edge: the drag is
/// over, the view does not go on scrolling, and nothing is left running.
#[test]
fn showing_another_window_or_resizing_ends_the_drag() {
    let daemon = TestDaemon::start();
    let mut client = start(&daemon);
    fill(&mut client, 200);
    press(&mut client, 30, 20);
    motion(&mut client, 4, 10);
    motion(&mut client, 4, 0);
    assert!(client.wait_until(Duration::from_secs(5), |c| above(c) >= 5));
    let (ok, why) = daemon.command("window.new", &[]).unwrap();
    assert!(ok, "{why}");
    client.wait_until(Duration::from_millis(300), |_| false);
    let (ok, why) = daemon.command("window.select", &["1"]).unwrap();
    assert!(ok, "{why}");
    client.wait_until(Duration::from_millis(300), |_| false);
    let at = above(&client);
    client.wait_until(Duration::from_millis(600), |_| false);
    assert_eq!(above(&client), at);
    release(&mut client, 4, 0);
    // A resize of the pane ends it too.
    press(&mut client, 30, 20);
    motion(&mut client, 4, 10);
    motion(&mut client, 4, 0);
    assert!(client.wait_until(Duration::from_secs(5), |c| above(c) >= 5));
    client.send(PREFIX);
    client.send(b"|");
    client.wait_until(Duration::from_millis(600), |_| false);
    client.type_line("echo alive-$((2+3))");
    client.wait_for_text("alive-5").unwrap();
}
