// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.

//! Spreading routes over the room they have: every upright piece is moved sideways, and every
//! level piece up or down, to share the space between the cards and the other routes evenly.
//! Ported from TALA `internal/routing`: `postprocess.go` (`balanceRegularEdgesGuarded`,
//! `isSpecialEdgeForBalancing`), `balance_route_guard.go` (`edgeSegmentsGuarded`,
//! `nodeSegmentsGuarded`, `routeSegmentBounds`, `evenlyDistributeGuarded`, `checkBalanceOrder`,
//! `removeDuplicatePointsGuarded`), `balance_crossings.go` and D2 `geo.Segment.Overlaps`.
//! With `getSharedCluster` for clusters. Not ported: routes with fixed ports (tables,
//! diamonds), cards of 1 by 1.
//!
//! The order of the work, in TALA's words: for each axis, (1) every piece gets its range of
//! movement, bounded by the borders of cards and by pieces that are already locked; (2) the pieces
//! with the smallest range that overlap each other form a batch; (3) the batch is spread evenly
//! over the range and locked; (4) repeat until every piece is locked.

use crate::graph::{Arrangement, Graph, Point};
use crate::routing::line::crosses;
use crate::routing::tuning::{SEGMENT_SPACING_BUFFER, UNBOUNDED_SEGMENT_MOVE};

/// One straight piece of a route: its connection and the two points it joins, by their place in
/// the route. `start` is the end with the smaller coordinate along the piece.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct Piece {
    pub(crate) edge: usize,
    pub(crate) start: usize,
    pub(crate) end: usize,
}

/// Something a piece cannot be moved past: a side of a card, or a piece that keeps its place.
#[derive(Clone, Copy)]
enum Locked {
    Border(Point, Point),
    Piece(Piece),
}

/// TALA `balanceOrderStatus`: what moving a batch does to its order with the other routes.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
enum Order {
    Preserved,
    Reversed,
    ContactChanged,
}

/// Which pieces are being moved: upright ones sideways (`sideways`), or level ones up and down.
#[derive(Clone, Copy)]
pub(crate) struct Axis {
    pub(crate) sideways: bool,
}

impl Axis {
    /// The coordinate that balancing changes.
    pub(crate) fn across(self, p: Point) -> f64 {
        if self.sideways { p.x } else { p.y }
    }

    /// The coordinate along the piece.
    pub(crate) fn along(self, p: Point) -> f64 {
        if self.sideways { p.y } else { p.x }
    }

    pub(crate) fn set_across(self, p: &mut Point, value: f64) {
        if self.sideways {
            p.x = value;
        } else {
            p.y = value;
        }
    }
}

pub(crate) fn ends(routes: &[Vec<Point>], piece: Piece) -> (Point, Point) {
    (routes[piece.edge][piece.start], routes[piece.edge][piece.end])
}

/// TALA `edgeSegmentsGuarded`: the pieces of the given connections that the axis moves.
pub(crate) fn pieces_of(routes: &[Vec<Point>], edges: &[usize], axis: Axis) -> Vec<Piece> {
    let mut out = Vec::new();
    for &edge in edges {
        let points = &routes[edge];
        for i in 0..points.len().saturating_sub(1) {
            let (p, q) = (points[i], points[i + 1]);
            if axis.across(p) != axis.across(q) {
                continue;
            }
            let (start, end) = if axis.along(p) < axis.along(q) { (i, i + 1) } else { (i + 1, i) };
            out.push(Piece { edge, start, end });
        }
    }
    out
}

/// D2 `Segment.Overlaps`: the two pieces lie side by side for some stretch, or closer than
/// `buffer` to doing so.
pub(crate) fn overlaps(a: (Point, Point), b: (Point, Point), axis: Axis, buffer: f64) -> bool {
    let (a_min, a_max) = (axis.along(a.0).min(axis.along(a.1)), axis.along(a.0).max(axis.along(a.1)));
    let (b_min, b_max) = (axis.along(b.0).min(axis.along(b.1)), axis.along(b.0).max(axis.along(b.1)));
    !(a_min - b_max >= buffer || b_min - a_max >= buffer)
}

