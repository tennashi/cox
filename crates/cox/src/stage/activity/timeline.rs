//! A chat's story, as the Activity page tells it once the human opens
//! the chat's row: when it started, what it did when, and when and how
//! it ended, read from what coxd keeps of the chat only when asked, as
//! `cox://api/activity/timeline/<chat>`, rather than of every chat every
//! time the page reads what goes on.
//!
//! What it tells is the chat's milestones: the first line, each step of
//! its plans started and done, each change of its plan, with why how far
//! it went moved, each question asked and what was chosen,
//! each line on a step and the reply, each subagent opened and done, the
//! commits, the pull requests opened, merged and closed, and their CI
//! ended, each turn's end, and how what ran after it went, the chat's
//! goal and its triggers, with the end of what one that failed printed.

use cox_core::entry::{self, Check, Entry, QuestionAnswer, Revision};
use cox_core::store::{ChatId, KeptAnswer, KeptStep, Resolved, ResolvedBy, Said, Store, TurnEnd};
use coxd::daemon::wire::PullRow;
use coxd::github::CheckState;
use jiff::tz::TimeZone;
use serde_json::{Value, json};

use super::{Decided, Told};

/// The chat whose story the page asks for, as coxd keeps it.
#[derive(Clone, Copy, Debug, PartialEq, Eq, serde::Deserialize)]
pub struct Of {
    chat: ChatId,
}

/// What coxd keeps of a chat that its story is told from.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Kept {
    pub chat: ChatId,
    pub name: String,
    /// When it was opened, in milliseconds since the epoch.
    pub opened: i64,
    pub resolved: Option<Resolved>,
    /// Its milestone entries, each with when it was written, as
    /// [`Store::milestones`] reads them.
    pub milestones: Vec<(i64, Entry)>,
    /// The steps its plans had, as [`Store::steps`] reads them.
    pub steps: Vec<KeptStep>,
    pub turns: Vec<TurnEnd>,
    /// The threads of the steps the human wrote on, each with its step,
    /// by its text as it reads last.
    pub threads: Vec<(String, Vec<Said>)>,
    /// The chats its agent opened, the first first.
    pub subagents: Vec<Sub>,
}

/// A chat a chat's agent opened.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Sub {
    pub chat: ChatId,
    pub name: String,
    pub opened: i64,
    pub resolved: Option<Resolved>,
}

/// The story of the chat `of` names, as JSON, or null when coxd keeps no
/// such chat.
#[must_use]
pub fn timeline(of: &Of) -> Value {
    let read = coxd::storage::path(coxd::storage::Kept::Chats)
        .ok_or_else(|| anyhow::anyhow!("no place keeps the chats"))
        .and_then(|path| kept(&Store::read(&path)?, of.chat));
    match read {
        Ok(Some(kept)) => story(
            &kept,
            &super::now(),
            &TimeZone::system(),
            cox_core::store::now(),
        ),
        Ok(None) => Value::Null,
        Err(error) => {
            tracing::warn!("the story of a chat is not read: {error:#}");
            Value::Null
        }
    }
}

/// What `store` keeps of `chat` that its story is told from, if it keeps
/// the chat.
fn kept(store: &Store, chat: ChatId) -> anyhow::Result<Option<Kept>> {
    let chats = store.chats()?;
    let Some(this) = chats.iter().find(|kept| kept.id == chat) else {
        return Ok(None);
    };
    let opened = store.opened()?;
    let opened_of = |chat: ChatId| {
        opened
            .iter()
            .find(|(id, _)| *id == chat)
            .map_or(0, |(_, at)| *at)
    };
    let steps = store.steps(chat)?;
    let threads = store
        .on_steps(chat)?
        .into_iter()
        .map(|(step, _)| {
            let text = steps
                .iter()
                .find(|kept| kept.id == step)
                .map_or_else(String::new, |kept| kept.text.clone());
            Ok((text, store.thread(chat, step)?))
        })
        .collect::<anyhow::Result<_>>()?;
    Ok(Some(Kept {
        chat,
        name: this.name.clone(),
        opened: opened_of(chat),
        resolved: this.resolved,
        milestones: store.milestones(chat)?,
        steps,
        turns: store.turns(chat)?,
        threads,
        subagents: chats
            .iter()
            .filter(|kept| kept.parent == Some(chat))
            .map(|kept| Sub {
                chat: kept.id,
                name: kept.name.clone(),
                opened: opened_of(kept.id),
                resolved: kept.resolved,
            })
            .collect(),
    }))
}

