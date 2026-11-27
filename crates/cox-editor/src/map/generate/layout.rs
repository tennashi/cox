//! Where things sit on the map. A person places the levels the
//! vocabulary has; the map adds the lines the code gives between their
//! boxes, and swaps two boxes' places when that shortens those lines, so
//! that a box moves only to untangle them. A level the person placed
//! nothing around opens where its box sits in the level above, which
//! stays around it. Around another, what its lines reach and the person
//! did not place, the map places beside the box, on the side it lies on
//! in the level above. The types inside
//! a box are placed on a grid, the one most lines end on in the middle
//! and each near what it is wired to, pulled toward the side of the box
//! that the boxes it is wired to are on; those wired to nothing after
//! them, in name order.

use std::collections::{BTreeMap, BTreeSet, HashMap};

use super::Map;
use crate::map::generated::{By, Level, Line};
use crate::map::vocabulary::{self, Side};

/// A card's width and its height, as the page draws them, whatever its
/// name is long: the page fits the name to the card.
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
/// What a line no code backs weighs, against `1 + log10(references)` for
/// one the code gives: a person drew it, so it holds its ends together.
const HAND_WEIGHT: f64 = 6.0;
/// What a line between types that is always drawn weighs as the types
/// are placed, against one drawn only while an end is chosen.
const DRAWN: f64 = 3.0;

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
/// A level that places nothing around its box shows the level above
/// around it, moved aside for the box to grow where it sits there, and
/// has the code's lines go on to the boxes the levels above show closed.
/// Every end of a level's lines that is placed nowhere then, the map
/// places around its box.
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
                    stops: false,
                    by: By::Code,
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
        let mut frames = level.frames.clone();
        // A level that places nothing around its box opens where the box
        // sits in the level above, which stays around it.
        if level.at.is_empty()
            && level.outer.is_empty()
            && level.frames.is_empty()
            && let Some(&outer) = map.inside_of.get(opened.as_str())
            && let Some(above) = levels.get(outer)
            && let Some(&(_, slot)) = slots.get(opened.as_str())
        {
            (at, frames) =
                above_moved_aside(map, (outer, above), (opened.as_str(), slot), level.area);
        }
        let mut placed = Level {
            area: Some(level.area),
            size: None,
            outer: level.outer.clone(),
            at,
            frames,
            parts,
            lines,
        };
        let more = around(map, &slots, opened, &placed);
        placed.at.extend(more);
        levels.insert(opened.clone(), placed);
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
/// each end of its lines that `level`, as placed so far, places
/// nowhere: beside the box, past what is placed there, on the side the end lies on in the level above (see
/// [`lies`]); those of one side in the order of the `parts` their lines
/// go to. Not the boxes `opened` is inside of, whose edges are drawn, nor
/// what is inside it.
fn around(
    map: &Map<'_, '_>,
    slots: &HashMap<&str, (&str, [i32; 2])>,
    opened: &str,
    level: &Level,
) -> BTreeMap<String, [i32; 2]> {
    let Some(area) = level.area else {
        return BTreeMap::new();
    };
    let (parts, lines) = (&level.parts, &level.lines);
    let above = map.and_around(opened);
    let [left, top, width, height] = area;
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
    // The edges of what is placed, which the map places past.
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
                    let size = card(map, id);
                    (size, beside(area, side, along, size))
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

/// Where the page draws a card of `size` that a level places by `side`
/// of its box, at `area`, `along` it.
fn beside(area: [i32; 4], side: Side, along: i32, [w, h]: [i32; 2]) -> [i32; 2] {
    let [left, top, width, height] = area;
    match side {
        Side::Left => [left - BESIDE - w, along],
        Side::Right => [left + width + BESIDE, along],
        Side::Top => [along, top - ABOVE - h],
        Side::Bottom => [along, top + height + ABOVE],
    }
}

/// What the level `above`, inside `outer`, shows, as the level inside
/// `opened` shows it around its box, at `area`: the box grows from the
/// middle of its card, which sits at `slot` there, and what is beside
/// the card moves aside by as much as the box grows that way, as the
/// page has a level of types grow. Gives where each node above sits,
/// and the edges of the boxes above, grown as much.
fn above_moved_aside(
    map: &Map<'_, '_>,
    (outer, above): (&str, &Level),
    (opened, slot): (&str, [i32; 2]),
    area: [i32; 4],
) -> (BTreeMap<String, [i32; 2]>, BTreeMap<String, [i32; 4]>) {
    let Some(above_area) = above.area else {
        return (BTreeMap::new(), BTreeMap::new());
    };
    let [w, h] = card(map, opened);
    let [left, top, width, height] = area;
    let more = [(width - w) / 2, (height - h) / 2];
    // The level keeps its own places: those above are told from its box.
    let from = [left + more[0] - slot[0], top + more[1] - slot[1]];
    let aside = |at: [i32; 2], size: [i32; 2]| {
        [0, 1].map(|axis| {
            let way = if at[axis] >= slot[axis] + [w, h][axis] {
                more[axis]
            } else if at[axis] + size[axis] <= slot[axis] {
                -more[axis]
            } else {
                0
            };
            at[axis] + way + from[axis]
        })
    };
    let grown = |[x, y, width, height]: [i32; 4]| {
        [
            x - more[0] + from[0],
            y - more[1] + from[1],
            width + 2 * more[0],
            height + 2 * more[1],
        ]
    };
    let placed = above
        .parts
        .iter()
        .chain(&above.at)
        .map(|(id, &at)| (id, at));
    let by_side = above
        .outer
        .iter()
        .map(|(id, &vocabulary::Outer(side, along))| {
            (id, beside(above_area, side, along, card(map, id)))
        });
    let at = placed
        .chain(by_side)
        .filter(|(id, _)| id.as_str() != opened)
        .map(|(id, at)| (id.clone(), aside(at, card(map, id))))
        .collect();
    let mut frames: BTreeMap<String, [i32; 4]> = above
        .frames
        .iter()
        .map(|(id, &edge)| (id.clone(), grown(edge)))
        .collect();
    frames.insert(outer.to_owned(), grown(above_area));
    (at, frames)
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
            return if top.on_the_left().any(|left| left == id) {
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
    card_of(map.vocabulary, id)
}

fn card_of(vocabulary: &vocabulary::Vocabulary, id: &str) -> [i32; 2] {
    match vocabulary.nodes.get(id) {
        Some(node) if node.kind == vocabulary::Kind::Outside => OUTSIDE_CARD,
        _ => [CARD_WIDTH, CARD_HEIGHT],
    }
}

/// The steps of the grid a level's parts close up on, across and down,
/// where the level's own parts are not a step apart that keeps two cards
/// clear of each other.
const STEP: [i32; 2] = [320, 200];

/// `level`, a level of `vocabulary`, with only what is `shown` of what
/// it places, and no room left where the rest was: a level that loses
/// none of its parts keeps them where a person put them; one that loses
/// some has those left in rows, in the order they read in, from the top
/// left, on the level's own grid, and its box as much smaller as the
/// rows and columns it lost. What it places beside its box by `outer`
/// closes up around the middle of each side; what it places by `at`
/// right of the box or below it, and the edges around, follow the box's
/// edge in.
pub(super) fn closed_up(
    vocabulary: &vocabulary::Vocabulary,
    level: &vocabulary::Level,
    shown: &BTreeSet<String>,
) -> vocabulary::Level {
    if level
        .parts
        .keys()
        .chain(level.outer.keys())
        .all(|id| shown.contains(id))
    {
        let mut same = level.clone();
        same.at.retain(|id, _| shown.contains(id));
        same.frames.retain(|id, _| shown.contains(id));
        return same;
    }
    let mut left: Vec<(&String, [i32; 2])> = level
        .parts
        .iter()
        .filter(|(id, _)| shown.contains(*id))
        .map(|(id, &at)| (id, at))
        .collect();
    left.sort_by_key(|&(id, [x, y])| (y, x, id));
    let [area_x, area_y, area_width, area_height] = level.area;
    let mut area = level.area;
    let mut parts: BTreeMap<String, [i32; 2]> =
        left.iter().map(|&(id, at)| (id.clone(), at)).collect();
    if left.len() < level.parts.len() {
        // The grid the person placed the parts on: where it starts, its
        // steps, and how many columns it has.
        let along = |axis: usize| -> (i32, i32, i32) {
            let mut all: Vec<i32> = level.parts.values().map(|at| at[axis]).collect();
            all.sort_unstable();
            all.dedup();
            let clear = [CARD_WIDTH, CARD_HEIGHT][axis] + BETWEEN;
            let step = all
                .windows(2)
                .map(|pair| pair[1] - pair[0])
                .min()
                .filter(|&step| step >= clear)
                .unwrap_or(STEP[axis]);
            let (first, last) = (all[0], all[all.len() - 1]);
            (first, step, last)
        };
        let (x, across, last_x) = along(0);
        let (y, down, last_y) = along(1);
        let columns = usize::try_from((last_x - x) / across + 1)
            .unwrap_or(1)
            .max(1);
        parts = left
            .iter()
            .enumerate()
            .map(|(index, &(id, _))| {
                (
                    id.clone(),
                    [
                        x + int(index % columns) * across,
                        y + int(index / columns) * down,
                    ],
                )
            })
            .collect();
        let right = x + int(left.len().min(columns) - 1) * across;
        let bottom = y + int((left.len() - 1) / columns) * down;
        area = [
            area_x,
            area_y,
            (area_width - (last_x - right)).max(CARD_WIDTH),
            (area_height - (last_y - bottom)).max(CARD_HEIGHT),
        ];
    }
    let less = [area_width - area[2], area_height - area[3]];
    let outer = beside_closed_up(vocabulary, level, area, shown);
    let past = [area_x + area_width, area_y + area_height];
    vocabulary::Level {
        area,
        outer,
        at: level
            .at
            .iter()
            .filter(|(id, _)| shown.contains(*id))
            .map(|(id, &at)| {
                let moved = [0, 1].map(|axis| {
                    if at[axis] >= past[axis] {
                        at[axis] - less[axis]
                    } else {
                        at[axis]
                    }
                });
                (id.clone(), moved)
            })
            .collect(),
        frames: level
            .frames
            .iter()
            .filter(|(id, _)| shown.contains(*id))
            .map(|(id, &[x, y, width, height])| {
                (id.clone(), [x, y, width - less[0], height - less[1]])
            })
            .collect(),
        parts,
        lines: level.lines.clone(),
    }
}

/// What `level` places beside its box by `outer`, of what is `shown`,
/// around the box as it is at `area` now: each side's cards as far apart
/// as the person put them, around the middle of the side.
fn beside_closed_up(
    vocabulary: &vocabulary::Vocabulary,
    level: &vocabulary::Level,
    area: [i32; 4],
    shown: &BTreeSet<String>,
) -> BTreeMap<String, vocabulary::Outer> {
    let mut outer = BTreeMap::new();
    for side in [Side::Left, Side::Right, Side::Top, Side::Bottom] {
        let axis = usize::from(matches!(side, Side::Left | Side::Right));
        let mut all: Vec<(i32, &String)> = level
            .outer
            .iter()
            .filter(|(_, at)| at.0 == side)
            .map(|(id, at)| (at.1, id))
            .collect();
        all.sort_unstable();
        let most = all
            .iter()
            .map(|(_, id)| card_of(vocabulary, id)[axis])
            .max()
            .unwrap_or(0);
        let step = all
            .windows(2)
            .map(|pair| pair[1].0 - pair[0].0)
            .min()
            .filter(|&step| step >= most)
            .unwrap_or(most + BETWEEN);
        let kept: Vec<&String> = all
            .into_iter()
            .map(|(_, id)| id)
            .filter(|id| shown.contains(*id))
            .collect();
        let long = kept.last().map_or(0, |id| {
            card_of(vocabulary, id)[axis] + int(kept.len() - 1) * step
        });
        let from = area[axis] + area[2 + axis] / 2 - long / 2;
        for (index, id) in kept.into_iter().enumerate() {
            outer.insert(
                id.clone(),
                vocabulary::Outer(side, from + int(index) * step),
            );
        }
    }
    outer
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
}

/// Places the `cards` of a box sitting at `me`, with `lines` between
/// them and to the boxes `around`, and the `weak` ones, drawn only while
/// an end is chosen. Those a line ends on sit together, on a grid sized
/// by their own count. Those wired to nothing
/// follow below, a little apart, in rows as wide as the grid of all the
/// cards would be, in name order, so that they read as a list and leave
/// the others where they are. Gives the level's size, and where each
/// card sits, from its top left.
pub(super) fn types(
    cards: &[Card<'_>],
    lines: &BTreeSet<(String, String)>,
    weak: &BTreeSet<(String, String)>,
    around: &BTreeMap<&str, [i32; 2]>,
    me: [i32; 2],
) -> ([i32; 2], BTreeMap<String, [i32; 2]>) {
    let ends: BTreeSet<&str> = lines
        .iter()
        .chain(weak)
        .flat_map(|(a, b)| [a.as_str(), b.as_str()])
        .collect();
    let (wired, mut loose): (Vec<&Card>, Vec<&Card>) =
        cards.iter().partition(|card| ends.contains(card.id));
    let grid = wired_grid(&wired, [lines, weak], around, me);
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
    let right = parts
        .values()
        .map(|at| at[0] + CARD_WIDTH)
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
/// row, on a grid of their own. The one most lines end on takes the
/// middle, and each after it the free cell nearest what it is wired to,
/// those wired most to what is placed first: so the types a type names
/// sit around it, and its lines do not fan from a corner. Another box is
/// where it lies as seen from this one, and pulls what is wired to it
/// toward that side. Two cards then trade cells, or one moves to a free
/// cell, while that shortens the lines. A line always drawn counts as
/// [`DRAWN`] of those drawn only while an end is chosen, to be the
/// shorter.
fn wired_grid<'t>(
    cards: &[&Card<'t>],
    [lines, weak]: [&BTreeSet<(String, String)>; 2],
    around: &BTreeMap<&str, [i32; 2]>,
    me: [i32; 2],
) -> BTreeMap<&'t str, Cell> {
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
    let mut by_name: Vec<&Card<'t>> = cards.to_vec();
    by_name.sort_by(|a, b| a.name.cmp(b.name).then(a.id.cmp(b.id)));
    let index: BTreeMap<&str, usize> = by_name
        .iter()
        .enumerate()
        .map(|(index, card)| (card.id, index))
        .collect();
    let mut wires = Wires {
        cards: vec![Vec::new(); count],
        boxes: vec![Vec::new(); count],
        middle: (
            f64::from(int(columns - 1) * COLUMN + CARD_WIDTH) / 2.0,
            f64::from(int(rows.max(1) - 1) * ROW + CARD_HEIGHT) / 2.0,
        ),
    };
    let weighed = lines
        .iter()
        .map(|line| (line, DRAWN))
        .chain(weak.iter().map(|line| (line, 1.0)));
    for ((a, b), weight) in weighed {
        for (end, other) in [(a, b), (b, a)] {
            let Some(&end) = index.get(end.as_str()) else {
                continue;
            };
            if let Some(&other) = index.get(other.as_str()) {
                wires.cards[end].push((other, weight));
            } else if let Some(&toward) = pull.get(other.as_str()) {
                wires.boxes[end].push((toward, weight));
            }
        }
    }
    // The free cells, row by row.
    let mut free: Vec<Cell> = (0..rows)
        .flat_map(|row| (0..columns).map(move |column| (column, row)))
        .collect();
    let mut at = wires.placed(&mut free);
    wires.trade(&mut at, &mut free);
    by_name
        .iter()
        .zip(at)
        .filter_map(|(card, at)| Some((card.id, at?)))
        .collect()
}

/// A cell of the grid: its column and its row.
type Cell = (usize, usize);

/// The middle of a card in the cell `at`, where its lines pull it from.
fn middle_of((column, row): Cell) -> (f64, f64) {
    (
        f64::from(int(column) * COLUMN + CARD_WIDTH / 2),
        f64::from(int(row) * ROW + CARD_HEIGHT / 2),
    )
}

/// What the cards of a grid are wired to, each card being its place in
/// name order.
struct Wires {
    /// For each card, the cards its lines go to, and what each line
    /// weighs.
    cards: Vec<Vec<(usize, f64)>>,
    /// For each card, where the boxes around that its lines go to pull
    /// it toward, and what each line weighs.
    boxes: Vec<Vec<((f64, f64), f64)>>,
    /// The middle of the grid.
    middle: (f64, f64),
}

/// A way to shorten the lines: a card trades cells with another, or
/// moves to a free cell, by its place among the free ones.
#[derive(Clone, Copy)]
enum Trade {
    With(usize),
    To(usize),
}

impl Wires {
    /// How long the lines of `card` are, weighed, with it in the cell
    /// `here` and the others `at` their cells, one not placed yet
    /// counted as in the middle.
    fn length(&self, card: usize, here: Cell, at: &[Option<Cell>]) -> f64 {
        let here = middle_of(here);
        let to_cards: f64 = self.cards[card]
            .iter()
            .map(|&(other, weight)| weight * far(here, at[other].map_or(self.middle, middle_of)))
            .sum();
        let to_boxes: f64 = self.boxes[card]
            .iter()
            .map(|&(toward, weight)| weight * far(here, toward))
            .sum();
        to_cards + to_boxes
    }

    /// What the lines of `card` weigh: those to the cards placed `at` a
    /// cell, or, with none given, all of them.
    fn weight(&self, card: usize, at: Option<&[Option<Cell>]>) -> f64 {
        let to_cards = self.cards[card]
            .iter()
            .filter(|(other, _)| at.is_none_or(|at| at[*other].is_some()))
            .map(|&(_, weight)| weight);
        match at {
            Some(_) => to_cards.sum(),
            None => to_cards
                .chain(self.boxes[card].iter().map(|&(_, weight)| weight))
                .sum(),
        }
    }

    /// Places the cards, one at a time, each in the cell of `free` that
    /// leaves its lines shortest, and takes the cell from `free`: first
    /// the card wired most to those placed, or else the one most lines
    /// end on.
    fn placed(&self, free: &mut Vec<Cell>) -> Vec<Option<Cell>> {
        let count = self.cards.len();
        let mut at: Vec<Option<Cell>> = vec![None; count];
        for _ in 0..count {
            let mut next: Option<(usize, f64, f64)> = None;
            for card in (0..count).filter(|&card| at[card].is_none()) {
                let (placed, all) = (self.weight(card, Some(&at)), self.weight(card, None));
                if next.is_none_or(|(_, most, of_all)| {
                    placed > most || (placed >= most && all > of_all)
                }) {
                    next = Some((card, placed, all));
                }
            }
            let Some((next, ..)) = next else {
                break;
            };
            let mut best: Option<(f64, usize)> = None;
            for (index, &cell) in free.iter().enumerate() {
                // Of cells as near what it is wired to, the one nearest
                // the middle.
                let cost = self.length(next, cell, &at) + far(middle_of(cell), self.middle) * 1e-3;
                if best.is_none_or(|(least, _)| cost < least - 1e-6) {
                    best = Some((cost, index));
                }
            }
            let Some((_, index)) = best else {
                break;
            };
            at[next] = Some(free.remove(index));
        }
        at
    }

    /// Has two cards trade cells, or one move to a cell of `free`, the
    /// trade that shortens the lines most each time, while one does. A
    /// trade changes only the lines of the two cards, and the one
    /// between them stays as long.
    fn trade(&self, at: &mut [Option<Cell>], free: &mut [Cell]) {
        let count = at.len();
        for _ in 0..count * count {
            let mut gain: Option<(f64, usize, Trade)> = None;
            let mut better = |less: f64, card: usize, trade: Trade| {
                if less > 1e-6 && gain.is_none_or(|(most, ..)| less > most + 1e-6) {
                    gain = Some((less, card, trade));
                }
            };
            for a in 0..count {
                let Some(here) = at[a] else {
                    continue;
                };
                for b in a + 1..count {
                    let Some(there) = at[b] else {
                        continue;
                    };
                    let before = self.length(a, here, at) + self.length(b, there, at);
                    let mut tried = at.to_vec();
                    tried.swap(a, b);
                    let after = self.length(a, there, &tried) + self.length(b, here, &tried);
                    better(before - after, a, Trade::With(b));
                }
                for (index, &cell) in free.iter().enumerate() {
                    let less = self.length(a, here, at) - self.length(a, cell, at);
                    better(less, a, Trade::To(index));
                }
            }
            match gain {
                Some((_, a, Trade::With(b))) => at.swap(a, b),
                Some((_, a, Trade::To(index))) => {
                    if let Some(was) = at[a].replace(free[index]) {
                        free[index] = was;
                    }
                }
                None => break,
            }
        }
    }
}

/// How far two points are apart.
fn far(a: (f64, f64), b: (f64, f64)) -> f64 {
    ((a.0 - b.0).powi(2) + (a.1 - b.1).powi(2)).sqrt()
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
