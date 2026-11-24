# Porting TALA's placement

Goal: cards that go where TALA would put them. First one card at a time (a dropped card), then
all of them ("Arrange"). TALA's placement is `internal/placement` (about 9,400 lines of Go without
tests) and `internal/placementcost` (3,000), driven by `internal/engine` (800) with `packing`,
`proximity`, `quality`, `grouping`, `hierarchy` and `trees` (another 13,800) around it.
We port what plain cards need, one slice at a time, each with its own recorded reference
([reference.md](reference.md)).

## How TALA places cards

`engine/pipeline.go` runs stages in a fixed order. For plain cards the ones that move cards are:
`NodePlacement` (`placeNodesOrthogonally`: a first arrangement, then simulated annealing with a
"sizeless" optimizer on a coarse grid, compaction, and a "sized" optimizer), then `SwapStuff`,
`Transpose`, `AlignAxes`, `GapNormalization`, `BalanceSymmetry`, `Equidistance`, `BinPack` and
`CleanupStuff`. Routing comes after.

The sized optimizer repeats one move: take a card, find the middle of the cards it is connected
to, look at the free places around that point, price the card at each of them, and move it to the
cheapest. That move is slice 1.

## Slices

| # | Slice | TALA | State |
|---|---|---|---|
| 1 | One move of one card: the places around a point, the price of the card at a place, the choice | `sized_optimizer.go` (`fillPlacementPointsGuarded`, `moveNodeToBestGuarded`, `LongDistanceNeighborRequirements` in `newSizedOptimizer`), `placementcost` (`NodeEdgeLength`, `NodeSymmetry`, `flowContinuityCost`) | Done: `src/placement.rs`, `src/placementcost/` |
| 2 | `snap` in `src/api.rs` makes that move for a card dropped on a taken place | ours | Done |
| 3 | One pass of the sized optimizer over all cards: the point a card is drawn to, the move, the swap with a neighbour, the turn of a branch around a card, hubs | `optimizeGuarded`, `medianPointGuarded`, `bestSwapCandidateGuarded`, `withHubSpokesSuppressed`, `transpose.go`, `proximity.AddHubs`, and what a transaction's `Commit` refuses | Done: `src/optimizer.rs` |
| 4 | Go's random numbers (`math/rand` with a seed) and TALA's `Shuffle`, so that TALA's random choices can be replayed | Go standard library, `limits/optimization.go` | Done: `src/gorand.rs` |
| 5 | Node placement for one connected group of cards: the first arrangement, the sizeless optimizer, compaction, the loop that cools down, joining groups that drift apart | `node_placement.go` (`placeNodesOrthogonally`), `initialize.go`, `stress_initialize.go`, `sizeless_optimizer.go`, `compaction.go`, `moves.go`, `grouping/join.go` | Done: `src/node_placement.rs`, `sizeless.rs`, `compaction.rs`, `join.rs` |
| 6a | The node placement stage for cards in several groups: cards with many connections grow, the groups are split, placed alone, turned to their flow direction and packed. Go's sort, for the order of equal groups | `stages.go` (`Prescale`, `Place`), `placeNodes`, `direct.go`, `packing/combine.go`, Go's `sort.Slice` | Done: `src/place.rs`, `gosort.rs` |
| 6b | The tree stage, first half: finding trees, and the order of cards and connections it leaves | `trees/layout.go` (`Preprocess`, `extractTreesInContainer`, `handleTreeSubgraph`, `putBackNonBranchingTrees`), `preprocess_helpers.go` | Done: `src/trees.rs` |
| 6b | The tree stage, second half: laying the trees out around the cards they hang on, inside node placement, and where the labels of their connections are to go | `trees/layout.go` (`Place`, `placeTree`, `placeAtOrientationGuarded`, `positionTreeEdgeLabels`), `geometry.go` (`layoutTree`, `positionEdgeLabels`), `layoutgraph/transaction.go` (`Commit`) | Done: `src/tree_layout.rs`, `place::place_with_trees` |
| 6c | Hierarchies, first half: finding them. Rings are broken, the cards are ranked into levels, and shape and flow decide whether the group is one | `hierarchy/eligibility.go`, `discovery.go` (`Assign`, `build`, `isValid`, `makeSimpleDAG`), `rank.go`, `rank_graph.go`, `rank_simplex.go` | Done: `src/hierarchy.rs` |
| 6c | Hierarchies, second half: laying one out in layers, and what node placement does with it | `hierarchy/placement.go`, `crossing.go`, `sifting.go`, `brandes_kopf.go` | Done: `src/hierarchy_layout.rs` |
| 6d | Clusters, first half: finding them. Cards that are alike in size, neighbours and arrows become one card, the vessel | `grouping/clusters.go` (`AddClusters`, `AssignArrangement`, `PaddingBetween`, `CreateVessel`, `AddCluster`), `cluster_discovery.go`, `layoutgraph/cluster.go` | Done: `src/clusters.rs` |
| 6d | Clusters, second half: node placement with a vessel. The price of a connection to a cluster, the point a vessel is drawn to, the turn of a cluster from a row into a column | `placementcost/edge_length.go`, `flow_continuity.go`, `placement/sized_optimizer.go` (`protrudingChildren`), `optimization_guard.go` (`optimizerAdjacents`), `cluster_optimization.go` (`optimizeCluster` with `onlyFlip`), `transpose.go`, `direct.go` (`mirrorAxes`) | Done: in `src/placementcost/`, `optimizer.rs`, `place.rs` |
| 6e | The stages after node placement, first part: the graph they start from (the trees are back in it), `SwapStuff` and `Transpose` | `stages.go` (`Swap`, `TransposeAll`), `swap.go`, `transpose.go` without edge abductions, `direct.go` with the length check, `placementcost/graph.go` (`EdgeLength`, `GraphEdgeCrossings`) | Done: `src/swap.rs`, `place::Placed` |
| 6e | The stages after node placement, second part: `AlignAxes` | `stages.go` (`Align`), `alignment.go`, `metrics.go` | Done: `src/align.rs` |
| 6e | The stages after node placement, third part: `GapNormalization`, and `AlignAxes` again after it | `stages.go` (`NormalizeGaps`), `gapreduction.go` | Done: `src/gaps.rs` |
| 6e | The stages after node placement, fourth part: `OptimizeClusters`, and `AlignAxes` again after it | `cluster_optimization.go` (`OptimizeClusters`, `optimizeCluster` without `onlyFlip`) | Done: `src/cluster_stage.rs` |
| 6e | The stages after node placement, fifth part: `BalanceSymmetry`, `Equidistance`, and `AlignAxes` again after it | `symmetry.go`, `equidistance.go`, `placementcost/axis.go` (`AxisScore`), `engine/pipeline.go` (`equidistanceStage`) | Done: `src/balance.rs`, `src/equidistance.rs` |
| 6e | The stages after node placement, sixth part: `BinPack` before routing, `CleanupStuff`, `Rescale` | `packing/binpack.go`, `packing/guard.go`, `graphbounds/bounds.go`, `grouping/lifecycle.go` (`Cleanup`), `placement/stages.go` (`Pad`) | Done: `src/pack.rs`, `place::pad` |
| 6e | The stages after routing | `Dejitter`, `BinPack` again (with the routes), `Normalize` | Done for plain cards, trees and hierarchies: `src/dejitter.rs`, `pack::pack_routed`, `place::normalize`. See "After routing" below |
| 6f | TALA's three seeds and the choice between their results | `layout.go` | Done: `pipeline::layout_best`, `src/score.rs` (router-port.md, slice 13) |
| 7 | "Arrange" in the editor | ours | Done: `api::arrange` |

Nothing in the editor uses placement yet: it gets it with "Arrange" (slice 7).

To look at what the placement does today: `cargo run --release --example arrange_demo
examples/data/bookshop.json target/demo/index.html` writes a page with the editor's bookshop
example as placed by hand and as arranged by the ported stages. Serve `target/demo` with any
static web server to look at it.

## What TALA's pipeline does to plain cards

Measured on 200 random diagrams of 2 to 15 plain cards, by running TALA's stages one by one
(`docs/recorders/stages_test.go.txt`):

- Before placement TALA lays out part of the diagram as a **tree** in 22% of the diagrams, as a
  **hierarchy** in 6.5%, as a **cluster** in 6%, as a sequence in none. The other diagrams
  (62% of the recorded ones) go through node placement as plain cards.
- After node placement these stages move cards (diagrams out of 200): AlignAxes 78,
  GapNormalization 82, Equidistance 43, BinPack 21, SwapStuff 10, CleanupStuff 12 (clusters),
  Dejitter 6, BalanceSymmetry 5, Transpose 3, OptimizeClusters 1; Rescale and Normalize shift
  every diagram.

So slice 6 is six parts (6a to 6f above). With 6a to 6d the port gives TALA's places at the
end of the node placement stage, from the editor's own order of cards and connections, for
every diagram of plain cards. The first part of 6e adds the two stages that follow.

## The stages after node placement

`src/swap.rs`, on the diagram as node placement leaves it (`place::Placed`).

**The graph they start from.** The trees are back in it. TALA's lists are then in three
orders, and each of them decides somewhere:

- The cards: those node placement started from, then the tree cards, group by group, per card
  that trees hang on, per tree: the cards below the root level by level, then the root. Around
  a card that is alone: in the order the trees were laid out.
- The top-level list (`Graph::asking_order`: the cards a price asks whether they stand in the
  way, and the candidates for a swap): the tree cards in the order they were laid out, a card
  before the cards below it.
- Every card's own list of connections (`Graph::own_edges`): what it had, then its tree
  connections in the order the trees were laid out.

A connection to a cluster is still priced for the card of the cluster it belongs to. The cards
of a cluster may stand in the opposite order (`Vessel::backwards`) until the first transaction
that is kept.

**SwapStuff** (`swap`). Up to four rounds; a round without a swap ends them.

