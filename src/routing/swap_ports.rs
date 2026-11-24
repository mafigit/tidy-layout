// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.

//! The stage that swaps the ports of two connections of a card where they cross right beside
//! it. Two connections that leave on the same side exchange their ports. Two that leave on
//! sides next to each other exchange their first pieces, if that makes one of them simpler.
//! Ported from TALA `internal/routing`: `swap_ports.go` (`SwapAllEdgePorts`,
//! `swapEdgePortsGuarded`, `swapEdgesOnSameSide`, `swapEdgesOnAdjacentSides`, `edgesIntersect`,
//! `canSwapPorts`, `doesNotPassThroughNodesGuarded`, `refineEdgeGuarded`,
//! `makeStraightLineGuarded` and the shapes), and D2 `lib/geo/point.go` (`IntersectionPoint`).

use crate::graph::{Graph, Node, Point};
use crate::routing::geometry::Orientation;
use crate::routing::tunnel::ranges_between;

/// TALA `pathNodeProximityFloor`.
const PROXIMITY_FLOOR: f64 = 5.0;

/// TALA `pointsAt`: where in the route of a connection its first three points are, seen from
/// the card: the port, the first bend, the second.
fn at(graph: &Graph, routes: &[Vec<Point>], edge: usize, card: usize) -> [usize; 3] {
    let last = routes[edge].len() - 1;
    if graph.edges[edge].from == card { [0, 1, 2] } else { [last, last - 1, last - 2] }
}

fn points_at(graph: &Graph, routes: &[Vec<Point>], edge: usize, card: usize) -> [Point; 3] {
    at(graph, routes, edge, card).map(|index| routes[edge][index])
}

/// TALA `doesNotPassThroughNodesGuarded`: neither line comes nearer than 10 to a card other
/// than `own`.
fn clear_of_cards(graph: &Graph, own: &[usize], lines: [(Point, Point); 2]) -> bool {
    let pad = 2.0 * PROXIMITY_FLOOR;
    graph.nodes.iter().enumerate().filter(|(card, _)| !own.contains(card)).all(|(_, node)| {
        let padded = Node { top_left: Point { x: node.top_left.x - pad, y: node.top_left.y - pad }, width: node.width + 2.0 * pad, height: node.height + 2.0 * pad };
        lines.iter().all(|(from, to)| !padded.passes_through(*from, *to))
    })
}

/// TALA `edgesIntersect`: the two connections, which leave the card on the same side, cross
/// beside it.
fn cross_at(first: [Point; 3], second: [Point; 3], side: Orientation) -> bool {
    let ([p1, p2, p3], [q1, q2, q3]) = (first, second);
    match side {
        Orientation::Left => (q2.x < p2.x && p3.y > q1.y) || (p2.x < q2.x && q3.y < p1.y),
        Orientation::Right => (p2.x > q2.x && q3.y < p1.y) || (q2.x > p2.x && p3.y > q1.y),
        Orientation::Bottom => (p2.y > q2.y && q3.x < p1.x) || (q2.y > p2.y && p3.x > q1.x),
        Orientation::Top => (p2.y < q2.y && q3.x < p1.x) || (q2.y < p2.y && p3.x > q1.x),
        _ => false,
    }
}

/// D2 `IntersectionPoint`: where the two lines meet, if they do.
pub(crate) fn intersection(u0: Point, u1: Point, v0: Point, v1: Point) -> Option<Point> {
    let (udx, vdx, uvdx) = (u1.x - u0.x, v1.x - v0.x, v0.x - u0.x);
    let (udy, vdy, uvdy) = (u1.y - u0.y, v1.y - v0.y, v0.y - u0.y);
    // (TALA gives up here on lines that never meet. Their two shares below are no numbers
    // between 0 and 1, so the next question says the same.)
    let denominator = udy * vdx - udx * vdy;
    let s = (vdx * uvdy - vdy * uvdx) / denominator;
    let t = (udx * uvdy - udy * uvdx) / denominator;
    if !(0.0..=1.0).contains(&s) || !(0.0..=1.0).contains(&t) {
        return None;
    }
    Some(Point { x: u0.x + (s * udx).round(), y: u0.y + (s * udy).round() })
}

fn sign(value: f64) -> i8 {
    if value < 0.0 {
        -1
    } else if value > 0.0 {
        1
    } else {
        0
    }
}

/// TALA `isUShaped`: four points, the two ends on the same side of the middle piece.
fn is_u_shaped(points: &[Point]) -> bool {
    if points.len() != 4 {
        return false;
    }
    if points[0].x == points[1].x {
        sign(points[0].y - points[1].y) == sign(points[3].y - points[2].y) && points[1].y == points[2].y
    } else {
        sign(points[0].x - points[1].x) == sign(points[3].x - points[2].x) && points[1].x == points[2].x
    }
}

