#!/usr/bin/env python3
"""Deliberate breaks of src/trees.rs: each must make tests/stages_reference.rs fail.

Usage: tree_breaks.py <scratch dir> [probe-tala.json]
Works on a copy of the crate in the scratch dir. Checks the unchanged copy first. A break the
tests do not catch is tried against the probe file, if given.
Results: <scratch dir>/tree_breaks.jsonl, one line per break.
ONLY=<part of a name> runs the breaks with that name only.

A broken search may never end and take all memory (125 GB once). So a built program is run
itself, never through cargo, and it stops by three means that do not need this script alive:
- its own allocator ends it above 1 GB (MEMORY_CAP, added to the copy's test and probe);
- the kernel ends it after 400 s of processor time (RLIMIT_CPU);
- this script ends it after 150 s (the probe after 300 s), above 2 GB, and when the script
  itself is stopped.
"""
import atexit
import json
import os
import resource
import shutil
import signal
import subprocess
import sys
import tempfile
import time

CRATE = os.path.abspath(os.path.join(os.path.dirname(__file__), "..", ".."))
SCRATCH = sys.argv[1]
PROBE = sys.argv[2] if len(sys.argv) > 2 else None
COPY = os.path.join(SCRATCH, "breakcopy")
ENV = dict(os.environ, CARGO_TARGET_DIR=os.path.join(SCRATCH, "breaktarget"))
ONLY = os.environ.get("ONLY")
# The test file a break must make fail (TEST=routed_reference for the routing stages).
TEST = os.environ.get("TEST", "stages_reference")
PROBE_EXAMPLE = os.environ.get("PROBE_EXAMPLE", "probe_stages")
# What the probe is asked to do (PROBE_MODE=whole for probe_routed's whole pipeline).
PROBE_MODE = os.environ.get("PROBE_MODE", "compare")
NUMBERS = [int(n) for n in os.environ.get("NUMBERS", "").split(",") if n]
LIMIT_KB = 2 * 1024 * 1024
CPU_SECONDS = 400

MEMORY_CAP = """
mod memory_cap {
    use std::alloc::{GlobalAlloc, Layout, System};
    use std::sync::atomic::{AtomicUsize, Ordering};

    struct Capped;
    static HELD: AtomicUsize = AtomicUsize::new(0);
    const MOST: usize = 1 << 30;

    unsafe impl GlobalAlloc for Capped {
        unsafe fn alloc(&self, layout: Layout) -> *mut u8 {
            if HELD.fetch_add(layout.size(), Ordering::Relaxed) + layout.size() > MOST {
                std::process::abort();
            }
            unsafe { System.alloc(layout) }
        }
        unsafe fn dealloc(&self, pointer: *mut u8, layout: Layout) {
            HELD.fetch_sub(layout.size(), Ordering::Relaxed);
            unsafe { System.dealloc(pointer, layout) }
        }
    }

    #[global_allocator]
    static ALLOCATOR: Capped = Capped;
}
"""

# (name, old, new). `old` must occur exactly once in src/trees.rs.
# The last ones ("no answer") cannot change an answer: docs/placement-port.md says why.
BREAKS = [
    ("direction: swapped", "{ Direction::Inwards } else { Direction::Outwards }", "{ Direction::Outwards } else { Direction::Inwards }"),
    ("dominant: always the root", "self.direction(self.parts[root].children.first().copied().unwrap_or(root))", "self.direction(root)"),
    ("add_child: stays with old parent", "            self.parts[old].children.retain(|other| *other != child);\n", ""),
    ("add_child: in front", "self.parts[parent].children.push(child);", "self.parts[parent].children.insert(0, child);"),
    ("end_of_line: a leaf is no end", "[] => return Some(current),", "[] => return None,"),
    ("end_of_line: a fork is an end", "                _ => return None,", "                _ => return Some(current),"),
    ("end_of_line: stops at second card", "[only] => current = only,", "[only] => return Some(only),"),
    ("reverse: connections not shifted", "            self.parts[current].edge = self.parts[next].edge;\n", ""),
    ("reverse: old child kept", "            self.parts[current].children.retain(|other| *other != next);\n", ""),
    ("reverse: not turned", "self.add_child(pair[1], pair[0]);", "self.add_child(pair[0], pair[1]);"),
    ("remove: card stays in the list", "        self.nodes.retain(|other| *other != node);\n        for end in", "        for end in"),
    ("remove: connection stays at its target", "for end in [self.graph.edges[edge].from, self.graph.edges[edge].to] {", "for end in [self.graph.edges[edge].from] {"),
    ("remove: connection stays at its source", "for end in [self.graph.edges[edge].from, self.graph.edges[edge].to] {", "for end in [self.graph.edges[edge].to] {"),
    ("remove: connection stays in the list", "        self.edges.retain(|other| *other != edge);\n", ""),
    ("reconnect: card in front", "self.nodes.push(self.parts[current].node);", "self.nodes.insert(0, self.parts[current].node);"),
    ("reconnect: connection in front", 'self.edges.push(self.parts[current].edge.expect("a card of a tree has its connection"));', 'self.edges.insert(0, self.parts[current].edge.expect("a card of a tree has its connection"));'),
    ("reconnect: card without its connection", '            self.edges.push(self.parts[current].edge.expect("a card of a tree has its connection"));\n', ""),
    ("reconnect: only the root", "next = self.parts[current].children.first().copied();", "next = None;"),
    ("tree: children reversed", "children: children.iter().map(|child| self.tree(*child)).collect(),", "children: children.iter().rev().map(|child| self.tree(*child)).collect(),"),
    ("candidates: any outwards", "if outwards.len() == 1 {", "if !outwards.is_empty() {"),
    ("candidates: any inwards", "if inwards.len() == 1 {", "if !inwards.is_empty() {"),
    ("candidates: no outwards", "if outwards.len() == 1 {", "if false {"),
    ("candidates: no inwards", "if inwards.len() == 1 {", "if false {"),
    ("candidates: inwards first", "    candidates\n}", "    candidates.reverse();\n    candidates\n}"),
    ("merge: also with other connections", "if !(search.edges_of[sentinel].is_empty() && roots.len() > 1) {", "if !(roots.len() > 1) {"),
    ("merge: also one tree", "if !(search.edges_of[sentinel].is_empty() && roots.len() > 1) {", "if !search.edges_of[sentinel].is_empty() {"),
    ("merge: never", "if !(search.edges_of[sentinel].is_empty() && roots.len() > 1) {", "if true {"),
    ("merge: gives up at a candidate that branches", "        let Some(new_root) = search.end_of_line(candidate) else {\n            continue;", "        let Some(new_root) = search.end_of_line(candidate) else {\n            break;"),
    ("merge: all trees under the joint", "            if other != new_root {\n                search.add_child(joint, other);\n            }", "            search.add_child(joint, other);"),
    ("merge: chain not turned", "        search.reverse_chain(candidate);\n", ""),
    ("merge: joint under the new root", "search.add_child(candidate, joint);", "search.add_child(new_root, joint);"),
    ("merge: connection read after the turn", "        let edge_to_sentinel = search.parts[candidate].edge;\n        search.reverse_chain(candidate);", "        search.reverse_chain(candidate);\n        let edge_to_sentinel = search.parts[candidate].edge;"),
    ("merge: old card stays in the list", "        search.nodes.retain(|other| *other != sentinel);\n", ""),
    ("merge: new card in front", "search.nodes.push(new_sentinel);", "search.nodes.insert(0, new_sentinel);"),
    ("merge: new card not in the list", "        search.nodes.push(new_sentinel);\n", ""),
    ("merge: hangs on the old card", "return (new_sentinel, vec![tree_root]);", "return (sentinel, vec![tree_root]);"),
    ("merge: last candidate first", "for candidate in candidate_roots(search, &roots) {", "for candidate in candidate_roots(search, &roots).into_iter().rev() {"),
    ("pair: also a terminal partner", "if search.edges_of[sentinel].len() == 1 && !terminal[sentinel] {", "if search.edges_of[sentinel].len() == 1 {"),
    ("pair: the other one stays", "            if search.edges_of[sentinel].len() == 1 && !terminal[sentinel] {\n                terminal[node] = true;", "            if search.edges_of[sentinel].len() == 1 && !terminal[sentinel] {\n                terminal[sentinel] = true;"),
    ("pair: none stays", "            if search.edges_of[sentinel].len() == 1 && !terminal[sentinel] {\n                terminal[node] = true;\n                continue;\n            }\n", ""),
    ("round: root in front", "roots_at[sentinel].push(node);", "roots_at[sentinel].insert(0, node);"),
    ("arrows: one side only", "arrows_at[sentinel][usize::from(graph.edges[edge].to == sentinel)] = true;", "arrows_at[sentinel][0] = true;"),
    ("arrows: not noted", "            arrows_at[sentinel][usize::from(graph.edges[edge].to == sentinel)] = true;\n", ""),
    ("branches: terminal ones too", "let branches: Vec<usize> = roots_at[node].iter().copied().filter(|root| !terminal[*root]).collect();", "let branches: Vec<usize> = roots_at[node].clone();"),
    ("no answer: arrows asked of a card without branches", "            if branches.is_empty() {\n                continue;", "            if branches.is_empty() && arrows_at[node] != [true, true] {\n                continue;"),
    ("terminal: only by the branches left", "if arrows_at[node] == [true, true] {", "if branches.iter().any(|root| search.direction(*root) != search.direction(branches[0])) {"),
    ("terminal: never", "if arrows_at[node] == [true, true] {", "if false {"),
    ("terminal: one arrow is enough", "if arrows_at[node] == [true, true] {", "if arrows_at[node] != [false, false] {"),
    ("children: last first", "            for branch in branches {", "            for branch in branches.into_iter().rev() {"),
    ("goes on: always", "            if search.direction(node) != search.direction(branches[0]) {\n                terminal[sentinel] = true;\n            }\n", ""),
    ("goes on: turned round", "if search.direction(node) != search.direction(branches[0]) {", "if search.direction(node) == search.direction(branches[0]) {"),
    ("goes on: the card itself stays", "            if search.direction(node) != search.direction(branches[0]) {\n                terminal[sentinel] = true;", "            if search.direction(node) != search.direction(branches[0]) {\n                terminal[node] = true;"),
    ("taken off at once", "            search.parts[node].edge = Some(edge);\n", "            search.parts[node].edge = Some(edge);\n            search.remove(node, edge);\n"),
    ("hang on: terminal roots too", "let roots: Vec<usize> = roots_at[node].iter().copied().filter(|root| !terminal[*root]).collect();", "let roots: Vec<usize> = roots_at[node].clone();"),
    ("hang on: last card first", "    let mut trees: Vec<(usize, Vec<usize>)> = Vec::new();\n    for node in search.nodes.clone() {", "    let mut trees: Vec<(usize, Vec<usize>)> = Vec::new();\n    for node in search.nodes.clone().into_iter().rev() {"),
    ("put back: last card first", "    let mut kept: Vec<(usize, Vec<usize>)> = Vec::new();\n    for node in search.nodes.clone() {", "    let mut kept: Vec<(usize, Vec<usize>)> = Vec::new();\n    for node in search.nodes.clone().into_iter().rev() {"),
    ("put back: by number, not by order", "    let mut kept: Vec<(usize, Vec<usize>)> = Vec::new();\n    for node in search.nodes.clone() {", "    let mut kept: Vec<(usize, Vec<usize>)> = Vec::new();\n    for node in 0..count {"),
    ("put back: no card is alone", "if search.edges_of[node].is_empty() && roots.len() > 1 {", "if false {"),
    ("put back: every card is alone", "if search.edges_of[node].is_empty() && roots.len() > 1 {", "if roots.len() > 1 {"),
    ("put back: one chain on a lone card stays out", "if search.edges_of[node].is_empty() && roots.len() > 1 {", "if search.edges_of[node].is_empty() {"),
    ("put back: chains stay out", "            if search.branches(root) {\n                branching.push(root);", "            if true {\n                branching.push(root);"),
    ("put back: everything goes back", "            if search.branches(root) {\n                branching.push(root);", "            if false {\n                branching.push(root);"),
    ("put back: empty entries kept", "        if !branching.is_empty() {\n            kept.push((node, branching));\n        }", "        kept.push((node, branching));"),
    ("without: cards keep no number", '    Graph { ids: found.order.iter().map(|card| graph.id(*card)).collect(), trees_at, ..Graph::new(nodes, edges) }', '    Graph { trees_at, ..Graph::new(nodes, edges) }'),
    ("without: connections in the given order", "    let edges = found\n        .edges\n        .iter()\n        .map(|edge| graph.edges[*edge])", "    let mut sorted = found.edges.clone();\n    sorted.sort();\n    let edges = sorted\n        .iter()\n        .map(|edge| graph.edges[*edge])"),
    ("without: cards in the given order", "let nodes = found.order.iter().map(|card| graph.nodes[*card].clone()).collect();", "let nodes = (0..found.order.len()).map(|card| graph.nodes[card].clone()).collect();"),
    ("without: ends swapped", "Edge { from: place_of(edge.from), to: place_of(edge.to), ..edge }", "Edge { from: place_of(edge.to), to: place_of(edge.from), ..edge }"),
    ("no answer: last child's direction", "self.parts[root].children.first().copied().unwrap_or(root)", "self.parts[root].children.last().copied().unwrap_or(root)"),
    ("no answer: the chain's root left out of the joint", "if other != new_root {", "if other != candidate {"),
    ("no answer: a branch's direction from the card under its root", "if search.direction(node) != search.direction(branches[0]) {", "if search.direction(node) != search.dominant_direction(branches[0]) {"),
]

