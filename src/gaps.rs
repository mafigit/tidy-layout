// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.

//! The stage that closes large gaps: where a card's nearest connected card along an axis is
//! further away than it needs to be, that card and everything connected to it is pulled
//! closer, if that makes the whole diagram shorter. Ported from TALA `internal/placement`:
//! `stages.go` (`NormalizeGaps`) and `gapreduction.go` (`gapNormalization`, `isBetween`,
//! `nearestBetween`, `nearestConnectedAhead`, `reduceGapToNeighbors` for cards in no
//! container).

use crate::align::connected;
use crate::compaction::{Options, is_blocked};
use crate::graph::{Graph, Node, Point};
use crate::optimizer::Allowed;
use crate::place::Placed;
use crate::placementcost::{edge_length::node_edge_length_any_way, symmetry::node_symmetry};
use crate::routing::router::Costs;
use crate::swap::{crossings, in_order};

/// TALA `largeGapThreshold`: a gap of half a cell or less is left as it is.
const LARGE_GAP_THRESHOLD: f64 = 0.5;
/// TALA `IdealGapSize`: the gap a card is pulled to.
const IDEAL_GAP_SIZE: f64 = 150.0;

/// A card's start and end along the axis.
fn span(card: &Node, horizontal: bool) -> (f64, f64) {
    if horizontal { (card.top_left.x, card.right()) } else { (card.top_left.y, card.bottom()) }
}

/// TALA `EdgeLength` with sizes and without the direction penalty: what all cards cost where
/// the arrows may point any way, and every crossing.
fn length(graph: &Graph, costs: &Costs) -> f64 {
    let key = graph.length_key(false);
    if let Some(known) = graph.length_cache.get(&key) {
        return known;
    }
    let total = uncached_length(graph, costs);
    graph.length_cache.insert(key, total, costs.turn, costs.crossing);
    total
}

fn uncached_length(graph: &Graph, costs: &Costs) -> f64 {
    let cards: f64 = (0..graph.nodes.len())
        .map(|node| {
            let length = node_edge_length_any_way(graph, node, costs.turn);
            length - node_symmetry(graph, node) * graph.cell_size * graph.edges_of(node).count() as f64
        })
        .sum();
    cards + costs.crossing * crossings(graph) as f64
}

/// TALA `nearestConnectedAhead`: of the cards the card is connected to that lie wholly ahead
/// of it along the axis, the nearest.
fn nearest_connected_ahead(graph: &Graph, node: usize, horizontal: bool, forwards: bool) -> Option<usize> {
    let (start, end) = span(&graph.nodes[node], horizontal);
    let mut nearest: Option<usize> = None;
    for edge in graph.edges_of(node) {
        let other = edge.other(node);
        let (other_start, other_end) = span(&graph.nodes[other], horizontal);
        let nearer = |best: usize| {
            let (best_start, best_end) = span(&graph.nodes[best], horizontal);
            if forwards { other_start < best_start } else { other_end > best_end }
        };
        let ahead = if forwards { other_start >= end } else { other_end <= start };
        if ahead && nearest.is_none_or(nearer) {
            nearest = Some(other);
        }
    }
    nearest
}

/// TALA `isBetween`: the card lies between `behind` and `ahead` along the axis, and across the
/// axis no further from `behind` than the gap the two must keep.
fn is_between(graph: &Graph, node: usize, behind: usize, ahead: usize, horizontal: bool, forwards: bool) -> bool {
    let delta = graph.gap_between(behind, node, graph.nodes[behind].top_left);
    let (start, end) = span(&graph.nodes[node], !horizontal);
    let (behind_start, behind_end) = span(&graph.nodes[behind], !horizontal);
    if end < behind_start - delta || start > behind_end + delta {
        return false;
    }
    let (behind, ahead) = if forwards { (behind, ahead) } else { (ahead, behind) };
    let (start, end) = span(&graph.nodes[node], horizontal);
    end >= span(&graph.nodes[behind], horizontal).1 && start <= span(&graph.nodes[ahead], horizontal).0
}

/// TALA `nearestBetween`: of `cards`, the one between the two that is nearest to `behind`.
fn nearest_between(graph: &Graph, cards: &[usize], behind: usize, ahead: usize, horizontal: bool, forwards: bool) -> Option<usize> {
    let mut nearest: Option<usize> = None;
    for &card in cards {
        if card == behind || card == ahead || !is_between(graph, card, behind, ahead, horizontal, forwards) {
            continue;
        }
        let (start, end) = span(&graph.nodes[card], horizontal);
        let nearer = |best: usize| {
            let (best_start, best_end) = span(&graph.nodes[best], horizontal);
            if forwards { start < best_start } else { end > best_end }
        };
        if nearest.is_none_or(nearer) {
            nearest = Some(card);
        }
    }
    nearest
}

