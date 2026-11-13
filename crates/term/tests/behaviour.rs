//! What `arcade-term` adds on top of the library: the policy settings from the milestone 1 plan
//! (section 1.5) and the way it reports what changed on screen.

use std::time::{Duration, Instant};

use arcade_term::{
    CLIPBOARD_WRITE_LIMIT, Cell, CellWidth, Color, CompressProgress, Cursor, CursorShape, Damage,
    Event, Mode, Term, TermConfig,
};
use arcade_workload::{Rng, scrollback_fill};

mod support;
use support::base64_encode;

fn term(cols: u16, rows: u16) -> Term {
    Term::new(TermConfig::new(cols, rows)).expect("create terminal")
}

fn clipboard_sequence(payload: &[u8]) -> Vec<u8> {
    [
        b"\x1b]52;c;".as_slice(),
        base64_encode(payload).as_bytes(),
        b"\x07OK",
    ]
    .concat()
}

fn clipboard_writes(events: &[Event]) -> Vec<usize> {
    events
        .iter()
        .filter_map(|e| match e {
            Event::ClipboardWrite(data) => Some(data.len()),
            _ => None,
        })
        .collect()
}

// ---- Policy settings ----

#[test]
fn scrollback_keeps_all_fifty_thousand_lines() {
    let mut t = term(120, 40);
    // 70,000 numbered lines: more than the limit, so the oldest must be the ones dropped.
    let mut data = Vec::new();
    for n in 0..70_000u32 {
        data.extend_from_slice(format!("line {n}\r\n").as_bytes());
    }
    for chunk in data.chunks(65_536) {
        t.feed(chunk);
    }
    // The library prunes whole pages, so it may keep a little more than the limit, never less.
    let kept = t.scrollback_lines();
    assert!(kept >= 50_000, "only {kept} lines kept");
    assert!(
        kept <= 50_000 + 1_000,
        "{kept} lines kept, far above the limit"
    );
    // The newest line is still on screen; the screen has 40 rows, the cursor sits below the last.
    assert_eq!(t.row_text(38), "line 69999");
}

#[test]
fn scrollback_below_the_limit_loses_nothing() {
    let mut t = term(80, 24);
    t.feed(&scrollback_fill(30_000, 80, 7));
    // Every line the fill wrote beyond the 24 on screen is in the scrollback.
    assert!(t.scrollback_lines() >= 29_900, "{}", t.scrollback_lines());
}

#[test]
fn configured_scrollback_limit_applies() {
    let mut config = TermConfig::new(80, 24);
    config.scrollback_lines = 100;
    let mut t = Term::new(config).unwrap();
    t.feed(&scrollback_fill(5_000, 80, 3));
    // A page holds several hundred rows, so the surplus over a tiny limit is large but bounded.
    let kept = t.scrollback_lines();
    assert!(kept >= 100, "only {kept} lines kept");
    assert!(kept <= 1_000, "{kept} lines kept");
}

#[test]
fn scrollback_floor_holds_at_every_width() {
    for cols in [20u16, 80, 120, 200, 400] {
        let mut config = TermConfig::new(cols, 24);
        config.scrollback_lines = 50_000;
        let mut t = Term::new(config).unwrap();
        let mut data = Vec::new();
        for n in 0..60_000u32 {
            data.extend_from_slice(format!("l{n}\r\n").as_bytes());
        }
        for chunk in data.chunks(65_536) {
            t.feed(chunk);
        }
        let kept = t.scrollback_lines();
        assert!(kept >= 50_000, "{cols} columns: only {kept} lines kept");
    }
}

#[test]
fn scrollback_floor_holds_after_a_resize() {
    let mut t = term(200, 24);
    t.resize(30, 24).unwrap();
    let mut data = Vec::new();
    for n in 0..60_000u32 {
        data.extend_from_slice(format!("l{n}\r\n").as_bytes());
    }
    for chunk in data.chunks(65_536) {
        t.feed(chunk);
    }
    assert!(t.scrollback_lines() >= 50_000, "{}", t.scrollback_lines());
}

#[test]
fn clipboard_write_at_the_limit_is_delivered() {
    let mut t = term(80, 24);
    let payload = vec![b'q'; CLIPBOARD_WRITE_LIMIT];
    t.feed(&clipboard_sequence(&payload));
    assert_eq!(
        clipboard_writes(&t.drain_events()),
        vec![CLIPBOARD_WRITE_LIMIT]
    );
    assert_eq!(t.row_text(0), "OK");
}

