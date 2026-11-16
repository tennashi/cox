//! The pull requests the daemon's chats' agents opened, read now and
//! then with `gh` (see [`crate::github`]), for the window to show each
//! chat's, and to tell the human, and the agent that opened the chat,
//! once CI ends.

use std::collections::HashMap;
use std::time::{Duration, Instant};

use tokio::sync::mpsc::UnboundedSender;

use super::{Daemon, FromDaemon, Internal, Whose};
use crate::daemon::wire::PullRow;
use crate::github::{self, CheckState, Problem, Pull, Status};
use crate::workflow;

/// How many of a chat's pull requests, the last it opened, Cox reads: the
/// ones it waits on, and not those of long ago.
const LAST: usize = 5;

/// How long after a read Cox reads a pull request again: soon while its
/// CI runs, less often while it waits on review, and after a while once
/// `gh` could not read it.
const WHILE_CI_RUNS: Duration = Duration::from_secs(20);
const WHILE_OPEN: Duration = Duration::from_mins(2);
const AFTER_A_PROBLEM: Duration = Duration::from_mins(5);

/// How often coxd looks which pull requests are due to be read.
pub(super) const LOOK_EVERY: Duration = Duration::from_secs(10);

/// What the daemon knows of a pull request.
#[derive(Default)]
pub(super) struct Watch {
    /// How it stood when last read, once it was.
    status: Option<Status>,
    /// When it is due to be read again; `None` once it is merged or
    /// closed.
    next: Option<Instant>,
    /// Whether `gh` reads it now.
    reading: bool,
}

/// What a chat has of the pull requests its agent opened.
#[derive(Default)]
pub(super) struct Pulls {
    /// The last its agent opened, the first opened first.
    opened: Vec<Pull>,
    /// How CI ended on each whose CI ended while the human did not look at
    /// the chat.
    unseen: Vec<(Pull, CheckState)>,
    /// What the agent that opened the chat is to hear with its next
    /// report, of CI that ended.
    pub(super) told: Vec<String>,
}

impl Pulls {
    /// The human looked at the chat.
    pub(super) fn seen(&mut self) {
        self.unseen.clear();
    }
}

/// Has the daemon look, now and then, which pull requests are due to be
/// read, as `internal` tells it.
pub(super) async fn look(internal: UnboundedSender<Internal>) {
    loop {
        tokio::time::sleep(LOOK_EVERY).await;
        if internal.send(Internal::Pulls).is_err() {
            return;
        }
    }
}

impl Daemon {
    /// Has `gh` read the pull requests that are due at `now` (see
    /// [`Self::due_pulls`]).
    pub(super) fn look_at_pulls(&mut self, now: Instant) {
        for pull in self.due_pulls(now) {
            let internal = self.internal.clone();
            tokio::task::spawn_local(async move {
                let read = github::fetch(&pull).await;
                let _ = internal.send(Internal::Pulled { pull, read });
            });
        }
    }

    /// Finds the pull requests of each chat whose conversation is read,
    /// and answers those due to be read at `now`, marked as read from now
    /// on: those never read, and those whose time came, unless merged or
    /// closed.
    pub(super) fn due_pulls(&mut self, now: Instant) -> Vec<Pull> {
        for slot in &mut self.chats {
            let Some(chat) = &slot.chat else {
                continue;
            };
            let opened = github::opened(chat.conversation().entries());
            let last = opened.len().saturating_sub(LAST);
            opened[last..].clone_into(&mut slot.pulls.opened);
        }
        let mut due: Vec<Pull> = Vec::new();
        for pull in self.chats.iter().flat_map(|slot| &slot.pulls.opened) {
            let watch = self.watches.entry(pull.clone()).or_insert_with(|| Watch {
                next: Some(now),
                ..Watch::default()
            });
            if !watch.reading && watch.next.is_some_and(|next| next <= now) {
                watch.reading = true;
                due.push(pull.clone());
            }
        }
        due
    }

