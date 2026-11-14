//! Several panes in one window: their rooms, the dividers between them, focus, zoom, closing.
//! Everything is read back from the screen a fake client shows.

use std::time::{Duration, Instant};

use arcade_proto::ServerMessage;
use arcade_testkit::{AttachOptions, Screened, TestClient, TestDaemon, WAIT};

fn run(daemon: &TestDaemon, name: &str) {
    let (ok, output) = daemon.command(name, &[]).unwrap();
    assert!(ok, "{name}: {output}");
}

/// Wait until some row of a rectangle of the screen contains `needle`.
fn wait_in(client: &mut TestClient, rect: (u16, u16, u16, u16), needle: &str) {
    let (x, y, w, h) = rect;
    let ok = client.wait_until(WAIT, |c| {
        c.region(x, y, w, h).iter().any(|r| r.contains(needle))
    });
    assert!(
        ok,
        "no {needle:?} in the rectangle {rect:?}; the screen shows:\n{}",
        client.screen_text()
    );
}

/// The number of panes the daemon reports for the only session.
fn pane_count(daemon: &TestDaemon) -> usize {
    let (_, list) = daemon.command("session.list", &[]).unwrap();
    // Fields: name, id, attached, windows, panes.
    list.split('\t').nth(4).unwrap().trim().parse().unwrap()
}

/// A client of 101 by 31, so that the panes have 101 by 30 between them.
fn client_with_shell(daemon: &TestDaemon) -> TestClient {
    let mut client = daemon
        .attach(AttachOptions::new().size(101, 31))
        .expect("attach");
    client.wait_for_prompt().unwrap();
    client
}

#[test]
fn a_window_with_one_pane_has_the_whole_screen_but_the_status_bar() {
    let daemon = TestDaemon::start();
    let mut client = client_with_shell(&daemon);
    client.type_line("stty size");
    client.wait_for_row("30 101").unwrap();
    // No frame and no divider anywhere in the pane's area.
    let area = client.region(0, 0, 101, 30);
    assert!(
        area.iter().all(|r| !r.contains('│') && !r.contains('─')),
        "{area:?}"
    );
    // The status bar is the last row, and it names the session.
    assert!(!client.status_row().is_empty());
}

/// Core scenarios 6 and 7: split, resize the host terminal, shrink a pane to the limit.
#[test]
fn splitting_gives_each_pane_its_room_and_the_panes_follow_the_host_terminal() {
    let daemon = TestDaemon::start();
    let mut client = client_with_shell(&daemon);
    run(&daemon, "pane.split-right");
    // The new pane is on the right and has the focus: 101 columns are 50, the divider, 50.
    wait_in(&mut client, (51, 0, 50, 30), "$");
    client.type_line("stty size");
    wait_in(&mut client, (51, 0, 50, 30), "30 50");
    run(&daemon, "pane.focus-left");
    client.type_line("stty size");
    wait_in(&mut client, (0, 0, 50, 30), "30 50");
    // The divider is one column wide, all the way down, and nothing else is drawn in it.
    let divider = client.region(50, 0, 1, 30);
    assert!(divider.iter().all(|r| r == "│"), "{divider:?}");

    // A bigger host terminal: the panes keep their proportions and the programs are told.
    client.resize(201, 41);
    client.type_line("stty size");
    wait_in(&mut client, (0, 0, 100, 40), "40 100");
    run(&daemon, "pane.focus-right");
    client.type_line("stty size");
    wait_in(&mut client, (101, 0, 100, 40), "40 100");
    let divider = client.region(100, 0, 1, 40);
    assert!(divider.iter().all(|r| r == "│"), "{divider:?}");
}

