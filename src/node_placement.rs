// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.

//! Node placement for one connected group of cards: from nothing to a place for every card.
//! Ported from TALA `internal/placement/node_placement.go` (`placeNodesOrthogonally`), which
//! follows "Graph Compact Orthogonal Layout Algorithm" by Freivalds and Glagolevs:
//!
//! 1. a first arrangement on a coarse grid, one card per cell;
//! 2. the sizeless optimizer, cooling down, with compaction every ninth round;
//! 3. the change to real sizes: compaction makes room along both axes, cards go onto the grid;
//! 4. the sized optimizer, cooling down further, with compaction every ninth round, after which
//!    groups of cards that drifted far apart are pulled together;
//! 5. up to ten passes without temperature, until one changes nothing.
//!
//! For plain cards in no container, all connected. Not ported: fixed places, clusters, herds.

use crate::compaction::{Options, compaction};
use crate::gorand::GoRand;
use crate::graph::Graph;
use crate::join::join_distanced_clusters;
use crate::optimizer::optimize;
use crate::routing::router::Costs;
use crate::sizeless::{Sizeless, first_arrangement, first_arrangement_by_distance};

/// How far node placement has come. For those who watch it.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Stage {
    /// The first arrangement, in cells; `true` for the one by graph distance.
    First(bool),
    /// A round of the sizeless loop: after the optimizer, or after the compaction that follows.
    SizelessStep,
    /// The end of the sizeless loop, in cells.
    Sizeless,
    /// After the change to real sizes, in pixels from here on.
    Inflated,
    /// On the grid. With the price of a bend that the sized optimizer uses.
    OnGrid(f64),
    /// A round of the sized loop: after the optimizer, or after the compaction that follows.
    SizedStep,
    /// The end of the sized loop.
    Sized,
    /// A pass without temperature.
    ColdStep,
    Final,
}

/// Why node placement gave up. TALA reports an error in both cases.
#[derive(Debug, PartialEq)]
pub enum PlaceError {
    /// A card cannot be reached from the first one, or found no cell to start in.
    NotArranged,
    /// A card found no free place within 100 rings of the point it is drawn to.
    NoFreePlace { node: usize },
}

const ITERATIONS: f64 = 90.0;
const COMPACTION_FACTOR: f64 = 3.0;
/// Compaction runs every ninth round, along x and y in turn.
const COMPACTION_ROUND: usize = 9;

/// TALA `placeNodesOrthogonally`. `seed` is the one the random numbers were made from: an even
/// one starts from the arrangement by graph distance where that applies. `watch` is told after
/// every stage.
pub fn place_nodes_orthogonally(
    graph: &mut Graph,
    seed: i64,
    random: &mut GoRand,
    mut watch: impl FnMut(Stage, &Graph, &GoRand),
) -> Result<(), PlaceError> {
    let by_distance = seed % 2 == 0 && first_arrangement_by_distance(graph);
    if !by_distance && !first_arrangement(graph) {
        return Err(PlaceError::NotArranged);
    }
    watch(Stage::First(by_distance), graph, random);

    let count = graph.nodes.len() as f64;
    let iterations = (ITERATIONS * count.sqrt()) as usize;
    let mut temp = 2.0 * count.sqrt();
    // Go's `math.Pow`; its last bit depends on the processor, here it is Rust's.
    let cooling = (0.2 / temp).powf(1.0 / iterations as f64);
    let mut horizontal = true;

    let mut sizeless = Sizeless::new(graph);
    for round in 0..iterations / 2 {
        sizeless.optimize(graph, temp, random).map_err(|error| PlaceError::NoFreePlace { node: error.node })?;
        watch(Stage::SizelessStep, graph, random);
        if round % COMPACTION_ROUND == 0 {
            compaction(graph, &Options { horizontal, sizes: false, factor: COMPACTION_FACTOR, transition: false }, 0.0);
            horizontal = !horizontal;
            sizeless.reset_occupied(graph);
            watch(Stage::SizelessStep, graph, random);
        }
        temp *= cooling;
    }
    watch(Stage::Sizeless, graph, random);

    for horizontal in [true, false] {
        compaction(graph, &Options { horizontal, sizes: true, factor: COMPACTION_FACTOR, transition: true }, 0.0);
    }
    watch(Stage::Inflated, graph, random);
    let cell = graph.cell_size;
    for card in &mut graph.nodes {
        // TALA rounds down only what is off the grid; rounding down leaves the rest as it is.
        card.top_left.x = (card.top_left.x / cell).floor() * cell;
        card.top_left.y = (card.top_left.y / cell).floor() * cell;
    }
    // The price of a bend is taken here, once. The compaction is strong, so the longest
    // connection is expected to end at about half its length.
    let turn = Costs::of(graph).turn / 2.0;
    watch(Stage::OnGrid(turn), graph, random);

    let no_place = |error: crate::optimizer::NoFreePlace| PlaceError::NoFreePlace { node: error.node };
    for round in iterations / 2 + 1..iterations {
        optimize(graph, turn, temp, random).map_err(no_place)?;
        watch(Stage::SizedStep, graph, random);
        if round % COMPACTION_ROUND == 0 {
            let left = iterations as f64 - round as f64 - 30.0;
            let factor = 1.0_f64.max(1.0 + (2.0 * left / (0.5 * iterations as f64)));
            compaction(graph, &Options { horizontal, sizes: true, factor, transition: false }, turn);
            horizontal = !horizontal;
            join_distanced_clusters(graph);
            watch(Stage::SizedStep, graph, random);
        }
        temp *= cooling;
    }
    join_distanced_clusters(graph);
    watch(Stage::Sized, graph, random);

    for _ in 0..10 {
        let changed = optimize(graph, turn, 0.0, random).map_err(no_place)?;
        watch(Stage::ColdStep, graph, random);
        if !changed {
            break;
        }
    }
    watch(Stage::Final, graph, random);
    Ok(())
}
