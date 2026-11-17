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
    /// The tab of a window that is not the current one.
    Tab,
    /// The tab of the current window.
    TabCurrent,
    /// The number in front of the name in the tab of a window that is not the current one, and in
    /// the tab of the current window.
    TabNumber,
    TabNumberCurrent,
    /// The program's own title in a tab, fainter than the name.
    /// The words at the right end of the status bar (`PREFIX`, `LOCK`, `COPY`).
    Indicator,
    /// A line that replaces the tabs for a moment: news or a result, an error, a question.
    MessageInfo,
    MessageError,
    MessageQuestion,
    /// The label of a line the user types into.
    PromptLabel,
    /// The text of a message, and what was typed in a prompt.
    PromptText,
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
    /// The tabs and the session label of the bar (both styles share these colours): the name on
    /// a surface, the number as a lighter badge; the current tab's badge is the accent and its
    /// name is brighter. The session label has a green badge and the name on a surface.
    PillName,
    PillNameCurrent,
    PillQuiet,
    PillQuietCurrent,
    PillBadge,
    PillBadgeCurrent,
    SessionBadge,
    SessionName,
}

/// The palette of the bar's tabs (Catppuccin Frappe, as the author's tmux bar has it).
mod palette {
    use arcade_term::Color;
    pub const MANTLE: Color = Color::Rgb(0x29, 0x2c, 0x3c);
    pub const CRUST: Color = Color::Rgb(0x23, 0x26, 0x34);
    pub const SURFACE0: Color = Color::Rgb(0x41, 0x45, 0x59);
    pub const SURFACE1: Color = Color::Rgb(0x51, 0x57, 0x6d);
    pub const OVERLAY2: Color = Color::Rgb(0x94, 0x9c, 0xbb);
    pub const TEXT: Color = Color::Rgb(0xc6, 0xd0, 0xf5);
    pub const SUBTEXT0: Color = Color::Rgb(0xb5, 0xbf, 0xe2);
    pub const MAUVE: Color = Color::Rgb(0xca, 0x9e, 0xe6);
    pub const GREEN: Color = Color::Rgb(0xa6, 0xd1, 0x89);
    pub const RED: Color = Color::Rgb(0xe7, 0x82, 0x84);
    pub const YELLOW: Color = Color::Rgb(0xe5, 0xc8, 0x90);
    pub const PINK: Color = Color::Rgb(0xf4, 0xb8, 0xe4);
    pub const TEAL: Color = Color::Rgb(0x81, 0xc8, 0xbe);
    pub const BLUE: Color = Color::Rgb(0x8c, 0xaa, 0xee);
}

fn on(fg: Color, bg: Color) -> Style {
    Style {
        fg,
        bg,
        ..Style::default()
    }
}

/// The round (or plain) end of a block that stands on the bar: the colour of the block as text on
/// the bar's background.
pub fn cap_style(block: Style) -> Style {
    Style {
        fg: block.bg,
        bg: style(Role::StatusBar).bg,
        ..Style::default()
    }
}

/// The contrast ratio (1 to 21) of the text of a style on its background, for the colours that
/// are given as red, green and blue.
fn contrast(style: Style) -> f64 {
    fn luminance(c: Color) -> f64 {
        let Color::Rgb(r, g, b) = c else {
            panic!("not an RGB colour: {c:?}")
        };
        let lin = |v: u8| {
            let v = f64::from(v) / 255.0;
            if v <= 0.03928 {
                v / 12.92
            } else {
                ((v + 0.055) / 1.055).powf(2.4)
            }
        };
        0.2126 * lin(r) + 0.7152 * lin(g) + 0.0722 * lin(b)
    }
    let (a, b) = (luminance(style.fg), luminance(style.bg));
    (a.max(b) + 0.05) / (a.min(b) + 0.05)
}

/// A colour moved toward white, a little at a time, until it reads on `background` at a contrast
/// of at least 4.5; one that already does is left alone. Both must be given as red, green and blue.
fn readable_on(colour: Color, background: Color) -> Color {
    let Color::Rgb(r, g, b) = colour else {
        return colour;
    };
    for step in 0..=100u32 {
        let mix = |v: u8| (u32::from(v) + (255 - u32::from(v)) * step / 100) as u8;
        let candidate = Color::Rgb(mix(r), mix(g), mix(b));
        let probe = Style {
            fg: candidate,
            bg: background,
            ..Style::default()
        };
        if contrast(probe) >= 4.5 {
            return candidate;
        }
    }
    Color::Rgb(255, 255, 255)
}

fn fg(color: u8) -> Style {
    Style {
        fg: Color::Indexed(color),
        ..Style::default()
    }
}

