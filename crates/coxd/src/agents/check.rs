//! Trying an agent's command out, for the settings screen's "Test": Cox
//! starts the agent, as a chat would, and asks it over ACP to open a
//! session, which says whether it works, has the human log in first, or
//! cannot start, as when `npx` is not on Cox's `PATH`.

use std::path::{Path, PathBuf};
use std::time::Duration;

use cox_acp::{Login, LoginMethod};

/// How long the agent has to open a session: long enough for `npx` to
/// fetch the agent's package the first time.
const PATIENCE: Duration = Duration::from_mins(2);

/// What came of trying an agent out.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Checked {
    /// It opened a session, whose settings are these, each as its name
    /// and what it is set to.
    Works(Vec<(String, String)>),
    /// It has the human log in first, in these ways, by their names, as a
    /// chat asks them to.
    LogsIn(Vec<String>),
    /// Its program is not found, as on Cox's `PATH`.
    NotFound(String),
    /// It failed so.
    Failed(String),
}

impl Checked {
    /// What it says to the human.
    #[must_use]
    pub fn said(&self) -> String {
        match self {
            Self::Works(settings) if settings.is_empty() => {
                "It works: it opened a session.".to_owned()
            }
            Self::Works(settings) => format!(
                "It works: it opened a session, with {}.",
                settings
                    .iter()
                    .map(|(name, value)| format!("{name} {value}"))
                    .collect::<Vec<_>>()
                    .join(", ")
            ),
            Self::LogsIn(ways) if ways.is_empty() => {
                "It works, once you log in, which a chat asks you to do.".to_owned()
            }
            Self::LogsIn(ways) => format!(
                "It works, once you log in, which a chat asks you to do: {}.",
                ways.join(", ")
            ),
            Self::NotFound(program) => format!(
                "{program} is not found on the PATH Cox runs agents with: {}.",
                std::env::var("PATH").unwrap_or_default()
            ),
            Self::Failed(why) => format!("It failed: {why}"),
        }
    }

    /// Whether it says the agent works.
    #[must_use]
    pub fn works(&self) -> bool {
        matches!(self, Self::Works(_) | Self::LogsIn(_))
    }
}

/// The agent asked the human to log in, which a test does not.
#[derive(Debug)]
struct AsksToLogIn(Vec<String>);

impl std::fmt::Display for AsksToLogIn {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("the agent asks the human to log in")
    }
}

impl std::error::Error for AsksToLogIn {}

/// Tries `command` out, the program and its arguments, after what it sets
/// of the environment, as `NAME=value`, working in `cwd`. It blocks until
/// the agent has opened a session, or failed to, or [`PATIENCE`] runs
/// out, so it runs on a thread of its own.
#[must_use]
pub fn check(command: Vec<String>, cwd: &Path) -> Checked {
    let Some(program) = super::program(&command).map(str::to_owned) else {
        return Checked::Failed("the command names no program.".to_owned());
    };
    // A program's path is from the folder Cox runs in, as the agent
    // starts there.
    let here = std::env::current_dir().unwrap_or_default();
    if find(&program, std::env::var_os("PATH").as_deref(), &here).is_none() {
        return Checked::NotFound(program);
    }
    crate::process::close_on_exec();
    let runtime = match tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
    {
        Ok(runtime) => runtime,
        Err(error) => return Checked::Failed(error.to_string()),
    };
    let opened = runtime.block_on(async {
        let agent = cox_acp::launch(command)?;
        let refuse = async |ways: Vec<LoginMethod>, _: Option<String>| -> anyhow::Result<Login> {
            Err(AsksToLogIn(ways.into_iter().map(|way| way.name).collect()).into())
        };
        let session = cox_acp::connect(
            agent,
            cwd.to_owned(),
            Vec::new(),
            None,
            |_| {},
            refuse,
            async |session| {
                Ok(session
                    .settings()
                    .iter()
                    .map(|setting| {
                        let value = setting
                            .choices
                            .iter()
                            .find(|choice| choice.id == setting.value)
                            .map_or_else(|| setting.value.clone(), |choice| choice.name.clone());
                        (setting.name.clone(), value)
                    })
                    .collect())
            },
        );
        tokio::time::timeout(PATIENCE, session).await.map_err(|_| {
            anyhow::anyhow!("it opened no session in {} seconds.", PATIENCE.as_secs())
        })?
    });
    match opened {
        Ok(settings) => Checked::Works(settings),
        Err(error) => {
            if let Some(AsksToLogIn(ways)) = error.downcast_ref() {
                return Checked::LogsIn(ways.clone());
            }
            if error.chain().any(|cause| {
                cause
                    .downcast_ref::<std::io::Error>()
                    .is_some_and(|error| error.kind() == std::io::ErrorKind::NotFound)
            }) {
                return Checked::NotFound(program);
            }
            Checked::Failed(format!("{error:#}"))
        }
    }
}

/// Where `program` is: on `path`, as a shell finds a program by its
/// name, or, for a path, there, from `here` when it is relative.
#[must_use]
pub fn find(program: &str, path: Option<&std::ffi::OsStr>, here: &Path) -> Option<PathBuf> {
    use std::os::unix::fs::PermissionsExt as _;
    let runs = |file: &Path| {
        std::fs::metadata(file)
            .is_ok_and(|metadata| metadata.is_file() && metadata.permissions().mode() & 0o111 != 0)
    };
    if program.contains('/') {
        let file = here.join(program);
        return runs(&file).then_some(file);
    }
    std::env::split_paths(path?)
        .map(|folder| folder.join(program))
        .find(|file| runs(file))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::testing::TempDir;

    #[test]
    fn a_program_is_found_on_the_path_or_where_its_path_says() {
        let cwd = Path::new("/");
        assert_eq!(
            find("sh", Some("/nowhere:/bin".as_ref()), cwd),
            Some(PathBuf::from("/bin/sh"))
        );
        assert_eq!(find("sh", Some("/nowhere".as_ref()), cwd), None);
        assert_eq!(find("/bin/sh", None, cwd), Some(PathBuf::from("/bin/sh")));
        assert_eq!(find("bin/sh", None, cwd), Some(PathBuf::from("/bin/sh")));
    }

    #[test]
    fn a_file_that_does_not_run_is_not_a_program() {
        use std::os::unix::fs::PermissionsExt as _;
        let folder = TempDir::new();
        let notes = folder.join("notes");
        std::fs::write(&notes, "").unwrap();
        std::fs::set_permissions(&notes, std::fs::Permissions::from_mode(0o644)).unwrap();
        std::fs::create_dir(folder.join("tools")).unwrap();
        let path = folder.as_os_str();
        assert_eq!(find("notes", Some(path), &folder), None);
        assert_eq!(find("./notes", None, &folder), None);
        assert_eq!(find("tools", Some(path), &folder), None);
        std::fs::set_permissions(&notes, std::fs::Permissions::from_mode(0o755)).unwrap();
        assert_eq!(find("notes", Some(path), &folder), Some(notes.clone()));
        assert_eq!(find("./notes", None, &folder), Some(folder.join("./notes")));
    }

    #[test]
    fn a_command_with_no_program_or_none_on_the_path_fails() {
        let cwd = TempDir::new();
        assert!(matches!(
            check(vec!["A=1".to_owned()], &cwd),
            Checked::Failed(_)
        ));
        assert_eq!(
            check(vec!["no-such-agent-of-cox".to_owned()], &cwd),
            Checked::NotFound("no-such-agent-of-cox".to_owned())
        );
    }
}
