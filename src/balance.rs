// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.

//! The stage that puts a card in the middle of its neighbours where they all stand in one line
//! on one side of it. Ported from TALA `internal/placement/symmetry.go` (`BalanceSymmetry`,
//! `isSimple`), `internal/placementcost/axis.go` (`AxisScore`) and D2 `lib/geo/orientation.go`
//! (`SameSide`).

use crate::graph::{Graph, Node};
use crate::optimizer::Allowed;
use crate::place::{Placed, bounds};
use crate::routing::geometry::{Orientation, is_diagonal};

/// D2 `Orientation.SameSide`: both lie above, both below, both left or both right.
fn same_side(first: Orientation, second: Orientation) -> bool {
    use Orientation::{Bottom, BottomLeft, BottomRight, Left, Right, Top, TopLeft, TopRight};
    let sides = [[TopLeft, Top, TopRight], [BottomLeft, Bottom, BottomRight], [Left, TopLeft, BottomLeft], [Right, TopRight, BottomRight]];
    sides.iter().any(|side| side.contains(&first) && side.contains(&second))
}

/// TALA `AxisScore`: 1 for cards that stand in one row or one column, less the more they are
/// out of line, 0 for cards in neither.
fn axis_score(graph: &Graph, cards: &[usize]) -> f64 {
    let node = |card: usize| &graph.nodes[card];
    if cards.len() < 2 {
        return 1.0;
    }
    if cards.len() == 2 {
        return if is_diagonal(node(cards[0]).orientation(node(cards[1]))) { 0.0 } else { 1.0 };
    }
    // The first of the highest cards, and the first of the widest.
    let largest = |side: fn(&Node) -> f64| cards.iter().copied().fold(cards[0], |best, card| if side(node(card)) > side(node(best)) { card } else { best });
    // The cards are measured against the largest one: three lines run through it, at a quarter,
    // a half and three quarters. A card counts for the lines that meet it; a small card needs
    // fewer of them.
    let score = |largest: usize, start: fn(&Node) -> f64, size: fn(&Node) -> f64| {
        let mut score = 0.0;
        for &card in cards.iter().filter(|card| **card != largest) {
            let mut card_score = 0.0;
            for fraction in [0.25, 0.5, 0.75] {
                let line = start(node(largest)) + fraction * size(node(largest));
                // TALA `OverlapsLine`, for a line that ends at the card's middle.
                if start(node(card)) <= line && line <= start(node(card)) + size(node(card)) {
                    card_score += 0.33;
                }
            }
            if size(node(card)) < 0.25 * size(node(largest)) {
                card_score *= 3.0;
            } else if size(node(card)) < 0.75 * size(node(largest)) {
                card_score *= 2.0;
            }
            score += if card_score == 0.99 { 1.0 } else { card_score };
        }
        (score / (cards.len() - 1) as f64).max(0.33)
    };
    let highest = largest(|card| card.height);
    if cards.iter().all(|card| *card == highest || matches!(node(*card).orientation(node(highest)), Orientation::Left | Orientation::Right)) {
        return score(highest, |card| card.top_left.y, |card| card.height);
    }
    let widest = largest(|card| card.width);
    if cards.iter().all(|card| *card == widest || matches!(node(*card).orientation(node(widest)), Orientation::Top | Orientation::Bottom)) {
        return score(widest, |card| card.top_left.x, |card| card.width);
    }
    0.0
}

/// TALA `BalanceSymmetry`: every plain card with two or more connections whose neighbours all
/// stand on one side of it, in one line, and are of like size, moves along that line to the
/// middle of them, if it comes too close to no card there.
pub fn balance_symmetry(placed: &mut Placed) {
    for card in 0..placed.graph.nodes.len() {
        let graph = &mut placed.graph;
        // TALA `isSimple`: no card of a hierarchy, no card trees hang on, no vessel.
        if placed.layered[card] || graph.trees_at.contains(&card) || graph.vessel_at(card).is_some() {
            continue;
        }
        // TALA passes over a card with fewer than two connections here. It has fewer than two
        // neighbours, and is passed over below.
        let others: Vec<usize> = graph.edges_of(card).map(|edge| edge.other(card)).collect();
        let side = |other: usize| graph.nodes[card].orientation(&graph.nodes[other]);
        if others.windows(2).any(|pair| !same_side(side(pair[0]), side(pair[1]))) {
            continue;
        }
        // TALA keeps the neighbours in a map, so in no order.
        let mut neighbours: Vec<usize> = Vec::new();
        for other in others {
            if !neighbours.contains(&other) {
                neighbours.push(other);
            }
        }
        if neighbours.len() < 2 || axis_score(graph, &neighbours) != 1.0 {
            continue;
        }
        // Neighbours of unlike size do not look symmetric.
        let area = |other: &usize| graph.nodes[*other].width * graph.nodes[*other].height;
        let largest = neighbours.iter().map(area).fold(f64::NEG_INFINITY, f64::max);
        if neighbours.iter().any(|other| area(other) < 0.5 * largest) {
            continue;
        }
        let across = !matches!(graph.nodes[neighbours[0]].orientation(&graph.nodes[neighbours[1]]), Orientation::Top | Orientation::Bottom);
        // TALA's transaction puts the cards of every cluster in order here too. They are in
        // order since the gap stage.
        let allowed = Allowed::from(graph);
        let before = graph.nodes[card].top_left;
        let (top_left, bottom_right) = bounds(graph, &neighbours);
        let middle = graph.nodes[card].center();
        if across {
            graph.nodes[card].top_left.x += (top_left.x + (bottom_right.x - top_left.x) / 2.0 - middle.x).floor();
        } else {
            graph.nodes[card].top_left.y += (top_left.y + (bottom_right.y - top_left.y) / 2.0 - middle.y).floor();
        }
        if !allowed.now(graph) {
            graph.nodes[card].top_left = before;
        }
    }
}
