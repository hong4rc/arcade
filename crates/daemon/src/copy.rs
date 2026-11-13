//! Copy mode: where the cursor is, what is selected, searching, and the text that comes out.
//!
//! This file only computes. It reads the rows of a pane through [`Source`] (row 0 is the oldest
//! line of the scrollback) and changes nothing of the pane, so every command can be tried on a
//! small screen made of a few strings.

use arcade_term::{Cell, CellWidth, Line, Style};

use crate::commands::CopyCommand;
use crate::compose::blank;
use crate::theme::{self, Role};

/// The rows copy mode reads.
pub trait Source {
    /// Rows in all: the scrollback and the visible rows.
    fn total(&self) -> usize;
    fn line(&self, y: usize) -> Option<Line>;
}

impl Source for arcade_term::Term {
    fn total(&self) -> usize {
        self.total_rows()
    }

    fn line(&self, y: usize) -> Option<Line> {
        arcade_term::Term::line(self, y)
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SelectionKind {
    /// From the anchor to the cursor, character by character.
    Char,
    /// Whole lines, a line that was broken over several rows counted as one.
    Line,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Selection {
    pub kind: SelectionKind,
    pub row: usize,
    pub col: u16,
}

/// What a command asks of the code that called it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Outcome {
    /// Nothing but the view (or the cursor) changed.
    Moved,
    /// Say something on the status bar.
    Message(String),
    /// Ask for the text to search for, then call [`Copy::search`].
    AskSearch { forward: bool },
    /// Copy this text; leave copy mode afterwards unless `stay`.
    Yank { text: String, stay: bool },
    /// Leave copy mode without copying.
    Exit,
}

pub struct Copy {
    pub row: usize,
    pub col: u16,
    /// The column vertical moves aim for, so that passing a short line loses nothing.
    want: u16,
    /// The first row shown.
    pub top: usize,
    pub cols: u16,
    pub rows: u16,
    pub selection: Option<Selection>,
    /// The last thing searched for, and whether the search went down.
    pub search: Option<(String, bool)>,
    /// The view must be drawn again.
    pub dirty: bool,
}

fn is_blank(cell: &Cell) -> bool {
    cell.text.is_empty() || cell.text == " "
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum Class {
    Blank,
    Word,
    Punct,
}

fn class(cell: &Cell) -> Class {
    match cell.text.chars().next() {
        None | Some(' ') => Class::Blank,
        Some(c) if c.is_alphanumeric() || c == '_' => Class::Word,
        Some(_) => Class::Punct,
    }
}

fn blank_line(cols: u16) -> Line {
    Line {
        cells: vec![blank(Style::default()); usize::from(cols)],
        continues: false,
    }
}

/// The text of some cells: blanks as spaces, the second half of a wide character as nothing.
fn cells_text(cells: &[Cell]) -> String {
    let mut text = String::new();
    for cell in cells {
        if cell.width == CellWidth::SpacerTail {
            continue;
        }
        if cell.text.is_empty() {
            text.push(' ');
        } else {
            text.push_str(&cell.text);
        }
    }
    text
}

/// Where a wide character begins when `col` is its second half.
fn snap(line: &Line, col: u16) -> u16 {
    let mut col = usize::from(col).min(line.cells.len().saturating_sub(1));
    while col > 0 && line.cells[col].width == CellWidth::SpacerTail {
        col -= 1;
    }
    col as u16
}

fn last_filled(line: &Line) -> Option<u16> {
    line.cells
        .iter()
        .rposition(|c| !is_blank(c))
        .map(|i| snap(line, i as u16))
}

/// A cell-by-cell walk over the rows, for the moves by word.
struct Walk<'a, S: Source> {
    src: &'a S,
    cols: u16,
    row: usize,
    col: u16,
    line: Line,
    /// The last step crossed the end of a line (not just the end of a row of a broken line).
    broke: bool,
}

impl<'a, S: Source> Walk<'a, S> {
    fn new(src: &'a S, cols: u16, row: usize, col: u16) -> Self {
        let line = src.line(row).unwrap_or_else(|| blank_line(cols));
        Walk {
            src,
            cols,
            row,
            col,
            line,
            broke: false,
        }
    }

    fn class(&self) -> Class {
        self.line
            .cells
            .get(usize::from(self.col))
            .map_or(Class::Blank, class)
    }

    fn goto(&mut self, row: usize, col: u16) {
        if row != self.row {
            self.row = row;
            self.line = self.src.line(row).unwrap_or_else(|| blank_line(self.cols));
        }
        self.col = col;
    }

    fn step_next(&mut self) -> bool {
        self.broke = false;
        if self.col + 1 < self.cols {
            self.col += 1;
        } else if self.row + 1 < self.src.total() {
            let row = self.row + 1;
            self.goto(row, 0);
            self.broke = !self.line.continues;
        } else {
            return false;
        }
        true
    }

    fn step_prev(&mut self) -> bool {
        self.broke = false;
        if self.col > 0 {
            self.col -= 1;
        } else if self.row > 0 {
            self.broke = !self.line.continues;
            let row = self.row - 1;
            self.goto(row, self.cols - 1);
        } else {
            return false;
        }
        true
    }

    /// One cell on, never stopping on the second half of a wide character.
    fn next(&mut self) -> bool {
        loop {
            if !self.step_next() {
                return false;
            }
            if self.line.cells[usize::from(self.col)].width != CellWidth::SpacerTail {
                return true;
            }
        }
    }

    fn prev(&mut self) -> bool {
        loop {
            if !self.step_prev() {
                return false;
            }
            if self.line.cells[usize::from(self.col)].width != CellWidth::SpacerTail {
                return true;
            }
        }
    }
}

/// Where each match of `pattern` starts in a row, as a column, with the column after its end.
fn matches_in(line: &Line, pattern: &str) -> Vec<(u16, u16)> {
    if pattern.is_empty() {
        return Vec::new();
    }
    let mut text = String::new();
    // For each character start in `text`: the byte where it begins and the column.
    let mut starts: Vec<(usize, u16)> = Vec::new();
    for (col, cell) in line.cells.iter().enumerate() {
        if cell.width == CellWidth::SpacerTail {
            continue;
        }
        starts.push((text.len(), col as u16));
        if cell.text.is_empty() {
            text.push(' ');
        } else {
            text.push_str(&cell.text);
        }
    }
    let end_col = line.cells.len() as u16;
    let col_of = |byte: usize| -> Option<u16> {
        starts
            .binary_search_by_key(&byte, |(b, _)| *b)
            .ok()
            .map(|i| starts[i].1)
    };
    text.match_indices(pattern)
        .filter_map(|(at, found)| {
            let from = col_of(at)?;
            let to = col_of(at + found.len()).unwrap_or(end_col);
            Some((from, to))
        })
        .collect()
}

impl Copy {
    /// Start at the program's cursor, with the live screen in view.
    pub fn enter<S: Source>(src: &S, cursor: (u16, u16), cols: u16, rows: u16) -> Copy {
        let total = src.total();
        let top = total.saturating_sub(usize::from(rows));
        Copy {
            row: (top + usize::from(cursor.1)).min(total.saturating_sub(1)),
            col: cursor.0.min(cols.saturating_sub(1)),
            want: cursor.0,
            top,
            cols,
            rows,
            selection: None,
            search: None,
            dirty: true,
        }
    }

    fn line_at<S: Source>(&self, src: &S, row: usize) -> Line {
        src.line(row).unwrap_or_else(|| blank_line(self.cols))
    }

    /// Keep the cursor in view and the cursor and anchor inside the rows that exist.
    fn settle<S: Source>(&mut self, src: &S) {
        let total = src.total().max(1);
        self.row = self.row.min(total - 1);
        let rows = usize::from(self.rows.max(1));
        if self.row < self.top {
            self.top = self.row;
        } else if self.row >= self.top + rows {
            self.top = self.row + 1 - rows;
        }
        self.top = self.top.min(total.saturating_sub(rows));
        let line = self.line_at(src, self.row);
        self.col = snap(&line, self.col.min(self.cols.saturating_sub(1)));
        self.dirty = true;
    }

    fn go<S: Source>(&mut self, src: &S, row: usize, col: u16) {
        self.row = row;
        self.col = col;
        self.want = col;
        self.settle(src);
    }

    /// Run one command. The commands that end copy mode or copy something say so in the result.
    pub fn apply<S: Source>(&mut self, src: &S, command: CopyCommand) -> Outcome {
        let total = src.total().max(1);
        let half = usize::from(self.rows / 2).max(1);
        let page = usize::from(self.rows).max(1);
        match command {
            CopyCommand::Left => {
                let line = self.line_at(src, self.row);
                let col = snap(&line, self.col.saturating_sub(1));
                self.go(src, self.row, col);
            }
            CopyCommand::Right => {
                let line = self.line_at(src, self.row);
                let step = if line
                    .cells
                    .get(usize::from(self.col))
                    .is_some_and(|c| c.width == CellWidth::Wide)
                {
                    2
                } else {
                    1
                };
                let col = (self.col + step).min(self.cols.saturating_sub(1));
                self.go(src, self.row, col);
            }
            CopyCommand::Up => self.vertical(src, self.row.saturating_sub(1)),
            CopyCommand::Down => self.vertical(src, (self.row + 1).min(total - 1)),
            CopyCommand::HalfPageUp => self.vertical(src, self.row.saturating_sub(half)),
            CopyCommand::HalfPageDown => self.vertical(src, (self.row + half).min(total - 1)),
            CopyCommand::PageUp => self.vertical(src, self.row.saturating_sub(page)),
            CopyCommand::PageDown => self.vertical(src, (self.row + page).min(total - 1)),
            CopyCommand::Top => self.go(src, 0, 0),
            CopyCommand::Bottom => self.go(src, total - 1, 0),
            CopyCommand::LineStart => self.go(src, self.row, 0),
            CopyCommand::LineFirstChar => {
                let line = self.line_at(src, self.row);
                let col = line
                    .cells
                    .iter()
                    .position(|c| !is_blank(c))
                    .map_or(0, |i| i as u16);
                self.go(src, self.row, col);
            }
            CopyCommand::LineEnd => {
                let line = self.line_at(src, self.row);
                let col = last_filled(&line).unwrap_or(0);
                self.go(src, self.row, col);
            }
            CopyCommand::WordNext => {
                let (row, col) = self.word_next(src);
                self.go(src, row, col);
            }
            CopyCommand::WordBack => {
                let (row, col) = self.word_back(src);
                self.go(src, row, col);
            }
            CopyCommand::WordEnd => {
                let (row, col) = self.word_end(src);
                self.go(src, row, col);
            }
            CopyCommand::SearchForward => return Outcome::AskSearch { forward: true },
            CopyCommand::SearchBackward => return Outcome::AskSearch { forward: false },
            CopyCommand::SearchNext | CopyCommand::SearchPrevious => {
                let Some((pattern, forward)) = self.search.clone() else {
                    return Outcome::Message("nothing was searched for yet".to_string());
                };
                let forward = if command == CopyCommand::SearchNext {
                    forward
                } else {
                    !forward
                };
                return match self.find(src, &pattern, forward) {
                    Ok(false) => Outcome::Moved,
                    Ok(true) => Outcome::Message("search wrapped around".to_string()),
                    Err(message) => Outcome::Message(message),
                };
            }
            CopyCommand::SelectChar => self.toggle_selection(SelectionKind::Char),
            CopyCommand::SelectLine => self.toggle_selection(SelectionKind::Line),
            CopyCommand::Yank | CopyCommand::YankStay => {
                let stay = command == CopyCommand::YankStay;
                return match self.selected_text(src) {
                    Some(text) => Outcome::Yank { text, stay },
                    None => Outcome::Message("nothing is selected".to_string()),
                };
            }
            CopyCommand::Cancel => {
                if self.selection.take().is_some() {
                    self.dirty = true;
                } else {
                    return Outcome::Exit;
                }
            }
            CopyCommand::Exit => return Outcome::Exit,
        }
        Outcome::Moved
    }

    fn vertical<S: Source>(&mut self, src: &S, row: usize) {
        self.row = row;
        self.col = self.want;
        self.settle(src);
    }

    fn toggle_selection(&mut self, kind: SelectionKind) {
        self.selection = match self.selection {
            Some(s) if s.kind == kind => None,
            _ => Some(Selection {
                kind,
                row: self.row,
                col: self.col,
            }),
        };
        self.dirty = true;
    }

    fn word_next<S: Source>(&self, src: &S) -> (usize, u16) {
        let mut w = Walk::new(src, self.cols, self.row, self.col);
        let c = w.class();
        if c != Class::Blank {
            loop {
                if !w.next() {
                    return (w.row, w.col);
                }
                if w.broke || w.class() != c {
                    break;
                }
            }
        }
        while w.class() == Class::Blank {
            if !w.next() {
                break;
            }
        }
        (w.row, w.col)
    }

    fn word_back<S: Source>(&self, src: &S) -> (usize, u16) {
        let mut w = Walk::new(src, self.cols, self.row, self.col);
        if !w.prev() {
            return (w.row, w.col);
        }
        while w.class() == Class::Blank {
            if !w.prev() {
                return (w.row, w.col);
            }
        }
        let c = w.class();
        loop {
            let (row, col) = (w.row, w.col);
            if !w.prev() {
                break;
            }
            if w.broke || w.class() != c {
                w.goto(row, col);
                break;
            }
        }
        (w.row, w.col)
    }

    fn word_end<S: Source>(&self, src: &S) -> (usize, u16) {
        let mut w = Walk::new(src, self.cols, self.row, self.col);
        if !w.next() {
            return (w.row, w.col);
        }
        while w.class() == Class::Blank {
            if !w.next() {
                return (w.row, w.col);
            }
        }
        let c = w.class();
        loop {
            let (row, col) = (w.row, w.col);
            if !w.next() {
                break;
            }
            if w.broke || w.class() != c {
                w.goto(row, col);
                break;
            }
        }
        (w.row, w.col)
    }

    /// Search for `pattern` (as typed, no pattern syntax, capitals matter) from the cursor and
    /// move to the match. Remembers it for the next and previous match. `Ok(true)` when the
    /// match lies on the other side of the end the search ran into.
    pub fn search<S: Source>(
        &mut self,
        src: &S,
        pattern: &str,
        forward: bool,
    ) -> Result<bool, String> {
        self.search = Some((pattern.to_string(), forward));
        self.find(src, pattern, forward)
    }

    fn find<S: Source>(&mut self, src: &S, pattern: &str, forward: bool) -> Result<bool, String> {
        if pattern.is_empty() {
            return Err("nothing to search for".to_string());
        }
        let total = src.total().max(1);
        // Every row once, starting with the cursor's own (the part after the cursor) and ending
        // with it again (the part before the cursor), so a search wraps around.
        for step in 0..=total {
            let row = if forward {
                (self.row + step) % total
            } else {
                (self.row + total - step % total) % total
            };
            let line = self.line_at(src, row);
            let found = matches_in(&line, pattern);
            let hit = if step == 0 {
                if forward {
                    found.iter().find(|(from, _)| *from > self.col)
                } else {
                    found.iter().rev().find(|(from, _)| *from < self.col)
                }
            } else if step == total {
                // Back at the start row: the part the first look left out.
                if forward {
                    found.iter().find(|(from, _)| *from <= self.col)
                } else {
                    found.iter().rev().find(|(from, _)| *from >= self.col)
                }
            } else if forward {
                found.first()
            } else {
                found.last()
            };
            if let Some((from, _)) = hit {
                let wrapped = if forward {
                    row < self.row
                } else {
                    row > self.row
                };
                self.go(src, row, *from);
                return Ok(wrapped);
            }
        }
        Err(format!("\"{pattern}\" was not found"))
    }

    /// The selection as (first row, first column, last row, last column), the rows widened to
    /// whole lines for a selection by line.
    fn selection_span<S: Source>(&self, src: &S) -> Option<(usize, u16, usize, u16)> {
        let sel = self.selection?;
        let (mut first, mut last) = if (sel.row, sel.col) <= (self.row, self.col) {
            ((sel.row, sel.col), (self.row, self.col))
        } else {
            ((self.row, self.col), (sel.row, sel.col))
        };
        if sel.kind == SelectionKind::Line {
            while first.0 > 0 && self.line_at(src, first.0).continues {
                first.0 -= 1;
            }
            let total = src.total();
            while last.0 + 1 < total && self.line_at(src, last.0 + 1).continues {
                last.0 += 1;
            }
            first.1 = 0;
            last.1 = self.cols.saturating_sub(1);
        }
        Some((first.0, first.1, last.0, last.1))
    }

    /// The text of the selection, a line that was broken over rows joined back into one, the
    /// blanks at the end of each line left off.
    pub fn selected_text<S: Source>(&self, src: &S) -> Option<String> {
        let (r1, c1, r2, c2) = self.selection_span(src)?;
        let mut out = String::new();
        for row in r1..=r2 {
            let line = self.line_at(src, row);
            let from = if row == r1 { usize::from(c1) } else { 0 };
            let to = if row == r2 {
                usize::from(c2) + 1
            } else {
                line.cells.len()
            };
            let to = to.min(line.cells.len());
            let piece = if from < to {
                cells_text(&line.cells[from..to])
            } else {
                String::new()
            };
            let goes_on = row < r2 && self.line_at(src, row + 1).continues;
            if goes_on {
                out.push_str(&piece);
            } else {
                out.push_str(piece.trim_end());
                if row < r2 {
                    out.push('\n');
                }
            }
        }
        Some(out)
    }

    /// What the pane shows now: the rows from `top`, the selection and the matches marked.
    pub fn view<S: Source>(&self, src: &S) -> Vec<Vec<Cell>> {
        let mark = theme::style(Role::Selection);
        let span = self.selection_span(src);
        let pattern = self.search.as_ref().map(|(p, _)| p.as_str());
        (0..usize::from(self.rows))
            .map(|i| {
                let row = self.top + i;
                let mut line = src.line(row).unwrap_or_else(|| blank_line(self.cols));
                line.cells
                    .resize(usize::from(self.cols), blank(Style::default()));
                if let Some(pattern) = pattern {
                    for (from, to) in matches_in(&line, pattern) {
                        for cell in &mut line.cells[usize::from(from)..usize::from(to)] {
                            cell.style = mark;
                        }
                    }
                }
                if let Some((r1, c1, r2, c2)) = span
                    && (r1..=r2).contains(&row)
                {
                    let len = line.cells.len();
                    let from = if row == r1 { usize::from(c1) } else { 0 };
                    let to = if row == r2 { usize::from(c2) + 1 } else { len };
                    for cell in &mut line.cells[from.min(len)..to.min(len)] {
                        cell.style = mark;
                    }
                }
                line.cells
            })
            .collect()
    }

    /// Where the cursor is within the view, if it is in it.
    pub fn cursor_in_view(&self) -> Option<(u16, u16)> {
        let y = self.row.checked_sub(self.top)?;
        (y < usize::from(self.rows)).then_some((self.col, y as u16))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::compose::{char_width, text_cells};

    const COLS: u16 = 10;

    struct Rows(Vec<Line>);

    impl Source for Rows {
        fn total(&self) -> usize {
            self.0.len()
        }
        fn line(&self, y: usize) -> Option<Line> {
            self.0.get(y).cloned()
        }
    }

    /// Rows of `COLS` columns from strings; a leading `+` says the row goes on from the one
    /// above.
    fn rows(spec: &[&str]) -> Rows {
        Rows(
            spec.iter()
                .map(|text| {
                    let (continues, text) = match text.strip_prefix('+') {
                        Some(rest) => (true, rest),
                        None => (false, *text),
                    };
                    let mut cells = text_cells(text, Style::default());
                    cells.resize(usize::from(COLS), blank(Style::default()));
                    Line { cells, continues }
                })
                .collect(),
        )
    }

    fn copy_at(src: &Rows, row: usize, col: u16) -> Copy {
        let mut c = Copy::enter(src, (0, 0), COLS, 4);
        c.go(src, row, col);
        c
    }

    fn at(c: &Copy) -> (usize, u16) {
        (c.row, c.col)
    }

    fn run(c: &mut Copy, src: &Rows, commands: &[CopyCommand]) {
        for command in commands {
            c.apply(src, *command);
        }
    }

    #[test]
    fn the_cursor_starts_at_the_programs_cursor_on_the_live_screen() {
        let src = rows(&["a", "b", "c", "d", "e", "f"]);
        let c = Copy::enter(&src, (3, 1), COLS, 4);
        assert_eq!(c.top, 2);
        assert_eq!(at(&c), (3, 3));
    }

    #[test]
    fn single_steps_stop_at_the_edges() {
        let src = rows(&["abc", "de"]);
        let mut c = copy_at(&src, 0, 0);
        run(&mut c, &src, &[CopyCommand::Left, CopyCommand::Up]);
        assert_eq!(at(&c), (0, 0));
        run(&mut c, &src, &[CopyCommand::Right, CopyCommand::Down]);
        assert_eq!(at(&c), (1, 1));
        run(&mut c, &src, &[CopyCommand::Down]);
        assert_eq!(at(&c), (1, 1));
        let mut c = copy_at(&src, 0, 9);
        run(&mut c, &src, &[CopyCommand::Right]);
        assert_eq!(c.col, 9);
    }

    #[test]
    fn moving_over_a_short_line_keeps_the_column_aimed_for() {
        let src = rows(&["abcdefgh", "ab", "abcdefgh"]);
        let mut c = copy_at(&src, 0, 6);
        run(&mut c, &src, &[CopyCommand::Down, CopyCommand::Down]);
        assert_eq!(at(&c), (2, 6));
    }

    #[test]
    fn a_wide_character_is_one_step() {
        let src = rows(&["a漢b"]);
        let mut c = copy_at(&src, 0, 0);
        run(&mut c, &src, &[CopyCommand::Right]);
        assert_eq!(c.col, 1);
        run(&mut c, &src, &[CopyCommand::Right]);
        assert_eq!(c.col, 3);
        run(&mut c, &src, &[CopyCommand::Left]);
        assert_eq!(c.col, 1);
    }

    #[test]
    fn words_are_runs_of_letters_or_of_punctuation() {
        let src = rows(&["foo bar_b", "(qux)"]);
        let mut c = copy_at(&src, 0, 0);
        run(&mut c, &src, &[CopyCommand::WordNext]);
        assert_eq!(at(&c), (0, 4));
        run(&mut c, &src, &[CopyCommand::WordNext]);
        assert_eq!(at(&c), (1, 0));
        run(&mut c, &src, &[CopyCommand::WordNext]);
        assert_eq!(at(&c), (1, 1));
        run(&mut c, &src, &[CopyCommand::WordBack]);
        assert_eq!(at(&c), (1, 0));
        run(&mut c, &src, &[CopyCommand::WordBack]);
        assert_eq!(at(&c), (0, 4));
        let mut c = copy_at(&src, 0, 0);
        run(&mut c, &src, &[CopyCommand::WordEnd]);
        assert_eq!(at(&c), (0, 2));
        run(&mut c, &src, &[CopyCommand::WordEnd]);
        assert_eq!(at(&c), (0, 8));
    }

    #[test]
    fn a_word_does_not_run_over_the_end_of_a_line_but_does_over_a_broken_one() {
        let src = rows(&["abcdefghij", "klm"]);
        let mut c = copy_at(&src, 0, 0);
        run(&mut c, &src, &[CopyCommand::WordEnd]);
        assert_eq!(at(&c), (0, 9));
        let src = rows(&["abcdefghij", "+klm"]);
        let mut c = copy_at(&src, 0, 0);
        run(&mut c, &src, &[CopyCommand::WordEnd]);
        assert_eq!(at(&c), (1, 2));
    }

    #[test]
    fn the_line_moves_find_the_start_the_first_character_and_the_end() {
        let src = rows(&["  ab c  "]);
        let mut c = copy_at(&src, 0, 4);
        run(&mut c, &src, &[CopyCommand::LineFirstChar]);
        assert_eq!(c.col, 2);
        run(&mut c, &src, &[CopyCommand::LineEnd]);
        assert_eq!(c.col, 5);
        run(&mut c, &src, &[CopyCommand::LineStart]);
        assert_eq!(c.col, 0);
    }

    #[test]
    fn the_big_moves_go_by_pages_and_to_the_ends() {
        let names: Vec<String> = (0..20).map(|i| format!("l{i}")).collect();
        let refs: Vec<&str> = names.iter().map(String::as_str).collect();
        let src = rows(&refs);
        let mut c = copy_at(&src, 10, 0);
        run(&mut c, &src, &[CopyCommand::HalfPageUp]);
        assert_eq!(c.row, 8);
        run(&mut c, &src, &[CopyCommand::PageDown]);
        assert_eq!(c.row, 12);
        run(&mut c, &src, &[CopyCommand::Top]);
        assert_eq!((c.row, c.top), (0, 0));
        run(&mut c, &src, &[CopyCommand::Bottom]);
        assert_eq!((c.row, c.top), (19, 16));
        run(&mut c, &src, &[CopyCommand::PageDown]);
        assert_eq!(c.row, 19);
    }

    #[test]
    fn a_selection_by_character_gives_its_text_without_trailing_blanks() {
        let src = rows(&["one two", "three", "four"]);
        let mut c = copy_at(&src, 0, 4);
        run(&mut c, &src, &[CopyCommand::SelectChar]);
        c.go(&src, 2, 1);
        assert_eq!(c.selected_text(&src).unwrap(), "two\nthree\nfo");
    }

    #[test]
    fn a_selection_made_backwards_gives_the_same_text() {
        let src = rows(&["one two", "three"]);
        let mut c = copy_at(&src, 1, 2);
        run(&mut c, &src, &[CopyCommand::SelectChar]);
        c.go(&src, 0, 4);
        assert_eq!(c.selected_text(&src).unwrap(), "two\nthr");
    }

    #[test]
    fn a_selection_by_line_takes_whole_lines() {
        let src = rows(&["one", "two  ", "three"]);
        let mut c = copy_at(&src, 0, 2);
        run(&mut c, &src, &[CopyCommand::SelectLine]);
        c.go(&src, 1, 0);
        assert_eq!(c.selected_text(&src).unwrap(), "one\ntwo");
    }

    #[test]
    fn a_line_the_terminal_broke_is_joined_again() {
        let src = rows(&["hello worl", "+d and more", "next"]);
        let mut c = copy_at(&src, 0, 0);
        run(&mut c, &src, &[CopyCommand::SelectChar]);
        c.go(&src, 2, 3);
        assert_eq!(c.selected_text(&src).unwrap(), "hello world and more\nnext");
        // A selection by line that touches one row of a broken line takes all its rows.
        let mut c = copy_at(&src, 1, 0);
        run(&mut c, &src, &[CopyCommand::SelectLine]);
        assert_eq!(c.selected_text(&src).unwrap(), "hello world and more");
    }

    #[test]
    fn wide_characters_and_combining_marks_come_out_whole() {
        let src = rows(&["漢字 vie\u{323}\u{302}t"]);
        let mut c = copy_at(&src, 0, 0);
        run(&mut c, &src, &[CopyCommand::SelectLine]);
        assert_eq!(c.selected_text(&src).unwrap(), "漢字 vie\u{323}\u{302}t");
        assert_eq!(char_width('\u{323}'), 0);
        // Selecting from the second half of a wide character starts at its first half.
        let mut c = copy_at(&src, 0, 1);
        run(&mut c, &src, &[CopyCommand::SelectChar]);
        c.go(&src, 0, 2);
        assert_eq!(c.selected_text(&src).unwrap(), "漢字");
    }

    #[test]
    fn searching_goes_to_the_next_match_in_either_direction() {
        let src = rows(&["a cat", "no", "cat cat", "x"]);
        let mut c = copy_at(&src, 0, 0);
        assert_eq!(c.search(&src, "cat", true), Ok(false));
        assert_eq!(at(&c), (0, 2));
        run(&mut c, &src, &[CopyCommand::SearchNext]);
        assert_eq!(at(&c), (2, 0));
        run(&mut c, &src, &[CopyCommand::SearchNext]);
        assert_eq!(at(&c), (2, 4));
        run(&mut c, &src, &[CopyCommand::SearchPrevious]);
        assert_eq!(at(&c), (2, 0));
        assert_eq!(c.search(&src, "cat", false), Ok(false));
        assert_eq!(at(&c), (0, 2));
    }

    #[test]
    fn a_search_that_finds_nothing_leaves_the_cursor_and_says_so() {
        let src = rows(&["a cat", "dog"]);
        let mut c = copy_at(&src, 1, 1);
        let err = c.search(&src, "bird", true).unwrap_err();
        assert!(err.contains("bird"), "{err}");
        assert_eq!(at(&c), (1, 1));
        assert!(c.search(&src, "", true).is_err());
    }

    #[test]
    fn a_search_wraps_around_and_says_so() {
        let src = rows(&["cat", "dog", "x"]);
        let mut c = copy_at(&src, 1, 0);
        assert_eq!(c.search(&src, "cat", true), Ok(true));
        assert_eq!(at(&c), (0, 0));
        let mut c = copy_at(&src, 0, 0);
        assert_eq!(c.search(&src, "dog", false), Ok(true));
        assert_eq!(at(&c), (1, 0));
    }

    #[test]
    fn searching_without_a_previous_search_says_so() {
        let src = rows(&["a"]);
        let mut c = copy_at(&src, 0, 0);
        assert!(matches!(
            c.apply(&src, CopyCommand::SearchNext),
            Outcome::Message(_)
        ));
        assert_eq!(
            c.apply(&src, CopyCommand::SearchForward),
            Outcome::AskSearch { forward: true }
        );
    }

    #[test]
    fn cancel_drops_the_selection_first_and_leaves_the_second_time() {
        let src = rows(&["abc"]);
        let mut c = copy_at(&src, 0, 0);
        run(&mut c, &src, &[CopyCommand::SelectChar]);
        assert_eq!(c.apply(&src, CopyCommand::Cancel), Outcome::Moved);
        assert!(c.selection.is_none());
        assert_eq!(c.apply(&src, CopyCommand::Cancel), Outcome::Exit);
    }

    #[test]
    fn selecting_again_with_the_same_key_drops_the_selection() {
        let src = rows(&["abc"]);
        let mut c = copy_at(&src, 0, 0);
        run(&mut c, &src, &[CopyCommand::SelectChar]);
        assert!(c.selection.is_some());
        run(&mut c, &src, &[CopyCommand::SelectChar]);
        assert!(c.selection.is_none());
        // The other kind replaces it.
        run(
            &mut c,
            &src,
            &[CopyCommand::SelectChar, CopyCommand::SelectLine],
        );
        assert_eq!(c.selection.unwrap().kind, SelectionKind::Line);
    }

    #[test]
    fn copying_with_nothing_selected_says_so_and_copying_the_selection_gives_text() {
        let src = rows(&["abc def"]);
        let mut c = copy_at(&src, 0, 0);
        assert!(matches!(
            c.apply(&src, CopyCommand::Yank),
            Outcome::Message(_)
        ));
        run(
            &mut c,
            &src,
            &[CopyCommand::SelectChar, CopyCommand::WordEnd],
        );
        assert_eq!(
            c.apply(&src, CopyCommand::Yank),
            Outcome::Yank {
                text: "abc".to_string(),
                stay: false
            }
        );
        assert_eq!(
            c.apply(&src, CopyCommand::YankStay),
            Outcome::Yank {
                text: "abc".to_string(),
                stay: true
            }
        );
    }

    #[test]
    fn the_view_marks_the_selection_and_the_matches() {
        let src = rows(&["abc def", "abc"]);
        let mut c = copy_at(&src, 0, 4);
        run(
            &mut c,
            &src,
            &[CopyCommand::SelectChar, CopyCommand::WordEnd],
        );
        let view = c.view(&src);
        let mark = theme::style(Role::Selection);
        assert_eq!(view[0][3].style, Style::default());
        assert_eq!(view[0][4].style, mark);
        assert_eq!(view[0][6].style, mark);
        assert_eq!(view[0][7].style, Style::default());
        c.selection = None;
        let _ = c.search(&src, "abc", true);
        let view = c.view(&src);
        assert_eq!(view[1][0].style, mark);
        assert_eq!(view[1][3].style, Style::default());
    }
}
