// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.

//! The stages of TALA's pipeline, replayed from what TALA's own Go code did (docs/reference.md).
//! So far: the tree, hierarchy and cluster stages and the node placement stage, for every
//! diagram.

use tidy_layout::clusters::{Arrangement, find_clusters, sizes as cluster_sizes, with_vessels};
use tidy_layout::graph::{Edge, Graph, Node, Point};
use tidy_layout::hierarchy::find_hierarchies;
use tidy_layout::hierarchy_layout::place_hierarchies;
use tidy_layout::pack::pack;
use tidy_layout::place::{pad, place, place_with_trees, prescale};
use tidy_layout::align::align;
use tidy_layout::balance::balance_symmetry;
use tidy_layout::cluster_stage::optimize_clusters;
use tidy_layout::equidistance::equidistance;
use tidy_layout::gaps::normalize_gaps;
use tidy_layout::swap::{swap, transpose_all};
use tidy_layout::trees::{Tree, find_trees, without_trees};
use serde_json::{Value, json};
use std::collections::BTreeMap;
use serde::Deserialize;

#[derive(Deserialize)]
struct Case {
    name: String,
    nodes: Vec<[f64; 2]>,
    edges: Vec<[usize; 4]>,
    seed: i64,
    trees: bool,
    hierarchies: bool,
    clusters: bool,
    sequences: bool,
    /// After the tree stage: the cards and connections left in the graph, and per card that
    /// trees hang on the roots as [card, from, to, children...].
    #[serde(rename = "treeOrder")]
    tree_order: Vec<usize>,
    #[serde(rename = "treeEdges")]
    tree_edges: Vec<[usize; 2]>,
    #[serde(rename = "treeRoots")]
    tree_roots: BTreeMap<String, Value>,
    /// After the hierarchy stage: per card its level in a hierarchy, or -1, and its place if it
    /// is in one.
    levels: Vec<i64>,
    #[serde(rename = "hierarchyPlaced")]
    hierarchy_placed: Vec<[f64; 2]>,
    /// The cards in TALA's order when node placement starts, and their sizes then.
    order: Vec<i64>,
    sizes: Vec<[f64; 2]>,
    #[serde(rename = "edgesAfter")]
    edges_after: Vec<[i64; 2]>,
    /// After the cluster stage: the clusters in the order they were made, and every card's size.
    #[serde(rename = "clusterList")]
    cluster_list: Vec<ClusterRecord>,
    #[serde(rename = "clusterSizes")]
    cluster_sizes: Vec<[f64; 2]>,
    cell: f64,
    placed: Vec<[f64; 2]>,
    /// After node placement: the clusters, which may have turned, and every card's size.
    #[serde(rename = "clustersPlaced")]
    clusters_placed: Vec<ClusterRecord>,
    #[serde(rename = "placedSizes")]
    placed_sizes: Vec<[f64; 2]>,
    /// TALA's graph after node placement: the cards and vessels in its order, the connections
    /// as [from, to], its top-level list, and per card its own list of connections.
    #[serde(rename = "nodesPlaced")]
    nodes_placed: Vec<i64>,
    #[serde(rename = "edgesPlaced")]
    edges_placed: Vec<[i64; 2]>,
    #[serde(rename = "topLevelPlaced")]
    top_level_placed: Vec<i64>,
    #[serde(rename = "ownEdgesPlaced")]
    own_edges_placed: Vec<Vec<usize>>,
    /// Every later stage before routing that moved a card, with the places after it.
    moved: Vec<Moved>,
}

#[derive(Deserialize)]
struct Moved {
    stage: String,
    /// The stage's number in TALA's plan. The stage that lines cards up runs four times.
    index: usize,
    at: Vec<[f64; 2]>,
}

#[derive(Deserialize, Debug, PartialEq)]
struct ClusterRecord {
    vessel: i64,
    nodes: Vec<usize>,
    arrangement: String,
    padding: f64,
    /// The vessel's place (0, 0 before node placement) and size.
    #[serde(default)]
    at: [f64; 2],
    size: [f64; 2],
}

