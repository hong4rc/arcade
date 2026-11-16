//! Replacing the daemon's own program without ending its sessions.
//!
//! The running daemon writes what it holds (sessions, windows, layouts, every pane's terminal)
//! into a snapshot, checks with a rehearsal that the new program can read it, and then starts the
//! new program in the same process with `exec`. The process number, the programs running in the
//! panes, their terminals' descriptors, the listening socket and the attached clients' sockets
//! all stay; the new program builds its sessions again from the snapshot.
//!
//! Going back: before the exec the daemon keeps a private copy of its own program. If the new one
//! fails at any point before it says it is ready (an error, a panic, a crash, or taking too
//! long), it starts that copy with the same snapshot and descriptors, and the old program
//! restores itself and tells the one who asked that the upgrade was undone. Both directions are
//! the same code: every version restores from its own snapshot format.
//!
//! The descriptors stay open for the exec until the new program is ready, so that going back
//! needs nothing but the exec.

use std::collections::HashSet;
use std::ffi::CString;
use std::fs::File;
use std::io::{self, Read, Write};
use std::os::fd::{AsRawFd, FromRawFd, OwnedFd};
use std::os::unix::fs::{MetadataExt, OpenOptionsExt};
use std::os::unix::net::UnixListener as StdListener;
use std::os::unix::process::CommandExt;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::sync::OnceLock;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::{Duration, Instant};

use arcade_proto::{Decoder, DetachReason, ServerMessage};
use arcade_term::{Cursor, CursorShape, Term, TermConfig, TerminalState};
use mio::net::{UnixListener, UnixStream};
use mio::unix::SourceFd;
use mio::{Interest, Token};

use super::*;
use crate::layout::Layout;
use crate::notify::{Class, Notification};
use crate::snapshot::{
    ClientRecord, NoticeRecord, PaneRecord, SessionRecord, Snapshot, SnapshotError, TerminalRecord,
    TitleRecord, WindowRecord,
};

/// How long the last output of the panes and the last words of the clients get to arrive before
/// the snapshot is taken.
const QUIESCE_WAIT: Duration = Duration::from_millis(100);
/// The time the new program gets to read the snapshot and declare itself ready, before the old
/// one is started again; it grows with the size of the snapshot.
const READY_BASE: Duration = Duration::from_secs(10);
/// The time the rehearsal gets, with the same growth.
const REHEARSAL_BASE: Duration = Duration::from_secs(30);

/// What the program started by an upgrade is told.
#[derive(Clone, Debug)]
pub struct Resume {
    /// The descriptor that holds the snapshot.
    pub fd: i32,
    /// The saved copy of the program that ran before, to start again if this one fails.
    pub rollback: Option<PathBuf>,
    /// Set in the program started by a rollback: why the upgrade was undone.
    pub rolled_back: Option<String>,
}

/// What a rehearsal or a restore built.
#[derive(Clone, Copy, Debug, Default)]
pub struct Built {
    pub sessions: usize,
    pub windows: usize,
    pub panes: usize,
    /// Panes whose terminal could not be read and start with an empty screen.
    pub degraded: usize,
}

// ---- Going back, from any thread or from a signal handler ----

/// The program to start again and its arguments, prepared before anything can fail so that
/// starting it needs no allocation.
struct RollbackPlan {
    path: CString,
    /// Owns the text the pointers point into.
    #[allow(dead_code)]
    argv: Vec<CString>,
    pointers: Vec<*const libc::c_char>,
}

// SAFETY: the pointers point into `argv`, which lives as long as the plan and is never changed.
unsafe impl Sync for RollbackPlan {}
unsafe impl Send for RollbackPlan {}

static ROLLBACK: OnceLock<RollbackPlan> = OnceLock::new();
static ARMED: AtomicBool = AtomicBool::new(false);

/// Start the saved program in this process. Does not return unless that fails.
fn go_back() {
    if !ARMED.swap(false, Ordering::SeqCst) {
        return;
    }
    if let Some(plan) = ROLLBACK.get() {
        // SAFETY: execv with a NUL-terminated path and a NULL-terminated argument list that the
        // plan owns; it is async-signal-safe.
        unsafe {
            libc::execv(plan.path.as_ptr(), plan.pointers.as_ptr());
        }
    }
}

extern "C" fn crashed(_signal: libc::c_int) {
    go_back();
    // The exec failed or nothing was armed: die as the signal would have.
    // SAFETY: _exit ends the process at once.
    unsafe { libc::_exit(70) }
}

