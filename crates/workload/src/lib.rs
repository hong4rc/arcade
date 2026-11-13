//! Deterministic terminal output workloads shared by the parser evaluation and the benchmark suite.
//!
//! Each workload is a stream of bytes a program could write to a terminal. The same seed always
//! yields the same bytes, so two runs (or two multiplexers) are measured on identical input.
//! Streams are built from whole units (a line or a frame), so no escape sequence is ever cut by
//! the end of the buffer.

/// Terminal size the workloads are generated for.
pub const COLS: u16 = 120;
pub const ROWS: u16 = 40;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Workload {
    /// Plain ASCII text lines.
    Plain,
    /// Text where almost every word changes color or attributes.
    Colored,
    /// Very short lines, so nearly every few bytes scroll the screen.
    Scroll,
    /// Full-screen application style: every frame repaints all rows with cursor addressing.
    FullScreen,
    /// Wide characters, combining marks (Vietnamese in decomposed form) and emoji sequences.
    Wide,
    /// Many long escape sequences: hyperlinks, titles, clipboard writes, long color sequences.
    LongEscapes,
}

impl Workload {
    pub const ALL: [Workload; 6] = [
        Workload::Plain,
        Workload::Colored,
        Workload::Scroll,
        Workload::FullScreen,
        Workload::Wide,
        Workload::LongEscapes,
    ];

    pub fn name(self) -> &'static str {
        match self {
            Workload::Plain => "plain",
            Workload::Colored => "colored",
            Workload::Scroll => "scroll",
            Workload::FullScreen => "fullscreen",
            Workload::Wide => "wide",
            Workload::LongEscapes => "long-escapes",
        }
    }

    pub fn from_name(name: &str) -> Option<Workload> {
        Workload::ALL.into_iter().find(|w| w.name() == name)
    }
}

/// Small deterministic random number generator (xorshift64*), so the crate needs no dependency.
#[derive(Clone)]
pub struct Rng(u64);

impl Rng {
    pub fn new(seed: u64) -> Self {
        Rng(seed.wrapping_mul(0x9E37_79B9_7F4A_7C15) | 1)
    }

    pub fn next_u64(&mut self) -> u64 {
        self.0 ^= self.0 >> 12;
        self.0 ^= self.0 << 25;
        self.0 ^= self.0 >> 27;
        self.0.wrapping_mul(0x2545_F491_4F6C_DD1D)
    }

    /// Uniform value in `0..n`. `n` must be greater than zero.
    pub fn below(&mut self, n: usize) -> usize {
        (self.next_u64() % n as u64) as usize
    }

    pub fn range(&mut self, low: usize, high: usize) -> usize {
        low + self.below(high - low + 1)
    }

    pub fn fill(&mut self, buf: &mut [u8]) {
        for chunk in buf.chunks_mut(8) {
            let v = self.next_u64().to_le_bytes();
            chunk.copy_from_slice(&v[..chunk.len()]);
        }
    }
}

const WORDS: &[&str] = &[
    "error",
    "warning",
    "info",
    "debug",
    "trace",
    "build",
    "compile",
    "link",
    "test",
    "passed",
    "failed",
    "running",
    "finished",
    "target",
    "release",
    "src/main.rs",
    "lib.rs",
    "Cargo.toml",
    "fn",
    "let",
    "mut",
    "impl",
    "struct",
    "pub",
    "use",
    "match",
    "return",
    "async",
    "await",
    "the",
    "quick",
    "brown",
    "fox",
    "jumps",
    "over",
    "lazy",
    "dog",
    "0x7ffee4",
    "1024",
    "65536",
    "==>",
    "->",
    "::",
    "{",
    "}",
    "(",
    ")",
    "[ok]",
    "100%",
    "3.14159",
    "README.md",
    "/usr/local",
];

const WIDE_WORDS: &[&str] = &[
    "汉字",
    "終端",
    "日本語",
    "한국어",
    "テスト",
    // Vietnamese in decomposed form: base letters followed by combining marks.
    "Tie\u{302}\u{301}ng",
    "Vie\u{323}\u{302}t",
    "đu\u{301}ng",
    "co\u{323}\u{302}t",
    "ky\u{301}",
    "tu\u{31B}\u{323}",
    "👍",
    "🚀",
    "😀",
    // Emoji sequences: joined family, skin tone modifier, flag, variation selector.
    "👨\u{200D}👩\u{200D}👧\u{200D}👦",
    "👍\u{1F3FD}",
    "🇻🇳",
    "❤\u{FE0F}",
    "🧑\u{200D}💻",
    "ok",
    "mixed",
];

