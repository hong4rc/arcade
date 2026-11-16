//! Carrying a terminal's contents and settings from one terminal to another, for a daemon that
//! replaces its own program and must give every pane's terminal back as it was.
//!
//! The library's formatter writes the screen it shows as a stream of the control sequences that
//! would build it again: every row (scrollback included), the styles, the links, the cursor, the
//! modes, the scroll region, the tab stops, the colours and the keyboard flags. Three things need
//! help from here: the formatter writes only the screen that is in use, so a pane that runs a
//! full-screen program is read twice (the primary screen first, then the alternate one); the
//! sequences that switch screens are left out of what it writes and driven from here; and the
//! shape of the cursor is not part of it, so the caller hands that in.
//!
//! Not carried: the marks that say where a shell prompt began (OSC 133), and blank cells below
//! the text that were cleared with a colour of their own.

use libghostty_vt::fmt::{Extras, Format, Formatter};
use libghostty_vt::screen::Screen as ScreenKind;

use super::{CursorShape, Term, TermError, err};

/// What the formatter wrote for each screen. `alternate` is there only when a full-screen
/// program was on the alternate screen at the time.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct TerminalState {
    pub primary: Vec<u8>,
    pub alternate: Option<Vec<u8>>,
}

/// The sequences that switch screens; the formatter writes them to say which mode is set, and
/// following them would put the text of one screen onto the other.
const SCREEN_SWITCHES: [&[u8]; 6] = [
    b"\x1b[?1049h",
    b"\x1b[?1049l",
    b"\x1b[?47h",
    b"\x1b[?47l",
    b"\x1b[?1047h",
    b"\x1b[?1047l",
];

fn without_screen_switches(bytes: &[u8]) -> Vec<u8> {
    let mut out = Vec::with_capacity(bytes.len());
    let mut at = 0;
    'scan: while at < bytes.len() {
        if bytes[at] == 0x1b {
            for sequence in SCREEN_SWITCHES {
                if bytes[at..].starts_with(sequence) {
                    at += sequence.len();
                    continue 'scan;
                }
            }
        }
        out.push(bytes[at]);
        at += 1;
    }
    out
}

impl Term {
    fn dump_screen(&self) -> Result<Vec<u8>, TermError> {
        let mut formatter = Formatter::new(&self.term).map_err(|e| err("formatter", e))?;
        formatter
            .set_format(Format::Vt)
            .and_then(|f| f.set_trim(false))
            // Rows the terminal broke on its own are written as one line, so that the new
            // terminal breaks them again and knows the rows go on.
            .and_then(|f| f.set_unwrap(true))
            .and_then(|f| f.set_extras(Extras::ALL))
            .map_err(|e| err("formatter options", e))?;
        let bytes = formatter
            .format_alloc(None)
            .map_err(|e| err("format", e))?
            .to_vec();
        let mut bytes = without_screen_switches(&bytes);
        // The colours a program set as the screen's own (OSC 10, 11 and 12) are not part of what
        // the formatter writes.
        for (code, color) in [
            (10, self.term.fg_color()),
            (11, self.term.bg_color()),
            (12, self.term.cursor_color()),
        ] {
            if let Ok(Some(c)) = color {
                bytes.extend_from_slice(
                    format!("\x1b]{code};rgb:{:02x}/{:02x}/{:02x}\x1b\\", c.r, c.g, c.b).as_bytes(),
                );
            }
        }
        Ok(bytes)
    }

    /// Whether the program in the terminal is on the alternate screen.
    pub fn on_alternate_screen(&self) -> bool {
        matches!(self.term.active_screen(), Ok(ScreenKind::Alternate))
    }

    /// Write out everything a new terminal needs to show what this one shows. Takes `&mut self`
    /// because the primary screen under a full-screen program can only be read by switching to
    /// it for a moment (mode 47 does not clear anything), and the terminal is left as it was.
    pub fn save_state(&mut self) -> Result<TerminalState, TermError> {
        if !self.on_alternate_screen() {
            return Ok(TerminalState {
                primary: self.dump_screen()?,
                alternate: None,
            });
        }
        let alternate = self.dump_screen()?;
        self.term.vt_write(b"\x1b[?47l");
        let primary = self.dump_screen();
        self.term.vt_write(b"\x1b[?47h");
        Ok(TerminalState {
            primary: primary?,
            alternate: Some(alternate),
        })
    }

    /// Make this terminal, which is new and has the size the saved one had, show what was saved.
    /// `cursor` is the shape of the cursor and whether it blinks, when it is known.
    pub fn load_state(&mut self, state: &TerminalState, cursor: Option<(CursorShape, bool)>) {
        self.term.vt_write(&state.primary);
        if let Some(alternate) = &state.alternate {
            // Mode 47 switches without clearing and without saving the cursor; the screen is
            // cleared by hand, then written. The program leaves with its own 1049l.
            self.term.vt_write(b"\x1b[?47h\x1b[2J\x1b[H");
            self.term.vt_write(alternate);
        }
        if let Some((shape, blinking)) = cursor {
            let code = match (shape, blinking) {
                (CursorShape::Block, true) => Some(1),
                (CursorShape::Block, false) => Some(2),
                (CursorShape::Underline, true) => Some(3),
                (CursorShape::Underline, false) => Some(4),
                (CursorShape::Bar, true) => Some(5),
                (CursorShape::Bar, false) => Some(6),
                // The sequences cannot ask for a hollow block.
                (CursorShape::HollowBlock, _) => None,
            };
            if let Some(code) = code {
                self.term.vt_write(format!("\x1b[{code} q").as_bytes());
            }
        }
        // A new terminal has been written to; whatever it said back is not for the program.
        self.events.borrow_mut().clear();
    }
}
