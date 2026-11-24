// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.

//! The search for one connection through the visibility graph, with the routes found before it
//! in the way. Ported from TALA `internal/routing`: `ovg_edge_router.go` (`search`, `usedPorts`,
//! `centerSymmetricalPorts`, `routesBlockTraversal`, `bestRoute`, `addRoute`),
//! `ovg_crossing_index.go`, `priority_queue.go`, `turns.go`, `route_directions.go`, `route.go`
//! (`isEntireColinear`), `coordinator.go` (`directedEdgesCanOverlap`) and the three costs in
//! `internal/layoutgraph/graph.go`.
//!
//! `quickRoute` is here too: cards close together get a straight line (`line.rs`) without a search.
//!
//! For connections as the editor has them: an arrow at the target, no label of their own, between
//! plain cards. Two connected cards may touch, as TALA's own placement leaves them now and then:
//! a port both have takes no route, and a route may leave and reach a port across its side
//! either way. Cards that lie on each other are not checked against TALA. Not ported: undirected
//! and two-way connections, arrowhead labels, labels of cards, clusters, containers, tables.

use std::collections::{BTreeSet, HashMap, HashSet};

use crate::graph::{Arrangement, Graph, Node, Point};
use crate::routing::geometry::{Orientation, euclidean_distance, is_diagonal};
use crate::routing::line::{Routed, port_indices, ports, route_line};
use crate::routing::ovg::{Coord, EdgeId, Ovg, PointId, Side, key};
use crate::routing::slingshot::EdgeSet;
use crate::routing::swap_ports::intersection;
use crate::routing::tuning::{
    BASICALLY_INFINITY, CENTER_PORT_MULTIPLIER, CROSSING_COST_WEIGHT, IDEAL_TURN_AXIS_TOLERANCE,
    IDEAL_TURN_EVEN_CLUSTER_MULTIPLIER, IDEAL_TURN_MULTIPLIER, MIN_ARROWHEAD_CLEARANCE, MIN_ROUTE_NODE_CLEARANCE, NODE_PROXIMITY_PENALTY,
    PATH_NODE_PROXIMITY_FLOOR, PRECISION, TURN_ENDPOINT_CLEARANCE, TURN_PENALTY_MULTIPLIER,
};
use crate::policy::CONNECTED_NODE_GAP;

/// The three prices the search works with. TALA `Graph.TurnCost`, `CrossingCost`,
/// `NonCenterPortCost`: all grow with the number of connections and the longest one.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Costs {
    pub turn: f64,
    pub crossing: f64,
    pub non_center_port: f64,
}

impl Costs {
    pub fn of(graph: &Graph) -> Self {
        let edges = graph.edges.len() as f64;
        let longest = max_edge_length(graph);
        let mut non_center_port = 0.0;
        if !graph.edges.is_empty() {
            non_center_port = CENTER_PORT_MULTIPLIER * edges * longest;
            // The price must stay above the distance between two ports of a small card.
            let smallest = graph.nodes[graph.nearby..]
                .iter()
                .fold(f64::INFINITY, |least, node| least.min(node.height).min(node.width));
            if smallest.is_finite() {
                non_center_port = non_center_port.max(smallest / 3.0);
            }
        }
        Costs {
            turn: TURN_PENALTY_MULTIPLIER * edges * longest,
            crossing: CROSSING_COST_WEIGHT * edges * longest,
            non_center_port,
        }
    }
}

/// TALA `maxEdgeLength`: the widest gap a connection spans, at least the gap connected cards keep.
fn max_edge_length(graph: &Graph) -> f64 {
    if graph.edges.is_empty() {
        return 0.0;
    }
    graph.edges.iter().fold(CONNECTED_NODE_GAP, |longest, edge| {
        longest.max(graph.nodes[edge.from].distance_to(&graph.nodes[edge.to]))
    })
}

/// A route: the points it passes, from the centre of its first card to the centre of its second.
/// TALA `Route`.
#[derive(Clone, Debug)]
pub struct Route {
    /// Index of the connection in `Graph::edges`.
    pub edge: usize,
    /// The points of the graph it passes. The two ports of a straight line are not points of the
    /// graph: they are `DETACHED`.
    pub nodes: Vec<PointId>,
    /// Where those points are.
    pub points: Vec<Point>,
    pub from_port: Point,
    pub to_port: Point,
}

/// Stands for the two ports of a straight line (TALA makes fresh points for them, which are no
/// point of the graph and no port in its lists).
pub const DETACHED: [PointId; 2] = [usize::MAX, usize::MAX - 1];

impl Route {
    /// A route through points of the graph.
    pub fn through(ovg: &Ovg, edge: usize, nodes: Vec<PointId>) -> Self {
        let points: Vec<Point> = nodes.iter().map(|id| ovg.point(*id)).collect();
        Route { edge, from_port: points[1], to_port: points[points.len() - 2], nodes, points }
    }

    /// A straight line between a port of each card. TALA `quickRoute`.
    pub fn line(ovg: &Ovg, graph: &Graph, edge: usize, from_port: Point, to_port: Point) -> Self {
        let (source, target) = (ovg.centers[graph.edges[edge].from], ovg.centers[graph.edges[edge].to]);
        Route {
            edge,
            nodes: vec![source, DETACHED[0], DETACHED[1], target],
            points: vec![ovg.point(source), from_port, to_port, ovg.point(target)],
            from_port,
            to_port,
        }
    }

    /// TALA `createSegmentEndpoints`: the two ports and the bends.
    pub fn segment_endpoints(&self) -> Vec<Point> {
        let points = &self.points;
        let count = points.len();
        let mut ends = vec![points[1]];
        for i in 2..count.saturating_sub(2) {
            let (last, current, next) = (points[i - 1], points[i], points[i + 1]);
            if (current.x == last.x && current.x != next.x) || (current.y == last.y && current.y != next.y) {
                ends.push(current);
            }
        }
        ends.push(points[count - 2]);
        ends
    }
}

