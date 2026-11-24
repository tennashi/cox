//! The types of the boxes that open, found in the index, with their
//! fields and functions, and what each field's type names.

use std::collections::{BTreeMap, BTreeSet, HashMap, HashSet};

use super::code::{self, Role, SymbolId};
use super::text::{doc_of, sig_of};
use super::{FieldRec, FunctionRec, Map, TypeRec};
use crate::scip::symbol::{self, Suffix};
use crate::scip::{SymbolInformation, role};

impl Map<'_, '_> {
    /// Finds the types of the boxes that open.
    pub(super) fn find_types(&mut self) {
        let code = self.code;
        let mut defined: Vec<SymbolId> = (0..code.symbols.len())
            .filter(|&id| code.symbols[id].definition.is_some())
            .collect();
        defined.sort_by_key(|&id| code.symbols[id].text);
        for id in defined {
            let sym = &code.symbols[id];
            let Some((document, occurrence)) = sym.definition else {
                continue;
            };
            let line = code.documents[document].occurrences[occurrence]
                .span
                .start_line;
            if !sym.is_type || !sym.own || code.is_test(document, Some(id), Some(line)) {
                continue;
            }
            let Some(owner) = self.box_of_symbol(id) else {
                continue;
            };
            if !self.node(owner).open {
                continue;
            }
            let chain: Vec<&str> = sym
                .parsed
                .iter()
                .flat_map(|parsed| &parsed.descriptors)
                .filter(|descriptor| descriptor.suffix == Suffix::Type)
                .map(|descriptor| descriptor.name.as_ref())
                .collect();
            let type_id = format!("{}/{}", sym.module, chain.join("."));
            if self.type_ids.contains_key(&type_id) {
                continue;
            }
            let index = self.types.len();
            self.type_ids.insert(type_id.clone(), index);
            self.type_symbols.insert(id, index);
            if let Some((manager, package)) = sym.package() {
                self.by_name
                    .entry((manager, package, sym.name()))
                    .or_default()
                    .push(index);
            }
            self.types.push(TypeRec {
                id: type_id,
                name: sym.name().to_owned(),
                module: &sym.module,
                owner,
                kind: code::type_kind(sym.kind).unwrap_or("type"),
                document,
                line: line + 1,
                doc: doc_of(sym.info),
                fields: Vec::new(),
                functions: Vec::new(),
            });
        }
    }

    /// Finds the fields and functions of the map's types.
    pub(super) fn attach_members(&mut self) {
        let code = self.code;
        let mut defined: Vec<(usize, usize, usize, SymbolId)> = (0..code.symbols.len())
            .filter_map(|id| {
                let (document, occurrence) = code.symbols[id].definition?;
                let line = code.documents[document].occurrences[occurrence]
                    .span
                    .start_line;
                Some((document, line, occurrence, id))
            })
            .collect();
        defined.sort_unstable();
        for (document, line, occurrence, id) in defined {
            let sym = &code.symbols[id];
            let Some(role) = sym.role.filter(|_| sym.own && !sym.in_tests) else {
                continue;
            };
            let Some((index, mut implements)) = self.owner(id) else {
                continue;
            };
            let name = sym.name().to_owned();
            let sig = sig_of(sym.info, &code.documents[document].lines, line);
            let doc = doc_of(sym.info);
            let ty = &mut self.types[index];
            if role == Role::Field {
                let span = code.documents[document].occurrences[occurrence]
                    .enclosing
                    .map_or((line, line), |span| {
                        let end = if span.one_line {
                            span.start_line
                        } else {
                            span.end_line
                        };
                        (span.start_line, end.max(span.start_line))
                    });
                ty.fields.push(FieldRec {
                    key: String::new(),
                    symbol: id,
                    name,
                    sig,
                    line: line + 1,
                    doc,
                    span,
                    to: Vec::new(),
                });
                continue;
            }
            if code.is_test(document, Some(id), Some(line)) {
                continue;
            }
            if implements.is_none() {
                implements = implemented(sym.info);
            }
            let span = code.documents[document]
                .functions
                .iter()
                .find(|spanned| spanned.symbol == id)
                .map_or((line, line), |spanned| (spanned.start, spanned.end));
            let disambiguator = sym.last().map_or("", |last| last.disambiguator).to_owned();
            ty.functions.push(FunctionRec {
                key: String::new(),
                disambiguator,
                name,
                implements,
                sig,
                line: line + 1,
                doc,
                symbol: id,
                document,
                span,
                calls: Vec::new(),
            });
        }
        for index in 0..self.types.len() {
            self.key_members(index);
        }
    }

