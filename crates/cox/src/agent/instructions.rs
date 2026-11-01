//! A project's instructions: what its `COX.md` says, which the agent of
//! each chat in the project gets with the first line the human sends.

use std::path::{Path, PathBuf};

/// The file in a project's folder that holds its instructions.
pub const INSTRUCTIONS_FILE: &str = "COX.md";

/// A project's instructions, read from its folder.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Instructions {
    /// The file they were read from.
    pub path: PathBuf,
    pub text: String,
}

impl Instructions {
    /// The instructions in `folder`, or `None` when it has no
    /// [`INSTRUCTIONS_FILE`], or the file holds only white space.
    #[must_use]
    pub fn read(folder: &Path) -> Option<Self> {
        let path = folder.join(INSTRUCTIONS_FILE);
        let text = match std::fs::read_to_string(&path) {
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
        Some(Self { path, text })
    }

    /// What the agent gets ahead of the first line the human sends: the
    /// instructions, with where they come from.
    #[must_use]
    pub fn prompt(&self) -> String {
        format!(
            "The instructions for this project, from {}. Follow them for the whole session.\n\n{}",
            self.path.display(),
            self.text
        )
    }
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
    fn reads_the_instructions_in_the_folder() {
        let folder = scratch("read");
        std::fs::write(folder.join("COX.md"), "Answer in Japanese.\n").unwrap();

        let instructions = Instructions::read(&folder).expect("the folder has instructions");
        assert_eq!(instructions.path, folder.join("COX.md"));
        assert_eq!(instructions.text, "Answer in Japanese.\n");
        assert!(instructions.prompt().ends_with("\n\nAnswer in Japanese.\n"));
        assert!(
            instructions
                .prompt()
                .contains(&folder.join("COX.md").display().to_string())
        );
        let _ = std::fs::remove_dir_all(&folder);
    }

    #[test]
    fn a_folder_with_no_instructions_or_empty_ones_has_none() {
        let folder = scratch("none");
        assert_eq!(Instructions::read(&folder), None);
        std::fs::write(folder.join("COX.md"), " \n\n").unwrap();
        assert_eq!(Instructions::read(&folder), None);
        let _ = std::fs::remove_dir_all(&folder);
    }
}
