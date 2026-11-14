//! The mouse, through the daemon: the client sends the reports a host terminal sends in the SGR
//! form (`CSI < code ; column ; row M`, `m` for a release), and the tests look at the screen, at
//! what the programs in the panes receive, and at what the client is told to turn on and off.
//!
//! Places on the screen are counted from 0 here and sent from 1, as the host terminal does.

use std::time::Duration;

use arcade_term::Mode;
use arcade_testkit::{AttachOptions, Screened, TestClient, TestDaemon, WAIT};

const PREFIX: &[u8] = b"\x1b[109;5u";

fn client(daemon: &TestDaemon) -> TestClient {
    named(daemon, "mouse")
}

fn named(daemon: &TestDaemon, name: &str) -> TestClient {
    let mut client = daemon
        .attach(AttachOptions::new().size(100, 30).named(name))
        .unwrap();
    client.wait_for_prompt().unwrap();
    client
}

fn report(client: &mut TestClient, code: u16, x: u16, y: u16, up: bool) {
    let end = if up { 'm' } else { 'M' };
    client.send(format!("\x1b[<{code};{};{}{end}", x + 1, y + 1).as_bytes());
}

fn press(client: &mut TestClient, x: u16, y: u16) {
    report(client, 0, x, y, false);
}

fn release(client: &mut TestClient, x: u16, y: u16) {
    report(client, 0, x, y, true);
}

fn click(client: &mut TestClient, x: u16, y: u16) {
    press(client, x, y);
    release(client, x, y);
}

fn drag(client: &mut TestClient, from: (u16, u16), to: (u16, u16)) {
    press(client, from.0, from.1);
    // A few moves on the way, as a real mouse makes.
    report(client, 32, (from.0 + to.0) / 2, (from.1 + to.1) / 2, false);
    report(client, 32, to.0, to.1, false);
    release(client, to.0, to.1);
}

fn wheel_up(client: &mut TestClient, x: u16, y: u16) {
    report(client, 64, x, y, false);
}

fn wheel_down(client: &mut TestClient, x: u16, y: u16) {
    report(client, 65, x, y, false);
}

fn bar(client: &TestClient) -> String {
    format!("{} ", client.status_row())
}

fn in_copy_mode(client: &mut TestClient) -> bool {
    client.wait_until(WAIT, |c| bar(c).contains(" COPY "))
}

fn left_copy_mode(client: &mut TestClient) -> bool {
    client.wait_until(WAIT, |c| !bar(c).contains(" COPY "))
}

/// Split the pane in two, side by side; the new pane on the right has the focus. Returns the
/// column of the divider.
fn split(client: &mut TestClient) -> u16 {
    client.send(PREFIX);
    client.send(b"|");
    assert!(client.wait_until(WAIT, |c| c.screen_text().contains('│')));
    let row = client
        .screen_rows()
        .into_iter()
        .find(|r| r.contains('│'))
        .unwrap();
    row.chars().position(|c| c == '│').unwrap() as u16
}

/// A pane whose program turned on the mouse reports in `modes` and shows each byte it gets, as
/// `cat -vt` writes it.
///
/// The program says `ready` when it is set up. The shell redraws the line being typed bit by bit,
/// so a half-drawn line that ends in `echo ready` must not be taken for the word the program
/// prints: the typed line spells it `re''ady`, which the shell joins when it runs it.
fn mouse_program(client: &mut TestClient, modes: &str) {
    client.type_line(&format!(
        "stty raw -echo; printf '\\033[?{modes}'; echo re''ady; cat -vt"
    ));
    let ok = client.wait_until(WAIT, |c| {
        c.screen_rows().iter().any(|r| r.ends_with("ready"))
    });
    assert!(ok, "{}", client.screen_text());
}

fn wait_screen_has(client: &mut TestClient, text: &str) {
    let ok = client.wait_until(WAIT, |c| c.screen_text().contains(text));
    assert!(
        ok,
        "expected {text:?} on the screen:\n{}",
        client.screen_text()
    );
}

fn quiet(client: &mut TestClient) {
    std::thread::sleep(Duration::from_millis(150));
    client.read_more(std::time::Instant::now() + Duration::from_millis(50));
}

