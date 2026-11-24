// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.

//! The first half of node placement: cards as points on a coarse grid, one card per cell. Each
//! card's `top_left` holds its cell, not pixels; sizes play no part. Ported from TALA
//! `internal/placement`: `initialize.go` (`initializeNodes`, `nodeCandidatePositions`),
//! `stress_initialize.go` (`initializeByGraphDistance`), `sizeless_optimizer.go`, `metrics.go`
//! (`median`, `occupied`), `optimization_guard.go` (`optimizerMedian` without sizes), and
//! `internal/placementcost`: `NodeEdgeLength` without sizes, `sizelessOrientation`,
//! `placementDistance`; `internal/layoutgraph/node.go` (`isMajorityTarget`).
//!
//! For one connected group of plain cards. Not ported: fixed places, "nears", trees.

use crate::gorand::GoRand;
use crate::graph::{Arrangement, Graph, Point};
use crate::placement::precision_compare;
use crate::placementcost::edge_length::{DIRECTION_DELTA_FACTOR, compass_delta, direction_compass, opposite};
use crate::placementcost::flow_continuity::hypot;
use crate::policy::MAX_SEARCH_RINGS;
use crate::routing::geometry::{Orientation, euclidean_distance};
use std::collections::HashMap;

/// TALA reports an error when a card finds no free cell within 100 rings of its median point.
#[derive(Debug, PartialEq)]
pub struct NoFreeCell {
    pub node: usize,
}

/// TALA `sizelessOrientation`: where the first point lies, seen from the second.
fn orientation(node: Point, other: Point) -> Orientation {
    if node.y < other.y {
        if node.x < other.x {
            return Orientation::TopLeft;
        }
        if other.x < node.x {
            return Orientation::TopRight;
        }
        return Orientation::Top;
    }
    if other.y < node.y {
        if node.x < other.x {
            return Orientation::BottomLeft;
        }
        if other.x < node.x {
            return Orientation::BottomRight;
        }
        return Orientation::Bottom;
    }
    if other.x < node.x {
        return Orientation::Right;
    }
    if node.x < other.x {
        return Orientation::Left;
    }
    Orientation::None
}

/// TALA `NodeEdgeLength` without sizes: per connection the distance between the two cells, a
/// little for being out of line, and the penalty for an arrow that does not point down and to
/// the right. With `placed`, connections to cards that have no cell yet do not count.
pub fn edge_length(graph: &Graph, node: usize, placed: Option<&[bool]>) -> f64 {
    let at = graph.nodes[node].top_left;
    // A vessel's arrows should leave it across its cards: down from a row, right from a column.
    let (direction, factor) = match graph.vessel_at(node).map(|vessel| vessel.arrangement) {
        Some(Arrangement::Row) => (Orientation::Bottom, 0.2),
        Some(Arrangement::Column) => (Orientation::Right, 0.2),
        None => (Orientation::BottomRight, 0.3),
    };
    let compass = direction_compass(direction);
    let mut total = 0.0;
    for edge in graph.edges_of(node) {
        let adjacent = edge.other(node);
        if placed.is_some_and(|placed| !placed[adjacent]) {
            continue;
        }
        let other = graph.nodes[adjacent].top_left;
        let (x, y) = ((at.x - other.x).abs(), (at.y - other.y).abs());
        let mut distance = euclidean_distance(0.0, 0.0, x, y) + x.min(y) / 20.0;
        let mut pointing = orientation(at, other);
        if edge.from == node {
            pointing = opposite(pointing);
        }
        let delta = f64::from(compass_delta(compass, direction_compass(pointing))).abs();
        distance += factor * delta * 1.0 * DIRECTION_DELTA_FACTOR;
        total += distance;
    }
    total
}

/// TALA `median` without sizes, along one axis: half a cell beyond the middle card, or the mean
/// of that and the card before it. `cells` has one entry per connection.
fn median(mut cells: Vec<f64>) -> f64 {
    cells.sort_by(f64::total_cmp);
    let middle = cells.len() / 2;
    let median = cells[middle] + 0.5;
    if cells.len().is_multiple_of(2) { (median + cells[middle - 1]) / 2.0 } else { median }
}

