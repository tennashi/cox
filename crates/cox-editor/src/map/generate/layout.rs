//! Where things sit on the map. A person places the levels the
//! vocabulary has; the map adds the lines the code gives between their
//! boxes, and swaps two boxes' places when that shortens those lines, so
//! that a box moves only to untangle them. What a level's lines reach
//! around its box, and the person did not place, the map places beside
//! the box, on the side it lies on in the level above. The types inside
//! a box are placed on a grid, each near what it is wired to, and pulled
//! toward the side of the box that the boxes it is wired to are on; those
//! wired to nothing after them, in name order.

use std::cmp::Ordering;
use std::collections::{BTreeMap, BTreeSet, HashMap};

use super::Map;
use crate::map::generated::{Level, Line};
use crate::map::vocabulary::{self, Side};

/// A card's width at least, and its height, as the page draws them.
const CARD_WIDTH: i32 = 216;
const CARD_HEIGHT: i32 = 56;
/// The card of a person or a system, as the page draws it.
const OUTSIDE_CARD: [i32; 2] = [248, 60];
/// How far from a level's box the page draws those around it that the
/// vocabulary places by side: beside it, and above or below it.
const BESIDE: i32 = 150;
const ABOVE: i32 = 90;
/// The room between two cards the map places around a level's box, and
/// between them and what the person placed there.
const BETWEEN: i32 = 18;
/// The grid the types of a box are placed on.
const COLUMN: i32 = 280;
const ROW: i32 = 110;
/// The room a generated level leaves around its types: at the sides, at
/// the top, under the level's name, and at the bottom.
const SIDE: i32 = 24;
const TOP: i32 = 56;
const BOTTOM: i32 = 28;
const MOST_COLUMNS: usize = 5;
/// The room between the types wired to something and those wired to
/// nothing, beyond a row's.
const APART: i32 = CARD_HEIGHT;
/// How many times the types are placed again, each nearer what it is
/// wired to.
const ROUNDS: usize = 12;
/// What a line no code backs weighs, against `1 + log10(references)` for
/// one the code gives: a person drew it, so it holds its ends together.
const HAND_WEIGHT: f64 = 6.0;

/// The levels of the vocabulary, with the lines the code gives, and
/// where each box of them sits now, by the level it is in; and how often
/// the code of each two boxes meets that a level draws a line between
/// for boxes inside them.
pub(super) struct Placed<'c> {
    pub levels: BTreeMap<String, Level>,
    pub slots: HashMap<&'c str, (&'c str, [i32; 2])>,
    pub standing: BTreeMap<(&'c str, &'c str), usize>,
}

