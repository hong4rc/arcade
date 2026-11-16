//! The snapshot a daemon writes before it replaces its own program, and the new program reads
//! to put every session back: plain data and the code that writes and reads it.
//!
//! The file is a header and then a run of parts, each with a tag and a length, so that a reader
//! can skip a part it does not know and a part it cannot read costs only what it holds: a pane's
//! terminal that cannot be read leaves that pane with an empty screen; any other part that
//! cannot be read ends the restore, which the caller reports. Every version of the program
//! must read every format that an earlier release wrote (`tests/snapshots/` keeps one of each),
//! and refuses a format newer than it knows.
//!
//! Nothing here touches descriptors, processes or the screen: the numbers of the descriptors
//! that the new program inherits are data like the rest.

use crate::layout::Shape;

/// The newest format this program reads and the one it writes.
pub const FORMAT_VERSION: u32 = 1;

const MAGIC: &[u8; 8] = b"ARCSNAP\0";

const PART_META: u8 = 1;
const PART_SESSION: u8 = 2;
const PART_PANE: u8 = 3;
const PART_TERMINAL: u8 = 4;
const PART_CLIENT: u8 = 5;
const PART_NOTICE: u8 = 6;

#[derive(Clone, Debug, PartialEq)]
pub struct Snapshot {
    /// The version of the program that wrote this.
    pub program_version: String,
    /// The path of the program that wrote this, which is the one to run again when the new one
    /// cannot take over.
    pub program_path: String,
    pub socket_path: String,
    pub listener_fd: i32,
    pub next_token: u64,
    pub next_session: u64,
    pub next_window: u64,
    pub use_counter: u64,
    pub ever_had_session: bool,
    /// What the last copy held.
    pub clipboard: Option<String>,
    /// The process that holds the machine awake, still running.
    pub sleep_guard_pid: Option<u32>,
    pub sessions: Vec<SessionRecord>,
    pub panes: Vec<PaneRecord>,
    pub clients: Vec<ClientRecord>,
    pub notices: Vec<NoticeRecord>,
}

#[derive(Clone, Debug, PartialEq)]
pub struct SessionRecord {
    pub id: u64,
    pub name: String,
    pub env: Vec<String>,
    pub cwd: String,
    pub current: u64,
    pub last_window: Option<u64>,
    pub cols: u16,
    pub rows: u16,
    pub last_used: u64,
    pub windows: Vec<WindowRecord>,
}

#[derive(Clone, Debug, PartialEq)]
pub struct WindowRecord {
    pub id: u64,
    pub number: u64,
    pub name: String,
    pub name_manual: bool,
    pub focus: u64,
    pub last_focus: Option<u64>,
    pub zoomed: bool,
    pub arrangement: Option<u8>,
    pub layout_cols: u16,
    pub layout_rows: u16,
    pub layout: Shape,
}

#[derive(Clone, Debug, PartialEq)]
pub struct TitleRecord {
    pub title: Option<String>,
    pub owner: Option<i32>,
    pub by: Option<i32>,
}

