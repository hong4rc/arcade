//! The mouse: clicking a pane to focus it, turning the wheel to scroll, dragging to select.
//!
//! The host terminal reports the mouse in the SGR form (see `render::MOUSE_ON`). A pane whose
//! program asked for the mouse gets the reports, written the way that program asked for them;
//! every other pane is Arcade's: the wheel scrolls its copy mode and a drag selects text, without
//! copying. Holding Alt makes Arcade take the mouse from a program that asked for it.

use arcade_term::{Mods, MouseLevel};

use super::actions::Ctx;
use super::*;
use crate::keys::{Mouse, MouseKind, Wheel};

/// Rows one turn of the wheel scrolls.
const WHEEL_ROWS: isize = 3;

/// A button held down since a press: where it began, and who gets the moves and the release.
#[derive(Clone, Copy, Debug)]
pub(super) struct Drag {
    pane: u64,
    /// The pane's place on the client's screen.
    rect: Rect,
    /// The cell of the press, within the pane.
    from: (u16, u16),
    /// The program in the pane gets the moves and the release, not Arcade.
    forward: bool,
    /// The mouse has left the cell it pressed in, so a selection has begun.
    selecting: bool,
    /// While the pointer is at the first row of the pane or above it (negative) or at the last
    /// row or below it: the rows the view scrolls at each tick, more the further out it is.
    edge: isize,
    /// The column of the pointer, within the pane, for the cells the scrolling selects.
    col: u16,
    /// When the view scrolls next; none while the pointer is inside the pane or the view cannot
    /// move, so a drag that is not at an edge costs no wake-up.
    pub(super) next_tick: Option<Instant>,
}

/// How often the view scrolls while a selection is dragged at an edge of the pane.
const EDGE_TICK: Duration = Duration::from_millis(50);
/// The most rows one tick scrolls.
const EDGE_MAX_ROWS: isize = 5;

/// The bytes that tell a program about a mouse event at `column` and `row` of its pane (from 0),
/// or nothing when the program did not ask for events of that kind or cannot be told (the old
/// form cannot name a column past 222).
fn encode(
    level: MouseLevel,
    sgr: bool,
    kind: MouseKind,
    mods: Mods,
    column: u16,
    row: u16,
) -> Option<Vec<u8>> {
    let wanted = match kind {
        MouseKind::Press(_) | MouseKind::Release(_) | MouseKind::Wheel(_) => MouseLevel::Press,
        MouseKind::Drag(_) => MouseLevel::Drag,
        MouseKind::Move => MouseLevel::Any,
    };
    if level < wanted {
        return None;
    }
    let mut code: u16 = match kind {
        MouseKind::Press(b) => u16::from(b),
        // The old form does not say which button came up.
        MouseKind::Release(b) => {
            if sgr {
                u16::from(b)
            } else {
                3
            }
        }
        MouseKind::Drag(b) => 32 + u16::from(b),
        MouseKind::Move => 35,
        MouseKind::Wheel(Wheel::Up) => 64,
        MouseKind::Wheel(Wheel::Down) => 65,
        MouseKind::Wheel(Wheel::Left) => 66,
        MouseKind::Wheel(Wheel::Right) => 67,
    };
    code += u16::from(mods.shift) * 4 + u16::from(mods.alt) * 8 + u16::from(mods.ctrl) * 16;
    if sgr {
        let end = if matches!(kind, MouseKind::Release(_)) {
            'm'
        } else {
            'M'
        };
        return Some(format!("\x1b[<{code};{};{}{end}", column + 1, row + 1).into_bytes());
    }
    let (column, row) = (column + 1 + 32, row + 1 + 32);
    if column > 255 || row > 255 {
        return None;
    }
    Some(vec![
        0x1b,
        b'[',
        b'M',
        (code + 32) as u8,
        column as u8,
        row as u8,
    ])
}

fn clamp_into(rect: Rect, x: u16, y: u16) -> (u16, u16) {
    (
        x.clamp(rect.x, rect.right().saturating_sub(1)) - rect.x,
        y.clamp(rect.y, rect.bottom().saturating_sub(1)) - rect.y,
    )
}

