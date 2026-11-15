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

mod opening {
    use super::*;
    use arcade_testkit::wait_for_file;
    use std::os::unix::fs::PermissionsExt;
    use std::path::PathBuf;
    use std::sync::atomic::{AtomicUsize, Ordering};

    static COUNTER: AtomicUsize = AtomicUsize::new(0);

    /// An opener that records how many arguments it got and what the first was.
    fn opener() -> (PathBuf, PathBuf) {
        let dir = std::env::temp_dir().join(format!(
            "arc-open-{}-{}",
            std::process::id(),
            COUNTER.fetch_add(1, Ordering::Relaxed)
        ));
        std::fs::create_dir_all(&dir).unwrap();
        let script = dir.join("opener.sh");
        let record = dir.join("record");
        std::fs::write(
            &script,
            format!(
                "#!/bin/sh\nprintf '%s\\n%s\\n' \"$#\" \"$1\" > '{}.tmp' && mv '{}.tmp' '{}'\n",
                record.display(),
                record.display(),
                record.display()
            ),
        )
        .unwrap();
        std::fs::set_permissions(&script, std::fs::Permissions::from_mode(0o755)).unwrap();
        (script, record)
    }

    fn daemon_with(script: &std::path::Path) -> TestDaemon {
        let script = script.to_string_lossy().into_owned();
        TestDaemon::start_with(move |c| c.config.links.command = Some(vec![script]))
    }

    fn go_to(client: &mut TestClient, text: &str) {
        client.send(PREFIX);
        client.send(b"[");
        assert!(client.wait_until(WAIT, |c| format!("{} ", c.status_row()).contains(" COPY ")));
        client.send(b"/");
        client.send(text.as_bytes());
        client.send(b"\r");
    }

    #[test]
    fn a_link_with_shell_characters_reaches_the_opener_as_one_untouched_argument() {
        let (script, record) = opener();
        let daemon = daemon_with(&script);
        let mut client = client(&daemon, (100, 20), None);
        let marker = record.with_file_name("pwned");
        let nasty = format!(
            "https://e.example/a;b$(touch {})'q\"x&y|z>w",
            marker.display()
        );
        // The program writes the address in an OSC 8 link; the shell's printf must not mangle it.
        std::fs::write(
            record.with_file_name("nasty.txt"),
            format!("\x1b]8;;{nasty}\x1b\\click-here\x1b]8;;\x1b\\\n"),
        )
        .unwrap();
        client.type_line(&format!(
            "cat '{}'",
            record.with_file_name("nasty.txt").display()
        ));
        client.wait_for_text("click-here").unwrap();
        go_to(&mut client, "click-here");
        client.send(b"o");
        assert!(
            wait_for_file(&record, WAIT),
            "the opener did not run; the screen:\n{}",
            client.screen_text()
        );
        let got = std::fs::read_to_string(&record).unwrap();
        assert_eq!(got, format!("1\n{nasty}\n"));
        assert!(!marker.exists(), "a shell ran the address");
        // Copy mode goes on.
        assert!(
            format!("{} ", client.status_row()).contains(" COPY ")
                || client.status_row().contains("opening")
        );
    }

    #[test]
    fn a_plain_address_in_the_text_is_opened_and_one_of_another_kind_is_not() {
        let (script, record) = opener();
        let daemon = daemon_with(&script);
        let mut client = client(&daemon, (100, 20), None);
        client.type_line("echo see ftp://nope.example/x and https://yes.example/p.");
        client.wait_for_text("and https://yes.example/p.").unwrap();
        go_to(&mut client, "nope");
        client.send(b"o");
        assert!(client.wait_until(WAIT, |c| c.status_row().contains("no link")));
        assert!(!record.exists());
        client.send(b"/yes.example\r");
        client.send(b"o");
        assert!(wait_for_file(&record, WAIT));
        assert_eq!(
            std::fs::read_to_string(&record).unwrap(),
            "1\nhttps://yes.example/p\n"
        );
    }

    #[test]
    fn an_opener_that_cannot_start_is_reported_on_the_status_bar() {
        let daemon = daemon_with(std::path::Path::new("/nonexistent/opener"));
        let mut client = client(&daemon, (100, 20), None);
        client.type_line("echo https://yes.example/p");
        client.wait_for_text("https://yes.example/p").unwrap();
        go_to(&mut client, "yes.example");
        client.send(b"o");
        assert!(client.wait_until(WAIT, |c| c.status_row().contains("cannot run")));
    }
}
