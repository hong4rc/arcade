//! The layout of one window: a binary tree of splits.
//!
//! Pure arithmetic, no terminals and no sockets. Each split keeps a ratio; the real sizes are
//! worked out from the ratios every time the area is known, so a window that is resized keeps its
//! proportions. There is no frame around a pane: a window with one pane gives it the whole area,
//! and only between two neighbours is there exactly one column or row (the divider).

// The daemon starts using this module in the next step.
#![allow(dead_code)]

/// Identifier of a pane (the daemon's own number; this module does not interpret it).
pub type PaneId = u64;

/// Fewest columns and rows a pane may have.
pub const MIN_COLS: u16 = 5;
pub const MIN_ROWS: u16 = 2;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Rect {
    pub x: u16,
    pub y: u16,
    pub w: u16,
    pub h: u16,
}

impl Rect {
    pub fn new(x: u16, y: u16, w: u16, h: u16) -> Rect {
        Rect { x, y, w, h }
    }

    pub fn right(&self) -> u16 {
        self.x + self.w
    }

    pub fn bottom(&self) -> u16 {
        self.y + self.h
    }

    pub fn contains(&self, x: u16, y: u16) -> bool {
        x >= self.x && x < self.right() && y >= self.y && y < self.bottom()
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Direction {
    Left,
    Down,
    Up,
    Right,
}

/// How the two children of a split sit.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Axis {
    /// Side by side, with a one column divider between them.
    LeftRight,
    /// One above the other, with a one row divider between them.
    TopBottom,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum LayoutError {
    /// The operation would leave a pane below the minimum size.
    TooSmall,
    /// There is no divider to move in that direction.
    NothingThere,
    /// The pane is not in this layout (or is the only one, for a close).
    NoSuchPane,
}

/// A divider: one column (`vertical`) or one row of cells between two neighbours.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Divider {
    pub x: u16,
    pub y: u16,
    pub len: u16,
    /// A column of cells (between panes side by side); otherwise a row.
    pub vertical: bool,
}

#[derive(Clone, Debug)]
enum Node {
    Leaf(PaneId),
    Split(Box<Split>),
}

#[derive(Clone, Debug)]
struct Split {
    axis: Axis,
    /// Share of the room (without the divider) that the first child gets.
    ratio: f64,
    a: Node,
    b: Node,
}

/// Sizes of the two children of a split that has `total` cells along its axis.
fn split_sizes(total: u16, ratio: f64, min: u16) -> (u16, u16) {
    let avail = total.saturating_sub(1);
    let lo = min.min(avail / 2);
    let a = ((ratio * f64::from(avail)).round() as u16).clamp(lo, avail - lo);
    (a, avail - a)
}

impl Node {
    fn contains(&self, pane: PaneId) -> bool {
        match self {
            Node::Leaf(p) => *p == pane,
            Node::Split(s) => s.a.contains(pane) || s.b.contains(pane),
        }
    }

    fn first_leaf(&self) -> PaneId {
        match self {
            Node::Leaf(p) => *p,
            Node::Split(s) => s.a.first_leaf(),
        }
    }

    fn place(&self, rect: Rect, panes: &mut Vec<(PaneId, Rect)>, dividers: &mut Vec<Divider>) {
        match self {
            Node::Leaf(p) => panes.push((*p, rect)),
            Node::Split(s) => {
                let (ra, rb, divider) = s.children(rect);
                dividers.push(divider);
                s.a.place(ra, panes, dividers);
                s.b.place(rb, panes, dividers);
            }
        }
    }

    fn replace_leaf(&mut self, target: PaneId, with: Node) -> bool {
        match self {
            Node::Leaf(p) if *p == target => {
                *self = with;
                true
            }
            Node::Leaf(_) => false,
            Node::Split(s) => {
                if s.a.contains(target) {
                    s.a.replace_leaf(target, with)
                } else {
                    s.b.replace_leaf(target, with)
                }
            }
        }
    }

