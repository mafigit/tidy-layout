// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.

//! One pass of TALA's "sized" optimizer over all cards: each card in turn moves to the cheapest
//! place near the middle of the cards it is connected to; a card that stays swaps places with a
//! neighbour or turns its branch around a card where that is cheaper. Ported from TALA
//! `internal/placement`: `sized_optimizer.go` (`optimizeGuarded`, `medianPointGuarded`,
//! `bestSwapCandidateGuarded`, `protrudingChildrenGuarded`, `withHubSpokesSuppressed`),
//! `optimization_guard.go` (`optimizerMedian`, `optimizerAdjacents`, `optimizerSwapPositions`),
//! `transpose.go` (`transpose`, `rotateAround`), `cluster_optimization.go` (`optimizeCluster`
//! with `onlyFlip`), `cluster_connections.go`, `internal/proximity/hubs.go` (`AddHubs`) and
//! `internal/layoutgraph`: `transaction.go` (what `Commit` refuses), `node.go`
//! (`reachableNodesGuarded`, `isAdjacentTo`).
//!
//! For plain cards in no container, as node placement calls it for them (with the edge
//! abductions of the clusters). Not ported: cards that belong to a tree (`NodeToTree`), fixed
//! places, cards without connections that are "near" another.

use crate::gorand::GoRand;
use crate::graph::{Arrangement, Graph, Node, Point, Vessel};
use crate::placement::{
    Checked, closest_unoccupied_ring, does_overlap, fill_placement_points, move_node_to_best,
    precision_compare, round_to_cell,
};
use crate::placementcost::{edge_length::node_edge_length, symmetry::node_symmetry};
use crate::routing::geometry::{Orientation, is_diagonal};

/// TALA reports an error when a card finds no free place within 100 rings of its median point.
#[derive(Debug, PartialEq)]
pub struct NoFreePlace {
    pub node: usize,
}

/// TALA `proximity.AddHubs`: per card its spokes, if it is a hub. A spoke is a connected card
/// with no other connection; a hub has a spoke and a connection to a card that is none.
pub fn hubs(graph: &Graph) -> Vec<Option<Vec<usize>>> {
    (0..graph.nodes.len())
        .map(|node| {
            let mut spokes = Vec::new();
            let mut connected = false;
            for edge in graph.edges_of(node) {
                let adjacent = edge.other(node);
                if graph.edges_of(adjacent).count() == 1 {
                    spokes.push(adjacent);
                } else {
                    connected = true;
                }
            }
            (connected && !spokes.is_empty()).then_some(spokes)
        })
        .collect()
}

/// TALA `optimizerMedian` along one axis: the middle of the middle card, or of the two middle
/// cards, in pixels. `middles` has one entry per connection.
fn median(mut middles: Vec<f64>) -> f64 {
    // TALA sorts the cards (equal ones by their number) and adds place and half length one by
    // one; with places and sizes in whole pixels every sum is exact, so the values are enough.
    middles.sort_by(f64::total_cmp);
    let middle = middles.len() / 2;
    if middles.len().is_multiple_of(2) { (middles[middle] + middles[middle - 1]) / 2.0 } else { middles[middle] }
}

/// TALA `medianPointGuarded`: the point a card is drawn to, on the pixel: the middle of the
/// cards it is connected to, each counted once per connection. With a temperature the point is
/// off by up to that many card sizes either way, at random. The card needs a connection.
pub fn median_point(graph: &Graph, node: usize, temp: f64, random: &mut GoRand) -> Point {
    let cell = graph.cell_size;
    let card = &graph.nodes[node];
    let (width, height) = (card.width / cell, card.height / cell);
    // TALA `optimizerAdjacents`: a card connected to a cluster is drawn to the cluster's card
    // its connection belongs to, not to the vessel.
    let adjacents: Vec<Node> = graph
        .indexed_edges_of(node)
        .map(|(index, edge)| match graph.member_behind(index) {
            Some((vessel, member)) if vessel.node != node => member,
            _ => graph.nodes[edge.other(node)].clone(),
        })
        .collect();
    let along = |x: bool| {
        let middles = adjacents.iter().map(|other| {
            if x { other.top_left.x + other.width / 2.0 } else { other.top_left.y + other.height / 2.0 }
        });
        median(middles.collect()) / cell
    };
    let (mut median_x, mut median_y) = (along(true), along(false));
    median_x += (-temp * width) + random.float64() * (2.0 * temp * width);
    median_y += (-temp * height) + random.float64() * (2.0 * temp * height);
    // TALA `protrudingChildren`: a vessel goes where the middle of its cards, each counted once
    // per connection, comes to lie on the point, not its own corner.
    let members: Vec<Node> = graph
        .indexed_edges_of(node)
        .filter_map(|(index, _)| graph.member_behind(index))
        .filter(|(vessel, _)| vessel.node == node)
        .map(|(_, member)| member)
        .collect();
    if !members.is_empty() {
        median_x -= median(members.iter().map(|member| member.center().x).collect()) / cell - card.top_left.x / cell;
        median_y -= median(members.iter().map(|member| member.center().y).collect()) / cell - card.top_left.y / cell;
    }
    Point { x: (median_x * cell).round(), y: (median_y * cell).round() }
}

