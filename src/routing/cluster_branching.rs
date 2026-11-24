// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.

//! The stage `FixClusterEdgeBranching`, after `BalanceEdgeSegments`: the routes from one card
//! to the cards of a cluster that branch off to both sides of the cluster's line at different
//! places get one place to branch, where that makes no route dearer and none runs through a
//! card. Ported from TALA `internal/routing`: `postprocess.go`
//! (`fixClusterEdgeBranchingGuarded`) and `cluster_route_guard.go` (`estimateRouteCostGuarded`,
//! `routeIntersectsNodeGuarded`).

use crate::graph::{Arrangement, Graph, Point};
use crate::routing::line::{Routed, estimate_route_cost};

/// TALA `estimateRouteCostGuarded`: the price of a route among all others as they are now.
fn price(graph: &Graph, routes: &[Vec<Point>], edge: usize) -> f64 {
    let others: Vec<Routed> = (0..routes.len()).filter(|other| *other != edge).map(|other| Routed { from: graph.edges[other].from, to: graph.edges[other].to, points: &routes[other] }).collect();
    estimate_route_cost(&routes[edge], &others)
}

/// TALA `routeIntersectsNodeGuarded`: a piece of the route runs through a card that is not one
/// of its two.
fn runs_through_a_card(graph: &Graph, routes: &[Vec<Point>], edge: usize) -> bool {
    let wire = graph.edges[edge];
    graph.nodes.iter().enumerate().any(|(card, node)| card != wire.from && card != wire.to && routes[edge].windows(2).any(|piece| node.passes_through(piece[0], piece[1])))
}

/// Puts the branch of every route of `edges` at `at`: the bend at the card they share (the
/// second point where they leave it, the third where they reach it), and the next bend in
/// line with it.
fn branch_at(routes: &mut [Vec<Point>], edge: usize, at: Point, leaves: bool, row: bool) {
    let route = &mut routes[edge];
    let (moved, next) = if leaves { (1, 2) } else { (2, 1) };
    route[moved] = at;
    if row {
        route[next].y = at.y;
    } else {
        route[next].x = at.x;
    }
}

/// TALA `FixClusterEdgeBranching`. Every card outside clusters with more than one connection,
/// in the order of the graph; per cluster its routes of three pieces to the card's
/// connections, if the cluster has the arrangement the cards around it asked for. TALA takes
/// the clusters of one card in the random order of a Go map; here in the order their first
/// connection comes in the card's list.
pub fn fix_cluster_branching(graph: &Graph, routes: &mut [Vec<Point>]) {
    for card in 0..graph.nodes.len() {
        let own: Vec<usize> = graph.indexed_edges_of(card).map(|(edge, _)| edge).collect();
        if own.len() <= 1 || graph.cluster_of(card).is_some() {
            continue;
        }
        // Per cluster its connections, and whether the last of them leaves the card.
        let mut groups: Vec<(usize, Vec<usize>, bool)> = Vec::new();
        for &edge in &own {
            if routes[edge].len() != 4 {
                continue;
            }
            let Some(cluster) = graph.cluster_of(graph.edges[edge].other(card)) else { continue };
            if graph.clusters[cluster].desired != graph.clusters[cluster].arrangement {
                continue;
            }
            let leaves = graph.edges[edge].from == card;
            match groups.iter_mut().find(|(other, _, _)| *other == cluster) {
                Some((_, edges, last)) => {
                    edges.push(edge);
                    *last = leaves;
                }
                None => groups.push((cluster, vec![edge], leaves)),
            }
        }
        for (cluster, edges, leaves) in groups {
            let row = graph.clusters[cluster].arrangement == Arrangement::Row;
            let along = |point: Point| if row { point.x } else { point.y };
            // Routes that end before the card's middle and routes that end after it.
            let (mut back, mut front) = (false, false);
            for &edge in &edges {
                let (start, end) = (along(routes[edge][0]), along(routes[edge][3]));
                let (here, there) = if leaves { (start, end) } else { (end, start) };
                back |= there < here;
                front |= there > here;
            }
            if !back || !front {
                continue;
            }
            let original: Vec<(Point, Point)> = edges.iter().map(|edge| (routes[*edge][1], routes[*edge][2])).collect();
            let reset = |routes: &mut [Vec<Point>]| {
                for (edge, (first, second)) in edges.iter().zip(&original) {
                    routes[*edge][1] = *first;
                    routes[*edge][2] = *second;
                }
            };
            let candidates: Vec<Point> = original.iter().map(|(first, second)| if leaves { *first } else { *second }).collect();
            let mut best: Option<(Point, f64)> = None;
            'candidates: for at in candidates {
                reset(routes);
                let mut cost = 0.0;
                // TALA keeps a route that already branches at the candidate as it is, but asks
                // that by pointer, of points it has just copied: the answer is always no, so
                // every route is moved and asked.
                for &edge in &edges {
                    let before = price(graph, routes, edge);
                    branch_at(routes, edge, at, leaves, row);
                    if runs_through_a_card(graph, routes, edge) {
                        continue 'candidates;
                    }
                    let after = price(graph, routes, edge);
                    cost += after;
                    if after > before {
                        continue 'candidates;
                    }
                }
                if best.is_none_or(|(_, least)| cost < least) {
                    best = Some((at, cost));
                }
            }
            reset(routes);
            if let Some((at, _)) = best {
                for &edge in &edges {
                    branch_at(routes, edge, at, leaves, row);
                }
            }
        }
    }
}