/// The median of the cells of the cards a card is connected to, one per connection.
fn median_to_neighbors(graph: &Graph, node: usize, placed: Option<&[bool]>) -> (f64, f64) {
    let cells: Vec<Point> = graph
        .edges_of(node)
        .map(|edge| edge.other(node))
        .filter(|adjacent| placed.is_none_or(|placed| placed[*adjacent]))
        .map(|adjacent| graph.nodes[adjacent].top_left)
        .collect();
    (median(cells.iter().map(|cell| cell.x).collect()), median(cells.iter().map(|cell| cell.y).collect()))
}

/// TALA `findClosestUnoccupiedDistanceGuarded` of the sizeless optimizer: the ring of the
/// nearest free cell around a cell.
fn closest_unoccupied(median: Point, occupied: impl Fn(Point) -> bool) -> Option<f64> {
    for ring in 0..=MAX_SEARCH_RINGS {
        let ring = f64::from(ring);
        let (mut x, mut y) = (ring, 0.0);
        while x >= -ring {
            if y != 0.0 && !occupied(Point { x: median.x + x, y: median.y - y }) {
                return Some(ring);
            }
            if !occupied(Point { x: median.x + x, y: median.y + y }) {
                return Some(ring);
            }
            if x > 0.0 {
                y += 1.0;
            } else {
                y -= 1.0;
            }
            x -= 1.0;
        }
    }
    None
}

/// TALA `initializeNodes`: the first card goes to the cell (count, count); the others follow in
/// the order connections lead to them, each to the free cell near the median of its placed
/// neighbours where its connections are shortest. False when a card cannot be reached from the
/// first one or finds no cell.
pub fn first_arrangement(graph: &mut Graph) -> bool {
    let count = graph.nodes.len();
    // TALA `allReachableNodesGuarded`: nearest first.
    let mut order = vec![0];
    let mut next = 0;
    while next < order.len() {
        let current = order[next];
        next += 1;
        for adjacent in graph.edges_of(current).map(|edge| edge.other(current)).collect::<Vec<_>>() {
            if !order.contains(&adjacent) {
                order.push(adjacent);
            }
        }
    }
    let mut placed = vec![false; count];
    graph.nodes[0].top_left = Point { x: count as f64, y: count as f64 };
    placed[0] = true;
    for &node in &order[1..] {
        let (x, y) = median_to_neighbors(graph, node, Some(&placed));
        let middle = Point { x: x.floor(), y: y.floor() };
        let occupied = |graph: &Graph, placed: &[bool], cell: Point| {
            graph.nodes.iter().zip(placed).any(|(card, placed)| *placed && card.top_left == cell)
        };
        let Some(ring) = closest_unoccupied(middle, |cell| occupied(graph, &placed, cell)) else {
            return false;
        };
        let reach = ring + 2.0;
        // TALA `isMajorityTarget`: a card that more arrows point at than leave is tried from the
        // bottom right, the others from the top left.
        let arrows: i32 = graph.edges_of(node).map(|edge| if edge.to == node { 1 } else { -1 }).sum();
        let mut candidates = Vec::new();
        let mut x = middle.x - reach;
        while x <= middle.x + reach {
            let mut y = middle.y - reach;
            while y <= middle.y + reach {
                candidates.push(Point { x, y });
                y += 1.0;
            }
            x += 1.0;
        }
        if arrows > 0 {
            candidates.reverse();
        }
        let mut least = f64::INFINITY;
        let mut best = None;
        for cell in candidates {
            if occupied(graph, &placed, cell) {
                continue;
            }
            graph.nodes[node].top_left = cell;
            let length = edge_length(graph, node, Some(&placed));
            if length < least {
                least = length;
                best = Some(cell);
            }
        }
        let Some(cell) = best else {
            return false;
        };
        graph.nodes[node].top_left = cell;
        placed[node] = true;
    }
    placed.iter().all(|placed| *placed)
}

