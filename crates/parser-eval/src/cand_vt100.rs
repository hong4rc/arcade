//! Candidate: the `vt100` crate (parser from the `vte` crate, its own screen model).

use std::num::NonZeroU16;

use crate::candidate::{Candidate, CellStyle, Color, Event, Mode, Underline};

#[derive(Default)]
struct Callbacks {
    events: Vec<Event>,
}

impl vt100::Callbacks for Callbacks {
    fn audible_bell(&mut self, _: &mut vt100::Screen) {
        self.events.push(Event::Bell);
    }

    fn set_window_title(&mut self, _: &mut vt100::Screen, title: &[u8]) {
        self.events
            .push(Event::Title(String::from_utf8_lossy(title).into_owned()));
    }

    fn copy_to_clipboard(&mut self, _: &mut vt100::Screen, _ty: &[u8], data: &[u8]) {
        // The crate passes the payload still encoded; decoding is left to the embedder.
        self.events.push(Event::ClipboardWrite(
            crate::base64_decode(data).unwrap_or_default(),
        ));
    }

    fn paste_from_clipboard(&mut self, _: &mut vt100::Screen, _ty: &[u8]) {
        self.events.push(Event::ClipboardRead);
    }
}

pub struct Vt100 {
    parser: vt100::Parser<Callbacks>,
}

impl Vt100 {
    pub fn new(cols: u16, rows: u16, scrollback: usize) -> Self {
        let rows = NonZeroU16::new(rows).expect("rows");
        let cols = NonZeroU16::new(cols).expect("cols");
        Vt100 {
            parser: vt100::Parser::new_with_callbacks(rows, cols, scrollback, Callbacks::default()),
        }
    }
}

impl Candidate for Vt100 {
    fn feed(&mut self, bytes: &[u8]) {
        self.parser.process(bytes);
    }

    fn row_text(&self, row: u16) -> String {
        let screen = self.parser.screen();
        let cols = screen.size().1.get();
        let mut text = String::new();
        for col in 0..cols {
            let Some(cell) = screen.cell(row, col) else {
                continue;
            };
            if cell.is_wide_continuation() {
                continue;
            }
            if cell.has_contents() {
                text.push_str(cell.contents());
            } else {
                text.push(' ');
            }
        }
        text.trim_end().to_string()
    }

    fn cursor(&self) -> (u16, u16) {
        let (row, col) = self.parser.screen().cursor_position();
        (col, row)
    }

    fn drain_events(&mut self) -> Vec<Event> {
        std::mem::take(&mut self.parser.callbacks_mut().events)
    }

    fn style_at(&self, row: u16, col: u16) -> CellStyle {
        let screen = self.parser.screen();
        let Some(cell) = screen.cell(row, col) else {
            return CellStyle {
                bold: false,
                italic: false,
                underline: Underline::None,
                strike: false,
                inverse: false,
                fg: Color::Default,
                hyperlink: None,
            };
        };
        CellStyle {
            bold: cell.bold(),
            italic: cell.italic(),
            // The crate stores a single underline bit and has no strikethrough or hyperlinks.
            underline: if cell.underline() {
                Underline::Single
            } else {
                Underline::None
            },
            strike: false,
            inverse: cell.inverse(),
            fg: match cell.fgcolor() {
                vt100::Color::Default => Color::Default,
                vt100::Color::Idx(index) => Color::Indexed(index),
                vt100::Color::Rgb(r, g, b) => Color::Rgb(r, g, b),
            },
            hyperlink: None,
        }
    }

    fn scrollback_lines(&self) -> usize {
        self.parser.screen().scrollback_len()
    }

    fn mode(&self, mode: Mode) -> Option<bool> {
        let screen = self.parser.screen();
        match mode {
            Mode::AltScreen => Some(screen.alternate_screen()),
            Mode::BracketedPaste => Some(screen.bracketed_paste()),
            Mode::MouseTracking => {
                Some(screen.mouse_protocol_mode() != vt100::MouseProtocolMode::None)
            }
            Mode::MouseSgr => {
                Some(screen.mouse_protocol_encoding() == vt100::MouseProtocolEncoding::Sgr)
            }
            Mode::FocusReport | Mode::SyncOutput | Mode::KittyKeyboard => None,
        }
    }
}
