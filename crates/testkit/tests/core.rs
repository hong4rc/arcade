//! The core of the daemon, driven through the fake client: sessions, panes, keys, resizing and
//! the end of a session. These are the core spec's acceptance scenarios 1, 3 and 5 plus the
//! behaviours they rest on.

use std::time::Duration;

use arcade_proto::{ClientMessage, Decoder, DetachReason, PROTOCOL_VERSION, ServerMessage, encode};
use arcade_testkit::{AttachOptions, Screened, TestClient, TestDaemon, WAIT};

fn screen_has(client: &TestClient, row: &str) -> bool {
    client.screen_rows().iter().any(|r| r == row)
}

/// Scenario 1: a session with a pane running a shell; a typed command shows its result.
#[test]
fn a_session_with_a_shell_runs_typed_commands() {
    let daemon = TestDaemon::start();
    let mut client = daemon.attach_shell();
    client.wait_for_prompt().unwrap();

    client.type_line("echo result-$((6*7))");
    client.wait_for_text("result-42").unwrap();
    // The typed line itself is on screen too, so the result is the line that is exactly that.
    assert!(screen_has(&client, "result-42"), "{}", client.screen_text());

    let (ok, list) = daemon.command("session.list", &[]).unwrap();
    assert!(ok);
    let lines: Vec<&str> = list.lines().collect();
    assert_eq!(lines.len(), 1, "{list:?}");
    assert!(lines[0].ends_with("\t1"), "attached flag: {list:?}");
}

#[test]
fn default_shell_is_used_when_no_command_is_given() {
    let daemon = TestDaemon::start();
    let mut client = daemon
        .attach(AttachOptions::new().command(&[]))
        .expect("attach");
    // SHELL is /bin/sh in the test environment, started as a login shell.
    client.type_line("echo shell-is-$0");
    client.wait_for_text("shell-is--sh").unwrap();
}

#[test]
fn the_pane_gets_the_arcade_environment() {
    let daemon = TestDaemon::start();
    let mut client = daemon.attach_shell();
    client.wait_for_prompt().unwrap();
    client.type_line("echo T=$TERM C=$COLORTERM P=$ARCADE_PANE");
    client
        .wait_for_text("T=xterm-256color C=truecolor P=")
        .unwrap();
    let row = client
        .screen_rows()
        .into_iter()
        .find(|r| r.starts_with("T=xterm"))
        .unwrap();
    let pane: u64 = row
        .rsplit("P=")
        .next()
        .unwrap()
        .trim()
        .parse()
        .expect("pane id");
    assert!(pane > 0);

    client.type_line("echo S=$ARCADE");
    client
        .wait_for_text(&format!("S={}", daemon.socket().display()))
        .unwrap();
}

#[test]
fn the_pane_starts_in_the_requested_directory() {
    let daemon = TestDaemon::start();
    let dir = daemon.dir().join("work");
    std::fs::create_dir_all(&dir).unwrap();
    let mut options = AttachOptions::new();
    options.cwd = Some(dir.display().to_string());
    let mut client = daemon.attach(options).unwrap();
    client.type_line("pwd -P");
    let real = std::fs::canonicalize(&dir).unwrap();
    client.wait_for_text(&real.display().to_string()).unwrap();
    // The session is named after the directory.
    let (_, list) = daemon.command("session.list", &[]).unwrap();
    assert!(list.starts_with("work\t"), "{list:?}");
}

/// A daemon with no key of its own in the way: every byte a test sends should reach the program.
fn daemon_without_key_bindings() -> TestDaemon {
    TestDaemon::start_with(|c| {
        c.config.keys.prefix = arcade_daemon::parse_key("f12").unwrap();
        c.config.keys.direct_modifier = None;
    })
}

#[test]
fn keyboard_bytes_reach_the_program_unchanged() {
    let daemon = daemon_without_key_bindings();
    let mut client = daemon.attach_shell();
    client.wait_for_prompt().unwrap();
    // Read exactly five raw bytes and print them in hexadecimal.
    client.type_line("stty raw -echo; printf 'armed\\r\\n'; head -c 5 | od -An -tx1; stty sane");
    // Only once the program says it is reading are the bytes sent, so nothing is echoed.
    let armed = client.wait_until(WAIT, |c| c.screen_rows().iter().any(|r| r == "armed"));
    assert!(armed, "{}", client.screen_text());
    client.send(&[0x01, 0x02, 0x1b, b'[', b'A']);
    // od pads its columns differently on different systems, so compare without the padding.
    let ok = client.wait_until(WAIT, |c| {
        c.screen_rows()
            .iter()
            .any(|r| r.split_whitespace().collect::<Vec<_>>().join(" ") == "01 02 1b 5b 41")
    });
    assert!(ok, "{}", client.screen_text());
}

