//! The keyboard flags a program pushed, and what a terminal rebuilt from a saved state (a daemon
//! that upgraded itself) does when the program pops them again.

use arcade_term::{Term, TermConfig};

fn new() -> Term {
    Term::new(TermConfig::new(80, 24)).unwrap()
}

fn carried(t: &mut Term) -> Term {
    let state = t.save_state().unwrap();
    let mut b = new();
    b.load_state(&state, None);
    b
}

fn flags(t: &Term) -> u8 {
    t.key_modes().kitty_flags
}

#[test]
fn flags_and_their_stack_survive_a_carry_and_pops_end_where_they_would_have() {
    // Programs push once, twice, or set directly; every moment of the program's life is a moment
    // an upgrade can happen at.
    let programs: &[(&str, &[&[u8]])] = &[
        ("one push", &[b"\x1b[>1u"]),
        ("two pushes", &[b"\x1b[>1u", b"\x1b[>3u"]),
        ("push, set", &[b"\x1b[>1u", b"\x1b[=8;1u"]),
        ("set only", &[b"\x1b[=3;1u"]),
        (
            "push on the alternate screen",
            &[b"\x1b[?1049h", b"\x1b[>1u"],
        ),
        ("two screens", &[b"\x1b[>1u", b"\x1b[?1049h", b"\x1b[>5u"]),
    ];
    let mut problems = Vec::new();
    for (name, steps) in programs {
        let pops = steps
            .iter()
            .filter(|s| s.ends_with(b"u") && s.starts_with(b"\x1b[>"))
            .count();
        let mut original = new();
        for s in steps.iter() {
            original.feed(s);
        }
        let mut copy = carried(&mut original);
        if flags(&copy) != flags(&original) {
            problems.push(format!(
                "{name}: flags {} became {}",
                flags(&original),
                flags(&copy)
            ));
        }
        // The program ends: it leaves the alternate screen and pops what it pushed.
        for t in [&mut original, &mut copy] {
            t.feed(b"\x1b[?1049l");
            for _ in 0..pops {
                t.feed(b"\x1b[<u");
            }
        }
        if flags(&copy) != flags(&original) {
            problems.push(format!(
                "{name}: after the program ended the flags were {} (a terminal never carried: {})",
                flags(&copy),
                flags(&original)
            ));
        }
    }
    assert!(problems.is_empty(), "{problems:#?}");
}
