// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.

//! Finding the trees of a diagram: branches of cards that hang on the rest by one connection.
//! TALA takes them out before node placement and lays them out by other rules afterwards.
//! Ported from TALA `internal/trees`: `layout.go` (`Preprocess`, `extractTreesInContainer`,
//! `handleTreeSubgraph`, `putBackNonBranchingTrees`) and `preprocess_helpers.go`.
//!
//! For plain cards in no container, with an arrow at the target of every connection. Branches
//! that do not branch (chains) are put back, at the end of the lists of cards and connections:
//! the stage changes their order also where it leaves no tree.

use crate::graph::{Edge, Graph};

/// A card of a tree, with the cards that hang on it.
#[derive(Clone, Debug, PartialEq)]
pub struct Tree {
    pub node: usize,
    /// The connection to its parent or, for a root, to the card the tree hangs on: its index
    /// in `Graph::edges`.
    pub edge: usize,
    pub children: Vec<Tree>,
}

/// What the tree stage leaves.
#[derive(Clone, Debug, PartialEq)]
pub struct Found {
    /// The cards left in the graph, in TALA's order after the stage.
    pub order: Vec<usize>,
    /// The connections left in the graph, by index in `Graph::edges`, in TALA's order.
    pub edges: Vec<usize>,
    /// Per card that trees hang on: the roots of its trees. TALA keeps them in a map; here they
    /// come in the order of the cards left.
    pub trees: Vec<(usize, Vec<Tree>)>,
}

/// Does the arrow of a tree card's connection point at the card, or away from it? TALA
/// `treeEdgeDirection`, for connections with an arrow at their target.
#[derive(Clone, Copy, Debug, PartialEq)]
enum Direction {
    Inwards,
    Outwards,
}

/// One card's place in the trees while they are built. TALA `layoutgraph.Tree`.
#[derive(Clone, Debug)]
struct Part {
    node: usize,
    parent: Option<usize>,
    children: Vec<usize>,
    edge: Option<usize>,
}

/// The graph as the stage changes it, and the trees so far.
struct Search<'a> {
    graph: &'a Graph,
    /// TALA `g.Nodes`, `g.Edges` and every card's own `Edges`.
    nodes: Vec<usize>,
    edges: Vec<usize>,
    edges_of: Vec<Vec<usize>>,
    /// One part per card, at the card's index; joints come after.
    parts: Vec<Part>,
}

impl Search<'_> {
    fn other(&self, edge: usize, node: usize) -> usize {
        self.graph.edges[edge].other(node)
    }

    fn direction(&self, part: usize) -> Direction {
        let edge = self.parts[part].edge.expect("a card of a tree has its connection");
        if self.graph.edges[edge].to == self.parts[part].node { Direction::Inwards } else { Direction::Outwards }
    }

    /// TALA `dominantTreeEdgeDirection` without the root: every connection has a direction, so
    /// it is that of the first card below the root or, for a root alone, its own. (With the
    /// root it is the root's own: `direction`.)
    fn dominant_direction(&self, root: usize) -> Direction {
        self.direction(self.parts[root].children.first().copied().unwrap_or(root))
    }

    /// TALA `addTreeChild`.
    fn add_child(&mut self, parent: usize, child: usize) {
        if let Some(old) = self.parts[child].parent {
            self.parts[old].children.retain(|other| *other != child);
        }
        self.parts[child].parent = Some(parent);
        self.parts[parent].children.push(child);
    }

    /// TALA `treeEndOfLine`: the last card of a tree that never branches.
    fn end_of_line(&self, root: usize) -> Option<usize> {
        let mut current = root;
        loop {
            match self.parts[current].children[..] {
                [] => return Some(current),
                [only] => current = only,
                _ => return None,
            }
        }
    }

    /// TALA `treeBranches`: following only children, does a card have two?
    fn branches(&self, root: usize) -> bool {
        self.end_of_line(root).is_none()
    }

    /// TALA `reverseTreeChain`: a chain turned round, its last card becoming its root. That
    /// card keeps its connection: nothing asks for it.
    fn reverse_chain(&mut self, root: usize) {
        let mut chain = vec![root];
        let mut current = root;
        while let Some(&next) = self.parts[current].children.first() {
            self.parts[current].edge = self.parts[next].edge;
            self.parts[current].children.retain(|other| *other != next);
            chain.push(next);
            current = next;
        }
        for pair in chain.windows(2) {
            self.add_child(pair[1], pair[0]);
        }
    }

    /// TALA `removeNodeFromGraph` and `disconnectTreeEdge`.
    fn remove(&mut self, node: usize, edge: usize) {
        self.nodes.retain(|other| *other != node);
        for end in [self.graph.edges[edge].from, self.graph.edges[edge].to] {
            self.edges_of[end].retain(|other| *other != edge);
        }
        self.edges.retain(|other| *other != edge);
    }

    /// TALA `reconnectTreeGuarded`, for a chain: its cards back into the graph, each after its
    /// parent, with their connections: at the end of both lists. The cards' own lists are left
    /// alone: nothing asks for them any more.
    fn reconnect(&mut self, root: usize) {
        let mut next = Some(root);
        while let Some(current) = next {
            self.nodes.push(self.parts[current].node);
            self.edges.push(self.parts[current].edge.expect("a card of a tree has its connection"));
            next = self.parts[current].children.first().copied();
        }
    }

    fn tree(&self, part: usize) -> Tree {
        let Part { node, edge, children, .. } = &self.parts[part];
        Tree {
            node: *node,
            edge: edge.expect("a card of a tree has its connection"),
            children: children.iter().map(|child| self.tree(*child)).collect(),
        }
    }
}

