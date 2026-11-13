//! What the named commands do to sessions, windows and panes.

use super::*;
use crate::commands::{self, Command};
use crate::layout::{Axis, Direction, LayoutError};

/// Where a command acts: a session, one of its windows, and a pane in that window.
#[derive(Clone, Copy)]
pub(super) struct Ctx {
    pub session: u64,
    pub window: u64,
    pub pane: u64,
}

/// How far one resize step moves a divider.
const RESIZE_COLS: u16 = 5;
const RESIZE_ROWS: u16 = 2;

impl Server {
    pub(super) fn run_command(
        &mut self,
        name: &str,
        args: &[String],
        from_pane: Option<u64>,
    ) -> (bool, String) {
        match name {
            "server.ping" => return (true, "pong".to_string()),
            "server.kill" => {
                self.stopping = Some(Instant::now() + KILL_GRACE);
                for p in self.panes.values() {
                    pty::signal_group(p.pid, libc::SIGHUP);
                }
                let ids: Vec<u64> = self
                    .sessions
                    .values()
                    .filter(|s| s.client.is_some())
                    .map(|s| s.id)
                    .collect();
                for id in ids {
                    self.detach_client(id, DetachReason::ServerStopping, "server stopping");
                }
                return (true, String::new());
            }
            _ => {}
        }
        let command = match commands::parse(name, args) {
            Ok(command) => command,
            Err(message) => return (false, message),
        };
        match self.execute(command, from_pane) {
            Ok(output) => (true, output),
            Err(message) => (false, message),
        }
    }

    /// The session, window and pane a command from `from_pane` acts on; without a pane, the
    /// session whose client is attached (the one used last when several are), or else the one
    /// used last.
    fn resolve(&self, from_pane: Option<u64>) -> Result<Ctx, String> {
        if let Some(pane) = from_pane {
            return self
                .locate_pane(pane)
                .map(|(session, window)| Ctx {
                    session,
                    window,
                    pane,
                })
                .ok_or_else(|| "no such pane".to_string());
        }
        let session = self
            .sessions
            .values()
            .filter(|s| s.client.is_some())
            .max_by_key(|s| s.last_used)
            .or_else(|| self.sessions.values().max_by_key(|s| s.last_used))
            .ok_or_else(|| "no sessions".to_string())?;
        let window = session.current_window();
        Ok(Ctx {
            session: session.id,
            window: window.id,
            pane: window.focus,
        })
    }

    /// The session and window a pane is in.
    pub(super) fn locate_pane(&self, pane: u64) -> Option<(u64, u64)> {
        self.sessions.values().find_map(|s| {
            s.windows
                .iter()
                .find(|w| w.layout.contains(pane))
                .map(|w| (s.id, w.id))
        })
    }

    pub(super) fn execute(
        &mut self,
        command: Command,
        from_pane: Option<u64>,
    ) -> Result<String, String> {
        match &command {
            Command::SessionList => return Ok(self.list_sessions()),
            Command::SessionClose(Some(name)) => {
                let session = self
                    .sessions
                    .values()
                    .find(|s| &s.name == name)
                    .map(|s| s.id)
                    .ok_or_else(|| "no such session".to_string())?;
                self.end_session(session, "closed");
                return Ok(String::new());
            }
            _ => {}
        }
        let ctx = self.resolve(from_pane)?;
        self.execute_in(ctx, command)
    }