/// TALA `reduceGapToNeighbors` for a card in no container: the nearest connected card ahead,
/// with everything connected to it, is pulled to the ideal gap from the card, or from a card
/// that stands between the two. With `recover` the same is then tried on the card's other
/// side, which is kept where it makes the diagram shorter still. Says whether cards moved, and
/// how long the diagram is then.
pub(crate) fn reduce_gap(graph: &mut Graph, costs: &Costs, node: usize, horizontal: bool, forwards: bool, recover: bool, allowed: &Allowed) -> (bool, f64) {
    let Some(first_ahead) = nearest_connected_ahead(graph, node, horizontal, forwards) else {
        return (false, 0.0);
    };
    let not_the_card: Vec<bool> = (0..graph.nodes.len()).map(|card| card == node).collect();
    let pulled = connected(graph, first_ahead, &not_the_card);
    let ahead = nearest_between(graph, &pulled, node, first_ahead, horizontal, forwards).unwrap_or(first_ahead);
    let old = length(graph, costs);
    let mut new = old;
    let options = Options { horizontal, sizes: true, factor: 1.0, transition: false };
    let between: Vec<usize> = (0..graph.nodes.len())
        .filter(|&card| card != node && card != ahead && !pulled.contains(&card))
        .filter(|&card| if forwards { is_blocked(graph, card, node, ahead, &options) } else { is_blocked(graph, card, ahead, node, &options) })
        .collect();
    for candidate in std::iter::once(node).chain(between) {
        let (start, end) = span(&graph.nodes[candidate], horizontal);
        let (ahead_start, ahead_end) = span(&graph.nodes[ahead], horizontal);
        let gap = if forwards { ahead_start - end } else { start - ahead_end };
        if gap <= LARGE_GAP_THRESHOLD * graph.cell_size {
            continue;
        }
        let delta = if forwards { IDEAL_GAP_SIZE - gap } else { gap - IDEAL_GAP_SIZE };
        if delta == 0.0 {
            continue;
        }
        let before = (graph.nodes.clone(), graph.vessels.clone());
        for &card in &pulled {
            let at = graph.nodes[card].top_left;
            graph.nodes[card].top_left = if horizontal { Point { x: at.x + delta, y: at.y } } else { Point { x: at.x, y: at.y + delta } };
        }
        in_order(graph);
        let pulled_length = length(graph, costs);
        // The other side is tried before this pull is checked, and kept only where it moved
        // something and leaves the diagram shorter than at the start. (TALA also asks that it
        // is shorter than after the pull and brings no two cards too close: a pull on the
        // other side that moved something has passed both.)
        let mut recovered = false;
        if recover {
            let after_pull = (graph.nodes.clone(), graph.vessels.clone());
            let (moved, mirrored) = reduce_gap(graph, costs, node, horizontal, !forwards, false, allowed);
            recovered = moved && mirrored < old;
            if !recovered {
                (graph.nodes, graph.vessels) = after_pull;
            }
        }
        // A pull that does not make the diagram shorter ends the search: none further on will.
        if !recovered && pulled_length >= old {
            (graph.nodes, graph.vessels) = before;
            break;
        }
        // A pull that brings two cards too close is taken back; the next candidate is tried.
        // Only the cards moved since this try began are asked: in the pull on the other side,
        // not those the first pull moved.
        if !allowed.now_since(graph, &before.0) {
            (graph.nodes, graph.vessels) = before;
            continue;
        }
        new = pulled_length;
        break;
    }
    (new != old, new)
}

/// TALA `gapNormalization`: every card in turn, the cards with the most connections first.
/// Tree cards and cards in a hierarchy pull nothing.
fn normalize(placed: &mut Placed, costs: &Costs, horizontal: bool, forwards: bool, allowed: &Allowed) -> bool {
    let graph = &placed.graph;
    let mut order: Vec<usize> = (0..graph.nodes.len()).collect();
    order.sort_by_key(|&card| (std::cmp::Reverse(graph.edges_of(card).count()), graph.id(card)));
    let mut changed = false;
    for node in order {
        if node >= placed.tree_cards || placed.layered[node] {
            continue;
        }
        reduce_gap(&mut placed.graph, costs, node, horizontal, forwards, true, allowed);
        // TALA counts every transaction that was not refused, also one that moved nothing. Its
        // commit puts the cards of every cluster in order, also where nothing moved.
        in_order(&mut placed.graph);
        changed = true;
    }
    changed
}

/// TALA `NormalizeGaps`: across, forwards and backwards, then down. Says whether a card was
/// looked at: the stage that lines cards up runs again then.
pub fn normalize_gaps(placed: &mut Placed) -> bool {
    let costs = placed.prices();
    // TALA's transaction for the stage: the cards that are too close or cover each other at its
    // start may stay so in every try of the stage, also the nested ones; others may not
    // (`existingOverlaps` stay anchored where the transaction was made).
    let allowed = Allowed::from(&placed.graph);
    let mut changed = false;
    for horizontal in [true, false] {
        for forwards in [true, false] {
            changed |= normalize(placed, &costs, horizontal, forwards, &allowed);
        }
    }
    // TALA `ResetTurnCost`: the price of a bend is taken anew by the next stage that asks.
    placed.turn = None;
    placed.graph.length_cache.forget_turn();
    in_order(&mut placed.graph);
    changed
}
