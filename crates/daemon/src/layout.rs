//! The layout of one window: a binary tree of splits.
//!
//! Pure arithmetic, no terminals and no sockets. Each split keeps a ratio; the real sizes are
//! worked out from the ratios every time the area is known, so a window that is resized keeps its
//! proportions. There is no frame around a pane: a window with one pane gives it the whole area,
//! and only between two neighbours is there exactly one column or row (the divider).

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

/// Sizes of the two children of a split that has `total` cells along its axis. `need_*` is what
/// each child's subtree needs along the axis for all its panes to have the minimum size, and
/// `least_*` what it needs for all its panes to have one cell. The ratio decides, but never at
/// the cost of a pane that could still be given its room.
fn split_sizes(
    total: u16,
    ratio: f64,
    need_a: u16,
    need_b: u16,
    least_a: u16,
    least_b: u16,
) -> (u16, u16) {
    let avail = total.saturating_sub(1);
    let (lo_a, lo_b) = if need_a + need_b <= avail {
        (need_a, need_b)
    } else if least_a + least_b <= avail {
        // Not room for every pane at the minimum size: each child keeps up to half of the room
        // at the minimum it would like, and never less than it needs to show at all.
        let want_a = least_a.max(need_a.min(avail / 2));
        let want_b = least_b.max(need_b.min(avail / 2));
        if want_a + want_b <= avail {
            (want_a, want_b)
        } else {
            (least_a, least_b)
        }
    } else {
        // Not even one cell for every pane: some panes have to go without.
        (0, 0)
    };
    let a = ((ratio * f64::from(avail)).round() as u16).clamp(lo_a, avail - lo_b);
    (a, avail - a)
}

/// The layouts `window.layout-next` goes through, in this order.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Arrangement {
    /// Every pane the same width, side by side.
    EvenHorizontal,
    /// Every pane the same height, one above the other.
    EvenVertical,
    /// The first pane takes the left part, the others share the right part, one above the other.
    MainPane,
}

/// The share of the width the main pane takes.
const MAIN_PANE_SHARE: f64 = 0.6;

impl Arrangement {
    /// The layout after the one applied last (the first when none was).
    pub fn after(last: Option<Arrangement>) -> Arrangement {
        match last {
            None | Some(Arrangement::MainPane) => Arrangement::EvenHorizontal,
            Some(Arrangement::EvenHorizontal) => Arrangement::EvenVertical,
            Some(Arrangement::EvenVertical) => Arrangement::MainPane,
        }
    }
}

/// `panes` one after the other along `axis`, all of the same size.
fn chain(panes: &[PaneId], axis: Axis) -> Node {
    match panes {
        [] | [_] => Node::Leaf(panes.first().copied().unwrap_or(0)),
        [first, rest @ ..] => Node::Split(Box::new(Split {
            axis,
            ratio: 1.0 / panes.len() as f64,
            a: Node::Leaf(*first),
            b: chain(rest, axis),
        })),
    }
}

impl Node {
    /// How many panes lie one after the other along `along` in the widest place of this subtree.
    fn span(&self, along: Axis) -> usize {
        match self {
            Node::Leaf(_) => 1,
            Node::Split(s) if s.axis == along => s.a.span(along) + s.b.span(along),
            Node::Split(s) => s.a.span(along).max(s.b.span(along)),
        }
    }

    /// Set every ratio so that the panes along each axis get the same room, to the cell: the
    /// room that is left once the dividers are taken out is shared out, and the cells that do
    /// not divide go to the first panes. `cols` and `rows` are the room of this subtree.
    fn share_equally(&mut self, cols: u16, rows: u16) {
        let Node::Split(s) = self else {
            return;
        };
        let total = match s.axis {
            Axis::LeftRight => cols,
            Axis::TopBottom => rows,
        };
        let (pa, pb) = (s.a.span(s.axis), s.b.span(s.axis));
        let avail = usize::from(total.saturating_sub(1));
        // The panes in a row, and the dividers between them, take the room of the subtree.
        let for_panes = avail.saturating_sub(pa - 1 + pb - 1);
        let a_panes = (for_panes * pa).div_ceil(pa + pb);
        let a_cells = a_panes + pa - 1;
        s.ratio = if avail == 0 {
            0.5
        } else {
            a_cells as f64 / avail as f64
        };
        let a_cells = u16::try_from(a_cells).unwrap_or(u16::MAX);
        let b_cells = total.saturating_sub(1).saturating_sub(a_cells);
        match s.axis {
            Axis::LeftRight => {
                s.a.share_equally(a_cells, rows);
                s.b.share_equally(b_cells, rows);
            }
            Axis::TopBottom => {
                s.a.share_equally(cols, a_cells);
                s.b.share_equally(cols, b_cells);
            }
        }
    }

