//! The floating list: a framed box in the middle of the screen with one line per item and the
//! chosen line marked. It is drawn over the screen on every update while it is open, and when it
//! closes the screen under it is drawn again in full, so nothing of it stays behind.
//!
//! What a chosen line means is decided by the code that opened the list; this file only keeps
//! the lines, the choice, and what to draw.

use arcade_term::Cell;

mod keylist;

pub use keylist::{KeyCells, KeyRow, KeyView, items as key_items, rows as key_rows};

use crate::compose::{blank, text_cells};
use crate::keymap::ListKeys;
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
    /// Go to the pane on the line, which has a notification; Delete clears it.
    Notifications,
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
    /// What a row of the key list says besides its action.
    pub keys: Option<KeyCells>,
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
            keys: None,
        }
    }

    /// The line is a row of the key list.
    pub fn with_keys(mut self, keys: KeyCells) -> Item {
        self.keys = Some(keys);
        self
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
    /// Set for the key list, which draws its lines in groups and columns.
    key_view: Option<KeyView>,
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
        let visible = (0..items.len())
            .filter(|i| !items[*i].keys.as_ref().is_some_and(|k| k.filter_only))
            .collect();
        Picker {
            kind,
            title: title.to_string(),
            items,
            filter: String::new(),
            visible,
            selected,
            key_view: None,
        }
    }

    /// The picker is the key list, with these column titles.
    pub fn with_key_view(mut self, headers: [String; 3]) -> Picker {
        self.key_view = Some(KeyView { headers });
        self
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
            // The rows that show only for a filter (the keys of copy mode) wait for one.
            .filter(|(_, item)| {
                item.search.contains(&wanted)
                    && (!wanted.is_empty() || !item.keys.as_ref().is_some_and(|k| k.filter_only))
            })
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

    /// The line of keys at the bottom of the box.
    fn key_line(&self, keys: &ListKeys, filtered: bool) -> String {
        hint_line(
            &self
                .key_items(keys, filtered)
                .into_iter()
                .map(|(item, _)| item)
                .collect::<Vec<_>>(),
        )
    }

    /// The items of the key line, each with how much it matters when there is no room for all
    /// of them (the higher, the longer it stays).
    fn key_items(&self, keys: &ListKeys, filtered: bool) -> Vec<(Option<String>, u8)> {
        let go = match self.kind {
            PickerKind::MoveWindow => "move here",
            PickerKind::Keys | PickerKind::Commands => "run",
            PickerKind::Sessions | PickerKind::Windows | PickerKind::Notifications => "go",
        };
        let typing = if filtered {
            keys_item(&[&keys.filter_clear], "clear filter")
        } else {
            Some("type to filter".to_string())
        };
        let clear = (self.kind == PickerKind::Notifications)
            .then(|| keys_item(&[&keys.clear], "clear"))
            .flatten();
        vec![
            (keys_item(&[&keys.up, &keys.down], "move"), 2),
            (keys_item(&[&keys.select], go), 3),
            (clear, 2),
            (typing, 1),
            (keys_item(&[&keys.cancel], "close"), 9),
        ]
    }

    /// The box for a screen of this size. Its size comes from every line it could show and from
    /// the key line in either state, not from what is shown now, so typing a filter or moving
    /// about never moves the frame.
    pub fn draw(&self, cols: u16, rows: u16, keys: &ListKeys) -> Drawn {
        if self.key_view.is_some() {
            return keylist::draw(self, cols, rows, keys);
        }
        let mut lines: Vec<String> = self
            .shown()
            .enumerate()
            .map(|(i, item)| self.line(item, i == self.selected))
            .collect();
        let nothing = self.visible.is_empty();
        let no_match = format!("  {}", theme::symbol(Symbol::PickerNoMatch));
        if nothing {
            lines.push(no_match.clone());
        }
        let selected = (!nothing).then_some(self.selected);
        let filtered = !self.filter.is_empty();
        let widest = self
            .items
            .iter()
            .flat_map(|item| [self.line(item, true), self.line(item, false)])
            .chain([no_match])
            .map(|l| cells_of(&l))
            .chain([cells_of(&self.title) + 2])
            .chain([false, true].map(|f| cells_of(&self.key_line(keys, f))))
            .max()
            .unwrap_or(0);
        draw_box(&BoxSpec {
            title: &self.title,
            filter: &self.filter,
            lines: &lines,
            selected,
            footers: &[self.key_line(keys, filtered)],
            widest,
            list_rows: self.items.len().max(1),
            margin: (0, 0),
            cols,
            rows,
        })
    }
}

