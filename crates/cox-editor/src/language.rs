//! The languages the editor knows the syntax of, through their tree-sitter
//! grammars: which a file is in, by its name, and what in each is drawn
//! how, through the grammar's queries.

use std::path::Path;
use std::sync::LazyLock;

use tree_sitter::Query;

use crate::lsp::Server;

/// What a stretch of code is, which says how it is drawn.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Highlight {
    Attribute,
    Comment,
    Constant,
    /// Such as `true` and `None`.
    ConstantBuiltin,
    Constructor,
    Escape,
    Function,
    Keyword,
    /// Such as a lifetime, or a YAML anchor.
    Label,
    Number,
    Operator,
    Property,
    Punctuation,
    /// Such as a Markdown heading's `#`, or a list's `-`.
    PunctuationSpecial,
    String,
    /// Such as a regular expression, or a JSON key.
    StringSpecial,
    Tag,
    Type,
    TypeBuiltin,
    Variable,
    /// Such as `self` and `this`.
    VariableBuiltin,
    Heading,
    /// Code within Markdown.
    Literal,
    Emphasis,
    Strong,
    Link,
    /// A link's address.
    Uri,
}

/// The names queries capture, with what each is: a name with more after a
/// dot, such as `function.method`, is the longest of these it starts with.
const NAMES: &[(&str, Highlight)] = &[
    ("attribute", Highlight::Attribute),
    ("boolean", Highlight::ConstantBuiltin),
    ("comment", Highlight::Comment),
    ("constant", Highlight::Constant),
    ("constant.builtin", Highlight::ConstantBuiltin),
    ("constructor", Highlight::Constructor),
    ("escape", Highlight::Escape),
    ("function", Highlight::Function),
    ("keyword", Highlight::Keyword),
    ("label", Highlight::Label),
    ("number", Highlight::Number),
    ("operator", Highlight::Operator),
    ("property", Highlight::Property),
    ("punctuation", Highlight::Punctuation),
    ("punctuation.special", Highlight::PunctuationSpecial),
    ("string", Highlight::String),
    ("string.escape", Highlight::Escape),
    ("string.special", Highlight::StringSpecial),
    ("tag", Highlight::Tag),
    ("type", Highlight::Type),
    ("type.builtin", Highlight::TypeBuiltin),
    ("variable", Highlight::Variable),
    ("variable.builtin", Highlight::VariableBuiltin),
    ("text.title", Highlight::Heading),
    ("text.literal", Highlight::Literal),
    ("text.emphasis", Highlight::Emphasis),
    ("text.strong", Highlight::Strong),
    ("text.reference", Highlight::Link),
    ("text.uri", Highlight::Uri),
];

/// What a query's capture does to the code it captures.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Capture {
    /// Draws it so.
    Highlight(Highlight),
    /// Draws it plainly, whatever captured it before.
    Plain,
    /// Nothing: the capture is for something else, or a name the editor
    /// does not draw.
    Ignored,
}

impl Capture {
    fn named(name: &str) -> Self {
        if name == "none" {
            return Self::Plain;
        }
        NAMES
            .iter()
            .filter(|(known, _)| {
                name.strip_prefix(known)
                    .is_some_and(|rest| rest.is_empty() || rest.starts_with('.'))
            })
            .max_by_key(|(known, _)| known.len())
            .map_or(Self::Ignored, |&(_, highlight)| Self::Highlight(highlight))
    }
}

/// A language, with its grammar and queries.
pub struct Language {
    name: &'static str,
    pub(crate) grammar: tree_sitter::Language,
    pub(crate) highlights: Query,
    /// What each capture of `highlights` does, by its index.
    pub(crate) captures: Vec<Capture>,
    /// Where code in another language is within code in this one, as a
    /// Rust macro's arguments are Rust, if anywhere.
    pub(crate) injections: Option<Query>,
}

impl std::fmt::Debug for Language {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_tuple("Language").field(&self.name).finish()
    }
}

impl PartialEq for Language {
    fn eq(&self, other: &Self) -> bool {
        self.name == other.name
    }
}

impl Language {
    /// `grammar` with the queries `highlights`, in order, as later
    /// patterns win, and `injections`.
    ///
    /// # Panics
    ///
    /// If a query does not fit the grammar, which the tests rule out.
    fn new(
        name: &'static str,
        grammar: impl Into<tree_sitter::Language>,
        highlights: &[&str],
        injections: Option<&str>,
    ) -> Self {
        let grammar = grammar.into();
        let highlights = Query::new(&grammar, &highlights.concat())
            .unwrap_or_else(|error| panic!("the {name} highlights: {error}"));
        let captures = highlights
            .capture_names()
            .iter()
            .map(|name| Capture::named(name))
            .collect();
        let injections = injections.map(|injections| {
            Query::new(&grammar, injections)
                .unwrap_or_else(|error| panic!("the {name} injections: {error}"))
        });
        Self {
            name,
            grammar,
            highlights,
            captures,
            injections,
        }
    }

