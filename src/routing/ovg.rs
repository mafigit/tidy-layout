// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.

//! The orthogonal visibility graph: the points a route may pass and the straight, free stretches
//! between them. Ported from TALA `internal/routing`: `ovg.go`, `ovg_node.go`, `ovg_edge.go` and
//! the lookups in `ovg_resource.go`, for plain boxes. Not ported: containers, clusters, sequences,
//! trees, hierarchies, fixed and invisible nodes, shapes other than the rectangle, and the work
//! budgets.
//!
//! TALA walks some Go maps while it builds the graph, so the order of `Ovg::nodes` differs from run
//! to run there. Everything else (which points exist, their flags, the edges and their order) is
//! the same on every run, and that is what the reference test compares.

use std::cmp::Ordering;
use std::collections::{BTreeMap, HashMap, VecDeque};
use std::hash::{Hash, Hasher};

use crate::graph::{Graph, Node, Point};
use crate::routing::geometry::{Orientation, euclidean_distance};
use crate::routing::tuning::{
    EXTRA_POINT_LAYERS, MIN_PORT_CLEARANCE, MIN_ROUTE_NODE_CLEARANCE, NODE_PROXIMITY_THRESHOLD,
    OVERSHOOT, OVG_PADDING,
};
use crate::routing::ovg_hierarchy::build_hierarchy;
use crate::routing::tree_routes::add_tree_nodes;
use crate::routing::tunnel::build_tunnels;

/// Index of a point in `Ovg::points`.
pub type PointId = usize;
/// Index of an edge in `Ovg::edges`.
pub type EdgeId = usize;

/// A coordinate as a map key: ordered, and -0 is 0.
#[derive(Clone, Copy, Debug)]
pub struct Coord(f64);

impl Coord {
    pub fn of(value: f64) -> Self {
        Coord(value + 0.0)
    }

    pub fn value(self) -> f64 {
        self.0
    }
}

impl PartialEq for Coord {
    fn eq(&self, other: &Self) -> bool {
        self.0.to_bits() == other.0.to_bits()
    }
}

impl Eq for Coord {}

impl Hash for Coord {
    fn hash<H: Hasher>(&self, state: &mut H) {
        self.0.to_bits().hash(state);
    }
}

impl Ord for Coord {
    fn cmp(&self, other: &Self) -> Ordering {
        self.0.total_cmp(&other.0)
    }
}

impl PartialOrd for Coord {
    fn partial_cmp(&self, other: &Self) -> Option<Ordering> {
        Some(self.cmp(other))
    }
}

pub(crate) fn key(point: Point) -> (Coord, Coord) {
    (Coord::of(point.x), Coord::of(point.y))
}

/// The side of its card a port looks out of.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Side {
    Top,
    Right,
    Bottom,
    Left,
}

const SIDES: [Side; 4] = [Side::Top, Side::Right, Side::Bottom, Side::Left];

/// The sides one point is a port of, for one card. TALA `portDirectionSet`.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Sides(u8);

impl Sides {
    fn bit(side: Side) -> u8 {
        match side {
            Side::Top => 1,
            Side::Right => 2,
            Side::Bottom => 4,
            Side::Left => 8,
        }
    }

    pub fn has(self, side: Side) -> bool {
        self.0 & Sides::bit(side) != 0
    }

    fn add(&mut self, side: Side) {
        self.0 |= Sides::bit(side);
    }

    /// One side.
    pub(crate) fn of(side: Side) -> Self {
        Sides(Sides::bit(side))
    }

    /// TALA `portDirectionSet.any`: the sides in TALA's order; a set without sides counts as one
    /// unrestricted entry.
    pub(crate) fn each(self) -> Vec<Option<Side>> {
        if self.0 == 0 {
            return vec![None];
        }
        SIDES.iter().filter(|side| self.has(**side)).map(|side| Some(*side)).collect()
    }
}

/// What a point is for one card. TALA `ovgPortMetadata`.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct PortRole {
    pub sides: Sides,
    /// The middle port of its side.
    pub is_center: bool,
}

/// A point of the graph. TALA `OVGNode`.
#[derive(Clone, Debug)]
pub struct OvgNode {
    pub point: Point,
    /// In the order TALA connects them: along the row, then along the column, then the card centre.
    pub edges: Vec<EdgeId>,
    /// The cards that have a port close by on the same line. Sorted.
    pub near_port: Vec<usize>,
    /// The cards this point is a port of, in the order they were added.
    pub port_roles: Vec<(usize, PortRole)>,
    /// Position in `Ovg::nodes`.
    pub index: usize,
    /// The centre of a card: a route starts and ends there.
    pub is_node_center: bool,
    /// One end of a tunnel: a direct line between two connected cards that face each other.
    pub is_tunnel: bool,
}

impl OvgNode {
    fn new(point: Point) -> Self {
        OvgNode {
            point,
            edges: Vec::new(),
            near_port: Vec::new(),
            port_roles: Vec::new(),
            index: 0,
            is_node_center: false,
            is_tunnel: false,
        }
    }

