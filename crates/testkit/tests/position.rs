//! A pane that is scrolled back shows `[above/history]` at the right of its first row, as tmux
//! does: how many lines the view is above the bottom, over how many lines of history there are.

use arcade_testkit::{AttachOptions, Screened, TestClient, TestDaemon, WAIT};

const PREFIX: &[u8] = b"\x1b[109;5u";

fn client(daemon: &TestDaemon) -> TestClient {
    let mut client = daemon
        .attach(AttachOptions::new().size(100, 30).named("pos"))
        .unwrap();
    client.wait_for_prompt().unwrap();
    client
}

fn fill(client: &mut TestClient, lines: usize) {
    client.type_line(&format!(
        "i=1; while [ $i -le {lines} ]; do echo line-$i; i=$((i+1)); done; echo filled-$((100 + {lines}))"
    ));
    client
        .wait_for_text(&format!("filled-{}", 100 + lines))
        .unwrap();
}

fn wheel(client: &mut TestClient, up: bool, x: u16, y: u16) {
    let code = if up { 64 } else { 65 };
    client.send(format!("\x1b[<{code};{};{}M", x + 1, y + 1).as_bytes());
}

/// The numbers of the first mark on a row of the screen, `[a/b]`.
fn mark_in(row: &str) -> Option<(usize, usize)> {
    let end = row.rfind(']')?;
    let start = row[..end].rfind('[')?;
    let (a, b) = row[start + 1..end].split_once('/')?;
    Some((a.parse().ok()?, b.parse().ok()?))
}

fn mark(
    client: &TestClient,
    row: usize,
    pane_cols: std::ops::Range<usize>,
) -> Option<(usize, usize)> {
    let text: String = client.screen_rows()[row]
        .chars()
        .skip(pane_cols.start)
        .take(pane_cols.len())
        .collect();
    mark_in(&text)
}

fn enter_copy(client: &mut TestClient) {
    client.send(PREFIX);
    client.send(b"[");
    assert!(client.wait_until(WAIT, |c| format!("{} ", c.status_row()).contains(" COPY ")));
}

#[test]
fn the_mark_comes_with_the_wheel_shows_the_place_and_goes_at_the_bottom() {
    let daemon = TestDaemon::start();
    let mut client = client(&daemon);
    fill(&mut client, 200);
    assert!(mark(&client, 0, 0..100).is_none());
    wheel(&mut client, true, 10, 10);
    assert!(
        client.wait_until(WAIT, |c| mark(c, 0, 0..100).is_some()),
        "{}",
        client.screen_text()
    );
    let (above, history) = mark(&client, 0, 0..100).unwrap();
    // One turn of the wheel is three rows; the history is the lines above the live screen.
    assert_eq!(above, 3);
    assert!(history >= 170, "{history}");
    wheel(&mut client, true, 10, 10);
    assert!(client.wait_until(WAIT, |c| mark(c, 0, 0..100) == Some((6, history))));
    // Back down to the bottom: the pane leaves copy mode and the mark goes.
    wheel(&mut client, false, 10, 10);
    wheel(&mut client, false, 10, 10);
    assert!(
        client.wait_until(WAIT, |c| mark(c, 0, 0..100).is_none()),
        "{}",
        client.screen_text()
    );
    assert!(!client.status_row().contains("COPY"));
}

#[test]
fn at_the_top_of_the_history_the_two_numbers_are_the_same_and_at_the_bottom_of_copy_mode_it_is_zero()
 {
    let daemon = TestDaemon::start();
    let mut client = client(&daemon);
    fill(&mut client, 200);
    enter_copy(&mut client);
    // Copy mode at the bottom: the mark is there, with 0, so it does not come and go.
    let (above, history) = mark(&client, 0, 0..100).unwrap();
    assert_eq!(above, 0);
    client.send(b"g");
    assert!(
        client.wait_until(WAIT, |c| mark(c, 0, 0..100) == Some((history, history))),
        "{}",
        client.screen_text()
    );
    client.send(b"G");
    assert!(client.wait_until(WAIT, |c| mark(c, 0, 0..100) == Some((0, history))));
}

