//! The code, as the map reads it from an index: each symbol once, with
//! what it is; each file's occurrences; and where in each file the
//! functions, types and tests are, as spans of lines.

use std::collections::{BTreeSet, HashMap, HashSet};

use prost::Message;

use super::Sources;
use super::conventions::{self, Language};
use crate::scip::symbol::{self, Descriptor, Suffix, Symbol};
use crate::scip::{Document, Encoding, Index, Span, SymbolInformation, kind, role};

/// A symbol, by its place in [`Code::symbols`].
pub(super) type SymbolId = usize;

/// What a symbol is to the map, beyond what it is.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum Role {
    Function,
    Field,
}

pub(super) struct Code<'a> {
    /// The files, by path.
    pub documents: Vec<Doc<'a>>,
    pub symbols: Vec<Sym<'a>>,
    ids: HashMap<&'a str, SymbolId>,
    /// The names of the project's own packages.
    pub packages: BTreeSet<String>,
}

pub(super) struct Doc<'a> {
    pub path: &'a str,
    /// Its lines, if its text could be read.
    pub lines: Vec<String>,
    pub language: Option<Language>,
    /// Whether the indexer that wrote it marks writes and imports by their
    /// roles; where it does not, conventions stand in. An index can hold
    /// several indexers' documents, each telling it its own way.
    pub writes_by_role: bool,
    pub imports_by_role: bool,
    pub encoding: Encoding,
    pub occurrences: Vec<Occ>,
    /// The functions, and the terms code runs in, by the lines they span.
    pub functions: Vec<Spanned>,
    pub types: Vec<Spanned>,
    /// The lines of the namespaces that are tests', as `mod tests`.
    pub tests: Vec<(usize, usize)>,
    /// The module most of its definitions are in.
    pub module: String,
    /// The line of its first definition of a thing, before which every
    /// reference is taken for an import.
    pub first_definition: Option<usize>,
}

/// An occurrence that reads.
pub(super) struct Occ {
    /// Its symbol, unless a local one.
    pub symbol: Option<SymbolId>,
    pub span: Span,
    pub enclosing: Option<Span>,
    pub roles: i32,
}

/// Lines from `start` to `end`, both in, that a symbol's definition spans.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) struct Spanned {
    pub start: usize,
    pub end: usize,
    pub symbol: SymbolId,
}

pub(super) struct Sym<'a> {
    pub text: &'a str,
    pub parsed: Option<Symbol<'a>>,
    /// Its module: the package's name, then the namespaces it starts with.
    pub module: String,
    pub info: Option<&'a SymbolInformation>,
    pub kind: i32,
    /// Whether the project defines it, not a dependency.
    pub own: bool,
    pub is_type: bool,
    pub role: Option<Role>,
    /// Whether it is in a namespace that is tests'.
    pub in_tests: bool,
    /// Where it is defined first: the document and the occurrence.
    pub definition: Option<(usize, usize)>,
}

impl<'a> Sym<'a> {
    pub fn last(&self) -> Option<&Descriptor<'a>> {
        self.parsed.as_ref()?.descriptors.last()
    }

    /// Its name: the last descriptor's.
    pub fn name(&self) -> &str {
        self.last().map_or("", |last| last.name.as_ref())
    }

    /// Whether it is a function, or a method, by its kind or its suffix.
    pub fn function_like(&self) -> bool {
        self.role == Some(Role::Function)
            || self
                .last()
                .is_some_and(|last| last.suffix == Suffix::Method)
    }

    /// The package it is of, as the manager and the name.
    pub fn package(&self) -> Option<(&str, &str)> {
        let parsed = self.parsed.as_ref()?;
        Some((parsed.manager.as_ref(), parsed.package.as_ref()))
    }
}

/// What kind of type a symbol of this kind is, by name, if it is one.
pub(super) fn type_kind(kind: i32) -> Option<&'static str> {
    Some(match kind {
        kind::CLASS => "class",
        kind::ENUM => "enum",
        kind::INTERFACE => "interface",
        kind::STRUCT => "struct",
        kind::TRAIT => "trait",
        kind::PROTOCOL => "protocol",
        kind::UNION => "union",
        kind::TYPE_ALIAS => "type alias",
        kind::TYPE => "type",
        kind::OBJECT => "object",
        kind::SINGLETON_CLASS => "singleton class",
        kind::CONTRACT => "contract",
        kind::MIXIN => "mixin",
        kind::TYPE_CLASS => "type class",
        _ => return None,
    })
}

