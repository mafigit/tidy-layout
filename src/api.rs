// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.

//! What the editor asks the engine, as JSON. This file is ours, not a port: it decides how the
//! ported pieces are used.

use crate::graph::{Edge, Graph, Node, Point, compute_cell_size};
use crate::pipeline::{Drawing, SEEDS, layout};
use crate::place::prescale;
use crate::placement::{closest_unoccupied, does_overlap, fill_placement_points, move_node_to_best, round_to_cell};
use crate::placementcost::edge_length::SIDE_EDGE_SPACING;
use crate::policy::LABEL_PADDING;
use crate::routing::coordinator::route_edges_with_labels;
use crate::routing::labels;
use crate::routing::router::Costs;
use crate::score::{Score, best_seed, evaluate};
use serde::{Deserialize, Serialize};

#[derive(Deserialize)]
pub struct Card {
    pub id: String,
    pub x: f64,
    pub y: f64,
    pub width: f64,
    pub height: f64,
}

#[derive(Deserialize)]
pub struct Connection {
    pub from: String,
    pub to: String,
    /// The size of the label drawn on the connection, if it has one.
    pub label: Option<LabelSize>,
}

#[derive(Deserialize, Clone, Copy)]
pub struct LabelSize {
    pub width: f64,
    pub height: f64,
}

#[derive(Deserialize, Serialize, Clone, Copy, Debug, PartialEq)]
pub struct Position {
    pub x: f64,
    pub y: f64,
}

/// "Where does this card go if it is dropped here?"
#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SnapRequest {
    /// Every card on the diagram, the moving one included, where it is now.
    pub cards: Vec<Card>,
    pub connections: Vec<Connection>,
    /// Id of the card being moved.
    pub moving: String,
    /// Where its top-left corner is wanted.
    pub wanted: Position,
    /// Step of the grid places lie on. Without it, TALA's own: about the size of a card.
    pub cell_size: Option<f64>,
}

#[derive(Serialize, Debug, PartialEq)]
pub struct SnapResponse {
    /// The top-left corner the card gets: on the grid, and keeping its gap to every other card.
    pub x: f64,
    pub y: f64,
    /// How many grid rings away from the wanted place that is; 0 is the wanted place itself.
    pub ring: u32,
}

/// A place for the card where it keeps its gap to all others: 20 to cards it is not connected
/// to, 60 to connected ones, and more where a label between two cards needs the room: its width
/// when they sit side by side, its height when one is above the other.
///
/// The wanted place itself is taken when the card fits there, also when it is the card's own
/// place. (TALA's search counts a card's own exact position as taken; for a drop that would
/// push a card away from where it already is.)
///
/// Otherwise a card without connections gets the nearest free place. A card with connections
/// makes one move of TALA's optimizer around the wanted place: of the free places near it, it
/// takes the one where its connections are shortest and straightest ([`move_node_to_best`]).
/// TALA looks one ring of its grid further than the nearest free place. On a finer grid the
/// card looks 40 further: up to there TALA calls two sides in line (`SideEdgeSpacing`), so a
/// card dropped a little out of line lands in line, and never far from where it was dropped.
///
/// A connection from a card to itself has no say in any of this.
pub fn snap(request: &SnapRequest) -> Result<SnapResponse, String> {
    let index = |id: &str| {
        request.cards.iter().position(|c| c.id == id).ok_or_else(|| format!("unknown card: {id}"))
    };
    check_cards(&request.cards)?;
    if !request.wanted.x.is_finite() || !request.wanted.y.is_finite() {
        return Err("the wanted place must be finite".into());
    }
    let moving = index(&request.moving)?;
    let nodes: Vec<Node> = request
        .cards
        .iter()
        .map(|c| Node { top_left: Point { x: c.x, y: c.y }, width: c.width, height: c.height })
        .collect();
    let mut edges = Vec::with_capacity(request.connections.len());
    for connection in &request.connections {
        // TALA's adapter: the label's box with padding on both sides, in whole pixels.
        let room = |size: f64| if size.is_finite() && size > 0.0 { size.ceil() + 2.0 * LABEL_PADDING } else { 0.0 };
        let (min_width, min_height) =
            connection.label.map_or((0.0, 0.0), |label| (room(label.width), room(label.height)));
        let (from, to) = (index(&connection.from)?, index(&connection.to)?);
        if from != to {
            edges.push(Edge { from, to, min_width, min_height });
        }
    }
    let cell_size = match request.cell_size {
        Some(cell) if cell.is_finite() && cell >= 1.0 => cell,
        Some(_) => return Err("the cell size must be 1 or more".into()),
        None => compute_cell_size(&nodes),
    };
    let graph = Graph { cell_size, ..Graph::new(nodes, edges) };

    let wanted = Point { x: request.wanted.x, y: request.wanted.y };
    let on_grid = Point { x: round_to_cell(wanted.x, cell_size), y: round_to_cell(wanted.y, cell_size) };
    if !does_overlap(&graph, moving, on_grid) {
        return Ok(SnapResponse { x: on_grid.x, y: on_grid.y, ring: 0 });
    }
    let nearest = closest_unoccupied(&graph, moving, wanted).ok_or("no free place near the wanted one")?;
    let mut place = nearest.top_left;
    if graph.edges_of(moving).next().is_some() {
        let further = (SIDE_EDGE_SPACING / cell_size).ceil() as u32;
        let mut points = fill_placement_points(&graph, moving, wanted, nearest.ring + further - 1, false);
        // TALA lets a card stay where it is, also where it lies too close to another card.
        points.retain(|point| !does_overlap(&graph, moving, *point));
        // TALA's prices are tuned to its own cell, about the size of a card.
        let mut priced = Graph { cell_size: compute_cell_size(&graph.nodes), ..graph };
        let turn = Costs::of(&priced).turn;
        place = move_node_to_best(&mut priced, moving, &points, false, turn).unwrap_or(place);
    }
    let rings = ((place.x - on_grid.x).abs() + (place.y - on_grid.y).abs()) / cell_size;
    Ok(SnapResponse { x: place.x, y: place.y, ring: rings.round() as u32 })
}

