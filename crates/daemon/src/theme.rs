//! Every color and every drawn symbol of Arcade's own interface lives here, and nowhere else.
//!
//! Colors are given by role. The default theme assigns each role one of the 16 colors of the host
//! terminal, so the host's own palette decides what they look like. (A test scans the other
//! source files and fails when one of them writes a color code or a line-drawing character.)

use arcade_term::{Color, Style};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Role {
    /// The status bar's background and its plain text.
    StatusBar,
    /// The session's name at the left end of the status bar.
    StatusSession,
    /// The tab of a window that is not the current one.
    Tab,
    /// The tab of the current window.
    TabCurrent,
    /// The number in front of the name in the tab of a window that is not the current one, and in
    /// the tab of the current window.
    TabNumber,
    TabNumberCurrent,
    /// The program's own title in a tab, fainter than the name.
    TabTitle,
    TabTitleCurrent,
    /// The words at the right end of the status bar (`PREFIX`, `LOCK`, `COPY`).
    Indicator,
    /// A line of text that replaces the status bar for a moment (an error, a result).
    Message,
    /// A line the user types into, in place of the status bar.
    Prompt,
    /// A divider between two panes.
    Divider,
    /// The part of a divider that touches the focused pane.
    DividerFocus,
    /// The floating list: its frame and the lines that are not chosen.
    Picker,
    /// The line of the floating list that is chosen.
    PickerSelected,
    /// The line that names a group of lines, and the column titles, in the floating list.
    PickerHeading,
    /// The cells of copy mode that are selected or found by a search.
    Selection,
    /// The status bar word that says a selection is active (`VISUAL`, `V-LINE`) or a jump is open.
    IndicatorSelect,
    /// The `[above/history]` mark on the first row of a pane in copy mode.
    ScrollPosition,
    /// The underline a hidden link (OSC 8) that is passed on gets, unless the program underlined
    /// the cell itself.
    LinkUnderline,
    /// The cells that match what was typed in a jump, and the one-letter label of a match.
    JumpMatch,
    JumpLabel,
    /// The mark of a notification, one role for each class.
    NoticeApproval,
    NoticeInput,
    NoticeError,
    NoticeInfo,
}

fn fg(color: u8) -> Style {
    Style {
        fg: Color::Indexed(color),
        ..Style::default()
    }
}

