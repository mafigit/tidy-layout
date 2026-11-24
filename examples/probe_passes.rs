// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.

//! Compares one pass of the optimizer with TALA, or with a changed copy of the port, on
//! thousands of random layouts (docs/reference.md, "Finding cases"). Ours, not ported.
//!
//!   probe_passes write <file> <count> <seed>   make random layouts, keep the port's answers
//!   probe_passes compare <file> <limit>        answer them again; print the layouts that differ
//!
//! The file can be given to TALA's probe (docs/recorders/pass_test.go.txt, `TestPassProbe`),
//! which writes its own answers in the same form: `compare` on that file then checks the port
//! against TALA.

use tidy_layout::gorand::GoRand;
use tidy_layout::graph::{Edge, Graph, Node, Point, compute_cell_size};
use tidy_layout::optimizer::{best_swap_candidate, hubs, median_point, optimize, transpose};
use tidy_layout::routing::router::Costs;
use serde::{Deserialize, Serialize};

#[derive(Serialize, Deserialize, Clone, PartialEq, Default)]
struct Median {
    point: [f64; 2],
    drawn: u64,
}

#[derive(Serialize, Deserialize, Clone, PartialEq, Default)]
struct Swap {
    with: i64,
    drawn: u64,
}

#[derive(Serialize, Deserialize, Clone, PartialEq, Default)]
#[serde(rename_all = "camelCase", default)]
struct Case {
    name: String,
    /// 0: the cell size TALA computes from the card sizes.
    cell_size: f64,
    nodes: Vec<[f64; 4]>,
    /// from, to, and the room the label needs (0, 0 without a label)
    edges: Vec<[usize; 4]>,
    temp: f64,
    seed: i64,
    halve: bool,
    cell: f64,
    turn_cost: f64,
    hubs: Vec<Option<Vec<usize>>>,
    medians: Vec<Option<Median>>,
    swaps: Vec<Swap>,
    transposes: Vec<Option<Vec<[f64; 2]>>>,
    changed: bool,
    after: Vec<[f64; 2]>,
    drawn: u64,
}

/// xorshift64: enough randomness to scatter cards.
struct Random(u64);

impl Random {
    fn below(&mut self, limit: usize) -> usize {
        self.0 ^= self.0 << 13;
        self.0 ^= self.0 >> 7;
        self.0 ^= self.0 << 17;
        (self.0 % limit as u64) as usize
    }
}

/// Cards as the optimizer meets them: on the grid of the cell size, most of them clear of each
/// other, connected as a tree, a chain, stars or at random.
fn layout(random: &mut Random) -> Case {
    let mut case = Case { seed: 1 + random.below(1_000_000) as i64, ..Case::default() };
    let count = 2 + random.below(13);
    let uniform = random.below(3) > 0;
    let sizes: Vec<(f64, f64)> = (0..count)
        .map(|_| if uniform { (220.0, 120.0) } else { ((60 + 20 * random.below(14)) as f64, (60 + 20 * random.below(8)) as f64) })
        .collect();
    // Now and then sizes in odd pixels: the middle of such a card lies between two pixels.
    let mut sizes = sizes;
    if random.below(5) == 0 {
        for size in &mut sizes {
            size.0 += random.below(4) as f64;
            size.1 += random.below(4) as f64;
        }
    }
    let cards: Vec<Node> =
        sizes.iter().map(|(width, height)| Node { top_left: Point { x: 0.0, y: 0.0 }, width: *width, height: *height }).collect();
    let mut cell = compute_cell_size(&cards);
    match random.below(4) {
        0 => case.cell_size = 20.0,
        1 => case.cell_size = (40 * (1 + random.below(5))) as f64,
        _ => {}
    }
    if case.cell_size != 0.0 {
        cell = case.cell_size;
    }
    // Now and then a giant: more than 100 cells wide or high. The optimizer leaves it alone.
    // A card of exactly 100 cells is none.
    if case.cell_size != 0.0 && random.below(25) == 0 {
        let giant = random.below(count);
        let length = 100.0 * cell + (20 * random.below(2)) as f64;
        if random.below(2) == 0 {
            sizes[giant].0 = length;
        } else {
            sizes[giant].1 = length;
        }
    }
    let widest = sizes.iter().fold(0.0_f64, |most, size| most.max(size.0));
    let highest = sizes.iter().fold(0.0_f64, |most, size| most.max(size.1));
    let room = (20 + 20 * random.below(5)) as f64;
    let step_x = (((widest + room) / cell).floor() + 1.0) * cell;
    let step_y = (((highest + room) / cell).floor() + 1.0) * cell;
    let mut side = 1 + random.below(5);
    while side * side < count {
        side += 1;
    }
    let mut spots: Vec<usize> = (0..side * side).collect();
    for i in (1..spots.len()).rev() {
        spots.swap(i, random.below(i + 1));
    }
    let loose = random.below(4) == 0;
    for (n, (width, height)) in sizes.iter().enumerate() {
        let (mut x, mut y) = (step_x * (spots[n] % side) as f64, step_y * (spots[n] / side) as f64);
        if loose || random.below(6) == 0 {
            x += cell * (random.below(5) as f64 - 2.0);
            y += cell * (random.below(5) as f64 - 2.0);
        }
        case.nodes.push([x, y, *width, *height]);
    }
    let connect = |random: &mut Random, edges: &mut Vec<[usize; 4]>, a: usize, b: usize| {
        let mut edge = if random.below(2) == 0 { [b, a, 0, 0] } else { [a, b, 0, 0] };
        if random.below(4) == 0 {
            (edge[2], edge[3]) = (30 + random.below(200), 20 + random.below(30));
        }
        edges.push(edge);
        if random.below(10) == 0 {
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
                if random.below(8) > 0 {
                    connect(random, &mut edges, n - 1, n);
                }
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
            case.name = "any".into();
            for _ in 0..1 + random.below(count * 2) {
                let (a, b) = (random.below(count), random.below(count));
                if a != b {
                    connect(random, &mut edges, a, b);
                }
            }
        }
    }
    if random.below(3) == 0 {
        let (a, b) = (random.below(count), random.below(count));
        if a != b {
            connect(random, &mut edges, a, b);
        }
    }
    case.edges = edges;
    if random.below(2) == 0 {
        // Tenths, so that the number is the same in every language.
        case.temp = (2 + random.below(31)) as f64 / 10.0;
    }
    case.halve = random.below(2) == 0;
    case
}

