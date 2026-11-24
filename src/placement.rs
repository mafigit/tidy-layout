// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.

//! Finding a free place for a card. Ported from TALA `internal/placement`:
//! `sizedOptimizer.findClosestUnoccupiedDistanceGuarded`, `findUnoccupiedGuarded` and
//! `isPointOccupiedGuarded` (sized_optimizer.go), `optimizerIsOccupied` and
//! `optimizerDoesOverlap` (optimization_guard.go), `roundToNearestCellSize` (types.go).
//!
//! And one move of a card as the sized optimizer makes it: `fillPlacementPointsGuarded` with
//! `iterPlacementsAroundPoint` (the places it looks at) and `moveNodeToBestGuarded` (the place
//! it takes), with `indexedCanMove` (optimizer_spatial_index.go) and D2 `geo.PrecisionCompare`.
//!
//! `minimizingSelf` is on when the optimizer moves a card, off when the editor asks for a place.

use crate::graph::{Graph, Point};
use crate::placementcost::{edge_length, symmetry::node_symmetry};
use crate::policy::MAX_SEARCH_RINGS;
use crate::routing::tuning::PRECISION;
use std::collections::HashSet;

/// A free place found by [`closest_unoccupied`].
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct FreePlace {
    /// How many rings away from the wanted point it is; 0 is the point itself.
    pub ring: u32,
    /// The top-left corner for the card.
    pub top_left: Point,
}

pub fn round_to_cell(value: f64, cell_size: f64) -> f64 {
    (value / cell_size).round() * cell_size
}

/// Is the top-left corner of any card exactly here? This counts the moving card too, as in TALA.
fn is_occupied(graph: &Graph, point: Point) -> bool {
    graph.nodes.iter().any(|n| n.top_left == point)
}

/// Would the card, with its top-left corner at `point`, come closer to another card than the
/// gap the two must keep?
pub fn does_overlap(graph: &Graph, moving: usize, point: Point) -> bool {
    let node = &graph.nodes[moving];
    let right = point.x + node.width;
    let bottom = point.y + node.height;
    graph.nodes.iter().enumerate().any(|(index, other)| {
        if index == moving {
            return false;
        }
        let delta = graph.gap_between(moving, index, point);
        point.x < other.top_left.x + other.width + delta
            && right + delta > other.top_left.x
            && point.y < other.top_left.y + other.height + delta
            && bottom + delta > other.top_left.y
    })
}

/// The candidate at the offset from `p`, on the grid, if the card can go there.
fn free_at(graph: &Graph, moving: usize, p: Point, dx: f64, dy: f64) -> Option<Point> {
    let point = Point {
        x: round_to_cell(p.x + dx, graph.cell_size),
        y: round_to_cell(p.y + dy, graph.cell_size),
    };
    (!is_occupied(graph, point) && !does_overlap(graph, moving, point)).then_some(point)
}

/// The offsets from a point that were tried already. TALA `checkedPositions`.
pub type Checked = HashSet<(u64, u64)>;

/// TALA `findUnoccupiedGuarded`: the free place at the offset from `p`. With `minimizing_self`
/// also the places up to the card's width to the left of it and up to its height above it.
/// An offset in `checked` is not tried again: it counts as taken.
fn find_unoccupied(
    graph: &Graph,
    moving: usize,
    p: Point,
    (x, y): (f64, f64),
    minimizing_self: bool,
    checked: &mut Option<&mut Checked>,
) -> Option<Point> {
    let mut first_time = |x: f64, y: f64| checked.as_mut().is_none_or(|checked| checked.insert((x.to_bits(), y.to_bits())));
    if first_time(x, y)
        && let Some(place) = free_at(graph, moving, p, x, y)
    {
        return Some(place);
    }
    if !minimizing_self {
        return None;
    }
    let (node, cell) = (&graph.nodes[moving], graph.cell_size);
    let mut i = x - node.width;
    while i <= x {
        let mut j = y - node.height;
        while j <= y {
            // TALA skips the offset itself here; it was found taken above.
            if first_time(i, j)
                && let Some(place) = free_at(graph, moving, p, i, j)
            {
                return Some(place);
            }
            j += cell;
        }
        i += cell;
    }
    None
}

