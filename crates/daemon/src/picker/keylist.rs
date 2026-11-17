//! The key list (the prefix, then `?`): one row for each action, in groups, with the keys that
//! run it after the prefix and the keys that run it with no prefix. This file builds the rows
//! from the key tables and draws the list: one column of groups, or two side by side when the
//! screen is wide enough and one column would have to scroll.
//!
//! The box has the size of the whole list for the screen it is on, so typing a filter blanks
//! rows and never moves or resizes the box.

use std::collections::BTreeMap;

use arcade_term::{Cell, Code, Key, Mods};

use super::{BoxSpec, Drawn, Item, Picker, assemble, cells_of, cut, fit_hint_line, geometry};
use crate::commands;
use crate::compose::{blank, text_cells};
use crate::config::TableName;
use crate::keymap::{Binding, Keymap, ListKeys, key_name, typed_name};
use crate::theme::{self, Role, Symbol};

/// The groups of actions, in the order they show. The last two hold the keys of copy mode and of
/// the lists, which show only while something is typed in the filter.
pub const GROUPS: [&str; 8] = [
    "Panes",
    "Windows",
    "Sessions",
    "Copy and paste",
    "Notifications",
    "Arcade",
    "Copy mode",
    "Lists",
];
const SHOWN_ALWAYS: usize = 6;

/// Room kept free at the sides and above and below the box.
const MARGIN: (usize, usize) = (2, 1);
/// Cells between the two columns of groups.
const GAP: usize = 3;

/// What a row of the key list says besides its action.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct KeyCells {
    pub group: usize,
    /// The keys after the prefix.
    pub prefix: String,
    /// The keys that need no prefix.
    pub direct: String,
    /// Some key of the row cannot work on this terminal.
    pub flagged: bool,
    /// The row shows only while the filter has text in it.
    pub filter_only: bool,
}

/// The column titles of the key list.
#[derive(Clone, Debug)]
pub struct KeyView {
    pub headers: [String; 3],
}

/// One action of the key list, and what choosing it does.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct KeyRow {
    pub label: String,
    pub binding: Binding,
    pub cells: KeyCells,
    /// Choosing the row runs `binding`; the keys of copy mode and of the lists have no meaning
    /// outside them.
    pub runnable: bool,
    /// The words the filter also finds the row by.
    pub search: String,
}

fn group_of(command: &str) -> usize {
    match command {
        "pane.paste" | "copy.enter" => 3,
        "tree.open" => 2,
        "keys.list" | "command.list" => 5,
        c if c.starts_with("pane.") => 0,
        c if c.starts_with("window.") => 1,
        c if c.starts_with("session.") => 2,
        c if c.starts_with("notification.") => 4,
        c if c.starts_with("copy.") => 3,
        _ => 5,
    }
}

/// The action in words.
pub fn label_of(command: &str, args: &[String]) -> String {
    let fixed = match command {
        "pane.focus-last" => "focus last",
        "pane.swap-mode" => "swap panes",
        "pane.split-right" => "split to the right",
        "pane.split-down" => "split downward",
        "pane.zoom" => "zoom",
        "pane.close" => "close pane",
        "pane.paste" => "paste",
        "pane.break" => "break to window",
        "pane.capture-keys" => "key capture (records typing)",
        "pane.move" => "move pane (tree)",
        "window.new" => "new window",
        "window.next" => "next window",
        "window.previous" => "previous window",
        "window.last" => "last window",
        "window.rename" => "rename window",
        "window.move-left" => "move window left",
        "window.move-right" => "move window right",
        "window.move-to-session" => "window to session",
        "window.move" => "move window (tree)",
        "window.list" => "window list",
        "window.layout-next" => "next layout",
        "window.layout-even" => "even layout",
        "window.select" => "go to window N",
        "session.list" => "session list",
        "session.new" => "new session",
        "session.rename" => "rename session",
        "session.detach" => "detach",
        "session.close" => "close session",
        "tree.open" => "tree of sessions",
        "copy.enter" => "copy mode",
        "notification.next" => "next notification",
        "notification.list" => "list notifications",
        "notification.clear" => "clear notification",
        "notification.clear-all" => "clear all",
        "keys.list" => "key list",
        "command.list" => "command list",
        "arcade.lock" => "lock",
        "arcade.reload-config" => "reload configuration",
        "arcade.redraw" => "redraw",
        _ => "",
    };
    if !fixed.is_empty() {
        return fixed.to_string();
    }
    for (prefix, words) in [
        ("pane.focus-", "focus"),
        ("pane.resize-", "resize"),
        ("pane.swap-", "swap with"),
    ] {
        if let Some(direction) = command.strip_prefix(prefix) {
            return format!("{words} {direction}");
        }
    }
    if args.is_empty() {
        command.to_string()
    } else {
        format!("{command} {}", args.join(" "))
    }
}

