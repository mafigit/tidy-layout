// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.

//! Compares node placement with TALA, or with a changed copy of the port, on random groups of
//! connected cards (docs/reference.md, "Finding cases"). Ours, not ported.
//!
//!   probe_places write <file> <count> <seed> [most cards]   make groups, keep the port's answers
//!   probe_places compare <file> <limit>                     answer them again; print those that differ
//!   probe_places trace <file>                               the same, and where the first one differs
//!
//! The file can be given to TALA's probe (docs/recorders/place_test.go.txt, `TestPlaceProbe`),
//! which writes its own answers in the same form: `compare` on that file then checks the port
//! against TALA. With `"trace": true` in a layout both sides keep a number for the places after
//! every step of the two loops, and `trace` names the first step that differs.

use tidy_layout::clusters::{find_clusters, with_vessels};
use tidy_layout::gorand::GoRand;
use tidy_layout::graph::{Edge, Graph, Node, Point};
use tidy_layout::node_placement::{Stage, place_nodes_orthogonally};
use serde::{Deserialize, Serialize};

#[derive(Serialize, Deserialize, Clone, PartialEq, Default, Debug)]
struct Snapshot {
    at: Vec<[f64; 2]>,
    drawn: u64,
}

#[derive(Serialize, Deserialize, Clone, PartialEq, Default)]
#[serde(rename_all = "camelCase", default)]
struct Case {
    name: String,
    /// width, height
    nodes: Vec<[f64; 2]>,
    /// from, to, and the room the label needs (0, 0 without a label)
    edges: Vec<[usize; 4]>,
    seed: i64,
    /// TALA's number of each card, where it is not its place in `nodes`.
    #[serde(skip_serializing_if = "Vec::is_empty")]
    ids: Vec<i64>,
    #[serde(skip_serializing_if = "std::ops::Not::not")]
    trace: bool,
    /// Run the cluster stage first: the places are those of the cards left, then of the vessels.
    #[serde(skip_serializing_if = "std::ops::Not::not")]
    clusters: bool,
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
    /// With trace: per step the numbers drawn so far and a number made of all places.
    #[serde(skip_serializing_if = "Vec::is_empty")]
    steps: Vec<[i64; 2]>,
}

/// xorshift64: enough randomness to make groups of cards.
struct Random(u64);

impl Random {
    fn below(&mut self, limit: usize) -> usize {
        self.0 ^= self.0 << 13;
        self.0 ^= self.0 >> 7;
        self.0 ^= self.0 << 17;
        (self.0 % limit as u64) as usize
    }
}

/// One connected group of cards: a tree, a chain, stars or a ring, with a few more connections
/// now and then.
fn group(random: &mut Random, most: usize) -> Case {
    let mut case = Case { seed: 1 + random.below(1000) as i64, ..Case::default() };
    let count = 2 + random.below(most - 1);
    let uniform = random.below(3) > 0;
    let odd = random.below(6) == 0;
    for _ in 0..count {
        let mut size = if uniform { [220.0, 120.0] } else { [(60 + 20 * random.below(14)) as f64, (60 + 20 * random.below(8)) as f64] };
        if odd {
            size = [size[0] + random.below(4) as f64, size[1] + random.below(4) as f64];
        }
        case.nodes.push(size);
    }
    let connect = |random: &mut Random, edges: &mut Vec<[usize; 4]>, a: usize, b: usize| {
        let mut edge = if random.below(2) == 0 { [b, a, 0, 0] } else { [a, b, 0, 0] };
        if random.below(4) == 0 {
            (edge[2], edge[3]) = (30 + random.below(200), 20 + random.below(30));
        }
        edges.push(edge);
        if random.below(12) == 0 {
            for _ in 0..1 + random.below(3) {
                let mut more = edge;
                if random.below(2) == 0 {
                    (more[2], more[3]) = (30 + random.below(200), 20 + random.below(30));
                }
                edges.push(more);
            }
        }
    };
    let mut edges = Vec::new();
    match random.below(4) {
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
            case.name = "stars".into();
            let centres = (1 + random.below(2)).min(count - 1);
            for n in centres..count {
                let centre = random.below(centres);
                connect(random, &mut edges, centre, n);
            }
            if centres > 1 {
                connect(random, &mut edges, 0, 1);
            }
        }
        _ => {
            case.name = "ring".into();
            for n in 1..count {
                connect(random, &mut edges, n - 1, n);
            }
            if count > 2 {
                connect(random, &mut edges, count - 1, 0);
            }
        }
    }
    if count > 2 {
        for _ in 0..random.below(3) {
            let (a, b) = (random.below(count), random.below(count));
            if a != b {
                connect(random, &mut edges, a, b);
            }
        }
    }
    case.edges = edges;
    case
}