1. In a round every card with a connection, in the order of the cards, looks at every other
   card of the top-level list. Tree cards and cards in a hierarchy neither look nor are looked
   at.
2. Two swaps are tried with each: the two cards exchange their corners, or they exchange
   places so that the outer sides of the pair stay where they are (`smart_swap_positions`). A
   swap that brings two cards too close is not looked at.
3. The card takes the candidate and the kind of swap that leave the whole diagram shortest, if
   the swap also makes the card itself cheaper, the diagram shorter than it is, and adds no
   crossing.
4. The length of the diagram (`length`) is the price of every card (its connections, less a
   cell per connection where it lies symmetric) and the price of every crossing: two
   connections that share no card and whose straight lines, from middle to middle, cross.
   The prices of a bend and of a crossing are taken once, when a stage first asks for them,
   from the places then.
5. Last, the whole diagram is mirrored so that most arrows point right and down. Not if its
   first card is in a hierarchy, if two cards come too close, or if the diagram gets longer.

**A stage that runs four times needs its number.** On 20,000 more probe diagrams (seed 17)
the probe said the first `AlignAxes` differs in 2. It did not. In both, the first
`AlignAxes` moves no card and the gap stage moves none either, but the gap stage forgets
the price of a bend. The second `AlignAxes` takes the price anew, from the places then, and
now moves a card. The recorder wrote only the name of a stage that moved a card, and the
probe took that record for the first `AlignAxes`. The recorder now writes the stage's
number in TALA's plan too (`index`), and test and probe read by it. TALA's two prices after
every stage of both diagrams are the port's (`Placed::turn`, `Placed::crossing`), and a
transaction that is rolled back does not put them back: `Restore` in
`layoutgraph/placement_cost.go` runs only where a stage fails.

**Transpose** (`transpose_all`). Every card with one or two connections, in the order of the
cards, turns its branch around the card at the other end as in node placement
(`optimizer::transpose`), by the quarter turn that leaves the diagram shortest. Here the length
of the whole diagram counts, and the cards are not put on the grid. Tree cards are walked
through but neither counted nor turned: a card that trees hang on leaves them behind. Tree
cards, cards that trees hang on and cards in a hierarchy start no turn.

**AlignAxes** (`align` in `src/align.rs`). Rounds over all connections, until a round moves no
card (100 at most).

1. A connection is left alone if the middles of its two cards are in a row or a column
   already (within a pixel), if it is a tree connection, or if one of its cards is in a
   hierarchy. Tree connections are those inside a tree, and those of a tree's root where the
   card the tree hangs on has nothing but trees.
2. What is missing for a line is the distance between the two middles, rounded. Cards beside
   each other only slide down, cards above each other only across. Cards on a diagonal slide
   along an axis only if the room between them along the other axis is 60 or more.
3. First the card the connection starts at is tried, then the card it ends at, the other way.
   A card slides with every card connected to it, without the card at the other end and
   without tree cards. Down is tried, then across; of two slides that are as good the
   second is taken.
4. A slide is refused if the diagram gets larger than 30,000, if a card then lies on the line
   between the two middles, or if two cards come too close.
5. The first card wins if its slide makes the whole diagram shorter. The second wins if its
   slide beats that. The winner's cards move.

TALA runs the stage again after each of three later stages that changed something.

**GapNormalization** (`normalize_gaps` in `src/gaps.rs`). Across first, then down; each
forwards, then backwards.

1. In each pass every card is asked, those with the most connections first. Tree cards and
   cards in a hierarchy pull nothing.
2. The card looks for the nearest card it is connected to that lies wholly ahead of it. That
   card is pulled, with everything connected to it other than through the card itself. If one
   of those lies between the two, the gap is measured to that one.
3. The gap is measured from the card, then from every card that stands in the way between the
   two, in turn. A gap of half a cell or less is left. Otherwise the pulled cards move so that
   the gap becomes 150.
4. Before the pull is judged, the same is tried on the card's other side, so that it stays in
   the middle of its neighbours. That second pull is kept if it moved a card and leaves the
   diagram shorter than the first pull alone and than the start.
5. A pull that does not make the diagram shorter ends the search for this card. A pull that
   brings two cards too close is taken back, and the next card in the way is tried.
6. The length is that of the swap stage without the penalty for arrows that point the wrong
   way.

At its end the stage forgets the price of a bend: the next stage takes it anew, the price of a
crossing stays. The stage says it changed something whenever it asked a card, so the stage
that lines cards up runs again after it.

**OptimizeClusters** (`optimize_clusters` in `src/cluster_stage.rs`). Every cluster in turn.

1. The cluster turns where its neighbours ask for it, as after node placement
   (`turn_cluster`).
2. A cluster that then stands as its neighbours ask is put in line with them. Two ways are
   tried from the same places. The vessel moves along its row or column to the middle of its
   neighbours. Or every neighbour that stands across from the vessel moves to the vessel's
   middle line. The way that leaves the diagram shortest is kept, if one makes it shorter.
3. After that the neighbours across from the cluster are pulled closer, as the gap stage does
   it for a card (`reduce_gap`).

The stage that lines cards up runs a third time where a cluster changed.

**BalanceSymmetry** (`balance_symmetry` in `src/balance.rs`). Every plain card in turn (no card
of a hierarchy, no card trees hang on, no vessel). The card moves where all of this holds:

1. Each two of its connections that follow each other lead to cards on the same side of it
   (above, below, left or right; a card at a corner is on two sides).
2. It has two or more neighbours, and they stand in one line (`AxisScore` is 1): measured
   against the largest of them, at a quarter, a half and three quarters of its height or
   width. A small neighbour needs fewer of the three lines.
3. No neighbour has less than half the area of the largest.

Then the card moves along that line to the middle of the box around its neighbours, rounded
down, unless that brings two cards too close.

**Equidistance** (`equidistance` in `src/equidistance.rs`). Every card in turn, across and then
down (no card of a hierarchy, no tree card, no card trees hang on; a vessel does move).

1. The nearest connected card before the card and the nearest after it are found. Without
   both the card stays.
2. The card's place is halfway between the two, rounded.
3. Two ways are tried from the same places: the card moves alone, or with the cards that
   hang on it from the side (its other neighbours straight above or below it, and all that
   is connected to them without passing the card or the two nearest). A way counts if it
   brings no two cards too close and leaves the diagram no longer.
4. The shorter way is kept; where both are as short, the one with the hanging cards.

Where a way was kept along an axis, all cards are tried again along it, up to five times. The
stage that lines cards up runs a fourth time where a way was kept.

**BinPack** (`pack` in `src/pack.rs`), before routing. It packs the groups of connected cards.

1. The groups are taken in the order of the diagram. A group with a hierarchy stays where it
   is. Without one, the first group stays.
2. Every other group is moved as one piece. The corners it is tried at: where the first
   group began; beside every card packed so far (to its right, below it, right of its lower
   end, each only where no connection of that card leads that way); six corners at the box
   around each packed group; above, left of and above left of all that is packed; and where
   each of its own cards stood before.
3. A corner is out where the group comes too close to a card, where one of its cards lies in
   the box of a hierarchy, or where a card lies on a route a connection is likely to take:
   one straight line between cards that face each other, if it is longer than the smallest
   group still to pack; both L shapes and the middle lines of both S shapes between cards on
   a diagonal and at a cluster.
4. Of the corners left the group takes the one with the lowest score: the area of the box
   around all that is packed, and half the square of what the box lacks to be a square.
5. Where a group finds no corner, or the box around the whole diagram scores worse than
   before, every card goes back to where it was.

**CleanupStuff** and **Rescale** (`pad` and `cleanup` in `src/place.rs`). The cards of a
cluster are cards of the diagram again, where they stand in their vessel. Then every card
moves 1000 across and down. These are the places routing starts from. `cleanup` makes the
graph routing starts from, as TALA's `grouping.Cleanup` leaves it: the vessels leave, the
cards of every cluster come at the end, cluster after cluster in the order of the vessels'
numbers, each in its cluster's order from the vessel's corner on; connections that ended at
a vessel end at their card again, and the graph keeps the clusters for routing
([router-port.md](router-port.md), "Clusters").

### After routing

TALA still moves cards after it has routed the connections: `Dejitter`, a second `BinPack`
that keeps the routes, and `Normalize`, which moves the whole diagram to the origin. All
three are ported for plain cards, trees, hierarchies and clusters (`Dejitter` moves no card
of a tree or a cluster; the second `BinPack` leaves a hierarchy where it is; `Normalize`
needs nothing more). After routing the cards of clusters are cards of the graph, and TALA's
transaction asks no pair from one of them: two cards of clusters may come as close as they
like when the second `BinPack` moves a group. They need TALA's routes, and so does the choice between
the three seeds (slice 6f): it scores the routed diagram with its labels
(`internal/quality`; ported as `src/score.rs`, see router-port.md, slice 13).

**The whole pipeline** (`layout` in `src/pipeline.rs`). All stages in TALA's order, for
one seed, from the diagram as it is given (sizes of the cards, connections, sizes of the
labels) to the finished drawing, for every kind of diagram the engine has.
Checked: `tests/routed_reference.rs` runs it for the 174 recorded diagrams of plain cards,
and `probe_routed whole` for 45,419 probe diagrams: the cards in TALA's order at TALA's
final places, the routes and the label places are TALA's in all. 9 deliberate breaks of
the chain, all caught. For diagrams with trees (and neither hierarchy nor cluster):
`tests/routed_trees_reference.rs` runs it for the 73 recorded ones, and `probe_routed whole`
for 26,647 probe diagrams, with TALA's drawing in all. There the port's own placement says
what a tree is, on which side it lies and where its labels are to go; see
[router-port.md](router-port.md), "Trees". For diagrams with a hierarchy (and neither tree
nor cluster): `tests/routed_hierarchies_reference.rs` runs it for the 50 recorded ones, and
`probe_routed whole` for 15,441 probe diagrams, with TALA's drawing in all. The placement
hands routing the level of every card (`Graph::levels`); see "Hierarchies" there. For
diagrams with a cluster: `tests/routed_clusters_reference.rs` runs it for the 113 recorded
ones, `tests/routed_several_reference.rs` for the 28 with several kinds at once, with
TALA's drawing in all. On the cluster-heavy probe sets none of 43,613 differs either (below, "Clusters in
cluster-heavy sets").