    pub fn is_port(&self) -> bool {
        !self.port_roles.is_empty()
    }

    pub fn role_for(&self, card: usize) -> Option<PortRole> {
        self.port_roles.iter().find(|(owner, _)| *owner == card).map(|(_, role)| *role)
    }

    pub fn is_port_of(&self, card: usize) -> bool {
        self.role_for(card).is_some()
    }

    /// TALA `addPortMetadata`.
    pub(crate) fn add_role(&mut self, card: usize, added: PortRole) {
        match self.port_roles.iter_mut().find(|(owner, _)| *owner == card) {
            Some((_, role)) => {
                role.sides.0 |= added.sides.0;
                role.is_center = role.is_center || added.is_center;
            }
            None => self.port_roles.push((card, added)),
        }
    }

    fn sides_for(&self, card: usize) -> Sides {
        self.role_for(card).map(|role| role.sides).unwrap_or_default()
    }
}

/// A free, straight stretch between two points. TALA `OVGEdge`.
#[derive(Clone, Copy, Debug)]
pub struct OvgEdge {
    pub from: PointId,
    pub to: PointId,
    pub distance: f64,
}

/// TALA `OVG`.
#[derive(Clone, Debug, Default)]
pub struct Ovg {
    /// Every point that was made, also the ones that ended up without an edge.
    pub points: Vec<OvgNode>,
    /// The points of the graph. TALA `OVG.Nodes`.
    pub nodes: Vec<PointId>,
    /// TALA `OccupiedPoints`. A card centre is entered last, so it wins over a point at its place.
    pub occupied: HashMap<(Coord, Coord), PointId>,
    pub edges: Vec<OvgEdge>,
    pub vertical_edges: BTreeMap<Coord, Vec<EdgeId>>,
    pub horizontal_edges: BTreeMap<Coord, Vec<EdgeId>>,
    /// Per card: its ports. The first twelve are three per side (top, left, bottom, right), then
    /// tunnel ends. A point can be in the list twice.
    pub ports: Vec<Vec<PointId>>,
    /// Per card: its centre point.
    pub centers: Vec<PointId>,
}

impl Ovg {
    pub fn point(&self, id: PointId) -> Point {
        self.points[id].point
    }

    /// The other end of an edge. TALA `OVGNode.adjacent`.
    pub fn adjacent(&self, node: PointId, edge: EdgeId) -> PointId {
        let edge = &self.edges[edge];
        if edge.from == node { edge.to } else { edge.from }
    }

    fn add_unchecked(&mut self, node: OvgNode) -> PointId {
        let id = self.points.len();
        self.occupied.insert(key(node.point), id);
        self.points.push(node);
        self.nodes.push(id);
        id
    }

    /// TALA `addPoint`: the point at this place, made if there is none.
    pub(crate) fn add_point(&mut self, point: Point) -> PointId {
        match self.occupied.get(&key(point)) {
            Some(id) => *id,
            None => self.add_unchecked(OvgNode::new(point)),
        }
    }

    /// TALA `OVG.Connect`. Its rule that only ports connect to a card centre is not needed while
    /// building: it comes with the code that adds existing routes to the graph.
    pub(crate) fn connect(&mut self, a: PointId, b: PointId) -> EdgeId {
        let (pa, pb) = (self.points[a].point, self.points[b].point);
        let id = self.edges.len();
        self.edges.push(OvgEdge {
            from: a,
            to: b,
            distance: euclidean_distance(pa.x, pa.y, pb.x, pb.y),
        });
        self.points[a].edges.push(id);
        self.points[b].edges.push(id);
        if pa.x == pb.x {
            self.vertical_edges.entry(Coord::of(pa.x)).or_default().push(id);
        } else if pa.y == pb.y {
            self.horizontal_edges.entry(Coord::of(pa.y)).or_default().push(id);
        }
        id
    }

    /// The ports of a card that look out of one side, each once. TALA `portsByOrientation`.
    pub(crate) fn ports_on_side(&self, card: usize, side: Side) -> Vec<PointId> {
        let mut found: Vec<PointId> = Vec::new();
        for &port in &self.ports[card] {
            if !found.contains(&port) && self.points[port].sides_for(card).has(side) {
                found.push(port);
            }
        }
        found
    }

    /// TALA `passesThroughAllowingPorts`: a segment that leaves a card through one of its ports,
    /// in the direction the port looks, does not count as passing through that card.
    fn passes_through_allowing_ports(
        &self,
        graph: &Graph,
        card: usize,
        from: Point,
        to: Point,
        side: Option<Side>,
    ) -> bool {
        let leaves = match side {
            Some(Side::Top) => from.x == to.x && from.y > to.y,
            Some(Side::Bottom) => from.x == to.x && from.y < to.y,
            Some(Side::Left) => from.y == to.y && from.x > to.x,
            Some(Side::Right) => from.y == to.y && from.x < to.x,
            None => false,
        };
        if leaves
            && self.ports[card].iter().any(|port| {
                let p = self.points[*port].point;
                p == from || p == to
            })
        {
            return false;
        }
        graph.nodes[card].passes_through(from, to)
    }
}