/// A box with its sides level and upright: a card or a label.
#[derive(Clone, Copy)]
struct Area {
    left: f64,
    top: f64,
    right: f64,
    bottom: f64,
}

impl Area {
    fn of_label(place: Point, size: labels::LabelSize) -> Area {
        Area { left: place.x, top: place.y, right: place.x + size.width, bottom: place.y + size.height }
    }

    /// Whether the two share more than `slack` in both directions.
    fn overlaps(&self, other: &Area, slack: f64) -> bool {
        self.left < other.right - slack && other.left < self.right - slack && self.top < other.bottom - slack && other.top < self.bottom - slack
    }

    /// Whether the piece of a route from `a` to `b` (level or upright) passes through the box,
    /// more than `slack` inside it.
    fn crossed_by(&self, a: Point, b: Point, slack: f64) -> bool {
        let line = Area { left: a.x.min(b.x), top: a.y.min(b.y), right: a.x.max(b.x), bottom: a.y.max(b.y) };
        line.left < self.right - slack && line.right > self.left + slack && line.top < self.bottom - slack && line.bottom > self.top + slack
    }
}

/// How far a label's box keeps from its line: half the line's thickness and the padding, as
/// the ported placement keeps it (`routing::labels`).
const LABEL_OFF_LINE: f64 = 1.5 + LABEL_PADDING;
/// A label on a line counts as crossed when the line passes more than this inside its box.
const CROSSED_SLACK: f64 = 1.0;
/// How far apart the places are that a label is tried at along its line.
const LABEL_STEP: f64 = 4.0;

