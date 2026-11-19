//! What the named commands do to sessions, windows and panes.

use super::*;
use crate::commands::{self, Command, CopyCommand, ListCommand};
use crate::copy::{Copy, Outcome};
use crate::keymap::{Binding, typed_name};
use crate::layout::{Axis, Direction, LayoutError};
use crate::picker::{self, Item, PickerKind};
use crate::tree::Key as TreeKey;

/// Where a command acts: a session, one of its windows, and a pane in that window.
#[derive(Clone, Copy)]
pub(super) struct Ctx {
    pub session: u64,
    pub window: u64,
    pub pane: u64,
}

/// The line `pane.copy-info` copies: the version, the session, the window, the pane (the number
/// `arcade notify list` prints), its title, and the daemon clock that `arcade window log` prints.
/// Names are quoted so that spaces are plain. Nothing else is in it: no environment and no
/// screen contents.
fn pane_info_line(
    version: &str,
    session: &str,
    window_number: usize,
    window_name: &str,
    pane: u64,
    title: &str,
    at_seconds: f64,
) -> String {
    format!(
        "arcade {version} · session {session:?} · window {window_number} {window_name:?} · pane {pane} {title:?} · at {at_seconds:.1} s"
    )
}

/// How far one resize step moves a divider.
const RESIZE_COLS: u16 = 5;
const RESIZE_ROWS: u16 = 2;

/// Put a window in a session's bar: with the number it gets there (the next one, or the lowest
/// free one when numbers do not close up), in its place by number.
pub(super) fn add_window(
    session: &mut Session,
    mut window: Window,
    base_index: usize,
    renumber: bool,
) {
    window.number = if renumber {
        base_index + session.windows.len()
    } else {
        (base_index..)
            .find(|n| session.windows.iter().all(|w| w.number != *n))
            .unwrap_or(base_index)
    };
    let place = session
        .windows
        .iter()
        .position(|w| w.number > window.number)
        .unwrap_or(session.windows.len());
    session.windows.insert(place, window);
}

/// Put a window in a session's bar right after another one: with the next number when numbers
/// close up, else with the first free number after the other's.
pub(super) fn add_window_after(
    session: &mut Session,
    mut window: Window,
    after: u64,
    base_index: usize,
    renumber: bool,
) {
    let Some(at) = session.windows.iter().position(|w| w.id == after) else {
        return add_window(session, window, base_index, renumber);
    };
    if renumber {
        session.windows.insert(at + 1, window);
        for (i, w) in session.windows.iter_mut().enumerate() {
            w.number = base_index + i;
        }
    } else {
        let after_number = session.windows[at].number;
        window.number = (after_number + 1..)
            .find(|n| session.windows.iter().all(|w| w.number != *n))
            .unwrap_or(base_index);
        session.windows.insert(at + 1, window);
        session.windows.sort_by_key(|w| w.number);
    }
}

impl Server {
    /// Look over the daemon's own books and say what does not add up: every window has one
    /// focused pane that is in its layout, the panes fill the window's room exactly (with the
    /// dividers), none is empty, and no pane is left without a window. For the soak test; not a
    /// key command.
    fn check_consistency(&self) -> (bool, String) {
        let mut problems: Vec<String> = Vec::new();
        let mut in_layouts: Vec<PaneId> = Vec::new();
        for s in self.sessions.values() {
            let who = format!("session {}", s.name);
            if s.windows.is_empty() {
                problems.push(format!("{who} has no window"));
                continue;
            }
            if s.window(s.current).is_none() {
                problems.push(format!("{who}: the current window does not exist"));
            }
            if s.cols == 0 || s.rows == 0 {
                problems.push(format!("{who}: size {}x{}", s.cols, s.rows));
            }
            let mut numbers: Vec<usize> = s.windows.iter().map(|w| w.number).collect();
            numbers.sort_unstable();
            numbers.dedup();
            if numbers.len() != s.windows.len() {
                problems.push(format!("{who}: two windows share a number"));
            }
            for w in &s.windows {
                let at = format!("{who}, window {}", w.number);
                let area = w.layout.area();
                let rects = w.layout.rects();
                if rects.is_empty() || rects.len() != w.layout.len() {
                    problems.push(format!(
                        "{at}: {} rects for {} panes",
                        rects.len(),
                        w.layout.len()
                    ));
                }
                if !w.layout.contains(w.focus) {
                    problems.push(format!("{at}: the focused pane is not in the layout"));
                }
                if w.last_focus.is_some_and(|p| !w.layout.contains(p)) {
                    problems.push(format!("{at}: the last focused pane is gone"));
                }
                let mut cells: u32 = 0;
                // With less room than one cell for every pane, some panes cannot be shown; the
                // window is then too small to say anything about the sizes.
                let (least_w, least_h) = w.layout.least_room();
                let too_small = area.w < least_w || area.h < least_h;
                for (pane, _) in &rects {
                    in_layouts.push(*pane);
                    if !self.panes.contains_key(pane) {
                        problems.push(format!("{at}: pane {pane} has no terminal"));
                    }
                }
                if too_small {
                    continue;
                }
                for (pane, r) in &rects {
                    if r.w == 0 || r.h == 0 {
                        problems.push(format!("{at}: pane {pane} has size {}x{}", r.w, r.h));
                    }
                    if r.x + r.w > area.w || r.y + r.h > area.h {
                        problems.push(format!("{at}: pane {pane} sticks out of the window"));
                    }
                    cells += u32::from(r.w) * u32::from(r.h);
                }
                for (i, (a, ra)) in rects.iter().enumerate() {
                    for (b, rb) in &rects[i + 1..] {
                        let apart = ra.right() <= rb.x
                            || rb.right() <= ra.x
                            || ra.bottom() <= rb.y
                            || rb.bottom() <= ra.y;
                        if !apart {
                            problems.push(format!("{at}: panes {a} and {b} overlap"));
                        }
                    }
                }
                for d in w.layout.dividers() {
                    cells += u32::from(d.len);
                }
                if cells != u32::from(area.w) * u32::from(area.h) {
                    problems.push(format!(
                        "{at}: panes and dividers cover {cells} cells of {}",
                        u32::from(area.w) * u32::from(area.h)
                    ));
                }
            }
        }
        for pane in self.panes.keys() {
            if !in_layouts.contains(pane) && !self.exited.contains(pane) {
                problems.push(format!("pane {pane} belongs to no window"));
            }
        }
        for (pane, n) in self.notices.by_priority() {
            if !self.panes.contains_key(&pane) || !in_layouts.contains(&pane) {
                problems.push(format!("pane {pane} has a notification and is gone"));
            }
            let texts = [
                (&n.title, crate::notify::TITLE_LIMIT),
                (&n.message, crate::notify::MESSAGE_LIMIT),
                (&n.source, crate::notify::SOURCE_LIMIT),
            ];
            for (text, limit) in texts {
                if text.chars().count() > limit || text.chars().any(char::is_control) {
                    problems.push(format!("pane {pane}: a notification text is not clean"));
                }
            }
        }
        if problems.is_empty() {
            (true, "ok".to_string())
        } else {
            (false, problems.join("\n"))
        }
    }

