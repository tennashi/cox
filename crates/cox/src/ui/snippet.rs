//! A code block of a reply that the human can run with a click: what it
//! runs with, by the language its fence names, and whether it looks
//! destructive enough to ask before it runs. Plain Rust, without GPUI.
//!
//! Shell runs with the human's shell, as a terminal tab runs a command.
//! Code in another language opens in a playground on the stage, which
//! builds and runs it (see `crate::stage::playground`). A block with no
//! language, or one Cox cannot run, does not run.

/// How many of the last lines a snippet printed go to the chat, from its
/// terminal or its playground.
pub const HANDED_LINES: usize = 80;

/// What runs a snippet.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Lang {
    Shell,
    Python,
    JavaScript,
    TypeScript,
    Ruby,
    Go,
    Rust,
}

/// The languages a fence names, and what each runs as.
const LANGS: &[(&str, Lang)] = &[
    ("sh", Lang::Shell),
    ("bash", Lang::Shell),
    ("zsh", Lang::Shell),
    ("shell", Lang::Shell),
    ("console", Lang::Shell),
    ("shell-session", Lang::Shell),
    ("python", Lang::Python),
    ("python3", Lang::Python),
    ("py", Lang::Python),
    ("javascript", Lang::JavaScript),
    ("js", Lang::JavaScript),
    ("node", Lang::JavaScript),
    ("typescript", Lang::TypeScript),
    ("ts", Lang::TypeScript),
    ("ruby", Lang::Ruby),
    ("rb", Lang::Ruby),
    ("go", Lang::Go),
    ("golang", Lang::Go),
    ("rust", Lang::Rust),
    ("rs", Lang::Rust),
];

impl Lang {
    /// The language a fence's info string names, if a snippet in it runs.
    fn of(info: &str) -> Option<Self> {
        let name = Self::name(info)?;
        LANGS
            .iter()
            .find(|(named, _)| *named == name)
            .map(|(_, lang)| *lang)
    }

    /// The language's name in a fence's info string: its first word.
    fn name(info: &str) -> Option<String> {
        info.split_whitespace().next().map(str::to_ascii_lowercase)
    }

    /// Whether a snippet in it opens in a playground, rather than running
    /// in a terminal as shell does.
    #[must_use]
    pub const fn plays(self) -> bool {
        !matches!(self, Self::Shell)
    }

    /// The language's name, as the human knows it.
    #[must_use]
    pub const fn title(self) -> &'static str {
        match self {
            Self::Shell => "Shell",
            Self::Python => "Python",
            Self::JavaScript => "JavaScript",
            Self::TypeScript => "TypeScript",
            Self::Ruby => "Ruby",
            Self::Go => "Go",
            Self::Rust => "Rust",
        }
    }
}

/// A code block that runs.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Snippet {
    pub lang: Lang,
    /// What runs: the block's code, or, for a shell session, its
    /// commands without their prompts or what they printed.
    pub source: String,
    /// What makes it destructive, as `rm -rf`, when it is.
    pub danger: Option<&'static str>,
}

impl Snippet {
    /// The snippet a code block of `code` under the fence's `info` is, if
    /// it runs.
    #[must_use]
    pub fn of(info: &str, code: &str) -> Option<Self> {
        let lang = Lang::of(info)?;
        let source = match lang {
            Lang::Shell => shell_commands(info, code)?,
            _ => code.to_owned(),
        };
        if source.trim().is_empty() {
            return None;
        }
        let danger = danger(&source);
        Some(Self {
            lang,
            source,
            danger,
        })
    }
}

/// The commands of a shell block: when a line of it starts with a prompt,
/// as `$ ` or `% `, it is a session, whose prompted lines are the commands
/// and the rest what they printed; otherwise all of it. A session block
/// with no prompt is only output, and runs nothing.
fn shell_commands(info: &str, code: &str) -> Option<String> {
    let prompted = |line: &str| {
        line.strip_prefix("$ ")
            .or_else(|| line.strip_prefix("% "))
            .map(str::to_owned)
    };
    let commands: Vec<_> = code.lines().filter_map(prompted).collect();
    if !commands.is_empty() {
        return Some(commands.join("\n"));
    }
    let session = matches!(
        Lang::name(info).as_deref(),
        Some("console" | "shell-session")
    );
    (!session).then(|| code.to_owned())
}