/// One thing in a chat's story, at a time.
struct Event {
    at: i64,
    kind: &'static str,
    text: String,
    more: Value,
}

impl Event {
    fn new(at: i64, kind: &'static str, text: String) -> Self {
        Self {
            at,
            kind,
            text,
            more: json!({}),
        }
    }

    /// It, with `more` told beside what it is.
    fn with(self, more: Value) -> Self {
        Self { more, ..self }
    }

    /// It as JSON, its time as it reads in `zone` on the day of `now`.
    fn told(self, zone: &TimeZone, now: i64) -> Value {
        let mut told = time(self.at, zone, now);
        told["kind"] = json!(self.kind);
        told["text"] = json!(self.text);
        if let (Value::Object(more), Some(told)) = (self.more, told.as_object_mut()) {
            told.extend(more);
        }
        told
    }
}

/// The story of `kept`, as the window's chats stand in `told`, its times
/// as they read in `zone`, `now` being now: when it started, what it did
/// when, the first first, and how and when it ended, or null while it
/// goes on.
pub fn story(kept: &Kept, told: &Told, zone: &TimeZone, now: i64) -> Value {
    let shown = told.chats.iter().find(|chat| chat.kept == Some(kept.chat));
    let mut events = of_entries(kept, zone);
    for (step, said) in &kept.threads {
        for said in said {
            events.push(
                Event::new(said.at, "said", said.text.clone())
                    .with(json!({ "by": said.by, "step": step })),
            );
        }
    }
    for sub in &kept.subagents {
        events.push(Event::new(sub.opened, "subagent", sub.name.clone()));
        if let Some(resolved) = sub
            .resolved
            .filter(|resolved| resolved.by == ResolvedBy::Agent)
        {
            events.push(Event::new(resolved.at, "reported", sub.name.clone()));
        }
    }
    for row in shown.map_or(&[][..], |chat| chat.pulls.as_slice()) {
        events.extend(of_pull(row));
    }
    for turn in &kept.turns {
        events.push(Event::new(turn.at, "turn_end", turn.how.word().to_owned()));
    }
    // After the turn's end they ran after, though kept as soon.
    events.extend(of_checks(kept));
    events.sort_by_key(|event| event.at);
    let ended = ended(kept, shown, &events);
    json!({
        "chat": kept.chat,
        "started": time(kept.opened, zone, now),
        "ended": ended.map(|(at, how, by)| {
            let mut ended = time(at, zone, now);
            ended["how"] = json!(how);
            if let Some(by) = by {
                ended["by"] = json!(by);
            }
            ended
        }),
        "events": events
            .into_iter()
            .map(|event| event.told(zone, now))
            .collect::<Vec<_>>(),
    })
}

/// What the milestone entries and the steps of `kept` tell, the times of
/// the answers as they read in `zone`: the first line, each step started
/// and done, each change of the plan, each question asked and answered,
/// and the commits.
fn of_entries(kept: &Kept, zone: &TimeZone) -> Vec<Event> {
    let mut events = Vec::new();
    for (at, entry) in &kept.milestones {
        match entry {
            Entry::Human(line) => events.push(Event::new(*at, "start", line.clone())),
            Entry::PlanRevised(revision) => events.push(of_revision(*at, revision)),
            Entry::Question { .. } => events.extend(of_question(kept, *at, entry, zone)),
            Entry::Commits(commits) => {
                events.extend(commits.iter().map(|commit| {
                    Event::new(*at, "commit", commit.subject.clone())
                        .with(json!({ "added": commit.added, "removed": commit.removed }))
                }));
            }
            _ => {}
        }
    }
    for step in &kept.steps {
        events.extend(
            step.started
                .map(|at| Event::new(at, "step_started", step.text.clone())),
        );
        events.extend(
            step.done
                .map(|at| Event::new(at, "step_done", step.text.clone())),
        );
    }
    events
}