#[derive(Clone, Debug, PartialEq)]
pub struct PaneRecord {
    pub id: u64,
    pub pid: i32,
    pub master_fd: i32,
    pub slave_fd: Option<i32>,
    pub cols: u16,
    pub rows: u16,
    /// Bytes for the program that were not written to it yet.
    pub input: Vec<u8>,
    pub title: Option<String>,
    pub title_owner: Option<i32>,
    pub title_stack: Vec<TitleRecord>,
    /// The last progress report: state and value.
    pub progress: Option<(u8, u8)>,
    /// The process ended and its pane was going to be closed; the text for the client.
    pub exit_note: Option<String>,
    /// The shape of the cursor (0 block, 1 underline, 2 bar, 3 hollow block) and whether it
    /// blinks.
    pub cursor: Option<(u8, bool)>,
    /// The terminal as the formatter wrote it; `None` when that part could not be read.
    pub terminal: Option<TerminalRecord>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct TerminalRecord {
    pub primary: Vec<u8>,
    pub alternate: Option<Vec<u8>>,
}

#[derive(Clone, Debug, PartialEq)]
pub struct ClientRecord {
    pub id: u64,
    pub fd: i32,
    /// The session it was attached to, with the size of its terminal.
    pub session: Option<String>,
    pub cols: u16,
    pub rows: u16,
    /// The client that asked for the upgrade and waits for the answer.
    pub requester: bool,
}

#[derive(Clone, Debug, PartialEq)]
pub struct NoticeRecord {
    pub pane: u64,
    pub class: u8,
    pub source: String,
    pub title: String,
    pub message: String,
    pub age_ms: u64,
}

/// Why a snapshot could not be read.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum SnapshotError {
    /// Not a snapshot at all.
    NotASnapshot,
    /// Written by a newer format than this program reads.
    TooNew { found: u32, known: u32 },
    /// A part is cut short or holds nonsense.
    Damaged(String),
}

impl std::fmt::Display for SnapshotError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            SnapshotError::NotASnapshot => write!(f, "this is not an arcade snapshot"),
            SnapshotError::TooNew { found, known } => write!(
                f,
                "the snapshot was written in format {found}, which this version (it reads up to {known}) does not know: it comes from a newer arcade, and going back to an older one is not supported"
            ),
            SnapshotError::Damaged(what) => write!(f, "the snapshot is damaged: {what}"),
        }
    }
}

// ---- Writing ----

#[derive(Default)]
struct Out {
    buf: Vec<u8>,
}

impl Out {
    fn u8(&mut self, v: u8) {
        self.buf.push(v);
    }
    fn bool(&mut self, v: bool) {
        self.u8(u8::from(v));
    }
    fn u16(&mut self, v: u16) {
        self.buf.extend_from_slice(&v.to_le_bytes());
    }
    fn u32(&mut self, v: u32) {
        self.buf.extend_from_slice(&v.to_le_bytes());
    }
    fn i32(&mut self, v: i32) {
        self.buf.extend_from_slice(&v.to_le_bytes());
    }
    fn u64(&mut self, v: u64) {
        self.buf.extend_from_slice(&v.to_le_bytes());
    }
    fn f64(&mut self, v: f64) {
        self.u64(v.to_bits());
    }
    fn bytes(&mut self, v: &[u8]) {
        self.u64(v.len() as u64);
        self.buf.extend_from_slice(v);
    }
    fn str(&mut self, v: &str) {
        self.bytes(v.as_bytes());
    }
    fn opt<T>(&mut self, v: &Option<T>, write: impl FnOnce(&mut Out, &T)) {
        match v {
            Some(v) => {
                self.u8(1);
                write(self, v);
            }
            None => self.u8(0),
        }
    }
    fn list<T>(&mut self, v: &[T], mut write: impl FnMut(&mut Out, &T)) {
        self.u32(v.len() as u32);
        for item in v {
            write(self, item);
        }
    }
}

fn part(file: &mut Vec<u8>, tag: u8, body: &Out) {
    file.push(tag);
    file.extend_from_slice(&(body.buf.len() as u64).to_le_bytes());
    file.extend_from_slice(&body.buf);
}

fn write_shape(o: &mut Out, shape: &Shape) {
    match shape {
        Shape::Leaf(pane) => {
            o.u8(0);
            o.u64(*pane);
        }
        Shape::Split {
            side_by_side,
            ratio,
            first,
            second,
        } => {
            o.u8(1);
            o.bool(*side_by_side);
            o.f64(*ratio);
            write_shape(o, first);
            write_shape(o, second);
        }
    }
}

