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
    /// This is the counter for the other sessions, not a tab (a click on it opens the list of
    /// notifications).
    pub counter: bool,
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

#[derive(Clone)]
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
    /// What the shells of the window's panes say about their commands (see `shellmarks`).
    pub command: Option<CommandMark>,
}

/// What a command says on a tab.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum CommandMark {
    /// A command has been running for a while.
    Running,
    /// A long command ended well while nobody looked.
    FinishedWell,
    /// A long command ended with an error while nobody looked.
    FinishedWithError,
}

/// A line the user types into.
#[derive(Clone)]
pub struct Prompt {
    pub label: String,
    pub text: String,
    /// The prompt asks a question that is answered with one key (close this pane?), so its label
    /// is drawn like any question.
    pub question: bool,
}

/// What a line that replaces the bar for a moment says, which sets its colour.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum MessageKind {
    /// A result or a piece of news.
    #[default]
    Information,
    /// Something that did not work.
    Error,
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

#[derive(Clone)]
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
    /// The sessions that hold the notifications the counter counts, the one with the most urgent
    /// notification first; empty when the counter counts the session's own panes too.
    pub notice_sessions: Vec<String>,
    /// A line shown in place of the bar until the next key.
    pub message: Option<String>,
    pub message_kind: MessageKind,
    pub prompt: Option<Prompt>,
    /// The key hint, shown in place of the bar while the prefix is waiting for a key.
    pub hint: Option<String>,
}

/// The counter for the other sessions, one piece of the bar's style, the most urgent class first:
/// the symbol of each class in its colour and the count in the bar's text colour, on a tab's
/// surface. With `named` and when the sessions that hold the notifications are known it starts
/// with the name of the session that holds the most urgent one (cut in the middle to 12 cells)
/// and, when there are more, how many other sessions hold some.
fn counter_cells(info: &StatusInfo, named: bool) -> Vec<Cell> {
    let bar = theme::style(Role::StatusBar);
    let name = theme::style(Role::PillName);
    let mut inner: Vec<Cell> = Vec::new();
    if named && let Some(first) = info.notice_sessions.first() {
        let shown = shorten_pieces(&[TabPiece::plain(first.clone())], COUNTER_NAME_CELLS);
        let mut label: String = shown.iter().map(|p| p.text.as_str()).collect();
        if info.notice_sessions.len() > 1 {
            label.push_str(&format!(" +{}", info.notice_sessions.len() - 1));
        }
        inner.extend(text_cells(&format!(" {label}"), name));
    }
    let mut groups = 0;
    for (class, count) in Class::BY_URGENCY.iter().zip(info.notices) {
        if count > 0 {
            // One space after the name, two between the classes.
            let sep = if groups == 0 { " " } else { "  " };
            groups += 1;
            inner.extend(text_cells(sep, name));
            inner.extend(text_cells(
                theme::notice_symbol(*class),
                theme::notice_style(*class),
            ));
            inner.extend(text_cells(&count.to_string(), name));
        }
    }
    if inner.is_empty() {
        return Vec::new();
    }
    inner.extend(text_cells(" ", name));
    let mut cells = capped(
        info.style,
        inner,
        cap(Symbol::PillCapLeft, name),
        cap(Symbol::PillCapRight, name),
    );
    cells.insert(0, blank(bar));
    cells.push(blank(bar));
    cells
}

/// The longest a session's name is in the counter, in cells.
const COUNTER_NAME_CELLS: usize = 12;

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
    text.extend(text_cells(" ", name));
    // With a badge, the state shows in place of its number. With none (`number = "off"`) one
    // cell after the name is kept for it, so the tab's width does not depend on what it says.
    if info.number == NumberPosition::Off {
        match status_mark(tab) {
            Some(mark) => text.extend(mark_cells(mark, name, tab.current)),
            None => text.extend(text_cells(" ", name)),
        }
        text.extend(text_cells(" ", name));
    }
    let (number, badge) = badge_cells(tab, badge, name);
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
    /// A command that runs.
    Running,
    /// A long command that ended well, or with an error.
    Finished { ok: bool },
}

