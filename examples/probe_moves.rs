// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.

//! Compares one move of one card with TALA, or with a changed copy of the port, on thousands
//! of random layouts (docs/reference.md, "Finding cases"). Ours, not ported.
//!
//!   probe_moves write <file> <count> <seed>   make random layouts, keep the port's answers
//!   probe_moves compare <file> <limit>        answer them again; print the layouts that differ
//!
//! The file can be given to TALA's probe (docs/recorders/move_test.go.txt, `TestMoveProbe`),
//! which writes its own answers in the same form: `compare` on that file then checks the port
//! against TALA.

use tidy_layout::graph::{Edge, Graph, Node, Point};
use tidy_layout::placement::{closest_unoccupied, does_overlap, fill_placement_points, move_node_to_best};
use tidy_layout::placementcost::{edge_length::node_edge_length, symmetry::node_symmetry};
use tidy_layout::routing::router::Costs;
use serde::{Deserialize, Serialize};

#[derive(Serialize, Deserialize, Clone, PartialEq)]
#[serde(rename_all = "camelCase")]
struct Case {
    name: String,
    /// 0: the cell size TALA computes from the card sizes.
    cell_size: f64,
    nodes: Vec<[f64; 4]>,
    /// from, to, and the room the label needs (0, 0 without a label)
    edges: Vec<[usize; 4]>,
    moving: usize,
    point: [f64; 2],
    #[serde(default)]
    cell: f64,
    #[serde(default)]
    turn_cost: f64,
    #[serde(default)]
    distance: f64,
    /// x, y, can the card go there, and if so its edge length and its symmetry there.
    #[serde(default)]
    points: Vec<[f64; 5]>,
    #[serde(default)]
    best: [f64; 2],
    #[serde(default)]
    best_improving: [f64; 2],
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

    /// A multiple of 20 from `least` steps on, `steps` to choose from.
    fn grid(&mut self, least: i64, steps: usize) -> f64 {
        (20 * (least + self.below(steps) as i64)) as f64
    }
}

fn input(name: &str) -> Case {
    Case {
        name: name.into(),
        cell_size: 0.0,
        nodes: Vec::new(),
        edges: Vec::new(),
        moving: 0,
        point: [0.0, 0.0],
        cell: 0.0,
        turn_cost: 0.0,
        distance: 0.0,
        points: Vec::new(),
        best: [0.0, 0.0],
        best_improving: [0.0, 0.0],
    }
}

/// Cards as a person leaves them: on a lattice or anywhere, connected at random, now and then
/// with several connections between the same two cards.
fn scattered(random: &mut Random) -> Case {
    let tidy = random.below(2) == 0;
    let mut case = input(if tidy { "tidy" } else { "loose" });
    let count = 2 + random.below(13);
    let uniform = random.below(3) > 0;
    for _ in 0..count {
        let (width, height) = if uniform { (220.0, 120.0) } else { (random.grid(3, 14), random.grid(3, 8)) };
        let (mut x, mut y) = (random.grid(-10, 70), random.grid(-10, 45));
        if tidy {
            (x, y) = ((300 * random.below(5)) as f64, (200 * random.below(5)) as f64);
            if random.below(4) == 0 {
                x += random.grid(-2, 5);
                y += random.grid(-2, 5);
            }
        }
        case.nodes.push([x, y, width, height]);
    }
    for _ in 0..1 + random.below(count * 2) {
        let (a, b) = (random.below(count), random.below(count));
        if a == b {
            continue;
        }
        let times = if random.below(6) == 0 { 2 + random.below(3) } else { 1 };
        for k in 0..times {
            let mut edge = [a, b, 0, 0];
            if k > 0 && random.below(3) == 0 {
                edge = [b, a, 0, 0];
            }
            if random.below(2) == 0 {
                (edge[2], edge[3]) = (30 + random.below(200), 20 + random.below(30));
            }
            case.edges.push(edge);
        }
    }
    case.moving = random.below(count);
    case.cell_size = match random.below(3) {
        0 => 20.0,
        1 => (40 * (1 + random.below(3))) as f64,
        _ => 0.0,
    };
    let other = case.nodes[random.below(count)];
    case.point = [other[0] + random.below(160) as f64 - 80.0, other[1] + random.below(120) as f64 - 60.0];
    match random.below(8) {
        0 => {
            let own = case.nodes[case.moving];
            case.point = [own[0] + random.below(30) as f64 - 15.0, own[1] + random.below(30) as f64 - 15.0];
        }
        1 => case.point = [random.below(1500) as f64 - 200.0, random.below(1000) as f64 - 200.0],
        _ => {}
    }
    case
}

