//! the HTTP surface: sessions, prompts, files, skills, events.

use axum::extract::{Path, State};
use axum::http::{HeaderMap, StatusCode};
use axum::response::sse::{Event, Sse};
use axum::response::{IntoResponse, Json, Response};
use futures_util::StreamExt;
use std::convert::Infallible;
use std::path::PathBuf;
use std::sync::Arc;
use crate::daemon;
use crate::proto::{
    AnswerBody, Health, PromptBody, RewindBody, SessionSummary, WireAnswerRequest,
    WireEvent, WireMsg, WirePhase, WireSnapshot,
};
use crate::protocol::InputItem;
use crate::tools;
use crate::store;

use super::*;

// ------------------------------------------------------------------ routes

pub(crate) fn authorized(shared: &Shared, headers: &HeaderMap) -> bool {
    headers
        .get("authorization")
        .and_then(|value| value.to_str().ok())
        .and_then(|value| value.strip_prefix("Bearer "))
        .map(|token| token == shared.password)
        .unwrap_or(false)
}

pub(crate) fn unauthorized() -> Response {
    (StatusCode::UNAUTHORIZED, "missing or wrong server password").into_response()
}

pub(crate) async fn health(State(shared): State<Arc<Shared>>, headers: HeaderMap) -> Response {
    if !authorized(&shared, &headers) {
        return unauthorized();
    }
    Json(Health {
        healthy: true,
        version: env!("CARGO_PKG_VERSION").to_string(),
    })
    .into_response()
}

pub(crate) async fn list_sessions(State(shared): State<Arc<Shared>>, headers: HeaderMap) -> Response {
    if !authorized(&shared, &headers) {
        return unauthorized();
    }
    let sessions = shared.sessions.lock().expect("sessions");
    let list: Vec<SessionSummary> = sessions
        .values()
        .map(|session| {
            let state = session.state.lock().expect("session state");
            SessionSummary {
                id: session.id.clone(),
                title: state.title.clone(),
                name: state.name.clone(),
                phase: state.phase,
            }
        })
        .collect();
    Json(list).into_response()
}
/// `/skills`: what is installed.
pub(crate) async fn list_skills(State(shared): State<Arc<Shared>>, headers: HeaderMap) -> Response {
    if !authorized(&shared, &headers) {
        return unauthorized();
    }
    let skills: Vec<serde_json::Value> = crate::skills::discover()
        .into_iter()
        .map(|skill| {
            serde_json::json!({ "name": skill.name, "description": skill.description })
        })
        .collect();
    Json(skills).into_response()
}

/// `/<skill>`: hand a skill's instructions to the model as the next task.
pub(crate) async fn run_skill(
    State(shared): State<Arc<Shared>>,
    Path(id): Path<String>,
    headers: HeaderMap,
    Json(body): Json<crate::proto::SkillBody>,
) -> Response {
    if !authorized(&shared, &headers) {
        return unauthorized();
    }
    let Some(session) = find(&shared, &id) else {
        return (StatusCode::NOT_FOUND, "no such session").into_response();
    };
    let Some(skill) = crate::skills::discover()
        .into_iter()
        .find(|skill| skill.name == body.name)
    else {
        return (
            StatusCode::NOT_FOUND,
            format!("no skill named {} — install it with `exp-1 install`", body.name),
        )
            .into_response();
    };
    let Some(instructions) = crate::skills::instructions(&skill.name) else {
        return (StatusCode::BAD_REQUEST, "the skill has no instructions").into_response();
    };
    session.push("user", format!("/{}", skill.name));
    session.push(
        "system",
        if skill.description.is_empty() {
            format!("skill: {}", skill.name)
        } else {
            format!("skill {}: {}", skill.name, skill.description)
        },
    );
    let text = format!(
        "The user invoked the skill `{}`. Follow its instructions for this task.\n\n{instructions}",
        skill.name
    );
    let task = tokio::spawn(run_turn(shared.clone(), session.clone(), text, Vec::new()));
    if let Ok(mut slot) = session.agent.lock() {
        *slot = Some(task);
    }
    watch_questions(session.clone());
    StatusCode::NO_CONTENT.into_response()
}

