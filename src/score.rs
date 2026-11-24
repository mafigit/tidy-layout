// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.

//! TALA's score of a finished drawing and its choice between seeds. Lower is better: first
//! the penalty (bends, slanted pieces, crossings, how badly the labels sit), then the area of
//! the drawing. TALA lays a diagram out with seeds 1, 2 and 3 and keeps the drawing with the
//! lowest score. Ported from TALA `internal/quality`: `scoring.go` (`EvaluateWithArea`),
//! `score.go` (`Score.Compare`), `crossings.go` (`countNonSharedCrossings`), `labels.go`
//! (`scoreExistingLabelPlacements`, `findSharedSegments` and its helpers); from
//! `internal/layoutgraph`: `Graph.Area`, `Edge.boundingBoxValues`; from the package root:
//! `layout.go` (`coordinateLocalSeeds`). For the engine's diagrams: no containers, no labels
//! or icons of cards, no labels at arrowheads, no connection from a card to itself.

use std::cmp::Ordering;

use crate::graph::{Graph, Point};
use crate::quality::LabelBox;
use crate::routing::labels::Placed;
use crate::routing::line::crosses;

/// D2 `label.PADDING`.
const PADDING: f64 = 5.0;
/// TALA `labeling.SharedSegmentClearance`: half the width of the room around a stretch that
/// two routes share.
const SHARED_CLEARANCE: f64 = 2.5 * PADDING;

/// TALA `Score`: the penalty, and the area for a tie.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Score {
    pub penalty: f64,
    pub area: f64,
}

impl Score {
    /// TALA `Score.Compare`: by the penalty, then by the area. A number that is not finite
    /// (or a negative area) loses to one that is; two such are equal.
    pub fn compare(&self, other: &Score) -> Ordering {
        match compare_number(self.penalty, other.penalty, false) {
            Ordering::Equal => compare_number(self.area, other.area, true),
            order => order,
        }
    }
}

fn compare_number(left: f64, right: f64, nonnegative: bool) -> Ordering {
    let valid = |value: f64| value.is_finite() && (!nonnegative || value >= 0.0);
    match (valid(left), valid(right)) {
        (true, false) => Ordering::Less,
        (false, true) => Ordering::Greater,
        (false, false) => Ordering::Equal,
        // Go's cmp.Compare; both are finite here.
        (true, true) => left.partial_cmp(&right).unwrap_or(Ordering::Equal),
    }
}

/// TALA `coordinateLocalSeeds`: the number of the best of the scores, in the order of the
/// seeds. Of equal scores the later wins: TALA keeps a result that is no worse and comes later
/// in the list of seeds.
pub fn best_seed(scores: &[Score]) -> Option<usize> {
    let mut best: Option<usize> = None;
    for (index, score) in scores.iter().enumerate() {
        if best.is_none_or(|known| score.compare(&scores[known]) != Ordering::Greater) {
            best = Some(index);
        }
    }
    best
}

/// TALA `EvaluateWithArea` for a finished drawing: the cards at their places, the routes and,
/// per connection, the place of its label, if it has one.
pub fn evaluate(graph: &Graph, routes: &[Vec<Point>], labels: &[Option<Placed>]) -> Score {
    let mut penalty = 0.0;
    for (edge, route) in routes.iter().enumerate() {
        let wire = graph.edges[edge];
        // The routes to and from the cards of a cluster are left out of this part.
        if graph.cluster_of(wire.from).is_some() || graph.cluster_of(wire.to).is_some() {
            continue;
        }
        let bends = route.len().saturating_sub(2) as f64;
        penalty += bends * 0.5;
        for piece in route.windows(2) {
            // A slanted piece costs extra.
            if piece[0].x != piece[1].x && piece[0].y != piece[1].y {
                penalty += 3.0;
            }
        }
    }
    penalty += crossings(routes) as f64;
    let boxes = label_boxes(graph, labels);
    penalty += 1.0 - label_score(graph, routes, &boxes);
    Score { penalty, area: area(graph, routes, &boxes) }
}

