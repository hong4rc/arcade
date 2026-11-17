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
    alpha.split_right();
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

/// A line of the tree without the place and the program a pane line adds (`in <directory>`,
/// `running <program>`), which differ from machine to machine.
fn plain(line: &str) -> String {
    let mut words = line.split_whitespace();
    let mut kept = Vec::new();
    while let Some(w) = words.next() {
        if w == "in" || w == "running" {
            words.next();
        } else {
            kept.push(w);
        }
    }
    kept.join(" ")
}

/// The text of the chosen line.
fn chosen(client: &TestClient) -> String {
    box_rows(client)
        .iter()
        .find_map(|r| {
            let inner = r.strip_prefix('│')?.trim_start();
            inner
                .strip_prefix("> ")
                .map(|t| plain(t.trim_end_matches([' ', '│']).trim()))
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

// ---- Lifting a pane and putting it down ----

/// The process number of the shell in the focused pane, printed under a tag of its own.
fn shell_pid(client: &mut TestClient, tag: &str) -> String {
    client.type_line(&format!("echo {tag}-$$"));
    let prefix = format!("{tag}-");
    let found = |c: &TestClient| {
        c.screen_rows().iter().rev().find_map(|r| {
            let t = r.trim_end();
            let digits = t.strip_prefix(&prefix).or_else(|| {
                t.rsplit_once(&prefix)
                    .map(|(_, d)| d)
                    .filter(|_| !t.contains("echo"))
            })?;
            (!digits.is_empty() && digits.chars().all(|ch| ch.is_ascii_digit()))
                .then(|| digits.to_string())
        })
    };
    assert!(
        client.wait_until(WAIT, |c| found(c).is_some()),
        "{}",
        client.screen_text()
    );
    found(client).unwrap()
}

/// Open the tree with the pane of the client lifted (`m`).
fn lift_pane(client: &mut TestClient) {
    client.send(PREFIX);
    client.send(b"m");
    let ok = client.wait_until(WAIT, |c| !box_rows(c).is_empty());
    assert!(ok, "{}", client.screen_text());
}

fn footer(client: &TestClient) -> String {
    box_rows(client)
        .iter()
        .rev()
        .nth(2)
        .map(|r| r.trim_matches(['│', ' ']).to_string())
        .unwrap_or_default()
}

fn wait_footer(client: &mut TestClient, text: &str) {
    let ok = client.wait_until(WAIT, |c| footer(c).contains(text));
    assert!(
        ok,
        "the bottom line is {:?}, not {text:?}\n{}",
        footer(client),
        client.screen_text()
    );
}

#[test]
fn m_opens_with_the_pane_lifted_and_the_choice_on_its_window() {
    let daemon = TestDaemon::start();
    let mut client = world(&daemon);
    lift_pane(&mut client);
    wait_chosen(&mut client, "▾ 1 editor *");
    let text = box_rows(&client).join("\n");
    assert!(text.contains("[lifted]"), "{text}");
    // Esc puts it down once and the tree stays; a second Esc closes.
    client.send(ESC);
    let ok = client.wait_until(WAIT, |c| !box_rows(c).join("\n").contains("[lifted]"));
    assert!(ok, "{}", client.screen_text());
    assert!(!box_rows(&client).is_empty());
    client.send(ESC);
    closed(&mut client);
}

#[test]
fn a_pane_put_on_another_pane_goes_beside_it_with_the_same_process() {
    let daemon = TestDaemon::start();
    let mut client = world(&daemon);
    let before = shell_pid(&mut client, "before");
    lift_pane(&mut client);
    client.send(DOWN);
    client.send(RIGHT);
    wait_chosen(&mut client, "htop");
    wait_footer(&mut client, "put pane beside");
    client.send(ENTER);
    closed(&mut client);
    let ok = client.wait_until(WAIT, |c| {
        let t = c.screen_text();
        t.contains("in-htop") && t.contains("in-zsh") && !t.contains("in-nvim") && t.contains('│')
    });
    assert!(ok, "{}", client.screen_text());
    // The target is on the left, the moved pane on the right with the focus.
    let rows = client.screen_rows();
    let row = rows.iter().find(|r| r.contains("in-htop")).unwrap();
    assert!(
        row.find("in-htop").unwrap() < row.find("in-zsh").unwrap(),
        "{row}"
    );
    let after = shell_pid(&mut client, "after");
    assert_eq!(before, after);
    // The old line is still in the history of the pane.
    assert!(client.screen_text().contains("in-zsh"));
}

#[test]
fn a_pane_put_on_a_window_goes_beside_the_pane_in_focus_there() {
    let daemon = TestDaemon::start();
    let mut client = world(&daemon);
    lift_pane(&mut client);
    client.send(DOWN);
    wait_chosen(&mut client, "▸ 2 server");
    client.send(ENTER);
    closed(&mut client);
    let ok = client.wait_until(WAIT, |c| {
        let t = c.screen_text();
        t.contains("in-htop") && t.contains("in-zsh") && !t.contains("in-nvim")
    });
    assert!(ok, "{}", client.screen_text());
}

#[test]
fn a_pane_put_on_a_session_becomes_its_last_window_and_the_client_follows() {
    let daemon = TestDaemon::start();
    let mut client = world(&daemon);
    let before = shell_pid(&mut client, "before");
    lift_pane(&mut client);
    client.send(LEFT);
    client.send(DOWN);
    wait_chosen(&mut client, "▸ beta");
    wait_footer(&mut client, "in a new window of beta");
    client.send(ENTER);
    closed(&mut client);
    let ok = client.wait_until(WAIT, |c| {
        bar(c).contains(" beta ") && c.screen_text().contains("in-zsh")
    });
    assert!(ok, "{}\n{}", client.screen_text(), bar(&client));
    assert!(!client.screen_text().contains("in-emacs"));
    assert!(bar(&client).contains(" 2 "), "{}", bar(&client));
    let after = shell_pid(&mut client, "after");
    assert_eq!(before, after);
}

#[test]
fn the_last_pane_of_a_window_takes_the_window_with_it() {
    let daemon = TestDaemon::start();
    let mut client = world(&daemon);
    client.send(PREFIX);
    client.send(b"2");
    assert!(client.wait_until(WAIT, |c| c.screen_text().contains("in-htop")));
    lift_pane(&mut client);
    client.send(LEFT);
    client.send(DOWN);
    client.send(RIGHT);
    client.send(RIGHT);
    wait_chosen(&mut client, "emacs");
    client.send(ENTER);
    closed(&mut client);
    let ok = client.wait_until(WAIT, |c| {
        bar(c).contains(" beta ")
            && c.screen_text().contains("in-htop")
            && c.screen_text().contains("in-emacs")
    });
    assert!(ok, "{}", client.screen_text());
    // Alpha has lost its second window.
    open_tree(&mut client);
    client.send(UP);
    client.send(LEFT);
    client.send(LEFT);
    client.send(RIGHT);
    let text = box_rows(&client).join("\n");
    assert!(!text.contains("2 server"), "{text}");
}

#[test]
fn the_only_pane_of_a_session_takes_the_session_with_it_and_the_client_lives_on() {
    let daemon = TestDaemon::start();
    let mut client = world(&daemon);
    client.send(PREFIX);
    client.send(b"s");
    // Go to beta through the list of sessions.
    client.send(b"beta");
    client.send(ENTER);
    assert!(client.wait_until(WAIT, |c| bar(c).contains(" beta ")));
    lift_pane(&mut client);
    client.send(LEFT);
    client.send(UP);
    client.send(RIGHT);
    client.send(RIGHT);
    wait_chosen(&mut client, "zsh");
    client.send(ENTER);
    closed(&mut client);
    let ok = client.wait_until(WAIT, |c| {
        bar(c).contains(" alpha ")
            && c.screen_text().contains("in-emacs")
            && c.screen_text().contains("in-zsh")
    });
    assert!(ok, "{}\n{}", client.screen_text(), bar(&client));
    assert!(!client.closed);
    let (_, sessions) = run(&daemon, "list-sessions", &[]);
    assert!(!sessions.contains("beta"), "{sessions}");
}

#[test]
fn a_place_that_cannot_take_the_pane_says_why_and_enter_changes_nothing() {
    let daemon = TestDaemon::start();
    let mut client = world(&daemon);
    // The pane itself.
    lift_pane(&mut client);
    client.send(RIGHT);
    wait_chosen(&mut client, "● zsh [lifted]");
    wait_footer(&mut client, "lifted");
    // Its own window: the pane is in it already.
    client.send(LEFT);
    wait_footer(&mut client, "already");
    let before = client.screen_text();
    client.send(ENTER);
    quiet(&mut client);
    assert!(!box_rows(&client).is_empty());
    client.send(ESC);
    client.send(ESC);
    closed(&mut client);
    assert_eq!(
        before.contains("in-zsh"),
        client.screen_text().contains("in-zsh")
    );
    assert!(client.screen_text().contains("in-nvim"));
}

#[test]
fn the_only_pane_of_a_session_cannot_be_put_in_a_new_window_of_its_own_session() {
    let daemon = TestDaemon::start();
    let mut client = world(&daemon);
    client.send(PREFIX);
    client.send(b"s");
    client.send(b"beta");
    client.send(ENTER);
    assert!(client.wait_until(WAIT, |c| bar(c).contains(" beta ")));
    lift_pane(&mut client);
    client.send(LEFT);
    wait_chosen(&mut client, "▾ beta *");
    wait_footer(&mut client, "alone");
    client.send(ENTER);
    quiet(&mut client);
    assert!(!box_rows(&client).is_empty());
}

#[test]
fn a_lifted_pane_that_exits_leaves_a_usable_tree_with_a_notice() {
    let daemon = TestDaemon::start();
    let mut client = world(&daemon);
    // The pane in focus is the one lifted; closing it through the daemon makes it vanish.
    lift_pane(&mut client);
    client.send(DOWN);
    wait_chosen(&mut client, "▸ 2 server");
    // The lifted pane's shell exits: kill its process group through the daemon's own command.
    assert!(run(&daemon, "pane.close", &[]).0);
    let ok = client.wait_until(WAIT, |c| {
        let t = box_rows(c).join("\n");
        !t.contains("[lifted]") && t.contains("is gone")
    });
    assert!(ok, "{}", client.screen_text());
    // The tree still answers.
    client.send(UP);
    quiet(&mut client);
    assert!(!box_rows(&client).is_empty());
}

// ---- Lifting a window and putting it down ----

/// Open the tree with the window of the client lifted (`M`).
fn lift_window(client: &mut TestClient) {
    client.send(PREFIX);
    client.send(b"M");
    let ok = client.wait_until(WAIT, |c| !box_rows(c).is_empty());
    assert!(ok, "{}", client.screen_text());
}

/// The windows the tree shows for a session when the client looks at it from another session:
/// walk to the session, open it, and read the box.
fn windows_of_other_session(client: &mut TestClient) -> String {
    open_tree(client);
    client.send(LEFT);
    client.send(LEFT);
    client.send(UP);
    client.send(RIGHT);
    quiet(client);
    let text = box_rows(client).join("\n");
    client.send(ESC);
    closed(client);
    text
}

#[test]
fn capital_m_opens_with_the_window_lifted_and_the_choice_on_its_session() {
    let daemon = TestDaemon::start();
    let mut client = world(&daemon);
    lift_window(&mut client);
    wait_chosen(&mut client, "▾ alpha *");
    let text = box_rows(&client).join("\n");
    assert!(
        text.contains("1 editor *") && text.contains("[lifted]"),
        "{text}"
    );
    client.send(ESC);
    let ok = client.wait_until(WAIT, |c| !box_rows(c).join("\n").contains("[lifted]"));
    assert!(ok, "{}", client.screen_text());
    assert!(!box_rows(&client).is_empty());
    client.send(ESC);
    closed(&mut client);
}

#[test]
fn a_window_put_on_a_window_of_its_session_stands_right_after_it() {
    let daemon = TestDaemon::start();
    let mut client = world(&daemon);
    lift_window(&mut client);
    client.send(RIGHT);
    client.send(DOWN);
    wait_chosen(&mut client, "▸ 2 server");
    wait_footer(&mut client, "put window after 2 server (alpha)");
    client.send(ENTER);
    closed(&mut client);
    let ok = client.wait_until(WAIT, |c| {
        let b = bar(c);
        matches!((b.find("server"), b.find("editor")), (Some(s), Some(e)) if s < e)
            && c.screen_text().contains("in-nvim")
            && c.screen_text().contains("in-zsh")
    });
    assert!(ok, "{}\n{}", client.screen_text(), bar(&client));
}

#[test]
fn a_window_put_on_a_window_of_another_session_moves_there_with_its_processes() {
    let daemon = TestDaemon::start();
    let mut client = world(&daemon);
    client.send(PREFIX);
    client.send(b"2");
    assert!(client.wait_until(WAIT, |c| c.screen_text().contains("in-htop")));
    let before = shell_pid(&mut client, "before");
    lift_window(&mut client);
    client.send(DOWN);
    client.send(RIGHT);
    wait_footer(&mut client, "put window after 1 ");
    client.send(ENTER);
    closed(&mut client);
    let ok = client.wait_until(WAIT, |c| {
        bar(c).contains(" beta ")
            && bar(c).contains("server")
            && c.screen_text().contains("in-htop")
    });
    assert!(ok, "{}\n{}", client.screen_text(), bar(&client));
    let after = shell_pid(&mut client, "after");
    assert_eq!(before, after);
    // Alpha keeps only its first window.
    let text = windows_of_other_session(&mut client);
    let alpha = &text[text.find("alpha").unwrap()..text.find("beta").unwrap()];
    assert!(
        alpha.contains("1 editor") && !alpha.contains("server"),
        "{text}"
    );
}

#[test]
fn a_window_put_on_a_session_becomes_its_last_window_and_the_client_follows() {
    let daemon = TestDaemon::start();
    let mut client = world(&daemon);
    lift_window(&mut client);
    client.send(DOWN);
    wait_chosen(&mut client, "▸ beta");
    wait_footer(&mut client, "at the end of beta");
    client.send(ENTER);
    closed(&mut client);
    let ok = client.wait_until(WAIT, |c| {
        let b = bar(c);
        b.contains(" beta ")
            && matches!((b.find(" 1 "), b.find("editor")), (Some(one), Some(e)) if one < e)
            && c.screen_text().contains("in-nvim")
            && c.screen_text().contains("in-zsh")
    });
    assert!(ok, "{}\n{}", client.screen_text(), bar(&client));
}

#[test]
fn a_window_put_on_its_own_session_goes_to_the_end_unless_it_is_there() {
    let daemon = TestDaemon::start();
    let mut client = world(&daemon);
    lift_window(&mut client);
    wait_footer(&mut client, "at the end of alpha");
    client.send(ENTER);
    closed(&mut client);
    let ok = client.wait_until(WAIT, |c| {
        let b = bar(c);
        matches!((b.find("server"), b.find("editor")), (Some(s), Some(e)) if s < e)
    });
    assert!(ok, "{}", bar(&client));
    // The window is the last now: the same again says why and does nothing.
    lift_window(&mut client);
    wait_footer(&mut client, "last of its session already");
    client.send(ENTER);
    quiet(&mut client);
    assert!(!box_rows(&client).is_empty());
}

#[test]
fn the_only_window_of_a_session_takes_the_session_with_it_and_the_client_lives_on() {
    let daemon = TestDaemon::start();
    let mut client = world(&daemon);
    client.send(PREFIX);
    client.send(b"s");
    client.send(b"beta");
    client.send(ENTER);
    assert!(client.wait_until(WAIT, |c| bar(c).contains(" beta ")));
    lift_window(&mut client);
    client.send(UP);
    wait_chosen(&mut client, "▸ alpha");
    client.send(ENTER);
    closed(&mut client);
    let ok = client.wait_until(WAIT, |c| {
        bar(c).contains(" alpha ") && c.screen_text().contains("in-emacs")
    });
    assert!(ok, "{}\n{}", client.screen_text(), bar(&client));
    assert!(!client.closed);
    let (_, sessions) = run(&daemon, "session.list", &[]);
    assert!(!sessions.contains("beta"), "{sessions}");
}

#[test]
fn a_window_cannot_be_put_on_a_pane_or_on_itself() {
    let daemon = TestDaemon::start();
    let mut client = world(&daemon);
    lift_window(&mut client);
    client.send(RIGHT);
    wait_chosen(&mut client, "▾ 1 editor * [lifted]");
    wait_footer(&mut client, "lifted");
    client.send(RIGHT);
    wait_footer(&mut client, "put on a window or a session");
    client.send(ENTER);
    quiet(&mut client);
    assert!(!box_rows(&client).is_empty());
    client.send(ESC);
    client.send(ESC);
    closed(&mut client);
    assert!(client.screen_text().contains("in-nvim"));
}

// ---- Rebinding the keys of the tree ----

#[test]
fn the_lift_key_can_be_rebound_and_the_old_one_goes_into_the_filter() {
    let dir = std::env::temp_dir().join(format!("arc-tree-keys-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let file = dir.join("config.toml");
    std::fs::write(
        &file,
        "[keys.list]\n\"x\" = \"list.lift\"\n\"space\" = false\n",
    )
    .unwrap();
    let path = file.clone();
    let daemon = TestDaemon::start_with(move |c| c.config_path = Some(path));
    let mut client = world(&daemon);
    open_tree(&mut client);
    client.send(b"x");
    let ok = client.wait_until(WAIT, |c| chosen(c).contains("[lifted]"));
    assert!(ok, "{}", client.screen_text());
    client.send(b"x");
    let ok = client.wait_until(WAIT, |c| !chosen(c).contains("[lifted]"));
    assert!(ok, "{}", client.screen_text());
    // Space is a character of the filter now: "nvi " matches nothing.
    client.send(b"nvi");
    let ok = client.wait_until(WAIT, |c| box_rows(c).iter().any(|r| r.contains("› nvim")));
    assert!(ok, "{}", client.screen_text());
    client.send(b" ");
    let ok = client.wait_until(WAIT, |c| !box_rows(c).iter().any(|r| r.contains("› nvim")));
    assert!(ok, "{}", client.screen_text());
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn a_pane_line_names_the_directory_and_the_program_in_the_foreground_and_the_filter_finds_them() {
    let daemon = TestDaemon::start();
    let mut client = world(&daemon);
    let dir = daemon
        .dir()
        .file_name()
        .and_then(|n| n.to_str())
        .unwrap()
        .to_string();
    open_tree(&mut client);
    let text = box_rows(&client).join("\n");
    assert!(text.contains(&format!("zsh in {dir}")), "{text}");
    assert!(!text.contains("running"), "{text}");
    client.send(ESC);
    closed(&mut client);
    // A program that runs in the pane in front of the shell shows by its name.
    client.type_line("sleep 31");
    std::thread::sleep(std::time::Duration::from_millis(300));
    open_tree(&mut client);
    let ok = client.wait_until(WAIT, |c| {
        box_rows(c)
            .iter()
            .any(|r| r.contains(&format!("zsh in {dir} running sleep")))
    });
    assert!(ok, "{}", client.screen_text());
    // The filter looks at the new parts too: the program, and the directory.
    client.send(b"sleep");
    let ok = client.wait_until(WAIT, |c| {
        let lines: Vec<String> = box_rows(c)
            .into_iter()
            .filter(|r| r.contains('›'))
            .collect();
        lines.len() == 1 && lines[0].contains("running sleep")
    });
    assert!(ok, "{}", client.screen_text());
    client.send(b"\x15");
    client.send(dir.as_bytes());
    let ok = client.wait_until(WAIT, |c| {
        box_rows(c).iter().filter(|r| r.contains('›')).count() >= 4
    });
    assert!(ok, "{}", client.screen_text());
    client.send(ESC);
    client.send(ESC);
    closed(&mut client);
    client.send(b"\x03");
}

// ---- The box keeps its place and size, and says how to use it ----

use std::time::Duration;

/// Where the box is on the screen, in characters.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct Frame {
    left: usize,
    top: usize,
    width: usize,
    height: usize,
}

fn frame(client: &TestClient) -> Option<Frame> {
    let rows = client.screen_rows();
    let top = rows.iter().position(|r| r.contains('┌'))?;
    let left = rows[top].chars().position(|c| c == '┌')?;
    let right = rows[top].chars().position(|c| c == '┐')?;
    let bottom = rows
        .iter()
        .skip(top)
        .position(|r| r.contains('└'))
        .map(|b| b + top)?;
    Some(Frame {
        left,
        top,
        width: right - left + 1,
        height: bottom - top + 1,
    })
}

/// Let the daemon answer the last key and the screen show it.
fn settle(client: &mut TestClient) {
    client.wait_until(Duration::from_millis(150), |_| false);
}

/// Send each key and check that the frame has not moved or changed size.
fn keep_the_frame(client: &mut TestClient, steps: &[(&str, &[u8])], base: Frame) {
    for (what, key) in steps {
        client.send(key);
        settle(client);
        assert_eq!(
            frame(client),
            Some(base),
            "the frame changed after {what}:\n{}",
            client.screen_text()
        );
    }
}

const SPACE: &[u8] = b" ";
const CTRL_U: &[u8] = b"\x1b[117;5u";

#[test]
fn the_tree_never_moves_or_changes_size_whatever_is_done_in_it() {
    let daemon = TestDaemon::start();
    let mut client = world(&daemon);
    open_tree(&mut client);
    let base = frame(&client).expect("a frame");
    assert!(base.left + base.width <= 100 && base.top + base.height <= 30);
    keep_the_frame(
        &mut client,
        &[
            ("down", DOWN),
            ("down", DOWN),
            ("down", DOWN),
            ("up", UP),
            ("left (close the window)", LEFT),
            ("left (to the session)", LEFT),
            ("left (close it)", LEFT),
            ("right (open it)", RIGHT),
            ("right", RIGHT),
            ("right", RIGHT),
            ("down to the second session", DOWN),
            ("lift", SPACE),
            ("up", UP),
            ("up", UP),
            ("down", DOWN),
            ("down", DOWN),
            ("down", DOWN),
            ("put it down", ESC),
            ("a filter that matches many: e", b"e"),
            ("a filter that matches one: m", b"m"),
            ("a filter that matches nothing", b"zzz"),
            ("backspace", b"\x7f"),
            ("clear the filter", CTRL_U),
            ("a filter again", b"n"),
            ("lift while filtered", SPACE),
            ("put it down", ESC),
            ("clear", CTRL_U),
        ],
        base,
    );
}

#[test]
fn the_session_list_and_the_window_list_keep_their_frame_under_a_filter() {
    let daemon = TestDaemon::start();
    let mut client = world(&daemon);
    for key in [b"s", b"w"] {
        client.send(PREFIX);
        client.send(key);
        let ok = client.wait_until(WAIT, |c| frame(c).is_some());
        assert!(ok, "{}", client.screen_text());
        let base = frame(&client).unwrap();
        keep_the_frame(
            &mut client,
            &[
                ("typing a", b"a"),
                ("typing more", b"l"),
                ("typing what matches nothing", b"qqq"),
                ("backspace", b"\x7f"),
                ("clear", CTRL_U),
                ("down", DOWN),
                ("up", UP),
            ],
            base,
        );
        client.send(ESC);
        // `closed` looks for the tree only: wait here until no box of any list is left.
        let gone = client.wait_until(WAIT, |c| frame(c).is_none());
        assert!(gone, "{}", client.screen_text());
    }
}

/// The last two rows inside the box: what Enter does, and the keys.
fn footers(client: &TestClient) -> (String, String) {
    let rows = box_rows(client);
    let inner = |r: &str| {
        r.trim_start_matches('│')
            .trim_end_matches('│')
            .trim()
            .to_string()
    };
    let n = rows.len();
    (inner(&rows[n - 3]), inner(&rows[n - 2]))
}

#[test]
fn the_key_line_says_which_keys_there_are_in_each_state() {
    let daemon = TestDaemon::start();
    let mut client = world(&daemon);
    open_tree(&mut client);
    let (_, keys) = footers(&client);
    for expected in [
        "up down move",
        "left right close/open",
        "enter go",
        "space lift",
        "type to filter",
        "esc close",
    ] {
        assert!(keys.contains(expected), "no {expected:?} in {keys:?}");
    }
    // Something lifted: where to put it, and how to let go.
    client.send(SPACE);
    let ok = client.wait_until(WAIT, |c| footers(c).1.contains("cancel the lift"));
    assert!(ok, "{}", client.screen_text());
    let (_, keys) = footers(&client);
    for expected in ["choose where", "enter put down", "esc cancel the lift"] {
        assert!(keys.contains(expected), "no {expected:?} in {keys:?}");
    }
    assert!(
        box_rows(&client)[0].contains("moving"),
        "{:?}",
        box_rows(&client)[0]
    );
    client.send(ESC);
    // A filter: how to clear it.
    client.send(b"e");
    let ok = client.wait_until(WAIT, |c| footers(c).1.contains("clear filter"));
    assert!(ok, "{}", client.screen_text());
    let (_, keys) = footers(&client);
    for expected in [
        "up down move",
        "enter go",
        "ctrl+u clear filter",
        "esc close",
    ] {
        assert!(keys.contains(expected), "no {expected:?} in {keys:?}");
    }
    assert!(
        !keys.contains("type to filter") && !keys.contains("lift"),
        "{keys:?}"
    );
}

#[test]
fn the_title_says_what_is_lifted() {
    let daemon = TestDaemon::start();
    let mut client = world(&daemon);
    open_tree(&mut client);
    assert!(
        box_rows(&client)[0].starts_with("┌ Tree ─"),
        "{:?}",
        box_rows(&client)[0]
    );
    client.send(SPACE);
    let ok = client.wait_until(WAIT, |c| {
        box_rows(c)[0].contains("Tree \u{b7} moving pane zsh")
    });
    assert!(ok, "{}", client.screen_text());
    client.send(ESC);
    let ok = client.wait_until(WAIT, |c| box_rows(c)[0].starts_with("┌ Tree ─"));
    assert!(ok, "{}", client.screen_text());
}

#[test]
fn enter_says_it_does_nothing_when_nothing_matches_and_really_does_nothing() {
    let daemon = TestDaemon::start();
    let mut client = world(&daemon);
    open_tree(&mut client);
    client.send(b"zzzz");
    let ok = client.wait_until(WAIT, |c| footers(c).0.contains("nothing matches"));
    assert!(ok, "{}", client.screen_text());
    assert!(
        footers(&client).0.starts_with("Enter does nothing"),
        "{:?}",
        footers(&client)
    );
    client.send(ENTER);
    settle(&mut client);
    // The tree is still open, and the client has not gone anywhere.
    assert!(frame(&client).is_some(), "{}", client.screen_text());
}

#[test]
fn what_enter_does_is_said_in_a_short_line() {
    let daemon = TestDaemon::start();
    let mut client = world(&daemon);
    open_tree(&mut client);
    client.send(SPACE);
    // Over the pane nvim, in the same window.
    client.send(UP);
    let ok = client.wait_until(WAIT, |c| footers(c).0.contains("put pane beside nvim"));
    assert!(ok, "{}", client.screen_text());
    let line = footers(&client).0;
    assert!(line.contains(" (alpha \u{203a} 1 editor)"), "{line:?}");
    assert!(line.chars().count() < 80, "{line:?}");
    client.send(SPACE);
}

#[test]
fn a_narrow_terminal_cuts_the_key_line_without_widening_or_breaking_the_frame() {
    let daemon = TestDaemon::start();
    let mut client = world_sized(&daemon, 44, 24);
    open_tree(&mut client);
    let f = frame(&client).expect("a frame");
    assert!(f.left + f.width <= 44, "{f:?}");
    let rows = box_rows(&client);
    let width = rows[0].chars().count();
    assert!(rows.iter().all(|r| r.chars().count() == width), "{rows:?}");
    // What did not fit ends in the ellipsis.
    let (_, keys) = footers(&client);
    assert!(keys.ends_with('\u{2026}'), "{keys:?}");
    keep_the_frame(&mut client, &[("down", DOWN), ("a filter", b"e")], f);
}

#[test]
fn a_title_longer_than_the_screen_is_cut_and_the_frame_stays_inside() {
    let daemon = TestDaemon::start();
    let mut client = world_sized(&daemon, 60, 24);
    client.type_line(&format!(
        "printf '\\033]2;{}\\007'; echo long-$((40 + 2))",
        "w".repeat(150)
    ));
    client.wait_for_text("long-42").unwrap();
    open_tree(&mut client);
    let f = frame(&client).expect("a frame");
    assert!(f.left + f.width <= 60 && f.top + f.height <= 24, "{f:?}");
    assert!(box_rows(&client).join("\n").contains('\u{2026}'));
    keep_the_frame(
        &mut client,
        &[("down", DOWN), ("up", UP), ("a filter", b"w")],
        f,
    );
}

#[test]
fn rebinding_the_lift_key_changes_the_key_line() {
    let dir = std::env::temp_dir().join(format!("arc-tree-keyline-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let file = dir.join("config.toml");
    std::fs::write(
        &file,
        "[keys.list]\n\"x\" = \"list.lift\"\n\"space\" = false\n",
    )
    .unwrap();
    let path = file.clone();
    let daemon = TestDaemon::start_with(move |c| c.config_path = Some(path));
    let mut client = world(&daemon);
    open_tree(&mut client);
    let (_, keys) = footers(&client);
    assert!(keys.contains("x lift"), "{keys:?}");
    assert!(!keys.contains("space"), "{keys:?}");
    let _ = std::fs::remove_dir_all(&dir);
}
