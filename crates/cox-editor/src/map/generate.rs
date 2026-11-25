//! Generating a map: from a vocabulary, which a person keeps, and the
//! index of the project's code, which an indexer writes, everything
//! below the boxes that own code, and the drift between the two (see
//! [`super::generated`]).
//!
//! A box owns the code its claims name (see [`Vocabulary`]), and the map
//! reads, from the index:
//!
//! - the types each box that opens owns, with their fields and
//!   functions, where each is used, and what each function calls;
//! - how often each two boxes' code meets, outside tests and imports,
//!   which gives the lines between the boxes whose lines come from the
//!   code, thicker the more it meets, each box's two heaviest drawn and
//!   the rest only while one end is chosen, a box drawn closed standing
//!   for the boxes inside it;
//! - inside each box that opens, a line between two of its types where
//!   a field of one names the other, and one to each box around, from
//!   the type that names it most, or that it names most;
//! - where the vocabulary and the code disagree, as drift.
//!
//! It reads the index as SCIP defines it, the same for every language,
//! but for the few conventions it holds to where the index does not say,
//! all in `conventions.rs`. The same index, vocabulary and files give the
//! same map, byte for byte.

mod code;
mod conventions;
mod drift;
mod inside;
mod layout;
mod members;
mod references;
mod text;

use std::collections::{BTreeMap, HashMap};
use std::path::Path;

use self::code::{Code, SymbolId, under};
use super::generated::{Call, Edge, Generated, Line, Node, NodeKind, Site, Top, Type, Use};
use super::vocabulary::{self, Vocabulary};
use crate::scip::Index;
use crate::scip::symbol::Suffix;

/// Where the map reads the text of the project's files from, by their
/// paths from the project's folder, when the index does not hold it.
pub trait Sources {
    /// The text of the file at `path`, if there is one.
    fn read(&self, path: &str) -> Option<String>;
}

/// The project's folder: a path that leaves it, as by `..` or a link to
/// elsewhere, is no file of the project.
impl Sources for Path {
    fn read(&self, path: &str) -> Option<String> {
        let root = std::fs::canonicalize(self).ok()?;
        let file = std::fs::canonicalize(root.join(path)).ok()?;
        if !file.starts_with(&root) {
            return None;
        }
        let bytes = std::fs::read(file).ok()?;
        Some(String::from_utf8_lossy(&bytes).into_owned())
    }
}

impl Sources for BTreeMap<String, String> {
    fn read(&self, path: &str) -> Option<String> {
        self.get(path).cloned()
    }
}

/// The map of the code `index` has, as `vocabulary` tells it, with the
/// text of its files from `sources` where the index does not hold it.
/// The files `vocabulary` claims are from the folder `files_from` of the
/// index's paths, with a `/` after it, as `directories/cox/` for a map a
/// repository of the project keeps, or empty: a file outside it is none
/// of theirs.
#[must_use]
pub fn generate(
    index: &Index,
    vocabulary: &Vocabulary,
    files_from: &str,
    sources: &(impl Sources + ?Sized),
) -> Generated {
    let code = Code::read(index, sources);
    let mut map = Map::new(&code, vocabulary, files_from);
    map.find_types();
    map.attach_members();
    map.read_references();
    map.read_calls();
    map.find_field_targets();
    map.into_generated()
}

/// A type of a box that opens, as it is found.
struct TypeRec<'c> {
    id: String,
    name: String,
    module: &'c str,
    owner: &'c str,
    kind: &'static str,
    document: usize,
    line: usize,
    doc: String,
    fields: Vec<FieldRec>,
    functions: Vec<FunctionRec>,
}

struct FieldRec {
    /// Its key, once all of its type's members are found.
    key: String,
    symbol: SymbolId,
    name: String,
    sig: String,
    line: usize,
    doc: String,
    /// The lines its definition spans.
    span: (usize, usize),
    to: Vec<String>,
}

struct FunctionRec {
    key: String,
    /// What its symbol tells it from others of its name by, as an
    /// overload's `+1`.
    disambiguator: String,
    name: String,
    implements: Option<String>,
    sig: String,
    line: usize,
    doc: String,
    symbol: SymbolId,
    document: usize,
    span: (usize, usize),
    calls: Vec<Call>,
}

