//! The link runs a frame reports for rows that hold OSC 8 links.

use arcade_term::{LINK_MAX_LEN, LinkSpan, Term, TermConfig, link_is_safe};

fn spans(data: &str) -> Vec<LinkSpan> {
    let mut term = Term::new(TermConfig::new(30, 4)).unwrap();
    term.feed(data.as_bytes());
    let frame = term.snapshot(true).unwrap();
    frame.rows.into_iter().flat_map(|r| r.links).collect()
}

#[test]
fn a_row_reports_the_run_of_columns_of_its_link() {
    let got = spans("ab\x1b]8;;https://x.example/\x1b\\cde\x1b]8;;\x1b\\f");
    assert_eq!(
        got,
        [LinkSpan {
            from: 2,
            to: 5,
            uri: "https://x.example/".into()
        }]
    );
}

#[test]
fn two_different_links_next_to_each_other_are_two_runs() {
    let got = spans("\x1b]8;;http://a/\x1b\\aa\x1b]8;;http://b/\x1b\\bb\x1b]8;;\x1b\\");
    assert_eq!(got.len(), 2);
    assert_eq!(
        (got[0].from, got[0].to, got[1].from, got[1].to),
        (0, 2, 2, 4)
    );
}

#[test]
fn a_screen_without_links_reports_none() {
    assert!(spans("plain \x1b[1mbold\x1b[0m text\r\nmore").is_empty());
}

#[test]
fn only_plain_ascii_addresses_of_bounded_length_are_safe() {
    assert!(link_is_safe("https://a.example/x?y=1;z"));
    assert!(!link_is_safe(""));
    assert!(!link_is_safe("http://a b"));
    assert!(!link_is_safe("http://a/\u{e9}"));
    assert!(!link_is_safe("http://a/\x01"));
    assert!(!link_is_safe(&"a".repeat(LINK_MAX_LEN + 1)));
}
