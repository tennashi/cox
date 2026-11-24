//! What the map reads of the code by convention, where the index does
//! not say: what a module's id is; whether a reference writes what it
//! names, or imports it; which code is tests'; which terms are places
//! code runs in; and which type a method of rust-analyzer's `impl` blocks
//! is of. Everything else the map reads as SCIP defines it, the same for
//! every language; what depends on the language, or on how an indexer
//! writes it, is here alone.
//!
//! The indexers the map was tried with set none of the roles SCIP has
//! for these (`WriteAccess`, `Import`, `Test`), so these stand in for
//! them; where an indexer sets a role, the map reads that instead.

use crate::scip::kind;
use crate::scip::symbol::{Descriptor, Suffix};

/// A module's id: its package's name, then its namespaces, by `/`; but
/// not the package's name when the namespaces start with it already, as
/// scip-go's and scip-typescript's do (`example.com/m` and then
/// `example.com/m/pkg`). A package of `.`, SCIP's placeholder for none,
/// is left out.
pub(super) fn module_id(package: &str, namespaces: &[&str]) -> String {
    let path = namespaces.join("/");
    let starts_with_package = path == package
        || (path.starts_with(package)
            && matches!(path.as_bytes().get(package.len()), Some(b'/' | b'.' | b':')));
    if package.is_empty() || package == "." || starts_with_package {
        path
    } else if path.is_empty() {
        package.to_owned()
    } else {
        format!("{package}/{path}")
    }
}

/// Whether a symbol of the indexer `scheme` that starts with
/// `namespaces` names the Cargo target it is of first, as rust-analyzer's
/// releases since November 2026 do: `lib`, `proc-macro` or
/// `custom-build`, or `bin`, `test`, `example` or `bench` and the
/// target's name. Older releases name none, so a package's symbols name
/// their targets only when all of them that start with a namespace do:
/// those that start with none, as the functions of an `extern` block, a
/// newer release names no target of either (see [`modules`]).
pub(super) fn names_target(scheme: &str, namespaces: &[&str]) -> bool {
    scheme == "rust-analyzer"
        && matches!(
            namespaces,
            ["lib" | "proc-macro" | "custom-build", ..]
                | ["bin" | "test" | "example" | "bench", _, ..]
        )
}

/// The namespaces of a symbol that are its code's modules: all of them,
/// but for the library target its package's symbols name first, when
/// they name their targets (see [`names_target`]), which the library's
/// modules were not told by before. Another target's name stays, as
/// `bin/xtask`, so that the modules of two binaries stay apart.
pub(super) fn modules<'a, 'b>(names_targets: bool, namespaces: &'b [&'a str]) -> &'b [&'a str] {
    match namespaces {
        ["lib" | "proc-macro", modules @ ..] if names_targets => modules,
        _ => namespaces,
    }
}

/// Whether a reference on `line`, in no function, is an import, where
/// the indexer does not mark imports: it is above the file's first
/// definition, as a file's `use` and `import` lines are.
pub(super) fn imported(line: usize, first_definition: Option<usize>) -> bool {
    first_definition.is_some_and(|first| line < first)
}

/// Whether a term of `kind` whose definition spans the lines `start` to
/// `end` is a place code runs in, as TypeScript's `const f = () => {...}`,
/// rather than a field or a constant.
pub(super) fn runs_code(kind: i32, start: usize, end: usize) -> bool {
    end > start && !matches!(kind, kind::FIELD | kind::ENUM_MEMBER)
}

/// A language whose writes the map can tell from the text after a
/// reference.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum Language {
    Rust,
    Python,
    TypeScript,
    Go,
}

impl Language {
    /// The language of a document, by the name the index gives it, or
    /// else by the scheme of the indexer that wrote its symbols.
    pub(super) fn of(language: &str, scheme: Option<&str>) -> Option<Self> {
        if language.is_empty() {
            return match scheme? {
                "rust-analyzer" => Some(Self::Rust),
                "scip-typescript" => Some(Self::TypeScript),
                "scip-python" => Some(Self::Python),
                "scip-go" => Some(Self::Go),
                _ => None,
            };
        }
        match language.to_lowercase().as_str() {
            "rust" => Some(Self::Rust),
            "python" => Some(Self::Python),
            "typescript" | "typescriptreact" | "javascript" | "javascriptreact" => {
                Some(Self::TypeScript)
            }
            "go" => Some(Self::Go),
            _ => None,
        }
    }

