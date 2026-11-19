//! Putting what the client sees together: the screens of the panes that are showing, the dividers
//! between them and the status bar, in one grid of cells the size of the client's terminal.
//!
//! The grid keeps what was last drawn, and only the rows that were written to since the last
//! update are marked changed; the renderer then sends only those rows that really differ from
//! what the host terminal already shows.

use arcade_term::{Cell, CellWidth, Frame, LinkSpan, RowUpdate, Style};

use crate::layout::{Divider, Rect};
use crate::theme::{self, Role, Symbol};

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
    for unit in crate::width::units(text) {
        match unit.width {
            0 => {
                if let Some(last) = cells
                    .iter_mut()
                    .rev()
                    .find(|cell| cell.width != CellWidth::SpacerTail)
                {
                    last.text.push_str(unit.text);
                }
            }
            2 => {
                cells.push(Cell {
                    text: unit.text.to_string(),
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
                text: unit.text.to_string(),
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
    /// For each row, the runs of columns that belong to a link, left to right. Almost always
    /// empty; whatever is drawn over a link takes the link away from the cells it covers.
    links: Vec<Vec<LinkSpan>>,
    /// The underline given to the cells of a link that are not underlined already; `None` leaves
    /// them as the program drew them.
    link_underline: Option<Style>,
}

impl Screen {
    /// The style that marks the cells of a link, or none.
    pub fn set_link_underline(&mut self, style: Option<Style>) {
        self.link_underline = style;
    }

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
        self.links = vec![Vec::new(); usize::from(self.rows)];
    }

    /// The link runs of a row.
    pub fn row_links(&self, y: u16) -> &[LinkSpan] {
        &self.links[usize::from(y)]
    }

    /// Take the columns `from..to` of a row out of any link, cutting a run that reaches over
    /// them in two.
    fn unlink(spans: &mut Vec<LinkSpan>, from: usize, to: usize) {
        if spans.is_empty() {
            return;
        }
        let (from, to) = (from as u16, to.min(usize::from(u16::MAX)) as u16);
        let mut kept = Vec::with_capacity(spans.len() + 1);
        for span in spans.drain(..) {
            if span.to <= from || span.from >= to {
                kept.push(span);
                continue;
            }
            if span.from < from {
                kept.push(LinkSpan {
                    from: span.from,
                    to: from,
                    uri: span.uri.clone(),
                });
            }
            if span.to > to {
                kept.push(LinkSpan {
                    from: to,
                    to: span.to,
                    uri: span.uri,
                });
            }
        }
        *spans = kept;
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
        for RowUpdate { y, cells, links } in frame.rows {
            if y >= rect.h || rect.y + y >= self.rows {
                continue;
            }
            let gy = usize::from(rect.y + y);
            let row = &mut self.cells[gy];
            let start = usize::from(rect.x);
            let end = (start + usize::from(rect.w)).min(row.len());
            if start >= end {
                continue;
            }
            Self::unlink(&mut self.links[gy], start, end);
            let room = end - start;
            let mut source = cells.into_iter();
            for slot in &mut row[start..end] {
                *slot = source.next().unwrap_or_else(|| blank(Style::default()));
            }
            // A wide character cut off by the edge of the room would draw into the divider.
            let cut_wide = room > 0 && row[end - 1].width == CellWidth::Wide;
            if cut_wide {
                row[end - 1] = blank(Style::default());
            }
            // The links of the pane, moved to where the pane is and cut at the edge of its room.
            for span in links {
                let from = start + usize::from(span.from);
                let to = (start + usize::from(span.to)).min(end - usize::from(cut_wide));
                if from < to {
                    if let Some(mark) = self.link_underline {
                        for cell in &mut row[from..to] {
                            if cell.style.underline == arcade_term::Underline::None {
                                cell.style.underline = mark.underline;
                                cell.style.underline_color = mark.underline_color;
                            }
                        }
                    }
                    self.links[gy].push(LinkSpan {
                        from: from as u16,
                        to: to as u16,
                        uri: span.uri,
                    });
                }
            }
            self.links[gy].sort_by_key(|span| span.from);
            self.dirty[gy] = true;
        }
    }

    /// Draw the dividers (their coordinates are relative to a window area that starts at row
    /// `origin_y`). `focus` is the rectangle of the focused pane and `panes` the rectangles of
    /// every pane shown (both counted from the top of the screen). Every divider cell that
    /// touches the focused pane, along the whole of each of its edges and including the cells at
    /// the corners and where other dividers join, gets the focus colour. The one exception is a
    /// divider that has a single pane on each side over its whole length: the focus colour
    /// would be the same whichever of the two has the focus, so it marks only the half on the
    /// focused pane's side (the upper or left half for the pane before it).
    ///
    /// `marked` are panes out of focus that carry a state, each with the style its edges take,
    /// most urgent first; they follow the same rules as the focused pane, and where a cell would
    /// be claimed by both the focused pane and a marked one, the marked one wins.
    pub fn draw_dividers(
        &mut self,
        dividers: &[Divider],
        origin_y: u16,
        focus: Option<Rect>,
        panes: &[Rect],
        marked: &[(Rect, Style)],
    ) {
        // Every cell that holds a divider, and whether its line runs top to bottom, so that a
        // cell where two dividers meet can be drawn with the sign that joins them.
        let mut line_cells: std::collections::HashMap<(u16, u16), bool> =
            std::collections::HashMap::new();
        for d in dividers {
            for i in 0..d.len {
                let at = if d.vertical {
                    (d.x, d.y + i)
                } else {
                    (d.x + i, d.y)
                };
                line_cells.insert(at, d.vertical);
            }
        }
        let vertical_at = |x: u16, y: u16| line_cells.get(&(x, y)) == Some(&true);
        let horizontal_at = |x: u16, y: u16| line_cells.get(&(x, y)) == Some(&false);
        // The rectangles in the coordinates of the window area.
        let up = |r: Rect| Rect::new(r.x, r.y.saturating_sub(origin_y), r.w, r.h);
        let focus = focus.map(up);
        let marked: Vec<(Rect, Style)> = marked.iter().map(|(r, st)| (up(*r), *st)).collect();
        let panes: Vec<Rect> = panes.iter().copied().map(up).collect();
        // The cells of divider `o` that lie on the edge of pane `r`, corners included.
        let touches_edge = |o: &Divider, r: Rect| {
            (0..o.len).any(|j| {
                let (x, y) = if o.vertical {
                    (i32::from(o.x), i32::from(o.y + j))
                } else {
                    (i32::from(o.x + j), i32::from(o.y))
                };
                x >= i32::from(r.x) - 1
                    && x <= i32::from(r.x + r.w)
                    && y >= i32::from(r.y) - 1
                    && y <= i32::from(r.y + r.h)
            })
        };
        for (di, d) in dividers.iter().enumerate() {
            // The panes on each side of the divider, over its whole length.
            let side = |dx: i32, dy: i32| {
                let mut found: Vec<Option<usize>> = Vec::new();
                for i in 0..d.len {
                    let (x, y) = if d.vertical {
                        (i32::from(d.x), i32::from(d.y + i))
                    } else {
                        (i32::from(d.x + i), i32::from(d.y))
                    };
                    let at = panes.iter().position(|r| {
                        let (px, py) = (x + dx, y + dy);
                        px >= i32::from(r.x)
                            && px < i32::from(r.x + r.w)
                            && py >= i32::from(r.y)
                            && py < i32::from(r.y + r.h)
                    });
                    if !found.contains(&at) {
                        found.push(at);
                    }
                }
                found
            };
            let (before_side, after_side) = if d.vertical {
                (side(-1, 0), side(1, 0))
            } else {
                (side(0, -1), side(0, 1))
            };
            // Halves only when the two panes touch no divider but this one: colouring whole edges
            // would then give both panes the same cells along this stretch.
            let alone = |side: &[Option<usize>]| match side {
                [Some(p)] => !dividers
                    .iter()
                    .enumerate()
                    .any(|(k, o)| k != di && touches_edge(o, panes[*p])),
                _ => false,
            };
            let between_two = alone(&before_side) && alone(&after_side);
            for i in 0..d.len {
                let (x, y) = if d.vertical {
                    (d.x, d.y + i)
                } else {
                    (d.x + i, d.y)
                };
                let touches = |f: Rect| {
                    if between_two {
                        let (before, after) = if d.vertical {
                            (f.contains(x.wrapping_sub(1), y), f.contains(x + 1, y))
                        } else {
                            (f.contains(x, y.wrapping_sub(1)), f.contains(x, y + 1))
                        };
                        (before && i < d.len.div_ceil(2)) || (after && i >= d.len / 2)
                    } else {
                        // The ring of cells around the pane, corners included.
                        let (cx, cy) = (i32::from(x), i32::from(y));
                        let inside = f.contains(x, y);
                        !inside
                            && cx >= i32::from(f.x) - 1
                            && cx <= i32::from(f.x + f.w)
                            && cy >= i32::from(f.y) - 1
                            && cy <= i32::from(f.y + f.h)
                    }
                };
                let cell_style = marked
                    .iter()
                    .find(|(r, _)| touches(*r))
                    .map(|(_, st)| *st)
                    .unwrap_or_else(|| {
                        theme::style(if focus.is_some_and(touches) {
                            Role::DividerFocus
                        } else {
                            Role::Divider
                        })
                    });
                let (left, right) = (horizontal_at(x.wrapping_sub(1), y), horizontal_at(x + 1, y));
                let (up, down) = (vertical_at(x, y.wrapping_sub(1)), vertical_at(x, y + 1));
                let sym = if d.vertical {
                    match (left, right) {
                        (true, true) => Symbol::DividerCross,
                        (false, true) => Symbol::DividerJoinRight,
                        (true, false) => Symbol::DividerJoinLeft,
                        _ => Symbol::DividerVertical,
                    }
                } else {
                    match (up, down) {
                        (true, true) => Symbol::DividerCross,
                        (false, true) => Symbol::DividerJoinDown,
                        (true, false) => Symbol::DividerJoinUp,
                        _ => Symbol::DividerHorizontal,
                    }
                };
                let gy = y + origin_y;
                if x >= self.cols || gy >= self.rows {
                    continue;
                }
                Self::unlink(
                    &mut self.links[usize::from(gy)],
                    usize::from(x),
                    usize::from(x) + 1,
                );
                self.cells[usize::from(gy)][usize::from(x)] = Cell {
                    text: theme::symbol(sym).to_string(),
                    width: CellWidth::Narrow,
                    style: cell_style,
                };
                self.dirty[usize::from(gy)] = true;
            }
        }
    }

    /// Draw cells over the grid with their top left corner at (`x`, `y`), cut at the edges.
    pub fn overlay(&mut self, x: u16, y: u16, rows: &[Vec<Cell>]) {
        for (i, cells) in rows.iter().enumerate() {
            let gy = usize::from(y) + i;
            if gy >= usize::from(self.rows) {
                break;
            }
            let reach = (usize::from(x) + cells.len()).min(usize::from(self.cols));
            Self::unlink(&mut self.links[gy], usize::from(x), reach);
            let row = &mut self.cells[gy];
            for (j, cell) in cells.iter().enumerate() {
                let gx = usize::from(x) + j;
                if gx >= row.len() {
                    break;
                }
                row[gx] = cell.clone();
            }
            // A wide character cut off at the right edge would draw outside.
            if let Some(last) = row.last_mut()
                && last.width == CellWidth::Wide
            {
                *last = blank(Style::default());
            }
            self.dirty[gy] = true;
        }
    }

    /// Replace a whole row.
    pub fn set_row(&mut self, y: u16, cells: Vec<Cell>) {
        if y >= self.rows {
            return;
        }
        let cells = fit(cells, usize::from(self.cols), Style::default());
        self.cells[usize::from(y)] = cells;
        self.links[usize::from(y)].clear();
        self.dirty[usize::from(y)] = true;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The columns the pane's terminal engine gives a text: the cursor's place after printing it
    /// on an empty line.
    fn pane_columns(text: &str) -> usize {
        let mut term = arcade_term::Term::new(arcade_term::TermConfig::new(20, 3)).unwrap();
        term.feed(text.as_bytes());
        usize::from(term.snapshot(true).unwrap().cursor.x)
    }

    /// A name on the bar must be as wide there as in a pane, or the bar ends a cell out. The pane
    /// (the terminal engine, which is what Ghostty does) is the reference.
    #[test]
    fn the_bar_gives_a_character_the_width_the_pane_gives_it() {
        let texts = [
            "a",
            "\u{2665}",
            "\u{2764}",
            "\u{2605}",
            "\u{2022}",
            "\u{2192}",
            "\u{25cf}",
            "\u{25b6}",
            "\u{2500}",
            "\u{e0b6}",
            "\u{e795}",
            "\u{2b50}",
            "\u{26a1}",
            "\u{1f600}",
            "\u{4e2d}",
            "\u{ff21}",
            "e\u{301}",
            "\u{231a}",
            "\u{23e9}",
            "\u{2614}",
            "\u{2705}",
            "\u{274c}",
            "\u{1f680}",
        ];
        assert_eq!(disagreements(&texts), Vec::<String>::new());
    }

    /// The texts of `texts` whose width on the bar is not the width in a pane.
    fn disagreements(texts: &[&str]) -> Vec<String> {
        texts
            .iter()
            .filter_map(|text| {
                let bar = text_cells(text, Style::default()).len();
                let pane = pane_columns(text);
                (bar != pane).then(|| format!("{text:?}: bar {bar}, pane {pane}"))
            })
            .collect()
    }

    /// A sequence the engine joins into one cluster is as wide on the bar as in the pane, by the
    /// few rules of `width::units`.
    #[test]
    fn the_bar_gives_a_sequence_the_width_the_pane_gives_it() {
        let texts = [
            "\u{2665}\u{fe0f}",
            "\u{2764}\u{fe0f}",
            "\u{2764}\u{fe0e}",
            "1\u{fe0f}",
            "a\u{fe0f}",
            "\u{1f468}\u{200d}\u{1f469}",
            "\u{1f468}\u{200d}\u{1f469}\u{200d}\u{1f467}",
            "\u{1f44d}\u{1f3fd}",
            "\u{270b}\u{1f3fd}",
            "\u{1f1fa}\u{1f1f8}",
            "\u{1f1fa}\u{1f1f8}\u{1f1ec}",
            "\u{1f1fa}\u{1f1f8}\u{1f1ec}\u{1f1e7}",
            "\u{1f469}\u{200d}\u{2764}\u{fe0f}\u{200d}\u{1f468}",
            "a\u{200d}b",
            "\u{1f469}\u{200d}a",
        ];
        assert_eq!(disagreements(&texts), Vec::<String>::new());
    }

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

    /// The dividers as rows of text: `#` for a cell with the focus colour, `.` for any other
    /// divider cell, a blank for the rest.
    fn map(screen: &Screen, w: u16, h: u16) -> Vec<String> {
        let focus = theme::style(Role::DividerFocus);
        let plain = theme::style(Role::Divider);
        (0..h)
            .map(|y| {
                (0..w)
                    .map(|x| {
                        let style = screen.row(y)[usize::from(x)].style;
                        if style == focus {
                            '#'
                        } else if style == plain {
                            '.'
                        } else {
                            ' '
                        }
                    })
                    .collect()
            })
            .collect()
    }

    fn vertical(x: u16, y: u16, len: u16) -> Divider {
        Divider {
            x,
            y,
            len,
            vertical: true,
        }
    }

    fn horizontal(x: u16, y: u16, len: u16) -> Divider {
        Divider {
            x,
            y,
            len,
            vertical: false,
        }
    }

    /// Draw `dividers` with `focus` among `panes` on a window of 11 by `h` cells.
    fn drawn(h: u16, dividers: &[Divider], panes: &[Rect], focus: usize) -> Vec<String> {
        let mut screen = Screen::new();
        screen.resize(11, h);
        screen.draw_dividers(dividers, 0, Some(panes[focus]), panes, &[]);
        map(&screen, 11, h)
    }

    #[test]
    fn between_two_panes_each_edge_is_marked_on_the_half_of_the_pane_in_focus() {
        // Two panes side by side: the left one marks the upper half, the right one the lower.
        let side = [Rect::new(0, 0, 5, 6), Rect::new(6, 0, 5, 6)];
        let line = [vertical(5, 0, 6)];
        assert_eq!(
            drawn(6, &line, &side, 0),
            [
                "     #     ",
                "     #     ",
                "     #     ",
                "     .     ",
                "     .     ",
                "     .     "
            ]
        );
        assert_eq!(
            drawn(6, &line, &side, 1),
            [
                "     .     ",
                "     .     ",
                "     .     ",
                "     #     ",
                "     #     ",
                "     #     "
            ]
        );
        // Two panes one above the other: the upper one marks the left half, the lower the right.
        let stacked = [Rect::new(0, 0, 11, 3), Rect::new(0, 4, 11, 3)];
        let line = [horizontal(0, 3, 11)];
        assert_eq!(drawn(7, &line, &stacked, 0)[3], "######.....");
        assert_eq!(drawn(7, &line, &stacked, 1)[3], ".....######");
    }

    /// Like `map`, with `N` for a cell in the style of a notified pane.
    fn map_with_notice(screen: &Screen, w: u16, h: u16, notice: Style) -> Vec<String> {
        let focus = theme::style(Role::DividerFocus);
        (0..h)
            .map(|y| {
                (0..w)
                    .map(|x| {
                        let style = screen.row(y)[usize::from(x)].style;
                        if style == notice {
                            'N'
                        } else if style == focus {
                            '#'
                        } else {
                            '.'
                        }
                    })
                    .collect()
            })
            .collect()
    }

    #[test]
    fn a_notified_pane_out_of_focus_gets_its_edges_in_the_colour_of_its_state() {
        let notice = theme::divider_state_style(theme::Tint::Class(crate::notify::Class::Input));
        assert_ne!(notice, theme::style(Role::DividerFocus));
        assert_ne!(notice, theme::style(Role::Divider));
        // Two panes side by side, the right one notified, the left one in focus: each marks its
        // own half of the one line between them.
        let side = [Rect::new(0, 0, 5, 6), Rect::new(6, 0, 5, 6)];
        let mut screen = Screen::new();
        screen.resize(11, 6);
        screen.draw_dividers(
            &[vertical(5, 0, 6)],
            0,
            Some(side[0]),
            &side,
            &[(side[1], notice)],
        );
        let column: String = map_with_notice(&screen, 11, 6, notice)
            .iter()
            .map(|row| row.chars().nth(5).unwrap())
            .collect();
        assert_eq!(column, "###NNN");
        // With no pane in focus the notified pane's half is the only one coloured.
        let mut screen = Screen::new();
        screen.resize(11, 6);
        screen.draw_dividers(&[vertical(5, 0, 6)], 0, None, &side, &[(side[1], notice)]);
        let column: String = map_with_notice(&screen, 11, 6, notice)
            .iter()
            .map(|row| row.chars().nth(5).unwrap())
            .collect();
        assert_eq!(column, "...NNN");
    }

    #[test]
    fn where_the_focused_pane_and_a_notified_pane_claim_a_cell_the_notification_wins() {
        let notice = theme::divider_state_style(theme::Tint::Class(crate::notify::Class::Error));
        let (dividers, panes) = three();
        // The tall pane on the left is in focus and the upper right pane is notified: the cells
        // that both touch (the vertical line down to the corner) are the notified pane's.
        let mut screen = Screen::new();
        screen.resize(11, 7);
        screen.draw_dividers(&dividers, 0, Some(panes[0]), &panes, &[(panes[1], notice)]);
        assert_eq!(
            map_with_notice(&screen, 11, 7, notice),
            [
                ".....N.....",
                ".....N.....",
                ".....N.....",
                ".....NNNNNN",
                ".....#.....",
                ".....#.....",
                ".....#....."
            ]
        );
    }

    /// One pane on the left over the whole height, two on the right, one above the other.
    fn three() -> ([Divider; 2], [Rect; 3]) {
        (
            [vertical(5, 0, 7), horizontal(6, 3, 5)],
            [
                Rect::new(0, 0, 5, 7),
                Rect::new(6, 0, 5, 3),
                Rect::new(6, 4, 5, 3),
            ],
        )
    }

    #[test]
    fn with_three_panes_the_whole_edge_of_the_pane_in_focus_is_marked() {
        let (dividers, panes) = three();
        // The tall pane on the left: all of its right edge.
        assert_eq!(
            drawn(7, &dividers, &panes, 0),
            [
                "     #     ",
                "     #     ",
                "     #     ",
                "     #.....",
                "     #     ",
                "     #     ",
                "     #     "
            ]
        );
        // The upper right pane: all of its left edge down to the corner where the divider
        // under it joins, and all of the divider under it. That divider has one pane on each
        // side, but the upper right pane also touches the divider on its left, so the whole
        // stretch is marked, not a half.
        assert_eq!(
            drawn(7, &dividers, &panes, 1),
            [
                "     #     ",
                "     #     ",
                "     #     ",
                "     ######",
                "     .     ",
                "     .     ",
                "     .     "
            ]
        );
        // The lower right pane: its left edge from the corner down, and the whole divider
        // above it, for the same reason.
        assert_eq!(
            drawn(7, &dividers, &panes, 2),
            [
                "     .     ",
                "     .     ",
                "     .     ",
                "     ######",
                "     #     ",
                "     #     ",
                "     #     "
            ]
        );
    }

    #[test]
    fn in_a_grid_of_four_the_pane_in_focus_marks_its_edges_to_the_crossing() {
        // The window is split into a row above and a row below, each split in two.
        let dividers = [horizontal(0, 3, 11), vertical(5, 0, 3), vertical(5, 4, 3)];
        let panes = [
            Rect::new(0, 0, 5, 3),
            Rect::new(6, 0, 5, 3),
            Rect::new(0, 4, 5, 3),
            Rect::new(6, 4, 5, 3),
        ];
        // The upper left pane touches the divider beside it and the divider below it, so both
        // are marked along their whole edge, the corner included.
        assert_eq!(
            drawn(7, &dividers, &panes, 0),
            [
                "     #     ",
                "     #     ",
                "     #     ",
                "######.....",
                "     .     ",
                "     .     ",
                "     .     "
            ]
        );
        // The lower right pane: the whole divider above it and the whole divider beside it.
        assert_eq!(
            drawn(7, &dividers, &panes, 3),
            [
                "     .     ",
                "     .     ",
                "     .     ",
                ".....######",
                "     #     ",
                "     #     ",
                "     #     "
            ]
        );
    }

    #[test]
    fn in_a_nested_layout_a_divider_with_several_panes_on_one_side_is_marked_along_the_pane() {
        // A pane on the left, and on the right one above two side by side.
        let dividers = [vertical(5, 0, 7), horizontal(6, 3, 5), vertical(8, 4, 3)];
        let panes = [
            Rect::new(0, 0, 5, 7),
            Rect::new(6, 0, 5, 3),
            Rect::new(6, 4, 2, 3),
            Rect::new(9, 4, 2, 3),
        ];
        // The lower right pane: along its top edge (the divider above has the upper pane on one
        // side and two on the other), and the whole divider beside it, because the pane on its
        // left touches the divider above as well.
        assert_eq!(
            drawn(7, &dividers, &panes, 3),
            [
                "     .     ",
                "     .     ",
                "     .     ",
                "     ...###",
                "     .  #  ",
                "     .  #  ",
                "     .  #  "
            ]
        );
        // The upper right pane: its whole left edge and the corner, and its whole edge below,
        // which is the part of the divider between it and the two panes.
        assert_eq!(
            drawn(7, &dividers, &panes, 1),
            [
                "     #     ",
                "     #     ",
                "     #     ",
                "     ######",
                "     .  .  ",
                "     .  .  ",
                "     .  .  "
            ]
        );
    }

    #[test]
    fn a_top_bar_does_not_shift_which_cells_touch_the_focused_pane() {
        let mut screen = Screen::new();
        screen.resize(11, 7);
        // The window area starts at row 1; the divider is in its rows 0 to 5 (screen rows 1
        // to 6) and the focused pane is the left one, over the whole height.
        let dividers = [Divider {
            x: 5,
            y: 0,
            len: 6,
            vertical: true,
        }];
        screen.draw_dividers(
            &dividers,
            1,
            Some(Rect::new(0, 1, 5, 6)),
            &[Rect::new(0, 1, 5, 6), Rect::new(6, 1, 5, 6)],
            &[],
        );
        assert_eq!(
            map(&screen, 11, 7)[1..],
            [
                "     #     ",
                "     #     ",
                "     #     ",
                "     .     ",
                "     .     ",
                "     .     "
            ]
        );
    }
}

#[cfg(test)]
mod link_tests {
    use super::*;

    fn link(from: u16, to: u16) -> LinkSpan {
        LinkSpan {
            from,
            to,
            uri: "u".into(),
        }
    }

    fn row_update(links: Vec<LinkSpan>) -> RowUpdate {
        RowUpdate {
            y: 0,
            cells: text_cells("0123456789", Style::default()),
            links,
        }
    }

    fn frame(links: Vec<LinkSpan>) -> Frame {
        Frame {
            cols: 10,
            rows_total: 1,
            damage: arcade_term::Damage::Full,
            rows: vec![row_update(links)],
            cursor: arcade_term::Cursor {
                x: 0,
                y: 0,
                visible: false,
                shape: arcade_term::CursorShape::Block,
                blinking: false,
            },
        }
    }

    fn spans(screen: &Screen) -> Vec<(u16, u16)> {
        screen.row_links(0).iter().map(|s| (s.from, s.to)).collect()
    }

    #[test]
    fn a_pane_s_links_move_with_the_pane_and_are_cut_at_its_edge() {
        let mut screen = Screen::new();
        screen.resize(20, 1);
        screen.blit(Rect::new(5, 0, 6, 1), frame(vec![link(1, 3), link(4, 10)]));
        assert_eq!(spans(&screen), [(6, 8), (9, 11)]);
    }

    #[test]
    fn a_pane_drawn_again_replaces_its_links_and_leaves_the_neighbour_s() {
        let mut screen = Screen::new();
        screen.resize(20, 1);
        screen.blit(Rect::new(0, 0, 10, 1), frame(vec![link(0, 2)]));
        screen.blit(Rect::new(10, 0, 10, 1), frame(vec![link(0, 2)]));
        screen.blit(Rect::new(10, 0, 10, 1), frame(vec![]));
        assert_eq!(spans(&screen), [(0, 2)]);
    }

    #[test]
    fn links_stay_in_order_when_the_right_pane_is_drawn_first() {
        let mut screen = Screen::new();
        screen.resize(20, 1);
        screen.blit(Rect::new(10, 0, 10, 1), frame(vec![link(0, 2)]));
        screen.blit(Rect::new(0, 0, 10, 1), frame(vec![link(0, 2)]));
        assert_eq!(spans(&screen), [(0, 2), (10, 12)]);
    }

    #[test]
    fn a_box_drawn_over_a_link_cuts_it() {
        let mut screen = Screen::new();
        screen.resize(10, 1);
        screen.blit(Rect::new(0, 0, 10, 1), frame(vec![link(0, 10)]));
        screen.overlay(3, 0, &[text_cells("xxx", Style::default())]);
        assert_eq!(spans(&screen), [(0, 3), (6, 10)]);
        screen.overlay(0, 0, &[text_cells("yyyy", Style::default())]);
        assert_eq!(spans(&screen), [(6, 10)]);
    }

    #[test]
    fn a_divider_or_a_whole_row_drawn_over_a_link_takes_it_away() {
        let mut screen = Screen::new();
        screen.resize(10, 1);
        screen.blit(Rect::new(0, 0, 10, 1), frame(vec![link(0, 10)]));
        let divider = Divider {
            x: 4,
            y: 0,
            len: 1,
            vertical: true,
        };
        screen.draw_dividers(&[divider], 0, None, &[], &[]);
        assert_eq!(spans(&screen), [(0, 4), (5, 10)]);
        screen.set_row(0, text_cells("status", Style::default()));
        assert!(spans(&screen).is_empty());
    }
}
