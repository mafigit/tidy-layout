// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.

//! The cards and connections the engine works on. Ported from TALA `internal/layoutgraph`:
//! `Graph.ComputeCellSize` (graph.go), `Node.deltaToGuarded` (node.go) and `Cluster.Resize`,
//! `Cluster.ArrangeClusterNodes` (cluster.go).

use crate::policy::{CONNECTED_NODE_GAP, MIN_CELL_SIZE, NODE_GAP};

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Point {
    pub x: f64,
    pub y: f64,
}

/// A card: its top-left corner and its size.
#[derive(Clone, Debug, PartialEq)]
pub struct Node {
    pub top_left: Point,
    pub width: f64,
    pub height: f64,
}

/// A connection between two cards, by their index in `Graph::nodes`.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Edge {
    pub from: usize,
    pub to: usize,
    /// The room the connection needs between its cards when they sit side by side: its label's
    /// width with padding. 0 without a label.
    pub min_width: f64,
    /// The same when one card is above the other: the label's height with padding.
    pub min_height: f64,
}

impl Edge {
    /// A connection without a label.
    pub fn plain(from: usize, to: usize) -> Self {
        Edge { from, to, min_width: 0.0, min_height: 0.0 }
    }

    /// TALA `Edge.Label != nil`. A label always needs room: its box plus padding.
    pub fn has_label(&self) -> bool {
        self.min_width > 0.0 || self.min_height > 0.0
    }

    /// TALA `Node.Adjacent`: the card at the other end.
    pub fn other(&self, node: usize) -> usize {
        if self.from == node { self.to } else { self.from }
    }
}

/// Where a card is, seen from another one. TALA `Node.orientationAtPoint`.
#[derive(Clone, Copy, Debug, PartialEq)]
enum Side {
    Above,
    Below,
    Left,
    Right,
    /// On a diagonal, or overlapping.
    Neither,
}

/// TALA `ClusterArrangement`.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Arrangement {
    /// Side by side.
    Row,
    /// Under each other.
    Column,
}

/// A cluster while its cards are out of the graph and one card, the vessel, stands for them.
/// TALA `Cluster` with its `Vessel`.
#[derive(Clone, Debug, PartialEq)]
pub struct Vessel {
    /// The vessel's place in `Graph::nodes`.
    pub node: usize,
    pub arrangement: Arrangement,
    /// TALA `DesiredArrangement`: what the cards around the cluster last asked for. It stays
    /// when a turn to it was refused.
    pub desired: Arrangement,
    /// The room between two cards of the cluster.
    pub padding: f64,
    /// Whether the cards stand in the vessel in the opposite order: TALA mirrors a group card
    /// by card, and puts the cards of its clusters in order only at its next transaction.
    pub backwards: bool,
    /// How many cards the cluster has, and the size they all have.
    pub count: usize,
    pub width: f64,
    pub height: f64,
}

impl Vessel {
    /// TALA `Cluster.Resize`: the size of the vessel around its cards.
    pub fn size(&self) -> (f64, f64) {
        let (cards, gaps) = (self.count as f64, (self.count - 1) as f64);
        match self.arrangement {
            Arrangement::Row => (self.width * cards + self.padding * gaps, self.height),
            Arrangement::Column => (self.width, self.height * cards + self.padding * gaps),
        }
    }

    /// TALA `Cluster.ArrangeClusterNodes`: where a card of the cluster stands when the vessel
    /// is at `at`. TALA moves the cards with the vessel.
    pub fn member(&self, at: Point, place: usize) -> Node {
        let place = if self.backwards { self.count - 1 - place } else { place };
        let top_left = match self.arrangement {
            Arrangement::Row => Point { x: at.x + place as f64 * (self.width + self.padding), y: at.y },
            Arrangement::Column => Point { x: at.x, y: at.y + place as f64 * (self.height + self.padding) },
        };
        Node { top_left, width: self.width, height: self.height }
    }
}