/// TALA `candidateTreeRoots`: of the trees around a card that has no other connection, those
/// that can lead to the root of one tree for all of them, by the directions of their arrows.
fn candidate_roots(search: &Search, roots: &[usize]) -> Vec<usize> {
    let of = |direction: Direction| -> Vec<usize> {
        roots.iter().copied().filter(|root| search.dominant_direction(*root) == direction).collect()
    };
    let (outwards, inwards) = (of(Direction::Outwards), of(Direction::Inwards));
    let mut candidates = Vec::new();
    // One tree whose arrows point the other way than all the others' leads to the root.
    if outwards.len() == 1 {
        candidates.extend(&outwards);
    }
    if inwards.len() == 1 {
        candidates.extend(&inwards);
    }
    candidates
}

/// TALA `handleTreeSubgraph`: a card that only trees hang on is no core for them. Where one of
/// the trees is a chain that can lead to a root, the chain is turned round: its last card
/// becomes the card everything hangs on, and the old one a card of the tree.
fn merge_around(search: &mut Search, sentinel: usize, roots: Vec<usize>) -> (usize, Vec<usize>) {
    if !(search.edges_of[sentinel].is_empty() && roots.len() > 1) {
        return (sentinel, roots);
    }
    for candidate in candidate_roots(search, &roots) {
        let Some(new_root) = search.end_of_line(candidate) else {
            continue;
        };
        let joint = search.parts.len();
        search.parts.push(Part { node: sentinel, parent: None, children: Vec::new(), edge: None });
        for &other in &roots {
            if other != new_root {
                search.add_child(joint, other);
            }
        }
        let edge_to_sentinel = search.parts[candidate].edge;
        search.reverse_chain(candidate);
        search.add_child(candidate, joint);
        search.parts[joint].edge = edge_to_sentinel;
        let new_sentinel = search.parts[new_root].node;
        let tree_root = search.parts[new_root].children[0];
        search.nodes.retain(|other| *other != sentinel);
        search.nodes.push(new_sentinel);
        return (new_sentinel, vec![tree_root]);
    }
    (sentinel, roots)
}

