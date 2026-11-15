//! What the status bar says: the session's name, one tab per window, and the words that tell the
//! user which mode the keyboard is in. Each part is built on its own and put together here.

use arcade_term::Cell;

use crate::compose::{blank, fit, text_cells};
use crate::notify::Class;
use crate::theme::{self, Role, Symbol};

pub struct Tab {
    /// The number shown, which is the window's place in the bar plus the base index.
    pub number: usize,
    pub name: String,
    pub current: bool,
    pub zoomed: bool,
    /// The most urgent notification among the window's panes.
    pub notice: Option<Class>,
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
    pub tabs: Vec<Tab>,
    /// Waiting for the key after the prefix key.
    pub prefix: bool,
    pub locked: bool,
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
pub fn build(cols: u16, info: &StatusInfo) -> (Vec<Cell>, Option<u16>) {
    let width = usize::from(cols);
    let bar = theme::style(Role::StatusBar);
    if let Some(prompt) = &info.prompt {
        let style = theme::style(Role::Prompt);
        let mut cells = text_cells(&format!("{}{}", prompt.label, prompt.text), style);
        let cursor = cells.len().min(width.saturating_sub(1));
        cells = fit(cells, width, style);
        return (cells, Some(cursor as u16));
    }
    if let Some(hint) = &info.hint {
        let style = theme::style(Role::StatusBar);
        return (fit(text_cells(hint, style), width, style), None);
    }
    if let Some(message) = &info.message {
        let style = theme::style(Role::Message);
        return (
            fit(text_cells(&format!(" {message}"), style), width, style),
            None,
        );
    }
    let mut cells = text_cells(
        &format!(" {} ", info.session),
        theme::style(Role::StatusSession),
    );
    cells.push(blank(bar));
    for tab in &info.tabs {
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
        cells.extend(text_cells(
            &format!(" {} {}{zoom}", tab.number, tab.name),
            style,
        ));
        if let Some(class) = tab.notice {
            cells.extend(text_cells(" ", style));
            cells.extend(text_cells(
                theme::notice_symbol(class),
                theme::notice_style(class),
            ));
        }
        cells.extend(text_cells(" ", style));
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
    if right.len() < width {
        cells = fit(cells, width - right.len(), bar);
        cells.extend(right);
    } else {
        cells = fit(cells, width, bar);
    }
    (cells, None)
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
            tabs: vec![
                Tab {
                    number: 1,
                    name: "sh".into(),
                    current: false,
                    zoomed: false,
                    notice: None,
                },
                Tab {
                    number: 2,
                    name: "vim".into(),
                    current: true,
                    zoomed: true,
                    notice: None,
                },
            ],
            prefix: false,
            locked: false,
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
        let at = |needle: &str| cells[line.find(needle).unwrap()].style;
        assert_eq!(at("2 vim"), theme::style(Role::TabCurrent));
        assert_eq!(at("1 sh"), theme::style(Role::Tab));
        assert_ne!(at("2 vim"), at("1 sh"));
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
    fn when_the_tabs_do_not_fit_they_give_way_to_the_counters() {
        let mut i = info();
        for n in 3..30 {
            i.tabs.push(Tab {
                number: n,
                name: "window".into(),
                current: false,
                zoomed: false,
                notice: None,
            });
        }
        i.notices = [10, 0, 0, 0];
        let line = text(&build(60, &i).0);
        assert!(line.trim_end().ends_with("[!]10"), "{line:?}");
    }

    #[test]
    fn a_tab_with_a_notice_shows_the_mark_of_its_class_and_the_counters_are_at_the_right() {
        let mut i = info();
        i.tabs[0].notice = Some(Class::Approval);
        i.notices = [2, 0, 1, 3];
        let (cells, _) = build(80, &i);
        let line = text(&cells);
        assert!(line.contains(" 1 sh [!] "), "{line}");
        assert!(line.trim_end().ends_with("[!]2 [x]1 [i]3"), "{line:?}");
        let at = |needle: &str| cells[line.find(needle).unwrap()].style;
        assert_eq!(at("[!]2"), theme::notice_style(Class::Approval));
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
        assert!(!quiet.contains("[!]") && !quiet.contains("[i]"));
    }

    #[test]
    fn a_narrow_bar_is_cut_not_overflowed() {
        let (cells, _) = build(10, &info());
        assert_eq!(cells.len(), 10);
    }
}
