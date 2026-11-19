//! The title of the host terminal's window: what the focused pane's program calls itself, then
//! the session and the window number; sent when it changes and only then.

use std::time::Duration;

use arcade_daemon::config::TITLE_FIELDS;
use arcade_daemon::format::Template;
use arcade_testkit::{AttachOptions, Screened, TestClient, TestDaemon, WAIT, wait_for_file};

/// A daemon whose title format puts the program's title first and the session after it, which
/// the cases here were written for.
fn daemon() -> TestDaemon {
    TestDaemon::start_with(|c| c.config.title_format = old_order())
}

fn old_order() -> Template {
    Template::parse("{title} \u{b7} {session}:{number}", TITLE_FIELDS).unwrap()
}

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
fn attaching_gives_the_session_and_the_window_number_and_the_name_of_the_window() {
    // The default format: the Arcade part first, the program's own title after it.
    let daemon = TestDaemon::start();
    let client = attach(&daemon);
    let title = last_title(&client);
    assert!(title.starts_with("thu:1 "), "{title:?}");
    // The pane has set no title, so there is nothing after the name and no dangling separator.
    assert!(!title.contains('\u{b7}'), "{title:?}");
    assert!(title.len() > "thu:1 ".len(), "{title:?}");
}

#[test]
fn the_default_title_puts_arcade_first_and_the_title_of_the_program_after() {
    let daemon = TestDaemon::start();
    let mut client = attach(&daemon);
    client.type_line("printf '\\033]2;real-title\\007'");
    assert!(client.wait_until(WAIT, |c| last_title(c).ends_with(" \u{b7} real-title")));
    assert!(
        last_title(&client).starts_with("thu:1 "),
        "{:?}",
        client.titles
    );
}

#[test]
fn a_title_format_without_the_title_leaves_the_programs_title_out() {
    let daemon = TestDaemon::start_with(|c| {
        c.config.title_format = Template::parse("{session}:{number}", TITLE_FIELDS).unwrap();
    });
    let mut client = attach(&daemon);
    assert_eq!(last_title(&client), "thu:1");
    client.type_line("printf '\\033]2;secret\\007'; echo done-$((40 + 2))");
    client.wait_for_text("done-42").unwrap();
    client.wait_until(Duration::from_millis(400), |_| false);
    assert!(
        client.titles.iter().all(|t| !t.contains("secret")),
        "{:?}",
        client.titles
    );
}

#[test]
fn a_reordered_title_format_is_followed_and_an_empty_one_sets_no_title() {
    let daemon = TestDaemon::start_with(|c| {
        c.config.title_format =
            Template::parse("{number}/{session}|{title}", TITLE_FIELDS).unwrap();
    });
    let mut client = attach(&daemon);
    client.type_line("printf '\\033]2;abc\\007'");
    assert!(
        client.wait_until(WAIT, |c| last_title(c) == "1/thu|abc"),
        "{:?}",
        client.titles
    );

    let silent = TestDaemon::start_with(|c| {
        c.config.title_format = Template::parse("", TITLE_FIELDS).unwrap();
    });
    let mut quiet = attach(&silent);
    quiet.type_line("printf '\\033]2;abc\\007'; echo done-$((40 + 2))");
    quiet.wait_for_text("done-42").unwrap();
    quiet.wait_until(Duration::from_millis(400), |_| false);
    assert!(quiet.titles.is_empty(), "{:?}", quiet.titles);
}

#[test]
fn the_title_a_pane_sets_goes_to_the_terminal_and_a_second_window_changes_the_number() {
    let daemon = daemon();
    let mut client = attach(&daemon);
    client.type_line("printf '\\033]2;abc\\007'");
    wait_title(&mut client, "abc");
    assert_eq!(last_title(&client), "abc \u{b7} thu:1");
    run(&daemon, "window.new", &[]);
    assert!(client.wait_until(WAIT, |c| last_title(c).ends_with(":2")));
}

#[test]
fn moving_the_focus_to_another_pane_brings_that_pane_s_title() {
    let daemon = daemon();
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
    let daemon = daemon();
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
    let daemon = daemon();
    let mut client = attach(&daemon);
    client.type_line("printf '\\033]2;a\\001b\\037c\\007'");
    wait_title(&mut client, "abc");
    assert_eq!(last_title(&client), "abc \u{b7} thu:1");
}

