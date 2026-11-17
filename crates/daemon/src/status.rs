//! What the status bar says: the session's name, one tab per window, and the words that tell the
//! user which mode the keyboard is in. Each part is built on its own and put together here.

use arcade_term::Cell;

use crate::compose::{blank, fit, text_cells};
use crate::config::{NumberPosition, TabStyle};
use crate::notify::Class;
use crate::theme::{self, Role, Symbol};

/// The columns a tab covers on the bar as it is drawn (after any cutting): `from` to `to`, the
/// end not included, and the index of the tab in `StatusInfo::tabs`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct TabSpan {
    /// The tab, or for a marker the nearest hidden tab on its side, which a click selects.
    pub index: usize,
    pub from: u16,
    pub to: u16,
    /// This is the marker at an end of the bar that tells tabs are hidden, not a tab.
    pub marker: bool,
}

/// A piece of the text of a tab; a quiet piece (the program's own title) is drawn fainter than
/// the rest.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct TabPiece {
    pub text: String,
    pub quiet: bool,
}

impl TabPiece {
    pub fn plain(text: impl Into<String>) -> TabPiece {
        TabPiece {
            text: text.into(),
            quiet: false,
        }
    }
}

/// The longest a program's title is on a tab, in cells; a longer one is cut with a mark.
pub const TAB_TITLE_CELLS: usize = 24;

/// A text cut to `max` cells, with the cut mark at the end when something was left out.
pub fn cut_to_cells(text: &str, max: usize) -> String {
    if crate::width::text_width(text) <= max {
        return text.to_string();
    }
    let room = max.saturating_sub(1);
    let mut out = String::new();
    let mut used = 0;
    for unit in crate::width::units(text) {
        if used + unit.width > room {
            break;
        }
        used += unit.width;
        out.push_str(unit.text);
    }
    out.push_str(theme::symbol(Symbol::Ellipsis));
    out
}

pub struct Tab {
    /// The number shown, which is the window's place in the bar plus the base index.
    pub number: usize,
    /// What the tab says besides its number.
    pub text: Vec<TabPiece>,
    pub current: bool,
    pub zoomed: bool,
    /// The most urgent notification among the window's panes.
    pub notice: Option<Class>,
    /// The progress report of the window's focused pane, or else of another pane of it.
    pub progress: Option<arcade_term::Progress>,
}

/// A line the user types into.
pub struct Prompt {
    pub label: String,
    pub text: String,
}

/// What copy mode is doing, for the word on the bar.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum CopyState {
    /// Moving about.
    Normal,
    /// A selection by character is active.
    Visual,
    /// A selection by line is active.
    VisualLine,
    /// A jump is open, with what was typed so far.
    Jump(String),
}

pub struct StatusInfo {
    pub session: String,
    /// Where the number stands in a tab.
    pub number: NumberPosition,
    /// Round ends or plain blocks.
    pub style: TabStyle,
    pub tabs: Vec<Tab>,
    /// Waiting for the key after the prefix key.
    pub prefix: bool,
    pub locked: bool,
    /// The key capture is on for a pane of the window.
    pub recording: bool,
    /// The key that ends the lock, as it is typed, when there is one.
    pub unlock_key: Option<String>,
    /// The focused pane is in copy mode, and what it is doing.
    pub copy: Option<CopyState>,
    /// How many panes of the session have a notification, per class in the order of
    /// `Class::BY_URGENCY`.
    pub notices: [usize; 4],
    /// A line shown in place of the bar until the next key.
    pub message: Option<String>,
    pub prompt: Option<Prompt>,
    /// The key hint, shown in place of the bar while the prefix is waiting for a key.
    pub hint: Option<String>,
}

/// The cells of the bar, and the column the host's cursor should go to while a prompt is open.
#[cfg(test)]
pub fn build(cols: u16, info: &StatusInfo) -> (Vec<Cell>, Option<u16>) {
    let (cells, cursor, _) = build_with_tabs(cols, info);
    (cells, cursor)
}

/// A block of the bar with its ends: round ends in the pill style, nothing more in the flat one
/// (a block has a cell of padding at each end of its own).
fn capped(style: TabStyle, blocks: Vec<Cell>, left: Cell, right: Cell) -> Vec<Cell> {
    match style {
        TabStyle::Pill => [vec![left], blocks, vec![right]].concat(),
        TabStyle::Flat => blocks,
    }
}

fn cap(symbol: Symbol, block: arcade_term::Style) -> Cell {
    text_cells(theme::symbol(symbol), theme::cap_style(block))
        .into_iter()
        .next()
        .unwrap_or_else(|| blank(block))
}

/// The session's name at the left: in the pill style a green badge with an icon and the name on a
/// surface, each end round; in the flat style the name on a green block.
fn session_label(info: &StatusInfo) -> Vec<Cell> {
    let badge = theme::style(Role::SessionBadge);
    let name = theme::style(Role::SessionName);
    match info.style {
        TabStyle::Pill => {
            let mut blocks = text_cells(theme::symbol(Symbol::SessionIcon), badge);
            blocks.extend(text_cells(&format!(" {} ", info.session), name));
            capped(
                info.style,
                blocks,
                cap(Symbol::PillCapLeft, badge),
                cap(Symbol::PillCapRight, name),
            )
        }
        TabStyle::Flat => text_cells(&format!(" {} ", info.session), badge),
    }
}

/// One tab: the name on a surface and the number as a badge at the end `info.number` says, the
/// current one in brighter colours with the accent badge; in the pill style with round ends, in
/// the flat one as plain blocks. The marks (zoom, notification, progress) are in the name's block
/// and always have the same width, so the tabs to the right do not move when one appears.
fn tab_cells(tab: &Tab, info: &StatusInfo) -> Vec<Cell> {
    tab_cells_with(tab, &tab.text, info)
}

/// The same with the text of the tab given: the pieces cut short to make room.
fn tab_cells_with(tab: &Tab, pieces: &[TabPiece], info: &StatusInfo) -> Vec<Cell> {
    let (name_role, quiet_role, badge_role) = if tab.current {
        (
            Role::PillNameCurrent,
            Role::PillQuietCurrent,
            Role::PillBadgeCurrent,
        )
    } else {
        (Role::PillName, Role::PillQuiet, Role::PillBadge)
    };
    let name = theme::style(name_role);
    let quiet = theme::style(quiet_role);
    let badge = theme::style(badge_role);
    let mut text = text_cells(" ", name);
    for piece in pieces {
        text.extend(text_cells(
            &piece.text,
            if piece.quiet { quiet } else { name },
        ));
    }
    if tab.zoomed {
        text.extend(text_cells(
            &format!(" {}", theme::symbol(Symbol::Zoomed)),
            name,
        ));
    }
    // The one status cell: always there, so a tab's width does not depend on what it says.
    text.extend(text_cells(" ", name));
    match status_mark(tab) {
        Some(mark) => text.extend(mark_cells(mark, name)),
        None => text.extend(text_cells(" ", name)),
    }
    text.extend(text_cells(" ", name));
    let number = text_cells(&format!(" {} ", tab.number), badge);
    let (first, last, blocks) = match info.number {
        NumberPosition::Left => (badge, name, [number, text].concat()),
        NumberPosition::Right => (name, badge, [text, number].concat()),
        NumberPosition::Off => (name, name, text),
    };
    capped(
        info.style,
        blocks,
        cap(Symbol::PillCapLeft, first),
        cap(Symbol::PillCapRight, last),
    )
}

/// What the one status cell of a tab shows.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Mark {
    /// A notification of the window's panes.
    Notice(Class),
    /// A progress report that failed.
    ProgressError,
    /// A progress report with a value (`paused` when it is paused).
    Level { value: u8, paused: bool },
    /// A progress report with an unknown amount.
    Busy,
}