    /// The language's name, as code blocks in Markdown name it.
    #[must_use]
    pub fn name(&self) -> &'static str {
        self.name
    }

    /// The language's name, as the Language Server Protocol names it.
    #[must_use]
    pub fn protocol_name(&self) -> &'static str {
        match self.name {
            "bash" => "shellscript",
            "tsx" => "typescriptreact",
            "markdown_inline" => "markdown",
            name => name,
        }
    }

    /// The server that knows the language's code, if the editor knows one.
    #[must_use]
    pub fn server(&self) -> Option<&'static Server> {
        match self.name {
            "rust" => Some(&RUST_ANALYZER),
            _ => None,
        }
    }

    /// The language of the file at `path`, by its name, if the editor
    /// knows it.
    #[must_use]
    pub fn of_path(path: &Path) -> Option<&'static Self> {
        let name = path.file_name()?.to_str()?;
        let language: &LazyLock<Self> = match name {
            "Cargo.lock" => &TOML,
            ".bashrc" | ".bash_profile" | ".profile" | ".zshrc" => &BASH,
            _ => match name.rsplit_once('.')?.1 {
                "rs" => &RUST,
                "toml" => &TOML,
                "md" | "markdown" => &MARKDOWN,
                "json" | "jsonc" => &JSON,
                "yaml" | "yml" => &YAML,
                "sh" | "bash" | "zsh" => &BASH,
                "js" | "mjs" | "cjs" | "jsx" => &JAVASCRIPT,
                "ts" | "mts" | "cts" => &TYPESCRIPT,
                "tsx" => &TSX,
                "py" | "pyi" => &PYTHON,
                _ => return None,
            },
        };
        Some(language)
    }

    /// The language a Markdown code block or an injection names, if the
    /// editor knows it.
    #[must_use]
    pub fn named(name: &str) -> Option<&'static Self> {
        let language: &LazyLock<Self> = match name.to_ascii_lowercase().as_str() {
            "rust" | "rs" => &RUST,
            "toml" => &TOML,
            "markdown" | "md" => &MARKDOWN,
            "markdown_inline" => &MARKDOWN_INLINE,
            "json" | "jsonc" => &JSON,
            "yaml" | "yml" => &YAML,
            "bash" | "sh" | "shell" | "zsh" => &BASH,
            "javascript" | "js" | "jsx" => &JAVASCRIPT,
            "typescript" | "ts" => &TYPESCRIPT,
            "tsx" => &TSX,
            "python" | "py" => &PYTHON,
            _ => return None,
        };
        Some(language)
    }
}

/// Rust's server, which builds what it needs of the project, such as its
/// procedural macros, in a directory of its own, `target/rust-analyzer`,
/// so that it does not hold up the builds of the project while it does.
/// It does not check the project with `cargo check`, which only finds
/// errors, which the editor does not show yet.
static RUST_ANALYZER: Server = Server {
    program: "rust-analyzer",
    args: &[],
    markers: &["Cargo.toml"],
    settings: r#"{ "cargo": { "targetDir": true }, "checkOnSave": false }"#,
};

static RUST: LazyLock<Language> = LazyLock::new(|| {
    Language::new(
        "rust",
        tree_sitter_rust::LANGUAGE,
        &[tree_sitter_rust::HIGHLIGHTS_QUERY],
        Some(tree_sitter_rust::INJECTIONS_QUERY),
    )
});

static TOML: LazyLock<Language> = LazyLock::new(|| {
    // The grammar's highlights capture a key's whole pair, value and all,
    // as a property, where they mean its key.
    let highlights = tree_sitter_toml_ng::HIGHLIGHTS_QUERY.replace(
        "(pair\n  (bare_key)) @property",
        "(pair\n  (bare_key) @property)",
    );
    Language::new("toml", tree_sitter_toml_ng::LANGUAGE, &[&highlights], None)
});

/// Markdown's blocks: what is within them, such as a paragraph's text, is
/// [`MARKDOWN_INLINE`], injected.
static MARKDOWN: LazyLock<Language> = LazyLock::new(|| {
    Language::new(
        "markdown",
        tree_sitter_md::LANGUAGE,
        &[tree_sitter_md::HIGHLIGHT_QUERY_BLOCK],
        Some(tree_sitter_md::INJECTION_QUERY_BLOCK),
    )
});

