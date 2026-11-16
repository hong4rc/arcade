//! The key capture, a switch for finding out what a pane was really sent: when it is on for a
//! pane, every byte read from the host terminal while that pane has the focus, and every byte
//! written to that pane, goes to a file with the time it happened. It is off for every pane until
//! the command `pane.capture-keys` turns it on, and the file records everything typed, passwords
//! included, so it can be read by its owner only.

use std::fs::{File, OpenOptions};
use std::io::{self, Write};
use std::os::unix::fs::OpenOptionsExt;
use std::path::{Path, PathBuf};
use std::time::{Instant, SystemTime, UNIX_EPOCH};

/// Where the bytes came from or went to.
#[derive(Clone, Copy)]
pub enum Direction {
    /// Read from the host terminal.
    Host,
    /// Written to the pane's terminal.
    Pane,
}

pub struct Capture {
    file: File,
    path: PathBuf,
    started: Instant,
}

impl Capture {
    /// Make the file for `pane` in `dir` (the private directory of the socket). Nothing is
    /// overwritten, and nobody but the owner can read it.
    pub fn start(dir: &Path, pane: u64) -> io::Result<Self> {
        let stamp = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map_or(0, |d| d.as_secs());
        let path = dir.join(format!("capture-pane{pane}-{stamp}.log"));
        let mut file = OpenOptions::new()
            .write(true)
            .create_new(true)
            .mode(0o600)
            .open(&path)?;
        writeln!(
            file,
            "# arcade key capture of pane {pane}: seconds since the start, host (read from the host terminal) or pane (written to the pane), the bytes in hexadecimal"
        )?;
        Ok(Self {
            file,
            path,
            started: Instant::now(),
        })
    }

    pub fn path(&self) -> &Path {
        &self.path
    }

    /// One line per piece of bytes. A failed write ends nothing: the capture is for looking, and
    /// must never get in the way of the pane.
    pub fn record(&mut self, direction: Direction, bytes: &[u8]) {
        if bytes.is_empty() {
            return;
        }
        let elapsed = self.started.elapsed();
        let who = match direction {
            Direction::Host => "host",
            Direction::Pane => "pane",
        };
        let mut line = format!("{:.6} {who}", elapsed.as_secs_f64());
        for b in bytes {
            line.push_str(&format!(" {b:02x}"));
        }
        line.push('\n');
        let _ = self.file.write_all(line.as_bytes());
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::os::unix::fs::PermissionsExt;

    #[test]
    fn the_file_is_private_and_holds_exact_bytes_with_times() {
        let dir = std::env::temp_dir().join(format!("arcade-capture-test-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let mut capture = Capture::start(&dir, 7).unwrap();
        capture.record(Direction::Host, b"\xff\x1b[A");
        capture.record(Direction::Pane, b"a");
        capture.record(Direction::Pane, b"");
        let path = capture.path().to_path_buf();
        let mode = std::fs::metadata(&path).unwrap().permissions().mode();
        assert_eq!(mode & 0o777, 0o600);
        let text = std::fs::read_to_string(&path).unwrap();
        let lines: Vec<&str> = text.lines().skip(1).collect();
        assert_eq!(lines.len(), 2, "{text}");
        assert!(lines[0].ends_with(" host ff 1b 5b 41"), "{}", lines[0]);
        assert!(lines[1].ends_with(" pane 61"), "{}", lines[1]);
        assert!(lines[0].split(' ').next().unwrap().parse::<f64>().is_ok());
        std::fs::remove_dir_all(&dir).unwrap();
    }
}
