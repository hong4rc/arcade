//! Notifications in the daemon: the signals the panes and the commands give, what looking at a
//! pane or typing into it does to the one it has, and the log of what became of each signal.
//! What the notifications are is `notify.rs`.

use std::collections::VecDeque;

use super::*;
use crate::notify::{Class, Outcome, Signal, TERMINAL_SOURCE};

/// The version of the signal contract the daemon accepts over the socket.
const SIGNAL_VERSION: u32 = 1;

/// How many entries the log keeps.
const LOG_ENTRIES: usize = 100;

/// One line of the log: no title and no message, which may say things not meant to be kept.
pub(super) struct LogEntry {
    at: Instant,
    channel: &'static str,
    class: Option<Class>,
    source: String,
    pane: Option<u64>,
    result: String,
}

/// Where a signal came from.
#[derive(Clone, Copy, PartialEq, Eq)]
pub(super) enum Channel {
    /// A bell or an escape sequence in the pane.
    Terminal,
    /// `arcade notify` over the socket.
    Command,
}

impl Channel {
    fn name(self) -> &'static str {
        match self {
            Channel::Terminal => "terminal",
            Channel::Command => "cli",
        }
    }
}

/// The fields of a request over the socket, given as `key=value` words. Fields that are not
/// known are left out, so that a field added later does not break an older daemon.
fn fields(args: &[String]) -> HashMap<&str, &str> {
    args.iter().filter_map(|a| a.split_once('=')).collect()
}

impl Server {
    /// The pane the client of the session is looking at, when it has a client.
    fn seen_pane(session: &Session) -> Option<u64> {
        session.client.map(|_| session.current_window().focus)
    }

    /// What is drawn about notifications changed: the status bar and the marks on the panes are
    /// drawn again.
    fn notices_changed(&mut self, pane: u64) {
        if let Some((session_id, _)) = self.locate_pane(pane)
            && let Some(s) = self.sessions.get_mut(&session_id)
        {
            s.relayout = true;
            s.chrome_dirty = true;
        }
    }

    fn log_signal(
        &mut self,
        channel: &'static str,
        class: Option<Class>,
        source: &str,
        pane: Option<u64>,
        result: &str,
    ) {
        if self.notice_log.len() >= LOG_ENTRIES {
            self.notice_log.pop_front();
        }
        self.notice_log.push_back(LogEntry {
            at: Instant::now(),
            channel,
            class,
            source: source.to_string(),
            pane,
            result: result.to_string(),
        });
    }

    /// A signal for a pane, from its program or from a command.
    pub(super) fn signal_pane(&mut self, pane: u64, signal: Signal, channel: Channel) -> Outcome {
        let (class, source) = (signal.class, signal.source.clone());
        let outcome = self.take_signal(pane, signal);
        self.log_signal(
            channel.name(),
            Some(class),
            &source,
            Some(pane),
            outcome.describe(),
        );
        outcome
    }

    fn take_signal(&mut self, pane: u64, signal: Signal) -> Outcome {
        let Some((session_id, _)) = self.locate_pane(pane) else {
            return Outcome::Blocked;
        };
        let seen = self
            .sessions
            .get(&session_id)
            .and_then(Self::seen_pane)
            .is_some_and(|p| p == pane);
        // A class that goes away at a glance is not worth keeping for a pane that is looked at.
        if signal.class.clears_when_seen() && seen {
            return Outcome::Blocked;
        }
        let outcome = self.notices.signal(pane, signal, Instant::now());
        if outcome.took_effect() {
            self.notices_changed(pane);
        }
        outcome
    }

    /// Before a session's screen is built: let go of what the client has looked at, and bring in
    /// the signals that waited for their second to end.
    pub(super) fn settle_notices(&mut self, session_id: u64) {
        if self.notices.is_empty() {
            return;
        }
        for (pane, _) in self.notices.release_held(Instant::now()) {
            self.notices_changed(pane);
        }
        let seen = self.sessions.get(&session_id).and_then(Self::seen_pane);
        if let Some(pane) = seen
            && self
                .notices
                .get(pane)
                .is_some_and(|n| n.class.clears_when_seen())
        {
            self.notices.clear(pane);
            self.notices_changed(pane);
        }
    }

