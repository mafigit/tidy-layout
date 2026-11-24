// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.

//! What a pass of the optimizer keeps true whatever the layout. Whether it does what TALA does
//! is checked in `pass_reference.rs`.

use tidy_layout::gorand::GoRand;
use tidy_layout::graph::{Edge, Graph, Node, Point};
use tidy_layout::optimizer::optimize;
use tidy_layout::placement::does_overlap;
use tidy_layout::routing::router::Costs;

/// xorshift64.
struct Random(u64);

impl Random {
    fn below(&mut self, limit: usize) -> usize {
        self.0 ^= self.0 << 13;
        self.0 ^= self.0 >> 7;
        self.0 ^= self.0 << 17;
        (self.0 % limit as u64) as usize
    }
}

/// Cards of the editor's size on a grid of 20, none closer to another than the two must be.
fn tidy_layout(random: &mut Random) -> Graph {
    let count = 2 + random.below(9);
    let mut edges = Vec::new();
    for _ in 0..random.below(2 * count) {
        let (from, to) = (random.below(count), random.below(count));
        if from != to {
            let label = random.below(3) == 0;
            edges.push(Edge { from, to, min_width: if label { 140.0 } else { 0.0 }, min_height: if label { 36.0 } else { 0.0 } });
        }
    }
    let card = |x: f64, y: f64| Node { top_left: Point { x, y }, width: 220.0, height: 120.0 };
    let mut graph = Graph { cell_size: 20.0, ..Graph::new(vec![card(0.0, 0.0); count], edges) };
    for node in 0..count {
        // Far apart to start with, then the first place that keeps every gap.
        graph.nodes[node].top_left = Point { x: 100_000.0 * (node + 1) as f64, y: 0.0 };
    }
    for node in 0..count {
        loop {
            let place = Point { x: 20.0 * random.below(70) as f64, y: 20.0 * random.below(50) as f64 };
            if !does_overlap(&graph, node, place) {
                graph.nodes[node].top_left = place;
                break;
            }
        }
    }
    graph
}

#[test]
fn a_cold_pass_keeps_cards_on_the_grid_and_apart_and_moves_only_connected_cards() {
    let mut random = Random(20261122);
    let (mut passes, mut changed, mut moved) = (0, 0, 0);
    for round in 0..80 {
        let mut graph = tidy_layout(&mut random);
        let before: Vec<Point> = graph.nodes.iter().map(|card| card.top_left).collect();
        let turn = Costs::of(&graph).turn / 2.0;
        let mut numbers = GoRand::new(round);
        // TALA runs up to ten cold passes, until one changes nothing.
        for _ in 0..10 {
            let did = optimize(&mut graph, turn, 0.0, &mut numbers).expect("room for every card");
            passes += 1;
            changed += usize::from(did);
            for (node, card) in graph.nodes.iter().enumerate() {
                let what = format!("round {round}, card {node} at {:?}", card.top_left);
                assert!(card.top_left.x % 20.0 == 0.0 && card.top_left.y % 20.0 == 0.0, "off the grid: {what}");
                assert!(!does_overlap(&graph, node, card.top_left), "too close to another card: {what}");
                if graph.edges_of(node).next().is_none() {
                    assert_eq!(card.top_left, before[node], "moved without a connection: {what}");
                }
            }
            if !did {
                break;
            }
        }
        moved += graph.nodes.iter().zip(&before).filter(|(card, place)| card.top_left != **place).count();
    }
    assert_eq!((passes, changed, moved), (278, 201, 294));
}
