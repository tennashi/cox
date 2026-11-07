//! Agents run on a schedule: a chat that opens at the times the human
//! says, and goes on its first line with no human there, as to research
//! a topic every morning. Each is a trigger (see [`crate::trigger`]) that
//! runs on `schedule`, at the times its `cron` says, in the time of the
//! place Cox runs in, `by: new`, a new chat, whose first line is the
//! trigger's text:
//!
//! ```markdown
//! ---
//! on:
//!   schedule:
//!     cron: "0 9 * * *"
//! by: new
//! folder: ~/notes
//! allow: [fetch:*, search:*]
//! ---
//!
//! Find out what changed in Rust since {last}.
//! ```
//!
//! An earlier Cox kept them apart, in `schedules`, saying when they run
//! with `every`, as `daily 09:00`: Cox reads those as triggers too, as
//! the cron that means (see [`Cron::from_every`]).
//!
//! coxd looks every minute which are due, and opens a chat for each (see
//! [`crate::daemon`]). When each last ran is kept where Cox keeps what
//! lasts (see [`crate::storage::Kept::Schedules`]), so that a run Cox
//! missed, as while the machine slept, runs once as Cox finds it due,
//! however many were missed.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use jiff::civil::{Date, Time, Weekday};
use jiff::tz::TimeZone;
use jiff::{Span, Timestamp, Zoned};

/// The folder of the human's workflow where an earlier Cox kept the
/// schedules, which Cox reads as triggers.
pub const SCHEDULES: &str = "schedules";

/// The folder of a project where an earlier Cox kept its schedules.
pub const PROJECT_SCHEDULES: &str = ".cox/schedules";

/// How far ahead Cox looks for the next time a cron says, in days: far
/// enough for the 29th of February on a day of the week.
const AHEAD: i64 = 366 * 8;

/// The months, as a cron may name them.
const MONTHS: [&str; 12] = [
    "jan", "feb", "mar", "apr", "may", "jun", "jul", "aug", "sep", "oct", "nov", "dec",
];

/// The days of the week, from Sunday, as a cron may name them.
const DAYS: [&str; 7] = ["sun", "mon", "tue", "wed", "thu", "fri", "sat"];

/// The times a schedule runs, as cron writes them: the minutes, hours,
/// days of the month, months and days of the week, as `0 9 * * 1-5` for
/// nine on each weekday. Each field is `*`, a number, a range `a-b`, any
/// of them `/` a step, or a list of those, `,` between them; months and
/// days of the week may be named by their first three letters, and a day
/// of the week is 0 or 7 for Sunday. When both days are given, a day
/// either names runs it, as cron has it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Cron {
    text: String,
    minutes: u64,
    hours: u32,
    days: u32,
    months: u16,
    weekdays: u8,
    any_day: bool,
    any_weekday: bool,
}

impl Cron {
    /// The times `text` says.
    ///
    /// # Errors
    ///
    /// When `text` is not five fields cron knows.
    pub fn parse(text: &str) -> Result<Self, String> {
        let fields: Vec<&str> = text.split_whitespace().collect();
        let [minutes, hours, days, months, weekdays] = fields.as_slice() else {
            return Err(format!(
                "{text:?} is no cron: five fields, minute, hour, day, month and day of \
                 the week, as \"0 9 * * *\""
            ));
        };
        let said = |why: String| format!("{text:?}: {why}");
        let weekday_bits = field(weekdays, 0, 7, &DAYS, 0).map_err(said)?;
        Ok(Self {
            text: fields.join(" "),
            minutes: field(minutes, 0, 59, &[], 0).map_err(said)?,
            hours: u32::try_from(field(hours, 0, 23, &[], 0).map_err(said)?).unwrap_or(0),
            days: u32::try_from(field(days, 1, 31, &[], 0).map_err(said)?).unwrap_or(0),
            months: u16::try_from(field(months, 1, 12, &MONTHS, 1).map_err(said)?).unwrap_or(0),
            // Sunday is 0, and 7 too.
            weekdays: u8::try_from((weekday_bits | (weekday_bits >> 7)) & 0x7f).unwrap_or(0),
            any_day: days.starts_with('*'),
            any_weekday: weekdays.starts_with('*'),
        })
    }

