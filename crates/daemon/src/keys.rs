//! Reading the host terminal's keyboard bytes.
//!
//! The reader keeps what it could not finish yet (a sequence cut between two reads, or a lone
//! Esc that may be the start of one) and turns the bytes into [`Input`]: keys Arcade can act on,
//! pasted text, and anything it does not understand, which is passed on as raw bytes so that
//! nothing a program asked for is lost.
//!
//! The daemon asks the host terminal for the kitty keyboard protocol (flag 1, "disambiguate"),
//! so that Esc, Ctrl+m and Ctrl+i can be told from Enter and Tab. A terminal that ignores the
//! request still works: the legacy bytes are understood as well, and a lone Esc is told from the
//! start of a sequence by a short wait.

use std::time::{Duration, Instant};

use arcade_term::{Code, Key, Mods, Named};

/// How long a lone Esc waits for the rest of a sequence before it counts as the Esc key.
pub const ESC_WAIT: Duration = Duration::from_millis(10);

const PASTE_START: &[u8] = b"\x1b[200~";
const PASTE_END: &[u8] = b"\x1b[201~";

/// One thing read from the host terminal.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Input {
    /// A key, with the bytes it came as.
    Key { key: Key, raw: Vec<u8> },
    /// Text pasted between the bracketed paste markers (the markers are not included).
    Paste(Vec<u8>),
    /// A mouse report in the SGR form.
    Mouse(Mouse),
    /// The host terminal got (true) or lost (false) the focus, as its focus reporting says.
    Focus(bool),
    /// Bytes that are not a key Arcade knows (odd sequences).
    Raw(Vec<u8>),
}

/// What a mouse report says happened.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum MouseKind {
    /// A button went down (0 left, 1 middle, 2 right).
    Press(u8),
    /// A button came up.
    Release(u8),
    /// The mouse moved with a button held.
    Drag(u8),
    /// The mouse moved with no button held.
    Move,
    /// The wheel turned; `up` and `down` are the vertical turns, `left` and `right` the sideways.
    Wheel(Wheel),
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Wheel {
    Up,
    Down,
    Left,
    Right,
}

/// A mouse report: where, what, and the modifier keys held. `x` and `y` count from 0 at the
/// top left of the host terminal.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Mouse {
    pub kind: MouseKind,
    pub mods: Mods,
    pub x: u16,
    pub y: u16,
}

impl Mouse {
    /// Read `CSI < code ; column ; row M` (`m` for a release).
    fn from_sgr(params: &[u8], last: u8) -> Option<Mouse> {
        let text = std::str::from_utf8(params.strip_prefix(b"<")?).ok()?;
        let mut numbers = text.split(';').map(|n| n.parse::<u16>().ok());
        let code = numbers.next()??;
        let column = numbers.next()??;
        let row = numbers.next()??;
        if numbers.next().is_some() || column == 0 || row == 0 || code > 127 {
            return None;
        }
        let mods = Mods {
            shift: code & 4 != 0,
            alt: code & 8 != 0,
            ctrl: code & 16 != 0,
            super_key: false,
        };
        let button = (code & 3) as u8;
        let kind = if code & 64 != 0 {
            match code & 3 {
                0 => MouseKind::Wheel(Wheel::Up),
                1 => MouseKind::Wheel(Wheel::Down),
                2 => MouseKind::Wheel(Wheel::Left),
                _ => MouseKind::Wheel(Wheel::Right),
            }
        } else if code & 32 != 0 {
            if button == 3 {
                MouseKind::Move
            } else {
                MouseKind::Drag(button)
            }
        } else if last == b'm' {
            MouseKind::Release(button)
        } else {
            MouseKind::Press(button)
        };
        Some(Mouse {
            kind,
            mods,
            x: column - 1,
            y: row - 1,
        })
    }
}

#[derive(Default)]
pub struct KeyReader {
    pending: Vec<u8>,
    /// Set while the pending bytes are waiting for the rest of a sequence.
    waiting_since: Option<Instant>,
    /// The text of a paste that has started and not ended yet.
    paste: Option<Vec<u8>>,
}

