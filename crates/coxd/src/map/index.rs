//! Making the index of one commit's code with one indexer: in a copy of
//! the code as the commit has it, which `git archive` writes, so that
//! what is not committed yet, and what changes while the indexer runs,
//! stays out of it, and the same commit always gives the same index.
//! The text of each file goes into the index, from the copy, for the map
//! to be drawn from the index alone.

use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::time::Duration;

use cox_editor::map::generate::Sources as _;
use cox_editor::scip::Index;
use prost::Message as _;

use super::indexers::{self, Indexer};
use crate::process::Stop;

/// How long an indexer has, past which it is stopped, and the index
/// fails.
const DEADLINE: Duration = Duration::from_mins(30);

/// How many of the last lines an indexer wrote say why it failed.
const LAST_LINES: usize = 12;

/// The code of a commit, or of a folder of it, in a repository.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Code {
    /// Where the repository is checked out.
    pub checkout: PathBuf,
    pub commit: String,
    /// The folder of the commit's code, from the repository's top, as
    /// `web/`, or empty for all of it.
    pub within: String,
}

impl Code {
    /// The code `folder` has checked out: of the commit at its head, and
    /// of the folder within the repository's top, if it is not the top.
    ///
    /// # Errors
    ///
    /// When `folder` is in no repository, or its head is no commit yet.
    pub fn checked_out(folder: &Path) -> Result<Self, String> {
        let commit = cox_git::git::run(folder, &["rev-parse", "--verify", "HEAD^{commit}"])?;
        let within = cox_git::git::run(folder, &["rev-parse", "--show-prefix"])?;
        Ok(Self {
            checkout: folder.to_owned(),
            commit,
            within,
        })
    }

    /// The commit's code, or its folder's, as Git names it: a tree.
    fn tree(&self) -> String {
        if self.within.is_empty() {
            self.commit.clone()
        } else {
            format!("{}:{}", self.commit, self.within)
        }
    }

    /// The names of the files and folders at the top of the code.
    ///
    /// # Errors
    ///
    /// When Git cannot list them.
    pub fn top(&self) -> Result<Vec<String>, String> {
        let listed = cox_git::git::run(&self.checkout, &["ls-tree", "--name-only", &self.tree()])?;
        Ok(listed.lines().map(str::to_owned).collect())
    }

    /// The commits before this one, the last first, as many as `most`.
    #[must_use]
    pub fn earlier(&self, most: usize) -> Vec<String> {
        let most = format!("--max-count={most}");
        cox_git::git::run(
            &self.checkout,
            &[
                "rev-list",
                "--first-parent",
                &most,
                &format!("{}^", self.commit),
            ],
        )
        .map_or_default(|listed| listed.lines().map(str::to_owned).collect())
    }
}

/// The index of `code` that `indexer` writes, running `command`, with
/// each file's text in it: with what it borrows from the checkout, unless
/// `without`. It is made in `work`, a folder of its own, which no other
/// index is made in meanwhile, and goes once it is made; and stopped as
/// `stop` says.
///
/// # Errors
///
/// When the code cannot be copied, or the indexer fails or is stopped, or
/// writes no index that reads: why.
pub fn make(
    code: &Code,
    indexer: &Indexer,
    without: bool,
    command: &[String],
    work: &Path,
    stop: &Stop,
) -> Result<Index, String> {
    let copy = work.join("code");
    std::fs::create_dir_all(&copy).map_err(|error| format!("{}: {error}", copy.display()))?;
    let made = copied(code, &copy).and_then(|()| {
        if !without {
            borrow(code, indexer, &copy);
        }
        indexed(code, indexer, command, (work, &copy), stop)
    });
    let _ = std::fs::remove_dir_all(work);
    made
}

