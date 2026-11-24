// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.

//! The stage that lines cards up: where the two cards of a connection are a little out of
//! line, one of them moves, with everything connected to it, until their middles are in line,
//! if that makes the whole diagram shorter. Ported from TALA `internal/placement`:
//! `stages.go` (`Align`), `alignment.go` (`alignAxes`, `alignmentDeltas`, `tryMove`,
//! `attemptShift`), `metrics.go` (`withinMaxSize`, `intersectsOtherNode`) and
//! `internal/layoutgraph`: `edge.go` (`isAxisAligned`), `node.go` (`connectedNodes`,
//! `orthogonalDistanceTo`), `tree.go` (`buildIsTreeEdgeMap`, `addIsolatedTreeEdgesToMap`).

use crate::graph::{Graph, Point};
use crate::optimizer::Allowed;
use crate::place::{Placed, bounds};
use crate::placement::precision_compare;
use crate::policy::CONNECTED_NODE_GAP;
use crate::routing::geometry::Orientation;
use crate::swap::{in_order, length};

/// TALA `AxisAlignmentTolerance`: middles closer than this are in line.
const AXIS_ALIGNMENT_TOLERANCE: f64 = 1.0;
/// TALA `MaxGraphSize`: no diagram may get wider or higher.
const MAX_GRAPH_SIZE: f64 = 30_000.0;
/// TALA `Align` gives up after this many rounds.
const MAX_ROUNDS: usize = 100;

/// TALA `isAxisAligned`: the middles of the two cards are in a row or in a column.
fn is_axis_aligned(graph: &Graph, edge: usize) -> bool {
    let (from, to) = (graph.nodes[graph.edges[edge].from].center(), graph.nodes[graph.edges[edge].to].center());
    (from.y - to.y).abs() < AXIS_ALIGNMENT_TOLERANCE || (from.x - to.x).abs() < AXIS_ALIGNMENT_TOLERANCE
}

/// TALA `alignmentDeltas`: how far the first card of the connection is from being in line with
/// the second, across and down. Cards beside each other are lined up in a row, cards above
/// each other in a column; cards on a diagonal only along an axis on which they are at least
/// a connection's gap apart.
fn alignment_deltas(graph: &Graph, edge: usize) -> (f64, f64) {
    let (from, to) = (&graph.nodes[graph.edges[edge].from], &graph.nodes[graph.edges[edge].to]);
    let mut x = (to.center().x - from.center().x).round();
    let mut y = (to.center().y - from.center().y).round();
    match from.orientation(to) {
        Orientation::Left | Orientation::Right => x = 0.0,
        Orientation::Top | Orientation::Bottom => y = 0.0,
        _ => {
            // TALA `orthogonalDistanceTo`: the room between the two cards along each axis.
            let apart = |start: f64, end: f64, other_start: f64, other_end: f64| {
                if start < other_start { (other_start - end).max(0.0) } else { (start - other_end).max(0.0) }
            };
            if apart(from.top_left.x, from.right(), to.top_left.x, to.right()) < CONNECTED_NODE_GAP {
                y = 0.0;
            }
            if apart(from.top_left.y, from.bottom(), to.top_left.y, to.bottom()) < CONNECTED_NODE_GAP {
                x = 0.0;
            }
        }
    }
    (x, y)
}

/// TALA `connectedNodes`: the card and every card connections lead to from it, nearest first,
/// without going to a card of `excluded`. A card that is excluded itself has none.
pub(crate) fn connected(graph: &Graph, start: usize, excluded: &[bool]) -> Vec<usize> {
    if excluded[start] {
        return Vec::new();
    }
    let mut found = vec![start];
    let mut next = 0;
    while next < found.len() {
        let current = found[next];
        next += 1;
        for edge in graph.edges_of(current) {
            let other = edge.other(current);
            if !excluded[other] && !found.contains(&other) {
                found.push(other);
            }
        }
    }
    found
}

/// TALA `intersectsOtherNode`: another card lies on the straight line between the middles of
/// the two.
fn intersects_other(graph: &Graph, first: usize, second: usize) -> bool {
    let (from, to) = (graph.nodes[first].center(), graph.nodes[second].center());
    graph.nodes.iter().enumerate().any(|(index, other)| index != first && index != second && other.passes_through(from, to))
}

