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
    /// The floating list: the lines that are not chosen, and its filter line.
    Picker,
    /// The frame of the floating list.
    PickerFrame,
    /// The title in the top edge of the floating list.
    PickerTitle,
    /// The second thing a line says (its detail, "1 window") and the words of the key line.
    PickerQuiet,
    /// The keys of the key line, apart from the words after them.
    PickerKey,
    /// The number of a window in a line of the tree or of a list.
    PickerNumber,
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
    pub const SURFACE2: Color = Color::Rgb(0x62, 0x68, 0x80);
    /// Pure white is outside the palette. It is used only for the text on the current tab's
    /// block, by the author's choice: no palette colour reads at 4.5 on `SURFACE2`.
    pub const WHITE: Color = Color::Rgb(0xff, 0xff, 0xff);
    pub const OVERLAY0: Color = Color::Rgb(0x73, 0x79, 0x94);
    pub const OVERLAY2: Color = Color::Rgb(0x94, 0x9c, 0xbb);
    pub const TEXT: Color = Color::Rgb(0xc6, 0xd0, 0xf5);
    pub const SUBTEXT0: Color = Color::Rgb(0xb5, 0xbf, 0xe2);
    pub const LAVENDER: Color = Color::Rgb(0xba, 0xbb, 0xf1);
    pub const MAUVE: Color = Color::Rgb(0xca, 0x9e, 0xe6);
    pub const SAPPHIRE: Color = Color::Rgb(0x85, 0xc1, 0xdc);
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
/// The contrast ratio of a text colour on a background, for tests of other files.
#[cfg(test)]
pub fn contrast_of(fg: Color, bg: Color) -> f64 {
    contrast(Style {
        fg,
        bg,
        ..Style::default()
    })
}

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

pub fn style(role: Role) -> Style {
    match role {
        Role::StatusBar => on(palette::SUBTEXT0, palette::MANTLE),
        Role::PillName => on(palette::TEXT, palette::SURFACE0),
        // The current tab's name is a block of a lighter grey than the other tabs, with white
        // bold text; the number badge beside it carries the accent.
        Role::PillNameCurrent => Style {
            bold: true,
            ..on(palette::WHITE, palette::SURFACE2)
        },
        Role::PillQuiet => on(palette::SUBTEXT0, palette::SURFACE0),
        Role::PillQuietCurrent => on(palette::WHITE, palette::SURFACE2),
        Role::PillBadge => Style {
            bold: true,
            ..on(palette::CRUST, palette::OVERLAY2)
        },
        // The number of the current tab: the accent on the dark badge, so it does not merge with
        // the accent block of the name beside it.
        Role::PillBadgeCurrent => Style {
            bold: true,
            ..on(palette::MAUVE, palette::CRUST)
        },
        Role::SessionBadge => Style {
            bold: true,
            ..on(palette::CRUST, palette::GREEN)
        },
        Role::SessionName => on(palette::TEXT, palette::SURFACE0),
        Role::Indicator => Style {
            bold: true,
            ..on(palette::CRUST, palette::YELLOW)
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
        // The dividers stand on the terminal's own background, which is not known: the quiet
        // grey is visible on a dark one and on the bar's, and the focus colour is the accent.
        Role::Divider => Style {
            fg: palette::OVERLAY0,
            ..Style::default()
        },
        Role::DividerFocus => Style {
            fg: palette::MAUVE,
            bold: true,
            ..Style::default()
        },
        // The floating lists and the tree use the palette of the bar, not the 16 colours of the
        // terminal. The panel is darker than the bar and than a dark terminal, with a frame and
        // a title in an accent so that it stands apart from what is under it; the chosen line
        // is a grey block with white text and the state symbols stand on it in their own colours
        // (a test checks each text and each symbol).
        Role::Picker => on(palette::TEXT, palette::CRUST),
        Role::PickerFrame => on(palette::LAVENDER, palette::CRUST),
        Role::PickerQuiet => on(palette::OVERLAY2, palette::CRUST),
        Role::PickerTitle => Style {
            bold: true,
            ..on(palette::MAUVE, palette::CRUST)
        },
        Role::PickerKey => Style {
            bold: true,
            ..on(palette::MAUVE, palette::CRUST)
        },
        Role::PickerNumber => Style {
            bold: true,
            ..on(palette::BLUE, palette::CRUST)
        },
        Role::PickerHeading => Style {
            bold: true,
            ..on(palette::BLUE, palette::CRUST)
        },
        Role::PickerSelected => Style {
            bold: true,
            ..on(palette::WHITE, palette::SURFACE0)
        },
        Role::IndicatorSelect => Style {
            bold: true,
            ..on(palette::CRUST, palette::MAUVE)
        },
        // Copy mode: the position mark and the cells a jump matches look as the bar's question
        // does (dark on yellow), the label of a match is dark bold on red, and a selection is
        // dark on sapphire; the four are told apart from each other.
        Role::ScrollPosition => on(palette::CRUST, palette::YELLOW),
        Role::LinkUnderline => Style {
            underline: arcade_term::Underline::Dotted,
            ..Style::default()
        },
        Role::JumpMatch => on(palette::CRUST, palette::YELLOW),
        Role::JumpLabel => Style {
            bold: true,
            ..on(palette::CRUST, palette::RED)
        },
        Role::Selection => on(palette::CRUST, palette::SAPPHIRE),
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
    custom_symbol(StateName::of_class(class)).unwrap_or_else(|| default_notice_symbol(class))
}

fn default_notice_symbol(class: crate::notify::Class) -> &'static str {
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
    /// A progress report with a value.
    Progress,
    /// Something that runs: a progress report with no amount, or a command.
    Running,
    /// A progress report that is paused: the same block, dimmer.
    Paused,
    /// A command that ended well while nobody looked.
    Finished,
}

/// The symbol of the status cell for a progress report or a command that failed. One cell wide,
/// a shape of its own (the notification marks are filled shapes).
pub fn failed_symbol() -> &'static str {
    custom_symbol(StateName::Failed).unwrap_or("✗")
}