pub type RouteId = usize;

#[derive(Clone, Debug, PartialEq)]
pub enum RouteError {
    /// The search ran out of points before it reached the other card.
    NoPath,
    /// A point's price was lowered after the search had already left it.
    QueueOutOfStep,
    /// A route passes the same point twice.
    RepeatedPoint,
}

struct GridInterval {
    edge: EdgeId,
    start: f64,
    end: f64,
    max_end: f64,
}

struct GridLine {
    coordinate: f64,
    intervals: Vec<GridInterval>,
}

/// The edges of the graph by row and by column, to find the ones a new route crosses or runs
/// along. TALA `ovgCrossingIndex`.
struct CrossingIndex {
    horizontal: Vec<GridLine>,
    vertical: Vec<GridLine>,
    crossed: HashSet<EdgeId>,
}

impl CrossingIndex {
    fn new(ovg: &Ovg) -> Self {
        let index = |lines: &std::collections::BTreeMap<Coord, Vec<EdgeId>>, vertical: bool| {
            lines
                .iter()
                .map(|(coordinate, edges)| {
                    let mut intervals: Vec<GridInterval> = edges
                        .iter()
                        .map(|&edge| {
                            let (from, to) = (ovg.point(ovg.edges[edge].from), ovg.point(ovg.edges[edge].to));
                            let (a, b) = if vertical { (from.y, to.y) } else { (from.x, to.x) };
                            GridInterval { edge, start: a.min(b), end: a.max(b), max_end: 0.0 }
                        })
                        .collect();
                    intervals.sort_by(|a, b| a.start.total_cmp(&b.start));
                    let mut max_end = f64::NEG_INFINITY;
                    for interval in &mut intervals {
                        max_end = max_end.max(interval.end);
                        interval.max_end = max_end;
                    }
                    GridLine { coordinate: coordinate.value(), intervals }
                })
                .collect::<Vec<GridLine>>()
        };
        CrossingIndex {
            horizontal: index(&ovg.horizontal_edges, false),
            vertical: index(&ovg.vertical_edges, true),
            crossed: HashSet::new(),
        }
    }

    /// TALA `gridLines.visit`: every edge on the lines from `first` to `last` that reaches into
    /// the stretch from `start` to `end`, borders included.
    fn visit(lines: &[GridLine], first: f64, last: f64, start: f64, end: f64, mut found: impl FnMut(f64, &GridInterval)) {
        let from = lines.partition_point(|line| line.coordinate < first);
        for line in &lines[from..] {
            if line.coordinate > last {
                break;
            }
            let from = line.intervals.partition_point(|interval| interval.max_end < start);
            for interval in &line.intervals[from..] {
                if interval.start > end {
                    break;
                }
                if interval.end >= start {
                    found(line.coordinate, interval);
                }
            }
        }
    }

    /// TALA `ovgCrossingIndex.add`: marks the edges a new piece of a route crosses or lies on.
    fn add(&mut self, ovg: &Ovg, from: Point, to: Point) {
        let vertical = from.x == to.x;
        if !vertical && from.y != to.y {
            return;
        }
        let (axis, a, b) = if vertical { (from.x, from.y, to.y) } else { (from.y, from.x, to.x) };
        let (parallel, perpendicular) =
            if vertical { (&self.vertical, &self.horizontal) } else { (&self.horizontal, &self.vertical) };
        let (start, end) = (a.min(b), a.max(b));
        let shares_point = |edge: EdgeId| {
            let (edge_from, edge_to) = (ovg.point(ovg.edges[edge].from), ovg.point(ovg.edges[edge].to));
            edge_from == from || edge_from == to || edge_to == from || edge_to == to
        };
        let mut crossed = Vec::new();
        CrossingIndex::visit(parallel, axis, axis, start, end, |_, interval| {
            // TALA also asks that the two overlap by more than a point. Edges that only touch
            // the piece share that point with it, which is asked here anyway.
            if !shares_point(interval.edge) {
                crossed.push(interval.edge);
            }
        });
        CrossingIndex::visit(perpendicular, start, end, axis, axis, |_, interval| {
            if !shares_point(interval.edge) {
                crossed.push(interval.edge);
            }
        });
        self.crossed.extend(crossed);
    }
}

/// One waiting state of the search: a point, reached along its row or along its column.
#[derive(Clone, Copy)]
struct QueueEntry {
    node: PointId,
    is_horizontal: bool,
    priority: f64,
    order: u64,
    waiting: bool,
}

/// TALA `priorityQueue`: the cheapest entry first, the older one among equals. Any queue with
/// that order hands out the same sequence, so this one is a sorted set instead of TALA's heap.
#[derive(Default)]
struct Queue {
    entries: Vec<QueueEntry>,
    waiting: BTreeSet<(Coord, u64, usize)>,
}

impl Queue {
    fn push(&mut self, priority: f64, node: PointId, is_horizontal: bool) -> usize {
        let id = self.entries.len();
        let order = id as u64;
        self.entries.push(QueueEntry { node, is_horizontal, priority, order, waiting: true });
        self.waiting.insert((Coord::of(priority), order, id));
        id
    }

    fn pop(&mut self) -> Option<QueueEntry> {
        let (_, _, id) = self.waiting.pop_first()?;
        self.entries[id].waiting = false;
        Some(self.entries[id])
    }

    /// TALA `decrease`: fails for an entry that was already handed out.
    fn decrease(&mut self, id: usize, priority: f64) -> Result<(), RouteError> {
        let entry = self.entries[id];
        if !entry.waiting || priority >= entry.priority {
            return Err(RouteError::QueueOutOfStep);
        }
        self.waiting.remove(&(Coord::of(entry.priority), entry.order, id));
        self.waiting.insert((Coord::of(priority), entry.order, id));
        self.entries[id].priority = priority;
        Ok(())
    }
}

