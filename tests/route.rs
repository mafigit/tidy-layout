// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.

//! What the editor relies on when it asks how the connectors run.

use tidy_layout::api::{Position, RouteRequest, route, route_json};
use serde_json::{Value, json};

fn request(value: Value) -> RouteRequest {
    serde_json::from_value(value).unwrap()
}

fn card(id: &str, x: f64, y: f64) -> Value {
    json!({ "id": id, "x": x, "y": y, "width": 220, "height": 120 })
}

fn points(route: &[Position]) -> Vec<(f64, f64)> {
    route.iter().map(|p| (p.x, p.y)).collect()
}

/// The piece from `a` to `b` runs through the inside of the card at `x`, `y` (220 by 120): some
/// point of it, taken every pixel or so, lies more than a pixel inside.
fn runs_inside(a: (f64, f64), b: (f64, f64), x: f64, y: f64) -> bool {
    let steps = ((b.0 - a.0).abs().max((b.1 - a.1).abs())).ceil().max(1.0) as usize;
    (0..=steps).any(|step| {
        let part = step as f64 / steps as f64;
        let (px, py) = (a.0 + (b.0 - a.0) * part, a.1 + (b.1 - a.1) * part);
        px > x + 1.0 && px < x + 219.0 && py > y + 1.0 && py < y + 119.0
    })
}

#[test]
fn two_cards_side_by_side_are_joined_by_a_straight_line_between_their_middles() {
    let answer = route(&request(json!({
        "cards": [card("a", 0.0, 0.0), card("b", 600.0, 0.0)],
        "connections": [{ "from": "a", "to": "b" }],
    })))
    .unwrap();
    assert_eq!(points(&answer.routes[0]), [(220.0, 60.0), (600.0, 60.0)]);
}

#[test]
fn a_connector_goes_around_a_card_that_sits_between_its_ends() {
    let answer = route(&request(json!({
        "cards": [card("a", 0.0, 0.0), card("between", 300.0, 0.0), card("c", 600.0, 0.0)],
        "connections": [{ "from": "a", "to": "c" }],
    })))
    .unwrap();
    let route = points(&answer.routes[0]);
    assert!(route.len() > 2, "it needs bends: {route:?}");
    for piece in route.windows(2) {
        assert!(!runs_inside(piece[0], piece[1], 300.0, 0.0), "{route:?} runs through the card between");
        assert!(piece[0].0 == piece[1].0 || piece[0].1 == piece[1].1, "{route:?} has a slanted piece");
    }
}

fn is_slanted(route: &[(f64, f64)]) -> bool {
    route.windows(2).any(|piece| piece[0].0 != piece[1].0 && piece[0].1 != piece[1].1)
}

/// TALA would join these two with one slanted line, 5 off upright: cheaper than two bends.
#[test]
fn a_card_below_and_a_little_beside_another_gets_a_connector_with_bends_not_a_slanted_line() {
    let cards = json!([card("upper", 280.0, 320.0), card("lower", 220.0, 540.0)]);
    let up = route(&request(json!({ "cards": cards, "connections": [{ "from": "lower", "to": "upper" }] }))).unwrap();
    assert_eq!(points(&up.routes[0]), [(330.0, 540.0), (330.0, 490.0), (415.0, 490.0), (415.0, 440.0)]);
    let down = route(&request(json!({ "cards": cards, "connections": [{ "from": "upper", "to": "lower" }] }))).unwrap();
    assert!(!is_slanted(&points(&down.routes[0])) && down.routes[0].len() == 4, "{:?}", down.routes[0]);
}

/// 40 apart there is no room for a bend between two cards. TALA joins these with a slanted line
/// between two ports; here the line is upright, where both cards have a border above each other.
#[test]
fn cards_close_together_and_out_of_line_get_no_slanted_line_either() {
    let answer = route(&request(json!({
        "cards": [card("lower", 340.0, 580.0), card("upper", 360.0, 420.0)],
        "connections": [{ "from": "upper", "to": "lower" }],
    })))
    .unwrap();
    assert_eq!(points(&answer.routes[0]), [(460.0, 540.0), (460.0, 580.0)]);
}