/// How many cells a text takes.
pub fn cells_of(text: &str) -> usize {
    text_cells(text, Default::default()).len()
}

/// One item of a key line: the keys, then what they do (`Up Down move`); keys that have no
/// binding are left out, and when none has one there is no item.
pub fn keys_item(keys: &[&Option<String>], what: &str) -> Option<String> {
    let names: Vec<&str> = keys.iter().filter_map(|k| k.as_deref()).collect();
    (!names.is_empty()).then(|| format!("{} {what}", names.join(" ")))
}

/// The items that exist, side by side, as many as fit in `width` cells: when they do not all
/// fit, the one that matters least goes first, and the one that matters most always stays.
pub fn fit_hint_line(mut items: Vec<(Option<String>, u8)>, width: usize) -> String {
    loop {
        let line = hint_line(&items.iter().map(|(i, _)| i.clone()).collect::<Vec<_>>());
        if cells_of(&line) <= width {
            return line;
        }
        let least = items
            .iter()
            .enumerate()
            .filter(|(_, (item, _))| item.is_some())
            .min_by_key(|(_, (_, rank))| *rank)
            .map(|(at, _)| at);
        match least {
            Some(at) if items.iter().filter(|(i, _)| i.is_some()).count() > 1 => {
                items.remove(at);
            }
            _ => return line,
        }
    }
}

/// The items that exist, side by side.
pub fn hint_line(items: &[Option<String>]) -> String {
    items
        .iter()
        .flatten()
        .map(String::as_str)
        .collect::<Vec<_>>()
        .join("  ")
}

/// Give the notification marks in a line the colour of their class, as the status bar does, and
/// leave the rest of the cell's style (the background of the chosen line) as it is.
fn color_notice_marks(cells: &mut [Cell]) {
    for class in crate::notify::Class::BY_URGENCY {
        let mark: Vec<String> = theme::notice_symbol(class)
            .chars()
            .map(|c| c.to_string())
            .collect();
        let n = mark.len();
        let marked = theme::notice_style(class);
        let mut i = 0;
        while i + n <= cells.len() {
            if (0..n).all(|k| cells[i + k].text == mark[k]) {
                for cell in &mut cells[i..i + n] {
                    cell.style.fg = marked.fg;
                    cell.style.bold = marked.bold;
                }
                i += n;
            } else {
                i += 1;
            }
        }
    }
}

/// What a framed box is made of.
pub struct BoxSpec<'a> {
    pub title: &'a str,
    pub filter: &'a str,
    /// The lines that show now.
    pub lines: &'a [String],
    pub selected: Option<usize>,
    /// Lines of their own under the list; they never widen the box, and what does not fit is cut.
    pub footers: &'a [String],
    /// The widest text the box could ever show, in cells, from everything it holds and not
    /// from what is chosen or typed now.
    pub widest: usize,
    /// The rows the list may take: its length with nothing filtered away. Rows that hold no
    /// line are drawn empty, so the frame does not change size.
    pub list_rows: usize,
    /// Columns kept free at each side, and rows above and below, around the box.
    pub margin: (usize, usize),
    pub cols: u16,
    pub rows: u16,
}

/// The cells cut to `width`; when they were longer, the last of them is the theme's ellipsis.
fn cut(mut cells: Vec<Cell>, width: usize, pad: arcade_term::Style) -> Vec<Cell> {
    if cells.len() > width {
        cells.truncate(width.saturating_sub(1));
        if cells
            .last()
            .is_some_and(|c| c.width == arcade_term::CellWidth::Wide)
        {
            cells.pop();
        }
        cells.extend(text_cells(theme::symbol(Symbol::Ellipsis), pad));
        cells.truncate(width);
    }
    while cells.len() < width {
        cells.push(blank(pad));
    }
    cells
}

