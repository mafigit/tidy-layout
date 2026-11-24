// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.

//! The node placement stage for cards in no container: the cards are split into groups that
//! connections hold together, each group is placed on its own and turned so that most arrows
//! point right and down, and the groups are packed next to each other. Ported from TALA
//! `internal/placement`: `stages.go` (`Place`), `node_placement.go` (`placeNodes`), `direct.go`
//! (`direct`, `transformsTo`, `mirrorAxes`), `internal/layoutgraph/graph.go`
//! (`splitSubgraphsWithOwnership`, `boundingBox`, `area`) and `internal/packing/combine.go`
//! (`CombineSubgraphs`).
//!
//! `place` is what TALA does where its earlier stages found no tree, hierarchy or cluster among
//! the cards; `place_with_trees` where they did.

use crate::clusters::{Arrangement, Cluster, find_clusters, with_vessels};
use crate::gorand::GoRand;
use crate::gosort::sort_slice;
use crate::graph::{Edge, Graph, Node, Point, RoutedCluster, TreeLink, Vessel, compute_cell_size};
use crate::routing::labels::{LabelSize, Placed as PlacedLabel, Position, label_top_left};
use crate::hierarchy::find_hierarchies;
use crate::hierarchy_layout::place_hierarchies;
use crate::node_placement::{PlaceError, place_nodes_orthogonally};
use crate::optimizer::turn_cluster;
use crate::placementcost::edge_length::SIDE_EDGE_SPACING;
use crate::routing::router::Costs;
use crate::routing::geometry::Orientation;
use crate::tree_layout::place_trees;
use crate::trees::{Found, Tree, without_trees};

/// The room between two groups, where one is tried at a little distance from another.
const SUBGRAPH_PADDING: f64 = 20.0;
/// How much a packing that is not square costs.
const SUBGRAPH_SQUARE_DAMPENER: f64 = 0.5;

/// TALA `Prescale` (`scaleBasedOnEdges`): a card with many connections grows so that they find
/// room on its sides, 40 apart. With n connections on its fullest side (the most it shares with
/// one card, or a quarter of all of them, rounded up) it becomes at least (n + 1) * 40 wide and
/// high. A card with one connection per side stays as it is.
pub fn prescale(graph: &mut Graph) {
    for node in 0..graph.nodes.len() {
        // TALA leaves out a connection from a card to itself; the engine is given none.
        let mut per_card: Vec<(usize, usize)> = Vec::new();
        for edge in graph.edges_of(node) {
            let adjacent = edge.other(node);
            match per_card.iter_mut().find(|(card, _)| *card == adjacent) {
                Some((_, count)) => *count += 1,
                None => per_card.push((adjacent, 1)),
            }
        }
        let total: usize = per_card.iter().map(|(_, count)| count).sum();
        let most = per_card.iter().map(|(_, count)| *count).max().unwrap_or(0);
        // TALA divides by the number of connected cards where they are fewer than four; the
        // most shared with one card is never less than that share.
        let per_side = most.max(total.div_ceil(4));
        // No connection at all, or one per side.
        if per_side <= 1 {
            continue;
        }
        let least = (per_side + 1) as f64 * SIDE_EDGE_SPACING;
        let card = &mut graph.nodes[node];
        card.width = card.width.max(least);
        card.height = card.height.max(least);
    }
}

/// TALA `splitSubgraphsWithOwnership`: the groups of cards that connections hold together, in
/// the order of their first cards; in each group the first card, then the cards connections
/// lead to, nearest first.
pub fn split(graph: &Graph) -> Vec<Vec<usize>> {
    let mut added = vec![false; graph.nodes.len()];
    let mut groups = Vec::new();
    for start in 0..graph.nodes.len() {
        if added[start] {
            continue;
        }
        added[start] = true;
        let mut group = vec![start];
        let mut next = 0;
        while next < group.len() {
            let current = group[next];
            next += 1;
            for edge in graph.edges_of(current) {
                let adjacent = edge.other(current);
                if !added[adjacent] {
                    added[adjacent] = true;
                    group.push(adjacent);
                }
            }
        }
        groups.push(group);
    }
    groups
}