/// Get ready to go back: the panic hook, the crash signals and a watchdog. Returns whether it is
/// armed.
fn arm(rollback: &Path, socket: &Path, fd: i32, reason_prefix: &str, wait: Duration) -> bool {
    let to_c = |s: &std::ffi::OsStr| CString::new(s.as_encoded_bytes()).ok();
    let (Some(path), Some(socket_c)) = (to_c(rollback.as_os_str()), to_c(socket.as_os_str()))
    else {
        return false;
    };
    let mut argv = vec![
        path.clone(),
        CString::new("--socket").unwrap(),
        socket_c,
        CString::new("__daemon").unwrap(),
        CString::new("--resume").unwrap(),
        CString::new(fd.to_string()).unwrap(),
        CString::new("--rolled-back").unwrap(),
    ];
    argv.push(CString::new(reason_prefix.replace('\0', " ")).unwrap());
    let mut pointers: Vec<*const libc::c_char> = argv.iter().map(|a| a.as_ptr()).collect();
    pointers.push(std::ptr::null());
    if ROLLBACK
        .set(RollbackPlan {
            path,
            argv,
            pointers,
        })
        .is_err()
    {
        return false;
    }
    ARMED.store(true, Ordering::SeqCst);

    let previous = std::panic::take_hook();
    std::panic::set_hook(Box::new(move |info| {
        if ARMED.load(Ordering::SeqCst) {
            go_back();
        }
        previous(info);
    }));

    for signal in [
        libc::SIGSEGV,
        libc::SIGBUS,
        libc::SIGILL,
        libc::SIGABRT,
        libc::SIGFPE,
    ] {
        // SAFETY: a sigaction with a handler that only execs, on the alternate stack so that a
        // stack overflow is covered too; the stack is leaked on purpose.
        unsafe {
            let stack_size = 64 * 1024;
            let stack = libc::mmap(
                std::ptr::null_mut(),
                stack_size,
                libc::PROT_READ | libc::PROT_WRITE,
                libc::MAP_PRIVATE | libc::MAP_ANON,
                -1,
                0,
            );
            if stack != libc::MAP_FAILED {
                let alt = libc::stack_t {
                    ss_sp: stack,
                    ss_flags: 0,
                    ss_size: stack_size,
                };
                libc::sigaltstack(&alt, std::ptr::null_mut());
            }
            let mut action: libc::sigaction = std::mem::zeroed();
            action.sa_sigaction = crashed as extern "C" fn(libc::c_int) as usize;
            action.sa_flags = libc::SA_ONSTACK | libc::SA_NODEFER;
            libc::sigemptyset(&mut action.sa_mask);
            libc::sigaction(signal, &action, std::ptr::null_mut());
        }
    }

    std::thread::spawn(move || {
        std::thread::sleep(wait);
        go_back();
    });
    true
}

/// The new program is ready: nothing is armed any more.
fn disarm() {
    ARMED.store(false, Ordering::SeqCst);
    let _ = std::panic::take_hook();
    for signal in [
        libc::SIGSEGV,
        libc::SIGBUS,
        libc::SIGILL,
        libc::SIGABRT,
        libc::SIGFPE,
    ] {
        // SAFETY: back to the default action.
        unsafe {
            libc::signal(signal, libc::SIG_DFL);
        }
    }
}

// ---- The checks on the new program ----

fn check_program(path: &Path) -> Result<PathBuf, String> {
    let real = path
        .canonicalize()
        .map_err(|e| format!("{}: {e}", path.display()))?;
    let meta = std::fs::metadata(&real).map_err(|e| format!("{}: {e}", real.display()))?;
    if !meta.is_file() {
        return Err(format!("{} is not a regular file", real.display()));
    }
    if meta.mode() & 0o111 == 0 {
        return Err(format!("{} cannot be run", real.display()));
    }
    // SAFETY: geteuid has no arguments and cannot fail.
    let me = unsafe { libc::geteuid() };
    if meta.uid() != me && meta.uid() != 0 {
        return Err(format!(
            "{} belongs to another user: only a program of yours (or the system's) takes over",
            real.display()
        ));
    }
    if meta.mode() & 0o022 != 0 {
        return Err(format!(
            "{} can be written by others, so it is not trusted to take over",
            real.display()
        ));
    }
    Ok(real)
}

fn set_cloexec(fd: i32, on: bool) {
    let _ = pty::set_inheritable(fd, !on);
}

/// A private file, already unlinked, holding `bytes`; the descriptor is how it is passed on.
fn unlinked_file(dir: &Path, stem: &str, bytes: &[u8]) -> io::Result<File> {
    let path = dir.join(format!("{stem}-{}", std::process::id()));
    let _ = std::fs::remove_file(&path);
    let mut file = std::fs::OpenOptions::new()
        .read(true)
        .write(true)
        .create_new(true)
        .mode(0o600)
        .open(&path)?;
    let _ = std::fs::remove_file(&path);
    file.write_all(bytes)?;
    Ok(file)
}

fn read_whole(fd: i32) -> io::Result<Vec<u8>> {
    // SAFETY: the descriptor belongs to this process; it is only read from and not closed.
    let file = std::mem::ManuallyDrop::new(unsafe { File::from_raw_fd(fd) });
    let mut out = Vec::new();
    let mut offset = 0u64;
    let mut buf = vec![0u8; 1 << 20];
    loop {
        let n = std::os::unix::fs::FileExt::read_at(&*file, &mut buf, offset)?;
        if n == 0 {
            return Ok(out);
        }
        out.extend_from_slice(&buf[..n]);
        offset += n as u64;
    }
}