/// TALA `routeSegmentBounds`: how far the piece can move either way before it meets something
/// locked that lies beside it (or within the spacing buffer of its ends).
pub(crate) fn bounds(piece: (Point, Point), locked: &[(Point, Point)]) -> (f64, f64) {
    let (mut floor, mut ceil) = (f64::NEG_INFINITY, f64::INFINITY);
    if piece.0 == piece.1 {
        return (floor, ceil);
    }
    // TALA takes the direction from the piece itself: upright when its ends share an x.
    let axis = Axis { sideways: piece.0.x == piece.1.x };
    for other in locked {
        if axis.along(other.1) < axis.along(piece.0) - SEGMENT_SPACING_BUFFER
            || axis.along(other.0) > axis.along(piece.1) + SEGMENT_SPACING_BUFFER
        {
            continue;
        }
        let at = axis.across(other.0);
        if at <= axis.across(piece.0) {
            floor = floor.max(at);
        }
        if at > axis.across(piece.0) {
            ceil = ceil.min(at);
        }
    }
    (floor, ceil)
}

/// TALA `evenlyDistributeGuarded`: `count` places between `floor` and `ceil`, whole steps apart.
fn evenly_distribute(floor: f64, ceil: f64, count: usize) -> Vec<f64> {
    if count == 0 || floor >= ceil {
        return Vec::new();
    }
    let increment = ((ceil - floor) / (count + 1) as f64).floor();
    if increment <= 0.0 {
        return Vec::new();
    }
    (1..=count).map(|index| floor + index as f64 * increment).collect()
}

/// TALA `checkBalanceOrder`: would the batch, moved to `proposed`, pass pieces of other routes
/// that lie beside it, or come to lie on one or leave one it lies on?
fn check_order(
    routes: &[Vec<Point>],
    batch: &[Piece],
    others: &[Piece],
    proposed: &[f64],
    axis: Axis,
) -> Order {
    let mut status = Order::Preserved;
    for other in others {
        if batch.contains(other) {
            continue;
        }
        let other_ends = ends(routes, *other);
        for (piece, to) in batch.iter().zip(proposed) {
            // Pieces of one route may meet: that removes a jog.
            if piece.edge == other.edge {
                continue;
            }
            let piece_ends = ends(routes, *piece);
            let (old, position) = (axis.across(piece_ends.0), axis.across(other_ends.0));
            if *to == old {
                continue;
            }
            let (start, end) = (axis.along(piece_ends.0), axis.along(piece_ends.1));
            let (other_start, other_end) = (axis.along(other_ends.0), axis.along(other_ends.1));
            // Touching counts: a bend at the end of either piece moves with it.
            if start.max(end) < other_start.min(other_end) || other_start.max(other_end) < start.min(end) {
                continue;
            }
            if old == position || *to == position {
                return Order::ContactChanged;
            }
            if (old < position) != (*to < position) {
                status = Order::Reversed;
            }
        }
    }
    status
}

/// TALA `balanceCollinearOverlap`: how long two pieces lie on each other.
pub(crate) fn collinear_overlap(a: Point, b: Point, c: Point, d: Point) -> f64 {
    if a.x == b.x && c.x == d.x && a.x == c.x {
        return (a.y.max(b.y).min(c.y.max(d.y)) - a.y.min(b.y).max(c.y.min(d.y))).max(0.0);
    }
    if a.y == b.y && c.y == d.y && a.y == c.y {
        return (a.x.max(b.x).min(c.x.max(d.x)) - a.x.min(b.x).max(c.x.min(d.x))).max(0.0);
    }
    0.0
}

