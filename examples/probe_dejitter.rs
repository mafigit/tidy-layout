// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.

//! Compares the stage that takes the jitter out of routes with TALA on more layouts
//! (docs/reference.md, "Finding cases"). Ours, not ported.
//!
//!   probe_dejitter compare <file> <limit>
//!
//! The file is what the recorder writes with another seed and count
//! (docs/recorders/dejitter_test.go.txt, `DEJITTER_SEED`, `DEJITTER_COUNT`). Prints the layouts
//! that differ, smallest first, as the recorder takes them (`REFERENCE_FOUND`), and fails if
//! there is one.

use tidy_layout::dejitter::dejitter;
use tidy_layout::graph::{Edge, Graph, Node, Point};
use serde::{Deserialize, Serialize};

#[derive(Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
struct Case {
    cards: Vec<[f64; 4]>,
    links: Vec<[usize; 2]>,
    #[serde(skip_serializing)]
    routes: Vec<Vec<[f64; 2]>>,
    #[serde(skip_serializing)]
    moved: bool,
    #[serde(skip_serializing)]
    cards_after: Vec<[f64; 2]>,
    #[serde(skip_serializing)]
    routes_after: Vec<Vec<[f64; 2]>>,
}

fn same(case: &Case) -> bool {
    let nodes = case.cards.iter().map(|[x, y, width, height]| Node { top_left: Point { x: *x, y: *y }, width: *width, height: *height }).collect();
    let edges = case.links.iter().map(|[from, to]| Edge { from: *from, to: *to, min_width: 0.0, min_height: 0.0 }).collect();
    let mut graph = Graph::new(nodes, edges);
    let mut routes: Vec<Vec<Point>> = case.routes.iter().map(|route| route.iter().map(|[x, y]| Point { x: *x, y: *y }).collect()).collect();
    let says = dejitter(&mut graph, &mut routes);
    says == case.moved
        && graph.nodes.iter().map(|node| [node.top_left.x, node.top_left.y]).eq(case.cards_after.iter().copied())
        && routes.iter().map(|route| route.iter().map(|point| [point.x, point.y]).collect::<Vec<_>>()).eq(case.routes_after.iter().cloned())
}

fn main() {
    let arguments: Vec<String> = std::env::args().collect();
    if arguments.len() < 4 || arguments[1] != "compare" {
        eprintln!("usage: probe_dejitter compare <file> <limit>");
        std::process::exit(2);
    }
    let cases: Vec<Case> = serde_json::from_str(&std::fs::read_to_string(&arguments[2]).unwrap()).unwrap();
    let limit: usize = arguments[3].parse().unwrap();
    std::panic::set_hook(Box::new(|_| {}));
    let (count, moved) = (cases.len(), cases.iter().filter(|case| case.moved).count());
    let mut differ: Vec<Case> = cases.into_iter().filter(|case| !std::panic::catch_unwind(|| same(case)).unwrap_or(false)).collect();
    eprintln!("{count} layouts ({moved} in which TALA moves a card): {} differ", differ.len());
    differ.sort_by_key(|case| case.cards.len() * 100 + case.links.len());
    println!("{}", serde_json::to_string(&differ.iter().take(limit).collect::<Vec<_>>()).unwrap());
    if !differ.is_empty() {
        std::process::exit(1);
    }
}