LAYOUT = [('src/tree_layout.rs', 'spacing: siblings closer', 'const SIBLING_SPACING: f64 = 50.0;', 'const SIBLING_SPACING: f64 = 40.0;'),
    ('src/tree_layout.rs', 'spacing: levels closer', 'const TREE_PARENT_SPACING: f64 = 100.0;', 'const TREE_PARENT_SPACING: f64 = 90.0;'),
    ('src/tree_layout.rs', 'spacing: less room for arrowheads', 'const MIN_ARROWHEAD_CLEARANCE: f64 = 20.0;', 'const MIN_ARROWHEAD_CLEARANCE: f64 = 10.0;'),
    ('src/tree_layout.rs', 'sides: top first', '[Orientation::Bottom, Orientation::Top, Orientation::Left, Orientation::Right];', '[Orientation::Top, Orientation::Bottom, Orientation::Left, Orientation::Right];'),
    ('src/tree_layout.rs', 'sides: right before left', '[Orientation::Bottom, Orientation::Top, Orientation::Left, Orientation::Right];', '[Orientation::Bottom, Orientation::Top, Orientation::Right, Orientation::Left];'),
    ('src/tree_layout.rs', 'shift: only the card itself', '        for part in self.subtree(part) {\n            let at', '        for part in [part] {\n            let at'),
    ('src/tree_layout.rs', 'swap: sizes stay', '            (card.width, card.height) = (card.height, card.width);\n', ''),
    ('src/tree_layout.rs', 'swap: places stay', 'self.move_to(part, at.y, at.x);', 'self.move_to(part, at.x, at.y);'),
    ('src/tree_layout.rs', 'flip: about the near corner', '(-self.card(part).right(), -self.card(part).bottom())', '(-self.card(part).top_left.x, -self.card(part).top_left.y)'),
    ('src/tree_layout.rs', 'invert: top like bottom', 'Orientation::Top => self.flip(),', 'Orientation::Top => {}'),
    ('src/tree_layout.rs', 'invert: right like bottom', 'Orientation::Right => self.swap_dimensions(),', 'Orientation::Right => {}'),
    ('src/tree_layout.rs', 'invert: left like right', '                self.swap_dimensions();\n                self.flip();', '                self.swap_dimensions();'),
    ('src/tree_layout.rs', 'label: height for a tree to the side', 'if sideways { width } else { height }', 'if sideways { height } else { width }'),
    ('src/tree_layout.rs', "label: one arrowhead's room", 'largest + 2.0 * MIN_ARROWHEAD_CLEARANCE', 'largest + MIN_ARROWHEAD_CLEARANCE'),
    ('src/tree_layout.rs', 'label: an only child like one of several', 'if children.len() == 1 {', 'if false {'),
    ('src/tree_layout.rs', 'label: an only child with less than the least room', 'return TREE_PARENT_SPACING.max(with_arrowheads);', 'return with_arrowheads;'),
    ('src/tree_layout.rs', 'label: several children like an only one', 'TREE_PARENT_SPACING / 2.0 + (TREE_PARENT_SPACING / 2.0).max(with_arrowheads)', 'TREE_PARENT_SPACING.max(with_arrowheads)'),
    ('src/tree_layout.rs', 'room: from left edge to left edge', 'self.card(children[child]).top_left.x - self.card(children[child - 1]).right()', 'self.card(children[child]).top_left.x - self.card(children[child - 1]).top_left.x'),
    ('src/tree_layout.rs', 'room before: first level only', 'lefts.iter().zip(&rights).map(room)', 'lefts.iter().zip(&rights).take(1).map(room)'),
    ('src/tree_layout.rs', 'room before: the most', 'lefts.iter().zip(&rights).map(room).fold(f64::INFINITY, f64::min)', 'lefts.iter().zip(&rights).map(room).fold(f64::NEG_INFINITY, f64::max)'),
    ('src/tree_layout.rs', 'no answer: even: no least room', 'fold(SIBLING_SPACING, f64::max);', 'fold(0.0, f64::max);'),
    ('src/tree_layout.rs', 'even: a child that need not move is asked too', '            if more == 0.0 {\n                continue;\n            }\n', ''),
    ('src/tree_layout.rs', 'even: gives up sooner', '> 10.0 * SIBLING_SPACING', '> 5.0 * SIBLING_SPACING'),
    ('src/tree_layout.rs', 'even: never gives up', '> 10.0 * SIBLING_SPACING {\n                return;', '> 10.0 * SIBLING_SPACING {\n                continue;'),
    ('src/tree_layout.rs', 'even: next child keeps its room', '                rooms[child + 1] -= more;\n', '                let _ = more;\n'),
    ('src/tree_layout.rs', 'even: nothing moves', '                self.shift(self.parts[part].children[child], more, 0.0);\n', ''),
    ('src/tree_layout.rs', 'center: of the left edges', 'self.card(*child).top_left.x + self.card(*child).width / 2.0', 'self.card(*child).top_left.x'),
    ('src/tree_layout.rs', 'level: as high as its lowest card', 'map(|part| self.card(*part).height).fold(0.0, f64::max)', 'map(|part| self.card(*part).height).fold(f64::INFINITY, f64::min)'),
    ('src/tree_layout.rs', 'level: room of the last card', 'spacings[level + 1] = spacings[level + 1].max(self.spacing_to_children(part));', 'spacings[level + 1] = self.spacing_to_children(part);'),
    ('src/tree_layout.rs', 'level: cards at its top', 'let centered = ((heights[level] - self.card(part).height) / 2.0).round();', 'let centered = 0.0;'),
    ('src/tree_layout.rs', 'level: next one starts at its top', 'level_bottom = level_top + heights[level];', 'level_bottom = level_top;'),
    ('src/tree_layout.rs', 'across: top level first', 'for level in (1..levels.len()).rev() {', 'for level in 1..levels.len() {'),
    ('src/tree_layout.rs', 'across: a leaf stays', '                    self.move_to(part, next_position, self.card(part).top_left.y);\n', '                    let _ = next_position;\n'),
    ('src/tree_layout.rs', 'across: first child taken for the last', 'below.iter().rposition(', 'below.iter().position('),
    ('src/tree_layout.rs', 'across: the last child counts as later', 'below[(last_child + 1).min(below.len())..]', 'below[last_child.min(below.len())..]'),
    ('src/tree_layout.rs', 'across: later cards stay when children spread', '                            self.shift(other, grown, 0.0);\n', '                            let _ = other;\n'),
    ('src/tree_layout.rs', 'across: parent not on whole pixels', '(self.children_center(part) - self.card(part).width / 2.0).round();', '(self.children_center(part) - self.card(part).width / 2.0);'),
    ('src/tree_layout.rs', 'across: a card may stand left of its place', 'if short > 0.0 {', 'if false {'),
    ('src/tree_layout.rs', 'across: later cards stay when a subtree moves', '                            self.shift(other, short, 0.0);\n', '                            let _ = other;\n'),
    ('src/tree_layout.rs', 'across: only the card moves to its place', '                        self.shift(part, short, 0.0);\n', '                        self.move_to(part, next_position, self.card(part).top_left.y);\n'),
    ('src/tree_layout.rs', 'across: no room after a card', 'next_position = self.card(part).right() + SIBLING_SPACING;', 'next_position = self.card(part).right();'),
    ('src/tree_layout.rs', 'align: not under the card', '            self.shift(child, offset, 0.0);\n        }\n    }\n\n    /// TALA `constructToOrientation`', '            let _ = (child, offset);\n        }\n    }\n\n    /// TALA `constructToOrientation`'),
    ('src/tree_layout.rs', 'align: not on whole pixels', '- self.children_center(0)).round();', '- self.children_center(0));'),
    ('src/tree_layout.rs', 'construct: not turned back', '        self.center_align_children();\n        self.invert();', '        self.center_align_children();'),
    ('src/tree_layout.rs', 'construct: not turned first', '        self.orientation = orientation;\n        self.invert();', '        self.orientation = orientation;'),
    ('src/tree_layout.rs', 'position: left by the right edge', 'Orientation::Left => card.top_left.x,', 'Orientation::Left => card.right(),'),
    ('src/tree_layout.rs', 'position: right by the left edge', 'Orientation::Right => card.right(),', 'Orientation::Right => card.top_left.x,'),
    ('src/tree_layout.rs', 'position: top by the bottom edge', 'Orientation::Top => card.top_left.y,', 'Orientation::Top => card.bottom(),'),
    ('src/tree_layout.rs', 'position: bottom by the top edge', '            _ => card.bottom(),\n        }\n    }\n\n    fn is_further_out', '            _ => card.top_left.y,\n        }\n    }\n\n    fn is_further_out'),
    ('src/tree_layout.rs', 'further out: turned round', '{ a < b } else { a > b }', '{ a > b } else { a < b }'),
    ('src/tree_layout.rs', 'no answer: move: not on whole pixels', 'let by = (to - from).round();', 'let by = to - from;'),
    ('src/tree_layout.rs', 'move: across instead of along', '{ (by, 0.0) } else { (0.0, by) }', '{ (0.0, by) } else { (by, 0.0) }'),
    ('src/tree_layout.rs', 'refused: never', '                if bad {\n                    return true;', '                if bad {\n                    return false;'),
    ('src/tree_layout.rs', 'refused: a known pair may come to lie on each other', 'Some((_, _, lay_on_each_other)) => !lay_on_each_other && self.overlap(a, b),', 'Some(_) => false,'),
    ('src/tree_layout.rs', 'refused: a known pair too', 'Some((_, _, lay_on_each_other)) => !lay_on_each_other && self.overlap(a, b),', 'Some(_) => self.too_close(a, b),'),
    ('src/tree_layout.rs', 'border: at the card it hangs on', '                border = position;\n', '                let _ = position;\n'),
    ('src/tree_layout.rs', 'place: any first stop is taken', 'let mut best_distance = (best_position - root_position).abs();', 'let mut best_distance = f64::INFINITY;'),
    ('src/tree_layout.rs', 'place: does not start at the border', '        self.move_tree(0.0, border);\n', ''),
    ('src/tree_layout.rs', 'stops: also beyond the card it hangs on', 'self.is_further_out(border, *position) && self.is_further_out(*position, root_position)', 'self.is_further_out(border, *position)'),
    ('src/tree_layout.rs', 'no answer: stops: also at the border and beyond', 'self.is_further_out(border, *position) && self.is_further_out(*position, root_position)', 'self.is_further_out(*position, root_position)'),
    ('src/tree_layout.rs', 'stops: not at the card it hangs on', '        stops.push(root_position);\n', '        stops.push(border);\n'),
    ('src/tree_layout.rs', 'no answer: stops: nearest first', '        stops.push(root_position);\n', '        stops.push(root_position);\n        stops.reverse();\n'),
    ('src/tree_layout.rs', 'known: nothing noted', 'pairs.filter(|(a, b)| self.too_close(*a, *b))', 'pairs.filter(|(a, b)| a == b)'),
    ('src/tree_layout.rs', 'no answer: place: an equal stop is taken', '&& distance < best_distance {', '&& distance <= best_distance {'),
    ('src/tree_layout.rs', 'place: not put back', '                    self.graph.nodes[part.node].top_left = at;\n', '                    let _ = (part, at);\n'),
    ('src/tree_layout.rs', 'ratio: width by height', 'if width >= height { width / height } else { height / width }', 'width / height'),
    ('src/tree_layout.rs', 'second tree: same side', '            opposite(*only)', '            *only'),
    ('src/tree_layout.rs', 'second tree: searched like the first', 'if let [only] = taken {', 'if let [only, _] = taken {'),
    ('src/tree_layout.rs', 'sides: a taken one may be taken again', 'let order = if isolated && !free.is_empty() { free } else { SIDES.to_vec() };', 'let order = SIDES.to_vec();'),
    ('src/tree_layout.rs', 'sides: also a card with other connections keeps to free ones', 'let order = if isolated && !free.is_empty() { free } else { SIDES.to_vec() };', 'let order = if !free.is_empty() { free } else { SIDES.to_vec() };'),
    ('src/tree_layout.rs', 'choice: no tie-break by the square', 'if distance < best_distance || (distance == best_distance && ratio < best_ratio) {', 'if distance < best_distance {'),
    ('src/tree_layout.rs', 'choice: the least square', 'distance == best_distance && ratio < best_ratio', 'distance == best_distance && ratio > best_ratio'),
    ('src/tree_layout.rs', 'choice: only the square', 'if distance < best_distance || (distance == best_distance && ratio < best_ratio) {', 'if ratio < best_ratio {'),
    ('src/tree_layout.rs', 'choice: stays on the last side tried', '        self.construct(best);\n        self.place_at_orientation();\n        self.position_labels(isolated);', '        self.position_labels(isolated);'),
    ('src/tree_layout.rs', 'choice: drawn but not moved in', '        self.construct(best);\n        self.place_at_orientation();\n        self.position_labels(isolated);', '        self.construct(best);\n        self.position_labels(isolated);'),
    ('src/tree_layout.rs', 'opposite: of left and right', '        Orientation::Left => Orientation::Right,\n        _ => Orientation::Left,', '        Orientation::Left => Orientation::Left,\n        _ => Orientation::Right,'),
    ('src/tree_layout.rs', 'opposite: of top and bottom', '        Orientation::Bottom => Orientation::Top,\n        Orientation::Top => Orientation::Bottom,', '        Orientation::Bottom => Orientation::Bottom,\n        Orientation::Top => Orientation::Top,'),
    ('src/tree_layout.rs', 'size: every tree is one card', '1 + tree.children.iter().map(size).sum::<usize>()', '1 + tree.children.len().min(0)'),
    ('src/tree_layout.rs', 'trees: all roots of a connected card together', 'trees.extend(roots.iter().map(|root| (sentinel, false, vec![root])));', 'trees.push((sentinel, false, roots.iter().collect()));'),
    ('src/tree_layout.rs', "trees: a lone card's count as not alone", '(sentinel, true, together)', '(sentinel, false, together)'),
    ('src/tree_layout.rs', "trees: a connected card's count as alone", '(sentinel, false, vec![root])', '(sentinel, true, vec![root])'),
    ('src/tree_layout.rs', 'trees: every root on its own', 'if graph.edges_of(sentinel).next().is_some() {', 'if true {'),
    ('src/tree_layout.rs', 'class: without the arrow at the card', '(points_at(whole, root.children.first().unwrap_or(root)), whole.edges[root.edge].to == *card)', '(points_at(whole, root.children.first().unwrap_or(root)), false)'),
    ('src/tree_layout.rs', 'class: without the direction below the root', '(points_at(whole, root.children.first().unwrap_or(root)), whole.edges[root.edge].to == *card)', '(false, whole.edges[root.edge].to == *card)'),
    ('src/tree_layout.rs', 'order: smallest first', '(std::cmp::Reverse(roots.iter().map(|root| size(root)).sum::<usize>()), whole.id(roots[0].node))', '(roots.iter().map(|root| size(root)).sum::<usize>(), whole.id(roots[0].node))'),
    ('src/tree_layout.rs', 'order: highest number first', 'whole.id(roots[0].node)));', 'std::cmp::Reverse(whole.id(roots[0].node))));'),
    ('src/tree_layout.rs', 'order: not sorted', '    trees.sort_by_key(|(_, _, roots)| (std::cmp::Reverse(roots.iter().map(|root| size(root)).sum::<usize>()), whole.id(roots[0].node)));\n', ''),
    ('src/tree_layout.rs', 'taken: sides not noted', '            taken[at].1.push(side);\n', '            let _ = side;\n'),
    ('src/tree_layout.rs', 'taken: one list for all cards', 'taken.iter().position(|(other, _)| *other == sentinel)', 'taken.iter().position(|_| true)'),
    ('src/tree_layout.rs', 'join: children last first', 'stack.extend(tree.children.iter().rev().map(|child| (part, child)));', 'stack.extend(tree.children.iter().map(|child| (part, child)));'),
    ('src/tree_layout.rs', 'join: roots last first', 'roots.iter().rev().map(|root| (0, *root)).collect();', 'roots.iter().map(|root| (0, *root)).collect();'),
    ('src/tree_layout.rs', 'join: no labels', 'label: (edge.min_width, edge.min_height), edge: graph.edges.len() - 1, parent: Some(parent)', 'label: (0.0, 0.0), edge: graph.edges.len() - 1, parent: Some(parent)'),
    ('src/tree_layout.rs', 'join: connections stay out', '            graph.edges.push(Edge { from: place_of(edge.from), to: place_of(edge.to), ..edge });\n', '            let _ = (place_of(edge.from), Edge { ..edge });\n'),
    ('src/place.rs', 'direct: arrows inside a tree count', 'graph.edges.iter().filter(|edge| edge.from < own || edge.to < own)', 'graph.edges.iter().filter(|edge| edge.from < own || edge.to < own || true)'),
    ('src/place.rs', "direct: also not around a diagram's only card", 'let own = if alone { part.nodes.len() } else { group.len() };', 'let own = group.len();'),
    ('src/place.rs', 'alone: stays where it was', '            part.nodes[0].top_left = Point { x: 0.0, y: 0.0 };\n', ''),
    ('src/place.rs', 'alone: packed too', '    if !alone {\n        combine(whole, &placed);\n    }', '    combine(whole, &placed);'),
    ('src/place.rs', 'alone: placed like a group', 'let alone = vesseled.nodes.len() == 1;', 'let alone = vesseled.nodes.is_empty();'),
    ('src/place.rs', 'turned: cards that trees hang on too', '    let trees_at = graph.trees_at.iter().filter_map(|card| place_of(*card)).collect();\n    let vessels', '    let trees_at: Vec<usize> = Vec::new();\n    let vessels'),
    ('src/place.rs', 'cell: of the cards as given', '    graph.cell_size = compute_cell_size(&graph.nodes);\n    whole.cell_size = graph.cell_size;\n', '    whole.cell_size = graph.cell_size;\n'),
    ('src/optimizer.rs', 'turned: guard gone', '    if graph.trees_at.contains(&node) {\n        return false;\n    }\n', ''),
    ('src/tree_layout.rs', 'no answer: too close asked one way round', 'overlaps_at(self.graph, a, b, self.graph.nodes[a].top_left) || overlaps_at(self.graph, b, a, self.graph.nodes[b].top_left)', 'overlaps_at(self.graph, a, b, self.graph.nodes[a].top_left)'),
    ('src/tree_layout.rs', 'no answer: two cards of the tree are not asked', 'if a == b || (moved.contains(&b) && b < a) {', 'if a == b || moved.contains(&b) {'),
    ('src/tree_layout.rs', 'no answer: a tree that did not move is asked too', 'if !(moved && self.refused(&known)) && distance < best_distance {', 'if !self.refused(&known) && distance < best_distance {'),
    ('src/tree_layout.rs', 'no answer: left as flip then swap', '                self.swap_dimensions();\n                self.flip();', '                self.flip();\n                self.swap_dimensions();')]

