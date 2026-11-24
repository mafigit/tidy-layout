// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.

//! Routing connections around cards. Ported from TALA `internal/routing`, piece by piece
//! (docs/router-port.md says which pieces exist).

pub mod balance;
pub mod cluster_branching;
pub mod coordinator;
pub mod crosshatch;
pub mod geometry;
pub mod labels;
pub mod line;
pub mod nudge;
pub mod ovg;
mod ovg_hierarchy;
pub mod router;
pub mod shortcut;
pub mod simplify;
mod slingshot;
pub mod swap_ports;
mod tree_routes;
pub mod tuning;
mod tunnel;
