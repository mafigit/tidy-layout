// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.

//! Laying a hierarchy out in layers: every level a row, the rows under each other. Ported from
//! TALA `internal/hierarchy`: `placement.go` (`Place`, `placeNodesInHierarchy`,
//! `breakLongConnections`, `placeNodesByLevel`, `minimumDistanceToNextLevel`), `crossing.go`
//! (`minimizeHierarchyCrossings`, `minimizeCrossings`, `bestIndexBySwappingNeighbors`),
//! `sifting.go` (`globalSifting`, `sifting`) and `brandes_kopf.go` (`alignHierarchy`,
//! `markConflicts`, `verticalAlignment`, `horizontalCompaction`, `placeBlock`).
//!
//! For plain cards in no container, with the levels going down. An arrow over more than one
//! level gets a dummy on every level between, so that arrows only join neighbouring levels.
//! The order on a level is chosen for few crossings and short arrows, the places across after
//! Brandes and Köpf: four alignments, each card at the middle of their answers.

use crate::gorand::GoRand;
use crate::graph::{Graph, Point};
use crate::place::{split, subgraph};
use crate::placement::precision_compare;
use crate::routing::geometry::euclidean_distance;

/// The least room between two levels, and what every crossing between them adds.
const CROSSING_SPACING: f64 = 50.0;
/// The most room between two levels.
const PARENT_SPACING: f64 = 300.0;
const SIBLING_DUMMY_SPACING: f64 = 50.0;
const SIBLING_SPACING: f64 = 60.0;
/// TALA `layoutgraph.MinPortClearance`.
const MIN_PORT_CLEARANCE: f64 = 20.0;
/// TALA `layoutgraph.ContainerPadding`: what the first and the last card of a level keep to
/// the edge of what they are in, also where that is the diagram.
const CONTAINER_PADDING: f64 = 60.0;

/// A card of the hierarchy or a dummy on a long arrow. TALA `placementNode`.
struct Slot {
    /// The card's place in the group; none for a dummy.
    card: Option<usize>,
    level: usize,
    /// Its place on its level, from the left.
    rank: usize,
    aboves: Vec<usize>,
    belows: Vec<usize>,
    width: f64,
    height: f64,
    at: Point,
}

struct Layers {
    slots: Vec<Slot>,
    by_level: Vec<Vec<usize>>,
}

/// The sides a level's arrows are counted on.
#[derive(Clone, Copy, PartialEq)]
enum Arrows {
    Both,
    Below,
}

impl Layers {
    /// TALA `connect`: two slots on different levels know each other as above and below.
    fn connect(&mut self, a: usize, b: usize) {
        if self.slots[a].level == self.slots[b].level {
            return;
        }
        let (above, below) = if self.slots[a].level < self.slots[b].level { (a, b) } else { (b, a) };
        if !self.slots[below].aboves.contains(&above) {
            self.slots[below].aboves.push(above);
            self.slots[above].belows.push(below);
        }
    }

    /// TALA `computeLevelRanks`.
    fn rank_level(&mut self, level: usize) {
        for (rank, slot) in self.by_level[level].clone().into_iter().enumerate() {
            self.slots[slot].rank = rank;
        }
    }

    /// TALA `breakLongConnections`: dummies on every level an arrow passes. They join their
    /// level at its end.
    fn break_long_connections(&mut self, order: &[usize]) {
        for &slot in order {
            let level = self.slots[slot].level;
            let mut far: Vec<usize> = self.slots[slot].belows.iter().copied().filter(|below| self.slots[*below].level - level > 1).collect();
            far.sort_by_key(|below| (self.slots[*below].rank, self.slots[*below].level));
            for below in far {
                self.slots[slot].belows.retain(|other| *other != below);
                self.slots[below].aboves.retain(|other| *other != slot);
                let mut above = slot;
                for between in level + 1..self.slots[below].level {
                    let dummy = self.slots.len();
                    let rank = self.by_level[between].len();
                    self.slots.push(Slot { card: None, level: between, rank, aboves: Vec::new(), belows: Vec::new(), width: 1.0, height: 1.0, at: Point { x: 0.0, y: 0.0 } });
                    self.connect(dummy, above);
                    self.by_level[between].push(dummy);
                    above = dummy;
                }
                self.connect(below, above);
            }
        }
    }