    /// The cron of what an earlier Cox's schedule said with `every`:
    /// `daily HH:MM`, `weekly <day> HH:MM`, or `every <n>h`, which runs
    /// on the hours a multiple of `n` from midnight, as the clock has
    /// them, rather than `n` hours after it last ran.
    ///
    /// # Errors
    ///
    /// When `text` is none of those.
    pub fn from_every(text: &str) -> Result<Self, String> {
        let words: Vec<&str> = text.split_whitespace().collect();
        let time = |text: &str| {
            Time::strptime("%H:%M", text).map_err(|_| format!("{text:?} is no time, as 09:00"))
        };
        let cron = match words.as_slice() {
            ["daily", at] => {
                let at = time(at)?;
                format!("{} {} * * *", at.minute(), at.hour())
            }
            ["weekly", day, at] => {
                let at = time(at)?;
                let day = weekday(day)?.to_sunday_zero_offset();
                format!("{} {} * * {day}", at.minute(), at.hour())
            }
            ["every", hours] => {
                let hours = hours
                    .strip_suffix('h')
                    .and_then(|hours| hours.parse::<u32>().ok())
                    .filter(|&hours| hours > 0)
                    .ok_or_else(|| format!("{hours:?} is no number of hours, as 6h"))?;
                match hours {
                    1 => "0 * * * *".to_owned(),
                    2..24 => format!("0 */{hours} * * *"),
                    _ if hours % 24 == 0 => format!("0 0 */{} * *", hours / 24),
                    _ => return Err(format!("every {hours}h is neither under a day nor days")),
                }
            }
            _ => {
                return Err(format!(
                    "{text:?} says no time Cox knows: daily 09:00, weekly mon 09:00 or every 6h"
                ));
            }
        };
        Self::parse(&cron)
    }

    /// Whether it runs on `date`.
    fn on(&self, date: Date) -> bool {
        let month = self.months & (1 << date.month()) != 0;
        let day = self.days & (1 << date.day()) != 0;
        let weekday = self.weekdays & (1 << date.weekday().to_sunday_zero_offset()) != 0;
        month
            && match (self.any_day, self.any_weekday) {
                (true, true) => true,
                (true, false) => weekday,
                (false, true) => day,
                (false, false) => day || weekday,
            }
    }

    /// The first time it says after `after`, in `zone`.
    #[must_use]
    pub fn after(&self, after: Timestamp, zone: &TimeZone) -> Option<Zoned> {
        let first = after.to_zoned(zone.clone()).date();
        for ahead in 0..=AHEAD {
            let date = first.checked_add(Span::new().days(ahead)).ok()?;
            if !self.on(date) {
                continue;
            }
            for hour in (0..24i8).filter(|&hour| self.hours & (1 << hour) != 0) {
                for minute in (0..60i8).filter(|&minute| self.minutes & (1 << minute) != 0) {
                    let then = date.at(hour, minute, 0, 0).to_zoned(zone.clone()).ok()?;
                    if then.timestamp() > after {
                        return Some(then);
                    }
                }
            }
        }
        None
    }

    /// Whether a schedule that ran `last`, if it did, is due `now`: when
    /// a time it says is past since. One that never ran is due at once,
    /// for the human to see what it does.
    #[must_use]
    pub fn due(&self, last: Option<Timestamp>, now: &Zoned) -> bool {
        let Some(last) = last else {
            return true;
        };
        self.after(last, now.time_zone())
            .is_some_and(|next| next.timestamp() <= now.timestamp())
    }

