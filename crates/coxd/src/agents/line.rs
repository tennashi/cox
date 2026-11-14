//! An agent's command as one line, as a shell takes it, for the human to
//! write on the settings screen: `CLAUDE_CODE_ENABLE_TODO_TOOLS=1 npx -y
//! @agentclientprotocol/claude-agent-acp`, which `agents.json` keeps as
//! the program and its arguments, each a string of its own.

/// The words of `line`, split at spaces, as a shell splits them: a word in
/// single quotes as it is, one in double quotes but for `\"`, `\\`, `\$`
/// and `` \` ``, and a backslash outside quotes keeping the character
/// after it. Nothing else of a shell's applies, as `$NAME` or `*`.
///
/// # Errors
///
/// When a quote is not closed, or the line ends in a backslash.
pub fn split(line: &str) -> Result<Vec<String>, &'static str> {
    let mut words = Vec::new();
    let mut word: Option<String> = None;
    let mut chars = line.chars();
    while let Some(c) = chars.next() {
        match c {
            c if c.is_whitespace() => {
                words.extend(word.take());
            }
            '\'' => {
                let word = word.get_or_insert_with(String::new);
                loop {
                    match chars.next() {
                        Some('\'') => break,
                        Some(c) => word.push(c),
                        None => return Err("A single quote is not closed."),
                    }
                }
            }
            '"' => {
                let word = word.get_or_insert_with(String::new);
                loop {
                    match chars.next() {
                        Some('"') => break,
                        Some('\\') => match chars.next() {
                            Some(c @ ('"' | '\\' | '$' | '`')) => word.push(c),
                            Some(c) => {
                                word.push('\\');
                                word.push(c);
                            }
                            None => return Err("A double quote is not closed."),
                        },
                        Some(c) => word.push(c),
                        None => return Err("A double quote is not closed."),
                    }
                }
            }
            '\\' => match chars.next() {
                Some(c) => word.get_or_insert_with(String::new).push(c),
                None => return Err("The line ends in a backslash."),
            },
            c => word.get_or_insert_with(String::new).push(c),
        }
    }
    words.extend(word);
    Ok(words)
}

/// `command` as one line, which [`split`] splits as it is again: each
/// word as it is, unless it is empty or has what a shell takes apart, in
/// single quotes then. `NAME=value` keeps its name outside them.
#[must_use]
pub fn joined(command: &[String]) -> String {
    command
        .iter()
        .map(|word| match word.split_once('=') {
            Some((name, value)) if is_name(name) => format!("{name}={}", quoted(value)),
            _ => quoted(word),
        })
        .collect::<Vec<_>>()
        .join(" ")
}

/// `word`, in single quotes when a shell would take it apart.
fn quoted(word: &str) -> String {
    let plain = !word.is_empty()
        && word.chars().all(|c| {
            c.is_alphanumeric()
                || matches!(
                    c,
                    '-' | '_' | '.' | '/' | ':' | '@' | '=' | '+' | ',' | '%' | '{' | '}'
                )
        });
    if plain {
        word.to_owned()
    } else {
        format!("'{}'", word.replace('\'', r"'\''"))
    }
}

/// Whether `name` is the name of a variable of the environment.
fn is_name(name: &str) -> bool {
    !name.is_empty() && name.chars().all(|c| c.is_ascii_alphanumeric() || c == '_')
}

#[cfg(test)]
mod tests {
    use super::*;

    fn words(words: &[&str]) -> Vec<String> {
        words.iter().map(|&word| word.to_owned()).collect()
    }

    #[test]
    fn a_line_is_split_as_a_shell_splits_it() {
        assert_eq!(
            split("CLAUDE_CODE_ENABLE_TODO_TOOLS=1  npx -y @agentclientprotocol/claude-agent-acp"),
            Ok(words(&[
                "CLAUDE_CODE_ENABLE_TODO_TOOLS=1",
                "npx",
                "-y",
                "@agentclientprotocol/claude-agent-acp"
            ]))
        );
        assert_eq!(
            split(r#"NOTE='a b' agent --name "it's \"so\"" a\ b ''"#),
            Ok(words(&[
                "NOTE=a b",
                "agent",
                "--name",
                r#"it's "so""#,
                "a b",
                ""
            ]))
        );
        assert_eq!(split("   "), Ok(Vec::new()));
        assert!(split("agent 'open").is_err());
        assert!(split("agent \"open").is_err());
        assert!(split("agent \\").is_err());
    }

    #[test]
    fn a_command_joined_splits_as_it_was() {
        for command in [
            words(&["npx", "-y", "@agentclientprotocol/claude-agent-acp"]),
            words(&["plamo-translate", "--from", "{from}", "--to", "{to}"]),
            words(&["NOTE=a b", "/opt/my agent/bin", "it's", "", "$HOME", "a\"b"]),
        ] {
            assert_eq!(split(&joined(&command)), Ok(command));
        }
        assert_eq!(joined(&words(&["NOTE=a b", "agent"])), "NOTE='a b' agent");
    }
}
