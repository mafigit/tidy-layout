// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.

//! Finding the hierarchies of a diagram: groups of cards whose arrows flow one way through
//! three or more levels. TALA lays such a group out in layers instead of placing its cards one
//! by one. Ported from TALA `internal/hierarchy`: `eligibility.go` (`Candidates`),
//! `discovery.go` (`Assign`, `build`, `isValid`, `is1N1`, `makeSimpleDAG`, `findCycleEdges`,
//! `removeDuplicateEdges`), `rank.go`, `rank_graph.go` and `rank_simplex.go`.
//!
//! For plain cards in no container, with an arrow at the target of every connection. Laying a
//! hierarchy out (`hierarchy.Place`) is not ported.

use crate::graph::Graph;
use crate::place::split;

const MIN_HIERARCHY_LEVELS: usize = 3;
const MAX_AUTOMATIC_WORKFLOW_NODES: usize = 128;
const MAX_AUTOMATIC_WORKFLOW_EDGES: usize = 256;
/// TALA `crossingSpacing`: the least room between two levels.
const CROSSING_SPACING: f64 = 50.0;
/// TALA `layoutgraph.MinPortClearance`.
const MIN_PORT_CLEARANCE: f64 = 20.0;
/// TALA `limits.MaxGraphSize`.
const MAX_GRAPH_SIZE: f64 = 30_000.0;

/// A connection of a group, between two of its cards by their place in the group, made to
/// point down the levels. TALA numbers the arrows in the order of the group's connections;
/// here a list of arrows is in that order.
#[derive(Clone, Copy, Debug)]
struct Arrow {
    from: usize,
    to: usize,
    /// How many connections it stands for.
    weight: i64,
}

/// TALA `findCycleEdges`: the connections to turn round so that no ring is left. Cards that
/// nothing points at, and cards that point at nothing, are taken off until none is left; then
/// the card with the most arrows out over arrows in gives up the arrows that point at it.
fn cycle_edges(ids: &[i64], arrows: &[Arrow]) -> Vec<usize> {
    let count = ids.len();
    let mut incoming: Vec<Vec<usize>> = vec![Vec::new(); count];
    let mut outgoing: Vec<Vec<usize>> = vec![Vec::new(); count];
    for (index, arrow) in arrows.iter().enumerate() {
        outgoing[arrow.from].push(index);
        incoming[arrow.to].push(index);
    }
    let mut left: Vec<bool> = (0..count).map(|node| !incoming[node].is_empty() || !outgoing[node].is_empty()).collect();
    let mut in_no_ring = vec![false; arrows.len()];

    // TALA `pruneNode`: a card off the graph; its connections are in no ring.
    fn prune(node: usize, arrows: &[Arrow], incoming: &mut [Vec<usize>], outgoing: &mut [Vec<usize>], left: &mut [bool], in_no_ring: &mut [bool]) {
        for index in std::mem::take(&mut incoming[node]) {
            in_no_ring[index] = true;
            let other = arrows[index].from;
            outgoing[other].retain(|arrow| *arrow != index);
            if outgoing[other].is_empty() && incoming[other].is_empty() {
                left[other] = false;
            }
        }
        for index in std::mem::take(&mut outgoing[node]) {
            in_no_ring[index] = true;
            let other = arrows[index].to;
            incoming[other].retain(|arrow| *arrow != index);
            if incoming[other].is_empty() && outgoing[other].is_empty() {
                left[other] = false;
            }
        }
        left[node] = false;
    }

    while left.contains(&true) {
        for sinks in [true, false] {
            loop {
                let ends: Vec<usize> = (0..count).filter(|node| left[*node] && if sinks { outgoing[*node].is_empty() } else { incoming[*node].is_empty() }).collect();
                if ends.is_empty() {
                    break;
                }
                for node in ends {
                    if left[node] {
                        prune(node, arrows, &mut incoming, &mut outgoing, &mut left, &mut in_no_ring);
                    }
                }
            }
        }
        // Most arrows out over arrows in; of equal ones the card with most arrows, then the
        // lowest number.
        let most = (0..count).filter(|node| left[*node]).max_by_key(|node| {
            let (out, into) = (outgoing[*node].len() as i64, incoming[*node].len() as i64);
            (out - into, out + into, std::cmp::Reverse(ids[*node]))
        });
        if let Some(node) = most {
            for index in std::mem::take(&mut incoming[node]) {
                outgoing[arrows[index].from].retain(|arrow| *arrow != index);
            }
            prune(node, arrows, &mut incoming, &mut outgoing, &mut left, &mut in_no_ring);
        }
    }
    (0..arrows.len()).filter(|index| !in_no_ring[*index]).collect()
}

