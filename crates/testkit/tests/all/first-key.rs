//! The first key after a program that switched the keyboard protocol on has ended. A program in
//! the pane pushes and pops keyboard flags while keys come in, in every form the host terminal
//! sends them (with and without the text field, event types, lock-key modifier bits); the pane
//! must only ever receive what the key means: the letter itself, or a well-formed key report.

use arcade_testkit::{AttachOptions, Screened, TestDaemon};
use std::time::Duration;

/// The forms in which a host terminal sends the key `e`.
const FORMS: &[&[u8]] = &[
    b"e",
    b"\x1b[101u",
    b"\x1b[101;;101u",
    b"\x1b[101;1:1u",
    b"\x1b[101;1:1;101u",
    b"\x1b[101;65u",
    b"\x1b[101;129u",
    b"\x1b[101;193u",
    b"\x1b[101;193;101u",
];

#[test]
fn a_key_sent_while_the_keyboard_flags_change_reaches_the_pane_as_itself() {
    let daemon = TestDaemon::start();
    let mut client = daemon.attach(AttachOptions::new().size(100, 30)).unwrap();
    client.wait_for_prompt().unwrap();
    // Raw input, no echo, so that nothing but what the pane receives is shown, in `cat -v` form.
    client.type_line(
        "stty raw -echo; for f in 1 3 8 11 15 31 1 3 8 11 15 31; do printf \"\\033[>${f}u\"; sleep 0.03; \
         printf '\\033[<u'; sleep 0.03; done; printf 'LOOP-%s' END; exec cat -vt",
    );
    for _ in 0..40 {
        for form in FORMS {
            client.send(form);
            std::thread::sleep(Duration::from_millis(2));
        }
    }
    assert!(client.wait_until(Duration::from_secs(10), |c| {
        c.screen_text().contains("LOOP-END")
    }));
    // Let what was typed during the loop come out of `cat`.
    client.wait_until(Duration::from_millis(1500), |_| false);
    let all = client.screen_text();
    // What the pane received: from the marker to the status bar (the last row).
    let from = all.find("LOOP-END").map_or(0, |at| at + "LOOP-END".len());
    let mut lines: Vec<&str> = all[from..].lines().collect();
    lines.pop();
    let text: String = lines.concat();
    // Tokens: the letter, or a key report.
    let mut tokens = std::collections::BTreeMap::new();
    let mut rest = text.as_str();
    while !rest.is_empty() {
        let n = if let Some(r) = rest.strip_prefix("^[[") {
            let end = r.find('u').map_or(r.len(), |e| e + 1);
            3 + end
        } else {
            rest.chars().next().map_or(1, char::len_utf8)
        };
        *tokens.entry(rest[..n].to_string()).or_insert(0usize) += 1;
        rest = &rest[n..];
    }
    let odd: Vec<_> = tokens
        .keys()
        .filter(|t| *t != "e" && !(t.starts_with("^[[101") && t.ends_with('u')))
        .collect();
    assert!(odd.is_empty(), "odd tokens {odd:?}");
    // The daemon saw nothing that no key stands for.
    let (ok, stray) = daemon.command("keys.stray", &[]).unwrap();
    assert!(ok);
    assert_eq!(stray.lines().next(), Some("0"), "{stray}");
}

/// Answers and reports of the host terminal, whole and cut in two at a read boundary, right
/// before a key: the key still arrives as itself, once, and no stray byte is written.
#[test]
fn a_key_after_a_host_reply_or_a_cut_sequence_arrives_as_itself() {
    const REPLIES: &[&[u8]] = &[
        b"\x1b]11;rgb:1e1e/1e1e/2e2e\x1b\\",
        b"\x1b[?62;22c",
        b"\x1b[?0u",
        b"\x1b[12;40R",
        b"\x1b[I",
        b"\x1b[O",
        b"\x1b[200~",
        b"\x1b[201~",
    ];
    let daemon = TestDaemon::start();
    let mut client = daemon.attach(AttachOptions::new().size(100, 30)).unwrap();
    client.wait_for_prompt().unwrap();
    client.type_line("stty raw -echo; printf 'READY-%s' NOW; exec cat -vt");
    assert!(client.wait_until(Duration::from_secs(5), |c| {
        c.screen_text().contains("READY-NOW")
    }));
    let mut sent = 0;
    for reply in REPLIES {
        // Whole, then cut at every point.
        for cut in std::iter::once(0).chain(1..reply.len()) {
            if cut == 0 {
                client.send(reply);
            } else {
                client.send(&reply[..cut]);
                std::thread::sleep(Duration::from_millis(3));
                client.send(&reply[cut..]);
            }
            client.send(b"e");
            sent += 1;
            std::thread::sleep(Duration::from_millis(3));
        }
    }
    client.wait_until(Duration::from_millis(1500), |_| false);
    let all = client.screen_text();
    let from = all.find("READY-NOW").map_or(0, |at| at + 9);
    let mut lines: Vec<&str> = all[from..].lines().collect();
    lines.pop();
    let text: String = lines.concat();
    // Every key arrives; a program's own mode (`cat`) shows replies as they are.
    let letters = text.matches('e').count();
    assert!(letters >= sent, "{letters} of {sent} keys arrived:\n{text}");
    assert!(
        !text.contains("M-") && !text.contains("^R") && !text.contains("^?"),
        "{text}"
    );
    let (ok, stray) = daemon.command("keys.stray", &[]).unwrap();
    assert!(ok);
    assert_eq!(stray.lines().next(), Some("0"), "{stray}\n{text}");
}
