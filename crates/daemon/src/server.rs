//! The daemon: one thread, one event loop.
//!
//! Each turn of the loop does, in this order: (1) read what clients sent and write keyboard
//! bytes straight to the panes' terminals, (2) read at most 64 KiB from each pane that has output
//! and give it to the terminal emulation, (3) handle panes whose process ended, (4) build and
//! send a screen update for each attached client that can take one. The loop sleeps in the
//! operating system's event queue until something happens or a concrete deadline arrives; there
//! is no periodic timer.

use std::collections::{HashMap, HashSet};
use std::fs::File;
use std::io::{self, Read, Write};
use std::os::fd::AsRawFd;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::sync::mpsc::{self, Receiver, Sender};
use std::time::{Duration, Instant};

use arcade_proto::{
    AttachRequest, ClientMessage, CreateMode, Decoder, DetachReason, PROTOCOL_VERSION,
    ServerMessage,
};
use arcade_term::{Event as TermEvent, Term, TermConfig};
use mio::net::{UnixListener, UnixStream};
use mio::unix::SourceFd;
use mio::{Events, Interest, Poll, Token, Waker};

use crate::log::Log;
use crate::paths::prepare_socket_dir;
use crate::pty::{self, SpawnSpec};
use crate::render::{HostModes, Renderer};

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
        }
    }
}

struct Pane {
    id: u64,
    token: Token,
    master: File,
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
}

struct Session {
    id: u64,
    name: String,
    pane: Pane,
    client: Option<u64>,
    /// Ordering of use, for "the session used last".
    last_used: u64,
    /// The next update must redraw the whole screen.
    need_full: bool,
    /// What the attached client's terminal shows.
    renderer: Renderer,
    /// When the last update was sent to the attached client.
    last_frame: Option<Instant>,
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
    use_counter: u64,
    targets: HashMap<usize, Target>,
    clients: HashMap<u64, Client>,
    sessions: HashMap<u64, Session>,
    exits_rx: Receiver<(u64, String)>,
    exits_tx: Sender<(u64, String)>,
    waker: Arc<Waker>,
    /// Panes whose process ended, waiting to be drained and closed.
    exited: Vec<u64>,
    stopping: Option<Instant>,
    killed_hard: bool,
    ever_had_session: bool,
    empty_since: Option<Instant>,
    started: Instant,
    log: Log,
}

