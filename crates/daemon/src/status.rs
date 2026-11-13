//! What the status bar says: the session's name, one tab per window, and the words that tell the
//! user which mode the keyboard is in. Each part is built on its own and put together here.

use arcade_term::Cell;

use crate::compose::{blank, fit, text_cells};
use crate::theme::{self, Role, Symbol};

pub struct Tab {
    /// The number shown, which is the window's place in the bar plus the base index.
    pub number: usize,
    pub name: String,
    pub current: bool,
    pub zoomed: bool,
}

/// A line the user types into.
pub struct Prompt {
    pub label: String,
    pub text: String,
}

pub struct StatusInfo {
    pub session: String,
    pub tabs: Vec<Tab>,
    /// Waiting for the key after the prefix key.
    pub prefix: bool,
    pub locked: bool,
    /// The focused pane is in copy mode.
    pub copy: bool,
    /// A line shown in place of the bar until the next key.
    pub message: Option<String>,
    pub prompt: Option<Prompt>,
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
        cells.extend(text_cells(
            &format!(" {} {}{zoom} ", tab.number, tab.name),
            theme::style(role),
        ));
    }
    let mut words: Vec<&str> = Vec::new();
    if info.prefix {
        words.push("PREFIX");
    }
    if info.locked {
        words.push("LOCK");
    }
    if info.copy {
        words.push("COPY");
    }
    let right: Vec<Cell> = words
        .iter()
        .flat_map(|w| text_cells(&format!(" {w} "), theme::style(Role::Indicator)))
        .collect();
    if cells.len() + right.len() < width {
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
                },
                Tab {
                    number: 2,
                    name: "vim".into(),
                    current: true,
                    zoomed: true,
                },
            ],
            prefix: false,
            locked: false,
            copy: false,
            message: None,
            prompt: None,
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
    fn mode_words_are_at_the_right_end() {
        let mut i = info();
        i.prefix = true;
        i.locked = true;
        i.copy = true;
        let line = text(&build(60, &i).0);
        assert!(line.trim_end().ends_with("PREFIX  LOCK  COPY"), "{line:?}");
        let quiet = text(&build(60, &info()).0);
        assert!(!quiet.contains("PREFIX") && !quiet.contains("LOCK") && !quiet.contains("COPY"));
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
    fn a_narrow_bar_is_cut_not_overflowed() {
        let (cells, _) = build(10, &info());
        assert_eq!(cells.len(), 10);
    }
}