/// A group as a graph of its own: its cards in the group's order with their numbers, its
/// connections in the order of the whole graph, the cell size of its cards, and which of them
/// trees hang on.
pub(crate) fn subgraph(graph: &Graph, group: &[usize]) -> Graph {
    let place_of = |card: usize| group.iter().position(|member| *member == card);
    let nodes: Vec<Node> = group.iter().map(|card| graph.nodes[*card].clone()).collect();
    let edges: Vec<Edge> = graph
        .edges
        .iter()
        .filter_map(|edge| Some(Edge { from: place_of(edge.from)?, to: place_of(edge.to)?, ..*edge }))
        .collect();
    // Asked in the order of the whole diagram.
    let mut asking_order: Vec<usize> = (0..group.len()).collect();
    asking_order.sort_by_key(|place| group[*place]);
    let trees_at = graph.trees_at.iter().filter_map(|card| place_of(*card)).collect();
    let vessels = graph.vessels.iter().filter_map(|vessel| Some(Vessel { node: place_of(vessel.node)?, ..vessel.clone() })).collect();
    let behind = graph
        .edges
        .iter()
        .enumerate()
        .filter(|(_, edge)| place_of(edge.from).is_some() && place_of(edge.to).is_some())
        .map(|(index, _)| graph.behind.get(index).copied().flatten())
        .collect();
    Graph { ids: group.iter().map(|card| graph.id(*card)).collect(), asking_order, trees_at, vessels, behind, ..Graph::new(nodes, edges) }
}

/// TALA `direct` for cards with no direction of their own: the group is mirrored so that the
/// direction most arrows point in is right or down, and the next one too where it has more
/// arrows than the least. An arrow on a diagonal counts for both of its directions.
///
/// Only the arrows at one of the first `own` cards count. Tree cards come after those: TALA has
/// handed them back to the whole diagram by now, and `containerEdgeDirections` passes over a
/// card of another graph. So an arrow between two cards of a tree does not count, unless the
/// tree hangs on a diagram's only card: that one is never split off.
///
/// Says whether it mirrored left to right, and top to bottom.
fn direct(graph: &mut Graph, own: usize) -> (bool, bool) {
    let (mirror_x, mirror_y) = mirrors(graph, own);
    mirror(graph, mirror_x, mirror_y);
    (mirror_x, mirror_y)
}

/// TALA `containerEdgeDirections` and `transformsTo`: whether a group has to be mirrored left
/// to right, and top to bottom, so that most of its arrows point right and down.
pub(crate) fn mirrors(graph: &Graph, own: usize) -> (bool, bool) {
    // Right, down, left, up: in this order, so that of equal counts right and down come first.
    let mut counts = [(Orientation::Right, 0), (Orientation::Bottom, 0), (Orientation::Left, 0), (Orientation::Top, 0)];
    for edge in graph.edges.iter().filter(|edge| edge.from < own || edge.to < own) {
        // Where the arrow's end lies, seen from its start.
        let (right, down, left, up) = match graph.nodes[edge.to].orientation(&graph.nodes[edge.from]) {
            Orientation::Left => (0, 0, 1, 0),
            Orientation::TopLeft => (0, 0, 1, 1),
            Orientation::BottomLeft => (0, 1, 1, 0),
            Orientation::Right => (1, 0, 0, 0),
            Orientation::TopRight => (1, 0, 0, 1),
            Orientation::BottomRight => (1, 1, 0, 0),
            Orientation::Top => (0, 0, 0, 1),
            Orientation::Bottom => (0, 1, 0, 0),
            Orientation::None => (0, 0, 0, 0),
        };
        for (count, more) in counts.iter_mut().zip([right, down, left, up]) {
            count.1 += more;
        }
    }
    counts.sort_by_key(|(_, count)| std::cmp::Reverse(*count));
    let horizontal = |direction: Orientation| matches!(direction, Orientation::Left | Orientation::Right);
    let opposite = |direction: Orientation| match direction {
        Orientation::Right => Orientation::Left,
        Orientation::Left => Orientation::Right,
        Orientation::Bottom => Orientation::Top,
        _ => Orientation::Bottom,
    };
    let primary = counts[0];
    let secondary = if counts[1].0 == opposite(primary.0) { counts[2] } else { counts[1] };
    let (mut mirror_x, mut mirror_y) = (primary.0 == Orientation::Left, primary.0 == Orientation::Top);
    if secondary.1 > counts[3].1 {
        if horizontal(secondary.0) {
            mirror_x = secondary.0 != Orientation::Right;
        } else {
            mirror_y = secondary.0 != Orientation::Bottom;
        }
    }
    (mirror_x, mirror_y)
}

