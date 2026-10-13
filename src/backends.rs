//! Wire protocols for the non-Codex providers.
//!
//! Two shapes cover everything that can honestly be implemented: the OpenAI
//! chat-completions API (shared by a dozen vendors) and Anthropic's messages
//! API. Both stream Server-Sent Events and both are decoded into the same
//! [`ApiEvent`] stream the Codex client already produces, so the agent loop does
//! not care which one it is talking to.

use anyhow::{Context, Result, bail};
use futures_util::StreamExt;
use serde_json::{Value, json};
use tokio::sync::mpsc;

use crate::api::ApiEvent;
use crate::protocol::{InputItem, Usage};
use crate::providers::{Auth, Provider, Wire};

/// Builds the client for a model and starts a streamed turn. The protocol comes
/// from the model, not the provider: one gateway can serve several.
/// Everything a request needs. `endpoint` is built by the caller because some
/// providers resolve their host at request time — GitHub Copilot derives it from
/// a short-lived token — and `key` may differ from what is stored for the same
/// reason.
pub struct Target {
    pub provider: &'static Provider,
    pub wire: Wire,
    pub endpoint: String,
    pub key: Option<String>,
    pub model: String,
    /// Headers the provider needs on top of its own.
    pub headers: Vec<(String, String)>,
    /// The reasoning effort in use. Every model has levels of its own, and the
    /// request has to carry the one the user picked.
    pub effort: String,
}

pub async fn stream(target: Target, instructions: String, input: Vec<InputItem>, tools: Vec<Value>) -> Result<mpsc::Receiver<ApiEvent>> {
    match target.wire {
        Wire::OpenAiChat => openai_chat(target, instructions, input, tools).await,
        Wire::OpenAiResponses => openai_responses(target, instructions, input, tools).await,
        Wire::Anthropic => anthropic(target, instructions, input, tools).await,
        Wire::Gemini => gemini(target, instructions, input, tools).await,
        Wire::CodexResponses => bail!("the Codex provider is handled by the Codex client"),
    }
}

/// The Responses protocol, as Codex uses it, against any base URL.
async fn openai_responses(
    target: Target,
    instructions: String,
    input: Vec<InputItem>,
    tools: Vec<Value>,
) -> Result<mpsc::Receiver<ApiEvent>> {
    let Target { provider, endpoint, key: api_key, model, headers, effort, .. } = target;
    let model = model.as_str();
    let payload = serde_json::to_value(crate::protocol::ResponsesRequest {
        model: model.to_string(),
        stream: true,
        instructions: Some(instructions),
        input,
        tools,
        tool_choice: "auto",
        parallel_tool_calls: true,
        reasoning: crate::protocol::Reasoning {
            effort,
            summary: "auto",
        },
        store: false,
        include: Vec::new(),
    })?;

    let mut request = reqwest::Client::new()
        .post(&endpoint)
        .header("content-type", "application/json")
        .header("accept", "text/event-stream")
        .json(&payload);
    for (name, value) in &headers {
        request = request.header(name.as_str(), value.as_str());
    }
    let request = apply_auth(request, provider, api_key.as_deref())?;
    let response = request.send().await.context("send the request")?;
    if !response.status().is_success() {
        bail!("{}: {}", provider.label, fail_body(response).await);
    }

    let (tx, rx) = mpsc::channel(512);
    tokio::spawn(async move {
        let mut failure: Option<String> = None;
        let mut preparing = crate::protocol::Preparing::default();
        pump_sse(response, |payload| {
            let Ok(event) = serde_json::from_str::<Value>(payload) else {
                return;
            };
            if let Some(kind) = event.get("type").and_then(Value::as_str)
                && let Some(note) = preparing.note(kind, &event)
            {
                let _ = tx.try_send(ApiEvent::Preparing(note));
            }
            match event.get("type").and_then(Value::as_str) {
                Some("response.output_text.delta") => {
                    if let Some(delta) = event.get("delta").and_then(Value::as_str) {
                        let _ = tx.try_send(ApiEvent::TextDelta(delta.to_string()));
                    }
                }
                Some("response.output_item.done") => {
                    if let Some(item) = event.get("item")
                        && item.get("type").and_then(Value::as_str) == Some("function_call")
                    {
                        let _ = tx.try_send(ApiEvent::ItemDone(item.clone()));
                    }
                }
                Some("response.completed") => {
                    let usage = event
                        .get("response")
                        .and_then(|r| r.get("usage"))
                        .cloned()
                        .unwrap_or(Value::Null);
                    let _ = tx.try_send(ApiEvent::Completed(crate::protocol::Usage {
                        input_tokens: usage
                            .get("input_tokens")
                            .and_then(Value::as_u64)
                            .unwrap_or(0),
                        output_tokens: usage
                            .get("output_tokens")
                            .and_then(Value::as_u64)
                            .unwrap_or(0),
                        cached_tokens: 0,
                    }));
                }
                Some("response.failed") | Some("error") => {
                    failure = Some(event.to_string());
                }
                _ => {}
            }
        })
        .await;
        if let Some(message) = failure {
            let _ = tx.send(ApiEvent::Failed(message)).await;
        }
    });
    Ok(rx)
}

