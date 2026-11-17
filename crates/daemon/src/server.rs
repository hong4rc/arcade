//! The daemon: one thread, one event loop.
//!
//! Each turn of the loop does, in this order: (1) read what clients sent and write keyboard
//! bytes to the panes' terminals, (2) read at most 64 KiB from each pane that has output and give
//! it to the terminal emulation, (3) handle panes whose process ended, (4) build and send a
//! screen update for each attached client that can take one. The loop sleeps in the
//! operating system's event queue until something happens or a concrete deadline arrives; there
//! is no periodic timer.

use std::collections::{HashMap, HashSet};
use std::fs::File;
use std::io::{self, Read, Write};
use std::os::fd::{AsRawFd, OwnedFd};
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::sync::mpsc::{self, Receiver, Sender};
use std::time::{Duration, Instant};

use arcade_proto::{
    AttachRequest, ClientMessage, CreateMode, Decoder, DetachReason, PROTOCOL_VERSION,
    ServerMessage,
};
use arcade_term::{
    Code, Cursor, CursorShape, Event as TermEvent, Key, Named, Term, TermConfig, encode_key,
};
use mio::net::{UnixListener, UnixStream};
use mio::unix::SourceFd;
use mio::{Events, Interest, Poll, Token, Waker};

use crate::clipboard::Clipboard;
use crate::compose::Screen;
use crate::config::{self, Config, StatusPosition};
use crate::copy::Copy;
use crate::keymap::Keymap;
use crate::keys::{Input, KeyReader};
use crate::layout::{Layout, PaneId, Rect};
use crate::log::Log;
use crate::paths::prepare_socket_dir;
use crate::picker::Picker;
use crate::pty::{self, SpawnSpec};
use crate::render::Renderer;
use keys::KeyState;

mod actions;
mod frame;
mod keys;
mod mouse;
mod notices;
mod treeview;
mod upgrade;

pub use upgrade::{Resume, rehearse, resume};

const LISTENER: Token = Token(0);
const WAKER: Token = Token(1);
const FIRST_DYNAMIC_TOKEN: usize = 2;

/// Output read from one pane per turn of the loop.
const PANE_READ_LIMIT: usize = 64 * 1024;
/// Bytes read from one client per turn of the loop.
const CLIENT_READ_LIMIT: usize = 256 * 1024;
/// Most keyboard bytes kept waiting for a pane that does not read its input.
const PANE_INPUT_LIMIT: usize = 1024 * 1024;
/// Keyboard bytes kept waiting for a pane before we stop reading from its client, so that a
/// large paste is paced by the program instead of being dropped.
const PANE_INPUT_SOFT_LIMIT: usize = 256 * 1024;
/// Shortest time between two screen updates to one client: 60 a second at most.
const FRAME_INTERVAL: Duration = Duration::from_micros(16_667);
/// Output of the pane over the last two frame intervals below which updates are not paced (see
/// `frame_time`). A pane has to print more than this, about 120 KB per second, before the limit
/// of 60 updates per second applies.
const LIGHT_UPDATE_BYTES: usize = 4096;

/// Bytes a pane printed in the current and the previous frame interval.
#[derive(Default)]
struct OutputRate {
    window_start: Option<Instant>,
    current: usize,
    previous: usize,
}

impl OutputRate {
    fn add(&mut self, bytes: usize, now: Instant) {
        match self.window_start {
            Some(start) if now < start + FRAME_INTERVAL => {}
            Some(start) if now < start + 2 * FRAME_INTERVAL => {
                self.previous = self.current;
                self.current = 0;
                self.window_start = Some(now);
            }
            _ => {
                self.previous = 0;
                self.current = 0;
                self.window_start = Some(now);
            }
        }
        self.current += bytes;
    }

    fn is_light(&self) -> bool {
        self.current + self.previous < LIGHT_UPDATE_BYTES
    }
}
/// Largest piece of screen update put in one message.
const OUTPUT_CHUNK: usize = 1024 * 1024;
/// How long a pane must be silent before its old scrollback is compressed to save memory.
const IDLE_COMPRESS_DELAY: Duration = Duration::from_secs(1);
/// How long a freshly started daemon waits for its first client before it gives up.
const FIRST_CLIENT_GRACE: Duration = Duration::from_secs(10);
/// How long panes get to react to the hangup signal when the server is stopped.
const KILL_GRACE: Duration = Duration::from_secs(2);
/// How long a stopping daemon waits for clients to take their last messages.
const FLUSH_GRACE: Duration = Duration::from_secs(1);

#[derive(Clone, Debug)]
pub struct DaemonConfig {
    pub socket_path: PathBuf,
    pub scrollback_lines: usize,
    /// Where the daemon logs: next to the socket, with the extension `log`. `None` for no log.
    pub log_path: Option<PathBuf>,
    /// Size at which the log file is replaced by a new one (see `log::Log`).
    pub log_limit_bytes: u64,
    /// The settings used when there is no configuration file (and until it is read).
    pub config: Config,
    /// The configuration file. `None` reads none, which is what the tests use.
    pub config_path: Option<PathBuf>,
}

/// A log file of this size, plus the one before it, is the most the daemon keeps on disk.
pub const DEFAULT_LOG_LIMIT_BYTES: u64 = 1024 * 1024;

impl DaemonConfig {
    pub fn new(socket_path: impl Into<PathBuf>) -> Self {
        DaemonConfig {
            socket_path: socket_path.into(),
            scrollback_lines: arcade_term::DEFAULT_SCROLLBACK_LINES,
            log_path: None,
            log_limit_bytes: DEFAULT_LOG_LIMIT_BYTES,
            config: Config::default(),
            config_path: None,
        }
    }
}

struct Pane {
    id: u64,
    token: Token,
    master: File,
    /// The program's end of the terminal, kept until the output it left behind has been read
    /// (see `pty::Spawned::slave`).
    slave: Option<OwnedFd>,
    term: Term,
    pid: i32,
    /// Bytes waiting to be written to the pane's terminal.
    input: Vec<u8>,
    /// The terminal may still hold output we did not read yet.
    read_pending: bool,
    /// The screen changed since the last update was built.
    dirty: bool,
    /// How much the pane printed lately: tells a light update (a typed character echoed) from
    /// a flood.
    rate: OutputRate,
    /// Set when the first process ended; the text goes to the client.
    exit_note: Option<String>,
    /// When to start compressing scrollback: a moment after the last output.
    compress_at: Option<Instant>,
    /// Compression is in progress, one small step per turn of the loop.
    compressing: bool,
    /// Where the pane's cursor was when its screen was last read.
    last_cursor: Cursor,
    /// Set while the pane is in copy mode: the pane then shows this instead of its screen.
    copy: Option<Copy>,
    /// The title the program in the pane set (OSC 0 or 2); `None` while it has set none.
    title: Option<String>,
    /// The foreground process group that set `title`; `None` when the shell set it (or none was
    /// set). The title is dropped when that group is gone.
    title_owner: Option<i32>,
    /// Titles the program asked to keep (`CSI 22 t`), the newest last.
    title_stack: Vec<TitleSave>,
    /// The last progress report the program made (`OSC 9;4`), and how many it has made: a report
    /// that repeats the last one still counts, since programs repeat it to keep it shown.
    progress: Option<arcade_term::Progress>,
    progress_seq: u64,
    /// When the last report that is still shown came in: a report that nothing repeats goes
    /// after `[status] progress-expires` seconds, since the program may be gone without having
    /// taken it away.
    progress_at: Option<Instant>,
    /// The foreground process group that made the report; `None` when the shell did (or none
    /// could be found). The report goes at once when that group is gone.
    progress_owner: Option<i32>,
    /// What the shell says about the command that runs (OSC 133 marks); nothing when it sends none.
    command: crate::shellmarks::Track,
    /// The foreground group as last asked, and when: asking is a system call on the terminal,
    /// costly while a program floods the pane with titles.
    writer_seen: Option<(Option<i32>, Instant)>,
    /// The key capture, while it is on for this pane (off when the pane closes).
    capture: Option<crate::capture::Capture>,
}

/// A title kept by `CSI 22 t`, with who set it and who asked to keep it.
#[derive(Clone, Debug)]
struct TitleSave {
    title: Option<String>,
    owner: Option<i32>,
    /// The process group that asked; its entries go when it does.
    by: Option<i32>,
}

/// How many titles a pane keeps for its program; one more push drops the oldest.
const TITLE_STACK_DEPTH: usize = 10;

/// How long the foreground group asked for last is taken to be still the one.
const WRITER_CACHE: Duration = Duration::from_millis(50);

impl Pane {
    /// The foreground process group that is writing now, `None` for the shell itself; the answer
    /// of the last 50 milliseconds is reused.
    fn writer_group(&mut self, now: Instant) -> Option<i32> {
        if let Some((group, at)) = self.writer_seen
            && now.duration_since(at) < WRITER_CACHE
        {
            return group;
        }
        let group = pty::foreground_group(self.master.as_raw_fd()).filter(|g| *g != self.pid);
        self.writer_seen = Some((group, now));
        group
    }

