// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.

//! The stage that takes the jitter out of a route: a route that leaves a card, steps aside by a
//! little and goes on (two bends close together) becomes straight when the card moves by that
//! little. The stage runs after the first routing and moves the card, with the ends of all its
//! routes. Where it moved a card, the pipeline routes all connections again.
//! Ported from TALA `internal/placement`: `dejitter.go` (`Dejitter`). For plain cards, trees
//! and clusters: no containers, sequences or fixed cards.

use crate::graph::{Graph, Point};
use crate::optimizer::Allowed;
use crate::placementcost::symmetry::node_symmetry;
use crate::routing::geometry::euclidean_distance;

/// TALA `jitterThreshold`: two bends further apart than this are no jitter.
const JITTER_THRESHOLD: f64 = 80.0;
/// TALA `signFlipPadding`: a piece may not get shorter than this either.
const SIGN_FLIP_PADDING: f64 = 5.0;

/// Where in a route the points are that count at a card: the point on the card, the first
/// bend, the second and the third, seen from the card.
fn from_card(graph: &Graph, routes: &[Vec<Point>], edge: usize, card: usize) -> [usize; 4] {
    let last = routes[edge].len().wrapping_sub(1);
    if graph.edges[edge].from == card { [0, 1, 2, 3] } else { [last, last.wrapping_sub(1), last.wrapping_sub(2), last.wrapping_sub(3)] }
}

/// The first piece of the route at the card is upright.
fn leaves_upright(graph: &Graph, routes: &[Vec<Point>], edge: usize, card: usize) -> bool {
    let [on_card, bend, _, _] = from_card(graph, routes, edge, card);
    routes[edge][on_card].x == routes[edge][bend].x
}

/// A route of one slanted line: its end moves with the card and nothing else is asked.
fn is_slanted(route: &[Point]) -> bool {
    route.len() == 2 && route[0].x != route[1].x && route[0].y != route[1].y
}

/// How many pieces of routes of other cards run through the card.
fn passing(graph: &Graph, routes: &[Vec<Point>], card: usize) -> usize {
    routes.iter().enumerate().filter(|(edge, _)| graph.edges[*edge].from != card && graph.edges[*edge].to != card).map(|(_, route)| route.windows(2).filter(|piece| graph.nodes[card].passes_through(piece[0], piece[1])).count()).sum()
}

/// TALA `Dejitter`. Says whether a card moved.
pub fn dejitter(graph: &mut Graph, routes: &mut [Vec<Point>]) -> bool {
    let mut dejittered = false;
    // What TALA's one transaction for the stage refuses: the cards too close or covering each
    // other at its start may stay so, also after a card moved (`existingOverlaps`).
    let allowed = Allowed::from(graph);
    for card in 0..graph.nodes.len() {
        // A card of a tree or of a cluster stays.
        if graph.tree_link(card).is_some() || graph.cluster_of(card).is_some() {
            continue;
        }
        let symmetry_before = node_symmetry(graph, card);
        let own: Vec<usize> = graph.indexed_edges_of(card).map(|(edge, _)| edge).collect();
        for &edge in &own {
            // No jitter with fewer than three pieces, and none towards a card of a cluster.
            if routes[edge].len() < 4 || graph.cluster_of(graph.edges[edge].other(card)).is_some() {
                continue;
            }
            let [on_card, first, second, before] = from_card(graph, routes, edge, card);
            let (on_card, bends, before) = (routes[edge][on_card], [routes[edge][first], routes[edge][second]], routes[edge][before]);
            if euclidean_distance(bends[0].x, bends[0].y, bends[1].x, bends[1].y) > JITTER_THRESHOLD {
                continue;
            }
            // The piece between the two bends is level: the card moves across to make the piece
            // at the card and the piece after the bends one upright line.
            let across = bends[0].y == bends[1].y;
            let along = |point: Point| if across { point.y } else { point.x };
            let aside = |point: Point| if across { point.x } else { point.y };
            // Not a turn back: the route goes on beyond the bends on the side away from the card.
            if (along(before) > along(bends[0]) && along(on_card) > along(bends[0])) || (along(before) < along(bends[0]) && along(on_card) < along(bends[0])) {
                continue;
            }
            if aside(bends[1]) == aside(bends[0]) {
                continue;
            }
            // Another connection of the card that is one straight line the same way would have
            // to move its other card too.
            let held = own.iter().filter(|other| **other != edge && routes[**other].len() == 2).any(|other| {
                let [end, next, _, _] = from_card(graph, routes, *other, card);
                aside(routes[*other][end]) == aside(routes[*other][next])
            });
            if held {
                continue;
            }
            let delta = (aside(bends[1]) - aside(bends[0])).round();
            // A piece that would get very short counts as turned round too.
            let flip_delta = if delta < 0.0 { delta - SIGN_FLIP_PADDING } else { delta + SIGN_FLIP_PADDING };
            let moved = |point: Point| if across { Point { x: point.x + delta, y: point.y } } else { Point { x: point.x, y: point.y + delta } };
            // The first piece of every route of the card as it would be, and whether a piece
            // turns round: the one after the first bend where the first piece moves as a
            // whole, the first piece itself where only its end at the card moves.
            let mut new_pieces: Vec<(usize, Point, Point)> = Vec::new();
            let mut flips = false;
            for &other in &own {
                if is_slanted(&routes[other]) {
                    continue;
                }
                let [end, next, third, _] = from_card(graph, routes, other, card);
                let route = &routes[other];
                let whole = leaves_upright(graph, routes, other, card) == across;
                let turns = |from: Point, to: Point| (aside(from) + flip_delta < aside(to) && aside(from) > aside(to)) || (aside(from) + flip_delta > aside(to) && aside(from) < aside(to));
                if whole {
                    new_pieces.push((other, moved(route[end]), moved(route[next])));
                    if other != edge && turns(route[next], route[third]) {
                        flips = true;
                    }
                } else {
                    new_pieces.push((other, moved(route[end]), route[next]));
                    if turns(route[end], route[next]) {
                        flips = true;
                    }
                }
                if flips {
                    break;
                }
            }
            if flips {
                continue;
            }
            // No new first piece may run through a card that is not one of its own two.
            let through = new_pieces.iter().any(|(other, from, to)| {
                graph.nodes.iter().enumerate().any(|(node, at)| node != card && node != graph.edges[*other].from && node != graph.edges[*other].to && at.passes_through(*from, *to))
            });
            if through {
                continue;
            }
            // The card moves, unless routes of other cards then run through it more often, it
            // is less symmetric than before, or the move brings two cards too close.
            let passing_before = passing(graph, routes, card);
            let place = graph.nodes[card].top_left;
            graph.nodes[card].top_left = moved(place);
            if passing(graph, routes, card) > passing_before || node_symmetry(graph, card) < symmetry_before || !allowed.now(graph) {
                graph.nodes[card].top_left = place;
                continue;
            }
            for &other in &own {
                let [end, next, _, _] = from_card(graph, routes, other, card);
                let whole = !is_slanted(&routes[other]) && leaves_upright(graph, routes, other, card) == across;
                routes[other][end] = moved(routes[other][end]);
                if whole {
                    routes[other][next] = moved(routes[other][next]);
                }
            }
            // The two bends are gone.
            let [_, first, second, _] = from_card(graph, routes, edge, card);
            routes[edge].drain(first.min(second)..=first.max(second));
            dejittered = true;
        }
    }
    dejittered
}
