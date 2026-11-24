// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.

//! A straight line between a port of each of two cards, and when it replaces a route. Ported from
//! TALA `internal/routing`: `coordinator.go` (`routeLineChecked`, `tryStraightEdgeFallback`,
//! `hasSharpAngleToBorder`, `findOverlappingEdges`, `maxSideLength`), `cost.go`
//! (`estimateRouteCost`) and `internal/layoutgraph` (`portsByOrientation`, `portEdges`,
//! `isDuplicateOf`). For connections with an arrow at their target and no label; Go's `math.Pow`
//! is repeated step by step so that prices come out bit for bit.

use crate::graph::{Graph, Node, Point};
use crate::routing::geometry::{Orientation, euclidean_distance, segments_intersect};
use crate::routing::router::Costs;
use crate::routing::tuning::{
    AXIS_ALIGNMENT_TOLERANCE, CROSSING_COST_WEIGHT, LOWER_JITTER_THRESHOLD, MIN_STRAIGHT_EDGE_ANGLE,
    NON_ORTHOGONAL_FACTOR,
};

/// Another connection that already has a route: its two cards and the points of its route, from
/// port to port.
pub struct Routed<'a> {
    pub from: usize,
    pub to: usize,
    pub points: &'a [Point],
}

/// The twelve ports of a card: three on its top, left, bottom and right side, in that order.
/// TALA `Node.ports`.
pub(crate) fn ports(node: &Node) -> [Point; 12] {
    const SHARES: [f64; 3] = [0.25, 0.5, 0.75];
    let at = |x_share: f64, y_share: f64| Point {
        x: node.top_left.x + (node.width * x_share).round(),
        y: node.top_left.y + (node.height * y_share).round(),
    };
    let mut ports = [node.top_left; 12];
    for (i, share) in SHARES.into_iter().enumerate() {
        ports[i] = at(share, 0.0);
        ports[3 + i] = at(0.0, share);
        ports[6 + i] = at(share, 1.0);
        ports[9 + i] = at(1.0, share);
    }
    ports
}

/// TALA `shapeSquare.PortIndices`: which ports look in a direction.
pub(crate) fn port_indices(orientation: Orientation) -> &'static [usize] {
    match orientation {
        Orientation::Top => &[0, 1, 2],
        Orientation::Left => &[3, 4, 5],
        Orientation::Bottom => &[6, 7, 8],
        Orientation::Right => &[9, 10, 11],
        Orientation::TopLeft => &[0, 1, 2, 3, 4, 5],
        Orientation::TopRight => &[0, 1, 2, 9, 10, 11],
        Orientation::BottomLeft => &[6, 7, 8, 3, 4, 5],
        Orientation::BottomRight => &[6, 7, 8, 9, 10, 11],
        Orientation::None => &[],
    }
}

fn opposite(orientation: Orientation) -> Orientation {
    match orientation {
        Orientation::TopLeft => Orientation::BottomRight,
        Orientation::TopRight => Orientation::BottomLeft,
        Orientation::BottomLeft => Orientation::TopRight,
        Orientation::BottomRight => Orientation::TopLeft,
        Orientation::Top => Orientation::Bottom,
        Orientation::Bottom => Orientation::Top,
        Orientation::Left => Orientation::Right,
        Orientation::Right => Orientation::Left,
        Orientation::None => Orientation::None,
    }
}

/// Go's `math.Pow(1.4, bends)`, with the same multiplications in the same order.
fn turn_penalty(bends: usize) -> f64 {
    // 1.4 is 0.7 times 2 to the power of 1.
    let (mut mantissa, mut exponent) = (0.7_f64, 1_i32);
    let (mut result, mut result_exponent) = (1.0_f64, 0_i32);
    let mut left = bends;
    while left != 0 {
        if left & 1 == 1 {
            result *= mantissa;
            result_exponent += exponent;
        }
        mantissa *= mantissa;
        exponent <<= 1;
        if mantissa < 0.5 {
            mantissa += mantissa;
            exponent -= 1;
        }
        left >>= 1;
    }
    result * 2.0_f64.powi(result_exponent)
}

/// Positive for a turn to the right, negative to the left, 0 for three points on one line.
pub(crate) fn side(a: Point, b: Point, c: Point) -> f64 {
    (b.y - a.y) * (c.x - a.x) - (b.x - a.x) * (c.y - a.y)
}

fn equal_signs(first: f64, second: f64) -> bool {
    (first > 0.0 && second > 0.0) || (first == 0.0 && second == 0.0) || (first < 0.0 && second < 0.0)
}

