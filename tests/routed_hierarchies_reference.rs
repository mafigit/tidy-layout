// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.

//! The routing stages of TALA's pipeline, replayed for diagrams with a hierarchy recorded from
//! TALA's own Go code (docs/reference.md): the cards where the placement stages leave them, in
//! the order TALA's graph has them, the level of each card in its hierarchy, and the routes
//! and label places after the stages.

use tidy_layout::graph::{Edge, Graph, Node, Point};
use tidy_layout::pipeline::layout;
use tidy_layout::place::normalize;
use tidy_layout::routing::coordinator::{route_graph, route_stages};
use tidy_layout::routing::labels::LabelSize;
use serde::Deserialize;

#[derive(Deserialize)]
struct Case {
    case: usize,
    seed: i64,
    cards: Vec<[f64; 4]>,
    /// The number each card has in the diagram. The cards are in the order of TALA's graph.
    ids: Vec<i64>,
    links: Vec<[usize; 4]>,
    routed: Vec<Vec<[f64; 2]>>,
    /// Per card of a hierarchy: the number of its hierarchy and its level in it.
    levels: Vec<Option<(usize, i64)>>,
    /// Where the cards are after `Dejitter`, if it moves one, and after the second `BinPack`.
    dejittered: Option<Vec<[f64; 2]>>,
    packed: Option<Vec<[f64; 2]>>,
    /// The routes after `BalanceEdgeSegments` and after the last stage before `Normalize`.
    balanced: Vec<Vec<[f64; 2]>>,
    end: Vec<Vec<[f64; 2]>>,
    /// The top-left corner of every label's box after `PlaceLabels` and after the last stage
    /// before `Normalize`; 0, 0 without a label.
    labels: Vec<[f64; 2]>,
    #[serde(rename = "labelsEnd")]
    labels_end: Vec<[f64; 2]>,
    /// The drawing after `Normalize`.
    #[serde(rename = "final")]
    last: Final,
    /// The diagram as the recorder was given it: the sizes of the cards, and the connections
    /// (from, to, the width and height of the label).
    input: Input,
}

#[derive(Deserialize)]
struct Final {
    cards: Vec<[f64; 2]>,
    routes: Vec<Vec<[f64; 2]>>,
    labels: Vec<[f64; 2]>,
}

#[derive(Deserialize)]
struct Input {
    nodes: Vec<[f64; 2]>,
    edges: Vec<[usize; 4]>,
    seed: i64,
}

fn graph_of(case: &Case) -> Graph {
    let nodes = case.cards.iter().map(|[x, y, width, height]| Node { top_left: Point { x: *x, y: *y }, width: *width, height: *height }).collect();
    let edges = case.links.iter().map(|[from, to, width, height]| Edge { from: *from, to: *to, min_width: *width as f64, min_height: *height as f64 }).collect();
    Graph { ids: case.ids.clone(), levels: case.levels.clone(), ..Graph::new(nodes, edges) }
}

fn points(routes: &[Vec<Point>]) -> Vec<Vec<[f64; 2]>> {
    routes.iter().map(|route| route.iter().map(|point| [point.x, point.y]).collect()).collect()
}

fn places(labels: &[Option<tidy_layout::routing::labels::Placed>]) -> Vec<[f64; 2]> {
    labels.iter().map(|label| label.as_ref().map_or([0.0, 0.0], |label| [label.top_left.x, label.top_left.y])).collect()
}

/// The numbers: the diagrams, their connections, the cards in a hierarchy, the diagrams with
/// cards outside every hierarchy (other groups), the levels of all hierarchies, and the
/// connections that run level (between two cards of one level).
#[test]
fn the_first_routing_stage_lays_tala_s_routes_for_hierarchies() {
    let cases: Vec<Case> = serde_json::from_str(include_str!("fixtures/routed_hierarchies.tala.json")).unwrap();
    let (mut connections, mut layered, mut mixed, mut level_count, mut level_links, mut differ) = (0, 0, 0, 0, 0, Vec::new());
    for case in &cases {
        connections += case.links.len();
        layered += case.levels.iter().flatten().count();
        mixed += usize::from(case.levels.iter().any(Option::is_none));
        let mut seen: Vec<(usize, i64)> = case.levels.iter().flatten().copied().collect();
        seen.sort_unstable();
        seen.dedup();
        level_count += seen.len();
        level_links += case.links.iter().filter(|[from, to, _, _]| case.levels[*from].is_some() && case.levels[*from] == case.levels[*to]).count();
        if points(&route_graph(&graph_of(case)).unwrap()) != case.routed {
            differ.push(format!("case {} (seed {}, {} cards)", case.case, case.seed, case.cards.len()));
        }
    }
    assert_eq!(differ.len(), 0, "{} differ: {}", differ.len(), differ.join("; "));
    assert_eq!((cases.len(), connections, layered, mixed, level_count, level_links), (50, 566, 487, 12, 196, 0));
}