**The second BinPack** (`pack_routed` in `src/pack.rs`), after `ReorderDuplicates` and
before `PlaceLabels`. The same packing as before routing, with three differences
(`edgesPlaced` in TALA's `packGuarded`):

1. A group's routes move with its cards. Where the cards go back at the end (a group found
   no corner, or the box around the diagram got worse), the routes go back too.
2. Beside a packed card all three corners are tried. Before routing a corner is left out
   where a connection of the card may lead.
3. A group may not cover a route of a group packed before it: its routes as they lie, the
   upright and level pieces. Before routing these are the routes it is likely to get. For
   the group that is being packed the likely routes are asked still: it has not moved yet.

Checked in the pipeline: 2 recorded and 195 probe diagrams of plain cards in which it moves
a card, all with TALA's cards, routes and labels to the end. 16 deliberate breaks (12 of
the stage, 4 of its place in the chain): 14 make the pipeline replay fail, 6 of them only
after the fixture got two diagrams from the probe. Kept though no diagram can tell: that
the groups are packed after doubled connections swap for a label, not before. Kept, cannot
change an answer: that the routes of the first group move with it (it comes back to where
it was).

**Dejitter** (`dejitter` in `src/dejitter.rs`), after the first routing. Every card in turn,
and every connection of it whose route has three pieces or more:

1. A jitter is a route whose first two bends, seen from the card, are 80 apart or less, and
   that goes on beyond them on the side away from the card. The card would move along the
   piece between the two bends, by its length (rounded): then the piece at the card and
   the piece after the bends are one line.
2. The card stays if another connection of it is one straight line the same way (its other
   card would have to move too), or if a piece of one of its routes would turn round or get
   shorter than 5: the piece after the first bend where the first piece moves as a whole,
   the first piece where only its end at the card moves.
3. It stays if a new first piece would run through a card other than its own two.
4. It stays if routes of other cards then run through it more often, if it is then less
   symmetric to its neighbours than before (`NodeSymmetry`, taken once per card), or if the
   move brings two cards too close (what a transaction refuses).
5. Otherwise it moves. The ends of all its routes move with it, the first piece as a whole
   where it runs across the move, and the two bends of the jitter go.

Where a card moved, the pipeline routes all connections again from nothing (the second
`EdgeRouting`); `coordinator::route_stages` does both. In the pipeline the stage moves a
card in one diagram of plain cards of 1,800, so it has its own recorded layouts: cards in
rows and columns, most a little out of line, routed by TALA's `RouteGraph`.

Checked: `tests/dejitter_reference.rs` replays 409 such layouts (in 205 a card moves; 221
cards in all): the cards and the routes after the stage, and what it says.
`probe_dejitter compare` does the same for 20,000 more (10,000 moved): no difference. In
the pipeline: 2 recorded and 25 probe diagrams of plain cards in which it moves a card, all
with TALA's final drawing. 50
deliberate breaks of the stage: 36 make the replay fail (15 of them only after the fixture
got nine layouts from the probe), and both breaks of the chain (no jitter taken out; not
routed again) make the pipeline replay fail.

- **Kept though no layout can tell** (13 of 50, tried on the 20,000): that a route which
  turns back at its bends is left alone (three breaks: the router lays no such route here);
  the rounding of the move; that a slanted line is not asked whether it turns and moves
  only at its end (two: the first routing lays none); that a new piece is not asked for the
  card itself, and that the pieces of the card's other routes are asked too (two); the
  symmetry (two: that it may not get worse, and that it is taken once per card); that
  "too close" is judged from the places after the last move; and how routes through the
  card are counted (two: per piece, and without the card's own).
- **Kept, cannot change an answer here**: skipping two bends at one place (the router lays
  no route with a doubled point; without the check such a route would lose both).

**Normalize** (`normalize` in `src/place.rs`). The leftmost and the topmost point of the
drawing go to 0: a corner of a card, a point of a route or a corner of a label, the last two
rounded down. Cards and routes move by that much; a label keeps its place along its route.
With it the port gives TALA's final drawing for every diagram of plain cards: the places of
the cards, the routes and the label places of all 174 recorded and 44,119 probe diagrams
(`tests/routed_reference.rs`,
`probe_routed compare`). 10 deliberate breaks: 7 make the replay fail. Kept though no
diagram can tell (3): that a point of a route and the corner of a label are rounded down
(two breaks, across and down) and the corner of a card is not. The cards and nearly all
route points of plain cards lie on whole pixels.

The owner's call (2026-11-23): the port goes on until it gives TALA's final places and TALA's
pick of the three seeds. "Arrange" comes after that.

Measured on the 408 recorded diagrams, with the recorder's `STAGES_ROUTED` (places and routes
after every stage from the first routing on):

- Cards move after routing in 20 diagrams (`Dejitter` 6, the second `BinPack` 15). 16 have
  trees, a hierarchy or a cluster; 4 are plain cards, found for the two stages. Plain cards are
  not safe from it: of 44,119 plain probe diagrams 219 have a card moved (`Dejitter` 25,
  the second `BinPack` 195). In the other 388 the final places are the places after
  `Rescale`, moved as a whole.
- A route changes in: the first `EdgeRouting` 379 (all diagrams with a connection),
  `Crosshatch` 11, `Dejitter` 6, `EdgeRouting` again 3, `SimplifyEdgeRoutes` 1,
  `SwapEdgePorts` 31, `StraightEdgesFallback` 5, `BalanceEdgeSegments` 225, `FixClusterEdgeBranching` 2,
  `ReorderDuplicates` 1, the second `BinPack` 5, `NudgeEdgeChannels` 5.
- The same for plain cards alone, on the 8,921 plain probe diagrams: `Dejitter` 5,
  `EdgeRouting` again 1, `SwapEdgePorts` 125, `StraightEdgesFallback` 57,
  `BalanceEdgeSegments` 3,593, `ReorderDuplicates` 66, the second `BinPack` 26,
  `NudgeEdgeChannels` 16. `Crosshatch`, `SimplifyEdgeRoutes`, `TraceEdgesToShapeBorder` and
  `ShortcutEdgeRoutes` change no route of plain cards there. `SimplifyEdgeRoutes` does in one
  of 35,198 more plain probe diagrams, so it is ported; the other three are not. In the 8,862 other probe
  diagrams (trees, hierarchies, clusters) `Crosshatch` changes a route in 34,
  `FixClusterEdgeBranching` in 9, `ShortcutEdgeRoutes` in 1, `NudgeEdgeChannels` in 95.
- The router as ported (`RouteEdges`, for plain cards), given TALA's places before
  `Normalize`, against TALA's routes there (`cargo run --release --example probe_routed`):

  | Diagrams | All routes as TALA's | Connections the same |
  |---|---|---|
  | 169 plain | 112 | 1,107 of 1,268 |
  | 38 with a hierarchy | 18 | 305 of 409 |
  | 66 with trees | 5 | 236 of 424 |
  | 83 with a cluster | 2 | 286 of 616 |
  | 23 with several of these | 0 | 261 of 440 |

- The first `EdgeRouting` is ported for plain cards (`route_graph`, see
  [router-port.md](router-port.md), slice 7): all 174 plain diagrams have TALA's routes after
  it. With the later steps that were ported for `RouteEdges` chained to it
  (`coordinator::route_stages`), with `SimplifyEdgeRoutes`, `SwapEdgePorts` and
  `NudgeEdgeChannels` ported, `Dejitter` after the first routing and the second `BinPack`
  before the labels, all 174 have TALA's routes and label places before `Normalize`, and so
  have the 44,119 plain probe diagrams. `WHICH=1` lists the diagrams that differ and the stages in
  which TALA changed a route there.

Order of work: the routing stages for plain cards that are missing (the gap is smallest
there), then TALA's routes for trees, hierarchies and clusters, then `Dejitter`, the second
`BinPack` and `Normalize`, then the score and the three seeds.

## Finding trees

`find_trees` in `src/trees.rs`. A tree is a branch of cards that hangs on the rest of the
diagram by one connection.

1. Round after round, every card with one connection is taken off. It becomes the root of a
   tree that hangs on the card at the other end; the trees that hung on it become its children.
2. A card stays in the graph ("terminal") where its branches' arrows do not all point the same
   way (some towards it, some away), and so does the card it hangs on where the arrows do not
   go on in one direction through it. Of two cards left with one connection between them, one
   stays.
3. A card that has nothing left but trees is no core for them. Where one of its trees is a
   chain whose arrows point the other way than all the others', the chain is turned round: its
   last card becomes the card everything hangs on.
4. A branch that never branches (a chain) is put back, unless it hangs on a card that has
   nothing but trees, and more than one. Cards and connections that are put back go to the
   **end** of the lists. That is why the stage changes the order also where it leaves no tree:
   192 of 432 recorded diagrams.

`without_trees` gives the graph node placement works on.

## Laying trees out

`place_trees` in `src/tree_layout.rs`, called by `place_with_trees` for every group once its
cards are placed.

1. What is laid out as one tree: every root on its own where the card it hangs on has other
   connections. Around a card with nothing but trees the roots go together whose arrows point
   the same way. The largest tree is laid out first.
2. The tree is drawn level by level under the card it hangs on: a level is as high as its
   highest card, 100 below the level above, or more where a label has to fit beside the line;
   cards on a level are 50 apart; a card stands over the middle of its children.
3. It is drawn on each of the four sides (down, up, left, right), each time starting beyond the
   outermost card on that side, and tried next to every card between there and the card it
   hangs on, then at that card. A place that brings it closer to another card than the gap
   they must keep is refused. The side where it comes nearest wins; of equal ones, the side
   that leaves the diagram nearest to a square.
