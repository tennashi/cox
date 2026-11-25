//! The level inside a box that opens on its types: the types, placed,
//! and the lines between them and to the boxes around.

use std::collections::{BTreeMap, BTreeSet, HashMap};

use super::code::SymbolId;
use super::rules::Reach;
use super::{Map, layout};
use crate::map::generated::{Level, Line};
use crate::scip::role;

impl<'c> Map<'c, '_> {
    /// The symbols each type's fields name, on their lines.
    fn field_references(&self, index: usize) -> BTreeSet<SymbolId> {
        let ty = &self.types[index];
        self.code.documents[ty.document]
            .occurrences
            .iter()
            .filter(|occurrence| occurrence.roles & role::DEFINITION == 0)
            .filter_map(|occurrence| {
                let id = occurrence.symbol?;
                let line = occurrence.span.start_line;
                (self.code.symbols[id].own
                    && ty
                        .fields
                        .iter()
                        .any(|field| field.span.0 <= line && line <= field.span.1))
                .then_some(id)
            })
            .collect()
    }

    /// The level inside each box that opens on its types and sits in a
    /// level of the vocabulary: its types, placed, and the lines between
    /// them and to the boxes around; those that `reaches` pick placed
    /// toward the other end of their lines too.
    pub(super) fn type_levels(
        &self,
        slots: &HashMap<&'c str, (&'c str, [i32; 2])>,
        levels: &BTreeMap<String, Level>,
        reaches: &[Reach<'_>],
    ) -> Vec<(&'c str, Level)> {
        let owners: BTreeSet<&str> = self.types.iter().map(|ty| ty.owner).collect();
        let mut out = Vec::new();
        for owner in owners {
            let Some(&(level, me)) = slots.get(owner) else {
                continue;
            };
            let level = &levels[level];
            let around: BTreeMap<&str, [i32; 2]> = level
                .parts
                .iter()
                .chain(&level.at)
                .map(|(id, &at)| (id.as_str(), at))
                .collect();
            let mut types: Vec<usize> = (0..self.types.len())
                .filter(|&index| self.types[index].owner == owner)
                .collect();
            types.sort_by(|&a, &b| {
                let (a, b) = (&self.types[a], &self.types[b]);
                self.code.documents[a.document]
                    .path
                    .cmp(self.code.documents[b.document].path)
                    .then(a.line.cmp(&b.line))
            });
            let lines = self.type_lines(owner, &types, &around);
            let cards: Vec<layout::Card> = types
                .iter()
                .map(|&index| layout::Card {
                    id: &self.types[index].id,
                    name: &self.types[index].name,
                    code: self.types[index].kind,
                })
                .collect();
            // A box the level shows only by its edge, as `outer`, has no
            // place to pull a type toward; a rule's line to it still
            // wires the types it picks.
            let shown: BTreeSet<&str> = around
                .keys()
                .copied()
                .chain(level.outer.keys().map(String::as_str))
                .collect();
            let mut placing = lines.clone();
            placing.extend(self.reached_types(reaches, owner, &shown));
            let (size, parts) = layout::types(&cards, &placing, &around, me);
            out.push((
                owner,
                Level {
                    area: None,
                    size: Some(size),
                    outer: BTreeMap::new(),
                    at: BTreeMap::new(),
                    frames: BTreeMap::new(),
                    parts,
                    lines: lines
                        .into_iter()
                        .map(|(from, to)| Line {
                            from,
                            to,
                            both: false,
                            sides: None,
                            weak: false,
                        })
                        .collect(),
                },
            ));
        }
        out
    }

    /// The lines inside the box `owner`: between two of its `types` where
    /// a field of one names the other, once for both ways; and to each box
    /// `around`, from the type whose fields name it most, and to the type
    /// it uses most, a box around standing for the boxes inside it.
    pub(super) fn type_lines(
        &self,
        owner: &str,
        types: &[usize],
        around: &BTreeMap<&str, [i32; 2]>,
    ) -> BTreeSet<(String, String)> {
        let ids: BTreeSet<&str> = types
            .iter()
            .map(|&index| self.types[index].id.as_str())
            .collect();
        let mut lines: BTreeSet<(String, String)> = BTreeSet::new();
        let mut out: BTreeMap<(&str, &str), usize> = BTreeMap::new();
        for &index in types {
            let ty = &self.types[index];
            let mut named: Vec<SymbolId> = self.field_references(index).into_iter().collect();
            named.sort_by_key(|&id| self.code.symbols[id].text);
            for id in named {
                let key = self.key(id);
                if let Some(key) = key.filter(|key| ids.contains(key)) {
                    if key != ty.id && !lines.contains(&(key.to_owned(), ty.id.clone())) {
                        lines.insert((ty.id.clone(), key.to_owned()));
                    }
                    continue;
                }
                let other = match key.and_then(|key| self.key_type(key)) {
                    Some(other) => Some(self.types[self.type_ids[&other]].owner),
                    None => self.box_of_symbol(id),
                };
                if let Some(other) = other
                    .and_then(|other| self.shown_as(other, |at| around.contains_key(at)))
                    .filter(|&other| other != owner)
                {
                    *out.entry((other, ty.id.as_str())).or_default() += 1;
                }
            }
        }
        let mut into: BTreeMap<(&str, &str), usize> = BTreeMap::new();
        for &index in types {
            let ty = &self.types[index];
            let keys = std::iter::once(ty.id.as_str())
                .chain(ty.fields.iter().map(|field| field.key.as_str()))
                .chain(ty.functions.iter().map(|function| function.key.as_str()));
            for key in keys {
                for found in self.uses.get(key).into_iter().flatten() {
                    let other = found.node.as_deref().map(|node| {
                        self.type_ids
                            .get(node)
                            .map_or(node, |&other| self.types[other].owner)
                    });
                    if let Some(other) = other
                        .filter(|_| !found.test)
                        .and_then(|other| self.shown_as(other, |at| around.contains_key(at)))
                        .filter(|&other| other != owner)
                    {
                        *into.entry((other, ty.id.as_str())).or_default() += 1;
                    }
                }
            }
        }
        for (counts, inward) in [(out, false), (into, true)] {
            let mut best: BTreeMap<&str, (&str, usize)> = BTreeMap::new();
            for ((other, ty), count) in counts {
                if best.get(other).is_none_or(|&(_, most)| count > most) {
                    best.insert(other, (ty, count));
                }
            }
            for (other, (ty, _)) in best {
                let (from, to) = if inward { (other, ty) } else { (ty, other) };
                if !lines.contains(&(to.to_owned(), from.to_owned())) {
                    lines.insert((from.to_owned(), to.to_owned()));
                }
            }
        }
        lines
    }
}
