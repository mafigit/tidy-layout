// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.

//! The stage that packs the groups of a diagram: every group of connected cards is moved, as
//! one piece, to the place beside the groups packed before it where the box around all of them
//! stays smallest and nearest to a square, clear of the routes its connections are likely to
//! take. Ported from TALA `internal/packing`: `binpack.go` (`Pack`, `packGuarded` before
//! routing and for cards in no container, `placementCandidatesGuarded`,
//! `blocksRoutesWithDeltasGuarded`, `estimateRouteSegments`, `estimateEdgeSegments`),
//! `guard.go` (`binPackScoreGuarded`, `binPackHierarchyBoxes`, `binPackPointInHierarchy`,
//! `binPackSmallestDeltas`), `internal/graphbounds/bounds.go` (`BoundingBox`) and
//! `internal/layoutgraph/node.go` (`reachableNodesGuarded` with containers, `overlapsLine`).

use crate::graph::{Arrangement, Graph, Node, Point};
use crate::optimizer::Allowed;
use crate::place::Placed;
use crate::policy::NODE_GAP;
use crate::routing::geometry::{Orientation, is_diagonal, segments_intersect};

/// TALA `subgraphPadding`: the room between two groups.
const SUBGRAPH_PADDING: f64 = 20.0;
/// TALA `subgraphSquareDampener`: how much a box that is not square costs.
const SQUARE_DAMPENER: f64 = 0.5;
/// How far the groups are moved out of the way before they are packed.
const AWAY: f64 = 100_000.0;

/// A card of a group: a card of the graph, or a card of a cluster, which the graph keeps in its
/// vessel. TALA walks from a vessel to its cards, so they are in the group too.
#[derive(Clone, Copy, PartialEq)]
enum Item {
    Card(usize),
    Member(usize, usize),
}

type Segment = (Point, Point);

fn node_of(graph: &Graph, item: Item) -> Node {
    match item {
        Item::Card(card) => graph.nodes[card].clone(),
        Item::Member(vessel, place) => graph.vessels[vessel].member(graph.nodes[graph.vessels[vessel].node].top_left, place),
    }
}

/// TALA `graphbounds.BoundingBox` for cards without labels outside: the box around the cards.
fn bounding_box(graph: &Graph, items: &[Item]) -> (Point, Point) {
    let mut top_left = Point { x: f64::INFINITY, y: f64::INFINITY };
    let mut bottom_right = Point { x: f64::NEG_INFINITY, y: f64::NEG_INFINITY };
    for &item in items {
        let node = node_of(graph, item);
        top_left = Point { x: top_left.x.min(node.top_left.x), y: top_left.y.min(node.top_left.y) };
        bottom_right = Point { x: bottom_right.x.max(node.right().round()), y: bottom_right.y.max(node.bottom().round()) };
    }
    (top_left, bottom_right)
}

/// TALA `binPackScoreGuarded`: the area of the box around the cards, and half the square of
/// what it lacks to be a square.
fn score(graph: &Graph, items: &[Item]) -> f64 {
    let (top_left, bottom_right) = bounding_box(graph, items);
    let (width, height) = (bottom_right.x - top_left.x, bottom_right.y - top_left.y);
    width * height + (width - height).powi(2) * SQUARE_DAMPENER
}

/// TALA `binPackSmallestDeltas`: the smallest width and the smallest height among the groups.
fn smallest_deltas(graph: &Graph, groups: &[Vec<Item>]) -> (f64, f64) {
    groups.iter().fold((f64::INFINITY, f64::INFINITY), |(x, y), group| {
        let (top_left, bottom_right) = bounding_box(graph, group);
        (x.min(bottom_right.x - top_left.x), y.min(bottom_right.y - top_left.y))
    })
}

/// TALA `reachableNodesGuarded` with containers: the card and all that connections lead to from
/// it, nearest first. After the neighbours of a vessel come the cards of its cluster.
fn group_of(graph: &Graph, start: usize) -> Vec<Item> {
    let mut found = vec![Item::Card(start)];
    let mut next = 0;
    while next < found.len() {
        let current = found[next];
        next += 1;
        let Item::Card(card) = current else {
            continue;
        };
        for edge in graph.edges_of(card) {
            let other = Item::Card(edge.other(card));
            if !found.contains(&other) {
                found.push(other);
            }
        }
        if let Some(vessel) = graph.vessels.iter().position(|vessel| vessel.node == card) {
            found.extend((0..graph.vessels[vessel].count).map(|place| Item::Member(vessel, place)));
        }
    }
    found
}

