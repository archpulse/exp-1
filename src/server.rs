//! The background server: one process owns every session, every client attaches
//! to it over HTTP. Modelled on opencode's `serve`.

use anyhow::{Context, Result};
use axum::Router;
use axum::extract::{Path, Query, State};
use axum::http::{HeaderMap, StatusCode};
use axum::response::sse::{Event, Sse};
use axum::response::{IntoResponse, Json, Response};
use axum::routing::{get, post};
use futures_util::StreamExt;
use std::collections::HashMap;
use std::convert::Infallible;
use std::path::PathBuf;
use std::sync::{Arc, Mutex};
use std::time::Instant;
use tokio::net::TcpListener;
use tokio::sync::mpsc;

use crate::api::{ApiEvent, CodexClient};
use crate::catalog;
use crate::daemon;
use crate::proto::{
    AnswerBody, Health, LoginBody, PromptBody, RewindBody, ScopedBody, SessionSummary, WireAnswerRequest,
    WireEvent, WireMsg, WireMsgKind, WireEffort, WireModel, WirePhase, WireProvider, WireSnapshot,
    WireUsage,
};
use crate::protocol::{ContentPart, InputItem, Reasoning, ResponsesRequest, Usage};
use crate::tools;
use crate::tools::AskSlot;
use crate::store;

/// Tool rounds in one turn. Real work on a large project runs into the dozens;
/// the limit is a guard against a loop that never converges, not a budget.
const MAX_ITERATIONS: usize = 200;
/// Characters of a subagent's report the parent keeps; anything beyond is cut,
/// because the report lands in the parent's context.
const SUBAGENT_REPORT_CHARS: usize = 6_000;
/// How deep `task` may recurse before subagents stop being offered.
const MAX_SUBAGENT_DEPTH: usize = 2;

/// The subagent's own brief, kept deliberately narrow.
const SUBAGENT_INSTRUCTIONS: &str = r#"You are a subagent working for another agent. You have the same tools on the same machine, resolved against the current working directory.

Do the one task you were given, then stop. Do not expand its scope, start unrelated work, or spawn further subagents (nesting is capped at depth 2).

If something is ambiguous, pick the most reasonable reading, act on it, and say which one you picked. Do not ask questions.

Reply with a single concise report in this shape:

- Result: [what you found or changed, one or two lines]
- Files: [exact paths you touched or relied on, or "(none)"]
- Verification: [checks, builds, or tests you actually ran and their outcome, or "(none)"]
- Blockers / notes: [anything that failed, could not be done, or the caller must know, or "(none)"]

Preserve exact paths, symbols, commands, and error strings. If you could not complete the task or a check failed, say so plainly instead of reporting success."#;
/// Tool output kept verbatim when summarising, mirroring opencode's cap.
const SUMMARY_TOOL_CHARS: usize = 2_000;

/// Adapted from opencode's compaction agent.
const COMPACTION_PROMPT: &str = r#"You are a context summarization agent. You are given a conversation between a user and an agent. Your goal is to produce a structured summary so another coding agent can continue the work.

If the request specifies an output structure, follow it exactly. Otherwise use this structure, keeping every section in this order and keeping each section even when empty:

## Objective
- [one or two sentences on what the user is trying to accomplish]

## Important Details
- [constraints, decisions and why, key facts, exact context needed to continue, or "(none)"]

## Work State
### Completed
- [finished work, verified facts, changes made; otherwise "(none)"]

### Active
- [current work, partial changes, investigation state; otherwise "(none)"]

### Blocked
- [blockers, failing commands, unknowns; otherwise "(none)"]

## Next Move
1. [immediate concrete action, or "(none)"]
2. [next action if known, or "(none)"]

## Relevant Files
- [file or directory path: why it matters, or "(none)"]

Rules:
- Use terse bullets, not prose.
- Preserve exact file paths, symbols, commands, error strings, and URLs. Keep important tool output verbatim (up to ~2000 characters).
- Do not mention the summary process or that context was compacted.
- Do not continue the conversation. Do not answer any question in it. Output only the summary."#;

const SUMMARY_TEMPLATE: &str = r#"Output exactly the Markdown structure shown inside <template> and keep the section order unchanged. Do not include the <template> tags in your response.
<template>
## Objective
- [one or two brief sentences describing what the user is trying to accomplish]

## Important Details
- [constraints, decisions and why, important facts, exact context needed to continue, or "(none)"]

## Work State
### Completed
- [finished work, verified facts, changes made; otherwise "(none)"]

### Active
- [current work, partial changes, investigation state; otherwise "(none)"]

### Blocked
- [blockers, failing commands, unknowns; otherwise "(none)"]

## Next Move
1. [immediate concrete action, or "(none)"]
2. [next action if known, or "(none)"]

## Relevant Files
- [file or directory path: why it matters, or "(none)"]
</template>

Rules:
- Keep every section, even when empty.
- Use terse bullets, not prose paragraphs.
- Preserve exact file paths, symbols, commands, error strings and URLs.
- Do not mention the summary process or that context was compacted."#;