#[test]
fn clipboard_write_over_the_limit_is_dropped_whole() {
    let mut t = term(80, 24);
    t.feed(&clipboard_sequence(&vec![b'z'; 2 * 1024 * 1024]));
    assert_eq!(clipboard_writes(&t.drain_events()), Vec::<usize>::new());
    // Nothing of it shows on the pane.
    assert_eq!(t.row_text(0), "OK");
    assert_eq!(t.row_text(1), "");
    // The pane keeps working afterwards.
    t.feed(&clipboard_sequence(b"hello"));
    assert_eq!(clipboard_writes(&t.drain_events()), vec![5]);
}

#[test]
fn clipboard_read_request_is_never_answered() {
    let mut t = term(80, 24);
    t.feed(b"\x1b]52;c;?\x07\x1b]52;p;?\x07");
    let replies: Vec<_> = t
        .drain_events()
        .into_iter()
        .filter(|e| matches!(e, Event::Reply(_)))
        .collect();
    assert!(replies.is_empty(), "{replies:?}");
}

#[test]
fn unfinished_string_is_cancelled_after_the_timeout_and_shows_nothing() {
    let mut config = TermConfig::new(80, 24);
    config.string_timeout = Duration::from_millis(50);
    let mut t = Term::new(config).unwrap();
    assert!(t.stuck_deadline().is_none());

    t.feed(b"\x1b]0;a title that never ends\r\nand keeps going");
    let deadline = t.stuck_deadline().expect("stuck inside a string");
    // Before the deadline nothing happens.
    assert!(!t.expire_stuck(deadline - Duration::from_millis(1)));
    assert!(t.stuck_deadline().is_some());
    // At the deadline the string is cancelled.
    assert!(t.expire_stuck(deadline));
    assert!(t.stuck_deadline().is_none());

    t.feed(b"OK");
    assert_eq!(t.row_text(0), "OK");
    assert_eq!(t.row_text(1), "");
    assert_eq!(t.cursor_position(), (2, 0));
    // No title was set by the cancelled string.
    assert!(
        !t.drain_events()
            .iter()
            .any(|e| matches!(e, Event::Title(_)))
    );
}

#[test]
fn finished_sequences_leave_no_deadline() {
    let mut t = term(80, 24);
    t.feed(b"plain text\r\n\x1b[31mred\x1b[0m \x1b]0;title\x07");
    assert!(t.stuck_deadline().is_none());
    // A sequence cut in the middle of a feed does set one, and finishing it clears it.
    t.feed(b"\x1b[3");
    assert!(t.stuck_deadline().is_some());
    t.feed(b"1mX");
    assert!(t.stuck_deadline().is_none());
}

#[test]
fn each_feed_that_stays_inside_a_string_restarts_the_timeout() {
    let mut config = TermConfig::new(80, 24);
    config.string_timeout = Duration::from_secs(5);
    let mut t = Term::new(config).unwrap();
    t.feed(b"\x1b]0;abc");
    let first = t.stuck_deadline().unwrap();
    std::thread::sleep(Duration::from_millis(5));
    t.feed(b"def");
    assert!(t.stuck_deadline().unwrap() > first);
}

#[test]
fn sixty_four_megabytes_of_random_bytes_do_not_stop_the_terminal() {
    let mut t = term(120, 40);
    let mut rng = Rng::new(99);
    let mut block = vec![0u8; 64 * 1024];
    for _ in 0..1024 {
        rng.fill(&mut block);
        t.feed(&block);
        let _ = t.drain_events();
    }
    // Still usable: reset the parser state and print something.
    t.feed(b"\x18\x1b[2J\x1b[Hstill alive");
    assert_eq!(t.row_text(0), "still alive");
    assert!(t.snapshot(true).is_ok());
}

#[test]
fn compressing_idle_scrollback_shrinks_memory_and_keeps_the_text() {
    let mut t = term(120, 40);
    let data = scrollback_fill(50_000, 120, 5);
    for chunk in data.chunks(65_536) {
        t.feed(chunk);
    }
    let before = t.resident_bytes();
    let mut steps = 0;
    while t.compress_step() == CompressProgress::Pending {
        steps += 1;
        assert!(steps < 100_000, "compression never finished");
    }
    let after = t.resident_bytes();
    assert!(
        after * 2 < before,
        "memory went from {before} to {after} bytes; expected less than half"
    );
    // The memory requirement for one pane with a full scrollback is 30 MB.
    assert!(after < 30 * 1024 * 1024, "{after} bytes after compressing");
    assert!(t.scrollback_lines() >= 49_000);
}

#[test]
fn memory_of_an_idle_pane_is_small() {
    let mut t = term(120, 40);
    t.feed(b"$ ");
    assert!(
        t.resident_bytes() < 2 * 1024 * 1024,
        "{}",
        t.resident_bytes()
    );
}

