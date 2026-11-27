//! What each driver sets working: of the whole map, what is reached
//! from a node that starts the project working, as a person who uses it
//! or a system that calls it (see [`Vocabulary`]'s `top.drivers`).
//!
//! A driver's walk has two kinds of step. A line a person drew, or a
//! rule drew on, is followed from the end that starts it: into a box, to
//! the parts the level inside draws it on to, or, when it draws it on to
//! none, to all of the box; out of a box, unless the level inside draws
//! it on from its parts, whose line it then is. The code is followed the
//! way it goes, each reference of the index once, outside tests: from a
//! type to what its functions call and name and to what its fields hold,
//! and on through the functions that are no type's, each on its own, so
//! that one helper called does not bring in all its box's helpers name.
//!
//! The walk goes twice. First along the lines alone, from the driver,
//! through the people and systems around, and on from a box only once it
//! has come to all of it: that gives the types the driver comes in at,
//! its way in. Then with the code too, where a line that leaves a box
//! that owns code is followed once the code has come into the box, as
//! from one process to the next; a box of boxes' line is the part's that
//! the level inside draws it on from, and else is followed only where
//! the walk comes to all of the box. And where it comes to another
//! driver's way in, it shows that and goes no further from it: what
//! happens from there is the other's. So a type several drivers reach is in each one's slice, and
//! a driver's slice is not all of the map only because the code is
//! joined up somewhere.

use std::collections::{BTreeMap, BTreeSet, HashMap, HashSet, VecDeque};

use super::code::{Role, SymbolId};
use super::{Map, layout};
use crate::map::generated::{By, Level, Line};
use crate::map::vocabulary::{self, Kind, Vocabulary};
use crate::scip::role;

/// What a driver sets working, before it is placed.
pub(super) struct Driven {
    pub driver: String,
    /// Every node its slice shows: what the walk came to, the boxes
    /// those are inside of, the center and the driver.
    pub shown: BTreeSet<String>,
    /// The nodes the walk ended at, shown and not gone into.
    pub ends: BTreeSet<String>,
}

/// A line a person drew, or a rule drew on, as the walk follows it.
struct Wire<'m> {
    from: &'m str,
    to: &'m str,
    stops: bool,
}

/// What a walk comes to: a node of the map, as a type, a box, or a
/// person or a system around; or a function of no type of the map, which
/// the code calls on its way from one type to another.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
enum At<'m> {
    Node(&'m str),
    Function(SymbolId),
}

/// What the walks go over: the map's levels, its lines that are not the
/// code's, and the code between its types.
struct Ground<'m> {
    levels: &'m BTreeMap<String, Level>,
    /// The node each node a level places is inside of.
    inside: HashMap<&'m str, &'m str>,
    outside: BTreeSet<&'m str>,
    wires: Vec<Wire<'m>>,
    /// The wires that leave each node, by their place among `wires`: not
    /// those the level inside the node draws on from its parts.
    leaving: HashMap<&'m str, Vec<usize>>,
    /// The wires each level draws.
    drawn: HashMap<&'m str, Vec<usize>>,
    /// What the code of each type, and of each function of no type,
    /// calls, names and holds.
    code: HashMap<At<'m>, BTreeSet<At<'m>>>,
    /// The box that owns each function of no type, and each box's.
    owner: HashMap<SymbolId, &'m str>,
    functions: HashMap<&'m str, Vec<SymbolId>>,
}

