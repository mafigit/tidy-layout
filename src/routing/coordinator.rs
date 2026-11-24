// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.

//! Routing all connections of a layout: three orders of taking them, the cheapest result is kept.
//! Ported from TALA `internal/routing`: `coordinator.go` (`routeAdditionalEdgesWithResourceLimits`,
//! `routeEdgesWithResourceGuards` for plain cards and trees, `finalizeSelectedRoutes`,
//! `reorderSelectedRoutes`), `route.go` (`canSwapEdgesGuarded`), `ovg_edge_router.go` (`sortEdges`, `generateRoutes`), `route.go`
//! (`createSegmentEndpoints`) and `internal/layoutgraph/node.go` (`distance`,
//! `distanceBetweenCenters`). TALA runs the three orders at the same time; here they run one
//! after another, which gives the same routes.

use crate::dejitter::dejitter;
use crate::gosort::sort_slice;
use crate::pack::pack_routed;
use crate::graph::{Edge, Graph, Node, Point, RoutedCluster, TreeLink, compute_cell_size};
use crate::routing::geometry::{Orientation, is_diagonal};
use crate::routing::balance::balance;
use crate::routing::cluster_branching::fix_cluster_branching;
use crate::routing::crosshatch::crosshatch;
use crate::routing::labels::{LabelSize, Placed, label_top_left, place_labels, reorder_duplicates};
use crate::routing::line::straighten;
use crate::routing::nudge::nudge;
use crate::routing::ovg::{self, Ovg};
use crate::routing::router::{Route, RouteError, Router};
use crate::routing::shortcut::shortcut;
use crate::routing::simplify::simplify;
use crate::routing::swap_ports::swap_ports;
use crate::routing::tree_routes::tree_routes;
use crate::routing::tuning::PRECISION;

/// The order in which the connections are routed. TALA `RouteGenerationFlavor`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Flavor {
    ShortestToLongest,
    LongestToShortest,
    /// As they are listed.
    Default,
    /// From the top, and from the left where two start as high: the one order a hierarchy is
    /// routed in.
    TopDownLeftRight,
}

pub const FLAVORS: [Flavor; 3] = [Flavor::ShortestToLongest, Flavor::LongestToShortest, Flavor::Default];

/// TALA `Edge.EuclideanDistance`: the gap between two cards, plus a little for how far their
/// centres are out of line.
pub(crate) fn length(a: &Node, b: &Node) -> f64 {
    let x = ((a.top_left.x + a.width / 2.0) - (b.top_left.x + b.width / 2.0)).abs() / (a.width + b.width);
    let y = ((a.top_left.y + a.height / 2.0) - (b.top_left.y + b.height / 2.0)).abs() / (a.height + b.height);
    a.distance_to(b) + 1.0 / 20.0 * x.min(y)
}

/// TALA `sortEdges`: the connections in the order of a flavor, those of cards of clusters
/// first; equal ones keep their order.
pub fn order_of(flavor: Flavor, graph: &Graph) -> Vec<usize> {
    let length_of = |edge: usize| {
        let edge = graph.edges[edge];
        length(&graph.nodes[edge.from], &graph.nodes[edge.to])
    };
    let mut order: Vec<usize> = (0..graph.edges.len()).collect();
    match flavor {
        Flavor::ShortestToLongest => order.sort_by(|a, b| length_of(*a).total_cmp(&length_of(*b))),
        Flavor::LongestToShortest => order.sort_by(|a, b| length_of(*b).total_cmp(&length_of(*a))),
        Flavor::Default => {}
        Flavor::TopDownLeftRight => {
            // Each connection seen from its upper card to its lower one.
            let ends = |edge: usize| {
                let edge = graph.edges[edge];
                if graph.nodes[edge.from].top_left.y > graph.nodes[edge.to].top_left.y { (edge.to, edge.from) } else { (edge.from, edge.to) }
            };
            let by = |a: f64, b: f64| a.partial_cmp(&b).unwrap_or(std::cmp::Ordering::Equal);
            order.sort_by(|a, b| {
                let ((a_from, a_to), (b_from, b_to)) = (ends(*a), ends(*b));
                let at = |card: usize| graph.nodes[card].top_left;
                if at(a_from).y != at(b_from).y {
                    by(at(a_from).y, at(b_from).y)
                } else if a_from != b_from {
                    by(at(a_from).x, at(b_from).x)
                } else if at(a_to).y == at(b_to).y {
                    // From one card: the target further left first, of targets as high.
                    by(at(a_to).x, at(b_to).x)
                } else {
                    by(at(a_to).y, at(b_to).y)
                }
            });
        }
    }
    // Connections of a card of a cluster go first: "looks best".
    let clustered = |edge: usize| graph.cluster_of(graph.edges[edge].from).is_some() || graph.cluster_of(graph.edges[edge].to).is_some();
    order.sort_by_key(|edge| !clustered(*edge));
    order
}

