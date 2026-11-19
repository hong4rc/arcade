//! The look of the states from the configuration file: a symbol and a colour of the author's
//! choice reach the tab and the corner mark of a pane, a reload changes them without a restart,
//! and a file with a wrong entry is refused with the line named. Every file lives in a directory
//! of its own under the temporary directory.

use std::path::PathBuf;
use std::sync::atomic::{AtomicUsize, Ordering};

use arcade_testkit::{AttachOptions, Screened, TestClient, TestDaemon, WAIT};

static COUNTER: AtomicUsize = AtomicUsize::new(0);

fn config_dir() -> PathBuf {
    let dir = std::env::temp_dir().join(format!(
        "arc-state-look-{}-{}",
        std::process::id(),
        COUNTER.fetch_add(1, Ordering::Relaxed)
    ));
    std::fs::create_dir_all(&dir).unwrap();
    dir
}

fn notify(daemon: &TestDaemon, pane: u64, class: &str) {
    let target = format!("target={pane}");
    let class = format!("class={class}");
    let (ok, why) = daemon
        .command(
            "notify.send",
            &["version=1", &target, &class, "source=test", "message=hi"],
        )
        .unwrap();
    assert!(ok, "{why}");
}

/// A window of two panes with the focus in the second; the first pane's number is returned.
fn two_panes(client: &mut TestClient) -> u64 {
    let first = client.focused_pane_id();
    client.send(b"\x1b[109;5u");
    client.send(b"|");
    assert!(client.wait_until(WAIT, |c| c.screen_text().contains('│')));
    first
}

fn start(daemon: &TestDaemon) -> TestClient {
    let mut client = daemon
        .attach(AttachOptions::new().size(100, 30).named("work"))
        .unwrap();
    client.wait_for_prompt().unwrap();
    client
}

fn reload(daemon: &TestDaemon) -> (bool, String) {
    daemon.command("arcade.reload-config", &[]).unwrap()
}

#[test]
fn a_symbol_from_the_file_reaches_the_tab_and_the_corner_and_a_reload_changes_it() {
    let dir = config_dir();
    let file = dir.join("config.toml");
    std::fs::write(
        &file,
        "[status.state.input]\nsymbol = \"X\"\ncolor = \"#102030\"\n",
    )
    .unwrap();
    let path = file.clone();
    let daemon = TestDaemon::start_with(move |c| c.config_path = Some(path));
    let mut client = start(&daemon);
    let first = two_panes(&mut client);
    notify(&daemon, first, "input");
    // The tab and the corner of the pane out of focus: two cells of the symbol.
    assert!(client.wait_until(WAIT, |c| {
        c.screen_text().chars().filter(|ch| *ch == 'X').count() == 2
    }));
    assert!(!client.screen_text().contains('◆'));
    // The colour reaches the cell as given.
    let screen = client.screen_rows();
    let row = screen.iter().position(|r| r.contains('X')).unwrap();
    let col = screen[row].chars().position(|ch| ch == 'X').unwrap();
    let style = client.style_at(row as u16, col as u16);
    assert_eq!(
        style.fg,
        arcade_term::Color::Rgb(0x10, 0x20, 0x30),
        "{style:?}"
    );

    // Reloaded with another symbol: no restart, the new symbol shows, the old one is gone.
    std::fs::write(&file, "[status.state.input]\nsymbol = \"Y\"\n").unwrap();
    let (ok, message) = reload(&daemon);
    assert!(ok, "{message}");
    assert!(client.wait_until(WAIT, |c| {
        let text = c.screen_text();
        text.chars().filter(|ch| *ch == 'Y').count() == 2 && !text.contains('X')
    }));
    // Reloaded with the entry gone: the default symbol is back.
    std::fs::write(&file, "").unwrap();
    let (ok, message) = reload(&daemon);
    assert!(ok, "{message}");
    assert!(client.wait_until(WAIT, |c| {
        let text = c.screen_text();
        text.contains('◆') && !text.contains('Y')
    }));
}

#[test]
fn the_style_from_the_file_changes_the_tab_on_a_reload() {
    let dir = config_dir();
    let file = dir.join("config.toml");
    std::fs::write(&file, "[status]\nstate-style = \"number\"\n").unwrap();
    let path = file.clone();
    let daemon = TestDaemon::start_with(move |c| c.config_path = Some(path));
    let mut client = start(&daemon);
    let first = client.focused_pane_id();
    notify(&daemon, first, "input");
    // `number`: the number stays on the tab, the symbol is not drawn on it.
    assert!(client.wait_until(WAIT, |c| c.status_row().contains(" 1 ")));
    assert!(!client.status_row().contains('◆'));
    std::fs::write(&file, "[status]\nstate-style = \"plain\"\n").unwrap();
    let (ok, message) = reload(&daemon);
    assert!(ok, "{message}");
    assert!(client.wait_until(WAIT, |c| c.status_row().contains('◆')));
}

#[test]
fn a_wrong_entry_is_refused_with_its_line_and_changes_nothing() {
    let dir = config_dir();
    let file = dir.join("config.toml");
    std::fs::write(&file, "[status.state.input]\nsymbol = \"X\"\n").unwrap();
    let path = file.clone();
    let daemon = TestDaemon::start_with(move |c| c.config_path = Some(path));
    let mut client = start(&daemon);
    let first = client.focused_pane_id();
    notify(&daemon, first, "input");
    assert!(client.wait_until(WAIT, |c| c.status_row().contains('X')));
    for (body, expect) in [
        ("[status.state.nope]\nsymbol = \"X\"\n", "line 1"),
        ("[status.state.input]\nsymbol = \"\"\n", "line 2"),
        ("[status.state.input]\nsymbol = \"ab\"\n", "line 2"),
        ("[status.state.input]\ncolor = \"red\"\n", "line 2"),
        ("[status]\nstate-style = \"loud\"\n", "line 2"),
    ] {
        std::fs::write(&file, body).unwrap();
        let (ok, message) = reload(&daemon);
        assert!(!ok, "{body}");
        assert!(message.contains(expect), "{body}: {message}");
    }
    // Nothing changed: the symbol of the last good file is still on the tab.
    assert!(client.status_row().contains('X'), "{}", client.status_row());
}