impl Mark {
    /// How urgent the mark is; the smaller, the more.
    fn rank(self) -> usize {
        let class = |c: Class| Class::BY_URGENCY.iter().position(|b| *b == c).unwrap_or(0);
        match self {
            Mark::Notice(c) if c != Class::Info => class(c),
            Mark::ProgressError => 10,
            Mark::Finished { ok: false } => 15,
            Mark::Level { .. } => 20,
            Mark::Busy => 30,
            Mark::Running => 35,
            Mark::Finished { ok: true } => 43,
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
    let command = tab.command.map(|c| match c {
        CommandMark::Running => Mark::Running,
        CommandMark::FinishedWell => Mark::Finished { ok: true },
        CommandMark::FinishedWithError => Mark::Finished { ok: false },
    });
    tab.notice
        .map(Mark::Notice)
        .into_iter()
        .chain(progress)
        .chain(command)
        .min_by_key(|m| m.rank())
}

/// The symbol of a mark and what colours it. Something running (a progress report with no
/// amount, a command) has one symbol.
fn mark_parts(mark: Mark) -> (&'static str, theme::Tint) {
    match mark {
        Mark::Notice(class) => (theme::notice_symbol(class), theme::Tint::Class(class)),
        Mark::ProgressError => (theme::failed_symbol(), theme::Tint::ProgressError),
        Mark::Level { value, paused } => (
            theme::level_block(value),
            if paused {
                theme::Tint::Paused
            } else {
                theme::Tint::Progress
            },
        ),
        Mark::Busy | Mark::Running => (theme::running_symbol(), theme::Tint::Running),
        Mark::Finished { ok: true } => (theme::finished_symbol(), theme::Tint::Finished),
        Mark::Finished { ok: false } => (theme::failed_symbol(), theme::Tint::ProgressError),
    }
}

/// The cell of a mark on the block it stands on, in the mark's colour.
fn mark_cells(mark: Mark, block: arcade_term::Style, current: bool) -> Vec<Cell> {
    let (symbol, tint) = mark_parts(mark);
    text_cells(symbol, theme::cell_style_on(tint, block, current))
}

/// The badge of a tab and its colours: the number on the grey badge (the accent one for the
/// current tab), or when the window has a state what `[status] state-style` says: `plain` shows
/// the state's symbol in its colour on the block of the name, `dark` and `filled` give the badge
/// a colour of its own, `number` keeps the number and colours the badge. The symbol is padded to
/// the width of the number so that the tab does not change. Returns the style the round cap next
/// to the badge takes its colour from.
fn badge_cells(
    tab: &Tab,
    quiet: arcade_term::Style,
    name: arcade_term::Style,
) -> (Vec<Cell>, arcade_term::Style) {
    let digits = tab.number.to_string().len();
    let Some(mark) = status_mark(tab) else {
        return (text_cells(&format!(" {} ", tab.number), quiet), quiet);
    };
    let (symbol, tint) = mark_parts(mark);
    let look = theme::state_style();
    let padded = format!(" {symbol}{} ", " ".repeat(digits - 1));
    match look {
        theme::StateStyle::Plain => (
            text_cells(&padded, theme::cell_style_on(tint, name, tab.current)),
            name,
        ),
        theme::StateStyle::Number => {
            let badge = theme::state_badge_on(look, tint, tab.current);
            (text_cells(&format!(" {} ", tab.number), badge), badge)
        }
        theme::StateStyle::Dark | theme::StateStyle::Filled => {
            let badge = theme::state_badge_on(look, tint, tab.current);
            (text_cells(&padded, badge), badge)
        }
    }
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
        .filter(|m| {
            matches!(
                m,
                Mark::Notice(_) | Mark::ProgressError | Mark::Finished { ok: false }
            )
        })
        .min_by_key(|m| m.rank());
    if let Some(mark) = urgent {
        cells.extend(mark_cells(mark, bar, false));
    }
    let nearest = if before {
        *hidden.end()
    } else {
        *hidden.start()
    };
    (cells, nearest)
}

/// A piece of the bar that is not a tab: blocks of text, each in its own colours with a cell of
/// padding inside the ends of the piece, and in the pill style round ends. `room` is the most
/// cells it may take; the text of the last block, when it does not fit, is cut at its end with
/// the cut mark.
fn piece_cells(style: TabStyle, blocks: &[(&str, arcade_term::Style)], room: usize) -> Vec<Cell> {
    let ends = if style == TabStyle::Pill { 2 } else { 0 };
    let mut budget = room.saturating_sub(ends + 2);
    let last = blocks.len() - 1;
    let mut cells = Vec::new();
    for (n, (text, block)) in blocks.iter().enumerate() {
        let text = if n == last {
            cut_to_cells(text, budget)
        } else {
            (*text).to_string()
        };
        budget = budget.saturating_sub(crate::width::text_width(&text));
        let padded = match (n == 0, n == last) {
            (true, true) => format!(" {text} "),
            (true, false) => format!(" {text}"),
            (false, true) => format!("{text} "),
            (false, false) => text,
        };
        cells.extend(text_cells(&padded, *block));
    }
    let first = blocks[0].1;
    capped(
        style,
        cells,
        cap(Symbol::PillCapLeft, first),
        cap(Symbol::PillCapRight, blocks[last].1),
    )
}

/// The least a message piece is worth drawing at: a symbol and one character.
fn message_fits(style: TabStyle, room: usize) -> bool {
    room >= if style == TabStyle::Pill { 9 } else { 7 }
}

/// What a message or a prompt draws at the right end of the bar in at most `room` cells, and
/// for a prompt where the cursor is in them. Nothing when there is none or no room.
fn message_cells(info: &StatusInfo, room: usize) -> (Vec<Cell>, Option<usize>) {
    if let Some(prompt) = &info.prompt {
        if room < if info.style == TabStyle::Pill { 7 } else { 5 } {
            return (Vec::new(), None);
        }
        let (cells, cursor) = prompt_piece(info.style, prompt, room);
        return (cells, Some(cursor));
    }
    let Some(message) = &info.message else {
        return (Vec::new(), None);
    };
    if !message_fits(info.style, room) {
        return (Vec::new(), None);
    }
    let (mark, accent) = match info.message_kind {
        MessageKind::Information => (Symbol::MessageInformation, Role::MessageInfo),
        MessageKind::Error => (Symbol::MessageError, Role::MessageError),
    };
    let mark = format!("{} ", theme::symbol(mark));
    let cells = piece_cells(
        info.style,
        &[
            (mark.as_str(), theme::style(accent)),
            (message.as_str(), theme::style(Role::PromptText)),
        ],
        room,
    );
    (cells, None)
}

fn words_len_of(words: &[(&str, Role)], style: TabStyle) -> usize {
    word_pieces(style, words, usize::MAX).len()
}

/// The least room the session label (`label` cells with the gap after it) and the current tab
/// need beside each other: the tab compact (`current_tab` cells) and the markers for the tabs
/// that are hidden at either end.
fn tabs_needed(info: &StatusInfo, label: usize, current_tab: usize) -> usize {
    let Some(current) = info.tabs.iter().position(|t| t.current) else {
        return label;
    };
    let tabs = info.tabs.len();
    let before = if current > 0 {
        marker_cells(0..=current - 1, true, MarkerLevel::Short, info)
            .0
            .len()
            + 1
    } else {
        0
    };
    let after = if current + 1 < tabs {
        marker_cells(current + 1..=tabs - 1, false, MarkerLevel::Short, info)
            .0
            .len()
    } else {
        0
    };
    label + before + after + current_tab
}

/// The indicator words side by side, a cell of the bar between two, in at most `room` cells: a
/// word that does not fit whole is cut, and the ones after it are left out.
fn word_pieces(style: TabStyle, words: &[(&str, Role)], room: usize) -> Vec<Cell> {
    let bar = theme::style(Role::StatusBar);
    let mut cells: Vec<Cell> = Vec::new();
    for (word, role) in words {
        let gap = usize::from(!cells.is_empty());
        let left = room.saturating_sub(cells.len() + gap);
        if left < 5 {
            break;
        }
        if gap == 1 {
            cells.push(blank(bar));
        }
        cells.extend(piece_cells(style, &[(*word, theme::style(*role))], left));
    }
    cells
}

/// The prompt as a piece: its label on the colour of a question, what was typed after it on a
/// surface; the cells it takes (at most `room`), and where the cursor goes in them. What was typed
/// is shown from its end when it is longer than the room, so the cursor stays on the bar; the
/// label is cut only when the room is very small.
fn prompt_piece(style: TabStyle, prompt: &Prompt, room: usize) -> (Vec<Cell>, usize) {
    const LEAST_TYPED: usize = 3;
    let label_role = if prompt.question {
        Role::MessageQuestion
    } else {
        Role::PromptLabel
    };
    let label = if prompt.label.ends_with(' ') {
        prompt.label.clone()
    } else {
        format!("{} ", prompt.label)
    };
    let ends = usize::from(style == TabStyle::Pill);
    // One cell of padding before the label, one after what was typed, which is where the cursor is.
    let body = room.saturating_sub(ends * 2 + 2);
    let label_room = crate::width::text_width(&label).min(body.saturating_sub(LEAST_TYPED).max(1));
    let label = cut_to_cells(&label, label_room);
    let typed_room = body.saturating_sub(crate::width::text_width(&label));
    let mut typed = prompt.text.clone();
    if crate::width::text_width(&typed) > typed_room {
        let mut kept = String::new();
        let mut used = 0;
        for unit in crate::width::units(&typed).iter().rev() {
            if used + unit.width > typed_room {
                break;
            }
            used += unit.width;
            kept.insert_str(0, unit.text);
        }
        typed = kept;
    }
    let label_style = theme::style(label_role);
    let typed_style = theme::style(Role::PromptText);
    let mut cells = text_cells(&format!(" {label}"), label_style);
    // A question answered by one key has nothing typed after it: the piece is the coloured block
    // alone, ended in its own colour, with the cursor on its last cell.
    if prompt.question {
        let cursor = ends + cells.len() - 1;
        let cells = capped(
            style,
            cells,
            cap(Symbol::PillCapLeft, label_style),
            cap(Symbol::PillCapRight, label_style),
        );
        return (cells, cursor);
    }
    let typed_cells = text_cells(&typed, typed_style);
    let cursor = ends + cells.len() + typed_cells.len();
    cells.extend(typed_cells);
    cells.extend(text_cells(" ", typed_style));
    let cells = capped(
        style,
        cells,
        cap(Symbol::PillCapLeft, label_style),
        cap(Symbol::PillCapRight, typed_style),
    );
    (cells, cursor)
}

/// The cells of the bar, where the cursor goes when it is a line to type into, and where each
/// tab that shows (whole or in part) is. A bar that a prompt, a hint or a message replaces has
/// no tabs.
pub fn build_with_tabs(cols: u16, info: &StatusInfo) -> (Vec<Cell>, Option<u16>, Vec<TabSpan>) {
    let width = usize::from(cols);
    let bar = theme::style(Role::StatusBar);
    // The key hint is a line made to fit the whole width of the bar, so it takes the whole row.
    if let Some(hint) = &info.hint {
        return (fit(text_cells(hint, bar), width, bar), None, Vec::new());
    }
    // A message is painted over the right end of the bar as it would be without it: the tabs, the
    // counter and the words stay where they are, and the cells under the message are replaced (a
    // question that is typed into keeps its room instead, below).
    if info.message.is_some() && info.prompt.is_none() {
        let mut bare = info.clone();
        bare.message = None;
        let (mut cells, cursor, mut spans) = build_with_tabs(cols, &bare);
        // One blank cell before it keeps it apart from what is left of a tab under its edge.
        let (message, _) = message_cells(info, (width / 2).saturating_sub(1));
        if message.is_empty() {
            return (cells, cursor, spans);
        }
        let from = width.saturating_sub(message.len() + 1);
        let painted = std::iter::once(blank(bar)).chain(message);
        for (at, cell) in painted.enumerate() {
            if let Some(under) = cells.get_mut(from + at) {
                *under = cell;
            }
        }
        // What is covered cannot be clicked.
        spans.retain(|t| usize::from(t.from) < from);
        for t in &mut spans {
            t.to = t.to.min(from as u16);
        }
        return (cells, cursor, spans);
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
    let mut shown_words: Vec<(&str, Role)> = words
        .iter()
        .map(|w| (w.as_str(), Role::Indicator))
        .collect();
    if let Some(word) = &select_word {
        shown_words.push((word.as_str(), Role::IndicatorSelect));
    }
    let words_cells = word_pieces(info.style, &shown_words, usize::MAX);
    let label = session_label(info);
    let current_tab = info
        .tabs
        .iter()
        .position(|t| t.current)
        .map_or(0, |i| Fitting::new(info).compact(i));
    // A message or a prompt stands at the right, left of the indicator words, in at most half
    // the bar together with them; it also leaves the session label and the current tab their
    // room, so it is the first to give way.
    let gap = usize::from(!words_cells.is_empty());
    let half = width / 2;
    let left_needed = tabs_needed(info, label.len() + 1, current_tab);
    // The counter says which session holds what it counts while everything else shows whole; it
    // is the first thing to give way: the name goes before any tab is cut, then the counter goes
    // down to the symbols and counts alone, and those go before the current tab is touched.
    let counters = {
        let short = counter_cells(info, false);
        let long = counter_cells(info, true);
        let fitting = Fitting::new(info);
        let natural_tabs: usize = (0..info.tabs.len())
            .map(|i| fitting.width(i, fitting.natural[i]))
            .sum();
        let gap = usize::from(!words_cells.is_empty());
        if long.len() > short.len()
            && label.len() + 1 + natural_tabs + long.len() + words_cells.len() + gap <= width
        {
            long
        } else if left_needed + short.len() + words_cells.len() + gap <= width {
            short
        } else {
            Vec::new()
        }
    };
    // A question that is asked takes its room before the current tab does; a message does not.
    let kept_for_tabs = if info.prompt.is_some() {
        label.len() + 1
    } else {
        left_needed
    };
    let message_room = half
        .saturating_sub(words_cells.len() + gap)
        .min(width.saturating_sub(kept_for_tabs + counters.len() + words_cells.len() + gap));
    let (message, message_cursor) = message_cells(info, message_room);
    let counter_len = counters.len();
    let mut right = counters.clone();
    let message_at = right.len();
    right.extend(message.iter().cloned());
    if !message.is_empty() && gap == 1 {
        right.push(blank(bar));
    }
    right.extend(words_cells);
    // The counters and the indicators keep their place at the right; the tabs give way. When
    // there is not room for the session label and the current tab beside them, the tabs give way
    // altogether, and the indicators are cut (the counters first) rather than the label.
    if !right.is_empty() && width < right.len() + left_needed {
        let available = width.saturating_sub(label.len() + 1);
        let words = if counters.len() + words_len_of(&shown_words, info.style) <= available {
            right[message_at + message.len() + usize::from(gap == 1 && !message.is_empty())..]
                .to_vec()
        } else {
            word_pieces(info.style, &shown_words, available)
        };
        // A prompt that is open keeps its place beside the words; a message gives way.
        let (message, message_cursor) = if info.prompt.is_some() {
            let gap = usize::from(!words.is_empty());
            let room = half
                .saturating_sub(words.len() + gap)
                .min(available.saturating_sub(words.len() + gap));
            message_cells(info, room)
        } else {
            (Vec::new(), None)
        };
        let mut cells = label;
        cells.push(blank(bar));
        let from = width.saturating_sub(
            words.len() + message.len() + usize::from(!words.is_empty() && !message.is_empty()),
        );
        cells = fit(cells, from, bar);
        let cursor = message_cursor.map(|c| (cells.len() + c).min(width.saturating_sub(1)) as u16);
        cells.extend(message);
        if !words.is_empty() && cells.len() < width && cells.len() + words.len() < width {
            cells.push(blank(bar));
        }
        cells.extend(words);
        return (fit(cells, width, bar), cursor, Vec::new());
    }
    let room = width - right.len();
    let (mut cells, spans) = lay_out_tabs(room, info);
    cells = fit(cells, room, bar);
    cells.extend(right.into_iter().take(width - room));
    let cursor =
        message_cursor.map(|c| (room + message_at + c).min(width.saturating_sub(1)) as u16);
    // What was cut away cannot be clicked, and a tab cut in two is clickable where it shows.
    let spans = spans
        .into_iter()
        .filter(|(_, from, _, _)| *from < room)
        .map(|(index, from, to, marker)| TabSpan {
            index,
            from: from as u16,
            to: to.min(room) as u16,
            marker,
            counter: false,
        })
        .collect();
    let mut spans: Vec<TabSpan> = spans;
    if counter_len > 0 {
        spans.push(TabSpan {
            index: 0,
            from: room as u16,
            to: (room + counter_len).min(width) as u16,
            marker: false,
            counter: true,
        });
    }
    (cells, cursor, spans)
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
                    command: None,
                },
                Tab {
                    number: 2,
                    text: vec![TabPiece::plain("vim")],
                    current: true,
                    zoomed: true,
                    notice: None,
                    progress: None,
                    command: None,
                },
            ],
            prefix: false,
            locked: false,
            recording: false,
            unlock_key: None,
            copy: None,
            notices: [0; 4],
            notice_sessions: Vec::new(),
            message: None,
            message_kind: MessageKind::Information,
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
        assert!(line.contains(" sh  1 "), "{line}");
        assert!(line.contains(" vim [Z]  2 "), "{line}");
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
            (col(" sh  1 "), col(" sh  1 ") + 7)
        );
        assert_eq!(spans[1].index, 1);
        // One cell of the bar between two tabs, which belongs to neither.
        assert_eq!(spans[1].from, spans[0].to + 1);
        assert_eq!(spans[1].to, spans[1].from + " vim [Z]  2 ".len() as u16);
        // Too narrow for both: the first tab is hidden, a marker at the left stands for it (a
        // click on the marker selects it) and the tab of the window you are in shows whole.
        let (_, _, narrow) = build_with_tabs(18, &info());
        assert_eq!(narrow.first().map(|t| (t.index, t.marker)), Some((0, true)));
        assert_eq!(narrow.last().map(|t| (t.index, t.marker)), Some((1, false)));
        // Narrower still: the tab of the window you are in shows, and the first gives way to it.
        let (_, _, narrower) = build_with_tabs(12, &info());
        assert_eq!(narrower.last().map(|t| t.index), Some(1));
        assert!(narrower.iter().all(|t| t.to <= 12));
        // A message leaves the tabs where they are, as far as the room goes; a hint in place
        // of the bar leaves none.
        let mut m = info();
        m.message = Some("hello".into());
        assert_eq!(build_with_tabs(60, &m).2, build_with_tabs(60, &info()).2);
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
        assert!(
            line.trim_end().ends_with("PREFIX   LOCK   COPY"),
            "{line:?}"
        );
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
    fn a_message_or_a_prompt_stands_at_the_right_and_the_tabs_stay() {
        let mut i = info();
        i.message = Some("no room to split".into());
        let line = text(&build(60, &i).0);
        assert!(line.contains(" work ") && line.contains(" vim"), "{line:?}");
        assert!(line.trim_end().ends_with("no room to split"), "{line:?}");
        let mut i = info();
        i.prompt = Some(Prompt {
            label: "rename: ".into(),
            text: "abc".into(),
            question: false,
        });
        let (cells, cursor) = build(60, &i);
        let line = text(&cells);
        assert!(
            line.starts_with(" work ") && line.contains(" vim"),
            "{line:?}"
        );
        assert!(line.contains(" rename: abc "), "{line:?}");
        // The cursor stands right after what was typed.
        assert_eq!(cursor, Some((line.find("abc").unwrap() + 3) as u16));
    }

