// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.

//! Where the labels of connections go: beside or on their route, at the place where they cover
//! the least. Ported from TALA `internal/labeling` (`placeNewEdges`, `findBestEdgeLabelPosition`,
//! `findSharedSegmentsChecked`, `scoreEdgeLabelOverlaps`, `sortLabelPlacementEdges` and the
//! overlap counts in `guard.go`), `internal/routing/standalone_route_guard.go`
//! (`reorderDuplicatesInEdgesGuarded`), `internal/layoutgraph/node.go` (`overlapsLine`,
//! `boxesOverlapWithPadding`, `overlapArea`) and D2 `lib/label` (`GetPointOnRoute`), `lib/geo`
//! (`GetPointAtDistance`). For labels of connections between plain cards and in trees (a tree
//! connection's label goes where the tree stage wants it, if that place is free) and of
//! connections of clusters (`isClusterPathSharedChecked`, `clusterLabelPlacementOrientation`,
//! `labelPercentageSearchRange`). Not ported: labels and icons of cards, arrowhead labels,
//! labels with a place given by the caller.

use std::collections::HashMap;

use crate::graph::{Graph, Node, Point};
use crate::policy::LABEL_PADDING;
use crate::routing::geometry::{Orientation, euclidean_distance, segments_intersect};
use crate::routing::line::side;
use crate::routing::ovg::Coord;

/// The size of a label's box.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct LabelSize {
    pub width: f64,
    pub height: f64,
}

/// Where a label sits relative to its route. D2 `label.Position`, with D2's numbers.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[repr(u8)]
pub enum Position {
    OutsideTopLeft = 1,
    OutsideTopCenter = 2,
    OutsideTopRight = 3,
    OutsideBottomLeft = 10,
    OutsideBottomCenter = 11,
    OutsideBottomRight = 12,
    InsideMiddleLeft = 16,
    InsideMiddleCenter = 17,
    InsideMiddleRight = 18,
    UnlockedTop = 34,
    UnlockedMiddle = 35,
    UnlockedBottom = 36,
}

/// The order in which TALA tries the places: beside the middle of the route first, then beside
/// its quarters, then on it, then anywhere along it. TALA `edgeLabelPreferenceOrder` and what
/// `findBestEdgeLabelPosition` adds for a plain connection.
const PREFERENCE: [Position; 12] = [
    Position::OutsideTopCenter,
    Position::OutsideBottomCenter,
    Position::OutsideTopLeft,
    Position::OutsideTopRight,
    Position::OutsideBottomLeft,
    Position::OutsideBottomRight,
    Position::InsideMiddleCenter,
    Position::InsideMiddleLeft,
    Position::InsideMiddleRight,
    Position::UnlockedTop,
    Position::UnlockedBottom,
    Position::UnlockedMiddle,
];

/// D2 `edgeStrokeWidth`: how thick TALA takes a connector to be.
const STROKE_WIDTH: f64 = 3.0;
/// TALA `SharedSegmentClearance`: room kept around a stretch that several routes share.
const SHARED_SEGMENT_CLEARANCE: f64 = 2.5 * LABEL_PADDING;
/// TALA `placementSearchInterval`: the step in which places along a route are tried.
const SEARCH_INTERVAL: f64 = 0.025;

impl Position {
    /// The position with D2's number, as a recording holds it.
    pub fn of(number: u8) -> Option<Position> {
        PREFERENCE.into_iter().find(|position| *position as u8 == number)
    }

    /// D2 `Position.Mirrored`, for the unlocked positions: the other side of the line.
    pub(crate) fn mirrored(self) -> Position {
        match self {
            Position::UnlockedTop => Position::UnlockedBottom,
            Position::UnlockedBottom => Position::UnlockedTop,
            other => other,
        }
    }

    fn is_unlocked(self) -> bool {
        matches!(self, Position::UnlockedTop | Position::UnlockedMiddle | Position::UnlockedBottom)
    }

