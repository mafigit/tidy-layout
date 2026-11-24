"""Writes tests/fixtures/routed.tala.json from a recording of TALA's stages.

    python3 docs/recorders/routed_fixture.py <recording> tests/fixtures/routed.tala.json

The recording is what `TestExportStagesReference` writes with `STAGES_ROUTED=1`
(stages_test.go.txt). Kept per diagram of plain cards with a connection: the cards (x, y,
width, height), the number each card has in the diagram (`ids`) and the connections (from, to, label width, label height) in the order TALA's
graph has them when routing starts, at the places after `Rescale`, and the routes after the
first `EdgeRouting` (`routed`), after `BalanceEdgeSegments` (`balanced`) and after the last
stage before `Normalize` (`end`), and the top-left corner of every label's box right after
`PlaceLabels` (`labels`; 0, 0 for a connection without a label) and after the last stage
before `Normalize` (`labelsEnd`), and what TALA's `quality.Inspect` counts there (`metrics`:
cards on each other, routes through cards, covered labels, crossings, detour, route length).
`dejittered` is where the cards are after `Dejitter`, if it moves one, `packed` where they
are after the second `BinPack`, if it moves one.
`final` is the drawing after `Normalize`: the places of the cards, the routes and the label
places. `score` is TALA's score of that drawing (`quality.EvaluateWithArea`: the penalty and
the area); `seedScores` the same for seeds 1, 2 and 3 where the recording has them
(`STAGES_SEEDS=1`). The last six are left out
where TALA moves a card after the first routing (`Dejitter`, the second `BinPack`): those
stages are not ported. `input` is the diagram as the recorder was given it, so that a diagram a probe
finds can be added to found_layouts_stages.json.

With a third argument `trees` the file gets the diagrams with trees instead (and no
hierarchy or cluster): tests/fixtures/routed_trees.tala.json. Each has three more lists, as
TALA has them when routing starts: `own`, every card's own list of connections; `trees`, per
card of a tree the connection to the card it hangs on and the side of that card it lies on
(null for the other cards); `treeLinks`, the connections TALA takes for tree connections;
`presets`, where the tree stage wants the label of a connection (the connection, the
position, and how far along the route, from 0 to 1).

With `clusters` it gets the diagrams with a cluster (and no tree, hierarchy or sequence):
tests/fixtures/routed_clusters.tala.json. Each has two more lists: `own`, every card's own
list of connections; `clusters`, per cluster in TALA's order its cards (in its order), its
arrangement and the arrangement the cards around it asked for ("Row" or "Column").

With `several` it gets the diagrams with two or three of these kinds at once (and no
sequence), with the lists of each kind they have: tests/fixtures/routed_several.tala.json.

With `hierarchies` it gets the diagrams with a hierarchy (and no tree or cluster):
tests/fixtures/routed_hierarchies.tala.json. Each has one more list: `levels`, per card of a
hierarchy the number of its hierarchy and its level in it (null for the other cards).
"""
import json
import sys

recording, out = sys.argv[1], sys.argv[2]
kind = sys.argv[3] if len(sys.argv) > 3 else None
several = kind == "several"
trees, hierarchies, clusters = kind == "trees", kind == "hierarchies", kind == "clusters"
cases = []
for number, case in enumerate(json.load(open(recording))):
    kinds = int(case["trees"]) + int(case["hierarchies"]) + int(case["clusters"])
    if several:
        if not case["edges"] or kinds < 2 or case["sequences"]:
            continue
    elif not case["edges"] or case["trees"] != trees or case["hierarchies"] != hierarchies or case["clusters"] != clusters or case["sequences"]:
        continue
    at = [stage["at"] for stage in case["moved"] if stage["stage"] == "Rescale"][-1]
    nodes, edges = case["nodeOrderRouted"], case["edgeOrderRouted"]
    place = {card: index for index, card in enumerate(nodes)}
    # The first `EdgeRouting` is stage 21 of the pipeline, `BalanceEdgeSegments` 28, `Normalize` 36.
    def after(index):
        return [stage["routes"] for stage in case["routed"] if stage["index"] <= index and "routes" in stage][-1]
    routes = after(21)
    # Two stages move cards after the first routing: `Dejitter` and the second `BinPack`.
    moved = False
    dejittered = [stage["at"] for stage in case["routed"] if "at" in stage and stage["stage"] == "Dejitter"]
    packed = [stage["at"] for stage in case["routed"] if "at" in stage and stage["stage"] == "BinPack"]
    sizes = case["placedSizes"]
    cases.append({
        "case": number,
        "seed": case["seed"],
        "cards": [[at[card][0], at[card][1], sizes[card][0], sizes[card][1]] for card in nodes],
        "ids": nodes,
        "links": [[place[case["edges"][edge][0]], place[case["edges"][edge][1]], case["edges"][edge][2], case["edges"][edge][3]] for edge in edges],
        "routed": [routes[edge] for edge in edges],
        "dejittered": [dejittered[0][card] for card in nodes] if dejittered else None,
        "packed": [packed[0][card] for card in nodes] if packed else None,
        "balanced": None if moved else [after(28)[edge] for edge in edges],
        "end": None if moved else [after(35)[edge] for edge in edges],
        "labels": None if moved else [case["labelsPlaced"][edge] for edge in edges],
        "labelsEnd": None if moved else [case["labelsEnd"][edge] for edge in edges],
        "metrics": None if moved else case["metricsEnd"],
        "final": None if moved else {"cards": [case["final"][card] for card in nodes], "routes": [after(36)[edge] for edge in edges], "labels": [case["labels"][edge] for edge in edges]},
        "input": {"name": "found", "nodes": case["nodes"], "edges": case["edges"], "seed": case["seed"]},
        "score": case.get("score"),
        "seedScores": case.get("seedScores"),
    })
    if trees or (several and case["trees"]):
        link = {edge: index for index, edge in enumerate(edges)}
        hangs = {card: [link[edge], side] for card, edge, side in case["treesRouted"]}
        cases[-1]["own"] = [[link[edge] for edge in own] for own in case["ownEdgesRouted"]]
        cases[-1]["trees"] = [hangs.get(card) for card in nodes]
        cases[-1]["treeLinks"] = [link[edge] for edge in case["treeEdgesRouted"]]
        cases[-1]["presets"] = [[link[edge], position, share] for edge, position, share in case.get("labelPresetsRouted", [])]
    if hierarchies or (several and case["hierarchies"]):
        level = {card: [group, level] for card, group, level in case["hierarchiesRouted"]}
        cases[-1]["levels"] = [level.get(card) for card in nodes]
    if clusters or (several and case["clusters"]):
        link = {edge: index for index, edge in enumerate(edges)}
        cases[-1]["own"] = [[link[edge] for edge in own] for own in case["ownEdgesRouted"]]
        cases[-1]["clusters"] = [[[place[card] for card in cluster["nodes"]], cluster["arrangement"], cluster["desired"]] for cluster in case["clustersRouted"]]
json.dump(cases, open(out, "w"), separators=(",", ":"))
print(len(cases), "diagrams,", sum(len(case["links"]) for case in cases), "connections")
