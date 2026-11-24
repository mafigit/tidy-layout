// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.

//! Compaction along one axis: every card keeps a least distance to the card it sees behind it
//! (inflate), then groups of cards and single cards move back towards the cards behind them
//! where that makes connections shorter. Ported from TALA `internal/placement`: `compaction.go`,
//! `moves.go` (`moveNodeToBest`), and `internal/layoutgraph/node.go`
//! (`isVisibilityGraphCandidate`, `IsBlocked`, `doesOverlapAt`).
//!
//! It runs in both halves of node placement: without sizes the cards are points on the coarse
//! grid (`top_left` in cells), with sizes they are boxes in pixels. For plain cards; no fixed
//! places.

use crate::graph::{Graph, Point};
use crate::placement::precision_compare;
use crate::placementcost::{edge_length::node_edge_length, symmetry::node_symmetry};
use crate::policy::CONNECTED_NODE_GAP;
use crate::sizeless;

/// TALA `compactionOptions`.
#[derive(Clone, Copy, Debug)]
pub struct Options {
    /// Along x; otherwise along y.
    pub horizontal: bool,
    /// Cards are boxes in pixels; otherwise points in cells.
    pub sizes: bool,
    /// How many card lengths (with sizes) or cells a card keeps from the one behind it.
    pub factor: f64,
    /// The change from cells to pixels: only make room.
    pub transition: bool,
}

/// One card sees another: `from` lies behind `to` along the axis.
type Sight = (usize, usize);

fn along(point: Point, horizontal: bool) -> f64 {
    if horizontal { point.x } else { point.y }
}

fn moved_along(point: Point, horizontal: bool, to: f64) -> Point {
    if horizontal { Point { x: to, y: point.y } } else { Point { x: point.x, y: to } }
}

/// The price of a card where it is: with sizes TALA's edge length, without the sizeless one.
fn length(graph: &Graph, node: usize, sizes: bool, turn: f64) -> f64 {
    if sizes { node_edge_length(graph, node, turn) } else { sizeless::edge_length(graph, node, None) }
}

/// TALA `orderedAlongAxis`: the cards from the back to the front, equal ones by their number.
fn ordered_along_axis(graph: &Graph, horizontal: bool) -> Vec<usize> {
    let mut order: Vec<usize> = (0..graph.nodes.len()).collect();
    // 0 and -0 are the same place: neither comes first.
    order.sort_by(|a, b| {
        let (first, second) = (along(graph.nodes[*a].top_left, horizontal), along(graph.nodes[*b].top_left, horizontal));
        first.partial_cmp(&second).expect("places are numbers").then(graph.id(*a).cmp(&graph.id(*b)))
    });
    order
}

/// TALA `isVisibilityGraphCandidate` with the side check: `node` lies behind `other`, and the
/// two share a stretch across the axis (with sizes: come within `padding` of sharing one).
fn is_candidate(graph: &Graph, node: usize, other: usize, options: &Options, padding: f64) -> bool {
    let (a, b) = (&graph.nodes[node], &graph.nodes[other]);
    if along(a.top_left, options.horizontal) >= along(b.top_left, options.horizontal) {
        return false;
    }
    let (a_start, b_start) = (along(a.top_left, !options.horizontal), along(b.top_left, !options.horizontal));
    if !options.sizes {
        return a_start == b_start;
    }
    let (a_length, b_length) = if options.horizontal { (a.height, b.height) } else { (a.width, b.width) };
    a_start <= b_start + b_length + padding && a_start + a_length + padding >= b_start
}