/// What one order of routing gave. TALA `GenerateRouteResponse`.
pub struct Generated {
    /// In the order they were found.
    pub routes: Vec<Route>,
    /// The sum of the routes' prices.
    pub distance: f64,
}

/// TALA `generateRoutes`: each connection in turn, by rule where the cards lie diagonally and a
/// route with one or two bends is free, by search otherwise.
pub fn generate_routes(graph: &Graph, ovg: &Ovg, flavor: Flavor) -> Result<Generated, RouteError> {
    generate_more(graph, ovg, flavor, &[])
}

/// The same with routes that are laid already (the tree routes): they come first and are in
/// the way of the others, and their connections are not routed again. Their prices do not
/// count.
fn generate_more(graph: &Graph, ovg: &Ovg, flavor: Flavor, existing: &[Route]) -> Result<Generated, RouteError> {
    let mut router = Router::new(graph, ovg);
    for route in existing {
        router.add_route(route.clone())?;
    }
    let mut distance = 0.0;
    for edge in order_of(flavor, graph) {
        if existing.iter().any(|route| route.edge == edge) {
            continue;
        }
        let (route, price) = match router.slingshot(edge) {
            Some((nodes, price)) => (Route::through(ovg, edge, nodes), price),
            None => match router.search(edge) {
                // A connection without a way through the graph becomes a straight line.
                Err(RouteError::NoPath) => router.line_instead(edge),
                found => found?,
            },
        };
        distance += price;
        router.add_route(route)?;
    }
    Ok(Generated { routes: router.routes, distance })
}

/// TALA `routeAdditionalEdgesWithResourceLimits` for a layout without routes: per connection the
/// ports and bends of its route, from the order of routing that came out cheapest.
pub fn route_all(graph: &Graph) -> Result<Vec<Vec<Point>>, RouteError> {
    let ovg = ovg::build(graph);
    let mut best: Option<Generated> = None;
    let mut least = f64::INFINITY;
    for flavor in FLAVORS {
        let generated = generate_routes(graph, &ovg, flavor)?;
        // TALA `PrecisionCompare(...) < 0`: cheaper by more than a rounding error.
        if (generated.distance - least).abs() >= PRECISION && generated.distance < least {
            least = generated.distance;
            best = Some(generated);
        }
    }
    let mut kept = vec![Vec::new(); graph.edges.len()];
    for route in best.iter().flat_map(|generated| &generated.routes) {
        kept[route.edge] = route.segment_endpoints();
    }
    Ok(kept)
}