/// `/cd`: move the session's tools to another directory.
pub(crate) async fn cd(
    State(shared): State<Arc<Shared>>,
    Path(id): Path<String>,
    headers: HeaderMap,
    Json(body): Json<crate::proto::CdBody>,
) -> Response {
    if !authorized(&shared, &headers) {
        return unauthorized();
    }
    let Some(session) = find(&shared, &id) else {
        return (StatusCode::NOT_FOUND, "no such session").into_response();
    };
    let wanted = body.path.trim();
    if wanted.is_empty() {
        return Json(serde_json::json!({ "cwd": session.cwd().to_string_lossy() })).into_response();
    }
    // `~` and relative paths are resolved against where the session is now.
    let expanded = if wanted == "~" {
        store::home()
    } else if let Some(rest) = wanted.strip_prefix("~/") {
        store::home().join(rest)
    } else {
        PathBuf::from(wanted)
    };
    let target = if expanded.is_absolute() {
        expanded
    } else {
        session.cwd().join(expanded)
    };
    let Ok(target) = target.canonicalize() else {
        return (
            StatusCode::BAD_REQUEST,
            format!("no such directory: {}", target.display()),
        )
            .into_response();
    };
    if !target.is_dir() {
        return (
            StatusCode::BAD_REQUEST,
            format!("not a directory: {}", target.display()),
        )
            .into_response();
    }
    if let Ok(mut cwd) = session.cwd.lock() {
        *cwd = target.clone();
    }
    session.push(
        "system",
        format!("working directory is now {}", target.display()),
    );
    Json(serde_json::json!({ "cwd": target.to_string_lossy() })).into_response()
}

/// `/mcp`: which servers answered and what they offer.
pub(crate) async fn list_mcp(State(shared): State<Arc<Shared>>, headers: HeaderMap) -> Response {
    if !authorized(&shared, &headers) {
        return unauthorized();
    }
    Json(shared.mcp.status().await).into_response()
}

/// `shift+tab`: switch the session between building and planning.
pub(crate) async fn set_mode(
    State(shared): State<Arc<Shared>>,
    Path(id): Path<String>,
    headers: HeaderMap,
    Json(body): Json<crate::proto::ModeBody>,
) -> Response {
    if !authorized(&shared, &headers) {
        return unauthorized();
    }
    let Some(session) = find(&shared, &id) else {
        return (StatusCode::NOT_FOUND, "no such session").into_response();
    };
    let mode = match body.mode.trim() {
        "plan" => "plan",
        "build" => "build",
        other => {
            return (
                StatusCode::BAD_REQUEST,
                format!("unknown mode '{other}' (build|plan)"),
            )
                .into_response();
        }
    };
    if let Ok(mut state) = session.state.lock() {
        state.mode = mode.to_string();
    }
    let note = if mode == "plan" {
        "plan mode: investigating only — write, edit and delete are refused"
    } else {
        "build mode: tools can change files again"
    };
    session.push("system", note);
    Json(serde_json::json!({ "mode": mode, "note": note })).into_response()
}

/// `ctrl+shift+a`: how much the session asks before running a tool. The mode
/// is held per session; the choice is also written to `settings.json`, because
/// it is what the next launch — and every session it creates — starts with.
pub(crate) async fn set_permissions(
    State(shared): State<Arc<Shared>>,
    Path(id): Path<String>,
    headers: HeaderMap,
    Json(body): Json<crate::proto::PermissionsBody>,
) -> Response {
    if !authorized(&shared, &headers) {
        return unauthorized();
    }
    let Some(session) = find(&shared, &id) else {
        return (StatusCode::NOT_FOUND, "no such session").into_response();
    };
    let mode = body.permissions;
    if let Ok(mut state) = session.state.lock() {
        state.permissions = mode;
    }
    let mut settings = store::load_settings();
    settings.permissions = mode;
    store::save_settings(&settings);
    let note = format!(
        "permission mode: {} — {}",
        mode.label(),
        match mode {
            tools::PermissionMode::AlwaysAsk =>
                "write, edit, delete and shell ask before running",
            tools::PermissionMode::NeverAsk => "tools run without asking",
            tools::PermissionMode::AskOnlyCommands =>
                "only shell asks; file writes and edits run",
        }
    );
    session.push("system", note.clone());
    Json(serde_json::json!({
        "permissions": mode,
        "label": mode.label(),
        "note": note,
    }))
    .into_response()
}

