// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.

//! TALA's score of its finished drawings and its choice between three seeds, against the
//! recorded diagrams of all five routed fixtures (`score` and `seedScores`, recorded with
//! `STAGES_SEEDS=1`; see `docs/reference.md`).

use tidy_layout::graph::{Edge, Graph, Node, Point};
use tidy_layout::pipeline::{SEEDS, layout, layout_best};
use tidy_layout::score::{Score, best_seed, evaluate};
use serde::Deserialize;

#[derive(Deserialize)]
struct Input {
    nodes: Vec<[f64; 2]>,
    edges: Vec<[usize; 4]>,
    seed: i64,
}

#[derive(Deserialize)]
struct Case {
    case: usize,
    input: Input,
    score: [f64; 2],
    #[serde(rename = "seedScores")]
    seed_scores: Vec<[f64; 2]>,
}

const FIXTURES: [&str; 5] = [
    include_str!("fixtures/routed.tala.json"),
    include_str!("fixtures/routed_trees.tala.json"),
    include_str!("fixtures/routed_hierarchies.tala.json"),
    include_str!("fixtures/routed_clusters.tala.json"),
    include_str!("fixtures/routed_several.tala.json"),
];

fn cases() -> Vec<Case> {
    FIXTURES.iter().flat_map(|fixture| serde_json::from_str::<Vec<Case>>(fixture).unwrap()).collect()
}

/// The diagram as the recorder was given it, the cards anywhere.
fn given(input: &Input) -> Graph {
    let nodes = input.nodes.iter().enumerate().map(|(card, [width, height])| Node { top_left: Point { x: 13.0 * card as f64 + 7.0, y: -31.0 }, width: *width, height: *height }).collect();
    let edges = input.edges.iter().map(|[from, to, width, height]| Edge { from: *from, to: *to, min_width: *width as f64, min_height: *height as f64 }).collect();
    Graph::new(nodes, edges)
}

fn score_of(pair: [f64; 2]) -> Score {
    Score { penalty: pair[0], area: pair[1] }
}

/// The numbers: the diagrams, those with a penalty, those whose labels add to it, those
/// whose penalty is not a whole number.
#[test]
fn the_score_of_a_finished_drawing_is_tala_s() {
    let cases = cases();
    let (mut penalised, mut fractional, mut differ) = (0, 0, Vec::new());
    for case in &cases {
        let drawing = layout(&given(&case.input), case.input.seed).expect("a drawing");
        let score = evaluate(&drawing.graph, &drawing.routes, &drawing.labels);
        if [score.penalty, score.area] != case.score {
            differ.push(format!("case {} (seed {}): {score:?}, TALA {:?}", case.case, case.input.seed, case.score));
        }
        penalised += usize::from(case.score[0] > 0.0);
        fractional += usize::from(case.score[0].fract() != 0.0);
    }
    assert_eq!(differ.len(), 0, "{} differ: {}", differ.len(), differ.join("; "));
    assert_eq!((cases.len(), penalised, fractional), (438, 315, 171));
}

/// Every seed's drawing gets TALA's score, and the pick among seeds 1, 2 and 3 is TALA's. The
/// numbers: the diagrams, those where the seeds score differently, and how often each seed is
/// picked.
#[test]
fn the_best_of_three_seeds_is_tala_s() {
    let cases = cases();
    let (mut differing, mut picked, mut differ) = (0, [0; 3], Vec::new());
    for case in &cases {
        let theirs: Vec<Score> = case.seed_scores.iter().map(|pair| score_of(*pair)).collect();
        for (index, seed) in SEEDS.iter().enumerate() {
            let drawing = layout(&given(&case.input), *seed).expect("a drawing");
            let score = evaluate(&drawing.graph, &drawing.routes, &drawing.labels);
            if score != theirs[index] {
                differ.push(format!("case {} seed {seed}: {score:?}, TALA {:?}", case.case, theirs[index]));
            }
        }
        let best = layout_best(&given(&case.input), &SEEDS).expect("a drawing");
        let index = best_seed(&theirs).expect("three scores");
        if best.seed != SEEDS[index] || best.score != theirs[index] {
            differ.push(format!("case {}: seed {}, TALA's pick {}", case.case, best.seed, SEEDS[index]));
        }
        differing += usize::from(theirs.iter().any(|score| *score != theirs[0]));
        picked[index] += 1;
    }
    assert_eq!(differ.len(), 0, "{} differ: {}", differ.len(), differ.join("; "));
    assert_eq!((cases.len(), differing, picked), (438, 256, [59, 76, 303]));
}