/// A card dropped where the cards it is connected to lie as mirror images of each other,
/// exactly or nearly, with other cards around that may stand between them.
fn mirrored(random: &mut Random) -> Case {
    let mut case = input("mirror");
    let (w, h) = (220.0, 120.0);
    let (cx, cy) = ((440 * (1 + random.below(3))) as f64, (440 * (1 + random.below(2))) as f64);
    case.cell_size = match random.below(3) {
        0 => 20.0,
        1 => 40.0,
        _ => 0.0,
    };
    fn jitter(random: &mut Random) -> f64 {
        if random.below(3) > 0 { 0.0 } else { random.grid(-6, 13) }
    }
    fn connect(case: &mut Case, random: &mut Random, a: usize, b: usize) {
        let mut edge = if random.below(2) == 0 { [b, a, 0, 0] } else { [a, b, 0, 0] };
        if random.below(3) == 0 {
            (edge[2], edge[3]) = (30 + random.below(120), 20 + random.below(30));
        }
        case.edges.push(edge);
    }
    case.nodes.push(match random.below(4) {
        0 => [cx, cy, w, h],
        1 => [cx + random.grid(-4, 9), cy + random.grid(-4, 9), w, h],
        _ => [random.grid(0, 90), random.grid(0, 60), w, h],
    });
    for _ in 0..1 + random.below(3) {
        let (nw, nh) = if random.below(4) == 0 { (random.grid(5, 12), random.grid(3, 8)) } else { (w, h) };
        let gap = random.grid(3, 12);
        let (a, mut b);
        if random.below(2) == 0 {
            let y = cy + (h - nh) / 2.0;
            a = [cx - gap - nw + jitter(random), y + jitter(random), nw, nh];
            b = [cx + w + gap + jitter(random), y + jitter(random), nw, nh];
        } else {
            let x = cx + (w - nw) / 2.0;
            a = [x + jitter(random), cy - gap - nh + jitter(random), nw, nh];
            b = [x + jitter(random), cy + h + gap + jitter(random), nw, nh];
        }
        if random.below(6) == 0 {
            (b[2], b[3]) = (random.grid(3, 20), random.grid(3, 12));
        }
        case.nodes.push(a);
        case.nodes.push(b);
        let last = case.nodes.len() - 1;
        connect(&mut case, random, 0, last - 1);
        connect(&mut case, random, 0, last);
        if random.below(3) == 0 {
            case.nodes.push([2.0 * a[0] - cx + jitter(random), 2.0 * a[1] - cy + jitter(random), w, h]);
            connect(&mut case, random, last - 1, last + 1);
        }
    }
    for _ in 0..random.below(6) {
        case.nodes.push([cx + random.grid(-35, 70), cy + random.grid(-25, 50), random.grid(3, 12), random.grid(3, 6)]);
        if random.below(2) == 0 {
            let last = case.nodes.len() - 1;
            let other = random.below(last);
            connect(&mut case, random, other, last);
        }
    }
    case.point = [cx + random.below(15) as f64 - 7.0, cy + random.below(15) as f64 - 7.0];
    if random.below(3) == 0 {
        case.point[0] += random.grid(-3, 7);
        case.point[1] += random.grid(-3, 7);
    }
    case
}