impl<'m> Ground<'m> {
    fn new(map: &'m Map<'_, '_>, levels: &'m BTreeMap<String, Level>, top: &'m [Line]) -> Self {
        let mut inside = HashMap::new();
        for (opened, level) in levels {
            for part in level.parts.keys() {
                inside.entry(part.as_str()).or_insert(opened.as_str());
            }
        }
        let outside = map
            .vocabulary
            .nodes
            .iter()
            .filter(|(_, node)| node.kind == Kind::Outside)
            .map(|(id, _)| id.as_str())
            .collect();
        let mut ground = Ground {
            levels,
            inside,
            outside,
            wires: Vec::new(),
            leaving: HashMap::new(),
            drawn: HashMap::new(),
            code: HashMap::new(),
            owner: HashMap::new(),
            functions: HashMap::new(),
        };
        // A line with an end at the level's own box is one the box kept
        // as it opened, which the level above has.
        let lines =
            top.iter()
                .map(|line| (None, line))
                .chain(levels.iter().flat_map(|(opened, level)| {
                    level.lines.iter().map(move |line| (Some(opened), line))
                }));
        let mut level_of = Vec::new();
        for (opened, line) in lines {
            if line.by == By::Code || opened.is_some_and(|id| line.from == *id || line.to == *id) {
                continue;
            }
            if let Some(opened) = opened {
                ground
                    .drawn
                    .entry(opened.as_str())
                    .or_default()
                    .push(ground.wires.len());
            }
            level_of.push(opened);
            ground.wires.push(Wire {
                from: &line.from,
                to: &line.to,
                stops: line.stops,
            });
        }
        for (at, wire) in ground.wires.iter().enumerate() {
            let drawn_on = levels.get(wire.from).is_some_and(|within| {
                level_of[at].is_none_or(|opened| opened != wire.from)
                    && ground.drawn.get(wire.from).is_some_and(|inner| {
                        inner.iter().any(|&other| {
                            let other = &ground.wires[other];
                            within.parts.contains_key(other.from)
                                && ground.same_end(other.to, wire.to)
                        })
                    })
            });
            if !drawn_on {
                ground.leaving.entry(wire.from).or_default().push(at);
            }
        }
        ground.read_code(map);
        ground
    }