fn key(code: Code, mods: Mods, raw: &[u8]) -> Input {
    Input::Key {
        key: Key { code, mods },
        raw: raw.to_vec(),
    }
}

fn char_key(c: char, mut mods: Mods, raw: &[u8]) -> Input {
    // Letters are given in lower case with Shift set, as the key encoder wants them.
    if c.is_uppercase()
        && let Some(lower) = c.to_lowercase().next().filter(|l| l.is_alphabetic())
    {
        mods.shift = true;
        return key(Code::Char(lower), mods, raw);
    }
    key(Code::Char(c), mods, raw)
}

fn mods_from_param(param: u32) -> Mods {
    let bits = param.saturating_sub(1);
    Mods {
        shift: bits & 1 != 0,
        alt: bits & 2 != 0,
        ctrl: bits & 4 != 0,
        super_key: bits & 8 != 0,
    }
}

/// What a sequence's final byte and first number mean, for the forms with `~`.
fn tilde_key(number: u32) -> Option<Named> {
    Some(match number {
        1 | 7 => Named::Home,
        2 => Named::Insert,
        3 => Named::Delete,
        4 | 8 => Named::End,
        5 => Named::PageUp,
        6 => Named::PageDown,
        11 => Named::F(1),
        12 => Named::F(2),
        13 => Named::F(3),
        14 => Named::F(4),
        15 => Named::F(5),
        17 => Named::F(6),
        18 => Named::F(7),
        19 => Named::F(8),
        20 => Named::F(9),
        21 => Named::F(10),
        23 => Named::F(11),
        24 => Named::F(12),
        _ => return None,
    })
}

fn final_key(byte: u8) -> Option<Named> {
    Some(match byte {
        b'A' => Named::Up,
        b'B' => Named::Down,
        b'C' => Named::Right,
        b'D' => Named::Left,
        b'H' => Named::Home,
        b'F' => Named::End,
        b'P' => Named::F(1),
        b'Q' => Named::F(2),
        b'R' => Named::F(3),
        b'S' => Named::F(4),
        _ => return None,
    })
}

/// The code of a key reported by `CSI code u`.
fn unicode_key(code: u32) -> Option<Code> {
    Some(match code {
        9 => Code::Named(Named::Tab),
        13 => Code::Named(Named::Enter),
        27 => Code::Named(Named::Escape),
        127 => Code::Named(Named::Backspace),
        // The keypad and function keys of the protocol's private range are not handled.
        57344..=63743 => return None,
        c => {
            let c = char::from_u32(c)?;
            if c.is_control() {
                return None;
            }
            Code::Char(c)
        }
    })
}

enum Step {
    /// The input was a complete item of this many bytes.
    Done(usize, Option<Input>),
    /// More bytes are needed.
    Wait,
}

/// The first number of a `;`-separated parameter, ignoring `:` sub-parameters.
fn param(params: &[u8], index: usize) -> Option<u32> {
    let part = params.split(|b| *b == b';').nth(index)?;
    let first = part.split(|b| *b == b':').next()?;
    std::str::from_utf8(first).ok()?.parse().ok()
}

/// The event type of a `CSI code ; mods:event u` report: 1 press, 2 repeat, 3 release.
fn event_type(params: &[u8]) -> u32 {
    params
        .split(|b| *b == b';')
        .nth(1)
        .and_then(|part| part.split(|b| *b == b':').nth(1))
        .and_then(|e| std::str::from_utf8(e).ok()?.parse().ok())
        .unwrap_or(1)
}