impl Mark {
    /// How urgent the mark is; the smaller, the more.
    fn rank(self) -> usize {
        let class = |c: Class| Class::BY_URGENCY.iter().position(|b| *b == c).unwrap_or(0);
        match self {
            Mark::Notice(c) if c != Class::Info => class(c),
            Mark::ProgressError => 10,
            Mark::Level { .. } => 20,
            Mark::Busy => 30,
            Mark::Notice(c) => 40 + class(c),
        }
    }
}

/// The mark a tab shows, the most urgent of what it knows: a notification that asks for you or
/// reports an error, a failed progress report, a progress report with a value, one with an
/// unknown amount, then a notification that something finished.
pub fn status_mark(tab: &Tab) -> Option<Mark> {
    let progress = tab.progress.and_then(|r| match r.state {
        1 | 4 => Some(Mark::Level {
            value: r.value,
            paused: r.state == 4,
        }),
        2 => Some(Mark::ProgressError),
        3 => Some(Mark::Busy),
        _ => None,
    });
    tab.notice
        .map(Mark::Notice)
        .into_iter()
        .chain(progress)
        .min_by_key(|m| m.rank())
}

/// The cell of a mark on the block it stands on, in the mark's colour.
fn mark_cells(mark: Mark, block: arcade_term::Style) -> Vec<Cell> {
    let (symbol, tint) = match mark {
        Mark::Notice(class) => (theme::notice_symbol(class), theme::Tint::Class(class)),
        Mark::ProgressError => (theme::PROGRESS_ERROR_SYMBOL, theme::Tint::ProgressError),
        Mark::Level { value, paused } => (
            theme::level_block(value),
            if paused {
                theme::Tint::Paused
            } else {
                theme::Tint::Progress
            },
        ),
        Mark::Busy => (theme::BUSY_SYMBOL, theme::Tint::Progress),
    };
    text_cells(symbol, theme::cell_style(tint, block))
}

/// A laid-out bar: its cells and, for each tab or marker, its index (for a marker the nearest
/// hidden tab), where it starts and ends, and whether it is a marker.
type Laid = (Vec<Cell>, Vec<(usize, usize, usize, bool)>);

/// How small a tab's text is made, in cells, before a tab is hidden: three cells, the ellipsis and
/// three cells. A name of this many cells or fewer is never cut.
const TAB_TEXT_MIN: usize = 7;

fn pieces_width(pieces: &[TabPiece]) -> usize {
    pieces
        .iter()
        .map(|p| crate::width::text_width(&p.text))
        .sum()
}

/// The pieces of a tab's text cut to `limit` cells, with the cut marked by `…` in the middle:
/// about half of the cells that are kept come from the start (the start gets the extra one) and
/// half from the end, because names in one project often share a beginning (`everfit-api`,
/// `everfit-microservices`) and differ at the end. Text that is short enough is left as it is.
fn shorten_pieces(pieces: &[TabPiece], limit: usize) -> Vec<TabPiece> {
    if pieces_width(pieces) <= limit {
        return pieces.to_vec();
    }
    // A unit is a character or a sequence that is drawn as one; the cut never lands inside one.
    let chars: Vec<(&str, usize, usize)> = pieces
        .iter()
        .enumerate()
        .flat_map(|(p, piece)| {
            crate::width::units(&piece.text)
                .into_iter()
                .map(move |u| (u.text, p, u.width))
        })
        .collect();
    let keep = limit.saturating_sub(1);
    let (head_room, tail_room) = (keep.div_ceil(2), keep / 2);
    let mut head = 0;
    let mut used = 0;
    while head < chars.len() && used + chars[head].2 <= head_room {
        used += chars[head].2;
        head += 1;
    }
    let mut tail = chars.len();
    used = 0;
    while tail > head && used + chars[tail - 1].2 <= tail_room {
        used += chars[tail - 1].2;
        tail -= 1;
    }
    let mark_piece = if head > 0 { chars[head - 1].1 } else { 0 };
    let mut kept: Vec<(String, usize)> = Vec::new();
    let mut push = |text: &str, piece: usize| match kept.last_mut() {
        Some((t, p)) if *p == piece => t.push_str(text),
        _ => kept.push((text.to_string(), piece)),
    };
    for (unit, p, _) in &chars[..head] {
        push(unit, *p);
    }
    push(theme::symbol(Symbol::Ellipsis), mark_piece);
    for (unit, p, _) in &chars[tail..] {
        push(unit, *p);
    }
    kept.into_iter()
        .map(|(text, piece)| TabPiece {
            text,
            quiet: pieces[piece].quiet,
        })
        .collect()
}

/// What the bar knows about the tabs it lays out: for each, the cells around its text and how
/// wide its text is.
struct Fitting<'a> {
    info: &'a StatusInfo,
    /// The cells of tab `i` besides its text, gap after it included.
    chrome: Vec<usize>,
    /// The width of its whole text.
    natural: Vec<usize>,
}