/// Generate at least `min_bytes` of the workload, made of whole units.
pub fn generate(workload: Workload, min_bytes: usize, seed: u64) -> Vec<u8> {
    let mut rng = Rng::new(seed ^ (workload as u64 + 1).wrapping_mul(0xA5A5_5A5A_1234_5678));
    let mut out = Vec::with_capacity(min_bytes + 16 * 1024);
    if workload == Workload::FullScreen {
        // Enter the alternate screen and hide the cursor, as a full-screen program does.
        out.extend_from_slice(b"\x1b[?1049h\x1b[?25l");
    }
    let mut frame = 0usize;
    while out.len() < min_bytes {
        match workload {
            Workload::Plain => plain_line(&mut rng, &mut out),
            Workload::Colored => colored_line(&mut rng, &mut out),
            Workload::Scroll => scroll_line(&mut rng, &mut out),
            Workload::FullScreen => {
                fullscreen_frame(&mut rng, &mut out, frame);
                frame += 1;
            }
            Workload::Wide => wide_line(&mut rng, &mut out),
            Workload::LongEscapes => long_escape_line(&mut rng, &mut out),
        }
    }
    out
}

fn plain_line(rng: &mut Rng, out: &mut Vec<u8>) {
    let width = rng.range(40, COLS as usize - 2);
    let start = out.len();
    while out.len() - start < width {
        out.extend_from_slice(WORDS[rng.below(WORDS.len())].as_bytes());
        out.push(b' ');
    }
    out.truncate(start + width);
    out.extend_from_slice(b"\r\n");
}

fn push_sgr(rng: &mut Rng, out: &mut Vec<u8>) {
    match rng.below(8) {
        0 => out.extend_from_slice(format!("\x1b[{}m", 30 + rng.below(8)).as_bytes()),
        1 => out.extend_from_slice(format!("\x1b[1;{}m", 90 + rng.below(8)).as_bytes()),
        2 => out.extend_from_slice(format!("\x1b[38;5;{}m", rng.below(256)).as_bytes()),
        3 => out.extend_from_slice(
            format!(
                "\x1b[38;2;{};{};{}m",
                rng.below(256),
                rng.below(256),
                rng.below(256)
            )
            .as_bytes(),
        ),
        4 => out.extend_from_slice(
            format!(
                "\x1b[48;2;{};{};{};38;5;{}m",
                rng.below(256),
                rng.below(256),
                rng.below(256),
                rng.below(256)
            )
            .as_bytes(),
        ),
        5 => out.extend_from_slice(b"\x1b[3;4m"),
        6 => out.extend_from_slice(b"\x1b[0;7m"),
        _ => out.extend_from_slice(b"\x1b[0m"),
    }
}

fn colored_line(rng: &mut Rng, out: &mut Vec<u8>) {
    let words = rng.range(6, 16);
    for _ in 0..words {
        push_sgr(rng, out);
        out.extend_from_slice(WORDS[rng.below(WORDS.len())].as_bytes());
        out.push(b' ');
    }
    out.extend_from_slice(b"\x1b[0m\r\n");
}

fn scroll_line(rng: &mut Rng, out: &mut Vec<u8>) {
    let n = rng.next_u64() % 100_000;
    out.extend_from_slice(format!("{n}\r\n").as_bytes());
}

fn fullscreen_frame(rng: &mut Rng, out: &mut Vec<u8>, frame: usize) {
    // Synchronized output around the frame, cursor addressing per row, colors per segment.
    out.extend_from_slice(b"\x1b[?2026h\x1b[H");
    for row in 1..=ROWS {
        out.extend_from_slice(format!("\x1b[{row};1H").as_bytes());
        let mut col = 0usize;
        while col < COLS as usize {
            push_sgr(rng, out);
            let word = WORDS[(rng.below(WORDS.len()) + frame) % WORDS.len()];
            let take = word.len().min(COLS as usize - col);
            out.extend_from_slice(&word.as_bytes()[..take]);
            col += take;
            if col < COLS as usize {
                out.push(b' ');
                col += 1;
            }
        }
        out.extend_from_slice(b"\x1b[0m\x1b[K");
    }
    out.extend_from_slice(format!("\x1b[{};{}H\x1b[?2026l", ROWS, 1 + frame % 80).as_bytes());
}

fn wide_line(rng: &mut Rng, out: &mut Vec<u8>) {
    let words = rng.range(6, 18);
    for _ in 0..words {
        out.extend_from_slice(WIDE_WORDS[rng.below(WIDE_WORDS.len())].as_bytes());
        out.push(b' ');
    }
    out.extend_from_slice(b"\r\n");
}

fn base64ish(rng: &mut Rng, len: usize, out: &mut Vec<u8>) {
    const ALPHABET: &[u8] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
    // Whole groups of four characters, so the payload is valid base64 without padding.
    for _ in 0..(len / 4) * 4 {
        out.push(ALPHABET[rng.below(64)]);
    }
}