/// The symbol of the status for something that is running: a command (by the marks of the shell
/// prompt) and a progress report with no amount both show it.
pub fn running_symbol() -> &'static str {
    custom_symbol(StateName::Running).unwrap_or("▸")
}

/// The symbol of a command that ended well while nobody looked: the mark of news, unless the
/// setting for `finished` gives another.
pub fn finished_symbol() -> &'static str {
    custom_symbol(StateName::Finished)
        .unwrap_or_else(|| default_notice_symbol(crate::notify::Class::Info))
}

/// The eight level blocks, from a sliver to the full cell.
const LEVEL_BLOCKS: [&str; 8] = ["▁", "▂", "▃", "▄", "▅", "▆", "▇", "█"];

/// The level block for a value of 0 to 100: 0 to 12 the lowest, 88 to 100 the full block.
pub fn level_block(value: u8) -> &'static str {
    LEVEL_BLOCKS[(usize::from(value.min(100)) * 8 / 100).min(7)]
}

/// How a tab that has a state looks (`[status] state-style`). The width of a tab is the same in
/// every one of them.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum StateStyle {
    /// The symbol in its colour on the block of the tab's name, where the number was.
    Plain,
    /// A dark badge with the symbol in its colour.
    #[default]
    Dark,
    /// The badge filled with the state's colour and a dark symbol.
    Filled,
    /// The badge keeps the window's number and takes the state's colour as its background.
    Number,
}

impl StateStyle {
    pub fn parse(word: &str) -> Option<StateStyle> {
        match word {
            "plain" => Some(StateStyle::Plain),
            "dark" => Some(StateStyle::Dark),
            "filled" => Some(StateStyle::Filled),
            "number" => Some(StateStyle::Number),
            _ => None,
        }
    }
}

/// The states whose symbol and colour can be set (`[status.state]`).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum StateName {
    Approval,
    Input,
    Error,
    Info,
    Running,
    Progress,
    Failed,
    Finished,
    Paused,
}

impl StateName {
    pub const ALL: [StateName; 9] = [
        StateName::Approval,
        StateName::Input,
        StateName::Error,
        StateName::Info,
        StateName::Running,
        StateName::Progress,
        StateName::Failed,
        StateName::Finished,
        StateName::Paused,
    ];

    pub fn word(self) -> &'static str {
        match self {
            StateName::Approval => "approval",
            StateName::Input => "input",
            StateName::Error => "error",
            StateName::Info => "info",
            StateName::Running => "running",
            StateName::Progress => "progress",
            StateName::Failed => "failed",
            StateName::Finished => "finished",
            StateName::Paused => "paused",
        }
    }

    pub fn parse(word: &str) -> Option<StateName> {
        StateName::ALL.into_iter().find(|n| n.word() == word)
    }

    /// The level block is the symbol of these two, so a symbol is not taken for them.
    pub fn has_symbol(self) -> bool {
        !matches!(self, StateName::Progress | StateName::Paused)
    }

    fn index(self) -> usize {
        StateName::ALL.iter().position(|n| *n == self).unwrap_or(0)
    }

    fn of_class(class: crate::notify::Class) -> StateName {
        match class {
            crate::notify::Class::Approval => StateName::Approval,
            crate::notify::Class::Input => StateName::Input,
            crate::notify::Class::Error => StateName::Error,
            crate::notify::Class::Info => StateName::Info,
        }
    }

    fn of_tint(tint: Tint) -> StateName {
        match tint {
            Tint::Class(class) => StateName::of_class(class),
            Tint::ProgressError => StateName::Failed,
            Tint::Progress => StateName::Progress,
            Tint::Running => StateName::Running,
            Tint::Paused => StateName::Paused,
            Tint::Finished => StateName::Finished,
        }
    }
}