/// TALA `mirrorAxes` for cards with nothing inside. A tree that lies along a mirrored axis
/// lies on the other side of the card it hangs on afterwards.
pub(crate) fn mirror(graph: &mut Graph, mirror_x: bool, mirror_y: bool) {
    for card in &mut graph.nodes {
        if mirror_x {
            card.top_left.x = -card.top_left.x - card.width;
        }
        if mirror_y {
            card.top_left.y = -card.top_left.y - card.height;
        }
    }
    for link in graph.tree_links.iter_mut().flatten() {
        let sideways = matches!(link.side, Orientation::Left | Orientation::Right);
        if (mirror_x && sideways) || (mirror_y && !sideways) {
            link.side = link.side.opposite();
        }
    }
}

/// TALA `Graph.BoundingBox`: the corners of the box around a group, on whole pixels.
pub(crate) fn bounds(graph: &Graph, group: &[usize]) -> (Point, Point) {
    let mut top_left = Point { x: f64::INFINITY, y: f64::INFINITY };
    let mut bottom_right = Point { x: f64::NEG_INFINITY, y: f64::NEG_INFINITY };
    for &card in group {
        let card = &graph.nodes[card];
        top_left = Point { x: top_left.x.min(card.top_left.x), y: top_left.y.min(card.top_left.y) };
        bottom_right = Point { x: bottom_right.x.max(card.right().round()), y: bottom_right.y.max(card.bottom().round()) };
    }
    (Point { x: top_left.x.round(), y: top_left.y.round() }, Point { x: bottom_right.x.round(), y: bottom_right.y.round() })
}

/// TALA `CombineSubgraphs`: the groups, largest first, each at the corner of a group placed
/// before it where the box around all of them stays smallest and nearest to a square.
fn combine(graph: &mut Graph, groups: &[Vec<usize>]) {
    let area = |graph: &Graph, group: &[usize]| {
        let (top_left, bottom_right) = bounds(graph, group);
        (top_left.x - bottom_right.x).abs() * (top_left.y - bottom_right.y).abs()
    };
    // Go's sort does not keep groups of the same area in their order.
    let mut by_area: Vec<(&Vec<usize>, f64)> = groups.iter().map(|group| (group, area(graph, group))).collect();
    sort_slice(&mut by_area, |a, b| a.1 > b.1);

    let mut combined: Vec<usize> = Vec::new();
    let mut combined_end = Point { x: f64::NEG_INFINITY, y: f64::NEG_INFINITY };
    let mut candidates: Vec<Point> = Vec::new();
    for (group, _) in by_area {
        let (mut top_left, mut bottom_right) = bounds(graph, group);
        let (width, height) = (bottom_right.x - top_left.x, bottom_right.y - top_left.y);
        let mut least = f64::INFINITY;
        // The first group goes to the origin, and so does one that fits nowhere.
        let mut best = (0, Point { x: 0.0, y: 0.0 });
        let mut fits_somewhere = false;
        for (index, candidate) in candidates.iter().enumerate() {
            let (dx, dy) = (candidate.x - top_left.x, candidate.y - top_left.y);
            // Cards of different groups share no connection: they keep the gap of strangers.
            let collides = group.iter().any(|&card| {
                let at = graph.nodes[card].top_left;
                let point = Point { x: at.x + dx, y: at.y + dy };
                combined.iter().any(|&other| crate::compaction::overlaps_at(graph, card, other, point))
            });
            if collides {
                continue;
            }
            let (new_width, new_height) = (combined_end.x.max(candidate.x + width), combined_end.y.max(candidate.y + height));
            let cost = new_width * new_height + (new_width - new_height) * (new_width - new_height) * SUBGRAPH_SQUARE_DAMPENER;
            if !fits_somewhere || cost < least {
                (fits_somewhere, least, best) = (true, cost, (index, *candidate));
            }
        }
        if !candidates.is_empty() {
            candidates.swap_remove(best.0);
        }
        let (dx, dy) = (best.1.x - top_left.x, best.1.y - top_left.y);
        for &card in group {
            let at = graph.nodes[card].top_left;
            graph.nodes[card].top_left = Point { x: at.x + dx, y: at.y + dy };
            combined.push(card);
        }
        top_left = Point { x: top_left.x + dx, y: top_left.y + dy };
        bottom_right = Point { x: bottom_right.x + dx, y: bottom_right.y + dy };
        combined_end = Point { x: combined_end.x.max(bottom_right.x), y: combined_end.y.max(bottom_right.y) };
        // The three free corners of the group, touching it and a little away from it.
        candidates.extend([
            Point { x: bottom_right.x, y: top_left.y },
            Point { x: bottom_right.x, y: bottom_right.y },
            Point { x: top_left.x, y: bottom_right.y },
            Point { x: bottom_right.x + SUBGRAPH_PADDING, y: top_left.y },
            Point { x: bottom_right.x + SUBGRAPH_PADDING, y: bottom_right.y },
            Point { x: top_left.x, y: bottom_right.y + SUBGRAPH_PADDING },
        ]);
    }
}