    /// When a schedule that ran `last`, if it did, runs next: `now`,
    /// when it is due.
    #[must_use]
    pub fn next(&self, last: Option<Timestamp>, now: &Zoned) -> Option<Zoned> {
        if self.due(last, now) {
            return Some(now.clone());
        }
        self.after(last?, now.time_zone())
    }

    /// It in words, as the stage shows it, as `Daily 09:00`, or else as
    /// cron writes it.
    #[must_use]
    pub fn words(&self) -> String {
        let fields: Vec<&str> = self.text.split(' ').collect();
        let [minute, hour, day, month, weekday] = fields.as_slice() else {
            return self.text.clone();
        };
        let number = |field: &str| field.parse::<u8>().ok();
        let every = |field: &str| field.strip_prefix("*/").and_then(number);
        let any = |field: &&str| *field == "*";
        let at = number(hour)
            .zip(number(minute))
            .map(|(hour, minute)| format!("{hour:02}:{minute:02}"));
        let days = || {
            const NAMES: [&str; 7] = ["Sun", "Mon", "Tue", "Wed", "Thu", "Fri", "Sat"];
            (0..7)
                .filter(|day| self.weekdays & (1 << day) != 0)
                .map(|day| NAMES[day])
                .collect::<Vec<_>>()
        };
        match (at, any(day), any(month), any(weekday)) {
            (Some(at), true, true, true) => format!("Daily {at}"),
            (Some(at), true, true, false) => match days().as_slice() {
                ["Mon", "Tue", "Wed", "Thu", "Fri"] => format!("Weekdays {at}"),
                [one] => format!("Weekly {one} {at}"),
                many => format!("{} {at}", many.join(", ")),
            },
            (None, true, true, true) => match (every(hour), number(minute), every(minute)) {
                (Some(hours), Some(0), _) => format!("Every {hours} hours"),
                (Some(hours), Some(minute), _) => format!("Every {hours} hours at :{minute:02}"),
                (None, Some(minute), _) if any(hour) => format!("Hourly at :{minute:02}"),
                (None, None, Some(minutes)) if any(hour) => format!("Every {minutes} minutes"),
                _ => self.text.clone(),
            },
            _ => self.text.clone(),
        }
    }
}

impl std::fmt::Display for Cron {
    /// As cron writes it.
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.text)
    }
}

impl serde::Serialize for Cron {
    fn serialize<S: serde::Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        serializer.serialize_str(&self.text)
    }
}