#[test]
fn node_placement_puts_plain_cards_where_tala_puts_them() {
    let cases: Vec<Case> = serde_json::from_str(include_str!("fixtures/stages.tala.json")).unwrap();
    assert_eq!(cases.len(), 467);
    let (mut plain, mut cards, mut grown, mut differ) = (0, 0, 0, Vec::new());
    for (number, case) in cases.iter().enumerate() {
        // Where TALA lays part of the diagram out as a tree, a hierarchy or a cluster, the port
        // has nothing to say yet.
        if case.trees || case.hierarchies || case.clusters || case.sequences {
            continue;
        }
        // TALA's stages in their order: cards grow, the tree stage (which here leaves no tree
        // but its own order of cards and connections), node placement.
        let mut whole = given(case);
        prescale(&mut whole);
        let found = find_trees(&whole);
        assert!(found.trees.is_empty(), "case {number}: a tree");
        let mut graph = without_trees(&whole, &found);
        assert_eq!(graph.ids, case.order, "case {number}: the order node placement starts from");
        let sizes: Vec<[f64; 2]> = graph.nodes.iter().map(|card| [card.width, card.height]).collect();
        assert_eq!(sizes, case.sizes, "case {number}: sizes after cards with many connections grew");
        grown += usize::from(case.order.iter().zip(&sizes).any(|(card, size)| *size != case.nodes[*card as usize]));
        place(&mut graph, case.seed).unwrap();
        // By card, not by TALA's order.
        let mut at = vec![[0.0, 0.0]; case.nodes.len()];
        for (card, placed) in case.order.iter().zip(&graph.nodes) {
            at[*card as usize] = [placed.top_left.x, placed.top_left.y];
        }
        if at != case.placed || graph.cell_size != case.cell {
            differ.push(format!("case {number} ({}, {} cards, seed {})\n port {at:?}\n TALA {:?}", case.name, case.nodes.len(), case.seed, case.placed));
        }
        plain += 1;
        cards += case.nodes.len();
    }
    assert_eq!(differ.len(), 0, "{} differ, the first: {}", differ.len(), differ[0]);
    assert_eq!((plain, cards, grown), (203, 1361, 34));
}

#[test]
fn node_placement_lays_trees_out_where_tala_lays_them_out() {
    let cases: Vec<Case> = serde_json::from_str(include_str!("fixtures/stages.tala.json")).unwrap();
    let (mut with_trees, mut cards, mut differ) = (0, 0, Vec::new());
    for (number, case) in cases.iter().enumerate() {
        if !case.trees || case.hierarchies || case.clusters || case.sequences {
            continue;
        }
        let mut whole = given(case);
        prescale(&mut whole);
        let sizes: Vec<[f64; 2]> = whole.nodes.iter().map(|card| [card.width, card.height]).collect();
        let found = find_trees(&whole);
        place_with_trees(&mut whole, &found, case.seed).expect("a place for every card");
        let now: Vec<[f64; 2]> = whole.nodes.iter().map(|card| [card.width, card.height]).collect();
        assert_eq!(now, sizes, "case {number}: laying a tree out leaves the sizes");
        let placed: Vec<[f64; 2]> = whole.nodes.iter().map(|card| [card.top_left.x, card.top_left.y]).collect();
        if placed != case.placed || whole.cell_size != case.cell {
            differ.push(format!("case {number} ({}, {} cards, seed {})", case.name, case.nodes.len(), case.seed));
        }
        with_trees += 1;
        cards += case.nodes.len();
    }
    assert_eq!(differ.len(), 0, "{} differ, the first: {}", differ.len(), differ[0]);
    assert_eq!((with_trees, cards), (73, 647));
}

#[test]
fn the_hierarchy_stage_finds_the_levels_tala_finds() {
    let cases: Vec<Case> = serde_json::from_str(include_str!("fixtures/stages.tala.json")).unwrap();
    let (mut with_hierarchy, mut cards) = (0, 0);
    for (number, case) in cases.iter().enumerate() {
        let mut whole = given(case);
        prescale(&mut whole);
        let found = find_trees(&whole);
        let core = without_trees(&whole, &found);
        let mut levels = vec![-1; case.nodes.len()];
        for (card, level) in found.order.iter().zip(find_hierarchies(&core)) {
            levels[*card] = level.map_or(-1, |level| level as i64);
        }
        assert_eq!(levels, case.levels, "case {number} ({}, {} cards): the levels", case.name, case.nodes.len());
        with_hierarchy += usize::from(levels.iter().any(|level| *level >= 0));
        cards += levels.iter().filter(|level| **level >= 0).count();
        assert_eq!(case.hierarchies, levels.iter().any(|level| *level >= 0), "case {number}");
    }
    assert_eq!((with_hierarchy, cards), (55, 584));
}

