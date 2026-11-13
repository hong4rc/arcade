//! Turn what changed on a pane's screen into the bytes that draw it on the host terminal.
//!
//! The colors are the ones the program asked for: palette numbers stay palette numbers, so the
//! host terminal's own theme applies. Nothing here picks a color.

use arcade_term::{Cell, CellWidth, Color, Cursor, CursorShape, Frame, Style, Underline};

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

fn is_plain_blank(cell: &Cell) -> bool {
    cell.text.is_empty() && cell.style == Style::default()
}

/// Draw one row: move to it, then write every column (or erase the plain tail).
fn push_row(out: &mut Vec<u8>, y: u16, cells: &[Cell]) {
    out.extend_from_slice(format!("\x1b[{};1H", y + 1).as_bytes());
    // Leave out a tail of plain blank cells and erase it instead.
    let mut end = cells.len();
    while end > 0 && is_plain_blank(&cells[end - 1]) {
        end -= 1;
    }
    let mut current = Style::default();
    out.extend_from_slice(b"\x1b[0m");
    for cell in &cells[..end] {
        if cell.width == CellWidth::SpacerTail {
            continue;
        }
        if cell.style != current {
            push_style(out, &cell.style);
            current = cell.style;
        }
        if cell.text.is_empty() {
            out.push(b' ');
        } else {
            out.extend_from_slice(cell.text.as_bytes());
        }
    }
    if end < cells.len() {
        if current != Style::default() {
            out.extend_from_slice(b"\x1b[0m");
        }
        out.extend_from_slice(b"\x1b[K");
    }
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
}

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
    cursor: Option<SentCursor>,
    modes: Option<HostModes>,
}

impl Renderer {
    pub fn new() -> Self {
        Self::default()
    }

    /// Forget what the host shows: the next update sends everything. Used when a client
    /// attaches and when the screen changes size.
    pub fn reset(&mut self) {
        *self = Self::default();
    }

    /// The bytes that bring the host terminal up to date with `frame`; empty when it already is.
    ///
    /// Drawing rows is wrapped in synchronized output, so the host shows the update at once.
    pub fn render(&mut self, frame: &Frame, modes: HostModes) -> Vec<u8> {
        if self.rows.len() != usize::from(frame.rows_total) {
            self.rows = vec![None; usize::from(frame.rows_total)];
        }
        let mut body = Vec::new();
        for row in &frame.rows {
            let mut bytes = Vec::new();
            push_row(&mut bytes, row.y, &row.cells);
            let slot = &mut self.rows[usize::from(row.y)];
            if slot.as_deref() != Some(bytes.as_slice()) {
                body.extend_from_slice(&bytes);
                *slot = Some(bytes);
            }
        }
        let cursor = SentCursor {
            x: frame.cursor.x,
            y: frame.cursor.y,
            visible: frame.cursor.visible,
            shape: frame.cursor.shape,
            blinking: frame.cursor.blinking,
        };
        let cursor_changed = self.cursor != Some(cursor);
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
            push_cursor(&mut out, &frame.cursor);
        }
        if modes_changed {
            let previous = self.modes.unwrap_or_default();
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
        self.cursor = Some(cursor);
        self.modes = Some(modes);
        out
    }
}