fn cursor_number(shape: CursorShape) -> u8 {
    match shape {
        CursorShape::Block => 0,
        CursorShape::Underline => 1,
        CursorShape::Bar => 2,
        CursorShape::HollowBlock => 3,
    }
}

fn cursor_shape(n: u8) -> CursorShape {
    match n {
        1 => CursorShape::Underline,
        2 => CursorShape::Bar,
        3 => CursorShape::HollowBlock,
        _ => CursorShape::Block,
    }
}

/// Whether every reference in the snapshot points at something that is in it.
fn validate(snap: &Snapshot) -> Result<(), String> {
    let mut panes = HashSet::new();
    for p in &snap.panes {
        if !panes.insert(p.id) {
            return Err(format!("pane {} appears twice", p.id));
        }
        if p.id >= snap.next_token {
            return Err(format!("pane {} is beyond the token counter", p.id));
        }
    }
    let mut used = HashSet::new();
    let mut names = HashSet::new();
    for s in &snap.sessions {
        if !names.insert(s.name.as_str()) {
            return Err(format!("session name {:?} appears twice", s.name));
        }
        if !s.windows.iter().any(|w| w.id == s.current) {
            return Err(format!("session {} has no window {}", s.id, s.current));
        }
        for w in &s.windows {
            let leaves = w.layout.leaves();
            if !leaves.contains(&w.focus) {
                return Err(format!("window {} focuses a pane it has not", w.id));
            }
            for leaf in leaves {
                if !panes.contains(&leaf) {
                    return Err(format!("window {} has the unknown pane {leaf}", w.id));
                }
                if !used.insert(leaf) {
                    return Err(format!("pane {leaf} is in two places"));
                }
            }
        }
    }
    if used.len() != panes.len() {
        return Err("a pane belongs to no window".to_string());
    }
    Ok(())
}

impl Server {
    // ---- Writing the snapshot ----

    /// Everything the new program needs. `requester` is the client waiting for the answer.
    fn snapshot(&mut self, requester: Option<u64>) -> Snapshot {
        let mut sessions: Vec<_> = self.sessions.values().collect();
        sessions.sort_by_key(|s| s.id);
        let sessions = sessions
            .into_iter()
            .map(|s| SessionRecord {
                id: s.id,
                name: s.name.clone(),
                env: s.env.clone(),
                cwd: s.cwd.clone(),
                current: s.current,
                last_window: s.last_window,
                cols: s.cols,
                rows: s.rows,
                last_used: s.last_used,
                windows: s
                    .windows
                    .iter()
                    .map(|w| {
                        let area = w.layout.area();
                        WindowRecord {
                            id: w.id,
                            number: w.number as u64,
                            name: w.name.clone(),
                            name_manual: w.name_manual,
                            focus: w.focus,
                            last_focus: w.last_focus,
                            zoomed: w.zoomed,
                            arrangement: w.arrangement.map(|a| a.to_number()),
                            layout_cols: area.w,
                            layout_rows: area.h,
                            layout: w.layout.shape(),
                        }
                    })
                    .collect(),
            })
            .collect();

        let mut ids: Vec<u64> = self.panes.keys().copied().collect();
        ids.sort_unstable();
        let mut panes = Vec::new();
        for id in ids {
            let Some(pane) = self.panes.get_mut(&id) else {
                continue;
            };
            let terminal = match pane.term.save_state() {
                Ok(TerminalState { primary, alternate }) => {
                    Some(TerminalRecord { primary, alternate })
                }
                Err(_) => None,
            };
            panes.push(PaneRecord {
                id,
                pid: pane.pid,
                master_fd: pane.master.as_raw_fd(),
                slave_fd: pane.slave.as_ref().map(|s| s.as_raw_fd()),
                cols: pane.term.cols(),
                rows: pane.term.rows(),
                input: pane.input.clone(),
                title: pane.title.clone(),
                title_owner: pane.title_owner,
                title_stack: pane
                    .title_stack
                    .iter()
                    .map(|t| TitleRecord {
                        title: t.title.clone(),
                        owner: t.owner,
                        by: t.by,
                    })
                    .collect(),
                progress: pane.progress.map(|p| (p.state, p.value)),
                exit_note: pane.exit_note.clone(),
                cursor: Some((
                    cursor_number(pane.last_cursor.shape),
                    pane.last_cursor.blinking,
                )),
                terminal,
            });
        }

        let mut clients = Vec::new();
        for (id, c) in &self.clients {
            if c.gone {
                continue;
            }
            let is_requester = requester == Some(*id);
            let attached = match c.state {
                ClientState::Attached(s) => self.sessions.get(&s),
                _ => None,
            };
            if !is_requester && attached.is_none() {
                continue;
            }
            clients.push(ClientRecord {
                id: *id,
                fd: c.stream.as_raw_fd(),
                session: attached.map(|s| s.name.clone()),
                cols: attached.map_or(0, |s| s.cols),
                rows: attached.map_or(0, |s| s.rows),
                requester: is_requester,
            });
        }
        clients.sort_by_key(|c| c.id);

        let notices = self
            .notices
            .standing()
            .into_iter()
            .map(|(pane, n)| NoticeRecord {
                pane,
                class: n.class.to_number(),
                source: n.source.clone(),
                title: n.title.clone(),
                message: n.message.clone(),
                age_ms: n.since.elapsed().as_millis() as u64,
            })
            .collect();

        Snapshot {
            program_version: env!("CARGO_PKG_VERSION").to_string(),
            program_path: self
                .program_path
                .as_deref()
                .map(|p| p.display().to_string())
                .unwrap_or_default(),
            socket_path: self.socket_path.display().to_string(),
            listener_fd: self.listener.as_raw_fd(),
            next_token: self.next_token as u64,
            next_session: self.next_session,
            next_window: self.next_window,
            use_counter: self.use_counter,
            ever_had_session: self.ever_had_session,
            clipboard: self.clipboard.buffer().map(str::to_string),
            sleep_guard_pid: self
                .sleep_guard
                .as_ref()
                .filter(|(_, stopping)| !*stopping)
                .map(|(g, _)| g.pid()),
            sessions,
            panes,
            clients,
            notices,
        }
    }