/// TALA `pointNearGraphNode`: within 20 of a card of the group that is routed. Cards of other
/// groups that are only in the way are not asked.
fn point_near(graph: &Graph, point: Point) -> bool {
    graph.nodes[graph.nearby..].iter().any(|node| node.is_point_near(point))
}

/// TALA `tightBoundingBox`.
fn bounding_box(nodes: &[Node]) -> (Point, Point) {
    let mut tl = Point { x: f64::INFINITY, y: f64::INFINITY };
    let mut br = Point { x: f64::NEG_INFINITY, y: f64::NEG_INFINITY };
    for node in nodes {
        tl.x = tl.x.min(node.top_left.x);
        tl.y = tl.y.min(node.top_left.y);
        br.x = br.x.max(node.right());
        br.y = br.y.max(node.bottom());
    }
    (tl, br)
}

/// Where the ports are, for the questions asked about every candidate point. TALA `ovgPortIndex`.
struct PortIndex {
    /// The ports on a row, by their place along it, and the same for a column.
    x_by_y: HashMap<Coord, Vec<f64>>,
    y_by_x: HashMap<Coord, Vec<f64>>,
    owners_by_x: HashMap<Coord, Vec<usize>>,
    owners_by_y: HashMap<Coord, Vec<usize>>,
    vertical_blockers: HashMap<Coord, Vec<usize>>,
    horizontal_blockers: HashMap<Coord, Vec<usize>>,
}

impl PortIndex {
    fn new(ovg: &Ovg, graph: &Graph) -> Self {
        let mut index = PortIndex {
            x_by_y: HashMap::new(),
            y_by_x: HashMap::new(),
            owners_by_x: HashMap::new(),
            owners_by_y: HashMap::new(),
            vertical_blockers: HashMap::new(),
            horizontal_blockers: HashMap::new(),
        };
        for (owner, ports) in ovg.ports.iter().enumerate() {
            for &port in ports {
                let p = ovg.points[port].point;
                index.x_by_y.entry(Coord::of(p.y)).or_default().push(p.x);
                index.y_by_x.entry(Coord::of(p.x)).or_default().push(p.y);
                index.owners_by_x.entry(Coord::of(p.x)).or_default().push(owner);
                index.owners_by_y.entry(Coord::of(p.y)).or_default().push(owner);
            }
        }
        // Only the cards of the group that is routed block a view (TALA takes them from the
        // group's own graph), not the cards of other groups that are in the way.
        for x in index.owners_by_x.keys() {
            let blockers = (graph.nearby..graph.nodes.len())
                .filter(|n| {
                    let node = &graph.nodes[*n];
                    node.top_left.x <= x.value() && x.value() <= node.right()
                })
                .collect();
            index.vertical_blockers.insert(*x, blockers);
        }
        for y in index.owners_by_y.keys() {
            let blockers = (graph.nearby..graph.nodes.len())
                .filter(|n| {
                    let node = &graph.nodes[*n];
                    node.top_left.y <= y.value() && y.value() <= node.bottom()
                })
                .collect();
            index.horizontal_blockers.insert(*y, blockers);
        }
        index
    }

    /// TALA `tooClose`: a port lies on the point's row or column, 20 away or less.
    fn too_close(&self, point: Point) -> bool {
        let within = |values: Option<&Vec<f64>>, coordinate: f64| values.is_some_and(|values| values.iter().any(|value| (value - coordinate).abs() <= MIN_PORT_CLEARANCE));
        within(self.x_by_y.get(&Coord::of(point.y)), point.x) || within(self.y_by_x.get(&Coord::of(point.x)), point.y)
    }

    /// TALA `hasUnobstructedLineToPorts`: the point sees a port of `wanted` different cards along
    /// its row or column, with no other card in the way. A point that is in line with no port at
    /// all also passes.
    fn sees_ports(&self, ovg: &Ovg, graph: &Graph, point: Point, wanted: usize) -> bool {
        let empty = Vec::new();
        let by_x = self.owners_by_x.get(&Coord::of(point.x)).unwrap_or(&empty);
        let by_y = self.owners_by_y.get(&Coord::of(point.y)).unwrap_or(&empty);
        let mut owners: Vec<usize> = by_x.iter().chain(by_y.iter()).copied().collect();
        owners.sort_unstable();
        owners.dedup();
        let mut seen = 0;
        for &owner in &owners {
            let card = &graph.nodes[owner];
            for &port in &ovg.ports[owner] {
                let p = ovg.points[port].point;
                if p.x != point.x && p.y != point.y {
                    continue;
                }
                // A line that runs along the border of the port's own card is no clear view.
                if p.x == point.x && (p.x == card.top_left.x || p.x == card.right()) {
                    continue;
                }
                if p.y == point.y && (p.y == card.top_left.y || p.y == card.bottom()) {
                    continue;
                }
                let blockers = if p.x == point.x {
                    self.vertical_blockers.get(&Coord::of(p.x))
                } else {
                    self.horizontal_blockers.get(&Coord::of(p.y))
                };
                let blocked = blockers.is_some_and(|blockers| {
                    blockers.iter().any(|other| {
                        *other != owner && graph.nodes[*other].passes_through(p, point)
                    })
                });
                if !blocked {
                    seen += 1;
                    if seen == wanted {
                        return true;
                    }
                    break;
                }
            }
        }
        owners.is_empty()
    }
}

