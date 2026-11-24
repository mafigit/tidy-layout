// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.

//! The routes of tree connections. A card of a tree and the card it hangs on are joined by
//! rule, before any search: a straight line where the two are in line, an S with two bends
//! halfway otherwise. The routing graph gets the points such a route needs.
//! Ported from TALA `internal/routing`: `tree_routes.go` (`treeEdgePathForBuild`,
//! `alignedPortNodeForBuild`, `treeEdgeMidpoints`), `ovg.go` (`addTreeNodes`) and
//! `coordinator.go` (`routeSentinelEdge`, `routeInSShape`, and the loop over the tree cards in
//! `routeEdgesWithResourceGuards`). For connections with an arrow at their target.

use std::collections::HashMap;

use crate::graph::{Graph, Node, Point, TreeLink};
use crate::routing::geometry::{Orientation, euclidean_distance};
use crate::routing::ovg::{Ovg, PointId, PortRole, Side, Sides};
use crate::routing::router::Route;

/// TALA `TreeParentSpacing`: the room between a card and the cards that hang on it. The two
/// bends of an S lie halfway.
const TREE_PARENT_SPACING: f64 = 100.0;
/// TALA `treeChildAlignmentTolerance`: two cards whose middles are this close count as in line.
const ALIGNMENT_TOLERANCE: f64 = 10.0;
/// TALA `endOfNodeBuffer`: a port made for a straight line keeps this far from a corner.
const END_OF_NODE_BUFFER: f64 = 5.0;

/// The arrowheads that tree routes have at a port so far: none (where a connection starts)
/// and a triangle (where one ends).
#[derive(Clone, Copy, Default)]
struct Arrowheads {
    none: bool,
    triangle: bool,
}

impl Arrowheads {
    fn has(self, triangle: bool) -> bool {
        if triangle { self.triangle } else { self.none }
    }
}

/// A port of a tree route: a point of the routing graph, or a place that is none (yet).
#[derive(Clone, Copy)]
enum Port {
    At(PointId),
    Loose(Point),
}

/// TALA `TreeEdgePath`: the two ports and the two bends of a tree route, from the card the
/// connection starts at to the card it ends at.
struct TreePath {
    source_port: Port,
    target_port: Port,
    source_midpoint: Point,
    target_midpoint: Point,
    /// Where the first card is, seen from the second. TALA `SourceOrientationToTarget`.
    side: Orientation,
}

/// TALA `shapeSquare.CenterPortIndex`: the middle port of a side, by its place in a card's ports.
fn center_port(side: Orientation) -> usize {
    match side {
        Orientation::Top => 1,
        Orientation::Left => 4,
        Orientation::Bottom => 7,
        _ => 10,
    }
}

fn side_of(orientation: Orientation) -> Side {
    match orientation {
        Orientation::Top => Side::Top,
        Orientation::Left => Side::Left,
        Orientation::Bottom => Side::Bottom,
        _ => Side::Right,
    }
}

fn is_upright(side: Orientation) -> bool {
    matches!(side, Orientation::Top | Orientation::Bottom)
}

/// TALA `alignedPortNodeForBuild`: where the two cards are nearly in line but their ports are
/// not, the card of the tree gets a port straight across from the other card's port. (TALA
/// asks first whether the two ports are out of line. Where they are in line, the place across
/// is the card's own port, and that is what comes of it.)
fn aligned_port(parent: &Node, child: &Node, side: Orientation, parent_port: Point, child_port: PointId, child_at: Point) -> Port {
    let (parent_center, child_center) = (parent.center(), child.center());
    if is_upright(side)
        && (parent_center.x - child_center.x).abs() <= ALIGNMENT_TOLERANCE
        && child.top_left.x + END_OF_NODE_BUFFER < parent_port.x
        && parent_port.x < child.top_left.x + child.width - END_OF_NODE_BUFFER
    {
        return Port::Loose(Point { x: parent_port.x, y: child_at.y });
    }
    if !is_upright(side)
        && (parent_center.y - child_center.y).abs() <= ALIGNMENT_TOLERANCE
        && child.top_left.y + END_OF_NODE_BUFFER < parent_port.y
        && parent_port.y < child.top_left.y + child.height - END_OF_NODE_BUFFER
    {
        return Port::Loose(Point { x: child_at.x, y: parent_port.y });
    }
    Port::At(child_port)
}

