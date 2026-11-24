// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.

//! Laying out the trees of a diagram around the cards they hang on, once those are placed.
//! Ported from TALA `internal/trees`: `layout.go` (`Place`, `placeTrees`, `buildPlacementTrees`,
//! `placeTree`, `constructToOrientation`, `invertOrientationToBottom`,
//! `placeAtOrientationGuarded`, `normalizedAspectRatio`) and `geometry.go` (`layoutTree`,
//! `spacingToChildren`, `spaceChildrenEvenly`, `centerAlignChildren`, `swapDimensions`, `flip`),
//! and from `internal/layoutgraph/transaction.go` what `Commit` refuses for plain cards.
//!
//! A tree is drawn level by level under the card it hangs on, then turned to one of the four
//! sides and pushed towards that card as far as the other cards let it. For plain cards in no
//! container, with an arrow at the target of every connection. The stage also says where the
//! label of a tree's connection is to go (`positionTreeEdgeLabels` in `layout.go`,
//! `positionEdgeLabels` in `geometry.go`): the stage that places labels tries that place first.

use crate::compaction::overlaps_at;
use crate::graph::{Edge, Graph, Node, Point, TreeLink};
use crate::place::bounds;
use crate::routing::geometry::Orientation;
use crate::routing::labels::Position;
use crate::trees::{Found, Tree};

/// The room between two cards next to each other on a level.
const SIBLING_SPACING: f64 = 50.0;
/// TALA `layoutgraph.TreeParentSpacing`: the room between a card and the level under it.
const TREE_PARENT_SPACING: f64 = 100.0;
/// TALA `layoutgraph.MinArrowheadClearance`: the room a label leaves at each end of its line.
const MIN_ARROWHEAD_CLEARANCE: f64 = 20.0;
/// The sides in the order TALA tries them.
const SIDES: [Orientation; 4] = [Orientation::Bottom, Orientation::Top, Orientation::Left, Orientation::Right];

/// A card of a tree while it is laid out. TALA `layoutgraph.Tree`.
struct Part {
    /// The card's place in `Graph::nodes`.
    node: usize,
    /// The room the label on the connection to its parent needs: width, height.
    label: (f64, f64),
    /// The connection to its parent, by its index in `Graph::edges`. Not for the card the tree
    /// hangs on.
    edge: usize,
    parent: Option<usize>,
    children: Vec<usize>,
}

/// One tree to lay out: part 0 is the card it hangs on.
struct Layout<'a> {
    graph: &'a mut Graph,
    parts: Vec<Part>,
    orientation: Orientation,
}