fn long_escape_line(rng: &mut Rng, out: &mut Vec<u8>) {
    match rng.below(7) {
        0 => {
            // Hyperlink with a long address.
            out.extend_from_slice(b"\x1b]8;id=link;https://example.com/");
            for _ in 0..rng.range(40, 400) {
                out.push(b'a' + rng.below(26) as u8);
            }
            out.extend_from_slice(b"\x1b\\link text\x1b]8;;\x1b\\");
        }
        1 => {
            out.extend_from_slice(b"\x1b]0;");
            for _ in 0..rng.range(20, 200) {
                out.push(b'A' + rng.below(26) as u8);
            }
            out.push(0x07);
        }
        2 => {
            // Clipboard write with a few kilobytes of payload.
            out.extend_from_slice(b"\x1b]52;c;");
            let len = rng.range(256, 4096);
            base64ish(rng, len, out);
            out.push(0x07);
        }
        3 => {
            // Long color sequence using colon sub-parameters, plus styled underline.
            out.extend_from_slice(
                format!(
                    "\x1b[38:2::{}:{}:{};48:2::{}:{}:{};4:3;58:2::{}:{}:{}m",
                    rng.below(256),
                    rng.below(256),
                    rng.below(256),
                    rng.below(256),
                    rng.below(256),
                    rng.below(256),
                    rng.below(256),
                    rng.below(256),
                    rng.below(256)
                )
                .as_bytes(),
            );
            out.extend_from_slice(b"styled\x1b[0m");
        }
        4 => {
            // Device control string that no terminal is expected to recognise.
            out.extend_from_slice(b"\x1bP1000p");
            for _ in 0..rng.range(100, 2000) {
                out.push(b'0' + rng.below(10) as u8);
            }
            out.extend_from_slice(b"\x1b\\");
        }
        5 => {
            // Shell integration marks and a working-directory report.
            out.extend_from_slice(
                b"\x1b]133;A\x07\x1b]7;file://host/Users/someone/project/dir\x07",
            );
            out.extend_from_slice(b"\x1b]133;B\x07$ ls\x1b]133;C\x07");
        }
        _ => {
            // Application program command, used by image protocols.
            out.extend_from_slice(b"\x1b_Ga=q,i=1;");
            let len = rng.range(256, 2048);
            base64ish(rng, len, out);
            out.extend_from_slice(b"\x1b\\");
        }
    }
    out.extend_from_slice(b" text\r\n");
}

/// Fifty thousand lines of 120 columns: the scrollback memory case of the performance spec.
pub fn scrollback_fill(lines: usize, cols: usize, seed: u64) -> Vec<u8> {
    let mut rng = Rng::new(seed);
    let mut out = Vec::with_capacity(lines * (cols + 2));
    for _ in 0..lines {
        let start = out.len();
        while out.len() - start < cols {
            out.extend_from_slice(WORDS[rng.below(WORDS.len())].as_bytes());
            out.push(b' ');
        }
        out.truncate(start + cols);
        out.extend_from_slice(b"\r\n");
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn same_seed_gives_same_bytes() {
        for w in Workload::ALL {
            assert_eq!(
                generate(w, 64 * 1024, 7),
                generate(w, 64 * 1024, 7),
                "{}",
                w.name()
            );
        }
    }

    #[test]
    fn different_seed_gives_different_bytes() {
        for w in Workload::ALL {
            assert_ne!(
                generate(w, 64 * 1024, 1),
                generate(w, 64 * 1024, 2),
                "{}",
                w.name()
            );
        }
    }

    #[test]
    fn reaches_requested_size_and_is_valid_utf8() {
        for w in Workload::ALL {
            let data = generate(w, 200_000, 3);
            assert!(data.len() >= 200_000, "{}", w.name());
            assert!(std::str::from_utf8(&data).is_ok(), "{}", w.name());
        }
    }

    #[test]
    fn every_unit_ends_on_a_complete_sequence() {
        // The stream must end outside any escape sequence: its last bytes are a line end or the
        // end of a synchronized frame.
        for w in Workload::ALL {
            let data = generate(w, 50_000, 5);
            if w == Workload::FullScreen {
                assert!(data.ends_with(b"\x1b[?2026l"));
            } else {
                assert!(data.ends_with(b"\r\n"), "{}", w.name());
            }
        }
    }

    #[test]
    fn names_round_trip() {
        for w in Workload::ALL {
            assert_eq!(Workload::from_name(w.name()), Some(w));
        }
        assert_eq!(Workload::from_name("nope"), None);
    }

    #[test]
    fn scrollback_fill_has_exact_shape() {
        let data = scrollback_fill(100, 120, 1);
        assert_eq!(data.len(), 100 * 122);
        assert!(data.chunks(122).all(|line| line.ends_with(b"\r\n")));
    }
}