impl<'a> Fitting<'a> {
    fn new(info: &'a StatusInfo) -> Fitting<'a> {
        let natural: Vec<usize> = info.tabs.iter().map(|t| pieces_width(&t.text)).collect();
        let chrome = info
            .tabs
            .iter()
            .zip(&natural)
            .map(|(t, n)| tab_cells(t, info).len() - n + 1)
            .collect();
        Fitting {
            info,
            chrome,
            natural,
        }
    }

    /// The width tab `i` takes (gap included) when its text is cut to `limit`.
    fn width(&self, i: usize, limit: usize) -> usize {
        self.chrome[i] + pieces_width(&shorten_pieces(&self.info.tabs[i].text, limit))
    }

    /// The smallest a tab gets before it is hidden.
    fn compact(&self, i: usize) -> usize {
        self.width(i, self.natural[i].min(TAB_TEXT_MIN))
    }

    /// The text limits of the tabs `from..=to` that make them fit in `budget` cells: the longest
    /// text gives up a cell at a time, the current tab's text last, none below the smallest.
    /// `None` when even that is not enough.
    fn limits(&self, from: usize, to: usize, budget: usize) -> Option<Vec<usize>> {
        let mut limits: Vec<usize> = (from..=to).map(|i| self.natural[i]).collect();
        let mut total: usize = (from..=to).map(|i| self.width(i, limits[i - from])).sum();
        while total > budget {
            let pick = (from..=to)
                .filter(|&i| !self.info.tabs[i].current && limits[i - from] > TAB_TEXT_MIN)
                .max_by_key(|&i| (limits[i - from], i))
                .or_else(|| {
                    (from..=to)
                        .find(|&i| self.info.tabs[i].current && limits[i - from] > TAB_TEXT_MIN)
                })?;
            let before = self.width(pick, limits[pick - from]);
            limits[pick - from] -= 1;
            total = total - before + self.width(pick, limits[pick - from]);
        }
        Some(limits)
    }
}

/// How much a marker at an end of the bar says.
#[derive(Clone, Copy)]
enum MarkerLevel {
    /// The arrow, how many tabs are hidden, and the most urgent mark among them.
    Count,
    /// The arrow and the mark.
    Short,
}

/// The marker for the tabs `hidden` (indexes into the tabs), at the left (`before`) or the right
/// end, and the tab a click on it selects: the nearest of them.
fn marker_cells(
    hidden: std::ops::RangeInclusive<usize>,
    before: bool,
    level: MarkerLevel,
    info: &StatusInfo,
) -> (Vec<Cell>, usize) {
    let bar = theme::style(Role::StatusBar);
    let arrow = theme::symbol(if before {
        Symbol::HiddenBefore
    } else {
        Symbol::HiddenAfter
    });
    let text = match level {
        MarkerLevel::Count => format!("{arrow}{}", hidden.clone().count()),
        MarkerLevel::Short => arrow.to_string(),
    };
    let mut cells = text_cells(&text, bar);
    // The most urgent notification or failed report among the hidden tabs.
    let urgent = hidden
        .clone()
        .filter_map(|i| status_mark(&info.tabs[i]))
        .filter(|m| matches!(m, Mark::Notice(_) | Mark::ProgressError))
        .min_by_key(|m| m.rank());
    if let Some(mark) = urgent {
        cells.extend(mark_cells(mark, bar));
    }
    let nearest = if before {
        *hidden.end()
    } else {
        *hidden.start()
    };
    (cells, nearest)
}

/// The cells of the bar, where the cursor goes when it is a line to type into, and where each
/// tab that shows (whole or in part) is. A bar that a prompt, a hint or a message replaces has
/// no tabs.
pub fn build_with_tabs(cols: u16, info: &StatusInfo) -> (Vec<Cell>, Option<u16>, Vec<TabSpan>) {
    let width = usize::from(cols);
    let bar = theme::style(Role::StatusBar);
    if let Some(prompt) = &info.prompt {
        let style = theme::style(Role::Prompt);
        let mut cells = text_cells(&format!("{}{}", prompt.label, prompt.text), style);
        let cursor = cells.len().min(width.saturating_sub(1));
        cells = fit(cells, width, style);
        return (cells, Some(cursor as u16), Vec::new());
    }
    if let Some(hint) = &info.hint {
        let style = theme::style(Role::StatusBar);
        return (fit(text_cells(hint, style), width, style), None, Vec::new());
    }
    if let Some(message) = &info.message {
        let style = theme::style(Role::Message);
        return (
            fit(text_cells(&format!(" {message}"), style), width, style),
            None,
            Vec::new(),
        );
    }
    let mut words: Vec<String> = Vec::new();
    if info.prefix {
        words.push("PREFIX".to_string());
    }
    if info.locked {
        words.push(match &info.unlock_key {
            Some(key) => format!(
                "LOCK{}{key} to unlock",
                theme::symbol(Symbol::HintSeparator)
            ),
            None => "LOCK".to_string(),
        });
    }
    if info.recording {
        words.push("RECORDING KEYS".to_string());
    }
    let mut select_word = None;
    match &info.copy {
        None => {}
        Some(CopyState::Normal) => words.push("COPY".to_string()),
        Some(CopyState::Visual) => select_word = Some("VISUAL".to_string()),
        Some(CopyState::VisualLine) => select_word = Some("V-LINE".to_string()),
        Some(CopyState::Jump(typed)) => select_word = Some(format!("JUMP {typed}")),
    }
    let mut right: Vec<Cell> = Vec::new();
    for (class, count) in Class::BY_URGENCY.iter().zip(info.notices) {
        if count > 0 {
            right.push(blank(bar));
            right.extend(text_cells(
                &format!("{}{count}", theme::notice_symbol(*class)),
                theme::notice_style(*class),
            ));
        }
    }
    if !right.is_empty() {
        right.push(blank(bar));
    }
    right.extend(
        words
            .iter()
            .flat_map(|w| text_cells(&format!(" {w} "), theme::style(Role::Indicator))),
    );
    if let Some(word) = select_word {
        right.extend(text_cells(
            &format!(" {word} "),
            theme::style(Role::IndicatorSelect),
        ));
    }
    // The counters and the indicators keep their place at the right; the tabs give way.
    let room = if right.len() < width {
        width - right.len()
    } else {
        width
    };
    let (mut cells, spans) = lay_out_tabs(room, info);
    cells = fit(cells, room, bar);
    cells.extend(right.into_iter().take(width - room));
    // What was cut away cannot be clicked, and a tab cut in two is clickable where it shows.
    let spans = spans
        .into_iter()
        .filter(|(_, from, _, _)| *from < room)
        .map(|(index, from, to, marker)| TabSpan {
            index,
            from: from as u16,
            to: to.min(room) as u16,
            marker,
        })
        .collect();
    (cells, None, spans)
}

/// The session's label and the tabs in `room` cells: as they are when they fit; with the longest
/// names cut short (to no less than seven cells, the current tab's last) when they do not;
/// and only when that is not enough with tabs hidden, a marker at the end where they are. The
/// current tab always shows. Returns the cells and, for each tab or marker, its index (for a
/// marker the nearest hidden tab), where it starts and ends, and whether it is a marker.
fn lay_out_tabs(room: usize, info: &StatusInfo) -> Laid {
    let bar = theme::style(Role::StatusBar);
    let label = session_label(info);
    let label_width = label.len() + 1;
    let tabs = info.tabs.len();
    if tabs == 0 {
        let mut cells = label;
        cells.push(blank(bar));
        return (cells, Vec::new());
    }
    let fitting = Fitting::new(info);
    let current = info.tabs.iter().position(|t| t.current).unwrap_or(0);
    // A marker and the gap after it (at the left) or before it (at the right).
    let marker_room = |range: Option<std::ops::RangeInclusive<usize>>, before: bool, level| {
        range.map_or(0, |r| {
            marker_cells(r, before, level, info).0.len() + usize::from(before)
        })
    };
    // Everything shows, its names cut short only as far as they have to be.
    if let Some(limits) = fitting.limits(0, tabs - 1, room.saturating_sub(label_width)) {
        return assemble(&fitting, label, 0, tabs - 1, &limits, MarkerLevel::Count);
    }
    // Tabs are hidden: the label stays while the current tab and the markers fit beside it, then
    // goes; a marker that says less goes before the tab is cut.
    for (with_label, level) in [
        (true, MarkerLevel::Count),
        (true, MarkerLevel::Short),
        (false, MarkerLevel::Count),
        (false, MarkerLevel::Short),
    ] {
        let base = if with_label { label_width } else { 0 };
        let used = |lo: usize, hi: usize| {
            let before = (lo > 0).then(|| 0..=lo - 1);
            let after = (hi + 1 < tabs).then(|| hi + 1..=tabs - 1);
            base + marker_room(before, true, level)
                + marker_room(after, false, level)
                + (lo..=hi).map(|i| fitting.compact(i)).sum::<usize>()
        };
        if used(current, current) > room {
            continue;
        }
        let (mut lo, mut hi) = (current, current);
        loop {
            let mut grew = false;
            if hi + 1 < tabs && used(lo, hi + 1) <= room {
                hi += 1;
                grew = true;
            }
            if lo > 0 && used(lo - 1, hi) <= room {
                lo -= 1;
                grew = true;
            }
            if !grew {
                break;
            }
        }
        let before = (lo > 0).then(|| 0..=lo - 1);
        let after = (hi + 1 < tabs).then(|| hi + 1..=tabs - 1);
        let budget =
            room - base - marker_room(before, true, level) - marker_room(after, false, level);
        // The tabs that show grow back as far as the room goes.
        let limits = fitting.limits(lo, hi, budget).unwrap_or_else(|| {
            (lo..=hi)
                .map(|i| fitting.natural[i].min(TAB_TEXT_MIN))
                .collect()
        });
        let label = if with_label { label } else { Vec::new() };
        return assemble(&fitting, label, lo, hi, &limits, level);
    }
    // Not even the current tab and the markers fit: the tab is cut at its right end.
    let level = MarkerLevel::Short;
    let before = (current > 0).then(|| 0..=current - 1);
    let after = (current + 1 < tabs).then(|| current + 1..=tabs - 1);
    let left = marker_room(before, true, level);
    let right = marker_room(after, false, level);
    let own = room.saturating_sub(left + right).max(1);
    let limit = fitting.natural[current].min(TAB_TEXT_MIN);
    let (mut cells, mut spans) = assemble(&fitting, Vec::new(), current, current, &[limit], level);
    // The pieces of `assemble` in order: the left marker (and its gap), the tab, the gap, the right
    // marker. The tab is cut to what is left and the right marker follows it.
    let tab_from = left;
    let tab_to = cells.len().saturating_sub(right).min(tab_from + own);
    let tail: Vec<Cell> = cells.split_off(cells.len() - right);
    cells.truncate(tab_to);
    let end = cells.len();
    cells.extend(tail);
    for span in spans.iter_mut() {
        if !span.3 && span.0 == current {
            span.2 = end;
        } else if span.3 && span.1 > tab_from {
            let shift = span.1 - end;
            span.1 -= shift;
            span.2 -= shift;
        }
    }
    (cells, spans)
}

/// The cells of the label, the left marker, the tabs `lo..=hi` with their text limits, and the
/// right marker, with where each starts and ends.
fn assemble(
    fitting: &Fitting,
    label: Vec<Cell>,
    lo: usize,
    hi: usize,
    limits: &[usize],
    level: MarkerLevel,
) -> Laid {
    let info = fitting.info;
    let bar = theme::style(Role::StatusBar);
    let tabs = info.tabs.len();
    let mut cells = label;
    if !cells.is_empty() {
        cells.push(blank(bar));
    }
    let mut spans = Vec::new();
    if lo > 0 {
        let (marker, nearest) = marker_cells(0..=lo - 1, true, level, info);
        let from = cells.len();
        cells.extend(marker);
        spans.push((nearest, from, cells.len(), true));
        cells.push(blank(bar));
    }
    for i in lo..=hi {
        let from = cells.len();
        let pieces = shorten_pieces(&info.tabs[i].text, limits[i - lo]);
        cells.extend(tab_cells_with(&info.tabs[i], &pieces, info));
        spans.push((i, from, cells.len(), false));
        // One cell of the bar's own background between two tabs, so that they read as items.
        cells.push(blank(bar));
    }
    if hi + 1 < tabs {
        let (marker, nearest) = marker_cells(hi + 1..=tabs - 1, false, level, info);
        let from = cells.len();
        cells.extend(marker);
        spans.push((nearest, from, cells.len(), true));
    }
    (cells, spans)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn text(cells: &[Cell]) -> String {
        cells
            .iter()
            .map(|c| {
                if c.text.is_empty() {
                    " "
                } else {
                    c.text.as_str()
                }
            })
            .collect()
    }

    fn info() -> StatusInfo {
        StatusInfo {
            session: "work".into(),
            number: NumberPosition::Right,
            style: TabStyle::Flat,
            tabs: vec![
                Tab {
                    number: 1,
                    text: vec![TabPiece::plain("sh")],
                    current: false,
                    zoomed: false,
                    notice: None,
                    progress: None,
                },
                Tab {
                    number: 2,
                    text: vec![TabPiece::plain("vim")],
                    current: true,
                    zoomed: true,
                    notice: None,
                    progress: None,
                },
            ],
            prefix: false,
            locked: false,
            recording: false,
            unlock_key: None,
            copy: None,
            notices: [0; 4],
            message: None,
            prompt: None,
            hint: None,
        }
    }

    #[test]
    fn the_bar_shows_the_session_the_tabs_and_marks_the_current_and_zoomed_ones() {
        let (cells, cursor) = build(60, &info());
        assert_eq!(cells.len(), 60);
        assert!(cursor.is_none());
        let line = text(&cells);
        assert!(line.contains(" work "), "{line}");
        assert!(line.contains(" sh    1 "), "{line}");
        assert!(line.contains(" vim [Z]    2 "), "{line}");
        let at = |needle: &str| cells[line[..line.find(needle).unwrap()].chars().count()].style;
        assert_eq!(at("vim"), theme::style(Role::PillNameCurrent));
        assert_eq!(at("sh"), theme::style(Role::PillName));
        assert_ne!(at("vim"), at("sh"));
        // The number is a badge of its own at the end, in the accent colours in the current tab.
        assert_eq!(at("2 "), theme::style(Role::PillBadgeCurrent));
        assert_eq!(at("1 "), theme::style(Role::PillBadge));
    }

    #[test]
    fn each_tab_knows_the_columns_it_covers_as_drawn_and_a_replaced_bar_has_none() {
        let (cells, _, spans) = build_with_tabs(60, &info());
        let line = text(&cells);
        let col = |needle: &str| line.find(needle).unwrap() as u16;
        assert_eq!(spans.len(), 2);
        // A tab covers its number, name and the blanks around them, and no more.
        assert_eq!(spans[0].index, 0);
        assert_eq!(
            (spans[0].from, spans[0].to),
            (col(" sh    1 "), col(" sh    1 ") + 9)
        );
        assert_eq!(spans[1].index, 1);
        // One cell of the bar between two tabs, which belongs to neither.
        assert_eq!(spans[1].from, spans[0].to + 1);
        assert_eq!(spans[1].to, spans[1].from + " vim [Z]    2 ".len() as u16);
        // Too narrow for both: the first tab is hidden, a marker at the left stands for it (a
        // click on the marker selects it) and the tab of the window you are in shows whole.
        let (_, _, narrow) = build_with_tabs(18, &info());
        assert_eq!(narrow.first().map(|t| (t.index, t.marker)), Some((0, true)));
        assert_eq!(narrow.last().map(|t| (t.index, t.marker)), Some((1, false)));
        // Narrower still: the tab of the window you are in shows, and the first gives way to it.
        let (_, _, narrower) = build_with_tabs(12, &info());
        assert_eq!(narrower.last().map(|t| t.index), Some(1));
        assert!(narrower.iter().all(|t| t.to <= 12));
        // A message, a hint or a prompt in place of the bar leaves no tabs.
        let mut m = info();
        m.message = Some("hello".into());
        assert!(build_with_tabs(60, &m).2.is_empty());
        let mut h = info();
        h.hint = Some("hint".into());
        assert!(build_with_tabs(60, &h).2.is_empty());
    }

    #[test]
    fn the_copy_word_says_whether_a_selection_is_active_or_a_jump_is_open() {
        for (state, word) in [
            (CopyState::Normal, "COPY"),
            (CopyState::Visual, "VISUAL"),
            (CopyState::VisualLine, "V-LINE"),
            (CopyState::Jump("ab".into()), "JUMP ab"),
        ] {
            let mut i = info();
            i.copy = Some(state);
            let (cells, _) = build(60, &i);
            let line = text(&cells);
            assert!(line.trim_end().ends_with(&format!(" {word}")), "{line:?}");
        }
        let select = theme::style(Role::IndicatorSelect);
        let mut i = info();
        i.copy = Some(CopyState::Visual);
        let (cells, _) = build(60, &i);
        assert!(cells.iter().any(|c| c.style == select));
    }

    #[test]
    fn a_key_capture_is_said_in_words() {
        let mut i = info();
        i.recording = true;
        let line = text(&build(60, &i).0);
        assert!(line.trim_end().ends_with("RECORDING KEYS"), "{line:?}");
        assert!(!text(&build(60, &info()).0).contains("RECORDING"));
    }

    #[test]
    fn mode_words_are_at_the_right_end() {
        let mut i = info();
        i.prefix = true;
        i.locked = true;
        i.copy = Some(CopyState::Normal);
        let line = text(&build(60, &i).0);
        assert!(line.trim_end().ends_with("PREFIX  LOCK  COPY"), "{line:?}");
        let quiet = text(&build(60, &info()).0);
        assert!(!quiet.contains("PREFIX") && !quiet.contains("LOCK") && !quiet.contains("COPY"));
    }

    #[test]
    fn the_lock_word_names_the_unlock_key_when_there_is_one() {
        let mut i = info();
        i.locked = true;
        i.unlock_key = Some("ctrl+alt+g".into());
        let line = text(&build(60, &i).0);
        assert!(
            line.contains("LOCK \u{b7} ctrl+alt+g to unlock"),
            "{line:?}"
        );
    }

    #[test]
    fn a_message_or_a_prompt_replaces_the_bar() {
        let mut i = info();
        i.message = Some("no room to split".into());
        let line = text(&build(40, &i).0);
        assert!(line.contains("no room to split") && !line.contains("work"));
        let mut i = info();
        i.prompt = Some(Prompt {
            label: "rename: ".into(),
            text: "abc".into(),
        });
        let (cells, cursor) = build(40, &i);
        assert!(text(&cells).starts_with("rename: abc"));
        assert_eq!(cursor, Some(11));
    }

    /// A tab that says nothing, and the same tab with a mark of every kind.
    fn marks_to_try() -> Vec<(&'static str, Option<Class>, Option<arcade_term::Progress>)> {
        use arcade_term::Progress;
        let p = |state, value| Some(Progress { state, value });
        vec![
            ("nothing", None, None),
            ("approval", Some(Class::Approval), None),
            ("input", Some(Class::Input), None),
            ("error", Some(Class::Error), None),
            ("info", Some(Class::Info), None),
            ("value 0", None, p(1, 0)),
            ("value 42", None, p(1, 42)),
            ("value 100", None, p(1, 100)),
            ("failed", None, p(2, 0)),
            ("failed 30", None, p(2, 30)),
            ("busy", None, p(3, 0)),
            ("paused", None, p(4, 55)),
            ("error over 40%", Some(Class::Error), p(1, 40)),
            ("info over busy", Some(Class::Info), p(3, 0)),
        ]
    }

    /// Every tab has one status cell that is always there: a tab is as wide with a mark of any
    /// kind as with none, in both styles and at the three places of the number, so the tabs to its
    /// right never move.
    #[test]
    fn a_tab_is_as_wide_with_every_mark_as_with_none() {
        for style in [TabStyle::Pill, TabStyle::Flat] {
            for number in [
                NumberPosition::Left,
                NumberPosition::Right,
                NumberPosition::Off,
            ] {
                let spans_with = |notice, progress| {
                    let mut i = info();
                    i.style = style;
                    i.number = number;
                    i.tabs[0].notice = notice;
                    i.tabs[0].progress = progress;
                    build_with_tabs(120, &i).2
                };
                let none = spans_with(None, None);
                for (name, notice, progress) in marks_to_try() {
                    let other = spans_with(notice, progress);
                    assert_eq!(other.len(), none.len(), "{name}");
                    for (a, b) in other.iter().zip(&none) {
                        assert_eq!(
                            (a.from, a.to),
                            (b.from, b.to),
                            "{style:?} {number:?} {name}"
                        );
                    }
                }
            }
        }
    }

    /// Each state draws its own symbol after the name, in its colour, in both styles.
    #[test]
    fn the_status_cell_draws_the_symbol_of_the_state() {
        use arcade_term::Progress;
        let wanted = |notice: Option<Class>, progress: Option<Progress>| -> &'static str {
            match (notice, progress) {
                (Some(c), _) if c != Class::Info => theme::notice_symbol(c),
                (_, Some(Progress { state: 2, .. })) => theme::PROGRESS_ERROR_SYMBOL,
                (
                    _,
                    Some(Progress {
                        state: 1 | 4,
                        value,
                    }),
                ) => theme::level_block(value),
                (_, Some(Progress { state: 3, .. })) => theme::BUSY_SYMBOL,
                (Some(c), _) => theme::notice_symbol(c),
                _ => " ",
            }
        };
        for style in [TabStyle::Pill, TabStyle::Flat] {
            for (name, notice, progress) in marks_to_try() {
                let mut i = info();
                i.style = style;
                i.tabs[0].notice = notice;
                i.tabs[0].progress = progress;
                let (cells, _, spans) = build_with_tabs(120, &i);
                // The cell is the third from the end of the tab's text: the cell, a space, and the
                // number's badge follow.
                let line = text(&cells);
                let tab = shown_text(&cells, &spans[0]);
                assert!(
                    tab.contains(&format!("sh {} ", wanted(notice, progress))),
                    "{style:?} {name}: {tab:?} in {line:?}"
                );
            }
        }
        // The colours: a level in the progress colour, a pause dimmer, a failure and an
        // approval notification red, each readable on the tab.
        let block = theme::style(Role::PillName);
        let colour = |tint| theme::cell_style(tint, block).fg;
        assert_ne!(colour(theme::Tint::Progress), colour(theme::Tint::Paused));
        assert_ne!(
            colour(theme::Tint::Progress),
            colour(theme::Tint::ProgressError)
        );
        assert_ne!(
            colour(theme::Tint::ProgressError),
            colour(theme::Tint::Class(Class::Info))
        );
    }

