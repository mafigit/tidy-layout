// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.

//! What a card costs at the place it is: the length of its connections with penalties, less a
//! reward for symmetry. Ported from TALA `internal/placementcost`, for plain cards: no
//! containers, clusters, tables, sequences, "near" wishes or connections from a card to itself,
//! every connection an arrow from its first card to its second, and sizes always counted.
//!
//! TALA skips cards outside the box around the cards in question before it tests them
//! (`obstructionBounds`). A card outside that box cannot be in the way, so the port tests them all.

pub mod edge_length;
pub(crate) mod flow_continuity;
pub mod symmetry;
