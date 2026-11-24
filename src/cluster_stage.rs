// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.

//! The stage that puts every cluster right once all cards have their places: it turns to face
//! its neighbours, it and its neighbours are put in line, and the neighbours are pulled
//! closer. Ported from TALA `internal/placement/cluster_optimization.go` (`OptimizeClusters`,
//! `optimizeClustersAtomic`, `optimizeCluster` without `onlyFlip`, `alignVessel`,
//! `alignConnectedNodes`), `cluster_connections.go` and `axis.go` (`axisForArrangement`).

use crate::gaps::reduce_gap;
use crate::graph::{Arrangement, Graph, compute_cell_size};
use crate::optimizer::{Allowed, turn_cluster};
use crate::place::Placed;
use crate::placement::precision_compare;
use crate::swap::length;

/// TALA `optimizeCluster`: the cluster turns where its neighbours ask for it. A cluster that
/// stands as its neighbours ask is then put in line with them: its vessel moves to the middle
/// of the neighbours, or the neighbours across from it move to its middle, whichever leaves
/// the diagram shortest, if one makes it shorter. After that its neighbours are pulled
/// closer. Says whether anything changed.
fn optimize_cluster(placed: &mut Placed, which: usize) -> bool {
    // TALA's transactions put the cards of every cluster in order here too. They are in order
    // since the gap stage.
    let changed = turn_cluster(&mut placed.graph, which);
    let vessel = placed.graph.vessels[which].clone();
    if vessel.arrangement != vessel.desired {
        return changed;
    }
    let costs = placed.prices();
    let graph = &mut placed.graph;
    let row = vessel.arrangement == Arrangement::Row;
    // TALA `clusterExternalConnectedNodes`: the cards the cluster is connected to.
    let mut neighbours: Vec<usize> = Vec::new();
    for edge in graph.edges.iter().filter(|edge| edge.from == vessel.node || edge.to == vessel.node) {
        if !neighbours.contains(&edge.other(vessel.node)) {
            neighbours.push(edge.other(vessel.node));
        }
    }
    if neighbours.is_empty() {
        return changed;
    }
    // TALA `alignVessel`: the vessel moves along its row or column to the middle of its
    // neighbours.
    let move_vessel = |graph: &mut Graph| {
        let middle = |pick: fn(&crate::graph::Node) -> f64| neighbours.iter().map(|card| pick(&graph.nodes[*card])).sum::<f64>() / neighbours.len() as f64;
        let (x, y) = (middle(|card| card.center().x), middle(|card| card.center().y));
        let whole = &mut graph.nodes[vessel.node];
        if row {
            whole.top_left.x += (x - whole.width / 2.0 - whole.top_left.x).round();
        } else {
            whole.top_left.y += (y - whole.height / 2.0 - whole.top_left.y).round();
        }
    };
    // TALA `alignConnectedNodes`: every neighbour that stands across from the vessel moves to
    // the vessel's middle line.
    let move_neighbours = |graph: &mut Graph| {
        for &card in &neighbours {
            let whole = graph.nodes[vessel.node].clone();
            let node = &mut graph.nodes[card];
            if !node.faces(&whole, !row) {
                continue;
            }
            if row {
                node.top_left.x += (whole.top_left.x + whole.width / 2.0 - node.top_left.x - node.width / 2.0).round();
            } else {
                node.top_left.y += (whole.top_left.y + whole.height / 2.0 - node.top_left.y - node.height / 2.0).round();
            }
        }
    };
    let mut best = length(graph, &costs);
    let allowed = Allowed::from(graph);
    let before = graph.nodes.clone();
    // Both are tried from the same places. The neighbours win if they beat the vessel. (A try
    // moves cards only: the clusters need no putting back.)
    let mut shorter = |graph: &mut Graph, slide: &dyn Fn(&mut Graph)| {
        slide(graph);
        let slid = if allowed.now(graph) { length(graph, &costs) } else { f64::INFINITY };
        graph.nodes = before.clone();
        let wins = precision_compare(slid, best) < 0;
        if wins {
            best = slid;
        }
        wins
    };
    let vessel_wins = shorter(graph, &move_vessel);
    let neighbours_win = shorter(graph, &move_neighbours);
    if !vessel_wins && !neighbours_win {
        return changed;
    }
    if neighbours_win {
        move_neighbours(graph);
    } else {
        move_vessel(graph);
    }
    // The neighbours ahead of the vessel, across its cards, are pulled closer. (TALA checks
    // once more that the pull brings no two cards too close; the pull has checked that.)
    // Without a transaction of its own: the one the pull makes is anchored where it starts.
    let anchor = Allowed::from(&*graph);
    reduce_gap(graph, &costs, vessel.node, !row, true, true, &anchor);
    true
}

/// TALA `OptimizeClusters`: every cluster in turn. Says whether anything changed: the stage
/// that lines cards up runs again then.
pub fn optimize_clusters(placed: &mut Placed) -> bool {
    // A cluster may have turned since the cell size was taken.
    placed.graph.cell_size = compute_cell_size(&placed.graph.nodes);
    let mut changed = false;
    for which in 0..placed.graph.vessels.len() {
        changed |= optimize_cluster(placed, which);
    }
    changed
}