/// TALA `isNonSharedCrossing`: the two pieces cross at an angle, and not just where an upright or
/// level piece ends on the line of the other (a bend off a shared stretch).
pub(crate) fn crosses(a: Point, b: Point, c: Point, d: Point) -> bool {
    if equal_signs(side(a, b, c), side(a, b, d)) || equal_signs(side(c, d, a), side(c, d, b)) {
        return false;
    }
    if a.x == b.x && (a.x == c.x || a.x == d.x) {
        return false;
    }
    if a.y == b.y && (a.y == c.y || a.y == d.y) {
        return false;
    }
    true
}

/// TALA `estimateRouteCost`: the length of a route, times 1.4 for every bend, plus a little for
/// every crossing with another route.
pub fn estimate_route_cost(points: &[Point], others: &[Routed]) -> f64 {
    let length = points.windows(2).fold(0.0, |sum, piece| {
        sum + euclidean_distance(piece[0].x, piece[0].y, piece[1].x, piece[1].y)
    });
    let mut crossings = 0_u32;
    for other in others {
        for piece in points.windows(2) {
            for other_piece in other.points.windows(2) {
                if crosses(piece[0], piece[1], other_piece[0], other_piece[1]) {
                    crossings += 1;
                }
            }
        }
    }
    length * turn_penalty(points.len().saturating_sub(2)) + CROSSING_COST_WEIGHT * f64::from(crossings)
}

/// TALA `hasSharpAngleToBorder`: the line meets a side of the card it comes from (or goes to) at
/// a flatter angle than allowed. `orientation` says where that card lies.
fn has_sharp_angle_to_border(from: Point, to: Point, orientation: Orientation) -> bool {
    let (dx, dy) = (to.x - from.x, to.y - from.y);
    let radians = match orientation {
        Orientation::Top | Orientation::Bottom => dy.atan2(dx),
        Orientation::Left | Orientation::Right => dx.atan2(dy),
        _ => return false,
    };
    let mut angle = (180.0 * radians / std::f64::consts::PI).abs();
    if angle > 90.0 {
        angle = 180.0 - angle;
    }
    angle < MIN_STRAIGHT_EDGE_ANGLE
}

/// TALA `routeLineChecked`: the cheapest straight line from a port of the first card to a port of
/// the second, on the sides that face each other, with its price. `None` when no line is
/// possible.
///
/// A port is shared only with connections that start at the same card (for the first card) or
/// end at it (for the second), and never with another connection between the same two cards. The
/// line must not pass through a third card, nor run along a route it may not share.
pub fn route_line(
    graph: &Graph,
    non_center_port_cost: f64,
    from: usize,
    to: usize,
    others: &[Routed],
) -> Option<(Point, Point, f64)> {
    let (from_card, to_card) = (&graph.nodes[from], &graph.nodes[to]);
    let from_orientation = from_card.orientation(to_card);
    let to_orientation = to_card.orientation(from_card);
    if from_orientation == Orientation::None || to_orientation == Orientation::None {
        return None;
    }
    let (all_from, all_to) = (ports(from_card), ports(to_card));
    let is_center = |all: &[Point; 12], port: Point| [1, 4, 7, 10].into_iter().any(|i| all[i] == port);

    // The routes that use a port of a card, and how. TALA `portEdges`.
    let at_port = |card: usize, port: Point| {
        others.iter().filter(move |other| {
            if other.from == card {
                other.points.first() == Some(&port)
            } else {
                other.to == card && other.points.last() == Some(&port)
            }
        })
    };
    let is_duplicate = |other: &Routed| (other.from == from && other.to == to) || (other.from == to && other.to == from);

    let mut best: Option<(Point, Point, f64)> = None;
    for &from_index in port_indices(opposite(from_orientation)) {
        let from_port = all_from[from_index];
        // Our connections have no arrow at their start: the port is shared only with connections
        // that start here too, and sharing it costs nothing.
        if at_port(from, from_port).any(|other| is_duplicate(other) || other.from != from) {
            continue;
        }
        for &to_index in port_indices(opposite(to_orientation)) {
            let to_port = all_to[to_index];
            let mut to_port_cost = 0.0;
            if at_port(to, to_port).next().is_some() {
                // An arrow shares its port only with arrows that end here too.
                if at_port(to, to_port).any(|other| is_duplicate(other) || other.to != to) {
                    continue;
                }
                to_port_cost = match to_orientation {
                    Orientation::Top | Orientation::Bottom => to_card.width,
                    Orientation::Left | Orientation::Right => to_card.height,
                    _ => to_card.width.max(to_card.height),
                } / 2.0;
            }
            let through_a_card = graph
                .nodes
                .iter()
                .enumerate()
                .any(|(card, node)| card != from && card != to && node.passes_through(from_port, to_port));
            if through_a_card {
                continue;
            }
            // TALA `findOverlappingEdges`: routes with an upright or level piece on the line.
            let upright = from_port.x == to_port.x;
            let level = from_port.y == to_port.y;
            // Ours, not TALA's: the editor wants every piece of a connector upright or level.
            if !graph.slanted_lines && !upright && !level {
                continue;
            }
            let may_not_share = others.iter().any(|other| {
                let overlaps = other.points.windows(2).any(|piece| {
                    ((upright && piece[0].x == piece[1].x) || (level && piece[0].y == piece[1].y))
                        && segments_intersect(piece[0], piece[1], from_port, to_port)
                });
                overlaps && other.from != from && other.to != to
            });
            if may_not_share {
                continue;
            }

            let mut cost = 0.0 + to_port_cost + estimate_route_cost(&[from_port, to_port], others);
            if !upright && !level {
                let lined_up = (from_port.x - to_port.x).abs() < AXIS_ALIGNMENT_TOLERANCE
                    || (from_port.y - to_port.y).abs() < AXIS_ALIGNMENT_TOLERANCE;
                if !lined_up {
                    cost *= NON_ORTHOGONAL_FACTOR;
                }
            }
            if has_sharp_angle_to_border(from_port, to_port, from_orientation)
                || has_sharp_angle_to_border(from_port, to_port, to_orientation)
            {
                continue;
            }
            if !is_center(&all_from, from_port) {
                cost += non_center_port_cost;
            }
            if !is_center(&all_to, to_port) {
                cost += non_center_port_cost;
            }
            if best.is_none_or(|(_, _, least)| cost < least) {
                best = Some((from_port, to_port, cost));
            }
        }
    }
    best
}