    pub(super) fn execute_in(&mut self, ctx: Ctx, command: Command) -> Result<String, String> {
        match command {
            Command::PaneFocus(direction) => self.focus_direction(ctx, direction),
            Command::PaneFocusLast => self.focus_last(ctx),
            Command::PaneResize(direction) => self.resize_pane(ctx, direction),
            Command::PaneSplit(axis) => self.split_pane(ctx, axis),
            Command::PaneZoom => self.zoom_pane(ctx),
            Command::PaneClose => {
                if let Some(pane) = self.panes.get_mut(&ctx.pane) {
                    pane.exit_note = Some("closed".to_string());
                }
                self.close_pane(ctx.pane);
                Ok(String::new())
            }
            Command::WindowNew => self.new_window(ctx),
            Command::WindowSelect(number) => {
                let target = number
                    .checked_sub(1)
                    .and_then(|index| {
                        self.sessions
                            .get(&ctx.session)
                            .and_then(|s| s.windows.get(index))
                    })
                    .map(|w| w.id)
                    .ok_or_else(|| format!("no window {number}"))?;
                self.select_window(ctx.session, target);
                Ok(String::new())
            }
            Command::WindowNext => self.step_window(ctx.session, 1),
            Command::WindowPrevious => self.step_window(ctx.session, -1),
            Command::WindowLast => {
                let last = self
                    .sessions
                    .get(&ctx.session)
                    .and_then(|s| s.last_window)
                    .ok_or_else(|| "no window to go back to".to_string())?;
                self.select_window(ctx.session, last);
                Ok(String::new())
            }
            Command::WindowRename(Some(name)) => {
                let window = self
                    .sessions
                    .get_mut(&ctx.session)
                    .and_then(|s| s.window_mut(ctx.window))
                    .ok_or_else(|| "no such window".to_string())?;
                window.name = name;
                window.name_manual = true;
                if let Some(s) = self.sessions.get_mut(&ctx.session) {
                    s.chrome_dirty = true;
                }
                Ok(String::new())
            }
            Command::WindowRename(None) => {
                Err("window.rename needs a name when it is not typed at the keyboard".to_string())
            }
            Command::WindowMoveLeft => self.move_window(ctx, -1),
            Command::WindowMoveRight => self.move_window(ctx, 1),
            Command::SessionDetach => {
                self.detach_client(ctx.session, DetachReason::Requested, "detached");
                Ok(String::new())
            }
            Command::SessionClose(None) => {
                self.end_session(ctx.session, "closed");
                Ok(String::new())
            }
            Command::SessionList => Ok(self.list_sessions()),
            Command::SessionRename(Some(name)) => {
                if self
                    .sessions
                    .values()
                    .any(|s| s.name == name && s.id != ctx.session)
                {
                    return Err(format!("a session called {name} exists"));
                }
                if let Some(s) = self.sessions.get_mut(&ctx.session) {
                    s.name = name;
                    s.chrome_dirty = true;
                }
                Ok(String::new())
            }
            Command::SessionRename(None) => {
                Err("session.rename needs a name when it is not typed at the keyboard".to_string())
            }
            Command::ArcadeLock => {
                let can_unlock = self.keymap.has_lock_key();
                let session = self
                    .sessions
                    .get_mut(&ctx.session)
                    .ok_or_else(|| "no such session".to_string())?;
                if !session.key_state.locked && !can_unlock {
                    return Err("no key would unlock: the direct table has no lock key".to_string());
                }
                session.key_state.locked = !session.key_state.locked;
                session.key_state.prefix = keys::PrefixState::Idle;
                session.chrome_dirty = true;
                Ok(String::new())
            }
            Command::ArcadeRedraw => {
                if let Some(s) = self.sessions.get_mut(&ctx.session) {
                    s.need_full = true;
                    s.relayout = true;
                    s.chrome_dirty = true;
                }
                Ok(String::new())
            }
            _ => Err("this command is not available yet".to_string()),
        }
    }

    fn list_sessions(&self) -> String {
        let mut sessions: Vec<&Session> = self.sessions.values().collect();
        sessions.sort_by_key(|s| s.id);
        let mut text = String::new();
        for s in sessions {
            text.push_str(&format!(
                "{}\t{}\t{}\t{}\t{}\n",
                s.name,
                s.id,
                u8::from(s.client.is_some()),
                s.windows.len(),
                s.pane_count()
            ));
        }
        text
    }

    // ---- Focus ----