    /// The label lies on the line, not beside it.
    fn is_on_edge(self) -> bool {
        matches!(
            self,
            Position::InsideMiddleLeft | Position::InsideMiddleCenter | Position::InsideMiddleRight | Position::UnlockedMiddle
        )
    }
}

/// A label that has its place.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Placed {
    pub position: Position,
    /// How far along the route, from 0 to 1, for the unlocked positions; 0 for the others.
    pub percentage: f64,
    /// The top-left corner of the label's box.
    pub top_left: Point,
}

/// A box: a label, or the room around a shared stretch.
#[derive(Clone, Copy)]
struct Rect {
    top_left: Point,
    width: f64,
    height: f64,
}

impl Rect {
    fn of(node: &Node) -> Self {
        Rect { top_left: node.top_left, width: node.width, height: node.height }
    }

    /// TALA `boxesOverlapWithPadding`: the insides overlap, or are closer than `padding`.
    fn overlaps(&self, other: &Rect, padding: f64) -> bool {
        let (right, other_right) = (self.top_left.x + self.width, other.top_left.x + other.width);
        if self.top_left.x >= other_right + padding || other.top_left.x >= right + padding {
            return false;
        }
        let (bottom, other_bottom) = (self.top_left.y + self.height, other.top_left.y + other.height);
        self.top_left.y < other_bottom + padding && other.top_left.y < bottom + padding
    }

    /// TALA `overlapArea`: the rectangle between the two middle x and the two middle y.
    fn overlap_area(&self, other: &Rect) -> f64 {
        let mut xs = [self.top_left.x, self.top_left.x + self.width, other.top_left.x, other.top_left.x + other.width];
        let mut ys = [self.top_left.y, self.top_left.y + self.height, other.top_left.y, other.top_left.y + other.height];
        xs.sort_by(f64::total_cmp);
        ys.sort_by(f64::total_cmp);
        (xs[2] - xs[1]) * (ys[2] - ys[1])
    }

    /// TALA `overlapsLine`: the segment touches the box grown by `delta`.
    fn overlaps_line(&self, p1: Point, p2: Point, delta: f64) -> bool {
        let (l, r) = (self.top_left.x - delta, self.top_left.x + self.width + delta);
        let (t, b) = (self.top_left.y - delta, self.top_left.y + self.height + delta);
        let contains = |p: Point| l <= p.x && r >= p.x && t <= p.y && b >= p.y;
        if contains(p1) || contains(p2) {
            return true;
        }
        let (tl, tr, br, bl) = (Point { x: l, y: t }, Point { x: r, y: t }, Point { x: r, y: b }, Point { x: l, y: b });
        segments_intersect(tl, tr, p1, p2)
            || segments_intersect(tr, br, p1, p2)
            || segments_intersect(br, bl, p1, p2)
            || segments_intersect(bl, tl, p1, p2)
    }
}

fn route_length(route: &[Point]) -> f64 {
    route.windows(2).fold(0.0, |sum, piece| sum + euclidean_distance(piece[0].x, piece[0].y, piece[1].x, piece[1].y))
}

/// D2 `Route.GetPointAtDistance`: the point that far along the route, and the piece it is on.
fn point_at_distance(route: &[Point], distance: f64) -> (Point, usize) {
    let between = |a: Point, b: Point, t: f64| Point { x: a.x * (1.0 - t) + b.x * t, y: a.y * (1.0 - t) + b.y * t };
    let mut remaining = distance;
    let mut length = 0.0;
    for i in 0..route.len() - 1 {
        let (current, next) = (route[i], route[i + 1]);
        length = euclidean_distance(current.x, current.y, next.x, next.y);
        if remaining <= length {
            return (between(current, next, remaining / length), i);
        }
        remaining -= length;
    }
    // Past the end: go on along the last piece.
    let last = route.len() - 2;
    (between(route[last], route[last + 1], 1.0 + remaining / length), last)
}

/// D2 `chopPrecision`: to a whole number, by way of single precision.
fn chop(value: f64) -> f64 {
    let rounded = (f64::from((value * 10000.0) as f32) / 10000.0).round();
    if rounded == 0.0 { 0.0 } else { rounded }
}

