// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.

//! A page to look at what the engine does today: diagrams as they were placed by hand, and as
//! the placement stages arrange them. Nothing in the editor uses the placement yet.
//!
//!   mkdir -p target/demo
//!   cargo run --release --example arrange_demo examples/data/bookshop.json target/demo/index.html
//!
//! and serve `target/demo` with any static web server, for example
//! `python3 -m http.server 5180 --directory target/demo`.
//! `examples/data/bookshop.json` holds the four views of a bookshop example, written out on
//! 2026-11-23.
//!
//! The input is a list of diagrams: `{id, level, cards: [{id, name, kind, x, y}], links: [{from,
//! to, label}]}`, every card 220 by 120. The page shows each one twice. The connectors are laid
//! by the first routing stage of TALA's pipeline as ported for plain cards (`route_graph`), so
//! where TALA finds trees, a hierarchy or a cluster they are not TALA's yet.

use tidy_layout::align::align;
use tidy_layout::balance::balance_symmetry;
use tidy_layout::cluster_stage::optimize_clusters;
use tidy_layout::equidistance::equidistance;
use tidy_layout::gaps::normalize_gaps;
use tidy_layout::graph::{Edge, Graph, Node, Point};
use tidy_layout::pack::pack;
use tidy_layout::place::{pad, place_with_trees, prescale};
use tidy_layout::routing::coordinator::route_graph;
use tidy_layout::swap::{swap, transpose_all};
use tidy_layout::trees::find_trees;
use serde::Deserialize;
use std::fmt::Write;

#[derive(Deserialize)]
struct Card {
    id: String,
    name: String,
    #[serde(default)]
    kind: String,
    x: f64,
    y: f64,
}

#[derive(Deserialize)]
struct Link {
    from: String,
    to: String,
    #[serde(default)]
    label: String,
}

#[derive(Deserialize)]
struct Diagram {
    id: String,
    level: String,
    cards: Vec<Card>,
    links: Vec<Link>,
}

const CARD: (f64, f64) = (220.0, 120.0);

/// The room a label needs, roughly as the editor measures it.
fn label_room(label: &str) -> (f64, f64) {
    if label.is_empty() { (0.0, 0.0) } else { (label.chars().count() as f64 * 7.0 + 12.0, 26.0) }
}

/// Every stage of the placement that is ported, with TALA's first seed.
fn arranged(given: &Graph) -> Option<Vec<Node>> {
    let mut whole = given.clone();
    prescale(&mut whole);
    let found = find_trees(&whole);
    let mut placed = place_with_trees(&mut whole, &found, 1).ok()?;
    swap(&mut placed);
    transpose_all(&mut placed);
    align(&mut placed);
    if normalize_gaps(&mut placed) {
        align(&mut placed);
    }
    if optimize_clusters(&mut placed) {
        align(&mut placed);
    }
    balance_symmetry(&mut placed);
    if equidistance(&mut placed) {
        align(&mut placed);
    }
    pack(&mut placed);
    pad(&mut placed);
    let places = placed.places(given.nodes.len());
    // The stages make some cards larger: one with many connections, and the cards of a cluster.
    let mut sizes: Vec<(f64, f64)> = whole.nodes.iter().map(|node| (node.width, node.height)).collect();
    for (node, card) in placed.cards.iter().enumerate() {
        if let Some(card) = card {
            sizes[*card] = (placed.graph.nodes[node].width, placed.graph.nodes[node].height);
        }
    }
    for (cluster, vessel) in placed.clusters.iter().zip(&placed.graph.vessels) {
        for member in &cluster.members {
            sizes[*member] = (vessel.width, vessel.height);
        }
    }
    Some(places.iter().zip(sizes).map(|(at, (width, height))| Node { top_left: *at, width, height }).collect())
}

/// The connectors around the cards; straight lines where the router gives up.
fn routes(nodes: &[Node], edges: &[Edge]) -> (Vec<Vec<Point>>, bool) {
    let graph = Graph::new(nodes.to_vec(), edges.to_vec());
    match std::panic::catch_unwind(|| route_graph(&graph).ok()) {
        Ok(Some(routes)) if routes.iter().all(|route| route.len() >= 2) => (routes, true),
        _ => (edges.iter().map(|edge| vec![nodes[edge.from].center(), nodes[edge.to].center()]).collect(), false),
    }
}

fn escape(text: &str) -> String {
    text.replace('&', "&amp;").replace('<', "&lt;").replace('>', "&gt;")
}