pub fn style(role: Role) -> Style {
    match role {
        Role::StatusBar => on(palette::SUBTEXT0, palette::MANTLE),
        Role::PillName => on(palette::TEXT, palette::SURFACE0),
        Role::PillNameCurrent => Style {
            bold: true,
            ..on(palette::TEXT, palette::SURFACE1)
        },
        Role::PillQuiet => on(palette::SUBTEXT0, palette::SURFACE0),
        Role::PillQuietCurrent => on(palette::TEXT, palette::SURFACE1),
        Role::PillBadge => Style {
            bold: true,
            ..on(palette::CRUST, palette::OVERLAY2)
        },
        Role::PillBadgeCurrent => Style {
            bold: true,
            ..on(palette::CRUST, palette::MAUVE)
        },
        Role::SessionBadge => Style {
            bold: true,
            ..on(palette::CRUST, palette::GREEN)
        },
        Role::SessionName => on(palette::TEXT, palette::SURFACE0),
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
        Role::Indicator => Style {
            fg: Color::Indexed(0),
            bg: Color::Indexed(3),
            bold: true,
            ..Style::default()
        },
        // A message is a small block of the accent colour (the dark of the bar's text on a
        // pastel: green for news, red for an error, yellow for a question, as the mode words)
        // and its text on the surface of a tab's name; each reads at a contrast of 4.5 or more
        // (a test checks it). What is typed in a prompt is on the same surface.
        Role::MessageInfo => Style {
            bold: true,
            ..on(palette::CRUST, palette::GREEN)
        },
        Role::MessageError => Style {
            bold: true,
            ..on(palette::CRUST, palette::RED)
        },
        Role::MessageQuestion | Role::PromptLabel => Style {
            bold: true,
            ..on(palette::CRUST, palette::YELLOW)
        },
        Role::PromptText => on(palette::TEXT, palette::SURFACE0),
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
    }
}

/// How the mark of a class of notification looks wherever it is drawn outside a tab (the counter
/// at the right of the bar, the corner of a pane, the lists): the symbol in the colour of its
/// class on the surface of a tab's name, as the status cell of a tab draws it, never a solid
/// block of the class colour.
pub fn notice_style(class: crate::notify::Class) -> Style {
    cell_style(Tint::Class(class), style(Role::PillName))
}

/// The mark of a class: one cell wide wherever it is drawn (tab, count, pane, lists), a shape of
/// its own so that colour is not all that tells the classes apart, and a shape that text does
/// not use, so that a line of a list can be searched for it.
pub fn notice_symbol(class: crate::notify::Class) -> &'static str {
    match class {
        crate::notify::Class::Approval => "▲",
        crate::notify::Class::Input => "◆",
        crate::notify::Class::Error => "■",
        crate::notify::Class::Info => "●",
    }
}

/// What colours the one status cell of a tab: a class of notification, or a kind of progress
/// report.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Tint {
    Class(crate::notify::Class),
    /// A progress report that failed.
    ProgressError,
    /// A progress report with a value, or with an unknown amount.
    Progress,
    /// A progress report that is paused: the same block, dimmer.
    Paused,
    /// A command that ended well while nobody looked.
    Finished,
}

/// The symbol of the status cell for a progress report that failed. One cell wide, a shape of its
/// own (the notification marks are filled shapes).
pub const PROGRESS_ERROR_SYMBOL: &str = "✗";

/// The symbol of the status cell for a progress report with no amount: static, one cell wide.
pub const BUSY_SYMBOL: &str = "⟳";

/// The symbol of the status cell for a command that is running (by the marks of the shell
/// prompt): a small mark, not the busy symbol of a progress report with no amount.
pub const RUNNING_SYMBOL: &str = "▸";

/// The eight level blocks, from a sliver to the full cell.
const LEVEL_BLOCKS: [&str; 8] = ["▁", "▂", "▃", "▄", "▅", "▆", "▇", "█"];

/// The level block for a value of 0 to 100: 0 to 12 the lowest, 88 to 100 the full block.
pub fn level_block(value: u8) -> &'static str {
    LEVEL_BLOCKS[(usize::from(value.min(100)) * 8 / 100).min(7)]
}

/// The status cell's symbol in a colour on the block it stands on, as the marks of the classes:
/// the palette's colour, made lighter only as far as it takes to read at a contrast of 4.5.
pub fn cell_style(tint: Tint, block: Style) -> Style {
    let colour = match tint {
        Tint::Class(crate::notify::Class::Approval) => palette::RED,
        Tint::Class(crate::notify::Class::Input) => palette::YELLOW,
        Tint::Class(crate::notify::Class::Error) => palette::PINK,
        Tint::Class(crate::notify::Class::Info) => palette::TEAL,
        Tint::ProgressError => palette::RED,
        Tint::Progress => palette::BLUE,
        Tint::Paused => palette::OVERLAY2,
        Tint::Finished => palette::GREEN,
    };
    Style {
        fg: readable_on(colour, block.bg),
        bg: block.bg,
        bold: true,
        ..Style::default()
    }
}

