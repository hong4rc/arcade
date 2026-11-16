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
    /// The row that was marked as the one to follow (see `Term::anchor_last_row`), where it is
    /// now. A source that cannot say leaves copy mode to compare what the rows say.
    fn anchor_row(&self) -> Option<usize> {
        None
    }
    /// The address of the link (OSC 8) the cell belongs to.
    fn link_at(&self, _row: usize, _col: u16) -> Option<String> {
        None
    }
    /// The runs of columns of a row that belong to a link the host terminal can take.
    fn row_links(&self, _row: usize) -> Vec<arcade_term::LinkSpan> {
        Vec::new()
    }
    /// A shell prompt began on this row (an OSC 133 `A` mark).
    fn prompt_row(&self, _row: usize) -> bool {
        false
    }
    /// The row holds text of a prompt or of what was typed at it.
    fn prompt_or_input_row(&self, _row: usize) -> bool {
        false
    }
    /// A full-screen program is showing (the alternate screen).
    fn alt_screen(&self) -> bool {
        false
    }
}

impl Source for arcade_term::Term {
    fn link_at(&self, row: usize, col: u16) -> Option<String> {
        self.link_at_screen(row, col)
    }

    fn row_links(&self, row: usize) -> Vec<arcade_term::LinkSpan> {
        arcade_term::Term::row_links(self, row)
    }

    fn prompt_row(&self, row: usize) -> bool {
        self.row_is_prompt(row)
    }

    fn prompt_or_input_row(&self, row: usize) -> bool {
        self.row_is_prompt_or_input(row)
    }

    fn alt_screen(&self) -> bool {
        self.mode(arcade_term::Mode::AltScreen).unwrap_or(false)
    }

    fn total(&self) -> usize {
        self.total_rows()
    }

    fn line(&self, y: usize) -> Option<Line> {
        arcade_term::Term::line(self, y)
    }

    fn anchor_row(&self) -> Option<usize> {
        arcade_term::Term::anchor_row(self)
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
    /// Open this address with the system's opener.
    OpenLink { uri: String },
    /// Leave copy mode without copying.
    Exit,
}

/// The letters a jump labels its matches with, best first: the home row, then the rest. A
/// letter that could continue a match is left out when the labels are given (see
/// `Copy::jump_matches`).
pub const JUMP_LABELS: &str = "asdfghjklqwertyuiopzxcvbnm";

/// What a key does while a jump is open.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum JumpKey {
    /// A character: a label, or one more character to look for.
    Char(char),
    /// Go to the nearest match.
    Enter,
    /// Take the last typed character away.
    Backspace,
    /// Cancel and put the cursor back.
    Escape,
}

/// A place where what was typed is found on the visible rows.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct JumpMatch {
    /// The cells of the match, in order: (row, column).
    pub cells: Vec<(usize, u16)>,
    /// The cell right after the match, where the label is drawn (the last cell of the match if
    /// nothing follows it on the row).
    pub label_at: (usize, u16),
    pub label: Option<char>,
}

/// An open jump.
#[derive(Clone, Debug)]
pub struct Jump {
    pub typed: String,
    pub matches: Vec<JumpMatch>,
    /// Where the cursor and the view were, to put them back on Escape.
    origin: (usize, u16, usize),
}

pub struct Copy {
    /// The jump that is open, if any.
    pub jump: Option<Jump>,
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
    /// A turn of the wheel opened copy mode (and nothing else has been done in it since): it
    /// closes by itself when the wheel brings the view back to the live screen.
    pub by_wheel: bool,
    /// What the oldest row, the cursor's row and the selected row said when copy mode last
    /// looked, to find them again when old lines have been cut off the top of the scrollback.
    marks: Marks,
    /// Where the followed row (see `Source::anchor_row`) was when copy mode last looked.
    anchor: Option<usize>,
    /// Whether the mark of the place in the history is drawn on the first row, and the place
    /// the view was last drawn with, to draw it again when it changes.
    pub show_position: bool,
    /// Cells kept free at the right end of the first row, for the mark of a notification that the
    /// pane wears in its corner; the position mark ends before them.
    pub right_gap: u16,
    pub drawn_position: Option<(usize, usize)>,
}

#[derive(Default)]
struct Marks {
    first: String,
    cursor: String,
    selection: Option<String>,
}

const NO_MARKS: &str = "no shell prompt marks in this pane; see the user guide";
const ALT_SCREEN: &str = "a full-screen program is showing; there is no command output to use";

/// Whether any row has a prompt mark.
fn has_prompt_marks<S: Source>(src: &S) -> bool {
    (0..src.total()).any(|r| src.prompt_row(r))
}

/// The last row at or above `row` where a prompt began.
fn prompt_at_or_before<S: Source>(src: &S, row: usize) -> Option<usize> {
    (0..=row.min(src.total().saturating_sub(1)))
        .rev()
        .find(|r| src.prompt_row(*r))
}

/// The rows of the output of the command typed at the prompt that began on row `prompt`: from
/// the first row after the prompt and what was typed, to the row before the next prompt, without
/// the empty rows at the end. `None` when it printed nothing.
fn output_span<S: Source>(src: &S, prompt: usize) -> Option<(usize, usize)> {
    let total = src.total();
    let mut first = prompt + 1;
    while first < total && !src.prompt_row(first) && src.prompt_or_input_row(first) {
        first += 1;
    }
    let mut end = (first..total).find(|r| src.prompt_row(*r)).unwrap_or(total);
    let empty = |r: usize| {
        src.line(r)
            .is_none_or(|l| l.cells.iter().all(|c| c.text.trim().is_empty()))
    };
    while end > first && empty(end - 1) {
        end -= 1;
    }
    (end > first).then(|| (first, end - 1))
}

/// The text of the output of the last command that finished, and how many rows it has: the one
/// before the last prompt. Nothing is selected or moved; this is for copying without copy mode.
pub fn last_output<S: Source>(src: &S, cols: u16, rows: u16) -> Result<(String, usize), String> {
    if src.alt_screen() {
        return Err(ALT_SCREEN.to_string());
    }
    let Some(last) = prompt_at_or_before(src, src.total().saturating_sub(1)) else {
        return Err(NO_MARKS.to_string());
    };
    let Some(before) = last
        .checked_sub(1)
        .and_then(|r| prompt_at_or_before(src, r))
    else {
        return Err("no command has finished yet".to_string());
    };
    let Some((first, end)) = output_span(src, before) else {
        return Err("the last command printed nothing".to_string());
    };
    let mut copy = Copy::enter(src, (0, 0), cols, rows);
    copy.selection = Some(Selection {
        kind: SelectionKind::Line,
        row: first,
        col: 0,
    });
    copy.row = end;
    copy.col = 0;
    let text = copy.selected_text(src).unwrap_or_default();
    Ok((text, end - first + 1))
}

/// How far down the scrollback a line is looked for after old lines were cut off.
const FOLLOW_LIMIT: usize = 20_000;

