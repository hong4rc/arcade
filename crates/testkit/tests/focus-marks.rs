//! Which pane has the focus can be seen on the dividers: the half of a divider on the focused
//! pane's side is drawn in the focus color, the other half in the plain divider color.

use std::time::Duration;

use arcade_testkit::{AttachOptions, Screened, TestClient, TestDaemon, WAIT};

fn run(daemon: &TestDaemon, name: &str, args: &[&str]) {
    let (ok, output) = daemon.command(name, args).unwrap();
    assert!(ok, "{name} {args:?}: {output}");
}

fn two_panes(vertical_divider: bool) -> (TestDaemon, TestClient) {
    let daemon = TestDaemon::start();
    let mut client = daemon.attach(AttachOptions::new().size(60, 21)).unwrap();
    client.wait_for_prompt().unwrap();
    let split = if vertical_divider {
        "pane.split-right"
    } else {
        "pane.split-down"
    };
    run(&daemon, split, &[]);
    // The divider is drawn once the screen has two panes.
    let symbol = if vertical_divider {
        "\u{2502}"
    } else {
        "\u{2500}"
    };
    assert!(client.wait_until(WAIT, |c| c.screen_text().contains(symbol)));
    std::thread::sleep(Duration::from_millis(300));
    (daemon, client)
}

/// The cells of the divider, in order: a column's rows, or a row's columns.
fn divider_cells(client: &TestClient, vertical_divider: bool) -> Vec<(u16, u16)> {
    let symbol = if vertical_divider {
        "\u{2502}"
    } else {
        "\u{2500}"
    };
    let rows = client.screen_rows();
    let mut cells = Vec::new();
    if vertical_divider {
        let col = rows
            .iter()
            .find_map(|r| r.chars().position(|c| c.to_string() == symbol))
            .expect("a column divider") as u16;
        for (y, row) in rows.iter().enumerate() {
            if row.chars().nth(usize::from(col)).map(|c| c.to_string()) == Some(symbol.into()) {
                cells.push((y as u16, col));
            }
        }
    } else {
        let (y, row) = rows
            .iter()
            .enumerate()
            .find(|(_, r)| r.contains(symbol))
            .expect("a row divider");
        for (x, c) in row.chars().enumerate() {
            if c.to_string() == symbol {
                cells.push((y as u16, x as u16));
            }
        }
    }
    cells
}

fn styles(client: &TestClient, cells: &[(u16, u16)]) -> Vec<arcade_term::Style> {
    cells.iter().map(|&(y, x)| client.style_at(y, x)).collect()
}

/// The first half of the cells is one style and the second half another.
fn halves(styles: &[arcade_term::Style]) -> (arcade_term::Style, arcade_term::Style) {
    let first = styles[0];
    let last = styles[styles.len() - 1];
    assert!(
        styles[..styles.len() / 2].iter().all(|s| *s == first),
        "{styles:?}"
    );
    assert!(
        styles[styles.len().div_ceil(2)..]
            .iter()
            .all(|s| *s == last),
        "{styles:?}"
    );
    (first, last)
}

#[test]
fn between_a_left_and_a_right_pane_the_focused_one_marks_its_half_of_the_divider() {
    let (daemon, mut client) = two_panes(true);
    let cells = divider_cells(&client, true);
    assert!(cells.len() > 8);
    // The new pane, on the right, has the focus: the lower half is marked.
    let (upper, lower) = halves(&styles(&client, &cells));
    assert_ne!(upper, lower);
    run(&daemon, "pane.focus-left", &[]);
    assert!(client.wait_until(WAIT, |c| styles(c, &cells)[0] == lower));
    let (now_upper, now_lower) = halves(&styles(&client, &cells));
    // Swapped: the upper half has what the lower half had, and the other way round.
    assert_eq!(now_upper, lower);
    assert_eq!(now_lower, upper);
}

#[test]
fn between_a_top_and_a_bottom_pane_the_focused_one_marks_its_half_of_the_divider() {
    let (daemon, mut client) = two_panes(false);
    let cells = divider_cells(&client, false);
    assert!(cells.len() > 8);
    // The new pane, below, has the focus: the right half is marked.
    let (left, right) = halves(&styles(&client, &cells));
    assert_ne!(left, right);
    run(&daemon, "pane.focus-up", &[]);
    assert!(client.wait_until(WAIT, |c| styles(c, &cells)[0] == right));
    let (now_left, now_right) = halves(&styles(&client, &cells));
    assert_eq!(now_left, right);
    assert_eq!(now_right, left);
}
