//! Tells coxd what a build of it is, as `wire::VERSION` reads it (see
//! `src/daemon/wire.rs`): what its window and it say to each other, as a
//! hash of the types that go between them, so that a window of another
//! build that says it the same way joins the coxd there; and what coxd
//! does, as a hash of the code of coxd and of the crates it runs, so that
//! the window tells the human when coxd runs code other than its own.
//! Neither comments nor how the code is laid out count, as they change
//! nothing built.

use std::collections::{BTreeMap, BTreeSet};
use std::fs;
use std::path::{Path, PathBuf};

use proc_macro2::{Delimiter, TokenStream, TokenTree};
use quote::ToTokens as _;
use syn::{Attribute, Item};

/// The sources that define what a window and coxd say: the types of
/// coxd, and of the crates of Cox's own whose types it carries.
const PROTOCOL: &[&str] = &["src", "../cox-core/src", "../cox-acp/src"];

/// What a window and coxd say to each other, from which the types they
/// carry are found, and the `REVISION` that changes what they say when
/// no type does.
const SAID: &[&str] = &["Hello", "ToDaemon", "FromDaemon", "REVISION"];

/// The sources of what coxd does: its code and what it serves, the crates
/// of Cox's own it runs, and the versions of all the others.
const DAEMON: &[&str] = &[
    "src",
    "assets",
    "Cargo.toml",
    "../cox-core/src",
    "../cox-core/Cargo.toml",
    "../cox-acp/src",
    "../cox-acp/Cargo.toml",
    "../cox-editor/src",
    "../cox-editor/Cargo.toml",
    "../cox-git/src",
    "../cox-git/Cargo.toml",
    "../cox-mac/src",
    "../cox-mac/Cargo.toml",
    "../../Cargo.lock",
];

fn main() {
    println!("cargo::rerun-if-changed=build.rs");
    println!("cargo::rustc-env=COX_PROTOCOL={:016x}", protocol().0);
    println!("cargo::rustc-env=COX_DAEMON={:016x}", daemon().0);
}

/// The hash of the types that go between a window and coxd: those
/// [`SAID`] names, and each type their fields are of, by its name, in
/// turn. Two types of one name are both taken.
fn protocol() -> Hash {
    let mut defined: BTreeMap<String, Vec<Definition>> = BTreeMap::new();
    for file in files(PROTOCOL) {
        if file.extension().is_some_and(|extension| extension == "rs") {
            let text = fs::read_to_string(&file).expect("a source of coxd is readable");
            let parsed = syn::parse_file(&text).expect("a source of coxd parses");
            types(&parsed.items, &mut defined);
        }
    }
    let mut taken = BTreeSet::new();
    let mut named: Vec<String> = SAID.iter().map(|&name| name.to_owned()).collect();
    while let Some(name) = named.pop() {
        let Some(definitions) = defined.get(&name).filter(|_| !taken.contains(&name)) else {
            continue;
        };
        for definition in definitions {
            named.extend(
                definition
                    .names
                    .split(|letter: char| !(letter.is_alphanumeric() || letter == '_'))
                    .filter(|word| word.starts_with(char::is_uppercase) && !taken.contains(*word))
                    .map(str::to_owned),
            );
        }
        taken.insert(name);
    }
    let mut hash = Hash::new();
    for name in taken {
        for definition in &defined[&name] {
            hash.feed(definition.said.as_bytes());
        }
    }
    hash
}

/// What a type or constant is: its tokens without doc comments, and the
/// types its fields are of.
struct Definition {
    said: String,
    names: String,
}

/// Adds to `defined` each type and constant of `items`, and of the
/// modules among them, but for those only tests build, by its name.
fn types(items: &[Item], defined: &mut BTreeMap<String, Vec<Definition>>) {
    for item in items {
        if tested(item) {
            continue;
        }
        let fields = |fields: &syn::Fields| -> Vec<syn::Type> {
            fields.iter().map(|field| field.ty.clone()).collect()
        };
        let (name, of) = match item {
            Item::Mod(module) => {
                if let Some((_, inner)) = &module.content {
                    types(inner, defined);
                }
                continue;
            }
            Item::Struct(item) => (&item.ident, fields(&item.fields)),
            Item::Enum(item) => (
                &item.ident,
                item.variants
                    .iter()
                    .flat_map(|variant| fields(&variant.fields))
                    .collect(),
            ),
            Item::Union(item) => (
                &item.ident,
                fields(&syn::Fields::Named(item.fields.clone())),
            ),
            Item::Type(item) => (&item.ident, vec![(*item.ty).clone()]),
            Item::Const(item) => (&item.ident, vec![(*item.ty).clone()]),
            _ => continue,
        };
        let mut said = String::new();
        undocumented(item.to_token_stream(), &mut said);
        let mut names = String::new();
        for ty in of {
            undocumented(ty.to_token_stream(), &mut names);
        }
        defined
            .entry(name.to_string())
            .or_default()
            .push(Definition { said, names });
    }
}

