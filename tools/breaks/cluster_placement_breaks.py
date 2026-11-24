"""Deliberate breaks of what the placement stages before routing take from TALA's
transactions and its price cache: the cache of the price of the whole diagram (src/graph.rs,
src/swap.rs, src/gaps.rs), what a transaction lets through (src/optimizer.rs `Allowed`), and
where the stages anchor it. Run with LIST=cluster_placement (TEST=stages_reference,
PROBE_EXAMPLE=probe_stages, the default) and, for the stages after routing,
LIST=cluster_placement TEST=routed_clusters_reference PROBE_EXAMPLE=probe_routed."""
G = 'src/graph.rs'
S = 'src/swap.rs'
P = 'src/gaps.rs'
O = 'src/optimizer.rs'
A = 'src/align.rs'
D = 'src/dejitter.rs'
C = 'src/cluster_stage.rs'
N = 'src/compaction.rs'
E = 'src/placementcost/edge_length.rs'
CLUSTER_PLACEMENT = [
    # The cache.
    (S, 'cache: none for the price with direction', "    if let Some(known) = graph.length_cache.get(&key) {\n        return known;\n    }\n    let cards", "    let cards"),
    (P, 'cache: none for the price without direction', "    if let Some(known) = graph.length_cache.get(&key) {\n        return known;\n    }\n    let total = uncached_length(graph, costs);", "    let total = uncached_length(graph, costs);"),
    (G, 'cache: one key for both prices', "let mut key = vec![u64::from(direction), self.cell_size.to_bits(),", "let mut key = vec![0, self.cell_size.to_bits(),"),
    (G, 'cache: the cell size is not in the key', "let mut key = vec![u64::from(direction), self.cell_size.to_bits(),", "let mut key = vec![u64::from(direction), 0,"),
    (G, 'cache: the stored prices are not in the key', "let mut key = vec![u64::from(direction), self.cell_size.to_bits(), turn.to_bits(), 0.0_f64.to_bits(), crossing.to_bits()];", "let mut key = vec![u64::from(direction), self.cell_size.to_bits()];\n        let _ = (turn, crossing);"),
    (G, 'cache: no places in the key', "            key.extend([self.id(node) as u64, card.top_left.x.to_bits(), card.top_left.y.to_bits(), card.width.to_bits(), card.height.to_bits()]);", "            key.extend([self.id(node) as u64, card.width.to_bits(), card.height.to_bits()]);"),
    (G, 'cache: the prices are never stored', "        self.prices.borrow_mut().insert(key, price);\n        self.stored.set((turn, crossing));", "        self.prices.borrow_mut().insert(key, price);\n        let _ = (turn, crossing);"),
    (G, 'cache: a reset forgets the crossing too', "        let (_, crossing) = self.stored.get();\n        self.stored.set((0.0, crossing));", "        let (_, crossing) = self.stored.get();\n        let _ = crossing;\n        self.stored.set((0.0, 0.0));"),
    (P, 'cache: the gap stage does not reset the bend', "    placed.graph.length_cache.forget_turn();\n", ""),
    (A, 'cache: no price at the start of a round of lining up', "    length(&placed.graph, &costs);\n", ""),
    # What a transaction lets through.
    (O, 'transaction: cards of clusters are asked too', "            graph.cluster_of(a).is_some() || !too_close(graph, a, b)", "            !too_close(graph, a, b)"),
    (O, 'transaction: a pair is let through if either card is in a cluster', "            graph.cluster_of(a).is_some() || !too_close(graph, a, b)", "            graph.cluster_of(a).is_some() || graph.cluster_of(b).is_some() || !too_close(graph, a, b)"),
    (O, 'transaction: since the rollback point, every pair is asked', "        Self::pairs(graph).all(|(a, b)| !(moved(a) || moved(b)) || self.pair_allowed(graph, a, b))", "        let _ = moved;\n        Self::pairs(graph).all(|(a, b)| self.pair_allowed(graph, a, b))"),
    (O, 'transaction: since the rollback point, only pairs where both moved', "        Self::pairs(graph).all(|(a, b)| !(moved(a) || moved(b)) || self.pair_allowed(graph, a, b))", "        Self::pairs(graph).all(|(a, b)| !(moved(a) && moved(b)) || self.pair_allowed(graph, a, b))"),
    # Where the stages anchor it.
    (P, 'anchor: the gap stage per pull', "        let before = (graph.nodes.clone(), graph.vessels.clone());\n        for &card in &pulled {", "        let before = (graph.nodes.clone(), graph.vessels.clone());\n        let anchor = Allowed::from(&*graph);\n        let allowed = &anchor;\n        for &card in &pulled {"),
    (P, 'anchor: the pull asks every pair', "        if !allowed.now_since(graph, &before.0) {", "        if !allowed.now(graph) {"),
    (P, 'gaps: the clusters are put in order only where a card moved', "        // commit puts the cards of every cluster in order, also where nothing moved.\n        in_order(&mut placed.graph);", "        // commit puts the cards of every cluster in order, also where nothing moved."),
    (A, 'anchor: lining up per connection', "        let (x, y) = alignment_deltas(graph, edge);", "        let anchor = Allowed::from(&*graph);\n        let allowed = &anchor;\n        let (x, y) = alignment_deltas(graph, edge);"),
    (A, 'anchor: lining up per round', "fn align_round(placed: &mut Placed, allowed: &Allowed) -> bool {\n    let costs = placed.prices();", "fn align_round(placed: &mut Placed, _allowed: &Allowed) -> bool {\n    let anchor = Allowed::from(&placed.graph);\n    let allowed = &anchor;\n    let costs = placed.prices();"),
    (C, 'anchor: the cluster stage pulls with the stage\'s places at its start', "    let anchor = Allowed::from(&*graph);\n    reduce_gap(", "    let anchor = &allowed;\n    reduce_gap("),
    # Node placement of cards with clusters.
    (N, 'compaction: the group\'s own place is not tried as a move', "let mut moves = candidate_moves(graph, root, options, if at_the_back { 2.0 } else { 0.0 }, edges);\n        moves.push(home);", "let moves = candidate_moves(graph, root, options, if at_the_back { 2.0 } else { 0.0 }, edges);"),
    (E, 'price: the parts of the cluster penalty are added up before the distance', "cluster_penalty(graph, vessel, outside, direction, turn, &mut distance);", "let mut penalty = 0.0;\n            cluster_penalty(graph, vessel, outside, direction, turn, &mut penalty);\n            distance += penalty;"),
]
