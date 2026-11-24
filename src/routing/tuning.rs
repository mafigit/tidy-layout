// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.

//! The router's numbers. Ported from TALA `internal/routing/tuning.go`,
//! `internal/layoutgraph/geometry_policy.go` and `graph.go`, and D2 `lib/geo`.

/// How many rows of extra points the graph gets outside the cards, so routes can go around.
pub const EXTRA_POINT_LAYERS: u32 = 3;
/// The distance between those rows.
pub const OVG_PADDING: f64 = 20.0;
/// How far outside two connected cards their shared outline of points lies.
pub const OVERSHOOT: f64 = 100.0;
/// Points closer than this to a port, on its line, are marked as near it.
pub const NODE_PROXIMITY_THRESHOLD: f64 = 30.0;
/// The room one route needs next to another: decides how many tunnels fit between two cards.
pub const SEGMENT_SPACING_BUFFER: f64 = 40.0;
/// The shortest piece an arrowhead fits on.
pub const MIN_ARROWHEAD_CLEARANCE: f64 = 20.0;
/// A straight line that is neither upright nor level costs this many times its length.
pub const NON_ORTHOGONAL_FACTOR: f64 = 4.0;
/// A jog this short does not count against a straight line that would replace it.
pub const LOWER_JITTER_THRESHOLD: f64 = 5.0;
/// A straight line meets the side of a card at no flatter an angle than this, in degrees.
pub const MIN_STRAIGHT_EDGE_ANGLE: f64 = 30.0;
/// Two coordinates closer than this count as lined up.
pub const AXIS_ALIGNMENT_TOLERANCE: f64 = 1.0;
/// How far a piece of a route that nothing bounds may be moved either way.
pub const UNBOUNDED_SEGMENT_MOVE: f64 = 100.0;
/// A route's first bend stays this far from its port.
pub const MIN_PORT_CLEARANCE: f64 = 20.0;
/// Points of the graph stay this far from every card.
pub const MIN_ROUTE_NODE_CLEARANCE: f64 = 20.0;

/// A price no route pays unless there is no other way.
pub const BASICALLY_INFINITY: f64 = 10_000_000.0;
/// A route closer than this to an edge of the graph, on a parallel line, is "nearby".
pub const PATH_NODE_PROXIMITY_FLOOR: f64 = 5.0;
/// How close to a line halfway between two cards a turn counts as being on it.
pub const IDEAL_TURN_AXIS_TOLERANCE: f64 = 4.0;
/// A turn on such a line costs this share of a turn.
pub const IDEAL_TURN_MULTIPLIER: f64 = 0.98;
/// TALA `idealTurnEvenClusterMultiplier`.
pub const IDEAL_TURN_EVEN_CLUSTER_MULTIPLIER: f64 = 0.10;
/// The price for passing a point that is near a port of one of the two cards.
pub const NODE_PROXIMITY_PENALTY: f64 = 1.0;
/// A turn this close to one of the two cards costs one more turn.
pub const TURN_ENDPOINT_CLEARANCE: f64 = 20.0;
/// The three prices are these shares of (number of connections times the longest one).
pub const TURN_PENALTY_MULTIPLIER: f64 = 0.125;
pub const CENTER_PORT_MULTIPLIER: f64 = 0.04287499999999999;
/// 0.48 cubed, as TALA writes it.
pub const CROSSING_COST_WEIGHT: f64 = 0.110592;
/// D2 `geo.PRECISION`: prices closer than this count as equal.
pub const PRECISION: f64 = 0.0001;