/// `/clear`: start this session over. The transcript on screen is only half of
/// a conversation — the other half is the history the model is sent — so both
/// go, along with the undo log, and the title is taken again on the next prompt.
pub(crate) async fn clear_session(
    State(shared): State<Arc<Shared>>,
    Path(id): Path<String>,
    headers: HeaderMap,
) -> Response {
    if !authorized(&shared, &headers) {
        return unauthorized();
    }
    let Some(session) = find(&shared, &id) else {
        return (StatusCode::NOT_FOUND, "no such session").into_response();
    };
    // A turn in flight would keep writing into the conversation being cleared.
    if let Some(task) = session.agent.lock().ok().and_then(|mut slot| slot.take()) {
        task.abort();
    }
    {
        let Ok(mut state) = session.state.lock() else {
            return (StatusCode::INTERNAL_SERVER_ERROR, "session state is locked").into_response();
        };
        state.transcript.clear();
        state.live.clear();
        state.usage = None;
        state.error = None;
        state.question = None;
        state.title.clear();
        state.name.clear();
        state.phase = WirePhase::Idle;
    }
    session.history.lock().await.clear();
    if let Ok(mut log) = session.undo.lock() {
        *log = tools::UndoLog::default();
    }
    session.broadcast(WireEvent::Idle);
    StatusCode::NO_CONTENT.into_response()
}

/// `/fork`: continue this conversation as a new session, so a different
/// approach can be tried without losing the original.
pub(crate) async fn fork_session(
    State(shared): State<Arc<Shared>>,
    Path(id): Path<String>,
    headers: HeaderMap,
) -> Response {
    if !authorized(&shared, &headers) {
        return unauthorized();
    }
    let Some(source) = find(&shared, &id) else {
        return (StatusCode::NOT_FOUND, "no such session").into_response();
    };
    let session = Session::new(daemon::random_id(), source.cwd(), source.parent.clone());
    {
        let source_state = source.state.lock().expect("session state");
        if let Ok(mut state) = session.state.lock() {
            state.transcript = source_state.transcript.clone();
            state.name = source_state.name.clone();
            state.usage = source_state.usage.clone();
            state.permissions = source_state.permissions;
            state.title = if source_state.title.is_empty() {
                String::new()
            } else {
                format!("{} (fork)", source_state.title)
            };
        }
    }
    *session.history.lock().await = source.history.lock().await.clone();
    let snapshot = session.snapshot();
    shared
        .sessions
        .lock()
        .expect("sessions")
        .insert(snapshot.id.clone(), session);
    Json(snapshot).into_response()
}

/// `/undo`: put the files back the way they were before the last turn.
pub(crate) async fn undo(
    State(shared): State<Arc<Shared>>,
    Path(id): Path<String>,
    headers: HeaderMap,
) -> Response {
    if !authorized(&shared, &headers) {
        return unauthorized();
    }
    let Some(session) = find(&shared, &id) else {
        return (StatusCode::NOT_FOUND, "no such session").into_response();
    };
    let taken = match session.undo.lock() {
        Ok(mut log) => {
            let keep = log.turns().saturating_sub(1);
            log.rewind_to(keep)
        }
        Err(_) => Vec::new(),
    };
    if taken.iter().all(|turn| turn.is_empty()) {
        return (
            StatusCode::BAD_REQUEST,
            "nothing to undo — this session has no recorded changes",
        )
            .into_response();
    }
    if let Ok(mut points) = session.rewind_points.lock() {
        let keep = points.len().saturating_sub(1);
        points.truncate(keep);
    }
    let (restored, failed) = restore_turns(taken);
    let note = if failed.is_empty() {
        format!("undone: {}", restored.join(", "))
    } else {
        format!(
            "undone: {} · could not restore: {}",
            restored.join(", "),
            failed.join(", ")
        )
    };
    session.push("system", note.clone());
    Json(serde_json::json!({ "restored": restored, "failed": failed, "note": note }))
        .into_response()
}

