// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.

//! Tunnels: direct lines between two connected cards that face each other, one per connection as
//! far as they fit. Ported from TALA `internal/routing/tunnel.go` (`buildTunnels`,
//! `buildTunnelsBetween`, `tunnelRangesBetweenChecked`), for plain boxes.

use crate::graph::{Graph, Point};
use crate::routing::tuning::SEGMENT_SPACING_BUFFER;

/// A tunnel: for each end the card and the point on its border.
pub(crate) struct Tunnel {
    pub a: (usize, Point),
    pub b: (usize, Point),
}

pub(crate) struct Range {
    pub(crate) start: f64,
    pub(crate) end: f64,
}

/// TALA `buildTunnels`: every pair of connected cards once, in the order of the cards and of
/// their connections. A card of a tree or of a cluster gets no tunnel.
pub(crate) fn build_tunnels(graph: &Graph) -> Vec<Tunnel> {
    let mut tunnels = Vec::new();
    let mut searched: Vec<(usize, usize)> = Vec::new();
    let special = |card: usize| graph.tree_link(card).is_some() || graph.cluster_of(card).is_some();
    for card in 0..graph.nodes.len() {
        if special(card) {
            continue;
        }
        for edge in graph.edges_of(card) {
            let other = edge.other(card);
            if other == card || special(other) || searched.contains(&(card, other)) {
                continue;
            }
            searched.push((card, other));
            searched.push((other, card));
            tunnels.extend(tunnels_between(graph, card, other));
        }
    }
    tunnels
}

/// TALA `buildTunnelsBetween`.
fn tunnels_between(graph: &Graph, a: usize, b: usize) -> Vec<Tunnel> {
    let Some((ranges, horizontally)) = ranges_between(graph, a, b) else {
        return Vec::new();
    };
    let (node_a, node_b) = (&graph.nodes[a], &graph.nodes[b]);
    let mut to_fit = graph
        .edges
        .iter()
        .filter(|edge| (edge.from == a && edge.to == b) || (edge.from == b && edge.to == a))
        .count() as f64;

    let mut tunnels = Vec::new();
    for range in ranges {
        let length = range.end - range.start;
        let fit = (length / SEGMENT_SPACING_BUFFER).floor().min(to_fit);
        let mut i = 1.0;
        while i <= fit {
            let value = (range.start + i * length / (fit + 1.0)).round();
            let (point_a, point_b) = if horizontally {
                if node_a.top_left.x > node_b.top_left.x {
                    (Point { x: node_a.top_left.x, y: value }, Point { x: node_b.right(), y: value })
                } else {
                    (Point { x: node_a.right(), y: value }, Point { x: node_b.top_left.x, y: value })
                }
            } else if node_a.top_left.y > node_b.top_left.y {
                (Point { x: value, y: node_a.top_left.y }, Point { x: value, y: node_b.bottom() })
            } else {
                (Point { x: value, y: node_a.bottom() }, Point { x: value, y: node_b.top_left.y })
            };
            tunnels.push(Tunnel { a: (a, point_a), b: (b, point_b) });
            to_fit -= 1.0;
            i += 1.0;
        }
    }
    tunnels
}

/// TALA `tunnelRangesBetweenChecked`: the stretches along which the two cards see each other, and
/// whether they do so side by side (`true`) or one above the other. `None` when they do not face
/// each other. TALA also gives up early when a card blocks the whole view (`IsBlocked`) and drops
/// stretches too short for a tunnel; both only skip work here, the tunnels are the same without.
pub(crate) fn ranges_between(graph: &Graph, a: usize, b: usize) -> Option<(Vec<Range>, bool)> {
    let (node_a, node_b) = (&graph.nodes[a], &graph.nodes[b]);
    let horizontally = node_a.faces(node_b, true);
    if !horizontally && !node_a.faces(node_b, false) {
        return None;
    }
    let mut ranges = vec![if horizontally {
        Range {
            start: node_a.top_left.y.max(node_b.top_left.y),
            end: node_a.bottom().min(node_b.bottom()),
        }
    } else {
        Range {
            start: node_a.top_left.x.max(node_b.top_left.x),
            end: node_a.right().min(node_b.right()),
        }
    }];

    // Only the cards of the group that is routed are asked (TALA takes them from the group's
    // own graph), not the cards of other groups that are in the way.
    for (n, other) in graph.nodes.iter().enumerate().skip(graph.nearby) {
        if n == a || n == b {
            continue;
        }
        let between = |first: f64, middle: f64, last: f64| {
            (first < middle && middle < last) || (last < middle && middle < first)
        };
        let in_between = if horizontally {
            between(node_a.top_left.x, other.top_left.x, node_b.top_left.x)
        } else {
            between(node_a.top_left.y, other.top_left.y, node_b.top_left.y)
        };
        if !in_between {
            continue;
        }
        // The stretch the other card covers.
        let (from, to) = if horizontally {
            (other.top_left.y, other.bottom())
        } else {
            (other.top_left.x, other.right())
        };
        let mut left: Vec<Range> = Vec::new();
        for range in ranges {
            if from <= range.start && to >= range.end {
                // Covered completely.
            } else if from > range.start && to < range.end {
                left.push(Range { start: range.start, end: from });
                left.push(Range { start: to, end: range.end });
            } else if from <= range.start && to < range.end && to > range.start {
                left.push(Range { start: to, end: range.end });
            } else if from > range.start && to >= range.end && from < range.end {
                left.push(Range { start: range.start, end: from });
            } else {
                left.push(range);
            }
        }
        ranges = left;
    }
    Some((ranges, horizontally))
}
