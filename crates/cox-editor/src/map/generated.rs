//! A map as generated: its vocabulary's nodes and levels, with what the
//! code's index says below the boxes that own code, and where the two
//! disagree. It is all a page needs to draw the map, with nothing to ask
//! back, and it comes out the same, byte for byte, from the same index,
//! vocabulary and files (see [`super::generate`]).
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
    /// Whether it goes both ways.
    #[serde(skip_serializing_if = "std::ops::Not::not")]
    pub both: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub sides: Option<[Side; 2]>,
    /// Whether it is drawn only while one of its ends is chosen: a line
    /// the code gives that is not among either end's two heaviest.
    #[serde(skip_serializing_if = "std::ops::Not::not")]
    pub weak: bool,
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