    /// The more urgent state wins: a notification that asks for you or reports an error, a failed
    /// report, a value, an unknown amount, then a finished notification.
    #[test]
    fn the_status_cell_shows_the_most_urgent_state() {
        use arcade_term::Progress;
        let tab = |notice, progress| Tab {
            number: 1,
            text: vec![],
            current: false,
            zoomed: false,
            notice,
            progress,
        };
        let p = |state, value| Some(Progress { state, value });
        let mark = |notice, progress| status_mark(&tab(notice, progress));
        assert_eq!(mark(None, None), None);
        assert_eq!(
            mark(Some(Class::Error), p(1, 40)),
            Some(Mark::Notice(Class::Error))
        );
        assert_eq!(
            mark(Some(Class::Input), p(2, 0)),
            Some(Mark::Notice(Class::Input))
        );
        assert_eq!(mark(None, p(2, 5)), Some(Mark::ProgressError));
        assert_eq!(mark(Some(Class::Info), p(2, 5)), Some(Mark::ProgressError));
        assert_eq!(
            mark(Some(Class::Info), p(1, 40)),
            Some(Mark::Level {
                value: 40,
                paused: false
            })
        );
        assert_eq!(mark(Some(Class::Info), p(3, 0)), Some(Mark::Busy));
        assert_eq!(
            mark(Some(Class::Info), None),
            Some(Mark::Notice(Class::Info))
        );
        assert_eq!(
            mark(None, p(4, 20)),
            Some(Mark::Level {
                value: 20,
                paused: true
            })
        );
        // A value outranks an unknown amount only through the report itself: one report per tab.
        assert_eq!(mark(None, p(0, 0)), None);
    }