#[test]
fn the_terminal_answers_a_program_that_asks_where_the_cursor_is() {
    let daemon = TestDaemon::start();
    let mut client = daemon.attach_shell();
    client.wait_for_prompt().unwrap();
    client
        .type_line("stty raw -echo; printf '\\033[3;5H\\033[6n'; head -c 6 | od -An -c; stty sane");
    // The reply is ESC [ 3 ; 5 R, which od shows with its escape as 033.
    client.wait_for_text("033   [   3   ;   5   R").unwrap();
}

/// Scenario 3: the first process of the last pane ends; the session closes, the daemon stops.
#[test]
fn when_the_shell_exits_the_session_closes_and_the_daemon_stops() {
    let mut daemon = TestDaemon::start();
    let mut client = daemon.attach_shell();
    client.wait_for_prompt().unwrap();
    client.type_line("exit");
    let msg = client
        .wait_message(|m| matches!(m, ServerMessage::SessionEnded { .. }), WAIT)
        .unwrap();
    assert!(
        matches!(&msg, ServerMessage::SessionEnded { message } if message.contains("exited")),
        "{msg:?}"
    );
    assert!(client.wait_closed(WAIT), "connection stays open");
    daemon.wait_stopped(WAIT).expect("daemon stops");
    assert!(!daemon.socket_exists(), "socket file left behind");
}

#[test]
fn the_exit_status_is_reported() {
    let mut daemon = TestDaemon::start();
    let mut client = daemon.attach_shell();
    client.wait_for_prompt().unwrap();
    client.type_line("exit 7");
    let msg = client
        .wait_message(|m| matches!(m, ServerMessage::SessionEnded { .. }), WAIT)
        .unwrap();
    assert!(
        matches!(&msg, ServerMessage::SessionEnded { message } if message.contains("status 7")),
        "{msg:?}"
    );
    daemon.wait_stopped(WAIT).unwrap();
}

#[test]
fn output_written_just_before_exit_is_not_lost() {
    let mut daemon = TestDaemon::start();
    let mut client = daemon
        .attach(AttachOptions::new().command(&["/bin/sh", "-c", "echo last-words; exit 0"]))
        .unwrap();
    client
        .wait_message(|m| matches!(m, ServerMessage::SessionEnded { .. }), WAIT)
        .unwrap();
    client.wait_for_text("last-words").unwrap();
    daemon.wait_stopped(WAIT).unwrap();
}

#[test]
fn a_command_that_cannot_run_is_reported_and_creates_no_session() {
    let daemon = TestDaemon::start();
    let err = daemon
        .attach(AttachOptions::new().command(&["/nonexistent/program"]))
        .err()
        .expect("attach should fail");
    assert!(err.contains("cannot run"), "{err}");
    let (_, list) = daemon.command("session.list", &[]).unwrap();
    assert_eq!(list, "");
}

#[test]
fn a_command_runs_directly_without_a_shell() {
    let mut daemon = TestDaemon::start();
    let mut client = daemon
        .attach(AttachOptions::new().command(&["/bin/echo", "straight", "to", "echo"]))
        .unwrap();
    if let Err(e) = client.wait_for_text("straight to echo") {
        panic!("{e}\nmessages: {:?}", client.messages);
    }
    client
        .wait_message(|m| matches!(m, ServerMessage::SessionEnded { .. }), WAIT)
        .unwrap();
    daemon.wait_stopped(WAIT).unwrap();
}

