//! A map as generated: its vocabulary's nodes and levels, with what the
//! code's index says below the boxes that own code, and where the two
//! disagree. It is all a page needs to draw the map, and it comes out the
//! same, byte for byte, from the same index, vocabulary and files (see
//! [`super::generate`]). A page is told it in parts: its [`Outline`]
//! first, which the map is drawn from, then the [`Detail`] of each type
//! the human chooses.
//!
//! Every id is the index's: a module is its package and namespaces, as
//! `cox/daemon/chat`; a type is its module and its name, as
//! `cox/daemon/chat/Chat`; a field is its type's id, `.` and its name; a
//! function is its type's id, `::` and its name.

use std::collections::BTreeMap;

use serde::Serialize;

use super::vocabulary::{Outer, Side};

/// The map as generated.
#[derive(Clone, Debug, PartialEq, Serialize)]
pub struct Generated {
    pub says: String,
    /// The vocabulary's nodes, and a node for each type of a box that
    /// opens, by its type's id.
    pub nodes: BTreeMap<String, Node>,
    pub top: Top,
    /// The levels, by the node each is inside of: those the vocabulary
    /// placed, with the lines the code gives between their boxes, and one
    /// for each box that opens on its types.
    pub levels: BTreeMap<String, Level>,
    /// The types of the boxes that open, by id.
    pub types: BTreeMap<String, Type>,
    /// How often the code of each two boxes meets, outside tests.
    pub edges: Vec<Edge>,
    /// Where the vocabulary and the code disagree.
    pub drift: Vec<Drift>,
    /// The box that owns each file of the code, by its path from the
    /// project's folder, of the files some box owns: for what changed in
    /// a file to fall on its box. It stays with whoever drew the map, not
    /// in the JSON a page draws from.
    #[serde(skip)]
    pub files: BTreeMap<String, String>,
    /// What each driver of the top sets working, in the top's order: the
    /// map as it is drawn for that driver alone.
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub slices: Vec<Slice>,
}

/// What a page draws a map from before a type is chosen: all of it but
/// what only a type's detail shows, its documentation, its members and
/// where they are used, which is megabytes of a large project's map and
/// is told of one type at a time (see [`Generated::detail`]).
#[derive(Clone, Debug, Serialize)]
pub struct Outline<'m> {
    says: &'m str,
    nodes: &'m BTreeMap<String, Node>,
    top: &'m Top,
    levels: &'m BTreeMap<String, Level>,
    types: BTreeMap<&'m str, Head<'m>>,
    edges: &'m [Edge],
    drift: &'m [Drift],
    #[serde(skip_serializing_if = "<[Slice]>::is_empty")]
    slices: &'m [Slice],
}

/// A type as the map draws it: what it is and where, without its detail.
#[derive(Clone, Debug, Serialize)]
struct Head<'m> {
    id: &'m str,
    name: &'m str,
    module: &'m str,
    #[serde(rename = "box")]
    owner: &'m str,
    kind: &'m str,
    file: &'m str,
    line: usize,
}

impl Type {
    fn head(&self) -> Head<'_> {
        Head {
            id: &self.id,
            name: &self.name,
            module: &self.module,
            owner: &self.owner,
            kind: &self.kind,
            file: &self.file,
            line: self.line,
        }
    }
}

/// The detail of a type: all of it, and, for what its functions relate
/// to, the nodes that each field of another type they use names (see
/// [`Field::to`]), by the field's key, of the fields that name some.
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct Detail<'m> {
    #[serde(rename = "type")]
    pub ty: &'m Type,
    pub to: BTreeMap<&'m str, &'m [String]>,
}

impl Generated {
    /// The map without its types' details.
    #[must_use]
    pub fn outline(&self) -> Outline<'_> {
        Outline {
            says: &self.says,
            nodes: &self.nodes,
            top: &self.top,
            levels: &self.levels,
            types: self
                .types
                .iter()
                .map(|(id, ty)| (id.as_str(), ty.head()))
                .collect(),
            edges: &self.edges,
            drift: &self.drift,
            slices: &self.slices,
        }
    }

    /// The detail of the type `id`, if the map has it.
    #[must_use]
    pub fn detail(&self, id: &str) -> Option<Detail<'_>> {
        let ty = self.types.get(id)?;
        let to = ty
            .functions
            .iter()
            .flat_map(|function| &function.calls)
            .filter(|call| call.kind == CallKind::Field)
            .filter_map(|call| self.field(call.key.as_deref()?))
            .filter(|(owner, field)| owner.id != ty.id && !field.to.is_empty())
            .map(|(_, field)| (field.key.as_str(), field.to.as_slice()))
            .collect();
        Some(Detail { ty, to })
    }

    /// The field keyed `key`, and its type: a field's key is its type's
    /// id, `.` and its name.
    fn field(&self, key: &str) -> Option<(&Type, &Field)> {
        key.rmatch_indices('.').find_map(|(at, _)| {
            let ty = self.types.get(&key[..at])?;
            let field = ty.fields.iter().find(|field| field.key == key)?;
            Some((ty, field))
        })
    }
}

