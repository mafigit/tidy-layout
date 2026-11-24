// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.

//! What the editor relies on when it asks the engine to arrange a diagram.

use tidy_layout::api::{ArrangeRequest, ArrangeResponse, ArrangedCard, CARD_GAP, arrange, arrange_json, pick_seed};
use tidy_layout::graph::{Edge, Graph, Node, Point};
use tidy_layout::pipeline::{SEEDS, layout_best};
use tidy_layout::score::Score;
use serde_json::{Value, json};

fn request(value: Value) -> ArrangeRequest {
    serde_json::from_value(value).unwrap()
}

fn card(id: &str, x: f64, y: f64) -> Value {
    json!({ "id": id, "x": x, "y": y, "width": 220, "height": 120 })
}

/// A small landscape: `a` with three connections to `b`, a chain, a label, a card alone, and
/// cards that start on each other.
fn landscape() -> Value {
    json!({
        "cards": [
            card("a", 100.0, 40.0), card("b", 100.0, 40.0), card("c", 500.0, 300.0),
            card("d", 900.0, 60.0), card("e", 120.0, 700.0), card("alone", 300.0, 300.0),
        ],
        "connections": [
            { "from": "a", "to": "b" }, { "from": "a", "to": "b" }, { "from": "b", "to": "a" },
            { "from": "a", "to": "c", "label": { "width": 90, "height": 22 } },
            { "from": "c", "to": "d" }, { "from": "c", "to": "e" },
        ],
    })
}

fn arranged(value: Value) -> ArrangeResponse {
    arrange(&request(value)).unwrap()
}

fn gap(a: &ArrangedCard, b: &ArrangedCard) -> f64 {
    let beside = (b.x - (a.x + a.width)).max(a.x - (b.x + b.width));
    let above = (b.y - (a.y + a.height)).max(a.y - (b.y + b.height));
    beside.max(above)
}

#[test]
fn the_answer_keeps_the_order_and_the_ids_of_the_request() {
    let answer = arranged(landscape());
    let ids: Vec<&str> = answer.cards.iter().map(|card| card.id.as_str()).collect();
    assert_eq!(ids, ["a", "b", "c", "d", "e", "alone"]);
}

#[test]
fn only_a_card_with_many_connections_grows() {
    let answer = arranged(landscape());
    let size = |id: &str| {
        let card = answer.cards.iter().find(|card| card.id == id).unwrap();
        (card.width, card.height)
    };
    // Three connections to one card: one side needs room for three, 4 times 40.
    assert_eq!(size("a"), (220.0, 160.0));
    assert_eq!(size("b"), (220.0, 160.0));
    for id in ["c", "d", "e", "alone"] {
        assert_eq!(size(id), (220.0, 120.0), "{id}");
    }
}

/// TALA puts cards that are alike in a row and gives them all the size of the largest; here
/// every card keeps its own.
#[test]
fn cards_of_a_cluster_keep_their_own_sizes() {
    // `big` and `small` are each connected to all four others: TALA stacks them as a cluster
    // and draws `small` 220 by 120 too.
    let mut cards = vec![card("big", 0.0, 0.0), json!({ "id": "small", "x": 0, "y": 200, "width": 180, "height": 80 })];
    let mut connections = Vec::new();
    for i in 0..4 {
        let id = format!("other{i}");
        cards.push(card(&id, 300.0 * i as f64, 400.0));
        connections.push(json!({ "from": "big", "to": id }));
        connections.push(json!({ "from": "small", "to": id }));
    }
    let answer = arranged(json!({ "cards": cards, "connections": connections }));
    let sizes: Vec<(f64, f64)> = answer.cards.iter().map(|card| (card.width, card.height)).collect();
    assert_eq!(sizes, [(220.0, 120.0), (180.0, 80.0), (220.0, 120.0), (220.0, 120.0), (220.0, 120.0), (220.0, 120.0)]);
}

#[test]
fn no_two_cards_come_closer_than_the_editor_s_gap() {
    for value in [landscape(), json!({ "cards": (0..9).map(|i| card(&format!("c{i}"), 0.0, 0.0)).collect::<Vec<_>>(), "connections": [] })] {
        let answer = arranged(value);
        for (i, first) in answer.cards.iter().enumerate() {
            for second in &answer.cards[i + 1..] {
                assert!(gap(first, second) >= CARD_GAP, "{} and {} are {} apart", first.id, second.id, gap(first, second));
            }
        }
    }
}

#[test]
fn the_top_left_corner_of_the_diagram_stays_where_it_was() {
    let answer = arranged(landscape());
    let left = answer.cards.iter().map(|card| card.x).fold(f64::INFINITY, f64::min);
    let top = answer.cards.iter().map(|card| card.y).fold(f64::INFINITY, f64::min);
    assert_eq!((left, top), (100.0, 40.0));
}

