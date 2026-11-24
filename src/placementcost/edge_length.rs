// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.

//! The length of a card's connections, with penalties for cards in the way, for connections
//! that point the wrong way and for labels that get no room. Ported from TALA
//! `internal/placementcost`: `edge_length.go` (`NodeEdgeLength`, `prepareNodeEdgeLength`,
//! `evaluateNodeEdgeLength`, `calculateHorizontalMidpoint`, `calculateVerticalMidpoint`),
//! `cluster.go` (`clusterExactlyTwoExternalConnectedNodes`) and `geometry.go`
//! (`directionCompass`, `compassDelta`, `distanceToPoint`, `placementDistance`).

use crate::graph::{Arrangement, Graph, Node, Point, Vessel};
use crate::placementcost::flow_continuity::flow_continuity_cost;
use crate::routing::geometry::{Orientation, euclidean_distance, is_diagonal};

/// TALA `SideEdgeSpacing`: two sides further apart than this are not in line.
pub const SIDE_EDGE_SPACING: f64 = 40.0;
/// TALA `sizelessDirectionDeltaFactor`.
pub(crate) const DIRECTION_DELTA_FACTOR: f64 = 0.25;

/// What stays the same while a card is tried at several places. TALA `edgeLengthStatic`.
pub struct Setup {
    /// Where a card should lie, seen from the card its connection points to.
    direction: Orientation,
    direction_factor: f64,
}

/// TALA `prepareNodeEdgeLength`: the direction the card's connections should have. Down and to
/// the right; sideways and with far more weight when one card shares more than two labelled
/// connections with it, so that their labels get room.
pub fn prepare(graph: &Graph, node: usize) -> Setup {
    // Per other card: its labelled connections with this card, and how many of them start here.
    let mut counts: Vec<(usize, i32, i32)> = Vec::new();
    let mut many_labels = None;
    for (index, edge) in graph.indexed_edges_of(node) {
        let adjacent = edge.other(node);
        // The connection of a card in a cluster ends at that card for TALA, not at the vessel:
        // it is not counted here.
        if !edge.has_label() || adjacent == node || graph.member_behind(index).is_some() {
            continue;
        }
        let at = counts.iter().position(|(card, _, _)| *card == adjacent).unwrap_or_else(|| {
            counts.push((adjacent, 0, 0));
            counts.len() - 1
        });
        counts[at].1 += 1;
        if edge.from == node {
            counts[at].2 += 1;
        }
        if counts[at].1 > 2 {
            many_labels = Some(at);
        }
    }
    // A vessel's arrows should leave it across its cards: down from a row, right from a column.
    match (graph.vessel_at(node).map(|vessel| vessel.arrangement), many_labels) {
        (Some(Arrangement::Row), _) => Setup { direction: Orientation::Bottom, direction_factor: 0.2 },
        (Some(Arrangement::Column), _) => Setup { direction: Orientation::Right, direction_factor: 0.2 },
        (None, Some(at)) => {
            let (_, all, from_here) = counts[at];
            let direction = if from_here > all / 2 { Orientation::Left } else { Orientation::Right };
            Setup { direction, direction_factor: 10.0 }
        }
        (None, None) => Setup { direction: Orientation::BottomRight, direction_factor: 0.3 },
    }
}

/// TALA `NodeEdgeLength` with sizes and the direction penalty. `turn` is the price of a bend.
pub fn node_edge_length(graph: &Graph, node: usize, turn: f64) -> f64 {
    evaluate(graph, node, turn, &prepare(graph, node))
}

/// TALA `NodeEdgeLength` with sizes and without the direction penalty: the arrows may point
/// any way.
pub fn node_edge_length_any_way(graph: &Graph, node: usize, turn: f64) -> f64 {
    evaluate(graph, node, turn, &Setup { direction: Orientation::None, direction_factor: 0.0 })
}

