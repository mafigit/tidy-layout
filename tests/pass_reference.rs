// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.

//! One pass of the optimizer must leave the cards where TALA's leaves them: these tests replay
//! passes recorded from TALA's own Go code (docs/reference.md), with Go's random numbers.

use tidy_layout::gorand::GoRand;
use tidy_layout::graph::{Edge, Graph, Node, Point};
use tidy_layout::optimizer::{best_swap_candidate, hubs, median_point, optimize, transpose};
use tidy_layout::routing::router::Costs;
use serde::Deserialize;

#[derive(Deserialize)]
struct Median {
    point: [f64; 2],
    drawn: u64,
}

#[derive(Deserialize)]
struct Swap {
    with: i64,
    drawn: u64,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct Case {
    name: String,
    nodes: Vec<[f64; 4]>,
    edges: Vec<[usize; 4]>,
    temp: f64,
    seed: i64,
    halve: bool,
    cell: f64,
    turn_cost: f64,
    hubs: Vec<Option<Vec<usize>>>,
    medians: Vec<Option<Median>>,
    swaps: Vec<Swap>,
    transposes: Vec<Option<Vec<[f64; 2]>>>,
    changed: bool,
    after: Vec<[f64; 2]>,
    drawn: u64,
}

fn graph_of(case: &Case) -> Graph {
    let nodes = case
        .nodes
        .iter()
        .map(|[x, y, width, height]| Node { top_left: Point { x: *x, y: *y }, width: *width, height: *height })
        .collect();
    let edges = case
        .edges
        .iter()
        .map(|[from, to, width, height]| Edge { from: *from, to: *to, min_width: *width as f64, min_height: *height as f64 })
        .collect();
    Graph { cell_size: case.cell, ..Graph::new(nodes, edges) }
}

fn places(graph: &Graph) -> Vec<[f64; 2]> {
    graph.nodes.iter().map(|card| [card.top_left.x, card.top_left.y]).collect()
}

fn cases() -> Vec<Case> {
    serde_json::from_str(include_str!("fixtures/pass.tala.json")).unwrap()
}

/// The recorder asks in this order, with one generator: every card's median point, every card's
/// swap, every card's transpose, then the pass.
#[test]
fn a_pass_leaves_the_cards_where_tala_leaves_them() {
    let cases = cases();
    assert_eq!(cases.len(), 322);
    let (mut hub_count, mut medians, mut swaps, mut turns, mut changed, mut hot) = (0, 0, 0, 0, 0, 0);
    for (number, case) in cases.iter().enumerate() {
        let what = format!("case {number} ({}), temp {}", case.name, case.temp);
        let mut graph = graph_of(case);
        let before = places(&graph);
        // The price of a bend is TALA's, halved where node placement halves it.
        let turn = Costs::of(&graph).turn / if case.halve { 2.0 } else { 1.0 };
        assert_eq!(turn, case.turn_cost, "{what}: turn cost");
        assert_eq!(hubs(&graph), case.hubs, "{what}: hubs");
        hub_count += case.hubs.iter().flatten().count();

        let mut random = GoRand::new(case.seed);
        for (node, recorded) in case.medians.iter().enumerate() {
            let Some(recorded) = recorded else {
                assert_eq!(graph.edges_of(node).count(), 0, "{what}: card {node} has a connection");
                continue;
            };
            let drawn = random.drawn;
            let median = median_point(&graph, node, case.temp, &mut random);
            assert_eq!([median.x, median.y], recorded.point, "{what}: median of card {node}");
            assert_eq!(random.drawn - drawn, recorded.drawn, "{what}: numbers drawn for the median of card {node}");
            medians += 1;
        }
        for (node, recorded) in case.swaps.iter().enumerate() {
            let drawn = random.drawn;
            let with = best_swap_candidate(&mut graph, node, turn, &mut random);
            assert_eq!(with.map_or(-1, |card| card as i64), recorded.with, "{what}: swap of card {node}");
            assert_eq!(random.drawn - drawn, recorded.drawn, "{what}: numbers drawn for the swap of card {node}");
            assert_eq!(places(&graph), before, "{what}: looking for a swap moved a card");
            swaps += usize::from(with.is_some());
        }
        for (node, recorded) in case.transposes.iter().enumerate() {
            let mut turned = graph.clone();
            let did = transpose(&mut turned, node, turn);
            assert_eq!(did, recorded.is_some(), "{what}: transpose of card {node}");
            assert_eq!(&places(&turned), recorded.as_ref().unwrap_or(&before), "{what}: after the transpose of card {node}");
            turns += usize::from(did);
        }

        let drawn = random.drawn;
        let did = optimize(&mut graph, turn, case.temp, &mut random).expect("TALA found a place for every card");
        assert_eq!(places(&graph), case.after, "{what}: after the pass");
        assert_eq!(did, case.changed, "{what}: changed");
        assert_eq!(random.drawn - drawn, case.drawn, "{what}: numbers drawn in the pass");
        changed += usize::from(did);
        hot += usize::from(case.temp != 0.0);
    }
    assert_eq!((hub_count, medians, swaps, turns, changed, hot), (437, 2071, 210, 330, 318, 179));
}