fn field_kind(kind: i32) -> bool {
    matches!(
        kind,
        kind::FIELD
            | kind::PROPERTY
            | kind::ENUM_MEMBER
            | kind::STATIC_FIELD
            | kind::STATIC_PROPERTY
            | kind::CONSTANT
            | kind::STATIC_DATA_MEMBER
            | kind::EVENT
            | kind::STATIC_EVENT
    )
}

fn function_kind(kind: i32) -> bool {
    matches!(
        kind,
        kind::METHOD
            | kind::FUNCTION
            | kind::STATIC_METHOD
            | kind::TRAIT_METHOD
            | kind::ABSTRACT_METHOD
            | kind::METHOD_SPECIFICATION
            | kind::PROTOCOL_METHOD
            | kind::PURE_VIRTUAL_METHOD
            | kind::TYPE_CLASS_METHOD
            | kind::CONSTRUCTOR
            | kind::GETTER
            | kind::SETTER
            | kind::ACCESSOR
            | kind::SINGLETON_METHOD
            | kind::METHOD_ALIAS
    )
}

/// Whether `module` is `claim` or in it: its namespaces start with
/// those of the claim, whichever of `/`, `.` and `:` parts them.
pub(super) fn under(module: &str, claim: &str) -> bool {
    module == claim
        || (module.starts_with(claim)
            && matches!(module.as_bytes()[claim.len()], b'/' | b'.' | b':'))
}

/// The parts of a module's id, as `cox/daemon` has `cox` and `daemon`.
pub(super) fn module_parts(module: &str) -> impl Iterator<Item = &str> {
    module
        .split(['/', '.', ':'])
        .filter(|part| !part.is_empty())
}

impl Sym<'_> {
    /// The names it is known by, from its package down, as a rule names
    /// it: its module's parts, then those of what it is in, and its own,
    /// a method of an `impl` block of rust-analyzer's being its type's.
    pub fn names(&self) -> Vec<&str> {
        let mut names: Vec<&str> = module_parts(&self.module).collect();
        let Some(parsed) = &self.parsed else {
            return names;
        };
        let inner = &parsed.descriptors[namespaces(parsed).len()..];
        let Some((last, before)) = inner.split_last() else {
            return names;
        };
        let before = match conventions::impl_block(before) {
            Some((block, type_name, _)) => {
                names.extend(named(&before[..block]));
                names.push(type_name);
                &[]
            }
            None => before,
        };
        names.extend(named(before));
        names.extend(named(std::slice::from_ref(last)));
        names
    }
}