fn csi(buf: &[u8]) -> Step {
    // buf starts with ESC [
    let mut i = 2;
    while i < buf.len() && (0x30..=0x3f).contains(&buf[i]) {
        i += 1;
    }
    while i < buf.len() && (0x20..=0x2f).contains(&buf[i]) {
        i += 1;
    }
    if i >= buf.len() {
        return Step::Wait;
    }
    let last = buf[i];
    let end = i + 1;
    if !(0x40..=0x7e).contains(&last) {
        // Not a sequence; pass the ESC [ and let the rest be read again.
        return Step::Done(2, Some(Input::Raw(buf[..2].to_vec())));
    }
    let raw = &buf[..end];
    let params = &buf[2..i];
    let numbered = params.first().is_some_and(u8::is_ascii_digit);
    let input = match last {
        b'u' if numbered => {
            let code = param(params, 0).and_then(unicode_key);
            let mods = mods_from_param(param(params, 1).unwrap_or(1));
            match code {
                // A release is not a key press.
                _ if event_type(params) == 3 => None,
                Some(code) => Some(key(code, mods, raw)),
                None => Some(Input::Raw(raw.to_vec())),
            }
        }
        b'~' if numbered => match param(params, 0).and_then(tilde_key) {
            Some(named) => Some(key(
                Code::Named(named),
                mods_from_param(param(params, 1).unwrap_or(1)),
                raw,
            )),
            None => Some(Input::Raw(raw.to_vec())),
        },
        b'M' | b'm' if params.first() == Some(&b'<') => Some(
            Mouse::from_sgr(params, last).map_or_else(|| Input::Raw(raw.to_vec()), Input::Mouse),
        ),
        b'I' | b'O' if params.is_empty() => Some(Input::Focus(last == b'I')),
        b'Z' if params.is_empty() => Some(key(
            Code::Named(Named::Tab),
            Mods {
                shift: true,
                ..Mods::NONE
            },
            raw,
        )),
        b'A' | b'B' | b'C' | b'D' | b'H' | b'F' | b'P' | b'Q' | b'R' | b'S'
            if params.is_empty() || (params.first() == Some(&b'1') && params.len() > 1) =>
        {
            let mods = if params.is_empty() {
                Mods::NONE
            } else {
                mods_from_param(param(params, 1).unwrap_or(1))
            };
            final_key(last)
                .map(|n| Key {
                    code: Code::Named(n),
                    mods,
                })
                .map(|key| Input::Key {
                    key,
                    raw: raw.to_vec(),
                })
        }
        _ => Some(Input::Raw(raw.to_vec())),
    };
    Step::Done(end, input)
}

fn ss3(buf: &[u8]) -> Step {
    if buf.len() < 3 {
        return Step::Wait;
    }
    let raw = &buf[..3];
    let input = match final_key(buf[2]) {
        Some(n) => key(Code::Named(n), Mods::NONE, raw),
        None => Input::Raw(raw.to_vec()),
    };
    Step::Done(3, Some(input))
}

/// A key of one character or control byte, without any escape.
fn plain(buf: &[u8], extra: Mods) -> Step {
    let b = buf[0];
    let single = |code: Code, mods: Mods| Step::Done(1, Some(key(code, mods, &buf[..1])));
    let with = |mods: Mods| Mods {
        alt: extra.alt || mods.alt,
        ..mods
    };
    match b {
        0x0d => single(Code::Named(Named::Enter), with(Mods::NONE)),
        0x09 => single(Code::Named(Named::Tab), with(Mods::NONE)),
        0x7f => single(Code::Named(Named::Backspace), with(Mods::NONE)),
        0x00 => single(
            Code::Char(' '),
            with(Mods {
                ctrl: true,
                ..Mods::NONE
            }),
        ),
        0x01..=0x1a => single(
            Code::Char(char::from(b + 0x60)),
            with(Mods {
                ctrl: true,
                ..Mods::NONE
            }),
        ),
        0x1c..=0x1f => single(
            Code::Char(char::from(b + 0x40).to_ascii_lowercase()),
            with(Mods {
                ctrl: true,
                ..Mods::NONE
            }),
        ),
        0x20..=0x7e => Step::Done(
            1,
            Some(char_key(char::from(b), with(Mods::NONE), &buf[..1])),
        ),
        0x80..=0xff => {
            let need = match b {
                0xc2..=0xdf => 2,
                0xe0..=0xef => 3,
                0xf0..=0xf4 => 4,
                _ => return Step::Done(1, Some(Input::Raw(vec![b]))),
            };
            if buf.len() < need {
                return Step::Wait;
            }
            match std::str::from_utf8(&buf[..need]) {
                Ok(s) => {
                    let c = s.chars().next().unwrap_or('\u{fffd}');
                    Step::Done(need, Some(char_key(c, with(Mods::NONE), &buf[..need])))
                }
                Err(_) => Step::Done(1, Some(Input::Raw(vec![b]))),
            }
        }
        // Esc is handled by the caller.
        _ => Step::Done(1, Some(Input::Raw(vec![b]))),
    }
}

