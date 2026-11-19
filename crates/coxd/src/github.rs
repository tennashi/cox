//! The pull requests the chats' agents open on GitHub, and how their
//! checks stand, for the human to see a chat that waits on CI, and to
//! hear once CI ends.
//!
//! A chat's pull requests are those its agent opened with `gh pr create`,
//! as the address `gh` prints on a line of its own (see [`opened`]). How
//! one stands is read with `gh pr view`, as the human is logged in to
//! `gh` already: Cox asks for no token and keeps none (see [`fetch`]).

use std::time::Duration;

use cox_core::entry::{Content, Entry, ToolCallResult};
use serde::{Deserialize, Serialize};

/// A pull request, as its address names it.
#[derive(Clone, Debug, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
pub struct Pull {
    /// The repository, as `owner/name`.
    pub repo: String,
    pub number: u64,
}

impl Pull {
    /// The pull request `line` is the address of, as `gh pr create`
    /// prints it, alone on the line.
    #[must_use]
    pub fn of_line(line: &str) -> Option<Self> {
        let rest = line.trim().strip_prefix("https://github.com/")?;
        let mut parts = rest.split('/');
        let (owner, name, pull, number) =
            (parts.next()?, parts.next()?, parts.next()?, parts.next()?);
        let named = |part: &str| {
            !part.is_empty()
                && part
                    .chars()
                    .all(|c| c.is_ascii_alphanumeric() || matches!(c, '-' | '_' | '.'))
        };
        if parts.next().is_some() || pull != "pull" || !named(owner) || !named(name) {
            return None;
        }
        Some(Self {
            repo: format!("{owner}/{name}"),
            number: number.parse().ok()?,
        })
    }

    /// Its address.
    #[must_use]
    pub fn url(&self) -> String {
        format!("https://github.com/{}/pull/{}", self.repo, self.number)
    }
}

/// The pull requests the agent opened in `entries`, each once, in the
/// order it opened them: those whose address a command it ran with `gh pr
/// create` printed, once the command completed.
#[must_use]
pub fn opened(entries: &[Entry]) -> Vec<Pull> {
    let mut opened = Vec::new();
    for entry in entries {
        let Entry::ToolCall {
            title,
            content,
            result: Some(ToolCallResult::Completed),
            ..
        } = entry
        else {
            continue;
        };
        if !runs_pr_create(title) {
            continue;
        }
        for text in content.iter().filter_map(|content| match content {
            Content::Text(text) => Some(text),
            Content::Diff { .. } => None,
        }) {
            for pull in text.lines().filter_map(Pull::of_line) {
                if !opened.contains(&pull) {
                    opened.push(pull);
                }
            }
        }
    }
    opened
}

/// Whether the command `command` runs `gh pr create`, rather than only
/// naming it, as a search for it does.
fn runs_pr_create(command: &str) -> bool {
    command.match_indices("gh pr create").any(|(at, _)| {
        command[..at]
            .chars()
            .next_back()
            .is_none_or(|before| before.is_whitespace() || matches!(before, ';' | '&' | '|' | '('))
    })
}

/// How a pull request stands, as GitHub tells it.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum PullState {
    Open,
    Draft,
    Merged,
    Closed,
}

/// How a check, or all of a pull request's checks, stand.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum CheckState {
    /// It waits to run, or runs.
    Pending,
    Passed,
    Failed,
    /// It was skipped, or ended neither passing nor failing.
    Skipped,
}

/// A check of a pull request, as a job of its CI.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Check {
    pub name: String,
    pub state: CheckState,
    /// Where it is shown, as its job's page.
    pub link: Option<String>,
    /// When it ended, in milliseconds since the epoch, once it did, as
    /// GitHub tells of a job.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub ended: Option<i64>,
}

/// How a pull request stands, and its checks.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Status {
    pub title: String,
    pub state: PullState,
    /// The commit at its head, whose checks these are.
    pub head: String,
    pub checks: Vec<Check>,
    /// When it was opened, merged and closed, as GitHub tells it.
    #[serde(default)]
    pub times: Times,
}

/// When a pull request was opened, merged and closed, each in
/// milliseconds since the epoch, once it was, as GitHub tells it.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct Times {
    pub opened: Option<i64>,
    pub merged: Option<i64>,
    pub closed: Option<i64>,
}

/// The fields of `gh pr view --json` that [`Status::parse`] reads.
const FIELDS: &str = "title,state,isDraft,headRefOid,statusCheckRollup,createdAt,mergedAt,closedAt";

