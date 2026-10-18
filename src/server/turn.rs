//! one turn of the agent loop: requests, tool calls, background work.

use std::sync::Arc;
use std::time::Instant;
use tokio::sync::mpsc;
use crate::api::ApiEvent;
use crate::proto::{
    WireEvent, WireMsgKind, WirePhase,
};
use crate::protocol::{ContentPart, InputItem, Reasoning, ResponsesRequest};
use crate::tools;
use crate::store;

use super::*;

/// Watches a background command and tells the agent the moment it finishes,
/// interrupting the turn so the news is not missed.
fn watch_background(shared: Arc<Shared>, session: Arc<Session>, id: String) {
    tokio::spawn(async move {
        let registry = shared.agent.registry.clone();
        loop {
            tokio::time::sleep(std::time::Duration::from_millis(500)).await;
            let running = registry.lock().await.background_running(&id).await;
            if !running {
                break;
            }
        }

        let collected = tools::read_output(
            &registry,
            &serde_json::json!({ "output_id": id, "limit": 80 }),
        )
        .await;

        // Stop whatever the agent is doing; the result is more useful than the
        // work it was in the middle of.
        if let Ok(mut slot) = session.agent.lock()
            && let Some(task) = slot.take()
        {
            task.abort();
        }
        session.push("system", "interrupted: a background command finished");

        let notice = format!(
            "The background command '{id}' has finished. Its output:\n\n{}\n\nContinue from here.",
            collected.output
        );
        session.push("user", notice.clone());
        let task = tokio::spawn(run_turn(shared.clone(), session.clone(), notice, Vec::new()));
        if let Ok(mut slot) = session.agent.lock() {
            *slot = Some(task);
        }
    });
}
// ------------------------------------------------------------------ the turn

/// True for tools that only read: several of them in one model message can run
/// at the same time without changing what any of them sees.
fn reads_only(name: &str) -> bool {
    matches!(name, "read" | "grep" | "count" | "ast" | "fetch" | "websearch")
}

/// True when a permission rule would ask the user about this call. Such calls
/// run one at a time: there is a single question slot, and two prompts at once
/// would cancel each other.
fn needs_asking(session: &Arc<Session>, name: &str, arguments: &str) -> bool {
    let resource = permission_resource(name, arguments, &session.cwd());
    tools::load_rules()
        .decide(name, &resource)
        .is_some_and(|rule| rule.effect == "ask")
}

/// Runs one tool call with permissions and plan mode applied.
async fn run_one(
    shared: &Arc<Shared>,
    session: &Arc<Session>,
    name: &str,
    arguments: &str,
) -> tools::Outcome {
    let refusal = permission_check(session, name, arguments)
        .await
        .or_else(|| plan_refusal(session, name, arguments));
    if let Some(refusal) = refusal {
        session.push("system", refusal.clone());
        return tools::Outcome {
            output: refusal,
            background_id: None,
        };
    }
    if name == "task" {
        return run_subagent(shared.clone(), session.clone(), subagent_prompt(arguments), 0).await;
    }
    if name == "schedule" {
        // The scheduler owns the jobs, and only the server has it.
        return schedule_tool(shared, session, arguments).await;
    }
    if let Some(result) = shared.mcp.call(name, arguments).await {
        return match result {
            Ok(text) => tools::Outcome {
                output: text,
                background_id: None,
            },
            Err(err) => tools::Outcome {
                output: format!("mcp: {err:#}"),
                background_id: None,
            },
        };
    }
    tools::call(
        &session.cwd(),
        &shared.agent.registry,
        &session.undo,
        &session.ask,
        name,
        arguments,
    )
    .await
}

/// Runs the calls of one model message. Runs of read-only calls go together;
/// everything else keeps its order, so a write followed by a read of the same
/// file cannot be reordered.
async fn run_calls(
    shared: &Arc<Shared>,
    session: &Arc<Session>,
    calls: Vec<(String, String, String)>,
) -> Vec<(String, String, String, tools::Outcome)> {
    let mut results = Vec::with_capacity(calls.len());
    let mut group: Vec<(String, String, String)> = Vec::new();
    for (call_id, name, arguments) in calls {
        if reads_only(&name) && !needs_asking(session, &name, &arguments) {
            group.push((call_id, name, arguments));
            continue;
        }
        if !group.is_empty() {
            results.extend(run_group(shared, session, std::mem::take(&mut group)).await);
        }
        let outcome = run_one(shared, session, &name, &arguments).await;
        results.push((call_id, name, arguments, outcome));
    }
    if !group.is_empty() {
        results.extend(run_group(shared, session, group).await);
    }
    results
}