    /// Drop what the programs that are gone left behind: the title one of them set (the title it
    /// had kept comes back, when it kept one, else the pane has none) and the titles they kept.
    /// Returns whether the title changed.
    fn expire_titles(&mut self) -> bool {
        let gone = |group: Option<i32>| group.is_some_and(|g| !pty::group_alive(g));
        let before = self.title.clone();
        // Each round settles one dead owner; the stack is short, so a few rounds are all.
        for _ in 0..=TITLE_STACK_DEPTH {
            if !gone(self.title_owner) {
                break;
            }
            let owner = self.title_owner;
            match self.title_stack.iter().position(|s| s.by == owner) {
                Some(i) => {
                    self.title = self.title_stack[i].title.clone();
                    self.title_owner = self.title_stack[i].owner;
                }
                None => {
                    self.title = None;
                    self.title_owner = None;
                }
            }
            self.title_stack.retain(|s| s.by != owner);
        }
        self.title_stack.retain(|s| !gone(s.by));
        self.title != before
    }

    /// Take the progress report away when the program that made it is gone (it ended without
    /// saying so, and the shell or another program is in front). Returns whether it went.
    fn expire_progress_owner(&mut self) -> bool {
        let Some(owner) = self.progress_owner else {
            return false;
        };
        if self.progress.is_none() || pty::group_alive(owner) {
            return false;
        }
        self.progress = None;
        self.progress_at = None;
        self.progress_owner = None;
        self.progress_seq += 1;
        true
    }

    /// Something about the pane has to be drawn again.
    fn wants_frame(&self) -> bool {
        match &self.copy {
            Some(copy) => copy.dirty,
            None => self.dirty,
        }
    }
}

struct Window {
    id: u64,
    /// The number shown on the tab and used by `window.select`.
    number: usize,
    name: String,
    /// The user chose the name, so it is not taken from the working directory any more.
    name_manual: bool,
    /// The program in the foreground of the focused pane and its directory (the last part, `~`
    /// for the home directory), looked at about once a second and only while a format uses them.
    program: String,
    directory: String,
    layout: Layout,
    focus: PaneId,
    /// The pane that had the focus before this one.
    last_focus: Option<PaneId>,
    /// The focused pane fills the whole window; the layout underneath is kept.
    zoomed: bool,
    /// The arrangement `window.layout-next` applied last, which it goes on from.
    arrangement: Option<crate::layout::Arrangement>,
}

impl Window {
    /// The room of every pane: the layout's, except that a zoomed pane has the whole window.
    fn rects(&self) -> Vec<(PaneId, Rect)> {
        let mut rects = self.layout.rects();
        if self.zoomed {
            for (pane, rect) in &mut rects {
                if *pane == self.focus {
                    *rect = self.layout.area();
                }
            }
        }
        rects
    }

    /// The panes that show, with their rooms.
    fn visible_rects(&self) -> Vec<(PaneId, Rect)> {
        if self.zoomed {
            vec![(self.focus, self.layout.area())]
        } else {
            self.layout.rects()
        }
    }
}

struct Session {
    id: u64,
    name: String,
    /// The environment the session was created with, for the panes made later.
    env: Vec<String>,
    /// Where the session was created: the working directory of a pane whose own is unknown.
    cwd: String,
    windows: Vec<Window>,
    /// The window shown.
    current: u64,
    /// The window shown before the current one.
    last_window: Option<u64>,
    /// Size of the attached client's terminal.
    cols: u16,
    rows: u16,
    client: Option<u64>,
    /// Ordering of use, for "the session used last".
    last_used: u64,
    /// The client's screen is new or changed shape: the next update must redraw all of it.
    need_full: bool,
    /// The panes changed room or the window shown changed: they are all read again in full.
    relayout: bool,
    /// The dividers or the status bar must be drawn again.
    chrome_dirty: bool,
    /// Whether the bar as last drawn has a symbol that blinks on it, and in which half of a blink
    /// it was drawn (`[status] blink`).
    bar_blinks: bool,
    bar_blink_dim: bool,
    /// What the client's screen should show, and what it was last sent.
    screen: Screen,
    renderer: Renderer,
    /// When the last update was sent to the attached client.
    last_frame: Option<Instant>,
    /// When the windows' automatic names were last compared with the panes' directories.
    names_checked: Option<Instant>,
    /// The title last sent to the attached client's terminal; `None` until one is sent.
    sent_title: Option<String>,
    /// What the title was made from the last time, so that a title nothing changed for is not
    /// made again at every turn of the loop.
    title_inputs: Option<frame::TitleInputs>,
    /// Which progress report the attached client's terminal was last given: the pane and its
    /// count of reports; `None` while it shows none.
    sent_progress: Option<(u64, u64)>,
    /// The columns of the status bar that each window's tab covers, as the bar was last drawn:
    /// the window, from, to (not included). Empty while something else takes the bar's place.
    tab_bar: Vec<(u64, u16, u16)>,
    /// Where the counter for the other sessions is on the bar (a click on it opens the list of
    /// notifications).
    counter_bar: Option<(u16, u16)>,
    /// When the windows' names are to be looked at again because the last look was skipped.
    names_due: Option<Instant>,
    /// The attached client's keyboard bytes that are not complete yet.
    keys: KeyReader,
    /// The prefix, the lock, the prompt and the message line.
    key_state: KeyState,
    /// The floating list, while one is open.
    picker: Option<Picker>,
    /// A mouse button held down since a press in a pane.
    drag: Option<mouse::Drag>,
    /// A divider held with the mouse since a press on it: the window it is in, and which divider.
    divider_drag: Option<(u64, crate::layout::DividerHandle)>,
    /// A window whose layout moved while a divider was dragged: its panes are given their new
    /// sizes (and the programs told) once per frame, not once per mouse event.
    sizes_stale: Option<u64>,
    /// The tree of sessions, windows and panes, while it is open.
    tree: Option<crate::tree::Tree>,
    /// The host terminal has the focus, as its focus reports say (it has, when a client attaches).
    host_focused: bool,
    /// When the last BEL went to the attached client, and when the one that waited is due.
    last_bell: Option<Instant>,
    bell_due: Option<Instant>,
    /// The pane that sent the latest signal out while the host terminal was not looked at: where
    /// the client goes when the terminal gets the focus back.
    jump_candidate: Option<u64>,
    /// After such a jump: the pane it left and the pane it went to; the return key goes back.
    return_to: Option<(u64, u64)>,
    /// What the host terminal can send, once it has answered the question asked at attach.
    host_keyboard: Option<crate::keytools::Host>,
    /// The question of the host terminal's keyboard is out and not all answered; true when the
    /// answer that says the kitty protocol is there has come.
    host_probe: Option<bool>,
}

/// The shortest time between two looks at the panes' directories for the windows' names.
const NAME_CHECK_INTERVAL: Duration = Duration::from_secs(1);
/// How long, at most, to wait for the last output of a pane whose process has ended, when
/// something it started still holds the terminal open.
const EXIT_OUTPUT_DEADLINE: Duration = Duration::from_millis(300);

/// The size of the area the windows have, inside a client terminal of this size: the status bar
/// takes one row when it shows (and there is more than one row).
fn window_area(cols: u16, rows: u16, status: StatusPosition) -> (u16, u16) {
    let rows = if rows > 1 && status != StatusPosition::Off {
        rows - 1
    } else {
        rows
    };
    (cols.max(1), rows.max(1))
}

/// The row of the status bar, if it shows.
fn status_row(rows: u16, status: StatusPosition) -> Option<u16> {
    match status {
        StatusPosition::Off => None,
        _ if rows <= 1 => None,
        StatusPosition::Top => Some(0),
        StatusPosition::Bottom => Some(rows - 1),
    }
}

/// The row of the client's screen where the windows start.
fn window_origin(rows: u16, status: StatusPosition) -> u16 {
    u16::from(status == StatusPosition::Top && rows > 1)
}

/// What the status bar says when the file has mistakes: the first one, and how many there are.
fn summarize_config_errors(errors: &[String]) -> String {
    match errors {
        [] => "the configuration file has mistakes".to_string(),
        [one] => format!("config: {one}"),
        [first, rest @ ..] => format!(
            "config: {first} (and {} more; `arcade config check` lists them)",
            rest.len()
        ),
    }
}

impl Session {
    /// A session with no client, whose screen is to be drawn in full.
    #[allow(clippy::too_many_arguments)]
    fn new(
        id: u64,
        name: String,
        env: Vec<String>,
        cwd: String,
        windows: Vec<Window>,
        current: u64,
        (cols, rows): (u16, u16),
        last_used: u64,
    ) -> Session {
        Session {
            id,
            name,
            env,
            cwd,
            windows,
            current,
            last_window: None,
            cols,
            rows,
            client: None,
            last_used,
            need_full: true,
            relayout: true,
            chrome_dirty: true,
            bar_blinks: false,
            bar_blink_dim: false,
            screen: Screen::new(),
            renderer: Renderer::new(),
            last_frame: None,
            names_checked: None,
            sent_title: None,
            title_inputs: None,
            sent_progress: None,
            tab_bar: Vec::new(),
            counter_bar: None,
            names_due: None,
            keys: KeyReader::new(),
            key_state: KeyState::new(),
            picker: None,
            drag: None,
            divider_drag: None,
            sizes_stale: None,
            tree: None,
            host_focused: true,
            last_bell: None,
            bell_due: None,
            jump_candidate: None,
            return_to: None,
            host_keyboard: None,
            host_probe: None,
        }
    }

    fn window(&self, id: u64) -> Option<&Window> {
        self.windows.iter().find(|w| w.id == id)
    }

