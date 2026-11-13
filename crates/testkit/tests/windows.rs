//! Several windows in one session: the tabs on the status bar, moving between windows and moving
//! windows along the bar, and the names the tabs carry.

use std::time::{Duration, Instant};

use arcade_testkit::{AttachOptions, Screened, TestClient, TestDaemon, WAIT};

/// The status bar's text with a space after it: the screen reading cuts trailing blanks, and a
/// tab is a name between two spaces.
fn bar_of(client: &TestClient) -> String {
    format!("{} ", client.status_row())
}

fn run(daemon: &TestDaemon, name: &str, args: &[&str]) {
    let (ok, output) = daemon.command(name, args).unwrap();
    assert!(ok, "{name} {args:?}: {output}");
}

/// A session called `work` with three windows named A, B and C, each showing a line of its own
/// (`here-A` and so on). The last one, C, is the current window.
fn three_windows(daemon: &TestDaemon) -> TestClient {
    let mut client = daemon
        .attach(AttachOptions::new().size(100, 30).named("work"))
        .unwrap();
    client.wait_for_prompt().unwrap();
    for (index, name) in ["A", "B", "C"].into_iter().enumerate() {
        if index > 0 {
            run(daemon, "window.new", &[]);
        }
        run(daemon, "window.rename", &[name]);
        client.type_line(&format!("echo here-{name}"));
        client.wait_for_row(&format!("here-{name}")).unwrap();
    }
    client
}

/// Which window the screen shows, by the line each one printed.
fn shown(client: &mut TestClient) -> String {
    let mut found = None;
    let ok = client.wait_until(Duration::from_millis(500), |c| {
        let rows = c.content_rows();
        found = ["A", "B", "C"]
            .into_iter()
            .find(|n| rows.iter().any(|r| r == &format!("here-{n}")))
            .map(str::to_string);
        found.is_some()
    });
    assert!(
        ok,
        "no window marker on the screen:\n{}",
        client.screen_text()
    );
    found.unwrap()
}

fn wait_shows(client: &mut TestClient, name: &str) {
    let ok = client.wait_until(WAIT, |c| {
        c.content_rows()
            .iter()
            .any(|r| r == &format!("here-{name}"))
    });
    assert!(ok, "window {name} is not shown:\n{}", client.screen_text());
}

/// The order of the tabs, as the names on the status bar, left to right.
fn tab_order(client: &TestClient) -> Vec<String> {
    let bar = bar_of(client);
    let mut found: Vec<(usize, String)> = Vec::new();
    for number in 1..=9 {
        for name in ["A", "B", "C"] {
            if let Some(at) = bar.find(&format!(" {number} {name} ")) {
                found.push((at, format!("{number} {name}")));
            }
        }
    }
    found.sort();
    found.into_iter().map(|(_, tab)| tab).collect()
}

fn wait_tabs(client: &mut TestClient, want: &[&str]) {
    let ok = client.wait_until(WAIT, |c| tab_order(c) == want);
    assert!(
        ok,
        "tabs {:?}, wanted {want:?}; the bar says {:?}",
        tab_order(client),
        bar_of(client)
    );
}

#[test]
fn three_windows_show_three_tabs_and_the_current_one_stands_out() {
    let daemon = TestDaemon::start();
    let mut client = three_windows(&daemon);
    wait_tabs(&mut client, &["1 A", "2 B", "3 C"]);
    let bar = bar_of(&client);
    assert!(bar.contains(" work "), "{bar:?}");
    let bar_row = 29;
    let style = |c: &TestClient, tab: &str| c.style_at(bar_row, bar.find(tab).unwrap() as u16 + 1);
    let current = style(&client, " 3 C ");
    assert_ne!(current, style(&client, " 1 A "));
    assert_eq!(style(&client, " 1 A "), style(&client, " 2 B "));
    // Choosing another window moves the highlight.
    run(&daemon, "window.select", &["1"]);
    client.wait_until(Duration::from_millis(300), |_| false);
    assert_eq!(style(&client, " 1 A "), current);
    assert_ne!(style(&client, " 3 C "), current);
}

#[test]
fn windows_can_be_chosen_by_number_stepped_through_and_gone_back_to() {
    let daemon = TestDaemon::start();
    let mut client = three_windows(&daemon);
    assert_eq!(shown(&mut client), "C");
    run(&daemon, "window.select", &["1"]);
    wait_shows(&mut client, "A");
    run(&daemon, "window.next", &[]);
    wait_shows(&mut client, "B");
    run(&daemon, "window.last", &[]);
    wait_shows(&mut client, "A");
    run(&daemon, "window.last", &[]);
    wait_shows(&mut client, "B");
    run(&daemon, "window.previous", &[]);
    wait_shows(&mut client, "A");
    // The ends join up.
    run(&daemon, "window.previous", &[]);
    wait_shows(&mut client, "C");
    run(&daemon, "window.next", &[]);
    wait_shows(&mut client, "A");
    // A window that is not there is refused.
    let (ok, output) = daemon.command("window.select", &["9"]).unwrap();
    assert!(!ok && output.contains("no window 9"), "{output}");
    let (ok, _) = daemon.command("window.select", &["0"]).unwrap();
    assert!(!ok);
    // The shell of the window shown takes the keys: they do not go to the others.
    client.type_line("echo typed-in-A");
    client.wait_for_row("typed-in-A").unwrap();
    run(&daemon, "window.select", &["2"]);
    wait_shows(&mut client, "B");
    assert!(!client.content_rows().iter().any(|r| r == "typed-in-A"));
}