    /// Every state that is not the tabs, for one style: its name and the bar's info.
    fn lines_to_try(style: TabStyle) -> Vec<(&'static str, StatusInfo)> {
        let base = || {
            let mut i = info();
            i.style = style;
            i
        };
        let mut states = Vec::new();
        let mut i = base();
        i.message = Some("opening https://example.com/a/long/address/to/open".into());
        states.push(("information", i));
        let mut i = base();
        i.message = Some("line 3: unknown setting `tab-stile` in the [status] table".into());
        i.message_kind = MessageKind::Error;
        states.push(("error", i));
        let mut i = base();
        i.prompt = Some(Prompt {
            label: "Close this pane? (y/n) ".into(),
            text: String::new(),
            question: true,
        });
        states.push(("question", i));
        let mut i = base();
        i.prompt = Some(Prompt {
            label: "Window name: ".into(),
            text: "build and test the whole workspace".into(),
            question: false,
        });
        states.push(("prompt", i));
        let mut i = base();
        i.hint = Some(" | split   - split down   c new window   x close".into());
        states.push(("hint", i));
        let mut i = base();
        i.prefix = true;
        states.push(("prefix", i));
        let mut i = base();
        i.locked = true;
        i.unlock_key = Some("ctrl+alt+g".into());
        states.push(("lock", i));
        let mut i = base();
        i.recording = true;
        states.push(("recording", i));
        let mut i = base();
        i.prefix = true;
        i.message = Some("no binding for z".into());
        i.message_kind = MessageKind::Error;
        states.push(("message-and-prefix", i));
        states
    }

    /// Every state keeps the session label first and the bar's background everywhere (no cell
    /// with the terminal's default black), in both styles and at wide, medium and narrow bars.
    #[test]
    fn every_state_of_the_bar_keeps_the_session_label_and_the_bar_background() {
        for style in [TabStyle::Pill, TabStyle::Flat] {
            for width in [150u16, 80, 40] {
                for (name, i) in lines_to_try(style) {
                    // The line of key hints is made to fit the whole row, label included.
                    if name == "hint" {
                        continue;
                    }
                    let (cells, cursor) = build(width, &i);
                    let label = session_label(&i);
                    let line = text(&cells);
                    let context = format!("{name} {style:?} {width}: {line:?}");
                    assert_eq!(cells.len(), usize::from(width), "{context}");
                    assert_eq!(cells[..label.len()], label[..], "{context}");
                    for (n, cell) in cells.iter().enumerate() {
                        assert_ne!(
                            cell.style.bg,
                            arcade_term::Color::Default,
                            "{context} at {n}"
                        );
                    }
                    assert_eq!(cursor.is_some(), name == "question" || name == "prompt");
                    if let Some(at) = cursor {
                        assert!(usize::from(at) < usize::from(width), "{context}");
                    }
                }
            }
        }
    }

