# Recording reference answers from TALA

The port is correct when it answers like TALA. The fixtures in `tests/fixtures/` were recorded by
running TALA's own Go code on generated cases.

1. Clone `d2lang/d2` and check out the commit named in `NOTICE.md`.
2. Add a `_test.go` file next to the function to record (it must be in the same Go package to reach
   unexported functions). It builds random graphs with a fixed seed, calls the function and writes
   inputs and answers as JSON. The recorders used so far are kept in `docs/recorders/` (as
   `.txt`, so nothing tries to compile them here).
3. Run it: `SNAP_REFERENCE_OUT=<file> go test ./d2layouts/d2talalayout/internal/placement -run TestExportSnapReference`
   (each recorder names its own variable and test). Add `-count=1` when only the
   `REFERENCE_FOUND` file has changed: Go otherwise answers from its test cache and writes
   nothing new.
4. Copy the JSON into `tests/fixtures/` and add a Rust test that replays it.

A recorder adds each card with `addTopLevel` (`docs/recorders/top_level_test.go.txt`), which is
how TALA's adapter adds a card that lies in no container. With `Graph.AddNode` alone TALA's list
of top-level cards stays empty and the placement prices see no card in the way.

Never edit a fixture by hand, and never change one to make a test pass: record it again.

Record with `GOARCH=amd64` (on an Apple Silicon Mac the test then runs under Rosetta). Go on arm64
joins a multiplication and an addition into one rounding step, which changes the last bit of some
route prices; x86-64 and WebAssembly do not. In the search reference 9 of 1891 prices differed
between the two, the routes did not.

## Finding cases

Random cases do not reach every line. To find a layout on which a line matters:

1. `cargo run --release --example probe -- steps write probe.json 20000 53` writes random layouts
   with the port's answers. `steps` is everything `RouteEdges` does to the routes; `labels` gives
   most connections a label and adds the label places; `routes` is the routing without the later
   steps and without cards close together; `search` is the search alone. A last number sets the
   most cards per layout (7 without it).
2. TALA answers the same layouts: `PROBE_WHAT=steps PROBE_IN=probe.json PROBE_OUT=probe-tala.json
   go test ./d2layouts/d2talalayout/internal/routing -run 'TestProbe$'`
   (`docs/recorders/probe_test.go.txt`).
3. `probe steps compare probe-tala.json 5` prints the layouts where the port differs from TALA.
   None: the port agrees on all of them.
4. Break the port on purpose and run step 3 again. The layouts it prints are the ones on which
   that line matters. Put them into a "found" file of the recorder, so TALA's answers for them
   become part of the fixture.

The same for one move of a card: `cargo run --release --example probe_moves -- write moves.json
40000 13`, then `PROBE_IN=moves.json PROBE_OUT=moves-tala.json go test
./d2layouts/d2talalayout/internal/placement -run TestMoveProbe` (`docs/recorders/move_test.go.txt`),
then `probe_moves compare moves-tala.json 5`. A quarter of its layouts are unusual on purpose: a
card with eight or nine connections, connections both ways, cards beyond 1200, sides exactly on
a centre line or exactly 40 out of line, a card on top of another.

The same for a pass of the optimizer: `cargo run --release --example probe_passes -- write
passes.json 20000 7`, then `PROBE_IN=passes.json PROBE_OUT=passes-tala.json go test
./d2layouts/d2talalayout/internal/placement -run TestPassProbe` (`docs/recorders/pass_test.go.txt`),
then `probe_passes compare passes-tala.json 5`.

The same for node placement: `cargo run --release --example probe_places -- write places.json
3000 5 12` (the last number is the most cards per group), then `PROBE_IN=places.json
PROBE_OUT=places-tala.json go test ./d2layouts/d2talalayout/internal/placement -run
TestPlaceProbe -timeout 30m` (`docs/recorders/place_test.go.txt`), then `probe_places compare
places-tala.json 5`. To find where one group differs, give it `"trace": true`: both sides then
keep a number for the places after every step, and `probe_places trace places-tala.json` names
the first step that differs. `PLACE_DUMP=1` makes TALA's side keep all places of every step.
With `"clusters": true` in a question both sides run TALA's cluster stage first; the places are
then those of the cards left in the graph, followed by the vessels. The recorder needs
`pass_test.go.txt`, `move_test.go.txt` and `top_level_test.go.txt` (as `package placement`)
beside it.

