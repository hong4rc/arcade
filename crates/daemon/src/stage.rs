//! Timestamps at fixed points along the key path and the output path, for measuring where the
//! time goes (cargo feature `stage-timing`, off by default; with it off this file is not built
//! and the marks in the code expand to nothing).
//!
//! A mark is a point name, the time from the monotonic clock (the same clock in every process),
//! and how many `read`, `write` and `poll` calls the process had made by then. Marks go into a
//! list reserved at the start, so taking one does not allocate. The list is written to a file
//! when the process ends; `arcade-bench --tests stages` reads the files and cuts them into the
//! stretches between points.

use std::cell::RefCell;
use std::io::Write;
use std::path::Path;

/// Marks kept at most; later ones are dropped.
const CAPACITY: usize = 400_000;

/// The points, in the order a key and then its echo pass them.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Point {
    /// The client has read the key from its terminal.
    ClientKeyRead = 1,
    /// The client has written the key message to the socket.
    ClientKeySent,
    /// The daemon came back from waiting for events (every wakeup; also the start of output).
    DaemonWake,
    /// The daemon has read what the socket held.
    DaemonSocketRead,
    /// The daemon has decoded the messages.
    DaemonDecoded,
    /// The key was found in the tables and goes to the pane next.
    DaemonKeyHandled,
    /// The key has been written to the pane's terminal.
    DaemonPtyWritten,
    /// The daemon has read output from a pane's terminal.
    DaemonPtyRead,
    /// The terminal emulation has taken that output in.
    DaemonFed,
    /// The panes' screens have been read out (`snapshot`).
    DaemonSnapshot,
    /// The window's grid is composed: dividers, status bar, list.
    DaemonComposed,
    /// The difference to the last frame is found and encoded.
    DaemonEncoded,
    /// The daemon has written the frame to the socket.
    DaemonSocketWritten,
    /// The client came back from waiting for events.
    ClientWake,
    /// The client has written the frame to its terminal.
    ClientStdoutWritten,
    /// The daemon begins to build the frame (after the work of the turn that came before it).
    DaemonFrameStart,
    /// The daemon is about to make its first read from a client socket in this turn.
    DaemonBeforeRead,
    /// The client is about to write the key message to the socket.
    ClientBeforeWrite,
}

#[derive(Clone, Copy)]
pub enum Call {
    Read = 0,
    Write = 1,
    Poll = 2,
}

struct Record {
    point: u8,
    ns: u64,
    calls: [u32; 3],
}

struct Log {
    records: Vec<Record>,
    calls: [u32; 3],
}

thread_local! {
    static LOG: RefCell<Log> = RefCell::new(Log {
        records: Vec::with_capacity(CAPACITY),
        calls: [0; 3],
    });
}

fn now_ns() -> u64 {
    let mut ts = libc::timespec {
        tv_sec: 0,
        tv_nsec: 0,
    };
    // SAFETY: clock_gettime fills the struct it is given.
    unsafe { libc::clock_gettime(libc::CLOCK_MONOTONIC, &mut ts) };
    ts.tv_sec as u64 * 1_000_000_000 + ts.tv_nsec as u64
}

pub fn mark(point: Point) {
    let ns = now_ns();
    LOG.with(|log| {
        let mut log = log.borrow_mut();
        if log.records.len() < CAPACITY {
            let calls = log.calls;
            log.records.push(Record {
                point: point as u8,
                ns,
                calls,
            });
        }
    });
}

pub fn count(call: Call) {
    LOG.with(|log| log.borrow_mut().calls[call as usize] += 1);
}

/// Write the marks to `path`, one per line: point number, nanoseconds, then the reads, writes
/// and polls so far.
pub fn dump(path: &Path) {
    LOG.with(|log| {
        let log = log.borrow();
        let mut text = String::new();
        for r in &log.records {
            text.push_str(&format!(
                "{} {} {} {} {}\n",
                r.point, r.ns, r.calls[0], r.calls[1], r.calls[2]
            ));
        }
        if let Ok(mut file) = std::fs::File::create(path) {
            let _ = file.write_all(text.as_bytes());
        }
    });
}