/// TALA `countNonSharedCrossings`: pieces of two routes that cross at an angle.
fn crossings(routes: &[Vec<Point>]) -> i64 {
    let mut count = 0;
    for (index, route) in routes.iter().enumerate() {
        for other in &routes[index + 1..] {
            for piece in route.windows(2) {
                for other_piece in other.windows(2) {
                    if crosses(piece[0], piece[1], other_piece[0], other_piece[1]) {
                        count += 1;
                    }
                }
            }
        }
    }
    count
}

/// Per connection the box of its label where it is placed (TALA `labelBox`), if it has one.
fn label_boxes(graph: &Graph, labels: &[Option<Placed>]) -> Vec<Option<LabelBox>> {
    graph
        .edges
        .iter()
        .zip(labels)
        .map(|(edge, label)| {
            // A connection has a label where its `min_width` is more than 0, as in the pipeline.
            let has = edge.min_width > 0.0;
            label.as_ref().filter(|_| has).map(|label| LabelBox { top_left: label.top_left, width: edge.min_width, height: edge.min_height })
        })
        .collect()
}

/// TALA `boxesOverlapWithPadding`: the boxes overlap, or come closer than `padding`.
fn overlap(first: &LabelBox, second: &LabelBox, padding: f64) -> bool {
    let (first_right, second_right) = (first.top_left.x + first.width, second.top_left.x + second.width);
    if first.top_left.x >= second_right + padding || second.top_left.x >= first_right + padding {
        return false;
    }
    let (first_bottom, second_bottom) = (first.top_left.y + first.height, second.top_left.y + second.height);
    first.top_left.y < second_bottom + padding && second.top_left.y < first_bottom + padding
}

/// TALA `boxOverlapArea`.
fn overlap_area(first: &LabelBox, second: &LabelBox) -> f64 {
    let mut xs = [first.top_left.x, first.top_left.x + first.width, second.top_left.x, second.top_left.x + second.width];
    let mut ys = [first.top_left.y, first.top_left.y + first.height, second.top_left.y, second.top_left.y + second.height];
    xs.sort_by(f64::total_cmp);
    ys.sort_by(f64::total_cmp);
    (xs[2] - xs[1]) * (ys[2] - ys[1])
}

/// TALA `orientation`.
fn orientation(p: Point, q: Point, r: Point) -> f64 {
    (q.y - p.y) * (r.x - p.x) - (q.x - p.x) * (r.y - p.y)
}

/// TALA `segmentsIntersect`: the two pieces meet, also where one only touches the other.
fn segments_intersect(first_start: Point, first_end: Point, second_start: Point, second_end: Point) -> bool {
    let second_start_side = orientation(first_start, first_end, second_start);
    let second_end_side = orientation(first_start, first_end, second_end);
    let first_start_side = orientation(second_start, second_end, first_start);
    let first_end_side = orientation(second_start, second_end, first_end);
    if second_start_side == 0.0 && second_end_side == 0.0 && first_start_side == 0.0 && first_end_side == 0.0 {
        let within = |a: f64, b: f64, c: f64, d: f64| a.min(b) <= c.max(d) && c.min(d) <= a.max(b);
        return within(first_start.x, first_end.x, second_start.x, second_end.x) && within(first_start.y, first_end.y, second_start.y, second_end.y);
    }
    let straddles = |a: f64, b: f64| a == 0.0 || b == 0.0 || (a < 0.0) != (b < 0.0);
    straddles(second_start_side, second_end_side) && straddles(first_start_side, first_end_side)
}