    // ---- Building from the snapshot ----

    /// Put the sessions back. `real` takes the descriptors of the snapshot over; without it
    /// (the rehearsal) nothing of the process is touched and no program is waited for.
    fn restore(&mut self, snap: &Snapshot, real: bool) -> Result<Built, String> {
        validate(snap)?;
        let mut built = Built::default();
        let max_pane = snap.panes.iter().map(|p| p.id).max().unwrap_or(0);
        self.next_token = (snap.next_token as usize).max(max_pane as usize + 1);
        self.next_session = snap.next_session;
        self.next_window = snap.next_window;
        self.use_counter = snap.use_counter;
        self.ever_had_session = snap.ever_had_session;
        self.clipboard.restore(snap.clipboard.clone());

        for p in &snap.panes {
            let mut config = TermConfig::new(p.cols.max(1), p.rows.max(1));
            config.scrollback_lines = self.scrollback_lines;
            let mut term = Term::new(config).map_err(|e| format!("pane {}: {e}", p.id))?;
            let cursor = p
                .cursor
                .map(|(shape, blinking)| (cursor_shape(shape), blinking));
            match &p.terminal {
                Some(t) => term.load_state(
                    &TerminalState {
                        primary: t.primary.clone(),
                        alternate: t.alternate.clone(),
                    },
                    cursor,
                ),
                None => built.degraded += 1,
            }
            let token = Token(p.id as usize);
            let (master, slave) = if real {
                let fd = p.master_fd;
                self.poll
                    .registry()
                    .register(
                        &mut SourceFd(&fd),
                        token,
                        Interest::READABLE | Interest::WRITABLE,
                    )
                    .map_err(|e| format!("pane {}: {e}", p.id))?;
                // SAFETY: the descriptors are the ones the snapshot says the old program left
                // open for this pane, and nothing else in this process owns them.
                unsafe {
                    (
                        File::from_raw_fd(p.master_fd),
                        p.slave_fd.map(|s| OwnedFd::from_raw_fd(s)),
                    )
                }
            } else {
                (File::open("/dev/null").map_err(|e| e.to_string())?, None)
            };
            self.targets.insert(p.id as usize, Target::Pane(p.id));
            let last_cursor = Cursor {
                x: 0,
                y: 0,
                visible: true,
                shape: cursor.map_or(CursorShape::Block, |c| c.0),
                blinking: cursor.is_some_and(|c| c.1),
            };
            self.panes.insert(
                p.id,
                Pane {
                    id: p.id,
                    token,
                    master,
                    slave,
                    term,
                    pid: p.pid,
                    input: p.input.clone(),
                    read_pending: true,
                    dirty: true,
                    rate: OutputRate::default(),
                    exit_note: p.exit_note.clone(),
                    compress_at: None,
                    compressing: false,
                    last_cursor,
                    copy: None,
                    title: p.title.clone(),
                    title_owner: p.title_owner,
                    title_stack: p
                        .title_stack
                        .iter()
                        .map(|t| TitleSave {
                            title: t.title.clone(),
                            owner: t.owner,
                            by: t.by,
                        })
                        .collect(),
                    progress: p
                        .progress
                        .map(|(state, value)| arcade_term::Progress { state, value }),
                    progress_seq: u64::from(p.progress.is_some()),
                    writer_seen: None,
                },
            );
            built.panes += 1;
            if p.exit_note.is_some() {
                // Its status was taken before the snapshot; the loop closes the pane.
                self.exited.push(p.id);
            } else if real {
                self.watch_exit(p.id, p.pid);
            }
        }

        for s in &snap.sessions {
            let windows = s
                .windows
                .iter()
                .map(|w| Window {
                    id: w.id,
                    number: w.number as usize,
                    name: w.name.clone(),
                    name_manual: w.name_manual,
                    program: String::new(),
                    directory: String::new(),
                    layout: Layout::from_shape(
                        &w.layout,
                        w.layout_cols.max(1),
                        w.layout_rows.max(1),
                    ),
                    focus: w.focus,
                    last_focus: w.last_focus,
                    zoomed: w.zoomed,
                    arrangement: w
                        .arrangement
                        .and_then(crate::layout::Arrangement::from_number),
                })
                .collect::<Vec<_>>();
            built.windows += windows.len();
            let mut session = Session::new(
                s.id,
                s.name.clone(),
                s.env.clone(),
                s.cwd.clone(),
                windows,
                s.current,
                (s.cols.max(1), s.rows.max(1)),
                s.last_used,
            );
            session.last_window = s.last_window;
            self.sessions.insert(s.id, session);
            built.sessions += 1;
        }

        for n in &snap.notices {
            let Some(class) = Class::from_number(n.class) else {
                continue;
            };
            let now = Instant::now();
            self.notices.restore(
                n.pane,
                Notification {
                    class,
                    source: n.source.clone(),
                    title: n.title.clone(),
                    message: n.message.clone(),
                    since: now
                        .checked_sub(Duration::from_millis(n.age_ms))
                        .unwrap_or(now),
                },
            );
        }

        if real && let Some(pid) = snap.sleep_guard_pid {
            let tx = self.exits_tx.clone();
            let waker = self.waker.clone();
            let guard = crate::power::adopt(pid, move |pid, how| {
                let _ = tx.send((SLEEP_GUARD_EXIT, format!("{pid} {how}")));
                let _ = waker.wake();
            });
            self.sleep_guard = Some((guard, false));
        }
        Ok(built)
    }

