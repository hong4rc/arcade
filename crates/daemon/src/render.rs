//! Turn what changed on a pane's screen into the bytes that draw it on the host terminal.
//!
//! The colors are the ones the program asked for (or the theme's): palette numbers stay palette
//! numbers, so the host terminal's own theme applies. Nothing here picks a color.

use arcade_term::{Cell, CellWidth, Color, Cursor, CursorShape, LinkSpan, Style, Underline};

use crate::compose::Screen;

fn push_color(out: &mut Vec<u8>, color: Color, base: u8) {
    // `base` is 30 for the foreground and 40 for the background.
    match color {
        Color::Default => {}
        Color::Indexed(n) if n < 8 => {
            out.extend_from_slice(format!(";{}", base + n).as_bytes());
        }
        Color::Indexed(n) if n < 16 => {
            out.extend_from_slice(format!(";{}", base + 60 + (n - 8)).as_bytes());
        }
        Color::Indexed(n) => {
            out.extend_from_slice(format!(";{};5;{n}", base + 8).as_bytes());
        }
        Color::Rgb(r, g, b) => {
            out.extend_from_slice(format!(";{};2;{r};{g};{b}", base + 8).as_bytes());
        }
    }
}

/// Write the escape sequence that makes `style` current, starting from a reset.
fn push_style(out: &mut Vec<u8>, style: &Style) {
    out.extend_from_slice(b"\x1b[0");
    if style.bold {
        out.extend_from_slice(b";1");
    }
    if style.faint {
        out.extend_from_slice(b";2");
    }
    if style.italic {
        out.extend_from_slice(b";3");
    }
    match style.underline {
        Underline::None => {}
        Underline::Single => out.extend_from_slice(b";4"),
        Underline::Double => out.extend_from_slice(b";4:2"),
        Underline::Curly => out.extend_from_slice(b";4:3"),
        Underline::Dotted => out.extend_from_slice(b";4:4"),
        Underline::Dashed => out.extend_from_slice(b";4:5"),
    }
    if style.blink {
        out.extend_from_slice(b";5");
    }
    if style.inverse {
        out.extend_from_slice(b";7");
    }
    if style.invisible {
        out.extend_from_slice(b";8");
    }
    if style.strike {
        out.extend_from_slice(b";9");
    }
    if style.overline {
        out.extend_from_slice(b";53");
    }
    push_color(out, style.fg, 30);
    push_color(out, style.bg, 40);
    match style.underline_color {
        Color::Default => {}
        Color::Indexed(n) => out.extend_from_slice(format!(";58;5;{n}").as_bytes()),
        Color::Rgb(r, g, b) => out.extend_from_slice(format!(";58;2;{r};{g};{b}").as_bytes()),
    }
    out.push(b'm');
}

/// Make `to` current when `from` is: when only the colors differ, say only the colors that do
/// (a tab bar of true colors changes them at every block, and a whole reset and both colors each
/// time costs several times more bytes).
fn push_style_change(out: &mut Vec<u8>, from: &Style, to: &Style) {
    let same_apart_from_colors = Style {
        fg: to.fg,
        bg: to.bg,
        ..*from
    } == *to;
    if !same_apart_from_colors {
        push_style(out, to);
        return;
    }
    out.extend_from_slice(b"\x1b[");
    let mut first = true;
    let mut part = |out: &mut Vec<u8>, from: Color, to: Color, base: u8| {
        if from == to {
            return;
        }
        if !first {
            out.push(b';');
        }
        first = false;
        match to {
            Color::Default => out.extend_from_slice((base + 9).to_string().as_bytes()),
            _ => {
                let mut one = Vec::new();
                push_color(&mut one, to, base);
                // `push_color` leads with a separator.
                out.extend_from_slice(&one[1..]);
            }
        }
    };
    part(out, from.fg, to.fg, 30);
    part(out, from.bg, to.bg, 40);
    out.push(b'm');
}

fn is_plain_blank(cell: &Cell) -> bool {
    cell.text.is_empty() && cell.style == Style::default()
}

