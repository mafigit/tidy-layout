// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.

//! The routing stages of TALA's pipeline, replayed for diagrams with trees recorded from
//! TALA's own Go code (docs/reference.md): the cards where the placement stages leave them, in
//! the order TALA's graph has them, what TALA knows of the trees then, and the routes and
//! label places after the stages.

use tidy_layout::graph::{Edge, Graph, Node, Point, TreeLink};
use tidy_layout::pipeline::layout;
use tidy_layout::place::normalize;
use tidy_layout::routing::coordinator::{route_graph, route_stages};
use tidy_layout::routing::geometry::Orientation;
use tidy_layout::routing::labels::{LabelSize, Position};
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
    /// Per card of a tree: the connection to the card it hangs on, and the side of that card
    /// the tree lies on.
    trees: Vec<Option<(usize, String)>>,
    /// The connections TALA takes for tree connections.
    #[serde(rename = "treeLinks")]
    tree_links: Vec<usize>,
    /// Where the tree stage wants the label of a connection: the connection, the position, and
    /// how far along the route, from 0 to 1.
    presets: Vec<(usize, String, f64)>,
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
    let side = |name: &str| match name {
        "Top" => Orientation::Top,
        "Bottom" => Orientation::Bottom,
        "Left" => Orientation::Left,
        "Right" => Orientation::Right,
        other => panic!("a tree on the side {other}"),
    };
    Graph {
        ids: case.ids.clone(),
        own_edges: case.own.clone(),
        tree_links: case.trees.iter().map(|link| link.as_ref().map(|(edge, name)| TreeLink { edge: *edge, side: side(name) })).collect(),
        tree_edges: (0..case.links.len()).map(|link| case.tree_links.contains(&link)).collect(),
        label_presets: (0..case.links.len())
            .map(|link| {
                case.presets.iter().find(|(other, _, _)| *other == link).map(|(_, name, share)| {
                    let position = match name.as_str() {
                        "UNLOCKED_TOP" => Position::UnlockedTop,
                        "UNLOCKED_MIDDLE" => Position::UnlockedMiddle,
                        "UNLOCKED_BOTTOM" => Position::UnlockedBottom,
                        other => panic!("a label preset {other}"),
                    };
                    (position, *share)
                })
            })
            .collect(),
        ..Graph::new(nodes, edges)
    }
}

#[test]
fn the_first_routing_stage_lays_tala_s_routes_for_trees() {
    let cases: Vec<Case> = serde_json::from_str(include_str!("fixtures/routed_trees.tala.json")).unwrap();
    // Connections, tree connections, those of them that are one straight line, those that are
    // an S with two bends, cards of trees whose connection TALA takes for no tree connection,
    // and diagrams in which a card's own list of connections is in another order than the
    // graph's.
    let (mut connections, mut tree, mut straight, mut bent, mut loose, mut reordered, mut differ) = (0, 0, 0, 0, 0, 0, Vec::new());
    for case in &cases {
        let graph = graph_of(case);
        connections += case.links.len();
        tree += case.tree_links.len();
        straight += case.tree_links.iter().filter(|link| case.routed[**link].len() == 2).count();
        bent += case.tree_links.iter().filter(|link| case.routed[**link].len() == 4).count();
        loose += case.trees.iter().flatten().filter(|(link, _)| !case.tree_links.contains(link)).count();
        reordered += usize::from(case.own.iter().any(|own| own.windows(2).any(|pair| pair[0] > pair[1])));
        let ours: Vec<Vec<[f64; 2]>> = route_graph(&graph).unwrap().iter().map(|route| route.iter().map(|point| [point.x, point.y]).collect()).collect();
        if ours != case.routed {
            differ.push(format!("case {} (seed {}, {} cards)", case.case, case.seed, case.cards.len()));
        }
    }
    assert_eq!(differ.len(), 0, "{} differ: {}", differ.len(), differ.join("; "));
    assert_eq!((cases.len(), connections, tree, straight, bent, loose, reordered), (73, 477, 377, 165, 212, 15, 32));
}

