// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.

//! Compares the node placement stage with TALA on random diagrams (docs/reference.md, "Finding
//! cases"). Ours, not ported.
//!
//!   probe_stages write <file> <count> <seed> [most cards] [forest|layers|clusters]   make diagrams (questions only)
//!   probe_stages compare <file> <limit> [quick]             check TALA's answers; print those that differ
//!
//! `quick` leaves node placement out: the tree, the hierarchy and the cluster stage only.
//!
//! TALA answers the questions (docs/recorders/stages_test.go.txt, `TestStagesProbe`). `compare`
//! checks the tree, the hierarchy and the cluster stage and the places after node placement for
//! every diagram.

use tidy_layout::align::align;
use tidy_layout::balance::balance_symmetry;
use tidy_layout::equidistance::equidistance;
use tidy_layout::gaps::normalize_gaps;
use tidy_layout::cluster_stage::optimize_clusters;
use tidy_layout::clusters::{Arrangement, find_clusters, sizes as cluster_sizes, with_vessels};
use tidy_layout::graph::{Edge, Graph, Node, Point};
use tidy_layout::hierarchy::find_hierarchies;
use tidy_layout::pack::pack;
use tidy_layout::hierarchy_layout::place_hierarchies;
use tidy_layout::place::{Placed, pad, place_with_trees, prescale};
use tidy_layout::swap::{swap, transpose_all};
use tidy_layout::trees::{Tree, find_trees, without_trees};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use std::collections::BTreeMap;

#[derive(Serialize, Deserialize, Clone, Default)]
#[serde(rename_all = "camelCase", default)]
struct Case {
    name: String,
    /// width, height
    nodes: Vec<[f64; 2]>,
    /// from, to, and the room the label needs (0, 0 without a label)
    edges: Vec<[usize; 4]>,
    seed: i64,
    #[serde(skip_serializing)]
    trees: bool,
    #[serde(skip_serializing)]
    hierarchies: bool,
    #[serde(skip_serializing)]
    clusters: bool,
    #[serde(skip_serializing)]
    sequences: bool,
    #[serde(skip_serializing)]
    tree_order: Vec<usize>,
    #[serde(skip_serializing)]
    tree_edges: Vec<[usize; 2]>,
    #[serde(skip_serializing)]
    tree_roots: BTreeMap<String, Value>,
    #[serde(skip_serializing)]
    levels: Vec<i64>,
    #[serde(skip_serializing)]
    hierarchy_placed: Vec<[f64; 2]>,
    #[serde(skip_serializing)]
    cluster_list: Vec<Value>,
    #[serde(skip_serializing)]
    cluster_sizes: Vec<[f64; 2]>,
    #[serde(skip_serializing)]
    order: Vec<i64>,
    #[serde(skip_serializing)]
    sizes: Vec<[f64; 2]>,
    #[serde(skip_serializing)]
    edges_after: Vec<[i64; 2]>,
    #[serde(skip_serializing)]
    cell: f64,
    #[serde(skip_serializing)]
    placed: Vec<[f64; 2]>,
    #[serde(skip_serializing)]
    clusters_placed: Vec<Value>,
    #[serde(skip_serializing)]
    placed_sizes: Vec<[f64; 2]>,
    /// Every later stage before routing that moved a card: {stage, at}.
    #[serde(skip_serializing)]
    moved: Vec<Value>,
}

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

