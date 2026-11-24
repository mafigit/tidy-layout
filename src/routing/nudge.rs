// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.

//! The stage that nudges channels: pieces of routes that lie on one line and can move
//! between a wall on either side (a card, a piece that keeps its place) are moved so that the
//! routes get shorter or the gaps between them wider. A change stays only if no route then
//! runs through a card, no crossing is added, no label is covered and the routes are no longer.
//! Ported from TALA `internal/routing`: `nudge_channels.go` (`NudgeEdgeChannels`,
//! `nudgeChannelsGuarded`, `channelInputTooLarge`, `buildChannelProblem`, `solveChannelGap`,
//! `channelGap`, `applyChannelProblem`, `channelPointProposal`, `channelWireLength`,
//! `channelProposalSafe`), `port_interior_balance.go` (`changedRouteIsClear`),
//! `edge_simplify.go` (`sameRouteDirection`, `lineIntersectsUnrelatedNode`) and
//! `balance_route_guard.go` (`removeDuplicatePointsGuarded`, `nodeSegmentsGuarded`).
//!
//! TALA's words: the pieces on one line that overlap are a "group", with one "position". Two
//! groups side by side are joined by an "arc": the first must stay before the second.

use crate::graph::{Graph, Point};
use crate::placementcost::flow_continuity::hypot;
use crate::quality::{LabelBox, inspect};
use crate::routing::balance::{Axis, Piece, bounds, collinear_overlap, ends, is_special, overlaps, pieces_of};
use crate::routing::labels::{LabelSize, Placed, label_top_left};
use crate::routing::line::crosses;
use crate::routing::simplify::enters;
use crate::routing::swap_ports::intersection;
use crate::routing::tuning::SEGMENT_SPACING_BUFFER;

/// TALA `channelEpsilon`.
const EPSILON: f64 = 1e-6;
/// TALA `maxChannelSegments`: larger diagrams are left as they are.
const MOST_SEGMENTS: usize = 256;

/// TALA `channelGroup`.
struct Group {
    pieces: Vec<Piece>,
    position: f64,
    lower: f64,
    upper: f64,
    fixed: bool,
    /// How near the group may come to a card on either side.
    clearance: (f64, f64),
}

/// TALA `channelArc`. Pieces of one route may meet (`separate` is false): a bend can go.
#[derive(Clone, Copy)]
struct Arc {
    from: usize,
    to: usize,
    separate: bool,
}