/// What a list says about a progress report, in words: `42%`, `error 30%`, `error`, `paused 55%`,
/// `working` (an unknown amount). An error or a pause at 100 shows 99.
pub fn progress_text(report: arcade_term::Progress) -> String {
    let two = report.value.min(99);
    match report.state {
        1 => format!("{}%", report.value.min(100)),
        2 if report.value == 0 => "error".to_string(),
        2 => format!("error {two}%"),
        3 => "working".to_string(),
        _ => format!("paused {two}%"),
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
    /// The round ends of a block on the bar (the powerline half circles, which need a font that
    /// has them) and the icon in front of the session's name.
    PillCapLeft,
    PillCapRight,
    SessionIcon,
    /// At the left (right) end of the bar: tabs are hidden before the first (after the last) one
    /// that shows.
    HiddenBefore,
    HiddenAfter,
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
    /// In the coloured block in front of a message: news, or an error.
    MessageInformation,
    MessageError,
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
        Symbol::PillCapLeft => "\u{e0b6}",
        Symbol::PillCapRight => "\u{e0b4}",
        Symbol::SessionIcon => "\u{e795} ",
        Symbol::HiddenBefore => "‹",
        Symbol::HiddenAfter => "›",
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
        Symbol::MessageInformation => "●",
        Symbol::MessageError => "✗",
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
    }

    #[test]
    fn every_notification_mark_is_one_cell_in_its_colour_on_a_surface_and_never_a_solid_block() {
        use crate::notify::Class;
        let surface = style(Role::PillName).bg;
        for class in Class::BY_URGENCY {
            let cells = crate::compose::text_cells(notice_symbol(class), Style::default());
            assert_eq!(cells.len(), 1, "{class:?}: {:?}", notice_symbol(class));
            let mark = notice_style(class);
            assert_ne!(mark.fg, mark.bg, "{class:?}");
            // The surface of a tab's name, not a colour of a class.
            assert_eq!(mark.bg, surface, "{class:?}");
            assert!(contrast(mark) >= 4.5, "{class:?}");
            // The same colour as the tab's own status cell.
            assert_eq!(
                mark.fg,
                cell_style(Tint::Class(class), style(Role::PillName)).fg
            );
        }
    }

    #[test]
    fn the_mark_on_a_block_reads_at_a_contrast_of_four_and_a_half_on_every_block_of_the_bar() {
        use crate::notify::Class;
        for class in Class::BY_URGENCY {
            for role in [
                Role::StatusBar,
                Role::PillName,
                Role::PillNameCurrent,
                Role::PillBadge,
            ] {
                let on_block = cell_style(Tint::Class(class), style(role));
                if role == Role::PillBadge {
                    continue;
                }
                assert_eq!(on_block.bg, style(role).bg);
                let ratio = contrast(on_block);
                assert!(ratio >= 4.5, "{class:?} on {role:?}: {ratio:.2}");
            }
        }
    }

    #[test]
    fn no_role_of_the_bar_puts_the_same_colour_on_text_and_background() {
        for role in [
            Role::StatusBar,
            Role::PillName,
            Role::PillNameCurrent,
            Role::PillQuiet,
            Role::PillQuietCurrent,
            Role::PillBadge,
            Role::PillBadgeCurrent,
            Role::SessionBadge,
            Role::SessionName,
            Role::Tab,
            Role::TabCurrent,
            Role::TabNumber,
            Role::TabNumberCurrent,
            Role::Indicator,
            Role::MessageInfo,
            Role::MessageError,
            Role::MessageQuestion,
            Role::PromptLabel,
            Role::PromptText,
            Role::PickerSelected,
        ] {
            let s = style(role);
            assert_ne!(s.fg, s.bg, "{role:?}");
        }
    }

    /// Every text of the tab bar is readable on its background: a contrast ratio of 4.5 or more.
    #[test]
    fn the_text_of_the_tab_bar_has_a_contrast_ratio_of_at_least_4_5() {
        for role in [
            Role::StatusBar,
            Role::PillName,
            Role::PillNameCurrent,
            Role::PillQuiet,
            Role::PillQuietCurrent,
            Role::PillBadge,
            Role::PillBadgeCurrent,
            Role::SessionBadge,
            Role::SessionName,
            Role::MessageInfo,
            Role::MessageError,
            Role::MessageQuestion,
            Role::PromptLabel,
            Role::PromptText,
        ] {
            let ratio = contrast(style(role));
            assert!(ratio >= 4.5, "{role:?}: {ratio:.2}");
        }
    }

    /// Prints the ratios (run with `--nocapture`) for the report.
    #[test]
    fn the_contrast_ratios_are_printed_for_the_report() {
        for role in [
            Role::StatusBar,
            Role::PillName,
            Role::PillNameCurrent,
            Role::PillQuiet,
            Role::PillQuietCurrent,
            Role::PillBadge,
            Role::PillBadgeCurrent,
            Role::SessionBadge,
            Role::SessionName,
        ] {
            println!("contrast {role:?}: {:.2}", contrast(style(role)));
        }
    }
}
