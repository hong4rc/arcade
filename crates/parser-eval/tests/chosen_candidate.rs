//! Locks in the result that decision 001 rests on: the chosen library, libghostty-vt, passes
//! every correctness case. If an upgrade of the library breaks one, this test fails.

use parser_eval::cases;

#[test]
fn libghostty_vt_passes_every_required_case() {
    let mut failures = Vec::new();
    for case in cases::all() {
        // This case documents the library default (grapheme clustering off until the daemon
        // turns it on); it is informational, not a requirement.
        if case.id == "emoji-joined-family-default-mode" {
            continue;
        }
        if let Some(message) = cases::run_case("libghostty-vt", &case).failure {
            failures.push(format!("{}: {message}", case.id));
        }
    }
    assert!(
        failures.is_empty(),
        "failing cases:\n{}",
        failures.join("\n")
    );
}

#[test]
fn grapheme_clustering_is_off_by_default() {
    // The daemon must turn mode 2027 on itself. If this starts failing, the library default
    // changed and the daemon no longer needs to.
    let case = cases::all()
        .into_iter()
        .find(|c| c.id == "emoji-joined-family-default-mode")
        .unwrap();
    assert!(cases::run_case("libghostty-vt", &case).failure.is_some());
}

#[test]
fn endless_osc_string_does_not_grow_memory() {
    let mut candidate = parser_eval::candidate::make("libghostty-vt", 120, 40, 1000);
    let before = parser_eval::procstat::memory_bytes();
    candidate.feed(b"\x1b]0;");
    let block = vec![b'x'; 1024 * 1024];
    for _ in 0..64 {
        candidate.feed(&block);
    }
    let growth = parser_eval::procstat::memory_bytes().saturating_sub(before);
    assert!(growth < 8 * 1024 * 1024, "memory grew by {growth} bytes");
    candidate.feed(b"\x07OK");
    assert_eq!(candidate.row_text(0), "OK");
}
