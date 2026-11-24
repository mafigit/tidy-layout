// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.

//! Node placement must put the cards where TALA's puts them, stage by stage: these tests replay
//! placements recorded from TALA's own Go code (docs/reference.md), with Go's random numbers.

use tidy_layout::gorand::GoRand;
use tidy_layout::graph::{Edge, Graph, Node, Point};
use tidy_layout::node_placement::{Stage, place_nodes_orthogonally};
use serde::Deserialize;

#[derive(Deserialize, Debug, PartialEq, Clone)]
struct Snapshot {
    at: Vec<[f64; 2]>,
    drawn: u64,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct Case {
    name: String,
    nodes: Vec<[f64; 2]>,
    edges: Vec<[usize; 4]>,
    seed: i64,
    cell: f64,
    by_distance: bool,
    first: Snapshot,
    sizeless: Snapshot,
    inflated: Snapshot,
    on_grid: Snapshot,
    turn_cost: f64,
    sized: Snapshot,
    #[serde(rename = "final")]
    last: Snapshot,
}

fn graph_of(case: &Case) -> Graph {
    let nodes = case
        .nodes
        .iter()
        .map(|[width, height]| Node { top_left: Point { x: 0.0, y: 0.0 }, width: *width, height: *height })
        .collect();
    let edges = case
        .edges
        .iter()
        .map(|[from, to, width, height]| Edge { from: *from, to: *to, min_width: *width as f64, min_height: *height as f64 })
        .collect();
    Graph::new(nodes, edges)
}

fn cases() -> Vec<Case> {
    serde_json::from_str(include_str!("fixtures/place.tala.json")).unwrap()
}

#[test]
fn node_placement_goes_through_the_stages_tala_goes_through() {
    let cases = cases();
    assert_eq!(cases.len(), 139);
    let (mut by_distance, mut cards, mut drawn) = (0, 0, 0);
    for (number, case) in cases.iter().enumerate() {
        let what = format!("case {number} ({}, {} cards, seed {})", case.name, case.nodes.len(), case.seed);
        let mut graph = graph_of(case);
        assert_eq!(graph.cell_size, case.cell, "{what}: cell size");
        let mut random = GoRand::new(case.seed);
        // The first stage that differs says most: later ones follow from it.
        let mut differs: Option<String> = None;
        let result = place_nodes_orthogonally(&mut graph, case.seed, &mut random, |stage, graph, random| {
            let recorded = match stage {
                Stage::First(distance) => {
                    assert_eq!(distance, case.by_distance, "{what}: first arrangement by graph distance");
                    &case.first
                }
                Stage::Sizeless => &case.sizeless,
                Stage::Inflated => &case.inflated,
                Stage::OnGrid(turn) => {
                    assert_eq!(turn, case.turn_cost, "{what}: price of a bend");
                    &case.on_grid
                }
                Stage::Sized => &case.sized,
                Stage::Final => &case.last,
                Stage::SizelessStep | Stage::SizedStep | Stage::ColdStep => return,
            };
            let now = Snapshot { at: graph.nodes.iter().map(|card| [card.top_left.x, card.top_left.y]).collect(), drawn: random.drawn };
            if differs.is_none() && now != *recorded {
                differs = Some(format!("{what}: at stage {stage:?}\n port {now:?}\n TALA {recorded:?}"));
            }
        });
        assert_eq!(result, Ok(()), "{what}");
        assert_eq!(differs, None);
        by_distance += usize::from(case.by_distance);
        cards += case.nodes.len();
        drawn += random.drawn;
    }
    assert_eq!((by_distance, cards, drawn), (50, 905, 5051738));
}