/// Puts the files back as they were before a set of turns, newest turn first:
/// going back in the conversation has to take the working tree with it.
fn restore_turns(
    taken: Vec<Vec<(std::path::PathBuf, Option<Vec<u8>>)>>,
) -> (Vec<String>, Vec<String>) {
    let mut restored: Vec<String> = Vec::new();
    let mut failed: Vec<String> = Vec::new();
    for turn in taken.into_iter().rev() {
        for (path, before) in turn {
            let outcome = match before {
                Some(bytes) => std::fs::write(&path, bytes),
                None => std::fs::remove_file(&path).or_else(|e| {
                    // A file that is already gone was never created, or was
                    // removed by hand; either way the goal state is reached.
                    if e.kind() == std::io::ErrorKind::NotFound {
                        Ok(())
                    } else {
                        Err(e)
                    }
                }),
            };
            let name = path.display().to_string();
            match outcome {
                Ok(()) => {
                    if !restored.contains(&name) {
                        restored.push(name);
                    }
                }
                Err(e) => failed.push(format!("{name}: {e}")),
            }
        }
    }
    (restored, failed)
}

/// `POST /sessions/{id}/rewind`: go back to one user message. The transcript,
/// the model's history and every file change after it go away, and the text
/// comes back so it can be edited and sent again.
pub(crate) async fn rewind(
    State(shared): State<Arc<Shared>>,
    Path(id): Path<String>,
    headers: HeaderMap,
    Json(body): Json<RewindBody>,
) -> Response {
    if !authorized(&shared, &headers) {
        return unauthorized();
    }
    let Some(session) = find(&shared, &id) else {
        return (StatusCode::NOT_FOUND, "no such session").into_response();
    };

    // The transcript is the shared index between the client and the server, but
    // the text is checked too: going back to the wrong message would throw work
    // away for nothing.
    let (index, ordinal) = {
        // The client counts transcript entries, but it draws lines of its own
        // that the server never saw, so the text is what identifies the message
        // and it is looked for backwards from the index the client gave.
        let state = session.state.lock().expect("session state");
        let from = body.index.min(state.transcript.len().saturating_sub(1));
        let found = (0..=from).rev().find(|at| {
            state.transcript[*at].role == "user" && state.transcript[*at].text == body.text
        });
        let Some(index) = found else {
            return (
                StatusCode::BAD_REQUEST,
                "no such user message in this session",
            )
                .into_response();
        };
        let ordinal = state.transcript[..=index]
            .iter()
            .filter(|msg| msg.role == "user")
            .count();
        (index, ordinal)
    };

    // The conversation goes first, the files second: a half-rewound session
    // would be worse than either.
    if let Ok(mut state) = session.state.lock() {
        state.transcript.truncate(index);
    }
    let taken = {
        let keep = session
            .rewind_points
            .lock()
            .ok()
            .and_then(|points| points.get(ordinal - 1).copied());
        match (keep, session.undo.lock()) {
            (Some(keep), Ok(mut log)) => log.rewind_to(keep),
            _ => Vec::new(),
        }
    };
    if let Ok(mut points) = session.rewind_points.lock() {
        points.truncate(ordinal.saturating_sub(1));
    }
    {
        // The model's history mirrors the transcript: keep everything up to,
        // but not including, that user message.
        let mut seen = 0usize;
        let mut history = session.history.lock().await;
        let cut = history.iter().position(|item| match item {
            InputItem::Message { role, .. } if role == "user" => {
                seen += 1;
                seen == ordinal
            }
            _ => false,
        });
        if let Some(cut) = cut {
            history.truncate(cut);
        }
    }
    let (restored, failed) = restore_turns(taken);

    let note = match (restored.is_empty(), failed.is_empty()) {
        (true, true) => "rewound: nothing after that message had changed files".to_string(),
        (false, true) => format!("rewound: {}", restored.join(", ")),
        (true, false) => format!("rewound · could not restore: {}", failed.join(", ")),
        (false, false) => format!(
            "rewound: {} · could not restore: {}",
            restored.join(", "),
            failed.join(", ")
        ),
    };
    session.push("system", note.clone());
    Json(serde_json::json!({ "note": note, "text": body.text })).into_response()
}