impl Server {
    pub(super) fn handle_mouse(&mut self, session_id: u64, m: Mouse) {
        if self.in_prompt(session_id) || self.picker_open(session_id) {
            return;
        }
        let Some(session) = self.sessions.get(&session_id) else {
            return;
        };
        let locked = session.key_state.locked;
        // A divider held since a press on it: the moves and the release are its, wherever they
        // are on the screen. A new press means the release was lost, so that one starts over.
        if let Some((window_id, handle)) = session.divider_drag.clone() {
            match m.kind {
                MouseKind::Drag(_) => self.drag_divider(session_id, window_id, &handle, m),
                MouseKind::Release(_) => {
                    self.drag_divider(session_id, window_id, &handle, m);
                    self.end_divider_drag(session_id);
                }
                MouseKind::Press(_) => self.end_divider_drag(session_id),
                _ => {}
            }
            if !matches!(m.kind, MouseKind::Press(_)) {
                return;
            }
        }
        let Some(session) = self.sessions.get(&session_id) else {
            return;
        };
        // A left press on a tab of the status bar shows that window. Nothing else on the bar
        // does anything, and none of it goes to a pane or starts a selection.
        // (A drag that began in a pane goes on being that pane's, also over the bar.)
        let on_bar = status_row(session.rows, self.config.status_position) == Some(m.y);
        if on_bar && (matches!(m.kind, MouseKind::Press(_)) || session.drag.is_none()) {
            let target = match m.kind {
                MouseKind::Press(0) => session
                    .tab_bar
                    .iter()
                    .find(|(_, from, to)| (*from..*to).contains(&m.x))
                    .map(|(window, _, _)| *window),
                _ => None,
            };
            let on_counter = matches!(m.kind, MouseKind::Press(0))
                && session
                    .counter_bar
                    .is_some_and(|(from, to)| (from..to).contains(&m.x));
            self.set_drag(session_id, None);
            if on_counter {
                self.open_notifications_from_counter(session_id);
            } else if let Some(window) = target {
                let column = m.x;
                self.with_cause(format!("a click on the bar, column {column}"), |s| {
                    s.select_window(session_id, window)
                });
            }
            return;
        }
        let origin = window_origin(session.rows, self.config.status_position);
        let window = session.current_window();
        let window_id = window.id;
        let hit = window
            .visible_rects()
            .into_iter()
            .map(|(p, r)| (p, Rect::new(r.x, r.y + origin, r.w, r.h)))
            .find(|(_, r)| r.contains(m.x, m.y));
        // A left press on a divider holds it (the cell where one divider ends on another belongs
        // to the one it ends on). It is Arcade's even when a program beside it asked for the
        // mouse: a divider is not part of a pane. Nothing is sent to a pane and no selection
        // starts; a press that is not followed by a drag changes nothing. A zoomed pane has none.
        if matches!(m.kind, MouseKind::Press(0))
            && !locked
            && !window.zoomed
            && m.y >= origin
            && let Some(handle) = window.layout.divider_at(m.x, m.y - origin)
        {
            self.set_drag(session_id, None);
            if let Some(s) = self.sessions.get_mut(&session_id) {
                s.divider_drag = Some((window_id, handle));
            }
            return;
        }
        match m.kind {
            MouseKind::Press(button) => {
                let Some((pane, rect)) = hit else {
                    self.set_drag(session_id, None);
                    return;
                };
                let ours = !locked && self.arcade_takes_mouse(pane, m.mods);
                if button == 0 && !locked {
                    self.focus_by_mouse(session_id, window_id, pane);
                }
                let from = (m.x - rect.x, m.y - rect.y);
                let drag = Drag {
                    pane,
                    rect,
                    from,
                    forward: !ours,
                    selecting: false,
                    edge: 0,
                    col: from.0,
                    next_tick: None,
                };
                // A pane that is Arcade's only starts a selection with the left button.
                self.set_drag(session_id, (!ours || button == 0).then_some(drag));
                if !ours {
                    self.forward_mouse(pane, m, from);
                }
            }
            MouseKind::Drag(_) | MouseKind::Release(_) => {
                let drag = self.sessions.get(&session_id).and_then(|s| s.drag);
                let Some(drag) = drag else { return };
                let to = clamp_into(drag.rect, m.x, m.y);
                if drag.forward {
                    self.forward_mouse(drag.pane, m, to);
                } else if matches!(m.kind, MouseKind::Drag(_))
                    && (drag.selecting || to != drag.from)
                {
                    self.select_by_mouse(session_id, drag, to);
                    self.watch_edge(session_id, m, to);
                }
                if matches!(m.kind, MouseKind::Release(_)) {
                    self.set_drag(session_id, None);
                }
            }
            MouseKind::Move => {
                if let Some((pane, rect)) = hit
                    && (locked || !self.arcade_takes_mouse(pane, m.mods))
                {
                    self.forward_mouse(pane, m, (m.x - rect.x, m.y - rect.y));
                }
            }
            MouseKind::Wheel(direction) => {
                let Some((pane, rect)) = hit else { return };
                if locked || !self.arcade_takes_mouse(pane, m.mods) {
                    self.forward_mouse(pane, m, (m.x - rect.x, m.y - rect.y));
                } else {
                    match direction {
                        Wheel::Up => self.scroll_by_mouse(session_id, pane, -WHEEL_ROWS),
                        Wheel::Down => self.scroll_by_mouse(session_id, pane, WHEEL_ROWS),
                        Wheel::Left | Wheel::Right => {}
                    }
                }
            }
        }
    }