/// TALA `makeSimpleDAG`: the group's connections without rings and without doubles. A
/// connection that closes a ring is turned round; of several between the same two cards, the
/// same way, one is kept, with their number as its weight.
fn simple_dag(ids: &[i64], edges: &[(usize, usize)]) -> Vec<Arrow> {
    let mut arrows: Vec<Arrow> = edges.iter().map(|(from, to)| Arrow { from: *from, to: *to, weight: 1 }).collect();
    for index in cycle_edges(ids, &arrows) {
        let arrow = &mut arrows[index];
        (arrow.from, arrow.to) = (arrow.to, arrow.from);
    }
    let mut unique: Vec<Arrow> = Vec::new();
    for arrow in arrows {
        match unique.iter_mut().find(|kept| (kept.from, kept.to) == (arrow.from, arrow.to)) {
            Some(kept) => kept.weight += arrow.weight,
            None => unique.push(arrow),
        }
    }
    unique
}

/// The ranking of a group: TALA `rankProblem` and `rankSimplex`. Cards are numbered by TALA's
/// numbers here, lowest first.
struct Ranking {
    count: usize,
    /// By their number: the order in which TALA looks for an arrow to take or to leave
    /// (`pivotOrder`; the order by ends that it also keeps decides nothing).
    arrows: Vec<Arrow>,
    /// Per card its arrows, by number.
    incident: Vec<Vec<usize>>,
}

impl Ranking {
    fn slack(&self, arrow: usize, levels: &[i64]) -> i64 {
        levels[self.arrows[arrow].to] - levels[self.arrows[arrow].from] - 1
    }

    fn other(&self, arrow: usize, node: usize) -> usize {
        if self.arrows[arrow].from == node { self.arrows[arrow].to } else { self.arrows[arrow].from }
    }

    /// TALA `longestPathLevels`: every card as far up as its arrows allow. (TALA takes the
    /// cards from a heap; the levels do not depend on the order.)
    fn longest_path_levels(&self) -> Vec<i64> {
        let mut waiting: Vec<usize> = vec![0; self.count];
        for arrow in &self.arrows {
            waiting[arrow.to] += 1;
        }
        let mut ready: Vec<usize> = (0..self.count).filter(|node| waiting[*node] == 0).collect();
        let mut levels = vec![0_i64; self.count];
        while let Some(node) = ready.pop() {
            for arrow in self.arrows.iter().filter(|arrow| arrow.from == node) {
                levels[arrow.to] = levels[arrow.to].max(levels[node] + 1);
                waiting[arrow.to] -= 1;
                if waiting[arrow.to] == 0 {
                    ready.push(arrow.to);
                }
            }
        }
        levels
    }

    /// TALA `feasibleTree`: a tree of arrows that span exactly one level, reaching every card.
    /// It grows from the first card along such arrows; where none leads on, the tree moves by
    /// the least slack of an arrow that leaves it, which makes that arrow one.
    fn feasible_tree(&self, levels: &mut [i64]) -> Vec<bool> {
        let mut tree = vec![false; self.arrows.len()];
        let mut in_tree = vec![false; self.count];
        in_tree[0] = true;
        let mut reached = 1;
        let mut queue = vec![0];
        loop {
            let mut next = 0;
            while next < queue.len() {
                let node = queue[next];
                next += 1;
                for &arrow in &self.incident[node] {
                    let other = self.other(arrow, node);
                    if in_tree[other] || self.slack(arrow, levels) != 0 {
                        continue;
                    }
                    tree[arrow] = true;
                    in_tree[other] = true;
                    reached += 1;
                    queue.push(other);
                }
            }
            queue.clear();
            if reached == self.count {
                return tree;
            }

            let leaving: Vec<usize> = (0..self.arrows.len()).filter(|arrow| in_tree[self.arrows[*arrow].from] != in_tree[self.arrows[*arrow].to]).collect();
            let least = leaving.iter().map(|arrow| self.slack(*arrow, levels)).min().expect("the group is connected");
            let boundary: Vec<usize> = leaving.into_iter().filter(|arrow| self.slack(*arrow, levels) == least).collect();
            let selected_tail_in_tree = in_tree[self.arrows[boundary[0]].from];
            let delta = if selected_tail_in_tree { least } else { -least };
            for node in 0..self.count {
                if in_tree[node] {
                    levels[node] += delta;
                }
            }
            for arrow in boundary {
                let Arrow { from, to, .. } = self.arrows[arrow];
                if in_tree[from] == in_tree[to] {
                    continue;
                }
                let tail_in_tree = in_tree[from];
                if (least != 0 && tail_in_tree != selected_tail_in_tree) || self.slack(arrow, levels) != 0 {
                    continue;
                }
                let outside = if tail_in_tree { to } else { from };
                tree[arrow] = true;
                in_tree[outside] = true;
                reached += 1;
                queue.push(outside);
            }
        }
    }