/// TALA `placement.Place` for a diagram with trees, hierarchies or clusters: the cards the tree
/// stage left are placed, group by group, and each group's trees are laid out around it before
/// it is turned and packed. A group that is a hierarchy is laid out in layers instead. A
/// cluster is placed as one card, its vessel, and its cards get their places in it at the end.
/// Gives the diagram as TALA has it then.
pub fn place_with_trees(whole: &mut Graph, found: &Found, seed: i64) -> Result<Placed, PlaceError> {
    let core = without_trees(whole, found);
    let levels = find_hierarchies(&core);
    let layered = place_hierarchies(&core, &levels, seed);
    let mut clusters = find_clusters(&core, &levels, whole.nodes.len(), seed);
    let (vesseled, from) = with_vessels(&core, &clusters);
    // The card of the whole diagram for each card node placement works on. A vessel is none of
    // them: it gets a number no card has.
    let vessel = |cluster: usize| usize::MAX - cluster;
    let cards_kept = vesseled.nodes.len() - clusters.len();
    let card_of = |node: usize| match from[node] {
        Some(card) => found.order[card],
        None => vessel(node - cards_kept),
    };
    // A card alone is not placed: it goes to the origin, and is not packed.
    let alone = vesseled.nodes.len() == 1;
    let groups = if alone { vec![vec![0]] } else { split(&vesseled) };
    // A hierarchy is laid out already. It is neither placed nor turned, and no tree hangs on it.
    let hierarchy = |group: &[usize]| group.iter().map(|node| from[*node].and_then(|card| layered[card])).collect::<Option<Vec<Point>>>();
    // The cards outside a group that have a place while the group is placed, each group still
    // around its own origin: the hierarchies, and the groups placed before it. TALA asks them
    // too whether a cluster may turn (`turn_cluster`).
    let mut outside: Vec<Node> = Vec::new();
    for group in &groups {
        if let Some(places) = hierarchy(group) {
            outside.extend(group.iter().zip(places).map(|(node, top_left)| Node { top_left, ..vesseled.nodes[*node].clone() }));
        }
    }
    let mut placed: Vec<Vec<usize>> = Vec::new();
    // The tree cards in the order they came back into the graph, each with the connection it
    // hangs by: TALA's top-level list and the cards' own lists of connections have them so.
    let mut reconnected: Vec<(usize, Edge)> = Vec::new();
    // Per tree card: the side of the card it hangs on that its tree lies on, and where the
    // tree stage wants the label of the connection it hangs by.
    let mut hung: Vec<(usize, Orientation, Option<(Position, f64)>)> = Vec::new();
    // Per cluster: its vessel as it was placed.
    let mut vessels: Vec<Option<(Node, Vessel)>> = vec![None; clusters.len()];
    for group in &groups {
        let mut part = subgraph(&vesseled, group);
        let mut cards: Vec<usize> = group.iter().map(|node| card_of(*node)).collect();
        let mut mirrored = (false, false);
        if let Some(places) = hierarchy(group) {
            for (card, place) in part.nodes.iter_mut().zip(places) {
                card.top_left = place;
            }
        } else {
            if alone {
                part.nodes[0].top_left = Point { x: 0.0, y: 0.0 };
            } else {
                part.outside = outside.clone();
                place_nodes_orthogonally(&mut part, seed, &mut GoRand::new(seed), |_, _, _| {})?;
                for vessel in 0..part.vessels.len() {
                    turn_cluster(&mut part, vessel);
                }
            }
            place_trees(&mut part, &mut cards, whole, found);
            let own = if alone { part.nodes.len() } else { group.len() };
            mirrored = direct(&mut part, own);
            outside.extend(part.nodes.iter().cloned());
            let hangs_by = part.edges.len() - (part.nodes.len() - group.len());
            reconnected.extend(cards[group.len()..].iter().copied().zip(part.edges[hangs_by..].iter().copied()));
            for (node, card) in cards.iter().enumerate().skip(group.len()) {
                let link = part.tree_link(node).expect("a tree card knows its side");
                hung.push((*card, link.side, part.label_presets.get(link.edge).copied().flatten()));
            }
        }
        // A cluster that turns for good puts the cards of every cluster in order: TALA's
        // transaction for it is one on the whole diagram (`SyncClusters`).
        if part.turns > 0 {
            for (_, vessel) in vessels.iter_mut().flatten() {
                vessel.backwards = false;
            }
        }
        let mut members: Vec<usize> = Vec::new();
        for (card, node) in cards.iter().zip(&part.nodes) {
            let Some(cluster) = (0..clusters.len()).find(|cluster| vessel(*cluster) == *card) else {
                whole.nodes[*card].top_left = node.top_left;
                continue;
            };
            // The cluster may have turned while it was placed.
            let now = part.vessels.iter().find(|vessel| cards[vessel.node] == *card).expect("the vessel is in its group");
            // Where the group was mirrored along the cluster, TALA has mirrored every card:
            // they stand in the opposite order.
            let backwards = if now.arrangement == Arrangement::Row { mirrored.0 } else { mirrored.1 };
            vessels[cluster] = Some((node.clone(), Vessel { backwards, ..now.clone() }));
            members.extend(clusters[cluster].members.iter().map(|member| found.order[*member]));
        }
        cards.retain(|card| *card < whole.nodes.len());
        cards.extend(members);
        placed.push(cards);
    }
    // The cards of a cluster stand in its vessel, in their order or backwards.
    for (cluster, (node, now)) in clusters.iter_mut().zip(vessels.iter().flatten()) {
        cluster.arrangement = now.arrangement;
        for (place, member) in cluster.members.iter().enumerate() {
            whole.nodes[found.order[*member]] = now.member(node.top_left, place);
        }
    }
    if !alone {
        combine(whole, &placed);
    }
    for cluster in &mut clusters {
        cluster.members = cluster.members.iter().map(|member| found.order[*member]).collect();
    }

    // TALA's graph after the stage: the cards outside clusters and the vessels in the order
    // node placement started from, then the tree cards.
    let mut graph = vesseled.clone();
    let mut cards: Vec<Option<usize>> = from.iter().map(|card| card.map(|card| found.order[card])).collect();
    for (node, card) in cards.iter().enumerate() {
        if let Some(card) = card {
            graph.nodes[node].top_left = whole.nodes[*card].top_left;
        }
    }
    for ((cluster, vessel), (_, now)) in clusters.iter().zip(&mut graph.vessels).zip(vessels.iter().flatten()) {
        *vessel = Vessel { node: vessel.node, ..now.clone() };
        // The vessel is the box around its cards, where packing left them.
        let corner = |pick: fn(&Node) -> f64| cluster.members.iter().map(|member| pick(&whole.nodes[*member])).fold(f64::INFINITY, f64::min);
        let (width, height) = vessel.size();
        graph.nodes[vessel.node] = Node { top_left: Point { x: corner(|card| card.top_left.x), y: corner(|card| card.top_left.y) }, width, height };
    }
    // The tree cards and their connections come back group by group, per card that trees hang
    // on, per tree: the cards below the root level by level, then the root. Around a card that
    // is alone they come in the order the trees were laid out.
    for (card, _) in reconnected.iter().filter(|_| alone) {
        graph.nodes.push(whole.nodes[*card].clone());
        graph.ids.push(whole.id(*card));
        cards.push(Some(*card));
    }
    for group in groups.iter().filter(|_| !alone) {
        let sentinels: Vec<usize> = group.iter().filter_map(|node| cards[*node]).collect();
        for sentinel in sentinels {
            let Some((_, roots)) = found.trees.iter().find(|(card, _)| *card == sentinel) else {
                continue;
            };
            for root in roots {
                let mut below: Vec<&Tree> = Vec::new();
                let mut next = vec![root];
                while !next.is_empty() {
                    let children: Vec<&Tree> = next.iter().flat_map(|tree| &tree.children).collect();
                    // Level by level means: all children of one card, then those of the next.
                    below.extend(children.iter().copied());
                    next = children;
                }
                for tree in below.into_iter().chain([root]) {
                    graph.nodes.push(whole.nodes[tree.node].clone());
                    graph.ids.push(whole.id(tree.node));
                    cards.push(Some(tree.node));
                }
            }
        }
    }
    let node_of = |card: usize| cards.iter().position(|other| *other == Some(card)).expect("every card outside a cluster is in the graph");
    let core_edges = graph.edges.len();
    for card in cards[vesseled.nodes.len()..].iter().map(|card| card.expect("a tree card")) {
        let (_, edge) = reconnected.iter().find(|(other, _)| *other == card).expect("every tree card came back");
        let ends = found_edge(whole, found, card);
        graph.edges.push(Edge { from: node_of(ends.0), to: node_of(ends.1), ..*edge });
    }
    // TALA's top-level list, and every card's own list of connections: what was there before
    // the trees came back, then the tree cards and their connections in the order they came.
    graph.asking_order = (0..vesseled.nodes.len()).chain(reconnected.iter().map(|(card, _)| node_of(*card))).collect();
    graph.own_edges = (0..graph.nodes.len())
        .map(|node| (0..core_edges).filter(|edge| graph.edges[*edge].from == node || graph.edges[*edge].to == node).collect())
        .collect();
    for (card, _) in &reconnected {
        let edge = core_edges + (node_of(*card) - vesseled.nodes.len());
        for end in [graph.edges[edge].from, graph.edges[edge].to] {
            graph.own_edges[end].push(edge);
        }
    }
    graph.cell_size = compute_cell_size(&graph.nodes);
    whole.cell_size = graph.cell_size;
    graph.behind.resize(graph.edges.len(), None);
    graph.outside = Vec::new();
    let layered = cards.iter().map(|card| card.is_some_and(|card| found.order.iter().position(|other| *other == card).is_some_and(|core| layered[core].is_some()))).collect();
    // TALA `buildIsTreeEdgeMap` and `addIsolatedTreeEdgesToMap`: the connections inside a tree,
    // and those of its roots where the card the tree hangs on has no other connections.
    let tree_edges = (0..graph.edges.len())
        .map(|edge| {
            edge >= core_edges
                && [graph.edges[edge].from, graph.edges[edge].to]
                    .iter()
                    .all(|end| *end >= vesseled.nodes.len() || graph.own_edges[*end].iter().all(|own| *own >= core_edges))
        })
        .collect();
    // What routing and the label stage need of the trees (TALA `NodeToTree`).
    let tree_edges: Vec<bool> = tree_edges;
    graph.tree_links = vec![None; graph.nodes.len()];
    graph.label_presets = vec![None; graph.edges.len()];
    for node in vesseled.nodes.len()..graph.nodes.len() {
        let (_, side, preset) = hung.iter().find(|(card, _, _)| Some(*card) == cards[node]).expect("every tree card was laid out");
        let edge = core_edges + (node - vesseled.nodes.len());
        graph.tree_links[node] = Some(TreeLink { edge, side: *side });
        graph.label_presets[edge] = *preset;
    }
    graph.tree_edges = tree_edges.clone();
    // And of the hierarchies (TALA `Node.Hierarchy`): the group a card is in, and its level.
    graph.levels = (0..graph.nodes.len())
        .map(|node| {
            let core = found.order.iter().position(|other| Some(*other) == cards[node])?;
            let group = groups.iter().position(|group| group.contains(&node))?;
            Some((group, levels[core]? as i64))
        })
        .collect();
    Ok(Placed { graph, cards, clusters, tree_cards: vesseled.nodes.len(), tree_edges, layered, turn: None, crossing: None })
}