fn svg(diagram: &Diagram, nodes: &[Node], edges: &[Edge]) -> String {
    let (lines, routed) = routes(nodes, edges);
    let left = nodes.iter().map(|node| node.top_left.x).fold(f64::INFINITY, f64::min) - 40.0;
    let top = nodes.iter().map(|node| node.top_left.y).fold(f64::INFINITY, f64::min) - 40.0;
    let right = nodes.iter().map(Node::right).fold(f64::NEG_INFINITY, f64::max) + 40.0;
    let bottom = nodes.iter().map(Node::bottom).fold(f64::NEG_INFINITY, f64::max) + 40.0;
    let mut out = String::new();
    write!(out, r#"<svg viewBox="{left} {top} {} {}" role="img">"#, right - left, bottom - top).unwrap();
    for (line, link) in lines.iter().zip(&diagram.links) {
        let points: Vec<String> = line.iter().map(|point| format!("{},{}", point.x, point.y)).collect();
        write!(out, r#"<polyline class="link" points="{}" marker-end="url(#head)"/>"#, points.join(" ")).unwrap();
        if !link.label.is_empty() {
            // On the longest piece of the line.
            let piece = line.windows(2).max_by(|a, b| {
                let length = |ends: &&[Point]| (ends[0].x - ends[1].x).abs() + (ends[0].y - ends[1].y).abs();
                length(a).total_cmp(&length(b))
            });
            if let Some(piece) = piece {
                write!(out, r#"<text class="label" x="{}" y="{}">{}</text>"#, (piece[0].x + piece[1].x) / 2.0, (piece[0].y + piece[1].y) / 2.0 - 5.0, escape(&link.label)).unwrap();
            }
        }
    }
    for (node, card) in nodes.iter().zip(&diagram.cards) {
        write!(
            out,
            r#"<rect class="card {}" x="{}" y="{}" width="{}" height="{}" rx="10"/><text class="name" x="{}" y="{}">{}</text>"#,
            escape(&card.kind),
            node.top_left.x,
            node.top_left.y,
            node.width,
            node.height,
            node.center().x,
            node.center().y + 5.0,
            escape(&card.name)
        )
        .unwrap();
    }
    out.push_str("</svg>");
    if !routed {
        out.push_str(r#"<p class="note">The router refused these places (cards overlap or touch): straight lines.</p>"#);
    }
    out
}

fn main() {
    let arguments: Vec<String> = std::env::args().collect();
    let diagrams: Vec<Diagram> = serde_json::from_str(&std::fs::read_to_string(&arguments[1]).unwrap()).unwrap();
    std::panic::set_hook(Box::new(|_| {}));
    let mut page = String::from(
        r#"<!doctype html><html lang="en"><head><meta charset="utf-8"><meta name="viewport" content="width=device-width, initial-scale=1"><title>Arrange demo</title><style>
:root { --ground: #f6f7f9; --panel: #ffffff; --ink: #1c2330; --quiet: #5b6676; --line: #5b6676; --card: #dbe7fb; --edge: #3b6fd4; --person: #e3f3e1; --person-edge: #3f8f4a; }
@media (prefers-color-scheme: dark) { :root { --ground: #12151b; --panel: #1b2029; --ink: #e8ecf2; --quiet: #9aa6b6; --line: #9aa6b6; --card: #24344f; --edge: #6f9cf0; --person: #213a26; --person-edge: #6cc279; } }
body { margin: 0; padding: 16px; background: var(--ground); color: var(--ink); font: 15px/1.45 system-ui, sans-serif; }
h1 { font-size: 22px; margin: 0 0 4px; } h2 { font-size: 17px; margin: 28px 0 8px; } h3 { font-size: 13px; font-weight: 600; color: var(--quiet); margin: 0 0 6px; text-transform: uppercase; letter-spacing: .04em; }
p { max-width: 70ch; margin: 4px 0; color: var(--quiet); } .note { font-size: 13px; }
.pair { display: grid; grid-template-columns: repeat(auto-fit, minmax(320px, 1fr)); gap: 12px; }
.panel { background: var(--panel); border-radius: 10px; padding: 10px; } svg { width: 100%; height: auto; display: block; }
.card { fill: var(--card); stroke: var(--edge); stroke-width: 2; } .card.person { fill: var(--person); stroke: var(--person-edge); }
.name { fill: var(--ink); font: 600 17px system-ui, sans-serif; text-anchor: middle; } .label { fill: var(--quiet); font: 13px system-ui, sans-serif; text-anchor: middle; paint-order: stroke; stroke: var(--panel); stroke-width: 4; }
.link { fill: none; stroke: var(--line); stroke-width: 2; stroke-linejoin: round; } #head path { fill: var(--line); }
</style></head><body><svg width="0" height="0" style="position:absolute"><defs><marker id="head" viewBox="0 0 10 10" refX="9" refY="5" markerWidth="7" markerHeight="7" orient="auto-start-reverse"><path d="M0,0 L10,5 L0,10 z"/></marker></defs></svg>
<h1>Arrange demo</h1><p>What the layout engine does today, outside the editor. Left: the example as it was placed by hand. Right: the same cards arranged by the ported placement stages (TALA's first seed), connectors by the first routing stage. The editor has no "Arrange" yet.</p>"#,
    );
    for diagram in &diagrams {
        let index = |id: &str| diagram.cards.iter().position(|card| card.id == id);
        let edges: Vec<Edge> = diagram
            .links
            .iter()
            .filter_map(|link| {
                let (width, height) = label_room(&link.label);
                Some(Edge { from: index(&link.from)?, to: index(&link.to)?, min_width: width, min_height: height })
            })
            .collect();
        if edges.len() != diagram.links.len() || diagram.cards.is_empty() {
            continue;
        }
        let by_hand: Vec<Node> = diagram.cards.iter().map(|card| Node { top_left: Point { x: card.x, y: card.y }, width: CARD.0, height: CARD.1 }).collect();
        let given = Graph::new(by_hand.clone(), edges.clone());
        write!(page, "<h2>{} ({}): {} cards, {} connections</h2><div class=\"pair\">", escape(&diagram.id), escape(&diagram.level), diagram.cards.len(), edges.len()).unwrap();
        write!(page, "<div class=\"panel\"><h3>By hand</h3>{}</div>", svg(diagram, &by_hand, &edges)).unwrap();
        match std::panic::catch_unwind(|| arranged(&given)) {
            Ok(Some(nodes)) => write!(page, "<div class=\"panel\"><h3>Arranged by the engine</h3>{}</div>", svg(diagram, &nodes, &edges)).unwrap(),
            _ => page.push_str("<div class=\"panel\"><h3>Arranged by the engine</h3><p>The placement gave up on this diagram.</p></div>"),
        }
        page.push_str("</div>");
    }
    page.push_str("</body></html>");
    std::fs::write(&arguments[2], page).unwrap();
}