/// Builds the graph for cards at their places. TALA `buildOVGFromGraphWithGuard`.
pub fn build(graph: &Graph) -> Ovg {
    let mut ovg = Ovg::default();
    if graph.nodes.is_empty() {
        return ovg;
    }
    // TALA `buildOVGFromGraphWithGuard`: every hierarchy of the group gets a graph of its own
    // first, in the order of the group's cards. Its ports join the graph before the points
    // where rows and columns of ports meet are made, its other points after.
    let mut hierarchies: Vec<usize> = Vec::new();
    for card in graph.nearby..graph.nodes.len() {
        if let Some((hierarchy, _)) = graph.level(card)
            && !hierarchies.contains(&hierarchy)
        {
            hierarchies.push(hierarchy);
        }
    }
    let lanes: Vec<Ovg> = hierarchies.iter().map(|hierarchy| build_hierarchy(graph, *hierarchy)).collect();
    add_ports(&mut ovg, graph, |card| graph.level(card).is_none());
    for lane in &lanes {
        merge_ports(&mut ovg, lane);
    }
    add_port_intersections(&mut ovg, graph);
    for lane in &lanes {
        merge_other_points(&mut ovg, lane);
    }
    add_edge_points(&mut ovg, graph);
    add_tree_nodes(&mut ovg, graph);
    let (tl, br) = bounding_box(&graph.nodes);
    add_port_connections_at_boundaries(&mut ovg, graph, tl, br);
    add_corners(&mut ovg, graph, tl, br);
    add_tunnels(&mut ovg, graph);
    connect_points(&mut ovg, graph);
    connect_ports_to_centers(&mut ovg, graph);
    remove_isolated(&mut ovg);
    flag_points_near_ports(&mut ovg, graph);
    for (index, id) in ovg.nodes.clone().into_iter().enumerate() {
        ovg.points[id].index = index;
    }
    ovg
}

/// TALA `addPorts`: three ports per side of a rectangle (`shapeSquare.SnapPointPercentages`), the
/// middle one of each side is its centre port. Without any connection there is nothing to route
/// and the cards get no ports.
pub(crate) fn add_ports(ovg: &mut Ovg, graph: &Graph, wanted: impl Fn(usize) -> bool) {
    const SHARES: [f64; 3] = [0.25, 0.5, 0.75];
    if graph.edges.is_empty() {
        ovg.ports = vec![Vec::new(); graph.nodes.len()];
        return;
    }
    for (card, node) in graph.nodes.iter().enumerate() {
        // A card that is not asked for gets its ports elsewhere (a card of a hierarchy).
        if !wanted(card) {
            ovg.ports.push(Vec::new());
            continue;
        }
        let mut ports = Vec::with_capacity(12);
        for side in [Side::Top, Side::Left, Side::Bottom, Side::Right] {
            for share in SHARES {
                let (x_share, y_share) = match side {
                    Side::Top => (share, 0.0),
                    Side::Left => (0.0, share),
                    Side::Bottom => (share, 1.0),
                    Side::Right => (1.0, share),
                };
                let port = ovg.add_point(Point {
                    x: node.top_left.x + (node.width * x_share).round(),
                    y: node.top_left.y + (node.height * y_share).round(),
                });
                let mut sides = Sides::default();
                sides.add(side);
                ovg.points[port].add_role(card, PortRole { sides, is_center: false });
                ports.push(port);
            }
        }
        for index in [1, 4, 7, 10] {
            ovg.points[ports[index]].add_role(card, PortRole { sides: Sides::default(), is_center: true });
        }
        ovg.ports.push(ports);
    }
}

/// TALA `addNodesIntersections`: where a row of one port meets a column of another, when no port
/// is close on the same row or column (`tooClose`), the point is clear of the group's cards and
/// sees ports of two cards. For a card of the group a point close to one of its ports is always
/// near the card too. For a card of another group that is only in the way, the ports are all
/// that keeps a point off: a point may lie right beside such a card.
pub(crate) fn add_port_intersections(ovg: &mut Ovg, graph: &Graph) {
    let xs: Vec<f64> = ovg.nodes.iter().map(|id| ovg.points[*id].point.x).collect();
    let ys: Vec<f64> = ovg.nodes.iter().map(|id| ovg.points[*id].point.y).collect();
    add_intersections(ovg, graph, xs, ys);
}