    /// The eight level blocks: 0 to 12 the lowest, 88 to 100 the full block.
    #[test]
    fn the_level_block_follows_the_value_in_eight_steps() {
        let blocks: Vec<&str> = (0..=100).map(theme::level_block).collect();
        assert_eq!(blocks[0], "\u{2581}");
        assert_eq!(blocks[12], "\u{2581}");
        assert_eq!(blocks[13], "\u{2582}");
        assert_eq!(blocks[87], "\u{2587}");
        assert_eq!(blocks[88], "\u{2588}");
        assert_eq!(blocks[100], "\u{2588}");
        assert!(blocks.windows(2).all(|w| w[0] <= w[1]));
        // Every symbol of the cell is one cell wide in the pane.
        for symbol in blocks
            .iter()
            .copied()
            .chain([theme::BUSY_SYMBOL, theme::PROGRESS_ERROR_SYMBOL])
        {
            assert_eq!(crate::width::text_width(symbol), 1, "{symbol}");
        }
    }

    /// A hidden tab with a failed report is as urgent as one with a notification of an error: the
    /// marker of the hidden tabs carries it.
    #[test]
    fn the_marker_of_hidden_tabs_carries_a_failed_report() {
        let mut i = four_windows();
        i.tabs[0].progress = Some(arcade_term::Progress { state: 2, value: 0 });
        i.tabs[1].progress = Some(arcade_term::Progress {
            state: 1,
            value: 50,
        });
        let (cells, _, spans) = build_with_tabs(40, &i);
        assert!(spans.iter().any(|s| s.marker), "{}", text(&cells));
        let line = text(&cells);
        assert!(line.contains(theme::PROGRESS_ERROR_SYMBOL), "{line}");
        assert!(
            !line.contains(theme::level_block(50)) || !line.contains("‹"),
            "{line}"
        );
    }