/// Keys in the order a person reads them: plain keys first, then those with modifiers, then the
/// named keys; each by name.
fn key_order(key: &Key) -> (bool, usize, String) {
    let mods = [
        key.mods.ctrl,
        key.mods.alt,
        key.mods.shift,
        key.mods.super_key,
    ]
    .iter()
    .filter(|m| **m)
    .count();
    (matches!(key.code, Code::Named(_)), mods, key_name(*key))
}

/// The keys as a cell of the list. A run of digit keys for the numbers of one command shows
/// as a range (`ctrl+alt+1-9`).
fn keys_text(mut keys: Vec<RowKey>, name: &dyn Fn(Key) -> String) -> (String, bool) {
    if keys.is_empty() {
        return (String::new(), false);
    }
    let flagged = keys.iter().any(|k| k.2);
    let mark = if flagged {
        theme::symbol(Symbol::KeyUnavailable)
    } else {
        ""
    };
    if keys.iter().all(|k| k.0.is_some()) && keys.len() >= 3 {
        keys.sort_by_key(|k| k.0);
        let digits_in_order = keys.iter().enumerate().all(|(i, (arg, key, _))| {
            let first = keys[0].1;
            *arg == keys[0].0.map(|a| a + i as u32)
                && key.mods == first.mods
                && matches!(key.code, Code::Char(c) if Some(c) == char::from_digit(arg.unwrap_or(99), 10))
        });
        if digits_in_order && let Code::Char(last) = keys[keys.len() - 1].1.code {
            return (format!("{}-{last}{mark}", name(keys[0].1)), flagged);
        }
    }
    let mut keys = keys;
    keys.sort_by_key(|k| key_order(&k.1));
    let names: Vec<String> = keys
        .iter()
        .map(|(_, key, bad)| {
            format!(
                "{}{}",
                name(*key),
                if *bad {
                    theme::symbol(Symbol::KeyUnavailable)
                } else {
                    ""
                }
            )
        })
        .collect();
    (names.join(", "), flagged)
}

#[derive(Default)]
struct Acc {
    binding: Option<Binding>,
    low: u32,
    prefix: Vec<RowKey>,
    direct: Vec<RowKey>,
}

