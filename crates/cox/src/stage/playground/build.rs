//! What a run of a playground writes, and how it builds and runs it, by
//! its language. Plain Rust, without GPUI.
//!
//! Each run writes its files into the playground's folder, emptied first,
//! with `run.sh`, which builds them there with the language's tools, apart
//! from any workspace around the folder, and runs them from the chat's
//! folder. A run so starts clean: nothing it or another wrote before is
//! left but what the tools keep themselves, as Go's build cache, and, for
//! Rust, the versions its crates resolved to, and the target folder all
//! playgrounds share, so that a crate built once is not built again.
//!
//! Code an agent writes is often a part of a program, as a few lines with
//! no `main`. Go code with no `package` clause becomes `package main`,
//! its statements, when it declares no function or type, the body of
//! `func main`, with the standard packages it names imported (and
//! `goimports` run, when it is on `PATH`). Rust code with no `fn main` is
//! the body of one. A Rust playground's dependencies are those of a
//! `//! ```cargo` block at its top, as `rust-script` reads it, or, with
//! none, the crates its `use` lines name, at their newest versions.

use std::fmt::Write as _;
use std::path::Path;

use crate::ui::snippet::Lang;

/// The files a run writes into the playground's folder, by their names
/// there, `run.sh` among them.
#[derive(Debug, PartialEq, Eq)]
pub struct Build {
    pub files: Vec<(&'static str, String)>,
}

/// The script that builds and runs a run.
pub const SCRIPT: &str = "run.sh";

/// The versions of the crates a Rust run resolved, which the next run of
/// the playground keeps, so that cargo does not look them up again: they
/// hold no state of the program.
const LOCK: &str = "Cargo.lock";

/// What the run of `source` in `lang` writes into `folder`, sharing
/// `target` with the other playgrounds as Rust's target folder.
#[must_use]
pub fn build(lang: Lang, source: &str, folder: &Path, target: &Path) -> Build {
    let dir = quoted(&folder.to_string_lossy());
    let script = |tool: &str, steps: &str| {
        format!(
            "dir={dir}\n\
             command -v {tool} >/dev/null 2>&1 || {{\n  \
             echo \"Cox runs {title} with {tool}, which is not on PATH.\" >&2\n  exit 127\n}}\n\
             {steps}\n",
            title = lang.title(),
        )
    };
    let file = |name: &'static str, tool: &str, run: &str| Build {
        files: vec![
            (name, source.to_owned()),
            (SCRIPT, script(tool, &format!("exec {run} \"$dir/{name}\""))),
        ],
    };
    match lang {
        Lang::Shell => file("main.sh", "sh", "sh"),
        Lang::Python => file("main.py", "python3", "python3"),
        Lang::JavaScript => file("main.js", "node", "node"),
        Lang::TypeScript => file("main.ts", "npx", "npx --yes tsx"),
        Lang::Ruby => file("main.rb", "ruby", "ruby"),
        Lang::Go => Build {
            files: vec![
                ("main.go", go_main(source)),
                (
                    SCRIPT,
                    script(
                        "go",
                        "export GOWORK=off\n\
                         go -C \"$dir\" mod init play >/dev/null 2>&1\n\
                         if command -v goimports >/dev/null 2>&1; then goimports -w \"$dir/main.go\"; fi\n\
                         go -C \"$dir\" mod tidy -e >/dev/null\n\
                         go -C \"$dir\" build -o play . && exec \"$dir/play\"",
                    ),
                ),
            ],
        },
        Lang::Rust => Build {
            files: vec![
                ("Cargo.toml", cargo_toml(source)),
                ("src/main.rs", rust_main(source)),
                (
                    SCRIPT,
                    script(
                        "cargo",
                        &format!(
                            "target={}\n\
                             (cd \"$dir\" && CARGO_TARGET_DIR=\"$target\" cargo build) && exec \"$target/debug/play\"",
                            quoted(&target.to_string_lossy())
                        ),
                    ),
                ),
            ],
        },
    }
}