/// Ours, not TALA's: no label stays under the line of a connection. TALA's placement takes the
/// place that covers the least, and where every place it tries covers something it leaves the
/// label on another connection's line. Such a label moves to the nearest place beside its own
/// line, on either side of any of its pieces, that lies on no card, on no other label and
/// under no line. Where there is none it stays. The routes do not change.
fn uncross_labels(cards: &[Node], routes: &[Vec<Point>], sizes: &[Option<labels::LabelSize>], places: &mut [Option<Point>]) {
    let card_areas: Vec<Area> = cards
        .iter()
        .map(|card| Area { left: card.top_left.x, top: card.top_left.y, right: card.top_left.x + card.width, bottom: card.top_left.y + card.height })
        .collect();
    let under_a_line =
        |area: &Area, slack: f64| routes.iter().any(|route| route.windows(2).any(|piece| area.crossed_by(piece[0], piece[1], slack)));
    for label in 0..places.len() {
        let (Some(place), Some(size)) = (places[label], sizes[label]) else { continue };
        if !under_a_line(&Area::of_label(place, size), CROSSED_SLACK) {
            continue;
        }
        let others: Vec<Area> = (0..places.len())
            .filter(|other| *other != label)
            .filter_map(|other| Some(Area::of_label(places[other]?, sizes[other]?)))
            .collect();
        let free = |at: Point| {
            let area = Area::of_label(at, size);
            !under_a_line(&area, 0.0)
                && card_areas.iter().all(|card| !area.overlaps(card, 0.0))
                && others.iter().all(|other| !area.overlaps(other, 0.0))
        };
        let mut best: Option<(f64, Point)> = None;
        for piece in routes[label].windows(2) {
            let (a, b) = (piece[0], piece[1]);
            let level = a.y == b.y;
            // Along the piece: the label may reach past either end by half its length, so that
            // its middle is still beside the piece. That also gives a short piece its places.
            let (from, to, length) = if level { (a.x.min(b.x), a.x.max(b.x), size.width) } else { (a.y.min(b.y), a.y.max(b.y), size.height) };
            let mut along = Vec::new();
            let mut at = from - length / 2.0;
            while at <= to - length / 2.0 {
                along.push(at);
                at += LABEL_STEP;
            }
            for at in along {
                let sides = if level {
                    [Point { x: at, y: a.y - LABEL_OFF_LINE - size.height }, Point { x: at, y: a.y + LABEL_OFF_LINE }]
                } else {
                    [Point { x: a.x - LABEL_OFF_LINE - size.width, y: at }, Point { x: a.x + LABEL_OFF_LINE, y: at }]
                };
                for corner in sides {
                    // Whole numbers, as the ported places are.
                    let corner = Point { x: corner.x.round(), y: corner.y.round() };
                    let moved = (corner.x - place.x).hypot(corner.y - place.y);
                    // The first of equally near places stays: the answer never depends on chance.
                    if best.is_none_or(|(least, _)| moved < least) && free(corner) {
                        best = Some((moved, corner));
                    }
                }
            }
        }
        if let Some((_, corner)) = best {
            places[label] = Some(corner);
        }
    }
}

/// "How do the connectors run?"
#[derive(Deserialize)]
pub struct RouteRequest {
    /// Every card on the diagram, where it is.
    pub cards: Vec<Card>,
    pub connections: Vec<Connection>,
}

#[derive(Serialize, Debug, PartialEq)]
pub struct RouteResponse {
    /// Per connection, in the order of the request: the points its connector runs through, from
    /// the border of its first card to the border of its second. Empty for a connection from a
    /// card to itself.
    pub routes: Vec<Vec<Position>>,
    /// Per connection: the top-left corner of its label's box, for a connection with a label.
    pub labels: Vec<Option<Position>>,
}

fn check_cards(cards: &[Card]) -> Result<(), String> {
    for card in cards {
        let positive = card.width > 0.0 && card.height > 0.0;
        if ![card.x, card.y, card.width, card.height].into_iter().all(f64::is_finite) || !positive {
            return Err(format!("card {} needs a finite position and a positive size", card.id));
        }
    }
    Ok(())
}

/// The connectors of a diagram, as TALA's router lays them: around all cards, each from the
/// border of its first card to the border of its second, with as few turns and crossings as it
/// finds, and spread over the room between the cards. Every piece is upright or level: the
/// slanted line TALA draws where the way around is several times as long is turned off.
///
/// A connection with a label also gets the place for it: beside the connector where there is
/// room, where it covers the least otherwise, and clear of the labels placed before it.
///
/// The router expects what the editor guarantees: cards that neither overlap nor touch. Such a
/// request is refused instead of answered with routes nobody checked.
pub fn route(request: &RouteRequest) -> Result<RouteResponse, String> {
    check_cards(&request.cards)?;
    let index = |id: &str| {
        request.cards.iter().position(|c| c.id == id).ok_or_else(|| format!("unknown card: {id}"))
    };
    let nodes: Vec<Node> = request
        .cards
        .iter()
        .map(|c| Node { top_left: Point { x: c.x, y: c.y }, width: c.width, height: c.height })
        .collect();
    for (a, first) in nodes.iter().enumerate() {
        for (b, second) in nodes.iter().enumerate().skip(a + 1) {
            if first.distance_to(second) <= 0.0 {
                return Err(format!("cards {} and {} overlap or touch", request.cards[a].id, request.cards[b].id));
            }
        }
    }
    // The router gets the connections between two different cards; the answer keeps the order
    // and the number of the request.
    let mut edges = Vec::new();
    let mut sizes = Vec::new();
    let mut edge_of: Vec<Option<usize>> = Vec::with_capacity(request.connections.len());
    for connection in &request.connections {
        let (from, to) = (index(&connection.from)?, index(&connection.to)?);
        if from == to {
            edge_of.push(None);
        } else {
            edge_of.push(Some(edges.len()));
            edges.push(Edge::plain(from, to));
            sizes.push(connection.label.and_then(|label| {
                let usable = label.width.is_finite() && label.height.is_finite() && label.width > 0.0 && label.height > 0.0;
                usable.then_some(labels::LabelSize { width: label.width, height: label.height })
            }));
        }
    }
    let graph = Graph { slanted_lines: false, ..Graph::new(nodes, edges) };
    let routing = route_edges_with_labels(&graph, &sizes).map_err(|error| format!("no route found: {error:?}"))?;
    let kept = routing.routes;
    let mut places: Vec<Option<Point>> = routing.labels.iter().map(|place| place.map(|place| place.top_left)).collect();
    uncross_labels(&graph.nodes, &kept, &sizes, &mut places);
    let routes = edge_of
        .iter()
        .map(|edge| match edge {
            Some(edge) => kept[*edge].iter().map(|p| Position { x: p.x, y: p.y }).collect(),
            None => Vec::new(),
        })
        .collect();
    let labels = edge_of
        .iter()
        .map(|edge| edge.and_then(|edge| places[edge]).map(|place| Position { x: place.x, y: place.y }))
        .collect();
    Ok(RouteResponse { routes, labels })
}