/// What the search knows about a point it has reached. TALA `searchNodeContext`.
#[derive(Clone, Copy)]
struct Reached {
    vertical_distance: f64,
    horizontal_distance: f64,
    vertical_entry: Option<usize>,
    horizontal_entry: Option<usize>,
}

impl Reached {
    fn new() -> Self {
        Reached {
            vertical_distance: f64::INFINITY,
            horizontal_distance: f64::INFINITY,
            vertical_entry: None,
            horizontal_entry: None,
        }
    }
}

/// A line where a route between two cards can turn without a detour. TALA `idealTurnAxis`.
pub(crate) struct TurnAxis {
    pub(crate) is_x: bool,
    pub(crate) value: f64,
}

/// TALA `idealTurnAxes`: the lines halfway between two cards.
pub(crate) fn ideal_turn_axes(a: &Node, b: &Node) -> Vec<TurnAxis> {
    let (x1, y1, x1b, y1b) = (a.top_left.x, a.top_left.y, a.right(), a.bottom());
    let (x2, y2, x2b, y2b) = (b.top_left.x, b.top_left.y, b.right(), b.bottom());
    let (left, right, top, bottom) = (x2b < x1, x1b < x2, y2b < y1, y1b < y2);
    let x = |value: f64| TurnAxis { is_x: true, value };
    let y = |value: f64| TurnAxis { is_x: false, value };
    if top && left {
        vec![x((x1 + x2b) / 2.0), y((y1 + y2b) / 2.0)]
    } else if left && bottom {
        vec![x((x1 + x2b) / 2.0), y((y2 + y1b) / 2.0)]
    } else if bottom && right {
        vec![x((x2 + x1b) / 2.0), y((y2 + y1b) / 2.0)]
    } else if right && top {
        vec![x((x2 + x1b) / 2.0), y((y1 + y2b) / 2.0)]
    } else if left {
        vec![x((x1 + x2b) / 2.0)]
    } else if right {
        vec![x((x2 + x1b) / 2.0)]
    } else if bottom {
        vec![y((y2 + y1b) / 2.0)]
    } else if top {
        vec![y((y1 + y2b) / 2.0)]
    } else {
        Vec::new()
    }
}

/// A turn at an ideal line is a little cheaper; much cheaper on a route from a card of a
/// cluster with an even number of cards, in the arrangement the cards around it asked for.
fn turn_multiplier(axes: &[TurnAxis], at: Point, even_cluster: bool) -> f64 {
    for axis in axes {
        let coordinate = if axis.is_x { at.x } else { at.y };
        if (coordinate - axis.value).abs() <= IDEAL_TURN_AXIS_TOLERANCE {
            return if even_cluster { IDEAL_TURN_EVEN_CLUSTER_MULTIPLIER } else { IDEAL_TURN_MULTIPLIER };
        }
    }
    1.0
}

/// Whether a card is in a cluster with an even number of cards, in the arrangement the cards
/// around it asked for.
fn in_even_cluster(graph: &Graph, card: usize) -> bool {
    graph.cluster_of(card).is_some_and(|cluster| {
        let cluster = &graph.clusters[cluster];
        cluster.arrangement == cluster.desired && cluster.members.len() % 2 == 0
    })
}

/// The other cards of the cluster a card is in (TALA `sourceAndTargetClusterNodesGuarded`).
pub(crate) fn cluster_others(graph: &Graph, card: usize) -> Vec<usize> {
    graph.cluster_of(card).map(|cluster| graph.clusters[cluster].members.iter().copied().filter(|member| *member != card).collect()).unwrap_or_default()
}

/// TALA `clusterHasDesirableArrangementTo`: the cards of a cluster stand side by side where
/// the card lies above or below them, under each other where it lies beside them. The box of
/// the cluster reaches from its first card to its last.
fn has_desirable_arrangement_to(graph: &Graph, cluster: usize, card: usize) -> bool {
    let members = &graph.clusters[cluster].members;
    let (first, last) = (&graph.nodes[members[0]], &graph.nodes[members[members.len() - 1]]);
    let bounds = Node { top_left: first.top_left, width: last.right() - first.top_left.x, height: last.bottom() - first.top_left.y };
    match bounds.orientation(&graph.nodes[card]) {
        Orientation::Top | Orientation::Bottom => graph.clusters[cluster].arrangement == Arrangement::Row,
        Orientation::Left | Orientation::Right => graph.clusters[cluster].arrangement == Arrangement::Column,
        _ => false,
    }
}

/// TALA `isUndesirableClusterArrangement`: a card of a cluster at one end whose cluster is
/// not arranged towards the other end. Its cluster is then no cluster for the search.
pub(crate) fn is_undesirable_cluster_arrangement(graph: &Graph, source: usize, target: usize) -> bool {
    graph.cluster_of(source).is_some_and(|cluster| !has_desirable_arrangement_to(graph, cluster, target))
        || graph.cluster_of(target).is_some_and(|cluster| !has_desirable_arrangement_to(graph, cluster, source))
}

/// TALA `geo.PrecisionCompare(a, b) > 0`.
#[allow(clippy::neg_cmp_op_on_partial_ord)]
fn clearly_more(a: f64, b: f64) -> bool {
    // Written as TALA does, so that two infinite prices compare the same way.
    !((a - b).abs() < PRECISION) && !(a < b)
}

