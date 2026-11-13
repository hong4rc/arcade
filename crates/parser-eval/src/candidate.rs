//! The common surface every parser candidate is driven through.

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Event {
    Bell,
    Title(String),
    /// A program asked to write the clipboard; the bytes are the decoded text.
    ClipboardWrite(Vec<u8>),
    /// A program asked to read the clipboard. The harness never answers it.
    ClipboardRead,
    Notification {
        title: String,
        body: String,
    },
    /// Bytes the terminal wants to send back to the program (answers to queries).
    Reply(Vec<u8>),
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Underline {
    None,
    Single,
    Double,
    Curly,
    Dotted,
    Dashed,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Color {
    Default,
    Indexed(u8),
    Rgb(u8, u8, u8),
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct CellStyle {
    pub bold: bool,
    pub italic: bool,
    pub underline: Underline,
    pub strike: bool,
    pub inverse: bool,
    pub fg: Color,
    /// `None` when the library cannot report hyperlinks at all.
    pub hyperlink: Option<String>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Mode {
    AltScreen,
    BracketedPaste,
    MouseTracking,
    MouseSgr,
    FocusReport,
    SyncOutput,
    KittyKeyboard,
}

pub trait Candidate {
    fn feed(&mut self, bytes: &[u8]);
    /// Text of one visible row, with trailing blanks removed.
    fn row_text(&self, row: u16) -> String;
    /// Cursor position as (column, row), both starting at zero.
    fn cursor(&self) -> (u16, u16);
    fn drain_events(&mut self) -> Vec<Event>;
    fn style_at(&self, row: u16, col: u16) -> CellStyle;
    /// Whether a mode is currently on. `None` when the library does not track or expose it.
    fn mode(&self, mode: Mode) -> Option<bool>;
    /// Number of lines currently kept above the visible screen.
    fn scrollback_lines(&self) -> usize {
        0
    }
    /// Ask the library to shrink idle scrollback, if it can. Returns whether it supports that.
    fn compress_scrollback(&mut self) -> bool {
        false
    }
}

pub const CANDIDATES: [&str; 4] = [
    "alacritty_terminal",
    "vt100",
    "libghostty-vt",
    "vte-minimal",
];

/// The candidates that are complete terminal emulators. The minimal self-written screen is only
/// a speed floor and is left out of the correctness and memory comparisons.
pub const EMULATORS: [&str; 3] = ["alacritty_terminal", "vt100", "libghostty-vt"];

pub fn make(name: &str, cols: u16, rows: u16, scrollback: usize) -> Box<dyn Candidate> {
    match name {
        "alacritty_terminal" => Box::new(crate::cand_alacritty::Alacritty::new(
            cols, rows, scrollback,
        )),
        "vt100" => Box::new(crate::cand_vt100::Vt100::new(cols, rows, scrollback)),
        "libghostty-vt" => Box::new(crate::cand_ghostty::Ghostty::new(cols, rows, scrollback)),
        "vte-minimal" => Box::new(crate::cand_vte_min::VteMinimal::new(cols, rows, scrollback)),
        other => panic!("unknown candidate {other}"),
    }
}