/// What a card costs where it is, as the swap sees it: the length of its connections, less a
/// cell per connection when it lies symmetric.
fn price(graph: &Graph, node: usize, turn: f64) -> f64 {
    let length = node_edge_length(graph, node, turn);
    length - node_symmetry(graph, node) * graph.cell_size * graph.edges_of(node).count() as f64
}

/// TALA `bestSwapCandidateGuarded`: the card that `node` should swap places with: one no further
/// away than a cell, where the swap makes `node` cheaper and the two together cheaper, and
/// neither comes too close to another card. Of several the one with the cheapest result; the
/// cards are asked in a random order. The graph is left as it was.
pub fn best_swap_candidate(graph: &mut Graph, node: usize, turn: f64, random: &mut GoRand) -> Option<usize> {
    let current = price(graph, node, turn);
    let mut order: Vec<usize> = (0..graph.nodes.len()).collect();
    random.shuffle(&mut order);
    let mut least = f64::INFINITY;
    let mut best = None;
    for candidate in order {
        // TALA skips the card itself; a swap with itself changes no price and is never taken.
        if graph.nodes[candidate].distance_to(&graph.nodes[node]) > graph.cell_size {
            continue;
        }
        let (home, other) = (graph.nodes[node].top_left, graph.nodes[candidate].top_left);
        // TALA first asks whether each of the two fits the other's place with the other gone;
        // the test after the swap refuses all of that too.
        let current_other = price(graph, candidate, turn);

        (graph.nodes[node].top_left, graph.nodes[candidate].top_left) = (other, home);
        // Two cards of different sizes may overlap each other after the swap.
        let valid = !does_overlap(graph, candidate, home) && !does_overlap(graph, node, other);
        let (mut swapped, mut swapped_other) = (0.0, 0.0);
        if valid {
            swapped = price(graph, node, turn);
            if precision_compare(swapped, current) < 0 {
                swapped_other = price(graph, candidate, turn);
            }
        }
        (graph.nodes[node].top_left, graph.nodes[candidate].top_left) = (home, other);

        if valid
            && precision_compare(swapped, current) < 0
            && precision_compare(swapped + swapped_other, current + current_other) < 0
            && precision_compare(swapped + swapped_other, least) < 0
        {
            least = swapped + swapped_other;
            best = Some(candidate);
        }
    }
    best
}

/// TALA `reachableNodesGuarded`: `start` and every card that connections lead to from it without
/// passing `ignore`, nearest first.
pub(crate) fn reachable(graph: &Graph, start: usize, ignore: usize) -> Vec<usize> {
    let mut found = vec![start];
    let mut next = 0;
    while next < found.len() {
        let current = found[next];
        next += 1;
        for edge in graph.edges_of(current) {
            let adjacent = edge.other(current);
            if adjacent != ignore && !found.contains(&adjacent) {
                found.push(adjacent);
            }
        }
    }
    found
}

/// Does the first card, where it is, come closer to the second than the gap they must keep?
fn too_close(graph: &Graph, first: usize, second: usize) -> bool {
    let (a, b) = (&graph.nodes[first], &graph.nodes[second]);
    let delta = graph.gap_between(first, second, a.top_left);
    a.top_left.x < b.right() + delta
        && a.right() + delta > b.top_left.x
        && a.top_left.y < b.bottom() + delta
        && a.bottom() + delta > b.top_left.y
}