/// TALA `OVGNode.distanceToBoundary`.
fn distance_to_boundary(point: Point, node: &Node) -> f64 {
    let (x1, y1) = (point.x, point.y);
    let (x2, y2, x2b, y2b) = (node.top_left.x, node.top_left.y, node.right(), node.bottom());
    let (left, right, top, bottom) = (x2b < x1, x1 < x2, y2b < y1, y1 < y2);
    if top && left {
        euclidean_distance(x1, y1, x2b, y2b)
    } else if left && bottom {
        euclidean_distance(x1, y1, x2b, y2)
    } else if bottom && right {
        euclidean_distance(x1, y1, x2, y2)
    } else if right && top {
        euclidean_distance(x1, y1, x2, y2b)
    } else if left {
        x1 - x2b
    } else if right {
        x2 - x1
    } else if bottom {
        y2 - y1
    } else if top {
        y1 - y2b
    } else {
        0.0
    }
}

/// The pieces of a route by line, with their direction. TALA `routeDirectionIndex`.
type Directions = HashMap<(Coord, bool), Vec<(f64, f64)>>;

/// TALA `directedInterval.opposes`: `(stop, opposing)`.
fn opposes(segment: (f64, f64), from: f64, to: f64) -> (bool, bool) {
    let (lo, hi) = (segment.0.min(segment.1), segment.0.max(segment.1));
    if from.max(to) < lo || hi < from.min(to) {
        return (false, false);
    }
    if ((from == lo || from == hi) && (to < lo || to > hi))
        || ((to == lo || to == hi) && (from < lo || from > hi))
    {
        return (true, false);
    }
    let opposing = (segment.1 > segment.0 && to < from) || (segment.1 < segment.0 && to > from);
    (opposing, opposing)
}

/// Routes the connections of a graph one after another. TALA `ovgEdgeRouter`.
pub struct Router<'a> {
    pub(crate) graph: &'a Graph,
    pub(crate) ovg: &'a Ovg,
    pub costs: Costs,
    pub routes: Vec<Route>,
    point_to_route: HashMap<(Coord, Coord), Vec<RouteId>>,
    pub(crate) edge_set: EdgeSet,
    crossings: CrossingIndex,
    has_nearby_edge: HashSet<EdgeId>,
    overlapping_routes: HashMap<EdgeId, Vec<RouteId>>,
    route_directions: HashMap<RouteId, Directions>,
}

impl<'a> Router<'a> {
    pub fn new(graph: &'a Graph, ovg: &'a Ovg) -> Self {
        Router {
            graph,
            ovg,
            costs: Costs::of(graph),
            routes: Vec::new(),
            point_to_route: HashMap::new(),
            edge_set: EdgeSet::default(),
            crossings: CrossingIndex::new(ovg),
            has_nearby_edge: HashSet::new(),
            overlapping_routes: HashMap::new(),
            route_directions: HashMap::new(),
        }
    }

    pub(crate) fn routes_at(&self, point: Point) -> &[RouteId] {
        self.point_to_route.get(&key(point)).map(Vec::as_slice).unwrap_or(&[])
    }

    /// TALA `addRoute`: from now on the route is in the way of the next ones.
    pub fn add_route(&mut self, route: Route) -> Result<(), RouteError> {
        for (position, node) in route.nodes.iter().enumerate() {
            if route.nodes[..position].contains(node) {
                return Err(RouteError::RepeatedPoint);
            }
        }
        let id = self.routes.len();
        let ovg = self.ovg;
        let count = route.points.len();
        for (i, &start) in route.points.iter().enumerate() {
            self.point_to_route.entry(key(start)).or_default().push(id);
            // Only the pieces between the two ports: not the ones from a port to its card's centre.
            if i > 0 && i + 2 < count {
                self.edge_set.add(start, route.points[i + 1]);
                self.crossings.add(ovg, start, route.points[i + 1]);
            }
        }
        for i in 1..count.saturating_sub(2) {
            let (from, to) = (route.points[i], route.points[i + 1]);
            let (axis, a, b, lines) = if from.x == to.x {
                (from.x, from.y, to.y, &self.crossings.vertical)
            } else {
                (from.y, from.x, to.x, &self.crossings.horizontal)
            };
            let first = (axis - PATH_NODE_PROXIMITY_FLOOR).next_down();
            let last = (axis + PATH_NODE_PROXIMITY_FLOOR).next_up();
            let (has_nearby_edge, overlapping_routes) = (&mut self.has_nearby_edge, &mut self.overlapping_routes);
            CrossingIndex::visit(lines, first, last, a.min(b), a.max(b), |coordinate, interval| {
                if (coordinate - axis).abs() > PATH_NODE_PROXIMITY_FLOOR {
                    return;
                }
                has_nearby_edge.insert(interval.edge);
                if coordinate == axis {
                    overlapping_routes.entry(interval.edge).or_default().push(id);
                }
            });
        }
        self.routes.push(route);
        Ok(())
    }

    /// TALA `directedEdgesCanOverlap`: a connection may run along other routes when each of them
    /// starts at the same card as it or ends at the same card as it.
    pub(crate) fn can_overlap(&self, edge: usize, others: &[RouteId]) -> bool {
        let edge = &self.graph.edges[edge];
        others.iter().all(|route| {
            let other = &self.graph.edges[self.routes[*route].edge];
            edge.from == other.from || edge.to == other.to
        })
    }

    /// TALA `hasOpposingSegment`: the route runs along the step from `from` to `to` in the other
    /// direction.
    pub(crate) fn has_opposing_segment(&mut self, route: RouteId, from: Point, to: Point) -> bool {
        let points = &self.routes[route].points;
        let index = self.route_directions.entry(route).or_insert_with(|| {
            let mut index = Directions::new();
            // The piece that leaves the first card's centre counts, the one that enters the
            // second card's centre does not.
            for pair in points[..points.len() - 1].windows(2) {
                let (a, b) = (pair[0], pair[1]);
                if a.y == b.y && a.x != b.x {
                    index.entry((Coord::of(a.y), false)).or_default().push((a.x, b.x));
                } else if a.x == b.x && a.y != b.y {
                    index.entry((Coord::of(a.x), true)).or_default().push((a.y, b.y));
                }
            }
            index
        });
        let (line, a, b) = if from.y != to.y {
            if from.x != to.x {
                return false;
            }
            ((Coord::of(from.x), true), from.y, to.y)
        } else {
            ((Coord::of(from.y), false), from.x, to.x)
        };
        for segment in index.get(&line).map(Vec::as_slice).unwrap_or(&[]) {
            let (stop, opposing) = opposes(*segment, a, b);
            if stop {
                return opposing;
            }
        }
        false
    }