#[test]
fn cards_close_together_and_in_line_keep_their_straight_line() {
    let beside = route(&request(json!({
        "cards": [card("a", 0.0, 0.0), card("b", 260.0, 0.0)],
        "connections": [{ "from": "a", "to": "b" }],
    })))
    .unwrap();
    assert_eq!(points(&beside.routes[0]), [(220.0, 60.0), (260.0, 60.0)]);
    let below = route(&request(json!({
        "cards": [card("a", 0.0, 0.0), card("b", 0.0, 160.0)],
        "connections": [{ "from": "a", "to": "b" }],
    })))
    .unwrap();
    assert_eq!(points(&below.routes[0]), [(110.0, 120.0), (110.0, 160.0)]);
}

/// The search leaves a jog here. TALA takes the line instead, and so does the editor.
#[test]
fn an_upright_or_level_line_still_replaces_a_route_with_a_jog() {
    let level = route(&request(json!({
        "cards": [card("a", 380.0, 240.0), card("b", 660.0, 300.0)],
        "connections": [{ "from": "a", "to": "b" }],
    })))
    .unwrap();
    assert_eq!(points(&level.routes[0]), [(600.0, 330.0), (660.0, 330.0)]);
    // Two connections between cards 25 apart: without the lines they would swap places.
    let upright = route(&request(json!({
        "cards": [card("a", 570.0, 195.0), card("b", 625.0, 340.0)],
        "connections": [{ "from": "b", "to": "a" }, { "from": "b", "to": "a" }],
    })))
    .unwrap();
    assert_eq!(points(&upright.routes[0]), [(680.0, 340.0), (680.0, 315.0)]);
    assert_eq!(points(&upright.routes[1]), [(735.0, 340.0), (735.0, 315.0)]);
}

#[test]
fn routes_come_back_in_the_order_of_the_connections() {
    let answer = route(&request(json!({
        "cards": [card("a", 0.0, 0.0), card("b", 600.0, 0.0), card("c", 0.0, 400.0)],
        "connections": [{ "from": "c", "to": "a" }, { "from": "a", "to": "b" }],
    })))
    .unwrap();
    assert_eq!(points(&answer.routes[0]), [(110.0, 400.0), (110.0, 120.0)]);
    assert_eq!(points(&answer.routes[1]), [(220.0, 60.0), (600.0, 60.0)]);
}

#[test]
fn a_connection_from_a_card_to_itself_gets_no_route() {
    let answer = route(&request(json!({
        "cards": [card("a", 0.0, 0.0), card("b", 600.0, 0.0)],
        "connections": [{ "from": "a", "to": "a" }, { "from": "a", "to": "b" }],
    })))
    .unwrap();
    assert!(answer.routes[0].is_empty());
    assert_eq!(answer.routes[1].len(), 2);
    assert_eq!(answer.labels, [None, None]);
}

#[test]
fn a_label_gets_a_place_beside_its_connector_clear_of_the_cards() {
    let answer = route(&request(json!({
        "cards": [card("a", 0.0, 0.0), card("b", 600.0, 0.0), card("c", 0.0, 400.0)],
        "connections": [
            { "from": "a", "to": "b", "label": { "width": 150, "height": 22 } },
            { "from": "c", "to": "a" },
        ],
    })))
    .unwrap();
    // The connector runs level at y = 60 from x = 220 to x = 600. The label sits above its
    // middle, 6.5 off the line: half the line's thickness and the padding.
    assert_eq!(answer.labels[0], Some(Position { x: 335.0, y: 32.0 }));
    // A connection without a label has no place for one.
    assert_eq!(answer.labels[1], None);
}

#[test]
fn labels_keep_clear_of_each_other() {
    // Two connectors side by side, 40 apart, each with a label wider than that.
    let answer = route(&request(json!({
        "cards": [card("a", 0.0, 0.0), card("b", 0.0, 400.0)],
        "connections": [
            { "from": "a", "to": "b", "label": { "width": 120, "height": 22 } },
            { "from": "a", "to": "b", "label": { "width": 120, "height": 22 } },
        ],
    })))
    .unwrap();
    let (first, second) = (answer.labels[0].unwrap(), answer.labels[1].unwrap());
    let apart = (first.x - second.x).abs() >= 120.0 || (first.y - second.y).abs() >= 22.0;
    assert!(apart, "the labels at {first:?} and {second:?} lie on each other");
}