/// TALA `addIntersections`: a point where one of the columns meets one of the rows, under the
/// conditions of [`add_port_intersections`].
pub(crate) fn add_intersections(ovg: &mut Ovg, graph: &Graph, mut xs: Vec<f64>, mut ys: Vec<f64>) {
    for values in [&mut xs, &mut ys] {
        values.sort_by(f64::total_cmp);
        values.dedup();
    }
    let index = PortIndex::new(ovg, graph);
    for &x in &xs {
        let near: Vec<&Node> = graph.nodes[graph.nearby..]
            .iter()
            .filter(|node| {
                node.top_left.x - MIN_ROUTE_NODE_CLEARANCE <= x
                    && node.right() + MIN_ROUTE_NODE_CLEARANCE >= x
            })
            .collect();
        for &y in &ys {
            let point = Point { x, y };
            if index.too_close(point) || near.iter().any(|node| node.is_point_near(point)) {
                continue;
            }
            if index.sees_ports(ovg, graph, point, 2) {
                ovg.add_point(point);
            }
        }
    }
}

/// TALA `addEdgesNodes`: for every connection, points around its two cards and halfway between
/// them.
fn add_edge_points(ovg: &mut Ovg, graph: &Graph) {
    let index = PortIndex::new(ovg, graph);
    for edge in &graph.edges {
        if edge.from == edge.to {
            continue;
        }
        // Between two cards of one hierarchy the hierarchy has made the points already.
        if let (Some((from, _)), Some((to, _))) = (graph.level(edge.from), graph.level(edge.to))
            && from == to
        {
            continue;
        }
        let mut fill = perimeter_points(ovg, graph, edge.from, edge.to);
        fill.extend(halfway_points(ovg, graph, edge.from, edge.to));
        for point in fill {
            if point_near(graph, point) {
                continue;
            }
            if index.sees_ports(ovg, graph, point, 1) {
                ovg.add_point(point);
            }
        }
    }
}

/// TALA `perimeterPoints`: the ports of both cards, moved out to a box around the two.
fn perimeter_points(ovg: &Ovg, graph: &Graph, from: usize, to: usize) -> Vec<Point> {
    let (node, other) = (&graph.nodes[from], &graph.nodes[to]);
    let top = node.top_left.y.min(other.top_left.y) - OVERSHOOT;
    let bottom = node.bottom().max(other.bottom()) + OVERSHOOT;
    let left = node.top_left.x.min(other.top_left.x) - OVERSHOOT;
    let right = node.right().max(other.right()) + OVERSHOOT;
    let mut fill = Vec::new();
    for card in [from, to] {
        for port in ovg.ports_on_side(card, Side::Top) {
            fill.push(Point { x: ovg.point(port).x, y: top });
        }
        for port in ovg.ports_on_side(card, Side::Bottom) {
            fill.push(Point { x: ovg.point(port).x, y: bottom });
        }
        for port in ovg.ports_on_side(card, Side::Left) {
            fill.push(Point { x: left, y: ovg.point(port).y });
        }
        for port in ovg.ports_on_side(card, Side::Right) {
            fill.push(Point { x: right, y: ovg.point(port).y });
        }
    }
    fill.push(Point { x: left, y: top });
    fill.push(Point { x: right, y: top });
    fill.push(Point { x: left, y: bottom });
    fill.push(Point { x: right, y: bottom });
    fill
}