/// TALA `routeEdgesWithResourceGuards` for plain cards and trees, as the stage `EdgeRouting`
/// of the pipeline calls it: the connections of trees get their routes by rule first, the
/// others the routes of [`route_all`] around those, and the routes of connections between
/// the same two cards are put in the order of the connections.
pub fn route_group(graph: &Graph) -> Result<Vec<Vec<Point>>, RouteError> {
    let ovg = ovg::build(graph);
    // The connections of trees are routed first, by rule.
    let trees = tree_routes(graph, &ovg);
    let mut best: Option<Generated> = None;
    let mut least = f64::INFINITY;
    // A group whose first card is in a hierarchy is routed in one order only.
    let layered = graph.nodes.len() > graph.nearby && graph.level(graph.nearby).is_some();
    let flavors: &[Flavor] = if layered { &[Flavor::TopDownLeftRight] } else { &FLAVORS };
    for &flavor in flavors {
        let generated = generate_more(graph, &ovg, flavor, &trees)?;
        if (generated.distance - least).abs() >= PRECISION && generated.distance < least {
            least = generated.distance;
            best = Some(generated);
        }
    }
    let mut routes = best.map(|generated| generated.routes).unwrap_or_default();
    reorder_parallel(graph, &mut routes);
    let mut kept = vec![Vec::new(); graph.edges.len()];
    for route in &routes {
        kept[route.edge] = route.segment_endpoints();
    }
    Ok(kept)
}

/// TALA `overshootAmount`: how far around a group cards of other groups are in the way.
const OVERSHOOT: f64 = 100.0;

/// TALA `RouteGraph` for plain cards and trees without routes, the stage `EdgeRouting` of the pipeline:
/// every group of connected cards is routed alone ([`route_group`]), with its own cell size.
/// Cards of other groups that reach into the box around the group, 100 wider on every side,
/// are in the way.
pub fn route_graph(graph: &Graph) -> Result<Vec<Vec<Point>>, RouteError> {
    route_graph_again(graph, &[], &[])
}

