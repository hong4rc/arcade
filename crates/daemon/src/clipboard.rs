//! The buffer Arcade keeps for itself, and how a copy reaches the system clipboard.
//!
//! A copy is kept in the buffer and then goes out in exactly one way, chosen by the settings:
//! as an OSC 52 sequence for the client that copied, as the input of a command, or not at all.

use std::io::Write;
use std::process::{Command, Stdio};

use crate::config::{ClipboardConfig, ClipboardMode};

/// Above this many bytes after encoding, a terminal may quietly drop an OSC 52 sequence.
pub const OSC52_WARN_BYTES: usize = 100 * 1024;

#[derive(Default)]
pub struct Clipboard {
    /// What the last copy held; what `pane.paste` writes.
    buffer: Option<String>,
}

/// What a copy did, for the line on the status bar.
pub struct Sent {
    /// Bytes for the client's terminal (an OSC 52 sequence), when that is the way.
    pub osc52: Option<Vec<u8>>,
    pub message: String,
}

impl Clipboard {
    pub fn buffer(&self) -> Option<&str> {
        self.buffer.as_deref()
    }

    /// Put back what the last copy held.
    pub fn restore(&mut self, text: Option<String>) {
        self.buffer = text;
    }

    /// Keep `text` and send it the way the settings say.
    pub fn copy(&mut self, text: &str, config: &ClipboardConfig) -> Sent {
        self.buffer = Some(text.to_string());
        let chars = text.chars().count();
        match config.mode {
            ClipboardMode::Osc52 => {
                let encoded = base64(text.as_bytes());
                let mut message = format!("copied {chars} characters");
                if encoded.len() > OSC52_WARN_BYTES {
                    message.push_str("; the terminal may have ignored so much");
                }
                let mut bytes = b"\x1b]52;c;".to_vec();
                bytes.extend_from_slice(encoded.as_bytes());
                bytes.push(0x07);
                Sent {
                    osc52: Some(bytes),
                    message,
                }
            }
            ClipboardMode::Command => {
                let message = match config.command.as_deref() {
                    Some(command) => match pipe_to(command, text) {
                        Ok(()) => format!("copied {chars} characters"),
                        Err(e) => format!("could not run the clipboard command: {e}"),
                    },
                    None => "no clipboard command is set".to_string(),
                };
                Sent {
                    osc52: None,
                    message,
                }
            }
            ClipboardMode::Off => Sent {
                osc52: None,
                message: format!("kept {chars} characters in the buffer"),
            },
        }
    }
}

/// Run `command` with `text` as its input. The call returns at once: a helper thread feeds
/// the command and waits for it, so a slow command never holds up the daemon.
fn pipe_to(command: &str, text: &str) -> std::io::Result<()> {
    let mut child = Command::new("sh")
        .arg("-c")
        .arg(command)
        .stdin(Stdio::piped())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()?;
    let mut stdin = child.stdin.take();
    let text = text.to_string();
    std::thread::spawn(move || {
        if let Some(stdin) = stdin.as_mut() {
            let _ = stdin.write_all(text.as_bytes());
        }
        drop(stdin);
        let _ = child.wait();
    });
    Ok(())
}

const ALPHABET: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";

fn base64(bytes: &[u8]) -> String {
    let mut out = String::with_capacity(bytes.len().div_ceil(3) * 4);
    for chunk in bytes.chunks(3) {
        let b = [
            chunk[0],
            chunk.get(1).copied().unwrap_or(0),
            chunk.get(2).copied().unwrap_or(0),
        ];
        let n = (u32::from(b[0]) << 16) | (u32::from(b[1]) << 8) | u32::from(b[2]);
        out.push(char::from(ALPHABET[(n >> 18) as usize & 63]));
        out.push(char::from(ALPHABET[(n >> 12) as usize & 63]));
        out.push(if chunk.len() > 1 {
            char::from(ALPHABET[(n >> 6) as usize & 63])
        } else {
            '='
        });
        out.push(if chunk.len() > 2 {
            char::from(ALPHABET[n as usize & 63])
        } else {
            '='
        });
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn config(mode: ClipboardMode) -> ClipboardConfig {
        ClipboardConfig {
            mode,
            command: None,
            allow_pane_write: true,
        }
    }

    #[test]
    fn base64_matches_the_known_answers() {
        assert_eq!(base64(b""), "");
        assert_eq!(base64(b"f"), "Zg==");
        assert_eq!(base64(b"fo"), "Zm8=");
        assert_eq!(base64(b"foo"), "Zm9v");
        assert_eq!(base64(b"foobar"), "Zm9vYmFy");
        assert_eq!(base64("Việt".as_bytes()), "Vmnhu4d0");
    }

    #[test]
    fn osc52_mode_gives_the_sequence_and_keeps_the_buffer() {
        let mut clipboard = Clipboard::default();
        let sent = clipboard.copy("hi", &config(ClipboardMode::Osc52));
        assert_eq!(sent.osc52.unwrap(), b"\x1b]52;c;aGk=\x07");
        assert_eq!(sent.message, "copied 2 characters");
        assert_eq!(clipboard.buffer(), Some("hi"));
    }

    #[test]
    fn a_very_large_copy_comes_with_a_warning() {
        let mut clipboard = Clipboard::default();
        let sent = clipboard.copy(&"x".repeat(100_000), &config(ClipboardMode::Osc52));
        assert!(
            sent.message.contains("may have ignored"),
            "{}",
            sent.message
        );
        let sent = clipboard.copy("small", &config(ClipboardMode::Osc52));
        assert!(!sent.message.contains("ignored"));
    }

    #[test]
    fn off_mode_only_keeps_the_buffer() {
        let mut clipboard = Clipboard::default();
        let sent = clipboard.copy("abc", &config(ClipboardMode::Off));
        assert!(sent.osc52.is_none());
        assert_eq!(clipboard.buffer(), Some("abc"));
        assert!(sent.message.contains("kept 3"));
    }

    #[test]
    fn command_mode_feeds_the_command_once_and_sends_no_sequence() {
        let dir = std::env::temp_dir().join(format!("arcade-clip-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let file = dir.join("out");
        let mut settings = config(ClipboardMode::Command);
        settings.command = Some(format!("cat >> '{}'", file.display()));
        let mut clipboard = Clipboard::default();
        let sent = clipboard.copy("once", &settings);
        assert!(sent.osc52.is_none());
        assert_eq!(sent.message, "copied 4 characters");
        for _ in 0..100 {
            if std::fs::read_to_string(&file).is_ok_and(|t| t == "once") {
                break;
            }
            std::thread::sleep(std::time::Duration::from_millis(20));
        }
        assert_eq!(std::fs::read_to_string(&file).unwrap(), "once");
        let _ = std::fs::remove_dir_all(&dir);
    }
}