    fn set_focus(&mut self, ctx: Ctx, pane: u64) {
        let Some(session) = self.sessions.get_mut(&ctx.session) else {
            return;
        };
        let Some(window) = session.window_mut(ctx.window) else {
            return;
        };
        if window.focus == pane {
            return;
        }
        window.last_focus = Some(window.focus);
        window.focus = pane;
        session.chrome_dirty = true;
    }

    /// Put a zoomed window back to its layout.
    fn unzoom(&mut self, ctx: Ctx) {
        let Some(window) = self
            .sessions
            .get_mut(&ctx.session)
            .and_then(|s| s.window_mut(ctx.window))
        else {
            return;
        };
        if window.zoomed {
            window.zoomed = false;
            self.window_changed(ctx);
        }
    }

    /// The rooms of the window's panes changed: size the panes and draw the window again.
    fn window_changed(&mut self, ctx: Ctx) {
        self.apply_window_sizes(ctx.session, ctx.window);
        if let Some(session) = self.sessions.get_mut(&ctx.session) {
            session.relayout = true;
        }
    }

    fn focus_direction(&mut self, ctx: Ctx, direction: Direction) -> Result<String, String> {
        self.unzoom(ctx);
        let cursor = self.panes.get(&ctx.pane).map(|p| p.last_cursor);
        let Some(window) = self
            .sessions
            .get(&ctx.session)
            .and_then(|s| s.window(ctx.window))
        else {
            return Err("no such window".to_string());
        };
        let Some(rect) = window.layout.rect_of(ctx.pane) else {
            return Err("no such pane".to_string());
        };
        let (cx, cy) = cursor.map_or((0, 0), |c| (c.x, c.y));
        let line = match direction {
            Direction::Left | Direction::Right => rect.y + cy,
            Direction::Up | Direction::Down => rect.x + cx,
        };
        if let Some(next) = window.layout.neighbour(ctx.pane, direction, line) {
            self.set_focus(ctx, next);
        }
        Ok(String::new())
    }

    fn focus_last(&mut self, ctx: Ctx) -> Result<String, String> {
        let last = self
            .sessions
            .get(&ctx.session)
            .and_then(|s| s.window(ctx.window))
            .and_then(|w| w.last_focus.filter(|p| w.layout.contains(*p)));
        match last {
            Some(pane) => {
                self.unzoom(ctx);
                self.set_focus(ctx, pane);
                Ok(String::new())
            }
            None => Ok(String::new()),
        }
    }

    // ---- Layout ----

    fn resize_pane(&mut self, ctx: Ctx, direction: Direction) -> Result<String, String> {
        self.unzoom(ctx);
        let amount = match direction {
            Direction::Left | Direction::Right => RESIZE_COLS,
            Direction::Up | Direction::Down => RESIZE_ROWS,
        };
        let window = self
            .sessions
            .get_mut(&ctx.session)
            .and_then(|s| s.window_mut(ctx.window))
            .ok_or_else(|| "no such window".to_string())?;
        match window.layout.resize(ctx.pane, direction, amount) {
            Ok(()) => {
                self.window_changed(ctx);
                Ok(String::new())
            }
            Err(LayoutError::TooSmall) => Err("cannot resize: a pane would be too small".into()),
            Err(LayoutError::NothingThere) => Err("no divider to move that way".into()),
            Err(LayoutError::NoSuchPane) => Err("no such pane".into()),
        }
    }