/// "Arrange the diagram."
#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ArrangeRequest {
    /// Every card on the diagram, where it is now. Cards may overlap: they are being moved.
    pub cards: Vec<Card>,
    pub connections: Vec<Connection>,
}

#[derive(Serialize, Debug, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct ArrangedCard {
    pub id: String,
    /// The top-left corner the card gets.
    pub x: f64,
    pub y: f64,
    /// The size the card gets: its own, larger for a card with many connections.
    pub width: f64,
    pub height: f64,
}

#[derive(Serialize, Debug, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct ArrangeResponse {
    /// Every card, in the order of the request.
    pub cards: Vec<ArrangedCard>,
    /// The seed of TALA's pipeline the places come from.
    pub seed: i64,
}

/// The gap the editor keeps between two cards.
pub const CARD_GAP: f64 = 20.0;

/// The gap between two boxes as the editor measures it: the larger of the gap beside and the
/// gap above or below. Negative where they overlap.
fn gap_between(a: &Node, b: &Node) -> f64 {
    let beside = (b.top_left.x - a.right()).max(a.top_left.x - b.right());
    let above = (b.top_left.y - a.bottom()).max(a.top_left.y - b.bottom());
    beside.max(above)
}

/// The smallest gap between any two of the cards; infinite for fewer than two.
fn smallest_gap(cards: &[Node]) -> f64 {
    let mut smallest = f64::INFINITY;
    for (a, first) in cards.iter().enumerate() {
        for second in &cards[a + 1..] {
            smallest = smallest.min(gap_between(first, second));
        }
    }
    smallest
}

/// Of the drawings of several seeds, each with its score and the smallest gap between two of
/// its cards: TALA's best (`score::best_seed`) among those that keep [`CARD_GAP`]. `None` if
/// none keeps it.
pub fn pick_seed(drawings: &[(Score, f64)]) -> Option<usize> {
    let keeping: Vec<usize> = (0..drawings.len()).filter(|seed| drawings[*seed].1 >= CARD_GAP).collect();
    let scores: Vec<Score> = keeping.iter().map(|seed| drawings[*seed].0).collect();
    best_seed(&scores).map(|best| keeping[best])
}

/// Where each card of the request lies in a drawing, with the size it is given: in the order
/// of the request.
fn arranged_boxes(drawing: &Drawing, sizes: &[Node]) -> Result<Vec<Node>, String> {
    let mut boxes: Vec<Option<Node>> = vec![None; sizes.len()];
    for (node, card) in drawing.graph.nodes.iter().enumerate() {
        let id = usize::try_from(drawing.graph.id(node)).ok().filter(|id| *id < sizes.len());
        let id = id.ok_or("the drawing has a card the request has not")?;
        boxes[id] = Some(Node { top_left: card.top_left, ..sizes[id].clone() });
    }
    boxes.into_iter().map(|card| card.ok_or_else(|| "the drawing lost a card".to_string())).collect()
}