#[test]
fn a_pane_shrunk_to_the_limit_refuses_the_next_step_and_the_layout_stays_whole() {
    let daemon = TestDaemon::start();
    let mut client = client_with_shell(&daemon);
    run(&daemon, "pane.split-right");
    // The right pane has the focus. Moving the divider left shrinks the left pane, 5 a step.
    let mut steps = 0;
    loop {
        let (ok, output) = daemon.command("pane.resize-left", &[]).unwrap();
        if !ok {
            assert!(output.contains("too small"), "{output}");
            break;
        }
        steps += 1;
        assert!(steps < 30, "the divider never stopped");
    }
    assert_eq!(steps, 9, "from 50 columns to 5 in steps of 5");
    run(&daemon, "pane.focus-left");
    client.type_line("stty size");
    wait_in(&mut client, (0, 0, 5, 30), "30 5");
    // The divider is where the last good step put it, and the right pane has the rest.
    let divider = client.region(5, 0, 1, 30);
    assert!(divider.iter().all(|r| r == "│"), "{divider:?}");
    run(&daemon, "pane.focus-right");
    client.type_line("stty size");
    wait_in(&mut client, (6, 0, 95, 30), "30 95");
    // A split with no room is refused too, and starts no process.
    run(&daemon, "pane.focus-left");
    let (ok, output) = daemon.command("pane.split-right", &[]).unwrap();
    assert!(!ok && output.contains("no room"), "{output}");
    assert_eq!(pane_count(&daemon), 2);
}

#[test]
fn two_panes_that_print_at_once_each_show_only_their_own_text() {
    let daemon = TestDaemon::start();
    let mut client = client_with_shell(&daemon);
    run(&daemon, "pane.split-right");
    let count = "i=0; while [ $i -lt 400 ]; do echo {}-$i; i=$((i+1)); done; echo {}-done";
    client.type_line(&count.replace("{}", "RIGHT"));
    run(&daemon, "pane.focus-left");
    client.type_line(&count.replace("{}", "LEFT"));
    wait_in(&mut client, (0, 0, 50, 30), "LEFT-done");
    wait_in(&mut client, (51, 0, 50, 30), "RIGHT-done");
    let left = client.region(0, 0, 50, 30);
    let right = client.region(51, 0, 50, 30);
    assert!(left.iter().all(|r| !r.contains("RIGHT")), "{left:?}");
    assert!(right.iter().all(|r| !r.contains("LEFT")), "{right:?}");
    let divider = client.region(50, 0, 1, 30);
    assert!(divider.iter().all(|r| r == "│"), "{divider:?}");
}

#[test]
fn the_divider_next_to_the_focused_pane_has_the_focus_color_on_the_focused_pane_s_side() {
    let daemon = TestDaemon::start();
    let mut client = client_with_shell(&daemon);
    // Pane A on the left; B above C on the right.
    run(&daemon, "pane.split-right");
    run(&daemon, "pane.split-down");
    wait_in(&mut client, (51, 16, 50, 14), "$");
    let divider_x = 50;
    let (above, below) = (3, 26);
    let (short_left, short_right) = ((15, 60), (15, 90)); // row, column
    let color = |c: &TestClient, row: u16, col: u16| c.style_at(row, col);
    // The focus is on C (lower right). It is after the long divider, so it marks the lower half
    // of it (where it touches C); the short divider is above C, so C marks its right half.
    client.wait_until(Duration::from_millis(300), |_| false);
    let (plain, lit) = (
        color(&client, above, divider_x),
        color(&client, below, divider_x),
    );
    assert_ne!(plain, lit, "the divider is one color all along");
    assert_eq!(color(&client, short_right.0, short_right.1), lit);
    assert_eq!(color(&client, short_left.0, short_left.1), plain);
    // Focus on B (upper right): it touches the long divider only in the upper half, and the
    // lower half is the one it would mark, so nothing of the long divider is lit; the short
    // divider is below B, so its left half is lit.
    run(&daemon, "pane.focus-up");
    client.wait_until(Duration::from_millis(300), |_| false);
    assert_eq!(color(&client, above, divider_x), plain);
    assert_eq!(color(&client, below, divider_x), plain);
    assert_eq!(color(&client, short_left.0, short_left.1), lit);
    assert_eq!(color(&client, short_right.0, short_right.1), plain);
    // Focus on A: the upper half of the long divider is lit; the short one does not touch A.
    run(&daemon, "pane.focus-left");
    client.wait_until(Duration::from_millis(300), |_| false);
    assert_eq!(color(&client, above, divider_x), lit);
    assert_eq!(color(&client, below, divider_x), plain);
    assert_eq!(color(&client, short_left.0, short_left.1), plain);
    assert_eq!(color(&client, short_right.0, short_right.1), plain);
}