    /// How many cells at the right end of the row belong to one piece that is not a tab: the
    /// run of cells with a colour of their own, round ends included, up to the bar's own cell.
    fn piece_width_at_the_right(cells: &[Cell]) -> usize {
        let bar = theme::style(Role::StatusBar).bg;
        let caps = [
            theme::symbol(Symbol::PillCapLeft),
            theme::symbol(Symbol::PillCapRight),
        ];
        cells
            .iter()
            .rev()
            .take_while(|c| c.style.bg != bar || caps.contains(&c.text.as_str()))
            .count()
    }

    /// A message or a prompt, with nothing else at the right, is never wider than half the screen,
    /// at every width from 20 to 200, and the current tab shows while it is up whenever it
    /// shows without it.
    #[test]
    fn a_message_never_takes_more_than_half_the_screen_and_the_current_tab_stays() {
        for style in [TabStyle::Pill, TabStyle::Flat] {
            for (name, i) in lines_to_try(style) {
                if !matches!(name, "information" | "error" | "question" | "prompt") {
                    continue;
                }
                for width in 20u16..=200 {
                    let (cells, _, spans) = build_with_tabs(width, &i);
                    let context = format!("{name} {style:?} {width}: {:?}", text(&cells));
                    assert_eq!(cells.len(), usize::from(width), "{context}");
                    let piece = piece_width_at_the_right(&cells);
                    assert!(piece <= usize::from(width) / 2, "{piece} cells: {context}");
                    let mut quiet = info();
                    quiet.style = style;
                    let (_, _, quiet_spans) = build_with_tabs(width, &quiet);
                    let shows = |spans: &[TabSpan]| {
                        spans
                            .iter()
                            .any(|s| !s.marker && !s.counter && s.index == 1)
                    };
                    // A prompt takes its room before the current tab does; a message is painted
                    // over the right end, so the current tab shows unless it lies under it.
                    if shows(&quiet_spans) && name != "question" && name != "prompt" {
                        let covered = usize::from(width) - piece;
                        let tab = quiet_spans
                            .iter()
                            .find(|s| !s.marker && !s.counter && s.index == 1);
                        assert!(
                            shows(&spans) || tab.is_some_and(|t| usize::from(t.to) > covered),
                            "the current tab is gone: {context}"
                        );
                    }
                }
            }
        }
    }

    /// With a mode word too, the message is painted at the far right over it, within half the
    /// screen, and the word keeps its place under it.
    #[test]
    fn a_message_is_painted_over_the_mode_word_and_keeps_to_half_the_screen() {
        for style in [TabStyle::Pill, TabStyle::Flat] {
            let (_, i) = lines_to_try(style).swap_remove(8);
            for width in 60u16..=200 {
                let (cells, _) = build(width, &i);
                let line = text(&cells);
                let trimmed = line.trim_end_matches('\u{e0b4}').trim_end();
                assert!(trimmed.ends_with("no binding for z"), "{width}: {line:?}");
                let m = line.find("no binding").unwrap();
                let from = line[..m].chars().count();
                assert!(
                    usize::from(width) - from <= usize::from(width) / 2 + 4,
                    "{width}: {line:?}"
                );
                // Without the message the word is at the right end, where the message is now.
                let mut quiet = i.clone();
                quiet.message = None;
                let quiet_line = text(&build(width, &quiet).0);
                assert!(quiet_line.contains("PREFIX"), "{width}: {quiet_line:?}");
            }
        }
    }

    /// The round ends of a piece are its colour on the bar's background, never on the default
    /// black: a mode word, a message, a question that is only a coloured block (its end in
    /// that colour) and a state badge.
    #[test]
    fn the_round_ends_of_every_piece_are_on_the_bars_background() {
        let bar = theme::style(Role::StatusBar).bg;
        let left = theme::symbol(Symbol::PillCapLeft);
        let right = theme::symbol(Symbol::PillCapRight);
        for (name, i) in lines_to_try(TabStyle::Pill) {
            if name == "hint" {
                continue;
            }
            let (cells, _) = build(150, &i);
            for (n, c) in cells.iter().enumerate() {
                if c.text == left || c.text == right {
                    assert_eq!(c.style.bg, bar, "{name} cap at {n}");
                    assert_ne!(c.style.fg, bar, "{name} cap at {n}");
                }
            }
        }
        // A question has only its coloured block: both ends are in its colour.
        let (_, i) = lines_to_try(TabStyle::Pill)
            .into_iter()
            .find(|(n, _)| *n == "question")
            .unwrap();
        let cells = build(150, &i).0;
        let yellow = theme::style(Role::MessageQuestion).bg;
        let last_cap = cells.iter().rposition(|c| c.text == right).unwrap();
        assert_eq!(cells[last_cap].style.fg, yellow);
        // PREFIX and the question are the same yellow.
        assert_eq!(theme::style(Role::Indicator).bg, yellow);
    }

    /// The pill style rounds the ends of a message, a prompt and a mode mark as it does a tab;
    /// the flat style has a block with a cell of padding.
    #[test]
    fn a_message_is_a_pill_in_the_pill_style_and_a_block_in_the_flat_one() {
        let round = theme::symbol(Symbol::PillCapRight);
        for (name, i) in lines_to_try(TabStyle::Pill) {
            if name == "hint" {
                continue;
            }
            let (cells, _) = build(150, &i);
            let right_of_tabs = text(&cells[cells.len() / 2..]);
            assert!(right_of_tabs.contains(round), "{name}: {right_of_tabs:?}");
        }
        for (name, i) in lines_to_try(TabStyle::Flat) {
            let line = text(&build(150, &i).0);
            assert!(!line.contains(round), "{name}: {line:?}");
        }
    }

    #[test]
    fn the_colour_of_a_message_says_what_kind_it_is() {
        let at = |i: &StatusInfo, role| {
            build(150, i)
                .0
                .iter()
                .any(|c| c.style == theme::style(role) && !c.text.is_empty())
        };
        let states = lines_to_try(TabStyle::Flat);
        let find = |n: &str| &states.iter().find(|(name, _)| *name == n).unwrap().1;
        assert!(at(find("information"), Role::MessageInfo));
        assert!(at(find("information"), Role::PromptText));
        assert!(at(find("error"), Role::MessageError));
        assert!(at(find("question"), Role::MessageQuestion));
        assert!(at(find("prompt"), Role::PromptLabel));
        assert!(at(find("prompt"), Role::PromptText));
        assert!(at(find("prefix"), Role::Indicator));
    }