/// TALA `buildChannelProblem`: the groups that can move, in the order of their positions, and
/// the arcs between them.
fn build_problem(graph: &Graph, routes: &[Vec<Point>], axis: Axis) -> (Vec<Group>, Vec<Arc>) {
    let all: Vec<usize> = (0..routes.len()).collect();
    let pieces = pieces_of(routes, &all, axis);
    // A route with a slanted piece or a piece without length keeps its place, and so does one
    // whose ends are a pixel out of line, and a connection of a card of a tree. (Curves,
    // loops and tables: not for plain cards.)
    let locked: Vec<bool> = routes.iter().enumerate().map(|(edge, route)| route.len() >= 2 && (is_special(graph, edge, route) || route.windows(2).any(|piece| (piece[0].x != piece[1].x && piece[0].y != piece[1].y) || piece[0] == piece[1]))).collect();
    // The two ends of a route keep their place.
    let at_an_end = |piece: &Piece| [piece.start, piece.end].iter().any(|point| *point == 0 || *point == routes[piece.edge].len() - 1);
    let mut parent: Vec<usize> = (0..pieces.len()).collect();
    fn root(parent: &mut [usize], mut i: usize) -> usize {
        while parent[i] != i {
            parent[i] = parent[parent[i]];
            i = parent[i];
        }
        i
    }
    for i in 0..pieces.len() {
        for j in 0..i {
            let (s, other) = (ends(routes, pieces[i]), ends(routes, pieces[j]));
            if axis.across(s.0) == axis.across(other.0) && overlaps(s, other, axis, SEGMENT_SPACING_BUFFER) {
                let (a, b) = (root(&mut parent, i), root(&mut parent, j));
                parent[a] = b;
            }
        }
    }
    let mut group_of_root: Vec<Option<usize>> = vec![None; pieces.len()];
    let mut groups: Vec<Group> = Vec::new();
    for (i, piece) in pieces.iter().enumerate() {
        let at = root(&mut parent, i);
        let group = *group_of_root[at].get_or_insert_with(|| {
            groups.push(Group { pieces: Vec::new(), position: axis.across(ends(routes, *piece).0), lower: 0.0, upper: 0.0, fixed: false, clearance: (f64::NEG_INFINITY, f64::INFINITY) });
            groups.len() - 1
        });
        groups[group].pieces.push(*piece);
        groups[group].fixed = groups[group].fixed || locked[piece.edge] || at_an_end(piece);
        // (TALA also fixes a group that ends at a card of one pixel. Cards are larger.)
    }
    // TALA `nodeSegmentsGuarded`: the two sides of every card that lie along the pieces.
    let walls: Vec<(Point, Point)> = graph
        .nodes
        .iter()
        .flat_map(|node| {
            let (left, top, right, bottom) = (node.top_left.x, node.top_left.y, node.right(), node.bottom());
            if axis.sideways {
                [(Point { x: left, y: top }, Point { x: left, y: bottom }), (Point { x: right, y: top }, Point { x: right, y: bottom })]
            } else {
                [(Point { x: left, y: top }, Point { x: right, y: top }), (Point { x: left, y: bottom }, Point { x: right, y: bottom })]
            }
        })
        .collect();
    for group in groups.iter_mut().filter(|group| !group.fixed) {
        // A gap to a card that is short already stays; a larger one may shrink to the spacing.
        for piece in &group.pieces {
            let (lower, upper) = bounds(ends(routes, *piece), &walls);
            if lower != f64::NEG_INFINITY {
                group.clearance.0 = group.clearance.0.max(lower + (group.position - lower).min(SEGMENT_SPACING_BUFFER));
            }
            if upper != f64::INFINITY {
                group.clearance.1 = group.clearance.1.min(upper - (upper - group.position).min(SEGMENT_SPACING_BUFFER));
            }
        }
    }
    let mut fixed = walls;
    for group in groups.iter().filter(|group| group.fixed) {
        fixed.extend(group.pieces.iter().map(|piece| ends(routes, *piece)));
    }
    // A group needs a wall on both sides. One without becomes a wall itself, twice over.
    for _ in 0..2 {
        for group in groups.iter_mut() {
            if group.fixed {
                continue;
            }
            (group.lower, group.upper) = (f64::NEG_INFINITY, f64::INFINITY);
            for piece in &group.pieces {
                let (low, high) = bounds(ends(routes, *piece), &fixed);
                (group.lower, group.upper) = (group.lower.max(low), group.upper.min(high));
            }
            if group.lower.is_infinite() || group.upper.is_infinite() || group.lower > group.position || group.upper < group.position || group.lower >= group.upper {
                group.fixed = true;
                fixed.extend(group.pieces.iter().map(|piece| ends(routes, *piece)));
            }
        }
    }
    let mut free: Vec<Group> = groups.into_iter().filter(|group| !group.fixed).collect();
    // TALA's sort keeps groups at the same position in order.
    free.sort_by(|a, b| a.position.partial_cmp(&b.position).unwrap_or(std::cmp::Ordering::Equal));
    let mut arcs = Vec::new();
    for i in 0..free.len() {
        for j in i + 1..free.len() {
            let owner = free[i].pieces[0].edge;
            let same_owner = free[i].pieces.iter().chain(&free[j].pieces).all(|piece| piece.edge == owner);
            let buffer = if same_owner { 0.0 } else { SEGMENT_SPACING_BUFFER };
            let linked = free[i].pieces.iter().any(|s| free[j].pieces.iter().any(|t| overlaps(ends(routes, *s), ends(routes, *t), axis, buffer)));
            if linked {
                arcs.push(Arc { from: i, to: j, separate: !same_owner });
            }
        }
    }
    (free, arcs)
}

/// TALA `solveChannelGap`: the earliest and the latest positions that keep `gap` to the walls
/// and between groups joined by an arc. `None` where there are none.
fn solve_gap(groups: &[&Group], arcs: &[Arc], gap: f64) -> Option<(Vec<f64>, Vec<f64>)> {
    let mut low: Vec<f64> = groups.iter().map(|group| (group.lower + gap).max(group.clearance.0)).collect();
    let mut high: Vec<f64> = groups.iter().map(|group| (group.upper - gap).min(group.clearance.1)).collect();
    let separation = |arc: &Arc| if arc.separate { gap } else { 0.0 };
    for arc in arcs {
        low[arc.to] = low[arc.to].max(low[arc.from] + separation(arc));
    }
    for arc in arcs.iter().rev() {
        high[arc.from] = high[arc.from].min(high[arc.to] - separation(arc));
    }
    low.iter().zip(&high).all(|(low, high)| low <= high).then_some((low, high))
}

