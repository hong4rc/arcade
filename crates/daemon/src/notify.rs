//! What the notifications are: one at most for each pane, in one of four classes, and the rules
//! for which signal replaces which. Nothing here knows about panes' windows, clients or the
//! screen; the server asks and acts on the answers.

use std::collections::HashMap;
use std::time::{Duration, Instant};

/// The class of a notification, from the least urgent to the most (so that `>=` reads as "is at
/// least as urgent as").
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub enum Class {
    Info,
    Error,
    Input,
    Approval,
}

impl Class {
    /// Every class, the most urgent first.
    pub const BY_URGENCY: [Class; 4] = [Class::Approval, Class::Input, Class::Error, Class::Info];

    #[cfg_attr(not(test), allow(dead_code))]
    pub fn name(self) -> &'static str {
        match self {
            Class::Info => "info",
            Class::Error => "error",
            Class::Input => "input",
            Class::Approval => "approval",
        }
    }

    #[cfg_attr(not(test), allow(dead_code))]
    pub fn parse(text: &str) -> Option<Class> {
        Class::BY_URGENCY.into_iter().find(|c| c.name() == text)
    }

    /// Looking at the pane is enough for this class: it goes away by itself.
    pub fn clears_when_seen(self) -> bool {
        self == Class::Info
    }
}

/// The longest title, message and source, in characters.
pub const TITLE_LIMIT: usize = 64;
pub const MESSAGE_LIMIT: usize = 256;
pub const SOURCE_LIMIT: usize = 32;

/// More signals than this for one pane in a second: the rest wait, and only the last of them
/// counts.
pub const SIGNALS_PER_SECOND: u32 = 20;

/// Text from a pane or from a command is not to be trusted: no control character of any kind
/// (so no escape sequence can start), and no longer than the limit.
pub fn sanitize(text: &str, limit: usize) -> String {
    text.chars()
        .filter(|c| !c.is_control())
        .take(limit)
        .collect()
}

/// Where a signal came from.
pub const TERMINAL_SOURCE: &str = "terminal";

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Signal {
    pub class: Class,
    pub source: String,
    pub title: String,
    pub message: String,
}

impl Signal {
    /// A signal with its texts made safe and cut to the limits.
    pub fn new(class: Class, source: &str, title: &str, message: &str) -> Signal {
        Signal {
            class,
            source: sanitize(source, SOURCE_LIMIT),
            title: sanitize(title, TITLE_LIMIT),
            message: sanitize(message, MESSAGE_LIMIT),
        }
    }
}

#[derive(Clone, Debug)]
// The texts and the time are read by the list of notifications, which comes next.
#[allow(dead_code)]
pub struct Notification {
    pub class: Class,
    pub source: String,
    pub title: String,
    pub message: String,
    /// When the signal that is standing now arrived.
    pub since: Instant,
}

/// What became of a signal.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Outcome {
    /// The pane had none; it has one now.
    Created,
    /// It took the place of the pane's notification.
    Replaced,
    /// Dropped: another source's notification of a higher class stands.
    Superseded,
    /// Dropped: too many signals for this pane within the second; the last one is kept for later.
    RateLimited,
}

#[derive(Default)]
pub struct Store {
    standing: HashMap<u64, Notification>,
    /// For each pane, when the current second began and how many signals it saw since.
    rates: HashMap<u64, (Instant, u32)>,
    /// The last signal that was over the limit, for each pane.
    held: HashMap<u64, Signal>,
}

impl Store {
    pub fn get(&self, pane: u64) -> Option<&Notification> {
        self.standing.get(&pane)
    }

    pub fn is_empty(&self) -> bool {
        self.standing.is_empty()
    }

    /// A signal for a pane.
    pub fn signal(&mut self, pane: u64, signal: Signal, now: Instant) -> Outcome {
        let rate = self.rates.entry(pane).or_insert((now, 0));
        if now.duration_since(rate.0) >= Duration::from_secs(1) {
            *rate = (now, 0);
        }
        rate.1 += 1;
        if rate.1 > SIGNALS_PER_SECOND {
            self.held.insert(pane, signal);
            return Outcome::RateLimited;
        }
        self.apply(pane, signal, now)
    }