pub fn style(role: Role) -> Style {
    match role {
        Role::StatusBar => Style {
            fg: Color::Indexed(7),
            bg: Color::Indexed(8),
            ..Style::default()
        },
        Role::StatusSession => Style {
            fg: Color::Indexed(0),
            bg: Color::Indexed(2),
            bold: true,
            ..Style::default()
        },
        Role::Tab => style(Role::StatusBar),
        Role::TabCurrent => Style {
            fg: Color::Indexed(0),
            bg: Color::Indexed(4),
            bold: true,
            ..Style::default()
        },
        Role::TabNumber => Style {
            fg: Color::Indexed(3),
            bold: true,
            ..style(Role::Tab)
        },
        // The colours of the current tab swapped: blue on the dark of its text, so the number
        // stands out from the name without a colour the tab does not already use.
        Role::TabNumberCurrent => Style {
            fg: style(Role::TabCurrent).bg,
            bg: style(Role::TabCurrent).fg,
            ..style(Role::TabCurrent)
        },
        Role::TabTitle => Style {
            faint: true,
            ..style(Role::Tab)
        },
        Role::TabTitleCurrent => Style {
            faint: true,
            bold: false,
            ..style(Role::TabCurrent)
        },
        Role::Indicator => Style {
            fg: Color::Indexed(0),
            bg: Color::Indexed(3),
            bold: true,
            ..Style::default()
        },
        Role::Message => Style {
            fg: Color::Indexed(0),
            bg: Color::Indexed(3),
            ..Style::default()
        },
        Role::Prompt => Style {
            fg: Color::Indexed(15),
            bg: Color::Indexed(0),
            ..Style::default()
        },
        Role::Divider => fg(8),
        Role::DividerFocus => Style {
            bold: true,
            ..fg(4)
        },
        Role::Picker => Style {
            fg: Color::Indexed(7),
            bg: Color::Indexed(0),
            ..Style::default()
        },
        Role::PickerHeading => Style {
            fg: Color::Indexed(6),
            bg: Color::Indexed(0),
            bold: true,
            ..Style::default()
        },
        Role::PickerSelected => Style {
            fg: Color::Indexed(0),
            bg: Color::Indexed(4),
            bold: true,
            ..Style::default()
        },
        Role::IndicatorSelect => Style {
            fg: Color::Indexed(0),
            bg: Color::Indexed(5),
            bold: true,
            ..Style::default()
        },
        Role::ScrollPosition => Style {
            fg: Color::Indexed(0),
            bg: Color::Indexed(3),
            ..Style::default()
        },
        Role::LinkUnderline => Style {
            underline: arcade_term::Underline::Dotted,
            ..Style::default()
        },
        Role::JumpMatch => Style {
            fg: Color::Indexed(0),
            bg: Color::Indexed(3),
            ..Style::default()
        },
        Role::JumpLabel => Style {
            fg: Color::Indexed(15),
            bg: Color::Indexed(1),
            bold: true,
            ..Style::default()
        },
        Role::Selection => Style {
            fg: Color::Indexed(0),
            bg: Color::Indexed(6),
            ..Style::default()
        },
        Role::NoticeApproval => Style {
            fg: Color::Indexed(0),
            bg: Color::Indexed(1),
            bold: true,
            ..Style::default()
        },
        Role::NoticeInput => Style {
            fg: Color::Indexed(0),
            bg: Color::Indexed(3),
            bold: true,
            ..Style::default()
        },
        Role::NoticeError => Style {
            fg: Color::Indexed(0),
            bg: Color::Indexed(5),
            bold: true,
            ..Style::default()
        },
        Role::NoticeInfo => Style {
            fg: Color::Indexed(0),
            bg: Color::Indexed(6),
            ..Style::default()
        },
    }
}

/// How the mark of a class of notification looks: a colour and, because a colour alone is not
/// enough on a terminal with few of them, a symbol of its own.
pub fn notice_style(class: crate::notify::Class) -> Style {
    style(match class {
        crate::notify::Class::Approval => Role::NoticeApproval,
        crate::notify::Class::Input => Role::NoticeInput,
        crate::notify::Class::Error => Role::NoticeError,
        crate::notify::Class::Info => Role::NoticeInfo,
    })
}

pub fn notice_symbol(class: crate::notify::Class) -> &'static str {
    match class {
        crate::notify::Class::Approval => "[!]",
        crate::notify::Class::Input => "[?]",
        crate::notify::Class::Error => "[x]",
        crate::notify::Class::Info => "[i]",
    }
}

/// How many cells the progress mark of a tab takes whenever it shows, so that a tab changes
/// width only when a report appears or goes.
pub const PROGRESS_MARK_WIDTH: usize = 4;