#[test]
fn the_hierarchy_stage_lays_hierarchies_out_where_tala_lays_them_out() {
    let cases: Vec<Case> = serde_json::from_str(include_str!("fixtures/stages.tala.json")).unwrap();
    let (mut with_hierarchy, mut differ) = (0, Vec::new());
    for (number, case) in cases.iter().enumerate() {
        let mut whole = given(case);
        prescale(&mut whole);
        let found = find_trees(&whole);
        let core = without_trees(&whole, &found);
        let mut placed = vec![[0.0, 0.0]; case.nodes.len()];
        for (card, place) in found.order.iter().zip(place_hierarchies(&core, &find_hierarchies(&core), case.seed)) {
            placed[*card] = place.map_or([0.0, 0.0], |at| [at.x, at.y]);
        }
        if placed != case.hierarchy_placed {
            differ.push(format!("case {number} ({}, {} cards, seed {})", case.name, case.nodes.len(), case.seed));
        }
        with_hierarchy += usize::from(case.hierarchies);
    }
    assert_eq!(differ.len(), 0, "{} differ, the first: {}", differ.len(), differ[0]);
    assert_eq!(with_hierarchy, 55);
}

#[test]
fn node_placement_packs_hierarchies_where_tala_packs_them() {
    let cases: Vec<Case> = serde_json::from_str(include_str!("fixtures/stages.tala.json")).unwrap();
    let (mut with_hierarchy, mut cards, mut differ) = (0, 0, Vec::new());
    for (number, case) in cases.iter().enumerate() {
        if !case.hierarchies || case.clusters || case.sequences {
            continue;
        }
        let mut whole = given(case);
        prescale(&mut whole);
        let found = find_trees(&whole);
        place_with_trees(&mut whole, &found, case.seed).expect("a place for every card");
        let placed: Vec<[f64; 2]> = whole.nodes.iter().map(|card| [card.top_left.x, card.top_left.y]).collect();
        if placed != case.placed || whole.cell_size != case.cell {
            differ.push(format!("case {number} ({}, {} cards, seed {})", case.name, case.nodes.len(), case.seed));
        }
        with_hierarchy += 1;
        cards += case.nodes.len();
    }
    assert_eq!(differ.len(), 0, "{} differ, the first: {}", differ.len(), differ[0]);
    assert_eq!((with_hierarchy, cards), (52, 582));
}

#[test]
fn the_cluster_stage_finds_the_clusters_tala_finds() {
    let cases: Vec<Case> = serde_json::from_str(include_str!("fixtures/stages.tala.json")).unwrap();
    let (mut with_clusters, mut clusters, mut cards, mut rows) = (0, 0, 0, 0);
    for (number, case) in cases.iter().enumerate() {
        let what = format!("case {number} ({}, {} cards, seed {})", case.name, case.nodes.len(), case.seed);
        let mut whole = given(case);
        prescale(&mut whole);
        let found = find_trees(&whole);
        let core = without_trees(&whole, &found);
        let list = find_clusters(&core, &find_hierarchies(&core), case.nodes.len(), case.seed);
        // By the cards' own numbers, as TALA's are recorded.
        let mut sizes: Vec<[f64; 2]> = whole.nodes.iter().map(|card| [card.width, card.height]).collect();
        let recorded: Vec<ClusterRecord> = list
            .iter()
            .map(|cluster| {
                let ((width, height), vessel) = cluster_sizes(&core, cluster);
                for member in &cluster.members {
                    sizes[found.order[*member]] = [width, height];
                }
                ClusterRecord {
                    vessel: cluster.vessel,
                    nodes: cluster.members.iter().map(|member| found.order[*member]).collect(),
                    arrangement: if cluster.arrangement == Arrangement::Row { "Row" } else { "Column" }.to_string(),
                    padding: cluster.padding,
                    at: [0.0, 0.0],
                    size: [vessel.0, vessel.1],
                }
            })
            .collect();
        assert_eq!(recorded, case.cluster_list, "{what}: the clusters");
        assert_eq!(sizes, case.cluster_sizes, "{what}: the sizes of the cards");
        assert_eq!(case.clusters, !list.is_empty(), "{what}");
        // The graph as node placement meets it.
        let (graph, _) = with_vessels(&core, &list);
        assert_eq!(graph.ids, case.order, "{what}: the cards node placement starts from, in order");
        let now: Vec<[f64; 2]> = graph.nodes.iter().map(|card| [card.width, card.height]).collect();
        assert_eq!(now, case.sizes, "{what}: their sizes");
        let ends: Vec<[i64; 2]> = graph.edges.iter().map(|edge| [graph.id(edge.from), graph.id(edge.to)]).collect();
        assert_eq!(ends, case.edges_after, "{what}: the connections, in order");
        with_clusters += usize::from(!list.is_empty());
        clusters += list.len();
        cards += list.iter().map(|cluster| cluster.members.len()).sum::<usize>();
        rows += list.iter().filter(|cluster| cluster.arrangement == Arrangement::Row).count();
    }
    assert_eq!((with_clusters, clusters, cards, rows), (139, 179, 466, 30));
}

