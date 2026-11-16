//! What a tab says: how a window gets its name (the directory by default), where its number
//! stands and in what colour, and the format of its text.

use std::time::Duration;

use arcade_daemon::config::{NameFrom, NumberPosition, TAB_FIELDS};
use arcade_daemon::format::Template;
use arcade_testkit::{AttachOptions, Screened, TestClient, TestDaemon, WAIT};

fn bar(client: &TestClient) -> String {
    format!("{} ", client.status_row())
}

fn run(daemon: &TestDaemon, name: &str, args: &[&str]) {
    let (ok, output) = daemon.command(name, args).unwrap();
    assert!(ok, "{name} {args:?}: {output}");
}

fn attach(daemon: &TestDaemon) -> TestClient {
    let mut client = daemon
        .attach(AttachOptions::new().size(100, 30).named("tabs"))
        .unwrap();
    client.wait_for_prompt().unwrap();
    client
}

fn start_dir(daemon: &TestDaemon) -> String {
    daemon
        .dir()
        .file_name()
        .unwrap()
        .to_string_lossy()
        .into_owned()
}

fn template(source: &str) -> Template {
    Template::parse(source, TAB_FIELDS).unwrap()
}

fn column_of(bar: &str, needle: &str) -> u16 {
    let at = bar
        .find(needle)
        .unwrap_or_else(|| panic!("{needle:?} in {bar:?}"));
    bar[..at].chars().count() as u16
}

fn status_y(client: &TestClient) -> u16 {
    (client.screen_rows().len() - 1) as u16
}

#[test]
fn a_window_is_named_after_its_directory_and_the_name_stays_when_the_shell_moves() {
    let daemon = TestDaemon::start();
    let mut client = attach(&daemon);
    let dir = start_dir(&daemon);
    assert!(
        bar(&client).contains(&format!(" 1 {dir} ")),
        "{:?}",
        bar(&client)
    );
    client.type_line("cd /; echo moved-$((40 + 2))");
    client.wait_for_text("moved-42").unwrap();
    client.wait_until(Duration::from_millis(1500), |_| false);
    client.type_line("");
    assert!(
        bar(&client).contains(&format!(" 1 {dir} ")),
        "{:?}",
        bar(&client)
    );
}

#[test]
fn the_name_follows_the_directory_when_the_configuration_says_so() {
    let daemon = TestDaemon::start_with(|c| c.config.window_name_follow = true);
    let mut client = attach(&daemon);
    client.type_line("cd /; echo moved-$((40 + 2))");
    client.wait_for_text("moved-42").unwrap();
    let ok = (0..30).any(|_| {
        client.type_line("");
        client.wait_until(Duration::from_millis(250), |c| bar(c).contains(" 1 / "))
    });
    assert!(ok, "{:?}", bar(&client));
}

#[test]
fn the_home_directory_is_called_a_tilde() {
    let daemon = TestDaemon::start_with(|c| c.config.window_name_follow = true);
    let mut client = attach(&daemon);
    // HOME is the daemon's own environment, so the tilde is checked against what the daemon has.
    let real = std::env::var("HOME").unwrap_or_default();
    if real.is_empty() {
        return;
    }
    client.type_line(&format!("cd '{real}'; echo moved-$((40 + 2))"));
    client.wait_for_text("moved-42").unwrap();
    let ok = (0..30).any(|_| {
        client.type_line("");
        client.wait_until(Duration::from_millis(250), |c| bar(c).contains(" 1 ~ "))
    });
    assert!(ok, "{:?}", bar(&client));
}

#[test]
fn the_name_can_come_from_the_program_or_from_the_title_the_program_set() {
    let daemon = TestDaemon::start_with(|c| {
        c.config.window_name_from = NameFrom::Program;
        c.config.window_name_follow = true;
    });
    let mut client = attach(&daemon);
    client.type_line("sleep 37");
    assert!(
        client.wait_until(WAIT, |c| bar(c).contains(" 1 sleep ")),
        "{:?}",
        bar(&client)
    );
    client.send(b"\x03");

    let daemon = TestDaemon::start_with(|c| {
        c.config.window_name_from = NameFrom::Title;
        c.config.window_name_follow = true;
    });
    let mut client = attach(&daemon);
    let dir = start_dir(&daemon);
    // No title yet: the directory.
    assert!(
        bar(&client).contains(&format!(" 1 {dir} ")),
        "{:?}",
        bar(&client)
    );
    client.type_line("printf '\\033]2;from-title\\007'");
    let ok = (0..30).any(|_| {
        client.type_line("");
        client.wait_until(Duration::from_millis(250), |c| {
            bar(c).contains(" 1 from-title ")
        })
    });
    assert!(ok, "{:?}", bar(&client));
}

