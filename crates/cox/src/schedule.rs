//! Agents run on a schedule: a chat that opens at the times the human
//! says, and goes on its first line with no human there, as to research
//! a topic every morning. Each is a file of the human's workflow (see
//! [`crate::settings::workflow`]), in `schedules`, named after it, as
//! `schedules/rust-news.md`, whose properties, the front matter between
//! `---` lines at its top, say when it runs (`every`), where its chat
//! works (`folder`), and what its agent does unasked (`allow`), and whose
//! rest is the chat's first line:
//!
//! ```markdown
//! ---
//! every: daily 09:00
//! folder: ~/notes
//! allow:
//!   - fetch:*
//!   - search:*
//! ---
//!
//! Find out what changed in Rust since {last}.
//! ```
//!
//! coxd looks every minute which are due, and opens a chat for each (see
//! [`crate::daemon`]). When each last ran is kept where Cox keeps what
//! lasts (see [`crate::storage::Kept::Schedules`]), so that a run Cox
//! missed, as while the machine slept, runs once as Cox finds it due,
//! however many were missed.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use jiff::civil::{Time, Weekday};
use jiff::{Span, Timestamp, Zoned};

/// The folder of the human's workflow that holds the schedules.
pub const SCHEDULES: &str = "schedules";

/// When a schedule runs.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Every {
    /// Each day, at that time, as `daily 09:00`.
    Day(Time),
    /// Each week, on that day, at that time, as `weekly mon 09:00`.
    Week(Weekday, Time),
    /// So many hours after it last ran, as `every 6h`.
    Hours(i64),
}

impl Every {
    /// What `text` says, as a schedule's `every` writes it, or why it
    /// says nothing Cox knows.
    ///
    /// # Errors
    ///
    /// When `text` is none of `daily HH:MM`, `weekly <day> HH:MM` and
    /// `every <n>h`.
    pub fn parse(text: &str) -> Result<Self, String> {
        let words: Vec<&str> = text.split_whitespace().collect();
        let time = |text: &str| {
            Time::strptime("%H:%M", text).map_err(|_| format!("{text:?} is no time, as 09:00"))
        };
        match words.as_slice() {
            ["daily", at] => Ok(Self::Day(time(at)?)),
            ["weekly", day, at] => Ok(Self::Week(weekday(day)?, time(at)?)),
            ["every", hours] => hours
                .strip_suffix('h')
                .and_then(|hours| hours.parse().ok())
                .filter(|&hours: &i64| hours > 0)
                .map(Self::Hours)
                .ok_or_else(|| format!("{hours:?} is no number of hours, as 6h")),
            _ => Err(format!(
                "{text:?} says no time Cox knows: daily 09:00, weekly mon 09:00 or every 6h"
            )),
        }
    }

    /// Whether a schedule that ran `last`, if it did, is due `now`: when
    /// the last time it was to run since is past, or, run every so many
    /// hours, so many have gone by. One that never ran is due at once,
    /// for the human to see what it does.
    #[must_use]
    pub fn due(self, last: Option<Timestamp>, now: &Zoned) -> bool {
        let Some(last) = last else {
            return true;
        };
        match self {
            Self::Hours(hours) => last
                .checked_add(Span::new().hours(hours))
                .is_ok_and(|next| next <= now.timestamp()),
            Self::Day(_) | Self::Week(..) => self
                .last_before(now)
                .is_some_and(|was| last < was.timestamp()),
        }
    }

    /// The last time, up to `now`, that a schedule run each day or week
    /// was to run.
    fn last_before(self, now: &Zoned) -> Option<Zoned> {
        let (day, at) = match self {
            Self::Day(at) => (None, at),
            Self::Week(day, at) => (Some(day), at),
            Self::Hours(_) => return None,
        };
        let today = now.date();
        (0..8).find_map(|back| {
            let date = today.checked_sub(Span::new().days(back)).ok()?;
            if day.is_some_and(|day| date.weekday() != day) {
                return None;
            }
            let then = date
                .to_datetime(at)
                .to_zoned(now.time_zone().clone())
                .ok()?;
            (then.timestamp() <= now.timestamp()).then_some(then)
        })
    }
}

