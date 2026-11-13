//! User interface scenario 15: no color and no drawn symbol outside the theme.
//!
//! Every source file of the daemon but `theme.rs` is read, and none may write a color value of
//! its own or a line-drawing or private-use character.

use std::path::{Path, PathBuf};

fn rust_files(dir: &Path, out: &mut Vec<PathBuf>) {
    for entry in std::fs::read_dir(dir).unwrap() {
        let path = entry.unwrap().path();
        if path.is_dir() {
            rust_files(&path, out);
        } else if path.extension().is_some_and(|e| e == "rs") {
            out.push(path);
        }
    }
}

/// Characters that draw lines, blocks and shapes, and those a patched font puts its icons at.
fn is_symbol(c: char) -> bool {
    matches!(c as u32,
        0x2500..=0x25FF | 0x2600..=0x27BF | 0xE000..=0xF8FF | 0xF0000..=0xFFFFD)
}

#[test]
fn the_theme_is_the_only_place_that_names_a_color_or_a_symbol() {
    let src = Path::new(env!("CARGO_MANIFEST_DIR")).join("src");
    let mut files = Vec::new();
    rust_files(&src, &mut files);
    assert!(files.len() > 8, "found only {files:?}");
    let mut problems = Vec::new();
    for file in files {
        if file.file_name().is_some_and(|n| n == "theme.rs") {
            continue;
        }
        let text = std::fs::read_to_string(&file).unwrap();
        for (number, line) in text.lines().enumerate() {
            let code = line.trim_start();
            if code.starts_with("//") {
                continue;
            }
            let at = format!("{}:{}", file.display(), number + 1);
            // A color given as a number or as red, green and blue (a color that is only passed
            // on, such as `Color::Indexed(n)`, is not a choice of color).
            for pattern in ["Indexed(", "Rgb("] {
                let mut rest = code;
                while let Some(i) = rest.find(pattern) {
                    let after = &rest[i + pattern.len()..];
                    if after.starts_with(|c: char| c.is_ascii_digit()) {
                        problems.push(format!("{at}: a color value in `{code}`"));
                    }
                    rest = after;
                }
            }
            if code.chars().any(is_symbol) {
                problems.push(format!("{at}: a drawn symbol in `{code}`"));
            }
        }
    }
    assert!(problems.is_empty(), "{}", problems.join("\n"));
}

#[test]
fn the_scan_would_notice_a_stray_color_or_symbol() {
    assert!(is_symbol('│') && is_symbol('─') && is_symbol('\u{e0b0}'));
    assert!(!is_symbol('a') && !is_symbol('é') && !is_symbol('汉'));
}
