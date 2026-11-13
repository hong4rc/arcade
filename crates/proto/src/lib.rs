//! The protocol between the `arcade` client and the daemon.
//!
//! Every message is `u32 length (little endian) | u8 tag | payload`, where the length counts the
//! tag and the payload. Integers are little endian, strings and byte blocks carry a `u32` length.
//!
//! Four messages must never change layout, so that a client and a daemon of different versions
//! can still tell each other so, and so that a new client can stop an old daemon: the client's
//! [`ClientMessage::Hello`] (tag 1, the `u32` protocol version first) and
//! [`ClientMessage::Command`] (tag 6), and the daemon's [`ServerMessage::VersionMismatch`]
//! (tag 2) and [`ServerMessage::CommandResult`] (tag 7).

use std::fmt;

/// Version of this protocol. Bump it whenever any message changes.
pub const PROTOCOL_VERSION: u32 = 2;

/// Largest message the decoder accepts, tag and payload together.
pub const MAX_MESSAGE_LEN: usize = 8 * 1024 * 1024;

/// Largest number of items in a list inside a message (arguments, environment).
const MAX_LIST: usize = 4096;

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ProtoError {
    /// A message announced a length above [`MAX_MESSAGE_LEN`] (or could not be encoded).
    TooLarge {
        len: usize,
    },
    /// The tag byte belongs to no message.
    UnknownTag(u8),
    /// The payload ended before all fields were read.
    Truncated,
    /// The payload held more bytes than the message needs.
    TrailingBytes,
    InvalidUtf8,
    /// A list in the message was longer than allowed.
    ListTooLong,
    /// A field held a value that is not defined (an unknown enum number).
    InvalidValue,
    /// The daemon and the client speak different versions of the protocol.
    VersionMismatch {
        daemon: u32,
        client: u32,
    },
}

impl fmt::Display for ProtoError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            ProtoError::TooLarge { len } => {
                write!(
                    f,
                    "message of {len} bytes exceeds the {MAX_MESSAGE_LEN} byte limit"
                )
            }
            ProtoError::UnknownTag(tag) => write!(f, "unknown message tag {tag}"),
            ProtoError::Truncated => write!(f, "message ended too early"),
            ProtoError::TrailingBytes => write!(f, "message has bytes left over"),
            ProtoError::InvalidUtf8 => write!(f, "text in message is not valid UTF-8"),
            ProtoError::ListTooLong => write!(f, "list in message is too long"),
            ProtoError::InvalidValue => write!(f, "message has an undefined value"),
            ProtoError::VersionMismatch { daemon, client } => write!(
                f,
                "protocol version mismatch: daemon speaks {daemon}, client speaks {client}"
            ),
        }
    }
}

impl std::error::Error for ProtoError {}

/// Compare the version a peer announced with ours (`ours` is the daemon's or the client's own).
pub fn check_version(daemon: u32, client: u32) -> Result<(), ProtoError> {
    if daemon == client {
        Ok(())
    } else {
        Err(ProtoError::VersionMismatch { daemon, client })
    }
}

/// Whether an attach may or must create a session.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum CreateMode {
    /// Attach to an existing session; fail when there is none.
    Never,
    /// Attach to an existing session, or create one when there is none at all.
    IfNone,
    /// Always create a new session.
    Always,
}