/// Run the daemon until its last session closes (or it is told to stop). Removes the socket
/// file on the way out.
pub fn run(config: DaemonConfig) -> io::Result<()> {
    prepare_socket_dir(&config.socket_path)?;
    claim_socket_path(&config.socket_path)?;
    let mut listener = UnixListener::bind(&config.socket_path)?;
    std::fs::set_permissions(
        &config.socket_path,
        std::os::unix::fs::PermissionsExt::from_mode(0o600),
    )?;
    let poll = Poll::new()?;
    poll.registry()
        .register(&mut listener, LISTENER, Interest::READABLE)?;
    let waker = Arc::new(Waker::new(poll.registry(), WAKER)?);
    let (exits_tx, exits_rx) = mpsc::channel();
    let mut server = Server {
        poll,
        listener,
        socket_path: config.socket_path.clone(),
        scrollback_lines: config.scrollback_lines,
        next_token: FIRST_DYNAMIC_TOKEN,
        next_session: 1,
        use_counter: 0,
        targets: HashMap::new(),
        clients: HashMap::new(),
        sessions: HashMap::new(),
        exits_rx,
        exits_tx,
        waker,
        exited: Vec::new(),
        stopping: None,
        killed_hard: false,
        ever_had_session: false,
        empty_since: None,
        started: Instant::now(),
        log: config
            .log_path
            .as_deref()
            .map_or_else(Log::disabled, |path| {
                Log::open(path, config.log_limit_bytes)
            }),
    };
    server.log.line(&format!(
        "daemon started, process {}, protocol {}",
        std::process::id(),
        arcade_proto::PROTOCOL_VERSION
    ));
    let result = server.event_loop();
    match &result {
        Ok(()) => server.log.line("daemon stopped"),
        Err(e) => server.log.line(&format!("daemon failed: {:?}", e.kind())),
    }
    server.shutdown_cleanup();
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
                || self.sessions.values().any(|s| s.pane.read_pending)
                || !self.exited.is_empty()
                || self.sessions.values().any(|s| s.pane.compressing)
                || self.needs_frame();
            let timeout = if busy {
                Some(Duration::ZERO)
            } else {
                self.next_deadline()
            };
            match self.poll.poll(&mut events, timeout) {
                Ok(()) => {}
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
            pane_ready.extend(
                self.sessions
                    .values()
                    .filter(|s| s.pane.read_pending)
                    .map(|s| s.pane.id),
            );

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
            for id in pane_ready.drain() {
                self.read_pane(id, PANE_READ_LIMIT);
            }
            // (3) Processes that ended.
            self.finish_exited_panes();
            // (4) Screen updates.
            self.send_frames();
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

    /// A client whose pane still has a lot of unwritten input is not read from for now.
    fn client_blocked(&self, client_id: u64) -> bool {
        self.sessions
            .values()
            .any(|s| s.client == Some(client_id) && s.pane.input.len() >= PANE_INPUT_SOFT_LIMIT)
    }

    /// When the next screen update for this session may be sent: `None` when there is nothing
    /// to send or the client is still busy taking the last one.
    fn frame_time(&self, s: &Session, now: Instant) -> Option<Instant> {
        let ready = s
            .client
            .and_then(|c| self.clients.get(&c))
            .is_some_and(|c| c.out.is_empty() && !c.gone);
        if !(s.pane.dirty || s.need_full) || !ready {
            return None;
        }
        // The 60 per second limit is for floods. A light update (an echoed key) goes out at
        // once, so typing is not delayed by the pacing of the update before it.
        if s.pane.rate.is_light() {
            return Some(now);
        }
        Some(s.last_frame.map_or(now, |t| t + FRAME_INTERVAL))
    }

    fn needs_frame(&self) -> bool {
        let now = Instant::now();
        self.sessions
            .values()
            .any(|s| self.frame_time(s, now).is_some_and(|t| t <= now))
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
        for s in self.sessions.values() {
            if let Some(t) = s.pane.term.stuck_deadline() {
                consider(t);
            }
            if let Some(t) = self.frame_time(s, now) {
                consider(t);
            }
            if let Some(t) = s.pane.compress_at {
                consider(t);
            }
        }
        deadline.map(|t| t.saturating_duration_since(now))
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
            for s in self.sessions.values() {
                pty::signal_group(s.pane.pid, libc::SIGKILL);
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
            match client.stream.write(&client.out[client.out_pos..]) {
                Ok(0) => client.gone = true,
                Ok(n) => client.out_pos += n,
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
            loop {
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
                    }
                    Err(e) if would_block(&e) => break,
                    Err(e) if e.kind() == io::ErrorKind::Interrupted => {}
                    Err(_) => {
                        client.gone = true;
                        break;
                    }
                }
            }
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
            ClientMessage::Command { name, args }
                if state == ClientState::Mismatched && name == "server.kill" =>
            {
                let (ok, output) = self.run_command(&name, &args);
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
                    self.write_to_pane(session, &bytes);
                }
            }
            ClientMessage::Resize { cols, rows } => {
                if let ClientState::Attached(session) = state {
                    self.resize_session(session, cols, rows);
                }
            }
            ClientMessage::Detach => {
                if let ClientState::Attached(session) = state {
                    self.detach_client(session, DetachReason::Requested, "detached");
                }
            }
            ClientMessage::Command { name, args } => {
                let (ok, output) = self.run_command(&name, &args);
                self.send(id, &ServerMessage::CommandResult { ok, output });
                if let Some(c) = self.clients.get_mut(&id)
                    && !matches!(c.state, ClientState::Attached(_))
                {
                    c.close_after_flush = true;
                }
            }
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
                for s in self.sessions.values_mut() {
                    if s.client == Some(id) {
                        s.client = None;
                    }
                }
            }
        }
    }

    // ---- Sessions ----

    fn attach(&mut self, id: u64, request: AttachRequest) {
        let (cols, rows) = (request.cols.max(1), request.rows.max(1));
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
        self.use_counter += 1;
        let name = {
            let Some(session) = self.sessions.get_mut(&session_id) else {
                return;
            };
            session.client = Some(id);
            session.need_full = true;
            session.last_used = self.use_counter;
            session.pane.dirty = true;
            session.name.clone()
        };
        if let Some(c) = self.clients.get_mut(&id) {
            c.state = ClientState::Attached(session_id);
        }
        self.send(id, &ServerMessage::Attached { session: name });
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
        let session_id = self.next_session;
        self.next_session += 1;
        let token = self.alloc_token();
        let pane_id = token.0 as u64;

        let mut env: Vec<String> = request
            .env
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

        let spawned = pty::spawn(&SpawnSpec {
            argv: request.command.clone(),
            cwd: request.cwd.clone(),
            env,
            cols: request.cols.max(1),
            rows: request.rows.max(1),
        })
        .map_err(|e| e.to_string())?;

        let mut config = TermConfig::new(request.cols.max(1), request.rows.max(1));
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
        let tx = self.exits_tx.clone();
        let waker = self.waker.clone();
        let mut child = spawned.child;
        std::thread::spawn(move || {
            let note = match child.wait() {
                Ok(status) => describe_exit(status),
                Err(e) => format!("[lost: {e}]"),
            };
            let _ = tx.send((pane_id, note));
            let _ = waker.wake();
        });

        self.log.line(&format!(
            "session {session_id} created, pane process {}",
            spawned.pid
        ));
        self.use_counter += 1;
        self.ever_had_session = true;
        self.sessions.insert(
            session_id,
            Session {
                id: session_id,
                name,
                pane: Pane {
                    id: pane_id,
                    token,
                    master: spawned.master,
                    term,
                    pid: spawned.pid,
                    input: Vec::new(),
                    read_pending: true,
                    dirty: true,
                    rate: OutputRate::default(),
                    exit_note: None,
                    compress_at: None,
                    compressing: false,
                },
                client: None,
                last_used: self.use_counter,
                need_full: true,
                renderer: Renderer::new(),
                last_frame: None,
            },
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

    fn detach_client(&mut self, session_id: u64, reason: DetachReason, message: &str) {
        let Some(client_id) = self
            .sessions
            .get_mut(&session_id)
            .and_then(|s| s.client.take())
        else {
            return;
        };
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

    fn resize_session(&mut self, session_id: u64, cols: u16, rows: u16) {
        let (cols, rows) = (cols.max(1), rows.max(1));
        let Some(session) = self.sessions.get_mut(&session_id) else {
            return;
        };
        let pane = &mut session.pane;
        if pane.term.cols() != cols || pane.term.rows() != rows {
            let _ = pty::set_window_size(&pane.master, cols, rows);
            if pane.term.resize(cols, rows).is_err() {
                pane.exit_note = Some("[terminal error]".into());
            }
        }
        // The client's screen changes shape, so it needs everything again.
        session.need_full = true;
        pane.dirty = true;
    }

    // ---- Panes ----

    fn write_to_pane(&mut self, session_id: u64, bytes: &[u8]) {
        let Some(session) = self.sessions.get_mut(&session_id) else {
            return;
        };
        queue_pane_input(&mut session.pane, bytes);
    }

    fn flush_pane_input(&mut self, pane_id: u64) {
        let mut unblocked = None;
        if let Some(session) = self.sessions.values_mut().find(|s| s.pane.id == pane_id) {
            write_pending(&mut session.pane);
            if session.pane.input.len() < PANE_INPUT_SOFT_LIMIT {
                unblocked = session.client;
            }
        }
        // The client was not being read; now it can be.
        if let Some(c) = unblocked.and_then(|id| self.clients.get_mut(&id)) {
            c.read_pending = true;
        }
    }

    /// Read up to `limit` bytes of output from a pane and give them to its terminal.
    fn read_pane(&mut self, pane_id: u64, limit: usize) {
        let Some(session) = self.sessions.values_mut().find(|s| s.pane.id == pane_id) else {
            return;
        };
        let pane = &mut session.pane;
        pane.read_pending = false;
        let mut buf = [0u8; 16 * 1024];
        let mut total = 0;
        loop {
            match pane.master.read(&mut buf) {
                // End of output: the process side closed. The exit notice does the closing.
                Ok(0) => break,
                Ok(n) => {
                    pane.term.feed(&buf[..n]);
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
                Err(_) => break,
            }
        }
        // Answers the terminal wants to give to the program go back to it at once.
        if pane.term.has_events() {
            let mut replies = Vec::new();
            for event in pane.term.drain_events() {
                if let TermEvent::Reply(bytes) = event {
                    replies.extend_from_slice(&bytes);
                }
            }
            if !replies.is_empty() {
                queue_pane_input(pane, &replies);
            }
        }
    }

    fn collect_exits(&mut self) {
        while let Ok((pane_id, note)) = self.exits_rx.try_recv() {
            if let Some(session) = self.sessions.values_mut().find(|s| s.pane.id == pane_id) {
                session.pane.exit_note = Some(note);
                self.exited.push(pane_id);
            }
        }
    }

    /// Close the panes whose process ended, after reading the output they left behind.
    fn finish_exited_panes(&mut self) {
        self.collect_exits();
        // Cancel string sequences that stayed unfinished for too long.
        let now = Instant::now();
        for s in self.sessions.values_mut() {
            if s.pane.term.expire_stuck(now) {
                s.pane.dirty = true;
            }
        }
        let exited = std::mem::take(&mut self.exited);
        for pane_id in exited {
            // What the process wrote just before it ended is still in the terminal's buffer.
            // Bounded, so a background job that keeps the terminal open cannot hold us here.
            for _ in 0..16 {
                self.read_pane(pane_id, PANE_READ_LIMIT);
                let more = self
                    .sessions
                    .values()
                    .find(|s| s.pane.id == pane_id)
                    .is_some_and(|s| s.pane.read_pending);
                if !more {
                    break;
                }
            }
            self.close_pane(pane_id);
        }
    }

    fn close_pane(&mut self, pane_id: u64) {
        let Some(session_id) = self
            .sessions
            .values()
            .find(|s| s.pane.id == pane_id)
            .map(|s| s.id)
        else {
            return;
        };
        // Show the final screen before saying goodbye.
        self.build_frame_for(session_id);
        let Some(session) = self.sessions.remove(&session_id) else {
            return;
        };
        let fd = session.pane.master.as_raw_fd();
        let _ = self.poll.registry().deregister(&mut SourceFd(&fd));
        self.targets.remove(&session.pane.token.0);
        // The first process is gone; hang up anything it left running in the same group.
        pty::signal_group(session.pane.pid, libc::SIGHUP);
        let note = session.pane.exit_note.clone().unwrap_or_default();
        self.log.line(&format!(
            "session {} closed: {}",
            session.id,
            note.trim_start_matches('[').trim_end_matches(']')
        ));
        if let Some(client_id) = session.client {
            self.send(
                client_id,
                &ServerMessage::SessionEnded {
                    message: format!(
                        "[{}: {}]",
                        session.name,
                        note.trim_start_matches('[').trim_end_matches(']')
                    ),
                },
            );
            if let Some(c) = self.clients.get_mut(&client_id) {
                c.state = ClientState::Ready;
                c.close_after_flush = true;
            }
        }
    }

    // ---- Idle work ----

    fn compress_idle_panes(&mut self) {
        let now = Instant::now();
        for session in self.sessions.values_mut() {
            let pane = &mut session.pane;
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

    // ---- Screen updates ----

    fn send_frames(&mut self) {
        let now = Instant::now();
        let due: Vec<u64> = self
            .sessions
            .values()
            .filter(|s| self.frame_time(s, now).is_some_and(|t| t <= now))
            .map(|s| s.id)
            .collect();
        for id in due {
            self.build_frame_for(id);
        }
    }

    /// Build the screen update for a session's client and queue it.
    fn build_frame_for(&mut self, session_id: u64) {
        let Some(session) = self.sessions.get_mut(&session_id) else {
            return;
        };
        let Some(client_id) = session.client else {
            return;
        };
        if !session.pane.dirty && !session.need_full {
            return;
        }
        let force_full = session.need_full;
        if force_full {
            // The client's screen is new or changed shape: nothing of it can be trusted.
            session.renderer.reset();
        }
        let frame = session.pane.term.snapshot(force_full);
        session.pane.dirty = false;
        session.need_full = false;
        match frame {
            Ok(frame) => {
                let modes = HostModes {
                    app_cursor_keys: session
                        .pane
                        .term
                        .mode(arcade_term::Mode::AppCursorKeys)
                        .unwrap_or(false),
                    bracketed_paste: session
                        .pane
                        .term
                        .mode(arcade_term::Mode::BracketedPaste)
                        .unwrap_or(false),
                };
                let bytes = session.renderer.render(&frame, modes);
                if !bytes.is_empty() {
                    session.last_frame = Some(Instant::now());
                    self.send_output(client_id, &bytes);
                }
            }
            Err(e) => {
                session.pane.exit_note = Some(format!("[{e}]"));
                let pane_id = session.pane.id;
                pty::signal_group(session.pane.pid, libc::SIGKILL);
                self.exited.push(pane_id);
            }
        }
    }

    // ---- Commands ----

    fn run_command(&mut self, name: &str, args: &[String]) -> (bool, String) {
        match name {
            "server.ping" => (true, "pong".to_string()),
            "session.list" => {
                let mut sessions: Vec<&Session> = self.sessions.values().collect();
                sessions.sort_by_key(|s| s.id);
                let mut text = String::new();
                for s in sessions {
                    text.push_str(&format!(
                        "{}\t{}\t{}\n",
                        s.name,
                        s.id,
                        u8::from(s.client.is_some())
                    ));
                }
                (true, text)
            }
            "session.detach" => {
                // Optional `--pane <id>`: the pane the command is run from.
                let pane_id = args
                    .windows(2)
                    .find(|w| w[0] == "--pane")
                    .and_then(|w| w[1].parse::<u64>().ok());
                let target = self
                    .sessions
                    .values()
                    .find(|s| pane_id.is_some_and(|p| s.pane.id == p))
                    .or_else(|| {
                        if pane_id.is_none() {
                            self.sessions.values().find(|s| s.client.is_some())
                        } else {
                            None
                        }
                    })
                    .map(|s| s.id);
                match target {
                    Some(session_id) => {
                        self.detach_client(session_id, DetachReason::Requested, "detached");
                        (true, String::new())
                    }
                    None => (false, "no such pane".to_string()),
                }
            }
            "session.close" => {
                let name = args.first();
                let target = self
                    .sessions
                    .values()
                    .find(|s| name.is_none_or(|n| &s.name == n))
                    .map(|s| (s.id, s.pane.pid, s.pane.id));
                match target {
                    Some((session_id, pid, pane_id)) => {
                        if let Some(s) = self.sessions.get_mut(&session_id) {
                            s.pane.exit_note = Some("closed".to_string());
                        }
                        pty::signal_group(pid, libc::SIGHUP);
                        // Closing does not wait for the process; the pane is gone now.
                        self.close_pane(pane_id);
                        (true, String::new())
                    }
                    None => (false, "no such session".to_string()),
                }
            }
            "server.kill" => {
                self.stopping = Some(Instant::now() + KILL_GRACE);
                for s in self.sessions.values() {
                    pty::signal_group(s.pane.pid, libc::SIGHUP);
                }
                let ids: Vec<u64> = self
                    .sessions
                    .values()
                    .filter(|s| s.client.is_some())
                    .map(|s| s.id)
                    .collect();
                for id in ids {
                    self.detach_client(id, DetachReason::ServerStopping, "server stopping");
                }
                (true, String::new())
            }
            other => (false, format!("unknown command: {other}")),
        }
    }

    // ---- Shutdown ----

    fn shutdown_cleanup(&mut self) {
        for s in self.sessions.values() {
            pty::signal_group(s.pane.pid, libc::SIGKILL);
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

fn queue_pane_input(pane: &mut Pane, bytes: &[u8]) {
    if pane.input.len() + bytes.len() > PANE_INPUT_LIMIT {
        // The program is not reading; dropping further keys beats growing without bound.
        return;
    }
    pane.input.extend_from_slice(bytes);
    write_pending(pane);
}

fn write_pending(pane: &mut Pane) {
    let mut written = 0;
    while written < pane.input.len() {
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
