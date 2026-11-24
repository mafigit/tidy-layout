// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.

//! Compares the stage that nudges channels, and the counts it judges by, with TALA on more
//! layouts (docs/reference.md, "Finding cases"). Ours, not ported.
//!
//!   probe_nudge compare <file> <limit>
//!
//! The file is what the recorder writes with another seed and count
//! (docs/recorders/nudge_test.go.txt, `NUDGE_SEED`, `NUDGE_COUNT`). Prints the layouts that
//! differ, smallest first, as the recorder takes them (`REFERENCE_FOUND`), and fails if there
//! is one.

use tidy_layout::graph::{Edge, Graph, Node, Point};
use tidy_layout::quality::inspect;
use tidy_layout::routing::labels::{LabelSize, Placed, Position};
use tidy_layout::routing::nudge::{label_boxes, nudge};
use serde::{Deserialize, Serialize};

#[derive(Serialize, Deserialize, Clone)]
struct Card {
    x: f64,
    y: f64,
    width: f64,
    height: f64,
}

#[derive(Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
struct Case {
    cards: Vec<Card>,
    links: Vec<[usize; 2]>,
    sizes: Vec<[f64; 2]>,
    #[serde(skip_serializing)]
    places: Vec<[f64; 2]>,
    #[serde(skip_serializing)]
    before: Vec<Vec<[f64; 2]>>,
    #[serde(skip_serializing)]
    after: Vec<Vec<[f64; 2]>>,
    #[serde(skip_serializing)]
    counts_before: Vec<f64>,
    #[serde(skip_serializing)]
    counts_after: Vec<f64>,
}

fn same(case: &Case) -> bool {
    let nodes = case.cards.iter().map(|card| Node { top_left: Point { x: card.x, y: card.y }, width: card.width, height: card.height }).collect();
    let edges = case.links.iter().map(|[from, to]| Edge { from: *from, to: *to, min_width: 0.0, min_height: 0.0 }).collect();
    let graph = Graph::new(nodes, edges);
    let sizes: Vec<Option<LabelSize>> = case.sizes.iter().map(|[width, height]| (*width > 0.0).then_some(LabelSize { width: *width, height: *height })).collect();
    let Some(places) = case
        .sizes
        .iter()
        .zip(&case.places)
        .map(|([width, _], [position, percentage])| if *width > 0.0 { Position::of(*position as u8).map(|position| Some(Placed { position, percentage: *percentage, top_left: Point { x: 0.0, y: 0.0 } })) } else { Some(None) })
        .collect::<Option<Vec<Option<Placed>>>>()
    else {
        return false;
    };
    let mut routes: Vec<Vec<Point>> = case.before.iter().map(|route| route.iter().map(|[x, y]| Point { x: *x, y: *y }).collect()).collect();
    let counts = |routes: &[Vec<Point>]| {
        let counted = inspect(&graph, routes, &label_boxes(routes, &places, &sizes));
        [counted.node_overlaps as f64, counted.route_obstructions as f64, counted.text_occlusions as f64, counted.crossings, counted.detour, counted.route_length].map(f64::to_bits).to_vec()
    };
    let bits = |values: &[f64]| values.iter().map(|value| value.to_bits()).collect::<Vec<_>>();
    if counts(&routes) != bits(&case.counts_before) {
        return false;
    }
    nudge(&graph, &mut routes, &places, &sizes);
    routes.iter().map(|route| route.iter().map(|point| [point.x, point.y]).collect::<Vec<_>>()).eq(case.after.iter().cloned()) && counts(&routes) == bits(&case.counts_after)
}

fn main() {
    let arguments: Vec<String> = std::env::args().collect();
    if arguments.len() < 4 || arguments[1] != "compare" {
        eprintln!("usage: probe_nudge compare <file> <limit>");
        std::process::exit(2);
    }
    let cases: Vec<Case> = serde_json::from_str(&std::fs::read_to_string(&arguments[2]).unwrap()).unwrap();
    let limit: usize = arguments[3].parse().unwrap();
    std::panic::set_hook(Box::new(|_| {}));
    let (count, changed) = (cases.len(), cases.iter().filter(|case| case.before != case.after).count());
    let mut differ: Vec<Case> = cases.into_iter().filter(|case| !std::panic::catch_unwind(|| same(case)).unwrap_or(false)).collect();
    eprintln!("{count} layouts ({changed} in which TALA changes a route): {} differ", differ.len());
    differ.sort_by_key(|case| case.cards.len() * 100 + case.links.len());
    println!("{}", serde_json::to_string(&differ.iter().take(limit).collect::<Vec<_>>()).unwrap());
    if !differ.is_empty() {
        std::process::exit(1);
    }
}
