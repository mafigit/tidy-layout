// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.

//! A layout engine for diagrams of cards and connections: a Rust port of parts of TALA, the layout
//! engine of D2. See NOTICE.md for where each part comes from.

pub mod align;
pub mod api;
pub mod balance;
pub mod cluster_stage;
pub mod clusters;
pub mod compaction;
pub mod dejitter;
pub mod equidistance;
pub mod gaps;
pub mod gorand;
pub mod gosort;
pub mod graph;
pub mod hierarchy;
pub mod hierarchy_layout;
pub mod join;
pub mod node_placement;
pub mod optimizer;
pub mod pack;
pub mod pipeline;
pub mod place;
pub mod placement;
pub mod placementcost;
pub mod policy;
pub mod quality;
pub mod routing;
pub mod score;
pub mod sizeless;
pub mod swap;
mod tree_layout;
pub mod trees;
#[cfg(target_arch = "wasm32")]
mod wasm;
