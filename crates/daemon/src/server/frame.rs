//! Building the screen update for a client: when it may go out, and what goes in it.

use super::*;
use crate::render::HostModes;
use crate::status::{self, StatusInfo, Tab};

fn status_info(s: &Session, copy: bool) -> StatusInfo {
    StatusInfo {
        session: s.name.clone(),
        tabs: s
            .windows
            .iter()
            .map(|w| Tab {
                number: w.number,
                name: w.name.clone(),
                current: w.id == s.current,
                zoomed: w.zoomed,
            })
            .collect(),
        prefix: s.key_state.prefix_active(),
        locked: s.key_state.locked,
        copy,
        message: s.key_state.message.clone(),
        prompt: s.key_state.status_prompt(),
    }
}

/// The longest pane title that goes to the host terminal, in characters.
const TITLE_LIMIT: usize = 200;

/// Text from a program or a name, made safe to put inside an escape sequence for the host
/// terminal: no control characters of any kind, and not longer than `limit` characters.
fn clean_title(text: &str, limit: usize) -> String {
    text.chars()
        .filter(|c| !c.is_control())
        .take(limit)
        .collect()
}

impl Server {
    /// The title of the host terminal's window: what the focused pane's program calls itself
    /// (or the window's name when it has set no title), then the session and the window number.
    fn host_title(&self, s: &Session) -> String {
        let window = s.current_window();
        let pane_title = self
            .panes
            .get(&window.focus)
            .and_then(|p| p.title.as_deref())
            .unwrap_or(&window.name);
        format!(
            "{} \u{b7} {}:{}",
            clean_title(pane_title, TITLE_LIMIT),
            clean_title(&s.name, TITLE_LIMIT),
            window.number
        )
    }

    /// Tell each attached client's terminal the title of its window, when and only when it is
    /// not the one it was told last.
    pub(super) fn send_titles(&mut self) {
        let mut due = Vec::new();
        for s in self.sessions.values() {
            let Some(client) = s.client else {
                continue;
            };
            if !self.clients.get(&client).is_some_and(|c| !c.gone) {
                continue;
            }
            let title = self.host_title(s);
            if s.sent_title.as_deref() != Some(title.as_str()) {
                due.push((s.id, client, title));
            }
        }
        for (session_id, client, title) in due {
            self.send_output(client, format!("\x1b]2;{title}\x07").as_bytes());
            if let Some(s) = self.sessions.get_mut(&session_id) {
                s.sent_title = Some(title);
            }
        }
    }

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
                && pane.wants_frame()
            {
                dirty = true;
                heavy |= pane.copy.is_none() && !pane.rate.is_light();
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
        let status_position = self.config.status_position;
        let Some(session) = self.sessions.get_mut(&session_id) else {
            return;
        };
        let Some(client_id) = session.client else {
            return;
        };
        let origin = window_origin(session.rows, status_position);
        let visible: Vec<(PaneId, Rect)> = session
            .current_window()
            .visible_rects()
            .into_iter()
            .map(|(p, r)| (p, Rect::new(r.x, r.y + origin, r.w, r.h)))
            .collect();
        let panes_dirty = visible
            .iter()
            .any(|(p, _)| self.panes.get(p).is_some_and(Pane::wants_frame));
        if !(panes_dirty || session.need_full || session.relayout || session.chrome_dirty) {
            return;
        }
        // The names of the windows come from the panes' directories. They are looked at only
        // now, when an update is going out anyway, and at most once a second. A look that is
        // skipped for being too soon leaves one deadline for the loop, at the moment a second
        // is full, so a directory changed in the meantime is seen then; no deadline is left
        // once the names have been looked at.
        let now = Instant::now();
        match session.names_checked {
            Some(t) if now.duration_since(t) < NAME_CHECK_INTERVAL => {
                session.names_due = Some(t + NAME_CHECK_INTERVAL);
            }
            _ => {
                session.names_checked = Some(now);
                session.names_due = None;
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
            // A pane in copy mode shows its view of the rows, not what the program draws; what
            // the program draws is read again when copy mode ends.
            if let Some(copy) = pane.copy.as_mut() {
                pane.dirty = false;
                if relayout || copy.dirty {
                    copy.dirty = false;
                    let rows = copy
                        .view(&pane.term)
                        .into_iter()
                        .enumerate()
                        .map(|(y, cells)| arcade_term::RowUpdate { y: y as u16, cells })
                        .collect();
                    session.screen.blit(
                        *rect,
                        arcade_term::Frame {
                            cols: rect.w,
                            rows_total: rect.h,
                            damage: arcade_term::Damage::Full,
                            rows,
                            cursor: pane.last_cursor,
                        },
                    );
                }
                continue;
            }
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
            session.screen.draw_dividers(&dividers, origin, focus_rect);
            if let Some(y) = status_row(rows, status_position) {
                let copy_on = self.panes.get(&focus).is_some_and(|p| p.copy.is_some());
                let (cells, prompt_cursor) = status::build(cols, &status_info(session, copy_on));
                session.screen.set_row(y, cells);
                cursor_override = prompt_cursor.map(|x| (x, y));
            }
        }
        // The floating list is drawn over everything on every update.
        let picker_open = session.picker.is_some();
        if let Some(picker) = &session.picker {
            let drawn = picker.draw(cols, rows);
            session.screen.overlay(drawn.x, drawn.y, &drawn.rows);
        }
        let cursor_override = if picker_open {
            Some((0, 0))
        } else {
            cursor_override
        };
        let cursor = match (cursor_override, focus_rect.zip(self.panes.get(&focus))) {
            (Some(_), _) if picker_open => arcade_term::Cursor {
                x: 0,
                y: 0,
                visible: false,
                shape: CursorShape::Block,
                blinking: false,
            },
            (Some((x, y)), _) => arcade_term::Cursor {
                x,
                y,
                visible: true,
                shape: CursorShape::Bar,
                blinking: false,
            },
            (None, Some((rect, pane))) if pane.copy.is_some() => {
                let shown = pane.copy.as_ref().and_then(|c| c.cursor_in_view());
                arcade_term::Cursor {
                    x: rect.x + shown.map_or(0, |(x, _)| x),
                    y: rect.y + shown.map_or(0, |(_, y)| y),
                    visible: shown.is_some(),
                    shape: CursorShape::Block,
                    blinking: false,
                }
            }
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_title_loses_every_control_character_and_is_cut_to_the_limit() {
        assert_eq!(clean_title("a\x1b[31m\x07b\u{9b}c\x7f", 200), "a[31mbc");
        let long = "x".repeat(500);
        assert_eq!(clean_title(&long, TITLE_LIMIT).chars().count(), TITLE_LIMIT);
        // Characters, not bytes: a wide character is not cut in the middle.
        let wide = "\u{6f22}".repeat(300);
        assert_eq!(clean_title(&wide, TITLE_LIMIT).chars().count(), TITLE_LIMIT);
    }
}
