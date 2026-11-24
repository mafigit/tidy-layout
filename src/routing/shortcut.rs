// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.

//! The stage `ShortcutEdgeRoutes`, after `NudgeEdgeChannels`: a route without a label loses
//! bends where a part of it can be replaced by one corner (an L) that is no longer, keeps its
//! ends and the way it leaves and arrives, enters no card and no label, keeps its distance to
//! the sides of cards and to other routes, and touches other routes only where the old route
//! did. The change is kept unless it makes what TALA counts on the drawing worse. Ported from
//! TALA `internal/routing/shortcut_routes.go` (`shortcutRoutesGuarded` and its helpers), with
//! `changedRouteIsClear` (`port_interior_balance.go`). Without containers, curved routes and
//! labels of cards.

use crate::graph::{Graph, Node, Point};
use crate::quality::{LabelBox, inspect};
use crate::routing::labels::{LabelSize, Placed};
use crate::routing::nudge::{changed_route_is_clear, label_boxes, same_direction};
use crate::routing::simplify::enters;
use crate::routing::tuning::SEGMENT_SPACING_BUFFER;

/// TALA `maxShortcutInput`: larger diagrams are left as they are.
const MOST_INPUT: usize = 256;
/// TALA `maxShortcutRoute`: a route with more points is left as it is.
const MOST_POINTS: usize = 32;
/// TALA `shortcutEpsilon`.
const EPSILON: f64 = 1e-6;

/// TALA `shortcutOrthogonal`: every piece upright or level and of some length, and no turn
/// back.
fn orthogonal(points: &[Point]) -> bool {
    for i in 0..points.len() {
        let p = points[i];
        if !p.x.is_finite() || !p.y.is_finite() {
            return false;
        }
        if i > 0 && (p == points[i - 1] || (p.x != points[i - 1].x && p.y != points[i - 1].y)) {
            return false;
        }
        if i > 1 && !same_direction(points[i - 2], points[i - 1], points[i - 1], p) && (points[i - 2].x == p.x || points[i - 2].y == p.y) {
            return false;
        }
    }
    points.len() >= 2
}

/// TALA `shortcutLength`.
fn length(points: &[Point]) -> f64 {
    points.windows(2).fold(0.0, |sum, piece| sum + (piece[1].x - piece[0].x).abs() + (piece[1].y - piece[0].y).abs())
}

/// TALA `shortcutBends`.
fn bends(points: &[Point]) -> usize {
    points.windows(3).filter(|three| (three[1].x - three[0].x) * (three[2].y - three[1].y) != (three[1].y - three[0].y) * (three[2].x - three[1].x)).count()
}

/// TALA `shortcutCandidate`: the points from `start` to `end` replaced by one corner, level
/// first or upright first. Repeated points and points in a straight line go.
fn candidate(points: &[Point], start: usize, end: usize, horizontal: bool) -> Vec<Point> {
    let elbow = if horizontal { Point { x: points[end].x, y: points[start].y } } else { Point { x: points[start].x, y: points[end].y } };
    let mut result: Vec<Point> = Vec::new();
    for &p in points[..=start].iter().chain([&elbow]).chain(&points[end..]) {
        if result.last() == Some(&p) {
            continue;
        }
        while result.len() >= 2 && same_direction(result[result.len() - 2], result[result.len() - 1], result[result.len() - 1], p) {
            result.pop();
        }
        result.push(p);
    }
    result
}

/// TALA `channelCoordinate`.
fn coordinate(p: Point, horizontal: bool) -> f64 {
    if horizontal { p.x } else { p.y }
}

/// TALA `shortcutPointOnSegment`.
fn on_segment(p: Point, a: Point, b: Point) -> bool {
    ((a.x == b.x && p.x == a.x) || (a.y == b.y && p.y == a.y))
        && p.x >= a.x.min(b.x) - EPSILON
        && p.x <= a.x.max(b.x) + EPSILON
        && p.y >= a.y.min(b.y) - EPSILON
        && p.y <= a.y.max(b.y) + EPSILON
}

