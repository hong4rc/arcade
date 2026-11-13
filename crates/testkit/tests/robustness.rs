//! What happens when things go wrong or get big: a gigabyte of random bytes, a pane that dies,
//! and the daemon's log file.

use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

use arcade_proto::ServerMessage;
use arcade_testkit::{AttachOptions, Screened, TestDaemon, WAIT, footprint_kib};

/// Core scenario 10: a gigabyte of random bytes goes into a pane. The daemon keeps running, the
/// client keeps receiving screen updates the whole time, and keys typed during the flood reach
/// the program in the pane.
#[test]
fn a_gigabyte_of_random_bytes_does_not_stop_the_daemon() {
    let daemon = TestDaemon::start();
    let dir = daemon.dir().to_path_buf();
    let done = dir.join("flood-done");
    let keys = dir.join("keys.txt");
    // `head` floods the terminal in the background while `cat` reads what is typed into a file.
    // The terminal is not in line mode: random bytes make the terminal answer queries, and those
    // answers, with no line ending, would fill a line buffer and make it drop what comes after.
    let script = format!(
        "stty -icanon -echo; (head -c 1073741824 /dev/urandom; touch '{}') & exec cat > '{}'",
        done.display(),
        keys.display()
    );
    let mut client = daemon
        .attach(AttachOptions::new().command(&["/bin/sh", "-c", &script]))
        .unwrap();

    let started = Instant::now();
    let mut typed = 0;
    let mut next_key = Instant::now();
    let mut updates_in_windows = Vec::new();
    let mut window_start = (Instant::now(), client.output_messages);
    while !done.exists() {
        assert!(
            started.elapsed() < Duration::from_secs(300),
            "the flood did not finish in 300 s"
        );
        assert!(!daemon.is_stopped(), "the daemon stopped during the flood");
        let tick = Instant::now() + Duration::from_millis(50);
        client.pump(tick);
        if Instant::now() >= next_key {
            typed += 1;
            client.send(format!("key-{typed}\n").as_bytes());
            next_key = Instant::now() + Duration::from_millis(500);
        }
        if window_start.0.elapsed() >= Duration::from_secs(2) {
            updates_in_windows.push(client.output_messages - window_start.1);
            window_start = (Instant::now(), client.output_messages);
        }
    }
    let seconds = started.elapsed().as_secs_f64();
    eprintln!(
        "flood: 1 GiB in {seconds:.1} s, updates per 2 s window: {updates_in_windows:?}, keys typed: {typed}"
    );
    // Not one window of two seconds without screen updates.
    assert!(
        updates_in_windows.iter().all(|n| *n >= 10),
        "the client went without updates: {updates_in_windows:?}"
    );
    // The memory stays bounded: scrollback is capped, and a pending update is one frame.
    let footprint_mb = footprint_kib(std::process::id()) / 1024;
    eprintln!("flood: footprint {footprint_mb} MB");
    assert!(footprint_mb < 600, "{footprint_mb} MB");

    // The pane is still alive and takes keys after the flood.
    client.send(b"after-the-flood\n");
    let deadline = Instant::now() + WAIT;
    loop {
        let text = std::fs::read_to_string(&keys).unwrap_or_default();
        if text.contains("after-the-flood") {
            // Every key typed during the flood arrived too, in order.
            let mut at = 0;
            for n in 1..=typed {
                let key = format!("key-{n}");
                let found = text[at..].find(&key);
                assert!(found.is_some(), "{key} never reached the pane:\n{text:?}");
                at += found.unwrap() + key.len();
            }
            break;
        }
        assert!(Instant::now() < deadline, "keys stopped reaching the pane");
        client.pump(Instant::now() + Duration::from_millis(20));
    }
    let (ok, _) = daemon.command("session.list", &[]).unwrap();
    assert!(ok);
}