/// What the settings give for one state: either part may be left out.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct StateSetting {
    pub symbol: Option<&'static str>,
    pub color: Option<Color>,
}

/// The look of the states the settings ask for: the style of a tab, and a symbol and colour for
/// any state.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct StateLook {
    pub style: StateStyle,
    settings: [StateSetting; 9],
    /// The states whose symbol blinks on the bar (`[status] blink`), and the time of one half of
    /// a blink in milliseconds (`[status] blink-interval`).
    blink: [bool; 9],
    pub blink_ms: u64,
    /// Whether a tab with a state takes turns between its symbol and its number
    /// (`[status] state-number = "alternate"`); `replace` shows the symbol only.
    pub alternate: bool,
    /// How long each phase of that lasts, in milliseconds (`[status] alternate-interval`).
    pub alternate_ms: u64,
}

/// The shortest phase the settings allow and the default one, in milliseconds.
pub const ALTERNATE_MIN_MS: u64 = 200;
pub const ALTERNATE_DEFAULT_MS: u64 = 2000;

/// The shortest half blink the settings allow and the default one, in milliseconds.
pub const BLINK_MIN_MS: u64 = 200;
pub const BLINK_DEFAULT_MS: u64 = 1000;

impl Default for StateLook {
    fn default() -> Self {
        // The two states that wait for a person blink until something answers them.
        let mut blink = [false; 9];
        blink[StateName::Approval.index()] = true;
        blink[StateName::Input.index()] = true;
        StateLook {
            style: StateStyle::default(),
            settings: [StateSetting::default(); 9],
            blink,
            blink_ms: BLINK_DEFAULT_MS,
            alternate: true,
            alternate_ms: ALTERNATE_DEFAULT_MS,
        }
    }
}

impl StateLook {
    pub fn set(&mut self, name: StateName, setting: StateSetting) {
        self.settings[name.index()] = setting;
    }

    pub fn set_blink(&mut self, name: StateName, on: bool) {
        self.blink[name.index()] = on;
    }

    pub fn blinks(&self, name: StateName) -> bool {
        self.blink[name.index()]
    }

    /// Whether any state blinks at all.
    pub fn any_blink(&self) -> bool {
        self.blink.iter().any(|b| *b)
    }
}

/// What the bar being drawn knows about blinking: whether it is being drawn at all, which half of
/// a blink it is, and whether a symbol that blinks was drawn on it. Blinking is only for the
/// status bar; the lists, the tree and the corner of a pane never take part.
#[derive(Clone, Copy, Default)]
struct BarBlink {
    drawing: bool,
    dim: bool,
    /// Whether the tabs show their number in this phase of the alternation (not the symbol).
    number: bool,
    seen: bool,
    alternate_seen: bool,
}

thread_local! {
    static BAR: std::cell::Cell<BarBlink> = const { std::cell::Cell::new(BarBlink { drawing: false, dim: false, number: false, seen: false, alternate_seen: false }) };
}

/// What the bar drawn has in it that changes with time: a symbol that blinks, and a tab that
/// takes turns between its symbol and its number.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct BarSeen {
    pub blink: bool,
    pub alternate: bool,
}

/// Draw the status bar in the given half of a blink (`dim`: the symbols that blink are in their
/// dim colour) and in the given phase of the alternation (`number`: the tabs with a state show
/// their number). Returns what `f` returned and what changes with time on the bar.
pub fn drawing_the_bar_at<R>(dim: bool, number: bool, f: impl FnOnce() -> R) -> (R, BarSeen) {
    BAR.with(|b| {
        b.set(BarBlink {
            drawing: true,
            dim,
            number,
            seen: false,
            alternate_seen: false,
        })
    });
    let result = f();
    let seen = BAR.with(|b| {
        let bar = b.get();
        b.set(BarBlink::default());
        BarSeen {
            blink: bar.seen,
            alternate: bar.alternate_seen,
        }
    });
    (result, seen)
}

/// Draw the status bar in the given half of a blink. Returns what `f` returned and whether a
/// symbol that blinks is on the bar.
#[cfg(test)]
pub fn drawing_the_bar<R>(dim: bool, f: impl FnOnce() -> R) -> (R, bool) {
    let (result, seen) = drawing_the_bar_at(dim, false, f);
    (result, seen.blink)
}