/// The numbers: the diagrams, those with TALA's routes after `BalanceEdgeSegments`, with
/// TALA's routes at the end, with every label where TALA puts it after `PlaceLabels` and at
/// the end, and with TALA's cards, routes and labels after `Normalize`; the labels in all;
/// and the diagrams in which `Dejitter` moves a card and in which the second `BinPack` does.
#[test]
fn the_later_routing_stages_lay_tala_s_routes_for_hierarchies() {
    let cases: Vec<Case> = serde_json::from_str(include_str!("fixtures/routed_hierarchies.tala.json")).unwrap();
    let (mut balanced, mut end, mut labelled, mut labelled_at_the_end, mut normalized) = (0, 0, 0, 0, 0);
    let (mut label_count, mut dejittered, mut packed) = (0, 0, 0);
    for case in &cases {
        let mut graph = graph_of(case);
        let labels: Vec<Option<LabelSize>> = case.links.iter().map(|[_, _, width, height]| (*width > 0).then_some(LabelSize { width: *width as f64, height: *height as f64 })).collect();
        let staged = route_stages(&mut graph, &labels).unwrap();
        let now: Vec<[f64; 2]> = graph.nodes.iter().map(|node| [node.top_left.x, node.top_left.y]).collect();
        let before: Vec<[f64; 2]> = case.cards.iter().map(|[x, y, _, _]| [*x, *y]).collect();
        assert_eq!(&now, case.packed.as_ref().or(case.dejittered.as_ref()).unwrap_or(&before), "case {}: the cards after Dejitter and the second BinPack", case.case);
        dejittered += usize::from(case.dejittered.is_some());
        packed += usize::from(case.packed.is_some());
        balanced += usize::from(case.balanced == points(&staged.balanced));
        end += usize::from(case.end == points(&staged.routing.routes));
        labelled += usize::from(case.labels == places(&staged.placed));
        labelled_at_the_end += usize::from(case.labels_end == places(&staged.routing.labels));
        label_count += labels.iter().flatten().count();
        let (mut routes, mut placed) = (staged.routing.routes.clone(), staged.routing.labels.clone());
        normalize(&mut graph, &mut routes, &mut placed, &labels);
        let cards: Vec<[f64; 2]> = graph.nodes.iter().map(|node| [node.top_left.x, node.top_left.y]).collect();
        normalized += usize::from(case.last.cards == cards && case.last.routes == points(&routes) && case.last.labels == places(&placed));
    }
    assert_eq!((cases.len(), balanced, end, labelled, labelled_at_the_end, normalized), (50, 50, 50, 50, 50, 50));
    assert_eq!((label_count, dejittered, packed), (316, 1, 1));
}

/// The whole pipeline, from the diagram as it is given to the finished drawing: the cards in
/// TALA's order at TALA's final places, the routes and the label places. Here the port's own
/// placement says which cards are a hierarchy and on which level each is.
#[test]
fn a_diagram_with_a_hierarchy_gets_tala_s_drawing() {
    let cases: Vec<Case> = serde_json::from_str(include_str!("fixtures/routed_hierarchies.tala.json")).unwrap();
    let mut differ = Vec::new();
    for case in &cases {
        let nodes = case.input.nodes.iter().enumerate().map(|(card, [width, height])| Node { top_left: Point { x: 13.0 * card as f64 + 7.0, y: -31.0 }, width: *width, height: *height }).collect();
        let edges = case.input.edges.iter().map(|[from, to, width, height]| Edge { from: *from, to: *to, min_width: *width as f64, min_height: *height as f64 }).collect();
        let same = layout(&Graph::new(nodes, edges), case.input.seed).is_ok_and(|drawing| {
            let ids: Vec<i64> = (0..drawing.graph.nodes.len()).map(|node| drawing.graph.id(node)).collect();
            let cards: Vec<[f64; 2]> = drawing.graph.nodes.iter().map(|node| [node.top_left.x, node.top_left.y]).collect();
            ids == case.ids && cards == case.last.cards && points(&drawing.routes) == case.last.routes && places(&drawing.labels) == case.last.labels
        });
        if !same {
            differ.push(format!("case {} ({} cards)", case.case, case.cards.len()));
        }
    }
    assert_eq!(differ.len(), 0, "{} of {} differ: {}", differ.len(), cases.len(), differ.join("; "));
    assert_eq!(cases.len(), 50);
}