/// Cards as an architecture diagram has them: one or several groups, most connections labelled.
fn diagram(random: &mut Random, most: usize) -> Case {
    let mut case = Case { seed: 1 + random.below(3) as i64, ..Case::default() };
    let count = 1 + random.below(most);
    let uniform = random.below(4) > 0;
    for _ in 0..count {
        case.nodes.push(if uniform { [220.0, 120.0] } else { [(60 + 20 * random.below(14)) as f64, (60 + 20 * random.below(8)) as f64] });
    }
    let connect = |random: &mut Random, edges: &mut Vec<[usize; 4]>, a: usize, b: usize| {
        if a == b {
            return;
        }
        let mut edge = if random.below(2) == 0 { [b, a, 0, 0] } else { [a, b, 0, 0] };
        if random.below(3) > 0 {
            (edge[2], edge[3]) = (40 + random.below(180), 26);
        }
        edges.push(edge);
    };
    let mut edges = Vec::new();
    match random.below(5) {
        0 => {
            case.name = "tree".into();
            for n in 1..count {
                let parent = random.below(n);
                connect(random, &mut edges, parent, n);
            }
        }
        1 => {
            case.name = "chain".into();
            for n in 1..count {
                connect(random, &mut edges, n - 1, n);
            }
        }
        2 => {
            case.name = "groups".into();
            for n in 1..count {
                if random.below(3) > 0 {
                    let parent = random.below(n);
                    connect(random, &mut edges, parent, n);
                }
            }
        }
        3 => {
            case.name = "loose".into();
            for _ in 0..random.below(count + 1) {
                let (a, b) = (random.below(count), random.below(count));
                connect(random, &mut edges, a, b);
            }
        }
        _ => {
            case.name = "dense".into();
            for n in 1..count {
                let parent = random.below(n);
                connect(random, &mut edges, parent, n);
            }
            for _ in 0..1 + count / 2 {
                let (a, b) = (random.below(count), random.below(count));
                connect(random, &mut edges, a, b);
            }
        }
    }
    case.edges = edges;
    case
}

/// One to three cards with large trees on them, wide and deep, the arrows of a branch mostly
/// one way: what `diagram` seldom makes, and what laying trees out needs.
fn forest(random: &mut Random, most: usize) -> Case {
    let mut case = Case { name: "forest".into(), seed: 1 + random.below(3) as i64, ..Case::default() };
    let count = 2 + random.below(most.max(3) - 1);
    let uniform = random.below(3) > 0;
    for _ in 0..count {
        case.nodes.push(if uniform { [220.0, 120.0] } else { [(60 + 20 * random.below(14)) as f64, (60 + 20 * random.below(8)) as f64] });
    }
    let label = |random: &mut Random| if random.below(3) == 0 { [40 + random.below(180), 26] } else { [0, 0] };
    // The cards the trees hang on: one alone, or two or three in a ring, which is no tree.
    let core = (1 + random.below(3)).min(count);
    for card in 0..core {
        if core > 1 {
            let [width, height] = label(random);
            case.edges.push([card, (card + 1) % core, width, height]);
        }
    }
    let mut outwards = vec![false; count];
    for card in core..count {
        // Under one of the last cards for depth, under any card for width, or on a card of the
        // ring, so that it carries several trees.
        let parent = match random.below(4) {
            0 | 1 => card - 1 - random.below(card.min(3)),
            2 => random.below(card),
            _ => random.below(core),
        };
        outwards[card] = if parent < core || random.below(10) == 0 { random.below(2) == 0 } else { outwards[parent] };
        let [width, height] = label(random);
        case.edges.push(if outwards[card] { [parent, card, width, height] } else { [card, parent, width, height] });
    }
    case
}

/// Cards in three to six rows with arrows mostly down, over one row or several, some up, some
/// doubled: what TALA takes for a hierarchy, with work left for its ranking.
fn layers(random: &mut Random, most: usize) -> Case {
    let mut case = Case { name: "layers".into(), seed: 1 + random.below(3) as i64, ..Case::default() };
    let rows = 3 + random.below(4);
    let count = (rows * rows / 2 + random.below(rows * rows * 2)).clamp(rows, most.max(rows));
    let uniform = random.below(3) > 0;
    for _ in 0..count {
        case.nodes.push(if uniform { [220.0, 120.0] } else { [(60 + 20 * random.below(14)) as f64, (60 + 20 * random.below(8)) as f64] });
    }
    // Every row has a card; the others go anywhere.
    let row: Vec<usize> = (0..count).map(|card| if card < rows { card } else { random.below(rows) }).collect();
    let label = |random: &mut Random| if random.below(3) == 0 { [40 + random.below(180), 26] } else { [0, 0] };
    for card in 0..count {
        if row[card] == 0 {
            continue;
        }
        // An arrow from a card further up: mostly from the row above.
        let up_to = if random.below(3) > 0 { row[card] - 1 } else { random.below(row[card]) };
        let above: Vec<usize> = (0..count).filter(|other| row[*other] == up_to).collect();
        let [width, height] = label(random);
        case.edges.push([above[random.below(above.len())], card, width, height]);
    }
    for _ in 0..random.below(count / 2 + 1) {
        let (a, b) = (random.below(count), random.below(count));
        let [width, height] = label(random);
        match random.below(10) {
            // A connection once more, the same way.
            0 => {
                let again = case.edges[random.below(case.edges.len())];
                case.edges.push([again[0], again[1], width, height]);
            }
            _ if a == b => {}
            // Mostly down, sometimes up; on one row either way.
            kind => {
                let down = (row[a] < row[b]) == (kind > 2);
                case.edges.push(if down { [a, b, width, height] } else { [b, a, width, height] });
            }
        }
    }
    case
}

