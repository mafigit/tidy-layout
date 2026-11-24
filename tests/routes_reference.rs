// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.

//! All connections of a layout must get TALA's routes: these tests replay layouts recorded from
//! TALA's own Go code (docs/reference.md).

use tidy_layout::graph::{Edge, Graph, Node, Point};
use tidy_layout::routing::coordinator::{FLAVORS, generate_routes, route_all};
use tidy_layout::routing::ovg;
use serde::Deserialize;

#[derive(Deserialize)]
struct Card {
    x: f64,
    y: f64,
    width: f64,
    height: f64,
}

#[derive(Deserialize)]
struct FlavorRoute {
    link: usize,
    points: usize,
    hash: String,
}

#[derive(Deserialize)]
struct Flavor {
    flavor: String,
    distance: f64,
    routes: Vec<FlavorRoute>,
}

#[derive(Deserialize)]
struct Case {
    kind: String,
    cards: Vec<Card>,
    links: Vec<[usize; 2]>,
    flavors: Vec<Flavor>,
    kept: Vec<Vec<[i64; 2]>>,
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

#[test]
fn all_connections_get_the_routes_tala_gives_them() {
    let cases: Vec<Case> = serde_json::from_str(include_str!("fixtures/routes.tala.json")).unwrap();
    assert_eq!(cases.len(), 244);
    let (mut routes_checked, mut kept_checked) = (0, 0);
    let mut winners = [0; 3];
    for (number, case) in cases.iter().enumerate() {
        let nodes = case
            .cards
            .iter()
            .map(|c| Node { top_left: Point { x: c.x, y: c.y }, width: c.width, height: c.height })
            .collect();
        let edges = case.links.iter().map(|[from, to]| Edge::plain(*from, *to)).collect();
        let graph = Graph::new(nodes, edges);
        let what = format!("case {number} ({}, {} cards)", case.kind, case.cards.len());
        if case.links.is_empty() {
            assert!(route_all(&graph).unwrap().is_empty(), "{what}");
            continue;
        }

        let ovg = ovg::build(&graph);
        // Which order TALA keeps: the first that is cheaper by more than a rounding error.
        let (mut cheapest, mut least) = (0, f64::INFINITY);
        for (index, (flavor, theirs)) in FLAVORS.iter().zip(&case.flavors).enumerate() {
            let what = format!("{what}, {}", theirs.flavor);
            assert_eq!(format!("{flavor:?}"), theirs.flavor);
            let ours = generate_routes(&graph, &ovg, *flavor).unwrap_or_else(|error| panic!("{what}: {error:?}"));
            assert_eq!(ours.routes.len(), theirs.routes.len(), "{what}: number of routes");
            for (position, (route, theirs)) in ours.routes.iter().zip(&theirs.routes).enumerate() {
                let what = format!("{what}, route {position} (link {} {:?})", theirs.link, case.links[theirs.link]);
                assert_eq!(route.edge, theirs.link, "{what}: order");
                let text: Vec<String> = route.points.iter().map(|p| format!("{},{}", q(p.x), q(p.y))).collect();
                assert_eq!(text.len(), theirs.points, "{what}: number of points: {text:?}");
                assert_eq!(fnv1a(&text.join(" ")), theirs.hash, "{what}: the points differ: {text:?}");
                routes_checked += 1;
            }
            assert_eq!(ours.distance, theirs.distance, "{what}: sum of prices");
            if (theirs.distance - least).abs() >= 0.0001 && theirs.distance < least {
                (cheapest, least) = (index, theirs.distance);
            }
        }
        winners[cheapest] += 1;

        let kept = route_all(&graph).unwrap();
        assert_eq!(kept.len(), case.kept.len(), "{what}");
        for (link, (ours, theirs)) in kept.iter().zip(&case.kept).enumerate() {
            let ours: Vec<[i64; 2]> = ours.iter().map(|p| [q(p.x), q(p.y)]).collect();
            assert_eq!(&ours, theirs, "{what}, link {link} {:?}: kept route", case.links[link]);
            kept_checked += 1;
        }
    }
    assert_eq!((routes_checked, kept_checked), (5286, 1762));
    // How often each order of routing is the one TALA keeps: all three must stay covered.
    assert_eq!(winners, [187, 29, 15]);
}