#[test]
fn node_placement_places_clusters_where_tala_places_them() {
    let cases: Vec<Case> = serde_json::from_str(include_str!("fixtures/stages.tala.json")).unwrap();
    let (mut with_clusters, mut cards, mut turned, mut differ) = (0, 0, 0, Vec::new());
    for (number, case) in cases.iter().enumerate() {
        if !case.clusters || case.sequences {
            continue;
        }
        let mut whole = given(case);
        prescale(&mut whole);
        let found = find_trees(&whole);
        let clusters = place_with_trees(&mut whole, &found, case.seed).expect("a place for every card").clusters;
        let placed: Vec<[f64; 2]> = whole.nodes.iter().map(|card| [card.top_left.x, card.top_left.y]).collect();
        let sizes: Vec<[f64; 2]> = whole.nodes.iter().map(|card| [card.width, card.height]).collect();
        // A vessel is the box around its cards.
        let recorded: Vec<ClusterRecord> = clusters
            .iter()
            .zip(&case.clusters_placed)
            .map(|(cluster, theirs)| {
                let of = |pick: fn(&Node) -> f64, most: bool| {
                    let all = cluster.members.iter().map(|member| pick(&whole.nodes[*member]));
                    if most { all.fold(f64::NEG_INFINITY, f64::max) } else { all.fold(f64::INFINITY, f64::min) }
                };
                let (left, top) = (of(|card| card.top_left.x, false), of(|card| card.top_left.y, false));
                ClusterRecord {
                    vessel: theirs.vessel,
                    nodes: cluster.members.clone(),
                    arrangement: if cluster.arrangement == Arrangement::Row { "Row" } else { "Column" }.to_string(),
                    padding: cluster.padding,
                    at: [left, top],
                    size: [of(|card| card.top_left.x + card.width, true) - left, of(|card| card.top_left.y + card.height, true) - top],
                }
            })
            .collect();
        if placed != case.placed || sizes != case.placed_sizes || whole.cell_size != case.cell || recorded != case.clusters_placed {
            differ.push(format!("case {number} ({}, {} cards, seed {})", case.name, case.nodes.len(), case.seed));
        }
        with_clusters += 1;
        cards += case.nodes.len();
        turned += usize::from(case.cluster_list.iter().zip(&case.clusters_placed).any(|(was, is)| was.arrangement != is.arrangement));
    }
    assert_eq!(differ.len(), 0, "{} differ, the first: {}", differ.len(), differ[0]);
    assert_eq!((with_clusters, cards, turned), (139, 1374, 32));
}