/// TALA `initializeByGraphDistance`: an arrangement in which cards lie as far apart as the
/// number of connections between them, for 4 to 64 cards that are all connected. False when it
/// does not apply; the cards are then left as they were.
///
/// TALA computes the step size of each sweep with Go's `math.Pow`, whose last bit depends on
/// the processor; here it is Rust's. The cells are whole numbers, so that bit rarely matters.
pub fn first_arrangement_by_distance(graph: &mut Graph) -> bool {
    let n = graph.nodes.len();
    if !(4..=64).contains(&n) || graph.edges.len() > 256 {
        return false;
    }
    let mut d = vec![vec![f64::INFINITY; n]; n];
    for (i, row) in d.iter_mut().enumerate() {
        row[i] = 0.0;
    }
    for edge in &graph.edges {
        if edge.from != edge.to {
            d[edge.from][edge.to] = 1.0;
            d[edge.to][edge.from] = 1.0;
        }
    }
    for k in 0..n {
        for i in 0..n {
            for j in 0..n {
                d[i][j] = d[i][j].min(d[i][k] + d[k][j]);
            }
        }
    }
    if d.iter().flatten().any(|distance| distance.is_infinite()) {
        return false;
    }
    let (mut x, mut y) = (vec![0.0_f64; n], vec![0.0_f64; n]);
    for i in 0..n {
        let angle = 2.0 * std::f64::consts::PI * i as f64 / n as f64;
        (x[i], y[i]) = ((n as f64).sqrt() * angle.cos(), (n as f64).sqrt() * angle.sin());
    }
    // Pairs of cards move towards the distance the graph gives them, in ever smaller steps.
    for sweep in 0..48 {
        let eta = 0.7 * (0.02_f64 / 0.7).powf(sweep as f64 / 47.0);
        for p in 0..n {
            let i = (p + sweep) % n;
            for q in p + 1..n {
                let j = (q + sweep) % n;
                let (mut dx, mut dy) = (x[i] - x[j], y[i] - y[j]);
                let mut length = hypot(dx, dy);
                if length < 1e-9 {
                    (dx, dy, length) = (1e-6, 1e-6, 2.0_f64.sqrt() * 1e-6);
                }
                let mu = 1.0_f64.min(eta / (d[i][j] * d[i][j]));
                let amount = 0.5 * mu * (length - d[i][j]) / length;
                x[i] -= dx * amount;
                y[i] -= dy * amount;
                x[j] += dx * amount;
                y[j] += dy * amount;
            }
        }
    }
    // The cards with the most connections choose their cell first.
    let mut order: Vec<usize> = (0..n).collect();
    order.sort_by_key(|card| std::cmp::Reverse(graph.edges_of(*card).count()));
    let mut taken: Vec<(i64, i64)> = Vec::with_capacity(n);
    let mut cells = vec![(0, 0); n];
    for i in order {
        let (tx, ty) = (x[i] * 2.0 + n as f64, y[i] * 2.0 + n as f64);
        let (cx, cy) = (tx.round() as i64, ty.round() as i64);
        let mut best = (cx, cy);
        let mut cost = f64::INFINITY;
        for radius in 0..=n as i64 {
            for xx in cx - radius..=cx + radius {
                for yy in cy - radius..=cy + radius {
                    let inside = xx != cx - radius && xx != cx + radius && yy != cy - radius && yy != cy + radius;
                    if (radius > 0 && inside) || taken.contains(&(xx, yy)) {
                        continue;
                    }
                    let c = (xx as f64 - tx) * (xx as f64 - tx) + (yy as f64 - ty) * (yy as f64 - ty);
                    if c < cost {
                        (best, cost) = ((xx, yy), c);
                    }
                }
            }
            if cost < f64::INFINITY {
                break;
            }
        }
        taken.push(best);
        cells[i] = best;
    }
    for (card, (x, y)) in graph.nodes.iter_mut().zip(cells) {
        card.top_left = Point { x: x as f64, y: y as f64 };
    }
    true
}

fn key(cell: Point) -> (i64, i64) {
    (cell.x as i64, cell.y as i64)
}

/// TALA `sizelessOptimizer`: which card is in which cell.
pub struct Sizeless {
    occupied: HashMap<(i64, i64), usize>,
}

impl Sizeless {
    pub fn new(graph: &Graph) -> Self {
        let mut optimizer = Sizeless { occupied: HashMap::new() };
        optimizer.reset_occupied(graph);
        optimizer
    }

    /// TALA `resetOccupied`: after the cards were moved by something else.
    pub fn reset_occupied(&mut self, graph: &Graph) {
        self.occupied = graph.nodes.iter().enumerate().map(|(node, card)| (key(card.top_left), node)).collect();
    }

    fn is_occupied(&self, cell: Point) -> bool {
        self.occupied.contains_key(&key(cell))
    }

