// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.

//! Shared geometry rules. Ported from TALA `internal/layoutgraph/geometry_policy.go` and
//! `internal/layoutgraph/graph.go`.

/// The minimum gap between cards that are not connected.
pub const NODE_GAP: f64 = 20.0;
/// The minimum gap between connected cards: room for the connector and its label.
pub const CONNECTED_NODE_GAP: f64 = 60.0;
/// The space around a label inside the room a connection keeps for it. From D2 `lib/label`
/// (`PADDING`), as TALA's adapter applies it on both sides.
pub const LABEL_PADDING: f64 = 5.0;
/// The smallest cell of the placement grid.
pub const MIN_CELL_SIZE: f64 = 10.0;
/// How many rings around a point the search for a free place looks at.
pub const MAX_SEARCH_RINGS: u32 = 100;