    fn window_mut(&mut self, id: u64) -> Option<&mut Window> {
        self.windows.iter_mut().find(|w| w.id == id)
    }

    fn current_window(&self) -> &Window {
        self.window(self.current)
            .unwrap_or_else(|| &self.windows[0])
    }

    fn pane_count(&self) -> usize {
        self.windows.iter().map(|w| w.layout.len()).sum()
    }

    fn all_panes(&self) -> Vec<PaneId> {
        self.windows
            .iter()
            .flat_map(|w| w.layout.rects().into_iter().map(|(p, _)| p))
            .collect()
    }
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum ClientState {
    New,
    /// Spoke another protocol version; only `server.kill` is allowed, so that a new client can
    /// still stop an old daemon.
    Mismatched,
    Ready,
    Attached(u64),
}

struct Client {
    stream: UnixStream,
    decoder: Decoder<ClientMessage>,
    out: Vec<u8>,
    out_pos: usize,
    state: ClientState,
    close_after_flush: bool,
    read_pending: bool,
    gone: bool,
    /// What this client's terminal can send, once it answered; kept when the client moves to
    /// another session, so the question is asked once for each time a client attaches.
    host_keyboard: Option<crate::keytools::Host>,
}

enum Target {
    Client(u64),
    Pane(u64),
}

struct Server {
    poll: Poll,
    listener: UnixListener,
    socket_path: PathBuf,
    scrollback_lines: usize,
    next_token: usize,
    next_session: u64,
    next_window: u64,
    use_counter: u64,
    targets: HashMap<usize, Target>,
    clients: HashMap<u64, Client>,
    sessions: HashMap<u64, Session>,
    panes: HashMap<u64, Pane>,
    exits_rx: Receiver<(u64, String)>,
    exits_tx: Sender<(u64, String)>,
    /// The process of each pane whose end is being waited for, until its status is taken.
    watched: HashMap<u64, i32>,
    /// This program's own file, held open since the start (see `upgrade`), and where it is.
    program_file: Option<File>,
    program_path: Option<PathBuf>,
    waker: Arc<Waker>,
    /// Panes whose process ended, waiting to be drained and closed.
    exited: Vec<u64>,
    /// The latest clipboard write of each pane, handled once the screen updates are out.
    pending_clipboard: HashMap<u64, Vec<u8>>,
    /// The notifications of the panes.
    notices: crate::notify::Store,
    /// What became of the latest signals, for `arcade notify log`.
    notice_log: std::collections::VecDeque<notices::LogEntry>,
    stopping: Option<Instant>,
    killed_hard: bool,
    ever_had_session: bool,
    empty_since: Option<Instant>,
    started: Instant,
    log: Log,
    config: Config,
    config_path: Option<PathBuf>,
    keymap: Keymap,
    /// A line to show on the status bar when a client next attaches (a mistake in the file the
    /// daemon started with).
    startup_message: Option<String>,
    /// What the last copy held.
    clipboard: Clipboard,
    /// The process that keeps the machine awake, while there is one; and whether it was asked
    /// to end (so that its ending is not a surprise).
    sleep_guard: Option<(crate::power::SleepGuard, bool)>,
}

/// The number the exit of the sleep guard is reported under, in place of a pane's.
const SLEEP_GUARD_EXIT: u64 = u64::MAX;

/// Run the daemon until its last session closes (or it is told to stop). Removes the socket
/// file on the way out.
pub fn run(config: DaemonConfig) -> io::Result<()> {
    prepare_socket_dir(&config.socket_path)?;
    claim_socket_path(&config.socket_path)?;
    let listener = UnixListener::bind(&config.socket_path)?;
    std::fs::set_permissions(
        &config.socket_path,
        std::os::unix::fs::PermissionsExt::from_mode(0o600),
    )?;
    let server = new_server(&config, listener)?;
    serve(server, &config, "daemon started")
}

/// A server with no sessions that listens on `listener`.
fn new_server(config: &DaemonConfig, mut listener: UnixListener) -> io::Result<Server> {
    let poll = Poll::new()?;
    poll.registry()
        .register(&mut listener, LISTENER, Interest::READABLE)?;
    let waker = Arc::new(Waker::new(poll.registry(), WAKER)?);
    let (exits_tx, exits_rx) = mpsc::channel();
    // A file with a mistake never stops the daemon: it starts with the settings it was given
    // and says so on the first status bar.
    let mut startup_message = None;
    let mut startup_log = Vec::new();
    let settings = match config.config_path.as_deref().map(config::load) {
        Some(Ok(settings)) => settings,
        Some(Err(errors)) => {
            startup_message = Some(summarize_config_errors(&errors));
            startup_log = errors;
            config.config.clone()
        }
        None => config.config.clone(),
    };
    let scrollback_lines = settings.scrollback_lines.unwrap_or(config.scrollback_lines);
    let program_path = std::env::current_exe().ok();
    let mut server = Server {
        poll,
        listener,
        socket_path: config.socket_path.clone(),
        scrollback_lines,
        next_token: FIRST_DYNAMIC_TOKEN,
        next_session: 1,
        next_window: 1,
        use_counter: 0,
        targets: HashMap::new(),
        clients: HashMap::new(),
        sessions: HashMap::new(),
        panes: HashMap::new(),
        exits_rx,
        exits_tx,
        watched: HashMap::new(),
        waker,
        exited: Vec::new(),
        pending_clipboard: HashMap::new(),
        notices: crate::notify::Store::default(),
        notice_log: notices::new_log(),
        stopping: None,
        killed_hard: false,
        ever_had_session: false,
        empty_since: None,
        started: Instant::now(),
        keymap: settings.keymap(),
        config: settings,
        config_path: config.config_path.clone(),
        startup_message,
        clipboard: Clipboard::default(),
        sleep_guard: None,
        // Held open from the start: an install may replace or remove the file later, and the
        // copy that an upgrade keeps for going back is made from this descriptor.
        program_file: program_path.as_deref().and_then(|p| File::open(p).ok()),
        program_path,
        log: config
            .log_path
            .as_deref()
            .map_or_else(Log::disabled, |path| {
                Log::open(path, config.log_limit_bytes)
            }),
    };
    for e in &startup_log {
        server.log.line(&format!("configuration: {e}"));
    }
    Ok(server)
}

/// Run a server until it is done, then clean up.
fn serve(mut server: Server, config: &DaemonConfig, started: &str) -> io::Result<()> {
    server.log.line(&format!(
        "{started}, process {}, version {}, protocol {}",
        std::process::id(),
        env!("CARGO_PKG_VERSION"),
        arcade_proto::PROTOCOL_VERSION
    ));
    server.sync_sleep_guard();
    // The look of the states is read on this thread, which draws every bar.
    crate::theme::set_state_look(server.config.state_look);
    let result = server.event_loop();
    match &result {
        Ok(()) => server.log.line("daemon stopped"),
        Err(e) => server.log.line(&format!("daemon failed: {:?}", e.kind())),
    }
    server.shutdown_cleanup();
    #[cfg(feature = "stage-timing")]
    crate::stage::dump(&config.socket_path.with_extension("stages-daemon"));
    #[cfg(not(feature = "stage-timing"))]
    let _ = config;
    result
}

/// Make the socket path free for binding: refuse if a daemon answers there, delete a stale file.
fn claim_socket_path(path: &Path) -> io::Result<()> {
    match std::os::unix::net::UnixStream::connect(path) {
        Ok(_) => Err(io::Error::new(
            io::ErrorKind::AddrInUse,
            format!("a daemon is already running on {}", path.display()),
        )),
        Err(e) if e.kind() == io::ErrorKind::NotFound => Ok(()),
        Err(_) => match std::fs::remove_file(path) {
            Ok(()) => Ok(()),
            Err(e) if e.kind() == io::ErrorKind::NotFound => Ok(()),
            Err(e) => Err(e),
        },
    }
}

fn would_block(e: &io::Error) -> bool {
    e.kind() == io::ErrorKind::WouldBlock
}

impl Server {
    fn alloc_token(&mut self) -> Token {
        let t = Token(self.next_token);
        self.next_token += 1;
        t
    }

    // ---- The loop ----