#[test]
fn replies_go_out_as_events() {
    let mut t = term(80, 24);
    t.feed(b"\x1b[3;5H\x1b[6n");
    assert!(
        t.drain_events()
            .contains(&Event::Reply(b"\x1b[3;5R".to_vec()))
    );
}

#[test]
fn modes_are_reported() {
    let mut t = term(80, 24);
    assert_eq!(t.mode(Mode::GraphemeCluster), Some(true));
    assert_eq!(t.mode(Mode::BracketedPaste), Some(false));
    t.feed(b"\x1b[?2004h\x1b[?1h");
    assert_eq!(t.mode(Mode::BracketedPaste), Some(true));
    assert_eq!(t.mode(Mode::AppCursorKeys), Some(true));
    // A full reset keeps grapheme clustering on.
    t.feed(b"\x1bc");
    assert_eq!(t.mode(Mode::GraphemeCluster), Some(true));
    assert_eq!(t.mode(Mode::BracketedPaste), Some(false));
}

#[test]
fn resize_changes_the_screen_and_the_reported_size() {
    let mut t = term(80, 24);
    t.feed(b"hello");
    t.resize(100, 30).unwrap();
    assert_eq!((t.cols(), t.rows()), (100, 30));
    assert_eq!(t.row_text(0), "hello");
    let frame = t.snapshot(false).unwrap();
    assert_eq!((frame.cols, frame.rows_total), (100, 30));
    assert_eq!(frame.damage, Damage::Full);
}

// ---- Frames ----

fn text_of(cells: &[Cell]) -> String {
    cells
        .iter()
        .map(|c| match c.width {
            CellWidth::SpacerTail => String::new(),
            _ if c.text.is_empty() => " ".to_string(),
            _ => c.text.clone(),
        })
        .collect::<String>()
        .trim_end()
        .to_string()
}

#[test]
fn first_snapshot_is_full_then_clean_when_nothing_changed() {
    let mut t = term(20, 5);
    t.feed(b"abc");
    let first = t.snapshot(false).unwrap();
    assert_eq!(first.damage, Damage::Full);
    assert_eq!(first.rows.len(), 5);
    assert_eq!(text_of(&first.rows[0].cells), "abc");

    let second = t.snapshot(false).unwrap();
    assert_eq!(second.damage, Damage::Clean);
    assert!(second.rows.is_empty());
}

#[test]
fn only_changed_rows_are_reported() {
    let mut t = term(20, 5);
    t.feed(b"one\r\ntwo\r\nthree");
    t.snapshot(false).unwrap();

    t.feed(b"\x1b[2;1HTWO");
    let frame = t.snapshot(false).unwrap();
    assert_eq!(frame.damage, Damage::Partial);
    // The row that changed is reported. The library also reports the row the cursor left, so
    // one more row may come along, but never the others.
    let changed = frame.rows.iter().find(|r| r.y == 1).expect("changed row");
    assert_eq!(text_of(&changed.cells), "TWO");
    assert!(
        frame.rows.len() <= 2,
        "{:?}",
        frame.rows.iter().map(|r| r.y).collect::<Vec<_>>()
    );
    assert!(frame.rows.iter().all(|r| r.y <= 2));
}

#[test]
fn force_full_returns_every_row() {
    let mut t = term(20, 5);
    t.feed(b"a\r\nb");
    t.snapshot(false).unwrap();
    let frame = t.snapshot(true).unwrap();
    assert_eq!(frame.damage, Damage::Full);
    assert_eq!(frame.rows.len(), 5);
    assert_eq!(
        frame.rows.iter().map(|r| r.y).collect::<Vec<_>>(),
        vec![0, 1, 2, 3, 4]
    );
}

#[test]
fn scrolling_marks_the_moved_rows() {
    let mut t = term(10, 3);
    t.feed(b"1\r\n2\r\n3");
    t.snapshot(false).unwrap();
    t.feed(b"\r\n4");
    let frame = t.snapshot(false).unwrap();
    // After the scroll the screen reads 2, 3, 4.
    let screen = t.screen().unwrap();
    assert_eq!(
        screen.iter().map(|r| text_of(r)).collect::<Vec<_>>(),
        vec!["2", "3", "4"]
    );
    assert_ne!(frame.damage, Damage::Clean);
    for update in &frame.rows {
        assert!(update.y < 3);
    }
}

