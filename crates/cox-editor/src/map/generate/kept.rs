//! The lines a box keeps as it opens. Opening a box replaces its lines
//! with those of the level inside it, which go on to the parts there: a
//! person's, as from a user to the part that serves them, or the code's.
//! A line the level inside draws on to none of its parts stays, to the
//! edge of the box opened, so that what the box is wired to stays in
//! sight: a person's to a person or a system, which no code tells the
//! part of, or one whose code is not among the types' lines, as a call
//! from a function of no type.

use std::collections::{BTreeMap, BTreeSet};

use crate::map::generated::{Level, Line};
use crate::map::vocabulary::Vocabulary;

/// Has each of `levels` keep the lines its box has where it is drawn
/// closed, in the level it is in, or the top's `top_lines` for the top's
/// center, that it draws on to none of its parts: drawn to the box, its
/// edge there, when the level shows the line's other end.
pub(super) fn keep(
    vocabulary: &Vocabulary,
    top_lines: &[Line],
    levels: &mut BTreeMap<String, Level>,
) {
    let inside_of = vocabulary.inside_of();
    let mut kept: Vec<(String, Line)> = Vec::new();
    for (opened, level) in levels.iter() {
        let (above, around) = match inside_of.get(opened.as_str()) {
            Some(&parent) => match levels.get(parent) {
                Some(above) => (&above.lines[..], shown(above, parent, opened)),
                None => continue,
            },
            None if *opened == vocabulary.top.center => (
                top_lines,
                vocabulary
                    .top
                    .left
                    .iter()
                    .chain(&vocabulary.top.right)
                    .map(String::as_str)
                    .collect(),
            ),
            None => continue,
        };
        // A level of types grows out of the level above, which it shows
        // all of around it; a person places what a level of boxes shows.
        let around = if level.size.is_some() {
            around
        } else {
            level
                .at
                .keys()
                .chain(level.outer.keys())
                .map(String::as_str)
                .collect()
        };
        let reached = reached(level, opened);
        for line in above {
            let other = if line.to == *opened {
                &line.from
            } else if line.from == *opened {
                &line.to
            } else {
                continue;
            };
            if other != opened
                && around.contains(other.as_str())
                && !reached.contains(other.as_str())
            {
                kept.push((opened.clone(), line.clone()));
            }
        }
    }
    for (opened, line) in kept {
        if let Some(level) = levels.get_mut(&opened) {
            level.lines.push(line);
        }
    }
}

/// The nodes the level of types inside `opened` shows around it, as it
/// grows out of `above`, the level inside `parent` that `opened` is in:
/// those `above` places, but `opened`, and `parent`, whose edge it is.
fn shown<'l>(above: &'l Level, parent: &str, opened: &str) -> BTreeSet<&'l str> {
    above
        .parts
        .keys()
        .chain(above.at.keys())
        .chain(above.outer.keys())
        .map(String::as_str)
        .filter(|&id| id != opened && id != parent)
        .collect()
}

/// The nodes around `level`, the level inside `opened`, that its lines
/// join to one of its parts, or to the box itself.
fn reached<'l>(level: &'l Level, opened: &str) -> BTreeSet<&'l str> {
    let inside = |id: &str| id == opened || level.parts.contains_key(id);
    let mut reached = BTreeSet::new();
    for line in &level.lines {
        if inside(&line.from) && !inside(&line.to) {
            reached.insert(line.to.as_str());
        }
        if inside(&line.to) && !inside(&line.from) {
            reached.insert(line.from.as_str());
        }
    }
    reached
}
