//! The tree of sessions, windows and panes: what is on it, where the choice is, what is lifted,
//! and what pressing Enter would do. Nothing here touches a session or a process: the daemon
//! makes the [`Entry`] lines from its own books, and does what a [`Plan`] says.

use crate::picker::{Drawn, draw_box};
use crate::theme::{self, Symbol};

/// What an item of the tree is, by the daemon's own number for it (unique over the daemon).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Key {
    Session(u64),
    Window(u64),
    Pane(u64),
}

impl Key {
    fn same_tier(self, other: Key) -> bool {
        std::mem::discriminant(&self) == std::mem::discriminant(&other)
    }
}

/// One item, with what the tree needs to know about it. The entries come in the order of the
/// tree: a session, its windows each followed by its panes, then the next session.
#[derive(Clone, Debug)]
pub struct Entry {
    pub key: Key,
    /// What the line says: the session's name, the window's number and name, the pane's title.
    pub label: String,
    /// Position of the entry this one is inside.
    pub parent: Option<usize>,
    /// The client is in it.
    pub here: bool,
    /// Where Right goes: the window a session shows, the pane a window has the focus in.
    pub inner: Option<Key>,
    /// A pane: there is room beside it to put another pane.
    pub room: bool,
}

/// What Enter does.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Action {
    /// Go to the item.
    Jump(Key),
    /// Split the target pane and put the pane in the new right half.
    PaneBeside { pane: u64, target: u64 },
    /// The pane becomes a new window at the end of the session.
    PaneToSession { pane: u64, session: u64 },
    /// The window stands right after the target window.
    WindowAfter { window: u64, target: u64 },
    /// The window becomes the last of the session.
    WindowToSession { window: u64, session: u64 },
}

pub struct Tree {
    pub title: String,
    entries: Vec<Entry>,
    /// The items that show what is inside them.
    open: Vec<Key>,
    selected: Key,
    lifted: Option<Key>,
    filter: String,
    /// A line for the user, shown in place of what Enter does until the next key.
    pub notice: Option<String>,
}

impl Tree {
    /// A tree with the branch of the client's place open and the choice on `select`.
    pub fn new(entries: Vec<Entry>, select: Key, lifted: Option<Key>) -> Tree {
        let mut tree = Tree {
            title: "Tree".to_string(),
            entries,
            open: Vec::new(),
            selected: select,
            lifted,
            filter: String::new(),
            notice: None,
        };
        let here: Vec<usize> = (0..tree.entries.len())
            .filter(|i| tree.entries[*i].here && matches!(tree.entries[*i].key, Key::Pane(_)))
            .collect();
        for i in here {
            tree.reveal(i);
        }
        if let Some(i) = tree.index_of(select) {
            tree.reveal(i);
        } else if let Some(first) = tree.entries.first() {
            tree.selected = first.key;
        }
        tree
    }

    fn index_of(&self, key: Key) -> Option<usize> {
        self.entries.iter().position(|e| e.key == key)
    }