/// TALA `channelGap`: the smallest gap the positions leave.
fn channel_gap(groups: &[&Group], arcs: &[Arc], positions: &[f64]) -> f64 {
    let mut gap = f64::INFINITY;
    for (group, position) in groups.iter().zip(positions) {
        gap = gap.min((position - group.lower).min(group.upper - position));
    }
    for arc in arcs.iter().filter(|arc| arc.separate) {
        gap = gap.min(positions[arc.to] - positions[arc.from]);
    }
    gap
}

/// The points of the routes a proposal moves, and where to. TALA `channelPointProposal` keeps
/// them by the point; here they are kept per route, by the point's place in it.
type Proposal = Vec<Vec<Option<Point>>>;

/// TALA `channelPointProposal`: every piece of a group at the group's new position. `None`
/// where a point would have to go to two places.
fn point_proposal(routes: &[Vec<Point>], groups: &[&Group], positions: &[f64], axis: Axis) -> Option<Proposal> {
    let mut proposal: Proposal = routes.iter().map(|route| vec![None; route.len()]).collect();
    for (group, position) in groups.iter().zip(positions) {
        for piece in &group.pieces {
            for point in [piece.start, piece.end] {
                let mut candidate = routes[piece.edge][point];
                axis.set_across(&mut candidate, *position);
                if proposal[piece.edge][point].is_some_and(|old| old != candidate) {
                    return None;
                }
                proposal[piece.edge][point] = Some(candidate);
            }
        }
    }
    Some(proposal)
}

fn proposed(routes: &[Vec<Point>], proposal: &Proposal) -> Vec<Vec<Point>> {
    routes.iter().zip(proposal).map(|(route, moved)| route.iter().zip(moved).map(|(point, to)| to.unwrap_or(*point)).collect()).collect()
}

/// TALA `channelWireLength`.
fn wire_length(routes: &[Vec<Point>]) -> f64 {
    let mut length = 0.0;
    for route in routes {
        for piece in route.windows(2) {
            length += hypot(piece[1].x - piece[0].x, piece[1].y - piece[0].y);
        }
    }
    length
}

/// TALA `sameRouteDirection`: the two pieces point the same way along parallel lines.
pub(crate) fn same_direction(a: Point, b: Point, c: Point, d: Point) -> bool {
    (b.x - a.x) * (d.x - c.x) + (b.y - a.y) * (d.y - c.y) > 0.0 && (b.x - a.x) * (d.y - c.y) == (b.y - a.y) * (d.x - c.x)
}

/// TALA `changedRouteIsClear` with fixed ports: the route keeps its two ends and the way it
/// leaves and arrives, and every piece that is new is upright or level and runs through no card.
pub(crate) fn changed_route_is_clear(graph: &Graph, now: &[Point], before: &[Point]) -> bool {
    if now.len() < 2 || before.len() < 2 {
        return false;
    }
    // (TALA also asks that the two ends are where they were. A piece at an end of a route is
    // in a group that keeps its place, so they are.)
    let (last, last_before) = (now.len() - 1, before.len() - 1);
    if !same_direction(before[0], before[1], now[0], now[1]) || !same_direction(before[last_before - 1], before[last_before], now[last - 1], now[last]) {
        return false;
    }
    now.windows(2).all(|piece| {
        let (a, b) = (piece[0], piece[1]);
        if before.windows(2).any(|old| old[0] == a && old[1] == b) {
            return true;
        }
        (a.x == b.x || a.y == b.y) && !graph.nodes.iter().any(|node| node.width > 0.0 && node.height > 0.0 && enters(node, a, b))
    })
}

/// D2 `Segment.Intersects`.
fn meet(a: Point, b: Point, c: Point, d: Point) -> bool {
    intersection(a, b, c, d).is_some()
}