/// The map as it is generated.
struct Map<'c, 'a> {
    code: &'c Code<'a>,
    vocabulary: &'c Vocabulary,
    /// The modules claimed, and the files, each with the box that owns it.
    module_claims: Vec<(&'c str, &'c str)>,
    file_claims: Vec<(&'c str, &'c str)>,
    /// The folder the files claimed are from (see [`generate`]).
    files_from: &'c str,
    /// The box that owns each module the code has, if one does.
    module_boxes: HashMap<&'c str, Option<&'c str>>,
    /// The node each node a level places is inside of.
    inside_of: BTreeMap<&'c str, &'c str>,
    types: Vec<TypeRec<'c>>,
    type_ids: HashMap<String, usize>,
    type_symbols: HashMap<SymbolId, usize>,
    /// The key of each member of a type of the map, by its symbol.
    member_keys: HashMap<SymbolId, String>,
    functions: HashMap<String, (usize, usize)>,
    fields: HashMap<String, (usize, usize)>,
    /// The types of the map, by their package and name.
    by_name: HashMap<(&'c str, &'c str, &'c str), Vec<usize>>,
    /// Where each type and member is named, by its key.
    uses: HashMap<String, Vec<Use>>,
    /// How often the code of each two boxes meets, as a pair in order,
    /// and from the first to the second.
    edges: BTreeMap<(&'c str, &'c str), usize>,
    directed: HashMap<(&'c str, &'c str), usize>,
    edge_sites: HashMap<(&'c str, &'c str), Vec<Site>>,
    /// How often boxes whose lines come from the code use each module no
    /// box owns, and which boxes, as often, in the order first seen.
    free: HashMap<&'c str, usize>,
    free_boxes: HashMap<&'c str, Vec<(&'c str, usize)>>,
}

impl<'c, 'a> Map<'c, 'a> {
    fn new(code: &'c Code<'a>, vocabulary: &'c Vocabulary, files_from: &'c str) -> Self {
        let mut module_claims = Vec::new();
        let mut file_claims = Vec::new();
        for (id, node) in &vocabulary.nodes {
            module_claims.extend(
                node.claims
                    .iter()
                    .map(|claim| (claim.as_str(), id.as_str())),
            );
            file_claims.extend(node.files.iter().map(|file| (file.as_str(), id.as_str())));
        }
        let mut map = Map {
            code,
            vocabulary,
            module_claims,
            file_claims,
            files_from,
            module_boxes: HashMap::new(),
            inside_of: vocabulary.inside_of(),
            types: Vec::new(),
            type_ids: HashMap::new(),
            type_symbols: HashMap::new(),
            member_keys: HashMap::new(),
            functions: HashMap::new(),
            fields: HashMap::new(),
            by_name: HashMap::new(),
            uses: HashMap::new(),
            edges: BTreeMap::new(),
            directed: HashMap::new(),
            edge_sites: HashMap::new(),
            free: HashMap::new(),
            free_boxes: HashMap::new(),
        };
        let modules = code.symbols.iter().map(|sym| sym.module.as_str()).chain(
            code.documents
                .iter()
                .map(|document| document.module.as_str()),
        );
        for module in modules {
            if !map.module_boxes.contains_key(module) {
                let owner = map.claimed_module(module);
                map.module_boxes.insert(module, owner);
            }
        }
        map
    }