    /// TALA `computeCutValues`: per arrow of the tree, how much the arrows weigh that cross the
    /// cut it makes, those along it minus those against it.
    fn cut_values(&self, tree: &[bool]) -> Vec<i64> {
        let mut balance = vec![0_i64; self.count];
        for arrow in &self.arrows {
            balance[arrow.from] += arrow.weight;
            balance[arrow.to] -= arrow.weight;
        }
        let mut parent_arrow: Vec<Option<usize>> = vec![None; self.count];
        let mut seen = vec![false; self.count];
        seen[0] = true;
        let mut order = Vec::new();
        let mut stack = vec![0];
        while let Some(node) = stack.pop() {
            order.push(node);
            for &arrow in &self.incident[node] {
                let other = self.other(arrow, node);
                if tree[arrow] && !seen[other] {
                    seen[other] = true;
                    parent_arrow[other] = Some(arrow);
                    stack.push(other);
                }
            }
        }
        let mut cut = vec![0_i64; self.arrows.len()];
        for &node in order.iter().skip(1).rev() {
            let arrow = parent_arrow[node].expect("every card but the first hangs on an arrow of the tree");
            cut[arrow] = if self.arrows[arrow].from == node { balance[node] } else { -balance[node] };
            balance[self.other(arrow, node)] += balance[node];
        }
        cut
    }

    /// TALA `optimize`: while an arrow of the tree has a negative cut, it leaves the tree. The
    /// cards on its head's side move down by the least slack of an arrow from them to the
    /// other side, and that arrow joins the tree.
    fn optimize(&self, levels: &mut [i64], tree: &mut [bool]) {
        loop {
            let cut = self.cut_values(tree);
            let Some(leaving) = (0..self.arrows.len()).find(|arrow| tree[*arrow] && cut[*arrow] < 0) else {
                return;
            };
            // TALA `computeHeadComponent`.
            let mut head = vec![false; self.count];
            head[self.arrows[leaving].to] = true;
            let mut queue = vec![self.arrows[leaving].to];
            let mut next = 0;
            while next < queue.len() {
                let node = queue[next];
                next += 1;
                for &arrow in &self.incident[node] {
                    let other = self.other(arrow, node);
                    if arrow != leaving && tree[arrow] && !head[other] {
                        head[other] = true;
                        queue.push(other);
                    }
                }
            }
            let crossing = (0..self.arrows.len()).filter(|arrow| !tree[*arrow] && head[self.arrows[*arrow].from] && !head[self.arrows[*arrow].to]);
            // The first of those with the least slack.
            let entering = crossing.min_by_key(|arrow| self.slack(*arrow, levels)).expect("an arrow crosses a negative cut");
            let slack = self.slack(entering, levels);
            for node in 0..self.count {
                if head[node] {
                    levels[node] += slack;
                }
            }
            tree[leaving] = false;
            tree[entering] = true;
        }
    }
}

/// TALA `rankDAG`: a level for every card of a group, so that every arrow points down and the
/// arrows are as short as they can be together (network simplex, after Gansner and others).
/// Gives the levels, from 0, and how many there are.
fn rank(ids: &[i64], dag: &[Arrow]) -> (Vec<usize>, usize) {
    let mut by_number: Vec<usize> = (0..ids.len()).collect();
    by_number.sort_by_key(|node| ids[*node]);
    let place_of = |node: usize| by_number.iter().position(|other| *other == node).expect("a card of the group");
    // The arrows come by their number already: `simple_dag` keeps their order.
    let arrows: Vec<Arrow> = dag.iter().map(|arrow| Arrow { from: place_of(arrow.from), to: place_of(arrow.to), ..*arrow }).collect();
    let incident = (0..ids.len()).map(|node| (0..arrows.len()).filter(|arrow| arrows[*arrow].from == node || arrows[*arrow].to == node).collect()).collect();
    let ranking = Ranking { count: ids.len(), arrows, incident };

    let mut levels = ranking.longest_path_levels();
    let mut tree = ranking.feasible_tree(&mut levels);
    ranking.optimize(&mut levels, &mut tree);

    let (least, most) = (*levels.iter().min().expect("a card"), *levels.iter().max().expect("a card"));
    let of = |node: usize| (levels[place_of(node)] - least) as usize;
    ((0..ids.len()).map(of).collect(), (most - least) as usize + 1)
}