    /// Whether `after`, the text of a line after a reference to a field
    /// or a variable, writes it: assigns it, or calls a method on it that
    /// changes it, as `.push(`. A guess from the text, which can be wrong.
    pub(super) fn writes(self, after: &str) -> bool {
        let rest = after.trim_start();
        let mut chars = rest.chars();
        let (first, second) = (chars.next(), chars.next());
        // An assignment, but not a comparison (`==`), nor TypeScript's
        // arrow (`=>`).
        if first == Some('=') {
            return match second {
                Some('=') | None => false,
                Some('>') => self != Self::TypeScript,
                Some(_) => true,
            };
        }
        if let Some(method) = rest.strip_prefix('.') {
            return self.changes(method);
        }
        match self {
            Self::Rust => rest.starts_with("+=") || rest.starts_with("-="),
            Self::Python => compound(rest, "-+*/|&%", false),
            Self::TypeScript => {
                rest.starts_with("++") || rest.starts_with("--") || compound(rest, "-+*/|&?%", true)
            }
            Self::Go => {
                rest.starts_with("++") || rest.starts_with("--") || compound(rest, "-+*/|&%", false)
            }
        }
    }

    /// Whether `method`, the text after a `.`, calls a method that
    /// changes what it is called on.
    fn changes(self, method: &str) -> bool {
        let name_length = method
            .char_indices()
            .find(|&(_, c)| !(c.is_alphanumeric() || c == '_'))
            .map_or(method.len(), |(at, _)| at);
        let (name, after) = method.split_at(name_length);
        match self {
            Self::Rust => {
                [
                    "push", "insert", "remove", "take", "clear", "retain", "extend", "replace",
                    "drain", "entry", "pop", "truncate",
                ]
                .contains(&name)
                    || name.starts_with("get_or_insert")
                    || name.starts_with("sort")
            }
            Self::Python => {
                after.starts_with('(')
                    && [
                        "append",
                        "extend",
                        "insert",
                        "pop",
                        "remove",
                        "clear",
                        "update",
                        "setdefault",
                        "add",
                        "discard",
                        "sort",
                    ]
                    .contains(&name)
            }
            Self::TypeScript => {
                after.starts_with('(')
                    && [
                        "push", "pop", "shift", "unshift", "splice", "set", "delete", "clear",
                        "add", "sort",
                    ]
                    .contains(&name)
            }
            Self::Go => false,
        }
    }
}

/// Whether `text` starts with an operator of `operators`, or several
/// when `many`, then `=`, as `+=`.
fn compound(text: &str, operators: &str, many: bool) -> bool {
    let taken = text.chars().take_while(|&c| operators.contains(c)).count();
    (taken == 1 || (many && taken > 1)) && text[taken..].starts_with('=')
}

/// Whether a folder of this name holds tests, or what they read.
fn tests_folder(name: &str) -> bool {
    matches!(
        name,
        "test"
            | "tests"
            | "__tests__"
            | "spec"
            | "specs"
            | "testing"
            | "testdata"
            | "fixture"
            | "fixtures"
    )
}

/// Whether the file at `path`, from the project's folder, is tests', by
/// the folders it is in or its name, as the common languages name them.
pub(super) fn test_path(path: &str) -> bool {
    let (folders, name) = path.rsplit_once('/').unwrap_or(("", path));
    folders.split('/').any(tests_folder) || test_file(name)
}

#[expect(
    clippy::case_sensitive_file_extension_comparisons,
    reason = "the test runners match these names as they are written"
)]
fn test_file(name: &str) -> bool {
    if name == "conftest.py"
        || (name.starts_with("test_") && name.ends_with(".py"))
        || name.ends_with("_test.go")
        || name.ends_with("_test.py")
    {
        return true;
    }
    // `a.test.ts`, `a.spec.mjs`, `a.test.tsx`
    let Some((stem, extension)) = name.rsplit_once('.') else {
        return false;
    };
    let extension = extension.strip_prefix(['c', 'm']).unwrap_or(extension);
    let extension = extension.strip_suffix('x').unwrap_or(extension);
    matches!(extension, "js" | "ts") && (stem.ends_with(".test") || stem.ends_with(".spec"))
}

/// Whether a namespace of this name is tests', as Rust's `mod tests` or a
/// module of Python's `test_` files.
pub(super) fn test_namespace(name: &str) -> bool {
    tests_folder(name)
        || name
            .split(['/', '.', ':'])
            .filter(|part| !part.is_empty())
            .any(|part| tests_folder(part) || part.starts_with("test_"))
}