/// TALA `shortcutSegmentsMeet`: the closed boxes of two upright or level pieces overlap.
fn meet(a: Point, b: Point, c: Point, d: Point) -> bool {
    a.x.min(b.x).max(c.x.min(d.x)) <= a.x.max(b.x).min(c.x.max(d.x)) && a.y.min(b.y).max(c.y.min(d.y)) <= a.y.max(b.y).min(c.y.max(d.y))
}

/// TALA `shortcutThroughRays`: whether the route runs through the point from left to right,
/// and from top to bottom.
fn through_rays(points: &[Point], p: Point) -> (bool, bool) {
    let (mut left, mut right, mut above, mut below) = (false, false, false, false);
    for piece in points.windows(2) {
        let (a, b) = (piece[0], piece[1]);
        if !on_segment(p, a, b) {
            continue;
        }
        if a.y == b.y {
            left |= a.x.min(b.x) < p.x;
            right |= a.x.max(b.x) > p.x;
        } else {
            above |= a.y.min(b.y) < p.y;
            below |= a.y.max(b.y) > p.y;
        }
    }
    (left && right, above && below)
}

/// TALA `shortcutContactsPreserved`: where the new route meets another, the old one met it
/// there too, and it crosses it only where the old one did; where it lies on it, the old one
/// lay on all of that stretch.
fn contacts_preserved(before: &[Point], after: &[Point], other: &[Point]) -> bool {
    for piece in after.windows(2) {
        for other_piece in other.windows(2) {
            let (a, b, c, d) = (piece[0], piece[1], other_piece[0], other_piece[1]);
            if !meet(a, b, c, d) {
                continue;
            }
            if (a.x == b.x) != (c.x == d.x) {
                let p = if a.y == b.y { Point { x: c.x, y: a.y } } else { Point { x: a.x, y: c.y } };
                let covered = before.windows(2).any(|old| on_segment(p, old[0], old[1]));
                let (old_h, old_v) = through_rays(before, p);
                let (new_h, new_v) = through_rays(after, p);
                let (other_h, other_v) = through_rays(other, p);
                let new_crossing = (new_h && other_v) || (new_v && other_h);
                let old_crossing = (old_h && other_v) || (old_v && other_h);
                if !covered || (new_crossing && !old_crossing) {
                    return false;
                }
                continue;
            }
            let horizontal = a.y == b.y;
            let position = if horizontal { a.y } else { a.x };
            let along = |p: Point| coordinate(p, horizontal);
            let lo = along(a).min(along(b)).max(along(c).min(along(d)));
            let hi = along(a).max(along(b)).min(along(c).max(along(d)));
            let mut cursor = lo;
            loop {
                let (mut next, mut contains) = (cursor, false);
                for old in before.windows(2) {
                    let (u, v) = (old[0], old[1]);
                    if (horizontal && (u.y != position || v.y != position)) || (!horizontal && (u.x != position || v.x != position)) {
                        continue;
                    }
                    let (left, right) = (along(u).min(along(v)), along(u).max(along(v)));
                    if left <= cursor + EPSILON && right >= cursor - EPSILON {
                        contains = true;
                        next = next.max(right);
                    }
                }
                if contains && next >= hi - EPSILON {
                    break;
                }
                if next <= cursor + EPSILON {
                    return false;
                }
                cursor = next;
            }
        }
    }
    true
}