/// TALA `halfwayPoints`: the ports of both cards, moved to the line halfway between the cards.
fn halfway_points(ovg: &Ovg, graph: &Graph, from: usize, to: usize) -> Vec<Point> {
    let (node, other) = (&graph.nodes[from], &graph.nodes[to]);
    let (fill_top, fill_right, fill_bottom, fill_left) = match node.orientation(other) {
        Orientation::None => return Vec::new(),
        Orientation::Top => (false, false, true, false),
        Orientation::TopLeft => (false, true, true, false),
        Orientation::TopRight => (false, false, true, true),
        Orientation::Bottom => (true, false, false, false),
        Orientation::BottomLeft => (true, true, false, false),
        Orientation::BottomRight => (true, false, false, true),
        Orientation::Left => (false, true, false, false),
        Orientation::Right => (false, false, false, true),
    };
    // All cards of a cluster take the same halfway line: from the box around the cluster
    // (not rounded, as TALA's `tightBoundingBox`).
    let corners = |card: usize, node: &Node| match graph.cluster_of(card) {
        Some(cluster) => {
            let (mut tl, mut br) = (Point { x: f64::INFINITY, y: f64::INFINITY }, Point { x: f64::NEG_INFINITY, y: f64::NEG_INFINITY });
            for member in graph.clusters[cluster].members.iter().map(|member| &graph.nodes[*member]) {
                tl = Point { x: tl.x.min(member.top_left.x), y: tl.y.min(member.top_left.y) };
                br = Point { x: br.x.max(member.right()), y: br.y.max(member.bottom()) };
            }
            (tl, br)
        }
        None => (node.top_left, Point { x: node.right().round(), y: node.bottom().round() }),
    };
    let (node_tl, node_br) = corners(from, node);
    let (other_tl, other_br) = corners(to, other);

    let ports: Vec<Point> =
        ovg.ports[from].iter().chain(ovg.ports[to].iter()).map(|port| ovg.point(*port)).collect();
    let top_y = (node_tl.y + other_br.y) / 2.0;
    let bottom_y = (other_tl.y + node_br.y) / 2.0;
    let left_x = (node_tl.x + other_br.x) / 2.0;
    let right_x = (other_tl.x + node_br.x) / 2.0;

    let mut fill = Vec::new();
    if fill_top {
        fill.extend(ports.iter().map(|port| Point { x: port.x, y: top_y }));
    }
    if fill_bottom {
        fill.extend(ports.iter().map(|port| Point { x: port.x, y: bottom_y }));
    }
    if fill_left {
        fill.extend(ports.iter().map(|port| Point { x: left_x, y: port.y }));
    }
    if fill_right {
        fill.extend(ports.iter().map(|port| Point { x: right_x, y: port.y }));
    }
    if fill_right && fill_bottom {
        fill.push(Point { x: right_x, y: bottom_y });
    }
    if fill_right && fill_top {
        fill.push(Point { x: right_x, y: top_y });
    }
    if fill_left && fill_top {
        fill.push(Point { x: left_x, y: top_y });
    }
    if fill_left && fill_bottom {
        fill.push(Point { x: left_x, y: bottom_y });
    }
    fill
}

fn layers() -> impl Iterator<Item = f64> {
    (1..=EXTRA_POINT_LAYERS).map(|layer| OVG_PADDING * f64::from(layer))
}

/// TALA `addPortConnectionNodesAtBoundaries`: every port gets points outside all cards, straight
/// above, below, left and right of it, where no card is in the way.
fn add_port_connections_at_boundaries(ovg: &mut Ovg, graph: &Graph, tl: Point, br: Point) {
    for layer in layers() {
        for card in 0..graph.nodes.len() {
            let ports = ovg.ports[card].clone();
            let mut added = false;
            let mut seen: Vec<PointId> = Vec::new();
            for &port in &ports {
                if seen.contains(&port) {
                    continue;
                }
                seen.push(port);
                let p = ovg.point(port);
                for side in ovg.points[port].sides_for(card).each() {
                    let boundary = [
                        Point { x: p.x, y: tl.y - layer },
                        Point { x: p.x, y: br.y + layer },
                        Point { x: tl.x - layer, y: p.y },
                        Point { x: br.x + layer, y: p.y },
                    ];
                    for point in boundary {
                        let blocked = (0..graph.nodes.len()).any(|rectangle| {
                            ovg.passes_through_allowing_ports(graph, rectangle, p, point, side)
                        });
                        if blocked {
                            continue;
                        }
                        if !point_near(graph, point) {
                            ovg.add_point(point);
                            added = true;
                        }
                    }
                }
            }
            if !added {
                add_padded_ports(ovg, card, &ports, tl, br);
            }
        }
    }
}

/// TALA `createPortsConnectionsToBoundaries`: a card with no free line to the outside gets a
/// point in front of each port, and points outside all cards in line with it.
fn add_padded_ports(ovg: &mut Ovg, card: usize, ports: &[PointId], tl: Point, br: Point) {
    let mut seen: Vec<PointId> = Vec::new();
    for &port in ports {
        if seen.contains(&port) {
            continue;
        }
        seen.push(port);
        let p = ovg.point(port);
        for side in ovg.points[port].sides_for(card).each() {
            let Some(side) = side else { continue };
            let padded = match side {
                Side::Top => Point { x: p.x, y: p.y - MIN_PORT_CLEARANCE },
                Side::Bottom => Point { x: p.x, y: p.y + MIN_PORT_CLEARANCE },
                Side::Left => Point { x: p.x - MIN_PORT_CLEARANCE, y: p.y },
                Side::Right => Point { x: p.x + MIN_PORT_CLEARANCE, y: p.y },
            };
            ovg.add_point(padded);
            for layer in layers() {
                match side {
                    Side::Top | Side::Bottom => {
                        ovg.add_point(Point { x: tl.x - layer, y: padded.y });
                        ovg.add_point(Point { x: br.x + layer, y: padded.y });
                    }
                    Side::Left | Side::Right => {
                        ovg.add_point(Point { x: padded.x, y: tl.y - layer });
                        ovg.add_point(Point { x: padded.x, y: br.y + layer });
                    }
                }
            }
        }
    }
}