/// TALA `isSShaped`.
fn is_s_shaped(points: &[Point]) -> bool {
    points.len() == 4 && !is_u_shaped(points)
}

/// TALA `isLShaped`.
fn is_l_shaped(points: &[Point]) -> bool {
    if points.len() != 3 {
        return false;
    }
    let (down, across) = (points[0].x == points[1].x, points[1].y == points[2].y);
    down == across
}

/// TALA `occupiedPortsGuarded`: where the connections of a card leave it. (Every connection has
/// a route when this stage runs.)
fn occupied_ports(graph: &Graph, routes: &[Vec<Point>], card: usize) -> Vec<Point> {
    graph.indexed_edges_of(card).map(|(edge, link)| if link.from == card { routes[edge][0] } else { routes[edge][routes[edge].len() - 1] }).collect()
}

/// TALA `makeStraightLineGuarded`: the connection becomes one straight line where its two
/// cards see each other, at the first place in steps of 5 where both ports are free.
fn make_straight_line(graph: &Graph, routes: &mut [Vec<Point>], edge: usize, occupied: &[Point]) -> bool {
    let (from_port, to_port) = (routes[edge][0], routes[edge][routes[edge].len() - 1]);
    let Some((ranges, horizontal)) = ranges_between(graph, graph.edges[edge].from, graph.edges[edge].to) else {
        return false;
    };
    for range in ranges {
        // Not along the border of a card.
        let mut value = range.start + PROXIMITY_FLOOR;
        while value <= range.end - PROXIMITY_FLOOR {
            let (from, to) = if horizontal { (Point { x: from_port.x, y: value }, Point { x: to_port.x, y: value }) } else { (Point { x: value, y: from_port.y }, Point { x: value, y: to_port.y }) };
            if !occupied.contains(&from) && !occupied.contains(&to) {
                routes[edge] = vec![from, to];
                return true;
            }
            value += PROXIMITY_FLOOR;
        }
    }
    false
}

/// TALA `refineEdgeGuarded`: a route with two bends or one becomes a straight line where it
/// can. A longer route that starts with two bends loses one of them, if the shorter way comes
/// near no card. Says whether the route changed.
fn refine(graph: &Graph, routes: &mut [Vec<Point>], edge: usize) -> bool {
    let (from, to) = (graph.edges[edge].from, graph.edges[edge].to);
    // (TALA leaves a connection from a card to itself and a route without a bend alone here.
    // The engine is given no such connection, and a route without a bend has none of the
    // shapes below.)
    // A connection of a card of a tree keeps its route.
    if graph.hangs_in_tree(edge) {
        return false;
    }
    let mut occupied = occupied_ports(graph, routes, from);
    occupied.extend(occupied_ports(graph, routes, to));
    if is_s_shaped(&routes[edge]) || is_l_shaped(&routes[edge]) {
        return make_straight_line(graph, routes, edge, &occupied);
    }
    // (TALA asks for a U shape first: it has four points and is no S shape.)
    if routes[edge].len() < 4 || !is_s_shaped(&routes[edge][..4]) {
        return false;
    }
    // TALA `sToLShapedBendPoint`.
    let start = &routes[edge][..4];
    let bend = if start[0].x == start[1].x { Point { x: start[1].x, y: start[3].y } } else { Point { x: start[3].x, y: start[1].y } };
    if !clear_of_cards(graph, &[from, to], [(start[0], bend), (bend, start[3])]) {
        return false;
    }
    routes[edge].splice(1..4, [bend]);
    true
}

