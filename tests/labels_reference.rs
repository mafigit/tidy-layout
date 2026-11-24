// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.

//! Labels must get the places TALA gives them: these tests replay layouts recorded from TALA's
//! own Go code (docs/reference.md).

use tidy_layout::graph::{Edge, Graph, Node, Point};
use tidy_layout::routing::coordinator::{route_edges, route_edges_with_labels};
use tidy_layout::routing::labels::LabelSize;
use serde::Deserialize;

#[derive(Deserialize)]
struct Card {
    x: f64,
    y: f64,
    width: f64,
    height: f64,
}

#[derive(Deserialize)]
struct Place {
    position: u8,
    percentage: f64,
    x: f64,
    y: f64,
}

#[derive(Deserialize)]
struct Case {
    kind: String,
    cards: Vec<Card>,
    links: Vec<[usize; 2]>,
    sizes: Vec<[f64; 2]>,
    routes: Vec<Vec<[f64; 2]>>,
    #[serde(default)]
    reordered: bool,
    places: Vec<Option<Place>>,
}

#[test]
fn labels_get_the_places_tala_gives_them() {
    let cases: Vec<Case> = serde_json::from_str(include_str!("fixtures/labels.tala.json")).unwrap();
    assert_eq!(cases.len(), 246);
    let (mut labels, mut reordered) = (0, 0);
    let mut positions = std::collections::BTreeMap::new();
    for (number, case) in cases.iter().enumerate() {
        if case.links.is_empty() {
            continue;
        }
        let nodes = case
            .cards
            .iter()
            .map(|c| Node { top_left: Point { x: c.x, y: c.y }, width: c.width, height: c.height })
            .collect();
        let edges = case.links.iter().map(|[from, to]| Edge::plain(*from, *to)).collect();
        let graph = Graph::new(nodes, edges);
        let sizes: Vec<Option<LabelSize>> = case
            .sizes
            .iter()
            .map(|[width, height]| (*width > 0.0).then_some(LabelSize { width: *width, height: *height }))
            .collect();
        let what = format!("case {number} ({}, {} cards)", case.kind, case.cards.len());

        let routing = route_edges_with_labels(&graph, &sizes).unwrap();
        let (routes, places) = (routing.routes, routing.labels);
        let ours: Vec<Vec<[f64; 2]>> = routes.iter().map(|route| route.iter().map(|p| [p.x, p.y]).collect()).collect();
        assert_eq!(ours, case.routes, "{what}: routes");
        // The recorder says whether the labels changed the routes: so must we.
        assert_eq!(routes != route_edges(&graph).unwrap(), case.reordered, "{what}: routes swapped for labels");
        reordered += usize::from(case.reordered);

        for (link, (ours, theirs)) in places.iter().zip(&case.places).enumerate() {
            let what = format!("{what}, link {link} {:?}, label {:?}, route {:?}", case.links[link], case.sizes[link], case.routes[link]);
            match (ours, theirs) {
                (None, None) => {}
                (Some(ours), Some(theirs)) => {
                    assert_eq!(ours.position as u8, theirs.position, "{what}: position");
                    assert_eq!(ours.percentage, theirs.percentage, "{what}: how far along");
                    assert_eq!((ours.top_left.x, ours.top_left.y), (theirs.x, theirs.y), "{what}: corner");
                    labels += 1;
                    *positions.entry(theirs.position).or_insert(0) += 1;
                }
                _ => panic!("{what}: one of us placed a label, the other did not"),
            }
        }
    }
    // The cases must keep covering what is easy to lose: every position TALA uses, and routes
    // swapped for a label.
    assert_eq!((labels, reordered), (1217, 4));
    assert_eq!(positions.len(), 12, "{positions:?}");
}