/// The numbers: the diagrams, those with TALA's routes after `BalanceEdgeSegments`, with
/// TALA's routes at the end, with every label where TALA puts it after `PlaceLabels` and at
/// the end, and with TALA's cards, routes and labels after `Normalize`; the labels in all,
/// those the tree stage wants on the line, above and below it; and the diagrams in which
/// `Dejitter` moves a card and in which the second `BinPack` does.
#[test]
fn the_later_routing_stages_lay_tala_s_routes_for_trees() {
    let cases: Vec<Case> = serde_json::from_str(include_str!("fixtures/routed_trees.tala.json")).unwrap();
    let (mut balanced, mut end, mut labelled, mut labelled_at_the_end, mut normalized) = (0, 0, 0, 0, 0);
    let (mut label_count, mut on_the_line, mut above, mut below, mut dejittered, mut packed) = (0, 0, 0, 0, 0, 0);
    for case in &cases {
        let mut graph = graph_of(case);
        let labels: Vec<Option<LabelSize>> = case.links.iter().map(|[_, _, width, height]| (*width > 0).then_some(LabelSize { width: *width as f64, height: *height as f64 })).collect();
        let staged = route_stages(&mut graph, &labels).unwrap();
        let now: Vec<[f64; 2]> = graph.nodes.iter().map(|node| [node.top_left.x, node.top_left.y]).collect();
        let before: Vec<[f64; 2]> = case.cards.iter().map(|[x, y, _, _]| [*x, *y]).collect();
        assert_eq!(&now, case.packed.as_ref().or(case.dejittered.as_ref()).unwrap_or(&before), "case {}: the cards after Dejitter and the second BinPack", case.case);
        dejittered += usize::from(case.dejittered.is_some());
        packed += usize::from(case.packed.is_some());
        let points = |routes: &[Vec<Point>]| -> Vec<Vec<[f64; 2]>> { routes.iter().map(|route| route.iter().map(|point| [point.x, point.y]).collect()).collect() };
        let places = |labels: &[Option<tidy_layout::routing::labels::Placed>]| -> Vec<[f64; 2]> { labels.iter().map(|label| label.as_ref().map_or([0.0, 0.0], |label| [label.top_left.x, label.top_left.y])).collect() };
        balanced += usize::from(case.balanced == points(&staged.balanced));
        end += usize::from(case.end == points(&staged.routing.routes));
        labelled += usize::from(case.labels == places(&staged.placed));
        labelled_at_the_end += usize::from(case.labels_end == places(&staged.routing.labels));
        label_count += labels.iter().flatten().count();
        on_the_line += case.presets.iter().filter(|(_, name, _)| name == "UNLOCKED_MIDDLE").count();
        above += case.presets.iter().filter(|(_, name, _)| name == "UNLOCKED_TOP").count();
        below += case.presets.iter().filter(|(_, name, _)| name == "UNLOCKED_BOTTOM").count();
        let (mut routes, mut placed) = (staged.routing.routes.clone(), staged.routing.labels.clone());
        normalize(&mut graph, &mut routes, &mut placed, &labels);
        let cards: Vec<[f64; 2]> = graph.nodes.iter().map(|node| [node.top_left.x, node.top_left.y]).collect();
        normalized += usize::from(case.last.cards == cards && case.last.routes == points(&routes) && case.last.labels == places(&placed));
    }
    assert_eq!((cases.len(), balanced, end, labelled, labelled_at_the_end, normalized), (73, 73, 73, 73, 73, 73));
    assert_eq!((label_count, on_the_line, above, below, dejittered, packed), (263, 85, 64, 52, 3, 5));
}

/// The whole pipeline, from the diagram as it is given to the finished drawing: the cards in
/// TALA's order at TALA's final places, the routes and the label places. Here the port's own
/// placement says what a tree is, on which side it lies and where its labels are to go.
#[test]
fn a_diagram_with_trees_gets_tala_s_drawing() {
    let cases: Vec<Case> = serde_json::from_str(include_str!("fixtures/routed_trees.tala.json")).unwrap();
    let mut differ = Vec::new();
    for case in &cases {
        let nodes = case.input.nodes.iter().enumerate().map(|(card, [width, height])| Node { top_left: Point { x: 13.0 * card as f64 + 7.0, y: -31.0 }, width: *width, height: *height }).collect();
        let edges = case.input.edges.iter().map(|[from, to, width, height]| Edge { from: *from, to: *to, min_width: *width as f64, min_height: *height as f64 }).collect();
        let same = layout(&Graph::new(nodes, edges), case.input.seed).is_ok_and(|drawing| {
            let ids: Vec<i64> = (0..drawing.graph.nodes.len()).map(|node| drawing.graph.id(node)).collect();
            let cards: Vec<[f64; 2]> = drawing.graph.nodes.iter().map(|node| [node.top_left.x, node.top_left.y]).collect();
            let routes: Vec<Vec<[f64; 2]>> = drawing.routes.iter().map(|route| route.iter().map(|point| [point.x, point.y]).collect()).collect();
            let labels: Vec<[f64; 2]> = drawing.labels.iter().map(|label| label.as_ref().map_or([0.0, 0.0], |label| [label.top_left.x, label.top_left.y])).collect();
            ids == case.ids && cards == case.last.cards && routes == case.last.routes && labels == case.last.labels
        });
        if !same {
            differ.push(format!("case {} ({} cards)", case.case, case.cards.len()));
        }
    }
    assert_eq!(differ.len(), 0, "{} of {} differ: {}", differ.len(), cases.len(), differ.join("; "));
    assert_eq!(cases.len(), 73);
}