/// A title is the program's own words: a long one is cut, and the host's window title never grows
/// past the limit however often it is set.
#[test]
fn a_long_title_is_cut_to_the_limit() {
    let daemon = daemon();
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
    let daemon = daemon();
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

/// What is typed to push the pane's title (`CSI 22 ; 2 t`), set one, and pop (`CSI 23 ; 2 t`).
const PUSH: &str = "\\033[22;2t";
const POP: &str = "\\033[23;2t";

fn set(title: &str) -> String {
    format!("\\033]2;{title}\\007")
}

/// Send the bytes from the shell's own printf and wait for the shell to be done with them.
fn emit(client: &mut TestClient, bytes: &str, done: &str) {
    client.type_line(&format!("printf '{bytes}'; echo {done}-$((40 + 2))"));
    client.wait_for_text(&format!("{done}-42")).unwrap();
}

#[test]
fn a_program_can_keep_a_title_and_get_it_back_nested() {
    let daemon = daemon();
    let mut client = attach(&daemon);
    let line = format!("{}{PUSH}{}{PUSH}{}", set("one"), set("two"), set("three"));
    emit(&mut client, &line, "nest");
    wait_title(&mut client, "three");
    emit(&mut client, POP, "popa");
    wait_title(&mut client, "two");
    emit(&mut client, POP, "popb");
    wait_title(&mut client, "one");
    // Nothing more is kept: another pop changes nothing.
    emit(&mut client, POP, "popc");
    client.wait_until(Duration::from_millis(500), |_| false);
    assert_eq!(last_title(&client), "one \u{b7} thu:1");
}

#[test]
fn a_pop_can_bring_back_having_no_title() {
    let daemon = daemon();
    let mut client = attach(&daemon);
    emit(&mut client, &format!("{PUSH}{}", set("temporary")), "tmp");
    wait_title(&mut client, "temporary");
    emit(&mut client, POP, "back");
    assert!(client.wait_until(WAIT, |c| !last_title(c).starts_with("temporary")));
    // The window's name shows again, so the title still has its session and number.
    assert!(last_title(&client).ends_with(":1"), "{:?}", client.titles);
}

#[test]
fn only_the_title_form_of_push_and_pop_counts() {
    let daemon = daemon();
    let mut client = attach(&daemon);
    // `;1` is the icon: nothing is kept and nothing comes back.
    let line = format!("{}\\033[22;1t{}\\033[23;1t", set("keep"), set("other"));
    emit(&mut client, &line, "icon");
    wait_title(&mut client, "other");
}

#[test]
fn at_most_ten_titles_are_kept() {
    let daemon = daemon();
    let mut client = attach(&daemon);
    client.type_line(&format!(
        "i=1; while [ $i -le 12 ]; do printf '{}{PUSH}' $i; i=$((i+1)); done; \
         printf '{}'; i=0; while [ $i -lt 11 ]; do printf '{POP}'; i=$((i+1)); done; echo deep-$((40 + 2))",
        set("t%s"),
        set("top"),
    ));
    client.wait_for_text("deep-42").unwrap();
    // Twelve were kept, ten stayed: eleven pops end at the third, not the second.
    wait_title(&mut client, "t3");
    assert_eq!(last_title(&client), "t3 \u{b7} thu:1");
}

#[test]
fn other_title_cases_hold() {
    let daemon = daemon();
    let mut client = attach(&daemon);
    // OSC 1 (the icon name) is ignored; OSC 0 sets; a string terminator ends it as BEL does.
    emit(&mut client, "\\033]1;icon\\007", "icon");
    assert!(!client.titles.iter().any(|t| t.starts_with("icon")));
    emit(&mut client, "\\033]0;zero\\033\\\\", "zero");
    wait_title(&mut client, "zero");
    // An empty title gives the window's name back.
    emit(&mut client, "\\033]2;\\007", "gone");
    assert!(client.wait_until(WAIT, |c| !last_title(c).starts_with("zero")));
}

/// A program's title lasts as long as the program does; the shell's own stays.
#[test]
fn a_title_goes_when_the_program_that_set_it_ends_and_the_shells_stays() {
    let daemon = daemon();
    let mut client = attach(&daemon);
    client.type_line(&format!(
        "sh -c \"printf '{}'; sleep 2\"; echo ended-$((40 + 2))",
        set("prog")
    ));
    wait_title(&mut client, "prog");
    client.wait_for_text("ended-42").unwrap();
    let deadline = std::time::Instant::now() + WAIT;
    while last_title(&client).starts_with("prog") {
        client.type_line("");
        client.wait_until(Duration::from_millis(250), |_| false);
        assert!(
            std::time::Instant::now() < deadline,
            "the title stayed: {:?}",
            client.titles
        );
    }
    // A title the shell itself set stays while other programs run and end.
    emit(&mut client, &set("shells"), "mine");
    wait_title(&mut client, "shells");
    client.type_line("sleep 1; echo slept-$((40 + 2))");
    client.wait_for_text("slept-42").unwrap();
    let until = std::time::Instant::now() + Duration::from_millis(2500);
    while std::time::Instant::now() < until {
        client.type_line("");
        client.wait_until(Duration::from_millis(250), |_| false);
    }
    assert_eq!(last_title(&client), "shells \u{b7} thu:1");
}

#[test]
fn a_title_that_is_the_name_of_the_window_is_said_once() {
    let daemon = TestDaemon::start();
    let mut client = attach(&daemon);
    run(&daemon, "window.rename", &["same"]);
    wait_title(&mut client, "thu:1 same");
    client.type_line("printf '\\033]2;same\\007'; echo done-$((40 + 2))");
    client.wait_for_text("done-42").unwrap();
    client.wait_until(Duration::from_millis(400), |_| false);
    assert_eq!(last_title(&client), "thu:1 same");
    client.type_line("printf '\\033]2;other\\007'");
    wait_title(&mut client, "thu:1 same \u{b7} other");
}

#[test]
fn the_title_is_sent_again_to_a_client_that_attaches() {
    let daemon = daemon();
    let mut first = attach(&daemon);
    emit(&mut first, &set("again"), "set");
    wait_title(&mut first, "again");
    let mut second = daemon
        .attach(AttachOptions::new().size(100, 30).existing(Some("thu")))
        .unwrap();
    wait_title(&mut second, "again");
}

#[test]
fn two_sessions_started_from_one_folder_get_different_names() {
    let daemon = daemon();
    let mut a = daemon.attach(AttachOptions::new().size(100, 30)).unwrap();
    a.wait_for_prompt().unwrap();
    let mut b = daemon.attach(AttachOptions::new().size(100, 30)).unwrap();
    b.wait_for_prompt().unwrap();
    let (_, list) = daemon.command("session.list", &[]).unwrap();
    let names: Vec<&str> = list.lines().filter_map(|l| l.split('\t').next()).collect();
    assert_eq!(names.len(), 2, "{list:?}");
    assert_ne!(names[0], names[1], "{list:?}");
}