/// Places the vocabulary's levels: each with its hand lines and, between
/// its boxes whose lines come from the code, a line for each two that
/// meet in it, all but each box's two heaviest weak; then swaps boxes
/// while that shortens the lines. A box drawn closed stands for the
/// boxes inside it, its lines for theirs.
///
/// A level that places none of those around it has the code's lines go
/// on to the boxes the levels above show closed. Every end of a level's
/// lines that the person did not place, the map places around its box.
pub(super) fn place<'c>(map: &Map<'c, '_>) -> Placed<'c> {
    let vocabulary = map.vocabulary;
    let mut levels = BTreeMap::new();
    let mut slots = HashMap::new();
    let mut found: BTreeMap<&str, BTreeMap<(&'c str, &'c str), usize>> = BTreeMap::new();
    // From the top down: where a box lies in a level above tells the
    // side it is placed on further down.
    let mut down: Vec<(&'c String, &'c vocabulary::Level)> = vocabulary.levels.iter().collect();
    down.sort_by_key(|(opened, _)| map.and_around(opened).len());
    for (opened, level) in down {
        let mut lines: Vec<Line> = level.lines.iter().map(super::hand_line).collect();
        let mut parts = level.parts.clone();
        if level.parts.keys().any(|id| vocabulary.generates(id)) {
            let (counts, directed) = meeting(map, opened, level);
            let edges: Vec<((&str, &str), usize)> =
                counts.iter().map(|(&pair, &count)| (pair, count)).collect();
            let shown: BTreeSet<&str> = edges.iter().flat_map(|((a, b), _)| [*a, *b]).collect();
            let mut strong = BTreeSet::new();
            for &node in &shown {
                let mut own: Vec<&((&str, &str), usize)> = edges
                    .iter()
                    .filter(|((a, b), _)| *a == node || *b == node)
                    .collect();
                own.sort_by(|x, y| y.1.cmp(&x.1).then(x.0.cmp(&y.0)));
                strong.extend(own.iter().take(2).map(|(pair, _)| *pair));
            }
            for &((a, b), _) in &edges {
                let forth = directed.get(&(a, b)).copied().unwrap_or(0);
                let back = directed.get(&(b, a)).copied().unwrap_or(0);
                let (from, to) = if forth >= back { (a, b) } else { (b, a) };
                lines.push(Line {
                    from: from.to_owned(),
                    to: to.to_owned(),
                    both: 4 * forth.min(back) >= forth.max(back),
                    sides: None,
                    weak: !strong.contains(&(a, b)),
                });
            }
            parts = swapped(&parts, &level.at, &lines, &counts, map);
            found.insert(opened.as_str(), counts);
        }
        for (part, &at) in &parts {
            if let Some((part, _)) = vocabulary.nodes.get_key_value(part) {
                slots.insert(part.as_str(), (opened.as_str(), at));
            }
        }
        let mut at = level.at.clone();
        at.extend(around(map, &slots, opened, level, &parts, &lines));
        levels.insert(
            opened.clone(),
            Level {
                area: Some(level.area),
                size: None,
                outer: level.outer.clone(),
                at,
                frames: level.frames.clone(),
                parts,
                lines,
            },
        );
    }
    let mut standing = BTreeMap::new();
    for counts in found.values() {
        for (&pair, &count) in counts {
            if !map.edges.contains_key(&pair) {
                standing.entry(pair).or_insert(count);
            }
        }
    }
    Placed {
        levels,
        slots,
        standing,
    }
}

/// How often the code of each two boxes the level inside `opened` shows
/// meets, as a pair in order, and from the first to the second, a box
/// drawn closed standing for the boxes inside it. It shows its parts and
/// what `level` places in `at`; where the person places none around it,
/// the boxes the levels above show closed, which are those beside a box
/// this one is in, each for where it meets a part.
#[expect(
    clippy::type_complexity,
    reason = "two maps of counts by a pair of boxes, read where they are made"
)]
fn meeting<'c>(
    map: &Map<'c, '_>,
    opened: &str,
    level: &vocabulary::Level,
) -> (
    BTreeMap<(&'c str, &'c str), usize>,
    HashMap<(&'c str, &'c str), usize>,
) {
    let vocabulary = map.vocabulary;
    let above = map.and_around(opened);
    let placed = |id: &str| level.parts.contains_key(id) || level.at.contains_key(id);
    let beside = |id: &str| {
        level.at.is_empty()
            && level.outer.is_empty()
            && !above.contains(&id)
            && map
                .inside_of
                .get(id)
                .is_some_and(|outer| above[1..].contains(outer))
    };
    let drawn = |id: &'c str| {
        map.shown_as(id, |at| placed(at) || beside(at))
            .filter(|&at| vocabulary.generates(at))
    };
    let meets = |a: &'c str, b: &'c str| match (drawn(a), drawn(b)) {
        (Some(x), Some(y)) if x != y && (placed(x) || placed(y)) => Some((x, y)),
        _ => None,
    };
    let mut counts: BTreeMap<(&'c str, &'c str), usize> = BTreeMap::new();
    for (&(a, b), &count) in &map.edges {
        if let Some((x, y)) = meets(a, b) {
            *counts
                .entry(if x < y { (x, y) } else { (y, x) })
                .or_default() += count;
        }
    }
    let mut directed: HashMap<(&'c str, &'c str), usize> = HashMap::new();
    for (&(a, b), &count) in &map.directed {
        if let Some(pair) = meets(a, b) {
            *directed.entry(pair).or_default() += count;
        }
    }
    (counts, directed)
}

