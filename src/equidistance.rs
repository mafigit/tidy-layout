// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.

//! The stage that puts a card halfway between its nearest neighbour on one side and its
//! nearest on the other. Ported from TALA `internal/placement/equidistance.go`
//! (`Equidistance`, `equidistanceNodeGuarded`), `internal/layoutgraph/node.go`
//! (`reachableNodesGuarded`) and `internal/engine/pipeline.go` (`equidistanceStage`).

use crate::graph::{Graph, compute_cell_size};
use crate::optimizer::Allowed;
use crate::place::Placed;
use crate::placement::precision_compare;
use crate::routing::geometry::Orientation;
use crate::swap::length;

/// TALA tries the whole diagram again this often, as long as a card moves.
const MAX_ROUNDS: usize = 5;

/// TALA `equidistanceNodeGuarded`: the card moves across (`horizontal`) or down to the middle
/// between the nearest connected card before it and the nearest after it, alone or with the
/// cards that hang on it from the side, whichever leaves the diagram shorter, if it gets no
/// longer. Says whether a way was kept, also one that moves the card by nothing.
fn equidistance_node(placed: &mut Placed, node: usize, horizontal: bool) -> bool {
    // Cards of a hierarchy, tree cards and cards trees hang on stay.
    if placed.layered[node] || node >= placed.tree_cards || placed.graph.trees_at.contains(&node) {
        return false;
    }
    let costs = placed.prices();
    let tree_cards = placed.tree_cards;
    let graph = &mut placed.graph;
    let start = |graph: &Graph, card: usize| if horizontal { graph.nodes[card].top_left.x } else { graph.nodes[card].top_left.y };
    let end = |graph: &Graph, card: usize| if horizontal { graph.nodes[card].right() } else { graph.nodes[card].bottom() };
    let others: Vec<usize> = graph.edges_of(node).map(|edge| edge.other(node)).collect();
    // The nearest connected card before the card, and the nearest after it; of two as near, the
    // first.
    let mut back: Option<usize> = None;
    let mut front: Option<usize> = None;
    for &other in &others {
        if end(graph, other) < start(graph, node) && back.is_none_or(|back| end(graph, other) > end(graph, back)) {
            back = Some(other);
        }
        if start(graph, other) > end(graph, node) && front.is_none_or(|front| start(graph, other) < start(graph, front)) {
            front = Some(other);
        }
    }
    let (Some(back), Some(front)) = (back, front) else {
        return false;
    };
    // The cards that hang on the card from the side: its other neighbours straight above or
    // below it (beside it, when it moves down), with all that is connected to them.
    let mut hanging: Vec<usize> = Vec::new();
    // (TALA passes over the two nearest here. They stand before and after the card, so not
    // beside it.)
    for &other in &others {
        let side = graph.nodes[other].orientation(&graph.nodes[node]);
        let beside = if horizontal { matches!(side, Orientation::Top | Orientation::Bottom) } else { matches!(side, Orientation::Left | Orientation::Right) };
        if !beside {
            continue;
        }
        // TALA `reachableNodesGuarded`: tree cards are walked through and left out.
        let mut found = vec![other];
        let mut next = 0;
        while next < found.len() {
            let current = found[next];
            next += 1;
            for edge in graph.edges_of(current) {
                let adjacent = edge.other(current);
                if adjacent != node && adjacent != back && adjacent != front && !found.contains(&adjacent) {
                    found.push(adjacent);
                }
            }
        }
        for card in found.into_iter().filter(|card| *card < tree_cards) {
            if !hanging.contains(&card) {
                hanging.push(card);
            }
        }
    }
    let before = length(graph, &costs);
    let size = if horizontal { graph.nodes[node].width } else { graph.nodes[node].height };
    let by = ((end(graph, back) + start(graph, front)) / 2.0 - size / 2.0).round() - start(graph, node);
    let slide = |graph: &mut Graph, cards: &[usize]| {
        for &card in cards {
            if horizontal {
                graph.nodes[card].top_left.x += by;
            } else {
                graph.nodes[card].top_left.y += by;
            }
        }
    };
    // TALA's transaction puts the cards of every cluster in order here too. They are in order
    // since the gap stage.
    let allowed = Allowed::from(graph);
    let places = graph.nodes.clone();
    // A way counts if it brings no two cards too close and leaves the diagram no longer. TALA
    // takes a length of 0 for "does not count".
    let attempt = |graph: &mut Graph, cards: &[usize]| {
        slide(graph, cards);
        let slid = if allowed.now(graph) { length(graph, &costs) } else { 0.0 };
        graph.nodes = places.clone();
        if precision_compare(slid, before) <= 0 { slid } else { 0.0 }
    };
    let alone = attempt(graph, &[node]);
    let with_hanging: Vec<usize> = hanging.iter().copied().chain([node]).collect();
    let together = if hanging.is_empty() { 0.0 } else { attempt(graph, &with_hanging) };
    if alone == 0.0 && together == 0.0 {
        return false;
    }
    if together == 0.0 || (alone != 0.0 && precision_compare(alone, together) < 0) {
        slide(graph, &[node]);
    } else {
        slide(graph, &with_hanging);
    }
    true
}

/// TALA `Equidistance`: every card in turn, across and then down; where a way was kept, the
/// whole diagram up to five times more along that axis, as one card's move makes room for
/// another's. Says whether a way was kept: the stage that lines cards up runs again then.
pub fn equidistance(placed: &mut Placed) -> bool {
    placed.graph.cell_size = compute_cell_size(&placed.graph.nodes);
    let count = placed.graph.nodes.len();
    let (mut across, mut down) = (false, false);
    for node in 0..count {
        across |= equidistance_node(placed, node, true);
        down |= equidistance_node(placed, node, false);
    }
    for (kept, horizontal) in [(across, true), (down, false)] {
        if !kept {
            continue;
        }
        for _ in 0..MAX_ROUNDS {
            let mut again = false;
            for node in 0..count {
                again |= equidistance_node(placed, node, horizontal);
            }
            if !again {
                break;
            }
        }
    }
    across || down
}
