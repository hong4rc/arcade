//! Progress reports of the program in a pane: `ESC ] 9 ; 4 ; <state> ; <value> BEL` (or `ST`
//! as the end). The terminal library tells them apart from notifications but does not hand them
//! over, so they are read here, from the bytes the program writes.
//!
//! Only a small, checked report comes out: a state from 0 to 4 and a value from 0 to 100.
//! Nothing else of the program's bytes is kept.

use memchr::memmem::Finder;

const PREFIX: &[u8] = b"\x1b]9;4;";
/// The longest body a report can have (`4;100` is five bytes); longer is not a report.
const BODY_MAX: usize = 8;

/// A progress report: the state (0 remove, 1 value, 2 error, 3 unknown amount, 4 paused) and
/// the value (0 to 100; 0 when the state has none).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Progress {
    pub state: u8,
    pub value: u8,
}

pub struct ProgressScan {
    finder: Finder<'static>,
    /// The last bytes of the previous write, in case the start of a report was cut after them.
    tail: Vec<u8>,
    /// The body of a report whose end has not come yet.
    body: Option<Vec<u8>>,
}

impl ProgressScan {
    pub fn new() -> ProgressScan {
        ProgressScan {
            finder: Finder::new(PREFIX).into_owned(),
            tail: Vec::new(),
            body: None,
        }
    }

    /// Look through the bytes of one write and push each report found.
    pub fn scan(&mut self, bytes: &[u8], out: &mut Vec<Progress>) {
        let mut at = 0;
        if self.body.is_some() {
            at = self.collect(bytes, 0, out);
        } else if !self.tail.is_empty() {
            // A report whose start was cut between two writes.
            let mut joined = self.tail.clone();
            joined.extend_from_slice(&bytes[..bytes.len().min(PREFIX.len() - 1)]);
            if let Some(s) = self.finder.find(&joined)
                && s < self.tail.len()
                && s + PREFIX.len() > self.tail.len()
            {
                self.body = Some(Vec::new());
                at = self.collect(bytes, s + PREFIX.len() - self.tail.len(), out);
            }
        }
        while self.body.is_none() {
            let Some(n) = self.finder.find(&bytes[at.min(bytes.len())..]) else {
                break;
            };
            self.body = Some(Vec::new());
            at = self.collect(bytes, at + n + PREFIX.len(), out);
        }
        if self.body.is_some() {
            self.tail.clear();
        } else {
            self.tail.extend_from_slice(bytes);
            let keep = self.tail.len().saturating_sub(PREFIX.len() - 1);
            self.tail.drain(..keep);
        }
    }

    /// Read the body from `from`; returns where reading stopped.
    fn collect(&mut self, bytes: &[u8], from: usize, out: &mut Vec<Progress>) -> usize {
        let body = self.body.as_mut().expect("a body is being read");
        let mut i = from;
        while i < bytes.len() {
            let b = bytes[i];
            match b {
                0x07 | 0x1b => {
                    let done = self.body.take().unwrap_or_default();
                    out.extend(parse(&done));
                    return if b == 0x07 { i + 1 } else { i };
                }
                b'0'..=b'9' | b';' if body.len() < BODY_MAX => body.push(b),
                _ => {
                    self.body = None;
                    return i;
                }
            }
            i += 1;
        }
        bytes.len()
    }
}

fn parse(body: &[u8]) -> Option<Progress> {
    let text = std::str::from_utf8(body).ok()?;
    let mut parts = text.split(';');
    let state: u8 = parts.next()?.parse().ok().filter(|s| *s <= 4)?;
    let value = match parts.next() {
        None | Some("") => 0,
        Some(v) => v.parse::<u8>().ok().filter(|v| *v <= 100)?,
    };
    if parts.next().is_some() {
        return None;
    }
    // A removal and an unknown amount have no value.
    let value = if matches!(state, 0 | 3) { 0 } else { value };
    Some(Progress { state, value })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn scan_all(chunks: &[&[u8]]) -> Vec<Progress> {
        let mut scan = ProgressScan::new();
        let mut out = Vec::new();
        for c in chunks {
            scan.scan(c, &mut out);
        }
        out
    }

    fn p(state: u8, value: u8) -> Progress {
        Progress { state, value }
    }

    #[test]
    fn reports_are_read_with_either_end() {
        assert_eq!(scan_all(&[b"x\x1b]9;4;1;50\x07y"]), vec![p(1, 50)]);
        assert_eq!(scan_all(&[b"\x1b]9;4;2;7\x1b\\"]), vec![p(2, 7)]);
        assert_eq!(scan_all(&[b"\x1b]9;4;0\x07"]), vec![p(0, 0)]);
        assert_eq!(scan_all(&[b"\x1b]9;4;3\x07"]), vec![p(3, 0)]);
        assert_eq!(scan_all(&[b"\x1b]9;4;3;80\x07"]), vec![p(3, 0)]);
        assert_eq!(scan_all(&[b"\x1b]9;4;1\x07"]), vec![p(1, 0)]);
        assert_eq!(
            scan_all(&[b"\x1b]9;4;1;10\x07..\x1b]9;4;1;20\x07"]),
            vec![p(1, 10), p(1, 20)]
        );
    }

    #[test]
    fn what_is_not_a_valid_report_is_dropped() {
        for bad in [
            &b"\x1b]9;4;5;10\x07"[..],
            b"\x1b]9;4;1;101\x07",
            b"\x1b]9;4;1;-1\x07",
            b"\x1b]9;4;\x07",
            b"\x1b]9;4;1;2;3\x07",
            b"\x1b]9;4;1;abc\x07",
            b"\x1b]9;4;1;99999999999\x07",
            b"\x1b]9;4;99\x07",
            b"\x1b]9;5;1;10\x07",
            b"\x1b]9;hello\x07",
        ] {
            assert_eq!(scan_all(&[bad]), vec![], "{bad:?}");
        }
    }

    #[test]
    fn a_report_cut_between_writes_is_still_read() {
        let full: &[u8] = b"ab\x1b]9;4;1;42\x07cd";
        for cut in 1..full.len() {
            let got = scan_all(&[&full[..cut], &full[cut..]]);
            assert_eq!(got, vec![p(1, 42)], "cut at {cut}");
        }
        // One byte at a time.
        let chunks: Vec<&[u8]> = full.chunks(1).collect();
        assert_eq!(scan_all(&chunks), vec![p(1, 42)]);
    }

    #[test]
    fn an_unfinished_report_does_not_stick() {
        let got = scan_all(&[b"\x1b]9;4;1;5", b"zzz plain text\x1b]9;4;1;9\x07"]);
        assert_eq!(got, vec![p(1, 9)]);
    }
}