/// Where the map places, around the box of the level inside `opened`,
/// each end of its `lines` that `level` does not place: beside the box,
/// past what the person placed there, on the side the end lies on in the
/// level above (see [`lies`]); those of one side in the order of the
/// `parts` their lines go to. Not the boxes `opened` is inside of, whose
/// edges are drawn, nor what is inside it.
fn around(
    map: &Map<'_, '_>,
    slots: &HashMap<&str, (&str, [i32; 2])>,
    opened: &str,
    level: &vocabulary::Level,
    parts: &BTreeMap<String, [i32; 2]>,
    lines: &[Line],
) -> BTreeMap<String, [i32; 2]> {
    let above = map.and_around(opened);
    let [left, top, width, height] = level.area;
    let middle = [left + width / 2, top + height / 2];
    let placed = |id: &str| {
        parts.contains_key(id)
            || level.at.contains_key(id)
            || level.outer.contains_key(id)
            || level.frames.contains_key(id)
    };
    // Each end to place, with the middles of what its lines go to.
    let mut ends: BTreeMap<&str, Vec<[i32; 2]>> = BTreeMap::new();
    for line in lines {
        for (end, other) in [(&line.from, &line.to), (&line.to, &line.from)] {
            if placed(end)
                || above.contains(&end.as_str())
                || map.and_around(end).contains(&opened)
                || !map.vocabulary.nodes.contains_key(end)
            {
                continue;
            }
            let to = parts.get(other).map_or(middle, |&[x, y]| {
                let [width, height] = card(map, other);
                [x + width / 2, y + height / 2]
            });
            ends.entry(end).or_default().push(to);
        }
    }
    // The edges of what the person placed, which the map places past.
    let mut edges = [
        left - BESIDE,
        top - ABOVE,
        left + width + BESIDE,
        top + height + ABOVE,
    ];
    let by_hand = level
        .at
        .iter()
        .map(|(id, &[x, y])| (card(map, id), [x, y]))
        .chain(
            level
                .outer
                .iter()
                .map(|(id, &vocabulary::Outer(side, along))| {
                    let [w, h] = card(map, id);
                    let at = match side {
                        Side::Left => [left - BESIDE - w, along],
                        Side::Right => [left + width + BESIDE, along],
                        Side::Top => [along, top - ABOVE - h],
                        Side::Bottom => [along, top + height + ABOVE],
                    };
                    ([w, h], at)
                }),
        );
    for ([w, h], [x, y]) in by_hand {
        edges = [
            edges[0].min(x - 2 * BETWEEN),
            edges[1].min(y - 2 * BETWEEN),
            edges[2].max(x + w + 2 * BETWEEN),
            edges[3].max(y + h + 2 * BETWEEN),
        ];
    }
    let mut sides: BTreeMap<Side, Vec<(i32, &str)>> = BTreeMap::new();
    for (&end, to) in &ends {
        let side = lies(map, slots, &above, end);
        let across = matches!(side, Side::Left | Side::Right);
        let toward = to.iter().map(|at| at[usize::from(across)]).sum::<i32>() / int(to.len());
        sides.entry(side).or_default().push((toward, end));
    }
    let mut at = BTreeMap::new();
    for (side, mut ends) in sides {
        ends.sort_unstable();
        let across = matches!(side, Side::Left | Side::Right);
        let long = |id: &str| card(map, id)[usize::from(across)];
        let all: i32 = ends.iter().map(|(_, id)| long(id) + BETWEEN).sum::<i32>() - BETWEEN;
        let mut along = middle[usize::from(across)] - all / 2;
        for (_, id) in ends {
            let [w, h] = card(map, id);
            let place = match side {
                Side::Left => [edges[0] - w, along],
                Side::Right => [edges[2], along],
                Side::Top => [along, edges[1] - h],
                Side::Bottom => [along, edges[3]],
            };
            at.insert(id.to_owned(), place);
            along += long(id) + BETWEEN;
        }
    }
    at
}

