// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.

//! The stage that nudges channels and the counts it judges its work by, replayed on layouts
//! recorded from TALA's own Go code (docs/reference.md): cards, the routes and label places
//! TALA's `RouteEdges` gave, the routes after `NudgeEdgeChannels`, and what `quality.Inspect`
//! counts before and after.

use tidy_layout::graph::{Edge, Graph, Node, Point};
use tidy_layout::quality::inspect;
use tidy_layout::routing::labels::{LabelSize, Placed, Position};
use tidy_layout::routing::nudge::{label_boxes, nudge};
use serde::Deserialize;

#[derive(Deserialize)]
struct Card {
    x: f64,
    y: f64,
    width: f64,
    height: f64,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct Case {
    kind: String,
    cards: Vec<Card>,
    links: Vec<[usize; 2]>,
    sizes: Vec<[f64; 2]>,
    places: Vec<[f64; 2]>,
    before: Vec<Vec<[f64; 2]>>,
    after: Vec<Vec<[f64; 2]>>,
    counts_before: Vec<f64>,
    counts_after: Vec<f64>,
}

#[test]
fn channels_are_nudged_as_tala_nudges_them() {
    let cases: Vec<Case> = serde_json::from_str(include_str!("fixtures/nudge.tala.json")).unwrap();
    let (mut changed, mut labels, mut through_a_card, mut covered, mut crossing, mut differ) = (0, 0, 0, 0, 0, Vec::new());
    for (number, case) in cases.iter().enumerate() {
        let nodes = case.cards.iter().map(|card| Node { top_left: Point { x: card.x, y: card.y }, width: card.width, height: card.height }).collect();
        let edges = case.links.iter().map(|[from, to]| Edge { from: *from, to: *to, min_width: 0.0, min_height: 0.0 }).collect();
        let graph = Graph::new(nodes, edges);
        let sizes: Vec<Option<LabelSize>> = case.sizes.iter().map(|[width, height]| (*width > 0.0).then_some(LabelSize { width: *width, height: *height })).collect();
        let places: Vec<Option<Placed>> = case
            .sizes
            .iter()
            .zip(&case.places)
            .map(|([width, _], [position, percentage])| (*width > 0.0).then(|| Placed { position: Position::of(*position as u8).expect("a position TALA uses"), percentage: *percentage, top_left: Point { x: 0.0, y: 0.0 } }))
            .collect();
        let mut routes: Vec<Vec<Point>> = case.before.iter().map(|route| route.iter().map(|[x, y]| Point { x: *x, y: *y }).collect()).collect();
        let counts = |routes: &[Vec<Point>]| {
            let counted = inspect(&graph, routes, &label_boxes(routes, &places, &sizes));
            [counted.node_overlaps as f64, counted.route_obstructions as f64, counted.text_occlusions as f64, counted.crossings, counted.detour, counted.route_length].map(f64::to_bits).to_vec()
        };
        let bits = |values: &[f64]| values.iter().map(|value| value.to_bits()).collect::<Vec<_>>();
        let what = format!("case {number} ({}, {} cards, {} connections)", case.kind, case.cards.len(), case.links.len());
        if counts(&routes) != bits(&case.counts_before) {
            differ.push(format!("{what}: the counts before"));
        }
        nudge(&graph, &mut routes, &places, &sizes);
        let ours: Vec<Vec<[f64; 2]>> = routes.iter().map(|route| route.iter().map(|point| [point.x, point.y]).collect()).collect();
        if ours != case.after {
            differ.push(format!("{what}: the routes after the stage"));
        }
        if counts(&routes) != bits(&case.counts_after) {
            differ.push(format!("{what}: the counts after"));
        }
        changed += usize::from(case.before != case.after);
        labels += sizes.iter().flatten().count();
        through_a_card += usize::from(case.counts_before[1] > 0.0);
        covered += usize::from(case.counts_before[2] > 0.0);
        crossing += usize::from(case.counts_before[3] > 0.0);
    }
    assert_eq!(differ.len(), 0, "{} differ: {}", differ.len(), differ.iter().take(5).cloned().collect::<Vec<_>>().join("; "));
    // The cases, those in which the stage changes a route, the labels, and the cases with a
    // route through a card, with a covered label and with a crossing before the stage.
    assert_eq!((cases.len(), changed, labels, through_a_card, covered, crossing), (403, 203, 3586, 10, 247, 282));
}
