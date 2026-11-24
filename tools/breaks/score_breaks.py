"""Deliberate breaks of TALA's score of a finished drawing and its choice between seeds
(src/score.rs, src/pipeline.rs). Run with LIST=score TEST=score_reference
PROBE_EXAMPLE=probe_routed PROBE_MODE=whole and a probe file made by routed_fixture.py from a
recording with STAGES_SEEDS=1."""
S = 'src/score.rs'
P = 'src/pipeline.rs'
SCORE = [
    # The order of two scores, and the pick.
    (S, 'compare: the area before the penalty', "        match compare_number(self.penalty, other.penalty, false) {\n            Ordering::Equal => compare_number(self.area, other.area, true),", "        match compare_number(self.area, other.area, true) {\n            Ordering::Equal => compare_number(self.penalty, other.penalty, false),"),
    (S, 'compare: the area alone', "        match compare_number(self.penalty, other.penalty, false) {", "        match Ordering::Equal {"),
    (S, 'compare: a larger area wins', "            Ordering::Equal => compare_number(self.area, other.area, true),", "            Ordering::Equal => compare_number(other.area, self.area, true),"),
    (S, 'pick: of equal scores the earlier seed wins', "score.compare(&scores[known]) != Ordering::Greater", "score.compare(&scores[known]) == Ordering::Less"),
    (S, 'pick: the last seed always', "score.compare(&scores[known]) != Ordering::Greater", "true"),
    (P, 'seeds: two only', "pub const SEEDS: [i64; 3] = [1, 2, 3];", "pub const SEEDS: [i64; 2] = [1, 2];"),
    (P, 'seeds: the other order', "pub const SEEDS: [i64; 3] = [1, 2, 3];", "pub const SEEDS: [i64; 3] = [3, 2, 1];"),
    # The penalty of the routes.
    (S, 'routes of clusters: counted too', "        if graph.cluster_of(wire.from).is_some() || graph.cluster_of(wire.to).is_some() {\n            continue;\n        }", ""),
    (S, 'routes of clusters: only those that leave one are left out', "        if graph.cluster_of(wire.from).is_some() || graph.cluster_of(wire.to).is_some() {", "        if graph.cluster_of(wire.from).is_some() {"),
    (S, 'bends: one each', "        penalty += bends * 0.5;", "        penalty += bends;"),
    (S, 'bends: the pieces are counted', "        let bends = route.len().saturating_sub(2) as f64;", "        let bends = route.len().saturating_sub(1) as f64;"),
    (S, 'slanted pieces: cost nothing', "                penalty += 3.0;", "                penalty += 0.0;"),
    (S, 'slanted pieces: a piece along one axis costs', "            if piece[0].x != piece[1].x && piece[0].y != piece[1].y {", "            if piece[0].x != piece[1].x || piece[0].y != piece[1].y {"),
    (S, 'crossings: not counted', "    penalty += crossings(routes) as f64;", ""),
    (S, 'crossings: of a route with itself too', "        for other in &routes[index + 1..] {\n            for piece in route.windows(2) {\n                for other_piece in other.windows(2) {\n                    if crosses(", "        for other in &routes[index..] {\n            for piece in route.windows(2) {\n                for other_piece in other.windows(2) {\n                    if crosses("),
    (S, 'labels: their score instead of one less it', "    penalty += 1.0 - label_score(graph, routes, &boxes);", "    penalty += label_score(graph, routes, &boxes);"),
    (S, 'labels: not counted', "    penalty += 1.0 - label_score(graph, routes, &boxes);", ""),
    # The labels.
    (S, 'labels: the sum, not one over one more', "    1.0 / (1.0 + total)", "    1.0 - total"),
    (S, 'labels: a label is not kept for the next', "        total += score;\n        placed.push(label);", "        total += score;"),
    (S, 'cards: no padding', "            if !overlap(&label, card, PADDING) {\n                continue;", "            if !overlap(&label, card, 0.0) {\n                continue;"),
    (S, 'cards: the covered part only where the boxes overlap with padding', "            if overlap(&label, card, 0.0) {\n                covered += overlap_area(&label, card);", "            if overlap(&label, card, PADDING) {\n                covered += overlap_area(&label, card);"),
    (S, 'cards: no covered part', "                covered += overlap_area(&label, card);", "                covered += 0.0;"),
    (S, 'cards: no whole area once more', "            covered += label.width * label.height;\n        }", "        }"),
    (S, 'cards: the covered part counts once', "        let mut score = covered / (label.width * label.height) * 2.0;", "        let mut score = covered / (label.width * label.height);"),
    (S, 'pieces: its own route too', "        let pieces = routes.iter().enumerate().filter(|(other, _)| *other != edge)", "        let pieces = routes.iter().enumerate().filter(|_| true)"),
    (S, 'pieces: no padding', "route.windows(2).filter(|piece| overlaps_line(&label, piece[0], piece[1], PADDING)).count()", "route.windows(2).filter(|piece| overlaps_line(&label, piece[0], piece[1], 0.0)).count()"),
    (S, 'pieces: once per route', "route.windows(2).filter(|piece| overlaps_line(&label, piece[0], piece[1], PADDING)).count()", "usize::from(route.windows(2).any(|piece| overlaps_line(&label, piece[0], piece[1], PADDING)))"),
    (S, 'pieces: count once', "        score += pieces as f64 * 2.0;", "        score += pieces as f64;"),
    (S, 'labels on labels: count once', "        score += on_labels as f64 * 10.0;", "        score += on_labels as f64;"),
    (S, 'labels near labels: not counted', "        score += near_labels as f64;", ""),
    (S, 'labels near labels: no padding', "        let near_labels = placed.iter().filter(|other| overlap(&label, other, PADDING)).count();", "        let near_labels = placed.iter().filter(|other| overlap(&label, other, 0.0)).count();"),
    (S, 'shared stretches: not counted', "        score += on_shared as f64;", ""),
    (S, 'shared stretches: no padding', "        let on_shared = shared.iter().filter(|stretch| overlap(&label, stretch, PADDING)).count();", "        let on_shared = shared.iter().filter(|stretch| overlap(&label, stretch, 0.0)).count();"),
    # Boxes and lines.
    (S, 'overlap: touching counts', "    if first.top_left.x >= second_right + padding || second.top_left.x >= first_right + padding {", "    if first.top_left.x > second_right + padding || second.top_left.x > first_right + padding {"),
    (S, 'overlap: touching counts across', "    first.top_left.y < second_bottom + padding && second.top_left.y < first_bottom + padding", "    first.top_left.y <= second_bottom + padding && second.top_left.y <= first_bottom + padding"),
    (S, 'overlap area: the outer two', "    (xs[2] - xs[1]) * (ys[2] - ys[1])", "    (xs[3] - xs[0]) * (ys[3] - ys[0])"),
    (S, 'line: an end inside does not count', "    if contains(first) || contains(second) {\n        return true;\n    }", ""),
    (S, 'line: the box without padding', "    let (left, right) = (area.top_left.x - padding, area.top_left.x + area.width + padding);", "    let (left, right) = (area.top_left.x, area.top_left.x + area.width);"),
    (S, 'line: an end on the border is outside', "    let contains = |point: Point| left <= point.x && right >= point.x && top <= point.y && bottom >= point.y;", "    let contains = |point: Point| left < point.x && right > point.x && top < point.y && bottom > point.y;"),
    (S, 'line: a touch is no meeting', "    let straddles = |a: f64, b: f64| a == 0.0 || b == 0.0 || (a < 0.0) != (b < 0.0);", "    let straddles = |a: f64, b: f64| (a < 0.0) != (b < 0.0) && a != 0.0 && b != 0.0;"),
    (S, 'line: pieces on one line never meet', "        return within(first_start.x, first_end.x, second_start.x, second_end.x) && within(first_start.y, first_end.y, second_start.y, second_end.y);", "        return false;"),
    # The stretches that routes share.
    (S, 'shared: only upright ones', "                add(&mut horizontal, start.y, (start, end));", ""),
    (S, 'shared: only level ones', "                add(&mut vertical, start.x, (start, end));", ""),
    (S, 'shared: pieces that only touch share', "                if along(&previous.1) > along(&current.0) {", "                if along(&previous.1) >= along(&current.0) {"),
    (S, 'shared: the stretch runs to the further end', "                            set(&mut end, along(&previous.1).min(along(&current.1)));", "                            set(&mut end, along(&previous.1).max(along(&current.1)));"),
    (S, 'shared: a longer stretch is not grown', "                            let value = along(end).max(least);", "                            let value = along(end);"),
    (S, 'shared: the last stretch of a line is lost', "            if let Some(done) = stretch {\n                shared.push(done);\n            }", ""),
    (S, 'shared: a stretch is ended by every gap only once', "                } else if let Some(done) = stretch.take() {\n                    shared.push(done);\n                }", "                }"),
    (S, 'shared: the piece reaching furthest is not followed', "                if along(&current.1) > along(&previous.1) {\n                    previous = current;\n                }", "                previous = current;"),
    (S, 'shared: sorted by end only', "pieces.sort_by(|left, right| if along(&left.0) == along(&right.0) { along(&left.1).total_cmp(&along(&right.1)) } else { along(&left.0).total_cmp(&along(&right.0)) });", "pieces.sort_by(|left, right| along(&left.1).total_cmp(&along(&right.1)));"),
    (S, 'shared: room twice as wide', "const SHARED_CLEARANCE: f64 = 2.5 * PADDING;", "const SHARED_CLEARANCE: f64 = 5.0 * PADDING;"),
    (S, 'shared: no room around a stretch', "                width = 2.0 * SHARED_CLEARANCE;\n                top_left.x -= SHARED_CLEARANCE;", "                width = 0.0;"),
    # The area.
    (S, 'area: without the labels', "        if let Some(label) = label {\n            low = Point { x: low.x.min(label.top_left.x)", "        if let Some(label) = label.filter(|_| false) {\n            low = Point { x: low.x.min(label.top_left.x)"),
    (S, 'area: without the routes', "        for point in route {\n            low = Point { x: low.x.min(point.x), y: low.y.min(point.y) };", "        for point in route.iter().take(0) {\n            low = Point { x: low.x.min(point.x), y: low.y.min(point.y) };"),
    (S, 'area: the box of a route is not rounded', "        min = Point { x: min.x.min(low.x.round()), y: min.y.min(low.y.round()) };\n        max = Point { x: max.x.max(high.x.round()), y: max.y.max(high.y.round()) };", "        min = Point { x: min.x.min(low.x), y: min.y.min(low.y) };\n        max = Point { x: max.x.max(high.x), y: max.y.max(high.y) };"),
    (S, 'area: the whole box is rounded', "    (min.x - max.x).abs() * (min.y - max.y).abs()", "    (min.x.round() - max.x.round()).abs() * (min.y.round() - max.y.round()).abs()"),
    (S, 'area: the width only', "    (min.x - max.x).abs() * (min.y - max.y).abs()", "    (min.x - max.x).abs()"),
]