fn places(graph: &Graph) -> Vec<[f64; 2]> {
    graph.nodes.iter().map(|card| [card.top_left.x, card.top_left.y]).collect()
}

/// The port's answers for a layout, asked in the order of TALA's recorder.
fn answered(case: &Case) -> Case {
    let mut answer = Case {
        name: case.name.clone(),
        cell_size: case.cell_size,
        nodes: case.nodes.clone(),
        edges: case.edges.clone(),
        temp: case.temp,
        seed: case.seed,
        halve: case.halve,
        ..Case::default()
    };
    let nodes = case
        .nodes
        .iter()
        .map(|[x, y, width, height]| Node { top_left: Point { x: *x, y: *y }, width: *width, height: *height })
        .collect();
    let edges = case
        .edges
        .iter()
        .map(|[from, to, width, height]| Edge { from: *from, to: *to, min_width: *width as f64, min_height: *height as f64 })
        .collect();
    let mut graph = Graph::new(nodes, edges);
    if case.cell_size != 0.0 {
        graph.cell_size = case.cell_size;
    }
    let turn = Costs::of(&graph).turn / if case.halve { 2.0 } else { 1.0 };
    (answer.cell, answer.turn_cost) = (graph.cell_size, turn);
    answer.hubs = hubs(&graph);
    let mut random = GoRand::new(case.seed);
    for node in 0..graph.nodes.len() {
        if graph.edges_of(node).next().is_none() {
            answer.medians.push(None);
            continue;
        }
        let drawn = random.drawn;
        let median = median_point(&graph, node, case.temp, &mut random);
        answer.medians.push(Some(Median { point: [median.x, median.y], drawn: random.drawn - drawn }));
    }
    for node in 0..graph.nodes.len() {
        let drawn = random.drawn;
        let with = best_swap_candidate(&mut graph, node, turn, &mut random);
        answer.swaps.push(Swap { with: with.map_or(-1, |card| card as i64), drawn: random.drawn - drawn });
    }
    for node in 0..graph.nodes.len() {
        let mut turned = graph.clone();
        answer.transposes.push(transpose(&mut turned, node, turn).then(|| places(&turned)));
    }
    let drawn = random.drawn;
    // TALA's probe stops at a card that finds no free place; no layout made here has one.
    answer.changed = optimize(&mut graph, turn, case.temp, &mut random).expect("a free place for every card");
    answer.after = places(&graph);
    answer.drawn = random.drawn - drawn;
    answer
}

fn main() {
    let arguments: Vec<String> = std::env::args().collect();
    match arguments.get(1).map(String::as_str) {
        Some("write") if arguments.len() == 5 => {
            let count: usize = arguments[3].parse().expect("count");
            let mut random = Random(arguments[4].parse::<u64>().expect("seed").max(1));
            let cases: Vec<Case> = (0..count).map(|_| answered(&layout(&mut random))).collect();
            std::fs::write(&arguments[2], serde_json::to_string(&cases).unwrap()).unwrap();
        }
        Some("compare") if arguments.len() == 4 => {
            let limit: usize = arguments[3].parse().expect("limit");
            let cases: Vec<Case> = serde_json::from_str(&std::fs::read_to_string(&arguments[2]).unwrap()).unwrap();
            let different: Vec<Case> = cases.iter().filter(|case| answered(case) != **case).cloned().collect();
            let count = |of: fn(&Case) -> usize| cases.iter().map(of).sum::<usize>();
            eprintln!(
                "{} layouts, {} cards, {} swaps, {} transposes, {} passes that changed something, {} layouts differ",
                cases.len(),
                count(|case| case.nodes.len()),
                count(|case| case.swaps.iter().filter(|swap| swap.with >= 0).count()),
                count(|case| case.transposes.iter().flatten().count()),
                count(|case| usize::from(case.changed)),
                different.len()
            );
            let shown: Vec<&Case> = different.iter().take(limit).collect();
            println!("{}", serde_json::to_string(&shown).unwrap());
            std::process::exit(i32::from(!different.is_empty()));
        }
        _ => {
            eprintln!("usage: probe_passes write <file> <count> <seed> | probe_passes compare <file> <limit>");
            std::process::exit(2);
        }
    }
}