impl Snapshot {
    /// The bytes of the file.
    pub fn encode(&self) -> Vec<u8> {
        let mut file = Vec::new();
        file.extend_from_slice(MAGIC);
        file.extend_from_slice(&FORMAT_VERSION.to_le_bytes());

        let mut o = Out::default();
        o.str(&self.program_version);
        o.str(&self.program_path);
        o.str(&self.socket_path);
        o.i32(self.listener_fd);
        o.u64(self.next_token);
        o.u64(self.next_session);
        o.u64(self.next_window);
        o.u64(self.use_counter);
        o.bool(self.ever_had_session);
        o.opt(&self.clipboard, |o, v| o.str(v));
        o.opt(&self.sleep_guard_pid, |o, v| o.u32(*v));
        part(&mut file, PART_META, &o);

        for s in &self.sessions {
            let mut o = Out::default();
            o.u64(s.id);
            o.str(&s.name);
            o.list(&s.env, |o, v| o.str(v));
            o.str(&s.cwd);
            o.u64(s.current);
            o.opt(&s.last_window, |o, v| o.u64(*v));
            o.u16(s.cols);
            o.u16(s.rows);
            o.u64(s.last_used);
            o.list(&s.windows, |o, w| {
                o.u64(w.id);
                o.u64(w.number);
                o.str(&w.name);
                o.bool(w.name_manual);
                o.u64(w.focus);
                o.opt(&w.last_focus, |o, v| o.u64(*v));
                o.bool(w.zoomed);
                o.opt(&w.arrangement, |o, v| o.u8(*v));
                o.u16(w.layout_cols);
                o.u16(w.layout_rows);
                write_shape(o, &w.layout);
            });
            part(&mut file, PART_SESSION, &o);
        }

        for p in &self.panes {
            let mut o = Out::default();
            o.u64(p.id);
            o.i32(p.pid);
            o.i32(p.master_fd);
            o.opt(&p.slave_fd, |o, v| o.i32(*v));
            o.u16(p.cols);
            o.u16(p.rows);
            o.bytes(&p.input);
            o.opt(&p.title, |o, v| o.str(v));
            o.opt(&p.title_owner, |o, v| o.i32(*v));
            o.list(&p.title_stack, |o, t| {
                o.opt(&t.title, |o, v| o.str(v));
                o.opt(&t.owner, |o, v| o.i32(*v));
                o.opt(&t.by, |o, v| o.i32(*v));
            });
            o.opt(&p.progress, |o, (state, value)| {
                o.u8(*state);
                o.u8(*value);
            });
            o.opt(&p.exit_note, |o, v| o.str(v));
            o.opt(&p.cursor, |o, (shape, blinking)| {
                o.u8(*shape);
                o.bool(*blinking);
            });
            part(&mut file, PART_PANE, &o);
            if let Some(t) = &p.terminal {
                let mut o = Out::default();
                o.u64(p.id);
                o.bytes(&t.primary);
                o.opt(&t.alternate, |o, v| o.bytes(v));
                part(&mut file, PART_TERMINAL, &o);
            }
        }

        for c in &self.clients {
            let mut o = Out::default();
            o.u64(c.id);
            o.i32(c.fd);
            o.opt(&c.session, |o, v| o.str(v));
            o.u16(c.cols);
            o.u16(c.rows);
            o.bool(c.requester);
            part(&mut file, PART_CLIENT, &o);
        }

        for n in &self.notices {
            let mut o = Out::default();
            o.u64(n.pane);
            o.u8(n.class);
            o.str(&n.source);
            o.str(&n.title);
            o.str(&n.message);
            o.u64(n.age_ms);
            part(&mut file, PART_NOTICE, &o);
        }
        file
    }
}

// ---- Reading ----

struct In<'a> {
    data: &'a [u8],
    at: usize,
}

type Read<T> = Result<T, SnapshotError>;

fn damaged<T>(what: &str) -> Read<T> {
    Err(SnapshotError::Damaged(what.to_string()))
}