#[derive(Clone, Debug)]
pub struct Graph {
    pub nodes: Vec<Node>,
    pub edges: Vec<Edge>,
    /// The step of the placement grid.
    pub cell_size: f64,
    /// Whether a connection may be one slanted line from card to card, as TALA draws it where
    /// that is cheaper than a route with bends. The editor turns it off.
    pub slanted_lines: bool,
    /// Connections, by their index in `edges`, that no card sees for a while (`edges_of`). TALA
    /// `withHubSpokesSuppressed` takes them out of the hub's list only; while they are out,
    /// nothing reads the list of a spoke.
    pub hidden: Vec<usize>,
    /// TALA's number of each card, where it is not the card's place in `nodes`: in a group taken
    /// out of a larger graph the cards come in another order, and TALA breaks some ties by
    /// number. Empty: the numbers are the places.
    pub ids: Vec<i64>,
    /// The cards in the order TALA asks them whether they stand in the way (`cards_in_the_way`):
    /// the order of the whole diagram, also for a group taken out of it. Empty: the order of
    /// `nodes`.
    pub asking_order: Vec<usize>,
    /// The cards that trees hang on while the trees are out of the graph (TALA
    /// `IsTreeSentinel`): the optimizer does not turn them.
    pub trees_at: Vec<usize>,
    /// TALA `Graph.Clusters`: the clusters whose vessels are cards of this graph.
    pub vessels: Vec<Vessel>,
    /// Per connection, by its index in `edges`: the card of a cluster it belongs to, where one
    /// of its ends is a vessel (TALA `EdgeAbduction.OriginallyFrom`, `OriginallyTo`), as the
    /// card's place in its cluster. Shorter than `edges`: the rest have none.
    pub behind: Vec<Option<usize>>,
    /// Per card, its connections in the order of the card's own list (TALA `Node.Edges`), by
    /// their index in `edges`, where that is another order than that of `edges`: once trees
    /// are back in the graph. Empty: the order of `edges`.
    pub own_edges: Vec<Vec<usize>>,
    /// The cards of the diagram outside this graph that have a place, where they stand now.
    /// TALA asks them too whether a cluster may turn: its transaction for that is one on the
    /// whole diagram.
    pub outside: Vec<Node>,
    /// How often a cluster of this graph turned for good.
    pub turns: usize,
    /// For routing one group of a diagram: how many of the first cards are cards of other
    /// groups that are only in the way (TALA `nearbyNodes`). The prices of the router are
    /// taken from the group's own cards.
    pub nearby: usize,
    /// For routing, once the trees are back in the graph: per card of a tree, the connection
    /// to the card it hangs on and the side of that card it lies on (TALA `NodeToTree`).
    /// Shorter than `nodes`: the rest are no tree cards.
    pub tree_links: Vec<Option<TreeLink>>,
    /// For routing: per connection, whether TALA takes it for a tree connection (`TreeEdgeMap`
    /// with `AddIsolatedTreeEdges`). Shorter than `edges`: the rest are none.
    pub tree_edges: Vec<bool>,
    /// For routing: per card of a hierarchy, the number of its hierarchy and its level in it
    /// (TALA `Node.Hierarchy`, `HierarchyLevel`). Shorter than `nodes`: the rest are in none.
    pub levels: Vec<Option<(usize, i64)>>,
    /// For placing labels: per connection, where the tree stage wants its label (TALA
    /// `Edge.Label.Position` and `Edge.LabelPercentage` as `trees.Place` sets them): on the
    /// line or beside it, and how far along the route, from 0 to 1. Shorter than `edges`: the
    /// rest have none.
    pub label_presets: Vec<Option<(crate::routing::labels::Position, f64)>>,
    /// For routing, once the cards of the clusters are cards of the graph again (TALA
    /// `grouping.Cleanup`): the clusters, in TALA's order (`ClusterOrder`). Their vessels are
    /// gone.
    pub clusters: Vec<RoutedCluster>,
    /// TALA `Graph.edgeLengthCache`: the price of the whole diagram (`swap::length`,
    /// `gaps::length`) by a key of what TALA hashes: the cell size, the prices, and of every
    /// card its number, place, size and connections. The cards of a cluster are not in the
    /// key: where the graph's cards stand as before while a cluster's cards stand in another
    /// order, TALA takes the old price. A rejected try keeps what it stored.
    pub length_cache: LengthCache,
}

/// The price cache of a graph (`Graph::length_cache`). A panic while it is borrowed leaves at
/// worst a price that is missing, never a wrong one: it is safe to use after one.
///
/// It also holds the prices of a bend and of a crossing as TALA's graph has them stored
/// (`Graph.turnCost`, `crossingCost`), which go into the key: 0 until a price of the whole
/// diagram is first worked out, and the bend's again after the gap stage (`ResetTurnCost`).
#[derive(Clone, Debug, Default)]
pub struct LengthCache {
    prices: std::cell::RefCell<std::collections::HashMap<Vec<u64>, f64>>,
    stored: std::cell::Cell<(f64, f64)>,
}

impl std::panic::RefUnwindSafe for LengthCache {}
impl std::panic::UnwindSafe for LengthCache {}

impl LengthCache {
    pub fn get(&self, key: &[u64]) -> Option<f64> {
        self.prices.borrow().get(key).copied()
    }