/// Where a box goes and how big it is, from what it could hold and the size of the screen.
pub struct Geometry {
    /// The cells between the two sides of the frame.
    pub inner: usize,
    /// The rows of the body (between the filter line and the footers).
    pub shown: usize,
    /// How many footers there is room for.
    pub footers: usize,
    /// The first line of the list that shows, when the chosen line has to stay in sight.
    pub first: usize,
    pub x: u16,
    pub y: u16,
}

/// The box for a screen of this size. `margin` is the room to keep free at the sides and above
/// and below.
pub fn geometry(spec: &BoxSpec) -> Geometry {
    let (cols, rows) = (spec.cols, spec.rows);
    let (margin_x, margin_y) = spec.margin;
    // One column of room inside each side, and the frame.
    let most = usize::from(cols).saturating_sub(2 + 2 * margin_x).max(1);
    let inner = (spec.widest + 2).clamp(20.min(most), most);
    let width = inner + 2;
    // The frame and the filter line take three rows; each footer one more, when there is room.
    let footers = spec.footers.len().min(usize::from(rows).saturating_sub(3));
    let extra = 3 + footers;
    let room = usize::from(rows).saturating_sub(extra + 2 * margin_y);
    let shown = spec.list_rows.min(room);
    // The lines that show: the first ones, or those that end with the chosen one.
    let first = (spec.selected.unwrap_or(0) + 1).saturating_sub(shown);
    let height = shown + extra;
    let x = (usize::from(cols).saturating_sub(width) / 2) as u16;
    let y = (usize::from(rows).saturating_sub(height) / 2) as u16;
    Geometry {
        inner,
        shown,
        footers,
        first,
        x,
        y,
    }
}

/// A framed box: the title in the top edge, the filter line, the body rows (each with the style
/// of its blank space, and exactly `geometry.shown` of them), and a line of its own for each
/// footer.
pub fn assemble(
    geometry: &Geometry,
    title: &str,
    filter: &str,
    body: Vec<(Vec<Cell>, arcade_term::Style)>,
    footers: &[String],
) -> Drawn {
    let inner = geometry.inner;
    let filter_line = format!("{}{}", theme::symbol(Symbol::PickerFilterMark), filter);
    let plain = theme::style(Role::Picker);
    let side = |c: Symbol| text_cells(theme::symbol(c), plain);
    let mut out: Vec<Vec<Cell>> = Vec::new();

    let mut top = side(Symbol::PickerTopLeft);
    let title_cells = text_cells(&format!(" {title} "), plain);
    let rule = text_cells(theme::symbol(Symbol::PickerHorizontal), plain);
    let mut title_cells = if title_cells.len() > inner {
        cut(title_cells, inner, plain)
    } else {
        title_cells
    };
    while title_cells.len() < inner {
        title_cells.extend(rule.clone());
    }
    title_cells.truncate(inner);
    top.extend(title_cells);
    top.extend(side(Symbol::PickerTopRight));
    out.push(top);

    let framed = |body: Vec<Cell>, style: arcade_term::Style| {
        let mut row = side(Symbol::PickerVertical);
        row.extend(cut(body, inner, style));
        row.extend(side(Symbol::PickerVertical));
        row
    };
    let mut line = vec![blank(plain)];
    line.extend(text_cells(&filter_line, plain));
    out.push(framed(line, plain));

    let mut body = body.into_iter();
    for _ in 0..geometry.shown {
        match body.next() {
            Some((cells, style)) => out.push(framed(cells, style)),
            None => out.push(framed(Vec::new(), plain)),
        }
    }

    for footer in footers.iter().take(geometry.footers) {
        let mut line = vec![blank(plain)];
        line.extend(text_cells(footer, plain));
        out.push(framed(line, plain));
    }

    let mut bottom = side(Symbol::PickerBottomLeft);
    for _ in 0..inner {
        bottom.extend(rule.clone());
    }
    bottom.extend(side(Symbol::PickerBottomRight));
    out.push(bottom);
    Drawn {
        x: geometry.x,
        y: geometry.y,
        rows: out,
    }
}