impl<'a> In<'a> {
    fn take(&mut self, n: usize) -> Read<&'a [u8]> {
        let end = self.at.checked_add(n).filter(|e| *e <= self.data.len());
        match end {
            Some(end) => {
                let out = &self.data[self.at..end];
                self.at = end;
                Ok(out)
            }
            None => damaged("a part ends too soon"),
        }
    }
    fn u8(&mut self) -> Read<u8> {
        Ok(self.take(1)?[0])
    }
    fn bool(&mut self) -> Read<bool> {
        match self.u8()? {
            0 => Ok(false),
            1 => Ok(true),
            _ => damaged("a flag is neither 0 nor 1"),
        }
    }
    fn u16(&mut self) -> Read<u16> {
        Ok(u16::from_le_bytes(self.take(2)?.try_into().unwrap()))
    }
    fn u32(&mut self) -> Read<u32> {
        Ok(u32::from_le_bytes(self.take(4)?.try_into().unwrap()))
    }
    fn i32(&mut self) -> Read<i32> {
        Ok(i32::from_le_bytes(self.take(4)?.try_into().unwrap()))
    }
    fn u64(&mut self) -> Read<u64> {
        Ok(u64::from_le_bytes(self.take(8)?.try_into().unwrap()))
    }
    fn f64(&mut self) -> Read<f64> {
        Ok(f64::from_bits(self.u64()?))
    }
    fn bytes(&mut self) -> Read<Vec<u8>> {
        let n = usize::try_from(self.u64()?).or_else(|_| damaged("a length is too large"))?;
        Ok(self.take(n)?.to_vec())
    }
    fn str(&mut self) -> Read<String> {
        String::from_utf8(self.bytes()?).or_else(|_| damaged("text that is not UTF-8"))
    }
    fn opt<T>(&mut self, read: impl FnOnce(&mut In<'a>) -> Read<T>) -> Read<Option<T>> {
        match self.u8()? {
            0 => Ok(None),
            1 => Ok(Some(read(self)?)),
            _ => damaged("an option is neither 0 nor 1"),
        }
    }
    fn list<T>(&mut self, mut read: impl FnMut(&mut In<'a>) -> Read<T>) -> Read<Vec<T>> {
        let n = self.u32()? as usize;
        // Each item takes at least a byte, so a count beyond what is left is nonsense (and must
        // not make an allocation).
        if n > self.data.len() - self.at {
            return damaged("a list is longer than the part");
        }
        (0..n).map(|_| read(self)).collect()
    }
    fn done(&self) -> Read<()> {
        if self.at == self.data.len() {
            Ok(())
        } else {
            damaged("bytes left at the end of a part")
        }
    }
}

/// The deepest layout tree read: a window cannot hold more panes than that.
const SHAPE_DEPTH: usize = 256;

fn read_shape(i: &mut In<'_>, depth: usize) -> Read<Shape> {
    if depth > SHAPE_DEPTH {
        return damaged("a layout is nested too deep");
    }
    match i.u8()? {
        0 => Ok(Shape::Leaf(i.u64()?)),
        1 => {
            let side_by_side = i.bool()?;
            let ratio = i.f64()?;
            if !ratio.is_finite() {
                return damaged("a split ratio is not a number");
            }
            let first = Box::new(read_shape(i, depth + 1)?);
            let second = Box::new(read_shape(i, depth + 1)?);
            Ok(Shape::Split {
                side_by_side,
                ratio,
                first,
                second,
            })
        }
        _ => damaged("a layout node of an unknown kind"),
    }
}

impl Snapshot {
    /// Read a snapshot. A terminal part that cannot be read is left out (the pane has
    /// `terminal: None`); every other fault is an error.
    pub fn decode(data: &[u8]) -> Result<Snapshot, SnapshotError> {
        if data.len() < MAGIC.len() + 4 || &data[..MAGIC.len()] != MAGIC {
            return Err(SnapshotError::NotASnapshot);
        }
        let found = u32::from_le_bytes(data[MAGIC.len()..MAGIC.len() + 4].try_into().unwrap());
        if found > FORMAT_VERSION {
            return Err(SnapshotError::TooNew {
                found,
                known: FORMAT_VERSION,
            });
        }
        if found == 0 {
            return Err(SnapshotError::NotASnapshot);
        }
        let mut file = In {
            data,
            at: MAGIC.len() + 4,
        };
        let mut meta = None;
        let mut sessions = Vec::new();
        let mut panes: Vec<PaneRecord> = Vec::new();
        let mut terminals: Vec<(u64, Option<TerminalRecord>)> = Vec::new();
        let mut clients = Vec::new();
        let mut notices = Vec::new();
        while file.at < data.len() {
            let tag = file.u8()?;
            let len = usize::try_from(file.u64()?).or_else(|_| damaged("a part is too long"))?;
            let body = file.take(len)?;
            let mut i = In { data: body, at: 0 };
            match tag {
                PART_META => meta = Some(read_meta(&mut i)?),
                PART_SESSION => sessions.push(read_session(&mut i)?),
                PART_PANE => panes.push(read_pane(&mut i)?),
                PART_TERMINAL => {
                    // A terminal that cannot be read costs that pane its history, nothing more.
                    let id = body
                        .get(..8)
                        .map(|b| u64::from_le_bytes(b.try_into().unwrap()));
                    if let Some(id) = id {
                        terminals.push((id, read_terminal(&mut i).ok()));
                    }
                }
                PART_CLIENT => clients.push(read_client(&mut i)?),
                PART_NOTICE => notices.push(read_notice(&mut i)?),
                // A part of a kind this program does not know: skipped.
                _ => continue,
            }
        }
        let mut snapshot = meta.ok_or(SnapshotError::Damaged("the main part is missing".into()))?;
        for (id, terminal) in terminals {
            if let Some(p) = panes.iter_mut().find(|p| p.id == id) {
                p.terminal = terminal;
            }
        }
        snapshot.sessions = sessions;
        snapshot.panes = panes;
        snapshot.clients = clients;
        snapshot.notices = notices;
        Ok(snapshot)
    }
}

fn read_meta(i: &mut In<'_>) -> Read<Snapshot> {
    let s = Snapshot {
        program_version: i.str()?,
        program_path: i.str()?,
        socket_path: i.str()?,
        listener_fd: i.i32()?,
        next_token: i.u64()?,
        next_session: i.u64()?,
        next_window: i.u64()?,
        use_counter: i.u64()?,
        ever_had_session: i.bool()?,
        clipboard: i.opt(|i| i.str())?,
        sleep_guard_pid: i.opt(|i| i.u32())?,
        sessions: Vec::new(),
        panes: Vec::new(),
        clients: Vec::new(),
        notices: Vec::new(),
    };
    i.done()?;
    Ok(s)
}

fn read_session(i: &mut In<'_>) -> Read<SessionRecord> {
    let s = SessionRecord {
        id: i.u64()?,
        name: i.str()?,
        env: i.list(|i| i.str())?,
        cwd: i.str()?,
        current: i.u64()?,
        last_window: i.opt(|i| i.u64())?,
        cols: i.u16()?,
        rows: i.u16()?,
        last_used: i.u64()?,
        windows: i.list(|i| {
            Ok(WindowRecord {
                id: i.u64()?,
                number: i.u64()?,
                name: i.str()?,
                name_manual: i.bool()?,
                focus: i.u64()?,
                last_focus: i.opt(|i| i.u64())?,
                zoomed: i.bool()?,
                arrangement: i.opt(|i| i.u8())?,
                layout_cols: i.u16()?,
                layout_rows: i.u16()?,
                layout: read_shape(i, 0)?,
            })
        })?,
    };
    i.done()?;
    Ok(s)
}

fn read_pane(i: &mut In<'_>) -> Read<PaneRecord> {
    let p = PaneRecord {
        id: i.u64()?,
        pid: i.i32()?,
        master_fd: i.i32()?,
        slave_fd: i.opt(|i| i.i32())?,
        cols: i.u16()?,
        rows: i.u16()?,
        input: i.bytes()?,
        title: i.opt(|i| i.str())?,
        title_owner: i.opt(|i| i.i32())?,
        title_stack: i.list(|i| {
            Ok(TitleRecord {
                title: i.opt(|i| i.str())?,
                owner: i.opt(|i| i.i32())?,
                by: i.opt(|i| i.i32())?,
            })
        })?,
        progress: i.opt(|i| Ok((i.u8()?, i.u8()?)))?,
        exit_note: i.opt(|i| i.str())?,
        cursor: i.opt(|i| Ok((i.u8()?, i.bool()?)))?,
        terminal: None,
    };
    i.done()?;
    Ok(p)
}

fn read_terminal(i: &mut In<'_>) -> Read<TerminalRecord> {
    let _pane = i.u64()?;
    let t = TerminalRecord {
        primary: i.bytes()?,
        alternate: i.opt(|i| i.bytes())?,
    };
    i.done()?;
    Ok(t)
}

fn read_client(i: &mut In<'_>) -> Read<ClientRecord> {
    let c = ClientRecord {
        id: i.u64()?,
        fd: i.i32()?,
        session: i.opt(|i| i.str())?,
        cols: i.u16()?,
        rows: i.u16()?,
        requester: i.bool()?,
    };
    i.done()?;
    Ok(c)
}

fn read_notice(i: &mut In<'_>) -> Read<NoticeRecord> {
    let n = NoticeRecord {
        pane: i.u64()?,
        class: i.u8()?,
        source: i.str()?,
        title: i.str()?,
        message: i.str()?,
        age_ms: i.u64()?,
    };
    i.done()?;
    Ok(n)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sample() -> Snapshot {
        Snapshot {
            program_version: "0.1.5".into(),
            program_path: "/opt/arcade/bin/arcade".into(),
            socket_path: "/tmp/arcade-501/default".into(),
            listener_fd: 7,
            next_token: 44,
            next_session: 3,
            next_window: 9,
            use_counter: 12,
            ever_had_session: true,
            clipboard: Some("copied text\nwith a second line".into()),
            sleep_guard_pid: Some(4242),
            sessions: vec![SessionRecord {
                id: 1,
                name: "work".into(),
                env: vec!["A=1".into(), "SHELL=/bin/zsh".into()],
                cwd: "/Users/someone/project".into(),
                current: 2,
                last_window: Some(1),
                cols: 120,
                rows: 40,
                last_used: 11,
                windows: vec![WindowRecord {
                    id: 2,
                    number: 1,
                    name: "arcade".into(),
                    name_manual: true,
                    focus: 5,
                    last_focus: Some(6),
                    zoomed: false,
                    arrangement: Some(2),
                    layout_cols: 120,
                    layout_rows: 39,
                    layout: Shape::Split {
                        side_by_side: true,
                        ratio: 0.375,
                        first: Box::new(Shape::Leaf(5)),
                        second: Box::new(Shape::Split {
                            side_by_side: false,
                            ratio: 0.5,
                            first: Box::new(Shape::Leaf(6)),
                            second: Box::new(Shape::Leaf(8)),
                        }),
                    },
                }],
            }],
            panes: vec![
                PaneRecord {
                    id: 5,
                    pid: 900,
                    master_fd: 11,
                    slave_fd: Some(12),
                    cols: 44,
                    rows: 39,
                    input: b"ls\r".to_vec(),
                    title: Some("vim".into()),
                    title_owner: Some(901),
                    title_stack: vec![TitleRecord {
                        title: None,
                        owner: Some(1),
                        by: Some(2),
                    }],
                    progress: Some((1, 42)),
                    exit_note: None,
                    cursor: Some((2, true)),
                    terminal: Some(TerminalRecord {
                        primary: b"\x1b[31mhello\x1b[0m".to_vec(),
                        alternate: Some(b"alt".to_vec()),
                    }),
                },
                PaneRecord {
                    id: 6,
                    pid: 910,
                    master_fd: 13,
                    slave_fd: None,
                    cols: 75,
                    rows: 19,
                    input: Vec::new(),
                    title: None,
                    title_owner: None,
                    title_stack: Vec::new(),
                    progress: None,
                    exit_note: Some("exited".into()),
                    cursor: None,
                    terminal: Some(TerminalRecord {
                        primary: Vec::new(),
                        alternate: None,
                    }),
                },
            ],
            clients: vec![ClientRecord {
                id: 20,
                fd: 15,
                session: Some("work".into()),
                cols: 120,
                rows: 40,
                requester: false,
            }],
            notices: vec![NoticeRecord {
                pane: 6,
                class: 3,
                source: "claude".into(),
                title: "Needs approval".into(),
                message: "run ls?".into(),
                age_ms: 12_345,
            }],
        }
    }

    #[test]
    fn a_snapshot_is_read_back_as_it_was_written() {
        let s = sample();
        assert_eq!(Snapshot::decode(&s.encode()).unwrap(), s);
    }

    #[test]
    fn a_newer_format_and_a_file_that_is_not_a_snapshot_are_refused_with_their_reasons() {
        let mut bytes = sample().encode();
        bytes[8..12].copy_from_slice(&(FORMAT_VERSION + 1).to_le_bytes());
        match Snapshot::decode(&bytes) {
            Err(SnapshotError::TooNew { found, known }) => {
                assert_eq!((found, known), (FORMAT_VERSION + 1, FORMAT_VERSION));
            }
            other => panic!("{other:?}"),
        }
        assert_eq!(
            Snapshot::decode(b"something else entirely"),
            Err(SnapshotError::NotASnapshot)
        );
        assert_eq!(Snapshot::decode(b""), Err(SnapshotError::NotASnapshot));
        let message = SnapshotError::TooNew { found: 2, known: 1 }.to_string();
        assert!(message.contains("newer"), "{message}");
    }

    #[test]
    fn a_part_of_an_unknown_kind_is_skipped_and_a_damaged_terminal_costs_only_its_pane() {
        let s = sample();
        let mut bytes = s.encode();
        // A part of a kind from the future, in the middle of the file.
        let mut future = vec![99u8];
        future.extend_from_slice(&3u64.to_le_bytes());
        future.extend_from_slice(b"xyz");
        bytes.extend_from_slice(&future);
        assert_eq!(Snapshot::decode(&bytes).unwrap(), s);
        // The first terminal part, found by walking the parts: make a length inside it nonsense.
        let mut at = 12;
        let mut terminal_at = None;
        while at < bytes.len() {
            let len = u64::from_le_bytes(bytes[at + 1..at + 9].try_into().unwrap()) as usize;
            if bytes[at] == PART_TERMINAL && terminal_at.is_none() {
                terminal_at = Some(at);
            }
            at += 9 + len;
        }
        let at = terminal_at.unwrap();
        // The part starts with the pane's number, then the length of the primary text.
        bytes[at + 9 + 8..at + 9 + 16].copy_from_slice(&(1u64 << 40).to_le_bytes());
        let read = Snapshot::decode(&bytes).unwrap();
        assert_eq!(read.panes[0].terminal, None);
        assert_eq!(read.panes[1], s.panes[1]);
        assert_eq!(read.sessions, s.sessions);
    }

    #[test]
    fn a_cut_file_and_nonsense_inside_a_part_are_errors_and_never_a_crash() {
        let bytes = sample().encode();
        for cut in [13, 20, bytes.len() / 2, bytes.len() - 1] {
            assert!(Snapshot::decode(&bytes[..cut]).is_err(), "cut at {cut}");
        }
        // Every single byte changed to every value the formats use: never a panic.
        for at in 12..bytes.len().min(400) {
            for value in [0u8, 1, 2, 0x7f, 0xff] {
                let mut changed = bytes.clone();
                changed[at] = value;
                let _ = Snapshot::decode(&changed);
            }
        }
    }
}
