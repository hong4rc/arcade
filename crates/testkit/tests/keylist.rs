//! The key list: one row for each action, in groups, in a box that keeps its size and place.

use std::path::PathBuf;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::time::Duration;

use arcade_testkit::{AttachOptions, Screened, TestClient, TestDaemon};

const PREFIX: &[u8] = b"\x1b[109;5u";
const QUESTION: &[u8] = b"\x1b[47;2u";
const DOWN: &[u8] = b"\x1b[B";
const UP: &[u8] = b"\x1b[A";
const ESC: &[u8] = b"\x1b";
const WAIT: Duration = Duration::from_secs(5);
const LEGACY_ANSWER: &[u8] = b"\x1b[?62;22c";

const HEADINGS: [&str; 8] = [
    "Panes",
    "Windows",
    "Sessions",
    "Copy and paste",
    "Notifications",
    "Arcade",
    "Copy mode",
    "Lists",
];

static COUNTER: AtomicUsize = AtomicUsize::new(0);

fn config_file(text: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!(
        "arc-keylist-{}-{}",
        std::process::id(),
        COUNTER.fetch_add(1, Ordering::Relaxed)
    ));
    std::fs::create_dir_all(&dir).unwrap();
    let file = dir.join("config.toml");
    std::fs::write(&file, text).unwrap();
    file
}

fn daemon_with(text: &str) -> TestDaemon {
    let path = config_file(text);
    TestDaemon::start_with(move |c| c.config_path = Some(path))
}

fn open_at(daemon: &TestDaemon, cols: u16, rows: u16) -> TestClient {
    let mut client = daemon
        .attach(
            AttachOptions::new()
                .size(cols, rows)
                .named(&format!("k{cols}x{rows}")),
        )
        .unwrap();
    client.wait_for_prompt().unwrap();
    open_list(&mut client);
    client
}

fn open_list(client: &mut TestClient) {
    client.send(PREFIX);
    client.send(QUESTION);
    assert!(client.wait_until(WAIT, |c| !box_rows(c).is_empty()));
    settle(client);
}

/// Read what the daemon has sent so far, so that a screen read after a key shows its effect.
fn settle(client: &mut TestClient) {
    client.read_more(std::time::Instant::now() + Duration::from_millis(120));
}

