//! The tree of sessions, windows and panes in the daemon: making its lines from the daemon's
//! books, what its keys do, and going to what was chosen.

use super::actions::Ctx;
use super::*;
use crate::commands::ListCommand;
use crate::layout::Axis;
use crate::tree::{Action, Entry, Key as TreeKey, Tree};

/// How the tree opens.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum TreeStart {
    /// To look and to jump, with the choice on the pane the client is in.
    Browse,
    /// With the pane the client is in lifted, the choice on its window.
    LiftPane,
    /// With the window the client is in lifted, the choice on its session.
    LiftWindow,
}

impl Server {
    /// What the tree shows now: every session (by name), its windows, and their panes.
    fn tree_entries(&self, viewing: u64) -> Vec<Entry> {
        let mut sessions: Vec<&Session> = self.sessions.values().collect();
        sessions.sort_by(|a, b| a.name.cmp(&b.name));
        let mut entries = Vec::new();
        for s in sessions {
            let here = s.id == viewing;
            let session_at = entries.len();
            entries.push(Entry {
                key: TreeKey::Session(s.id),
                label: s.name.clone(),
                parent: None,
                here,
                inner: Some(TreeKey::Window(s.current)),
                room: false,
            });
            for w in &s.windows {
                let window_here = here && w.id == s.current;
                let window_at = entries.len();
                entries.push(Entry {
                    key: TreeKey::Window(w.id),
                    label: format!("{} {}", w.number, w.name),
                    parent: Some(session_at),
                    here: window_here,
                    inner: Some(TreeKey::Pane(w.focus)),
                    room: false,
                });
                for (pane, _) in w.layout.rects() {
                    let mut probe = w.layout.clone();
                    entries.push(Entry {
                        key: TreeKey::Pane(pane),
                        label: self.pane_label(pane),
                        parent: Some(window_at),
                        here: window_here && pane == w.focus,
                        inner: None,
                        room: probe.split(pane, Axis::LeftRight, u64::MAX).is_ok(),
                    });
                }
            }
        }
        entries
    }

    /// The title the program in the pane set, or the name of the program.
    fn pane_label(&self, pane: u64) -> String {
        let Some(p) = self.panes.get(&pane) else {
            return "pane".to_string();
        };
        p.title
            .clone()
            .or_else(|| pty::process_name(p.pid))
            .unwrap_or_else(|| "pane".to_string())
    }

    /// Open the tree for the client of the session.
    pub(super) fn open_tree(&mut self, session_id: u64, start: TreeStart) -> Result<(), String> {
        let session = self
            .sessions
            .get(&session_id)
            .ok_or_else(|| "no such session".to_string())?;
        let window = session.current_window();
        let (window_key, pane_key) = (TreeKey::Window(window.id), TreeKey::Pane(window.focus));
        let (select, lifted) = match start {
            TreeStart::Browse => (pane_key, None),
            TreeStart::LiftPane => (window_key, Some(pane_key)),
            TreeStart::LiftWindow => (TreeKey::Session(session_id), Some(window_key)),
        };
        let tree = Tree::new(self.tree_entries(session_id), select, lifted);
        if let Some(s) = self.sessions.get_mut(&session_id) {
            s.tree = Some(tree);
            s.relayout = true;
            s.chrome_dirty = true;
        }
        Ok(())
    }

    /// Bring the open tree up to date with what exists.
    pub(super) fn refresh_tree(&mut self, session_id: u64) {
        if !self
            .sessions
            .get(&session_id)
            .is_some_and(|s| s.tree.is_some())
        {
            return;
        }
        let entries = self.tree_entries(session_id);
        if let Some(tree) = self
            .sessions
            .get_mut(&session_id)
            .and_then(|s| s.tree.as_mut())
        {
            tree.refresh(entries);
        }
    }

    /// A key of the list, for the open tree.
    pub(super) fn tree_command(
        &mut self,
        session_id: u64,
        command: ListCommand,
    ) -> Result<String, String> {
        self.refresh_tree(session_id);
        let session = self
            .sessions
            .get_mut(&session_id)
            .ok_or_else(|| "no such session".to_string())?;
        let Some(tree) = session.tree.as_mut() else {
            return Err("no list is open".to_string());
        };
        tree.notice = None;
        // The box changes with what it shows: what was under it is drawn again.
        session.relayout = true;
        session.chrome_dirty = true;
        match command {
            ListCommand::Up => tree.up(),
            ListCommand::Down => tree.down(),
            ListCommand::Left => tree.left(),
            ListCommand::Right => tree.right(),
            ListCommand::Lift => tree.toggle_lift(),
            ListCommand::FilterDelete => tree.delete_char(),
            ListCommand::FilterClear => tree.clear_filter(),
            ListCommand::Cancel => {
                if !tree.put_down() {
                    session.tree = None;
                }
            }
            ListCommand::Select => match tree.plan() {
                Err(why) => tree.notice = Some(format!("Enter does nothing: {why}")),
                Ok((action, _)) => {
                    session.tree = None;
                    return self.do_tree_action(session_id, action);
                }
            },
        }
        Ok(String::new())
    }

    fn do_tree_action(&mut self, session_id: u64, action: Action) -> Result<String, String> {
        match action {
            Action::Jump(key) => self.jump_to(session_id, key),
            _ => Err("moving is not done yet".to_string()),
        }
    }

    /// Go to a session, a window or a pane: the client changes session when it has to.
    fn jump_to(&mut self, from: u64, key: TreeKey) -> Result<String, String> {
        let (target, window, pane) = match key {
            TreeKey::Session(s) => (s, None, None),
            TreeKey::Window(w) => {
                let s = self
                    .sessions
                    .values()
                    .find(|s| s.window(w).is_some())
                    .map(|s| s.id)
                    .ok_or_else(|| "that window is gone".to_string())?;
                (s, Some(w), None)
            }
            TreeKey::Pane(p) => {
                let (s, w) = self
                    .locate_pane(p)
                    .ok_or_else(|| "that pane is gone".to_string())?;
                (s, Some(w), Some(p))
            }
        };
        if !self.sessions.contains_key(&target) {
            return Err("that session is gone".to_string());
        }
        if let Some(window) = window {
            self.select_window(target, window);
            if let Some(pane) = pane {
                let ctx = Ctx {
                    session: target,
                    window,
                    pane,
                };
                let was = self
                    .sessions
                    .get(&target)
                    .and_then(|s| s.window(window))
                    .map(|w| (w.focus, w.zoomed));
                self.set_focus(ctx, pane);
                if was.is_some_and(|(focus, zoomed)| zoomed && focus != pane) {
                    self.unzoom(ctx);
                }
            }
        }
        if let Some(s) = self.sessions.get_mut(&from) {
            s.relayout = true;
            s.chrome_dirty = true;
        }
        self.switch_session_id(from, target)
    }
}
