//! Links (OSC 8) a program writes in a pane reach the host terminal, on the cells they belong to
//! and on no others.

use arcade_testkit::{AttachOptions, Screened, TestClient, TestDaemon, WAIT};

const PREFIX: &[u8] = b"\x1b[109;5u";
const URL: &str = "https://example.com/a?b=1;c=2";

fn client(daemon: &TestDaemon, size: (u16, u16), command: Option<&[&str]>) -> TestClient {
    let mut options = AttachOptions::new().size(size.0, size.1).named("lnk");
    if let Some(argv) = command {
        options = options.command(argv);
    }
    let mut client = daemon.attach(options).unwrap();
    if command.is_none() {
        client.wait_for_prompt().unwrap();
    }
    client
}

/// The link of every cell of a row of the client's screen.
fn links_of_row(client: &TestClient, row: u16) -> Vec<Option<String>> {
    (0..client.screen.cols())
        .map(|col| client.screen.hyperlink_at(row, col))
        .collect()
}

/// The row and first column where `needle` is drawn.
fn find(client: &TestClient, needle: &str) -> Option<(u16, u16)> {
    client
        .screen_rows()
        .iter()
        .enumerate()
        .find_map(|(y, row)| {
            row.find(needle)
                .map(|at| (y as u16, row[..at].chars().count() as u16))
        })
}

fn printf_link(text: &str) -> String {
    format!("printf '\\033]8;;{URL}\\033\\\\{text}\\033]8;;\\033\\\\ after\\n'")
}

#[test]
fn the_cells_of_a_link_carry_its_address_and_the_cells_around_them_do_not() {
    let daemon = TestDaemon::start();
    let mut client = client(&daemon, (80, 20), None);
    client.type_line(&printf_link("click-me"));
    client.wait_for_text("click-me after").unwrap();
    let (row, col) = find(&client, "click-me after").unwrap();
    let links = links_of_row(&client, row);
    for (x, link) in links.iter().enumerate() {
        let inside = (usize::from(col)..usize::from(col) + 8).contains(&x);
        assert_eq!(
            link.as_deref(),
            inside.then_some(URL),
            "column {x} of {links:?}"
        );
    }
    // The status bar has none.
    let status = client.screen.rows() - 1;
    assert!(links_of_row(&client, status).iter().all(Option::is_none));
}

#[test]
fn a_link_that_wraps_stays_one_link_and_stops_at_the_edge_of_its_pane() {
    let daemon = TestDaemon::start();
    let text = "w".repeat(60);
    let script = format!("{}; sleep 30", printf_link(&text));
    let mut client = client(&daemon, (100, 20), Some(&["sh", "-c", &script]));
    client.wait_for_text(" after").unwrap();
    client.split_right();
    let (row, _) = find(&client, "wwww").unwrap();
    let first = links_of_row(&client, row);
    let second = links_of_row(&client, row + 1);
    // The pane on the left ends where the divider is: the link is on its last column, not on
    // the divider or the new pane beside it.
    let divider = (0..client.screen.cols())
        .find(|&x| client.screen.row_text(row).chars().nth(usize::from(x)) == Some('│'))
        .unwrap();
    assert_eq!(first[usize::from(divider) - 1].as_deref(), Some(URL));
    assert!(first[usize::from(divider)..].iter().all(Option::is_none));
    assert_eq!(second[0].as_deref(), Some(URL));
    assert!(second[usize::from(divider)..].iter().all(Option::is_none));
}

#[test]
fn a_pane_in_copy_mode_shows_its_text_without_the_link() {
    let daemon = TestDaemon::start();
    let mut client = client(&daemon, (80, 20), None);
    client.type_line(&printf_link("click-me"));
    client.wait_for_text("click-me after").unwrap();
    let (row, col) = find(&client, "click-me after").unwrap();
    assert!(client.screen.hyperlink_at(row, col).is_some());
    client.send(PREFIX);
    client.send(b"[");
    assert!(client.wait_until(WAIT, |c| format!("{} ", c.status_row()).contains(" COPY ")));
    let gone = client.wait_until(WAIT, |c| c.screen.hyperlink_at(row, col).is_none());
    assert!(gone, "{}", client.screen_text());
    assert!(client.screen_text().contains("click-me after"));
}

#[test]
fn a_link_with_a_control_byte_or_a_non_ascii_character_is_not_passed_on() {
    let daemon = TestDaemon::start();
    let mut client = client(&daemon, (80, 20), None);
    client.type_line(
        "printf '\\033]8;;https://bad.example/\\001x\\033\\\\bad-one\\033]8;;\\033\\\\ end\\n'",
    );
    client.type_line(
        "printf '\\033]8;;https://bad.example/\\303\\251\\033\\\\bad-two\\033]8;;\\033\\\\ end\\n'",
    );
    client.wait_for_text("bad-two end").unwrap();
    for needle in ["bad-one", "bad-two"] {
        let (row, col) = find(&client, needle).unwrap();
        assert!(client.screen.hyperlink_at(row, col).is_none(), "{needle}");
    }
}
