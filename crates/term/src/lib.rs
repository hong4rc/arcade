//! Terminal emulation for one pane.
//!
//! This is the only crate in the workspace that calls libghostty-vt. Everything outside it sees
//! plain Rust types: bytes go in with [`Term::feed`], and what the pane looks like comes out as
//! [`Frame`] (rows that changed since the last frame), [`Term::row_text`] and [`Term::style_at`].
//!
//! Policy that the library leaves to its host is set here:
//!
//! - grapheme clustering (mode 2027) is on from the start, so joined emoji take two columns;
//! - scrollback is limited by lines only, not by bytes;
//! - clipboard reads are never answered, and clipboard writes above 1 MiB are dropped whole;
//! - a string sequence that never ends is cancelled after a timeout (see [`Term::expire_stuck`]);
//! - idle scrollback can be compressed in small steps (see [`Term::compress_step`]).

use std::cell::RefCell;
use std::fmt;
use std::rc::Rc;
use std::time::{Duration, Instant};

use libghostty_vt::mode::Mode as GMode;
use libghostty_vt::point::{Coordinate, Point};
use libghostty_vt::render::{CellIterator, CursorVisualStyle, Dirty, RenderState, RowIterator};
use libghostty_vt::screen::{CellContentTag, CellWide};
use libghostty_vt::style::{Style as GStyle, StyleColor, Underline as GUnderline};
use libghostty_vt::terminal::CompressionMode;
use libghostty_vt::{Terminal, key::KittyKeyFlags};

/// Largest clipboard write accepted from a pane, in bytes after decoding.
pub const CLIPBOARD_WRITE_LIMIT: usize = 1024 * 1024;

/// How long a string sequence may stay unfinished before it is cancelled.
pub const DEFAULT_STRING_TIMEOUT: Duration = Duration::from_secs(5);

/// Scrollback lines kept per pane unless configured otherwise.
pub const DEFAULT_SCROLLBACK_LINES: usize = 50_000;

#[derive(Debug)]
pub struct TermError(String);

impl fmt::Display for TermError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "terminal emulation error: {}", self.0)
    }
}

impl std::error::Error for TermError {}

fn err(context: &str, e: impl fmt::Debug) -> TermError {
    TermError(format!("{context}: {e:?}"))
}

#[derive(Clone, Debug)]
pub struct TermConfig {
    pub cols: u16,
    pub rows: u16,
    pub scrollback_lines: usize,
    pub string_timeout: Duration,
}

impl TermConfig {
    pub fn new(cols: u16, rows: u16) -> Self {
        TermConfig {
            cols,
            rows,
            scrollback_lines: DEFAULT_SCROLLBACK_LINES,
            string_timeout: DEFAULT_STRING_TIMEOUT,
        }
    }
}

