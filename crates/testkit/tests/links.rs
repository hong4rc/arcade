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
fn a_pane_in_copy_mode_keeps_the_links_of_its_text() {
    let daemon = TestDaemon::start();
    let mut client = client(&daemon, (80, 20), None);
    client.type_line(&printf_link("click-me"));
    client.wait_for_text("click-me after").unwrap();
    let (row, col) = find(&client, "click-me after").unwrap();
    assert!(client.screen.hyperlink_at(row, col).is_some());
    client.send(PREFIX);
    client.send(b"[");
    assert!(client.wait_until(WAIT, |c| format!("{} ", c.status_row()).contains(" COPY ")));
    assert!(client.screen_text().contains("click-me after"));
    let (row, col) = find(&client, "click-me after").unwrap();
    for x in col..col + 8 {
        assert_eq!(client.screen.hyperlink_at(row, x).as_deref(), Some(URL));
    }
    assert_eq!(client.screen.hyperlink_at(row, col + 8), None);
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
    fn an_address_a_program_broke_over_two_rows_is_opened_whole() {
        let (script, record) = opener();
        let daemon = daemon_with(&script);
        // A 60-column pane: the program wraps at the pane width and indents the rest.
        let mut client = client(&daemon, (60, 20), None);
        let first = format!("{:<60}", "").replace(' ', "7");
        let first = format!(
            "Use this link: https://h.example/wiki/{}",
            &first[..60 - 38]
        );
        assert_eq!(first.len(), 60);
        let cat = record.with_file_name("wrapped.txt");
        std::fs::write(&cat, format!("{first}\n  558/DevOps+request\n")).unwrap();
        client.type_line(&format!("cat '{}'", cat.display()));
        client.wait_for_text("558/DevOps+request").unwrap();
        go_to(&mut client, "558/Dev");
        client.send(b"o");
        assert!(wait_for_file(&record, WAIT), "{}", client.screen_text());
        assert_eq!(
            std::fs::read_to_string(&record).unwrap(),
            format!("1\n{}558/DevOps+request\n", &first[15..])
        );
        // The status bar shows the whole address that was opened.
        assert!(client.wait_until(WAIT, |c| c.status_row().contains("opening")));
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

mod two_rows {
    use super::*;

    const LONG: &str = "https://host.example/wiki/spaces/EV/pages/3954966558/DevOps+request";

    /// Print one program output that shows `first` on a row and `second` on the next, indented by
    /// two cells, with the link written the way `open` and `close` say, and check what reaches the
    /// host: both parts carry the same address, and the same id when the program gave one.
    fn check(label: &str, script_body: &str, expect_same_link_id: bool) {
        let daemon = TestDaemon::start();
        let script = format!("{script_body}; sleep 30");
        let mut client = client(&daemon, (80, 20), Some(&["sh", "-c", &script]));
        client.wait_for_text("558/DevOps").unwrap();
        let (row1, col1) = find(&client, "https://host").unwrap();
        let (row2, col2) = find(&client, "558/DevOps").unwrap();
        assert_eq!(row2, row1 + 1, "{label}");
        assert_eq!(col2, 2, "{label}: the indent is kept");
        let first = client.screen.hyperlink_at(row1, col1);
        let second = client.screen.hyperlink_at(row2, col2);
        assert_eq!(first.as_deref(), Some(LONG), "{label}: first part");
        assert_eq!(second.as_deref(), Some(LONG), "{label}: second part");
        // The indent is not part of the link when the program closed it before the indent.
        let bytes = String::from_utf8_lossy(&client.raw_output).into_owned();
        let ids: Vec<&str> = bytes
            .split("\x1b]8;")
            .skip(1)
            .filter_map(|s| s.split(';').next())
            .filter(|p| !p.is_empty())
            .collect();
        assert!(!ids.is_empty(), "{label}: ids {ids:?}");
        if expect_same_link_id {
            let first_id = ids[0];
            assert!(ids.iter().all(|i| *i == first_id), "{label}: ids {ids:?}");
        }
    }

    #[test]
    fn one_link_over_two_rows_without_an_id_reaches_the_host_as_one_address() {
        check(
            "continuous, no id",
            &format!(
                "printf 'Use this link instead: \\033]8;;{LONG}\\033\\\\https://host.example/wiki/spaces/EV/pages/3954966\\n  558/DevOps+request\\033]8;;\\033\\\\\\n'"
            ),
            true,
        );
    }

    #[test]
    fn one_link_over_two_rows_with_an_id_reaches_the_host_as_one_address() {
        check(
            "continuous, id",
            &format!(
                "printf 'Use this link instead: \\033]8;id=abc;{LONG}\\033\\\\https://host.example/wiki/spaces/EV/pages/3954966\\n  558/DevOps+request\\033]8;;\\033\\\\\\n'"
            ),
            true,
        );
    }

    #[test]
    fn a_link_opened_again_on_the_second_row_has_the_same_address_and_id() {
        check(
            "reopened per row, id",
            &format!(
                "printf 'Use this link instead: \\033]8;id=abc;{LONG}\\033\\\\https://host.example/wiki/spaces/EV/pages/3954966\\033]8;;\\033\\\\\\n  \\033]8;id=abc;{LONG}\\033\\\\558/DevOps+request\\033]8;;\\033\\\\\\n'"
            ),
            true,
        );
    }
}

mod redraws {
    use super::*;

    /// Every cell of `click-me` on the screen carries the link, and no other cell of its row does.
    fn whole_link(client: &TestClient, what: &str) {
        let (row, col) = find(client, "click-me").unwrap_or_else(|| {
            panic!("{what}: the text is gone:\n{}", client.screen_text());
        });
        let links = links_of_row(client, row);
        for (x, link) in links.iter().enumerate() {
            let inside = (usize::from(col)..usize::from(col) + 8).contains(&x);
            assert_eq!(link.as_deref(), inside.then_some(URL), "{what}: column {x}");
        }
    }

    fn print_link_at(row: u16, col: u16) -> String {
        format!("\\033[{row};{col}H\\033]8;;{URL}\\033\\\\click-me\\033]8;;\\033\\\\ after")
    }

    #[test]
    fn a_link_stays_whole_after_a_resize_a_second_client_and_a_split() {
        let daemon = TestDaemon::start();
        let script = format!("printf '\\033[2J{}'; sleep 60", print_link_at(5, 3));
        let mut client = client(&daemon, (80, 20), Some(&["sh", "-c", &script]));
        client.wait_for_text("click-me after").unwrap();
        whole_link(&client, "first print");
        client.resize(70, 18);
        client.wait_until(WAIT, |c| c.screen.cols() == 70);
        whole_link(&client, "after a resize");
        // A second client gets a full repaint of the same session.
        let mut second = daemon
            .attach(AttachOptions::new().size(70, 18).existing(Some("lnk")))
            .unwrap();
        second.wait_for_text("click-me after").unwrap();
        whole_link(&second, "a client that attached later");
        second.split_right();
        second.wait_for_text("│").unwrap();
        whole_link(&second, "after a split");
    }

    #[test]
    fn a_link_stays_whole_when_the_program_redraws_part_of_it_or_scrolls_a_region() {
        let daemon = TestDaemon::start();
        let again = print_link_at(5, 3);
        let script = format!(
            "printf '\\033[2J{again}'; sleep 3; \
             printf '\\033[5;3Hxxx'; sleep 3; \
             printf '\\033[5;3H\\033]8;;{URL}\\033\\\\cli\\033]8;;\\033\\\\'; sleep 3; \
             printf '\\033[1;10r\\033[10;1H\\033[2S'; sleep 60"
        );
        let mut client = client(&daemon, (80, 20), Some(&["sh", "-c", &script]));
        client.wait_for_text("click-me after").unwrap();
        whole_link(&client, "first print");
        // Part of the link overwritten with plain text: those cells are not a link any more.
        client.wait_for_text("xxxck-me after").unwrap();
        let (row, col) = find(&client, "xxxck-me").unwrap();
        let links = links_of_row(&client, row);
        assert!(
            links[usize::from(col)..usize::from(col) + 3]
                .iter()
                .all(Option::is_none)
        );
        assert_eq!(links[usize::from(col) + 3].as_deref(), Some(URL));
        // The first three cells written again as a link: the whole word is the link again.
        client.wait_for_text("click-me after").unwrap();
        whole_link(&client, "after the part was written again");
        // The region scrolled two rows up: the link moved with its text.
        assert!(client.wait_until(WAIT, |c| {
            find(c, "click-me").is_some_and(|(r, _)| r == row - 2)
        }));
        whole_link(&client, "after a region scrolled");
    }
}

mod scrolled {
    use super::*;

    fn wheel(client: &mut TestClient, up: bool) {
        let code = if up { 64 } else { 65 };
        client.send(format!("\x1b[<{code};11;11M").as_bytes());
    }

    /// 80 lines; every tenth one is a link `click-NN` (8 cells), then the program waits.
    fn script() -> String {
        format!(
            "i=1; while [ $i -le 80 ]; do \
               if [ $((i % 10)) -eq 0 ]; then printf '\\033]8;;{URL}\\033\\\\click-%s\\033]8;;\\033\\\\ end\\n' $i; \
               else echo line-$i; fi; i=$((i+1)); done; sleep 60"
        )
    }

    fn start(daemon: &TestDaemon) -> TestClient {
        let script = script();
        let mut client = client(daemon, (100, 30), Some(&["sh", "-c", &script]));
        client.wait_for_text("click-80 end").unwrap();
        client
    }

    /// Every `click-NN` on the screen is a link on its 8 cells and on no other cell of its row.
    fn all_links_whole(client: &TestClient) -> usize {
        let mut seen = 0;
        for (y, row) in client.screen_rows().iter().enumerate() {
            let Some(at) = row.find("click-") else {
                continue;
            };
            let col = row[..at].chars().count();
            let links = links_of_row(client, y as u16);
            for (x, link) in links.iter().enumerate() {
                let inside = (col..col + 8).contains(&x);
                assert_eq!(link.as_deref(), inside.then_some(URL), "row {y} column {x}");
            }
            seen += 1;
        }
        seen
    }

    fn enter_copy(client: &mut TestClient) {
        client.send(PREFIX);
        client.send(b"[");
        assert!(client.wait_until(WAIT, |c| format!("{} ", c.status_row()).contains(" COPY ")));
    }

    #[test]
    fn links_stay_on_rows_scrolled_back_by_the_wheel() {
        let daemon = TestDaemon::start();
        let mut client = start(&daemon);
        assert!(all_links_whole(&client) >= 2);
        for _ in 0..8 {
            wheel(&mut client, true);
        }
        assert!(client.wait_until(WAIT, |c| format!("{} ", c.status_row()).contains(" COPY ")));
        assert!(client.wait_until(WAIT, |c| !c.screen_text().contains("click-80")));
        assert!(all_links_whole(&client) >= 2, "{}", client.screen_text());
        // And back at the bottom the live rows are as before.
        for _ in 0..12 {
            wheel(&mut client, false);
        }
        assert!(client.wait_until(WAIT, |c| !format!("{} ", c.status_row()).contains(" COPY ")));
        assert!(client.wait_until(WAIT, |c| c.screen_text().contains("click-80 end")));
        assert!(all_links_whole(&client) >= 2);
    }

    #[test]
    fn links_stay_on_rows_shown_by_page_up_and_page_down() {
        let daemon = TestDaemon::start();
        let mut client = start(&daemon);
        enter_copy(&mut client);
        client.send(b"\x1b[98;5u");
        client.send(b"\x1b[98;5u");
        assert!(client.wait_until(WAIT, |c| !c.screen_text().contains("click-80")));
        assert!(all_links_whole(&client) >= 1, "{}", client.screen_text());
        client.send(b"\x1b[98;5u");
        client.send(b"\x1b[102;5u");
        assert!(client.wait_until(WAIT, |c| {
            c.screen_rows().iter().any(|r| r.contains("click-"))
        }));
        assert!(all_links_whole(&client) >= 1, "{}", client.screen_text());
    }

    #[test]
    fn a_link_that_starts_in_the_scrollback_and_ends_on_the_screen_is_whole() {
        let daemon = TestDaemon::start();
        // A link of 150 cells wraps over two rows of a 100-column pane; 28 more lines push the
        // first row out of the live screen.
        let text = "w".repeat(150);
        let script = format!(
            "{}; i=1; while [ $i -le 27 ]; do echo pad-$i; i=$((i+1)); done; sleep 60",
            printf_link(&text)
        );
        let mut client = client(&daemon, (100, 30), Some(&["sh", "-c", &script]));
        client.wait_for_text("pad-27").unwrap();
        enter_copy(&mut client);
        // The second row of the link is on the first row of the screen; the first row is above.
        let first = client.screen_rows()[0].clone();
        assert!(first.starts_with(&"w".repeat(50)), "{first:?}");
        assert!(client.screen_rows()[0].contains("[0/1]"), "{first:?}");
        let links = links_of_row(&client, 0);
        assert!(
            links[..50].iter().all(|l| l.as_deref() == Some(URL)),
            "{links:?}"
        );
        // Scroll up one row: both parts show, both with the link.
        wheel(&mut client, true);
        assert!(
            client.wait_until(WAIT, |c| c.screen_rows()[0].starts_with("www")
                && c.screen_rows()[1].starts_with("www"))
        );
        let (row, _) = find(&client, "wwww").unwrap();
        assert_eq!(links_of_row(&client, row)[0].as_deref(), Some(URL));
        assert_eq!(links_of_row(&client, row + 1)[0].as_deref(), Some(URL));
    }

    #[test]
    fn a_selection_over_a_link_leaves_the_link_whole() {
        let daemon = TestDaemon::start();
        let mut client = start(&daemon);
        enter_copy(&mut client);
        client.send(b"/click-80\r");
        assert!(client.wait_until(WAIT, |c| c.status_row().contains("COPY")));
        client.send(b"0");
        client.send(b"v");
        client.send(b"llllll");
        assert!(
            client.wait_until(WAIT, |c| c.status_row().contains("V-CHAR")
                || c.status_row().contains("VISUAL"))
        );
        // The cells inside the selection and those outside it all carry the link.
        let (row, col) = find(&client, "click-80").unwrap();
        let whole = |client: &TestClient| {
            for x in col..col + 8 {
                assert_eq!(
                    client.screen.hyperlink_at(row, x).as_deref(),
                    Some(URL),
                    "{x}"
                );
            }
            assert_eq!(client.screen.hyperlink_at(row, col + 8), None);
        };
        whole(&client);
        // The selection is drawn on those cells (their look changed), and the link is still there.
        assert_ne!(client.style_at(row, col), arcade_term::Style::default());
    }

    #[test]
    fn an_address_that_is_not_safe_is_not_passed_on_from_the_scrollback_either() {
        let daemon = TestDaemon::start();
        let script = "printf '\\033]8;;https://bad.example/\\303\\251\\033\\\\bad-one\\033]8;;\\033\\\\ end\\n'; \
                      i=1; while [ $i -le 40 ]; do echo pad-$i; i=$((i+1)); done; sleep 60";
        let mut client = client(&daemon, (100, 30), Some(&["sh", "-c", script]));
        client.wait_for_text("pad-40").unwrap();
        for _ in 0..16 {
            wheel(&mut client, true);
        }
        assert!(client.wait_until(WAIT, |c| c.screen_text().contains("bad-one end")));
        let (row, col) = find(&client, "bad-one").unwrap();
        assert!(client.screen.hyperlink_at(row, col).is_none());
    }
}

/// A measurement, not a check: a pane with 3000 lines of history, every line a link, scrolled
/// by the wheel and by page. Run with `--release --ignored --nocapture`; it prints the frames,
/// the bytes and the time the client needed for the same movements.
#[test]
#[ignore]
fn measure_scrolling_a_pane_full_of_links() {
    for round in 0..4 {
        let daemon = TestDaemon::start();
        let script = format!(
            "i=1; while [ $i -le 3000 ]; do printf '\\033]8;;{URL}?n=%s\\033\\\\link-number-%s\\033]8;;\\033\\\\ and some words around it\\n' $i $i; i=$((i+1)); done; sleep 120"
        );
        let mut client = client(&daemon, (100, 30), Some(&["sh", "-c", &script]));
        client.wait_for_text("link-number-3000").unwrap();
        let (m0, b0) = (client.output_messages, client.output_bytes);
        // 100 wheel turns up, 100 down, then 20 pages up and down: each is sent alone, and the
        // time until the frame it causes has arrived is added up.
        let mut moves: Vec<&[u8]> = Vec::new();
        moves.extend(std::iter::repeat_n(&b"\x1b[<64;11;11M"[..], 100));
        moves.extend(std::iter::repeat_n(&b"\x1b[<65;11;11M"[..], 100));
        moves.extend(std::iter::repeat_n(&b"\x1b[98;5u"[..], 20));
        moves.extend(std::iter::repeat_n(&b"\x1b[102;5u"[..], 20));
        let mut total = std::time::Duration::ZERO;
        for key in moves {
            let before = client.output_messages;
            let start = std::time::Instant::now();
            client.send(key);
            while client.output_messages == before
                && start.elapsed() < std::time::Duration::from_secs(2)
            {
                client.read_more(std::time::Instant::now() + std::time::Duration::from_millis(20));
            }
            total += start.elapsed();
        }
        let frames = (client.output_messages - m0).max(1);
        eprintln!(
            "round {round}: {frames} frames, {} bytes ({} per frame), {:?} per frame",
            client.output_bytes - b0,
            (client.output_bytes - b0) / frames,
            total / frames as u32
        );
    }
}

mod underline {
    use super::*;
    use arcade_term::Underline;

    fn underline_at(client: &TestClient, row: u16, col: u16) -> Underline {
        client.style_at(row, col).underline
    }

    #[test]
    fn a_link_that_is_passed_on_is_underlined_and_the_cells_around_it_are_not() {
        let daemon = TestDaemon::start();
        let mut client = client(&daemon, (80, 20), None);
        client.type_line(&printf_link("click-me"));
        client.wait_for_text("click-me after").unwrap();
        let (row, col) = find(&client, "click-me after").unwrap();
        for x in col..col + 8 {
            assert_eq!(underline_at(&client, row, x), Underline::Dotted, "{x}");
        }
        for x in [col + 8, col + 9] {
            assert_eq!(underline_at(&client, row, x), Underline::None, "{x}");
        }
    }

    #[test]
    fn a_link_that_is_not_passed_on_is_not_underlined() {
        let daemon = TestDaemon::start();
        let script = "printf '\\033]8;;https://bad.example/\\303\\251\\033\\\\bad-one\\033]8;;\\033\\\\ end\\n'; sleep 30";
        let mut client = client(&daemon, (80, 20), Some(&["sh", "-c", script]));
        client.wait_for_text("bad-one end").unwrap();
        let (row, col) = find(&client, "bad-one").unwrap();
        assert!(client.screen.hyperlink_at(row, col).is_none());
        assert_eq!(underline_at(&client, row, col), Underline::None);
    }

    #[test]
    fn the_programs_own_underline_is_kept() {
        let daemon = TestDaemon::start();
        let script = format!(
            "printf '\\033]8;;{URL}\\033\\\\\\033[4:3m\\033[58;5;9mcurly\\033[0m-plain\\033]8;;\\033\\\\ end\\n'; sleep 30"
        );
        let mut client = client(&daemon, (80, 20), Some(&["sh", "-c", &script]));
        client.wait_for_text("-plain end").unwrap();
        let (row, col) = find(&client, "curly").unwrap();
        // The program's curly underline and its colour stay; the plain part gets the link's.
        assert_eq!(underline_at(&client, row, col), Underline::Curly);
        assert_ne!(
            client.style_at(row, col).underline_color,
            arcade_term::Color::Default
        );
        assert_eq!(underline_at(&client, row, col + 6), Underline::Dotted);
    }

    #[test]
    fn with_the_setting_off_nothing_is_underlined() {
        let daemon = TestDaemon::start_with(|c| c.config.links.underline = false);
        let mut client = client(&daemon, (80, 20), None);
        client.type_line(&printf_link("click-me"));
        client.wait_for_text("click-me after").unwrap();
        let (row, col) = find(&client, "click-me after").unwrap();
        assert!(client.screen.hyperlink_at(row, col).is_some());
        assert_eq!(underline_at(&client, row, col), Underline::None);
        // No cell of the screen is underlined.
        for y in 0..client.screen.rows() {
            for x in 0..client.screen.cols() {
                assert_eq!(underline_at(&client, y, x), Underline::None, "{y},{x}");
            }
        }
    }

    #[test]
    fn copy_mode_keeps_the_underline_and_the_copied_text_has_none_of_it() {
        let daemon = TestDaemon::start();
        let mut client = client(&daemon, (80, 20), None);
        client.type_line(&printf_link("click-me"));
        client.wait_for_text("click-me after").unwrap();
        client.send(PREFIX);
        client.send(b"[");
        assert!(client.wait_until(WAIT, |c| format!("{} ", c.status_row()).contains(" COPY ")));
        let (row, col) = find(&client, "click-me after").unwrap();
        assert_eq!(underline_at(&client, row, col), Underline::Dotted);
        // Select the line of the link and copy it: plain text, no escape sequence.
        client.send(b"/click-me after\r");
        client.send(b"V");
        client.send(b"y");
        assert!(client.wait_until(WAIT, |c| !c.clipboard_writes.is_empty()));
        let copied = String::from_utf8_lossy(&client.clipboard_writes[0]).into_owned();
        assert!(copied.contains("click-me after"), "{copied:?}");
        assert!(!copied.contains('\x1b'));
    }
}
