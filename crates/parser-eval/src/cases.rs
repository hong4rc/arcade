//! Correctness cases taken from section 4 of the core spec and section 7 of the copy mode and
//! clipboard spec. Every case is fed whole and in several split patterns; a candidate passes a
//! case only if it passes under every pattern.

use crate::base64_encode;
use crate::candidate::{Candidate, Color, Event, Mode, Underline};

pub const COLS: u16 = 80;
pub const ROWS: u16 = 24;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Group {
    /// Operating system command strings and other string sequences must never show as text.
    Strings,
    Unicode,
    /// Cursor, erase, scroll region, alternate screen, attributes, modes.
    Screen,
    /// Answers to the queries programs send at startup.
    Replies,
}

impl Group {
    pub fn name(self) -> &'static str {
        match self {
            Group::Strings => "strings",
            Group::Unicode => "unicode",
            Group::Screen => "screen",
            Group::Replies => "replies",
        }
    }
}

type Check = Box<dyn Fn(&mut dyn Candidate) -> Result<(), String>>;

pub struct Case {
    pub id: &'static str,
    pub group: Group,
    pub input: Vec<u8>,
    pub check: Check,
}

fn rows_of(c: &dyn Candidate) -> Vec<String> {
    (0..ROWS).map(|row| c.row_text(row)).collect()
}

/// The screen shows exactly these rows from the top and nothing else.
fn screen_is(c: &dyn Candidate, expected: &[&str]) -> Result<(), String> {
    let rows = rows_of(c);
    for (index, row) in rows.iter().enumerate() {
        let want = expected.get(index).copied().unwrap_or("");
        if row != want {
            let shown: String = row.chars().take(60).collect();
            return Err(format!("row {index}: expected {want:?}, got {shown:?}"));
        }
    }
    Ok(())
}

fn replies(events: &[Event]) -> Vec<u8> {
    let mut all = Vec::new();
    for event in events {
        if let Event::Reply(bytes) = event {
            all.extend_from_slice(bytes);
        }
    }
    all
}

fn contains(haystack: &[u8], needle: &[u8]) -> bool {
    haystack
        .windows(needle.len())
        .any(|window| window == needle)
}

fn show(bytes: &[u8]) -> String {
    String::from_utf8_lossy(bytes).escape_debug().to_string()
}

fn case(
    id: &'static str,
    group: Group,
    input: impl Into<Vec<u8>>,
    check: impl Fn(&mut dyn Candidate) -> Result<(), String> + 'static,
) -> Case {
    Case {
        id,
        group,
        input: input.into(),
        check: Box::new(check),
    }
}

/// A string sequence followed by "OK": only "OK" may be visible.
fn swallowed(id: &'static str, sequence: impl Into<Vec<u8>>) -> Case {
    let mut input = sequence.into();
    input.extend_from_slice(b"OK");
    case(id, Group::Strings, input, |c| {
        screen_is(c, &["OK"])?;
        if c.cursor() != (2, 0) {
            return Err(format!("cursor at {:?}, expected (2, 0)", c.cursor()));
        }
        Ok(())
    })
}

fn width(id: &'static str, text: &'static str, columns: u16) -> Case {
    // Grapheme clustering (mode 2027) is turned on first, as the daemon would do by default.
    let input = format!("\x1b[?2027h{text}");
    case(id, Group::Unicode, input, move |c| {
        let (col, row) = c.cursor();
        if (col, row) != (columns, 0) {
            return Err(format!(
                "cursor column {col} (row {row}), expected {columns}"
            ));
        }
        let shown = c.row_text(0);
        if shown != text {
            return Err(format!("row text {shown:?} differs from input {text:?}"));
        }
        Ok(())
    })
}

