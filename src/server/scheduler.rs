//! The clock behind `/schedule`: jobs due, runs unattended, notices queued.
//!
//! The scheduler lives in the server process, which is the only long-lived one,
//! so a job still fires with every terminal closed. Runs happen in the job's own
//! session, one at a time; a notice about each run waits in `schedules.json`
//! until a client is attached to read it.

use std::collections::HashSet;
use std::path::PathBuf;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use chrono::Local;

use crate::proto::WireEvent;
use crate::schedule;
use crate::tools;

use super::*;

/// How often the schedule is looked at. A few seconds of slack on a timer is
/// not worth waking the process every tick for.
const TICK: Duration = Duration::from_secs(5);

/// The state behind `/schedule`, held by the server.
pub(crate) struct Scheduler {
    pub(crate) state: Mutex<schedule::Schedules>,
    /// Jobs whose run is in flight, so nothing starts one twice.
    pub(crate) running: Mutex<HashSet<String>>,
}

impl Scheduler {
    pub(crate) fn load() -> Self {
        Self {
            state: Mutex::new(schedule::load()),
            running: Mutex::new(HashSet::new()),
        }
    }

    /// Every job, as the `/schedule` list shows it.
    pub(crate) fn jobs(&self) -> Vec<schedule::Job> {
        self.state
            .lock()
            .map(|state| state.jobs.clone())
            .unwrap_or_default()
    }
}

/// Starts the scheduler: first what was missed, then the clock.
pub(crate) fn spawn_scheduler(shared: Arc<Shared>) {
    tokio::spawn(async move {
        catch_up(shared.clone()).await;
        let mut tick = tokio::time::interval(TICK);
        loop {
            tick.tick().await;
            flush_notices(&shared);
            {
                // One run at a time: five due jobs are not five agents hitting
                // the same provider at once.
                let busy = shared
                    .scheduler
                    .running
                    .lock()
                    .map(|running| !running.is_empty())
                    .unwrap_or(true);
                if busy {
                    continue;
                }
            }
            let due: Option<String> = shared
                .scheduler
                .state
                .lock()
                .ok()
                .and_then(|state| state.due(Local::now()));
            if let Some(id) = due {
                fire_job(&shared, &id, None);
            }
        }
    });
}

/// Runs at startup: every job whose moment passed while nothing was running is
/// advanced past it, reported, and started.
async fn catch_up(shared: Arc<Shared>) {
    let now = Local::now();
    let mut overdue: Vec<String> = Vec::new();
    let mut notices: Vec<schedule::Notice> = Vec::new();
    {
        let Ok(mut state) = shared.scheduler.state.lock() else {
            return;
        };
        for job in state.jobs.iter_mut() {
            if job.paused || job.done {
                continue;
            }
            let Some(at) = job.due_at() else {
                continue;
            };
            if at > now {
                continue;
            }
            let was = schedule::when_text(&job.at);
            let repeats = job.every.is_some();
            let missed = job.advance(now).max(1);
            let text = if repeats {
                format!(
                    "⏱ «{}» was due {was} while exp-1 was not running ({missed} missed) · running it now",
                    job.name
                )
            } else {
                format!(
                    "⏱ «{}» was due {was} while exp-1 was not running · running it now",
                    job.name
                )
            };
            notices.push(schedule::Notice {
                id: crate::daemon::random_id(),
                text,
            });
            overdue.push(job.id.clone());
        }
        if !notices.is_empty() {
            state.pending.extend(notices);
            let _ = schedule::save(&state);
        }
    }
    if let Some(id) = overdue.first() {
        fire_job(&shared, id, None);
    }
}

/// Starts one job now. The run happens in its own task; the caller keeps its
/// turn. `why` is the line to show when the start itself is news, such as a
/// missed occurrence being caught up.
pub(crate) fn fire_job(shared: &Arc<Shared>, id: &str, why: Option<String>) {
    let shared = shared.clone();
    let id = id.to_string();
    {
        let Ok(mut running) = shared.scheduler.running.lock() else {
            return;
        };
        if !running.insert(id.clone()) {
            return;
        }
    }
    let job = {
        let Ok(mut state) = shared.scheduler.state.lock() else {
            release(&shared, &id);
            return;
        };
        if let Some(text) = why {
            state.pending.push(schedule::Notice {
                id: crate::daemon::random_id(),
                text,
            });
        }
        let Some(job) = state.jobs.iter_mut().find(|job| job.id == id) else {
            release(&shared, &id);
            return;
        };
        if job.paused || (job.every.is_none() && job.done) {
            let _ = schedule::save(&state);
            release(&shared, &id);
            return;
        }
        // A one-shot is spent the moment it fires; a repeat was already moved
        // past this occurrence by `advance`, on the tick that found it due.
        if job.every.is_none() {
            job.done = true;
        }
        job.last_run = Some(store::now_iso());
        let job = job.clone();
        let _ = schedule::save(&state);
        job
    };
    tokio::spawn(async move {
        run_job(shared.clone(), job).await;
        release(&shared, &id);
    });
}