/// The hash of the code of [`DAEMON`], each file by its path, in the order
/// of their paths: of a Rust file, its items but those only tests build,
/// without doc comments.
fn daemon() -> Hash {
    let mut hash = Hash::new();
    for file in files(DAEMON) {
        hash.feed(file.to_string_lossy().as_bytes());
        let bytes = fs::read(&file).expect("a source of coxd is readable");
        let parsed = file
            .extension()
            .is_some_and(|extension| extension == "rs")
            .then(|| syn::parse_file(std::str::from_utf8(&bytes).ok()?).ok())
            .flatten();
        let Some(parsed) = parsed else {
            hash.feed(&bytes);
            continue;
        };
        for item in parsed.items.iter().filter(|item| !tested(item)) {
            let mut code = String::new();
            undocumented(item.to_token_stream(), &mut code);
            hash.feed(code.as_bytes());
        }
    }
    hash
}

/// Writes `tokens` to `written`, a space between each, but for the doc
/// comments among them.
fn undocumented(tokens: TokenStream, written: &mut String) {
    let mut tokens = tokens.into_iter().peekable();
    while let Some(token) = tokens.next() {
        match token {
            TokenTree::Punct(punct) if punct.as_char() == '#' && tokens.peek().is_some_and(doc) => {
                tokens.next();
            }
            TokenTree::Group(group) => {
                let (open, close) = match group.delimiter() {
                    Delimiter::Parenthesis => ("(", ")"),
                    Delimiter::Brace => ("{", "}"),
                    Delimiter::Bracket => ("[", "]"),
                    Delimiter::None => ("", ""),
                };
                written.push_str(open);
                undocumented(group.stream(), written);
                written.push_str(close);
                written.push(' ');
            }
            other => {
                written.push_str(&other.to_string());
                written.push(' ');
            }
        }
    }
}

/// Whether `token`, after a `#`, is what a doc comment is made into:
/// `[doc = "..."]`.
fn doc(token: &TokenTree) -> bool {
    let TokenTree::Group(group) = token else {
        return false;
    };
    group.delimiter() == Delimiter::Bracket
        && matches!(
            group.stream().into_iter().next(),
            Some(TokenTree::Ident(ident)) if ident == "doc"
        )
}

/// Whether `item` is built only for tests, as `#[cfg(test)]` says.
fn tested(item: &Item) -> bool {
    let attributes: &[Attribute] = match item {
        Item::Mod(item) => &item.attrs,
        Item::Fn(item) => &item.attrs,
        Item::Impl(item) => &item.attrs,
        Item::Struct(item) => &item.attrs,
        Item::Enum(item) => &item.attrs,
        Item::Use(item) => &item.attrs,
        Item::Const(item) => &item.attrs,
        _ => &[],
    };
    attributes.iter().any(|attribute| {
        attribute.path().is_ident("cfg")
            && attribute
                .parse_args::<syn::Ident>()
                .is_ok_and(|ident| ident == "test")
    })
}

/// The files of `sources`, or under each that is a folder, in the order
/// of their paths, each watched for changes.
fn files(sources: &[&str]) -> Vec<PathBuf> {
    let mut files = Vec::new();
    for source in sources {
        println!("cargo::rerun-if-changed={source}");
        files_of(Path::new(source), &mut files);
    }
    files.sort();
    files
}

/// Adds `path` to `files`, or the files under it, if it is a folder.
fn files_of(path: &Path, files: &mut Vec<PathBuf>) {
    if path.is_dir() {
        for entry in fs::read_dir(path).expect("a source folder of coxd is readable") {
            files_of(
                &entry.expect("a source folder of coxd is readable").path(),
                files,
            );
        }
    } else {
        files.push(path.to_owned());
    }
}

/// The FNV-1a hash of what is fed to it.
struct Hash(u64);

impl Hash {
    fn new() -> Self {
        Self(0xcbf2_9ce4_8422_2325)
    }

    fn feed(&mut self, bytes: &[u8]) {
        for &byte in bytes.iter().chain(&[0]) {
            self.0 ^= u64::from(byte);
            self.0 = self.0.wrapping_mul(0x0100_0000_01b3);
        }
    }
}