/// Resizing the client resizes the terminal the program runs in.
#[test]
fn resizing_the_client_gives_the_program_the_new_size() {
    let daemon = TestDaemon::start();
    let mut client = daemon.attach_shell();
    client.wait_for_prompt().unwrap();
    client.type_line("stty size");
    // One row of the client's terminal is the status bar.
    client.wait_for_text("29 100").unwrap();

    client.resize(60, 20);
    client.type_line("stty size");
    client.wait_for_text("19 60").unwrap();
    assert_eq!((client.screen.cols(), client.screen.rows()), (60, 20));

    client.resize(132, 43);
    client.type_line("stty size");
    client.wait_for_text("42 132").unwrap();
}

#[test]
fn the_program_is_told_about_a_resize_with_a_signal() {
    let daemon = TestDaemon::start();
    let mut client = daemon.attach_shell();
    client.wait_for_prompt().unwrap();
    client.type_line("trap 'echo winch-$(stty size)' WINCH; echo ready-to-trap");
    client.wait_for_text("ready-to-trap").unwrap();
    client.resize(70, 25);
    // Some shells (dash) run a trap only once the next command is read, so a command follows.
    client.type_line("true");
    client.wait_for_text("winch-24 70").unwrap();
}

/// Scenario 5 through the whole path: a line with combining Vietnamese, Han characters and a
/// joined emoji leaves the cursor in the right column on the client's screen.
#[test]
fn mixed_width_text_puts_the_cursor_in_the_right_column() {
    let daemon = TestDaemon::start();
    let mut client = daemon.attach_shell();
    client.wait_for_prompt().unwrap();
    let line = "Tie\u{302}\u{301}ng Vie\u{323}\u{302}t 汉字 👨\u{200D}👩\u{200D}👧\u{200D}👦";
    // `read` keeps the shell from printing its next prompt after the line.
    client.type_line(&format!("printf '\\033[2J\\033[H%s' '{line}'; read x"));
    client.wait_for_text("汉字").unwrap();
    // 10 columns of Vietnamese, a space, 4 of Han, a space, 2 for the emoji.
    let ok = client.wait_until(WAIT, |c| c.screen.cursor_position() == (18, 0));
    assert!(
        ok,
        "cursor at {:?}\n{}",
        client.screen.cursor_position(),
        client.screen_text()
    );
    assert_eq!(client.screen.row_text(0), line);
}

#[test]
fn colors_and_attributes_reach_the_client_screen() {
    let daemon = TestDaemon::start();
    let mut client = daemon.attach_shell();
    client.wait_for_prompt().unwrap();
    client.type_line(
        "printf '\\033[2J\\033[H\\033[1;31mR\\033[0;38;5;200mP\\033[0;38;2;1;2;3;48;2;9;8;7mT\\033[0;4:3mC\\033[0m.'",
    );
    client.wait_until(WAIT, |c| c.screen.row_text(0).starts_with("RPTC."));
    let s = |col| client.screen.style_at(0, col);
    assert!(s(0).bold);
    assert_eq!(s(0).fg, arcade_term::Color::Indexed(1));
    assert_eq!(s(1).fg, arcade_term::Color::Indexed(200));
    assert_eq!(s(2).fg, arcade_term::Color::Rgb(1, 2, 3));
    assert_eq!(s(2).bg, arcade_term::Color::Rgb(9, 8, 7));
    assert_eq!(s(3).underline, arcade_term::Underline::Curly);
    assert_eq!(s(4), arcade_term::Style::default());
}

#[test]
fn a_client_that_disconnects_leaves_the_session_running() {
    let daemon = TestDaemon::start();
    let mut client = daemon.attach_shell();
    client.wait_for_prompt().unwrap();
    client.type_line("echo before-leaving");
    client.wait_for_text("before-leaving").unwrap();
    drop(client);

    // The daemon notices the closed connection: the session is listed as not attached.
    let start = std::time::Instant::now();
    loop {
        let (_, list) = daemon.command("session.list", &[]).unwrap();
        // Fields: name, id, attached, windows, panes.
        if list.lines().count() == 1 && list.split('\t').nth(2) == Some("0") {
            break;
        }
        assert!(start.elapsed() < WAIT, "still attached: {list:?}");
        std::thread::sleep(Duration::from_millis(5));
    }
    assert!(!daemon.is_stopped());
}

