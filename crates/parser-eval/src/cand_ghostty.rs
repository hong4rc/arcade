//! Candidate: libghostty-vt, the terminal core of Ghostty, through its C interface.

use std::cell::RefCell;
use std::rc::Rc;

use libghostty_vt::Terminal;
use libghostty_vt::key::KittyKeyFlags;
use libghostty_vt::mode::Mode as GMode;
use libghostty_vt::point::{Coordinate, Point};
use libghostty_vt::screen::CellWide;
use libghostty_vt::style::{StyleColor, Underline as GUnderline};

use crate::candidate::{Candidate, CellStyle, Color, Event, Mode, Underline};

type Events = Rc<RefCell<Vec<Event>>>;

pub struct Ghostty {
    term: Terminal<'static, 'static>,
    events: Events,
    cols: u16,
}

impl Ghostty {
    pub fn new(cols: u16, rows: u16, scrollback: usize) -> Self {
        let events: Events = Rc::default();
        let mut term = Terminal::new(cols, rows).expect("create terminal");
        // The library limits scrollback both by lines and by bytes, and the default byte limit is
        // about one page (a few hundred lines). Lift it so that only the line limit applies.
        term.set_scrollback_max_bytes(Some(usize::MAX))
            .expect("scrollback byte limit");
        term.set_scrollback_max_lines(Some(scrollback))
            .expect("scrollback line limit");

        let sink = events.clone();
        term.on_pty_write(move |_, data| sink.borrow_mut().push(Event::Reply(data.to_vec())))
            .expect("pty write callback");
        let sink = events.clone();
        term.on_bell(move |_| sink.borrow_mut().push(Event::Bell))
            .expect("bell callback");
        let sink = events.clone();
        term.on_title_changed(move |term| {
            let title = String::from_utf8_lossy(term.title().unwrap_or_default()).into_owned();
            sink.borrow_mut().push(Event::Title(title));
        })
        .expect("title callback");
        let sink = events.clone();
        term.on_clipboard_write(move |_, write| {
            let data: Vec<u8> = write.contents().flat_map(|c| c.data.to_vec()).collect();
            sink.borrow_mut().push(Event::ClipboardWrite(data));
            write.reply(Ok(()), false);
        })
        .expect("clipboard write callback");
        // No clipboard read callback is installed: the library then ignores read requests,
        // which is the default the spec asks for.
        let sink = events.clone();
        term.on_desktop_notification(move |_, notification| {
            sink.borrow_mut().push(Event::Notification {
                title: String::from_utf8_lossy(notification.title()).into_owned(),
                body: String::from_utf8_lossy(notification.body()).into_owned(),
            });
        })
        .expect("notification callback");

        Ghostty { term, events, cols }
    }
}

impl Candidate for Ghostty {
    fn feed(&mut self, bytes: &[u8]) {
        self.term.vt_write(bytes);
    }

    fn row_text(&self, row: u16) -> String {
        let mut text = String::new();
        let mut buf = ['\0'; 32];
        for col in 0..self.cols {
            let point = Point::Active(Coordinate {
                x: col,
                y: row as u32,
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

    fn cursor(&self) -> (u16, u16) {
        (
            self.term.cursor_x().unwrap_or(0),
            self.term.cursor_y().unwrap_or(0),
        )
    }

    fn drain_events(&mut self) -> Vec<Event> {
        std::mem::take(&mut *self.events.borrow_mut())
    }

    fn style_at(&self, row: u16, col: u16) -> CellStyle {
        let point = Point::Active(Coordinate {
            x: col,
            y: row as u32,
        });
        let grid_ref = self.term.grid_ref(point).expect("grid ref");
        let style = grid_ref.style().expect("style");
        let mut uri = vec![0u8; 4096];
        let len = grid_ref.hyperlink_uri(&mut uri).unwrap_or(0);
        CellStyle {
            bold: style.bold,
            italic: style.italic,
            underline: match style.underline {
                GUnderline::None => Underline::None,
                GUnderline::Single => Underline::Single,
                GUnderline::Double => Underline::Double,
                GUnderline::Curly => Underline::Curly,
                GUnderline::Dotted => Underline::Dotted,
                GUnderline::Dashed => Underline::Dashed,
                _ => Underline::Single,
            },
            strike: style.strikethrough,
            inverse: style.inverse,
            fg: match style.fg_color {
                StyleColor::None => Color::Default,
                StyleColor::Palette(index) => Color::Indexed(index.0),
                StyleColor::Rgb(rgb) => Color::Rgb(rgb.r, rgb.g, rgb.b),
            },
            hyperlink: Some(String::from_utf8_lossy(&uri[..len]).into_owned()),
        }
    }

    fn scrollback_lines(&self) -> usize {
        self.term.scrollback_rows().unwrap_or(0)
    }

    fn compress_scrollback(&mut self) -> bool {
        self.term
            .compress(libghostty_vt::terminal::CompressionMode::Full)
            .is_ok()
    }

    fn mode(&self, mode: Mode) -> Option<bool> {
        let on = |m: GMode| self.term.mode(m).ok();
        match mode {
            Mode::AltScreen => on(GMode::ALT_SCREEN_SAVE),
            Mode::BracketedPaste => on(GMode::BRACKETED_PASTE),
            Mode::MouseTracking => self.term.is_mouse_tracking().ok(),
            Mode::MouseSgr => on(GMode::SGR_MOUSE),
            Mode::FocusReport => on(GMode::FOCUS_EVENT),
            Mode::SyncOutput => on(GMode::SYNC_OUTPUT),
            Mode::KittyKeyboard => self
                .term
                .kitty_keyboard_flags()
                .ok()
                .map(|flags| flags != KittyKeyFlags::DISABLED),
        }
    }
}