    fn apply(&mut self, pane: u64, signal: Signal, now: Instant) -> Outcome {
        let outcome = match self.standing.get(&pane) {
            None => Outcome::Created,
            Some(current) if current.source == signal.source || signal.class >= current.class => {
                Outcome::Replaced
            }
            Some(_) => return Outcome::Superseded,
        };
        self.standing.insert(
            pane,
            Notification {
                class: signal.class,
                source: signal.source,
                title: signal.title,
                message: signal.message,
                since: now,
            },
        );
        outcome
    }

    /// The signals that were held back, once the second they were held in is over: each pane's
    /// last one counts now. Returns the panes it changed, with what became of the signal.
    pub fn release_held(&mut self, now: Instant) -> Vec<(u64, Outcome)> {
        let due: Vec<u64> = self
            .held
            .keys()
            .copied()
            .filter(|p| {
                self.rates
                    .get(p)
                    .is_none_or(|(start, _)| now.duration_since(*start) >= Duration::from_secs(1))
            })
            .collect();
        let mut done = Vec::new();
        for pane in due {
            if let Some(signal) = self.held.remove(&pane) {
                self.rates.insert(pane, (now, 1));
                done.push((pane, self.apply(pane, signal, now)));
            }
        }
        done
    }

    /// Clear the pane's notification: true when there was one.
    pub fn clear(&mut self, pane: u64) -> bool {
        self.held.remove(&pane);
        self.standing.remove(&pane).is_some()
    }

    /// Clear every notification: how many there were.
    #[cfg_attr(not(test), allow(dead_code))]
    pub fn clear_all(&mut self) -> usize {
        self.held.clear();
        let n = self.standing.len();
        self.standing.clear();
        n
    }

    /// The pane is gone: nothing of it is kept.
    pub fn forget(&mut self, pane: u64) {
        self.clear(pane);
        self.rates.remove(&pane);
    }

    /// How many of these panes have a notification, for each class in the order of
    /// `Class::BY_URGENCY`. Panes are counted, not signals.
    pub fn counts(&self, panes: impl IntoIterator<Item = u64>) -> [usize; 4] {
        let mut counts = [0; 4];
        for pane in panes {
            if let Some(n) = self.standing.get(&pane)
                && let Some(i) = Class::BY_URGENCY.iter().position(|c| *c == n.class)
            {
                counts[i] += 1;
            }
        }
        counts
    }