#[test]
fn node_placement_leaves_the_graph_tala_leaves() {
    let cases: Vec<Case> = serde_json::from_str(include_str!("fixtures/stages.tala.json")).unwrap();
    let (mut reordered, mut own_reordered) = (0, 0);
    for (number, case) in cases.iter().enumerate() {
        let what = format!("case {number} ({}, {} cards, seed {})", case.name, case.nodes.len(), case.seed);
        let mut whole = given(case);
        prescale(&mut whole);
        let found = find_trees(&whole);
        let graph = place_with_trees(&mut whole, &found, case.seed).expect("a place for every card").graph;
        assert_eq!(graph.ids, case.nodes_placed, "{what}: the cards and vessels, in order");
        let ends: Vec<[i64; 2]> = graph.edges.iter().map(|edge| [graph.id(edge.from), graph.id(edge.to)]).collect();
        assert_eq!(ends, case.edges_placed, "{what}: the connections, in order");
        let top_level: Vec<i64> = graph.cards_in_the_way().map(|node| graph.id(node)).collect();
        assert_eq!(top_level, case.top_level_placed, "{what}: the top-level list");
        let own: Vec<Vec<usize>> = (0..graph.nodes.len()).map(|node| graph.indexed_edges_of(node).map(|(index, _)| index).collect()).collect();
        assert_eq!(own, case.own_edges_placed, "{what}: every card's own list of connections");
        reordered += usize::from(case.top_level_placed != case.nodes_placed);
        own_reordered += usize::from(own.iter().any(|list| list.windows(2).any(|pair| pair[0] > pair[1])));
    }
    assert_eq!((reordered, own_reordered), (57, 57));
}

#[test]
fn the_swap_stage_swaps_the_cards_tala_swaps() {
    let cases: Vec<Case> = serde_json::from_str(include_str!("fixtures/stages.tala.json")).unwrap();
    let (mut moved, mut differ) = (0, Vec::new());
    for (number, case) in cases.iter().enumerate() {
        let mut whole = given(case);
        prescale(&mut whole);
        let found = find_trees(&whole);
        let mut placed = place_with_trees(&mut whole, &found, case.seed).expect("a place for every card");
        swap(&mut placed);
        let at: Vec<[f64; 2]> = placed.places(case.nodes.len()).iter().map(|place| [place.x, place.y]).collect();
        // The stage is recorded where it moved a card.
        let after = case.moved.first().filter(|stage| stage.stage == "SwapStuff").map_or(&case.placed, |stage| &stage.at);
        if at != *after {
            differ.push(format!("case {number} ({}, {} cards, seed {}, clusters {}, trees {}, hierarchies {})\n placed {:?}\n port {at:?}\n TALA {after:?}", case.name, case.nodes.len(), case.seed, case.clusters, case.trees, case.hierarchies, case.placed));
        }
        moved += usize::from(*after != case.placed);
    }
    assert_eq!(differ.len(), 0, "{} differ: {}", differ.len(), differ.iter().map(|one| one.lines().next().unwrap_or("")).collect::<Vec<_>>().join("; ") + "\n" + differ.first().map_or("", String::as_str));
    assert_eq!(moved, 76);
}

#[test]
fn the_transpose_stage_turns_the_branches_tala_turns() {
    let cases: Vec<Case> = serde_json::from_str(include_str!("fixtures/stages.tala.json")).unwrap();
    let (mut moved, mut differ) = (0, Vec::new());
    for (number, case) in cases.iter().enumerate() {
        let mut whole = given(case);
        prescale(&mut whole);
        let found = find_trees(&whole);
        let mut placed = place_with_trees(&mut whole, &found, case.seed).expect("a place for every card");
        swap(&mut placed);
        transpose_all(&mut placed);
        let at: Vec<[f64; 2]> = placed.places(case.nodes.len()).iter().map(|place| [place.x, place.y]).collect();
        // A stage is recorded where it moved a card.
        let after = case.moved.iter().take_while(|stage| ["SwapStuff", "Transpose"].contains(&stage.stage.as_str())).last().map_or(&case.placed, |stage| &stage.at);
        if at != *after {
            differ.push(format!("case {number} ({}, {} cards, seed {}, clusters {}, trees {}, hierarchies {})", case.name, case.nodes.len(), case.seed, case.clusters, case.trees, case.hierarchies));
        }
        moved += usize::from(case.moved.iter().any(|stage| stage.stage == "Transpose"));
    }
    assert_eq!(differ.len(), 0, "{} differ: {}", differ.len(), differ.join("; "));
    assert_eq!(moved, 31);
}

