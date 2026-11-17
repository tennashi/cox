//! The compactions of a session's context, as `claude-agent-acp` reports
//! them to a client of the `JetBrains` AIR extension, as Cox declares
//! itself: a "Compact conversation" tool call whose `_meta` holds
//! `jetbrains.air.contextCompaction`, empty as it starts, then, once it
//! ended, with what started it and how many tokens the context held
//! before and after. Cox shows it as what it is, a compaction, and not as
//! a tool call: the agent compacts a session left idle by itself, which is
//! no work the human is to hear of as work.

use std::time::Duration;

use super::Meta;

/// What started a compaction.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum CompactionTrigger {
    /// The agent compacted a session left idle by itself, shortly before
    /// its prompt cache expired, so the next prompt costs less.
    Idle,
    /// The context filled up in a turn.
    Automatic,
    /// The human asked for it, with `/compact`.
    Manual,
}

/// A compaction of the session's context that ended.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Compaction {
    /// What started it, when the agent told it.
    pub trigger: Option<CompactionTrigger>,
    /// The tokens in the context before it, when told.
    pub before: Option<u64>,
    /// The tokens in the context after it, when told.
    pub after: Option<u64>,
    /// How long it took, when told.
    pub took: Option<Duration>,
    /// Why it failed, when it did: the agent's error, or `Some("")` when
    /// it told none.
    pub failed: Option<String>,
}

/// What the agent told of a compaction in the `_meta` of one update of
/// its tool call: each a fact told then, or `None`.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub(crate) struct Told {
    trigger: Option<CompactionTrigger>,
    before: Option<u64>,
    after: Option<u64>,
    took: Option<Duration>,
}

impl Told {
    /// What `meta`, a tool call's, tells of a compaction, or `None` when
    /// it is not of one.
    pub(crate) fn of(meta: Option<&Meta>) -> Option<Self> {
        let facts = meta?
            .get("jetbrains")?
            .get("air")?
            .get("contextCompaction")?
            .as_object()?;
        let number = |key: &str| facts.get(key).and_then(serde_json::Value::as_u64);
        let trigger = match facts.get("trigger").and_then(serde_json::Value::as_str) {
            Some("idle") => Some(CompactionTrigger::Idle),
            Some("automatic") => Some(CompactionTrigger::Automatic),
            Some("manual") => Some(CompactionTrigger::Manual),
            _ => None,
        };
        Some(Self {
            trigger,
            before: number("preTokens"),
            after: number("postTokens"),
            took: number("durationMs").map(Duration::from_millis),
        })
    }
}

/// A compaction the agent reported, from its start, gathered from the
/// updates of its tool call until it is told.
#[derive(Debug, Default)]
pub(crate) struct Compacting {
    told: Told,
    failed: Option<String>,
    /// Whether it was shown, once ended: news of it after is not.
    shown: bool,
}

impl Compacting {
    /// Takes in an update of the compaction's tool call, with what its
    /// `_meta` `told` of it, and the `error` its content tells when it
    /// `failed`, and returns the compaction once it ended and is known:
    /// when it failed, or when the agent told how many tokens the
    /// context held before it, which it tells once it completed, in an
    /// update of its own after the one that completes the call.
    pub(crate) fn follow(
        &mut self,
        told: Option<Told>,
        failed: bool,
        error: Option<String>,
    ) -> Option<Compaction> {
        if let Some(told) = told {
            let was = &mut self.told;
            was.trigger = told.trigger.or(was.trigger);
            was.before = told.before.or(was.before);
            was.after = told.after.or(was.after);
            was.took = told.took.or(was.took);
        }
        if failed {
            let error = error.unwrap_or_default();
            let error = error
                .strip_prefix("Compaction failed: ")
                .unwrap_or(&error)
                .trim()
                .to_owned();
            self.failed = Some(error);
        }
        let known = self.failed.is_some() || self.told.before.is_some();
        if self.shown || !known {
            return None;
        }
        self.shown = true;
        Some(Compaction {
            trigger: self.told.trigger,
            before: self.told.before,
            after: self.told.after,
            took: self.told.took,
            failed: self.failed.clone(),
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn meta(facts: &serde_json::Value) -> Meta {
        let serde_json::Value::Object(meta) = serde_json::json!({
            "jetbrains": { "air": { "contextCompaction": facts } }
        }) else {
            unreachable!("an object");
        };
        meta
    }

    #[test]
    fn the_facts_of_a_compaction_are_read_from_meta() {
        let told = Told::of(Some(&meta(&serde_json::json!({
            "version": 1, "trigger": "idle", "preTokens": 206_713,
            "postTokens": 12_136, "durationMs": 42_410,
        }))));

        assert_eq!(
            told,
            Some(Told {
                trigger: Some(CompactionTrigger::Idle),
                before: Some(206_713),
                after: Some(12_136),
                took: Some(Duration::from_millis(42_410)),
            })
        );
    }

    #[test]
    fn meta_of_no_compaction_tells_none() {
        let mut other = Meta::new();
        other.insert(
            "claudeCode".to_owned(),
            serde_json::json!({ "toolName": "Bash" }),
        );

        assert_eq!(Told::of(Some(&other)), None);
        assert_eq!(Told::of(None), None);
    }

    #[test]
    fn a_compaction_shows_once_its_tokens_are_told() {
        let mut compacting = Compacting::default();
        let started = Told::of(Some(&meta(&serde_json::json!({ "version": 1 }))));
        let ended = Told::of(Some(&meta(&serde_json::json!({
            "version": 1, "trigger": "idle", "preTokens": 206_713, "postTokens": 12_136,
        }))));

        assert_eq!(compacting.follow(started, false, None), None);
        assert_eq!(compacting.follow(None, false, None), None, "completed");
        let shown = compacting.follow(ended.clone(), false, None);
        assert_eq!(
            shown,
            Some(Compaction {
                trigger: Some(CompactionTrigger::Idle),
                before: Some(206_713),
                after: Some(12_136),
                took: None,
                failed: None,
            })
        );
        assert_eq!(compacting.follow(ended, false, None), None, "shown once");
    }

    #[test]
    fn a_compaction_that_failed_shows_with_its_error() {
        let mut compacting = Compacting::default();
        let told = Told::of(Some(&meta(&serde_json::json!({
            "version": 1, "trigger": "manual",
        }))));

        let shown = compacting
            .follow(
                told,
                true,
                Some("Compaction failed: Not enough messages to compact.".to_owned()),
            )
            .expect("it ended");

        assert_eq!(shown.trigger, Some(CompactionTrigger::Manual));
        assert_eq!(
            shown.failed.as_deref(),
            Some("Not enough messages to compact.")
        );
    }
}
