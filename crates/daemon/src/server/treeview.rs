//! The tree of sessions, windows and panes in the daemon: making its lines from the daemon's
//! books, what its keys do, and going to what was chosen.

use super::actions::Ctx;
use super::actions::{add_window, add_window_after};
use super::*;
use crate::commands::ListCommand;
use crate::layout::{Axis, Layout, LayoutError};
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

/// Where a lifted pane is put down.
#[derive(Clone, Copy)]
enum PaneDest {
    /// To the right of this pane, which gives up half its room.
    Beside(u64),
    /// In a new window at the end of this session.
    NewWindow(u64),
}

/// Where a lifted window is put down.
#[derive(Clone, Copy)]
enum WindowDest {
    /// Right after this window, in its session.
    After(u64),
    /// At the end of this session.
    End(u64),
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
            let session_notice = self.notices.highest(
                s.windows
                    .iter()
                    .flat_map(|w| w.layout.rects().into_iter().map(|(p, _)| p)),
            );
            entries.push(Entry {
                notice: session_notice,
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
                    notice: self
                        .notices
                        .highest(w.layout.rects().into_iter().map(|(p, _)| p)),
                    key: TreeKey::Window(w.id),
                    label: format!(
                        "{} {}{}",
                        w.number,
                        super::frame::tab_text(&self.config, &self.panes, w)
                            .into_iter()
                            .map(|p| p.text)
                            .collect::<String>(),
                        // What the tab shows as one symbol, the list says in words.
                        self.panes
                            .get(&w.focus)
                            .and_then(|p| p.progress)
                            .or_else(|| {
                                w.layout
                                    .rects()
                                    .into_iter()
                                    .find_map(|(p, _)| self.panes.get(&p).and_then(|p| p.progress))
                            })
                            .map_or(String::new(), |r| format!(
                                " ({})",
                                crate::theme::progress_text(r)
                            ))
                            + &w.layout
                                .rects()
                                .into_iter()
                                .find_map(|(p, _)| {
                                    self.panes.get(&p).and_then(|p| self.command_words(p))
                                })
                                .map_or(String::new(), |words| format!(" ({words})"))
                    ),
                    parent: Some(session_at),
                    here: window_here,
                    inner: Some(TreeKey::Pane(w.focus)),
                    room: false,
                });
                for (pane, _) in w.layout.rects() {
                    let mut probe = w.layout.clone();
                    entries.push(Entry {
                        notice: self.notices.get(pane).map(|n| n.class),
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

    /// What a pane is called in the tree: the title the program set, or the name of the
    /// program the pane started; then where it stands (the last part of its directory), and
    /// the program in the foreground when that is not the first one. Several panes that run the
    /// same shell tell apart by these.
    pub(super) fn pane_label(&self, pane: u64) -> String {
        let Some(p) = self.panes.get(&pane) else {
            return "pane".to_string();
        };
        let base = p
            .title
            .clone()
            .or_else(|| pty::process_name(p.pid))
            .unwrap_or_else(|| "pane".to_string());
        let mut label = base.clone();
        if let Some(dir) = pty::process_cwd(p.pid) {
            label.push_str(" in ");
            label.push_str(&directory_name(&dir));
        }
        if let Some(running) = pty::foreground_name(p.master.as_raw_fd(), p.pid)
            && running != base
        {
            label.push_str(" running ");
            label.push_str(&running);
        }
        if let Some(report) = p.progress {
            label.push_str(&format!(" ({})", crate::theme::progress_text(report)));
        }
        if let Some(words) = self.command_words(p) {
            label.push_str(&format!(" ({words})"));
        }
        label
    }

    /// What the shell of a pane says about its command, in words (see `shellmarks`): how long it
    /// has run, or how a long one ended while nobody looked. The marks do not carry the command's
    /// text, so it is not named.
    pub(super) fn command_words(&self, pane: &Pane) -> Option<String> {
        let limits = crate::shellmarks::Thresholds {
            running_after: self.config.command_running_after,
            finished_after: self.config.command_finished_after,
        };
        let now = Instant::now();
        match (pane.command.finished, pane.command.running_since) {
            (Some(false), _) => Some("finished with an error".to_string()),
            (_, Some(t)) if pane.command.is_running(now, limits) => Some(format!(
                "running {}",
                crate::shellmarks::duration_words(now.saturating_duration_since(t))
            )),
            (Some(true), _) => Some("finished well".to_string()),
            _ => None,
        }
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
            ListCommand::Clear => {}
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
            Action::PaneBeside { pane, target } => {
                self.move_pane(session_id, pane, PaneDest::Beside(target))
            }
            Action::PaneToSession { pane, session } => {
                self.move_pane(session_id, pane, PaneDest::NewWindow(session))
            }
            Action::WindowAfter { window, target } => {
                self.put_window(session_id, window, WindowDest::After(target))
            }
            Action::WindowToSession { window, session } => {
                self.put_window(session_id, window, WindowDest::End(session))
            }
        }
    }

    /// Put a pane down: its process, its screen and its history go on as they are, only the
    /// place changes. Nothing changes when the place cannot take it.
    fn move_pane(&mut self, from: u64, pane: u64, dest: PaneDest) -> Result<String, String> {
        let (src_session, src_window) = self
            .locate_pane(pane)
            .ok_or_else(|| "that pane is gone".to_string())?;
        let (dst_session, dst_window) = match dest {
            PaneDest::Beside(target) => {
                if target == pane {
                    return Err("a pane cannot be put next to itself".to_string());
                }
                let (s, w) = self
                    .locate_pane(target)
                    .ok_or_else(|| "that pane is gone".to_string())?;
                (s, Some(w))
            }
            PaneDest::NewWindow(s) => {
                if !self.sessions.contains_key(&s) {
                    return Err("that session is gone".to_string());
                }
                (s, None)
            }
        };
        let (alone, windows_in_source) = {
            let session = self
                .sessions
                .get(&src_session)
                .ok_or_else(|| "no such session".to_string())?;
            let window = session
                .window(src_window)
                .ok_or_else(|| "no such window".to_string())?;
            (window.layout.len() < 2, session.windows.len())
        };
        // Look before touching anything: does the place have room?
        match (dest, dst_window) {
            (PaneDest::Beside(target), Some(dw)) => {
                let window = self
                    .sessions
                    .get(&dst_session)
                    .and_then(|s| s.window(dw))
                    .ok_or_else(|| "no such window".to_string())?;
                let mut probe = window.layout.clone();
                if dst_session == src_session && dw == src_window {
                    probe.close(pane).map_err(|_| "no such pane".to_string())?;
                }
                match probe.split(target, Axis::LeftRight, pane) {
                    Ok(()) => {}
                    Err(LayoutError::TooSmall) => {
                        return Err("no room in that window to split".to_string());
                    }
                    Err(_) => return Err("no such pane".to_string()),
                }
            }
            (PaneDest::NewWindow(_), _)
                if dst_session == src_session && alone && windows_in_source < 2 =>
            {
                return Err("the pane is alone in its window".to_string());
            }
            _ => {}
        }

        // Leave the old place.
        if self.panes.get(&pane).is_some_and(|p| p.copy.is_some()) {
            self.exit_copy(src_session, pane);
        }
        let source = Ctx {
            session: src_session,
            window: src_window,
            pane,
        };
        let mut emptied = None;
        if !alone {
            self.unzoom(source);
            self.remove_leaf(source);
        } else if windows_in_source > 1 {
            self.take_window(source);
        } else {
            emptied = self.sessions.remove(&src_session);
        }

        // Take the new place.
        let moved_into = match dest {
            PaneDest::Beside(target) => {
                let window = dst_window.unwrap_or(src_window);
                let ctx = Ctx {
                    session: dst_session,
                    window,
                    pane: target,
                };
                self.unzoom(ctx);
                if let Some(w) = self
                    .sessions
                    .get_mut(&dst_session)
                    .and_then(|s| s.window_mut(window))
                {
                    let _ = w.layout.split(target, Axis::LeftRight, pane);
                }
                self.set_focus(Ctx { pane, ..ctx }, pane);
                self.window_changed(ctx);
                window
            }
            PaneDest::NewWindow(_) => {
                let status = self.config.status_position;
                let (base_index, renumber) = (self.config.base_index, self.config.renumber);
                let window_id = self.next_window;
                self.next_window += 1;
                let moved_cwd = self.panes.get(&pane).and_then(|p| pty::process_cwd(p.pid));
                let from = self.config.window_name_from;
                let Some(destination) = self.sessions.get_mut(&dst_session) else {
                    return Err("that session is gone".to_string());
                };
                let cwd = moved_cwd.unwrap_or_else(|| destination.cwd.clone());
                let moved_name = name_of_pane(&self.panes, from, pane, &cwd);
                let (cols, rows) = window_area(destination.cols, destination.rows, status);
                add_window(
                    destination,
                    Window {
                        id: window_id,
                        number: 0,
                        name: moved_name,
                        name_manual: false,
                        program: String::new(),
                        directory: String::new(),
                        layout: Layout::new(pane, cols, rows),
                        focus: pane,
                        last_focus: None,
                        zoomed: false,
                        arrangement: None,
                    },
                    base_index,
                    renumber,
                );
                self.apply_window_sizes(dst_session, window_id);
                window_id
            }
        };
        self.select_window(dst_session, moved_into);
        if let Some(s) = self.sessions.get_mut(&dst_session) {
            s.relayout = true;
            s.chrome_dirty = true;
        }
        self.follow(from, dst_session, emptied)
    }

    /// Put a window down: in the bar after another window, or at the end of a session. Its
    /// panes and their processes go on as they are.
    fn put_window(&mut self, from: u64, window: u64, dest: WindowDest) -> Result<String, String> {
        let session_of = |server: &Server, w: u64| {
            server
                .sessions
                .values()
                .find(|s| s.window(w).is_some())
                .map(|s| s.id)
        };
        let src_session =
            session_of(self, window).ok_or_else(|| "that window is gone".to_string())?;
        let dst_session = match dest {
            WindowDest::After(target) => {
                if target == window {
                    return Err("a window cannot be put after itself".to_string());
                }
                session_of(self, target).ok_or_else(|| "that window is gone".to_string())?
            }
            WindowDest::End(s) => {
                if !self.sessions.contains_key(&s) {
                    return Err("that session is gone".to_string());
                }
                s
            }
        };
        let same = src_session == dst_session;
        let (windows_in_source, last_in_source) = {
            let s = &self.sessions[&src_session];
            (s.windows.len(), s.windows.last().map(|w| w.id))
        };
        if same && matches!(dest, WindowDest::End(_)) && last_in_source == Some(window) {
            return Err("the window is the last of its session already".to_string());
        }
        let (base_index, renumber) = (self.config.base_index, self.config.renumber);
        let status = self.config.status_position;

        // Leave the old place.
        let mut emptied = None;
        let moved = if same {
            let s = self
                .sessions
                .get_mut(&src_session)
                .ok_or_else(|| "no such session".to_string())?;
            let at = s.windows.iter().position(|w| w.id == window);
            at.map(|i| s.windows.remove(i))
        } else if windows_in_source > 1 {
            let focus = self.sessions[&src_session]
                .window(window)
                .map_or(0, |w| w.focus);
            self.take_window(Ctx {
                session: src_session,
                window,
                pane: focus,
            })
        } else {
            self.sessions.remove(&src_session).and_then(|mut old| {
                let moved = old.windows.pop();
                emptied = Some(old);
                moved
            })
        };
        let Some(mut moved) = moved else {
            return Err("that window is gone".to_string());
        };

        // Take the new place.
        let Some(destination) = self.sessions.get_mut(&dst_session) else {
            return Err("that session is gone".to_string());
        };
        if !same {
            let (cols, rows) = window_area(destination.cols, destination.rows, status);
            moved.layout.set_area(cols, rows);
        }
        let after = match dest {
            WindowDest::After(target) => Some(target),
            WindowDest::End(_) => destination.windows.last().map(|w| w.id),
        };
        match after {
            Some(after) => add_window_after(destination, moved, after, base_index, renumber),
            None => add_window(destination, moved, base_index, renumber),
        }
        destination.chrome_dirty = true;
        destination.relayout = true;
        self.apply_window_sizes(dst_session, window);
        self.select_window(dst_session, window);
        self.follow(from, dst_session, emptied)
    }

    /// The client goes to the session the moved item is in. The session the item left, when it
    /// was emptied, closes; a client other than the one that moved it is told.
    fn follow(&mut self, from: u64, to: u64, emptied: Option<Session>) -> Result<String, String> {
        let Some(old) = emptied else {
            return self.switch_session_id(from, to);
        };
        if old.id == from {
            if let Some(client) = old.client {
                self.bind_client(client, to, old.cols, old.rows);
            }
            self.log.line(&format!(
                "session {} closed: its last pane was moved",
                old.id
            ));
            return Ok(String::new());
        }
        self.finish_session(&old, "its last pane was moved to another session");
        self.switch_session_id(from, to)
    }

    /// Go to a session, a window or a pane: the client changes session when it has to.
    pub(super) fn jump_to(&mut self, from: u64, key: TreeKey) -> Result<String, String> {
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