#[test]
fn the_align_stage_lines_up_the_cards_tala_lines_up() {
    let cases: Vec<Case> = serde_json::from_str(include_str!("fixtures/stages.tala.json")).unwrap();
    let (mut moved, mut differ) = (0, Vec::new());
    for (number, case) in cases.iter().enumerate() {
        let mut whole = given(case);
        prescale(&mut whole);
        let found = find_trees(&whole);
        let mut placed = place_with_trees(&mut whole, &found, case.seed).expect("a place for every card");
        swap(&mut placed);
        transpose_all(&mut placed);
        align(&mut placed);
        let at: Vec<[f64; 2]> = placed.places(case.nodes.len()).iter().map(|place| [place.x, place.y]).collect();
        // A stage is recorded where it moved a card. The stage runs again later: this is the
        // first time.
        let (after, aligned) = recorded(case, &["SwapStuff", "Transpose", "AlignAxes"]);
        if at != *after {
            differ.push(format!("case {number} ({}, {} cards, seed {}, clusters {}, trees {}, hierarchies {})", case.name, case.nodes.len(), case.seed, case.clusters, case.trees, case.hierarchies));
        }
        moved += usize::from(aligned);
    }
    assert_eq!(differ.len(), 0, "{} differ: {}", differ.len(), differ.join("; "));
    assert_eq!(moved, 210);
}

/// The stages after node placement, in TALA's order, and the number of each in TALA's plan.
const STAGES: [(&str, usize); 12] = [
    ("SwapStuff", 8),
    ("Transpose", 9),
    ("AlignAxes", 10),
    ("GapNormalization", 11),
    ("AlignAxes", 12),
    ("OptimizeClusters", 13),
    ("AlignAxes", 14),
    ("BalanceSymmetry", 15),
    ("Equidistance", 16),
    ("AlignAxes", 17),
    ("BinPack", 18),
    ("Rescale", 20),
];

/// Where TALA has the cards after these stages, which are the first of the pipeline after node
/// placement, in its order, and whether the last of them moved a card. A stage is recorded
/// where it moved one, with its number in the plan.
fn recorded<'a>(case: &'a Case, stages: &[&str]) -> (&'a Vec<[f64; 2]>, bool) {
    assert!(stages.iter().zip(STAGES).all(|(name, (stage, _))| *name == stage), "not the first stages: {stages:?}");
    let (name, number) = STAGES[stages.len() - 1];
    let last = case.moved.iter().take_while(|stage| stage.index <= number).last();
    assert!(last.is_none_or(|stage| stage.index != number || stage.stage == name));
    (last.map_or(&case.placed, |stage| &stage.at), last.is_some_and(|stage| stage.index == number))
}

#[test]
fn the_gap_stage_closes_the_gaps_tala_closes() {
    let cases: Vec<Case> = serde_json::from_str(include_str!("fixtures/stages.tala.json")).unwrap();
    let (mut moved, mut lined_up, mut differ) = (0, 0, Vec::new());
    for (number, case) in cases.iter().enumerate() {
        let what = format!("case {number} ({}, {} cards, seed {}, clusters {}, trees {}, hierarchies {})", case.name, case.nodes.len(), case.seed, case.clusters, case.trees, case.hierarchies);
        let mut whole = given(case);
        prescale(&mut whole);
        let found = find_trees(&whole);
        let mut placed = place_with_trees(&mut whole, &found, case.seed).expect("a place for every card");
        swap(&mut placed);
        transpose_all(&mut placed);
        align(&mut placed);
        let places = |placed: &tidy_layout::place::Placed| -> Vec<[f64; 2]> { placed.places(case.nodes.len()).iter().map(|place| [place.x, place.y]).collect() };
        let again = normalize_gaps(&mut placed);
        let stages = ["SwapStuff", "Transpose", "AlignAxes", "GapNormalization", "AlignAxes"];
        let (after, gaps_moved) = recorded(case, &stages[..4]);
        if places(&placed) != *after {
            differ.push(format!("{what} after the gaps"));
            continue;
        }
        // The stage that lines cards up runs again where the gap stage looked at a card.
        if again {
            align(&mut placed);
        }
        let (after, aligned) = recorded(case, &stages);
        if places(&placed) != *after {
            differ.push(format!("{what} after lining up again"));
        }
        moved += usize::from(gaps_moved);
        lined_up += usize::from(aligned);
    }
    assert_eq!(differ.len(), 0, "{} differ: {}", differ.len(), differ.join("; "));
    assert_eq!((moved, lined_up), (145, 23));
}