    /// Takes how `gh` read `pull` at `now`: once its CI ended since it was
    /// last read, the human hears so, unless they look at the chat, and
    /// the agent that opened the chat with its next report.
    pub(super) fn pulled(&mut self, pull: &Pull, read: Result<Status, Problem>, now: Instant) {
        let (status, next) = match read {
            Ok(status) => {
                self.github = None;
                let next = if status.done() {
                    None
                } else if status.checks() == Some(CheckState::Pending) {
                    Some(now + WHILE_CI_RUNS)
                } else {
                    Some(now + WHILE_OPEN)
                };
                (Some(status), next)
            }
            Err(problem) => {
                tracing::warn!(
                    pull = pull.url(),
                    "gh does not read the pull request: {problem:?}"
                );
                self.github = problem.for_all().map(str::to_owned);
                (None, Some(now + AFTER_A_PROBLEM))
            }
        };
        let watch = self.watches.entry(pull.clone()).or_default();
        watch.reading = false;
        watch.next = next;
        let Some(status) = status else {
            return;
        };
        let ended = status.ended_since(watch.status.as_ref());
        let title = status.title.clone();
        watch.status = Some(status);
        if let Some(ended) = ended {
            self.ci_ended(pull, &title, ended);
        }
    }

    /// Tells of the CI of `pull`, titled `title`, which ended as `ended`:
    /// the human, of each chat that opened it, unless they look at it, and
    /// the agent that opened a chat that is a subagent's.
    fn ci_ended(&mut self, pull: &Pull, title: &str, ended: CheckState) {
        let outcome = match ended {
            CheckState::Passed => "passed",
            CheckState::Failed => "failed",
            CheckState::Pending | CheckState::Skipped => "ended",
        };
        let failed: Vec<String> = self.watches[pull]
            .status
            .iter()
            .flat_map(|status| &status.checks)
            .filter(|check| check.state == CheckState::Failed)
            .map(|check| check.name.clone())
            .collect();
        let shown = self.shown;
        let mut news = Vec::new();
        for slot in self.chats.iter_mut().chain(&mut self.archived) {
            if !slot.pulls.opened.contains(pull) {
                continue;
            }
            tracing::info!(pull = pull.url(), outcome, "CI ended");
            if shown != Some(slot.key) {
                slot.pulls.unseen.retain(|(unseen, _)| unseen != pull);
                slot.pulls.unseen.push((pull.clone(), ended));
            }
            news.push(FromDaemon::News {
                key: slot.key,
                name: slot.name.clone(),
                body: format!("CI {outcome} on #{} {title}", pull.number),
            });
            if let Whose::Agent { report, .. } = &mut slot.whose {
                let failed = if failed.is_empty() {
                    String::new()
                } else {
                    format!(" Failed: {}.", failed.join(", "))
                };
                slot.pulls.told.push(workflow::SUBAGENT_CI.say(&[
                    ("url", &pull.url()),
                    ("title", title),
                    ("outcome", outcome),
                    ("failed", &failed),
                ]));
                report.ci_ended();
            }
        }
        for message in news {
            self.tell_news(message);
        }
    }

    /// The pull requests of the chat whose pull requests `pulls` holds, as
    /// the window shows them.
    pub(super) fn pull_rows(&self, pulls: &Pulls) -> Vec<PullRow> {
        pulls
            .opened
            .iter()
            .map(|pull| PullRow {
                pull: pull.clone(),
                status: self
                    .watches
                    .get(pull)
                    .and_then(|watch| watch.status.clone()),
                unseen: pulls
                    .unseen
                    .iter()
                    .find(|(unseen, _)| unseen == pull)
                    .map(|&(_, ended)| ended),
            })
            .collect()
    }
}

/// What the daemon knows of each pull request, by its address.
pub(super) type Watches = HashMap<Pull, Watch>;