/// TALA `IsBlocked`: `node` lies between `a` and `b` along the axis and across the line of
/// sight between them.
pub(crate) fn is_blocked(graph: &Graph, node: usize, a: usize, b: usize, options: &Options) -> bool {
    let (card, first, second) = (&graph.nodes[node], &graph.nodes[a], &graph.nodes[b]);
    let horizontal = options.horizontal;
    let size = |card: &crate::graph::Node, axis: bool| {
        if !options.sizes {
            0.0
        } else if axis {
            card.width
        } else {
            card.height
        }
    };
    let (start, first_start, second_start) =
        (along(card.top_left, horizontal), along(first.top_left, horizontal), along(second.top_left, horizontal));
    if !(start >= first_start + size(first, horizontal) && start + size(card, horizontal) <= second_start) {
        return false;
    }
    let (across, first_across, second_across) =
        (along(card.top_left, !horizontal), along(first.top_left, !horizontal), along(second.top_left, !horizontal));
    across <= first_across.max(second_across)
        && across + size(card, !horizontal)
            >= (first_across + size(first, !horizontal)).min(second_across + size(second, !horizontal))
}

/// TALA `visibilityEdges`: every pair of cards of which the first lies behind the second with
/// no third card between them.
fn visibility_edges(graph: &Graph, options: &Options) -> Vec<Sight> {
    let count = graph.nodes.len();
    let mut edges = Vec::new();
    for node in 0..count {
        for other in 0..count {
            if node == other {
                continue;
            }
            let delta = graph.gap_between(node, other, graph.nodes[node].top_left);
            if !is_candidate(graph, node, other, options, delta) {
                continue;
            }
            let blocked = (0..count).any(|third| third != node && third != other && is_blocked(graph, third, node, other, options));
            if !blocked {
                edges.push((node, other));
            }
        }
    }
    edges
}

/// TALA `nearestFrom`: of the cards that see `node` in front of them, the one that reaches
/// furthest along the axis.
fn nearest_from(graph: &Graph, edges: &[Sight], node: usize, options: &Options) -> Option<usize> {
    let mut nearest = None;
    let mut furthest = f64::NEG_INFINITY;
    for &(from, to) in edges {
        if to != node {
            continue;
        }
        let card = &graph.nodes[from];
        let mut end = along(card.top_left, options.horizontal);
        if options.sizes {
            end += if options.horizontal { card.width } else { card.height };
        }
        if end > furthest {
            furthest = end;
            nearest = Some(from);
        }
    }
    nearest
}

/// TALA `compactionFloor`: the least coordinate a card may have in front of `anchor`; with sizes
/// in cells of the grid.
fn compaction_floor(graph: &Graph, anchor: usize, options: &Options, padding: f64) -> f64 {
    let card = &graph.nodes[anchor];
    let start = along(card.top_left, options.horizontal);
    if !options.sizes {
        return start + options.factor.floor();
    }
    let size = if options.horizontal { card.width } else { card.height };
    let mut floor = ((start + options.factor * size) / graph.cell_size).ceil();
    while floor * graph.cell_size - (start + size) <= padding {
        floor += 1.0;
    }
    floor
}

/// TALA `inflateAlongAxis`: from the back to the front, every card moves forward to its floor in
/// front of the card it sees behind it.
fn inflate_along_axis(graph: &mut Graph, options: &Options, edges: &[Sight]) {
    for node in ordered_along_axis(graph, options.horizontal) {
        let Some(nearest) = nearest_from(graph, edges, node, options) else {
            continue;
        };
        let at = graph.nodes[node].top_left;
        // At the change to pixels cards may overlap for a while; the gap of connected cards
        // keeps every pair in sight of each other for the next axis.
        let delta = if options.transition { CONNECTED_NODE_GAP } else { graph.gap_between(node, nearest, at) };
        let mut floor = compaction_floor(graph, nearest, options, delta);
        if options.sizes {
            floor *= graph.cell_size;
        }
        if floor > along(at, options.horizontal) {
            graph.nodes[node].top_left = moved_along(at, options.horizontal, floor);
        }
    }
}

/// TALA `globallyFurthestBehind`.
fn globally_furthest_behind(graph: &Graph, horizontal: bool) -> usize {
    let mut furthest = 0;
    for (node, card) in graph.nodes.iter().enumerate() {
        if along(card.top_left, horizontal) < along(graph.nodes[furthest].top_left, horizontal) {
            furthest = node;
        }
    }
    furthest
}