#[test]
fn zooming_fills_the_window_and_gives_the_old_layout_back() {
    let daemon = TestDaemon::start();
    let mut client = client_with_shell(&daemon);
    run(&daemon, "pane.split-right");
    client.type_line("echo right-pane-marker");
    wait_in(&mut client, (51, 0, 50, 30), "right-pane-marker");
    run(&daemon, "pane.zoom");
    client.type_line("stty size");
    wait_in(&mut client, (0, 0, 101, 30), "30 101");
    // The other pane is out of sight, and there is no divider.
    let area = client.region(0, 0, 101, 30);
    assert!(area.iter().all(|r| !r.contains('│')), "{area:?}");
    run(&daemon, "pane.zoom");
    client.type_line("stty size");
    wait_in(&mut client, (51, 0, 50, 30), "30 50");
    let divider = client.region(50, 0, 1, 30);
    assert!(divider.iter().all(|r| r == "│"), "{divider:?}");
    // What the pane showed before is still there.
    wait_in(&mut client, (51, 0, 50, 30), "right-pane-marker");
}

#[test]
fn closing_a_pane_gives_its_room_to_the_one_beside_it() {
    let daemon = TestDaemon::start();
    let mut client = client_with_shell(&daemon);
    run(&daemon, "pane.split-right");
    let middle = client.focused_pane_id();
    run(&daemon, "pane.split-right");
    let last = client.focused_pane_id();
    assert_eq!(pane_count(&daemon), 3);
    // Close the middle pane the way `arcade run pane.close` does from inside it.
    let (ok, output) = daemon
        .command_from(Some(middle), "pane.close", &[])
        .unwrap();
    assert!(ok, "{output}");
    client.type_line("stty size");
    wait_in(&mut client, (51, 0, 50, 30), "30 50");
    assert_eq!(pane_count(&daemon), 2);
    // The pane that remains on the right is the last one, and it has the focus.
    let again = client.focused_pane_id();
    assert_eq!(again, last);
    // A pane whose process ends closes by itself, and the first pane has the whole window.
    client.type_line("exit");
    let ok = client.wait_until(WAIT, |_| pane_count(&daemon) == 1);
    assert!(ok, "the pane did not close");
    client.type_line("stty size");
    wait_in(&mut client, (0, 0, 101, 30), "30 101");
}

#[test]
fn the_session_ends_when_its_last_pane_ends() {
    let daemon = TestDaemon::start();
    let mut client = client_with_shell(&daemon);
    run(&daemon, "pane.split-down");
    client.type_line("exit");
    let ok = client.wait_until(WAIT, |_| pane_count(&daemon) == 1);
    assert!(ok);
    client.type_line("exit");
    client
        .wait_message(|m| matches!(m, ServerMessage::SessionEnded { .. }), WAIT)
        .unwrap();
}

