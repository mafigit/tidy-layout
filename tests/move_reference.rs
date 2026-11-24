// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.

//! A card must go where TALA moves it: these tests replay moves recorded from TALA's own Go
//! code (docs/reference.md). Prices are compared bit for bit.

use tidy_layout::graph::{Edge, Graph, Node, Point};
use tidy_layout::placement::{closest_unoccupied, fill_placement_points, move_node_to_best};
use tidy_layout::placementcost::{edge_length::node_edge_length, symmetry::node_symmetry};
use tidy_layout::routing::router::Costs;
use serde::Deserialize;

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct Case {
    name: String,
    nodes: Vec<[f64; 4]>,
    edges: Vec<[usize; 4]>,
    moving: usize,
    point: [f64; 2],
    cell: f64,
    turn_cost: f64,
    distance: f64,
    /// x, y, can the card go there, and if so its edge length and its symmetry there.
    points: Vec<[f64; 5]>,
    best: [f64; 2],
    best_improving: [f64; 2],
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

fn cases() -> Vec<Case> {
    serde_json::from_str(include_str!("fixtures/move.tala.json")).unwrap()
}

#[test]
fn a_card_looks_at_the_places_tala_looks_at() {
    let cases = cases();
    assert_eq!(cases.len(), 341);
    let mut places = 0;
    for (number, case) in cases.iter().enumerate() {
        let graph = graph_of(case);
        let median = Point { x: case.point[0], y: case.point[1] };
        let nearest = closest_unoccupied(&graph, case.moving, median).expect("TALA found a free place");
        assert_eq!(f64::from(nearest.ring), case.distance, "case {number} ({})", case.name);
        let points = fill_placement_points(&graph, case.moving, median, nearest.ring, false);
        let recorded: Vec<Point> = case.points.iter().map(|p| Point { x: p[0], y: p[1] }).collect();
        assert_eq!(points, recorded, "case {number} ({})", case.name);
        places += points.len();
    }
    assert_eq!(places, 14567);
}

#[test]
fn a_card_costs_at_each_place_what_tala_says() {
    let (mut priced, mut symmetric, mut mirrored) = (0, 0, 0);
    for (number, case) in cases().iter().enumerate() {
        let mut graph = graph_of(case);
        let turn = Costs::of(&graph).turn;
        assert_eq!(turn.to_bits(), case.turn_cost.to_bits(), "case {number} ({}): the price of a bend", case.name);
        for [x, y, can, length, symmetry] in &case.points {
            if *can == 0.0 {
                continue;
            }
            graph.nodes[case.moving].top_left = Point { x: *x, y: *y };
            let at = format!("case {number} ({}) at {x}, {y}", case.name);
            assert_eq!(node_edge_length(&graph, case.moving, turn).to_bits(), length.to_bits(), "{at}: edge length");
            assert_eq!(node_symmetry(&graph, case.moving).to_bits(), symmetry.to_bits(), "{at}: symmetry");
            priced += 1;
            symmetric += usize::from(*symmetry > 0.0);
            mirrored += usize::from(*symmetry == 1.0);
        }
    }
    // The fixture must keep meeting symmetry, also a card with a mirror image for every neighbour.
    assert_eq!((priced, symmetric, mirrored), (1994, 236, 40));
}

#[test]
fn a_card_takes_the_place_tala_takes() {
    let (mut moved, mut held_back) = (0, 0);
    for (number, case) in cases().iter().enumerate() {
        let graph = graph_of(case);
        let home = graph.nodes[case.moving].top_left;
        let points: Vec<Point> = case.points.iter().map(|p| Point { x: p[0], y: p[1] }).collect();
        for (must_improve, recorded) in [(false, case.best), (true, case.best_improving)] {
            let mut graph = graph.clone();
            let turn = Costs::of(&graph).turn;
            let place = move_node_to_best(&mut graph, case.moving, &points, must_improve, turn);
            let recorded = Point { x: recorded[0], y: recorded[1] };
            let at = format!("case {number} ({}), must improve: {must_improve}", case.name);
            assert_eq!(graph.nodes[case.moving].top_left, recorded, "{at}");
            // TALA reports an error when the card can go nowhere; the recorder has no such case.
            assert_eq!(place, Some(recorded), "{at}");
        }
        moved += usize::from(case.best != [home.x, home.y]);
        held_back += usize::from(case.best != case.best_improving);
    }
    assert_eq!((moved, held_back), (323, 79));
}