/// How what ran as each turn of `kept` ended went, or as something else
/// happened in it, as its goal being met: each of the chat's goal and its
/// triggers that ran, or runs, with how it went, and, for one that failed
/// or could not run, the end of what it printed, or why, and the event
/// it ran on, if it ran on one. One whose turn did not come, as one after
/// one that failed, tells nothing.
fn of_checks(kept: &Kept) -> Vec<Event> {
    let mut events = Vec::new();
    for (at, entry) in &kept.milestones {
        let on = match entry {
            Entry::Event { on, .. } => Some(on.as_str()),
            _ => None,
        };
        for check in entry.checks().unwrap_or_default() {
            events.extend(of_check(*at, check, on));
        }
    }
    events
}

/// What `check`, which ran `at` then, on the event `on`, if it ran on
/// one, tells, once its turn came.
fn of_check(at: i64, check: &Check, on: Option<&str>) -> Option<Event> {
    let state = match check.state {
        entry::CheckState::Waiting => return None,
        entry::CheckState::Running => "running",
        entry::CheckState::Passed => "passed",
        entry::CheckState::Failed => "failed",
        entry::CheckState::Broken => "broken",
    };
    let said = matches!(
        check.state,
        entry::CheckState::Failed | entry::CheckState::Broken
    )
    .then_some(check.said.as_deref())
    .flatten();
    Some(
        Event::new(at, "check", check.name.clone())
            .with(json!({ "state": state, "said": said, "on": on })),
    )
}

/// What the receipt of how the plan changed, written `at` then, tells:
/// how many steps came and went, and how much of the plan was done
/// before and after, then each step, and whether less is done than
/// before, for the page to tell why.
fn of_revision(at: i64, revision: &Revision) -> Event {
    Event::new(at, "plan_revised", revision.headline()).with(json!({
        "added": revision.added,
        "removed": revision.removed,
        "reworded": revision.reworded,
        "undone": revision.undone,
        "before": revision.before,
        "after": revision.after,
        "dropped": revision.dropped(),
    }))
}

/// What `question`, asked in `kept` `at` then, tells: that it was asked,
/// and what the human chose, or that they declined, once they answered.
fn of_question(kept: &Kept, at: i64, question: &Entry, zone: &TimeZone) -> Vec<Event> {
    let Entry::Question {
        message,
        questions,
        answer,
        answered_at,
    } = question
    else {
        return Vec::new();
    };
    let asked: Vec<String> = questions
        .iter()
        .filter_map(|question| question.text.clone().or_else(|| question.header.clone()))
        .collect();
    let asked = if asked.len() == 1 || message.trim().is_empty() {
        asked.join(" · ")
    } else {
        message.trim().to_owned()
    };
    let mut events = vec![Event::new(at, "asked", asked)];
    let answered = answered_at.unwrap_or(at);
    match answer {
        Some(QuestionAnswer::Given(_)) => {
            let kept = KeptAnswer {
                chat: kept.chat,
                name: kept.name.clone(),
                project: None,
                folder: None,
                at: answered,
                entry: question.clone(),
            };
            events.extend(Decided::of(&kept, zone).into_iter().map(|decided| {
                Event::new(answered, "answered", decided.choice).with(json!({
                    "question": decided.question,
                    "why": decided.why,
                    "other": decided.other,
                }))
            }));
        }
        Some(QuestionAnswer::Declined) => {
            events.push(Event::new(answered, "declined", String::new()));
        }
        None => {}
    }
    events
}

/// What the pull request of `row` tells, as coxd last read it: when it
/// was opened, merged or else closed, and when its CI ended, and how.
fn of_pull(row: &PullRow) -> Vec<Event> {
    let Some(status) = &row.status else {
        return Vec::new();
    };
    let number = json!({ "number": row.pull.number });
    let pull = |at: Option<i64>, kind| {
        at.map(|at| Event::new(at, kind, status.title.clone()).with(number.clone()))
    };
    let mut events: Vec<Event> = pull(status.times.opened, "pull_opened")
        .into_iter()
        .chain(pull(status.times.merged, "pull_merged"))
        .collect();
    if status.times.merged.is_none() {
        events.extend(pull(status.times.closed, "pull_closed"));
    }
    let ci = status.checks().and_then(|state| match state {
        CheckState::Passed => Some("passed"),
        CheckState::Failed => Some("failed"),
        CheckState::Skipped => Some("skipped"),
        CheckState::Pending => None,
    });
    if let (Some(at), Some(ci)) = (status.checks_ended(), ci) {
        events.push(Event::new(at, "ci", ci.to_owned()).with(number));
    }
    events
}

