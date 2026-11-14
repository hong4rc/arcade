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
    /// The cells of copy mode that are selected or found by a search.
    Selection,
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
        Role::PickerSelected => Style {
            fg: Color::Indexed(0),
            bg: Color::Indexed(4),
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
        Symbol::HintSeparator => " · ",
        Symbol::TreeOpen => "▾ ",
        Symbol::TreeClosed => "▸ ",
        Symbol::TreeHere => "● ",
        Symbol::TreeOtherPane => "  ",
        Symbol::TreeLifted => " [lifted]",
        Symbol::TreePathSeparator => " › ",
    }
}
