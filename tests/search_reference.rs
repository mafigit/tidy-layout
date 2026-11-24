// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.

//! The search must find TALA's routes: these tests replay searches recorded from TALA's own Go
//! code (docs/reference.md).

use tidy_layout::graph::{Edge, Graph, Node, Point};
use tidy_layout::routing::ovg;
use tidy_layout::routing::router::{Costs, Router};
use serde::Deserialize;

#[derive(Deserialize)]
struct Card {
    x: f64,
    y: f64,
    width: f64,
    height: f64,
}

#[derive(Deserialize)]
struct RefRoute {
    link: usize,
    #[serde(default)]
    failed: bool,
    distance: f64,
    points: usize,
    hash: String,
    corners: Option<Vec<[i64; 2]>>,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct Case {
    kind: String,
    cards: Vec<Card>,
    links: Vec<[usize; 2]>,
    turn_cost: f64,
    crossing_cost: f64,
    non_center_port_cost: f64,
    routes: Vec<RefRoute>,
}

fn q(value: f64) -> i64 {
    let quarters = (value * 4.0) as i64;
    assert_eq!(quarters as f64, value * 4.0, "{value} is not a multiple of 0.25");
    quarters
}

/// FNV-1a, 64 bit: what Go's `hash/fnv` `New64a` computes.
fn fnv1a(text: &str) -> String {
    let mut hash: u64 = 0xcbf29ce484222325;
    for byte in text.bytes() {
        hash ^= u64::from(byte);
        hash = hash.wrapping_mul(0x100000001b3);
    }
    format!("{hash:016x}")
}

/// The card centres, the ports and every bend of a route, in quarters.
fn corners(points: &[Point]) -> Vec<[i64; 2]> {
    let count = points.len();
    points
        .iter()
        .enumerate()
        .filter(|(n, point)| {
            let n = *n;
            if n < 2 || n + 2 >= count {
                return true;
            }
            let (last, next) = (points[n - 1], points[n + 1]);
            (point.x == last.x && point.x != next.x) || (point.y == last.y && point.y != next.y)
        })
        .map(|(_, point)| [q(point.x), q(point.y)])
        .collect()
}

#[test]
fn search_finds_the_routes_tala_finds() {
    let cases: Vec<Case> = serde_json::from_str(include_str!("fixtures/search.tala.json")).unwrap();
    assert_eq!(cases.len(), 312);
    let mut routes_checked = 0;
    for (number, case) in cases.iter().enumerate() {
        let nodes = case
            .cards
            .iter()
            .map(|c| Node { top_left: Point { x: c.x, y: c.y }, width: c.width, height: c.height })
            .collect();
        let edges = case.links.iter().map(|[from, to]| Edge::plain(*from, *to)).collect();
        let graph = Graph::new(nodes, edges);
        let what = format!("case {number} ({}, {} cards)", case.kind, case.cards.len());

        let costs = Costs::of(&graph);
        assert_eq!(costs.turn, case.turn_cost, "{what}: price of a turn");
        assert_eq!(costs.crossing, case.crossing_cost, "{what}: price of a crossing");
        assert_eq!(costs.non_center_port, case.non_center_port_cost, "{what}: price of a side port");

        let ovg = ovg::build(&graph);
        let mut router = Router::new(&graph, &ovg);
        for theirs in &case.routes {
            let what = format!("{what}, link {} {:?}", theirs.link, case.links[theirs.link]);
            assert!(!theirs.failed, "{what}: the reference has no failed search");
            let (found, distance) = router.search(theirs.link).unwrap_or_else(|error| panic!("{what}: {error:?}"));
            let points = found.points.clone();
            assert_eq!(Some(corners(&points)), theirs.corners, "{what}: corners");
            assert_eq!(distance, theirs.distance, "{what}: price");
            assert_eq!(points.len(), theirs.points, "{what}: number of points");
            let text: Vec<String> = points.iter().map(|p| format!("{},{}", q(p.x), q(p.y))).collect();
            assert_eq!(fnv1a(&text.join(" ")), theirs.hash, "{what}: the points differ somewhere");
            router.add_route(found).unwrap();
            routes_checked += 1;
        }
    }
    assert_eq!(routes_checked, 1955);
}