#[test]
fn the_cluster_stage_puts_clusters_right_as_tala_does() {
    let cases: Vec<Case> = serde_json::from_str(include_str!("fixtures/stages.tala.json")).unwrap();
    let (mut moved, mut lined_up, mut differ) = (0, 0, Vec::new());
    for (number, case) in cases.iter().enumerate() {
        let what = format!("case {number} ({}, {} cards, seed {}, clusters {}, trees {}, hierarchies {})", case.name, case.nodes.len(), case.seed, case.clusters, case.trees, case.hierarchies);
        let mut whole = given(case);
        prescale(&mut whole);
        let found = find_trees(&whole);
        let mut placed = place_with_trees(&mut whole, &found, case.seed).expect("a place for every card");
        swap(&mut placed);
        transpose_all(&mut placed);
        align(&mut placed);
        if normalize_gaps(&mut placed) {
            align(&mut placed);
        }
        let places = |placed: &tidy_layout::place::Placed| -> Vec<[f64; 2]> { placed.places(case.nodes.len()).iter().map(|place| [place.x, place.y]).collect() };
        let again = optimize_clusters(&mut placed);
        let stages = ["SwapStuff", "Transpose", "AlignAxes", "GapNormalization", "AlignAxes", "OptimizeClusters", "AlignAxes"];
        let (after, clusters_moved) = recorded(case, &stages[..6]);
        if places(&placed) != *after {
            differ.push(format!("{what} after the clusters"));
            continue;
        }
        if again {
            align(&mut placed);
        }
        let (after, aligned) = recorded(case, &stages);
        if places(&placed) != *after {
            differ.push(format!("{what} after lining up again"));
        }
        moved += usize::from(clusters_moved);
        lined_up += usize::from(aligned);
    }
    assert_eq!(differ.len(), 0, "{} differ: {}", differ.len(), differ.join("; "));
    assert_eq!((moved, lined_up), (29, 6));
}

/// The diagram as the editor gives it: cards and connections in their own order, the cards
/// somewhere (where, must not matter).
fn given(case: &Case) -> Graph {
    let nodes = case
        .nodes
        .iter()
        .enumerate()
        .map(|(card, [width, height])| Node { top_left: Point { x: 13.0 * card as f64 + 7.0, y: -31.0 }, width: *width, height: *height })
        .collect();
    let edges = case
        .edges
        .iter()
        .map(|[from, to, width, height]| Edge { from: *from, to: *to, min_width: *width as f64, min_height: *height as f64 })
        .collect();
    Graph::new(nodes, edges)
}

#[test]
fn the_tree_stage_finds_the_trees_tala_finds_and_leaves_its_order() {
    let cases: Vec<Case> = serde_json::from_str(include_str!("fixtures/stages.tala.json")).unwrap();
    let (mut with_trees, mut tree_cards, mut reordered) = (0, 0, 0);
    for (number, case) in cases.iter().enumerate() {
        let what = format!("case {number} ({}, {} cards)", case.name, case.nodes.len());
        let graph = given(case);
        let found = find_trees(&graph);
        assert_eq!(found.order, case.tree_order, "{what}: the cards left, in order");
        let left: Vec<[usize; 2]> = found.edges.iter().map(|edge| [graph.edges[*edge].from, graph.edges[*edge].to]).collect();
        assert_eq!(left, case.tree_edges, "{what}: the connections left, in order");

        fn as_recorded(graph: &Graph, tree: &Tree, cards: &mut usize) -> Value {
            *cards += 1;
            let mut list = vec![json!(tree.node), json!(graph.edges[tree.edge].from), json!(graph.edges[tree.edge].to)];
            list.extend(tree.children.iter().map(|child| as_recorded(graph, child, cards)));
            Value::Array(list)
        }
        let trees: BTreeMap<String, Value> = found
            .trees
            .iter()
            .map(|(sentinel, roots)| {
                (sentinel.to_string(), Value::Array(roots.iter().map(|root| as_recorded(&graph, root, &mut tree_cards)).collect()))
            })
            .collect();
        assert_eq!(trees, case.tree_roots, "{what}: the trees");
        with_trees += usize::from(!found.trees.is_empty());
        reordered += usize::from(found.trees.is_empty() && found.order != (0..case.nodes.len()).collect::<Vec<_>>());
    }
    assert_eq!((with_trees, tree_cards, reordered), (99, 515, 213));
}

