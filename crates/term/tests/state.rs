//! A terminal built again from what `save_state` wrote shows what the first one showed: every
//! row with its styles, links and soft-wrap marks, the cursor, the modes and the answers it
//! gives to a program that asks about its settings, and both screens of a full-screen program.

use arcade_term::{CursorShape, Event, Line, Term, TermConfig};

const SCROLLBACK: usize = 50_000;

fn new(cols: u16, rows: u16) -> Term {
    let mut config = TermConfig::new(cols, rows);
    config.scrollback_lines = SCROLLBACK;
    Term::new(config).unwrap()
}

fn lines(t: &Term) -> Vec<Line> {
    (0..t.total_rows()).filter_map(|y| t.line(y)).collect()
}

/// A terminal with every kind of content and setting the daemon's panes have.
fn rich(history: usize) -> Term {
    let mut t = new(80, 24);
    t.feed(b"\x1b]4;1;rgb:12/34/56\x07\x1b]10;rgb:aa/bb/cc\x07");
    t.feed(b"\x1b[?2004h\x1b[?1000h\x1b[?1006h\x1b[?1004h\x1b[?1h\x1b[>5u");
    t.feed(b"\x1b[3g\x1b[1;20H\x1bH\x1b[1;40H\x1bH");
    for i in 0..history {
        let line = match i % 6 {
            0 => format!("\x1b[1;31mred bold {i}\x1b[0m plain\r\n"),
            1 => format!("\x1b[38;2;10;200;30mtruecolor {i}\x1b[0m \x1b[4:3mcurly\x1b[0m\r\n"),
            2 => format!(
                "wide: \u{6f22}\u{5b57} emoji \u{1f468}\u{200d}\u{1f469}\u{200d}\u{1f467} {i}\r\n"
            ),
            3 => format!(
                "\x1b]8;id=x{i};https://example.com/{i}\x1b\\link {i}\x1b]8;;\x1b\\ tail\r\n"
            ),
            4 => "x".repeat(90) + "\r\n",
            _ => format!("\x1b[7minverse {i}\x1b[0m \x1b(0lqqk\x1b(B {i}\r\n"),
        };
        t.feed(line.as_bytes());
    }
    t.feed(b"\x1b[5;20r\x1b[7;3H\x1b[48;5;99m\x1b[1mhere");
    t
}

/// What a terminal says back to a program that asks about its settings.
fn answers(t: &mut Term) -> Vec<String> {
    let mut q = String::new();
    for m in [
        1, 6, 7, 12, 25, 45, 66, 69, 1000, 1002, 1003, 1004, 1005, 1006, 1015, 1016, 1036, 2004,
        2026, 2027, 2031,
    ] {
        q += &format!("\x1b[?{m}$p");
    }
    // Scroll region, rendition, kitty flags, the palette and the default colours.
    q += "\x1bP$qr\x1b\\\x1bP$qm\x1b\\\x1b[?u";
    for i in 0..16 {
        q += &format!("\x1b]4;{i};?\x07");
    }
    q += "\x1b]10;?\x07\x1b]11;?\x07\x1b]12;?\x07";
    t.drain_events();
    t.feed(q.as_bytes());
    t.drain_events()
        .into_iter()
        .filter_map(|e| match e {
            Event::Reply(b) => Some(String::from_utf8_lossy(&b).into_owned()),
            _ => None,
        })
        .collect()
}

fn rebuilt(t: &mut Term, cursor: Option<(CursorShape, bool)>) -> Term {
    let state = t.save_state().unwrap();
    let mut b = new(80, 24);
    b.load_state(&state, cursor);
    b
}

#[test]
fn a_rebuilt_terminal_shows_every_row_link_and_setting_of_the_first() {
    for history in [0, 300] {
        let mut a = rich(history);
        let mut b = rebuilt(&mut a, None);
        assert_eq!(a.total_rows(), b.total_rows(), "history {history}");
        // Cells, styles, wide characters and the soft-wrap marks of every row, scrollback too.
        assert_eq!(lines(&a), lines(&b), "history {history}");
        for y in 0..a.total_rows() {
            assert_eq!(a.row_links(y), b.row_links(y), "links of row {y}");
        }
        assert_eq!(a.cursor_position(), b.cursor_position());
        assert_eq!(a.key_modes(), b.key_modes());
        let (aa, ab) = (answers(&mut a), answers(&mut b));
        let differ: Vec<_> = aa.iter().filter(|x| !ab.contains(x)).collect();
        assert!(
            differ.is_empty() && aa.len() == ab.len(),
            "history {history}: {differ:?} ({} vs {} answers)",
            aa.len(),
            ab.len()
        );
        // They go on the same way: tab stops, scroll region and the cursor position.
        let more = b"\r\t\t\tA\t\tB\x1b[999;1H\n\n\nZ";
        a.feed(more);
        b.feed(more);
        assert_eq!(lines(&a), lines(&b));
    }
}

#[test]
fn a_full_screen_program_comes_back_with_both_screens() {
    let mut a = rich(300);
    let primary_before = lines(&a);
    // Cells cleared with a colour of their own below the text are the one thing that does not
    // come back, so the program here clears with the default one.
    a.feed(b"\x1b[0m\x1b[?1049h\x1b[2J\x1b[3;3Hvim here\x1b[44mblue\x1b[0m\x1b[5;1H");
    assert!(a.on_alternate_screen());
    let alternate_before = lines(&a);
    let mut b = rebuilt(&mut a, None);
    // Reading the primary screen did not disturb the old terminal.
    assert!(a.on_alternate_screen());
    assert_eq!(lines(&a), alternate_before);
    assert!(b.on_alternate_screen());
    let lb = lines(&b);
    for (y, (p, q)) in alternate_before.iter().zip(&lb).enumerate() {
        if p != q {
            let x = (0..80).find(|&x| p.cells[x] != q.cells[x]);
            panic!(
                "alt row {y}: col {x:?}: {:?} vs {:?}",
                x.map(|x| &p.cells[x].style),
                x.map(|x| &q.cells[x].style)
            );
        }
    }
    // When the program leaves, the shell's screen and its history are there.
    a.feed(b"\x1b[?1049l");
    b.feed(b"\x1b[?1049l");
    assert!(!b.on_alternate_screen());
    let (la, lb) = (lines(&a), lines(&b));
    let first = (0..la.len()).find(|&y| la[y] != lb[y]);
    if let Some(y) = first {
        let x = (0..80).find(|&x| la[y].cells[x] != lb[y].cells[x]);
        panic!(
            "row {y} of {}: col {x:?}: {:?} vs {:?}; continues {} {}",
            la.len(),
            x.map(|x| &la[y].cells[x]),
            x.map(|x| &lb[y].cells[x]),
            la[y].continues,
            lb[y].continues
        );
    }
    assert_eq!(lb.len(), primary_before.len());
}

#[test]
fn the_shape_of_the_cursor_is_set_again() {
    let mut a = new(80, 24);
    a.feed(b"\x1b[5 q");
    let mut b = rebuilt(&mut a, Some((CursorShape::Bar, true)));
    let frame = b.snapshot(true).unwrap();
    assert_eq!(frame.cursor.shape, CursorShape::Bar);
    assert!(frame.cursor.blinking);
}