4. A card's second tree goes opposite its first, without a search. Around a card with nothing
   but trees no side is taken twice while one is free.
5. Every card of the tree knows the side its tree lies on (`Graph::tree_links`, TALA
   `Tree.Orientation`). Routing reads it. Where a group or the whole diagram is mirrored
   afterwards, a tree along the mirrored axis lies on the other side.
6. The stage says where the label of a tree's connection is to go (`position_labels`, in
   `Graph::label_presets`): a place along the route, as a share of its length, and on the
   line or on one side of it. Seen with the tree growing down: an only child has its label
   on the line, halfway. One of several children has it beside the piece that runs down to
   it, halfway along that piece, on the side away from the other children. A root's
   connection gets a place only where the card the tree hangs on has nothing but trees:
   only then it is routed by rule. A label without size across the line gets none. The
   places are not turned when the diagram is mirrored later (TALA does not either).

Two things reach beyond the trees. The optimizer never turns a card that trees hang on
(`Graph::trees_at`). And when a group is mirrored to its flow direction, the arrows between
two cards of a tree do not count, only those at a card of the group itself, unless the tree
hangs on the diagram's only card.

## Finding hierarchies

`find_hierarchies` in `src/hierarchy.rs`, on what the tree stage leaves. A hierarchy is a group
of connected cards whose arrows flow one way through three or more levels; TALA lays it out in
layers instead of placing its cards one by one.

1. A group is asked only if no tree hangs on one of its cards, and if some card has no arrow
   pointing at it and some card points at nothing.
2. Rings are broken (`cycle_edges`): cards that nothing points at and cards that point at
   nothing are taken off until none is left; then the card with the most arrows out over
   arrows in gives up the arrows that point at it, and those are turned round. Of several
   connections between two cards the same way one is kept, with their number as its weight.
3. The cards are ranked (`rank`): every arrow points down at least one level, and the arrows
   are as short as they can be together. That is network simplex: a first ranking by the
   longest path, a tree of arrows that span exactly one level, and arrows of the tree with a
   negative cut value swapped for others until none is left. Where two arrows are equally
   good the one with the lower number is taken: the answer depends on it.
4. The group is a hierarchy if it has three levels or more; if cards by levels squared is
   between a half and two (a taller one passes where it is one workflow from one card to one
   card with no arrow up); if at least three arrows in five point down; if no card has more
   than twice the root of the number of cards as connections; and if it is not one card, a row
   and one card (`is_one_many_one`).

## Laying a hierarchy out

`place_hierarchies` in `src/hierarchy_layout.rs`. Every level is a row, the rows are under
each other.

1. The cards start in a shuffled order (TALA's random numbers, which go on from one hierarchy
   of a diagram to the next). An arrow over more than one level gets a dummy on every level
   between, so that arrows only join neighbouring levels.
2. The order on a level, twice. First every level is sorted by where its cards' neighbours
   stand on the level above, and every card is exchanged with one of its next three
   neighbours where that gives fewer crossings or, of as many, shorter arrows; as many rounds
   as the widest level has cards. Then every card, those with the most connections first, is
   moved along its whole level and left at the best place ("sifting"), up to ten rounds.
3. The rows: a level is as high as its highest card, with the cards at its middle. The next
   level starts 40 below plus 50 for every crossing on the way down, or the height of a label
   and 50; 50 at least, 300 at most.
4. The places across, after Brandes and Köpf: a card goes straight under the middle one of
   the cards above it where no dummy's arrow crosses, which makes blocks; blocks are pushed
   together 60 apart (50 beside a dummy, 60 more at the ends of a level). That is done four
   ways (down or up, from the left or the right); the four are moved to the ends of the
   narrowest, and a card gets the mean of its two middle places.

Node placement leaves a hierarchy as it is: the group is not placed and not mirrored, only
packed with the others.

## Finding clusters

`find_clusters` in `src/clusters.rs`, on what the tree stage leaves. A cluster is a set of
cards that are alike. TALA puts them in a row or a column and places that as one card, the
vessel.

1. The cards are asked in their order. A card in a hierarchy, a card that trees hang on and a
   card connected to a cluster made before start no cluster and join none.
2. Two cards are alike if neither is more than four times as wide or as high as the other, if
   their connections lead to the same cards (one at least), and if as many arrows leave each
   and as many reach each.
3. Wide cards go under each other (a column), high cards side by side (a row). The mean width
   and height decide, rounded. For square cards a random number decides.
4. All cards of a cluster get its largest width and height. Between two cards is a tenth of a
   card, 20 at least.
5. The vessel is a new card at the end of the list. Its number is a random number of 63 bits.
   The connections of the cluster's cards now end at the vessel. The graph keeps for each of
   them the card it belongs to (`Graph::behind`).

A card with one connection is a tree, so it is never in a cluster: the cards of a cluster have
two connections or more.

## Node placement with a cluster

The vessel is placed like a card of its size (`Graph::vessels`). The differences:

1. The price of a connection between a card and a cluster (`edge_length.rs`), once the cards
   have their real sizes. The line runs from the card to the cluster's card the connection
   belongs to, between the two sides that face each other, from middle to middle; it is never
   taken for diagonal. The distance is that to the vessel. A card in the way costs two bends.
   So does a card beside a row, or above or below a column. Where a cluster has exactly two
   neighbours, both on the same side cost one bend. A card across from the vessel pays how far
   it is off the vessel's middle line; a card that is not across pays one bend.
2. A vessel wants its arrows down (a row) or to the right (a column), with less weight than a
   card (0.2 against 0.3). That also holds while the places are cells.
3. A card is drawn to the cards of the cluster it is connected to, not to the vessel. A vessel
   is drawn so that the middle of its cards comes to lie on the point (`median_point`), and it
   is tried only at the places themselves, not a little to the left of and above them.
4. A cluster turns (`turn_cluster`). With more neighbours beside it than above and below it
   becomes a column, with more above and below a row. It turns around its middle, or around
   its top-left corner where that brings it too close to a card, or not at all. TALA asks this
   after a group is placed and whenever `transpose` turns a vessel around a card. Neither a
   vessel nor a card connected to one turns its own branch.
5. The turn is checked against the whole diagram. The hierarchies and the groups placed
   before, each still around its own origin, must not come too close either
   (`Graph::outside`).
6. When a group is mirrored along a cluster, the cards of the cluster stand in the opposite
   order. A cluster that turns later, in another group, puts the cards of all clusters in
   order again. A later stage does that for the rest (slice 6e).

## The node placement stage

`place` in `src/place.rs`, for cards in no container (`place_with_trees` where the earlier
stages found a tree, a hierarchy or a cluster):

1. Before it (`prescale`): a card with many connections grows, so that they find room on its
   sides 40 apart. With n connections on its fullest side it becomes at least (n + 1) * 40 wide
   and high: a card of 220 by 120 with three connections to one card, or nine in all, becomes
   160 high.
2. The cards are split into groups that connections hold together (`split`).
3. Every group is placed on its own (slice 5), with the random numbers started anew.
4. The group is mirrored so that most arrows point right or down (`direct`).
5. The groups are packed, largest first, each at a corner of one placed before it where the box
   around all stays smallest and nearest to a square (`combine`).

## Node placement for one group

`place_nodes_orthogonally` in `src/node_placement.rs`, after "Graph Compact Orthogonal Layout
Algorithm" by Freivalds and Glagolevs. For n connected cards, with a seed:

1. **First arrangement** (`sizeless.rs`), on a coarse grid with one card per cell. The first card
   goes to cell (n, n); the others follow in the order connections lead to them, each to the free
   cell near the median of its placed neighbours where its connections are shortest. With an even
   seed and 4 to 64 cards the start is another one: cards as far apart as the number of
   connections between them, relaxed in 48 sweeps.
2. **Sizeless loop**, 45·√n rounds: every card moves to the free cell near its median point
   where its connections are shortest, or swaps with a neighbour cell. The temperature (how far
   the median point is thrown off) starts at 2·√n cells and cools by a constant factor per round.
   Every ninth round compaction runs, along x and y in turn.
3. **Change to real sizes**: compaction along both axes makes room for the cards' sizes (cells
   become pixels), cards go onto the grid, and the price of a bend is taken, halved.
4. **Sized loop**, another 45·√n rounds of the pass of slice 3, cooling further. Every ninth
   round compaction runs, ever tighter, and groups of cards that drifted more than three cells
   apart are pulled together a cell at a time (`join.rs`).
5. Up to ten passes without temperature, until one changes nothing.

**Compaction** (`compaction.rs`) along one axis: lines of sight are taken once (which card lies
behind which, with nothing between); every card moves forward to a least distance from the card
it sees behind it; then cards that see nothing behind them move back together with the cards
that see them, and single cards move back, as far as that makes connections shorter.

## One pass of the sized optimizer

`optimize` in `src/optimizer.rs`. The cards are taken in a random order. A card without
connections, or more than 100 cells wide or high, is left alone. For each of the others:

1. **Move.** The point it is drawn to is the median of the middles of the cards it is connected
   to, one per connection (`median_point`). With a temperature the point is off by up to that
   many card sizes, at random. Then the move of slice 1 around that point, with the places in a
   random order. Without temperature (TALA's last ten passes) the card moves only to a cheaper
   place.
2. **Swap**, if it did not move (`best_swap_candidate`): with a card at most one cell away, when
   that makes the card cheaper and the two together cheaper and brings neither too close to a
   third.
3. **Turn**, if there was no swap (`transpose`): a card with one connection turns around the card
   at its other end; a card with two turns, with the smaller of the two branches it joins, around
   the first card of the larger one. One, two or three quarter turns; the one that makes the
   connections of the card and of its neighbours shortest is taken, if it is shorter than now
   and brings no two cards too close that were not before.
4. **Hub**, if nothing turned and there is a temperature: a card with a spoke (a connected card
   that has no other connection) and another connection moves once more as if it had no spokes.
   They follow in the next pass.

The pass says whether anything moved, swapped or turned; the hub's second move does not count.

## The price of a card at a place

`moveNodeToBestGuarded` prices a place as `NodeEdgeLength - NodeSymmetry * cell * connections`.
Lower is better. Among places of the same price (closer than 0.0001) the first one wins, the
card's own place before any other.

- **Edge length** (`placementcost::edge_length`), summed over the card's connections:
  - the gap between the two cards; for cards on a diagonal the two legs of an L plus one bend;
  - one or two bends more when another card stands in the way: for cards that face each other
    the straight line between them, and when they are more than 40 out of line also the two
    routes around; for diagonal cards both L routes;
  - a penalty for an arrow that does not point down and to the right: 0.3 of a quarter cell per
    eighth of a turn. A card that shares more than two labelled connections with another wants
    it beside itself instead, with 10 in place of 0.3, and pays a bend per label when they are
    above each other;
  - "flow": a card with a connection coming in and one going out pays up to two bends when the
    two do not continue each other, and up to one when connections of the same kind leave less
    than 60 degrees apart (cards with up to 8 connections).
- **Symmetry** (`placementcost::symmetry`): pairs of connected cards that mirror each other across
  the card, within one cell, with no card between. 2 for a pair in line with the card, 0.5
  otherwise, divided by the number of connected cards (those within 1200). A card without a
  mirror pair of its own gets the symmetry of the neighbours it helps to make symmetric.

The price of a bend is TALA's turn cost, an eighth of the number of connections times the longest
one, taken before the move.

## What the port taught us

- **Read, then measure.** `restoreEdgeAbductions` reads as if, after node placement, the
  records of a cluster pointed at its vessel, so that the later stages would price a
  connection to a cluster like one to a card. 16 recorded diagrams said no. With the card
  behind the connection kept, none differed.
- **The orders part again when the trees come back.** TALA adds the tree cards to its list of
  cards in one order, to its top-level list in another, and a card's own list of connections
  is in a third. The recorder now records all three after node placement. In 52 of 432
  recorded diagrams the top-level list is in another order than the cards.
- **Walked through is not counted.** TALA's `reachableNodes` walks over tree cards but leaves
  them out of its answer. So a branch that turns leaves its trees behind. One probe diagram in
  3,000 showed it; the fixture had none and got that one.
- **Counted by name, counted wrong.** The first count of diagrams in which the second run of
  the stage that lines cards up moves a card was 1,748 of 9,000. It counted every diagram
  whose last recorded stage had that name, also where it was the first run. By its place in
  the pipeline it is 535. The comparison itself was not touched by this.
- **Left out of the stage that puts clusters right**, because it cannot change an answer (each
  has a comment): putting the cards of every cluster in order at each of its three
  transactions (they are in order since the gap stage); putting the clusters back after a try
  (a try moves cards only); and asking once more after the pull that no two cards came too
  close (the pull has asked).
- **Kept though no test or probe diagram can tell** (4 of 26 deliberate breaks of the stage
  that puts clusters right, tried on the 89 probe diagrams in which it moves a card): that a
  cluster which did not turn as its neighbours ask is not put in line; that a card with two
  connections to the cluster counts once for the middle; that the pull also tries the other
  side; and that the cell size is taken anew at the start.
- **The cards of a cluster are cards again when the groups are packed.** TALA walks from a
  vessel to the cards of its cluster, and they are in no container. So each of them gives
  corners of its own, and where it stood before is a corner too. The port keeps them in the
  group (`Item::Member`).
- **Left out of the packing stage**, because it cannot change an answer (it has a comment):
  TALA's note that a group found no corner. The groups not packed are still far away then,
  so the box around the diagram is worse than before and all cards go back for that. Also
  left out: everything for containers, cards with a fixed place and routes that exist (the
  second run of the stage).
- **Cannot change an answer** (3 of 110 deliberate breaks of the packing stage): the fourth
  side of a card when a line is tested against it (a straight line that meets one side meets
  another or ends in the card), and trying a corner only once (two breaks: the second try
  scores the same, and the first of the best wins).
- **Kept though no test or probe diagram can tell** (20 of 110, 17 of them also tried on 557
  more probe diagrams with several groups): that a straight route only counts when it is
  longer than the smallest group still to pack (six breaks: beside a shorter one no group
  finds room anyway); the order of a cluster's cards in its group (two breaks); the middle
  line of the S that goes down; that a line ending in a card's gap counts; that a card with
  a connection down and to the right gives no corner at its lower right; that one card of a
  hierarchy is enough for the group to stay; that the groups come in the order of the
  top-level list, not of the cards; that the first group is not packed like the others;
  that a corner in a hierarchy's box is no candidate (the group is refused there later);
  the corner above left of all, and that above is tried before left; that packing ends at
  the first group without a corner; how far the groups are moved away; and the rounding of
  a box's far corner (the cards have whole sizes).
- **Kept is not moved.** `Equidistance` says it moved a card where a way was kept, also where
  the card was halfway already and the way moves it by nothing. So nearly every diagram with
  a card between two neighbours gets all five rounds more, and the stage that lines cards up
  a fourth time.
- **A map has no order.** `BalanceSymmetry` keeps a card's neighbours in a Go map, which
  gives them in another order at every run. `AxisScore` measures against the first of the
  largest, and the first two say which way the card moves. For three or more neighbours of
  the same height that are a little out of line, TALA's answer could change from run to
  run. The port takes them in the order of the card's connections. No recorded or probe
  diagram differs.
- **Left out of the balance and equidistance stages**, because it cannot change an answer
  (each has a comment): in `BalanceSymmetry`, passing over a card with fewer than two
  connections (it has fewer than two neighbours, and is passed over for that); in
  `Equidistance`, passing over the two nearest cards when the hanging cards are collected
  (they stand before and after the card, so not beside it); and everything for containers
  and cards with a fixed place.
- **Cannot change an answer** (6 of 96 deliberate breaks of the two stages): the floor of a
  third under `AxisScore` (the stage only asks whether it is 1), its answer for fewer than
  two cards (the stage asks for two or more), and from which of two cards the side is seen;
  in `Equidistance`, trying the way with the hanging cards where none hang (it is the way
  alone then), ending the rounds at the first that keeps no way (the rounds after it would
  keep none either), and walking through tree cards (a tree hangs on the rest by one card,
  so nothing new is reached through it).
- **TALA has no one answer** (2 of 96): which of several largest neighbours the others are
  measured against, and which two neighbours say the axis. TALA takes them from a map.
- **Kept though no test or probe diagram can tell** (16 of 96, the first eleven also tried on
  211 probe diagrams with a balanced card or with three or more neighbours on one side of a
  card): in `AxisScore`, where the three lines run (two breaks), that a line along a card's
  border meets the card, how much a small and a middle card count (three breaks), that
  cards beside each other a little out of line are not in line, and that cards above each
  other can be in line; that a tree card can be balanced; that each two connections that
  follow each other are compared, not each with the first; that a neighbour of exactly half
  the largest area is alike; the rounding down of the way (four breaks: the cards of the
  probe have sizes in steps of 20, so every way is a whole number); and in `Equidistance`
  that the way with the hanging cards wins where both are as short.
- **The gap stage always says it changed something.** TALA counts every transaction of the
  stage that was not refused, also one that moved no card. So the stage that lines cards up
  runs a second time for nearly every diagram, and moves a card in one of seventeen.
- **Two prices, two lifetimes.** The gap stage forgets the price of a bend at its end
  (`ResetTurnCost`), not the price of a crossing. The port keeps the two apart
  (`Placed::prices`).
- **Left out of the gap stage**, because it cannot change an answer (each has a comment): that
  the pull on the other side must leave the diagram shorter than the first pull and bring no
  two cards too close (a pull that moved something has passed both); and everything for
  cards inside a container.
- **Kept though no test or probe diagram can tell** (6 of 50 deliberate breaks of the gap
  stage, tried on 1,786 probe diagrams): that the cards standing in the way between the two
  are tried as the place to pull to (two breaks), that a pull to such a card which brings two
  cards too close is followed by the next one, and that the search ends at the first pull
  that is kept; that a pull on the other side that is not kept is taken back; and that the
  price of a crossing is kept from the first stage that asked for it.
- **A rule that only spares work.** The stage that lines cards up does not try to slide cards
  that are in line already, cards beside each other across, or cards on a diagonal that are
  close along the other axis. Such a slide ends on the other card or closer to it than
  connected cards may be, and the transaction refuses it. So nine deliberate breaks of those
  rules change no answer. The port keeps the rules: they save the tries.
- **Kept though no test or probe diagram can tell** (5 of 40 deliberate breaks of the stage
  that lines cards up): that middles within one pixel are in line, not within five (the cards
  of the probe have sizes in steps of 20, so no two middles are one to five pixels apart); the
  rounding of the distance down; the limit of 30,000 for the size of the diagram; and, for
  cards on a diagonal that can slide both ways, that down is tried before across and that the
  second of two equally good slides is taken.
- **Left out of the two stages**, because it cannot change an answer (each has a comment): the
  early answer for parallel lines (the division by 0 gives no number from 0 to 1 either);
  taking the cell size anew (no card has changed its size).
- **Cannot change an answer** (3 of 67 deliberate breaks of the two stages): asking the swap
  candidates in the order of the cards instead of the top-level list (they differ only in
  tree cards, which are no candidates); going on after a round without a swap (the next round
  meets the same diagram); letting a tree card start a turn (tree cards are not turned).
- **Kept though no test or probe diagram can tell** (7 of 67, tried on 1,786 probe diagrams):
  the smart swap of two cards on a diagonal down and to the right; pricing a tried swap with
  the cards of every cluster in order; the swap of a card whose own price is exactly 0; that a
  later swap is checked against the places the swap before left; that the mirror of the whole
  diagram is taken back where it makes the diagram longer, and that the clusters are then as
  before; and that a vessel turned in the transpose stage may turn from a row into a column.

- **A transaction can be on the whole diagram.** A cluster keeps the graph it was made in.
  `optimizeCluster` starts its transaction there, not on the group that is placed. So a turn is
  refused for a card of another group that happens to stand in the way around its own origin,
  and a turn that is kept arranges the cards of every cluster. 3 of 2,122 probe diagrams with
  clusters showed it, each of about 40 cards. A line of log output added to TALA's
  `arrangeNodesWithWork` for one run named the caller.
- **Mirrored one by one.** `mirrorAxes` mirrors the vessel and each of its cards. The cards of
  a row then stand in the opposite order, and nothing puts them back before `AlignAxes`.
- **The cards of a cluster follow the vessel.** Every move of a vessel in node placement takes
  its cards along. So the port keeps no places for them: they follow from the vessel's place
  (`Vessel::member`).
- **The card behind a connection.** TALA goes through its list of "abductions" for the first
  unused record with the same two ends. The port keeps the card per connection. Both give the
  same card, because a card's connections and the records are in the order of the graph.
- **A recorder for every level.** The stage recorder shows that the final places differ, not
  where. The placement recorder got an option that runs the cluster stage first.
  `probe_places trace` then named the first pass of the sized optimizer, and the count of
  random numbers drawn (48 against 58) pointed at the number of places tried.
- **A leaf is a tree.** A card with one connection is taken out by the tree stage. The first
  hand-written diagrams for clusters had none.
- **The number of a vessel has 63 bits.** The numbers of cards were `usize`, which has 32 bits
  in WebAssembly. They are `i64` now (`Graph::ids`).
- **Left out of the cluster code**, because it cannot change an answer (each has a comment):
  rounding the mean size up again; the new padding when a cluster turns (it is the old one: a
  row is made of cards higher than wide, so a tenth of the other side is never less); taking
  diagonals together for a card that wants its arrows up (no card and no vessel does);
  passing over a card that is in a cluster already (its connections end at the vessel, so it
  is like no card); an own price for a card in the way of a cluster connection (two bends, as
  between cards in line); refusing to turn the branch of a vessel or of a card at one (a
  vessel has four connections or more, and a card's two connections to a cluster are a ring).
- **Cannot change an answer** (1 of 136 deliberate breaks of the cluster code): rays for a
  vessel in the "flow" price. They would point at its own cards, the same wherever it stands.
- **Kept though no test or probe diagram can tell** (7 of 136, tried on all 2,122 probe
  diagrams with a cluster): the next free number where a vessel's random number is taken
  (once in 2^59 diagrams); that a turn tried inside `transpose` and taken back does not put
  other clusters in order; that a neighbour exactly on the diagonal counts for neither side;
  that a tie between the sides keeps the last wish, also after a turn was refused (three
  breaks); and that the groups placed before are asked whether a cluster may turn (the
  hierarchies are asked too, and that is caught).

- **The recorder must build the graph the way TALA's adapter does.** A card has to be added with
  `AddNewNodeToContainer(nil, node)`. With `AddNode` alone TALA's list of top-level cards stays
  empty, and the price knows no card that could stand in the way: the first replay differed by
  exactly the bends for blocked routes. All earlier recorders were changed and run again
  (`docs/recorders/top_level_test.go.txt`). Seven fixtures came out byte for byte the same. The
  visibility graph changed for layouts without any connection: TALA then gives the cards no
  ports. The port does the same now; nothing is routed in such a layout.
- **A mutation run that reports no survivor is not proof.** The first run of 170 deliberate
  breaks reported none because `cargo` did not start in the scratch copy. The runner now checks
  that the unchanged copy passes first.
- **Left out**, because it cannot change a price or a choice for plain cards (each has a comment
  where it would be): the box around two cards that TALA uses to skip cards before testing them;
  stopping to ask once a route is known to be blocked; swapping the two cards for the routes
  around when the first is below or right of the second; the lower limit of two connections for
  "flow"; the early stop at a mirror pair that scores 2; the test for two cards that start on the
  same line; asking whether another card's corner is exactly at a place; skipping the symmetry of
  a place that full symmetry could not make the cheapest (symmetry is never above 1).
- **A card looks further when the optimizer moves it.** In the pass `minimizingSelf` is on: the
  search for the nearest free ring and the list of places also take the places up to the card's
  width to the left of and up to its height above each place. Slice 1 recorded with it off, which
  is what the editor's drop uses.
- **The price of a bend is not the graph's.** Node placement computes it once before the sized
  optimizer runs and halves it. The pass takes it as a number; the fixture has both.
- **An empty list is not nil.** Node placement hands the optimizer an empty list of edge
  abductions for cards in no container, and `transpose` asks whether the list is nil: with nil
  it turns other cards and measures all connections. The recorder passes the empty list.
- **TALA forgets to forget.** With more than 10 cards the search for a free ring remembers the
  offsets it tried. A hub's second move starts with the offsets of the first, around another
  point, and counts them as taken. The port does the same (`Checked`).
- **A hub without its spokes keeps less room to them.** While a hub moves without its spokes,
  their connections are gone from its list, so the gap to a spoke is that of unconnected cards
  (20, not 60); the places beside a neighbour with a large label were worked out before and
  stay. TALA takes the connections out of the hub's list only. The port hides them from every
  card: nothing reads a spoke's list meanwhile.
- **Random numbers come from the seed.** The first recording kept every number TALA drew: 5 MB.
  With Go's generator ported the fixture holds the seed and how many numbers each step drew.
  One number too many or too few in any step and every later choice differs.
- **Left out of the pass**, because it cannot change an answer (each has a comment): the median
  of a single card without sorting, and adding place and half size one by one (the sums are
  exact for whole pixels); skipping the card itself as a swap partner; asking before a swap
  whether each card fits the other's place (the test after the swap asks more); asking both
  ways round whether two cards are too close; rounding a turned card to the grid only when it
  is off it; skipping an offset that was just tried; folding 0 and -0 into one offset; Go's
  step that brings a negative seed into range (the next step does it again).
- **Kept though no test can tell**: a swap prices the second card only when the first gets
  cheaper (it saves work, the answer is the same); a hub's second move does not count as a
  change (only passes without temperature are asked, and they have no second move). Go's
  shuffle draws again when a number falls in the uneven remainder, and `Float64` when it
  rounds to 1: both are ported and no recorded case reaches them (once in billions).
- **A count that is pinned twice is pinned wrong.** Replacing a test's expected counts by a
  search-and-replace also replaced the counters' start values; the test passed in the build it
  was tried in and failed in the next. The runner of the deliberate breaks refused to start on
  it: it checks the unchanged copy first, and touches every file so that no old build passes
  for a new one.
- **Measure before porting.** Running TALA's stages one by one on 200 diagrams, and noting which
  move a card, took an hour and turned "the stages after placement" into a list with numbers.
- **A stage that finds nothing can still change something.** TALA's tree stage takes cards out
  and puts them back. Where no tree remains, the cards and connections are left in another
  order, and every later stage that walks them in order answers differently: 28 of 125
  diagrams. The recorder keeps the order node placement starts from; since the tree search is
  ported, the port arrives at that order itself.
- **Keep what took work outside the scratch folder.** A crash took the scratch folder with the
  TALA clone, one recorder change that was not yet copied to `docs/recorders/` and the scripts
  for the deliberate breaks. The recorder was written again and checked by recording again: the
  fixture came out byte for byte the same. A recorder is copied to `docs/recorders/` when its
  fixture is, not at the end.
- **A break that never ends takes the machine with it.** Two breaks of the tree search made it
  go round forever and add to a list every round. The runner's time limit stopped `cargo`, not
  the test program under it: that one went on to 125 GB and the session died. The runner now
  builds first and runs the test program itself, and the program stops without the runner: its
  own allocator ends it above 1 GB (added to the scratch copy only), the kernel after 120 s of
  processor time.
- **A break nothing catches may point at a line nobody needs.** The first 88 breaks of the tree
  search left 19 uncaught. 11 of them were lines that cannot change an answer in the port, and
  are gone: clearing the connection of a turned chain's last card and the parent of its new
  root; joining the turned chain from its end; putting connections back into the cards' own
  lists and walking a put-back branch as a tree (only chains are put back, and nothing reads
  the lists afterwards); skipping a card that became terminal earlier in its round (it cannot);
  asking whether a card's branches point both ways (the arrows noted at the card say so, also
  for branches that stayed); keeping a lone card's single tree when it branches (the general
  rule keeps it too); sorting the trees by card (TALA keeps them in a map).