    fn event_loop(&mut self) -> io::Result<()> {
        let mut events = Events::with_capacity(256);
        let mut client_ready: HashSet<u64> = HashSet::new();
        let mut client_writable: HashSet<u64> = HashSet::new();
        let mut pane_ready: HashSet<u64> = HashSet::new();
        let mut pane_writable: HashSet<u64> = HashSet::new();
        loop {
            // Anything left over from the last turn means we must not sleep.
            let busy = self
                .clients
                .iter()
                .any(|(id, c)| c.read_pending && !self.client_blocked(*id))
                || self.panes.values().any(|p| p.read_pending || p.compressing)
                || !self.exited.is_empty()
                || self.needs_frame();
            let timeout = if busy {
                Some(Duration::ZERO)
            } else {
                self.next_deadline()
            };
            match self.poll.poll(&mut events, timeout) {
                Ok(()) => {
                    stage_mark!(DaemonWake);
                    stage_count!(Poll);
                }
                Err(e) if e.kind() == io::ErrorKind::Interrupted => continue,
                Err(e) => return Err(e),
            }
            for event in events.iter() {
                match event.token() {
                    LISTENER => self.accept_clients()?,
                    WAKER => self.collect_exits(),
                    Token(t) => match self.targets.get(&t) {
                        Some(Target::Client(id)) => {
                            if event.is_readable() || event.is_read_closed() || event.is_error() {
                                client_ready.insert(*id);
                            }
                            if event.is_writable() {
                                client_writable.insert(*id);
                            }
                        }
                        Some(Target::Pane(id)) => {
                            if event.is_readable() || event.is_read_closed() || event.is_error() {
                                pane_ready.insert(*id);
                            }
                            if event.is_writable() {
                                pane_writable.insert(*id);
                            }
                        }
                        None => {}
                    },
                }
            }
            // A previous turn may have left reads unfinished.
            client_ready.extend(
                self.clients
                    .iter()
                    .filter(|(_, c)| c.read_pending)
                    .map(|(id, _)| *id),
            );
            pane_ready.extend(self.panes.values().filter(|p| p.read_pending).map(|p| p.id));

            // (1) Clients first, so keys reach the pane before any output is handled.
            for id in client_writable.drain() {
                self.flush_client(id);
            }
            for id in client_ready.drain() {
                self.read_client(id);
            }
            for id in pane_writable.drain() {
                self.flush_pane_input(id);
            }
            // (2) Pane output, a bounded amount from each.
            let mut read_now: Vec<u64> = Vec::new();
            for id in pane_ready.drain() {
                self.read_pane(id, PANE_READ_LIMIT);
                read_now.push(id);
            }
            // (3) Processes that ended.
            self.finish_exited_panes();
            // (4) Screen updates.
            self.wake_name_checks();
            self.wake_running_marks();
            self.wake_progress_expiry();
            self.wake_blink();
            self.tick_drag_scrolls();
            self.send_frames();
            self.send_titles();
            self.send_progress();
            self.handle_pending_clipboard(&read_now);
            // (5) Panes that have been quiet shrink their old scrollback, a little at a time.
            self.compress_idle_panes();
            // Push out what was queued.
            let ids: Vec<u64> = self.clients.keys().copied().collect();
            for id in ids {
                self.flush_client(id);
            }
            self.reap_clients();

            if self.should_exit() {
                return Ok(());
            }
        }
    }

    /// The session the client is attached to.
    fn session_of_client(&self, client_id: u64) -> Option<u64> {
        self.sessions
            .values()
            .find(|s| s.client == Some(client_id))
            .map(|s| s.id)
    }

    /// The pane that has the focus in a session's current window.
    fn focus_pane_of(&self, session_id: u64) -> Option<u64> {
        self.sessions
            .get(&session_id)
            .map(|s| s.current_window().focus)
    }

    /// A client whose pane still has a lot of unwritten input is not read from for now.
    fn client_blocked(&self, client_id: u64) -> bool {
        self.session_of_client(client_id)
            .and_then(|s| self.focus_pane_of(s))
            .and_then(|p| self.panes.get(&p))
            .is_some_and(|p| p.input.len() >= PANE_INPUT_SOFT_LIMIT)
    }

    fn next_deadline(&self) -> Option<Duration> {
        let now = Instant::now();
        let mut deadline: Option<Instant> = None;
        let mut consider = |t: Instant| {
            deadline = Some(deadline.map_or(t, |d| d.min(t)));
        };
        if !self.ever_had_session && self.sessions.is_empty() {
            consider(self.started + FIRST_CLIENT_GRACE);
        }
        if let Some(t) = self.empty_since {
            consider(t + FLUSH_GRACE);
        }
        if let Some(t) = self.stopping {
            consider(t);
        }
        for p in self.panes.values() {
            if let Some(t) = self.running_mark_due(p) {
                consider(t);
            }
            if let Some(t) = self.progress_expiry_due(p) {
                consider(t);
            }
            if let Some(t) = p.term.stuck_deadline() {
                consider(t);
            }
            if let Some(t) = p.compress_at {
                consider(t);
            }
        }
        for s in self.sessions.values() {
            // One shared deadline for every blinking symbol, and none when nothing blinks.
            if s.client.is_some()
                && s.bar_blinks
                && let Some(t) = self.next_blink_change(now)
            {
                consider(t);
            }
            if let Some(t) = self.frame_time(s, now) {
                consider(t);
            }
            if let Some(t) = s.keys.deadline() {
                consider(t);
            }
            if let Some(t) = s.key_state.deadline() {
                consider(t);
            }
            if let Some(t) = s.drag.and_then(|d| d.next_tick) {
                consider(t);
            }
            if let Some(t) = s.bell_due {
                consider(t);
            }
            if s.client.is_some()
                && let Some(t) = s.names_due
            {
                consider(t);
            }
        }
        deadline.map(|t| t.saturating_duration_since(now))
    }

    /// Whether it is the dim half of a blink at `now`. Every blinking symbol of every bar is in
    /// the same half, counted from the start of the daemon.
    pub(super) fn blink_dim_at(&self, now: Instant) -> bool {
        let half = u128::from(self.config.state_look.blink_ms.max(1));
        (now.saturating_duration_since(self.started).as_millis() / half) % 2 == 1
    }

    /// When the next half of a blink starts.
    fn next_blink_change(&self, now: Instant) -> Option<Instant> {
        let half = u128::from(self.config.state_look.blink_ms.max(1));
        let since = now.saturating_duration_since(self.started).as_millis();
        let next = (since / half + 1) * half;
        Some(self.started + Duration::from_millis(u64::try_from(next).ok()?))
    }

    /// A bar with a symbol that blinks is drawn again when the half of the blink has changed
    /// since it was drawn; only the cells of the symbols differ, so only they are sent.
    fn wake_blink(&mut self) {
        let dim = self.blink_dim_at(Instant::now());
        for s in self.sessions.values_mut() {
            if s.client.is_some() && s.bar_blinks && s.bar_blink_dim != dim {
                s.chrome_dirty = true;
            }
        }
    }

    /// A session whose names were due to be looked at gets an update, which looks at them.
    fn wake_name_checks(&mut self) {
        let now = Instant::now();
        for s in self.sessions.values_mut() {
            if s.client.is_some()
                && let Some(t) = s.names_due
                && t <= now
            {
                s.names_due = None;
                s.chrome_dirty = true;
            }
        }
    }

    fn should_exit(&mut self) -> bool {
        let now = Instant::now();
        if self.sessions.is_empty() {
            if !self.ever_had_session && self.stopping.is_none() {
                // Nobody created a session yet; give up when the start-up grace is over.
                return now >= self.started + FIRST_CLIENT_GRACE;
            }
            // The last session is gone. Let clients take their final messages, but not forever.
            let since = *self.empty_since.get_or_insert(now);
            let pending = self.clients.values().any(|c| !c.gone && !c.out.is_empty());
            return !pending || now >= since + FLUSH_GRACE;
        }
        self.empty_since = None;
        if let Some(t) = self.stopping
            && now >= t
            && !self.killed_hard
        {
            self.killed_hard = true;
            for p in self.panes.values() {
                pty::signal_group(p.pid, libc::SIGKILL);
            }
        }
        false
    }

    // ---- Clients ----

    fn accept_clients(&mut self) -> io::Result<()> {
        loop {
            match self.listener.accept() {
                Ok((mut stream, _)) => {
                    let token = self.alloc_token();
                    self.poll.registry().register(
                        &mut stream,
                        token,
                        Interest::READABLE | Interest::WRITABLE,
                    )?;
                    self.targets.insert(token.0, Target::Client(token.0 as u64));
                    self.clients.insert(
                        token.0 as u64,
                        Client {
                            stream,
                            decoder: Decoder::new(),
                            out: Vec::new(),
                            out_pos: 0,
                            state: ClientState::New,
                            close_after_flush: false,
                            host_keyboard: None,
                            read_pending: false,
                            gone: false,
                        },
                    );
                    self.log.line(&format!("client {} connected", token.0));
                }
                Err(e) if would_block(&e) => return Ok(()),
                Err(e) if e.kind() == io::ErrorKind::Interrupted => {}
                // A connection that died before we accepted it is not our problem.
                Err(_) => return Ok(()),
            }
        }
    }

    fn send(&mut self, id: u64, msg: &ServerMessage) {
        if let Some(client) = self.clients.get_mut(&id) {
            if client.gone {
                return;
            }
            if arcade_proto::encode(msg, &mut client.out).is_err() {
                client.gone = true;
            }
        }
    }

    fn send_output(&mut self, id: u64, bytes: &[u8]) {
        for chunk in bytes.chunks(OUTPUT_CHUNK) {
            self.send(id, &ServerMessage::Output(chunk.to_vec()));
        }
    }

    fn flush_client(&mut self, id: u64) {
        let Some(client) = self.clients.get_mut(&id) else {
            return;
        };
        while client.out_pos < client.out.len() && !client.gone {
            stage_count!(Write);
            match client.stream.write(&client.out[client.out_pos..]) {
                Ok(0) => client.gone = true,
                Ok(n) => {
                    client.out_pos += n;
                    stage_mark!(DaemonSocketWritten);
                }
                Err(e) if would_block(&e) => break,
                Err(e) if e.kind() == io::ErrorKind::Interrupted => {}
                Err(_) => client.gone = true,
            }
        }
        if client.out_pos >= client.out.len() {
            client.out.clear();
            client.out_pos = 0;
            if client.close_after_flush {
                client.gone = true;
            }
        } else if client.out_pos > 64 * 1024 {
            client.out.drain(..client.out_pos);
            client.out_pos = 0;
        }
    }

