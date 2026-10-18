//! Scheduled requests: work the agent does later, without being asked.
//!
//! A job is one request plus a time. The time is stored as local RFC3339; a
//! repeating job also carries its interval in seconds. The file lives beside
//! the configuration, so the schedule survives a restart and a missed
//! occurrence can be noticed the next time the server starts.

use anyhow::{Context, Result};
use chrono::{DateTime, Local, NaiveDate, NaiveDateTime, NaiveTime, TimeZone};
use serde::{Deserialize, Serialize};
use std::path::PathBuf;

/// One scheduled request.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Job {
    pub id: String,
    /// Short human name, used in the list and in notifications.
    pub name: String,
    /// What the agent is asked to do when the job fires. It runs as a normal
    /// prompt, with the session's tools, in its own session.
    pub request: String,
    /// Local RFC3339 of the next run (the only run, for a one-shot).
    pub at: String,
    /// Seconds between runs; `None` is a one-shot.
    #[serde(default)]
    pub every: Option<u64>,
    /// Directory the run works in.
    pub directory: String,
    /// The session the runs land in, created on the first run.
    #[serde(default)]
    pub session: Option<String>,
    #[serde(default)]
    pub created: String,
    #[serde(default)]
    pub last_run: Option<String>,
    /// Short outcome of the last run: `ok: <summary>` or `failed: <reason>`.
    #[serde(default)]
    pub last_status: Option<String>,
    #[serde(default)]
    pub runs: u32,
    #[serde(default)]
    pub paused: bool,
    /// A one-shot that has already run. Kept so `/schedule` can show it.
    #[serde(default)]
    pub done: bool,
}

/// A line the user has not seen yet: a finished run, or one that was missed
/// while nothing was running.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Notice {
    pub id: String,
    pub text: String,
}

/// Everything in `schedules.json`.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(default)]
pub struct Schedules {
    pub jobs: Vec<Job>,
    /// Unread notices, delivered the next time a client is attached.
    pub pending: Vec<Notice>,
}

/// `~/.config/exp-1/schedules.json`.
pub fn path() -> PathBuf {
    crate::store::config_dir().join("schedules.json")
}

pub fn load() -> Schedules {
    std::fs::read_to_string(path())
        .ok()
        .and_then(|raw| serde_json::from_str(&raw).ok())
        .unwrap_or_default()
}

pub fn save(schedules: &Schedules) -> Result<()> {
    let path = path();
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent).with_context(|| format!("create {}", parent.display()))?;
    }
    let json = serde_json::to_vec_pretty(schedules)?;
    let tmp = path.with_extension("tmp");
    std::fs::write(&tmp, &json).with_context(|| format!("write {}", tmp.display()))?;
    std::fs::rename(&tmp, &path).with_context(|| format!("replace {}", path.display()))?;
    Ok(())
}

// ------------------------------------------------------------------ parsing

/// Reads a time the model wrote. Accepted shapes, in order: RFC3339, a local
/// `YYYY-MM-DD HH:MM[:SS]` (with `T` or a space), a bare `HH:MM` (today, or
/// tomorrow when that moment has passed), a bare date (09:00 that day), and a
/// relative `in 5m` / `+90m`.
pub fn parse_when(text: &str, now: DateTime<Local>) -> Result<DateTime<Local>, String> {
    let raw = text.trim();
    if raw.is_empty() {
        return Err("a time is required".to_string());
    }
    if let Ok(stamp) = DateTime::parse_from_rfc3339(raw) {
        return Ok(stamp.with_timezone(&Local));
    }
    let lowered = raw.to_ascii_lowercase();
    for prefix in ["in ", "after ", "+"] {
        if let Some(rest) = lowered.strip_prefix(prefix) {
            let seconds = parse_duration(rest)?;
            return Ok(now + chrono::Duration::seconds(seconds as i64));
        }
    }
    let body = raw.replace('T', " ");
    for format in ["%Y-%m-%d %H:%M:%S", "%Y-%m-%d %H:%M"] {
        if let Ok(naive) = NaiveDateTime::parse_from_str(&body, format) {
            return Ok(local(naive, now));
        }
    }
    if let Ok(time) = NaiveTime::parse_from_str(&body, "%H:%M") {
        let mut candidate = local(now.date_naive().and_time(time), now);
        if candidate <= now {
            candidate += chrono::Duration::days(1);
        }
        return Ok(candidate);
    }
    if let Ok(date) = NaiveDate::parse_from_str(&body, "%Y-%m-%d") {
        // A date without a time means the start of that working day.
        let time = NaiveTime::from_hms_opt(9, 0, 0).unwrap_or_default();
        return Ok(local(date.and_time(time), now));
    }
    Err(format!(
        "'{text}' is not a time this understands — use a local timestamp like 2026-10-18T09:00"
    ))
}