#[test]
fn the_host_terminal_is_asked_for_the_mouse_and_gives_it_back_whenever_the_client_leaves() {
    let daemon = TestDaemon::start();
    let mut first = client(&daemon);
    assert_eq!(first.screen.mode(Mode::MouseSgr), Some(true));
    assert_eq!(first.screen.mode(Mode::MouseTracking), Some(true));
    first.detach();
    assert!(first.wait_until(WAIT, |c| c.screen.mode(Mode::MouseTracking) == Some(false)));
    assert_eq!(first.screen.mode(Mode::MouseSgr), Some(false));

    // Another client takes the session over: the first is told to leave, and gives the mouse back.
    let mut one = named(&daemon, "shared");
    let mut two = daemon
        .attach(AttachOptions::new().existing(Some("shared")))
        .unwrap();
    two.wait_for_prompt().unwrap();
    assert!(one.wait_until(WAIT, |c| c.screen.mode(Mode::MouseTracking) == Some(false)));
    assert_eq!(two.screen.mode(Mode::MouseTracking), Some(true));

    // The session ends: the client in it gives the mouse back.
    two.type_line("exit");
    assert!(two.wait_until(WAIT, |c| c.screen.mode(Mode::MouseTracking) == Some(false)));
}

#[test]
fn a_click_moves_the_focus_to_the_pane_and_a_click_elsewhere_changes_nothing() {
    let daemon = TestDaemon::start();
    let mut client = client(&daemon);
    let divider = split(&mut client);
    let right = client.focused_pane_id();
    click(&mut client, 5, 3);
    let left = client.focused_pane_id();
    assert_ne!(left, right);
    click(&mut client, divider + 5, 3);
    assert_eq!(client.focused_pane_id(), right);
    // On the divider, on the status bar: nothing happens.
    click(&mut client, divider, 3);
    assert_eq!(client.focused_pane_id(), right);
    click(&mut client, 5, 29);
    assert_eq!(client.focused_pane_id(), right);
    // Clicking the pane that has the focus changes nothing either, and does not open copy mode.
    click(&mut client, divider + 8, 5);
    assert_eq!(client.focused_pane_id(), right);
    assert!(!bar(&client).contains(" COPY "));
}

#[test]
fn a_program_that_asked_for_the_mouse_gets_places_counted_in_its_own_pane() {
    let daemon = TestDaemon::start();
    let mut client = client(&daemon);
    let divider = split(&mut client);
    mouse_program(&mut client, "1000h\\033[?1006h");
    // Column 10 and row 7 of the pane, which starts one column after the divider.
    press(&mut client, divider + 1 + 9, 6);
    release(&mut client, divider + 1 + 9, 6);
    wait_screen_has(&mut client, "^[[<0;10;7M^[[<0;10;7m");
}

#[test]
fn the_old_form_is_used_for_a_program_that_did_not_ask_for_the_sgr_form() {
    let daemon = TestDaemon::start();
    let mut client = client(&daemon);
    mouse_program(&mut client, "1000h");
    // Button 0 is a space (32), column 5 is 37 (%), row 4 is 36 ($).
    press(&mut client, 4, 3);
    wait_screen_has(&mut client, "^[[M %$");
}

#[test]
fn a_program_gets_the_moves_only_when_its_level_asks_for_them() {
    let daemon = TestDaemon::start();
    let mut client = client(&daemon);
    mouse_program(&mut client, "1000h\\033[?1006h");
    press(&mut client, 2, 2);
    report(&mut client, 32, 3, 2, false);
    release(&mut client, 3, 2);
    wait_screen_has(&mut client, "^[[<0;4;3m");
    assert!(
        !client.screen_text().contains("<32;"),
        "{}",
        client.screen_text()
    );

    let mut client2 = {
        let mut c = daemon
            .attach(AttachOptions::new().size(100, 30).named("level"))
            .unwrap();
        c.wait_for_prompt().unwrap();
        c
    };
    mouse_program(&mut client2, "1002h\\033[?1006h");
    press(&mut client2, 2, 2);
    report(&mut client2, 32, 3, 2, false);
    release(&mut client2, 3, 2);
    wait_screen_has(&mut client2, "^[[<32;4;3M");
}