    /// Take over the clients that were passed on and tell them how it went. `undone` is why the
    /// upgrade was undone, when it was.
    fn settle_clients(&mut self, snap: &Snapshot, built: Built, undone: Option<&str>) {
        let program = std::env::current_exe()
            .map(|p| p.display().to_string())
            .unwrap_or_default();
        for c in &snap.clients {
            // SAFETY: the descriptor is a client's socket that the old program left open.
            let mut stream = unsafe { UnixStream::from_raw_fd(c.fd) };
            let token = self.alloc_token();
            if self
                .poll
                .registry()
                .register(&mut stream, token, Interest::READABLE | Interest::WRITABLE)
                .is_err()
            {
                continue;
            }
            let id = token.0 as u64;
            self.targets.insert(token.0, Target::Client(id));
            self.clients.insert(
                id,
                Client {
                    stream,
                    decoder: Decoder::new(),
                    out: Vec::new(),
                    out_pos: 0,
                    state: ClientState::Ready,
                    close_after_flush: false,
                    read_pending: false,
                    gone: false,
                    host_keyboard: None,
                },
            );
            let session = c
                .session
                .as_ref()
                .and_then(|name| self.sessions.values().find(|s| &s.name == name))
                .map(|s| (s.id, s.name.clone()));
            match (undone, &session) {
                // Undone: the client carries on with the session it had.
                (Some(_), Some((session_id, _))) => {
                    self.bind_client(id, *session_id, c.cols, c.rows);
                }
                // Done: the client is told to start the new program and attach again.
                (None, Some((_, name))) => {
                    self.send(
                        id,
                        &ServerMessage::Detached {
                            reason: DetachReason::Upgraded,
                            message: format!("{program}\0{name}"),
                        },
                    );
                    if let Some(client) = self.clients.get_mut(&id) {
                        client.close_after_flush = true;
                    }
                }
                _ => {}
            }
            if c.requester {
                let (ok, output) = match undone {
                    None => (
                        true,
                        format!(
                            "upgraded {} -> {}: kept {} sessions, {} windows, {} panes{}",
                            snap.program_version,
                            env!("CARGO_PKG_VERSION"),
                            built.sessions,
                            built.windows,
                            built.panes,
                            if built.degraded > 0 {
                                format!(" ({} of them with an empty screen)", built.degraded)
                            } else {
                                String::new()
                            }
                        ),
                    ),
                    Some(why) => (false, format!("the upgrade was undone: {why}")),
                };
                self.send(id, &ServerMessage::CommandResult { ok, output });
                if let Some(client) = self.clients.get_mut(&id) {
                    client.close_after_flush = true;
                }
            }
        }
    }

    // ---- The command ----