#[test]
fn a_name_given_by_hand_wins_and_an_empty_one_gives_the_window_back_to_the_rule() {
    let daemon = TestDaemon::start();
    let mut client = attach(&daemon);
    let dir = start_dir(&daemon);
    run(&daemon, "window.rename", &["mine"]);
    assert!(client.wait_until(WAIT, |c| bar(c).contains(" 1 mine ")));
    client.type_line("cd /; echo moved-$((40 + 2))");
    client.wait_for_text("moved-42").unwrap();
    assert!(bar(&client).contains(" 1 mine "));
    // Given back: named by the directory the pane is in now, at once.
    run(&daemon, "window.rename", &[""]);
    assert!(
        client.wait_until(WAIT, |c| bar(c).contains(" 1 / ")),
        "{:?}",
        bar(&client)
    );
    assert!(!bar(&client).contains(&dir));
}

#[test]
fn the_number_stands_left_or_right_of_the_name_or_is_not_shown_with_a_colour_of_its_own() {
    for (position, expected) in [
        (NumberPosition::Left, "1 nm "),
        (NumberPosition::Right, "nm 1 "),
        (NumberPosition::Off, " nm "),
    ] {
        let daemon = TestDaemon::start_with(move |c| c.config.tab_number = position);
        let mut client = attach(&daemon);
        run(&daemon, "window.rename", &["nm"]);
        assert!(
            client.wait_until(WAIT, |c| bar(c).contains(expected)),
            "{position:?} {:?}",
            bar(&client)
        );
        let line = bar(&client);
        let y = status_y(&client);
        let name = client.screen.style_at(y, column_of(&line, "nm"));
        if position == NumberPosition::Off {
            assert!(!line.contains(" 1 "), "{line:?}");
            continue;
        }
        // The number is drawn in another colour than the name, and in the colours of the tab
        // swapped (the tab shown is the current one).
        let at = match position {
            NumberPosition::Left => column_of(&line, "1 nm"),
            _ => column_of(&line, "nm 1") + 3,
        };
        let number = client.screen.style_at(y, at);
        assert_ne!(number.fg, name.fg, "{position:?}");
        assert_eq!(number.bg, name.fg, "{position:?}");
        assert_eq!(number.fg, name.bg, "{position:?}");
    }
}

#[test]
fn the_text_of_a_tab_is_the_name_by_default_and_follows_the_format() {
    let daemon = TestDaemon::start();
    let mut client = attach(&daemon);
    run(&daemon, "window.rename", &["nm"]);
    client.type_line("printf '\\033]2;prog-title\\007'");
    // By default no title of the program shows on a tab.
    client.wait_until(Duration::from_millis(500), |_| false);
    assert!(bar(&client).contains(" 1 nm "), "{:?}", bar(&client));
    assert!(!bar(&client).contains("prog-title"));

    let daemon = TestDaemon::start_with(|c| c.config.tab_format = template("{name} {title}"));
    let mut client = attach(&daemon);
    run(&daemon, "window.rename", &["nm"]);
    assert!(client.wait_until(WAIT, |c| bar(c).contains(" 1 nm ")));
    client.type_line("printf '\\033]2;prog-title\\007'");
    assert!(
        client.wait_until(WAIT, |c| bar(c).contains(" 1 nm prog-title ")),
        "{:?}",
        bar(&client)
    );
    // The title is quieter than the name.
    let y = status_y(&client);
    let line = bar(&client);
    let name = client.screen.style_at(y, column_of(&line, "nm"));
    let title = client.screen.style_at(y, column_of(&line, "prog-title"));
    assert!(title.faint && !name.faint);
    // A title equal to the name is said once, and a long one is cut.
    client.type_line("printf '\\033]2;nm\\007'");
    assert!(client.wait_until(WAIT, |c| bar(c).contains(" 1 nm ")
        && !bar(c).contains("nm nm")));
    client.type_line("printf '\\033]2;%s\\007' abcdefghijklmnopqrstuvwxyz0123456789");
    assert!(
        client.wait_until(WAIT, |c| bar(c).contains("abcdefghijklmnopqrstuvw\u{2026}")),
        "{:?}",
        bar(&client)
    );
}