    /// Reads the code between the types of `map`, each reference once,
    /// outside tests and imports: from the type whose function or
    /// declaration names something, or the function of no type that does,
    /// to the type it names, or a member of, or the function of no type
    /// it calls; and to the box that owns anything else it names, where
    /// the box shows no types to name.
    fn read_code(&mut self, map: &'m Map<'_, '_>) {
        let code = map.code;
        // What a symbol is to the walk.
        let node = |id: SymbolId| -> Option<At<'m>> {
            let member = || {
                let key = map.member_keys.get(&id)?;
                map.functions.get(key).or_else(|| map.fields.get(key))
            };
            if let Some(ty) = map
                .type_symbols
                .get(&id)
                .copied()
                .or_else(|| member().map(|&(ty, _)| ty))
            {
                return Some(At::Node(map.types[ty].id.as_str()));
            }
            let sym = &code.symbols[id];
            if sym.role == Some(Role::Function) {
                return Some(At::Function(id));
            }
            let owner = map.box_of_symbol(id)?;
            let node = map.vocabulary.nodes.get_key_value(owner)?;
            (!node.1.open).then_some(At::Node(node.0.as_str()))
        };
        for (id, sym) in code.symbols.iter().enumerate() {
            if sym.own
                && sym.role == Some(Role::Function)
                && sym.definition.is_some()
                && map.key(id).is_none()
                && let Some((owner, _)) = map
                    .box_of_symbol(id)
                    .and_then(|owner| map.vocabulary.nodes.get_key_value(owner))
            {
                self.owner.insert(id, owner.as_str());
                self.functions.entry(owner.as_str()).or_default().push(id);
            }
        }
        for (document, doc) in code.documents.iter().enumerate() {
            for occurrence in &doc.occurrences {
                let Some(id) = occurrence.symbol else {
                    continue;
                };
                if occurrence.roles & role::DEFINITION != 0 || !code.symbols[id].own {
                    continue;
                }
                let line = occurrence.span.start_line;
                let function = map.inner(&doc.functions, line);
                if code.is_test(document, function, Some(line))
                    || occurrence.roles & role::TEST != 0
                    || (function.is_none() && code.is_import(document, occurrence))
                {
                    continue;
                }
                let from = match function {
                    Some(function) => node(function),
                    None => map
                        .inner(&doc.types, line)
                        .and_then(|ty| map.type_symbols.get(&ty))
                        .map(|&ty| At::Node(map.types[ty].id.as_str())),
                };
                if let Some((from, to)) = from.zip(node(id))
                    && from != to
                {
                    self.code.entry(from).or_default().insert(to);
                }
            }
        }
    }

    /// `id`, and the nodes it is inside of, from the nearest out.
    fn chain(&self, id: &'m str) -> Vec<&'m str> {
        let mut chain = vec![id];
        while let Some(&outer) = chain.last().and_then(|at| self.inside.get(at)) {
            if chain.contains(&outer) {
                break;
            }
            chain.push(outer);
        }
        chain
    }

    /// The node of the map `at` is, or is a function of.
    fn node(&self, at: At<'m>) -> Option<&'m str> {
        match at {
            At::Node(id) => Some(id),
            At::Function(id) => self.owner.get(&id).copied(),
        }
    }

    /// Whether two ends of lines are of one: the same node, or one
    /// inside the other, as a level tells the end of a line it draws on
    /// by a box the level above shows.
    fn same_end(&self, a: &'m str, b: &'m str) -> bool {
        self.chain(a).contains(&b) || self.chain(b).contains(&a)
    }

    /// What `driver` reaches: along the lines alone, or the code too,
    /// going no further from those of `theirs` that are not its `own`.
    fn walk(
        &self,
        driver: &'m str,
        code: bool,
        theirs: &BTreeSet<At<'m>>,
        own: &BTreeSet<At<'m>>,
    ) -> Walk<'_, 'm> {
        let mut walk = Walk {
            ground: self,
            code,
            theirs: theirs.clone(),
            own: own.clone(),
            reached: BTreeSet::new(),
            ends: BTreeSet::new(),
            touched: HashSet::new(),
            fired: VecDeque::new(),
            named: VecDeque::new(),
        };
        walk.reach(At::Node(driver));
        loop {
            if let Some(wire) = walk.fired.pop_front() {
                let wire = &self.wires[wire];
                walk.enter(wire.to, wire.from, wire.stops);
            } else if let Some(from) = walk.named.pop_front() {
                for &to in self.code.get(&from).into_iter().flatten() {
                    walk.reach(to);
                }
            } else {
                break;
            }
        }
        walk
    }
}

/// A driver's walk, as far as it has come.
struct Walk<'g, 'm> {
    ground: &'g Ground<'m>,
    /// Whether it follows the code.
    code: bool,
    /// The other drivers' ways in, which it goes no further from, and
    /// its own, which it does.
    theirs: BTreeSet<At<'m>>,
    own: BTreeSet<At<'m>>,
    /// What it came to and went on from.
    reached: BTreeSet<At<'m>>,
    /// What it came to and went no further from.
    ends: BTreeSet<At<'m>>,
    /// The nodes whose lines it has followed.
    touched: HashSet<&'m str>,
    /// The wires to follow, and what to follow the code of.
    fired: VecDeque<usize>,
    named: VecDeque<At<'m>>,
}

impl<'m> Walk<'_, 'm> {
    /// Comes to `at`, and goes on from it: along the wires that leave
    /// its node and the boxes that is inside of, and along its code.
    fn reach(&mut self, at: At<'m>) {
        if self.reached.contains(&at) {
            return;
        }
        if self.theirs.contains(&at) && !self.own.contains(&at) {
            self.ends.insert(at);
            return;
        }
        self.ends.remove(&at);
        self.reached.insert(at);
        // The lines that leave what it came to: its own node's; and,
        // with the code, those of the box that owns the type or the
        // function, which the code has come into. A box of boxes' are
        // followed where the walk comes to the box itself, as to all of
        // it: from one part, only what the level inside draws on from it.
        let ground = self.ground;
        let nodes = match ground.node(at) {
            Some(node) if at == At::Node(node) => {
                let owner = ground.inside.get(node).filter(|owner| {
                    self.code
                        && ground
                            .levels
                            .get(**owner)
                            .is_some_and(|level| level.size.is_some())
                });
                std::iter::once(node).chain(owner.copied()).collect()
            }
            Some(owner) if self.code => vec![owner],
            _ => Vec::new(),
        };
        for node in nodes {
            if self.touched.insert(node) {
                self.fired
                    .extend(self.ground.leaving.get(node).into_iter().flatten());
            }
        }
        if self.code {
            self.named.push_back(at);
        }
    }

    /// Follows a wire from `from` into `to`: to the parts the level
    /// inside `to` draws it on to, from `from` or a box it is inside of,
    /// or else to all of it.
    fn enter(&mut self, to: &'m str, from: &'m str, stops: bool) {
        if stops {
            if !self.reached.contains(&At::Node(to)) {
                self.ends.insert(At::Node(to));
            }
            return;
        }
        let ground = self.ground;
        if ground.outside.contains(to) {
            self.reach(At::Node(to));
            return;
        }
        let above = ground.chain(from);
        let on: Vec<&Wire<'m>> = ground
            .levels
            .get(to)
            .into_iter()
            .flat_map(|level| {
                ground
                    .drawn
                    .get(to)
                    .into_iter()
                    .flatten()
                    .map(|&wire| &ground.wires[wire])
                    .filter(|wire| above.contains(&wire.from) && level.parts.contains_key(wire.to))
            })
            .collect();
        if on.is_empty() {
            self.all(to);
        }
        for wire in on {
            self.enter(wire.to, from, wire.stops);
        }
    }

    /// Comes to all of the box `id`: every type inside it, and every
    /// function of no type that it owns.
    fn all(&mut self, id: &'m str) {
        let ground = self.ground;
        self.reach(At::Node(id));
        for &function in ground.functions.get(id).into_iter().flatten() {
            self.reach(At::Function(function));
        }
        for part in ground
            .levels
            .get(id)
            .into_iter()
            .flat_map(|level| level.parts.keys())
        {
            self.all(part);
        }
    }
}