    fn split_pane(&mut self, ctx: Ctx, axis: Axis) -> Result<String, String> {
        self.unzoom(ctx);
        let session = self
            .sessions
            .get(&ctx.session)
            .ok_or_else(|| "no such session".to_string())?;
        let window = session
            .window(ctx.window)
            .ok_or_else(|| "no such window".to_string())?;
        // Try the split on a copy first, so that no process is started for a split that has no
        // room.
        let mut probe = window.layout.clone();
        match probe.split(ctx.pane, axis, u64::MAX) {
            Ok(()) => {}
            Err(LayoutError::TooSmall) => return Err("no room to split".into()),
            Err(_) => return Err("no such pane".into()),
        }
        let rect = probe
            .rect_of(u64::MAX)
            .ok_or_else(|| "no room to split".to_string())?;
        let cwd = self
            .panes
            .get(&ctx.pane)
            .and_then(|p| pty::process_cwd(p.pid))
            .unwrap_or_else(|| session.cwd.clone());
        let env = session.env.clone();
        let pane = self.spawn_pane(Vec::new(), cwd, &env, rect.w, rect.h)?;
        let new_id = pane.id;
        self.panes.insert(new_id, pane);
        if let Some(window) = self
            .sessions
            .get_mut(&ctx.session)
            .and_then(|s| s.window_mut(ctx.window))
        {
            let _ = window.layout.split(ctx.pane, axis, new_id);
        }
        self.set_focus(ctx, new_id);
        self.window_changed(ctx);
        Ok(String::new())
    }

    fn zoom_pane(&mut self, ctx: Ctx) -> Result<String, String> {
        let window = self
            .sessions
            .get_mut(&ctx.session)
            .and_then(|s| s.window_mut(ctx.window))
            .ok_or_else(|| "no such window".to_string())?;
        if !window.zoomed && window.layout.len() < 2 {
            return Err("only one pane".to_string());
        }
        window.zoomed = !window.zoomed;
        // The zoomed pane is the one with the focus.
        if window.zoomed && window.focus != ctx.pane {
            window.last_focus = Some(window.focus);
            window.focus = ctx.pane;
        }
        self.window_changed(ctx);
        if let Some(session) = self.sessions.get_mut(&ctx.session) {
            session.chrome_dirty = true;
        }
        Ok(String::new())
    }

    // ---- Windows ----

    fn new_window(&mut self, ctx: Ctx) -> Result<String, String> {
        let session = self
            .sessions
            .get(&ctx.session)
            .ok_or_else(|| "no such session".to_string())?;
        let (cols, rows) = window_area(session.cols, session.rows);
        let cwd = self
            .panes
            .get(&ctx.pane)
            .and_then(|p| pty::process_cwd(p.pid))
            .unwrap_or_else(|| session.cwd.clone());
        let env = session.env.clone();
        let pane = self.spawn_pane(Vec::new(), cwd.clone(), &env, cols, rows)?;
        let pane_id = pane.id;
        self.panes.insert(pane_id, pane);
        let window_id = self.next_window;
        self.next_window += 1;
        let Some(session) = self.sessions.get_mut(&ctx.session) else {
            return Ok(String::new());
        };
        session.windows.push(Window {
            id: window_id,
            name: directory_name(&cwd),
            name_manual: false,
            layout: Layout::new(pane_id, cols, rows),
            focus: pane_id,
            last_focus: None,
            zoomed: false,
        });
        self.select_window(ctx.session, window_id);
        Ok(String::new())
    }

    /// Show another window; the one shown now becomes the one to go back to.
    fn select_window(&mut self, session_id: u64, window_id: u64) {
        let Some(session) = self.sessions.get_mut(&session_id) else {
            return;
        };
        if session.current == window_id || session.window(window_id).is_none() {
            return;
        }
        session.last_window = Some(session.current);
        session.current = window_id;
        session.relayout = true;
        session.chrome_dirty = true;
    }

    /// The next window (`step` 1) or the previous one (`step` -1), going round at the ends.
    fn step_window(&mut self, session_id: u64, step: isize) -> Result<String, String> {
        let session = self
            .sessions
            .get(&session_id)
            .ok_or_else(|| "no such session".to_string())?;
        let count = session.windows.len() as isize;
        let at = session
            .windows
            .iter()
            .position(|w| w.id == session.current)
            .unwrap_or(0) as isize;
        let target = session.windows[(at + step).rem_euclid(count) as usize].id;
        self.select_window(session_id, target);
        Ok(String::new())
    }