/// TALA `DoesOverlapExact`: the two cards cover a common area.
fn covers(graph: &Graph, first: usize, second: usize) -> bool {
    let (a, b) = (&graph.nodes[first], &graph.nodes[second]);
    a.top_left.x < b.right() && b.top_left.x < a.right() && a.top_left.y < b.bottom() && b.top_left.y < a.bottom()
}

/// TALA `rotateAround`: the card a quarter turn further around the middle of `center`; with
/// `on_grid` on the grid, as node placement wants it.
pub(crate) fn rotate_around(graph: &mut Graph, node: usize, center: usize, times: usize, on_grid: bool) {
    let cell = graph.cell_size;
    for _ in 0..times {
        let (card, around) = (&graph.nodes[node], &graph.nodes[center]);
        let middle = card.center();
        let translated_x = middle.x - around.top_left.x - around.width / 2.0;
        let translated_y = middle.y - around.top_left.y - around.height / 2.0;
        let (rotated_x, rotated_y) = (-translated_y, translated_x);
        let new_x = rotated_x + around.top_left.x + around.width / 2.0;
        let new_y = rotated_y + around.top_left.y + around.height / 2.0;
        // TALA rounds to the grid only what is not on it; rounding leaves the rest as it is.
        let (x, y) = ((new_x - card.width / 2.0).round(), (new_y - card.height / 2.0).round());
        graph.nodes[node].top_left = if on_grid { Point { x: round_to_cell(x, cell), y: round_to_cell(y, cell) } } else { Point { x, y } };
    }
}

/// TALA `transpose`: a card with one connection turns around the card at its other end; a card
/// with two turns, with the smaller of the two branches it joins, around the first card of the
/// larger one. It takes the quarter turn (one, two or three) that makes the connections of the
/// card and of the cards it is connected to shortest, if any is shorter than now and brings no
/// two cards too close. Says whether it turned anything.
///
/// ```text
/// E -- A          E -- A -- C -- B
/// |    |     =>   |
/// D    C          D
///      |
///      B
/// ```
pub fn transpose(graph: &mut Graph, node: usize, turn: f64) -> bool {
    if graph.trees_at.contains(&node) {
        return false;
    }
    // TALA turns neither the branch of a vessel nor that of a card connected to one. Neither
    // gets further than this anyway: a vessel has four connections or more, and a card has
    // one to every card of the cluster, which is a ring where they are its only two.
    let adjacents: Vec<usize> = graph.edges_of(node).map(|edge| edge.other(node)).collect();
    let diagonal = |other: usize| is_diagonal(graph.nodes[node].orientation(&graph.nodes[other]));
    let (turning, center) = match adjacents[..] {
        [a] => {
            if diagonal(a) {
                return false;
            }
            (vec![node], a)
        }
        [a, b] => {
            let from_a = reachable(graph, a, node);
            // The card closes a ring: there are no two branches.
            if from_a.contains(&b) || diagonal(a) || diagonal(b) {
                return false;
            }
            let from_b = reachable(graph, b, node);
            let (mut smaller, center) = if from_a.len() >= from_b.len() { (from_b, a) } else { (from_a, b) };
            smaller.push(node);
            (smaller, center)
        }
        _ => return false,
    };

    let length = |graph: &Graph| {
        adjacents.iter().fold(node_edge_length(graph, node, turn), |sum, adjacent| {
            sum + node_edge_length(graph, *adjacent, turn)
        })
    };
    let allowed = Allowed::from(graph);
    // A vessel that is turned around a card may turn from a row into a column, or back.
    let rotate = |graph: &mut Graph, times: usize| {
        for &card in &turning {
            rotate_around(graph, card, center, times, true);
            if let Some(vessel) = graph.vessels.iter().position(|vessel| vessel.node == card) {
                turn_cluster(graph, vessel);
            }
        }
    };
    let before = (graph.nodes.clone(), graph.vessels.clone(), graph.turns);
    let mut best_length = length(graph);
    let mut best_times = None;
    for times in 1..=3 {
        rotate(graph, times);
        if allowed.now(graph) {
            let turned = length(graph);
            if precision_compare(turned, best_length) < 0 {
                best_length = turned;
                best_times = Some(times);
            }
        }
        (graph.nodes, graph.vessels, graph.turns) = before.clone();
    }
    let Some(times) = best_times else {
        return false;
    };
    rotate(graph, times);
    true
}