/// What a client asks for when it connects to work with a session.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct AttachRequest {
    /// Name of the session to attach to; `None` means the most recently used one.
    pub session: Option<String>,
    pub create: CreateMode,
    /// Name for a session to create; `None` uses the name of the working directory.
    pub new_name: Option<String>,
    /// Command for the first pane of a created session; empty means the user's shell.
    pub command: Vec<String>,
    /// Working directory for the created session's pane.
    pub cwd: String,
    /// Environment variables to give the pane, as `NAME=value`.
    pub env: Vec<String>,
    pub cols: u16,
    pub rows: u16,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ClientMessage {
    /// First message on every connection. Layout fixed forever.
    Hello {
        version: u32,
    },
    /// Work with a session: create or find it and start receiving frames.
    Attach(AttachRequest),
    /// Bytes typed on the keyboard, passed to the pane untouched.
    Input(Vec<u8>),
    Resize {
        cols: u16,
        rows: u16,
    },
    Detach,
    /// A named command (`arcade run`, `ls`, `kill-server`), answered with `CommandResult`.
    Command {
        name: String,
        args: Vec<String>,
    },
    /// A named command run from inside a pane (`arcade run` with the `ARCADE_PANE` variable): it
    /// acts on the session and window that pane is in. A message of its own, so that the layout
    /// of [`ClientMessage::Command`] stays as it was.
    PaneCommand {
        pane: u64,
        name: String,
        args: Vec<String>,
    },
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum DetachReason {
    /// The client asked (a `session.detach` command, or its own detach message).
    Requested,
    /// Another client attached to the same session.
    Replaced,
    /// The daemon is stopping.
    ServerStopping,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ServerMessage {
    /// Reply to a matching [`ClientMessage::Hello`].
    HelloOk { version: u32 },
    /// Reply to a Hello with another version. Layout fixed forever.
    VersionMismatch { daemon: u32, client: u32 },
    /// The attach worked; frames follow.
    Attached { session: String },
    /// Bytes to write to the host terminal, as they are.
    Output(Vec<u8>),
    /// The client is no longer attached. The text is a line for the user, without escapes.
    Detached {
        reason: DetachReason,
        message: String,
    },
    /// The session ended (its last pane closed).
    SessionEnded { message: String },
    /// Answer to a [`ClientMessage::Command`] or a refused request.
    CommandResult { ok: bool, output: String },
}

// ---- Encoding ----

struct Writer {
    buf: Vec<u8>,
}

impl Writer {
    fn u8(&mut self, v: u8) {
        self.buf.push(v);
    }
    fn u16(&mut self, v: u16) {
        self.buf.extend_from_slice(&v.to_le_bytes());
    }
    fn u32(&mut self, v: u32) {
        self.buf.extend_from_slice(&v.to_le_bytes());
    }
    fn u64(&mut self, v: u64) {
        self.buf.extend_from_slice(&v.to_le_bytes());
    }
    fn bytes(&mut self, v: &[u8]) {
        self.u32(v.len() as u32);
        self.buf.extend_from_slice(v);
    }
    fn str(&mut self, v: &str) {
        self.bytes(v.as_bytes());
    }
    fn opt_str(&mut self, v: &Option<String>) {
        match v {
            Some(s) => {
                self.u8(1);
                self.str(s);
            }
            None => self.u8(0),
        }
    }
    fn list(&mut self, v: &[String]) {
        self.u32(v.len() as u32);
        for s in v {
            self.str(s);
        }
    }
}

struct Reader<'a> {
    data: &'a [u8],
}

impl<'a> Reader<'a> {
    fn take(&mut self, n: usize) -> Result<&'a [u8], ProtoError> {
        if self.data.len() < n {
            return Err(ProtoError::Truncated);
        }
        let (head, rest) = self.data.split_at(n);
        self.data = rest;
        Ok(head)
    }
    fn u8(&mut self) -> Result<u8, ProtoError> {
        Ok(self.take(1)?[0])
    }
    fn u16(&mut self) -> Result<u16, ProtoError> {
        let b = self.take(2)?;
        Ok(u16::from_le_bytes([b[0], b[1]]))
    }
    fn u32(&mut self) -> Result<u32, ProtoError> {
        let b = self.take(4)?;
        Ok(u32::from_le_bytes([b[0], b[1], b[2], b[3]]))
    }
    fn u64(&mut self) -> Result<u64, ProtoError> {
        let b = self.take(8)?;
        Ok(u64::from_le_bytes([
            b[0], b[1], b[2], b[3], b[4], b[5], b[6], b[7],
        ]))
    }
    fn bytes(&mut self) -> Result<Vec<u8>, ProtoError> {
        let len = self.u32()? as usize;
        // `take` checks the length against what is really there, so a huge announced length
        // allocates nothing.
        Ok(self.take(len)?.to_vec())
    }
    fn str(&mut self) -> Result<String, ProtoError> {
        String::from_utf8(self.bytes()?).map_err(|_| ProtoError::InvalidUtf8)
    }
    fn opt_str(&mut self) -> Result<Option<String>, ProtoError> {
        match self.u8()? {
            0 => Ok(None),
            1 => Ok(Some(self.str()?)),
            _ => Err(ProtoError::InvalidValue),
        }
    }
    fn list(&mut self) -> Result<Vec<String>, ProtoError> {
        let count = self.u32()? as usize;
        if count > MAX_LIST {
            return Err(ProtoError::ListTooLong);
        }
        let mut out = Vec::with_capacity(count.min(64));
        for _ in 0..count {
            out.push(self.str()?);
        }
        Ok(out)
    }
    fn finish(self) -> Result<(), ProtoError> {
        if self.data.is_empty() {
            Ok(())
        } else {
            Err(ProtoError::TrailingBytes)
        }
    }
}

