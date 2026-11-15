//! Links in a pane: finding an address in text, deciding which ones may be opened, and opening
//! one with the system's opener.

use std::process::{Command, Stdio};

/// The kinds of address that are opened. Anything else is refused.
const SCHEMES: [&str; 3] = ["http://", "https://", "file://"];

/// Whether `uri` is one of the kinds of address that may be opened.
pub fn may_open(uri: &str) -> bool {
    let lower = uri.get(..8).unwrap_or(uri).to_ascii_lowercase();
    SCHEMES.iter().any(|s| lower.starts_with(s)) && !uri.chars().any(char::is_control)
}

/// The address in `text` that covers the byte offset `at`, if there is one: it starts with a
/// scheme of [`SCHEMES`] and goes on to the next blank or one of `<>"` or a backquote, minus
/// the punctuation that ends a sentence rather than an address.
pub fn find_address(text: &str, at: usize) -> Option<String> {
    let lower = text.to_ascii_lowercase();
    let mut from = 0;
    while from < text.len() {
        let start = SCHEMES
            .iter()
            .filter_map(|s| lower[from..].find(s).map(|i| i + from))
            .min()?;
        let end = text[start..]
            .find(|c: char| c.is_whitespace() || c.is_control() || "<>\"`".contains(c))
            .map_or(text.len(), |i| start + i);
        let address = trim_end(&text[start..end]);
        if start <= at && at < start + address.len() {
            return Some(address.to_string());
        }
        if start >= at {
            return None;
        }
        from = end.max(start + 1);
    }
    None
}

/// Cut the punctuation off the end of an address: `. , ; : ! ? ' "` always, and a closing
/// bracket only when the address has no opening one for it (so `/wiki/Rust_(language)` keeps
/// its bracket and `(see https://a.example/x)` loses it).
fn trim_end(address: &str) -> &str {
    let mut end = address.len();
    while let Some(last) = address[..end].chars().next_back() {
        let cut = match last {
            '.' | ',' | ';' | ':' | '!' | '?' | '\'' | '"' => true,
            ')' | ']' | '}' => {
                let open = match last {
                    ')' => '(',
                    ']' => '[',
                    _ => '{',
                };
                let slice = &address[..end];
                slice.matches(last).count() > slice.matches(open).count()
            }
            _ => false,
        };
        if !cut {
            break;
        }
        end -= last.len_utf8();
    }
    &address[..end]
}

/// The opener when the configuration names none.
pub fn default_opener() -> Vec<String> {
    let program = if cfg!(target_os = "macos") {
        "open"
    } else {
        "xdg-open"
    };
    vec![program.to_string()]
}

/// Run the opener with the address as one argument, never through a shell. The call returns at
/// once; a helper thread waits for the child so that no zombie is left.
pub fn open(opener: &[String], uri: &str) -> std::io::Result<()> {
    let (program, args) = opener
        .split_first()
        .ok_or_else(|| std::io::Error::other("no opener"))?;
    let mut child = Command::new(program)
        .args(args)
        .arg(uri)
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()?;
    std::thread::spawn(move || {
        let _ = child.wait();
    });
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn at(text: &str, needle: &str) -> Option<String> {
        find_address(text, text.find(needle).unwrap())
    }

    #[test]
    fn a_plain_address_is_found_from_any_cell_of_it() {
        let text = "see https://example.com/a?b=1 now";
        assert_eq!(
            at(text, "https").as_deref(),
            Some("https://example.com/a?b=1")
        );
        assert_eq!(
            at(text, "a?b").as_deref(),
            Some("https://example.com/a?b=1")
        );
        assert_eq!(at(text, "see"), None);
        assert_eq!(at(text, "now"), None);
    }

    #[test]
    fn trailing_punctuation_is_not_part_of_the_address() {
        for (text, want) in [
            ("go to https://a.example/x.", "https://a.example/x"),
            ("https://a.example/x, then", "https://a.example/x"),
            ("(https://a.example/x)", "https://a.example/x"),
            ("[https://a.example/x]", "https://a.example/x"),
            ("<https://a.example/x>", "https://a.example/x"),
            ("\"https://a.example/x\"", "https://a.example/x"),
            ("'https://a.example/x'", "https://a.example/x"),
            ("https://a.example/x?!", "https://a.example/x"),
            (
                "https://a.example/wiki/Rust_(language)",
                "https://a.example/wiki/Rust_(language)",
            ),
            (
                "(see https://a.example/wiki/A_(b))",
                "https://a.example/wiki/A_(b)",
            ),
        ] {
            assert_eq!(at(text, "http").as_deref(), Some(want), "{text}");
        }
    }

    #[test]
    fn the_three_schemes_are_found_in_any_case_and_others_are_not() {
        assert!(find_address("HTTP://A.EXAMPLE/", 0).is_some());
        assert!(find_address("file:///tmp/a.txt", 3).is_some());
        assert_eq!(find_address("ftp://a.example/x", 0), None);
        assert_eq!(find_address("mailto:a@b.example", 0), None);
    }

    #[test]
    fn with_two_addresses_the_one_under_the_cursor_is_found() {
        let text = "https://a.example/1 https://b.example/2";
        assert_eq!(
            at(text, "b.example").as_deref(),
            Some("https://b.example/2")
        );
        assert_eq!(
            at(text, "a.example").as_deref(),
            Some("https://a.example/1")
        );
    }

    #[test]
    fn only_the_three_schemes_may_be_opened() {
        assert!(may_open("https://a.example/"));
        assert!(may_open("HTTP://a.example/"));
        assert!(may_open("file:///tmp/x"));
        for bad in [
            "ftp://a.example/",
            "javascript:alert(1)",
            "mailto:a@b.example",
            "ssh://host",
            "-o http://x",
            "x-https://a",
            "",
            "https://a\x01",
        ] {
            assert!(!may_open(bad), "{bad:?}");
        }
    }
}