    /// Remove the leaf; returns the pane that took its place (first leaf of the sibling).
    fn remove_leaf(&mut self, target: PaneId) -> Option<PaneId> {
        let Node::Split(s) = self else {
            return None;
        };
        let in_a = matches!(&s.a, Node::Leaf(p) if *p == target);
        let in_b = matches!(&s.b, Node::Leaf(p) if *p == target);
        if in_a || in_b {
            let sibling = if in_a {
                std::mem::replace(&mut s.b, Node::Leaf(0))
            } else {
                std::mem::replace(&mut s.a, Node::Leaf(0))
            };
            let taken = sibling.first_leaf();
            *self = sibling;
            return Some(taken);
        }
        if s.a.contains(target) {
            s.a.remove_leaf(target)
        } else {
            s.b.remove_leaf(target)
        }
    }
}

impl Split {
    fn min(&self) -> u16 {
        match self.axis {
            Axis::LeftRight => MIN_COLS,
            Axis::TopBottom => MIN_ROWS,
        }
    }

    /// Rectangles of the two children and the divider, inside `rect`.
    fn children(&self, rect: Rect) -> (Rect, Rect, Divider) {
        match self.axis {
            Axis::LeftRight => {
                let (aw, bw) = split_sizes(rect.w, self.ratio, self.min());
                (
                    Rect::new(rect.x, rect.y, aw, rect.h),
                    Rect::new(rect.x + aw + 1, rect.y, bw, rect.h),
                    Divider {
                        x: rect.x + aw,
                        y: rect.y,
                        len: rect.h,
                        vertical: true,
                    },
                )
            }
            Axis::TopBottom => {
                let (ah, bh) = split_sizes(rect.h, self.ratio, self.min());
                (
                    Rect::new(rect.x, rect.y, rect.w, ah),
                    Rect::new(rect.x, rect.y + ah + 1, rect.w, bh),
                    Divider {
                        x: rect.x,
                        y: rect.y + ah,
                        len: rect.w,
                        vertical: false,
                    },
                )
            }
        }
    }
}

enum Moved {
    NotFound,
    Done,
    Rejected,
}

/// Move the divider of the nearest split on the pane's side of the tree that lies along the
/// direction's axis.
fn resize_node(
    node: &mut Node,
    rect: Rect,
    pane: PaneId,
    direction: Direction,
    amount: u16,
) -> Moved {
    let Node::Split(s) = node else {
        return Moved::NotFound;
    };
    let (ra, rb, _) = s.children(rect);
    let in_a = s.a.contains(pane);
    if !in_a && !s.b.contains(pane) {
        return Moved::NotFound;
    }
    let inner = if in_a {
        resize_node(&mut s.a, ra, pane, direction, amount)
    } else {
        resize_node(&mut s.b, rb, pane, direction, amount)
    };
    if !matches!(inner, Moved::NotFound) {
        return inner;
    }
    let along = match direction {
        Direction::Left | Direction::Right => Axis::LeftRight,
        Direction::Up | Direction::Down => Axis::TopBottom,
    };
    if s.axis != along {
        return Moved::NotFound;
    }
    let total = match s.axis {
        Axis::LeftRight => rect.w,
        Axis::TopBottom => rect.h,
    };
    let avail = total.saturating_sub(1);
    let (a_now, _) = split_sizes(total, s.ratio, s.min());
    let grow = matches!(direction, Direction::Right | Direction::Down);
    let a_new = if grow {
        i32::from(a_now) + i32::from(amount)
    } else {
        i32::from(a_now) - i32::from(amount)
    };
    let min = i32::from(s.min());
    if a_new < min || i32::from(avail) - a_new < min {
        return Moved::Rejected;
    }
    s.ratio = f64::from(a_new as u16) / f64::from(avail);
    Moved::Done
}

#[derive(Clone, Debug)]
pub struct Layout {
    root: Node,
    cols: u16,
    rows: u16,
}

impl Layout {
    /// A layout of one pane filling `cols` by `rows`.
    pub fn new(pane: PaneId, cols: u16, rows: u16) -> Layout {
        Layout {
            root: Node::Leaf(pane),
            cols,
            rows,
        }
    }