/// TALA `candidateMoves`: the places from the card's floor in front of the card behind it up
/// to its own place, a cell apart. `floor_decrease` lets the card furthest behind go further.
fn candidate_moves(graph: &Graph, node: usize, options: &Options, floor_decrease: f64, edges: &[Sight]) -> Vec<Point> {
    let (horizontal, sizes, cell) = (options.horizontal, options.sizes, graph.cell_size);
    // A card that sees no card behind it starts a chain.
    let behind = nearest_from(graph, edges, node, options).unwrap_or_else(|| globally_furthest_behind(graph, horizontal));
    let (card, anchor) = (&graph.nodes[node], &graph.nodes[behind]);
    let mut floor = compaction_floor(graph, behind, options, graph.gap_between(node, behind, card.top_left));

    // The card itself, or a card that lies beside it across the axis: no room to keep.
    let (start, anchor_start) = (along(card.top_left, !horizontal), along(anchor.top_left, !horizontal));
    let another_axis = if sizes {
        let (length, anchor_length) = if horizontal { (card.height, anchor.height) } else { (card.width, anchor.width) };
        anchor_start > start + length || anchor_start + anchor_length < start
    } else {
        anchor_start != start
    };
    if along(anchor.top_left, horizontal) == along(card.top_left, horizontal) || another_axis {
        floor = along(anchor.top_left, horizontal);
        if sizes {
            floor /= cell;
        }
    }

    let mut ceil = along(card.top_left, horizontal);
    if sizes {
        ceil /= cell;
    }
    let mut positions = Vec::new();
    let mut i = floor - floor_decrease;
    while i <= ceil {
        positions.push(moved_along(card.top_left, horizontal, if sizes { i * cell } else { i }));
        i += 1.0;
    }
    positions
}

/// TALA `doesOverlapAt`: would the card at `point` come closer to `other` than the gap the two
/// must keep there?
pub(crate) fn overlaps_at(graph: &Graph, node: usize, other: usize, point: Point) -> bool {
    let (card, second) = (&graph.nodes[node], &graph.nodes[other]);
    let delta = graph.gap_between(node, other, point);
    point.x < second.right() + delta
        && second.top_left.x < point.x + card.width + delta
        && point.y < second.bottom() + delta
        && second.top_left.y < point.y + card.height + delta
}

/// TALA `shiftSubgraphs`: a card that sees no card behind it moves back together with the cards
/// that see it nearest behind them, to where the group's connections are shortest. Says whether
/// a group moved.
fn shift_subgraphs(graph: &mut Graph, options: &Options, turn: f64, edges: &[Sight]) -> bool {
    let (horizontal, sizes) = (options.horizontal, options.sizes);
    let count = graph.nodes.len();
    let mut changed = false;
    // Per card: the cards that see it nearest behind them, a root first in its own list.
    let mut groups: Vec<Vec<usize>> = vec![Vec::new(); count];
    // TALA sorts the roots once more; they are found in that order.
    let mut roots = Vec::new();
    for node in ordered_along_axis(graph, horizontal) {
        match nearest_from(graph, edges, node, options) {
            None => {
                // The lines of sight are from before the cards moved: a card that saw this one
                // behind it may lie behind it by now and be in its list already. TALA starts
                // the list anew, and so that card stays where it is.
                groups[node] = vec![node];
                roots.push(node);
            }
            Some(nearest) => groups[nearest].push(node),
        }
    }

    let furthest_behind = globally_furthest_behind(graph, horizontal);
    for root in roots {
        let group = &groups[root];
        let home = graph.nodes[root].top_left;
        let at_the_back = along(home, horizontal) == along(graph.nodes[furthest_behind].top_left, horizontal);
        // TALA adds the group's own place to these, last. It is priced like the others, with the
        // symmetry taken off, while the price to beat at the start is without it: staying can
        // win over a move that beats that price.
        let mut moves = candidate_moves(graph, root, options, if at_the_back { 2.0 } else { 0.0 }, edges);
        moves.push(home);

        let others: Vec<usize> = (0..count).filter(|card| !group.contains(card)).collect();
        let connections: usize = group.iter().map(|card| graph.edges_of(*card).count()).sum();
        let symmetry_cost = if sizes { graph.cell_size } else { 1.0 } * connections as f64;
        let group_length = |graph: &Graph| group.iter().fold(0.0, |sum, card| sum + length(graph, *card, sizes, turn));

        let mut best = group_length(graph);
        let mut best_delta = 0.0;
        let before: Vec<Point> = group.iter().map(|card| graph.nodes[*card].top_left).collect();
        let shifted = |place: Point, delta: f64| moved_along(place, horizontal, along(place, horizontal) - delta);
        for target in moves {
            let delta = along(home, horizontal) - along(target, horizontal);
            let overlaps = group.iter().zip(&before).any(|(card, place)| {
                let point = shifted(*place, delta);
                others.iter().any(|other| {
                    if sizes { overlaps_at(graph, *card, *other, point) } else { graph.nodes[*other].top_left == point }
                })
            });
            if overlaps {
                continue;
            }
            for (card, place) in group.iter().zip(&before) {
                graph.nodes[*card].top_left = shifted(*place, delta);
            }
            let mut after = group_length(graph);
            if sizes {
                let symmetry = group.iter().fold(0.0, |sum, card| sum + node_symmetry(graph, *card));
                after -= symmetry * symmetry_cost;
            }
            for (card, place) in group.iter().zip(&before) {
                graph.nodes[*card].top_left = *place;
            }
            if after < best {
                best = after;
                best_delta = delta;
            }
        }
        if best_delta != 0.0 {
            changed = true;
            for (card, place) in group.iter().zip(&before) {
                graph.nodes[*card].top_left = shifted(*place, best_delta);
            }
        }
    }
    changed
}

