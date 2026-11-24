// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.

//! The stage that takes a needless hook out of a route. The search can only turn at points of
//! its graph, so a route may run past its goal, step aside and come back. Where the first and
//! the fourth piece of such a hook meet at a free corner, the three bends become one.
//! Ported from TALA `internal/routing`: `edge_simplify.go` (`SimplifyEdgeRoutes`,
//! `simplifyPoints`, `findIntersectionPoint`, `sameRouteDirection`,
//! `lineIntersectsUnrelatedNode`, `orthogonalSegmentEntersNode`, `lineIntersectsOtherEdges`)
//! and `layoutgraph/node.go` (`containsPoint`).

use crate::graph::{Graph, Node, Point};
use crate::routing::geometry::segments_intersect;

fn is_level(a: Point, b: Point) -> bool {
    a.y == b.y
}

fn is_upright(a: Point, b: Point) -> bool {
    a.x == b.x
}

/// TALA `findIntersectionPoint`: the corner where the line of the first piece meets the line of
/// the fourth.
fn corner(p1: Point, p2: Point, p4: Point, p5: Point) -> Option<Point> {
    if is_level(p1, p2) && is_upright(p4, p5) {
        Some(Point { x: p5.x, y: p1.y })
    } else if is_upright(p1, p2) && is_level(p4, p5) {
        Some(Point { x: p1.x, y: p5.y })
    } else {
        None
    }
}

/// TALA `sameRouteDirection`: the two pieces point the same way, not only along the same axis.
/// (TALA also asks that they lie on parallel lines. Here both lie on one line.)
fn same_direction(a: Point, b: Point, c: Point, d: Point) -> bool {
    (b.x - a.x) * (d.x - c.x) + (b.y - a.y) * (d.y - c.y) > 0.0
}

/// TALA `containsPoint`.
fn contains(node: &Node, point: Point, delta: f64) -> bool {
    node.top_left.x - delta <= point.x && node.right() + delta >= point.x && node.top_left.y - delta <= point.y && node.bottom() + delta >= point.y
}

/// TALA `orthogonalSegmentEntersNode`: the piece runs through the inside of the card. Running
/// along its border does not count.
pub(crate) fn enters(node: &Node, a: Point, b: Point) -> bool {
    const EPSILON: f64 = 1e-6;
    let (left, right) = (node.top_left.x + EPSILON, node.right() - EPSILON);
    let (top, bottom) = (node.top_left.y + EPSILON, node.bottom() - EPSILON);
    if a.y == b.y {
        return top < a.y && a.y < bottom && a.x.min(b.x) < right && a.x.max(b.x) > left;
    }
    if a.x == b.x {
        return left < a.x && a.x < right && a.y.min(b.y) < bottom && a.y.max(b.y) > top;
    }
    true
}

/// TALA `simplifyPoints`: the route without its first hook that can go, or the route as it is.
fn simplify_points(graph: &Graph, routes: &[Vec<Point>], edge: usize) -> Vec<Point> {
    let points = &routes[edge];
    // A hook has five points. (A shorter route would come out as it is anyway.)
    if points.len() < 5 {
        return points.clone();
    }
    let mut result = vec![points[0]];
    let mut i = 0;
    while i < points.len() - 4 {
        let (p1, p2, p3, p4, p5) = (points[i], points[i + 1], points[i + 2], points[i + 3], points[i + 4]);
        let opposite = |a: f64, b: f64| (a > 0.0 && b < 0.0) || (a < 0.0 && b > 0.0);
        let same = |a: f64, b: f64| (a > 0.0 && b > 0.0) || (a < 0.0 && b < 0.0);
        // The second and the fourth piece run opposite ways, the first and the third the same
        // way, across them.
        let upright_hook = is_upright(p2, p3) && is_upright(p4, p5) && opposite(p3.y - p2.y, p5.y - p4.y) && is_level(p1, p2) && is_level(p3, p4) && same(p2.x - p1.x, p4.x - p3.x);
        let level_hook = is_level(p2, p3) && is_level(p4, p5) && opposite(p3.x - p2.x, p5.x - p4.x) && is_upright(p1, p2) && is_upright(p3, p4) && same(p2.y - p1.y, p4.y - p3.y);
        let mut free = None;
        if upright_hook || level_hook {
            free = corner(p1, p2, p4, p5).filter(|at| {
                // The route still reaches its second card the same way. (TALA asks the same of
                // the way out of the first card. The corner lies on the first piece's line, past
                // its end, so that way never turns round.)
                if i + 4 == points.len() - 1 && contains(&graph.nodes[graph.edges[edge].to], p5, 1.0) && !same_direction(p4, p5, *at, p5) {
                    return false;
                }
                // Neither new piece runs through a card or meets another route.
                [(p1, *at), (*at, p5)].iter().all(|(from, to)| {
                    !graph.nodes.iter().any(|node| node.width > 0.0 && node.height > 0.0 && enters(node, *from, *to))
                        && !routes.iter().enumerate().any(|(other, route)| other != edge && route.windows(2).any(|piece| segments_intersect(*from, *to, piece[0], piece[1])))
                })
            });
        }
        if let Some(at) = free {
            result.push(at);
            result.push(p5);
            i += 4;
            // One hook at a time.
            break;
        }
        result.push(p2);
        i += 1;
    }
    result.extend_from_slice(&points[i + 1..]);
    result
}

/// TALA `SimplifyEdgeRoutes`: every route in turn, until it has no hook left that can go.
pub fn simplify(graph: &Graph, routes: &mut [Vec<Point>]) {
    for edge in 0..routes.len() {
        loop {
            let points = simplify_points(graph, routes, edge);
            if points.len() >= routes[edge].len() {
                break;
            }
            routes[edge] = points;
        }
    }
}