/// TALA `evaluateNodeEdgeLength`.
pub fn evaluate(graph: &Graph, node_index: usize, turn: f64, setup: &Setup) -> f64 {
    let compass = direction_compass(setup.direction);
    let mut total = 0.0;
    for (edge_index, edge) in graph.indexed_edges_of(node_index) {
        let adjacent_index = edge.other(node_index);
        // TALA `nodeReplacement`, `adjacentNodeReplacement`: a connection at a vessel is priced
        // for the card of the cluster it belongs to.
        let behind = graph.member_behind(edge_index);
        let (node, adjacent) = match &behind {
            Some((vessel, member)) if vessel.node == node_index => (member, &graph.nodes[adjacent_index]),
            Some((_, member)) => (&graph.nodes[node_index], member),
            None => (&graph.nodes[node_index], &graph.nodes[adjacent_index]),
        };
        let direction = node.orientation(adjacent);
        if direction == Orientation::None {
            continue;
        }
        let diagonal = behind.is_none() && is_diagonal(direction);
        // The two ends of the straight line between the cards; for diagonal cards their centres.
        // At a cluster the line is never taken for diagonal: it runs between the two sides
        // that face each other, from middle to middle.
        let (node_end, adjacent_end, semi_diagonal) = match direction {
            _ if behind.is_some() => {
                let (from, to) = (node.center(), adjacent.center());
                match direction {
                    Orientation::Top | Orientation::TopLeft | Orientation::TopRight => {
                        (Point { x: from.x, y: node.bottom() }, Point { x: to.x, y: adjacent.top_left.y }, false)
                    }
                    Orientation::Bottom | Orientation::BottomLeft | Orientation::BottomRight => {
                        (Point { x: from.x, y: node.top_left.y }, Point { x: to.x, y: adjacent.bottom() }, false)
                    }
                    Orientation::Left => (Point { x: node.right(), y: from.y }, Point { x: adjacent.top_left.x, y: to.y }, false),
                    _ => (Point { x: node.top_left.x, y: from.y }, Point { x: adjacent.right(), y: to.y }, false),
                }
            }
            Orientation::Top => {
                let (mid, semi) = horizontal_midpoint(node, adjacent);
                (Point { x: mid, y: node.bottom() }, Point { x: mid, y: adjacent.top_left.y }, semi)
            }
            Orientation::Bottom => {
                let (mid, semi) = horizontal_midpoint(node, adjacent);
                (Point { x: mid, y: node.top_left.y }, Point { x: mid, y: adjacent.bottom() }, semi)
            }
            Orientation::Left => {
                let (mid, semi) = vertical_midpoint(node, adjacent);
                (Point { x: node.right(), y: mid }, Point { x: adjacent.top_left.x, y: mid }, semi)
            }
            Orientation::Right => {
                let (mid, semi) = vertical_midpoint(node, adjacent);
                (Point { x: node.top_left.x, y: mid }, Point { x: adjacent.right(), y: mid }, semi)
            }
            _ => (node.center(), adjacent.center(), false),
        };
        let mut distance = if let Some((vessel, _)) = &behind {
            // From the vessel, or a card would jump back and forth between the cluster's cards.
            placement_distance(&graph.nodes[vessel.node], if vessel.node == node_index { adjacent } else { node })
        } else if diagonal {
            // The two legs of an L, and its bend.
            let corner = Point { x: node_end.x, y: adjacent_end.y };
            distance_to_point(node, corner) + distance_to_point(adjacent, corner) + turn
        } else {
            placement_distance(node, adjacent)
        };

        let mut way = Way {
            node,
            adjacent,
            direction,
            node_end,
            adjacent_end,
            corner_a: false,
            corner_b: false,
            l1: false,
            l2: false,
        };
        if semi_diagonal {
            way.rule_out_the_short_side();
        }
        // TALA stops asking once it knows a route is blocked; asking again changes nothing.
        let mut best_blocked = false;
        let mut alternate_blocked = !semi_diagonal;
        // The order counts: of the routes around, only cards after the first one in the way
        // are asked.
        for index in graph.cards_in_the_way() {
            if index == node_index || index == adjacent_index {
                continue;
            }
            let obstruction = &graph.nodes[index];
            best_blocked |= way.best_is_blocked_by(obstruction, diagonal);
            // Only cards from the one that blocks the straight line on are asked about the
            // routes around.
            if best_blocked && !alternate_blocked && way.alternate_is_blocked_by(obstruction) {
                alternate_blocked = true;
            }
        }
        if best_blocked {
            // At a cluster two bends: as for cards that face each other in line, which a
            // cluster connection always counts as.
            distance += if diagonal {
                // The L becomes an S: one bend more.
                turn
            } else if semi_diagonal {
                if alternate_blocked { turn * 2.0 } else { turn }
            } else {
                // Two bends to get around the card in the way.
                turn * 2.0
            };
        }

        if let Some((vessel, _)) = &behind {
            let outside = if vessel.node == node_index { adjacent_index } else { node_index };
            cluster_penalty(graph, vessel, outside, direction, turn, &mut distance);
        }

        // The connection should point the way `setup.direction` says.
        let mut pointing = direction;
        if edge.from == node_index {
            pointing = opposite(pointing);
        }
        if behind.is_some() {
            // At a cluster the middle counts more than the compass: a diagonal on the wanted
            // side is as good as straight. TALA has the same for a wanted direction up, which
            // no card and no vessel has.
            pointing = match (setup.direction, pointing) {
                (Orientation::Left, Orientation::TopLeft | Orientation::BottomLeft) => Orientation::Left,
                (Orientation::Right, Orientation::TopRight | Orientation::BottomRight) => Orientation::Right,
                (Orientation::Bottom, Orientation::BottomLeft | Orientation::BottomRight) => Orientation::Bottom,
                _ => pointing,
            };
        }
        let delta = f64::from(compass_delta(compass, direction_compass(pointing))).abs();
        distance += setup.direction_factor * delta * graph.cell_size * DIRECTION_DELTA_FACTOR;

        // Not asked for a vessel whose arrows should point down (TALA: up or down).
        if edge.has_label() && setup.direction != Orientation::Bottom {
            // Two labels fit beside an upright connection, one on each side. More do not.
            let member = |index: usize| graph.behind.get(index).copied().flatten();
            let labelled = graph
                .indexed_edges_of(node_index)
                .filter(|(index, other)| other.has_label() && other.other(node_index) == adjacent_index && member(*index) == member(edge_index))
                .count();
            let sideways = matches!(direction, Orientation::Left | Orientation::Right);
            if labelled > 2 && !sideways {
                distance += labelled as f64 * turn;
            }
        }
        total += distance;
    }
    total + flow_continuity_cost(graph, node_index, turn)
}