/// `/diff`: what the last turn changed, as a unified diff.
pub(crate) async fn show_diff(
    State(shared): State<Arc<Shared>>,
    Path(id): Path<String>,
    headers: HeaderMap,
) -> Response {
    if !authorized(&shared, &headers) {
        return unauthorized();
    }
    let Some(session) = find(&shared, &id) else {
        return (StatusCode::NOT_FOUND, "no such session").into_response();
    };
    let changes: Vec<(PathBuf, Option<Vec<u8>>)> = match session.undo.lock() {
        Ok(log) => log.last().to_vec(),
        Err(_) => Vec::new(),
    };
    if changes.is_empty() {
        return (
            StatusCode::BAD_REQUEST,
            "nothing to show — this session has no recorded changes",
        )
            .into_response();
    }
    let cwd = session.cwd();
    let mut out = String::new();
    for (path, before) in changes {
        let before = before
            .and_then(|bytes| String::from_utf8(bytes).ok())
            .unwrap_or_default();
        let after = std::fs::read_to_string(&path).unwrap_or_default();
        let name = path
            .strip_prefix(&cwd)
            .map(|rest| rest.display().to_string())
            .unwrap_or_else(|_| path.display().to_string());
        match tools::unified_diff(&name, &before, &after) {
            Some(diff) => {
                out.push_str(&diff);
                out.push('\n');
            }
            None => {
                if before.is_empty() && !after.is_empty() {
                    out.push_str(&format!("--- {name} (new file, {} lines)\n", after.lines().count()));
                }
            }
        }
    }
    if out.trim().is_empty() {
        out = "the last turn left no differences".to_string();
    }
    session.push("system", out.clone());
    Json(serde_json::json!({ "diff": out })).into_response()
}

/// `/compact`: summarise the conversation now instead of waiting for the
/// window to fill.
pub(crate) async fn compact_now(
    State(shared): State<Arc<Shared>>,
    Path(id): Path<String>,
    headers: HeaderMap,
) -> Response {
    if !authorized(&shared, &headers) {
        return unauthorized();
    }
    let Some(session) = find(&shared, &id) else {
        return (StatusCode::NOT_FOUND, "no such session").into_response();
    };
    session.push("system", "compacting the conversation on request…");
    match compact(&shared, &session).await {
        Some(summary) => Json(serde_json::json!({ "summary": summary })).into_response(),
        None => (
            StatusCode::BAD_REQUEST,
            "there is nothing to compact yet",
        )
            .into_response(),
    }
}

/// Names the session again from what it has become.
pub(crate) async fn retitle(
    State(shared): State<Arc<Shared>>,
    Path(id): Path<String>,
    headers: HeaderMap,
) -> Response {
    if !authorized(&shared, &headers) {
        return unauthorized();
    }
    let Some(session) = find(&shared, &id) else {
        return (StatusCode::NOT_FOUND, "no such session").into_response();
    };
    regenerate_title(shared.clone(), session);
    StatusCode::NO_CONTENT.into_response()
}