impl Build {
    /// Writes the files into `folder`, emptied first, answering the
    /// command line that runs them with the human's shell.
    ///
    /// # Errors
    ///
    /// When the folder cannot be emptied or a file written.
    pub fn write(&self, folder: &Path) -> std::io::Result<String> {
        let lock = std::fs::read(folder.join(LOCK)).ok();
        match std::fs::remove_dir_all(folder) {
            Err(error) if error.kind() != std::io::ErrorKind::NotFound => return Err(error),
            _ => {}
        }
        if let Some(lock) = lock {
            std::fs::create_dir_all(folder)?;
            std::fs::write(folder.join(LOCK), lock)?;
        }
        for (name, text) in &self.files {
            let path = folder.join(name);
            if let Some(parent) = path.parent() {
                std::fs::create_dir_all(parent)?;
            }
            std::fs::write(path, text)?;
        }
        Ok(format!(
            "sh {}",
            quoted(&folder.join(SCRIPT).to_string_lossy())
        ))
    }
}

/// The command line that says `message` and fails, for a run that
/// cannot start.
#[must_use]
pub fn failing(message: &str) -> String {
    format!("echo {} >&2; exit 1", quoted(message))
}

/// `text` between single quotes, for the shell to read as one word.
fn quoted(text: &str) -> String {
    format!("'{}'", text.replace('\'', r"'\''"))
}

/// Whether a line starts at the left edge with one of `words`.
fn starts_with_any(line: &str, words: &[&str]) -> bool {
    words.iter().any(|word| line.starts_with(word))
}

/// Go code that builds as a program: `source` as it is, when it has a
/// `package` clause; else in `package main`, with the standard packages
/// it names imported, its statements in `func main` when it declares no
/// function or type.
fn go_main(source: &str) -> String {
    if source.lines().any(|line| line.starts_with("package ")) {
        return source.to_owned();
    }
    let (imports, rest) = go_imports(source);
    let mut imported: Vec<String> = imports.clone();
    for (name, path) in GO_PACKAGES {
        let named = names(rest, name);
        if named
            && !imported
                .iter()
                .any(|import| import.contains(&format!("\"{path}\"")))
        {
            imported.push(format!("import \"{path}\""));
        }
    }
    let declares = rest
        .lines()
        .any(|line| starts_with_any(line, &["func ", "type "]));
    let body = if declares {
        rest.to_owned()
    } else {
        format!("func main() {{\n{}}}\n", indented(rest))
    };
    let mut program = String::from("package main\n\n");
    if !imported.is_empty() {
        program.push_str(&imported.join("\n"));
        program.push_str("\n\n");
    }
    program.push_str(&body);
    program
}

/// The import declarations at the top of Go code, one each, and the code
/// after them.
fn go_imports(source: &str) -> (Vec<String>, &str) {
    let mut imports = Vec::new();
    let mut rest = source;
    loop {
        let trimmed = rest.trim_start();
        let line_end = trimmed.find('\n').map_or(trimmed.len(), |at| at + 1);
        if trimmed.starts_with("import (") {
            let Some(end) = trimmed.find(')') else { break };
            imports.extend(
                trimmed["import (".len()..end]
                    .lines()
                    .map(str::trim)
                    .filter(|line| !line.is_empty())
                    .map(|line| format!("import {line}")),
            );
            rest = &trimmed[end + 1..];
        } else if trimmed.starts_with("import ") {
            imports.push(trimmed[..line_end].trim().to_owned());
            rest = &trimmed[line_end..];
        } else {
            return (imports, rest.trim_start_matches('\n'));
        }
    }
    (imports, rest)
}