    fn read_client(&mut self, id: u64) {
        if self.client_blocked(id) {
            // Come back when the pane has taken its input.
            if let Some(c) = self.clients.get_mut(&id) {
                c.read_pending = true;
            }
            return;
        }
        let mut messages = Vec::new();
        let mut protocol_error = None;
        {
            let Some(client) = self.clients.get_mut(&id) else {
                return;
            };
            if client.gone {
                return;
            }
            let mut buf = [0u8; 16 * 1024];
            let mut total = 0;
            client.read_pending = false;
            stage_mark!(DaemonBeforeRead);
            loop {
                stage_count!(Read);
                match client.stream.read(&mut buf) {
                    Ok(0) => {
                        client.gone = true;
                        break;
                    }
                    Ok(n) => {
                        client.decoder.push(&buf[..n]);
                        total += n;
                        if total >= CLIENT_READ_LIMIT {
                            client.read_pending = true;
                            break;
                        }
                        // A read that did not fill the buffer took everything the socket held;
                        // what arrives later reports itself again, so no read is needed just to
                        // be told "nothing more".
                        if n < buf.len() {
                            break;
                        }
                    }
                    Err(e) if would_block(&e) => break,
                    Err(e) if e.kind() == io::ErrorKind::Interrupted => {}
                    Err(_) => {
                        client.gone = true;
                        break;
                    }
                }
            }
            stage_mark!(DaemonSocketRead);
            loop {
                match client.decoder.next_message() {
                    Ok(Some(m)) => messages.push(m),
                    Ok(None) => break,
                    Err(e) => {
                        protocol_error = Some(e);
                        break;
                    }
                }
            }
        }
        stage_mark!(DaemonDecoded);
        for message in messages {
            if self.clients.get(&id).is_none_or(|c| c.gone) {
                break;
            }
            self.handle_message(id, message);
        }
        if let Some(e) = protocol_error {
            self.log
                .line(&format!("client {id} sent a bad message: {e}"));
            self.send(
                id,
                &ServerMessage::CommandResult {
                    ok: false,
                    output: format!("protocol error: {e}"),
                },
            );
            if let Some(client) = self.clients.get_mut(&id) {
                client.close_after_flush = true;
            }
        }
    }

    fn handle_message(&mut self, id: u64, message: ClientMessage) {
        let state = match self.clients.get(&id) {
            Some(c) => c.state,
            None => return,
        };
        match message {
            ClientMessage::Hello { version } => {
                if version == PROTOCOL_VERSION {
                    self.send(
                        id,
                        &ServerMessage::HelloOk {
                            version: PROTOCOL_VERSION,
                        },
                    );
                    if let Some(c) = self.clients.get_mut(&id) {
                        c.state = ClientState::Ready;
                    }
                } else {
                    self.send(
                        id,
                        &ServerMessage::VersionMismatch {
                            daemon: PROTOCOL_VERSION,
                            client: version,
                        },
                    );
                    if let Some(c) = self.clients.get_mut(&id) {
                        c.state = ClientState::Mismatched;
                    }
                }
            }
            // A client of another protocol version can still stop the daemon, and can ask it to
            // continue as that client's program: the daemon becomes the new program, so what the
            // two protocols share (the first message and the layout of a command) is enough.
            ClientMessage::Command { name, args }
                if state == ClientState::Mismatched && name == "server.upgrade" =>
            {
                self.command_from_client(id, &name, &args, None);
            }
            ClientMessage::Command { name, args }
                if state == ClientState::Mismatched
                    && (name == "server.kill" || name == "server.version") =>
            {
                let (ok, output) = self.run_command(&name, &args, None);
                self.send(id, &ServerMessage::CommandResult { ok, output });
                if let Some(c) = self.clients.get_mut(&id) {
                    c.close_after_flush = true;
                }
            }
            _ if state == ClientState::Mismatched => {
                if let Some(c) = self.clients.get_mut(&id) {
                    c.close_after_flush = true;
                }
            }
            _ if state == ClientState::New => {
                self.send(
                    id,
                    &ServerMessage::CommandResult {
                        ok: false,
                        output: "the first message must be Hello".into(),
                    },
                );
                if let Some(c) = self.clients.get_mut(&id) {
                    c.close_after_flush = true;
                }
            }
            ClientMessage::Attach(request) => self.attach(id, request),
            ClientMessage::Input(bytes) => {
                if let ClientState::Attached(session) = state {
                    self.handle_input(session, &bytes);
                }
            }
            ClientMessage::Resize { cols, rows } => {
                if let ClientState::Attached(session) = state {
                    self.resize_session(session, cols, rows);
                }
            }
            ClientMessage::Detach => {
                if let ClientState::Attached(session) = state {
                    self.detach_on_request(session);
                }
            }
            ClientMessage::Command { name, args } => {
                self.command_from_client(id, &name, &args, None)
            }
            ClientMessage::PaneCommand { pane, name, args } => {
                self.command_from_client(id, &name, &args, Some(pane));
            }
        }
    }

    fn command_from_client(&mut self, id: u64, name: &str, args: &[String], pane: Option<u64>) {
        if name == "server.upgrade" {
            // Does not come back when it works: the program is replaced.
            let (ok, output) = self.upgrade(id, args);
            self.send(id, &ServerMessage::CommandResult { ok, output });
            if let Some(c) = self.clients.get_mut(&id) {
                c.close_after_flush = true;
            }
            return;
        }
        let (ok, output) = self.run_command(name, args, pane);
        self.send(id, &ServerMessage::CommandResult { ok, output });
        if let Some(c) = self.clients.get_mut(&id)
            && !matches!(c.state, ClientState::Attached(_))
        {
            c.close_after_flush = true;
        }
    }

    fn reap_clients(&mut self) {
        let gone: Vec<u64> = self
            .clients
            .iter()
            .filter(|(_, c)| c.gone)
            .map(|(id, _)| *id)
            .collect();
        for id in gone {
            if let Some(mut client) = self.clients.remove(&id) {
                self.log.line(&format!("client {id} disconnected"));
                let _ = self.poll.registry().deregister(&mut client.stream);
                self.targets.remove(&(id as usize));
                // A client that disappears only detaches; the session carries on.
                let mut idle_panes = Vec::new();
                for s in self.sessions.values_mut() {
                    if s.client == Some(id) {
                        s.client = None;
                        // Nobody is looking: the grid and the last frame sent are given back.
                        // The next attach redraws everything and builds them again.
                        s.screen = Screen::new();
                        s.renderer = Renderer::new();
                        idle_panes.extend(s.all_panes());
                    }
                }
                for pane in idle_panes {
                    if let Some(pane) = self.panes.get_mut(&pane) {
                        pane.term.release_snapshot_state();
                    }
                }
            }
        }
    }

    // ---- Sessions ----

    fn attach(&mut self, id: u64, request: AttachRequest) {
        let (cols, rows) = (request.cols.max(1), request.rows.max(1));
        if let Some(message) = self.attach_from_inside(&request) {
            self.send(
                id,
                &ServerMessage::CommandResult {
                    ok: false,
                    output: message,
                },
            );
            if let Some(c) = self.clients.get_mut(&id) {
                c.close_after_flush = true;
            }
            return;
        }
        let session_id = match self.find_or_create(&request) {
            Ok(session_id) => session_id,
            Err(message) => {
                self.send(
                    id,
                    &ServerMessage::CommandResult {
                        ok: false,
                        output: message,
                    },
                );
                if let Some(c) = self.clients.get_mut(&id) {
                    c.close_after_flush = true;
                }
                return;
            }
        };
        self.bind_client(id, session_id, cols, rows);
    }

    /// Why an attach request is refused when it comes from a pane of the session it asks for: the
    /// screen would show Arcade drawing itself and the client that is there would be thrown out.
    /// The request says where it comes from in its environment (`ARCADE` is this daemon's socket,
    /// `ARCADE_PANE` the pane); an Arcade of another socket is real nesting and goes through.
    fn attach_from_inside(&self, request: &AttachRequest) -> Option<String> {
        if request.create == CreateMode::Always {
            return None;
        }
        let value = |name: &str| {
            request
                .env
                .iter()
                .find_map(|e| e.strip_prefix(name)?.strip_prefix('='))
        };
        let socket = value("ARCADE")?;
        if std::path::Path::new(socket) != self.socket_path.as_path() {
            return None;
        }
        let pane: u64 = value("ARCADE_PANE")?.parse().ok()?;
        let (session, _) = self.locate_pane(pane)?;
        (self.pick_session(request) == Some(session)).then(|| {
            "already inside Arcade: this is a pane of that session; use `arcade new` or `arcade attach -t <session>` to go to another one".to_string()
        })
    }

