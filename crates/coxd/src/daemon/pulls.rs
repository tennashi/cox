//! The pull requests the daemon's chats' agents opened, which each
//! chat's project holds, whichever chat opened it, kept in the store past
//! a restart, and read now and then with `gh` (see [`crate::github`]),
//! for the window to show each project's, and each chat's, and to tell
//! the human, and the chat that opened one while it is open, once CI
//! ends.
//!
//! A project holds too the human's open pull requests of its
//! repositories (see [`crate::repositories::of`]) that Cox finds on
//! GitHub, as `gh pr list --author @me` lists them, however the human
//! opened them: in Cox, by hand, or with another tool. Cox lists them
//! now and then, once for each repository, and holds each it does not
//! yet, with no chat that opened it, unless a chat is seen to open it
//! later. Not everyone's: a busy repository's would fill the Activity
//! page.
//!
//! A chat's work does not wait on its pull requests: one marked done
//! stays done as CI ends, which the Activity page shows. While it is
//! open, its agent hears how CI ended as before: in a chat of the
//! human's, in a line of its own; in a subagent's, the agent that opened
//! it, with its report, which it hears too once the subagent is done, as
//! long as its own chat is open.

use std::collections::{HashMap, HashSet};
use std::path::PathBuf;
use std::time::{Duration, Instant};

use tokio::sync::mpsc::UnboundedSender;

use cox_core::store::{ChatId, ProjectId};

use super::{Daemon, FromDaemon, Hears, Internal, Whose};
use crate::daemon::wire::{ProjectPull, PullRow};
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

/// How often Cox lists the human's open pull requests of each project's
/// repositories, as often as it reads one that waits on review.
pub(super) const LIST_EVERY: Duration = WHILE_OPEN;

/// How often coxd looks which pull requests are due to be read.
pub(super) const LOOK_EVERY: Duration = Duration::from_secs(10);

/// How long, in milliseconds, a pull request merged or closed is still
/// read from the store as coxd starts, for the window to show it went.
const ENDED_KEPT: i64 = 24 * 60 * 60 * 1000;

/// What the daemon knows of a pull request.
#[derive(Default)]
pub(super) struct Watch {
    /// The project that holds it; `None` for Cox's own.
    project: Option<ProjectId>,
    /// The chat whose agent opened it, as kept, if it is.
    author: Option<ChatId>,
    /// When coxd first saw it, in milliseconds since the epoch.
    added: i64,
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
    /// What is to be told of CI that ended: to the agent that opened the
    /// chat, with its next report, or, in a chat of the human's, to its own
    /// agent, in a line of its own once its turn is not running (see
    /// [`Daemon::tell_ci`]).
    pub(super) told: Vec<String>,
}

impl Pulls {
    /// The human looked at the chat.
    pub(super) fn seen(&mut self) {
        self.unseen.clear();
    }
}