#[test]
fn the_balance_and_equidistance_stages_move_the_cards_tala_moves() {
    let cases: Vec<Case> = serde_json::from_str(include_str!("fixtures/stages.tala.json")).unwrap();
    let (mut balanced, mut spread, mut lined_up, mut differ) = (0, 0, 0, Vec::new());
    for (number, case) in cases.iter().enumerate() {
        let what = format!("case {number} ({}, {} cards, seed {}, clusters {}, trees {}, hierarchies {})", case.name, case.nodes.len(), case.seed, case.clusters, case.trees, case.hierarchies);
        let mut whole = given(case);
        prescale(&mut whole);
        let found = find_trees(&whole);
        let mut placed = place_with_trees(&mut whole, &found, case.seed).expect("a place for every card");
        swap(&mut placed);
        transpose_all(&mut placed);
        align(&mut placed);
        if normalize_gaps(&mut placed) {
            align(&mut placed);
        }
        if optimize_clusters(&mut placed) {
            align(&mut placed);
        }
        let places = |placed: &tidy_layout::place::Placed| -> Vec<[f64; 2]> { placed.places(case.nodes.len()).iter().map(|place| [place.x, place.y]).collect() };
        let stages = ["SwapStuff", "Transpose", "AlignAxes", "GapNormalization", "AlignAxes", "OptimizeClusters", "AlignAxes", "BalanceSymmetry", "Equidistance", "AlignAxes"];
        balance_symmetry(&mut placed);
        let (after, moved) = recorded(case, &stages[..8]);
        balanced += usize::from(moved);
        if places(&placed) != *after {
            differ.push(format!("{what} after the balance"));
            continue;
        }
        let again = equidistance(&mut placed);
        let (after, moved) = recorded(case, &stages[..9]);
        spread += usize::from(moved);
        if places(&placed) != *after {
            differ.push(format!("{what} after the equidistance"));
            continue;
        }
        if again {
            align(&mut placed);
        }
        let (after, moved) = recorded(case, &stages);
        lined_up += usize::from(moved);
        if places(&placed) != *after {
            differ.push(format!("{what} after lining up a fourth time"));
        }
    }
    assert_eq!(differ.len(), 0, "{} differ: {}", differ.len(), differ.join("; "));
    assert_eq!((balanced, spread, lined_up), (28, 103, 26));
}

#[test]
fn the_packing_stage_packs_the_groups_as_tala_does() {
    let cases: Vec<Case> = serde_json::from_str(include_str!("fixtures/stages.tala.json")).unwrap();
    let (mut packed, mut padded, mut differ) = (0, 0, Vec::new());
    for (number, case) in cases.iter().enumerate() {
        let what = format!("case {number} ({}, {} cards, seed {}, clusters {}, trees {}, hierarchies {})", case.name, case.nodes.len(), case.seed, case.clusters, case.trees, case.hierarchies);
        let mut whole = given(case);
        prescale(&mut whole);
        let found = find_trees(&whole);
        let mut placed = place_with_trees(&mut whole, &found, case.seed).expect("a place for every card");
        swap(&mut placed);
        transpose_all(&mut placed);
        align(&mut placed);
        if normalize_gaps(&mut placed) {
            align(&mut placed);
        }
        if optimize_clusters(&mut placed) {
            align(&mut placed);
        }
        balance_symmetry(&mut placed);
        if equidistance(&mut placed) {
            align(&mut placed);
        }
        pack(&mut placed);
        let places = |placed: &tidy_layout::place::Placed| -> Vec<[f64; 2]> { placed.places(case.nodes.len()).iter().map(|place| [place.x, place.y]).collect() };
        let stages = ["SwapStuff", "Transpose", "AlignAxes", "GapNormalization", "AlignAxes", "OptimizeClusters", "AlignAxes", "BalanceSymmetry", "Equidistance", "AlignAxes", "BinPack", "Rescale"];
        let (after, moved) = recorded(case, &stages[..11]);
        packed += usize::from(moved);
        if places(&placed) != *after {
            differ.push(format!("{what} after the packing"));
            continue;
        }
        pad(&mut placed);
        let (after, moved) = recorded(case, &stages);
        padded += usize::from(moved);
        if places(&placed) != *after {
            differ.push(format!("{what} after the padding"));
        }
    }
    assert_eq!(differ.len(), 0, "{} differ: {}", differ.len(), differ.join("; "));
    assert_eq!((packed, padded), (105, 467));
}