/// D2 `GetPointOnRoute`: the top-left corner of a label of this size at a position on a route.
pub fn label_top_left(route: &[Point], position: Position, percentage: f64, size: LabelSize) -> Point {
    let total = route_length(route);
    let distance = match position {
        Position::OutsideTopLeft | Position::OutsideBottomLeft | Position::InsideMiddleLeft => 1.0 / 4.0 * total,
        Position::OutsideTopCenter | Position::OutsideBottomCenter | Position::InsideMiddleCenter => 2.0 / 4.0 * total,
        Position::OutsideTopRight | Position::OutsideBottomRight | Position::InsideMiddleRight => 3.0 / 4.0 * total,
        Position::UnlockedTop | Position::UnlockedMiddle | Position::UnlockedBottom => percentage * total,
    };
    let (base, index) = point_at_distance(route, distance);
    // A label beside the line is moved off it along the line's normal: half the line's
    // thickness, the padding and half the label. "Top" is the side of the flipped normal.
    let flip = match position {
        Position::OutsideTopLeft | Position::OutsideTopCenter | Position::OutsideTopRight | Position::UnlockedTop => Some(true),
        Position::OutsideBottomLeft | Position::OutsideBottomCenter | Position::OutsideBottomRight | Position::UnlockedBottom => {
            Some(false)
        }
        _ => None,
    };
    let center = match flip {
        None => base,
        Some(flip) => {
            let (start, end) = (route[index], route[index + 1]);
            let length = euclidean_distance(start.x, start.y, end.x, end.y);
            let (mut normal_x, mut normal_y) = ((start.y - end.y) / length, (end.x - start.x) / length);
            if flip {
                normal_x *= -1.0;
                normal_y *= -1.0;
            }
            let offset_x = STROKE_WIDTH / 2.0 + LABEL_PADDING + size.width / 2.0;
            let offset_y = STROKE_WIDTH / 2.0 + LABEL_PADDING + size.height / 2.0;
            Point { x: base.x + normal_x * offset_x, y: base.y + normal_y * offset_y }
        }
    };
    Point { x: chop(center.x - size.width / 2.0), y: chop(center.y - size.height / 2.0) }
}

/// TALA `findSharedSegmentsChecked`: the stretches where upright or level pieces of routes lie on
/// each other, each with room around it. A label there would belong to no one route.
fn shared_segment_boxes(routes: &[Vec<Point>]) -> Vec<Rect> {
    // Pieces by the line they lie on, each from its smaller end to its larger one.
    let mut upright: HashMap<Coord, Vec<(f64, f64)>> = HashMap::new();
    let mut level: HashMap<Coord, Vec<(f64, f64)>> = HashMap::new();
    for route in routes {
        for piece in route.windows(2) {
            let (a, b) = (piece[0], piece[1]);
            if a == b {
                continue;
            }
            if a.x == b.x {
                upright.entry(Coord::of(a.x)).or_default().push((a.y.min(b.y), a.y.max(b.y)));
            } else if a.y == b.y {
                level.entry(Coord::of(a.y)).or_default().push((a.x.min(b.x), a.x.max(b.x)));
            }
        }
    }
    let mut boxes = Vec::new();
    for (lines, is_level) in [(upright, false), (level, true)] {
        for (line, mut pieces) in lines {
            pieces.sort_by(|a, b| a.0.total_cmp(&b.0).then(a.1.total_cmp(&b.1)));
            let mut shared: Vec<(f64, f64)> = Vec::new();
            let mut previous = pieces[0];
            let mut open: Option<(f64, f64)> = None;
            for &current in &pieces[1..] {
                if previous.1 > current.0 {
                    // A long piece can be shared in two places: the room between stays free.
                    if let Some(stretch) = open
                        && current.0 > stretch.1
                    {
                        shared.push(stretch);
                        open = None;
                    }
                    let overlap_end = previous.1.min(current.1);
                    open = Some(match open {
                        None => (current.0, overlap_end),
                        Some(stretch) => (stretch.0, stretch.1.max(overlap_end)),
                    });
                } else if let Some(stretch) = open.take() {
                    shared.push(stretch);
                }
                // Keep the piece that reaches furthest.
                if current.1 > previous.1 {
                    previous = current;
                }
            }
            shared.extend(open);
            for (start, end) in shared {
                boxes.push(if is_level {
                    Rect {
                        top_left: Point { x: start, y: line.value() - SHARED_SEGMENT_CLEARANCE },
                        width: end - start,
                        height: 2.0 * SHARED_SEGMENT_CLEARANCE,
                    }
                } else {
                    Rect {
                        top_left: Point { x: line.value() - SHARED_SEGMENT_CLEARANCE, y: start },
                        width: 2.0 * SHARED_SEGMENT_CLEARANCE,
                        height: end - start,
                    }
                });
            }
        }
    }
    boxes
}