    /// Keeps a price that was worked out, with the prices of a bend and a crossing it was
    /// worked out with: TALA's graph has them stored from then on.
    pub fn insert(&self, key: Vec<u64>, price: f64, turn: f64, crossing: f64) {
        self.prices.borrow_mut().insert(key, price);
        self.stored.set((turn, crossing));
    }

    /// TALA `ResetTurnCost`.
    pub fn forget_turn(&self) {
        let (_, crossing) = self.stored.get();
        self.stored.set((0.0, crossing));
    }

    /// The prices of a bend and of a crossing as TALA's graph has them stored now.
    pub fn stored(&self) -> (f64, f64) {
        self.stored.get()
    }
}

/// A cluster as routing sees it: TALA `Cluster` after `grouping.Cleanup`.
#[derive(Clone, Debug, PartialEq)]
pub struct RoutedCluster {
    /// Its cards, by their place in `Graph::nodes`, in the cluster's order: from the top or
    /// from the left.
    pub members: Vec<usize>,
    pub arrangement: Arrangement,
    /// TALA `DesiredArrangement`.
    pub desired: Arrangement,
}

/// What ties a card of a tree to the card it hangs on. TALA `Tree.SentinelEdge` and
/// `Tree.Orientation`.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct TreeLink {
    /// The connection to the card it hangs on, by its index in `Graph::edges`.
    pub edge: usize,
    /// The side of that card the tree lies on.
    pub side: crate::routing::geometry::Orientation,
}

impl Graph {
    /// A graph with the cell size TALA derives from the card sizes, routed as TALA routes it.
    pub fn new(nodes: Vec<Node>, edges: Vec<Edge>) -> Self {
        let cell_size = compute_cell_size(&nodes);
        Graph {
            nodes,
            edges,
            cell_size,
            slanted_lines: true,
            hidden: Vec::new(),
            ids: Vec::new(),
            asking_order: Vec::new(),
            nearby: 0,
            trees_at: Vec::new(),
            vessels: Vec::new(),
            behind: Vec::new(),
            own_edges: Vec::new(),
            outside: Vec::new(),
            turns: 0,
            tree_links: Vec::new(),
            tree_edges: Vec::new(),
            label_presets: Vec::new(),
            levels: Vec::new(),
            clusters: Vec::new(),
            length_cache: Default::default(),
        }
    }

    /// TALA `placementEdgeLengthState`: the key of the price cache, with the prices TALA's
    /// graph has stored at the time. `direction` says whether the price punishes arrows
    /// against the wanted direction.
    pub fn length_key(&self, direction: bool) -> Vec<u64> {
        // TALA's graph stores no price of a side port before routing.
        let (turn, crossing) = self.length_cache.stored();
        let mut key = vec![u64::from(direction), self.cell_size.to_bits(), turn.to_bits(), 0.0_f64.to_bits(), crossing.to_bits()];
        for (node, card) in self.nodes.iter().enumerate() {
            key.extend([self.id(node) as u64, card.top_left.x.to_bits(), card.top_left.y.to_bits(), card.width.to_bits(), card.height.to_bits()]);
            key.extend(self.indexed_edges_of(node).map(|(edge, _)| edge as u64));
            key.push(u64::MAX);
        }
        key
    }

    /// The cluster a card is in, by its place in `clusters` (TALA `Node.Cluster`).
    pub fn cluster_of(&self, node: usize) -> Option<usize> {
        self.clusters.iter().position(|cluster| cluster.members.contains(&node))
    }

    /// For a card of a hierarchy: the number of its hierarchy and its level in it.
    pub fn level(&self, node: usize) -> Option<(usize, i64)> {
        self.levels.get(node).copied().flatten()
    }

    /// For a card of a tree: what ties it to the card it hangs on.
    pub fn tree_link(&self, node: usize) -> Option<TreeLink> {
        self.tree_links.get(node).copied().flatten()
    }

    /// Whether one end of a connection is a card of a tree (TALA `Edge.IsTreeEdge`).
    pub fn hangs_in_tree(&self, edge: usize) -> bool {
        self.tree_link(self.edges[edge].from).is_some() || self.tree_link(self.edges[edge].to).is_some()
    }

    /// Whether TALA takes a connection for a tree connection when it routes.
    pub fn is_tree_edge(&self, edge: usize) -> bool {
        self.tree_edges.get(edge).copied().unwrap_or(false)
    }

    /// TALA `Node.Edges`: the connections of a card, in the order of its own list.
    pub fn edges_of(&self, node: usize) -> impl Iterator<Item = &Edge> {
        self.indexed_edges_of(node).map(|(_, edge)| edge)
    }

