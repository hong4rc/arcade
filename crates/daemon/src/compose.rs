//! Putting what the client sees together: the screens of the panes that are showing, the dividers
//! between them and the status bar, in one grid of cells the size of the client's terminal.
//!
//! The grid keeps what was last drawn, and only the rows that were written to since the last
//! update are marked changed; the renderer then sends only those rows that really differ from
//! what the host terminal already shows.

use arcade_term::{Cell, CellWidth, Frame, RowUpdate, Style};

use crate::layout::{Divider, Rect};
use crate::theme::{self, Role, Symbol};

/// How many columns a character takes on screen: 0 for combining marks, 2 for wide characters.
pub fn char_width(c: char) -> usize {
    let n = c as u32;
    match n {
        0x0300..=0x036F | 0x1AB0..=0x1AFF | 0x1DC0..=0x1DFF | 0x20D0..=0x20FF => 0,
        0xFE00..=0xFE0F | 0x200B..=0x200F | 0xFE20..=0xFE2F => 0,
        0x1100..=0x115F
        | 0x2E80..=0xA4CF
        | 0xAC00..=0xD7A3
        | 0xF900..=0xFAFF
        | 0xFE30..=0xFE6F
        | 0xFF00..=0xFF60
        | 0xFFE0..=0xFFE6
        | 0x1F300..=0x1F64F
        | 0x1F900..=0x1F9FF
        | 0x20000..=0x3FFFD => 2,
        _ => 1,
    }
}

pub fn blank(style: Style) -> Cell {
    Cell {
        text: String::new(),
        width: CellWidth::Narrow,
        style,
    }
}

/// The cells of a piece of text: a wide character takes two (the second draws nothing) and a
/// combining mark joins the cell before it.
pub fn text_cells(text: &str, style: Style) -> Vec<Cell> {
    let mut cells: Vec<Cell> = Vec::new();
    for c in text.chars() {
        match char_width(c) {
            0 => {
                if let Some(last) = cells
                    .iter_mut()
                    .rev()
                    .find(|cell| cell.width != CellWidth::SpacerTail)
                {
                    last.text.push(c);
                }
            }
            2 => {
                cells.push(Cell {
                    text: c.to_string(),
                    width: CellWidth::Wide,
                    style,
                });
                cells.push(Cell {
                    text: String::new(),
                    width: CellWidth::SpacerTail,
                    style,
                });
            }
            _ => cells.push(Cell {
                text: c.to_string(),
                width: CellWidth::Narrow,
                style,
            }),
        }
    }
    cells
}

/// Cut `cells` to `width` columns, or pad with blanks of `pad` style up to it. A wide character
/// that would be cut in half is replaced by a blank.
pub fn fit(mut cells: Vec<Cell>, width: usize, pad: Style) -> Vec<Cell> {
    if cells.len() > width {
        cells.truncate(width);
        if cells.last().is_some_and(|c| c.width == CellWidth::Wide) {
            let last = cells.len() - 1;
            cells[last] = blank(pad);
        }
    }
    while cells.len() < width {
        cells.push(blank(pad));
    }
    cells
}

/// The grid of cells.
#[derive(Default)]
pub struct Screen {
    cols: u16,
    rows: u16,
    cells: Vec<Vec<Cell>>,
    dirty: Vec<bool>,
}

impl Screen {
    pub fn new() -> Screen {
        Screen::default()
    }

    pub fn rows(&self) -> u16 {
        self.rows
    }

    /// Make the grid `cols` by `rows`, blank, with every row changed.
    pub fn resize(&mut self, cols: u16, rows: u16) {
        self.cols = cols;
        self.rows = rows;
        self.clear();
    }

    /// Blank every cell and mark every row changed.
    pub fn clear(&mut self) {
        let row = vec![blank(Style::default()); usize::from(self.cols)];
        self.cells = vec![row; usize::from(self.rows)];
        self.dirty = vec![true; usize::from(self.rows)];
    }

    pub fn row(&self, y: u16) -> &[Cell] {
        &self.cells[usize::from(y)]
    }

    /// Whether the row changed since the last call; clears the mark.
    pub fn take_dirty(&mut self, y: u16) -> bool {
        std::mem::take(&mut self.dirty[usize::from(y)])
    }