/// TALA `isClusterPathSharedChecked`: the piece of the route at the other card (not the card of
/// the cluster) lies on the same piece of the route of another card of the cluster to that
/// card. The cluster of the connection's start is asked first.
fn cluster_path_shared(graph: &Graph, routes: &[Vec<Point>], edge: usize) -> bool {
    let wire = graph.edges[edge];
    let (cluster, other_card, from_cluster) = match (graph.cluster_of(wire.from), graph.cluster_of(wire.to)) {
        (Some(cluster), _) => (cluster, wire.to, true),
        (None, Some(cluster)) => (cluster, wire.from, false),
        (None, None) => return false,
    };
    // Coming from a cluster only the last piece can be shared, going to one only the first.
    let piece = |route: &[Point]| if from_cluster { (route[route.len() - 1], route[route.len() - 2]) } else { (route[0], route[1]) };
    let (p1, p2) = piece(&routes[edge]);
    for &member in &graph.clusters[cluster].members {
        for (other, other_wire) in graph.indexed_edges_of(member) {
            if other == edge || other_wire.other(member) != other_card {
                continue;
            }
            let (o1, o2) = piece(&routes[other]);
            if p1.x == p2.x && o1.x == o2.x && p1.x == o1.x {
                if p1.y.max(p2.y) >= o1.y.min(o2.y) && o1.y.max(o2.y) >= p1.y.min(p2.y) {
                    return true;
                }
            } else if p1.y == p2.y && o1.y == o2.y && p1.y == o1.y && p1.x.max(p2.x) >= o1.x.min(o2.x) && o1.x.max(o2.x) >= p1.x.min(p2.x) {
                return true;
            }
        }
    }
    false
}

/// TALA `clusterLabelPlacementOrientation` and what `findBestEdgeLabelPosition` makes of it: a
/// label of a route that shares its path with other routes of the cluster goes to the side
/// away from the cluster's middle, so that the labels of the cluster lie symmetrically. None
/// for the card in the middle, or where the other card lies on a diagonal.
fn cluster_side(graph: &Graph, edge: usize) -> Option<Position> {
    let wire = graph.edges[edge];
    let (member, card, from_cluster) = match graph.cluster_of(wire.from) {
        Some(_) => (wire.from, wire.to, true),
        None => (wire.to, wire.from, false),
    };
    let members = &graph.clusters[graph.cluster_of(member)?].members;
    let (mut tl, mut br) = (Point { x: f64::INFINITY, y: f64::INFINITY }, Point { x: f64::NEG_INFINITY, y: f64::NEG_INFINITY });
    for node in members.iter().map(|member| &graph.nodes[*member]) {
        tl = Point { x: tl.x.min(node.top_left.x), y: tl.y.min(node.top_left.y) };
        br = Point { x: br.x.max((node.top_left.x + node.width).round()), y: br.y.max((node.top_left.y + node.height).round()) };
    }
    let shell = Node { top_left: tl, width: br.x - tl.x, height: br.y - tl.y };
    let index = members.iter().position(|other| *other == member)? as f64;
    let middle = (members.len() - 1) as f64 / 2.0;
    let (before, after) = match (graph.nodes[card].orientation(&shell), from_cluster) {
        (Orientation::Right | Orientation::Top, true) | (Orientation::Left | Orientation::Bottom, false) => (Position::UnlockedTop, Position::UnlockedBottom),
        (Orientation::Left | Orientation::Bottom, true) | (Orientation::Right | Orientation::Top, false) => (Position::UnlockedBottom, Position::UnlockedTop),
        _ => return None,
    };
    if index < middle {
        Some(before)
    } else if index > middle {
        Some(after)
    } else {
        None
    }
}

