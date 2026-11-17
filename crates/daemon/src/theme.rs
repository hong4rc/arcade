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
        // The current tab's name is a block of the accent, with the dark text of the bar on it.
        Role::PillNameCurrent => Style {
            bold: true,
            ..on(palette::CRUST, palette::MAUVE)
        },
        Role::PillQuiet => on(palette::SUBTEXT0, palette::SURFACE0),
        Role::PillQuietCurrent => on(palette::CRUST, palette::MAUVE),
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
            bold: true,
            ..on(palette::CRUST, palette::MAUVE)
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
}

/// The shortest half blink the settings allow and the default one, in milliseconds.
pub const BLINK_MIN_MS: u64 = 200;
pub const BLINK_DEFAULT_MS: u64 = 500;

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
    seen: bool,
}

thread_local! {
    static BAR: std::cell::Cell<BarBlink> = const { std::cell::Cell::new(BarBlink { drawing: false, dim: false, seen: false }) };
}

/// Draw the status bar in the given half of a blink (`dim`: the symbols that blink are in their
/// dim colour). Returns what `f` returned and whether a symbol that blinks is on the bar.
pub fn drawing_the_bar<R>(dim: bool, f: impl FnOnce() -> R) -> (R, bool) {
    BAR.with(|b| {
        b.set(BarBlink {
            drawing: true,
            dim,
            seen: false,
        })
    });
    let result = f();
    let seen = BAR.with(|b| {
        let seen = b.get().seen;
        b.set(BarBlink::default());
        seen
    });
    (result, seen)
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
    fn the_current_tab_name_is_the_accent_block_with_dark_text_and_its_number_is_the_accent_on_dark()
     {
        assert_eq!(
            style(Role::PillNameCurrent),
            Style {
                bold: true,
                ..on(palette::CRUST, palette::MAUVE)
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
