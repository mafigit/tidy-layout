// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.

//! Compares the router with TALA, or with a changed copy of itself, on thousands of random
//! layouts (docs/reference.md, "Finding cases"). Ours, not ported.
//!
//!   probe <what> write <file> <count> <seed> [most cards]
//!                                      route random layouts, keep the answers
//!   probe <what> compare <file> <limit>   route them again; print the layouts that now differ
//!
//! <what> is `search` (one connection after another, slice 2), `routes` (all connections in the
//! three orders and the routes that are kept, slice 3), `steps` (the same with cards close
//! together, and the routes after each later step, slice 5) or `labels` (most connections carry
//! a label: the final routes and the place of every label, slice 6).
//!
//! The file can be given to TALA's probe (docs/recorders/probe_test.go.txt), which writes its own
//! answers in the same form: `compare` on that file then checks the port against TALA.

use tidy_layout::graph::{Edge, Graph, Node, Point};
use tidy_layout::routing::balance::balance;
use tidy_layout::routing::coordinator::{FLAVORS, generate_routes, route_all, route_edges_with_labels};
use tidy_layout::routing::labels::LabelSize;
use tidy_layout::routing::line::straighten;
use tidy_layout::routing::ovg;
use tidy_layout::routing::router::Router;
use serde::{Deserialize, Serialize};

#[derive(Serialize, Deserialize, Clone)]
struct Card {
    x: f64,
    y: f64,
    width: f64,
    height: f64,
}

#[derive(Serialize, Deserialize, Clone)]
struct Case {
    cards: Vec<Card>,
    links: Vec<[usize; 2]>,
    /// Per link the width and height of its label, 0 by 0 for none. Only in the `labels` probe.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    sizes: Vec<[f64; 2]>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    answers: Vec<String>,
}

/// xorshift64: enough randomness to scatter cards.
struct Random(u64);

impl Random {
    fn below(&mut self, limit: usize) -> usize {
        self.0 ^= self.0 << 13;
        self.0 ^= self.0 >> 7;
        self.0 ^= self.0 << 17;
        (self.0 % limit as u64) as usize
    }
}

fn keeps_gap(cards: &[Card], card: &Card, gap: f64) -> bool {
    cards.iter().all(|o| {
        !(card.x < o.x + o.width + gap
            && o.x < card.x + card.width + gap
            && card.y < o.y + o.height + gap
            && o.y < card.y + card.height + gap)
    })
}

/// The search does not handle two facing cards closer than 42 (TALA joins them with a straight
/// line instead), so such pairs get no link.
fn searchable(a: &Card, b: &Card) -> bool {
    let (a, b) = (node(a), node(b));
    use tidy_layout::routing::geometry::Orientation::*;
    match a.orientation(&b) {
        TopLeft | TopRight | BottomLeft | BottomRight => true,
        _ => {
            let distance = a.distance_to(&b);
            !(distance < 42.0 && distance > 20.0)
        }
    }
}

fn node(card: &Card) -> Node {
    Node { top_left: Point { x: card.x, y: card.y }, width: card.width, height: card.height }
}

fn random_case(random: &mut Random, most_cards: usize, close_cards: bool) -> Case {
    const SIZES: [(f64, f64); 5] = [(220.0, 120.0), (220.0, 120.0), (220.0, 120.0), (160.0, 80.0), (64.0, 64.0)];
    let count = 2 + random.below(most_cards - 1);
    // Tight layouts make routes meet; a step of 1 puts lines a few pixels apart.
    let step = [20, 20, 10, 5, 1][random.below(5)] as f64;
    let span = (200 + 150 * count) as f64;
    let mut cards: Vec<Card> = Vec::new();
    let mut tries = 0;
    while cards.len() < count && tries < 500 {
        tries += 1;
        let (width, height) = SIZES[random.below(SIZES.len())];
        let card = Card {
            x: (random.below((span / step) as usize) as f64) * step,
            y: (random.below((span * 0.7 / step) as usize) as f64) * step,
            width,
            height,
        };
        if keeps_gap(&cards, &card, 20.0) {
            cards.push(card);
        }
    }
    let mut links = Vec::new();
    for _ in 0..(1 + random.below(4 * cards.len())) {
        let (a, b) = (random.below(cards.len()), random.below(cards.len()));
        if a != b && (close_cards || searchable(&cards[a], &cards[b])) {
            links.push([a, b]);
            // Doubled and opposite connections are where routes compete for ports.
            match random.below(6) {
                0 => links.push([a, b]),
                1 => links.push([b, a]),
                _ => {}
            }
        }
    }
    Case { cards, links, sizes: Vec::new(), answers: Vec::new() }
}