/// What the tab of a window says about a progress report of one of its panes, always
/// [`PROGRESS_MARK_WIDTH`] cells: the percentage right-aligned (`  9%`, ` 42%`, `100%`); `x` and
/// two digits for an error (`x30%`, ` err` when it has no value); `p` and two digits for a
/// pause (`p55%`); ` ...` when the amount is unknown. An error or a pause at 100 shows 99.
pub fn progress_mark(report: arcade_term::Progress) -> String {
    let two = report.value.min(99);
    let mark = match report.state {
        1 => format!("{:>3}%", report.value.min(100)),
        2 if report.value == 0 => " err".to_string(),
        2 => format!("x{two:>2}%"),
        3 => " ...".to_string(),
        _ => format!("p{two:>2}%"),
    };
    debug_assert_eq!(mark.chars().count(), PROGRESS_MARK_WIDTH);
    mark
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Symbol {
    /// The cell of a divider that runs top to bottom.
    DividerVertical,
    /// The cell of a divider that runs left to right.
    DividerHorizontal,
    /// Where a divider meets another: the line goes on to the right, to the left, down, up, or
    /// every way.
    DividerJoinRight,
    DividerJoinLeft,
    DividerJoinDown,
    DividerJoinUp,
    DividerCross,
    /// Marks the tab of a window whose pane is zoomed.
    Zoomed,
    /// The corners and sides of the frame of the floating list.
    PickerTopLeft,
    PickerTopRight,
    PickerBottomLeft,
    PickerBottomRight,
    PickerHorizontal,
    PickerVertical,
    /// In front of the line of the floating list that is chosen, and of the others (blank).
    PickerSelectedMark,
    PickerOtherMark,
    /// After the line that stands for what is current (the session shown).
    PickerCurrentMark,
    /// In front of the text typed to filter the lines, and what the box says when none is left.
    PickerFilterMark,
    PickerNoMatch,
    /// Ends a line that was cut because it did not fit.
    Ellipsis,
    /// After a key of the key list that this terminal cannot send.
    KeyUnavailable,
    /// Between a mode word on the status bar and what it tells the user to do.
    HintSeparator,
    /// In front of a session or window of the tree that shows what is inside it, and of one that
    /// does not.
    TreeOpen,
    TreeClosed,
    /// In front of the pane the client is in, and of the others (blank).
    TreeHere,
    TreeOtherPane,
    /// After the line of the item that is lifted.
    TreeLifted,
    /// Between the names on the path that a line shows while the tree is filtered.
    TreePathSeparator,
}

pub fn symbol(symbol: Symbol) -> &'static str {
    match symbol {
        Symbol::DividerVertical => "│",
        Symbol::DividerHorizontal => "─",
        Symbol::DividerJoinRight => "├",
        Symbol::DividerJoinLeft => "┤",
        Symbol::DividerJoinDown => "┬",
        Symbol::DividerJoinUp => "┴",
        Symbol::DividerCross => "┼",
        Symbol::Zoomed => "[Z]",
        Symbol::PickerTopLeft => "┌",
        Symbol::PickerTopRight => "┐",
        Symbol::PickerBottomLeft => "└",
        Symbol::PickerBottomRight => "┘",
        Symbol::PickerHorizontal => "─",
        Symbol::PickerVertical => "│",
        Symbol::PickerSelectedMark => "> ",
        Symbol::PickerOtherMark => "  ",
        Symbol::PickerCurrentMark => " *",
        Symbol::PickerFilterMark => "/ ",
        Symbol::PickerNoMatch => "no match",
        Symbol::Ellipsis => "…",
        Symbol::KeyUnavailable => "!",
        Symbol::HintSeparator => " · ",
        Symbol::TreeOpen => "▾ ",
        Symbol::TreeClosed => "▸ ",
        Symbol::TreeHere => "● ",
        Symbol::TreeOtherPane => "  ",
        Symbol::TreeLifted => " [lifted]",
        Symbol::TreePathSeparator => " › ",
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_number_of_the_current_tab_uses_only_the_colours_of_its_tab_and_stays_readable() {
        let tab = style(Role::TabCurrent);
        let number = style(Role::TabNumberCurrent);
        // The two colours the tab already shows together, either way round.
        assert!(number.fg == tab.fg || number.fg == tab.bg);
        assert!(number.bg == tab.fg || number.bg == tab.bg);
        assert_ne!(number.fg, number.bg);
        assert_ne!(number.fg, tab.fg, "it must differ from the name next to it");
        // The name's own lighter text keeps the tab's colours too.
        let title = style(Role::TabTitleCurrent);
        assert_eq!((title.fg, title.bg), (tab.fg, tab.bg));
    }

    #[test]
    fn no_role_of_the_bar_puts_the_same_colour_on_text_and_background() {
        for role in [
            Role::StatusBar,
            Role::StatusSession,
            Role::Tab,
            Role::TabCurrent,
            Role::TabNumber,
            Role::TabNumberCurrent,
            Role::TabTitle,
            Role::TabTitleCurrent,
            Role::Indicator,
            Role::Message,
            Role::Prompt,
            Role::PickerSelected,
            Role::NoticeApproval,
            Role::NoticeInput,
            Role::NoticeError,
            Role::NoticeInfo,
        ] {
            let s = style(role);
            assert_ne!(s.fg, s.bg, "{role:?}");
        }
    }
}