/// TALA `mergePorts`: the ports of a hierarchy's graph become points of this graph, with what
/// they are for their cards.
fn merge_ports(ovg: &mut Ovg, lane: &Ovg) {
    for (card, ports) in lane.ports.iter().enumerate().filter(|(_, ports)| !ports.is_empty()) {
        let mut merged = Vec::with_capacity(ports.len());
        for &port in ports {
            let id = ovg.add_point(lane.point(port));
            for (owner, role) in lane.points[port].port_roles.clone() {
                ovg.points[id].add_role(owner, role);
            }
            merged.push(id);
        }
        ovg.ports[card] = merged;
    }
}

/// TALA `mergeNonPorts`: the points of a hierarchy's graph that are no ports. (TALA leaves
/// the ports out here. They are points of this graph already: taking them again adds none.)
fn merge_other_points(ovg: &mut Ovg, lane: &Ovg) {
    for &id in &lane.nodes {
        ovg.add_point(lane.point(id));
    }
}

/// TALA `ovgBoundingBox`: the box around all points of a graph.
pub(crate) fn points_box(ovg: &Ovg) -> (Point, Point) {
    let mut tl = Point { x: f64::INFINITY, y: f64::INFINITY };
    let mut br = Point { x: f64::NEG_INFINITY, y: f64::NEG_INFINITY };
    for &id in &ovg.nodes {
        let at = ovg.point(id);
        tl = Point { x: tl.x.min(at.x), y: tl.y.min(at.y) };
        br = Point { x: br.x.max(at.x), y: br.y.max(at.y) };
    }
    (tl, br)
}

/// TALA `addNewBoundaryLayers`: every point on the outline of a box gets three copies further
/// out, 20 apart, where no card of the group is near. (For the graph of all cards the points
/// on the outline are ports, and the next step adds the same points for every port: there
/// the step is left out. A hierarchy's own graph has other points on its outline.)
pub(crate) fn add_boundary_layers(ovg: &mut Ovg, graph: &Graph, tl: Point, br: Point) {
    for layer in layers() {
        for id in ovg.nodes.clone() {
            let at = ovg.point(id);
            // A point on two sides of the outline goes out on the later one: top, bottom,
            // left, right.
            let mut out = None;
            if at.y == tl.y {
                out = Some(Point { x: at.x, y: at.y - layer });
            }
            if at.y == br.y {
                out = Some(Point { x: at.x, y: at.y + layer });
            }
            if at.x == tl.x {
                out = Some(Point { x: at.x - layer, y: at.y });
            }
            if at.x == br.x {
                out = Some(Point { x: at.x + layer, y: at.y });
            }
            if let Some(point) = out.filter(|point| !point_near(graph, *point)) {
                ovg.add_point(point);
            }
        }
    }
}

/// TALA `addCornerNodes`.
pub(crate) fn add_corners(ovg: &mut Ovg, graph: &Graph, tl: Point, br: Point) {
    for layer in layers() {
        let corners = [
            Point { x: tl.x - layer, y: tl.y - layer },
            Point { x: br.x + layer, y: tl.y - layer },
            Point { x: br.x + layer, y: br.y + layer },
            Point { x: tl.x - layer, y: br.y + layer },
        ];
        for point in corners {
            if !point_near(graph, point) {
                ovg.add_point(point);
            }
        }
    }
}

/// TALA `addTunnels`: both ends of a tunnel become ports of their cards, joined by an edge.
fn add_tunnels(ovg: &mut Ovg, graph: &Graph) {
    for tunnel in build_tunnels(graph) {
        let mut ends = [0, 0];
        for (end, (card, point)) in [tunnel.a, tunnel.b].into_iter().enumerate() {
            let id = ovg.add_point(point);
            ovg.points[id].add_role(card, PortRole::default());
            ovg.points[id].is_tunnel = true;
            ovg.ports[card].push(id);
            ends[end] = id;
        }
        ovg.connect(ends[0], ends[1]);
    }
}

/// TALA `connectNodes`: every point is joined to the next one on its row and on its column, when
/// no card lies between them.
fn connect_points(ovg: &mut Ovg, graph: &Graph) {
    let mut horizontal: BTreeMap<Coord, Vec<PointId>> = BTreeMap::new();
    let mut vertical: BTreeMap<Coord, Vec<PointId>> = BTreeMap::new();
    for &id in &ovg.nodes {
        let node = &ovg.points[id];
        // Tunnel ends connect only to each other and to their card's centre.
        if node.is_tunnel {
            continue;
        }
        horizontal.entry(Coord::of(node.point.y)).or_default().push(id);
        vertical.entry(Coord::of(node.point.x)).or_default().push(id);
    }
    for (y, line) in horizontal {
        connect_line(ovg, graph, line, y.value(), true);
    }
    for (x, line) in vertical {
        connect_line(ovg, graph, line, x.value(), false);
    }
}

