// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.

//! Compares the stage that takes a hook out of a route with TALA on more made-up routes
//! (docs/reference.md, "Finding cases"). Ours, not ported.
//!
//!   probe_simplify compare <file> <limit>
//!
//! The file is what the recorder writes with another seed and count
//! (docs/recorders/simplify_test.go.txt, `SIMPLIFY_SEED`, `SIMPLIFY_COUNT`). Prints the cases
//! that differ, smallest first, as the recorder takes them (`REFERENCE_FOUND`), and fails if
//! there is one.

use tidy_layout::graph::{Edge, Graph, Node, Point};
use tidy_layout::routing::simplify::simplify;
use serde::{Deserialize, Serialize};

#[derive(Serialize, Deserialize)]
struct Case {
    cards: Vec<[f64; 4]>,
    links: Vec<[usize; 2]>,
    before: Vec<Vec<[f64; 2]>>,
    #[serde(skip_serializing)]
    after: Vec<Vec<[f64; 2]>>,
}

fn main() {
    let arguments: Vec<String> = std::env::args().collect();
    if arguments.len() < 4 || arguments[1] != "compare" {
        eprintln!("usage: probe_simplify compare <file> <limit>");
        std::process::exit(2);
    }
    let cases: Vec<Case> = serde_json::from_str(&std::fs::read_to_string(&arguments[2]).unwrap()).unwrap();
    let limit: usize = arguments[3].parse().unwrap();
    std::panic::set_hook(Box::new(|_| {}));
    let count = cases.len();
    let mut differ: Vec<Case> = cases
        .into_iter()
        .filter(|case| {
            let nodes = case.cards.iter().map(|[x, y, width, height]| Node { top_left: Point { x: *x, y: *y }, width: *width, height: *height }).collect();
            let edges = case.links.iter().map(|[from, to]| Edge { from: *from, to: *to, min_width: 0.0, min_height: 0.0 }).collect();
            let graph = Graph::new(nodes, edges);
            let before: Vec<Vec<Point>> = case.before.iter().map(|route| route.iter().map(|[x, y]| Point { x: *x, y: *y }).collect()).collect();
            let ours = std::panic::catch_unwind(|| {
                let mut routes = before.clone();
                simplify(&graph, &mut routes);
                routes
            });
            !ours.is_ok_and(|ours| ours.iter().map(|route| route.iter().map(|point| [point.x, point.y]).collect::<Vec<_>>()).eq(case.after.iter().cloned()))
        })
        .collect();
    eprintln!("{count} cases: {} differ", differ.len());
    differ.sort_by_key(|case| case.cards.len() * 100 + case.before.iter().map(Vec::len).sum::<usize>());
    println!("{}", serde_json::to_string(&differ.iter().take(limit).collect::<Vec<_>>()).unwrap());
    if !differ.is_empty() {
        std::process::exit(1);
    }
}
