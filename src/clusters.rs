// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.

//! Clusters: cards of similar size that are connected to the same cards in the same way. TALA
//! puts them in a row or a column and places that as one card, the "vessel". Ported from TALA
//! `internal/grouping`: `clusters.go` (`AddClusters`, `AssignArrangement`, `PaddingBetween`,
//! `CreateVessel`, `AddCluster`, `abductClusterEdges`), `cluster_discovery.go`
//! (`buildClusterDiscoveryIndex`, `refreshNeighbors`, `clusterEdgeSignature`), and
//! `internal/layoutgraph/cluster.go` (`Resize`).

use crate::gorand::GoRand;
pub use crate::graph::Arrangement;
use crate::graph::{Edge, Graph, Node, Vessel};
use crate::policy::NODE_GAP;

/// TALA `maxSizeDiff`: a card four times as wide or as high as another is not like it.
const MAX_SIZE_DIFF: f64 = 4.0;

/// TALA `Cluster`.
#[derive(Clone, Debug, PartialEq)]
pub struct Cluster {
    /// TALA's number of the vessel: drawn at random, so far above the numbers of the cards.
    pub vessel: i64,
    /// The cards, by their place in the graph the clusters were found in, in the order found.
    pub members: Vec<usize>,
    pub arrangement: Arrangement,
    /// The room between two cards.
    pub padding: f64,
}

/// Where a connection ends once clusters are made: at a card, or at the vessel of a cluster.
#[derive(Clone, Copy, PartialEq)]
enum End {
    Card(usize),
    Vessel(usize),
}

/// TALA `AddClusters` for cards in no container. `levels` says which cards are in a hierarchy
/// (`find_hierarchies`), `cards` is the number of cards in the whole diagram, trees included:
/// their numbers are taken.
///
/// A card starts a cluster with every later or earlier card that is like it: no side four times
/// the other's, the same cards at the other end of its connections, and as many connections
/// from it and to it. Cards in a hierarchy, cards that trees hang on and cards connected to a
/// cluster made before are left alone.
pub fn find_clusters(graph: &Graph, levels: &[Option<usize>], cards: usize, seed: i64) -> Vec<Cluster> {
    let count = graph.nodes.len();
    // TALA also leaves out a card with a connection to itself; the engine is given none.
    let excluded = |card: usize| levels[card].is_some() || graph.trees_at.contains(&card);
    // TALA `clusterEdgeSignature`, for arrows that all have one head: how many leave the card
    // and how many reach it.
    let signature = |card: usize| {
        (graph.edges.iter().filter(|edge| edge.from == card).count(), graph.edges.iter().filter(|edge| edge.to == card).count())
    };
    let mut ends: Vec<(End, End)> = graph.edges.iter().map(|edge| (End::Card(edge.from), End::Card(edge.to))).collect();
    // TALA `refreshNeighbors`: the cards and vessels a card's connections lead to, each once.
    let neighbours = |ends: &[(End, End)], card: usize| {
        let mut near: Vec<End> = Vec::new();
        for (from, to) in ends {
            let other = if *from == End::Card(card) {
                *to
            } else if *to == End::Card(card) {
                *from
            } else {
                continue;
            };
            if !near.contains(&other) {
                near.push(other);
            }
        }
        near
    };
    let mut clusters: Vec<Cluster> = Vec::new();
    // Two rows of random numbers from the same seed: one decides for square cards, one numbers
    // the vessels.
    let mut coin = GoRand::new(seed);
    let mut numbers = GoRand::new(seed);
    for node in 0..count {
        // TALA also passes over a card that is in a cluster already. Its connections end at
        // the vessel by then: it has no neighbours left, so it is like no card.
        if excluded(node) {
            continue;
        }
        let near = neighbours(&ends, node);
        if near.iter().any(|end| matches!(end, End::Vessel(_))) {
            continue;
        }
        let (width, height) = (graph.nodes[node].width, graph.nodes[node].height);
        let mut members = vec![node];
        for other in 0..count {
            if other == node || excluded(other) {
                continue;
            }
            let (other_width, other_height) = (graph.nodes[other].width, graph.nodes[other].height);
            if MAX_SIZE_DIFF * other_width < width
                || MAX_SIZE_DIFF * width < other_width
                || MAX_SIZE_DIFF * other_height < height
                || MAX_SIZE_DIFF * height < other_height
            {
                continue;
            }
            let theirs = neighbours(&ends, other);
            let same_cards = !near.is_empty() && near.len() == theirs.len() && near.iter().all(|end| theirs.contains(end));
            if same_cards && signature(node) == signature(other) {
                members.push(other);
            }
        }
        if members.len() <= 1 {
            continue;
        }
        // TALA `AssignArrangement`: wide cards go under each other, high ones side by side.
        let mean = |side: fn(&Node) -> f64| (members.iter().map(|card| side(&graph.nodes[*card])).sum::<f64>() / members.len() as f64).round();
        let (mean_width, mean_height) = (mean(|card| card.width), mean(|card| card.height));
        let arrangement = if mean_width > mean_height {
            Arrangement::Column
        } else if mean_width < mean_height {
            Arrangement::Row
        } else if coin.float64() > 0.5 {
            Arrangement::Column
        } else {
            Arrangement::Row
        };
        // TALA `maybeCreateCluster`. It refuses a cluster with a connection to a container it is
        // in, and then tries the other arrangement: not for cards in no container.
        // TALA numbers cards from 1; a number that is taken gives way to the next one.
        let mut number = numbers.int63();
        while (1..=cards as i64).contains(&number) || clusters.iter().any(|cluster| cluster.vessel + 1 == number) {
            number = if number == i64::MAX { 0 } else { number + 1 };
        }
        // TALA `PaddingBetween`: a tenth of a card, 20 at least. (TALA rounds the mean size up
        // first, which is a whole number already.)
        let along = if arrangement == Arrangement::Row { mean_width } else { mean_height };
        let padding = NODE_GAP.max(along * 0.1).round();
        // TALA `abductClusterEdges`: the cards' connections now end at the vessel.
        for (from, to) in &mut ends {
            for end in [from, to] {
                if matches!(end, End::Card(card) if members.contains(card)) {
                    *end = End::Vessel(clusters.len());
                }
            }
        }
        clusters.push(Cluster { vessel: number - 1, members, arrangement, padding });
    }
    clusters
}

