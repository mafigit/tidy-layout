// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.

//! What the editor relies on when it asks where a dropped card goes.

use tidy_layout::api::{SnapRequest, SnapResponse, snap, snap_json};
use serde_json::{Value, json};

fn request(value: Value) -> SnapRequest {
    serde_json::from_value(value).unwrap()
}

fn card(id: &str, x: f64, y: f64) -> Value {
    json!({ "id": id, "x": x, "y": y, "width": 220, "height": 120 })
}

fn drop_at(connected: bool, x: f64, y: f64) -> SnapResponse {
    let connections = if connected { json!([{ "from": "a", "to": "b" }]) } else { json!([]) };
    snap(&request(json!({
        "cards": [card("a", 0.0, 0.0), card("b", 600.0, 0.0)],
        "connections": connections,
        "moving": "b",
        "wanted": { "x": x, "y": y },
        "cellSize": 20,
    })))
    .unwrap()
}

#[test]
fn a_free_place_is_taken_on_the_grid() {
    assert_eq!(drop_at(false, 407.0, 293.0), SnapResponse { x: 400.0, y: 300.0, ring: 0 });
}

#[test]
fn a_card_can_stay_where_it_is() {
    assert_eq!(drop_at(false, 600.0, 0.0), SnapResponse { x: 600.0, y: 0.0, ring: 0 });
}

#[test]
fn a_card_dropped_on_another_goes_to_the_nearest_place_with_a_gap_of_20() {
    // Card a ends at x = 220, so the nearest free column starts at 240.
    let place = drop_at(false, 200.0, 0.0);
    assert_eq!((place.x, place.y), (240.0, 0.0));
    assert!(place.ring > 0);
}

#[test]
fn connected_cards_keep_a_gap_of_60() {
    let place = drop_at(true, 200.0, 0.0);
    assert_eq!((place.x, place.y), (280.0, 0.0));
}

#[test]
fn a_card_dropped_a_little_out_of_line_with_its_connection_lands_in_line() {
    // The nearest free place is (40, 180): below card a, 40 out of line. Straight below it,
    // two steps of the grid further, the connection costs less.
    assert_eq!(drop_at(true, 40.0, 40.0), SnapResponse { x: 0.0, y: 180.0, ring: 9 });
}

#[test]
fn a_card_looks_no_further_than_40_beyond_the_nearest_free_place() {
    // Card b is connected to card c far to the right, and dropped on card a. The nearest free
    // place is straight above a, 7 steps away. Every step to the right is cheaper; b takes two.
    let place = snap(&request(json!({
        "cards": [card("a", 0.0, 0.0), card("b", 600.0, 600.0), card("c", 2000.0, 0.0)],
        "connections": [{ "from": "b", "to": "c" }],
        "moving": "b",
        "wanted": { "x": 0.0, "y": 0.0 },
        "cellSize": 20,
    })))
    .unwrap();
    assert_eq!(place, SnapResponse { x: 40.0, y: -140.0, ring: 9 });
}

#[test]
fn on_a_grid_of_30_a_card_looks_two_steps_further() {
    // 40 is more than one step of this grid: the card looks two steps beyond the nearest free
    // place, (60, 180), and lands in line.
    let place = snap(&request(json!({
        "cards": [card("a", 0.0, 0.0), card("b", 600.0, 0.0)],
        "connections": [{ "from": "a", "to": "b" }],
        "moving": "b",
        "wanted": { "x": 60.0, "y": 30.0 },
        "cellSize": 30,
    })))
    .unwrap();
    assert_eq!(place, SnapResponse { x: 0.0, y: 180.0, ring: 7 });
}

#[test]
fn the_rings_are_counted_in_whole_steps_on_any_grid() {
    // 14 steps of 1.1 to the right; in floating point a little less than 14.
    let place = snap(&request(json!({
        "cards": [card("a", 0.0, 0.0), card("b", 600.0, 0.0)],
        "connections": [],
        "moving": "b",
        "wanted": { "x": 225.0, "y": 3.0 },
        "cellSize": 1.1,
    })))
    .unwrap();
    assert_eq!(place.ring, 14);
}

#[test]
fn an_arrow_would_rather_point_sideways_than_up() {
    // Card a points to card b with a label that needs 80 between cards side by side. Above a,
    // b would be 20 nearer than beside it, with the arrow pointing up. TALA prices the direction
    // in cells of its own grid, the size of a card and not a step of the editor's: up costs 33
    // more than sideways.
    let place = snap(&request(json!({
        "cards": [card("a", 0.0, 0.0), card("b", 900.0, 900.0)],
        "connections": [{ "from": "a", "to": "b", "label": { "width": 70, "height": 16 } }],
        "moving": "b",
        "wanted": { "x": 200.0, "y": -100.0 },
        "cellSize": 20,
    })))
    .unwrap();
    assert_eq!((place.x, place.y), (300.0, -80.0));
}