/// TALA `is1N1`: one card, a row of cards that all hang between it and one other card.
fn is_one_many_one(count: usize, edges: &[(usize, usize)], levels: &[usize]) -> bool {
    let on = |level: usize| (0..count).filter(move |node| levels[*node] == level);
    if on(0).count() != 1 || on(2).count() != 1 {
        return false;
    }
    let others_of = |node: usize| edges.iter().filter(move |(from, to)| *from == node || *to == node).map(move |(from, to)| if *from == node { *to } else { *from });
    for node in on(1) {
        let (mut above, mut below) = (false, false);
        for other in others_of(node) {
            match levels[other] {
                0 => above = true,
                2 => below = true,
                _ => return false,
            }
            if above && below {
                break;
            }
        }
        if !above || !below {
            return false;
        }
    }
    let source = on(0).next().expect("one card on the first level");
    others_of(source).all(|other| levels[other] == 1)
}

/// TALA `build` and `isValid`: the levels of a group's cards if TALA lays the group out as a
/// hierarchy. It does when no tree hangs on a card, the arrows start and end somewhere, the
/// levels are three or more, the group is neither much wider than high nor (unless it is one
/// workflow from one card to one card) much higher than wide, at least three arrows in five
/// point down, and no card has too many connections.
fn build(graph: &Graph, group: &[usize]) -> Option<Vec<usize>> {
    if group.len() <= 1 || group.iter().any(|card| graph.trees_at.contains(card)) {
        return None;
    }
    let place_of = |card: usize| group.iter().position(|member| *member == card);
    // TALA leaves out a connection from a card to itself; the engine is given none.
    let edges: Vec<(usize, usize)> = graph.edges.iter().filter_map(|edge| Some((place_of(edge.from)?, place_of(edge.to)?))).collect();
    let count = group.len();
    // A card that nothing points at is a source, even if it points at nothing.
    let sources = (0..count).filter(|node| !edges.iter().any(|(_, to)| to == node)).count();
    let sinks = (0..count).filter(|node| edges.iter().any(|(_, to)| to == node) && !edges.iter().any(|(from, _)| from == node)).count();
    if sources == 0 || sinks == 0 {
        return None;
    }

    let ids: Vec<i64> = group.iter().map(|card| graph.id(*card)).collect();
    let (levels, level_count) = rank(&ids, &simple_dag(&ids, &edges));
    if level_count < MIN_HIERARCHY_LEVELS {
        return None;
    }
    let aspect_ratio = count as f64 / (level_count * level_count) as f64;
    let (too_tall, too_wide) = (aspect_ratio < 0.5, aspect_ratio > 2.0);
    let forward = edges.iter().filter(|(from, to)| levels[*to] > levels[*from]).count();
    let back = edges.len() - forward;
    let flows_one_way = forward as f64 >= 1.5 * back as f64;
    let mut workflow = count > level_count && sources == 1 && sinks == 1 && back == 0 && count <= MAX_AUTOMATIC_WORKFLOW_NODES && edges.len() <= MAX_AUTOMATIC_WORKFLOW_EDGES;
    if too_tall && workflow {
        // TALA `minimumWorkflowExtentFits`: the levels on top of each other, with the least room.
        let highest = |level: usize| (0..count).filter(|node| levels[*node] == level).map(|node| graph.nodes[group[node]].height.ceil()).fold(0.0, f64::max);
        let extent = (level_count - 1) as f64 * (CROSSING_SPACING + 2.0 * MIN_PORT_CLEARANCE) + (0..level_count).map(highest).sum::<f64>();
        workflow = extent <= MAX_GRAPH_SIZE;
    }
    let most_edges = 2 * (count as f64).sqrt().ceil() as usize;
    let dense = (0..count).any(|node| edges.iter().filter(|(from, to)| *from == node || *to == node).count() > most_edges);
    let valid = !too_wide && (!too_tall || workflow) && flows_one_way && !dense && !(level_count == 3 && is_one_many_one(count, &edges, &levels));
    valid.then_some(levels)
}

/// TALA `hierarchy.Assign`: per card its level in a hierarchy, or none. `graph` is what the
/// tree stage leaves, with the cards that trees hang on in `Graph::trees_at`: a group with such
/// a card is no hierarchy.
pub fn find_hierarchies(graph: &Graph) -> Vec<Option<usize>> {
    let mut levels = vec![None; graph.nodes.len()];
    for group in split(graph) {
        if let Some(found) = build(graph, &group) {
            for (card, level) in group.iter().zip(found) {
                levels[*card] = Some(level);
            }
        }
    }
    levels
}