    /// TALA `Route.isEntireColinear`: all of the route between its ports lies on the step from
    /// `from` to `to`.
    fn is_entire_colinear(&self, route: RouteId, from: Point, to: Point) -> bool {
        let points = &self.routes[route].points;
        let pieces = || points[1..points.len() - 1].windows(2).map(|pair| (pair[0], pair[1]));
        if from.y == to.y {
            let (min, max) = (from.x.min(to.x), from.x.max(to.x));
            return pieces().all(|(a, b)| a.y == from.y && b.y == from.y && a.x.min(b.x) >= min && a.x.max(b.x) <= max);
        }
        if from.x == to.x {
            let (min, max) = (from.y.min(to.y), from.y.max(to.y));
            return pieces().all(|(a, b)| a.x == from.x && b.x == from.x && a.y.min(b.y) >= min && a.y.max(b.y) <= max);
        }
        false
    }

    /// TALA `routesBlockTraversal`, for a connection with a direction: a route going the other
    /// way blocks the step, and so does a route that lies whole on the step into a tunnel end.
    fn routes_block_traversal(&mut self, routes: &[RouteId], from: PointId, to: PointId) -> bool {
        let (from_point, to_point) = (self.ovg.point(from), self.ovg.point(to));
        let into_tunnel = self.ovg.points[to].is_tunnel;
        for &route in routes {
            if self.has_opposing_segment(route, from_point, to_point) {
                return true;
            }
            if into_tunnel && self.is_entire_colinear(route, from_point, to_point) {
                return true;
            }
        }
        false
    }

    /// TALA `routeLine`: a straight line for a connection the search found no way for (a card
    /// with no free line outwards). The line of [`route_line`] where one is possible, between
    /// the points of the graph at its two ports. Otherwise the line from middle to middle, cut
    /// where it leaves the first card and where it reaches the second, at the price of its
    /// whole length. With its price.
    pub fn line_instead(&self, edge_index: usize) -> (Route, f64) {
        let (graph, ovg) = (self.graph, self.ovg);
        let edge = graph.edges[edge_index];
        let (source, target) = (ovg.centers[edge.from], ovg.centers[edge.to]);
        let ends: Vec<Vec<Point>> = self.routes.iter().map(Route::segment_endpoints).collect();
        let others: Vec<Routed> =
            self.routes.iter().zip(&ends).map(|(route, points)| Routed { from: graph.edges[route.edge].from, to: graph.edges[route.edge].to, points }).collect();
        if let Some((from_port, to_port, price)) = route_line(graph, self.costs.non_center_port, edge.from, edge.to, &others) {
            let at = |port: Point, spare: PointId| ovg.occupied.get(&key(port)).copied().unwrap_or(spare);
            let route = Route {
                edge: edge_index,
                nodes: vec![source, at(from_port, DETACHED[0]), at(to_port, DETACHED[1]), target],
                points: vec![ovg.point(source), from_port, to_port, ovg.point(target)],
                from_port,
                to_port,
            };
            return (route, price);
        }
        // D2 `Box.Intersections`: where a line meets the sides of a card, asked in the order
        // top, right, bottom, left. The first answer counts.
        let meets = |card: &Node, from: Point, to: Point| {
            let (tl, br) = (card.top_left, Point { x: card.right(), y: card.bottom() });
            let (tr, bl) = (Point { x: br.x, y: tl.y }, Point { x: tl.x, y: br.y });
            [(tl, tr), (tr, br), (br, bl), (bl, tl)].into_iter().find_map(|(a, b)| intersection(from, to, a, b))
        };
        let (from_center, to_center) = (ovg.point(source), ovg.point(target));
        let leaves = meets(&graph.nodes[edge.from], from_center, to_center);
        let reaches = meets(&graph.nodes[edge.to], leaves.unwrap_or(from_center), to_center);
        let (from_port, to_port) = (leaves.unwrap_or(from_center), reaches.unwrap_or(to_center));
        (Route::line(ovg, graph, edge_index, from_port, to_port), euclidean_distance(from_center.x, from_center.y, to_center.x, to_center.y))
    }