    /// The connections of a card as `edges_of` gives them, each with its index in `edges`.
    pub fn indexed_edges_of(&self, node: usize) -> impl Iterator<Item = (usize, &Edge)> {
        let (own, all) = match self.own_edges.get(node) {
            Some(own) => (own.as_slice(), 0..0),
            None => (&[][..], 0..self.edges.len()),
        };
        own.iter()
            .copied()
            .chain(all)
            .map(|index| (index, &self.edges[index]))
            .filter(move |(index, edge)| (edge.from == node || edge.to == node) && !self.hidden.contains(index))
    }

    /// The cluster a card is the vessel of.
    pub fn vessel_at(&self, node: usize) -> Option<&Vessel> {
        self.vessels.iter().find(|vessel| vessel.node == node)
    }

    /// For a connection with a vessel at one end: the vessel, and the card of its cluster the
    /// connection belongs to, where that card stands now.
    pub fn member_behind(&self, edge: usize) -> Option<(&Vessel, Node)> {
        let place = self.behind.get(edge).copied().flatten()?;
        let ends = [self.edges[edge].from, self.edges[edge].to];
        let vessel = self.vessels.iter().find(|vessel| ends.contains(&vessel.node))?;
        Some((vessel, vessel.member(self.nodes[vessel.node].top_left, place)))
    }

    /// TALA's number of a card (`Node.ID`).
    pub fn id(&self, node: usize) -> i64 {
        self.ids.get(node).copied().unwrap_or(node as i64)
    }

    /// TALA `Graph.Containers[nil]`, of the cards in this graph: the cards in the order of the
    /// whole diagram.
    pub fn cards_in_the_way(&self) -> impl Iterator<Item = usize> + '_ {
        let natural = if self.asking_order.is_empty() { 0..self.nodes.len() } else { 0..0 };
        self.asking_order.iter().copied().chain(natural)
    }

    pub fn are_connected(&self, a: usize, b: usize) -> bool {
        self.edges
            .iter()
            .any(|e| (e.from == a && e.to == b) || (e.from == b && e.to == a))
    }

    /// The gap the card `moving`, with its top-left corner at `at`, must keep to `other`:
    /// 20, or 60 when they are connected, or the room a label between them needs in the direction
    /// they lie to each other. TALA `deltaToGuarded`, for plain boxes: no tables, no self-loops
    /// and no label margins on the cards themselves.
    pub fn gap_between(&self, moving: usize, other: usize, at: Point) -> f64 {
        let mut connected = false;
        let mut label_width = f64::NEG_INFINITY;
        let mut label_height = f64::NEG_INFINITY;
        for edge in self.edges_of(moving) {
            if edge.other(moving) == other {
                connected = true;
                label_width = label_width.max(edge.min_width);
                label_height = label_height.max(edge.min_height);
            }
        }
        let base = if connected { CONNECTED_NODE_GAP } else { NODE_GAP };
        let horizontal = base.max(label_width);
        let vertical = base.max(label_height);
        if horizontal == vertical {
            return horizontal;
        }
        match self.side_of(moving, other, at) {
            Side::Above | Side::Below => vertical,
            Side::Left | Side::Right => horizontal,
            Side::Neither => horizontal.min(vertical),
        }
    }

    /// On which side of `other` the card `moving` is when its top-left corner is at `at`.
    fn side_of(&self, moving: usize, other: usize, at: Point) -> Side {
        let node = &self.nodes[moving];
        let other = &self.nodes[other];
        let beside = |otherwise: Side| {
            let left = at.x + node.width < other.top_left.x;
            let right = other.top_left.x + other.width < at.x;
            if left || right { Side::Neither } else { otherwise }
        };
        if at.y + node.height < other.top_left.y {
            return beside(Side::Above);
        }
        if other.top_left.y + other.height < at.y {
            return beside(Side::Below);
        }
        if other.top_left.x + other.width < at.x {
            return Side::Right;
        }
        if at.x + node.width < other.top_left.x {
            return Side::Left;
        }
        Side::Neither
    }
}

/// TALA `Graph.ComputeCellSize`: about one card per cell when the cards are of similar size.
pub fn compute_cell_size(nodes: &[Node]) -> f64 {
    let mut min_width = f64::INFINITY;
    let mut min_height = f64::INFINITY;
    let mut max_width = f64::NEG_INFINITY;
    let mut max_height = f64::NEG_INFINITY;
    for node in nodes {
        min_width = min_width.min(node.width);
        min_height = min_height.min(node.height);
        max_width = max_width.max(node.width);
        max_height = max_height.max(node.height);
    }
    let min_length = min_width.min(min_height);
    let max_length = max_width.max(max_height);
    let cell = if max_length < 3.0 * min_length {
        max_length.ceil()
    } else {
        (3.0 * min_length / 2.0).ceil()
    };
    cell.max(MIN_CELL_SIZE)
}