fn next(buf: &[u8]) -> Step {
    if buf[0] != 0x1b {
        return plain(buf, Mods::NONE);
    }
    if buf.len() == 1 {
        return Step::Wait;
    }
    match buf[1] {
        b'[' => csi(buf),
        b'O' => ss3(buf),
        0x1b => Step::Done(
            2,
            Some(key(
                Code::Named(Named::Escape),
                Mods {
                    alt: true,
                    ..Mods::NONE
                },
                &buf[..2],
            )),
        ),
        _ => {
            // Alt held with a key: Esc, then the key's own bytes.
            let alt = Mods {
                alt: true,
                ..Mods::NONE
            };
            match plain(&buf[1..], alt) {
                Step::Done(n, Some(Input::Key { key, .. })) => Step::Done(
                    n + 1,
                    Some(Input::Key {
                        key,
                        raw: buf[..n + 1].to_vec(),
                    }),
                ),
                Step::Done(n, other) => Step::Done(
                    n + 1,
                    other.map(|i| match i {
                        Input::Raw(mut r) => {
                            r.insert(0, 0x1b);
                            Input::Raw(r)
                        }
                        other => other,
                    }),
                ),
                Step::Wait => Step::Wait,
            }
        }
    }
}

impl KeyReader {
    pub fn new() -> KeyReader {
        KeyReader::default()
    }

    /// Take bytes from the host terminal and return what they complete.
    pub fn feed(&mut self, bytes: &[u8], now: Instant) -> Vec<Input> {
        let mut out = Vec::new();
        self.pending.extend_from_slice(bytes);
        self.drain(&mut out);
        self.waiting_since = if self.pending.is_empty() || self.paste.is_some() {
            None
        } else {
            Some(self.waiting_since.unwrap_or(now))
        };
        out
    }

    /// When the bytes that are waiting must be given up on, if any are.
    pub fn deadline(&self) -> Option<Instant> {
        self.waiting_since.map(|t| t + ESC_WAIT)
    }

    /// Give up on what is waiting once its time is up: a lone Esc is the Esc key, anything else
    /// is passed on as it is.
    pub fn expire(&mut self, now: Instant) -> Vec<Input> {
        let Some(deadline) = self.deadline() else {
            return Vec::new();
        };
        if now < deadline {
            return Vec::new();
        }
        self.waiting_since = None;
        let pending = std::mem::take(&mut self.pending);
        if pending == [0x1b] {
            return vec![key(Code::Named(Named::Escape), Mods::NONE, &pending)];
        }
        vec![Input::Raw(pending)]
    }

    fn drain(&mut self, out: &mut Vec<Input>) {
        let mut at = 0;
        loop {
            if let Some(text) = self.paste.as_mut() {
                let rest = &self.pending[at..];
                if let Some(pos) = find(rest, PASTE_END) {
                    text.extend_from_slice(&rest[..pos]);
                    at += pos + PASTE_END.len();
                    out.push(Input::Paste(self.paste.take().unwrap_or_default()));
                    continue;
                }
                // Keep a possible start of the end marker for the next read.
                let keep = (1..PASTE_END.len().min(rest.len() + 1))
                    .rev()
                    .find(|n| rest.ends_with(&PASTE_END[..*n]))
                    .unwrap_or(0);
                text.extend_from_slice(&rest[..rest.len() - keep]);
                at = self.pending.len() - keep;
                break;
            }
            if at >= self.pending.len() {
                break;
            }
            let rest = &self.pending[at..];
            if rest.starts_with(PASTE_START) {
                self.paste = Some(Vec::new());
                at += PASTE_START.len();
                continue;
            }
            if PASTE_START.starts_with(rest) && rest.len() < PASTE_START.len() {
                // May become the start of a paste.
                break;
            }
            match next(rest) {
                Step::Wait => break,
                Step::Done(n, input) => {
                    at += n;
                    out.extend(input);
                }
            }
        }
        self.pending.drain(..at);
    }
}

