//! Where the vocabulary and the code disagree, each as a drift with an
//! id that stays the same from one generation to the next, so that an
//! agreement to leave it names it.

use std::collections::{BTreeMap, BTreeSet};

use super::Map;
use super::code::under;
use crate::map::generated::{Drift, DriftKind, Level, Site};

/// How often the boxes must use a module no box owns for that to be
/// drift: fewer is a helper used in passing.
const UNOWNED_AT: usize = 20;
/// How many places in the code a drift shows.
const SITES: usize = 3;

/// The drift of `map`, whose levels are `levels`: claims the code no
/// longer has, boxes with no types of their own, code two boxes share
/// that no level shows, modules no box owns, and agreements no drift
/// matches.
pub(super) fn find(map: &Map<'_, '_>, levels: &BTreeMap<String, Level>) -> Vec<Drift> {
    let mut drift = claims(map);
    drift.extend(no_types(map));
    drift.extend(undrawn(map, levels));
    drift.extend(unowned(map));
    let agreed: BTreeMap<&str, &str> = map
        .vocabulary
        .agreed
        .iter()
        .map(|agreement| (agreement.id.as_str(), agreement.why.as_str()))
        .collect();
    let found: BTreeSet<String> = drift.iter().map(|drift| drift.id.clone()).collect();
    for &id in agreed.keys() {
        if !found.contains(id) {
            drift.push(Drift {
                id: format!("agreed-stale:{id}"),
                kind: DriftKind::AgreedStale,
                nodes: Vec::new(),
                text: format!("The drift agreed as {id} no longer happens"),
                count: None,
                at: Vec::new(),
                agreed: None,
            });
        }
    }
    for drift in &mut drift {
        drift.agreed = agreed.get(drift.id.as_str()).map(|&why| why.to_owned());
    }
    drift
}

/// The name of the node `node`.
fn name<'v>(map: &'v Map<'_, '_>, node: &'v str) -> &'v str {
    map.vocabulary
        .nodes
        .get(node)
        .map_or(node, |node| node.name.as_str())
}

/// The claims of modules and files the code no longer has.
fn claims(map: &Map<'_, '_>) -> Vec<Drift> {
    let code = map.code;
    let modules: BTreeSet<&str> = code
        .documents
        .iter()
        .map(|document| document.module.as_str())
        .chain(
            code.symbols
                .iter()
                .filter(|sym| sym.own && sym.definition.is_some())
                .map(|sym| sym.module.as_str()),
        )
        .collect();
    let mut claims: Vec<(&str, &str, bool)> = map
        .module_claims
        .iter()
        .map(|&(claim, owner)| (claim, owner, false))
        .chain(
            map.file_claims
                .iter()
                .map(|&(file, owner)| (file, owner, true)),
        )
        .collect();
    claims.sort_unstable();
    claims
        .into_iter()
        .filter(|&(claim, _, file)| {
            if file {
                !code.documents.iter().any(|document| {
                    let path = document.path;
                    path == claim
                        || (path.starts_with(claim)
                            && (claim.ends_with('/') || path.as_bytes()[claim.len()] == b'/'))
                })
            } else {
                !modules.iter().any(|module| under(module, claim))
            }
        })
        .map(|(claim, owner, _)| Drift {
            id: format!("claim-gone:{claim}"),
            kind: DriftKind::ClaimGone,
            nodes: vec![owner.to_owned()],
            text: format!(
                "{} claims {claim}, which the code no longer has",
                name(map, owner)
            ),
            count: None,
            at: Vec::new(),
            agreed: None,
        })
        .collect()
}

/// The boxes that open but own no types: their code is only functions of
/// types other boxes own, as methods put in a module of their own.
fn no_types(map: &Map<'_, '_>) -> Vec<Drift> {
    let code = map.code;
    let mut drift = Vec::new();
    for (id, node) in &map.vocabulary.nodes {
        let owns_code = !node.claims.is_empty() || !node.files.is_empty();
        if !owns_code || !node.open || map.types.iter().any(|ty| ty.owner == id) {
            continue;
        }
        let mut of_types = BTreeSet::new();
        let mut free = false;
        for (symbol, sym) in code.symbols.iter().enumerate() {
            let Some((document, occurrence)) = sym.definition else {
                continue;
            };
            if !sym.own || !sym.function_like() || map.box_of_symbol(symbol) != Some(id.as_str()) {
                continue;
            }
            if let Some((index, _)) = map.owner(symbol) {
                of_types.insert(map.types[index].name.as_str());
            } else {
                let line = code.documents[document].occurrences[occurrence]
                    .span
                    .start_line;
                free |= !code.is_test(document, Some(symbol), Some(line));
            }
        }
        if free || of_types.is_empty() {
            continue;
        }
        let of_types: Vec<&str> = of_types.into_iter().collect();
        drift.push(Drift {
            id: format!("no-types:{id}"),
            kind: DriftKind::NoTypes,
            nodes: vec![id.clone()],
            text: format!(
                "{} owns no types of its own, only functions of {}",
                node.name,
                of_types.join(", ")
            ),
            count: None,
            at: Vec::new(),
            agreed: None,
        });
    }
    drift
}