/// The side of a level's box that `node` lies on, `above` being the box
/// and those it is inside of, from the nearest out: where `node`, or the
/// box that stands for it, sits beside one of `above` in a level, the
/// side it is on from that one, as `slots` place them; a node of the
/// top, the side of the top it is on; anything else, below.
fn lies(
    map: &Map<'_, '_>,
    slots: &HashMap<&str, (&str, [i32; 2])>,
    above: &[&str],
    node: &str,
) -> Side {
    let top = &map.vocabulary.top;
    for id in map.and_around(node) {
        let Some(outer) = map.inside_of.get(id) else {
            return if top.left.iter().any(|left| left == id) {
                Side::Left
            } else if top.right.iter().any(|right| right == id) {
                Side::Right
            } else {
                Side::Bottom
            };
        };
        let from = above
            .iter()
            .find(|&&from| map.inside_of.get(from) == Some(outer));
        if let Some((&(_, there), &(_, here))) =
            from.and_then(|from| slots.get(id).zip(slots.get(from)))
        {
            let (dx, dy) = (there[0] - here[0], there[1] - here[1]);
            return if dx.abs() >= dy.abs() {
                if dx < 0 { Side::Left } else { Side::Right }
            } else if dy < 0 {
                Side::Top
            } else {
                Side::Bottom
            };
        }
    }
    Side::Bottom
}

/// How big the page draws the card of the node `id`.
fn card(map: &Map<'_, '_>, id: &str) -> [i32; 2] {
    match map.vocabulary.nodes.get(id) {
        Some(node) if node.kind == vocabulary::Kind::Outside => OUTSIDE_CARD,
        Some(node) => {
            let drawn = super::drawn(node);
            [
                card_width(&drawn.name, drawn.code.as_deref().unwrap_or("")),
                CARD_HEIGHT,
            ]
        }
        None => [CARD_WIDTH, CARD_HEIGHT],
    }
}

/// `parts`, with their places swapped, two at a time, the swap that
/// shortens the weighted length of the lines most each time, while one
/// does; `fixed` are the boxes around, which stay. A line weighs as often
/// as the code of its ends meets, as `counts` has it for the level, or
/// else as their own code does.
fn swapped(
    parts: &BTreeMap<String, [i32; 2]>,
    fixed: &BTreeMap<String, [i32; 2]>,
    lines: &[Line],
    counts: &BTreeMap<(&str, &str), usize>,
    map: &Map<'_, '_>,
) -> BTreeMap<String, [i32; 2]> {
    let mut weights: BTreeMap<&str, Vec<(&str, f64)>> = BTreeMap::new();
    for line in lines.iter().filter(|line| !line.weak) {
        let (a, b) = (line.from.as_str(), line.to.as_str());
        let pair = if a < b { (a, b) } else { (b, a) };
        let weight = counts
            .get(&pair)
            .or_else(|| map.edges.get(&pair))
            .map_or(HAND_WEIGHT, |&count| 1.0 + float(count).log10());
        weights.entry(a).or_default().push((b, weight));
        weights.entry(b).or_default().push((a, weight));
    }
    let cost = |at: &BTreeMap<String, [i32; 2]>| -> f64 {
        let mut total = 0.0;
        for (node, &here) in at {
            for &(other, weight) in weights.get(node.as_str()).into_iter().flatten() {
                let inside = at.get(other);
                if let Some(&there) = inside.or_else(|| fixed.get(other))
                    && (inside.is_none() || node.as_str() < other)
                {
                    total += weight * distance(here, there);
                }
            }
        }
        total
    };
    let mut current = parts.clone();
    let mut best = cost(&current);
    let names: Vec<&String> = parts.keys().collect();
    loop {
        let mut gain: Option<(f64, BTreeMap<String, [i32; 2]>)> = None;
        for (index, a) in names.iter().enumerate() {
            for b in &names[index + 1..] {
                let mut tried = current.clone();
                tried.insert((*a).clone(), current[*b]);
                tried.insert((*b).clone(), current[*a]);
                let cost = cost(&tried);
                if cost < best - 1e-6 && gain.as_ref().is_none_or(|(most, _)| cost < *most) {
                    gain = Some((cost, tried));
                }
            }
        }
        let Some((cost, tried)) = gain else {
            return current;
        };
        best = cost;
        current = tried;
    }
}

