// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.

//! Routes by rule for two cards that lie diagonally: one bend (an L) or two (an S), tried before
//! the search. Ported from TALA `internal/routing`: `slingshot.go` (`slingshot`,
//! `findLShapedRoute`, `findSShapedRoute`, `launch`, `findLaunchings`, `findLandings`,
//! `undershot`, `overshot`, `isOnFlightPlan`, `preferLaunchingVertically`, `fillPathGuarded`),
//! `ovg_edge_set.go`, `ovg_edge_router.go` (`findOverlappingRoutes`) and `cluster_route_guard.go`
//! (`routeIntersectsNodeGuarded`). Not ported: tables, clusters, arrowhead labels.
//!
//! TALA's words: the route "launches" from a port of the first card, turns at an "anchor" and
//! "lands" on a port of the second card. A vertical launch leaves the first card up or down.

use std::collections::HashMap;

use crate::graph::{Graph, Point};
use crate::routing::geometry::{Orientation, euclidean_distance, is_diagonal, segments_intersect};
use crate::routing::ovg::{Coord, PointId, Side};
use crate::routing::router::{RouteId, Router, ideal_turn_axes};

/// The pieces of the routes found so far, by row and by column. TALA `ovgEdgeSet`.
#[derive(Default)]
pub(crate) struct EdgeSet {
    horizontal: HashMap<Coord, Vec<(Point, Point)>>,
    vertical: HashMap<Coord, Vec<(Point, Point)>>,
}

impl EdgeSet {
    /// TALA keeps a piece only once; a second copy changes no answer, so this set does not check.
    pub(crate) fn add(&mut self, a: Point, b: Point) {
        if a.x == b.x {
            self.vertical.entry(Coord::of(a.x)).or_default().push((a, b));
        } else if a.y == b.y {
            self.horizontal.entry(Coord::of(a.y)).or_default().push((a, b));
        }
    }

    /// TALA `intersectsWithChecked`: a piece of a route runs along the stretch from `from` to `to`
    /// or crosses it, and does not just share one of its ends.
    fn intersects_with(&self, from: Point, to: Point) -> bool {
        let vertical = from.x == to.x;
        if !vertical && from.y != to.y {
            return false;
        }
        // Along the line the stretch lies on (`along`), and across it.
        let along = |p: Point| if vertical { p.y } else { p.x };
        let across = |p: Point| if vertical { p.x } else { p.y };
        let (min, max) = (along(from).min(along(to)), along(from).max(along(to)));
        let axis = across(from);
        let shares_point = |a: Point, b: Point| a == from || a == to || b == from || b == to;
        let (parallel, perpendicular) =
            if vertical { (&self.vertical, &self.horizontal) } else { (&self.horizontal, &self.vertical) };

        for &(a, b) in parallel.get(&Coord::of(axis)).map(Vec::as_slice).unwrap_or(&[]) {
            let (other_min, other_max) = (along(a).min(along(b)), along(a).max(along(b)));
            if other_min < max && min < other_max && !shares_point(a, b) {
                return true;
            }
        }
        for (line, pieces) in perpendicular {
            if line.value() < min || max < line.value() {
                continue;
            }
            for &(a, b) in pieces {
                // Here the pieces run across: their extent is along the other axis.
                let (other_min, other_max) = (across(a).min(across(b)), across(a).max(across(b)));
                if other_min <= axis && axis <= other_max && !shares_point(a, b) {
                    return true;
                }
            }
        }
        false
    }

    /// TALA `overlappingEdgesChecked`: the pieces on the same line that touch the stretch.
    fn overlapping(&self, from: Point, to: Point) -> Vec<(Point, Point)> {
        let candidates = if from.x == to.x {
            self.vertical.get(&Coord::of(from.x))
        } else if from.y == to.y {
            self.horizontal.get(&Coord::of(from.y))
        } else {
            None
        };
        candidates
            .map(Vec::as_slice)
            .unwrap_or(&[])
            .iter()
            .copied()
            .filter(|(a, b)| segments_intersect(from, to, *a, *b))
            .collect()
    }
}