/// TALA `balanceReversalRemovesCrossings`: a batch that would change its order with other routes
/// may still move when that removes crossings and adds none, for no pair of routes.
fn reversal_removes_crossings(routes: &[Vec<Point>], batch: &[Piece], proposed: &[f64], axis: Axis) -> bool {
    // The routes as they would be. A point that two pieces of the batch want at different places
    // settles it: no.
    let mut candidates: Vec<Option<Vec<Point>>> = vec![None; routes.len()];
    let mut moved: Vec<(usize, usize, f64)> = Vec::new();
    for (piece, to) in batch.iter().zip(proposed) {
        for point in [piece.start, piece.end] {
            if let Some((_, _, before)) = moved.iter().find(|(edge, index, _)| *edge == piece.edge && *index == point) {
                if before != to {
                    return false;
                }
                continue;
            }
            moved.push((piece.edge, point, *to));
        }
    }
    let mut affected: Vec<usize> = Vec::new();
    for (edge, points) in routes.iter().enumerate() {
        let mut candidate = points.clone();
        for (_, index, to) in moved.iter().filter(|(moved_edge, _, _)| *moved_edge == edge) {
            axis.set_across(&mut candidate[*index], *to);
        }
        if candidate == *points {
            continue;
        }
        // Moving must not slant a piece, and a slanted route is not judged at all.
        let slanted = |points: &[Point]| points.windows(2).any(|pair| pair[0].x != pair[1].x && pair[0].y != pair[1].y);
        if slanted(points) || slanted(&candidate) {
            return false;
        }
        candidates[edge] = Some(candidate);
        affected.push(edge);
    }
    let mut improved = false;
    for &i in &affected {
        let candidate = candidates[i].as_ref().expect("an affected route has a candidate");
        for (j, other) in routes.iter().enumerate() {
            if i == j || (candidates[j].is_some() && j < i) {
                continue;
            }
            let other_candidate = candidates[j].as_ref().unwrap_or(other);
            let (mut before, mut after) = (0, 0);
            for a in 0..routes[i].len().saturating_sub(1) {
                for b in 0..other.len().saturating_sub(1) {
                    if crosses(routes[i][a], routes[i][a + 1], other[b], other[b + 1]) {
                        before += 1;
                    }
                    if crosses(candidate[a], candidate[a + 1], other_candidate[b], other_candidate[b + 1]) {
                        after += 1;
                    }
                    // A piece dragged along must not come to lie on another route.
                    let old_overlap = collinear_overlap(routes[i][a], routes[i][a + 1], other[b], other[b + 1]);
                    let new_overlap =
                        collinear_overlap(candidate[a], candidate[a + 1], other_candidate[b], other_candidate[b + 1]);
                    if old_overlap == 0.0 && new_overlap > 0.0 {
                        return false;
                    }
                }
            }
            if after > before {
                return false;
            }
            improved = improved || after < before;
        }
    }
    improved
}

/// TALA `isSpecialEdgeForBalancing`: a connection of a card of a tree keeps its route, and so
/// does a route whose two ends are one pixel out of line (the cards' middles are a pixel
/// apart; balancing would make it worse).
pub(crate) fn is_special(graph: &Graph, edge: usize, route: &[Point]) -> bool {
    if graph.hangs_in_tree(edge) {
        return true;
    }
    match (route.first(), route.last()) {
        (Some(first), Some(last)) => (first.x - last.x).abs() == 1.0 || (first.y - last.y).abs() == 1.0,
        _ => false,
    }
}

/// TALA `getSharedCluster`: a cluster that a card of each of the two connections is in, asked
/// first for the start of the first connection, then for its end.
fn shared_cluster(graph: &Graph, first: usize, second: usize) -> Option<usize> {
    let (a, b) = (&graph.edges[first], &graph.edges[second]);
    let (from_b, to_b) = (graph.cluster_of(b.from), graph.cluster_of(b.to));
    [graph.cluster_of(a.from), graph.cluster_of(a.to)].into_iter().flatten().find(|cluster| Some(*cluster) == from_b || Some(*cluster) == to_b)
}

