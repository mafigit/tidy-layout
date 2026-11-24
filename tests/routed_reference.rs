// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.

//! The routing stages of TALA's pipeline, replayed for diagrams of plain cards recorded from
//! TALA's own Go code (docs/reference.md): the cards where the placement stages leave them, in
//! the order TALA's graph has them, and the routes after the stage.

use tidy_layout::graph::{Edge, Graph, Node, Point};
use tidy_layout::pipeline::layout;
use tidy_layout::place::normalize;
use tidy_layout::quality::{LabelBox, inspect};
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
    /// Where the cards are after `Dejitter`, if it moves one.
    dejittered: Option<Vec<[f64; 2]>>,
    /// Where the cards are after the second `BinPack`, if it moves one.
    packed: Option<Vec<[f64; 2]>>,
    balanced: Option<Vec<Vec<[f64; 2]>>>,
    end: Option<Vec<Vec<[f64; 2]>>>,
    /// The top-left corner of every label's box after `PlaceLabels`; 0, 0 without a label.
    labels: Option<Vec<[f64; 2]>>,
    /// The same after the last stage before `Normalize`.
    #[serde(rename = "labelsEnd")]
    labels_end: Option<Vec<[f64; 2]>>,
    /// What TALA's `quality.Inspect` counts there: cards on each other, routes through cards,
    /// covered labels, crossings, detour, route length.
    metrics: Option<Vec<f64>>,
    /// The drawing after `Normalize`.
    #[serde(rename = "final")]
    last: Option<Final>,
    /// The diagram as the recorder was given it: the sizes of the cards, and the connections
    /// (from, to, the width and height of the label).
    input: Input,
}

#[derive(Deserialize)]
struct Input {
    nodes: Vec<[f64; 2]>,
    edges: Vec<[usize; 4]>,
    seed: i64,
}

#[derive(Deserialize)]
struct Final {
    cards: Vec<[f64; 2]>,
    routes: Vec<Vec<[f64; 2]>>,
    labels: Vec<[f64; 2]>,
}

fn graph_of(case: &Case) -> Graph {
    let nodes = case.cards.iter().map(|[x, y, width, height]| Node { top_left: Point { x: *x, y: *y }, width: *width, height: *height }).collect();
    let edges = case.links.iter().map(|[from, to, width, height]| Edge { from: *from, to: *to, min_width: *width as f64, min_height: *height as f64 }).collect();
    Graph { ids: case.ids.clone(), ..Graph::new(nodes, edges) }
}