/// Remembers an answer per step. TALA `RouteChecker`.
#[derive(Default)]
struct Remembered(HashMap<(PointId, PointId), bool>);

impl Remembered {
    fn check(&mut self, from: PointId, to: PointId, compute: impl FnOnce() -> bool) -> bool {
        *self.0.entry((from, to)).or_insert_with(compute)
    }
}

fn opposite(orientation: Orientation) -> Orientation {
    match orientation {
        Orientation::TopLeft => Orientation::BottomRight,
        Orientation::TopRight => Orientation::BottomLeft,
        Orientation::BottomLeft => Orientation::TopRight,
        Orientation::BottomRight => Orientation::TopLeft,
        other => other,
    }
}

/// The first card is above the second (`true`) or below it.
fn is_above(orientation: Orientation) -> bool {
    matches!(orientation, Orientation::TopLeft | Orientation::TopRight)
}

/// The first card is left of the second (`true`) or right of it.
fn is_left(orientation: Orientation) -> bool {
    matches!(orientation, Orientation::TopLeft | Orientation::BottomLeft)
}

/// TALA `findLaunchings`: the side of the first card a route leaves through.
fn launch_side(orientation: Orientation, vertical_launch: bool) -> Side {
    match (vertical_launch, is_above(orientation), is_left(orientation)) {
        (true, true, _) => Side::Bottom,
        (true, false, _) => Side::Top,
        (false, _, true) => Side::Right,
        (false, _, false) => Side::Left,
    }
}

/// TALA `findLandings`: the side of the second card a route with one bend arrives at.
fn landing_side(orientation: Orientation, vertical_launch: bool) -> Side {
    match (vertical_launch, is_above(orientation), is_left(orientation)) {
        (true, _, true) => Side::Left,
        (true, _, false) => Side::Right,
        (false, true, _) => Side::Top,
        (false, false, _) => Side::Bottom,
    }
}

/// TALA `isOnFlightPlan`: the step from `previous` to `next` goes straight on in the direction of
/// the launch.
fn is_on_flight_plan(previous: Point, next: Point, orientation: Orientation, vertical_launch: bool) -> bool {
    if vertical_launch {
        next.x == previous.x && if is_above(orientation) { next.y > previous.y } else { next.y < previous.y }
    } else {
        next.y == previous.y && if is_left(orientation) { next.x > previous.x } else { next.x < previous.x }
    }
}