fn release(shared: &Arc<Shared>, id: &str) {
    if let Ok(mut running) = shared.scheduler.running.lock() {
        running.remove(id);
    }
}

/// One unattended run: its own session, tools allowed, the outcome written down
/// and announced.
async fn run_job(shared: Arc<Shared>, job: schedule::Job) {
    let started = Instant::now();
    let session = job_session(&shared, &job).await;
    session.push(
        "system",
        format!(
            "⏱ scheduled run {} · {}",
            job.runs + 1,
            Local::now().format("%d %b %H:%M")
        ),
    );
    run_turn(shared.clone(), session.clone(), job.request.clone(), Vec::new()).await;
    // `run_turn` persists on its way out, but a failed request returns early;
    // a run nobody can read afterwards would be worse than a slow one.
    persist(&session).await;

    let (error, summary) = {
        let state = session.state.lock().expect("session state");
        let error = state.error.clone();
        let summary = state
            .transcript
            .iter()
            .rev()
            .find(|message| message.role == "assistant")
            .map(|message| {
                let first = message.text.lines().next().unwrap_or_default().trim();
                let mut out: String = first.chars().take(140).collect();
                if first.chars().count() > 140 {
                    out.push('…');
                }
                out
            })
            .unwrap_or_default();
        (error, summary)
    };
    let seconds = started.elapsed().as_secs();
    let human = if seconds >= 60 {
        format!("{}m {}s", seconds / 60, seconds % 60)
    } else {
        format!("{seconds}s")
    };
    let status = match error {
        Some(message) => format!("failed — {}", one_line(&message)),
        None if summary.is_empty() => format!("finished in {human}"),
        None => format!("finished in {human} — {summary}"),
    };

    record(&shared, &job.id, &status, &session.id);
    queue_notice(&shared, format!("⏱ «{}» {status}", job.name));
}

/// Writes the outcome of a run back into the job.
fn record(shared: &Arc<Shared>, id: &str, status: &str, session: &str) {
    let Ok(mut state) = shared.scheduler.state.lock() else {
        return;
    };
    if let Some(job) = state.jobs.iter_mut().find(|job| job.id == id) {
        job.runs += 1;
        job.last_status = Some(status.to_string());
        job.session = Some(session.to_string());
    }
    let _ = schedule::save(&state);
}

/// The session a job's runs land in, made on the first run. It is an ordinary
/// session: `/sessions` lists it, opening it shows the whole history, and the
/// next run appends to it rather than starting over.
async fn job_session(shared: &Arc<Shared>, job: &schedule::Job) -> Arc<Session> {
    if let Some(id) = job.session.as_deref()
        && let Some(session) = find(shared, id)
    {
        return session;
    }
    let cwd = if job.directory.is_empty() {
        shared.agent.cwd.clone()
    } else {
        PathBuf::from(&job.directory)
    };
    let id = crate::daemon::random_id();
    let session = Session::new(id.clone(), cwd, None);
    if let Ok(mut state) = session.state.lock() {
        state.name = format!("{} (scheduled)", job.name);
        state.title = job.name.clone();
        state.mode = "build".to_string();
        // A scheduled run has nobody to answer a permission question, so its
        // session never asks. Rules that deny still deny.
        state.permissions = tools::PermissionMode::NeverAsk;
    }
    if let Ok(mut sessions) = shared.sessions.lock() {
        sessions.insert(id.clone(), session.clone());
    }
    if let Ok(mut state) = shared.scheduler.state.lock() {
        if let Some(stored) = state.jobs.iter_mut().find(|stored| stored.id == job.id) {
            stored.session = Some(id.clone());
        }
        let _ = schedule::save(&state);
    }
    session
}

/// Adds a notice, and hands it over at once when somebody is watching.
pub(crate) fn queue_notice(shared: &Arc<Shared>, text: String) {
    if let Ok(mut state) = shared.scheduler.state.lock() {
        state.pending.push(schedule::Notice {
            id: crate::daemon::random_id(),
            text,
        });
        let _ = schedule::save(&state);
    }
    flush_notices(shared);
}

/// Delivers what is waiting, if any client is attached. Notices are broadcast
/// to every session: the scheduler has no idea which conversation is on screen,
/// and the client shows each id once.
pub(crate) fn flush_notices(shared: &Arc<Shared>) {
    let waiting = match shared.scheduler.state.lock() {
        Ok(state) => state.pending.clone(),
        Err(_) => return,
    };
    if waiting.is_empty() {
        return;
    }
    let mut delivered = 0usize;
    {
        let Ok(sessions) = shared.sessions.lock() else {
            return;
        };
        for session in sessions.values() {
            for notice in &waiting {
                delivered += session.broadcast(WireEvent::Notice {
                    id: notice.id.clone(),
                    text: notice.text.clone(),
                });
            }
        }
    }
    if delivered == 0 {
        // Nobody is attached; the notice stays in the file for the next launch.
        return;
    }
    if let Ok(mut state) = shared.scheduler.state.lock() {
        let handed_over: HashSet<&str> = waiting.iter().map(|notice| notice.id.as_str()).collect();
        state
            .pending
            .retain(|notice| !handed_over.contains(notice.id.as_str()));
        let _ = schedule::save(&state);
    }
}

