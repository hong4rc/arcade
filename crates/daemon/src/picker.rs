//! The floating list: a framed box in the middle of the screen with one line per item and the
//! chosen line marked. It is drawn over the screen on every update while it is open, and when it
//! closes the screen under it is drawn again in full, so nothing of it stays behind.
//!
//! What a chosen line means is decided by the code that opened the list; this file only keeps
//! the lines, the choice, and what to draw.

use arcade_term::Cell;

use crate::compose::{blank, fit, text_cells};
use crate::theme::{self, Role, Symbol};

/// What choosing a line does.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PickerKind {
    /// Go to the session on the line.
    Sessions,
    /// Move the current window to the session on the line.
    MoveWindow,
    /// Run the command of the key binding on the line.
    Keys,
    /// Go to the window of the current session on the line.
    Windows,
    /// Run the command on the line.
    Commands,
}

pub struct Item {
    /// What the code that opened the list knows the line by (a position in its own list).
    pub id: usize,
    /// The words of the line; for the sessions, the session's name.
    pub name: String,
    /// What is said after the name.
    pub detail: String,
    /// The line stands for what is current (the session shown).
    pub current: bool,
    /// The words the filter looks in, in lower case.
    search: String,
}

impl Item {
    /// A line the filter finds by its name.
    pub fn new(id: usize, name: String, detail: String, current: bool) -> Item {
        let search = name.to_lowercase();
        Item {
            id,
            name,
            detail,
            current,
            search,
        }
    }

    /// The filter also finds the line by these words.
    pub fn also_found_by(mut self, words: &str) -> Item {
        self.search.push(' ');
        self.search.push_str(&words.to_lowercase());
        self
    }
}

pub struct Picker {
    pub kind: PickerKind,
    pub title: String,
    /// Every line; `visible` says which ones the filter lets through.
    items: Vec<Item>,
    /// What was typed to filter the lines.
    filter: String,
    /// The positions in `items` that show, in order.
    visible: Vec<usize>,
    /// The chosen line, as a position in `visible`.
    pub selected: usize,
}

/// Where the box is and what is in it, in the cells of the client's screen.
pub struct Drawn {
    pub x: u16,
    pub y: u16,
    pub rows: Vec<Vec<Cell>>,
}

impl Picker {
    pub fn new(kind: PickerKind, title: &str, items: Vec<Item>, selected: usize) -> Picker {
        let selected = selected.min(items.len().saturating_sub(1));
        let visible = (0..items.len()).collect();
        Picker {
            kind,
            title: title.to_string(),
            items,
            filter: String::new(),
            visible,
            selected,
        }
    }

    /// Add typed text to the filter.
    pub fn type_text(&mut self, text: &str) {
        self.filter.push_str(text);
        self.refilter();
    }

    /// Take the last typed character off the filter.
    pub fn delete_char(&mut self) {
        self.filter.pop();
        self.refilter();
    }

    pub fn clear_filter(&mut self) {
        self.filter.clear();
        self.refilter();
    }

    /// The lines that match show, and the first of them is chosen.
    fn refilter(&mut self) {
        let wanted = self.filter.to_lowercase();
        self.visible = self
            .items
            .iter()
            .enumerate()
            .filter(|(_, item)| item.search.contains(&wanted))
            .map(|(i, _)| i)
            .collect();
        self.selected = 0;
    }

    /// The lines that show now.
    #[cfg_attr(not(test), allow(dead_code))] // tests look at what the filter leaves
    pub fn shown(&self) -> impl Iterator<Item = &Item> {
        self.visible.iter().map(|i| &self.items[*i])
    }

    pub fn up(&mut self) {
        self.selected = self.selected.saturating_sub(1);
    }

    pub fn down(&mut self) {
        if self.selected + 1 < self.visible.len() {
            self.selected += 1;
        }
    }

    pub fn chosen(&self) -> Option<&Item> {
        self.visible.get(self.selected).map(|i| &self.items[*i])
    }