fn apply_auth(
    mut request: reqwest::RequestBuilder,
    provider: &Provider,
    api_key: Option<&str>,
) -> Result<reqwest::RequestBuilder> {
    for (name, value) in provider.headers {
        request = request.header(*name, *value);
    }
    let Some(key) = api_key else {
        bail!("{} has no API key stored — run /login", provider.label);
    };
    request = match provider.auth {
        Auth::Bearer => request.bearer_auth(key),
        Auth::ApiKeyHeader => request.header("x-api-key", key),
        // An OAuth token goes in as a bearer token, but the backend also wants
        // the first-party client's identity, so it is applied in `anthropic`.
        Auth::OAuthToken => request,
        // Device sessions are stored as a bearer credential once signed in.
        Auth::DeviceLogin => request.bearer_auth(key),
        // Ambient credentials are fetched per request and applied by the caller.
        Auth::Ambient => request,
    };
    Ok(request)
}

/// Reads an SSE body and hands every `data:` payload to `emit`.
async fn pump_sse<F>(response: reqwest::Response, mut emit: F)
where
    F: FnMut(&str),
{
    let mut body = response.bytes_stream();
    let mut buffer = String::new();
    while let Some(chunk) = body.next().await {
        let Ok(chunk) = chunk else { return };
        buffer.push_str(&String::from_utf8_lossy(&chunk));
        while let Some(newline) = buffer.find('\n') {
            let line: String = buffer.drain(..=newline).collect();
            let line = line.trim();
            let Some(payload) = line.strip_prefix("data:") else {
                continue;
            };
            let payload = payload.trim();
            if payload.is_empty() || payload == "[DONE]" {
                continue;
            }
            emit(payload);
        }
    }
}

async fn fail_body(response: reqwest::Response) -> String {
    let status = response.status();
    let body = response.text().await.unwrap_or_default();
    format!("HTTP {status}: {}", body.chars().take(300).collect::<String>())
}

// ------------------------------------------------------- OpenAI chat completions

/// Tool specs arrive in Responses form; chat completions nests them one deeper.
fn chat_tools(tools: &[Value]) -> Vec<Value> {
    tools
        .iter()
        .map(|tool| {
            json!({
                "type": "function",
                "function": {
                    "name": tool.get("name").cloned().unwrap_or(Value::Null),
                    "description": tool.get("description").cloned().unwrap_or(Value::Null),
                    "parameters": tool.get("parameters").cloned().unwrap_or(json!({})),
                }
            })
        })
        .collect()
}