/// TALA `channelProposalSafe`: no changed route runs through a card, gets longer, gets a piece
/// shorter than the spacing, turns back on itself or meets itself; and no two routes cross more
/// often, come to lie on each other or touch where they did not.
fn proposal_safe(graph: &Graph, routes: &[Vec<Point>], proposal: &Proposal) -> bool {
    let candidates = proposed(routes, proposal);
    let changed: Vec<bool> = routes.iter().zip(&candidates).map(|(route, candidate)| route != candidate).collect();
    for ((route, candidate), _) in routes.iter().zip(&candidates).zip(&changed).filter(|(_, changed)| **changed) {
        if !changed_route_is_clear(graph, candidate, route) {
            return false;
        }
        let (mut before_length, mut after_length) = (0.0, 0.0);
        let mut nonzero: Vec<Point> = Vec::new();
        for (i, point) in candidate.iter().enumerate() {
            if !point.x.is_finite() || !point.y.is_finite() {
                return false;
            }
            if i > 0 {
                let before = hypot(route[i].x - route[i - 1].x, route[i].y - route[i - 1].y);
                let after = hypot(point.x - candidate[i - 1].x, point.y - candidate[i - 1].y);
                before_length += before;
                after_length += after;
                if after > EPSILON && after + EPSILON < before.min(SEGMENT_SPACING_BUFFER) {
                    return false;
                }
            }
            if nonzero.last() != Some(point) {
                nonzero.push(*point);
            }
        }
        if after_length > before_length + EPSILON {
            return false;
        }
        if nonzero.windows(3).any(|three| (three[1].x - three[0].x) * (three[2].x - three[1].x) + (three[1].y - three[0].y) * (three[2].y - three[1].y) < 0.0) {
            return false;
        }
        for i in 0..nonzero.len().saturating_sub(1) {
            for j in i + 2..nonzero.len().saturating_sub(1) {
                if meet(nonzero[i], nonzero[i + 1], nonzero[j], nonzero[j + 1]) {
                    return false;
                }
            }
        }
    }
    for i in 0..routes.len() {
        for j in i + 1..routes.len() {
            // (Two routes that both stay pass every question below: this only saves work.)
            if !changed[i] && !changed[j] {
                continue;
            }
            let (mut before, mut after) = (0, 0);
            for a in 0..routes[i].len().saturating_sub(1) {
                for b in 0..routes[j].len().saturating_sub(1) {
                    let (old, other_old) = ((routes[i][a], routes[i][a + 1]), (routes[j][b], routes[j][b + 1]));
                    let (new, other_new) = ((candidates[i][a], candidates[i][a + 1]), (candidates[j][b], candidates[j][b + 1]));
                    before += usize::from(crosses(old.0, old.1, other_old.0, other_old.1));
                    after += usize::from(crosses(new.0, new.1, other_new.0, other_new.1));
                    if collinear_overlap(old.0, old.1, other_old.0, other_old.1) == 0.0 && collinear_overlap(new.0, new.1, other_new.0, other_new.1) > 0.0 {
                        return false;
                    }
                    if !meet(old.0, old.1, other_old.0, other_old.1) && meet(new.0, new.1, other_new.0, other_new.1) {
                        return false;
                    }
                }
            }
            if after > before {
                return false;
            }
        }
    }
    true
}