/// TALA `estimateEdgeSegments`: the lines a connection is likely to take. Between cards that
/// face each other one straight line, if it is long enough for a group to fit in. Between cards
/// on a diagonal, and to and from a cluster, both L shapes and the middle lines of both S
/// shapes.
fn edge_segments(graph: &Graph, edge: usize, smallest_x: f64, smallest_y: f64) -> Vec<Segment> {
    let (from, to) = (graph.edges[edge].from, graph.edges[edge].to);
    let side = graph.nodes[from].orientation(&graph.nodes[to]);
    if !is_diagonal(side) && graph.vessel_at(from).is_none() && graph.vessel_at(to).is_none() {
        let (start, end) = (graph.nodes[from].center(), graph.nodes[to].center());
        let long = match side {
            Orientation::Left | Orientation::Right => (start.x - end.x).abs() > smallest_x,
            Orientation::Top | Orientation::Bottom => (start.y - end.y).abs() > smallest_y,
            _ => false,
        };
        return if long { vec![(start, end)] } else { Vec::new() };
    }
    // A connection leaves a cluster at one of its two ends.
    let ends = |card: usize| -> Vec<Point> {
        let node = &graph.nodes[card];
        let center = node.center();
        match graph.vessel_at(card) {
            Some(vessel) if vessel.arrangement == Arrangement::Row => {
                vec![Point { x: node.top_left.x, y: center.y }, Point { x: node.right().round(), y: center.y }]
            }
            Some(_) => vec![Point { x: center.x, y: node.top_left.y }, Point { x: center.x, y: node.bottom().round() }],
            None => vec![center],
        }
    };
    let mut segments = Vec::new();
    for start in ends(from) {
        for end in ends(to) {
            let (down, across) = (Point { x: start.x, y: end.y }, Point { x: end.x, y: start.y });
            let (middle_x, middle_y) = (((start.x + end.x) / 2.0).round(), ((start.y + end.y) / 2.0).round());
            segments.extend([
                (start, down),
                (down, end),
                (start, across),
                (across, end),
                (Point { x: middle_x, y: start.y }, Point { x: middle_x, y: end.y }),
                (Point { x: start.x, y: middle_y }, Point { x: end.x, y: middle_y }),
            ]);
        }
    }
    segments
}

/// TALA `estimateRouteSegments`: the likely lines of all connections of the cards, each
/// connection once.
fn route_segments(graph: &Graph, items: &[Item], smallest_x: f64, smallest_y: f64) -> Vec<Segment> {
    let mut seen: Vec<usize> = Vec::new();
    let mut segments = Vec::new();
    for &item in items {
        let Item::Card(card) = item else {
            continue;
        };
        for (edge, _) in graph.indexed_edges_of(card) {
            if !seen.contains(&edge) {
                seen.push(edge);
                segments.extend(edge_segments(graph, edge, smallest_x, smallest_y));
            }
        }
    }
    segments
}

/// TALA `overlapsLine` with the gap between cards: the line comes nearer to the card than that.
fn overlaps_line(node: &Node, (start, end): Segment) -> bool {
    let (left, right) = (node.top_left.x - NODE_GAP, node.right() + NODE_GAP);
    let (top, bottom) = (node.top_left.y - NODE_GAP, node.bottom() + NODE_GAP);
    let contains = |point: Point| left <= point.x && right >= point.x && top <= point.y && bottom >= point.y;
    if contains(start) || contains(end) {
        return true;
    }
    let corners = [Point { x: left, y: top }, Point { x: right, y: top }, Point { x: right, y: bottom }, Point { x: left, y: bottom }];
    (0..4).any(|corner| segments_intersect(corners[corner], corners[(corner + 1) % 4], start, end))
}

