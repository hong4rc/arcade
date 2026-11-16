//! Dragging a divider with the mouse: a left press on a divider and a drag move it, the two sides
//! it separates are resized as the resize commands do, and the programs are told their sizes.
//!
//! Places are counted from 0 here and sent from 1, as the host terminal does. The client is 101 by
//! 31, so the panes have 101 by 30 between them.

use std::time::{Duration, Instant};

use arcade_testkit::{AttachOptions, Screened, TestClient, TestDaemon, WAIT};

const PREFIX: &[u8] = b"\x1b[109;5u";

fn run(daemon: &TestDaemon, name: &str) {
    let (ok, output) = daemon.command(name, &[]).unwrap();
    assert!(ok, "{name}: {output}");
}

fn client(daemon: &TestDaemon) -> TestClient {
    let mut client = daemon
        .attach(AttachOptions::new().size(101, 31).named("div"))
        .unwrap();
    client.wait_for_prompt().unwrap();
    client
}

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

/// Press on the divider at `from`, move in a few steps to `to`, release there.
fn drag(client: &mut TestClient, from: (u16, u16), to: (u16, u16)) {
    press(client, from.0, from.1);
    motion(client, (from.0 + to.0) / 2, (from.1 + to.1) / 2);
    motion(client, to.0, to.1);
    release(client, to.0, to.1);
}

/// The columns of the screen that hold `│` on row `y`.
fn bars(client: &TestClient, y: u16) -> Vec<usize> {
    client.screen_rows()[usize::from(y)]
        .chars()
        .enumerate()
        .filter(|(_, c)| *c == '│')
        .map(|(x, _)| x)
        .collect()
}

/// What the focused pane's program says its size is, as `rows cols`.
fn size_here(client: &mut TestClient, marker: &str) -> (u16, u16) {
    client.type_line(&format!("echo {marker}-$(stty size | tr ' ' x)-end"));
    let mut found = None;
    let ok = client.wait_until(WAIT, |c| {
        for row in c.screen_rows() {
            // The echoed command line holds `$(`; the answer does not.
            if let Some(at) = row.find(&format!("{marker}-"))
                && !row.contains("$(")
            {
                let rest = &row[at + marker.len() + 1..];
                if let Some(end) = rest.find("-end") {
                    let (r, c) = rest[..end].split_once('x').unwrap();
                    found = Some((r.parse().unwrap(), c.parse().unwrap()));
                    return true;
                }
            }
        }
        false
    });
    assert!(ok, "{}", client.screen_text());
    found.unwrap()
}

fn settle(client: &mut TestClient) {
    std::thread::sleep(Duration::from_millis(200));
    client.read_more(Instant::now() + Duration::from_millis(100));
}

#[test]
fn dragging_a_vertical_divider_moves_it_and_tells_both_programs_their_new_sizes() {
    let daemon = TestDaemon::start();
    let mut client = client(&daemon);
    run(&daemon, "pane.split-right");
    assert!(client.wait_until(WAIT, |c| bars(c, 3) == vec![50]));
    drag(&mut client, (50, 5), (30, 5));
    assert!(
        client.wait_until(WAIT, |c| bars(c, 3) == vec![30]),
        "{}",
        client.screen_text()
    );
    // The right pane has the focus: 101 columns are 70, the divider, 30.
    assert_eq!(size_here(&mut client, "right"), (30, 70));
    run(&daemon, "pane.focus-left");
    assert_eq!(size_here(&mut client, "left"), (30, 30));
    // The divider is drawn from the top to the bottom of the window in its new column.
    for y in 0..30 {
        assert_eq!(bars(&client, y), vec![30], "row {y}");
    }
}

#[test]
fn dragging_a_horizontal_divider_moves_it_by_rows() {
    let daemon = TestDaemon::start();
    let mut client = client(&daemon);
    run(&daemon, "pane.split-down");
    let row_of_divider = |c: &TestClient| (0..30).find(|&y| c.screen_rows()[y].starts_with('─'));
    assert!(client.wait_until(WAIT, |c| row_of_divider(c).is_some()));
    let start = row_of_divider(&client).unwrap();
    drag(&mut client, (10, start as u16), (10, 8));
    assert!(
        client.wait_until(WAIT, |c| row_of_divider(c) == Some(8)),
        "{}",
        client.screen_text()
    );
    // The lower pane has the focus: 30 rows are 8, the divider, 21.
    assert_eq!(size_here(&mut client, "low"), (21, 101));
    run(&daemon, "pane.focus-up");
    assert_eq!(size_here(&mut client, "up"), (8, 101));
}