/// Runs a batch of read-only calls at the same time, keeping the answers in the
/// order the model asked for them.
async fn run_group(
    shared: &Arc<Shared>,
    session: &Arc<Session>,
    group: Vec<(String, String, String)>,
) -> Vec<(String, String, String, tools::Outcome)> {
    let mut running = Vec::with_capacity(group.len());
    for (call_id, name, arguments) in group {
        let shared = shared.clone();
        let session = session.clone();
        let (job_name, job_arguments) = (name.clone(), arguments.clone());
        running.push((
            call_id,
            name,
            arguments,
            tokio::spawn(async move { run_one(&shared, &session, &job_name, &job_arguments).await }),
        ));
    }
    let mut results = Vec::with_capacity(running.len());
    for (call_id, name, arguments, handle) in running {
        let outcome = handle.await.unwrap_or_else(|_| tools::Outcome {
            output: "the tool task failed".to_string(),
            background_id: None,
        });
        results.push((call_id, name, arguments, outcome));
    }
    results
}

pub(crate) async fn run_turn(
    shared: Arc<Shared>,
    session: Arc<Session>,
    prompt: String,
    images: Vec<crate::proto::WireImage>,
) {
    {
        let mut history = session.history.lock().await;
        let mut parts = Vec::new();
        if !prompt.trim().is_empty() {
            parts.push(ContentPart::InputText {
                text: prompt.clone(),
            });
        }
        for image in &images {
            parts.push(ContentPart::InputImage {
                image_url: format!("data:{};base64,{}", image.media_type, image.data),
            });
        }
        history.push(InputItem::Message {
            role: "user".to_string(),
            content: parts,
        });
    }
    let started = Instant::now();
    if let Ok(mut log) = session.undo.lock() {
        log.begin();
        // Where this message starts, for a rewind back to it: the turns that
        // came before.
        let before = log.turns().saturating_sub(1);
        if let Ok(mut points) = session.rewind_points.lock() {
            points.push(before);
        }
    }
    // The prompt file plus whatever the project itself asks for.
    let instructions = {
        let base = store::prompt("system", INSTRUCTIONS);
        let project = project_instructions(&session.cwd());
        let mut text = if project.is_empty() {
            base
        } else {
            format!("{base}{project}")
        };
        if session.mode() == "plan" {
            text.push_str("\n\n");
            text.push_str(&store::prompt("plan", PLAN_NOTE));
        }
        text
    };

    let mut finished = false;
    for _iteration in 0..MAX_ITERATIONS {
        session.set_phase(WirePhase::Streaming);

        // What the model is shown: a side chat sees the whole conversation it
        // follows first, then its own questions. The parent's history is only
        // read, never written, so a side question cannot disturb the main chat.
        // Prune before the request is built: old tool output is the bulk of a
        // long conversation, and dropping it keeps every request cheap.
        let dropped = {
            let mut history = session.history.lock().await;
            prune_history(&mut history)
        };
        if dropped > 0 {
            session.push(
                "system",
                format!("dropped {dropped} old tool output(s) to keep the context light"),
            );
        }

        let mut context: Vec<InputItem> = Vec::new();
        if let Some(parent) = session.parent.as_deref() {
            // The map lock is dropped before the await: holding it would make
            // this future non-Send and stall every other session.
            let parent = {
                let sessions = shared.sessions.lock().expect("sessions");
                sessions.get(parent).cloned()
            };
            if let Some(parent) = parent {
                context = parent.history.lock().await.clone();
            }
        }
        context.extend(session.history.lock().await.clone());

        let request = ResponsesRequest {
            model: shared.agent.model(),
            stream: true,
            instructions: Some(instructions.clone()),
            input: context.clone(),
            tools: specs_with_mcp(&shared, &session).await,
            tool_choice: "auto",
            parallel_tool_calls: true,
            reasoning: Reasoning {
                effort: shared.agent.effort(),
                summary: "auto",
            },
            store: false,
            include: Vec::new(),
        };

        let active = shared.agent.provider();
        let provider = crate::providers::by_id(&active);
        let model = shared.agent.model();

        // A server-side or transport failure is worth another attempt; when the
        // primary model keeps failing, a fallback model gets one try of its own.
        let fallback = store::load_settings().fallback_model;
        let mut candidates = vec![model.clone()];
        if !fallback.is_empty() && fallback != model {
            candidates.push(fallback.clone());
        }
        let mut streamed: Option<mpsc::Receiver<ApiEvent>> = None;
        let mut last_error = String::new();
        for (index, candidate) in candidates.iter().enumerate() {
            // Retry every request failure; configuration controls total attempts.
            let max_attempts = store::load_settings().retry_attempts.clamp(1, 10);
            for attempt in 0..max_attempts {
                let mut attempt_request = request.clone();
                attempt_request.model = candidate.clone();
                let tried = match provider {
                    // The Codex backend keeps its own client: it is the only
                    // one using the OAuth token.
                    Some(entry)
                        if entry.wire_for(candidate) == crate::providers::Wire::CodexResponses =>
                    {
                        match shared.agent.codex() {
                            Ok(client) => client.stream(&attempt_request, &session.id).await,
                            Err(err) => Err(err),
                        }
                    }
                    Some(entry) => match request_target(entry, candidate, &session, &shared.agent.effort()).await {
                        Ok(target) => {
                            crate::backends::stream(
                                target,
                                instructions.clone(),
                                context.clone(),
                                specs_with_mcp(&shared, &session).await,
                            )
                            .await
                        }
                        Err(message) => Err(anyhow::anyhow!("{message}")),
                    },
                    None => Err(anyhow::anyhow!("unknown provider '{active}' — run /login")),
                };
                match tried {
                    Ok(stream) => {
                        streamed = Some(stream);
                        break;
                    }
                    Err(err) => {
                        last_error = format!("{err:#}");
                        // A failure that will not improve is not worth five
                        // attempts; a transient one is worth the wait.
                        if attempt + 1 >= max_attempts || !transient(&last_error) {
                            break;
                        }
                        // Retries used to be silent, which reads as nothing
                        // happening at all. Pushed and broadcast: a message
                        // that only lands in the transcript is not seen until
                        // the next attach, which is the same as silence.
                        let note = format!(
                            "{}: {} — retrying ({}/{max_attempts})",
                            candidate,
                            one_line(&last_error),
                            attempt + 2
                        );
                        session.push("tool", note.clone());
                        session.broadcast(WireEvent::Tool {
                            text: note,
                            line: WireMsgKind::Plain,
                        });
                        // Capped exponential backoff between attempts.
                        let delay = std::time::Duration::from_secs(1u64 << attempt.min(5));
                        tokio::time::sleep(delay).await;
                    }
                }
            }
            if streamed.is_some() || index + 1 >= candidates.len() {
                break;
            }
            if index + 1 < candidates.len() {
                let next = candidates
                    .get(index + 1)
                    .cloned()
                    .unwrap_or_else(|| fallback.clone());
                session.push(
                    "system",
                    format!("{candidate} is failing ({last_error}); trying {next}"),
                );
            }
        }
        let streamed = streamed.ok_or_else(|| anyhow::anyhow!(last_error));

        let mut stream = match streamed {
            Ok(stream) => stream,
            Err(err) => {
                let message = format!("{err:#}");
                if let Ok(mut state) = session.state.lock() {
                    state.error = Some(message.clone());
                }
                session.broadcast(WireEvent::Error { message });
                session.set_phase(WirePhase::Idle);
                session.broadcast(WireEvent::Idle);
                return;
            }
        };

        let mut text = String::new();
        let mut calls: Vec<(String, String, String)> = Vec::new();
        let mut usage = None;
        let mut failure = None;

        while let Some(event) = stream.recv().await {
            match event {
                ApiEvent::TextDelta(delta) => {
                    text.push_str(&delta);
                    if let Ok(mut state) = session.state.lock() {
                        state.live.push_str(&delta);
                    }
                    session.broadcast(WireEvent::Delta { text: delta });
                }
                ApiEvent::Preparing(note) => {
                    // The activity line's label, not a transcript line: a model
                    // writing a large file is doing one thing, and the line
                    // under the prompt should say which.
                    session.broadcast(WireEvent::Preparing { text: note });
                }
                ApiEvent::Reasoning(delta) => {
                    session.broadcast(WireEvent::Reasoning { text: delta });
                }
                ApiEvent::ItemDone(item) => {
                    if item.get("type").and_then(|t| t.as_str()) != Some("function_call") {
                        continue;
                    }
                    let call_id = item
                        .get("call_id")
                        .and_then(|v| v.as_str())
                        .unwrap_or_default()
                        .to_string();
                    if call_id.is_empty() {
                        continue;
                    }
                    let name = item
                        .get("name")
                        .and_then(|v| v.as_str())
                        .unwrap_or("shell")
                        .to_string();
                    let arguments = item
                        .get("arguments")
                        .and_then(|v| v.as_str())
                        .unwrap_or("{}")
                        .to_string();
                    calls.push((call_id, name, arguments));
                }
                ApiEvent::Completed(seen) => usage = Some(seen),
                ApiEvent::Failed(message) => failure = Some(message),
            }
        }

        if let Some(seen) = usage {
            let wire = usage_to_wire(seen);
            if let Ok(mut state) = session.state.lock() {
                state.usage = Some(wire.clone());
            }
            session.broadcast(WireEvent::Usage { usage: wire });
        }
        if let Some(message) = failure {
            if let Ok(mut state) = session.state.lock() {
                state.error = Some(message.clone());
            }
            session.broadcast(WireEvent::Error { message });
            break;
        }

        if !text.is_empty() {
            session.push("assistant", text.clone());
            session.broadcast(WireEvent::Assistant { text: text.clone() });
            session.history.lock().await.push(InputItem::Message {
                role: "assistant".to_string(),
                content: vec![ContentPart::OutputText { text }],
            });
        }

        if calls.is_empty() {
            finished = true;
            break;
        }

        // Labels first, so a batch of calls reads as a batch in the transcript.
        for (_, name, arguments) in &calls {
            // An MCP tool has no shell line worth showing; its own name is the
            // useful thing, since that is what the model asked for.
            let label = if shared.mcp.owns(name).await {
                format!("mcp: {name}")
            } else {
                command_preview(name, arguments)
            };
            session.push("tool", label.clone());
            session.broadcast(WireEvent::Tool {
                text: label,
                line: WireMsgKind::Plain,
            });
        }
        session.set_phase(WirePhase::Running);

        for (call_id, name, arguments, outcome) in run_calls(&shared, &session, calls).await {
            // A promoted command needs a watcher so its finish is not lost.
            if let Some(id) = outcome.background_id.clone() {
                watch_background(shared.clone(), session.clone(), id);
            }
            let shown = preview(&outcome.output);
            session.push_output(shown.clone());
            session.broadcast(WireEvent::Tool {
                text: shown,
                line: WireMsgKind::ToolOutput,
            });

            let mut history = session.history.lock().await;
            history.push(InputItem::FunctionCall {
                call_id: call_id.clone(),
                name,
                arguments,
            });
            history.push(InputItem::FunctionCallOutput {
                call_id,
                output: outcome.output,
            });
        }
        session.set_phase(WirePhase::Streaming);
    }

    if !finished {
        // The loop ran out of rounds rather than the model running out of work.
        // Saying so beats stopping in silence, which reads as being cut off.
        session.push(
            "system",
            format!(
                "stopped after {MAX_ITERATIONS} tool rounds in one turn; say \"continue\" and it picks up where it left off"
            ),
        );
    }

    compact_if_full(&shared, &session).await;

    let seconds = started.elapsed().as_secs();
    let human = if seconds >= 60 {
        format!("{}m {}s", seconds / 60, seconds % 60)
    } else {
        format!("{seconds}s")
    };
    let label = format!(
        "Worked for {human} · {}",
        chrono::Local::now().format("%I:%M %p")
    );
    session.push("system", label.clone());
    session.broadcast(WireEvent::Done { label });
    session.set_phase(WirePhase::Idle);
    session.broadcast(WireEvent::Idle);
    persist(&session).await;
}

    /// The first line of an error, short enough for a transcript note.
    pub(crate) fn one_line(message: &str) -> String {
        let first = message.lines().next().unwrap_or(message).trim();
        let mut out: String = first.chars().take(160).collect();
        if first.chars().count() > 160 {
            out.push('…');
        }
        out
    }

    /// Whether a failure deserves another attempt: server-side and transport
    /// problems do, a rejected request does not. It was `#[cfg(test)]` once, so
    /// the retry loop had nothing to consult and tried everything — including
    /// requests that would never work.
    pub(crate) fn transient(message: &str) -> bool {
        let lowered = message.to_ascii_lowercase();
        // A gateway can dress its own failure as a 400: opencode-go answers
        // "Upstream response was not valid JSON" with a `server_error` when its
        // upstream hiccups. That is worth another attempt; a 400 that means what
        // it says is not.
        if lowered.contains("server_error") || lowered.contains("not valid json") {
            return true;
        }
        // A rate limit is a wait, not a rejection.
        if lowered.contains("http 429") || lowered.contains("rate limit") {
            return true;
        }
        if lowered.contains("http 4") {
            return false;
        }
        [
            "http 5",
            "timed out",
            "timeout",
            "connection",
            "error sending request",
            "dns error",
            "failed to lookup address",
            "temporary failure",
            "temporary fail",
            "name resolution",
            "name or service not known",
            "temporary failure in name resolution",
            "could not resolve host",
            "no such host",
            "broken pipe",
            "reset by peer",
            "unexpected eof",
        ]
        .iter()
        .any(|needle| lowered.contains(needle))
}
/// The tools the model is offered: the built-in ones plus every MCP tool.
async fn specs_with_mcp(shared: &Arc<Shared>, session: &Arc<Session>) -> Vec<serde_json::Value> {
    let planning = session.mode() == "plan";
    let mut specs: Vec<serde_json::Value> = tools::specs()
        .into_iter()
        .filter(|spec| {
            if !planning {
                return true;
            }
            // Not offered at all in plan mode, so the model plans rather than
            // trying and being refused.
            !matches!(
                spec.get("name").and_then(|name| name.as_str()),
                Some("write") | Some("edit") | Some("delete")
            )
        })
        .collect();
    specs.extend(shared.mcp.specs().await);
    specs
}