/// The diagram as TALA arranges it: TALA's pipeline for seeds 1, 2 and 3, and of the drawings
/// the best by TALA's score among those that keep every two cards 20 apart (TALA's best can
/// leave two cards closer). An error if no seed keeps that gap.
///
/// A card keeps its own size, except that a card with many connections grows as TALA makes it
/// grow. TALA gives all cards of a cluster the size of the largest; here each keeps its own,
/// at the top-left corner TALA gives it.
///
/// The arranged diagram starts where the request's started: the top-left corner of the box
/// around all cards stays where it was. A connection from a card to itself has no say.
pub fn arrange(request: &ArrangeRequest) -> Result<ArrangeResponse, String> {
    check_cards(&request.cards)?;
    if request.cards.is_empty() {
        return Ok(ArrangeResponse { cards: Vec::new(), seed: SEEDS[0] });
    }
    let index = |id: &str| {
        request.cards.iter().position(|c| c.id == id).ok_or_else(|| format!("unknown card: {id}"))
    };
    let nodes: Vec<Node> = request
        .cards
        .iter()
        .map(|c| Node { top_left: Point { x: c.x, y: c.y }, width: c.width, height: c.height })
        .collect();
    let mut edges = Vec::with_capacity(request.connections.len());
    for connection in &request.connections {
        let (from, to) = (index(&connection.from)?, index(&connection.to)?);
        if from == to {
            continue;
        }
        // The pipeline reads a label from the connection's least room: the label's size.
        let (min_width, min_height) = connection
            .label
            .filter(|label| label.width.is_finite() && label.height.is_finite() && label.width > 0.0 && label.height > 0.0)
            .map_or((0.0, 0.0), |label| (label.width, label.height));
        edges.push(Edge { from, to, min_width, min_height });
    }
    let given = Graph::new(nodes, edges);
    let mut sizes = given.clone();
    prescale(&mut sizes);

    let mut drawings = Vec::new();
    let mut first_error = None;
    for seed in SEEDS {
        match layout(&given, seed) {
            Ok(drawing) => {
                let score = evaluate(&drawing.graph, &drawing.routes, &drawing.labels);
                let boxes = arranged_boxes(&drawing, &sizes.nodes)?;
                drawings.push((seed, score, boxes));
            }
            Err(error) => {
                first_error.get_or_insert(error);
            }
        }
    }
    if drawings.is_empty() {
        return Err(format!("no drawing found: {:?}", first_error.expect("at least one seed")));
    }
    let measured: Vec<(Score, f64)> = drawings.iter().map(|(_, score, boxes)| (*score, smallest_gap(boxes))).collect();
    let best = pick_seed(&measured).ok_or(format!("no seed keeps every two cards {CARD_GAP} apart"))?;
    let (seed, _, boxes) = &drawings[best];

    let corner = |cards: &mut dyn Iterator<Item = Point>| {
        cards.fold(Point { x: f64::INFINITY, y: f64::INFINITY }, |low, p| Point { x: low.x.min(p.x), y: low.y.min(p.y) })
    };
    let was = corner(&mut request.cards.iter().map(|c| Point { x: c.x, y: c.y }));
    let now = corner(&mut boxes.iter().map(|card| card.top_left));
    let cards = request
        .cards
        .iter()
        .zip(boxes)
        .map(|(card, arranged)| ArrangedCard {
            id: card.id.clone(),
            x: arranged.top_left.x - now.x + was.x,
            y: arranged.top_left.y - now.y + was.y,
            width: arranged.width,
            height: arranged.height,
        })
        .collect();
    Ok(ArrangeResponse { cards, seed: *seed })
}

#[derive(Serialize)]
struct Failure {
    error: String,
}

/// [`route`] from and to JSON. A failure is `{"error": "..."}`.
pub fn route_json(input: &str) -> String {
    let answer = serde_json::from_str::<RouteRequest>(input)
        .map_err(|e| format!("not a route request: {e}"))
        .and_then(|request| route(&request));
    match answer {
        Ok(response) => serde_json::to_string(&response),
        Err(error) => serde_json::to_string(&Failure { error }),
    }
    .expect("the answer is plain data")
}

/// [`snap`] from and to JSON. A failure is `{"error": "..."}`.
pub fn snap_json(input: &str) -> String {
    let answer = serde_json::from_str::<SnapRequest>(input)
        .map_err(|e| format!("not a snap request: {e}"))
        .and_then(|request| snap(&request));
    match answer {
        Ok(response) => serde_json::to_string(&response),
        Err(error) => serde_json::to_string(&Failure { error }),
    }
    .expect("the answer is plain data")
}

/// [`arrange`] from and to JSON. A failure is `{"error": "..."}`.
pub fn arrange_json(input: &str) -> String {
    let answer = serde_json::from_str::<ArrangeRequest>(input)
        .map_err(|e| format!("not an arrange request: {e}"))
        .and_then(|request| arrange(&request));
    match answer {
        Ok(response) => serde_json::to_string(&response),
        Err(error) => serde_json::to_string(&Failure { error }),
    }
    .expect("the answer is plain data")
}