/// Every other connection with its route.
fn others_of<'a>(graph: &Graph, routes: &'a [Vec<Point>], edge: usize) -> Vec<Routed<'a>> {
    graph
        .edges
        .iter()
        .zip(routes)
        .enumerate()
        .filter(|(index, _)| *index != edge)
        .map(|(_, (other, points))| Routed { from: other.from, to: other.to, points })
        .collect()
}

/// What `tryStraightEdgeFallback` compares for one connection: the price of its route, and the
/// price of the straight line that could replace it, if there is one.
pub fn straight_line_costs(graph: &Graph, routes: &[Vec<Point>], edge: usize) -> (f64, Option<f64>) {
    let others = others_of(graph, routes, edge);
    let wire = graph.edges[edge];
    let line = route_line(graph, Costs::of(graph).non_center_port, wire.from, wire.to, &others);
    (estimate_route_cost(&routes[edge], &others), line.map(|(_, _, cost)| cost))
}

/// TALA `tryStraightEdgeFallback`: a connection gets a straight line when that is cheaper than
/// its route. A slanted line counts four times its length, so it wins only over a long detour.
pub fn straighten_one(graph: &Graph, routes: &mut [Vec<Point>], edge: usize) {
    let others = others_of(graph, routes, edge);
    let wire = graph.edges[edge];
    let Some((from_port, to_port, mut line_cost)) =
        route_line(graph, Costs::of(graph).non_center_port, wire.from, wire.to, &others)
    else {
        return;
    };
    let points = &routes[edge];
    // A route with a jog of a few pixels in its middle is as good as straight already: the line
    // that removes the jog is not charged for being slanted.
    if points.len() == 4
        && euclidean_distance(points[1].x, points[1].y, points[2].x, points[2].y) <= LOWER_JITTER_THRESHOLD
    {
        line_cost /= NON_ORTHOGONAL_FACTOR;
    }
    if line_cost < estimate_route_cost(points, &others) {
        routes[edge] = vec![from_port, to_port];
    }
}

/// [`straighten_one`] for every connection in turn; each sees the lines the earlier ones got.
/// A tree connection keeps its route, and so does a connection of a card of a cluster and a
/// connection from a card of a hierarchy (TALA `isStraightEdgeFallbackCandidate`).
pub fn straighten(graph: &Graph, routes: &mut [Vec<Point>]) {
    for edge in 0..graph.edges.len() {
        let wire = graph.edges[edge];
        let clustered = graph.cluster_of(wire.from).is_some() || graph.cluster_of(wire.to).is_some();
        if !clustered && !graph.is_tree_edge(edge) && graph.level(wire.from).is_none() {
            straighten_one(graph, routes, edge);
        }
    }
}