/// The standard packages Go code most names, by the name it names them
/// by, and their import paths.
const GO_PACKAGES: &[(&str, &str)] = &[
    ("atomic", "sync/atomic"),
    ("base64", "encoding/base64"),
    ("bufio", "bufio"),
    ("bytes", "bytes"),
    ("cmp", "cmp"),
    ("context", "context"),
    ("errors", "errors"),
    ("exec", "os/exec"),
    ("filepath", "path/filepath"),
    ("fmt", "fmt"),
    ("hex", "encoding/hex"),
    ("http", "net/http"),
    ("io", "io"),
    ("json", "encoding/json"),
    ("log", "log"),
    ("maps", "maps"),
    ("math", "math"),
    ("os", "os"),
    ("rand", "math/rand"),
    ("reflect", "reflect"),
    ("regexp", "regexp"),
    ("runtime", "runtime"),
    ("sha256", "crypto/sha256"),
    ("slices", "slices"),
    ("sort", "sort"),
    ("strconv", "strconv"),
    ("strings", "strings"),
    ("sync", "sync"),
    ("time", "time"),
    ("unicode", "unicode"),
    ("url", "net/url"),
    ("utf8", "unicode/utf8"),
];

/// Whether `code` names something of the package `name`, as `name.X`.
fn names(code: &str, name: &str) -> bool {
    code.match_indices(&format!("{name}.")).any(|(at, _)| {
        !code[..at]
            .chars()
            .next_back()
            .is_some_and(|before| before.is_alphanumeric() || before == '_' || before == '.')
    })
}

/// Each line of `code` one step in, ending with a line break.
fn indented(code: &str) -> String {
    code.lines()
        .map(|line| {
            if line.is_empty() {
                "\n".to_owned()
            } else {
                format!("\t{line}\n")
            }
        })
        .collect()
}

/// Rust code that builds as a program: `source` as it is, when it has a
/// `fn main`; else its body, under the crate's inner attributes and doc
/// comments.
fn rust_main(source: &str) -> String {
    if source.contains("fn main(") {
        return source.to_owned();
    }
    let lines: Vec<&str> = source.lines().collect();
    let top = lines
        .iter()
        .take_while(|line| {
            let line = line.trim_start();
            line.is_empty() || starts_with_any(line, &["//!", "#!["])
        })
        .count();
    let mut program = String::new();
    for line in &lines[..top] {
        let _ = writeln!(program, "{line}");
    }
    program.push_str("fn main() {\n");
    for line in &lines[top..] {
        if line.is_empty() {
            program.push('\n');
        } else {
            let _ = writeln!(program, "    {line}");
        }
    }
    program.push_str("}\n");
    program
}

/// The manifest of a Rust playground, `play`, with its dependencies.
fn cargo_toml(source: &str) -> String {
    let manifest = cargo_block(source).unwrap_or_else(|| {
        let crates = used_crates(source);
        let mut manifest = String::from("[dependencies]\n");
        for name in crates {
            let _ = writeln!(manifest, "{name} = \"*\"");
        }
        manifest
    });
    format!(
        "[package]\nname = \"play\"\nversion = \"0.0.0\"\nedition = \"2021\"\n\n\
         [workspace]\n\n{manifest}"
    )
}

/// What a `//! ```cargo` block at the top of Rust code says, without the
/// comment's marks: the manifest's lines past `[package]`.
fn cargo_block(source: &str) -> Option<String> {
    let mut lines = source
        .lines()
        .map_while(|line| line.trim_start().strip_prefix("//!"))
        .map(|line| line.strip_prefix(' ').unwrap_or(line));
    lines.find(|line| line.trim() == "```cargo")?;
    let mut block = String::new();
    for line in lines {
        if line.trim() == "```" {
            return Some(block);
        }
        block.push_str(line);
        block.push('\n');
    }
    None
}