/// How `kept` ended, as it shows in the window as `shown`, with the
/// events of its story: when, how and by whom it was marked done; that
/// it asks the human, since it asked; or how and when its last turn
/// ended; none while its agent works, or before any turn ended.
fn ended(
    kept: &Kept,
    shown: Option<&super::Chat>,
    events: &[Event],
) -> Option<(i64, &'static str, Option<&'static str>)> {
    if let Some(resolved) = kept.resolved {
        let by = match resolved.by {
            ResolvedBy::Human => "human",
            ResolvedBy::Agent => "agent",
        };
        return Some((resolved.at, "resolved", Some(by)));
    }
    if shown.is_some_and(|chat| chat.working) {
        return None;
    }
    if shown.is_some_and(|chat| chat.asking) {
        let asked = events.iter().rev().find(|event| event.kind == "asked")?;
        return Some((asked.at, "asks", None));
    }
    let last = kept.turns.last()?;
    Some((last.at, last.how.word(), None))
}

/// `at`, in milliseconds since the epoch, as JSON: as it is; as a line
/// reads it, `when`, by the time of day in `zone`, with the day too when
/// it is not the day of `now`; and apart, for the story's column of times,
/// which has room for the time of day alone, the time of day, `clock`,
/// and the day, `day`, when it is not the day of `now`, which the story
/// tells on a line of its own where it changes.
fn time(at: i64, zone: &TimeZone, now: i64) -> Value {
    let zoned = |at: i64| {
        jiff::Timestamp::from_millisecond(at)
            .ok()
            .map(|at| at.to_zoned(zone.clone()))
    };
    let (when, clock, day) = match (zoned(at), zoned(now)) {
        (Some(at), Some(now)) if at.date() == now.date() => {
            let clock = at.strftime("%H:%M").to_string();
            (clock.clone(), clock, None)
        }
        (Some(at), _) => (
            at.strftime("%b %-d %H:%M").to_string(),
            at.strftime("%H:%M").to_string(),
            Some(at.strftime("%b %-d").to_string()),
        ),
        _ => (String::new(), String::new(), None),
    };
    json!({ "at": at, "when": when, "clock": clock, "day": day })
}

#[cfg(test)]
mod tests {
    use super::*;
    use cox_core::entry::{Committed, Picked, Question, QuestionOption};
    use cox_core::store::{Ended, Sayer};
    use coxd::github::{Check, Pull, PullState, Status, Times};

    use crate::stage::activity::Chat;

    const MIN: i64 = 60_000;

    fn kept(number: u64) -> ChatId {
        serde_json::from_value(json!(number)).unwrap()
    }

    fn shown(key: u64, chat: ChatId) -> Chat {
        Chat {
            key,
            name: "Speed up CI".to_owned(),
            cwd: "/src/app".into(),
            project: "/src/app".into(),
            asking: false,
            unread: false,
            goal: None,
            working: false,
            background: Vec::new(),
            parent: None,
            kept: Some(chat),
            pulls: Vec::new(),
            spent: None,
            resolved: false,
            reported: false,
            archived: false,
            steers: false,
            on_steps: Vec::new(),
        }
    }

    /// What each event of `story` is, by its kind, its text and its time
    /// of day.
    fn told(story: &Value) -> Vec<(String, String, String)> {
        story["events"]
            .as_array()
            .unwrap()
            .iter()
            .map(|event| {
                (
                    event["when"].as_str().unwrap().to_owned(),
                    event["kind"].as_str().unwrap().to_owned(),
                    event["text"].as_str().unwrap().to_owned(),
                )
            })
            .collect()
    }

    fn at(hour: i64, minute: i64) -> i64 {
        // 2026-11-19 in UTC.
        1_795_046_400_000 + (hour * 60 + minute) * MIN
    }