/// The rows of the key list: one for each action that has a key after the prefix or with no
/// prefix, in groups; then, for the filter only, the keys of copy mode and of the lists.
/// `cannot_work` says whether a key of a table cannot work on the terminal.
///
/// Every key of the direct table carries the direct modifier, so it is left out of the cell
/// (the column title says it) when `direct` is given; a key with other modifiers is written whole.
pub fn rows(
    map: &Keymap,
    prefix: Key,
    direct: Option<Mods>,
    cannot_work: &dyn Fn(TableName, Key) -> bool,
) -> Vec<KeyRow> {
    let direct_name = |key: Key| -> String {
        let Some(m) = direct else {
            return typed_name(key);
        };
        let has = |want: bool, got: bool| !want || got;
        if !(has(m.ctrl, key.mods.ctrl)
            && has(m.alt, key.mods.alt)
            && has(m.shift, key.mods.shift)
            && has(m.super_key, key.mods.super_key))
        {
            return typed_name(key);
        }
        let rest = Mods {
            ctrl: key.mods.ctrl && !m.ctrl,
            alt: key.mods.alt && !m.alt,
            shift: key.mods.shift && !m.shift,
            super_key: key.mods.super_key && !m.super_key,
        };
        typed_name(Key {
            code: key.code,
            mods: rest,
        })
    };
    let prefix_cell_name = |key: Key| typed_name(key);
    let order: Vec<&str> = commands::all_names().collect();
    let position = |command: &str| {
        order
            .iter()
            .position(|n| *n == command)
            .unwrap_or(usize::MAX)
    };
    let mut actions: BTreeMap<(usize, usize, Vec<String>), Acc> = BTreeMap::new();
    for (table, name) in [
        (&map.prefix, TableName::Prefix),
        (&map.direct, TableName::Direct),
    ] {
        for (key, binding) in table {
            let number = (binding.command == "window.select")
                .then(|| binding.args.first().and_then(|a| a.parse::<u32>().ok()))
                .flatten();
            let args = if number.is_some() {
                Vec::new()
            } else {
                binding.args.clone()
            };
            let id = (
                group_of(&binding.command),
                position(&binding.command),
                args.iter()
                    .cloned()
                    .chain([binding.command.clone()])
                    .collect(),
            );
            let acc = actions.entry(id).or_default();
            if acc.binding.is_none() || number.is_some_and(|n| n < acc.low) {
                acc.binding = Some(binding.clone());
                acc.low = number.unwrap_or(0);
            }
            let item = (number, *key, cannot_work(name, *key));
            if name == TableName::Prefix {
                acc.prefix.push(item);
            } else {
                acc.direct.push(item);
            }
        }
    }
    let prefix_name = typed_name(prefix);
    let mut out = Vec::new();
    for ((group, _, _), acc) in actions {
        let Some(binding) = acc.binding.clone() else {
            continue;
        };
        let mut names: Vec<String> = acc
            .prefix
            .iter()
            .map(|(_, k, _)| format!("{prefix_name} {}", typed_name(*k)))
            .chain(acc.direct.iter().map(|(_, k, _)| typed_name(*k)))
            .collect();
        names.extend(
            acc.prefix
                .iter()
                .chain(&acc.direct)
                .map(|(_, k, _)| key_name(*k)),
        );
        let (prefix_cell, prefix_bad) = keys_text(acc.prefix, &prefix_cell_name);
        let (direct_cell, direct_bad) = keys_text(acc.direct, &direct_name);
        out.push(KeyRow {
            label: label_of(&binding.command, &binding.args),
            search: format!(
                "{} {} {}",
                binding.command,
                binding.args.join(" "),
                names.join(" ")
            ),
            binding,
            cells: KeyCells {
                group,
                prefix: prefix_cell,
                direct: direct_cell,
                flagged: prefix_bad || direct_bad,
                filter_only: false,
            },
            runnable: true,
        });
    }
    // The keys of copy mode and of the lists: found by typing, and not run from here.
    for (table, name, group) in [
        (&map.copy, TableName::Copy, 6),
        (&map.list, TableName::List, 7),
    ] {
        let mut by_action: BTreeMap<(usize, Vec<String>), Vec<RowKey>> = BTreeMap::new();
        let mut bindings: BTreeMap<(usize, Vec<String>), Binding> = BTreeMap::new();
        for (key, binding) in table {
            let id = (
                position(&binding.command),
                [binding.command.clone()]
                    .into_iter()
                    .chain(binding.args.clone())
                    .collect::<Vec<_>>(),
            );
            by_action
                .entry(id.clone())
                .or_default()
                .push((None, *key, cannot_work(name, *key)));
            bindings.insert(id, binding.clone());
        }
        for (id, keys) in by_action {
            let binding = bindings[&id].clone();
            let names: Vec<String> = keys.iter().map(|(_, k, _)| key_name(*k)).collect();
            let (cell, bad) = keys_text(keys, &prefix_cell_name);
            out.push(KeyRow {
                label: label_of(&binding.command, &binding.args),
                search: format!("{} {}", binding.command, names.join(" ")),
                binding,
                cells: KeyCells {
                    group,
                    prefix: cell,
                    direct: String::new(),
                    flagged: bad,
                    filter_only: true,
                },
                runnable: false,
            });
        }
    }
    out
}