pub(crate) async fn create_session(State(shared): State<Arc<Shared>>, headers: HeaderMap) -> Response {
    if !authorized(&shared, &headers) {
        return unauthorized();
    }
    // The caller says which project this session belongs to.
    let cwd = headers
        .get("x-exp1-directory")
        .and_then(|value| value.to_str().ok())
        .map(PathBuf::from)
        .filter(|path| path.is_dir())
        .unwrap_or_else(|| shared.agent.cwd.clone());
    // A side chat names the session it follows; the parent must exist, so a
    // stale id from a finished session is simply ignored.
    let parent = headers
        .get("x-exp1-parent")
        .and_then(|value| value.to_str().ok())
        .map(str::to_string)
        .filter(|id| {
            !id.is_empty() && shared.sessions.lock().expect("sessions").contains_key(id)
        });
    let id = daemon::random_id();
    let session = Session::new(id.clone(), cwd, parent);
    // A new session starts on the permission mode the user last chose.
    if let Ok(mut state) = session.state.lock() {
        state.permissions = store::load_settings().permissions;
    }
    shared
        .sessions
        .lock()
        .expect("sessions")
        .insert(id.clone(), session.clone());
    Json(session.snapshot()).into_response()
}

fn find(shared: &Shared, id: &str) -> Option<Arc<Session>> {
    shared.sessions.lock().expect("sessions").get(id).cloned()
}

pub(crate) async fn session_snapshot(
    State(shared): State<Arc<Shared>>,
    Path(id): Path<String>,
    headers: HeaderMap,
) -> Response {
    if !authorized(&shared, &headers) {
        return unauthorized();
    }
    if let Some(session) = find(&shared, &id) {
        return Json(session.snapshot()).into_response();
    }
    // A conversation saved before this server started is still worth reading,
    // so the disk is the fallback rather than a 404.
    match store::load_session(&id) {
        Some(saved) => Json(WireSnapshot {
            id: saved.id.clone(),
            title: saved.title.clone(),
            name: saved.main.name.clone(),
            phase: WirePhase::Idle,
            transcript: saved
                .main
                .transcript
                .iter()
                .map(|msg| WireMsg {
                    role: msg.role.clone(),
                    text: msg.text.clone(),
                    images: 0,
                    kind: msg.kind,
                })
                .collect(),
            live: String::new(),
            usage: None,
            mode: "build".to_string(),
            permissions: store::load_settings().permissions,
            error: None,
            question: None,
        })
        .into_response(),
        None => (StatusCode::NOT_FOUND, "no such session").into_response(),
    }
}