/// TALA `connectNodesOnSameLine`.
fn connect_line(ovg: &mut Ovg, graph: &Graph, mut line: Vec<PointId>, position: f64, horizontal: bool) {
    let mut candidates: Vec<usize> = (0..graph.nodes.len())
        .filter(|n| {
            let node = &graph.nodes[*n];
            if horizontal {
                node.top_left.y <= position && node.bottom() >= position
            } else {
                node.top_left.x <= position && node.right() >= position
            }
        })
        .collect();
    let start_of = |n: usize| {
        if horizontal { graph.nodes[n].top_left.x } else { graph.nodes[n].top_left.y }
    };
    candidates.sort_by(|a, b| start_of(*a).total_cmp(&start_of(*b)));
    let along = |ovg: &Ovg, id: PointId| if horizontal { ovg.point(id).x } else { ovg.point(id).y };
    line.sort_by(|a, b| along(ovg, *a).total_cmp(&along(ovg, *b)));

    let mut search_from = 0;
    for pair in line.windows(2) {
        let (node, closest) = (pair[0], pair[1]);
        if is_misdirected_port_pair(ovg, node, closest, horizontal) {
            continue;
        }
        let mut should_connect = true;
        for (j, &rectangle) in candidates.iter().enumerate().skip(search_from) {
            if start_of(rectangle) > along(ovg, closest) {
                // The cards are sorted: no later one can lie between the two points.
                break;
            }
            let passes = |from: PointId, to: PointId| {
                ovg.points[from].sides_for(rectangle).each().into_iter().all(|side| {
                    ovg.passes_through_allowing_ports(
                        graph,
                        rectangle,
                        ovg.point(from),
                        ovg.point(to),
                        side,
                    )
                })
            };
            if passes(node, closest) && passes(closest, node) {
                should_connect = false;
                break;
            }
            search_from = j;
        }
        if should_connect {
            ovg.connect(node, closest);
        }
    }
}

/// TALA `isMisdirectedPortPair`: two ports of different cards connect only when they look at each
/// other. `first` is left of or above `second`.
fn is_misdirected_port_pair(ovg: &Ovg, first: PointId, second: PointId, horizontal: bool) -> bool {
    let first_roles = &ovg.points[first].port_roles;
    let second_roles = &ovg.points[second].port_roles;
    if first_roles.is_empty() || second_roles.is_empty() {
        return false;
    }
    for (first_owner, first_role) in first_roles {
        for (second_owner, second_role) in second_roles {
            if first_owner == second_owner {
                return false;
            }
            let facing = if horizontal {
                first_role.sides.has(Side::Right) && second_role.sides.has(Side::Left)
            } else {
                first_role.sides.has(Side::Bottom) && second_role.sides.has(Side::Top)
            };
            if facing {
                return false;
            }
        }
    }
    true
}

/// TALA `connectPortsToCenter`.
fn connect_ports_to_centers(ovg: &mut Ovg, graph: &Graph) {
    for (card, node) in graph.nodes.iter().enumerate() {
        let mut center = OvgNode::new(node.center());
        center.is_node_center = true;
        let center = ovg.add_unchecked(center);
        ovg.centers.push(center);
        for port in ovg.ports[card].clone() {
            ovg.connect(center, port);
        }
    }
}

/// TALA `removeIsolatedNodes`.
fn remove_isolated(ovg: &mut Ovg) {
    let points = &ovg.points;
    ovg.nodes.retain(|id| !points[*id].edges.is_empty());
}

/// TALA `flagNodesNearPorts`: marks the points that sit on a port's row or column and closer to it
/// than the threshold; the search costs them extra.
fn flag_points_near_ports(ovg: &mut Ovg, graph: &Graph) {
    for owner in 0..graph.nodes.len() {
        for port in ovg.ports[owner].clone() {
            let origin = ovg.point(port);
            let aligned = |ovg: &Ovg, id: PointId| -> Vec<PointId> {
                ovg.points[id]
                    .edges
                    .iter()
                    .map(|edge| ovg.adjacent(id, *edge))
                    .filter(|adjacent| {
                        let p = ovg.point(*adjacent);
                        p.x == origin.x || p.y == origin.y
                    })
                    .collect()
            };
            let mut seen = vec![port];
            let mut queue: VecDeque<PointId> = aligned(ovg, port).into();
            while let Some(current) = queue.pop_front() {
                if seen.contains(&current) {
                    continue;
                }
                seen.push(current);
                if !ovg.points[current].is_port_of(owner) {
                    let p = ovg.point(current);
                    if (p.y - origin.y).abs() >= NODE_PROXIMITY_THRESHOLD
                        || (p.x - origin.x).abs() >= NODE_PROXIMITY_THRESHOLD
                    {
                        continue;
                    }
                    let near = &mut ovg.points[current].near_port;
                    if let Err(place) = near.binary_search(&owner) {
                        near.insert(place, owner);
                    }
                }
                queue.extend(aligned(ovg, current));
            }
        }
    }
}