/// Whether the phase of an alternation that lasts `interval_ms` at `elapsed_ms` is the second one
/// (the number).
pub fn alternate_phase(elapsed_ms: u128, interval_ms: u128) -> bool {
    (elapsed_ms / interval_ms.max(1)) % 2 == 1
}

/// Whether tabs with a state take turns between their symbol and their number.
pub fn alternates() -> bool {
    LOOK.with(|l| l.get().alternate)
}

/// Whether the tabs are showing their number in this phase, while the bar is being drawn.
pub fn number_phase() -> bool {
    BAR.with(|b| b.get().drawing && b.get().number)
}

/// Note that a tab with a state takes turns on the bar being drawn.
pub fn note_alternate() {
    BAR.with(|b| {
        b.set(BarBlink {
            alternate_seen: true,
            ..b.get()
        })
    });
}

/// Run `f` as if nothing blinks: the colours it draws are the bright ones (the number of a tab
/// that takes turns does not blink).
pub fn without_blink<R>(f: impl FnOnce() -> R) -> R {
    let saved = BAR.with(std::cell::Cell::get);
    BAR.with(|b| {
        b.set(BarBlink {
            drawing: false,
            ..saved
        })
    });
    let out = f();
    BAR.with(|b| b.set(saved));
    out
}

/// The colour of a state's symbol as drawn now: its own, or the dim one in the dim half of a
/// blink when the bar is being drawn and the state is one that blinks.
fn blinked(name: StateName, colour: Color) -> Color {
    let look = LOOK.with(std::cell::Cell::get);
    if !look.blinks(name) {
        return colour;
    }
    let bar = BAR.with(std::cell::Cell::get);
    if !bar.drawing {
        return colour;
    }
    BAR.with(|b| b.set(BarBlink { seen: true, ..bar }));
    if bar.dim { dimmed(colour) } else { colour }
}

/// A colour a little over half way to the bar's own background: still there, plainly less bright.
fn dimmed(colour: Color) -> Color {
    let (Color::Rgb(r, g, b), Color::Rgb(br, bg, bb)) = (colour, palette::MANTLE) else {
        return colour;
    };
    let mix = |v: u8, to: u8| ((u32::from(v) * 40 + u32::from(to) * 60) / 100) as u8;
    Color::Rgb(mix(r, br), mix(g, bg), mix(b, bb))
}

thread_local! {
    // The server runs on one thread and draws on it, so the look it was given is read there; a
    // test draws on its own thread with its own look.
    static LOOK: std::cell::Cell<StateLook> = std::cell::Cell::new(StateLook::default());
}

/// Give the thread the look of the states from the configuration (at start and on reload).
pub fn set_state_look(look: StateLook) {
    LOOK.with(|l| l.set(look));
}

pub fn state_style() -> StateStyle {
    LOOK.with(|l| l.get().style)
}

fn custom_symbol(name: StateName) -> Option<&'static str> {
    LOOK.with(|l| l.get().settings[name.index()].symbol)
}

fn custom_colour(name: StateName) -> Option<Color> {
    LOOK.with(|l| l.get().settings[name.index()].color)
}

/// A symbol kept for the life of the program, so that a look can be copied around freely; the
/// same text is kept once.
pub fn intern(text: &str) -> &'static str {
    static KEPT: std::sync::Mutex<Vec<&'static str>> = std::sync::Mutex::new(Vec::new());
    let mut kept = KEPT.lock().unwrap_or_else(|e| e.into_inner());
    if let Some(found) = kept.iter().find(|k| **k == text) {
        return found;
    }
    let leaked: &'static str = Box::leak(text.to_string().into_boxed_str());
    kept.push(leaked);
    leaked
}

/// A colour written as `#rrggbb`.
pub fn parse_color(text: &str) -> Option<Color> {
    let hex = text.strip_prefix('#')?;
    if hex.len() != 6 || !hex.chars().all(|c| c.is_ascii_hexdigit()) {
        return None;
    }
    let part = |i: usize| u8::from_str_radix(&hex[i..i + 2], 16).ok();
    Some(Color::Rgb(part(0)?, part(2)?, part(4)?))
}

fn default_tint_colour(tint: Tint) -> Color {
    match tint {
        Tint::Class(crate::notify::Class::Approval) => palette::RED,
        Tint::Class(crate::notify::Class::Input) => palette::YELLOW,
        Tint::Class(crate::notify::Class::Error) => palette::PINK,
        Tint::Class(crate::notify::Class::Info) => palette::TEAL,
        Tint::ProgressError => palette::RED,
        Tint::Progress | Tint::Running => palette::BLUE,
        Tint::Paused => palette::OVERLAY2,
        Tint::Finished => palette::GREEN,
    }
}

