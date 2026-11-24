//! The indexes being made, and those that failed: one at most being made
//! of each checkout's code by each indexer, as its head moves on, a newer
//! commit's index stopping an older one's, which is no longer wanted,
//! rather than running beside it.

use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::atomic::{AtomicU64, Ordering};

use super::cache::Key;
use crate::process::Stop;

/// The code of a checkout that one index at most is made of at a time by
/// an indexer: the checkout, the folder of the code within the
/// repository, and the indexer.
pub type Slot = (PathBuf, String, String);

/// How an index that is not kept stands.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Standing {
    /// Being made, since then, in seconds since the epoch, by the job of
    /// that number.
    Indexing {
        since: u64,
        job: u64,
    },
    Failed {
        reason: String,
    },
}

/// A job that makes an index.
#[derive(Clone, Debug)]
struct Running {
    key: Key,
    job: u64,
    stop: Stop,
}

/// The indexes being made, and those that failed.
#[derive(Debug, Default)]
pub struct Jobs {
    /// How each index that is not kept stands, by what it is kept by.
    standing: HashMap<Key, Standing>,
    /// The job that makes an index of each slot's code.
    running: HashMap<Slot, Running>,
}

/// The number of the job started last, of every [`Jobs`] of the process,
/// which names the folder each makes its index in.
static LAST: AtomicU64 = AtomicU64::new(0);

impl Jobs {
    /// How the index of `key` stands, if it is being made or failed.
    pub fn standing(&self, key: &Key) -> Option<&Standing> {
        self.standing.get(key)
    }

    /// Starts a job, since `since`, that makes the index of `key` of
    /// `slot`'s code, stopping the one that makes another of it, which is
    /// forgotten; and answers its number and what stops it.
    pub fn start(&mut self, slot: Slot, key: Key, since: u64) -> (u64, Stop) {
        if let Some(older) = self.running.remove(&slot) {
            older.stop.stop();
            if self.is_job(&older.key, older.job) {
                self.standing.remove(&older.key);
            }
        }
        let job = LAST.fetch_add(1, Ordering::Relaxed) + 1;
        let stop = Stop::default();
        self.standing
            .insert(key.clone(), Standing::Indexing { since, job });
        self.running.insert(
            slot,
            Running {
                key,
                job,
                stop: stop.clone(),
            },
        );
        (job, stop)
    }

    /// The job `job`, which made the index of `key` of `slot`'s code,
    /// ended so: what it says is kept, unless another job of the index
    /// started since, or it was stopped for another.
    pub fn end(&mut self, slot: &Slot, key: &Key, job: u64, ended: Result<(), String>) {
        if self
            .running
            .get(slot)
            .is_some_and(|running| running.job == job)
        {
            self.running.remove(slot);
        }
        if !self.is_job(key, job) {
            return;
        }
        match ended {
            Ok(()) => self.standing.remove(key),
            Err(reason) => self
                .standing
                .insert(key.clone(), Standing::Failed { reason }),
        };
    }

    /// Whether the index of `key` is being made by the job `job`.
    fn is_job(&self, key: &Key, job: u64) -> bool {
        matches!(self.standing.get(key), Some(Standing::Indexing { job: running, .. }) if *running == job)
    }

    /// How many jobs run.
    #[cfg(test)]
    pub fn running(&self) -> usize {
        self.running.len()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn key(commit: &str) -> Key {
        Key {
            commit: commit.to_owned(),
            within: String::new(),
            indexer: "fake".to_owned(),
            version: "1".to_owned(),
            without: false,
        }
    }

    fn slot(checkout: &str) -> Slot {
        (PathBuf::from(checkout), String::new(), "fake".to_owned())
    }

    #[test]
    fn a_newer_commits_index_stops_and_forgets_the_older_ones() {
        let mut jobs = Jobs::default();
        let (older, stops_older) = jobs.start(slot("/a"), key("1"), 0);
        let (_, stops_other) = jobs.start(slot("/b"), key("3"), 0);
        let (newer, stops_newer) = jobs.start(slot("/a"), key("2"), 0);
        assert!(stops_older.stopped());
        assert!(!stops_other.stopped(), "another checkout's runs on");
        assert!(!stops_newer.stopped());
        assert_eq!(jobs.running(), 2);
        // The older ends as it was stopped: nothing of it is kept.
        jobs.end(&slot("/a"), &key("1"), older, Err("stopped".to_owned()));
        assert_eq!(jobs.standing(&key("1")), None);
        assert!(matches!(
            jobs.standing(&key("2")),
            Some(Standing::Indexing { job, .. }) if *job == newer
        ));
        jobs.end(&slot("/a"), &key("2"), newer, Err("broke".to_owned()));
        assert_eq!(
            jobs.standing(&key("2")),
            Some(&Standing::Failed {
                reason: "broke".to_owned()
            })
        );
        assert_eq!(jobs.running(), 1);
    }
}
