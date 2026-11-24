// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.

//! The steps after the route search must change routes as TALA's do: these tests replay layouts
//! recorded from TALA's own Go code, step by step (docs/reference.md).

use tidy_layout::graph::{Edge, Graph, Node, Point};
use tidy_layout::routing::balance::balance;
use tidy_layout::routing::coordinator::{FLAVORS, generate_routes, route_all, route_edges};
use tidy_layout::routing::line::{straight_line_costs, straighten_one};
use tidy_layout::routing::ovg;
use serde::Deserialize;

#[derive(Deserialize)]
struct Card {
    x: f64,
    y: f64,
    width: f64,
    height: f64,
}

type Routes = Vec<Vec<[f64; 2]>>;

#[derive(Deserialize)]
struct Case {
    kind: String,
    cards: Vec<Card>,
    links: Vec<[usize; 2]>,
    distances: Option<Vec<f64>>,
    costs: Option<Vec<Vec<f64>>>,
    searched: Option<Routes>,
    straightened: Option<Routes>,
    balanced: Option<Routes>,
    reordered: Option<Routes>,
}

fn pairs(routes: &[Vec<Point>]) -> Routes {
    routes.iter().map(|route| route.iter().map(|p| [p.x, p.y]).collect()).collect()
}

#[test]
fn the_steps_after_the_search_change_routes_as_talas_do() {
    let cases: Vec<Case> = serde_json::from_str(include_str!("fixtures/steps.tala.json")).unwrap();
    assert_eq!(cases.len(), 262);
    let (mut links, mut possible, mut quick, mut straightened, mut balanced) = (0, 0, 0, 0, 0);
    for (number, case) in cases.iter().enumerate() {
        if case.links.is_empty() {
            continue;
        }
        let nodes = case
            .cards
            .iter()
            .map(|c| Node { top_left: Point { x: c.x, y: c.y }, width: c.width, height: c.height })
            .collect();
        let edges = case.links.iter().map(|[from, to]| Edge::plain(*from, *to)).collect();
        let graph = Graph::new(nodes, edges);
        let what = format!("case {number} ({}, {} cards)", case.kind, case.cards.len());

        // The search, now with cards close together: they get a straight line at once.
        let ovg = ovg::build(&graph);
        for (flavor, theirs) in FLAVORS.iter().zip(case.distances.as_ref().unwrap()) {
            let ours = generate_routes(&graph, &ovg, *flavor).unwrap_or_else(|error| panic!("{what}: {error:?}"));
            assert_eq!(ours.distance, *theirs, "{what}, {flavor:?}: sum of prices");
        }
        let mut routes = route_all(&graph).unwrap();
        let searched = case.searched.as_ref().unwrap();
        assert_eq!(&pairs(&routes), searched, "{what}: after the search");
        quick += searched.iter().filter(|route| route.len() == 2 && route[0][0] != route[1][0] && route[0][1] != route[1][1]).count();

        // A straight line where it is cheaper, one connection after the other.
        for (link, theirs) in case.costs.as_ref().unwrap().iter().enumerate() {
            let what = format!("{what}, link {link} {:?}", case.links[link]);
            let (original, line) = straight_line_costs(&graph, &routes, link);
            assert_eq!(original, theirs[0], "{what}: price of the route {:?}", routes[link]);
            assert_eq!(line, theirs.get(1).copied(), "{what}: price of the straight line");
            links += 1;
            possible += usize::from(line.is_some());
            let before = routes[link].clone();
            straighten_one(&graph, &mut routes, link);
            straightened += usize::from(routes[link] != before);
        }
        assert_eq!(&pairs(&routes), case.straightened.as_ref().unwrap(), "{what}: after straightening");

        // The routes spread over the room they have.
        let before = routes.clone();
        balance(&graph, &mut routes);
        let theirs = case.balanced.as_ref().unwrap();
        for (link, (ours, theirs)) in pairs(&routes).iter().zip(theirs).enumerate() {
            assert_eq!(ours, theirs, "{what}, link {link} {:?}: after balancing, from {:?}", case.links[link], before[link]);
        }
        balanced += routes.iter().zip(&before).filter(|(after, before)| after != before).count();

        // And all steps in one go, as the editor asks for them. TALA's last two steps change
        // nothing for plain cards in these layouts.
        assert_eq!(case.reordered, case.balanced, "{what}: the reference changed after balancing");
        assert_eq!(&pairs(&route_edges(&graph).unwrap()), theirs, "{what}: all steps together");
    }
    // The cases must keep covering what is easy to lose.
    assert_eq!((links, possible, quick, straightened, balanced), (1828, 1541, 12, 54, 1315));
}