    /// TALA `search`: the cheapest way from one card's centre to the other's. Roughly Dijkstra's
    /// shortest path over (point, arrived along its row or its column), where a step costs its
    /// length plus prices for turning, crossing, running along other routes and unloved ports.
    /// Returns the route and its price.
    pub fn search(&mut self, edge_index: usize) -> Result<(Route, f64), RouteError> {
        let (graph, ovg, costs) = (self.graph, self.ovg, self.costs);
        let edge = graph.edges[edge_index];
        let (g_source, g_target) = (edge.from, edge.to);
        let (source_card, target_card) = (&graph.nodes[g_source], &graph.nodes[g_target]);
        let (source, target) = (ovg.centers[g_source], ovg.centers[g_target]);

        // TALA `quickRoute`: two cards that face each other closer than 42 have no room for a
        // route with bends between them. They are joined by a straight line, if one is free.
        let distance = source_card.distance_to(target_card);
        // Two cards that overlap or touch: neither is on a side of the other. (TALA asks both
        // ways. Where the first is on no side of the second, the second is on none of the
        // first. TALA also keeps such a pair from the straight line below: their distance is
        // 0, which is too close for it anyway.)
        let overlap = source_card.orientation(target_card) == Orientation::None;
        if !is_diagonal(source_card.orientation(target_card))
            && distance < 2.0 * (MIN_ROUTE_NODE_CLEARANCE + 1.0)
            && distance > MIN_ARROWHEAD_CLEARANCE
        {
            let ends: Vec<Vec<Point>> = self.routes.iter().map(Route::segment_endpoints).collect();
            let others: Vec<Routed> = self
                .routes
                .iter()
                .zip(&ends)
                .map(|(route, points)| Routed { from: graph.edges[route.edge].from, to: graph.edges[route.edge].to, points })
                .collect();
            if let Some((from_port, to_port, price)) = route_line(graph, costs.non_center_port, g_source, g_target, &others) {
                return Ok((Route::line(ovg, graph, edge_index, from_port, to_port), price));
            }
        }

        // TALA `OverlappingPorts`: a port both cards have (they touch there) takes no route. Of
        // the twelve ports of a side's quarters: the end of a tunnel is none of them.
        let own_ports = |card: usize| ovg.ports[card].iter().take(12).map(|port| key(ovg.point(*port)));
        let blocked_ports: HashSet<(Coord, Coord)> = own_ports(g_source).filter(|port| own_ports(g_target).any(|other| other == *port)).collect();

        // A connection of a card of a cluster, where the cluster is arranged towards the other
        // card: it leaves and reaches the cards on the sides that face each other, and may
        // share ports with routes to the other cards of the cluster (also with a second
        // connection between the same two cards).
        let (source_cluster, target_cluster) = (cluster_others(graph, g_source), cluster_others(graph, g_target));
        let undesirable = is_undesirable_cluster_arrangement(graph, g_source, g_target);
        let prefer_facing_ports = (!source_cluster.is_empty() || !target_cluster.is_empty()) && !undesirable;
        let non_facing_ports_cost = costs.turn * 2.0;
        let allow_port_sharing = prefer_facing_ports;
        let facing = |card: &Node, other: &Node| -> HashSet<(Coord, Coord)> {
            let all = ports(card);
            port_indices(card.orientation(other).opposite()).iter().map(|index| key(all[*index])).collect()
        };
        let (from_facing, to_facing) = if prefer_facing_ports { (facing(source_card, target_card), facing(target_card, source_card)) } else { (HashSet::new(), HashSet::new()) };
        let even_cluster = in_even_cluster(graph, g_source);

        let turn_axes = ideal_turn_axes(source_card, target_card);

        // TALA `usedPorts`: the ports that routes of these two cards already use.
        let mut source_ports_used: HashSet<(Coord, Coord)> = HashSet::new();
        let mut target_ports_used: HashSet<(Coord, Coord)> = HashSet::new();
        let mut duplicate_source_ports: HashSet<(Coord, Coord)> = HashSet::new();
        let mut duplicate_target_ports: HashSet<(Coord, Coord)> = HashSet::new();
        // TALA `ClusterPortSharing`: the ports at the target that routes from the other cards
        // of the source's cluster use, and the ports at the source that routes to the other
        // cards of the target's cluster use.
        let mut shared_source_ports: HashSet<(Coord, Coord)> = HashSet::new();
        let mut shared_target_ports: HashSet<(Coord, Coord)> = HashSet::new();
        for center in [source, target] {
            for &route_id in self.routes_at(ovg.point(center)) {
                let route = &self.routes[route_id];
                let other = graph.edges[route.edge];
                if other.from != g_source && other.to != g_source && other.from != g_target && other.to != g_target {
                    continue;
                }
                if other.from == g_source && other.to == g_target {
                    duplicate_source_ports.insert(key(route.from_port));
                    duplicate_target_ports.insert(key(route.to_port));
                }
                if !undesirable {
                    if other.to == g_target && source_cluster.contains(&other.from) {
                        shared_target_ports.insert(key(route.to_port));
                    }
                    if other.from == g_source && target_cluster.contains(&other.to) {
                        shared_source_ports.insert(key(route.from_port));
                    }
                }
                // Only ports that are points of the graph count: not those of a straight line.
                for (node, point) in route.nodes.iter().zip(&route.points) {
                    if ovg.ports[g_source].contains(node) {
                        source_ports_used.insert(key(*point));
                    }
                    if ovg.ports[g_target].contains(node) {
                        target_ports_used.insert(key(*point));
                    }
                }
            }
        }
        // TALA `centerSymmetricalPorts`: a middle port whose opposite middle port is in use.
        let mut symmetrical_ports: HashSet<PointId> = HashSet::new();
        for (card, used) in [(g_source, &source_ports_used), (g_target, &target_ports_used)] {
            let ports = &ovg.ports[card];
            for &port in ports {
                for (one, other) in [(1, 7), (7, 1), (4, 10), (10, 4)] {
                    if ovg.point(port) == ovg.point(ports[one]) && used.contains(&key(ovg.point(ports[other]))) {
                        symmetrical_ports.insert(port);
                    }
                }
            }
        }

        let count = ovg.points.len();
        let mut vertical_hops: Vec<Option<PointId>> = vec![None; count];
        let mut horizontal_hops: Vec<Option<PointId>> = vec![None; count];
        let mut reached: Vec<Option<Reached>> = vec![None; count];
        reached[source] = Some(Reached::new());

        let mut queue = Queue::default();
        // Each point can be reached twice, once along each axis. The column state goes first.
        queue.push(0.0, source, false);
        queue.push(0.0, source, true);

        while let Some(least) = queue.pop() {
            let (least_node, is_from_horizontal, least_distance) = (least.node, least.is_horizontal, least.priority);
            if least_node == target {
                let nodes = self.best_route(g_source, g_target, source, target, &reached, &vertical_hops, &horizontal_hops);
                return Ok((Route::through(ovg, edge_index, nodes), least_distance));
            }
            let least_point = ovg.point(least_node);
            let last_node = if is_from_horizontal { horizontal_hops[least_node] } else { vertical_hops[least_node] };

            let occupied: Vec<RouteId> =
                if least_node != source { self.routes_at(least_point).to_vec() } else { Vec::new() };
            let is_on_node_of_route = !occupied.is_empty();
            let all_occupied_shareable = self.can_overlap(edge_index, &occupied);

            for &e in &ovg.points[least_node].edges {
                let adjacent = ovg.adjacent(least_node, e);
                if Some(adjacent) == last_node {
                    continue;
                }
                let adjacent_node = &ovg.points[adjacent];
                let adjacent_point = adjacent_node.point;

                // An arrival that already costs as much as the best one on that axis cannot win.
                if let Some(known) = &reached[adjacent] {
                    let best = if adjacent_point.y == least_point.y {
                        known.horizontal_distance
                    } else {
                        known.vertical_distance
                    };
                    if least_distance >= best {
                        continue;
                    }
                }

                // A route leaves a port of its first card straight out of the card. This keeps the
                // search from stepping back into the centre it came from. Where the two cards
                // overlap or touch, any step across the side will do.
                if let Some(role) = ovg.points[least_node].role_for(g_source) {
                    let valid_exit = role.sides.each().into_iter().any(|side| match side {
                        None => true,
                        Some(Side::Top | Side::Bottom) if overlap => adjacent_point.y != least_point.y,
                        Some(Side::Right | Side::Left) if overlap => adjacent_point.x != least_point.x,
                        Some(Side::Top) => adjacent_point.y < least_point.y,
                        Some(Side::Bottom) => adjacent_point.y > least_point.y,
                        Some(Side::Right) => adjacent_point.x > least_point.x,
                        Some(Side::Left) => adjacent_point.x < least_point.x,
                    });
                    if !valid_exit {
                        continue;
                    }
                }
                // The same for reaching a port of the second card: from outside, straight at
                // its side. A port the two cards share is not reached at all. (TALA asks that
                // once more further down, for a port of the first card. A shared port is a
                // port of both, so this question has answered it.)
                if let Some(role) = adjacent_node.role_for(g_target) {
                    if blocked_ports.contains(&key(adjacent_point)) {
                        continue;
                    }
                    let valid_approach = role.sides.each().into_iter().any(|side| match side {
                        None => true,
                        Some(Side::Top | Side::Bottom) if overlap => adjacent_point.y != least_point.y,
                        Some(Side::Right | Side::Left) if overlap => adjacent_point.x != least_point.x,
                        Some(Side::Top) => adjacent_point.y > least_point.y,
                        Some(Side::Bottom) => adjacent_point.y < least_point.y,
                        Some(Side::Right) => adjacent_point.x < least_point.x,
                        Some(Side::Left) => adjacent_point.x > least_point.x,
                    });
                    if !valid_approach {
                        continue;
                    }
                }
                // No diagonal steps, except between a card's centre and its ports.
                if adjacent != target
                    && least_node != source
                    && adjacent_point.x != least_point.x
                    && adjacent_point.y != least_point.y
                {
                    continue;
                }

                let mut next_distance;
                if least_node == source {
                    next_distance = 1.0;
                    // A second connection between the same cards takes another port.
                    if !allow_port_sharing && duplicate_source_ports.contains(&key(adjacent_point)) {
                        next_distance = BASICALLY_INFINITY;
                    }
                    // A port that the route of another card of the cluster does not use costs
                    // two turns; so does a port on a side that does not face the other card.
                    if !shared_source_ports.is_empty() && !shared_source_ports.contains(&key(adjacent_point)) {
                        next_distance += 2.0 * costs.turn;
                    }
                    if prefer_facing_ports && !from_facing.contains(&key(adjacent_point)) {
                        next_distance += non_facing_ports_cost;
                    }
                    // Routes look best out of the middle port of a side, and a little better
                    // still when the opposite middle port is in use too.
                    if !adjacent_node.role_for(g_source).is_some_and(|role| role.is_center) {
                        next_distance += costs.non_center_port;
                    } else if !symmetrical_ports.contains(&adjacent) {
                        next_distance += 1.0;
                    }
                } else {
                    next_distance = ovg.edges[e].distance;
                }

                // TALA `sharesPortOwner`: a step between two ports of one card costs.
                if ovg.points[least_node].port_roles.iter().any(|(owner, _)| adjacent_node.is_port_of(*owner)) {
                    next_distance += costs.turn * 4.0;
                }

                if adjacent == target {
                    next_distance = 1.0;
                    if !allow_port_sharing && duplicate_target_ports.contains(&key(least_point)) {
                        next_distance = BASICALLY_INFINITY;
                    }
                    if !shared_target_ports.is_empty() && !shared_target_ports.contains(&key(least_point)) {
                        next_distance += 2.0 * costs.turn;
                    }
                    if prefer_facing_ports && !to_facing.contains(&key(least_point)) {
                        next_distance += non_facing_ports_cost;
                    }
                    if let Some(role) = ovg.points[least_node].role_for(g_target) {
                        if !role.is_center {
                            next_distance += costs.non_center_port;
                        } else if !symmetrical_ports.contains(&least_node) {
                            next_distance += 1.0;
                        }
                    }
                } else if adjacent_node.is_node_center {
                    next_distance = BASICALLY_INFINITY;
                } else {
                    let mut is_prohibited_sharing = false;
                    if least_node != source {
                        let routes = self.overlapping_routes.get(&e).cloned().unwrap_or_default();
                        is_prohibited_sharing = !self.can_overlap(edge_index, &routes)
                            || self.routes_block_traversal(&routes, least_node, adjacent);
                    }
                    let is_adjacent_on_route =
                        !is_prohibited_sharing && least_node != source && !self.routes_at(adjacent_point).is_empty();
                    if !is_prohibited_sharing
                        && !is_on_node_of_route
                        && !is_adjacent_on_route
                        && least_node != source
                        && self.has_nearby_edge.contains(&e)
                    {
                        is_prohibited_sharing = true;
                    }
                    if is_prohibited_sharing {
                        next_distance = BASICALLY_INFINITY;
                    } else {
                        // Crossing a route, either at a point of the graph or between two.
                        let is_crossing = (is_on_node_of_route && !is_adjacent_on_route && !all_occupied_shareable)
                            || (least_node != source && self.crossings.crossed.contains(&e));
                        if is_crossing {
                            next_distance += costs.crossing;
                        }
                    }
                }

                // Turns cost; leaving the first card and entering the second do not count.
                if let Some(last) = last_node
                    && adjacent != target
                    && least_node != source
                    && last != source
                {
                    let turns = (!is_from_horizontal && adjacent_point.x != least_point.x)
                        || (is_from_horizontal && adjacent_point.y != least_point.y);
                    if turns {
                        next_distance += costs.turn * turn_multiplier(&turn_axes, adjacent_point, even_cluster);
                        // A turn right next to one of the two cards looks bad.
                        if distance_to_boundary(least_point, target_card) <= TURN_ENDPOINT_CLEARANCE {
                            next_distance += costs.turn;
                        }
                        if distance_to_boundary(least_point, source_card) <= TURN_ENDPOINT_CLEARANCE {
                            next_distance += costs.turn;
                        }
                    }
                }

                // All else being equal, stay away from the ports of the two cards.
                if least_node != source
                    && adjacent != target
                    && (adjacent_node.near_port.contains(&g_source) || adjacent_node.near_port.contains(&g_target))
                {
                    next_distance += NODE_PROXIMITY_PENALTY;
                }

                let maybe_new_distance = least_distance + next_distance;
                let horizontal = adjacent_point.y == least_point.y;
                let hops = if horizontal { &mut horizontal_hops } else { &mut vertical_hops };
                let known = *reached[adjacent].get_or_insert_with(Reached::new);
                let (known_distance, known_entry) = if horizontal {
                    (known.horizontal_distance, known.horizontal_entry)
                } else {
                    (known.vertical_distance, known.vertical_entry)
                };
                if maybe_new_distance < known_distance {
                    // TALA puts the point this one was reached from before back into the queue
                    // here. That entry repeats one the point already has (or has an infinite
                    // price), so handling it never finds anything cheaper; it is left out.
                    let entry = match known_entry {
                        None => queue.push(maybe_new_distance, adjacent, horizontal),
                        Some(entry) => {
                            queue.decrease(entry, maybe_new_distance)?;
                            entry
                        }
                    };
                    let state = reached[adjacent].as_mut().expect("just set");
                    if horizontal {
                        state.horizontal_entry = Some(entry);
                        state.horizontal_distance = maybe_new_distance;
                    } else {
                        state.vertical_entry = Some(entry);
                        state.vertical_distance = maybe_new_distance;
                    }
                    hops[adjacent] = Some(least_node);
                }
            }
        }
        Err(RouteError::NoPath)
    }