    #[test]
    #[expect(clippy::too_many_lines, reason = "one chat's whole story, told")]
    fn tells_what_a_chat_did_when_and_how_it_ended_the_first_first() {
        let chat = kept(7);
        let question = Entry::Question {
            message: "Where should CI keep its cache?".to_owned(),
            questions: vec![Question {
                header: Some("Cache".to_owned()),
                text: None,
                options: vec![
                    QuestionOption {
                        label: "GitHub Actions cache".to_owned(),
                        description: Some("No other service to set up.".to_owned()),
                    },
                    QuestionOption {
                        label: "S3".to_owned(),
                        description: None,
                    },
                ],
                multiple: false,
                other: true,
            }],
            answer: Some(QuestionAnswer::Given(vec![Picked {
                chosen: vec![0],
                other: None,
            }])),
            answered_at: Some(at(14, 20)),
        };
        let kept = Kept {
            chat,
            name: "Speed up CI".to_owned(),
            opened: at(13, 48),
            resolved: None,
            milestones: vec![
                (
                    at(13, 48),
                    Entry::Human("CI takes 11 minutes, make it faster".to_owned()),
                ),
                (at(13, 52), question),
                (
                    at(14, 25),
                    Entry::Commits(vec![Committed {
                        folder: "/src/app".into(),
                        repository: None,
                        id: "abc".to_owned(),
                        subject: "ci: cache cargo".to_owned(),
                        added: 38,
                        removed: 4,
                        amended: None,
                    }]),
                ),
            ],
            steps: vec![
                KeptStep {
                    id: 1,
                    text: "Find why the build is slow".to_owned(),
                    started: Some(at(13, 49)),
                    done: Some(at(13, 51)),
                },
                // Reworded since it was planned, and done.
                KeptStep {
                    id: 2,
                    text: "Cache the dependencies in CI".to_owned(),
                    started: Some(at(14, 21)),
                    done: Some(at(14, 24)),
                },
            ],
            turns: vec![TurnEnd {
                at: at(14, 29),
                how: Ended::Done,
            }],
            threads: vec![(
                "Cache the dependencies in CI".to_owned(),
                vec![
                    Said {
                        by: Sayer::Human,
                        text: "cache target too".to_owned(),
                        at: at(14, 22),
                        number: 9,
                    },
                    Said {
                        by: Sayer::Agent,
                        text: "Will do.".to_owned(),
                        at: at(14, 23),
                        number: 10,
                    },
                ],
            )],
            subagents: vec![Sub {
                chat: kept(8),
                name: "Time the build".to_owned(),
                opened: at(13, 50),
                resolved: Some(Resolved {
                    at: at(13, 51),
                    by: ResolvedBy::Agent,
                }),
            }],
        };
        let mut row = shown(3, chat);
        row.pulls = vec![PullRow {
            pull: Pull {
                repo: "tennashi/cox".to_owned(),
                number: 86,
            },
            status: Some(Status {
                title: "Cache the dependencies in CI".to_owned(),
                state: PullState::Merged,
                head: "a".to_owned(),
                checks: vec![Check {
                    name: "ci".to_owned(),
                    state: CheckState::Passed,
                    link: None,
                    ended: Some(at(14, 30)),
                }],
                times: Times {
                    opened: Some(at(14, 26)),
                    merged: Some(at(14, 31)),
                    closed: Some(at(14, 31)),
                },
            }),
            unseen: None,
        }];
        let told_now = Told {
            chats: vec![row],
            ..Told::default()
        };

        let story = story(&kept, &told_now, &TimeZone::UTC, at(15, 0));

        assert_eq!(story["started"]["when"], "13:48");
        let events = told(&story);
        let expected: Vec<(&str, &str, &str)> = vec![
            ("13:48", "start", "CI takes 11 minutes, make it faster"),
            ("13:49", "step_started", "Find why the build is slow"),
            ("13:50", "subagent", "Time the build"),
            ("13:51", "step_done", "Find why the build is slow"),
            ("13:51", "reported", "Time the build"),
            ("13:52", "asked", "Cache"),
            ("14:20", "answered", "GitHub Actions cache"),
            ("14:21", "step_started", "Cache the dependencies in CI"),
            ("14:22", "said", "cache target too"),
            ("14:23", "said", "Will do."),
            ("14:24", "step_done", "Cache the dependencies in CI"),
            ("14:25", "commit", "ci: cache cargo"),
            ("14:26", "pull_opened", "Cache the dependencies in CI"),
            ("14:29", "turn_end", "done"),
            ("14:30", "ci", "passed"),
            ("14:31", "pull_merged", "Cache the dependencies in CI"),
        ];
        assert_eq!(
            events,
            expected
                .iter()
                .map(|(when, kind, text)| (
                    (*when).to_owned(),
                    (*kind).to_owned(),
                    (*text).to_owned()
                ))
                .collect::<Vec<_>>()
        );
        let answered = &story["events"][6];
        assert_eq!(answered["question"], "Where should CI keep its cache?");
        assert_eq!(answered["why"], "No other service to set up.");
        assert_eq!(story["events"][11]["added"], 38);
        assert_eq!(story["events"][8]["by"], "human");
        assert_eq!(story["events"][8]["step"], "Cache the dependencies in CI");
        // Its last turn ended as turns do, and nothing works now.
        assert_eq!(story["ended"]["how"], "done");
        assert_eq!(story["ended"]["when"], "14:29");
    }