/// A type, as its card shows it.
pub(super) struct Card<'t> {
    pub id: &'t str,
    pub name: &'t str,
    pub code: &'t str,
}

/// Places the `cards` of a box sitting at `me`, with `lines` between
/// them and to the boxes `around`. Those a line ends on sit together, on
/// a grid sized by their own count. Those wired to nothing
/// follow below, a little apart, in rows as wide as the grid of all the
/// cards would be, in name order, so that they read as a list and leave
/// the others where they are. Gives the level's size, and where each
/// card sits, from its top left.
pub(super) fn types(
    cards: &[Card<'_>],
    lines: &BTreeSet<(String, String)>,
    around: &BTreeMap<&str, [i32; 2]>,
    me: [i32; 2],
) -> ([i32; 2], BTreeMap<String, [i32; 2]>) {
    let ends: BTreeSet<&str> = lines
        .iter()
        .flat_map(|(a, b)| [a.as_str(), b.as_str()])
        .collect();
    let (wired, mut loose): (Vec<&Card>, Vec<&Card>) =
        cards.iter().partition(|card| ends.contains(card.id));
    let grid = wired_grid(&wired, lines, around, me);
    let mut parts: BTreeMap<String, [i32; 2]> = grid
        .iter()
        .map(|(&id, &(column, row))| {
            (
                id.to_owned(),
                [SIDE + int(column) * COLUMN, TOP + int(row) * ROW],
            )
        })
        .collect();
    let below = grid
        .values()
        .map(|&(_, row)| row + 1)
        .max()
        .map_or(TOP, |rows| TOP + int(rows) * ROW + APART);
    let columns = columns(cards.len());
    loose.sort_by(|a, b| a.name.cmp(b.name).then(a.id.cmp(b.id)));
    for (index, card) in loose.iter().enumerate() {
        parts.insert(
            card.id.to_owned(),
            [
                SIDE + int(index % columns) * COLUMN,
                below + int(index / columns) * ROW,
            ],
        );
    }
    let right = cards
        .iter()
        .map(|card| parts[card.id][0] + card_width(card.name, card.code))
        .max()
        .unwrap_or(0);
    let bottom = parts.values().map(|at| at[1]).max().unwrap_or(0);
    ([right + SIDE, bottom + CARD_HEIGHT + BOTTOM], parts)
}

/// How many columns a grid of `count` cards has.
fn columns(count: usize) -> usize {
    whole((float(count) * 1.3).sqrt().ceil()).clamp(2, MOST_COLUMNS)
}

/// Where each of the `cards` that a line ends on sits, by column and
/// row, on a grid of their own: placed again each round in the order of
/// where each wants to be, the mean of what it is wired to, another box
/// being where it lies as seen from this one.
fn wired_grid<'t>(
    cards: &[&Card<'t>],
    lines: &BTreeSet<(String, String)>,
    around: &BTreeMap<&str, [i32; 2]>,
    me: [i32; 2],
) -> BTreeMap<&'t str, (usize, usize)> {
    let count = cards.len();
    let columns = columns(count);
    let rows = count.div_ceil(columns);
    let (width, height) = (
        float(columns) * f64::from(COLUMN),
        float(rows) * f64::from(ROW),
    );
    let pull: BTreeMap<&str, (f64, f64)> = around
        .iter()
        .map(|(&id, &at)| {
            let (dx, dy) = (f64::from(at[0] - me[0]), f64::from(at[1] - me[1]));
            let length = match (dx * dx + dy * dy).sqrt() {
                0.0 => 1.0,
                length => length,
            };
            (
                id,
                (
                    width / 2.0 + dx / length * width,
                    height / 2.0 + dy / length * height,
                ),
            )
        })
        .collect();
    // Where a card in a cell is pulled from: its middle.
    let cell = |column: usize, row: usize| {
        (
            f64::from(int(column) * COLUMN + CARD_WIDTH / 2),
            f64::from(int(row) * ROW + CARD_HEIGHT / 2),
        )
    };
    let mut by_name: Vec<&Card> = cards.to_vec();
    by_name.sort_by(|a, b| a.name.cmp(b.name).then(a.id.cmp(b.id)));
    let mut at: BTreeMap<&str, (f64, f64)> = by_name
        .iter()
        .enumerate()
        .map(|(index, card)| (card.id, cell(index % columns, index / columns)))
        .collect();
    let mut grid: BTreeMap<&'t str, (usize, usize)> = BTreeMap::new();
    for _ in 0..ROUNDS {
        let mut want: BTreeMap<&str, (f64, f64)> = BTreeMap::new();
        for card in cards {
            let mut points: Vec<(f64, f64)> = Vec::new();
            for (a, b) in lines {
                let other = if a == card.id {
                    b.as_str()
                } else if b == card.id {
                    a.as_str()
                } else {
                    continue;
                };
                points.extend(at.get(other).copied());
            }
            for (a, b) in lines {
                let other = if a == card.id {
                    b.as_str()
                } else if b == card.id {
                    a.as_str()
                } else {
                    continue;
                };
                points.extend(pull.get(other).copied());
            }
            let mean = if points.is_empty() {
                at[card.id]
            } else {
                let n = float(points.len());
                (
                    points.iter().map(|point| point.0).sum::<f64>() / n,
                    points.iter().map(|point| point.1).sum::<f64>() / n,
                )
            };
            want.insert(card.id, mean);
        }
        let mut by_x: Vec<&'t str> = cards.iter().map(|card| card.id).collect();
        by_x.sort_by(|a, b| wanted(&want, a, b, |point| point.0));
        for (column, chunk) in by_x.chunks(rows.max(1)).enumerate() {
            let mut chunk = chunk.to_vec();
            chunk.sort_by(|a, b| wanted(&want, a, b, |point| point.1));
            for (row, id) in chunk.into_iter().enumerate() {
                at.insert(id, cell(column, row));
                grid.insert(id, (column, row));
            }
        }
    }
    grid
}