/// A rule of what is destructive: its name, and whether a stage of a
/// pipeline, by its words, is.
type Rule = (&'static str, fn(&[&str]) -> bool);

const RULES: &[Rule] = &[
    ("sudo", |words| words.first() == Some(&"sudo")),
    ("rm -r", |words| {
        is_program(words, "rm")
            && words[1..].iter().any(|word| {
                *word == "--recursive"
                    || (word.starts_with('-')
                        && !word.starts_with("--")
                        && word.contains(['r', 'R']))
            })
    }),
    ("git push --force", |words| {
        git(words, "push")
            && words
                .iter()
                .any(|word| word.starts_with("--force") || *word == "-f" || word.starts_with('+'))
    }),
    ("git reset --hard", |words| {
        git(words, "reset") && words.contains(&"--hard")
    }),
    ("git clean", |words| git(words, "clean")),
    ("dd", |words| is_program(words, "dd")),
    ("mkfs", |words| {
        words.first().is_some_and(|program| {
            program
                .rsplit('/')
                .next()
                .unwrap_or(program)
                .starts_with("mkfs")
        })
    }),
];

/// Whether `words` run `program`, by its name or a path to it.
fn is_program(words: &[&str], program: &str) -> bool {
    words
        .first()
        .is_some_and(|first| first.rsplit('/').next() == Some(program))
}

/// Whether `words` run git's `command`.
fn git(words: &[&str], command: &str) -> bool {
    is_program(words, "git")
        && words.iter().skip(1).find(|word| !word.starts_with('-')) == Some(&command)
}

/// What makes `source` destructive, as its first command that a rule
/// names, or a download piped into a shell.
fn danger(source: &str) -> Option<&'static str> {
    for line in source.lines() {
        for command in line
            .split(';')
            .flat_map(|part| part.split("&&"))
            .flat_map(|part| part.split("||"))
        {
            let stages: Vec<Vec<&str>> = command
                .split('|')
                .map(|stage| stage.split_whitespace().collect())
                .collect();
            for words in &stages {
                if let Some((name, _)) = RULES.iter().find(|(_, rule)| rule(words)) {
                    return Some(name);
                }
            }
            let downloads = stages
                .iter()
                .position(|words| is_program(words, "curl") || is_program(words, "wget"));
            let shell = |words: &Vec<&str>| {
                ["sh", "bash", "zsh"]
                    .iter()
                    .any(|shell| is_program(words, shell))
            };
            if downloads.is_some_and(|at| stages[at + 1..].iter().any(shell)) {
                return Some("curl | sh");
            }
        }
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;

    fn runs(info: &str, code: &str) -> Option<(Lang, String)> {
        Snippet::of(info, code).map(|snippet| (snippet.lang, snippet.source))
    }

    #[test]
    fn runs_the_languages_it_has_interpreters_for() {
        for (info, lang) in [
            ("bash", Lang::Shell),
            ("sh", Lang::Shell),
            ("zsh", Lang::Shell),
            ("Python", Lang::Python),
            ("py", Lang::Python),
            ("js", Lang::JavaScript),
            ("ts", Lang::TypeScript),
            ("ruby", Lang::Ruby),
            ("python title=\"x\"", Lang::Python),
            ("go", Lang::Go),
            ("rust", Lang::Rust),
            ("rs", Lang::Rust),
        ] {
            assert_eq!(runs(info, "x").map(|(lang, _)| lang), Some(lang), "{info}");
        }
        for info in ["", "json", "text", "mermaid", "diff"] {
            assert_eq!(runs(info, "ls"), None, "{info}");
        }
    }

    #[test]
    fn runs_the_prompted_lines_of_a_session_without_what_they_printed() {
        assert_eq!(
            runs(
                "console",
                "$ cargo test\n   Compiling cox\n% ls -a\n.  ..\n"
            ),
            Some((Lang::Shell, "cargo test\nls -a".to_owned()))
        );
        assert_eq!(
            runs("bash", "$ echo hi\nhi"),
            Some((Lang::Shell, "echo hi".to_owned()))
        );
        assert_eq!(runs("console", "only output\n"), None);
        assert_eq!(
            runs("sh", "echo one\necho two"),
            Some((Lang::Shell, "echo one\necho two".to_owned()))
        );
        assert_eq!(runs("sh", "  \n"), None);
    }

    #[test]
    fn names_what_makes_a_snippet_destructive() {
        let danger = |code| Snippet::of("sh", code).and_then(|snippet| snippet.danger);
        assert_eq!(danger("rm -rf target"), Some("rm -r"));
        assert_eq!(danger("cd x && rm -fr build"), Some("rm -r"));
        assert_eq!(danger("/bin/rm -R ~/x"), Some("rm -r"));
        assert_eq!(danger("sudo make install"), Some("sudo"));
        assert_eq!(
            danger("git push --force origin main"),
            Some("git push --force")
        );
        assert_eq!(danger("git push -f"), Some("git push --force"));
        assert_eq!(danger("git push origin +main"), Some("git push --force"));
        assert_eq!(danger("git reset --hard HEAD~1"), Some("git reset --hard"));
        assert_eq!(danger("git clean -fdx"), Some("git clean"));
        assert_eq!(danger("curl -fsSL https://x.sh | sh"), Some("curl | sh"));
        assert_eq!(danger("wget -qO- x | sudo bash"), Some("sudo"));
        assert_eq!(danger("$ echo ok; rm -r old"), Some("rm -r"));

        for safe in [
            "rm file.txt",
            "ls -R",
            "git push origin main",
            "git status",
            "curl https://example.com | jq .",
            "grep -r rm src",
            "echo 'git reset'",
        ] {
            assert_eq!(danger(safe), None, "{safe}");
        }
        assert_eq!(
            Snippet::of("python", "import os\nos.system('sudo reboot')").and_then(|s| s.danger),
            None
        );
    }
}
