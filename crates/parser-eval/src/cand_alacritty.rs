//! Candidate: `alacritty_terminal`, the terminal core of Alacritty (parser from the `vte` crate).

use std::sync::{Arc, Mutex};

use alacritty_terminal::Term;
use alacritty_terminal::event::{Event as AEvent, EventListener};
use alacritty_terminal::grid::Dimensions;
use alacritty_terminal::index::{Column, Line};
use alacritty_terminal::term::cell::Flags;
use alacritty_terminal::term::{Config, TermMode};
use alacritty_terminal::vte::ansi::{Color as AColor, Processor};

use crate::candidate::{Candidate, CellStyle, Color, Event, Mode, Underline};

#[derive(Clone, Default)]
struct Sink(Arc<Mutex<Vec<Event>>>);

impl EventListener for Sink {
    fn send_event(&self, event: AEvent) {
        let mapped = match event {
            AEvent::Bell => Event::Bell,
            AEvent::Title(title) => Event::Title(title),
            AEvent::ClipboardStore(_, text) => Event::ClipboardWrite(text.into_bytes()),
            AEvent::ClipboardLoad(..) => Event::ClipboardRead,
            AEvent::PtyWrite(text) => Event::Reply(text.into_bytes()),
            // The library hands color and size queries to the embedder as a formatter. Answer
            // with fixed values, the way a daemon would, so the reply is visible to the test.
            AEvent::ColorRequest(_, format) => Event::Reply(
                format(alacritty_terminal::vte::ansi::Rgb { r: 0, g: 0, b: 0 }).into_bytes(),
            ),
            _ => return,
        };
        self.0.lock().unwrap().push(mapped);
    }
}

struct Size {
    cols: usize,
    rows: usize,
}

impl Dimensions for Size {
    fn total_lines(&self) -> usize {
        self.rows
    }

    fn screen_lines(&self) -> usize {
        self.rows
    }

    fn columns(&self) -> usize {
        self.cols
    }
}

pub struct Alacritty {
    term: Term<Sink>,
    processor: Processor,
    sink: Sink,
}

impl Alacritty {
    pub fn new(cols: u16, rows: u16, scrollback: usize) -> Self {
        let sink = Sink::default();
        let config = Config {
            scrolling_history: scrollback,
            kitty_keyboard: true,
            ..Config::default()
        };
        let size = Size {
            cols: cols as usize,
            rows: rows as usize,
        };
        Alacritty {
            term: Term::new(config, &size, sink.clone()),
            processor: Processor::new(),
            sink,
        }
    }
}

impl Candidate for Alacritty {
    fn feed(&mut self, bytes: &[u8]) {
        self.processor.advance(&mut self.term, bytes);
    }

    fn row_text(&self, row: u16) -> String {
        let grid = self.term.grid();
        let mut text = String::new();
        for col in 0..grid.columns() {
            let cell = &grid[Line(row as i32)][Column(col)];
            if cell
                .flags
                .intersects(Flags::WIDE_CHAR_SPACER | Flags::LEADING_WIDE_CHAR_SPACER)
            {
                continue;
            }
            text.push(cell.c);
            if let Some(extra) = cell.zerowidth() {
                text.extend(extra.iter());
            }
        }
        text.trim_end().to_string()
    }

    fn cursor(&self) -> (u16, u16) {
        let point = self.term.grid().cursor.point;
        (point.column.0 as u16, point.line.0 as u16)
    }

    fn drain_events(&mut self) -> Vec<Event> {
        std::mem::take(&mut *self.sink.0.lock().unwrap())
    }

    fn style_at(&self, row: u16, col: u16) -> CellStyle {
        let cell = &self.term.grid()[Line(row as i32)][Column(col as usize)];
        let flags = cell.flags;
        let underline = if flags.contains(Flags::UNDERCURL) {
            Underline::Curly
        } else if flags.contains(Flags::DOUBLE_UNDERLINE) {
            Underline::Double
        } else if flags.contains(Flags::DOTTED_UNDERLINE) {
            Underline::Dotted
        } else if flags.contains(Flags::DASHED_UNDERLINE) {
            Underline::Dashed
        } else if flags.contains(Flags::UNDERLINE) {
            Underline::Single
        } else {
            Underline::None
        };
        let fg = match cell.fg {
            AColor::Spec(rgb) => Color::Rgb(rgb.r, rgb.g, rgb.b),
            AColor::Indexed(index) => Color::Indexed(index),
            AColor::Named(named) => {
                let index = named as usize;
                if index < 16 {
                    Color::Indexed(index as u8)
                } else {
                    Color::Default
                }
            }
        };
        CellStyle {
            bold: flags.contains(Flags::BOLD),
            italic: flags.contains(Flags::ITALIC),
            underline,
            strike: flags.contains(Flags::STRIKEOUT),
            inverse: flags.contains(Flags::INVERSE),
            fg,
            hyperlink: Some(
                cell.hyperlink()
                    .map(|link| link.uri().to_string())
                    .unwrap_or_default(),
            ),
        }
    }

    fn scrollback_lines(&self) -> usize {
        self.term.grid().history_size()
    }

    fn mode(&self, mode: Mode) -> Option<bool> {
        let flags = *self.term.mode();
        Some(match mode {
            Mode::AltScreen => flags.contains(TermMode::ALT_SCREEN),
            Mode::BracketedPaste => flags.contains(TermMode::BRACKETED_PASTE),
            Mode::MouseTracking => flags.intersects(TermMode::MOUSE_MODE),
            Mode::MouseSgr => flags.contains(TermMode::SGR_MOUSE),
            Mode::FocusReport => flags.contains(TermMode::FOCUS_IN_OUT),
            // Synchronized output lives in the processor, which buffers bytes until it ends.
            Mode::SyncOutput => self.processor.sync_bytes_count() > 0,
            Mode::KittyKeyboard => flags.intersects(TermMode::KITTY_KEYBOARD_PROTOCOL),
        })
    }
}