/// What TALA's transaction refuses: a state in which two cards are too close that were not
/// before, or cover each other that did not before.
pub(crate) struct Allowed {
    close_before: Vec<(usize, usize)>,
    covering_before: Vec<(usize, usize)>,
}

impl Allowed {
    fn pairs(graph: &Graph) -> impl Iterator<Item = (usize, usize)> + Clone {
        let count = graph.nodes.len();
        (0..count).flat_map(move |a| (0..count).filter(move |b| *b != a).map(move |b| (a, b)))
    }

    /// The state the transaction starts from.
    pub(crate) fn from(graph: &Graph) -> Self {
        // TALA asks both ways round; the gap two plain cards must keep is the same both ways.
        Allowed {
            close_before: Self::pairs(graph).filter(|(a, b)| too_close(graph, *a, *b)).collect(),
            covering_before: Self::pairs(graph).filter(|(a, b)| covers(graph, *a, *b)).collect(),
        }
    }

    /// Whether the transaction takes the state. TALA asks every pair from each of its two
    /// cards, but not from a card of a cluster (`hasBadPostStateOverlap`): two cards of
    /// clusters may come as close as they like, though not cover each other where they did
    /// not.
    pub(crate) fn now(&self, graph: &Graph) -> bool {
        Self::pairs(graph).all(|(a, b)| self.pair_allowed(graph, a, b))
    }

    /// The same where the transaction's rollback point is `since`: TALA asks only the pairs
    /// with a card that moved since then (`collectOverlapValidationNodes`). A pair two other
    /// moves brought too close, before the rollback point, passes.
    pub(crate) fn now_since(&self, graph: &Graph, since: &[Node]) -> bool {
        let moved = |card: usize| graph.nodes[card] != since[card];
        Self::pairs(graph).all(|(a, b)| !(moved(a) || moved(b)) || self.pair_allowed(graph, a, b))
    }

    fn pair_allowed(&self, graph: &Graph, a: usize, b: usize) -> bool {
        if self.close_before.contains(&(a, b)) {
            self.covering_before.contains(&(a, b)) || !covers(graph, a, b)
        } else {
            graph.cluster_of(a).is_some() || !too_close(graph, a, b)
        }
    }
}

/// TALA `optimizeCluster` with `onlyFlip`: a cluster with more cards beside it than above and
/// below becomes a column, one with more above and below a row. It turns around its middle,
/// or where that brings it too close to a card around its top-left corner, or not at all. Says
/// whether it turned.
pub fn turn_cluster(graph: &mut Graph, which: usize) -> bool {
    let vessel = graph.vessels[which].clone();
    let whole = graph.nodes[vessel.node].clone();
    // TALA `clusterExternalConnectedNodes`: the cards the cluster is connected to.
    let mut neighbours: Vec<usize> = Vec::new();
    for edge in graph.edges.iter().filter(|edge| edge.from == vessel.node || edge.to == vessel.node) {
        if !neighbours.contains(&edge.other(vessel.node)) {
            neighbours.push(edge.other(vessel.node));
        }
    }
    let (mut beside, mut upright) = (0, 0);
    for card in neighbours.iter().map(|card| &graph.nodes[*card]) {
        match whole.orientation(card) {
            Orientation::None => {}
            Orientation::Left | Orientation::Right => beside += 1,
            Orientation::Top | Orientation::Bottom => upright += 1,
            _ => {
                let (x, y) = ((card.center().x - whole.center().x).abs(), (card.center().y - whole.center().y).abs());
                if x > y {
                    beside += 1;
                } else if y > x {
                    upright += 1;
                }
            }
        }
    }
    let desired = match upright.cmp(&beside) {
        std::cmp::Ordering::Greater => Arrangement::Row,
        std::cmp::Ordering::Less => Arrangement::Column,
        std::cmp::Ordering::Equal => vessel.desired,
    };
    graph.vessels[which].desired = desired;
    if vessel.arrangement == desired {
        return false;
    }
    // The padding stays. TALA `PaddingBetween` with the places of the cards takes the room they
    // have now, or a tenth of a card along the new arrangement where that is less. It never is:
    // a row was made of cards higher than wide, a column of cards wider than high.
    let turned = Vessel { arrangement: desired, desired, ..vessel.clone() };
    let (width, height) = turned.size();
    // TALA's transaction for a cluster is one on the whole diagram: the cards outside this
    // graph, where they stand now, must not come too close either.
    let with_outside = |graph: &Graph| {
        let mut all = graph.clone();
        all.nodes.extend(graph.outside.iter().cloned());
        all
    };
    let allowed = Allowed::from(&with_outside(graph));
    for around_middle in [true, false] {
        let mut top_left = whole.top_left;
        if around_middle {
            match desired {
                Arrangement::Column => top_left.y += ((whole.height - height) / 2.0).round(),
                Arrangement::Row => top_left.x += ((whole.width - width) / 2.0).round(),
            }
        }
        graph.vessels[which] = turned.clone();
        graph.nodes[vessel.node] = Node { top_left, width, height };
        if allowed.now(&with_outside(graph)) {
            graph.turns += 1;
            return true;
        }
        graph.vessels[which] = Vessel { desired, ..vessel.clone() };
        graph.nodes[vessel.node] = whole.clone();
    }
    false
}

