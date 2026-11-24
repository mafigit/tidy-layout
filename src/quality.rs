// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.

//! What TALA counts on a finished drawing: cards on each other, routes through cards, covered
//! labels, crossings, detours and the length of all routes. A stage that changes routes late
//! asks for these counts before and after and takes its change back where one got worse.
//! Ported from TALA `internal/quality`: `inspection.go` (`Inspect`), `inspection_geometry.go`
//! (`measureGeometry`, `segmentEntersBox`), `inspection_labels.go` (`measureLabels`),
//! `crossings.go` (`countNonSharedCrossings`) and `labels.go` (`boxesOverlapWithPadding`).
//! For plain cards: no containers, no labels of cards, no labels at arrowheads.

use crate::graph::{Graph, Point};
use crate::placementcost::flow_continuity::hypot;
use crate::routing::line::crosses;

/// TALA `Metrics`.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Metrics {
    pub node_overlaps: i64,
    pub route_obstructions: i64,
    pub text_occlusions: i64,
    pub crossings: f64,
    pub detour: f64,
    pub route_length: f64,
}

/// A box: a card, or the label of a connection where it is placed.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct LabelBox {
    pub top_left: Point,
    pub width: f64,
    pub height: f64,
}

/// TALA `boxesOverlapWithPadding` without padding: the insides of the two boxes overlap.
fn boxes_overlap(first: &LabelBox, second: &LabelBox) -> bool {
    let (first_right, second_right) = (first.top_left.x + first.width, second.top_left.x + second.width);
    if first.top_left.x >= second_right || second.top_left.x >= first_right {
        return false;
    }
    let (first_bottom, second_bottom) = (first.top_left.y + first.height, second.top_left.y + second.height);
    first.top_left.y < second_bottom && second.top_left.y < first_bottom
}

/// TALA `segmentEntersBox`: the piece runs through the inside of the box. Touching its border
/// or running along it does not count. Also for a slanted piece.
fn enters(area: &LabelBox, a: Point, b: Point) -> bool {
    if area.width <= 0.0 || area.height <= 0.0 || (a.x == b.x && a.y == b.y) {
        return false;
    }
    let (mut low, mut high) = (0.0_f64, 1.0_f64);
    for (start, delta, least, most) in [(a.x, b.x - a.x, area.top_left.x, area.top_left.x + area.width), (a.y, b.y - a.y, area.top_left.y, area.top_left.y + area.height)] {
        if delta == 0.0 {
            if start <= least || start >= most {
                return false;
            }
            continue;
        }
        let (mut first, mut last) = ((least - start) / delta, (most - start) / delta);
        if first > last {
            (first, last) = (last, first);
        }
        (low, high) = (low.max(first), high.min(last));
        if low >= high {
            return false;
        }
    }
    low < high
}

/// TALA `Inspect`: the counts for cards at their places, the routes of the connections and the
/// boxes of their labels (`None` for a connection without a label or without a place for it).
pub fn inspect(graph: &Graph, routes: &[Vec<Point>], labels: &[Option<LabelBox>]) -> Metrics {
    let mut score = Metrics { node_overlaps: 0, route_obstructions: 0, text_occlusions: 0, crossings: 0.0, detour: 0.0, route_length: 0.0 };
    // Lengths count in cards, not in pixels: the mean of half a card's width and height.
    let scale = graph.nodes.iter().fold(0.0, |sum, node| sum + (node.width + node.height) / 2.0);
    let scale = (scale / graph.nodes.len().max(1) as f64).max(1.0);
    for route in routes {
        let length = route.windows(2).fold(0.0, |sum, piece| sum + hypot(piece[1].x - piece[0].x, piece[1].y - piece[0].y));
        score.route_length += length / scale;
        if let (Some(first), Some(last), true) = (route.first(), route.last(), route.len() >= 2) {
            let direct = (last.x - first.x).abs() + (last.y - first.y).abs();
            score.detour += (length - direct).max(0.0) / scale.max(direct);
        }
    }
    // TALA `countNonSharedCrossings`.
    for (index, route) in routes.iter().enumerate() {
        for other in &routes[index + 1..] {
            for piece in route.windows(2) {
                for other_piece in other.windows(2) {
                    if crosses(piece[0], piece[1], other_piece[0], other_piece[1]) {
                        score.crossings += 1.0;
                    }
                }
            }
        }
    }
    // TALA `measureGeometry`.
    let cards: Vec<LabelBox> = graph.nodes.iter().map(|node| LabelBox { top_left: node.top_left, width: node.width, height: node.height }).collect();
    for (index, card) in cards.iter().enumerate() {
        score.node_overlaps += cards[index + 1..].iter().filter(|other| boxes_overlap(card, other)).count() as i64;
        for (edge, route) in routes.iter().enumerate() {
            // The first piece may start inside the card it leaves, the last may end inside the
            // card it reaches. Coming back into one of the two counts. Once per card and route.
            let through = (1..route.len()).any(|piece| {
                let own = (piece == 1 && graph.edges[edge].from == index) || (piece == route.len() - 1 && graph.edges[edge].to == index);
                !own && enters(card, route[piece - 1], route[piece])
            });
            score.route_obstructions += i64::from(through);
        }
    }
    // TALA `measureLabels`: a label on a label placed before it, on a card, under another route.
    let placed: Vec<(usize, &LabelBox)> =
        labels.iter().enumerate().filter_map(|(edge, label)| label.as_ref().map(|label| (edge, label))).filter(|(edge, label)| routes[*edge].len() >= 2 && label.width > 0.0 && label.height > 0.0).collect();
    for (index, (edge, label)) in placed.iter().enumerate() {
        score.text_occlusions += placed[..index].iter().filter(|(_, other)| boxes_overlap(label, other)).count() as i64;
        score.text_occlusions += cards.iter().filter(|card| boxes_overlap(label, card)).count() as i64;
        score.text_occlusions += routes.iter().enumerate().filter(|(other, route)| other != edge && route.windows(2).any(|piece| enters(label, piece[0], piece[1]))).count() as i64;
    }
    score
}