HIERARCHY = [
    ('src/hierarchy.rs', 'levels: two are enough', 'const MIN_HIERARCHY_LEVELS: usize = 3;', 'const MIN_HIERARCHY_LEVELS: usize = 2;'),
    ('src/hierarchy.rs', 'levels: four needed', 'const MIN_HIERARCHY_LEVELS: usize = 3;', 'const MIN_HIERARCHY_LEVELS: usize = 4;'),
    ('src/hierarchy.rs', 'workflow: never', 'let mut workflow = count > level_count &&', 'let mut workflow = count > usize::MAX - 1 &&'),
    ('src/hierarchy.rs', 'workflow: never fits when tall', 'const MAX_GRAPH_SIZE: f64 = 30_000.0;', 'const MAX_GRAPH_SIZE: f64 = 300.0;'),
    ('src/hierarchy.rs', 'rings: arrows at a card taken off stay in a ring', '            in_no_ring[index] = true;\n            let other = arrows[index].from;', '            let other = arrows[index].from;'),
    ('src/hierarchy.rs', 'rings: arrows from a card taken off stay in a ring', '            in_no_ring[index] = true;\n            let other = arrows[index].to;', '            let other = arrows[index].to;'),
    ('src/hierarchy.rs', 'rings: only cards that point at nothing are taken off', 'for sinks in [true, false] {', 'for sinks in [true, true] {'),
    ('src/hierarchy.rs', 'rings: only cards nothing points at are taken off', 'for sinks in [true, false] {', 'for sinks in [false, false] {'),
    ('src/hierarchy.rs', 'rings: most arrows in', '(out - into, out + into, std::cmp::Reverse(ids[*node]))', '(into - out, out + into, std::cmp::Reverse(ids[*node]))'),
    ('src/hierarchy.rs', 'rings: most arrows first', '(out - into, out + into, std::cmp::Reverse(ids[*node]))', '(out + into, out - into, std::cmp::Reverse(ids[*node]))'),
    ('src/hierarchy.rs', 'rings: number of arrows does not count', '(out - into, out + into, std::cmp::Reverse(ids[*node]))', '(out - into, 0, std::cmp::Reverse(ids[*node]))'),
    ('src/hierarchy.rs', 'rings: highest number', '(out - into, out + into, std::cmp::Reverse(ids[*node]))', '(out - into, out + into, std::cmp::Reverse(usize::MAX - ids[*node]))'),
    ('src/hierarchy.rs', 'rings: the card keeps the arrows that point at it', '            for index in std::mem::take(&mut incoming[node]) {\n                outgoing[arrows[index].from].retain(|arrow| *arrow != index);\n            }\n', ''),
    ('src/hierarchy.rs', 'dag: nothing turned round', '(arrow.from, arrow.to) = (arrow.to, arrow.from);', '(arrow.from, arrow.to) = (arrow.from, arrow.to);'),
    ('src/hierarchy.rs', 'dag: doubles weigh one', 'Some(kept) => kept.weight += arrow.weight,', 'Some(kept) => kept.weight += 0,'),
    ('src/hierarchy.rs', 'dag: doubles kept', 'match unique.iter_mut().find(|kept| (kept.from, kept.to) == (arrow.from, arrow.to)) {', 'match unique.iter_mut().find(|kept| (kept.from, kept.to) == (arrow.from, arrow.to) && kept.weight < 0) {'),
    ('src/hierarchy.rs', 'slack: an arrow may stay on its level', 'levels[self.arrows[arrow].to] - levels[self.arrows[arrow].from] - 1', 'levels[self.arrows[arrow].to] - levels[self.arrows[arrow].from]'),
    ('src/hierarchy.rs', 'first levels: the last arrow decides', 'levels[arrow.to] = levels[arrow.to].max(levels[node] + 1);', 'levels[arrow.to] = levels[node] + 1;'),
    ('src/hierarchy.rs', 'tree: any arrow joins', 'if in_tree[other] || self.slack(arrow, levels) != 0 {', 'if in_tree[other] {'),
    ('src/hierarchy.rs', 'tree: moves the same way whichever end is outside', 'let delta = if selected_tail_in_tree { least } else { -least };', 'let delta = least;'),
    ('src/hierarchy.rs', 'tree: the last arrow of least slack decides', 'in_tree[self.arrows[boundary[0]].from];', 'in_tree[self.arrows[boundary[boundary.len() - 1]].from];'),
    ('src/hierarchy.rs', 'tree: arrows the other way round join too', 'if (least != 0 && tail_in_tree != selected_tail_in_tree) || self.slack(arrow, levels) != 0 {', 'if self.slack(arrow, levels) != 0 {'),
    ('src/hierarchy.rs', 'tree: moves by the most slack', '.map(|arrow| self.slack(*arrow, levels)).min()', '.map(|arrow| self.slack(*arrow, levels)).max()'),
    ('src/hierarchy.rs', 'cut: sign turned', 'if self.arrows[arrow].from == node { balance[node] } else { -balance[node] }', 'if self.arrows[arrow].from == node { -balance[node] } else { balance[node] }'),
    ('src/hierarchy.rs', "cut: only the card's own arrows", '            balance[self.other(arrow, node)] += balance[node];\n', '            let _ = &mut balance;\n'),
    ('src/hierarchy.rs', 'cut: every arrow weighs one', '            balance[arrow.from] += arrow.weight;\n            balance[arrow.to] -= arrow.weight;', '            balance[arrow.from] += 1;\n            balance[arrow.to] -= 1;'),
    ('src/hierarchy.rs', 'better: never', 'find(|arrow| tree[*arrow] && cut[*arrow] < 0)', 'find(|arrow| tree[*arrow] && cut[*arrow] < i64::MIN + 1)'),
    ('src/hierarchy.rs', 'better: the last negative arrow leaves', 'let Some(leaving) = (0..self.arrows.len()).find(', 'let Some(leaving) = (0..self.arrows.len()).rfind('),
    ('src/hierarchy.rs', "better: the tail's side moves", '            head[self.arrows[leaving].to] = true;\n            let mut queue = vec![self.arrows[leaving].to];', '            head[self.arrows[leaving].from] = true;\n            let mut queue = vec![self.arrows[leaving].from];'),
    ('src/hierarchy.rs', "better: arrows into the head's side", '!tree[*arrow] && head[self.arrows[*arrow].from] && !head[self.arrows[*arrow].to]', '!tree[*arrow] && !head[self.arrows[*arrow].from] && head[self.arrows[*arrow].to]'),
    ('src/hierarchy.rs', 'better: the arrow of most slack joins', 'crossing.min_by_key(', 'crossing.max_by_key('),
    ('src/hierarchy.rs', 'better: the side moves up', '                    levels[node] += slack;\n', '                    levels[node] -= slack;\n'),
    ('src/hierarchy.rs', 'better: the joining arrow is not noted', '            tree[entering] = true;\n', '            let _ = entering;\n'),
    ('src/hierarchy.rs', 'better: the leaving arrow stays', '            tree[leaving] = false;\n', ''),
    ('src/hierarchy.rs', "rank: cards in the group's order", '    by_number.sort_by_key(|node| ids[*node]);\n', ''),
    ('src/hierarchy.rs', 'rank: first levels kept', '    ranking.optimize(&mut levels, &mut tree);\n', '    let _ = &mut tree;\n'),
    ('src/hierarchy.rs', 'rank: one level too few', '((0..ids.len()).map(of).collect(), (most - least) as usize + 1)', '((0..ids.len()).map(of).collect(), (most - least) as usize)'),
    ('src/hierarchy.rs', 'one-many-one: any first level', 'if on(0).count() != 1 || on(2).count() != 1 {', 'if on(2).count() != 1 {'),
    ('src/hierarchy.rs', 'one-many-one: any last level', 'if on(0).count() != 1 || on(2).count() != 1 {', 'if on(0).count() != 1 {'),
    ('src/hierarchy.rs', 'no answer: one-many-one: a connection in the row is fine', '                _ => return false,', '                _ => {}'),
    ('src/hierarchy.rs', 'no answer: one-many-one: every connection looked at', '            if above && below {\n                break;\n            }\n', ''),
    ('src/hierarchy.rs', 'one-many-one: one side is enough', 'if !above || !below {', 'if !above && !below {'),
    ('src/hierarchy.rs', 'one-many-one: the first card may reach further', 'others_of(source).all(|other| levels[other] == 1)', 'others_of(source).all(|_| true)'),
    ('src/hierarchy.rs', 'one-many-one: laid out as a hierarchy', ' && !(level_count == 3 && is_one_many_one(count, &edges, &levels));', ';'),
    ('src/hierarchy.rs', 'group: also one with a tree on a card', 'if group.len() <= 1 || group.iter().any(|card| graph.trees_at.contains(card)) {', 'if group.len() <= 1 {'),
    ('src/hierarchy.rs', 'group: also without a start or an end', 'if sources == 0 || sinks == 0 {', 'if sources == 0 && sinks == 0 {'),
    ('src/hierarchy.rs', 'shape: cards by levels, not by levels squared', 'count as f64 / (level_count * level_count) as f64', 'count as f64 / level_count as f64'),
    ('src/hierarchy.rs', 'shape: half as wide as high is too tall', 'aspect_ratio < 0.5', 'aspect_ratio <= 0.5'),
    ('src/hierarchy.rs', 'shape: twice as wide as high is too wide', 'aspect_ratio > 2.0', 'aspect_ratio >= 2.0'),
    ('src/hierarchy.rs', 'shape: never too tall', '(!too_tall || workflow)', 'true'),
    ('src/hierarchy.rs', 'shape: a tall workflow is refused', '(!too_tall || workflow)', '!too_tall'),
    ('src/hierarchy.rs', 'shape: never too wide', 'let valid = !too_wide && ', 'let valid = '),
    ('src/hierarchy.rs', 'flow: arrows up count', 'levels[*to] > levels[*from]', 'levels[*to] < levels[*from]'),
    ('src/hierarchy.rs', 'flow: as many down as up is enough', 'forward as f64 >= 1.5 * back as f64', 'forward as f64 >= 1.0 * back as f64'),
    ('src/hierarchy.rs', 'flow: exactly three in five is not enough', 'forward as f64 >= 1.5 * back as f64', 'forward as f64 > 1.5 * back as f64'),
    ('src/hierarchy.rs', 'flow: not asked', 'flows_one_way && !dense', '!dense'),
    ('src/hierarchy.rs', 'workflow: as many levels as cards', 'let mut workflow = count > level_count &&', 'let mut workflow = count >= level_count &&'),
    ('src/hierarchy.rs', 'workflow: several ends', 'sources == 1 && sinks == 1 && back == 0', 'sources == 1 && back == 0'),
    ('src/hierarchy.rs', 'workflow: several starts', 'sources == 1 && sinks == 1 && back == 0', 'sinks == 1 && back == 0'),
    ('src/hierarchy.rs', 'workflow: with arrows up', 'sources == 1 && sinks == 1 && back == 0', 'sources == 1 && sinks == 1'),
    ('src/hierarchy.rs', 'dense: not asked', 'flows_one_way && !dense', 'flows_one_way'),
    ('src/hierarchy.rs', 'dense: root rounded down', '(count as f64).sqrt().ceil() as usize', '(count as f64).sqrt().floor() as usize'),
    ('src/hierarchy.rs', 'dense: half the connections', 'let most_edges = 2 * ', 'let most_edges = '),
    ('src/trees.rs', 'sentinels: none noted', 'let trees_at = found.trees.iter().map(|(sentinel, _)| place_of(*sentinel)).collect();', 'let trees_at = Vec::new();'),
    ('src/place.rs', 'sentinels: lost in a group', 'let trees_at = graph.trees_at.iter().filter_map(|card| place_of(*card)).collect();', 'let trees_at = Vec::new();'),
]

