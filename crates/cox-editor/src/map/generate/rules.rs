//! The lines whose end at a box is a rule: as the box opens, each goes
//! on to the types inside it the rule picks, by what every index of the
//! code tells the same way, whatever its language: which types implement
//! a trait or an interface, or subclass a class, by the relationships the
//! index gives them, or by rust-analyzer's `impl` blocks, which name the
//! trait; and where a name is used, by its occurrences. In a level of
//! boxes, it goes on to the boxes that hold types it picks.

use std::collections::{BTreeMap, BTreeSet};

use super::code::{SymbolId, module_parts};
use super::{Map, kept};
use crate::map::generated::{By, Level, Line};
use crate::map::vocabulary::{self, Rule};
use crate::scip::role;

/// How many of the lines a rule draws from the node at a line's other
/// end, in one level, are drawn whole, those to the types it picks most
/// strongly; the rest are weak, drawn while one of their ends is chosen,
/// as of the lines the code gives between boxes.
const STRONG: usize = 2;

/// A line's end at a box that is a rule, with the types it picks.
pub(super) struct Reach<'v> {
    pub line: &'v vocabulary::Line,
    /// Which of its ends: 0, `from`; 1, `to`.
    pub end: usize,
    pub rule: &'v Rule,
    /// The types it picks inside the box, by their place in the map's,
    /// with how strongly: how often the code names a type that
    /// implements, or how often a type's code uses the name.
    pub picked: Vec<(usize, usize)>,
}

impl Reach<'_> {
    /// The box its end is at.
    pub fn node(&self) -> &str {
        if self.end == 0 {
            &self.line.from
        } else {
            &self.line.to
        }
    }

    /// The node at the line's other end.
    fn other(&self) -> &str {
        if self.end == 0 {
            &self.line.to
        } else {
            &self.line.from
        }
    }
}

/// Whether `names` end in those of `wanted`.
fn ends_in(names: &[&str], wanted: &[&str]) -> bool {
    !wanted.is_empty() && names.ends_with(wanted)
}