/// The items of the picker for these rows; the item's id is the row's position.
pub fn items(rows: &[KeyRow]) -> Vec<Item> {
    rows.iter()
        .enumerate()
        .map(|(id, row)| {
            Item::new(id, row.label.clone(), String::new(), false)
                .also_found_by(&row.search)
                .with_keys(row.cells.clone())
        })
        .collect()
}

/// A key of a row: the number it stands for (when it selects by number), the key, and whether
/// this terminal cannot send it.
type RowKey = (Option<u32>, Key, bool);

/// What a line of a column is.
#[derive(Clone, Copy)]
enum Line {
    Heading(usize),
    Blank(usize),
    /// The position among the lines that show, and the group.
    Row(usize, usize),
    NoMatch,
}

/// How tall a column is that holds these numbers of rows of each group: a heading above the
/// rows of each group present, and a blank line between groups.
fn column_height(rows_of_group: &[usize]) -> usize {
    let present = rows_of_group.iter().filter(|n| **n > 0).count();
    rows_of_group.iter().sum::<usize>() + present + present.saturating_sub(1)
}

/// How many of the first rows go to the first column when there are two: the number that leaves
/// the taller of the two columns the shortest. A group may continue in the second column.
fn split(groups: &[usize]) -> usize {
    let counts = |rows: &[usize]| {
        let mut per = [0usize; SHOWN_ALWAYS];
        for g in rows {
            per[*g] += 1;
        }
        per
    };
    (1..groups.len())
        .min_by_key(|k| {
            column_height(&counts(&groups[..*k])).max(column_height(&counts(&groups[*k..])))
        })
        .unwrap_or(groups.len())
}

fn pad(text: &str, width: usize) -> String {
    let have = cells_of(text);
    format!("{text}{}", " ".repeat(width.saturating_sub(have)))
}

/// The footer: the keys of the list, and what a mark on a key means when some key has one.
fn footer(picker: &Picker, keys: &ListKeys, filtered: bool, flagged: bool, width: usize) -> String {
    let mut items = picker.key_items(keys, filtered);
    if flagged {
        items.push((
            Some(format!(
                "{} not on this terminal",
                theme::symbol(Symbol::KeyUnavailable)
            )),
            8,
        ));
    }
    fit_hint_line(items, width)
}

fn probe_spec<'a>(
    picker: &'a Picker,
    list_rows: usize,
    widest: usize,
    footers: &'a [String],
    cols: u16,
    rows: u16,
) -> BoxSpec<'a> {
    BoxSpec {
        title: &picker.title,
        filter: &picker.filter,
        lines: &[],
        numbers: &[],
        selected: None,
        footers,
        widest,
        list_rows,
        margin: MARGIN,
        cols,
        rows,
    }
}