    pub fn area(&self) -> Rect {
        Rect::new(0, 0, self.cols, self.rows)
    }

    /// The window changed size; panes follow by ratio.
    pub fn set_area(&mut self, cols: u16, rows: u16) {
        self.cols = cols;
        self.rows = rows;
    }

    /// Every pane with its rectangle, left to right and top to bottom as the tree is walked.
    pub fn rects(&self) -> Vec<(PaneId, Rect)> {
        self.placed().0
    }

    pub fn dividers(&self) -> Vec<Divider> {
        self.placed().1
    }

    fn placed(&self) -> (Vec<(PaneId, Rect)>, Vec<Divider>) {
        let mut panes = Vec::new();
        let mut dividers = Vec::new();
        self.root.place(self.area(), &mut panes, &mut dividers);
        (panes, dividers)
    }

    pub fn panes(&self) -> Vec<PaneId> {
        self.rects().into_iter().map(|(p, _)| p).collect()
    }

    pub fn len(&self) -> usize {
        self.rects().len()
    }

    pub fn is_empty(&self) -> bool {
        false
    }

    pub fn contains(&self, pane: PaneId) -> bool {
        self.root.contains(pane)
    }

    pub fn rect_of(&self, pane: PaneId) -> Option<Rect> {
        self.rects()
            .into_iter()
            .find(|(p, _)| *p == pane)
            .map(|(_, r)| r)
    }

    fn all_big_enough(&self) -> bool {
        self.rects()
            .iter()
            .all(|(_, r)| r.w >= MIN_COLS && r.h >= MIN_ROWS)
    }

    /// Split `target`'s room in two; `new_pane` takes the right or lower half. Refused (and the
    /// layout left as it was) when a pane would end up below the minimum size.
    pub fn split(
        &mut self,
        target: PaneId,
        axis: Axis,
        new_pane: PaneId,
    ) -> Result<(), LayoutError> {
        if !self.contains(target) {
            return Err(LayoutError::NoSuchPane);
        }
        let before = self.root.clone();
        let node = Node::Split(Box::new(Split {
            axis,
            ratio: 0.5,
            a: Node::Leaf(target),
            b: Node::Leaf(new_pane),
        }));
        self.root.replace_leaf(target, node);
        if self.all_big_enough() {
            Ok(())
        } else {
            self.root = before;
            Err(LayoutError::TooSmall)
        }
    }

    /// Remove a pane and give its room to its neighbour. Returns the pane that took the room.
    /// The only pane cannot be removed.
    pub fn close(&mut self, pane: PaneId) -> Result<PaneId, LayoutError> {
        if !self.contains(pane) {
            return Err(LayoutError::NoSuchPane);
        }
        self.root.remove_leaf(pane).ok_or(LayoutError::NoSuchPane)
    }

    /// Move the divider nearest to `pane` along the direction's axis by `amount` cells in the
    /// direction. Refused when a pane would drop below the minimum size.
    pub fn resize(
        &mut self,
        pane: PaneId,
        direction: Direction,
        amount: u16,
    ) -> Result<(), LayoutError> {
        if !self.contains(pane) {
            return Err(LayoutError::NoSuchPane);
        }
        let before = self.root.clone();
        let area = self.area();
        match resize_node(&mut self.root, area, pane, direction, amount) {
            Moved::NotFound => Err(LayoutError::NothingThere),
            Moved::Rejected => {
                self.root = before;
                Err(LayoutError::TooSmall)
            }
            Moved::Done => {
                if self.all_big_enough() {
                    Ok(())
                } else {
                    self.root = before;
                    Err(LayoutError::TooSmall)
                }
            }
        }
    }