impl Router<'_> {
    /// TALA `findOverlappingRoutes`: the routes with a piece on the stretch, each once.
    fn find_overlapping_routes(&self, from: Point, to: Point) -> Vec<RouteId> {
        let mut found: Vec<RouteId> = Vec::new();
        for (a, b) in self.edge_set.overlapping(from, to) {
            for &at_from in self.routes_at(a) {
                for &at_to in self.routes_at(b) {
                    if at_from == at_to && !found.contains(&at_to) {
                        found.push(at_to);
                    }
                }
            }
        }
        found
    }

    /// The step is taken by routes this connection may not run along: one going the other way,
    /// or one that shares neither its start nor its end.
    fn is_occupied(&mut self, edge_index: usize, from: PointId, to: PointId) -> bool {
        let (from, to) = (self.ovg.point(from), self.ovg.point(to));
        let overlapping = self.find_overlapping_routes(from, to);
        for &route in &overlapping {
            if self.has_opposing_segment(route, from, to) {
                return true;
            }
        }
        !overlapping.is_empty() && !self.can_overlap(edge_index, &overlapping)
    }

    /// TALA `routeIntersectsNodeGuarded` for one segment: it passes through a card other than the
    /// two the connection joins.
    fn hits_a_card(&self, edge_index: usize, from: Point, to: Point) -> bool {
        let edge = self.graph.edges[edge_index];
        self.graph
            .nodes
            .iter()
            .enumerate()
            .any(|(card, node)| card != edge.from && card != edge.to && node.passes_through(from, to))
    }

    /// TALA `fillPathGuarded`: the points from `from` straight on to `to`, without `to`. `None`
    /// when the graph has no straight way.
    fn fill_path(&self, from: PointId, to: PointId) -> Option<Vec<PointId>> {
        let ovg = self.ovg;
        let (start, end) = (ovg.point(from), ovg.point(to));
        let mut out = vec![from];
        let mut current = from;
        loop {
            let here = ovg.point(current);
            let next = ovg.points[current].edges.iter().map(|edge| ovg.adjacent(current, *edge)).find(|adjacent| {
                let there = ovg.point(*adjacent);
                if start.x == end.x && there.x == here.x {
                    (start.y < end.y && here.y < there.y) || (start.y > end.y && here.y > there.y)
                } else if start.y == end.y && there.y == here.y {
                    (start.x < end.x && here.x < there.x) || (start.x > end.x && here.x > there.x)
                } else {
                    false
                }
            })?;
            if next == to {
                return Some(out);
            }
            out.push(next);
            current = next;
        }
    }

    /// TALA `slingshot`: a route with one or two bends for two cards on a diagonal, and its
    /// price. `None` when the cards are not on a diagonal or no such route is free.
    pub fn slingshot(&mut self, edge_index: usize) -> Option<(Vec<PointId>, f64)> {
        let graph = self.graph;
        let edge = graph.edges[edge_index];
        // No rule route for a card of a cluster.
        if graph.cluster_of(edge.from).is_some() || graph.cluster_of(edge.to).is_some() {
            return None;
        }
        let orientation = graph.nodes[edge.from].orientation(&graph.nodes[edge.to]);
        if !is_diagonal(orientation) {
            return None;
        }
        let mut occupied = Remembered::default();
        let mut crossed = Remembered::default();
        let (vertical_preferred, strong) = prefer_launching_vertically(graph, edge.from, edge.to, orientation);
        let order = if vertical_preferred { [true, false] } else { [false, true] };
        // What a launch against a strong preference costs on top.
        let against = |vertical_launch: bool| {
            if strong && vertical_preferred != vertical_launch { self.costs.crossing / 2.0 } else { 0.0 }
        };
        let against = [against(order[0]), against(order[1])];

        let (path, flight) = self.find_l_shaped_route(edge_index, orientation, order, against, &mut occupied, &mut crossed);
        if flight != f64::INFINITY {
            return Some((path, flight + self.costs.turn));
        }
        let (path, flight) = self.find_s_shaped_route(edge_index, orientation, order, against, &mut occupied, &mut crossed);
        if path.len() <= 1 {
            return None;
        }
        Some((path, flight))
    }

    /// The next point straight on in the direction of the launch.
    fn next_on_flight_plan(&self, current: PointId, orientation: Orientation, vertical_launch: bool) -> Option<PointId> {
        let ovg = self.ovg;
        ovg.points[current]
            .edges
            .iter()
            .map(|edge| ovg.adjacent(current, *edge))
            .find(|adjacent| is_on_flight_plan(ovg.point(current), ovg.point(*adjacent), orientation, vertical_launch))
    }

    /// The ports of a card on one side, each once. TALA `findLaunchings`, `findLandings`.
    fn ports_on(&self, card: usize, side: Side) -> Vec<PointId> {
        let mut found: Vec<PointId> = Vec::new();
        for &port in &self.ovg.ports[card] {
            if !found.contains(&port) && self.ovg.points[port].role_for(card).is_some_and(|role| role.sides.has(side)) {
                found.push(port);
            }
        }
        found
    }

    /// TALA `findLShapedRoute`: straight out of the first card until in line with a port of the
    /// second, one bend, straight in.
    fn find_l_shaped_route(
        &mut self,
        edge_index: usize,
        orientation: Orientation,
        order: [bool; 2],
        against: [f64; 2],
        occupied: &mut Remembered,
        crossed: &mut Remembered,
    ) -> (Vec<PointId>, f64) {
        let (graph, ovg) = (self.graph, self.ovg);
        let edge = graph.edges[edge_index];
        let target_card = &graph.nodes[edge.to];
        let (source, target) = (ovg.centers[edge.from], ovg.centers[edge.to]);
        let mut shortest_path: Vec<PointId> = Vec::new();
        let mut shortest_flight = f64::INFINITY;

        for (vertical_launch, against) in order.into_iter().zip(against) {
            let source_ports = self.ports_on(edge.from, launch_side(orientation, vertical_launch));
            let target_ports = self.ports_on(edge.to, landing_side(orientation, vertical_launch));
            for &s_port in &source_ports {
                let mut current = s_port;
                while let Some(next) = self.next_on_flight_plan(current, orientation, vertical_launch) {
                    let anchor = ovg.point(next);
                    if overshot(anchor, target_card, orientation, vertical_launch, true) {
                        break;
                    }
                    if occupied.check(current, next, || self.is_occupied(edge_index, current, next)) {
                        break;
                    }
                    current = next;
                    if undershot(anchor, target_card, orientation, vertical_launch) {
                        continue;
                    }
                    for &t_port in &target_ports {
                        let landing = ovg.point(t_port);
                        if (vertical_launch && anchor.y != landing.y) || (!vertical_launch && anchor.x != landing.x) {
                            continue;
                        }
                        // TALA `launch`: both legs must be clear of other cards.
                        let launching = ovg.point(s_port);
                        if self.hits_a_card(edge_index, launching, anchor) || self.hits_a_card(edge_index, landing, anchor) {
                            continue;
                        }
                        let mut d = euclidean_distance(launching.x, launching.y, anchor.x, anchor.y)
                            + euclidean_distance(landing.x, landing.y, anchor.x, anchor.y);
                        if d == 0.0 {
                            continue;
                        }
                        if crossed.check(s_port, next, || self.edge_set.intersects_with(launching, anchor)) {
                            d += self.costs.crossing;
                        }
                        if crossed.check(next, t_port, || self.edge_set.intersects_with(anchor, landing)) {
                            d += self.costs.crossing;
                        }
                        d += against;
                        if d >= shortest_flight {
                            continue;
                        }
                        let mut path = vec![source];
                        let Some(first_leg) = self.fill_path(s_port, next) else { continue };
                        path.extend(first_leg);
                        let Some(mut second_leg) = self.fill_path(next, t_port) else { continue };
                        second_leg.push(t_port);
                        // The first leg was checked on the way out. TALA leaves out the last step
                        // of the second leg here.
                        let bad_share = (0..second_leg.len().saturating_sub(2)).any(|i| {
                            let (a, b) = (second_leg[i], second_leg[i + 1]);
                            occupied.check(a, b, || self.is_occupied(edge_index, a, b))
                        });
                        if bad_share {
                            continue;
                        }
                        path.extend(second_leg);
                        path.push(target);
                        // Running along another route is fine, being the very same route is not.
                        let duplicate = self.routes.iter().any(|route| {
                            route.from_port == launching
                                && route.to_port == landing
                                && route.nodes.len() == path.len()
                                && (2..path.len().saturating_sub(2)).all(|i| route.points[i] == ovg.point(path[i]))
                        });
                        if duplicate {
                            continue;
                        }
                        shortest_flight = d;
                        shortest_path = path;
                    }
                }
            }
        }
        (shortest_path, shortest_flight)
    }

    /// TALA `findSShapedRoute`: straight out of the first card, across on a line both cards can
    /// reach, straight into the second card. The price includes the two turns.
    fn find_s_shaped_route(
        &mut self,
        edge_index: usize,
        orientation: Orientation,
        order: [bool; 2],
        against: [f64; 2],
        occupied: &mut Remembered,
        crossed: &mut Remembered,
    ) -> (Vec<PointId>, f64) {
        let (graph, ovg) = (self.graph, self.ovg);
        let edge = graph.edges[edge_index];
        let (source_card, target_card) = (&graph.nodes[edge.from], &graph.nodes[edge.to]);
        let (source, target) = (ovg.centers[edge.from], ovg.centers[edge.to]);
        let turn_axes = ideal_turn_axes(source_card, target_card);
        let mut shortest_path: Vec<PointId> = Vec::new();
        let mut shortest_flight = f64::INFINITY;

        for (vertical_launch, against) in order.into_iter().zip(against) {
            let source_ports = self.ports_on(edge.from, launch_side(orientation, vertical_launch));
            // The second card is entered on the side that faces the first card's launch.
            let target_ports = self.ports_on(edge.to, landing_side(orientation, !vertical_launch));

            // Every point a straight run out of a port reaches, with the port it came from.
            let mut source_anchors: Vec<(PointId, PointId)> = Vec::new();
            for &s_port in &source_ports {
                let mut current = s_port;
                while let Some(next) = self.next_on_flight_plan(current, orientation, vertical_launch) {
                    if overshot(ovg.point(next), target_card, orientation, vertical_launch, false) {
                        break;
                    }
                    if occupied.check(current, next, || self.is_occupied(edge_index, current, next)) {
                        break;
                    }
                    source_anchors.push((next, s_port));
                    current = next;
                }
            }
            let mut target_anchors: Vec<(PointId, PointId)> = Vec::new();
            for &t_port in &target_ports {
                let mut current = t_port;
                while let Some(next) = self.next_on_flight_plan(current, opposite(orientation), vertical_launch) {
                    if overshot(ovg.point(next), source_card, opposite(orientation), vertical_launch, false) {
                        break;
                    }
                    if occupied.check(current, next, || self.is_occupied(edge_index, current, next)) {
                        break;
                    }
                    target_anchors.push((next, t_port));
                    current = next;
                }
            }
            // Runs out of different ports are on different lines, so a point has one port.
            let port_of = |anchors: &[(PointId, PointId)], anchor: PointId| {
                anchors.iter().find(|(point, _)| *point == anchor).map(|(_, port)| *port).expect("an anchor")
            };

            // Anchors closer to the line halfway between the cards first.
            let axis_of = |id: PointId| if vertical_launch { ovg.point(id).y } else { ovg.point(id).x };
            let halfway = if vertical_launch { turn_axes[1].value } else { turn_axes[0].value };
            let mut source_order: Vec<PointId> = source_anchors.iter().map(|(anchor, _)| *anchor).collect();
            source_order.sort_by(|a, b| (axis_of(*a) - halfway).abs().total_cmp(&(axis_of(*b) - halfway).abs()));

            for &s_anchor in &source_order {
                let s_port = port_of(&source_anchors, s_anchor);
                for &(t_anchor, _) in target_anchors.iter().filter(|(anchor, _)| axis_of(*anchor) == axis_of(s_anchor)) {
                    let t_port = port_of(&target_anchors, t_anchor);
                    let mut d = 0.0;
                    let mut hits = false;
                    for (a, b) in [(s_port, s_anchor), (s_anchor, t_anchor), (t_anchor, t_port)] {
                        let (from, to) = (ovg.point(a), ovg.point(b));
                        if self.hits_a_card(edge_index, from, to) {
                            hits = true;
                            break;
                        }
                        d += euclidean_distance(from.x, from.y, to.x, to.y);
                        if crossed.check(a, b, || self.edge_set.intersects_with(from, to)) {
                            d += self.costs.crossing;
                        }
                    }
                    if hits {
                        continue;
                    }
                    d += against;
                    if d >= shortest_flight {
                        continue;
                    }
                    let mut path = vec![source];
                    let Some(first_leg) = self.fill_path(s_port, s_anchor) else { continue };
                    path.extend(first_leg);
                    let Some(mut middle) = self.fill_path(s_anchor, t_anchor) else { continue };
                    // The two outer legs were checked while collecting the anchors. TALA leaves
                    // out the last step of the middle leg here.
                    middle.push(t_anchor);
                    let bad_share = (0..middle.len().saturating_sub(2)).any(|i| {
                        let (a, b) = (middle[i], middle[i + 1]);
                        occupied.check(a, b, || self.is_occupied(edge_index, a, b))
                    });
                    if bad_share {
                        continue;
                    }
                    middle.pop();
                    path.extend(middle);
                    let Some(last_leg) = self.fill_path(t_anchor, t_port) else { continue };
                    path.extend(last_leg);
                    path.push(t_port);
                    path.push(target);
                    shortest_flight = d;
                    shortest_path = path;
                }
            }
        }
        if shortest_flight != f64::INFINITY {
            shortest_flight += self.costs.turn * 2.0;
        }
        (shortest_path, shortest_flight)
    }
}