#[test]
fn a_connection_from_a_card_to_itself_has_no_say() {
    let place = snap(&request(json!({
        "cards": [card("a", 0.0, 0.0), card("b", 600.0, 0.0)],
        "connections": [{ "from": "b", "to": "b" }],
        "moving": "b",
        "wanted": { "x": 200.0, "y": 0.0 },
        "cellSize": 20,
    })))
    .unwrap();
    assert_eq!((place.x, place.y), (240.0, 0.0));
}

fn drop_beside_or_above(wanted: (f64, f64)) -> SnapResponse {
    snap(&request(json!({
        "cards": [card("a", 0.0, 0.0), card("b", 900.0, 900.0)],
        // A label 150 wide and 16 high: with padding it needs 160 by 26.
        "connections": [{ "from": "a", "to": "b", "label": { "width": 149.2, "height": 16 } }],
        "moving": "b",
        "wanted": { "x": wanted.0, "y": wanted.1 },
        "cellSize": 20,
    })))
    .unwrap()
}

#[test]
fn a_label_gets_its_width_between_cards_side_by_side() {
    // Card a ends at x = 220; the label needs 160: the nearest place on the grid starts at 380.
    let place = drop_beside_or_above((230.0, 0.0));
    assert_eq!((place.x, place.y), (380.0, 0.0));
}

#[test]
fn a_label_needs_only_its_height_between_cards_above_each_other() {
    // Card a ends at y = 120; the label needs 26, a connection 60: the 60 decides.
    let place = drop_beside_or_above((0.0, 130.0));
    assert_eq!((place.x, place.y), (0.0, 180.0));
}

#[test]
fn without_a_cell_size_the_grid_is_talas() {
    let place = snap(&request(json!({
        "cards": [card("a", 0.0, 0.0), card("b", 1000.0, 1000.0)],
        "connections": [],
        "moving": "b",
        "wanted": { "x": 500.0, "y": 300.0 },
    })))
    .unwrap();
    // TALA's cell for 220 by 120 cards is 220.
    assert_eq!((place.x, place.y), (440.0, 220.0));
}

#[derive(serde::Deserialize)]
struct RefNode {
    id: usize,
    x: f64,
    y: f64,
    width: f64,
    height: f64,
}

#[derive(serde::Deserialize)]
struct RefCase {
    nodes: Vec<RefNode>,
    edges: Option<Vec<[usize; 2]>>,
    moving: usize,
    point: [f64; 2],
}

/// On every recorded layout, with a fine grid: wherever the card is wanted, the place it gets
/// keeps the gap to every other card.
#[test]
fn the_place_never_breaks_a_gap() {
    let cases: Vec<RefCase> =
        serde_json::from_str(include_str!("fixtures/closest-unoccupied.tala.json")).unwrap();
    for (number, case) in cases.iter().enumerate() {
        let connected = |a: usize, b: usize| {
            case.edges.iter().flatten().any(|e| (e[0] == a && e[1] == b) || (e[0] == b && e[1] == a))
        };
        let cards: Vec<Value> = case
            .nodes
            .iter()
            .map(|n| json!({ "id": n.id.to_string(), "x": n.x, "y": n.y, "width": n.width, "height": n.height }))
            .collect();
        let connections: Vec<Value> = case
            .edges
            .iter()
            .flatten()
            .map(|e| json!({ "from": e[0].to_string(), "to": e[1].to_string() }))
            .collect();
        let place = snap(&request(json!({
            "cards": cards,
            "connections": connections,
            "moving": case.moving.to_string(),
            "wanted": { "x": case.point[0], "y": case.point[1] },
            "cellSize": 20,
        })))
        .unwrap();
        assert_eq!((place.x % 20.0, place.y % 20.0), (0.0, 0.0), "case {number}: on the grid");
        let me = case.nodes.iter().find(|n| n.id == case.moving).unwrap();
        for other in case.nodes.iter().filter(|n| n.id != case.moving) {
            let gap = if connected(me.id, other.id) { 60.0 } else { 20.0 };
            let apart = place.x >= other.x + other.width + gap
                || place.x + me.width + gap <= other.x
                || place.y >= other.y + other.height + gap
                || place.y + me.height + gap <= other.y;
            assert!(apart, "case {number}: too close to card {}", other.id);
        }
    }
}

#[test]
fn a_bad_request_is_an_error_not_a_crash() {
    let error = |input: &str| serde_json::from_str::<Value>(&snap_json(input)).unwrap()["error"].clone();
    assert!(error("{").is_string());
    assert_eq!(
        error(r#"{"cards":[],"connections":[],"moving":"x","wanted":{"x":0,"y":0}}"#),
        json!("unknown card: x")
    );
    let one = r#"{"id":"a","x":0,"y":0,"width":0,"height":10}"#;
    assert!(error(&format!(r#"{{"cards":[{one}],"connections":[],"moving":"a","wanted":{{"x":0,"y":0}}}}"#)).is_string());
    let ok = snap_json(r#"{"cards":[{"id":"a","x":0,"y":0,"width":10,"height":10}],"connections":[],"moving":"a","wanted":{"x":3,"y":3},"cellSize":0}"#);
    assert!(serde_json::from_str::<Value>(&ok).unwrap()["error"].is_string());
}
