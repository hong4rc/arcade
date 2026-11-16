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

use libghostty_vt::grid_ref::TrackedGridRef;
use libghostty_vt::mode::Mode as GMode;
use libghostty_vt::point::{Coordinate, Point, Space};
use libghostty_vt::render::{CellIterator, CursorVisualStyle, Dirty, RenderState, RowIterator};
use libghostty_vt::screen::{CellContentTag, CellSemanticContent, CellWide, RowSemanticPrompt};
use libghostty_vt::style::{Style as GStyle, StyleColor, Underline as GUnderline};
use libghostty_vt::terminal::CompressionMode;
use libghostty_vt::{Terminal, key::KittyKeyFlags};

mod keys;
mod progress;
pub use keys::{Code, Key, Mods, Named, PaneKeyModes, encode as encode_key};
pub use progress::Progress;

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
    /// `CSI 22 t`: the program asks to keep the title it has now (the library does not report this).
    TitlePush,
    /// `CSI 23 t`: the program asks to get back the title it kept last.
    TitlePop,
    /// The program asked to write the clipboard; the bytes are the decoded text.
    ClipboardWrite(Vec<u8>),
    Notification {
        title: String,
        body: String,
    },
    /// The program reported progress (`OSC 9;4`); state 0 takes the report away.
    Progress(Progress),
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
    /// The runs of columns that belong to a link (OSC 8), left to right. Empty for nearly every
    /// row. Addresses the host terminal could not take safely (control bytes, not ASCII, too
    /// long) are left out.
    pub links: Vec<LinkSpan>,
}

/// Columns `from..to` of a row that all belong to the link `uri`.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct LinkSpan {
    pub from: u16,
    pub to: u16,
    pub uri: String,
}

/// The longest link address that is passed on to the host terminal.
pub const LINK_MAX_LEN: usize = 2048;

fn link_at(term: &Terminal<'_, '_>, point: Point) -> Option<String> {
    let uri = raw_link_at(term, point)?;
    link_is_safe(&uri).then_some(uri)
}

/// The address of the link at `point` as the program wrote it, if it is text without control
/// characters and not longer than 4096 bytes.
fn raw_link_at(term: &Terminal<'_, '_>, point: Point) -> Option<String> {
    let grid_ref = term.grid_ref(point).ok()?;
    let mut uri = vec![0u8; 4097];
    let len = grid_ref.hyperlink_uri(&mut uri).ok()?;
    let uri = std::str::from_utf8(&uri[..len]).ok()?;
    (!uri.is_empty() && uri.len() <= 4096 && !uri.chars().any(char::is_control))
        .then(|| uri.to_string())
}

