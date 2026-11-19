//! Building the screen update for a client: when it may go out, and what goes in it.

use super::*;
use crate::compose::text_cells;
use crate::format::Field;
use crate::render::HostModes;
use crate::status::{self, StatusInfo, Tab};
use crate::theme;

/// What a tab says besides its number, from the format of the configuration: by default the
/// window's name. The program's title is a quiet piece, cut to `TAB_TITLE_CELLS`.
pub(super) fn tab_text(
    config: &Config,
    panes: &HashMap<u64, Pane>,
    w: &Window,
) -> Vec<status::TabPiece> {
    let title: String = panes
        .get(&w.focus)
        .and_then(|p| p.title.as_deref())
        .map(|t| clean_title(t, TITLE_LIMIT).trim().to_string())
        .unwrap_or_default();
    let title = status::cut_to_cells(&title, status::TAB_TITLE_CELLS);
    let pieces = config.tab_format.render(|field| match field {
        Field::Name => w.name.clone(),
        // The same word twice says nothing more.
        Field::Title if title == w.name => String::new(),
        Field::Title => title.clone(),
        Field::Program => w.program.clone(),
        Field::Directory => w.directory.clone(),
        Field::Session | Field::Number => String::new(),
    });
    let mut out: Vec<status::TabPiece> = pieces
        .into_iter()
        .map(|p| status::TabPiece {
            text: clean_title(&p.text, TITLE_LIMIT),
            quiet: p.field == Some(Field::Title),
        })
        .collect();
    if out.is_empty() {
        // A format that says nothing for this window must not leave an empty tab.
        out.push(status::TabPiece::plain(w.name.clone()));
    }
    out
}

/// What the shells of a window's panes say about their commands, the most urgent: an error, a
/// command that runs, then one that ended well.
pub(super) fn window_command(
    config: &Config,
    panes: &HashMap<u64, Pane>,
    w: &Window,
) -> Option<status::CommandMark> {
    let limits = crate::shellmarks::Thresholds {
        running_after: config.command_running_after,
        finished_after: config.command_finished_after,
    };
    let now = Instant::now();
    w.layout
        .rects()
        .into_iter()
        .filter_map(|(p, _)| panes.get(&p))
        .filter_map(|p| pane_command(&p.command, now, limits))
        .min_by_key(|c| match c {
            status::CommandMark::FinishedWithError => 0,
            status::CommandMark::Running => 1,
            status::CommandMark::FinishedWell => 2,
        })
}

/// What one pane says about its command.
pub(super) fn pane_command(
    track: &crate::shellmarks::Track,
    now: Instant,
    limits: crate::shellmarks::Thresholds,
) -> Option<status::CommandMark> {
    match track.finished {
        Some(false) => Some(status::CommandMark::FinishedWithError),
        _ if track.is_running(now, limits) => Some(status::CommandMark::Running),
        Some(true) => Some(status::CommandMark::FinishedWell),
        None => None,
    }
}

fn status_info(
    s: &Session,
    config: &Config,
    panes: &HashMap<u64, Pane>,
    copy: Option<crate::status::CopyState>,
    unlock_key: Option<String>,
    notices: &crate::notify::Store,
    hint: Option<String>,
) -> StatusInfo {
    StatusInfo {
        session: s.name.clone(),
        number: config.tab_number,
        style: config.tab_style,
        tabs: s
            .windows
            .iter()
            .map(|w| Tab {
                number: w.number,
                text: tab_text(config, panes, w),
                current: w.id == s.current,
                zoomed: w.zoomed,
                notice: config
                    .notification
                    .tab_mark
                    .then(|| notices.highest(w.layout.rects().into_iter().map(|(p, _)| p)))
                    .flatten(),
                progress: panes.get(&w.focus).and_then(|p| p.progress).or_else(|| {
                    w.layout
                        .rects()
                        .into_iter()
                        .find_map(|(p, _)| panes.get(&p).and_then(|p| p.progress))
                }),
                command: window_command(config, panes, w),
            })
            .collect(),
        prefix: s.key_state.prefix_active(),
        locked: s.key_state.locked,
        recording: s
            .current_window()
            .layout
            .rects()
            .into_iter()
            .any(|(p, _)| panes.get(&p).is_some_and(|p| p.capture.is_some())),
        unlock_key,
        copy,
        notices: match config.notification.status_count {
            crate::config::StatusCount::Off => [0; 4],
            crate::config::StatusCount::All => notices.counts(s.all_panes()),
            crate::config::StatusCount::OtherSessions => notices.counts_outside(&s.all_panes()),
        },
        notice_sessions: Vec::new(),
        message: s.key_state.message.clone(),
        message_kind: s.key_state.message_kind,
        prompt: s.key_state.status_prompt(),
        hint,
    }
}

