//! What the status bar says: the session's name, one tab per window, and the words that tell the
//! user which mode the keyboard is in. Each part is built on its own and put together here.

use arcade_term::Cell;

use crate::compose::{blank, fit, text_cells};
use crate::config::NumberPosition;
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
    let mut cells = text_cells(
        &format!(" {} ", info.session),
        theme::style(Role::StatusSession),
    );
    cells.push(blank(bar));
    let mut spans = Vec::new();
    for (index, tab) in info.tabs.iter().enumerate() {
        let tab_from = cells.len();
        let zoom = if tab.zoomed {
            format!(" {}", theme::symbol(Symbol::Zoomed))
        } else {
            String::new()
        };
        let role = if tab.current {
            Role::TabCurrent
        } else {
            Role::Tab
        };
        let style = theme::style(role);
        let number_style = theme::style(if tab.current {
            Role::TabNumberCurrent
        } else {
            Role::TabNumber
        });
        let quiet_style = theme::style(if tab.current {
            Role::TabTitleCurrent
        } else {
            Role::TabTitle
        });
        let number = tab.number.to_string();
        cells.extend(text_cells(" ", style));
        if info.number == NumberPosition::Left {
            cells.extend(text_cells(&number, number_style));
            cells.extend(text_cells(" ", style));
        }
        for piece in &tab.text {
            let piece_style = if piece.quiet { quiet_style } else { style };
            cells.extend(text_cells(&piece.text, piece_style));
        }
        cells.extend(text_cells(&zoom, style));
        if info.number == NumberPosition::Right {
            cells.extend(text_cells(" ", style));
            cells.extend(text_cells(&number, number_style));
        }
        if let Some(class) = tab.notice {
            cells.extend(text_cells(" ", style));
            cells.extend(text_cells(
                theme::notice_symbol(class),
                theme::notice_style(class),
            ));
        }
        if let Some(report) = tab.progress {
            cells.extend(text_cells(" ", style));
            let mark_style = if report.state == 2 {
                theme::notice_style(Class::Error)
            } else {
                number_style
            };
            cells.extend(text_cells(&theme::progress_mark(report), mark_style));
        }
        cells.extend(text_cells(" ", style));
        spans.push((index, tab_from, cells.len()));
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
            number: NumberPosition::Left,
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
        assert!(line.contains(" 1 sh "), "{line}");
        assert!(line.contains(" 2 vim [Z] "), "{line}");
        let at = |needle: &str| cells[line[..line.find(needle).unwrap()].chars().count()].style;
        assert_eq!(at("vim"), theme::style(Role::TabCurrent));
        assert_eq!(at("sh"), theme::style(Role::Tab));
        assert_ne!(at("vim"), at("sh"));
        // The number of a tab is drawn apart from its name: on the same background, except in the
        // current tab, where it is the colours of the tab swapped.
        assert_eq!(at("2 vim"), theme::style(Role::TabNumberCurrent));
        assert_eq!(at("1 sh"), theme::style(Role::TabNumber));
        for (number, name) in [("2 vim", "vim"), ("1 sh", "sh")] {
            assert_ne!(at(number).fg, at(name).fg, "{number}");
            if name == "sh" {
                assert_eq!(at(number).bg, at(name).bg, "{number}");
            } else {
                assert_eq!(at(number).bg, at(name).fg, "{number}");
                assert_eq!(at(number).fg, at(name).bg, "{number}");
            }
        }
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
            (col(" 1 sh "), col(" 1 sh ") + 6)
        );
        assert_eq!(spans[1].index, 1);
        assert_eq!(spans[1].from, spans[0].to);
        assert_eq!(spans[1].to, spans[1].from + " 2 vim [Z] ".len() as u16);
        // Cut short: the tab that is cut in two covers what shows, the one cut away is gone.
        let (_, _, narrow) = build_with_tabs(18, &info());
        assert_eq!(narrow.len(), 2);
        assert_eq!(narrow[1].to, 18);
        let (_, _, narrower) = build_with_tabs(12, &info());
        assert_eq!(narrower.len(), 1);
        assert!(narrower[0].to <= 12);
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
        assert!(line.contains(" 1 sh  42% "), "{line}");
        assert!(line.contains(" 2 vim [Z]  err "), "{line}");
        let at = |needle: &str| cells[line[..line.find(needle).unwrap()].chars().count()].style;
        assert_eq!(at("42%"), theme::style(Role::TabNumber));
        assert_eq!(at("err"), theme::notice_style(Class::Error));
        // The mark belongs to its tab: a click on it is a click on the tab.
        assert_eq!(spans[0].to as usize, line.find("42%").unwrap() + 4);
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
        assert!(line.contains(&format!("1 sh {approval} ")), "{line}");
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
}