/// TALA `boxOverlapsLine`: the piece has an end in the box widened by `padding`, or meets its
/// border.
fn overlaps_line(area: &LabelBox, first: Point, second: Point, padding: f64) -> bool {
    let (left, right) = (area.top_left.x - padding, area.top_left.x + area.width + padding);
    let (top, bottom) = (area.top_left.y - padding, area.top_left.y + area.height + padding);
    let contains = |point: Point| left <= point.x && right >= point.x && top <= point.y && bottom >= point.y;
    if contains(first) || contains(second) {
        return true;
    }
    let corners = [Point { x: left, y: top }, Point { x: right, y: top }, Point { x: right, y: bottom }, Point { x: left, y: bottom }];
    (0..4).any(|side| segments_intersect(corners[side], corners[(side + 1) % 4], first, second))
}

/// TALA `findSharedSegments`: the stretches where routes run on top of each other, per line
/// of pieces that run along it. TALA walks the lines in the order of a Go map; only how many of
/// the stretches a label meets counts, so the order does not matter.
fn shared_segments(routes: &[Vec<Point>]) -> Vec<(Point, Point)> {
    let mut vertical: Vec<(f64, Vec<(Point, Point)>)> = Vec::new();
    let mut horizontal: Vec<(f64, Vec<(Point, Point)>)> = Vec::new();
    let add = |lines: &mut Vec<(f64, Vec<(Point, Point)>)>, at: f64, piece: (Point, Point)| match lines.iter_mut().find(|(line, _)| *line == at) {
        Some((_, pieces)) => pieces.push(piece),
        None => lines.push((at, vec![piece])),
    };
    for route in routes {
        for piece in route.windows(2) {
            let (mut start, mut end) = (piece[0], piece[1]);
            if start.x == end.x {
                if end.y < start.y {
                    (start.y, end.y) = (end.y, start.y);
                }
                add(&mut vertical, start.x, (start, end));
            } else if start.y == end.y {
                if end.x < start.x {
                    (start.x, end.x) = (end.x, start.x);
                }
                add(&mut horizontal, start.y, (start, end));
            }
        }
    }
    let mut shared = Vec::new();
    for (lines, along_x) in [(vertical, false), (horizontal, true)] {
        let along = |point: &Point| if along_x { point.x } else { point.y };
        let set = |point: &mut Point, value: f64| if along_x { point.x = value } else { point.y = value };
        for (_, mut pieces) in lines {
            if pieces.len() == 1 {
                continue;
            }
            // TALA `sortSharedSegments`, a stable merge sort by start, then end.
            pieces.sort_by(|left, right| if along(&left.0) == along(&right.0) { along(&left.1).total_cmp(&along(&right.1)) } else { along(&left.0).total_cmp(&along(&right.0)) });
            let mut previous = pieces[0];
            let mut stretch: Option<(Point, Point)> = None;
            for &current in &pieces[1..] {
                if along(&previous.1) > along(&current.0) {
                    match &mut stretch {
                        None => {
                            let mut end = current.0;
                            set(&mut end, along(&previous.1).min(along(&current.1)));
                            stretch = Some((current.0, end));
                        }
                        Some((_, end)) => {
                            let least = along(&previous.1).min(along(&current.1));
                            let value = along(end).max(least);
                            set(end, value);
                        }
                    }
                } else if let Some(done) = stretch.take() {
                    shared.push(done);
                }
                if along(&current.1) > along(&previous.1) {
                    previous = current;
                }
            }
            if let Some(done) = stretch {
                shared.push(done);
            }
        }
    }
    shared
}

