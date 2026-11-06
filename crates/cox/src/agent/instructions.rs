//! What each chat's agent is told first, ahead of the first line the
//! human sends: how the human works with Cox, whatever the project, then
//! what the project's `COX.md` says.
//!
//! How the human works is the `COX.md` of their workflow's folder (see
//! [`crate::settings::workflow`]), or, when they keep none, Cox's own
//! (`instructions/cox.md`), which says when to use Cox's tools, such as
//! when to try work out in a sketch. The tools themselves only say what
//! they do, so that how they are used is the human's to set.

use std::path::{Path, PathBuf};

/// The file in a project's folder that holds its instructions, and in the
/// human's workflow, theirs.
pub const INSTRUCTIONS_FILE: &str = "COX.md";

/// How Cox would have the agent use its tools, for a human who says
/// nothing of their own.
const COX: &str = include_str!("instructions/cox.md");

/// How what the agent is told of Cox's own starts, which tells it apart
/// from the rest of a prompt.
pub const COX_START: &str = "How to work with Cox, from Cox itself";

/// What the agent is told first, part by part.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Instructions {
    parts: Vec<Part>,
}

/// One part of what the agent is told first.
#[derive(Clone, Debug, PartialEq, Eq)]
struct Part {
    whose: Whose,
    text: String,
}

/// Whose instructions a part is.
#[derive(Clone, Debug, PartialEq, Eq)]
enum Whose {
    /// Cox's own, for a human who keeps none.
    Cox,
    /// The human's, for every project, from the file at the path.
    Human(PathBuf),
    /// The project's, from the file at the path.
    Project(PathBuf),
}

impl Instructions {
    /// What the agent of a chat that works in `folder` is told first: the
    /// human's instructions, kept in their workflow's folder, or else
    /// Cox's, then those of the project, in `folder`.
    #[must_use]
    pub fn read(folder: &Path) -> Self {
        Self::read_with(folder, Some(crate::settings::workflow().as_path()))
    }

    /// What an agent that is not a chat's, as a goal's judge, is told
    /// first: only the instructions of the project, in `folder`, since how
    /// the human works with Cox's tools is for the chats' agents, which
    /// have them. `None` when the project has none.
    #[must_use]
    pub fn project(folder: &Path) -> Option<Self> {
        let (path, text) = file(&folder.join(INSTRUCTIONS_FILE))?;
        Some(Self {
            parts: vec![Part {
                whose: Whose::Project(path),
                text,
            }],
        })
    }

    /// As [`Self::read`], with the human's workflow in `workflow`.
    #[must_use]
    pub fn read_with(folder: &Path, workflow: Option<&Path>) -> Self {
        let human = workflow
            .and_then(|workflow| file(&workflow.join(INSTRUCTIONS_FILE)))
            .map_or_else(
                || Part {
                    whose: Whose::Cox,
                    text: COX.to_owned(),
                },
                |(path, text)| Part {
                    whose: Whose::Human(path),
                    text,
                },
            );
        let project = file(&folder.join(INSTRUCTIONS_FILE)).map(|(path, text)| Part {
            whose: Whose::Project(path),
            text,
        });
        Self {
            parts: std::iter::once(human).chain(project).collect(),
        }
    }

    /// The files the instructions come from, for the human to see which
    /// the agent gets: Cox's own come from none.
    #[must_use]
    pub fn paths(&self) -> Vec<PathBuf> {
        self.parts
            .iter()
            .filter_map(|part| match &part.whose {
                Whose::Cox => None,
                Whose::Human(path) | Whose::Project(path) => Some(path.clone()),
            })
            .collect()
    }

    /// What the agent gets ahead of the first line the human sends: each
    /// part, with whose it is and where it comes from.
    #[must_use]
    pub fn prompts(&self) -> Vec<String> {
        self.parts.iter().map(Part::prompt).collect()
    }
}

impl Part {
    fn prompt(&self) -> String {
        let from = match &self.whose {
            Whose::Cox => format!(
                "{COX_START}, for every project. Follow it for the whole session, unless the \
                 project's instructions say otherwise."
            ),
            Whose::Human(path) => format!(
                "The human's own instructions, from {}, for every project. Follow them for the \
                 whole session, unless the project's instructions say otherwise.",
                path.display()
            ),
            Whose::Project(path) => format!(
                "The instructions for this project, from {}. Follow them for the whole session.",
                path.display()
            ),
        };
        format!("{from}\n\n{}", self.text)
    }
}

/// The path and text of the instructions in the file at `path`, or `None`
/// when there is no such file, or it holds only white space.
fn file(path: &Path) -> Option<(PathBuf, String)> {
    let text = match std::fs::read_to_string(path) {
        Ok(text) => text,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return None,
        Err(error) => {
            tracing::error!("cannot read {}: {error}", path.display());
            return None;
        }
    };
    if text.trim().is_empty() {
        return None;
    }
    Some((path.to_owned(), text))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn scratch(name: &str) -> PathBuf {
        let folder =
            std::env::temp_dir().join(format!("cox-instructions-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&folder);
        std::fs::create_dir_all(&folder).unwrap();
        folder
    }

    #[test]
    fn the_human_s_instructions_go_ahead_of_the_project_s() {
        let folder = scratch("read");
        let project = folder.join("project");
        let workflow = folder.join("workflow");
        std::fs::create_dir_all(&project).unwrap();
        std::fs::create_dir_all(&workflow).unwrap();
        std::fs::write(project.join("COX.md"), "Answer in Japanese.\n").unwrap();
        std::fs::write(workflow.join("COX.md"), "Try vague work in a sketch.\n").unwrap();

        let instructions = Instructions::read_with(&project, Some(&workflow));
        assert_eq!(
            instructions.paths(),
            [workflow.join("COX.md"), project.join("COX.md")]
        );
        let prompts = instructions.prompts();
        assert_eq!(prompts.len(), 2);
        assert!(prompts[0].starts_with("The human's own instructions, from "));
        assert!(prompts[0].contains(&workflow.join("COX.md").display().to_string()));
        assert!(prompts[0].ends_with("\n\nTry vague work in a sketch.\n"));
        assert!(prompts[1].contains(&project.join("COX.md").display().to_string()));
        assert!(prompts[1].ends_with("\n\nAnswer in Japanese.\n"));
        let _ = std::fs::remove_dir_all(&folder);
    }

    #[test]
    fn a_human_with_no_instructions_gets_cox_s() {
        let folder = scratch("none");
        let instructions = Instructions::read_with(&folder, Some(&folder.join("workflow")));
        assert_eq!(instructions.paths(), Vec::<PathBuf>::new());
        let prompts = instructions.prompts();
        assert_eq!(prompts.len(), 1);
        assert!(prompts[0].starts_with(COX_START));
        assert!(prompts[0].contains("start_sketch"));

        // Empty instructions count as none.
        std::fs::write(folder.join("COX.md"), " \n\n").unwrap();
        assert_eq!(
            Instructions::read_with(&folder, Some(&folder)),
            instructions
        );
        assert_eq!(Instructions::read_with(&folder, None), instructions);
        let _ = std::fs::remove_dir_all(&folder);
    }
}