/// A naive local time to a real instant. A time inside a DST gap does not
/// exist; the later reading of the wall clock is the one people mean.
fn local(naive: NaiveDateTime, now: DateTime<Local>) -> DateTime<Local> {
    let _ = now;
    match Local.from_local_datetime(&naive) {
        chrono::LocalResult::Single(time) => time,
        chrono::LocalResult::Ambiguous(first, _) => first,
        chrono::LocalResult::None => Local
            .from_local_datetime(&(naive + chrono::Duration::hours(1)))
            .earliest()
            .unwrap_or_else(|| Local.from_utc_datetime(&naive).with_timezone(&Local)),
    }
}

/// A repeat interval: `30m`, `2h`, `1d`, `1w`, a bare number of minutes, or the
/// same in words (`every day`, `2 hours`). Returns seconds, never below a
/// minute: a job that fires every few seconds is a runaway, not a schedule.
pub fn parse_every(text: &str) -> Result<u64, String> {
    let seconds = parse_duration(text)?;
    if seconds < 60 {
        return Ok(60);
    }
    Ok(seconds)
}

/// The same grammar without the floor, for waits such as `in 90s`.
fn parse_duration(text: &str) -> Result<u64, String> {
    let lowered = text.trim().to_ascii_lowercase();
    let lowered = lowered.strip_prefix("every ").unwrap_or(&lowered).trim();
    let lowered = lowered
        .trim_start_matches("a ")
        .trim_start_matches("an ")
        .trim();
    let digits: String = lowered
        .chars()
        .take_while(|ch| ch.is_ascii_digit() || *ch == '.')
        .collect();
    let rest = lowered[digits.len()..].trim();
    if digits.is_empty() {
        // A word without a number: "day", "week", "hour".
        let unit = unit_seconds(rest).or_else(|| unit_seconds(lowered));
        return unit.ok_or_else(|| format!("'{text}' is not an interval this understands"));
    }
    let value: f64 = digits
        .parse()
        .map_err(|_| format!("'{text}' is not an interval this understands"))?;
    let unit = if rest.is_empty() {
        // A bare number is minutes, the unit people schedule in.
        60.0
    } else {
        unit_seconds(rest).ok_or_else(|| {
            format!("'{text}': unknown unit '{rest}' — use s, m, h, d or w")
        })? as f64
    };
    Ok((value * unit).round().max(1.0) as u64)
}

fn unit_seconds(text: &str) -> Option<u64> {
    match text.trim() {
        "s" | "sec" | "secs" | "second" | "seconds" => Some(1),
        "m" | "min" | "mins" | "minute" | "minutes" => Some(60),
        "h" | "hr" | "hrs" | "hour" | "hours" => Some(3_600),
        "d" | "day" | "days" => Some(86_400),
        "w" | "week" | "weeks" => Some(604_800),
        _ => None,
    }
}

/// A short, readable interval: `30m`, `2h`, `1d`.
pub fn humanize(seconds: u64) -> String {
    if seconds % 604_800 == 0 {
        return format!("{}w", seconds / 604_800);
    }
    if seconds % 86_400 == 0 {
        return format!("{}d", seconds / 86_400);
    }
    if seconds % 3_600 == 0 {
        return format!("{}h", seconds / 3_600);
    }
    if seconds % 60 == 0 {
        return format!("{}m", seconds / 60);
    }
    format!("{seconds}s")
}

/// The local `18 Oct 14:03` a stored timestamp means.
pub fn when_text(at: &str) -> String {
    DateTime::parse_from_rfc3339(at)
        .map(|time| time.with_timezone(&Local).format("%d %b %H:%M").to_string())
        .unwrap_or_else(|_| at.to_string())
}

impl Job {
    /// The moment the next run is due, if the stored stamp parses.
    pub fn due_at(&self) -> Option<DateTime<Local>> {
        DateTime::parse_from_rfc3339(&self.at)
            .ok()
            .map(|time| time.with_timezone(&Local))
    }

