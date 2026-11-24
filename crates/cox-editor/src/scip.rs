//! A SCIP index: what an indexer, such as rust-analyzer's `scip` or
//! `scip-typescript`, writes of a project's code, as `index.scip`. It has
//! each file's occurrences of symbols, where each is defined and named,
//! and what each symbol is. It reads the same whatever the language, so
//! the project's map is drawn from it (see [`crate::map::generate`]).
//!
//! The messages here are those of `scip.proto` the map reads, with the
//! fields it reads, by their numbers there; protobuf skips the others, as
//! the metadata and what the indexer knows of the dependencies' symbols.
//! Their names follow the proto's. An index reads with
//! [`prost::Message::decode`].

pub mod symbol;

use prost::Message;

/// What an indexer wrote: the files of the project it indexed, each with
/// what is defined and named in it.
#[derive(Clone, PartialEq, Message)]
pub struct Index {
    #[prost(message, repeated, tag = "2")]
    pub documents: Vec<Document>,
}

/// A file of the project, as the index has it.
#[derive(Clone, PartialEq, Message)]
pub struct Document {
    /// Its path, from the project's folder, with `/` between the names.
    #[prost(string, tag = "1")]
    pub relative_path: String,
    #[prost(message, repeated, tag = "2")]
    pub occurrences: Vec<Occurrence>,
    /// What the symbols defined in the file are.
    #[prost(message, repeated, tag = "3")]
    pub symbols: Vec<SymbolInformation>,
    /// Its language, as `rust` or `TypeScript`, if the indexer says.
    #[prost(string, tag = "4")]
    pub language: String,
    /// What the file says, if the indexer wrote it into the index, which
    /// most do not.
    #[prost(string, tag = "5")]
    pub text: String,
    /// What a character of a line is, in the occurrences' ranges: see
    /// [`Encoding`].
    #[prost(int32, tag = "6")]
    pub position_encoding: i32,
}

/// A symbol where the file names it or defines it.
#[derive(Clone, PartialEq, Message)]
pub struct Occurrence {
    /// Where it is, as three numbers, a line and the characters it starts
    /// and ends at, or four, when it ends on another line; unless
    /// `typed_range` says.
    #[prost(int32, repeated, tag = "1")]
    pub range: Vec<i32>,
    #[prost(string, tag = "2")]
    pub symbol: String,
    /// What the occurrence is, in [`role`]s.
    #[prost(int32, tag = "3")]
    pub symbol_roles: i32,
    /// The whole of what the occurrence is in, as `range` is written: for
    /// a definition, all of it.
    #[prost(int32, repeated, tag = "7")]
    pub enclosing_range: Vec<i32>,
    #[prost(oneof = "TypedRange", tags = "8, 9")]
    pub typed_range: Option<TypedRange>,
    #[prost(oneof = "TypedEnclosingRange", tags = "10, 11")]
    pub typed_enclosing_range: Option<TypedEnclosingRange>,
}

/// Where an occurrence is, as newer indexers write it.
#[derive(Clone, PartialEq, prost::Oneof)]
pub enum TypedRange {
    #[prost(message, tag = "8")]
    SingleLine(SingleLineRange),
    #[prost(message, tag = "9")]
    MultiLine(MultiLineRange),
}

/// What an occurrence is in, as newer indexers write it.
#[derive(Clone, PartialEq, prost::Oneof)]
pub enum TypedEnclosingRange {
    #[prost(message, tag = "10")]
    SingleLine(SingleLineRange),
    #[prost(message, tag = "11")]
    MultiLine(MultiLineRange),
}

#[derive(Clone, PartialEq, Message)]
pub struct SingleLineRange {
    #[prost(int32, tag = "1")]
    pub line: i32,
    #[prost(int32, tag = "2")]
    pub start_character: i32,
    #[prost(int32, tag = "3")]
    pub end_character: i32,
}

#[derive(Clone, PartialEq, Message)]
pub struct MultiLineRange {
    #[prost(int32, tag = "1")]
    pub start_line: i32,
    #[prost(int32, tag = "2")]
    pub start_character: i32,
    #[prost(int32, tag = "3")]
    pub end_line: i32,
    #[prost(int32, tag = "4")]
    pub end_character: i32,
}

