//! Which language a file is in, as the Language Server Protocol names it
//! (its `languageId`), by the file's name: what picks the server that
//! knows the file, apart from the grammars the editor draws code with
//! (see [`crate::language`]), so that a language with a server and no
//! grammar, such as Go, is known to its server all the same.

use std::path::Path;

/// The language of the file at `path`, as the protocol names it, if Cox
/// knows it: the names the protocol lists, and for a language it does
/// not list, such as TOML or Zig, the name its servers go by.
#[must_use]
pub fn id_of(path: &Path) -> Option<&'static str> {
    let name = path.file_name()?.to_str()?;
    let id = match name {
        "Cargo.lock" | "uv.lock" | "poetry.lock" => "toml",
        "Dockerfile" | "Containerfile" => "dockerfile",
        "Makefile" | "makefile" | "GNUmakefile" => "makefile",
        ".bashrc" | ".bash_profile" | ".profile" | ".zshrc" => "shellscript",
        _ => match name.rsplit_once('.')?.1 {
            "rs" => "rust",
            "py" | "pyi" => "python",
            "js" | "mjs" | "cjs" => "javascript",
            "jsx" => "javascriptreact",
            "ts" | "mts" | "cts" => "typescript",
            "tsx" => "typescriptreact",
            "go" => "go",
            "c" | "h" => "c",
            "cc" | "cpp" | "cxx" | "hh" | "hpp" | "hxx" => "cpp",
            "m" => "objective-c",
            "mm" => "objective-cpp",
            "java" => "java",
            "kt" | "kts" => "kotlin",
            "scala" | "sc" => "scala",
            "cs" => "csharp",
            "fs" | "fsi" | "fsx" => "fsharp",
            "swift" => "swift",
            "dart" => "dart",
            "rb" => "ruby",
            "php" => "php",
            "pl" | "pm" => "perl",
            "lua" => "lua",
            "zig" => "zig",
            "hs" => "haskell",
            "ml" | "mli" => "ocaml",
            "ex" | "exs" => "elixir",
            "erl" | "hrl" => "erlang",
            "clj" | "cljs" | "cljc" | "edn" => "clojure",
            "r" | "R" => "r",
            "nix" => "nix",
            "sh" | "bash" | "zsh" => "shellscript",
            "ps1" => "powershell",
            "sql" => "sql",
            "html" | "htm" => "html",
            "css" => "css",
            "scss" => "scss",
            "less" => "less",
            "vue" => "vue",
            "svelte" => "svelte",
            "xml" => "xml",
            "json" | "jsonc" => "json",
            "yaml" | "yml" => "yaml",
            "toml" => "toml",
            "md" | "markdown" => "markdown",
            "tex" => "latex",
            _ => return None,
        },
    };
    Some(id)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn knows_a_file_by_its_name() {
        let of = |path: &str| id_of(Path::new(path));
        assert_eq!(of("crates/cox/src/main.rs"), Some("rust"));
        assert_eq!(of("app/main.go"), Some("go"));
        assert_eq!(of("web/App.tsx"), Some("typescriptreact"));
        assert_eq!(of("deploy.sh"), Some("shellscript"));
        assert_eq!(of("Cargo.lock"), Some("toml"));
        assert_eq!(of("Makefile"), Some("makefile"));
        assert_eq!(of("LICENSE"), None);
        assert_eq!(of("notes.txt"), None);
    }
}
