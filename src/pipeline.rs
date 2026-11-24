// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.

//! The stages of TALA's pipeline in their order, from a diagram to its finished drawing, for
//! one seed. The order is TALA's (`internal/engine/pipeline.go`, `pipelineStages`); every
//! stage is ported in its own file. For diagrams of plain cards, trees, hierarchies and
//! clusters. `layout_best` lays the diagram out with TALA's three seeds and keeps the best
//! drawing by TALA's score (`crate::score`).

use crate::align::align;
use crate::balance::balance_symmetry;
use crate::cluster_stage::optimize_clusters;
use crate::equidistance::equidistance;
use crate::gaps::normalize_gaps;
use crate::graph::{Graph, Point};
use crate::node_placement::PlaceError;
use crate::pack::pack;
use crate::place::{cleanup, normalize, pad, place_with_trees, prescale};
use crate::routing::coordinator::route_stages;
use crate::routing::labels::{LabelSize, Placed};
use crate::routing::router::RouteError;
use crate::score::{Score, best_seed, evaluate};
use crate::swap::{swap, transpose_all};
use crate::trees::find_trees;

/// A finished drawing.
pub struct Drawing {
    /// The cards at their places, in the order TALA's graph has them, with the connections
    /// in its order too. `Graph::id` gives the place a card has in the diagram.
    pub graph: Graph,
    /// Per connection of `graph` the points of its route.
    pub routes: Vec<Vec<Point>>,
    /// Per connection of `graph` the place of its label, if it has one.
    pub labels: Vec<Option<Placed>>,
}

/// Why a diagram got no drawing.
#[derive(Debug)]
pub enum LayoutError {
    Place(PlaceError),
    Route(RouteError),
}

/// TALA's pipeline for one seed. A connection has a label where its `min_width` is more than 0.
pub fn layout(given: &Graph, seed: i64) -> Result<Drawing, LayoutError> {
    let mut whole = given.clone();
    prescale(&mut whole);
    let found = find_trees(&whole);
    let mut placed = place_with_trees(&mut whole, &found, seed).map_err(LayoutError::Place)?;
    swap(&mut placed);
    transpose_all(&mut placed);
    align(&mut placed);
    if normalize_gaps(&mut placed) {
        align(&mut placed);
    }
    if optimize_clusters(&mut placed) {
        align(&mut placed);
    }
    balance_symmetry(&mut placed);
    if equidistance(&mut placed) {
        align(&mut placed);
    }
    pack(&mut placed);
    pad(&mut placed);
    // The routing stages take the cards as TALA's graph has them now: the cards of the
    // clusters back in it, with the numbers they have in the diagram, and what the placement
    // knows of trees, hierarchies and clusters.
    let mut graph = cleanup(&placed);
    let sizes: Vec<Option<LabelSize>> = graph.edges.iter().map(|edge| (edge.min_width > 0.0).then_some(LabelSize { width: edge.min_width, height: edge.min_height })).collect();
    let staged = route_stages(&mut graph, &sizes).map_err(LayoutError::Route)?;
    let (mut routes, mut labels) = (staged.routing.routes, staged.routing.labels);
    normalize(&mut graph, &mut routes, &mut labels, &sizes);
    Ok(Drawing { graph, routes, labels })
}

/// The seeds TALA's `Layout` tries when nothing else is asked for (`DefaultOptions`).
pub const SEEDS: [i64; 3] = [1, 2, 3];

/// The drawing TALA keeps of those of its seeds.
pub struct Best {
    pub drawing: Drawing,
    pub seed: i64,
    pub score: Score,
}

/// TALA `Layout`: the pipeline for each of the seeds, and of the drawings the one with the
/// lowest score (`score::best_seed`; of equal ones the later seed's). A seed that gives no
/// drawing is left out; if none gives one, the error of the first.
pub fn layout_best(given: &Graph, seeds: &[i64]) -> Result<Best, LayoutError> {
    let mut drawings = Vec::new();
    let mut first_error = None;
    for &seed in seeds {
        match layout(given, seed) {
            Ok(drawing) => {
                let score = evaluate(&drawing.graph, &drawing.routes, &drawing.labels);
                drawings.push(Best { drawing, seed, score });
            }
            Err(error) => {
                first_error.get_or_insert(error);
            }
        }
    }
    let scores: Vec<Score> = drawings.iter().map(|best| best.score).collect();
    match best_seed(&scores) {
        Some(index) => Ok(drawings.swap_remove(index)),
        None => Err(first_error.expect("at least one seed")),
    }
}