/// Something the program in the pane asked the terminal to do or answer.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Event {
    Bell,
    Title(String),
    /// The program asked to write the clipboard; the bytes are the decoded text.
    ClipboardWrite(Vec<u8>),
    Notification {
        title: String,
        body: String,
    },
    /// Bytes the terminal sends back to the program (answers to its queries).
    Reply(Vec<u8>),
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
pub enum Color {
    #[default]
    Default,
    Indexed(u8),
    Rgb(u8, u8, u8),
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
pub enum Underline {
    #[default]
    None,
    Single,
    Double,
    Curly,
    Dotted,
    Dashed,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
pub struct Style {
    pub fg: Color,
    pub bg: Color,
    pub underline_color: Color,
    pub bold: bool,
    pub faint: bool,
    pub italic: bool,
    pub blink: bool,
    pub inverse: bool,
    pub invisible: bool,
    pub strike: bool,
    pub overline: bool,
    pub underline: Underline,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum CellWidth {
    Narrow,
    /// First column of a two column character.
    Wide,
    /// Second column of a two column character; draws nothing.
    SpacerTail,
}

/// One column of the screen.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Cell {
    /// The text drawn in this column; empty for a blank cell.
    pub text: String,
    pub width: CellWidth,
    pub style: Style,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct RowUpdate {
    pub y: u16,
    pub cells: Vec<Cell>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum CursorShape {
    Block,
    Underline,
    Bar,
    HollowBlock,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Cursor {
    pub x: u16,
    pub y: u16,
    /// False when the program hid the cursor or it is outside the visible screen.
    pub visible: bool,
    pub shape: CursorShape,
    pub blinking: bool,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Damage {
    /// Nothing on the screen changed (the cursor may still have moved).
    Clean,
    /// Only the rows in [`Frame::rows`] changed.
    Partial,
    /// Everything may have changed; [`Frame::rows`] holds every row.
    Full,
}

/// What changed on screen since the previous frame.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Frame {
    pub cols: u16,
    pub rows_total: u16,
    pub damage: Damage,
    pub rows: Vec<RowUpdate>,
    pub cursor: Cursor,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Mode {
    AltScreen,
    BracketedPaste,
    MouseTracking,
    MouseSgr,
    FocusReport,
    SyncOutput,
    KittyKeyboard,
    GraphemeCluster,
    /// Application cursor keys (DECCKM): the arrow keys send `ESC O` instead of `ESC [`.
    AppCursorKeys,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum CompressProgress {
    /// More work remains; call [`Term::compress_step`] again while the pane stays idle.
    Pending,
    /// Nothing left to do until the pane produces output again.
    Done,
}

/// Size in bytes of the unit the library allocates and prunes scrollback in ("about 400 KB").
const PAGE_BYTES: usize = 400 * 1024;
/// Bytes one screen column takes in a page, with room for the row bookkeeping.
const CELL_BYTES: usize = 8;

/// The line limit to hand to the library so that at least `wanted` lines are kept.
///
/// The library prunes whole pages, so with the plain limit it can keep up to a page less than
/// asked for (measured: 49,921 of 50,000 at 80 columns, 47,891 at 20). Adding the number of rows
/// one page holds makes `wanted` a floor; the surplus is at most about two pages of memory.
fn library_line_limit(wanted: usize, cols: u16) -> usize {
    let rows_per_page = PAGE_BYTES / (usize::from(cols.max(1)) * CELL_BYTES);
    wanted + rows_per_page + 1
}

type Events = Rc<RefCell<Vec<Event>>>;

pub struct Term {
    term: Terminal<'static, 'static>,
    events: Events,
    render: RenderState<'static>,
    rows_iter: RowIterator<'static>,
    cells_iter: CellIterator<'static>,
    cols: u16,
    rows: u16,
    scrollback_lines: usize,
    string_timeout: Duration,
    /// When set, the parser was left in the middle of a sequence at this time + timeout.
    stuck_deadline: Option<Instant>,
}

impl Term {
    pub fn new(config: TermConfig) -> Result<Term, TermError> {
        let events: Events = Rc::default();
        let mut term =
            Terminal::new(config.cols, config.rows).map_err(|e| err("create terminal", e))?;
        // The library limits scrollback by lines and by bytes, and its byte limit holds only a
        // few hundred lines. Lift it so that the line limit alone decides.
        term.set_scrollback_max_bytes(Some(usize::MAX))
            .map_err(|e| err("scrollback byte limit", e))?;
        term.set_scrollback_max_lines(Some(library_line_limit(
            config.scrollback_lines,
            config.cols,
        )))
        .map_err(|e| err("scrollback line limit", e))?;
        // On by default, and restored by a full reset. Without it a joined emoji takes eight
        // columns and every program that draws it ends up misaligned.
        term.set_default_mode(GMode::GRAPHEME_CLUSTER, true)
            .map_err(|e| err("grapheme clustering", e))?;
        term.set_terminfo_name("xterm-256color")
            .map_err(|e| err("terminfo name", e))?;

        let sink = events.clone();
        term.on_pty_write(move |_, data| sink.borrow_mut().push(Event::Reply(data.to_vec())))
            .map_err(|e| err("reply callback", e))?;
        let sink = events.clone();
        term.on_bell(move |_| sink.borrow_mut().push(Event::Bell))
            .map_err(|e| err("bell callback", e))?;
        let sink = events.clone();
        term.on_title_changed(move |term| {
            let title = String::from_utf8_lossy(term.title().unwrap_or_default()).into_owned();
            sink.borrow_mut().push(Event::Title(title));
        })
        .map_err(|e| err("title callback", e))?;
        let sink = events.clone();
        term.on_clipboard_write(move |_, write| {
            let size: usize = write.contents().map(|c| c.data.len()).sum();
            if size <= CLIPBOARD_WRITE_LIMIT {
                let data: Vec<u8> = write.contents().flat_map(|c| c.data.to_vec()).collect();
                sink.borrow_mut().push(Event::ClipboardWrite(data));
            }
            // Over the limit the write is dropped whole, never cut short.
            write.reply(Ok(()), false);
        })
        .map_err(|e| err("clipboard callback", e))?;
        // No clipboard read callback is installed: the library then ignores read requests.
        let sink = events.clone();
        term.on_desktop_notification(move |_, notification| {
            sink.borrow_mut().push(Event::Notification {
                title: String::from_utf8_lossy(notification.title()).into_owned(),
                body: String::from_utf8_lossy(notification.body()).into_owned(),
            });
        })
        .map_err(|e| err("notification callback", e))?;

        Ok(Term {
            term,
            events,
            render: RenderState::new().map_err(|e| err("render state", e))?,
            rows_iter: RowIterator::new().map_err(|e| err("row iterator", e))?,
            cells_iter: CellIterator::new().map_err(|e| err("cell iterator", e))?,
            cols: config.cols,
            rows: config.rows,
            scrollback_lines: config.scrollback_lines,
            string_timeout: config.string_timeout,
            stuck_deadline: None,
        })
    }

    pub fn cols(&self) -> u16 {
        self.cols
    }

    pub fn rows(&self) -> u16 {
        self.rows
    }

    /// Give the terminal bytes the program wrote.
    pub fn feed(&mut self, bytes: &[u8]) {
        self.term.vt_write(bytes);
        // Not at ground means the bytes ended inside a sequence (or inside a multi-byte
        // character). Remember when, so a sequence that never ends can be cancelled.
        if self.term.is_vt_ground().unwrap_or(true) {
            self.stuck_deadline = None;
        } else {
            self.stuck_deadline = Some(Instant::now() + self.string_timeout);
        }
    }

    /// When the sequence the parser is stuck in will be cancelled, if it is stuck in one.
    pub fn stuck_deadline(&self) -> Option<Instant> {
        self.stuck_deadline
    }

    /// Cancel the unfinished sequence if its deadline has passed. Returns whether it did.
    ///
    /// Cancelling sends the CAN control byte (0x18), which every sequence parser treats as "drop
    /// what you were reading"; nothing of the sequence reaches the screen.
    pub fn expire_stuck(&mut self, now: Instant) -> bool {
        match self.stuck_deadline {
            Some(deadline) if now >= deadline => {
                self.term.vt_write(&[0x18]);
                self.stuck_deadline = None;
                true
            }
            _ => false,
        }
    }

    pub fn resize(&mut self, cols: u16, rows: u16) -> Result<(), TermError> {
        self.term
            .resize(cols, rows, 0, 0)
            .map_err(|e| err("resize", e))?;
        self.cols = cols;
        self.rows = rows;
        // How many rows a page holds depends on the width, so the allowance changes with it.
        self.term
            .set_scrollback_max_lines(Some(library_line_limit(self.scrollback_lines, cols)))
            .map_err(|e| err("scrollback line limit", e))?;
        Ok(())
    }

    pub fn drain_events(&mut self) -> Vec<Event> {
        std::mem::take(&mut *self.events.borrow_mut())
    }

    /// Whether any event is waiting; cheaper than draining.
    pub fn has_events(&self) -> bool {
        !self.events.borrow().is_empty()
    }

    /// Text of one visible row, with trailing blanks removed.
    pub fn row_text(&self, row: u16) -> String {
        let mut text = String::new();
        let mut buf = ['\0'; 32];
        for col in 0..self.cols {
            let point = Point::Active(Coordinate {
                x: col,
                y: u32::from(row),
            });
            let Ok(grid_ref) = self.term.grid_ref(point) else {
                continue;
            };
            let wide = grid_ref
                .cell()
                .and_then(|cell| cell.wide())
                .unwrap_or(CellWide::Narrow);
            if matches!(wide, CellWide::SpacerTail | CellWide::SpacerHead) {
                continue;
            }
            match grid_ref.graphemes(&mut buf) {
                Ok(0) | Err(_) => text.push(' '),
                Ok(n) => text.extend(buf[..n].iter()),
            }
        }
        text.trim_end().to_string()
    }

    /// Cursor position as (column, row), both starting at zero.
    pub fn cursor_position(&self) -> (u16, u16) {
        (
            self.term.cursor_x().unwrap_or(0),
            self.term.cursor_y().unwrap_or(0),
        )
    }

    /// Style of one visible cell, as the program set it.
    pub fn style_at(&self, row: u16, col: u16) -> Style {
        let point = Point::Active(Coordinate {
            x: col,
            y: u32::from(row),
        });
        self.term
            .grid_ref(point)
            .ok()
            .and_then(|grid_ref| grid_ref.style().ok())
            .map(convert_style)
            .unwrap_or_default()
    }

    /// The link (OSC 8) a visible cell belongs to, if any.
    pub fn hyperlink_at(&self, row: u16, col: u16) -> Option<String> {
        let point = Point::Active(Coordinate {
            x: col,
            y: u32::from(row),
        });
        let grid_ref = self.term.grid_ref(point).ok()?;
        let mut uri = vec![0u8; 4096];
        let len = grid_ref.hyperlink_uri(&mut uri).unwrap_or(0);
        if len == 0 {
            None
        } else {
            Some(String::from_utf8_lossy(&uri[..len]).into_owned())
        }
    }

    pub fn mode(&self, mode: Mode) -> Option<bool> {
        let on = |m: GMode| self.term.mode(m).ok();
        match mode {
            Mode::AltScreen => on(GMode::ALT_SCREEN_SAVE),
            Mode::BracketedPaste => on(GMode::BRACKETED_PASTE),
            Mode::MouseTracking => self.term.is_mouse_tracking().ok(),
            Mode::MouseSgr => on(GMode::SGR_MOUSE),
            Mode::FocusReport => on(GMode::FOCUS_EVENT),
            Mode::SyncOutput => on(GMode::SYNC_OUTPUT),
            Mode::GraphemeCluster => on(GMode::GRAPHEME_CLUSTER),
            Mode::AppCursorKeys => on(GMode::DECCKM),
            Mode::KittyKeyboard => self
                .term
                .kitty_keyboard_flags()
                .ok()
                .map(|flags| flags != KittyKeyFlags::DISABLED),
        }
    }

    /// Number of lines currently kept above the visible screen.
    pub fn scrollback_lines(&self) -> usize {
        self.term.scrollback_rows().unwrap_or(0)
    }

    /// Bytes of memory the screen and scrollback hold right now.
    pub fn resident_bytes(&self) -> usize {
        self.term
            .memory_usage()
            .map(|usage| (usage.primary_resident_bytes + usage.alternate_resident_bytes) as usize)
            .unwrap_or(0)
    }

    /// One bounded step of shrinking idle scrollback. Call it while the pane is quiet.
    pub fn compress_step(&mut self) -> CompressProgress {
        use libghostty_vt::terminal::CompressionResult as R;
        match self.term.compress(CompressionMode::Incremental) {
            Ok(R::Pending) => CompressProgress::Pending,
            Ok(R::Complete | R::Unsupported) | Err(_) => CompressProgress::Done,
        }
    }

    /// What changed on screen since the last call, and clear that record.
    ///
    /// With `force_full` every row is returned whatever changed (a new client, a resize).
    pub fn snapshot(&mut self, force_full: bool) -> Result<Frame, TermError> {
        let snapshot = self
            .render
            .update(&self.term)
            .map_err(|e| err("render update", e))?;
        let dirty = snapshot.dirty().map_err(|e| err("dirty state", e))?;
        let cols = snapshot.cols().map_err(|e| err("columns", e))?;
        let rows_total = snapshot.rows().map_err(|e| err("rows", e))?;
        let full = force_full || dirty == Dirty::Full;
        let cursor = {
            let c = snapshot.cursor().map_err(|e| err("cursor", e))?;
            let (x, y, in_view) = match c.viewport {
                Some(v) => (v.x, v.y, true),
                None => (0, 0, false),
            };
            Cursor {
                x,
                y,
                visible: c.visible && in_view,
                shape: match c.visual_style {
                    CursorVisualStyle::Bar => CursorShape::Bar,
                    CursorVisualStyle::Underline => CursorShape::Underline,
                    CursorVisualStyle::BlockHollow => CursorShape::HollowBlock,
                    _ => CursorShape::Block,
                },
                blinking: c.blinking,
            }
        };

        let mut updates = Vec::new();
        if full || dirty != Dirty::Clean {
            let mut row_iter = self
                .rows_iter
                .update(&snapshot)
                .map_err(|e| err("row iterator", e))?;
            let mut y: u16 = 0;
            while let Some(row) = row_iter.next() {
                let changed = row.dirty().map_err(|e| err("row dirty", e))?;
                if full || changed {
                    let mut cells = Vec::with_capacity(usize::from(cols));
                    let mut cell_iter = self
                        .cells_iter
                        .update(row)
                        .map_err(|e| err("cell iterator", e))?;
                    while let Some(cell) = cell_iter.next() {
                        let raw = cell.raw_cell().map_err(|e| err("raw cell", e))?;
                        let width = match raw.wide().map_err(|e| err("cell width", e))? {
                            CellWide::Wide => CellWidth::Wide,
                            CellWide::SpacerTail => CellWidth::SpacerTail,
                            CellWide::Narrow | CellWide::SpacerHead => CellWidth::Narrow,
                        };
                        let mut text = String::new();
                        if raw.has_text().unwrap_or(false) {
                            cell.graphemes_utf8(&mut text)
                                .map_err(|e| err("cell text", e))?;
                        }
                        let mut style = if raw.has_styling().unwrap_or(false) {
                            convert_style(cell.style().map_err(|e| err("cell style", e))?)
                        } else {
                            Style::default()
                        };
                        // A cell cleared with a background color holds no text; the color
                        // lives in the cell itself rather than in a style.
                        match raw.content_tag() {
                            Ok(CellContentTag::BgColorPalette) => {
                                if let Ok(index) = raw.bg_color_palette() {
                                    style.bg = Color::Indexed(index.0);
                                }
                            }
                            Ok(CellContentTag::BgColorRgb) => {
                                if let Ok(rgb) = raw.bg_color_rgb() {
                                    style.bg = Color::Rgb(rgb.r, rgb.g, rgb.b);
                                }
                            }
                            _ => {}
                        }
                        cells.push(Cell { text, width, style });
                    }
                    updates.push(RowUpdate { y, cells });
                    if changed {
                        row.set_dirty(false).map_err(|e| err("clear row", e))?;
                    }
                }
                y += 1;
            }
        }
        snapshot.clean().map_err(|e| err("clear dirty state", e))?;

        let damage = if full {
            Damage::Full
        } else if dirty == Dirty::Clean {
            Damage::Clean
        } else {
            Damage::Partial
        };
        Ok(Frame {
            cols,
            rows_total,
            damage,
            rows: updates,
            cursor,
        })
    }

    /// Every visible cell, row by row. Uses (and clears) the same change record as
    /// [`Term::snapshot`], so a pane should be read through one of the two paths.
    pub fn screen(&mut self) -> Result<Vec<Vec<Cell>>, TermError> {
        Ok(self
            .snapshot(true)?
            .rows
            .into_iter()
            .map(|row| row.cells)
            .collect())
    }
}

fn convert_color(color: StyleColor) -> Color {
    match color {
        StyleColor::None => Color::Default,
        StyleColor::Palette(index) => Color::Indexed(index.0),
        StyleColor::Rgb(rgb) => Color::Rgb(rgb.r, rgb.g, rgb.b),
    }
}

fn convert_style(style: GStyle) -> Style {
    Style {
        fg: convert_color(style.fg_color),
        bg: convert_color(style.bg_color),
        underline_color: convert_color(style.underline_color),
        bold: style.bold,
        faint: style.faint,
        italic: style.italic,
        blink: style.blink,
        inverse: style.inverse,
        invisible: style.invisible,
        strike: style.strikethrough,
        overline: style.overline,
        underline: match style.underline {
            GUnderline::None => Underline::None,
            GUnderline::Single => Underline::Single,
            GUnderline::Double => Underline::Double,
            GUnderline::Curly => Underline::Curly,
            GUnderline::Dotted => Underline::Dotted,
            GUnderline::Dashed => Underline::Dashed,
            _ => Underline::Single,
        },
    }
}