/// `at`, a time as GitHub tells it, as `2026-11-16T14:19:00Z`, in
/// milliseconds since the epoch; none for none, or for one it cannot read.
fn millis(at: Option<&str>) -> Option<i64> {
    at.filter(|at| !at.is_empty())?
        .parse::<jiff::Timestamp>()
        .ok()
        .map(jiff::Timestamp::as_millisecond)
        // What gh tells of a job not ended, as the year 1.
        .filter(|at| *at > 0)
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct Viewed {
    title: String,
    state: String,
    #[serde(default)]
    is_draft: bool,
    #[serde(default)]
    head_ref_oid: String,
    #[serde(default)]
    status_check_rollup: Vec<Rolled>,
    created_at: Option<String>,
    merged_at: Option<String>,
    closed_at: Option<String>,
}

/// A check as `gh` tells it: a check run, of GitHub Actions or another
/// app, or a commit status, which says only its `state`.
#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct Rolled {
    name: Option<String>,
    context: Option<String>,
    status: Option<String>,
    conclusion: Option<String>,
    state: Option<String>,
    details_url: Option<String>,
    target_url: Option<String>,
    completed_at: Option<String>,
}

impl Rolled {
    fn check(self) -> Check {
        let state = match (
            self.status.as_deref(),
            self.conclusion.as_deref(),
            self.state.as_deref(),
        ) {
            (Some("COMPLETED"), Some("SUCCESS"), _) | (None, _, Some("SUCCESS")) => {
                CheckState::Passed
            }
            (Some("COMPLETED"), Some("NEUTRAL" | "SKIPPED" | "STALE"), _) => CheckState::Skipped,
            (Some("COMPLETED"), _, _) | (None, _, Some("FAILURE" | "ERROR")) => CheckState::Failed,
            _ => CheckState::Pending,
        };
        Check {
            name: self.name.or(self.context).unwrap_or_default(),
            state,
            link: self
                .details_url
                .or(self.target_url)
                .filter(|link| !link.is_empty()),
            ended: millis(self.completed_at.as_deref()),
        }
    }
}

impl Status {
    /// The status `gh pr view --json` told as `json`, with [`FIELDS`].
    ///
    /// # Errors
    ///
    /// When `json` is not what `gh` tells.
    pub fn parse(json: &str) -> Result<Self, String> {
        let viewed: Viewed = serde_json::from_str(json).map_err(|error| error.to_string())?;
        let state = match viewed.state.as_str() {
            "MERGED" => PullState::Merged,
            "CLOSED" => PullState::Closed,
            _ if viewed.is_draft => PullState::Draft,
            _ => PullState::Open,
        };
        Ok(Self {
            title: viewed.title,
            state,
            head: viewed.head_ref_oid,
            checks: viewed
                .status_check_rollup
                .into_iter()
                .map(Rolled::check)
                .collect(),
            times: Times {
                opened: millis(viewed.created_at.as_deref()),
                merged: millis(viewed.merged_at.as_deref()),
                closed: millis(viewed.closed_at.as_deref()),
            },
        })
    }

    /// When its checks ended, together: when the last of them ended, once
    /// none waits or runs, if GitHub told when each ended.
    #[must_use]
    pub fn checks_ended(&self) -> Option<i64> {
        if matches!(self.checks(), None | Some(CheckState::Pending)) {
            return None;
        }
        self.checks.iter().filter_map(|check| check.ended).max()
    }

    /// How its checks stand, together: pending while any is, then failed
    /// if any failed; `None` while it has none.
    #[must_use]
    pub fn checks(&self) -> Option<CheckState> {
        let any = |state| self.checks.iter().any(|check| check.state == state);
        if self.checks.is_empty() {
            None
        } else if any(CheckState::Pending) {
            Some(CheckState::Pending)
        } else if any(CheckState::Failed) {
            Some(CheckState::Failed)
        } else if any(CheckState::Passed) {
            Some(CheckState::Passed)
        } else {
            Some(CheckState::Skipped)
        }
    }

    /// How its checks ended, if they ended since it stood as `was`: they
    /// were pending at the same head, or the head moved and its checks ran
    /// in between. A pull request first seen tells nothing: Cox did not
    /// see its checks run.
    #[must_use]
    pub fn ended_since(&self, was: Option<&Self>) -> Option<CheckState> {
        let was = was?;
        let now = self.checks().filter(|&now| now != CheckState::Pending)?;
        let ran = was.head != self.head || was.checks() == Some(CheckState::Pending);
        ran.then_some(now)
    }

