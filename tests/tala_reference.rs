// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.

//! The port must answer like TALA: these tests replay cases recorded from TALA's own Go code
//! (docs/reference.md).

use tidy_layout::graph::{Edge, Graph, Node, Point};
use tidy_layout::placement::closest_unoccupied;
use serde::Deserialize;

#[derive(Deserialize)]
struct RefNode {
    id: usize,
    x: f64,
    y: f64,
    width: f64,
    height: f64,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct RefCase {
    cell_size: f64,
    nodes: Vec<RefNode>,
    edges: Option<Vec<[usize; 2]>>,
    moving: usize,
    point: [f64; 2],
    distance: f64,
    found: Option<[f64; 2]>,
}

#[test]
fn closest_unoccupied_answers_like_tala() {
    let cases: Vec<RefCase> =
        serde_json::from_str(include_str!("fixtures/closest-unoccupied.tala.json")).unwrap();
    assert_eq!(cases.len(), 500);
    for (number, case) in cases.iter().enumerate() {
        let index = |id: usize| case.nodes.iter().position(|n| n.id == id).unwrap();
        let nodes = case
            .nodes
            .iter()
            .map(|n| Node { top_left: Point { x: n.x, y: n.y }, width: n.width, height: n.height })
            .collect();
        let edges = case
            .edges
            .iter()
            .flatten()
            .map(|[a, b]| Edge::plain(index(*a), index(*b)))
            .collect();
        let graph = Graph::new(nodes, edges);
        assert_eq!(graph.cell_size, case.cell_size, "case {number}: cell size");

        let wanted = Point { x: case.point[0], y: case.point[1] };
        let found = closest_unoccupied(&graph, index(case.moving), wanted);
        match (found, case.found) {
            (Some(place), Some([x, y])) => {
                assert_eq!(f64::from(place.ring), case.distance, "case {number}: ring");
                assert_eq!(place.top_left, Point { x, y }, "case {number}: place");
            }
            (None, None) => assert_eq!(case.distance, -1.0, "case {number}"),
            (ours, theirs) => panic!("case {number}: we say {ours:?}, TALA says {theirs:?}"),
        }
    }
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct LabelEdge {
    from: usize,
    to: usize,
    min_width: f64,
    min_height: f64,
}

#[derive(Deserialize)]
struct RefGap {
    other: usize,
    at: [f64; 2],
    gap: f64,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct LabelCase {
    cell_size: f64,
    nodes: Vec<RefNode>,
    edges: Option<Vec<LabelEdge>>,
    moving: usize,
    gaps: Option<Vec<RefGap>>,
    point: [f64; 2],
    distance: f64,
    found: Option<[f64; 2]>,
}

/// Connections with labels: the gap between two cards, and the search that keeps it.
#[test]
fn label_gaps_answer_like_tala() {
    let cases: Vec<LabelCase> =
        serde_json::from_str(include_str!("fixtures/label-gaps.tala.json")).unwrap();
    assert_eq!(cases.len(), 500);
    let mut gaps_checked = 0;
    for (number, case) in cases.iter().enumerate() {
        let index = |id: usize| case.nodes.iter().position(|n| n.id == id).unwrap();
        let nodes = case
            .nodes
            .iter()
            .map(|n| Node { top_left: Point { x: n.x, y: n.y }, width: n.width, height: n.height })
            .collect();
        let edges = case
            .edges
            .iter()
            .flatten()
            .map(|e| Edge {
                from: index(e.from),
                to: index(e.to),
                min_width: e.min_width,
                min_height: e.min_height,
            })
            .collect();
        let graph = Graph { cell_size: case.cell_size, ..Graph::new(nodes, edges) };
        let moving = index(case.moving);

        for gap in case.gaps.iter().flatten() {
            let at = Point { x: gap.at[0], y: gap.at[1] };
            let ours = graph.gap_between(moving, index(gap.other), at);
            assert_eq!(ours, gap.gap, "case {number}: gap to card {} at {at:?}", gap.other);
            gaps_checked += 1;
        }

        let wanted = Point { x: case.point[0], y: case.point[1] };
        match (closest_unoccupied(&graph, moving, wanted), case.found) {
            (Some(place), Some([x, y])) => {
                assert_eq!(f64::from(place.ring), case.distance, "case {number}: ring");
                assert_eq!(place.top_left, Point { x, y }, "case {number}: place");
            }
            (None, None) => assert_eq!(case.distance, -1.0, "case {number}"),
            (ours, theirs) => panic!("case {number}: we say {ours:?}, TALA says {theirs:?}"),
        }
    }
    assert_eq!(gaps_checked, 5066);
}

