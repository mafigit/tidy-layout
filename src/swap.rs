// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.

//! The first two stages after node placement. Two cards swap places where that makes the whole
//! diagram shorter, and the whole diagram is mirrored where most arrows then point right and
//! down. Then every card with one or two connections turns its branch around the card at
//! the other end where that makes the diagram shorter. Ported from TALA
//! `internal/placement`: `stages.go` (`Swap`, `TransposeAll`), `swap.go` (`swapOptimize`,
//! `swapPositions`, `smartSwapPositions`), `direct.go` (`direct` with `checkEdgeLength`),
//! `transpose.go` (`transpose` without edge abductions),
//! `internal/placementcost/graph.go` (`EdgeLength`, `GraphEdgeCrossings`,
//! `countEdgeCrossingsExcludingSharedNodes`) and `internal/layoutgraph/segment_crossing.go`
//! (`SegmentsCross`).

use crate::graph::{Graph, Point};
use crate::optimizer::{Allowed, reachable, rotate_around, turn_cluster};
use crate::place::{Placed, mirror, mirrors};
use crate::placement::precision_compare;
use crate::placementcost::{edge_length::node_edge_length, symmetry::node_symmetry};
use crate::routing::geometry::{Orientation, is_diagonal};
use crate::routing::router::Costs;

/// TALA `SegmentsCross`: do the two lines cross, their ends included? Parallel lines do not:
/// TALA asks for them first, here the division by 0 gives no number from 0 to 1 either.
fn segments_cross(u0: Point, u1: Point, v0: Point, v1: Point) -> bool {
    let denominator = (u1.y - u0.y) * (v1.x - v0.x) - (u1.x - u0.x) * (v1.y - v0.y);
    let s = ((v1.x - v0.x) * (v0.y - u0.y) - (v1.y - v0.y) * (v0.x - u0.x)) / denominator;
    if !(0.0..=1.0).contains(&s) {
        return false;
    }
    let t = ((u1.x - u0.x) * (v0.y - u0.y) - (u1.y - u0.y) * (v0.x - u0.x)) / denominator;
    (0.0..=1.0).contains(&t)
}

/// TALA `GraphEdgeCrossings`: the pairs of connections that share no card and whose straight
/// lines, from the middle of a card to the middle of the other, cross.
pub fn crossings(graph: &Graph) -> i64 {
    let line = |edge: usize| (graph.nodes[graph.edges[edge].from].center(), graph.nodes[graph.edges[edge].to].center());
    let mut count = 0;
    for first in 0..graph.edges.len() {
        for second in first + 1..graph.edges.len() {
            let (a, b) = (graph.edges[first], graph.edges[second]);
            if a.from == b.from || a.from == b.to || a.to == b.from || a.to == b.to {
                continue;
            }
            let ((u0, u1), (v0, v1)) = (line(first), line(second));
            count += i64::from(segments_cross(u0, u1, v0, v1));
        }
    }
    count
}

/// What a card costs where it is: the length of its connections, less a cell per connection
/// when it lies symmetric. The closure `measure` in TALA `swapOptimize`.
fn measure(graph: &Graph, node: usize, costs: &Costs) -> f64 {
    let length = node_edge_length(graph, node, costs.turn);
    length - node_symmetry(graph, node) * graph.cell_size * graph.edges_of(node).count() as f64
}

/// TALA `EdgeLength` with sizes: what all cards cost, and every crossing. Through TALA's
/// cache (`Graph::length_cache`).
pub fn length(graph: &Graph, costs: &Costs) -> f64 {
    let key = graph.length_key(true);
    if let Some(known) = graph.length_cache.get(&key) {
        return known;
    }
    let cards: f64 = (0..graph.nodes.len()).map(|node| measure(graph, node, costs)).sum();
    let total = cards + costs.crossing * crossings(graph) as f64;
    graph.length_cache.insert(key, total, costs.turn, costs.crossing);
    total
}

/// TALA `SyncClusters`, as every transaction calls it: the cards of every cluster stand in
/// their vessel in order again.
pub(crate) fn in_order(graph: &mut Graph) {
    for vessel in &mut graph.vessels {
        vessel.backwards = false;
    }
}

/// TALA `swapPositions`: the two cards exchange their top-left corners.
fn swap_positions(graph: &mut Graph, a: usize, b: usize) {
    (graph.nodes[a].top_left, graph.nodes[b].top_left) = (graph.nodes[b].top_left, graph.nodes[a].top_left);
}