    #[test]
    fn tells_each_change_of_the_plan_with_why_how_far_it_went_moved() {
        let chat = kept(7);
        let revision = Revision {
            before: cox_core::entry::StepsDone { done: 4, of: 5 },
            after: cox_core::entry::StepsDone { done: 5, of: 8 },
            added: vec![
                "Write the docs".to_owned(),
                "Push".to_owned(),
                "Open a pull request".to_owned(),
            ],
            removed: Vec::new(),
            reworded: vec![cox_core::entry::Reworded {
                from: "Test".to_owned(),
                to: "Test it all".to_owned(),
            }],
            undone: Vec::new(),
            changes: 1,
        };
        let kept = Kept {
            chat,
            name: String::new(),
            opened: at(10, 0),
            resolved: None,
            threads: Vec::new(),
            subagents: Vec::new(),
            steps: Vec::new(),
            milestones: vec![(at(10, 7), Entry::PlanRevised(revision))],
            turns: Vec::new(),
        };

        let story = story(&kept, &Told::default(), &TimeZone::UTC, at(12, 0));

        assert_eq!(
            told(&story),
            [(
                "10:07".to_owned(),
                "plan_revised".to_owned(),
                "+3 steps, 1 reworded · 80% → 62%".to_owned()
            )]
        );
        let event = &story["events"][0];
        assert_eq!(
            event["added"],
            json!(["Write the docs", "Push", "Open a pull request"])
        );
        assert_eq!(
            event["reworded"],
            json!([{ "from": "Test", "to": "Test it all" }])
        );
        assert_eq!(event["removed"], json!([]));
        assert_eq!(
            (&event["before"], &event["after"]),
            (
                &json!({ "done": 4, "of": 5 }),
                &json!({ "done": 5, "of": 8 })
            )
        );
        assert_eq!(event["dropped"], true);
    }