The same for the node placement stage: `cargo run --release --example probe_stages -- write
stages.json 3000 9 14` writes diagrams (questions only), `PROBE_IN=stages.json
PROBE_OUT=stages-tala.json go test ./d2layouts/d2talalayout/internal/engine -run TestStagesProbe
-timeout 30m` (`docs/recorders/stages_test.go.txt`) answers them, and `probe_stages compare
stages-tala.json 5` (`WHICH=1` names each diagram that differs and the first stage it differs
after, `WHICH=2` prints the places, the port's and TALA's) checks the tree, hierarchy and cluster stages and the places after node
placement and after each of the twelve stages that follow it for every diagram. `write stages.json 3000 5 30
forest` makes forests instead: one to three cards with large trees on them, which random
diagrams seldom have and laying trees out needs. `write stages.json 3000 11 40 layers` makes
layered diagrams: rows of cards with arrows mostly down, which gives TALA's ranking work to
do (of 6,000 other diagrams only 12 needed one step of it). `compare stages-tala.json 5 quick`
leaves node placement out: a full comparison of the layered set takes four minutes, the tree,
hierarchy and cluster stages six seconds.

The same for the routing stages of the pipeline: answer the diagrams of `probe_stages write`
with `STAGES_ROUTED=1` set (`TestStagesProbe` then records routes and label places too), turn
the answers into a file with `python3 docs/recorders/routed_fixture.py <answers> <file>`, and
run `cargo run --release --example probe_routed -- compare <file> 5`. It prints the diagrams
that differ, as the recorder takes them; `WHICH=1` names the connections. Use 20,000 diagrams
per seed and several seeds: about 9,000 of 20,000 are plain cards with a connection, and
TALA answers 20,000 in eight minutes.