    /// Start before the exec: leave the daemon with nothing in flight.
    fn quiesce(&mut self) {
        self.collect_exits();
        let ids: Vec<u64> = self.panes.keys().copied().collect();
        let deadline = Instant::now() + QUIESCE_WAIT;
        loop {
            for id in &ids {
                if self.panes.contains_key(id) {
                    self.read_pane(*id, usize::MAX);
                }
            }
            let waiting = self
                .panes
                .values()
                .any(|p| p.term.stuck_deadline().is_some());
            if !waiting || Instant::now() >= deadline {
                break;
            }
            std::thread::sleep(Duration::from_millis(5));
        }
        for id in &ids {
            self.flush_pane_input(*id);
        }
        self.collect_exits();
        // What the clients were sent reaches them; one that cannot take it, or whose last
        // message is cut in two, is let go (it would be out of step after the exec).
        let deadline = Instant::now() + QUIESCE_WAIT;
        loop {
            let ids: Vec<u64> = self.clients.keys().copied().collect();
            for id in &ids {
                self.flush_client(*id);
            }
            let pending = self
                .clients
                .values()
                .any(|c| !c.gone && c.out_pos < c.out.len());
            if !pending || Instant::now() >= deadline {
                break;
            }
            std::thread::sleep(Duration::from_millis(2));
        }
        for c in self.clients.values_mut() {
            if c.out_pos < c.out.len() || c.decoder.buffered() > 0 {
                c.gone = true;
            }
        }
        self.reap_clients();
    }

    /// `server.upgrade`: replace this program by the one at `args[0]`. Returns only when the
    /// upgrade did not happen (the reason is the answer); on success it does not return.
    pub(super) fn upgrade(&mut self, requester: u64, args: &[String]) -> (bool, String) {
        let force = args.iter().any(|a| a == "--force");
        let Some(path) = args.first().filter(|a| !a.starts_with("--")) else {
            return (false, "no program given".to_string());
        };
        let new_program = match check_program(Path::new(path)) {
            Ok(p) => p,
            Err(e) => return (false, e),
        };
        let Some(own_path) = self.program_path.clone() else {
            return (
                false,
                "this daemon does not know where its own program is".into(),
            );
        };
        let Some(own_file) = self.program_file.as_ref().and_then(|f| f.try_clone().ok()) else {
            return (
                false,
                "this daemon cannot read its own program, so it could not go back".into(),
            );
        };
        let Some(dir) = self.socket_path.parent().map(Path::to_path_buf) else {
            return (false, "the socket has no directory".into());
        };
        if self.stopping.is_some() {
            return (false, "the daemon is stopping".into());
        }

        self.log.line(&format!(
            "upgrade to {} asked by client {requester}",
            new_program.display()
        ));
        self.quiesce();
        let snapshot = self.snapshot(Some(requester));
        let bytes = snapshot.encode();
        // A test that makes the fixtures asks to keep a copy of what the snapshot holds.
        if injected_fault(&self.socket_path, &["keep-snapshot"]).is_some() {
            let _ = std::fs::write(self.socket_path.with_extension("snapshot"), &bytes);
        }
        let snap_file = match unlinked_file(&dir, "arcade-snapshot", &bytes) {
            Ok(f) => f,
            Err(e) => return (false, format!("the snapshot could not be written: {e}")),
        };
        let snap_fd = snap_file.as_raw_fd();
        // The pane that holds a program must not be left in the middle of a read when the
        // exec comes: the loop is not running, so nothing is.
        let scale = Duration::from_millis(bytes.len() as u64 / 10_000);

        // The rehearsal: the new program builds everything from the snapshot, apart from
        // taking the real descriptors, and says whether it could.
        set_cloexec(snap_fd, false);
        let rehearsal = rehearse_with(
            &new_program,
            &self.socket_path,
            snap_fd,
            REHEARSAL_BASE + scale,
        );
        set_cloexec(snap_fd, true);
        let version = match rehearsal {
            Ok(v) => v,
            Err(e) => {
                self.log.line(&format!("upgrade refused: {e}"));
                return (false, format!("the new program cannot take over: {e}"));
            }
        };
        if !force
            && new_program == own_path.canonicalize().unwrap_or(own_path.clone())
            && version == env!("CARGO_PKG_VERSION")
        {
            return (
                false,
                format!(
                    "{} is this program and version {version}: nothing to upgrade (`--force` does it anyway)",
                    new_program.display()
                ),
            );
        }

        // The way back: this program's own file, copied where an install cannot touch it.
        let copy = dir.join(format!("arcade-previous-{}", std::process::id()));
        if let Err(e) = copy_program(&own_file, &copy) {
            let _ = std::fs::remove_file(&copy);
            return (
                false,
                format!("a copy of this program could not be kept: {e}"),
            );
        }

        // Everything the new program must find stays open across the exec.
        let mut fds = vec![snap_fd, self.listener.as_raw_fd()];
        fds.extend(
            snapshot
                .panes
                .iter()
                .flat_map(|p| [Some(p.master_fd), p.slave_fd])
                .flatten(),
        );
        fds.extend(snapshot.clients.iter().map(|c| c.fd));
        for fd in &fds {
            set_cloexec(*fd, false);
        }
        self.log.line(&format!(
            "upgrade: {} sessions, {} panes, snapshot {} bytes; starting {}",
            snapshot.sessions.len(),
            snapshot.panes.len(),
            bytes.len(),
            new_program.display()
        ));
        let error = Command::new(&new_program)
            .arg("--socket")
            .arg(&self.socket_path)
            .arg("__daemon")
            .arg("--resume")
            .arg(snap_fd.to_string())
            .arg("--rollback")
            .arg(&copy)
            .arg0("arcade")
            .exec();
        // The exec failed: nothing changed.
        for fd in &fds {
            set_cloexec(*fd, true);
        }
        let _ = std::fs::remove_file(&copy);
        self.log.line(&format!("upgrade: the exec failed: {error}"));
        (
            false,
            format!("the new program could not be started: {error}"),
        )
    }
}