HLAYOUT = [
    ('src/hierarchy_layout.rs', 'room: less per crossing', 'const CROSSING_SPACING: f64 = 50.0;', 'const CROSSING_SPACING: f64 = 40.0;'),
    ('src/hierarchy_layout.rs', 'room: levels at most 200 apart', 'const PARENT_SPACING: f64 = 300.0;', 'const PARENT_SPACING: f64 = 200.0;'),
    ('src/hierarchy_layout.rs', 'room: less beside a dummy', 'const SIBLING_DUMMY_SPACING: f64 = 50.0;', 'const SIBLING_DUMMY_SPACING: f64 = 40.0;'),
    ('src/hierarchy_layout.rs', 'room: less between cards', 'const SIBLING_SPACING: f64 = 60.0;', 'const SIBLING_SPACING: f64 = 50.0;'),
    ('src/hierarchy_layout.rs', 'room: less for ports', 'const MIN_PORT_CLEARANCE: f64 = 20.0;', 'const MIN_PORT_CLEARANCE: f64 = 10.0;'),
    ('src/hierarchy_layout.rs', 'room: no padding at the ends of a level', 'const CONTAINER_PADDING: f64 = 60.0;', 'const CONTAINER_PADDING: f64 = 0.0;'),
    ('src/hierarchy_layout.rs', 'connect: above and below changed', 'if self.slots[a].level < self.slots[b].level { (a, b) } else { (b, a) }', 'if self.slots[a].level > self.slots[b].level { (a, b) } else { (b, a) }'),
    ('src/hierarchy_layout.rs', 'dummies: none', '.filter(|below| self.slots[*below].level - level > 1)', '.filter(|below| self.slots[*below].level - level > 1000)'),
    ('src/hierarchy_layout.rs', 'dummies: only over three levels', '.filter(|below| self.slots[*below].level - level > 1)', '.filter(|below| self.slots[*below].level - level > 2)'),
    ('src/hierarchy_layout.rs', 'dummies: far cards by level first', 'far.sort_by_key(|below| (self.slots[*below].rank, self.slots[*below].level));', 'far.sort_by_key(|below| (self.slots[*below].level, self.slots[*below].rank));'),
    ('src/hierarchy_layout.rs', 'dummies: far cards from the right', 'far.sort_by_key(|below| (self.slots[*below].rank, self.slots[*below].level));', 'far.sort_by_key(|below| std::cmp::Reverse((self.slots[*below].rank, self.slots[*below].level)));'),
    ('src/hierarchy_layout.rs', 'dummies: join their level in front', '                    self.by_level[between].push(dummy);\n', '                    self.by_level[between].insert(0, dummy);\n'),
    ('src/hierarchy_layout.rs', 'dummies: as wide as nothing', 'width: 1.0, height: 1.0, at', 'width: 0.0, height: 1.0, at'),
    ('src/hierarchy_layout.rs', 'no answer: dummies: as high as nothing', 'width: 1.0, height: 1.0, at', 'width: 1.0, height: 0.0, at'),
    ('src/hierarchy_layout.rs', 'crossings: arrows up count on the way down', 'filter(|(_, down)| arrows == Arrows::Both || *down)', 'filter(|(_, down)| arrows == Arrows::Both || !*down)'),
    ('src/hierarchy_layout.rs', 'crossings: also between up and down', 'i64::from(first.2 == second.2 && first.0 < second.0 && first.1 > second.1)', 'i64::from(first.0 < second.0 && first.1 > second.1)'),
    ('src/hierarchy_layout.rs', 'crossings: a shared end counts', 'first.0 < second.0 && first.1 > second.1)', 'first.0 < second.0 && first.1 >= second.1)'),
    ('src/hierarchy_layout.rs', 'crossings: lengths not counted', '(crossings, segments.iter().map(|segment| segment.3).sum())', '(crossings, 0.0)'),
    ('src/hierarchy_layout.rs', 'neighbours: two tried', 'for next in index + 1..index + 4 {', 'for next in index + 1..index + 3 {'),
    ('src/hierarchy_layout.rs', 'neighbours: four tried', 'for next in index + 1..index + 4 {', 'for next in index + 1..index + 5 {'),
    ('src/hierarchy_layout.rs', 'neighbours: no going round', 'let other = next % count;', 'let other = next.min(count - 1);'),
    ('src/hierarchy_layout.rs', 'neighbours: length does not decide', 'if crossings < best_crossings || (crossings == best_crossings && precision_compare(length, best_length) == -1) {\n                (best_crossings, best_length, best_index) = (crossings, length, other);', 'if crossings < best_crossings {\n                (best_crossings, best_length, best_index) = (crossings, length, other);'),
    ('src/hierarchy_layout.rs', 'neighbours: the last equal one wins', 'if crossings < best_crossings || (crossings == best_crossings && precision_compare(length, best_length) == -1) {\n                (best_crossings, best_length, best_index) = (crossings, length, other);', 'if crossings <= best_crossings {\n                (best_crossings, best_length, best_index) = (crossings, length, other);'),
    ('src/hierarchy_layout.rs', 'order: level not sorted first', '        self.by_level[level] = sorted;\n', '        let _ = sorted;\n'),
    ('src/hierarchy_layout.rs', 'order: sorted by the other side', 'if top_down { &layers.slots[slot].aboves } else { &layers.slots[slot].belows }', 'if top_down { &layers.slots[slot].belows } else { &layers.slots[slot].aboves }'),
    ('src/hierarchy_layout.rs', 'order: a card without neighbours goes last', '                return 0.0;\n', '                return f64::MAX;\n'),
    ('src/hierarchy_layout.rs', 'order: no exchange', '                self.by_level[level].swap(index, swap_index);\n', '                let _ = swap_index;\n'),
    ('src/hierarchy_layout.rs', 'order: shorter arrows are no reason', 'if new_crossings < crossings || (new_crossings == crossings && precision_compare(new_length, length) == -1) {', 'if new_crossings < crossings {'),
    ('src/hierarchy_layout.rs', 'order: as many crossings is reason enough', 'if new_crossings < crossings || (new_crossings == crossings && precision_compare(new_length, length) == -1) {', 'if new_crossings <= crossings {'),
    ('src/hierarchy_layout.rs', 'sift: also without crossings', '        if best_crossings == 0 || count == 1 {', '        if count == 1 {'),
    ('src/hierarchy_layout.rs', 'sift: an equal place is never taken', '                crossings <= best_crossings\n', '                crossings < best_crossings\n'),
    ('src/hierarchy_layout.rs', 'sift: an equal place is always taken', '            if improve_if_equal {\n', '            if true {\n'),
    ('src/hierarchy_layout.rs', 'sift: shorter arrows are no reason', '                crossings < best_crossings || (crossings == best_crossings && precision_compare(length, best_length) == -1)\n            }', '                crossings < best_crossings\n            }'),
    ('src/hierarchy_layout.rs', 'sift: not tried to the left', '        for index in (1..count).rev() {\n            self.by_level[level].swap(index - 1, index);\n            self.rank_level(level);\n            let (crossings, length) = self.crossings_and_length(level, Arrows::Both);\n            if improved(crossings, best_crossings, length, best_length) {', '        for index in (1..count).rev() {\n            self.by_level[level].swap(index - 1, index);\n            self.rank_level(level);\n            let (crossings, length) = self.crossings_and_length(level, Arrows::Both);\n            if false && improved(crossings, best_crossings, length, best_length) {'),
    ('src/hierarchy_layout.rs', 'sift: not tried to the right', '        for index in start..count - 1 {\n            self.by_level[level].swap(index, index + 1);\n            self.rank_level(level);\n            let (crossings, length) = self.crossings_and_length(level, Arrows::Both);\n            if improved(crossings, best_crossings, length, best_length) {', '        for index in start..count - 1 {\n            self.by_level[level].swap(index, index + 1);\n            self.rank_level(level);\n            let (crossings, length) = self.crossings_and_length(level, Arrows::Both);\n            if false && improved(crossings, best_crossings, length, best_length) {'),
    ('src/hierarchy_layout.rs', 'sift: left at the left end', '        for index in 0..best_index {\n            self.by_level[level].swap(index, index + 1);', '        for index in 0..best_index.min(0) {\n            self.by_level[level].swap(index, index + 1);'),
    ('src/hierarchy_layout.rs', 'sifting: none', '                self.sift(slot, improve_if_equal);\n', '                let _ = improve_if_equal;\n'),
    ('src/hierarchy_layout.rs', 'sifting: one round', '        for _ in 0..10 {', '        for _ in 0..1 {'),
    ('src/hierarchy_layout.rs', 'sifting: a dummy counts for nothing', 'map_or(2, |card| graph.edges_of(card).count())', 'map_or(0, |card| graph.edges_of(card).count())'),
    ('src/hierarchy_layout.rs', 'sifting: fewest connections first', '(std::cmp::Reverse(degree(self, *slot)), self.slots[*slot].level, self.slots[*slot].rank)', '(degree(self, *slot), self.slots[*slot].level, self.slots[*slot].rank)'),
    ('src/hierarchy_layout.rs', 'sifting: lowest level last', '(std::cmp::Reverse(degree(self, *slot)), self.slots[*slot].level, self.slots[*slot].rank)', '(std::cmp::Reverse(degree(self, *slot)), std::cmp::Reverse(self.slots[*slot].level), self.slots[*slot].rank)'),
    ('src/hierarchy_layout.rs', 'sifting: equal places never allowed', '            improve_if_equal = !improved;', '            improve_if_equal = false;'),
    ('src/hierarchy_layout.rs', 'sifting: stops after the first quiet round', '            if !improved && improve_if_equal {', '            if !improved {'),
    ('src/hierarchy_layout.rs', "between levels: label's width", '                label = label.max(edge.min_height);', '                label = label.max(edge.min_width);'),
    ('src/hierarchy_layout.rs', 'between levels: labels of far arrows too', '.abs_diff(levels[card]) == 1 && edge.has_label()', '.abs_diff(levels[card]) >= 1 && edge.has_label()'),
    ('src/hierarchy_layout.rs', 'between levels: crossings do not count', '(crossings as f64 * CROSSING_SPACING).max(label + CROSSING_SPACING)', '(label + CROSSING_SPACING)'),
    ('src/hierarchy_layout.rs', 'between levels: labels do not count', '(crossings as f64 * CROSSING_SPACING).max(label + CROSSING_SPACING)', '(crossings as f64 * CROSSING_SPACING)'),
    ('src/hierarchy_layout.rs', 'between levels: no most', '.clamp(CROSSING_SPACING, PARENT_SPACING).round()', '.max(CROSSING_SPACING).round()'),
    ('src/hierarchy_layout.rs', 'between levels: crossings of arrows up too', 'self.crossings_and_length(level, Arrows::Below)', 'self.crossings_and_length(level, Arrows::Both)'),
    ('src/hierarchy_layout.rs', 'rows: cards at the top of their level', '                slot.at.y += (middle - (slot.at.y + slot.height / 2.0)).ceil();\n', '                let _ = (middle, &slot);\n'),
    ('src/hierarchy_layout.rs', 'rows: middle not on a whole pixel', 'let middle = (y + (bottom - y) / 2.0).ceil();', 'let middle = y + (bottom - y) / 2.0;'),
    ('src/hierarchy_layout.rs', 'rows: card not moved by whole pixels', '(middle - (slot.at.y + slot.height / 2.0)).ceil();', '(middle - (slot.at.y + slot.height / 2.0));'),
    ('src/hierarchy_layout.rs', 'rows: next level not on a whole pixel', 'y += (height + self.distance_to_next_level(graph, levels, level) + 2.0 * MIN_PORT_CLEARANCE).ceil();', 'y += height + self.distance_to_next_level(graph, levels, level) + 2.0 * MIN_PORT_CLEARANCE + 0.25;'),
    ('src/hierarchy_layout.rs', 'no answer: rows: a dummy counts for the height', '                if self.slots[slot].card.is_some() {\n                    height = height.max(self.slots[slot].height);', '                if self.slots[slot].card.is_some() || true {\n                    height = height.max(self.slots[slot].height);'),
    ('src/hierarchy_layout.rs', 'conflicts: none', '                            conflicts.push((below, above));\n                            conflicts.push((above, below));', '                            let _ = (below, above);'),
    ('src/hierarchy_layout.rs', 'conflicts: from the first level', 'for level in 1..self.by_level.len().saturating_sub(1) {', 'for level in 0..self.by_level.len().saturating_sub(1) {'),
    ('src/hierarchy_layout.rs', 'conflicts: the last card does not close', 'let k1 = if index == next.len() - 1 {', 'let k1 = if index == usize::MAX {'),
    ('src/hierarchy_layout.rs', 'conflicts: window not moved on', '                k0 = k1;\n', '                let _ = &mut k0;\n'),
    ('src/hierarchy_layout.rs', 'conflicts: only right of the window', 'if self.slots[above].rank < k0 || self.slots[above].rank > k1 {', 'if self.slots[above].rank > k1 {'),
    ('src/hierarchy_layout.rs', 'conflicts: only left of the window', 'if self.slots[above].rank < k0 || self.slots[above].rank > k1 {', 'if self.slots[above].rank < k0 {'),
    ('src/hierarchy_layout.rs', 'align: no padding left', 'left_pad: pad(index == 0),', 'left_pad: pad(false),'),
    ('src/hierarchy_layout.rs', 'align: no padding right', 'right_pad: pad(index == level.len() - 1),', 'right_pad: pad(false),'),
    ('src/hierarchy_layout.rs', 'align: always from the top', '(if down { level as i64 } else { -(level as i64) },', '(level as i64,'),
    ('src/hierarchy_layout.rs', 'align: always from the left card', 'if from_left { rank as i64 } else { -(rank as i64) })', 'rank as i64)'),
    ('src/hierarchy_layout.rs', 'align: always under the cards above', 'let before = if down { &slot.aboves } else { &slot.belows };', 'let before = &slot.aboves;'),
    ('src/hierarchy_layout.rs', 'align: under the first neighbour', 'count if count % 2 == 1 => vec![neighbours[count / 2]],', 'count if count % 2 == 1 => vec![neighbours[0]],'),
    ('src/hierarchy_layout.rs', 'align: left middle first also from the right', 'if from_left { vec![left, right] } else { vec![right, left] }', 'vec![left, right]'),
    ('src/hierarchy_layout.rs', 'align: only the left middle', 'if from_left { vec![left, right] } else { vec![right, left] }', 'if from_left { vec![left] } else { vec![right] }'),
    ('src/hierarchy_layout.rs', 'align: the card before may be on another level', 'nodes[index].previous = same_level.then(|| index - 1);', 'nodes[index].previous = (index > 0 || same_level).then(|| index - 1);'),
    ('src/hierarchy_layout.rs', 'align: conflicts ignored', '                if conflicts.contains(&(nodes[index].slot, nodes[median].slot))\n                    || nodes[index].aligned_with != index', '                if nodes[index].aligned_with != index'),
    ('src/hierarchy_layout.rs', 'align: a card aligned twice', '                    || nodes[index].aligned_with != index\n', '                    || false\n'),
    ('src/hierarchy_layout.rs', 'align: under a card already passed', '(from_left && last_aligned_rank >= rank)', '(from_left && last_aligned_rank > rank)'),
    ('src/hierarchy_layout.rs', 'align: over a card already passed', '(!from_left && last_aligned_rank <= rank)', '(!from_left && last_aligned_rank < rank)'),
    ('src/hierarchy_layout.rs', 'align: block as wide as its first card', '                nodes[root].block_size = nodes[root].block_size.max(nodes[index].block_size);\n', ''),
    ('src/hierarchy_layout.rs', 'align: block without the left padding of its cards', '                nodes[root].left_pad = nodes[root].left_pad.max(nodes[index].left_pad);\n', ''),
    ('src/hierarchy_layout.rs', 'align: block without the right padding of its cards', '                nodes[root].right_pad = nodes[root].right_pad.max(nodes[index].right_pad);\n', ''),
    ('src/hierarchy_layout.rs', 'align: last aligned card not noted', '                last_aligned_rank = rank;\n', '                let _ = rank;\n'),
    ('src/hierarchy_layout.rs', 'align: last aligned card kept over levels', '                last_aligned_rank = if from_left { i64::MIN } else { i64::MAX };\n', ''),
    ('src/hierarchy_layout.rs', 'blocks: class not taken over', '                        nodes[root].sink = nodes[previous_root].sink;\n', '                        let _ = previous_root;\n'),
    ('src/hierarchy_layout.rs', 'blocks: a dummy keeps the room of a card', 'let delta = if beside_dummy { SIBLING_DUMMY_SPACING } else { SIBLING_SPACING } + pads;', 'let delta = SIBLING_SPACING + pads;'),
    ('src/hierarchy_layout.rs', 'blocks: only a dummy itself keeps less room', 'slots[nodes[root].slot].card.is_none() || nodes[root].previous.is_some_and(|before| slots[nodes[before].slot].card.is_none());', 'slots[nodes[root].slot].card.is_none();'),
    ('src/hierarchy_layout.rs', 'blocks: paddings of the wrong sides', 'if from_left { nodes[previous_root].right_pad + nodes[root].left_pad } else { nodes[previous_root].left_pad + nodes[root].right_pad }', 'if from_left { nodes[previous_root].left_pad + nodes[root].right_pad } else { nodes[previous_root].right_pad + nodes[root].left_pad }'),
    ('src/hierarchy_layout.rs', 'blocks: classes never pushed apart', '                    if nodes[root].sink != sink {', '                    if nodes[root].sink != sink && nodes.is_empty() {'),
    ('src/hierarchy_layout.rs', 'blocks: classes pushed apart by the most', 'nodes[sink].shift.min(nodes[root].x - nodes[previous_root].x - nodes[previous_root].block_size - delta)', 'nodes[sink].shift.max(nodes[root].x - nodes[previous_root].x - nodes[previous_root].block_size - delta)'),
    ('src/hierarchy_layout.rs', 'blocks: from the left without the block before', 'nodes[root].x.max(nodes[previous_root].x + nodes[previous_root].block_size + delta)', 'nodes[root].x.max(nodes[previous_root].x + delta)'),
    ('src/hierarchy_layout.rs', 'blocks: from the right without the block itself', 'nodes[root].x.min(nodes[previous_root].x - nodes[root].block_size - delta)', 'nodes[root].x.min(nodes[previous_root].x - delta)'),
    ('src/hierarchy_layout.rs', "blocks: only the first card's neighbour asked", '                node = nodes[node].aligned_with;\n', '                node = root;\n'),
    ('src/hierarchy_layout.rs', 'blocks: class shift not applied', '                nodes[index].x += shift;\n', '                let _ = shift;\n'),
    ('src/hierarchy_layout.rs', 'no answer: blocks: class shift applied to every card', 'if nodes[index].root == index && shift.is_finite() {', 'if shift.is_finite() {'),
    ('src/hierarchy_layout.rs', "blocks: card at the block's edge", 'xs[node.slot] = node.x + nodes[node.root].block_size / 2.0 - self.slots[node.slot].width / 2.0;', 'xs[node.slot] = node.x;'),
    ('src/hierarchy_layout.rs', 'blocks: width without the last block', 'most = most.max(node.x + node.block_size);', 'most = most.max(node.x);'),
    ('src/hierarchy_layout.rs', 'four: always moved to the first alignment', '                narrowest = index;\n', '                let _ = index;\n'),
    ('src/hierarchy_layout.rs', 'four: the widest decides', 'if most - least < alignments[narrowest].2 - alignments[narrowest].1 {', 'if most - least > alignments[narrowest].2 - alignments[narrowest].1 {'),
    ('src/hierarchy_layout.rs', 'four: not moved together', '                places.push(xs[slot] + shift);\n', '                places.push(xs[slot] + shift * 0.0);\n'),
    ('src/hierarchy_layout.rs', 'four: moved by the wrong ends', 'let shift = if directions[index].1 { alignments[narrowest].1 - least } else { alignments[narrowest].2 - most };', 'let shift = if directions[index].1 { alignments[narrowest].2 - most } else { alignments[narrowest].1 - least };'),
    ('src/hierarchy_layout.rs', 'four: the second place, not the middle', '(places[1] / 2.0 + places[2] / 2.0).round();', 'places[1].round();'),
    ('src/hierarchy_layout.rs', 'four: the mean of the outer two', '(places[1] / 2.0 + places[2] / 2.0).round();', '(places[0] / 2.0 + places[3] / 2.0).round();'),
    ('src/hierarchy_layout.rs', 'four: not on a whole pixel', '(places[1] / 2.0 + places[2] / 2.0).round();', '(places[1] / 2.0 + places[2] / 2.0);'),
    ('src/hierarchy_layout.rs', 'start: not shuffled', '    random.shuffle(&mut order);\n', '    let _ = &mut *random;\n'),
    ('src/hierarchy_layout.rs', "start: levels filled in the cards' order", '    for &card in &order {\n        layers.by_level[levels[card]].push(slot_of(card));', '    for card in 0..graph.nodes.len() {\n        layers.by_level[levels[card]].push(slot_of(card));'),
    ('src/hierarchy_layout.rs', "start: dummies made in the cards' order", '    layers.break_long_connections(&order);', '    layers.break_long_connections(&(0..graph.nodes.len()).collect::<Vec<usize>>());'),
    ('src/hierarchy_layout.rs', 'start: one round of ordering', '    for _ in 0..widest {', '    for _ in 0..widest.min(1) {'),
    ('src/hierarchy_layout.rs', 'start: first level also sorted by the cards above', 'layers.minimize_crossings(level, level > 0);', 'layers.minimize_crossings(level, true);'),
    ('src/hierarchy_layout.rs', 'start: no places across', '    layers.align_hierarchy();\n', ''),
    ('src/hierarchy_layout.rs', 'random: started anew for every hierarchy', 'place_group(&mut part, &group_levels, &mut random);', 'place_group(&mut part, &group_levels, &mut GoRand::new(seed));'),
    ('src/hierarchy_layout.rs', 'random: another seed', 'let mut random = GoRand::new(seed);', 'let mut random = GoRand::new(seed + 1);'),
    ('src/place.rs', 'packed: a hierarchy is placed like any group', '        if let Some(places) = hierarchy(group) {\n            for (card, place) in part.nodes.iter_mut().zip(places) {', '        if let Some(places) = hierarchy(group).filter(|_| false) {\n            for (card, place) in part.nodes.iter_mut().zip(places) {'),
    ('src/place.rs', 'packed: a hierarchy is turned to its flow', '                card.top_left = place;\n            }\n        } else {', '                card.top_left = place;\n            }\n            direct(&mut part, group.len());\n        } else {'),
]