/// TALA `findClosestUnoccupiedDistanceGuarded`: the ring of the nearest free place and the place.
fn search(
    graph: &Graph,
    moving: usize,
    p: Point,
    minimizing_self: bool,
    mut checked: Option<&mut Checked>,
) -> Option<FreePlace> {
    let cell = graph.cell_size;
    for ring in 0..=MAX_SEARCH_RINGS {
        let reach = f64::from(ring) * cell;
        let mut x = reach;
        let mut y = 0.0;
        while x >= -reach {
            if y != 0.0
                && let Some(top_left) = find_unoccupied(graph, moving, p, (x, -y), minimizing_self, &mut checked)
            {
                return Some(FreePlace { ring, top_left });
            }
            if let Some(top_left) = find_unoccupied(graph, moving, p, (x, y), minimizing_self, &mut checked) {
                return Some(FreePlace { ring, top_left });
            }
            if x > 0.0 {
                y += cell;
            } else {
                y -= cell;
            }
            x -= cell;
        }
    }
    None
}

/// The free place nearest to `p` for the card `moving`, looking outward ring by ring:
///
/// ```text
///          3
///       3  2  3
///    3  2  1  2  3
/// 3  2  1  0  1  2  3
///    3  2  1  2  3
///       3  2  3
///          3
/// ```
///
/// On one ring the candidates are tried from right to left, above the point before below it.
/// `None` when nothing is free within [`MAX_SEARCH_RINGS`] rings.
pub fn closest_unoccupied(graph: &Graph, moving: usize, p: Point) -> Option<FreePlace> {
    search(graph, moving, p, false, None)
}

/// The ring of the nearest free place as the optimizer looks for it: with `minimizing_self` a
/// ring counts as free also when the card fits a little to the left of or above a place on it.
pub fn closest_unoccupied_ring(graph: &Graph, moving: usize, p: Point, minimizing_self: bool, checked: Option<&mut Checked>) -> Option<u32> {
    search(graph, moving, p, minimizing_self, checked).map(|free| free.ring)
}