/// TALA `labelPercentageSearchRange`: where along the route a label beside it is looked for.
/// Coming from a card of a cluster only along the first piece, going to one only along the
/// last: the pieces the routes of the cluster do not share. Elsewhere the whole route.
fn search_range(graph: &Graph, route: &[Point], edge: usize, size: LabelSize, length: f64) -> (f64, f64) {
    let (half_height, half_width) = (size.height / 2.0, size.width / 2.0);
    let wire = graph.edges[edge];
    if graph.cluster_of(wire.from).is_some() {
        let (p1, p2) = (route[0], route[1]);
        if p1.x == p2.x && (p1.y - p2.y).abs() > half_height {
            return (0.0, (((p1.y - p2.y).abs() - half_height) / length).min(1.0));
        } else if (p1.x - p2.x).abs() > half_width {
            return (0.0, (((p1.x - p2.x).abs() - half_width) / length).min(1.0));
        }
    } else if graph.cluster_of(wire.to).is_some() {
        let (p1, p2) = (route[route.len() - 1], route[route.len() - 2]);
        if p1.x == p2.x && (p1.y - p2.y).abs() > half_height {
            return (((length - (p1.y - p2.y).abs() + half_height) / length).max(0.0), 1.0);
        } else if (p1.x - p2.x).abs() > half_width {
            return (((length - (p1.x - p2.x).abs() + half_width) / length).max(0.0), 1.0);
        }
    }
    (0.0, 1.0)
}