/// The colour of a state: the one the settings give, or the palette's.
fn tint_colour(tint: Tint) -> Color {
    custom_colour(StateName::of_tint(tint)).unwrap_or_else(|| default_tint_colour(tint))
}

/// The style of the divider cells around a pane that carries a state: the state's colour, the
/// same as its mark on the tab, on the divider's own background (the terminal's, which the
/// dividers leave as it is), bold as the focus colour is.
pub fn divider_state_style(tint: Tint) -> Style {
    Style {
        fg: tint_colour(tint),
        bold: true,
        ..style(Role::Divider)
    }
}

/// The status cell's symbol in a colour on the block it stands on, as the marks of the classes:
/// the palette's colour, made lighter only as far as it takes to read at a contrast of 4.5. A
/// colour the settings gave is used as it is, whatever its contrast.
pub fn cell_style(tint: Tint, block: Style) -> Style {
    cell_style_on(tint, block, false)
}

/// Whether a state is one that asks nothing of the user: on the tab of the window that is
/// current its symbol takes the colour of the current tab (the accent), so the tab stays the one
/// the eye finds. Approval, input, error and a failed command keep their own colours.
fn quiet_tint(tint: Tint) -> bool {
    matches!(
        tint,
        Tint::Running | Tint::Progress | Tint::Paused | Tint::Finished
    )
}

/// The colour of the state's symbol on a tab: the settings' colour wins, then the accent on the
/// current tab for a quiet state, then the palette's.
fn tint_colour_on(tint: Tint, current: bool) -> Color {
    if current && quiet_tint(tint) && custom_colour(StateName::of_tint(tint)).is_none() {
        return palette::MAUVE;
    }
    tint_colour(tint)
}

/// `cell_style` for a tab that is `current` or not.
pub fn cell_style_on(tint: Tint, block: Style, current: bool) -> Style {
    let name = StateName::of_tint(tint);
    let fg = match custom_colour(name) {
        Some(colour) => colour,
        None if current && quiet_tint(tint) => readable_on(palette::MAUVE, block.bg),
        None => readable_on(default_tint_colour(tint), block.bg),
    };
    let fg = blinked(name, fg);
    Style {
        fg,
        bg: block.bg,
        bold: true,
        ..Style::default()
    }
}

/// The badge of a tab that has a state, in the three looks that give the badge a colour of its
/// own (`plain` has none: the symbol stands on the name's block, see `cell_style`):
/// - `dark`: the darkest base of the palette (the bar's own background is MANTLE and the name's
///   surface is lighter, so CRUST is the dark that stands apart from both) with the state's
///   colour for the symbol;
/// - `filled` and `number`: the state's colour as the background and the dark text colour of the
///   bar on it (for the symbol, or for the number).
#[cfg(test)]
pub fn state_badge(look: StateStyle, tint: Tint) -> Style {
    state_badge_on(look, tint, false)
}