#[test]
fn a_diagram_without_connections_has_no_routes() {
    let answer = route(&request(json!({ "cards": [card("a", 0.0, 0.0)], "connections": [] }))).unwrap();
    assert!(answer.routes.is_empty());
}

#[test]
fn cards_that_overlap_or_touch_are_refused() {
    for x in [100.0, 220.0] {
        let error = route(&request(json!({
            "cards": [card("a", 0.0, 0.0), card("b", x, 0.0)],
            "connections": [{ "from": "a", "to": "b" }],
        })))
        .unwrap_err();
        assert_eq!(error, "cards a and b overlap or touch");
    }
}

#[test]
fn a_bad_request_is_an_error_not_a_crash() {
    assert_eq!(
        route_json(r#"{"cards":[],"connections":[{"from":"x","to":"y"}]}"#),
        r#"{"error":"unknown card: x"}"#
    );
    assert!(route_json("nonsense").starts_with(r#"{"error":"not a route request"#));
    let nan = r#"{"cards":[{"id":"a","x":0,"y":0,"width":0,"height":120}],"connections":[]}"#;
    assert_eq!(route_json(nan), r#"{"error":"card a needs a finite position and a positive size"}"#);
}

/// xorshift64.
struct Random(u64);

impl Random {
    fn below(&mut self, limit: u64) -> u64 {
        self.0 ^= self.0 << 13;
        self.0 ^= self.0 >> 7;
        self.0 ^= self.0 << 17;
        self.0 % limit
    }
}

/// Whatever the layout: every route starts on the border of its first card, ends on the border of
/// its second and runs through the inside of no card. Every piece is upright or level.
#[test]
fn every_route_joins_its_cards_and_stays_out_of_all_cards() {
    let mut random = Random(20261121);
    let (mut routes_checked, mut lines) = (0, 0);
    for round in 0..600 {
        // The editor keeps 20 between cards, and 60 between cards that are connected when they
        // are placed. Every second layout has 60 between all cards.
        let gap = if round % 2 == 0 { 60.0 } else { 20.0 };
        let mut places: Vec<(f64, f64)> = Vec::new();
        let wanted = 2 + random.below(8) as usize;
        let mut tries = 0;
        while places.len() < wanted && tries < 400 {
            tries += 1;
            let place = (20.0 * random.below(70) as f64, 20.0 * random.below(45) as f64);
            let free = places.iter().all(|other: &(f64, f64)| {
                (place.0 - other.0).abs() >= 220.0 + gap || (place.1 - other.1).abs() >= 120.0 + gap
            });
            if free {
                places.push(place);
            }
        }
        let cards: Vec<Value> = places.iter().enumerate().map(|(n, p)| card(&n.to_string(), p.0, p.1)).collect();
        let mut connections: Vec<(usize, usize)> = Vec::new();
        for _ in 0..random.below(3 * places.len() as u64) {
            let (from, to) = (random.below(places.len() as u64) as usize, random.below(places.len() as u64) as usize);
            if from != to {
                connections.push((from, to));
            }
        }
        let wires: Vec<Value> =
            connections.iter().map(|(from, to)| json!({ "from": from.to_string(), "to": to.to_string() })).collect();
        let answer = route(&request(json!({ "cards": cards, "connections": wires }))).unwrap();
        assert_eq!(answer.routes.len(), connections.len());
        // No connection of this test carries a label.
        assert!(answer.labels.iter().all(Option::is_none) && answer.labels.len() == connections.len());

        for (route, (from, to)) in answer.routes.iter().zip(&connections) {
            let route = points(route);
            let what = format!("{route:?} from card at {:?} to card at {:?}, cards at {places:?}", places[*from], places[*to]);
            assert!(route.len() >= 2, "{what}");
            let on_border = |point: (f64, f64), card: (f64, f64)| {
                let (dx, dy) = (point.0 - card.0, point.1 - card.1);
                ((dx == 0.0 || dx == 220.0) && (0.0..=120.0).contains(&dy))
                    || ((dy == 0.0 || dy == 120.0) && (0.0..=220.0).contains(&dx))
            };
            assert!(on_border(route[0], places[*from]), "does not start on its card: {what}");
            assert!(on_border(route[route.len() - 1], places[*to]), "does not end on its card: {what}");
            for piece in route.windows(2) {
                assert!(piece[0].0 == piece[1].0 || piece[0].1 == piece[1].1, "slanted piece: {what}");
                for place in &places {
                    assert!(!runs_inside(piece[0], piece[1], place.0, place.1), "through the card at {place:?}: {what}");
                }
            }
            routes_checked += 1;
            lines += usize::from(route.len() == 2);
        }
    }
    assert_eq!((routes_checked, lines), (3758, 1045));
}

/// What lies wrong with the labels of an answer, in words: a label on a card, on another label,
/// or crossed by the line of a connection (its own or another). A pixel of overlap does not count.
fn label_trouble(request: &RouteRequest, answer: &tidy_layout::api::RouteResponse) -> Vec<String> {
    struct Rect(f64, f64, f64, f64);
    let overlap = |a: &Rect, b: &Rect| a.0 < b.2 - 1.0 && b.0 < a.2 - 1.0 && a.1 < b.3 - 1.0 && b.1 < a.3 - 1.0;
    let boxes: Vec<Option<Rect>> = answer
        .labels
        .iter()
        .zip(&request.connections)
        .map(|(place, connection)| {
            let (place, size) = ((*place)?, connection.label?);
            Some(Rect(place.x, place.y, place.x + size.width, place.y + size.height))
        })
        .collect();
    let mut found = Vec::new();
    for (i, label) in boxes.iter().enumerate() {
        let Some(label) = label else { continue };
        for card in &request.cards {
            if overlap(label, &Rect(card.x, card.y, card.x + card.width, card.y + card.height)) {
                found.push(format!("label {i} lies on card {}", card.id));
            }
        }
        for (j, other) in boxes.iter().enumerate().skip(i + 1) {
            if other.as_ref().is_some_and(|other| overlap(label, other)) {
                found.push(format!("label {i} lies on label {j}"));
            }
        }
        for (j, route) in answer.routes.iter().enumerate() {
            let crossed = route.windows(2).any(|piece| {
                let line = Rect(
                    piece[0].x.min(piece[1].x),
                    piece[0].y.min(piece[1].y),
                    piece[0].x.max(piece[1].x),
                    piece[0].y.max(piece[1].y),
                );
                // A line has no width: it crosses when it passes more than a pixel inside.
                line.0 < label.2 - 1.0 && line.2 > label.0 + 1.0 && line.1 < label.3 - 1.0 && line.3 > label.1 + 1.0
            });
            if crossed {
                found.push(format!("label {i} is crossed by the line of connection {j}"));
            }
        }
    }
    found
}

/// Where the ported placement alone puts the labels of a request: TALA's places, before `route`
/// moves any. For requests without a connection from a card to itself.
fn ported_places(request: &RouteRequest) -> Vec<Option<Position>> {
    use tidy_layout::graph::{Edge, Graph, Node, Point};
    use tidy_layout::routing::coordinator::route_edges_with_labels;
    use tidy_layout::routing::labels::LabelSize;
    let index = |id: &str| request.cards.iter().position(|card| card.id == id).unwrap();
    let nodes = request.cards.iter().map(|c| Node { top_left: Point { x: c.x, y: c.y }, width: c.width, height: c.height }).collect();
    let edges = request.connections.iter().map(|c| Edge::plain(index(&c.from), index(&c.to))).collect();
    let sizes: Vec<Option<LabelSize>> =
        request.connections.iter().map(|c| c.label.map(|label| LabelSize { width: label.width, height: label.height })).collect();
    let graph = Graph { slanted_lines: false, ..Graph::new(nodes, edges) };
    let routing = route_edges_with_labels(&graph, &sizes).unwrap();
    routing.labels.iter().map(|place| place.map(|place| Position { x: place.top_left.x, y: place.top_left.y })).collect()
}

/// Ours, where TALA differs: three connections run side by side, 30 apart, and TALA's placement
/// leaves the label of the middle one on its own line. `route` moves it up beside the line.
#[test]
fn a_label_under_a_line_moves_to_the_nearest_free_place_beside_its_own_line() {
    let request = request(json!({
        "cards": [card("a", 1220.0, 160.0), card("b", 400.0, 160.0), card("c", 0.0, 220.0)],
        "connections": [
            { "from": "a", "to": "b", "label": { "width": 159, "height": 22 } },
            { "from": "b", "to": "a", "label": { "width": 128, "height": 22 } },
            { "from": "a", "to": "b", "label": { "width": 83, "height": 22 } },
        ],
    }));
    let ported = ported_places(&request);
    // The middle connection runs level at y = 220; its label lay from 209 to 231.
    assert_eq!(ported[0], Some(Position { x: 841.0, y: 209.0 }));
    let answer = route(&request).unwrap();
    // Now it ends 6 above the line. The other two labels keep TALA's places.
    assert_eq!(answer.labels[0], Some(Position { x: 841.0, y: 192.0 }));
    assert_eq!(answer.labels[1..], ported[1..]);
    assert_eq!(label_trouble(&request, &answer), Vec::<String>::new());
}

/// A diagram of 23 cards as an editor had it. Three lines run upright 55 apart, and the label of
/// the middle one is 183 wide: no place beside its line is free, so it stays where TALA put it,
/// under the line of another connection. Every other label is clear.
#[test]
fn a_label_with_no_free_place_beside_its_line_stays_where_it_was() {
    let request: RouteRequest = serde_json::from_str(include_str!("fixtures/crossed-label.json")).unwrap();
    let answer = route(&request).unwrap();
    assert_eq!(answer.labels, ported_places(&request));
    assert_eq!(label_trouble(&request, &answer), vec!["label 6 is crossed by the line of connection 23".to_string()]);
}

/// Whatever the layout: a label is where TALA's placement put it, or it was under a line there
/// and is now clear of every line, card and other label. So moving never makes a drawing worse,
/// and over many layouts it clears a good part of the labels that were under a line.
#[test]
fn a_moved_label_is_always_clear_and_no_other_label_moves() {
    let mut random = Random(20261124);
    let (mut under_a_line, mut cleared) = (0, 0);
    for round in 0..400 {
        let gap = if round % 2 == 0 { 60.0 } else { 20.0 };
        let mut places: Vec<(f64, f64)> = Vec::new();
        let wanted = 3 + random.below(8) as usize;
        let mut tries = 0;
        while places.len() < wanted && tries < 400 {
            tries += 1;
            let place = (20.0 * random.below(70) as f64, 20.0 * random.below(45) as f64);
            if places.iter().all(|other| (place.0 - other.0).abs() >= 220.0 + gap || (place.1 - other.1).abs() >= 120.0 + gap) {
                places.push(place);
            }
        }
        let cards: Vec<Value> = places.iter().enumerate().map(|(n, p)| card(&n.to_string(), p.0, p.1)).collect();
        let mut wires: Vec<Value> = Vec::new();
        for _ in 0..random.below(2 * places.len() as u64) {
            let (from, to) = (random.below(places.len() as u64), random.below(places.len() as u64));
            if from != to {
                let width = 60 + random.below(180);
                wires.push(json!({ "from": from.to_string(), "to": to.to_string(), "label": { "width": width, "height": 22 } }));
            }
        }
        let request = request(json!({ "cards": cards, "connections": wires }));
        let answer = route(&request).unwrap();
        let ported = ported_places(&request);
        // The same answer with TALA's places: what was wrong before any label moved.
        let before = tidy_layout::api::RouteResponse { routes: answer.routes.clone(), labels: ported.clone() };
        let (was, is) = (label_trouble(&request, &before), label_trouble(&request, &answer));
        let crossed = |trouble: &[String], label: usize| trouble.iter().any(|line| line.starts_with(&format!("label {label} is crossed")));
        let any = |trouble: &[String], label: usize| {
            trouble.iter().any(|line| line.starts_with(&format!("label {label} ")) || line.ends_with(&format!("on label {label}")))
        };
        for label in 0..ported.len() {
            if crossed(&was, label) {
                under_a_line += 1;
            }
            if answer.labels[label] != ported[label] {
                assert!(crossed(&was, label), "label {label} moved though no line crossed it: {request_text}", request_text = json!({ "cards": cards, "connections": wires }));
                assert!(!any(&is, label), "label {label} moved to a place that is not clear: {is:?}");
                cleared += 1;
            }
        }
    }
    // Pinned: counted once, 2026-11-24. A change here means the search finds other places.
    assert_eq!((under_a_line, cleared), (144, 61));
}