    fn line(&self, item: &Item, selected: bool) -> String {
        let mark = theme::symbol(if selected {
            Symbol::PickerSelectedMark
        } else {
            Symbol::PickerOtherMark
        });
        let current = if item.current {
            theme::symbol(Symbol::PickerCurrentMark)
        } else {
            ""
        };
        if item.detail.is_empty() {
            format!("{mark}{}{current}", item.name)
        } else {
            format!("{mark}{}  {}{current}", item.name, item.detail)
        }
    }

    /// The box for a screen of this size.
    pub fn draw(&self, cols: u16, rows: u16) -> Drawn {
        let mut lines: Vec<String> = self
            .shown()
            .enumerate()
            .map(|(i, item)| self.line(item, i == self.selected))
            .collect();
        let filter_line = format!("{}{}", theme::symbol(Symbol::PickerFilterMark), self.filter);
        let nothing = self.visible.is_empty();
        if nothing {
            lines.push(format!("  {}", theme::symbol(Symbol::PickerNoMatch)));
        }
        let widest = lines
            .iter()
            .map(|l| text_cells(l, Default::default()).len())
            .chain(std::iter::once(
                text_cells(&filter_line, Default::default()).len(),
            ))
            .chain(std::iter::once(
                text_cells(&self.title, Default::default()).len() + 2,
            ))
            .max()
            .unwrap_or(0);
        // One column of room inside each side, and the frame.
        let most = usize::from(cols).saturating_sub(2).max(1);
        let inner = (widest + 2).clamp(20.min(most), most);
        let width = inner + 2;
        // The frame and the filter line take three rows.
        let room = usize::from(rows).saturating_sub(3);
        let shown = lines.len().min(room);
        // The lines that show: the first ones, or those that end with the chosen one.
        let first = (self.selected + 1).saturating_sub(shown);
        let height = shown + 3;
        let x = (usize::from(cols).saturating_sub(width) / 2) as u16;
        let y = (usize::from(rows).saturating_sub(height) / 2) as u16;

        let plain = theme::style(Role::Picker);
        let chosen = theme::style(Role::PickerSelected);
        let side = |c: Symbol| text_cells(theme::symbol(c), plain);
        let mut out: Vec<Vec<Cell>> = Vec::with_capacity(height);

        let mut top = side(Symbol::PickerTopLeft);
        let title = format!(" {} ", self.title);
        let mut title_cells = fit(text_cells(&title, plain), inner, plain);
        let rule = text_cells(theme::symbol(Symbol::PickerHorizontal), plain);
        for (i, cell) in title_cells.iter_mut().enumerate() {
            if i >= text_cells(&title, plain).len() {
                *cell = rule[0].clone();
            }
        }
        top.extend(title_cells);
        top.extend(side(Symbol::PickerTopRight));
        out.push(top);

        let mut filter_row = side(Symbol::PickerVertical);
        let mut body = vec![blank(plain)];
        body.extend(text_cells(&filter_line, plain));
        filter_row.extend(fit(body, inner, plain));
        filter_row.extend(side(Symbol::PickerVertical));
        out.push(filter_row);

        for (index, line) in lines.iter().enumerate().skip(first).take(shown) {
            let style = if index == self.selected && !nothing {
                chosen
            } else {
                plain
            };
            let mut row = side(Symbol::PickerVertical);
            let mut body = vec![blank(style)];
            body.extend(text_cells(line, style));
            row.extend(fit(body, inner, style));
            row.extend(side(Symbol::PickerVertical));
            out.push(row);
        }

        let mut bottom = side(Symbol::PickerBottomLeft);
        for _ in 0..inner {
            bottom.extend(rule.clone());
        }
        bottom.extend(side(Symbol::PickerBottomRight));
        out.push(bottom);
        Drawn { x, y, rows: out }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn items(names: &[&str]) -> Vec<Item> {
        names
            .iter()
            .enumerate()
            .map(|(i, n)| Item::new(i, (*n).to_string(), format!("{} windows", i + 1), i == 0))
            .collect()
    }

    fn text(row: &[Cell]) -> String {
        row.iter()
            .map(|c| {
                if c.text.is_empty() {
                    " "
                } else {
                    c.text.as_str()
                }
            })
            .collect()
    }

    #[test]
    fn the_choice_moves_and_stops_at_the_ends() {
        let mut p = Picker::new(PickerKind::Sessions, "Sessions", items(&["a", "b", "c"]), 0);
        p.up();
        assert_eq!(p.selected, 0);
        p.down();
        p.down();
        p.down();
        assert_eq!(p.selected, 2);
        assert_eq!(p.chosen().unwrap().name, "c");
    }

    #[test]
    fn the_box_is_in_the_middle_with_one_line_for_each_item() {
        let p = Picker::new(
            PickerKind::Sessions,
            "Sessions",
            items(&["alpha", "beta"]),
            1,
        );
        let d = p.draw(80, 24);
        assert_eq!(d.rows.len(), 5);
        let width = d.rows[0].len();
        assert!(d.rows.iter().all(|r| r.len() == width));
        assert_eq!(usize::from(d.x), (80 - width) / 2);
        assert_eq!(usize::from(d.y), (24 - 5) / 2);
        assert!(text(&d.rows[0]).contains("Sessions"));
        assert!(text(&d.rows[1]).contains("/ "));
        assert!(text(&d.rows[2]).contains("  alpha"));
        assert!(text(&d.rows[2]).contains('*'));
        assert!(text(&d.rows[3]).contains("> beta"));
        // The chosen line has its own look.
        assert_ne!(d.rows[2][3].style, d.rows[3][3].style);
    }

    #[test]
    fn a_long_list_shows_the_part_around_the_choice() {
        let names: Vec<String> = (0..30).map(|i| format!("s{i}")).collect();
        let refs: Vec<&str> = names.iter().map(String::as_str).collect();
        let p = Picker::new(PickerKind::Sessions, "Sessions", items(&refs), 25);
        let d = p.draw(80, 10);
        assert_eq!(d.rows.len(), 10);
        assert!(d.rows.iter().any(|r| text(r).contains("> s25")));
    }

    #[test]
    fn a_small_screen_gets_a_box_that_fits() {
        let p = Picker::new(
            PickerKind::Sessions,
            "Sessions",
            items(&["a-rather-long-name-indeed"]),
            0,
        );
        let d = p.draw(12, 3);
        assert!(d.rows[0].len() <= 12);
        assert!(d.rows.len() <= 3);
    }

    #[test]
    fn typing_leaves_the_lines_that_hold_the_text_and_deleting_brings_them_back() {
        let mut p = Picker::new(
            PickerKind::Sessions,
            "Sessions",
            items(&["a", "ab", "b"]),
            2,
        );
        p.type_text("a");
        assert_eq!(
            p.shown().map(|i| i.name.as_str()).collect::<Vec<_>>(),
            ["a", "ab"]
        );
        assert_eq!(p.chosen().unwrap().name, "a");
        p.type_text("B");
        assert_eq!(
            p.shown().map(|i| i.name.as_str()).collect::<Vec<_>>(),
            ["ab"]
        );
        p.delete_char();
        p.delete_char();
        assert_eq!(p.shown().count(), 3);
        p.type_text("zzz");
        assert!(p.chosen().is_none());
        assert!(text(&p.draw(80, 24).rows[2]).contains("no match"));
        p.clear_filter();
        assert_eq!(p.shown().count(), 3);
    }

    #[test]
    fn the_filter_also_finds_a_line_by_the_words_it_was_given() {
        let mut p = Picker::new(
            PickerKind::Keys,
            "Keys",
            vec![
                Item::new(0, "Ctrl+m z".into(), "pane.zoom".into(), false)
                    .also_found_by("pane.zoom"),
            ],
            0,
        );
        p.type_text("zoom");
        assert_eq!(p.shown().count(), 1);
        assert_eq!(p.chosen().unwrap().id, 0);
    }
}