    #[test]
    fn when_the_tabs_do_not_fit_they_give_way_to_the_counters() {
        let mut i = info();
        for n in 3..30 {
            i.tabs.push(Tab {
                number: n,
                text: vec![TabPiece::plain("window")],
                current: false,
                zoomed: false,
                notice: None,
                progress: None,
            });
        }
        i.notices = [10, 0, 0, 0];
        let line = text(&build(60, &i).0);
        let approval = theme::notice_symbol(Class::Approval);
        assert!(
            line.trim_end().ends_with(&format!("{approval}10")),
            "{line:?}"
        );
    }

    #[test]
    fn a_tab_with_a_notice_shows_the_mark_of_its_class_and_the_counters_are_at_the_right() {
        let mut i = info();
        i.tabs[0].notice = Some(Class::Approval);
        i.notices = [2, 0, 1, 3];
        let (cells, _) = build(80, &i);
        let line = text(&cells);
        let [approval, error, dot] =
            [Class::Approval, Class::Error, Class::Info].map(theme::notice_symbol);
        assert!(line.contains(&format!("sh {approval} ")), "{line}");
        assert!(
            line.trim_end()
                .ends_with(&format!("{approval}2 {error}1 {dot}3")),
            "{line:?}"
        );
        let at = |needle: &str| cells[line[..line.find(needle).unwrap()].chars().count()].style;
        assert_eq!(
            at(&format!("{approval}2")),
            theme::notice_style(Class::Approval)
        );
        assert_ne!(
            theme::notice_style(Class::Approval),
            theme::notice_style(Class::Info)
        );
        // Four classes, four different marks, so that colour is not all that tells them apart.
        let marks: Vec<&str> = Class::BY_URGENCY
            .iter()
            .map(|c| theme::notice_symbol(*c))
            .collect();
        for (n, a) in marks.iter().enumerate() {
            for b in &marks[n + 1..] {
                assert_ne!(a, b);
            }
        }
        let quiet = text(&build(80, &info()).0);
        assert!(!quiet.contains(approval) && !quiet.contains(dot));
    }

    #[test]
    fn a_narrow_bar_is_cut_not_overflowed() {
        let (cells, _) = build(10, &info());
        assert_eq!(cells.len(), 10);
    }

    /// A tab that carries everything (number, name, title, zoom, notification, progress) at every
    /// narrow width: the bar has the width, the spans are in order and inside it, the current tab
    /// can always be clicked, and a tab that is shown whole shows its marks.
    #[test]
    fn a_tab_with_everything_keeps_its_clicks_and_marks_at_every_width() {
        let loaded = |number: usize, current: bool| Tab {
            number,
            text: vec![
                TabPiece::plain("a-long-directory-name"),
                TabPiece {
                    text: "a title of the program".into(),
                    quiet: true,
                },
            ],
            current,
            zoomed: true,
            notice: Some(Class::Input),
            progress: Some(arcade_term::Progress {
                state: 1,
                value: 40,
            }),
        };
        for (style, number) in [
            (TabStyle::Pill, NumberPosition::Right),
            (TabStyle::Pill, NumberPosition::Left),
            (TabStyle::Pill, NumberPosition::Off),
            (TabStyle::Flat, NumberPosition::Right),
            (TabStyle::Flat, NumberPosition::Left),
            (TabStyle::Flat, NumberPosition::Off),
        ] {
            let mut i = info();
            i.style = style;
            i.number = number;
            i.tabs = vec![loaded(1, false), loaded(2, true), loaded(3, false)];
            let (_, _, whole) = build_with_tabs(400, &i);
            let full: Vec<u16> = whole.iter().map(|t| t.to - t.from).collect();
            let mark = theme::notice_symbol(Class::Input);
            for width in 8..=120u16 {
                let (cells, _, spans) = build_with_tabs(width, &i);
                assert_eq!(
                    cells.len(),
                    usize::from(width),
                    "width {width} {style:?} {number:?}"
                );
                let mut last_end = 0;
                for span in &spans {
                    assert!(
                        span.from < span.to && span.to <= width,
                        "width {width} {style:?} {number:?}: {span:?}"
                    );
                    assert!(
                        span.from >= last_end,
                        "width {width} {style:?} {number:?}: {spans:?}"
                    );
                    last_end = span.to;
                    let shown: String = text(&cells[usize::from(span.from)..usize::from(span.to)]);
                    if span.to - span.from == full[span.index] {
                        assert!(
                            shown.contains(mark),
                            "width {width} {style:?} {number:?}: {shown:?}"
                        );
                    }
                }
                if width >= 40 {
                    assert!(
                        spans.iter().any(|t| t.index == 1),
                        "width {width} {style:?} {number:?}: {spans:?}"
                    );
                }
            }
        }
    }

    /// The flat style draws plain characters only: no cap glyph and no icon, so any font shows it.
    #[test]
    fn the_flat_style_uses_plain_characters_only_and_the_pill_style_adds_caps() {
        let mut i = info();
        i.style = TabStyle::Flat;
        let flat = text(&build(60, &i).0);
        assert!(
            flat.chars()
                .all(|c| !('\u{e000}'..='\u{f8ff}').contains(&c)),
            "{flat:?}"
        );
        i.style = TabStyle::Pill;
        let pill = text(&build(60, &i).0);
        assert!(
            pill.contains(theme::symbol(Symbol::PillCapLeft)),
            "{pill:?}"
        );
        assert!(
            pill.contains(theme::symbol(Symbol::PillCapRight)),
            "{pill:?}"
        );
        assert!(
            pill.contains(theme::symbol(Symbol::SessionIcon).trim()),
            "{pill:?}"
        );
    }

    /// Where the number goes: after the name (right), before it (left), or nowhere (off).
    #[test]
    fn the_number_goes_right_left_or_nowhere() {
        for (position, want) in [
            (NumberPosition::Right, " sh    1 "),
            (NumberPosition::Left, " 1  sh   "),
            (NumberPosition::Off, " sh   "),
        ] {
            let mut i = info();
            i.number = position;
            let line = text(&build(60, &i).0);
            assert!(line.contains(want), "{position:?}: {line}");
            if position == NumberPosition::Off {
                assert!(!line.contains(" 1 "), "{line}");
            }
        }
    }

    /// Names that share their beginning still differ after the cut, because the cut is in the
    /// middle and the end is kept.
    #[test]
    fn a_cut_name_keeps_its_end_so_names_with_one_beginning_stay_apart() {
        let cut = |name: &str, limit: usize| -> String {
            shorten_pieces(&[TabPiece::plain(name)], limit)
                .iter()
                .map(|p| p.text.as_str())
                .collect()
        };
        assert_eq!(cut("everfit-microservices", 9), "ever…ices");
        assert_eq!(cut("everfit-api", 9), "ever…-api");
        // Seven cells kept for a limit of eight: the start gets the odd one.
        assert_eq!(cut("everfit-microservices", 8), "ever…ces");
        assert_eq!(cut("abcdefghij", 4), "ab…j");
        assert_eq!(cut("short", 9), "short");
        assert_eq!(cut("abcdefghij", 1), "…");
        for limit in 4..=20 {
            let (a, b) = (
                cut("everfit-microservices", limit),
                cut("everfit-api", limit),
            );
            if limit < 11 {
                assert_ne!(a, b, "limit {limit}");
            }
            assert!(a.chars().count() <= limit && b.chars().count() <= limit);
        }
        // A title in its own quiet piece keeps its colour where it was cut.
        let pieces = [
            TabPiece::plain("name"),
            TabPiece {
                text: " a long title of the program".into(),
                quiet: true,
            },
        ];
        let cut = shorten_pieces(&pieces, 12);
        assert_eq!(pieces_width(&cut), 12);
        assert!(!cut[0].quiet && cut.last().unwrap().quiet);
    }