    /// TALA `crossLevelSegments`, `countCrossings` and `addLengths`: how often the arrows of a
    /// level cross each other, and how long they are together, in levels and ranks. Two arrows
    /// cross when they go to the same side and their ends are in the other order than their
    /// starts. (TALA counts with a tree where a level has 32 arrows or more; the count is the
    /// same.)
    fn crossings_and_length(&self, level: usize, arrows: Arrows) -> (i64, f64) {
        // Start rank, end rank, down, length.
        let mut segments: Vec<(usize, usize, bool, f64)> = Vec::new();
        for &slot in &self.by_level[level] {
            let from = &self.slots[slot];
            let sides = [(&from.aboves, false), (&from.belows, true)];
            for (others, down) in sides.into_iter().filter(|(_, down)| arrows == Arrows::Both || *down) {
                for &other in others {
                    let to = &self.slots[other];
                    let length = euclidean_distance(from.rank as f64, from.level as f64, to.rank as f64, to.level as f64);
                    segments.push((from.rank, to.rank, down, length));
                }
            }
        }
        segments.sort_by_key(|(start, end, _, _)| (*start, *end));
        let mut crossings = 0;
        for (index, first) in segments.iter().enumerate() {
            for second in &segments[index + 1..] {
                crossings += i64::from(first.2 == second.2 && first.0 < second.0 && first.1 > second.1);
            }
        }
        (crossings, segments.iter().map(|segment| segment.3).sum())
    }

    /// TALA `bestIndexBySwappingNeighbors`: the card at `index` changes places with each of the
    /// next three on its level, going round at the end; the best exchange, and what it gives.
    fn best_by_swapping_neighbors(&mut self, level: usize, index: usize) -> (i64, f64, usize) {
        let count = self.by_level[level].len();
        let (mut best_crossings, mut best_length, mut best_index) = (i64::MAX, f64::INFINITY, 0);
        for next in index + 1..index + 4 {
            let other = next % count;
            self.by_level[level].swap(index, other);
            self.rank_level(level);
            let (crossings, length) = self.crossings_and_length(level, Arrows::Both);
            if crossings < best_crossings || (crossings == best_crossings && precision_compare(length, best_length) == -1) {
                (best_crossings, best_length, best_index) = (crossings, length, other);
            }
            self.by_level[level].swap(index, other);
        }
        (best_crossings, best_length, best_index)
    }

    /// TALA `minimizeCrossings`: a level is sorted by where its cards' neighbours stand on the
    /// level above (below, for the first level), then every card is exchanged with a
    /// neighbour where that gives fewer crossings or, of as many, shorter arrows.
    fn minimize_crossings(&mut self, level: usize, top_down: bool) {
        let average = |layers: &Layers, slot: usize| {
            let others = if top_down { &layers.slots[slot].aboves } else { &layers.slots[slot].belows };
            if others.is_empty() {
                return 0.0;
            }
            others.iter().map(|other| layers.slots[*other].rank).sum::<usize>() as f64 / others.len() as f64
        };
        let mut sorted = self.by_level[level].clone();
        sorted.sort_by(|a, b| average(self, *a).total_cmp(&average(self, *b)));
        self.by_level[level] = sorted;
        self.rank_level(level);
        let (mut crossings, mut length) = self.crossings_and_length(level, Arrows::Both);
        for index in 0..self.by_level[level].len() {
            let (new_crossings, new_length, swap_index) = self.best_by_swapping_neighbors(level, index);
            if new_crossings < crossings || (new_crossings == crossings && precision_compare(new_length, length) == -1) {
                (crossings, length) = (new_crossings, new_length);
                self.by_level[level].swap(index, swap_index);
            }
            self.rank_level(level);
        }
    }