/// TALA `applyChannelProblem`: the groups that hang together by arcs are solved together. Six
/// answers are tried: the gap as it is and the largest gap that fits, each with all groups as
/// early as they can be, as late, and in the middle. An answer is taken if it makes the routes
/// shorter, or leaves them as long and widens the smallest gap by a pixel or more, and is safe.
/// Says whether a point moved.
fn apply_problem(graph: &Graph, routes: &mut [Vec<Point>], groups: &[Group], arcs: &[Arc], axis: Axis) -> bool {
    let mut parent: Vec<usize> = (0..groups.len()).collect();
    fn root(parent: &mut [usize], mut i: usize) -> usize {
        while parent[i] != i {
            parent[i] = parent[parent[i]];
            i = parent[i];
        }
        i
    }
    for arc in arcs {
        let (to, from) = (root(&mut parent, arc.to), root(&mut parent, arc.from));
        parent[to] = from;
    }
    let mut components: Vec<(usize, Vec<usize>)> = Vec::new();
    for i in 0..groups.len() {
        let id = root(&mut parent, i);
        match components.iter_mut().find(|(other, _)| *other == id) {
            Some((_, members)) => members.push(i),
            None => components.push((id, vec![i])),
        }
    }
    let mut changed = false;
    for (_, members) in components {
        let local: Vec<&Group> = members.iter().map(|group| &groups[*group]).collect();
        let place = |group: usize| members.iter().position(|other| *other == group);
        let local_arcs: Vec<Arc> = arcs.iter().filter_map(|arc| Some(Arc { from: place(arc.from)?, to: place(arc.to)?, separate: arc.separate })).collect();
        let positions: Vec<f64> = local.iter().map(|group| group.position).collect();
        let old_gap = channel_gap(&local, &local_arcs, &positions).max(0.0);
        let (mut left, mut right) = (old_gap, f64::INFINITY);
        for group in &local {
            right = right.min((group.upper - group.lower) / 2.0);
        }
        let mut iteration = 0;
        while iteration < 32 && right - left > EPSILON {
            let middle = (left + right) / 2.0;
            if solve_gap(&local, &local_arcs, middle).is_some() {
                left = middle;
            } else {
                right = middle;
            }
            iteration += 1;
        }
        let mut best_length = wire_length(routes);
        let mut best_gap = old_gap;
        let mut best: Option<Proposal> = None;
        for gap in [old_gap, left] {
            let Some((earliest, latest)) = solve_gap(&local, &local_arcs, gap) else {
                continue;
            };
            for fraction in [0.0, 1.0, 0.5] {
                let candidate: Vec<f64> = earliest.iter().zip(&latest).map(|(early, late)| early * (1.0 - fraction) + late * fraction).collect();
                let Some(proposal) = point_proposal(routes, &local, &candidate, axis) else {
                    continue;
                };
                let length = wire_length(&proposed(routes, &proposal));
                let actual_gap = channel_gap(&local, &local_arcs, &candidate);
                // A wider gap by less than a pixel is no gain.
                if length > best_length + EPSILON || ((length - best_length).abs() <= EPSILON && actual_gap < best_gap + 1.0) {
                    continue;
                }
                if !proposal_safe(graph, routes, &proposal) {
                    continue;
                }
                (best, best_length, best_gap) = (Some(proposal), length, actual_gap);
            }
        }
        if let Some(best) = best {
            for (route, moved) in routes.iter_mut().zip(&best) {
                for (point, to) in route.iter_mut().zip(moved) {
                    *point = to.unwrap_or(*point);
                }
            }
            changed = true;
        }
    }
    changed
}

/// TALA `removeDuplicatePointsGuarded`: a point that repeats the one before it goes. A route
/// left with one point gets a second, a pixel beside it.
fn remove_duplicate_points(points: &[Point]) -> Vec<Point> {
    if points.len() <= 1 {
        return points.to_vec();
    }
    let mut result = vec![points[0]];
    result.extend(points.windows(2).filter(|pair| pair[0] != pair[1]).map(|pair| pair[1]));
    if result.len() == 1 {
        result.push(Point { x: result[0].x + 1.0, y: result[0].y });
    }
    result
}

/// The boxes of the labels where they are on these routes.
pub fn label_boxes(routes: &[Vec<Point>], labels: &[Option<Placed>], sizes: &[Option<LabelSize>]) -> Vec<Option<LabelBox>> {
    labels
        .iter()
        .zip(sizes)
        .zip(routes)
        .map(|((label, size), route)| {
            let (label, size) = (label.as_ref()?, size.as_ref()?);
            Some(LabelBox { top_left: label_top_left(route, label.position, label.percentage, *size), width: size.width, height: size.height })
        })
        .collect()
}

/// TALA `NudgeEdgeChannels`: first the upright pieces sideways, then the level ones up and
/// down. The labels keep their places along their routes and move with them.
pub fn nudge(graph: &Graph, routes: &mut [Vec<Point>], labels: &[Option<Placed>], sizes: &[Option<LabelSize>]) {
    // TALA `channelInputTooLarge`.
    if graph.nodes.len() > MOST_SEGMENTS || routes.len() > MOST_SEGMENTS || routes.iter().map(Vec::len).sum::<usize>() > MOST_SEGMENTS {
        return;
    }
    for sideways in [true, false] {
        let axis = Axis { sideways };
        let (groups, arcs) = build_problem(graph, routes, axis);
        if groups.is_empty() {
            continue;
        }
        let before = routes.to_vec();
        let before_metrics = inspect(graph, routes, &label_boxes(routes, labels, sizes));
        if !apply_problem(graph, routes, &groups, &arcs, axis) {
            continue;
        }
        let after_metrics = inspect(graph, routes, &label_boxes(routes, labels, sizes));
        if after_metrics.route_obstructions > before_metrics.route_obstructions
            || after_metrics.crossings > before_metrics.crossings
            || after_metrics.text_occlusions > before_metrics.text_occlusions
            || after_metrics.route_length > before_metrics.route_length + EPSILON
        {
            routes.clone_from_slice(&before);
            continue;
        }
        for (route, old) in routes.iter_mut().zip(&before) {
            if route != old {
                *route = remove_duplicate_points(route);
            }
        }
    }
}