/// Open a link on the host terminal (OSC 8). The id is made from the address, so a link that
/// goes on over several rows, or over two runs, is one link for the host terminal.
fn push_link_open(out: &mut Vec<u8>, uri: &str) {
    use std::hash::{Hash, Hasher};
    let mut hasher = std::collections::hash_map::DefaultHasher::new();
    uri.hash(&mut hasher);
    out.extend_from_slice(format!("\x1b]8;id={:x};", hasher.finish()).as_bytes());
    out.extend_from_slice(uri.as_bytes());
    out.extend_from_slice(b"\x1b\\");
}

const LINK_CLOSE: &[u8] = b"\x1b]8;;\x1b\\";

/// Draw one row: move to it, then write every column (or erase the plain tail). The cells in
/// `links` are written inside a link, which is closed again before anything else.
pub(crate) fn push_row(out: &mut Vec<u8>, y: u16, cells: &[Cell], links: &[LinkSpan]) {
    out.extend_from_slice(format!("\x1b[{};1H", y + 1).as_bytes());
    // Leave out a tail of plain blank cells and erase it instead.
    let mut end = cells.len();
    while end > 0 && is_plain_blank(&cells[end - 1]) {
        end -= 1;
    }
    let mut current = Style::default();
    out.extend_from_slice(b"\x1b[0m");
    let mut open_until: Option<u16> = None;
    let mut next_link = links.iter().peekable();
    for (x, cell) in cells[..end].iter().enumerate() {
        let x = x as u16;
        if open_until == Some(x) {
            out.extend_from_slice(LINK_CLOSE);
            open_until = None;
        }
        if open_until.is_none()
            && let Some(span) = next_link.next_if(|span| span.from <= x && x < span.to)
        {
            push_link_open(out, &span.uri);
            open_until = Some(span.to);
        }
        if cell.width == CellWidth::SpacerTail {
            continue;
        }
        if cell.style != current {
            push_style_change(out, &current, &cell.style);
            current = cell.style;
        }
        if cell.text.is_empty() {
            out.push(b' ');
        } else {
            out.extend_from_slice(cell.text.as_bytes());
        }
    }
    if open_until.is_some() {
        out.extend_from_slice(LINK_CLOSE);
    }
    if end < cells.len() {
        if current != Style::default() {
            out.extend_from_slice(b"\x1b[0m");
        }
        out.extend_from_slice(b"\x1b[K");
    }
}

/// What brings a row that the host shows as `before` to `now` when only a few cells differ: for
/// each run of different cells, a move to it and the cells. `None` when the rows have another
/// width or a large part differs (the whole row is cheaper then, and simpler).
fn changed_cells(y: u16, before: &[Cell], now: &[Cell]) -> Option<Vec<u8>> {
    if before.len() != now.len() {
        return None;
    }
    let mut out = Vec::new();
    let mut x = 0;
    let mut changed = 0;
    while x < now.len() {
        if before[x] == now[x] {
            x += 1;
            continue;
        }
        // A run starts on the first cell of a wide character and ends after its last one.
        let mut from = x;
        while from > 0 && now[from].width == CellWidth::SpacerTail {
            from -= 1;
        }
        let mut to = x + 1;
        loop {
            while to < now.len() && now[to].width == CellWidth::SpacerTail {
                to += 1;
            }
            if to < now.len() && before[to] != now[to] {
                to += 1;
            } else {
                break;
            }
        }
        changed += to - from;
        if changed * 4 > now.len() {
            return None;
        }
        out.extend_from_slice(format!("\x1b[{};{}H\x1b[0m", y + 1, from + 1).as_bytes());
        let mut current = Style::default();
        for cell in &now[from..to] {
            if cell.width == CellWidth::SpacerTail {
                continue;
            }
            if cell.style != current {
                push_style_change(&mut out, &current, &cell.style);
                current = cell.style;
            }
            if cell.text.is_empty() {
                out.push(b' ');
            } else {
                out.extend_from_slice(cell.text.as_bytes());
            }
        }
        x = to;
    }
    Some(out)
}