/// Copy the running program's file (read through the descriptor kept open since the start) to
/// `to`, private to the user.
fn copy_program(from: &File, to: &Path) -> io::Result<()> {
    let mut out = std::fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .mode(0o700)
        .open(to)?;
    let mut offset = 0u64;
    let mut buf = vec![0u8; 1 << 20];
    loop {
        let n = std::os::unix::fs::FileExt::read_at(from, &mut buf, offset)?;
        if n == 0 {
            break;
        }
        out.write_all(&buf[..n])?;
        offset += n as u64;
    }
    out.sync_all()
}

/// Run `program` as a rehearsal on the snapshot and return its version when it could rebuild
/// everything.
fn rehearse_with(program: &Path, socket: &Path, fd: i32, wait: Duration) -> Result<String, String> {
    let mut child = Command::new(program)
        .arg("--socket")
        .arg(socket)
        .arg("__rehearse")
        .arg(fd.to_string())
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .spawn()
        .map_err(|e| format!("it did not start: {e}"))?;
    let mut stdout = child.stdout.take().ok_or("no output")?;
    let (tx, rx) = std::sync::mpsc::channel();
    std::thread::spawn(move || {
        let mut text = String::new();
        let _ = stdout.read_to_string(&mut text);
        let _ = tx.send(text);
    });
    let text = match rx.recv_timeout(wait) {
        Ok(text) => text,
        Err(_) => {
            let _ = child.kill();
            let _ = child.wait();
            return Err("the rehearsal took too long".to_string());
        }
    };
    let status = child.wait().map_err(|e| e.to_string())?;
    let line = text.lines().last().unwrap_or("").trim();
    if let Some(rest) = line.strip_prefix("ok ") {
        return Ok(rest.split(' ').next().unwrap_or("").to_string());
    }
    if let Some(why) = line.strip_prefix("error ") {
        return Err(why.to_string());
    }
    Err(format!("the rehearsal ended without an answer ({status})"))
}

// ---- Failures made on purpose, for the tests (debug builds only) ----

/// What a test asked this program to do wrong (or to keep), from the file next to the socket that
/// names it.
/// The file is taken away when it is read, so that the program started to go back does not do
/// the same. Compiled out of release builds.
#[cfg(debug_assertions)]
fn injected_fault(socket: &Path, only: &[&str]) -> Option<String> {
    let path = socket.with_extension("fault");
    let text = std::fs::read_to_string(&path).ok()?;
    let kind = text.trim().to_string();
    if only.contains(&kind.as_str()) {
        let _ = std::fs::remove_file(&path);
        Some(kind)
    } else {
        None
    }
}

#[cfg(not(debug_assertions))]
fn injected_fault(_socket: &Path, _only: &[&str]) -> Option<String> {
    None
}

// ---- In the new program ----

fn listener_for_rehearsal(socket: &Path) -> io::Result<UnixListener> {
    let path = socket.with_extension(format!("r{}", std::process::id()));
    let _ = std::fs::remove_file(&path);
    let std_listener = StdListener::bind(&path)?;
    let _ = std::fs::remove_file(&path);
    std_listener.set_nonblocking(true)?;
    Ok(UnixListener::from_std(std_listener))
}

/// The answer of a rehearsal: `ok <version> <sessions> <windows> <panes> <degraded>` or
/// `error <why>`.
pub fn rehearse(config: &DaemonConfig, fd: i32) -> String {
    if injected_fault(&config.socket_path, &["rehearsal-error"]).is_some() {
        return "error made on purpose by a test".to_string();
    }
    let result = (|| -> Result<Built, String> {
        let data = read_whole(fd).map_err(|e| format!("the snapshot cannot be read: {e}"))?;
        let snap = Snapshot::decode(&data).map_err(|e| e.to_string())?;
        let listener = listener_for_rehearsal(&config.socket_path).map_err(|e| e.to_string())?;
        let mut config = config.clone();
        config.log_path = None;
        config.config_path = None;
        let mut server = new_server(&config, listener).map_err(|e| e.to_string())?;
        let built = server.restore(&snap, false);
        // Nothing of the rehearsal is a real session: no process is touched on the way out.
        std::mem::forget(server);
        built
    })();
    match result {
        Ok(b) => format!(
            "ok {} {} {} {} {}",
            env!("CARGO_PKG_VERSION"),
            b.sessions,
            b.windows,
            b.panes,
            b.degraded
        ),
        Err(why) => format!("error {}", why.replace('\n', " ")),
    }
}