    /// Put `a` where `b` is and `b` where `a` is.
    fn swap_leaves(&mut self, a: PaneId, b: PaneId) {
        match self {
            Node::Leaf(p) if *p == a => *p = b,
            Node::Leaf(p) if *p == b => *p = a,
            Node::Leaf(_) => {}
            Node::Split(s) => {
                s.a.swap_leaves(a, b);
                s.b.swap_leaves(a, b);
            }
        }
    }

    /// The room this subtree needs along `along` for every pane in it to have `cols` columns and
    /// `rows` rows.
    fn need(&self, along: Axis, cols: u16, rows: u16) -> u16 {
        match self {
            Node::Leaf(_) => match along {
                Axis::LeftRight => cols,
                Axis::TopBottom => rows,
            },
            Node::Split(s) => {
                let a = s.a.need(along, cols, rows);
                let b = s.b.need(along, cols, rows);
                if s.axis == along { a + b + 1 } else { a.max(b) }
            }
        }
    }

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

    /// How many cells of `total` along the axis the first child gets, and the second.
    fn sizes(&self, total: u16) -> (u16, u16) {
        split_sizes(
            total,
            self.ratio,
            self.a.need(self.axis, MIN_COLS, MIN_ROWS),
            self.b.need(self.axis, MIN_COLS, MIN_ROWS),
            self.a.need(self.axis, 1, 1),
            self.b.need(self.axis, 1, 1),
        )
    }