/// TALA `Cluster.Resize`: the width and height of the largest card, which every card of the
/// cluster gets, and the size of the vessel around them.
pub fn sizes(graph: &Graph, cluster: &Cluster) -> ((f64, f64), (f64, f64)) {
    let vessel = vessel_of(graph, cluster, 0);
    ((vessel.width, vessel.height), vessel.size())
}

fn vessel_of(graph: &Graph, cluster: &Cluster, node: usize) -> Vessel {
    let width = cluster.members.iter().map(|card| graph.nodes[*card].width).fold(0.0, f64::max);
    let height = cluster.members.iter().map(|card| graph.nodes[*card].height).fold(0.0, f64::max);
    Vessel { node, arrangement: cluster.arrangement, desired: cluster.arrangement, padding: cluster.padding, backwards: false, count: cluster.members.len(), width, height }
}

/// TALA `AddCluster`: the graph node placement works on. The cards of a cluster are taken out,
/// its vessel comes at the end, and the connections of its cards end at the vessel. Also, per
/// card of the result, its place in `graph`, or none for a vessel.
pub fn with_vessels(graph: &Graph, clusters: &[Cluster]) -> (Graph, Vec<Option<usize>>) {
    let cluster_of = |card: usize| clusters.iter().position(|cluster| cluster.members.contains(&card));
    let kept: Vec<usize> = (0..graph.nodes.len()).filter(|card| cluster_of(*card).is_none()).collect();
    let place_of = |card: usize| match cluster_of(card) {
        Some(cluster) => kept.len() + cluster,
        None => kept.iter().position(|other| *other == card).expect("a card that is kept"),
    };
    let mut nodes: Vec<Node> = kept.iter().map(|card| graph.nodes[*card].clone()).collect();
    let mut ids: Vec<i64> = kept.iter().map(|card| graph.id(*card)).collect();
    for cluster in clusters {
        let (_, (width, height)) = sizes(graph, cluster);
        nodes.push(Node { top_left: graph.nodes[cluster.members[0]].top_left, width, height });
        ids.push(cluster.vessel);
    }
    let edges = graph.edges.iter().map(|edge| Edge { from: place_of(edge.from), to: place_of(edge.to), ..*edge }).collect();
    let trees_at = graph.trees_at.iter().map(|card| place_of(*card)).collect();
    let from = kept.iter().map(|card| Some(*card)).chain(clusters.iter().map(|_| None)).collect();
    let vessels = clusters.iter().enumerate().map(|(cluster, found)| vessel_of(graph, found, kept.len() + cluster)).collect();
    // TALA `abductClusterEdges`: which card of the cluster a connection at a vessel belongs to.
    // Two cards of clusters share no connection: a card connected to a cluster starts none.
    let behind = graph
        .edges
        .iter()
        .map(|edge| clusters.iter().find_map(|cluster| cluster.members.iter().position(|member| *member == edge.from || *member == edge.to)))
        .collect();
    (Graph { ids, trees_at, vessels, behind, ..Graph::new(nodes, edges) }, from)
}