/// Gives most links a label: one line of text of some width, sometimes two lines.
fn with_labels(mut case: Case, random: &mut Random) -> Case {
    case.sizes = case
        .links
        .iter()
        .map(|_| {
            if random.below(10) < 7 {
                [(30 + random.below(290)) as f64, if random.below(8) == 0 { 38.0 } else { 22.0 }]
            } else {
                [0.0, 0.0]
            }
        })
        .collect();
    case
}

fn bits(points: &[Point]) -> String {
    let text: Vec<String> = points.iter().map(|p| format!("{:x},{:x}", p.x.to_bits(), p.y.to_bits())).collect();
    text.join(" ")
}

/// What routing with labels answers: per link its final route, then per link the place of its
/// label (position, how far along, corner) or "-".
fn label_answers(case: &Case) -> Vec<String> {
    let graph = graph_of(case);
    if graph.edges.is_empty() {
        return Vec::new();
    }
    let sizes: Vec<Option<LabelSize>> = case
        .sizes
        .iter()
        .map(|[width, height]| (*width > 0.0).then_some(LabelSize { width: *width, height: *height }))
        .collect();
    let Ok(routing) = route_edges_with_labels(&graph, &sizes) else {
        return vec!["failed".into()];
    };
    let (routes, places) = (routing.routes, routing.labels);
    let mut found: Vec<String> = routes.iter().map(|points| bits(points)).collect();
    found.extend(places.iter().map(|place| match place {
        None => "-".to_string(),
        Some(place) => format!(
            "{} {:x} {:x},{:x}",
            place.position as u8,
            place.percentage.to_bits(),
            place.top_left.x.to_bits(),
            place.top_left.y.to_bits()
        ),
    }));
    found
}

fn quarters(points: &[Point]) -> String {
    let text: Vec<String> = points.iter().map(|p| format!("{},{}", (p.x * 4.0) as i64, (p.y * 4.0) as i64)).collect();
    text.join(" ")
}

fn graph_of(case: &Case) -> Graph {
    let nodes = case.cards.iter().map(node).collect();
    let edges = case.links.iter().map(|[from, to]| Edge::plain(*from, *to)).collect();
    Graph::new(nodes, edges)
}

/// What routing all connections answers: per order its routes and the sum of their prices, then
/// the kept route of every link.
fn route_answers(case: &Case) -> Vec<String> {
    let graph = graph_of(case);
    if graph.edges.is_empty() {
        return Vec::new();
    }
    let ovg = ovg::build(&graph);
    let mut found = Vec::new();
    for flavor in FLAVORS {
        match generate_routes(&graph, &ovg, flavor) {
            Err(_) => found.push("failed".into()),
            Ok(generated) => {
                let routes: Vec<String> = generated
                    .routes
                    .iter()
                    .map(|route| format!("{}:{}", route.edge, quarters(&route.points)))
                    .collect();
                found.push(format!("{:016x} {}", generated.distance.to_bits(), routes.join(";")));
            }
        }
    }
    match route_all(&graph) {
        Err(_) => found.push("failed".into()),
        Ok(kept) => found.extend(kept.iter().map(|points| quarters(points))),
    }
    found
}