- **Kept though no test or probe diagram can tell** (4 of 70 deliberate breaks of the tree
  search): a tree's direction is taken from the first card under its root, and any other would
  do (cards join a root only where they all point one way); a branch's direction is taken from
  its root, and the card under it would do (where they differ, the card the branch hangs on
  stays in the graph and never asks); the
  turned chain's last card is left out of the joint, as in TALA, and its first would do (the
  turn moves it); a card whose branches all stayed is not asked for its arrows (it has them
  both ways only with two such branches, and then it is no card of the round).
- **TALA lays trees out inside node placement, not after it.** `placeNodes` calls `trees.Place`
  for every group as soon as its cards are placed, before the group is mirrored and packed. So
  the places recorded after the stage already had the trees in them, and no new recording was
  needed to start: 42 of 49 diagrams with trees were equal at the first run.
- **Which graph a card belongs to decides what counts.** The mirror to the flow direction counts
  arrows card by card and passes over a card of another graph. By then TALA has handed a
  group's tree cards back to the whole diagram, so the arrows between two cards of a tree do
  not count; around a diagram's only card they do, because that card is never split off. Four
  diagrams with trees two levels deep came out mirrored until the port did the same.
- **A card that trees hang on is not turned.** `transpose` leaves it alone, also while the
  trees are out of the graph. Three diagrams differed by that, all with seed 3.