    fn children(&self, index: usize) -> impl Iterator<Item = usize> + '_ {
        (0..self.entries.len()).filter(move |i| self.entries[*i].parent == Some(index))
    }

    /// Open every item that holds the entry, so that it shows.
    fn reveal(&mut self, index: usize) {
        let mut at = self.entries[index].parent;
        while let Some(i) = at {
            let key = self.entries[i].key;
            if !self.open.contains(&key) {
                self.open.push(key);
            }
            at = self.entries[i].parent;
        }
    }

    #[cfg_attr(not(test), allow(dead_code))] // tests look at where the choice is
    pub fn selected_key(&self) -> Key {
        self.selected
    }

    #[cfg_attr(not(test), allow(dead_code))] // tests look at what is lifted
    pub fn lifted_key(&self) -> Option<Key> {
        self.lifted
    }

    fn depth(&self, index: usize) -> usize {
        let mut depth = 0;
        let mut at = self.entries[index].parent;
        while let Some(i) = at {
            depth += 1;
            at = self.entries[i].parent;
        }
        depth
    }

    /// The path of names from the session down to the entry.
    fn path(&self, index: usize) -> Vec<&str> {
        let mut names = vec![self.entries[index].label.as_str()];
        let mut at = self.entries[index].parent;
        while let Some(i) = at {
            names.push(self.entries[i].label.as_str());
            at = self.entries[i].parent;
        }
        names.reverse();
        names
    }

    /// The entries that show, in order: those with every item around them open, or, while there
    /// is a filter, those whose name has the filter in it, whatever tier they are on.
    fn rows(&self) -> Vec<usize> {
        if self.filter.is_empty() {
            return (0..self.entries.len())
                .filter(|i| {
                    let mut at = self.entries[*i].parent;
                    while let Some(p) = at {
                        if !self.open.contains(&self.entries[p].key) {
                            return false;
                        }
                        at = self.entries[p].parent;
                    }
                    true
                })
                .collect();
        }
        let wanted = self.filter.to_lowercase();
        (0..self.entries.len())
            .filter(|i| self.entries[*i].label.to_lowercase().contains(&wanted))
            .collect()
    }

    fn selected_row(&self, rows: &[usize]) -> Option<usize> {
        rows.iter()
            .position(|i| self.entries[*i].key == self.selected)
    }

    /// The choice goes to the first line that shows, when the one it was on does not.
    fn settle(&mut self) {
        let rows = self.rows();
        if self.selected_row(&rows).is_none()
            && let Some(first) = rows.first()
        {
            self.selected = self.entries[*first].key;
        }
    }

    fn go_to(&mut self, index: usize) {
        self.selected = self.entries[index].key;
        self.reveal(index);
    }

    /// The previous item of the same tier, going on into the item before the parent.
    pub fn up(&mut self) {
        self.step(-1);
    }

    pub fn down(&mut self) {
        self.step(1);
    }

    fn step(&mut self, by: isize) {
        let rows = self.rows();
        let Some(index) = self.index_of(self.selected) else {
            return;
        };
        if !self.filter.is_empty() {
            let Some(at) = self.selected_row(&rows) else {
                return;
            };
            if let Some(next) = at.checked_add_signed(by).and_then(|n| rows.get(n)) {
                self.go_to(*next);
            }
            return;
        }
        let tier: Vec<usize> = (0..self.entries.len())
            .filter(|i| self.entries[*i].key.same_tier(self.selected))
            .collect();
        let at = tier.iter().position(|i| *i == index).unwrap_or(0);
        if let Some(next) = at.checked_add_signed(by).and_then(|n| tier.get(n)) {
            self.go_to(*next);
        }
    }

    /// Into the item: the window a session shows, the pane a window has the focus in.
    pub fn right(&mut self) {
        if !self.filter.is_empty() {
            return;
        }
        let Some(index) = self.index_of(self.selected) else {
            return;
        };
        let Some(inner) = self.entries[index].inner.and_then(|k| self.index_of(k)) else {
            return;
        };
        if !self.open.contains(&self.selected) {
            self.open.push(self.selected);
        }
        self.go_to(inner);
    }

    /// Out to the item this one is inside.
    pub fn left(&mut self) {
        if !self.filter.is_empty() {
            return;
        }
        let Some(index) = self.index_of(self.selected) else {
            return;
        };
        if let Some(parent) = self.entries[index].parent {
            self.go_to(parent);
        }
    }

    /// Lift the chosen pane or window, or put it down again.
    pub fn toggle_lift(&mut self) {
        match self.selected {
            Key::Session(_) => {
                self.notice = Some("a session cannot be lifted".to_string());
            }
            key if self.lifted == Some(key) => self.lifted = None,
            key => self.lifted = Some(key),
        }
    }

    /// Put down what is lifted; `false` when nothing was, and the tree should close.
    pub fn put_down(&mut self) -> bool {
        self.lifted.take().is_some()
    }

    pub fn type_text(&mut self, text: &str) {
        self.filter.push_str(text);
        self.settle();
    }

    pub fn delete_char(&mut self) {
        self.filter.pop();
        self.settle();
    }

    pub fn clear_filter(&mut self) {
        self.filter.clear();
        self.settle();
    }

    /// Take the daemon's new account of what exists. What is gone is dropped from the tree: a
    /// lifted item that is gone is put down (and the notice says so), and the choice goes to the
    /// item now where the old one was.
    pub fn refresh(&mut self, entries: Vec<Entry>) {
        let old_at = self.index_of(self.selected).unwrap_or(0);
        self.entries = entries;
        self.open
            .retain(|k| self.entries.iter().any(|e| e.key == *k));
        if let Some(lifted) = self.lifted
            && self.index_of(lifted).is_none()
        {
            self.lifted = None;
            self.notice = Some("the lifted item is gone".to_string());
        }
        if self.index_of(self.selected).is_none()
            && let Some(near) = self
                .entries
                .get(old_at.min(self.entries.len().saturating_sub(1)))
        {
            self.selected = near.key;
        }
        if let Some(index) = self.index_of(self.selected) {
            self.reveal(index);
        }
        self.settle();
    }

    // ---- What Enter does ----

    fn describe(&self, index: usize) -> String {
        let e = &self.entries[index];
        match e.key {
            Key::Session(_) => format!("session {}", e.label),
            Key::Window(_) => match e.parent {
                Some(s) => format!("window {} of {}", e.label, self.entries[s].label),
                None => format!("window {}", e.label),
            },
            Key::Pane(_) => format!("pane {}", e.label),
        }
    }

    fn window_of(&self, index: usize) -> Option<usize> {
        match self.entries[index].key {
            Key::Window(_) => Some(index),
            Key::Pane(_) => self.entries[index].parent,
            Key::Session(_) => None,
        }
    }

    fn session_of(&self, index: usize) -> usize {
        let mut at = index;
        while let Some(p) = self.entries[at].parent {
            at = p;
        }
        at
    }

    /// What Enter would do with the chosen item, in words, or why it would do nothing.
    pub fn plan(&self) -> Result<(Action, String), String> {
        let at = self
            .index_of(self.selected)
            .ok_or_else(|| "nothing is chosen".to_string())?;
        let target = &self.entries[at];
        let Some(lifted) = self.lifted.and_then(|k| self.index_of(k)) else {
            let what = format!("go to {}", self.describe(at));
            return Ok((Action::Jump(target.key), what));
        };
        let from = &self.entries[lifted];
        if lifted == at {
            return Err(format!(
                "this is the {} that is lifted",
                tier_name(from.key)
            ));
        }
        let here = self.describe(lifted);
        match (from.key, target.key) {
            (Key::Pane(pane), Key::Pane(t)) => {
                let same_window = self.window_of(lifted) == self.window_of(at);
                if !same_window && !target.room {
                    return Err("no room in that window to split".to_string());
                }
                let what = format!("move {here} next to {}", self.describe_in_window(at));
                Ok((Action::PaneBeside { pane, target: t }, what))
            }
            (Key::Pane(pane), Key::Window(_)) => {
                if self.window_of(lifted) == Some(at) {
                    return Err("the pane is in that window already".to_string());
                }
                let Some(Key::Pane(t)) = target.inner else {
                    return Err("that window has no pane".to_string());
                };
                let focus = self.index_of(Key::Pane(t));
                if !focus.is_some_and(|f| self.entries[f].room) {
                    return Err("no room in that window to split".to_string());
                }
                let what = format!("move {here} to {}", self.describe(at));
                Ok((Action::PaneBeside { pane, target: t }, what))
            }
            (Key::Pane(pane), Key::Session(session)) => {
                let own_window = self.window_of(lifted);
                let alone = own_window.is_some_and(|w| self.children(w).count() == 1);
                if self.session_of(lifted) == at && alone {
                    return Err("the pane is alone in its window".to_string());
                }
                let what = format!("move {here} to a new window of {}", self.describe(at));
                Ok((Action::PaneToSession { pane, session }, what))
            }
            (Key::Window(window), Key::Window(t)) => {
                let what = format!("move {here} after {}", self.describe(at));
                Ok((Action::WindowAfter { window, target: t }, what))
            }
            (Key::Window(_), Key::Pane(_)) => {
                Err("a window is put on a window or a session".to_string())
            }
            (Key::Window(window), Key::Session(session)) => {
                let last = self
                    .children(at)
                    .last()
                    .is_some_and(|c| self.entries[c].key == Key::Window(window));
                if last {
                    return Err("the window is the last of its session already".to_string());
                }
                let what = format!("move {here} to the end of {}", self.describe(at));
                Ok((Action::WindowToSession { window, session }, what))
            }
            (Key::Session(_), _) => Err("a session cannot be moved".to_string()),
        }
    }

    fn describe_in_window(&self, index: usize) -> String {
        match self.entries[index].parent {
            Some(w) => format!("{} in {}", self.describe(index), self.describe(w)),
            None => self.describe(index),
        }
    }

    /// The line at the bottom of the box.
    fn footer(&self) -> String {
        if let Some(notice) = &self.notice {
            return notice.clone();
        }
        match self.plan() {
            Ok((_, what)) => format!("Enter: {what}"),
            Err(why) => format!("Enter does nothing: {why}"),
        }
    }

    // ---- Drawing ----

    fn line(&self, index: usize, selected: bool, flat: bool) -> String {
        let e = &self.entries[index];
        let mark = theme::symbol(if selected {
            Symbol::PickerSelectedMark
        } else {
            Symbol::PickerOtherMark
        });
        let (indent, opener) = if flat {
            (String::new(), "")
        } else {
            let indent = "    ".repeat(self.depth(index));
            let opener = match e.key {
                Key::Pane(_) => theme::symbol(if e.here {
                    Symbol::TreeHere
                } else {
                    Symbol::TreeOtherPane
                }),
                _ if self.open.contains(&e.key) => theme::symbol(Symbol::TreeOpen),
                _ => theme::symbol(Symbol::TreeClosed),
            };
            (indent, opener)
        };
        let name = if flat {
            self.path(index)
                .join(theme::symbol(Symbol::TreePathSeparator))
        } else {
            e.label.clone()
        };
        let here = if e.here && !matches!(e.key, Key::Pane(_)) {
            theme::symbol(Symbol::PickerCurrentMark)
        } else {
            ""
        };
        let lifted = if self.lifted == Some(e.key) {
            theme::symbol(Symbol::TreeLifted)
        } else {
            ""
        };
        format!("{mark}{indent}{opener}{name}{here}{lifted}")
    }

    pub fn draw(&self, cols: u16, rows: u16) -> Drawn {
        let shown = self.rows();
        let flat = !self.filter.is_empty();
        let selected = self.selected_row(&shown);
        let mut lines: Vec<String> = shown
            .iter()
            .enumerate()
            .map(|(row, index)| self.line(*index, Some(row) == selected, flat))
            .collect();
        if shown.is_empty() {
            lines.push(format!("  {}", theme::symbol(Symbol::PickerNoMatch)));
        }
        draw_box(
            &self.title,
            &self.filter,
            &lines,
            selected,
            Some(&self.footer()),
            cols,
            rows,
        )
    }
}