/// `state_badge` for a tab that is `current` or not.
pub fn state_badge_on(look: StateStyle, tint: Tint, current: bool) -> Style {
    let colour = blinked(StateName::of_tint(tint), tint_colour_on(tint, current));
    // A quiet state on the current tab is the accent: a filled badge would take the colour of the
    // accent block beside it, so it is the dark badge whatever the look.
    let look = if current && quiet_tint(tint) {
        StateStyle::Dark
    } else {
        look
    };
    Style {
        bold: true,
        ..match look {
            StateStyle::Dark => on(colour, palette::CRUST),
            StateStyle::Plain | StateStyle::Filled | StateStyle::Number => {
                on(palette::CRUST, colour)
            }
        }
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
        Symbol::TreeOtherPane => "  ",
        Symbol::TreeLifted => " [lifted]",
        Symbol::TreePathSeparator => " › ",
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_phase_of_an_alternation_changes_each_interval_from_the_symbol_to_the_number() {
        assert!(!alternate_phase(0, 1000));
        assert!(!alternate_phase(999, 1000));
        assert!(alternate_phase(1000, 1000));
        assert!(alternate_phase(1999, 1000));
        assert!(!alternate_phase(2000, 1000));
        // A zero interval is taken as one millisecond, not a division by zero.
        assert!(!alternate_phase(0, 0));
    }

    #[test]
    fn the_current_tab_name_is_a_lighter_grey_block_and_its_number_is_the_accent_on_dark() {
        assert_eq!(
            style(Role::PillNameCurrent),
            Style {
                bold: true,
                ..on(palette::WHITE, palette::SURFACE2)
            }
        );
        assert_eq!(
            style(Role::PillBadgeCurrent),
            Style {
                bold: true,
                ..on(palette::MAUVE, palette::CRUST)
            }
        );
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
            for role in [Role::StatusBar, Role::PillName] {
                let on_block = cell_style(Tint::Class(class), style(role));
                assert_eq!(on_block.bg, style(role).bg);
                let ratio = contrast(on_block);
                assert!(ratio >= 4.5, "{class:?} on {role:?}: {ratio:.2}");
            }
            // On the current tab the symbol sits on the dark badge: the accent block carries none.
            let on_badge = state_badge_on(StateStyle::Dark, Tint::Class(class), true);
            let ratio = contrast(on_badge);
            assert!(
                ratio >= 4.5,
                "{class:?} on the current tab's badge: {ratio:.2}"
            );
        }
    }

    const TINTS: [Tint; 9] = [
        Tint::Class(crate::notify::Class::Approval),
        Tint::Class(crate::notify::Class::Input),
        Tint::Class(crate::notify::Class::Error),
        Tint::Class(crate::notify::Class::Info),
        Tint::ProgressError,
        Tint::Progress,
        Tint::Running,
        Tint::Paused,
        Tint::Finished,
    ];

    fn black_contrast(colour: Color) -> f64 {
        contrast(Style {
            fg: colour,
            bg: Color::Rgb(0, 0, 0),
            ..Style::default()
        })
    }

    /// In the default look the symbol stands on the name's block in its class colour, readable at
    /// 4.5 or more and never darker than the class colour itself.
    #[test]
    fn the_symbol_of_a_state_is_never_darker_than_its_colour_and_reads_on_the_tab() {
        for role in [Role::PillName, Role::StatusBar] {
            let block = style(role);
            for tint in TINTS {
                let drawn = cell_style(tint, block);
                assert_eq!(drawn.bg, block.bg, "{tint:?}");
                assert!(contrast(drawn) >= 4.5, "{tint:?} on {role:?}");
                assert!(
                    black_contrast(drawn.fg) >= black_contrast(default_tint_colour(tint)) - 1e-9,
                    "{tint:?} on {role:?}: the symbol is darker than its class colour"
                );
            }
        }
        // On the current tab the symbol sits on the dark badge.
        for tint in TINTS {
            let drawn = state_badge_on(StateStyle::Dark, tint, true);
            assert_eq!(drawn.bg, palette::CRUST, "{tint:?}");
            assert!(
                contrast(drawn) >= 4.5,
                "{tint:?} on the current tab's badge"
            );
        }
    }

    /// The three looks with a badge of their own: the dark base with the bright colour, or the
    /// colour filled with the dark text; each reads at 4.5 or more for every state.
    #[test]
    fn the_badge_of_a_state_reads_in_every_style() {
        let bar = style(Role::StatusBar).bg;
        for tint in TINTS {
            let dark = state_badge(StateStyle::Dark, tint);
            assert_eq!(dark.bg, palette::CRUST, "{tint:?}");
            assert_eq!(dark.fg, tint_colour(tint), "{tint:?}");
            assert_ne!(dark.bg, bar);
            assert_ne!(dark.bg, style(Role::PillName).bg);
            assert!(contrast(dark) >= 4.5, "{tint:?}");
            for look in [StateStyle::Filled, StateStyle::Number] {
                let filled = state_badge(look, tint);
                assert_eq!(filled.bg, tint_colour(tint), "{tint:?}");
                assert_eq!(filled.fg, palette::CRUST, "{tint:?}");
                assert!(contrast(filled) >= 4.5, "{tint:?} {look:?}");
            }
        }
        // Something that asks for you, an error, news, a level and a pause are told apart.
        assert_ne!(
            tint_colour(Tint::Class(crate::notify::Class::Input)),
            tint_colour(Tint::Class(crate::notify::Class::Error))
        );
        assert_ne!(tint_colour(Tint::Progress), tint_colour(Tint::Finished));
    }

    /// A colour from the settings is used as given, a symbol from the settings replaces the
    /// default one, and both go back with the default look.
    #[test]
    fn the_settings_change_the_symbol_and_colour_of_a_state() {
        let mut look = StateLook::default();
        look.set(
            StateName::Finished,
            StateSetting {
                symbol: Some(intern("X")),
                color: parse_color("#102030"),
            },
        );
        set_state_look(look);
        let drawn = cell_style(Tint::Finished, style(Role::PillName));
        assert_eq!(drawn.fg, Color::Rgb(0x10, 0x20, 0x30));
        assert_eq!(finished_symbol(), "X");
        set_state_look(StateLook::default());
        assert_ne!(finished_symbol(), "X");
        assert_eq!(
            cell_style(Tint::Finished, style(Role::PillName)).fg,
            readable_on(palette::GREEN, style(Role::PillName).bg)
        );
        assert_eq!(parse_color("#12345"), None);
        assert_eq!(parse_color("123456"), None);
        assert_eq!(parse_color("#12345g"), None);
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

    /// A quiet current tab is a grey block, not the accent and not the grey of the paused state;
    /// it is clearly brighter than the block of the other tabs and its text reads on it.
    #[test]
    fn the_quiet_current_tab_block_is_a_lighter_grey_that_reads() {
        for role in [Role::PillNameCurrent, Role::PillQuietCurrent] {
            let current = style(role);
            assert_ne!(current.bg, palette::MAUVE, "{role:?}");
            assert_ne!(current.bg, palette::OVERLAY2, "{role:?}");
            let against_other_tabs = contrast(Style {
                fg: current.bg,
                bg: palette::SURFACE0,
                ..Style::default()
            });
            assert!(
                against_other_tabs >= 1.5,
                "{role:?} against the other tabs: {against_other_tabs:.2}"
            );
            let ratio = contrast(current);
            assert!(ratio >= 4.5, "{role:?}: {ratio:.2}");
        }
    }

    /// The roles of the floating lists and the tree are drawn in the palette of the bar: no
    /// index of the terminal's 16 colours is left, the panel is darker than the bar, the frame
    /// and the title are accents, the chosen line is white on SURFACE0, and every text reads at
    /// 4.5 or more on its own background.
    #[test]
    fn the_list_roles_use_the_bars_palette_and_every_text_reads() {
        let roles = [
            Role::Picker,
            Role::PickerFrame,
            Role::PickerTitle,
            Role::PickerQuiet,
            Role::PickerKey,
            Role::PickerHeading,
            Role::PickerNumber,
            Role::PickerSelected,
        ];
        for role in roles {
            let st = style(role);
            assert!(matches!(st.fg, Color::Rgb(..)), "{role:?} fg {:?}", st.fg);
            assert!(matches!(st.bg, Color::Rgb(..)), "{role:?} bg {:?}", st.bg);
            let ratio = contrast(st);
            assert!(ratio >= 4.5, "{role:?}: {ratio:.2}");
            println!("{role:?}: {ratio:.2}");
        }
        let chosen = style(Role::PickerSelected);
        assert_eq!((chosen.fg, chosen.bg), (palette::WHITE, palette::SURFACE0));
        assert!(chosen.bold);
        // The panel is darker than the bar and than the base of the theme, so that it stands
        // apart from what is under it; its frame and title are accents.
        assert_eq!(style(Role::Picker).bg, palette::CRUST);
        assert_eq!(style(Role::PickerFrame).fg, palette::LAVENDER);
        assert_eq!(style(Role::PickerTitle).fg, palette::MAUVE);
        let base = Color::Rgb(0x30, 0x34, 0x46);
        let frame_on_base = contrast_of(palette::LAVENDER, base);
        println!("frame against BASE: {frame_on_base:.2}");
        assert!(frame_on_base >= 4.5);
        // The key and the word after it are told apart by colour and by weight.
        assert_ne!(style(Role::PickerKey).fg, style(Role::PickerQuiet).fg);
        assert!(style(Role::PickerKey).bold && !style(Role::PickerQuiet).bold);
    }

    /// A state symbol reads on every line of a list: at 4.5 on the panel, and on the chosen
    /// line, where it stands directly on the grey in its own colour, at 3.0 or more (it is a
    /// symbol, not text).
    #[test]
    fn a_state_symbol_reads_on_a_list_line_and_on_the_chosen_line() {
        use crate::notify::Class;
        let panel = style(Role::Picker).bg;
        let chosen = style(Role::PickerSelected).bg;
        let mut colours: Vec<(String, Color)> = Class::BY_URGENCY
            .into_iter()
            .map(|c| (format!("{c:?}"), notice_style(c).fg))
            .collect();
        for tint in [Tint::Finished, Tint::ProgressError, Tint::Running] {
            colours.push((
                format!("{tint:?}"),
                cell_style(tint, style(Role::PillName)).fg,
            ));
        }
        for (name, colour) in colours {
            let on_panel = contrast_of(colour, panel);
            let on_chosen = contrast_of(colour, chosen);
            println!("{name}: panel {on_panel:.2}, chosen line {on_chosen:.2}");
            assert!(on_panel >= 4.5, "{name} on the panel: {on_panel:.2}");
            assert!(
                on_chosen >= 3.0,
                "{name} on the chosen line: {on_chosen:.2}"
            );
        }
    }

    /// No role of the theme is drawn with one of the terminal's 16 (or 256) colours, whatever the
    /// terminal's theme is; the roles that have a background read on it at 4.5 or more. The
    /// match has no catch-all arm, so a new role has to be listed here.
    #[test]
    fn no_role_uses_an_indexed_colour_and_every_text_reads() {
        fn kind(role: Role) -> bool {
            // true: has its own background, so the pair is checked; false: sits on whatever is
            // under it (the dividers) or only draws a line under text.
            match role {
                Role::Divider | Role::DividerFocus | Role::LinkUnderline => false,
                Role::StatusBar
                | Role::Indicator
                | Role::MessageInfo
                | Role::MessageError
                | Role::MessageQuestion
                | Role::PromptLabel
                | Role::PromptText
                | Role::Picker
                | Role::PickerFrame
                | Role::PickerTitle
                | Role::PickerQuiet
                | Role::PickerKey
                | Role::PickerNumber
                | Role::PickerSelected
                | Role::PickerHeading
                | Role::Selection
                | Role::IndicatorSelect
                | Role::ScrollPosition
                | Role::JumpMatch
                | Role::JumpLabel
                | Role::PillName
                | Role::PillNameCurrent
                | Role::PillQuiet
                | Role::PillQuietCurrent
                | Role::PillBadge
                | Role::PillBadgeCurrent
                | Role::SessionBadge
                | Role::SessionName => true,
            }
        }
        let roles = [
            Role::StatusBar,
            Role::Indicator,
            Role::MessageInfo,
            Role::MessageError,
            Role::MessageQuestion,
            Role::PromptLabel,
            Role::PromptText,
            Role::Divider,
            Role::DividerFocus,
            Role::Picker,
            Role::PickerFrame,
            Role::PickerTitle,
            Role::PickerQuiet,
            Role::PickerKey,
            Role::PickerNumber,
            Role::PickerSelected,
            Role::PickerHeading,
            Role::Selection,
            Role::IndicatorSelect,
            Role::ScrollPosition,
            Role::LinkUnderline,
            Role::JumpMatch,
            Role::JumpLabel,
            Role::PillName,
            Role::PillNameCurrent,
            Role::PillQuiet,
            Role::PillQuietCurrent,
            Role::PillBadge,
            Role::PillBadgeCurrent,
            Role::SessionBadge,
            Role::SessionName,
        ];
        for role in roles {
            let st = style(role);
            assert!(
                !matches!(st.fg, Color::Indexed(_)),
                "{role:?} fg {:?}",
                st.fg
            );
            assert!(
                !matches!(st.bg, Color::Indexed(_)),
                "{role:?} bg {:?}",
                st.bg
            );
            if kind(role) {
                let ratio = contrast(st);
                assert!(ratio >= 4.5, "{role:?}: {ratio:.2}");
            }
        }
        // The dividers: the focus colour is the accent, the others a quiet grey; both read on
        // the bar's background and on the base colour of the theme.
        assert_eq!(style(Role::DividerFocus).fg, palette::MAUVE);
        assert!(style(Role::DividerFocus).bold);
        assert_ne!(style(Role::Divider).fg, style(Role::DividerFocus).fg);
        let base = Color::Rgb(0x30, 0x34, 0x46);
        for (name, fg) in [
            ("divider", style(Role::Divider).fg),
            ("focus", style(Role::DividerFocus).fg),
        ] {
            let on_mantle = contrast_of(fg, palette::MANTLE);
            let on_base = contrast_of(fg, base);
            println!("{name}: {on_mantle:.2} on MANTLE, {on_base:.2} on BASE");
            assert!(on_mantle >= 3.0 && on_base >= 2.5, "{name}");
        }
        // Copy mode: the position mark, the jump match, the label and the selection differ.
        let looks = [
            style(Role::ScrollPosition).bg,
            style(Role::JumpLabel).bg,
            style(Role::Selection).bg,
        ];
        assert_ne!(looks[0], looks[1]);
        assert_ne!(looks[0], looks[2]);
        assert_ne!(looks[1], looks[2]);
        assert_ne!(style(Role::JumpMatch).bg, style(Role::JumpLabel).bg);
        assert_ne!(style(Role::JumpMatch).bg, style(Role::Selection).bg);
        // The focus colour is not the colour of a state that can colour a border.
        use crate::notify::Class;
        for tint in [
            Tint::Class(Class::Approval),
            Tint::Class(Class::Input),
            Tint::Class(Class::Error),
            Tint::Class(Class::Info),
            Tint::ProgressError,
            Tint::Progress,
            Tint::Running,
            Tint::Paused,
            Tint::Finished,
        ] {
            assert_ne!(
                style(Role::DividerFocus).fg,
                default_tint_colour(tint),
                "{tint:?}"
            );
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