/// What a driver sets working: the map with what the driver does not
/// reach left out, placed anew, so that what is left sits together.
///
/// The driver starts at the types the lines a person drew from it go on
/// to, each line from the end that starts it, through the people and
/// systems on the way, and from there follows the code, the way it goes:
/// what a type's functions call and name, and what its fields hold. A
/// line a person drew from a box that owns code is followed once the code
/// comes into the box, as from one process to another. Coming to a type
/// another driver starts at, it shows the type and goes no further from
/// it.
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct Slice {
    /// The driver's node.
    pub driver: String,
    /// The top, with the driver alone of the drivers, and of the others
    /// around the center those it reaches.
    pub top: Top,
    /// The levels of what it reaches, by the node each is inside of: a
    /// box none of whose parts it reaches has none, and stays closed.
    pub levels: BTreeMap<String, Level>,
    /// Of the types it shows, those no other driver's slice shows.
    pub own: Vec<String>,
    /// The nodes its walk ended at, shown and not gone into: the types
    /// another driver starts at, and the ends of the lines that stop.
    pub ends: Vec<String>,
    /// How many boxes it shows, how many types, and how many people and
    /// systems around the project, itself not counted.
    pub boxes: usize,
    pub types: usize,
    pub outside: usize,
}

/// A node, as drawn.
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct Node {
    pub name: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub icon: Option<String>,
    pub kind: NodeKind,
    /// What it is to the code, drawn under its name: for a type, what
    /// kind of type, as `struct`.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub code: Option<String>,
    #[serde(skip_serializing_if = "std::ops::Not::not")]
    pub many: bool,
}

/// What a node is.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum NodeKind {
    /// A part of the project, as the vocabulary has it.
    Part,
    /// A person or a system around the project.
    Outside,
    /// A type of the code, inside the box that owns it.
    Type,
}

/// The top level, as the vocabulary has it.
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct Top {
    pub center: String,
    /// The drivers, which `left` starts with: the nodes that start the
    /// project working, each with a slice of the map.
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub drivers: Vec<String>,
    pub left: Vec<String>,
    pub right: Vec<String>,
    pub lines: Vec<Line>,
}

/// A level: one the vocabulary placed, with `area`, or one generated for
/// a box's types, with `size`, which grows out of the box where it sits.
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct Level {
    /// Where the node opened sits, in a placed level: `[x, y, width,
    /// height]`.
    #[serde(rename = "box", skip_serializing_if = "Option::is_none")]
    pub area: Option<[i32; 4]>,
    /// How big a generated level is: `[width, height]`; its parts sit
    /// from its top left.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub size: Option<[i32; 2]>,
    #[serde(skip_serializing_if = "BTreeMap::is_empty")]
    pub outer: BTreeMap<String, Outer>,
    #[serde(skip_serializing_if = "BTreeMap::is_empty")]
    pub at: BTreeMap<String, [i32; 2]>,
    #[serde(skip_serializing_if = "BTreeMap::is_empty")]
    pub frames: BTreeMap<String, [i32; 4]>,
    /// The nodes inside it, by where each sits: `[x, y]`.
    pub parts: BTreeMap<String, [i32; 2]>,
    pub lines: Vec<Line>,
}

/// A line between two nodes.
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct Line {
    pub from: String,
    pub to: String,
    /// Whether it is drawn both ways: it is `from` that starts it.
    #[serde(skip_serializing_if = "std::ops::Not::not")]
    pub both: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub sides: Option<[Side; 2]>,
    /// Whether it is drawn only while one of its ends is chosen: a line
    /// the code gives that is not among either end's two heaviest, or one
    /// between two types that no field gives, only a name in a function.
    #[serde(skip_serializing_if = "std::ops::Not::not")]
    pub weak: bool,
    /// Whether what a driver sets working ends at `to`.
    #[serde(skip_serializing_if = "std::ops::Not::not")]
    pub stops: bool,
    /// What draws it, for what a driver sets working to follow: it stays
    /// with whoever drew the map.
    #[serde(skip)]
    pub by: By,
}