/// What a connection between a card and a cluster costs more, where the card `outside` lies
/// badly to the cluster: beside a row or above or below a column (`direction`, seen from the
/// card of the cluster); on the side of the cluster's only other neighbour; out of line with the
/// middle of the vessel, or not across from it at all. The blocks on `Cluster != nil` in TALA
/// `evaluateNodeEdgeLength`.
fn cluster_penalty(graph: &Graph, vessel: &Vessel, outside: usize, direction: Orientation, turn: f64, distance: &mut f64) {
    use Orientation::{Bottom, BottomLeft, BottomRight, Left, Right, Top, TopLeft, TopRight};
    let row = vessel.arrangement == Arrangement::Row;
    let (whole, card) = (&graph.nodes[vessel.node], &graph.nodes[outside]);
    // TALA adds every part to the distance on its own: added up first, the sum rounds otherwise.
    if (row && matches!(direction, Left | Right)) || (!row && matches!(direction, Top | Bottom)) {
        *distance += turn * 2.0;
    }
    let this = whole.orientation(card);
    // TALA `clusterExactlyTwoExternalConnectedNodes`: the cards the cluster is connected to.
    let mut neighbours: Vec<usize> = Vec::new();
    for edge in graph.edges.iter().filter(|edge| edge.from == vessel.node || edge.to == vessel.node) {
        if !neighbours.contains(&edge.other(vessel.node)) {
            neighbours.push(edge.other(vessel.node));
        }
    }
    if neighbours.len() == 2 {
        // Two cards above a row would escape every other penalty.
        let other = whole.orientation(&graph.nodes[if neighbours[0] == outside { neighbours[1] } else { neighbours[0] }]);
        let side = |of: Orientation| match (row, of) {
            (true, TopLeft | Top | TopRight) | (false, TopLeft | Left | BottomLeft) => 1,
            (true, BottomLeft | Bottom | BottomRight) | (false, TopRight | Right | BottomRight) => 2,
            _ => 0,
        };
        if side(other) != 0 && side(other) == side(this) {
            *distance += turn;
        }
    }
    *distance += match (row, this) {
        (true, Top | Bottom) => (whole.center().x - card.center().x).abs(),
        (false, Left | Right) => (whole.center().y - card.center().y).abs(),
        _ => turn,
    };
}

/// The routes a connection between two cards can take, and which of them are known to be
/// blocked. The closures `checkBestRouteBlocked` and `checkAlternateRouteBlocked` in TALA
/// `evaluateNodeEdgeLength`.
struct Way<'a> {
    node: &'a Node,
    adjacent: &'a Node,
    direction: Orientation,
    node_end: Point,
    adjacent_end: Point,
    /// Diagonal cards: is the L through the one corner blocked, and through the other?
    corner_a: bool,
    corner_b: bool,
    /// Cards that face each other but are out of line: the route around clockwise is blocked,
    /// and the one counterclockwise.
    l1: bool,
    l2: bool,
}