/// A few cards with groups of like cards on them: cards of one size connected to the same
/// one or two cards the same way, which TALA takes for clusters. Some loose cards and
/// connections besides, so that clusters meet other routes.
fn clusters(random: &mut Random, most: usize) -> Case {
    let mut case = Case { name: "clusters".into(), seed: 1 + random.below(3) as i64, ..Case::default() };
    let label = |random: &mut Random| if random.below(3) == 0 { [40 + random.below(180), 26] } else { [0, 0] };
    let size = |random: &mut Random| if random.below(2) == 0 { [220.0, 120.0] } else { [(60 + 20 * random.below(14)) as f64, (60 + 20 * random.below(8)) as f64] };
    let core = 1 + random.below(4);
    for _ in 0..core {
        let card = size(random);
        case.nodes.push(card);
    }
    for card in 1..core {
        if random.below(4) > 0 {
            let [width, height] = label(random);
            case.edges.push([random.below(card), card, width, height]);
        }
    }
    let groups = 1 + random.below(3);
    for _ in 0..groups {
        if case.nodes.len() + 2 > most.max(4) {
            break;
        }
        let members = (2 + random.below(5)).min(most.max(4) - case.nodes.len());
        // One card the group hangs on, or two.
        let first = random.below(case.nodes.len());
        let second = (random.below(3) == 0).then(|| random.below(case.nodes.len())).filter(|second| *second != first);
        let (out_first, out_second) = (random.below(2) == 0, random.below(2) == 0);
        let card = size(random);
        for _ in 0..members {
            let at = case.nodes.len();
            case.nodes.push(card);
            for (other, outwards) in [(Some(first), out_first), (second, out_second)] {
                if let Some(other) = other {
                    let [width, height] = label(random);
                    case.edges.push(if outwards { [other, at, width, height] } else { [at, other, width, height] });
                }
            }
        }
    }
    let loose = random.below(3);
    for _ in 0..loose {
        if case.nodes.len() >= most.max(4) {
            break;
        }
        let card = size(random);
        case.nodes.push(card);
    }
    let count = case.nodes.len();
    for _ in 0..random.below(3) {
        let (a, b) = (random.below(count), random.below(count));
        if a != b {
            let [width, height] = label(random);
            case.edges.push([a, b, width, height]);
        }
    }
    case
}