/// Situations a person rarely makes and the other layouts therefore rarely hold: a card with
/// eight or nine connections, connections both ways, a card far away, two cards on one side,
/// a card twice the size of its mirror image, small cards on a coarse grid, sides exactly on a
/// centre line or exactly 40 out of line, many labels between the same cards, a card lying
/// on top of another.
fn unusual(random: &mut Random) -> Case {
    let mut case = input("unusual");
    let (w, h) = if random.below(2) == 0 { (220.0, 120.0) } else { (random.grid(3, 9), random.grid(3, 5)) };
    let (cx, cy) = ((440 * (1 + random.below(3))) as f64, (440 * (1 + random.below(2))) as f64);
    case.cell_size = [20.0, 40.0, 160.0, 220.0, 0.0, 0.0][random.below(6)];
    let cell = if case.cell_size == 0.0 { 220 } else { case.cell_size as usize };
    case.nodes.push(match random.below(3) {
        0 => [cx, cy, w, h],
        1 => [cx + random.grid(-4, 9), cy + random.grid(-4, 9), w, h],
        _ => [random.grid(0, 90), random.grid(0, 60), w, h],
    });
    fn connect(case: &mut Case, random: &mut Random, cell: usize, a: usize, b: usize) {
        let times = [1, 1, 1, 1, 2, 3, 4][random.below(7)];
        let labelled = random.below(2) == 0;
        for _ in 0..times {
            let mut edge = if random.below(2) == 0 { [b, a, 0, 0] } else { [a, b, 0, 0] };
            if labelled || random.below(4) == 0 {
                (edge[2], edge[3]) = match random.below(4) {
                    // As wide or as high as a cell, or a little more or less; narrow and tall.
                    0 => (cell + random.below(3) - 1, 20 + random.below(30)),
                    1 => (20 + random.below(20), cell + random.below(3) - 1),
                    _ => (30 + random.below(200), 20 + random.below(30)),
                };
            }
            case.edges.push(edge);
        }
    }
    let offset = |random: &mut Random| [0.0, 0.0, 0.0, 20.0, -20.0, 40.0, -40.0, 60.0, -60.0][random.below(9)];
    for _ in 0..1 + random.below(3) {
        let (nw, nh) = if random.below(3) == 0 { (random.grid(3, 9), random.grid(3, 5)) } else { (w, h) };
        // Mostly near; now and then about as far as symmetry looks, and beyond.
        let gap = if random.below(6) == 0 { 1200.0 + random.grid(-1, 3) } else { random.grid(3, 12) };
        let (a, mut b);
        if random.below(2) == 0 {
            let y = cy + (h - nh) / 2.0;
            a = [cx - gap - nw + offset(random), y + offset(random), nw, nh];
            b = [cx + w + gap + offset(random), y + offset(random), nw, nh];
        } else {
            let x = cx + (w - nw) / 2.0;
            a = [x + offset(random), cy - gap - nh + offset(random), nw, nh];
            b = [x + offset(random), cy + h + gap + offset(random), nw, nh];
        }
        match random.below(8) {
            0 => b[2] *= 2.0,
            1 => b[3] *= 2.0,
            _ => {}
        }
        case.nodes.push(a);
        case.nodes.push(b);
        let last = case.nodes.len() - 1;
        connect(&mut case, random, cell, 0, last - 1);
        connect(&mut case, random, cell, 0, last);
        // A second card on the side of the first, close enough to be taken for it.
        if random.below(3) == 0 {
            let twin = if random.below(2) == 0 { [a[0], a[1] + a[3] + 20.0, nw, nh] } else { [a[0] + offset(random), a[1] - a[3] - 20.0, nw, nh] };
            case.nodes.push(twin);
            let last = case.nodes.len() - 1;
            connect(&mut case, random, cell, 0, last);
        }
    }
    // Small cards across a corner of the middle, a side on its centre line or just beside it.
    if random.below(3) == 0 {
        let size = random.grid(3, 3);
        let (dx, dy) = (random.grid(0, 3), random.grid(0, 3));
        let axis_x = cx + w / 2.0;
        let (below, above) = (cy + h + 60.0 + random.grid(0, 3), cy - 60.0 - size - random.grid(0, 3));
        let pair = if random.below(2) == 0 {
            [[axis_x + dx, below, size, size], [axis_x - size - dy, above, size, size]]
        } else {
            [[axis_x - size - dx, below, size, size], [axis_x + dy, above, size, size]]
        };
        for card in pair {
            case.nodes.push(card);
            let last = case.nodes.len() - 1;
            connect(&mut case, random, cell, 0, last);
        }
        // On a coarse enough grid the two count as mirror images across both centre lines.
        if random.below(2) == 0 {
            case.cell_size = 400.0;
        }
    }
    // A ring of cards, all connected to the moving one.
    if random.below(4) == 0 {
        for _ in 0..4 + random.below(6) {
            case.nodes.push([cx + random.grid(-40, 80), cy + random.grid(-30, 60), random.grid(3, 9), random.grid(3, 5)]);
            let last = case.nodes.len() - 1;
            let mut edge = if random.below(2) == 0 { [0, last, 0, 0] } else { [last, 0, 0, 0] };
            if random.below(4) == 0 {
                (edge[2], edge[3]) = (30 + random.below(100), 20 + random.below(30));
            }
            case.edges.push(edge);
        }
    }
    for _ in 0..random.below(5) {
        case.nodes.push([cx + random.grid(-35, 70), cy + random.grid(-25, 50), random.grid(3, 12), random.grid(3, 6)]);
        if random.below(2) == 0 {
            let last = case.nodes.len() - 1;
            let other = random.below(last);
            connect(&mut case, random, cell, other, last);
        }
    }
    // The moving card lies on top of a card it is connected to.
    if random.below(12) == 0 {
        case.nodes[0] = case.nodes[1];
    }
    case.point = [cx + random.below(15) as f64 - 7.0, cy + random.below(15) as f64 - 7.0];
    match random.below(6) {
        0 => {
            case.point[0] += random.grid(-3, 7);
            case.point[1] += random.grid(-3, 7);
        }
        1 => case.point = [case.nodes[0][0] + random.below(9) as f64 - 4.0, case.nodes[0][1] + random.below(9) as f64 - 4.0],
        _ => {}
    }
    case
}