    /// Move the held divider under the pointer. The layout changes at once; the programs are told
    /// the new sizes when the frame goes out.
    fn drag_divider(
        &mut self,
        session_id: u64,
        window_id: u64,
        handle: &crate::layout::DividerHandle,
        m: Mouse,
    ) {
        let status_position = self.config.status_position;
        let Some(session) = self.sessions.get_mut(&session_id) else {
            return;
        };
        let origin = window_origin(session.rows, status_position);
        let to = if handle.vertical() {
            m.x
        } else {
            m.y.saturating_sub(origin)
        };
        let Some(window) = session.window_mut(window_id) else {
            session.divider_drag = None;
            return;
        };
        match window.layout.drag_divider(handle, to) {
            Ok(moved) => {
                if moved {
                    session.sizes_stale = Some(window_id);
                    session.relayout = true;
                }
            }
            // The divider is gone (a pane closed) or the window is too small: the drag is over.
            Err(_) => session.divider_drag = None,
        }
    }

    fn end_divider_drag(&mut self, session_id: u64) {
        if let Some(s) = self.sessions.get_mut(&session_id) {
            s.divider_drag = None;
            // The last frame of the drag is not held back.
            s.relayout = true;
        }
    }

    fn set_drag(&mut self, session_id: u64, drag: Option<Drag>) {
        if let Some(s) = self.sessions.get_mut(&session_id) {
            s.drag = drag;
        }
    }

    /// Arcade, and not the program in the pane, gets the mouse: the pane shows copy mode, or
    /// its program did not ask for the mouse, or Alt is held.
    fn arcade_takes_mouse(&self, pane: u64, mods: Mods) -> bool {
        self.panes.get(&pane).is_some_and(|p| {
            p.copy.is_some() || mods.alt || p.term.mouse_level() == MouseLevel::Off
        })
    }

    fn focus_by_mouse(&mut self, session_id: u64, window_id: u64, pane: u64) {
        self.set_focus(
            Ctx {
                session: session_id,
                window: window_id,
                pane,
            },
            pane,
        );
    }

    /// Tell the program in the pane about the event, if it asked for events like it.
    fn forward_mouse(&mut self, pane: u64, m: Mouse, at: (u16, u16)) {
        let Some(p) = self.panes.get(&pane) else {
            return;
        };
        let sgr = p.term.mode(arcade_term::Mode::MouseSgr) == Some(true);
        if let Some(bytes) = encode(p.term.mouse_level(), sgr, m.kind, m.mods, at.0, at.1) {
            self.write_to_pane(pane, &bytes);
        }
    }

