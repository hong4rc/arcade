//! Candidate: a deliberately minimal self-written screen on top of the `vte` state machine.
//!
//! This is not a terminal emulator. It handles printing, line feeds, cursor addressing, erasing
//! and color changes only, with no scrollback. It exists to measure the cost floor of the
//! "write it ourselves" direction: a real implementation does strictly more work per byte, so it
//! cannot be faster than this.

use unicode_width::UnicodeWidthChar;
use vte::{Params, Parser, Perform};

use crate::candidate::{Candidate, CellStyle, Color, Event, Mode, Underline};

#[derive(Clone, Copy)]
struct Cell {
    c: char,
    fg: u32,
    bg: u32,
    attrs: u16,
}

const BLANK: Cell = Cell {
    c: ' ',
    fg: 0,
    bg: 0,
    attrs: 0,
};

struct Screen {
    cols: usize,
    rows: usize,
    cells: Vec<Cell>,
    x: usize,
    y: usize,
    fg: u32,
    bg: u32,
    attrs: u16,
    events: Vec<Event>,
}

impl Screen {
    fn scroll_up(&mut self) {
        self.cells.copy_within(self.cols.., 0);
        let start = (self.rows - 1) * self.cols;
        self.cells[start..].fill(BLANK);
    }

    fn line_feed(&mut self) {
        if self.y + 1 == self.rows {
            self.scroll_up()
        } else {
            self.y += 1
        }
    }
}

impl Perform for Screen {
    fn print(&mut self, c: char) {
        let width = c.width().unwrap_or(0);
        if width == 0 {
            return;
        }
        if self.x + width > self.cols {
            self.x = 0;
            self.line_feed();
        }
        let cell = Cell {
            c,
            fg: self.fg,
            bg: self.bg,
            attrs: self.attrs,
        };
        self.cells[self.y * self.cols + self.x] = cell;
        if width == 2 {
            self.cells[self.y * self.cols + self.x + 1] = Cell { c: '\0', ..cell };
        }
        self.x += width;
    }

    fn execute(&mut self, byte: u8) {
        match byte {
            b'\n' => self.line_feed(),
            b'\r' => self.x = 0,
            0x08 => self.x = self.x.saturating_sub(1),
            0x07 => self.events.push(Event::Bell),
            _ => {}
        }
    }

    fn osc_dispatch(&mut self, params: &[&[u8]], _bell_terminated: bool) {
        if let [code, title, ..] = params
            && (*code == b"0" || *code == b"2")
        {
            self.events
                .push(Event::Title(String::from_utf8_lossy(title).into_owned()));
        }
    }

    fn csi_dispatch(&mut self, params: &Params, intermediates: &[u8], _ignore: bool, action: char) {
        if !intermediates.is_empty() {
            return;
        }
        let mut values = params.iter().map(|p| p[0] as usize);
        match action {
            'H' | 'f' => {
                let row = values.next().unwrap_or(1).max(1);
                let col = values.next().unwrap_or(1).max(1);
                self.y = (row - 1).min(self.rows - 1);
                self.x = (col - 1).min(self.cols - 1);
            }
            'K' => {
                let start = self.y * self.cols + self.x;
                let end = (self.y + 1) * self.cols;
                self.cells[start..end].fill(BLANK);
            }
            'J' => self.cells.fill(BLANK),
            'm' => {
                // Enough color handling to do comparable work; not a complete implementation.
                let all: Vec<usize> = values.collect();
                let mut i = 0;
                while i < all.len() {
                    match all[i] {
                        0 => {
                            self.fg = 0;
                            self.bg = 0;
                            self.attrs = 0;
                        }
                        n @ 1..=9 => self.attrs |= 1 << n,
                        n @ (30..=37 | 90..=97) => self.fg = n as u32,
                        n @ (40..=47 | 100..=107) => self.bg = n as u32,
                        n @ (38 | 48) => {
                            let (value, skip) = match all.get(i + 1) {
                                Some(5) => (all.get(i + 2).copied().unwrap_or(0) as u32, 2),
                                Some(2) => {
                                    let part =
                                        |k: usize| all.get(i + k).copied().unwrap_or(0) as u32;
                                    (0x0100_0000 | part(2) << 16 | part(3) << 8 | part(4), 4)
                                }
                                _ => (0, 0),
                            };
                            if n == 38 {
                                self.fg = value
                            } else {
                                self.bg = value
                            }
                            i += skip;
                        }
                        _ => {}
                    }
                    i += 1;
                }
            }
            _ => {}
        }
    }
}

pub struct VteMinimal {
    parser: Parser,
    screen: Screen,
}

impl VteMinimal {
    pub fn new(cols: u16, rows: u16, _scrollback: usize) -> Self {
        let (cols, rows) = (cols as usize, rows as usize);
        VteMinimal {
            parser: Parser::new(),
            screen: Screen {
                cols,
                rows,
                cells: vec![BLANK; cols * rows],
                x: 0,
                y: 0,
                fg: 0,
                bg: 0,
                attrs: 0,
                events: Vec::new(),
            },
        }
    }
}

impl Candidate for VteMinimal {
    fn feed(&mut self, bytes: &[u8]) {
        self.parser.advance(&mut self.screen, bytes);
    }

    fn row_text(&self, row: u16) -> String {
        let start = row as usize * self.screen.cols;
        let text: String = self.screen.cells[start..start + self.screen.cols]
            .iter()
            .filter(|cell| cell.c != '\0')
            .map(|cell| cell.c)
            .collect();
        text.trim_end().to_string()
    }

    fn cursor(&self) -> (u16, u16) {
        (self.screen.x as u16, self.screen.y as u16)
    }

    fn drain_events(&mut self) -> Vec<Event> {
        std::mem::take(&mut self.screen.events)
    }

    fn style_at(&self, row: u16, col: u16) -> CellStyle {
        let cell = self.screen.cells[row as usize * self.screen.cols + col as usize];
        CellStyle {
            bold: cell.attrs & 2 != 0,
            italic: cell.attrs & 8 != 0,
            underline: if cell.attrs & 16 != 0 {
                Underline::Single
            } else {
                Underline::None
            },
            strike: cell.attrs & 512 != 0,
            inverse: cell.attrs & 128 != 0,
            fg: match cell.fg {
                0 => Color::Default,
                n @ 30..=37 => Color::Indexed(n as u8 - 30),
                n if n & 0x0100_0000 != 0 => Color::Rgb((n >> 16) as u8, (n >> 8) as u8, n as u8),
                n => Color::Indexed(n as u8),
            },
            hyperlink: if cell.bg == u32::MAX {
                Some(String::new())
            } else {
                None
            },
        }
    }

    fn mode(&self, _mode: Mode) -> Option<bool> {
        None
    }
}