/// TALA `treeEdgeMidpoints`: the two bends, halfway through the room beside the card the tree
/// hangs on: the one on the line of that card's port, the one on the line of the tree card's.
fn midpoints(parent: &Node, side: Orientation, parent_port: Point, child_port: Point) -> (Point, Point) {
    match side {
        Orientation::Left | Orientation::Right => {
            let x = if side == Orientation::Left { parent.top_left.x - TREE_PARENT_SPACING / 2.0 } else { parent.top_left.x + parent.width + TREE_PARENT_SPACING / 2.0 };
            (Point { x, y: parent_port.y }, Point { x, y: child_port.y })
        }
        _ => {
            let y = if side == Orientation::Top { parent.top_left.y - TREE_PARENT_SPACING / 2.0 } else { parent.top_left.y + parent.height + TREE_PARENT_SPACING / 2.0 };
            (Point { x: parent_port.x, y }, Point { x: child_port.x, y })
        }
    }
}

/// TALA `treeEdgePathForBuild`: the path of the connection between a card of a tree and the
/// card it hangs on. Both use the middle port of the sides that face each other. Where the
/// middle port of the card the tree hangs on has a tree route with another arrowhead already,
/// the nearest port of that side without one is taken.
fn tree_path(graph: &Graph, ovg: &Ovg, card: usize, link: TreeLink, arrowheads: &HashMap<PointId, Arrowheads>) -> TreePath {
    let edge = graph.edges[link.edge];
    let from_tree = edge.from == card;
    let parent = if from_tree { edge.to } else { edge.from };
    let mut parent_port = ovg.ports[parent][center_port(link.side)];
    let child_port = ovg.ports[card][center_port(link.side.opposite())];
    let child_at = ovg.point(child_port);
    // The arrowhead at the card the tree hangs on: a triangle where the connection ends there.
    // (A port that is known has an arrowhead: TALA also asks whether it has any. And TALA
    // leaves the middle port out of the ports it looks at next: that port has another
    // arrowhead, which leaves it out anyway.)
    if arrowheads.get(&parent_port).is_some_and(|found| !found.has(from_tree)) {
        let mut closest: Option<(PointId, f64)> = None;
        for index in crate::routing::line::port_indices(link.side) {
            let port = ovg.ports[parent][*index];
            if arrowheads.get(&port).is_some_and(|found| !found.has(from_tree)) {
                continue;
            }
            let at = ovg.point(port);
            let distance = euclidean_distance(at.x, at.y, child_at.x, child_at.y);
            if closest.is_none_or(|(_, least)| distance < least) {
                closest = Some((port, distance));
            }
        }
        if let Some((port, _)) = closest {
            parent_port = port;
        }
    }
    let parent_at = ovg.point(parent_port);
    let mut child = aligned_port(&graph.nodes[parent], &graph.nodes[card], link.side, parent_at, child_port, child_at);
    // A port made for a straight line is a point of the graph once the graph is built.
    if let Port::Loose(at) = child
        && let Some(known) = ovg.ports[card].iter().find(|port| ovg.point(**port) == at)
    {
        child = Port::At(*known);
    }
    let child_point = match child {
        Port::At(port) => ovg.point(port),
        Port::Loose(at) => at,
    };
    let (parent_midpoint, child_midpoint) = midpoints(&graph.nodes[parent], link.side, parent_at, child_point);
    if from_tree {
        TreePath { source_port: child, target_port: Port::At(parent_port), source_midpoint: child_midpoint, target_midpoint: parent_midpoint, side: link.side }
    } else {
        TreePath { source_port: Port::At(parent_port), target_port: child, source_midpoint: parent_midpoint, target_midpoint: child_midpoint, side: link.side.opposite() }
    }
}