#[test]
fn moving_a_window_swaps_its_tab_and_the_number_and_the_focus_go_with_it() {
    let daemon = TestDaemon::start();
    let mut client = three_windows(&daemon);
    wait_tabs(&mut client, &["1 A", "2 B", "3 C"]);
    run(&daemon, "window.move-left", &[]);
    wait_tabs(&mut client, &["1 A", "2 C", "3 B"]);
    // C is still the window shown, and its shell takes the keys.
    wait_shows(&mut client, "C");
    client.type_line("echo C-is-focused");
    client.wait_for_row("C-is-focused").unwrap();
    run(&daemon, "window.move-left", &[]);
    wait_tabs(&mut client, &["1 C", "2 A", "3 B"]);
    // At the left end nothing changes, and it is not an error.
    run(&daemon, "window.move-left", &[]);
    wait_tabs(&mut client, &["1 C", "2 A", "3 B"]);
    wait_shows(&mut client, "C");
    run(&daemon, "window.move-right", &[]);
    run(&daemon, "window.move-right", &[]);
    wait_tabs(&mut client, &["1 A", "2 B", "3 C"]);
    run(&daemon, "window.move-right", &[]);
    wait_tabs(&mut client, &["1 A", "2 B", "3 C"]);
    // `window.select 3` is the same window as before the moves.
    run(&daemon, "window.select", &["1"]);
    wait_shows(&mut client, "A");
    run(&daemon, "window.select", &["3"]);
    wait_shows(&mut client, "C");
}

/// User interface scenario 8: closing the window in the middle makes the numbers follow on.
#[test]
fn closing_the_window_in_the_middle_closes_the_gap_in_the_numbers() {
    let daemon = TestDaemon::start();
    let mut client = three_windows(&daemon);
    run(&daemon, "window.select", &["2"]);
    wait_shows(&mut client, "B");
    client.type_line("exit");
    wait_tabs(&mut client, &["1 A", "2 C"]);
    // A window is shown again: the one that was shown before B.
    let seen = shown(&mut client);
    assert!(seen == "A" || seen == "C", "{seen}");
    run(&daemon, "window.select", &["2"]);
    wait_shows(&mut client, "C");
    let (_, list) = daemon.command("session.list", &[]).unwrap();
    assert_eq!(list.split('\t').nth(3), Some("2"), "{list:?}");
}

#[test]
fn a_tab_is_named_after_the_directory_unless_it_was_named_by_hand() {
    let daemon = TestDaemon::start();
    let dir = daemon.dir().to_path_buf();
    std::fs::create_dir_all(dir.join("alpha")).unwrap();
    std::fs::create_dir_all(dir.join("beta")).unwrap();
    let mut client = daemon
        .attach(AttachOptions::new().size(100, 30).named("names"))
        .unwrap();
    client.wait_for_prompt().unwrap();
    let start = dir.file_name().unwrap().to_str().unwrap().to_string();
    assert!(client.wait_until(WAIT, |c| bar_of(c).contains(&format!(" 1 {start} "))));
    // The name is looked at when something is drawn anyway, so keep the pane printing.
    let follows = |client: &mut TestClient, command: &str, name: &str| {
        client.type_line(command);
        let deadline = Instant::now() + WAIT;
        loop {
            client.type_line("true");
            client.wait_until(Duration::from_millis(250), |c| {
                bar_of(c).contains(&format!(" 1 {name} "))
            });
            if bar_of(client).contains(&format!(" 1 {name} ")) {
                return;
            }
            assert!(
                Instant::now() < deadline,
                "the tab never became {name}: {:?}",
                bar_of(client)
            );
        }
    };
    follows(&mut client, "cd alpha", "alpha");
    follows(&mut client, "cd ../beta", "beta");
    // A name chosen by hand stays, whatever the pane does afterwards.
    run(&daemon, "window.rename", &["mine"]);
    assert!(client.wait_until(WAIT, |c| bar_of(c).contains(" 1 mine ")));
    client.type_line("cd ../alpha");
    let until = Instant::now() + Duration::from_millis(2500);
    while Instant::now() < until {
        client.type_line("true");
        client.wait_until(Duration::from_millis(250), |_| false);
    }
    let bar = bar_of(&client);
    assert!(
        bar.contains(" 1 mine ") && !bar.contains("alpha"),
        "{bar:?}"
    );
}

#[test]
fn a_zoomed_pane_is_marked_on_its_tab() {
    let daemon = TestDaemon::start();
    let mut client = three_windows(&daemon);
    run(&daemon, "pane.split-right", &[]);
    assert!(!bar_of(&client).contains("[Z]"));
    run(&daemon, "pane.zoom", &[]);
    assert!(client.wait_until(WAIT, |c| bar_of(c).contains(" 3 C [Z] ")));
    run(&daemon, "pane.zoom", &[]);
    assert!(client.wait_until(WAIT, |c| !bar_of(c).contains("[Z]")));
}

/// User interface scenario 14: with every pane still, nothing is sent to the client, however
/// many windows and tabs there are.
#[test]
fn nothing_reaches_the_client_while_every_pane_is_still() {
    let daemon = TestDaemon::start();
    let mut client = three_windows(&daemon);
    run(&daemon, "pane.split-down", &[]);
    client.wait_for_prompt().unwrap();
    // Let the last drawing settle, then count.
    client.wait_until(Duration::from_millis(1500), |_| false);
    let (messages, bytes) = (client.output_messages, client.output_bytes);
    let until = Instant::now() + Duration::from_secs(3);
    while Instant::now() < until {
        client.pump(until);
    }
    assert_eq!(
        (client.output_messages, client.output_bytes),
        (messages, bytes),
        "the status bar or a pane drew while everything was still"
    );
}