    /// TALA `bestRoute`: walks back from the second card to the first, at every point along the
    /// cheaper of its two arrivals.
    #[allow(clippy::too_many_arguments)]
    fn best_route(
        &self,
        g_source: usize,
        g_target: usize,
        source: PointId,
        target: PointId,
        reached: &[Option<Reached>],
        vertical_hops: &[Option<PointId>],
        horizontal_hops: &[Option<PointId>],
    ) -> Vec<PointId> {
        let ovg = self.ovg;
        let turn_axes = ideal_turn_axes(&self.graph.nodes[g_source], &self.graph.nodes[g_target]);
        let mut sequence: Vec<PointId> = Vec::new();
        let mut current = Some(target);
        let mut previous: Option<PointId> = None;
        while let Some(node) = current {
            if sequence.contains(&node) {
                // A cycle: stop here.
                break;
            }
            sequence.push(node);
            let state = reached[node].expect("a point on the way back was reached");
            let (vertical_distance, horizontal_distance) = (state.vertical_distance, state.horizontal_distance);
            let (mut next, is_horizontal) = match (vertical_hops[node], horizontal_hops[node]) {
                (Some(vertical), Some(horizontal)) => {
                    if vertical_distance < horizontal_distance { (Some(vertical), false) } else { (Some(horizontal), true) }
                }
                (Some(vertical), None) => (Some(vertical), false),
                (None, horizontal) => (horizontal, true),
            };

            // The price of a turn sits on the point after the turn, not on the corner. So a
            // cheaper arrival without a turn can hide that the way through the corner is only
            // better by less than a turn. Put the price back here before choosing.
            if let (Some(before), Some(after)) = (previous, next)
                && after != source
            {
                let (before, here) = (ovg.point(before), ovg.point(node));
                if (before.x == here.x && is_horizontal) || (before.y == here.y && !is_horizontal) {
                    let turn = self.costs.turn * turn_multiplier(&turn_axes, here, in_even_cluster(self.graph, g_source));
                    let mut alternative = None;
                    let mut alternative_distance = 0.0;
                    if is_horizontal && clearly_more(horizontal_distance + turn, vertical_distance) {
                        alternative_distance = vertical_distance;
                        alternative = vertical_hops[node];
                    }
                    if !is_horizontal && clearly_more(vertical_distance + turn, horizontal_distance) {
                        alternative_distance = horizontal_distance;
                        alternative = horizontal_hops[node];
                    }
                    if alternative.is_some() && alternative_distance < BASICALLY_INFINITY {
                        next = alternative;
                    }
                }
            }
            previous = Some(node);
            current = next;
        }
        sequence.reverse();
        sequence
    }
}