/// The type a method is of, by name, to show it: the type of its
/// rust-analyzer `impl` block (see [`impl_block`]), or else the last type
/// its symbol is in. `before` is its descriptors but its own.
pub(super) fn type_named<'d>(before: &'d [Descriptor<'d>]) -> Option<&'d str> {
    impl_block(before)
        .map(|(_, type_name, _)| type_name)
        .or_else(|| {
            before
                .iter()
                .rev()
                .find(|descriptor| descriptor.suffix == Suffix::Type)
                .map(|descriptor| descriptor.name.as_ref())
        })
}

/// The type a method is of, by name, and the trait it implements, if
/// any, when its symbol is of rust-analyzer's `impl` blocks: the
/// descriptors before the method end in a type that is no type of the
/// code (`impl#`) and type parameters after it, the first the type the
/// block is for, the second the trait, as `impl#[Chat][Drop]drop().`.
/// The method's own descriptor is left out of `descriptors`. Also the
/// type descriptor's place among them, so that the caller can ask
/// whether it is a type of the code after all.
pub(super) fn impl_block<'d>(
    descriptors: &'d [Descriptor<'d>],
) -> Option<(usize, &'d str, Option<&'d str>)> {
    let parameters = descriptors
        .iter()
        .rev()
        .take_while(|descriptor| descriptor.suffix == Suffix::TypeParameter)
        .count();
    let block = descriptors.len().checked_sub(parameters + 1)?;
    if parameters == 0 || descriptors[block].suffix != Suffix::Type {
        return None;
    }
    let parameters = &descriptors[block + 1..];
    Some((
        block,
        parameters[0].name.as_ref(),
        parameters.get(1).map(|trait_| trait_.name.as_ref()),
    ))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_write_is_told_from_the_text_after_the_reference() {
        let rust = Language::Rust;
        assert!(rust.writes(" = 3;"));
        assert!(rust.writes(" += 1;"));
        assert!(rust.writes(".push(x);"));
        assert!(rust.writes(".sort_by_key(|x| x);"));
        assert!(rust.writes(".get_or_insert_with(f)"));
        assert!(!rust.writes(" == 3"));
        assert!(!rust.writes(".len()"));
        assert!(!rust.writes(".pushed"));
        assert!(!rust.writes(" ="));
        assert!(!rust.writes(""));

        let typescript = Language::TypeScript;
        assert!(typescript.writes(" ??= 1"));
        assert!(typescript.writes("++"));
        assert!(typescript.writes(".set(k, v)"));
        assert!(!typescript.writes(" => 1"));
        assert!(!typescript.writes(".set"));
        assert!(!typescript.writes(" === 1"));

        assert!(Language::Python.writes(".append(x)"));
        assert!(Language::Python.writes(" |= x"));
        assert!(!Language::Python.writes(" ||= x"));
        assert!(Language::Go.writes("--"));
        assert!(!Language::Go.writes(".Push(x)"));
    }

    #[test]
    fn a_language_is_known_by_its_name_or_its_indexer() {
        assert_eq!(
            Language::of("TypeScriptReact", None),
            Some(Language::TypeScript)
        );
        assert_eq!(Language::of("", Some("scip-go")), Some(Language::Go));
        assert_eq!(Language::of("", Some("scip-java")), None);
        assert_eq!(Language::of("Haskell", Some("rust-analyzer")), None);
    }

    #[test]
    fn the_library_target_rust_analyzer_names_is_no_module() {
        let rust = |namespaces: &[&str]| names_target("rust-analyzer", namespaces);
        assert!(rust(&["lib", "daemon"]));
        assert!(rust(&["proc-macro"]));
        assert!(rust(&["bin", "xtask", "ci"]));
        assert!(!rust(&["bin"]), "a module `bin` of an older release");
        assert!(!rust(&["daemon", "chat"]));
        assert!(!rust(&[]));
        assert!(!names_target("scip-go", &["lib", "x"]));

        let modules = |named, namespaces: &[&str]| modules(named, namespaces).join("/");
        assert_eq!(modules(true, &["lib", "daemon", "chat"]), "daemon/chat");
        assert_eq!(modules(true, &["lib"]), "");
        assert_eq!(modules(true, &["proc-macro", "derive"]), "derive");
        assert_eq!(modules(true, &["bin", "a", "foo"]), "bin/a/foo");
        assert_eq!(
            modules(true, &["test", "window", "fake"]),
            "test/window/fake"
        );
        // An older release's module `lib`.
        assert_eq!(modules(false, &["lib", "x"]), "lib/x");
    }

    #[test]
    fn tests_are_known_by_their_folders_and_names() {
        assert!(test_path("crates/cox/tests/it.rs"));
        assert!(test_path("src/__tests__/a.ts"));
        assert!(test_path("tests/test_client.py"));
        assert!(test_path("pkg/a_test.go"));
        assert!(test_path("src/router.test.ts"));
        assert!(test_path("src/router.spec.mjs"));
        assert!(test_path("src/view.test.tsx"));
        assert!(!test_path("src/testing.rs.bak/x"));
        assert!(!test_path("src/tests.rs"));
        assert!(!test_path("src/router.ts"));
        assert!(!test_path("src/latest.ts"));
        assert!(test_namespace("tests"));
        assert!(test_namespace("httpx.tests.test_api"));
        assert!(test_namespace("pkg.test_client"));
        assert!(!test_namespace("daemon"));
        assert!(!test_namespace("contest"));
    }
}