/// The panes that wear the mark of their notification in the top right corner: out of focus, in a
/// window of several panes, wide enough, with a notification, and the setting on.
fn panes_wearing_a_mark(
    config: &crate::config::Config,
    notices: &crate::notify::Store,
    in_focus: PaneId,
    visible: &[(PaneId, Rect)],
) -> Vec<PaneId> {
    if !config.notification.pane_mark || visible.len() < 2 || notices.is_empty() {
        return Vec::new();
    }
    visible
        .iter()
        .filter(|(p, r)| *p != in_focus && r.w >= MARK_MIN_WIDTH && notices.get(*p).is_some())
        .map(|(p, _)| *p)
        .collect()
}

/// A pane narrower than this has no room for the mark of its notification.
const MARK_MIN_WIDTH: u16 = 12;

/// Everything the title of the host terminal's window is made from, kept to tell whether it has to
/// be made again.
#[derive(Default)]
pub(super) struct TitleInputs {
    window: u64,
    focus: u64,
    number: usize,
    pane_title: Option<String>,
    session: String,
    name: String,
    program: String,
    directory: String,
}

impl TitleInputs {
    fn matches(&self, s: &Session, pane_title: Option<&str>) -> bool {
        let w = s.current_window();
        self.window == w.id
            && self.focus == w.focus
            && self.number == w.number
            && self.pane_title.as_deref() == pane_title
            && self.session == s.name
            && self.name == w.name
            && self.program == w.program
            && self.directory == w.directory
    }

    fn of(s: &Session, pane_title: Option<&str>) -> TitleInputs {
        let w = s.current_window();
        TitleInputs {
            window: w.id,
            focus: w.focus,
            number: w.number,
            pane_title: pane_title.map(str::to_string),
            session: s.name.clone(),
            name: w.name.clone(),
            program: w.program.clone(),
            directory: w.directory.clone(),
        }
    }
}

/// The longest pane title that goes to the host terminal, in characters.
pub(super) const TITLE_LIMIT: usize = 200;

/// Text from a program or a name, made safe to put inside an escape sequence for the host
/// terminal: no control characters of any kind, and not longer than `limit` characters.
fn clean_title(text: &str, limit: usize) -> String {
    text.chars()
        .filter(|c| !c.is_control())
        .take(limit)
        .collect()
}

/// The sequence that takes a progress bar away from the host terminal.
pub(super) const PROGRESS_REMOVE: &[u8] = b"\x1b]9;4;0\x1b\\";

impl Server {
    /// The title of the host terminal's window, made from the format of the configuration: by
    /// default the session, the window's number and name, then what the focused pane's program
    /// calls itself. Empty when the format says Arcade sets none.
    fn host_title(&self, s: &Session) -> String {
        let window = s.current_window();
        let pane = self.panes.get(&window.focus);
        let title = pane
            .and_then(|p| p.title.as_deref())
            .map(|t| clean_title(t, TITLE_LIMIT).trim().to_string())
            .unwrap_or_default();
        let session = clean_title(&s.name, TITLE_LIMIT);
        let name = clean_title(&window.name, TITLE_LIMIT);
        let text = self.config.title_format.render_text(|field| match field {
            Field::Session => session.clone(),
            Field::Number => window.number.to_string(),
            Field::Name => name.clone(),
            // The same word twice says nothing more.
            Field::Title if title == name || title == session => String::new(),
            Field::Title => title.clone(),
            Field::Program => window.program.clone(),
            Field::Directory => window.directory.clone(),
        });
        clean_title(&text, TITLE_LIMIT * 2)
    }

    /// Tell each attached client's terminal the title of its window, when and only when it is
    /// not the one it was told last.
    pub(super) fn send_titles(&mut self) {
        let mut due = Vec::new();
        let mut titles_seen = Vec::new();
        for s in self.sessions.values() {
            let Some(client) = s.client else {
                continue;
            };
            if !self.clients.get(&client).is_some_and(|c| !c.gone) {
                continue;
            }
            let pane_title = self
                .panes
                .get(&s.current_window().focus)
                .and_then(|p| p.title.as_deref());
            if s.title_inputs
                .as_ref()
                .is_some_and(|k| k.matches(s, pane_title))
            {
                continue;
            }
            let key = TitleInputs::of(s, pane_title);
            let title = self.host_title(s);
            titles_seen.push((s.id, key));
            if !title.is_empty() && s.sent_title.as_deref() != Some(title.as_str()) {
                due.push((s.id, client, title));
            }
        }
        for (session_id, key) in titles_seen {
            if let Some(s) = self.sessions.get_mut(&session_id) {
                s.title_inputs = Some(key);
            }
        }
        for (session_id, client, title) in due {
            self.send_output(client, format!("\x1b]2;{title}\x07").as_bytes());
            if let Some(s) = self.sessions.get_mut(&session_id) {
                s.sent_title = Some(title);
            }
        }
    }