/// TALA `scoreExistingLabelPlacements`, from 1 for labels that sit well towards 0.
fn label_score(graph: &Graph, routes: &[Vec<Point>], boxes: &[Option<LabelBox>]) -> f64 {
    // TALA first gives the labels of connections from a card back to itself their room, and
    // scores them not. The engine is given no such connection (`coordinator`).
    let mut placed: Vec<LabelBox> = Vec::new();
    let mut total = 0.0;
    let shared: Vec<LabelBox> = shared_segments(routes)
        .into_iter()
        .map(|(start, end)| {
            let (mut top_left, mut width, mut height) = (start, end.x - start.x, end.y - start.y);
            if end.x == start.x {
                width = 2.0 * SHARED_CLEARANCE;
                top_left.x -= SHARED_CLEARANCE;
            } else {
                height = 2.0 * SHARED_CLEARANCE;
                top_left.y -= SHARED_CLEARANCE;
            }
            LabelBox { top_left, width, height }
        })
        .collect();
    let cards: Vec<LabelBox> = graph.nodes.iter().map(|node| LabelBox { top_left: node.top_left, width: node.width, height: node.height }).collect();
    for (edge, label) in boxes.iter().enumerate() {
        let Some(label) = *label else { continue };
        // TALA `nodeOverlapArea` over all cards (no card holds another): the part the label
        // covers, and the label's whole area once more, for every card it comes within the
        // padding of.
        let mut covered = 0.0;
        for card in &cards {
            if !overlap(&label, card, PADDING) {
                continue;
            }
            if overlap(&label, card, 0.0) {
                covered += overlap_area(&label, card);
            }
            covered += label.width * label.height;
        }
        let pieces = routes.iter().enumerate().filter(|(other, _)| *other != edge).map(|(_, route)| route.windows(2).filter(|piece| overlaps_line(&label, piece[0], piece[1], PADDING)).count()).sum::<usize>();
        let on_labels = placed.iter().filter(|other| overlap(&label, other, 0.0)).count();
        let near_labels = placed.iter().filter(|other| overlap(&label, other, PADDING)).count();
        let on_shared = shared.iter().filter(|stretch| overlap(&label, stretch, PADDING)).count();
        // TALA `scoreEdgeLabelOverlaps`, term by term in its order.
        let mut score = covered / (label.width * label.height) * 2.0;
        score += on_labels as f64 * 10.0;
        score += near_labels as f64;
        score += 0.0;
        score += pieces as f64 * 2.0;
        score += on_shared as f64;
        total += score;
        placed.push(label);
    }
    1.0 / (1.0 + total)
}

/// TALA `Graph.Area`: the area of the box around the cards, the routes and the labels. The
/// cards count unrounded, but the box of each route with its label is rounded first.
fn area(graph: &Graph, routes: &[Vec<Point>], boxes: &[Option<LabelBox>]) -> f64 {
    if graph.nodes.is_empty() {
        return 0.0;
    }
    let (mut min, mut max) = (Point { x: f64::INFINITY, y: f64::INFINITY }, Point { x: f64::NEG_INFINITY, y: f64::NEG_INFINITY });
    for node in &graph.nodes {
        min = Point { x: min.x.min(node.top_left.x), y: min.y.min(node.top_left.y) };
        max = Point { x: max.x.max(node.top_left.x + node.width), y: max.y.max(node.top_left.y + node.height) };
    }
    for (route, label) in routes.iter().zip(boxes) {
        if route.is_empty() {
            continue;
        }
        // TALA `Edge.boundingBoxValues`: the box of the route and its label, rounded.
        let (mut low, mut high) = (Point { x: f64::INFINITY, y: f64::INFINITY }, Point { x: f64::NEG_INFINITY, y: f64::NEG_INFINITY });
        for point in route {
            low = Point { x: low.x.min(point.x), y: low.y.min(point.y) };
            high = Point { x: high.x.max(point.x), y: high.y.max(point.y) };
        }
        if let Some(label) = label {
            low = Point { x: low.x.min(label.top_left.x), y: low.y.min(label.top_left.y) };
            high = Point { x: high.x.max(label.top_left.x + label.width), y: high.y.max(label.top_left.y + label.height) };
        }
        min = Point { x: min.x.min(low.x.round()), y: min.y.min(low.y.round()) };
        max = Point { x: max.x.max(high.x.round()), y: max.y.max(high.y.round()) };
    }
    (min.x - max.x).abs() * (min.y - max.y).abs()
}