    fn claimed_module(&self, module: &str) -> Option<&'c str> {
        self.module_claims
            .iter()
            .filter(|(claim, _)| under(module, claim))
            .max_by_key(|(claim, _)| claim.len())
            .map(|&(_, owner)| owner)
    }

    fn box_of_module(&self, module: &str) -> Option<&'c str> {
        match self.module_boxes.get(module) {
            Some(&owner) => owner,
            None => self.claimed_module(module),
        }
    }

    /// The box that owns the file at `path`, by the longest of the files
    /// claimed that it is, or is in.
    fn box_of_file(&self, path: &str) -> Option<&'c str> {
        self.file_claims
            .iter()
            .filter(|(file, _)| self.claims_file(file, path))
            .max_by_key(|(file, _)| file.len())
            .map(|&(_, owner)| owner)
    }

    /// Whether the claim of the file or folder `file` is of the file at
    /// `path`, from the index's folder: whether, from the folder the
    /// claims are from, it is the file, or is in the folder.
    fn claims_file(&self, file: &str, path: &str) -> bool {
        let Some(path) = path.strip_prefix(self.files_from) else {
            return false;
        };
        path == file
            || (path.starts_with(file)
                && (file.ends_with('/') || path.as_bytes()[file.len()] == b'/'))
    }

    /// The box that owns code in `document`, in `module`.
    fn box_of_site(&self, document: usize, module: &str) -> Option<&'c str> {
        self.box_of_file(self.code.documents[document].path)
            .or_else(|| self.box_of_module(module))
    }

    /// The box that owns a symbol: by the file it is defined in, or else
    /// by its module.
    fn box_of_symbol(&self, id: SymbolId) -> Option<&'c str> {
        let sym = &self.code.symbols[id];
        sym.definition
            .and_then(|(document, _)| self.box_of_file(self.code.documents[document].path))
            .or_else(|| self.box_of_module(&sym.module))
    }

    fn node(&self, id: &str) -> &'c vocabulary::Node {
        let vocabulary: &'c Vocabulary = self.vocabulary;
        &vocabulary.nodes[id]
    }

    /// The box drawn for the box `id` where those `shown` are: itself,
    /// if it is shown, or else the nearest box it is inside of that is,
    /// which stands for it there; none when a box on the way, itself
    /// first, keeps its lines by hand, whose lines stand for all of it.
    fn shown_as<'s>(&self, id: &'s str, shown: impl Fn(&str) -> bool) -> Option<&'s str>
    where
        'c: 's,
    {
        self.and_around(id).into_iter().find_map(|at| {
            if shown(at) {
                Some(Some(at))
            } else if self
                .vocabulary
                .nodes
                .get(at)
                .is_some_and(|node| node.lines == vocabulary::Lines::Hand)
            {
                Some(None)
            } else {
                None
            }
        })?
    }

    /// `id`, and the nodes it is inside of, from the nearest out.
    fn and_around<'s>(&self, id: &'s str) -> Vec<&'s str>
    where
        'c: 's,
    {
        let mut chain = vec![id];
        while let Some(&outer) = chain.last().and_then(|at| self.inside_of.get(at)) {
            if chain.contains(&outer) {
                break;
            }
            chain.push(outer);
        }
        chain
    }

    /// The innermost of `spans` around `line`: the shortest, then the
    /// last to start.
    fn inner(&self, spans: &[code::Spanned], line: usize) -> Option<SymbolId> {
        spans
            .iter()
            .filter(|spanned| spanned.start <= line && line <= spanned.end)
            .min_by(|a, b| {
                (a.end - a.start)
                    .cmp(&(b.end - b.start))
                    .then(b.start.cmp(&a.start))
                    .then(
                        self.code.symbols[a.symbol]
                            .text
                            .cmp(self.code.symbols[b.symbol].text),
                    )
            })
            .map(|spanned| spanned.symbol)
    }

    /// The key of the type or member `id` is, if it is one of the map.
    fn key(&self, id: SymbolId) -> Option<&str> {
        self.type_symbols
            .get(&id)
            .map(|&index| self.types[index].id.as_str())
            .or_else(|| self.member_keys.get(&id).map(String::as_str))
    }

    /// The type of the map a key is of: itself, or its member's type.
    fn key_type(&self, key: &str) -> Option<String> {
        if self.type_ids.contains_key(key) {
            return Some(key.to_owned());
        }
        self.functions
            .get(key)
            .or_else(|| self.fields.get(key))
            .map(|&(index, _)| self.types[index].id.clone())
    }

    /// The type of the map a member is of, and the trait it implements, if
    /// that is in its symbol: by the symbol the index says it is in, or by
    /// the type its descriptors end in, or, for rust-analyzer's `impl`
    /// blocks, by the one type of that name in its module or above it.
    fn owner(&self, id: SymbolId) -> Option<(usize, Option<String>)> {
        let sym = &self.code.symbols[id];
        if let Some(enclosing) = sym.info.map(|info| info.enclosing_symbol.as_str())
            && let Some(&index) = self
                .code
                .id(enclosing)
                .and_then(|enclosing| self.type_symbols.get(&enclosing))
        {
            return Some((index, None));
        }
        let parsed = sym.parsed.as_ref()?;
        let before = &parsed.descriptors[..parsed.descriptors.len().checked_sub(1)?];
        if let Some(last) = before.last()
            && last.suffix == Suffix::Type
        {
            let index = self
                .code
                .id(&sym.text[..last.end])
                .and_then(|outer| self.type_symbols.get(&outer))?;
            return Some((*index, None));
        }
        let (block, type_name, trait_) = conventions::impl_block(before)?;
        let block = &sym.text[..before[block].end];
        if self
            .code
            .id(block)
            .is_some_and(|block| self.key(block).is_some())
        {
            return None;
        }
        let candidates: Vec<usize> = self
            .by_name
            .get(&(parsed.manager.as_ref(), parsed.package.as_ref(), type_name))
            .into_iter()
            .flatten()
            .copied()
            .filter(|&index| under(&sym.module, self.types[index].module))
            .collect();
        match candidates[..] {
            [index] => Some((index, trait_.map(str::to_owned))),
            _ => None,
        }
    }

    /// A name to show for a symbol: `Type::method`, or a free function's
    /// name, after its module's last part when `with_module`.
    fn short(&self, id: SymbolId, with_module: bool) -> String {
        if let Some(&(index, function)) = self
            .member_keys
            .get(&id)
            .and_then(|key| self.functions.get(key))
        {
            let ty = &self.types[index];
            return format!("{}::{}", ty.name, ty.functions[function].name);
        }
        let sym = &self.code.symbols[id];
        let Some(parsed) = &sym.parsed else {
            return sym.text.to_owned();
        };
        let name = sym.name();
        if sym.function_like()
            && let Some((index, _)) = self.owner(id)
        {
            return format!("{}::{name}", self.types[index].name);
        }
        let before = &parsed.descriptors[..parsed.descriptors.len().saturating_sub(1)];
        match conventions::type_named(before) {
            Some(outer) => format!("{outer}::{name}"),
            None if with_module => format!(
                "{}::{name}",
                code::module_parts(&sym.module).last().unwrap_or("?")
            ),
            None => name.to_owned(),
        }
    }

    /// The map, as generated: the vocabulary's levels with the lines the
    /// code gives, a level for each box that opens, and the drift.
    fn into_generated(mut self) -> Generated {
        let placed = layout::place(&self);
        let mut levels = placed.levels;
        let mut edges = self.edges.clone();
        edges.extend(placed.standing);
        let mut nodes: BTreeMap<String, Node> = self
            .vocabulary
            .nodes
            .iter()
            .map(|(id, node)| (id.clone(), drawn(node)))
            .collect();
        for (owner, level) in self.type_levels(&placed.slots, &levels) {
            levels.insert(owner.to_owned(), level);
        }
        let drift = drift::find(&self, &levels);
        let files = self
            .code
            .documents
            .iter()
            .filter_map(|document| {
                let owner = self
                    .box_of_file(document.path)
                    .or_else(|| self.box_of_module(&document.module))?;
                Some((document.path.to_owned(), owner.to_owned()))
            })
            .collect();
        let mut types = BTreeMap::new();
        for ty in std::mem::take(&mut self.types) {
            nodes.insert(
                ty.id.clone(),
                Node {
                    name: ty.name.clone(),
                    icon: None,
                    kind: NodeKind::Type,
                    code: Some(ty.kind.to_owned()),
                    many: false,
                },
            );
            let output = self.output_type(ty);
            types.insert(output.id.clone(), output);
        }
        let top = &self.vocabulary.top;
        Generated {
            says: self.vocabulary.says.clone(),
            nodes,
            top: Top {
                center: top.center.clone(),
                left: top.left.clone(),
                right: top.right.clone(),
                lines: top.lines.iter().map(hand_line).collect(),
            },
            levels,
            types,
            edges: edges
                .iter()
                .map(|(&(a, b), &count)| Edge {
                    a: a.to_owned(),
                    b: b.to_owned(),
                    count,
                })
                .collect(),
            drift,
            files,
        }
    }

    fn output_type(&mut self, ty: TypeRec<'c>) -> Type {
        let mut uses = |key: &str| self.uses.remove(key).unwrap_or_default();
        let fields = ty
            .fields
            .into_iter()
            .map(|field| super::generated::Field {
                uses: uses(&field.key),
                key: field.key,
                name: field.name,
                sig: field.sig,
                line: field.line,
                doc: field.doc,
                to: field.to,
            })
            .collect();
        let functions = ty
            .functions
            .into_iter()
            .map(|function| super::generated::Function {
                uses: uses(&function.key),
                key: function.key,
                name: function.name,
                implements: function.implements,
                sig: function.sig,
                file: self.code.documents[function.document].path.to_owned(),
                line: function.line,
                doc: function.doc,
                calls: function.calls,
            })
            .collect();
        Type {
            uses: uses(&ty.id),
            id: ty.id,
            name: ty.name,
            module: ty.module.to_owned(),
            owner: ty.owner.to_owned(),
            kind: ty.kind.to_owned(),
            file: self.code.documents[ty.document].path.to_owned(),
            line: ty.line,
            doc: ty.doc,
            fields,
            functions,
        }
    }
}

/// A node of the vocabulary, as drawn: under its name, what it is to the
/// code, or else the first code it claims.
fn drawn(node: &vocabulary::Node) -> Node {
    Node {
        name: node.name.clone(),
        icon: node.icon.clone(),
        kind: match node.kind {
            vocabulary::Kind::Part => NodeKind::Part,
            vocabulary::Kind::Outside => NodeKind::Outside,
        },
        code: node
            .code
            .clone()
            .or_else(|| node.claims.first().or(node.files.first()).cloned()),
        many: node.many,
    }
}

fn hand_line(line: &vocabulary::Line) -> Line {
    Line {
        from: line.from.clone(),
        to: line.to.clone(),
        both: line.both,
        sides: line.sides,
        weak: false,
    }
}

#[cfg(test)]
mod tests;