/// The Responses conversation, rewritten as chat messages.
fn chat_messages(instructions: &str, input: &[InputItem]) -> Vec<Value> {
    let mut messages = vec![json!({ "role": "system", "content": instructions })];
    for item in input {
        match item {
            InputItem::Message { role, content } => {
                let text: String = content
                    .iter()
                    .filter_map(|part| match part {
                        crate::protocol::ContentPart::InputText { text }
                        | crate::protocol::ContentPart::OutputText { text } => Some(text.clone()),
                        crate::protocol::ContentPart::InputImage { .. } => None,
                    })
                    .collect::<Vec<_>>()
                    .join("\n");
                let images: Vec<&str> = content
                    .iter()
                    .filter_map(|part| match part {
                        crate::protocol::ContentPart::InputImage { image_url } => {
                            Some(image_url.as_str())
                        }
                        _ => None,
                    })
                    .collect();
                if images.is_empty() {
                    messages.push(json!({ "role": role, "content": text }));
                } else {
                    // A picture forces the content-parts form.
                    let mut parts = vec![json!({ "type": "text", "text": text })];
                    for url in images {
                        parts.push(json!({
                            "type": "image_url",
                            "image_url": { "url": url }
                        }));
                    }
                    messages.push(json!({ "role": role, "content": parts }));
                }
            }
            InputItem::FunctionCall {
                call_id,
                name,
                arguments,
            } => messages.push(json!({
                "role": "assistant",
                "content": Value::Null,
                "tool_calls": [{
                    "id": call_id,
                    "type": "function",
                    "function": { "name": name, "arguments": arguments }
                }]
            })),
            InputItem::FunctionCallOutput { call_id, output } => messages.push(json!({
                "role": "tool",
                "tool_call_id": call_id,
                "content": output
            })),
        }
    }
    messages
}

async fn openai_chat(
    target: Target,
    instructions: String,
    input: Vec<InputItem>,
    tools: Vec<Value>,
) -> Result<mpsc::Receiver<ApiEvent>> {
    let Target { provider, endpoint, key: api_key, model, headers, effort, .. } = target;
    let model = model.as_str();
    let mut payload = json!({
        "model": model,
        "stream": true,
        "stream_options": { "include_usage": true },
        "messages": chat_messages(&instructions, &input),
        "tools": chat_tools(&tools),
        "tool_choice": "auto",
    });
    // The opencode gateways take the level as the OpenAI field; other providers
    // speak their own dialect, or none, so nothing is sent on their behalf.
    if crate::providers::key_group(provider.id) == Some("opencode") && !effort.is_empty() {
        payload["reasoning_effort"] = json!(effort);
    }

    let mut request = reqwest::Client::new()
        .post(&endpoint)
        .header("content-type", "application/json")
        .header("accept", "text/event-stream")
        .json(&payload);
    for (name, value) in &headers {
        request = request.header(name.as_str(), value.as_str());
    }
    let request = apply_auth(request, provider, api_key.as_deref())?;
    let response = request.send().await.context("send the request")?;
    if !response.status().is_success() {
        bail!("{}: {}", provider.label, fail_body(response).await);
    }

    let (tx, rx) = mpsc::channel(512);
    tokio::spawn(async move {
        // Tool calls arrive in fragments and are stitched back together here.
        let mut calls: Vec<(String, String, String)> = Vec::new();
        let mut failure: Option<String> = None;

        pump_sse(response, |payload| {
            let Ok(event) = serde_json::from_str::<Value>(payload) else {
                return;
            };
            if let Some(error) = event.get("error") {
                failure = Some(error.to_string());
                return;
            }
            if let Some(usage) = event.get("usage").filter(|u| !u.is_null()) {
                let _ = tx.try_send(ApiEvent::Completed(Usage {
                    input_tokens: usage.get("prompt_tokens").and_then(Value::as_u64).unwrap_or(0),
                    output_tokens: usage
                        .get("completion_tokens")
                        .and_then(Value::as_u64)
                        .unwrap_or(0),
                    cached_tokens: usage
                        .get("prompt_tokens_details")
                        .and_then(|d| d.get("cached_tokens"))
                        .and_then(Value::as_u64)
                        .unwrap_or(0),
                }));
            }
            let Some(choice) = event
                .get("choices")
                .and_then(|c| c.as_array())
                .and_then(|c| c.first())
            else {
                return;
            };
            let Some(delta) = choice.get("delta") else {
                return;
            };

            if let Some(text) = delta.get("content").and_then(Value::as_str)
                && !text.is_empty()
            {
                let _ = tx.try_send(ApiEvent::TextDelta(text.to_string()));
            }
            let Some(fragments) = delta.get("tool_calls").and_then(|c| c.as_array()) else {
                return;
            };
            for fragment in fragments {
                let index = fragment
                    .get("index")
                    .and_then(Value::as_u64)
                    .unwrap_or(0) as usize;
                while calls.len() <= index {
                    calls.push((String::new(), String::new(), String::new()));
                }
                let slot = &mut calls[index];
                if let Some(id) = fragment.get("id").and_then(Value::as_str) {
                    slot.0.push_str(id);
                }
                let function = fragment.get("function");
                if let Some(name) = function.and_then(|f| f.get("name")).and_then(Value::as_str) {
                    slot.1.push_str(name);
                }
                if let Some(args) = function
                    .and_then(|f| f.get("arguments"))
                    .and_then(Value::as_str)
                {
                    slot.2.push_str(args);
                }
            }
        })
        .await;

        for (index, (id, name, arguments)) in calls.into_iter().enumerate() {
            if name.is_empty() {
                continue;
            }
            let id = if id.is_empty() {
                format!("call_{index}")
            } else {
                id
            };
            let _ = tx
                .send(ApiEvent::ItemDone(json!({
                    "type": "function_call",
                    "call_id": id,
                    "name": name,
                    "arguments": if arguments.is_empty() { "{}".to_string() } else { arguments },
                })))
                .await;
        }
        if let Some(message) = failure {
            let _ = tx.send(ApiEvent::Failed(message)).await;
        }
    });
    Ok(rx)
}

