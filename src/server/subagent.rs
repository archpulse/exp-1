//! running a subagent and shaping the prompt it gets.

use std::sync::Arc;
use crate::api::ApiEvent;
use crate::proto::{
    WireEvent, WireMsgKind,
};
use crate::protocol::{ContentPart, InputItem, Reasoning, ResponsesRequest};
use crate::tools;
use crate::store;

use super::*;

/// Runs a subagent to completion and returns its report.
pub(crate) async fn run_subagent(
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
                    Some(entry) => match request_target(entry, candidate, &session, &shared.agent.effort()).await {
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
                        if attempt + 1 >= max_attempts || !transient(&last_error) {
                            break;
                        }
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
                        tokio::time::sleep(std::time::Duration::from_secs(1u64 << attempt.min(5))).await;
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
                ApiEvent::Preparing(_) => {}
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

pub(crate) fn subagent_prompt(arguments: &str) -> String {
    let value: serde_json::Value = serde_json::from_str(arguments).unwrap_or_default();
    value
        .get("prompt")
        .and_then(|p| p.as_str())
        .unwrap_or("Do the task described in the arguments.")
        .to_string()
}
