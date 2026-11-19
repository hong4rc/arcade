//! A tab with a state takes turns between its symbol and its number (`[status] state-number =
//! "alternate"`, one phase per `[status] alternate-interval`). `replace` keeps the symbol alone,
//! and then nothing ticks.

use std::path::PathBuf;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::time::{Duration, Instant};

use arcade_testkit::{AttachOptions, Screened, TestClient, TestDaemon, WAIT};

static COUNTER: AtomicUsize = AtomicUsize::new(0);
const PREFIX: &[u8] = b"\x1b[109;5u";

fn config_file(body: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!(
        "arc-alternate-{}-{}",
        std::process::id(),
        COUNTER.fetch_add(1, Ordering::Relaxed)
    ));
    std::fs::create_dir_all(&dir).unwrap();
    let file = dir.join("config.toml");
    std::fs::write(&file, body).unwrap();
    file
}

fn daemon_with(file: &std::path::Path) -> TestDaemon {
    let path = file.to_path_buf();
    TestDaemon::start_with(move |c| c.config_path = Some(path))
}

/// Two windows: window 1 has the pane `first`; the second is shown, and has an input notification.
fn with_input_on_the_shown_tab(daemon: &TestDaemon) -> TestClient {
    let mut client = daemon
        .attach(AttachOptions::new().size(100, 30).named("work"))
        .unwrap();
    client.wait_for_prompt().unwrap();
    client.send(PREFIX);
    client.send(b"c");
    client.wait_for_prompt().unwrap();
    let pane = client.focused_pane_id();
    let target = format!("target={pane}");
    let (ok, why) = daemon
        .command(
            "notify.send",
            &[
                "version=1",
                &target,
                "class=input",
                "source=test",
                "message=hi",
            ],
        )
        .unwrap();
    assert!(ok, "{why}");
    assert!(client.wait_until(WAIT, |c| c.status_row().contains('◆')));
    client
}

fn updates_in(client: &mut TestClient, time: Duration) -> usize {
    let end = Instant::now() + time;
    let mut n = 0;
    while Instant::now() < end {
        if client.read_more(end) {
            n += 1;
        }
    }
    n
}

fn reload(daemon: &TestDaemon, file: &std::path::Path, body: &str) {
    std::fs::write(file, body).unwrap();
    assert!(daemon.command("arcade.reload-config", &[]).unwrap().0);
}

#[test]
fn a_tab_with_a_state_takes_turns_by_default_and_replace_keeps_it_still() {
    let file = config_file("[status]\nblink = []\n");
    let daemon = daemon_with(&file);
    let mut client = with_input_on_the_shown_tab(&daemon);
    // The default alternates: the bar is drawn again at each phase, two seconds apart.
    assert!(updates_in(&mut client, Duration::from_millis(4500)) >= 2);
    // With `replace` the symbol stays, and nothing is drawn again.
    reload(
        &daemon,
        &file,
        "[status]\nblink = []\nstate-number = \"replace\"\n",
    );
    updates_in(&mut client, Duration::from_millis(600));
    assert_eq!(updates_in(&mut client, Duration::from_millis(2600)), 0);
    assert!(client.status_row().contains('◆'), "{}", client.status_row());
}

#[test]
fn turning_alternation_back_on_by_reload_starts_the_turns_again() {
    let file = config_file("[status]\nblink = []\nstate-number = \"replace\"\n");
    let daemon = daemon_with(&file);
    let mut client = with_input_on_the_shown_tab(&daemon);
    assert_eq!(updates_in(&mut client, Duration::from_millis(2600)), 0);
    reload(&daemon, &file, "[status]\nblink = []\n");
    assert!(updates_in(&mut client, Duration::from_millis(4500)) >= 2);
}

#[test]
fn number_off_has_no_number_to_take_turns_with_and_nothing_ticks() {
    let file = config_file("[status]\nblink = []\nnumber = \"off\"\n");
    let daemon = daemon_with(&file);
    let mut client = with_input_on_the_shown_tab(&daemon);
    assert_eq!(updates_in(&mut client, Duration::from_millis(2600)), 0);
    assert!(client.status_row().contains('◆'), "{}", client.status_row());
}