/// The numbers: the diagrams, those with TALA's routes after `BalanceEdgeSegments`, with
/// TALA's routes at the end, with every label where TALA puts it after `PlaceLabels`, the
/// labels in all, the diagrams with every label where TALA has it at the end, and those with
/// TALA's cards, routes and labels after `Normalize`, those in which `Dejitter` moves a card
/// and those in which the second `BinPack` does.
#[test]
fn the_later_routing_stages_lay_tala_s_routes_where_they_are_ported() {
    let cases: Vec<Case> = serde_json::from_str(include_str!("fixtures/routed.tala.json")).unwrap();
    let (mut balanced, mut end, mut labelled, mut label_count, mut labelled_at_the_end, mut normalized, mut dejittered, mut packed) = (0, 0, 0, 0, 0, 0, 0, 0);
    for case in &cases {
        let mut graph = graph_of(case);
        let labels: Vec<Option<LabelSize>> = case.links.iter().map(|[_, _, width, height]| (*width > 0).then_some(LabelSize { width: *width as f64, height: *height as f64 })).collect();
        let staged = route_stages(&mut graph, &labels).unwrap();
        // `Dejitter` and the second `BinPack` move the cards TALA moves, and none where TALA
        // moves none.
        let now: Vec<[f64; 2]> = graph.nodes.iter().map(|node| [node.top_left.x, node.top_left.y]).collect();
        let before: Vec<[f64; 2]> = case.cards.iter().map(|[x, y, _, _]| [*x, *y]).collect();
        assert_eq!(&now, case.packed.as_ref().or(case.dejittered.as_ref()).unwrap_or(&before), "case {}: the cards after Dejitter and the second BinPack", case.case);
        dejittered += usize::from(case.dejittered.is_some());
        packed += usize::from(case.packed.is_some());
        let (routing, after_balance) = (&staged.routing, &staged.balanced);
        let points = |routes: &[Vec<Point>]| -> Vec<Vec<[f64; 2]>> { routes.iter().map(|route| route.iter().map(|point| [point.x, point.y]).collect()).collect() };
        balanced += usize::from(case.balanced.as_ref() == Some(&points(after_balance)));
        end += usize::from(case.end.as_ref() == Some(&points(&routing.routes)));
        let places = |labels: &[Option<tidy_layout::routing::labels::Placed>]| -> Vec<[f64; 2]> { labels.iter().map(|label| label.as_ref().map_or([0.0, 0.0], |label| [label.top_left.x, label.top_left.y])).collect() };
        labelled += usize::from(case.labels.as_ref() == Some(&places(&staged.placed)));
        label_count += labels.iter().flatten().count();
        labelled_at_the_end += usize::from(case.labels_end.as_ref() == Some(&places(&routing.labels)));
        // And the last stage: the whole drawing moved to the origin.
        let (mut routes, mut placed) = (routing.routes.clone(), routing.labels.clone());
        normalize(&mut graph, &mut routes, &mut placed, &labels);
        let cards: Vec<[f64; 2]> = graph.nodes.iter().map(|node| [node.top_left.x, node.top_left.y]).collect();
        normalized += usize::from(case.last.as_ref().is_some_and(|last| last.cards == cards && last.routes == points(&routes) && last.labels == places(&placed)));
    }
    assert_eq!((cases.len(), balanced, end, labelled, label_count, labelled_at_the_end, normalized, dejittered, packed), (174, 174, 174, 174, 879, 174, 174, 2, 2));
}

#[test]
fn the_first_routing_stage_lays_the_routes_tala_lays() {
    let cases: Vec<Case> = serde_json::from_str(include_str!("fixtures/routed.tala.json")).unwrap();
    let (mut connections, mut groups, mut doubled, mut differ) = (0, 0, 0, Vec::new());
    for case in &cases {
        let nodes = case.cards.iter().map(|[x, y, width, height]| Node { top_left: Point { x: *x, y: *y }, width: *width, height: *height }).collect();
        let edges: Vec<Edge> = case.links.iter().map(|[from, to, width, height]| Edge { from: *from, to: *to, min_width: *width as f64, min_height: *height as f64 }).collect();
        connections += edges.len();
        // Diagrams with more than one group of connected cards, and with two connections between
        // the same two cards.
        let mut group: Vec<usize> = (0..case.cards.len()).collect();
        for _ in 0..case.cards.len() {
            for edge in &edges {
                let least = group[edge.from].min(group[edge.to]);
                (group[edge.from], group[edge.to]) = (least, least);
            }
        }
        let mut with_links: Vec<usize> = edges.iter().map(|edge| group[edge.from]).collect();
        with_links.sort_unstable();
        with_links.dedup();
        groups += usize::from(with_links.len() > 1);
        doubled += usize::from(edges.iter().enumerate().any(|(index, edge)| edges[..index].iter().any(|other| (other.from, other.to) == (edge.from, edge.to) || (other.from, other.to) == (edge.to, edge.from))));
        let graph = Graph { ids: case.ids.clone(), ..Graph::new(nodes, edges) };
        let ours: Vec<Vec<[f64; 2]>> = route_graph(&graph).unwrap().iter().map(|route| route.iter().map(|point| [point.x, point.y]).collect()).collect();
        if ours != case.routed {
            differ.push(format!("case {} (seed {}, {} cards)", case.case, case.seed, case.cards.len()));
        }
    }
    assert_eq!(differ.len(), 0, "{} differ: {}", differ.len(), differ.join("; "));
    assert_eq!((cases.len(), connections, groups, doubled), (174, 1303, 22, 67));
}

