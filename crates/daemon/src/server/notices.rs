//! Notifications in the daemon: the signals the panes give, and what looking at a pane or typing
//! into it does to the one it has. What the notifications are is `notify.rs`.

use super::*;
use crate::notify::Signal;

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

    /// A signal for a pane, from its program or from a command.
    pub(super) fn signal_pane(&mut self, pane: u64, signal: Signal) {
        let Some((session_id, _)) = self.locate_pane(pane) else {
            return;
        };
        let seen = self
            .sessions
            .get(&session_id)
            .and_then(Self::seen_pane)
            .is_some_and(|p| p == pane);
        // A class that goes away at a glance is not worth keeping for a pane that is looked at.
        if signal.class.clears_when_seen() && seen {
            return;
        }
        let outcome = self.notices.signal(pane, signal, Instant::now());
        if matches!(
            outcome,
            crate::notify::Outcome::Created | crate::notify::Outcome::Replaced
        ) {
            self.notices_changed(pane);
        }
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
}
