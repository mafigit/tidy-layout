# Porting TALA's router

Goal: connectors that go around cards, with the routes TALA would draw. TALA's router is
`internal/routing` (39 files, about 16,000 lines of Go without tests) plus `internal/labeling`.
We port what plain cards need, in the order the router runs, one slice at a time. Each slice has
its own recorded reference ([reference.md](reference.md)).

Entry point in TALA: `RouteEdges(ctx, graph, edges)` in `standalone_router.go`. It routes the given
connections between cards that already have their places, which is what the editor needs.

## Slices

| # | Slice | TALA | State |
|---|---|---|---|
| 1 | The visibility graph: the points a route may pass and the free stretches between them | `ovg.go`, `ovg_node.go`, `ovg_edge.go`, `tunnel.go`, lookups in `ovg_resource.go` | Done: `src/routing/ovg.rs`, `tunnel.rs`, `geometry.rs` |
| 2 | The search for one connection through that graph, with the routes found so far in the way | `ovg_edge_router.go` (`search`, `usedPorts`, `bestRoute`, `addRoute`), `priority_queue.go`, `ovg_crossing_index.go`, `turns.go`, `route_directions.go` | Done: `src/routing/router.rs` |
| 3 | All connections: routes by rule for cards on a diagonal (one or two bends), three orders of routing, the cheapest result wins | `slingshot.go`, `ovg_edge_set.go`, `coordinator.go` (`routeAdditionalEdgesWithResourceLimits`, `finalizeSelectedRoutes`), `sortEdges`, `generateRoutes` | Done: `src/routing/slingshot.rs`, `coordinator.rs` |
| 4 | `route` in `src/api.rs`, the WebAssembly export, the editor draws the routes | ours | Done |
| 5 | After the search: a straight line for cards close together and where it is cheaper; spreading routes over the room between cards | `coordinator.go` (`routeLineChecked`, `quickRoute`, `tryStraightEdgeFallback`), `cost.go`, `postprocess.go` (`balanceRegularEdgesGuarded`), `balance_route_guard.go`, `balance_crossings.go` | Done: `src/routing/line.rs`, `balance.rs`. `coordinator::route_edges` is TALA's `RouteEdges` |
| 6 | Label places on the routes; doubled connections swap routes to give a label room | `labeling` (`placeNewEdges`, `findBestEdgeLabelPosition`, `findSharedSegmentsChecked`), `reorderDuplicatesInEdgesGuarded`, D2 `lib/label` (`GetPointOnRoute`) | Done: `src/routing/labels.rs`. `coordinator::route_edges_with_labels` |
| 7 | The first routing stage of TALA's pipeline (`EdgeRouting`), for plain cards: every group of connected cards routed alone, the routes of connections between the same two cards put in the order of the connections | `graph_stage.go` (`RouteGraph`), `coordinator.go` (`routeEdgesWithResourceGuards`, `reorderSelectedRoutes`), `route.go` (`canSwapEdgesGuarded`), `layoutgraph/graph.go` (`splitSubgraphsWithOwnership`) | Done: `coordinator::route_graph`. Not used by the editor yet |
| 8 | The pipeline's later routing stages, for plain cards: a hook is taken out of a route (`SimplifyEdgeRoutes`), two connections of a card exchange their ports (`SwapEdgePorts`), then the steps of slices 5 and 6 | `edge_simplify.go`, `swap_ports.go`, `engine/pipeline.go` | Done: `simplify.rs`, `swap_ports.rs`, `coordinator::route_stages`. Not used by the editor yet |
| 9 | The last routing stage that changes plain routes: channels are nudged (`NudgeEdgeChannels`). It judges its work by what TALA counts on a drawing | `nudge_channels.go`, `port_interior_balance.go` (`changedRouteIsClear`), `internal/quality` (`Inspect`: `inspection.go`, `inspection_geometry.go`, `inspection_labels.go`, `crossings.go`) | Done: `nudge.rs`, `src/quality.rs` |
| 10 | The pipeline's routing for trees: the connections of a tree are routed by rule, straight or as an S, before the search; the later stages leave them alone; their labels go where the tree stage wants them | `tree_routes.go`, `ovg.go` (`addTreeNodes`), `coordinator.go` (`routeSentinelEdge`, `routeInSShape`, `isStraightEdgeFallbackCandidate`), `tunnel.go` (`buildTunnels`), `postprocess.go` (`isSpecialEdgeForBalancing`), `labeling/placement.go` (`findBestEdgeLabelPosition`) | Done: `src/routing/tree_routes.rs`, and in `coordinator.rs`, `tunnel.rs`, `balance.rs`, `swap_ports.rs`, `line.rs`, `labels.rs`, `src/dejitter.rs` |
| 11 | The pipeline's routing for hierarchies: the routing graph gets rows between the levels and points beside the cards of every level; one order of routing, from the top; no straight line instead of a route | `ovg_hierarchy.go` (`newOVGForHierarchy`, `addHierarchyLevelNodes`, `addBoundaryNodesAboveLevelNodes`, `addPortAlignedNodes`, `addLevelNodes`), `ovg.go` (`mergePorts`, `mergeNonPorts`, `addNewBoundaryLayers`, `addEdgesNodes`), `ovg_edge_router.go` (`sortEdges`), `coordinator.go` (`isStraightEdgeFallbackCandidate`) | Done: `src/routing/ovg_hierarchy.rs`, and in `ovg.rs`, `coordinator.rs`, `line.rs`, `src/pack.rs` |
| 12 | The pipeline's routing for clusters: the search prefers the sides that face each other and the ports the other cards of the cluster use; cluster connections first; no rule routes, tunnels or straight lines for their cards; `Crosshatch`, `FixClusterEdgeBranching`, `ShortcutEdgeRoutes`; label places of a cluster; the graph as `CleanupStuff` leaves it | `ovg_edge_router.go` (`usedPorts`, `isUndesirableClusterArrangement`, `sortEdges`, `search`), `slingshot.go`, `tunnel.go`, `ovg.go` (`halfwayPoints`), `coordinator.go` (`Crosshatch`, `isStraightEdgeFallbackCandidate`), `postprocess.go` (`getSharedCluster`, `FixClusterEdgeBranching`), `shortcut_routes.go`, `labeling/placement.go`, `placement/dejitter.go`, `grouping/lifecycle.go` (`Cleanup`) | Done: `src/routing/crosshatch.rs`, `cluster_branching.rs`, `shortcut.rs`, `place::cleanup`, and in `router.rs`, `coordinator.rs`, `ovg.rs`, `tunnel.rs`, `slingshot.rs`, `line.rs`, `balance.rs`, `labels.rs`, `src/dejitter.rs` |
| 13 | TALA's score of a finished drawing (bends, slanted pieces, crossings, how badly the labels sit; the area for a tie) and its choice between seeds 1, 2 and 3 | `quality/scoring.go` (`EvaluateWithArea`), `score.go`, `crossings.go`, `labels.go` (`scoreExistingLabelPlacements`, `findSharedSegments`), `layoutgraph` (`Graph.Area`), `layout.go` (`coordinateLocalSeeds`), `result.go` | Done: `src/score.rs`, `pipeline::layout_best` |