/// The names of `descriptors` a rule names a thing by: its types', its
/// terms', its methods' and its macros'.
fn named<'d>(descriptors: &'d [Descriptor<'_>]) -> impl Iterator<Item = &'d str> {
    descriptors
        .iter()
        .filter(|descriptor| {
            matches!(
                descriptor.suffix,
                Suffix::Type | Suffix::Term | Suffix::Method | Suffix::Macro
            )
        })
        .map(|descriptor| descriptor.name.as_ref())
}

/// The namespaces a symbol's descriptors start with.
fn namespaces<'s>(parsed: &'s Symbol<'_>) -> Vec<&'s str> {
    parsed
        .descriptors
        .iter()
        .take_while(|descriptor| descriptor.suffix == Suffix::Namespace)
        .map(|descriptor| descriptor.name.as_ref())
        .collect()
}

/// A symbol's module: by its package and the namespaces its descriptors
/// start with, but for the library target it names first, when its
/// package's symbols all name their targets, `names_targets` (see
/// [`conventions::module_id`] and [`conventions::modules`]).
fn module_of(parsed: &Symbol<'_>, names_targets: bool) -> String {
    conventions::module_id(
        &parsed.package,
        conventions::modules(names_targets, &namespaces(parsed)),
    )
}

/// The packages, by their scheme, manager and name, whose symbols that
/// start with a namespace all name the Cargo target they are of first,
/// as rust-analyzer's newer releases write them, of `symbols` (see
/// [`conventions::names_target`]).
fn naming_targets<'s>(symbols: &'s [Sym<'_>]) -> HashSet<(&'s str, &'s str, &'s str)> {
    let mut packages: HashMap<(&str, &str, &str), bool> = HashMap::new();
    for parsed in symbols.iter().filter_map(|sym| sym.parsed.as_ref()) {
        let namespaces = namespaces(parsed);
        if namespaces.is_empty() {
            continue;
        }
        let names = conventions::names_target(&parsed.scheme, &namespaces);
        packages
            .entry((&parsed.scheme, &parsed.manager, &parsed.package))
            .and_modify(|all| *all &= names)
            .or_insert(names);
    }
    packages
        .into_iter()
        .filter_map(|(package, all)| all.then_some(package))
        .collect()
}

impl<'a> Code<'a> {
    pub fn read(index: &'a Index, sources: &(impl Sources + ?Sized)) -> Self {
        let mut documents: Vec<&Document> = index.documents.iter().collect();
        // By path, and two of one path by all they hold, so that the order
        // the indexer wrote them in makes no difference.
        documents.sort_by(|a, b| {
            a.relative_path
                .cmp(&b.relative_path)
                .then_with(|| a.encode_to_vec().cmp(&b.encode_to_vec()))
        });
        let mut code = Code {
            documents: Vec::new(),
            symbols: Vec::new(),
            ids: HashMap::new(),
            packages: BTreeSet::new(),
        };
        for document in &documents {
            for information in &document.symbols {
                let id = code.intern(&information.symbol);
                code.symbols[id].info.get_or_insert(information);
            }
        }
        // What a type implements may be named nowhere else, as an
        // interface of Go's a type satisfies.
        for document in &documents {
            for information in &document.symbols {
                for relationship in &information.relationships {
                    code.intern(&relationship.symbol);
                }
            }
        }
        for document in &documents {
            let occurrences = document
                .occurrences
                .iter()
                .filter_map(|occurrence| {
                    Some(Occ {
                        symbol: (!occurrence.symbol.is_empty())
                            .then(|| code.intern(&occurrence.symbol)),
                        span: occurrence.span()?,
                        enclosing: occurrence.enclosing(),
                        roles: occurrence.symbol_roles,
                    })
                })
                .collect();
            code.documents.push(Doc {
                path: &document.relative_path,
                lines: lines(document, sources),
                language: None,
                writes_by_role: false,
                imports_by_role: false,
                encoding: document.encoding(),
                occurrences,
                functions: Vec::new(),
                types: Vec::new(),
                tests: Vec::new(),
                module: String::new(),
                first_definition: None,
            });
        }
        code.know_symbols();
        code.know_indexers(&documents);
        code.span_documents();
        code
    }

    /// Finds each document's language, and whether the indexer that wrote
    /// it, by the scheme of its symbols, marks writes and imports.
    fn know_indexers(&mut self, documents: &[&Document]) {
        let schemes: Vec<Option<&str>> = self
            .documents
            .iter()
            .map(|document| {
                document.occurrences.iter().find_map(|occurrence| {
                    let parsed = self.symbols[occurrence.symbol?].parsed.as_ref()?;
                    Some(parsed.scheme.as_ref())
                })
            })
            .collect();
        let mut roles: HashMap<Option<&str>, i32> = HashMap::new();
        for (document, &scheme) in self.documents.iter().zip(&schemes) {
            let marked = document
                .occurrences
                .iter()
                .fold(0, |roles, occurrence| roles | occurrence.roles);
            *roles.entry(scheme).or_default() |= marked;
        }
        let known: Vec<(Option<Language>, i32)> = documents
            .iter()
            .zip(&schemes)
            .map(|(document, &scheme)| (Language::of(&document.language, scheme), roles[&scheme]))
            .collect();
        for (document, (language, roles)) in self.documents.iter_mut().zip(known) {
            document.language = language;
            document.writes_by_role = roles & role::WRITE_ACCESS != 0;
            document.imports_by_role = roles & role::IMPORT != 0;
        }
    }

    fn intern(&mut self, text: &'a str) -> SymbolId {
        if let Some(&id) = self.ids.get(text) {
            return id;
        }
        let id = self.symbols.len();
        self.symbols.push(Sym {
            text,
            parsed: None,
            module: String::new(),
            info: None,
            kind: 0,
            own: false,
            is_type: false,
            role: None,
            in_tests: false,
            definition: None,
        });
        self.ids.insert(text, id);
        id
    }

    /// The symbol whose text is `text`, if the index has it.
    pub fn id(&self, text: &str) -> Option<SymbolId> {
        self.ids.get(text).copied()
    }

    /// Reads each symbol, and finds which are the project's own: those of
    /// the packages its files define symbols of.
    fn know_symbols(&mut self) {
        for sym in &mut self.symbols {
            sym.parsed = symbol::parse(sym.text);
            sym.kind = sym.info.map_or(0, |info| info.kind);
        }
        let modules: Vec<String> = {
            let naming = naming_targets(&self.symbols);
            self.symbols
                .iter()
                .map(|sym| {
                    sym.parsed.as_ref().map_or_default(|parsed| {
                        let package = (&*parsed.scheme, &*parsed.manager, &*parsed.package);
                        module_of(parsed, naming.contains(&package))
                    })
                })
                .collect()
        };
        for (sym, module) in self.symbols.iter_mut().zip(modules) {
            if let Some(parsed) = &sym.parsed {
                sym.module = module;
                sym.is_type = is_type(parsed, sym.kind);
                sym.role = role_of(parsed, sym.kind);
                sym.in_tests = parsed.descriptors.iter().any(|descriptor| {
                    descriptor.suffix == Suffix::Namespace
                        && conventions::test_namespace(&descriptor.name)
                });
            }
        }
        let mut own: HashSet<(String, String)> = HashSet::new();
        for (at, document) in self.documents.iter().enumerate() {
            for (index, occurrence) in document.occurrences.iter().enumerate() {
                let Some(id) = occurrence.symbol else {
                    continue;
                };
                if occurrence.roles & role::DEFINITION == 0 {
                    continue;
                }
                let sym = &mut self.symbols[id];
                if let Some((manager, package)) = sym.package() {
                    own.insert((manager.to_owned(), package.to_owned()));
                    sym.definition.get_or_insert((at, index));
                }
            }
        }
        for sym in &mut self.symbols {
            sym.own = sym.package().is_some_and(|(manager, package)| {
                own.contains(&(manager.to_owned(), package.to_owned()))
            });
        }
        self.packages = own.into_iter().map(|(_, package)| package).collect();
    }

    /// Finds in each document where its functions, types and tests are,
    /// the module it is, and where its imports end.
    fn span_documents(&mut self) {
        for at in 0..self.documents.len() {
            let document = &self.documents[at];
            let mut definitions: Vec<(usize, SymbolId, Option<Span>)> = document
                .occurrences
                .iter()
                .filter(|occurrence| occurrence.roles & role::DEFINITION != 0)
                .filter_map(|occurrence| {
                    let id = occurrence.symbol?;
                    self.symbols[id].parsed.as_ref()?;
                    Some((occurrence.span.start_line, id, occurrence.enclosing))
                })
                .collect();
            definitions.sort_by_key(|&(line, _, _)| line);
            let marks: Vec<usize> = definitions
                .iter()
                .filter(|&&(_, id, _)| self.symbols[id].function_like() || self.symbols[id].is_type)
                .map(|&(line, _, _)| line)
                .collect();
            let last_line = if document.lines.is_empty() {
                definitions
                    .iter()
                    .map(|&(line, _, _)| line)
                    .max()
                    .unwrap_or(0)
            } else {
                document.lines.len() - 1
            };
            let (mut functions, mut types, mut tests) = (Vec::new(), Vec::new(), Vec::new());
            for &(line, id, enclosing) in &definitions {
                let sym = &self.symbols[id];
                let (start, end) = match enclosing {
                    Some(span) if span.one_line => (span.start_line, span.start_line),
                    Some(span) => (span.start_line, span.end_line.max(span.start_line)),
                    // Without its enclosing range, a definition is taken to
                    // run up to the next function or type.
                    None => (
                        line,
                        marks
                            .iter()
                            .find(|&&mark| mark > line)
                            .map_or(last_line, |&next| next - 1),
                    ),
                };
                let spanned = Spanned {
                    start,
                    end,
                    symbol: id,
                };
                let last = sym.last().map(|last| last.suffix);
                if enclosing.is_some() && last == Some(Suffix::Namespace) && sym.in_tests {
                    tests.push((start, end));
                }
                if sym.function_like() {
                    functions.push(spanned);
                } else if sym.is_type {
                    types.push(spanned);
                } else if enclosing.is_some()
                    && last == Some(Suffix::Term)
                    && conventions::runs_code(sym.kind, start, end)
                {
                    functions.push(spanned);
                }
            }
            let module = most_common(
                definitions
                    .iter()
                    .filter(|&&(_, id, _)| self.symbols[id].own)
                    .map(|&(_, id, _)| self.symbols[id].module.as_str()),
            );
            let first_definition = definitions
                .iter()
                .filter(|&&(_, id, _)| {
                    let sym = &self.symbols[id];
                    !matches!(
                        sym.last().map(|last| last.suffix),
                        Some(Suffix::Namespace | Suffix::Meta)
                    ) && !matches!(
                        sym.kind,
                        kind::MODULE | kind::FILE | kind::NAMESPACE | kind::PACKAGE
                    )
                })
                .map(|&(line, _, _)| line)
                .min();
            let document = &mut self.documents[at];
            document.functions = functions;
            document.types = types;
            document.tests = tests;
            document.module = module;
            document.first_definition = first_definition;
        }
    }

    /// Whether the line `line` of `document`, and `symbol` if given, is
    /// tests': its file, its symbol's namespace, or a namespace of tests
    /// around it.
    pub fn is_test(&self, document: usize, symbol: Option<SymbolId>, line: Option<usize>) -> bool {
        let document = &self.documents[document];
        conventions::test_path(document.path)
            || symbol.is_some_and(|id| self.symbols[id].in_tests)
            || line.is_some_and(|line| {
                document
                    .tests
                    .iter()
                    .any(|&(start, end)| start <= line && line <= end)
            })
    }

    /// Whether `occurrence` of `document` is an import: by its role, if
    /// its indexer marks them, or else by convention.
    pub fn is_import(&self, document: usize, occurrence: &Occ) -> bool {
        let document = &self.documents[document];
        if document.imports_by_role {
            occurrence.roles & role::IMPORT != 0
        } else {
            conventions::imported(occurrence.span.start_line, document.first_definition)
        }
    }

    /// The text of a line of `document` after a reference at `span`, for
    /// telling whether it writes; nothing when the reference spans lines.
    pub fn after<'d>(&'d self, document: usize, span: &Span) -> Option<&'d str> {
        let document = &self.documents[document];
        if !span.one_line {
            return None;
        }
        let line = document.lines.get(span.start_line)?;
        Some(&line[document.encoding.byte(line, span.end_character)..])
    }

    /// Whether a reference at `occurrence` of `document` writes what it
    /// names: by its role, if the index marks writes, or else by the text
    /// after it.
    pub fn writes(&self, document: usize, occurrence: &Occ, after: Option<&str>) -> bool {
        let document = &self.documents[document];
        if document.writes_by_role {
            return occurrence.roles & role::WRITE_ACCESS != 0;
        }
        match (document.language, after) {
            (Some(language), Some(after)) => language.writes(after),
            _ => false,
        }
    }
}