    /// Write the rows a pane's terminal reported into the room `rect` (in grid coordinates).
    pub fn blit(&mut self, rect: Rect, frame: Frame) {
        for RowUpdate { y, cells } in frame.rows {
            if y >= rect.h || rect.y + y >= self.rows {
                continue;
            }
            let row = &mut self.cells[usize::from(rect.y + y)];
            let start = usize::from(rect.x);
            let end = (start + usize::from(rect.w)).min(row.len());
            if start >= end {
                continue;
            }
            let room = end - start;
            let mut source = cells.into_iter();
            for slot in &mut row[start..end] {
                *slot = source.next().unwrap_or_else(|| blank(Style::default()));
            }
            // A wide character cut off by the edge of the room would draw into the divider.
            if room > 0 && row[end - 1].width == CellWidth::Wide {
                row[end - 1] = blank(Style::default());
            }
            self.dirty[usize::from(rect.y + y)] = true;
        }
    }

    /// Draw the dividers (their coordinates are relative to a window area that starts at row
    /// `origin_y`). The cells that touch the focused pane's room get the focus color.
    pub fn draw_dividers(&mut self, dividers: &[Divider], origin_y: u16, focus: Option<Rect>) {
        for d in dividers {
            for i in 0..d.len {
                let (x, y) = if d.vertical {
                    (d.x, d.y + i)
                } else {
                    (d.x + i, d.y)
                };
                let touches = focus.is_some_and(|f| {
                    if d.vertical {
                        f.contains(x.wrapping_sub(1), y) || f.contains(x + 1, y)
                    } else {
                        f.contains(x, y.wrapping_sub(1)) || f.contains(x, y + 1)
                    }
                });
                let role = if touches {
                    Role::DividerFocus
                } else {
                    Role::Divider
                };
                let sym = if d.vertical {
                    Symbol::DividerVertical
                } else {
                    Symbol::DividerHorizontal
                };
                let gy = y + origin_y;
                if x >= self.cols || gy >= self.rows {
                    continue;
                }
                self.cells[usize::from(gy)][usize::from(x)] = Cell {
                    text: theme::symbol(sym).to_string(),
                    width: CellWidth::Narrow,
                    style: theme::style(role),
                };
                self.dirty[usize::from(gy)] = true;
            }
        }
    }

    /// Replace a whole row.
    pub fn set_row(&mut self, y: u16, cells: Vec<Cell>) {
        if y >= self.rows {
            return;
        }
        let cells = fit(cells, usize::from(self.cols), Style::default());
        self.cells[usize::from(y)] = cells;
        self.dirty[usize::from(y)] = true;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn wide_and_combining_characters_take_the_right_columns() {
        assert_eq!(text_cells("abc", Style::default()).len(), 3);
        // Two wide characters: four columns.
        assert_eq!(text_cells("漢字", Style::default()).len(), 4);
        // A letter and a combining accent: one column, one cell holding both.
        let cells = text_cells("e\u{301}", Style::default());
        assert_eq!(cells.len(), 1);
        assert_eq!(cells[0].text, "e\u{301}");
    }

    #[test]
    fn fitting_cuts_and_pads_without_splitting_a_wide_character() {
        let cells = fit(text_cells("a漢", Style::default()), 2, Style::default());
        assert_eq!(cells.len(), 2);
        assert!(cells[1].text.is_empty());
        let cells = fit(text_cells("ab", Style::default()), 5, Style::default());
        assert_eq!(cells.len(), 5);
    }

    #[test]
    fn only_the_dividers_that_touch_the_focused_pane_get_the_focus_color() {
        let mut screen = Screen::new();
        screen.resize(11, 6);
        // A column divider at x = 5, rows 0 to 5; the focused pane is the left one, but only
        // for the upper three rows.
        let dividers = [Divider {
            x: 5,
            y: 0,
            len: 6,
            vertical: true,
        }];
        screen.draw_dividers(&dividers, 0, Some(Rect::new(0, 0, 5, 3)));
        let focus = theme::style(Role::DividerFocus);
        let plain = theme::style(Role::Divider);
        for y in 0..3 {
            assert_eq!(screen.row(y)[5].style, focus, "row {y}");
        }
        for y in 3..6 {
            assert_eq!(screen.row(y)[5].style, plain, "row {y}");
        }
    }
}
