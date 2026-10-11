//! Wire types for the Codex Responses endpoint.
//!
//! Shapes verified against a live `POST https://chatgpt.com/backend-api/codex/responses`
//! stream and cross-checked with the openai/codex source (`codex-rs/protocol/src/models.rs`,
//! `codex-rs/codex-api/src/common.rs`).

use serde::{Deserialize, Serialize};
use serde_json::Value;

#[derive(Debug, Clone, Serialize)]
pub struct ResponsesRequest {
    pub model: String,
    pub stream: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub instructions: Option<String>,
    pub input: Vec<InputItem>,
    pub tools: Vec<Value>,
    pub tool_choice: &'static str,
    pub parallel_tool_calls: bool,
    pub reasoning: Reasoning,
    pub store: bool,
    pub include: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum InputItem {
    Message {
        role: String,
        content: Vec<ContentPart>,
    },
    FunctionCall {
        call_id: String,
        name: String,
        arguments: String,
    },
    FunctionCallOutput {
        call_id: String,
        output: String,
    },
}

impl InputItem {
    pub fn user_text(text: impl Into<String>) -> Self {
        InputItem::Message {
            role: "user".to_string(),
            content: vec![ContentPart::InputText { text: text.into() }],
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum ContentPart {
    InputText { text: String },
    OutputText { text: String },
    /// A picture the user attached, carried as a data URL so every backend can
    /// pass it on without knowing where it came from.
    InputImage { image_url: String },
}

#[derive(Debug, Clone, Serialize)]
pub struct Reasoning {
    pub effort: String,
    pub summary: &'static str,
}

/// One `data:` payload from the SSE stream.
#[derive(Debug, Deserialize)]
pub struct RawEvent {
    #[serde(rename = "type")]
    pub kind: String,
    #[serde(default)]
    pub delta: Option<String>,
    #[serde(default)]
    pub item: Option<Value>,
    #[serde(default)]
    pub response: Option<Value>,
    #[serde(default)]
    pub error: Option<Value>,
}

#[derive(Debug, Clone, Copy, Default)]
pub struct Usage {
    pub input_tokens: u64,
    pub output_tokens: u64,
    pub cached_tokens: u64,
}

impl Usage {
    pub fn from_response(response: &Value) -> Self {
        let usage = response.get("usage");
        let get =
            |key: &str| usage.and_then(|u| u.get(key)).and_then(Value::as_u64).unwrap_or(0);
        let cached_tokens = usage
            .and_then(|u| u.get("input_tokens_details"))
            .and_then(|d| d.get("cached_tokens"))
            .and_then(Value::as_u64)
            .unwrap_or(0);
        Self {
            input_tokens: get("input_tokens"),
            output_tokens: get("output_tokens"),
            cached_tokens,
        }
    }

    pub fn render(&self) -> String {
        let cached = if self.cached_tokens > 0 {
            format!(" ({} cached)", self.cached_tokens)
        } else {
            String::new()
        };
        format!(
            "in {} / out {}{}",
            self.input_tokens, self.output_tokens, cached
        )
    }
}