/// TALA `findBestEdgeLabelPosition` for a plain connection: every position in the order of
/// preference, and for the unlocked ones every place along the route; the one that covers the
/// least wins, an earlier one among equals. The search ends at a place that covers nothing.
fn best_place(
    graph: &Graph,
    routes: &[Vec<Point>],
    edge: usize,
    size: LabelSize,
    placed: &[Rect],
    shared: &[Rect],
) -> Option<(Placed, Rect)> {
    let route = &routes[edge];
    let cards: Vec<Rect> = graph.nodes.iter().map(Rect::of).collect();
    let mut best: Option<(Placed, Rect)> = None;
    let mut best_score = f64::INFINITY;

    let wire = graph.edges[edge];
    let doubled: Vec<usize> = match (graph.cluster_of(wire.from), graph.cluster_of(wire.to)) {
        (Some(cluster), _) => (0..routes.len()).filter(|other| *other != edge && graph.cluster_of(graph.edges[*other].from) == Some(cluster)).collect(),
        (None, Some(cluster)) => (0..routes.len()).filter(|other| *other != edge && graph.cluster_of(graph.edges[*other].to) == Some(cluster)).collect(),
        (None, None) => Vec::new(),
    };

    let mut check = |position: Position, percentage: f64, best_score: &mut f64| {
        let top_left = label_top_left(route, position, percentage, size);
        let label = Rect { top_left, width: size.width, height: size.height };
        let area = size.width * size.height;

        // Cards: every card closer than the padding counts once with the label's whole area, and
        // again with the area it really covers.
        let (mut card_area, mut card_count) = (0.0, 0);
        for card in &cards {
            if !label.overlaps(card, LABEL_PADDING) {
                continue;
            }
            if label.overlaps(card, 0.0) {
                card_area += label.overlap_area(card);
                card_count += 1;
            }
            card_area += area;
            card_count += 1;
        }
        // Routes: every piece closer than the padding, and half of those it really touches. A
        // label on its own line does not count that line. The routes of the other cards of the
        // cluster the connection comes from (or goes to) count twice.
        let pieces_touched = |delta: f64| {
            routes
                .iter()
                .enumerate()
                .filter(|(other, _)| !(position.is_on_edge() && *other == edge))
                .chain(doubled.iter().map(|other| (*other, &routes[*other])))
                .flat_map(|(_, points)| points.windows(2))
                .filter(|piece| label.overlaps_line(piece[0], piece[1], delta))
                .count()
        };
        let edge_overlaps = pieces_touched(LABEL_PADDING) + (pieces_touched(0.0) as f64 * 0.5).ceil() as usize;
        // Labels placed before, the same way; a label really on another one counts much more.
        let labels_touched = |delta: f64| placed.iter().filter(|other| label.overlaps(other, delta)).count();
        let label_overlaps = labels_touched(0.0);
        let almost_label_overlaps = labels_touched(LABEL_PADDING) + (label_overlaps as f64 * 0.5).ceil() as usize;
        let shared_overlaps = shared.iter().filter(|other| label.overlaps(other, LABEL_PADDING)).count();

        // TALA `scoreEdgeLabelOverlaps`.
        let mut score = (card_area / area) * 2.0;
        score += label_overlaps as f64 * 10.0;
        score += almost_label_overlaps as f64;
        score += f64::from(card_count) * 2.0;
        score += edge_overlaps as f64 * 2.0;
        score += shared_overlaps as f64;
        if score < *best_score {
            *best_score = score;
            best = Some((Placed { position, percentage, top_left }, label));
        }
    };

    // Where the tree stage wants the label, that place comes first, then the other side of the
    // line, then the line itself; the positions at the route's middle follow. Such a label
    // keeps the share of the route the tree stage gave it.
    let preset = graph.label_presets.get(edge).copied().flatten().filter(|(position, _)| position.is_unlocked());
    // A connection of a card of a cluster whose route shares its path with the routes of the
    // other cards of the cluster: the side away from the cluster's middle first, then the line.
    let clustered = graph.cluster_of(wire.from).is_some() || graph.cluster_of(wire.to).is_some();
    let positions: Vec<Position> = match preset {
        Some((Position::UnlockedMiddle, _)) => [Position::UnlockedMiddle, Position::UnlockedTop, Position::UnlockedBottom].into_iter().chain(PREFERENCE.into_iter().take(9)).collect(),
        Some((position, _)) => [position, position.mirrored(), Position::UnlockedMiddle].into_iter().chain(PREFERENCE.into_iter().take(9)).collect(),
        None if clustered && cluster_path_shared(graph, routes, edge) => cluster_side(graph, edge).into_iter().chain([Position::UnlockedMiddle]).chain(PREFERENCE.into_iter().take(9)).collect(),
        None => PREFERENCE.to_vec(),
    };
    let share = preset.map_or(0.0, |(_, share)| share);
    let (start, end) = search_range(graph, route, edge, size, route_length(route));
    for position in positions {
        if best_score == 0.0 {
            break;
        }
        if position.is_unlocked() && share != 0.0 {
            check(position, share, &mut best_score);
        } else if position.is_unlocked() {
            let mut percentage = start;
            while percentage < end {
                check(position, percentage, &mut best_score);
                if best_score == 0.0 {
                    break;
                }
                percentage += SEARCH_INTERVAL;
            }
        } else {
            check(position, 0.0, &mut best_score);
        }
    }
    // Nothing was scored only for a route without length.
    best
}