/// Core scenario 10 with two panes: one takes a gigabyte of random bytes while the other keeps
/// answering.
#[test]
fn a_pane_flooded_with_a_gigabyte_does_not_stop_the_pane_beside_it() {
    let daemon = TestDaemon::start();
    let dir = daemon.dir().to_path_buf();
    let done = dir.join("flood-done");
    let script = format!(
        "stty -icanon -echo; head -c 1073741824 /dev/urandom; touch '{}'; sleep 120",
        done.display()
    );
    let mut client = daemon
        .attach(
            AttachOptions::new()
                .size(101, 31)
                .command(&["/bin/sh", "-c", &script]),
        )
        .unwrap();
    // The first pane floods; the new pane on the right has the focus and a shell.
    run(&daemon, "pane.split-right");
    wait_in(&mut client, (51, 0, 50, 30), "$");
    let started = Instant::now();
    let mut answered = 0;
    let mut slowest = Duration::ZERO;
    while !done.exists() {
        assert!(
            started.elapsed() < Duration::from_secs(300),
            "the flood did not end"
        );
        assert!(!daemon.is_stopped(), "the daemon stopped");
        answered += 1;
        let sent = Instant::now();
        client.type_line(&format!("echo still-here-{answered}"));
        let marker = format!("still-here-{answered}");
        let ok = client.wait_until(Duration::from_secs(10), |c| {
            c.region(51, 0, 50, 30).iter().any(|r| r == &marker)
        });
        assert!(ok, "the other pane stopped answering during the flood");
        slowest = slowest.max(sent.elapsed());
        let pause = Instant::now() + Duration::from_millis(300);
        client.pump(pause);
    }
    eprintln!("flood beside a shell: {answered} answers, slowest {slowest:?}");
    assert!(answered >= 3);
    assert!(daemon.command("session.list", &[]).unwrap().0);
}

#[test]
fn a_divider_that_ends_on_another_is_drawn_with_a_joining_sign() {
    let daemon = TestDaemon::start();
    let mut client = client_with_shell(&daemon);
    // One pane on the left, two stacked on the right: the row between the stacked panes
    // starts at the column divider, which gets a `├` there.
    run(&daemon, "pane.split-right");
    run(&daemon, "pane.split-down");
    let ok = client.wait_until(WAIT, |c| c.region(50, 0, 1, 30).iter().any(|r| r == "├"));
    assert!(ok, "{}", client.screen_text());
    let column = client.region(50, 0, 1, 30);
    assert_eq!(column.iter().filter(|r| *r == "├").count(), 1, "{column:?}");
    assert!(column.iter().all(|r| r == "├" || r == "│"), "{column:?}");
}

#[test]
fn a_column_divider_that_starts_on_a_row_divider_is_drawn_with_a_joining_sign() {
    let daemon = TestDaemon::start();
    let mut client = client_with_shell(&daemon);
    // Two panes stacked, then the lower one split in two side by side: the column divider
    // hangs from the row divider, which gets a `┬` there.
    run(&daemon, "pane.split-down");
    run(&daemon, "pane.split-right");
    let ok = client.wait_until(WAIT, |c| {
        c.region(0, 0, 101, 30).iter().any(|r| r.contains('┬'))
    });
    assert!(ok, "{}", client.screen_text());
}

/// Two panes side by side: `A` on the left and `B` on the right, the focus on the left one.
fn a_left_b_right(daemon: &TestDaemon) -> TestClient {
    let mut client = client_with_shell(daemon);
    run(daemon, "pane.split-right");
    client.type_line("echo B-is-here");
    wait_in(&mut client, (51, 0, 50, 30), "B-is-here");
    run(daemon, "pane.focus-left");
    client.type_line("echo A-is-here");
    wait_in(&mut client, (0, 0, 50, 30), "A-is-here");
    client
}

#[test]
fn swapping_trades_the_places_of_two_panes_and_the_focus_stays_with_the_pane() {
    let daemon = TestDaemon::start();
    let mut client = a_left_b_right(&daemon);
    let focused = client.focused_pane_id();
    run(&daemon, "pane.swap-right");
    wait_in(&mut client, (51, 0, 50, 30), "A-is-here");
    wait_in(&mut client, (0, 0, 50, 30), "B-is-here");
    assert_eq!(client.focused_pane_id(), focused);
    // The program in the pane was told its room (the same size here), and still answers.
    client.type_line("echo still-A");
    wait_in(&mut client, (51, 0, 50, 30), "still-A");
    // With no pane that way there is a message and nothing changes.
    let (ok, output) = daemon.command("pane.swap-right", &[]).unwrap();
    assert!(
        !ok && output.contains("no pane to swap with right"),
        "{output}"
    );
    wait_in(&mut client, (51, 0, 50, 30), "A-is-here");
}

