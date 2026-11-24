// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.

//! The stage that takes a hook out of a route, replayed on routes recorded from TALA's own Go
//! code (docs/reference.md). The routes are made up: the router makes a hook in about one
//! diagram of 44,000.

use tidy_layout::graph::{Edge, Graph, Node, Point};
use tidy_layout::routing::simplify::simplify;
use serde::Deserialize;

#[derive(Deserialize)]
struct Case {
    cards: Vec<[f64; 4]>,
    links: Vec<[usize; 2]>,
    before: Vec<Vec<[f64; 2]>>,
    after: Vec<Vec<[f64; 2]>>,
}

#[test]
fn hooks_go_where_tala_takes_them_out() {
    let cases: Vec<Case> = serde_json::from_str(include_str!("fixtures/simplify.tala.json")).unwrap();
    let (mut routes, mut shorter, mut twice, mut differ) = (0, 0, 0, Vec::new());
    for (number, case) in cases.iter().enumerate() {
        let nodes = case.cards.iter().map(|[x, y, width, height]| Node { top_left: Point { x: *x, y: *y }, width: *width, height: *height }).collect();
        let edges = case.links.iter().map(|[from, to]| Edge { from: *from, to: *to, min_width: 0.0, min_height: 0.0 }).collect();
        let graph = Graph::new(nodes, edges);
        let mut ours: Vec<Vec<Point>> = case.before.iter().map(|route| route.iter().map(|[x, y]| Point { x: *x, y: *y }).collect()).collect();
        simplify(&graph, &mut ours);
        let ours: Vec<Vec<[f64; 2]>> = ours.iter().map(|route| route.iter().map(|point| [point.x, point.y]).collect()).collect();
        if ours != case.after {
            differ.push(format!("case {number}: ours {ours:?}, TALA {:?}", case.after));
        }
        routes += case.before.len();
        shorter += case.before.iter().zip(&case.after).filter(|(before, after)| after.len() < before.len()).count();
        twice += case.before.iter().zip(&case.after).filter(|(before, after)| after.len() + 2 < before.len()).count();
    }
    assert_eq!(differ.len(), 0, "{} differ, the first: {}", differ.len(), differ[0]);
    assert_eq!((cases.len(), routes, shorter, twice), (1200, 2372, 1059, 285));
}