fn push_cursor(out: &mut Vec<u8>, cursor: &Cursor) {
    let shape = match (cursor.shape, cursor.blinking) {
        (CursorShape::Block | CursorShape::HollowBlock, true) => 1,
        (CursorShape::Block | CursorShape::HollowBlock, false) => 2,
        (CursorShape::Underline, true) => 3,
        (CursorShape::Underline, false) => 4,
        (CursorShape::Bar, true) => 5,
        (CursorShape::Bar, false) => 6,
    };
    out.extend_from_slice(format!("\x1b[{shape} q").as_bytes());
    // The position is sent also while the cursor is hidden, so that the host terminal's cursor
    // is where the pane's is whenever it is shown again.
    out.extend_from_slice(format!("\x1b[{};{}H", cursor.y + 1, cursor.x + 1).as_bytes());
    out.extend_from_slice(if cursor.visible {
        b"\x1b[?25h"
    } else {
        b"\x1b[?25l"
    });
}

/// The modes of the pane that the host terminal must share, so that the keys it sends are the
/// ones the program expects.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
pub struct HostModes {
    /// Arrow keys send `ESC O A` instead of `ESC [ A` (DECCKM).
    pub app_cursor_keys: bool,
    /// Pasted text is wrapped in `ESC [ 200 ~` and `ESC [ 201 ~`.
    pub bracketed_paste: bool,
    /// The host terminal reports the mouse (presses, releases, moves with a button held, the
    /// wheel), in the SGR form.
    pub mouse: bool,
}

/// Ask the host terminal for the mouse, and give it back.
pub const MOUSE_ON: &[u8] = b"\x1b[?1002h\x1b[?1006h";
pub const MOUSE_OFF: &[u8] = b"\x1b[?1002l\x1b[?1006l";

#[derive(Clone, Copy, PartialEq, Eq)]
struct SentCursor {
    x: u16,
    y: u16,
    visible: bool,
    shape: CursorShape,
    blinking: bool,
}

/// Remembers what the host terminal shows and turns each new frame into the smallest update.
///
/// Rows the library calls changed are compared with what was last sent, so a row that is
/// identical (for instance one the cursor merely passed over) is not sent again.
#[derive(Default)]
pub struct Renderer {
    rows: Vec<Option<Vec<u8>>>,
    /// The row of the status bar and its cells as last sent: when only a few of them change (a
    /// symbol that blinks), only those are sent.
    bar_row: Option<u16>,
    bar_cells: Option<Vec<Cell>>,
    cursor: Option<SentCursor>,
    modes: Option<HostModes>,
}

impl Renderer {
    pub fn new() -> Self {
        Self::default()
    }

    /// Say which row is the status bar (none when it is hidden).
    pub fn set_bar_row(&mut self, row: Option<u16>) {
        if self.bar_row != row {
            self.bar_row = row;
            self.bar_cells = None;
        }
    }

    /// Forget what the host shows: the next update sends everything. Used when a client
    /// attaches and when the screen changes size.
    pub fn reset(&mut self) {
        *self = Self::default();
    }