impl Map<'_, '_> {
    /// What each driver of the top sets working, in the top's order, on
    /// the map whose `levels` and `top` lines are drawn whole.
    pub(super) fn driven(&self, levels: &BTreeMap<String, Level>, top: &[Line]) -> Vec<Driven> {
        let vocabulary = self.vocabulary;
        let drivers = &vocabulary.top.drivers;
        if drivers.is_empty() {
            return Vec::new();
        }
        let ground = Ground::new(self, levels, top);
        let none = BTreeSet::new();
        // Each driver's way in: the types, and the functions of no type,
        // that the lines alone bring it to.
        let ways_in: Vec<BTreeSet<At>> = drivers
            .iter()
            .map(|driver| {
                let walk = ground.walk(driver, false, &none, &none);
                walk.reached
                    .into_iter()
                    .filter(|at| match at {
                        At::Node(id) => self.type_ids.contains_key(*id),
                        At::Function(_) => true,
                    })
                    .collect()
            })
            .collect();
        drivers
            .iter()
            .enumerate()
            .map(|(at, driver)| {
                let theirs: BTreeSet<At> = ways_in
                    .iter()
                    .enumerate()
                    .filter(|(other, _)| *other != at)
                    .flat_map(|(_, way_in)| way_in.iter().copied())
                    .collect();
                let walk = ground.walk(driver, true, &theirs, &ways_in[at]);
                let mut shown: BTreeSet<String> =
                    [driver.clone(), vocabulary.top.center.clone()].into();
                // What the walk went on from, and the boxes that is in:
                // a box the walk is inside of is no end of it.
                let mut inside: BTreeSet<&str> = BTreeSet::new();
                for &at in &walk.reached {
                    if let Some(node) = ground.node(at) {
                        inside.extend(ground.chain(node));
                    }
                }
                for &at in walk.reached.iter().chain(&walk.ends) {
                    if let Some(node) = ground.node(at) {
                        shown.extend(ground.chain(node).into_iter().map(str::to_owned));
                    }
                }
                Driven {
                    driver: driver.clone(),
                    shown,
                    ends: walk
                        .ends
                        .iter()
                        .filter_map(|at| match at {
                            At::Node(id) if !inside.contains(id) => Some((*id).to_owned()),
                            _ => None,
                        })
                        .collect(),
                }
            })
            .collect()
    }
}

impl Driven {
    /// The vocabulary with what the driver does not reach left out: the
    /// top's nodes and each level's parts that it shows, each level's
    /// parts closed up over those left out, and the lines between what
    /// is left. Every node stays, placed or not, as each owns its code
    /// whoever drives.
    pub(super) fn cut(&self, vocabulary: &Vocabulary) -> Vocabulary {
        let shown = |id: &String| self.shown.contains(id);
        let kept = |lines: &[vocabulary::Line]| -> Vec<vocabulary::Line> {
            lines
                .iter()
                .filter(|line| shown(&line.from) && shown(&line.to))
                .cloned()
                .collect()
        };
        let top = &vocabulary.top;
        let levels = vocabulary
            .levels
            .iter()
            .filter(|(opened, level)| shown(opened) && level.parts.keys().any(shown))
            .map(|(opened, level)| {
                let mut level = layout::closed_up(vocabulary, level, &self.shown);
                level.lines = kept(&level.lines);
                (opened.clone(), level)
            })
            .collect();
        Vocabulary {
            says: vocabulary.says.clone(),
            nodes: vocabulary.nodes.clone(),
            top: vocabulary::Top {
                center: top.center.clone(),
                drivers: std::iter::once(&self.driver)
                    .chain(
                        top.drivers
                            .iter()
                            .filter(|id| **id != self.driver && shown(id)),
                    )
                    .cloned()
                    .collect(),
                left: top.left.iter().filter(|id| shown(id)).cloned().collect(),
                right: top.right.iter().filter(|id| shown(id)).cloned().collect(),
                lines: kept(&top.lines),
            },
            levels,
            agreed: Vec::new(),
        }
    }
}