/// [`route_graph`] once more, where the connections have routes already (the second
/// `EdgeRouting`, after `Dejitter`). TALA keeps the old routes on the connections until each
/// group is routed anew, and the box around a group (`routeStageGraphBoundingBox`) takes in
/// the points of its routes, and the box of every label that has a place (the tree stage's
/// presets) where it lies on them. So the box can be larger than the cards, and more cards of
/// other groups be in the way. `previous` is empty for the first routing; `labels` has the
/// size of each connection's label, if any.
pub fn route_graph_again(graph: &Graph, previous: &[Vec<Point>], labels: &[Option<LabelSize>]) -> Result<Vec<Vec<Point>>, RouteError> {
    // TALA `splitSubgraphsWithOwnership`: the groups in the order of their first cards, the
    // cards of a group nearest first, its connections in the order of the graph.
    let mut group_of: Vec<Option<usize>> = vec![None; graph.nodes.len()];
    let mut groups: Vec<Vec<usize>> = Vec::new();
    for start in 0..graph.nodes.len() {
        if group_of[start].is_some() {
            continue;
        }
        let mut found = vec![start];
        group_of[start] = Some(groups.len());
        let mut next = 0;
        while next < found.len() {
            let current = found[next];
            next += 1;
            for edge in graph.edges_of(current) {
                let other = edge.other(current);
                if group_of[other].is_none() {
                    group_of[other] = Some(groups.len());
                    found.push(other);
                }
            }
        }
        groups.push(found);
    }
    let mut routes = vec![Vec::new(); graph.edges.len()];
    for (number, group) in groups.iter().enumerate() {
        let edges: Vec<usize> = (0..graph.edges.len()).filter(|edge| group_of[graph.edges[*edge].from] == Some(number)).collect();
        if edges.is_empty() {
            continue;
        }
        let mut top_left = Point { x: f64::INFINITY, y: f64::INFINITY };
        let mut bottom_right = Point { x: f64::NEG_INFINITY, y: f64::NEG_INFINITY };
        for &card in group {
            let node = &graph.nodes[card];
            top_left = Point { x: top_left.x.min(node.top_left.x), y: top_left.y.min(node.top_left.y) };
            bottom_right = Point { x: bottom_right.x.max(node.right().round()), y: bottom_right.y.max(node.bottom().round()) };
        }
        // The routes the connections have from before, and their labels with a place.
        let (mut min, mut max) = (top_left, bottom_right);
        for &edge in &edges {
            let Some(route) = previous.get(edge).filter(|route| !route.is_empty()) else { continue };
            for point in route {
                min = Point { x: min.x.min(point.x), y: min.y.min(point.y) };
                max = Point { x: max.x.max(point.x), y: max.y.max(point.y) };
            }
            if let (Some(Some((position, share))), Some(Some(size))) = (graph.label_presets.get(edge), labels.get(edge)) {
                let corner = label_top_left(route, *position, *share, *size);
                min = Point { x: min.x.min(corner.x), y: min.y.min(corner.y) };
                max = Point { x: max.x.max(corner.x + size.width), y: max.y.max(corner.y + size.height) };
            }
        }
        // TALA rounds the box as a whole.
        let (top_left, bottom_right) = (Point { x: min.x.round(), y: min.y.round() }, Point { x: max.x.round(), y: max.y.round() });
        let (top_left, bottom_right) = (Point { x: top_left.x - OVERSHOOT, y: top_left.y - OVERSHOOT }, Point { x: bottom_right.x + OVERSHOOT, y: bottom_right.y + OVERSHOOT });
        // A card that covers a card of the group is not in the way: it may sit on a middle.
        let covers = |a: &Node, b: &Node| a.top_left.x < b.right() && b.top_left.x < a.right() && a.top_left.y < b.bottom() && b.top_left.y < a.bottom();
        let nearby_cards: Vec<usize> = groups
            .iter()
            .enumerate()
            .filter(|(other, _)| *other != number)
            .flat_map(|(_, cards)| cards)
            .copied()
            .filter(|card| {
                let node = &graph.nodes[*card];
                node.top_left.x <= bottom_right.x
                    && node.top_left.y <= bottom_right.y
                    && node.right() >= top_left.x
                    && node.bottom() >= top_left.y
                    && !group.iter().any(|own| covers(node, &graph.nodes[*own]))
            })
            .collect();
        let nearby: Vec<Node> = nearby_cards.iter().map(|card| graph.nodes[*card].clone()).collect();
        // TALA `buildOVGFromGraphWithGuard`: the cards in the way come before the group's own.
        let place_of = |card: usize| nearby.len() + group.iter().position(|other| *other == card).expect("a card of the group");
        let own: Vec<Node> = group.iter().map(|card| graph.nodes[*card].clone()).collect();
        let cell_size = compute_cell_size(&own);
        let nodes: Vec<Node> = nearby.iter().cloned().chain(own).collect();
        let part_edges = edges.iter().map(|edge| Edge { from: place_of(graph.edges[*edge].from), to: place_of(graph.edges[*edge].to), ..graph.edges[*edge] }).collect();
        // What the graph knows of trees goes with the group: every card's own list of
        // connections, the connection a tree card hangs by, and which are tree connections.
        let part_edge = |edge: usize| edges.iter().position(|other| *other == edge).expect("a connection of the group");
        let own_edges: Vec<Vec<usize>> = if graph.own_edges.is_empty() {
            Vec::new()
        } else {
            (0..nearby.len()).map(|_| Vec::new()).chain(group.iter().map(|card| graph.own_edges[*card].iter().map(|edge| part_edge(*edge)).collect())).collect()
        };
        let tree_links = (0..nearby.len()).map(|_| None).chain(group.iter().map(|card| graph.tree_link(*card).map(|link| TreeLink { edge: part_edge(link.edge), ..link }))).collect();
        let tree_edges = edges.iter().map(|edge| graph.is_tree_edge(*edge)).collect();
        let levels = (0..nearby.len()).map(|_| None).chain(group.iter().map(|card| graph.level(*card))).collect();
        // The cards keep the numbers they have in the diagram: a rule route breaks a tie by them.
        // (The cards in the way are asked for none.)
        let ids = (0..nearby.len()).map(|_| -1).chain(group.iter().map(|card| graph.id(*card))).collect();
        // The clusters go with it, as far as their cards are in it: a cluster of the group
        // whole, one of another group with its cards that are in the way.
        let part_card = |card: usize| nearby_cards.iter().position(|other| *other == card).or_else(|| group.iter().position(|other| *other == card).map(|place| nearby.len() + place));
        let clusters = graph
            .clusters
            .iter()
            .map(|cluster| RoutedCluster { members: cluster.members.iter().filter_map(|card| part_card(*card)).collect(), ..cluster.clone() })
            .filter(|cluster| !cluster.members.is_empty())
            .collect();
        let part = Graph { cell_size, slanted_lines: graph.slanted_lines, nearby: nearby.len(), ids, own_edges, tree_links, tree_edges, levels, clusters, ..Graph::new(nodes, part_edges) };
        for (route, edge) in route_group(&part)?.into_iter().zip(edges) {
            routes[edge] = route;
        }
    }
    Ok(routes)
}