/// The landscape as the pipeline is given it: the cards in the order of the request.
fn landscape_graph() -> Graph {
    let nodes = (0..6).map(|_| Node { top_left: Point { x: 0.0, y: 0.0 }, width: 220.0, height: 120.0 }).collect();
    let plain = |from, to| Edge { from, to, min_width: 0.0, min_height: 0.0 };
    let edges = vec![plain(0, 1), plain(0, 1), plain(1, 0), Edge { from: 0, to: 2, min_width: 90.0, min_height: 22.0 }, plain(2, 3), plain(2, 4)];
    Graph::new(nodes, edges)
}

#[test]
fn the_seed_is_tala_s_pick_where_that_keeps_the_gap() {
    let answer = arranged(landscape());
    let best = layout_best(&landscape_graph(), &SEEDS).unwrap();
    assert_eq!(answer.seed, best.seed);
    // The places are TALA's, moved as a whole.
    let drawing = &best.drawing.graph;
    let at = |id: usize| drawing.nodes[(0..drawing.nodes.len()).find(|node| drawing.id(*node) == id as i64).unwrap()].top_left;
    let (dx, dy) = (answer.cards[0].x - at(0).x, answer.cards[0].y - at(0).y);
    for (id, card) in answer.cards.iter().enumerate() {
        assert_eq!((card.x - dx, card.y - dy), (at(id).x, at(id).y), "{}", card.id);
    }
}

fn score(penalty: f64, area: f64) -> Score {
    Score { penalty, area }
}

#[test]
fn of_the_seeds_that_keep_the_gap_the_best_by_tala_s_score_is_picked() {
    // All keep it: TALA's pick, the lowest penalty.
    assert_eq!(pick_seed(&[(score(3.0, 10.0), 40.0), (score(1.0, 10.0), 20.0), (score(2.0, 10.0), 60.0)]), Some(1));
    // TALA's best leaves two cards 19 apart: the next best.
    assert_eq!(pick_seed(&[(score(3.0, 10.0), 40.0), (score(1.0, 10.0), 19.0), (score(2.0, 10.0), 60.0)]), Some(2));
    // Equal penalties: the smaller area; equal scores: the later seed.
    assert_eq!(pick_seed(&[(score(1.0, 9.0), 40.0), (score(1.0, 10.0), 40.0), (score(5.0, 1.0), 40.0)]), Some(0));
    assert_eq!(pick_seed(&[(score(1.0, 9.0), 40.0), (score(1.0, 9.0), 40.0), (score(1.0, 9.0), -5.0)]), Some(1));
    // None keeps it.
    assert_eq!(pick_seed(&[(score(1.0, 9.0), 0.0), (score(1.0, 9.0), 19.9), (score(1.0, 9.0), -100.0)]), None);
    assert_eq!(pick_seed(&[]), None);
}

#[test]
fn a_connection_from_a_card_to_itself_has_no_say() {
    let mut with_loop = landscape();
    with_loop["connections"].as_array_mut().unwrap().insert(1, json!({ "from": "c", "to": "c" }));
    assert_eq!(arranged(with_loop), arranged(landscape()));
}

#[test]
fn a_card_alone_and_an_empty_diagram() {
    let one = arranged(json!({ "cards": [card("a", 37.0, -12.0)], "connections": [] }));
    assert_eq!(one.cards, [ArrangedCard { id: "a".into(), x: 37.0, y: -12.0, width: 220.0, height: 120.0 }]);
    assert!(arranged(json!({ "cards": [], "connections": [] })).cards.is_empty());
}

#[test]
fn unknown_cards_and_bad_sizes_are_refused() {
    let error = arrange(&request(json!({ "cards": [card("a", 0.0, 0.0)], "connections": [{ "from": "a", "to": "nope" }] })));
    assert_eq!(error.unwrap_err(), "unknown card: nope");
    let error = arrange(&request(json!({ "cards": [{ "id": "a", "x": 0, "y": 0, "width": 0, "height": 120 }], "connections": [] })));
    assert_eq!(error.unwrap_err(), "card a needs a finite position and a positive size");
}

#[test]
fn json_in_and_out() {
    let answer: Value = serde_json::from_str(&arrange_json(&json!({ "cards": [card("a", 0.0, 0.0)], "connections": [] }).to_string())).unwrap();
    assert_eq!(answer, json!({ "cards": [{ "id": "a", "x": 0.0, "y": 0.0, "width": 220.0, "height": 120.0 }], "seed": answer["seed"] }));
    let failure: Value = serde_json::from_str(&arrange_json("{}")).unwrap();
    assert!(failure["error"].as_str().unwrap().starts_with("not an arrange request"), "{failure}");
}