    /// Make `id` the client of the session. The session's earlier client, if another, is told and
    /// let go: a session has one client at a time.
    fn bind_client(&mut self, id: u64, session_id: u64, cols: u16, rows: u16) {
        // Only one client per session: the earlier one is told and let go.
        let previous = self.sessions.get(&session_id).and_then(|s| s.client);
        if let Some(previous) = previous
            && previous != id
        {
            self.detach_client(
                session_id,
                DetachReason::Replaced,
                "detached: another client attached to this session",
            );
        }
        let known_host = self.clients.get(&id).and_then(|c| c.host_keyboard);
        self.use_counter += 1;
        let name = {
            let Some(session) = self.sessions.get_mut(&session_id) else {
                return;
            };
            session.client = Some(id);
            session.keys = KeyReader::new();
            session.key_state.prefix = keys::PrefixState::Idle;
            session.key_state.prompt = None;
            session.picker = None;
            session.tree = None;
            // A person who attaches is looking at the terminal.
            session.host_focused = true;
            // What this terminal can send is asked once for each client; a client that moves
            // over from another session has answered already.
            session.host_keyboard = known_host;
            session.host_probe = known_host.is_none().then_some(false);
            if let Some(message) = self.startup_message.take() {
                session.key_state.show_message(
                    message,
                    crate::status::MessageKind::Error,
                    Instant::now(),
                );
            }
            session.need_full = true;
            session.sent_title = None;
            session.title_inputs = None;
            // A terminal that was shown another session's bar must be told what is true here.
            session.sent_progress = Some((u64::MAX, 0));
            session.last_used = self.use_counter;
            session.name.clone()
        };
        if let Some(c) = self.clients.get_mut(&id) {
            c.state = ClientState::Attached(session_id);
        }
        if let Some(session) = self.sessions.get_mut(&session_id) {
            session.drag = None;
            session.divider_drag = None;
        }
        self.send(id, &ServerMessage::Attached { session: name });
        if known_host.is_none() {
            self.send_output(id, crate::keytools::HOST_QUERY);
        }
        self.resize_session(session_id, cols, rows);
    }

    fn find_or_create(&mut self, request: &AttachRequest) -> Result<u64, String> {
        if request.create == CreateMode::Always {
            return self.create_session(request);
        }
        match self.pick_session(request) {
            Some(id) => Ok(id),
            None if request.create == CreateMode::IfNone && request.session.is_none() => {
                self.create_session(request)
            }
            None => match &request.session {
                Some(name) => Err(format!("no such session: {name}")),
                None => Err("no sessions".to_string()),
            },
        }
    }

    fn pick_session(&self, request: &AttachRequest) -> Option<u64> {
        match &request.session {
            Some(name) => self
                .sessions
                .values()
                .find(|s| &s.name == name)
                .map(|s| s.id),
            None => self
                .sessions
                .values()
                .max_by_key(|s| s.last_used)
                .map(|s| s.id),
        }
    }

    /// Start a process in a new pseudo terminal and give it a terminal emulation. The pane is
    /// registered with the event queue but belongs to no window yet.
    fn spawn_pane(
        &mut self,
        argv: Vec<String>,
        cwd: String,
        base_env: &[String],
        cols: u16,
        rows: u16,
    ) -> Result<Pane, String> {
        let token = self.alloc_token();
        let pane_id = token.0 as u64;
        let mut env: Vec<String> = base_env
            .iter()
            .filter(|e| {
                !e.starts_with("ARCADE=")
                    && !e.starts_with("ARCADE_PANE=")
                    && !e.starts_with("TERM=")
                    && !e.starts_with("COLORTERM=")
            })
            .cloned()
            .collect();
        env.push(format!("ARCADE={}", self.socket_path.display()));
        env.push(format!("ARCADE_PANE={pane_id}"));
        env.push("TERM=xterm-256color".to_string());
        env.push("COLORTERM=truecolor".to_string());

        let (cols, rows) = (cols.max(1), rows.max(1));
        let spawned = pty::spawn(&SpawnSpec {
            argv,
            cwd,
            env,
            cols,
            rows,
        })
        .map_err(|e| e.to_string())?;

        let mut config = TermConfig::new(cols, rows);
        config.scrollback_lines = self.scrollback_lines;
        let term = Term::new(config).map_err(|e| e.to_string())?;

        let fd = spawned.master.as_raw_fd();
        self.poll
            .registry()
            .register(
                &mut SourceFd(&fd),
                token,
                Interest::READABLE | Interest::WRITABLE,
            )
            .map_err(|e| e.to_string())?;
        self.targets.insert(token.0, Target::Pane(pane_id));

        // A thread that only waits for the process to end and wakes the loop; it never touches
        // terminal state.
        // The process is not waited for through the child object: the wait is by process number
        // and leaves the status for the loop to take, so a daemon that replaces its own program
        // keeps its children and loses no exit.
        drop(spawned.child);
        self.watch_exit(pane_id, spawned.pid);

        Ok(Pane {
            id: pane_id,
            token,
            master: spawned.master,
            slave: Some(spawned.slave),
            term,
            pid: spawned.pid,
            input: Vec::new(),
            read_pending: true,
            dirty: true,
            rate: OutputRate::default(),
            exit_note: None,
            compress_at: None,
            compressing: false,
            last_cursor: Cursor {
                x: 0,
                y: 0,
                visible: true,
                shape: CursorShape::Block,
                blinking: false,
            },
            copy: None,
            title: None,
            title_owner: None,
            title_stack: Vec::new(),
            progress: None,
            progress_seq: 0,
            progress_at: None,
            progress_owner: None,
            command: crate::shellmarks::Track::default(),
            writer_seen: None,
            capture: None,
        })
    }

    fn create_session(&mut self, request: &AttachRequest) -> Result<u64, String> {
        let name = match &request.new_name {
            Some(name) => {
                if self.sessions.values().any(|s| &s.name == name) {
                    return Err(format!("duplicate session: {name}"));
                }
                name.clone()
            }
            None => self.unique_name(&default_session_name(&request.cwd)),
        };
        let (cols, rows) = (request.cols.max(1), request.rows.max(1));
        let (area_cols, area_rows) = window_area(cols, rows, self.config.status_position);
        let pane = self.spawn_pane(
            request.command.clone(),
            request.cwd.clone(),
            &request.env,
            area_cols,
            area_rows,
        )?;
        let session_id = self.next_session;
        self.next_session += 1;
        let window_id = self.next_window;
        self.next_window += 1;
        self.log.line(&format!(
            "session {session_id} created, pane process {}",
            pane.pid
        ));
        self.use_counter += 1;
        self.ever_had_session = true;
        let pane_id = pane.id;
        self.panes.insert(pane_id, pane);
        let first_name = name_at_creation(
            self.config.window_name_from,
            &request.cwd,
            first_program_name(&request.command, &request.env),
        );
        self.sessions.insert(
            session_id,
            Session::new(
                session_id,
                name,
                request.env.clone(),
                request.cwd.clone(),
                vec![Window {
                    id: window_id,
                    name: first_name,
                    number: self.config.base_index,
                    name_manual: false,
                    program: String::new(),
                    directory: String::new(),
                    layout: Layout::new(pane_id, area_cols, area_rows),
                    focus: pane_id,
                    last_focus: None,
                    zoomed: false,
                    arrangement: None,
                }],
                window_id,
                (cols, rows),
                self.use_counter,
            ),
        );
        Ok(session_id)
    }

    fn unique_name(&self, base: &str) -> String {
        if !self.sessions.values().any(|s| s.name == base) {
            return base.to_string();
        }
        (2..)
            .map(|n| format!("{base}-{n}"))
            .find(|candidate| !self.sessions.values().any(|s| &s.name == candidate))
            .unwrap_or_else(|| base.to_string())
    }

    /// The client asked to leave: the last line it prints names the session it left.
    pub(super) fn detach_on_request(&mut self, session_id: u64) {
        let name = self
            .sessions
            .get(&session_id)
            .map(|s| s.name.clone())
            .unwrap_or_default();
        self.detach_client(
            session_id,
            DetachReason::Requested,
            &format!("detached: {name}"),
        );
    }

    fn detach_client(&mut self, session_id: u64, reason: DetachReason, message: &str) {
        let Some((client_id, bar_shown)) = self.sessions.get_mut(&session_id).and_then(|s| {
            let shown = s.sent_progress.take().is_some();
            s.client.take().map(|c| (c, shown))
        }) else {
            return;
        };
        self.send_output(client_id, crate::render::MOUSE_OFF);
        // The bar of a progress report must not stay behind in the terminal.
        if bar_shown {
            self.send_output(client_id, frame::PROGRESS_REMOVE);
        }
        self.send(
            client_id,
            &ServerMessage::Detached {
                reason,
                message: message.to_string(),
            },
        );
        if let Some(c) = self.clients.get_mut(&client_id) {
            c.state = ClientState::Ready;
            c.close_after_flush = true;
        }
    }

    /// The client's terminal has this size: every window is laid out again for it.
    fn resize_session(&mut self, session_id: u64, cols: u16, rows: u16) {
        let (cols, rows) = (cols.max(1), rows.max(1));
        let (area_cols, area_rows) = window_area(cols, rows, self.config.status_position);
        let Some(session) = self.sessions.get_mut(&session_id) else {
            return;
        };
        session.cols = cols;
        session.rows = rows;
        for window in &mut session.windows {
            window.layout.set_area(area_cols, area_rows);
        }
        // The client's screen changes shape, so it needs everything again.
        session.need_full = true;
        session.relayout = true;
        let windows: Vec<u64> = session.windows.iter().map(|w| w.id).collect();
        for window_id in windows {
            self.apply_window_sizes(session_id, window_id);
        }
    }

    /// Give every pane of the window the size of its room, telling the program in it.
    fn apply_window_sizes(&mut self, session_id: u64, window_id: u64) {
        let rects = match self
            .sessions
            .get(&session_id)
            .and_then(|s| s.window(window_id))
        {
            Some(window) => window.rects(),
            None => return,
        };
        for (pane_id, rect) in rects {
            self.set_pane_size(pane_id, rect.w, rect.h);
        }
    }