pub(crate) async fn prompt(
    State(shared): State<Arc<Shared>>,
    Path(id): Path<String>,
    headers: HeaderMap,
    Json(body): Json<PromptBody>,
) -> Response {
    if !authorized(&shared, &headers) {
        return unauthorized();
    }
    let Some(session) = find(&shared, &id) else {
        return (StatusCode::NOT_FOUND, "no such session").into_response();
    };
    if body.text.trim().is_empty() && body.images.is_empty() {
        return (StatusCode::BAD_REQUEST, "empty prompt").into_response();
    }
    let text = body.text.trim().to_string();
    let mut images: Vec<crate::proto::WireImage> = body.images.clone();
    session.push("user", text.clone());
    if let Ok(mut state) = session.state.lock()
        && let Some(last) = state.transcript.last_mut()
    {
        last.images = images.len();
    }
    let mut readable: Vec<String> = Vec::new();
    let mut missing: Vec<String> = Vec::new();
    for named in &body.files {
        let path = PathBuf::from(named);
        let path = if path.is_absolute() {
            path
        } else {
            session.cwd().join(path)
        };
        match std::fs::read(&path) {
            Ok(bytes) if is_image(&path) => {
                use base64::Engine;
                images.push(crate::proto::WireImage {
                    media_type: media_type(&path).to_string(),
                    data: base64::engine::general_purpose::STANDARD.encode(&bytes),
                });
            }
            Ok(_) => readable.push(path.display().to_string()),
            Err(_) => missing.push(path.display().to_string()),
        }
    }
    if !readable.is_empty() || !missing.is_empty() {
        let mut note = String::new();
        if !readable.is_empty() {
            note.push_str(&format!("attached: {}", readable.join(", ")));
        }
        if !missing.is_empty() {
            if !note.is_empty() {
                note.push_str(" · ");
            }
            note.push_str(&format!("not found: {}", missing.join(", ")));
        }
        session.push("system", note);
    }
    let for_model = if readable.is_empty() {
        text.clone()
    } else {
        format!(
            "{text}\n\n[The user attached these files; read each one before answering: {}]",
            readable.join(", ")
        )
    };
    {
        let mut state = session.state.lock().expect("session state");
        if state.name.is_empty() {
            state.name = text.lines().next().unwrap_or(&text).chars().take(32).collect();
        }
        state.error = None;
    }

    let task = tokio::spawn(run_turn(
        shared.clone(),
        session.clone(),
        for_model,
        images,
    ));
    if let Ok(mut slot) = session.agent.lock() {
        *slot = Some(task);
    }
    watch_questions(session.clone());
    // Name the session once, from its first prompt.
    maybe_name(shared.clone(), session.clone(), text);
    StatusCode::NO_CONTENT.into_response()
}
pub(crate) async fn answer(
    State(shared): State<Arc<Shared>>,
    Path(id): Path<String>,
    headers: HeaderMap,
    Json(body): Json<AnswerBody>,
) -> Response {
    if !authorized(&shared, &headers) {
        return unauthorized();
    }
    let Some(session) = find(&shared, &id) else {
        return (StatusCode::NOT_FOUND, "no such session").into_response();
    };
    if session.ask.peek().is_none() {
        return (StatusCode::CONFLICT, "no question is pending").into_response();
    }
    session.ask.answer(body.answers);
    StatusCode::NO_CONTENT.into_response()
}
/// Mirrors the session's pending question into the UI, and clears it after.
fn watch_questions(session: Arc<Session>) {
    tokio::spawn(async move {
        let mut shown = false;
        let mut idle_checks = 0u32;
        loop {
            let pending = session.ask.peek();
            match (&pending, shown) {
                (Some(snapshot), false) => {
                    let request = WireAnswerRequest {
                        questions: snapshot
                            .questions
                            .iter()
                            .map(|item| crate::proto::WireQuestion {
                                label: item.label.clone(),
                                text: item.text.clone(),
                                options: item.options.clone(),
                            })
                            .collect(),
                        submit_label: snapshot.submit_label.clone(),
                    };
                    if let Ok(mut state) = session.state.lock() {
                        state.question = Some(request.clone());
                    }
                    session.broadcast(WireEvent::Ask(request));
                    shown = true;
                }
                (None, true) => {
                    if let Ok(mut state) = session.state.lock() {
                        state.question = None;
                    }
                    session.broadcast(WireEvent::AskClosed);
                    shown = false;
                }
                _ => {}
            }

            let idle = session
                .state
                .lock()
                .map(|state| state.phase == WirePhase::Idle)
                .unwrap_or(true);
            if idle && pending.is_none() {
                // A turn is admitted a moment after the prompt arrives, so one
                // idle look must not end the watch — a permission question
                // would then never reach the screen.
                idle_checks += 1;
                if idle_checks >= 14 {
                    return;
                }
            } else {
                idle_checks = 0;
            }
            tokio::time::sleep(std::time::Duration::from_millis(150)).await;
        }
    });
}

pub(crate) async fn interrupt(
    State(shared): State<Arc<Shared>>,
    Path(id): Path<String>,
    headers: HeaderMap,
) -> Response {
    if !authorized(&shared, &headers) {
        return unauthorized();
    }
    let Some(session) = find(&shared, &id) else {
        return (StatusCode::NOT_FOUND, "no such session").into_response();
    };
    let task = session.agent.lock().ok().and_then(|mut slot| slot.take());
    if let Some(task) = task {
        task.abort();
        session.push("system", "interrupted");
        session.set_phase(WirePhase::Idle);
        session.broadcast(WireEvent::Idle);
    }
    StatusCode::NO_CONTENT.into_response()
}