    /// TALA `sifting`: a card is moved to every place on its level, first to the right end and
    /// then to the left end, and left at the best one.
    fn sift(&mut self, slot: usize, improve_if_equal: bool) {
        let level = self.slots[slot].level;
        let count = self.by_level[level].len();
        let (mut best_crossings, mut best_length) = self.crossings_and_length(level, Arrows::Both);
        if best_crossings == 0 || count == 1 {
            return;
        }
        let improved = |crossings: i64, best_crossings: i64, length: f64, best_length: f64| {
            if improve_if_equal {
                crossings <= best_crossings
            } else {
                crossings < best_crossings || (crossings == best_crossings && precision_compare(length, best_length) == -1)
            }
        };
        let start = self.by_level[level].iter().position(|other| *other == slot).expect("the card is on its level");
        let mut best_index = start;
        for index in start..count - 1 {
            self.by_level[level].swap(index, index + 1);
            self.rank_level(level);
            let (crossings, length) = self.crossings_and_length(level, Arrows::Both);
            if improved(crossings, best_crossings, length, best_length) {
                (best_crossings, best_length, best_index) = (crossings, length, index + 1);
            }
        }
        for index in (1..count).rev() {
            self.by_level[level].swap(index - 1, index);
            self.rank_level(level);
            let (crossings, length) = self.crossings_and_length(level, Arrows::Both);
            if improved(crossings, best_crossings, length, best_length) {
                (best_crossings, best_length, best_index) = (crossings, length, index - 1);
            }
        }
        for index in 0..best_index {
            self.by_level[level].swap(index, index + 1);
        }
        self.rank_level(level);
    }

    /// TALA `globalSifting`: the cards with the most connections first, every card sifted, up
    /// to ten rounds. After a round that moved no card, a place that is as good may be taken
    /// too, once.
    fn global_sifting(&mut self, graph: &Graph) {
        let degree = |layers: &Layers, slot: usize| layers.slots[slot].card.map_or(2, |card| graph.edges_of(card).count());
        let mut queue: Vec<usize> = self.by_level.iter().flatten().copied().collect();
        queue.sort_by_key(|slot| (std::cmp::Reverse(degree(self, *slot)), self.slots[*slot].level, self.slots[*slot].rank));
        let mut improve_if_equal = false;
        for _ in 0..10 {
            let mut improved = false;
            for &slot in &queue {
                let before = self.slots[slot].rank;
                self.sift(slot, improve_if_equal);
                improved |= self.slots[slot].rank != before;
            }
            if !improved && improve_if_equal {
                break;
            }
            improve_if_equal = !improved;
        }
    }

    /// TALA `minimumDistanceToNextLevel`: 50 for every crossing on the way down, or the room
    /// of the highest label on an arrow to a neighbouring level and 50; 50 at least, 300 at
    /// most.
    fn distance_to_next_level(&self, graph: &Graph, levels: &[usize], level: usize) -> f64 {
        let (crossings, _) = self.crossings_and_length(level, Arrows::Below);
        let mut label: f64 = 0.0;
        for card in self.by_level[level].iter().filter_map(|slot| self.slots[*slot].card) {
            for edge in graph.edges_of(card).filter(|edge| levels[edge.other(card)].abs_diff(levels[card]) == 1 && edge.has_label()) {
                label = label.max(edge.min_height);
            }
        }
        (crossings as f64 * CROSSING_SPACING).max(label + CROSSING_SPACING).clamp(CROSSING_SPACING, PARENT_SPACING).round()
    }

    /// TALA `placeNodesByLevel`: the levels under each other, every card of a level at the
    /// level's middle height, the cards in a row for now.
    fn place_by_level(&mut self, graph: &Graph, levels: &[usize]) {
        let mut y = 0.0;
        for level in 0..self.by_level.len() {
            let (mut x, mut height): (f64, f64) = (0.0, 0.0);
            for &slot in &self.by_level[level] {
                self.slots[slot].at = Point { x, y };
                if self.slots[slot].card.is_some() {
                    height = height.max(self.slots[slot].height);
                    x += self.slots[slot].width.ceil() + SIBLING_SPACING;
                } else {
                    x += SIBLING_SPACING;
                }
            }
            // The middle of the box around the level, dummies too.
            let bottom = self.by_level[level].iter().map(|slot| (y + self.slots[*slot].height).round()).fold(f64::NEG_INFINITY, f64::max);
            let middle = (y + (bottom - y) / 2.0).ceil();
            for &slot in &self.by_level[level] {
                let slot = &mut self.slots[slot];
                slot.at.y += (middle - (slot.at.y + slot.height / 2.0)).ceil();
            }
            y += (height + self.distance_to_next_level(graph, levels, level) + 2.0 * MIN_PORT_CLEARANCE).ceil();
        }
    }