    /// The user typed into a pane: whatever it was waiting for, it has it.
    pub(super) fn typed_into(&mut self, pane: u64) {
        if self.notices.clear(pane) {
            self.notices_changed(pane);
        }
    }

    /// `arcade notify`: a request over the socket. The fields are `version`, `target` (a pane),
    /// `class`, `title`, `message` and `source`.
    pub(super) fn notify_send(&mut self, args: &[String]) -> (bool, String) {
        let f = fields(args);
        let source = f.get("source").copied().unwrap_or("cli");
        let source = crate::notify::sanitize(source, crate::notify::SOURCE_LIMIT);
        let source = if source.is_empty() {
            "cli".to_string()
        } else {
            source
        };
        let refuse = |server: &mut Server, class: Option<Class>, pane: Option<u64>, why: String| {
            server.log_signal("cli", class, &source, pane, &format!("rejected: {why}"));
            (false, why)
        };
        let version = f.get("version").and_then(|v| v.parse::<u32>().ok());
        match version {
            Some(v) if v > SIGNAL_VERSION => {
                let why = format!(
                    "version {v} is not supported; this daemon supports version {SIGNAL_VERSION}"
                );
                return refuse(self, None, None, why);
            }
            Some(_) => {}
            None => return refuse(self, None, None, "no version given".to_string()),
        }
        let class = match f.get("class") {
            None => Class::Info,
            Some(text) => match Class::parse(text) {
                Some(c) => c,
                None => {
                    let why = format!("unknown class {text:?}: use info, input, approval or error");
                    return refuse(self, None, None, why);
                }
            },
        };
        let Some(pane) = f.get("target").and_then(|t| t.parse::<u64>().ok()) else {
            return refuse(self, Some(class), None, "no target pane given".to_string());
        };
        if self.locate_pane(pane).is_none() {
            return refuse(self, Some(class), Some(pane), "no such pane".to_string());
        }
        let signal = Signal::new(
            class,
            &source,
            f.get("title").copied().unwrap_or(""),
            f.get("message").copied().unwrap_or(""),
        );
        let outcome = self.signal_pane(pane, signal, Channel::Command);
        (true, outcome.describe().to_string())
    }

    /// `arcade notify clear`: the notification of one pane (`target`), or of every pane
    /// (`all=1`).
    pub(super) fn notify_clear(&mut self, args: &[String]) -> (bool, String) {
        let f = fields(args);
        if f.get("all").is_some_and(|v| *v == "1") {
            let panes: Vec<u64> = self.panes.keys().copied().collect();
            let mut cleared = 0;
            for pane in panes {
                if self.notices.clear(pane) {
                    self.notices_changed(pane);
                    cleared += 1;
                }
            }
            self.log_signal("cli", None, "cli", None, &format!("cleared {cleared}"));
            return (true, format!("cleared {cleared}"));
        }
        let Some(pane) = f.get("target").and_then(|t| t.parse::<u64>().ok()) else {
            return (false, "no target pane given".to_string());
        };
        if self.locate_pane(pane).is_none() {
            return (false, "no such pane".to_string());
        }
        let had = self.notices.clear(pane);
        if had {
            self.notices_changed(pane);
        }
        let result = if had { "cleared" } else { "nothing to clear" };
        self.log_signal("cli", None, "cli", Some(pane), result);
        (true, result.to_string())
    }

    /// `arcade notify log`: the latest signals, oldest first, one per line with tab-separated
    /// fields: seconds ago, channel, class, source, pane, result.
    pub(super) fn notify_log(&self) -> (bool, String) {
        let now = Instant::now();
        let lines: Vec<String> = self
            .notice_log
            .iter()
            .map(|e| {
                format!(
                    "{}\t{}\t{}\t{}\t{}\t{}",
                    now.duration_since(e.at).as_secs(),
                    e.channel,
                    e.class.map_or("-", Class::name),
                    if e.source.is_empty() {
                        TERMINAL_SOURCE
                    } else {
                        &e.source
                    },
                    e.pane.map_or_else(|| "-".to_string(), |p| p.to_string()),
                    e.result
                )
            })
            .collect();
        (
            true,
            lines.join("\n") + if lines.is_empty() { "" } else { "\n" },
        )
    }
}

pub(super) fn new_log() -> VecDeque<LogEntry> {
    VecDeque::new()
}