#[test]
fn a_program_that_did_not_ask_gets_nothing_and_a_wheel_it_asked_for_is_passed_on() {
    let daemon = TestDaemon::start();
    let mut client = client(&daemon);
    client.type_line("stty raw -echo; echo ready; cat -vt");
    client.wait_for_row("ready").unwrap();
    click(&mut client, 3, 3);
    wheel_up(&mut client, 3, 3);
    quiet(&mut client);
    assert!(
        !client.screen_text().contains("^[["),
        "{}",
        client.screen_text()
    );

    // A program that asked for the mouse gets the wheel and Arcade stays out of copy mode.
    client.send(b"\x03");
    let mut asked = daemon
        .attach(AttachOptions::new().size(100, 30).named("wheel"))
        .unwrap();
    asked.wait_for_prompt().unwrap();
    mouse_program(&mut asked, "1000h\\033[?1006h");
    wheel_up(&mut asked, 10, 4);
    wheel_down(&mut asked, 10, 4);
    wait_screen_has(&mut asked, "^[[<64;11;5M^[[<65;11;5M");
    assert!(!bar(&asked).contains(" COPY "));
}

#[test]
fn the_wheel_scrolls_a_pane_up_through_copy_mode_and_back_down_out_of_it() {
    let daemon = TestDaemon::start();
    let mut client = client(&daemon);
    client.type_line("seq 1 200");
    client.wait_for_row("200").unwrap();
    wheel_up(&mut client, 10, 10);
    assert!(in_copy_mode(&mut client), "{}", client.screen_text());
    // Three rows up: the last row shown is no longer the prompt, and the newest lines are gone.
    let shown = client.content_rows();
    assert!(
        !shown.iter().any(|r| r == "$" || r == "200" || r == "199"),
        "{shown:?}"
    );
    assert!(shown.iter().any(|r| r == "198"), "{shown:?}");
    // Further up, and then back to the bottom: copy mode ends by itself.
    wheel_up(&mut client, 10, 10);
    wheel_down(&mut client, 10, 10);
    assert!(in_copy_mode(&mut client));
    wheel_down(&mut client, 10, 10);
    assert!(left_copy_mode(&mut client), "{}", client.screen_text());
    assert!(client.content_rows().iter().any(|r| r == "200"));
}

#[test]
fn copy_mode_opened_with_the_keyboard_does_not_close_when_the_wheel_reaches_the_bottom() {
    let daemon = TestDaemon::start();
    let mut client = client(&daemon);
    client.type_line("seq 1 100");
    client.wait_for_row("100").unwrap();
    client.send(PREFIX);
    client.send(b"[");
    assert!(in_copy_mode(&mut client));
    wheel_down(&mut client, 10, 10);
    wheel_down(&mut client, 10, 10);
    quiet(&mut client);
    assert!(bar(&client).contains(" COPY "), "{}", bar(&client));
    // And once the wheel took it further up, it still stays when it comes back down.
    wheel_up(&mut client, 10, 10);
    wheel_down(&mut client, 10, 10);
    quiet(&mut client);
    assert!(bar(&client).contains(" COPY "));
}

#[test]
fn the_wheel_scrolls_the_pane_under_the_mouse_and_leaves_the_focus_where_it_is() {
    let daemon = TestDaemon::start();
    let mut client = client(&daemon);
    let divider = split(&mut client);
    let right = client.focused_pane_id();
    click(&mut client, 5, 3);
    client.type_line("seq 1 200");
    let ok = client.wait_until(WAIT, |c| {
        c.region(0, 0, divider, 29).iter().any(|r| r == "200")
    });
    assert!(ok, "{}", client.screen_text());
    click(&mut client, divider + 5, 3);
    assert_eq!(client.focused_pane_id(), right);
    wheel_up(&mut client, 5, 10);
    let ok = client.wait_until(WAIT, |c| {
        let left = c.region(0, 0, divider, 29);
        !left.iter().any(|r| r == "200") && left.iter().any(|r| r == "198")
    });
    assert!(ok, "{}", client.screen_text());
    assert_eq!(client.focused_pane_id(), right);
}

#[test]
fn dragging_selects_text_and_letting_go_does_not_copy() {
    let daemon = TestDaemon::start();
    let mut client = client(&daemon);
    client.type_line("echo hello-world");
    client.wait_for_row("hello-world").unwrap();
    let row = client
        .screen_rows()
        .iter()
        .position(|r| r == "hello-world")
        .unwrap() as u16;
    drag(&mut client, (0, row), (4, row));
    assert!(in_copy_mode(&mut client), "{}", client.screen_text());
    quiet(&mut client);
    // Nothing was copied by letting go.
    assert!(client.clipboard_writes.is_empty());
    assert!(bar(&client).contains(" COPY "));
    // The copy key copies what was selected.
    client.send(b"y");
    let ok = client.wait_until(WAIT, |c| !c.clipboard_writes.is_empty());
    assert!(ok, "{}", client.screen_text());
    assert_eq!(client.clipboard_writes, vec![b"hello".to_vec()]);
}