/// What draws a line.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum By {
    /// A person, in the vocabulary, or a rule a person wrote there.
    #[default]
    Hand,
    /// The code.
    Code,
}

/// A type of a box that opens.
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct Type {
    pub id: String,
    pub name: String,
    pub module: String,
    /// The box that owns it.
    #[serde(rename = "box")]
    pub owner: String,
    /// What kind of type, as `struct` or `class`, or `type` when the
    /// index does not say.
    pub kind: String,
    /// Where it is defined: its file, from the project's folder, and the
    /// line, from 1.
    pub file: String,
    pub line: usize,
    /// The first sentence of its documentation.
    pub doc: String,
    pub fields: Vec<Field>,
    pub functions: Vec<Function>,
    /// Where it is named, outside its own members.
    pub uses: Vec<Use>,
}

/// A field of a type, or a variant of an enum.
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct Field {
    pub key: String,
    pub name: String,
    pub sig: String,
    pub line: usize,
    pub doc: String,
    pub uses: Vec<Use>,
    /// The nodes its declared type names: a type of its own box, by id,
    /// or another box.
    pub to: Vec<String>,
}

/// A function of a type.
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct Function {
    pub key: String,
    pub name: String,
    /// The trait or interface it implements, if it does.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub implements: Option<String>,
    pub sig: String,
    pub file: String,
    pub line: usize,
    pub doc: String,
    pub uses: Vec<Use>,
    /// What it calls, and the fields it uses, each once.
    pub calls: Vec<Call>,
}

/// A place a type or a member is named.
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct Use {
    pub file: String,
    pub line: usize,
    /// What it is named in: a function, as `Chat::attach`, or a type's
    /// declaration, as `Chat.agent`.
    #[serde(rename = "in")]
    pub within: String,
    /// Whether it is named in a type's declaration, rather than in a
    /// function.
    #[serde(skip_serializing_if = "std::ops::Not::not")]
    pub declaration: bool,
    /// The key of the function it is named in, if that is a function of a
    /// type of the map.
    #[serde(rename = "fn", skip_serializing_if = "Option::is_none")]
    pub function: Option<String>,
    pub module: String,
    /// The node it is named from: the type of the function, or the box
    /// that owns the code.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub node: Option<String>,
    #[serde(skip_serializing_if = "std::ops::Not::not")]
    pub test: bool,
    /// Whether it is written there, as by `=` or `.push(`.
    #[serde(skip_serializing_if = "std::ops::Not::not")]
    pub writes: bool,
}

/// What a function calls, or a field it uses.
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct Call {
    pub kind: CallKind,
    /// As `Chat::attach`, `chat::spawn` for a free function, or
    /// `Chat.agent` for a field.
    pub name: String,
    /// The key of the member, if it is one of a type of the map.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub key: Option<String>,
    /// The node it is on: its type, or the box that owns its code.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub node: Option<String>,
    /// The first line it is named on, from 1.
    pub line: usize,
    pub count: usize,
    #[serde(skip_serializing_if = "std::ops::Not::not")]
    pub writes: bool,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum CallKind {
    Function,
    Field,
}

/// How often the code of two boxes meets: the references from either
/// one's code to the other's, outside tests and imports.
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct Edge {
    pub a: String,
    pub b: String,
    pub count: usize,
}

/// A place the vocabulary and the code disagree, to talk through: to fix
/// the vocabulary, change the code, or agree to leave it.
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct Drift {
    /// What it is about, stable from one generation to the next: its
    /// kind, `:`, and what it is of.
    pub id: String,
    pub kind: DriftKind,
    /// The nodes it is on.
    pub nodes: Vec<String>,
    pub text: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub count: Option<usize>,
    /// Places in the code that show it.
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub at: Vec<Site>,
    /// Why it was agreed to leave it, if it was.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub agreed: Option<String>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "kebab-case")]
pub enum DriftKind {
    /// A box claims a module or a file the code no longer has.
    ClaimGone,
    /// A box that opens owns no types: its code is only functions of
    /// types other boxes own.
    NoTypes,
    /// No box owns a module that the boxes use often.
    Unowned,
    /// The code of two boxes meets, but no level shows them together.
    Undrawn,
    /// A line's end at a box is a rule that picks none of its types.
    RuleEmpty,
    /// An agreement that no drift matches any longer.
    AgreedStale,
}

/// A place in the code.
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct Site {
    pub file: String,
    pub line: usize,
    /// The function it is in, if any.
    #[serde(rename = "in", skip_serializing_if = "String::is_empty")]
    pub within: String,
}