    /// The most urgent class among these panes.
    pub fn highest(&self, panes: impl IntoIterator<Item = u64>) -> Option<Class> {
        panes
            .into_iter()
            .filter_map(|p| self.standing.get(&p).map(|n| n.class))
            .max()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn at(secs: u64, base: Instant) -> Instant {
        base + Duration::from_secs(secs)
    }

    fn sig(class: Class, source: &str) -> Signal {
        Signal::new(class, source, "t", "m")
    }

    #[test]
    fn a_signal_from_the_terminal_does_not_overwrite_a_higher_class_from_another_source() {
        let now = Instant::now();
        let mut s = Store::default();
        assert_eq!(
            s.signal(1, sig(Class::Approval, "cli"), now),
            Outcome::Created
        );
        assert_eq!(
            s.signal(1, sig(Class::Info, TERMINAL_SOURCE), now),
            Outcome::Superseded
        );
        assert_eq!(s.get(1).unwrap().class, Class::Approval);
        assert_eq!(s.get(1).unwrap().source, "cli");
    }

    #[test]
    fn the_same_source_always_replaces_and_another_source_needs_an_equal_or_higher_class() {
        let now = Instant::now();
        let mut s = Store::default();
        s.signal(1, sig(Class::Approval, "a"), now);
        // The same source says its state is lower now: it goes down.
        assert_eq!(s.signal(1, sig(Class::Info, "a"), now), Outcome::Replaced);
        assert_eq!(s.get(1).unwrap().class, Class::Info);
        // Another source at an equal class replaces; at a lower class it does not.
        assert_eq!(s.signal(1, sig(Class::Info, "b"), now), Outcome::Replaced);
        s.signal(1, sig(Class::Input, "b"), now);
        assert_eq!(
            s.signal(1, sig(Class::Error, "c"), now),
            Outcome::Superseded
        );
        assert_eq!(s.signal(1, sig(Class::Input, "c"), now), Outcome::Replaced);
        assert_eq!(s.get(1).unwrap().source, "c");
    }

    #[test]
    fn the_classes_are_ordered_from_info_to_approval() {
        assert!(Class::Approval > Class::Input);
        assert!(Class::Input > Class::Error);
        assert!(Class::Error > Class::Info);
        assert_eq!(Class::parse("input"), Some(Class::Input));
        assert_eq!(Class::parse("urgent"), None);
        assert!(Class::Info.clears_when_seen() && !Class::Error.clears_when_seen());
    }

    #[test]
    fn texts_lose_every_control_character_and_are_cut_to_their_limits() {
        let s = Signal::new(
            Class::Info,
            "so\x1burce",
            "a\x1b[31mb\x07",
            &"x".repeat(400),
        );
        assert_eq!(s.source, "source");
        assert_eq!(s.title, "a[31mb");
        assert_eq!(s.message.chars().count(), MESSAGE_LIMIT);
        assert_eq!(sanitize("\u{9b}c\x7f", 10), "c");
    }

    #[test]
    fn counts_are_of_panes_and_highest_is_the_most_urgent() {
        let now = Instant::now();
        let mut s = Store::default();
        for _ in 0..5 {
            s.signal(1, sig(Class::Info, "a"), now);
        }
        s.signal(2, sig(Class::Approval, "a"), now);
        s.signal(3, sig(Class::Approval, "a"), now);
        s.signal(4, sig(Class::Error, "a"), now);
        assert_eq!(s.counts([1, 2, 3, 4, 5]), [2, 0, 1, 1]);
        assert_eq!(s.counts([1]), [0, 0, 0, 1]);
        assert_eq!(s.highest([1, 4]), Some(Class::Error));
        assert_eq!(s.highest([5]), None);
    }

    #[test]
    fn clearing_and_forgetting_remove_what_stands() {
        let now = Instant::now();
        let mut s = Store::default();
        s.signal(1, sig(Class::Info, "a"), now);
        s.signal(2, sig(Class::Info, "a"), now);
        assert!(s.clear(1));
        assert!(!s.clear(1));
        s.forget(2);
        assert!(s.is_empty());
        s.signal(3, sig(Class::Info, "a"), now);
        s.signal(4, sig(Class::Input, "a"), now);
        assert_eq!(s.clear_all(), 2);
        assert!(s.is_empty());
    }

    #[test]
    fn past_twenty_signals_a_second_only_the_last_is_kept_for_the_next_second() {
        let base = Instant::now();
        let mut s = Store::default();
        for i in 0..SIGNALS_PER_SECOND {
            let out = s.signal(1, Signal::new(Class::Info, "a", &format!("n{i}"), ""), base);
            assert_ne!(out, Outcome::RateLimited);
        }
        for i in 0..5 {
            let out = s.signal(
                1,
                Signal::new(Class::Info, "a", &format!("late{i}"), ""),
                base,
            );
            assert_eq!(out, Outcome::RateLimited);
        }
        assert_eq!(
            s.get(1).unwrap().title,
            format!("n{}", SIGNALS_PER_SECOND - 1)
        );
        // Not yet: the second is not over.
        assert!(s.release_held(at(0, base)).is_empty());
        let done = s.release_held(at(2, base));
        assert_eq!(done, vec![(1, Outcome::Replaced)]);
        assert_eq!(s.get(1).unwrap().title, "late4");
        assert!(s.release_held(at(4, base)).is_empty());
    }
}