/// What a symbol is.
#[derive(Clone, PartialEq, Message)]
pub struct SymbolInformation {
    #[prost(string, tag = "1")]
    pub symbol: String,
    /// Its documentation, in Markdown; some indexers put its signature
    /// first, in a block of code.
    #[prost(string, repeated, tag = "3")]
    pub documentation: Vec<String>,
    #[prost(message, repeated, tag = "4")]
    pub relationships: Vec<Relationship>,
    /// What kind of thing it is, as `scip.proto`'s `Kind` numbers it (see
    /// [`kind`]); 0 when the indexer does not say.
    #[prost(int32, tag = "5")]
    pub kind: i32,
    #[prost(message, optional, tag = "7")]
    pub signature_documentation: Option<Signature>,
    /// The symbol it is defined in, as a method in its type, if the
    /// indexer says.
    #[prost(string, tag = "8")]
    pub enclosing_symbol: String,
}

/// A symbol's signature, as its documentation shows it.
#[derive(Clone, PartialEq, Message)]
pub struct Signature {
    #[prost(string, tag = "5")]
    pub text: String,
}

/// How a symbol stands to another.
#[derive(Clone, PartialEq, Message)]
pub struct Relationship {
    #[prost(string, tag = "1")]
    pub symbol: String,
    /// Whether the symbol implements the other, as a method one of an
    /// interface or a trait.
    #[prost(bool, tag = "3")]
    pub is_implementation: bool,
}

/// The roles of an occurrence, as bits of [`Occurrence::symbol_roles`].
pub mod role {
    pub const DEFINITION: i32 = 0x1;
    pub const IMPORT: i32 = 0x2;
    pub const WRITE_ACCESS: i32 = 0x4;
    pub const TEST: i32 = 0x20;
}

/// The kinds of symbol the map tells apart, as [`SymbolInformation::kind`]
/// numbers them.
pub mod kind {
    pub const CLASS: i32 = 7;
    pub const CONSTANT: i32 = 8;
    pub const CONSTRUCTOR: i32 = 9;
    pub const ENUM: i32 = 11;
    pub const ENUM_MEMBER: i32 = 12;
    pub const EVENT: i32 = 13;
    pub const FIELD: i32 = 15;
    pub const FILE: i32 = 16;
    pub const FUNCTION: i32 = 17;
    pub const GETTER: i32 = 18;
    pub const INTERFACE: i32 = 21;
    pub const METHOD: i32 = 26;
    pub const MODULE: i32 = 29;
    pub const NAMESPACE: i32 = 30;
    pub const OBJECT: i32 = 33;
    pub const PACKAGE: i32 = 35;
    pub const PROPERTY: i32 = 41;
    pub const PROTOCOL: i32 = 42;
    pub const SETTER: i32 = 45;
    pub const STRUCT: i32 = 49;
    pub const TRAIT: i32 = 53;
    pub const TYPE: i32 = 54;
    pub const TYPE_ALIAS: i32 = 55;
    pub const TYPE_CLASS: i32 = 56;
    pub const UNION: i32 = 59;
    pub const CONTRACT: i32 = 62;
    pub const ABSTRACT_METHOD: i32 = 66;
    pub const METHOD_SPECIFICATION: i32 = 67;
    pub const PROTOCOL_METHOD: i32 = 68;
    pub const PURE_VIRTUAL_METHOD: i32 = 69;
    pub const TRAIT_METHOD: i32 = 70;
    pub const TYPE_CLASS_METHOD: i32 = 71;
    pub const ACCESSOR: i32 = 72;
    pub const METHOD_ALIAS: i32 = 74;
    pub const SINGLETON_CLASS: i32 = 75;
    pub const SINGLETON_METHOD: i32 = 76;
    pub const STATIC_DATA_MEMBER: i32 = 77;
    pub const STATIC_EVENT: i32 = 78;
    pub const STATIC_FIELD: i32 = 79;
    pub const STATIC_METHOD: i32 = 80;
    pub const STATIC_PROPERTY: i32 = 81;
    pub const MIXIN: i32 = 85;
}

/// Where in a file an occurrence is, or what it is in: from a line and a
/// character to a line and a character, the end left out, each counted
/// from 0.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Span {
    pub start_line: usize,
    pub start_character: usize,
    pub end_line: usize,
    pub end_character: usize,
    /// Whether the index wrote it as a span of one line, which it may do
    /// only when it is one.
    pub one_line: bool,
}

impl Span {
    fn single(line: i32, start: i32, end: i32) -> Option<Self> {
        let line = usize::try_from(line).ok()?;
        Some(Self {
            start_line: line,
            start_character: usize::try_from(start).ok()?,
            end_line: line,
            end_character: usize::try_from(end).ok()?,
            one_line: true,
        })
    }

    fn multi(start_line: i32, start: i32, end_line: i32, end: i32) -> Option<Self> {
        Some(Self {
            start_line: usize::try_from(start_line).ok()?,
            start_character: usize::try_from(start).ok()?,
            end_line: usize::try_from(end_line).ok()?,
            end_character: usize::try_from(end).ok()?,
            one_line: false,
        })
    }

