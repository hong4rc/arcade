//! The tree of sessions, windows and panes, through the daemon: looking, moving about, jumping,
//! and (below) lifting a pane or a window and putting it down somewhere else.
//!
//! The world of most tests: session `alpha` with two windows, `1 editor` with two panes (titles
//! `nvim` on the left and `zsh` on the right, the one with the focus) and `2 server` with one
//! pane (`htop`); session `beta` with one window and one pane (`emacs`). Each pane has printed a
//! line of its own, `in-<title>`.

use arcade_testkit::{AttachOptions, Screened, TestClient, TestDaemon, WAIT};

const PREFIX: &[u8] = b"\x1b[109;5u";
const ESC: &[u8] = b"\x1b[27u";
const UP: &[u8] = b"\x1b[A";
const DOWN: &[u8] = b"\x1b[B";
const RIGHT: &[u8] = b"\x1b[C";
const LEFT: &[u8] = b"\x1b[D";
const ENTER: &[u8] = b"\r";

fn run(daemon: &TestDaemon, name: &str, args: &[&str]) -> (bool, String) {
    daemon.command(name, args).unwrap()
}

/// Set the title of the pane and print a line that names it.
fn name_pane(client: &mut TestClient, title: &str) {
    client.type_line(&format!("printf '\\033]0;{title}\\007'; echo in-{title}"));
    let marker = format!("in-{title}");
    let ok = client.wait_until(WAIT, |c| {
        c.screen_rows()
            .iter()
            .any(|r| r.ends_with(&marker) && !r.contains("echo"))
    });
    assert!(ok, "{}", client.screen_text());
}

fn world_sized(daemon: &TestDaemon, cols: u16, rows: u16) -> TestClient {
    let mut beta = daemon
        .attach(AttachOptions::new().size(cols, rows).named("beta"))
        .unwrap();
    beta.wait_for_prompt().unwrap();
    name_pane(&mut beta, "emacs");
    beta.detach();
    assert!(beta.wait_closed(WAIT));

    let mut alpha = daemon
        .attach(AttachOptions::new().size(cols, rows).named("alpha"))
        .unwrap();
    alpha.wait_for_prompt().unwrap();
    name_pane(&mut alpha, "nvim");
    assert!(run(daemon, "window.rename", &["editor"]).0);
    alpha.send(PREFIX);
    alpha.send(b"|");
    assert!(alpha.wait_until(WAIT, |c| c.screen_text().contains('│')));
    name_pane(&mut alpha, "zsh");
    alpha.send(PREFIX);
    alpha.send(b"c");
    assert!(alpha.wait_until(WAIT, |c| !c.screen_text().contains("in-zsh")));
    alpha.wait_for_prompt().unwrap();
    name_pane(&mut alpha, "htop");
    assert!(run(daemon, "window.rename", &["server"]).0);
    alpha.send(PREFIX);
    alpha.send(b"1");
    assert!(alpha.wait_until(WAIT, |c| c.screen_text().contains("in-zsh")));
    alpha
}

fn world(daemon: &TestDaemon) -> TestClient {
    world_sized(daemon, 100, 30)
}

/// The inside of the box of the tree: the rows between the frame lines, text only.
fn box_rows(client: &TestClient) -> Vec<String> {
    let rows = client.screen_rows();
    let top = rows.iter().position(|r| r.contains("┌ Tree"));
    let bottom = rows.iter().position(|r| r.contains('└'));
    match (top, bottom) {
        (Some(t), Some(b)) if t < b => rows[t..=b]
            .iter()
            .map(|r| {
                let from = r.find(['┌', '│', '└']).unwrap_or(0);
                r[from..].to_string()
            })
            .collect(),
        _ => Vec::new(),
    }
}

fn open_tree(client: &mut TestClient) {
    client.send(PREFIX);
    client.send(b"t");
    let ok = client.wait_until(WAIT, |c| !box_rows(c).is_empty());
    assert!(ok, "{}", client.screen_text());
}

/// The text of the chosen line.
fn chosen(client: &TestClient) -> String {
    box_rows(client)
        .iter()
        .find_map(|r| {
            let inner = r.strip_prefix('│')?.trim_start();
            inner
                .strip_prefix("> ")
                .map(|t| t.trim_end_matches([' ', '│']).trim().to_string())
        })
        .unwrap_or_default()
}

fn wait_chosen(client: &mut TestClient, text: &str) {
    let ok = client.wait_until(WAIT, |c| chosen(c) == text);
    assert!(
        ok,
        "the choice is {:?}, not {text:?}:\n{}",
        chosen(client),
        client.screen_text()
    );
}

fn closed(client: &mut TestClient) {
    let ok = client.wait_until(WAIT, |c| box_rows(c).is_empty());
    assert!(ok, "{}", client.screen_text());
}

fn bar(client: &TestClient) -> String {
    format!("{} ", client.status_row())
}

#[test]
fn the_tree_has_three_tiers_with_the_place_of_the_client_open_and_marked() {
    let daemon = TestDaemon::start();
    let mut client = world_sized(&daemon, 80, 24);
    open_tree(&mut client);
    wait_chosen(&mut client, "● zsh");
    let rows = box_rows(&client);
    let text = rows.join("\n");
    for expected in [
        "▾ alpha *",
        "    ▾ 1 editor *",
        "      nvim",
        "● zsh",
        "▸ 2 server",
        "▸ beta",
        "Enter: go to pane zsh",
    ] {
        assert!(text.contains(expected), "no {expected:?} in\n{text}");
    }
    // The branches that hold nothing of the client are closed: no htop, no emacs.
    assert!(!text.contains("htop") && !text.contains("emacs"), "{text}");
    // The tiers are indented one beneath the other.
    let indent = |needle: &str| {
        let row = rows.iter().find(|r| r.contains(needle)).unwrap();
        row.find(needle).unwrap()
    };
    assert!(indent("1 editor") > indent("alpha"));
    assert!(indent("nvim") > indent("1 editor"));
    // It fits the screen.
    assert!(rows.iter().all(|r| r.chars().count() <= 80), "{text}");
}