// ------------------------------------------------------------------ the tool

/// The `schedule` tool: how a natural-language request becomes a job. `/schedule
/// push this every day` sends the model here, and so does any sentence of the
/// same shape typed into a normal prompt.
pub(crate) async fn schedule_tool(
    shared: &Arc<Shared>,
    session: &Arc<Session>,
    arguments: &str,
) -> tools::Outcome {
    let args: serde_json::Value = match serde_json::from_str(arguments) {
        Ok(args) => args,
        Err(err) => return outcome(format!("schedule: invalid JSON arguments: {err}")),
    };
    let text = |key: &str| {
        args.get(key)
            .and_then(|value| value.as_str())
            .map(str::trim)
            .filter(|value| !value.is_empty())
            .map(str::to_string)
    };
    let action = text("action").unwrap_or_else(|| "create".to_string());

    match action.as_str() {
        "list" => {
            let jobs = shared.scheduler.jobs();
            if jobs.is_empty() {
                return outcome("No schedules yet.".to_string());
            }
            let mut out = format!("{} schedule(s):\n", jobs.len());
            for job in &jobs {
                out.push_str(&job.line());
                out.push('\n');
            }
            outcome(out.trim_end().to_string())
        }
        "cancel" | "pause" | "resume" => {
            let Some(key) = text("id") else {
                return outcome("schedule: 'id' is required for cancel, pause and resume".to_string());
            };
            let Ok(mut state) = shared.scheduler.state.lock() else {
                return outcome("schedule: the schedule lock is poisoned".to_string());
            };
            let Some(index) = state.position(&key) else {
                return outcome(format!("schedule: no job matches '{key}'"));
            };
            if action == "cancel" {
                let job = state.jobs.remove(index);
                let _ = schedule::save(&state);
                return outcome(format!("cancelled «{}» #{}", job.name, job.id));
            }
            let job = &mut state.jobs[index];
            job.paused = action == "pause";
            let (name, id, paused) = (job.name.clone(), job.id.clone(), job.paused);
            let _ = schedule::save(&state);
            outcome(format!(
                "«{name}» #{id} is now {}",
                if paused { "paused" } else { "running again" }
            ))
        }
        "create" | "" => {
            let Some(request) = text("request") else {
                return outcome("schedule: 'request' is required".to_string());
            };
            let Some(when) = text("at") else {
                return outcome(
                    "schedule: 'at' is required — a local timestamp like 2026-10-18T09:00"
                        .to_string(),
                );
            };
            let now = Local::now();
            let at = match schedule::parse_when(&when, now) {
                Ok(at) => at,
                Err(message) => return outcome(format!("schedule: {message}")),
            };
            let every = match text("every") {
                Some(interval) => match schedule::parse_every(&interval) {
                    Ok(seconds) => Some(seconds),
                    Err(message) => return outcome(format!("schedule: {message}")),
                },
                None => None,
            };
            let name = text("name").unwrap_or_else(|| {
                let first = request.lines().next().unwrap_or(&request).trim();
                let mut name: String = first.chars().take(40).collect();
                if first.chars().count() > 40 {
                    name.push('…');
                }
                name
            });
            let directory = text("directory")
                .unwrap_or_else(|| session.cwd().to_string_lossy().to_string());
            let Ok(mut state) = shared.scheduler.state.lock() else {
                return outcome("schedule: the schedule lock is poisoned".to_string());
            };
            let id = state.fresh_id();
            state.jobs.push(schedule::Job {
                id: id.clone(),
                name: name.clone(),
                request: request.clone(),
                at: at.to_rfc3339(),
                every,
                directory,
                session: None,
                created: store::now_iso(),
                last_run: None,
                last_status: None,
                runs: 0,
                paused: false,
                done: false,
            });
            let _ = schedule::save(&state);
            let when_text = schedule::when_text(&at.to_rfc3339());
            let repeat = every
                .map(|seconds| format!(", repeating every {}", schedule::humanize(seconds)))
                .unwrap_or_else(|| ", once".to_string());
            outcome(format!(
                "scheduled «{name}» #{id}: {} {when_text}{repeat}. It runs unattended (permission \
                 mode «never ask», deny rules still apply) in its own session, and you are told \
                 the outcome when it finishes.",
                if every.is_some() { "first run" } else { "runs at" }
            ))
        }
        other => outcome(format!(
            "schedule: unknown action '{other}' — use create, list, cancel, pause or resume"
        )),
    }
}

fn outcome(output: String) -> tools::Outcome {
    tools::Outcome {
        output,
        background_id: None,
    }
}