/// The two cards at the ends of the connection a tree card hangs by.
fn found_edge(whole: &Graph, found: &Found, card: usize) -> (usize, usize) {
    fn search(tree: &Tree, card: usize) -> Option<usize> {
        if tree.node == card {
            return Some(tree.edge);
        }
        tree.children.iter().find_map(|child| search(child, card))
    }
    let edge = found.trees.iter().flat_map(|(_, roots)| roots).find_map(|root| search(root, card)).expect("a tree card");
    (whole.edges[edge].from, whole.edges[edge].to)
}

/// The diagram as TALA has it after the node placement stage.
pub struct Placed {
    /// TALA's graph: the cards outside clusters, the vessels and the tree cards, in its order
    /// and at their places. `asking_order` is its top-level list, `own_edges` every card's own
    /// list of connections.
    pub graph: Graph,
    /// Per card of `graph`: the card of the diagram, or none for a vessel.
    pub cards: Vec<Option<usize>>,
    /// The clusters as they are then, their cards as cards of the diagram. `graph.vessels` has
    /// their vessels, in the same order.
    pub clusters: Vec<Cluster>,
    /// The cards of `graph` from this one on are tree cards (TALA `NodeToTree`).
    pub tree_cards: usize,
    /// Per connection of `graph`: whether TALA takes it for a tree connection, which the
    /// stage that lines cards up leaves alone.
    pub tree_edges: Vec<bool>,
    /// Per card of `graph`: whether it is in a hierarchy.
    pub layered: Vec<bool>,
    /// TALA `Graph.TurnCost` and `CrossingCost` of the whole diagram: each is taken when a
    /// stage first asks for it, from the places then, and kept. The stage that normalizes gaps
    /// forgets the price of a bend at its end.
    pub turn: Option<f64>,
    pub crossing: Option<f64>,
}

