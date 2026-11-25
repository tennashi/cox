//! Where things sit on the map. A person places the levels the
//! vocabulary has; the map adds the lines the code gives between their
//! boxes, and swaps two boxes' places when that shortens those lines, so
//! that a box moves only to untangle them. The types inside a box are
//! placed on a grid, each near what it is wired to, and pulled toward
//! the side of the box that the boxes it is wired to are on.

use std::cmp::Ordering;
use std::collections::{BTreeMap, BTreeSet, HashMap};

use super::Map;
use crate::map::generated::{Level, Line};

/// A card's width at least, and its height, as the page draws them.
const CARD_WIDTH: i32 = 216;
const CARD_HEIGHT: i32 = 56;
/// The grid the types of a box are placed on.
const COLUMN: i32 = 280;
const ROW: i32 = 110;
/// The room a generated level leaves around its types: at the sides, at
/// the top, under the level's name, and at the bottom.
const SIDE: i32 = 24;
const TOP: i32 = 56;
const BOTTOM: i32 = 28;
const MOST_COLUMNS: usize = 5;
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
pub(super) fn place<'c>(map: &Map<'c, '_>) -> Placed<'c> {
    let vocabulary = map.vocabulary;
    let mut levels = BTreeMap::new();
    let mut slots = HashMap::new();
    let mut standing = BTreeMap::new();
    for (opened, level) in &vocabulary.levels {
        let mut lines: Vec<Line> = level.lines.iter().map(super::hand_line).collect();
        let mut parts = level.parts.clone();
        if level.parts.keys().any(|id| vocabulary.generates(id)) {
            let shown: BTreeSet<&str> = level
                .parts
                .keys()
                .chain(level.at.keys())
                .map(String::as_str)
                .collect();
            let drawn = |id: &'c str| {
                map.shown_as(id, |at| shown.contains(at))
                    .filter(|&at| vocabulary.generates(at))
            };
            let mut counts: BTreeMap<(&'c str, &'c str), usize> = BTreeMap::new();
            for (&(a, b), &count) in &map.edges {
                if let (Some(x), Some(y)) = (drawn(a), drawn(b))
                    && x != y
                {
                    *counts
                        .entry(if x < y { (x, y) } else { (y, x) })
                        .or_default() += count;
                }
            }
            let mut directed: HashMap<(&str, &str), usize> = HashMap::new();
            for (&(a, b), &count) in &map.directed {
                if let (Some(x), Some(y)) = (drawn(a), drawn(b))
                    && x != y
                {
                    *directed.entry((x, y)).or_default() += count;
                }
            }
            for (&pair, &count) in &counts {
                if !map.edges.contains_key(&pair) {
                    standing.entry(pair).or_insert(count);
                }
            }
            let edges: Vec<((&str, &str), usize)> =
                counts.iter().map(|(&pair, &count)| (pair, count)).collect();
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
        }
        for (part, &at) in &parts {
            if let Some((part, _)) = vocabulary.nodes.get_key_value(part) {
                slots.insert(part.as_str(), (opened.as_str(), at));
            }
        }
        levels.insert(
            opened.clone(),
            Level {
                area: Some(level.area),
                size: None,
                outer: level.outer.clone(),
                at: level.at.clone(),
                frames: level.frames.clone(),
                parts,
                lines,
            },
        );
    }
    Placed {
        levels,
        slots,
        standing,
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
/// them and to the boxes `around`: on a grid, placed again each round in
/// the order of where each wants to be, the mean of what it is wired to,
/// another box being where it lies as seen from this one. Gives the
/// level's size, and where each card sits, from its top left.
pub(super) fn types(
    cards: &[Card<'_>],
    lines: &BTreeSet<(String, String)>,
    around: &BTreeMap<&str, [i32; 2]>,
    me: [i32; 2],
) -> ([i32; 2], BTreeMap<String, [i32; 2]>) {
    let count = cards.len();
    let columns = whole((float(count) * 1.3).sqrt().ceil()).clamp(2, MOST_COLUMNS);
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
    let mut by_name: Vec<&Card> = cards.iter().collect();
    by_name.sort_by(|a, b| a.name.cmp(b.name).then(a.id.cmp(b.id)));
    let mut at: BTreeMap<&str, (f64, f64)> = by_name
        .iter()
        .enumerate()
        .map(|(index, card)| (card.id, cell(index % columns, index / columns)))
        .collect();
    let mut grid: BTreeMap<&str, (usize, usize)> = BTreeMap::new();
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
        let mut by_x: Vec<&str> = cards.iter().map(|card| card.id).collect();
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
    let parts: BTreeMap<String, [i32; 2]> = grid
        .iter()
        .map(|(&id, &(column, row))| {
            (
                id.to_owned(),
                [SIDE + int(column) * COLUMN, TOP + int(row) * ROW],
            )
        })
        .collect();
    let right = cards
        .iter()
        .map(|card| parts[card.id][0] + card_width(card.name, card.code))
        .max()
        .unwrap_or(0);
    let bottom = parts.values().map(|at| at[1]).max().unwrap_or(0);
    ([right + SIDE, bottom + CARD_HEIGHT + BOTTOM], parts)
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
