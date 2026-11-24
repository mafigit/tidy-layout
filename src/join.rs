// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.

//! Pulling groups of cards together that lie far apart though connections join them. Ported
//! from TALA `internal/grouping/join.go` (`JoinDistancedClusters`), `internal/layoutgraph`:
//! `node.go` (`clustersWithWorkGuard`, `boundingBox`, `distance`), `graph.go`
//! (`doesOverlapWithDimensionsContext`), and D2 `lib/geo` (`Points.GetMedian`,
//! `Point.GetOrientation`). For plain cards; no fixed places, no "nears".

use crate::compaction::overlaps_at;
use crate::graph::{Graph, Point};
use crate::placement::precision_compare;
use crate::routing::coordinator::length;
use crate::routing::geometry::euclidean_distance;

/// TALA `clustersWithWorkGuard`: the groups of cards that lie within `threshold` of each other,
/// if any connection is longer than that. Each group in the order of the cards.
fn distance_clusters(graph: &Graph, threshold: f64) -> Option<Vec<Vec<usize>>> {
    let count = graph.nodes.len();
    let far = |a: usize, b: usize| precision_compare(length(&graph.nodes[a], &graph.nodes[b]), threshold) > 0;
    // Both ends of every connection that is too long start a group.
    let mut seeds = Vec::new();
    for node in 0..count {
        for edge in graph.edges_of(node) {
            let adjacent = edge.other(node);
            if far(node, adjacent) {
                seeds.extend([node, adjacent]);
            }
        }
    }
    if seeds.len() < 2 {
        return None;
    }
    let mut cluster_of: Vec<Option<usize>> = vec![None; count];
    // A card takes every card near it into its group, and those take the cards near them. A card
    // that had a group already changes over but brings none along.
    fn merge(node: usize, cluster_of: &mut [Option<usize>], far: &dyn Fn(usize, usize) -> bool) {
        for other in 0..cluster_of.len() {
            if other == node || far(node, other) || cluster_of[other] == cluster_of[node] {
                continue;
            }
            let new = cluster_of[other].is_none();
            cluster_of[other] = cluster_of[node];
            if new {
                merge(other, cluster_of, far);
            }
        }
    }
    let mut next = 0;
    for node in seeds {
        if cluster_of[node].is_none() {
            cluster_of[node] = Some(next);
            next += 1;
        }
        merge(node, &mut cluster_of, &far);
    }
    let clusters: Vec<Vec<usize>> = (0..next)
        // TALA sorts each group by the cards' numbers; nothing below depends on that order.
        .map(|cluster| (0..count).filter(|node| cluster_of[*node] == Some(cluster)).collect::<Vec<_>>())
        .filter(|cluster| !cluster.is_empty())
        .collect();
    Some(clusters)
}

/// TALA `Nodes.boundingBox`: the corners of the box around a group of cards.
fn bounds(graph: &Graph, cluster: &[usize]) -> (Point, Point) {
    let mut top_left = Point { x: f64::INFINITY, y: f64::INFINITY };
    let mut bottom_right = Point { x: f64::NEG_INFINITY, y: f64::NEG_INFINITY };
    for &node in cluster {
        let card = &graph.nodes[node];
        top_left = Point { x: top_left.x.min(card.top_left.x), y: top_left.y.min(card.top_left.y) };
        bottom_right = Point { x: bottom_right.x.max(card.right().round()), y: bottom_right.y.max(card.bottom().round()) };
    }
    (top_left, bottom_right)
}

fn center(graph: &Graph, cluster: &[usize]) -> Point {
    let (top_left, bottom_right) = bounds(graph, cluster);
    Point { x: top_left.x + (bottom_right.x - top_left.x) / 2.0, y: top_left.y + (bottom_right.y - top_left.y) / 2.0 }
}

/// D2 `Points.GetMedian` along one axis.
fn median(mut values: Vec<f64>) -> f64 {
    values.sort_by(f64::total_cmp);
    let middle = values.len() / 2;
    if values.len().is_multiple_of(2) { (values[middle] + values[middle - 1]) / 2.0 } else { values[middle] }
}

/// One cell towards `target` for a whole group, if no card of it then comes too close to a card
/// outside it. Says whether it moved.
fn inch_towards(graph: &mut Graph, cluster: &[usize], target: Point) -> bool {
    let (top_left, bottom_right) = bounds(graph, cluster);
    let middle = center(graph, cluster);
    let distance = euclidean_distance(middle.x, middle.y, target.x, target.y);
    let longer_side = (bottom_right.x - top_left.x).max(bottom_right.y - top_left.y);
    // Close enough: the target lies well inside the group.
    if distance < longer_side / 2.0 {
        return false;
    }
    let step = |from: f64, to: f64| if from < to { 1.0 } else if from > to { -1.0 } else { 0.0 };
    let (dx, dy) = (step(middle.x, target.x) * graph.cell_size, step(middle.y, target.y) * graph.cell_size);
    if dx == 0.0 && dy == 0.0 {
        return false;
    }
    let overlaps = cluster.iter().any(|&node| {
        let at = graph.nodes[node].top_left;
        let point = Point { x: at.x + dx, y: at.y + dy };
        (0..graph.nodes.len()).any(|other| !cluster.contains(&other) && overlaps_at(graph, node, other, point))
    });
    if overlaps {
        return false;
    }
    for &node in cluster {
        let at = graph.nodes[node].top_left;
        graph.nodes[node].top_left = Point { x: at.x + dx, y: at.y + dy };
    }
    true
}

/// TALA `JoinDistancedClusters`: groups of cards more than three cells apart move, a cell at a
/// time, towards the median of the groups' middles, as long as one of them can.
pub fn join_distanced_clusters(graph: &mut Graph) {
    if graph.nodes.len() <= 1 || graph.edges.is_empty() {
        return;
    }
    let Some(clusters) = distance_clusters(graph, 3.0 * graph.cell_size) else {
        return;
    };
    let centers: Vec<Point> = clusters.iter().map(|cluster| center(graph, cluster)).collect();
    let target = Point {
        x: median(centers.iter().map(|center| center.x).collect()),
        y: median(centers.iter().map(|center| center.y).collect()),
    };
    for _ in 0..1000 {
        let mut moved = false;
        for cluster in &clusters {
            moved |= inch_towards(graph, cluster, target);
        }
        if !moved {
            break;
        }
    }
}