/// TALA `placementCandidatesGuarded`: the corners a group can be put at beside these cards.
/// Beside each card: to its right, below it, and to the right of its lower end, where no
/// connection of it leads that way. And six corners at the box around all of them.
fn placement_candidates(graph: &Graph, items: &[Item], within: bool, routed: bool) -> Vec<Point> {
    let mut candidates = Vec::new();
    for &item in items.iter().filter(|_| within) {
        let node = node_of(graph, item);
        let (mut right, mut below, mut below_right) = (false, false, false);
        // With the routes laid, no corner is left out for a connection that may lead there.
        if let (Item::Card(card), false) = (item, routed) {
            for edge in graph.edges_of(card) {
                match node.orientation(&graph.nodes[edge.other(card)]) {
                    Orientation::Left => right = true,
                    Orientation::Top => below = true,
                    Orientation::TopLeft => below_right = true,
                    _ => {}
                }
            }
        }
        if !right {
            candidates.push(Point { x: node.right() + SUBGRAPH_PADDING, y: node.top_left.y });
        }
        if !below {
            candidates.push(Point { x: node.top_left.x, y: node.bottom() + SUBGRAPH_PADDING });
        }
        if !below_right {
            candidates.push(Point { x: node.right() + SUBGRAPH_PADDING, y: node.bottom() });
        }
    }
    let (top_left, bottom_right) = bounding_box(graph, items);
    candidates.extend([
        Point { x: bottom_right.x, y: top_left.y },
        Point { x: bottom_right.x, y: bottom_right.y },
        Point { x: top_left.x, y: bottom_right.y },
        Point { x: bottom_right.x + SUBGRAPH_PADDING, y: top_left.y },
        Point { x: bottom_right.x + SUBGRAPH_PADDING, y: bottom_right.y },
        Point { x: top_left.x, y: bottom_right.y + SUBGRAPH_PADDING },
    ]);
    candidates
}

/// The cards of the group move by this much. The cards of a cluster move with their vessel.
fn translate(graph: &mut Graph, group: &[Item], x: f64, y: f64) {
    for &item in group {
        if let Item::Card(card) = item {
            graph.nodes[card].top_left.x += x;
            graph.nodes[card].top_left.y += y;
        }
    }
}

/// TALA `routedEdgeSegments`: the upright and level pieces of the routes of a group's
/// connections, as they lie now.
fn routed_segments(graph: &Graph, routes: &[Vec<Point>], group: &[Item]) -> Vec<Segment> {
    let mut edges: Vec<usize> = Vec::new();
    for &item in group {
        if let Item::Card(card) = item {
            for (edge, _) in graph.indexed_edges_of(card) {
                if !edges.contains(&edge) {
                    edges.push(edge);
                }
            }
        }
    }
    let pieces = |level: bool| {
        let edges = &edges;
        (0..edges.len()).flat_map(move |index| routes[edges[index]].windows(2).filter(move |piece| if level { piece[0].y == piece[1].y } else { piece[0].x == piece[1].x }).map(move |piece| {
            let first_is_start = if level { piece[0].x < piece[1].x } else { piece[0].y < piece[1].y };
            if first_is_start { (piece[0], piece[1]) } else { (piece[1], piece[0]) }
        }))
    };
    pieces(true).chain(pieces(false)).collect()
}

/// The routes of a group's connections move with its cards: every route by as much as the
/// first of its cards in the group is away from where it was.
fn move_routes(graph: &Graph, routes: &mut [Vec<Point>], group: &[Item], original: &[Node]) {
    let mut moved: Vec<usize> = Vec::new();
    for &item in group {
        if let Item::Card(card) = item {
            let (x, y) = (graph.nodes[card].top_left.x - original[card].top_left.x, graph.nodes[card].top_left.y - original[card].top_left.y);
            let own: Vec<usize> = graph.indexed_edges_of(card).map(|(edge, _)| edge).collect();
            for edge in own {
                if !moved.contains(&edge) {
                    moved.push(edge);
                    for point in &mut routes[edge] {
                        *point = Point { x: point.x + x, y: point.y + y };
                    }
                }
            }
        }
    }
}

