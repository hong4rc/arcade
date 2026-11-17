//! The marks a shell prompt leaves in a pane's output (OSC 133): where a prompt starts (`A`),
//! where what is typed starts (`B`), where the command starts to run (`C`) and where it ends
//! with its exit code (`D`). A pane whose shell sends none never has any of them. Nothing here
//! knows which program is running: only the marks are read.

use std::time::{Duration, Instant};

/// One mark.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Mark {
    PromptStart,
    InputStart,
    CommandStart,
    /// The command ended; the exit code when the shell gave one.
    CommandEnd(Option<i32>),
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
enum State {
    #[default]
    Ground,
    Escape,
    Osc,
    OscEscape,
}

/// The longest payload of an OSC that is kept; a longer one (a long title) is not a mark.
const PAYLOAD_LIMIT: usize = 32;

/// Finds the marks in a pane's output as it comes, across the cuts between reads.
#[derive(Debug, Default)]
pub struct Scanner {
    state: State,
    payload: Vec<u8>,
    too_long: bool,
}

impl Scanner {
    /// Add `bytes`, and push the marks they hold on `out`.
    pub fn feed(&mut self, bytes: &[u8], out: &mut Vec<Mark>) {
        // Most output holds no escape at all: nothing to find while none is under way.
        if self.state == State::Ground && !bytes.contains(&0x1b) {
            return;
        }
        for &b in bytes {
            match self.state {
                State::Ground => {
                    if b == 0x1b {
                        self.state = State::Escape;
                    }
                }
                State::Escape => {
                    self.state = match b {
                        b']' => {
                            self.payload.clear();
                            self.too_long = false;
                            State::Osc
                        }
                        0x1b => State::Escape,
                        _ => State::Ground,
                    };
                }
                State::Osc => match b {
                    0x07 => self.finish(out),
                    0x1b => self.state = State::OscEscape,
                    _ => {
                        if self.payload.len() < PAYLOAD_LIMIT {
                            self.payload.push(b);
                        } else {
                            self.too_long = true;
                        }
                    }
                },
                State::OscEscape => {
                    if b == b'\\' {
                        self.finish(out);
                    } else {
                        self.state = if b == 0x1b {
                            State::Escape
                        } else {
                            State::Ground
                        };
                    }
                }
            }
        }
    }

    fn finish(&mut self, out: &mut Vec<Mark>) {
        self.state = State::Ground;
        if self.too_long {
            return;
        }
        let Some(rest) = self.payload.strip_prefix(b"133;") else {
            return;
        };
        let mut parts = rest.split(|b| *b == b';');
        let mark = match parts.next() {
            Some(b"A") => Mark::PromptStart,
            Some(b"B") => Mark::InputStart,
            Some(b"C") => Mark::CommandStart,
            Some(b"D") => Mark::CommandEnd(
                parts
                    .next()
                    .and_then(|c| std::str::from_utf8(c).ok())
                    .and_then(|c| c.parse().ok()),
            ),
            _ => return,
        };
        out.push(mark);
    }
}

/// What a pane's shell says about the command that runs in it.
#[derive(Debug, Default)]
pub struct Track {
    pub scanner: Scanner,
    /// When the command that runs now started (mark `C`), until its end.
    pub running_since: Option<Instant>,
    /// The running mark has been shown on the tab (the time it takes has passed).
    pub running_shown: bool,
    /// How a command that took long ended while nobody looked at the pane: `true` for exit code
    /// 0. Stays until the pane is looked at, a key is typed into it, or another command starts.
    pub finished: Option<bool>,
}

/// The two times that decide what a pane shows; zero turns a state off.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Thresholds {
    /// How long a command runs before the tab says so.
    pub running_after: Duration,
    /// How long a command has to run for its end to be left on the tab of a pane out of sight.
    pub finished_after: Duration,
}

impl Track {
    /// A mark came at `now`; `seen` is whether the pane is looked at. Returns whether what the
    /// tab shows changed.
    pub fn mark(&mut self, mark: Mark, now: Instant, seen: bool, limits: Thresholds) -> bool {
        match mark {
            Mark::CommandStart => {
                let changed = self.finished.take().is_some();
                self.running_since = Some(now);
                self.running_shown = false;
                changed
            }
            Mark::CommandEnd(code) => self.end(now, Some(code.unwrap_or(0) == 0), seen, limits),
            // A prompt after a command that gave no end mark ends it too, with no result.
            Mark::PromptStart => self.end(now, None, seen, limits),
            Mark::InputStart => false,
        }
    }

    fn end(&mut self, now: Instant, ok: Option<bool>, seen: bool, limits: Thresholds) -> bool {
        let Some(since) = self.running_since.take() else {
            return false;
        };
        let shown = std::mem::take(&mut self.running_shown);
        let long = !limits.finished_after.is_zero()
            && now.saturating_duration_since(since) >= limits.finished_after;
        if let (true, false, Some(ok)) = (long, seen, ok) {
            self.finished = Some(ok);
            return true;
        }
        shown
    }