    pub fn is_due(&self, now: DateTime<Local>) -> bool {
        if self.paused || self.done {
            return false;
        }
        self.due_at().is_some_and(|at| at <= now)
    }

    /// Occurrences that passed while nothing was running. Advancing stops when
    /// the next one is in the future.
    pub fn advance(&mut self, now: DateTime<Local>) -> u32 {
        let Some(every) = self.every.filter(|every| *every >= 60) else {
            return 0;
        };
        let mut missed = 0;
        while let Some(at) = self.due_at() {
            if at > now {
                break;
            }
            let next = at + chrono::Duration::seconds(every as i64);
            self.at = next.to_rfc3339();
            missed += 1;
            if missed >= 10_000 {
                // A job left alone for years: jump to the next future round
                // rather than walking every one of them.
                let jump = ((now - at).num_seconds() / every as i64) + 1;
                let skipped = jump.max(1) as u32;
                self.at = (at + chrono::Duration::seconds(every as i64 * jump as i64))
                    .to_rfc3339();
                missed += skipped;
                break;
            }
        }
        missed
    }

    /// The row `/schedule` prints.
    pub fn line(&self) -> String {
        let when = if self.done {
            "done".to_string()
        } else if self.paused {
            format!("paused · due {}", when_text(&self.at))
        } else if let Some(every) = self.every {
            format!("every {} · next {}", humanize(every), when_text(&self.at))
        } else {
            format!("once · due {}", when_text(&self.at))
        };
        let last = self
            .last_status
            .as_ref()
            .map(|status| format!(" · last {}", status))
            .unwrap_or_default();
        let where_ = self
            .session
            .as_ref()
            .map(|_| " · own session".to_string())
            .unwrap_or_default();
        format!(
            "{:<8} {} ({}) — {when}{last}{where_}",
            self.id, self.name, self.request
        )
    }
}

impl Schedules {
    /// The index of a job by id, or by a name that matches exactly once.
    /// `/schedule rm 3f2a` and `/schedule rm "push to github"` both work.
    pub fn position(&self, key: &str) -> Option<usize> {
        self.jobs.iter().position(|job| job.id == key).or_else(|| {
            let matches: Vec<usize> = self
                .jobs
                .iter()
                .enumerate()
                .filter(|(_, job)| job.name.eq_ignore_ascii_case(key))
                .map(|(index, _)| index)
                .collect();
            (matches.len() == 1).then(|| matches[0])
        })
    }

    /// An id nobody else holds: short enough to type, unique on one machine.
    pub fn fresh_id(&self) -> String {
        for _ in 0..64 {
            let id = &crate::daemon::random_id()[..8];
            if !self.jobs.iter().any(|job| job.id == id) {
                return id.to_string();
            }
        }
        crate::daemon::random_id()[..8].to_string()
    }