/// `a` before `b` if it wants to be further up or left, as `pick` reads
/// where it wants to be, or else by id.
fn wanted(
    want: &BTreeMap<&str, (f64, f64)>,
    a: &str,
    b: &str,
    pick: fn(&(f64, f64)) -> f64,
) -> Ordering {
    pick(&want[a])
        .partial_cmp(&pick(&want[b]))
        .unwrap_or(Ordering::Equal)
        .then(a.cmp(b))
}

/// How wide the page draws a card with `name` over `code`: by the
/// letters' widths, a wide letter's 14.5 and an ASCII one's 0.6 of it.
fn card_width(name: &str, code: &str) -> i32 {
    let letters = |text: &str| -> f64 {
        text.chars()
            .map(|c| {
                if (' '..='~').contains(&c) {
                    0.6 * 14.5
                } else {
                    14.5
                }
            })
            .fold(0.0, |sum, width| sum + width)
    };
    let text = letters(name).max(letters(code) * 0.8);
    CARD_WIDTH.max(int(whole((58.0 + text + 18.0).ceil())))
}

fn distance(a: [i32; 2], b: [i32; 2]) -> f64 {
    let (dx, dy) = (f64::from(a[0] - b[0]), f64::from(a[1] - b[1]));
    (dx * dx + dy * dy).sqrt()
}

#[expect(
    clippy::cast_precision_loss,
    reason = "counts of the code's things, far below 2^52"
)]
pub(super) fn float(count: usize) -> f64 {
    count as f64
}

#[expect(
    clippy::cast_possible_truncation,
    clippy::cast_sign_loss,
    reason = "a whole number of columns or pixels, small and not negative"
)]
fn whole(value: f64) -> usize {
    value as usize
}

fn int(value: usize) -> i32 {
    i32::try_from(value).unwrap_or(i32::MAX)
}