/// Writes the code of `code` into `copy`, as `git archive` gives it.
fn copied(code: &Code, copy: &Path) -> Result<(), String> {
    let mut git = cox_git::git::command()
        .arg("-C")
        .arg(&code.checkout)
        .args(["archive", "--format=tar", &code.tree()])
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .map_err(|error| format!("Git does not start: {error}"))?;
    let stdout = git.stdout.take().ok_or("Git gave no output")?;
    let unpacked = tar::Archive::new(stdout).unpack(copy);
    let output = git
        .wait_with_output()
        .map_err(|error| format!("Git: {error}"))?;
    if !output.status.success() {
        return Err(format!(
            "git archive {}: {}",
            code.tree(),
            String::from_utf8_lossy(&output.stderr).trim()
        ));
    }
    unpacked.map_err(|error| format!("the code of {} does not unpack: {error}", code.tree()))
}

/// Links into `copy` what `indexer` borrows from the checkout.
fn borrow(code: &Code, indexer: &Indexer, copy: &Path) {
    if let Some((borrows, _)) = &indexer.borrows {
        let linked = std::os::unix::fs::symlink(code.checkout.join(borrows), copy.join(borrows));
        if let Err(error) = linked {
            tracing::warn!("{borrows} is not linked for {}: {error}", indexer.name);
        }
    }
}

/// Runs `indexer` with `command` in `copy`, writing in `work`, until
/// `stop` stops it, and answers the index it wrote, with each file's text
/// from `copy`.
fn indexed(
    code: &Code,
    indexer: &Indexer,
    command: &[String],
    (work, copy): (&Path, &Path),
    stop: &Stop,
) -> Result<Index, String> {
    let (program, before) = command
        .split_first()
        .ok_or_else(|| format!("{} has no program", indexer.name))?;
    let out = work.join("index.scip");
    let config = work.join("config.json");
    if let Some(text) = &indexer.config {
        std::fs::write(&config, text).map_err(|error| format!("{}: {error}", config.display()))?;
    }
    let name = if indexer.args.iter().any(|arg| arg.contains("{name}")) {
        indexers::project_name(copy, &code.checkout)
    } else {
        String::new()
    };
    let args = indexer.args.iter().map(|arg| {
        arg.replace("{out}", &out.to_string_lossy())
            .replace("{config}", &config.to_string_lossy())
            .replace("{name}", &name)
    });
    let mut run = Command::new(program);
    run.args(before).args(args).current_dir(copy);
    tracing::info!(indexer = %indexer.name, commit = %code.commit, "indexing {}", code.checkout.display());
    let ran = crate::process::run_stoppable(&mut run, DEADLINE, stop)
        .map_err(|error| format!("{program} does not start: {error}"))?;
    if stop.stopped() {
        return Err(format!("{} was stopped", indexer.name));
    }
    if ran.status.is_none() {
        return Err(format!(
            "{} took longer than {} minutes, and was stopped",
            indexer.name,
            DEADLINE.as_secs() / 60
        ));
    }
    if !ran.success() {
        return Err(format!(
            "{} failed ({}):\n{}",
            indexer.name,
            ran.status
                .map_or_else(String::new, |status| status.to_string()),
            last_lines(&ran.stderr, &ran.stdout)
        ));
    }
    let bytes =
        std::fs::read(&out).map_err(|error| format!("{} wrote no index: {error}", indexer.name))?;
    let mut index = Index::decode(bytes.as_slice())
        .map_err(|error| format!("the index {} wrote does not read: {error}", indexer.name))?;
    for document in &mut index.documents {
        if document.text.is_empty()
            && let Some(text) = copy.read(&document.relative_path)
        {
            document.text = text;
        }
    }
    Ok(index)
}

/// The last lines a program wrote on its standard error, or else on its
/// standard output.
fn last_lines(stderr: &[u8], stdout: &[u8]) -> String {
    let said = if stderr.iter().any(|byte| !byte.is_ascii_whitespace()) {
        stderr
    } else {
        stdout
    };
    let said = String::from_utf8_lossy(said);
    let lines: Vec<&str> = said.trim_end().lines().collect();
    lines[lines.len().saturating_sub(LAST_LINES)..].join("\n")
}
