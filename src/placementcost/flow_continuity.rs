// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.

//! A card with a connection coming in and one going out reads best when the two continue each
//! other; connections of the same kind read best when they leave in clearly different
//! directions. Ported from TALA `internal/placementcost/flow_continuity.go`
//! (`flowContinuityCost`) and Go `math.Hypot`.

use crate::graph::Graph;

const INCOMING: u8 = 1;
const OUTGOING: u8 = 2;

/// Go `math.Hypot`, step by step: the library's own `hypot` rounds differently.
pub(crate) fn hypot(p: f64, q: f64) -> f64 {
    let (mut p, mut q) = (p.abs(), q.abs());
    if p < q {
        std::mem::swap(&mut p, &mut q);
    }
    if p == 0.0 {
        return 0.0;
    }
    q /= p;
    p * (1.0 + q * q).sqrt()
}

/// The card a ray points at (with the card of a cluster its connection belongs to), its
/// direction, and whether connections come in, go out or both.
type Ray = ((usize, Option<usize>), f64, f64, u8);

/// TALA `flowContinuityCost`, in bends.
pub fn flow_continuity_cost(graph: &Graph, node_index: usize, turn: f64) -> f64 {
    // TALA also leaves out a card with fewer than two connections: that one has no pair below.
    let count = graph.edges_of(node_index).count();
    if count > 8 {
        return 0.0;
    }
    let center = graph.nodes[node_index].center();
    // Per connected card: the direction to it, and whether connections come in, go out or both.
    let mut rays: Vec<Ray> = Vec::with_capacity(count);
    for (index, edge) in graph.indexed_edges_of(node_index) {
        if edge.from == edge.to {
            continue;
        }
        // A connection to a cluster is a ray to the cluster's card it belongs to. A vessel's own
        // connections belong to its cards: it has no rays.
        let behind = graph.member_behind(index);
        if behind.as_ref().is_some_and(|(vessel, _)| vessel.node == node_index) {
            continue;
        }
        let adjacent = (edge.other(node_index), graph.behind.get(index).copied().flatten());
        let direction = if edge.to == node_index { INCOMING } else { OUTGOING };
        if let Some(ray) = rays.iter_mut().find(|ray| ray.0 == adjacent) {
            ray.3 |= direction;
            continue;
        }
        let other = behind.map_or(graph.nodes[adjacent.0].center(), |(_, member)| member.center());
        let (x, y) = (other.x - center.x, other.y - center.y);
        let length = hypot(x, y);
        if length == 0.0 {
            continue;
        }
        rays.push((adjacent, x / length, y / length, direction));
    }
    let mut spine = f64::INFINITY;
    let mut branch_sum = 0.0;
    let mut branches = 0;
    for (i, first) in rays.iter().enumerate() {
        for second in &rays[i + 1..] {
            let dot = (first.1 * second.1 + first.2 * second.2).clamp(-1.0, 1.0);
            let (a, b) = (first.3, second.3);
            if (a & INCOMING != 0 && b & OUTGOING != 0) || (a & OUTGOING != 0 && b & INCOMING != 0) {
                // 0 when the one continues the other straight through the card.
                spine = spine.min(1.0 + dot);
            }
            if a & b != 0 {
                // Less than 60 degrees apart: they compete for the same side of the card.
                branch_sum += (2.0 * dot - 1.0).max(0.0);
                branches += 1;
            }
        }
    }
    let mut cost = 0.0;
    if spine.is_finite() {
        cost += spine;
    }
    if branches > 0 {
        cost += branch_sum / f64::from(branches);
    }
    turn * cost
}