fn graph_of(case: &Case) -> Graph {
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
    graph
}

/// The port's answers for a layout, in the form TALA's probe writes.
fn answered(case: &Case) -> Case {
    let mut graph = graph_of(case);
    let mut answer = Case { cell: graph.cell_size, turn_cost: Costs::of(&graph).turn, ..input(&case.name) };
    (answer.cell_size, answer.nodes, answer.edges) = (case.cell_size, case.nodes.clone(), case.edges.clone());
    (answer.moving, answer.point) = (case.moving, case.point);
    let median = Point { x: case.point[0], y: case.point[1] };
    let Some(nearest) = closest_unoccupied(&graph, case.moving, median) else {
        answer.distance = -1.0;
        return answer;
    };
    answer.distance = f64::from(nearest.ring);
    let points = fill_placement_points(&graph, case.moving, median, nearest.ring, false);
    let home = graph.nodes[case.moving].top_left;
    for point in &points {
        let taken = graph.nodes.iter().any(|node| node.top_left == *point);
        let mut row = [point.x, point.y, 0.0, 0.0, 0.0];
        if *point == home || (!taken && !does_overlap(&graph, case.moving, *point)) {
            graph.nodes[case.moving].top_left = *point;
            row[2] = 1.0;
            row[3] = node_edge_length(&graph, case.moving, answer.turn_cost);
            row[4] = node_symmetry(&graph, case.moving);
            graph.nodes[case.moving].top_left = home;
        }
        answer.points.push(row);
    }
    for must_improve in [false, true] {
        let mut graph = graph.clone();
        move_node_to_best(&mut graph, case.moving, &points, must_improve, answer.turn_cost);
        let place = graph.nodes[case.moving].top_left;
        *(if must_improve { &mut answer.best_improving } else { &mut answer.best }) = [place.x, place.y];
    }
    answer
}

/// Bit for bit: the prices must be the very numbers TALA computes.
fn same(a: &Case, b: &Case) -> bool {
    let bits = |rows: &[[f64; 5]]| rows.iter().flatten().map(|value| value.to_bits()).collect::<Vec<_>>();
    a.cell == b.cell
        && a.turn_cost.to_bits() == b.turn_cost.to_bits()
        && a.distance == b.distance
        && bits(&a.points) == bits(&b.points)
        && a.best == b.best
        && a.best_improving == b.best_improving
}

fn main() {
    let arguments: Vec<String> = std::env::args().collect();
    match arguments.get(1).map(String::as_str) {
        Some("write") if arguments.len() == 5 => {
            let count: usize = arguments[3].parse().expect("count");
            let mut random = Random(arguments[4].parse::<u64>().expect("seed").max(1));
            let cases: Vec<Case> = (0..count)
                .map(|i| {
                    answered(&match i % 4 {
                        2 => mirrored(&mut random),
                        3 => unusual(&mut random),
                        _ => scattered(&mut random),
                    })
                })
                .collect();
            std::fs::write(&arguments[2], serde_json::to_string(&cases).unwrap()).unwrap();
        }
        Some("compare") if arguments.len() == 4 => {
            let limit: usize = arguments[3].parse().expect("limit");
            let cases: Vec<Case> = serde_json::from_str(&std::fs::read_to_string(&arguments[2]).unwrap()).unwrap();
            let different: Vec<Case> = cases.iter().filter(|case| !same(&answered(case), case)).cloned().collect();
            let (places, priced) = cases.iter().fold((0, 0), |(places, priced), case| {
                (places + case.points.len(), priced + case.points.iter().filter(|row| row[2] == 1.0).count())
            });
            eprintln!("{} layouts, {places} places, {priced} of them priced, {} layouts differ", cases.len(), different.len());
            let shown: Vec<&Case> = different.iter().take(limit).collect();
            println!("{}", serde_json::to_string(&shown).unwrap());
            std::process::exit(i32::from(!different.is_empty()));
        }
        _ => {
            eprintln!("usage: probe_moves write <file> <count> <seed> | probe_moves compare <file> <limit>");
            std::process::exit(2);
        }
    }
}
