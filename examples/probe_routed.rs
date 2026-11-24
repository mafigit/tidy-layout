// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.

//! Measures how far the router is from the routes TALA's whole pipeline ends with.
//!
//!   probe_routed <file>
//!   probe_routed compare <file> <limit>   see `compare`
//!   probe_routed whole <file> <limit>     see `whole`
//!
//! The file is a recording of the stages made with `STAGES_ROUTED` set
//! (docs/recorders/stages_test.go.txt): the places and routes after every stage from the first
//! routing on. The cards are taken where TALA has them before its last stage (`Normalize`), the
//! router routes them, and the routes are compared with TALA's there.

use tidy_layout::graph::{Arrangement, Edge, Graph, Node, Point, RoutedCluster, TreeLink};
use tidy_layout::routing::geometry::Orientation;
use tidy_layout::routing::coordinator::{route_edges_with_labels, route_graph};
use tidy_layout::pipeline::{SEEDS, layout_best};
use tidy_layout::place::prescale;
use tidy_layout::score::{Score, best_seed, evaluate};
use tidy_layout::routing::labels::{LabelSize, Position};
use tidy_layout::trees::find_trees;
use serde::Deserialize;

#[derive(Deserialize)]
struct Stage {
    stage: String,
    #[serde(default)]
    index: usize,
    at: Option<Vec<[f64; 2]>>,
    routes: Option<Vec<Vec<[f64; 2]>>>,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct Case {
    nodes_placed: Vec<usize>,
    edges_placed: Vec<[usize; 2]>,
    nodes: Vec<[f64; 2]>,
    edges: Vec<[usize; 4]>,
    seed: i64,
    trees: bool,
    hierarchies: bool,
    clusters: bool,
    placed_sizes: Vec<[f64; 2]>,
    placed: Vec<[f64; 2]>,
    moved: Vec<Stage>,
    #[serde(default)]
    routed: Vec<Stage>,
}

/// A diagram of plain cards as tests/fixtures/routed.tala.json has it.
#[derive(Deserialize)]
struct Routed {
    #[serde(default)]
    case: usize,
    cards: Vec<[f64; 4]>,
    ids: Vec<i64>,
    links: Vec<[usize; 4]>,
    routed: Vec<Vec<[f64; 2]>>,
    #[serde(default)]
    dejittered: Option<Vec<[f64; 2]>>,
    #[serde(default)]
    packed: Option<Vec<[f64; 2]>>,
    balanced: Option<Vec<Vec<[f64; 2]>>>,
    end: Option<Vec<Vec<[f64; 2]>>>,
    #[serde(default)]
    labels: Option<Vec<[f64; 2]>>,
    #[serde(default, rename = "labelsEnd")]
    labels_end: Option<Vec<[f64; 2]>>,
    #[serde(default)]
    metrics: Option<Vec<f64>>,
    #[serde(default, rename = "final")]
    last: Option<Final>,
    /// TALA's score of its finished drawing, and of the drawings of seeds 1, 2 and 3, where
    /// the recording has them.
    #[serde(default)]
    score: Option<[f64; 2]>,
    #[serde(default, rename = "seedScores")]
    seed_scores: Option<Vec<[f64; 2]>>,
    /// For a diagram with trees (routed_fixture.py with `trees`): every card's own list of
    /// connections, per card of a tree the connection to the card it hangs on and the side
    /// of that card it lies on, and the connections TALA takes for tree connections.
    #[serde(default)]
    own: Vec<Vec<usize>>,
    #[serde(default)]
    trees: Vec<Option<(usize, String)>>,
    #[serde(default, rename = "treeLinks")]
    tree_links: Vec<usize>,
    /// Where the tree stage wants a connection's label: the connection, the position and how
    /// far along the route.
    #[serde(default)]
    presets: Vec<(usize, String, f64)>,
    /// For a diagram with a hierarchy: per card of one the number of its hierarchy and its
    /// level in it.
    #[serde(default)]
    levels: Vec<Option<(usize, i64)>>,
    /// For a diagram with a cluster: per cluster its cards in its order, its arrangement and
    /// the one the cards around it asked for.
    #[serde(default)]
    clusters: Vec<(Vec<usize>, String, String)>,
    input: serde_json::Value,
}

/// The diagram as TALA's graph has it when routing starts.
fn graph_of(case: &Routed) -> Graph {
    let nodes = case.cards.iter().map(|[x, y, width, height]| Node { top_left: Point { x: *x, y: *y }, width: *width, height: *height }).collect();
    let edges = case.links.iter().map(|[from, to, width, height]| Edge { from: *from, to: *to, min_width: *width as f64, min_height: *height as f64 }).collect();
    let side = |name: &str| match name {
        "Top" => Orientation::Top,
        "Bottom" => Orientation::Bottom,
        "Left" => Orientation::Left,
        _ => Orientation::Right,
    };
    Graph {
        ids: case.ids.clone(),
        own_edges: case.own.clone(),
        tree_links: case.trees.iter().map(|link| link.as_ref().map(|(edge, name)| TreeLink { edge: *edge, side: side(name) })).collect(),
        tree_edges: if case.trees.is_empty() { Vec::new() } else { (0..case.links.len()).map(|link| case.tree_links.contains(&link)).collect() },
        label_presets: (0..case.links.len())
            .map(|link| {
                case.presets.iter().find(|(other, _, _)| *other == link).map(|(_, name, share)| {
                    let position = match name.as_str() {
                        "UNLOCKED_TOP" => Position::UnlockedTop,
                        "UNLOCKED_BOTTOM" => Position::UnlockedBottom,
                        _ => Position::UnlockedMiddle,
                    };
                    (position, *share)
                })
            })
            .collect(),
        levels: case.levels.clone(),
        clusters: case
            .clusters
            .iter()
            .map(|(members, now, desired)| {
                let arrangement = |name: &str| if name == "Row" { Arrangement::Row } else { Arrangement::Column };
                RoutedCluster { members: members.clone(), arrangement: arrangement(now), desired: arrangement(desired) }
            })
            .collect(),
        ..Graph::new(nodes, edges)
    }
}

#[derive(Deserialize)]
struct Final {
    cards: Vec<[f64; 2]>,
    routes: Vec<Vec<[f64; 2]>>,
    labels: Vec<[f64; 2]>,
}

/// `probe_routed compare <file> <limit>`: the first routing stage for every diagram of a file
/// made by docs/recorders/routed_fixture.py. Prints the diagrams that differ, as the recorder
/// takes them, and fails if there is one.
fn compare(file: &str, limit: usize) {
    let cases: Vec<Routed> = serde_json::from_str(&std::fs::read_to_string(file).unwrap()).unwrap();
    std::panic::set_hook(Box::new(|_| {}));
    let mut differ = Vec::new();
    let mut counts = [0usize; 9];
    for case in &cases {
        // Two cards that touch: TALA's own placement leaves such a pair now and then (6 of
        // 44,119 diagrams). They are counted, and compared like all others.
        let touch = |a: &[f64; 4], b: &[f64; 4]| a[0] <= b[0] + b[2] && b[0] <= a[0] + a[2] && a[1] <= b[1] + b[3] && b[1] <= a[1] + a[3];
        let touching = case.cards.iter().enumerate().any(|(index, card)| case.cards[..index].iter().any(|other| touch(card, other)));
        let graph = graph_of(case);
        let points = |routes: &[Vec<Point>]| -> Vec<Vec<[f64; 2]>> { routes.iter().map(|route| route.iter().map(|point| [point.x, point.y]).collect()).collect() };
        let ours = std::panic::catch_unwind(|| route_graph(&graph).ok()).ok().flatten();
        let labels: Vec<Option<LabelSize>> = case.links.iter().map(|[_, _, width, height]| (*width > 0).then_some(LabelSize { width: *width as f64, height: *height as f64 })).collect();
        // The later stages may move a card (`Dejitter`): they get their own copy of the cards.
        let later = std::panic::catch_unwind(|| {
            let mut after = graph_of(case);
            tidy_layout::routing::coordinator::route_stages(&mut after, &labels).ok().map(|staged| (staged, after))
        })
        .ok()
        .flatten();
        let (later, after) = match later {
            Some((staged, after)) => (Some(staged), after),
            None => (None, graph_of(case)),
        };
        let first = ours.is_some_and(|ours| points(&ours) == case.routed);
        // `Dejitter` and the second `BinPack` move the cards TALA moves, and none where TALA
        // moves none.
        let dejittered = later.is_some() && after.nodes.iter().map(|node| [node.top_left.x, node.top_left.y]).eq(case.packed.clone().or(case.dejittered.clone()).unwrap_or(case.cards.iter().map(|[x, y, _, _]| [*x, *y]).collect()));
        counts[7] += usize::from(case.dejittered.is_some());
        counts[3] += usize::from(case.packed.is_some());
        counts[5] += usize::from(touching);
        // The later stages count where TALA moves no card after the first routing.
        let balanced = case.balanced.as_ref().is_none_or(|theirs| later.as_ref().is_some_and(|ours| points(&ours.balanced) == *theirs));
        let places = |labels: &[Option<tidy_layout::routing::labels::Placed>]| -> Vec<[f64; 2]> { labels.iter().map(|label| label.as_ref().map_or([0.0, 0.0], |label| [label.top_left.x, label.top_left.y])).collect() };
        // The routes and the label places at the end, before `Normalize`.
        let end = case.end.as_ref().is_none_or(|theirs| later.as_ref().is_some_and(|ours| points(&ours.routing.routes) == *theirs))
            && case.labels_end.as_ref().is_none_or(|theirs| later.as_ref().is_some_and(|ours| places(&ours.routing.labels) == *theirs));
        // The drawing after the last stage, `Normalize`.
        let end = end
            && case.last.as_ref().is_none_or(|theirs| {
                later.as_ref().is_some_and(|ours| {
                    let mut moved = Graph { nodes: after.nodes.clone(), ..graph_of(case) };
                    let (mut routes, mut placed) = (ours.routing.routes.clone(), ours.routing.labels.clone());
                    tidy_layout::place::normalize(&mut moved, &mut routes, &mut placed, &labels);
                    moved.nodes.iter().map(|node| [node.top_left.x, node.top_left.y]).eq(theirs.cards.iter().copied()) && points(&routes) == theirs.routes && places(&placed) == theirs.labels
                })
            });
        // What TALA counts on its own drawing at the end.
        let counted = case.metrics.as_ref().zip(case.end.as_ref()).zip(case.labels_end.as_ref()).is_none_or(|((theirs, end), labels)| {
            let routes: Vec<Vec<Point>> = end.iter().map(|route| route.iter().map(|[x, y]| Point { x: *x, y: *y }).collect()).collect();
            let boxes: Vec<Option<tidy_layout::quality::LabelBox>> = case
                .links
                .iter()
                .zip(labels)
                .map(|([_, _, width, height], [x, y])| (*width > 0).then_some(tidy_layout::quality::LabelBox { top_left: Point { x: *x, y: *y }, width: *width as f64, height: *height as f64 }))
                .collect();
            let mut theirs_graph = Graph::new(graph.nodes.clone(), graph.edges.clone());
            for (node, at) in theirs_graph.nodes.iter_mut().zip(case.packed.as_ref().or(case.dejittered.as_ref()).into_iter().flatten()) {
                node.top_left = Point { x: at[0], y: at[1] };
            }
            let ours = tidy_layout::quality::inspect(&theirs_graph, &routes, &boxes);
            [ours.node_overlaps as f64, ours.route_obstructions as f64, ours.text_occlusions as f64, ours.crossings, ours.detour, ours.route_length].iter().map(|value| value.to_bits()).eq(theirs.iter().map(|value| value.to_bits()))
        });
        counts[0] += usize::from(!first);
        counts[1] += usize::from(!balanced);
        counts[2] += usize::from(!end);
        // The labels right after `PlaceLabels`.
        let labelled = case.labels.as_ref().is_none_or(|theirs| later.as_ref().is_some_and(|ours| places(&ours.placed) == *theirs));
        counts[4] += usize::from(!labelled);
        counts[6] += usize::from(!counted);
        counts[8] += usize::from(!dejittered);
        if !first || !balanced || !labelled || !end || !counted || !dejittered {
            differ.push(&case.input);
            // `WHICH=1` names the diagram and the connections whose first routes differ.
            if std::env::var("WHICH").is_ok() {
                eprintln!("case {}: {} cards, {} connections; first routing {}, Dejitter {}, balanced {}, labels {}, end {}, counts {}", case.case, case.cards.len(), case.links.len(), first, dejittered, balanced, labelled, end, counted);
                if !dejittered {
                    eprintln!("  cards after Dejitter and the second BinPack: ours {:?}, TALA {:?}", after.nodes.iter().map(|node| [node.top_left.x, node.top_left.y]).collect::<Vec<_>>(), case.packed.clone().or(case.dejittered.clone()));
                }
                if let Some(ours) = std::panic::catch_unwind(|| route_graph(&graph).ok()).ok().flatten() {
                    for (edge, (ours, theirs)) in points(&ours).iter().zip(&case.routed).enumerate().filter(|(_, (ours, theirs))| ours != theirs) {
                        eprintln!("  connection {edge} {:?}: ours {ours:?}, TALA {theirs:?}", case.links[edge]);
                    }
                }
            }
        }
    }
    eprintln!(
        "{} diagrams: {} differ after the first routing, {} in the cards after Dejitter and the second BinPack (Dejitter moves one in {}), {} after BalanceEdgeSegments, {} in a label, {} at the end, {} in what Inspect counts; the second BinPack moves a card in {}; two cards touch in {}",
        cases.len(),
        counts[0],
        counts[8],
        counts[7],
        counts[1],
        counts[4],
        counts[2],
        counts[6],
        counts[3],
        counts[5]
    );
    println!("{}", serde_json::to_string(&differ.iter().take(limit).collect::<Vec<_>>()).unwrap());
    if !differ.is_empty() {
        std::process::exit(1);
    }
}

/// `probe_routed whole <file> <limit>`: the whole pipeline (`pipeline::layout`) for every
/// diagram of such a file, from the diagram as the recorder was given it to the finished
/// drawing. Prints the diagrams that differ and fails if there is one.
fn whole(file: &str, limit: usize) {
    let cases: Vec<Routed> = serde_json::from_str(&std::fs::read_to_string(file).unwrap()).unwrap();
    std::panic::set_hook(Box::new(|_| {}));
    let (mut differ, mut errors) = (Vec::new(), 0);
    let (mut scored, mut seeded, mut other_score, mut other_seed) = (0, 0, Vec::new(), Vec::new());
    for case in &cases {
        let Some(last) = &case.last else { continue };
        let nodes: Vec<Node> = case.input["nodes"].as_array().unwrap().iter().enumerate().map(|(card, size)| Node { top_left: Point { x: 13.0 * card as f64 + 7.0, y: -31.0 }, width: size[0].as_f64().unwrap(), height: size[1].as_f64().unwrap() }).collect();
        let edges: Vec<Edge> = case.input["edges"].as_array().unwrap().iter().map(|edge| Edge { from: edge[0].as_u64().unwrap() as usize, to: edge[1].as_u64().unwrap() as usize, min_width: edge[2].as_f64().unwrap(), min_height: edge[3].as_f64().unwrap() }).collect();
        let seed = case.input["seed"].as_i64().unwrap();
        let drawing = std::panic::catch_unwind(|| tidy_layout::pipeline::layout(&Graph::new(nodes.clone(), edges.clone()), seed).ok()).ok().flatten();
        errors += usize::from(drawing.is_none());
        let graph = Graph::new(nodes, edges);
        if let (Some(theirs), Some(drawing)) = (case.score, &drawing) {
            scored += 1;
            let score = evaluate(&drawing.graph, &drawing.routes, &drawing.labels);
            if [score.penalty, score.area] != theirs {
                if other_score.len() < 5 {
                    eprintln!("case {} seed {seed}: score {score:?}, TALA {theirs:?}", case.case);
                }
                other_score.push(&case.input);
            }
        }
        if let Some(theirs) = &case.seed_scores {
            seeded += 1;
            let theirs: Vec<Score> = theirs.iter().map(|pair| Score { penalty: pair[0], area: pair[1] }).collect();
            let best = std::panic::catch_unwind(|| layout_best(&graph, &SEEDS).ok()).ok().flatten();
            let pick = best_seed(&theirs).map(|index| (SEEDS[index], theirs[index]));
            if best.map(|best| (best.seed, best.score)) != pick {
                other_seed.push(&case.input);
            }
        }
        let same = drawing.is_some_and(|drawing| {
            (0..drawing.graph.nodes.len()).map(|node| drawing.graph.id(node)).eq(case.ids.iter().copied())
                && drawing.graph.nodes.iter().map(|node| [node.top_left.x, node.top_left.y]).eq(last.cards.iter().copied())
                && drawing.routes.iter().map(|route| route.iter().map(|point| [point.x, point.y]).collect::<Vec<_>>()).eq(last.routes.iter().cloned())
                && drawing.labels.iter().map(|label| label.as_ref().map_or([0.0, 0.0], |label| [label.top_left.x, label.top_left.y])).eq(last.labels.iter().copied())
        });
        if !same {
            differ.push(&case.input);
        }
    }
    eprintln!("{} diagrams: {} differ from TALA's finished drawing ({errors} got none)", cases.len(), differ.len());
    eprintln!("score: {} of {scored} differ from TALA's; seeds: TALA picks another in {} of {seeded}", other_score.len(), other_seed.len());
    for input in other_score.iter().chain(&other_seed) {
        if !differ.contains(input) {
            differ.push(input);
        }
    }
    println!("{}", serde_json::to_string(&differ.iter().take(limit).collect::<Vec<_>>()).unwrap());
    if !differ.is_empty() {
        std::process::exit(1);
    }
}

fn main() {
    let arguments: Vec<String> = std::env::args().collect();
    if arguments.get(1).map(String::as_str) == Some("whole") {
        return whole(&arguments[2], arguments.get(3).and_then(|limit| limit.parse().ok()).unwrap_or(usize::MAX));
    }
    if arguments.get(1).map(String::as_str) == Some("compare") {
        return compare(&arguments[2], arguments.get(3).and_then(|limit| limit.parse().ok()).unwrap_or(usize::MAX));
    }
    let cases: Vec<Case> = serde_json::from_str(&std::fs::read_to_string(&arguments[1]).unwrap()).unwrap();
    // Per kind of diagram: diagrams, those with all routes the same, connections, those the same,
    // diagrams the router refused or broke on.
    let mut counts = std::collections::BTreeMap::<&str, [usize; 5]>::new();
    std::panic::set_hook(Box::new(|_| {}));
    for case in cases.iter().filter(|case| !case.edges.is_empty()) {
        let before_last = || case.routed.iter().filter(|stage| stage.stage != "Normalize");
        let at = before_last().filter_map(|stage| stage.at.as_ref()).next_back().or(case.moved.iter().filter_map(|stage| stage.at.as_ref()).next_back()).unwrap_or(&case.placed);
        let Some(theirs) = before_last().filter_map(|stage| stage.routes.as_ref()).next_back() else {
            continue;
        };
        let nodes = at.iter().zip(&case.placed_sizes).map(|([x, y], [width, height])| Node { top_left: Point { x: *x, y: *y }, width: *width, height: *height }).collect();
        let edges = case.edges.iter().map(|[from, to, width, height]| Edge { from: *from, to: *to, min_width: *width as f64, min_height: *height as f64 }).collect();
        let labels: Vec<Option<LabelSize>> = case.edges.iter().map(|[_, _, width, height]| (*width > 0).then_some(LabelSize { width: *width as f64, height: *height as f64 })).collect();
        let graph = Graph::new(nodes, edges);
        let kind = match (case.trees, case.hierarchies, case.clusters) {
            (false, false, false) => "plain",
            (true, false, false) => "trees",
            (false, true, false) => "hierarchy",
            (false, false, true) => "cluster",
            _ => "several",
        };
        let count = counts.entry(kind).or_default();
        count[0] += 1;
        count[2] += theirs.len();
        let ours = std::panic::catch_unwind(|| route_edges_with_labels(&graph, &labels).ok().map(|routing| routing.routes));
        let Ok(Some(ours)) = ours else {
            count[4] += 1;
            continue;
        };
        let same = ours.iter().zip(theirs).filter(|(ours, theirs)| ours.iter().map(|point| [point.x, point.y]).eq(theirs.iter().copied())).count();
        count[3] += same;
        count[1] += usize::from(same == theirs.len());
    }
    // For plain cards, stage by stage: where does the router leave TALA's routes? The cards
    // are where TALA has them after `Rescale`; no plain diagram moves a card after that.
    let mut steps = [0usize; 4];
    for case in cases.iter().filter(|case| !case.edges.is_empty() && !case.trees && !case.hierarchies && !case.clusters) {
        let Some(at) = case.moved.iter().filter_map(|stage| stage.at.as_ref()).next_back() else {
            continue;
        };
        // TALA's tree stage leaves the cards and connections in another order, also where it
        // finds no tree. The router is given that order; its routes are put back in the
        // order of the diagram.
        let mut given = Graph::new(
            case.nodes.iter().map(|[width, height]| Node { top_left: Point { x: 0.0, y: 0.0 }, width: *width, height: *height }).collect(),
            case.edges.iter().map(|[from, to, width, height]| Edge { from: *from, to: *to, min_width: *width as f64, min_height: *height as f64 }).collect(),
        );
        prescale(&mut given);
        let found = find_trees(&given);
        if found.order.len() != case.nodes.len() || found.edges.len() != case.edges.len() {
            continue;
        }
        // The order TALA has after node placement: the recording gives the cards, and the
        // connections as pairs of cards. Connections between the same two cards the same way
        // keep the order the tree stage left them in.
        let mut left = found.edges.clone();
        let order: Vec<usize> = case
            .edges_placed
            .iter()
            .map(|[from, to]| left.remove(left.iter().position(|edge| case.edges[*edge][0] == *from && case.edges[*edge][1] == *to).expect("a connection of the diagram")))
            .collect();
        let found = tidy_layout::trees::Found { order: case.nodes_placed.clone(), edges: order, trees: Vec::new() };
        let place_of = |card: usize| found.order.iter().position(|other| *other == card).unwrap();
        let nodes = found.order.iter().map(|card| Node { top_left: Point { x: at[*card][0], y: at[*card][1] }, width: case.placed_sizes[*card][0], height: case.placed_sizes[*card][1] }).collect();
        let edges = found.edges.iter().map(|edge| Edge { from: place_of(case.edges[*edge][0]), to: place_of(case.edges[*edge][1]), min_width: case.edges[*edge][2] as f64, min_height: case.edges[*edge][3] as f64 }).collect();
        let labels: Vec<Option<LabelSize>> = found.edges.iter().map(|edge| (case.edges[*edge][2] > 0).then_some(LabelSize { width: case.edges[*edge][2] as f64, height: case.edges[*edge][3] as f64 })).collect();
        let graph = Graph::new(nodes, edges);
        let back = |routes: Vec<Vec<Point>>| {
            let mut ordered = vec![Vec::new(); routes.len()];
            for (route, edge) in routes.into_iter().zip(&found.edges) {
                ordered[*edge] = route;
            }
            ordered
        };
        let theirs = |until: usize| case.routed.iter().filter(|stage| stage.index <= until).filter_map(|stage| stage.routes.as_ref()).next_back();
        let same = |ours: Option<Vec<Vec<Point>>>, until: usize| {
            ours.zip(theirs(until)).is_some_and(|(ours, theirs)| ours.iter().zip(theirs).all(|(ours, theirs)| ours.iter().map(|point| [point.x, point.y]).eq(theirs.iter().copied())))
        };
        steps[0] += 1;
        let first = same(std::panic::catch_unwind(|| route_graph(&graph).ok().map(back)).ok().flatten(), 21);
        if !first && std::env::var("WHICH").is_ok() {
            eprintln!("differs after the first routing: {} cards, {} connections, seed {}", case.placed.len(), case.edges.len(), case.seed);
            if case.placed.len() <= std::env::var("WHICH").ok().and_then(|most| most.parse().ok()).unwrap_or(3) {
                eprintln!("  cards {:?} sizes {:?} connections {:?}", at, case.placed_sizes, case.edges);
                eprintln!("  ours {:?}", route_graph(&graph).ok().map(back).map(|routes| routes.iter().map(|route| route.iter().map(|point| (point.x, point.y)).collect::<Vec<_>>()).collect::<Vec<_>>()));
                eprintln!("  TALA {:?}", theirs(21));
            }
        }
        steps[1] += usize::from(first);
        // The steps after the search as `RouteEdges` has them, from the routes of the pipeline's
        // first stage.
        let chained = || {
            let mut routes = route_graph(&graph).ok()?;
            tidy_layout::routing::line::straighten(&graph, &mut routes);
            tidy_layout::routing::balance::balance(&graph, &mut routes);
            Some(routes)
        };
        let after_balance = same(std::panic::catch_unwind(|| chained().map(back)).ok().flatten(), 28);
        if !after_balance && std::env::var("WHICH").is_ok() {
            let changed: Vec<&str> = case.routed.iter().filter(|stage| stage.index > 21 && stage.index <= 28 && stage.routes.is_some()).map(|stage| stage.stage.as_str()).collect();
            eprintln!("differs after BalanceEdgeSegments: {} cards, {} connections, seed {}; TALA changed routes in {:?}", case.placed.len(), case.edges.len(), case.seed, changed);
        }
        steps[2] += usize::from(after_balance);
        // And the swap of doubled connections for their labels.
        let has_label: Vec<bool> = labels.iter().map(Option::is_some).collect();
        let to_the_end = || {
            let mut routes = chained()?;
            tidy_layout::routing::labels::reorder_duplicates(&graph, &mut routes, &has_label);
            Some(routes)
        };
        let at_the_end = same(std::panic::catch_unwind(|| to_the_end().map(back)).ok().flatten(), 35);
        if !at_the_end && after_balance && std::env::var("WHICH").is_ok() {
            let changed: Vec<&str> = case.routed.iter().filter(|stage| stage.index > 28 && stage.index <= 35 && stage.routes.is_some()).map(|stage| stage.stage.as_str()).collect();
            eprintln!("differs before Normalize: {} cards, {} connections, seed {}; TALA changed routes in {:?}", case.placed.len(), case.edges.len(), case.seed, changed);
        }
        steps[3] += usize::from(at_the_end);
    }
    println!("plain, cards as after Rescale: {} diagrams; as TALA's after the first EdgeRouting {}, after BalanceEdgeSegments {}, before Normalize {}", steps[0], steps[1], steps[2], steps[3]);
    for (kind, [diagrams, same, connections, same_connections, failed]) in counts {
        println!("{kind}: {diagrams} diagrams, {same} with every route as TALA's, {failed} not routed; {same_connections} of {connections} connections the same");
    }
}