/// A pane that dies closes its own session with a notice, and nothing else: the other session
/// goes on, and the daemon leaves cleanly (no dangling socket) once the last session is gone.
#[test]
fn a_dying_pane_closes_only_its_own_session() {
    let mut daemon = TestDaemon::start();
    let mut survivor = daemon
        .attach(
            AttachOptions::new()
                .command(&["/bin/cat"])
                .named("survivor"),
        )
        .unwrap();
    let mut doomed = daemon
        .attach(
            AttachOptions::new()
                .command(&["/bin/sh", "-c", "kill -9 $$"])
                .named("doomed"),
        )
        .unwrap();
    let msg = doomed
        .wait_message(|m| matches!(m, ServerMessage::SessionEnded { .. }), WAIT)
        .unwrap();
    assert!(
        matches!(&msg, ServerMessage::SessionEnded { message } if message.contains("signal 9")),
        "{msg:?}"
    );
    assert!(!daemon.is_stopped(), "one dead pane took the daemon down");
    let (_, list) = daemon.command("session.list", &[]).unwrap();
    assert!(
        list.contains("survivor") && !list.contains("doomed"),
        "{list}"
    );

    // The other pane still works.
    survivor.send(b"still-here\n");
    survivor.wait_for_text("still-here").unwrap();

    // Closing the last session ends the daemon without being killed, and removes its socket.
    survivor.send(&[4]);
    survivor
        .wait_message(|m| matches!(m, ServerMessage::SessionEnded { .. }), WAIT)
        .unwrap();
    daemon.wait_stopped(WAIT).unwrap();
    assert!(!daemon.socket_exists(), "the socket file was left behind");
}

fn log_files(log: &Path) -> (PathBuf, PathBuf) {
    let mut old = log.as_os_str().to_owned();
    old.push(".1");
    (log.to_path_buf(), PathBuf::from(old))
}

/// The daemon's log has a size limit, and holds what happened, never what a pane showed, what
/// was typed, or what command was run.
#[test]
fn the_log_has_a_size_limit_and_no_pane_content() {
    let log = PathBuf::from(format!(
        "/tmp/arc-log-{}-{}.log",
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos()
    ));
    let (current, old) = log_files(&log);
    let limit = 4096;
    let path = log.clone();
    let daemon = TestDaemon::start_with(move |config| {
        config.log_path = Some(path);
        config.log_limit_bytes = limit;
    });

    // One session that keeps living, shows a secret and is typed a secret.
    let mut keeper = daemon
        .attach(AttachOptions::new().command(&["/bin/cat"]).named("keeper"))
        .unwrap();
    keeper.send(b"typed-secret-kqzv\n");
    keeper.wait_for_text("typed-secret-kqzv").unwrap();

    // Many short sessions, so much more than the limit is logged.
    for n in 0..80 {
        let secret = format!("argument-secret-kqzv-{n}");
        let mut client = daemon
            .attach(AttachOptions::new().command(&["/bin/echo", &secret]))
            .unwrap();
        client.wait_for_text(&secret).unwrap();
        client
            .wait_message(|m| matches!(m, ServerMessage::SessionEnded { .. }), WAIT)
            .unwrap();
    }
    let (_, list) = daemon.command("session.list", &[]).unwrap();
    assert!(list.contains("keeper"));

    let size = std::fs::metadata(&current).expect("the log exists").len();
    assert!(size <= limit, "the log grew to {size} bytes");
    let old_size = std::fs::metadata(&old).expect("the log was rotated").len();
    assert!(old_size <= limit, "the old log is {old_size} bytes");
    let text = format!(
        "{}{}",
        std::fs::read_to_string(&old).unwrap(),
        std::fs::read_to_string(&current).unwrap()
    );
    assert!(text.contains("created"), "{text}");
    for secret in ["typed-secret", "argument-secret", "kqzv"] {
        assert!(
            !text.contains(secret),
            "the log holds pane content: {secret}"
        );
    }
    drop(keeper);
    drop(daemon);
    let _ = std::fs::remove_file(&current);
    let _ = std::fs::remove_file(&old);
}