    /// TALA `markConflicts`: the arrows a card must not be put straight under, because they
    /// cross the arrow of a dummy further along.
    fn conflicts(&self) -> Vec<(usize, usize)> {
        let mut conflicts = Vec::new();
        for level in 1..self.by_level.len().saturating_sub(1) {
            let next = &self.by_level[level + 1];
            let (mut k0, mut reached) = (0, 0);
            for (index, &slot) in next.iter().enumerate() {
                let k1 = if index == next.len() - 1 {
                    let last = self.by_level[level][self.by_level[level].len() - 1];
                    self.slots[last].rank
                } else if self.slots[slot].card.is_none() {
                    // The dummy above it, if there is one.
                    self.slots[slot].aboves.iter().filter(|above| self.slots[**above].card.is_none()).map(|above| self.slots[*above].rank).next_back().unwrap_or(0)
                } else {
                    continue;
                };
                while reached <= index {
                    let below = next[reached];
                    for &above in &self.slots[below].aboves {
                        if self.slots[above].rank < k0 || self.slots[above].rank > k1 {
                            conflicts.push((below, above));
                            conflicts.push((above, below));
                        }
                    }
                    reached += 1;
                }
                k0 = k1;
            }
        }
        conflicts
    }

    /// One of the four alignments of TALA `alignHierarchy`: `createAlignmentNodes`,
    /// `verticalAlignment` and `horizontalCompaction`. Cards are put straight under (or over)
    /// the middle one of their neighbours where nothing is in the way, which makes blocks; the
    /// blocks are pushed together from the left (or the right). Gives per slot its left edge,
    /// and the left and right end of all.
    fn align(&self, conflicts: &[(usize, usize)], down: bool, from_left: bool) -> (Vec<f64>, f64, f64) {
        // A slot in this alignment. TALA `alignmentNode`.
        struct Aligned {
            slot: usize,
            previous: Option<usize>,
            root: usize,
            aligned_with: usize,
            sink: usize,
            medians: Vec<usize>,
            shift: f64,
            x: f64,
            block_size: f64,
            left_pad: f64,
            right_pad: f64,
        }
        let mut nodes: Vec<Aligned> = Vec::new();
        for level in &self.by_level {
            for (index, &slot) in level.iter().enumerate() {
                let pad = |end: bool| if end { CONTAINER_PADDING } else { 0.0 };
                nodes.push(Aligned {
                    slot,
                    previous: None,
                    root: 0,
                    aligned_with: 0,
                    sink: 0,
                    medians: Vec::new(),
                    shift: if from_left { f64::INFINITY } else { f64::NEG_INFINITY },
                    x: if from_left { f64::NEG_INFINITY } else { f64::INFINITY },
                    block_size: self.slots[slot].width,
                    left_pad: pad(index == 0),
                    right_pad: pad(index == level.len() - 1),
                });
            }
        }
        // TALA `sortAlignmentNodes`: level by level in the direction of the alignment.
        nodes.sort_by_key(|node| {
            let Slot { level, rank, .. } = self.slots[node.slot];
            (if down { level as i64 } else { -(level as i64) }, if from_left { rank as i64 } else { -(rank as i64) })
        });
        let place_of = |nodes: &[Aligned], slot: usize| nodes.iter().position(|node| node.slot == slot).expect("every slot is aligned");
        for index in 0..nodes.len() {
            let slot = &self.slots[nodes[index].slot];
            // TALA `medianNeighborsContext`: the middle one, or the middle two, of the cards
            // it is joined to on the level before, from the left.
            let before = if down { &slot.aboves } else { &slot.belows };
            let mut neighbours: Vec<usize> = before.clone();
            neighbours.sort_by(|a, b| self.slots[*a].at.x.total_cmp(&self.slots[*b].at.x));
            let medians: Vec<usize> = match neighbours.len() {
                0 => Vec::new(),
                count if count % 2 == 1 => vec![neighbours[count / 2]],
                count => {
                    let (left, right) = (neighbours[count / 2 - 1], neighbours[count / 2]);
                    if from_left { vec![left, right] } else { vec![right, left] }
                }
            };
            nodes[index].medians = medians.into_iter().map(|slot| place_of(&nodes, slot)).collect();
            let same_level = index > 0 && self.slots[nodes[index - 1].slot].level == slot.level;
            nodes[index].previous = same_level.then(|| index - 1);
            (nodes[index].root, nodes[index].aligned_with, nodes[index].sink) = (index, index, index);
        }

        // TALA `verticalAlignment`.
        let mut last_aligned_rank = i64::MAX;
        for index in 0..nodes.len() {
            if nodes[index].previous.is_none() {
                last_aligned_rank = if from_left { i64::MIN } else { i64::MAX };
            }
            for median in nodes[index].medians.clone() {
                let rank = self.slots[nodes[median].slot].rank as i64;
                if conflicts.contains(&(nodes[index].slot, nodes[median].slot))
                    || nodes[index].aligned_with != index
                    || (from_left && last_aligned_rank >= rank)
                    || (!from_left && last_aligned_rank <= rank)
                {
                    continue;
                }
                nodes[median].aligned_with = index;
                let root = nodes[median].root;
                nodes[index].root = root;
                nodes[index].aligned_with = root;
                nodes[root].block_size = nodes[root].block_size.max(nodes[index].block_size);
                nodes[root].left_pad = nodes[root].left_pad.max(nodes[index].left_pad);
                nodes[root].right_pad = nodes[root].right_pad.max(nodes[index].right_pad);
                last_aligned_rank = rank;
            }
        }

        // TALA `placeBlock`: a block goes next to the blocks before its cards; where those are
        // of another class of blocks, the classes are pushed apart instead.
        fn place_block(nodes: &mut [Aligned], slots: &[Slot], root: usize, from_left: bool) {
            if nodes[root].x.is_finite() {
                return;
            }
            nodes[root].x = 0.0;
            let mut node = root;
            loop {
                if let Some(previous) = nodes[node].previous {
                    let previous_root = nodes[previous].root;
                    place_block(nodes, slots, previous_root, from_left);
                    if nodes[root].sink == root {
                        nodes[root].sink = nodes[previous_root].sink;
                    }
                    // TALA `distanceFromPreviousRoot`.
                    let beside_dummy = slots[nodes[root].slot].card.is_none() || nodes[root].previous.is_some_and(|before| slots[nodes[before].slot].card.is_none());
                    let pads = if from_left { nodes[previous_root].right_pad + nodes[root].left_pad } else { nodes[previous_root].left_pad + nodes[root].right_pad };
                    let delta = if beside_dummy { SIBLING_DUMMY_SPACING } else { SIBLING_SPACING } + pads;
                    let sink = nodes[previous_root].sink;
                    if nodes[root].sink != sink {
                        nodes[sink].shift = if from_left {
                            nodes[sink].shift.min(nodes[root].x - nodes[previous_root].x - nodes[previous_root].block_size - delta)
                        } else {
                            nodes[sink].shift.max(nodes[root].x - nodes[previous_root].x + nodes[root].block_size + delta)
                        };
                    } else if from_left {
                        nodes[root].x = nodes[root].x.max(nodes[previous_root].x + nodes[previous_root].block_size + delta);
                    } else {
                        nodes[root].x = nodes[root].x.min(nodes[previous_root].x - nodes[root].block_size - delta);
                    }
                }
                node = nodes[node].aligned_with;
                if node == root {
                    return;
                }
            }
        }
        // TALA `horizontalCompaction`.
        for index in 0..nodes.len() {
            if nodes[index].root == index {
                place_block(&mut nodes, &self.slots, index, from_left);
            }
        }
        for index in 0..nodes.len() {
            nodes[index].x = nodes[nodes[index].root].x;
            let shift = nodes[nodes[index].sink].shift;
            if nodes[index].root == index && shift.is_finite() {
                nodes[index].x += shift;
            }
        }

        let mut xs = vec![0.0; self.slots.len()];
        let (mut least, mut most) = (f64::INFINITY, f64::NEG_INFINITY);
        for node in &nodes {
            xs[node.slot] = node.x + nodes[node.root].block_size / 2.0 - self.slots[node.slot].width / 2.0;
            least = least.min(node.x);
            most = most.max(node.x + node.block_size);
        }
        (xs, least, most)
    }

