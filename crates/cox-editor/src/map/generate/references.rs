//! Every reference the index has, read once: where the code of each two
//! boxes meets, the modules no box owns, where each type and member is
//! used, and what each function calls.

use super::Map;
use super::code::{Role, SymbolId};
use crate::map::generated::{Call, CallKind, Site, Use};
use crate::map::vocabulary::Lines;
use crate::scip::role;
use crate::scip::symbol::Suffix;

impl<'c> Map<'c, '_> {
    /// Reads every reference once: how often the boxes' code meets, the
    /// modules no box owns, and where each type and member is used.
    pub(super) fn read_references(&mut self) {
        let code = self.code;
        for (document, doc) in code.documents.iter().enumerate() {
            for occurrence in &doc.occurrences {
                let Some(id) = occurrence.symbol else {
                    continue;
                };
                if occurrence.roles & role::DEFINITION != 0 || !code.symbols[id].own {
                    continue;
                }
                let line = occurrence.span.start_line;
                let function = self.inner(&doc.functions, line);
                let module = function.map_or(doc.module.as_str(), |function| {
                    code.symbols[function].module.as_str()
                });
                let site = self.box_of_site(document, module);
                let test = code.is_test(document, function, Some(line))
                    || occurrence.roles & role::TEST != 0;
                let import = function.is_none() && code.is_import(document, occurrence);
                if !test
                    && !import
                    && let Some(site) = site
                {
                    self.count_reference(id, site, document, line, function);
                }
                if import {
                    continue;
                }
                let Some(key) = self.key(id).map(str::to_owned) else {
                    continue;
                };
                let Some((within, declaration, function)) = self.within(document, line, function)
                else {
                    continue;
                };
                let writes =
                    code.writes(document, occurrence, code.after(document, &occurrence.span));
                let node = match &function {
                    Some(function) => self.key_type(function),
                    None => site.map(str::to_owned),
                };
                self.uses.entry(key).or_default().push(Use {
                    file: doc.path.to_owned(),
                    line: line + 1,
                    within,
                    declaration,
                    function,
                    module: module.to_owned(),
                    node,
                    test,
                    writes,
                });
            }
        }
    }

    /// Counts a reference to `id` from code of the box `site`: toward the
    /// edge between the two boxes, or, if no box owns what it names, the
    /// module's use.
    pub(super) fn count_reference(
        &mut self,
        id: SymbolId,
        site: &'c str,
        document: usize,
        line: usize,
        function: Option<SymbolId>,
    ) {
        let code = self.code;
        let sym = &code.symbols[id];
        match self.box_of_symbol(id) {
            Some(target) if target != site => {
                let pair = if site < target {
                    (site, target)
                } else {
                    (target, site)
                };
                *self.edges.entry(pair).or_default() += 1;
                *self.directed.entry((site, target)).or_default() += 1;
                let within = function.map_or_default(|function| self.short(function, false));
                self.edge_sites.entry(pair).or_default().push(Site {
                    file: code.documents[document].path.to_owned(),
                    line: line + 1,
                    within,
                });
            }
            None if self.node(site).lines != Lines::Hand
                && sym
                    .last()
                    .is_some_and(|last| last.suffix != Suffix::Namespace) =>
            {
                *self.free.entry(&sym.module).or_default() += 1;
                let boxes = self.free_boxes.entry(&sym.module).or_default();
                match boxes.iter_mut().find(|(seen, _)| *seen == site) {
                    Some((_, count)) => *count += 1,
                    None => boxes.push((site, 1)),
                }
            }
            _ => {}
        }
    }

    /// What a use at `line` of `document` is in: the function's name, or
    /// the type's and the field's whose declaration it is in; whether it is
    /// a declaration; and the key of the function, if it is a function of
    /// the map. Nothing when it is in neither.
    pub(super) fn within(
        &self,
        document: usize,
        line: usize,
        function: Option<SymbolId>,
    ) -> Option<(String, bool, Option<String>)> {
        if let Some(function) = function {
            let key = self
                .member_keys
                .get(&function)
                .filter(|key| self.functions.contains_key(*key))
                .cloned();
            return Some((self.short(function, false), false, key));
        }
        let ty = self.inner(&self.code.documents[document].types, line)?;
        let (name, field) = match self.type_symbols.get(&ty) {
            Some(&index) => {
                let ty = &self.types[index];
                let field = ty
                    .fields
                    .iter()
                    .find(|field| field.span.0 <= line && line <= field.span.1);
                (ty.name.clone(), field.map(|field| field.name.as_str()))
            }
            None => (self.code.symbols[ty].name().to_owned(), None),
        };
        let within = match field {
            Some(field) => format!("{name}.{field}"),
            None => name,
        };
        Some((within, true, None))
    }

    /// Finds what each function of the map calls, and the fields it uses.
    pub(super) fn read_calls(&mut self) {
        let code = self.code;
        let mut all = Vec::new();
        for (index, ty) in self.types.iter().enumerate() {
            for (at, function) in ty.functions.iter().enumerate() {
                let (start, end) = function.span;
                let document = function.document;
                let mut calls: Vec<(SymbolId, Call)> = Vec::new();
                for occurrence in &code.documents[document].occurrences {
                    let line = occurrence.span.start_line;
                    let Some(id) = occurrence.symbol else {
                        continue;
                    };
                    let sym = &code.symbols[id];
                    if occurrence.roles & role::DEFINITION != 0
                        || !sym.own
                        || line < start
                        || line > end
                        || id == function.symbol
                    {
                        continue;
                    }
                    let is_function = sym.role == Some(Role::Function);
                    let owner = if sym.role == Some(Role::Field) {
                        self.owner(id)
                    } else {
                        None
                    };
                    if !is_function && owner.is_none() {
                        continue;
                    }
                    let at = if let Some(at) = calls.iter().position(|(seen, _)| *seen == id) {
                        at
                    } else {
                        let owner = owner.map(|(index, _)| index);
                        calls.push((id, self.call(id, is_function, owner, line)));
                        calls.len() - 1
                    };
                    let call = &mut calls[at].1;
                    call.count += 1;
                    if !is_function && occurrence.span.one_line {
                        call.writes |= code.writes(
                            document,
                            occurrence,
                            code.after(document, &occurrence.span),
                        );
                    }
                }
                let mut calls: Vec<Call> = calls.into_iter().map(|(_, call)| call).collect();
                calls.sort_by(|a, b| a.line.cmp(&b.line).then_with(|| a.name.cmp(&b.name)));
                all.push((index, at, calls));
            }
        }
        for (index, at, calls) in all {
            self.types[index].functions[at].calls = calls;
        }
    }

    /// A call to `id` first named on `line`, of the type `owner` if it is
    /// a field.
    pub(super) fn call(
        &self,
        id: SymbolId,
        is_function: bool,
        owner: Option<usize>,
        line: usize,
    ) -> Call {
        let key = self
            .member_keys
            .get(&id)
            .filter(|key| self.functions.contains_key(*key) || self.fields.contains_key(*key))
            .cloned();
        let node = match &key {
            Some(key) => self.key_type(key),
            None => self.box_of_symbol(id).map(str::to_owned),
        };
        let name = match owner {
            Some(owner) if !is_function => {
                format!(
                    "{}.{}",
                    self.types[owner].name,
                    self.code.symbols[id].name()
                )
            }
            _ => self.short(id, true),
        };
        Call {
            kind: if is_function {
                CallKind::Function
            } else {
                CallKind::Field
            },
            name,
            key,
            node,
            line: line + 1,
            count: 0,
            writes: false,
        }
    }
}