#[test]
fn a_tab_format_may_use_the_program_and_the_directory_and_reorder_them() {
    let daemon =
        TestDaemon::start_with(|c| c.config.tab_format = template("{directory}|{program}"));
    let mut client = attach(&daemon);
    let dir = start_dir(&daemon);
    client.type_line("sleep 38");
    assert!(
        client.wait_until(WAIT, |c| bar(c).contains(&format!(" 1 {dir}|sleep "))),
        "{:?}",
        bar(&client)
    );
    client.send(b"\x03");
}

#[test]
fn the_number_and_the_formats_are_read_from_the_file_and_taken_up_on_reload() {
    let dir = std::env::temp_dir().join(format!("arc-tabcfg-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let file = dir.join("config.toml");
    std::fs::write(
        &file,
        "[status]\nnumber = \"right\"\ntab-format = \"{name}!\"\n[window]\nname-from = \"program\"\n",
    )
    .unwrap();
    let path = file.clone();
    let daemon = TestDaemon::start_with(move |c| c.config_path = Some(path));
    let mut client = attach(&daemon);
    run(&daemon, "window.rename", &["nm"]);
    assert!(
        client.wait_until(WAIT, |c| bar(c).contains(" nm! 1 ")),
        "{:?}",
        bar(&client)
    );
    std::fs::write(&file, "[status]\nnumber = \"off\"\n").unwrap();
    run(&daemon, "arcade.reload-config", &[]);
    assert!(
        client.wait_until(WAIT, |c| bar(c).contains(" nm ") && !bar(c).contains("nm!")),
        "{:?}",
        bar(&client)
    );
    // A format with a field that does not exist is a mistake, reported, and nothing changes.
    std::fs::write(&file, "[status]\ntab-format = \"{nope}\"\n").unwrap();
    let (ok, message) = daemon.command("arcade.reload-config", &[]).unwrap();
    assert!(!ok);
    assert!(
        message.contains("tab-format") && message.contains("{nope}"),
        "{message}"
    );
    let _ = std::fs::remove_dir_all(dir);
}

/// A program that changes its title ten times a second (a spinner) costs the status bar a
/// redraw each time, a few dozen bytes, and nothing of the panes.
#[test]
fn a_title_that_changes_ten_times_a_second_costs_only_a_few_bytes_of_the_bar() {
    let daemon = TestDaemon::start_with(|c| c.config.tab_format = template("{name} {title}"));
    let mut client = attach(&daemon);
    run(&daemon, "window.rename", &["nm"]);
    client.type_line(
        "i=0; while [ $i -lt 50 ]; do printf '\\033]2;spin-%s\\007' $((i%10)); sleep 0.1; i=$((i+1)); done; echo spun-$((40 + 2))",
    );
    // Wait until the loop has begun, then count what the client gets during the rest of it.
    assert!(client.wait_until(WAIT, |c| bar(c).contains(" nm spin-")));
    let (messages, bytes) = (client.output_messages, client.output_bytes);
    assert!(client.wait_until(Duration::from_secs(20), |c| {
        c.screen_text().contains("spun-42")
    }));
    let (messages, bytes) = (
        client.output_messages - messages,
        client.output_bytes - bytes,
    );
    eprintln!("spinner: {messages} messages, {bytes} bytes for about 45 title changes");
    // The prompt line and its echo account for some; each title change is a redraw of the bar.
    assert!(bytes < 45 * 400 + 2000, "{bytes} bytes");
}

#[test]
fn every_way_to_make_a_window_names_it_after_a_directory_by_default() {
    let daemon = TestDaemon::start();
    let dir = start_dir(&daemon);
    // A session started with a program that is not a shell: still named by its directory.
    let mut client = daemon
        .attach(AttachOptions::new().size(100, 30).named("tabs").command(&[
            "/bin/sh",
            "-c",
            "mkdir -p inner-dir; cd inner-dir; exec /bin/sh",
        ]))
        .unwrap();
    client.wait_for_text("$").unwrap();
    assert!(
        bar(&client).contains(&format!(" 1 {dir} ")),
        "{:?}",
        bar(&client)
    );
    // A new window takes the directory of the pane it was made from, not the program.
    run(&daemon, "window.new", &[]);
    assert!(
        client.wait_until(WAIT, |c| bar(c).contains(" 2 inner-dir ")),
        "{:?}",
        bar(&client)
    );
    // A pane split off and then broken out into its own window is named the same way.
    run(&daemon, "pane.split-right", &[]);
    client.wait_until(WAIT, |c| c.screen_text().contains('│'));
    run(&daemon, "pane.break", &[]);
    assert!(
        client.wait_until(WAIT, |c| bar(c).contains(" 3 inner-dir ")),
        "{:?}",
        bar(&client)
    );
}