/// Splits `data:image/png;base64,AAAA` into its media type and payload.
fn split_data_url(url: &str) -> (String, String) {
    let rest = url.strip_prefix("data:").unwrap_or(url);
    match rest.split_once(";base64,") {
        Some((media, data)) => (media.to_string(), data.to_string()),
        None => ("image/png".to_string(), rest.to_string()),
    }
}

// ------------------------------------------------------------------ Gemini

/// Gemini's `contents` array, shared by Vertex and the AI Studio API.
fn gemini_contents(input: &[InputItem]) -> Vec<Value> {
    let mut names: std::collections::HashMap<String, String> = std::collections::HashMap::new();
    let mut contents: Vec<Value> = Vec::new();
    for item in input {
        match item {
            InputItem::Message { role, content } => {
                let text: String = content
                    .iter()
                    .filter_map(|part| match part {
                        crate::protocol::ContentPart::InputText { text }
                        | crate::protocol::ContentPart::OutputText { text } => Some(text.clone()),
                        crate::protocol::ContentPart::InputImage { .. } => None,
                    })
                    .collect::<Vec<_>>()
                    .join("
");
                let mut parts: Vec<Value> = Vec::new();
                if !text.is_empty() {
                    parts.push(json!({ "text": text }));
                }
                for part in content {
                    if let crate::protocol::ContentPart::InputImage { image_url } = part {
                        let (media, data) = split_data_url(image_url);
                        parts.push(json!({
                            "inlineData": { "mimeType": media, "data": data }
                        }));
                    }
                }
                // Gemini calls the assistant "model".
                let role = if role == "assistant" { "model" } else { "user" };
                contents.push(json!({ "role": role, "parts": parts }));
            }
            InputItem::FunctionCall {
                call_id,
                name,
                arguments,
            } => {
                names.insert(call_id.clone(), name.clone());
                let args: Value = serde_json::from_str(arguments).unwrap_or(json!({}));
                contents.push(json!({
                    "role": "model",
                    "parts": [{ "functionCall": { "name": name, "args": args } }]
                }));
            }
            InputItem::FunctionCallOutput { call_id, output } => {
                // Gemini matches a response to its call by name, not id.
                let name = names.get(call_id).cloned().unwrap_or_else(|| "tool".to_string());
                contents.push(json!({
                    "role": "user",
                    "parts": [{
                        "functionResponse": {
                            "name": name,
                            "response": { "output": output }
                        }
                    }]
                }));
            }
        }
    }
    contents
}

async fn gemini(
    target: Target,
    instructions: String,
    input: Vec<InputItem>,
    tools: Vec<Value>,
) -> Result<mpsc::Receiver<ApiEvent>> {
    let Target { provider, endpoint, key: api_key, model: _, headers, .. } = target;

    let declarations: Vec<Value> = tools
        .iter()
        .map(|tool| {
            json!({
                "name": tool.get("name").cloned().unwrap_or(Value::Null),
                "description": tool.get("description").cloned().unwrap_or(Value::Null),
                "parameters": tool.get("parameters").cloned().unwrap_or(json!({})),
            })
        })
        .collect();

    let payload = json!({
        "contents": gemini_contents(&input),
        "systemInstruction": { "parts": [{ "text": instructions }] },
        "tools": [{ "functionDeclarations": declarations }],
    });

    let mut request = reqwest::Client::new()
        .post(&endpoint)
        .header("content-type", "application/json")
        .header("accept", "text/event-stream")
        .json(&payload);
    for (name, value) in &headers {
        request = request.header(name.as_str(), value.as_str());
    }
    let Some(key) = api_key else {
        bail!("{} has no credential stored — run /login", provider.label);
    };
    let request = request.bearer_auth(key);

    let response = request.send().await.context("send the request")?;
    if !response.status().is_success() {
        bail!("{}: {}", provider.label, fail_body(response).await);
    }

    let (tx, rx) = mpsc::channel(512);
    tokio::spawn(async move {
        // Text arrives in fragments and function calls whole, so both are
        // accumulated and flushed at the end of each chunk.
        pump_sse(response, |payload| {
            let Ok(event) = serde_json::from_str::<Value>(payload) else {
                return;
            };
            if let Some(usage) = event.get("usageMetadata") {
                let _ = tx.try_send(ApiEvent::Completed(crate::protocol::Usage {
                    input_tokens: usage
                        .get("promptTokenCount")
                        .and_then(Value::as_u64)
                        .unwrap_or(0),
                    output_tokens: usage
                        .get("candidatesTokenCount")
                        .and_then(Value::as_u64)
                        .unwrap_or(0),
                    cached_tokens: usage
                        .get("cachedContentTokenCount")
                        .and_then(Value::as_u64)
                        .unwrap_or(0),
                }));
            }
            let Some(parts) = event
                .get("candidates")
                .and_then(|c| c.as_array())
                .and_then(|c| c.first())
                .and_then(|c| c.get("content"))
                .and_then(|c| c.get("parts"))
                .and_then(|p| p.as_array())
            else {
                return;
            };
            for part in parts {
                if let Some(text) = part.get("text").and_then(Value::as_str)
                    && !text.is_empty()
                {
                    let _ = tx.try_send(ApiEvent::TextDelta(text.to_string()));
                }
                if let Some(call) = part.get("functionCall") {
                    let name = call
                        .get("name")
                        .and_then(Value::as_str)
                        .unwrap_or_default()
                        .to_string();
                    if name.is_empty() {
                        continue;
                    }
                    let args = call.get("args").cloned().unwrap_or(json!({}));
                    let _ = tx.try_send(ApiEvent::ItemDone(json!({
                        "type": "function_call",
                        "call_id": format!("call_{name}"),
                        "name": name,
                        "arguments": args.to_string(),
                    })));
                }
            }
        })
        .await;
    });
    Ok(rx)
}

// ------------------------------------------------------------- Anthropic

async fn anthropic(
    target: Target,
    instructions: String,
    input: Vec<InputItem>,
    tools: Vec<Value>,
) -> Result<mpsc::Receiver<ApiEvent>> {
    let Target { provider, endpoint, key: api_key, model, headers, .. } = target;
    let model = model.as_str();
    // The OAuth backend only accepts the first-party tool names.
    let oauth = api_key.as_deref().is_some_and(crate::providers::is_oauth_token);
    let anthropic_tools: Vec<Value> = tools
        .iter()
        .map(|tool| {
            let name = tool.get("name").and_then(Value::as_str).unwrap_or_default();
            json!({
                "name": if oauth {
                    crate::providers::claude_code_name(name)
                } else {
                    name.to_string()
                },
                "description": tool.get("description").cloned().unwrap_or(Value::Null),
                "input_schema": tool.get("parameters").cloned().unwrap_or(json!({})),
            })
        })
        .collect();

    let payload = json!({
        "model": model,
        "max_tokens": 8192,
        "system": instructions,
        "stream": true,
        "messages": anthropic_messages(&input),
        "tools": anthropic_tools,
    });

    let mut request = reqwest::Client::new()
        .post(&endpoint)
        .header("content-type", "application/json")
        .header("accept", "text/event-stream")
        .json(&payload);
    for (name, value) in &headers {
        request = request.header(name.as_str(), value.as_str());
    }
    let request = apply_auth(request, provider, api_key.as_deref())?;
    // An OAuth token is a bearer token carrying the first-party client's
    // identity; the backend rejects the raw key of a login without these.
    let oauth = api_key.as_deref().filter(|key| crate::providers::is_oauth_token(key));
    let request = match oauth {
        Some(token) => request
            .bearer_auth(token)
            .header("anthropic-dangerous-direct-browser-access", "true")
            .header("user-agent", "claude-cli/2.1.280")
            .header("x-app", "cli")
            .header("anthropic-beta", "claude-code-20250219,oauth-2025-04-20"),
        None => request,
    };
    let response = request.send().await.context("send the request")?;
    if !response.status().is_success() {
        bail!("{}: {}", provider.label, fail_body(response).await);
    }

    let (tx, rx) = mpsc::channel(512);
    tokio::spawn(async move {
        let mut text = String::new();
        let mut tool: Option<(String, String)> = None; // (id, name)
        let mut arguments = String::new();
        let mut failure: Option<String> = None;

        pump_sse(response, |payload| {
            let Ok(event) = serde_json::from_str::<Value>(payload) else {
                return;
            };
            match event.get("type").and_then(Value::as_str) {
                Some("content_block_delta") => {
                    let Some(delta) = event.get("delta") else { return };
                    match delta.get("type").and_then(Value::as_str) {
                        Some("text_delta") => {
                            if let Some(chunk) = delta.get("text").and_then(Value::as_str) {
                                text.push_str(chunk);
                                let _ = tx.try_send(ApiEvent::TextDelta(chunk.to_string()));
                            }
                        }
                        Some("input_json_delta") => {
                            if let Some(chunk) = delta.get("partial_json").and_then(Value::as_str) {
                                arguments.push_str(chunk);
                            }
                        }
                        _ => {}
                    }
                }
                Some("content_block_start") => {
                    if let Some(block) = event.get("content_block")
                        && block.get("type").and_then(Value::as_str) == Some("tool_use")
                    {
                        let id = block
                            .get("id")
                            .and_then(Value::as_str)
                            .unwrap_or("toolu_0")
                            .to_string();
                        let name = block
                            .get("name")
                            .and_then(Value::as_str)
                            .unwrap_or("")
                            .to_string();
                        tool = Some((id, name));
                        arguments.clear();
                    }
                }
                Some("message_delta") => {
                    if let Some(usage) = event.get("usage") {
                        let _ = tx.try_send(ApiEvent::Completed(Usage {
                            input_tokens: event
                                .get("message")
                                .and_then(|m| m.get("usage"))
                                .and_then(|u| u.get("input_tokens"))
                                .and_then(Value::as_u64)
                                .unwrap_or(0),
                            output_tokens: usage
                                .get("output_tokens")
                                .and_then(Value::as_u64)
                                .unwrap_or(0),
                            cached_tokens: 0,
                        }));
                    }
                }
                Some("message_start") => {
                    if let Some(usage) = event
                        .get("message")
                        .and_then(|m| m.get("usage"))
                        .and_then(|u| u.get("input_tokens"))
                        .and_then(Value::as_u64)
                    {
                        let _ = tx.try_send(ApiEvent::Completed(Usage {
                            input_tokens: usage,
                            output_tokens: 0,
                            cached_tokens: 0,
                        }));
                    }
                }
                Some("error") => {
                    failure = Some(event.to_string());
                }
                _ => {}
            }
        })
        .await;

        if let Some((id, name)) = tool
            && !name.is_empty()
        {
            let _ = tx
                .send(ApiEvent::ItemDone(json!({
                    "type": "function_call",
                    "call_id": id,
                    "name": name,
                    "arguments": if arguments.is_empty() { "{}".to_string() } else { arguments },
                })))
                .await;
        }
        if let Some(message) = failure {
            let _ = tx.send(ApiEvent::Failed(message)).await;
        }
    });
    Ok(rx)
}

fn anthropic_messages(input: &[InputItem]) -> Vec<Value> {
    let mut messages: Vec<Value> = Vec::new();
    for item in input {
        match item {
            InputItem::Message { role, content } => {
                let text: String = content
                    .iter()
                    .filter_map(|part| match part {
                        crate::protocol::ContentPart::InputText { text }
                        | crate::protocol::ContentPart::OutputText { text } => Some(text.clone()),
                        crate::protocol::ContentPart::InputImage { .. } => None,
                    })
                    .collect::<Vec<_>>()
                    .join("\n");
                let mut blocks: Vec<Value> = Vec::new();
                if !text.is_empty() {
                    blocks.push(json!({ "type": "text", "text": text }));
                }
                for part in content {
                    if let crate::protocol::ContentPart::InputImage { image_url } = part {
                        let (media, data) = split_data_url(image_url);
                        blocks.push(json!({
                            "type": "image",
                            "source": { "type": "base64", "media_type": media, "data": data }
                        }));
                    }
                }
                if blocks.is_empty() {
                    blocks.push(json!({ "type": "text", "text": String::new() }));
                }
                messages.push(json!({
                    "role": if role == "assistant" { "assistant" } else { "user" },
                    "content": blocks
                }));
            }
            InputItem::FunctionCall {
                call_id,
                name,
                arguments,
            } => {
                let parsed: Value = serde_json::from_str(arguments).unwrap_or(json!({}));
                messages.push(json!({
                    "role": "assistant",
                    "content": [{
                        "type": "tool_use",
                        "id": call_id,
                        "name": name,
                        "input": parsed
                    }]
                }));
            }
            InputItem::FunctionCallOutput { call_id, output } => messages.push(json!({
                "role": "user",
                "content": [{
                    "type": "tool_result",
                    "tool_use_id": call_id,
                    "content": output
                }]
            })),
        }
    }
    messages
}
#[cfg(test)]
mod tests {
    use super::*;
    use crate::protocol::ContentPart;

    fn picture() -> ContentPart {
        ContentPart::InputImage {
            image_url: "data:image/png;base64,AAAB".to_string(),
        }
    }

    #[test]
    fn splits_a_data_url() {
        assert_eq!(
            split_data_url("data:image/jpeg;base64,AAAA"),
            ("image/jpeg".to_string(), "AAAA".to_string())
        );
        // A bare payload is assumed to be a PNG rather than dropped.
        assert_eq!(
            split_data_url("AAAA"),
            ("image/png".to_string(), "AAAA".to_string())
        );
    }

    #[test]
    fn openai_chat_sends_a_picture_as_content_parts() {
        let input = vec![InputItem::Message {
            role: "user".to_string(),
            content: vec![ContentPart::InputText { text: "what is this?".into() }, picture()],
        }];
        let messages = chat_messages("system", &input);
        let content = &messages[1]["content"];
        assert!(content.is_array(), "{content}");
        assert_eq!(content[0]["type"], "text");
        assert_eq!(content[1]["type"], "image_url");
        assert_eq!(content[1]["image_url"]["url"], "data:image/png;base64,AAAB");
        // Without a picture the plain string form is kept.
        let plain = chat_messages("system", &[InputItem::user_text("hi")]);
        assert_eq!(plain[1]["content"], "hi");
    }

    #[test]
    fn anthropic_sends_a_picture_as_an_image_block() {
        let input = vec![InputItem::Message {
            role: "user".to_string(),
            content: vec![ContentPart::InputText { text: "look".into() }, picture()],
        }];
        let messages = anthropic_messages(&input);
        let blocks = &messages[0]["content"];
        assert_eq!(blocks[0]["type"], "text");
        assert_eq!(blocks[1]["type"], "image");
        assert_eq!(blocks[1]["source"]["media_type"], "image/png");
        assert_eq!(blocks[1]["source"]["data"], "AAAB");
    }

    #[test]
    fn gemini_sends_a_picture_as_inline_data() {
        let input = vec![InputItem::Message {
            role: "user".to_string(),
            content: vec![ContentPart::InputText { text: "look".into() }, picture()],
        }];
        let contents = gemini_contents(&input);
        assert_eq!(contents[0]["parts"][0]["text"], "look");
        assert_eq!(contents[0]["parts"][1]["inlineData"]["mimeType"], "image/png");
        assert_eq!(contents[0]["parts"][1]["inlineData"]["data"], "AAAB");
    }
}