LIST = os.environ.get('LIST', 'trees')
sys.path.insert(0, os.path.dirname(os.path.abspath(__file__)))
from cluster_breaks import CLUSTERS
from swap_breaks import SWAP
from align_breaks import ALIGN
from gaps_breaks import GAPS
from cluster_stage_breaks import CLUSTER_STAGE
from balance_breaks import BALANCE
from pack_breaks import PACK
from routed_breaks import ROUTED
from swap_ports_breaks import SWAP_PORTS, ROUTE_STAGES, NORMALIZE, PACK_ROUTED, PIPELINE
from simplify_breaks import SIMPLIFY
from dejitter_breaks import DEJITTER
from nudge_breaks import NUDGE, QUALITY
from tree_routes_breaks import TREE_ROUTES
from hierarchy_routes_breaks import HIERARCHY_ROUTES
from cluster_routes_breaks import CLUSTER_ROUTES
from cluster_placement_breaks import CLUSTER_PLACEMENT
from score_breaks import SCORE
BREAKS = {'layout': LAYOUT, 'hierarchy': HIERARCHY, 'hlayout': HLAYOUT, 'clusters': CLUSTERS, 'swap': SWAP, 'align': ALIGN, 'gaps': GAPS, 'cluster_stage': CLUSTER_STAGE, 'balance': BALANCE, 'pack': PACK, 'routed': ROUTED, 'swap_ports': SWAP_PORTS, 'simplify': SIMPLIFY, 'pipeline': PIPELINE, 'pack_routed': PACK_ROUTED, 'dejitter': DEJITTER, 'normalize': NORMALIZE, 'nudge': NUDGE, 'quality': QUALITY, 'route_stages': ROUTE_STAGES, 'tree_routes': TREE_ROUTES, 'hierarchy_routes': HIERARCHY_ROUTES, 'cluster_routes': CLUSTER_ROUTES, 'cluster_placement': CLUSTER_PLACEMENT, 'score': SCORE, 'trees': [('src/trees.rs', *one) for one in BREAKS]}[LIST]
RESULTS = {'clusters': 'cluster_breaks.jsonl', 'swap': 'swap_breaks.jsonl', 'align': 'align_breaks.jsonl', 'gaps': 'gaps_breaks.jsonl', 'cluster_stage': 'cluster_stage_breaks.jsonl', 'balance': 'balance_breaks.jsonl', 'pack': 'pack_breaks.jsonl', 'routed': 'routed_breaks.jsonl', 'swap_ports': 'swap_ports_breaks.jsonl', 'simplify': 'simplify_breaks.jsonl', 'pipeline': 'pipeline_breaks.jsonl', 'pack_routed': 'pack_routed_breaks.jsonl', 'dejitter': 'dejitter_breaks.jsonl', 'normalize': 'normalize_breaks.jsonl', 'nudge': 'nudge_breaks.jsonl', 'quality': 'quality_breaks.jsonl', 'route_stages': 'route_stages_breaks.jsonl', 'tree_routes': 'tree_routes_breaks.jsonl', 'hierarchy_routes': 'hierarchy_routes_breaks.jsonl', 'cluster_routes': 'cluster_routes_breaks.jsonl', 'cluster_placement': 'cluster_placement_breaks.jsonl', 'score': 'score_breaks.jsonl', 'layout': 'tree_layout_breaks.jsonl', 'hierarchy': 'hierarchy_breaks.jsonl', 'hlayout': 'hierarchy_layout_breaks.jsonl', 'trees': 'tree_breaks.jsonl'}[LIST]

