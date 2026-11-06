//! What a chat works until, when the human set a goal: a command, which
//! passes when it exits with 0, or words, which an agent judges met or
//! not. The human sets either in one input: a line that starts with `$ `
//! is a command, as at a shell's prompt, and any other line is words.

/// What a chat works until, as the human set it.
#[derive(Clone, Debug, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub enum Goal {
    /// A command, met when it exits with 0.
    Command(String),
    /// Words, met when an agent judges them so.
    Words(String),
}

/// What starts a line that sets a command.
const COMMAND_PREFIX: &str = "$ ";

impl Goal {
    /// The goal `input` sets, or `None` when it sets none: `$ ` ahead of a
    /// command, or words. Blank, it sets none.
    #[must_use]
    pub fn parse(input: &str) -> Option<Self> {
        let input = input.trim();
        match input.strip_prefix(COMMAND_PREFIX.trim_end()) {
            Some(command) if input.starts_with(COMMAND_PREFIX) || command.is_empty() => {
                let command = command.trim();
                (!command.is_empty()).then(|| Self::Command(command.to_owned()))
            }
            _ => (!input.is_empty()).then(|| Self::Words(input.to_owned())),
        }
    }

    /// The line that sets the goal, as [`Goal::parse`] reads it.
    #[must_use]
    pub fn input(&self) -> String {
        match self {
            Self::Command(command) => format!("{COMMAND_PREFIX}{command}"),
            Self::Words(words) => words.clone(),
        }
    }

    /// The command, for a goal that is one.
    #[must_use]
    pub fn command(&self) -> Option<&str> {
        match self {
            Self::Command(command) => Some(command),
            Self::Words(_) => None,
        }
    }

    /// The words, for a goal that is words.
    #[must_use]
    pub fn words(&self) -> Option<&str> {
        match self {
            Self::Words(words) => Some(words),
            Self::Command(_) => None,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_line_sets_a_command_or_words() {
        assert_eq!(
            Goal::parse("$ cargo xtask ci"),
            Some(Goal::Command("cargo xtask ci".to_owned()))
        );
        assert_eq!(
            Goal::parse("  $   make  "),
            Some(Goal::Command("make".to_owned()))
        );
        assert_eq!(
            Goal::parse("The greeting names the user."),
            Some(Goal::Words("The greeting names the user.".to_owned()))
        );
        // A `$` that starts no command is words.
        assert_eq!(
            Goal::parse("$HOME is set"),
            Some(Goal::Words("$HOME is set".to_owned()))
        );
        assert_eq!(Goal::parse(""), None);
        assert_eq!(Goal::parse("  "), None);
        assert_eq!(Goal::parse("$ "), None);
        assert_eq!(Goal::parse("$"), None);
    }

    #[test]
    fn the_line_that_set_a_goal_comes_back() {
        for line in [
            "$ cargo xtask ci",
            "The tests pass and the greeting names the user.",
        ] {
            assert_eq!(Goal::parse(line).unwrap().input(), line);
        }
        let command = Goal::Command("make".to_owned());
        assert_eq!((command.command(), command.words()), (Some("make"), None));
        let words = Goal::Words("done".to_owned());
        assert_eq!((words.command(), words.words()), (None, Some("done")));
    }
}
