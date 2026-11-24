// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.

//! Ported from TALA `internal/layoutgraph/node.go` (`segmentIntersectsBox`, `isPointNear`,
//! `orientation`, `isVisibilityGraphCandidate`, `distanceTo`), for plain boxes, and D2 `lib/geo/math.go`.

use crate::graph::{Node, Point};
use crate::routing::tuning::MIN_ROUTE_NODE_CLEARANCE;

/// Where a card lies, seen from another one. TALA `geo.Orientation`: "Left" means the card is to
/// the left of the other one.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Orientation {
    TopLeft,
    TopRight,
    BottomLeft,
    BottomRight,
    Top,
    Right,
    Bottom,
    Left,
    None,
}

impl Orientation {
    /// D2 `Orientation.GetOpposite`.
    pub fn opposite(self) -> Orientation {
        match self {
            Orientation::TopLeft => Orientation::BottomRight,
            Orientation::TopRight => Orientation::BottomLeft,
            Orientation::BottomLeft => Orientation::TopRight,
            Orientation::BottomRight => Orientation::TopLeft,
            Orientation::Top => Orientation::Bottom,
            Orientation::Bottom => Orientation::Top,
            Orientation::Left => Orientation::Right,
            Orientation::Right => Orientation::Left,
            Orientation::None => Orientation::None,
        }
    }
}

/// D2 `geo.EuclideanDistance`: exact along a row or a column.
pub fn euclidean_distance(x1: f64, y1: f64, x2: f64, y2: f64) -> f64 {
    if x1 == x2 {
        (y1 - y2).abs()
    } else if y1 == y2 {
        (x1 - x2).abs()
    } else {
        ((x1 - x2) * (x1 - x2) + (y1 - y2) * (y1 - y2)).sqrt()
    }
}

/// TALA `intersects` (`internal/routing/geometry.go`): two segments cross, touch, or lie on one
/// line and overlap or touch there.
pub fn segments_intersect(first_start: Point, first_end: Point, second_start: Point, second_end: Point) -> bool {
    // Positive for a turn to the right, negative to the left, 0 for three points on one line.
    let side = |a: Point, b: Point, c: Point| (b.y - a.y) * (c.x - a.x) - (b.x - a.x) * (c.y - a.y);
    let second_start_side = side(first_start, first_end, second_start);
    let second_end_side = side(first_start, first_end, second_end);
    let first_start_side = side(second_start, second_end, first_start);
    let first_end_side = side(second_start, second_end, first_end);
    if second_start_side == 0.0 && second_end_side == 0.0 && first_start_side == 0.0 && first_end_side == 0.0 {
        let overlap = |a: f64, b: f64, c: f64, d: f64| a.min(b) <= c.max(d) && c.min(d) <= a.max(b);
        return overlap(first_start.x, first_end.x, second_start.x, second_end.x)
            && overlap(first_start.y, first_end.y, second_start.y, second_end.y);
    }
    let straddles = |one: f64, other: f64| one == 0.0 || other == 0.0 || (one < 0.0) != (other < 0.0);
    straddles(second_start_side, second_end_side) && straddles(first_start_side, first_end_side)
}

/// D2 `Orientation.IsDiagonal`.
pub fn is_diagonal(orientation: Orientation) -> bool {
    matches!(
        orientation,
        Orientation::TopLeft | Orientation::TopRight | Orientation::BottomLeft | Orientation::BottomRight
    )
}

impl Node {
    /// TALA `distanceTo` with sizes: the shortest distance between two cards, 0 when they touch.
    pub fn distance_to(&self, other: &Node) -> f64 {
        let gap = |a_start: f64, a_end: f64, b_start: f64, b_end: f64| {
            if a_end < b_start {
                b_start - a_end
            } else if b_end < a_start {
                a_start - b_end
            } else {
                0.0
            }
        };
        let dx = gap(self.top_left.x, self.right(), other.top_left.x, other.right());
        let dy = gap(self.top_left.y, self.bottom(), other.top_left.y, other.bottom());
        euclidean_distance(0.0, 0.0, dx, dy)
    }

    pub fn right(&self) -> f64 {
        self.top_left.x + self.width
    }

    pub fn bottom(&self) -> f64 {
        self.top_left.y + self.height
    }

    pub fn center(&self) -> Point {
        Point { x: self.top_left.x + self.width / 2.0, y: self.top_left.y + self.height / 2.0 }
    }

    /// TALA `isPointNear`: inside the card or within the clearance around it, borders included.
    pub fn is_point_near(&self, point: Point) -> bool {
        self.top_left.x - MIN_ROUTE_NODE_CLEARANCE <= point.x
            && self.right() + MIN_ROUTE_NODE_CLEARANCE >= point.x
            && self.top_left.y - MIN_ROUTE_NODE_CLEARANCE <= point.y
            && self.bottom() + MIN_ROUTE_NODE_CLEARANCE >= point.y
    }

    /// TALA `passesThrough`: the segment crosses the card or runs along its border. Touching the
    /// card in one point from outside does not count.
    pub fn passes_through(&self, first: Point, second: Point) -> bool {
        let (left, right) = (self.top_left.x, self.right());
        let (top, bottom) = (self.top_left.y, self.bottom());
        if (first.x < left && second.x < left)
            || (first.x > right && second.x > right)
            || (first.y < top && second.y < top)
            || (first.y > bottom && second.y > bottom)
        {
            return false;
        }
        let contains = |p: Point| left <= p.x && p.x <= right && top <= p.y && p.y <= bottom;
        if contains(first) || contains(second) {
            return true;
        }
        // Clip the segment against the box; both ends are outside here.
        let mut enter = 0.0_f64;
        let mut exit = 1.0_f64;
        let mut clip = |start: f64, delta: f64, min: f64, max: f64| {
            if delta == 0.0 {
                return min <= start && start <= max;
            }
            let mut one = (min - start) / delta;
            let mut two = (max - start) / delta;
            if one > two {
                std::mem::swap(&mut one, &mut two);
            }
            enter = enter.max(one);
            exit = exit.min(two);
            enter <= exit
        };
        if !clip(first.x, second.x - first.x, left, right)
            || !clip(first.y, second.y - first.y, top, bottom)
        {
            return false;
        }
        enter < exit
    }

    /// TALA `orientation`: where this card lies, seen from `other`.
    pub fn orientation(&self, other: &Node) -> Orientation {
        if self.bottom() < other.top_left.y {
            if self.right() < other.top_left.x {
                return Orientation::TopLeft;
            }
            if other.right() < self.top_left.x {
                return Orientation::TopRight;
            }
            return Orientation::Top;
        }
        if other.bottom() < self.top_left.y {
            if self.right() < other.top_left.x {
                return Orientation::BottomLeft;
            }
            if other.right() < self.top_left.x {
                return Orientation::BottomRight;
            }
            return Orientation::Bottom;
        }
        if other.right() < self.top_left.x {
            return Orientation::Right;
        }
        if self.right() < other.top_left.x {
            return Orientation::Left;
        }
        Orientation::None
    }

    /// TALA `isVisibilityGraphCandidate` with sizes and without a side check or padding: the two
    /// cards share a stretch of rows (`horizontal`) or of columns.
    pub fn faces(&self, other: &Node, horizontal: bool) -> bool {
        if horizontal {
            self.top_left.y <= other.bottom() && self.bottom() >= other.top_left.y
        } else {
            self.top_left.x <= other.right() && self.right() >= other.top_left.x
        }
    }
}
