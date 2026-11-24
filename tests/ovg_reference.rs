// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.

//! The visibility graph must be the one TALA builds: these tests replay graphs recorded from
//! TALA's own Go code (docs/reference.md).

use tidy_layout::graph::{Edge, Graph, Node, Point};
use tidy_layout::routing::ovg::{self, Ovg, Side};
use serde::Deserialize;

#[derive(Deserialize)]
struct Card {
    x: f64,
    y: f64,
    width: f64,
    height: f64,
}

#[derive(Deserialize)]
struct Case {
    kind: String,
    cards: Vec<Card>,
    links: Vec<[usize; 2]>,
    nodes: usize,
    edges: usize,
    tunnels: usize,
    near: usize,
    hash: String,
    dump: Option<Vec<String>>,
}

/// A coordinate in quarters, as the recorder writes it.
fn q(value: f64) -> i64 {
    let quarters = (value * 4.0) as i64;
    assert_eq!(quarters as f64, value * 4.0, "{value} is not a multiple of 0.25");
    quarters
}

/// The graph as text, in the form of the recorder (docs/recorders/ovg_test.go.txt).
fn dump(ovg: &Ovg) -> Vec<String> {
    let at = |id: usize| {
        let p = ovg.point(id);
        format!("{},{}", q(p.x), q(p.y))
    };
    let mut nodes = ovg.nodes.clone();
    nodes.sort_by(|a, b| {
        let (pa, pb) = (ovg.point(*a), ovg.point(*b));
        pa.x.total_cmp(&pb.x)
            .then(pa.y.total_cmp(&pb.y))
            .then(ovg.points[*a].is_node_center.cmp(&ovg.points[*b].is_node_center))
    });
    let mut lines = Vec::new();
    for id in nodes {
        let node = &ovg.points[id];
        let mut flags = String::new();
        if node.is_node_center {
            flags.push('c');
        }
        if node.is_tunnel {
            flags.push('t');
        }
        if flags.is_empty() {
            flags.push('-');
        }
        let mut ports: Vec<String> = node
            .port_roles
            .iter()
            .map(|(owner, role)| {
                let sides: String = [(Side::Top, 'T'), (Side::Right, 'R'), (Side::Bottom, 'B'), (Side::Left, 'L')]
                    .iter()
                    .filter(|(side, _)| role.sides.has(*side))
                    .map(|(_, letter)| *letter)
                    .collect();
                format!("{owner:03}:{sides}:{}", u8::from(role.is_center))
            })
            .collect();
        ports.sort();
        let near: Vec<String> = node.near_port.iter().map(|owner| format!("{owner:03}")).collect();
        let adjacent: Vec<String> = node.edges.iter().map(|edge| at(ovg.adjacent(id, *edge))).collect();
        let p = node.point;
        lines.push(format!(
            "N {} {} {flags} P {} K {} A {}",
            q(p.x),
            q(p.y),
            ports.join(","),
            near.join(","),
            adjacent.join(" ")
        ));
    }
    for edge in &ovg.edges {
        let (from, to) = (ovg.point(edge.from), ovg.point(edge.to));
        lines.push(format!("E {} {} {} {}", q(from.x), q(from.y), q(to.x), q(to.y)));
    }
    for (card, ports) in ovg.ports.iter().enumerate() {
        let ports: Vec<String> = ports.iter().map(|port| at(*port)).collect();
        lines.push(format!("O {card} {}", ports.join(" ")));
    }
    for (card, center) in ovg.centers.iter().enumerate() {
        lines.push(format!("C {card} {}", at(*center)));
    }
    lines
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

fn graph_of(case: &Case) -> Graph {
    let nodes = case
        .cards
        .iter()
        .map(|c| Node { top_left: Point { x: c.x, y: c.y }, width: c.width, height: c.height })
        .collect();
    let edges = case.links.iter().map(|[from, to]| Edge::plain(*from, *to)).collect();
    Graph::new(nodes, edges)
}

#[test]
fn visibility_graph_is_the_one_tala_builds() {
    let cases: Vec<Case> = serde_json::from_str(include_str!("fixtures/ovg.tala.json")).unwrap();
    assert_eq!(cases.len(), 480);
    let (mut with_dump, mut tunnels, mut near) = (0, 0, 0);
    for (number, case) in cases.iter().enumerate() {
        let ovg = ovg::build(&graph_of(case));
        let ours = dump(&ovg);
        let what = format!("case {number} ({}, {} cards)", case.kind, case.cards.len());
        if let Some(theirs) = &case.dump {
            with_dump += 1;
            for (line, (ours, theirs)) in ours.iter().zip(theirs).enumerate() {
                assert_eq!(ours, theirs, "{what}: line {line}");
            }
            assert_eq!(ours.len(), theirs.len(), "{what}: number of lines");
        }
        assert_eq!(ovg.nodes.len(), case.nodes, "{what}: points");
        assert_eq!(ovg.edges.len(), case.edges, "{what}: edges");
        let ours_tunnels = ovg.nodes.iter().filter(|id| ovg.points[**id].is_tunnel).count();
        assert_eq!(ours_tunnels, case.tunnels, "{what}: tunnel ends");
        let ours_near = ovg.nodes.iter().filter(|id| !ovg.points[**id].near_port.is_empty()).count();
        assert_eq!(ours_near, case.near, "{what}: points near a port");
        assert_eq!(fnv1a(&ours.join("\n")), case.hash, "{what}: the graph differs somewhere");
        tunnels += ours_tunnels;
        near += ours_near;
    }
    // The cases must keep covering what is easy to lose.
    assert_eq!(with_dump, 20);
    assert_eq!(tunnels, 2146);
    assert_eq!(near, 6900);
}

/// Every point's position in the list is its index, and every edge joins two listed points.
#[test]
fn visibility_graph_is_consistent() {
    let cases: Vec<Case> = serde_json::from_str(include_str!("fixtures/ovg.tala.json")).unwrap();
    for case in &cases {
        let ovg = ovg::build(&graph_of(case));
        for (position, id) in ovg.nodes.iter().enumerate() {
            assert_eq!(ovg.points[*id].index, position);
        }
        for edge in &ovg.edges {
            assert!(ovg.nodes.contains(&edge.from) && ovg.nodes.contains(&edge.to));
            let (from, to) = (ovg.point(edge.from), ovg.point(edge.to));
            assert_eq!(edge.distance, tidy_layout::routing::geometry::euclidean_distance(from.x, from.y, to.x, to.y));
        }
    }
}

#[derive(Deserialize)]
struct PassCase {
    card: Card,
    from: [f64; 2],
    to: [f64; 2],
    passes: bool,
}

/// A segment and a card: along a border and through the card count, touching a corner does not.
#[test]
fn passes_through_answers_like_tala() {
    let cases: Vec<PassCase> =
        serde_json::from_str(include_str!("fixtures/passes-through.tala.json")).unwrap();
    assert_eq!(cases.len(), 600);
    for (number, case) in cases.iter().enumerate() {
        let c = &case.card;
        let node = Node { top_left: Point { x: c.x, y: c.y }, width: c.width, height: c.height };
        let from = Point { x: case.from[0], y: case.from[1] };
        let to = Point { x: case.to[0], y: case.to[1] };
        assert_eq!(node.passes_through(from, to), case.passes, "case {number}: {from:?} to {to:?}");
    }
}