    /// Rectangles of the two children and the divider, inside `rect`.
    fn children(&self, rect: Rect) -> (Rect, Rect, Divider) {
        match self.axis {
            Axis::LeftRight => {
                let (aw, bw) = self.sizes(rect.w);
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
                let (ah, bh) = self.sizes(rect.h);
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

/// Which divider a drag moves: the split it belongs to, named by the way down the tree (false is
/// the first child, true the second), so it stays the same divider while it moves.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct DividerHandle {
    path: Vec<bool>,
    vertical: bool,
}

impl DividerHandle {
    /// A divider between panes side by side (moved along the columns), or one between panes
    /// above one another.
    pub fn vertical(&self) -> bool {
        self.vertical
    }
}

fn on_divider(d: &Divider, x: u16, y: u16) -> bool {
    if d.vertical {
        d.x == x && y >= d.y && y < d.y + d.len
    } else {
        d.y == y && x >= d.x && x < d.x + d.len
    }
}

impl Node {
    /// The divider that runs through the cell (x, y), if one does. A cell where a divider ends
    /// on another belongs to the one it ends on: the dividers' cells never overlap.
    fn divider_at(
        &self,
        rect: Rect,
        x: u16,
        y: u16,
        path: &mut Vec<bool>,
    ) -> Option<DividerHandle> {
        let Node::Split(s) = self else {
            return None;
        };
        let (ra, rb, d) = s.children(rect);
        if on_divider(&d, x, y) {
            return Some(DividerHandle {
                path: path.clone(),
                vertical: d.vertical,
            });
        }
        path.push(false);
        let found = s.a.divider_at(ra, x, y, path);
        path.pop();
        if found.is_some() {
            return found;
        }
        path.push(true);
        let found = s.b.divider_at(rb, x, y, path);
        path.pop();
        found
    }

    /// The split at the end of `path` and the room it has.
    fn split_at(&mut self, rect: Rect, path: &[bool]) -> Option<(&mut Split, Rect)> {
        let Node::Split(s) = self else {
            return None;
        };
        match path.split_first() {
            None => Some((s, rect)),
            Some((&second, rest)) => {
                let (ra, rb, _) = s.children(rect);
                if second {
                    s.b.split_at(rb, rest)
                } else {
                    s.a.split_at(ra, rest)
                }
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
    let (a_now, _) = s.sizes(total);
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

    pub fn len(&self) -> usize {
        self.rects().len()
    }

    /// The room, in columns and rows, in which every pane could still have one cell.
    pub fn least_room(&self) -> (u16, u16) {
        (
            self.root.need(Axis::LeftRight, 1, 1),
            self.root.need(Axis::TopBottom, 1, 1),
        )
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
        // The pane must have room for two of its own: a split never moves the dividers of the
        // other panes to find the room.
        let rect = self.rect_of(target).ok_or(LayoutError::NoSuchPane)?;
        let fits = match axis {
            Axis::LeftRight => rect.w > 2 * MIN_COLS,
            Axis::TopBottom => rect.h > 2 * MIN_ROWS,
        };
        if !fits {
            return Err(LayoutError::TooSmall);
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

    /// Put two panes in each other's place; the rooms stay as they are.
    pub fn swap(&mut self, a: PaneId, b: PaneId) -> Result<(), LayoutError> {
        if !self.contains(a) || !self.contains(b) {
            return Err(LayoutError::NoSuchPane);
        }
        self.root.swap_leaves(a, b);
        Ok(())
    }

    /// Lay the panes out again in this arrangement, in the order they have now. Refused (and
    /// the layout left as it was) when a pane would end up below the minimum size.
    pub fn arrange(&mut self, kind: Arrangement) -> Result<(), LayoutError> {
        let panes: Vec<PaneId> = self.rects().into_iter().map(|(p, _)| p).collect();
        let before = self.root.clone();
        self.root = match kind {
            Arrangement::EvenHorizontal => chain(&panes, Axis::LeftRight),
            Arrangement::EvenVertical => chain(&panes, Axis::TopBottom),
            Arrangement::MainPane => match panes.split_first() {
                Some((main, rest)) if !rest.is_empty() => Node::Split(Box::new(Split {
                    axis: Axis::LeftRight,
                    ratio: MAIN_PANE_SHARE,
                    a: Node::Leaf(*main),
                    b: chain(rest, Axis::TopBottom),
                })),
                _ => before.clone(),
            },
        };
        self.keep_if_big_enough(before)
    }

    /// Keep the tree and give the panes along each axis the same room.
    pub fn even(&mut self) -> Result<(), LayoutError> {
        let before = self.root.clone();
        let (cols, rows) = (self.cols, self.rows);
        self.root.share_equally(cols, rows);
        self.keep_if_big_enough(before)
    }

    fn keep_if_big_enough(&mut self, before: Node) -> Result<(), LayoutError> {
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

    /// The divider that runs through the cell (x, y) of the window, if there is one.
    pub fn divider_at(&self, x: u16, y: u16) -> Option<DividerHandle> {
        self.root.divider_at(self.area(), x, y, &mut Vec::new())
    }

    /// Move a divider so that it lies under `to`, a column (for a divider between panes side by
    /// side) or a row of the window, as far as the minimum sizes allow: the same limits as
    /// [`Layout::resize`], and a pointer that goes further stops the divider at the limit.
    /// Returns whether anything moved. A divider that no longer exists is an error.
    pub fn drag_divider(&mut self, handle: &DividerHandle, to: u16) -> Result<bool, LayoutError> {
        let area = self.area();
        let (origin, total, now, min) = {
            let (split, rect) = self
                .root
                .split_at(area, &handle.path)
                .ok_or(LayoutError::NothingThere)?;
            let (origin, total) = match split.axis {
                Axis::LeftRight => (rect.x, rect.w),
                Axis::TopBottom => (rect.y, rect.h),
            };
            (origin, total, split.sizes(total).0, split.min())
        };
        let avail = total.saturating_sub(1);
        if avail < 2 * min {
            return Err(LayoutError::TooSmall);
        }
        let mut want =
            (i32::from(to) - i32::from(origin)).clamp(i32::from(min), i32::from(avail - min));
        let now = i32::from(now);
        let before = self.root.clone();
        while want != now {
            if let Some((split, _)) = self.root.split_at(area, &handle.path) {
                split.ratio = f64::from(want as u16) / f64::from(avail);
                if i32::from(split.sizes(total).0) == want && self.all_big_enough() {
                    return Ok(true);
                }
            }
            self.root = before.clone();
            want += (now - want).signum();
        }
        self.root = before;
        Ok(false)
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

    #[test]
    fn a_window_squeezed_below_the_minimum_still_gives_every_pane_a_cell_when_it_can() {
        // 1 | (2 | 3) made roomy, then the window shrinks: ratios alone would give pane 3 or
        // pane 2 nothing at 6 columns, though 5 columns are enough for one cell each.
        let mut l = Layout::new(1, 40, 10);
        l.split(1, Axis::LeftRight, 2).unwrap();
        l.split(2, Axis::LeftRight, 3).unwrap();
        for cols in [5, 6, 7, 8, 9, 11] {
            l.set_area(cols, 3);
            let rects = l.rects();
            assert!(
                rects.iter().all(|(_, r)| r.w >= 1 && r.h >= 1),
                "{cols} columns: {rects:?}"
            );
            assert_covers(&l);
        }
        // With a pane to the left, a pane up and a pane down in the second half.
        let mut l = Layout::new(1, 40, 20);
        l.split(1, Axis::LeftRight, 2).unwrap();
        l.split(2, Axis::TopBottom, 3).unwrap();
        l.split(3, Axis::TopBottom, 4).unwrap();
        for rows in [5, 6, 7] {
            l.set_area(40, rows);
            assert!(
                l.rects().iter().all(|(_, r)| r.w >= 1 && r.h >= 1),
                "{rows} rows"
            );
        }
        assert_eq!(l.least_room(), (3, 5));
    }

    #[test]
    fn swapping_two_panes_keeps_every_room_and_trades_who_is_in_it() {
        let mut l = Layout::new(1, 101, 30);
        l.split(1, Axis::LeftRight, 2).unwrap();
        l.split(2, Axis::TopBottom, 3).unwrap();
        let before = l.rects();
        l.swap(1, 3).unwrap();
        let after = l.rects();
        assert_eq!(after.len(), before.len());
        for ((pa, ra), (pb, rb)) in before.iter().zip(&after) {
            assert_eq!(ra, rb);
            let expected = match pa {
                1 => 3,
                3 => 1,
                other => *other,
            };
            assert_eq!(*pb, expected);
        }
        assert_eq!(l.swap(1, 9), Err(LayoutError::NoSuchPane));
    }

    fn four_panes(cols: u16, rows: u16) -> Layout {
        let mut l = Layout::new(1, cols, rows);
        l.split(1, Axis::LeftRight, 2).unwrap();
        l.split(2, Axis::TopBottom, 3).unwrap();
        l.split(1, Axis::TopBottom, 4).unwrap();
        l
    }

    /// The panes and dividers fill the area exactly and every pane has some room.
    fn assert_fills(l: &Layout) {
        let cells: usize = l
            .rects()
            .iter()
            .map(|(_, r)| usize::from(r.w) * usize::from(r.h))
            .sum();
        let dividers: usize = l.dividers().iter().map(|d| usize::from(d.len)).sum();
        assert!(l.rects().iter().all(|(_, r)| r.w > 0 && r.h > 0));
        assert_eq!(
            cells + dividers,
            usize::from(l.area().w) * usize::from(l.area().h)
        );
    }

    #[test]
    fn the_arrangements_go_round_in_order_and_each_fills_the_area() {
        for (cols, rows) in [(101, 30), (80, 24), (47, 13)] {
            let mut l = four_panes(cols, rows);
            let order: Vec<PaneId> = l.rects().into_iter().map(|(p, _)| p).collect();
            let mut last = None;
            let mut seen = Vec::new();
            for _ in 0..6 {
                let next = Arrangement::after(last);
                l.arrange(next).unwrap();
                assert_fills(&l);
                // The order of the panes is kept.
                let now: Vec<PaneId> = l.rects().into_iter().map(|(p, _)| p).collect();
                assert_eq!(now, order);
                seen.push(next);
                last = Some(next);
            }
            use Arrangement::*;
            assert_eq!(
                seen,
                [
                    EvenHorizontal,
                    EvenVertical,
                    MainPane,
                    EvenHorizontal,
                    EvenVertical,
                    MainPane
                ]
            );
        }
    }

    #[test]
    fn even_arrangements_give_equal_rooms_and_the_main_pane_the_larger_share() {
        let mut l = four_panes(101, 30);
        l.arrange(Arrangement::EvenHorizontal).unwrap();
        let widths: Vec<u16> = l.rects().iter().map(|(_, r)| r.w).collect();
        assert!(
            widths.iter().max().unwrap() - widths.iter().min().unwrap() <= 1,
            "{widths:?}"
        );
        assert!(l.rects().iter().all(|(_, r)| r.h == 30));
        l.arrange(Arrangement::EvenVertical).unwrap();
        let heights: Vec<u16> = l.rects().iter().map(|(_, r)| r.h).collect();
        assert!(
            heights.iter().max().unwrap() - heights.iter().min().unwrap() <= 1,
            "{heights:?}"
        );
        l.arrange(Arrangement::MainPane).unwrap();
        let rects = l.rects();
        assert_eq!(rects[0].1.h, 30);
        assert!(rects[0].1.w > rects[1].1.w);
        assert!(rects[1..].iter().all(|(_, r)| r.x == rects[1].1.x));
    }

    #[test]
    fn an_arrangement_that_leaves_a_pane_too_small_is_refused_and_changes_nothing() {
        let mut l = four_panes(22, 30);
        let before = l.rects();
        // Four panes side by side need 4 * 5 + 3 = 23 columns.
        assert_eq!(
            l.arrange(Arrangement::EvenHorizontal),
            Err(LayoutError::TooSmall)
        );
        assert_eq!(l.rects(), before);
    }

    #[test]
    fn evening_out_leaves_the_panes_of_one_direction_within_one_cell_of_each_other() {
        for cols in [60u16, 79, 101, 130] {
            let mut l = Layout::new(1, cols, 30);
            l.split(1, Axis::LeftRight, 2).unwrap();
            l.split(2, Axis::LeftRight, 3).unwrap();
            l.split(3, Axis::LeftRight, 4).unwrap();
            // Skew it: the divider next to pane 4 goes left.
            for _ in 0..3 {
                let _ = l.resize(4, Direction::Left, 5);
            }
            l.even().unwrap();
            assert_fills(&l);
            let widths: Vec<u16> = l.rects().iter().map(|(_, r)| r.w).collect();
            assert!(
                widths.iter().max().unwrap() - widths.iter().min().unwrap() <= 1,
                "{cols}: {widths:?}"
            );
        }
        // A grid of two columns of two: the columns are equal too.
        let mut l = four_panes(101, 30);
        l.resize(2, Direction::Left, 5).unwrap();
        l.even().unwrap();
        assert_fills(&l);
        let rects = l.rects();
        let left = rects
            .iter()
            .filter(|(_, r)| r.x == 0)
            .map(|(_, r)| r.w)
            .max()
            .unwrap();
        let right = rects
            .iter()
            .filter(|(_, r)| r.x != 0)
            .map(|(_, r)| r.w)
            .max()
            .unwrap();
        assert!(left.abs_diff(right) <= 1, "{left} {right}");
    }

    // ---- Dragging a divider ----

    fn width_of(layout: &Layout, pane: PaneId) -> u16 {
        layout.rect_of(pane).unwrap().w
    }

    #[test]
    fn a_dragged_divider_lands_under_the_pointer_and_stops_at_the_minimum_sizes() {
        let mut layout = Layout::new(1, 81, 24);
        layout.split(1, Axis::LeftRight, 2).unwrap();
        let divider = layout.dividers()[0];
        let handle = layout.divider_at(divider.x, 5).unwrap();
        assert!(handle.vertical());
        assert_eq!(layout.drag_divider(&handle, 30), Ok(true));
        assert_eq!(layout.dividers()[0].x, 30);
        assert_eq!((width_of(&layout, 1), width_of(&layout, 2)), (30, 50));
        assert_covers(&layout);
        // The same place again moves nothing.
        assert_eq!(layout.drag_divider(&handle, 30), Ok(false));
        // Past the minimum on either side the divider stops at the limit, as the resize
        // commands refuse to go beyond it.
        assert_eq!(layout.drag_divider(&handle, 0), Ok(true));
        assert_eq!(width_of(&layout, 1), MIN_COLS);
        assert_eq!(layout.drag_divider(&handle, 80), Ok(true));
        assert_eq!(width_of(&layout, 2), MIN_COLS);
        assert_covers(&layout);
    }

    #[test]
    fn a_horizontal_divider_moves_by_rows_with_the_same_limits() {
        let mut layout = Layout::new(1, 80, 25);
        layout.split(1, Axis::TopBottom, 2).unwrap();
        let divider = layout.dividers()[0];
        let handle = layout.divider_at(10, divider.y).unwrap();
        assert!(!handle.vertical());
        assert_eq!(layout.drag_divider(&handle, 7), Ok(true));
        assert_eq!(layout.rect_of(1).unwrap().h, 7);
        assert_eq!(layout.drag_divider(&handle, 0), Ok(true));
        assert_eq!(layout.rect_of(1).unwrap().h, MIN_ROWS);
        assert_eq!(layout.drag_divider(&handle, 24), Ok(true));
        assert_eq!(layout.rect_of(2).unwrap().h, MIN_ROWS);
        assert_covers(&layout);
    }

    #[test]
    fn in_nested_splits_the_divider_that_was_pressed_is_the_one_that_moves() {
        // 1 | (2 over 3), and 3 split again into 3 | 4.
        let mut layout = Layout::new(1, 81, 25);
        layout.split(1, Axis::LeftRight, 2).unwrap();
        layout.split(2, Axis::TopBottom, 3).unwrap();
        layout.split(3, Axis::LeftRight, 4).unwrap();
        let before: Vec<_> = layout.rects();
        let dividers = layout.dividers();
        // The inner vertical divider (between 3 and 4) lies in the lower right.
        let inner = dividers
            .iter()
            .find(|d| d.vertical && d.x > dividers[0].x)
            .copied()
            .unwrap();
        let handle = layout.divider_at(inner.x, inner.y).unwrap();
        assert_eq!(layout.drag_divider(&handle, inner.x + 6), Ok(true));
        // Pane 1 and pane 2 did not move; 3 and 4 shared the change.
        assert_eq!(
            layout.rect_of(1),
            before.iter().find(|(p, _)| *p == 1).map(|(_, r)| *r)
        );
        assert_eq!(
            layout.rect_of(2),
            before.iter().find(|(p, _)| *p == 2).map(|(_, r)| *r)
        );
        assert_eq!(width_of(&layout, 3), width_of_before(&before, 3) + 6);
        assert_eq!(width_of(&layout, 4), width_of_before(&before, 4) - 6);
        // The outer vertical divider moves 1 and the whole right side, not the inner one.
        let outer = layout.divider_at(dividers[0].x, 0).unwrap();
        assert_ne!(outer, handle);
        let inner_ratio_before = layout.rect_of(3).unwrap().w;
        assert_eq!(layout.drag_divider(&outer, 20), Ok(true));
        assert_eq!(width_of(&layout, 1), 20);
        assert_covers(&layout);
        assert!(layout.rect_of(3).unwrap().w > 0 && inner_ratio_before > 0);
    }

    fn width_of_before(rects: &[(PaneId, Rect)], pane: PaneId) -> u16 {
        rects.iter().find(|(p, _)| *p == pane).unwrap().1.w
    }

    #[test]
    fn where_one_divider_ends_on_another_the_cell_belongs_to_the_one_it_ends_on() {
        // 1 | (2 over 3): the horizontal divider ends on the vertical one.
        let mut layout = Layout::new(1, 81, 25);
        layout.split(1, Axis::LeftRight, 2).unwrap();
        layout.split(2, Axis::TopBottom, 3).unwrap();
        let ds = layout.dividers();
        let vertical = ds.iter().find(|d| d.vertical).copied().unwrap();
        let horizontal = ds.iter().find(|d| !d.vertical).copied().unwrap();
        // The cell of the vertical divider next to where the horizontal one starts is the
        // vertical divider's; the first cell of the horizontal one is just right of it.
        let junction = layout.divider_at(vertical.x, horizontal.y).unwrap();
        assert!(junction.vertical());
        let first = layout.divider_at(horizontal.x, horizontal.y).unwrap();
        assert!(!first.vertical());
        // A cell inside a pane is no divider at all.
        assert!(layout.divider_at(2, 2).is_none());
    }

    #[test]
    fn a_handle_to_a_divider_that_is_gone_is_an_error() {
        let mut layout = Layout::new(1, 81, 24);
        layout.split(1, Axis::LeftRight, 2).unwrap();
        let handle = layout.divider_at(layout.dividers()[0].x, 3).unwrap();
        layout.close(2).unwrap();
        assert_eq!(
            layout.drag_divider(&handle, 10),
            Err(LayoutError::NothingThere)
        );
    }
}
