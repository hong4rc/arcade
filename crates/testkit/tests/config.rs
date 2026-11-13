//! The configuration file at work: the author's file, reloading it, a file with mistakes, and the
//! status bar at the top. Every file lives in a directory of its own under the temporary
//! directory; the real configuration file is never read.

use std::path::PathBuf;
use std::sync::atomic::{AtomicUsize, Ordering};

use arcade_testkit::{AttachOptions, Screened, TestClient, TestDaemon, WAIT};

static COUNTER: AtomicUsize = AtomicUsize::new(0);

/// A directory for one test's configuration file.
fn config_dir() -> PathBuf {
    let dir = std::env::temp_dir().join(format!(
        "arc-config-{}-{}",
        std::process::id(),
        COUNTER.fetch_add(1, Ordering::Relaxed)
    ));
    std::fs::create_dir_all(&dir).unwrap();
    dir
}

fn daemon_with(path: &std::path::Path) -> TestDaemon {
    let path = path.to_path_buf();
    TestDaemon::start_with(move |c| c.config_path = Some(path))
}

fn run(daemon: &TestDaemon, name: &str, args: &[&str]) -> (bool, String) {
    daemon.command(name, args).unwrap()
}

fn row(client: &TestClient, y: usize) -> String {
    format!("{} ", client.screen_rows()[y])
}

fn wait_row(client: &mut TestClient, y: usize, text: &str) {
    let ok = client.wait_until(WAIT, |c| row(c, y).contains(text));
    assert!(ok, "row {y} lacks {text:?}:\n{}", client.screen_text());
}

const AUTHOR: &str = "[keys]\nprefix = \"ctrl+m\"\ndirect-modifier = \"ctrl+alt\"\nnav = \"hnei\"\n\n[status]\nposition = \"top\"\n";

#[test]
fn the_authors_file_gives_a_top_bar_and_its_keys() {
    let dir = config_dir();
    let file = dir.join("config.toml");
    std::fs::write(&file, AUTHOR).unwrap();
    let daemon = daemon_with(&file);
    let mut client = daemon
        .attach(AttachOptions::new().size(100, 30).named("author"))
        .unwrap();
    client.wait_for_prompt().unwrap();
    // The bar is the first row; the panes start under it and have one row less.
    wait_row(&mut client, 0, " author ");
    client.type_line("stty size");
    client.wait_for_text("29 100").unwrap();
    // The prefix is Ctrl+m as the new protocol reports it; Enter is not the prefix.
    client.send(b"\r");
    assert!(!row(&client, 0).contains("PREFIX"));
    client.send(b"\x1b[109;5u");
    wait_row(&mut client, 0, "PREFIX");
    client.send(b"|");
    // Ctrl+Alt+h moves left, and Ctrl+Alt+i right (movement keys are h n e i).
    let right = client.focused_pane_id();
    client.send(b"\x1b[104;7u");
    assert!(client.wait_until(WAIT, |c| c.focused_pane_id() != right));
    client.send(b"\x1b[105;7u");
    assert!(client.wait_until(WAIT, |c| c.focused_pane_id() == right));
}

#[test]
fn no_file_means_the_defaults() {
    let dir = config_dir();
    let daemon = daemon_with(&dir.join("not-there.toml"));
    let mut client = daemon
        .attach(AttachOptions::new().size(100, 30).named("plain"))
        .unwrap();
    client.wait_for_prompt().unwrap();
    // The bar is the last row.
    assert!(client.status_row().contains(" plain "));
}

#[test]
fn reloading_replaces_the_key_tables_and_a_file_with_mistakes_changes_nothing() {
    let dir = config_dir();
    let file = dir.join("config.toml");
    std::fs::write(&file, "[keys.prefix]\n\"q\" = \"pane.zoom\"\n").unwrap();
    let daemon = daemon_with(&file);
    let mut client = daemon
        .attach(AttachOptions::new().size(100, 30).named("w"))
        .unwrap();
    client.wait_for_prompt().unwrap();
    run(&daemon, "pane.split-right", &[]);
    client.send(b"\x01q");
    assert!(client.wait_until(WAIT, |c| c.status_row().contains("[Z]")));
    client.send(b"\x01q");
    assert!(client.wait_until(WAIT, |c| !c.status_row().contains("[Z]")));

    // A file with two mistakes: the old tables stay, and the lines are named.
    std::fs::write(
        &file,
        "[keys.prefix]\n\"q\" = \"pane.fly\"\n\"w\" = \"pane.zoom\"\n[nope]\n",
    )
    .unwrap();
    let (ok, message) = run(&daemon, "arcade.reload-config", &[]);
    assert!(!ok);
    assert!(message.contains("line 2"), "{message}");
    assert!(message.contains("1 more"), "{message}");
    client.send(b"\x01q");
    assert!(client.wait_until(WAIT, |c| c.status_row().contains("[Z]")));
    client.send(b"\x01q");
    assert!(client.wait_until(WAIT, |c| !c.status_row().contains("[Z]")));

    // A fixed file replaces the tables: `w` zooms now and `q` does nothing more.
    std::fs::write(&file, "[keys.prefix]\n\"w\" = \"pane.zoom\"\n").unwrap();
    let (ok, message) = run(&daemon, "arcade.reload-config", &[]);
    assert!(ok, "{message}");
    client.send(b"\x01w");
    assert!(client.wait_until(WAIT, |c| c.status_row().contains("[Z]")));
    client.send(b"\x01w");
    assert!(client.wait_until(WAIT, |c| !c.status_row().contains("[Z]")));
    client.send(b"\x01q");
    assert!(client.wait_until(WAIT, |c| c.status_row().contains("no key")));
}