    /// Gives each member of a type a key of its own: the type's id, `.` or
    /// `::`, and its name, and for a function, what tells it from others
    /// of its name: the disambiguator of an overload, as `(+1)`, and, of
    /// several of a name, `#` and the trait each implements; then, of any
    /// still alike, `#2`, `#3` in order.
    fn key_members(&mut self, index: usize) {
        let ty = &mut self.types[index];
        let mut taken = HashSet::new();
        let mut unique = |key: String| {
            let mut unique = key.clone();
            let mut count = 1;
            while !taken.insert(unique.clone()) {
                count += 1;
                unique = format!("{key}#{count}");
            }
            unique
        };
        for field in &mut ty.fields {
            field.key = unique(format!("{}.{}", ty.id, field.name));
        }
        let mut named: HashMap<(String, String), usize> = HashMap::new();
        for function in &ty.functions {
            *named
                .entry((function.name.clone(), function.disambiguator.clone()))
                .or_default() += 1;
        }
        for function in &mut ty.functions {
            let mut key = format!("{}::{}", ty.id, function.name);
            if !function.disambiguator.is_empty() {
                key = format!("{key}({})", function.disambiguator);
            }
            let shared = named[&(function.name.clone(), function.disambiguator.clone())] > 1;
            if let Some(trait_) = function.implements.as_ref().filter(|_| shared) {
                key = format!("{key}#{trait_}");
            }
            function.key = unique(key);
        }
        for (at, field) in ty.fields.iter().enumerate() {
            self.member_keys.insert(field.symbol, field.key.clone());
            self.fields.insert(field.key.clone(), (index, at));
        }
        for (at, function) in ty.functions.iter().enumerate() {
            self.member_keys
                .insert(function.symbol, function.key.clone());
            self.functions.insert(function.key.clone(), (index, at));
        }
    }

    /// Finds the nodes each field's declared type names: a type of its own
    /// box, or another box.
    pub(super) fn find_field_targets(&mut self) {
        let mut all = Vec::new();
        for (index, ty) in self.types.iter().enumerate() {
            let mut by_line: BTreeMap<usize, BTreeSet<SymbolId>> = BTreeMap::new();
            for occurrence in &self.code.documents[ty.document].occurrences {
                if let Some(id) = occurrence.symbol
                    && occurrence.roles & role::DEFINITION == 0
                    && self.code.symbols[id].own
                {
                    by_line
                        .entry(occurrence.span.start_line)
                        .or_default()
                        .insert(id);
                }
            }
            for (at, field) in ty.fields.iter().enumerate() {
                let mut to = BTreeSet::new();
                for &id in by_line
                    .range(field.span.0..=field.span.1)
                    .flat_map(|(_, ids)| ids)
                {
                    if let Some(&other) = self.type_symbols.get(&id) {
                        if other != index {
                            let other = &self.types[other];
                            to.insert(if other.owner == ty.owner {
                                other.id.clone()
                            } else {
                                other.owner.to_owned()
                            });
                        }
                    } else if let Some(owner) = self.box_of_symbol(id)
                        && owner != ty.owner
                    {
                        to.insert(owner.to_owned());
                    }
                }
                all.push((index, at, to.into_iter().collect()));
            }
        }
        for (index, at, to) in all {
            self.types[index].fields[at].to = to;
        }
    }
}

/// The trait or interface a method implements, by the index's
/// relationships: the last type in the symbol it implements.
fn implemented(info: Option<&SymbolInformation>) -> Option<String> {
    let relationship = info?
        .relationships
        .iter()
        .find(|relationship| relationship.is_implementation)?;
    let parsed = symbol::parse(&relationship.symbol)?;
    let before = &parsed.descriptors[..parsed.descriptors.len().saturating_sub(1)];
    before
        .iter()
        .rev()
        .find(|descriptor| descriptor.suffix == Suffix::Type)
        .map(|descriptor| descriptor.name.clone().into_owned())
}
