// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.

//! The routing stages of TALA's pipeline, replayed for diagrams with a cluster recorded from
//! TALA's own Go code (docs/reference.md): the cards where the placement stages leave them, in
//! the order TALA's graph has them, every card's own list of connections, the clusters, and
//! the routes and label places after the stages.

use tidy_layout::graph::{Arrangement, Edge, Graph, Node, Point, RoutedCluster};
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
    /// Every card's own list of connections.
    own: Vec<Vec<usize>>,
    /// Per cluster its cards in its order, its arrangement and the one the cards around it
    /// asked for.
    clusters: Vec<(Vec<usize>, String, String)>,
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
    let arrangement = |name: &str| if name == "Row" { Arrangement::Row } else { Arrangement::Column };
    let clusters = case.clusters.iter().map(|(members, now, desired)| RoutedCluster { members: members.clone(), arrangement: arrangement(now), desired: arrangement(desired) }).collect();
    Graph { ids: case.ids.clone(), own_edges: case.own.clone(), clusters, ..Graph::new(nodes, edges) }
}

fn points(routes: &[Vec<Point>]) -> Vec<Vec<[f64; 2]>> {
    routes.iter().map(|route| route.iter().map(|point| [point.x, point.y]).collect()).collect()
}

fn places(labels: &[Option<tidy_layout::routing::labels::Placed>]) -> Vec<[f64; 2]> {
    labels.iter().map(|label| label.as_ref().map_or([0.0, 0.0], |label| [label.top_left.x, label.top_left.y])).collect()
}

/// The numbers: the diagrams, their connections, the clusters, the cards in them, the
/// connections of a card of a cluster, and the clusters whose arrangement is not the one the
/// cards around them asked for.
#[test]
fn the_first_routing_stage_lays_tala_s_routes_for_clusters() {
    let cases: Vec<Case> = serde_json::from_str(include_str!("fixtures/routed_clusters.tala.json")).unwrap();
    let (mut connections, mut clusters, mut members, mut cluster_links, mut turned, mut differ) = (0, 0, 0, 0, 0, Vec::new());
    for case in &cases {
        connections += case.links.len();
        clusters += case.clusters.len();
        members += case.clusters.iter().map(|(cards, _, _)| cards.len()).sum::<usize>();
        turned += case.clusters.iter().filter(|(_, now, desired)| now != desired).count();
        let in_cluster = |card: usize| case.clusters.iter().any(|(cards, _, _)| cards.contains(&card));
        cluster_links += case.links.iter().filter(|[from, to, _, _]| in_cluster(*from) || in_cluster(*to)).count();
        if points(&route_graph(&graph_of(case)).unwrap()) != case.routed {
            differ.push(format!("case {} (seed {}, {} cards)", case.case, case.seed, case.cards.len()));
        }
    }
    assert_eq!(differ.len(), 0, "{} differ: {}", differ.len(), differ.join("; "));
    assert_eq!((cases.len(), connections, clusters, members, cluster_links, turned), (113, 1000, 140, 371, 605, 4));
}

/// The numbers: the diagrams, those with TALA's routes after `BalanceEdgeSegments`, with
/// TALA's routes at the end, with every label where TALA puts it after `PlaceLabels` and at
/// the end, and with TALA's cards, routes and labels after `Normalize`; the labels in all;
/// and the diagrams in which `Dejitter` moves a card and in which the second `BinPack` does.
#[test]
fn the_later_routing_stages_lay_tala_s_routes_for_clusters() {
    let cases: Vec<Case> = serde_json::from_str(include_str!("fixtures/routed_clusters.tala.json")).unwrap();
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
    assert_eq!((cases.len(), balanced, end, labelled, labelled_at_the_end, normalized), (113, 113, 113, 113, 113, 113));
    assert_eq!((label_count, dejittered, packed), (414, 1, 21));
}

/// The whole pipeline, from the diagram as it is given to the finished drawing: the cards in
/// TALA's order at TALA's final places, every card's own list of connections, the clusters,
/// the routes and the label places. Here the port's own placement finds the clusters and puts
/// their cards back into the graph.
#[test]
fn a_diagram_with_a_cluster_gets_tala_s_drawing() {
    let cases: Vec<Case> = serde_json::from_str(include_str!("fixtures/routed_clusters.tala.json")).unwrap();
    let mut differ = Vec::new();
    for case in &cases {
        let nodes = case.input.nodes.iter().enumerate().map(|(card, [width, height])| Node { top_left: Point { x: 13.0 * card as f64 + 7.0, y: -31.0 }, width: *width, height: *height }).collect();
        let edges = case.input.edges.iter().map(|[from, to, width, height]| Edge { from: *from, to: *to, min_width: *width as f64, min_height: *height as f64 }).collect();
        let theirs = graph_of(case);
        let same = layout(&Graph::new(nodes, edges), case.input.seed).is_ok_and(|drawing| {
            let graph = &drawing.graph;
            let ids: Vec<i64> = (0..graph.nodes.len()).map(|node| graph.id(node)).collect();
            let cards: Vec<[f64; 2]> = graph.nodes.iter().map(|node| [node.top_left.x, node.top_left.y]).collect();
            let own = (0..graph.nodes.len()).all(|node| graph.indexed_edges_of(node).map(|(edge, _)| edge).eq(theirs.indexed_edges_of(node).map(|(edge, _)| edge)));
            ids == case.ids && own && graph.clusters == theirs.clusters && cards == case.last.cards && points(&drawing.routes) == case.last.routes && places(&drawing.labels) == case.last.labels
        });
        if !same {
            differ.push(format!("case {} ({} cards)", case.case, case.cards.len()));
        }
    }
    assert_eq!(differ.len(), 0, "{} of {} differ: {}", differ.len(), cases.len(), differ.join("; "));
    assert_eq!(cases.len(), 113);
}