/// The text of a row and of the one below it: enough to tell a line from its neighbours.
fn signature<S: Source>(src: &S, row: usize) -> String {
    let text = |y: usize| {
        src.line(y)
            .map(|l| cells_text(&l.cells))
            .unwrap_or_default()
    };
    format!("{}\n{}", text(row), text(row + 1))
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
            jump: None,
            row: (top + usize::from(cursor.1)).min(total.saturating_sub(1)),
            col: cursor.0.min(cols.saturating_sub(1)),
            want: cursor.0,
            top,
            cols,
            rows,
            selection: None,
            search: None,
            dirty: true,
            by_wheel: false,
            marks: Marks::default(),
            anchor: None,
            show_position: true,
            right_gap: 0,
            drawn_position: None,
        }
        .remembered(src)
    }

    fn remembered<S: Source>(mut self, src: &S) -> Copy {
        self.remember(src);
        self
    }

    fn remember<S: Source>(&mut self, src: &S) {
        self.anchor = src.anchor_row();
        self.marks = Marks {
            first: signature(src, 0),
            cursor: signature(src, self.row),
            selection: self.selection.map(|s| signature(src, s.row)),
        };
    }

    /// A full scrollback loses its oldest lines as the program prints, and every row number
    /// moves up by that many. The followed row says how many exactly; a source that cannot say
    /// is read for it: when the oldest row no longer says what it said, the cursor's line is
    /// found again by its text (which cannot tell identical lines apart). The cursor, the view
    /// and the selection move with their lines. A selection whose line is gone is dropped, and
    /// the message says so.
    fn follow<S: Source>(&mut self, src: &S) -> Option<String> {
        let exact = match (self.anchor, src.anchor_row()) {
            (Some(was), Some(now)) if now == was => return None,
            // A followed row at the very top may be one whose line was cut off itself.
            (Some(was), Some(now)) if now > 0 && now < was => Some(was - now),
            _ => None,
        };
        if exact.is_none() && signature(src, 0) == self.marks.first {
            return None;
        }
        let gone = || "the selected text is no longer in the scrollback".to_string();
        let mut message = None;
        let cut = match exact {
            Some(k) if k <= self.row => Some(k),
            Some(_) => None,
            None => (1..=self.row.min(FOLLOW_LIMIT))
                .find(|k| signature(src, self.row - k) == self.marks.cursor),
        };
        match cut {
            Some(k) => {
                self.row -= k;
                self.top = self.top.saturating_sub(k);
                if let Some(selection) = self.selection.as_mut() {
                    let same = selection.row >= k
                        && (exact.is_some()
                            || self.marks.selection.as_deref()
                                == Some(signature(src, selection.row - k).as_str()));
                    if same {
                        selection.row -= k;
                    } else {
                        self.selection = None;
                        message = Some(gone());
                    }
                }
            }
            None => {
                if self.selection.take().is_some() {
                    message = Some(gone());
                }
            }
        }
        self.settle(src);
        self.remember(src);
        message
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

    /// Move the view `lines` rows (up when negative) and keep the cursor inside it.
    pub fn scroll<S: Source>(&mut self, src: &S, lines: isize) {
        self.follow(src);
        let total = src.total().max(1);
        let rows = usize::from(self.rows.max(1));
        let last_top = total.saturating_sub(rows);
        self.top = self.top.saturating_add_signed(lines).min(last_top);
        self.row = self
            .row
            .clamp(self.top, (self.top + rows - 1).min(total - 1));
        self.settle(src);
        self.remember(src);
    }

    /// The view shows the last rows, the live screen.
    pub fn at_bottom<S: Source>(&self, src: &S) -> bool {
        self.top + usize::from(self.rows) >= src.total()
    }

    /// While a selection is being dragged: put the cursor on a cell of the view and keep the
    /// anchor where it is, even when the view has moved since the press.
    pub fn extend_to<S: Source>(&mut self, src: &S, to: (u16, u16)) {
        self.follow(src);
        let last_row = self.rows.saturating_sub(1);
        let last_col = self.cols.saturating_sub(1);
        self.go(
            src,
            self.top + usize::from(to.1.min(last_row)),
            to.0.min(last_col),
        );
        self.by_wheel = false;
        self.remember(src);
    }

    /// While a selection is being dragged at an edge of the pane: move the view `lines` rows
    /// (up when negative) and put the cursor on the cell `to` of the view, so the selection
    /// grows with it. True when the view moved (it does not at the top of the history or at the
    /// live bottom).
    pub fn drag_scroll<S: Source>(&mut self, src: &S, lines: isize, to: (u16, u16)) -> bool {
        let before = self.top;
        self.scroll(src, lines);
        let moved = self.top != before;
        self.extend_to(src, to);
        moved
    }

    /// Select from one cell of the view to another, character by character, with the cursor at
    /// the second. Cells outside the view are moved into it.
    pub fn select_between<S: Source>(&mut self, src: &S, from: (u16, u16), to: (u16, u16)) {
        self.follow(src);
        let last_row = self.rows.saturating_sub(1);
        let last_col = self.cols.saturating_sub(1);
        let anchor_row = self.top + usize::from(from.1.min(last_row));
        let anchor_line = self.line_at(src, anchor_row);
        self.selection = Some(Selection {
            kind: SelectionKind::Char,
            row: anchor_row,
            col: snap(&anchor_line, from.0.min(last_col)),
        });
        self.go(
            src,
            self.top + usize::from(to.1.min(last_row)),
            to.0.min(last_col),
        );
        self.by_wheel = false;
        self.remember(src);
    }

    /// Run one command. The commands that end copy mode or copy something say so in the result.
    pub fn apply<S: Source>(&mut self, src: &S, command: CopyCommand) -> Outcome {
        self.by_wheel = false;
        if let Some(message) = self.follow(src) {
            return Outcome::Message(message);
        }
        let outcome = self.apply_command(src, command);
        self.remember(src);
        outcome
    }

    fn apply_command<S: Source>(&mut self, src: &S, command: CopyCommand) -> Outcome {
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
            CopyCommand::PromptPrevious | CopyCommand::PromptNext => {
                let forward = command == CopyCommand::PromptNext;
                if src.alt_screen() {
                    return Outcome::Message(ALT_SCREEN.to_string());
                }
                let found = if forward {
                    (self.row + 1..src.total()).find(|r| src.prompt_row(*r))
                } else {
                    (0..self.row).rev().find(|r| src.prompt_row(*r))
                };
                match found {
                    Some(row) => self.go(src, row, 0),
                    None if !has_prompt_marks(src) => {
                        return Outcome::Message(NO_MARKS.to_string());
                    }
                    None => {
                        return Outcome::Message(
                            if forward {
                                "no later prompt"
                            } else {
                                "no earlier prompt"
                            }
                            .to_string(),
                        );
                    }
                }
            }
            CopyCommand::SelectOutput => {
                if src.alt_screen() {
                    return Outcome::Message(ALT_SCREEN.to_string());
                }
                let Some(here) = prompt_at_or_before(src, self.row) else {
                    return Outcome::Message(if has_prompt_marks(src) {
                        "no command above the cursor".to_string()
                    } else {
                        NO_MARKS.to_string()
                    });
                };
                // On the rows of a prompt and what was typed at it, the output wanted is that of
                // the command before; in an output, that output.
                let own = output_span(src, here);
                let (mut block, mut span) = match own {
                    Some(s) if self.row >= s.0 => (here, Some(s)),
                    _ => {
                        let before = here
                            .checked_sub(1)
                            .and_then(|r| prompt_at_or_before(src, r));
                        match before {
                            Some(b) => (b, output_span(src, b)),
                            None => (here, None),
                        }
                    }
                };
                // Again, on the output that is selected: the one before it.
                if let (Some(sel), Some(s)) = (self.selection, span)
                    && sel.kind == SelectionKind::Line
                    && (sel.row.min(self.row), sel.row.max(self.row)) == s
                {
                    let before = block
                        .checked_sub(1)
                        .and_then(|r| prompt_at_or_before(src, r));
                    match before {
                        Some(b) => {
                            block = b;
                            span = output_span(src, b);
                        }
                        None => {
                            return Outcome::Message("no earlier command".to_string());
                        }
                    }
                }
                let _ = block;
                match span {
                    Some((first, last)) => {
                        self.selection = Some(Selection {
                            kind: SelectionKind::Line,
                            row: first,
                            col: 0,
                        });
                        self.go(src, last, 0);
                    }
                    None => return Outcome::Message("that command printed nothing".to_string()),
                }
            }
            CopyCommand::Jump => {
                self.jump = Some(Jump {
                    typed: String::new(),
                    matches: Vec::new(),
                    origin: (self.row, self.col, self.top),
                });
                self.dirty = true;
            }
            CopyCommand::OpenLink => {
                return match self.link_under_cursor(src) {
                    None => Outcome::Message("no link under the cursor".to_string()),
                    Some(uri) if crate::links::may_open(&uri) => Outcome::OpenLink { uri },
                    Some(_) => {
                        Outcome::Message("only http, https and file links are opened".to_string())
                    }
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

    /// The text of a row and the column of its last written cell (blanks are spaces).
    fn row_text<S: Source>(&self, src: &S, row: usize) -> (String, Option<usize>) {
        let line = self.line_at(src, row);
        let mut text = String::new();
        let mut last = None;
        for (col, cell) in line.cells.iter().enumerate() {
            if cell.width == CellWidth::SpacerTail {
                continue;
            }
            if cell.text.trim().is_empty() {
                text.push(' ');
            } else {
                text.push_str(&cell.text);
                last = Some(col);
            }
        }
        (text, last)
    }

    /// Whether `row` goes on from the row above: the terminal wrapped the line there, or the
    /// program broke an address by hand (see `links::hard_break_continues`).
    fn goes_on_from_above<S: Source>(&self, src: &S, row: usize) -> bool {
        if row == 0 {
            return false;
        }
        self.line_at(src, row).continues || self.hand_broken(src, row)
    }

    /// Whether the row above `row` is an address the program cut by hand and `row` is the rest.
    fn hand_broken<S: Source>(&self, src: &S, row: usize) -> bool {
        self.hand_broken_within(src, row, 40)
    }

    fn hand_broken_within<S: Source>(&self, src: &S, row: usize, reach: usize) -> bool {
        if row == 0 || self.line_at(src, row).continues {
            return false;
        }
        let (prev, last) = self.row_text(src, row - 1);
        let Some(last) = last else {
            return false;
        };
        // The row above can itself be the rest of an address broken one row before.
        let prev_is_continuation = reach > 0 && self.hand_broken_within(src, row - 1, reach - 1);
        let (next, _) = self.row_text(src, row);
        crate::links::hard_break_continues(
            prev.trim_end(),
            last,
            usize::from(self.cols),
            prev_is_continuation,
            &next,
        )
    }

    /// The link the cursor is on: the one the program marked (OSC 8) on that cell, otherwise an
    /// address written in the text, also when the terminal wrapped it over several rows or the
    /// program broke it over several rows by hand.
    fn link_under_cursor<S: Source>(&self, src: &S) -> Option<String> {
        if let Some(uri) = src.link_at(self.row, self.col) {
            return Some(uri);
        }
        // The rows of the line the cursor is on (a bounded number each way).
        const REACH: usize = 40;
        let total = src.total();
        let mut first = self.row;
        while first > 0 && self.row - first < REACH && self.goes_on_from_above(src, first) {
            first -= 1;
        }
        let mut last = self.row;
        while last + 1 < total && last - self.row < REACH && self.goes_on_from_above(src, last + 1)
        {
            last += 1;
        }
        let mut text = String::new();
        let mut at = None;
        for row in first..=last {
            let line = self.line_at(src, row);
            let by_hand = row > first && !line.continues && self.hand_broken(src, row);
            let hand_next = row < last && !self.line_at(src, row + 1).continues;
            // Cells up to the last written one when the next row is the rest of the address, and
            // none of the indentation of a row that is the rest of the one above.
            let end = if hand_next {
                self.row_text(src, row).1.map_or(0, |l| l + 1)
            } else {
                line.cells.len()
            };
            let mut indent = by_hand;
            for (col, cell) in line.cells[..end.min(line.cells.len())].iter().enumerate() {
                if cell.width == CellWidth::SpacerTail {
                    continue;
                }
                let blank = cell.text.trim().is_empty();
                if indent && blank {
                    continue;
                }
                indent = false;
                if row == self.row && col <= usize::from(self.col) {
                    at = Some(text.len());
                }
                text.push_str(if cell.text.is_empty() {
                    " "
                } else {
                    &cell.text
                });
            }
        }
        crate::links::find_address(&text, at?)
    }

    /// What copy mode is doing, for the status bar.
    pub fn state(&self) -> crate::status::CopyState {
        use crate::status::CopyState;
        if let Some(jump) = &self.jump {
            return CopyState::Jump(jump.typed.clone());
        }
        match self.selection.map(|s| s.kind) {
            Some(SelectionKind::Char) => CopyState::Visual,
            Some(SelectionKind::Line) => CopyState::VisualLine,
            None => CopyState::Normal,
        }
    }

    /// A key while a jump is open. Moving the cursor keeps the selection's anchor, so a
    /// selection that is active is extended to the match.
    pub fn jump_key<S: Source>(&mut self, src: &S, key: JumpKey) {
        let Some(jump) = self.jump.as_mut() else {
            return;
        };
        self.dirty = true;
        match key {
            JumpKey::Escape => {
                let (row, col, top) = jump.origin;
                self.jump = None;
                self.row = row;
                self.col = col;
                self.want = col;
                self.top = top;
                self.settle(src);
            }
            JumpKey::Backspace => {
                jump.typed.pop();
                self.rematch(src);
            }
            JumpKey::Enter => {
                // The first match after the cursor, or the first of all when none is after it.
                let at = (self.row, self.col);
                let target = jump
                    .matches
                    .iter()
                    .map(|m| m.cells[0])
                    .find(|cell| *cell > at)
                    .or_else(|| jump.matches.first().map(|m| m.cells[0]));
                if let Some((row, col)) = target {
                    self.jump = None;
                    self.go(src, row, col);
                }
            }
            JumpKey::Char(c) => {
                let labelled = jump
                    .matches
                    .iter()
                    .find(|m| m.label == Some(c))
                    .map(|m| m.cells[0]);
                if let Some((row, col)) = labelled {
                    self.jump = None;
                    self.go(src, row, col);
                } else {
                    jump.typed.push(c);
                    self.rematch(src);
                }
            }
        }
        self.remember(src);
    }

    /// Find again what was typed on the visible rows, and give the labels.
    fn rematch<S: Source>(&mut self, src: &S) {
        let Some(typed) = self.jump.as_ref().map(|j| j.typed.clone()) else {
            return;
        };
        let matches = self.jump_matches(src, &typed);
        if let Some(jump) = self.jump.as_mut() {
            jump.matches = matches;
        }
    }

    /// Every place on the visible rows where `typed` is, with labels for the ones nearest to
    /// the cursor. A lower case `typed` matches either case; an upper case letter makes it exact.
    /// A match may go over the end of a row the terminal broke.
    fn jump_matches<S: Source>(&self, src: &S, typed: &str) -> Vec<JumpMatch> {
        let want: Vec<char> = typed.chars().collect();
        if want.is_empty() {
            return Vec::new();
        }
        let exact = want.iter().any(|c| c.is_uppercase());
        let same = |a: char, b: char| {
            if exact {
                a == b
            } else {
                a.to_lowercase().eq(b.to_lowercase())
            }
        };
        // The visible cells that hold something, row by row, with their places; a row the
        // terminal broke goes on from the one above it in the same run.
        let mut found: Vec<JumpMatch> = Vec::new();
        let mut run: Vec<(char, usize, u16)> = Vec::new();
        // The characters that would continue some match cannot be labels, or typing one would be
        // both a label and a longer search.
        let mut continuation: Vec<char> = Vec::new();
        let mut flush = |run: &mut Vec<(char, usize, u16)>, found: &mut Vec<JumpMatch>| {
            let n = want.len();
            let mut i = 0;
            while i + n <= run.len() {
                if (0..n).all(|k| same(run[i + k].0, want[k])) {
                    let cells: Vec<(usize, u16)> =
                        run[i..i + n].iter().map(|(_, r, c)| (*r, *c)).collect();
                    let last = cells[n - 1];
                    let label_at = match run.get(i + n) {
                        Some((c, r, col)) if *r == last.0 => {
                            continuation.push(c.to_ascii_lowercase());
                            (*r, *col)
                        }
                        _ => last,
                    };
                    found.push(JumpMatch {
                        cells,
                        label_at,
                        label: None,
                    });
                }
                i += 1;
            }
            run.clear();
        };
        for row in self.top..(self.top + usize::from(self.rows)).min(src.total()) {
            let line = self.line_at(src, row);
            if !line.continues {
                flush(&mut run, &mut found);
            }
            for (col, cell) in line.cells.iter().enumerate().take(usize::from(self.cols)) {
                if cell.width == CellWidth::SpacerTail {
                    continue;
                }
                let c = cell.text.chars().next().unwrap_or(' ');
                run.push((c, row, col as u16));
            }
        }
        flush(&mut run, &mut found);
        let cursor = (self.row, self.col);
        found.sort_by_key(|m| {
            let (r, c) = m.cells[0];
            (r.abs_diff(cursor.0), c.abs_diff(cursor.1), r, c)
        });
        let mut labels = JUMP_LABELS.chars().filter(|l| !continuation.contains(l));
        for m in &mut found {
            m.label = labels.next();
        }
        found.sort_by_key(|m| m.cells[0]);
        found
    }

    /// Mark the jump's matches and labels on a row of the view.
    fn mark_jump(&self, row: usize, cells: &mut [Cell]) {
        let Some(jump) = &self.jump else {
            return;
        };
        let hit = theme::style(Role::JumpMatch);
        let tag = theme::style(Role::JumpLabel);
        for m in &jump.matches {
            for (r, c) in &m.cells {
                if *r == row
                    && let Some(cell) = cells.get_mut(usize::from(*c))
                {
                    cell.style = hit;
                }
            }
            if let (Some(label), (r, c)) = (m.label, m.label_at)
                && r == row
                && let Some(cell) = cells.get_mut(usize::from(c))
            {
                *cell = Cell {
                    text: label.to_string(),
                    width: CellWidth::Narrow,
                    style: tag,
                };
            }
        }
    }

    fn vertical<S: Source>(&mut self, src: &S, row: usize) {
        self.row = row;
        self.col = self.want;
        self.settle(src);
    }

    fn toggle_selection(&mut self, kind: SelectionKind) {
        // As in Vim: the same kind again turns the selection off; the other kind keeps the
        // anchor and changes how the span from it to the cursor is read.
        self.selection = match self.selection {
            Some(s) if s.kind == kind => None,
            Some(s) => Some(Selection { kind, ..s }),
            None => Some(Selection {
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
        if let Some(message) = self.follow(src) {
            return Err(message);
        }
        self.search = Some((pattern.to_string(), forward));
        let found = self.find(src, pattern, forward);
        self.remember(src);
        found
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

    /// Where the view is in the history: how many lines it is above the bottom, and how many
    /// lines of history there are (the rows above the live screen). Two counters, no walk.
    pub fn position<S: Source>(&self, src: &S) -> (usize, usize) {
        let history = src.total().saturating_sub(usize::from(self.rows));
        (history.saturating_sub(self.top), history)
    }

    /// What the pane shows now: the rows from `top`, the selection and the matches marked, and
    /// on the first row, at the right, `[above/history]` (as tmux does) when that is wanted and
    /// there is room. The mark is drawn over the view only: the text the rows hold is not touched.
    #[cfg(test)]
    pub fn view<S: Source>(&self, src: &S) -> Vec<Vec<Cell>> {
        self.view_with_links(src).0
    }

    /// The same view, and with it the links of every row of it (read for the rows that are
    /// drawn, and only when the library says the row holds one). The mark of the place in the
    /// history takes the link away from the cells it covers.
    pub fn view_with_links<S: Source>(
        &self,
        src: &S,
    ) -> (Vec<Vec<Cell>>, Vec<Vec<arcade_term::LinkSpan>>) {
        let cols = self.cols;
        let mut links: Vec<Vec<arcade_term::LinkSpan>> = (0..usize::from(self.rows))
            .map(|i| {
                let mut spans = src.row_links(self.top + i);
                spans.retain_mut(|s| {
                    s.to = s.to.min(cols);
                    s.from < s.to
                });
                spans
            })
            .collect();
        let mut rows = self.view_rows(src);
        if self.show_position {
            let (above, history) = self.position(src);
            let text = format!("[{above}/{history}]");
            let cols = usize::from(self.cols);
            let gap = usize::from(self.right_gap);
            if let Some(first) = rows.first_mut()
                && text.len() + gap <= cols
                && first.len() >= cols
            {
                let start = cols - gap - text.len();
                if let Some(spans) = links.first_mut() {
                    let cut = u16::try_from(start).unwrap_or(u16::MAX);
                    spans.retain_mut(|s| {
                        s.to = s.to.min(cut);
                        s.from < s.to
                    });
                }
                if first[start].width == arcade_term::CellWidth::SpacerTail && start > 0 {
                    first[start - 1] = blank(Style::default());
                }
                let style = theme::style(Role::ScrollPosition);
                for (cell, c) in first[start..cols - gap].iter_mut().zip(text.chars()) {
                    *cell = Cell {
                        text: c.to_string(),
                        width: arcade_term::CellWidth::Narrow,
                        style,
                    };
                }
            }
        }
        (rows, links)
    }

    fn view_rows<S: Source>(&self, src: &S) -> Vec<Vec<Cell>> {
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
                self.mark_jump(row, &mut line.cells);
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

    fn first_row_text(c: &Copy, src: &Rows) -> String {
        c.view(src)[0]
            .iter()
            .map(|cell| {
                if cell.text.is_empty() {
                    " "
                } else {
                    cell.text.as_str()
                }
            })
            .collect()
    }

    #[test]
    fn the_first_row_carries_the_place_in_the_history_and_leaves_the_text_alone() {
        let spec: Vec<String> = (0..20).map(|i| format!("row{i:02}")).collect();
        let refs: Vec<&str> = spec.iter().map(String::as_str).collect();
        let src = rows(&refs);
        // 20 rows, a view of 4: 16 rows of history. At the bottom the view is 0 above it.
        let mut c = Copy::enter(&src, (0, 0), COLS, 4);
        assert_eq!(c.position(&src), (0, 16));
        assert_eq!(first_row_text(&c, &src), "row1[0/16]");
        c.top = 10;
        assert_eq!(c.position(&src), (6, 16));
        // The mark takes the right end of the first row; what the row holds is not touched.
        assert_eq!(first_row_text(&c, &src), "row1[6/16]");
        assert_eq!(src.line(10).unwrap().cells[4].text, "0");
        c.top = 0;
        assert_eq!(c.position(&src), (16, 16));
        // Off: the row is as it was.
        c.show_position = false;
        assert_eq!(first_row_text(&c, &src), "row00     ");
    }

    #[test]
    fn a_pane_too_narrow_for_the_mark_has_none() {
        let spec: Vec<String> = (0..200).map(|i| format!("r{i}")).collect();
        let refs: Vec<&str> = spec.iter().map(String::as_str).collect();
        let src = rows(&refs);
        let mut c = Copy::enter(&src, (0, 0), 8, 4);
        c.top = 0;
        // "[196/196]" is 9 cells; the pane has 8.
        assert!(!first_row_text(&c, &src).contains('['));
        let mut roomy = Copy::enter(&src, (0, 0), COLS, 4);
        roomy.top = 0;
        assert!(first_row_text(&roomy, &src).ends_with("[196/196]"));
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

    /// Like `rows` and `copy_at`, for rows of 40 columns.
    fn wide(spec: &[&str], row: usize, col: u16) -> (Rows, Copy) {
        let src = Rows(
            spec.iter()
                .map(|text| {
                    let (continues, text) = match text.strip_prefix('+') {
                        Some(rest) => (true, rest),
                        None => (false, *text),
                    };
                    let mut cells = text_cells(text, Style::default());
                    cells.resize(40, blank(Style::default()));
                    Line { cells, continues }
                })
                .collect(),
        );
        let mut c = Copy::enter(&src, (0, 0), 40, 4);
        c.go(&src, row, col);
        (src, c)
    }

    fn jump_open(c: &mut Copy, src: &Rows) {
        c.apply(src, CopyCommand::Jump);
    }

    fn type_jump(c: &mut Copy, src: &Rows, text: &str) {
        for ch in text.chars() {
            c.jump_key(src, JumpKey::Char(ch));
        }
    }

    fn labels(c: &Copy) -> Vec<(usize, u16, Option<char>)> {
        c.jump
            .as_ref()
            .unwrap()
            .matches
            .iter()
            .map(|m| (m.cells[0].0, m.cells[0].1, m.label))
            .collect()
    }

    /// A copy mode with a view of `rows` rows.
    fn tall(spec: &[&str], rows: u16) -> (Rows, Copy) {
        let src = rows_of(spec);
        let c = Copy::enter(&src, (0, 0), COLS, rows);
        (src, c)
    }

    fn rows_of(spec: &[&str]) -> Rows {
        rows(spec)
    }

    #[test]
    fn typing_finds_every_match_and_more_typing_narrows_them() {
        let (src, mut c) = tall(&["ab ac ab", "xx ab"], 2);
        c.go(&src, 0, 0);
        jump_open(&mut c, &src);
        type_jump(&mut c, &src, "a");
        assert_eq!(labels(&c).len(), 4);
        type_jump(&mut c, &src, "b");
        assert_eq!(labels(&c).len(), 3);
        c.jump_key(&src, JumpKey::Backspace);
        assert_eq!(labels(&c).len(), 4);
        assert_eq!(c.state(), crate::status::CopyState::Jump("a".into()));
    }

    #[test]
    fn a_label_is_never_a_character_that_could_continue_a_match() {
        // After each `a` comes `s`, `d`, `f` or `g`: the first labels of the alphabet.
        let (src, mut c) = tall(&["as ad af", "ag"], 2);
        jump_open(&mut c, &src);
        type_jump(&mut c, &src, "a");
        let got = labels(&c);
        assert_eq!(got.len(), 4);
        for (_, _, label) in &got {
            let label = label.expect("every match has a label");
            assert!(!"sdfg".contains(label), "label {label}");
        }
        // And all four are different.
        let mut letters: Vec<char> = got.iter().filter_map(|l| l.2).collect();
        letters.sort_unstable();
        letters.dedup();
        assert_eq!(letters.len(), 4);
    }

    #[test]
    fn a_label_jumps_to_the_start_of_its_match_and_ends_the_jump_without_a_selection() {
        let (src, mut c) = tall(&["ab  cd ab"], 1);
        jump_open(&mut c, &src);
        type_jump(&mut c, &src, "ab");
        let last = labels(&c).last().copied().unwrap();
        c.jump_key(&src, JumpKey::Char(last.2.unwrap()));
        assert!(c.jump.is_none());
        assert_eq!(at(&c), (0, 7));
        assert!(c.selection.is_none());
        assert_eq!(c.state(), crate::status::CopyState::Normal);
    }

    #[test]
    fn a_jump_extends_the_selection_that_is_active_and_keeps_its_anchor() {
        let (src, mut c) = tall(&["one two", "ten"], 2);
        c.go(&src, 0, 4);
        run(&mut c, &src, &[CopyCommand::SelectChar]);
        jump_open(&mut c, &src);
        type_jump(&mut c, &src, "te");
        c.jump_key(&src, JumpKey::Enter);
        assert_eq!(at(&c), (1, 0));
        let sel = c.selection.unwrap();
        assert_eq!((sel.row, sel.col, sel.kind), (0, 4, SelectionKind::Char));
        assert_eq!(c.selected_text(&src).unwrap(), "two\nt");
        assert_eq!(c.state(), crate::status::CopyState::Visual);
        run(&mut c, &src, &[CopyCommand::SelectLine]);
        assert_eq!(c.state(), crate::status::CopyState::VisualLine);
    }

    #[test]
    fn with_more_matches_than_labels_the_nearest_get_them() {
        let spec: Vec<&str> = vec!["a a a a a "; 12];
        let (src, mut c) = tall(&spec, 12);
        c.go(&src, 6, 4);
        jump_open(&mut c, &src);
        type_jump(&mut c, &src, "a");
        let got = labels(&c);
        assert_eq!(got.len(), 60);
        assert_eq!(
            got.iter().filter(|l| l.2.is_some()).count(),
            JUMP_LABELS.len()
        );
        // The match under the cursor is the nearest, so it has the first label; the far
        // corners are left without.
        let near = got.iter().find(|l| (l.0, l.1) == (6, 4)).unwrap();
        assert_eq!(near.2, Some('a'));
        let far = got.iter().find(|l| (l.0, l.1) == (0, 0)).unwrap();
        assert_eq!(far.2, None);
        // Typing a letter that is not a label narrows, it does not jump.
        type_jump(&mut c, &src, " ");
        assert!(c.jump.is_some());
    }

    #[test]
    fn lower_case_matches_both_cases_and_an_upper_case_letter_makes_it_exact() {
        let (src, mut c) = tall(&["ab AB Ab", "aB"], 2);
        jump_open(&mut c, &src);
        type_jump(&mut c, &src, "ab");
        assert_eq!(labels(&c).len(), 4);
        c.jump_key(&src, JumpKey::Backspace);
        c.jump_key(&src, JumpKey::Backspace);
        type_jump(&mut c, &src, "Ab");
        assert_eq!(
            labels(&c).iter().map(|l| (l.0, l.1)).collect::<Vec<_>>(),
            [(0, 6)]
        );
    }

    #[test]
    fn wide_characters_count_as_one_character_at_their_first_column() {
        let (src, mut c) = tall(&["漢字 字x"], 1);
        jump_open(&mut c, &src);
        type_jump(&mut c, &src, "字");
        assert_eq!(labels(&c).iter().map(|l| l.1).collect::<Vec<_>>(), [2, 5]);
    }

    #[test]
    fn a_match_can_go_over_the_end_of_a_row_the_terminal_broke() {
        let (src, mut c) = tall(&["abcdefghhe", "+llo there", "hello"], 3);
        jump_open(&mut c, &src);
        type_jump(&mut c, &src, "hello");
        let got = labels(&c);
        assert_eq!(
            got.iter().map(|l| (l.0, l.1)).collect::<Vec<_>>(),
            [(0, 8), (2, 0)]
        );
        // The first match has cells on both rows.
        let first = &c.jump.as_ref().unwrap().matches[0];
        assert_eq!(first.cells[0], (0, 8));
        assert_eq!(first.cells[4], (1, 2));
        // And a match does not go over a row that is a line of its own.
        let (src, mut c) = tall(&["abcdefghhe", "llo"], 2);
        jump_open(&mut c, &src);
        type_jump(&mut c, &src, "hello");
        assert!(labels(&c).is_empty());
    }

    #[test]
    fn escape_cancels_and_puts_the_cursor_and_the_view_back() {
        let names: Vec<String> = (0..20).map(|i| format!("l{i}")).collect();
        let refs: Vec<&str> = names.iter().map(String::as_str).collect();
        let src = rows(&refs);
        let mut c = copy_at(&src, 10, 1);
        let before = (c.row, c.col, c.top);
        jump_open(&mut c, &src);
        type_jump(&mut c, &src, "l1");
        c.jump_key(&src, JumpKey::Escape);
        assert!(c.jump.is_none());
        assert_eq!((c.row, c.col, c.top), before);
    }

    #[test]
    fn enter_goes_to_the_first_match_after_the_cursor_and_wraps() {
        let (src, mut c) = tall(&["ab cd ab", "ab"], 2);
        c.go(&src, 0, 3);
        jump_open(&mut c, &src);
        type_jump(&mut c, &src, "ab");
        c.jump_key(&src, JumpKey::Enter);
        assert_eq!(at(&c), (0, 6));
        jump_open(&mut c, &src);
        type_jump(&mut c, &src, "ab");
        c.jump_key(&src, JumpKey::Enter);
        assert_eq!(at(&c), (1, 0));
        jump_open(&mut c, &src);
        type_jump(&mut c, &src, "ab");
        c.jump_key(&src, JumpKey::Enter);
        assert_eq!(at(&c), (0, 0));
    }

    #[test]
    fn the_view_shows_the_matches_and_their_labels() {
        let (src, mut c) = tall(&["ab xx ab"], 1);
        jump_open(&mut c, &src);
        type_jump(&mut c, &src, "ab");
        let view = c.view(&src);
        let hit = theme::style(Role::JumpMatch);
        let tag = theme::style(Role::JumpLabel);
        assert_eq!(view[0][0].style, hit);
        assert_eq!(view[0][1].style, hit);
        // The label stands on the cell after the match.
        assert_eq!(view[0][2].style, tag);
        assert_eq!(view[0][2].text.len(), 1);
        assert_eq!(view[0][4].style, Style::default());
    }

    /// How long the matches and labels of a jump take on a full 200 by 60 pane of dense text
    /// (run with `--release` and `--nocapture` to see the numbers; the limit here is for the
    /// slow debug build).
    #[test]
    fn a_jump_on_a_full_dense_pane_is_fast() {
        let mut config = arcade_term::TermConfig::new(200, 60);
        config.scrollback_lines = 1000;
        let mut term = arcade_term::Term::new(config).unwrap();
        let words = [
            "the", "quick", "brown", "fox", "jumps", "over", "lazy", "dog", "e", "a",
        ];
        for y in 0..60 {
            let mut line = String::new();
            let mut i = y;
            while line.len() < 199 {
                line.push_str(words[i % words.len()]);
                line.push(' ');
                i += 3;
            }
            term.feed(format!("{}\r\n", &line[..199]).as_bytes());
        }
        let mut c = Copy::enter(&term, term.cursor_position(), 200, 60);
        c.apply(&term, CopyCommand::Jump);
        let mut times = Vec::new();
        for typed in ['e', 'r', ' '] {
            let start = std::time::Instant::now();
            c.jump_key(&term, JumpKey::Char(typed));
            let spent = start.elapsed();
            let n = c.jump.as_ref().unwrap().matches.len();
            println!("typed {typed:?}: {n} matches, {spent:?}");
            times.push(spent);
        }
        let start = std::time::Instant::now();
        let view = c.view(&term);
        println!("drawing the view with labels: {:?}", start.elapsed());
        assert_eq!(view.len(), 60);
        assert!(times.iter().all(|t| t.as_millis() < 500), "{times:?}");
    }

    /// A terminal of `rows` rows after a shell session: each command is a prompt (OSC 133 `A`,
    /// `B`), the command typed, `C`, its output lines, `D`; the last prompt is waiting.
    fn shell(rows: u16, commands: &[(&str, &[&str])]) -> arcade_term::Term {
        let mut config = arcade_term::TermConfig::new(30, rows);
        config.scrollback_lines = 1000;
        let mut term = arcade_term::Term::new(config).unwrap();
        for (typed, output) in commands {
            term.feed(
                format!("\x1b]133;A\x1b\\$ \x1b]133;B\x1b\\{typed}\r\n\x1b]133;C\x1b\\").as_bytes(),
            );
            for line in *output {
                term.feed(format!("{line}\r\n").as_bytes());
            }
            term.feed(b"\x1b]133;D;0\x1b\\");
        }
        term.feed(b"\x1b]133;A\x1b\\$ \x1b]133;B\x1b\\");
        term
    }

    fn enter_at(term: &arcade_term::Term, row: usize) -> Copy {
        let mut c = Copy::enter(term, term.cursor_position(), 30, 4);
        c.go(term, row, 0);
        c
    }

    /// The rows: 0 `$ ls`, 1 one, 2 two, 3 `$ pwd`, 4 /home, 5 and 6 empty rows of the output,
    /// 7 `$ echo`, 8 `x`, 9 `$ ` (waiting).
    fn three_commands() -> arcade_term::Term {
        shell(
            4,
            &[
                ("ls", &["one", "two"]),
                ("pwd", &["/home", "", ""]),
                ("echo x", &["x"]),
            ],
        )
    }

    #[test]
    fn the_prompt_keys_jump_from_prompt_to_prompt() {
        let term = three_commands();
        let mut c = enter_at(&term, 9);
        assert!(c.apply(&term, CopyCommand::PromptPrevious) == Outcome::Moved);
        assert_eq!(at(&c), (7, 0));
        c.apply(&term, CopyCommand::PromptPrevious);
        assert_eq!(at(&c), (3, 0));
        c.apply(&term, CopyCommand::PromptPrevious);
        assert_eq!(at(&c), (0, 0));
        assert!(matches!(
            c.apply(&term, CopyCommand::PromptPrevious),
            Outcome::Message(m) if m.contains("earlier")
        ));
        assert_eq!(at(&c), (0, 0));
        c.apply(&term, CopyCommand::PromptNext);
        assert_eq!(at(&c), (3, 0));
        let mut c = enter_at(&term, 9);
        assert!(matches!(
            c.apply(&term, CopyCommand::PromptNext),
            Outcome::Message(m) if m.contains("later")
        ));
    }

    #[test]
    fn selecting_output_takes_the_rows_of_the_command_the_cursor_is_in_without_trailing_blanks() {
        let term = three_commands();
        // In the output of `pwd`, which ends in two empty rows.
        let mut c = enter_at(&term, 4);
        assert_eq!(c.apply(&term, CopyCommand::SelectOutput), Outcome::Moved);
        assert_eq!(c.selected_text(&term).unwrap(), "/home");
        assert_eq!(c.state(), crate::status::CopyState::VisualLine);
        // In the output of `ls`.
        let mut c = enter_at(&term, 2);
        c.apply(&term, CopyCommand::SelectOutput);
        assert_eq!(c.selected_text(&term).unwrap(), "one\ntwo");
    }

    #[test]
    fn on_a_prompt_row_the_output_of_the_command_before_is_taken_and_again_goes_further_back() {
        let term = three_commands();
        // On the waiting prompt: the output of `echo x`.
        let mut c = enter_at(&term, 9);
        c.apply(&term, CopyCommand::SelectOutput);
        assert_eq!(c.selected_text(&term).unwrap(), "x");
        c.apply(&term, CopyCommand::SelectOutput);
        assert_eq!(c.selected_text(&term).unwrap(), "/home");
        c.apply(&term, CopyCommand::SelectOutput);
        assert_eq!(c.selected_text(&term).unwrap(), "one\ntwo");
        assert!(matches!(
            c.apply(&term, CopyCommand::SelectOutput),
            Outcome::Message(m) if m.contains("no earlier")
        ));
        // The first prompt has no command before it.
        let mut c = enter_at(&term, 0);
        assert!(matches!(
            c.apply(&term, CopyCommand::SelectOutput),
            Outcome::Message(_)
        ));
    }

    #[test]
    fn an_output_that_runs_into_the_scrollback_is_taken_whole() {
        let lines: Vec<String> = (1..=30).map(|i| format!("line {i}")).collect();
        let refs: Vec<&str> = lines.iter().map(String::as_str).collect();
        let term = shell(4, &[("seq", &refs), ("next", &["done"])]);
        let mut c = enter_at(&term, term.total_rows() - 1);
        c.apply(&term, CopyCommand::SelectOutput);
        c.apply(&term, CopyCommand::SelectOutput);
        let text = c.selected_text(&term).unwrap();
        assert_eq!(text.lines().count(), 30);
        assert!(text.starts_with("line 1\nline 2\n") && text.ends_with("line 30"));
    }

    #[test]
    fn a_pane_without_marks_says_so_for_each_command() {
        let mut term = arcade_term::Term::new(arcade_term::TermConfig::new(30, 4)).unwrap();
        term.feed(b"$ ls\r\none\r\n$ ");
        let mut c = enter_at(&term, 1);
        for command in [
            CopyCommand::PromptPrevious,
            CopyCommand::PromptNext,
            CopyCommand::SelectOutput,
        ] {
            assert!(
                matches!(c.apply(&term, command), Outcome::Message(m) if m.contains("no shell prompt marks")),
                "{command:?}"
            );
        }
        assert_eq!(at(&c), (1, 0));
        assert!(c.selection.is_none());
        assert!(
            last_output(&term, 30, 4)
                .unwrap_err()
                .contains("no shell prompt marks")
        );
    }

    #[test]
    fn a_full_screen_program_gets_a_message_and_nothing_else() {
        let mut term = three_commands();
        term.feed(b"\x1b[?1049h");
        let mut c = enter_at(&term, 0);
        assert!(matches!(
            c.apply(&term, CopyCommand::SelectOutput),
            Outcome::Message(m) if m.contains("full-screen")
        ));
        assert!(
            last_output(&term, 30, 4)
                .unwrap_err()
                .contains("full-screen")
        );
    }

    #[test]
    fn the_last_output_is_the_one_before_the_waiting_prompt() {
        let term = three_commands();
        assert_eq!(last_output(&term, 30, 4).unwrap(), ("x".to_string(), 1));
        let term = shell(4, &[("ls", &["one", "two"])]);
        assert_eq!(
            last_output(&term, 30, 4).unwrap(),
            ("one\ntwo".to_string(), 2)
        );
        let term = shell(4, &[]);
        assert!(
            last_output(&term, 30, 4)
                .unwrap_err()
                .contains("no command")
        );
        let term = shell(4, &[("true", &[])]);
        assert!(last_output(&term, 30, 4).unwrap_err().contains("nothing"));
    }

    /// How long the prompt keys take on a full scrollback (run with `--release --nocapture`).
    #[test]
    fn the_prompt_keys_on_a_full_scrollback_take_a_fraction_of_a_frame() {
        let mut config = arcade_term::TermConfig::new(200, 60);
        config.scrollback_lines = 50_000;
        let mut marked = arcade_term::Term::new(config.clone()).unwrap();
        let mut plain = arcade_term::Term::new(config).unwrap();
        for i in 0..52_000 {
            if i % 20 == 0 {
                marked.feed(b"\x1b]133;A\x1b\\$ \x1b]133;B\x1b\\cmd\r\n\x1b]133;C\x1b\\");
            }
            let line = format!("output line {i} with some text to fill the row\r\n");
            marked.feed(line.as_bytes());
            plain.feed(line.as_bytes());
        }
        println!(
            "scrollback of {} rows, a prompt every 21 rows",
            marked.total_rows()
        );
        let time = |what: &str, term: &arcade_term::Term, command: CopyCommand, from: usize| {
            let mut c = Copy::enter(term, term.cursor_position(), 200, 60);
            c.go(term, from, 0);
            let start = std::time::Instant::now();
            let outcome = c.apply(term, command);
            println!(
                "{what}: {:?} ({:?})",
                start.elapsed(),
                matches!(outcome, Outcome::Message(_))
            );
        };
        let bottom = marked.total_rows() - 1;
        time(
            "previous prompt from the bottom",
            &marked,
            CopyCommand::PromptPrevious,
            bottom,
        );
        time(
            "next prompt from the middle",
            &marked,
            CopyCommand::PromptNext,
            bottom / 2,
        );
        time(
            "select output at the bottom",
            &marked,
            CopyCommand::SelectOutput,
            bottom,
        );
        time(
            "select output in the middle",
            &marked,
            CopyCommand::SelectOutput,
            bottom / 2,
        );
        time(
            "worst case, no marks at all: previous prompt",
            &plain,
            CopyCommand::PromptPrevious,
            bottom,
        );
        time(
            "worst case, no marks at all: select output",
            &plain,
            CopyCommand::SelectOutput,
            bottom,
        );
        let start = std::time::Instant::now();
        let _ = last_output(&marked, 200, 60);
        println!("last output (no copy mode): {:?}", start.elapsed());
    }

    fn open(c: &mut Copy, src: &Rows) -> Outcome {
        c.apply(src, CopyCommand::OpenLink)
    }

    fn opens(uri: &str) -> Outcome {
        Outcome::OpenLink { uri: uri.into() }
    }

    #[test]
    fn a_plain_address_under_the_cursor_is_opened_without_its_trailing_punctuation() {
        let text = ["see https://a.example/x?y=1, then"];
        for col in [4, 10, 25] {
            let (src, mut c) = wide(&text, 0, col);
            assert_eq!(
                open(&mut c, &src),
                opens("https://a.example/x?y=1"),
                "{col}"
            );
        }
        // Off the address, or on the comma after it.
        for col in [0, 3, 27, 30] {
            let (src, mut c) = wide(&text, 0, col);
            assert_eq!(
                open(&mut c, &src),
                Outcome::Message("no link under the cursor".into()),
                "{col}"
            );
        }
    }

    #[test]
    fn an_address_the_terminal_broke_over_two_rows_is_one_address() {
        // The first row is full: that is what a broken line looks like.
        let text = [
            "go https://a.example/aaaaaaaaaaaaaaaaaaa",
            "+aa/path and more",
        ];
        let want = opens("https://a.example/aaaaaaaaaaaaaaaaaaaaa/path");
        let (src, mut c) = wide(&text, 0, 5);
        assert_eq!(open(&mut c, &src), want);
        let (src, mut c) = wide(&text, 1, 2);
        assert_eq!(open(&mut c, &src), want);
        // A row that does not go on from the one above is another line.
        let (src, mut c) = wide(
            &["go https://a.example/aaaaaaaaaaaaaaaaaaa", "aa/path"],
            0,
            5,
        );
        assert_eq!(
            open(&mut c, &src),
            opens("https://a.example/aaaaaaaaaaaaaaaaaaa")
        );
    }

    // The panes of these tests are 40 columns wide: a first row is filled up to the edge.
    fn full(start: &str, fill: char) -> String {
        format!("{start}{}", fill.to_string().repeat(40 - start.len()))
    }

    #[test]
    fn an_address_the_program_broke_by_hand_is_opened_whole_from_either_row() {
        let first = full("see https://h.example/p/", '7');
        let text = [first.as_str(), "  558/DevOps+request"];
        let whole = format!("{}558/DevOps+request", &first[4..]);
        for (row, col) in [(0, 6), (0, 39), (1, 2), (1, 15)] {
            let (src, mut c) = wide(&text, row, col);
            assert_eq!(open(&mut c, &src), opens(&whole), "{row},{col}");
        }
        // On the indentation nothing is under the cursor.
        let (src, mut c) = wide(&text, 1, 0);
        assert!(matches!(open(&mut c, &src), Outcome::Message(m) if m.contains("no link")));
    }

    #[test]
    fn an_address_broken_by_hand_over_three_rows_stops_at_a_space() {
        let first = full("see https://h.example/p/", '7');
        let second = format!("  {}", "b-".repeat(19));
        let text = [first.as_str(), second.as_str(), "  c/d?e=1 and then words"];
        let whole = format!("{}{}c/d?e=1", &first[4..], second.trim());
        for (row, col) in [(0, 8), (1, 10), (2, 3)] {
            let (src, mut c) = wide(&text, row, col);
            assert_eq!(open(&mut c, &src), opens(&whole), "{row},{col}");
        }
        // The words after the space are not part of it.
        let (src, mut c) = wide(&text, 2, 12);
        assert!(matches!(open(&mut c, &src), Outcome::Message(m) if m.contains("no link")));
        // A middle row of plain letters is not joined, so the address stops at the first row.
        let plain = format!("  {}", "b".repeat(38));
        let text = [first.as_str(), plain.as_str(), "  c/d?e=1"];
        let (src, mut c) = wide(&text, 0, 8);
        assert_eq!(open(&mut c, &src), opens(&first[4..]));
    }

    #[test]
    fn rows_that_only_look_like_the_rest_of_an_address_are_not_joined() {
        let first = full("see https://h.example/p/", '7');
        let alone = opens(&first[4..]);
        // A sentence after an address that reaches the edge, a new address, a blank row, and
        // text that is not indented.
        for next in [
            "  Thanks for reading",
            "  Thanks",
            "  https://o.example/x",
            "   ",
            "558/x",
        ] {
            let (src, mut c) = wide(&[first.as_str(), next], 0, 8);
            assert_eq!(open(&mut c, &src), alone, "{next:?}");
        }
        // A short address at the end of a paragraph, then a row with address characters.
        let (src, mut c) = wide(
            &["go to https://a.example/x", "  and see /etc/hosts"],
            0,
            10,
        );
        assert_eq!(open(&mut c, &src), opens("https://a.example/x"));
        // A list of addresses, one per row, each reaching the edge.
        let one = full("https://aaaaaaaaaaaaa.example/p/", '1');
        let two = full("https://bbbbbbbbbbbbb.example/p/", '2');
        for (row, want) in [(0, &one), (1, &two)] {
            let (src, mut c) = wide(&[one.as_str(), two.as_str()], row, 3);
            assert_eq!(open(&mut c, &src), opens(want), "{row}");
        }
        // A table.
        let table = ["| https://h.example/wiki/pages/3954966 |", "| 558/x | 12 |"];
        let (src, mut c) = wide(&table, 0, 5);
        assert_eq!(
            open(&mut c, &src),
            opens("https://h.example/wiki/pages/3954966")
        );
    }

    #[test]
    fn a_row_with_no_address_says_so_and_stays() {
        let text = ["nothing here", "ftp://a.example/x"];
        let (src, mut c) = wide(&text, 0, 2);
        assert!(matches!(open(&mut c, &src), Outcome::Message(m) if m.contains("no link")));
        let (src, mut c) = wide(&text, 1, 2);
        assert!(matches!(open(&mut c, &src), Outcome::Message(m) if m.contains("no link")));
        assert_eq!((c.row, c.col), (1, 2));
    }

    #[test]
    fn the_link_the_program_marked_wins_over_an_address_in_the_text() {
        let mut term = arcade_term::Term::new(arcade_term::TermConfig::new(60, 5)).unwrap();
        term.feed(
            b"see https://text.example/ \x1b]8;;https://marked.example/\x1b\\here\x1b]8;;\x1b\\ \
              \x1b]8;;ftp://bad.example/\x1b\\odd\x1b]8;;\x1b\\",
        );
        let mut c = Copy::enter(&term, term.cursor_position(), 60, 5);
        let row = c.row;
        // Find the first row of the screen.
        let first = term.total_rows() - 5;
        let _ = row;
        c.go(&term, first, 28);
        assert_eq!(
            c.apply(&term, CopyCommand::OpenLink),
            opens("https://marked.example/")
        );
        // On the plain address, the text decides.
        c.go(&term, first, 8);
        assert_eq!(
            c.apply(&term, CopyCommand::OpenLink),
            opens("https://text.example/")
        );
        // A marked link of another kind is refused, not guessed at.
        c.go(&term, first, 32);
        assert!(matches!(
            c.apply(&term, CopyCommand::OpenLink),
            Outcome::Message(m) if m.contains("only http")
        ));
    }

    #[test]
    fn lines_cut_off_the_top_move_the_cursor_and_the_selection_with_their_lines() {
        let before = rows(&["a", "b", "c", "d", "e", "f", "g", "h"]);
        let mut c = copy_at(&before, 5, 0);
        c.apply(&before, CopyCommand::SelectLine);
        c.apply(&before, CopyCommand::Down);
        assert_eq!(c.row, 6);
        // Two lines were cut off the top and two new ones printed at the bottom.
        let after = rows(&["c", "d", "e", "f", "g", "h", "i", "j"]);
        let outcome = c.apply(&after, CopyCommand::Down);
        assert_eq!(outcome, Outcome::Moved);
        // The cursor was on `g`, now row 4, and moved one down to `h`; the line selected, `f`,
        // is row 3.
        assert_eq!(c.row, 5);
        assert_eq!(c.selection.map(|s| s.row), Some(3));
    }

    /// A real terminal whose scrollback is full of the same line, with one line that is unique
    /// and a cursor a fixed distance below it.
    fn identical_lines_with_a_mark() -> (arcade_term::Term, Copy, usize) {
        let mut config = arcade_term::TermConfig::new(20, 5);
        config.scrollback_lines = 100;
        let mut term = arcade_term::Term::new(config).unwrap();
        feed_lines(&mut term, "same", 2500);
        feed_lines(&mut term, "MARK", 1);
        feed_lines(&mut term, "same", 100);
        let distance = 50;
        let at = mark_row(&term).unwrap() + distance;
        term.anchor_last_row();
        let mut c = Copy::enter(&term, term.cursor_position(), 20, 5);
        c.go(&term, at, 0);
        (term, c, distance)
    }

    fn feed_lines(term: &mut arcade_term::Term, text: &str, count: usize) {
        for _ in 0..count {
            term.feed(format!("{text}\r\n").as_bytes());
        }
    }

    fn mark_row(term: &arcade_term::Term) -> Option<usize> {
        (0..term.total_rows()).find(|y| {
            term.line(*y)
                .is_some_and(|l| cells_text(&l.cells).trim_end() == "MARK")
        })
    }

    #[test]
    fn the_cursor_keeps_its_line_when_identical_lines_are_cut_off_the_top() {
        let (mut term, mut c, distance) = identical_lines_with_a_mark();
        let before = mark_row(&term).unwrap();
        // Print the same line until the library has cut old lines off the top (it does so a
        // page at a time).
        let mut cut = 0;
        for _ in 0..60 {
            feed_lines(&mut term, "same", 200);
            cut = before - mark_row(&term).unwrap_or(before);
            if cut > 0 {
                break;
            }
        }
        assert!(cut > 0, "no line was cut off the top");
        c.scroll(&term, 0);
        assert_eq!(
            c.row - mark_row(&term).unwrap(),
            distance,
            "{cut} lines were cut off the top"
        );
    }

    #[test]
    fn the_selection_keeps_its_line_when_identical_lines_are_cut_off_the_top() {
        let (mut term, mut c, distance) = identical_lines_with_a_mark();
        c.apply(&term, CopyCommand::SelectLine);
        let before = mark_row(&term).unwrap();
        for _ in 0..60 {
            feed_lines(&mut term, "same", 200);
            if mark_row(&term).is_some_and(|m| m < before) {
                break;
            }
        }
        c.scroll(&term, 0);
        let mark = mark_row(&term).unwrap();
        assert_eq!(c.selection.map(|s| s.row), Some(mark + distance));
        assert_eq!(c.row, mark + distance);
    }

    #[test]
    fn a_selected_line_cut_off_the_top_is_dropped_and_the_message_says_so() {
        let before = rows(&["a", "b", "c", "d", "e", "f", "g", "h"]);
        let mut c = copy_at(&before, 1, 0);
        c.apply(&before, CopyCommand::SelectLine);
        // Everything that was there is gone.
        let after = rows(&["1", "2", "3", "4", "5", "6", "7", "8"]);
        let outcome = c.apply(&after, CopyCommand::Down);
        assert_eq!(
            outcome,
            Outcome::Message("the selected text is no longer in the scrollback".to_string())
        );
        assert!(c.selection.is_none());
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
    fn changing_from_characters_to_lines_keeps_the_anchor_row_going_down() {
        let src = rows(&["one two", "three", "four", "five six"]);
        let mut c = copy_at(&src, 0, 4);
        run(&mut c, &src, &[CopyCommand::SelectChar]);
        c.go(&src, 2, 1);
        run(&mut c, &src, &[CopyCommand::SelectLine]);
        assert_eq!(c.selected_text(&src).unwrap(), "one two\nthree\nfour");
    }

    #[test]
    fn changing_from_characters_to_lines_keeps_the_anchor_row_going_up() {
        let src = rows(&["one two", "three", "four", "five six"]);
        let mut c = copy_at(&src, 3, 5);
        run(&mut c, &src, &[CopyCommand::SelectChar]);
        c.go(&src, 1, 2);
        run(&mut c, &src, &[CopyCommand::SelectLine]);
        assert_eq!(c.selected_text(&src).unwrap(), "three\nfour\nfive six");
    }

    #[test]
    fn changing_from_lines_to_characters_keeps_the_anchor_cell() {
        let src = rows(&["one two", "three", "four"]);
        let mut c = copy_at(&src, 0, 4);
        run(&mut c, &src, &[CopyCommand::SelectLine]);
        c.go(&src, 2, 1);
        run(&mut c, &src, &[CopyCommand::SelectChar]);
        assert_eq!(c.selected_text(&src).unwrap(), "two\nthree\nfo");
        // Upwards: from the anchor cell back to the cursor.
        let mut c = copy_at(&src, 2, 1);
        run(&mut c, &src, &[CopyCommand::SelectLine]);
        c.go(&src, 0, 4);
        run(&mut c, &src, &[CopyCommand::SelectChar]);
        assert_eq!(c.selected_text(&src).unwrap(), "two\nthree\nfo");
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
        // The mark of the place in the history is not what is looked at here.
        c.show_position = false;
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
