// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.

//! The stage `Crosshatch`, right after the first routing: where two or more connections of a
//! cluster leave or reach one of its cards at the same port, each becomes a straight line from
//! the border of one card to the border of the other, on the line between their middles,
//! unless that line runs through a third card. Ported from TALA `internal/routing`:
//! `coordinator.go` (`crosshatchWithWorkLimit`, `externalEdgesForClusterGuarded`,
//! `groupEdgesByClusterPortGuarded`, `convertToStraightLineGuarded`,
//! `findLineBorderIntersection`, `straightLineIntersectsNonAncestorNodesGuarded`). No
//! containers.

use crate::graph::{Graph, Node, Point};

/// TALA `findLineBorderIntersection`: where the line from the middle of a card towards a point
/// leaves the card.
fn border_towards(center: Point, towards: Point, node: &Node) -> Point {
    let (dx, dy) = (towards.x - center.x, towards.y - center.y);
    if dx == 0.0 && dy == 0.0 {
        return center;
    }
    let mut t = f64::INFINITY;
    if dx > 0.0 {
        t = t.min((node.right() - center.x) / dx);
    } else if dx < 0.0 {
        t = t.min((node.top_left.x - center.x) / dx);
    }
    if dy > 0.0 {
        t = t.min((node.bottom() - center.y) / dy);
    } else if dy < 0.0 {
        t = t.min((node.top_left.y - center.y) / dy);
    }
    if t == f64::INFINITY || t <= 0.0 {
        return center;
    }
    Point { x: center.x + t * dx, y: center.y + t * dy }
}

/// TALA `Crosshatch`. The clusters in their order; per cluster its connections (TALA
/// `EdgeAbductions`: every connection of one of its cards) grouped by the port they have at
/// that card. Each connection is changed on its own, so the order within a group does not
/// matter.
pub fn crosshatch(graph: &Graph, routes: &mut [Vec<Point>]) {
    for cluster in &graph.clusters {
        let edges: Vec<usize> = (0..graph.edges.len()).filter(|edge| cluster.members.contains(&graph.edges[*edge].from) || cluster.members.contains(&graph.edges[*edge].to)).collect();
        if edges.len() < 2 {
            continue;
        }
        // The port at the card of the cluster: the start of the route where the connection
        // leaves one, else its end.
        let port_of = |edge: usize| {
            let route = &routes[edge];
            if cluster.members.contains(&graph.edges[edge].from) { route.first().copied() } else { route.last().copied() }
        };
        let mut groups: Vec<(Point, Vec<usize>)> = Vec::new();
        for &edge in &edges {
            let Some(port) = port_of(edge) else { continue };
            match groups.iter_mut().find(|(other, _)| *other == port) {
                Some((_, group)) => group.push(edge),
                None => groups.push((port, vec![edge])),
            }
        }
        for (_, group) in groups.iter().filter(|(_, group)| group.len() >= 2) {
            for &edge in group {
                let wire = graph.edges[edge];
                let (from, to) = (&graph.nodes[wire.from], &graph.nodes[wire.to]);
                let middle = |node: &Node| Point { x: node.top_left.x + node.width / 2.0, y: node.top_left.y + node.height / 2.0 };
                let (from_center, to_center) = (middle(from), middle(to));
                let (start, end) = (border_towards(from_center, to_center, from), border_towards(to_center, from_center, to));
                let blocked = graph.nodes.iter().enumerate().any(|(card, node)| card != wire.from && card != wire.to && node.passes_through(start, end));
                if !blocked {
                    routes[edge] = vec![start, end];
                }
            }
        }
    }
}
