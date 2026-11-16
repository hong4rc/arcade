//! A left click on a window's tab on the status bar shows that window; nothing else on the bar
//! does anything, and none of it reaches a pane.

use std::time::Duration;

use arcade_daemon::config::StatusPosition;
use arcade_testkit::{AttachOptions, Screened, TestClient, TestDaemon, WAIT};

fn run(daemon: &TestDaemon, name: &str, args: &[&str]) {
    let (ok, output) = daemon.command(name, args).unwrap();
    assert!(ok, "{name} {args:?}: {output}");
}

fn three_windows(daemon: &TestDaemon, cols: u16) -> TestClient {
    let mut client = daemon
        .attach(AttachOptions::new().size(cols, 30).named("work"))
        .unwrap();
    client.wait_for_prompt().unwrap();
    for (index, name) in ["A", "B", "C"].into_iter().enumerate() {
        if index > 0 {
            run(daemon, "window.new", &[]);
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

fn bar_row(client: &TestClient, top: bool) -> (u16, String) {
    let rows = client.screen_rows();
    if top {
        (0, rows[0].clone())
    } else {
        ((rows.len() - 1) as u16, rows[rows.len() - 1].clone())
    }
}

fn click(client: &mut TestClient, x: u16, y: u16) {
    client.send(format!("\x1b[<0;{};{}M\x1b[<0;{};{}m", x + 1, y + 1, x + 1, y + 1).as_bytes());
}

fn shows(client: &mut TestClient, window: &str) -> bool {
    let text = format!("here-{window}");
    client.wait_until(WAIT, |c| c.screen_text().contains(&text))
}

fn quiet(client: &mut TestClient) {
    client.wait_until(Duration::from_millis(400), |_| false);
}

/// The column where the text of the tab (`2 B`) begins on the bar.
fn column_of(bar: &str, needle: &str) -> u16 {
    let at = bar
        .find(needle)
        .unwrap_or_else(|| panic!("{needle:?} in {bar:?}"));
    bar[..at].chars().count() as u16
}

fn goes_to_the_tab_that_was_clicked(position: StatusPosition) {
    let daemon = TestDaemon::start_with(move |c| c.config.status_position = position);
    let top = position == StatusPosition::Top;
    let mut client = three_windows(&daemon, 100);
    let (y, bar) = bar_row(&client, top);
    // Window C is shown. Click the tab of A, then of B, then the tab that is already shown.
    click(&mut client, column_of(&bar, "1 A"), y);
    assert!(shows(&mut client, "A"), "{}", client.screen_text());
    let (y, bar) = bar_row(&client, top);
    click(&mut client, column_of(&bar, "2 B") + 1, y);
    assert!(shows(&mut client, "B"));
    let (y, bar) = bar_row(&client, top);
    click(&mut client, column_of(&bar, "2 B"), y);
    quiet(&mut client);
    assert!(client.screen_text().contains("here-B"));
}

#[test]
fn a_click_on_a_tab_shows_its_window_with_the_bar_at_the_bottom() {
    goes_to_the_tab_that_was_clicked(StatusPosition::Bottom);
}

#[test]
fn a_click_on_a_tab_shows_its_window_with_the_bar_at_the_top() {
    goes_to_the_tab_that_was_clicked(StatusPosition::Top);
}

#[test]
fn the_session_name_the_gaps_and_the_right_hand_words_do_nothing() {
    let daemon = TestDaemon::start();
    let mut client = three_windows(&daemon, 100);
    let (y, bar) = bar_row(&client, false);
    // The session's name, then the blank at the far right, then the gap before the tabs.
    click(&mut client, column_of(&bar, "work"), y);
    click(&mut client, 99, y);
    quiet(&mut client);
    assert!(client.screen_text().contains("here-C"));
    // A right-button press and the wheel on a tab do nothing either.
    let x = column_of(&bar, "1 A");
    client.send(format!("\x1b[<2;{};{}M\x1b[<2;{};{}m", x + 1, y + 1, x + 1, y + 1).as_bytes());
    client.send(format!("\x1b[<64;{};{}M", x + 1, y + 1).as_bytes());
    quiet(&mut client);
    assert!(client.screen_text().contains("here-C"));
}

#[test]
fn the_click_is_the_bars_even_when_the_program_asked_for_the_mouse() {
    let daemon = TestDaemon::start();
    let mut client = three_windows(&daemon, 100);
    // In window C a program turns on every kind of mouse report and shows what it gets.
    client.type_line("stty raw -echo; printf '\\033[?1003h\\033[?1006h'; echo re''ady; cat -vt");
    assert!(client.wait_until(WAIT, |c| {
        c.screen_rows().iter().any(|r| r.ends_with("ready"))
    }));
    let (y, bar) = bar_row(&client, false);
    let x = column_of(&bar, "1 A");
    // Press, a drag along the bar and the release.
    client.send(format!("\x1b[<0;{};{}M", x + 1, y + 1).as_bytes());
    assert!(shows(&mut client, "A"));
    client.send(format!("\x1b[<32;{};{}M\x1b[<0;{};{}m", x + 3, y + 1, x + 3, y + 1).as_bytes());
    // Back in C, the program has received nothing: no escape sequence is shown.
    run(&daemon, "window.select", &["3"]);
    quiet(&mut client);
    assert!(
        !client.screen_text().contains("^[["),
        "{}",
        client.screen_text()
    );
}

#[test]
fn a_narrow_bar_cuts_the_tabs_and_a_tab_that_is_cut_away_does_nothing() {
    let daemon = TestDaemon::start();
    let mut client = three_windows(&daemon, 16);
    let (y, bar) = bar_row(&client, false);
    // The session's name and a blank take 7 columns, the tab of A 5 more, and the tab of B is
    // cut at the edge; the tab of C is gone.
    assert!(
        bar.contains("1 A") && bar.contains("2 B") && !bar.contains("3 C"),
        "{bar:?}"
    );
    // The last column belongs to the part of B that shows.
    click(&mut client, 15, y);
    assert!(shows(&mut client, "B"), "{}", client.screen_text());
    // Nothing else on this bar leads to C, whatever column is clicked.
    for x in 0..7 {
        click(&mut client, x, y);
    }
    quiet(&mut client);
    assert!(client.screen_text().contains("here-B"));
    click(&mut client, column_of(&bar, "1 A"), y);
    assert!(shows(&mut client, "A"));
}