/// TALA `smartSwapPositions`: the two cards exchange places so that the outer sides of the pair
/// stay where they were.
fn smart_swap_positions(graph: &mut Graph, a: usize, b: usize) {
    let (first, second) = (graph.nodes[a].clone(), graph.nodes[b].clone());
    let (x, y) = (first.top_left.x, first.top_left.y);
    let far_x = second.top_left.x + second.width - first.width;
    let far_y = second.top_left.y + second.height - first.height;
    let (back_x, back_y) = (x + first.width - second.width, y + first.height - second.height);
    let (to_a, to_b) = match first.orientation(&second) {
        Orientation::Left => (Point { x: far_x, y }, Point { x, y: second.top_left.y }),
        Orientation::Right => (Point { x: second.top_left.x, y }, Point { x: back_x, y: second.top_left.y }),
        Orientation::Top => (Point { x, y: far_y }, Point { x: second.top_left.x, y }),
        Orientation::Bottom => (Point { x, y: second.top_left.y }, Point { x: second.top_left.x, y: back_y }),
        Orientation::TopLeft => (Point { x: far_x, y: far_y }, Point { x, y }),
        Orientation::TopRight => (Point { x: second.top_left.x, y: far_y }, Point { x: back_x, y }),
        Orientation::BottomLeft => (Point { x: far_x, y: second.top_left.y }, Point { x, y: back_y }),
        Orientation::BottomRight => (second.top_left, Point { x: back_x, y: back_y }),
        Orientation::None => return,
    };
    graph.nodes[a].top_left = to_a;
    graph.nodes[b].top_left = to_b;
}

/// TALA `swapOptimize`: every card in turn swaps places with the card that makes the whole
/// diagram shortest, if that also makes the card itself cheaper and adds no crossing. Tree
/// cards, cards in a hierarchy and cards without connections stay. Says whether cards swapped.
fn swap_optimize(placed: &mut Placed) -> bool {
    let stays = |placed: &Placed, node: usize| node >= placed.tree_cards || placed.layered[node];
    let mut swapped = false;
    // TALA's one transaction for the stage: the cards too close or covering each other at its
    // start may stay so; an accepted swap does not change that (`existingOverlaps`).
    let allowed = Allowed::from(&placed.graph);
    for node in 0..placed.graph.nodes.len() {
        if stays(placed, node) || placed.graph.edges_of(node).next().is_none() {
            continue;
        }
        // The prices of a bend and of a crossing are taken when they are first asked for.
        let costs = placed.prices();
        let graph = &mut placed.graph;
        let now = (measure(graph, node, &costs), length(graph, &costs), crossings(graph));
        let mut best: Option<(usize, bool)> = None;
        let mut best_length = f64::INFINITY;
        let candidates: Vec<usize> = graph.cards_in_the_way().collect();
        for candidate in candidates {
            if candidate == node || stays(placed, candidate) {
                continue;
            }
            let graph = &mut placed.graph;
            // A swap that brings two cards too close is not looked at: its numbers stay 0. A
            // swap that is looked at is priced with the cards of every cluster in order: the
            // transaction that tries it has put them so.
            let tried = |graph: &mut Graph, smart: bool| {
                let before = (graph.nodes[node].top_left, graph.nodes[candidate].top_left, graph.vessels.clone());
                in_order(graph);
                if smart {
                    smart_swap_positions(graph, node, candidate);
                } else {
                    swap_positions(graph, node, candidate);
                }
                let result =
                    if allowed.now(graph) { (measure(graph, node, &costs), length(graph, &costs), crossings(graph)) } else { (0.0, 0.0, 0) };
                (graph.nodes[node].top_left, graph.nodes[candidate].top_left, graph.vessels) = before;
                result
            };
            let (plain, smart) = (tried(graph, false), tried(graph, true));
            if plain.1 == 0.0 && smart.1 == 0.0 {
                continue;
            }
            let less = |a: f64, b: f64| precision_compare(a, b) < 0;
            // With its own price, or for a card that has none, without.
            let better = |swap: (f64, f64, i64), own: bool| {
                swap.2 <= now.2 && (!own || less(swap.0, now.0)) && less(swap.1, now.1) && less(swap.1, best_length)
            };
            let take_smart = match (plain.0 != 0.0, smart.0 != 0.0) {
                (true, false) => better(plain, true).then_some(false),
                (false, true) => better(smart, true).then_some(true),
                (true, true) => {
                    if less(smart.0, plain.0) {
                        better(smart, true).then_some(true)
                    } else {
                        better(plain, true).then_some(false)
                    }
                }
                (false, false) => match (plain.1 != 0.0, smart.1 != 0.0) {
                    (true, false) => better(plain, false).then_some(false),
                    (false, true) => better(smart, false).then_some(true),
                    _ => {
                        if less(plain.1, smart.1) {
                            better(plain, false).then_some(false)
                        } else {
                            better(smart, false).then_some(true)
                        }
                    }
                },
            };
            if let Some(smart_one) = take_smart {
                best_length = if smart_one { smart.1 } else { plain.1 };
                best = Some((candidate, smart_one));
            }
        }
        if let Some((candidate, smart)) = best {
            swapped = true;
            if smart {
                smart_swap_positions(&mut placed.graph, node, candidate);
            } else {
                swap_positions(&mut placed.graph, node, candidate);
            }
            in_order(&mut placed.graph);
        }
    }
    swapped
}