/// When Cox lists the human's open pull requests of the projects'
/// repositories next, and of which projects it lists them now.
#[derive(Default)]
pub(super) struct Listing {
    /// When it is due next; `None` until it first listed them.
    next: Option<Instant>,
    /// The projects whose pull requests `gh` lists now.
    reading: HashSet<ProjectId>,
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
        for (project, folders) in self.due_listings(now) {
            let internal = self.internal.clone();
            tokio::task::spawn_local(async move {
                let repos = tokio::task::spawn_blocking(move || github::repos(&folders))
                    .await
                    .unwrap_or_default();
                let mut read = Vec::new();
                for repo in repos {
                    let listed = github::mine(&repo).await;
                    read.push((repo, listed));
                }
                let _ = internal.send(Internal::Listed { project, read });
            });
        }
        for pull in self.due_pulls(now) {
            let internal = self.internal.clone();
            tokio::task::spawn_local(async move {
                let read = github::fetch(&pull).await;
                let _ = internal.send(Internal::Pulled { pull, read });
            });
        }
    }

    /// Finds the pull requests of each chat whose conversation is read,
    /// for its project to hold those it does not yet, and answers those
    /// of every project due to be read at `now`, marked as read from now
    /// on: those never read, and those whose time came, unless merged or
    /// closed.
    pub(super) fn due_pulls(&mut self, now: Instant) -> Vec<Pull> {
        let mut found: Vec<(Pull, Option<ProjectId>, Option<ChatId>)> = Vec::new();
        for slot in &mut self.chats {
            let Some(chat) = &slot.chat else {
                continue;
            };
            let opened = github::opened(chat.conversation().entries());
            let last = opened.len().saturating_sub(LAST);
            opened[last..].clone_into(&mut slot.pulls.opened);
            let project = self
                .projects
                .get(slot.project)
                .and_then(|project| project.id);
            found.extend(
                slot.pulls
                    .opened
                    .iter()
                    .map(|pull| (pull.clone(), project, slot.kept)),
            );
        }
        let added = cox_core::store::now();
        for (pull, project, author) in found {
            if let Some(watch) = self.watches.get_mut(&pull) {
                // Found on GitHub before the chat was seen to open it.
                if let (None, Some(author)) = (watch.author, author) {
                    watch.project = project;
                    watch.author = Some(author);
                    if let Err(error) =
                        self.store
                            .keep_pull_author(project, &pull.repo, pull.number, author)
                    {
                        tracing::error!("the pull request's chat is not kept: {error:#}");
                    }
                }
                continue;
            }
            if let Err(error) =
                self.store
                    .keep_pull(project, &pull.repo, pull.number, author, added)
            {
                tracing::error!("the pull request is not kept: {error:#}");
            }
            self.watches.insert(
                pull,
                Watch {
                    project,
                    author,
                    added,
                    next: Some(now),
                    ..Watch::default()
                },
            );
        }
        let mut due: Vec<Pull> = self
            .watches
            .iter_mut()
            .filter(|(_, watch)| !watch.reading && watch.next.is_some_and(|next| next <= now))
            .map(|(pull, watch)| {
                watch.reading = true;
                pull.clone()
            })
            .collect();
        due.sort();
        due
    }

    /// The projects whose open pull requests are due to be listed at
    /// `now`, marked as listed from now on, each with the folders of its
    /// repositories: once each [`LIST_EVERY`], of each project the human
    /// has listed and not put away, unless they are listed still.
    pub(super) fn due_listings(&mut self, now: Instant) -> Vec<(ProjectId, Vec<PathBuf>)> {
        if self.listing.next.is_some_and(|next| next > now) {
            return Vec::new();
        }
        self.listing.next = Some(now + LIST_EVERY);
        let mut due = Vec::new();
        for project in &self.projects {
            let Some(id) = project.id else {
                continue;
            };
            if project.deleted || project.archived || self.listing.reading.contains(&id) {
                continue;
            }
            let folders: Vec<PathBuf> = crate::repositories::of(&project.folder)
                .into_iter()
                .map(|(_, folder)| folder)
                .collect();
            if folders.is_empty() {
                continue;
            }
            self.listing.reading.insert(id);
            due.push((id, folders));
        }
        due
    }

    /// Takes the open pull requests of the human's `gh` listed of each
    /// repository of `project`, as `read` says, at `now`: the project
    /// holds each that no project holds yet, with no chat that opened it,
    /// to be read from now. One that is merged or closed is read so, as
    /// any other, and listed no more.
    pub(super) fn listed(
        &mut self,
        project: ProjectId,
        read: Vec<(String, Result<Vec<Pull>, Problem>)>,
        now: Instant,
    ) {
        self.listing.reading.remove(&project);
        let added = cox_core::store::now();
        for (repo, listed) in read {
            let pulls = match listed {
                Ok(pulls) => pulls,
                Err(problem) => {
                    tracing::warn!(repo, "gh does not list the pull requests: {problem:?}");
                    if let Some(all) = problem.for_all() {
                        self.github = Some(all.to_owned());
                    }
                    continue;
                }
            };
            for pull in pulls {
                if self.watches.contains_key(&pull) {
                    continue;
                }
                if let Err(error) =
                    self.store
                        .keep_pull(Some(project), &pull.repo, pull.number, None, added)
                {
                    tracing::error!("the pull request is not kept: {error:#}");
                }
                self.watches.insert(
                    pull,
                    Watch {
                        project: Some(project),
                        added,
                        next: Some(now),
                        ..Watch::default()
                    },
                );
            }
        }
    }

    /// Reads the pull requests the projects hold, as kept, for coxd to
    /// read again those not merged or closed, from `now`.
    pub(super) fn read_pulls(&mut self, now: Instant) {
        let since = cox_core::store::now() - ENDED_KEPT;
        let kept = match self.store.pulls::<Status>(since) {
            Ok(kept) => kept,
            Err(error) => {
                tracing::error!("the pull requests kept cannot be read: {error:#}");
                return;
            }
        };
        for kept in kept {
            let next = (!kept.status.as_ref().is_some_and(Status::done)).then_some(now);
            self.watches.insert(
                Pull {
                    repo: kept.repo,
                    number: kept.number,
                },
                Watch {
                    project: kept.project,
                    author: kept.author,
                    added: kept.added,
                    status: kept.status,
                    next,
                    reading: false,
                },
            );
        }
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
        if watch.status.as_ref() != Some(&status) {
            let gone = status.done().then(|| {
                status
                    .times
                    .merged
                    .or(status.times.closed)
                    .unwrap_or_else(cox_core::store::now)
            });
            if let Err(error) = self
                .store
                .keep_pull_status(&pull.repo, pull.number, &status, gone)
            {
                tracing::error!("how the pull request stands is not kept: {error:#}");
            }
        }
        watch.status = Some(status);
        if let Some(ended) = ended {
            self.ci_ended(pull, &title, ended);
        }
    }

    /// Tells of the CI of `pull`, titled `title`, which ended as `ended`,
    /// the chat whose agent opened it, its author (see [`Self::author`]):
    /// the human, unless they look at it, and, while it is open, the
    /// agent that opened it, if a subagent's, or else its own agent. One
    /// marked done stays so: the agent that opened it, if its chat is
    /// open, hears with its report again; no other.
    fn ci_ended(&mut self, pull: &Pull, title: &str, ended: CheckState) {
        let outcome = match ended {
            CheckState::Passed => "passed",
            CheckState::Failed => "failed",
            CheckState::Pending | CheckState::Skipped => "ended",
        };
        let url = pull.url();
        tracing::info!(pull = url, outcome, "CI ended");
        let Some(key) = self.author(pull) else {
            return;
        };
        let failed: Vec<String> = self.watches[pull]
            .status
            .iter()
            .flat_map(|status| &status.checks)
            .filter(|check| check.state == CheckState::Failed)
            .map(|check| match &check.link {
                Some(link) => format!("{}, {link}", check.name),
                None => check.name.clone(),
            })
            .collect();
        let failed = if failed.is_empty() {
            String::new()
        } else {
            format!(" Failed: {}.", failed.join("; "))
        };
        let number = pull.number.to_string();
        let values = [
            ("url", url.as_str()),
            ("number", &number),
            ("title", title),
            ("outcome", outcome),
            ("failed", &failed),
        ];
        let opener_open = self.slot(key).is_some_and(|slot| match slot.whose {
            Whose::Agent { parent, .. } => self
                .slot(parent)
                .is_some_and(|parent| parent.resolved.is_none()),
            Whose::Human | Whose::Sketch { .. } => false,
        });
        let shown = self.shown;
        let Some(slot) = self.slot_mut(key) else {
            return;
        };
        let open = slot.resolved.is_none();
        if open && shown != Some(key) {
            slot.pulls.unseen.retain(|(unseen, _)| unseen != pull);
            slot.pulls.unseen.push((pull.clone(), ended));
        }
        let news = FromDaemon::News {
            key,
            name: slot.name.clone(),
            body: format!("CI {outcome} on #{} {title}", pull.number),
        };
        match &mut slot.whose {
            Whose::Agent { report, .. } if open || opener_open => {
                slot.pulls.told.push(workflow::SUBAGENT_CI.say(&values));
                report.ci_ended();
            }
            Whose::Human | Whose::Sketch { .. } if open => {
                slot.pulls.told.push(workflow::CI_ENDED.say(&values));
            }
            Whose::Agent { .. } | Whose::Human | Whose::Sketch { .. } => {}
        }
        self.tell_news(news);
    }

    /// The chat whose agent opened `pull`, by its key, while it is listed:
    /// the one kept as its author, or, for one opened by a chat not kept,
    /// the first whose conversation has it.
    fn author(&self, pull: &Pull) -> Option<u64> {
        let mut slots = self.chats.iter().chain(&self.archived);
        match self.watches.get(pull)?.author {
            Some(author) => slots.find(|slot| slot.kept == Some(author)),
            None => slots.find(|slot| slot.pulls.opened.contains(pull)),
        }
        .map(|slot| slot.key)
    }

    /// Tells the agent of each chat of the human's what it is to hear of
    /// CI that ended (see [`Pulls::told`]), in a line of its own, once its
    /// turn is not running, as [`Self::tell_reports`] tells reports; a
    /// chat that never hears, as one archived or closed for good, hears
    /// nothing of it. Answers whether it told any agent.
    pub(super) fn tell_ci(&mut self) -> bool {
        let keys: Vec<u64> = self
            .chats
            .iter()
            .chain(&self.archived)
            .filter(|slot| slot.whose.waits() && !slot.pulls.told.is_empty())
            .map(|slot| slot.key)
            .collect();
        let mut told = false;
        for key in keys {
            let line = match self.hears(key) {
                Hears::Later => continue,
                Hears::Now => self.slot(key).map(|slot| slot.pulls.told.join("\n\n")),
                Hears::Never => None,
            };
            if let Some(slot) = self.slot_mut(key) {
                slot.pulls.told.clear();
            }
            if let Some(line) = line {
                self.go_on(key, &line);
                told = true;
            }
        }
        told
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

impl Daemon {
    /// The pull requests the projects hold, as the window lists them, the
    /// first seen first: of each project listed and not deleted, with the
    /// chat that opened it, while it is listed.
    pub(super) fn project_pulls(&self) -> Vec<ProjectPull> {
        let mut pulls: Vec<(&Pull, &Watch)> = self.watches.iter().collect();
        pulls.sort_by(|(a, one), (b, other)| (one.added, *a).cmp(&(other.added, *b)));
        pulls
            .into_iter()
            .filter_map(|(pull, watch)| {
                let project = self
                    .projects
                    .iter()
                    .position(|project| project.id == watch.project && !project.deleted)?;
                Some(ProjectPull {
                    pull: pull.clone(),
                    project,
                    status: watch.status.clone(),
                    author: self.author(pull),
                })
            })
            .collect()
    }
}

/// What the daemon knows of each pull request, by its address.
pub(super) type Watches = HashMap<Pull, Watch>;