#[test]
fn a_drag_stops_at_the_minimum_size_and_does_not_go_back_when_the_pointer_does_not() {
    let daemon = TestDaemon::start();
    let mut client = client(&daemon);
    run(&daemon, "pane.split-right");
    assert!(client.wait_until(WAIT, |c| bars(c, 3) == vec![50]));
    // Far past the left edge: the left pane keeps its minimum width (5 columns).
    drag(&mut client, (50, 5), (0, 5));
    assert!(
        client.wait_until(WAIT, |c| bars(c, 3) == vec![5]),
        "{}",
        client.screen_text()
    );
    // Past the right edge: the right pane keeps 5 columns.
    drag(&mut client, (5, 5), (100, 5));
    assert!(
        client.wait_until(WAIT, |c| bars(c, 3) == vec![95]),
        "{}",
        client.screen_text()
    );
}

#[test]
fn in_nested_splits_the_divider_under_the_pointer_moves_and_no_other() {
    let daemon = TestDaemon::start();
    let mut client = client(&daemon);
    run(&daemon, "pane.split-right");
    assert!(client.wait_until(WAIT, |c| bars(c, 3) == vec![50]));
    // The right pane is split in two, one over the other.
    run(&daemon, "pane.split-down");
    let row_of_divider =
        |c: &TestClient| (0..30).find(|&y| c.screen_rows()[y].chars().nth(60) == Some('─'));
    assert!(client.wait_until(WAIT, |c| row_of_divider(c).is_some()));
    let start = row_of_divider(&client).unwrap();
    // Press on the horizontal divider inside the right column and move it up.
    drag(&mut client, (70, start as u16), (70, 6));
    assert!(
        client.wait_until(WAIT, |c| row_of_divider(c) == Some(6)),
        "{}",
        client.screen_text()
    );
    // The vertical divider and the left pane did not move.
    assert_eq!(bars(&client, 3), vec![50]);
    run(&daemon, "pane.focus-left");
    assert_eq!(size_here(&mut client, "nest"), (30, 50));
    // The vertical divider, pressed beside the horizontal one, moves the whole right column.
    drag(&mut client, (50, 20), (40, 20));
    assert!(
        client.wait_until(WAIT, |c| bars(c, 3) == vec![40]),
        "{}",
        client.screen_text()
    );
    assert_eq!(row_of_divider(&client), Some(6));
}

#[test]
fn a_press_that_is_not_followed_by_a_drag_changes_nothing() {
    let daemon = TestDaemon::start();
    let mut client = client(&daemon);
    run(&daemon, "pane.split-right");
    assert!(client.wait_until(WAIT, |c| bars(c, 3) == vec![50]));
    settle(&mut client);
    let before = client.screen_text();
    press(&mut client, 50, 5);
    release(&mut client, 50, 5);
    settle(&mut client);
    assert_eq!(bars(&client, 3), vec![50]);
    assert_eq!(client.screen_text(), before);
    assert_eq!(size_here(&mut client, "still"), (30, 50));
}

#[test]
fn a_divider_drag_is_arcades_even_when_the_program_beside_it_asked_for_the_mouse() {
    let daemon = TestDaemon::start();
    let mut client = client(&daemon);
    run(&daemon, "pane.split-right");
    assert!(client.wait_until(WAIT, |c| bars(c, 3) == vec![50]));
    // The right pane's program turns on all mouse reports and shows every byte it gets.
    client.type_line("stty raw -echo; printf '\\033[?1003h\\033[?1006h'; echo re''ady; cat -vt");
    assert!(client.wait_until(WAIT, |c| {
        c.screen_rows().iter().any(|r| r.ends_with("ready"))
    }));
    drag(&mut client, (50, 5), (35, 5));
    assert!(
        client.wait_until(WAIT, |c| bars(c, 3) == vec![35]),
        "{}",
        client.screen_text()
    );
    settle(&mut client);
    // Nothing of the drag reached the program: no escape sequence of a report on the screen.
    assert!(
        !client.screen_text().contains("^[[<"),
        "{}",
        client.screen_text()
    );
    // No selection began either: the pane is not in copy mode.
    assert!(!client.status_row().contains("COPY") && !client.status_row().contains("VISUAL"));
}