    /// The bytes that bring the host terminal up to date with `screen`; empty when it already is.
    ///
    /// Only the rows the grid marks as written to are looked at, and one that comes out
    /// identical to what was last sent (for instance one the cursor merely passed over) is not
    /// sent again. Drawing rows is wrapped in synchronized output, so the host shows the update at
    /// once.
    pub fn render(&mut self, screen: &mut Screen, cursor: &Cursor, modes: HostModes) -> Vec<u8> {
        if self.rows.len() != usize::from(screen.rows()) {
            self.rows = vec![None; usize::from(screen.rows())];
        }
        let mut body = Vec::new();
        for y in 0..screen.rows() {
            if !screen.take_dirty(y) {
                continue;
            }
            let mut bytes = Vec::new();
            push_row(&mut bytes, y, screen.row(y), screen.row_links(y));
            let slot = &mut self.rows[usize::from(y)];
            if slot.as_deref() != Some(bytes.as_slice()) {
                let cells = screen.row(y);
                let small = (self.bar_row == Some(y) && screen.row_links(y).is_empty())
                    .then_some(self.bar_cells.as_deref())
                    .flatten()
                    .and_then(|before| changed_cells(y, before, cells))
                    .filter(|small| small.len() < bytes.len());
                match small {
                    Some(small) => body.extend_from_slice(&small),
                    None => body.extend_from_slice(&bytes),
                }
                *slot = Some(bytes);
                if self.bar_row == Some(y) {
                    self.bar_cells = Some(cells.to_vec());
                }
            }
        }
        let sent = SentCursor {
            x: cursor.x,
            y: cursor.y,
            visible: cursor.visible,
            shape: cursor.shape,
            blinking: cursor.blinking,
        };
        let cursor_changed = self.cursor != Some(sent);
        let modes_changed = self.modes != Some(modes);
        if body.is_empty() && !cursor_changed && !modes_changed {
            return Vec::new();
        }
        let mut out = Vec::with_capacity(body.len() + 64);
        if !body.is_empty() {
            out.extend_from_slice(b"\x1b[?2026h\x1b[?25l");
            out.extend_from_slice(&body);
            out.extend_from_slice(b"\x1b[0m");
        }
        if !body.is_empty() || cursor_changed {
            push_cursor(&mut out, cursor);
        }
        if modes_changed {
            let previous = self.modes.unwrap_or_default();
            if self.modes.is_none() || previous.mouse != modes.mouse {
                out.extend_from_slice(if modes.mouse { MOUSE_ON } else { MOUSE_OFF });
            }
            if self.modes.is_none() || previous.app_cursor_keys != modes.app_cursor_keys {
                out.extend_from_slice(if modes.app_cursor_keys {
                    b"\x1b[?1h"
                } else {
                    b"\x1b[?1l"
                });
            }
            if self.modes.is_none() || previous.bracketed_paste != modes.bracketed_paste {
                out.extend_from_slice(if modes.bracketed_paste {
                    b"\x1b[?2004h"
                } else {
                    b"\x1b[?2004l"
                });
            }
        }
        if !body.is_empty() {
            out.extend_from_slice(b"\x1b[?2026l");
        }
        self.cursor = Some(sent);
        self.modes = Some(modes);
        out
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn has(haystack: &[u8], needle: &[u8]) -> bool {
        haystack.windows(needle.len()).any(|w| w == needle)
    }

    #[test]
    fn the_mouse_is_asked_for_once_and_given_back_when_it_is_turned_off() {
        let mut screen = Screen::new();
        screen.resize(10, 3);
        let cursor = Cursor {
            x: 0,
            y: 0,
            visible: true,
            shape: CursorShape::Block,
            blinking: false,
        };
        let mut renderer = Renderer::new();
        let on = HostModes {
            mouse: true,
            ..HostModes::default()
        };
        let first = renderer.render(&mut screen, &cursor, on);
        assert!(has(&first, MOUSE_ON));
        let again = renderer.render(&mut screen, &cursor, on);
        assert!(
            !has(&again, b"?1002") && !has(&again, b"?1006"),
            "{again:?}"
        );
        let off = renderer.render(&mut screen, &cursor, HostModes::default());
        assert!(has(&off, MOUSE_OFF));
    }
}

#[cfg(test)]
mod link_tests {
    use super::*;

    fn cells(text: &str) -> Vec<Cell> {
        crate::compose::text_cells(text, Style::default())
    }

    fn span(from: u16, to: u16) -> LinkSpan {
        LinkSpan {
            from,
            to,
            uri: "https://a.example/x".into(),
        }
    }

    fn row(text: &str, links: &[LinkSpan]) -> String {
        let mut out = Vec::new();
        push_row(&mut out, 0, &cells(text), links);
        String::from_utf8(out).unwrap().replace('\x1b', "<E>")
    }

    #[test]
    fn a_link_is_opened_before_its_first_cell_and_closed_before_the_next_one() {
        let got = row("ab cd ef", &[span(3, 5)]);
        let id = got.split("id=").nth(1).unwrap().split(';').next().unwrap();
        assert_eq!(
            got,
            format!("<E>[1;1H<E>[0mab <E>]8;id={id};https://a.example/x<E>\\cd<E>]8;;<E>\\ ef")
        );
    }

