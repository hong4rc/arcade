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
    if cursor.visible {
        out.extend_from_slice(
            format!("\x1b[{};{}H\x1b[?25h", cursor.y + 1, cursor.x + 1).as_bytes(),
        );
    }
}

/// The bytes that bring the host terminal up to date with `frame`.
///
/// The whole update is wrapped in synchronized output, so the host shows it all at once.
pub fn render(frame: &Frame) -> Vec<u8> {
    let mut out = Vec::new();
    out.extend_from_slice(b"\x1b[?2026h\x1b[?25l");
    for row in &frame.rows {
        push_row(&mut out, row.y, &row.cells);
    }
    out.extend_from_slice(b"\x1b[0m");
    push_cursor(&mut out, &frame.cursor);
    out.extend_from_slice(b"\x1b[?2026l");
    out
}
