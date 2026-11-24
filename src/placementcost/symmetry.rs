// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.

//! How symmetric the cards around a card lie. Ported from TALA
//! `internal/placementcost/symmetry.go`: `NodeSymmetry`, `nodeSymmetry`,
//! `computeSymmetryScoreInto`, `obstructed` and `isMirrored`.

use crate::graph::{Graph, Node};

/// Cards further away than this have no say in a card's symmetry.
const MAX_DISTANCE: f64 = 1200.0;
/// TALA `symmetryToleranceBand`, in cells.
const TOLERANCE_BAND: f64 = 1.0;

/// TALA `NodeSymmetry`: 0 for none; 1 when every connected card has a mirror image on the
/// other side of the card. More when the card also makes its neighbours symmetric.
pub fn node_symmetry(graph: &Graph, node: usize) -> f64 {
    symmetry(graph, node, true)
}

fn symmetry(graph: &Graph, node_index: usize, check_neighbors: bool) -> f64 {
    let node = &graph.nodes[node_index];
    let mut neighbors: Vec<usize> = Vec::new();
    for edge in graph.edges_of(node_index) {
        let adjacent = edge.other(node_index);
        if !neighbors.contains(&adjacent) {
            neighbors.push(adjacent);
        }
    }
    neighbors.retain(|adjacent| node.distance_to(&graph.nodes[*adjacent]) <= MAX_DISTANCE);
    let (mut score, matched) = score_of(graph, node_index, &neighbors);
    if check_neighbors {
        // A card without a mirror image of its own may be the mirror image of another: it
        // should stay where it makes its neighbour symmetric.
        for (adjacent, matched) in neighbors.iter().zip(&matched) {
            if !matched {
                score += symmetry(graph, *adjacent, false);
            }
        }
    }
    if neighbors.is_empty() {
        return 0.0;
    }
    score / neighbors.len() as f64
}

/// TALA `computeSymmetryScoreInto`: pairs of neighbours that mirror each other across the card
/// score 2 when they lie in line with it, 0.5 otherwise. Also which neighbours found a partner.
fn score_of(graph: &Graph, node_index: usize, neighbors: &[usize]) -> (f64, Vec<bool>) {
    let node = &graph.nodes[node_index];
    let mut matched = vec![false; neighbors.len()];
    let mut score = 0.0;
    for i in 0..neighbors.len() {
        if matched[i] {
            continue;
        }
        let first = &graph.nodes[neighbors[i]];
        let mut best: Option<(usize, f64)> = None;
        for j in i + 1..neighbors.len() {
            if matched[j] {
                continue;
            }
            let second = &graph.nodes[neighbors[j]];
            // Cards of very different size do not look like mirror images.
            let (first_area, second_area) = (first.width * first.height, second.width * second.height);
            if first_area > 2.0 * second_area || second_area > 2.0 * first_area {
                continue;
            }
            let mut mirror = None;
            for across_x in [true, false] {
                let axis = if across_x { node.center().x } else { node.center().y };
                if is_mirrored(first, second, across_x, axis, graph.cell_size) {
                    let in_line = node.faces(first, across_x) && node.faces(second, across_x);
                    mirror = Some(if in_line { 2.0 } else { 0.5 });
                    break;
                }
            }
            let Some(mirror) = mirror else { continue };
            if obstructed(graph, node_index, neighbors[i], neighbors[j]) {
                continue;
            }
            if mirror > best.map_or(0.0, |(_, score)| score) {
                best = Some((j, mirror));
            }
        }
        if let Some((j, mirror)) = best {
            matched[i] = true;
            matched[j] = true;
            score += mirror;
        }
    }
    (score, matched)
}

/// TALA `obstructed`: another card lies between the card and one of the two.
fn obstructed(graph: &Graph, center: usize, a: usize, b: usize) -> bool {
    let from = graph.nodes[center].center();
    graph.nodes.iter().enumerate().any(|(index, other)| {
        index != center
            && index != a
            && index != b
            && (other.passes_through(from, graph.nodes[a].center())
                || other.passes_through(from, graph.nodes[b].center()))
    })
}

/// TALA `isMirrored`: the two cards lie on opposite sides of the axis, about as far from it,
/// and about in line with each other. `across_x`: the axis is a column at x = `axis`.
fn is_mirrored(a: &Node, b: &Node, across_x: bool, axis: f64, cell_size: f64) -> bool {
    let tolerance = TOLERANCE_BAND * cell_size;
    // The cards along the axis' own direction (`along`) and across it (`start`, `end`).
    let span = |node: &Node| {
        if across_x {
            (node.top_left.x, node.right(), node.top_left.y, node.bottom())
        } else {
            (node.top_left.y, node.bottom(), node.top_left.x, node.right())
        }
    };
    let (a_start, a_end, a_along_start, a_along_end) = span(a);
    let (b_start, b_end, b_along_start, b_along_end) = span(b);
    // Two cards that start on the same line have no axis between them: the test below fails.
    let (near_end, far_start) = if a_start > b_start { (b_end, a_start) } else { (a_end, b_start) };
    if !(axis > near_end && axis < far_start) {
        return false;
    }
    if ((far_start - axis) - (axis - near_end)).abs() > tolerance {
        return false;
    }
    (((a_along_start + a_along_end) / 2.0) - ((b_along_start + b_along_end) / 2.0)).abs() <= tolerance
}