/// Streams every event for a session, starting with a snapshot.
pub(crate) async fn events(
    State(shared): State<Arc<Shared>>,
    Path(id): Path<String>,
    headers: HeaderMap,
) -> Response {
    if !authorized(&shared, &headers) {
        return unauthorized();
    }
    let Some(session) = find(&shared, &id) else {
        return (StatusCode::NOT_FOUND, "no such session").into_response();
    };
    let receiver = session.subscribe();
    let snapshot = session.snapshot();
    let head = futures_util::stream::iter(std::iter::once(Ok::<_, Infallible>(
        Event::default()
            .json_data(WireEvent::Snapshot(snapshot))
            .unwrap_or_default(),
    )));
    let stream = head.chain(futures_util::stream::unfold(
        receiver,
        |mut receiver| async move {
            match receiver.recv().await {
                Some(event) => {
                    let payload = Event::default().json_data(event).ok()?;
                    Some((Ok::<_, Infallible>(payload), receiver))
                }
                None => None,
            }
        },
    ));
    Sse::new(stream).into_response()
}

// ------------------------------------------------------------------ schedules

/// `/schedule`: every job as stored. The client is what formats them.
pub(crate) async fn list_schedules(
    State(shared): State<Arc<Shared>>,
    headers: HeaderMap,
) -> Response {
    if !authorized(&shared, &headers) {
        return unauthorized();
    }
    Json(shared.scheduler.jobs()).into_response()
}

/// `/schedule run|pause|resume <id>`: one job, one verb.
pub(crate) async fn schedule_action(
    State(shared): State<Arc<Shared>>,
    Path((id, action)): Path<(String, String)>,
    headers: HeaderMap,
) -> Response {
    if !authorized(&shared, &headers) {
        return unauthorized();
    }
    let mut start: Option<String> = None;
    let note = {
        let Ok(mut state) = shared.scheduler.state.lock() else {
            return (StatusCode::INTERNAL_SERVER_ERROR, "the schedule lock is poisoned")
                .into_response();
        };
        let Some(index) = state.position(&id) else {
            return (StatusCode::NOT_FOUND, "no such schedule").into_response();
        };
        match action.as_str() {
            "pause" | "resume" => {
                let job = &mut state.jobs[index];
                job.paused = action == "pause";
                let (name, id, paused) = (job.name.clone(), job.id.clone(), job.paused);
                let _ = crate::schedule::save(&state);
                format!(
                    "«{name}» #{id} is now {}",
                    if paused { "paused" } else { "running again" }
                )
            }
            "run" => {
                let job = &mut state.jobs[index];
                job.at = chrono::Local::now().to_rfc3339();
                job.paused = false;
                job.done = false;
                let (name, id) = (job.name.clone(), job.id.clone());
                let _ = crate::schedule::save(&state);
                start = Some(id.clone());
                format!("«{name}» #{id}: starting now")
            }
            other => {
                return (StatusCode::BAD_REQUEST, format!("unknown schedule action '{other}'"))
                    .into_response()
            }
        }
    };
    if let Some(id) = start {
        // Outside the lock: `fire_job` takes it again.
        fire_job(&shared, &id, None);
    }
    Json(serde_json::json!({ "note": note })).into_response()
}

/// `/schedule rm <id>`: drop a job for good.
pub(crate) async fn delete_schedule(
    State(shared): State<Arc<Shared>>,
    Path(id): Path<String>,
    headers: HeaderMap,
) -> Response {
    if !authorized(&shared, &headers) {
        return unauthorized();
    }
    let note = {
        let Ok(mut state) = shared.scheduler.state.lock() else {
            return (StatusCode::INTERNAL_SERVER_ERROR, "the schedule lock is poisoned")
                .into_response();
        };
        let Some(index) = state.position(&id) else {
            return (StatusCode::NOT_FOUND, "no such schedule").into_response();
        };
        let job = state.jobs.remove(index);
        let _ = crate::schedule::save(&state);
        format!("removed «{}» #{}", job.name, job.id)
    };
    Json(serde_json::json!({ "note": note })).into_response()
}