    /// Tell each attached client's terminal the progress of the focused pane of the window it
    /// shows, when that is not what it was told last: a new report from that pane (also one that
    /// repeats the last), another pane in focus, or none to show any more.
    pub(super) fn send_progress(&mut self) {
        let mut due = Vec::new();
        for s in self.sessions.values() {
            let Some(client) = s.client else {
                continue;
            };
            if !self.clients.get(&client).is_some_and(|c| !c.gone) {
                continue;
            }
            let focus = s.current_window().focus;
            let pane = self.panes.get(&focus);
            let shown = pane.and_then(|p| p.progress.map(|r| (r, (focus, p.progress_seq))));
            if shown.map(|(_, key)| key) != s.sent_progress {
                due.push((s.id, client, shown));
            }
        }
        for (session_id, client, shown) in due {
            match shown {
                Some((report, _)) => self.send_output(
                    client,
                    format!("\x1b]9;4;{};{}\x1b\\", report.state, report.value).as_bytes(),
                ),
                None => self.send_output(client, PROGRESS_REMOVE),
            }
            if let Some(s) = self.sessions.get_mut(&session_id) {
                s.sent_progress = shown.map(|(_, key)| key);
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
        if !(dirty || s.need_full || s.relayout || s.chrome_dirty || s.blink_due) {
            return None;
        }
        // The 60 per second limit is for floods. A light update (an echoed key) goes out at
        // once, so typing is not delayed by the pacing of the update before it.
        // While a divider is dragged every update is paced like a flood, so the sizes the
        // programs are told follow the frame rate and not the rate of the mouse.
        if !heavy && s.divider_drag.is_none() {
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
            // A clipboard write must not wait for the end of a flood: it is handled when the
            // next update of its session goes out.
            self.handle_pending_clipboard_of(id);
        }
    }

    /// Build the screen update for a session's client and queue it.
    pub(super) fn build_frame_for(&mut self, session_id: u64) {
        stage_mark!(DaemonFrameStart);
        let status_position = self.config.status_position;
        let unlock_key = self.unlock_key_name();
        let blink_dim = self.blink_dim_at(Instant::now());
        let number_phase = self.number_phase_at(Instant::now());
        let notice_sessions = self.notice_sessions(session_id);
        // Panes get the sizes the dragged divider gave them, once per frame.
        let stale = self
            .sessions
            .get_mut(&session_id)
            .and_then(|s| s.sizes_stale.take());
        if let Some(window) = stale {
            self.apply_window_sizes(session_id, window);
        }
        self.refresh_tree(session_id);
        self.settle_notices(session_id);
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
        if !(panes_dirty
            || session.need_full
            || session.relayout
            || session.chrome_dirty
            || session.blink_due)
        {
            return;
        }
        // Only a blink is due when nothing else changed and the bar was drawn before: the bar is
        // then taken from its cache, and nothing else is built.
        let blink_only =
            !(panes_dirty || session.need_full || session.relayout || session.chrome_dirty)
                && session.bar_cache.is_some();
        // The names of the windows come from the panes' directories. They are looked at only
        // now, when an update is going out anyway, and at most once a second. A look that is
        // skipped for being too soon leaves one deadline for the loop, at the moment a second
        // is full, so a directory changed in the meantime is seen then; no deadline is left
        // once the names have been looked at.
        if !blink_only {
            let now = Instant::now();
            match session.names_checked {
                Some(t) if now.duration_since(t) < NAME_CHECK_INTERVAL => {
                    session.names_due = Some(t + NAME_CHECK_INTERVAL);
                }
                _ => {
                    session.names_checked = Some(now);
                    session.names_due = None;
                    // A title lasts as long as the program that set it, and so does a progress
                    // report.
                    for window in &session.windows {
                        for (pane_id, _) in window.layout.rects() {
                            if let Some(pane) = self.panes.get_mut(&pane_id)
                                && (pane.title_owner.is_some() || !pane.title_stack.is_empty())
                            {
                                pane.expire_titles();
                            }
                            if let Some(pane) = self.panes.get_mut(&pane_id)
                                && pane.expire_progress_owner()
                            {
                                session.chrome_dirty = true;
                            }
                        }
                    }
                    // The program and the directory of each window, for the formats that use them.
                    let wants_program = self.config.tab_format.uses(Field::Program)
                        || self.config.title_format.uses(Field::Program);
                    let wants_directory = self.config.tab_format.uses(Field::Directory)
                        || self.config.title_format.uses(Field::Directory);
                    if wants_program || wants_directory {
                        for window in session.windows.iter_mut() {
                            let Some(p) = self.panes.get(&window.focus) else {
                                continue;
                            };
                            let program = if wants_program {
                                pty::foreground_program(p.master.as_raw_fd(), p.pid)
                                    .unwrap_or_default()
                            } else {
                                String::new()
                            };
                            let directory = if wants_directory {
                                pty::process_cwd(p.pid)
                                    .map(|c| dir_label(&c))
                                    .unwrap_or_default()
                            } else {
                                String::new()
                            };
                            if program != window.program || directory != window.directory {
                                window.program = program;
                                window.directory = directory;
                                session.chrome_dirty = true;
                            }
                        }
                    }
                    // The automatic name goes on following its source only when the configuration
                    // says so; otherwise it was given when the window was made.
                    if self.config.window_name_follow {
                        for window in session.windows.iter_mut().filter(|w| !w.name_manual) {
                            let name = name_of_pane(
                                &self.panes,
                                self.config.window_name_from,
                                window.focus,
                                &session.cwd,
                            );
                            if name != window.name {
                                window.name = name;
                                session.chrome_dirty = true;
                            }
                        }
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
        session.screen.set_link_underline(
            self.config
                .links
                .underline
                .then(|| theme::style(theme::Role::LinkUnderline)),
        );
        let mut failed = Vec::new();
        if !blink_only {
            let wearing = panes_wearing_a_mark(
                &self.config,
                &self.notices,
                session.current_window().focus,
                &visible,
            );
            for (pane_id, rect) in &visible {
                let Some(pane) = self.panes.get_mut(pane_id) else {
                    continue;
                };
                // A pane in copy mode shows its view of the rows, not what the program draws; what
                // the program draws is read again when copy mode ends.
                if let Some(copy) = pane.copy.as_mut() {
                    pane.dirty = false;
                    // The mark of the place in the history is drawn again only when a number moved.
                    copy.show_position = self.config.scroll_position;
                    // The corner belongs to the mark of a notification; the position mark gives way.
                    let gap = u16::from(wearing.contains(pane_id));
                    if copy.right_gap != gap {
                        copy.right_gap = gap;
                        copy.dirty = true;
                    }
                    let position = copy.position(&pane.term);
                    if copy.show_position && copy.drawn_position != Some(position) {
                        copy.dirty = true;
                    }
                    copy.drawn_position = Some(position);
                    if relayout || copy.dirty {
                        copy.dirty = false;
                        let (view, links) = copy.view_with_links(&pane.term);
                        let rows = view
                            .into_iter()
                            .zip(links)
                            .enumerate()
                            .map(|(y, (cells, links))| arcade_term::RowUpdate {
                                y: y as u16,
                                cells,
                                links,
                            })
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
            stage_mark!(DaemonSnapshot);
            // A pane out of focus, in a window of several panes, wears the mark of its notification in
            // one cell at the top right (with one pane the tab says it, and the pane in focus is
            // being looked at). It covers that cell of what the program drew.
            {
                for (pane_id, rect) in &visible {
                    if wearing.contains(pane_id)
                        && let Some(n) = self.notices.get(*pane_id)
                    {
                        let mark =
                            text_cells(theme::notice_symbol(n.class), theme::notice_style(n.class));
                        let x = rect.x + rect.w - mark.len() as u16;
                        session.screen.overlay(x, rect.y, &[mark]);
                    }
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
            let rects: Vec<crate::layout::Rect> = visible.iter().map(|(_, r)| *r).collect();
            session
                .screen
                .draw_dividers(&dividers, origin, focus_rect, &rects);
            if let Some(y) = status_row(rows, status_position) {
                let copy_on = self
                    .panes
                    .get(&focus)
                    .and_then(|p| p.copy.as_ref())
                    .map(crate::copy::Copy::state);
                // The key hint is off unless `[keys] hint` is set; the line that says which keys give
                // the direction of a swap is not a hint but the question the swap is waiting on.
                let swapping =
                    matches!(session.key_state.prefix, keys::PrefixState::Swapping { .. });
                let notes_first = !notice_sessions.is_empty();
                let hint = ((self.config.keys.hint || swapping)
                    && session.picker.is_none()
                    && session.tree.is_none()
                    && session.key_state.hint_visible())
                .then(|| {
                    let width = usize::from(cols);
                    if swapping {
                        crate::keymap::swap_hint_line(&self.keymap.prefix, width)
                    } else {
                        crate::keymap::hint_line(&self.keymap.prefix, width, notes_first)
                    }
                });
                let mut info = status_info(
                    session,
                    &self.config,
                    &self.panes,
                    copy_on,
                    unlock_key,
                    &self.notices,
                    hint,
                );
                info.notice_sessions = notice_sessions;
                let dim = blink_dim;
                let number = number_phase;
                let ((cells, prompt_cursor, spans), seen) =
                    crate::theme::drawing_the_bar_at(dim, number, || {
                        status::build_with_tabs(cols, &info)
                    });
                session.bar_blinks = seen.blink;
                session.bar_blink_dim = dim;
                session.bar_alternates = seen.alternate;
                session.bar_number_phase = number;
                // Where the tabs are as drawn, for a click on one.
                session.counter_bar = spans.iter().find(|t| t.counter).map(|t| (t.from, t.to));
                session.tab_bar = spans
                    .iter()
                    .filter(|t| !t.counter)
                    .filter_map(|t| Some((session.windows.get(t.index)?.id, t.from, t.to)))
                    .collect();
                session.bar_cache = if seen.blink || seen.alternate {
                    // Every phase is built now, so a blink or a turn draws the bar from here.
                    let mut rows = [Vec::new(), Vec::new(), Vec::new(), Vec::new()];
                    let mut cursors = [None; 4];
                    for (d, n) in [(false, false), (false, true), (true, false), (true, true)] {
                        let i = super::bar_index(d, n);
                        if (d, n) == (dim, number) {
                            rows[i] = cells.clone();
                            cursors[i] = prompt_cursor;
                        } else {
                            let ((c, p, _), _) = crate::theme::drawing_the_bar_at(d, n, || {
                                status::build_with_tabs(cols, &info)
                            });
                            rows[i] = c;
                            cursors[i] = p;
                        }
                    }
                    Some(BarCache {
                        rows,
                        prompt_cursor: cursors,
                    })
                } else {
                    None
                };
                session.screen.set_row(y, cells);
                cursor_override = prompt_cursor.map(|x| (x, y));
            }
        } else if blink_only {
            if let (Some(y), Some(cache)) = (
                status_row(rows, status_position),
                session.bar_cache.as_ref(),
            ) {
                let i = super::bar_index(blink_dim, number_phase);
                session.screen.set_row(y, cache.rows[i].clone());
                cursor_override = cache.prompt_cursor[i].map(|x| (x, y));
            }
            session.bar_blink_dim = blink_dim;
            session.bar_number_phase = number_phase;
        }
        // The floating list is drawn over everything on every update.
        let picker_open = session.picker.is_some() || session.tree.is_some();
        let list_keys = crate::keymap::ListKeys::new(&self.keymap.list);
        if let Some(picker) = &session.picker {
            let drawn = picker.draw(cols, rows, &list_keys);
            session.screen.overlay(drawn.x, drawn.y, &drawn.rows);
        }
        if let Some(tree) = &session.tree {
            let drawn = tree.draw(cols, rows, &list_keys);
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
        let modes = self.panes.get(&focus).map_or_else(
            || HostModes {
                mouse: true,
                ..HostModes::default()
            },
            |p| HostModes {
                mouse: true,
                app_cursor_keys: p
                    .term
                    .mode(arcade_term::Mode::AppCursorKeys)
                    .unwrap_or(false),
                bracketed_paste: p
                    .term
                    .mode(arcade_term::Mode::BracketedPaste)
                    .unwrap_or(false),
            },
        );
        let Some(session) = self.sessions.get_mut(&session_id) else {
            return;
        };
        session.need_full = false;
        session.relayout = false;
        session.chrome_dirty = false;
        session.blink_due = false;
        stage_mark!(DaemonComposed);
        session
            .renderer
            .set_bar_row(status_row(session.rows, status_position));
        let bytes = session.renderer.render(&mut session.screen, &cursor, modes);
        stage_mark!(DaemonEncoded);
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