    pub(super) fn run_command(
        &mut self,
        name: &str,
        args: &[String],
        from_pane: Option<u64>,
    ) -> (bool, String) {
        // A command run by a key, the bar or a list keeps the cause that asked for it.
        if self.cause.is_some() {
            return self.run_command_body(name, args, from_pane);
        }
        self.with_cause(format!("command {name}"), |s| {
            s.run_command_body(name, args, from_pane)
        })
    }

    fn run_command_body(
        &mut self,
        name: &str,
        args: &[String],
        from_pane: Option<u64>,
    ) -> (bool, String) {
        match name {
            "server.ping" => return (true, "pong".to_string()),
            "server.version" => return (true, crate::program_version()),
            "server.check" => return self.check_consistency(),
            "notify.send" => return self.notify_send(args),
            "notify.clear" => return self.notify_clear(args),
            "notify.log" => return self.notify_log(),
            "notify.list" => return self.notify_list(),
            "keys.stray" => {
                let lines: Vec<&str> = self.stray_keys.1.iter().map(String::as_str).collect();
                return (true, format!("{}\n{}", self.stray_keys.0, lines.join("\n")));
            }
            "window.log" => return (true, self.window_log()),
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
            Command::KeysList => return Ok(self.list_keys()),
            Command::Palette => return Ok(self.list_commands()),
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
            Command::SessionNew(name) => return self.new_session(from_pane, name.clone()),
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
            Command::PaneSwap(direction) => self.swap_pane(ctx, direction),
            Command::PaneBreak => self.break_pane(ctx),
            Command::PaneCaptureKeys => self.toggle_capture(ctx),
            Command::WindowLayoutNext => self.layout_next(ctx),
            Command::WindowLayoutEven => self.layout_even(ctx),
            Command::PaneSplit(axis) => self.split_pane(ctx, axis),
            Command::PaneZoom => self.zoom_pane(ctx),
            Command::PaneCopyInfo => self.copy_pane_info(ctx),
            Command::PaneClose => {
                if let Some(pane) = self.panes.get_mut(&ctx.pane) {
                    pane.exit_note = Some("closed".to_string());
                }
                self.close_pane(ctx.pane);
                Ok(String::new())
            }
            Command::WindowNew(directory) => self.new_window(ctx, directory),
            Command::WindowSelect(number) => {
                let target = self
                    .sessions
                    .get(&ctx.session)
                    .and_then(|s| s.windows.iter().find(|w| w.number == number))
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
                // An empty name gives the window back to automatic naming: it is named by the
                // rule of the configuration at once.
                let automatic = name.is_empty().then(|| {
                    let focus = self
                        .sessions
                        .get(&ctx.session)
                        .and_then(|s| s.window(ctx.window))
                        .map(|w| w.focus);
                    let cwd = self.sessions.get(&ctx.session).map(|s| s.cwd.clone());
                    name_of_pane(
                        &self.panes,
                        self.config.window_name_from,
                        focus.unwrap_or(ctx.pane),
                        &cwd.unwrap_or_default(),
                    )
                });
                let window = self
                    .sessions
                    .get_mut(&ctx.session)
                    .and_then(|s| s.window_mut(ctx.window))
                    .ok_or_else(|| "no such window".to_string())?;
                window.name_manual = !name.is_empty();
                window.name = automatic.unwrap_or(name);
                if let Some(s) = self.sessions.get_mut(&ctx.session) {
                    s.chrome_dirty = true;
                    s.names_checked = None;
                }
                Ok(String::new())
            }
            Command::WindowRename(None) => {
                Err("window.rename needs a name when it is not typed at the keyboard".to_string())
            }
            Command::WindowMoveLeft => self.move_window(ctx, -1),
            Command::WindowMoveRight => self.move_window(ctx, 1),
            Command::SessionDetach => {
                self.detach_on_request(ctx.session);
                Ok(String::new())
            }
            Command::SessionClose(None) => {
                self.end_session(ctx.session, "closed");
                Ok(String::new())
            }
            Command::SessionList => Ok(self.list_sessions()),
            Command::WindowList => self.list_windows(ctx.session),
            Command::KeysList => Ok(self.list_keys()),
            Command::NotificationNext => self.notification_next(ctx.session),
            Command::NotificationList => Ok(self.notification_lines()),
            Command::NotificationClear => self.notification_clear_focused(ctx),
            Command::NotificationClearAll => Ok(self.notification_clear_all()),
            Command::Palette => Ok(self.list_commands()),
            Command::TreeOpen | Command::PaneMove | Command::WindowMove => {
                Err("this opens a list that is used at the keyboard".to_string())
            }
            Command::PaneSwapMode => Err(
                "this waits for a movement key; use pane.swap-left, -down, -up or -right"
                    .to_string(),
            ),
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
            // Handled before the context is found, in `execute`.
            Command::SessionNew(_) => Err("session.new is run by the server".to_string()),
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
                session.key_state.hint_shown = false;
                session.relayout = true;
                session.chrome_dirty = true;
                Ok(String::new())
            }
            Command::ArcadeReloadConfig => self.reload_config(ctx.session),
            Command::List(command) => self.list_command(ctx.session, command),
            Command::SessionSwitch(name) => self.switch_session(ctx.session, &name),
            Command::WindowMoveToSession(Some(name)) => self.move_window_to_session(ctx, &name),
            Command::WindowMoveToSession(None) => Err(
                "window.move-to-session needs a session name when it is not chosen from the list"
                    .to_string(),
            ),
            Command::ArcadeRedraw => {
                if let Some(s) = self.sessions.get_mut(&ctx.session) {
                    s.need_full = true;
                    s.relayout = true;
                    s.chrome_dirty = true;
                }
                Ok(String::new())
            }
            // `execute` handles the form with a name before it gets here.
            Command::SessionClose(Some(_)) => Err("no such session".to_string()),
            Command::CopyEnter => self.enter_copy(ctx),
            Command::CopyLastOutput => self.copy_last_output(ctx),
            Command::Copy(command) => self.copy_command(ctx, command),
            Command::PanePaste => self.paste_buffer(ctx.pane),
        }
    }

    // ---- Copy mode and the clipboard ----

    fn enter_copy(&mut self, ctx: Ctx) -> Result<String, String> {
        let pane = self
            .panes
            .get_mut(&ctx.pane)
            .ok_or_else(|| "no such pane".to_string())?;
        if pane.copy.is_none() {
            let (cols, rows) = (pane.term.cols(), pane.term.rows());
            pane.term.anchor_last_row();
            pane.copy = Some(Copy::enter(
                &pane.term,
                pane.term.cursor_position(),
                cols,
                rows,
            ));
            self.copy_mode_changed(ctx.session);
        }
        Ok(String::new())
    }

    /// The status bar and the pane's room must be drawn again.
    pub(super) fn copy_mode_changed(&mut self, session_id: u64) {
        if let Some(s) = self.sessions.get_mut(&session_id) {
            s.relayout = true;
            s.chrome_dirty = true;
        }
    }

    pub(super) fn exit_copy(&mut self, session_id: u64, pane_id: u64) {
        if let Some(pane) = self.panes.get_mut(&pane_id) {
            pane.copy = None;
            // What the screen shows now was not looked at while copy mode was on.
            pane.dirty = true;
        }
        self.copy_mode_changed(session_id);
    }

    fn copy_command(&mut self, ctx: Ctx, command: CopyCommand) -> Result<String, String> {
        let pane = self
            .panes
            .get_mut(&ctx.pane)
            .ok_or_else(|| "no such pane".to_string())?;
        let Some(copy) = pane.copy.as_mut() else {
            return Err("the pane is not in copy mode".to_string());
        };
        let outcome = copy.apply(&pane.term, command);
        // The word on the status bar may have changed (a selection began or ended, a jump opened).
        if let Some(s) = self.sessions.get_mut(&ctx.session) {
            s.chrome_dirty = true;
        }
        match outcome {
            Outcome::Moved => Ok(String::new()),
            Outcome::Message(message) => {
                self.set_message(ctx.session, &message);
                Ok(String::new())
            }
            Outcome::AskSearch { .. } => {
                Err("the text to search for is typed at the keyboard".to_string())
            }
            Outcome::Yank { text, stay } => {
                self.copy_text(ctx.session, &text);
                if !stay {
                    self.exit_copy(ctx.session, ctx.pane);
                }
                Ok(String::new())
            }
            Outcome::OpenLink { uri } => {
                let opener = self
                    .config
                    .links
                    .command
                    .clone()
                    .unwrap_or_else(crate::links::default_opener);
                match crate::links::open(&opener, &uri) {
                    Ok(()) => self.set_message(ctx.session, &format!("opening {uri}")),
                    Err(e) => self.set_error(
                        ctx.session,
                        &format!(
                            "cannot run {}: {e}",
                            opener.first().map_or("", String::as_str)
                        ),
                    ),
                }
                Ok(String::new())
            }
            Outcome::Exit => {
                self.exit_copy(ctx.session, ctx.pane);
                Ok(String::new())
            }
        }
    }

    /// Search in the pane's copy mode for what was typed at the prompt.
    pub(super) fn copy_search(&mut self, session_id: u64, pane_id: u64, text: &str, forward: bool) {
        let Some(pane) = self.panes.get_mut(&pane_id) else {
            return;
        };
        let Some(copy) = pane.copy.as_mut() else {
            return;
        };
        match copy.search(&pane.term, text, forward) {
            Ok(false) => {}
            Ok(true) => self.set_message(session_id, "search wrapped around"),
            Err(message) => self.set_error(session_id, &message),
        }
    }

    /// Keep `text` in the buffer and send it out the way the settings say, to the client of
    /// this session only.
    pub(super) fn copy_text(&mut self, session_id: u64, text: &str) {
        self.copy_text_noted(session_id, text, "");
    }

    /// Like `copy_text`, with `note` added to the message on the status bar.
    fn copy_text_noted(&mut self, session_id: u64, text: &str, note: &str) {
        let sent = self.clipboard.copy(text, &self.config.clipboard);
        let client = self.sessions.get(&session_id).and_then(|s| s.client);
        if let (Some(bytes), Some(client)) = (sent.osc52, client) {
            self.send_output(client, &bytes);
        }
        self.set_message(session_id, &format!("{}{note}", sent.message));
    }

    /// Copy one line that says where the pane in focus is and what it is called, for a person to
    /// paste to whoever traces a problem. It goes the way a copy does (see `copy_text`).
    fn copy_pane_info(&mut self, ctx: Ctx) -> Result<String, String> {
        let session = self
            .sessions
            .get(&ctx.session)
            .ok_or_else(|| "no such session".to_string())?;
        let window = session
            .window(ctx.window)
            .ok_or_else(|| "no such window".to_string())?;
        let title = self
            .panes
            .get(&ctx.pane)
            .and_then(|p| p.title.as_deref())
            .unwrap_or("");
        let line = pane_info_line(
            env!("CARGO_PKG_VERSION"),
            &session.name,
            window.number,
            &window.name,
            ctx.pane,
            title,
            self.started.elapsed().as_secs_f64(),
        );
        self.copy_text_noted(ctx.session, &line, " (pane info)");
        Ok(String::new())
    }

    /// Copy the output of the last command that finished in the pane (see `copy::last_output`).
    fn copy_last_output(&mut self, ctx: Ctx) -> Result<String, String> {
        let pane = self
            .panes
            .get(&ctx.pane)
            .ok_or_else(|| "no such pane".to_string())?;
        let (text, rows) =
            crate::copy::last_output(&pane.term, pane.term.cols(), pane.term.rows())?;
        let note = format!(" ({rows} line{})", if rows == 1 { "" } else { "s" });
        self.copy_text_noted(ctx.session, &text, &note);
        Ok(String::new())
    }

    /// A program in a pane asked for the clipboard to be set.
    /// Handle the clipboard write each pane made last, after the screen updates have gone out
    /// and only on a turn where the pane gave no output: a pane that keeps writing has only its
    /// latest write handled, when it pauses.
    pub(super) fn handle_pending_clipboard(&mut self, read_now: &[u64]) {
        if self.pending_clipboard.is_empty() {
            return;
        }
        let ready: Vec<u64> = self
            .pending_clipboard
            .keys()
            .copied()
            .filter(|id| {
                !read_now.contains(id) && self.panes.get(id).is_none_or(|p| !p.read_pending)
            })
            .collect();
        for pane_id in ready {
            if let Some(data) = self.pending_clipboard.remove(&pane_id) {
                self.pane_wrote_clipboard(pane_id, &data);
            }
        }
    }

    /// Handle the pending clipboard writes of the panes of one session.
    pub(super) fn handle_pending_clipboard_of(&mut self, session_id: u64) {
        if self.pending_clipboard.is_empty() {
            return;
        }
        let ready: Vec<u64> = self
            .pending_clipboard
            .keys()
            .copied()
            .filter(|id| self.locate_pane(*id).is_some_and(|(s, _)| s == session_id))
            .collect();
        for pane_id in ready {
            if let Some(data) = self.pending_clipboard.remove(&pane_id) {
                self.pane_wrote_clipboard(pane_id, &data);
            }
        }
    }

    fn pane_wrote_clipboard(&mut self, pane_id: u64, data: &[u8]) {
        if !self.config.clipboard.allow_pane_write {
            return;
        }
        let Some((session, _)) = self.locate_pane(pane_id) else {
            return;
        };
        let text = String::from_utf8_lossy(data).into_owned();
        self.copy_text(session, &text);
    }

    fn paste_buffer(&mut self, pane_id: u64) -> Result<String, String> {
        let text = self
            .clipboard
            .buffer()
            .map(str::to_string)
            .ok_or_else(|| "nothing has been copied yet".to_string())?;
        self.send_paste_to_pane(pane_id, text.as_bytes());
        Ok(String::new())
    }

    // ---- The floating list, switching sessions, moving windows between sessions ----

    /// Open the list of sessions, to go to one or to move the current window to one.
    pub(super) fn open_picker(&mut self, session_id: u64, kind: PickerKind) -> Result<(), String> {
        if kind == PickerKind::Notifications {
            return self.open_notifications(session_id, 0);
        }
        if kind == PickerKind::Keys {
            let rows = self.key_rows(session_id);
            let headers = [
                "Action".to_string(),
                format!("After {}", typed_name(self.config.keys.prefix)),
                match self.config.keys.direct_modifier {
                    Some(m) => format!("With {}", crate::keymap::mods_name(m)),
                    None => "Direct".to_string(),
                },
            ];
            if let Some(s) = self.sessions.get_mut(&session_id) {
                s.picker = Some(
                    Picker::new(kind, "Keys", picker::key_items(&rows), 0).with_key_view(headers),
                );
                s.chrome_dirty = true;
            }
            return Ok(());
        }
        if kind == PickerKind::Commands {
            let items: Vec<Item> = self
                .command_names()
                .into_iter()
                .enumerate()
                .map(|(id, name)| {
                    let keys = self.keys_of(name).join(", ");
                    Item::new(id, name.to_string(), keys, false)
                })
                .collect();
            if let Some(s) = self.sessions.get_mut(&session_id) {
                s.picker = Some(Picker::new(kind, "Commands", items, 0));
                s.chrome_dirty = true;
            }
            return Ok(());
        }
        if kind == PickerKind::Windows {
            let session = self
                .sessions
                .get(&session_id)
                .ok_or_else(|| "no such session".to_string())?;
            let items: Vec<Item> = session
                .windows
                .iter()
                .enumerate()
                .map(|(position, w)| {
                    Item::new(
                        position,
                        w.name.clone(),
                        format!("#{}", w.number),
                        w.id == session.current,
                    )
                })
                .collect();
            let selected = items.iter().position(|i| i.current).unwrap_or(0);
            if let Some(s) = self.sessions.get_mut(&session_id) {
                s.picker = Some(Picker::new(kind, "Windows", items, selected));
                s.chrome_dirty = true;
            }
            return Ok(());
        }
        let current = self
            .sessions
            .get(&session_id)
            .ok_or_else(|| "no such session".to_string())?;
        if kind == PickerKind::MoveWindow && current.windows.len() < 2 {
            return Err("this is the last window of its session".to_string());
        }
        // In the order they were made, so a new session comes last.
        let mut sessions: Vec<&Session> = self.sessions.values().collect();
        sessions.sort_by_key(|s| s.id);
        let mut items: Vec<Item> = sessions
            .iter()
            .filter(|s| kind == PickerKind::Sessions || s.id != session_id)
            .enumerate()
            .map(|(id, s)| {
                let detail = match s.windows.len() {
                    1 => "1 window".to_string(),
                    n => format!("{n} windows"),
                };
                Item::new(id, s.name.clone(), detail, s.id == session_id)
            })
            .collect();
        if items.is_empty() {
            return Err("no other session".to_string());
        }
        let selected = items.iter().position(|i| i.current).unwrap_or(0);
        if kind == PickerKind::Sessions {
            items.push(Item::new_session_row(items.len()));
        }
        let title = match kind {
            PickerKind::Sessions => "Sessions",
            PickerKind::MoveWindow => "Move window to",
            PickerKind::Keys => "Keys",
            PickerKind::Windows => "Windows",
            PickerKind::Commands => "Commands",
            PickerKind::Notifications => "Notifications",
        };
        if let Some(s) = self.sessions.get_mut(&session_id) {
            s.picker = Some(Picker::new(kind, title, items, selected));
            s.chrome_dirty = true;
        }
        Ok(())
    }

    /// Every binding of the prefix table, then of the direct table, as what a person presses and
    /// the binding, in a fixed order (by command, then by keys).
    /// The rows of the key list: one for each action, in groups.
    fn key_rows(&self, session_id: u64) -> Vec<picker::KeyRow> {
        let host = self.sessions.get(&session_id).and_then(|s| s.host_keyboard);
        let warnings = crate::keytools::warnings(&self.config, host);
        let cannot_work = |table: crate::config::TableName, key: Key| {
            let place = format!("keys.{}", table.word());
            warnings.iter().any(|w| {
                w.place == place && crate::keymap::normalize(w.key) == crate::keymap::normalize(key)
            })
        };
        picker::key_rows(
            &self.keymap,
            self.config.keys.prefix,
            self.config.keys.direct_modifier,
            &cannot_work,
        )
    }

    fn key_bindings(&self) -> Vec<(String, Binding)> {
        let prefix = typed_name(self.config.keys.prefix);
        let mut after_prefix: Vec<(String, Binding)> = self
            .keymap
            .prefix
            .iter()
            .map(|(key, b)| (format!("{prefix} {}", typed_name(*key)), b.clone()))
            .collect();
        let mut direct: Vec<(String, Binding)> = self
            .keymap
            .direct
            .iter()
            .map(|(key, b)| (typed_name(*key), b.clone()))
            .collect();
        let order = |a: &(String, Binding), b: &(String, Binding)| {
            (binding_line(&a.1), &a.0).cmp(&(binding_line(&b.1), &b.0))
        };
        after_prefix.sort_by(order);
        direct.sort_by(order);
        after_prefix.extend(direct);
        after_prefix
    }

    /// The key of the direct table that ends the lock, as it is typed (the first by name when
    /// several are bound).
    pub(super) fn unlock_key_name(&self) -> Option<String> {
        self.keymap
            .direct
            .iter()
            .filter(|(_, b)| b.command == "arcade.lock")
            .map(|(key, _)| typed_name(*key))
            .min()
    }

    /// The bindings as text, one per line: the keys, a tab, the command.
    fn list_keys(&self) -> String {
        self.key_bindings()
            .iter()
            .map(|(label, b)| format!("{label}\t{}", binding_line(b)))
            .collect::<Vec<_>>()
            .join("\n")
    }

    fn list_command(&mut self, session_id: u64, command: ListCommand) -> Result<String, String> {
        let session = self
            .sessions
            .get_mut(&session_id)
            .ok_or_else(|| "no such session".to_string())?;
        if session.tree.is_some() {
            return self.tree_command(session_id, command);
        }
        let Some(picker) = session.picker.as_mut() else {
            return Err("no list is open".to_string());
        };
        match command {
            // Only the tree has anything to go into or lift.
            ListCommand::Left | ListCommand::Right | ListCommand::Lift => {}
            ListCommand::Clear => {
                if picker.kind == PickerKind::Notifications {
                    let (pane, selected) = (picker.chosen().map(|i| i.id as u64), picker.selected);
                    if let Some(pane) = pane {
                        return self.clear_from_list(session_id, pane, selected);
                    }
                }
            }
            ListCommand::Up => picker.up(),
            ListCommand::Down => picker.down(),
            ListCommand::FilterDelete | ListCommand::FilterClear => {
                if command == ListCommand::FilterDelete {
                    picker.delete_char();
                } else {
                    picker.clear_filter();
                }
                // The box changes size with the lines that are left: draw what was under it.
                session.relayout = true;
            }
            ListCommand::Cancel => {
                session.picker = None;
                // What was under the list is drawn again in full.
                session.relayout = true;
            }
            ListCommand::Select => {
                let kind = picker.kind;
                let chosen = picker.chosen().map(|i| (i.id, i.name.clone()));
                let new_row = picker.chosen().is_some_and(|i| i.new_row);
                let typed = picker.filter().to_string();
                session.picker = None;
                session.relayout = true;
                session.chrome_dirty = true;
                let Some((index, name)) = chosen else {
                    return Ok(String::new());
                };
                if new_row {
                    // The Enter on this row is the confirmation: the session is made and shown
                    // as with the session.new key, and its name is asked for on its prompt,
                    // starting from what was typed to filter.
                    if let Some(pane) = self.focus_pane_of(session_id) {
                        self.new_session_then_name(session_id, pane, typed);
                    }
                    return Ok(String::new());
                }
                return match kind {
                    PickerKind::Keys => {
                        // The rows were made from the tables in this order, and nothing in them
                        // changes while the list is open.
                        if let Some(row) = self.key_rows(session_id).into_iter().nth(index)
                            && row.runnable
                        {
                            self.run_binding(session_id, &row.binding);
                        }
                        Ok(String::new())
                    }
                    PickerKind::Commands => {
                        // The list was made from the names in this order.
                        if let Some(name) = self.command_names().get(index) {
                            let binding = Binding {
                                command: (*name).to_string(),
                                args: Vec::new(),
                                repeat: false,
                            };
                            self.run_binding(session_id, &binding);
                        }
                        Ok(String::new())
                    }
                    PickerKind::Windows => {
                        let window = self
                            .sessions
                            .get(&session_id)
                            .and_then(|s| s.windows.get(index))
                            .map(|w| w.id);
                        if let Some(window) = window {
                            self.with_cause("the window list", |s| {
                                s.select_window(session_id, window)
                            });
                        }
                        Ok(String::new())
                    }
                    PickerKind::Notifications => self.with_cause("the notification list", |s| {
                        s.jump_to(session_id, TreeKey::Pane(index as u64))
                    }),
                    PickerKind::Sessions => self.switch_session(session_id, &name),
                    PickerKind::MoveWindow => {
                        let ctx = self.current_ctx(session_id)?;
                        self.move_window_to_session(ctx, &name)
                    }
                };
            }
        }
        session.chrome_dirty = true;
        Ok(String::new())
    }

    fn current_ctx(&self, session_id: u64) -> Result<Ctx, String> {
        let session = self
            .sessions
            .get(&session_id)
            .ok_or_else(|| "no such session".to_string())?;
        let window = session.current_window();
        Ok(Ctx {
            session: session_id,
            window: window.id,
            pane: window.focus,
        })
    }

    /// Make a session that starts in the directory of the pane the command comes from (the
    /// directory of the session, when there is none), with the name given or an automatic one.
    /// When the command comes from a pane, the screen of that pane goes to the new session; from
    /// a script with no pane it is only made.
    pub(super) fn new_session(
        &mut self,
        from_pane: Option<u64>,
        name: Option<String>,
    ) -> Result<String, String> {
        let ctx = self.resolve(from_pane)?;
        if let Some(name) = &name
            && self.sessions.values().any(|s| &s.name == name)
        {
            return Err(format!("a session called {name} exists"));
        }
        let source = self
            .sessions
            .get(&ctx.session)
            .ok_or_else(|| "no such session".to_string())?;
        let cwd = self
            .panes
            .get(&ctx.pane)
            .and_then(|p| pty::process_cwd(p.pid))
            .unwrap_or_else(|| source.cwd.clone());
        let request = arcade_proto::AttachRequest {
            session: None,
            create: arcade_proto::CreateMode::Always,
            new_name: name,
            command: Vec::new(),
            cwd,
            env: source.env.clone(),
            cols: source.cols,
            rows: source.rows,
        };
        let created = self.create_session(&request)?;
        let created_name = self
            .sessions
            .get(&created)
            .map(|s| s.name.clone())
            .unwrap_or_default();
        let shown = from_pane.is_some()
            && self
                .sessions
                .get(&ctx.session)
                .is_some_and(|s| s.client.is_some());
        if shown {
            self.switch_session_id(ctx.session, created)?;
        }
        Ok(created_name)
    }

    /// The client of this session goes to another session without leaving. A client that the
    /// other session had is told and let go.
    fn switch_session(&mut self, from: u64, name: &str) -> Result<String, String> {
        let target = self
            .sessions
            .values()
            .find(|s| s.name == name)
            .map(|s| s.id)
            .ok_or_else(|| format!("no such session: {name}"))?;
        self.switch_session_id(from, target)
    }

    /// As `switch_session`, for a session known by its number.
    pub(super) fn switch_session_id(&mut self, from: u64, target: u64) -> Result<String, String> {
        if target == from {
            return Ok(String::new());
        }
        let source = self
            .sessions
            .get_mut(&from)
            .ok_or_else(|| "no such session".to_string())?;
        let Some(client) = source.client.take() else {
            return Err("no client is attached to switch".to_string());
        };
        let (cols, rows) = (source.cols, source.rows);
        source.key_state.prefix = keys::PrefixState::Idle;
        source.picker = None;
        source.tree = None;
        self.bind_client(client, target, cols, rows);
        Ok(String::new())
    }

    /// The current window, with its panes and processes, goes to the end of another session's
    /// bar. The client stays where it is.
    fn move_window_to_session(&mut self, ctx: Ctx, name: &str) -> Result<String, String> {
        let target = self
            .sessions
            .values()
            .find(|s| s.name == name)
            .map(|s| s.id)
            .ok_or_else(|| format!("no such session: {name}"))?;
        if target == ctx.session {
            return Err("the window is in that session already".to_string());
        }
        let source = self
            .sessions
            .get(&ctx.session)
            .ok_or_else(|| "no such session".to_string())?;
        if source.windows.len() < 2 {
            return Err("this is the last window of its session".to_string());
        }
        let mut window = self
            .take_window(ctx)
            .ok_or_else(|| "no such window".to_string())?;
        let (base_index, renumber) = (self.config.base_index, self.config.renumber);
        let status = self.config.status_position;
        let window_id = window.id;
        let Some(destination) = self.sessions.get_mut(&target) else {
            return Err("no such session".to_string());
        };
        let (cols, rows) = window_area(destination.cols, destination.rows, status);
        window.layout.set_area(cols, rows);
        add_window(destination, window, base_index, renumber);
        destination.chrome_dirty = true;
        self.apply_window_sizes(target, window_id);
        Ok(String::new())
    }

    /// Read the configuration file again and replace the settings and the key tables with it.
    /// A file with a mistake changes nothing and says what is wrong.
    fn reload_config(&mut self, session_id: u64) -> Result<String, String> {
        let path = self
            .config_path
            .clone()
            .ok_or_else(|| "no configuration file is in use".to_string())?;
        match config::load(&path) {
            Ok(new) => {
                self.apply_config(new);
                self.log.line("configuration reloaded");
                Ok(match self.key_warning_line(session_id) {
                    Some(line) => format!("configuration reloaded; {line}"),
                    None => "configuration reloaded".to_string(),
                })
            }
            Err(errors) => {
                for e in &errors {
                    self.log.line(&format!("configuration: {e}"));
                }
                Err(summarize_config_errors(&errors))
            }
        }
    }

    fn apply_config(&mut self, new: Config) {
        let layout_changed = new.status_position != self.config.status_position;
        let underline_changed = new.links.underline != self.config.links.underline;
        self.keymap = new.keymap();
        if let Some(lines) = new.scrollback_lines {
            self.scrollback_lines = lines;
        }
        crate::theme::set_state_look(new.state_look);
        self.config = new;
        self.sync_sleep_guard();
        for s in self.sessions.values_mut() {
            s.title_inputs = None;
        }
        let (base_index, renumber) = (self.config.base_index, self.config.renumber);
        let mut resize = Vec::new();
        for s in self.sessions.values_mut() {
            s.key_state.prefix = keys::PrefixState::Idle;
            s.chrome_dirty = true;
            // Cells that are already composed carry the old look of a link: draw it all again.
            if underline_changed {
                s.need_full = true;
            }
            // The program and directory of each window are looked at again.
            s.names_checked = None;
            if renumber {
                for (i, w) in s.windows.iter_mut().enumerate() {
                    w.number = base_index + i;
                }
            }
            resize.push((s.id, s.cols, s.rows));
        }
        if layout_changed {
            for (id, cols, rows) in resize {
                self.resize_session(id, cols, rows);
            }
        }
    }

    /// Every command name that can be run by hand, in order: those of the lists themselves act
    /// on an open list only, so they are left out.
    fn command_names(&self) -> Vec<&'static str> {
        let mut names: Vec<&'static str> = commands::all_names()
            .filter(|n| !n.starts_with("list."))
            .collect();
        names.sort_unstable();
        names
    }

    /// The keys a command has, as they are typed: after the prefix, in the direct layer, in copy
    /// mode (only bindings that take no argument).
    fn keys_of(&self, name: &str) -> Vec<String> {
        let prefix = typed_name(self.config.keys.prefix);
        let in_table = |table: &crate::keymap::Table, label: &dyn Fn(Key) -> String| {
            let mut keys: Vec<String> = table
                .iter()
                .filter(|(_, b)| b.command == name && b.args.is_empty())
                .map(|(key, _)| label(*key))
                .collect();
            keys.sort();
            keys
        };
        let mut keys = in_table(&self.keymap.prefix, &|k| {
            format!("{prefix} {}", typed_name(k))
        });
        keys.extend(in_table(&self.keymap.direct, &typed_name));
        keys.extend(in_table(&self.keymap.copy, &|k| {
            format!("copy: {}", typed_name(k))
        }));
        keys
    }

    /// The commands as text, one per line: the name, a tab, the keys.
    fn list_commands(&self) -> String {
        self.command_names()
            .into_iter()
            .map(|n| format!("{n}\t{}", self.keys_of(n).join(", ")))
            .collect::<Vec<_>>()
            .join("\n")
    }

    /// The windows of a session as text, one per line: number, name, whether it is the one shown.
    fn list_windows(&self, session_id: u64) -> Result<String, String> {
        let session = self
            .sessions
            .get(&session_id)
            .ok_or_else(|| "no such session".to_string())?;
        Ok(session
            .windows
            .iter()
            .map(|w| {
                format!(
                    "{}\t{}\t{}",
                    w.number,
                    w.name,
                    u8::from(w.id == session.current)
                )
            })
            .collect::<Vec<_>>()
            .join("\n"))
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

    pub(super) fn set_focus(&mut self, ctx: Ctx, pane: u64) {
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
    pub(super) fn unzoom(&mut self, ctx: Ctx) {
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
    pub(super) fn window_changed(&mut self, ctx: Ctx) {
        self.apply_window_sizes(ctx.session, ctx.window);
        if let Some(session) = self.sessions.get_mut(&ctx.session) {
            session.relayout = true;
        }
    }

    /// The pane next to the one in `ctx` in the direction, looked for from the line its cursor is
    /// on; `None` when there is none.
    fn pane_towards(&self, ctx: Ctx, direction: Direction) -> Result<Option<PaneId>, String> {
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
        Ok(window.layout.neighbour(ctx.pane, direction, line))
    }

    fn focus_direction(&mut self, ctx: Ctx, direction: Direction) -> Result<String, String> {
        self.unzoom(ctx);
        if let Some(next) = self.pane_towards(ctx, direction)? {
            self.set_focus(ctx, next);
        }
        Ok(String::new())
    }

    /// The pane trades places with the one next to it; the focus stays with it.
    fn swap_pane(&mut self, ctx: Ctx, direction: Direction) -> Result<String, String> {
        self.unzoom(ctx);
        let Some(other) = self.pane_towards(ctx, direction)? else {
            return Err(format!(
                "no pane to swap with {}",
                direction_name(direction)
            ));
        };
        let window = self
            .sessions
            .get_mut(&ctx.session)
            .and_then(|s| s.window_mut(ctx.window))
            .ok_or_else(|| "no such window".to_string())?;
        window
            .layout
            .swap(ctx.pane, other)
            .map_err(|_| "no such pane".to_string())?;
        self.window_changed(ctx);
        Ok(String::new())
    }

    fn focus_last(&mut self, ctx: Ctx) -> Result<String, String> {
        if self.return_after_jump(ctx) {
            return Ok(String::new());
        }
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

    /// The pane leaves its window for a window of its own, which comes right after this one and
    /// is shown. The process and what is on its screen are untouched.
    fn break_pane(&mut self, ctx: Ctx) -> Result<String, String> {
        let session = self
            .sessions
            .get(&ctx.session)
            .ok_or_else(|| "no such session".to_string())?;
        let window = session
            .window(ctx.window)
            .ok_or_else(|| "no such window".to_string())?;
        if window.layout.len() < 2 {
            return Err("only one pane in this window".to_string());
        }
        let (cols, rows) = window_area(session.cols, session.rows, self.config.status_position);
        let cwd = self
            .panes
            .get(&ctx.pane)
            .and_then(|p| pty::process_cwd(p.pid))
            .unwrap_or_else(|| session.cwd.clone());
        let name = name_of_pane(&self.panes, self.config.window_name_from, ctx.pane, &cwd);
        self.unzoom(ctx);
        self.remove_leaf(ctx);
        let window_id = self.next_window;
        self.next_window += 1;
        let (base_index, renumber) = (self.config.base_index, self.config.renumber);
        let Some(session) = self.sessions.get_mut(&ctx.session) else {
            return Ok(String::new());
        };
        add_window_after(
            session,
            Window {
                id: window_id,
                number: 0,
                name,
                name_manual: false,
                program: String::new(),
                directory: String::new(),
                layout: Layout::new(ctx.pane, cols, rows),
                focus: ctx.pane,
                last_focus: None,
                zoomed: false,
                arrangement: None,
            },
            ctx.window,
            base_index,
            renumber,
        );
        self.select_window(ctx.session, window_id);
        self.apply_window_sizes(ctx.session, window_id);
        if let Some(session) = self.sessions.get_mut(&ctx.session) {
            session.chrome_dirty = true;
            session.relayout = true;
        }
        Ok(String::new())
    }

    /// Lay the panes out in the arrangement after the one applied last.
    fn layout_next(&mut self, ctx: Ctx) -> Result<String, String> {
        self.unzoom(ctx);
        let window = self
            .sessions
            .get_mut(&ctx.session)
            .and_then(|s| s.window_mut(ctx.window))
            .ok_or_else(|| "no such window".to_string())?;
        let next = crate::layout::Arrangement::after(window.arrangement);
        window
            .layout
            .arrange(next)
            .map_err(|_| "cannot arrange: a pane would be too small".to_string())?;
        window.arrangement = Some(next);
        self.window_changed(ctx);
        Ok(String::new())
    }

    /// Give the panes of the window the same room along each axis.
    fn layout_even(&mut self, ctx: Ctx) -> Result<String, String> {
        self.unzoom(ctx);
        let window = self
            .sessions
            .get_mut(&ctx.session)
            .and_then(|s| s.window_mut(ctx.window))
            .ok_or_else(|| "no such window".to_string())?;
        window
            .layout
            .even()
            .map_err(|_| "cannot even out: a pane would be too small".to_string())?;
        self.window_changed(ctx);
        Ok(String::new())
    }

    /// Turn the key capture of the pane on, or off when it is on.
    fn toggle_capture(&mut self, ctx: Ctx) -> Result<String, String> {
        let dir = self
            .socket_path
            .parent()
            .map(std::path::Path::to_path_buf)
            .ok_or_else(|| "no private directory for the capture".to_string())?;
        let window_label = self
            .sessions
            .get(&ctx.session)
            .and_then(|s| s.windows.iter().find(|w| w.id == ctx.window))
            .map_or_else(String::new, |w| format!("window {} {}", w.number, w.name));
        let pane = self
            .panes
            .get_mut(&ctx.pane)
            .ok_or_else(|| "no such pane".to_string())?;
        let message = if let Some(capture) = pane.capture.take() {
            format!(
                "key capture stopped in pane {} ({}): {}",
                ctx.pane,
                window_label,
                capture.path().display()
            )
        } else {
            let capture = crate::capture::Capture::start(&dir, ctx.pane)
                .map_err(|e| format!("cannot start the key capture: {e}"))?;
            let message = format!(
                "RECORDING every key typed in pane {} ({}), passwords too: {}",
                ctx.pane,
                window_label,
                capture.path().display()
            );
            pane.capture = Some(capture);
            message
        };
        if let Some(session) = self.sessions.get_mut(&ctx.session) {
            session.chrome_dirty = true;
        }
        Ok(message)
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

    /// A new window. Its shell starts in `directory` when one is given and exists, else where the
    /// shell of the pane that asked is now; the window is named after the directory its own
    /// shell starts in.
    fn new_window(&mut self, ctx: Ctx, directory: Option<String>) -> Result<String, String> {
        let session = self
            .sessions
            .get(&ctx.session)
            .ok_or_else(|| "no such session".to_string())?;
        let (cols, rows) = window_area(session.cols, session.rows, self.config.status_position);
        let cwd = directory
            .filter(|d| std::path::Path::new(d).is_dir())
            .or_else(|| {
                self.panes
                    .get(&ctx.pane)
                    .and_then(|p| pty::process_cwd(p.pid))
            })
            .unwrap_or_else(|| session.cwd.clone());
        let env = session.env.clone();
        let pane = self.spawn_pane(Vec::new(), cwd.clone(), &env, cols, rows)?;
        let pane_id = pane.id;
        self.panes.insert(pane_id, pane);
        let window_id = self.next_window;
        self.next_window += 1;
        let (base_index, renumber) = (self.config.base_index, self.config.renumber);
        let name = name_at_creation(
            self.config.window_name_from,
            &cwd,
            first_program_name(&[], &env),
        );
        let Some(session) = self.sessions.get_mut(&ctx.session) else {
            return Ok(String::new());
        };
        add_window(
            session,
            Window {
                id: window_id,
                number: 0,
                name,
                name_manual: false,
                program: String::new(),
                directory: String::new(),
                layout: Layout::new(pane_id, cols, rows),
                focus: pane_id,
                last_focus: None,
                zoomed: false,
                arrangement: None,
            },
            base_index,
            renumber,
        );
        self.select_window(ctx.session, window_id);
        Ok(String::new())
    }

    /// Show another window; the one shown now becomes the one to go back to.
    pub(super) fn select_window(&mut self, session_id: u64, window_id: u64) {
        let at = self.started.elapsed().as_secs_f64();
        let cause = self.cause_text();
        let Some(session) = self.sessions.get_mut(&session_id) else {
            return;
        };
        if session.current == window_id || session.window(window_id).is_none() {
            return;
        }
        let from = session.window(session.current).map_or(0, |w| w.number);
        let to = session.window(window_id).map_or(0, |w| w.number);
        let kind = ChangeKind::Window {
            session: session.name.clone(),
            from,
            to,
            cause,
        };
        session.last_window = Some(session.current);
        session.current = window_id;
        session.relayout = true;
        session.chrome_dirty = true;
        keep_change(&mut session.window_changes, at, kind);
    }

    /// The cause of a window change as it is kept: what was set, or a plain statement that none
    /// was recorded.
    fn cause_text(&self) -> String {
        self.cause
            .clone()
            .unwrap_or_else(|| "not from a key, the bar, a command or a list".to_string())
    }

    /// Run `f` with `cause` as the reason for any window change it makes, then put back the
    /// reason that was there. The reason that was named first is kept, so a change made under
    /// the notification jump keeps that name, whatever helper it went through. The one exception
    /// is a key: a list or the tree that a key opened names its own choice, since the key only
    /// asked for the list.
    pub(super) fn with_cause<T>(
        &mut self,
        cause: impl Into<String>,
        f: impl FnOnce(&mut Self) -> T,
    ) -> T {
        if self.cause.as_deref().is_some_and(|c| !is_key_cause(c)) {
            return f(self);
        }
        let previous = self.cause.replace(cause.into());
        let out = f(self);
        self.cause = previous;
        out
    }

    /// The latest changes of the windows shown, in every session, newest first.
    fn window_log(&self) -> String {
        let mut lines: Vec<(f64, String)> = self
            .sessions
            .values()
            .flat_map(|s| {
                s.window_changes
                    .iter()
                    .chain(s.focus_changes.iter())
                    .map(|c| (c.at, c.kind.text()))
            })
            .collect();
        lines.sort_by(|a, b| b.0.total_cmp(&a.0));
        lines
            .iter()
            .map(|(at, text)| format!("{at:>10.3} s  {text}"))
            .collect::<Vec<_>>()
            .join("\n")
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
        // The windows trade places, and the numbers stay with the places.
        session.windows.swap(at, to as usize);
        let (a, b) = (
            session.windows[at].number,
            session.windows[to as usize].number,
        );
        session.windows[at].number = b;
        session.windows[to as usize].number = a;
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
    pub(super) fn remove_leaf(&mut self, ctx: Ctx) {
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
        self.take_window(ctx);
    }

    /// Take a window out of its session, which must have another, and show another window. The
    /// window and its panes carry on; the caller decides what becomes of them.
    pub(super) fn take_window(&mut self, ctx: Ctx) -> Option<Window> {
        let (base_index, renumber) = (self.config.base_index, self.config.renumber);
        let at = self.started.elapsed().as_secs_f64();
        let cause = self.cause_text();
        let session = self.sessions.get_mut(&ctx.session)?;
        let index = session.windows.iter().position(|w| w.id == ctx.window)?;
        let window = session.windows.remove(index);
        if renumber {
            for (i, w) in session.windows.iter_mut().enumerate() {
                w.number = base_index + i;
            }
        }
        if session.last_window == Some(ctx.window) {
            session.last_window = None;
        }
        if session.current == ctx.window {
            let next = session
                .last_window
                .take()
                .filter(|id| session.windows.iter().any(|w| w.id == *id))
                .unwrap_or_else(|| session.windows[index.min(session.windows.len() - 1)].id);
            let to = session.window(next).map_or(0, |w| w.number);
            let kind = ChangeKind::Window {
                session: session.name.clone(),
                from: window.number,
                to,
                cause,
            };
            session.current = next;
            keep_change(&mut session.window_changes, at, kind);
        }
        session.relayout = true;
        session.chrome_dirty = true;
        Some(window)
    }

    /// End a session: hang up every pane and tell the client.
    ///
    /// A client that was in it goes to another session, when there is one nobody else is
    /// attached to and the settings say so (`[session] on-end`).
    pub(super) fn end_session(&mut self, session_id: u64, note: &str) {
        let Some(session) = self.sessions.remove(&session_id) else {
            return;
        };
        for pane in session.all_panes() {
            self.drop_pane(pane);
        }
        if let Some(client) = session.client
            && self.config.on_session_end == config::OnSessionEnd::Switch
            && let Some(target) = self.session_to_go_to()
        {
            self.log
                .line(&format!("session {} closed: {note}", session.id));
            self.bind_client(client, target, session.cols, session.rows);
            let now = self
                .sessions
                .get(&target)
                .map(|s| s.name.clone())
                .unwrap_or_default();
            self.set_message(
                target,
                &format!("session {} ended; now in {now}", session.name),
            );
            return;
        }
        self.finish_session(&session, note);
    }

    /// The session a client goes to when its own ends: the one that was used last, and when none
    /// ever was, the one made first. Only a session with no client of its own: no one is thrown
    /// out of theirs.
    fn session_to_go_to(&self) -> Option<u64> {
        let mut free: Vec<&Session> = self
            .sessions
            .values()
            .filter(|s| s.client.is_none())
            .collect();
        free.sort_by_key(|s| s.id);
        let used = free
            .iter()
            .filter(|s| s.last_used > 0)
            .max_by_key(|s| s.last_used);
        used.or(free.first()).map(|s| s.id)
    }

    /// A session that is already out of the books: log it and tell its client goodbye. The
    /// panes are the caller's to deal with.
    pub(super) fn finish_session(&mut self, session: &Session, note: &str) {
        self.log
            .line(&format!("session {} closed: {note}", session.id));
        if let Some(client_id) = session.client {
            self.send_output(client_id, crate::render::MOUSE_OFF);
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

/// A binding's command with its arguments, as it would be typed.
fn binding_line(binding: &Binding) -> String {
    if binding.args.is_empty() {
        binding.command.clone()
    } else {
        format!("{} {}", binding.command, binding.args.join(" "))
    }
}

fn direction_name(direction: Direction) -> &'static str {
    match direction {
        Direction::Left => "left",
        Direction::Down => "down",
        Direction::Up => "up",
        Direction::Right => "right",
    }
}

#[cfg(test)]
mod pane_info_tests {
    use super::pane_info_line;

    #[test]
    fn the_info_line_names_version_session_window_pane_title_and_the_clock_on_one_line() {
        assert_eq!(
            pane_info_line(
                "0.1.5",
                "main work",
                1,
                "everfit-api",
                147,
                "vim notes.txt",
                8272.04
            ),
            "arcade 0.1.5 · session \"main work\" · window 1 \"everfit-api\" · pane 147 \"vim notes.txt\" · at 8272.0 s"
        );
        // A title with a line break or a quote cannot break the line.
        let line = pane_info_line("0.1.5", "a", 2, "b", 3, "x\ny\"z", 1.0);
        assert!(!line.contains('\n'), "{line}");
        assert!(line.contains("pane 3 \"x\\ny\\\"z\""), "{line}");
    }
}