- **Random diagrams have small trees.** Of 105 deliberate breaks of laying trees out, 3,000
  random diagrams of up to 14 cards left 19 uncaught: the lines for children of unequal width,
  for subtrees that push each other and for a card with three trees. A generator for the thing
  under test (`probe_stages write ... forest`) caught 8 more, with diagrams of up to 28 cards;
  seven of those are in the fixture now.
- **Left out of laying trees out**, because it cannot change an answer (each has a comment
  where it would be): the penalty for a side that is not the container's direction (plain
  cards have none, so every side gets it); a number and a place in the asking order for a tree
  card that joins the graph (only the optimizer reads them).
- **Cannot change an answer** (9 of 105 deliberate breaks of laying trees out): asking whether
  two cards are too close both ways round (the gap is the same); asking two cards of the tree
  that moves, and asking a tree that did not move; a tree to the left as swap then flip or
  flip then swap; the least room of 50 between children (they never stand closer); rounding
  the move of a tree (cards are on whole pixels); a stop at the border (the tree stands
  there); the order of the stops, and taking a stop that is as near as the best (the nearest
  stop that is not refused wins in any order).
- **Kept though no test or probe diagram can tell** (6 of 105): the pairs of cards that are too
  close already when the tree stands at the border, which may stay so (three breaks; no such
  pair in 6,226 diagrams); leaving out the stops beyond the card the tree hangs on; and that
  the third tree of a card with other connections may take a side again (two breaks).
- **Random diagrams give the ranking nothing to do.** Of 6,000 random diagrams and forests,
  12 needed a single step of the network simplex: the first ranking, by the longest path, is
  nearly always the best one. So the breaks of its steps went uncaught, 36 of 71 at first. A
  generator for layered diagrams (`probe_stages write ... layers`) made 940 hierarchies of up
  to 8 levels in 3,000 diagrams, and about 200 of them need a step.
- **A probe too slow to ask is not asked.** Comparing node placement for the layered set takes
  four minutes; the breaks of the hierarchy stage only need its own answers. `compare ...
  quick` leaves node placement out and takes six seconds.
- **Where no generator makes the case, write the question by hand.** Three recorded diagrams
  are hand-made: one card, a row and one card with an arrow from the first straight to the
  last (which TALA takes for a hierarchy, where without that arrow it does not); twenty cards
  on three levels (too wide); a hierarchy of cards with odd sizes, for the rounding of half
  pixels. The answers are TALA's, as for every other diagram.
- **Go's maps have no order, TALA's answers have one.** The hierarchy code walks maps in many
  places (the cards above a card, the arrows of a ring). Where the order could matter TALA
  sorts afterwards, by rank or by number; the port keeps lists and sorts the same way. One
  such sort decides nothing and is left out: the ranking's arrows by their ends (they are
  asked by number, which is the order they come in).
- **Cannot change an answer** (5 deliberate breaks of the hierarchy stage): a connection
  inside the row of "one card, a row, one card", and going on after a card of the row has
  both its connections (two cards with a connection are never on one level); the height of a
  dummy (nothing reads where it stands), and counting dummies for the height of a level; the
  shift of a class of blocks for a card that is not the first of its block (only a first card
  can be asked for it).
- **Kept though no test or probe diagram can tell** (5 deliberate breaks of the hierarchy
  stage): merging doubled connections into one arrow with a weight (the ranking would give the
  same levels with both); that an arrow joins the growing tree only from the side the tree
  was moved for; and the rule that three arrows in five must point down (three breaks). In
  9,247 diagrams that rule never decided: where it fails, the shape or the number of
  connections has refused the group already. A case could not be built by hand either.
- **Go keeps test answers.** Recording again after only the file of found diagrams had changed
  wrote the old 217 diagrams: `go test` answered from its cache. `-count=1` records.
- **A group taken out of a diagram keeps three orders.** Its cards come in the order the split
  found them; ties are broken by the cards' numbers (`Graph::ids`); and the cards that may stand
  in the way are asked in the order of the whole diagram (`Graph::asking_order`), which decides
  for cards half out of line. Until the fixture had groups whose three orders differ, one list
  served for all.
- **Equal is not in place.** TALA sorts the groups by area with Go's `sort.Slice`, which keeps
  equal elements in order only up to 12 of them. Thirteen lone cards of one size come out
  shuffled. `src/gosort.rs` is Go's sort, checked against 90 sorts recorded from Go.
- **TALA changes the size of cards.** The editor's cards are all 220 by 120. "Arrange" will
  have to say what happens to a card TALA makes higher (open, slice 7).
- **Left out of the stage**, because it cannot change an answer (each has a comment): leaving
  out a card's connection to itself; dividing by fewer than four sides; not growing a card that
  is large enough already; sorting a group of cards that drifted apart by number.
- **Kept though no test or probe diagram can tell** (3 of 83 deliberate breaks): the cards a
  card shares three labelled connections with are taken by number (no group in the tests has
  such cards in another order); and two lines of the heap sort Go's sort falls back to on input
  made to defeat it.
- **A stage that looks like a no-op may not be one.** `JoinDistancedClusters` sounded like a
  matter of D2's clusters. It pulls any groups of cards together that drifted apart. The first
  replay differed at the second compaction of the sized loop; a trace of every step from both
  sides (`probe_places trace`) named the step.
- **Minus zero is a place.** Rounding a small negative number gives -0. Go's comparisons call
  -0 and 0 equal; a sort by Rust's `total_cmp` puts -0 first, and compaction then takes the
  cards in another order. 10 of 3,000 groups differed until the sort compared as Go does.
- **A list that is started anew loses what was in it.** In `shiftSubgraphs` a card that sees no
  card behind it starts its group with itself, also when cards were attached to it already
  (the lines of sight are from before the cards moved). Appending instead kept them: 3 of 3,000
  groups differed.
- **Not every bit is TALA's.** The cooling factor and the step size of the first arrangement by
  graph distance come from Go's `math.Pow`, whose last bit depends on the processor (its `Exp`
  uses fused multiply-add where the chip has it); `cos` and `sin` differ between math libraries
  too. The port uses Rust's. Cells and grid places are whole numbers, so a last bit decides
  only on an exact tie: 3,000 groups show none.
- **Left out of node placement**, because it cannot change an answer (each has a comment):
  rounding to the grid only what is off it; sorting the roots of the groups once more; adding
  a group's own place to its moves; leaving out neighbour cells whose card has no connection.
- **Kept though no test or probe layout can tell** (8 of 186 deliberate breaks): one cell more
  around the median in the first arrangement; the first of two equally near cells and the stop
  at the first ring with a free cell in the arrangement by graph distance; a card exactly at
  the edge of a line of sight; a group shifted onto a taken cell and a card moved onto one
  (sizeless); a card that changes groups without bringing its neighbours; a compaction factor
  below 1 in the last rounds.
- **Go's `math.Hypot` is not the C library's.** The port repeats Go's steps
  (`p * sqrt(1 + (q/p)^2)`), or the last bit of "flow" differs.
- **A pair of cards can mirror each other across both centre lines of a card at once** when the
  cell is large against the cards. TALA's comment says it cannot happen; the order in which the
  two lines are tried then decides the score. Not possible with the editor's cards of 220 by
  120; the fixture holds such layouts.

## How it is checked

- `tests/move_reference.rs` replays 341 recorded moves: the places looked at, edge length and
  symmetry at each place a card can go to (bit for bit), and the place taken, with and without
  "must improve".
- 156 deliberate breaks of the ported lines: each makes the replay fail. 41 of the 341 layouts
  were found with the probe for breaks that random layouts did not catch.
- `cargo run --release --example probe_moves` against TALA: 40,000 random layouts, 1,980,062
  places, 263,499 of them priced, no difference.
- `tests/random_reference.rs`: Go's numbers for 10 seeds, the first five and the 700th to 704th,
  three `Float64`, three `Uint32` and a shuffle of twelve.
- `tests/pass_reference.rs` replays 322 recorded layouts with one generator each, in the
  recorder's order: 2071 median points, the swap partner of every card (210 swaps), the turn
  around every card (330 turns) and the pass itself (318 change something, 179 with a
  temperature), each with the count of numbers drawn. 22 of the layouts were found with the
  probe for breaks that the 300 random ones did not catch.
- 133 deliberate breaks of the pass, the generator and the search with `minimizingSelf`: each
  makes a replay fail.
- `cargo run --release --example probe_passes` against TALA: 20,000 random layouts (159,709
  cards, 18,208 swaps, 22,958 turns), no difference.
- `tests/place_reference.rs` replays 139 recorded node placements stage by stage: the first
  arrangement (50 by graph distance), the end of the sizeless loop, the change to sizes, the
  grid with the price of a bend, the end of the sized loop and the final places, each with the
  count of random numbers drawn (5,051,738 in all). 19 of the groups were found with the probe.
- 186 deliberate breaks of node placement: 178 make the replay fail; the other 8 are listed
  above.
- `cargo run --release --example probe_places` against TALA: 3,000 groups of up to 12 cards
  (21,056 cards, 1,245 arranged by graph distance, 126,531,339 random numbers), no difference.
- `tests/node_placement.rs`: a single card, two cards, cards no connection leads to, and 40
  random groups that end on the grid, all cards apart, the same places for the same seed; and
  40 diagrams of several groups that end packed from the origin with all cards apart.
- `tests/stages_reference.rs` replays the node placement stage for the 203 of 467 recorded
  diagrams in which TALA finds no tree, hierarchy or cluster (1361 cards; in 34 of them cards
  grow): sizes, places and cell size. 232 of the diagrams were found with the probe or, 39 of
  them, written by hand.
- `tests/stages_reference.rs` replays node placement with trees for the 73 recorded diagrams
  that have trees and no hierarchy or cluster (647 cards): every card's place, and the cell
  size. The cards start somewhere, not at 0.