/// TALA `addTreeNodes`: the two bends of every tree route become points of the graph, and so
/// does a port made for a straight line, as a port of its card.
pub(crate) fn add_tree_nodes(ovg: &mut Ovg, graph: &Graph) {
    let mut arrowheads: HashMap<PointId, Arrowheads> = HashMap::new();
    for card in 0..graph.nodes.len() {
        let Some(link) = graph.tree_link(card) else {
            continue;
        };
        let path = tree_path(graph, ovg, card, link, &arrowheads);
        ovg.add_point(path.source_midpoint);
        ovg.add_point(path.target_midpoint);
        let from_tree = graph.edges[link.edge].from == card;
        let (own, other) = if from_tree { (path.source_port, path.target_port) } else { (path.target_port, path.source_port) };
        let port = match own {
            Port::At(port) => port,
            Port::Loose(at) => ovg.add_point(at),
        };
        if !ovg.points[port].is_port_of(card) {
            ovg.ports[card].push(port);
            ovg.points[port].add_role(card, PortRole { sides: Sides::of(side_of(link.side.opposite())), is_center: false });
        }
        // Only the port of the card the tree hangs on counts here: it may get several routes.
        if let Port::At(port) = other {
            let found = arrowheads.entry(port).or_default();
            if from_tree {
                found.triangle = true;
            } else {
                found.none = true;
            }
        }
    }
}

/// TALA `routeInSShape`: the points of the graph from the first port to the second: away from
/// the first card to its bend, across to the other bend, on to the second card. `None` where
/// the graph has no edge for a step.
fn s_shape(ovg: &Ovg, path: &TreePath) -> Option<Vec<PointId>> {
    // A port that is no point of the graph has no edge to walk along.
    let Port::At(start) = path.source_port else {
        return None;
    };
    let target = match path.target_port {
        Port::At(port) => ovg.point(port),
        Port::Loose(at) => at,
    };
    // The nearest neighbour of a point on the side away from where `side` says the point is.
    let step = |current: PointId, side: Orientation| -> Option<PointId> {
        let at = ovg.point(current);
        let mut closest: Option<PointId> = None;
        for &edge in &ovg.points[current].edges {
            let next = ovg.adjacent(current, edge);
            let (to, best) = (ovg.point(next), closest.map(|closest| ovg.point(closest)));
            let nearer = match side {
                Orientation::Left => at.y == to.y && to.x > at.x && best.is_none_or(|best| to.x < best.x),
                Orientation::Right => at.y == to.y && to.x < at.x && best.is_none_or(|best| to.x > best.x),
                Orientation::Top => at.x == to.x && to.y > at.y && best.is_none_or(|best| to.y < best.y),
                _ => at.x == to.x && to.y < at.y && best.is_none_or(|best| to.y > best.y),
            };
            if nearer {
                closest = Some(next);
            }
        }
        closest
    };
    let source = ovg.point(start);
    let across = if is_upright(path.side) {
        if source.x < target.x { Orientation::Left } else { Orientation::Right }
    } else if source.y < target.y {
        Orientation::Top
    } else {
        Orientation::Bottom
    };
    let mut nodes = vec![start];
    let mut current = start;
    for (goal, side) in [(path.source_midpoint, path.side), (path.target_midpoint, across), (target, path.side)] {
        while ovg.point(current) != goal {
            current = step(current, side)?;
            nodes.push(current);
        }
    }
    Some(nodes)
}

/// The tree routes of a graph, in the order of its cards: TALA `routeSentinelEdge` for every
/// card of a tree whose connection TALA takes for a tree connection. A connection whose route
/// cannot be walked in the graph, or runs over a port of a third card, gets none here: the
/// search routes it with the others.
pub(crate) fn tree_routes(graph: &Graph, ovg: &Ovg) -> Vec<Route> {
    let mut routes = Vec::new();
    let mut arrowheads: HashMap<PointId, Arrowheads> = HashMap::new();
    for card in 0..graph.nodes.len() {
        let Some(link) = graph.tree_link(card).filter(|link| graph.is_tree_edge(link.edge)) else {
            continue;
        };
        let path = tree_path(graph, ovg, card, link, &arrowheads);
        let Some(walked) = s_shape(ovg, &path) else {
            continue;
        };
        let edge = graph.edges[link.edge];
        let foreign = |point: &PointId| ovg.points[*point].is_port() && !ovg.points[*point].is_port_of(edge.from) && !ovg.points[*point].is_port_of(edge.to);
        if walked.iter().any(foreign) {
            continue;
        }
        let (first, last) = (walked[0], walked[walked.len() - 1]);
        let nodes = std::iter::once(ovg.centers[edge.from]).chain(walked).chain(std::iter::once(ovg.centers[edge.to])).collect();
        routes.push(Route::through(ovg, link.edge, nodes));
        arrowheads.entry(first).or_default().none = true;
        arrowheads.entry(last).or_default().triangle = true;
    }
    routes
}