    fn of(numbers: &[i32]) -> Option<Self> {
        match *numbers {
            [line, start, end] => Self::single(line, start, end),
            [start_line, start, end_line, end] => Self::multi(start_line, start, end_line, end),
            _ => None,
        }
    }
}

impl Occurrence {
    /// Where the occurrence is, if the index says in a way that reads.
    #[must_use]
    pub fn span(&self) -> Option<Span> {
        match &self.typed_range {
            Some(TypedRange::SingleLine(r)) => {
                Span::single(r.line, r.start_character, r.end_character)
            }
            Some(TypedRange::MultiLine(r)) => {
                Span::multi(r.start_line, r.start_character, r.end_line, r.end_character)
            }
            None => Span::of(&self.range),
        }
    }

    /// What the occurrence is in: all of a definition, with its
    /// documentation, or the expression a reference is part of; if the
    /// index says.
    #[must_use]
    pub fn enclosing(&self) -> Option<Span> {
        match &self.typed_enclosing_range {
            Some(TypedEnclosingRange::SingleLine(r)) => {
                Span::single(r.line, r.start_character, r.end_character)
            }
            Some(TypedEnclosingRange::MultiLine(r)) => {
                Span::multi(r.start_line, r.start_character, r.end_line, r.end_character)
            }
            None => Span::of(&self.enclosing_range),
        }
    }

    /// Whether the occurrence has `role`, of [`role`].
    #[must_use]
    pub fn is(&self, role: i32) -> bool {
        self.symbol_roles & role != 0
    }
}

/// What a character is in a document's ranges.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Encoding {
    Utf8,
    Utf16,
    Utf32,
}

impl Document {
    /// What a character is in the document's ranges. A document that does
    /// not say is taken to count as LSP does, in UTF-16, as the indexers
    /// built on a language server do.
    #[must_use]
    pub fn encoding(&self) -> Encoding {
        match self.position_encoding {
            1 => Encoding::Utf8,
            3 => Encoding::Utf32,
            _ => Encoding::Utf16,
        }
    }
}

impl Encoding {
    /// The byte of `line` that its `character`th character starts at, as
    /// this encoding counts them, or its end if it is shorter.
    #[must_use]
    pub fn byte(self, line: &str, character: usize) -> usize {
        let mut counted = 0;
        for (at, c) in line.char_indices() {
            if counted >= character {
                return at;
            }
            counted += match self {
                Self::Utf8 => c.len_utf8(),
                Self::Utf16 => c.len_utf16(),
                Self::Utf32 => 1,
            };
        }
        line.len()
    }
}

/// Indexes real indexers wrote, of a small file each, and the file:
/// `scip-go --output=cart-go.scip`, scip-go 0.2.10, in a module
/// `example.com/shop` of `cart.go`; and `rust-analyzer scip .`,
/// rust-analyzer 1.99.1, in a crate `shop` of `src/lib.rs`.
#[cfg(test)]
pub(crate) mod fixtures {
    pub(crate) const GO_INDEX: &[u8] = include_bytes!("scip/fixtures/cart-go.scip");
    pub(crate) const GO_FILE: (&str, &str) = ("cart.go", include_str!("scip/fixtures/cart-go.txt"));
    pub(crate) const RUST_INDEX: &[u8] = include_bytes!("scip/fixtures/cart-rust.scip");
    pub(crate) const RUST_FILE: (&str, &str) =
        ("src/lib.rs", include_str!("scip/fixtures/cart-rust.txt"));
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn an_index_reads_back_as_it_was_written() {
        let index = Index {
            documents: vec![Document {
                relative_path: "a.go".to_owned(),
                occurrences: vec![
                    Occurrence {
                        range: vec![1, 2, 5],
                        symbol: "scip-go gomod a v1 a/A#".to_owned(),
                        symbol_roles: role::DEFINITION,
                        enclosing_range: vec![0, 0, 3, 1],
                        ..Occurrence::default()
                    },
                    Occurrence {
                        typed_range: Some(TypedRange::MultiLine(MultiLineRange {
                            start_line: 4,
                            start_character: 0,
                            end_line: 5,
                            end_character: 2,
                        })),
                        typed_enclosing_range: Some(TypedEnclosingRange::SingleLine(
                            SingleLineRange {
                                line: 4,
                                start_character: 0,
                                end_character: 9,
                            },
                        )),
                        range: vec![9, 9, 9],
                        ..Occurrence::default()
                    },
                ],
                position_encoding: 1,
                ..Document::default()
            }],
        };
        let back = Index::decode(index.encode_to_vec().as_slice()).unwrap();
        assert_eq!(back, index);

        let occurrences = &back.documents[0].occurrences;
        assert_eq!(
            occurrences[0].span(),
            Some(Span {
                start_line: 1,
                start_character: 2,
                end_line: 1,
                end_character: 5,
                one_line: true
            })
        );
        assert_eq!(occurrences[0].enclosing().unwrap().end_line, 3);
        assert!(occurrences[0].is(role::DEFINITION));
        // The typed range wins over the old one.
        assert_eq!(occurrences[1].span().unwrap().start_line, 4);
        assert!(!occurrences[1].span().unwrap().one_line);
        assert!(occurrences[1].enclosing().unwrap().one_line);
        assert_eq!(back.documents[0].encoding(), Encoding::Utf8);
        assert!(Index::decode(&b"\xff\xff"[..]).is_err());
    }

