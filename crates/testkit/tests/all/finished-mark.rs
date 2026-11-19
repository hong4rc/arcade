//! The mark of a command that ended after a long run (the shell's prompt marks) is left on the
//! tab only for a pane that nobody looks at: it goes the moment its pane is looked at, and it is
//! not raised for a command that ends in the pane that is looked at.

use arcade_testkit::{AttachOptions, Screened, TestClient, TestDaemon};
use std::time::Duration;

const MARK: &str = "\u{25cf}";

fn config() -> std::path::PathBuf {
    let dir = std::env::temp_dir().join(format!("arc-finished-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let file = dir.join("config.toml");
    std::fs::write(&file, "[status]\nfinished-after = 1\n").unwrap();
    file
}

/// A command that runs for `seconds` as far as the shell's marks tell.
fn long_command(client: &mut TestClient, seconds: u32, code: u32) {
    client.type_line(&format!(
        "printf '\\033]133;C\\007'; sleep {seconds}; printf '\\033]133;D;{code}\\007'"
    ));
}

fn bar_has_mark(client: &mut TestClient) -> bool {
    client.status_row().contains(MARK)
}

#[test]
fn a_command_that_ends_out_of_sight_leaves_a_mark_that_goes_when_its_pane_is_looked_at() {
    let path = config();
    let daemon = TestDaemon::start_with(|c| c.config_path = Some(path.clone()));
    let mut client = daemon.attach(AttachOptions::new().size(100, 30)).unwrap();
    client.wait_for_prompt().unwrap();
    long_command(&mut client, 2, 0);
    let (ok, why) = daemon.command("window.new", &[]).unwrap();
    assert!(ok, "{why}");
    // It ends while the first window is hidden: its tab shows the mark.
    assert!(
        client.wait_until(Duration::from_secs(6), bar_has_mark),
        "no mark: {}",
        client.status_row()
    );
    // Selecting that window takes it away at once, with nothing else changing on the bar.
    let (ok, why) = daemon.command("window.select", &["1"]).unwrap();
    assert!(ok, "{why}");
    assert!(
        client.wait_until(Duration::from_secs(2), |c| !bar_has_mark(c)),
        "the mark stayed: {}",
        client.status_row()
    );
}

#[test]
fn a_command_that_ends_in_the_pane_that_is_looked_at_leaves_no_mark() {
    let path = config();
    let daemon = TestDaemon::start_with(|c| c.config_path = Some(path.clone()));
    let mut client = daemon.attach(AttachOptions::new().size(100, 30)).unwrap();
    client.wait_for_prompt().unwrap();
    long_command(&mut client, 2, 0);
    client.wait_until(Duration::from_secs(5), |_| false);
    assert!(!bar_has_mark(&mut client), "{}", client.status_row());
}

#[test]
fn a_failed_command_is_taken_away_the_same_way() {
    let path = config();
    let daemon = TestDaemon::start_with(|c| c.config_path = Some(path.clone()));
    let mut client = daemon.attach(AttachOptions::new().size(100, 30)).unwrap();
    client.wait_for_prompt().unwrap();
    long_command(&mut client, 2, 1);
    let (ok, why) = daemon.command("window.new", &[]).unwrap();
    assert!(ok, "{why}");
    assert!(
        client.wait_until(Duration::from_secs(6), |c| c
            .status_row()
            .contains('\u{2717}')),
        "no mark: {}",
        client.status_row()
    );
    let (ok, why) = daemon.command("window.select", &["1"]).unwrap();
    assert!(ok, "{why}");
    assert!(
        client.wait_until(Duration::from_secs(2), |c| !c
            .status_row()
            .contains('\u{2717}')),
        "the mark stayed: {}",
        client.status_row()
    );
}

#[test]
fn a_command_that_ends_in_a_hidden_pane_of_the_window_leaves_a_mark_that_goes_when_it_is_drawn() {
    let path = config();
    let daemon = TestDaemon::start_with(|c| c.config_path = Some(path.clone()));
    let mut client = daemon.attach(AttachOptions::new().size(100, 30)).unwrap();
    client.wait_for_prompt().unwrap();
    // The left pane runs the command; the focus goes to the right pane, which is zoomed, while
    // the command runs, so the left pane is not drawn.
    long_command(&mut client, 2, 0);
    let (ok, why) = daemon.command("pane.split-right", &[]).unwrap();
    assert!(ok, "{why}");
    let (ok, why) = daemon.command("pane.zoom", &[]).unwrap();
    assert!(ok, "{why}");
    assert!(
        client.wait_until(Duration::from_secs(6), bar_has_mark),
        "no mark: {}",
        client.status_row()
    );
    // Zoom off: the left pane is drawn, so it is looked at.
    let (ok, why) = daemon.command("pane.zoom", &[]).unwrap();
    assert!(ok, "{why}");
    assert!(
        client.wait_until(Duration::from_secs(2), |c| !bar_has_mark(c)),
        "the mark stayed: {}",
        client.status_row()
    );
}

#[test]
fn a_command_that_ends_in_a_pane_that_is_drawn_without_the_focus_leaves_its_mark_for_a_glance() {
    let path = config();
    let daemon = TestDaemon::start_with(|c| c.config_path = Some(path.clone()));
    let mut client = daemon.attach(AttachOptions::new().size(100, 30)).unwrap();
    client.wait_for_prompt().unwrap();
    long_command(&mut client, 2, 0);
    let (ok, why) = daemon.command("pane.split-right", &[]).unwrap();
    assert!(ok, "{why}");
    // The pane is drawn but not in focus: the mark shows, and goes by itself after the glance.
    assert!(
        client.wait_until(Duration::from_secs(6), bar_has_mark),
        "no mark: {}",
        client.status_row()
    );
    assert!(
        client.wait_until(Duration::from_secs(6), |c| !bar_has_mark(c)),
        "the mark stayed: {}",
        client.status_row()
    );
}

#[test]
fn a_command_that_ends_while_the_terminal_is_not_looked_at_leaves_a_mark_that_goes_on_return() {
    let path = config();
    let daemon = TestDaemon::start_with(|c| c.config_path = Some(path.clone()));
    let mut client = daemon.attach(AttachOptions::new().size(100, 30)).unwrap();
    client.wait_for_prompt().unwrap();
    long_command(&mut client, 2, 0);
    // The host terminal loses the focus (the person looks at another program).
    client.send(b"\x1b[O");
    assert!(
        client.wait_until(Duration::from_secs(6), bar_has_mark),
        "no mark: {}",
        client.status_row()
    );
    client.send(b"\x1b[I");
    assert!(
        client.wait_until(Duration::from_secs(2), |c| !bar_has_mark(c)),
        "the mark stayed: {}",
        client.status_row()
    );
}