impl Layout<'_> {
    fn card(&self, part: usize) -> &Node {
        &self.graph.nodes[self.parts[part].node]
    }

    /// TALA `moveTreeNodeAbsWithChildren`, for a card without cards inside.
    fn move_to(&mut self, part: usize, x: f64, y: f64) {
        let at = &mut self.graph.nodes[self.parts[part].node].top_left;
        if at.x != x || at.y != y {
            *at = Point { x: at.x + (x - at.x), y: at.y + (y - at.y) };
        }
    }

    /// A part and everything under it.
    fn subtree(&self, part: usize) -> Vec<usize> {
        let mut all = vec![part];
        let mut next = 0;
        while next < all.len() {
            all.extend(&self.parts[all[next]].children);
            next += 1;
        }
        all
    }

    /// TALA `offsetSubtree`.
    fn shift(&mut self, part: usize, dx: f64, dy: f64) {
        if dx == 0.0 && dy == 0.0 {
            return;
        }
        for part in self.subtree(part) {
            let at = &mut self.graph.nodes[self.parts[part].node].top_left;
            *at = Point { x: at.x + dx, y: at.y + dy };
        }
    }

    /// TALA `swapDimensions`: x and y change places, and so do width and height, so that a tree
    /// to the side can be laid out as one that grows down.
    fn swap_dimensions(&mut self) {
        for part in 0..self.parts.len() {
            let at = self.card(part).top_left;
            self.move_to(part, at.y, at.x);
            let card = &mut self.graph.nodes[self.parts[part].node];
            (card.width, card.height) = (card.height, card.width);
        }
    }

    /// TALA `flip`: every card mirrored through the origin.
    fn flip(&mut self) {
        for part in 0..self.parts.len() {
            let (x, y) = (-self.card(part).right(), -self.card(part).bottom());
            self.move_to(part, x, y);
        }
    }

    /// TALA `invertOrientationToBottom`: turns the tree so that its side becomes "down". Done
    /// twice it gives the tree back.
    fn invert(&mut self) {
        match self.orientation {
            Orientation::Top => self.flip(),
            Orientation::Right => self.swap_dimensions(),
            Orientation::Left => {
                self.swap_dimensions();
                self.flip();
            }
            _ => {}
        }
    }

    /// TALA `levelsByDepth`: the parts under `part`, level by level, `part` alone on the first.
    fn levels(&self, part: usize) -> Vec<Vec<usize>> {
        let mut levels = vec![vec![part]];
        loop {
            let below: Vec<usize> = levels[levels.len() - 1].iter().flat_map(|part| self.parts[*part].children.clone()).collect();
            if below.is_empty() {
                return levels;
            }
            levels.push(below);
        }
    }

    /// TALA `spacingToChildren`: the room between a card and its children, with the largest
    /// label on the way and an arrowhead's clearance at both ends of it.
    fn spacing_to_children(&self, part: usize) -> f64 {
        let sideways = matches!(self.orientation, Orientation::Left | Orientation::Right);
        let children = &self.parts[part].children;
        let label = children.iter().map(|child| self.parts[*child].label).map(|(width, height)| if sideways { width } else { height });
        let largest = label.fold(0.0, f64::max);
        if largest == 0.0 {
            return TREE_PARENT_SPACING;
        }
        let with_arrowheads = largest + 2.0 * MIN_ARROWHEAD_CLEARANCE;
        if children.len() == 1 {
            return TREE_PARENT_SPACING.max(with_arrowheads);
        }
        // The line forks half way: the label goes on the half below the fork.
        TREE_PARENT_SPACING / 2.0 + (TREE_PARENT_SPACING / 2.0).max(with_arrowheads)
    }

    /// The room between a child and the one before it.
    fn room_before(&self, part: usize, child: usize) -> f64 {
        let children = &self.parts[part].children;
        self.card(children[child]).top_left.x - self.card(children[child - 1]).right()
    }

    /// TALA `totalChildrenSpacing`.
    fn total_children_spacing(&self, part: usize) -> f64 {
        (1..self.parts[part].children.len()).map(|child| self.room_before(part, child)).sum()
    }

    /// TALA `spacingBefore`: the least room between the subtree of a child and that of the one
    /// before it, over the levels both have.
    fn spacing_before(&self, part: usize, child: usize) -> f64 {
        let children = &self.parts[part].children;
        let rights = self.levels(children[child - 1]);
        let lefts = self.levels(children[child]);
        let room = |(left, right): (&Vec<usize>, &Vec<usize>)| self.card(left[0]).top_left.x - self.card(right[right.len() - 1]).right();
        lefts.iter().zip(&rights).map(room).fold(f64::INFINITY, f64::min)
    }

    /// TALA `spaceChildrenEvenly`: all children as far apart as the two that are furthest apart,
    /// unless that takes too much room.
    fn space_children_evenly(&mut self, part: usize) {
        let count = self.parts[part].children.len();
        let mut rooms = vec![0.0];
        rooms.extend((1..count).map(|child| self.room_before(part, child)));
        let most = rooms[1..].iter().copied().fold(SIBLING_SPACING, f64::max);
        for child in 1..count {
            let more = most - rooms[child];
            if more == 0.0 {
                continue;
            }
            if self.spacing_before(part, child) + more > 10.0 * SIBLING_SPACING {
                return;
            }
            // The next child has less room once this one has moved.
            if child + 1 < count {
                rooms[child + 1] -= more;
            }
        }
        for (child, room) in rooms.iter().enumerate().skip(1) {
            let more = most - room;
            if more > 0.0 {
                self.shift(self.parts[part].children[child], more, 0.0);
            }
        }
    }

    /// TALA `childrenCenterX`.
    fn children_center(&self, part: usize) -> f64 {
        let children = &self.parts[part].children;
        let sum: f64 = children.iter().map(|child| self.card(*child).top_left.x + self.card(*child).width / 2.0).sum();
        sum / children.len() as f64
    }

    /// TALA `layoutTree`: the tree under part 0, growing down from y = 0. Each level is as high
    /// as its highest card; a card stands over the middle of its children.
    fn layout_tree(&mut self) {
        let levels = self.levels(0);
        let heights: Vec<f64> = levels.iter().map(|level| level.iter().map(|part| self.card(*part).height).fold(0.0, f64::max)).collect();
        let mut spacings = vec![0.0_f64; levels.len()];
        for level in 0..levels.len() - 1 {
            for &part in &levels[level] {
                spacings[level + 1] = spacings[level + 1].max(self.spacing_to_children(part));
            }
        }

        let mut level_bottom = 0.0;
        for level in 1..levels.len() {
            let level_top = level_bottom + spacings[level];
            for &part in &levels[level] {
                let centered = ((heights[level] - self.card(part).height) / 2.0).round();
                self.move_to(part, self.card(part).top_left.x, level_top + centered);
            }
            level_bottom = level_top + heights[level];
        }

        // From the lowest level up, each level from the left.
        for level in (1..levels.len()).rev() {
            let mut next_position = 0.0;
            for &part in &levels[level] {
                if self.parts[part].children.is_empty() {
                    self.move_to(part, next_position, self.card(part).top_left.y);
                } else {
                    let below = &levels[level + 1];
                    let last_child = below.iter().rposition(|child| self.parts[*child].parent == Some(part)).unwrap_or(below.len());
                    let later: Vec<usize> = below[(last_child + 1).min(below.len())..].to_vec();

                    let before = self.total_children_spacing(part);
                    self.space_children_evenly(part);
                    let grown = self.total_children_spacing(part) - before;
                    if grown > 0.0 {
                        for &other in &later {
                            self.shift(other, grown, 0.0);
                        }
                    }
                    let over_children = (self.children_center(part) - self.card(part).width / 2.0).round();
                    self.move_to(part, over_children, self.card(part).top_left.y);

                    // Too far left for its place on the level: the whole subtree moves right.
                    let short = next_position - self.card(part).top_left.x;
                    if short > 0.0 {
                        self.shift(part, short, 0.0);
                        for &other in &later {
                            self.shift(other, short, 0.0);
                        }
                    }
                }
                next_position = self.card(part).right() + SIBLING_SPACING;
            }
        }
    }

    /// TALA `centerAlignChildren`: the tree moves sideways until the card it hangs on stands
    /// over the middle of its roots.
    fn center_align_children(&mut self) {
        let card = self.card(0);
        let offset = ((card.top_left.x + card.width / 2.0) - self.children_center(0)).round();
        for child in self.parts[0].children.clone() {
            self.shift(child, offset, 0.0);
        }
    }

    /// TALA `constructToOrientation`: the tree drawn on one side of the card it hangs on,
    /// starting at 0 in the direction it grows.
    fn construct(&mut self, orientation: Orientation) {
        self.orientation = orientation;
        self.invert();
        self.layout_tree();
        self.center_align_children();
        self.invert();
    }

    /// How far out a card reaches on the tree's side.
    fn position_of(&self, node: usize) -> f64 {
        let card = &self.graph.nodes[node];
        match self.orientation {
            Orientation::Left => card.top_left.x,
            Orientation::Right => card.right(),
            Orientation::Top => card.top_left.y,
            _ => card.bottom(),
        }
    }

    fn is_further_out(&self, a: f64, b: f64) -> bool {
        if matches!(self.orientation, Orientation::Left | Orientation::Top) { a < b } else { a > b }
    }

    /// The tree, without the card it hangs on, moved along its side.
    fn move_tree(&mut self, from: f64, to: f64) {
        let by = (to - from).round();
        let (dx, dy) = if matches!(self.orientation, Orientation::Left | Orientation::Right) { (by, 0.0) } else { (0.0, by) };
        for child in self.parts[0].children.clone() {
            self.shift(child, dx, dy);
        }
    }

    /// TALA `doesOverlap`, either way round: are two cards closer than the gap they must keep?
    fn too_close(&self, a: usize, b: usize) -> bool {
        overlaps_at(self.graph, a, b, self.graph.nodes[a].top_left) || overlaps_at(self.graph, b, a, self.graph.nodes[b].top_left)
    }

    /// TALA `DoesOverlapExact`.
    fn overlap(&self, a: usize, b: usize) -> bool {
        let (a, b) = (&self.graph.nodes[a], &self.graph.nodes[b]);
        a.top_left.x < b.right() && b.top_left.x < a.right() && a.top_left.y < b.bottom() && b.top_left.y < a.bottom()
    }

    /// TALA `Transaction.Commit`, for plain cards: is the tree, where it is now, closer to a
    /// card than it may be? Two cards that were too close when the tree stood at the border
    /// (`known`) may stay so, but must not come to lie on each other.
    fn refused(&self, known: &[(usize, usize, bool)]) -> bool {
        let moved: Vec<usize> = self.parts[1..].iter().map(|part| part.node).collect();
        for &a in &moved {
            for b in 0..self.graph.nodes.len() {
                // Two cards of the tree are a pair once.
                if a == b || (moved.contains(&b) && b < a) {
                    continue;
                }
                let before = known.iter().find(|(first, second, _)| (*first, *second) == (a.min(b), a.max(b)));
                let bad = match before {
                    Some((_, _, lay_on_each_other)) => !lay_on_each_other && self.overlap(a, b),
                    None => self.too_close(a, b),
                };
                if bad {
                    return true;
                }
            }
        }
        false
    }

    /// TALA `placeAtOrientationGuarded`: the tree starts beyond the outermost card on its side
    /// and is tried next to every card between there and the card it hangs on, then at that
    /// card: the nearest place that brings it too close to no card is kept. Gives the distance
    /// left to the card it hangs on.
    fn place_at_orientation(&mut self) -> f64 {
        let in_tree: Vec<usize> = self.parts.iter().map(|part| part.node).collect();
        let others: Vec<usize> = (0..self.graph.nodes.len()).filter(|node| !in_tree.contains(node)).collect();
        let root_position = self.position_of(self.parts[0].node);
        let mut border = root_position;
        for &other in &others {
            let position = self.position_of(other);
            if self.is_further_out(position, border) {
                border = position;
            }
        }
        let mut best_position = border;
        let mut best_distance = (best_position - root_position).abs();
        self.move_tree(0.0, border);

        let mut stops: Vec<f64> = others
            .iter()
            .map(|other| self.position_of(*other))
            .filter(|position| self.is_further_out(border, *position) && self.is_further_out(*position, root_position))
            .collect();
        stops.push(root_position);

        // TALA's transaction notes the pairs that are too close when it starts, and no later.
        let count = self.graph.nodes.len();
        let pairs = (0..count).flat_map(|a| (a + 1..count).map(move |b| (a, b)));
        let known: Vec<(usize, usize, bool)> = pairs.filter(|(a, b)| self.too_close(*a, *b)).map(|(a, b)| (a, b, self.overlap(a, b))).collect();

        for stop in stops {
            let before: Vec<Point> = self.parts.iter().map(|part| self.graph.nodes[part.node].top_left).collect();
            self.move_tree(best_position, stop);
            let moved = self.parts.iter().zip(&before).any(|(part, at)| self.graph.nodes[part.node].top_left != *at);
            let distance = (stop - root_position).abs();
            if !(moved && self.refused(&known)) && distance < best_distance {
                (best_position, best_distance) = (stop, distance);
            } else {
                for (part, at) in self.parts.iter().zip(before) {
                    self.graph.nodes[part.node].top_left = at;
                }
            }
        }
        best_distance
    }

    /// TALA `normalizedAspectRatio`: how far the box around all cards is from a square, 1 or
    /// more.
    fn aspect_ratio(&self) -> f64 {
        let all: Vec<usize> = (0..self.graph.nodes.len()).collect();
        let (top_left, bottom_right) = bounds(self.graph, &all);
        let (width, height) = (bottom_right.x - top_left.x, bottom_right.y - top_left.y);
        if width >= height { width / height } else { height / width }
    }

    /// TALA `placeTree`, from its second step: the side where the tree comes nearest to the
    /// card it hangs on and, of equal ones, where the diagram stays nearest to a square. A
    /// card's second tree goes opposite its first.
    fn place(&mut self, isolated: bool, taken: &[Orientation]) -> Orientation {
        let best = if let [only] = taken {
            opposite(*only)
        } else {
            // Trees around a card with nothing else do not share a side while one is free.
            let free: Vec<Orientation> = SIDES.iter().copied().filter(|side| !taken.contains(side)).collect();
            let order = if isolated && !free.is_empty() { free } else { SIDES.to_vec() };
            let (mut best, mut best_distance, mut best_ratio) = (order[0], f64::INFINITY, f64::INFINITY);
            for side in order {
                self.construct(side);
                // TALA adds a penalty for a side that is not the container's direction: plain
                // cards have none, so every side gets it.
                let distance = self.place_at_orientation();
                let ratio = self.aspect_ratio();
                if distance < best_distance || (distance == best_distance && ratio < best_ratio) {
                    (best, best_distance, best_ratio) = (side, distance, ratio);
                }
            }
            best
        };
        self.construct(best);
        self.place_at_orientation();
        self.position_labels(isolated);
        // TALA `setOrientation`: every card of the tree knows the side the tree lies on.
        for part in &self.parts[1..] {
            if self.graph.tree_links.len() <= part.node {
                self.graph.tree_links.resize(part.node + 1, None);
            }
            self.graph.tree_links[part.node] = Some(TreeLink { edge: part.edge, side: best });
        }
        best
    }

    /// TALA `positionTreeEdgeLabels` with `positionEdgeLabels`: where the labels of the tree's
    /// connections are to go. Seen with the tree growing down: an only child has its label on
    /// the line, halfway. One of several has it beside the last piece of its route, the one
    /// that runs down to it, halfway along that piece, on the side away from the others. A
    /// root's connection gets a place only where the card the tree hangs on has nothing else:
    /// only then it is routed by rule.
    fn position_labels(&mut self, isolated: bool) {
        let sideways = matches!(self.orientation, Orientation::Left | Orientation::Right);
        self.invert();
        let roots = self.parts[0].children.clone();
        let labelled: Vec<usize> = if isolated { roots } else { roots.iter().flat_map(|root| self.parts[*root].children.clone()).collect() };
        let below: Vec<usize> = labelled.into_iter().flat_map(|part| self.subtree(part)).collect();
        for part in below {
            let (width, height) = self.parts[part].label;
            // The size across the line. (Turned, the tree's cards have their sizes swapped,
            // the labels have not.)
            let across = match sideways {
                true => width,
                false => height,
            };
            if across == 0.0 {
                continue;
            }
            let parent = self.parts[part].parent.expect("a card of a tree hangs on one");
            let edge = self.parts[part].edge;
            let preset = if self.parts[parent].children.len() == 1 {
                (Position::UnlockedMiddle, 0.5)
            } else {
                let (child, above) = (self.card(part), self.card(parent));
                let dx = child.center().x - above.center().x;
                let dy = child.top_left.y - (above.top_left.y + above.height);
                let total = dx.abs() + dy;
                let last_piece = dy - TREE_PARENT_SPACING / 2.0;
                // How far along the route from its start, as a share of the route.
                let from_child = self.graph.edges[edge].from == self.parts[part].node;
                let along = if from_child { last_piece / 2.0 } else { total - last_piece / 2.0 };
                // A child to the left has its label on the left: below the line as the route
                // runs to it. The other way round the route runs the other way, and a tree to
                // the side is turned once more.
                let mut position = if dx < 0.0 { Position::UnlockedBottom } else { Position::UnlockedTop };
                if from_child {
                    position = position.mirrored();
                }
                if sideways {
                    position = position.mirrored();
                }
                (position, along / total)
            };
            if self.graph.label_presets.len() <= edge {
                self.graph.label_presets.resize(edge + 1, None);
            }
            self.graph.label_presets[edge] = Some(preset);
        }
        self.invert();
    }
}