/// TALA `balanceRegularEdgesGuarded`: spreads the pieces of all routes over the room between the
/// cards, first the upright pieces sideways, then the level ones up and down.
pub fn balance(graph: &Graph, routes: &mut [Vec<Point>]) {
    let (special, regular): (Vec<usize>, Vec<usize>) = (0..routes.len()).partition(|edge| is_special(graph, *edge, &routes[*edge]));
    let close = |a: f64, b: f64| (a - b).abs() <= 1.0;

    for axis in [Axis { sideways: true }, Axis { sideways: false }] {
        // The sides of the cards that run along the pieces. TALA `nodeSegmentsGuarded`.
        let mut locked: Vec<Locked> = Vec::new();
        for node in &graph.nodes {
            let (tl, br) = (node.top_left, Point { x: node.right(), y: node.bottom() });
            if axis.sideways {
                locked.push(Locked::Border(tl, Point { x: tl.x, y: br.y }));
                locked.push(Locked::Border(Point { x: br.x, y: tl.y }, br));
            } else {
                locked.push(Locked::Border(tl, Point { x: br.x, y: tl.y }));
                locked.push(Locked::Border(Point { x: tl.x, y: br.y }, br));
            }
        }
        let pieces = pieces_of(routes, &regular, axis);
        let special_pieces = pieces_of(routes, &special, axis);
        let mut is_locked = vec![false; pieces.len()];
        // Routes that keep their place are not moved, but nothing is moved past them. A piece
        // that lies on one of them keeps its place too.
        let mut kept: Vec<Piece> = special_pieces.clone();
        for (index, piece) in pieces.iter().enumerate() {
            let piece_ends = ends(routes, *piece);
            let on_kept = kept.iter().any(|other| {
                let other_ends = ends(routes, *other);
                overlaps(piece_ends, other_ends, axis, 1.0) && close(axis.across(piece_ends.0), axis.across(other_ends.0))
            });
            if on_kept {
                kept.push(*piece);
                is_locked[index] = true;
            }
        }
        locked.extend(kept.into_iter().map(Locked::Piece));

        while is_locked.iter().any(|locked| !locked) {
            let locked_now: Vec<(Point, Point)> = locked
                .iter()
                .map(|lock| match lock {
                    Locked::Border(a, b) => (*a, *b),
                    Locked::Piece(piece) => ends(routes, *piece),
                })
                .collect();
            // Every free piece with its range; the smallest range goes first.
            let mut ranges: Vec<Option<(f64, f64)>> = vec![None; pieces.len()];
            let mut least = (f64::NEG_INFINITY, f64::INFINITY);
            for (index, piece) in pieces.iter().enumerate() {
                if is_locked[index] {
                    continue;
                }
                let piece_ends = ends(routes, *piece);
                let (mut floor, mut ceil) = bounds(piece_ends, &locked_now);
                if floor == f64::NEG_INFINITY {
                    floor = axis.across(piece_ends.0) - UNBOUNDED_SEGMENT_MOVE;
                }
                if ceil == f64::INFINITY {
                    ceil = axis.across(piece_ends.0) + UNBOUNDED_SEGMENT_MOVE;
                }
                ranges[index] = Some((floor, ceil));
                if ceil - floor < least.1 - least.0 {
                    least = (floor, ceil);
                }
            }
            let with_least: Vec<usize> = (0..pieces.len()).filter(|index| ranges[*index] == Some(least)).collect();

            for &index in &with_least {
                if is_locked[index] {
                    continue;
                }
                let piece = pieces[index];
                let piece_ends = ends(routes, piece);
                // The pieces with the same range that lie beside this one share the room.
                let mut batch = vec![index];
                for &other in &with_least {
                    if other == index || is_locked[other] {
                        continue;
                    }
                    // Pieces of one route need no room between them.
                    let buffer = if pieces[other].edge == piece.edge { 0.0 } else { SEGMENT_SPACING_BUFFER };
                    if overlaps(piece_ends, ends(routes, pieces[other]), axis, buffer) {
                        batch.push(other);
                    }
                }
                // A piece that lies on this one, whatever its range, moves with it: routes that
                // share a stretch keep sharing it.
                for other in 0..pieces.len() {
                    if batch.contains(&other) || is_locked[other] {
                        continue;
                    }
                    let other_ends = ends(routes, pieces[other]);
                    // Routes of the cards of one cluster, arranged as the cards around it asked:
                    // the middle pieces that cross the line of the cluster at one place move
                    // together, also where they do not overlap.
                    if let Some(cluster) = shared_cluster(graph, piece.edge, pieces[other].edge) {
                        let cluster = &graph.clusters[cluster];
                        let across = if axis.sideways { Arrangement::Column } else { Arrangement::Row };
                        if cluster.arrangement == cluster.desired && cluster.arrangement == across && close(axis.across(piece_ends.0), axis.across(other_ends.0)) {
                            batch.push(other);
                            continue;
                        }
                    }
                    if overlaps(piece_ends, other_ends, axis, 1.0)
                        && close(axis.across(piece_ends.0), axis.across(other_ends.0))
                    {
                        batch.push(other);
                    }
                }
                batch.sort_by(|a, b| {
                    let (a, b) = (ends(routes, pieces[*a]).0, ends(routes, pieces[*b]).0);
                    axis.across(a).total_cmp(&axis.across(b)).then(axis.along(a).total_cmp(&axis.along(b)))
                });
                // Pieces at the same place share a route on purpose: they get one place together.
                let mut distinct: Vec<f64> = batch.iter().map(|b| axis.across(ends(routes, pieces[*b]).0).round()).collect();
                distinct.sort_by(f64::total_cmp);
                distinct.dedup();
                let places = evenly_distribute(least.0, least.1, distinct.len());
                let mut proposed: Vec<f64> = batch.iter().map(|b| axis.across(ends(routes, pieces[*b]).0)).collect();
                let mut next = 0;
                for place in places {
                    let mut shared = 0;
                    while next + shared < batch.len()
                        && close(
                            axis.across(ends(routes, pieces[batch[next]]).0),
                            axis.across(ends(routes, pieces[batch[next + shared]]).0),
                        )
                    {
                        shared += 1;
                    }
                    for entry in &mut proposed[next..next + shared] {
                        *entry = place;
                    }
                    next += shared;
                }

                let batch_pieces: Vec<Piece> = batch.iter().map(|b| pieces[*b]).collect();
                let mut order = check_order(routes, &batch_pieces, &pieces, &proposed, axis);
                if order != Order::ContactChanged {
                    order = order.max(check_order(routes, &batch_pieces, &special_pieces, &proposed, axis));
                }
                let accept = match order {
                    Order::Preserved => true,
                    Order::Reversed => reversal_removes_crossings(routes, &batch_pieces, &proposed, axis),
                    Order::ContactChanged => false,
                };
                if accept {
                    for (piece, to) in batch_pieces.iter().zip(&proposed) {
                        axis.set_across(&mut routes[piece.edge][piece.start], *to);
                        axis.set_across(&mut routes[piece.edge][piece.end], *to);
                    }
                }
                for &member in &batch {
                    is_locked[member] = true;
                    locked.push(Locked::Piece(pieces[member]));
                }
            }
        }
    }

    // TALA `removeDuplicatePointsGuarded`: pieces that met leave a point twice.
    for &edge in &regular {
        let points = &mut routes[edge];
        points.dedup();
        if points.len() == 1 {
            let mut second = points[0];
            second.x += 1.0;
            points.push(second);
        }
    }
}