/// The port's answers for a group.
fn answered(case: &Case) -> Case {
    let mut answer = Case {
        name: case.name.clone(),
        nodes: case.nodes.clone(),
        edges: case.edges.clone(),
        seed: case.seed,
        ids: case.ids.clone(),
        trace: case.trace,
        clusters: case.clusters,
        ..Case::default()
    };
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
    let mut graph = Graph { ids: case.ids.clone(), ..Graph::new(nodes, edges) };
    if case.clusters {
        let found = find_clusters(&graph, &vec![None; graph.nodes.len()], graph.nodes.len(), case.seed);
        graph = with_vessels(&graph, &found).0;
        for card in &mut graph.nodes {
            card.top_left = Point { x: 0.0, y: 0.0 };
        }
    }
    answer.cell = graph.cell_size;
    let mut random = GoRand::new(case.seed);
    // TALA's probe stops at an error; no group made here causes one.
    place_nodes_orthogonally(&mut graph, case.seed, &mut random, |stage, graph, random| {
        let now = Snapshot { at: graph.nodes.iter().map(|card| [card.top_left.x, card.top_left.y]).collect(), drawn: random.drawn };
        match stage {
            Stage::First(by_distance) => (answer.by_distance, answer.first) = (by_distance, now),
            Stage::Sizeless => answer.sizeless = now,
            Stage::Inflated => answer.inflated = now,
            Stage::OnGrid(turn) => (answer.turn_cost, answer.on_grid) = (turn, now),
            Stage::Sized => answer.sized = now,
            Stage::Final => answer.last = now,
            Stage::SizelessStep | Stage::SizedStep | Stage::ColdStep => {
                if case.trace {
                    let sum = now.at.iter().enumerate().fold(0_i64, |sum, (i, at)| {
                        sum.wrapping_add((i as i64 + 1).wrapping_mul((at[0] as i64).wrapping_mul(1000003).wrapping_add(at[1] as i64)))
                    });
                    answer.steps.push([now.drawn as i64, sum]);
                }
            }
        }
    })
    .expect("a place for every card");
    answer
}

fn read(file: &str) -> Vec<Case> {
    serde_json::from_str(&std::fs::read_to_string(file).unwrap()).unwrap()
}

fn main() {
    let arguments: Vec<String> = std::env::args().collect();
    match arguments.get(1).map(String::as_str) {
        Some("write") if arguments.len() == 5 || arguments.len() == 6 => {
            let count: usize = arguments[3].parse().expect("count");
            let mut random = Random(arguments[4].parse::<u64>().expect("seed").max(1));
            let most = arguments.get(5).map_or(10, |most| most.parse().expect("most cards"));
            let cases: Vec<Case> = (0..count).map(|_| answered(&group(&mut random, most))).collect();
            std::fs::write(&arguments[2], serde_json::to_string(&cases).unwrap()).unwrap();
        }
        Some("compare") if arguments.len() == 4 => {
            let limit: usize = arguments[3].parse().expect("limit");
            let cases = read(&arguments[2]);
            let different: Vec<Case> = cases.iter().filter(|case| answered(case) != **case).cloned().collect();
            eprintln!(
                "{} groups, {} cards, {} arranged by graph distance first, {} random numbers, {} groups differ",
                cases.len(),
                cases.iter().map(|case| case.nodes.len()).sum::<usize>(),
                cases.iter().filter(|case| case.by_distance).count(),
                cases.iter().map(|case| case.last.drawn).sum::<u64>(),
                different.len()
            );
            let shown: Vec<&Case> = different.iter().take(limit).collect();
            println!("{}", serde_json::to_string(&shown).unwrap());
            std::process::exit(i32::from(!different.is_empty()));
        }
        Some("trace") if arguments.len() == 3 => {
            for (number, case) in read(&arguments[2]).iter().enumerate() {
                let ours = answered(case);
                let stages = [
                    ("first", &ours.first, &case.first),
                    ("sizeless", &ours.sizeless, &case.sizeless),
                    ("inflated", &ours.inflated, &case.inflated),
                    ("on the grid", &ours.on_grid, &case.on_grid),
                    ("sized", &ours.sized, &case.sized),
                    ("final", &ours.last, &case.last),
                ];
                let Some((stage, port, tala)) = stages.iter().find(|(_, port, tala)| port != tala) else {
                    continue;
                };
                println!("group {number} ({} cards, seed {}) differs at stage: {stage}\n port {port:?}\n TALA {tala:?}", case.nodes.len(), case.seed);
                if let Some(step) = ours.steps.iter().zip(&case.steps).position(|(port, tala)| port != tala) {
                    println!(" first step that differs: {step} of {} (port {:?}, TALA {:?}); the step before: {:?}", case.steps.len(), ours.steps[step], case.steps[step], step.checked_sub(1).map(|before| case.steps[before]));
                }
                return;
            }
            println!("no group differs");
        }
        _ => {
            eprintln!("usage: probe_places write <file> <count> <seed> [most cards] | compare <file> <limit> | trace <file>");
            std::process::exit(2);
        }
    }
}
