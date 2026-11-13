//! Building the screen update for a client: when it may go out, and what goes in it.

use super::*;
use crate::render::HostModes;
use crate::status::{self, StatusInfo, Tab};

/// The row of the client's screen the status bar is on, if there is room for one.
fn status_row(rows: u16) -> Option<u16> {
    (rows > STATUS_ROWS).then(|| rows - STATUS_ROWS)
}

fn status_info(s: &Session) -> StatusInfo {
    StatusInfo {
        session: s.name.clone(),
        tabs: s
            .windows
            .iter()
            .enumerate()
            .map(|(index, w)| Tab {
                number: index + 1,
                name: w.name.clone(),
                current: w.id == s.current,
                zoomed: w.zoomed,
            })
            .collect(),
        prefix: false,
        locked: false,
        copy: false,
        message: None,
        prompt: None,
    }
}

impl Server {
    /// When the next screen update for this session may be sent: `None` when there is nothing
    /// to send or the client is still busy taking the last one.
    pub(super) fn frame_time(&self, s: &Session, now: Instant) -> Option<Instant> {
        let ready = s
            .client
            .and_then(|c| self.clients.get(&c))
            .is_some_and(|c| c.out.is_empty() && !c.gone);
        if !ready {
            return None;
        }
        let mut dirty = false;
        let mut heavy = false;
        for (pane_id, _) in s.current_window().visible_rects() {
            if let Some(pane) = self.panes.get(&pane_id)
                && pane.dirty
            {
                dirty = true;
                heavy |= !pane.rate.is_light();
            }
        }
        if !(dirty || s.need_full || s.relayout || s.chrome_dirty) {
            return None;
        }
        // The 60 per second limit is for floods. A light update (an echoed key) goes out at
        // once, so typing is not delayed by the pacing of the update before it.
        if !heavy {
            return Some(now);
        }
        Some(s.last_frame.map_or(now, |t| t + FRAME_INTERVAL))
    }

    pub(super) fn needs_frame(&self) -> bool {
        let now = Instant::now();
        self.sessions
            .values()
            .any(|s| self.frame_time(s, now).is_some_and(|t| t <= now))
    }

    pub(super) fn send_frames(&mut self) {
        let now = Instant::now();
        let due: Vec<u64> = self
            .sessions
            .values()
            .filter(|s| self.frame_time(s, now).is_some_and(|t| t <= now))
            .map(|s| s.id)
            .collect();
        for id in due {
            self.build_frame_for(id);
        }
    }

    /// Build the screen update for a session's client and queue it.
    pub(super) fn build_frame_for(&mut self, session_id: u64) {
        let Some(session) = self.sessions.get_mut(&session_id) else {
            return;
        };
        let Some(client_id) = session.client else {
            return;
        };
        let visible = session.current_window().visible_rects();
        let panes_dirty = visible
            .iter()
            .any(|(p, _)| self.panes.get(p).is_some_and(|p| p.dirty));
        if !(panes_dirty || session.need_full || session.relayout || session.chrome_dirty) {
            return;
        }
        // The names of the windows come from the panes' directories. They are looked at only
        // now, when an update is going out anyway, and at most once a second.
        let now = Instant::now();
        if session
            .names_checked
            .is_none_or(|t| now.duration_since(t) >= NAME_CHECK_INTERVAL)
        {
            session.names_checked = Some(now);
            for window in session.windows.iter_mut().filter(|w| !w.name_manual) {
                let name = self
                    .panes
                    .get(&window.focus)
                    .and_then(|p| pty::process_cwd(p.pid))
                    .map(|cwd| directory_name(&cwd));
                if let Some(name) = name
                    && name != window.name
                {
                    window.name = name;
                    session.chrome_dirty = true;
                }
            }
        }
        let full = session.need_full;
        let relayout = session.relayout || full;
        let (cols, rows) = (session.cols, session.rows);
        if full {
            // The client's screen is new or changed shape: nothing of it can be trusted.
            session.renderer.reset();
            session.screen.resize(cols, rows);
        } else if relayout {
            session.screen.clear();
        }
        let mut failed = Vec::new();
        for (pane_id, rect) in &visible {
            let Some(pane) = self.panes.get_mut(pane_id) else {
                continue;
            };
            if !(relayout || pane.dirty) {
                continue;
            }
            match pane.term.snapshot(relayout) {
                Ok(frame) => {
                    pane.last_cursor = frame.cursor;
                    pane.dirty = false;
                    session.screen.blit(*rect, frame);
                }
                Err(e) => {
                    pane.exit_note = Some(format!("[{e}]"));
                    pty::signal_group(pane.pid, libc::SIGKILL);
                    failed.push(*pane_id);
                }
            }
        }
        let window = session.current_window();
        let focus = window.focus;
        let focus_rect = visible.iter().find(|(p, _)| *p == focus).map(|(_, r)| *r);
        let mut cursor_override = None;
        if relayout || session.chrome_dirty {
            let dividers = if window.zoomed {
                Vec::new()
            } else {
                window.layout.dividers()
            };
            session.screen.draw_dividers(&dividers, 0, focus_rect);
            if let Some(y) = status_row(rows) {
                let (cells, prompt_cursor) = status::build(cols, &status_info(session));
                session.screen.set_row(y, cells);
                cursor_override = prompt_cursor.map(|x| (x, y));
            }
        }
        let cursor = match (cursor_override, focus_rect.zip(self.panes.get(&focus))) {
            (Some((x, y)), _) => arcade_term::Cursor {
                x,
                y,
                visible: true,
                shape: CursorShape::Bar,
                blinking: false,
            },
            (None, Some((rect, pane))) => {
                let c = pane.last_cursor;
                arcade_term::Cursor {
                    x: rect.x + c.x,
                    y: rect.y + c.y,
                    visible: c.visible && c.x < rect.w && c.y < rect.h,
                    shape: c.shape,
                    blinking: c.blinking,
                }
            }
            (None, None) => arcade_term::Cursor {
                x: 0,
                y: 0,
                visible: false,
                shape: CursorShape::Block,
                blinking: false,
            },
        };
        let modes = self
            .panes
            .get(&focus)
            .map_or_else(HostModes::default, |p| HostModes {
                app_cursor_keys: p
                    .term
                    .mode(arcade_term::Mode::AppCursorKeys)
                    .unwrap_or(false),
                bracketed_paste: p
                    .term
                    .mode(arcade_term::Mode::BracketedPaste)
                    .unwrap_or(false),
            });
        let Some(session) = self.sessions.get_mut(&session_id) else {
            return;
        };
        session.need_full = false;
        session.relayout = false;
        session.chrome_dirty = false;
        let bytes = session.renderer.render(&mut session.screen, &cursor, modes);
        if !bytes.is_empty() {
            session.last_frame = Some(Instant::now());
            self.send_output(client_id, &bytes);
        }
        self.exited.extend(failed);
    }
}