    /// The occurrence of `symbol` in `document` that defines it.
    fn definition<'d>(document: &'d Document, symbol: &str) -> &'d Occurrence {
        document
            .occurrences
            .iter()
            .find(|occurrence| occurrence.symbol == symbol && occurrence.is(role::DEFINITION))
            .unwrap()
    }

    fn information<'d>(document: &'d Document, symbol: &str) -> &'d SymbolInformation {
        document
            .symbols
            .iter()
            .find(|information| information.symbol == symbol)
            .unwrap()
    }

    #[test]
    fn an_index_scip_go_wrote_reads_with_its_typed_ranges() {
        let index = Index::decode(fixtures::GO_INDEX).unwrap();
        assert_eq!(index.documents.len(), 1);
        let document = &index.documents[0];
        assert_eq!(document.relative_path, fixtures::GO_FILE.0);
        assert_eq!(document.language, "go");
        assert_eq!(document.encoding(), Encoding::Utf8);
        let cart = "scip-go gomod example.com/shop . `example.com/shop`/Cart#";
        let defined = definition(document, cart);
        assert_eq!(
            (
                defined.span().unwrap().start_line,
                defined.span().unwrap().start_character
            ),
            (3, 5)
        );
        // scip-go writes the newer typed ranges: a type spans lines, a
        // field one.
        let enclosing = defined.enclosing().unwrap();
        assert_eq!(
            (enclosing.start_line, enclosing.end_line, enclosing.one_line),
            (2, 6, false)
        );
        let items = definition(document, &format!("{cart}items."));
        assert!(items.enclosing().unwrap().one_line);
        assert_eq!(information(document, cart).kind, kind::STRUCT);
        let add = information(document, &format!("{cart}Add()."));
        assert_eq!(add.kind, kind::METHOD);
        assert_eq!(
            add.signature_documentation.as_ref().unwrap().text,
            "func (*Cart).Add(item string)"
        );
        let writes = document
            .occurrences
            .iter()
            .filter(|occurrence| {
                occurrence.symbol == format!("{cart}count.") && !occurrence.is(role::DEFINITION)
            })
            .count();
        assert_eq!(writes, 1);
    }

    #[test]
    fn an_index_rust_analyzer_wrote_reads_with_its_ranges_of_numbers() {
        let index = Index::decode(fixtures::RUST_INDEX).unwrap();
        let document = &index.documents[0];
        assert_eq!(document.relative_path, fixtures::RUST_FILE.0);
        let cart = "rust-analyzer cargo shop 0.1.0 Cart#";
        let defined = definition(document, cart);
        assert_eq!(defined.span().unwrap().start_line, 1);
        assert!(defined.span().unwrap().one_line);
        assert_eq!(defined.enclosing().unwrap().end_line, 4);
        assert_eq!(information(document, cart).kind, kind::STRUCT);
        let add = information(document, "rust-analyzer cargo shop 0.1.0 impl#[Cart]add().");
        assert_eq!(
            add.signature_documentation.as_ref().unwrap().text,
            "pub fn add(&mut self, item: String)"
        );
        assert_eq!(add.documentation, ["Puts an item in the cart."]);
    }

    #[test]
    fn a_range_of_the_wrong_length_does_not_read() {
        let occurrence = Occurrence {
            range: vec![1, 2],
            ..Occurrence::default()
        };
        assert_eq!(occurrence.span(), None);
        assert_eq!(occurrence.enclosing(), None);
    }

    #[test]
    fn a_character_is_counted_as_the_document_counts_it() {
        let line = "let 地図 = x;";
        assert_eq!(Encoding::Utf8.byte(line, 11), 11);
        assert_eq!(Encoding::Utf16.byte(line, 7), 11);
        assert_eq!(Encoding::Utf32.byte(line, 7), 11);
        assert_eq!(Encoding::Utf32.byte(line, 99), line.len());
    }
}