/// The move of one card: to the cheapest place around its median point. Says whether it moved.
fn settle(
    graph: &mut Graph,
    node: usize,
    turn: f64,
    temp: f64,
    random: &mut GoRand,
    checked: Option<&mut Checked>,
) -> Result<bool, NoFreePlace> {
    let median = median_point(graph, node, temp, random);
    // A vessel is tried at the places themselves only, not a little to the left of and above
    // them (TALA `minimizingSelf`).
    let minimizing_self = graph.vessel_at(node).is_none();
    let ring = closest_unoccupied_ring(graph, node, median, minimizing_self, checked).ok_or(NoFreePlace { node })?;
    let mut points = fill_placement_points(graph, node, median, ring, minimizing_self);
    random.shuffle(&mut points);
    let home = graph.nodes[node].top_left;
    // In the last passes, without temperature, a card moves only to a cheaper place.
    move_node_to_best(graph, node, &points, temp == 0.0, turn);
    Ok(graph.nodes[node].top_left != home)
}

/// TALA `sizedOptimizer.optimize`: one pass over all cards in a random order. `turn` is the
/// price of a bend, `temp` the temperature. Says whether a card moved, swapped or turned.
pub fn optimize(graph: &mut Graph, turn: f64, temp: f64, random: &mut GoRand) -> Result<bool, NoFreePlace> {
    let hubs = hubs(graph);
    let count = graph.nodes.len();
    let cell = graph.cell_size;
    let mut order: Vec<usize> = (0..count).collect();
    random.shuffle(&mut order);
    let mut changed = false;
    for node in order {
        let card = &graph.nodes[node];
        // A giant card finds no free place within 100 cells.
        if card.width > 100.0 * cell || card.height > 100.0 * cell {
            continue;
        }
        if graph.edges_of(node).next().is_none() {
            continue;
        }
        // TALA remembers the offsets it tried only where there are many cards, and does not
        // forget them before the second try of a hub.
        let mut checked = (count > 10).then(Checked::new);
        if settle(graph, node, turn, temp, random, checked.as_mut())? {
            changed = true;
        } else if let Some(other) = best_swap_candidate(graph, node, turn, random) {
            changed = true;
            let (home, place) = (graph.nodes[node].top_left, graph.nodes[other].top_left);
            (graph.nodes[node].top_left, graph.nodes[other].top_left) = (place, home);
        } else if transpose(graph, node, turn) {
            changed = true;
        } else if let (Some(spokes), true) = (&hubs[node], temp != 0.0) {
            // A hub that stays is held by its spokes. It moves as if it had none; they follow in
            // the next pass.
            graph.hidden = spokes
                .iter()
                .filter_map(|spoke| graph.edges.iter().position(|edge| edge.other(node) == *spoke && edge.other(*spoke) == node))
                .collect();
            let moved = settle(graph, node, turn, temp, random, checked.as_mut());
            graph.hidden.clear();
            // This move does not count as a change. Only the passes without temperature are
            // asked whether they changed something, and they never get here.
            moved?;
        }
    }
    Ok(changed)
}