/// TALA `fillPlacementPointsGuarded`: every place on the grid up to one ring further from
/// `median` than the nearest free one (`nearest_free` rings away), column by column from the
/// right, in each column outward from the middle, above before below. With `minimizing_self`
/// each of them is followed by the places up to the card's width to the left of it and up to
/// its height above it.
///
/// A card that shares three or more connections with another card, with a label too large for
/// a cell, also looks at the places beside, above and below each card it is connected to, at
/// the distance the largest label between the two needs (TALA `newSizedOptimizer`,
/// `LongDistanceNeighborRequirements`).
pub fn fill_placement_points(
    graph: &Graph,
    moving: usize,
    median: Point,
    nearest_free: u32,
    minimizing_self: bool,
) -> Vec<Point> {
    let cell = graph.cell_size;
    let node = &graph.nodes[moving];
    let reach = f64::from(nearest_free) + 1.0;
    let mut seen = HashSet::new();
    let mut points = Vec::new();
    let mut add = |x: f64, y: f64| {
        let point = Point { x: round_to_cell(median.x + x, cell), y: round_to_cell(median.y + y, cell) };
        // Far from the origin two places can round to the same cell.
        if seen.insert(((point.x / cell) as i64 as u32, (point.y / cell) as i64 as u32)) {
            points.push(point);
        }
    };
    // TALA `iterPlacementsAroundPoint`.
    let mut around = |x: f64, y: f64| {
        let (x, y) = (x * cell, y * cell);
        add(x, y);
        if !minimizing_self {
            return;
        }
        let mut current_x = x - node.width;
        while current_x <= x {
            let mut current_y = y - node.height;
            while current_y <= y {
                // TALA skips the place itself here; `add` takes no place twice.
                add(current_x, current_y);
                current_y += cell;
            }
            current_x += cell;
        }
    };
    let mut x = reach;
    while x >= -reach {
        let mut y = 0.0;
        while y <= reach - x.abs() {
            if y != 0.0 {
                around(x, -y);
            }
            around(x, y);
            y += 1.0;
        }
        x -= 1.0;
    }

    // Per connected card, in the order of the cards: connections, widest and highest label room.
    let mut rooms: Vec<(usize, u32, f64, f64)> = Vec::new();
    // TALA works this out once, before any connection is hidden.
    for (index, edge) in graph.edges.iter().enumerate().filter(|(_, edge)| edge.from == moving || edge.to == moving) {
        // Not a connection of a cluster's card (TALA `abductedEdges`).
        if graph.behind.get(index).copied().flatten().is_some() {
            continue;
        }
        let adjacent = edge.other(moving);
        let at = rooms.iter().position(|room| room.0 == adjacent).unwrap_or_else(|| {
            rooms.push((adjacent, 0, 0.0, 0.0));
            rooms.len() - 1
        });
        rooms[at].1 += 1;
        rooms[at].2 = rooms[at].2.max(edge.min_width);
        rooms[at].3 = rooms[at].3.max(edge.min_height);
    }
    rooms.sort_by_key(|room| graph.id(room.0));
    if rooms.iter().any(|(_, count, width, height)| *count >= 3 && (*width > cell || *height > cell)) {
        for (adjacent, _, width, height) in rooms {
            let adjacent = &graph.nodes[adjacent];
            let (x, y) = (adjacent.top_left.x - median.x, adjacent.top_left.y - median.y);
            if width > cell {
                add(((x - node.width - width) / cell).floor() * cell, y);
                add(((x + adjacent.width + width) / cell).ceil() * cell, y);
            }
            if height > cell {
                add(x, ((y - node.height - height) / cell).floor() * cell);
                add(x, ((y + adjacent.height + height) / cell).ceil() * cell);
            }
        }
    }
    points
}

/// D2 `geo.PrecisionCompare`: -1, 0 or 1; equal when closer than a rounding error.
pub fn precision_compare(a: f64, b: f64) -> i32 {
    if (a - b).abs() < PRECISION {
        0
    } else if a < b {
        -1
    } else {
        1
    }
}

/// TALA `moveNodeToBestGuarded`: moves the card to the cheapest of the places it can go to and
/// says where that is. The price of a place is the length of the card's connections there
/// (`placementcost::edge_length`), less a cell per connection when the card lies symmetric
/// (`placementcost::symmetry`). Among places of the same price the first wins, the card's own
/// place before any other. With `must_improve` the card moves only to a cheaper place than its
/// own. `None`, and the card stays, when it can go to none of the places. `turn` is the price
/// of a bend: TALA keeps the one from before the optimizer started.
pub fn move_node_to_best(
    graph: &mut Graph,
    moving: usize,
    points: &[Point],
    must_improve: bool,
    turn: f64,
) -> Option<Point> {
    let home = graph.nodes[moving].top_left;
    let setup = edge_length::prepare(graph, moving);
    let symmetry_cost = graph.cell_size * graph.edges_of(moving).count() as f64;
    let mut least = f64::INFINITY;
    if must_improve {
        least = edge_length::evaluate(graph, moving, turn, &setup) - node_symmetry(graph, moving) * symmetry_cost;
    }
    let mut best = home;
    for &point in points {
        // TALA also asks whether another card has its corner exactly here; such a card overlaps.
        if point != home && does_overlap(graph, moving, point) {
            continue;
        }
        graph.nodes[moving].top_left = point;
        // TALA skips the symmetry of a place that full symmetry could not make the cheapest.
        let price = edge_length::evaluate(graph, moving, turn, &setup) - node_symmetry(graph, moving) * symmetry_cost;
        match precision_compare(price, least) {
            -1 => (least, best) = (price, point),
            0 if point == home => (least, best) = (price, point),
            _ => {}
        }
    }
    graph.nodes[moving].top_left = if least == f64::INFINITY { home } else { best };
    (least != f64::INFINITY).then_some(best)
}