    /// A window named with a character that is wide in a pane (U+2B50, a star) still gives a bar
    /// exactly as wide as the screen, at the widths of the author's terminal and below, in both
    /// styles.
    #[test]
    fn a_wide_character_in_a_name_does_not_push_the_bar_out() {
        for style in [TabStyle::Pill, TabStyle::Flat] {
            let mut i = four_windows();
            i.style = style;
            i.session = "\u{2b50}work".into();
            i.tabs[0].text = vec![TabPiece::plain("\u{2b50}everfit-microservices")];
            i.tabs[1].text = vec![TabPiece::plain("api-\u{26a1}\u{2b50}")];
            for width in [150u16, 80, 50] {
                let (cells, _, spans) = build_with_tabs(width, &i);
                assert_eq!(cells.len(), usize::from(width), "{style:?} {width}");
                // Every wide character is followed by its second cell, none cut in half.
                for (n, cell) in cells.iter().enumerate() {
                    if cell.width == arcade_term::CellWidth::Wide {
                        assert_eq!(
                            cells.get(n + 1).map(|c| c.width),
                            Some(arcade_term::CellWidth::SpacerTail),
                            "{style:?} {width} at {n}"
                        );
                    }
                }
                assert!(spans.iter().all(|s| s.to <= width));
            }
        }
    }

    /// The four windows of the author's bar: three quiet, one with a notification that needs them.
    fn four_windows() -> StatusInfo {
        let tab = |number: usize, name: &str, current: bool, notice: Option<Class>| Tab {
            number,
            text: vec![TabPiece::plain(name)],
            current,
            zoomed: false,
            notice,
            progress: None,
        };
        let mut i = info();
        i.session = "work".into();
        i.tabs = vec![
            tab(1, "everfit-microservices", false, None),
            tab(2, "everfit-api", false, None),
            tab(3, "arcade", true, None),
            tab(4, "foundry", false, Some(Class::Input)),
        ];
        i
    }

    /// The cells of the tab `span` covers, as text.
    fn shown_text(cells: &[Cell], span: &TabSpan) -> String {
        text(&cells[usize::from(span.from)..usize::from(span.to)])
    }

    /// A window named with a heart in picture form or with flags still gives a bar exactly as wide
    /// as the screen, and a cut never lands inside such a sequence: no cell starts with a
    /// selector or a joiner, and a cell with a regional indicator holds exactly two.
    #[test]
    fn a_cut_never_lands_inside_a_joined_sequence() {
        let name = "\u{1f1fa}\u{1f1f8}-\u{2764}\u{fe0f}-everfit-microservices-\u{2764}\u{fe0f}-\u{1f1ec}\u{1f1e7}-api";
        let family = "\u{1f468}\u{200d}\u{1f469}\u{200d}\u{1f467}-everfit-api-\u{1f1fa}\u{1f1f8}";
        for style in [TabStyle::Pill, TabStyle::Flat] {
            let mut i = four_windows();
            i.style = style;
            i.tabs[0].text = vec![TabPiece::plain(name)];
            i.tabs[1].text = vec![TabPiece::plain(family)];
            for width in 20u16..=200 {
                let (cells, _, _) = build_with_tabs(width, &i);
                assert_eq!(cells.len(), usize::from(width), "{style:?} {width}");
                for cell in &cells {
                    let first = cell.text.chars().next();
                    assert!(
                        !matches!(first, Some('\u{fe0f}' | '\u{fe0e}' | '\u{200d}')),
                        "{style:?} {width}: a cell starts with a selector or joiner: {}",
                        text(&cells)
                    );
                    let indicators = cell
                        .text
                        .chars()
                        .filter(|c| ('\u{1f1e6}'..='\u{1f1ff}').contains(c))
                        .count();
                    assert!(
                        indicators == 0 || indicators == 2,
                        "{style:?} {width}: a flag is split: {}",
                        text(&cells)
                    );
                }
            }
        }
    }

    /// A cut name stays readable and is cut no more than the width needs: at every width the bar
    /// is as wide as the screen, the current tab shows, no visible name is under seven cells (or
    /// its own length when that is less), and no name is cut while the bar has room to show it
    /// whole.
    #[test]
    fn a_cut_name_keeps_seven_cells_and_takes_back_all_the_room_there_is() {
        for style in [TabStyle::Pill, TabStyle::Flat] {
            let mut i = four_windows();
            i.style = style;
            let (_, _, whole) = build_with_tabs(400, &i);
            // What a tab takes besides its name, at the width where nothing is cut.
            let names: Vec<usize> = i.tabs.iter().map(|t| pieces_width(&t.text)).collect();
            let chrome: Vec<usize> = whole
                .iter()
                .map(|s| usize::from(s.to - s.from) - names[s.index])
                .collect();
            for width in 20u16..=200 {
                let (cells, _, spans) = build_with_tabs(width, &i);
                let line = text(&cells);
                let ctx = format!("{style:?} at {width}: {line:?}");
                assert_eq!(cells.len(), usize::from(width), "{ctx}");
                assert!(
                    spans.iter().any(|s| !s.marker && s.index == 2),
                    "the current tab shows: {ctx}"
                );
                let end = spans.iter().map(|s| usize::from(s.to)).max().unwrap();
                // The cell after the last tab is its gap; whatever is beyond is empty room.
                let free = usize::from(width).saturating_sub(end + 1);
                for span in spans.iter().filter(|s| !s.marker) {
                    let name = usize::from(span.to - span.from) - chrome[span.index];
                    let natural = names[span.index];
                    assert!(
                        name >= natural.min(7),
                        "{} is cut too far: {ctx}",
                        span.index
                    );
                    assert!(
                        name == natural || free < natural - name,
                        "{} is cut with {free} empty cells to spare: {ctx}",
                        span.index
                    );
                }
            }
        }
    }

    /// Long names give up cells before any tab is hidden: every tab keeps its number and marks,
    /// and the current tab's name is the last one to be shortened.
    #[test]
    fn names_are_shortened_before_any_tab_is_hidden() {
        let mut i = four_windows();
        i.style = TabStyle::Flat;
        let (_, _, whole) = build_with_tabs(400, &i);
        assert_eq!(whole.len(), 4);
        // Room for everything once the longest names are cut: nothing is hidden.
        let natural = usize::from(whole.last().unwrap().to) + 1;
        let (cells, _, spans) = build_with_tabs((natural - 12) as u16, &i);
        assert_eq!(spans.len(), 4, "{}", text(&cells));
        assert!(spans.iter().all(|s| !s.marker));
        let line = text(&cells);
        assert!(line.contains('…'), "{line}");
        // The numbers and the notification mark are all there, and the current name is whole.
        for number in 1..=4 {
            assert!(line.contains(&format!(" {number} ")), "{line}");
        }
        assert!(line.contains(theme::notice_symbol(Class::Input)), "{line}");
        assert!(line.contains("arcade"), "{line}");
    }