/// The day `text` names, by its first three letters, as `mon`.
fn weekday(text: &str) -> Result<Weekday, String> {
    let day = match text.to_ascii_lowercase().get(..3) {
        Some("mon") => Weekday::Monday,
        Some("tue") => Weekday::Tuesday,
        Some("wed") => Weekday::Wednesday,
        Some("thu") => Weekday::Thursday,
        Some("fri") => Weekday::Friday,
        Some("sat") => Weekday::Saturday,
        Some("sun") => Weekday::Sunday,
        _ => return Err(format!("{text:?} is no day of the week, as mon")),
    };
    Ok(day)
}

/// A schedule, as its file has it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Schedule {
    /// What it goes by: its file's name, less `.md`.
    pub name: String,
    /// When it runs.
    pub every: Every,
    /// Where its chat works, or, when `None`, Cox's home, in no project.
    pub folder: Option<PathBuf>,
    /// The patterns of the permission requests its agent goes on with
    /// unasked, as `open_chat`'s `allow` takes them.
    pub allow: Vec<String>,
    /// Its chat's first line, with `{name}`, `{now}` and `{last}` standing
    /// for its name, when it runs and when it ran last.
    pub line: String,
}

impl Schedule {
    /// The schedule of the file at `path`, from the home directory
    /// `user` for a folder from `~`, unless it is no Markdown file or
    /// says nothing of when it runs.
    ///
    /// # Errors
    ///
    /// When it says when it runs, or where, in a way Cox does not know.
    pub fn read(path: &Path, user: Option<&Path>) -> Result<Option<Self>, String> {
        if path.extension().is_none_or(|extension| extension != "md") {
            return Ok(None);
        }
        let Some(name) = path.file_stem().and_then(|name| name.to_str()) else {
            return Ok(None);
        };
        let Ok(text) = std::fs::read_to_string(path) else {
            return Ok(None);
        };
        let (properties, line) = crate::wiki::front(&text);
        let value = |key| {
            properties
                .iter()
                .find_map(|line| crate::wiki::property(line, key))
                .map(unquoted)
                .filter(|value| !value.is_empty())
        };
        let Some(every) = value("every") else {
            return Ok(None);
        };
        let said = |why: String| format!("{}: {why}", path.display());
        let every = Every::parse(every).map_err(said)?;
        let folder = value("folder")
            .map(|folder| crate::settings::folder_of(folder, user))
            .transpose()
            .map_err(|why| said(why.to_owned()))?;
        Ok(Some(Self {
            name: name.to_owned(),
            every,
            folder,
            allow: list(&properties, "allow"),
            line: line.trim().to_owned(),
        }))
    }

    /// The first line of its run `now`, which ran last `last`, if it did.
    #[must_use]
    pub fn line(&self, now: &Zoned, last: Option<Timestamp>) -> String {
        let at = |when: &Zoned| when.strftime("%Y-%m-%d %H:%M").to_string();
        let last = last.map_or_else(
            || "never".to_owned(),
            |last| at(&last.to_zoned(now.time_zone().clone())),
        );
        crate::workflow::fill(
            &self.line,
            &[("name", &self.name), ("now", &at(now)), ("last", &last)],
        )
    }

    /// What its chat is named, as it runs `now`.
    #[must_use]
    pub fn chat_name(&self, now: &Zoned) -> String {
        format!("{} {}", self.name, now.strftime("%m/%d %H:%M"))
    }
}

/// `value` without the quotes around it, if it has them.
fn unquoted(value: &str) -> &str {
    let value = value.trim();
    ['"', '\'']
        .iter()
        .find_map(|&quote| value.strip_prefix(quote)?.strip_suffix(quote))
        .unwrap_or(value)
}

/// The list the property `key` of `properties` sets, as YAML writes one:
/// on its line, as `[read:*, fetch:*]`, or a line each after it, as `-
/// read:*`.
fn list(properties: &[&str], key: &str) -> Vec<String> {
    let Some(at) = properties
        .iter()
        .position(|line| crate::wiki::property(line, key).is_some())
    else {
        return Vec::new();
    };
    let inline = crate::wiki::property(properties[at], key).unwrap_or_default();
    if let Some(items) = inline
        .strip_prefix('[')
        .and_then(|items| items.strip_suffix(']'))
    {
        return items
            .split(',')
            .map(unquoted)
            .filter(|item| !item.is_empty())
            .map(str::to_owned)
            .collect();
    }
    properties[at + 1..]
        .iter()
        .map_while(|line| line.trim_start().strip_prefix("- "))
        .map(|item| unquoted(item).to_owned())
        .filter(|item| !item.is_empty())
        .collect()
}