/// The boxes whose code meets, whose lines come from the code, but that
/// no level shows a line between, the most often meeting first.
fn undrawn(map: &Map<'_, '_>, levels: &BTreeMap<String, Level>) -> Vec<Drift> {
    let vocabulary = map.vocabulary;
    let ends = |from: &str, to: &str| {
        if from < to {
            (from.to_owned(), to.to_owned())
        } else {
            (to.to_owned(), from.to_owned())
        }
    };
    let drawn: BTreeSet<(String, String)> = levels
        .values()
        .flat_map(|level| &level.lines)
        .map(|line| ends(&line.from, &line.to))
        .chain(
            vocabulary
                .top
                .lines
                .iter()
                .map(|line| ends(&line.from, &line.to)),
        )
        .collect();
    let generates = |id: &str| {
        vocabulary
            .nodes
            .get(id)
            .is_some_and(crate::map::vocabulary::Node::generates_lines)
    };
    let mut edges: Vec<(&(&str, &str), &usize)> = map
        .edges
        .iter()
        .filter(|((a, b), _)| generates(a) && generates(b))
        .filter(|((a, b), _)| !drawn.contains(&((*a).to_owned(), (*b).to_owned())))
        .collect();
    edges.sort_by(|x, y| y.1.cmp(x.1).then(x.0.cmp(y.0)));
    edges
        .into_iter()
        .map(|(&(a, b), &count)| Drift {
            id: format!("undrawn:{a}|{b}"),
            kind: DriftKind::Undrawn,
            nodes: vec![a.to_owned(), b.to_owned()],
            text: format!(
                "The code of {} and {} meets {count} times, but no level shows a line between them",
                name(map, a),
                name(map, b)
            ),
            count: Some(count),
            at: sites(map.edge_sites.get(&(a, b)).map_or(&[], Vec::as_slice)),
            agreed: None,
        })
        .collect()
}

/// A few of `sites` to show: in functions first, then by place, one for
/// each function.
fn sites(sites: &[Site]) -> Vec<Site> {
    let mut sorted: Vec<&Site> = sites.iter().collect();
    sorted.sort_by(|a, b| {
        a.within
            .is_empty()
            .cmp(&b.within.is_empty())
            .then(a.file.cmp(&b.file))
            .then(a.line.cmp(&b.line))
            .then(a.within.cmp(&b.within))
    });
    let mut seen = BTreeSet::new();
    sorted
        .into_iter()
        .filter(|site| seen.insert(site.within.as_str()))
        .take(SITES)
        .cloned()
        .collect()
}

/// The modules no box owns that the boxes whose lines come from the code
/// use often, but not a package's own top, the most used first.
fn unowned(map: &Map<'_, '_>) -> Vec<Drift> {
    let mut free: Vec<(&str, usize)> = map
        .free
        .iter()
        .filter(|&(&module, &count)| count >= UNOWNED_AT && !map.code.packages.contains(module))
        .map(|(&module, &count)| (module, count))
        .collect();
    free.sort_by(|x, y| y.1.cmp(&x.1).then(x.0.cmp(y.0)));
    free.into_iter()
        .map(|(module, count)| {
            let mut boxes = map.free_boxes.get(module).cloned().unwrap_or_default();
            boxes.sort_by_key(|&(_, count)| std::cmp::Reverse(count));
            boxes.truncate(SITES);
            let names: Vec<&str> = boxes.iter().map(|&(owner, _)| name(map, owner)).collect();
            Drift {
                id: format!("unowned:{module}"),
                kind: DriftKind::Unowned,
                nodes: boxes.iter().map(|&(owner, _)| owner.to_owned()).collect(),
                text: format!(
                    "No box claims {module}, yet {} use it {count} times",
                    names.join(", ")
                ),
                count: Some(count),
                at: Vec::new(),
                agreed: None,
            }
        })
        .collect()
}