fn reply(id: &'static str, input: &'static [u8], expected: &'static [u8]) -> Case {
    case(id, Group::Replies, input, move |c| {
        let got = replies(&c.drain_events());
        if contains(&got, expected) {
            Ok(())
        } else {
            Err(format!(
                "expected a reply containing {:?}, got {:?}",
                show(expected),
                show(&got)
            ))
        }
    })
}

pub fn all() -> Vec<Case> {
    let clipboard_text: Vec<u8> = (0..75_000u32).map(|i| b'a' + (i % 26) as u8).collect();
    let clipboard_b64 = base64_encode(&clipboard_text);
    assert_eq!(clipboard_b64.len(), 100_000);
    let big_text: Vec<u8> = vec![b'z'; 2 * 1024 * 1024];
    let big_b64 = base64_encode(&big_text);

    let mut cases = vec![
        // ---- Strings: nothing of the sequence may ever be drawn. ----
        case("osc-title-bel", Group::Strings, b"\x1b]0;hello\x07OK", |c| {
            screen_is(c, &["OK"])?;
            let events = c.drain_events();
            if events.contains(&Event::Title("hello".into())) {
                Ok(())
            } else {
                Err(format!("no title event, got {events:?}"))
            }
        }),
        case("osc-title-st", Group::Strings, b"\x1b]2;hello\x1b\\OK", |c| {
            screen_is(c, &["OK"])?;
            let events = c.drain_events();
            if events.contains(&Event::Title("hello".into())) {
                Ok(())
            } else {
                Err(format!("no title event, got {events:?}"))
            }
        }),
        swallowed("osc-unknown-code", b"\x1b]9999;some;payload\x07"),
        case(
            "osc52-write-100kb",
            Group::Strings,
            [b"\x1b]52;c;".as_slice(), clipboard_b64.as_bytes(), b"\x07OK"].concat(),
            move |c| {
                screen_is(c, &["OK"])?;
                let events = c.drain_events();
                let written: Vec<&Vec<u8>> = events
                    .iter()
                    .filter_map(|e| if let Event::ClipboardWrite(d) = e { Some(d) } else { None })
                    .collect();
                match written.as_slice() {
                    [data] if **data == clipboard_text => Ok(()),
                    [data] => Err(format!("clipboard got {} bytes, expected 75000", data.len())),
                    other => Err(format!("{} clipboard writes, expected exactly 1", other.len())),
                }
            },
        ),
        case("osc52-read-refused", Group::Strings, b"\x1b]52;c;?\x07OK", |c| {
            screen_is(c, &["OK"])?;
            let got = replies(&c.drain_events());
            if got.is_empty() { Ok(()) } else { Err(format!("replied {:?}", show(&got))) }
        }),
        case("osc52-invalid-base64", Group::Strings, b"\x1b]52;c;!!!not*base64!!!\x07OK", |c| {
            screen_is(c, &["OK"])?;
            let events = c.drain_events();
            let wrote =
                events.iter().any(|e| matches!(e, Event::ClipboardWrite(d) if !d.is_empty()));
            if wrote { Err(format!("clipboard written from invalid payload: {events:?}")) } else { Ok(()) }
        }),
        case(
            "osc52-oversized-2mb",
            Group::Strings,
            [b"\x1b]52;c;".as_slice(), big_b64.as_bytes(), b"\x07OK"].concat(),
            |c| {
                screen_is(c, &["OK"])?;
                // The whole payload or nothing; a partial write is the failure.
                for event in c.drain_events() {
                    if let Event::ClipboardWrite(data) = event
                        && !data.is_empty()
                        && data.len() != 2 * 1024 * 1024
                    {
                        return Err(format!("partial clipboard write of {} bytes", data.len()));
                    }
                }
                Ok(())
            },
        ),
        swallowed("osc-oversized-8mb", {
            let mut seq = b"\x1b]0;".to_vec();
            seq.resize(seq.len() + 8 * 1024 * 1024, b'x');
            seq.push(0x07);
            seq
        }),
        swallowed("osc-unterminated-then-new-escape", b"\x1b]0;abc def\x1b[31m"),
        case(
            "osc-unterminated-shows-nothing",
            Group::Strings,
            b"\x1b]0;this string never ends\r\nsecond line of it\r\n",
            |c| screen_is(c, &[]),
        ),
        // On a timeout the daemon sends CAN (0x18), which cancels any sequence in progress.
        swallowed("osc-unterminated-cancelled", b"\x1b]0;this string never ends\r\nmore\x18"),
        case("osc9-notification", Group::Strings, b"\x1b]9;build done\x07OK", |c| {
            screen_is(c, &["OK"])?;
            let events = c.drain_events();
            let found = events
                .iter()
                .any(|e| matches!(e, Event::Notification { body, .. } if body == "build done"));
            if found { Ok(()) } else { Err(format!("no notification event, got {events:?}")) }
        }),
        case("osc777-notification", Group::Strings, b"\x1b]777;notify;Title;Body\x07OK", |c| {
            screen_is(c, &["OK"])?;
            let events = c.drain_events();
            let want = Event::Notification { title: "Title".into(), body: "Body".into() };
            if events.contains(&want) {
                Ok(())
            } else {
                Err(format!("no notification event, got {events:?}"))
            }
        }),
        case("osc9-progress-is-not-notification", Group::Strings, b"\x1b]9;4;1;50\x07OK", |c| {
            screen_is(c, &["OK"])?;
            let events = c.drain_events();
            if events.iter().any(|e| matches!(e, Event::Notification { .. })) {
                Err(format!("progress report became a notification: {events:?}"))
            } else {
                Ok(())
            }
        }),
        case("osc99-swallowed", Group::Strings, b"\x1b]99;i=1:d=0;Hello\x1b\\OK", |c| {
            screen_is(c, &["OK"])?;
            let events = c.drain_events();
            if events.iter().any(|e| matches!(e, Event::Notification { .. })) {
                Err(format!("unexpected notification: {events:?}"))
            } else {
                Ok(())
            }
        }),
        case(
            "osc8-hyperlink",
            Group::Strings,
            b"\x1b]8;;https://example.com\x1b\\link\x1b]8;;\x1b\\ OK",
            |c| {
                screen_is(c, &["link OK"])?;
                match c.style_at(0, 0).hyperlink.as_deref() {
                    Some("https://example.com") => Ok(()),
                    Some(other) => Err(format!("hyperlink is {other:?}")),
                    None => Err("library does not keep hyperlinks".into()),
                }
            },
        ),
        swallowed("dcs-sixel", b"\x1bPq#0;2;0;0;0#0~~@@vv@@~~$\x1b\\"),
        swallowed("dcs-termcap-query", b"\x1bP+q544e\x1b\\"),
        swallowed("apc-image-protocol", b"\x1b_Gf=24,s=1,v=1;AAAA\x1b\\"),
        swallowed("pm-and-sos-strings", b"\x1b^private message\x1b\\\x1bXstart of string\x1b\\"),
        swallowed(
            "csi-unknown",
            b"\x1b[?9999h\x1b[98765z\x1b[1;2;3;4;5;6;7;8;9;10;11;12;13;14;15;16;17;18;19;20;21;22;23;24;25;26;27;28;29;30;31;32;33;34;35y\x1b[<1;2;3T"
                .as_slice(),
        ),
        swallowed("cursor-style-and-color", b"\x1b[5 q\x1b]12;#ff0000\x07"),
        // ---- Unicode: the cursor must land on the right column. ----
        width("wide-cjk", "汉字", 4),
        width("vietnamese-combining", "Tie\u{302}\u{301}ng Vie\u{323}\u{302}t", 10),
        width("emoji-single", "😀", 2),
        // Same joined emoji without turning grapheme clustering on first: shows the default.
        case(
            "emoji-joined-family-default-mode",
            Group::Unicode,
            "👨\u{200D}👩\u{200D}👧\u{200D}👦",
            |c| {
                if c.cursor() == (2, 0) { Ok(()) } else { Err(format!("cursor column {}", c.cursor().0)) }
            },
        ),
        width("emoji-joined-family", "👨\u{200D}👩\u{200D}👧\u{200D}👦", 2),
        width("emoji-skin-tone", "👍\u{1F3FD}", 2),
        width("emoji-flag", "🇻🇳", 2),
        width("emoji-variation-selector", "❤\u{FE0F}", 2),
        width(
            "mixed-line",
            "Tie\u{302}\u{301}ng Vie\u{323}\u{302}t 汉字 👨\u{200D}👩\u{200D}👧\u{200D}👦",
            18,
        ),
        // ---- Screen ----
        case("cursor-addressing", Group::Screen, b"\x1b[5;10HX\x1b[2;3HY", |c| {
            screen_is(c, &["", "  Y", "", "", "         X"])?;
            if c.cursor() == (3, 1) { Ok(()) } else { Err(format!("cursor {:?}", c.cursor())) }
        }),
        case("erase-line-and-screen", Group::Screen, b"junk\r\nmore\x1b[2J\x1b[HABCDEF\x1b[3D\x1b[K", |c| {
            screen_is(c, &["ABC"])
        }),
        case(
            "scroll-region-delete-line",
            Group::Screen,
            b"1\r\n2\r\n3\r\n4\r\n5\x1b[2;4r\x1b[2;1H\x1b[M",
            |c| screen_is(c, &["1", "3", "4", "", "5"]),
        ),
        case(
            "scroll-region-insert-line",
            Group::Screen,
            b"1\r\n2\r\n3\r\n4\r\n5\x1b[2;4r\x1b[2;1H\x1b[L",
            |c| screen_is(c, &["1", "", "2", "3", "5"]),
        ),
        case(
            "scroll-region-line-feed",
            Group::Screen,
            b"1\r\n2\r\n3\r\n4\r\n5\x1b[2;4r\x1b[4;1H\n",
            |c| screen_is(c, &["1", "3", "4", "", "5"]),
        ),
        case("alternate-screen-enter", Group::Screen, b"main\x1b[?1049h\x1b[Halt", |c| {
            screen_is(c, &["alt"])?;
            match c.mode(Mode::AltScreen) {
                Some(true) => Ok(()),
                other => Err(format!("alternate screen mode reads {other:?}")),
            }
        }),
        case("alternate-screen-leave", Group::Screen, b"main\x1b[?1049h\x1b[Halt\x1b[?1049l", |c| {
            screen_is(c, &["main"])?;
            if c.cursor() == (4, 0) { Ok(()) } else { Err(format!("cursor {:?}", c.cursor())) }
        }),
        case(
            "text-attributes",
            Group::Screen,
            b"\x1b[1mB\x1b[0;3mI\x1b[0;4mU\x1b[0;4:3mC\x1b[0;4:2mD\x1b[0;9mS\x1b[0;7mR\x1b[0mN",
            |c| {
                let mut problems = Vec::new();
                if !c.style_at(0, 0).bold {
                    problems.push("bold");
                }
                if !c.style_at(0, 1).italic {
                    problems.push("italic");
                }
                if c.style_at(0, 2).underline != Underline::Single {
                    problems.push("underline");
                }
                if c.style_at(0, 3).underline != Underline::Curly {
                    problems.push("curly underline");
                }
                if c.style_at(0, 4).underline != Underline::Double {
                    problems.push("double underline");
                }
                if !c.style_at(0, 5).strike {
                    problems.push("strikethrough");
                }
                if !c.style_at(0, 6).inverse {
                    problems.push("inverse");
                }
                let plain = c.style_at(0, 7);
                if plain.bold || plain.italic || plain.strike || plain.inverse {
                    problems.push("reset");
                }
                if problems.is_empty() { Ok(()) } else { Err(format!("wrong: {}", problems.join(", "))) }
            },
        ),
        case(
            "colors-16-256-full",
            Group::Screen,
            b"\x1b[31ma\x1b[38;5;200mb\x1b[38;2;1;2;3mc\x1b[38:2::4:5:6md\x1b[39me",
            |c| {
                let got: Vec<Color> = (0..5).map(|col| c.style_at(0, col).fg).collect();
                let want = [
                    Color::Indexed(1),
                    Color::Indexed(200),
                    Color::Rgb(1, 2, 3),
                    Color::Rgb(4, 5, 6),
                    Color::Default,
                ];
                if got == want { Ok(()) } else { Err(format!("colors {got:?}")) }
            },
        ),
        case("modes-paste-focus-mouse", Group::Screen, b"\x1b[?2004h\x1b[?1004h\x1b[?1000h\x1b[?1006h", |c| {
            let mut problems = Vec::new();
            for (mode, name) in [
                (Mode::BracketedPaste, "bracketed paste"),
                (Mode::FocusReport, "focus reporting"),
                (Mode::MouseTracking, "mouse tracking"),
                (Mode::MouseSgr, "mouse SGR"),
            ] {
                match c.mode(mode) {
                    Some(true) => {}
                    Some(false) => problems.push(format!("{name} not set")),
                    None => problems.push(format!("{name} not tracked")),
                }
            }
            if problems.is_empty() { Ok(()) } else { Err(problems.join(", ")) }
        }),
        case("synchronized-output-mode", Group::Screen, b"\x1b[?2026hX", |c| match c.mode(Mode::SyncOutput) {
            Some(true) => Ok(()),
            Some(false) => Err("mode not set".into()),
            None => Err("mode not tracked".into()),
        }),
        case("kitty-keyboard-push", Group::Screen, b"\x1b[>1u", |c| match c.mode(Mode::KittyKeyboard) {
            Some(true) => Ok(()),
            Some(false) => Err("flags not set".into()),
            None => Err("protocol not tracked".into()),
        }),
        // ---- Replies ----
        reply("device-attributes", b"\x1b[c", b"\x1b[?"),
        reply("secondary-device-attributes", b"\x1b[>c", b"\x1b[>"),
        reply("cursor-position-report", b"\x1b[3;5H\x1b[6n", b"\x1b[3;5R"),
        reply("mode-report-sync-output", b"\x1b[?2026$p", b"\x1b[?2026;2$y"),
        reply("mode-report-bracketed-paste", b"\x1b[?2004h\x1b[?2004$p", b"\x1b[?2004;1$y"),
        reply("kitty-keyboard-query", b"\x1b[>1u\x1b[?u", b"\x1b[?1u"),
    ];
    cases.sort_by_key(|c| c.group as u8);
    cases
}

/// The ways a case's input is cut into separate writes.
pub const SPLITS: [(&str, usize); 4] = [
    ("whole", usize::MAX),
    ("1 byte", 1),
    ("7 bytes", 7),
    ("1024 bytes", 1024),
];

pub struct Outcome {
    pub id: &'static str,
    pub group: Group,
    /// `None` when the case passes under every split.
    pub failure: Option<String>,
}

pub fn run_case(candidate_name: &str, case: &Case) -> Outcome {
    for (split_name, size) in SPLITS {
        let mut candidate = crate::candidate::make(candidate_name, COLS, ROWS, 1000);
        if size == usize::MAX {
            candidate.feed(&case.input);
        } else {
            for chunk in case.input.chunks(size) {
                candidate.feed(chunk);
            }
        }
        if let Err(message) = (case.check)(candidate.as_mut()) {
            return Outcome {
                id: case.id,
                group: case.group,
                failure: Some(format!("[split: {split_name}] {message}")),
            };
        }
    }
    Outcome {
        id: case.id,
        group: case.group,
        failure: None,
    }
}