running = None


def stop_running(*_):
    if running and running.poll() is None:
        running.kill()
    if _:
        sys.exit("stopped")


atexit.register(stop_running)
signal.signal(signal.SIGTERM, stop_running)
signal.signal(signal.SIGINT, stop_running)
signal.signal(signal.SIGHUP, stop_running)


def build(arguments):
    """Builds in the copy; gives the built program, or None with cargo's output."""
    done = subprocess.run(["cargo", *arguments, "--message-format=json"], cwd=COPY, env=ENV, capture_output=True, text=True, timeout=900)
    program = None
    for line in done.stdout.splitlines():
        if line.startswith("{") and '"executable":"' in line:
            program = json.loads(line).get("executable") or program
    return (program if done.returncode == 0 else None), done.stdout + done.stderr


def run(arguments, timeout):
    """Runs a built program itself, so that it can be stopped."""
    global running
    with tempfile.TemporaryFile("w+") as out:
        limit = lambda: resource.setrlimit(resource.RLIMIT_CPU, (CPU_SECONDS, CPU_SECONDS))
        running = child = subprocess.Popen(arguments, cwd=COPY, env=ENV, stdout=out, stderr=subprocess.STDOUT, text=True, preexec_fn=limit)
        deadline = time.time() + timeout
        code = None
        while code is None:
            try:
                code = child.wait(0.05)
            except subprocess.TimeoutExpired:
                size = subprocess.run(["ps", "-o", "rss=", "-p", str(child.pid)], capture_output=True, text=True).stdout.strip()
                if time.time() > deadline or (size and int(size) > LIMIT_KB):
                    child.kill()
                    child.wait()
                    code = "timeout" if time.time() > deadline else "memory"
        out.seek(0)
        output = out.read()
        if code == -signal.SIGABRT and "panicked" not in output:
            code = "memory"
        if code == -signal.SIGXCPU:
            code = "timeout"
        return code, output