fn given(case: &Case) -> Graph {
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

/// Does the port's tree stage leave what TALA's leaves: the cards and connections in the graph,
/// in order, and the trees?
fn same_trees(case: &Case) -> bool {
    let graph = given(case);
    let found = find_trees(&graph);
    fn as_recorded(graph: &Graph, tree: &Tree) -> Value {
        let mut list = vec![json!(tree.node), json!(graph.edges[tree.edge].from), json!(graph.edges[tree.edge].to)];
        list.extend(tree.children.iter().map(|child| as_recorded(graph, child)));
        Value::Array(list)
    }
    let trees: BTreeMap<String, Value> = found
        .trees
        .iter()
        .map(|(sentinel, roots)| (sentinel.to_string(), Value::Array(roots.iter().map(|root| as_recorded(&graph, root)).collect())))
        .collect();
    let left: Vec<[usize; 2]> = found.edges.iter().map(|edge| [graph.edges[*edge].from, graph.edges[*edge].to]).collect();
    found.order == case.tree_order && left == case.tree_edges && trees == case.tree_roots
}

/// Does the port's hierarchy stage give every card the level TALA gives it, and the cards of a
/// hierarchy their places?
fn same_levels(case: &Case) -> bool {
    let mut whole = given(case);
    prescale(&mut whole);
    let found = find_trees(&whole);
    let core = without_trees(&whole, &found);
    let found_levels = find_hierarchies(&core);
    let mut levels = vec![-1; case.nodes.len()];
    let mut placed = vec![[0.0, 0.0]; case.nodes.len()];
    for ((card, level), place) in found.order.iter().zip(&found_levels).zip(place_hierarchies(&core, &found_levels, case.seed)) {
        levels[*card] = level.map_or(-1, |level| level as i64);
        placed[*card] = place.map_or([0.0, 0.0], |at| [at.x, at.y]);
    }
    levels == case.levels && placed == case.hierarchy_placed
}

/// Does the port's cluster stage make the clusters TALA makes, and leave node placement the
/// graph TALA leaves it: the cards and vessels in order, their sizes, the connections?
fn same_clusters(case: &Case) -> bool {
    let mut whole = given(case);
    prescale(&mut whole);
    let found = find_trees(&whole);
    let core = without_trees(&whole, &found);
    let list = find_clusters(&core, &find_hierarchies(&core), case.nodes.len(), case.seed);
    let mut sizes: Vec<[f64; 2]> = whole.nodes.iter().map(|card| [card.width, card.height]).collect();
    let recorded: Vec<Value> = list
        .iter()
        .map(|cluster| {
            let ((width, height), vessel) = cluster_sizes(&core, cluster);
            for member in &cluster.members {
                sizes[found.order[*member]] = [width, height];
            }
            json!({
                "vessel": cluster.vessel,
                "nodes": cluster.members.iter().map(|member| found.order[*member]).collect::<Vec<_>>(),
                "arrangement": if cluster.arrangement == Arrangement::Row { "Row" } else { "Column" },
                "padding": cluster.padding,
                "at": [0.0, 0.0],
                "size": [vessel.0, vessel.1],
            })
        })
        .collect();
    let (graph, _) = with_vessels(&core, &list);
    let now: Vec<[f64; 2]> = graph.nodes.iter().map(|card| [card.width, card.height]).collect();
    let ends: Vec<[i64; 2]> = graph.edges.iter().map(|edge| [graph.id(edge.from), graph.id(edge.to)]).collect();
    // TALA's numbers are whole; a recorded 20 and the port's 20.0 are the same padding.
    let same_list = recorded.len() == case.cluster_list.len()
        && recorded.iter().zip(&case.cluster_list).all(|(ours, theirs)| {
            ["vessel", "nodes", "arrangement"].iter().all(|key| ours[key] == theirs[key])
                && ours["padding"].as_f64() == theirs["padding"].as_f64()
                && (0..2).all(|side| ours["size"][side].as_f64() == theirs["size"][side].as_f64())
        });
    same_list && sizes == case.cluster_sizes && graph.ids == case.order && now == case.sizes && ends == case.edges_after
}

/// The port's places for a diagram, through TALA's stages in their order, by card; and the sizes
/// of the cards the tree stage leaves, in its order.
fn placed(case: &Case) -> (Vec<[f64; 2]>, Vec<[f64; 2]>, f64) {
    let mut whole = given(case);
    prescale(&mut whole);
    let found = find_trees(&whole);
    let sizes = without_trees(&whole, &found).nodes.iter().map(|card| [card.width, card.height]).collect();
    place_with_trees(&mut whole, &found, case.seed).expect("a place for every card");
    let at = whole.nodes.iter().map(|card| [card.top_left.x, card.top_left.y]).collect();
    (at, sizes, whole.cell_size)
}

/// With clusters: does the port give every card TALA's place and size after node placement, and
/// every cluster TALA's arrangement?
fn same_with_clusters(case: &Case) -> bool {
    let mut whole = given(case);
    prescale(&mut whole);
    let found = find_trees(&whole);
    let Ok(placed) = place_with_trees(&mut whole, &found, case.seed) else {
        return false;
    };
    let clusters = placed.clusters;
    let at: Vec<[f64; 2]> = whole.nodes.iter().map(|card| [card.top_left.x, card.top_left.y]).collect();
    let sizes: Vec<[f64; 2]> = whole.nodes.iter().map(|card| [card.width, card.height]).collect();
    let arrangements = clusters.iter().zip(&case.clusters_placed).all(|(ours, theirs)| {
        theirs["arrangement"] == if ours.arrangement == Arrangement::Row { "Row" } else { "Column" } && theirs["padding"].as_f64() == Some(ours.padding)
    });
    let same = at == case.placed && sizes == case.placed_sizes && whole.cell_size == case.cell && arrangements && clusters.len() == case.clusters_placed.len();
    if !same && std::env::var("WHICH").is_ok_and(|which| which == "2") {
        eprintln!("after node placement: ours {at:?} sizes {sizes:?} cell {} arrangements {arrangements}\n  TALA {:?} sizes {:?} cell {}", whole.cell_size, case.placed, case.placed_sizes, case.cell);
    }
    same
}

/// The number of each of `STAGES` in TALA's plan.
const PLAN: [u64; 12] = [8, 9, 10, 11, 12, 13, 14, 15, 16, 17, 18, 20];

/// Where TALA has the cards after these stages, which are the first of the pipeline after
/// node placement, in its order. A stage is recorded where it moved a card, with its number
/// in the plan; before the first there are the places of node placement. Also whether the
/// last of them moved one.
fn recorded(case: &Case, stages: &[&str]) -> (Vec<[f64; 2]>, bool) {
    let number = PLAN[stages.len() - 1];
    let index = |stage: &Value| stage["index"].as_u64().expect("an answer file of a recorder that numbers the stages");
    let last = case.moved.iter().take_while(|stage| index(stage) <= number).last();
    let at = last.and_then(|stage| serde_json::from_value(stage["at"].clone()).ok()).unwrap_or_else(|| case.placed.clone());
    (at, last.is_some_and(|stage| index(stage) == number))
}

/// The stages after node placement that are ported, in TALA's order.
const STAGES: [&str; 12] = ["SwapStuff", "Transpose", "AlignAxes", "GapNormalization", "AlignAxes", "OptimizeClusters", "AlignAxes", "BalanceSymmetry", "Equidistance", "AlignAxes", "BinPack", "Rescale"];

/// Does the port leave every card where TALA leaves it after each of the stages?
fn same_after_stages(case: &Case) -> [bool; 12] {
    let mut whole = given(case);
    prescale(&mut whole);
    let found = find_trees(&whole);
    let Ok(mut placed) = place_with_trees(&mut whole, &found, case.seed) else {
        return [false; 12];
    };
    let places = |placed: &Placed| -> Vec<[f64; 2]> { placed.places(case.nodes.len()).iter().map(|place| [place.x, place.y]).collect() };
    let mut same = [false; 12];
    // `WHICH=2` prints where the cards are, the port's and TALA's, after the first stage that differs.
    let show = |stage: usize, ours: &[[f64; 2]], theirs: &[[f64; 2]], same: &[bool; 12]| {
        if std::env::var("WHICH").is_ok_and(|which| which == "2") && ours != theirs && same[..stage].iter().all(|same| *same) {
            eprintln!("after {}: ours {:?}\n  TALA {:?}", STAGES[stage], ours, theirs);
        }
    };
    swap(&mut placed);
    same[0] = places(&placed) == recorded(case, &STAGES[..1]).0;
    show(0, &places(&placed), &recorded(case, &STAGES[..1]).0, &same);
    transpose_all(&mut placed);
    same[1] = places(&placed) == recorded(case, &STAGES[..2]).0;
    show(1, &places(&placed), &recorded(case, &STAGES[..2]).0, &same);
    align(&mut placed);
    same[2] = places(&placed) == recorded(case, &STAGES[..3]).0;
    show(2, &places(&placed), &recorded(case, &STAGES[..3]).0, &same);
    let again = normalize_gaps(&mut placed);
    same[3] = places(&placed) == recorded(case, &STAGES[..4]).0;
    show(3, &places(&placed), &recorded(case, &STAGES[..4]).0, &same);
    // The stage that lines cards up runs again where the gap stage looked at a card.
    if again {
        align(&mut placed);
    }
    same[4] = places(&placed) == recorded(case, &STAGES[..5]).0;
    show(4, &places(&placed), &recorded(case, &STAGES[..5]).0, &same);
    let again = optimize_clusters(&mut placed);
    same[5] = places(&placed) == recorded(case, &STAGES[..6]).0;
    show(5, &places(&placed), &recorded(case, &STAGES[..6]).0, &same);
    // And again where a cluster changed.
    if again {
        align(&mut placed);
    }
    same[6] = places(&placed) == recorded(case, &STAGES[..7]).0;
    show(6, &places(&placed), &recorded(case, &STAGES[..7]).0, &same);
    balance_symmetry(&mut placed);
    same[7] = places(&placed) == recorded(case, &STAGES[..8]).0;
    show(7, &places(&placed), &recorded(case, &STAGES[..8]).0, &same);
    let again = equidistance(&mut placed);
    same[8] = places(&placed) == recorded(case, &STAGES[..9]).0;
    show(8, &places(&placed), &recorded(case, &STAGES[..9]).0, &same);
    // And again where a card was put halfway between two others.
    if again {
        align(&mut placed);
    }
    same[9] = places(&placed) == recorded(case, &STAGES[..10]).0;
    show(9, &places(&placed), &recorded(case, &STAGES[..10]).0, &same);
    pack(&mut placed);
    same[10] = places(&placed) == recorded(case, &STAGES[..11]).0;
    show(10, &places(&placed), &recorded(case, &STAGES[..11]).0, &same);
    pad(&mut placed);
    same[11] = places(&placed) == recorded(case, &STAGES).0;
    same
}

fn main() {
    let arguments: Vec<String> = std::env::args().collect();
    match arguments.get(1).map(String::as_str) {
        Some("write") if (5..=7).contains(&arguments.len()) => {
            let count: usize = arguments[3].parse().expect("count");
            let mut random = Random(arguments[4].parse::<u64>().expect("seed").max(1));
            let most = arguments.get(5).map_or(12, |most| most.parse().expect("most cards"));
            let make = match arguments.get(6).map(String::as_str) {
                Some("forest") => forest,
                Some("layers") => layers,
                Some("clusters") => clusters,
                _ => diagram,
            };
            let cases: Vec<Case> = (0..count).map(|_| make(&mut random, most)).collect();
            std::fs::write(&arguments[2], serde_json::to_string(&cases).unwrap()).unwrap();
        }
        Some("compare") if arguments.len() == 4 || arguments.len() == 5 => {
            let quick = arguments.get(4).is_some_and(|mode| mode == "quick");
            let limit: usize = arguments[3].parse().expect("limit");
            let cases: Vec<Case> = serde_json::from_str(&std::fs::read_to_string(&arguments[2]).unwrap()).unwrap();
            let plain: Vec<&Case> = cases.iter().filter(|case| !(case.clusters || case.sequences)).collect();
            let other_trees: Vec<&Case> = cases.iter().filter(|case| !same_trees(case)).collect();
            let other_levels: Vec<&Case> = cases.iter().filter(|case| !same_levels(case)).collect();
            let other_clusters: Vec<&Case> = cases.iter().filter(|case| !same_clusters(case)).collect();
            let all_clusters = || cases.iter().flat_map(|case| &case.cluster_list);
            eprintln!(
                "{} diagrams: the cluster stage differs in {} ({} with a cluster, {} clusters, {} in a row, {} of more than two cards)",
                cases.len(),
                other_clusters.len(),
                cases.iter().filter(|case| case.clusters).count(),
                all_clusters().count(),
                all_clusters().filter(|cluster| cluster["arrangement"] == "Row").count(),
                all_clusters().filter(|cluster| cluster["nodes"].as_array().is_some_and(|nodes| nodes.len() > 2)).count(),
            );
            let mut different: Vec<&Case> =
                plain.iter().copied().filter(|case| !quick && placed(case) != (case.placed.clone(), case.sizes.clone(), case.cell)).collect();
            let with_clusters: Vec<&Case> = cases.iter().filter(|case| case.clusters && !case.sequences).collect();
            let other_places: Vec<&Case> = with_clusters.iter().copied().filter(|case| !quick && !same_with_clusters(case)).collect();
            eprintln!(
                "{} with a cluster ({} with trees too, {} with a hierarchy too, {} turned in node placement), {} of those differ after node placement{}",
                with_clusters.len(),
                with_clusters.iter().filter(|case| case.trees).count(),
                with_clusters.iter().filter(|case| case.hierarchies).count(),
                with_clusters.iter().filter(|case| case.cluster_list.iter().zip(&case.clusters_placed).any(|(was, is)| was["arrangement"] != is["arrangement"])).count(),
                other_places.len(),
                if quick { " (not asked: quick)" } else { "" }
            );
            let after: Vec<(&Case, [bool; 12])> =
                cases.iter().filter(|case| !quick && !case.sequences).map(|case| (case, same_after_stages(case))).collect();
            let other_swaps: Vec<&Case> = after.iter().filter(|(_, same)| same.contains(&false)).map(|(case, _)| *case).collect();
            let per_stage: Vec<String> = (0..STAGES.len())
                .map(|stage| {
                    format!(
                        "{} moves a card in {}, {} differ after it",
                        match stage {
                            4 => "AlignAxes again",
                            6 => "AlignAxes a third time",
                            9 => "AlignAxes a fourth time",
                            _ => STAGES[stage],
                        },
                        cases.iter().filter(|case| recorded(case, &STAGES[..=stage]).1).count(),
                        after.iter().filter(|(_, same)| !same[stage]).count()
                    )
                })
                .collect();
            eprintln!("{} diagrams: {}{}", cases.len(), per_stage.join("; "), if quick { " (not asked: quick)" } else { "" });
            eprintln!(
                "{} diagrams: the tree stage differs in {}, the hierarchy stage in {} ({} with a hierarchy, {} cards in one); {} without cluster and sequence ({} cards; {} with trees), {} of those differ after node placement{}",
                cases.len(),
                other_trees.len(),
                other_levels.len(),
                cases.iter().filter(|case| case.hierarchies).count(),
                cases.iter().flat_map(|case| &case.levels).filter(|level| **level >= 0).count(),
                plain.len(),
                plain.iter().map(|case| case.nodes.len()).sum::<usize>(),
                plain.iter().filter(|case| case.trees).count(),
                different.len(),
                if quick { " (not asked: quick)" } else { "" }
            );
            // `WHICH=1` names each diagram that differs and the first stage it differs after.
            if std::env::var("WHICH").is_ok() {
                for case in &other_places {
                    eprintln!("{} cards, {} connections, seed {}: differs after node placement", case.nodes.len(), case.edges.len(), case.seed);
                }
                for (case, same) in after.iter().filter(|(_, same)| same.contains(&false)) {
                    let first = same.iter().position(|same| !same).expect("one differs");
                    eprintln!("{} cards, {} connections, seed {}: differs first after stage {} ({})", case.nodes.len(), case.edges.len(), case.seed, first, STAGES[first]);
                }
            }
            different.extend(other_trees);
            different.extend(other_levels);
            different.extend(other_clusters);
            different.extend(other_places);
            different.extend(other_swaps);
            let shown: Vec<&&Case> = different.iter().take(limit).collect();
            println!("{}", serde_json::to_string(&shown).unwrap());
            std::process::exit(i32::from(!different.is_empty()));
        }
        _ => {
            eprintln!("usage: probe_stages write <file> <count> <seed> [most cards] [forest|layers|clusters] | compare <file> <limit> [quick]");
            std::process::exit(2);
        }
    }
}
