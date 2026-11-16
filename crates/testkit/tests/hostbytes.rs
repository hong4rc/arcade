//! What a pane's program is sent when the host terminal sends bytes that are no key: a pane must
//! never get a byte sequence that is not what a key means (an input method posts odd keys and
//! half-characters, and one stray byte in a shell's line is shown as `<ffffffff>`).

use std::time::Duration;

use arcade_testkit::{Screened, TestClient, TestDaemon, WAIT};

/// A pane whose program prints the hexadecimal of every byte it is sent, in raw mode; `flags`
/// is what it asked of the kitty keyboard protocol (0: nothing, like a shell).
fn hex_pane(daemon: &TestDaemon, flags: u8) -> TestClient {
    let mut client = daemon.attach_shell();
    client.wait_for_prompt().unwrap();
    let ask = if flags == 0 {
        String::new()
    } else {
        format!("printf '\\033[>{flags}u'; ")
    };
    client.type_line(&format!(
        "stty raw -echo; echo ready; {ask}perl -e 'binmode STDIN; $|=1; while(sysread(STDIN,$b,1)){{printf \"%02x \",ord($b)}}'"
    ));
    client.wait_for_row("ready").unwrap();
    std::thread::sleep(Duration::from_millis(500));
    client
}

fn daemon() -> TestDaemon {
    TestDaemon::start_with(|c| c.config.keys.prefix = arcade_daemon::parse_key("f12").unwrap())
}

/// What the program printed so far: its bytes as hexadecimal, separated by spaces.
fn seen(client: &TestClient) -> String {
    client
        .content_rows()
        .iter()
        .map(|r| r.trim().to_string())
        .skip_while(|r| r != "ready")
        .skip(1)
        .collect::<Vec<_>>()
        .join(" ")
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ")
}

fn settle(client: &mut TestClient) {
    client.wait_until(Duration::from_millis(300), |_| false);
}

/// Forms that are no key: none of them reaches a program that asked for nothing, and the key
/// after each one (a `Z`, 5a) does.
#[test]
fn bytes_that_are_no_key_never_reach_a_pane_that_asked_for_nothing() {
    let forms: &[(&str, &[u8])] = &[
        ("lone ff", b"\xff"),
        ("lone 80", b"\x80"),
        ("C1 9b", b"\x9b"),
        ("C1 85", b"\x85"),
        ("overlong", b"\xc0\x80"),
        ("lead f8", b"\xf8"),
        ("alt and ff", b"\x1b\xff"),
        ("csi and ff", b"\x1b[\xff"),
        ("C1 as utf-8", b"\xc2\x85"),
        ("kitty code 0", b"\x1b[0u"),
        ("kitty C1", b"\x1b[133u"),
        ("kitty surrogate", b"\x1b[55296u"),
        ("kitty past the end", b"\x1b[1114112u"),
        ("kitty text only", b"\x1b[0;;101u"),
        ("kitty private use", b"\x1b[57344u"),
        ("kitty release only", b"\x1b[101;1:3u"),
    ];
    let daemon = daemon();
    let mut client = hex_pane(&daemon, 0);
    for (name, form) in forms {
        client.send(form);
        client.send(b"Z");
        // Each form is followed by a Z, so the number of Z that arrived says the pane is caught up.
        let expected = forms.iter().position(|(n, _)| n == name).unwrap() + 1;
        assert!(
            client.wait_until(WAIT, |c| seen(c).matches("5a").count() >= expected),
            "{name}: nothing arrived: {}",
            seen(&client)
        );
        let all = seen(&client);
        assert!(
            all.split(' ').all(|b| b == "5a"),
            "{name}: the pane was sent {all}"
        );
    }
}

/// A character is not cut: pieces that come later than the wait for an Esc are put together, and
/// every character reaches the pane whole.
#[test]
fn a_character_that_arrives_in_pieces_reaches_the_pane_whole() {
    let daemon = daemon();
    let mut client = hex_pane(&daemon, 0);
    // The three bytes of one character, 100 ms apart.
    for byte in [0xe1u8, 0xba, 0xbd] {
        client.send(&[byte]);
        std::thread::sleep(Duration::from_millis(100));
    }
    client.send("é€".as_bytes());
    settle(&mut client);
    assert_eq!(
        seen(&client),
        "e1 ba bd c3 a9 e2 82 ac",
        "{}",
        client.screen_text()
    );
}

/// A program that asked for the kitty form gets the keys that are in it, and still not the ones
/// that are no key.
#[test]
fn a_pane_that_asked_for_the_kitty_form_gets_real_keys_and_not_the_others() {
    let daemon = daemon();
    let mut client = hex_pane(&daemon, 1);
    client.send(b"\x1b[0u");
    client.send(b"\x1b[133u");
    client.send(b"\xff");
    client.send(b"\x1b[57344u");
    settle(&mut client);
    // The private-use key is a key (a program may have asked for it); the others are not.
    assert_eq!(
        seen(&client),
        "1b 5b 35 37 33 34 34 75",
        "{}",
        client.screen_text()
    );
}
