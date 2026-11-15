//! The shell prompt marks (OSC 133) the library records, read on demand.

use arcade_term::{Term, TermConfig};

/// A shell session of two commands: prompt, typed command, output.
fn two_commands() -> Term {
    let mut config = TermConfig::new(30, 4);
    config.scrollback_lines = 100;
    let mut term = Term::new(config).unwrap();
    let a = "\x1b]133;A\x1b\\";
    let b = "\x1b]133;B\x1b\\";
    let c = "\x1b]133;C\x1b\\";
    let d = "\x1b]133;D;0\x1b\\";
    term.feed(format!("{a}$ {b}ls\r\n{c}one\r\ntwo\r\n{d}").as_bytes());
    term.feed(format!("{a}$ {b}pwd\r\n{c}/home\r\n{d}{a}$ {b}").as_bytes());
    term
}

fn kinds(term: &Term) -> Vec<(bool, bool)> {
    (0..term.total_rows())
        .map(|y| (term.row_is_prompt(y), term.row_is_prompt_or_input(y)))
        .collect()
}

#[test]
fn prompt_rows_and_input_rows_are_told_from_output_rows() {
    let term = two_commands();
    // Rows: `$ ls`, `one`, `two`, `$ pwd`, `/home`, `$ ` (the screen is four rows, so the
    // first two have gone to the scrollback).
    assert_eq!(term.total_rows(), 6);
    assert_eq!(
        kinds(&term),
        [
            (true, true),
            (false, false),
            (false, false),
            (true, true),
            (false, false),
            (true, true),
        ]
    );
}

#[test]
fn a_pane_without_marks_has_no_prompt_rows() {
    let mut term = Term::new(TermConfig::new(30, 4)).unwrap();
    term.feed(b"$ ls\r\none\r\ntwo\r\n$ ");
    assert!(kinds(&term).iter().all(|k| *k == (false, false)));
}

#[test]
fn rows_out_of_range_are_not_prompts() {
    let term = two_commands();
    assert!(!term.row_is_prompt(500));
    assert!(!term.row_is_prompt_or_input(500));
}