impl Way<'_> {
    /// Where the two cards are in line on one side there is no room for a route on that side.
    fn rule_out_the_short_side(&mut self) {
        let (node, adjacent) = (self.node, self.adjacent);
        match self.direction {
            Orientation::Left | Orientation::Right => {
                if (node.top_left.y - adjacent.top_left.y).abs() <= SIDE_EDGE_SPACING {
                    self.l1 = true;
                } else if (node.bottom() - adjacent.bottom()).abs() <= SIDE_EDGE_SPACING {
                    self.l2 = true;
                }
            }
            _ => {
                if (node.top_left.x - adjacent.top_left.x).abs() <= SIDE_EDGE_SPACING {
                    self.l2 = true;
                } else if (node.right() - adjacent.right()).abs() <= SIDE_EDGE_SPACING {
                    self.l1 = true;
                }
            }
        }
    }

    /// The straight line, or for diagonal cards both L routes (one card may block the one, a
    /// card seen earlier the other).
    fn best_is_blocked_by(&mut self, other: &Node, diagonal: bool) -> bool {
        let (node_end, adjacent_end) = (self.node_end, self.adjacent_end);
        if !diagonal {
            return other.passes_through(node_end, adjacent_end);
        }
        let a = Point { x: node_end.x, y: adjacent_end.y };
        let b = Point { x: adjacent_end.x, y: node_end.y };
        if !self.corner_a && (other.passes_through(node_end, a) || other.passes_through(adjacent_end, a)) {
            self.corner_a = true;
        }
        if !self.corner_b && (other.passes_through(node_end, b) || other.passes_through(adjacent_end, b)) {
            self.corner_b = true;
        }
        self.corner_a && self.corner_b
    }

    /// Cards that face each other out of line: the routes that leave the larger card where the
    /// smaller one does not reach, on either side. TALA swaps the two cards when the first is
    /// below or to the right of the second; the routes tested are the same either way round.
    fn alternate_is_blocked_by(&mut self, other: &Node) -> bool {
        match self.direction {
            Orientation::Top | Orientation::Bottom => self.upright(self.node, self.adjacent, other),
            Orientation::Left | Orientation::Right => self.sideways(self.node, self.adjacent, other),
            _ => unreachable!("only cards that face each other are out of line"),
        }
        self.l1 && self.l2
    }

    /// One card above the other.
    fn upright(&mut self, upper: &Node, lower: &Node, other: &Node) {
        let (upper_center, lower_center) = (upper.center(), lower.center());
        let floor = upper.top_left.x.max(lower.top_left.x);
        let ceil = upper.right().min(lower.right());
        if !self.l2 {
            let min_x = upper.top_left.x.min(lower.top_left.x);
            let mid = min_x + (floor - min_x).abs() / 2.0;
            let down = (Point { x: mid, y: upper_center.y }, Point { x: mid, y: lower_center.y });
            if upper.top_left.x < lower.top_left.x
                && (other.passes_through(down.0, down.1) || other.passes_through(down.1, lower_center))
            {
                self.l2 = true;
            }
            if upper.top_left.x > lower.top_left.x
                && (other.passes_through(upper_center, down.0) || other.passes_through(down.1, down.0))
            {
                self.l2 = true;
            }
        }
        if !self.l1 {
            let max_x = upper.right().max(lower.right());
            let mid = max_x - (ceil - max_x).abs() / 2.0;
            let down = (Point { x: mid, y: upper_center.y }, Point { x: mid, y: lower_center.y });
            if upper.right() > lower.right()
                && (other.passes_through(down.0, down.1) || other.passes_through(lower_center, down.1))
            {
                self.l1 = true;
            }
            if upper.right() < lower.right()
                && (other.passes_through(upper_center, down.0) || other.passes_through(down.1, down.0))
            {
                self.l1 = true;
            }
        }
    }

    /// One card beside the other.
    fn sideways(&mut self, left: &Node, right: &Node, other: &Node) {
        let (left_center, right_center) = (left.center(), right.center());
        let floor = left.top_left.y.max(right.top_left.y);
        let ceil = left.bottom().min(right.bottom());
        if !self.l1 {
            let min_y = left.top_left.y.min(right.top_left.y);
            let mid = min_y + (floor - min_y).abs() / 2.0;
            let across = (Point { x: left_center.x, y: mid }, Point { x: right_center.x, y: mid });
            if left.top_left.y < right.top_left.y
                && (other.passes_through(across.0, across.1) || other.passes_through(right_center, across.1))
            {
                self.l1 = true;
            }
            if left.top_left.y > right.top_left.y
                && (other.passes_through(left_center, across.0) || other.passes_through(across.1, across.0))
            {
                self.l1 = true;
            }
        }
        if !self.l2 {
            let max_y = left.bottom().max(right.bottom());
            let mid = max_y - (ceil - max_y).abs() / 2.0;
            let across = (Point { x: left_center.x, y: mid }, Point { x: right_center.x, y: mid });
            if left.bottom() > right.bottom()
                && (other.passes_through(across.0, across.1) || other.passes_through(right_center, across.1))
            {
                self.l2 = true;
            }
            if left.bottom() < right.bottom()
                && (other.passes_through(left_center, across.0) || other.passes_through(across.1, across.0))
            {
                self.l2 = true;
            }
        }
    }
}