/// TALA `shortcutWallClearance`: a new piece keeps the spacing to every side of a card it runs
/// along, or at least the distance the old route kept on that side of it.
fn wall_clearance(graph: &Graph, before: &[Point], a: Point, b: Point) -> bool {
    let horizontal = a.y == b.y;
    let position = coordinate(a, !horizontal);
    let (lo, hi) = (coordinate(a, horizontal).min(coordinate(b, horizontal)), coordinate(a, horizontal).max(coordinate(b, horizontal)));
    for node in &graph.nodes {
        let nlo = coordinate(node.top_left, horizontal);
        let (nhi, walls) = if horizontal { (nlo + node.width, [node.top_left.y, node.top_left.y + node.height]) } else { (nlo + node.height, [node.top_left.x, node.top_left.x + node.width]) };
        if hi.min(nhi) - lo.max(nlo) <= EPSILON {
            continue;
        }
        for wall in walls {
            let gap = (position - wall).abs();
            if gap >= SEGMENT_SPACING_BUFFER - EPSILON {
                continue;
            }
            let mut minimum = SEGMENT_SPACING_BUFFER;
            for old in before.windows(2) {
                let (u, v) = (old[0], old[1]);
                let old_position = coordinate(u, !horizontal);
                if old_position != coordinate(v, !horizontal) || (old_position - wall) * (position - wall) < 0.0 {
                    continue;
                }
                let (old_lo, old_hi) = (coordinate(u, horizontal).min(coordinate(v, horizontal)), coordinate(u, horizontal).max(coordinate(v, horizontal)));
                if old_hi.min(nhi) - old_lo.max(nlo) > EPSILON {
                    minimum = minimum.min((old_position - wall).abs());
                }
            }
            if gap + EPSILON < minimum {
                return false;
            }
        }
    }
    true
}

/// TALA `shortcutParallelClearance`: a new piece keeps the spacing to every piece of another
/// route it runs beside, or at least the distance the old route kept to it.
fn parallel_clearance(routes: &[Vec<Point>], edge: usize, a: Point, b: Point) -> bool {
    let horizontal = a.y == b.y;
    let position = coordinate(a, !horizontal);
    let (lo, hi) = (coordinate(a, horizontal).min(coordinate(b, horizontal)), coordinate(a, horizontal).max(coordinate(b, horizontal)));
    let own = &routes[edge];
    for (other, points) in routes.iter().enumerate() {
        if other == edge {
            continue;
        }
        for piece in points.windows(2) {
            let (c, d) = (piece[0], piece[1]);
            let wall = coordinate(c, !horizontal);
            if wall != coordinate(d, !horizontal) {
                continue;
            }
            let (other_lo, other_hi) = (coordinate(c, horizontal).min(coordinate(d, horizontal)), coordinate(c, horizontal).max(coordinate(d, horizontal)));
            if hi.min(other_hi) - lo.max(other_lo) <= EPSILON {
                continue;
            }
            let gap = (position - wall).abs();
            if gap <= EPSILON || gap >= SEGMENT_SPACING_BUFFER - EPSILON {
                continue;
            }
            let mut minimum = SEGMENT_SPACING_BUFFER;
            for old in own.windows(2) {
                let (u, v) = (old[0], old[1]);
                let old_position = coordinate(u, !horizontal);
                if old_position != coordinate(v, !horizontal) {
                    continue;
                }
                let (old_lo, old_hi) = (coordinate(u, horizontal).min(coordinate(v, horizontal)), coordinate(u, horizontal).max(coordinate(v, horizontal)));
                let old_gap = (old_position - wall).abs();
                if old_gap > EPSILON && old_hi.min(other_hi) - old_lo.max(other_lo) > EPSILON {
                    minimum = minimum.min(old_gap);
                }
            }
            if gap + EPSILON < minimum {
                return false;
            }
        }
    }
    true
}

