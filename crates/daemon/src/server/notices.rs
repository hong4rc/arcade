//! Notifications in the daemon: the signals the panes and the commands give, what looking at a
//! pane or typing into it does to the one it has, and the log of what became of each signal.
//! What the notifications are is `notify.rs`.

use std::collections::VecDeque;

use super::actions::Ctx;
use super::*;
use crate::notify::{Class, Outcome, Signal, TERMINAL_SOURCE};
use crate::picker::{Item, Picker, PickerKind};
use crate::tree::Key as TreeKey;

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
        session
            .client
            .filter(|_| session.host_focused)
            .map(|_| session.current_window().focus)
    }

    /// The pane that has the focus in the window the client shows, whether or not the host
    /// terminal is looked at.
    fn focused_pane(session: &Session) -> Option<u64> {
        session.client.map(|_| session.current_window().focus)
    }

    /// The host terminal got or lost the focus.
    pub(super) fn host_focus(&mut self, session_id: u64, focused: bool) {
        if let Some(s) = self.sessions.get_mut(&session_id)
            && s.host_focused != focused
        {
            s.host_focused = focused;
            // Looking at a pane again lets go of what it had; the screen is built to do it.
            s.relayout = true;
            s.chrome_dirty = true;
        }
    }

    /// Ring the client of a session: one bell in `BELL_INTERVAL`; a bell that comes sooner waits
    /// for the interval to end, and all that waited ring once.
    fn ring(&mut self, session_id: u64, now: Instant) {
        let Some(s) = self.sessions.get_mut(&session_id) else {
            return;
        };
        let Some(client) = s.client else {
            return;
        };
        match s.last_bell {
            Some(t) if now.duration_since(t) < crate::notify::BELL_INTERVAL => {
                s.bell_due = Some(t + crate::notify::BELL_INTERVAL);
            }
            _ => {
                s.last_bell = Some(now);
                s.bell_due = None;
                self.send_output(client, b"\x07");
            }
        }
    }

    /// The bells that waited ring now, once for each client.
    pub(super) fn release_bells(&mut self, now: Instant) {
        let due: Vec<u64> = self
            .sessions
            .values()
            .filter(|s| s.bell_due.is_some_and(|t| now >= t))
            .map(|s| s.id)
            .collect();
        for id in due {
            self.ring(id, now);
        }
    }

    /// Ask the host terminal of the client to get the person's attention for the pane's
    /// notification: its own notification and, for the classes that ask for one, a bell. Not for
    /// the pane that is looked at, and not more than once in `PUSH_INTERVAL` for a pane. What
    /// became of it, in words, for the log.
    fn push_out(&mut self, pane: u64) -> &'static str {
        let Some(n) = self.notices.get(pane) else {
            return "nothing standing";
        };
        let (class, source, title, message) = (
            n.class,
            n.source.clone(),
            n.title.clone(),
            n.message.clone(),
        );
        let Some((session_id, _)) = self.locate_pane(pane) else {
            return "no client";
        };
        let Some(session) = self.sessions.get(&session_id) else {
            return "no client";
        };
        let Some(client) = session.client else {
            return "no client to tell";
        };
        if Self::seen_pane(session) == Some(pane) {
            return "not sent out: the pane is looked at";
        }
        let now = Instant::now();
        if !self
            .notices
            .may_push(pane, now, crate::notify::PUSH_INTERVAL)
        {
            return "not sent out: this pane sent one a moment ago";
        }
        let sequence =
            crate::notify::host_sequence(source == TERMINAL_SOURCE, &source, &title, &message);
        if !sequence.is_empty() {
            self.send_output(client, &sequence);
        }
        if class != Class::Info {
            self.ring(session_id, now);
        }
        "sent out"
    }

    /// What is drawn about notifications changed: the status bar and the marks on the panes are
    /// drawn again.
    fn notices_changed(&mut self, pane: u64) {
        let owner = self.locate_pane(pane).map(|(s, _)| s);
        for s in self.sessions.values_mut() {
            // A tree that is open shows the marks of every session.
            if Some(s.id) == owner || s.tree.is_some() {
                s.relayout = true;
                s.chrome_dirty = true;
            }
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
        let result = if outcome.took_effect() {
            format!("{}; {}", outcome.describe(), self.push_out(pane))
        } else {
            outcome.describe().to_string()
        };
        self.log_signal(channel.name(), Some(class), &source, Some(pane), &result);
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
        for (pane, outcome) in self.notices.release_held(Instant::now()) {
            self.notices_changed(pane);
            if outcome.took_effect() {
                self.push_out(pane);
            }
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

    /// The session, window and pane a notification sits in, for a person to read.
    fn notice_place(&self, pane: u64) -> String {
        let Some((session_id, window_id)) = self.locate_pane(pane) else {
            return "gone".to_string();
        };
        let session = self.sessions.get(&session_id);
        let window = session
            .and_then(|s| s.window(window_id))
            .map(|w| w.name.clone());
        format!(
            "{} {} {}",
            session.map_or("?", |s| s.name.as_str()),
            window.unwrap_or_default(),
            self.pane_label(pane)
        )
    }

    /// Go to the pane whose notification is the most urgent, the one that waited longest first
    /// within a class. The pane that is looked at is skipped, so that pressing the key again goes
    /// on to the next.
    pub(super) fn notification_next(&mut self, session_id: u64) -> Result<String, String> {
        let seen = self.sessions.get(&session_id).and_then(Self::focused_pane);
        let next = self
            .notices
            .by_priority()
            .into_iter()
            .map(|(pane, _)| pane)
            .find(|pane| Some(*pane) != seen && self.locate_pane(*pane).is_some());
        match next {
            Some(pane) => self.jump_to(session_id, TreeKey::Pane(pane)),
            None if self.notices.is_empty() => Err("no notification is waiting".to_string()),
            None => Err("no other pane has a notification".to_string()),
        }
    }

    pub(super) fn notification_clear_focused(&mut self, ctx: Ctx) -> Result<String, String> {
        let had = self.notices.clear(ctx.pane);
        if had {
            self.notices_changed(ctx.pane);
        }
        self.log_signal(
            "key",
            None,
            "key",
            Some(ctx.pane),
            if had { "cleared" } else { "nothing to clear" },
        );
        Ok(String::new())
    }

    pub(super) fn notification_clear_all(&mut self) -> String {
        let panes: Vec<u64> = self.panes.keys().copied().collect();
        let mut cleared = 0;
        for pane in panes {
            if self.notices.clear(pane) {
                self.notices_changed(pane);
                cleared += 1;
            }
        }
        self.log_signal("key", None, "key", None, &format!("cleared {cleared}"));
        String::new()
    }

    /// The notifications as text, one per line with tab-separated fields: class, source, pane,
    /// seconds waited, where, title, message. The most urgent first.
    pub(super) fn notification_lines(&self) -> String {
        let now = Instant::now();
        self.notices
            .by_priority()
            .into_iter()
            .map(|(pane, n)| {
                format!(
                    "{}\t{}\t{}\t{}\t{}\t{}\t{}\n",
                    n.class.name(),
                    n.source,
                    pane,
                    now.duration_since(n.since).as_secs(),
                    self.notice_place(pane),
                    n.title,
                    n.message
                )
            })
            .collect()
    }

    /// Open the list of notifications with the line at `selected` chosen.
    pub(super) fn open_notifications(
        &mut self,
        session_id: u64,
        selected: usize,
    ) -> Result<(), String> {
        let now = Instant::now();
        let items: Vec<Item> = self
            .notices
            .by_priority()
            .into_iter()
            .map(|(pane, n)| {
                let mark = crate::theme::notice_symbol(n.class);
                let text = match (n.title.is_empty(), n.message.is_empty()) {
                    (false, false) => format!("{}: {}", n.title, n.message),
                    (false, true) => n.title.clone(),
                    (true, false) => n.message.clone(),
                    (true, true) => String::new(),
                };
                let name = format!("{mark} {}  {}", n.class.name(), n.source);
                let detail = format!(
                    "{text}  waited {}s  {}",
                    now.duration_since(n.since).as_secs(),
                    self.notice_place(pane)
                );
                Item::new(pane as usize, name, detail.clone(), false).also_found_by(&detail)
            })
            .collect();
        if items.is_empty() {
            if let Some(s) = self.sessions.get_mut(&session_id) {
                s.picker = None;
                s.relayout = true;
                s.chrome_dirty = true;
            }
            return Err("no notification is waiting".to_string());
        }
        if let Some(s) = self.sessions.get_mut(&session_id) {
            s.picker = Some(Picker::new(
                PickerKind::Notifications,
                "Notifications",
                items,
                selected,
            ));
            s.relayout = true;
            s.chrome_dirty = true;
        }
        Ok(())
    }

    /// Delete in the list: clear the chosen notification and show the list again without it.
    pub(super) fn clear_from_list(
        &mut self,
        session_id: u64,
        pane: u64,
        selected: usize,
    ) -> Result<String, String> {
        if self.notices.clear(pane) {
            self.notices_changed(pane);
        }
        self.log_signal("key", None, "key", Some(pane), "cleared");
        // The list is made again; when nothing is left it closes.
        match self.open_notifications(session_id, selected) {
            Ok(()) | Err(_) => Ok(String::new()),
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