/// A message that can be written to and read from the wire.
pub trait Message: Sized {
    fn encode_body(&self, w: &mut Vec<u8>) -> u8;
    fn decode_body(tag: u8, payload: &[u8]) -> Result<Self, ProtoError>;
}

impl Message for ClientMessage {
    fn encode_body(&self, out: &mut Vec<u8>) -> u8 {
        let mut w = Writer {
            buf: std::mem::take(out),
        };
        let tag = match self {
            ClientMessage::Hello { version } => {
                w.u32(*version);
                1
            }
            ClientMessage::Attach(a) => {
                w.opt_str(&a.session);
                w.u8(match a.create {
                    CreateMode::Never => 0,
                    CreateMode::IfNone => 1,
                    CreateMode::Always => 2,
                });
                w.opt_str(&a.new_name);
                w.list(&a.command);
                w.str(&a.cwd);
                w.list(&a.env);
                w.u16(a.cols);
                w.u16(a.rows);
                2
            }
            ClientMessage::Input(bytes) => {
                w.buf.extend_from_slice(bytes);
                3
            }
            ClientMessage::Resize { cols, rows } => {
                w.u16(*cols);
                w.u16(*rows);
                4
            }
            ClientMessage::Detach => 5,
            ClientMessage::Command { name, args } => {
                w.str(name);
                w.list(args);
                6
            }
            ClientMessage::PaneCommand { pane, name, args } => {
                w.u64(*pane);
                w.str(name);
                w.list(args);
                7
            }
        };
        *out = w.buf;
        tag
    }

    fn decode_body(tag: u8, payload: &[u8]) -> Result<Self, ProtoError> {
        let mut r = Reader { data: payload };
        let msg = match tag {
            1 => ClientMessage::Hello { version: r.u32()? },
            2 => ClientMessage::Attach(AttachRequest {
                session: r.opt_str()?,
                create: match r.u8()? {
                    0 => CreateMode::Never,
                    1 => CreateMode::IfNone,
                    2 => CreateMode::Always,
                    _ => return Err(ProtoError::InvalidValue),
                },
                new_name: r.opt_str()?,
                command: r.list()?,
                cwd: r.str()?,
                env: r.list()?,
                cols: r.u16()?,
                rows: r.u16()?,
            }),
            3 => {
                // Keyboard bytes fill the whole payload.
                let bytes = r.data.to_vec();
                r.data = &[];
                ClientMessage::Input(bytes)
            }
            4 => ClientMessage::Resize {
                cols: r.u16()?,
                rows: r.u16()?,
            },
            5 => ClientMessage::Detach,
            6 => ClientMessage::Command {
                name: r.str()?,
                args: r.list()?,
            },
            7 => ClientMessage::PaneCommand {
                pane: r.u64()?,
                name: r.str()?,
                args: r.list()?,
            },
            other => return Err(ProtoError::UnknownTag(other)),
        };
        r.finish()?;
        Ok(msg)
    }
}

impl Message for ServerMessage {
    fn encode_body(&self, out: &mut Vec<u8>) -> u8 {
        let mut w = Writer {
            buf: std::mem::take(out),
        };
        let tag = match self {
            ServerMessage::HelloOk { version } => {
                w.u32(*version);
                1
            }
            ServerMessage::VersionMismatch { daemon, client } => {
                w.u32(*daemon);
                w.u32(*client);
                2
            }
            ServerMessage::Attached { session } => {
                w.str(session);
                3
            }
            ServerMessage::Output(bytes) => {
                w.buf.extend_from_slice(bytes);
                4
            }
            ServerMessage::Detached { reason, message } => {
                w.u8(match reason {
                    DetachReason::Requested => 0,
                    DetachReason::Replaced => 1,
                    DetachReason::ServerStopping => 2,
                });
                w.str(message);
                5
            }
            ServerMessage::SessionEnded { message } => {
                w.str(message);
                6
            }
            ServerMessage::CommandResult { ok, output } => {
                w.u8(u8::from(*ok));
                w.str(output);
                7
            }
        };
        *out = w.buf;
        tag
    }