/// The schedules in `workflow`, the folder of the human's workflow, by
/// name, from the home directory `user`, saying in the log those Cox
/// cannot read.
#[must_use]
pub fn schedules(workflow: &Path, user: Option<&Path>) -> Vec<Schedule> {
    let Ok(entries) = std::fs::read_dir(workflow.join(SCHEDULES)) else {
        return Vec::new();
    };
    let mut schedules: Vec<Schedule> = entries
        .filter_map(Result::ok)
        .filter_map(|entry| match Schedule::read(&entry.path(), user) {
            Ok(schedule) => schedule,
            Err(why) => {
                tracing::warn!("a schedule does not run: {why}");
                None
            }
        })
        .collect();
    schedules.sort_by(|one, other| one.name.cmp(&other.name));
    schedules
}

/// When each schedule last ran, by its name, as Cox keeps it in the file
/// at `path`.
#[derive(Debug, Default, PartialEq, Eq)]
pub struct Ran {
    path: Option<PathBuf>,
    last: BTreeMap<String, Timestamp>,
}

impl Ran {
    /// What the file at `path` keeps, if there is a place for it, or
    /// nothing yet.
    #[must_use]
    pub fn read(path: Option<PathBuf>) -> Self {
        let last = path
            .as_deref()
            .and_then(|path| std::fs::read_to_string(path).ok())
            .and_then(|text| {
                serde_json::from_str::<BTreeMap<String, String>>(&text)
                    .inspect_err(|error| tracing::warn!("when schedules ran is lost: {error}"))
                    .ok()
            })
            .unwrap_or_default()
            .into_iter()
            .filter_map(|(name, when)| Some((name, when.parse().ok()?)))
            .collect();
        Self { path, last }
    }

    /// When the schedule `name` last ran.
    #[must_use]
    pub fn last(&self, name: &str) -> Option<Timestamp> {
        self.last.get(name).copied()
    }

    /// Keeps that the schedule `name` ran `now`.
    pub fn ran(&mut self, name: &str, now: Timestamp) {
        self.last.insert(name.to_owned(), now);
        let Some(path) = &self.path else {
            return;
        };
        let kept: BTreeMap<&str, String> = self
            .last
            .iter()
            .map(|(name, when)| (name.as_str(), when.to_string()))
            .collect();
        let written = path
            .parent()
            .map_or(Ok(()), std::fs::create_dir_all)
            .and_then(|()| {
                let text = serde_json::to_string_pretty(&kept).map_err(std::io::Error::other)?;
                std::fs::write(path, text + "\n")
            });
        if let Err(error) = written {
            tracing::error!(
                "{} does not keep when schedules ran: {error}",
                path.display()
            );
        }
    }
}

#[cfg(test)]
mod tests {
    use std::path::Path;

    use jiff::Zoned;
    use jiff::civil::{Time, Weekday, date};
    use jiff::tz::TimeZone;

    use super::{Every, Ran, Schedule};

    fn at(day: i8, hour: i8, minute: i8) -> Zoned {
        // 2026-11-02 is a Monday.
        date(2026, 11, day)
            .at(hour, minute, 0, 0)
            .to_zoned(TimeZone::fixed(jiff::tz::offset(9)))
            .unwrap()
    }

    /// Having run at that time, as the schedule's last run.
    fn ran(day: i8, hour: i8, minute: i8) -> Option<jiff::Timestamp> {
        at(day, hour, minute).timestamp().into()
    }

    #[test]
    fn when_a_schedule_runs_is_said_in_a_few_ways() {
        assert_eq!(
            Every::parse("daily 09:00"),
            Ok(Every::Day(Time::constant(9, 0, 0, 0)))
        );
        assert_eq!(
            Every::parse("weekly Mon 18:30"),
            Ok(Every::Week(Weekday::Monday, Time::constant(18, 30, 0, 0)))
        );
        assert_eq!(Every::parse("every 6h"), Ok(Every::Hours(6)));
        assert!(Every::parse("every 0h").is_err());
        assert!(Every::parse("daily 9am").is_err());
        assert!(Every::parse("hourly").is_err());
    }