    /// The next run that is due, skipping paused and finished jobs.
    pub fn due(&self, now: DateTime<Local>) -> Option<String> {
        self.jobs
            .iter()
            .filter(|job| job.is_due(now))
            .min_by_key(|job| job.due_at())
            .map(|job| job.id.clone())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn at(text: &str) -> DateTime<Local> {
        DateTime::parse_from_rfc3339(text)
            .expect("stamp")
            .with_timezone(&Local)
    }

    #[test]
    fn a_time_can_be_written_in_several_shapes() {
        let now = at("2026-10-18T14:03:00+03:00");
        // RFC3339 keeps its own offset.
        assert_eq!(
            parse_when("2026-10-18T09:00:00+03:00", now).unwrap().timestamp(),
            at("2026-10-18T09:00:00+03:00").timestamp()
        );
        // A local stamp is read as local.
        assert_eq!(
            parse_when("2026-10-18T09:00", now).unwrap().timestamp(),
            at("2026-10-18T09:00:00+03:00").timestamp()
        );
        assert_eq!(
            parse_when("2026-10-18 09:00", now).unwrap().timestamp(),
            at("2026-10-18T09:00:00+03:00").timestamp()
        );
        // A bare time is today, or tomorrow once it has passed.
        assert_eq!(
            parse_when("23:30", now).unwrap().date_naive(),
            now.date_naive()
        );
        assert_eq!(
            parse_when("09:00", now).unwrap().date_naive(),
            now.date_naive() + chrono::Duration::days(1)
        );
        // A relative wait.
        assert_eq!(parse_when("in 5m", now).unwrap().timestamp(), now.timestamp() + 300);
        assert_eq!(parse_when("+90m", now).unwrap().timestamp(), now.timestamp() + 5_400);
        // A bare date means the start of that working day.
        let day = parse_when("2026-12-01", now).unwrap();
        assert_eq!(day.format("%H:%M").to_string(), "09:00");
        assert!(parse_when("whenever", now).is_err());
    }

    #[test]
    fn an_interval_can_be_written_in_several_shapes() {
        assert_eq!(parse_every("30m").unwrap(), 1_800);
        assert_eq!(parse_every("2h").unwrap(), 7_200);
        assert_eq!(parse_every("1d").unwrap(), 86_400);
        assert_eq!(parse_every("1w").unwrap(), 604_800);
        assert_eq!(parse_every("every day").unwrap(), 86_400);
        assert_eq!(parse_every("2 hours").unwrap(), 7_200);
        // A bare number is minutes.
        assert_eq!(parse_every("90").unwrap(), 5_400);
        assert_eq!(parse_every("1.5h").unwrap(), 5_400);
        // Anything faster than a minute is raised to a minute.
        assert_eq!(parse_every("5s").unwrap(), 60);
        assert!(parse_every("soon").is_err());
    }

    #[test]
    fn a_daily_job_left_alone_reports_every_missed_run() {
        let now = at("2026-10-18T14:03:00+03:00");
        let mut job = Job {
            id: "abc12345".to_string(),
            name: "push".to_string(),
            request: "push the project".to_string(),
            at: "2026-10-15T09:00:00+03:00".to_string(),
            every: Some(86_400),
            directory: "/tmp".to_string(),
            session: None,
            created: String::new(),
            last_run: None,
            last_status: None,
            runs: 0,
            paused: false,
            done: false,
        };
        assert!(job.is_due(now));
        let missed = job.advance(now);
        assert_eq!(missed, 4, "15,16,17 and 18 October all passed");
        assert!(!job.is_due(now), "the next run is in the future");
        assert_eq!(when_text(&job.at), "19 Oct 09:00");
    }

    #[test]
    fn a_paused_or_finished_job_never_fires() {
        let now = at("2026-10-18T14:03:00+03:00");
        let mut job = Job {
            id: "abc12345".to_string(),
            name: "timer".to_string(),
            request: "remind me".to_string(),
            at: "2026-10-18T14:00:00+03:00".to_string(),
            every: None,
            directory: "/tmp".to_string(),
            session: None,
            created: String::new(),
            last_run: None,
            last_status: None,
            runs: 0,
            paused: true,
            done: false,
        };
        assert!(!job.is_due(now));
        job.paused = false;
        assert!(job.is_due(now));
        job.done = true;
        assert!(!job.is_due(now));
        // A one-shot has nothing to advance.
        job.done = false;
        assert_eq!(job.advance(now), 0);
    }

    #[test]
    fn a_job_is_found_by_id_or_by_a_unique_name() {
        let mut schedules = Schedules::default();
        for name in ["push to github", "weekly news"] {
            schedules.jobs.push(Job {
                id: schedules.fresh_id(),
                name: name.to_string(),
                request: name.to_string(),
                at: "2026-10-19T09:00:00+03:00".to_string(),
                every: None,
                directory: "/tmp".to_string(),
                session: None,
                created: String::new(),
                last_run: None,
                last_status: None,
                runs: 0,
                paused: false,
                done: false,
            });
        }
        let first = schedules.jobs[0].id.clone();
        // The second job is not due at the moment the first one is, so the
        // picker below has only one candidate to offer.
        schedules.jobs[1].at = "2026-10-20T09:00:00+03:00".to_string();
        assert_eq!(schedules.position(&first), Some(0));
        assert_eq!(schedules.position("PUSH TO GITHUB"), Some(0));
        assert!(schedules.position("nothing like this").is_none());
        // Ids are short and unique.
        assert_eq!(first.len(), 8);
        assert_ne!(schedules.jobs[0].id, schedules.jobs[1].id);
        // The row prints what the picker needs.
        let line = schedules.jobs[0].line();
        assert!(line.contains("push to github"), "{line}");
        assert!(line.contains("19 Oct 09:00"), "{line}");
        // Only the due job is offered, and a paused one never is.
        let now = at("2026-10-19T10:00:00+03:00");
        assert_eq!(schedules.due(now), Some(first.clone()));
        schedules.jobs[0].paused = true;
        assert_eq!(schedules.due(now), None);
    }
}