    fn decode_body(tag: u8, payload: &[u8]) -> Result<Self, ProtoError> {
        let mut r = Reader { data: payload };
        let msg = match tag {
            1 => ServerMessage::HelloOk { version: r.u32()? },
            2 => ServerMessage::VersionMismatch {
                daemon: r.u32()?,
                client: r.u32()?,
            },
            3 => ServerMessage::Attached { session: r.str()? },
            4 => {
                let bytes = r.data.to_vec();
                r.data = &[];
                ServerMessage::Output(bytes)
            }
            5 => ServerMessage::Detached {
                reason: match r.u8()? {
                    0 => DetachReason::Requested,
                    1 => DetachReason::Replaced,
                    2 => DetachReason::ServerStopping,
                    _ => return Err(ProtoError::InvalidValue),
                },
                message: r.str()?,
            },
            6 => ServerMessage::SessionEnded { message: r.str()? },
            7 => ServerMessage::CommandResult {
                ok: match r.u8()? {
                    0 => false,
                    1 => true,
                    _ => return Err(ProtoError::InvalidValue),
                },
                output: r.str()?,
            },
            other => return Err(ProtoError::UnknownTag(other)),
        };
        r.finish()?;
        Ok(msg)
    }
}

/// Append one message, with its length prefix, to `out`.
///
/// Nothing is appended when the message is too large.
pub fn encode<M: Message>(msg: &M, out: &mut Vec<u8>) -> Result<(), ProtoError> {
    let start = out.len();
    out.extend_from_slice(&[0; 5]);
    let mut body = out.split_off(start + 5);
    let tag = msg.encode_body(&mut body);
    let len = body.len() + 1;
    if len > MAX_MESSAGE_LEN {
        out.truncate(start);
        return Err(ProtoError::TooLarge { len });
    }
    out[start..start + 4].copy_from_slice(&(len as u32).to_le_bytes());
    out[start + 4] = tag;
    out.extend_from_slice(&body);
    Ok(())
}

/// Encode a message into a fresh buffer.
pub fn to_bytes<M: Message>(msg: &M) -> Result<Vec<u8>, ProtoError> {
    let mut out = Vec::new();
    encode(msg, &mut out)?;
    Ok(out)
}

/// Reads messages out of a byte stream that arrives in arbitrary pieces.
///
/// After an error the stream cannot be trusted any more; the connection should be closed.
#[derive(Debug)]
pub struct Decoder<M> {
    buf: Vec<u8>,
    start: usize,
    failed: Option<ProtoError>,
    _marker: std::marker::PhantomData<M>,
}

impl<M: Message> Default for Decoder<M> {
    fn default() -> Self {
        Decoder {
            buf: Vec::new(),
            start: 0,
            failed: None,
            _marker: std::marker::PhantomData,
        }
    }
}

impl<M: Message> Decoder<M> {
    pub fn new() -> Self {
        Self::default()
    }

    /// Give the decoder bytes read from the connection.
    pub fn push(&mut self, bytes: &[u8]) {
        if self.failed.is_none() {
            self.buf.extend_from_slice(bytes);
        }
    }

    /// Bytes received and not yet turned into messages.
    pub fn buffered(&self) -> usize {
        self.buf.len() - self.start
    }

    /// The next complete message, `Ok(None)` when more bytes are needed.
    pub fn next_message(&mut self) -> Result<Option<M>, ProtoError> {
        if let Some(e) = &self.failed {
            return Err(e.clone());
        }
        let avail = &self.buf[self.start..];
        if avail.len() < 4 {
            return Ok(None);
        }
        let len = u32::from_le_bytes([avail[0], avail[1], avail[2], avail[3]]) as usize;
        // Judge the announced length at once, so a bogus one never makes us wait for (and
        // buffer) gigabytes.
        if len > MAX_MESSAGE_LEN {
            return Err(self.fail(ProtoError::TooLarge { len }));
        }
        if len == 0 {
            return Err(self.fail(ProtoError::Truncated));
        }
        if avail.len() < 4 + len {
            return Ok(None);
        }
        let tag = avail[4];
        let result = M::decode_body(tag, &avail[5..4 + len]);
        match result {
            Ok(msg) => {
                self.start += 4 + len;
                if self.start == self.buf.len() {
                    self.buf.clear();
                    self.start = 0;
                } else if self.start > 64 * 1024 {
                    self.buf.drain(..self.start);
                    self.start = 0;
                }
                Ok(Some(msg))
            }
            Err(e) => Err(self.fail(e)),
        }
    }

    fn fail(&mut self, e: ProtoError) -> ProtoError {
        self.failed = Some(e.clone());
        self.buf = Vec::new();
        self.start = 0;
        e
    }
}