def test():
    program, output = build(["test", "--release", "--test", TEST, "--no-run"])
    if program is None:
        return 1, "could not compile\n" + output
    return run([program], 150)


def probe():
    program, output = build(["build", "--release", "--example", PROBE_EXAMPLE])
    if program is None:
        return 1, "could not compile\n" + output
    return run([program, PROBE_MODE, PROBE, "100000"] + (["quick"] if os.environ.get("QUICK") else []), 300)


def main():
    sources = {file: open(os.path.join(CRATE, file)).read() for file in {one[0] for one in BREAKS}}
    for file, name, old, new in BREAKS:
        if sources[file].count(old) != 1:
            sys.exit(f"{name}: the text occurs {sources[file].count(old)} times")
    if os.path.exists(COPY):
        shutil.rmtree(COPY)
    shutil.copytree(CRATE, COPY, ignore=shutil.ignore_patterns("target", "pkg", "node_modules"))
    for capped in (f"tests/{TEST}.rs", f"examples/{PROBE_EXAMPLE}.rs"):
        open(os.path.join(COPY, capped), "a").write(MEMORY_CAP)
    # The copy keeps the files' times: touch them, so that no build of a broken copy passes for it.
    for folder, _, files in os.walk(COPY):
        for file in files:
            os.utime(os.path.join(folder, file))
    code, output = test()
    if code != 0 or "test result: ok." not in output or " 0 failed" not in output:
        sys.exit(f"the unchanged copy does not pass: {code}\n{output[-2000:]}")
    if PROBE:
        code, output = probe()
        if code != 0:
            sys.exit(f"the unchanged copy differs from the probe: {code}\n{output[-2000:]}")
    results = open(os.path.join(SCRATCH, RESULTS), "a" if ONLY or NUMBERS else "w")
    counts = {}
    for number, (file, name, old, new) in enumerate(BREAKS):
        if (ONLY and ONLY not in name) or (NUMBERS and number not in NUMBERS):
            continue
        for other, source in sources.items():
            open(os.path.join(COPY, other), "w").write(source.replace(old, new) if other == file else source)
        code, output = test()
        detail = []
        if "could not compile" in output:
            outcome = "does not compile"
            detail = [json.loads(line)["message"]["message"] for line in output.splitlines() if line.startswith("{") and '"level":"error"' in line][:3]
        elif code == 0:
            outcome = "not caught"
            if PROBE:
                code, output = probe()
                if "could not compile" in output:
                    outcome = "does not compile (probe)"
                elif code != 0:
                    outcome = "caught by the probe only"
                    open(os.path.join(SCRATCH, f"{RESULTS}.{number}.json"), "w").write(next((line for line in output.splitlines() if line.startswith("[")), "[]"))
                    detail = [line[:160] for line in output.splitlines() if "diagrams:" in line or "panicked" in line][:2]
        else:
            outcome = "caught"
            detail = [line.strip()[:160] for line in output.splitlines() if "panicked" in line][:1]
        if code in ("timeout", "memory"):
            detail = [f"never ends ({code})"]
        if name.startswith("no answer"):
            outcome = "no answer, as expected" if outcome == "not caught" else f"no answer expected, but: {outcome}"
        counts[outcome] = counts.get(outcome, 0) + 1
        results.write(json.dumps({"number": number, "name": name, "outcome": outcome, "detail": detail}) + "\n")
        results.flush()
    print(len(BREAKS), counts)


main()