- `tests/stages_reference.rs` also replays the tree stage for all 467 recorded diagrams: the
  cards and connections left in the graph, in order, and the trees (99 diagrams keep trees, with
  515 cards in them; 213 more are only reordered). Node placement is replayed from the order the
  port's own tree stage leaves.
- 70 deliberate breaks of the tree stage: 66 make a replay fail, 5 of them by never ending; the
  other 4 are listed above. Two were caught by the probe only, until the fixture got a diagram
  for each.
- 105 deliberate breaks of laying trees out, of the mirror's count and of the card that is not
  turned: 90 make a replay fail; the other 15 are listed above. 18 were caught by the probe
  only, until the fixture got seven diagrams for them.
- `tests/sort_reference.rs`: 90 sorts recorded from Go, of 0 to 700 keys with many equal ones;
  in 42 of them a sort that keeps equal keys in place gives another order.
- 83 deliberate breaks of the stage, the three orders and Go's sort: 80 make a replay fail; the
  other 3 are listed above.
- `tests/stages_reference.rs` replays the hierarchy stage for all 467 recorded diagrams: every
  card's level, or that it is in none (55 diagrams have a hierarchy, 584 cards are in one), and
  the place the stage gives every card of a hierarchy. For the 52 without a cluster it also
  replays node placement (582 cards; two of them have trees too).
- `tests/stages_reference.rs` replays the cluster stage for all 467 recorded diagrams: the
  clusters (139 diagrams have one; 179 clusters of 466 cards, 30 of them rows), every card's
  size, and the cards and connections node placement starts from. For those 139 it also replays
  node placement (1374 cards; in 32 a cluster turns): every card's place and size, the cell
  size, and the arrangement, padding and box of every cluster.
- 136 deliberate breaks of the cluster code (finding clusters, the price of a connection to
  a cluster, the turn, the order of a mirrored cluster's cards): 128 make a replay fail; the
  other 8 are listed above. In a first run 12 were caught by the probe only, until the fixture
  got nine diagrams for them, and 8 pointed at code that could go (also listed above).
- 65 deliberate breaks of finding hierarchies: 58 make a replay fail, 2 of them by never
  ending; the other 7 are listed above. One was caught by the probe only, until the fixture
  got a diagram for it.
- 108 deliberate breaks of laying a hierarchy out: 105 make a replay fail, one of them by never
  ending; the other 3 are listed above.
- **Clusters in cluster-heavy sets** (`probe_stages write <file> 20000 <seed> 16 clusters`,
  2026-11-24). Seven such sets have about 55,000 diagrams with a cluster. They found four
  things TALA does that the 9,000 earlier probe diagrams never needed:
  1. **TALA caches the price of the whole diagram** (`EdgeLength`, `Graph.edgeLengthCache`),
     keyed by the cell size, the prices of a bend and a crossing as the graph has them stored,
     and every card's number, place, size and connections. The cards of a cluster are not in
     the key. Right after node placement they can stand in the opposite order; when the
     graph's cards later stand exactly where they stood then, TALA takes the old price,
     worked out with the cards the other way round. The port keeps the same cache
     (`Graph::length_cache`). The stored prices start at 0 and are set by the first price
     worked out; the gap stage sets the bend's back to 0 at its end (`ResetTurnCost`). So
     the key of the first price differs from all later ones. The port must also ask for the
     price where TALA does even if it does not need it: every round of lining up starts
     with one (`initialLength`).
  2. **A transaction lets through the pairs that were too close when it was made**, also
     after later commits (`existingOverlaps` "intentionally remain anchored to transaction
     construction"). Swap, `Dejitter`, lining up and the gap stage each have one transaction
     for the whole stage; the port now anchors `Allowed` there, not after every accepted move.
  3. **A transaction asks only the pairs with a card that moved since its rollback point**,
     and never asks from a card of a cluster (`hasBadPostStateOverlap`). The second matters
     after routing, where the cards of clusters are cards of the graph: two of them may come
     as close as they like (the second `BinPack`). The first matters in the gap stage, where
     the pull on the other side is a nested try whose rollback point is after the first pull
     (`Allowed::now_since`).
  4. **Every card of the gap stage is one commit**, and a commit puts the cards of every
     cluster in order, also where nothing moved.
  5. **`shiftSubgraphs` also tries the group's own place**, last among its moves (TALA
     appends the move by nothing). That move is priced like the others, with the symmetry
     taken off, while the price to beat at the start is the plain one. So staying can win
     over a move that beats the starting price, and the group stays. The port pushes the
     home place onto the moves (`compaction::shift_subgraphs`).
  6. **TALA adds the parts of a cluster's penalty to the distance one by one**
     (`evaluateNodeEdgeLength`): a bad arrangement, the side of the other neighbour, then
     the distance from the middle. Added up first, the sum rounds otherwise, and two places
     of equal price in TALA are not equal in the port
     (`placementcost::edge_length::cluster_penalty` takes the distance to add to).
  6 was the three diagrams that differed right after node placement; 5 changes none of
  them, nor any of 12,000 other cluster diagrams, and is kept because TALA does it. The
  same group placed alone with `probe_places` had matched: there no two places came that
  close in price.
  Checked: `probe_stages compare` on the seven cluster sets, the random sets, the forests
  and the layered set: no difference. 19 diagrams of these kinds are in the fixtures now.
  Deliberate breaks (`tools/breaks/cluster_placement_breaks.py`,
  `LIST=cluster_placement`): 22, of which 17 make a replay fail (one only after the
  fixture got a diagram for it). **Kept though no test or probe diagram can tell** (tried on
  2,000 cluster diagrams of a cluster set, the `Dejitter` one on 20,305 routed ones, the
  home place of `shiftSubgraphs` on 12,000 cluster diagrams of three sets): that
  `shiftSubgraphs` also tries the group's own place (5 above); that the
  gap stage's reset forgets only the bend's price; where lining up anchors its transaction
  (per stage, not per round or per connection: two breaks); where the cluster stage's pull
  anchors its own; and, of the older lists, that swaps and `Dejitter` keep the anchor of
  their stage after an accepted move (`swap` 32, `dejitter` 37).
- `cargo run --release --example probe_stages` against TALA, three sets of 3,000 diagrams:
  random ones of up to 14 cards; forests of up to 30 (`write ... forest`: one to three cards
  with large trees on them); layered ones of up to 40 (`write ... layers`: three to six rows,
  arrows mostly down, some long, some up, some doubled). The tree stage, the hierarchy
  stage (levels and places) and the cluster stage differ in none of the 9,000: 1,500 have a
  hierarchy, of up to 9 levels (24,785 cards in one); 2,122 have a cluster (2,854 clusters, 159
  of them rows, 638 of more than two cards). After node placement none of the 9,000 differs
  (in 167 a cluster turns).
- `cargo run --release --example probe_places` with `"clusters": true` in the questions, which
  runs the cluster stage first: 400 groups of up to 12 cards, no difference at any step.
- `tests/stages_reference.rs` replays, for all 467 recorded diagrams, the graph node placement
  leaves (the cards, the connections, the top-level list and every card's own list of
  connections, each in TALA's order), the places after the swap stage (it moves a card in 76),
  after the transpose stage (in 31), after the stage that lines cards up (in 210), after the
  gap stage (in 145), after the stage that lines cards up again (in 23), after the stage that
  puts clusters right (in 29) and after the stage that lines cards up a third time (in 6).
- 67 deliberate breaks of the two stages and of the graph they start from: 57 make a replay
  fail; the other 10 are listed above. In a first run 11 were caught by the probe only, until
  the fixture got ten diagrams for them.
- `probe_stages compare` also checks the places after each of those stages: no difference in
  the 9,000 diagrams. The swap stage moves a card in 1,697 of them, the transpose stage in
  949, the stage that lines cards up in 3,896, the gap stage in 3,634, the stage that lines
  cards up again in 535, the stage that puts clusters right in 89, and the stage that lines
  cards up a third time in 12.
- `tests/stages_reference.rs` also replays, for all 467 recorded diagrams, the places after
  `BalanceSymmetry` (it moves a card in 28), after `Equidistance` (in 103) and after the stage
  that lines cards up a fourth time (in 26).
- `tests/stages_reference.rs` also replays, for all 467 recorded diagrams, the places after
  `BinPack` (it moves a card in 105) and after `Rescale` (in all).
- 110 deliberate breaks of the packing stage: 87 make a replay fail; the other 23 are listed
  above. In a first run 19 were caught by the probe only, until the fixture got thirteen
  diagrams for them, and one pointed at code that could go (also listed above).
- `probe_stages compare` checks the places after `BinPack` too: no difference in the 9,000
  diagrams; it moves a card in 2,225 of them. The places after `Rescale` were compared for
  886 of them.
- 96 deliberate breaks of the two stages: 72 make a replay fail; the other 24 are listed
  above. In a first run 13 were caught by the probe only, until the fixture got nine
  diagrams for them, and one pointed at a check that could go (also listed above).
- `probe_stages compare` checks the places after these three too: no difference in the
  9,000 diagrams. `BalanceSymmetry` moves a card in 272 of them, `Equidistance` in 2,799, and
  the stage that lines cards up a fourth time in 560.
- 26 deliberate breaks of the stage that puts clusters right: 22 make a replay fail; the other
  4 are listed above. In a first run 2 were caught by the probe only, until the fixture got a
  diagram for them, and 5 pointed at code that could go (also listed above).
- 50 deliberate breaks of the gap stage: 44 make a replay fail; the other 6 are listed above.
  In a first run 4 were caught by the probe only, until the fixture got a diagram for each,
  and 2 pointed at conditions that could go (also listed above).
- 40 deliberate breaks of the stage that lines cards up: 26 make a replay fail; the other 14
  are listed above. In a first run 8 were caught by the probe only, until the fixture got
  seven diagrams for them.
- `tests/optimizer.rs`: whatever the layout, a pass without temperature keeps every card on the
  grid, brings no two cards closer than they must be, and moves no card without a connection
  (80 layouts, up to ten passes each).