/// TALA `moveNodeToBest` of compaction: the card moves to the cheapest of the places it can go
/// to; of equally cheap ones the first, its own before any other. Says whether it moved.
fn move_node_to_best(graph: &mut Graph, node: usize, points: &[Point], sizes: bool, turn: f64) -> bool {
    let home = graph.nodes[node].top_left;
    let symmetry_cost = graph.cell_size * graph.edges_of(node).count() as f64;
    let mut least = f64::INFINITY;
    let mut best = home;
    for &point in points {
        if point != home {
            let taken = graph.nodes.iter().any(|card| card.top_left == point);
            if taken || (sizes && crate::placement::does_overlap(graph, node, point)) {
                continue;
            }
        }
        graph.nodes[node].top_left = point;
        let mut price = length(graph, node, sizes, turn);
        if sizes {
            // TALA skips the symmetry of a place that full symmetry could not make the cheapest.
            price -= node_symmetry(graph, node) * symmetry_cost;
        }
        match precision_compare(price, least) {
            -1 => (least, best) = (price, point),
            0 if point == home => (least, best) = (price, point),
            _ => {}
        }
    }
    graph.nodes[node].top_left = best;
    best != home
}

/// TALA `compactAlongAxis`: from the back to the front, every card moves back towards the card
/// it sees behind it, as far as that makes its connections shorter. Says whether a card moved.
fn compact_along_axis(graph: &mut Graph, options: &Options, turn: f64, edges: &[Sight]) -> bool {
    let mut changed = false;
    for node in ordered_along_axis(graph, options.horizontal) {
        let mut moves = candidate_moves(graph, node, options, 0.0, edges);
        moves.push(graph.nodes[node].top_left);
        changed |= move_node_to_best(graph, node, &moves, options.sizes, turn);
    }
    changed
}

/// TALA `compaction`. `turn` is the price of a bend; it counts only with sizes.
pub fn compaction(graph: &mut Graph, options: &Options, turn: f64) {
    let edges = visibility_edges(graph, options);
    inflate_along_axis(graph, options, &edges);
    // At the change to pixels cards may overlap along one axis; what follows expects none.
    if options.transition {
        return;
    }
    for _ in 0..20 {
        if !shift_subgraphs(graph, options, turn, &edges) {
            break;
        }
    }
    for _ in 0..20 {
        if !compact_along_axis(graph, options, turn, &edges) {
            break;
        }
    }
}