/// The routing stages of TALA's pipeline for plain cards, trees, hierarchies and clusters, as
/// far as they are ported: the first `EdgeRouting` ([`route_graph`]), `Crosshatch` (for
/// clusters), `SimplifyEdgeRoutes`, `SwapEdgePorts`, `StraightEdgesFallback`,
/// `BalanceEdgeSegments`, `FixClusterEdgeBranching`, `ReorderDuplicates`, `PlaceLabels`,
/// `NudgeEdgeChannels` and `ShortcutEdgeRoutes`.
/// Before `PlaceLabels` runs the second `BinPack`, a placement stage too ([`pack_routed`]).
///
/// Between the first routing and the others runs `Dejitter`, a placement stage: it may move
/// cards of `graph`. Where it does, all connections are routed again (the second
/// `EdgeRouting`). The second `BinPack` may move whole groups, with their routes.
pub fn route_stages(graph: &mut Graph, labels: &[Option<LabelSize>]) -> Result<Staged, RouteError> {
    let mut routes = route_graph(graph)?;
    crosshatch(graph, &mut routes);
    if dejitter(graph, &mut routes) {
        routes = route_graph_again(graph, &routes, labels)?;
    }
    simplify(graph, &mut routes);
    swap_ports(graph, &mut routes);
    straighten(graph, &mut routes);
    balance(graph, &mut routes);
    let balanced = routes.clone();
    fix_cluster_branching(graph, &mut routes);
    let has_label: Vec<bool> = labels.iter().map(Option::is_some).collect();
    reorder_duplicates(graph, &mut routes, &has_label);
    // The second `BinPack`: the groups are packed once more, with their routes.
    pack_routed(graph, &mut routes);
    let placed = place_labels(graph, &routes, labels);
    nudge(graph, &mut routes, &placed, labels);
    shortcut(graph, &mut routes, &placed, labels);
    // A label keeps its place along its route and moves with it.
    let mut at_the_end = placed.clone();
    for ((label, size), route) in at_the_end.iter_mut().zip(labels).zip(&routes) {
        if let (Some(label), Some(size)) = (label, size) {
            label.top_left = label_top_left(route, label.position, label.percentage, *size);
        }
    }
    Ok(Staged { routing: Routing { routes, labels: at_the_end }, balanced, placed })
}

/// What [`route_stages`] gives.
pub struct Staged {
    /// The routes at the end, and the labels where they are then.
    pub routing: Routing,
    /// The routes after `BalanceEdgeSegments`.
    pub balanced: Vec<Vec<Point>>,
    /// The labels right after `PlaceLabels`.
    pub placed: Vec<Option<Placed>>,
}