/// TALA `Pack` after routing (the second `BinPack` of the pipeline), for plain cards and trees: as
/// [`pack`], but the routes are laid. A group's routes move with it, a corner beside a card
/// is tried whatever the card's connections, and what a group may not cover of the groups
/// packed before it are their routes as they lie, not the routes they are likely to get.
pub fn pack_routed(graph: &mut Graph, routes: &mut [Vec<Point>]) {
    // A group that is a hierarchy stays where it is, as before routing.
    let layered: Vec<bool> = (0..graph.nodes.len()).map(|card| graph.level(card).is_some()).collect();
    pack_groups(graph, &layered, Some(routes));
}

/// TALA `Pack` before routing: the groups of connected cards are packed, in the order of the
/// diagram. A group with a hierarchy stays. The first group stays too, if no group with a
/// hierarchy does. Every other group goes to the best of the corners beside the groups packed
/// so far, at which it comes too close to no card and lies on no likely route, and no likely
/// route of its own runs over a card. Where a group finds no corner, or the box around the
/// diagram got worse, all cards go back.
pub fn pack(placed: &mut Placed) {
    pack_groups(&mut placed.graph, &placed.layered, None);
}

/// TALA `packGuarded` for cards in no container, before routing (`routes` is `None`) or after.
fn pack_groups(graph: &mut Graph, layered: &[bool], mut routes: Option<&mut [Vec<Point>]>) {
    let routed = routes.is_some();
    let original_routes: Option<Vec<Vec<Point>>> = routes.as_ref().map(|routes| routes.to_vec());
    let is_layered = |item: Item| matches!(item, Item::Card(card) if layered[card]);
    let top_level: Vec<usize> = graph.cards_in_the_way().collect();
    let mut added = vec![false; graph.nodes.len()];
    let mut to_pack: Vec<Vec<Item>> = Vec::new();
    let mut packed: Vec<Vec<Item>> = Vec::new();
    for pass in 0..2 {
        for &card in &top_level {
            if added[card] {
                continue;
            }
            let group = group_of(graph, card);
            // First the groups that can move, then those with a hierarchy.
            if pass == 0 && group.iter().any(|item| is_layered(*item)) {
                continue;
            }
            for item in &group {
                if let Item::Card(card) = item {
                    added[*card] = true;
                }
            }
            if pass == 0 { to_pack.push(group) } else { packed.push(group) }
        }
    }
    if to_pack.is_empty() {
        return;
    }
    let everything: Vec<Item> = top_level.iter().map(|card| Item::Card(*card)).collect();
    let root = bounding_box(graph, &to_pack[0]).0;
    let original_score = score(graph, &everything);
    let original = graph.nodes.clone();
    let vessels = graph.vessels.clone();
    let original_of = |item: Item| match item {
        Item::Card(card) => original[card].top_left,
        Item::Member(vessel, place) => vessels[vessel].member(original[vessels[vessel].node].top_left, place).top_left,
    };
    // Out of the way. Without a group that stays, the first one comes back to where it was.
    for group in &to_pack {
        translate(graph, group, AWAY, AWAY);
    }
    if packed.is_empty() {
        let first = to_pack.remove(0);
        let away = bounding_box(graph, &first).0;
        translate(graph, &first, root.x - away.x, root.y - away.y);
        if let Some(routes) = routes.as_deref_mut() {
            move_routes(graph, routes, &first, &original);
        }
        packed.push(first);
    }
    let mut packed_cards: Vec<Item> = Vec::new();
    let mut packed_segments: Vec<Segment> = Vec::new();
    let (smallest_x, smallest_y) = smallest_deltas(graph, &to_pack);
    for group in &packed {
        packed_cards.extend(group);
        match routes.as_deref() {
            Some(routes) => packed_segments.extend(routed_segments(graph, routes, group)),
            None => packed_segments.extend(route_segments(graph, group, smallest_x, smallest_y)),
        }
    }
    let mut occupied: Vec<Point> = Vec::new();
    while !to_pack.is_empty() {
        // No group goes into the box around a hierarchy.
        let hierarchy_boxes: Vec<(Point, Point)> = packed.iter().filter(|group| is_layered(group[0])).map(|group| bounding_box(graph, group)).collect();
        let in_hierarchy = |point: Point| hierarchy_boxes.iter().any(|(top_left, bottom_right)| top_left.x <= point.x && point.x <= bottom_right.x && top_left.y <= point.y && point.y <= bottom_right.y);
        let mut candidates = vec![root];
        let mut packed_top_left = Point { x: f64::INFINITY, y: f64::INFINITY };
        for group in &packed {
            let top_left = bounding_box(graph, group).0;
            packed_top_left = Point { x: packed_top_left.x.min(top_left.x), y: packed_top_left.y.min(top_left.y) };
            for point in placement_candidates(graph, group, !is_layered(group[0]), routed) {
                if !occupied.contains(&point) && !in_hierarchy(point) && !candidates.contains(&point) {
                    candidates.push(point);
                }
            }
        }
        let current = to_pack.remove(0);
        let (away, far_corner) = bounding_box(graph, &current);
        let (width, height) = (far_corner.x - away.x, far_corner.y - away.y);
        // Above, left of and above left of all that is packed. And where each of its cards was.
        let mut known = candidates.clone();
        candidates.extend([
            Point { x: packed_top_left.x, y: packed_top_left.y - height - SUBGRAPH_PADDING },
            Point { x: packed_top_left.x - width - SUBGRAPH_PADDING, y: packed_top_left.y },
            Point { x: packed_top_left.x - width - SUBGRAPH_PADDING, y: packed_top_left.y - height - SUBGRAPH_PADDING },
        ]);
        for &item in &current {
            let was = original_of(item);
            if !known.contains(&was) {
                known.push(was);
                candidates.push(was);
            }
        }
        let with_current: Vec<Item> = packed_cards.iter().chain(&current).copied().collect();
        let (packed_x, packed_y) = smallest_deltas(graph, &packed);
        // TALA's transaction puts the cards of every cluster in order here too. They are in
        // order since the gap stage.
        let allowed = Allowed::from(graph);
        let places = graph.nodes.clone();
        let mut best: Option<(f64, Point)> = None;
        for &point in &candidates {
            translate(graph, &current, point.x - away.x, point.y - away.y);
            let fits = allowed.now(graph)
                && (routed
                    || !current.iter().any(|item| {
                        let node = node_of(graph, *item);
                        in_hierarchy(node.top_left) || in_hierarchy(Point { x: node.right().round(), y: node.bottom().round() })
                    }))
                && !blocks_routes(graph, &current, &packed, &packed_segments, packed_x, packed_y);
            if fits {
                let scored = score(graph, &with_current);
                if best.is_none_or(|(least, _)| scored < least) {
                    best = Some((scored, point));
                }
            }
            graph.nodes = places.clone();
        }
        // Without a corner for a group the packing ends. (TALA puts all cards back then. The
        // check at the end does that: the groups not packed are still far away.)
        let Some((_, point)) = best else {
            break;
        };
        translate(graph, &current, point.x - away.x, point.y - away.y);
        occupied.push(point);
        packed_cards.extend(&current);
        match routes.as_deref_mut() {
            Some(routes) => {
                move_routes(graph, routes, &current, &original);
                packed_segments.extend(routed_segments(graph, routes, &current));
            }
            None => {
                let (smallest_x, smallest_y) = smallest_deltas(graph, &to_pack);
                packed_segments.extend(route_segments(graph, &current, smallest_x, smallest_y));
            }
        }
        packed.push(current);
    }
    if original_score < score(graph, &everything) {
        graph.nodes = original;
        if let (Some(routes), Some(before)) = (routes, original_routes) {
            routes.clone_from_slice(&before);
        }
    }
}

/// TALA `blocksRoutesWithDeltasGuarded`: a card packed before lies on a likely route of the
/// group, or a card of the group on a likely route of what is packed.
fn blocks_routes(graph: &Graph, group: &[Item], packed: &[Vec<Item>], packed_segments: &[Segment], smallest_x: f64, smallest_y: f64) -> bool {
    let own = route_segments(graph, group, smallest_x, smallest_y);
    packed.iter().flatten().any(|item| own.iter().any(|segment| overlaps_line(&node_of(graph, *item), *segment)))
        || group.iter().any(|item| packed_segments.iter().any(|segment| overlaps_line(&node_of(graph, *item), *segment)))
}