    /// The pane next to `from` in the direction. When several touch it, the one that holds
    /// `line` (the cursor's row for left and right, its column for up and down) wins; if none
    /// does, the nearest to it.
    pub fn neighbour(&self, from: PaneId, direction: Direction, line: u16) -> Option<PaneId> {
        let rects = self.rects();
        let fr = rects.iter().find(|(p, _)| *p == from).map(|(_, r)| *r)?;
        let overlaps = |a0: u16, a1: u16, b0: u16, b1: u16| a0 < b1 && b0 < a1;
        let mut best: Option<(u16, PaneId)> = None;
        for (pane, r) in &rects {
            if *pane == from {
                continue;
            }
            let (adjacent, lo, hi) = match direction {
                Direction::Left => (
                    r.right() + 1 == fr.x && overlaps(r.y, r.bottom(), fr.y, fr.bottom()),
                    r.y,
                    r.bottom(),
                ),
                Direction::Right => (
                    r.x == fr.right() + 1 && overlaps(r.y, r.bottom(), fr.y, fr.bottom()),
                    r.y,
                    r.bottom(),
                ),
                Direction::Up => (
                    r.bottom() + 1 == fr.y && overlaps(r.x, r.right(), fr.x, fr.right()),
                    r.x,
                    r.right(),
                ),
                Direction::Down => (
                    r.y == fr.bottom() + 1 && overlaps(r.x, r.right(), fr.x, fr.right()),
                    r.x,
                    r.right(),
                ),
            };
            if !adjacent {
                continue;
            }
            let distance = if line < lo {
                lo - line
            } else if line >= hi {
                line - hi + 1
            } else {
                0
            };
            if best.is_none_or(|(d, _)| distance < d) {
                best = Some((distance, *pane));
            }
        }
        best.map(|(_, p)| p)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashSet;

    /// Every cell of the area is covered exactly once by a pane or a divider.
    fn assert_covers(layout: &Layout) {
        let area = layout.area();
        let mut seen: HashSet<(u16, u16)> = HashSet::new();
        for (pane, r) in layout.rects() {
            for y in r.y..r.bottom() {
                for x in r.x..r.right() {
                    assert!(seen.insert((x, y)), "pane {pane} overlaps at {x},{y}");
                }
            }
        }
        for d in layout.dividers() {
            for i in 0..d.len {
                let cell = if d.vertical {
                    (d.x, d.y + i)
                } else {
                    (d.x + i, d.y)
                };
                assert!(seen.insert(cell), "divider overlaps at {cell:?}");
            }
        }
        assert_eq!(
            seen.len(),
            usize::from(area.w) * usize::from(area.h),
            "the area is not covered"
        );
        for (x, y) in seen {
            assert!(area.contains(x, y));
        }
    }

    /// A layout of `n` panes, split alternately left-right and top-bottom.
    fn grown(n: u64, cols: u16, rows: u16) -> Layout {
        let mut l = Layout::new(1, cols, rows);
        for id in 2..=n {
            let axis = if id % 2 == 0 {
                Axis::LeftRight
            } else {
                Axis::TopBottom
            };
            l.split(id - 1, axis, id).expect("split");
        }
        l
    }

    #[test]
    fn one_pane_fills_the_area_and_loses_no_cell() {
        let l = Layout::new(1, 100, 29);
        assert_eq!(l.rects(), vec![(1, Rect::new(0, 0, 100, 29))]);
        assert!(l.dividers().is_empty());
        assert_covers(&l);
    }

    #[test]
    fn rooms_and_dividers_cover_the_window_for_one_to_eight_panes() {
        for n in 1..=8 {
            let l = grown(n, 200, 60);
            assert_eq!(l.len(), n as usize);
            assert_covers(&l);
            for (_, r) in l.rects() {
                assert!(r.w >= MIN_COLS && r.h >= MIN_ROWS);
            }
        }
    }

    #[test]
    fn two_neighbours_are_exactly_one_cell_apart() {
        let mut l = Layout::new(1, 101, 30);
        l.split(1, Axis::LeftRight, 2).unwrap();
        let r = l.rects();
        assert_eq!(r[0].1, Rect::new(0, 0, 50, 30));
        assert_eq!(r[1].1, Rect::new(51, 0, 50, 30));
        assert_eq!(
            l.dividers(),
            vec![Divider {
                x: 50,
                y: 0,
                len: 30,
                vertical: true
            }]
        );
        let mut l = Layout::new(1, 80, 31);
        l.split(1, Axis::TopBottom, 2).unwrap();
        let r = l.rects();
        assert_eq!(r[1].1.y, r[0].1.bottom() + 1);
    }

    #[test]
    fn splitting_then_closing_gives_back_the_old_room() {
        let mut l = Layout::new(1, 120, 40);
        let before = l.rects();
        l.split(1, Axis::LeftRight, 2).unwrap();
        l.split(2, Axis::TopBottom, 3).unwrap();
        assert_eq!(l.close(3), Ok(2));
        assert_eq!(l.close(2), Ok(1));
        assert_eq!(l.rects(), before);
        // Closing a middle pane hands its room to the one beside it.
        let mut l = Layout::new(1, 120, 40);
        l.split(1, Axis::LeftRight, 2).unwrap();
        let two_panes = l.rects();
        l.split(2, Axis::LeftRight, 3).unwrap();
        l.close(3).unwrap();
        assert_eq!(l.rects(), two_panes);
        assert_eq!(l.close(1), Ok(2));
        assert_eq!(l.rects(), vec![(2, Rect::new(0, 0, 120, 40))]);
        assert_eq!(l.close(2), Err(LayoutError::NoSuchPane));
    }

    #[test]
    fn resizing_the_window_keeps_the_proportions() {
        let mut l = Layout::new(1, 101, 30);
        l.split(1, Axis::LeftRight, 2).unwrap();
        l.resize(1, Direction::Right, 10).unwrap();
        let first = l.rect_of(1).unwrap();
        assert_eq!(first.w, 60);
        l.set_area(201, 40);
        assert_covers(&l);
        // 60 of 100 stays 60 percent: 120 of 200.
        assert_eq!(l.rect_of(1).unwrap().w, 120);
        assert_eq!(l.rect_of(2).unwrap().w, 80);
        l.set_area(101, 30);
        assert_eq!(l.rect_of(1).unwrap(), first);
    }

    #[test]
    fn a_shrinking_window_never_breaks_the_covering() {
        let mut l = grown(5, 200, 60);
        for (cols, rows) in [
            (150, 40),
            (80, 24),
            (40, 12),
            (20, 8),
            (11, 5),
            (7, 3),
            (3, 1),
        ] {
            l.set_area(cols, rows);
            let (panes, dividers) = (l.rects(), l.dividers());
            assert_eq!(panes.len(), 5);
            assert!(!dividers.is_empty());
            for (_, r) in panes {
                assert!(r.right() <= cols.max(r.x) || cols < 11);
            }
        }
    }

    #[test]
    fn resizing_down_to_the_minimum_stops_and_leaves_the_tree_alone() {
        let mut l = Layout::new(1, 101, 30);
        l.split(1, Axis::LeftRight, 2).unwrap();
        // 50 columns: 45 to give before pane 1 has the minimum 5... in steps of 5.
        let mut steps = 0;
        while l.resize(1, Direction::Left, 5).is_ok() {
            steps += 1;
            assert!(steps < 100);
        }
        assert_eq!(l.rect_of(1).unwrap().w, 5);
        let frozen = l.rects();
        assert_eq!(l.resize(1, Direction::Left, 5), Err(LayoutError::TooSmall));
        assert_eq!(l.resize(1, Direction::Left, 1), Err(LayoutError::TooSmall));
        assert_eq!(l.rects(), frozen);
        assert_covers(&l);
        // The other way too.
        let mut l = Layout::new(1, 40, 9);
        l.split(1, Axis::TopBottom, 2).unwrap();
        while l.resize(1, Direction::Up, 1).is_ok() {}
        assert_eq!(l.rect_of(1).unwrap().h, MIN_ROWS);
        let frozen = l.rects();
        assert!(l.resize(1, Direction::Up, 2).is_err());
        assert_eq!(l.rects(), frozen);
    }

    #[test]
    fn resizing_with_no_divider_that_way_is_refused() {
        let mut l = Layout::new(1, 100, 30);
        assert_eq!(
            l.resize(1, Direction::Left, 5),
            Err(LayoutError::NothingThere)
        );
        l.split(1, Axis::LeftRight, 2).unwrap();
        // Only left-right dividers exist.
        assert_eq!(
            l.resize(1, Direction::Up, 2),
            Err(LayoutError::NothingThere)
        );
    }

    #[test]
    fn splitting_with_no_room_is_refused_and_changes_nothing() {
        let mut l = Layout::new(1, 10, 30);
        let before = l.rects();
        assert_eq!(l.split(1, Axis::LeftRight, 2), Err(LayoutError::TooSmall));
        assert_eq!(l.rects(), before);
        // 11 columns hold two panes of five and the divider.
        let mut l = Layout::new(1, 11, 4);
        assert!(l.split(1, Axis::LeftRight, 2).is_ok());
        assert_eq!(l.split(1, Axis::TopBottom, 3), Err(LayoutError::TooSmall));
        assert_eq!(l.len(), 2);
        let mut l = Layout::new(1, 40, 4);
        assert_eq!(l.split(1, Axis::TopBottom, 2), Err(LayoutError::TooSmall));
        let mut l = Layout::new(1, 40, 5);
        assert!(l.split(1, Axis::TopBottom, 2).is_ok());
    }

    #[test]
    fn focus_moves_in_four_directions_in_a_two_by_two_grid() {
        // 1 | 2 on top, 3 | 4 below.
        let mut l = Layout::new(1, 101, 41);
        l.split(1, Axis::TopBottom, 3).unwrap();
        l.split(1, Axis::LeftRight, 2).unwrap();
        l.split(3, Axis::LeftRight, 4).unwrap();
        let near = |p, d| l.neighbour(p, d, 0);
        assert_eq!(near(1, Direction::Right), Some(2));
        assert_eq!(near(1, Direction::Down), Some(3));
        assert_eq!(near(4, Direction::Left), Some(3));
        assert_eq!(near(4, Direction::Up), Some(2));
        assert_eq!(near(2, Direction::Left), Some(1));
        assert_eq!(near(3, Direction::Up), Some(1));
        assert_eq!(near(1, Direction::Left), None);
        assert_eq!(near(1, Direction::Up), None);
        assert_eq!(near(4, Direction::Right), None);
        assert_eq!(near(4, Direction::Down), None);
    }

    #[test]
    fn focus_prefers_the_neighbour_that_holds_the_cursor_line() {
        // Pane 1 on the left, panes 2 (top) and 3 (bottom) on the right.
        let mut l = Layout::new(1, 101, 41);
        l.split(1, Axis::LeftRight, 2).unwrap();
        l.split(2, Axis::TopBottom, 3).unwrap();
        let top = l.rect_of(2).unwrap();
        let bottom = l.rect_of(3).unwrap();
        assert_eq!(l.neighbour(1, Direction::Right, top.y + 1), Some(2));
        assert_eq!(l.neighbour(1, Direction::Right, bottom.y + 1), Some(3));
        // From either one, left is pane 1.
        assert_eq!(l.neighbour(3, Direction::Left, 0), Some(1));
        // A line that holds none (the divider row) goes to the nearest.
        let on_divider = top.bottom();
        assert!(matches!(
            l.neighbour(1, Direction::Right, on_divider),
            Some(2 | 3)
        ));
    }

    #[test]
    fn resizing_moves_the_nearest_divider() {
        // 1 | (2 / 3)
        let mut l = Layout::new(1, 101, 41);
        l.split(1, Axis::LeftRight, 2).unwrap();
        l.split(2, Axis::TopBottom, 3).unwrap();
        l.resize(3, Direction::Up, 4).unwrap();
        assert_eq!(l.rect_of(2).unwrap().h, 20 - 4);
        assert_eq!(l.rect_of(3).unwrap().h, 20 + 4);
        // Pane 3 has no divider of its own on the left-right axis nearer than the root's.
        l.resize(3, Direction::Right, 5).unwrap();
        assert_eq!(l.rect_of(1).unwrap().w, 55);
        assert_covers(&l);
    }
}