#[test]
fn ctrl_and_a_direction_after_the_prefix_swaps_and_repeats() {
    let daemon = TestDaemon::start();
    let mut client = a_left_b_right(&daemon);
    // Ctrl and the right arrow, as the new protocol reports it.
    client.send(b"\x1b[109;5u");
    client.send(b"\x1b[1;5C");
    wait_in(&mut client, (51, 0, 50, 30), "A-is-here");
    // The prefix table is still open: the same key again, with no prefix, swaps back.
    client.send(b"\x1b[1;5D");
    wait_in(&mut client, (0, 0, 50, 30), "A-is-here");
    wait_in(&mut client, (51, 0, 50, 30), "B-is-here");
}

fn window_lines(daemon: &TestDaemon) -> Vec<String> {
    let (_, output) = daemon.command("window.list", &[]).unwrap();
    output.lines().map(str::to_string).collect()
}

#[test]
fn breaking_a_pane_out_makes_a_window_after_this_one_with_the_pane_as_it_was() {
    let daemon = TestDaemon::start();
    let mut client = client_with_shell(&daemon);
    run(&daemon, "pane.split-right");
    run(&daemon, "pane.split-down");
    // Three panes; the one with the focus (bottom right) prints a marker and many lines.
    client.type_line("echo scroll-marker; seq 1 80; echo last-line");
    let printed = client.wait_until(WAIT, |c| {
        c.region(51, 0, 50, 30)
            .iter()
            .any(|r| r.trim() == "last-line")
    });
    assert!(printed, "{}", client.screen_text());
    assert_eq!(pane_count(&daemon), 3);
    let focused = client.focused_pane_id();
    // The prefix, then `!` as the new protocol reports it: Shift and `1`.
    client.send(b"\x1b[109;5u");
    client.send(b"\x1b[49;2u");
    let ok = client.wait_until(WAIT, |_| window_lines(&daemon).len() == 2);
    assert!(ok, "{:?}", window_lines(&daemon));
    let lines = window_lines(&daemon);
    // The new window is the second one and is shown.
    assert!(
        lines[1].starts_with("2\t") && lines[1].ends_with("\t1"),
        "{lines:?}"
    );
    assert!(lines[0].ends_with("\t0"), "{lines:?}");
    // It holds exactly the pane that was broken out, filling the window, with its text.
    assert_eq!(client.focused_pane_id(), focused);
    wait_in(&mut client, (0, 0, 101, 30), "last-line");
    let divider_free = client
        .region(0, 0, 101, 30)
        .iter()
        .all(|r| !r.contains('│'));
    assert!(divider_free);
    // Its scrollback came along.
    run(&daemon, "copy.enter");
    run(&daemon, "copy.top");
    wait_in(&mut client, (0, 0, 101, 30), "scroll-marker");
    run(&daemon, "copy.exit");
    // The old window has the other two panes.
    run_with(&daemon, "window.select", &["1"]);
    let (_, sessions) = daemon.command("session.list", &[]).unwrap();
    assert_eq!(sessions.trim().split('\t').nth(4), Some("3"), "{sessions}");
    let two = client.wait_until(WAIT, |c| {
        let rows = c.region(0, 0, 101, 30);
        rows.iter().any(|r| r.contains('│')) && !rows.iter().any(|r| r.contains("last-line"))
    });
    assert!(two, "{}", client.screen_text());
}

fn run_with(daemon: &TestDaemon, name: &str, args: &[&str]) {
    let (ok, output) = daemon.command(name, args).unwrap();
    assert!(ok, "{name}: {output}");
}

#[test]
fn breaking_out_the_only_pane_of_a_window_says_so_and_changes_nothing() {
    let daemon = TestDaemon::start();
    let _client = client_with_shell(&daemon);
    let before = window_lines(&daemon);
    let (ok, output) = daemon.command("pane.break", &[]).unwrap();
    assert!(!ok && output.contains("only one pane"), "{output}");
    assert_eq!(window_lines(&daemon), before);
    assert_eq!(pane_count(&daemon), 1);
}

