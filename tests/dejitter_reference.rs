// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.

//! The stage that takes the jitter out of routes, replayed on layouts recorded from TALA's own
//! Go code (docs/reference.md): cards a little out of line, the routes TALA's first routing
//! gave, and the cards and routes after `Dejitter`.

use tidy_layout::dejitter::dejitter;
use tidy_layout::graph::{Edge, Graph, Node, Point};
use serde::Deserialize;

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct Case {
    cards: Vec<[f64; 4]>,
    links: Vec<[usize; 2]>,
    routes: Vec<Vec<[f64; 2]>>,
    moved: bool,
    cards_after: Vec<[f64; 2]>,
    routes_after: Vec<Vec<[f64; 2]>>,
}

#[test]
fn jitter_goes_where_tala_takes_it_out() {
    let cases: Vec<Case> = serde_json::from_str(include_str!("fixtures/dejitter.tala.json")).unwrap();
    let (mut moved, mut cards_moved, mut differ) = (0, 0, Vec::new());
    for (number, case) in cases.iter().enumerate() {
        let nodes = case.cards.iter().map(|[x, y, width, height]| Node { top_left: Point { x: *x, y: *y }, width: *width, height: *height }).collect();
        let edges = case.links.iter().map(|[from, to]| Edge { from: *from, to: *to, min_width: 0.0, min_height: 0.0 }).collect();
        let mut graph = Graph::new(nodes, edges);
        let mut routes: Vec<Vec<Point>> = case.routes.iter().map(|route| route.iter().map(|[x, y]| Point { x: *x, y: *y }).collect()).collect();
        let says = dejitter(&mut graph, &mut routes);
        let cards: Vec<[f64; 2]> = graph.nodes.iter().map(|node| [node.top_left.x, node.top_left.y]).collect();
        let ours: Vec<Vec<[f64; 2]>> = routes.iter().map(|route| route.iter().map(|point| [point.x, point.y]).collect()).collect();
        if says != case.moved || cards != case.cards_after || ours != case.routes_after {
            differ.push(format!("case {number} ({} cards, {} connections)", case.cards.len(), case.links.len()));
        }
        moved += usize::from(case.moved);
        cards_moved += case.cards.iter().zip(&case.cards_after).filter(|(before, after)| [before[0], before[1]] != **after).count();
    }
    assert_eq!(differ.len(), 0, "{} differ: {}", differ.len(), differ.iter().take(8).cloned().collect::<Vec<_>>().join("; "));
    // The cases, those in which the stage moves a card, and the cards it moves in all.
    assert_eq!((cases.len(), moved, cards_moved), (409, 205, 221));
}
