//! The daemon's log file: what happened (start, stop, sessions, clients, failures), never what a
//! pane showed or what was typed. It is size limited: when the file reaches the limit it becomes
//! `<name>.1`, replacing the one before, and a new file starts. So the log never takes more than
//! twice the limit on disk.

use std::fs::{self, File, OpenOptions};
use std::io::Write;
use std::os::unix::fs::OpenOptionsExt;
use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

/// A line longer than this is cut. Lines are built from fixed words and numbers, so this only
/// guards against a mistake.
const MAX_LINE: usize = 200;

pub struct Log {
    path: Option<PathBuf>,
    file: Option<File>,
    size: u64,
    limit: u64,
}

impl Log {
    /// A log that writes nowhere.
    pub fn disabled() -> Log {
        Log {
            path: None,
            file: None,
            size: 0,
            limit: 0,
        }
    }

    /// Open (and append to) the log file. A log that cannot be opened is a disabled log: the
    /// daemon never fails because of it.
    pub fn open(path: &Path, limit: u64) -> Log {
        let mut log = Log {
            path: Some(path.to_path_buf()),
            file: None,
            size: 0,
            limit,
        };
        log.reopen();
        log
    }

    fn reopen(&mut self) {
        let Some(path) = &self.path else { return };
        self.file = OpenOptions::new()
            .create(true)
            .append(true)
            .mode(0o600)
            .open(path)
            .ok();
        self.size = self
            .file
            .as_ref()
            .and_then(|f| f.metadata().ok())
            .map_or(0, |m| m.len());
    }

    pub fn line(&mut self, text: &str) {
        if self.file.is_none() {
            return;
        }
        let now = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap_or_default();
        let clean: String = text
            .chars()
            .map(|c| if c.is_control() { ' ' } else { c })
            .take(MAX_LINE)
            .collect();
        let line = format!("{}.{:03} {clean}\n", now.as_secs(), now.subsec_millis());
        if self.size + line.len() as u64 > self.limit {
            self.rotate();
        }
        if let Some(file) = &mut self.file
            && file.write_all(line.as_bytes()).is_ok()
        {
            self.size += line.len() as u64;
        }
    }

    fn rotate(&mut self) {
        let Some(path) = self.path.clone() else {
            return;
        };
        self.file = None;
        let mut old = path.clone().into_os_string();
        old.push(".1");
        let _ = fs::rename(&path, PathBuf::from(old));
        self.reopen();
    }
}
