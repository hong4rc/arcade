//! The title of the host terminal's window: what the focused pane's program calls itself, then
//! the session and the window number; sent when it changes and only then.

use std::time::Duration;

use arcade_testkit::{AttachOptions, Screened, TestClient, TestDaemon, WAIT, wait_for_file};

fn run(daemon: &TestDaemon, name: &str, args: &[&str]) {
    let (ok, output) = daemon.command(name, args).unwrap();
    assert!(ok, "{name} {args:?}: {output}");
}

fn attach(daemon: &TestDaemon) -> TestClient {
    let mut client = daemon
        .attach(AttachOptions::new().size(100, 30).named("thu"))
        .unwrap();
    client.wait_for_prompt().unwrap();
    client
}

fn last_title(client: &TestClient) -> String {
    client.titles.last().cloned().unwrap_or_default()
}

fn wait_title(client: &mut TestClient, wanted: &str) {
    let ok = client.wait_until(WAIT, |c| last_title(c).starts_with(wanted));
    assert!(ok, "titles so far: {:?}", client.titles);
}

#[test]
fn attaching_gives_the_window_name_the_session_and_the_window_number() {
    let daemon = TestDaemon::start();
    let client = attach(&daemon);
    let title = last_title(&client);
    assert!(title.ends_with(" \u{b7} thu:1"), "{title:?}");
    // The pane has set no title, so the front of it is the window's name, which is not empty.
    assert!(title.len() > " \u{b7} thu:1".len(), "{title:?}");
}

#[test]
fn the_title_a_pane_sets_goes_to_the_terminal_and_a_second_window_changes_the_number() {
    let daemon = TestDaemon::start();
    let mut client = attach(&daemon);
    client.type_line("printf '\\033]2;abc\\007'");
    wait_title(&mut client, "abc");
    assert_eq!(last_title(&client), "abc \u{b7} thu:1");
    run(&daemon, "window.new", &[]);
    assert!(client.wait_until(WAIT, |c| last_title(c).ends_with(":2")));
}

#[test]
fn moving_the_focus_to_another_pane_brings_that_pane_s_title() {
    let daemon = TestDaemon::start();
    let mut client = attach(&daemon);
    client.type_line("printf '\\033]2;left-one\\007'");
    wait_title(&mut client, "left-one");
    run(&daemon, "pane.split-right", &[]);
    // The new pane has set no title: its window's name is shown.
    assert!(client.wait_until(WAIT, |c| !last_title(c).starts_with("left-one")));
    std::thread::sleep(Duration::from_millis(300));
    run(&daemon, "pane.focus-left", &[]);
    wait_title(&mut client, "left-one");
}

#[test]
fn a_pane_without_the_focus_setting_a_title_sends_nothing() {
    let daemon = TestDaemon::start();
    let mut client = attach(&daemon);
    // This shell sets a title a second from now, after the focus has moved to a new pane, and
    // then writes a file so the test knows it has.
    let marker = std::env::temp_dir().join(format!("arc-title-{}-behind", std::process::id()));
    let _ = std::fs::remove_file(&marker);
    client.type_line(&format!(
        "(sleep 1; printf '\\033]2;from-behind\\007'; touch '{}') &",
        marker.display()
    ));
    client.wait_for_text("$").unwrap();
    run(&daemon, "pane.split-right", &[]);
    assert!(client.wait_until(WAIT, |c| c.focused_pane_id() > 0));
    assert!(
        wait_for_file(&marker, WAIT),
        "the late title was never printed"
    );
    client.read_more(std::time::Instant::now() + Duration::from_millis(200));
    let _ = std::fs::remove_file(&marker);
    assert!(
        client.titles.iter().all(|t| !t.contains("from-behind")),
        "{:?}",
        client.titles
    );
}

#[test]
fn control_characters_in_a_title_are_dropped() {
    let daemon = TestDaemon::start();
    let mut client = attach(&daemon);
    client.type_line("printf '\\033]2;a\\001b\\037c\\007'");
    wait_title(&mut client, "abc");
    assert_eq!(last_title(&client), "abc \u{b7} thu:1");
}

/// A title is the program's own words: a long one is cut, and the host's window title never grows
/// past the limit however often it is set.
#[test]
fn a_long_title_is_cut_to_the_limit() {
    let daemon = TestDaemon::start();
    let mut client = attach(&daemon);
    client.type_line(
        "x=$(printf '%01000d' 0 | tr 0 q); printf '\\033]2;%s\\007' \"$x\"; echo set-$((40 + 2))",
    );
    client.wait_for_text("set-42").unwrap();
    wait_title(&mut client, "qqqq");
    for title in &client.titles {
        assert!(
            title.chars().count() <= 200 + " \u{b7} thu:1".len(),
            "{}",
            title.len()
        );
    }
    assert_eq!(
        last_title(&client).chars().filter(|c| *c == 'q').count(),
        200
    );
}

#[test]
fn the_title_is_not_sent_again_while_nothing_changes() {
    let daemon = TestDaemon::start();
    let mut client = attach(&daemon);
    client.type_line("printf '\\033]2;steady\\007'");
    wait_title(&mut client, "steady");
    let before = client.titles.len();
    // Output that does not change the title does not send it again.
    client.type_line("echo some-more-output");
    client.wait_for_row("some-more-output").unwrap();
    std::thread::sleep(Duration::from_millis(300));
    client.read_more(std::time::Instant::now() + Duration::from_millis(100));
    assert_eq!(client.titles.len(), before, "{:?}", client.titles);
}