    fn set_pane_size(&mut self, pane_id: u64, cols: u16, rows: u16) {
        let (cols, rows) = (cols.max(1), rows.max(1));
        let Some(pane) = self.panes.get_mut(&pane_id) else {
            return;
        };
        if pane.term.cols() != cols || pane.term.rows() != rows {
            let _ = pty::set_window_size(&pane.master, cols, rows);
            if pane.term.resize(cols, rows).is_err() {
                pane.exit_note = Some("[terminal error]".into());
            }
            // The rows copy mode counts from change with the width; it starts over.
            pane.copy = None;
            pane.dirty = true;
        }
    }

    // ---- Panes ----

    fn write_to_pane(&mut self, pane_id: u64, bytes: &[u8]) {
        if let Some(pane) = self.panes.get_mut(&pane_id) {
            queue_pane_input(pane, bytes);
        }
    }

    fn flush_pane_input(&mut self, pane_id: u64) {
        let mut unblocked = None;
        if let Some(pane) = self.panes.get_mut(&pane_id) {
            write_pending(pane);
            if pane.input.len() < PANE_INPUT_SOFT_LIMIT {
                unblocked = self
                    .sessions
                    .values()
                    .find(|s| s.current_window().focus == pane_id)
                    .and_then(|s| s.client);
            }
        }
        // The client was not being read; now it can be.
        if let Some(c) = unblocked.and_then(|id| self.clients.get_mut(&id)) {
            c.read_pending = true;
        }
    }

    /// Read up to `limit` bytes of output from a pane and give them to its terminal.
    /// Read what a pane's program wrote. True when its output has ended for good: the master
    /// reports the end (a read of nothing, or the error Linux gives), which is when no process
    /// holds the other end any more.
    fn read_pane(&mut self, pane_id: u64, limit: usize) -> bool {
        let Some(pane) = self.panes.get_mut(&pane_id) else {
            return true;
        };
        let mut ended = false;
        pane.read_pending = false;
        let mut marks: Vec<crate::shellmarks::Mark> = Vec::new();
        let mut buf = [0u8; 16 * 1024];
        let mut total = 0;
        loop {
            stage_count!(Read);
            match pane.master.read(&mut buf) {
                // End of output: the process side closed. The exit notice does the closing.
                Ok(0) => {
                    ended = true;
                    break;
                }
                Ok(n) => {
                    stage_mark!(DaemonPtyRead);
                    pane.term.feed(&buf[..n]);
                    pane.command.scanner.feed(&buf[..n], &mut marks);
                    stage_mark!(DaemonFed);
                    pane.dirty = true;
                    pane.rate.add(n, Instant::now());
                    // New output: whatever compression was under way starts over later.
                    pane.compressing = false;
                    pane.compress_at = Some(Instant::now() + IDLE_COMPRESS_DELAY);
                    total += n;
                    if total >= limit {
                        pane.read_pending = true;
                        break;
                    }
                }
                Err(e) if would_block(&e) => break,
                Err(e) if e.kind() == io::ErrorKind::Interrupted => {}
                // Linux reports EIO once the other side is gone.
                Err(_) => {
                    ended = true;
                    break;
                }
            }
        }
        // Answers the terminal wants to give to the program go back to it at once.
        let mut clipboard_writes = Vec::new();
        let mut signals = Vec::new();
        let mut title_changed = false;
        let mut progress_changed = false;

        if pane.term.has_events() {
            // Who has the terminal now: a title belongs to the program that wrote it. Asked
            // only when something came, since it is a system call.
            let writer = pane.writer_group(Instant::now());
            let mut replies = Vec::new();
            for event in pane.term.drain_events() {
                match event {
                    TermEvent::Reply(bytes) => replies.extend_from_slice(&bytes),
                    TermEvent::ClipboardWrite(data) => clipboard_writes.push(data),
                    TermEvent::Title(title) => {
                        // A title is the program's own words: it is kept no longer than it is
                        // ever shown.
                        let title: String = title.chars().take(frame::TITLE_LIMIT).collect();
                        let title = Some(title).filter(|t| !t.is_empty());
                        title_changed |= title != pane.title;
                        pane.title = title;
                        pane.title_owner = writer;
                    }
                    TermEvent::TitlePush => {
                        pane.title_stack.push(TitleSave {
                            title: pane.title.clone(),
                            owner: pane.title_owner,
                            by: writer,
                        });
                        if pane.title_stack.len() > TITLE_STACK_DEPTH {
                            pane.title_stack.remove(0);
                        }
                    }
                    TermEvent::TitlePop => {
                        if let Some(saved) = pane.title_stack.pop() {
                            pane.title = saved.title;
                            pane.title_owner = saved.owner;
                        }
                    }
                    TermEvent::Progress(report) => {
                        pane.progress = Some(report).filter(|r| r.state != 0);
                        pane.progress_at = pane.progress.map(|_| Instant::now());
                        // Asked afresh: the answer kept for titles may name a program that has
                        // just ended, and the report would be taken away at once.
                        pane.progress_owner = pane
                            .progress
                            .and_then(|_| pty::foreground_group(pane.master.as_raw_fd()))
                            .filter(|g| *g != pane.pid);
                        pane.progress_seq += 1;
                        progress_changed = true;
                    }
                    TermEvent::Bell => signals.push(crate::notify::Signal::new(
                        crate::notify::Class::Info,
                        crate::notify::TERMINAL_SOURCE,
                        "",
                        "",
                    )),
                    TermEvent::Notification { title, body } => {
                        signals.push(crate::notify::Signal::new(
                            crate::notify::Class::Info,
                            crate::notify::TERMINAL_SOURCE,
                            &title,
                            &body,
                        ));
                    }
                }
            }
            if !replies.is_empty() {
                queue_pane_input(pane, &replies);
            }
        }
        // A tab that shows the title of the program has to be drawn again; only the bar is.
        if title_changed && self.config.tab_format.uses(crate::format::Field::Title) {
            for s in self.sessions.values_mut() {
                if s.windows.iter().any(|w| w.focus == pane_id) {
                    s.chrome_dirty = true;
                }
            }
        }
        // The tab of the window shows the report, so the bar is drawn again at once; waiting for
        // some other change of the bar left it out of date until the next one.
        if progress_changed {
            self.bar_changed(pane_id);
        }
        // Only the latest write counts, and it waits until the output has been passed on.
        if let Some(data) = clipboard_writes.pop() {
            self.pending_clipboard.insert(pane_id, data);
        }
        for signal in signals {
            self.signal_pane(pane_id, signal, notices::Channel::Terminal);
        }
        for mark in marks {
            self.command_mark(pane_id, mark);
        }
        ended
    }

    /// Start or stop the process that keeps the machine awake, to match the settings. A failure
    /// is a line in the log and nothing else.
    pub(super) fn sync_sleep_guard(&mut self) {
        let wanted = self.config.prevent_sleep;
        // One that was asked to end and has not been heard of since makes way for a new one.
        if wanted
            && self
                .sleep_guard
                .as_ref()
                .is_some_and(|(_, stopping)| *stopping)
        {
            self.sleep_guard = None;
        }
        match (&mut self.sleep_guard, wanted) {
            (None, true) => {
                let tx = self.exits_tx.clone();
                let waker = self.waker.clone();
                match crate::power::start(move |pid, how| {
                    let _ = tx.send((SLEEP_GUARD_EXIT, format!("{pid} {how}")));
                    let _ = waker.wake();
                }) {
                    Ok(guard) => {
                        self.log.line(&format!(
                            "keeping the machine awake, process {}",
                            guard.pid()
                        ));
                        self.sleep_guard = Some((guard, false));
                    }
                    Err(why) => self
                        .log
                        .line(&format!("the machine is not kept awake: {why}")),
                }
            }
            (Some((guard, stopping)), false) if !*stopping => {
                guard.stop();
                *stopping = true;
                self.log.line("no longer keeping the machine awake");
            }
            _ => {}
        }
    }

    /// The sleep guard's process ended: if it was not asked to, say so once and carry on.
    fn sleep_guard_exited(&mut self, note: &str) {
        let (pid, how) = note.split_once(' ').unwrap_or((note, ""));
        let Some((guard, stopping)) = &self.sleep_guard else {
            return;
        };
        if guard.pid().to_string() != pid {
            return;
        }
        let asked = *stopping;
        self.sleep_guard = None;
        if !asked {
            self.log.line(&format!(
                "the process that keeps the machine awake ended ({how}); it stays ended until the configuration is loaded again"
            ));
        }
    }

    /// Start the thread that tells the loop when a pane's process has ended. It only waits and
    /// wakes the loop; the loop takes the exit status.
    fn watch_exit(&mut self, pane_id: u64, pid: i32) {
        self.watched.insert(pane_id, pid);
        let tx = self.exits_tx.clone();
        let waker = self.waker.clone();
        std::thread::spawn(move || {
            let note = match pty::wait_ended(pid) {
                Ok(()) => String::new(),
                Err(e) => format!("[lost: {e}]"),
            };
            let _ = tx.send((pane_id, note));
            let _ = waker.wake();
        });
    }