/// TALA `alignAxes`: one round over all connections. Says whether a card moved. `allowed` is
/// what the stage's one transaction allows.
fn align_round(placed: &mut Placed, allowed: &Allowed) -> bool {
    let costs = placed.prices();
    // TALA prices the diagram at the start of every round (`initialLength`). It needs the
    // price only for tables, but the price cache keeps it.
    length(&placed.graph, &costs);
    let tree_cards = placed.tree_cards;
    let mut changed = false;
    for edge in 0..placed.graph.edges.len() {
        let graph = &mut placed.graph;
        let (from, to) = (graph.edges[edge].from, graph.edges[edge].to);
        if placed.tree_edges[edge] || is_axis_aligned(graph, edge) || placed.layered[from] || placed.layered[to] {
            continue;
        }
        let (x, y) = alignment_deltas(graph, edge);
        // TALA `tryMove`: the cards slide down by `y` or across by `x`. A slide is refused where
        // it makes the diagram too large, puts a card between the two, or brings two cards too
        // close. Of two slides that are as good, the second.
        let try_move = |graph: &mut Graph, cards: &[usize], x: f64, y: f64| {
            let mut best = (f64::INFINITY, (0.0, 0.0));
            let slides = [(0.0, y), (x, 0.0)];
            for (dx, dy) in slides.into_iter().filter(|(dx, dy)| precision_compare(*dx + *dy, 0.0) != 0) {
                let before = (graph.nodes.clone(), graph.vessels.clone());
                for &card in cards {
                    let at = graph.nodes[card].top_left;
                    graph.nodes[card].top_left = Point { x: at.x + dx, y: at.y + dy };
                }
                let all: Vec<usize> = (0..graph.nodes.len()).collect();
                let (top_left, bottom_right) = bounds(graph, &all);
                let fits = bottom_right.x - top_left.x <= MAX_GRAPH_SIZE && bottom_right.y - top_left.y <= MAX_GRAPH_SIZE;
                if fits && !intersects_other(graph, from, to) {
                    // The transaction puts the cards of every cluster in order, then checks.
                    in_order(graph);
                    if allowed.now(graph) {
                        let slid = length(graph, &costs);
                        if precision_compare(slid, best.0) < 1 {
                            best = (slid, (dx, dy));
                        }
                    }
                }
                (graph.nodes, graph.vessels) = before;
            }
            best
        };
        // A tree card does not move with the cards it is connected to, and neither does the
        // card at the other end of the connection.
        let count = graph.nodes.len();
        let excluding = |other: usize| -> Vec<bool> { (0..count).map(|card| card >= tree_cards || card == other).collect() };
        let mut best = length(graph, &costs);
        let mut winner: Option<(Vec<usize>, (f64, f64))> = None;
        let with_from = connected(graph, from, &excluding(to));
        let (slid, by) = try_move(graph, &with_from, x, y);
        if precision_compare(slid, best) < 0 {
            best = slid;
            winner = Some((with_from, by));
        }
        // The second card is asked against the first one's best, and wins where it beats it.
        let with_to = connected(graph, to, &excluding(from));
        let (slid, by) = try_move(graph, &with_to, -x, -y);
        if precision_compare(slid, best) < 0 {
            winner = Some((with_to, by));
        }
        if let Some((cards, (dx, dy))) = winner.filter(|(cards, _)| !cards.is_empty()) {
            for card in cards {
                let at = graph.nodes[card].top_left;
                graph.nodes[card].top_left = Point { x: at.x + dx, y: at.y + dy };
            }
            in_order(graph);
            changed = true;
        }
    }
    changed
}

/// TALA `Align`: rounds until one moves no card. One transaction for all of them: the cards
/// too close or covering each other at the start may stay so (`existingOverlaps`).
pub fn align(placed: &mut Placed) {
    let allowed = Allowed::from(&placed.graph);
    for _ in 0..MAX_ROUNDS {
        if !align_round(placed, &allowed) {
            break;
        }
    }
}