    #[test]
    fn a_daily_schedule_runs_once_its_time_is_past_however_many_were_missed() {
        let daily = Every::parse("daily 09:00").unwrap();
        // Never ran: at once.
        assert!(daily.due(None, &at(3, 8, 59)));
        // Ran at nine: not again until nine tomorrow.
        assert!(!daily.due(ran(3, 9, 0), &at(3, 23, 0)));
        assert!(daily.due(ran(3, 9, 0), &at(4, 9, 1)));
        // Missed for days, it runs once, and then waits.
        assert!(daily.due(ran(1, 9, 0), &at(5, 7, 0)));
        assert!(!daily.due(ran(5, 7, 0), &at(5, 8, 0)));
    }

    #[test]
    fn a_weekly_schedule_runs_on_its_day() {
        let weekly = Every::parse("weekly wed 10:00").unwrap();
        assert!(!weekly.due(ran(2, 12, 0), &at(4, 9, 0)));
        assert!(weekly.due(ran(2, 12, 0), &at(4, 10, 0)));
        assert!(!weekly.due(ran(4, 10, 0), &at(10, 10, 0)));
        assert!(weekly.due(ran(4, 10, 0), &at(11, 10, 0)));
    }

    #[test]
    fn a_schedule_run_every_so_many_hours_runs_at_once_and_then_so_long_after() {
        let hours = Every::parse("every 6h").unwrap();
        assert!(hours.due(None, &at(3, 1, 0)));
        assert!(!hours.due(ran(3, 1, 0), &at(3, 6, 59)));
        assert!(hours.due(ran(3, 1, 0), &at(3, 7, 0)));
    }

    #[test]
    fn a_schedule_is_a_file_of_the_workflow() {
        let workflow = std::env::temp_dir().join(format!("cox-schedules-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&workflow);
        let schedules = workflow.join("schedules");
        std::fs::create_dir_all(&schedules).unwrap();
        std::fs::write(
            schedules.join("rust-news.md"),
            "---\nevery: \"daily 09:00\"\nfolder: ~/notes\nallow:\n  - fetch:*\n  - 'search:*'\n---\n\nFind out what changed in Rust since {last}, as {name}.\n",
        )
        .unwrap();
        std::fs::write(
            schedules.join("tidy.md"),
            "---\nevery: every 6h\nallow: [read:*, \"edit:*\"]\n---\nTidy up.\n",
        )
        .unwrap();
        std::fs::write(schedules.join("broken.md"), "---\nevery: soon\n---\nNo.\n").unwrap();
        std::fs::write(schedules.join("notes.md"), "Not a schedule.\n").unwrap();
        std::fs::write(schedules.join("readme.txt"), "---\nevery: every 1h\n---\n").unwrap();

        let found = super::schedules(&workflow, Some(Path::new("/home/me")));
        assert_eq!(
            found,
            [
                Schedule {
                    name: "rust-news".to_owned(),
                    every: Every::Day(Time::constant(9, 0, 0, 0)),
                    folder: Some("/home/me/notes".into()),
                    allow: vec!["fetch:*".to_owned(), "search:*".to_owned()],
                    line: "Find out what changed in Rust since {last}, as {name}.".to_owned(),
                },
                Schedule {
                    name: "tidy".to_owned(),
                    every: Every::Hours(6),
                    folder: None,
                    allow: vec!["read:*".to_owned(), "edit:*".to_owned()],
                    line: "Tidy up.".to_owned(),
                },
            ]
        );
        let news = &found[0];
        assert_eq!(
            news.line(&at(3, 9, 30), None),
            "Find out what changed in Rust since never, as rust-news."
        );
        assert_eq!(
            news.line(&at(4, 9, 0), ran(3, 9, 30)),
            "Find out what changed in Rust since 2026-11-03 09:30, as rust-news."
        );
        assert_eq!(news.chat_name(&at(3, 9, 30)), "rust-news 11/03 09:30");
        let _ = std::fs::remove_dir_all(&workflow);
    }

    #[test]
    fn when_each_schedule_ran_is_kept() {
        let path = std::env::temp_dir()
            .join(format!("cox-ran-{}", std::process::id()))
            .join("schedules.json");
        let _ = std::fs::remove_file(&path);
        let mut kept = Ran::read(Some(path.clone()));
        assert_eq!(kept.last("rust-news"), None);
        kept.ran("rust-news", at(3, 9, 30).timestamp());
        assert_eq!(
            Ran::read(Some(path.clone())).last("rust-news"),
            ran(3, 9, 30)
        );
        let _ = std::fs::remove_dir_all(path.parent().unwrap());
    }
}