#[test]
fn each_scrolled_pane_has_its_own_mark_and_a_narrow_pane_has_none() {
    let daemon = TestDaemon::start();
    let mut client = client(&daemon);
    fill(&mut client, 150);
    client.split_right();
    client.wait_for_prompt().ok();
    fill(&mut client, 120);
    let divider = client
        .screen_rows()
        .iter()
        .find_map(|r| r.chars().position(|c| c == '│'))
        .unwrap();
    // The right pane is scrolled one turn, the left pane three.
    wheel(&mut client, true, (divider + 5) as u16, 10);
    assert!(client.wait_until(WAIT, |c| mark(c, 0, divider + 1..100).is_some()));
    assert!(mark(&client, 0, 0..divider).is_none());
    wheel(&mut client, true, 5, 10);
    wheel(&mut client, true, 5, 10);
    assert!(
        client.wait_until(WAIT, |c| mark(c, 0, 0..divider).is_some_and(|m| m.0 == 6)),
        "{}",
        client.screen_text()
    );
    let left = mark(&client, 0, 0..divider).unwrap();
    let right = mark(&client, 0, divider + 1..100).unwrap();
    assert_eq!(left.0, 6);
    assert_eq!(right.0, 3);
    assert_ne!(left.1, 0);
}

#[test]
fn a_selection_over_the_first_row_does_not_copy_the_mark() {
    let daemon = TestDaemon::start();
    let mut client = client(&daemon);
    fill(&mut client, 100);
    enter_copy(&mut client);
    client.send(b"g");
    assert!(client.wait_until(WAIT, |c| mark(c, 0, 0..100).is_some_and(|m| m.0 == m.1)));
    let (_, history) = mark(&client, 0, 0..100).unwrap();
    // The first row is selected, line by line, with the row below it, and copied.
    client.send(b"Vjy");
    assert!(
        client.wait_until(WAIT, |c| !c.clipboard_writes.is_empty()),
        "{}",
        client.screen_text()
    );
    let text = String::from_utf8_lossy(&client.clipboard_writes[0]).into_owned();
    assert!(!text.contains(&format!("/{history}]")), "{text:?}");
    assert!(text.contains("line-1"), "{text:?}");
}

#[test]
fn the_setting_turns_the_mark_off() {
    let daemon = TestDaemon::start_with(|c| c.config.scroll_position = false);
    let mut client = client(&daemon);
    fill(&mut client, 100);
    enter_copy(&mut client);
    client.send(b"g");
    client.wait_until(std::time::Duration::from_millis(400), |_| false);
    assert!(
        mark(&client, 0, 0..100).is_none(),
        "{}",
        client.screen_text()
    );
}

/// A measurement, not a check: a pane scrolled back while its program floods it with output, the
/// one case the mark adds work to. Run with `--ignored --nocapture`; it prints what the client
/// received in three seconds with the mark on and with it off.
#[test]
#[ignore]
fn measure_a_scrolled_pane_while_output_arrives() {
    for on in [true, false, true, false] {
        let daemon = TestDaemon::start_with(move |c| c.config.scroll_position = on);
        let mut client = client(&daemon);
        fill(&mut client, 300);
        client.type_line("yes some-flooding-output | head -c 400000000");
        client.wait_for_text("some-flooding-output").unwrap();
        wheel(&mut client, true, 10, 10);
        assert!(client.wait_until(WAIT, |c| c.status_row().contains("COPY")));
        let (m0, b0) = (client.output_messages, client.output_bytes);
        let start = std::time::Instant::now();
        while start.elapsed() < std::time::Duration::from_secs(3) {
            client.read_more(std::time::Instant::now() + std::time::Duration::from_millis(50));
        }
        eprintln!(
            "mark {}: {} frames, {} bytes in 3 s",
            if on { "on " } else { "off" },
            client.output_messages - m0,
            client.output_bytes - b0
        );
        client.send(b"\x03");
    }
}