fn opposite(side: Orientation) -> Orientation {
    match side {
        Orientation::Bottom => Orientation::Top,
        Orientation::Top => Orientation::Bottom,
        Orientation::Left => Orientation::Right,
        _ => Orientation::Left,
    }
}

/// Does the arrow of a tree card's connection point at the card? TALA `treeEdgeDirection`.
fn points_at(whole: &Graph, tree: &Tree) -> bool {
    whole.edges[tree.edge].to == tree.node
}

fn size(tree: &Tree) -> usize {
    1 + tree.children.iter().map(size).sum::<usize>()
}

/// TALA `buildPlacementTrees`: what is laid out as one tree, per card that trees hang on, and
/// whether the card has nothing but trees. A card with other connections gets every root on its
/// own. Around a card with nothing but trees the roots go together whose arrows point the same
/// way, at the card and below the root.
fn placement_trees<'a>(graph: &Graph, cards: &[usize], whole: &Graph, found: &'a Found) -> Vec<(usize, bool, Vec<&'a Tree>)> {
    let mut trees: Vec<(usize, bool, Vec<&Tree>)> = Vec::new();
    for (sentinel, card) in cards.iter().enumerate() {
        let Some((_, roots)) = found.trees.iter().find(|(other, _)| other == card) else {
            continue;
        };
        if graph.edges_of(sentinel).next().is_some() {
            trees.extend(roots.iter().map(|root| (sentinel, false, vec![root])));
            continue;
        }
        let mut classes: Vec<((bool, bool), Vec<&Tree>)> = Vec::new();
        for root in roots {
            let class = (points_at(whole, root.children.first().unwrap_or(root)), whole.edges[root.edge].to == *card);
            match classes.iter_mut().find(|(other, _)| *other == class) {
                Some((_, together)) => together.push(root),
                None => classes.push((class, vec![root])),
            }
        }
        trees.extend(classes.into_iter().map(|(_, together)| (sentinel, true, together)));
    }
    // The largest first; of equal ones the one whose first root has the lowest number.
    trees.sort_by_key(|(_, _, roots)| (std::cmp::Reverse(roots.iter().map(|root| size(root)).sum::<usize>()), whole.id(roots[0].node)));
    trees
}