#[test]
fn the_daemon_refuses_to_start_twice_on_one_socket() {
    let daemon = TestDaemon::start();
    let config = arcade_daemon::DaemonConfig::new(daemon.socket());
    let err = arcade_daemon::run(config).expect_err("second daemon must not start");
    assert_eq!(err.kind(), std::io::ErrorKind::AddrInUse, "{err}");
    // The first one still works.
    assert_eq!(
        daemon.command("server.ping", &[]).unwrap(),
        (true, "pong".into())
    );
}

#[test]
fn a_stale_socket_file_is_replaced() {
    let dir = std::path::PathBuf::from(format!("/tmp/arc-stale-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let socket = dir.join("s");
    // A socket file nobody listens on.
    drop(std::os::unix::net::UnixListener::bind(&socket).unwrap());
    assert!(socket.exists());
    let path = socket.clone();
    let handle =
        std::thread::spawn(move || arcade_daemon::run(arcade_daemon::DaemonConfig::new(path)));
    let start = std::time::Instant::now();
    let mut client = loop {
        match TestClient::connect(&socket) {
            Ok(c) => break c,
            Err(_) => {
                assert!(start.elapsed() < WAIT);
                std::thread::sleep(Duration::from_millis(5));
            }
        }
    };
    client
        .send_message(&ClientMessage::Command {
            name: "server.kill".into(),
            args: vec![],
        })
        .unwrap();
    handle.join().unwrap().unwrap();
    std::fs::remove_dir_all(&dir).ok();
}

#[test]
fn the_socket_is_private_to_the_user() {
    use std::os::unix::fs::PermissionsExt;
    let daemon = TestDaemon::start();
    let mode = std::fs::metadata(daemon.socket())
        .unwrap()
        .permissions()
        .mode();
    assert_eq!(mode & 0o777, 0o600, "socket mode {mode:o}");
    let dir_mode = std::fs::metadata(daemon.dir())
        .unwrap()
        .permissions()
        .mode();
    assert_eq!(dir_mode & 0o077, 0, "directory mode {dir_mode:o}");
}

#[test]
fn a_client_with_another_protocol_version_is_told_both_versions() {
    let daemon = TestDaemon::start();
    let mut stream = std::os::unix::net::UnixStream::connect(daemon.socket()).unwrap();
    let mut bytes = Vec::new();
    encode(&ClientMessage::Hello { version: 99 }, &mut bytes).unwrap();
    std::io::Write::write_all(&mut stream, &bytes).unwrap();
    stream.set_read_timeout(Some(WAIT)).unwrap();
    let mut decoder = Decoder::<ServerMessage>::new();
    let mut buf = [0u8; 1024];
    let message = loop {
        let n = std::io::Read::read(&mut stream, &mut buf).unwrap();
        if n == 0 {
            panic!("closed without an answer");
        }
        decoder.push(&buf[..n]);
        if let Some(m) = decoder.next_message().unwrap() {
            break m;
        }
    };
    assert_eq!(
        message,
        ServerMessage::VersionMismatch {
            daemon: PROTOCOL_VERSION,
            client: 99
        }
    );
    // The connection stays open for one thing only, stopping the daemon, so that a new client
    // can stop an old daemon. Anything else ends it.
    let mut bytes = Vec::new();
    encode(&ClientMessage::Input(b"x".to_vec()), &mut bytes).unwrap();
    std::io::Write::write_all(&mut stream, &bytes).unwrap();
    let n = std::io::Read::read(&mut stream, &mut buf).unwrap();
    assert_eq!(n, 0);
}

#[test]
fn garbage_on_a_connection_closes_only_that_connection() {
    let daemon = TestDaemon::start();
    let mut stream = std::os::unix::net::UnixStream::connect(daemon.socket()).unwrap();
    std::io::Write::write_all(&mut stream, &[0xff; 64]).unwrap();
    stream.set_read_timeout(Some(WAIT)).unwrap();
    let mut buf = [0u8; 256];
    // Whatever the daemon says, it ends the connection.
    loop {
        let n = std::io::Read::read(&mut stream, &mut buf).unwrap_or(0);
        if n == 0 {
            break;
        }
    }
    assert_eq!(daemon.command("server.ping", &[]).unwrap().1, "pong");
}

#[test]
fn input_before_the_handshake_is_refused() {
    let daemon = TestDaemon::start();
    let mut stream = std::os::unix::net::UnixStream::connect(daemon.socket()).unwrap();
    let mut bytes = Vec::new();
    encode(&ClientMessage::Input(b"x".to_vec()), &mut bytes).unwrap();
    std::io::Write::write_all(&mut stream, &bytes).unwrap();
    stream.set_read_timeout(Some(WAIT)).unwrap();
    let mut buf = [0u8; 256];
    let n = std::io::Read::read(&mut stream, &mut buf).unwrap();
    assert!(n > 0);
    assert_eq!(daemon.command("server.ping", &[]).unwrap().1, "pong");
}

#[test]
fn server_kill_stops_the_panes_and_the_daemon() {
    let mut daemon = TestDaemon::start();
    let mut client = daemon.attach_shell();
    client.wait_for_prompt().unwrap();
    client.type_line("echo $$");
    client.wait_until(WAIT, |c| {
        c.screen_rows()
            .iter()
            .any(|r| r.trim().parse::<u32>().is_ok())
    });
    let pid: i32 = client
        .screen_rows()
        .iter()
        .find_map(|r| r.trim().parse().ok())
        .expect("pid of the shell");
    let (ok, _) = daemon.command("server.kill", &[]).unwrap();
    assert!(ok);
    let msg = client
        .wait_message(|m| matches!(m, ServerMessage::Detached { .. }), WAIT)
        .unwrap();
    assert!(matches!(
        msg,
        ServerMessage::Detached {
            reason: DetachReason::ServerStopping,
            ..
        }
    ));
    daemon.wait_stopped(WAIT).unwrap();
    // The shell is gone too (kill with signal 0 only checks that the process exists).
    let start = std::time::Instant::now();
    loop {
        // SAFETY: signal 0 sends nothing; it only reports whether the process exists.
        let alive = unsafe { libc::kill(pid, 0) } == 0;
        if !alive {
            break;
        }
        assert!(start.elapsed() < WAIT, "shell {pid} still running");
        std::thread::sleep(Duration::from_millis(10));
    }
}

/// A paste far larger than the daemon is willing to hold arrives complete: the daemon stops
/// reading from the client while the program catches up, instead of dropping bytes.
#[test]
fn a_large_paste_arrives_complete() {
    const SIZE: usize = 3 * 1024 * 1024;
    let daemon = daemon_without_key_bindings();
    let mut client = daemon.attach_shell();
    client.wait_for_prompt().unwrap();
    client.type_line(
        "stty raw -echo; printf 'armed\\r\\n'; head -c 3145728 > paste.bin; stty sane; echo stored-$(wc -c < paste.bin | tr -d ' ')",
    );
    client.wait_for_row("armed").unwrap();
    let data: Vec<u8> = (0..SIZE).map(|i| (i % 251) as u8).collect();
    // Send it in pieces the size a terminal would, the way `arcade` does.
    for chunk in data.chunks(16 * 1024) {
        client.send(chunk);
    }
    client.wait_for_row(&format!("stored-{SIZE}")).unwrap();
    let stored = std::fs::read(daemon.dir().join("paste.bin")).unwrap();
    assert_eq!(stored.len(), SIZE);
    assert!(stored == data, "the pasted bytes arrived changed");
}

/// The last output of a program that ends right after a key reaches the screen before the
/// session ends, however the two events line up. The count can be raised with `ARCADE_LOOPS`.
#[test]
fn the_last_output_of_a_program_is_shown_before_the_session_ends() {
    let loops: usize = std::env::var("ARCADE_LOOPS")
        .ok()
        .and_then(|v| v.parse().ok())
        .unwrap_or(20);
    for round in 0..loops {
        let mut daemon = TestDaemon::start();
        let mut client = daemon
            .attach(AttachOptions::new().command(&[
                "sh",
                "-c",
                "printf READY; read x; printf MARK",
            ]))
            .unwrap();
        if let Err(e) = client.wait_for_text("READY") {
            panic!("round {round}: {e}\nmessages: {:?}", client.messages);
        }
        client.send(b"\r");
        client
            .wait_message(|m| matches!(m, ServerMessage::SessionEnded { .. }), WAIT)
            .unwrap();
        assert!(
            client.screen_text().contains("MARK"),
            "round {round}: the session ended without MARK on the screen:\n{}\nmessages: {:?}",
            client.screen_text(),
            client.messages
        );
        daemon.wait_stopped(WAIT).unwrap();
    }
}