/// Start as the program that takes over from another one (or, after a rollback, from the one
/// that failed to). Runs the daemon until it is done.
pub fn resume(config: DaemonConfig, resume: Resume) -> io::Result<()> {
    let fail = |why: String| -> io::Error { io::Error::other(why) };
    let data =
        read_whole(resume.fd).map_err(|e| fail(format!("the snapshot cannot be read: {e}")))?;
    let scale = Duration::from_millis(data.len() as u64 / 10_000);
    // A test may ask for this program to fail on its way up (the file is read only here, by the
    // program that is not going back).
    let fault = match &resume.rolled_back {
        None => injected_fault(&config.socket_path, &["error", "panic", "crash", "hang"]),
        Some(_) => None,
    };
    let wait = if fault.as_deref() == Some("hang") {
        Duration::from_secs(2)
    } else {
        READY_BASE + scale
    };
    let armed = match (&resume.rollback, &resume.rolled_back) {
        (Some(copy), None) => arm(
            copy,
            &config.socket_path,
            resume.fd,
            "the new program did not become ready",
            wait,
        ),
        _ => false,
    };
    // A failure before the new program is ready sends the old one back in.
    let give_up = |why: String| -> io::Error {
        if armed {
            // The reason travels in the plan's argument list as prepared; the log has the rest.
            go_back();
        }
        fail(why)
    };
    let snap = match Snapshot::decode(&data) {
        Ok(s) => s,
        Err(e @ SnapshotError::TooNew { .. }) => return Err(give_up(e.to_string())),
        Err(e) => return Err(give_up(e.to_string())),
    };
    // The listening socket is used through a duplicate, so that a failure that drops it does
    // not close the descriptor the old program is to find.
    // SAFETY: fcntl duplicates a descriptor the snapshot names, and the duplicate is ours.
    let dup = unsafe { libc::fcntl(snap.listener_fd, libc::F_DUPFD_CLOEXEC, 3) };
    if dup < 0 {
        return Err(give_up(format!(
            "the listening socket is gone: {}",
            io::Error::last_os_error()
        )));
    }
    // SAFETY: `dup` is a fresh descriptor of a listening socket that nothing else owns.
    let listener = unsafe { UnixListener::from_raw_fd(dup) };
    let mut server = match new_server(&config, listener) {
        Ok(s) => s,
        Err(e) => return Err(give_up(e.to_string())),
    };
    let rolled_back = resume.rolled_back.clone();
    if rolled_back.is_some() && !snap.program_path.is_empty() {
        server.program_path = Some(PathBuf::from(&snap.program_path));
    }
    let built = match server.restore(&snap, true) {
        Ok(b) => b,
        Err(why) => {
            server.log.line(&format!("restore failed: {why}"));
            // Its descriptors are the old program's to use again: they are not closed here.
            std::mem::forget(server);
            return Err(give_up(why));
        }
    };
    match fault.as_deref() {
        Some("error") => {
            std::mem::forget(server);
            return Err(give_up("made on purpose by a test".to_string()));
        }
        Some("panic") => panic!("made on purpose by a test"),
        // SAFETY: raising a signal in this process.
        Some("crash") => unsafe {
            libc::raise(libc::SIGSEGV);
        },
        Some("hang") => loop {
            std::thread::sleep(Duration::from_secs(1));
        },
        _ => {}
    }
    server.settle_clients(&snap, built, rolled_back.as_deref());
    server.log.line(&match &rolled_back {
        None => format!(
            "upgraded from {}: {} sessions, {} windows, {} panes{}",
            snap.program_version,
            built.sessions,
            built.windows,
            built.panes,
            if built.degraded > 0 {
                format!(", {} with an empty screen", built.degraded)
            } else {
                String::new()
            }
        ),
        Some(why) => format!("the upgrade to a newer program was undone: {why}"),
    });
    // Ready: the descriptors are closed on exec again, the way back is given up, and the
    // saved copy and the snapshot go.
    for p in &snap.panes {
        set_cloexec(p.master_fd, true);
        if let Some(s) = p.slave_fd {
            set_cloexec(s, true);
        }
    }
    for c in &snap.clients {
        set_cloexec(c.fd, true);
    }
    set_cloexec(snap.listener_fd, true);
    if armed {
        disarm();
    }
    if let Some(copy) = &resume.rollback {
        let _ = std::fs::remove_file(copy);
    }
    // The program that went back runs from the saved copy: that file is not needed any more
    // (the running program keeps its own), and the daemon's program is where it was before.
    if rolled_back.is_some()
        && let Ok(own) = std::env::current_exe()
        && own
            .file_name()
            .is_some_and(|n| n.to_string_lossy().starts_with("arcade-previous-"))
    {
        let _ = std::fs::remove_file(own);
    }
    // SAFETY: closing the snapshot's descriptor, which only this program holds now, and the
    // original listener descriptor, of which the server uses the duplicate.
    unsafe {
        libc::close(resume.fd);
        libc::close(snap.listener_fd);
    }
    let started = if rolled_back.is_some() {
        "daemon started again after an undone upgrade"
    } else {
        "daemon replaced its program"
    };
    serve(server, &config, started)
}