#[test]
fn a_click_without_a_drag_does_not_open_copy_mode() {
    let daemon = TestDaemon::start();
    let mut client = client(&daemon);
    client.type_line("echo hello-world");
    client.wait_for_row("hello-world").unwrap();
    click(&mut client, 2, 3);
    // Moves inside the cell of the press are not a drag either.
    press(&mut client, 2, 3);
    report(&mut client, 32, 2, 3, false);
    release(&mut client, 2, 3);
    quiet(&mut client);
    assert!(!bar(&client).contains(" COPY "));
}

#[test]
fn a_selection_stops_at_the_edge_of_the_pane_it_began_in() {
    let daemon = TestDaemon::start();
    let mut client = client(&daemon);
    let divider = split(&mut client);
    client.type_line("echo right-text");
    let ok = client.wait_until(WAIT, |c| {
        c.region(divider + 1, 0, 99, 29)
            .iter()
            .any(|r| r == "right-text")
    });
    assert!(ok, "{}", client.screen_text());
    click(&mut client, 5, 3);
    client.type_line("echo left-text");
    let ok = client.wait_until(WAIT, |c| {
        c.region(0, 0, divider, 29).iter().any(|r| r == "left-text")
    });
    assert!(ok, "{}", client.screen_text());
    let row = client
        .region(0, 0, divider, 29)
        .iter()
        .position(|r| r == "left-text")
        .unwrap() as u16;
    // From the first column of the left pane to the far right of the screen.
    drag(&mut client, (0, row), (99, row));
    assert!(in_copy_mode(&mut client));
    client.send(b"y");
    let ok = client.wait_until(WAIT, |c| !c.clipboard_writes.is_empty());
    assert!(ok, "{}", client.screen_text());
    let text = String::from_utf8(client.clipboard_writes[0].clone()).unwrap();
    assert!(text.starts_with("left-text"), "{text:?}");
    assert!(!text.contains("right"), "{text:?}");
}

#[test]
fn a_program_with_the_mouse_gets_the_drag_and_alt_makes_arcade_select_instead() {
    let daemon = TestDaemon::start();
    let mut client = client(&daemon);
    mouse_program(&mut client, "1002h\\033[?1006h");
    drag(&mut client, (1, 3), (6, 3));
    wait_screen_has(&mut client, "^[[<32;");
    assert!(!bar(&client).contains(" COPY "));
    // With Alt held, the program is left out and a selection begins.
    client.send(b"\x03");
    let mut other = daemon
        .attach(AttachOptions::new().size(100, 30).named("alt"))
        .unwrap();
    other.wait_for_prompt().unwrap();
    mouse_program(&mut other, "1002h\\033[?1006h");
    report(&mut other, 8, 1, 3, false);
    report(&mut other, 8 + 32, 6, 3, false);
    report(&mut other, 8, 6, 3, true);
    assert!(in_copy_mode(&mut other), "{}", other.screen_text());
    assert!(
        !other.screen_text().contains("^[[<"),
        "{}",
        other.screen_text()
    );
}

#[test]
fn a_list_a_prompt_and_the_lock_keep_the_mouse_from_the_pane() {
    let daemon = TestDaemon::start();
    let mut client = client(&daemon);
    let divider = split(&mut client);
    let right = client.focused_pane_id();

    // With a list open nothing the mouse does reaches the panes.
    client.send(PREFIX);
    client.send(b"w");
    client.wait_for_text("Windows").unwrap();
    let before = client.screen_text();
    click(&mut client, 5, 3);
    wheel_up(&mut client, 5, 3);
    drag(&mut client, (1, 1), (8, 1));
    quiet(&mut client);
    assert_eq!(client.screen_text(), before);
    client.send(b"\x1b[27u");
    assert!(client.wait_until(WAIT, |c| c.screen_text() != before));
    assert_eq!(client.focused_pane_id(), right);

    // Locked, a pane whose program did not ask for the mouse is not moved to either.
    client.send(PREFIX);
    client.send(b"g");
    click(&mut client, 5, 3);
    wheel_up(&mut client, divider + 5, 3);
    quiet(&mut client);
    assert!(!bar(&client).contains(" COPY "));
    assert_eq!(client.focused_pane_id(), right);
}