/// TALA `swapEdgePortsGuarded` for one card.
fn swap_at(graph: &Graph, routes: &mut [Vec<Point>], card: usize) {
    // The connections with a bend, by the side they leave the card on, and how many leave at
    // each port.
    const SIDES: [Orientation; 4] = [Orientation::Top, Orientation::Bottom, Orientation::Left, Orientation::Right];
    let mut by_side: [Vec<usize>; 4] = Default::default();
    let mut ports: Vec<Point> = Vec::new();
    for (edge, _) in graph.indexed_edges_of(card) {
        // (TALA skips a connection from a card to itself. The engine is given none.)
        if routes[edge].len() < 3 {
            continue;
        }
        let [port, bend, _] = points_at(graph, routes, edge, card);
        let side = if port.x < bend.x {
            Orientation::Right
        } else if port.x > bend.x {
            Orientation::Left
        } else if port.y < bend.y {
            Orientation::Bottom
        } else {
            Orientation::Top
        };
        by_side[SIDES.iter().position(|other| *other == side).expect("one of four sides")].push(edge);
        ports.push(port);
    }
    // TALA `canSwapPorts`: each of the two ports has one connection only, or both connections
    // have the same end at the card (an arrowhead or none).
    let can_swap = |routes: &[Vec<Point>], first: usize, second: usize| {
        let alone = |edge: usize| ports.iter().filter(|port| **port == points_at(graph, routes, edge, card)[0]).count() == 1;
        (alone(first) && alone(second)) || (graph.edges[first].to == card) == (graph.edges[second].to == card)
    };

    // TALA `swapEdgesOnSameSide`. (TALA takes the sides in the order of a map. They do not
    // depend on each other.)
    for (side, edges) in SIDES.iter().zip(by_side.iter_mut()) {
        let upright = matches!(side, Orientation::Top | Orientation::Bottom);
        let along = |point: Point| if upright { point.x } else { point.y };
        // By the port, then by the second bend. TALA's sort keeps equal ones in order.
        edges.sort_by(|first, second| {
            let ([p1, _, p3], [q1, _, q3]) = (points_at(graph, routes, *first, card), points_at(graph, routes, *second, card));
            along(p1).total_cmp(&along(q1)).then(along(p3).total_cmp(&along(q3)))
        });
        for index in 0..edges.len().saturating_sub(1) {
            let (first, second) = (edges[index], edges[index + 1]);
            if !cross_at(points_at(graph, routes, first, card), points_at(graph, routes, second, card), *side) || !can_swap(routes, first, second) {
                continue;
            }
            let before = (routes[first].clone(), routes[second].clone());
            let ([a1, a2, _], [b1, b2, _]) = (at(graph, routes, first, card), at(graph, routes, second, card));
            // TALA `swapCoordinate`: the two change places along the side.
            for (a, b) in [(a1, b1), (a2, b2)] {
                let (p, q) = (routes[first][a], routes[second][b]);
                if upright {
                    (routes[first][a].x, routes[second][b].x) = (q.x, p.x);
                } else {
                    (routes[first][a].y, routes[second][b].y) = (q.y, p.y);
                }
            }
            if clear_of_cards(graph, &[card], [(routes[first][a1], routes[first][a2]), (routes[second][b1], routes[second][b2])]) {
                edges.swap(index, index + 1);
            } else {
                (routes[first], routes[second]) = before;
            }
        }
    }

    // TALA `swapEdgesOnAdjacentSides`.
    let upright: Vec<usize> = by_side[0].iter().chain(&by_side[1]).copied().collect();
    let level: Vec<usize> = by_side[2].iter().chain(&by_side[3]).copied().collect();
    for &a in &upright {
        for &b in &level {
            // A route may have become a straight line meanwhile.
            if routes[a].len() <= 2 {
                break;
            }
            if routes[b].len() <= 2 || !can_swap(routes, a, b) {
                continue;
            }
            let ([_, a2, a3], [_, b2, b3]) = (points_at(graph, routes, a, card), points_at(graph, routes, b, card));
            let Some(crossing) = intersection(a2, a3, b2, b3) else {
                continue;
            };
            let before = (routes[a].clone(), routes[b].clone());
            // The two exchange their first pieces, and each gets the crossing as a bend.
            let ([a_port, a_bend, _], [b_port, b_bend, _]) = (at(graph, routes, a, card), at(graph, routes, b, card));
            (routes[a][a_port], routes[b][b_port]) = (routes[b][b_port], routes[a][a_port]);
            (routes[a][a_bend], routes[b][b_bend]) = (routes[b][b_bend], routes[a][a_bend]);
            for edge in [a, b] {
                let from_card = graph.edges[edge].from == card;
                let place = if from_card { 2 } else { routes[edge].len() - 2 };
                routes[edge].insert(place, crossing);
                // The piece through the crossing moves aside a little, so that it does not look
                // like a crossing any more.
                let (bend, added) = if from_card { (1, 2) } else { (routes[edge].len() - 2, routes[edge].len() - 3) };
                let offset = PROXIMITY_FLOOR / 2.0;
                if routes[edge][added].x == routes[edge][bend].x {
                    routes[edge][added].x += offset;
                    routes[edge][bend].x += offset;
                } else {
                    routes[edge][added].y -= offset;
                    routes[edge][bend].y -= offset;
                }
            }
            let a_better = refine(graph, routes, a);
            let b_better = refine(graph, routes, b);
            if !a_better && !b_better {
                // Two bends more in place of one crossing: not worth it.
                (routes[a], routes[b]) = before;
            }
        }
    }
}

/// TALA `SwapAllEdgePorts`: every card in turn.
pub fn swap_ports(graph: &Graph, routes: &mut [Vec<Point>]) {
    for card in 0..graph.nodes.len() {
        swap_at(graph, routes, card);
    }
}