/// The values a field of a cron says, from `min` to `max`, as bits:
/// names, if it has them, standing for `offset` and those after it.
fn field(text: &str, min: u32, max: u32, names: &[&str], offset: u32) -> Result<u64, String> {
    let value = |text: &str| {
        names
            .iter()
            .position(|name| name.eq_ignore_ascii_case(text))
            .and_then(|at| u32::try_from(at).ok())
            .map(|at| at + offset)
            .or_else(|| text.parse().ok())
            .filter(|value| (min..=max).contains(value))
            .ok_or_else(|| format!("{text:?} is not from {min} to {max}"))
    };
    let mut bits = 0u64;
    for item in text.split(',') {
        let (range, step) = match item.split_once('/') {
            Some((range, step)) => (
                range,
                step.parse::<u32>()
                    .ok()
                    .filter(|&step| step > 0)
                    .ok_or_else(|| format!("{step:?} is no step"))?,
            ),
            None => (item, 1),
        };
        let (from, to) = match range.split_once('-') {
            _ if range == "*" => (min, max),
            Some((from, to)) => (value(from)?, value(to)?),
            // `5/15` goes from 5 on.
            None if step > 1 => (value(range)?, max),
            None => {
                let at = value(range)?;
                (at, at)
            }
        };
        if from > to {
            return Err(format!("{range:?} goes backwards"));
        }
        for at in (from..=to).step_by(usize::try_from(step).unwrap_or(1)) {
            bits |= 1 << at;
        }
    }
    Ok(bits)
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

/// What the schedule of `file`, named `name`, is kept by apart from
/// every other: its name, for one an earlier Cox kept in the `schedules`
/// of the human's `workflow`, as it kept them, and else its file's path.
#[must_use]
pub fn id(name: &str, file: &Path, workflow: &Path) -> String {
    if file.parent() == Some(&workflow.join(SCHEDULES)) {
        name.to_owned()
    } else {
        file.display().to_string()
    }
}

/// The first line of the run `now` of the schedule `name`, whose text is
/// `text`, which ran last `last`, if it did: `{name}`, `{now}` and
/// `{last}` standing for its name, when it runs and when it ran last, or
/// `never`.
#[must_use]
pub fn line(text: &str, name: &str, now: &Zoned, last: Option<Timestamp>) -> String {
    let at = |when: &Zoned| when.strftime("%Y-%m-%d %H:%M").to_string();
    let last = last.map_or_else(
        || "never".to_owned(),
        |last| at(&last.to_zoned(now.time_zone().clone())),
    );
    crate::workflow::fill(text, &[("name", name), ("now", &at(now)), ("last", &last)])
}

/// What the chat of the schedule `name` is named, as it runs `now`.
#[must_use]
pub fn chat_name(name: &str, now: &Zoned) -> String {
    format!("{name} {}", now.strftime("%m/%d %H:%M"))
}

/// When each schedule last ran, by what it is kept by (see [`id`]), as
/// Cox keeps it in the file at `path`.
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

    /// When the schedule `id` last ran.
    #[must_use]
    pub fn last(&self, id: &str) -> Option<Timestamp> {
        self.last.get(id).copied()
    }

    /// Keeps that the schedule `id` ran `now`.
    pub fn ran(&mut self, id: &str, now: Timestamp) {
        self.last.insert(id.to_owned(), now);
        let Some(path) = &self.path else {
            return;
        };
        let kept: BTreeMap<&str, String> = self
            .last
            .iter()
            .map(|(id, when)| (id.as_str(), when.to_string()))
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
    use jiff::civil::date;
    use jiff::tz::TimeZone;

    use super::{Cron, Ran};

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

    fn cron(text: &str) -> Cron {
        Cron::parse(text).unwrap()
    }

    #[test]
    fn a_cron_says_when_in_five_fields() {
        assert!(Cron::parse("0 9 * *").is_err());
        assert!(Cron::parse("60 9 * * *").is_err());
        assert!(Cron::parse("0 9 * * 8").is_err());
        assert!(Cron::parse("0 9-5 * * *").is_err());
        assert!(Cron::parse("*/0 * * * *").is_err());
        assert_eq!(cron("0  9 * * *").to_string(), "0 9 * * *");
        // Sunday is 0 and 7, and days and months go by their names too.
        assert_eq!(
            cron("0 9 * * 7"),
            Cron {
                text: "0 9 * * 7".to_owned(),
                ..cron("0 9 * * 0")
            }
        );
        assert_eq!(
            cron("0 9 * jan sun").after(ran(2, 0, 0).unwrap(), at(2, 0, 0).time_zone()),
            Some(
                date(2027, 1, 3)
                    .at(9, 0, 0, 0)
                    .to_zoned(TimeZone::fixed(jiff::tz::offset(9)))
                    .unwrap()
            )
        );
    }

    #[test]
    fn a_cron_runs_next_at_the_first_time_it_says() {
        let zone = at(2, 0, 0).time_zone().clone();
        let after = |text: &str, day, hour, minute| {
            cron(text).after(ran(day, hour, minute).unwrap(), &zone)
        };
        assert_eq!(after("0 9 * * *", 3, 8, 0), Some(at(3, 9, 0)));
        assert_eq!(after("0 9 * * *", 3, 9, 0), Some(at(4, 9, 0)));
        assert_eq!(after("0 */6 * * *", 3, 7, 0), Some(at(3, 12, 0)));
        assert_eq!(after("*/15 * * * *", 3, 7, 1), Some(at(3, 7, 15)));
        // Weekdays: from Friday to Monday.
        assert_eq!(after("30 8 * * 1-5", 6, 9, 0), Some(at(9, 8, 30)));
        // Both days given: either runs it.
        assert_eq!(after("0 0 15 * 3", 3, 1, 0), Some(at(4, 0, 0)));
        assert_eq!(after("0 0 1,15 * *", 3, 1, 0), Some(at(15, 0, 0)));
    }

    #[test]
    fn a_schedule_runs_once_its_time_is_past_however_many_were_missed() {
        let daily = cron("0 9 * * *");
        // Never ran: at once.
        assert!(daily.due(None, &at(3, 8, 59)));
        // Ran at nine: not again until nine tomorrow.
        assert!(!daily.due(ran(3, 9, 0), &at(3, 23, 0)));
        assert!(daily.due(ran(3, 9, 0), &at(4, 9, 1)));
        // Missed for days, it runs once, and then waits.
        assert!(daily.due(ran(1, 9, 0), &at(5, 7, 0)));
        assert!(!daily.due(ran(5, 7, 0), &at(5, 8, 0)));
        assert_eq!(daily.next(None, &at(3, 8, 0)), Some(at(3, 8, 0)));
        assert_eq!(daily.next(ran(3, 9, 0), &at(3, 10, 0)), Some(at(4, 9, 0)));
    }

    #[test]
    fn what_an_earlier_cox_said_every_is_a_cron() {
        let every = |text: &str| Cron::from_every(text).map(|cron| cron.to_string());
        assert_eq!(every("daily 09:00").as_deref(), Ok("0 9 * * *"));
        assert_eq!(every("weekly Mon 18:30").as_deref(), Ok("30 18 * * 1"));
        assert_eq!(every("every 6h").as_deref(), Ok("0 */6 * * *"));
        assert_eq!(every("every 1h").as_deref(), Ok("0 * * * *"));
        assert_eq!(every("every 48h").as_deref(), Ok("0 0 */2 * *"));
        assert!(every("every 0h").is_err());
        assert!(every("every 30h").is_err());
        assert!(every("daily 9am").is_err());
        assert!(every("hourly").is_err());
    }

    #[test]
    fn a_cron_is_shown_in_words_where_it_has_them() {
        let words = |text: &str| cron(text).words();
        assert_eq!(words("0 9 * * *"), "Daily 09:00");
        assert_eq!(words("30 8 * * 1-5"), "Weekdays 08:30");
        assert_eq!(words("0 9 * * mon"), "Weekly Mon 09:00");
        assert_eq!(words("0 9 * * 1,3"), "Mon, Wed 09:00");
        assert_eq!(words("0 */6 * * *"), "Every 6 hours");
        assert_eq!(words("15 * * * *"), "Hourly at :15");
        assert_eq!(words("*/10 * * * *"), "Every 10 minutes");
        assert_eq!(words("0 9 1 * *"), "0 9 1 * *");
    }

    #[test]
    fn a_schedule_of_an_earlier_cox_is_kept_by_its_name() {
        let workflow = Path::new("/w");
        assert_eq!(
            super::id("news", Path::new("/w/schedules/news.md"), workflow),
            "news"
        );
        assert_eq!(
            super::id("news", Path::new("/w/triggers/news.md"), workflow),
            "/w/triggers/news.md"
        );
        assert_eq!(
            super::line(
                "Since {last}, as {name}.",
                "news",
                &at(4, 9, 0),
                ran(3, 9, 30)
            ),
            "Since 2026-11-03 09:30, as news."
        );
        assert_eq!(super::line("{last}", "news", &at(4, 9, 0), None), "never");
        assert_eq!(super::chat_name("news", &at(3, 9, 30)), "news 11/03 09:30");
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