    #[test]
    fn tells_how_what_ran_as_each_turn_ended_went_after_the_turn() {
        use cox_core::entry::CheckState as Ran;
        let chat = kept(7);
        let check = |name: &str, state, said: Option<&str>| entry::Check {
            name: name.to_owned(),
            state,
            said: said.map(str::to_owned),
        };
        let kept = Kept {
            chat,
            name: String::new(),
            opened: at(10, 0),
            resolved: None,
            threads: Vec::new(),
            subagents: Vec::new(),
            steps: Vec::new(),
            // Kept as the turn ended, in the same millisecond.
            milestones: vec![
                (
                    at(10, 5),
                    Entry::Checks(vec![
                        check("goal", Ran::Failed, Some("test a ... FAILED")),
                        check("lint", Ran::Waiting, None),
                    ]),
                ),
                (
                    at(10, 9),
                    Entry::Checks(vec![
                        check("goal", Ran::Passed, Some("test result: ok")),
                        check("lint", Ran::Running, None),
                    ]),
                ),
                (
                    at(10, 10),
                    Entry::Event {
                        on: "goal.met".to_owned(),
                        checks: vec![check("docs", Ran::Broken, Some("no agent"))],
                    },
                ),
            ],
            turns: vec![
                TurnEnd {
                    at: at(10, 5),
                    how: Ended::Done,
                },
                TurnEnd {
                    at: at(10, 9),
                    how: Ended::Done,
                },
            ],
        };

        let story = story(&kept, &Told::default(), &TimeZone::UTC, at(12, 0));

        let expected: Vec<(String, String, String)> = [
            ("10:05", "turn_end", "done"),
            ("10:05", "check", "goal"),
            ("10:09", "turn_end", "done"),
            ("10:09", "check", "goal"),
            ("10:09", "check", "lint"),
            ("10:10", "check", "docs"),
        ]
        .iter()
        .map(|(when, kind, text)| ((*when).to_owned(), (*kind).to_owned(), (*text).to_owned()))
        .collect();
        assert_eq!(told(&story), expected);
        let checks: Vec<(&Value, &Value, &Value)> = story["events"]
            .as_array()
            .unwrap()
            .iter()
            .filter(|event| event["kind"] == "check")
            .map(|event| (&event["state"], &event["said"], &event["on"]))
            .collect();
        assert_eq!(
            checks,
            [
                // What one that failed printed, at the end, opens under it.
                (&json!("failed"), &json!("test a ... FAILED"), &Value::Null),
                (&json!("passed"), &Value::Null, &Value::Null),
                (&json!("running"), &Value::Null, &Value::Null),
                (&json!("broken"), &json!("no agent"), &json!("goal.met")),
            ]
        );
    }

    #[test]
    fn a_chat_ends_marked_done_asking_or_stopped_and_goes_on_while_at_work() {
        let chat = kept(7);
        let mut kept = Kept {
            chat,
            name: String::new(),
            opened: at(10, 0),
            resolved: None,
            threads: Vec::new(),
            subagents: Vec::new(),
            steps: Vec::new(),
            milestones: vec![(
                at(10, 5),
                Entry::Question {
                    message: "Tabs or one page?".to_owned(),
                    questions: Vec::new(),
                    answer: None,
                    answered_at: None,
                },
            )],
            turns: vec![TurnEnd {
                at: at(10, 2),
                how: Ended::Stopped,
            }],
        };
        let mut row = shown(3, chat);
        let story_of = |kept: &Kept, row: &Chat| {
            story(
                kept,
                &Told {
                    chats: vec![row.clone()],
                    ..Told::default()
                },
                &TimeZone::UTC,
                at(12, 0),
            )
        };

        assert_eq!(story_of(&kept, &row)["ended"]["how"], "stopped");
        assert_eq!(
            told(&story_of(&kept, &row)),
            [
                (
                    "10:02".to_owned(),
                    "turn_end".to_owned(),
                    "stopped".to_owned()
                ),
                (
                    "10:05".to_owned(),
                    "asked".to_owned(),
                    "Tabs or one page?".to_owned()
                ),
            ]
        );

        row.asking = true;
        assert_eq!(
            (
                story_of(&kept, &row)["ended"]["how"].clone(),
                story_of(&kept, &row)["ended"]["when"].clone()
            ),
            (json!("asks"), json!("10:05"))
        );

        row.asking = false;
        row.working = true;
        assert_eq!(story_of(&kept, &row)["ended"], Value::Null);

        kept.resolved = Some(Resolved {
            at: at(11, 0),
            by: ResolvedBy::Human,
        });
        let story = story_of(&kept, &row);
        assert_eq!(
            (&story["ended"]["how"], &story["ended"]["by"]),
            (&json!("resolved"), &json!("human"))
        );

        // Another day's time tells the day, and apart from the time of
        // day, for the story's column, which has room for that alone.
        let yesterday = super::time(at(11, 0) - 24 * 60 * MIN, &TimeZone::UTC, at(12, 0));
        assert_eq!(
            yesterday,
            json!({
                "at": at(11, 0) - 24 * 60 * MIN,
                "when": "Nov 18 11:00",
                "clock": "11:00",
                "day": "Nov 18",
            })
        );
        let today = super::time(at(11, 0), &TimeZone::UTC, at(12, 0));
        assert_eq!(
            (&today["when"], &today["clock"], &today["day"]),
            (&json!("11:00"), &json!("11:00"), &Value::Null)
        );
    }
}