impl Placed {
    /// The prices of a bend and of a crossing, taken now where they are not known yet.
    pub fn prices(&mut self) -> Costs {
        let now = Costs::of(&self.graph);
        Costs { turn: *self.turn.get_or_insert(now.turn), crossing: *self.crossing.get_or_insert(now.crossing), ..now }
    }

    /// Where every card of the diagram stands: the cards of the graph, and the cards of the
    /// clusters in their vessels.
    pub fn places(&self, cards: usize) -> Vec<Point> {
        let mut at = vec![Point { x: 0.0, y: 0.0 }; cards];
        for (node, card) in self.cards.iter().enumerate() {
            if let Some(card) = card {
                at[*card] = self.graph.nodes[node].top_left;
            }
        }
        for (cluster, vessel) in self.clusters.iter().zip(&self.graph.vessels) {
            for (place, member) in cluster.members.iter().enumerate() {
                at[*member] = vessel.member(self.graph.nodes[vessel.node].top_left, place).top_left;
            }
        }
        at
    }
}

/// TALA `grouping.Cleanup` as routing sees its result: the graph routing starts from. The
/// vessels leave the graph; the cards of every cluster come back at its end, cluster after
/// cluster in the order of the vessels' numbers (`ClusterOrder`), each cluster's cards in its
/// order, from the vessel's corner on (`ArrangeClusterNodes`). A connection that ended at a
/// vessel ends at its card again. The connections keep their order, every card its own list
/// of them; a card of a cluster lists its connections in the order of the graph. With the
/// numbers the cards have in the diagram, and what the placement knows of trees, hierarchies
/// and clusters.
pub fn cleanup(placed: &Placed) -> Graph {
    let graph = &placed.graph;
    let kept: Vec<usize> = (0..graph.nodes.len()).filter(|node| placed.cards[*node].is_some()).collect();
    let mut order: Vec<usize> = (0..placed.clusters.len()).collect();
    order.sort_by_key(|cluster| placed.clusters[*cluster].vessel);
    let mut nodes: Vec<Node> = kept.iter().map(|node| graph.nodes[*node].clone()).collect();
    let mut ids: Vec<i64> = kept.iter().map(|node| graph.id(*node)).collect();
    let mut clusters = Vec::new();
    // Per vessel of the graph and place in its cluster: the card's place in the new graph.
    let mut member_at: Vec<(usize, usize, usize)> = Vec::new();
    for &cluster in &order {
        let vessel = &graph.vessels[cluster];
        let at = graph.nodes[vessel.node].top_left;
        let mut members = Vec::new();
        for (place, card) in placed.clusters[cluster].members.iter().enumerate() {
            member_at.push((vessel.node, place, nodes.len()));
            members.push(nodes.len());
            nodes.push(Vessel { backwards: false, ..vessel.clone() }.member(at, place));
            ids.push(*card as i64);
        }
        clusters.push(RoutedCluster { members, arrangement: vessel.arrangement, desired: vessel.desired });
    }
    let new_place = |node: usize| kept.iter().position(|other| *other == node);
    let edges: Vec<Edge> = graph
        .edges
        .iter()
        .enumerate()
        .map(|(index, edge)| {
            let end = |node: usize| {
                new_place(node).unwrap_or_else(|| {
                    let place = graph.behind[index].expect("a connection that ends at a vessel knows its card");
                    member_at.iter().find(|(vessel, at, _)| *vessel == node && *at == place).expect("the card is in its cluster").2
                })
            };
            Edge { from: end(edge.from), to: end(edge.to), ..*edge }
        })
        .collect();
    let levels = if graph.levels.is_empty() { Vec::new() } else { kept.iter().map(|node| graph.level(*node)).chain(member_at.iter().map(|_| None)).collect() };
    let tree_links = if graph.tree_links.is_empty() { Vec::new() } else { kept.iter().map(|node| graph.tree_link(*node)).chain(member_at.iter().map(|_| None)).collect() };
    // A card outside the clusters keeps its own list; a card of a cluster lists its
    // connections in the order of the graph.
    let own_edges = if graph.own_edges.is_empty() {
        Vec::new()
    } else {
        let mut own: Vec<Vec<usize>> = kept.iter().map(|node| graph.own_edges[*node].clone()).collect();
        for (_, _, node) in &member_at {
            own.push((0..edges.len()).filter(|edge| edges[*edge].from == *node || edges[*edge].to == *node).collect());
        }
        own
    };
    Graph {
        ids,
        own_edges,
        tree_links,
        tree_edges: graph.tree_edges.clone(),
        label_presets: graph.label_presets.clone(),
        levels,
        clusters,
        ..Graph::new(nodes, edges)
    }
}