#[test]
fn cells_carry_color_and_attributes() {
    let mut t = term(20, 3);
    t.feed(b"\x1b[1;31mR\x1b[0;38;5;200mP\x1b[0;38;2;1;2;3;48;2;9;8;7mT\x1b[0;4:3;7mU");
    let frame = t.snapshot(false).unwrap();
    let cells = &frame.rows[0].cells;
    assert!(cells[0].style.bold);
    assert_eq!(cells[0].style.fg, Color::Indexed(1));
    assert_eq!(cells[1].style.fg, Color::Indexed(200));
    assert_eq!(cells[2].style.fg, Color::Rgb(1, 2, 3));
    assert_eq!(cells[2].style.bg, Color::Rgb(9, 8, 7));
    assert_eq!(cells[3].style.underline, arcade_term::Underline::Curly);
    assert!(cells[3].style.inverse);
}

#[test]
fn background_color_of_blank_cells_is_kept() {
    let mut t = term(10, 2);
    // Erase the line with a palette background, then a true color one on the second row.
    t.feed(b"\x1b[44m\x1b[2K\x1b[2;1H\x1b[48;2;10;20;30m\x1b[K");
    let frame = t.snapshot(false).unwrap();
    assert!(
        frame.rows[0]
            .cells
            .iter()
            .all(|c| c.style.bg == Color::Indexed(4))
    );
    assert!(
        frame.rows[1]
            .cells
            .iter()
            .all(|c| c.style.bg == Color::Rgb(10, 20, 30))
    );
}

#[test]
fn wide_characters_have_a_spacer_cell() {
    let mut t = term(10, 2);
    t.feed("汉a".as_bytes());
    let frame = t.snapshot(false).unwrap();
    let cells = &frame.rows[0].cells;
    assert_eq!(cells[0].text, "汉");
    assert_eq!(cells[0].width, CellWidth::Wide);
    assert_eq!(cells[1].width, CellWidth::SpacerTail);
    assert_eq!(cells[2].text, "a");
    assert_eq!(cells.len(), 10);
}

#[test]
fn joined_emoji_is_one_wide_cell() {
    let mut t = term(10, 2);
    t.feed("👨\u{200D}👩\u{200D}👧\u{200D}👦x".as_bytes());
    let frame = t.snapshot(false).unwrap();
    let cells = &frame.rows[0].cells;
    assert_eq!(cells[0].text, "👨\u{200D}👩\u{200D}👧\u{200D}👦");
    assert_eq!(cells[0].width, CellWidth::Wide);
    assert_eq!(cells[2].text, "x");
}

#[test]
fn cursor_position_visibility_and_shape() {
    let mut t = term(20, 5);
    t.feed(b"\x1b[3;7H");
    let cursor = t.snapshot(false).unwrap().cursor;
    assert_eq!(
        cursor,
        Cursor {
            x: 6,
            y: 2,
            visible: true,
            shape: CursorShape::Block,
            blinking: cursor.blinking,
        }
    );

    t.feed(b"\x1b[?25l");
    assert!(!t.snapshot(false).unwrap().cursor.visible);
    t.feed(b"\x1b[?25h\x1b[6 q");
    let cursor = t.snapshot(false).unwrap().cursor;
    assert!(cursor.visible);
    assert_eq!(cursor.shape, CursorShape::Bar);
    t.feed(b"\x1b[4 q");
    assert_eq!(
        t.snapshot(false).unwrap().cursor.shape,
        CursorShape::Underline
    );
}

#[test]
fn alternate_screen_is_what_snapshots_show() {
    let mut t = term(20, 3);
    t.feed(b"main text");
    t.snapshot(false).unwrap();
    t.feed(b"\x1b[?1049h\x1b[Halt text");
    let frame = t.snapshot(false).unwrap();
    assert_eq!(frame.damage, Damage::Full);
    assert_eq!(text_of(&frame.rows[0].cells), "alt text");
    t.feed(b"\x1b[?1049l");
    let frame = t.snapshot(false).unwrap();
    assert_eq!(text_of(&frame.rows[0].cells), "main text");
}

#[test]
fn snapshot_after_random_bytes_does_not_fail() {
    let mut t = term(60, 20);
    let mut rng = Rng::new(5);
    let mut block = vec![0u8; 4096];
    for _ in 0..200 {
        rng.fill(&mut block);
        t.feed(&block);
        t.snapshot(false).expect("snapshot");
    }
}

#[test]
fn expire_stuck_uses_the_real_clock_too() {
    let mut config = TermConfig::new(10, 2);
    config.string_timeout = Duration::from_millis(1);
    let mut t = Term::new(config).unwrap();
    t.feed(b"\x1bP+q");
    let start = Instant::now();
    // Condition-based wait with a limit rather than a fixed sleep.
    while !t.expire_stuck(Instant::now()) {
        assert!(start.elapsed() < Duration::from_secs(5), "never expired");
        std::thread::yield_now();
    }
}