    /// Whether the running mark shows at `now`.
    pub fn is_running(&self, now: Instant, limits: Thresholds) -> bool {
        !limits.running_after.is_zero()
            && self
                .running_since
                .is_some_and(|t| now.saturating_duration_since(t) >= limits.running_after)
    }
}

/// How long, in words a list can carry: `45s`, `1m20s`, `2h05m`.
pub fn duration_words(d: Duration) -> String {
    let s = d.as_secs();
    match s {
        0..=59 => format!("{s}s"),
        60..=3599 => format!("{}m{:02}s", s / 60, s % 60),
        _ => format!("{}h{:02}m", s / 3600, s % 3600 / 60),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn marks(chunks: &[&[u8]]) -> Vec<Mark> {
        let mut scanner = Scanner::default();
        let mut out = Vec::new();
        for c in chunks {
            scanner.feed(c, &mut out);
        }
        out
    }

    const LIMITS: Thresholds = Thresholds {
        running_after: Duration::from_secs(2),
        finished_after: Duration::from_secs(10),
    };

    #[test]
    fn the_four_marks_are_found_with_either_ending_and_the_exit_code() {
        let found = marks(&[
            b"\x1b]133;A\x1b\\$ \x1b]133;B\x07ls\r\n\x1b]133;C\x1b\\out\x1b]133;D;3\x07\x1b]133;D\x1b\\",
        ]);
        assert_eq!(
            found,
            [
                Mark::PromptStart,
                Mark::InputStart,
                Mark::CommandStart,
                Mark::CommandEnd(Some(3)),
                Mark::CommandEnd(None)
            ]
        );
    }

    #[test]
    fn a_mark_cut_between_two_reads_is_still_found_and_other_sequences_are_not_marks() {
        assert_eq!(
            marks(&[b"x\x1b", b"]13", b"3;C\x1b", b"\\"]),
            [Mark::CommandStart]
        );
        // A title, a colour query, a long OSC and a bare escape are no marks.
        let long = format!("\x1b]0;{}\x07", "t".repeat(200));
        assert!(
            marks(&[
                b"\x1b]0;title\x07\x1b[31m\x1b]133;X\x07",
                long.as_bytes(),
                b"\x1b\x1b]133\x07"
            ])
            .is_empty()
        );
        // A mark right after a long OSC still counts.
        assert_eq!(
            marks(&[long.as_bytes(), b"\x1b]133;C\x07"]),
            [Mark::CommandStart]
        );
    }

    #[test]
    fn a_command_that_ran_long_out_of_sight_leaves_its_result_and_a_short_one_nothing() {
        let t0 = Instant::now();
        let at = |s| t0 + Duration::from_secs(s);
        let mut track = Track::default();
        track.mark(Mark::CommandStart, at(0), false, LIMITS);
        assert!(!track.is_running(at(1), LIMITS));
        assert!(track.is_running(at(2), LIMITS));
        track.mark(Mark::CommandEnd(Some(0)), at(12), false, LIMITS);
        assert_eq!(track.finished, Some(true));
        // The next command starts clean; a failure is told apart; a short one leaves nothing.
        track.mark(Mark::CommandStart, at(20), false, LIMITS);
        assert_eq!(track.finished, None);
        track.mark(Mark::CommandEnd(Some(1)), at(40), false, LIMITS);
        assert_eq!(track.finished, Some(false));
        track.mark(Mark::CommandStart, at(50), false, LIMITS);
        track.mark(Mark::CommandEnd(Some(1)), at(52), false, LIMITS);
        assert_eq!(track.finished, None);
        // One that ends while the pane is looked at leaves nothing either.
        track.mark(Mark::CommandStart, at(60), true, LIMITS);
        track.mark(Mark::CommandEnd(Some(0)), at(100), true, LIMITS);
        assert_eq!(track.finished, None);
        // A prompt with no end mark ends the running state and leaves no result.
        track.mark(Mark::CommandStart, at(110), false, LIMITS);
        track.mark(Mark::PromptStart, at(150), false, LIMITS);
        assert!(track.running_since.is_none() && track.finished.is_none());
    }

    #[test]
    fn zero_turns_each_state_off() {
        let t0 = Instant::now();
        let off = Thresholds {
            running_after: Duration::ZERO,
            finished_after: Duration::ZERO,
        };
        let mut track = Track::default();
        track.mark(Mark::CommandStart, t0, false, off);
        assert!(!track.is_running(t0 + Duration::from_secs(99), off));
        track.mark(
            Mark::CommandEnd(Some(1)),
            t0 + Duration::from_secs(99),
            false,
            off,
        );
        assert_eq!(track.finished, None);
    }

    #[test]
    fn durations_are_written_in_words() {
        assert_eq!(duration_words(Duration::from_secs(45)), "45s");
        assert_eq!(duration_words(Duration::from_secs(80)), "1m20s");
        assert_eq!(duration_words(Duration::from_secs(7500)), "2h05m");
    }
}