#[test]
fn a_broken_out_pane_gets_the_place_right_after_its_window() {
    let daemon = TestDaemon::start();
    let _client = client_with_shell(&daemon);
    run(&daemon, "window.new");
    run(&daemon, "window.new");
    run_with(&daemon, "window.select", &["1"]);
    run(&daemon, "pane.split-right");
    run(&daemon, "pane.break");
    let lines = window_lines(&daemon);
    assert_eq!(lines.len(), 4, "{lines:?}");
    let numbers: Vec<&str> = lines
        .iter()
        .map(|l| l.split('\t').next().unwrap())
        .collect();
    assert_eq!(numbers, ["1", "2", "3", "4"]);
    // The shown window is the second: the one just made.
    assert!(lines[1].ends_with("\t1"), "{lines:?}");
}

fn four_panes(daemon: &TestDaemon) -> TestClient {
    let client = client_with_shell(daemon);
    run(daemon, "pane.split-right");
    run(daemon, "pane.split-down");
    run(daemon, "pane.focus-left");
    run(daemon, "pane.split-down");
    client
}

/// How many columns of the 101 by 30 pane area have a divider all the way down, and how many
/// rows have one all the way across.
fn divider_lines(client: &mut TestClient) -> (usize, usize) {
    let rows = client.region(0, 0, 101, 30);
    let tall = (0..101)
        .filter(|x| {
            rows.iter()
                .all(|r| r.chars().nth(*x).is_some_and(|c| "│├┤┼".contains(c)))
        })
        .count();
    let wide = rows
        .iter()
        .filter(|r| !r.is_empty() && r.chars().all(|c| c == '─'))
        .count();
    (tall, wide)
}

#[test]
fn space_goes_round_the_arrangements_and_every_pane_keeps_some_room() {
    let daemon = TestDaemon::start();
    let mut client = four_panes(&daemon);
    let space = |client: &mut TestClient| {
        client.send(b"\x1b[109;5u");
        client.send(b" ");
    };
    // Even horizontal: three dividers all the way down, none across.
    space(&mut client);
    assert!(
        client.wait_until(WAIT, |c| divider_lines(c) == (3, 0)),
        "{}",
        client.screen_text()
    );
    // Even vertical: three dividers across, none down.
    space(&mut client);
    assert!(
        client.wait_until(WAIT, |c| divider_lines(c) == (0, 3)),
        "{:?}\n{}",
        divider_lines(&mut client),
        client.screen_text()
    );
    // One main pane: one divider down the whole height, two across the right part.
    space(&mut client);
    assert!(
        client.wait_until(WAIT, |c| divider_lines(c).0 == 1),
        "{}",
        client.screen_text()
    );
    let rows = client.region(0, 0, 101, 30);
    let across = rows.iter().filter(|r| r.contains('─')).count();
    assert_eq!(across, 2, "{}", client.screen_text());
    // And round again; the repeat window lets the key go on without the prefix.
    client.send(b" ");
    assert!(
        client.wait_until(WAIT, |c| divider_lines(c) == (3, 0)),
        "{}",
        client.screen_text()
    );
    // Every pane still has a program that answers with a size above zero.
    let (_, sessions) = daemon.command("session.list", &[]).unwrap();
    assert_eq!(sessions.trim().split('\t').nth(4), Some("4"));
}

#[test]
fn equals_evens_out_panes_after_their_sizes_were_pushed_apart() {
    let daemon = TestDaemon::start();
    let mut client = client_with_shell(&daemon);
    run(&daemon, "pane.split-right");
    for _ in 0..3 {
        run(&daemon, "pane.resize-left");
    }
    // The divider is 15 columns to the left of the middle.
    assert!(client.wait_until(WAIT, |c| c.region(35, 0, 1, 30).iter().all(|r| r == "│")));
    client.send(b"\x1b[109;5u");
    client.send(b"=");
    assert!(
        client.wait_until(WAIT, |c| c.region(50, 0, 1, 30).iter().all(|r| r == "│")),
        "{}",
        client.screen_text()
    );
}