#[test]
fn a_tree_longer_than_the_screen_scrolls_to_keep_the_choice_in_sight() {
    let daemon = TestDaemon::start();
    let mut client = world_sized(&daemon, 60, 9);
    open_tree(&mut client);
    // Open everything by walking over it: windows of alpha, then beta.
    client.send(LEFT);
    client.send(LEFT);
    client.send(DOWN);
    client.send(RIGHT);
    client.send(RIGHT);
    wait_chosen(&mut client, "emacs");
    assert!(client.screen_text().contains("emacs"));
    // The box never grows past the screen.
    assert!(box_rows(&client).len() <= 9);
}

#[test]
fn up_and_down_keep_to_a_tier_and_left_and_right_go_out_and_in() {
    let daemon = TestDaemon::start();
    let mut client = world(&daemon);
    open_tree(&mut client);
    wait_chosen(&mut client, "● zsh");
    // Down from the last pane of window 1 is the first pane of window 2.
    client.send(DOWN);
    wait_chosen(&mut client, "htop");
    client.send(UP);
    wait_chosen(&mut client, "● zsh");
    client.send(UP);
    wait_chosen(&mut client, "nvim");
    // The first of its tier: Up stays.
    client.send(UP);
    quiet(&mut client);
    wait_chosen(&mut client, "nvim");
    // Left out to the window, then to the session, then nowhere.
    client.send(LEFT);
    wait_chosen(&mut client, "▾ 1 editor *");
    client.send(LEFT);
    wait_chosen(&mut client, "▾ alpha *");
    client.send(LEFT);
    quiet(&mut client);
    wait_chosen(&mut client, "▾ alpha *");
    // Right goes in: the window the session shows, then the pane the window has the focus in.
    client.send(RIGHT);
    wait_chosen(&mut client, "▾ 1 editor *");
    client.send(RIGHT);
    wait_chosen(&mut client, "● zsh");
    client.send(RIGHT);
    quiet(&mut client);
    wait_chosen(&mut client, "● zsh");
}

fn quiet(client: &mut TestClient) {
    std::thread::sleep(std::time::Duration::from_millis(150));
    client.read_more(std::time::Instant::now() + std::time::Duration::from_millis(50));
}

#[test]
fn enter_goes_to_a_pane_of_another_session() {
    let daemon = TestDaemon::start();
    let mut client = world(&daemon);
    open_tree(&mut client);
    client.send(LEFT);
    client.send(LEFT);
    client.send(DOWN);
    client.send(RIGHT);
    client.send(RIGHT);
    wait_chosen(&mut client, "emacs");
    client.send(ENTER);
    closed(&mut client);
    let ok = client.wait_until(WAIT, |c| {
        bar(c).contains(" beta ") && c.screen_text().contains("in-emacs")
    });
    assert!(ok, "{}\n{}", client.screen_text(), bar(&client));
    // The pane has the focus: what is typed goes to it.
    client.type_line("echo typed-here");
    client.wait_for_row("typed-here").unwrap();
    assert!(!client.closed);
}

#[test]
fn enter_goes_to_a_window_and_to_a_pane_of_another_window() {
    let daemon = TestDaemon::start();
    let mut client = world(&daemon);
    open_tree(&mut client);
    client.send(DOWN);
    wait_chosen(&mut client, "htop");
    client.send(ENTER);
    closed(&mut client);
    let ok = client.wait_until(WAIT, |c| {
        c.screen_text().contains("in-htop") && !c.screen_text().contains("in-zsh")
    });
    assert!(ok, "{}", client.screen_text());
    // To a window: from the second window to the first.
    open_tree(&mut client);
    client.send(LEFT);
    client.send(UP);
    wait_chosen(&mut client, "▸ 1 editor");
    client.send(ENTER);
    closed(&mut client);
    let ok = client.wait_until(WAIT, |c| {
        c.screen_text().contains("in-zsh") && c.screen_text().contains("in-nvim")
    });
    assert!(ok, "{}", client.screen_text());
}

#[test]
fn escape_closes_the_tree_and_the_screen_is_as_it_was() {
    let daemon = TestDaemon::start();
    let mut client = world(&daemon);
    let before = format!("{:?}", client.screen.screen().unwrap());
    open_tree(&mut client);
    client.send(DOWN);
    client.send(RIGHT);
    client.send(ESC);
    closed(&mut client);
    let after = format!("{:?}", client.screen.screen().unwrap());
    assert_eq!(before, after);
}

#[test]
fn a_filter_shows_the_matching_panes_with_their_path_and_enter_jumps_to_the_one_chosen() {
    let daemon = TestDaemon::start();
    let mut client = world(&daemon);
    open_tree(&mut client);
    client.send(b"emacs");
    let ok = client.wait_until(WAIT, |c| {
        box_rows(c)
            .iter()
            .any(|r| r.contains("beta › 1 ") && r.contains("› emacs"))
    });
    assert!(ok, "{}", client.screen_text());
    let lines = box_rows(&client)
        .into_iter()
        .filter(|r| r.contains('›'))
        .count();
    assert_eq!(lines, 1, "{}", client.screen_text());
    client.send(ENTER);
    closed(&mut client);
    let ok = client.wait_until(WAIT, |c| bar(c).contains(" beta "));
    assert!(ok, "{}", client.screen_text());
}