    /// TALA `alignHierarchy`: the four alignments, moved to the ends of the narrowest of them,
    /// and every slot at the middle of its four places.
    fn align_hierarchy(&mut self) {
        let conflicts = self.conflicts();
        // Down from the left, down from the right, up from the left, up from the right.
        let directions = [(true, true), (true, false), (false, true), (false, false)];
        let alignments: Vec<(Vec<f64>, f64, f64)> = directions.iter().map(|(down, from_left)| self.align(&conflicts, *down, *from_left)).collect();
        let mut narrowest = 0;
        for (index, (_, least, most)) in alignments.iter().enumerate() {
            if most - least < alignments[narrowest].2 - alignments[narrowest].1 {
                narrowest = index;
            }
        }
        for slot in 0..self.slots.len() {
            let mut places: Vec<f64> = Vec::new();
            for (index, (xs, least, most)) in alignments.iter().enumerate() {
                let shift = if directions[index].1 { alignments[narrowest].1 - least } else { alignments[narrowest].2 - most };
                places.push(xs[slot] + shift);
            }
            places.sort_by(f64::total_cmp);
            self.slots[slot].at.x = (places[1] / 2.0 + places[2] / 2.0).round();
        }
    }
}

/// TALA `placeNodesInHierarchy`: a place for every card of a hierarchy. `graph` is the group,
/// `levels` the level of each of its cards. The cards start in a shuffled order, which the
/// random numbers decide.
fn place_group(graph: &mut Graph, levels: &[usize], random: &mut GoRand) {
    let mut order: Vec<usize> = (0..graph.nodes.len()).collect();
    random.shuffle(&mut order);
    let slot_of = |card: usize| card;
    let slots = (0..graph.nodes.len())
        .map(|card| Slot { card: Some(card), level: levels[card], rank: 0, aboves: Vec::new(), belows: Vec::new(), width: graph.nodes[card].width, height: graph.nodes[card].height, at: Point { x: 0.0, y: 0.0 } })
        .collect();
    let level_count = levels.iter().max().map_or(0, |most| most + 1);
    let mut layers = Layers { slots, by_level: vec![Vec::new(); level_count] };
    for edge in &graph.edges {
        layers.connect(slot_of(edge.to), slot_of(edge.from));
    }
    for &card in &order {
        layers.by_level[levels[card]].push(slot_of(card));
    }
    for level in 0..level_count {
        layers.rank_level(level);
    }
    layers.break_long_connections(&order);

    // TALA `minimizeHierarchyCrossings`: as many rounds as the widest level has slots.
    let widest = layers.by_level.iter().map(Vec::len).max().unwrap_or(0);
    for _ in 0..widest {
        for level in 0..level_count {
            layers.minimize_crossings(level, level > 0);
        }
    }
    layers.global_sifting(graph);
    layers.place_by_level(graph, levels);
    layers.align_hierarchy();
    for card in 0..graph.nodes.len() {
        graph.nodes[card].top_left = layers.slots[slot_of(card)].at;
    }
}

/// TALA `hierarchy.Place`: per card of `graph` its place in a hierarchy, or none. `levels` is
/// what `find_hierarchies` gives. Every hierarchy is laid out on its own, from 0; the random
/// numbers go on from one to the next.
pub fn place_hierarchies(graph: &Graph, levels: &[Option<usize>], seed: i64) -> Vec<Option<Point>> {
    let mut places = vec![None; graph.nodes.len()];
    let mut random = GoRand::new(seed);
    for group in split(graph) {
        let Some(group_levels) = group.iter().map(|card| levels[*card]).collect::<Option<Vec<usize>>>() else {
            continue;
        };
        let mut part = subgraph(graph, &group);
        place_group(&mut part, &group_levels, &mut random);
        for (card, placed) in group.iter().zip(&part.nodes) {
            places[*card] = Some(placed.top_left);
        }
    }
    places
}