/// TALA `canSwapEdgesGuarded`: the two routes join the same two cards, which face each other,
/// leave and reach them on the same sides where they run the same way, and share no port with
/// a third route.
fn can_swap(graph: &Graph, routes: &[Route], first: usize, second: usize) -> bool {
    let (a, b) = (&routes[first], &routes[second]);
    let (edge, other) = (&graph.edges[a.edge], &graph.edges[b.edge]);
    // TALA also refuses a connection from a card to itself. The engine is given none.
    let same_way = edge.from == other.from && edge.to == other.to;
    if !same_way && !(edge.from == other.to && edge.to == other.from) {
        return false;
    }
    let side = graph.nodes[edge.from].orientation(&graph.nodes[edge.to]);
    if is_diagonal(side) {
        return false;
    }
    if same_way {
        if matches!(side, Orientation::Top | Orientation::Bottom) && (a.from_port.y != b.from_port.y || a.to_port.y != b.to_port.y) {
            return false;
        }
        if matches!(side, Orientation::Left | Orientation::Right) && (a.from_port.x != b.from_port.x || a.to_port.x != b.to_port.x) {
            return false;
        }
    }
    routes.iter().enumerate().filter(|(third, _)| *third != first && *third != second).all(|(_, third)| {
        [a.from_port, a.to_port, b.from_port, b.to_port].iter().all(|port| *port != third.from_port && *port != third.to_port)
    })
}

/// TALA `reorderSelectedRoutes`: routes that can swap are given to their connections anew, from
/// left to right or from top to bottom in the order of the connections.
fn reorder_parallel(graph: &Graph, routes: &mut [Route]) {
    let mut grouped = vec![false; routes.len()];
    let mut buckets: Vec<Vec<usize>> = Vec::new();
    for index in 0..routes.len() {
        if grouped[index] {
            continue;
        }
        grouped[index] = true;
        let mut bucket = vec![index];
        let free: Vec<usize> = (index + 1..routes.len()).filter(|other| !grouped[*other]).collect();
        for other in free {
            if can_swap(graph, routes, index, other) {
                bucket.push(other);
                grouped[other] = true;
            }
        }
        if bucket.len() > 1 {
            buckets.push(bucket);
        }
    }
    for mut bucket in buckets {
        // TALA sorts with Go's `sort.Slice`, and asks the first of two routes which way the
        // cards face each other.
        let ends = |route: &Route| (graph.edges[route.edge].from, graph.edges[route.edge].to);
        sort_slice(&mut bucket, |first, second| {
            let (a, b) = (&routes[*first], &routes[*second]);
            let (from, to) = ends(a);
            let other_port = if from == ends(b).0 { b.from_port } else { b.to_port };
            if matches!(graph.nodes[from].orientation(&graph.nodes[to]), Orientation::Top | Orientation::Bottom) {
                a.from_port.x < other_port.x
            } else {
                a.from_port.y < other_port.y
            }
        });
        let mut edges: Vec<usize> = bucket.iter().map(|route| routes[*route].edge).collect();
        edges.sort_unstable();
        for (route, edge) in bucket.into_iter().zip(edges) {
            let route = &mut routes[route];
            if ends(route).0 != graph.edges[edge].from {
                (route.from_port, route.to_port) = (route.to_port, route.from_port);
                route.nodes.reverse();
                route.points.reverse();
            }
            route.edge = edge;
        }
    }
}

/// TALA `RouteEdges` for a layout without routes, as far as it is ported: the routes of
/// [`route_all`], then a straight line wherever one is cheaper than the route, then the routes
/// spread over the room they have.
pub fn route_edges(graph: &Graph) -> Result<Vec<Vec<Point>>, RouteError> {
    let mut routes = route_all(graph)?;
    straighten(graph, &mut routes);
    balance(graph, &mut routes);
    Ok(routes)
}

/// The routes of all connections and the places of their labels.
pub struct Routing {
    /// Per connection the points of its route, from port to port.
    pub routes: Vec<Vec<Point>>,
    /// Per connection the place of its label, if it has one.
    pub labels: Vec<Option<Placed>>,
}

/// TALA `RouteEdges` for connections of which some carry a label: the routes of [`route_edges`]
/// (doubled connections may swap routes to give a label room), and the place of every label.
pub fn route_edges_with_labels(graph: &Graph, labels: &[Option<LabelSize>]) -> Result<Routing, RouteError> {
    let mut routes = route_edges(graph)?;
    let has_label: Vec<bool> = labels.iter().map(Option::is_some).collect();
    reorder_duplicates(graph, &mut routes, &has_label);
    let labels = place_labels(graph, &routes, labels);
    Ok(Routing { routes, labels })
}