#[test]
fn a_release_outside_the_window_ends_the_drag_where_the_pointer_was() {
    let daemon = TestDaemon::start();
    let mut client = client(&daemon);
    run(&daemon, "pane.split-right");
    assert!(client.wait_until(WAIT, |c| bars(c, 3) == vec![50]));
    press(&mut client, 50, 5);
    motion(&mut client, 40, 5);
    // The button comes up on the status bar, a row that is not part of any pane.
    release(&mut client, 40, 30);
    assert!(
        client.wait_until(WAIT, |c| bars(c, 3) == vec![40]),
        "{}",
        client.screen_text()
    );
    // The drag is over: more moves do nothing.
    motion(&mut client, 20, 5);
    settle(&mut client);
    assert_eq!(bars(&client, 3), vec![40]);
}

#[test]
fn a_zoomed_pane_has_no_dividers_to_press() {
    let daemon = TestDaemon::start();
    let mut client = client(&daemon);
    run(&daemon, "pane.split-right");
    assert!(client.wait_until(WAIT, |c| bars(c, 3) == vec![50]));
    run(&daemon, "pane.zoom");
    assert!(client.wait_until(WAIT, |c| bars(c, 3).is_empty()));
    drag(&mut client, (50, 5), (30, 5));
    settle(&mut client);
    run(&daemon, "pane.zoom");
    assert!(
        client.wait_until(WAIT, |c| bars(c, 3) == vec![50]),
        "{}",
        client.screen_text()
    );
}

#[test]
fn the_programs_are_told_new_sizes_at_the_frame_rate_not_once_per_mouse_event() {
    let daemon = TestDaemon::start();
    let mut client = client(&daemon);
    run(&daemon, "pane.split-right");
    assert!(client.wait_until(WAIT, |c| bars(c, 3) == vec![50]));
    let dir = std::env::temp_dir().join(format!("arc-div-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let log = dir.join("winch");
    // The right pane's shell looks at its own size every 10 ms and logs each new size it sees.
    client.type_line(&format!(
        "(prev=; while :; do s=$(stty size); [ \"$s\" != \"$prev\" ] && echo \"$s\" >> {}; prev=$s; sleep 0.01; done) & echo loop''ed",
        log.display()
    ));
    assert!(client.wait_until(WAIT, |c| {
        c.screen_rows().iter().any(|r| r.ends_with("looped"))
    }));
    let frames_before = client.output_messages;
    let began = Instant::now();
    press(&mut client, 50, 5);
    // 40 moves of one cell, 5 ms apart: 200 ms when the machine keeps up, about 12 frames at 60
    // per second; a slow machine takes longer, and then more frames are allowed.
    for i in 0..40u16 {
        motion(&mut client, 49 - i, 5);
        std::thread::sleep(Duration::from_millis(5));
    }
    release(&mut client, 30, 5);
    assert!(
        client.wait_until(WAIT, |c| bars(c, 3) == vec![30]),
        "{}",
        client.screen_text()
    );
    // From the first move to the screen showing the last place of the divider.
    let elapsed = began.elapsed();
    settle(&mut client);
    // What is allowed: one update for each frame interval of the time it took, a few more for the
    // ends of the drag, and in any case fewer than half of the moves. (The statement is that the
    // rate of the programs' news follows the frame rate, not the rate of the mouse.)
    let allowed = (elapsed.as_secs_f64() / 0.016_667).ceil() as usize + 3;
    let allowed = allowed.min(20);
    let frames = client.output_messages - frames_before;
    assert!(
        frames <= allowed,
        "{frames} screen updates for 40 mouse moves in {elapsed:?} (allowed {allowed})"
    );
    let winches = std::fs::read_to_string(&log).map_or(0, |t| t.lines().count());
    // The first line is the size it had before the drag.
    assert!(winches >= 3, "the program saw too few sizes: {winches}");
    assert!(
        winches <= allowed,
        "the program saw {winches} sizes for 40 mouse moves in {elapsed:?} (allowed {allowed})"
    );
    let _ = std::fs::remove_dir_all(&dir);
    let _ = PREFIX;
}