/// TALA `trees.Place`: the trees that hang on the cards of a placed group, laid out around it.
/// Their cards and connections are added to `graph`, and the cards to `cards`: the card of the
/// whole diagram for each of the graph's.
pub(crate) fn place_trees(graph: &mut Graph, cards: &mut Vec<usize>, whole: &Graph, found: &Found) {
    let mut taken: Vec<(usize, Vec<Orientation>)> = Vec::new();
    for (sentinel, isolated, roots) in placement_trees(graph, cards, whole, found) {
        // TALA `reconnectTreeGuarded`: the tree's cards join the graph, each after its parent.
        let mut parts = vec![Part { node: sentinel, label: (0.0, 0.0), edge: 0, parent: None, children: Vec::new() }];
        let mut stack: Vec<(usize, &Tree)> = roots.iter().rev().map(|root| (0, *root)).collect();
        while let Some((parent, tree)) = stack.pop() {
            let node = graph.nodes.len();
            // The card gets no number and no place in the asking order here: only the optimizer
            // reads those, and it is done with this graph.
            graph.nodes.push(Node { top_left: Point { x: 0.0, y: 0.0 }, ..whole.nodes[tree.node].clone() });
            cards.push(tree.node);
            let edge = whole.edges[tree.edge];
            let place_of = |card: usize| cards.iter().position(|other| *other == card).expect("the card it hangs on is in the graph");
            graph.edges.push(Edge { from: place_of(edge.from), to: place_of(edge.to), ..edge });
            let part = parts.len();
            parts.push(Part { node, label: (edge.min_width, edge.min_height), edge: graph.edges.len() - 1, parent: Some(parent), children: Vec::new() });
            parts[parent].children.push(part);
            stack.extend(tree.children.iter().rev().map(|child| (part, child)));
        }

        let at = match taken.iter().position(|(other, _)| *other == sentinel) {
            Some(at) => at,
            None => {
                taken.push((sentinel, Vec::new()));
                taken.len() - 1
            }
        };
        let side = Layout { graph: &mut *graph, parts, orientation: Orientation::Bottom }.place(isolated, &taken[at].1);
        if !taken[at].1.contains(&side) {
            taken[at].1.push(side);
        }
    }
}