pub(super) fn draw(picker: &Picker, cols: u16, rows: u16, keys: &ListKeys) -> Drawn {
    let headers = picker
        .key_view
        .as_ref()
        .map(|v| v.headers.clone())
        .unwrap_or_default();
    let cells_of_item = |item: &Item| {
        item.keys.clone().unwrap_or_else(|| KeyCells {
            group: 0,
            prefix: String::new(),
            direct: String::new(),
            flagged: false,
            filter_only: false,
        })
    };
    let always: Vec<&Item> = picker
        .items
        .iter()
        .filter(|i| !cells_of_item(i).filter_only)
        .collect();
    let flagged = picker.items.iter().any(|i| cells_of_item(i).flagged);

    // The widths of the three columns, from every row that always shows.
    let widest_of = |header: &str, text: &dyn Fn(&Item) -> String| {
        always
            .iter()
            .map(|i| cells_of(&text(i)))
            .chain([cells_of(header)])
            .max()
            .unwrap_or(0)
    };
    let label_width = widest_of(&headers[0], &|i| i.name.clone());
    let prefix_width = widest_of(&headers[1], &|i| cells_of_item(i).prefix);
    let direct_width = widest_of(&headers[2], &|i| cells_of_item(i).direct);
    let mark_width = cells_of(theme::symbol(Symbol::PickerOtherMark));
    let cell_width = mark_width + label_width + 2 + prefix_width + 2 + direct_width;

    // The groups of the rows that always show, in order, and how tall one column of them is.
    let base_groups: Vec<usize> = always.iter().map(|i| cells_of_item(i).group).collect();
    let mut per_group = [0usize; SHOWN_ALWAYS];
    for g in &base_groups {
        per_group[*g] += 1;
    }
    let one_column = column_height(&per_group);

    let footers_for = |filtered: bool| [footer(picker, keys, filtered, flagged, usize::MAX)];
    let room = geometry(&probe_spec(
        picker,
        10_000,
        0,
        &footers_for(false),
        cols,
        rows,
    ))
    .shown;
    let body_room = room.saturating_sub(1);
    let most = usize::from(cols).saturating_sub(2 + 2 * MARGIN.0).max(1);
    let two = one_column > body_room && base_groups.len() >= 2 && 2 * cell_width + GAP + 2 <= most;
    let first_column = if two {
        split(&base_groups)
    } else {
        base_groups.len()
    };
    let columns = if two { 2 } else { 1 };
    // The column of each row, fixed by the whole list so that filtering never moves a row.
    let rank_of: Vec<Option<usize>> = {
        let mut next = 0;
        picker
            .items
            .iter()
            .map(|i| {
                if cells_of_item(i).filter_only {
                    None
                } else {
                    next += 1;
                    Some(next - 1)
                }
            })
            .collect()
    };
    let column_of = |item: usize| -> usize {
        match rank_of[item] {
            Some(rank) if rank < first_column => 0,
            _ => columns - 1,
        }
    };
    let mut column_rows = [[0usize; SHOWN_ALWAYS]; 2];
    for (rank, g) in base_groups.iter().enumerate() {
        column_rows[usize::from(rank >= first_column && two)][*g] += 1;
    }
    let tallest = column_height(&column_rows[0])
        .max(column_height(&column_rows[1]))
        .max(1);
    let widest = (columns * cell_width + (columns - 1) * GAP)
        .max(cells_of(&picker.title) + 2)
        .max(cells_of(&footers_for(false)[0]))
        .max(cells_of(&footers_for(true)[0]));

    // The lines of each column as they are now: only the groups with a match.
    let mut lines: [Vec<Line>; 2] = [Vec::new(), Vec::new()];
    if picker.visible.is_empty() {
        lines[0].push(Line::NoMatch);
    }
    for (c, column_lines) in lines.iter_mut().enumerate().take(columns) {
        for group in 0..GROUPS.len() {
            let ours: Vec<usize> = picker
                .visible
                .iter()
                .enumerate()
                .filter(|(_, i)| {
                    cells_of_item(&picker.items[**i]).group == group && column_of(**i) == c
                })
                .map(|(pos, _)| pos)
                .collect();
            if ours.is_empty() {
                continue;
            }
            if !column_lines.is_empty() {
                column_lines.push(Line::Blank(group));
            }
            column_lines.push(Line::Heading(group));
            column_lines.extend(ours.into_iter().map(|pos| Line::Row(pos, group)));
        }
    }

    let filtered = !picker.filter.is_empty();
    let footers = footers_for(filtered);
    let geo = geometry(&probe_spec(
        picker,
        1 + tallest,
        widest,
        &footers,
        cols,
        rows,
    ));
    // When the box is narrower than the key line, the hints that matter least are left out, so
    // that closing and what the mark means stay.
    let footers = [footer(
        picker,
        keys,
        filtered,
        flagged,
        geo.inner.saturating_sub(1),
    )];
    let shown_lines = geo.shown.saturating_sub(1);

    // Which line of which column is chosen, and the first line that shows: the chosen one stays
    // in sight, and under a first line that is not a heading, the heading of its group.
    let at = (0..columns)
        .find_map(|c| {
            lines[c]
                .iter()
                .position(|l| matches!(l, Line::Row(pos, _) if *pos == picker.selected))
                .map(|l| (c, l))
        })
        .unwrap_or((0, 0));
    let mut first = (at.1 + 1).saturating_sub(shown_lines);
    if first > 0 && at.1 <= first {
        first = at.1.saturating_sub(1);
    }

    let plain = theme::style(Role::Picker);
    let heading = theme::style(Role::PickerHeading);
    let chosen = theme::style(Role::PickerSelected);
    let render = |c: usize, line: Option<Line>| -> Vec<Cell> {
        let cells = match line {
            None | Some(Line::Blank(_)) => text_cells("", plain),
            Some(Line::NoMatch) => text_cells(
                &format!("  {}", theme::symbol(Symbol::PickerNoMatch)),
                plain,
            ),
            Some(Line::Heading(g)) => {
                // A group that goes on from the first column says so.
                let goes_on = c == 1
                    && lines[0]
                        .iter()
                        .any(|l| matches!(l, Line::Heading(h) if *h == g));
                let text = if goes_on {
                    format!("{} (continued)", GROUPS[g])
                } else {
                    GROUPS[g].to_string()
                };
                text_cells(&text, heading)
            }
            Some(Line::Row(pos, _)) => {
                let item = &picker.items[picker.visible[pos]];
                let k = cells_of_item(item);
                let selected = pos == picker.selected;
                let mark = theme::symbol(if selected {
                    Symbol::PickerSelectedMark
                } else {
                    Symbol::PickerOtherMark
                });
                let text = format!(
                    "{mark}{}  {}  {}",
                    pad(&item.name, label_width),
                    pad(&k.prefix, prefix_width),
                    k.direct
                );
                let style = if selected { chosen } else { plain };
                let mut cells = text_cells(&text, style);
                cells = cut(cells, cell_width, style);
                return cells;
            }
        };
        cut(cells, cell_width, plain)
    };
    let header_cells = || {
        let text = format!(
            "{}{}  {}  {}",
            " ".repeat(mark_width),
            pad(&headers[0], label_width),
            pad(&headers[1], prefix_width),
            headers[2]
        );
        cut(text_cells(&text, heading), cell_width, heading)
    };
    let line_at = |c: usize, i: usize| -> Option<Line> {
        let line = lines[c].get(first + i).copied()?;
        if i == 0 && first > 0 && !matches!(line, Line::Heading(_)) {
            let group = match line {
                Line::Row(_, g) | Line::Blank(g) => g,
                other => return Some(other),
            };
            return Some(Line::Heading(group));
        }
        Some(line)
    };

    let mut body: Vec<(Vec<Cell>, arcade_term::Style)> = Vec::new();
    let join = |left: Vec<Cell>, right: Option<Vec<Cell>>| {
        let mut row = vec![blank(plain)];
        row.extend(left);
        if let Some(right) = right {
            row.extend((0..GAP).map(|_| blank(plain)));
            row.extend(right);
        }
        (row, plain)
    };
    body.push(join(header_cells(), two.then(header_cells)));
    for i in 0..shown_lines {
        let left = render(0, line_at(0, i));
        let right = two.then(|| render(1, line_at(1, i)));
        body.push(join(left, right));
    }
    assemble(&geo, &picker.title, &picker.filter, body, &footers)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::keymap::KeysConfig;

    fn default_rows() -> Vec<KeyRow> {
        let config = KeysConfig::default();
        let map = Keymap::defaults(&config);
        rows(&map, config.prefix, config.direct_modifier, &|_, _| false)
    }

    fn row<'a>(rows: &'a [KeyRow], label: &str) -> &'a KeyRow {
        rows.iter()
            .find(|r| r.label == label)
            .unwrap_or_else(|| panic!("no row {label}"))
    }

    #[test]
    fn one_row_for_each_action_with_all_its_keys() {
        let rows = default_rows();
        let focus = row(&rows, "focus left");
        assert_eq!(focus.cells.prefix, "h, left");
        // The direct modifier is in the column title, not in each key.
        assert_eq!(focus.cells.direct, "h, left");
        let mut labels: Vec<&str> = rows
            .iter()
            .filter(|r| !r.cells.filter_only)
            .map(|r| r.label.as_str())
            .collect();
        let count = labels.len();
        labels.sort_unstable();
        labels.dedup();
        assert_eq!(labels.len(), count, "an action has two rows");
    }

    #[test]
    fn the_numbers_of_the_windows_make_one_row_with_a_range() {
        let rows = default_rows();
        let go = row(&rows, "go to window N");
        assert_eq!(go.cells.prefix, "1-9");
        assert_eq!(go.cells.direct, "1-9");
        assert_eq!(go.binding.args, ["1"]);
    }

    #[test]
    fn the_groups_come_in_order_and_copy_mode_waits_for_a_filter() {
        let rows = default_rows();
        let groups: Vec<usize> = rows.iter().map(|r| r.cells.group).collect();
        assert!(groups.windows(2).all(|w| w[0] <= w[1]), "{groups:?}");
        assert!(
            rows.iter()
                .filter(|r| r.cells.group >= SHOWN_ALWAYS)
                .all(|r| r.cells.filter_only && !r.runnable)
        );
        assert!(
            rows.iter()
                .filter(|r| r.cells.group < SHOWN_ALWAYS)
                .all(|r| r.runnable)
        );
    }

    #[test]
    fn keys_that_cannot_work_are_marked() {
        let config = KeysConfig::default();
        let map = Keymap::defaults(&config);
        let rows = rows(
            &map,
            config.prefix,
            config.direct_modifier,
            &|table, key| table == TableName::Prefix && key.code == Code::Char('z'),
        );
        let zoom = row(&rows, "zoom");
        assert!(zoom.cells.flagged);
        assert_eq!(zoom.cells.prefix, "z!");
        assert!(!row(&rows, "focus left").cells.flagged);
    }

    #[test]
    fn with_no_direct_modifier_there_are_no_direct_keys() {
        let config = KeysConfig {
            direct_modifier: None,
            ..KeysConfig::default()
        };
        let map = Keymap::defaults(&config);
        let rows = rows(&map, config.prefix, None, &|_, _| false);
        // With no direct modifier there is no direct table to show.
        assert!(
            rows.iter()
                .all(|r| r.cells.direct.is_empty() || r.cells.filter_only)
        );
    }

    #[test]
    fn two_columns_split_the_rows_so_that_the_taller_is_the_shortest() {
        // Rows of groups 0, 0, 0, 1, 1: one group split across the columns is allowed.
        let groups = [0, 0, 0, 1, 1];
        assert_eq!(split(&groups), 3);
        // Five rows, two headings and a blank line between the groups.
        assert_eq!(column_height(&[3, 2, 0, 0, 0, 0]), 8);
        assert_eq!(column_height(&[0, 0, 0, 0, 0, 0]), 0);
    }
}