/// TALA `undershot`: the point is not yet level with the second card.
fn undershot(point: Point, target: &crate::graph::Node, orientation: Orientation, vertical_launch: bool) -> bool {
    match (vertical_launch, is_above(orientation), is_left(orientation)) {
        (true, true, _) => point.y < target.top_left.y,
        (true, false, _) => point.y > target.bottom(),
        (false, _, true) => point.x < target.top_left.x,
        (false, _, false) => point.x > target.right(),
    }
}

/// TALA `overshot`: the point is past the second card. A route with one bend may go on until it
/// is past the whole card, one with two bends only until it reaches the card.
fn overshot(point: Point, target: &crate::graph::Node, orientation: Orientation, vertical_launch: bool, one_bend: bool) -> bool {
    match (vertical_launch, is_above(orientation), is_left(orientation)) {
        (true, true, _) => point.y > if one_bend { target.bottom() } else { target.top_left.y },
        (true, false, _) => point.y < if one_bend { target.top_left.y } else { target.bottom() },
        (false, _, true) => point.x > if one_bend { target.right() } else { target.top_left.x },
        (false, _, false) => point.x < if one_bend { target.top_left.x } else { target.right() },
    }
}

/// TALA `preferLaunchingVertically`: of the two L shapes, the one that takes less of the room the
/// two cards' other connections need. `(vertical, strong)`: a tie is broken by the numbers the
/// cards have in the diagram (`Graph::id`) and is not a strong preference.
fn prefer_launching_vertically(graph: &Graph, source: usize, target: usize, orientation: Orientation) -> (bool, bool) {
    use Orientation::*;
    let (vertical_source, vertical_target, horizontal_source, horizontal_target) = match orientation {
        TopLeft => ([BottomLeft, Bottom], [Left, BottomLeft], [TopRight, Right], [Top, TopRight]),
        TopRight => ([BottomRight, Bottom], [Right, BottomRight], [TopLeft, Left], [Top, TopLeft]),
        BottomLeft => ([TopLeft, Top], [Left, TopLeft], [BottomRight, Right], [Bottom, BottomRight]),
        _ => ([TopRight, Top], [Right, TopRight], [BottomLeft, Left], [Bottom, BottomLeft]),
    };
    let (mut in_vertical, mut in_horizontal) = (0, 0);
    // Where the cards a card is connected to lie, seen from it. TALA skips the other card of this
    // connection; it lies on the diagonal, which none of the lists holds, so it never counts.
    let neighbours = |card: usize| {
        graph.edges.iter().filter_map(move |edge| {
            let neighbour = if edge.from == card {
                edge.to
            } else if edge.to == card {
                edge.from
            } else {
                return Option::None;
            };
            Some(graph.nodes[neighbour].orientation(&graph.nodes[card]))
        })
    };
    for o in neighbours(source) {
        if vertical_source.contains(&o) {
            in_vertical += 1;
        }
        if horizontal_source.contains(&o) {
            in_horizontal += 1;
        }
    }
    for o in neighbours(target) {
        if vertical_target.contains(&o) {
            in_vertical += 1;
        } else if horizontal_target.contains(&o) {
            in_horizontal += 1;
        }
    }
    if in_vertical == in_horizontal {
        // By the cards' numbers in the diagram, not by their places in the graph's list.
        return (graph.id(source) < graph.id(target), false);
    }
    (in_vertical < in_horizontal, true)
}
