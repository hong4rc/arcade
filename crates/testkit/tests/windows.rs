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
            // Typing before the new window's shell has printed its prompt would mix the two.
            let before = format!("here-{}", ["A", "B", "C"][index - 1]);
            assert!(client.wait_until(WAIT, |c| !c.screen_text().contains(&before)));
            client.wait_for_prompt().unwrap();
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
            if let Some(at) = bar.find(&format!(" {name}  {number} ")) {
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
    let style = |c: &TestClient, tab: &str| {
        c.style_at(
            bar_row,
            bar[..bar.find(tab).unwrap()].chars().count() as u16 + 1,
        )
    };
    let current = style(&client, " C  3 ");
    assert_ne!(current, style(&client, " A  1 "));
    assert_eq!(style(&client, " A  1 "), style(&client, " B  2 "));
    // Choosing another window moves the highlight.
    run(&daemon, "window.select", &["1"]);
    client.wait_until(Duration::from_millis(300), |_| false);
    assert_eq!(style(&client, " A  1 "), current);
    assert_ne!(style(&client, " C  3 "), current);
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

/// Wait until the bar says the first tab has this name; the name is looked at when something is
/// drawn anyway, so the pane is kept printing.
fn tab_becomes(client: &mut TestClient, name: &str) {
    let deadline = Instant::now() + WAIT;
    loop {
        client.type_line("");
        client.wait_until(Duration::from_millis(250), |c| {
            bar_of(c).contains(&format!(" {name}  1 "))
        });
        if bar_of(client).contains(&format!(" {name}  1 ")) {
            return;
        }
        assert!(
            Instant::now() < deadline,
            "the tab never became {name}: {:?}",
            bar_of(client)
        );
    }
}

/// Wait until the first tab is not called this any more.
fn tab_stops_being(daemon: &TestDaemon, client: &mut TestClient, name: &str) {
    let deadline = Instant::now() + WAIT;
    while bar_of(client).contains(&format!(" {name}  1 ")) {
        client.type_line("");
        client.wait_until(Duration::from_millis(250), |_| false);
        if Instant::now() >= deadline {
            let ps = std::process::Command::new("ps")
                .args(["-ax", "-o", "pid,ppid,pgid,tpgid,stat,command"])
                .output()
                .map(|o| String::from_utf8_lossy(&o.stdout).to_string())
                .unwrap_or_default();
            let lines: Vec<&str> = ps.lines().filter(|l| l.contains("sleep 3")).collect();
            // What the daemon itself thinks the windows are called, and whether the program is
            // still alive: with these a stale bar can be told from a stale name.
            let windows = daemon.command("window.list", &[]);
            panic!(
                "the tab is still {name}: {:?}\nthe daemon's windows: {windows:?}\nprocesses: {lines:#?}",
                bar_of(client)
            );
        }
    }
}

#[test]
fn a_tab_is_named_after_the_program_unless_it_was_named_by_hand() {
    let daemon = TestDaemon::start_with(|c| {
        c.config.window_name_from = arcade_daemon::config::NameFrom::Program;
        c.config.window_name_follow = true;
    });
    let mut client = daemon
        .attach(AttachOptions::new().size(100, 30).named("names"))
        .unwrap();
    client.wait_for_prompt().unwrap();
    // The shell first: its name is not the directory's.
    let start = daemon
        .dir()
        .file_name()
        .unwrap()
        .to_str()
        .unwrap()
        .to_string();
    assert!(
        !bar_of(&client).contains(&format!(" 1 {start} ")),
        "{:?}",
        bar_of(&client)
    );
    // A program in the foreground gives its name; when it ends the shell's comes back.
    client.type_line("sleep 31");
    tab_becomes(&mut client, "sleep");
    client.send(b"\x03");
    tab_stops_being(&daemon, &mut client, "sleep");
    // A name chosen by hand stays, whatever runs afterwards.
    run(&daemon, "window.rename", &["mine"]);
    assert!(client.wait_until(WAIT, |c| bar_of(c).contains(" mine  1 ")));
    client.type_line("sleep 32");
    let until = Instant::now() + Duration::from_millis(2500);
    while Instant::now() < until {
        client.type_line("");
        client.wait_until(Duration::from_millis(250), |_| false);
    }
    let bar = bar_of(&client);
    assert!(
        bar.contains(" mine  1 ") && !bar.contains("sleep"),
        "{bar:?}"
    );
    // An empty name gives the tab back to the program.
    run(&daemon, "window.rename", &[""]);
    tab_becomes(&mut client, "sleep");
    client.send(b"\x03");
    tab_stops_being(&daemon, &mut client, "sleep");
}

#[test]
fn a_zoomed_pane_is_marked_on_its_tab() {
    let daemon = TestDaemon::start();
    let mut client = three_windows(&daemon);
    run(&daemon, "pane.split-right", &[]);
    assert!(!bar_of(&client).contains("[Z]"));
    run(&daemon, "pane.zoom", &[]);
    assert!(client.wait_until(WAIT, |c| bar_of(c).contains(" C [Z]  3 ")));
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

/// The tab takes the name of a program that starts in its pane, a moment after the screen was
/// last drawn, though nothing more is printed afterwards.
#[test]
fn a_tab_takes_the_name_of_a_program_that_starts_with_nothing_printed_later() {
    let daemon = TestDaemon::start_with(|c| {
        c.config.window_name_from = arcade_daemon::config::NameFrom::Program;
        c.config.window_name_follow = true;
    });
    let mut client = daemon.attach_shell();
    client.wait_for_prompt().unwrap();
    client.type_line("sleep 33");
    let ok = client.wait_until(Duration::from_secs(2), |c| bar_of(c).contains(" sleep "));
    assert!(ok, "the bar says {:?}", bar_of(&client));
}

/// Through the real keys of the prompt: an empty answer to the window name gives the window back
/// to automatic naming, and Esc leaves the name as it is.
#[test]
fn an_empty_window_name_typed_at_the_prompt_gives_the_window_back_to_the_program() {
    const PREFIX: &[u8] = b"\x1b[109;5u";
    const CLEAR_LINE: &[u8] = b"\x1b[117;5u";
    let daemon = TestDaemon::start();
    let mut client = daemon.attach_shell();
    client.wait_for_prompt().unwrap();
    let ask = |client: &mut TestClient| {
        client.send(PREFIX);
        client.send(b",");
        assert!(client.wait_until(WAIT, |c| bar_of(c).contains("Window name:")));
        client.send(CLEAR_LINE);
    };
    ask(&mut client);
    client.send(b"edit");
    client.send(b"\r");
    assert!(client.wait_until(WAIT, |c| bar_of(c).contains(" edit  1 ")));
    // Esc leaves the name alone, even with the line cleared.
    ask(&mut client);
    client.send(b"\x1b[27u");
    client.wait_until(Duration::from_millis(1500), |_| false);
    assert!(
        bar_of(&client).contains(" edit  1 "),
        "{:?}",
        bar_of(&client)
    );
    // Enter on an empty line is the answer "no name": the program's name comes back.
    ask(&mut client);
    client.send(b"\r");
    tab_stops_being(&daemon, &mut client, "edit");
}