static MARKDOWN_INLINE: LazyLock<Language> = LazyLock::new(|| {
    Language::new(
        "markdown_inline",
        tree_sitter_md::INLINE_LANGUAGE,
        &[tree_sitter_md::HIGHLIGHT_QUERY_INLINE],
        None,
    )
});

static JSON: LazyLock<Language> = LazyLock::new(|| {
    Language::new(
        "json",
        tree_sitter_json::LANGUAGE,
        &[tree_sitter_json::HIGHLIGHTS_QUERY],
        None,
    )
});

static YAML: LazyLock<Language> = LazyLock::new(|| {
    Language::new(
        "yaml",
        tree_sitter_yaml::LANGUAGE,
        &[tree_sitter_yaml::HIGHLIGHTS_QUERY],
        None,
    )
});

static BASH: LazyLock<Language> = LazyLock::new(|| {
    Language::new(
        "bash",
        tree_sitter_bash::LANGUAGE,
        &[tree_sitter_bash::HIGHLIGHT_QUERY],
        None,
    )
});

/// JavaScript, with JSX.
static JAVASCRIPT: LazyLock<Language> = LazyLock::new(|| {
    Language::new(
        "javascript",
        tree_sitter_javascript::LANGUAGE,
        &[
            tree_sitter_javascript::HIGHLIGHT_QUERY,
            tree_sitter_javascript::JSX_HIGHLIGHT_QUERY,
        ],
        None,
    )
});

/// TypeScript's grammar takes JavaScript's highlights, and adds its own.
static TYPESCRIPT: LazyLock<Language> = LazyLock::new(|| {
    Language::new(
        "typescript",
        tree_sitter_typescript::LANGUAGE_TYPESCRIPT,
        &[
            tree_sitter_javascript::HIGHLIGHT_QUERY,
            tree_sitter_typescript::HIGHLIGHTS_QUERY,
        ],
        None,
    )
});

static TSX: LazyLock<Language> = LazyLock::new(|| {
    Language::new(
        "tsx",
        tree_sitter_typescript::LANGUAGE_TSX,
        &[
            tree_sitter_javascript::HIGHLIGHT_QUERY,
            tree_sitter_javascript::JSX_HIGHLIGHT_QUERY,
            tree_sitter_typescript::HIGHLIGHTS_QUERY,
        ],
        None,
    )
});

static PYTHON: LazyLock<Language> = LazyLock::new(|| {
    Language::new(
        "python",
        tree_sitter_python::LANGUAGE,
        &[tree_sitter_python::HIGHLIGHTS_QUERY],
        None,
    )
});

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_query_fits_its_grammar() {
        for name in [
            "rust",
            "toml",
            "markdown",
            "markdown_inline",
            "json",
            "yaml",
            "bash",
            "javascript",
            "typescript",
            "tsx",
            "python",
        ] {
            let language = Language::named(name).unwrap();
            assert_eq!(language.name(), name);
        }
    }

    #[test]
    fn knows_a_file_by_its_name() {
        let of = |path: &str| Language::of_path(Path::new(path)).map(Language::name);
        assert_eq!(of("crates/cox/src/main.rs"), Some("rust"));
        assert_eq!(of("Cargo.lock"), Some("toml"));
        assert_eq!(of("README.md"), Some("markdown"));
        assert_eq!(of("ci.yml"), Some("yaml"));
        assert_eq!(of("LICENSE"), None);
        assert_eq!(of("notes.txt"), None);
    }

    #[test]
    fn knows_rusts_server() {
        let rust = Language::named("rust").unwrap();
        assert_eq!(
            rust.server().map(|server| server.program),
            Some("rust-analyzer")
        );
        assert_eq!(rust.protocol_name(), "rust");
        assert_eq!(
            Language::named("bash").unwrap().protocol_name(),
            "shellscript"
        );
        assert_eq!(Language::named("toml").unwrap().server(), None);
    }

    #[test]
    fn reads_the_names_queries_capture() {
        assert_eq!(
            Capture::named("function.method"),
            Capture::Highlight(Highlight::Function)
        );
        assert_eq!(
            Capture::named("constant.builtin"),
            Capture::Highlight(Highlight::ConstantBuiltin)
        );
        assert_eq!(
            Capture::named("string.special.key"),
            Capture::Highlight(Highlight::StringSpecial)
        );
        assert_eq!(Capture::named("none"), Capture::Plain);
        assert_eq!(Capture::named("embedded"), Capture::Ignored);
        assert_eq!(Capture::named("injection.content"), Capture::Ignored);
        // Not `type`, which it only starts like.
        assert_eq!(Capture::named("typewriter"), Capture::Ignored);
    }
}