    /// TALA `medianPointGuarded`: the cell a card is drawn to, off by up to `temp` cells either
    /// way, at random.
    fn median_point(graph: &Graph, node: usize, temp: f64, random: &mut GoRand) -> Point {
        let (mut x, mut y) = median_to_neighbors(graph, node, None);
        x += -temp + random.float64() * (2.0 * temp);
        y += -temp + random.float64() * (2.0 * temp);
        Point { x: x.round(), y: y.round() }
    }

    /// TALA `placementPointsGuarded`: the free cells up to one ring beyond the nearest free one.
    fn placement_points(&self, median: Point, nearest_free: f64) -> Vec<Point> {
        let reach = nearest_free + 1.0;
        let mut points = Vec::new();
        let mut x = reach;
        while x >= -reach {
            let mut y = 0.0;
            while y <= reach - x.abs() {
                let above = Point { x: median.x + x, y: median.y - y };
                if y != 0.0 && !self.is_occupied(above) {
                    points.push(above);
                }
                let below = Point { x: median.x + x, y: median.y + y };
                if !self.is_occupied(below) {
                    points.push(below);
                }
                y += 1.0;
            }
            x -= 1.0;
        }
        points
    }

    /// TALA `bestSwapCandidateGuarded`: the card in a cell beside, above or below, with which a
    /// swap makes this card's connections shorter and the other's no longer.
    fn best_swap_candidate(&self, graph: &mut Graph, node: usize, random: &mut GoRand) -> Option<usize> {
        let mut least = edge_length(graph, node, None);
        let home = graph.nodes[node].top_left;
        // Above, below, left, right. TALA leaves out a card without a connection; in a group of
        // connected cards there is none.
        let mut candidates: Vec<usize> = [(0.0, -1.0), (0.0, 1.0), (-1.0, 0.0), (1.0, 0.0)]
            .into_iter()
            .filter_map(|(dx, dy)| self.occupied.get(&key(Point { x: home.x + dx, y: home.y + dy })).copied())
            .collect();
        random.shuffle(&mut candidates);
        let mut best = None;
        for candidate in candidates {
            let current_other = edge_length(graph, candidate, None);
            let other = graph.nodes[candidate].top_left;
            (graph.nodes[node].top_left, graph.nodes[candidate].top_left) = (other, home);
            let swapped = edge_length(graph, node, None);
            let mut swapped_other = 0.0;
            if precision_compare(swapped, least) < 0 {
                swapped_other = edge_length(graph, candidate, None);
            }
            (graph.nodes[node].top_left, graph.nodes[candidate].top_left) = (home, other);
            if precision_compare(swapped, least) < 0 && precision_compare(swapped_other, current_other) <= 0 {
                least = swapped;
                best = Some(candidate);
            }
        }
        best
    }

    /// TALA `sizelessOptimizer.optimize`: one pass over the cards that have a connection, in a
    /// random order. Each moves to the free cell near its median point where its connections
    /// are shortest; one that stays swaps with a neighbour where that helps.
    pub fn optimize(&mut self, graph: &mut Graph, temp: f64, random: &mut GoRand) -> Result<(), NoFreeCell> {
        let mut order: Vec<usize> = (0..graph.nodes.len()).filter(|node| graph.edges_of(*node).next().is_some()).collect();
        random.shuffle(&mut order);
        for node in order {
            let home = graph.nodes[node].top_left;
            // The card may always go back to its own cell.
            self.occupied.remove(&key(home));
            let median = Self::median_point(graph, node, temp, random);
            let ring = closest_unoccupied(median, |cell| self.is_occupied(cell)).ok_or(NoFreeCell { node })?;
            let mut points = self.placement_points(median, ring);
            random.shuffle(&mut points);

            let mut least = f64::INFINITY;
            let mut best = home;
            for point in points {
                graph.nodes[node].top_left = point;
                let length = edge_length(graph, node, None);
                match precision_compare(length, least) {
                    -1 => (least, best) = (length, point),
                    0 if point == home => (least, best) = (length, point),
                    _ => {}
                }
            }
            graph.nodes[node].top_left = best;

            if best == home
                && let Some(other) = self.best_swap_candidate(graph, node, random)
            {
                let place = graph.nodes[other].top_left;
                (graph.nodes[node].top_left, graph.nodes[other].top_left) = (place, home);
                self.occupied.insert(key(home), other);
            }
            self.occupied.insert(key(graph.nodes[node].top_left), node);
        }
        Ok(())
    }
}
