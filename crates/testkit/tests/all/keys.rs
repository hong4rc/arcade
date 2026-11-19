//! Keys from the host terminal reaching the program in a pane: every form a terminal can send
//! comes out as the bytes the program expects, a lone Esc is passed on after its short wait, and
//! pasted text arrives whole.

use std::time::{Duration, Instant};

use arcade_testkit::{Screened, TestClient, TestDaemon, WAIT};

/// Start `command` in the pane, with the terminal in raw mode so that every byte reaches it
/// untouched, and wait until it is running.
fn raw_pane(daemon: &TestDaemon, command: &str) -> TestClient {
    let mut client = daemon.attach_shell();
    client.wait_for_prompt().unwrap();
    client.type_line(&format!("stty raw -echo; echo ready; {command}"));
    client.wait_for_row("ready").unwrap();
    client
}

/// Each host form with what `cat -v` shows for the bytes the pane receives.
const FORMS: &[(&[u8], &str)] = &[
    (b"a", "a"),
    (b"Z", "Z"),
    (b"\x1b[27u", "^["),
    (b"\x1b[109;5u", "^M"),
    (b"\x1b[105;5u", "^I"),
    (b"\r", "^M"),
    (b"\t", "^I"),
    (b"\x1b[A", "^[[A"),
    (b"\x1bOB", "^[[B"),
    (b"\x1b[1;5C", "^[[1;5C"),
    (b"\x1b[3~", "^[[3~"),
    (b"\x1b[5~", "^[[5~"),
    (b"\x1bx", "^[x"),
    (b"\x03", "^C"),
    (b"\x7f", "^?"),
    (b"\x1b[97;3u", "^[a"),
    (b"\x1b[Z", "^[[Z"),
    (b"\x1b[15~", "^[[15~"),
];

/// These tests send every kind of key, Ctrl+m among them, so the prefix must be one they do not use.
fn daemon_with_a_free_prefix() -> TestDaemon {
    TestDaemon::start_with(|c| c.config.keys.prefix = arcade_daemon::parse_key("f12").unwrap())
}

#[test]
fn every_host_form_reaches_the_pane_as_its_legacy_bytes() {
    let daemon = daemon_with_a_free_prefix();
    let mut client = raw_pane(&daemon, "cat -vt");
    let mut expected = String::new();
    for (bytes, shown) in FORMS {
        client.send(bytes);
        client.send(b"|");
        expected.push_str(shown);
        expected.push('|');
    }
    let ok = client.wait_until(WAIT, |c| {
        c.content_rows().iter().any(|r| r.trim_start() == expected)
    });
    assert!(
        ok,
        "expected {expected:?} on the screen:\n{}",
        client.screen_text()
    );
}

#[test]
fn a_lone_esc_reaches_the_pane_after_its_short_wait() {
    let daemon = daemon_with_a_free_prefix();
    let mut client = raw_pane(&daemon, "cat -vt");
    let sent = Instant::now();
    client.send(b"\x1b");
    assert!(client.wait_until(WAIT, |c| {
        c.content_rows().iter().any(|r| r.trim_start() == "^[")
    }));
    // It waits for the rest of a sequence, but not for long.
    assert!(sent.elapsed() < Duration::from_millis(500));
    // Esc and a bracket arriving in two pieces within the wait is one sequence, not two keys.
    client.send(b"\x1b");
    client.send(b"[A");
    assert!(client.wait_until(WAIT, |c| {
        c.content_rows().iter().any(|r| r.trim_start() == "^[^[[A")
    }));
}

/// A paste of this many bytes, with bytes that look like escape sequences in it.
fn paste_payload(len: usize) -> Vec<u8> {
    let pieces: [&[u8]; 5] = [
        b"line of text\r",
        b"\x1b[A",
        b"\xc3\xa9\xe6\xbc\xa2 ",
        b"x\x1b",
        b"\n",
    ];
    let mut out = Vec::with_capacity(len + 16);
    let mut i = 0usize;
    while out.len() < len {
        out.extend_from_slice(pieces[i % pieces.len()]);
        i += 1 + i % 3;
    }
    out.truncate(len);
    out
}

/// Paste `payload` into a pane that reads exactly the bytes it should get into a file, and
/// compare the file.
fn paste_and_compare(bracketed: bool, reader_starts_after: &str) {
    let daemon = daemon_with_a_free_prefix();
    let payload = paste_payload(1024 * 1024);
    let file = daemon.dir().join("pasted.out");
    let markers: &[u8] = if bracketed { b"\x1b[200~" } else { b"" };
    let end: &[u8] = if bracketed { b"\x1b[201~" } else { b"" };
    let total = payload.len() + markers.len() + end.len();
    let on = if bracketed {
        "printf '\\033[?2004h'; "
    } else {
        ""
    };
    let mut client = raw_pane(
        &daemon,
        &format!(
            "{on}{reader_starts_after}head -c {total} > {}; stty sane; echo fin''ished",
            file.display()
        ),
    );
    if bracketed {
        // The pane's mode reaches the daemon with its output; wait until it is in effect.
        assert!(
            client.wait_until(WAIT, |c| c.screen.mode(arcade_term::Mode::BracketedPaste)
                == Some(true))
        );
    }
    let mut host_bytes = b"\x1b[200~".to_vec();
    host_bytes.extend_from_slice(&payload);
    host_bytes.extend_from_slice(b"\x1b[201~");
    for chunk in host_bytes.chunks(8192) {
        client.send(chunk);
    }
    assert!(
        client.wait_until(Duration::from_secs(30), |c| c
            .content_rows()
            .iter()
            .any(|r| r.trim_start() == "finished")),
        "the pane did not get all of the paste (file has {} of {total} bytes):\n{}",
        std::fs::metadata(&file).map_or(0, |m| m.len()),
        client.screen_text()
    );
    let got = std::fs::read(&file).unwrap();
    let mut want = markers.to_vec();
    want.extend_from_slice(&payload);
    want.extend_from_slice(end);
    assert_eq!(got.len(), want.len());
    assert!(got == want, "the pasted bytes differ");
}

#[test]
fn a_megabyte_paste_arrives_whole_without_markers_when_the_pane_did_not_ask_for_them() {
    paste_and_compare(false, "");
}

#[test]
fn a_megabyte_paste_arrives_whole_between_markers_when_the_pane_asked_for_them() {
    paste_and_compare(true, "");
}

#[test]
fn a_pane_that_asked_for_the_kitty_protocol_gets_keys_in_that_form() {
    let daemon = daemon_with_a_free_prefix();
    let mut client = raw_pane(&daemon, "printf '\\033[>1u'; cat -vt");
    // The pane has read the request once it has echoed a first key.
    client.send(b"x");
    assert!(client.wait_until(WAIT, |c| {
        c.content_rows().iter().any(|r| r.trim_start() == "x")
    }));
    client.send(b"\x1b[109;5u");
    client.send(b"\x1b[27u");
    client.send(b"\r");
    let expected = "x^[[109;5u^[[27u^M";
    assert!(
        client.wait_until(WAIT, |c| c
            .content_rows()
            .iter()
            .any(|r| r.trim_start() == expected)),
        "{}",
        client.screen_text()
    );
}

/// The pane reads nothing for a while after the paste starts: the megabyte waits in the daemon,
/// the terminal's buffer fills, and everything still arrives complete and in order.
#[test]
fn a_one_megabyte_paste_into_a_pane_that_starts_reading_late_arrives_whole_and_in_order() {
    paste_and_compare(false, "sleep 1; ");
}