/// TALA `placeNewEdges` for a layout whose connections were all just routed: the labels get their
/// places one after the other, bent routes first and shorter ones before longer ones; each keeps
/// clear of the ones placed before it. `sizes` holds the label of every connection, if it has
/// one.
pub fn place_labels(graph: &Graph, routes: &[Vec<Point>], sizes: &[Option<LabelSize>]) -> Vec<Option<Placed>> {
    let shared = shared_segment_boxes(routes);
    let mut order: Vec<usize> = (0..routes.len()).filter(|edge| sizes[*edge].is_some()).collect();
    // TALA `sortLabelPlacementEdges`: straight lines of two points go last.
    let lengths: Vec<f64> = routes.iter().map(|route| route_length(route)).collect();
    order.sort_by(|a, b| {
        (routes[*a].len() == 2).cmp(&(routes[*b].len() == 2)).then(lengths[*a].total_cmp(&lengths[*b]))
    });
    let mut placed_boxes: Vec<Rect> = Vec::new();
    let mut places = vec![None; routes.len()];
    for edge in order {
        let size = sizes[edge].expect("only connections with a label are in the order");
        if let Some((place, label)) = best_place(graph, routes, edge, size, &placed_boxes, &shared) {
            placed_boxes.push(label);
            places[edge] = Some(place);
        }
    }
    places
}

/// TALA `reorderDuplicatesInEdgesGuarded`: of three or more straight connections between the same
/// two cards, one with a label that lies between the others swaps its route with an outer one
/// that has no label, so the label has room on the outside.
pub fn reorder_duplicates(graph: &Graph, routes: &mut [Vec<Point>], has_label: &[bool]) {
    let is_straight = |route: &[Point]| route.len() >= 2 && route.windows(3).all(|p| side(p[0], p[1], p[2]) == 0.0);
    // Another connection starts or ends at the point where this one starts or ends.
    let has_overlapping_end = |routes: &[Vec<Point>], edge: usize| {
        let wire = graph.edges[edge];
        let (start, end) = (routes[edge][0], routes[edge][routes[edge].len() - 1]);
        let touches = |card: usize, point: Point| {
            graph.edges.iter().enumerate().any(|(other, other_wire)| {
                other != edge
                    && (other_wire.from == card || other_wire.to == card)
                    && (routes[other][0] == point || routes[other][routes[other].len() - 1] == point)
            })
        };
        touches(wire.from, start) || touches(wire.to, end)
    };
    for edge in 0..routes.len() {
        if !has_label[edge] || !is_straight(&routes[edge]) {
            continue;
        }
        let wire = graph.edges[edge];
        let overlapping_end = has_overlapping_end(routes, edge);
        let mut duplicates: Vec<usize> = Vec::new();
        for (other, other_wire) in graph.edges.iter().enumerate() {
            let same_cards = (other_wire.from == wire.from && other_wire.to == wire.to)
                || (other_wire.from == wire.to && other_wire.to == wire.from);
            if other == edge || !same_cards || !is_straight(&routes[other]) {
                continue;
            }
            // Where ends meet, only connections that point the same way count: arrows that
            // share a point must match.
            if (overlapping_end || has_overlapping_end(routes, other)) && other_wire.from == wire.to {
                continue;
            }
            duplicates.push(other);
        }
        if duplicates.len() <= 1 {
            continue;
        }
        let (start, end) = (routes[edge][0], routes[edge][routes[edge].len() - 1]);
        let upright = end.y - start.y > end.x - start.x;
        let across = |p: Point| if upright { p.x } else { p.y };
        let group: Vec<usize> = std::iter::once(edge).chain(duplicates).collect();
        let mut source_order = group.clone();
        source_order.sort_by(|a, b| across(routes[*a][0]).total_cmp(&across(routes[*b][0])));
        let mut target_order = group;
        target_order
            .sort_by(|a, b| across(routes[*a][routes[*a].len() - 1]).total_cmp(&across(routes[*b][routes[*b].len() - 1])));
        if source_order != target_order {
            continue;
        }
        let at = source_order.iter().position(|member| *member == edge).expect("the connection is in its group");
        if at == 0 || at == source_order.len() - 1 {
            continue;
        }
        let (first, last) = (source_order[0], source_order[source_order.len() - 1]);
        let outer = if !has_label[last] {
            last
        } else if !has_label[first] {
            first
        } else {
            continue;
        };
        routes.swap(edge, outer);
        if graph.edges[outer].from == wire.to {
            routes[edge].reverse();
            routes[outer].reverse();
        }
    }
}
