// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.

//! The points a hierarchy adds to the routing graph. The cards of a hierarchy stand in levels,
//! one under the other. Between two levels the graph gets rows for the routes to run along,
//! one per connection that comes down, and every level gets points beside its cards, in
//! columns 20 apart, so that a route can pass a level anywhere.
//! Ported from TALA `internal/routing/ovg_hierarchy.go` (`newOVGForHierarchy`,
//! `addHierarchyLevelNodes`, `addBoundaryNodesAboveLevelNodes`, `addPortAlignedNodes`,
//! `addLevelNodes`). For cards in no container, in a diagram without a direction: TALA mirrors
//! and turns a hierarchy that runs up or sideways before it makes the points.

use std::collections::BTreeMap;

use crate::graph::{Graph, Point};
use crate::routing::ovg::{Ovg, Side, add_boundary_layers, add_corners, add_intersections, add_port_intersections, add_ports, points_box};
use crate::routing::tuning::OVG_PADDING;

/// TALA `newOVGForHierarchy`: a graph of the hierarchy's own ports and points. It is merged
/// into the graph of the whole group.
pub(crate) fn build_hierarchy(graph: &Graph, hierarchy: usize) -> Ovg {
    let mut ovg = Ovg::default();
    let level_of = |card: usize| graph.level(card).filter(|(other, _)| *other == hierarchy).map(|(_, level)| level);
    add_ports(&mut ovg, graph, |card| level_of(card).is_some());
    add_port_intersections(&mut ovg, graph);

    // TALA `addHierarchyLevelNodes`. The levels from the top, the cards of each from the left.
    // (Of two at the same place TALA takes the one with the lower number first. Two cards of
    // one level never stand at the same place.)
    let mut by_level: BTreeMap<i64, Vec<usize>> = BTreeMap::new();
    for card in 0..graph.nodes.len() {
        if let Some(level) = level_of(card) {
            by_level.entry(level).or_default().push(card);
        }
    }
    let rows: Vec<Vec<usize>> = by_level
        .into_values()
        .map(|mut cards| {
            cards.sort_by(|a, b| graph.nodes[*a].top_left.x.total_cmp(&graph.nodes[*b].top_left.x));
            cards
        })
        .collect();
    // The columns are 20 apart from a multiple of 20 left of all points, with one column to
    // spare on either side.
    let (tl, br) = points_box(&ovg);
    let left = ((tl.x - OVG_PADDING) / OVG_PADDING).floor() * OVG_PADDING;
    let right = left + ((br.x - left) / OVG_PADDING).ceil() * OVG_PADDING + OVG_PADDING;
    let mut ys: Vec<f64> = Vec::new();
    for level in 1..rows.len() {
        let (above, below) = (&rows[level - 1], &rows[level]);
        // TALA `addBoundaryNodesAboveLevelNodes`: one row per connection that comes down to
        // the level, and one more, as far as rows 20 apart fit between the two levels. Each
        // row gets a point at the far left and one at the far right.
        let top_of_lower = below.iter().map(|card| graph.nodes[*card].top_left.y).fold(f64::INFINITY, f64::min);
        let bottom_of_upper = above.iter().map(|card| graph.nodes[*card].bottom()).fold(f64::NEG_INFINITY, f64::max);
        let mut coming_down = 1.0;
        for &card in below {
            for edge in graph.edges_of(card) {
                let other = edge.other(card);
                if other != card && level_of(other).is_some_and(|level| Some(level) < level_of(card)) {
                    coming_down += 1.0;
                }
            }
        }
        let distance = top_of_lower - bottom_of_upper;
        let lines = f64::min(coming_down, distance / OVG_PADDING);
        // (TALA takes 20 where this is less. It never is: no more rows are asked for than fit
        // at 20 apart.)
        let pad = (distance / lines).ceil();
        let mut level_ys: Vec<f64> = Vec::new();
        let mut y = top_of_lower;
        for _ in 0..(lines as i64 - 1).max(0) {
            y -= pad;
            level_ys.push(y);
            ovg.add_point(Point { x: left, y });
            ovg.add_point(Point { x: right, y });
        }
        // TALA `addPortAlignedNodes`: on every such row a point under each bottom port of the
        // upper level and over each top port of the lower one.
        for &y in &level_ys {
            for (cards, side) in [(above, Side::Bottom), (below, Side::Top)] {
                for &card in cards {
                    for port in ovg.ports_on_side(card, side) {
                        let x = ovg.point(port).x;
                        ovg.add_point(Point { x, y });
                    }
                }
            }
        }
        ys.extend(level_ys);
    }
    // TALA `addLevelNodes`: in every column that passes no card of the level, a point on the
    // row of each port that looks sideways there: left of the first card its left ports,
    // between two cards the right ports of the one and the left ports of the next, right of
    // the last card its right ports.
    let mut xs: Vec<f64> = Vec::new();
    for row in &rows {
        let mut at = 0;
        let mut last_passed = false;
        let mut ports = ovg.ports_on_side(row[0], Side::Left);
        let mut x = left;
        while x <= right {
            if !xs.contains(&x) {
                xs.push(x);
            }
            for port in ports.clone() {
                let y = ovg.point(port).y;
                ovg.add_point(Point { x, y });
            }
            let card = &graph.nodes[row[at]];
            if !last_passed && x >= card.top_left.x - OVG_PADDING {
                x = ((card.top_left.x + card.width) / OVG_PADDING).ceil() * OVG_PADDING;
                ports = ovg.ports_on_side(row[at], Side::Right);
                last_passed = at == row.len() - 1;
                if !last_passed {
                    at += 1;
                    ports.extend(ovg.ports_on_side(row[at], Side::Left));
                }
            }
            x += OVG_PADDING;
        }
    }
    add_intersections(&mut ovg, graph, xs, ys);

    let (tl, br) = points_box(&ovg);
    add_boundary_layers(&mut ovg, graph, tl, br);
    add_corners(&mut ovg, graph, tl, br);
    ovg
}