/// The bytes of the next frames that change only the bar (one per phase change), read from the raw
/// output the client has received. A frame that carries a pane's text (a shell echo, a prompt) is
/// not a phase change and is skipped.
fn frames_of_phase_changes(client: &mut TestClient, count: usize) -> Vec<Vec<u8>> {
    let mut frames = Vec::new();
    let mut seen = 0;
    while frames.len() < count && seen < 40 {
        let before_len = client.raw_output.len();
        let before_msgs = client.output_messages;
        let end = Instant::now() + Duration::from_secs(3);
        while client.output_messages == before_msgs && Instant::now() < end {
            client.read_more(end);
        }
        seen += 1;
        let frame = client.raw_output[before_len..].to_vec();
        let text = String::from_utf8_lossy(&frame);
        if text.contains("\x1b[30;") && !text.contains("$ ") {
            frames.push(frame);
        }
    }
    assert_eq!(frames.len(), count, "phase changes seen");
    frames
}

/// A phase change writes only the badge cell that changed, inside one synchronized update: no
/// clear of the screen or of a line, and no bytes of a pane.
fn assert_only_the_badge_changed(frame: &[u8]) {
    let text = String::from_utf8_lossy(frame);
    assert!(!text.contains("\x1b[2J"), "the screen is cleared: {text:?}");
    assert!(!text.contains("\x1b[K"), "a line is cleared: {text:?}");
    assert_eq!(
        text.matches("\x1b[?25l").count(),
        text.matches("\x1b[?25h").count()
    );
    assert!(text.matches("\x1b[?25l").count() <= 1, "{text:?}");
    assert!(
        text.contains("\x1b[30;"),
        "the bar row is not in the frame: {text:?}"
    );
    assert!(frame.len() < 200, "{} bytes: {text:?}", frame.len());
}

#[test]
fn a_phase_change_writes_only_the_badge_of_one_running_tab() {
    let file = config_file("[status]\nblink = []\nalternate-interval = 200\n");
    let daemon = daemon_with(&file);
    let mut client = daemon
        .attach(AttachOptions::new().size(110, 30).named("work"))
        .unwrap();
    client.wait_for_prompt().unwrap();
    client.type_line("printf '\\033]9;4;3;\\007'");
    assert!(client.wait_until(WAIT, |c| c.status_row().contains('▸')));
    client.read_more(Instant::now() + Duration::from_millis(800));
    let frames = frames_of_phase_changes(&mut client, 3);
    for frame in &frames {
        assert_only_the_badge_changed(frame);
    }
    eprintln!(
        "bytes per phase change, one running tab: {:?}",
        frames.iter().map(Vec::len).collect::<Vec<_>>()
    );
}

#[test]
fn a_phase_change_of_three_tabs_with_states_writes_only_their_badges() {
    let file = config_file("[status]\nblink = []\nalternate-interval = 200\n");
    let daemon = daemon_with(&file);
    let mut client = daemon
        .attach(AttachOptions::new().size(110, 30).named("work"))
        .unwrap();
    client.wait_for_prompt().unwrap();
    // Window 1 runs, window 2 waits for input, window 3 (shown) runs.
    client.type_line("printf '\\033]9;4;3;\\007'");
    assert!(client.wait_until(WAIT, |c| c.status_row().contains('▸')));
    for _ in 0..2 {
        client.send(PREFIX);
        client.send(b"c");
        client.wait_for_prompt().unwrap();
    }
    let second = {
        let (ok, _) = daemon.command("window.select", &["2"]).unwrap();
        assert!(ok);
        client.focused_pane_id()
    };
    let target = format!("target={second}");
    let (ok, why) = daemon
        .command(
            "notify.send",
            &[
                "version=1",
                &target,
                "class=input",
                "source=test",
                "message=hi",
            ],
        )
        .unwrap();
    assert!(ok, "{why}");
    client.type_line("printf '\\033]9;4;3;\\007'");
    assert!(client.wait_until(WAIT, |c| c.status_row().matches('▸').count() >= 1));
    // Let the shell's echo of the command settle: only phase changes are taken from here on.
    client.read_more(Instant::now() + Duration::from_millis(2500));
    let frames = frames_of_phase_changes(&mut client, 3);
    for frame in &frames {
        let text = String::from_utf8_lossy(frame);
        assert!(
            !text.contains("\x1b[2J") && !text.contains("\x1b[K"),
            "{text:?}"
        );
        assert!(frame.len() < 400, "{} bytes", frame.len());
    }
    eprintln!(
        "bytes per phase change, three tabs with states: {:?}",
        frames.iter().map(Vec::len).collect::<Vec<_>>()
    );
}