    /// A message longer than its room is cut at its end with the cut mark, so a configuration
    /// error keeps its start; what is typed in a prompt is shown from its end, so the cursor
    /// stays on the bar.
    #[test]
    fn a_long_message_keeps_its_start_and_a_long_prompt_keeps_its_end() {
        for style in [TabStyle::Pill, TabStyle::Flat] {
            let mut i = info();
            i.style = style;
            i.message = Some("line 12: the key `copy.yank` is not a command that exists".into());
            i.message_kind = MessageKind::Error;
            let line = text(&build(80, &i).0);
            assert!(line.contains("line 12: the"), "{line:?}");
            assert!(line.contains(theme::symbol(Symbol::Ellipsis)), "{line:?}");
            let mut i = info();
            i.style = style;
            i.prompt = Some(Prompt {
                label: "Window name: ".into(),
                text: "a very long name that does not fit on the bar at all, ending here".into(),
                question: false,
            });
            let (cells, cursor) = build(80, &i);
            let line = text(&cells);
            assert!(line.contains("ending here"), "{line:?}");
            assert!(usize::from(cursor.unwrap()) < 80, "{line:?}");
        }
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

    /// A tab is as wide with a state of any kind as with none, in both styles, with the number at
    /// the right, at the left and off, with a one-digit and a two-digit number, so the tabs to its
    /// right never move.
    #[test]
    fn a_tab_is_as_wide_with_every_mark_as_with_none() {
        for look in [
            theme::StateStyle::Plain,
            theme::StateStyle::Dark,
            theme::StateStyle::Filled,
            theme::StateStyle::Number,
        ] {
            let mut setting = theme::StateLook::default();
            setting.style = look;
            theme::set_state_look(setting);
            tab_widths_do_not_depend_on_the_mark();
        }
        theme::set_state_look(theme::StateLook::default());
    }

    fn tab_widths_do_not_depend_on_the_mark() {
        for style in [TabStyle::Pill, TabStyle::Flat] {
            for number in [
                NumberPosition::Left,
                NumberPosition::Right,
                NumberPosition::Off,
            ] {
                for first_number in [1usize, 12] {
                    let spans_with = |notice, progress| {
                        let mut i = info();
                        i.style = style;
                        i.number = number;
                        i.tabs[0].number = first_number;
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
                                "{style:?} {number:?} {first_number} {name}"
                            );
                        }
                    }
                }
            }
        }
    }

    /// A symbol from the settings is the one drawn on the tab and in the counter for other
    /// sessions, in every style; the counter keeps its width.
    #[test]
    fn a_symbol_from_the_settings_reaches_the_tab_and_the_counter() {
        for look in [
            theme::StateStyle::Plain,
            theme::StateStyle::Dark,
            theme::StateStyle::Filled,
        ] {
            let mut setting = theme::StateLook::default();
            setting.style = look;
            setting.set(
                theme::StateName::Input,
                theme::StateSetting {
                    symbol: Some(theme::intern("Q")),
                    color: theme::parse_color("#445566"),
                },
            );
            let mut i = info();
            i.tabs[0].notice = Some(Class::Input);
            i.notices = [0, 1, 0, 0];
            theme::set_state_look(theme::StateLook::default());
            let (before, _, spans_before) = build_with_tabs(110, &i);
            theme::set_state_look(setting);
            let (cells, _, spans) = build_with_tabs(110, &i);
            let line = text(&cells);
            assert_eq!(line.matches('Q').count(), 2, "{look:?} {line:?}");
            assert_eq!(cells.len(), before.len());
            assert_eq!(
                spans.iter().map(|s| (s.from, s.to)).collect::<Vec<_>>(),
                spans_before
                    .iter()
                    .map(|s| (s.from, s.to))
                    .collect::<Vec<_>>()
            );
            let at = line.rfind('Q').unwrap();
            let counter = cells[line[..at].chars().count()].style;
            assert_eq!(
                counter.fg,
                theme::parse_color("#445566").unwrap(),
                "{look:?}"
            );
        }
        theme::set_state_look(theme::StateLook::default());
    }

    /// A state that blinks alternates its colour between two values on the bar, the symbol and
    /// the width stay, a state that does not blink never changes, and outside the bar (the lists,
    /// the corner of a pane) nothing blinks.
    #[test]
    fn a_blinking_symbol_alternates_its_colour_and_nothing_else() {
        let mut setting = theme::StateLook::default();
        setting.set_blink(theme::StateName::Approval, false);
        theme::set_state_look(setting);
        let mut i = info();
        i.tabs[0].notice = Some(Class::Input);
        i.tabs[1].notice = Some(Class::Approval);
        let draw = |dim| theme::drawing_the_bar(dim, || build_with_tabs(120, &i)).0;
        let (bright, _, spans_bright) = draw(false);
        let (dim, _, spans_dim) = draw(true);
        assert_eq!(text(&bright), text(&dim));
        assert_eq!(
            spans_bright
                .iter()
                .map(|s| (s.from, s.to))
                .collect::<Vec<_>>(),
            spans_dim.iter().map(|s| (s.from, s.to)).collect::<Vec<_>>()
        );
        let differing: Vec<usize> = (0..bright.len())
            .filter(|n| bright[*n].style != dim[*n].style)
            .collect();
        // Only the block of the first tab's status cell (the symbol and the spaces round it).
        let line = text(&bright);
        let at = line
            .chars()
            .position(|c| theme::notice_symbol(Class::Input).starts_with(c) && c != ' ')
            .unwrap();
        assert!(differing.contains(&at), "{differing:?} {at}");
        assert!(
            differing.iter().all(|n| n.abs_diff(at) <= 1),
            "{differing:?} {at}"
        );
        // Whether a symbol that blinks is on the bar is told.
        assert!(theme::drawing_the_bar(false, || build_with_tabs(120, &i)).1);
        i.tabs[0].notice = None;
        assert!(!theme::drawing_the_bar(false, || build_with_tabs(120, &i)).1);
        // Outside the bar the colour is always the bright one.
        assert_eq!(
            theme::notice_style(Class::Input),
            theme::notice_style(Class::Input)
        );
        let outside = theme::notice_style(Class::Input);
        let (_, _) = theme::drawing_the_bar(true, || ());
        assert_eq!(theme::notice_style(Class::Input), outside);
        theme::set_state_look(theme::StateLook::default());
    }

    /// The counter names the session that holds what it counts: one session by name, several by
    /// the name of the one with the most urgent notification and how many others there are; the
    /// name is cut in the middle to 12 cells.
    #[test]
    fn the_counter_names_the_session_that_holds_what_it_counts() {
        let mut i = info();
        i.notices = [0, 0, 0, 1];
        i.notice_sessions = vec!["hahaa".to_string()];
        let line = text(&build(110, &i).0);
        assert!(line.contains(" hahaa \u{25cf}1 "), "{line:?}");
        i.notices = [1, 0, 0, 2];
        i.notice_sessions = vec!["hahaa".to_string(), "web".to_string()];
        let line = text(&build(110, &i).0);
        assert!(line.contains(" hahaa +1 \u{25b2}1  \u{25cf}2 "), "{line:?}");
        i.notice_sessions = vec!["a-very-long-session-name".to_string()];
        let line = text(&build(110, &i).0);
        let counter = line.trim_end();
        assert!(
            counter.contains("a-ver\u{2026}-name") || counter.contains('\u{2026}'),
            "{line:?}"
        );
        assert!(!counter.contains("a-very-long-session-name"), "{line:?}");
        // Without the names (the counter counts every pane) it is as it was.
        i.notice_sessions = Vec::new();
        let line = text(&build(110, &i).0);
        assert!(line.contains(" \u{25b2}1  \u{25cf}2 "), "{line:?}");
    }

    /// On a narrow bar the name is the first to give way, then the whole counter, and the current
    /// tab shows at every width from 20 to 200.
    #[test]
    fn the_counter_gives_way_before_the_tabs_do_and_the_current_tab_always_shows() {
        let mut i = info();
        i.tabs.push(Tab {
            number: 3,
            text: vec![TabPiece::plain("a-long-window-name")],
            current: false,
            zoomed: false,
            notice: None,
            progress: None,
            command: None,
        });
        i.notices = [1, 0, 0, 2];
        i.notice_sessions = vec!["hahaa".to_string(), "web".to_string()];
        let rank = |cells: &[Cell]| {
            let line = text(cells);
            if line.contains("hahaa") {
                0
            } else if line.contains('\u{25b2}') {
                1
            } else {
                2
            }
        };
        let mut previous = 0;
        for width in (20..=200u16).rev() {
            let (cells, _, spans) = build_with_tabs(width, &i);
            assert_eq!(cells.len(), usize::from(width));
            let now = rank(&cells);
            assert!(now >= previous, "width {width}: {:?}", text(&cells));
            previous = now;
            let current = i.tabs.iter().position(|t| t.current).unwrap();
            assert!(
                spans
                    .iter()
                    .any(|s| !s.marker && !s.counter && s.index == current),
                "width {width}: the current tab is missing in {:?}",
                text(&cells)
            );
            if now == 0 {
                // The name stays only while every tab shows whole.
                assert!(spans.iter().all(|s| !s.marker), "width {width}");
                let line = text(&cells);
                assert!(
                    line.contains("a-long-window-name"),
                    "width {width}: {line:?}"
                );
            }
            // A click on the counter lands on the counter, where it is.
            if let Some(counter) = spans.iter().find(|s| s.counter) {
                let line: Vec<char> = text(&cells).chars().collect();
                let shown: String = line[usize::from(counter.from)..usize::from(counter.to)]
                    .iter()
                    .collect();
                assert!(
                    shown.contains('\u{25cf}') || shown.contains('\u{25b2}'),
                    "{shown:?}"
                );
            }
        }
        // All three forms were seen.
        assert_eq!(previous, 2);
    }

    /// With a number a quiet tab is as narrow as it was before the status had a place of its
    /// own: the name, one cell, the number between its spaces.
    #[test]
    fn a_quiet_tab_with_a_number_is_as_narrow_as_it_ever_was() {
        for number in [NumberPosition::Left, NumberPosition::Right] {
            let mut i = info();
            i.number = number;
            i.style = TabStyle::Flat;
            let (cells, _, spans) = build_with_tabs(120, &i);
            // " sh " and " 1 " are 7 cells with the gap-free joins of the flat style.
            assert_eq!(
                spans[0].to - spans[0].from,
                7,
                "{number:?} {:?}",
                text(&cells)
            );
        }
    }

    /// Each state draws its own symbol, in its colour: in the badge in place of the number, and
    /// with no number in the one cell after the name.
    #[test]
    fn the_status_cell_draws_the_symbol_of_the_state() {
        use arcade_term::Progress;
        let wanted = |notice: Option<Class>, progress: Option<Progress>| -> &'static str {
            match (notice, progress) {
                (Some(c), _) if c != Class::Info => theme::notice_symbol(c),
                (_, Some(Progress { state: 2, .. })) => theme::failed_symbol(),
                (
                    _,
                    Some(Progress {
                        state: 1 | 4,
                        value,
                    }),
                ) => theme::level_block(value),
                (_, Some(Progress { state: 3, .. })) => theme::running_symbol(),
                (Some(c), _) => theme::notice_symbol(c),
                _ => " ",
            }
        };
        for style in [TabStyle::Pill, TabStyle::Flat] {
            for number in [
                NumberPosition::Left,
                NumberPosition::Right,
                NumberPosition::Off,
            ] {
                for (name, notice, progress) in marks_to_try() {
                    let mut i = info();
                    i.style = style;
                    i.number = number;
                    i.tabs[0].notice = notice;
                    i.tabs[0].progress = progress;
                    let (cells, _, spans) = build_with_tabs(120, &i);
                    let tab = shown_text(&cells, &spans[0]);
                    let context = format!("{style:?} {number:?} {name}: {tab:?}");
                    let symbol = wanted(notice, progress);
                    if number == NumberPosition::Off {
                        assert!(tab.contains(&format!("sh {symbol} ")), "{context}");
                    } else if symbol == " " {
                        assert!(tab.contains('1'), "{context}");
                    } else {
                        // The symbol is where the number was; the number is not there.
                        assert!(tab.contains(&format!(" {symbol} ")), "{context}");
                        assert!(!tab.contains('1'), "{context}");
                    }
                }
            }
        }
        // The number still shows in the other tab; the symbol's look follows the style.
        let tint = theme::Tint::Class(Class::Approval);
        for look in [
            theme::StateStyle::Plain,
            theme::StateStyle::Dark,
            theme::StateStyle::Filled,
        ] {
            let mut setting = theme::StateLook::default();
            setting.style = look;
            theme::set_state_look(setting);
            for current in [false, true] {
                let mut i = info();
                i.tabs[usize::from(current)].notice = Some(Class::Approval);
                let (cells, _, spans) = build_with_tabs(120, &i);
                let span = &spans[usize::from(current)];
                let tab = shown_text(&cells, span);
                assert!(!tab.contains('1'), "{look:?} {tab:?}");
                let at = tab.find(theme::notice_symbol(Class::Approval)).unwrap();
                let col = span.from as usize + tab[..at].chars().count();
                let drawn = cells[col].style;
                let name = theme::style(if current {
                    Role::PillNameCurrent
                } else {
                    Role::PillName
                });
                let want = match look {
                    theme::StateStyle::Plain => theme::cell_style(tint, name),
                    other => theme::state_badge(other, tint),
                };
                assert_eq!(
                    (drawn.bg, drawn.fg),
                    (want.bg, want.fg),
                    "{look:?} current={current}"
                );
            }
            let mut i = info();
            i.tabs[0].notice = Some(Class::Approval);
            let (cells, _, _) = build_with_tabs(120, &i);
            let line = text(&cells);
            assert!(line.contains(" 2 "), "{line:?}");
        }
        // `number`: the number stays, on the colour of the state, with no symbol on the tab.
        let mut setting = theme::StateLook::default();
        setting.style = theme::StateStyle::Number;
        theme::set_state_look(setting);
        let mut i = info();
        i.tabs[0].notice = Some(Class::Approval);
        let (cells, _, spans) = build_with_tabs(120, &i);
        let tab = shown_text(&cells, &spans[0]);
        assert!(tab.contains('1'), "{tab:?}");
        assert!(
            !tab.contains(theme::notice_symbol(Class::Approval)),
            "{tab:?}"
        );
        let col = spans[0].from as usize + tab.find('1').unwrap();
        let want = theme::state_badge(theme::StateStyle::Number, tint);
        assert_eq!(
            (cells[col].style.bg, cells[col].style.fg),
            (want.bg, want.fg)
        );
        theme::set_state_look(theme::StateLook::default());
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
            command: None,
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

    /// The command states take their place in the order: after an error notification, a failed
    /// report, and a command that ended with an error; before a progress value is passed by none
    /// (a value outranks a running command), and a running command outranks a finished one and an
    /// info notification. A tab is as wide with each of them.
    #[test]
    fn the_command_states_have_their_place_in_the_order_and_keep_the_tab_width() {
        use arcade_term::Progress;
        let p = |state, value| Some(Progress { state, value });
        let mark = |notice, progress, command| {
            status_mark(&Tab {
                number: 1,
                text: vec![],
                current: false,
                zoomed: false,
                notice,
                progress,
                command,
            })
        };
        use CommandMark::{FinishedWell, FinishedWithError, Running};
        assert_eq!(
            mark(Some(Class::Error), None, Some(FinishedWithError)),
            Some(Mark::Notice(Class::Error))
        );
        assert_eq!(
            mark(None, p(2, 0), Some(FinishedWithError)),
            Some(Mark::ProgressError)
        );
        assert_eq!(
            mark(None, p(1, 40), Some(FinishedWithError)),
            Some(Mark::Finished { ok: false })
        );
        assert_eq!(
            mark(None, p(1, 40), Some(Running)),
            Some(Mark::Level {
                value: 40,
                paused: false
            })
        );
        assert_eq!(mark(None, p(3, 0), Some(Running)), Some(Mark::Busy));
        assert_eq!(mark(None, None, Some(Running)), Some(Mark::Running));
        assert_eq!(
            mark(Some(Class::Info), None, Some(Running)),
            Some(Mark::Running)
        );
        assert_eq!(
            mark(None, None, Some(FinishedWell)),
            Some(Mark::Finished { ok: true })
        );
        assert_eq!(mark(None, None, None), None);
        // The symbol is one cell and is the one of a report with no amount too; the tab keeps its width.
        assert_eq!(crate::width::text_width(theme::running_symbol()), 1);
        for style in [TabStyle::Pill, TabStyle::Flat] {
            for number in [
                NumberPosition::Left,
                NumberPosition::Right,
                NumberPosition::Off,
            ] {
                let spans_with = |command| {
                    let mut i = info();
                    i.style = style;
                    i.number = number;
                    i.tabs[0].command = command;
                    let (cells, _, spans) = build_with_tabs(120, &i);
                    (text(&cells), spans)
                };
                let (_, none) = spans_with(None);
                for command in [Running, FinishedWell, FinishedWithError] {
                    let (line, other) = spans_with(Some(command));
                    for (a, b) in other.iter().zip(&none) {
                        assert_eq!((a.from, a.to), (b.from, b.to), "{style:?} {command:?}");
                    }
                    let symbol = match command {
                        Running => theme::running_symbol(),
                        FinishedWell => theme::notice_symbol(Class::Info),
                        FinishedWithError => theme::failed_symbol(),
                    };
                    let shown = if number == NumberPosition::Off {
                        format!("sh {symbol} ")
                    } else {
                        format!(" {symbol} ")
                    };
                    assert!(line.contains(&shown), "{line:?}");
                }
            }
        }
    }

    /// A report with no amount and a command that runs say the same thing, with one symbol; and
    /// there is no counter when no other session has a notification.
    #[test]
    fn one_symbol_for_something_that_runs_and_no_counter_without_other_sessions() {
        let mut busy = info();
        busy.tabs[0].progress = Some(arcade_term::Progress { state: 3, value: 0 });
        let mut running = info();
        running.tabs[0].command = Some(CommandMark::Running);
        let tab = |i: &StatusInfo| {
            let (cells, _, spans) = build_with_tabs(120, i);
            (
                shown_text(&cells, &spans[0]),
                cells[spans[0].to as usize - 3].style,
            )
        };
        assert_eq!(tab(&busy), tab(&running));
        assert!(tab(&busy).0.contains(theme::running_symbol()));
        for style in [TabStyle::Pill, TabStyle::Flat] {
            let mut quiet = info();
            quiet.style = style;
            quiet.tabs[0].notice = Some(Class::Input);
            let line = text(&build(120, &quiet).0);
            // The tab says it; nothing is counted at the right end.
            assert_eq!(
                line.matches(theme::notice_symbol(Class::Input)).count(),
                1,
                "{line:?}"
            );
            quiet.notices = [0, 1, 0, 0];
            let line = text(&build(120, &quiet).0);
            assert_eq!(
                line.matches(theme::notice_symbol(Class::Input)).count(),
                2,
                "{line:?}"
            );
        }
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
            .chain([theme::running_symbol(), theme::failed_symbol()])
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
        assert!(line.contains(theme::failed_symbol()), "{line}");
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
                command: None,
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
        assert!(line.contains(&format!("sh  {approval} ")), "{line}");
        assert!(
            line.trim_end()
                .ends_with(&format!("{approval}2  {error}1  {dot}3")),
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

    /// The counters are one piece on the surface of a tab's name, in both styles: the symbol of
    /// each class in its colour, the count in the bar's text colour, and no cell with the colour
    /// of a class as its background. The bar is as wide as it was.
    #[test]
    fn the_counters_are_one_piece_on_a_surface_and_never_a_block_of_a_class_colour() {
        let surface = theme::style(Role::PillName);
        for style in [TabStyle::Pill, TabStyle::Flat] {
            for width in [150u16, 80, 40] {
                let mut i = info();
                i.style = style;
                i.notices = [2, 0, 1, 3];
                let (cells, _) = build(width, &i);
                assert_eq!(cells.len(), usize::from(width));
                let line = text(&cells);
                let counted = format!(
                    "{}2  {}1  {}3",
                    theme::notice_symbol(Class::Approval),
                    theme::notice_symbol(Class::Error),
                    theme::notice_symbol(Class::Info)
                );
                assert!(line.contains(&counted) || width < 80, "{line:?}");
                for class in Class::BY_URGENCY {
                    let mark = theme::notice_style(class);
                    for cell in &cells {
                        assert_ne!(cell.style.bg, mark.fg, "{class:?} {style:?} {width}");
                    }
                }
                if width >= 80 {
                    let at = line.find(theme::notice_symbol(Class::Approval)).unwrap();
                    let from = line[..at].chars().count();
                    // The symbol in its colour on the surface, then the count in the text colour.
                    assert_eq!(cells[from].style, theme::notice_style(Class::Approval));
                    assert_eq!(cells[from].style.bg, surface.bg);
                    assert_eq!(cells[from + 1].style, surface);
                }
            }
        }
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
            command: None,
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
            (NumberPosition::Right, " sh  1 "),
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
            command: None,
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
        // The numbers are all there but the last one, which the notification mark stands in for,
        // and the current name is whole.
        for number in 1..=3 {
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
                        let tabs: Vec<&TabSpan> =
                            spans.iter().filter(|s| !s.marker && !s.counter).collect();
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

    /// A crowded bar: many tabs with long names, the counter for another session and a message.
    fn crowded(tabs: usize) -> StatusInfo {
        let mut i = info();
        i.tabs = (0..tabs)
            .map(|n| Tab {
                number: n + 1,
                text: vec![TabPiece::plain(format!("window-name-{n}"))],
                current: n == tabs / 2,
                zoomed: false,
                notice: None,
                progress: None,
                command: None,
            })
            .collect();
        i.notices = [1, 0, 0, 2];
        i.notice_sessions = vec!["hahaa".to_string()];
        i
    }

    #[test]
    fn a_message_is_painted_over_the_right_end_and_nothing_is_laid_out_again() {
        for style in [TabStyle::Pill, TabStyle::Flat] {
            for tabs in [2, 6, 12, 30] {
                for width in [20u16, 40, 60, 80, 100, 160] {
                    let mut bare = crowded(tabs);
                    bare.style = style;
                    let mut said = crowded(tabs);
                    said.style = style;
                    said.message = Some("config reloaded".into());
                    let ctx = format!("{style:?} {tabs} tabs {width} columns");
                    let (before, _, spans_before) = build_with_tabs(width, &bare);
                    let (after, _, spans_after) = build_with_tabs(width, &said);
                    assert_eq!(after.len(), usize::from(width), "{ctx}");
                    // Every cell that changed is within the right half of the bar.
                    let changed: Vec<usize> = (0..before.len())
                        .filter(|&c| {
                            before[c].text != after[c].text || before[c].style != after[c].style
                        })
                        .collect();
                    let half = usize::from(width) / 2;
                    assert!(
                        changed.iter().all(|&c| c >= usize::from(width) - half),
                        "{ctx}: {changed:?}"
                    );
                    // The message is there, at the right end, and fills what it changed.
                    let line = text(&after);
                    assert!(
                        line.contains("config reloaded") || width < 60,
                        "{ctx}: {line}"
                    );
                    if let Some(&first) = changed.first() {
                        assert!(first >= half, "{ctx}");
                    }
                    // The tabs and the counter are where they were, except where covered.
                    let covered = changed.first().copied().unwrap_or(usize::from(width));
                    let before_spans: Vec<_> = spans_before
                        .iter()
                        .filter(|t| usize::from(t.from) < covered)
                        .map(|t| {
                            (
                                t.index,
                                t.from,
                                t.to.min(covered as u16),
                                t.marker,
                                t.counter,
                            )
                        })
                        .collect();
                    let after_spans: Vec<_> = spans_after
                        .iter()
                        .map(|t| (t.index, t.from, t.to, t.marker, t.counter))
                        .collect();
                    assert_eq!(before_spans, after_spans, "{ctx}");
                    for c in 0..covered {
                        assert_eq!(before[c].text, after[c].text, "{ctx} column {c}");
                    }
                }
            }
        }
    }

    /// The tab of the window that is current stays the one the eye finds while its state shows:
    /// a state that asks nothing (running, a level, paused, finished) has its symbol in the accent
    /// of the current tab; approval, input, error and a failure keep their own colours; a colour
    /// the settings give wins.
    #[test]
    fn a_quiet_state_on_the_current_tab_takes_the_accent_colour() {
        let accent = theme::style(Role::PillBadgeCurrent).bg;
        let symbol_style = |mark: &str,
                            look: theme::StateStyle,
                            setting: &theme::StateLook,
                            current: bool,
                            prepare: &dyn Fn(&mut Tab)| {
            let mut state = *setting;
            state.style = look;
            theme::set_state_look(state);
            let mut i = info();
            prepare(&mut i.tabs[usize::from(current)]);
            let (cells, _, spans) = build_with_tabs(120, &i);
            let span = &spans[usize::from(current)];
            let shown = shown_text(&cells, span);
            let from = shown.find(mark).unwrap();
            let col = usize::from(span.from) + shown[..from].chars().count();
            cells[col].style
        };
        let running = |t: &mut Tab| t.command = Some(CommandMark::Running);
        let approval = |t: &mut Tab| t.notice = Some(Class::Approval);
        let none = theme::StateLook::default();
        for look in [theme::StateStyle::Dark, theme::StateStyle::Plain] {
            let off = symbol_style(theme::running_symbol(), look, &none, false, &running);
            let on = symbol_style(theme::running_symbol(), look, &none, true, &running);
            // The symbol is the accent on the current tab (made readable on its block), and the
            // palette's running colour on the others.
            assert_ne!(on.fg, off.fg, "{look:?}");
            let block = theme::style(if look == theme::StateStyle::Plain {
                Role::PillNameCurrent
            } else {
                Role::PillBadge
            });
            if look == theme::StateStyle::Dark {
                assert_eq!(on.fg, accent, "{look:?}");
                assert_ne!(off.fg, accent, "{look:?}");
            } else {
                assert_eq!(
                    on.fg,
                    theme::cell_style_on(theme::Tint::Running, block, true).fg
                );
            }
            // What asks for the user keeps its colour on the current tab.
            let a_off = symbol_style(
                theme::notice_symbol(Class::Approval),
                look,
                &none,
                false,
                &approval,
            );
            let a_on = symbol_style(
                theme::notice_symbol(Class::Approval),
                look,
                &none,
                true,
                &approval,
            );
            if look == theme::StateStyle::Dark {
                assert_eq!(a_on.fg, a_off.fg, "{look:?}");
            } else {
                // On its own block, the colour of the class and no other.
                let own = theme::cell_style(theme::Tint::Class(Class::Approval), block);
                assert_eq!(a_on.fg, own.fg, "{look:?}");
            }
        }
        // A colour the settings give wins, current tab or not.
        let mut set = theme::StateLook::default();
        set.set(
            theme::StateName::Running,
            theme::StateSetting {
                symbol: None,
                color: theme::parse_color("#ff8800"),
            },
        );
        let own = theme::parse_color("#ff8800").unwrap();
        assert_eq!(
            symbol_style(
                theme::running_symbol(),
                theme::StateStyle::Dark,
                &set,
                true,
                &running
            )
            .fg,
            own
        );
        assert_eq!(
            symbol_style(
                theme::running_symbol(),
                theme::StateStyle::Dark,
                &set,
                false,
                &running
            )
            .fg,
            own
        );
        theme::set_state_look(theme::StateLook::default());
    }

    /// The current tab with a state, for looking at: the symbol in the accent on the dark badge
    /// (what is built) and, to compare, the badge in the accent with the dark symbol.
    #[test]
    fn sample_current_tab_with_a_running_state() {
        let Ok(dir) = std::env::var("ARCADE_SAMPLE_DIR") else {
            return;
        };
        let accent_badge = theme::style(Role::PillBadgeCurrent);
        for (name, style) in [("pill", TabStyle::Pill), ("flat", TabStyle::Flat)] {
            for (word, look) in [
                ("plain", theme::StateStyle::Plain),
                ("dark", theme::StateStyle::Dark),
                ("filled", theme::StateStyle::Filled),
                ("number", theme::StateStyle::Number),
            ] {
                let mut state = theme::StateLook::default();
                state.style = look;
                theme::set_state_look(state);
                let mut i = info();
                i.style = style;
                i.tabs[1].command = Some(CommandMark::Running);
                let (cells, _) = build(110, &i);
                let write = |tag: &str, cells: &[Cell]| {
                    let mut bytes = Vec::new();
                    crate::render::push_row(&mut bytes, 0, cells, &[]);
                    bytes.extend_from_slice(b"\x1b[0m\n");
                    std::fs::write(
                        format!("{dir}/current-running-{tag}-{word}-{name}-110.bin"),
                        bytes,
                    )
                    .unwrap();
                };
                write("symbol-in-accent", &cells);
                // The other reading: the badge itself in the accent colours.
                let dark = theme::state_badge(
                    if look == theme::StateStyle::Plain {
                        theme::StateStyle::Dark
                    } else {
                        look
                    },
                    theme::Tint::Running,
                );
                let mut badge_accent = cells.clone();
                for c in &mut badge_accent {
                    if (c.style.bg, c.style.fg) == (dark.bg, accent_badge.bg)
                        || (c.style.bg, c.style.fg) == (accent_badge.bg, dark.fg)
                    {
                        c.style = accent_badge;
                    }
                }
                write("badge-in-accent", &badge_accent);
                let at = cells
                    .iter()
                    .position(|c| c.text == theme::running_symbol())
                    .map(|n| (cells[n].style.fg, cells[n].style.bg));
                eprintln!("{word} {name}: symbol (fg, bg) on the current tab = {at:?}");
            }
        }
        theme::set_state_look(theme::StateLook::default());
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
                    command: None,
                });
                let (cells, _) = build(60, &i);
                let mut bytes = Vec::new();
                crate::render::push_row(&mut bytes, 0, &cells, &[]);
                bytes.extend_from_slice(b"\x1b[0m\n");
                std::fs::write(format!("{dir}/bar-{name}-{place}.bin"), bytes).unwrap();
            }
            // Six windows: the current one quiet, then one that asks for you, one running, one
            // finished, one failed and a quiet one, in each look of the states, and once more in
            // `plain` with a symbol and colours of the author's choice, at 110 columns.
            {
                let tab = |number: usize, name: &str| Tab {
                    number,
                    text: vec![TabPiece::plain(name)],
                    current: number == 1,
                    zoomed: false,
                    notice: None,
                    progress: None,
                    command: None,
                };
                let mut i = info();
                i.style = style;
                i.tabs = vec![
                    tab(1, "api"),
                    tab(2, "web"),
                    tab(3, "build"),
                    tab(4, "tests"),
                    tab(5, "deploy"),
                    tab(6, "notes"),
                ];
                i.tabs[1].notice = Some(Class::Approval);
                i.tabs[2].command = Some(CommandMark::Running);
                i.tabs[3].command = Some(CommandMark::FinishedWell);
                i.tabs[4].progress = Some(arcade_term::Progress { state: 2, value: 0 });
                let mut custom = theme::StateLook::default();
                custom.set(
                    theme::StateName::Running,
                    theme::StateSetting {
                        symbol: Some(theme::intern("\u{f04b}")),
                        color: None,
                    },
                );
                custom.set(
                    theme::StateName::Finished,
                    theme::StateSetting {
                        symbol: None,
                        color: theme::parse_color("#8bd5ca"),
                    },
                );
                let mut looks = Vec::new();
                for (word, look) in [
                    ("plain", theme::StateStyle::Plain),
                    ("dark", theme::StateStyle::Dark),
                    ("filled", theme::StateStyle::Filled),
                    ("number", theme::StateStyle::Number),
                ] {
                    let mut setting = theme::StateLook::default();
                    setting.style = look;
                    looks.push((word, setting));
                }
                let mut custom_plain = custom;
                custom_plain.style = theme::StateStyle::Plain;
                looks.push(("custom", custom_plain));
                for (word, setting) in looks {
                    theme::set_state_look(setting);
                    let (cells, _) = build(110, &i);
                    let mut bytes = Vec::new();
                    crate::render::push_row(&mut bytes, 0, &cells, &[]);
                    bytes.extend_from_slice(b"\x1b[0m\n");
                    std::fs::write(format!("{dir}/tabs-states-{word}-{name}-110.bin"), bytes)
                        .unwrap();
                    std::fs::write(
                        format!("{dir}/tabs-states-{word}-{name}-110.txt"),
                        format!("{}\n", text(&cells)),
                    )
                    .unwrap();
                }
                theme::set_state_look(theme::StateLook::default());
            }
            // A bar with the counter at the right, at 110 columns: one other session holds a
            // notification, and two sessions hold some.
            for (file, notices, sessions) in [
                ("one", [0, 0, 0, 1], vec!["hahaa"]),
                ("two", [1, 0, 0, 2], vec!["hahaa", "web"]),
            ] {
                let mut i = info();
                i.style = style;
                i.notices = notices;
                i.notice_sessions = sessions.into_iter().map(String::from).collect();
                let (cells, _) = build(110, &i);
                let mut bytes = Vec::new();
                crate::render::push_row(&mut bytes, 0, &cells, &[]);
                bytes.extend_from_slice(b"\x1b[0m\n");
                std::fs::write(format!("{dir}/counters-{file}-{name}-110.bin"), bytes).unwrap();
                std::fs::write(
                    format!("{dir}/counters-{file}-{name}-110.txt"),
                    format!("{}\n", text(&cells)),
                )
                .unwrap();
            }
            // The top row of a window of two panes whose left pane wears the mark of an info
            // notification in its corner (the cell the frame draws with `notice_style`).
            {
                let plain = arcade_term::Style::default();
                let mut row = text_cells("$ ls", plain);
                row = fit(row, 54, plain);
                row.extend(text_cells(
                    theme::notice_symbol(Class::Info),
                    theme::notice_style(Class::Info),
                ));
                row.extend(text_cells("\u{2502}", theme::style(Role::Divider)));
                row.extend(text_cells("$ ", plain));
                let row = fit(row, 110, plain);
                let mut bytes = Vec::new();
                crate::render::push_row(&mut bytes, 0, &row, &[]);
                bytes.extend_from_slice(b"\x1b[0m\n");
                std::fs::write(format!("{dir}/corner-mark-{name}-110.bin"), bytes).unwrap();
            }
            // Each line that replaces the tabs, at 110 columns.
            for (state, mut i) in lines_to_try(style) {
                i.style = style;
                let (cells, _) = build(110, &i);
                let mut bytes = Vec::new();
                crate::render::push_row(&mut bytes, 0, &cells, &[]);
                bytes.extend_from_slice(b"\x1b[0m\n");
                std::fs::write(format!("{dir}/line-{state}-{name}-110.bin"), bytes).unwrap();
                std::fs::write(
                    format!("{dir}/line-{state}-{name}-110.txt"),
                    format!("{}\n", text(&cells)),
                )
                .unwrap();
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
            // Five windows, one for each kind of what the status cell shows, at 110 columns.
            let mut i = four_windows();
            i.style = style;
            let tab = |number: usize, name: &str, notice, progress| Tab {
                number,
                text: vec![TabPiece::plain(name)],
                current: number == 3,
                zoomed: false,
                notice,
                progress,
                command: None,
            };
            let level = arcade_term::Progress {
                state: 1,
                value: 40,
            };
            let busy = arcade_term::Progress { state: 3, value: 0 };
            i.tabs = vec![
                tab(1, "asks-for-you", Some(Class::Input), None),
                tab(2, "tests-at-40", None, Some(level)),
                tab(3, "agent-working", None, Some(busy)),
                tab(4, "finished", Some(Class::Info), None),
                tab(5, "quiet", None, None),
            ];
            let (cells, _) = build(110, &i);
            let mut bytes = Vec::new();
            crate::render::push_row(&mut bytes, 0, &cells, &[]);
            bytes.extend_from_slice(b"\x1b[0m\n");
            std::fs::write(format!("{dir}/status-cell-{name}-110.bin"), bytes).unwrap();
            let mut i = four_windows();
            i.style = style;
            i.tabs[0].command = Some(CommandMark::FinishedWithError);
            i.tabs[1].command = Some(CommandMark::FinishedWell);
            i.tabs[3].command = Some(CommandMark::Running);
            i.tabs[3].notice = None;
            let (command_cells, _) = build(110, &i);
            let mut command_bytes = Vec::new();
            crate::render::push_row(&mut command_bytes, 0, &command_cells, &[]);
            command_bytes.extend_from_slice(b"\x1b[0m\n");
            std::fs::write(
                format!("{dir}/command-states-{name}-110.bin"),
                command_bytes,
            )
            .unwrap();
            std::fs::write(
                format!("{dir}/command-states-{name}-110.txt"),
                format!("{}\n", text(&command_cells)),
            )
            .unwrap();
            std::fs::write(
                format!("{dir}/status-cell-{name}-110.txt"),
                format!("{}\n", text(&cells)),
            )
            .unwrap();
        }
    }
}