    /// Swap the window with its neighbour in the status bar; at either end nothing happens.
    fn move_window(&mut self, ctx: Ctx, step: isize) -> Result<String, String> {
        let session = self
            .sessions
            .get_mut(&ctx.session)
            .ok_or_else(|| "no such session".to_string())?;
        let Some(at) = session.windows.iter().position(|w| w.id == ctx.window) else {
            return Err("no such window".to_string());
        };
        let to = at as isize + step;
        if to < 0 || to as usize >= session.windows.len() {
            return Ok(String::new());
        }
        session.windows.swap(at, to as usize);
        session.chrome_dirty = true;
        Ok(String::new())
    }

    // ---- Closing ----

    /// A pane's process ended, or the pane was closed: take it out of its window, close the
    /// window with its last pane, and the session with its last window.
    pub(super) fn close_pane(&mut self, pane_id: u64) {
        let Some((session_id, window_id)) = self.locate_pane(pane_id) else {
            self.drop_pane(pane_id);
            return;
        };
        let Some(session) = self.sessions.get(&session_id) else {
            return;
        };
        let panes_in_window = session.window(window_id).map_or(0, |w| w.layout.len());
        if panes_in_window <= 1 && session.windows.len() <= 1 {
            // Show the final screen before saying goodbye.
            self.build_frame_for(session_id);
            let note = self
                .panes
                .get(&pane_id)
                .and_then(|p| p.exit_note.clone())
                .unwrap_or_default();
            let note = note
                .trim_start_matches('[')
                .trim_end_matches(']')
                .to_string();
            self.end_session(session_id, &note);
            return;
        }
        let ctx = Ctx {
            session: session_id,
            window: window_id,
            pane: pane_id,
        };
        if panes_in_window <= 1 {
            self.remove_window(ctx);
        } else {
            self.remove_leaf(ctx);
        }
        self.drop_pane(pane_id);
    }

    /// Remove one pane of a window that has others; its room goes to a neighbour.
    fn remove_leaf(&mut self, ctx: Ctx) {
        let Some(window) = self
            .sessions
            .get_mut(&ctx.session)
            .and_then(|s| s.window_mut(ctx.window))
        else {
            return;
        };
        let taker = window.layout.close(ctx.pane).ok();
        if window.last_focus == Some(ctx.pane) {
            window.last_focus = None;
        }
        if window.focus == ctx.pane {
            window.focus = window.last_focus.take().or(taker).unwrap_or(window.focus);
            window.zoomed = false;
        }
        if let Some(session) = self.sessions.get_mut(&ctx.session) {
            session.chrome_dirty = true;
        }
        self.window_changed(ctx);
    }

    /// Remove a window, which has one pane left (the caller drops it), and show another.
    fn remove_window(&mut self, ctx: Ctx) {
        let Some(session) = self.sessions.get_mut(&ctx.session) else {
            return;
        };
        let Some(index) = session.windows.iter().position(|w| w.id == ctx.window) else {
            return;
        };
        session.windows.remove(index);
        if session.last_window == Some(ctx.window) {
            session.last_window = None;
        }
        if session.current == ctx.window {
            let next = session
                .last_window
                .take()
                .filter(|id| session.windows.iter().any(|w| w.id == *id))
                .unwrap_or_else(|| session.windows[index.min(session.windows.len() - 1)].id);
            session.current = next;
        }
        session.relayout = true;
        session.chrome_dirty = true;
    }

    /// End a session: hang up every pane and tell the client.
    pub(super) fn end_session(&mut self, session_id: u64, note: &str) {
        let Some(session) = self.sessions.remove(&session_id) else {
            return;
        };
        for pane in session.all_panes() {
            self.drop_pane(pane);
        }
        self.log
            .line(&format!("session {} closed: {note}", session.id));
        if let Some(client_id) = session.client {
            self.send(
                client_id,
                &ServerMessage::SessionEnded {
                    message: format!("[{}: {note}]", session.name),
                },
            );
            if let Some(c) = self.clients.get_mut(&client_id) {
                c.state = ClientState::Ready;
                c.close_after_flush = true;
            }
        }
    }
}