/// What the steps after the search answer: the sum of prices per order (cards close together get
/// a straight line during the search), then per link its route after the search, after the
/// straight lines and after all steps.
fn step_answers(case: &Case) -> Vec<String> {
    let graph = graph_of(case);
    if graph.edges.is_empty() {
        return Vec::new();
    }
    let ovg = ovg::build(&graph);
    let mut found = Vec::new();
    for flavor in FLAVORS {
        match generate_routes(&graph, &ovg, flavor) {
            Err(_) => found.push("failed".into()),
            Ok(generated) => found.push(format!("{:016x}", generated.distance.to_bits())),
        }
    }
    let Ok(mut routes) = route_all(&graph) else {
        found.push("failed".into());
        return found;
    };
    found.extend(routes.iter().map(|points| quarters(points)));
    straighten(&graph, &mut routes);
    found.extend(routes.iter().map(|points| quarters(points)));
    // Spreading the routes can leave a point anywhere: every bit of it is compared.
    balance(&graph, &mut routes);
    found.extend(routes.iter().map(|points| bits(points)));
    found
}

/// What the search answers for every link in turn: price and points, or the error.
fn search_answers(case: &Case) -> Vec<String> {
    let graph = graph_of(case);
    let ovg = ovg::build(&graph);
    let mut router = Router::new(&graph, &ovg);
    let mut found = Vec::new();
    for link in 0..case.links.len() {
        match router.search(link) {
            Err(_) => found.push("failed".into()),
            Ok((route, price)) => {
                // The price bit for bit, the points in quarters: TALA's probe writes the same text.
                found.push(format!("{:016x} {}", price.to_bits(), quarters(&route.points)));
                if router.add_route(route).is_err() {
                    found.push("cannot add".into());
                    break;
                }
            }
        }
    }
    found
}

fn main() {
    let arguments: Vec<String> = std::env::args().collect();
    let what = arguments.get(1).map(String::as_str);
    let (answers, close_cards): (fn(&Case) -> Vec<String>, bool) = match what {
        Some("search") => (search_answers, false),
        Some("routes") => (route_answers, false),
        Some("steps") => (step_answers, true),
        Some("labels") => (label_answers, true),
        _ => return usage(),
    };
    match arguments.get(2).map(String::as_str) {
        Some("write") if arguments.len() >= 6 => {
            let count: usize = arguments[4].parse().unwrap();
            let mut random = Random(arguments[5].parse::<u64>().unwrap().max(1));
            let most_cards: usize = arguments.get(6).map_or(7, |most| most.parse().unwrap());
            let cases: Vec<Case> = (0..count)
                .map(|_| {
                    let mut case = random_case(&mut random, most_cards, close_cards);
                    if what == Some("labels") {
                        case = with_labels(case, &mut random);
                    }
                    case.answers = answers(&case);
                    case
                })
                .collect();
            std::fs::write(&arguments[3], serde_json::to_string(&cases).unwrap()).unwrap();
        }
        Some("compare") if arguments.len() >= 5 => {
            let limit: usize = arguments[4].parse().unwrap();
            let cases: Vec<Case> = serde_json::from_str(&std::fs::read_to_string(&arguments[3]).unwrap()).unwrap();
            let mut different: Vec<Case> = cases
                .into_iter()
                .filter(|case| answers(case) != case.answers)
                .map(|case| Case { answers: Vec::new(), ..case })
                .collect();
            // The smallest layouts are the easiest to read.
            different.sort_by_key(|case| case.cards.len() * 100 + case.links.len());
            different.truncate(limit);
            println!("{}", serde_json::to_string(&different).unwrap());
        }
        _ => usage(),
    }
}

fn usage() {
    eprintln!("usage: probe search|routes|steps|labels write <file> <count> <seed> [most cards]");
    eprintln!("       probe search|routes|steps|labels compare <file> <limit>");
}