/// TALA `Swap`: up to four rounds of swaps, then the whole diagram is mirrored so that most
/// arrows point right and down, unless that makes it longer.
pub fn swap(placed: &mut Placed) {
    // TALA takes the cell size anew here. No card has changed its size since node placement.
    for _ in 0..4 {
        if !swap_optimize(placed) {
            break;
        }
    }
    // TALA `direct` leaves a diagram alone whose first card is in a hierarchy.
    if placed.graph.nodes.is_empty() || placed.layered[0] {
        return;
    }
    let (mirror_x, mirror_y) = mirrors(&placed.graph, placed.graph.nodes.len());
    if !mirror_x && !mirror_y {
        return;
    }
    let costs = placed.prices();
    let (before, was) = ((placed.graph.nodes.clone(), placed.graph.vessels.clone(), placed.graph.tree_links.clone()), length(&placed.graph, &costs));
    let allowed = Allowed::from(&placed.graph);
    mirror(&mut placed.graph, mirror_x, mirror_y);
    in_order(&mut placed.graph);
    if !allowed.now(&placed.graph) || precision_compare(length(&placed.graph, &costs), was) > 0 {
        (placed.graph.nodes, placed.graph.vessels, placed.graph.tree_links) = before;
    }
}

/// TALA `transpose` as the stage after node placement calls it: the card and everything that
/// hangs on it on the far side of `center` turn around `center` by the quarter turn that makes
/// the whole diagram shortest, if one makes it shorter and brings no two cards too close. A
/// vessel that is turned may turn from a row into a column. Says whether anything turned.
fn transpose(placed: &mut Placed, node: usize) -> bool {
    let graph = &placed.graph;
    let adjacents: Vec<usize> = graph.edges_of(node).map(|edge| edge.other(node)).collect();
    let diagonal = |other: usize| is_diagonal(graph.nodes[node].orientation(&graph.nodes[other]));
    // TALA walks through tree cards but does not count them, and does not turn them: a card
    // that trees hang on leaves its trees behind.
    let tree_cards = placed.tree_cards;
    let reachable = |start: usize, ignore: usize| -> Vec<usize> { reachable(graph, start, ignore).into_iter().filter(|card| *card < tree_cards).collect() };
    let center = match adjacents[..] {
        [a] => {
            if diagonal(a) {
                return false;
            }
            a
        }
        [a, b] => {
            let from_a = reachable(a, node);
            // The card closes a ring: there are no two branches.
            if from_a.contains(&b) || diagonal(a) || diagonal(b) {
                return false;
            }
            // The smaller branch turns around the first card of the larger one.
            if from_a.len() >= reachable(b, node).len() { a } else { b }
        }
        _ => return false,
    };
    let turning = reachable(node, center);
    let costs = placed.prices();
    let graph = &mut placed.graph;
    let rotate = |graph: &mut Graph, times: usize| {
        for &card in &turning {
            rotate_around(graph, card, center, times, false);
            if let Some(vessel) = graph.vessels.iter().position(|vessel| vessel.node == card) {
                turn_cluster(graph, vessel);
            }
        }
        in_order(graph);
    };
    let mut best_length = length(graph, &costs);
    let mut best_times = None;
    let allowed = Allowed::from(graph);
    let before = (graph.nodes.clone(), graph.vessels.clone());
    for times in 1..=3 {
        rotate(graph, times);
        if allowed.now(graph) {
            let turned = length(graph, &costs);
            if precision_compare(turned, best_length) < 0 {
                best_length = turned;
                best_times = Some(times);
            }
        }
        (graph.nodes, graph.vessels) = before.clone();
    }
    let Some(times) = best_times else {
        return false;
    };
    rotate(graph, times);
    true
}

/// TALA `TransposeAll`: every card in turn. Tree cards, cards that trees hang on and cards in a
/// hierarchy stay.
pub fn transpose_all(placed: &mut Placed) {
    // TALA takes the cell size anew here too; the stage before changes no size.
    for node in 0..placed.graph.nodes.len() {
        if node >= placed.tree_cards || placed.layered[node] || placed.graph.trees_at.contains(&node) {
            continue;
        }
        transpose(placed, node);
    }
}