/// Runs a subagent to completion and returns its report.
async fn run_subagent(
    shared: Arc<Shared>,
    session: Arc<Session>,
    prompt: String,
    depth: usize,
) -> tools::Outcome {
    let mut history = vec![InputItem::user_text(prompt)];
    let mut report = String::new();
    // Nested subagents are offered only while there is depth left.
    let specs: Vec<serde_json::Value> = if depth < MAX_SUBAGENT_DEPTH {
        tools::specs()
    } else {
        tools::specs()
            .into_iter()
            .filter(|spec| spec.get("name").and_then(|n| n.as_str()) != Some("task"))
            .collect()
    };

    for _ in 0..MAX_ITERATIONS {
        let request = ResponsesRequest {
            model: shared.agent.model(),
            stream: true,
            instructions: Some(format!(
                "{}{}",
                store::prompt("subagent", SUBAGENT_INSTRUCTIONS),
                project_instructions(&session.cwd())
            )),
            input: history.clone(),
            tools: specs.clone(),
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
        let effort = shared.agent.effort();
        let request = ResponsesRequest {
            model: model.clone(),
            reasoning: Reasoning { effort: effort.clone(), summary: "auto" },
            ..request
        };

        let model = shared.agent.model();

        // Refresh subagent defaults from the current parent settings/model on
        // each tool invocation; setting edits take effect without a restart.
        let fallback = store::load_settings().fallback_model;
        let max_attempts = store::load_settings().retry_attempts.clamp(1, 10);
        let mut candidates = vec![model.clone()];
        if !fallback.is_empty() && fallback != model {
            candidates.push(fallback.clone());
        }
        let mut streamed = None;
        let mut last_error = String::new();
        for candidate in &candidates {
            for attempt in 0..max_attempts {
                let mut attempt_request = request.clone();
                attempt_request.model = candidate.clone();
                attempt_request.reasoning.effort = shared.agent.effort();
                let tried = match provider {
                    Some(entry) if entry.wire_for(candidate) == crate::providers::Wire::CodexResponses => {
                        match shared.agent.codex() {
                            Ok(client) => client.stream(&attempt_request, &session.id).await,
                            Err(err) => Err(err),
                        }
                    }
                    Some(entry) => match request_target(entry, candidate, &session).await {
                        Ok(target) => crate::backends::stream(
                            target,
                            store::prompt("subagent", SUBAGENT_INSTRUCTIONS),
                            history.clone(),
                            specs.clone(),
                        ).await,
                        Err(message) => Err(anyhow::anyhow!("{message}")),
                    },
                    None => Err(anyhow::anyhow!("unknown provider '{active}'")),
                };
                match tried {
                    Ok(stream) => { streamed = Some(stream); break; }
                    Err(err) => {
                        last_error = format!("{err:#}");
                        if attempt + 1 < max_attempts {
                            tokio::time::sleep(std::time::Duration::from_secs(1u64 << attempt.min(5))).await;
                        }
                    }
                }
            }
            if streamed.is_some() { break; }
        }
        let streamed = streamed.ok_or_else(|| anyhow::anyhow!(last_error.clone()));
        let Ok(mut stream) = streamed else {
            return tools::Outcome {
                output: format!("the subagent could not start: {last_error}"),
                background_id: None,
            };
        };

        let mut text = String::new();
        let mut calls: Vec<(String, String, String)> = Vec::new();
        while let Some(event) = stream.recv().await {
            match event {
                ApiEvent::TextDelta(delta) => text.push_str(&delta),
                ApiEvent::ItemDone(item) => {
                    if item.get("type").and_then(|t| t.as_str()) != Some("function_call") {
                        continue;
                    }
                    let call_id = item
                        .get("call_id")
                        .and_then(|v| v.as_str())
                        .unwrap_or_default()
                        .to_string();
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
                    if !call_id.is_empty() {
                        calls.push((call_id, name, arguments));
                    }
                }
                _ => {}
            }
        }

        if !text.is_empty() {
            report = text.clone();
            history.push(InputItem::Message {
                role: "assistant".to_string(),
                content: vec![ContentPart::OutputText { text }],
            });
        }
        if calls.is_empty() {
            break;
        }

        for (call_id, name, arguments) in calls {
            // One line that updates in place: a subagent's steps replace each
            // other instead of filling the transcript one line at a time.
            session.progress(format!("  subagent → {}", command_preview(&name, &arguments)));
            // A subagent follows the same rules as its parent.
            let refusal = permission_check(&session, &name, &arguments).await;
            let outcome = if let Some(refusal) = refusal {
                tools::Outcome {
                    output: refusal,
                    background_id: None,
                }
            } else if name == "task" {
                Box::pin(run_subagent(
                    shared.clone(),
                    session.clone(),
                    subagent_prompt(&arguments),
                    depth + 1,
                ))
                .await
            } else {
                tools::call(
                    &session.cwd(),
                    &shared.agent.registry,
                    &session.undo,
                    &session.ask,
                    &name,
                    &arguments,
                )
                .await
            };
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
    }

    tools::Outcome {
        output: if report.is_empty() {
            "the subagent finished without a report".to_string()
        } else {
            // A subagent's report lands in the parent's context: keep it tight.
            let kept = clip(&report, SUBAGENT_REPORT_CHARS);
            if kept.chars().count() < report.chars().count() {
                format!("{kept}\n... [report cut at {SUBAGENT_REPORT_CHARS} characters]")
            } else {
                kept
            }
        },
        background_id: None,
    }
}

fn subagent_prompt(arguments: &str) -> String {
    let value: serde_json::Value = serde_json::from_str(arguments).unwrap_or_default();
    value
        .get("prompt")
        .and_then(|p| p.as_str())
        .unwrap_or("Do the task described in the arguments.")
        .to_string()
}

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

/// Renders the conversation into the text the summariser reads.
fn history_to_text(history: &[InputItem]) -> String {
    let mut out = String::new();
    for item in history {
        match item {
            InputItem::Message { role, content } => {
                let text: Vec<&str> = content
                    .iter()
                    .map(|part| match part {
                        ContentPart::InputText { text } | ContentPart::OutputText { text } => {
                            text.as_str()
                        }
                        ContentPart::InputImage { .. } => "[image]",
                    })
                    .collect();
                out.push_str(&format!("[{role}] {}

", text.join("
")));
            }
            InputItem::FunctionCall { name, arguments, .. } => {
                out.push_str(&format!(
                    "[tool call] {name} {}
",
                    clip(arguments, 500)
                ));
            }
            InputItem::FunctionCallOutput { output, .. } => {
                out.push_str(&format!(
                    "[tool result] {}

",
                    clip(output, SUMMARY_TOOL_CHARS)
                ));
            }
        }
    }
    out
}

fn clip(text: &str, limit: usize) -> String {
    if text.chars().count() <= limit {
        return text.to_string();
    }
    let mut out: String = text.chars().take(limit).collect();
    out.push_str(" …");
    out
}

/// How many of the user's most recent turns are kept verbatim after a
/// compaction, together with everything the agent did in them.
const KEEP_RECENT_TURNS: usize = 5;

/// Everything from the `turns`-th most recent user message onwards, so the tail
/// carries the user's words and the agent's actions between them.
fn recent_tail(history: &[InputItem], turns: usize) -> Vec<InputItem> {
    let starts: Vec<usize> = history
        .iter()
        .enumerate()
        .filter(|(_, item)| {
            matches!(item, InputItem::Message { role, .. } if role == "user")
        })
        .map(|(index, _)| index)
        .collect();
    if starts.len() <= turns {
        return history.to_vec();
    }
    history[starts[starts.len() - turns]..].to_vec()
}

/// Roughly how many characters an item adds to a request.
fn item_chars(item: &InputItem) -> usize {
    match item {
        InputItem::Message { content, .. } => content
            .iter()
            .map(|part| match part {
                ContentPart::InputText { text } | ContentPart::OutputText { text } => {
                    text.chars().count()
                }
                // A picture costs roughly this much in most vision models.
                ContentPart::InputImage { .. } => 4_400,
            })
            .sum(),
        InputItem::FunctionCall { arguments, name, .. } => {
            arguments.chars().count() + name.chars().count()
        }
        InputItem::FunctionCallOutput { output, .. } => output.chars().count(),
    }
}

/// Rough token count: four characters to a token is close enough to decide
/// whether a tail still fits.
fn rough_tokens(items: &[InputItem]) -> u64 {
    (items.iter().map(item_chars).sum::<usize>() / 4) as u64
}

/// Tool output is what fills a long conversation, and every request re-sends
/// all of it. Before a request goes out, the oldest outputs — outside the last
/// turns, which are the working set — are replaced with a note saying what was
/// dropped: the model knows what it did and can run the tool again for anything
/// it still needs. The transcript the user sees keeps the full text.
const PRUNE_AT_CHARS: usize = 200_000;
const PRUNE_TO_CHARS: usize = 120_000;
const PRUNE_KEEP_TURNS: usize = 2;
/// Results the working set keeps whole, whatever the turns say: a single long
/// turn must not overflow the window before it ends.
const PRUNE_KEEP_OUTPUTS: usize = 6;

/// Replaces old tool output with a note; returns how many items were dropped.
fn prune_history(history: &mut [InputItem]) -> usize {
    // The last turns stay whole: that is the working set.
    let starts: Vec<usize> = history
        .iter()
        .enumerate()
        .filter(|(_, item)| matches!(item, InputItem::Message { role, .. } if role == "user"))
        .map(|(index, _)| index)
        .collect();
    let turn_from = if starts.len() > PRUNE_KEEP_TURNS {
        starts[starts.len() - PRUNE_KEEP_TURNS]
    } else {
        0
    };
    // …and so do the last few results, turn or no turn.
    let outputs: Vec<usize> = history
        .iter()
        .enumerate()
        .filter(|(_, item)| matches!(item, InputItem::FunctionCallOutput { .. }))
        .map(|(index, _)| index)
        .collect();
    let protected_from = if outputs.len() > PRUNE_KEEP_OUTPUTS {
        outputs[outputs.len() - PRUNE_KEEP_OUTPUTS]
    } else {
        history.len()
    };
    let scan_to = if turn_from == 0 { history.len() } else { turn_from };

    let mut total: usize = history.iter().map(item_chars).sum();
    if total <= PRUNE_AT_CHARS {
        return 0;
    }

    // What each call was, for the note that replaces its output.
    let mut labels: std::collections::HashMap<String, String> = std::collections::HashMap::new();
    for item in history.iter() {
        if let InputItem::FunctionCall {
            call_id,
            name,
            arguments,
        } = item
        {
            labels.insert(call_id.clone(), command_preview(name, arguments));
        }
    }

    let mut dropped = 0usize;
    for index in 0..scan_to {
        if total <= PRUNE_TO_CHARS {
            break;
        }
        if index >= protected_from {
            continue;
        }
        match &mut history[index] {
            InputItem::FunctionCallOutput { call_id, output } => {
                let was = output.chars().count();
                if was <= 400 {
                    continue;
                }
                let label = labels.get(call_id).map(String::as_str).unwrap_or("a tool");
                *output = format!(
                    "[output dropped to keep the context light: {label} returned {was} characters; run the tool again if you need it]"
                );
                total = total - was + output.chars().count();
                dropped += 1;
            }
            InputItem::FunctionCall { name, arguments, .. } => {
                let was = arguments.chars().count();
                if was <= 2_000 {
                    continue;
                }
                *arguments = format!(
                    "[arguments dropped to keep the context light: {name} was called with {was} characters of input]"
                );
                total = total - was + arguments.chars().count();
                dropped += 1;
            }
            _ => {}
        }
    }
    dropped
}

/// Replaces the conversation with a summary once it fills the context window.
async fn compact(shared: &Arc<Shared>, session: &Arc<Session>) -> Option<String> {
    let history = session.history.lock().await.clone();
    if history.is_empty() {
        return None;
    }
    let conversation = history_to_text(&history);

    let request = ResponsesRequest {
        model: shared.agent.model(),
        stream: true,
        instructions: Some(store::prompt("compaction", COMPACTION_PROMPT)),
        input: vec![InputItem::user_text(format!(
            "{SUMMARY_TEMPLATE}\n\n<conversation>\n{conversation}\n</conversation>"
        ))],
        tools: Vec::new(),
        tool_choice: "auto",
        parallel_tool_calls: false,
        reasoning: Reasoning {
            effort: "low".to_string(),
            summary: "auto",
        },
        store: false,
        include: Vec::new(),
    };

    let mut stream = shared
        .agent
        .codex()
        .ok()?
        .stream(&request, &session.id)
        .await
        .ok()?;
    let mut summary = String::new();
    while let Some(event) = stream.recv().await {
        if let ApiEvent::TextDelta(delta) = event {
            summary.push_str(&delta);
        }
    }
    let summary = summary.trim().to_string();
    if summary.is_empty() {
        return None;
    }

    // The summary opens the new conversation; the most recent turns stay
    // verbatim so the agent does not lose the thread it was holding.
    let head = InputItem::user_text(format!(
        "The earlier part of this conversation was compacted. Summary of everything so far:\n\n{summary}"
    ));
    let mut tail = recent_tail(&history, KEEP_RECENT_TURNS);

    // A single enormous turn could re-fill the window on its own, so drop the
    // oldest tail entries until the result fits again.
    let budget = model_context_limit(shared);
    while !tail.is_empty() && budget > 0 && rough_tokens(&tail) + rough_tokens(&[head.clone()]) > budget
    {
        let drop = tail
            .iter()
            .position(|item| matches!(item, InputItem::Message { role, .. } if role == "user"))
            .unwrap_or(0);
        tail.drain(..=drop.min(tail.len().saturating_sub(1)));
    }

    let mut next = vec![head];
    next.extend(tail);

    let mut guard = session.history.lock().await;
    *guard = next;
    Some(summary)
}

const INSTRUCTIONS: &str = r#"You are exp-1 agent, a coding assistant operating on the user's machine.

## Environment
- All tool paths are resolved against the session working directory. Use `/cd` to move to another project.
- Nothing is sandboxed: shell commands run as the user, with their full permissions. Be deliberate about anything destructive.

## Tools

- `read` — read a text file. `path` (required) is relative to the working directory. It returns **at most 400 lines** by default: pass `limit` for a different number and `pos` for a 1-based first line to start at. Output is prefixed with `path (N lines)` and every line is numbered. When the read was cut short, a trailing line says how many lines were shown and tells you to continue with a further `pos` — a whole 3000-line file is walked in pieces on purpose, because swallowing it costs tens of thousands of tokens of context. Read the part you need, not the file.
- `write` — create a file or overwrite it entirely. `path`, `content`.
- `delete` — delete a file. `path`. Directories are refused (use shell for those) and there is no undo, so delete only what you are sure about.
- `edit` — replace an exact snippet in a file. `path`, `old`, `new`; `old` must match the file exactly and only once, or the call fails with the closest lines (or the positions of the matches), so include enough context to be unique. `old` and `new` must differ. The result shows what changed, so there is no need to read the file back. To change several places in one file, pass `edits` (an array of `{"old", "new"}`, applied in order, all or nothing) instead of making several calls.
- `shell` — run a command. `command` is an **array of strings** (`["cargo", "test"]`), not a single string. Options:
  - Foreground by default: you wait for it to finish. A command still running after 60s is automatically promoted to a background task, so you are never blocked indefinitely.
  - `background: true` returns immediately with a `bg-…` id; you are woken with its output the moment it finishes, so poll with that id in `output_id` only to check progress.
  - Output is capped: a very long one keeps the beginning and the end, with the gap marked, because errors live at the end. Re-read any earlier run (foreground `out-…` id or background `bg-…` id) by passing its id in `output_id` together with a new `limit`.
  - A background command is never killed on a timer: a long build or a test suite runs to the end. `TERM=dumb` is set, so output is plain and non-interactive; `stdin` is closed, so nothing will wait for input.
- `ast` — show the directory tree of the project. `path` (defaults to the working directory), `refresh` (set true to rebuild instead of using the `.cache/ast` cache). The tree caps at a few thousand entries and skips `.git`.
- `count` — count the lines of a file, or of every file under a directory, without pulling the contents into the conversation. `path` (required), `refresh` (set true to recount instead of using the `.cache/count` cache). Answers are cached per file and recounted whenever its size or modification time change.
- `grep` — find where something is without reading whole files. `pattern` (required) is a regular expression (`(?i)` for case-insensitive), `path` (defaults to the working directory), `include` (a file-name pattern such as `*.rs`) and `max` (matches returned, default 100). Skips `.git`, `.cache`, `target` and `node_modules`, follows no symlinks, ignores non-text files. Answers are `path:line: text`, capped, and a broad pattern says how to narrow it.
- `fetch` — read one web page as plain text, paged like a file: `url` (required), `limit` (maximum lines, default 400) and `pos` (the 1-based line to start at). Use it when you already have a URL — documentation, a changelog, an issue — instead of searching for the page.
- `websearch` — search the web and get page contents back. `query` (required). Tune with `numResults` (default 8), `type` (`auto`|`fast`|`deep`), `livecrawl` (`fallback`|`preferred`), and `contextMaxCharacters` (default 10000). Use it for anything outside this machine: current versions, documentation, error messages, facts you cannot read from the filesystem.
- `ask_question` — ask the user a question and wait for the answer. `question` (required), with optional `options` to offer choices. Whatever the user types is taken as their answer. Use it when a decision is theirs to make and guessing would waste work — a choice of approach, a missing credential, an ambiguous requirement.
- `task` — spawn a subagent to do one focused piece of work in its own context and report back. `description` (three or four words) and `prompt` (the full task, with everything the subagent needs). Use it for work that would otherwise flood your context — a broad search across many files, a long investigation — or for something you can run in parallel with your own work.

## How to work

- Prefer the dedicated tools for file work: `read` to look, `edit` for a precise change, `write` for a new file, `delete` to remove one. Use `shell` for building, testing, running, and anything with no dedicated tool (git, moving files, removing directories).
- Read a file before editing it, and copy `old` from what you actually read. For `edit`, put in `old` the smallest snippet that uniquely identifies what to change. The result shows the lines around the change, so there is no need to read the file back to see what you did.
- Run `shell` commands in the foreground unless you genuinely need to keep working while one runs; then use `background` — you are told when it finishes. Re-read long output with a larger `limit` instead of guessing at it.
- Reach for `count` when the question is how big a file or a tree is in lines — never read a file just to count it.
- Reach for `grep` to find where something is; reading files to look for it wastes context.
- When you need several independent things at once — a few reads, greps or searches — ask for them in one message: they run at the same time, and one wait beats three.
- Long sessions drop old tool output to keep the context light; if you need something you saw earlier, run the tool again.
- Reach for `fetch` when you have the URL and `websearch` when you only have the question.
- Reach for `websearch` when the answer depends on something outside this machine.
- Ask with `ask_question` rather than guessing when a choice is the user's to make; delegate broad or parallel work to `task` instead of flooding your own context.
- Before you show the user a result, verify it by actually running the relevant checks with `shell` — the real build, the real test suite, the real type/lint command. Read the actual exit code and output; do not infer success from grepping for a phrase like "all tests passed" or from the code merely looking correct. If a check fails, fix it or report the failure plainly — never hide it.
- Be exacting and skeptical about your own work; never agreeable for its own sake. Question your assumptions, look for edge cases and regressions, and surface risks, gaps, and uncertainty instead of reassuring the user. When you have finished, state what you changed, what you verified, and what you could not verify.
- Never claim a check, build, or test passed unless you actually ran it and saw it pass."#;

/// Appended to the system prompt while the session is in plan mode; written to
/// `prompts/plan.md` so it can be edited like the rest.
const PLAN_NOTE: &str = r#"## Mode: plan

You are in plan mode. Investigate, read, search, and then propose what to do —
in enough detail that the plan can be followed without you.

- You cannot change anything: `write`, `edit` and `delete` are refused and are
  not even offered, and `shell` must stay read-only (no writes, no state changes).
- Read whatever you need: `read`, `ast`, `count`, shell commands that only
  inspect (`git status`, `git diff`, `grep`), and `websearch` or `fetch`.
- Finish with a concrete plan: the files to touch, the change to make in each,
  and how to verify it. Do not start implementing."#;

/// Everything a turn needs, shared by every session.
pub struct Agent {
    /// The Codex client, built the first time a Codex-protocol request needs it.
    /// Requiring its credentials at startup would stop the whole server for
    /// anyone signed into a different provider.
    codex: Mutex<Option<Arc<CodexClient>>>,
    /// Which registry entry is active; decides the wire protocol and the key.
    pub provider: Mutex<String>,
    pub model: Mutex<String>,
    pub effort: Mutex<String>,
    pub registry: tools::Shared,
    pub cwd: PathBuf,
}

impl Agent {
    /// The Codex client, loaded on demand and cached. A failure is not
    /// remembered: signing in afterwards makes the next call succeed.
    fn codex(&self) -> Result<Arc<CodexClient>> {
        let mut slot = self
            .codex
            .lock()
            .map_err(|_| anyhow::anyhow!("the Codex client lock was poisoned"))?;
        if let Some(client) = slot.as_ref() {
            return Ok(client.clone());
        }
        let credentials = crate::auth::Auth::load().context("load Codex credentials")?;
        let client = Arc::new(CodexClient::new(credentials)?);
        *slot = Some(client.clone());
        Ok(client)
    }

    /// Drops the cached client, so freshly stored credentials are picked up.
    fn forget_codex(&self) {
        if let Ok(mut slot) = self.codex.lock() {
            *slot = None;
        }
    }

    fn model(&self) -> String {
        self.model.lock().map(|g| g.clone()).unwrap_or_default()
    }
    fn effort(&self) -> String {
        self.effort.lock().map(|g| g.clone()).unwrap_or_default()
    }
    fn provider(&self) -> String {
        self.provider
            .lock()
            .map(|g| g.clone())
            .unwrap_or_else(|_| crate::providers::DEFAULT_PROVIDER.to_string())
    }
}

#[derive(Default)]
struct SessionState {
    title: String,
    /// `build` runs everything; `plan` refuses the tools that change files, so
    /// the model investigates and proposes instead of editing.
    mode: String,
    name: String,
    phase: WirePhase,
    transcript: Vec<WireMsg>,
    live: String,
    usage: Option<WireUsage>,
    error: Option<String>,
    question: Option<WireAnswerRequest>,
}

impl Default for WirePhase {
    fn default() -> Self {
        WirePhase::Idle
    }
}

pub struct Session {
    id: String,
    /// Working directory for this session's tools. A single server serves
    /// several projects, so this arrives with the request rather than coming
    /// from the server's own cwd; `/cd` changes it while the session lives.
    cwd: std::sync::Mutex<PathBuf>,
    state: Mutex<SessionState>,
    history: Arc<tokio::sync::Mutex<Vec<InputItem>>>,
    /// One channel per attached client, so nobody misses events.
    clients: Mutex<Vec<mpsc::UnboundedSender<WireEvent>>>,
    /// The session this one follows, for a side chat: its questions see the
    /// whole conversation they were opened from, without writing into it.
    parent: Option<String>,
    /// What this session's turns changed, for `/undo` and `/diff`.
    undo: Arc<std::sync::Mutex<tools::UndoLog>>,
    /// How many undo turns existed when each user message arrived, so a rewind
    /// knows how far back the files have to go.
    rewind_points: Mutex<Vec<usize>>,
    /// This session's window into `ask_question`, so two sessions never mix up
    /// their questions.
    ask: Arc<AskSlot>,
    agent: Mutex<Option<tokio::task::JoinHandle<()>>>,
}

impl Session {
    fn new(id: String, cwd: PathBuf, parent: Option<String>) -> Arc<Self> {
        Arc::new(Self {
            id,
            cwd: std::sync::Mutex::new(cwd),
            parent,
            state: Mutex::new(SessionState::default()),
            history: Arc::new(tokio::sync::Mutex::new(Vec::new())),
            clients: Mutex::new(Vec::new()),
            undo: Arc::new(std::sync::Mutex::new(tools::UndoLog::default())),
            rewind_points: Mutex::new(Vec::new()),
            ask: AskSlot::new(),
            agent: Mutex::new(None),
        })
    }

    /// Subagent picks up provider/model/effort changes from the parent each time.
    fn mode(&self) -> String {
        self.state
            .lock()
            .map(|state| state.mode.clone())
            .unwrap_or_else(|_| "build".to_string())
    }

    /// The directory the tools of this session work in.
    fn cwd(&self) -> PathBuf {
        self.cwd
            .lock()
            .map(|cwd| cwd.clone())
            .unwrap_or_else(|_| PathBuf::from("."))
    }

    fn broadcast(&self, event: WireEvent) {
        if let Ok(mut clients) = self.clients.lock() {
            clients.retain(|sender| sender.send(event.clone()).is_ok());
        }
    }

    fn subscribe(&self) -> mpsc::UnboundedReceiver<WireEvent> {
        let (tx, rx) = mpsc::unbounded_channel();
        if let Ok(mut clients) = self.clients.lock() {
            clients.push(tx);
        }
        rx
    }

    fn snapshot(&self) -> WireSnapshot {
        let state = self.state.lock().expect("session state");
        WireSnapshot {
            id: self.id.clone(),
            title: state.title.clone(),
            name: state.name.clone(),
            phase: state.phase,
            transcript: state.transcript.clone(),
            live: state.live.clone(),
            usage: state.usage.clone(),
            mode: if state.mode.is_empty() {
                "build".to_string()
            } else {
                state.mode.clone()
            },
            error: state.error.clone(),
            question: state.question.clone(),
        }
    }

    fn push(&self, role: &str, text: impl Into<String>) {
        self.push_kind(role, text, WireMsgKind::Plain);
    }

    /// A tool's output: kept in the transcript, hidden by default in the UI.
    fn push_output(&self, text: impl Into<String>) {
        self.push_kind("tool", text, WireMsgKind::ToolOutput);
    }

    /// A line the next one replaces: a subagent's current step, so a long
    /// investigation does not grow the transcript one step at a time.
    fn progress(&self, text: impl Into<String>) {
        let text = text.into();
        if let Ok(mut state) = self.state.lock() {
            match state.transcript.last_mut() {
                Some(last) if last.kind == WireMsgKind::Progress => last.text = text.clone(),
                _ => state.transcript.push(WireMsg {
                    role: "tool".to_string(),
                    text: text.clone(),
                    images: 0,
                    kind: WireMsgKind::Progress,
                }),
            }
        }
        self.broadcast(WireEvent::Tool {
            text,
            line: WireMsgKind::Progress,
        });
    }

    fn push_kind(&self, role: &str, text: impl Into<String>, kind: WireMsgKind) {
        if let Ok(mut state) = self.state.lock() {
            state.transcript.push(WireMsg {
                role: role.to_string(),
                text: text.into(),
                images: 0,
                kind,
            });
        }
    }

    fn set_phase(&self, phase: WirePhase) {
        if let Ok(mut state) = self.state.lock() {
            state.phase = phase;
            if phase != WirePhase::Streaming {
                state.live.clear();
            }
        }
        self.broadcast(WireEvent::Phase { phase });
    }
}

/// `POST /config` — the model and effort are server-wide, so every attached UI
/// changes the same values rather than keeping private copies.
#[derive(Debug, serde::Serialize, serde::Deserialize)]
pub struct ConfigBody {
    pub model: Option<String>,
    pub effort: Option<String>,
}

#[derive(Debug, serde::Serialize, serde::Deserialize)]
pub struct WireConfig {
    pub model: String,
    pub effort: String,
    pub plan: Option<String>,
}

/// What a `/login` actually did, so the client can say the truth: a provider
/// that was already signed in is switched to, not signed into again.
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum LoginOutcome {
    /// A credential was already on disk; the provider was just activated.
    Switched,
    /// A pasted key was stored and the provider activated.
    SignedIn,
    /// A device flow is running; the page and code are in the response.
    Device,
}

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct WireLogin {
    pub outcome: LoginOutcome,
    pub label: String,
    pub model: String,
    pub effort: String,
    #[serde(default)]
    pub verification_uri: Option<String>,
    #[serde(default)]
    pub user_code: Option<String>,
    /// The line the client shows, in the same words the session received.
    pub message: String,
}

/// Whether a credential for this provider is already on the machine. The
/// Codex-style provider keeps its tokens in its own file, and Vertex takes
/// whatever gcloud is signed in with, so neither is in `credentials.json`.
fn has_credential(provider: &crate::providers::Provider) -> bool {
    match provider.id {
        "openai-auth" => crate::auth::Auth::load().is_ok(),
        "vertex" => true,
        _ => store::api_key(provider.id).is_some(),
    }
}

struct Shared {
    password: String,
    agent: Arc<Agent>,
    /// Loaded once; the window sizes decide when to compact.
    catalog: Vec<catalog::ModelInfo>,
    /// The MCP servers and the tools they offer, filled in the background.
    mcp: Arc<crate::mcp::Registry>,
    plan: Mutex<Option<String>>,
    sessions: Mutex<HashMap<String, Arc<Session>>>,
}

fn read_lock(value: &Mutex<String>) -> String {
    value.lock().map(|guard| guard.clone()).unwrap_or_default()
}

fn usage_to_wire(usage: Usage) -> WireUsage {
    WireUsage {
        input_tokens: usage.input_tokens,
        output_tokens: usage.output_tokens,
        cached_tokens: usage.cached_tokens,
    }
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
        .or_else(|| plan_refusal(session, name));
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

async fn run_turn(
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
                    Some(entry) => match request_target(entry, candidate, &session).await {
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
                        if attempt + 1 < max_attempts {
                            // Capped exponential backoff between attempts.
                            let delay = std::time::Duration::from_secs(1u64 << attempt.min(5));
                            tokio::time::sleep(delay).await;
                        }
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

/// Writes a session to `~/.config/exp-1/sessions/`, so `/sessions` on any client
/// can reopen it and a restarted server still finds it.
async fn persist(session: &Arc<Session>) {
    let (title, name, transcript) = {
        let state = match session.state.lock() {
            Ok(state) => state,
            Err(_) => return,
        };
        (
            state.title.clone(),
            state.name.clone(),
            state
                .transcript
                .iter()
                .skip(state.transcript.len().saturating_sub(MAX_STORED_MESSAGES))
                .map(|msg| store::StoredMsg {
                    role: msg.role.clone(),
                    text: msg.text.clone(),
                    kind: msg.kind,
                })
                .collect::<Vec<_>>(),
        )
    };
    let history = session.history.lock().await.clone();
    let stored = store::StoredSession {
        id: session.id.clone(),
        title,
        updated: store::now_iso(),
        directory: session.cwd().to_string_lossy().to_string(),
        main: store::StoredThread {
            name,
            transcript,
            history,
            input_history: Vec::new(),
        },
        side: store::StoredThread::default(),
    };
    let _ = store::save_session(&stored);
}

/// How many messages of a conversation are kept on disk. A transcript line is
/// small — tool output is previewed, not stored whole — so this is generous on
/// purpose: a long conversation should still be there when it is reopened. The
/// model's own context is bounded separately by pruning and compaction.
const MAX_STORED_MESSAGES: usize = 5_000;

fn command_preview(name: &str, arguments: &str) -> String {
    let value: serde_json::Value = match serde_json::from_str(arguments) {
        Ok(value) => value,
        Err(_) => return name.to_string(),
    };
    let path = |key: &str| {
        value
            .get(key)
            .and_then(|v| v.as_str())
            .map(str::to_string)
    };
    match name {
        "read" => path("path").map(|p| format!("read {p}")).unwrap_or_default(),
        "write" => path("path").map(|p| format!("write {p}")).unwrap_or_default(),
        "edit" => path("path").map(|p| format!("edit {p}")).unwrap_or_default(),
        "delete" => path("path")
            .map(|p| format!("delete {p}"))
            .unwrap_or_default(),
        "ast" => "ast".to_string(),
        "count" => path("path")
            .map(|p| format!("count {p}"))
            .unwrap_or_default(),
        "grep" => path("pattern")
            .map(|p| format!("grep {p}"))
            .unwrap_or_default(),
        "fetch" => path("url").map(|u| format!("fetch {u}")).unwrap_or_default(),
        "websearch" => path("query")
            .map(|q| format!("search: {q}"))
            .unwrap_or_else(|| "websearch".to_string()),
        "ask_question" => path("question")
            .map(|q| format!("ask: {q}"))
            .unwrap_or_else(|| "ask".to_string()),
        // A subagent call is named by its description, not by the whole task
        // text it carries.
        "task" => path("description")
            .map(|d| format!("task: {d}"))
            .unwrap_or_else(|| "task".to_string()),
        _ => {
            let argv = value
                .get("command")
                .and_then(|c| c.as_array())
                .map(|items| {
                    items
                        .iter()
                        .filter_map(|i| i.as_str())
                        .collect::<Vec<_>>()
                        .join(" ")
                })
                .or_else(|| path("cmd"))
                .unwrap_or_else(|| arguments.to_string());
            format!("$ {argv}")
        }
    }
}

fn preview(text: &str) -> String {
    let collapsed: Vec<&str> = text.lines().take(12).collect();
    let mut out = collapsed.join("\n");
    if text.lines().count() > 12 || out.chars().count() > 400 {
        // Truncate at the 400th character, not the 400th byte: slicing a
        // multi-byte character in half panics, and Cyrillic output is routine.
        let end = out
            .char_indices()
            .nth(400)
            .map(|(at, _)| at)
            .unwrap_or(out.len());
        out.truncate(end);
        out.push_str("\n…");
    }
    out
}

// ------------------------------------------------------------------ routes

fn authorized(shared: &Shared, headers: &HeaderMap) -> bool {
    headers
        .get("authorization")
        .and_then(|value| value.to_str().ok())
        .and_then(|value| value.strip_prefix("Bearer "))
        .map(|token| token == shared.password)
        .unwrap_or(false)
}

fn unauthorized() -> Response {
    (StatusCode::UNAUTHORIZED, "missing or wrong server password").into_response()
}

async fn health(State(shared): State<Arc<Shared>>, headers: HeaderMap) -> Response {
    if !authorized(&shared, &headers) {
        return unauthorized();
    }
    Json(Health {
        healthy: true,
        version: env!("CARGO_PKG_VERSION").to_string(),
    })
    .into_response()
}

async fn list_sessions(State(shared): State<Arc<Shared>>, headers: HeaderMap) -> Response {
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

/// The media type of a picture, by its extension.
fn media_type(path: &std::path::Path) -> &'static str {
    match path
        .extension()
        .and_then(|ext| ext.to_str())
        .map(str::to_ascii_lowercase)
        .as_deref()
    {
        Some("jpg") | Some("jpeg") => "image/jpeg",
        Some("gif") => "image/gif",
        Some("webp") => "image/webp",
        _ => "image/png",
    }
}

fn is_image(path: &std::path::Path) -> bool {
    matches!(
        path.extension()
            .and_then(|ext| ext.to_str())
            .map(str::to_ascii_lowercase)
            .as_deref(),
        Some("png") | Some("jpg") | Some("jpeg") | Some("gif") | Some("webp")
    )
}

/// In plan mode the tools that change files are refused before they run.
fn plan_refusal(session: &Arc<Session>, name: &str) -> Option<String> {
    if session.mode() != "plan" {
        return None;
    }
    matches!(name, "write" | "edit" | "delete").then(|| {
        format!("{name}: refused — the session is in plan mode (shift+tab to leave it)")
    })
}

/// What a permission rule matches for this call: the path for file tools, the
/// command line for shell, the URL or query for the net tools.
fn permission_resource(name: &str, arguments: &str, cwd: &std::path::Path) -> String {
    let Ok(value) = serde_json::from_str::<serde_json::Value>(arguments) else {
        return arguments.to_string();
    };
    let text = |key: &str| {
        value
            .get(key)
            .and_then(|field| field.as_str())
            .map(str::to_string)
    };
    match name {
        "read" | "write" | "edit" | "delete" | "count" | "grep" => text("path")
            .map(|raw| {
                let path = std::path::Path::new(&raw);
                if path.is_absolute() {
                    raw
                } else {
                    cwd.join(path).display().to_string()
                }
            })
            .unwrap_or_else(|| arguments.to_string()),
        "shell" => value
            .get("command")
            .and_then(|command| command.as_array())
            .map(|items| {
                items
                    .iter()
                    .filter_map(|item| item.as_str())
                    .collect::<Vec<_>>()
                    .join(" ")
            })
            .unwrap_or_else(|| arguments.to_string()),
        "fetch" => text("url").unwrap_or_else(|| arguments.to_string()),
        "websearch" => text("query").unwrap_or_else(|| arguments.to_string()),
        _ => arguments.to_string(),
    }
}

/// Asks the rules file, and when it says so the user, whether a call may run.
/// `None` means it may; `Some(reason)` is what to hand back instead.
async fn permission_check(
    session: &Arc<Session>,
    name: &str,
    arguments: &str,
) -> Option<String> {
    let resource = permission_resource(name, arguments, &session.cwd());
    let decision = tools::load_rules()
        .decide(name, &resource)
        .map(|rule| rule.effect.clone());
    match decision.as_deref() {
        Some("deny") => Some(format!(
            "{name}: not permitted — permissions.json denies `{resource}`"
        )),
        Some("ask") => {
            let question = tools::AskItem {
                label: "Permission".to_string(),
                text: format!("Allow `{name}` on `{resource}`?"),
                options: vec!["Allow".to_string(), "Deny".to_string()],
            };
            let allowed = session
                .ask
                .ask_within(vec![question], "Submit".to_string(), tools::PERMISSION_TIMEOUT)
                .await
                .and_then(|answers| answers.first().cloned())
                .map(|answer| {
                    let answer = answer.to_ascii_lowercase();
                    answer.contains("allow") || matches!(answer.trim(), "y" | "yes")
                })
                .unwrap_or(false);
            if allowed {
                None
            } else {
                Some(format!("{name}: refused by the user (`{resource}`)"))
            }
        }
        _ => None,
    }
}

    /// Whether a failure deserves another attempt: server-side and transport
    /// problems do, a rejected request does not.
    #[cfg(test)]
    fn transient(message: &str) -> bool {
    let lowered = message.to_ascii_lowercase();
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

#[cfg(test)]
mod tests {
    use super::*;

    /// Old tool output is dropped before a request goes out; the last turns
    /// stay whole, and the note names the call it replaced.
    #[test]
    fn old_tool_output_is_dropped_from_requests() {
        let big = "x".repeat(60_000);
        let mut history = Vec::new();
        for turn in 0..5 {
            history.push(InputItem::user_text(format!("question {turn}")));
            history.push(InputItem::FunctionCall {
                call_id: format!("call-{turn}"),
                name: "read".to_string(),
                arguments: format!("{{\"path\": \"file-{turn}.rs\"}}"),
            });
            history.push(InputItem::FunctionCallOutput {
                call_id: format!("call-{turn}"),
                output: big.clone(),
            });
        }
        let before: usize = history.iter().map(item_chars).sum();
        assert!(before > PRUNE_AT_CHARS, "the fixture must be over the limit");

        let dropped = prune_history(&mut history);
        let after: usize = history.iter().map(item_chars).sum();
        assert!(dropped >= 3, "the oldest outputs go first: {dropped}");
        assert!(before - after > 150_000, "most of the bulk is gone: {}", before - after);
        assert!(after < PRUNE_AT_CHARS, "{after}");

        let outputs: Vec<&String> = history
            .iter()
            .filter_map(|item| match item {
                InputItem::FunctionCallOutput { output, .. } => Some(output),
                _ => None,
            })
            .collect();
        assert_eq!(outputs.len(), 5);
        assert!(outputs[3] == &big && outputs[4] == &big, "the working set stays whole");
        assert!(outputs[0].contains("dropped"), "{}", outputs[0]);
        assert!(outputs[0].contains("read file-0.rs"), "the note names the call: {}", outputs[0]);
    }

    /// One long turn: the oldest results go, the last few stay whole, so a
    /// twenty-call turn cannot overflow the window before it ends.
    #[test]
    fn a_single_long_turn_is_pruned_too() {
        let big = "y".repeat(40_000);
        let mut history = vec![InputItem::user_text("do the thing")];
        for call in 0..10 {
            history.push(InputItem::FunctionCall {
                call_id: format!("call-{call}"),
                name: "shell".to_string(),
                arguments: "{\"command\": [\"ls\"]}".to_string(),
            });
            history.push(InputItem::FunctionCallOutput {
                call_id: format!("call-{call}"),
                output: big.clone(),
            });
        }
        let before: usize = history.iter().map(item_chars).sum();
        assert!(before > PRUNE_AT_CHARS, "{before}");

        let dropped = prune_history(&mut history);
        let after: usize = history.iter().map(item_chars).sum();
        assert!(dropped >= 2, "{dropped}");
        assert!(before - after > 80_000, "{}", before - after);
        assert!(after < before);
    }

    /// A tool preview is cut at a character boundary. The old code truncated at
    /// byte 400, which panics inside a multi-byte character; this sample is
    /// built so byte 400 lands in the middle of a Cyrillic letter.
    #[test]
    fn a_preview_never_splits_a_character() {
        // Each unit is 27 bytes and 400 = 27*14 + 22, which is inside `р`.
        let sample = "привет — мир!   ".repeat(200);
        assert!(!sample.is_char_boundary(400), "the sample must reproduce the trap");

        let shown = preview(&sample);
        assert!(shown.ends_with('…'), "{}", shown);
        let kept = shown.trim_end_matches('…').trim_end_matches('\n');
        assert!(sample.starts_with(kept), "nothing was mangled");
        assert!(shown.chars().count() <= 402, "cut at the 400th character");
    }

    /// A failure that is worth another attempt, and one that is not.
    #[test]
    fn only_transient_failures_are_retried() {
        assert!(transient("HTTP 500 Internal Server Error: upstream"));
        assert!(transient("error sending request for url (https://api.openai.com)"));
        assert!(transient("operation timed out after 25s"));
        assert!(transient("connection reset by peer"));
        assert!(transient("dns error: failed to lookup address information"));
        assert!(transient("Temporary failure: DNS lookup failed"));
        assert!(transient("temporary failure in name resolution"));
        assert!(!transient("HTTP 404: model not found"));
        assert!(!transient("HTTP 401: invalid api key"));
        assert!(!transient("unknown provider 'nope' — run /login"));
    }
}

/// What a project says about itself: `AGENTS.md` from the working directory and
/// every parent above it, outermost first so the nearest file wins by being
/// read last.
fn project_instructions(cwd: &std::path::Path) -> String {
    let mut files: Vec<PathBuf> = Vec::new();
    let mut dir = Some(cwd);
    while let Some(current) = dir {
        let candidate = current.join("AGENTS.md");
        if candidate.is_file() {
            files.push(candidate);
        }
        dir = current.parent();
    }
    files.reverse();
    let mut out = String::new();
    for path in files {
        let Ok(text) = std::fs::read_to_string(&path) else {
            continue;
        };
        if text.trim().is_empty() {
            continue;
        }
        out.push_str(&format!(
            "\n\n# Instructions from {}\n\n{}",
            path.display(),
            text.trim()
        ));
    }
    out
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

/// `/skills`: what is installed.
async fn list_skills(State(shared): State<Arc<Shared>>, headers: HeaderMap) -> Response {
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
async fn run_skill(
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
async fn cd(
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
async fn list_mcp(State(shared): State<Arc<Shared>>, headers: HeaderMap) -> Response {
    if !authorized(&shared, &headers) {
        return unauthorized();
    }
    Json(shared.mcp.status().await).into_response()
}

/// `shift+tab`: switch the session between building and planning.
async fn set_mode(
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

/// `/clear`: start this session over. The transcript on screen is only half of
/// a conversation — the other half is the history the model is sent — so both
/// go, along with the undo log, and the title is taken again on the next prompt.
async fn clear_session(
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
async fn fork_session(
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
async fn undo(
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
async fn rewind(
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
async fn show_diff(
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
async fn compact_now(
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
async fn retitle(
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

async fn create_session(State(shared): State<Arc<Shared>>, headers: HeaderMap) -> Response {
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

async fn session_snapshot(
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
            error: None,
            question: None,
        })
        .into_response(),
        None => (StatusCode::NOT_FOUND, "no such session").into_response(),
    }
}

async fn prompt(
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

/// Titles the session from its first prompt, off the turn's critical path.
fn maybe_name(shared: Arc<Shared>, session: Arc<Session>, first_prompt: String) {
    {
        let state = session.state.lock().expect("session state");
        if !state.title.is_empty() {
            return;
        }
    }
    // A side chat belongs to the conversation it follows and is never named.
    if session.parent.is_some() {
        return;
    }
    tokio::spawn(async move {
        let (title, usage) = ask_for_title(&shared, &first_prompt).await;
        record_title_usage(&session, usage);
        store_title(&session, title);
    });
}

/// Names a session again from the original request and the recent conversation,
/// which is what `/retitle` runs — opencode's `session.update` with an empty
/// title does the same thing.
fn regenerate_title(shared: Arc<Shared>, session: Arc<Session>) {
    tokio::spawn(async move {
        let text = conversation_for_title(&session).await;
        let (title, usage) = ask_for_title(&shared, &text).await;
        record_title_usage(&session, usage);
        store_title(&session, title);
    });
}

async fn answer(
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

/// Builds the request target for a provider. GitHub Copilot is the odd one:
/// its stored credential is a long-lived GitHub token, and the host and bearer
/// token are both derived from a short-lived exchange done per request.
/// Headers the opencode gateways want on every call. They route by session and
/// refuse a request without it outright: "Request is missing x-opencode-session
/// and cannot be routed efficiently".
fn gateway_headers(
    provider: &'static crate::providers::Provider,
    session: &Arc<Session>,
) -> Vec<(String, String)> {
    if crate::providers::key_group(provider.id) != Some("opencode") {
        return Vec::new();
    }
    vec![
        ("x-opencode-session".to_string(), session.id.clone()),
        (
            "x-opencode-project".to_string(),
            session.cwd().display().to_string(),
        ),
        ("x-opencode-client".to_string(), "exp-1-agent".to_string()),
    ]
}

async fn request_target(
    provider: &'static crate::providers::Provider,
    model: &str,
    session: &Arc<Session>,
) -> Result<crate::backends::Target, String> {
    let stored = store::api_key(provider.id);
    let wire = provider.wire_for(model);
    let headers = gateway_headers(provider, session);

    // Vertex mints an access token from gcloud's application default
    // credentials, and builds its path from the project and location.
    if provider.id == "vertex" {
        let config = crate::oauth::vertex_config()
            .await
            .map_err(|err| format!("{err:#}"))?;
        let model_path = format!(
            "/projects/{}/locations/{}/publishers/google/models/{}:streamGenerateContent?alt=sse",
            config.project, config.location, model
        );
        return Ok(crate::backends::Target {
            provider,
            wire,
            endpoint: format!(
                "https://{}-aiplatform.googleapis.com/v1{model_path}",
                config.location
            ),
            key: Some(config.access_token),
            model: model.to_string(),
            headers,
        });
    }

    // GitLab Duo rejects a personal access token directly; it has to be
    // exchanged for the OIDC token the gateway accepts.
    if provider.id == "gitlab-duo" {
        let pat = stored.ok_or_else(|| "store a GitLab token with /login first".to_string())?;
        let instance = std::env::var("GITLAB_INSTANCE_URL")
            .unwrap_or_else(|_| "https://gitlab.com".to_string());
        let oidc = crate::oauth::gitlab_oidc(&pat, &instance)
            .await
            .map_err(|err| format!("{err:#}"))?;
        return Ok(crate::backends::Target {
            provider,
            wire,
            endpoint: provider.endpoint_for(model),
            key: Some(oidc),
            model: model.to_string(),
            headers,
        });
    }

    if provider.id == "github-copilot" {
        let github = stored.ok_or_else(|| "sign in to github copilot with /login first".to_string())?;
        let (host, token) = crate::oauth::copilot_bearer(&github)
            .await
            .map_err(|err| format!("{err:#}"))?;
        return Ok(crate::backends::Target {
            provider,
            wire,
            endpoint: format!("{host}{}", wire.suffix()),
            key: Some(token),
            model: model.to_string(),
            headers,
        });
    }

    Ok(crate::backends::Target {
        provider,
        wire,
        endpoint: provider.endpoint_for(model),
        key: stored,
        model: model.to_string(),
        headers,
    })
}

/// Writes a freshly obtained Codex-style auth file into our own state dir.
fn store_auth_file(raw: &str) -> Result<(), String> {
    let path = crate::auth::Auth::state_path();
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent).map_err(|err| err.to_string())?;
    }
    std::fs::write(&path, raw).map_err(|err| err.to_string())
}

/// A conservative window for providers whose catalogue we do not have.
fn provider_context_window() -> u64 {
    128_000
}

/// The active model's compaction threshold, zero when the model is unknown.
fn model_context_limit(shared: &Arc<Shared>) -> u64 {
    let slug = shared.agent.model();
    if let Some(model) = shared.catalog.iter().find(|model| model.slug == slug) {
        return model.context_limit();
    }
    // A model the Codex cache does not list: models.dev knows its window, and
    // without one the conversation would never be compacted at all.
    crate::providers::catalog_id(&shared.agent.provider())
        .and_then(|catalog| crate::models_dev::context_limit(catalog, &slug))
        .map(|window| window * catalog::compact_threshold_percent() / 100)
        .unwrap_or(0)
}

/// Compacts when the last turn filled 95% of the model's context window.
async fn compact_if_full(shared: &Arc<Shared>, session: &Arc<Session>) -> bool {
    let used = session
        .state
        .lock()
        .ok()
        .and_then(|state| state.usage.as_ref().map(|usage| usage.input_tokens))
        .unwrap_or(0);
    let limit = model_context_limit(shared);
    if limit == 0 || used < limit {
        return false;
    }

    session.push(
        "system",
        format!(
            "Context at {used} of {limit} tokens ({}% threshold) — compacting",
            catalog::compact_threshold_percent()
        ),
    );
    session.broadcast(WireEvent::Tool {
        text: "compacting the conversation…".to_string(),
        line: WireMsgKind::Plain,
    });

    if compact(shared, session).await.is_some() {
        session.push("system", "Conversation compacted; continuing from the summary");
        session.broadcast(WireEvent::Tool {
            text: "context compacted".to_string(),
            line: WireMsgKind::Plain,
        });
        true
    } else {
        session.push("system", "Compaction failed; keeping the full conversation");
        false
    }
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

async fn interrupt(
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
async fn events(
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

// ------------------------------------------------------------------ titles

/// The bundled T5, loaded at most once. Compiled in only with the `titler`
/// feature; without it, titles come from the model or not at all.
#[cfg(feature = "titler")]
static TITLER: std::sync::OnceLock<Mutex<Option<titler::Titler>>> = std::sync::OnceLock::new();

#[cfg(feature = "titler")]
fn local_title(prompt: &str) -> Option<String> {
    let cell = TITLER
        .get_or_init(|| Mutex::new(titler::Titler::from_hub("Cyronius/titler", None).ok()));
    let mut guard = cell.lock().ok()?;
    let model = guard.as_mut()?;
    model
        .generate(prompt)
        .ok()
        .map(|title| title.trim().to_string())
        .filter(|title| !title.is_empty())
}

#[cfg(not(feature = "titler"))]
fn local_title(_prompt: &str) -> Option<String> {
    None
}

/// The model that names sessions. opencode asks the provider for its small
/// model; this catalogue carries no such flag, so the cheap `luna` tier is
/// preferred by name and the session's own model is the fallback.
fn title_model(shared: &Arc<Shared>) -> String {
    catalog::load()
        .into_iter()
        .find(|model| model.listed && model.slug.contains("luna"))
        .map(|model| model.slug)
        .unwrap_or_else(|| shared.agent.model())
}

/// Asks the Codex model for a title, returning it with what it cost.
async fn remote_title(shared: &Arc<Shared>, text: &str) -> (Option<String>, Option<Usage>) {
    let request = ResponsesRequest {
        model: title_model(shared),
        stream: true,
        instructions: Some(store::prompt("title", TITLE_PROMPT)),
        // The prompt carries the instructions; the model gets the text alone,
        // exactly as opencode sends it.
        input: vec![InputItem::user_text(text)],
        tools: Vec::new(),
        tool_choice: "auto",
        parallel_tool_calls: false,
        reasoning: Reasoning {
            effort: "low".to_string(),
            summary: "auto",
        },
        store: false,
        include: Vec::new(),
    };
    let Ok(client) = shared.agent.codex() else {
        return (None, None);
    };
    let Ok(mut stream) = client.stream(&request, "exp1-title").await else {
        return (None, None);
    };
    let mut text_out = String::new();
    let mut usage = None;
    while let Some(event) = stream.recv().await {
        match event {
            ApiEvent::TextDelta(delta) => text_out.push_str(&delta),
            ApiEvent::Completed(seen) => usage = Some(seen),
            _ => {}
        }
    }
    let title = text_out
        .lines()
        .map(str::trim)
        .find(|line| !line.is_empty())
        .map(|line| line.trim_matches(['"', '\u{27}', '`']).trim().to_string())
        .filter(|line| !line.is_empty());
    (title, usage)
}

/// The title, from whichever generator the setting asks for: the Codex model by
/// default, the bundled 7.6M titler as the offline fallback, or only one of
/// them when the setting says so.
async fn ask_for_title(shared: &Arc<Shared>, text: &str) -> (Option<String>, Option<Usage>) {
    let mode = store::load_settings().titler;
    if mode == "local" {
        let prompt = text.to_string();
        let title = tokio::task::spawn_blocking(move || local_title(&prompt))
            .await
            .ok()
            .flatten();
        return (title, None);
    }
    let (title, usage) = remote_title(shared, text).await;
    if title.is_some() || mode == "remote" {
        return (title, usage);
    }
    let prompt = text.to_string();
    let title = tokio::task::spawn_blocking(move || local_title(&prompt))
        .await
        .ok()
        .flatten();
    (title, usage)
}

/// Stores a title, capped the way opencode caps it so a runaway model cannot
/// push the picker and the footer around.
fn store_title(session: &Arc<Session>, title: Option<String>) {
    let Some(title) = title else {
        return;
    };
    let title = if title.chars().count() <= 100 {
        title
    } else {
        let mut short: String = title.chars().take(97).collect();
        short.push('\u{2026}');
        short
    };
    if let Ok(mut state) = session.state.lock() {
        state.title = title.clone();
    }
    session.broadcast(WireEvent::Title { title });
}

/// Title tokens are the session's spend like any other, so they are added to
/// the running total and published, the way opencode records them.
fn record_title_usage(session: &Arc<Session>, usage: Option<Usage>) {
    let Some(seen) = usage else {
        return;
    };
    let Ok(mut state) = session.state.lock() else {
        return;
    };
    let mut total = state
        .usage
        .clone()
        .map(|wire| Usage {
            input_tokens: wire.input_tokens,
            output_tokens: wire.output_tokens,
            cached_tokens: wire.cached_tokens,
        })
        .unwrap_or(Usage {
            input_tokens: 0,
            output_tokens: 0,
            cached_tokens: 0,
        });
    total.input_tokens += seen.input_tokens;
    total.output_tokens += seen.output_tokens;
    total.cached_tokens += seen.cached_tokens;
    let wire = usage_to_wire(total);
    state.usage = Some(wire.clone());
    drop(state);
    session.broadcast(WireEvent::Usage { usage: wire });
}

/// The text a regenerated title is drawn from: the original request plus the
/// recent conversation, bounded the way opencode bounds it.
async fn conversation_for_title(session: &Arc<Session>) -> String {
    const MAX_CONTEXT: usize = 8_000;
    const MAX_FIRST: usize = 2_000;
    let history = session.history.lock().await.clone();
    let part_text = |content: &Vec<ContentPart>| -> String {
        content
            .iter()
            .filter_map(|part| match part {
                ContentPart::InputText { text } | ContentPart::OutputText { text } => {
                    Some(text.trim().to_string())
                }
                ContentPart::InputImage { .. } => None,
            })
            .filter(|text| !text.is_empty())
            .collect::<Vec<_>>()
            .join("\n")
    };
    let first = history
        .iter()
        .find_map(|item| match item {
            InputItem::Message { role, content } if role == "user" => {
                let text = part_text(content);
                (!text.is_empty()).then_some(text)
            }
            _ => None,
        })
        .unwrap_or_default();
    let original = format!(
        "Original request:\n{}",
        first.chars().take(MAX_FIRST).collect::<String>()
    );
    let mut recent = String::new();
    for item in &history {
        if let InputItem::Message { role, content } = item {
            let text = part_text(content);
            if text.is_empty() {
                continue;
            }
            let label = if role == "assistant" { "Assistant" } else { "User" };
            recent.push_str(&format!("{label}: {text}\n\n"));
        }
    }
    if recent.trim().is_empty() {
        return original;
    }
    let prefix = format!("{original}\n\nRecent conversation:\n");
    let budget = MAX_CONTEXT.saturating_sub(prefix.chars().count());
    let tail: String = {
        let chars: Vec<char> = recent.chars().collect();
        let start = chars.len().saturating_sub(budget);
        chars[start..].iter().collect()
    };
    format!("{prefix}{tail}")
}

const TITLE_PROMPT: &str = r#"You are a title generator. You output ONLY a thread title. Nothing else.

<task>
Generate a brief title that would help the user find this conversation later.

Follow all rules in <rules>
Use the <examples> so you know what a good title looks like.
Your output must be:
- A single line
- <=50 characters, counting letters, spaces, and punctuation
- No explanations
</task>

<rules>
- Use the same language as the user message you are summarizing.
- The title must be grammatically correct and read naturally — no word salad.
- Never include tool names (e.g. "read tool", "bash tool", "edit tool").
- Focus on the main topic or question the user needs to retrieve.
- Vary your phrasing — avoid repetitive patterns like always starting with "Analyzing".
- When a file is mentioned, focus on WHAT the user wants to do WITH the file, not just that they shared it.
- Keep exact: technical terms, numbers, filenames, HTTP codes.
- Drop filler articles and pronouns (the, this, my, a, an — and their equivalents in the output language) only when removing them keeps the title grammatical.
- Keep terms of art in their conventional form (e.g. "React hooks", "refresh token"); never translate code identifiers, file names, or product names.
- Never assume a tech stack that was not mentioned.
- NEVER respond to questions — just generate a title for the conversation.
- The title should NEVER include "summarizing" or "generating".
- DO NOT SAY YOU CANNOT GENERATE A TITLE OR COMPLAIN ABOUT THE INPUT.
- Always output something meaningful, even if the input is minimal.
- If the user message is short or conversational (e.g. "hello", "lol", "what's up", "hey"):
  -> create a title that reflects the user's tone or intent (such as Greeting, Quick check-in, Light chat, Intro message, etc.)
</rules>

<examples>
"debug 500 errors in production" -> Debugging production 500 errors
"refactor user service" -> Refactoring user service
"why is app.js failing" -> app.js failure investigation
"implement rate limiting" -> Rate limiting implementation
"how do I connect postgres to my API" -> Postgres API connection
"best practices for React hooks" -> React hooks best practices
"@src/credential.ts can you add refresh token support" -> Credential refresh token support
"@utils/parser.ts this is broken" -> Parser bug fix
"look at @config.json" -> Config review
"@App.tsx add dark mode toggle" -> Dark mode toggle in App
</examples>"#;

// ------------------------------------------------------------------ startup

/// Rebuilds the session map from what is on disk.
fn restore_sessions() -> HashMap<String, Arc<Session>> {
    let mut sessions = HashMap::new();
    for meta in store::list_sessions() {
        let Some(saved) = store::load_session(&meta.id) else {
            continue;
        };
        let session = Session::new(meta.id.clone(), std::env::current_dir().unwrap_or_default(), None);
        {
            let Ok(mut state) = session.state.lock() else {
                continue;
            };
            state.title = saved.title.clone();
            state.mode = "build".to_string();
            state.name = saved.main.name.clone();
            state.transcript = saved
                .main
                .transcript
                .iter()
                .map(|msg| WireMsg {
                    role: msg.role.clone(),
                    text: msg.text.clone(),
                    images: 0,
                    kind: msg.kind,
                })
                .collect();
        }
        if let Ok(mut history) = session.history.try_lock() {
            *history = saved.main.history.clone();
        }
        sessions.insert(meta.id, session);
    }
    sessions
}

/// Binds the listening socket so the caller can register the real port.
pub async fn bind(port: Option<u16>) -> Result<TcpListener> {
    let address = format!("127.0.0.1:{}", port.unwrap_or(0));
    TcpListener::bind(&address)
        .await
        .with_context(|| format!("bind {address}"))
}

pub async fn serve(listener: TcpListener, port: u16) -> Result<()> {
    // Older builds kept conversations beside the configuration; move them into
    // the cache before anything reads, and keep that cache bounded.
    store::migrate();
    store::trim_cache(store::CACHE_LIMIT_BYTES);
    // Every prompt gets its file up front, so there is something to edit
    // before the role is first used.
    for (name, builtin) in [
        ("system", INSTRUCTIONS),
        ("subagent", SUBAGENT_INSTRUCTIONS),
        ("compaction", COMPACTION_PROMPT),
        ("title", TITLE_PROMPT),
        ("plan", PLAN_NOTE),
    ] {
        let _ = store::prompt(name, builtin);
    }
    let agent = Agent {
        codex: Mutex::new(None),
        provider: Mutex::new(
            store::load_settings()
                .provider
                .is_empty()
                .then(|| crate::providers::DEFAULT_PROVIDER.to_string())
                .unwrap_or_else(|| store::load_settings().provider),
        ),
        model: Mutex::new(crate::catalog::load().iter().find(|m| m.listed).map(|m| m.slug.clone()).unwrap_or_default()),
        effort: Mutex::new("medium".to_string()),
        registry: Arc::new(tokio::sync::Mutex::new(tools::Registry::new())),
        cwd: std::env::current_dir().unwrap_or_else(|_| PathBuf::from(".")),
    };

    // Sensible starting values from the saved settings, without ever resuming.
    let settings = store::load_settings();
    if !settings.model.is_empty() {
        if let Ok(mut model) = agent.model.lock() {
            *model = settings.model.clone();
        }
    }
    if let Ok(mut effort) = agent.effort.lock() {
        *effort = if settings.effort.is_empty() {
            "medium".to_string()
        } else {
            settings.effort.clone()
        };
    }

    // Only listed models, with their own efforts, so the UI can offer the right list.
    let catalog = catalog::load();
    if let Some(first) = catalog.iter().find(|m| m.listed)
        && let Ok(mut model) = agent.model.lock()
        && model.is_empty()
    {
        *model = first.slug.clone();
    }

    let plan = agent.codex().ok().and_then(|client| client.plan());
    // MCP servers are reached in the background: a slow or dead one must not
    // hold up the agent, and its tools appear once it answers.
    let mcp = Arc::new(crate::mcp::Registry::default());
    let mcp_loading = mcp.clone();
    tokio::spawn(async move {
        mcp_loading.fill().await;
    });
    let restored = restore_sessions();
    let catalog = catalog::load();
    let shared = Arc::new(Shared {
        password: daemon::password(),
        agent: Arc::new(agent),
        catalog,
        mcp,
        plan: Mutex::new(plan),
        sessions: Mutex::new(restored),
    });

    // The model list is fetched, not baked in: bring it up to date in the
    // background so the first picker of the day is current.
    {
        let shared = shared.clone();
        tokio::spawn(async move {
            let http = shared.agent.registry.lock().await.http();
            crate::models_dev::refresh_if_stale(&http).await;
        });
    }

    let app = Router::new()
        .route("/health", get(health))
        .route("/sessions", get(list_sessions).post(create_session))
        .route("/sessions/{id}", get(session_snapshot))
        .route("/sessions/{id}/prompt", post(prompt))
        .route("/sessions/{id}/answer", post(answer))
        .route("/sessions/{id}/interrupt", post(interrupt))
        .route("/sessions/{id}/events", get(events))
        .route("/sessions/{id}/retitle", post(retitle))
        .route("/sessions/{id}/compact", post(compact_now))
        .route("/sessions/{id}/undo", post(undo))
        .route("/sessions/{id}/rewind", post(rewind))
        .route("/sessions/{id}/fork", post(fork_session))
        .route("/sessions/{id}/mode", post(set_mode))
        .route("/sessions/{id}/clear", post(clear_session))
        .route("/sessions/{id}/diff", post(show_diff))
        .route("/mcp", get(list_mcp))
        .route("/sessions/{id}/cd", post(cd))
        .route("/sessions/{id}/skill", post(run_skill))
        .route("/skills", get(list_skills))
        .route("/models", get(models))
        .route("/config", get(get_config).post(config))
        .route("/providers", get(list_providers))
        .route("/login", post(login))
        .route("/scoped-models", post(scoped_models))
        .with_state(shared.clone());

    eprintln!("exp-1 server listening on http://127.0.0.1:{port}");
    axum::serve(listener, app).await.context("serve")?;
    Ok(())
}

/// What a model list request asks for.
#[derive(serde::Deserialize)]
struct ModelsQuery {
    /// `all=1` ignores the scope: the `/scoped-models` picker has to offer the
    /// models that are not ticked yet.
    #[serde(default)]
    all: Option<String>,
}

async fn models(
    State(shared): State<Arc<Shared>>,
    headers: HeaderMap,
    Query(query): Query<ModelsQuery>,
) -> Response {
    if !authorized(&shared, &headers) {
        return unauthorized();
    }
    let active = shared.agent.provider();
    let everything = query
        .all
        .as_deref()
        .is_some_and(|value| value == "1" || value == "true");
    let scoped = store::load_settings().scoped_models;
    let allowed = |slug: &str| {
        everything || scoped.is_empty() || scoped.iter().any(|chosen| chosen == slug)
    };

    // Every provider but Codex publishes a plain list of model ids.
    if let Some(provider) = crate::providers::by_id(&active)
        && provider.models.iter().any(|(_, wire)| wire.is_remote())
    {
        // The live catalogue first: it is the provider's own list, refreshed in
        // the background instead of baked into this binary. A provider with no
        // entry there keeps the table in providers.rs.
        let live = crate::providers::catalog_id(&active)
            .map(crate::models_dev::models_for)
            .unwrap_or_default();
        let ids: Vec<String> = if live.is_empty() {
            provider.model_ids()
        } else {
            live.into_iter().map(|entry| entry.slug).collect()
        };
        let list: Vec<WireModel> = ids
            .into_iter()
            .filter(|slug| allowed(slug))
            .map(|slug| WireModel {
                context_window: crate::providers::catalog_id(&active)
                    .and_then(|catalog| crate::models_dev::context_limit(catalog, &slug))
                    .unwrap_or_else(provider_context_window),
                display_name: slug.clone(),
                description: format!("via {}", provider.label),
                slug,
                efforts: Vec::new(),
                default_effort: None,
            })
            .collect();
        return Json(list).into_response();
    }

    let models: Vec<WireModel> = catalog::load()
        .into_iter()
        .filter(|model| model.listed)
        .filter(|model| allowed(&model.slug))
        .map(|model| WireModel {
            slug: model.slug,
            context_window: model.context_window,
            display_name: model.display_name,
            description: model.description,
            efforts: model
                .efforts
                .into_iter()
                .map(|(effort, description)| WireEffort {
                    effort,
                    description,
                })
                .collect(),
            default_effort: model.default_effort,
        })
        .collect();
    Json(models).into_response()
}

async fn list_providers(State(shared): State<Arc<Shared>>, headers: HeaderMap) -> Response {
    if !authorized(&shared, &headers) {
        return unauthorized();
    }
    let active = shared.agent.provider();
    let mut list: Vec<WireProvider> = crate::providers::PROVIDERS
        .iter()
        .map(|provider| WireProvider {
            id: provider.id.to_string(),
            label: provider.label.to_string(),
            logged_in: has_credential(provider),
            needs_key: provider.needs_key(),
            device_login: provider.needs_device_login(),
            active: provider.id == active,
        })
        .collect();
    // Strictly alphabetical, the way a list of names is read. The cursor still
    // opens on the active provider, so nothing has to be found by position.
    list.sort_by_key(|entry| entry.label.to_lowercase());
    Json(list).into_response()
}

/// `/login`: pick the active provider, and store a key when one is needed.
/// Stores a key for a provider and for every provider that shares its key:
/// the opencode gateways are one account, so one paste signs into both.
fn store_key(provider: &'static crate::providers::Provider, key: &str) -> anyhow::Result<()> {
    store::set_api_key(provider.id, key)?;
    if let Some(group) = crate::providers::key_group(provider.id) {
        for sibling in crate::providers::PROVIDERS
            .iter()
            .filter(|other| crate::providers::key_group(other.id) == Some(group))
        {
            let _ = store::set_api_key(sibling.id, key);
        }
    }
    Ok(())
}

async fn login(
    State(shared): State<Arc<Shared>>,
    headers: HeaderMap,
    Json(body): Json<LoginBody>,
) -> Response {
    if !authorized(&shared, &headers) {
        return unauthorized();
    }
    let Some(provider) = crate::providers::by_id(&body.provider) else {
        return (StatusCode::NOT_FOUND, "unknown provider").into_response();
    };
    // Picking a provider that is already signed in should switch to it, not
    // send the user through the sign-in page a second time.
    if has_credential(provider) && !body.force {
        // A credential stored before the shared-key groups existed still has to
        // reach its siblings, so the group is completed here too.
        if let Some(key) = store::api_key(provider.id) {
            let _ = store_key(provider, &key);
        }
        if let Ok(mut slot) = shared.agent.provider.lock() {
            *slot = provider.id.to_string();
        }
        if let Some((model, _)) = provider.models.first()
            && let Ok(mut slot) = shared.agent.model.lock()
        {
            *slot = (*model).to_string();
        }
        let mut settings = store::load_settings();
        settings.provider = provider.id.to_string();
        settings.model = shared.agent.model();
        let _ = store::save_settings(&settings);
        let message = format!(
            "Already signed in to {}; switched. Pick it again in /login to sign in anew.",
            provider.label
        );
        if let Some(session) = body.session.as_deref().and_then(|id| find(&shared, id)) {
            session.push("system", message.clone());
        }
        return Json(WireLogin {
            outcome: LoginOutcome::Switched,
            label: provider.label.to_string(),
            model: shared.agent.model(),
            effort: shared.agent.effort(),
            verification_uri: None,
            user_code: None,
            message,
        })
        .into_response();
    }

    // Device-code providers sign in on the vendor's own page; there is nothing
    // to paste, so the flow is started here and watched in the background.
    if provider.needs_device_login() {
        let narrator = body
            .session
            .as_deref()
            .and_then(|id| find(&shared, id));
        let (prompt, flow) = match crate::oauth::begin(provider.id).await {
            Ok(pair) => pair,
            Err(err) => {
                return (StatusCode::BAD_GATEWAY, format!("{err:#}")).into_response();
            }
        };
        // One wording for the session and for the client, so both say the same.
        let message = format!(
            "Sign in to {}: open {}\nand enter the code {}",
            provider.label, prompt.verification_uri, prompt.user_code
        );
        // The page opens by itself, as `codex login` does; the message above
        // stays the fallback for a machine without a browser.
        crate::oauth::open_in_browser(&prompt.verification_uri);
        if let Some(session) = &narrator {
            session.push("system", message.clone());
        }

        let provider_id = provider.id.to_string();
        let label = provider.label.to_string();
        let shared_for_poll = shared.clone();
        tokio::spawn(async move {
            let deadline =
                std::time::Instant::now() + std::time::Duration::from_secs(prompt.expires_in);
            let mut interval = prompt.interval.max(2);
            while std::time::Instant::now() < deadline {
                tokio::time::sleep(std::time::Duration::from_secs(interval)).await;
                match crate::oauth::poll(&provider_id, &flow).await {
                    Ok(crate::oauth::Poll::Pending) => {}
                    Ok(crate::oauth::Poll::SlowDown) => interval += 5,
                    Ok(crate::oauth::Poll::Ready(value)) => {
                        let stored = if provider_id == "openai-auth" {
                            // The Codex reader expects a whole auth file there.
                            store_auth_file(&value)
                        } else {
                            store::set_api_key(&provider_id, &value).map_err(|err| err.to_string())
                        };
                        match stored {
                            Ok(()) => {
                                // New tokens: drop the cached Codex client so
                                // the next request loads them instead of the
                                // ones it was built with.
                                shared_for_poll.agent.forget_codex();
                                if let Ok(mut slot) = shared_for_poll.agent.provider.lock() {
                                    *slot = provider_id.clone();
                                }
                                if let Ok(mut slot) = shared_for_poll.agent.model.lock()
                                    && let Some(provider) = crate::providers::by_id(&provider_id)
                                    && let Some((model, _)) = provider.models.first()
                                {
                                    *slot = (*model).to_string();
                                }
                                let mut settings = store::load_settings();
                                settings.provider = provider_id.clone();
                                settings.model = shared_for_poll.agent.model();
                                let _ = store::save_settings(&settings);
                                if let Some(session) = &narrator {
                                    session.push(
                                        "system",
                                        format!("Signed in to {label}; it is now the active provider"),
                                    );
                                }
                            }
                            Err(message) => {
                                if let Some(session) = &narrator {
                                    session.push(
                                        "system",
                                        format!("Signed in, but the credential could not be saved: {message}"),
                                    );
                                }
                            }
                        }
                        return;
                    }
                    Err(err) => {
                        if let Some(session) = &narrator {
                            session.push("system", format!("{label} login failed: {err:#}"));
                        }
                        return;
                    }
                }
            }
            if let Some(session) = &narrator {
                session.push("system", format!("{label} login timed out; try /login again"));
            }
        });

        return Json(WireLogin {
            outcome: LoginOutcome::Device,
            label: provider.label.to_string(),
            model: shared.agent.model(),
            effort: shared.agent.effort(),
            verification_uri: Some(prompt.verification_uri),
            user_code: Some(prompt.user_code),
            message,
        })
        .into_response();
    }
    if let Some(key) = body.key.as_deref().map(str::trim).filter(|k| !k.is_empty()) {
        if let Err(err) = store_key(provider, key) {
            return (
                StatusCode::INTERNAL_SERVER_ERROR,
                format!("could not store the key: {err:#}"),
            )
                .into_response();
        }
        // A fresh sign-in is a good moment to fetch the current model list.
        let http = shared.agent.registry.lock().await.http();
        tokio::spawn(async move {
            crate::models_dev::refresh_if_stale(&http).await;
        });
    }
    // The Anthropic OAuth provider adopts a token another tool already holds
    // before asking the user to sign in.
    if provider.auth == crate::providers::Auth::OAuthToken
        && store::api_key(provider.id).is_none()
        && let Some(token) = store::adopt_anthropic_token()
    {
        let _ = store::set_api_key(provider.id, &token);
    }
    if provider.needs_key() && store::api_key(provider.id).is_none() {
        return (StatusCode::BAD_REQUEST, "an API key is required").into_response();
    }

    if let Ok(mut slot) = shared.agent.provider.lock() {
        *slot = provider.id.to_string();
    }
    // The first model of the new provider, and an effort it actually takes.
    if let Some((model, _)) = provider.models.first()
        && let Ok(mut slot) = shared.agent.model.lock()
    {
        *slot = (*model).to_string();
    }
    let mut settings = store::load_settings();
    settings.provider = provider.id.to_string();
    settings.model = shared.agent.model();
    settings.effort = shared.agent.effort();
    let _ = store::save_settings(&settings);

    let message = format!(
        "Signed in to {}; model is {} {}",
        provider.label,
        shared.agent.model(),
        shared.agent.effort()
    );
    Json(WireLogin {
        outcome: LoginOutcome::SignedIn,
        label: provider.label.to_string(),
        model: shared.agent.model(),
        effort: shared.agent.effort(),
        verification_uri: None,
        user_code: None,
        message,
    })
    .into_response()
}

async fn scoped_models(
    State(shared): State<Arc<Shared>>,
    headers: HeaderMap,
    Json(body): Json<ScopedBody>,
) -> Response {
    if !authorized(&shared, &headers) {
        return unauthorized();
    }
    let mut settings = store::load_settings();
    settings.scoped_models = body
        .models
        .into_iter()
        .map(|model| model.trim().to_string())
        .filter(|model| !model.is_empty())
        .collect();
    let _ = store::save_settings(&settings);
    StatusCode::NO_CONTENT.into_response()
}

async fn get_config(State(shared): State<Arc<Shared>>, headers: HeaderMap) -> Response {
    if !authorized(&shared, &headers) {
        return unauthorized();
    }
    Json(WireConfig {
        model: shared.agent.model(),
        effort: shared.agent.effort(),
        plan: shared.plan.lock().ok().and_then(|guard| guard.clone()),
    })
    .into_response()
}

async fn config(
    State(shared): State<Arc<Shared>>,
    headers: HeaderMap,
    Json(body): Json<ConfigBody>,
) -> Response {
    if !authorized(&shared, &headers) {
        return unauthorized();
    }
    if let Some(model) = body.model
        && !model.trim().is_empty()
        && let Ok(mut slot) = shared.agent.model.lock()
    {
        *slot = model;
    }
    if let Some(effort) = body.effort
        && !effort.trim().is_empty()
        && let Ok(mut slot) = shared.agent.effort.lock()
    {
        *slot = effort;
    }
    // Load-modify-save, so fields the server does not own survive.
    let mut settings = store::load_settings();
    if let Ok(model) = shared.agent.model.lock() {
        settings.model = model.clone();
    }
    if let Ok(effort) = shared.agent.effort.lock() {
        settings.effort = effort.clone();
    }
    settings.provider = shared.agent.provider();
    let _ = store::save_settings(&settings);
    StatusCode::NO_CONTENT.into_response()
}

/// Keeps `read_lock` used from one place, so the helper is not dead code.
#[allow(dead_code)]
fn model_label(shared: &Shared) -> String {
    read_lock(&shared.agent.model)
}