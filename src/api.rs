//! HTTP transport for the Codex Responses endpoint, with manual SSE parsing.

use anyhow::{Context, Result, bail};
use futures_util::StreamExt;
use std::sync::Arc;
use tokio::sync::mpsc;

use crate::auth::Auth;
use crate::protocol::{Usage, RawEvent, ResponsesRequest};

/// Verified default for ChatGPT-authenticated Codex traffic.
const BASE_URL: &str = "https://chatgpt.com/backend-api/codex";
const ORIGINATOR: &str = "codex_cli_rs";
const VERSION: &str = env!("CARGO_PKG_VERSION");

#[derive(Debug)]
pub enum ApiEvent {
    TextDelta(String),
    Reasoning(String),
    /// A finished output item: `message`, `function_call`, or `reasoning`.
    ItemDone(serde_json::Value),
    Completed(Usage),
    Failed(String),
    /// The model is still writing a tool call's arguments; the text is a line
    /// to show while it does.
    Preparing(String),
}

pub struct CodexClient {
    http: reqwest::Client,
    auth: tokio::sync::Mutex<Auth>,
    user_agent: String,
}

impl CodexClient {
    pub fn new(auth: Auth) -> Result<Self> {
        let os = std::env::consts::OS;
        let arch = std::env::consts::ARCH;
        let user_agent = format!("{ORIGINATOR}/{VERSION} ({os}; {arch}) terminal");

        let http = reqwest::Client::builder()
            .connect_timeout(std::time::Duration::from_secs(20))
            .build()
            .context("build HTTP client")?;

        Ok(Self {
            http,
            auth: tokio::sync::Mutex::new(auth),
            user_agent,
        })
    }

    pub fn plan(&self) -> Option<String> {
        self.auth.try_lock().ok().and_then(|a| a.plan())
    }

    /// POSTs the request and returns a channel of decoded stream events.
    /// `conversation` is stable for the life of one session: the backend caches
    /// prompt prefixes, and a fresh id on every request would ask for a new
    /// cache each time.
    pub async fn stream(
        self: &Arc<Self>,
        request: &ResponsesRequest,
        conversation: &str,
    ) -> Result<mpsc::Receiver<ApiEvent>> {
        let (token, account_id) = {
            let mut guard = self.auth.lock().await;
            let token = guard.access_token().await?;
            let account_id = guard.account_id().map(str::to_string);
            (token, account_id)
        };

        let mut body = serde_json::to_value(request).context("serialize the request")?;
        if let Some(object) = body.as_object_mut() {
            object.insert(
                "prompt_cache_key".to_string(),
                serde_json::Value::String(conversation.to_string()),
            );
        }

        let url = format!("{BASE_URL}/responses");
        let response = self
            .http
            .post(&url)
            .bearer_auth(token)
            .header("chatgpt-account-id", account_id.unwrap_or_default())
            .header("originator", ORIGINATOR)
            .header("User-Agent", &self.user_agent)
            .header("Accept", "text/event-stream")
            .header("session-id", conversation)
            .json(&body)
            .send()
            .await
            .context("POST responses")?;

        let status = response.status();
        if !status.is_success() {
            let body = response.text().await.unwrap_or_default();
            bail!("responses endpoint returned {status}: {body}");
        }

        let (tx, rx) = mpsc::channel(512);
        tokio::spawn(pump(response, tx));
        Ok(rx)
    }
}

/// Reads the response body as text/event-stream and forwards decoded events.
async fn pump(response: reqwest::Response, tx: mpsc::Sender<ApiEvent>) {
    let mut body = response.bytes_stream();
    let mut buffer = String::new();
    let mut preparing = crate::protocol::Preparing::default();

    while let Some(chunk) = body.next().await {
        let chunk = match chunk {
            Ok(chunk) => chunk,
            Err(err) => {
                let _ = tx.send(ApiEvent::Failed(format!("stream aborted: {err}"))).await;
                return;
            }
        };
        buffer.push_str(&String::from_utf8_lossy(&chunk));

        // Consume every complete line currently buffered.
        while let Some(newline) = buffer.find('\n') {
            let line: String = buffer.drain(..=newline).collect();
            let line = line.trim_end_matches(['\r', '\n']);
            let Some(payload) = line.strip_prefix("data:") else {
                continue; // `event:`, `: keep-alive`, id, retry — all ignored
            };
            let payload = payload.trim();
            if payload.is_empty() || payload == "[DONE]" {
                continue;
            }
            dispatch(payload, &tx, &mut preparing).await;
        }
    }
}

async fn dispatch(
    payload: &str,
    tx: &mpsc::Sender<ApiEvent>,
    preparing: &mut crate::protocol::Preparing,
) {
    let Ok(event) = serde_json::from_str::<RawEvent>(payload) else {
        return; // Unknown event shapes are not fatal.
    };

    if let Ok(value) = serde_json::from_str::<serde_json::Value>(payload)
        && let Some(note) = preparing.note(&event.kind, &value)
    {
        let _ = tx.send(ApiEvent::Preparing(note)).await;
    }

    match event.kind.as_str() {
        "response.output_text.delta" => {
            if let Some(delta) = event.delta {
                let _ = tx.send(ApiEvent::TextDelta(delta)).await;
            }
        }
        "response.reasoning_summary_text.delta" | "response.reasoning_text.delta" => {
            if let Some(delta) = event.delta {
                let _ = tx.send(ApiEvent::Reasoning(delta)).await;
            }
        }
        "response.output_item.done" => {
            if let Some(item) = event.item {
                let _ = tx.send(ApiEvent::ItemDone(item)).await;
            }
        }
        "response.completed" => {
            let usage = event
                .response
                .as_ref()
                .map(Usage::from_response)
                .unwrap_or_default();
            let _ = tx.send(ApiEvent::Completed(usage)).await;
        }
        "response.failed" | "response.incomplete" => {
            let detail = event
                .response
                .as_ref()
                .and_then(|r| r.get("error"))
                .map(|e| e.to_string())
                .unwrap_or_else(|| "response did not complete".to_string());
            let _ = tx.send(ApiEvent::Failed(detail)).await;
        }
        "error" => {
            let detail = event.error.map(|e| e.to_string()).unwrap_or_else(|| "error".into());
            let _ = tx.send(ApiEvent::Failed(detail)).await;
        }
        _ => {}
    }
}