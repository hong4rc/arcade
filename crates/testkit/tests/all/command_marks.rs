//! What the tab of a window says about the command its shell runs, by the marks a shell prompt
//! leaves in the output (OSC 133): running after a while, and the end of a long command that
//! nobody watched. The panes here run a small script that prints the marks, not a shell.

use std::time::Duration;

use arcade_testkit::{AttachOptions, Screened, TestClient, TestDaemon, WAIT};

const RUNNING: char = '▸';
const FINISHED_WELL: char = '●';
const FINISHED_WITH_ERROR: char = '✗';

fn run(daemon: &TestDaemon, name: &str, args: &[&str]) {
    let (ok, output) = daemon.command(name, args).unwrap();
    assert!(ok, "{name} {args:?}: {output}");
}

/// A session whose first window runs `script` (`C` and `D` marks are made with `mark C` and
/// `mark D;<code>`). When `look_away` is set a second window is made and shown, so that the first
/// one is out of sight.
fn start(
    running_after: u64,
    finished_after: u64,
    script: &str,
    look_away: bool,
) -> (TestDaemon, TestClient) {
    let daemon = TestDaemon::start_with(move |c| {
        c.config.command_running_after = Duration::from_secs(running_after);
        c.config.command_finished_after = Duration::from_secs(finished_after);
    });
    let script =
        format!("mark() {{ printf '\\033]133;%s\\033\\\\' \"$1\"; }}; {script}; sleep 120");
    let client = daemon
        .attach(
            AttachOptions::new()
                .size(100, 30)
                .named("work")
                .command(&["/bin/sh", "-c", &script]),
        )
        .unwrap();
    let mut client = client;
    if look_away {
        run(&daemon, "window.new", &[]);
        client.wait_for_prompt().unwrap();
    }
    (daemon, client)
}

fn bar(client: &TestClient) -> String {
    client.status_row()
}

fn has(client: &mut TestClient, mark: char) -> bool {
    client.wait_until(WAIT, |c| bar(c).contains(mark))
}

fn never(client: &mut TestClient, marks: &[char], for_how_long: Duration) -> bool {
    !client.wait_until(for_how_long, |c| marks.iter().any(|m| bar(c).contains(*m)))
}

#[test]
fn a_command_shows_as_running_only_after_its_time_and_not_without_marks() {
    let (_daemon, mut client) = start(2, 0, "mark C", false);
    // Not at once.
    assert!(
        never(&mut client, &[RUNNING], Duration::from_millis(800)),
        "{}",
        bar(&client)
    );
    assert!(has(&mut client, RUNNING), "{}", bar(&client));

    // A shell that sends no marks never shows a state.
    let (_daemon, mut client) = start(1, 1, "true", false);
    assert!(
        never(
            &mut client,
            &[RUNNING, FINISHED_WELL, FINISHED_WITH_ERROR],
            Duration::from_millis(2500)
        ),
        "{}",
        bar(&client)
    );
}

#[test]
fn a_long_command_that_ends_out_of_sight_leaves_its_result_until_the_pane_is_looked_at() {
    // Ends with an error after two seconds, in a window that is not shown.
    let (daemon, mut client) = start(0, 1, "mark C; sleep 2; mark 'D;1'", true);
    assert!(has(&mut client, FINISHED_WITH_ERROR), "{}", bar(&client));
    run(&daemon, "window.select", &["1"]);
    assert!(
        client.wait_until(WAIT, |c| !bar(c).contains(FINISHED_WITH_ERROR)),
        "{}",
        bar(&client)
    );

    // Ends well.
    let (daemon, mut client) = start(0, 1, "mark C; sleep 2; mark 'D;0'", true);
    assert!(has(&mut client, FINISHED_WELL), "{}", bar(&client));
    run(&daemon, "window.select", &["1"]);
    assert!(
        client.wait_until(WAIT, |c| !bar(c).contains(FINISHED_WELL)),
        "{}",
        bar(&client)
    );
}

#[test]
fn a_short_command_or_one_that_ends_while_you_watch_leaves_nothing() {
    // Short: it ends before the time that counts.
    let (_daemon, mut client) = start(0, 3, "mark C; sleep 1; mark 'D;1'", true);
    assert!(
        never(
            &mut client,
            &[FINISHED_WELL, FINISHED_WITH_ERROR],
            Duration::from_millis(2500)
        ),
        "{}",
        bar(&client)
    );
    // Long, but the window is the one shown.
    let (_daemon, mut client) = start(0, 1, "mark C; sleep 2; mark 'D;1'", false);
    assert!(
        never(
            &mut client,
            &[FINISHED_WELL, FINISHED_WITH_ERROR],
            Duration::from_millis(3500)
        ),
        "{}",
        bar(&client)
    );
}

#[test]
fn zero_turns_both_states_off() {
    let (_daemon, mut client) = start(0, 0, "mark C; sleep 2; mark 'D;1'", true);
    assert!(
        never(
            &mut client,
            &[RUNNING, FINISHED_WELL, FINISHED_WITH_ERROR],
            Duration::from_millis(3500)
        ),
        "{}",
        bar(&client)
    );
}

#[test]
fn the_tree_says_it_in_words() {
    let (_daemon, mut client) = start(1, 1, "mark C", true);
    // The first window has run its command for over a second by the time the tab shows it.
    assert!(
        client.wait_until(WAIT, |c| bar(c).contains(RUNNING)),
        "{}",
        bar(&client)
    );
    client.send(b"\x1b[109;5u");
    client.send(b"t");
    assert!(
        client.wait_until(WAIT, |c| c.screen_text().contains("(running ")),
        "{}",
        client.screen_text()
    );
}