/// A framed box for a screen of this size: the title in the top edge, the filter line, the lines
/// (the ones that fit, ending with the chosen one when it would be out of sight, and empty rows
/// down to the size the whole list would take), and a line of its own for each footer.
pub fn draw_box(spec: &BoxSpec) -> Drawn {
    let geometry = geometry(spec);
    let plain = theme::style(Role::Picker);
    let chosen = theme::style(Role::PickerSelected);
    let mut body = Vec::new();
    for slot in 0..geometry.shown {
        let index = geometry.first + slot;
        if let Some(line) = spec.lines.get(index) {
            let style = if Some(index) == spec.selected {
                chosen
            } else {
                plain
            };
            let mut cells = vec![blank(style)];
            cells.extend(text_cells(line, style));
            color_notice_marks(&mut cells);
            body.push((cells, style));
        }
    }
    assemble(&geometry, spec.title, spec.filter, body, spec.footers)
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

    fn keys() -> ListKeys {
        let keymap = crate::keymap::Keymap::defaults(&Default::default());
        ListKeys::new(&keymap.list)
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
        let d = p.draw(80, 24, &keys());
        // The frame, the filter line, a line for each item and the key line.
        assert_eq!(d.rows.len(), 6);
        let width = d.rows[0].len();
        assert!(d.rows.iter().all(|r| r.len() == width));
        assert_eq!(usize::from(d.x), (80 - width) / 2);
        assert_eq!(usize::from(d.y), (24 - 6) / 2);
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
        let d = p.draw(80, 10, &keys());
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
        let d = p.draw(12, 3, &keys());
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
        assert!(text(&p.draw(80, 24, &keys()).rows[2]).contains("no match"));
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

    #[test]
    fn the_box_has_the_same_place_and_size_whatever_the_filter_leaves() {
        let names: Vec<String> = (0..9).map(|i| format!("session-{i}")).collect();
        let refs: Vec<&str> = names.iter().map(String::as_str).collect();
        let mut p = Picker::new(PickerKind::Sessions, "Sessions", items(&refs), 0);
        let shape = |p: &Picker| {
            let d = p.draw(100, 30, &keys());
            (d.x, d.y, d.rows.len(), d.rows[0].len())
        };
        let base = shape(&p);
        for typed in ["session-1", "x", "zzz"] {
            p.type_text(typed);
            assert_eq!(shape(&p), base, "after typing {typed}");
            p.clear_filter();
            assert_eq!(shape(&p), base);
        }
    }

    #[test]
    fn the_key_line_is_built_from_the_keys_and_never_widens_the_box() {
        let p = Picker::new(PickerKind::Notifications, "Notifications", items(&["a"]), 0);
        let wide = p.draw(100, 30, &keys());
        let line = text(&wide.rows[wide.rows.len() - 2]);
        assert!(
            line.contains("up down move") && line.contains("delete clear"),
            "{line}"
        );
        // On a narrow screen the line is cut with the ellipsis and the box stays inside.
        let narrow = p.draw(30, 30, &keys());
        assert!(narrow.rows[0].len() <= 30);
        let line = text(&narrow.rows[narrow.rows.len() - 2]);
        assert!(line.contains('\u{2026}'), "{line}");
        assert!(narrow.rows.iter().all(|r| r.len() == narrow.rows[0].len()));
    }

    #[test]
    fn a_hint_line_that_does_not_fit_loses_the_least_important_hints_first() {
        let items = || {
            vec![
                (Some("up down move".to_string()), 2),
                (Some("enter run".to_string()), 3),
                (None, 2),
                (Some("type to filter".to_string()), 1),
                (Some("esc close".to_string()), 9),
            ]
        };
        let all = fit_hint_line(items(), 200);
        assert_eq!(all, "up down move  enter run  type to filter  esc close");
        assert_eq!(
            fit_hint_line(items(), 36),
            "up down move  enter run  esc close"
        );
        assert_eq!(fit_hint_line(items(), 20), "enter run  esc close");
        // The one that matters most stays even when it does not fit either.
        assert_eq!(fit_hint_line(items(), 3), "esc close");
    }
}