    /// Hidden tabs are told by a marker at their end of the bar, with the count, and a hidden
    /// notification shows on it in its colour.
    #[test]
    fn hidden_tabs_leave_a_marker_that_shows_the_count_and_the_most_urgent_mark() {
        for style in [TabStyle::Pill, TabStyle::Flat] {
            // The notifications are on the tabs that are hidden: an input one on the first
            // window, an approval one (more urgent) on the second.
            let mut i = four_windows();
            i.style = style;
            i.tabs[0].notice = Some(Class::Input);
            i.tabs[1].notice = Some(Class::Approval);
            i.tabs[3].notice = None;
            // The widest bar on which the first two tabs are both hidden.
            let width = (20..=80u16)
                .rev()
                .find(|&w| {
                    let (_, _, spans) = build_with_tabs(w, &i);
                    spans.iter().filter(|s| !s.marker).all(|s| s.index >= 2)
                        && spans.iter().any(|s| s.marker && s.index == 1)
                })
                .expect("a width with two tabs hidden");
            let (cells, _, spans) = build_with_tabs(width, &i);
            let line = text(&cells);
            let left = spans.iter().find(|s| s.marker).expect(&line);
            // A click on the marker goes to the hidden tab nearest to the ones that show.
            assert_eq!(left.index, 1, "{line}");
            let shown = shown_text(&cells, left);
            let arrow = theme::symbol(Symbol::HiddenBefore);
            let approval = theme::notice_symbol(Class::Approval);
            assert_eq!(shown, format!("{arrow}2{approval}"), "{style:?}: {line}");
            // The mark is in its colour, on the bar's own background.
            let mark = cells[usize::from(left.to) - 1].style;
            let want = theme::cell_style(
                theme::Tint::Class(Class::Approval),
                theme::style(Role::StatusBar),
            );
            assert_eq!(mark, want, "{style:?}");
            // At the right end, tabs hidden after the last one that shows.
            let (cells, _, spans) = build_with_tabs(30, &i);
            let line = text(&cells);
            let right = spans
                .iter()
                .rev()
                .find(|s| s.marker)
                .unwrap_or_else(|| panic!("{style:?}: {line}"));
            assert!(right.index > 2, "{style:?}: {line}");
            assert!(
                shown_text(&cells, right).starts_with(theme::symbol(Symbol::HiddenAfter)),
                "{style:?}: {line}"
            );
        }
    }

    /// The four windows at 150, 80 and 50 columns, as the author's bar would draw them.
    #[test]
    fn the_four_windows_at_150_80_and_50_columns() {
        let mut i = four_windows();
        i.style = TabStyle::Pill;
        // Everything fits at 150.
        let (cells, _, spans) = build_with_tabs(150, &i);
        assert_eq!(spans.len(), 4, "{}", text(&cells));
        assert!(!text(&cells).contains('…'));
        // At 80 the names are cut and the fourth tab, with its mark, is still there.
        let (cells, _, spans) = build_with_tabs(80, &i);
        let line = text(&cells);
        assert_eq!(spans.len(), 4, "{line}");
        assert!(line.contains('…'), "{line}");
        assert!(line.contains(theme::notice_symbol(Class::Input)), "{line}");
        // At 50 tabs are hidden, and the marker for the fourth carries its mark.
        let (cells, _, spans) = build_with_tabs(50, &i);
        let line = text(&cells);
        assert!(spans.iter().any(|s| s.marker), "{line}");
        assert!(line.contains(theme::notice_symbol(Class::Input)), "{line}");
        assert!(spans.iter().any(|s| !s.marker && s.index == 2), "{line}");
    }

    /// At every width from 8 to 120, in both styles and with the number in each of its places: a
    /// tab is never gone without a marker on its side, the notification of a hidden tab always
    /// shows on a marker, tabs are hidden only when the names cut as short as they go still do not
    /// fit, and the tab of the current window always shows.
    #[test]
    fn at_every_width_no_tab_disappears_without_a_marker() {
        for style in [TabStyle::Pill, TabStyle::Flat] {
            for number in [
                NumberPosition::Right,
                NumberPosition::Left,
                NumberPosition::Off,
            ] {
                for notice_on in 0..4usize {
                    let mut i = four_windows();
                    i.style = style;
                    i.number = number;
                    for (n, tab) in i.tabs.iter_mut().enumerate() {
                        tab.notice = (n == notice_on).then_some(Class::Approval);
                    }
                    let fitting = Fitting::new(&i);
                    let compact_all: usize = session_label(&i).len()
                        + 1
                        + (0..i.tabs.len()).map(|t| fitting.compact(t)).sum::<usize>();
                    for width in 8..=120u16 {
                        let ctx =
                            format!("width {width} {style:?} {number:?} notice on {notice_on}");
                        let (cells, _, spans) = build_with_tabs(width, &i);
                        assert_eq!(cells.len(), usize::from(width), "{ctx}");
                        let tabs: Vec<&TabSpan> = spans.iter().filter(|s| !s.marker).collect();
                        let markers: Vec<&TabSpan> = spans.iter().filter(|s| s.marker).collect();
                        // The current tab always shows.
                        assert!(tabs.iter().any(|s| s.index == 2), "{ctx}: {}", text(&cells));
                        let first = tabs.first().unwrap().index;
                        let last = tabs.last().unwrap().index;
                        // Hidden before the first shown one: a marker at the left of it.
                        if first > 0 {
                            let m = markers
                                .iter()
                                .find(|m| m.to <= tabs[0].from)
                                .unwrap_or_else(|| {
                                    panic!("{ctx}: no left marker: {}", text(&cells))
                                });
                            assert_eq!(m.index, first - 1, "{ctx}");
                            assert!(
                                shown_text(&cells, m)
                                    .starts_with(theme::symbol(Symbol::HiddenBefore)),
                                "{ctx}"
                            );
                        }
                        if last + 1 < i.tabs.len() {
                            let m = markers
                                .iter()
                                .find(|m| m.from >= tabs.last().unwrap().to)
                                .unwrap_or_else(|| {
                                    panic!("{ctx}: no right marker: {}", text(&cells))
                                });
                            assert_eq!(m.index, last + 1, "{ctx}");
                            assert!(
                                shown_text(&cells, m)
                                    .starts_with(theme::symbol(Symbol::HiddenAfter)),
                                "{ctx}"
                            );
                        }
                        // A hidden notification shows on the marker of its side.
                        if notice_on < first || notice_on > last {
                            let side = markers
                                .iter()
                                .find(|m| (notice_on < first) == (m.to <= tabs[0].from))
                                .unwrap();
                            assert!(
                                shown_text(&cells, side)
                                    .contains(theme::notice_symbol(Class::Approval)),
                                "{ctx}: {}",
                                text(&cells)
                            );
                        }
                        // Tabs are hidden only when the names cut as short as they go do not fit.
                        if first > 0 || last + 1 < i.tabs.len() {
                            assert!(compact_all > usize::from(width), "{ctx}: {}", text(&cells));
                        }
                    }
                }
            }
        }
    }

    /// Writes the bar as the host terminal receives it, in both styles, to the folder named by
    /// `ARCADE_SAMPLE_DIR` (nothing happens when it is not set), so it can be looked at with `cat`.
    #[test]
    fn sample_bars_for_looking_at() {
        let Ok(dir) = std::env::var("ARCADE_SAMPLE_DIR") else {
            return;
        };
        for (style, name) in [(TabStyle::Pill, "pill"), (TabStyle::Flat, "flat")] {
            for (number, place) in [
                (NumberPosition::Right, "right"),
                (NumberPosition::Left, "left"),
                (NumberPosition::Off, "off"),
            ] {
                let mut i = info();
                i.style = style;
                i.number = number;
                i.tabs.push(Tab {
                    number: 3,
                    text: vec![TabPiece::plain("logs")],
                    current: false,
                    zoomed: false,
                    notice: Some(Class::Input),
                    progress: None,
                });
                let (cells, _) = build(60, &i);
                let mut bytes = Vec::new();
                crate::render::push_row(&mut bytes, 0, &cells, &[]);
                bytes.extend_from_slice(b"\x1b[0m\n");
                std::fs::write(format!("{dir}/bar-{name}-{place}.bin"), bytes).unwrap();
            }
            // The four windows of the narrow-bar case at three widths, as the host receives them
            // (`.bin`, with the colours) and as plain text (`.txt`).
            for width in [150u16, 80, 50] {
                let mut i = four_windows();
                i.style = style;
                let (cells, _) = build(width, &i);
                let mut bytes = Vec::new();
                crate::render::push_row(&mut bytes, 0, &cells, &[]);
                bytes.extend_from_slice(b"\x1b[0m\n");
                std::fs::write(format!("{dir}/four-windows-{name}-{width}.bin"), bytes).unwrap();
                std::fs::write(
                    format!("{dir}/four-windows-{name}-{width}.txt"),
                    format!("{}\n", text(&cells)),
                )
                .unwrap();
            }
        }
    }
}