/// The crates Rust code's `use` and `extern crate` lines name, but the
/// standard library's and the code's own modules, in order, once each.
fn used_crates(source: &str) -> Vec<String> {
    const OWN: &[&str] = &[
        "std",
        "core",
        "alloc",
        "crate",
        "self",
        "super",
        "proc_macro",
    ];
    let declared: Vec<&str> = source
        .lines()
        .filter_map(|line| line.trim_start().strip_prefix("mod "))
        .filter_map(|rest| rest.split(|c: char| !is_ident(c)).next())
        .collect();
    let mut crates: Vec<String> = Vec::new();
    for line in source.lines() {
        let line = line.trim_start();
        let line = line.strip_prefix("pub ").unwrap_or(line);
        let Some(path) = line
            .strip_prefix("use ")
            .or_else(|| line.strip_prefix("extern crate "))
        else {
            continue;
        };
        let path = path.trim_start().trim_start_matches("::");
        let name: String = path.chars().take_while(|&c| is_ident(c)).collect();
        let lower = name.chars().all(|c| !c.is_uppercase());
        if !name.is_empty()
            && lower
            && !OWN.contains(&name.as_str())
            && !declared.contains(&name.as_str())
            && !crates.contains(&name)
        {
            crates.push(name);
        }
    }
    crates
}

fn is_ident(c: char) -> bool {
    c.is_alphanumeric() || c == '_'
}

#[cfg(test)]
mod tests {
    use super::*;