/// The rows of the box, from the top edge to the bottom edge, each from its first frame cell.
fn box_rows(client: &TestClient) -> Vec<String> {
    let rows = client.screen_rows();
    let top = rows.iter().position(|r| r.contains("┌ Keys"));
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

/// Where the box is and how big: the column of its left edge, the row of its top edge, its width
/// and its height.
fn frame(client: &TestClient) -> (usize, usize, usize, usize) {
    let rows = client.screen_rows();
    let top = rows.iter().position(|r| r.contains("┌ Keys")).unwrap();
    let left = rows[top].find('┌').unwrap();
    let inside = box_rows(client);
    (
        rows[top][..left].chars().count(),
        top,
        inside[0].chars().count(),
        inside.len(),
    )
}

fn inner(row: &str) -> String {
    row.trim_start_matches(['│', '┌', '└'])
        .trim_end_matches(['│', '┐', '┘'])
        .to_string()
}

/// The action of the chosen row (the one with the `> ` mark), when the box has one. The mark is
/// in the first cell of a column, whose place the titles of the columns tell.
fn chosen(client: &TestClient) -> Option<String> {
    let rows = box_rows(client);
    let header = inner(rows.get(2)?);
    let mut marks = vec![1];
    if let Some(second) = header.rmatch_indices("Action").next().map(|(i, _)| i)
        && second > 7
    {
        marks.push(second - 2);
    }
    rows.iter().map(|r| inner(r)).find_map(|r| {
        marks.iter().find_map(|at| {
            let cell: String = r.chars().skip(*at).collect();
            cell.strip_prefix("> ")
                .and_then(|rest| rest.split("  ").next())
                .map(|label| label.trim().to_string())
        })
    })
}

/// The rows of the list: not the frame, the filter line, the column titles or the key line.
fn body(client: &TestClient) -> Vec<String> {
    let rows = box_rows(client);
    let n = rows.len();
    rows[3..n - 2].iter().map(|r| inner(r)).collect()
}

/// The row of the action, with the columns of the first (or the second) half of the box.
fn row_of(client: &TestClient, label: &str) -> String {
    let rows = body(client);
    rows.iter()
        .find(|r| r.contains(label))
        .unwrap_or_else(|| panic!("no row with {label:?}:\n{}", client.screen_text()))
        .clone()
}

/// What is typed in the filter line of the box.
fn filter_text(client: &TestClient) -> String {
    box_rows(client)
        .get(1)
        .map(|r| {
            inner(r)
                .trim()
                .strip_prefix('/')
                .unwrap_or("")
                .trim()
                .to_string()
        })
        .unwrap_or_default()
}

fn type_filter(client: &mut TestClient, text: &str) {
    let wanted = format!("{}{text}", filter_text(client));
    client.send(text.as_bytes());
    assert!(
        client.wait_until(WAIT, |c| filter_text(c) == wanted),
        "{}",
        client.screen_text()
    );
}

fn clear_filter(client: &mut TestClient) {
    client.send(b"\x15");
    assert!(client.wait_until(WAIT, |c| filter_text(c).is_empty()));
}

/// Press a key that moves the choice, and wait for the box to show the new choice (when there
/// is one: at the end of the list nothing changes).
fn press(client: &mut TestClient, key: &[u8]) {
    let before = chosen(client);
    client.send(key);
    client.wait_until(Duration::from_millis(300), |c| chosen(c) != before);
}

#[test]
fn the_list_has_the_groups_in_order_with_a_row_for_each_action() {
    let daemon = TestDaemon::start();
    let client = open_at(&daemon, 200, 70);
    let text = box_rows(&client).join("\n");
    let mut at = 0;
    for heading in &HEADINGS[..6] {
        let found = text[at..]
            .find(heading)
            .unwrap_or_else(|| panic!("{heading} not in order:\n{text}"));
        at += found;
    }
    // One row for the action, with the keys after the prefix and then the direct keys.
    let row = row_of(&client, "focus left");
    assert!(row.contains("h, left"), "{row}");
    assert!(
        row.contains("h, left") && row.matches("h, left").count() == 2,
        "{row}"
    );
    // `pane.focus-down` is on one row with both of its keys, not on two.
    let rows: Vec<String> = body(&client)
        .into_iter()
        .filter(|r| r.contains("focus down"))
        .collect();
    assert_eq!(rows.len(), 1, "{rows:?}");
    assert!(rows[0].contains("j, down"), "{rows:?}");
    // A command without a direct key leaves that column empty.
    let zoom = row_of(&client, "zoom");
    assert!(zoom.trim_end().ends_with('z'), "{zoom}");
    // The keys of copy mode and of the lists are not here until something is typed.
    assert!(
        !text.contains("Copy mode") && !text.contains("copy.left"),
        "{text}"
    );
}

#[test]
fn the_movement_letters_of_the_configuration_show_as_they_are_bound() {
    let daemon = daemon_with("[keys]\nnav = \"hnei\"\n");
    let client = open_at(&daemon, 200, 70);
    for (label, keys) in [
        ("focus left", "h, left"),
        ("focus down", "n, down"),
        ("focus up", "e, up"),
        ("focus right", "i, right"),
        ("resize down", "N, shift+down"),
    ] {
        let row = row_of(&client, label);
        assert_eq!(row.matches(keys).count(), 2, "{row}");
    }
}

#[test]
fn a_row_lists_every_key_of_a_configuration_that_rebinds_unbinds_and_adds() {
    let daemon = daemon_with(
        "[keys.prefix]\n\"b\" = \"pane.zoom\"\n\"z\" = false\n\"f1\" = \"window.new\"\n\"Q\" = false\n[keys.direct]\n\"x\" = \"pane.zoom\"\n\"alt+q\" = \"window.new\"\n",
    );
    let client = open_at(&daemon, 200, 70);
    // Zoom: `b` replaces `z` after the prefix, and a direct key is added.
    let zoom = row_of(&client, "zoom");
    let words: Vec<&str> = zoom.split_whitespace().collect();
    assert_eq!(words, ["zoom", "b", "x"], "{zoom}");
    // New window: its own key stays, a new one is added after the prefix, and a direct key is
    // added (it carries the direct modifier, which the column title names).
    let new = row_of(&client, "new window");
    let words: Vec<&str> = new.split_whitespace().collect();
    assert_eq!(words, ["new", "window", "c,", "f1", "q"], "{new}");
    // An action whose keys are all unbound has no row.
    let daemon = daemon_with("[keys.prefix]\n\"z\" = false\n[keys.direct]\n");
    let client = open_at(&daemon, 200, 70);
    assert!(
        !body(&client).iter().any(|r| r.contains(" zoom")),
        "{}",
        client.screen_text()
    );
}

#[test]
fn the_choice_skips_the_headings() {
    let daemon = TestDaemon::start();
    let mut client = open_at(&daemon, 120, 36);
    let mut seen = Vec::new();
    for _ in 0..70 {
        let row = chosen(&client).expect("a row is chosen");
        assert!(
            !HEADINGS.iter().any(|h| row.starts_with(h)),
            "a heading is chosen: {row:?}"
        );
        if seen.last() != Some(&row) {
            seen.push(row);
        }
        press(&mut client, DOWN);
    }
    let ok = client.wait_until(WAIT, |c| chosen(c).as_deref() == Some("redraw"));
    assert!(ok, "{:?}\n{}", chosen(&client), client.screen_text());
    // It went from the first row to the last, in order across the groups.
    assert!(seen.first().unwrap().contains("focus left"), "{seen:?}");
    assert!(seen.last().unwrap().contains("redraw"), "{seen:?}");
    assert!(seen.iter().any(|r| r.contains("new window")), "{seen:?}");
    // And back up.
    for _ in 0..80 {
        press(&mut client, UP);
    }
    assert!(chosen(&client).unwrap().contains("focus left"));
}

#[test]
fn the_filter_finds_by_key_and_by_command_and_drops_the_empty_groups() {
    let daemon = TestDaemon::start();
    let mut client = open_at(&daemon, 200, 70);
    // By command name.
    type_filter(&mut client, "pane.zoom");
    let rows = body(&client);
    assert!(rows.iter().any(|r| r.contains("zoom")), "{rows:?}");
    assert!(rows.iter().all(|r| !r.contains("focus left")));
    assert!(
        !rows.iter().any(|r| r.trim() == "Windows"),
        "a group with no match shows its heading: {rows:?}"
    );
    assert!(rows.iter().any(|r| r.trim() == "Panes"), "{rows:?}");
    clear_filter(&mut client);
    // By a key as it is typed with the prefix.
    type_filter(&mut client, "ctrl+m t");
    let rows = body(&client);
    assert!(
        rows.iter().any(|r| r.contains("tree of sessions")),
        "{rows:?}"
    );
    clear_filter(&mut client);
    // By a direct key written whole.
    type_filter(&mut client, "ctrl+alt+g");
    let rows = body(&client);
    assert!(rows.iter().any(|r| r.contains("lock")), "{rows:?}");
    assert!(rows.iter().all(|r| !r.contains("zoom")), "{rows:?}");
    clear_filter(&mut client);
    // Nothing matches.
    type_filter(&mut client, "zzzzz");
    assert!(
        body(&client).iter().any(|r| r.contains("no match")),
        "{}",
        client.screen_text()
    );
}

#[test]
fn the_keys_of_copy_mode_and_of_the_lists_are_found_by_typing_and_are_not_run() {
    let daemon = TestDaemon::start();
    let mut client = open_at(&daemon, 200, 70);
    type_filter(&mut client, "copy.word-back");
    let rows = body(&client);
    assert!(rows.iter().any(|r| r.trim() == "Copy mode"), "{rows:?}");
    assert!(
        rows.iter().any(|r| r.contains("copy.word-back")),
        "{rows:?}"
    );
    // Enter closes the list and starts nothing: there is no copy mode to move a word in.
    client.send(b"\r");
    assert!(client.wait_until(WAIT, |c| box_rows(c).is_empty()));
    assert!(
        !client.status_row().contains("COPY"),
        "{}",
        client.status_row()
    );
    // A key of the lists.
    open_list(&mut client);
    type_filter(&mut client, "list.lift");
    let rows = body(&client);
    assert!(rows.iter().any(|r| r.trim() == "Lists"), "{rows:?}");
    assert!(rows.iter().any(|r| r.contains("space")), "{rows:?}");
}

#[test]
fn the_box_keeps_its_place_and_size_while_filtering() {
    let daemon = TestDaemon::start();
    for (cols, rows) in [(120, 36), (80, 24), (200, 70)] {
        let mut client = open_at(&daemon, cols, rows);
        let base = frame(&client);
        for step in ["p", "a", "ne", "zzz"] {
            type_filter(&mut client, step);
            assert_eq!(frame(&client), base, "{cols}x{rows} after {step:?}");
        }
        client.send(b"\x15");
        settle(&mut client);
        assert_eq!(frame(&client), base, "{cols}x{rows} after clearing");
        for _ in 0..6 {
            press(&mut client, DOWN);
        }
        assert_eq!(frame(&client), base, "{cols}x{rows} after moving");
        client.send(ESC);
    }
}

#[test]
fn a_wide_screen_has_two_columns_a_narrower_one_has_one_and_the_narrowest_cuts() {
    let daemon = TestDaemon::start();
    // The author's usual window: two columns of groups, and the whole list without scrolling.
    let client = open_at(&daemon, 120, 36);
    let header = box_rows(&client)[2].clone();
    assert_eq!(header.matches("Action").count(), 2, "{header}");
    let text = box_rows(&client).join("\n");
    assert!(
        text.contains("redraw") && text.contains("focus left"),
        "{text}"
    );
    let (x, y, w, h) = frame(&client);
    // A margin is kept around the box.
    assert!(
        x >= 2 && y >= 1 && x + w <= 118 && y + h <= 35,
        "{x} {y} {w} {h}"
    );
    // One column that scrolls, with the heading of the group of the chosen row kept on top.
    let mut one = open_at(&daemon, 80, 24);
    let header = box_rows(&one)[2].clone();
    assert_eq!(header.matches("Action").count(), 1, "{header}");
    for _ in 0..30 {
        press(&mut one, DOWN);
    }
    let rows = body(&one);
    let chosen_row = chosen(&one).unwrap();
    assert!(
        HEADINGS.contains(&rows[0].trim()),
        "no heading on top: {rows:?}"
    );
    assert!(rows.iter().any(|r| r.contains(&chosen_row)));
    // Too narrow for the columns: the rows are cut with the ellipsis and the box stays in.
    let narrow = open_at(&daemon, 50, 20);
    let text = box_rows(&narrow).join("\n");
    assert!(text.contains('…'), "{text}");
    let (x, _, w, _) = frame(&narrow);
    assert!(x + w <= 50, "{x} {w}");
}

#[test]
fn enter_runs_the_chosen_action() {
    let daemon = TestDaemon::start();
    let mut client = open_at(&daemon, 120, 36);
    type_filter(&mut client, "copy.enter");
    client.send(b"\r");
    assert!(client.wait_until(WAIT, |c| c.status_row().contains("COPY")));
    assert!(box_rows(&client).is_empty());
    // An action chosen by moving.
    let daemon = TestDaemon::start();
    let mut client = open_at(&daemon, 120, 36);
    type_filter(&mut client, "window.new");
    client.send(b"\r");
    assert!(client.wait_until(WAIT, |c| c.status_row().contains(" 2 ")));
}

#[test]
fn a_key_this_terminal_cannot_send_is_marked_on_its_row() {
    let daemon = TestDaemon::start();
    let mut client = daemon
        .attach(AttachOptions::new().size(200, 70).named("legacy"))
        .unwrap();
    client.wait_for_prompt().unwrap();
    client.send(LEGACY_ANSWER);
    assert!(client.wait_until(WAIT, |c| c.status_row().contains("cannot work here")));
    open_list(&mut client);
    // Ctrl with a digit does not exist on the old key codes: the number keys are marked.
    let row = row_of(&client, "go to window N");
    assert!(row.contains("1-9!"), "{row}");
    // A key that works is not.
    let row = row_of(&client, "focus left");
    assert!(!row.contains('!'), "{row}");
    // The key line says what the mark means.
    let rows = box_rows(&client);
    assert!(
        inner(&rows[rows.len() - 2]).contains("! not on this terminal"),
        "{}",
        client.screen_text()
    );
}

#[test]
fn the_question_mark_opens_the_list_and_escape_closes_it_without_reaching_the_pane() {
    let daemon = TestDaemon::start();
    let mut client = open_at(&daemon, 120, 36);
    assert!(box_rows(&client)[0].contains("Keys"));
    client.send(ESC);
    assert!(client.wait_until(WAIT, |c| box_rows(c).is_empty()));
    client.type_line("echo after-$((20+22))");
    client.wait_for_text("after-42").unwrap();
}

#[test]
fn the_columns_of_the_rows_line_up_whatever_the_length_of_the_names_and_the_keys() {
    let daemon = TestDaemon::start();
    let client = open_at(&daemon, 200, 70);
    let at = |label: &str, keys: &str| {
        let row = row_of(&client, label);
        let start = row.find(label).unwrap();
        let after = start + label.len();
        (start, after + row[after..].find(keys).unwrap())
    };
    let short = at("zoom", "z");
    let long = at("resize right", "L, shift+right");
    assert_eq!(short.0, long.0, "the action column");
    assert_eq!(short.1, long.1, "the prefix column");
}