fn find(haystack: &[u8], needle: &[u8]) -> Option<usize> {
    haystack.windows(needle.len()).position(|w| w == needle)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn read_all(bytes: &[u8]) -> Vec<Input> {
        let mut r = KeyReader::new();
        let now = Instant::now();
        let mut out = r.feed(bytes, now);
        out.extend(r.expire(now + Duration::from_secs(1)));
        out
    }

    fn keys(inputs: &[Input]) -> Vec<Key> {
        inputs
            .iter()
            .map(|i| match i {
                Input::Key { key, .. } => *key,
                other => panic!("not a key: {other:?}"),
            })
            .collect()
    }

    fn named(n: Named, mods: Mods) -> Key {
        Key {
            code: Code::Named(n),
            mods,
        }
    }

    fn ctrl() -> Mods {
        Mods {
            ctrl: true,
            ..Mods::NONE
        }
    }

    fn alt() -> Mods {
        Mods {
            alt: true,
            ..Mods::NONE
        }
    }

    fn shift() -> Mods {
        Mods {
            shift: true,
            ..Mods::NONE
        }
    }

    /// Every key form with the key it must make.
    fn table() -> Vec<(Vec<u8>, Key)> {
        let ch = |c: char, m: Mods| Key {
            code: Code::Char(c),
            mods: m,
        };
        vec![
            (b"a".to_vec(), ch('a', Mods::NONE)),
            (b"A".to_vec(), ch('a', shift())),
            (b"/".to_vec(), ch('/', Mods::NONE)),
            ("é".as_bytes().to_vec(), ch('é', Mods::NONE)),
            ("漢".as_bytes().to_vec(), ch('漢', Mods::NONE)),
            (b"\r".to_vec(), named(Named::Enter, Mods::NONE)),
            (b"\t".to_vec(), named(Named::Tab, Mods::NONE)),
            (b"\x7f".to_vec(), named(Named::Backspace, Mods::NONE)),
            (b"\x03".to_vec(), ch('c', ctrl())),
            (b"\x00".to_vec(), ch(' ', ctrl())),
            (b"\x1bx".to_vec(), ch('x', alt())),
            (b"\x1b\r".to_vec(), named(Named::Enter, alt())),
            (b"\x1b[A".to_vec(), named(Named::Up, Mods::NONE)),
            (b"\x1bOB".to_vec(), named(Named::Down, Mods::NONE)),
            (b"\x1b[1;5C".to_vec(), named(Named::Right, ctrl())),
            (b"\x1b[1;3D".to_vec(), named(Named::Left, alt())),
            (b"\x1b[3~".to_vec(), named(Named::Delete, Mods::NONE)),
            (b"\x1b[5;2~".to_vec(), named(Named::PageUp, shift())),
            (b"\x1b[15~".to_vec(), named(Named::F(5), Mods::NONE)),
            (b"\x1bOP".to_vec(), named(Named::F(1), Mods::NONE)),
            (b"\x1b[Z".to_vec(), named(Named::Tab, shift())),
            (b"\x1b[27u".to_vec(), named(Named::Escape, Mods::NONE)),
            (b"\x1b[109;5u".to_vec(), ch('m', ctrl())),
            (b"\x1b[105;5u".to_vec(), ch('i', ctrl())),
            (b"\x1b[13;2u".to_vec(), named(Named::Enter, shift())),
            (b"\x1b[97;3u".to_vec(), ch('a', alt())),
            (b"\x1b[97;5:1u".to_vec(), ch('a', ctrl())),
        ]
    }

    #[test]
    fn every_key_form_is_read_when_it_arrives_whole() {
        for (bytes, expected) in table() {
            let got = read_all(&bytes);
            assert_eq!(keys(&got), vec![expected], "{bytes:?}");
            match &got[0] {
                Input::Key { raw, .. } => assert_eq!(raw, &bytes),
                _ => unreachable!(),
            }
        }
    }

    #[test]
    fn every_key_form_is_read_when_cut_at_any_place() {
        for (bytes, expected) in table() {
            for cut in 1..bytes.len() {
                let mut r = KeyReader::new();
                let now = Instant::now();
                let mut got = r.feed(&bytes[..cut], now);
                got.extend(r.feed(&bytes[cut..], now));
                got.extend(r.expire(now + Duration::from_secs(1)));
                assert_eq!(keys(&got), vec![expected], "{bytes:?} cut at {cut}");
            }
        }
    }

    #[test]
    fn keys_in_a_row_come_out_in_order() {
        let mut bytes = Vec::new();
        let mut expected = Vec::new();
        for (b, k) in table() {
            bytes.extend_from_slice(&b);
            expected.push(k);
        }
        assert_eq!(keys(&read_all(&bytes)), expected);
    }

    #[test]
    fn a_lone_esc_is_the_escape_key_only_after_the_wait() {
        let mut r = KeyReader::new();
        let now = Instant::now();
        assert!(r.feed(b"\x1b", now).is_empty());
        assert_eq!(r.deadline(), Some(now + ESC_WAIT));
        assert!(r.expire(now + ESC_WAIT / 2).is_empty());
        let got = r.expire(now + ESC_WAIT);
        assert_eq!(keys(&got), vec![named(Named::Escape, Mods::NONE)]);
        assert_eq!(r.deadline(), None);
    }

    #[test]
    fn esc_followed_by_a_sequence_within_the_wait_is_not_escape() {
        let mut r = KeyReader::new();
        let now = Instant::now();
        assert!(r.feed(b"\x1b", now).is_empty());
        let got = r.feed(b"[A", now + ESC_WAIT / 2);
        assert_eq!(keys(&got), vec![named(Named::Up, Mods::NONE)]);
        assert_eq!(r.deadline(), None);
    }

    #[test]
    fn things_that_are_not_keys_pass_through_as_raw_bytes() {
        // A damaged mouse report, a focus report, an unknown private sequence and a bare bad byte.
        for bytes in [
            &b"\x1b[<0;10M"[..],
            b"\x1b[<0;0;5M",
            b"\x1b[?1;2c",
            b"\x1b[57399;5u",
        ] {
            let got = read_all(bytes);
            assert_eq!(got, vec![Input::Raw(bytes.to_vec())], "{bytes:?}");
        }
        assert_eq!(read_all(b"\xff"), vec![Input::Raw(vec![0xff])]);
    }

    #[test]
    fn mouse_reports_are_read_with_their_place_and_modifiers() {
        let one = |bytes: &[u8]| match read_all(bytes).as_slice() {
            [Input::Mouse(m)] => *m,
            other => panic!("{bytes:?} gave {other:?}"),
        };
        let at = |m: Mouse| (m.x, m.y);
        let m = one(b"\x1b[<0;10;5M");
        assert_eq!(
            (m.kind, at(m), m.mods),
            (MouseKind::Press(0), (9, 4), Mods::NONE)
        );
        assert_eq!(one(b"\x1b[<0;10;5m").kind, MouseKind::Release(0));
        assert_eq!(one(b"\x1b[<2;1;1M").kind, MouseKind::Press(2));
        assert_eq!(one(b"\x1b[<32;3;4M").kind, MouseKind::Drag(0));
        assert_eq!(one(b"\x1b[<35;3;4M").kind, MouseKind::Move);
        assert_eq!(one(b"\x1b[<64;3;4M").kind, MouseKind::Wheel(Wheel::Up));
        assert_eq!(one(b"\x1b[<65;3;4M").kind, MouseKind::Wheel(Wheel::Down));
        assert_eq!(one(b"\x1b[<66;3;4M").kind, MouseKind::Wheel(Wheel::Left));
        assert_eq!(one(b"\x1b[<67;3;4M").kind, MouseKind::Wheel(Wheel::Right));
        let m = one(b"\x1b[<28;300;70M");
        assert_eq!(at(m), (299, 69));
        assert!(m.mods.shift && m.mods.alt && m.mods.ctrl);
    }

    #[test]
    fn a_mouse_report_cut_anywhere_is_put_together() {
        let bytes = b"a\x1b[<0;123;45Mb";
        for cut in 1..bytes.len() {
            let mut r = KeyReader::new();
            let now = Instant::now();
            let mut got = r.feed(&bytes[..cut], now);
            got.extend(r.feed(&bytes[cut..], now));
            assert_eq!(got.len(), 3, "cut at {cut}: {got:?}");
            assert!(
                matches!(got[1], Input::Mouse(m) if (m.x, m.y) == (122, 44)),
                "{got:?}"
            );
        }
    }

    #[test]
    fn a_key_release_makes_nothing() {
        assert!(read_all(b"\x1b[97;1:3u").is_empty());
    }

    #[test]
    fn a_paste_comes_whole_however_it_is_cut() {
        let mut payload: Vec<u8> = (0..20_000u32)
            .map(|i| b"ab\x1b[A\r\n\xc3\xa9"[(i % 9) as usize])
            .collect();
        payload.extend_from_slice(b"\x1b[20"); // looks like the start of the end marker
        let mut bytes = PASTE_START.to_vec();
        bytes.extend_from_slice(&payload);
        bytes.extend_from_slice(PASTE_END);
        bytes.extend_from_slice(b"z");
        for chunk in [1usize, 3, 7, 4096, bytes.len()] {
            let mut r = KeyReader::new();
            let now = Instant::now();
            let mut got = Vec::new();
            for piece in bytes.chunks(chunk) {
                got.extend(r.feed(piece, now));
            }
            assert_eq!(got.len(), 2, "chunk {chunk}");
            assert_eq!(got[0], Input::Paste(payload.clone()), "chunk {chunk}");
            assert_eq!(
                keys(&got[1..]),
                vec![Key {
                    code: Code::Char('z'),
                    mods: Mods::NONE
                }]
            );
            assert_eq!(r.deadline(), None);
        }
    }

    #[test]
    fn random_bytes_never_lose_or_invent_data() {
        // A simple generator; every byte fed must come out again, in order, as a key's raw bytes,
        // raw bytes, or a paste.
        let mut seed = 0x2545_f491_4f6c_dd1du64;
        let mut next_byte = || {
            seed ^= seed << 13;
            seed ^= seed >> 7;
            seed ^= seed << 17;
            seed as u8
        };
        for _ in 0..200 {
            let len = usize::from(next_byte()) + 1;
            let bytes: Vec<u8> = (0..len).map(|_| next_byte()).collect();
            let mut r = KeyReader::new();
            let now = Instant::now();
            let mut got = Vec::new();
            for piece in bytes.chunks(1 + usize::from(next_byte() % 5)) {
                got.extend(r.feed(piece, now));
            }
            got.extend(r.expire(now + Duration::from_secs(1)));
            let mut back = Vec::new();
            let mut saw_mouse = false;
            for i in got {
                match i {
                    Input::Key { raw, .. } | Input::Raw(raw) => back.extend(raw),
                    Input::Mouse(_) => saw_mouse = true,
                    Input::Focus(f) => {
                        back.extend_from_slice(if f { b"\x1b[I" } else { b"\x1b[O" })
                    }
                    Input::Paste(p) => {
                        back.extend_from_slice(PASTE_START);
                        back.extend(p);
                        back.extend_from_slice(PASTE_END);
                    }
                }
            }
            // A key release is dropped by design, and a paste that never ended is held back, so
            // compare only when neither happened.
            if !saw_mouse
                && !bytes.windows(3).any(|w| w == b"\x1b[2")
                && find(&bytes, b"3u").is_none()
            {
                assert_eq!(back, bytes);
            }
        }
    }
}