#[test]
fn the_reload_key_says_so_on_the_status_bar() {
    let dir = config_dir();
    let file = dir.join("config.toml");
    std::fs::write(&file, "").unwrap();
    let daemon = daemon_with(&file);
    let mut client = daemon.attach_shell();
    client.wait_for_prompt().unwrap();
    client.send(b"\x01r");
    assert!(client.wait_until(WAIT, |c| c.status_row().contains("configuration reloaded")));
    std::fs::write(&file, "bogus = 1\n").unwrap();
    client.send(b"x");
    client.send(b"\x01r");
    assert!(client.wait_until(WAIT, |c| c.status_row().contains("line 1")));
}

#[test]
fn a_file_with_mistakes_does_not_stop_the_daemon_and_says_so_when_a_client_attaches() {
    let dir = config_dir();
    let file = dir.join("config.toml");
    std::fs::write(&file, "[keys]\nnav = \"abc\"\n").unwrap();
    let daemon = daemon_with(&file);
    let mut client = daemon.attach_shell();
    client.wait_for_prompt().unwrap();
    assert!(client.wait_until(WAIT, |c| c.status_row().contains("config: line 2")));
    assert!(!daemon.is_stopped());
}

#[test]
fn the_status_bar_can_move_and_hide_when_the_file_is_reloaded() {
    let dir = config_dir();
    let file = dir.join("config.toml");
    std::fs::write(&file, "").unwrap();
    let daemon = daemon_with(&file);
    let mut client = daemon
        .attach(AttachOptions::new().size(100, 30).named("w"))
        .unwrap();
    client.wait_for_prompt().unwrap();
    client.type_line("stty size");
    client.wait_for_text("29 100").unwrap();
    std::fs::write(&file, "[status]\nposition = \"top\"\n").unwrap();
    assert!(run(&daemon, "arcade.reload-config", &[]).0);
    wait_row(&mut client, 0, " w ");
    client.type_line("stty size");
    assert!(client.wait_until(WAIT, |c| c.screen_text().matches("29 100").count() >= 2));
    std::fs::write(&file, "[status]\nposition = \"off\"\n").unwrap();
    assert!(run(&daemon, "arcade.reload-config", &[]).0);
    client.type_line("stty size");
    client.wait_for_text("30 100").unwrap();
}

#[test]
fn window_numbers_follow_the_base_index_and_the_renumber_setting() {
    let dir = config_dir();
    let file = dir.join("config.toml");
    std::fs::write(&file, "[window]\nbase-index = 0\nrenumber = false\n").unwrap();
    let daemon = daemon_with(&file);
    let mut client = daemon
        .attach(AttachOptions::new().size(100, 30).named("w"))
        .unwrap();
    client.wait_for_prompt().unwrap();
    for _ in 0..2 {
        assert!(run(&daemon, "window.new", &[]).0);
    }
    let ok = client.wait_until(WAIT, |c| {
        let bar = format!("{} ", c.status_row());
        bar.contains(" 0 ") && bar.contains(" 1 ") && bar.contains(" 2 ")
    });
    assert!(ok, "{}", client.status_row());
    // Close the middle window: the numbers do not close up.
    assert!(run(&daemon, "window.select", &["1"]).0);
    assert!(run(&daemon, "pane.close", &[]).0);
    let ok = client.wait_until(WAIT, |c| {
        let bar = format!("{} ", c.status_row());
        bar.contains(" 0 ") && !bar.contains(" 1 ") && bar.contains(" 2 ")
    });
    assert!(ok, "{}", client.status_row());
    assert!(run(&daemon, "window.select", &["2"]).0);
    assert!(!run(&daemon, "window.select", &["1"]).0);
    // A new window takes the lowest free number.
    assert!(run(&daemon, "window.new", &[]).0);
    let ok = client.wait_until(WAIT, |c| format!("{} ", c.status_row()).contains(" 1 "));
    assert!(ok, "{}", client.status_row());
}
