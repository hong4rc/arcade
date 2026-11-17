//! What the terminal writes back to the program (answers to its questions) is always whole and
//! well formed, whatever was asked and in whatever state the program left the terminal: never a
//! lone control byte, a byte that is not text, or a cut sequence.

use arcade_term::{Event, Term, TermConfig};
use arcade_workload::Rng;

fn replies(t: &mut Term, input: &[u8]) -> Vec<Vec<u8>> {
    t.drain_events();
    t.feed(input);
    t.drain_events()
        .into_iter()
        .filter_map(|e| match e {
            Event::Reply(b) => Some(b),
            _ => None,
        })
        .collect()
}

fn well_formed(r: &[u8]) -> bool {
    let Ok(text) = std::str::from_utf8(r) else {
        return false;
    };
    if !text.starts_with('\x1b') || r.len() < 3 {
        return false;
    }
    let end = *r.last().unwrap();
    // CSI/SS3 end with a final byte; OSC and DCS end with BEL or ST.
    match r[1] {
        b'[' | b'O' => (0x40..=0x7e).contains(&end),
        b']' | b'P' => end == 0x07 || r.ends_with(b"\x1b\\"),
        _ => false,
    }
}

const QUERIES: &[&[u8]] = &[
    b"\x1b[c",
    b"\x1b[>c",
    b"\x1b[=c",
    b"\x1b[5n",
    b"\x1b[6n",
    b"\x1b[?6n",
    b"\x1b[?u",
    b"\x1b[>q",
    b"\x1b[18t",
    b"\x1b[14t",
    b"\x1b[16t",
    b"\x1b[?996n",
    b"\x1b]10;?\x07",
    b"\x1b]11;?\x1b\\",
    b"\x1b]4;1;?\x07",
    b"\x1bP$qm\x1b\\",
    b"\x1bP$qr\x1b\\",
    b"\x1b[?2004$p",
    b"\x1b[?1004$p",
    b"\x1b[?2026$p",
];

/// Switches of modes a program turns on and off, which an answer may depend on.
const SWITCHES: &[&[u8]] = &[
    b"\x1b[>1u",
    b"\x1b[>31u",
    b"\x1b[<u",
    b"\x1b[<3u",
    b"\x1b[=5;2u",
    b"\x1b[?1004h",
    b"\x1b[?1004l",
    b"\x1b[?2004h",
    b"\x1b[?2004l",
    b"\x1b[?1049h",
    b"\x1b[?1049l",
    b"\x1b[?1000h",
    b"\x1b[?1006h",
    b"\x1b[?1000l",
    b"\x1b[?2031h",
    b"\x1b[?2031l",
    b"\x1b[?1h",
    b"\x1b[?1l",
    b"\x1b[2J",
    b"\x1bc",
];

#[test]
fn every_answer_is_whole_whatever_state_the_program_left() {
    let mut t = Term::new(TermConfig::new(80, 24)).unwrap();
    let mut rng = Rng::new(0x5eed);
    let mut asked = 0;
    for _ in 0..3000 {
        let mut input = Vec::new();
        for _ in 0..(rng.next_u64() % 4) {
            input.extend_from_slice(SWITCHES[(rng.next_u64() as usize) % SWITCHES.len()]);
        }
        let q = QUERIES[(rng.next_u64() as usize) % QUERIES.len()];
        input.extend_from_slice(q);
        // Sometimes the question is cut in two across two reads.
        let cut = (rng.next_u64() as usize) % (q.len() + 1);
        let at = input.len() - q.len() + cut;
        let (a, b) = input.split_at(at);
        let mut got = replies(&mut t, a);
        got.extend(replies(&mut t, b));
        for r in got {
            asked += 1;
            assert!(
                well_formed(&r),
                "reply {:02x?} to {:?} after {:?}",
                r,
                String::from_utf8_lossy(q),
                String::from_utf8_lossy(&input)
            );
        }
    }
    assert!(asked > 1000, "only {asked} answers were checked");
}