/// Whether an address can be written into an OSC 8 sequence as it is: printable ASCII only, not
/// empty, not longer than [`LINK_MAX_LEN`].
pub fn link_is_safe(uri: &str) -> bool {
    !uri.is_empty() && uri.len() <= LINK_MAX_LEN && uri.bytes().all(|b| (0x21..0x7f).contains(&b))
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

/// One row of the whole screen, scrollback included.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Line {
    pub cells: Vec<Cell>,
    /// The row goes on from the row above it: the terminal broke one long line over both.
    pub continues: bool,
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

/// How much of the mouse a program asked to be told about (DEC modes 9 and 1000 to 1003).
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub enum MouseLevel {
    /// The program did not ask: the mouse is not its business.
    Off,
    /// Presses, releases and the wheel (modes 9 and 1000).
    Press,
    /// Also moves while a button is held (mode 1002).
    Drag,
    /// Also every move (mode 1003).
    Any,
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
    /// A row the library keeps track of as lines are cut off the top of a full scrollback, so
    /// that a reader of the rows can tell how far they moved.
    anchor: Option<TrackedGridRef>,
    /// Where the bytes so far stand in a `CSI 22 t` / `CSI 23 t`, which the library ignores.
    title_scan: TitleScan,
    progress_scan: progress::ProgressScan,
    /// Where a title push or pop could start: `ESC [ 2`, with a 2 or a 3 after it.
    start: memchr::memmem::Finder<'static>,
}

/// Looks for `CSI 22 t` and `CSI 23 t` (also with a second parameter of 0 or 2, the ones that
/// say "window title"; 1 is the icon and changes nothing) in the bytes a program wrote. The
/// state is kept between calls, because a sequence may arrive in two pieces. It allocates
/// nothing: it sits on the path of every byte a pane prints.
#[derive(Clone, Copy, Debug, Default)]
enum TitleScan {
    #[default]
    Ground,
    Esc,
    /// Inside `ESC [`, with the parameter bytes so far (at most eight; more is not ours).
    Csi {
        params: [u8; 8],
        len: u8,
    },
}

/// Whether bytes that start with `ESC [ 2` are, or may become once more bytes come, one of the
/// title pushes and pops `22`, `22;0`, `22;2`, `23`, `23;0`, `23;2` followed by `t`. Most others
/// (`ESC[22;3H`, `ESC[2J`) are told apart here without the state machine.
fn could_be_title_push_or_pop(bytes: &[u8]) -> bool {
    match bytes {
        [_, _, _] => true,
        [_, _, _, b'2' | b'3', rest @ ..] => matches!(
            rest,
            [] | [b't', ..] | [b';'] | [b';', b'0' | b'2'] | [b';', b'0' | b'2', b't', ..]
        ),
        _ => false,
    }
}

/// Where the end of `bytes` is the beginning of a title push or pop that the next read may
/// finish: the position of a trailing `ESC`, `ESC [` or `ESC [ 2`.
fn split_start(bytes: &[u8]) -> Option<usize> {
    const START: &[u8] = b"\x1b[2";
    (1..=START.len().min(bytes.len()))
        .rev()
        .find(|n| bytes[bytes.len() - n..] == START[..*n])
        .map(|n| bytes.len() - n)
}

impl TitleScan {
    /// Take one byte; `Some(true)` when it ended a push, `Some(false)` when it ended a pop.
    #[inline]
    fn byte(&mut self, b: u8) -> Option<bool> {
        match self {
            TitleScan::Ground => {
                if b == 0x1b {
                    *self = TitleScan::Esc;
                }
            }
            TitleScan::Esc => {
                *self = match b {
                    b'[' => TitleScan::Csi {
                        params: [0; 8],
                        len: 0,
                    },
                    0x1b => TitleScan::Esc,
                    _ => TitleScan::Ground,
                };
            }
            TitleScan::Csi { params, len } => match b {
                b'0'..=b'9' | b';' if usize::from(*len) < params.len() => {
                    params[usize::from(*len)] = b;
                    *len += 1;
                }
                b't' => {
                    let found = match &params[..usize::from(*len)] {
                        b"22" | b"22;0" | b"22;2" => Some(true),
                        b"23" | b"23;0" | b"23;2" => Some(false),
                        _ => None,
                    };
                    *self = TitleScan::Ground;
                    return found;
                }
                0x1b => *self = TitleScan::Esc,
                _ => *self = TitleScan::Ground,
            },
        }
        None
    }
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
                let mut data: Vec<u8> = Vec::with_capacity(size);
                for part in write.contents() {
                    data.extend_from_slice(part.data);
                }
                // Only the latest write matters; an earlier one still waiting is replaced.
                let mut events = sink.borrow_mut();
                events.retain(|e| !matches!(e, Event::ClipboardWrite(_)));
                events.push(Event::ClipboardWrite(data));
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
            anchor: None,
            title_scan: TitleScan::default(),
            progress_scan: progress::ProgressScan::new(),
            start: memchr::memmem::Finder::new(b"\x1b[2").into_owned(),
        })
    }

    /// Give back what reading the screen out holds on to (a copy of every row it last read).
    /// The next `snapshot` builds it again, and it must then be asked for in full.
    pub fn release_snapshot_state(&mut self) {
        if let (Ok(render), Ok(rows_iter), Ok(cells_iter)) =
            (RenderState::new(), RowIterator::new(), CellIterator::new())
        {
            self.render = render;
            self.rows_iter = rows_iter;
            self.cells_iter = cells_iter;
        }
    }

    pub fn cols(&self) -> u16 {
        self.cols
    }

    pub fn rows(&self) -> u16 {
        self.rows
    }

    /// Give the terminal bytes the program wrote.
    pub fn feed(&mut self, bytes: &[u8]) {
        let mut reports = Vec::new();
        self.progress_scan.scan(bytes, &mut reports);
        if !reports.is_empty() {
            let mut events = self.events.borrow_mut();
            events.extend(reports.into_iter().map(Event::Progress));
        }
        // The bytes go to the library in pieces cut after each title push or pop, so that the
        // events come in the order the program wrote them.
        let mut start = 0;
        let mut at = 0;
        while at < bytes.len() {
            if matches!(self.title_scan, TitleScan::Ground) {
                // The usual case, a flood of output: look for where a title push or pop could
                // start (`ESC [ 2 2` or `ESC [ 2 3`, found at the speed of a substring search
                // since colours and cursor moves almost never start so), or where this read
                // ends inside what could become one. Only those go through the state machine.
                let rest = &bytes[at..];
                let mut from = 0;
                let mut found = split_start(rest);
                while let Some(n) = self.start.find(&rest[from..]) {
                    let n = from + n;
                    if found.is_some_and(|f| f < n) {
                        break;
                    }
                    if could_be_title_push_or_pop(&rest[n..]) {
                        found = Some(n);
                        break;
                    }
                    from = n + 1;
                }
                match found {
                    Some(n) => at += n,
                    None => break,
                }
            }
            if let Some(push) = self.title_scan.byte(bytes[at]) {
                self.term.vt_write(&bytes[start..=at]);
                start = at + 1;
                let event = if push {
                    Event::TitlePush
                } else {
                    Event::TitlePop
                };
                self.events.borrow_mut().push(event);
            }
            at += 1;
        }
        self.term.vt_write(&bytes[start..]);
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

    /// Start following the last row of the whole screen. The library moves the followed row up as
    /// old lines are cut off the top of a full scrollback, whatever those lines say, so
    /// [`Term::anchor_row`] tells exactly how many were cut.
    pub fn anchor_last_row(&mut self) {
        let last = u32::try_from(self.total_rows().saturating_sub(1)).unwrap_or(u32::MAX);
        let point = Point::Screen(Coordinate { x: 0, y: last });
        let placed = match self.anchor.as_mut() {
            Some(anchor) => anchor.set(&mut self.term, point).is_ok(),
            None => match self.term.track_grid_ref(point) {
                Ok(anchor) => {
                    self.anchor = Some(anchor);
                    true
                }
                Err(_) => false,
            },
        };
        if !placed {
            self.anchor = None;
        }
    }

    /// The row, counted from the oldest scrollback line, of the row [`Term::anchor_last_row`]
    /// chose; `None` when nothing is followed or the row is gone (cut off, or the screen was
    /// reset).
    pub fn anchor_row(&self) -> Option<usize> {
        let point = self.anchor.as_ref()?.point(Space::Screen).ok()??;
        usize::try_from(point.y).ok()
    }

    /// Rows of the whole screen: the scrollback and the visible rows.
    pub fn total_rows(&self) -> usize {
        self.scrollback_lines() + usize::from(self.rows)
    }

    /// One row of the whole screen, counted from the oldest scrollback line (row 0). `None`
    /// past the last row.
    ///
    /// Reads the terminal directly and leaves the change record alone, so it can be used while
    /// the pane's screen is being shown from somewhere else (copy mode).
    pub fn line(&self, y: usize) -> Option<Line> {
        if y >= self.total_rows() {
            return None;
        }
        let y32 = u32::try_from(y).ok()?;
        let first = self
            .term
            .grid_ref(Point::Screen(Coordinate { x: 0, y: y32 }))
            .ok()?;
        let continues = first
            .row()
            .and_then(|row| row.is_wrap_continuation())
            .unwrap_or(false);
        let mut cells = Vec::with_capacity(usize::from(self.cols));
        let mut buf = ['\0'; 32];
        for col in 0..self.cols {
            let Ok(grid_ref) = self
                .term
                .grid_ref(Point::Screen(Coordinate { x: col, y: y32 }))
            else {
                cells.push(Cell {
                    text: String::new(),
                    width: CellWidth::Narrow,
                    style: Style::default(),
                });
                continue;
            };
            let raw = grid_ref.cell().ok();
            let width = match raw.as_ref().and_then(|c| c.wide().ok()) {
                Some(CellWide::Wide) => CellWidth::Wide,
                Some(CellWide::SpacerTail) => CellWidth::SpacerTail,
                _ => CellWidth::Narrow,
            };
            let text = match grid_ref.graphemes(&mut buf) {
                Ok(n) if n > 0 && width != CellWidth::SpacerTail => buf[..n].iter().collect(),
                _ => String::new(),
            };
            let mut style = if raw
                .as_ref()
                .is_some_and(|c| c.has_styling().unwrap_or(false))
            {
                grid_ref.style().map(convert_style).unwrap_or_default()
            } else {
                Style::default()
            };
            match raw.as_ref().and_then(|c| c.content_tag().ok()) {
                Some(CellContentTag::BgColorPalette) => {
                    if let Some(index) = raw.as_ref().and_then(|c| c.bg_color_palette().ok()) {
                        style.bg = Color::Indexed(index.0);
                    }
                }
                Some(CellContentTag::BgColorRgb) => {
                    if let Some(rgb) = raw.as_ref().and_then(|c| c.bg_color_rgb().ok()) {
                        style.bg = Color::Rgb(rgb.r, rgb.g, rgb.b);
                    }
                }
                _ => {}
            }
            cells.push(Cell { text, width, style });
        }
        Some(Line { cells, continues })
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
        self.link_at(Point::Active(Coordinate {
            x: col,
            y: u32::from(row),
        }))
    }

    /// Whether a row of the whole screen (row 0 is the oldest line of the scrollback) is where a
    /// shell prompt began (an OSC 133 `A` mark). Read from the library's own record, only when
    /// asked; nothing is kept by Arcade while output goes by.
    pub fn row_is_prompt(&self, row: usize) -> bool {
        let Ok(y) = u32::try_from(row) else {
            return false;
        };
        self.term
            .grid_ref(Point::Screen(Coordinate { x: 0, y }))
            .ok()
            .and_then(|g| g.row().ok())
            .and_then(|r| r.semantic_prompt().ok())
            .is_some_and(|p| p == RowSemanticPrompt::Prompt)
    }

    /// Whether a row holds text of a shell prompt or of what was typed at it (OSC 133 `A`, `B`
    /// and `C` marks tell the library which cell is which).
    pub fn row_is_prompt_or_input(&self, row: usize) -> bool {
        let Ok(y) = u32::try_from(row) else {
            return false;
        };
        for x in 0..self.cols {
            let Ok(g) = self.term.grid_ref(Point::Screen(Coordinate { x, y })) else {
                return false;
            };
            let Ok(cell) = g.cell() else {
                return false;
            };
            if cell.has_text().unwrap_or(false)
                && matches!(
                    cell.semantic_content(),
                    Ok(CellSemanticContent::Prompt | CellSemanticContent::Input)
                )
            {
                return true;
            }
        }
        false
    }

    /// The link of a cell of the whole screen (row 0 is the oldest line of the scrollback), as
    /// the program wrote it. This is for opening it, as one argument; what is drawn on the host
    /// terminal goes through [`link_is_safe`] instead.
    pub fn link_at_screen(&self, row: usize, col: u16) -> Option<String> {
        let y = u32::try_from(row).ok()?;
        raw_link_at(&self.term, Point::Screen(Coordinate { x: col, y }))
    }

    /// The address of the link at `point`, if any, and if it is safe to pass on to the host.
    fn link_at(&self, point: Point) -> Option<String> {
        link_at(&self.term, point)
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

    /// How much of the mouse the program asked to be told about.
    pub fn mouse_level(&self) -> MouseLevel {
        let on = |m: GMode| self.term.mode(m).unwrap_or(false);
        if on(GMode::ANY_MOUSE) {
            MouseLevel::Any
        } else if on(GMode::BUTTON_MOUSE) {
            MouseLevel::Drag
        } else if on(GMode::NORMAL_MOUSE) || on(GMode::X10_MOUSE) {
            MouseLevel::Press
        } else {
            MouseLevel::Off
        }
    }

    /// What the program in this terminal asked for about its keys.
    pub fn key_modes(&self) -> PaneKeyModes {
        PaneKeyModes {
            app_cursor_keys: self.mode(Mode::AppCursorKeys) == Some(true),
            kitty_flags: self
                .term
                .kitty_keyboard_flags()
                .map(|flags| flags.bits())
                .unwrap_or(0),
        }
    }

    /// Whether the alternate screen is in use (shorthand for the mode).
    pub fn mode_alt_screen(&self) -> Option<bool> {
        self.mode(Mode::AltScreen)
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
                    // Only a row the library says may hold links is looked at cell by cell.
                    let row_links = row
                        .raw_row()
                        .and_then(|r| r.has_hyperlink())
                        .unwrap_or(false);
                    let mut links: Vec<LinkSpan> = Vec::new();
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
                        if row_links && raw.has_hyperlink().unwrap_or(false) {
                            let x = u16::try_from(cells.len()).unwrap_or(u16::MAX);
                            if let Some(uri) = link_at(
                                &self.term,
                                Point::Viewport(Coordinate { x, y: u32::from(y) }),
                            ) {
                                match links.last_mut() {
                                    Some(last) if last.to == x && last.uri == uri => {
                                        last.to = x + 1
                                    }
                                    _ => links.push(LinkSpan {
                                        from: x,
                                        to: x + 1,
                                        uri,
                                    }),
                                }
                            }
                        }
                        cells.push(Cell { text, width, style });
                    }
                    updates.push(RowUpdate { y, cells, links });
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