/// The counts of a finished drawing, on TALA's own routes and label places at the end.
#[test]
fn a_drawing_is_counted_as_tala_counts_it() {
    let cases: Vec<Case> = serde_json::from_str(include_str!("fixtures/routed.tala.json")).unwrap();
    let (mut counted, mut through_a_card, mut covered, mut crossing, mut differ) = (0, 0, 0, 0, Vec::new());
    for case in &cases {
        let (Some(end), Some(labels), Some(theirs)) = (&case.end, &case.labels_end, &case.metrics) else {
            continue;
        };
        let mut graph = graph_of(case);
        for (node, at) in graph.nodes.iter_mut().zip(case.packed.as_ref().or(case.dejittered.as_ref()).into_iter().flatten()) {
            node.top_left = Point { x: at[0], y: at[1] };
        }
        let routes: Vec<Vec<Point>> = end.iter().map(|route| route.iter().map(|[x, y]| Point { x: *x, y: *y }).collect()).collect();
        let boxes: Vec<Option<LabelBox>> =
            case.links.iter().zip(labels).map(|([_, _, width, height], [x, y])| (*width > 0).then_some(LabelBox { top_left: Point { x: *x, y: *y }, width: *width as f64, height: *height as f64 })).collect();
        let ours = inspect(&graph, &routes, &boxes);
        let ours = [ours.node_overlaps as f64, ours.route_obstructions as f64, ours.text_occlusions as f64, ours.crossings, ours.detour, ours.route_length];
        if ours.iter().map(|value| value.to_bits()).ne(theirs.iter().map(|value| value.to_bits())) {
            differ.push(format!("case {}: ours {ours:?}, TALA {theirs:?}", case.case));
        }
        counted += 1;
        through_a_card += usize::from(theirs[1] > 0.0);
        covered += usize::from(theirs[2] > 0.0);
        crossing += usize::from(theirs[3] > 0.0);
    }
    assert_eq!(differ.len(), 0, "{} differ, the first: {}", differ.len(), differ[0]);
    assert_eq!((counted, through_a_card, covered, crossing), (174, 1, 45, 46));
}

/// The whole pipeline, from the diagram as it is given to the finished drawing: the cards in
/// TALA's order at TALA's final places, the routes and the label places.
#[test]
fn a_diagram_of_plain_cards_gets_tala_s_drawing() {
    let cases: Vec<Case> = serde_json::from_str(include_str!("fixtures/routed.tala.json")).unwrap();
    let mut differ = Vec::new();
    for case in &cases {
        let nodes = case.input.nodes.iter().enumerate().map(|(card, [width, height])| Node { top_left: Point { x: 13.0 * card as f64 + 7.0, y: -31.0 }, width: *width, height: *height }).collect();
        let edges = case.input.edges.iter().map(|[from, to, width, height]| Edge { from: *from, to: *to, min_width: *width as f64, min_height: *height as f64 }).collect();
        let last = case.last.as_ref().expect("every case has the final drawing");
        let same = layout(&Graph::new(nodes, edges), case.input.seed).is_ok_and(|drawing| {
            let ids: Vec<i64> = (0..drawing.graph.nodes.len()).map(|node| drawing.graph.id(node)).collect();
            let cards: Vec<[f64; 2]> = drawing.graph.nodes.iter().map(|node| [node.top_left.x, node.top_left.y]).collect();
            let routes: Vec<Vec<[f64; 2]>> = drawing.routes.iter().map(|route| route.iter().map(|point| [point.x, point.y]).collect()).collect();
            let labels: Vec<[f64; 2]> = drawing.labels.iter().map(|label| label.as_ref().map_or([0.0, 0.0], |label| [label.top_left.x, label.top_left.y])).collect();
            ids == case.ids && cards == last.cards && routes == last.routes && labels == last.labels
        });
        if !same {
            differ.push(format!("case {} ({} cards)", case.case, case.cards.len()));
        }
    }
    assert_eq!(differ.len(), 0, "{} of {} differ: {}", differ.len(), cases.len(), differ.join("; "));
    assert_eq!(cases.len(), 174);
}