/// TALA `calculateHorizontalMidpoint`: the middle of the columns two cards share, and whether
/// the cards are out of line on a side.
fn horizontal_midpoint(node: &Node, adjacent: &Node) -> (f64, bool) {
    let ceil = node.right().min(adjacent.right());
    let floor = node.top_left.x.max(adjacent.top_left.x);
    let semi_diagonal = (node.top_left.x - adjacent.top_left.x).abs() > SIDE_EDGE_SPACING
        || (node.right() - adjacent.right()).abs() > SIDE_EDGE_SPACING;
    ((floor + ceil) / 2.0, semi_diagonal)
}

/// TALA `calculateVerticalMidpoint`.
fn vertical_midpoint(node: &Node, adjacent: &Node) -> (f64, bool) {
    let ceil = node.bottom().min(adjacent.bottom());
    let floor = node.top_left.y.max(adjacent.top_left.y);
    let semi_diagonal = (node.top_left.y - adjacent.top_left.y).abs() > SIDE_EDGE_SPACING
        || (node.bottom() - adjacent.bottom()).abs() > SIDE_EDGE_SPACING;
    ((floor + ceil) / 2.0, semi_diagonal)
}

/// TALA `distanceToPoint` with sizes.
fn distance_to_point(node: &Node, point: Point) -> f64 {
    let gap = |start: f64, end: f64, at: f64| {
        if end < at {
            at - end
        } else if at < start {
            start - at
        } else {
            0.0
        }
    };
    let dx = gap(node.top_left.x, node.right(), point.x);
    let dy = gap(node.top_left.y, node.bottom(), point.y);
    euclidean_distance(0.0, 0.0, dx, dy)
}

/// TALA `placementDistance` with sizes: the gap between two cards, plus a little for how far
/// their centres are out of line.
fn placement_distance(first: &Node, second: &Node) -> f64 {
    let x = (first.center().x - second.center().x).abs() / (first.width + second.width);
    let y = (first.center().y - second.center().y).abs() / (first.height + second.height);
    first.distance_to(second) + x.min(y) / 20.0
}

/// TALA `directionCompass`: eighths of a turn, clockwise from Top.
pub(crate) fn direction_compass(direction: Orientation) -> i32 {
    match direction {
        Orientation::BottomLeft => -3,
        Orientation::Left => -2,
        Orientation::TopLeft => -1,
        Orientation::Top | Orientation::None => 0,
        Orientation::TopRight => 1,
        Orientation::Right => 2,
        Orientation::BottomRight => 3,
        Orientation::Bottom => 4,
    }
}

/// TALA `compassDelta`: the shorter way round from one direction to another.
pub(crate) fn compass_delta(first: i32, second: i32) -> i32 {
    let delta = second - first;
    if delta > 4 {
        delta - 8
    } else if delta < -4 {
        delta + 8
    } else {
        delta
    }
}

/// D2 `Orientation.GetOpposite`.
pub(crate) fn opposite(direction: Orientation) -> Orientation {
    match direction {
        Orientation::TopLeft => Orientation::BottomRight,
        Orientation::TopRight => Orientation::BottomLeft,
        Orientation::BottomLeft => Orientation::TopRight,
        Orientation::BottomRight => Orientation::TopLeft,
        Orientation::Top => Orientation::Bottom,
        Orientation::Bottom => Orientation::Top,
        Orientation::Right => Orientation::Left,
        Orientation::Left => Orientation::Right,
        Orientation::None => Orientation::None,
    }
}