    fn file<'a>(build: &'a Build, name: &str) -> &'a str {
        &build
            .files
            .iter()
            .find(|(named, _)| *named == name)
            .unwrap()
            .1
    }

    #[test]
    fn go_statements_become_the_body_of_main_with_their_imports() {
        assert_eq!(
            go_main(
                "xs := []int{3, 1}\nsort.Ints(xs)\n\nfmt.Println(strings.Repeat(\"a\", xs[0]))\n"
            ),
            "package main\n\nimport \"fmt\"\nimport \"sort\"\nimport \"strings\"\n\n\
             func main() {\n\txs := []int{3, 1}\n\tsort.Ints(xs)\n\n\tfmt.Println(strings.Repeat(\"a\", xs[0]))\n}\n"
        );
    }

    #[test]
    fn go_keeps_the_imports_it_has_above_main() {
        assert_eq!(
            go_main("import (\n\t\"fmt\"\n\tm \"math\"\n)\n\nfmt.Println(m.Pi)\n"),
            "package main\n\nimport \"fmt\"\nimport m \"math\"\n\nfunc main() {\n\tfmt.Println(m.Pi)\n}\n"
        );
        assert_eq!(
            go_main("import \"fmt\"\nfmt.Println(1)"),
            "package main\n\nimport \"fmt\"\n\nfunc main() {\n\tfmt.Println(1)\n}\n"
        );
    }

    #[test]
    fn go_with_declarations_only_gets_its_package() {
        let source = "func main() {\n\tfmt.Println(double(2))\n}\n\nfunc double(n int) int { return n * 2 }\n";
        assert_eq!(
            go_main(source),
            format!("package main\n\nimport \"fmt\"\n\n{source}")
        );
    }

    #[test]
    fn go_with_a_package_is_left_as_it_is() {
        let source = "package main\n\nfunc main() {}\n";
        assert_eq!(go_main(source), source);
    }

    #[test]
    fn go_names_a_package_only_as_a_whole_word() {
        assert!(names("fmt.Println(x)", "fmt"));
        assert!(names("x := (os.Args)", "os"));
        assert!(!names("myos.Thing()", "os"));
        assert!(!names("a.os.b", "os"));
    }

    #[test]
    fn rust_statements_become_the_body_of_main() {
        assert_eq!(
            rust_main("//! A doc.\nuse std::fmt;\nlet x = 1;\n\nprintln!(\"{x}\");"),
            "//! A doc.\nfn main() {\n    use std::fmt;\n    let x = 1;\n\n    println!(\"{x}\");\n}\n"
        );
        let whole = "fn main() {\n    println!(\"hi\");\n}\n";
        assert_eq!(rust_main(whole), whole);
    }

    #[test]
    fn rust_dependencies_come_from_a_cargo_block() {
        let source =
            "//! ```cargo\n//! [dependencies]\n//! itoa = \"1\"\n//! ```\nuse serde_json::json;\n";
        assert_eq!(
            cargo_toml(source),
            "[package]\nname = \"play\"\nversion = \"0.0.0\"\nedition = \"2021\"\n\n\
             [workspace]\n\n[dependencies]\nitoa = \"1\"\n"
        );
    }

    #[test]
    fn rust_dependencies_without_a_block_are_the_crates_it_uses() {
        let source = "use std::io;\nuse serde_json::{json, Value};\npub use ::itoa;\n\
                      extern crate rand;\nuse self::inner::X;\nmod inner { pub struct X; }\n\
                      use inner::X as Y;\nuse Color::*;\nuse serde_json::Map;\n";
        assert_eq!(used_crates(source), ["serde_json", "itoa", "rand"]);
        assert!(
            cargo_toml(source)
                .ends_with("[dependencies]\nserde_json = \"*\"\nitoa = \"*\"\nrand = \"*\"\n")
        );
    }

    #[test]
    fn rust_builds_in_its_folder_into_the_shared_target_and_runs_from_the_chats() {
        let build = build(
            Lang::Rust,
            "println!(\"hi\");",
            Path::new("/p/1"),
            Path::new("/p/target"),
        );
        let script = file(&build, SCRIPT);
        assert!(
            script.contains(
                "target='/p/target'\n\
                 (cd \"$dir\" && CARGO_TARGET_DIR=\"$target\" cargo build) && exec \"$target/debug/play\""
            ),
            "{script}"
        );
        assert!(file(&build, "src/main.rs").starts_with("fn main() {"));
    }

    #[test]
    fn a_run_empties_its_folder_and_runs_its_script() {
        let scratch = coxd::testing::TempDir::new();
        let folder = scratch.path().join("play's");
        std::fs::create_dir_all(&folder).unwrap();
        std::fs::write(folder.join("left.txt"), "").unwrap();
        let build = build(Lang::Shell, "echo \"$PWD\"\n", &folder, Path::new("/t"));

        let command = build.write(&folder).unwrap();

        assert!(
            !folder.join("left.txt").exists(),
            "nothing of a run before is left"
        );
        std::fs::write(folder.join(LOCK), "versions").unwrap();
        build.write(&folder).unwrap();
        assert_eq!(
            std::fs::read_to_string(folder.join(LOCK)).unwrap(),
            "versions",
            "but the versions its crates resolved to"
        );
        let output = std::process::Command::new("sh")
            .args(["-c", &command])
            .current_dir(scratch.path())
            .output()
            .unwrap();
        assert_eq!(
            String::from_utf8_lossy(&output.stdout).trim(),
            std::fs::canonicalize(scratch.path())
                .unwrap()
                .to_string_lossy(),
            "it runs from where the command runs, the chat's folder"
        );
    }

    #[test]
    fn rust_builds_apart_from_a_workspace_around_it() {
        let scratch = coxd::testing::TempDir::new();
        std::fs::write(
            scratch.path().join("Cargo.toml"),
            "[workspace]\nmembers = []\n",
        )
        .unwrap();
        let folder = scratch.path().join("playgrounds/1");
        let target = scratch.path().join("playgrounds/target");
        let command = build(Lang::Rust, "println!(\"{}\", 6 * 7);", &folder, &target)
            .write(&folder)
            .unwrap();

        let output = std::process::Command::new("sh")
            .args(["-c", &command])
            .current_dir(scratch.path())
            .output()
            .unwrap();

        assert_eq!(
            String::from_utf8_lossy(&output.stdout).trim(),
            "42",
            "{}",
            String::from_utf8_lossy(&output.stderr)
        );
    }

    #[test]
    fn a_missing_tool_says_so() {
        let scratch = coxd::testing::TempDir::new();
        let folder = scratch.path().join("play");
        build(Lang::Ruby, "puts 1", &folder, Path::new("/t"))
            .write(&folder)
            .unwrap();
        let output = std::process::Command::new("/bin/sh")
            .arg(folder.join(SCRIPT))
            .env("PATH", "/nonexistent")
            .output()
            .unwrap();
        assert_eq!(output.status.code(), Some(127));
        assert_eq!(
            String::from_utf8_lossy(&output.stderr).trim(),
            "Cox runs Ruby with ruby, which is not on PATH."
        );
    }
}