The editor came before the steps after the search (4 before 5), so that connectors went around
cards as early as possible; slice 5 made the same routes look better.

Reference for slice 3: for each of the three orders, the routes in the order they were found
(points of the graph) and their total price, and the routes that are kept. A mismatch at route k
then points at that one connection.

## The pipeline's routing (slices 7 to 12)

"Arrange" needs the routes TALA's whole pipeline ends with: its last placement stages and its
choice between three seeds read them (see [placement-port.md](placement-port.md), "After
routing"). The pipeline does not call `RouteEdges`. Its stage `EdgeRouting` calls `RouteGraph`,
which differs for plain cards in five things:

1. Every group of connected cards is routed alone, with its own cell size and its own prices
   (a turn, a crossing, a side port). Cards of other groups that reach into the box around
   the group, 100 wider on every side, are in the way. They come before the group's own
   cards in the list the visibility graph is built from (`Graph::nearby`).
2. A card of another group is in the way less strictly than a card of the group. No route
   passes through it, and it has ports, which keep a crossing point 20 off along their row
   and column (`tooClose`). But the rule "no point within 20 of a card" asks the group's own
   cards only, and so do "which cards block the view from a point to a port" and "which
   cards cut a tunnel". So a route may run 10 beside a card of another group.
3. After the search, routes that can swap are given to their connections anew
   (`reorderSelectedRoutes`): connections between the same two cards, which face each other,
   get the routes from left to right or from top to bottom in the order of the connections.
   `RouteEdges` leaves them as the search found them.
4. The order of cards and connections is that of TALA's graph after the placement stages,
   not that of the diagram. The search breaks ties by it.
5. Where the two rule routes with one bend are as good, the tie goes by the numbers the two
   cards have in the diagram (`Node.ID`, `Graph::id`), not by their places in that order.

The stages that follow it are chained in `coordinator::route_stages`. First comes a
placement stage, `Dejitter`: where it moves a card, all connections are routed again (the
second `EdgeRouting`, `coordinator::route_graph_again`). The connections still have their
first routes then, and the box around each group (`routeStageGraphBoundingBox`) takes in
their points and the box of every label with a preset place (the tree stage's) where it
lies on them; only then is the box rounded and widened by 100. So the box can be larger
than the cards, and more cards of other groups are in the way of the search. Before
`PlaceLabels` comes another placement stage, the second `BinPack`. In order:

- `SimplifyEdgeRoutes` (`simplify::simplify`). The search turns only at points of its graph,
  so a route can run past its goal, step aside and come back: a hook of three bends. Where
  the first and the fourth piece of the hook meet at a corner, and the two new pieces run
  through no card and meet no other route, the three bends become one. A route loses one hook
  at a time, until none is left that can go. The router makes such a hook in one of 44,000
  plain diagrams.
- `SwapEdgePorts` (`swap_ports::swap_ports`): two connections of a card that cross right
  beside it exchange their ports, or their first pieces when they leave on sides next to
  each other.
- The steps that were ported for `RouteEdges`: a straight line where it is cheaper
  (`StraightEdgesFallback`), spreading the routes (`BalanceEdgeSegments`), the swap of
  doubled connections for a label (`ReorderDuplicates`) and the label places (`PlaceLabels`).

- `NudgeEdgeChannels` (`nudge::nudge`), after the labels have their places. First the
  upright pieces of all routes are looked at, then the level ones:
  1. Pieces on one line that lie side by side, or closer than 40 to doing so, are one
     group. A group keeps its place if one of its pieces is the first or the last of a
     route, or belongs to a route with a slanted piece, a piece without length or ends a
     pixel out of line.
  2. A group can move between two walls: sides of cards and groups that keep their place.
     A group without a wall on both sides becomes a wall itself (asked twice over).
  3. Two free groups side by side are joined (an arc): they keep their order, and 40
     between them unless all their pieces belong to one route.
  4. Groups that hang together are solved together. Six answers are tried: the gap as it
     is and the largest gap that fits (found by halving), each with every group as early
     as it can be, as late, and in the middle. An answer is taken if it makes all routes
     together shorter, or leaves them as long and widens the smallest gap by a pixel or
     more, and if it is safe: no changed route runs through a card, gets longer, gets a
     piece shorter than 40 that was longer, turns back or meets itself, and no two routes
     cross more often, come to lie on each other or touch where they did not.
  5. Then TALA counts (`quality::inspect`): routes through cards, crossings, covered
     labels, the length of all routes. If one got worse, the whole change of that
     direction is taken back. A label keeps its place along its route and moves with it.
  In the pipeline the stage changes a route in one plain diagram of 500. Layouts of more
  than 256 cards, connections or route points are left alone.

`TraceEdgesToShapeBorder` changes no route in any probe diagram and is not ported.
`Crosshatch` and `ShortcutEdgeRoutes` change no route of plain cards, trees or hierarchies;
they are ported for clusters (slice 12).

Checked:

- `tests/routed_reference.rs` replays 174 recorded diagrams of plain cards (1,303
  connections, 879 labels; in 5 two connected cards touch): the routes after the first
  `EdgeRouting`, after
  `BalanceEdgeSegments` and at the end (before `Normalize`), and the label places after
  `PlaceLabels` and at the end are TALA's in all of them. The test pins those numbers. It
  also counts TALA's own final drawing of each as `quality.Inspect` does: the same six
  numbers, bit for bit.
- `probe_routed compare` for 44,119 more plain diagrams (290,702 connections; in 6 two
  connected cards touch): no difference after the first routing, after
  `BalanceEdgeSegments`, at the end, in the label places and in the six counts. In 25 of
  them `Dejitter` moves a card, in 195 the second
  `BinPack` (see [placement-port.md](placement-port.md), "After routing"): the cards are
  where TALA has them.
- `tests/nudge_reference.rs` replays `NudgeEdgeChannels` on 403 layouts that TALA's
  `RouteEdges` routed and labelled (scattered cards, rows, hubs; 3,586 labels): the routes
  after the stage, and the six counts before and after it, bit for bit. In 203 of them the
  stage changes a route. `probe_nudge compare` does the same for 3,000 more (1,500
  changed): no difference.
- `tests/simplify_reference.rs` replays `SimplifyEdgeRoutes` on 1,200 made-up layouts
  (2,372 routes that walk from a port in upright and level pieces, half of them with a hook
  built in; 1,059 lose a hook, 285 more than one), and `probe_simplify compare` on 60,000
  more: no difference.
- Deliberate breaks (`tools/breaks`), 358 in all, 284 make a replay fail: 65 of the first
  routing stage (45 caught), 37 of `SimplifyEdgeRoutes` (all), 99 of `SwapEdgePorts` (80),
  108 of `NudgeEdgeChannels` (79), 30 of the counts (25) and 19 of the chain of the stages
  (18). 66 of them were caught by a probe only, until the fixtures got 32 diagrams and 3
  layouts for them. 5 more diagrams are the ones with two cards that touch.

What a larger probe found (2026-11-24; the first 1,300 probe diagrams had shown none of it):

- Points 2 and 5 above, and that `SimplifyEdgeRoutes` changes a route of plain cards at all.
  11 of 8,921 diagrams differed after the first routing before that.
- **Two cards that touch.** TALA's own placement leaves two connected cards touching now
  and then (6 of the 44,119 probe diagrams). In 4 of them the port laid other routes than
  TALA, until the search got what it had left out:
  - A port both cards have (`OverlappingPorts`: one of the twelve ports of each, at the
    same point) takes no route. The route goes around a corner to another port.
  - The end of a tunnel is not such a port. Where two cards touch, both ends of a tunnel
    between them are one point, and TALA's route is that point twice: a line without
    length.
  - For two cards of which neither is on a side of the other, a route may leave and reach
    a port across its side either way (TALA's `overlap`).
  - A step between two ports of one card costs four bends (`sharesPortOwner`), and a route
    reaches a port of its second card from outside (`validApproach`).
  With that all 6 have TALA's routes, and TALA's finished drawing. Cards that lie on each
  other are still not checked: the placement never leaves such a pair.

- **Left out**, because it cannot change an answer (each has a comment): that a connection
  from a card to itself swaps with none or is refined (the engine is given none); in
  `refineEdgeGuarded`, the early return for a route without a bend and the question for a U
  shape (such a route has none of the shapes that follow); in `IntersectionPoint`, giving
  up on lines that never meet (their shares are no numbers between 0 and 1); in
  `occupiedPortsGuarded`, skipping a connection without a route (all have one); in
  `simplifyPoints`, the question whether the way out of the first card turns round (the new
  corner lies on the first piece's line, past its end) and the second half of
  `sameRouteDirection` (both pieces lie on one line); in `changedRouteIsClear` as the nudge
  stage calls it, that the two ends of a route are where they were (a piece at an end is in
  a group that keeps its place); in `buildChannelProblem`, the rule for a card of one pixel
  (cards are larger). Kept, though it only saves work: in `channelProposalSafe`, skipping
  two routes that both stay.
- **Kept though no test or probe diagram can tell** (73 of the 358 breaks, tried on the
  44,119 diagrams, and for the nudge stage and the counts on 3,000 layouts):
  - the first routing stage (20): that two cards which touch count as lying on each other
    at all, and with it the looser rule for leaving and for reaching a port there (five
    breaks: never, the strict rule for leaving, for reaching, and any way for each); that a
    route reaches a port of its second card from outside (two); the price of a step between
    two ports of one card (two); that a turned route swaps its ports (nothing reads them
    after); the order of a group's cards; that a card which only touches the wider box is in
    the way, and one that covers a card of the group is not; the cell size of the group
    alone (two breaks); the rounding of the box; the order of the groups; that a port of
    another group's card keeps a point off along its column too; that a card of another
    group does not block the view from a point to a port (two breaks, along a row and along
    a column).
  - `SwapEdgePorts` (19): the rounding of the place where two first pieces cross, and that
    it is taken along the first line; that the middle of a U is level or upright (two); that
    an S shape is no U; what counts as an L shape (two); in the straight line a route may
    become: steps of 5, the last port being free, the order of the ranges and going on to
    the second (four); which card's taken ports count (two); that a route with one bend may
    become a line; that only a route starting with an S loses a bend, and that this counts
    as better (two); the order of the sides (top before bottom, left before right, upright
    ones outside: three); and which of the two routes is made simpler first.
  - the chain (1): that hooks are taken out before ports are swapped. One recorded diagram
    has a hook.
  - `NudgeEdgeChannels` (28): the limit of 256 points (no layout is larger); that a route
    with ends a pixel out of line, with a slanted piece or with a piece without length
    keeps its place (four breaks); that walls are looked for twice, not once or three
    times; that a group before its lower wall, after its upper wall or between walls that
    touch keeps its place (three); that pieces of one route need not stay apart, in the
    arcs and in the gap (two); that a point wanted at two places settles an answer; both
    halves of "the same way", and that a route leaves and reaches its cards as before
    (four); that a new piece is upright or level; that a changed route does not turn back,
    meet itself or keep a doubled point in the list (three); that two routes do not cross
    more often or come to lie on each other (two); that a gap below nothing counts as
    nothing; the second point given to a route left with one; and three of the four counts
    the stage is judged by (a route through a card, a crossing, the length: what the
    answer is asked before already keeps them from getting worse; the covered labels do
    decide).
  - the counts (5): that a piece without length enters no box; cards on each other (no
    drawing has any); and that the first and the last piece of a route do not count at
    their own card (three: a route starts on its card's border and runs away from it).

### Trees (slice 10)

A diagram with trees, and neither hierarchy nor cluster, is routed like one of plain cards,
with this on top (2026-11-24):

1. What routing knows of the trees comes with the graph: per card of a tree the connection
   to the card it hangs on and the side of that card the tree lies on (`Graph::tree_links`,
   TALA `NodeToTree`); the connections TALA takes for tree connections (`tree_edges`: those
   inside a tree, and those of its roots where the card the tree hangs on has nothing but
   trees); every card's own list of connections (`own_edges`); and where the tree stage
   wants a label (`label_presets`).
2. The groups are found by walking each card's own list of connections, which is in another
   order than the graph's once the trees are back. Tunnels are made in that order too. A
   card of a tree gets no tunnel.
3. The routing graph gets the two bends of every tree route as points
   (`tree_routes::add_tree_nodes`): halfway through the 100 between a card and the cards
   that hang on it, one on the line of each port. Both cards use the middle port of the
   sides that face each other. Where the two cards are within 10 of being in line and their
   ports are not, the tree card gets a port straight across from the other card's port, 5
   or more from its corners.
4. A middle port that has a tree route with another arrowhead already (one connection ends
   there, the other starts there) is left for the nearest port of the same side without
   one.
5. Tree connections are routed first, by rule (`tree_routes::tree_routes`): from port to
   port, straight or as an S through the two bends, along edges of the graph. A route that
   cannot be walked there, or runs over a port of a third card, is left to the search. So
   is the connection of a root where the card it hangs on has other connections.
6. The tree routes are in the way of the search. Their prices do not count when the three
   orders of routing are compared.
7. Later stages: `Dejitter` moves no card of a tree. `SwapEdgePorts`, `BalanceEdgeSegments`
   and `NudgeEdgeChannels` leave a connection with a tree card at one end as it is.
   `StraightEdgesFallback` leaves tree connections as they are. The second `BinPack` and
   `Normalize` need nothing more.
8. `PlaceLabels` tries first the place the tree stage wants (see
   [placement-port.md](placement-port.md), "Laying trees out"), at the share of the route
   the tree stage gave and nowhere else along it: that side of the line, the other side,
   the line itself. For a label wanted on the line: the line, above, below. Then come the
   nine places at the route's middle. The first place that covers nothing wins.

Checked:

- `tests/routed_trees_reference.rs` replays 73 recorded diagrams with trees (477
  connections, 377 of them tree connections: 165 straight, 212 as an S; 263 labels, 201 with
  a place wanted by the tree stage): the routes after the first `EdgeRouting`, after
  `BalanceEdgeSegments` and at the end, the label places, the cards after `Dejitter` (3
  diagrams) and the second `BinPack` (5), and the drawing after `Normalize`. For these the
  graph is as TALA has it when routing starts (`tests/fixtures/routed_trees.tala.json`).
  The same test file runs the whole pipeline from the diagram as given: there the port's
  own placement supplies the trees.
- `probe_routed compare` for 26,647 more such diagrams (161,579 connections, 120,495 tree
  connections; from the forests and the five random sets): no difference in any stage.
  `probe_routed whole` for the same: TALA's finished drawing in all.
- The 44,119 plain diagrams still show no difference.
- Deliberate breaks (`tools/breaks/tree_routes_breaks.py`, `LIST=tree_routes`): 93, of
  which 82 make a replay fail. 7 of them were caught by a probe only, until the fixtures
  got seven diagrams for them.
- **Left out**, because it cannot change an answer (each has a comment): in
  `alignedPortNodeForBuild`, asking first whether the two ports are out of line (where they
  are in line, the place across is the card's own port); in `treeEdgePathForBuild`, asking
  whether a known port has any arrowhead (it has one), and leaving the middle port out of
  the ports looked at next (it has another arrowhead, which leaves it out anyway).
- **Kept though no test or probe diagram can tell** (11 of the 93 breaks, tried on 6,285
  probe diagrams with trees; the last two through the whole pipeline): that a side port
  with another arrowhead is not taken either; that a port made for a straight line is no
  middle port; that the walk along a tree route takes the nearest point of the graph, not
  the furthest (two); that a route does not start at a port that is no point of the graph;
  that a tree route may not run over a port of a third card; that a group and the tunnels
  take a card's connections in the order of its own list (two; the routing as a whole
  does, which a diagram tells); that above comes before below for a label wanted on the
  line; that a label counts by its size
  across the line; that a mirroring of the whole diagram that is taken back turns the trees
  back too.

### Hierarchies (slice 11)

A group of cards that is a hierarchy (its cards stand in levels, one under the other) is
routed like a group of plain cards, with this on top (2026-11-24):

1. What routing knows of a hierarchy comes with the graph: per card the number of its
   hierarchy and its level in it (`Graph::levels`, TALA `Node.Hierarchy` and
   `HierarchyLevel`).
2. The hierarchy gets a routing graph of its own first (`ovg_hierarchy::build_hierarchy`):
   its ports, and the points where the rows and columns of those ports meet. Then:
   - Columns 20 apart, from a multiple of 20 left of all its points to one right of them,
     with a column to spare on either side.
   - Between two levels, rows for the routes to run along: one per connection that comes
     down to the lower level, and one more, as far as rows 20 apart fit. The rows are
     counted up from the lower level, evenly, on whole pixels. Each has a point at the far
     left and the far right, and a point in line with every bottom port of the upper level
     and every top port of the lower one.
   - On every level, in each column that passes no card of the level, a point on the row
     of each port that looks sideways there: left of the first card its left ports,
     between two cards the right ports of the one and the left ports of the next, right of
     the last card its right ports. The columns stop 20 short of a card and go on from the
     first multiple of 20 past it, plus 20.
   - The points where those columns meet those rows, under the conditions of all such
     points (not beside a port, not near a card, seen from two ports).
   - Around the box of all these points: three layers of copies of the points on its
     outline, 20 apart, and the corners (`add_boundary_layers`, TALA
     `addNewBoundaryLayers`: left out for plain cards, where the points on the outline are
     ports).
3. The hierarchy's ports join the graph of the whole group before the points where rows
   and columns of ports meet are made, its other points after. A connection between two
   cards of one hierarchy gets no points around its cards and halfway: the hierarchy has
   made its points.
4. A group whose first card is in a hierarchy is routed in one order instead of three
   (`Flavor::TopDownLeftRight`): each connection seen from its upper card; from the top;
   of two that start as high, from the left; from one card, the target further up first,
   and of targets as high the one further left.
5. `StraightEdgesFallback` leaves a connection from a card of a hierarchy as it is. The
   second `BinPack` leaves a hierarchy where it is, as the first does. The other stages
   need nothing more.
6. TALA mirrors and turns a hierarchy that runs up or sideways before it makes the points
   (a diagram with a direction, or with tables). The engine's diagrams have neither: not
   ported.
7. Found with a hierarchy, true for every group: a connection the search finds no way for
   becomes a straight line (`Router::line_instead`, TALA `routeLine`). A card in the middle
   of a dense hierarchy can have no free line outwards. The line is the one of slice 5 where
   that is possible. Otherwise it runs from middle to middle, cut where it leaves the first
   card and where it reaches the second, at the price of its whole length. One of 703
   layered probe diagrams has such a card (five connections); none of the 85,000 others.

Checked:

- `tests/routed_hierarchies_reference.rs` replays 50 recorded diagrams with a hierarchy
  (566 connections, 487 cards in a hierarchy on 196 levels, 316 labels; 12 have other
  groups too): the routes after the first `EdgeRouting`, after `BalanceEdgeSegments` and at
  the end, the label places, and the drawing after `Normalize`
  (`tests/fixtures/routed_hierarchies.tala.json`). The same test file runs the whole
  pipeline from the diagram as given: there the port's own placement supplies the levels.
- `probe_routed compare` for 15,441 more such diagrams (157,066 connections; the five random
  sets and the layered set `3000 11 40 layers`): no difference in any stage.
  `probe_routed whole` for the same: TALA's finished drawing in all.
- The 44,119 plain diagrams and the 26,647 with trees still show no difference.
- Deliberate breaks (`tools/breaks/hierarchy_routes_breaks.py`, `LIST=hierarchy_routes`):
  82, of which 61 make a replay fail. 11 of them were caught by a probe only, until the
  fixtures got eleven diagrams for them, and one more for the straight line.
- **Left out**, because it cannot change an answer (each has a comment): the least distance
  of 20 between two rows (no more rows are asked for than fit at 20 apart); the lower
  number first of two cards at the same place on a level (there are none); leaving the
  ports out when a hierarchy's other points are merged (they are points already).
- **Cannot change an answer in the engine's diagrams** (7 of the 82 breaks; kept as TALA
  has them): a hierarchy is always a whole group of connected cards, so it does not matter
  that only the cards of the hierarchy are taken, that points are made around a connection
  with one end outside, that the order of routing goes by the group's first card, and that
  the straight line is refused by the card a connection starts at (four). And a line from
  inside a card to a point outside meets its outline once, so the order of the sides, the
  first side met, and where the cut for the second card starts do not matter (three).
- **Kept though no test or probe diagram can tell** (14 of the 82 breaks, tried on the
  14,738 probe diagrams, the last three on the 703 layered ones): the points at the far
  left and the far right of a row between two levels (two); that the points on such a row
  are in line with the ports that face each other, not those of the far sides; on a level,
  the rows of the left ports left of the first card and of the right ports past a card
  (two); the points where rows and columns of the hierarchy's own ports meet (the graph of
  the whole group makes them again); of the layers around the hierarchy's points: that
  there are three, those above and below, that a corner goes out sideways, and that new
  points get no layers of their own in the same round (five); and of the straight line for
  a connection without a way: that the line of slice 5 is tried first, that it joins points
  of the graph, and what the line from middle to middle costs (three).

### Clusters (slice 12)

A cluster is a row or a column of like cards that hang on the same cards (see
[placement-port.md](placement-port.md), "Finding clusters"). It is placed as one card, its
vessel. When routing starts the vessels are gone and the cards of the clusters are cards of
the graph again (2026-11-24):

1. What routing knows of the clusters comes with the graph (`Graph::clusters`, TALA
   `Graph.Clusters` after `grouping.Cleanup`): per cluster its cards in its order, its
   arrangement and the arrangement the cards around it asked for (`DesiredArrangement`).
   `place::cleanup` makes that graph from the placement: the vessels leave, the cards of
   every cluster come at the end, cluster after cluster in the order of the vessels'
   numbers, from the vessel's corner on. A connection that ended at a vessel ends at its
   card again. A card of a cluster lists its connections in the order of the graph.
2. The search (`router.rs`). A cluster is arranged towards a card when the card lies above
   or below a row, or beside a column; the box of the cluster runs from its first card to
   its last (`isUndesirableClusterArrangement`). Where a connection has a card of a cluster
   at one end and every cluster at its ends is arranged towards the other end:
   - a port on a side that does not face the other card costs two turns, at both ends;
   - a second connection between the same two cards may take the same port;
   - a port that the routes from the other cards of the source's cluster do not use at the
     target, or that the routes to the other cards of the target's cluster do not use at
     the source, costs two turns (`ClusterPortSharing`). So the routes of a cluster share
     their stretch to the card they hang on.
   A turn on a line halfway between the two cards costs a tenth of a turn, not 0.98, on a
   route from a card of a cluster with an even number of cards in its wanted arrangement.
3. Connections of cards of clusters are routed first, in every order (`sortEdges`). They get
   no rule routes (slice 3), no tunnels, and the halfway points between two cards use the
   box of the whole cluster, unrounded, for a card of a cluster.
4. `Crosshatch`, right after the first routing: where two or more connections of a cluster
   leave or reach one card of it at the same port, each becomes a slanted line from border
   to border on the line between the two middles, unless that runs through a third card.
5. `Dejitter` moves no card of a cluster and no card for a route to one.
   `StraightEdgesFallback` leaves the connections of cards of clusters alone.
6. `BalanceEdgeSegments`: the middle pieces of routes of one cluster (in the arrangement its
   neighbours want) that cross the cluster's line at the same place move together, also
   where they do not overlap (`getSharedCluster`).
7. `FixClusterEdgeBranching`, after balancing: the routes of three pieces from one card to
   the cards of a cluster that branch to both sides of the card at different places get one
   branch point, the bend of one of them, where no route then runs through a card and none
   gets dearer; the cheapest such point wins. TALA keeps a route that already branches at
   the point as it is, but asks that by pointer, of points it has just copied: the answer is
   always no, so every route is moved and checked. TALA takes the clusters of one card in
   the random order of a Go map; here in the order of the card's connections.
8. Labels: a connection of a card of a cluster whose piece at the other card lies on the
   piece of another route of the cluster to that card (`isClusterPathSharedChecked`) tries
   first the side away from the cluster's middle (none for the middle card), then the line,
   then the usual nine positions. A label beside the line is looked for only along the
   piece at the cluster (`labelPercentageSearchRange`). The routes of the other cards of the
   same cluster count twice where the label would cover them.
9. `ShortcutEdgeRoutes`, the last stage before `Normalize`, for every route without a label
   outside trees with five points or more: a part of it is replaced by one corner where that
   saves a bend, is no longer, keeps the ends and the way it leaves and arrives, enters no
   card and no label, keeps 40 (or what the old route kept) to the sides of cards and to
   other routes, and touches other routes only where the old route did. The change is taken
   back if what TALA counts gets worse. A route that is not upright and level all through
   (a slanted line of `Crosshatch`) stops the whole stage. It changes routes in 95 of 20,305
   cluster probe diagrams and in none of the others.

Checked:

- `tests/routed_clusters_reference.rs` replays 113 recorded diagrams with a cluster (1000
  connections, 140 clusters of 371 cards, 605 connections at a card of a cluster, 414
  labels), and runs the whole pipeline from the diagram as given
  (`tests/fixtures/routed_clusters.tala.json`).
- `tests/routed_several_reference.rs` does the same for 28 recorded diagrams with two or
  three kinds at once (26 with trees, 5 with a hierarchy, 26 with clusters).
- `probe_routed compare` on 43,613 cluster probe diagrams (the five random sets, the layered
  set and seven sets of `probe_stages write 20000 <seed> 16 clusters`) and 17,700 with
  several kinds: no difference in any stage. The one of the 17,700 that had differed (after
  `Dejitter` moved a card next to a group with a tree and a hierarchy, the second routing
  gave a connection another row) needed the old routes and the preset labels in the box of
  the second routing (above). The second `BinPack` had differed in 7: TALA's transaction
  asks no pair from a card of a cluster, so two such cards may come close (`Allowed`).
  `probe_routed whole`: none of the 43,613 and the 17,700 differs either.
- The 44,119 plain, 26,647 tree and 15,441 hierarchy probe diagrams still show no
  difference, from TALA's graph and through the whole pipeline.
- Deliberate breaks (`tools/breaks/cluster_routes_breaks.py`, `LIST=cluster_routes`): 107,
  of which 95 make a replay fail. 14 of them were caught by a probe only, until the fixtures
  got thirteen diagrams for them.
- **Cannot change an answer** (5 of the 107; kept as TALA has them): which of the two asks
  which for the side a cluster lies on (the answer is the opposite side either way);
  `Crosshatch` needs two connections at one port, so a cluster with one connection changes
  nothing; a card with one connection has no branches to both sides; swapping back and
  front leaves "both sides" as it is; and the end of the range along a piece is never past
  the whole route.
- **Kept though no test or probe diagram can tell** (7 of the 107, tried on 8,000 cluster
  probe diagrams): that the card itself is not one of the other cards of its cluster; the
  unrounded box of a cluster for the halfway points, and the rounded one for labels; that a
  card of a cluster is not asked for branches, nor a cluster in an arrangement its
  neighbours did not ask for; that a route already at the branch point is moved and checked
  too; and that pieces that only touch share a path.
- The box of the second routing (`tools/breaks/swap_ports_breaks.py`, `LIST=route_stages`,
  run with `TEST=routed_several_reference`): leaving out the old routes and labels makes a
  replay fail. **Kept though no test or probe diagram can tell** (tried on 26,647 tree and
  17,700 several probe diagrams): the preset label boxes in it, and that it is rounded as a
  whole rather than from rounded card corners.

### The score and the seeds (slice 13)

TALA lays a diagram out with seeds 1, 2 and 3 (`DefaultOptions`) and keeps the drawing with
the lowest score (`layout.go`, `coordinateLocalSeeds`). The score is two numbers, compared
in turn (`Score.Compare`): the penalty, then the area. Of equal scores the later seed's
drawing wins: TALA takes a result that is no worse and comes later in its list. A seed
that gives no drawing is left out. The port: `pipeline::layout_best`, `score::evaluate`,
`score::best_seed`.

The penalty (`EvaluateWithArea`), added up in this order:

1. Per connection not to or from a card of a cluster: half a point per bend, and three per
   slanted piece.
2. One point per crossing that is not a shared stretch (`countNonSharedCrossings`, the count
   `quality::inspect` has too).
3. One less the label score (`scoreExistingLabelPlacements`), which is 1 / (1 + the sum
   over all labels, in the order of the connections). Per label: twice the part of it that
   covers cards, plus twice its whole area for every card it comes within 5 of, both over
   its own area; ten per label placed before it that it covers, one per such label it comes
   within 5 of; two per piece of another route it comes within 5 of; one per stretch two
   routes share (`findSharedSegments`, 12.5 to each side) it comes within 5 of. A label
   covers nothing it only touches.

The area is that of the box around the cards, the routes and the labels (`Graph.Area`). The
cards count unrounded; the box of each route with its label is rounded first
(`Edge.boundingBoxValues`), so a route at y = 0.5 counts from 1.

TALA walks the stretches that routes share in the order of a Go map; only how many a label
comes near counts, so the order does not matter. Its own merge sort of the pieces on a
line is a stable sort by start, then end.

Not ported: what the engine's diagrams do not have: cards that hold others, labels and icons
of cards, labels at arrowheads, connections from a card to itself (the engine is given none).

Checked:

- `tests/score_reference.rs`: the score of TALA's finished drawing for all 438 recorded
  diagrams of the five routed fixtures, and for each of seeds 1, 2 and 3; the port picks
  TALA's seed in all. In 256 the seeds score differently; TALA picks seed 1 in 59, seed 2
  in 76 and seed 3 in 303.
- `probe_routed whole` on 137,012 probe diagrams that TALA answered with the scores of
  all three seeds (`STAGES_SEEDS=1`; reference.md): the same drawing, the same score for each seed and the same pick in all 137,012 (38,345 plain, 40,591 with trees, 28,395 with a hierarchy, 20,167 with a cluster, 9,514 with several kinds). One had differed in its area: that showed the rounding per route (above). The rounded area was checked again against TALA's on all 137,012 recorded drawings.
- Deliberate breaks (`tools/breaks/score_breaks.py`, `LIST=score`, `TEST=score_reference`,
  `PROBE_EXAMPLE=probe_routed`, `PROBE_MODE=whole`): 57, of which 51 make a replay fail (one, the rounding of a route's box, only after the fixture got the diagram the probe found). **Cannot change an answer:** rounding the whole box (cards lie on whole pixels and each route's box is rounded already). **Kept though no test or probe diagram can tell** (tried on 701 probe diagrams of all kinds, most with seeds that score differently): that a piece with an end inside the label's box counts without crossing its border, that an end on the border is inside, that a piece that only touches the border meets it, that pieces on one line meet where they overlap, and that the walk along shared stretches follows the piece that reaches furthest. Left out, with a comment in `score.rs`: TALA's rule for labels of a connection from a card to itself, which the engine is never given.

## Not ported, on purpose

TALA handles much more than cards and connections. Left out until the editor has such things:
containers and their children, sequences (they get their own
graph or routes), tables and their column ports, self-loops, fixed and invisible nodes, shapes
other than the rectangle, and the work budgets and cancellation that protect a server from huge
inputs.

## What the port taught us

- **Numbers must be exact.** The search compares prices that differ in the last bit, so the port
  repeats TALA's arithmetic step by step, and the reference compares prices bit for bit. Two
  things got in the way: Go on arm64 fuses multiply and add (record with `GOARCH=amd64`), and
  `serde_json` reads numbers one bit off unless its `float_roundtrip` feature is on (it is, for
  the tests).
- **The search does not depend on the order of the graph's points.** Ties in its queue go to the
  entry that was added first. Three recordings were byte-identical.
- **Left out of the search**, because it cannot decide anything (each has a comment where it
  would be):
  - asking both ways whether one card is on a side of the other: where the first is on no
    side of the second, the second is on none of the first;
  - keeping two cards that lie on each other from the straight line of `quickRoute`: their
    distance is 0, which is too close for that line anyway;
  - asking a second time, for a port of the first card, whether both cards have it: such a
    port is a port of the second card too, and that was asked just before;
  - putting a point's previous neighbour back into the queue: it repeats an entry that point
    already has, or one with an infinite price;
  - the overlap test before marking an edge as crossed by a parallel piece: edges that only
    touch share a point with it, which is asked anyway.
- **Kept as TALA has them, but no recorded case tells them apart from simpler forms** (a deliberate
  break stayed unnoticed on the fixture and on the 312,324 probe routes): skipping an arrival
  that cannot be cheaper (it only saves work); the guard against walking back in a circle in
  `bestRoute` and its "not next to the first card" condition; a route that lies whole on the step
  into a tunnel end; which pieces of a route count for "runs the other way" and for crossings
  (with or without the pieces between a port and its card's centre); the exact distance test for
  "nearby" after the widened range. If a later slice disagrees with TALA, look here first.
- **The same two lists for routing all connections** (slice 3; 250,450 probe connections).
  Left out: skipping the connection's own other card when counting neighbours for the launch
  direction (it lies on the diagonal, which no list holds), and keeping the later port for an
  anchor reached from two ports (runs out of different ports never meet). Kept without a case
  that tells them apart: where a run out of a port stops looking (`overshot`, `undershot`, also
  for routes with two bends), the length test before two routes count as the same, whether pieces
  that only touch count in the set of routed pieces, and that set not holding a piece twice.
- **`traceToShapeBorder` does nothing for plain cards**: it moves an end onto the outline of a
  shape that is not a rectangle. Not ported; the probe compares against TALA's routes after it.
- **How TALA places a label.** It tries twelve positions in a fixed order: beside the middle of
  the route (above, then below), beside its quarters, on the route (middle, quarters), then
  anywhere along it in steps of 2.5 % (above, below, on). Each place gets a price: cards it
  covers or comes closer to than 5 px, pieces of routes it touches, labels placed before it, and
  stretches that several routes share. The cheapest wins, the earlier one among equals, and the
  search stops at a place that costs nothing. Labels of bent routes are placed first, shorter
  routes before longer ones.
- **Labels can change routes.** Of three or more straight connections between the same two cards,
  one with a label that lies between the others swaps its route with an outer one without a
  label (4 of 240 recorded layouts).
- **A route can be one slanted line.** TALA replaces a route by a straight line between two ports
  when that is cheaper; a slanted line counts four times its length, so it wins only over a long
  detour, and it never passes through a card. Two facing cards 21 to 41 apart always get a
  straight line. The editor does not want slanted lines: with `Graph::slanted_lines` off (as
  `route` in `src/api.rs` sets it) only upright and level lines are taken, and the search finds a
  route with bends for the rest. The port itself still gives TALA's lines, and the recorded
  layouts check those.
- **Balancing moves ports.** After it, a route no longer ends at a quarter, half or three quarters
  of a card's side: the pieces at a side share its length evenly.
- **Routes carry their own points.** TALA's straight lines end in points that are not part of the
  graph (and so do not count as used ports). In the port a route holds its coordinates itself.
- **Left out of slice 5** (provably without effect): the early "nothing moves" return in the order
  check of balancing; guarding against a route without points. **Kept without a case that tells
  them apart** on 259,990 probe connections: in the straight line, not sharing a line with a
  route it may not share, the "nearly upright or level" exception from the price of a slanted
  line, the order in which the ports of two sides are tried, and Go's own power function (the
  usual one gave the same prices); in balancing, the second key of the order within a batch, the
  order check against routes that keep their place, three conditions inside "does a reversal
  remove crossings", and the second point given to a route that collapsed into one.
- **Slice 6, kept without a case that tells them apart** on 181,239 probe labels: the exact borders of "a piece of a route
  touches the label" (a point on the border, the fourth side of the box), going on past the end
  of a route, skipping pieces without length, the second key when pieces on one line are
  ordered, closing a shared stretch where two pieces do not overlap, stopping at a place that
  costs nothing (it only saves work), the last step of the search along a route, and "more than
  one other connection" before doubled connections are reordered. Left out: skipping a line
  with a single piece on it when looking for shared stretches (it has none anyway).
- **Speed and size.** The bookshop landscape (22 cards, 27 connections, all with labels) is
  routed and labelled in 7 ms in WebAssembly. The engine is 371 KB, 119 KB compressed.

- **Order of the graph's points.** TALA walks Go maps while it adds points, so the order of
  `OVG.Nodes` (and each point's `Index`) differs between runs. Which points exist, their flags and
  the edges with their order do not. The reference compares exactly that, and three recordings
  were byte-identical.
- **Steps that never decide anything for plain cards** are left out of the port, each with a
  comment where it would be. Found by breaking the port on purpose and seeing the reference still
  pass, then checked in TALA itself:
  - `tooClose` (a port within 20 on the same row or column): such a point is always also within 20
    of that port's card, which is asked next. Counted in TALA over 400 recorded cases: it rejected
    no point that the next question would have let through.
  - `addNewBoundaryLayers` (copies of points on the outline of all cards, further out): for plain
    cards those points are ports, and the next step adds the same points for every port. With the
    step switched off in TALA, 30,000 random layouts gave identical graphs.
  - `IsBlocked` and dropping short stretches in the tunnel search: they only end the work early.
  - `OVG.Connect`'s rule that only ports connect to a card centre: cannot happen while building.
    It is needed in slice 3, where existing routes are added to the graph.
- **Cards 20 apart leave no room between them.** Points keep 20 to every card, borders included,
  so no route passes a gap of 20 or less. A card with no free line outwards gets points right in
  front of its ports (`createPortsConnectionsToBoundaries`).
- **Tunnels.** Two connected cards that face each other get direct lines between their borders,
  one per connection while 40 px each fit. A tunnel end that falls on an existing port turns that
  port into a tunnel end: it no longer connects along its row and column.