/// TALA `placementPadding`: how far the diagram is moved from the origin before routing.
const PLACEMENT_PADDING: f64 = 1000.0;

/// TALA `grouping.Cleanup` and `placement.Pad`, the stages `CleanupStuff` and `Rescale`. The
/// cards of a cluster are cards of the diagram again, where they stand in their vessel:
/// `Placed::places` gives them there. Then every card moves across and down by the padding.
pub fn pad(placed: &mut Placed) {
    // TALA takes the cell size anew here, from the cards without the vessels. Nothing that is
    // ported reads it after this.
    for node in &mut placed.graph.nodes {
        node.top_left.x += PLACEMENT_PADDING;
        node.top_left.y += PLACEMENT_PADDING;
    }
}

/// TALA `placement.Normalize`, the last stage: the whole drawing moves so that its leftmost
/// and its topmost point lie at 0: a corner of a card, a point of a route or a corner of a
/// label, the last two rounded down. A label keeps its place along its route.
pub fn normalize(graph: &mut Graph, routes: &mut [Vec<Point>], labels: &mut [Option<PlacedLabel>], sizes: &[Option<LabelSize>]) {
    let (mut least_x, mut least_y) = (f64::INFINITY, f64::INFINITY);
    for node in &graph.nodes {
        (least_x, least_y) = (least_x.min(node.top_left.x), least_y.min(node.top_left.y));
    }
    let corner = |route: &[Point], label: &Option<PlacedLabel>, size: &Option<LabelSize>| Some(label_top_left(route, label.as_ref()?.position, label.as_ref()?.percentage, (*size)?)).filter(|_| route.len() >= 2);
    for ((route, label), size) in routes.iter().zip(labels.iter()).zip(sizes) {
        for point in route.iter().copied().chain(corner(route, label, size)) {
            (least_x, least_y) = (least_x.min(point.x.floor()), least_y.min(point.y.floor()));
        }
    }
    if graph.nodes.is_empty() {
        return;
    }
    for node in &mut graph.nodes {
        node.top_left = Point { x: node.top_left.x - least_x, y: node.top_left.y - least_y };
    }
    for ((route, label), size) in routes.iter_mut().zip(labels.iter_mut()).zip(sizes) {
        for point in route.iter_mut() {
            *point = Point { x: point.x - least_x, y: point.y - least_y };
        }
        if let (Some(at), Some(label)) = (corner(route, label, size), label.as_mut()) {
            label.top_left = at;
        }
    }
}

/// TALA `placement.Place`: a place for every card, from nothing. `seed` chooses TALA's random
/// numbers; every group starts them anew.
pub fn place(graph: &mut Graph, seed: i64) -> Result<(), PlaceError> {
    match graph.nodes.len() {
        0 => return Ok(()),
        1 => graph.nodes[0].top_left = Point { x: 0.0, y: 0.0 },
        _ => {
            let groups = split(graph);
            for group in &groups {
                let mut alone = subgraph(graph, group);
                place_nodes_orthogonally(&mut alone, seed, &mut GoRand::new(seed), |_, _, _| {})?;
                let own = alone.nodes.len();
                direct(&mut alone, own);
                for (card, placed) in group.iter().zip(&alone.nodes) {
                    graph.nodes[*card].top_left = placed.top_left;
                }
            }
            combine(graph, &groups);
        }
    }
    graph.cell_size = compute_cell_size(&graph.nodes);
    Ok(())
}