impl<'c> Map<'c, '_> {
    /// The ends of the vocabulary's lines that are rules, each with the
    /// types it picks, in the order the vocabulary has them: the top's
    /// lines first, then each level's.
    pub(super) fn reaches(&self) -> Vec<Reach<'c>> {
        let vocabulary: &'c vocabulary::Vocabulary = self.vocabulary;
        let lines = vocabulary
            .top
            .lines
            .iter()
            .chain(vocabulary.levels.values().flat_map(|level| &level.lines));
        let mut reaches = Vec::new();
        for line in lines {
            for (end, rule) in line.rules.iter().enumerate() {
                let Some(rule) = rule else {
                    continue;
                };
                let node = if end == 0 { &line.from } else { &line.to };
                let inside: BTreeSet<usize> = (0..self.types.len())
                    .filter(|&index| {
                        self.shows(index)
                            && self
                                .and_around(self.types[index].owner)
                                .contains(&node.as_str())
                    })
                    .collect();
                let picked = match rule {
                    Rule::Implements(name) => self.implementing(&inside, name),
                    Rule::Uses(name) => self.using(&inside, name),
                };
                reaches.push(Reach {
                    line,
                    end,
                    rule,
                    picked,
                });
            }
        }
        reaches
    }

    /// Of the types `inside`, those that implement `name`: whose
    /// relationships, or those of what they implement in turn, reach a
    /// type of that name; or, as rust-analyzer tells it, with a function
    /// in an `impl` block of a trait of the name's last part. Each with
    /// how often the code names it, outside tests.
    fn implementing(&self, inside: &BTreeSet<usize>, name: &str) -> Vec<(usize, usize)> {
        let wanted: Vec<&str> = module_parts(name).collect();
        let Some(&bare) = wanted.last() else {
            return Vec::new();
        };
        let symbols: BTreeMap<usize, SymbolId> = self
            .type_symbols
            .iter()
            .map(|(&id, &index)| (index, id))
            .collect();
        inside
            .iter()
            .copied()
            .filter(|index| {
                let by_relationships = symbols.get(index).is_some_and(|&id| {
                    self.implemented(id)
                        .into_iter()
                        .any(|target| ends_in(&self.code.symbols[target].names(), &wanted))
                });
                by_relationships
                    || self.types[*index].functions.iter().any(|function| {
                        self.owner(function.symbol)
                            .and_then(|(_, trait_)| trait_)
                            .is_some_and(|trait_| trait_ == bare)
                    })
            })
            .map(|index| {
                let named = self
                    .uses
                    .get(&self.types[index].id)
                    .map_or(0, |uses| uses.iter().filter(|found| !found.test).count());
                (index, named)
            })
            .collect()
    }

    /// What the type `id` implements, by the relationships of the index,
    /// and what each of those implements in turn.
    fn implemented(&self, id: SymbolId) -> BTreeSet<SymbolId> {
        let mut found = BTreeSet::new();
        let mut next = vec![id];
        while let Some(id) = next.pop() {
            let relationships = self.code.symbols[id]
                .info
                .into_iter()
                .flat_map(|info| &info.relationships)
                .filter(|relationship| relationship.is_implementation);
            for relationship in relationships {
                if let Some(target) = self.code.id(&relationship.symbol)
                    && found.insert(target)
                {
                    next.push(target);
                }
            }
        }
        found
    }

    /// Of the types `inside`, those whose code uses `name`: names, in
    /// their declarations or their functions, outside tests and imports,
    /// a thing of that name that is not the type itself or its own. Each
    /// with how often.
    fn using(&self, inside: &BTreeSet<usize>, name: &str) -> Vec<(usize, usize)> {
        let code = self.code;
        let wanted: Vec<&str> = module_parts(name).collect();
        let named: BTreeSet<SymbolId> = (0..code.symbols.len())
            .filter(|&id| ends_in(&code.symbols[id].names(), &wanted))
            .collect();
        let mut counts: BTreeMap<usize, usize> = BTreeMap::new();
        if named.is_empty() {
            return Vec::new();
        }
        for (document, doc) in code.documents.iter().enumerate() {
            for occurrence in &doc.occurrences {
                let Some(id) = occurrence.symbol.filter(|id| named.contains(id)) else {
                    continue;
                };
                if occurrence.roles & role::DEFINITION != 0 {
                    continue;
                }
                let line = occurrence.span.start_line;
                let function = self.inner(&doc.functions, line);
                if code.is_test(document, function, Some(line))
                    || occurrence.roles & role::TEST != 0
                    || (function.is_none() && code.is_import(document, occurrence))
                {
                    continue;
                }
                let ty = match function {
                    Some(function) => self
                        .member_keys
                        .get(&function)
                        .and_then(|key| self.functions.get(key))
                        .map(|&(index, _)| index),
                    None => self
                        .inner(&doc.types, line)
                        .and_then(|ty| self.type_symbols.get(&ty).copied()),
                };
                let Some(ty) = ty.filter(|ty| inside.contains(ty)) else {
                    continue;
                };
                let own = self
                    .key(id)
                    .and_then(|key| self.key_type(key))
                    .is_some_and(|of| of == self.types[ty].id);
                if !own {
                    *counts.entry(ty).or_default() += 1;
                }
            }
        }
        counts.into_iter().collect()
    }

    /// The lines of the types a rule picks inside the box `owner`, to
    /// the boxes `shown` around it, for its types to be placed with those
    /// wired, and toward them.
    pub(super) fn reached_types(
        &self,
        reaches: &[Reach<'_>],
        owner: &str,
        shown: &BTreeSet<&str>,
    ) -> BTreeSet<(String, String)> {
        let mut lines = BTreeSet::new();
        for reach in reaches {
            let other = reach.other();
            if !shown.contains(other)
                || !self.and_around(owner).contains(&reach.node())
                || self.and_around(other).contains(&reach.node())
            {
                continue;
            }
            for &(index, _) in &reach.picked {
                let ty = &self.types[index];
                if ty.owner == owner {
                    lines.insert((other.to_owned(), ty.id.clone()));
                }
            }
        }
        lines
    }

    /// Draws each line whose end is a rule on into every level inside its
    /// box that shows the line's other end: to the types the rule picks
    /// there, in a level of types, or, in a level of boxes, to the boxes
    /// that hold them; but where the level joins the two already. The
    /// lines to the two it picks most strongly are drawn whole, the rest
    /// weak.
    pub(super) fn reach(&self, reaches: &[Reach<'_>], levels: &mut BTreeMap<String, Level>) {
        let inside_of = self.vocabulary.inside_of();
        for reach in reaches {
            let (node, other) = (reach.node(), reach.other());
            if self.and_around(other).contains(&node) {
                continue;
            }
            let mut drawn: Vec<(String, Line)> = Vec::new();
            for (opened, level) in levels.iter() {
                if !self.and_around(opened).contains(&node)
                    || !kept::around(self.vocabulary, &inside_of, levels, opened)
                        .is_some_and(|around| around.contains(other))
                {
                    continue;
                }
                let mut strength: BTreeMap<&str, usize> = BTreeMap::new();
                for &(index, count) in &reach.picked {
                    let ty = &self.types[index];
                    if level.size.is_some() {
                        if ty.owner == opened {
                            strength.insert(&ty.id, count);
                        }
                    } else if let Some(part) = self
                        .and_around(ty.owner)
                        .into_iter()
                        .find(|&at| level.parts.contains_key(at))
                    {
                        *strength.entry(part).or_default() += count;
                    }
                }
                let joined = |item: &str| {
                    level.lines.iter().any(|line| {
                        (line.from == other && line.to == item)
                            || (line.from == item && line.to == other)
                    })
                };
                let mut items: Vec<(&str, usize)> = strength
                    .into_iter()
                    .filter(|&(item, _)| !joined(item))
                    .collect();
                items.sort_by(|a, b| b.1.cmp(&a.1).then(a.0.cmp(b.0)));
                for (rank, (item, _)) in items.into_iter().enumerate() {
                    let (from, to) = if reach.end == 1 {
                        (other, item)
                    } else {
                        (item, other)
                    };
                    drawn.push((
                        opened.clone(),
                        Line {
                            from: from.to_owned(),
                            to: to.to_owned(),
                            both: reach.line.both,
                            sides: None,
                            weak: rank >= STRONG,
                            stops: reach.line.stops,
                            by: By::Hand,
                        },
                    ));
                }
            }
            for (opened, line) in drawn {
                if let Some(level) = levels.get_mut(&opened) {
                    level.lines.push(line);
                }
            }
        }
    }
}
