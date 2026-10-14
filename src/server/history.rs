//! what the model is shown: clipping, pruning, compaction.

use std::sync::Arc;
use crate::api::ApiEvent;
use crate::catalog;
use crate::proto::{
    WireEvent, WireMsgKind,
};
use crate::protocol::{ContentPart, InputItem, Reasoning, ResponsesRequest};
use crate::store;

use super::*;

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

pub(crate) fn clip(text: &str, limit: usize) -> String {
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
pub(crate) fn item_chars(item: &InputItem) -> usize {
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
pub(crate) const PRUNE_AT_CHARS: usize = 200_000;
const PRUNE_TO_CHARS: usize = 120_000;
const PRUNE_KEEP_TURNS: usize = 2;
/// Results the working set keeps whole, whatever the turns say: a single long
/// turn must not overflow the window before it ends.
const PRUNE_KEEP_OUTPUTS: usize = 6;

/// Replaces old tool output with a note; returns how many items were dropped.
pub(crate) fn prune_history(history: &mut [InputItem]) -> usize {
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
pub(crate) async fn compact(shared: &Arc<Shared>, session: &Arc<Session>) -> Option<String> {
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
/// A conservative window for providers whose catalogue we do not have.
pub(crate) fn provider_context_window() -> u64 {
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
pub(crate) async fn compact_if_full(shared: &Arc<Shared>, session: &Arc<Session>) -> bool {
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