    fn collect_exits(&mut self) {
        while let Ok((pane_id, note)) = self.exits_rx.try_recv() {
            if pane_id == SLEEP_GUARD_EXIT {
                self.sleep_guard_exited(&note);
                continue;
            }
            let watched = self.watched.remove(&pane_id);
            let Some(pane) = self.panes.get_mut(&pane_id) else {
                // The pane was closed before its process ended: nobody wants the status, but
                // the process must not stay behind as a zombie.
                if let Some(pid) = watched {
                    let _ = pty::reap(pid);
                }
                continue;
            };
            // An empty note means the wait ended well: the status is taken here.
            let note = if note.is_empty() {
                match pty::reap(pane.pid) {
                    Some(status) => describe_exit(status),
                    None => "[lost: no exit status]".to_string(),
                }
            } else {
                note
            };
            pane.exit_note = Some(note);
            self.exited.push(pane_id);
        }
    }

    /// Close the panes whose process ended, after reading the output they left behind.
    fn finish_exited_panes(&mut self) {
        self.collect_exits();
        // Cancel string sequences that stayed unfinished for too long.
        let now = Instant::now();
        for p in self.panes.values_mut() {
            if p.term.expire_stuck(now) {
                p.dirty = true;
            }
        }
        self.expire_key_states(now);
        self.release_bells(now);
        // A lone Esc that waited long enough is the Esc key.
        let waiting: Vec<u64> = self
            .sessions
            .iter()
            .filter(|(_, s)| s.keys.deadline().is_some_and(|t| now >= t))
            .map(|(id, _)| *id)
            .collect();
        for session_id in waiting {
            let inputs = match self.sessions.get_mut(&session_id) {
                Some(s) => s.keys.expire(now),
                None => continue,
            };
            self.dispatch_inputs(session_id, inputs);
        }
        let exited = std::mem::take(&mut self.exited);
        for pane_id in exited {
            // What the process wrote just before it ended is still in the terminal's buffer, or
            // on its way to it. Read until the master says the output is over, which it does
            // as soon as the process and everything it started have let go of the other end, so
            // the usual exit does not wait at all. A process left behind that holds the
            // terminal open silently ends the wait at the deadline.
            //
            // First everything that is there to read now, with the program's end of the terminal
            // still open: that is what keeps the last output from being thrown away on macOS.
            // Then that end is closed, so that the master reports the end.
            while self.panes.contains_key(&pane_id) {
                self.read_pane(pane_id, PANE_READ_LIMIT);
                if self.panes.get(&pane_id).is_none_or(|p| !p.read_pending) {
                    break;
                }
            }
            if let Some(pane) = self.panes.get_mut(&pane_id) {
                pane.slave = None;
            }
            let deadline = Instant::now() + EXIT_OUTPUT_DEADLINE;
            while let Some(pane) = self.panes.get(&pane_id) {
                let left = deadline.saturating_duration_since(Instant::now());
                if left.is_zero() {
                    break;
                }
                if !pane.read_pending {
                    let millis = i32::try_from(left.as_millis()).unwrap_or(i32::MAX).max(1);
                    pty::wait_readable(pane.master.as_raw_fd(), millis);
                }
                if self.read_pane(pane_id, PANE_READ_LIMIT) {
                    break;
                }
            }
            self.close_pane(pane_id);
        }
    }

    /// Stop watching a pane and hang up what runs in it. The pane's window must already have let
    /// go of it.
    fn drop_pane(&mut self, pane_id: u64) {
        self.notices.forget(pane_id);
        if let Some(pane) = self.panes.remove(&pane_id) {
            let fd = pane.master.as_raw_fd();
            let _ = self.poll.registry().deregister(&mut SourceFd(&fd));
            self.targets.remove(&pane.token.0);
            // The first process is gone; hang up anything it left running in the same group.
            pty::signal_group(pane.pid, libc::SIGHUP);
        }
    }

    // ---- Idle work ----

    fn compress_idle_panes(&mut self) {
        let now = Instant::now();
        for pane in self.panes.values_mut() {
            if pane.compressing {
                if pane.term.compress_step() == arcade_term::CompressProgress::Done {
                    pane.compressing = false;
                }
            } else if pane.compress_at.is_some_and(|t| now >= t) {
                pane.compress_at = None;
                pane.compressing = true;
            }
        }
    }

    // ---- Shutdown ----

    fn shutdown_cleanup(&mut self) {
        if let Some((guard, _)) = self.sleep_guard.take() {
            guard.stop();
        }
        for p in self.panes.values() {
            pty::signal_group(p.pid, libc::SIGKILL);
        }
        let _ = std::fs::remove_file(&self.socket_path);
    }
}

fn describe_exit(status: std::process::ExitStatus) -> String {
    use std::os::unix::process::ExitStatusExt;
    match (status.code(), status.signal()) {
        (Some(0), _) => "exited".to_string(),
        (Some(code), _) => format!("exited with status {code}"),
        (None, Some(signal)) => format!("killed by signal {signal}"),
        _ => "exited".to_string(),
    }
}

fn default_session_name(cwd: &str) -> String {
    Path::new(cwd)
        .file_name()
        .and_then(|n| n.to_str())
        .filter(|n| !n.is_empty())
        .unwrap_or("0")
        .to_string()
}

/// What a window of a new pane is called before the first look at the pane: the program it
/// starts (the first word of the command, else the user's shell).
fn first_program_name(command: &[String], env: &[String]) -> String {
    let program = match command.first() {
        Some(program) => program.clone(),
        None => env
            .iter()
            .find_map(|e| e.strip_prefix("SHELL=").map(str::to_string))
            .filter(|s| !s.is_empty())
            .unwrap_or_else(|| "sh".to_string()),
    };
    program
        .rsplit('/')
        .next()
        .unwrap_or(&program)
        .trim_start_matches('-')
        .to_string()
}

/// The longest name a window takes from the title of its program, in characters.
const NAME_FROM_TITLE_LIMIT: usize = 32;

/// How a directory is called in a window's name: its last part, `~` for the home directory.
fn dir_label(cwd: &str) -> String {
    if std::env::var("HOME").is_ok_and(|home| {
        !home.is_empty() && home.trim_end_matches('/') == cwd.trim_end_matches('/') && cwd != "/"
    }) {
        return "~".to_string();
    }
    directory_name(cwd)
}

/// What a window made now is called, by what the configuration says the name comes from. The
/// directory is the one the pane was started in; the program is what it was started with.
fn name_at_creation(from: crate::config::NameFrom, cwd: &str, program: String) -> String {
    match from {
        crate::config::NameFrom::Program => program,
        // No program has set a title yet.
        crate::config::NameFrom::Directory | crate::config::NameFrom::Title => dir_label(cwd),
    }
}

/// What a window is called by its focused pane, as it is now. `fallback_cwd` is used when the
/// pane's own directory cannot be read.
fn name_of_pane(
    panes: &HashMap<u64, Pane>,
    from: crate::config::NameFrom,
    pane: u64,
    fallback_cwd: &str,
) -> String {
    use crate::config::NameFrom;
    let Some(p) = panes.get(&pane) else {
        return dir_label(fallback_cwd);
    };
    let directory = || pty::process_cwd(p.pid).map(|c| dir_label(&c));
    let program = || pty::foreground_program(std::os::fd::AsRawFd::as_raw_fd(&p.master), p.pid);
    let title = || {
        let t: String = p
            .title
            .as_deref()
            .unwrap_or_default()
            .chars()
            .filter(|c| !c.is_control())
            .take(NAME_FROM_TITLE_LIMIT)
            .collect();
        let t = t.trim().to_string();
        (!t.is_empty()).then_some(t)
    };
    match from {
        // The directory the window was made in stands in for one that cannot be read; the
        // program only when there is none at all.
        NameFrom::Directory => directory()
            .or_else(|| (!fallback_cwd.is_empty()).then(|| dir_label(fallback_cwd)))
            .or_else(program),
        NameFrom::Program => program().or_else(directory),
        NameFrom::Title => title().or_else(directory).or_else(program),
    }
    .unwrap_or_else(|| dir_label(fallback_cwd))
}

/// The last part of a directory path, or the whole path for the root.
fn directory_name(cwd: &str) -> String {
    Path::new(cwd)
        .file_name()
        .and_then(|n| n.to_str())
        .filter(|n| !n.is_empty())
        .unwrap_or("/")
        .to_string()
}

fn queue_pane_input(pane: &mut Pane, bytes: &[u8]) {
    if pane.input.len() + bytes.len() > PANE_INPUT_LIMIT {
        // The program is not reading; dropping further keys beats growing without bound.
        return;
    }
    if let Some(capture) = pane.capture.as_mut() {
        capture.record(crate::capture::Direction::Pane, bytes);
    }
    pane.input.extend_from_slice(bytes);
    stage_mark!(DaemonKeyHandled);
    write_pending(pane);
    stage_mark!(DaemonPtyWritten);
}

fn write_pending(pane: &mut Pane) {
    let mut written = 0;
    while written < pane.input.len() {
        stage_count!(Write);
        match pane.master.write(&pane.input[written..]) {
            Ok(0) => break,
            Ok(n) => written += n,
            Err(e) if would_block(&e) => break,
            Err(e) if e.kind() == io::ErrorKind::Interrupted => {}
            Err(_) => {
                // The terminal is gone; the exit notice will close the pane.
                written = pane.input.len();
            }
        }
    }
    pane.input.drain(..written);
}