    #[test]
    fn a_link_that_reaches_the_end_of_the_row_is_closed_there() {
        let got = row("ab", &[span(0, 2)]);
        assert!(got.ends_with("ab<E>]8;;<E>\\"), "{got}");
        // Blank cells at the end are erased rather than written; the link is closed before.
        let mut blanks = cells("ab");
        blanks.extend(vec![crate::compose::blank(Style::default()); 2]);
        let mut out = Vec::new();
        push_row(&mut out, 0, &blanks, &[span(0, 4)]);
        let got = String::from_utf8(out).unwrap().replace('\x1b', "<E>");
        assert!(got.contains("ab<E>]8;;<E>\\<E>[K"), "{got}");
    }

    #[test]
    fn two_links_next_to_each_other_are_closed_and_opened_in_turn() {
        let got = row("abcd", &[span(0, 2), span(2, 4)]);
        assert_eq!(got.matches("]8;;<E>\\").count(), 2, "{got}");
        assert_eq!(got.matches("https://a.example/x").count(), 2, "{got}");
        assert!(got.contains("ab<E>]8;;<E>\\<E>]8;id="), "{got}");
    }

    #[test]
    fn a_wide_character_is_inside_the_link_with_its_second_column() {
        let got = row("a漢b", &[span(1, 3)]);
        assert!(got.contains("漢<E>]8;;<E>\\b"), "{got}");
    }

    #[test]
    fn a_row_without_links_has_no_link_bytes() {
        assert!(!row("abc", &[]).contains("]8;"));
    }
}

#[cfg(test)]
mod changed_cell_tests {
    use super::*;

    fn bar(parts: &[(&str, u8)]) -> Vec<Cell> {
        let mut cells = Vec::new();
        for (text, shade) in parts {
            let style = Style {
                fg: crate::theme::parse_color(&format!("#{shade:02x}6464")).unwrap(),
                bg: crate::theme::parse_color("#0a141e").unwrap(),
                bold: *shade > 128,
                ..Style::default()
            };
            cells.extend(crate::compose::text_cells(text, style));
        }
        crate::compose::fit(cells, 40, Style::default())
    }

    fn shown(rows: &[&[Cell]], extra: &[u8]) -> Vec<Vec<Cell>> {
        let mut term = arcade_term::Term::new(arcade_term::TermConfig::new(40, 2)).unwrap();
        for cells in rows {
            let mut bytes = Vec::new();
            push_row(&mut bytes, 1, cells, &[]);
            term.feed(&bytes);
        }
        term.feed(extra);
        term.screen().unwrap()
    }

    /// The short update leaves the host showing exactly what the whole row would, also next to a
    /// wide character, and a row that changed all over is not sent in pieces.
    #[test]
    fn the_changed_cells_alone_bring_the_row_to_the_new_one() {
        let before = bar(&[(" a ", 200), (" \u{4e2d} ", 50), (" b ", 200), (" end", 50)]);
        for now in [
            bar(&[(" a ", 100), (" \u{4e2d} ", 50), (" b ", 200), (" end", 50)]),
            bar(&[(" a ", 200), (" \u{4e2d} ", 90), (" b ", 200), (" end", 50)]),
            bar(&[(" a ", 200), (" \u{4e2d} ", 50), (" b ", 200), (" END", 50)]),
        ] {
            let small = changed_cells(1, &before, &now).expect("a few cells differ");
            let mut whole = Vec::new();
            push_row(&mut whole, 1, &now, &[]);
            assert!(small.len() < whole.len());
            assert_eq!(
                shown(&[&before], &small),
                shown(&[&now], &[]),
                "{:?}",
                String::from_utf8_lossy(&small)
            );
        }
        // Nothing sent for a row that is the same; the whole row when much of it differs.
        assert_eq!(changed_cells(1, &before, &before), Some(Vec::new()));
        let all = bar(&[("zzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzz", 10)]);
        assert_eq!(changed_cells(1, &before, &all), None);
        assert_eq!(changed_cells(1, &before, &before[..10]), None);
    }
}