    /// Whether the pointer of a selection being dragged is at the first row of its pane or above
    /// it, or at the last row or below it: then the view scrolls by itself (see
    /// `tick_drag_scrolls`), also while the pointer stands still.
    fn watch_edge(&mut self, session_id: u64, m: Mouse, to: (u16, u16)) {
        let Some(drag) = self
            .sessions
            .get_mut(&session_id)
            .and_then(|s| s.drag.as_mut())
        else {
            return;
        };
        let rect = drag.rect;
        let last = rect.bottom().saturating_sub(1);
        let outside = |distance: u16| (1 + distance as isize).min(EDGE_MAX_ROWS);
        drag.edge = if m.y <= rect.y {
            -outside(rect.y - m.y)
        } else if m.y >= last {
            outside(m.y - last)
        } else {
            0
        };
        drag.col = to.0;
        if drag.edge == 0 {
            drag.next_tick = None;
        } else if drag.next_tick.is_none() {
            drag.next_tick = Some(Instant::now() + EDGE_TICK);
        }
    }

    /// The selections that are being dragged at an edge scroll one step.
    pub(super) fn tick_drag_scrolls(&mut self) {
        let now = Instant::now();
        let due: Vec<u64> = self
            .sessions
            .values()
            .filter(|s| {
                s.drag
                    .is_some_and(|d| d.next_tick.is_some_and(|t| t <= now))
            })
            .map(|s| s.id)
            .collect();
        for id in due {
            self.drag_scroll_tick(id, now);
        }
    }

    fn drag_scroll_tick(&mut self, session_id: u64, now: Instant) {
        let Some(session) = self.sessions.get(&session_id) else {
            return;
        };
        let Some(drag) = session.drag else { return };
        // The drag is over when the pane is not where it began (closed, resized, another window
        // shown), the client is gone, or the pane left copy mode.
        let origin = window_origin(session.rows, self.config.status_position);
        let in_place = session.client.is_some()
            && session
                .current_window()
                .visible_rects()
                .into_iter()
                .any(|(p, r)| {
                    p == drag.pane && Rect::new(r.x, r.y + origin, r.w, r.h) == drag.rect
                });
        let moved = if in_place {
            self.panes.get_mut(&drag.pane).and_then(|pane| {
                let copy = pane.copy.as_mut()?;
                let row = if drag.edge < 0 {
                    0
                } else {
                    copy.rows.saturating_sub(1)
                };
                Some(copy.drag_scroll(&pane.term, drag.edge, (drag.col, row)))
            })
        } else {
            None
        };
        if let Some(s) = self.sessions.get_mut(&session_id) {
            match moved {
                None => s.drag = None,
                Some(moved) => {
                    if let Some(d) = s.drag.as_mut() {
                        d.next_tick = moved.then_some(now + EDGE_TICK);
                    }
                }
            }
        }
    }

    fn select_by_mouse(&mut self, session_id: u64, drag: Drag, to: (u16, u16)) {
        let Some(pane) = self.panes.get_mut(&drag.pane) else {
            return;
        };
        let opened = pane.copy.is_none();
        if opened {
            let (cols, rows) = (pane.term.cols(), pane.term.rows());
            pane.term.anchor_last_row();
            pane.copy = Some(Copy::enter(
                &pane.term,
                pane.term.cursor_position(),
                cols,
                rows,
            ));
        }
        if let Some(copy) = pane.copy.as_mut() {
            // Once the selection began, its anchor stays on its line of the history while the view
            // moves; the cell of the press is only the anchor the first time.
            if drag.selecting && copy.selection.is_some() {
                copy.extend_to(&pane.term, to);
            } else {
                copy.select_between(&pane.term, drag.from, to);
            }
        }
        if opened {
            self.copy_mode_changed(session_id);
        }
        if let Some(s) = self.sessions.get_mut(&session_id)
            && let Some(d) = s.drag.as_mut()
        {
            d.selecting = true;
        }
    }

