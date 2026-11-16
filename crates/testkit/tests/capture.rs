//! The key capture: off for every pane until it is turned on, it writes the bytes read from the
//! host and the bytes written to the pane, with times, to a file only the user can read, shows a
//! clear word on the status bar while it is on, and ends with the command or the pane.

use std::os::unix::fs::PermissionsExt;
use std::path::PathBuf;

use arcade_testkit::{Screened, TestDaemon, WAIT};

fn captures(daemon: &TestDaemon) -> Vec<PathBuf> {
    let mut found: Vec<PathBuf> = std::fs::read_dir(daemon.dir())
        .unwrap()
        .filter_map(Result::ok)
        .map(|e| e.path())
        .filter(|p| {
            p.file_name()
                .and_then(|n| n.to_str())
                .is_some_and(|n| n.starts_with("capture-"))
        })
        .collect();
    found.sort();
    found
}

#[test]
fn it_is_off_until_asked_and_records_the_exact_bytes_while_on() {
    let daemon = TestDaemon::start();
    let mut client = daemon.attach_shell();
    client.wait_for_prompt().unwrap();
    let pane = client.focused_pane_id();
    client.type_line("stty raw -echo; echo ready");
    client.wait_for_row("ready").unwrap();
    // Off by default: typing makes no file, and the bar says nothing.
    client.send(b"a");
    assert!(captures(&daemon).is_empty());
    assert!(!client.status_row().contains("RECORDING"));

    let (ok, message) = daemon
        .command_from(Some(pane), "pane.capture-keys", &[])
        .unwrap();
    assert!(ok, "{message}");
    assert!(message.contains("passwords"), "{message}");
    // The line says which pane and which window it is, not just "this pane".
    assert!(
        message.contains(&format!("in pane {pane} (window 1 ")),
        "{message}"
    );
    assert!(
        client.wait_until(WAIT, |c| c.status_row().contains("RECORDING KEYS")),
        "{}",
        client.status_row()
    );
    let files = captures(&daemon);
    assert_eq!(files.len(), 1, "{files:?}");
    let mode = std::fs::metadata(&files[0]).unwrap().permissions().mode();
    assert_eq!(mode & 0o777, 0o600, "{mode:o}");

    // A byte that is no key is read from the host, and is not written to the pane.
    client.send(b"b\xff\x1b[Ac");
    assert!(client.wait_until(WAIT, |_| {
        std::fs::read_to_string(&files[0]).is_ok_and(|t| t.contains(" pane 63"))
    }));
    let text = std::fs::read_to_string(&files[0]).unwrap();
    let host: Vec<&str> = text.lines().filter(|l| l.contains(" host ")).collect();
    assert!(
        host.iter().any(|l| l.ends_with("host 62 ff 1b 5b 41 63")),
        "{text}"
    );
    let written: Vec<&str> = text
        .lines()
        .skip(1)
        .filter(|l| l.contains(" pane "))
        .map(|l| l.rsplit_once(" pane ").unwrap().1)
        .collect();
    assert_eq!(
        written.concat().replace(' ', ""),
        "62".to_string() + "1b5b41" + "63",
        "{text}"
    );
    // Every line has a time.
    for line in text.lines().skip(1) {
        assert!(
            line.split(' ').next().unwrap().parse::<f64>().is_ok(),
            "{line}"
        );
    }

    // The same command turns it off; the bar goes back, and later keys are not recorded.
    let (ok, message) = daemon
        .command_from(Some(pane), "pane.capture-keys", &[])
        .unwrap();
    assert!(ok && message.contains("stopped"), "{message}");
    assert!(
        message.contains(&format!("in pane {pane} (window 1 ")),
        "{message}"
    );
    assert!(client.wait_until(WAIT, |c| !c.status_row().contains("RECORDING")));
    let size = std::fs::metadata(&files[0]).unwrap().len();
    client.send(b"zzz");
    std::thread::sleep(std::time::Duration::from_millis(300));
    assert_eq!(std::fs::metadata(&files[0]).unwrap().len(), size);
}

#[test]
fn closing_the_pane_ends_the_capture() {
    let daemon = TestDaemon::start();
    let mut client = daemon.attach_shell();
    client.wait_for_prompt().unwrap();
    let first = client.focused_pane_id();
    client.send(b"\x1b[109;5u");
    client.send(b"|");
    assert!(client.wait_until(WAIT, |c| c.screen_text().contains('│')));
    let second = client.focused_pane_id();
    daemon
        .command_from(Some(second), "pane.capture-keys", &[])
        .unwrap();
    assert!(client.wait_until(WAIT, |c| c.status_row().contains("RECORDING KEYS")));
    daemon
        .command_from(Some(second), "pane.close", &[])
        .unwrap();
    assert!(
        client.wait_until(WAIT, |c| !c.status_row().contains("RECORDING")),
        "{}",
        client.status_row()
    );
    // The other pane was not recorded, and the file is still there to read.
    assert_eq!(captures(&daemon).len(), 1);
    let _ = first;
}

/// Typed outside any pane, the command has no pane of its own: it acts on the focused pane of the
/// window being looked at, and the line it prints says which one that is.
#[test]
fn from_outside_a_pane_it_takes_the_focused_pane_and_says_which() {
    let daemon = TestDaemon::start();
    let mut client = daemon.attach_shell();
    client.wait_for_prompt().unwrap();
    let pane = client.focused_pane_id();
    let (ok, message) = daemon.command_from(None, "pane.capture-keys", &[]).unwrap();
    assert!(ok, "{message}");
    assert!(
        message.contains(&format!("in pane {pane} (window 1 ")),
        "{message}"
    );
    let (ok, message) = daemon.command_from(None, "pane.capture-keys", &[]).unwrap();
    assert!(ok && message.contains("stopped"), "{message}");
}