fn tier_name(key: Key) -> &'static str {
    match key {
        Key::Session(_) => "session",
        Key::Window(_) => "window",
        Key::Pane(_) => "pane",
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// work: window 1 "editor" (panes nvim, zsh), window 2 "server" (pane htop);
    /// notes: window 1 "todo" (pane vim).
    fn entries() -> Vec<Entry> {
        let e = |key, label: &str, parent, here, inner, room| Entry {
            key,
            label: label.to_string(),
            parent,
            here,
            inner,
            room,
        };
        vec![
            e(
                Key::Session(1),
                "work",
                None,
                true,
                Some(Key::Window(11)),
                false,
            ),
            e(
                Key::Window(11),
                "1 editor",
                Some(0),
                true,
                Some(Key::Pane(101)),
                false,
            ),
            e(Key::Pane(101), "nvim", Some(1), true, None, true),
            e(Key::Pane(102), "zsh", Some(1), false, None, true),
            e(
                Key::Window(12),
                "2 server",
                Some(0),
                false,
                Some(Key::Pane(103)),
                false,
            ),
            e(Key::Pane(103), "htop", Some(4), false, None, true),
            e(
                Key::Session(2),
                "notes",
                None,
                false,
                Some(Key::Window(21)),
                false,
            ),
            e(
                Key::Window(21),
                "1 todo",
                Some(6),
                false,
                Some(Key::Pane(104)),
                false,
            ),
            e(Key::Pane(104), "vim", Some(7), false, None, false),
        ]
    }

    fn tree() -> Tree {
        Tree::new(entries(), Key::Pane(101), None)
    }

    fn lines(tree: &Tree) -> Vec<String> {
        let shown = tree.rows();
        let flat = !tree.filter.is_empty();
        let selected = tree.selected_row(&shown);
        shown
            .iter()
            .enumerate()
            .map(|(row, i)| tree.line(*i, Some(row) == selected, flat))
            .collect()
    }

    #[test]
    fn the_branch_of_the_client_is_open_and_the_others_are_closed() {
        let t = tree();
        let spaces = |n: usize| " ".repeat(n);
        let (open, closed) = (
            theme::symbol(Symbol::TreeOpen),
            theme::symbol(Symbol::TreeClosed),
        );
        let here = theme::symbol(Symbol::TreeHere);
        assert_eq!(
            lines(&t),
            vec![
                format!("  {open}work *"),
                format!("{}{open}1 editor *", spaces(6)),
                format!("> {}{here}nvim", spaces(8)),
                format!("{}zsh", spaces(12)),
                format!("{}{closed}2 server", spaces(6)),
                format!("  {closed}notes"),
            ]
        );
    }

    #[test]
    fn up_and_down_stay_on_their_tier_across_parents_and_open_what_they_reach() {
        let mut t = tree();
        t.down();
        assert_eq!(t.selected_key(), Key::Pane(102));
        t.down();
        // From the last pane of window 1 to the first of window 2, which opens.
        assert_eq!(t.selected_key(), Key::Pane(103));
        assert!(t.rows().iter().any(|i| t.entries[*i].key == Key::Pane(103)));
        t.up();
        t.up();
        assert_eq!(t.selected_key(), Key::Pane(101));
        // The first of the tier: it stays.
        t.up();
        assert_eq!(t.selected_key(), Key::Pane(101));
        // The tier of the windows.
        t.left();
        assert_eq!(t.selected_key(), Key::Window(11));
        t.down();
        assert_eq!(t.selected_key(), Key::Window(12));
        t.down();
        assert_eq!(t.selected_key(), Key::Window(21));
        t.down();
        assert_eq!(t.selected_key(), Key::Window(21));
    }

    #[test]
    fn left_goes_out_and_right_goes_in() {
        let mut t = tree();
        t.left();
        assert_eq!(t.selected_key(), Key::Window(11));
        t.left();
        assert_eq!(t.selected_key(), Key::Session(1));
        t.left();
        assert_eq!(t.selected_key(), Key::Session(1));
        t.right();
        assert_eq!(t.selected_key(), Key::Window(11));
        t.right();
        assert_eq!(t.selected_key(), Key::Pane(101));
        t.right();
        assert_eq!(t.selected_key(), Key::Pane(101));
        // Into another session: the window it shows.
        t.left();
        t.left();
        t.down();
        assert_eq!(t.selected_key(), Key::Session(2));
        t.right();
        assert_eq!(t.selected_key(), Key::Window(21));
    }

    #[test]
    fn a_filter_shows_the_matching_lines_flat_with_their_path() {
        let mut t = tree();
        t.type_text("VIM");
        // Two panes match: nvim and vim.
        assert_eq!(t.rows().len(), 2);
        t.type_text("x");
        assert!(t.rows().is_empty());
        t.delete_char();
        assert_eq!(t.rows().len(), 2);
        let all = lines(&t);
        assert_eq!(all[0], "> work › 1 editor › nvim");
        assert_eq!(all[1], "  notes › 1 todo › vim");
        t.down();
        assert_eq!(t.selected_key(), Key::Pane(104));
        // Left and right do nothing while there is a filter.
        t.left();
        assert_eq!(t.selected_key(), Key::Pane(104));
        t.clear_filter();
        assert_eq!(t.selected_key(), Key::Pane(104));
        assert!(t.rows().iter().any(|i| t.entries[*i].key == Key::Pane(104)));
    }

    #[test]
    fn enter_without_a_lift_goes_to_the_item() {
        let mut t = tree();
        t.down();
        t.down();
        let (action, what) = t.plan().unwrap();
        assert_eq!(action, Action::Jump(Key::Pane(103)));
        assert_eq!(what, "go to pane htop");
    }

    fn plan_of(t: &mut Tree, lift: Key, onto: Key) -> Result<Action, String> {
        t.lifted = Some(lift);
        t.selected = onto;
        t.plan().map(|(a, _)| a)
    }

    #[test]
    fn the_places_a_lifted_pane_can_be_put_down() {
        let mut t = tree();
        let pane = Key::Pane(101);
        // On another pane, on a window, on a session.
        assert_eq!(
            plan_of(&mut t, pane, Key::Pane(103)),
            Ok(Action::PaneBeside {
                pane: 101,
                target: 103
            })
        );
        assert_eq!(
            plan_of(&mut t, pane, Key::Window(12)),
            Ok(Action::PaneBeside {
                pane: 101,
                target: 103
            })
        );
        assert_eq!(
            plan_of(&mut t, pane, Key::Session(2)),
            Ok(Action::PaneToSession {
                pane: 101,
                session: 2
            })
        );
        // Its own session, where its window has another pane: a window of its own.
        assert_eq!(
            plan_of(&mut t, pane, Key::Session(1)),
            Ok(Action::PaneToSession {
                pane: 101,
                session: 1
            })
        );
        // Not on itself, not on its own window, not where there is no room.
        assert!(plan_of(&mut t, pane, pane).is_err());
        assert!(plan_of(&mut t, pane, Key::Window(11)).is_err());
        assert!(
            plan_of(&mut t, pane, Key::Pane(104))
                .unwrap_err()
                .contains("room")
        );
        // Alone in its window, a pane gains nothing from a window of its own in its session.
        assert!(plan_of(&mut t, Key::Pane(103), Key::Session(1)).is_err());
        assert!(plan_of(&mut t, Key::Pane(103), Key::Session(2)).is_ok());
    }

    #[test]
    fn the_places_a_lifted_window_can_be_put_down() {
        let mut t = tree();
        let window = Key::Window(11);
        assert_eq!(
            plan_of(&mut t, window, Key::Window(21)),
            Ok(Action::WindowAfter {
                window: 11,
                target: 21
            })
        );
        assert_eq!(
            plan_of(&mut t, window, Key::Session(2)),
            Ok(Action::WindowToSession {
                window: 11,
                session: 2
            })
        );
        // Its own session: fine from the front, nothing to do from the end.
        assert_eq!(
            plan_of(&mut t, window, Key::Session(1)),
            Ok(Action::WindowToSession {
                window: 11,
                session: 1
            })
        );
        assert!(plan_of(&mut t, Key::Window(12), Key::Session(1)).is_err());
        assert!(plan_of(&mut t, window, window).is_err());
        assert!(plan_of(&mut t, window, Key::Pane(103)).is_err());
    }

    #[test]
    fn a_session_cannot_be_lifted_and_a_second_lift_puts_it_down() {
        let mut t = tree();
        t.selected = Key::Session(1);
        t.toggle_lift();
        assert_eq!(t.lifted_key(), None);
        assert!(t.notice.is_some());
        t.selected = Key::Pane(102);
        t.toggle_lift();
        assert_eq!(t.lifted_key(), Some(Key::Pane(102)));
        t.toggle_lift();
        assert_eq!(t.lifted_key(), None);
        t.toggle_lift();
        assert!(t.put_down());
        assert!(!t.put_down());
    }

    #[test]
    fn a_refresh_puts_down_what_is_gone_and_keeps_the_choice_near() {
        let mut t = tree();
        t.selected = Key::Pane(103);
        t.lifted = Some(Key::Pane(103));
        let mut fewer = entries();
        fewer.remove(5);
        t.refresh(fewer);
        assert_eq!(t.lifted_key(), None);
        assert!(t.notice.is_some());
        assert!(t.index_of(t.selected_key()).is_some());
        assert_ne!(t.selected_key(), Key::Pane(103));
    }
}
