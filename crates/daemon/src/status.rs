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
    pub index: usize,
    pub from: u16,
    pub to: u16,
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
    let total: usize = text.chars().map(crate::compose::char_width).sum();
    if total <= max {
        return text.to_string();
    }
    let room = max.saturating_sub(1);
    let mut out = String::new();
    let mut used = 0;
    for c in text.chars() {
        let w = crate::compose::char_width(c);
        if used + w > room {
            break;
        }
        used += w;
        out.push(c);
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
    for piece in &tab.text {
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
    if let Some(class) = tab.notice {
        text.extend(text_cells(" ", name));
        text.extend(text_cells(
            theme::notice_symbol(class),
            theme::notice_style(class),
        ));
    }
    if let Some(report) = tab.progress {
        text.extend(text_cells(" ", name));
        let style = if report.state == 2 {
            theme::notice_style(Class::Error)
        } else {
            name
        };
        text.extend(text_cells(&theme::progress_mark(report), style));
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
    let mut cells = session_label(info);
    cells.push(blank(bar));
    let mut spans = Vec::new();
    for (index, tab) in info.tabs.iter().enumerate() {
        let tab_from = cells.len();
        cells.extend(tab_cells(tab, info));
        spans.push((index, tab_from, cells.len()));
        // One cell of the bar's own background between two tabs, so that they read as items.
        cells.push(blank(bar));
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
    let current_span = spans
        .iter()
        .find(|(i, _, _)| info.tabs[*i].current)
        .copied();
    // The tab of the window you are in always shows: when it would be cut away, the tabs before
    // it give way from the left (the first one may show in part, and clicks follow what shows);
    // when that is not enough, the session's name gives way too.
    if let Some(&(_, tab_from, tab_to)) = current_span.as_ref().filter(|(_, _, to)| *to > room) {
        let first = spans.first().map_or(0, |s| s.1);
        let want = tab_to - room;
        let (from, shift) = if want > tab_from.saturating_sub(first) {
            (0, want.max(first).min(tab_from))
        } else {
            (first, want)
        };
        if shift > 0 {
            cells.drain(from..from + shift);
            let cut = from + shift;
            let moved = |p: usize| if p >= cut { p - shift } else { from };
            spans = spans
                .into_iter()
                .filter(|&(_, f, t)| moved(t) > moved(f))
                .map(|(i, f, t)| (i, moved(f), moved(t)))
                .collect();
        }
    }
    cells = fit(cells, room, bar);
    cells.extend(right.into_iter().take(width - room));
    // What was cut away cannot be clicked, and a tab cut in two is clickable where it shows.
    let spans = spans
        .into_iter()
        .filter(|(_, from, _)| *from < room)
        .map(|(index, from, to)| TabSpan {
            index,
            from: from as u16,
            to: to.min(room) as u16,
        })
        .collect();
    (cells, None, spans)
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
        // Cut short: the tab that is cut in two covers what shows, the one cut away is gone.
        let (_, _, narrow) = build_with_tabs(18, &info());
        assert_eq!(narrow.last().map(|t| (t.index, t.to)), Some((1, 18)));
        // The tab of the window you are in (the second) shows, and the first gives way to it.
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

    #[test]
    fn a_tab_with_a_progress_report_shows_its_mark_after_the_name() {
        use arcade_term::Progress;
        let mut i = info();
        i.tabs[0].progress = Some(Progress {
            state: 1,
            value: 42,
        });
        i.tabs[1].progress = Some(Progress { state: 2, value: 0 });
        let (cells, _, spans) = build_with_tabs(80, &i);
        let line = text(&cells);
        assert!(line.contains(" sh  42%  1 "), "{line}");
        assert!(line.contains(" vim [Z]  err  2 "), "{line}");
        let at = |needle: &str| cells[line[..line.find(needle).unwrap()].chars().count()].style;
        assert_eq!(at("42%"), theme::style(Role::PillName));
        assert_eq!(at("err"), theme::notice_style(Class::Error));
        // The mark belongs to its tab: a click on it is a click on the tab.
        assert_eq!(spans[0].to as usize, line.find("42%").unwrap() + 7);
        for (state, value, want) in [
            (1, 9, "  9%"),
            (1, 100, "100%"),
            (2, 30, "x30%"),
            (2, 100, "x99%"),
            (3, 0, " ..."),
            (4, 55, "p55%"),
        ] {
            assert_eq!(theme::progress_mark(Progress { state, value }), want);
        }
    }

    #[test]
    fn the_tab_has_the_same_width_for_every_value_and_state_so_the_tabs_to_its_right_stay() {
        use arcade_term::Progress;
        let spans_with = |report: Option<Progress>| {
            let mut i = info();
            i.tabs[0].progress = report;
            let (_, _, spans) = build_with_tabs(80, &i);
            spans
        };
        let shown = spans_with(Some(Progress { state: 1, value: 9 }));
        let width = |s: &[TabSpan]| s[0].to - s[0].from;
        for (state, value) in [
            (1, 0),
            (1, 42),
            (1, 100),
            (2, 0),
            (2, 7),
            (2, 100),
            (3, 0),
            (4, 5),
            (4, 100),
        ] {
            let other = spans_with(Some(Progress { state, value }));
            assert_eq!(width(&other), width(&shown), "{state} {value}");
            assert_eq!(other[1].from, shown[1].from, "{state} {value}");
            assert_eq!(other[1].to, shown[1].to, "{state} {value}");
        }
        // Only the appearing of a report moves the tabs, by the mark and the blank before it.
        let none = spans_with(None);
        assert_eq!(
            usize::from(shown[1].from - none[1].from),
            theme::PROGRESS_MARK_WIDTH + 1
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
                        assert!(
                            shown.contains("40%"),
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
            (NumberPosition::Left, " 1  sh "),
            (NumberPosition::Off, " sh "),
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
        }
    }
}