    /// Whether it will not change any more: merged, or closed.
    #[must_use]
    pub fn done(&self) -> bool {
        matches!(self.state, PullState::Merged | PullState::Closed)
    }
}

/// Why `gh` could not tell how a pull request stands.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Problem {
    /// `gh` is not on the `PATH` Cox runs with: no pull request can be read.
    Missing,
    /// The human is not logged in to `gh`: no pull request can be read.
    LoggedOut,
    /// This one pull request could not be read, as `gh` said.
    Failed(String),
}

impl Problem {
    /// What the human is told of it, when it holds for every pull
    /// request.
    #[must_use]
    pub fn for_all(&self) -> Option<&'static str> {
        match self {
            Self::Missing => Some(
                "Cox shows no pull requests: the GitHub CLI, gh, is not found on the PATH it runs with.",
            ),
            Self::LoggedOut => Some(
                "Cox shows no pull requests: gh is not logged in to GitHub. Run gh auth login in a terminal.",
            ),
            Self::Failed(_) => None,
        }
    }
}

/// How long `gh` has to answer.
const PATIENCE: Duration = Duration::from_secs(30);

/// How `pull` stands now, as `gh` tells it.
///
/// # Errors
///
/// When `gh` is missing, is not logged in, or cannot read it.
pub async fn fetch(pull: &Pull) -> Result<Status, Problem> {
    let mut command = tokio::process::Command::new("gh");
    command
        .args(["pr", "view", &pull.url(), "--json", FIELDS])
        .stdin(std::process::Stdio::null())
        .kill_on_drop(true);
    let output = match tokio::time::timeout(PATIENCE, command.output()).await {
        Err(_) => return Err(Problem::Failed("gh did not answer in time".to_owned())),
        Ok(Err(error)) if error.kind() == std::io::ErrorKind::NotFound => {
            return Err(Problem::Missing);
        }
        Ok(Err(error)) => return Err(Problem::Failed(error.to_string())),
        Ok(Ok(output)) => output,
    };
    if !output.status.success() {
        let said = String::from_utf8_lossy(&output.stderr);
        if said.contains("gh auth login") {
            return Err(Problem::LoggedOut);
        }
        return Err(Problem::Failed(
            said.lines().next().unwrap_or_default().to_owned(),
        ));
    }
    Status::parse(&String::from_utf8_lossy(&output.stdout)).map_err(Problem::Failed)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn command(title: &str, output: &str, result: Option<ToolCallResult>) -> Entry {
        Entry::ToolCall {
            id: title.to_owned(),
            title: title.to_owned(),
            command: true,
            about: None,
            content: vec![Content::Text(output.to_owned())],
            result,
        }
    }

    fn pull(number: u64) -> Pull {
        Pull {
            repo: "tennashi/cox".to_owned(),
            number,
        }
    }

    #[test]
    fn a_chat_s_pull_requests_are_those_its_gh_pr_create_printed() {
        let entries = [
            command(
                "cd /w && git push -u origin b && gh pr create --draft --title t --body-file b.md",
                "```console\nremote: \nhttps://github.com/tennashi/cox/pull/423\n```",
                Some(ToolCallResult::Completed),
            ),
            command(
                "cat > /tmp/b.md <<'EOF'\n## Why\nEOF\ngh pr create -F /tmp/b.md",
                "https://github.com/tennashi/cox/pull/424\n",
                Some(ToolCallResult::Completed),
            ),
            command(
                "gh pr create --fill",
                "https://github.com/tennashi/cox/pull/423",
                Some(ToolCallResult::Completed),
            ),
        ];
        assert_eq!(opened(&entries), [pull(423), pull(424)]);
    }

    #[test]
    fn a_pull_request_only_named_or_not_created_is_not_the_chat_s() {
        let entries = [
            command(
                "gh pr view 421 --json url",
                "https://github.com/tennashi/cox/pull/421",
                Some(ToolCallResult::Completed),
            ),
            command(
                "rg -n '%gh pr create%' logs",
                "https://github.com/tennashi/cox/pull/420",
                Some(ToolCallResult::Completed),
            ),
            command(
                "gh pr create --fill",
                "https://github.com/tennashi/cox/pull/419",
                Some(ToolCallResult::Failed),
            ),
            command(
                "gh pr create --fill",
                "See https://github.com/tennashi/cox/pull/418 and https://github.com/tennashi/cox/pull/new/b",
                Some(ToolCallResult::Completed),
            ),
            Entry::Reply("Opened https://github.com/tennashi/cox/pull/417".to_owned()),
        ];
        assert_eq!(opened(&entries), []);
    }

    #[test]
    fn a_pull_request_with_failed_ci_reads_failed_with_each_job() {
        let status = Status::parse(include_str!("github/failed.json")).unwrap();
        assert_eq!(status.state, PullState::Draft);
        assert_eq!(status.checks(), Some(CheckState::Failed));
        assert_eq!(
            status
                .checks
                .iter()
                .map(|check| (check.name.as_str(), check.state))
                .collect::<Vec<_>>(),
            [
                ("ci (ubuntu-latest)", CheckState::Failed),
                ("ci (macos-latest)", CheckState::Failed),
            ]
        );
        assert!(
            status.checks[0].link.as_deref().is_some_and(
                |link| link.starts_with("https://github.com/tennashi/cox/actions/runs/")
            )
        );
    }

    #[test]
    fn a_merged_pull_request_whose_ci_passed_is_done() {
        let status = Status::parse(include_str!("github/merged.json")).unwrap();
        assert_eq!(status.state, PullState::Merged);
        assert_eq!(status.checks(), Some(CheckState::Passed));
        assert!(status.done());
    }

    #[test]
    fn tells_when_a_pull_request_opened_merged_closed_and_its_ci_ended() {
        let at = |text: &str| text.parse::<jiff::Timestamp>().unwrap().as_millisecond();
        let status = Status::parse(include_str!("github/merged.json")).unwrap();
        assert_eq!(
            status.times,
            Times {
                opened: Some(at("2026-11-16T13:42:54Z")),
                merged: Some(at("2026-11-16T14:19:00Z")),
                closed: Some(at("2026-11-16T14:19:00Z")),
            }
        );
        // The last of its two jobs.
        assert_eq!(status.checks_ended(), Some(at("2026-11-16T14:17:14Z")));

        // Open, its CI still running: not merged, not closed, not ended.
        let json = r#"{"title":"t","state":"OPEN","headRefOid":"a","createdAt":"2026-11-16T13:42:54Z",
            "mergedAt":null,"closedAt":null,"statusCheckRollup":[
              {"name":"a","status":"COMPLETED","conclusion":"SUCCESS","completedAt":"2026-11-16T14:00:00Z"},
              {"name":"b","status":"IN_PROGRESS","conclusion":"","completedAt":"0001-01-01T00:00:00Z"}]}"#;
        let status = Status::parse(json).unwrap();
        assert_eq!((status.times.merged, status.times.closed), (None, None));
        assert_eq!(status.checks_ended(), None);
    }

    #[test]
    fn a_draft_with_a_running_job_and_a_commit_status_reads_pending() {
        let json = r#"{"title":"t","state":"OPEN","isDraft":true,"headRefOid":"a",
            "statusCheckRollup":[
              {"__typename":"CheckRun","name":"ci","status":"IN_PROGRESS","conclusion":"","detailsUrl":"https://x"},
              {"__typename":"StatusContext","context":"deploy","state":"SUCCESS","targetUrl":""}]}"#;
        let status = Status::parse(json).unwrap();
        assert_eq!(status.state, PullState::Draft);
        assert_eq!(status.checks(), Some(CheckState::Pending));
        assert_eq!(status.checks[1].name, "deploy");
        assert_eq!(status.checks[1].state, CheckState::Passed);
        assert_eq!(status.checks[1].link, None);
    }

    fn at(head: &str, states: &[CheckState]) -> Status {
        Status {
            title: "t".to_owned(),
            state: PullState::Open,
            head: head.to_owned(),
            checks: states
                .iter()
                .map(|&state| Check {
                    name: "ci".to_owned(),
                    state,
                    link: None,
                    ended: None,
                })
                .collect(),
            times: Times::default(),
        }
    }

    #[test]
    fn ci_ends_once_for_each_run_cox_saw() {
        use CheckState::{Failed, Passed, Pending};
        let running = at("a", &[Pending, Passed]);
        let failed = at("a", &[Failed, Passed]);
        assert_eq!(running.ended_since(None), None, "first seen running");
        assert_eq!(failed.ended_since(None), None, "first seen ended");
        assert_eq!(failed.ended_since(Some(&running)), Some(Failed));
        assert_eq!(failed.ended_since(Some(&failed)), None, "told once");
        let pushed = at("b", &[Passed]);
        assert_eq!(
            pushed.ended_since(Some(&failed)),
            Some(Passed),
            "ran between looks"
        );
        assert_eq!(at("b", &[]).ended_since(Some(&running)), None, "no checks");
    }
}