/// TALA `shortcutCandidateSafe`.
fn candidate_safe(graph: &Graph, routes: &[Vec<Point>], edge: usize, points: &[Point], labels: &[LabelBox]) -> bool {
    let before = &routes[edge];
    if !orthogonal(points) || bends(points) >= bends(before) || length(points) > length(before) + EPSILON {
        return false;
    }
    if points[0] != before[0] || points[points.len() - 1] != before[before.len() - 1] || !changed_route_is_clear(graph, points, before) {
        return false;
    }
    for i in 1..points.len() {
        let (a, b) = (points[i - 1], points[i]);
        let piece_length = (b.x - a.x).abs() + (b.y - a.y).abs();
        let mut minimum = SEGMENT_SPACING_BUFFER;
        let unchanged = before.windows(2).any(|old| a == old[0] && b == old[1]);
        if i == 1 {
            minimum = minimum.min(length(&before[..2]));
        }
        if i == points.len() - 1 {
            minimum = minimum.min(length(&before[before.len() - 2..]));
        }
        if !unchanged && piece_length + EPSILON < minimum {
            return false;
        }
        if (i + 2..points.len()).any(|j| meet(a, b, points[j - 1], points[j])) {
            return false;
        }
        if unchanged {
            continue;
        }
        if labels.iter().any(|label| enters(&Node { top_left: label.top_left, width: label.width, height: label.height }, a, b)) {
            return false;
        }
        if !wall_clearance(graph, before, a, b) || !parallel_clearance(routes, edge, a, b) {
            return false;
        }
    }
    routes.iter().enumerate().filter(|(other, _)| *other != edge).all(|(_, other)| contacts_preserved(before, points, other))
}

/// TALA `ShortcutEdgeRoutes`: the connections in their order, each with the routes the ones
/// before it left. Only connections without a label and outside trees; a route of five points
/// or more. Every candidate corner is tried, from every point to every later one but two, level
/// first and upright first; the one with the fewest bends wins, the shortest among those, the
/// first among equals. TALA stops the whole stage when a route is not upright and level all
/// through (a slanted line).
pub fn shortcut(graph: &Graph, routes: &mut [Vec<Point>], labels: &[Option<Placed>], sizes: &[Option<LabelSize>]) {
    // TALA `shortcutInputTooLarge`.
    let mut remaining = MOST_INPUT;
    if graph.nodes.len() > MOST_INPUT || routes.len() > MOST_INPUT {
        return;
    }
    for route in routes.iter() {
        if route.len() > remaining {
            return;
        }
        remaining -= route.len();
    }
    let mut boxes: Option<Vec<LabelBox>> = None;
    for edge in 0..routes.len() {
        let points = routes[edge].clone();
        if graph.hangs_in_tree(edge) || sizes[edge].is_some() || points.len() < 5 || points.len() > MOST_POINTS || !orthogonal(&points) {
            continue;
        }
        if routes.iter().any(|other| !orthogonal(other)) {
            return;
        }
        let boxes = boxes.get_or_insert_with(|| label_boxes(routes, labels, sizes).into_iter().flatten().collect());
        let (mut best, before_bends) = (points.clone(), bends(&points));
        let (mut best_length, mut best_bends) = (length(&best), before_bends);
        for start in 0..points.len().saturating_sub(3) {
            for end in start + 3..points.len() {
                for horizontal in [true, false] {
                    let next = candidate(&points, start, end, horizontal);
                    let (next_length, next_bends) = (length(&next), bends(&next));
                    if next_bends >= before_bends || next_bends > best_bends || next_length > length(&points) + EPSILON || (next_bends == best_bends && next_length >= best_length - EPSILON) {
                        continue;
                    }
                    if candidate_safe(graph, routes, edge, &next, boxes) {
                        (best, best_length, best_bends) = (next, next_length, next_bends);
                    }
                }
            }
        }
        if best_bends == before_bends {
            continue;
        }
        // What TALA counts on the drawing must not get worse.
        let before = inspect(graph, routes, &label_boxes(routes, labels, sizes));
        routes[edge] = best;
        let after = inspect(graph, routes, &label_boxes(routes, labels, sizes));
        if after.route_obstructions > before.route_obstructions || after.crossings > before.crossings || after.text_occlusions > before.text_occlusions || after.route_length > before.route_length + EPSILON {
            routes[edge] = points;
        }
    }
}