/// A document's lines: its text in the index, or else its file's.
fn lines(document: &Document, sources: &(impl Sources + ?Sized)) -> Vec<String> {
    let text = if document.text.is_empty() {
        sources.read(&document.relative_path)
    } else {
        Some(document.text.clone())
    };
    text.map_or_default(|text| text.split('\n').map(str::to_owned).collect())
}

/// Whether a symbol is a type of the code: a type, in namespaces and
/// types alone, not in a function, of a type's kind or of none.
fn is_type(parsed: &Symbol<'_>, kind: i32) -> bool {
    parsed
        .descriptors
        .last()
        .is_some_and(|last| last.suffix == Suffix::Type)
        && parsed
            .descriptors
            .iter()
            .all(|descriptor| matches!(descriptor.suffix, Suffix::Namespace | Suffix::Type))
        && (kind == 0 || type_kind(kind).is_some())
}

/// Whether a symbol is a function or a field: by its kind, if the index
/// gives one, or else by its suffix.
fn role_of(parsed: &Symbol<'_>, kind: i32) -> Option<Role> {
    let last = parsed.descriptors.last()?;
    if matches!(
        last.suffix,
        Suffix::Parameter
            | Suffix::TypeParameter
            | Suffix::Meta
            | Suffix::Macro
            | Suffix::Namespace
    ) {
        return None;
    }
    if function_kind(kind) {
        return Some(Role::Function);
    }
    if field_kind(kind) {
        return Some(Role::Field);
    }
    if kind != 0 {
        return None;
    }
    match last.suffix {
        Suffix::Method => Some(Role::Function),
        Suffix::Term => Some(Role::Field),
        _ => None,
    }
}

/// The item most of `items` are, the first of those as many as it.
fn most_common<'s>(items: impl Iterator<Item = &'s str>) -> String {
    let mut counts: Vec<(&str, usize)> = Vec::new();
    for item in items {
        match counts.iter_mut().find(|(seen, _)| *seen == item) {
            Some((_, count)) => *count += 1,
            None => counts.push((item, 1)),
        }
    }
    let mut best: Option<(&str, usize)> = None;
    for (item, count) in counts {
        if best.is_none_or(|(_, most)| count > most) {
            best = Some((item, count));
        }
    }
    best.map_or_else(String::new, |(item, _)| item.to_owned())
}
