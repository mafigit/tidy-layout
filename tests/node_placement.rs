// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.

//! What node placement keeps true whatever the group of cards. Whether it does what TALA does
//! is checked in `place_reference.rs`.

use tidy_layout::gorand::GoRand;
use tidy_layout::graph::{Edge, Graph, Node, Point};
use tidy_layout::node_placement::{PlaceError, place_nodes_orthogonally};
use tidy_layout::place::{place, prescale};

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

fn cards(count: usize) -> Vec<Node> {
    vec![Node { top_left: Point { x: 0.0, y: 0.0 }, width: 220.0, height: 120.0 }; count]
}

fn placed(mut graph: Graph, seed: i64) -> Result<Vec<Point>, PlaceError> {
    place_nodes_orthogonally(&mut graph, seed, &mut GoRand::new(seed), |_, _, _| {})?;
    Ok(graph.nodes.iter().map(|card| card.top_left).collect())
}

#[test]
fn a_single_card_goes_to_the_origin() {
    assert_eq!(placed(Graph::new(cards(1), Vec::new()), 1), Ok(vec![Point { x: 0.0, y: 0.0 }]));
}

#[test]
fn two_connected_cards_end_one_below_the_other_with_the_arrow_pointing_down() {
    for seed in 1..=6 {
        let at = placed(Graph::new(cards(2), vec![Edge::plain(0, 1)]), seed).unwrap();
        assert_eq!((at[1].x - at[0].x, at[1].y - at[0].y), (0.0, 220.0), "seed {seed}: {at:?}");
    }
}

#[test]
fn cards_that_no_connection_leads_to_are_refused() {
    assert_eq!(placed(Graph::new(cards(3), vec![Edge::plain(0, 1)]), 1), Err(PlaceError::NotArranged));
}

/// Whatever the group and the seed: every card ends on the grid, no two cards closer than 20,
/// and the same seed gives the same places again.
#[test]
fn every_group_ends_on_the_grid_with_all_cards_apart() {
    let mut random = Random(20261122);
    let (mut groups, mut total) = (0, 0);
    for round in 0..40 {
        let count = 2 + random.below(9);
        let mut edges: Vec<Edge> = (1..count).map(|card| Edge::plain(random.below(card), card)).collect();
        for _ in 0..random.below(3) {
            let (from, to) = (random.below(count), random.below(count));
            if from != to {
                let label = random.below(2) == 0;
                edges.push(Edge { from, to, min_width: if label { 150.0 } else { 0.0 }, min_height: if label { 36.0 } else { 0.0 } });
            }
        }
        let graph = Graph::new(cards(count), edges);
        let cell = graph.cell_size;
        let seed = 1 + round % 3;
        let at = placed(graph.clone(), seed).unwrap();
        assert_eq!(placed(graph, seed).unwrap(), at, "round {round}: not the same places twice");
        for (first, a) in at.iter().enumerate() {
            assert!(a.x % cell == 0.0 && a.y % cell == 0.0, "round {round}: card {first} off the grid at {a:?}");
            for b in &at[first + 1..] {
                let apart = (a.x - b.x).abs() >= 220.0 + 20.0 || (a.y - b.y).abs() >= 120.0 + 20.0;
                assert!(apart, "round {round}: cards at {a:?} and {b:?} too close");
            }
        }
        groups += 1;
        total += count;
    }
    assert_eq!((groups, total), (40, 243));
}

/// The whole stage, for cards in several groups: whatever the diagram, no two cards end closer
/// than 20, the top-left corner of the diagram is the origin, and the same seed gives the same
/// places again.
#[test]
fn every_diagram_is_packed_with_all_cards_apart() {
    let mut random = Random(20261123);
    let (mut diagrams, mut total) = (0, 0);
    for round in 0..40 {
        let count = 1 + random.below(14);
        let mut edges = Vec::new();
        for _ in 0..random.below(count + 2) {
            let (from, to) = (random.below(count), random.below(count));
            if from != to {
                let label = random.below(2) == 0;
                edges.push(Edge { from, to, min_width: if label { 150.0 } else { 0.0 }, min_height: if label { 36.0 } else { 0.0 } });
            }
        }
        let mut graph = Graph::new(cards(count), edges);
        prescale(&mut graph);
        let seed = 1 + round % 3;
        let mut again = graph.clone();
        place(&mut graph, seed).unwrap();
        place(&mut again, seed).unwrap();
        assert_eq!(graph.nodes, again.nodes, "round {round}: not the same places twice");
        let left = graph.nodes.iter().fold(f64::INFINITY, |least, card| least.min(card.top_left.x));
        let top = graph.nodes.iter().fold(f64::INFINITY, |least, card| least.min(card.top_left.y));
        assert_eq!((left, top), (0.0, 0.0), "round {round}: the diagram does not start at the origin");
        for (first, a) in graph.nodes.iter().enumerate() {
            for b in &graph.nodes[first + 1..] {
                let apart = a.top_left.x >= b.right() + 20.0
                    || b.top_left.x >= a.right() + 20.0
                    || a.top_left.y >= b.bottom() + 20.0
                    || b.top_left.y >= a.bottom() + 20.0;
                assert!(apart, "round {round}: cards at {:?} and {:?} too close", a.top_left, b.top_left);
            }
        }
        diagrams += 1;
        total += count;
    }
    assert_eq!((diagrams, total), (40, 287));
}