/// TALA `trees.Preprocess`: the trees of a diagram, and the graph without them.
///
/// Cards with one connection are taken off, round after round, each becoming a child of the
/// card it hung on once that one is taken off too. A card stays (it is "terminal") where the
/// arrows of its branches do not all point the same way. In the end a branch that never
/// branches is put back.
pub fn find_trees(graph: &Graph) -> Found {
    let count = graph.nodes.len();
    let mut search = Search {
        graph,
        nodes: (0..count).collect(),
        edges: (0..graph.edges.len()).collect(),
        edges_of: (0..count)
            .map(|node| (0..graph.edges.len()).filter(|edge| graph.edges[*edge].from == node || graph.edges[*edge].to == node).collect())
            .collect(),
        parts: (0..count).map(|node| Part { node, parent: None, children: Vec::new(), edge: None }).collect(),
    };
    let mut terminal = vec![false; count];
    // Per card: the roots that hang on it, and whether arrows point at it, away from it or both.
    let mut roots_at: Vec<Vec<usize>> = vec![Vec::new(); count];
    let mut arrows_at: Vec<[bool; 2]> = vec![[false; 2]; count];
    loop {
        let fringe: Vec<usize> = (0..count).filter(|node| search.edges_of[*node].len() == 1 && !terminal[*node]).collect();
        if fringe.is_empty() {
            break;
        }
        // No card of a round becomes terminal before its turn: only the other card of a pair
        // could make it so, and of a pair the first one stays.
        for &node in &fringe {
            let edge = search.edges_of[node][0];
            let sentinel = search.other(edge, node);
            // Two cards left with one connection between them: one of them has to stay.
            if search.edges_of[sentinel].len() == 1 && !terminal[sentinel] {
                terminal[node] = true;
                continue;
            }
            search.parts[node].edge = Some(edge);
            roots_at[sentinel].push(node);
            arrows_at[sentinel][usize::from(graph.edges[edge].to == sentinel)] = true;

            let branches: Vec<usize> = roots_at[node].iter().copied().filter(|root| !terminal[*root]).collect();
            if branches.is_empty() {
                continue;
            }
            // Arrows both ways, also those of branches that stayed in the graph.
            if arrows_at[node] == [true, true] {
                terminal[node] = true;
                continue;
            }
            // The card it hangs on stays where the arrows do not go on in one direction.
            if search.direction(node) != search.direction(branches[0]) {
                terminal[sentinel] = true;
            }
            for branch in branches {
                search.add_child(node, branch);
            }
        }
        for &node in &fringe {
            if !terminal[node] {
                let edge = search.parts[node].edge.expect("a card taken off has its connection");
                search.remove(node, edge);
            }
        }
    }

    // The cards left in the graph that trees hang on.
    let mut trees: Vec<(usize, Vec<usize>)> = Vec::new();
    for node in search.nodes.clone() {
        let roots: Vec<usize> = roots_at[node].iter().copied().filter(|root| !terminal[*root]).collect();
        if !roots.is_empty() {
            trees.push(merge_around(&mut search, node, roots));
        }
    }

    // TALA `putBackNonBranchingTrees`, in the order of the cards left.
    let mut kept: Vec<(usize, Vec<usize>)> = Vec::new();
    for node in search.nodes.clone() {
        let Some(at) = trees.iter().position(|(sentinel, _)| *sentinel == node) else {
            continue;
        };
        let roots = trees[at].1.clone();
        // A card with nothing but trees, and more than one: they stay out, chains too. (TALA
        // also asks for a single tree that branches; that one stays out below.)
        if search.edges_of[node].is_empty() && roots.len() > 1 {
            kept.push((node, roots));
            continue;
        }
        let mut branching = Vec::new();
        for root in roots {
            if search.branches(root) {
                branching.push(root);
            } else {
                search.reconnect(root);
            }
        }
        if !branching.is_empty() {
            kept.push((node, branching));
        }
    }
    Found {
        order: search.nodes.clone(),
        edges: search.edges.clone(),
        trees: kept.iter().map(|(sentinel, roots)| (*sentinel, roots.iter().map(|root| search.tree(*root)).collect())).collect(),
    }
}

/// The graph the stages after the tree stage work on: the cards and connections it leaves, in
/// its order, each card with its number, and which of them trees hang on.
pub fn without_trees(graph: &Graph, found: &Found) -> Graph {
    let place_of = |card: usize| found.order.iter().position(|other| *other == card).expect("both ends are left");
    let nodes = found.order.iter().map(|card| graph.nodes[*card].clone()).collect();
    let edges = found
        .edges
        .iter()
        .map(|edge| graph.edges[*edge])
        .map(|edge| Edge { from: place_of(edge.from), to: place_of(edge.to), ..edge })
        .collect();
    let trees_at = found.trees.iter().map(|(sentinel, _)| place_of(*sentinel)).collect();
    Graph { ids: found.order.iter().map(|card| graph.id(*card)).collect(), trees_at, ..Graph::new(nodes, edges) }
}