    fn scroll_by_mouse(&mut self, session_id: u64, pane_id: u64, rows: isize) {
        let Some(pane) = self.panes.get_mut(&pane_id) else {
            return;
        };
        let mut opened = false;
        if pane.copy.is_none() {
            if rows > 0 {
                return;
            }
            let (cols, height) = (pane.term.cols(), pane.term.rows());
            pane.term.anchor_last_row();
            let mut copy = Copy::enter(&pane.term, pane.term.cursor_position(), cols, height);
            copy.by_wheel = true;
            pane.copy = Some(copy);
            opened = true;
        }
        let Some(copy) = pane.copy.as_mut() else {
            return;
        };
        copy.scroll(&pane.term, rows);
        let done = copy.by_wheel && copy.selection.is_none() && copy.at_bottom(&pane.term);
        if done {
            self.exit_copy(session_id, pane_id);
        } else if opened {
            self.copy_mode_changed(session_id);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const NO: Mods = Mods::NONE;

    fn sgr(level: MouseLevel, kind: MouseKind, mods: Mods, x: u16, y: u16) -> Option<String> {
        encode(level, true, kind, mods, x, y).map(|b| String::from_utf8(b).unwrap())
    }

    #[test]
    fn a_report_names_the_place_in_the_pane_from_one() {
        let got = sgr(MouseLevel::Press, MouseKind::Press(0), NO, 9, 6);
        assert_eq!(got.as_deref(), Some("\x1b[<0;10;7M"));
        let got = sgr(MouseLevel::Press, MouseKind::Release(0), NO, 9, 6);
        assert_eq!(got.as_deref(), Some("\x1b[<0;10;7m"));
        let got = sgr(MouseLevel::Press, MouseKind::Wheel(Wheel::Down), NO, 0, 0);
        assert_eq!(got.as_deref(), Some("\x1b[<65;1;1M"));
    }

    #[test]
    fn the_level_the_program_asked_for_filters_what_it_gets() {
        let drag = MouseKind::Drag(0);
        assert_eq!(sgr(MouseLevel::Off, MouseKind::Press(0), NO, 0, 0), None);
        assert_eq!(sgr(MouseLevel::Press, drag, NO, 0, 0), None);
        assert!(sgr(MouseLevel::Drag, drag, NO, 0, 0).is_some());
        assert_eq!(sgr(MouseLevel::Drag, MouseKind::Move, NO, 0, 0), None);
        assert!(sgr(MouseLevel::Any, MouseKind::Move, NO, 0, 0).is_some());
        assert_eq!(
            sgr(MouseLevel::Drag, drag, NO, 1, 2).as_deref(),
            Some("\x1b[<32;2;3M")
        );
    }

    #[test]
    fn modifiers_stay_in_the_button_code() {
        let mods = Mods {
            shift: true,
            alt: false,
            ctrl: true,
            super_key: false,
        };
        let got = sgr(MouseLevel::Press, MouseKind::Press(2), mods, 0, 0);
        assert_eq!(got.as_deref(), Some("\x1b[<22;1;1M"));
    }

    #[test]
    fn the_old_form_adds_32_and_does_not_say_which_button_came_up() {
        let press = encode(MouseLevel::Press, false, MouseKind::Press(0), NO, 9, 6).unwrap();
        assert_eq!(press, vec![0x1b, b'[', b'M', 32, 10 + 32, 7 + 32]);
        let up = encode(MouseLevel::Press, false, MouseKind::Release(2), NO, 0, 0).unwrap();
        assert_eq!(up, vec![0x1b, b'[', b'M', 3 + 32, 33, 33]);
        // Column 222 is the last the old form can name.
        assert!(encode(MouseLevel::Press, false, MouseKind::Press(0), NO, 222, 0).is_some());
        assert!(encode(MouseLevel::Press, false, MouseKind::Press(0), NO, 223, 0).is_none());
    }

    #[test]
    fn a_drag_is_kept_inside_the_pane_it_began_in() {
        let rect = Rect::new(10, 2, 20, 5);
        assert_eq!(clamp_into(rect, 15, 4), (5, 2));
        assert_eq!(clamp_into(rect, 99, 99), (19, 4));
        assert_eq!(clamp_into(rect, 0, 0), (0, 0));
    }
}