The same for `SimplifyEdgeRoutes`: `SIMPLIFY_SEED=7 SIMPLIFY_COUNT=60000
SIMPLIFY_REFERENCE_OUT=<file> go test ./d2layouts/d2talalayout/internal/routing -run
'TestExportSimplifyReference$' -count=1` (`docs/recorders/simplify_test.go.txt`) writes made-up
layouts with TALA's answers, and `cargo run --release --example probe_simplify -- compare
<file> 5` checks them.

## closest-unoccupied.tala.json

500 cases for `findClosestUnoccupiedDistanceGuarded` with `minimizingSelf` off
(`docs/recorders/closest_unoccupied_test.go.txt`). Recorded: the ring TALA reports and the first
candidate on that ring that TALA calls unoccupied.

- 400 cases: 2 to 25 cards, uniform (220 by 120) or mixed sizes, placed anywhere (also
  overlapping), random connections, and a point that is on top of another card in half of them.
- 100 cases: every card on the grid and the point at the moving card's own place. They pin down
  that TALA counts a card's own exact position as taken.

## label-gaps.tala.json

500 cases for `Node.DeltaTo` and for the same search with labelled connections
(`docs/recorders/label_gaps_test.go.txt`): 2 to 15 cards of 220 by 120 on a 20 px grid, connections
of which two thirds carry a minimum width and height as TALA's adapter derives them from a label.
Recorded: the gap TALA demands between the moving card at a point and another card (about 12 per
case, around the other card on all sides and corners), and the search result as above.

- 400 cases with labels wider than tall.
- 100 cases with labels taller than wide. They pin down what TALA does when two cards lie on a
  diagonal: it takes the smaller of the two gaps.


## ovg.tala.json

480 graphs for `buildOVGFromGraphWithLimits`, the visibility graph of the router
(`docs/recorders/ovg_test.go.txt`, `OVG_REFERENCE_OUT`, `TestExportOVGReference`). Recorded per
case: the number of points and edges, how many points are tunnel ends and how many are flagged as
near a port, and a hash (FNV-1a, 64 bit) of the graph written as text: every point with its flags,
the cards it is a port of, and its neighbours in order; every edge in order; every card's ports in
order and its centre. 20 small cases also carry that text, so a difference can be read. The order
of the points themselves is not part of it, because it differs between runs of TALA.

- 260 "scattered": 1 to 9 cards anywhere, at least 20 apart, mostly 220 by 120, some tiny (40 by
  30), one case in five off the 20 px grid, random connections.
- 140 "rows": cards of 220 by 120 in rows and columns with gaps of 20 to 140, some places empty,
  connections mostly between neighbours, sometimes several between the same two cards.
- 80 "between": two long cards facing each other with one to five connections, and small cards in
  the space between them. They cut the stretch for tunnels into pieces.

Cards never overlap in these cases: the editor does not allow it.

75 of the cases have no connection at all. TALA then gives the cards no ports (`addPorts`: "If
all of a container's children have no connections, there's no need to consider them for
pathing"), and the graph is only the frame around the cards. The first recording missed that,
because its recorder added cards without `addTopLevel`.

## passes-through.tala.json

600 cases for `Node.PassesThrough` (same recorder file, `PASSES_REFERENCE_OUT`,
`TestExportPassesThroughReference`): a card and a segment. A third between points of a grid laid
over and around the card (along borders, ending on them, through the middle), a third diagonals
that touch one corner and nothing else (TALA: does not pass through), a third anywhere.

## search.tala.json

312 layouts, 1955 routes, for `ovgEdgeRouter.search`
(`docs/recorders/search_test.go.txt`, `SEARCH_REFERENCE_OUT`, `TestExportSearchReference`; with
`REFERENCE_FOUND=docs/recorders/found_layouts_search.json`). For every layout the connections are searched
in their order, each route is added before the next search. Recorded per layout: TALA's three
prices (turn, crossing, a port that is not the middle one). Per route: its price, bit for bit; the
number of points and a hash of all of them; and its corners (card centres, ports, bends).

Every connection has an arrow at its target, like a relationship in the editor. Two cards that
face each other 21 to 41 apart get no connection: TALA joins those with a straight line without
searching (`quickRoute`), which is a later slice.

- 100 "scattered", 100 "rows" (with doubled and opposite connections), 100 "hub" (one card in the
  middle that most connections start or end at).
- 12 "found": layouts on which a line of the search matters that the others do not reach: the
  cheaper turn on a line halfway between two cards (off the 20 px grid), and a route on a line
  next to another one but not on it.

Beyond the fixture: with the probe ("Finding cases") the port answered 34,000 random layouts
(312,324 routes, up to 18 cards) exactly like TALA.

## routes.tala.json

244 layouts for the whole routing step, `routeAdditionalEdgesWithLimits`
(`docs/recorders/routes_test.go.txt`, `ROUTES_REFERENCE_OUT`, `TestExportRoutesReference`; with
`REFERENCE_FOUND=docs/recorders/found_layouts_routes.json`). 240 layouts are made like those of
the search reference; 4 were found with the probe: in them two orders of routing cost the same
to less than a rounding error, and TALA keeps the first. Recorded per layout, for each of TALA's three
orders of routing (`generateRoutes`): the sum of the routes' prices, bit for bit, and per route its
connection, the number of points and a hash of them, in the order they were found. And the routes
TALA keeps: per connection its ports and bends.

5286 routes over the three orders, 1762 kept ones. TALA keeps shortest first in 187 layouts,
longest first in 29, the given order in 15.

Beyond the fixture: with the probe the port answered 22,500 random layouts (250,450 connections,
up to 18 cards) exactly like TALA, in all three orders and in what is kept.

## steps.tala.json

262 layouts for the steps of `RouteEdges` after the search, one by one
(`docs/recorders/steps_test.go.txt`, `STEPS_REFERENCE_OUT`, `TestExportStepsReference`; with
`REFERENCE_FOUND=docs/recorders/found_layouts_steps.json`). Here cards may be close together: two
facing cards 21 to 41 apart get a straight line during the search (`quickRoute`). Recorded per
layout:

- the sum of the routes' prices in each of the three orders of routing, bit for bit;
- per connection, what `tryStraightEdgeFallback` compares: the price of its route and the price of
  the straight line that could replace it (`estimateRouteCost`, `routeLine`);
- every connection's route after the search, after the straight lines (`straightened`), after
  balancing (`balanced`), and after TALA's last two steps (`traced`, `reordered`).

240 layouts are made like those of the search reference, without leaving close cards out; 22 were
found with the probe, each for a line of the straight-line or the balancing step that the others
do not reach (a route whose ends are one pixel out of line, pieces between whole pixels, a line
that would meet a card's side too flat, and more). 1828 connections; 54 routes become a straight
line, 1315 are moved by balancing.

The recorder checks that its steps are the ones `RouteEdges` runs: the real thing must give the
same routes. TALA's last two steps (`traceToShapeBorder`, `reorderDuplicatesInEdges`) change no
route of plain cards without labels of their own, in the fixture or in the probe.

Beyond the fixture: with the probe the port's final routes equal TALA's on 22,500 random layouts
(259,990 connections, up to 18 cards), every coordinate bit for bit.

## labels.tala.json

246 layouts for `RouteEdges` with labels on about seven connections in ten
(`docs/recorders/labels_test.go.txt`, `LABELS_REFERENCE_OUT`, `TestExportLabelsReference`; with
`REFERENCE_FOUND=docs/recorders/found_layouts_labels.json`). A label is a box 30 to 320 wide and
22 high, one in eight 38 high. Recorded per layout: every connection's final route, whether the
labels changed the routes (`reorderDuplicatesInEdges` swaps the routes of doubled connections),
and per label D2's position number, how far along the route it is, and the top-left corner of its
box (`Edge.LabelTopLeft`).

240 layouts are made like those of the steps reference; 6 were found with the probe (two pieces
of routes that only touch end to end, and doubled connections that must not swap their routes).
1745 connections, 1217 labels, every one of the twelve positions TALA uses for a plain
connection; in 4 layouts the labels change the routes.

Beyond the fixture: with the probe the port's routes and label places equal TALA's on 22,500
random layouts (258,767 connections, 181,239 labels), bit for bit.

## move.tala.json

341 moves of one card as TALA's sized optimizer makes them (`docs/recorders/move_test.go.txt`,
`MOVE_REFERENCE_OUT`, `TestExportMoveReference`; with
`REFERENCE_FOUND=docs/recorders/found_layouts_move.json`). Recorded per move: the cell size and
the price of a bend, the ring of the nearest free place around the point
(`findClosestUnoccupiedDistanceGuarded`), every place `fillPlacementPointsGuarded` then looks at,
in its order (both with `minimizingSelf` off), whether the card can go there (`indexedCanMove`)
and if so `placementcost.NodeEdgeLength` and `NodeSymmetry` with the card there, and where
`moveNodeToBestGuarded` leaves the card, once free to stay and once with "must improve". TALA
shuffles the places before it chooses; the recorder does not, so the choice can be replayed.

- 200 layouts of 2 to 14 cards, on a lattice ("tidy") or anywhere on a grid of 20 ("loose"),
  uniform or mixed sizes, random connections, half of them with a label, now and then several
  between the same two cards; the cell is TALA's own, 20, or 40 to 120.
- 100 layouts where the cards connected to the moving one lie as mirror images of each other,
  exactly or nearly ("mirror").
- 41 layouts found with the probe, one or two per deliberate break that the 300 did not catch.

14,567 places, 1994 of them priced, 236 with symmetry. In 323 layouts the card moves; in 79 "must
improve" keeps it from moving.

Beyond the fixture: with the probe the port equals TALA on 40,000 random layouts (1,980,062
places, 263,499 priced), bit for bit.


## random.go.json

Go's seeded random numbers (`math/rand`, `rand.NewSource`), which TALA's placement draws from
(`docs/recorders/pass_test.go.txt`, `RANDOM_REFERENCE_OUT`, `TestExportRandomReference`). For 10
seeds (1, 0, negative, beyond 31 bits, the one Go uses in place of 0): the first five numbers and
the 700th to 704th (the generator has 607 slots), and from a fresh generator three `Float64`,
three `Uint32` and the numbers 0 to 11 shuffled with TALA's `limits.Shuffle`. This file is Go's
answer, not TALA's.

## pass.tala.json

322 layouts for one pass of the sized optimizer, `sizedOptimizer.optimize`
(`docs/recorders/pass_test.go.txt`, `PASS_REFERENCE_OUT`, `TestExportPassReference`; with
`REFERENCE_FOUND=docs/recorders/found_layouts_pass.json`). The recorder builds the optimizer as
node placement does for cards in no container (an empty list of edge abductions, hubs from
`proximity.AddHubs`, the price of a bend halved in half of the layouts) and gives it Go's
generator with the layout's seed. With that one generator it asks, in this order:

- for every card with a connection the point it is drawn to (`medianPointGuarded`, at the
  layout's temperature);
- for every card the card it would swap places with (`bestSwapCandidateGuarded`);
- for every card what `transpose` turns around it: all places after it, or nothing;
- the pass: whether it changed something and where every card is after it.

Recorded with each answer: how many random numbers it drew. The replay draws from the port's
generator in the same order, so a step that draws one number more or less shows at once.

300 layouts of 2 to 12 cards on the grid of the cell (TALA's own, 20, or 40 to 200), most of them
clear of each other, uniform or mixed sizes (one layout in five in odd pixels), now and then a
card of exactly 100 cells or a giant of more;
connected as a tree, a chain, one or two stars or at random, one connection in four with a label,
now and then several between the same two cards; half of them with a temperature of 0.2 to 3.2.
22 more layouts were found with the probe, one or two per deliberate break that the 300 did not
catch.

437 hubs, 2071 median points, 210 swaps, 330 turns; 318 passes change something.

Beyond the fixture: with the probe the port equals TALA on 20,000 random layouts (159,709 cards,
18,208 swaps, 22,958 turns): every median, swap, turn, final place and count of numbers drawn.

## place.tala.json

139 groups of connected cards for node placement, `placeNodesOrthogonally`
(`docs/recorders/place_test.go.txt`, `PLACE_REFERENCE_OUT`, `TestExportPlaceReference`; with
`REFERENCE_FOUND=docs/recorders/found_layouts_place.json`). The recorder runs the steps of
`placeNodesOrthogonally` one by one with Go's generator, keeps the places and the count of
random numbers drawn after each stage, and checks that the real function ends in the same
places with the same count. Recorded per group: whether the first arrangement is the one by
graph distance; the places in cells after the first arrangement and after the sizeless loop; in
pixels after the change to sizes, on the grid, after the sized loop and at the end; and the
price of a bend the sized optimizer uses.

120 groups of 2 to 10 cards, uniform or mixed sizes, as a tree, a chain, one or two stars or a
ring, with up to two more connections, one connection in four with a label, seeds 1 to 1000.
19 more were found with the probe: for the two bugs the first replay had (minus zero in a sort,
a group list started anew) and for deliberate breaks the 120 did not catch.

50 groups start from the arrangement by graph distance; 5,051,738 random numbers are drawn.

Beyond the fixture: with the probe the port equals TALA on 3,000 groups of up to 12 cards
(21,056 cards, 126,531,339 random numbers) at every stage.

## sort.go.json

Go's `sort.Slice` (`docs/recorders/pass_test.go.txt`, `SORT_REFERENCE_OUT`,
`TestExportSortReference`): 90 lists of 0 to 700 keys (random with few or many values, runs,
descending, all equal, a repeating pattern) and the order Go leaves their places in. This file
is Go's answer, not TALA's.

## stages.tala.json

467 diagrams run through TALA's real pipeline, stage by stage
(`docs/recorders/stages_test.go.txt`, in TALA's `internal/engine`, `STAGES_REFERENCE_OUT`,
`TestExportStagesReference`; with `REFERENCE_FOUND=docs/recorders/found_layouts_stages.json`).
With `STAGES_ROUTED=1` the recorder adds, per diagram, the places and the routes after every
stage from the first routing on that changes one of them (`routed`), where the labels are
right after `PlaceLabels` (`labelsPlaced`) and at the end (`labels`), and the order of cards
and connections in TALA's graph when routing starts (`nodeOrderRouted`, `edgeOrderRouted`).
For a diagram with trees it also adds what TALA knows of them then: per card of a tree the
connection to the card it hangs on and the side of that card the tree lies on
(`treesRouted`), the connections TALA takes for tree connections (`treeEdgesRouted`), every
card's own list of connections and the top-level list (`ownEdgesRouted`, `topLevelRouted`),
and where the tree stage wants the label of a connection (`labelPresetsRouted`: the
position and the share of the route). For a diagram with a hierarchy it adds per card of
one the number of its hierarchy and its level (`hierarchiesRouted`). `cargo run --release --example probe_routed
<file>` measures the router against that recording.

`tests/fixtures/routed.tala.json` is made from it by `docs/recorders/routed_fixture.py`: the 174
diagrams of plain cards with a connection (1,303 connections, 879 with a label; 22 with
several groups, 67 with two connections between the same two cards, 5 with two connected
cards that touch), the cards and
connections in TALA's order at the places after `Rescale`, the number each card has in the
diagram (`ids`: a rule route breaks a tie by it), the routes after the first `EdgeRouting`,
after `BalanceEdgeSegments` and after the last stage before `Normalize`, the label places
after `PlaceLabels` and before `Normalize`, what TALA's `quality.Inspect` counts on the
drawing before `Normalize`, the drawing after `Normalize` (`final`: the cards, the routes,
the labels), and where the cards are after `Dejitter` and after the second `BinPack` if
they move one (`dejittered`, `packed`).
`tests/routed_reference.rs` replays it. The same script turns a recording of probe diagrams
into a file for `probe_routed compare <file> <limit>`: 44,119 more plain diagrams (290,702 connections;
from `probe_stages write <file> 20000 <seed> 14` with the seeds 17, 31, 43, 59 and 71), no
difference after the first routing, after `BalanceEdgeSegments`, at the end (before and
after `Normalize`), in the label places and in the counts. In 6 of them two connected
cards touch: TALA's own placement leaves such a pair now and then.
`Dejitter` moves a card in 25 of them, the second `BinPack` in 195.
`probe_routed whole <file> <limit>` runs the whole pipeline (`pipeline::layout`) from the
diagram as it was given and compares the finished drawing: no difference either (a quarter
of an hour for the 44,119; run the sets side by side). `WHICH=1` names the diagrams and connections that differ.

`tests/fixtures/routed_trees.tala.json` is made by the same script with a third argument
`trees`: the 73 recorded diagrams with trees and neither hierarchy nor cluster (477
connections), with the same lists and four more, as TALA has them when routing starts:
`own` (every card's own list of connections), `trees` (per card of a tree the connection it
hangs by and the side), `treeLinks` (the tree connections) and `presets` (where the tree
stage wants a label). `tests/routed_trees_reference.rs` replays it: the routing stages from
TALA's graph, and the whole pipeline from the diagram as given. `probe_routed compare` and
`probe_routed whole` take such a file too: 26,647 more diagrams with trees (161,579
connections; the forests `3000 5 30 forest` and the five random sets), no difference in any
stage and TALA's finished drawing in all.

`tests/fixtures/routed_hierarchies.tala.json` is made with `hierarchies`: the 50 recorded
diagrams with a hierarchy and neither tree nor cluster (566 connections), with one more
list: `levels`, per card of a hierarchy the number of its hierarchy and its level.
`tests/routed_hierarchies_reference.rs` replays it the same two ways. The probe file of
that kind: 15,441 diagrams (157,066 connections; the layered set `3000 11 40 layers` and
the five random sets), no difference in any stage and TALA's finished drawing in all.

`tests/fixtures/routed_clusters.tala.json` is made with `clusters`: the 113 recorded diagrams
with a cluster and neither tree nor hierarchy (1000 connections), with two more lists:
`own` (every card's own list of connections) and `clusters` (per cluster in TALA's order its
cards, its arrangement and the one the cards around it asked for). The recorder writes them
as `clustersRouted`, with `ownEdgesRouted` and `topLevelRouted`.
`tests/routed_clusters_reference.rs` replays it the same two ways. `routed_several.tala.json`
is made with `several`: the 28 recorded diagrams with two or three kinds at once, with the
lists of each kind they have (`tests/routed_several_reference.rs`).

The probe sets for clusters: `probe_stages write <file> 20000 <seed> 16 clusters` makes
diagrams with groups of like cards on one or two cards (seeds 5, 7, 11, 13, 17, 19 and 23
were used). About 5,700 of 20,000 have a cluster and no tree; most of the rest have a tree,
about 2,200 both. With the five random sets and the layered set they give 43,613 diagrams
with a cluster alone and 17,700 with several kinds (`routed_fixture.py ... clusters` and
`several`). TALA answers 20,000 of them in about half an hour when nine sets run side by side.

The score: with `STAGES_ROUTED=1` the recorder also writes TALA's score of the finished
drawing (`score`: `quality.EvaluateWithArea`, the penalty and the area), and with
`STAGES_SEEDS=1` too the score of the drawings of seeds 1, 2 and 3 (`seedScores`; the
pipeline runs four times per diagram). `routed_fixture.py` carries both into all five routed
fixtures. `tests/score_reference.rs` checks the port's score of every recorded diagram, of
each seed, and its pick among the three. `probe_routed whole` checks them too where a probe
file has them. TALA answers 2,000 diagrams with `STAGES_SEEDS=1` in about four minutes;
146,000 probe diagrams (the sets of seeds 31, 43, 59, `random2`, the cluster sets 5, 7 and 11, the layered set and the forests) were answered that way, in chunks of 2,000, twelve at a time.
Recorded per diagram:

- what the stages before node placement find: trees, hierarchies, clusters, sequences;
- after the tree stage: the cards and connections left in the graph, in its order, and the trees
  taken out, per card they hang on, each as [card, from, to, children...];
- after the hierarchy stage: every card's level in its hierarchy, or -1, and the places the
  stage gave the cards of a hierarchy;
- after the cluster stage: the clusters in the order they were made (the number of the vessel,
  the cards, row or column, the padding, the vessel's size) and every card's size;
- the graph as node placement meets it: the order of the cards (a vessel by its number), their
  sizes (cards with many connections have grown), the order of the connections;
- the cell size and the places after the node placement stage, and the clusters and every
  card's size again: a cluster may have turned;
- the graph as the later stages meet it (the trees are back in it): the cards and vessels and
  the connections in its order, its top-level list, and per card its own list of connections;
- for the parts to come: every later stage before routing that moves a card, with the places
  after it, and the places at the very end.

200 diagrams of 1 to 12 cards, three in four with cards of 220 by 120: a tree, a chain, several
groups, loose cards with few connections, or dense; two connections in three with a label;
seeds 1 to 3. 208 more were found with the probe (thirteen lone cards, and one or two per
deliberate break the 200 did not catch; seven of them for laying trees out, of up to 28 cards,
eighteen for hierarchies, of up to 36, 26 for clusters, of up to 40, eleven for the swap and
transpose stages, seven for the stage that lines cards up, four for the gap stage, fifteen for the stage
that puts clusters right, nine for the balance and equidistance stages, thirteen for the
packing stage, four for the first routing stage, and 36 for the routing stages: cards of
other groups in the way, the tie between two rule routes, a hook in a route, the stage that
swaps ports, a label that decides a nudge, a card that `Dejitter` moves, a group the second `BinPack` moves and
the chain of the stages) or
written by hand
where no generator made the case: one card, a row and one card with an arrow from the first
straight to the last; twenty cards on three levels; a hierarchy of cards with odd sizes; and
36 small diagrams for clusters (cards far higher or wider than the others, cards a pixel
apart in size, square cards, padding above 20, three labels between one card and a card of a
cluster, a card with many labels beside a cluster, two cards that point at the same two).

The tree stage, the hierarchy stage (levels and places) and the cluster stage are replayed for
all of them. 203 of the diagrams have no tree, hierarchy, cluster or sequence: for those the
replay checks the node placement stage, from the order the port's tree stage leaves. 73 more
have trees and nothing else: for those it checks the places of all cards, the trees laid out.
51 have a hierarchy and no cluster, 105 have a cluster: for those it checks the places after
node placement too. For all of them it checks the graph node placement leaves, and the places
after each of the twelve stages that follow it (`moved`: a stage that moved a card, with its name, its number in TALA's plan and the places after it; the stage that lines cards up runs four times under one name). The recorder also checks that the top-level list and every card's own
list of connections are in the order of the cards and connections.

Beyond the fixture: with the probe the port equals TALA in the tree, hierarchy and cluster
stages, after the node placement stage and after the twelve stages that follow it on 9,000
diagrams (3,000 random ones of up to 14
cards, 3,000 forests of up to 30, 3,000 layered ones of up to 40; 2,122 have a cluster).

## simplify.tala.json

1,200 made-up layouts for `SimplifyEdgeRoutes` (`docs/recorders/simplify_test.go.txt`, in
TALA's `internal/routing`, `SIMPLIFY_REFERENCE_OUT`, `TestExportSimplifyReference`): 2 to 6
cards, 1 to 4 connections, and per connection a route before the stage and after it (2,372
routes; 1,059 lose a hook, 285 more than one). The routes are not the router's: it makes a
hook in one of 44,000 diagrams. A route starts at a port of its first card and walks in
upright and level pieces of 10 to 300; half of the routes have a hook built in (across, on
in the direction of the piece before, back across); in two of five the second card is moved
to where the route ends. `tests/simplify_reference.rs` replays them. A found file
(`REFERENCE_FOUND`) can add cases; none was needed.

## nudge.tala.json

403 layouts for `NudgeEdgeChannels` and for `quality.Inspect`
(`docs/recorders/nudge_test.go.txt`, in TALA's `internal/routing`, `NUDGE_REFERENCE_OUT`,
`TestExportNudgeReference`, with `REFERENCE_FOUND=docs/recorders/found_layouts_nudge.json`;
it needs the `ovg`, `search`, `steps`, `labels` and `top_level` recorders beside it, the last
as `package routing`). The layouts are those of the search recorder (scattered cards, rows,
hubs), routed and labelled by TALA's `RouteEdges`. Recorded per layout: the routes and the
label positions before the stage, the routes after it, and the six numbers `quality.Inspect`
counts before and after. The stage changes a route in one such layout of 50, so the recorder
keeps every layout it changes and as many it leaves alone: 200 and 200, and 3 more found
with the probe. `tests/nudge_reference.rs` replays them.

For more layouts: `NUDGE_SEED=11 NUDGE_COUNT=3000 NUDGE_REFERENCE_OUT=<file> go test
./d2layouts/d2talalayout/internal/routing -run 'TestExportNudgeReference$' -count=1 -timeout
60m` (four minutes), then `cargo run --release --example probe_nudge -- compare <file> 5`.

## dejitter.tala.json

409 layouts for `Dejitter` (`docs/recorders/dejitter_test.go.txt`, in TALA's
`internal/engine`, `DEJITTER_REFERENCE_OUT`, `TestExportDejitterReference`, with
`REFERENCE_FOUND=docs/recorders/found_layouts_dejitter.json`). Cards in two to four columns
and one to three rows, most of them up to 40 out of line, connected to their neighbours and
by a few more connections, routed by TALA's `RouteGraph`. Recorded: the routes before the
stage, whether it says a card moved, and the cards and routes after it. The stage moves a
card in one such layout of seven, so the recorder keeps every layout in which it does and as
many in which it does not: 200 and 200, and 9 more found with the probe.
`tests/dejitter_reference.rs` replays them.

For more layouts: `DEJITTER_SEED=5 DEJITTER_COUNT=20000 DEJITTER_REFERENCE_OUT=<file> go
test ./d2layouts/d2talalayout/internal/engine -run 'TestExportDejitterReference$' -count=1`
(a minute), then `cargo run --release --example probe_dejitter -- compare <file> 5`.

## open_layouts.json

Not a fixture: `docs/recorders/open_layouts.json` holds diagrams on which the port is known
to differ from TALA and nothing is fixed yet, each with a line that says what differs. Take
a diagram out when its difference is gone and it is in a found file. It is empty now
(2026-11-24): the last two went into `found_layouts_stages.json`.
