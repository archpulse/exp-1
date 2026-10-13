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

/// The tool call being streamed. A model writing a large file spends a long
/// time inside its arguments — far longer than inside the text — and without a
/// word about it the interface looks like it died.
#[derive(Default)]
pub struct Preparing {
    name: String,
    bytes: usize,
    shown: usize,
}

impl Preparing {
    /// A line worth showing for this Responses event, if any. Shared by both
    /// Responses readers (the Codex client and the plain one), which see the
    /// same events.
    pub fn note(&mut self, kind: &str, event: &Value) -> Option<String> {
        match kind {
            "response.output_item.added" => {
                let item = event.get("item")?;
                if item.get("type").and_then(Value::as_str) != Some("function_call") {
                    return None;
                }
                let name = item
                    .get("name")
                    .and_then(Value::as_str)
                    .unwrap_or("a tool")
                    .to_string();
                Some(self.start(&name))
            }
            "response.function_call_arguments.delta" => {
                let delta = event.get("delta").and_then(Value::as_str)?;
                self.grew(delta)
            }
            _ => None,
        }
    }

    /// Wires that name the tool once and then stream its arguments in pieces:
    /// the chat dialects and Anthropic's partial JSON.
    pub fn fragment(&mut self, name: Option<&str>, args: Option<&str>) -> Option<String> {
        if let Some(name) = name.filter(|name| !name.is_empty()) {
            return Some(self.start(name));
        }
        self.grew(args?)
    }

    /// The tool is about to be written; this line goes up at once, before any
    /// of its arguments exist.
    pub fn start(&mut self, name: &str) -> String {
        self.name = if name.is_empty() {
            "a tool".to_string()
        } else {
            name.to_string()
        };
        self.bytes = 0;
        self.shown = 0;
        format!("preparing {}…", self.name)
    }

    fn grew(&mut self, delta: &str) -> Option<String> {
        self.bytes += delta.len();
        // Once per 4 KB: often enough to look alive, rare enough not to flood
        // the transcript.
        if self.bytes < self.shown + 4096 {
            return None;
        }
        self.shown = self.bytes;
        let name = if self.name.is_empty() {
            "a tool"
        } else {
            self.name.as_str()
        };
        Some(format!("preparing {name}… ({} KB)", self.bytes / 1024))
    }
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
#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn preparing_names_the_tool_and_counts_its_arguments() {
        let mut preparing = Preparing::default();
        // A text item is not a tool call and says nothing about one.
        let message = json!({"item": {"type": "message"}});
        assert!(preparing.note("response.output_item.added", &message).is_none());
        let call = json!({"item": {"type": "function_call", "name": "write"}});
        assert_eq!(
            preparing.note("response.output_item.added", &call).as_deref(),
            Some("preparing write…")
        );
        // Small arguments stay quiet: the first line already said what it was.
        let small = json!({"delta": "x".repeat(100)});
        assert!(
            preparing
                .note("response.function_call_arguments.delta", &small)
                .is_none()
        );
        // Past four kilobytes it says how much has been written.
        let big = json!({"delta": "x".repeat(4096)});
        assert_eq!(
            preparing
                .note("response.function_call_arguments.delta", &big)
                .as_deref(),
            Some("preparing write… (4 KB)")
        );
    }

    #[test]
    fn preparing_covers_the_wires_that_stream_fragments() {
        let mut preparing = Preparing::default();
        // The chat wire names the tool once and then streams its arguments.
        assert_eq!(
            preparing.fragment(Some("edit"), None).as_deref(),
            Some("preparing edit…")
        );
        assert!(preparing.fragment(None, Some("{}")).is_none());
        assert_eq!(
            preparing
                .fragment(None, Some(&"x".repeat(4096)))
                .as_deref(),
            Some("preparing edit… (4 KB)")
        );
        // Anthropic's block header carries the name, and it can be missing.
        assert_eq!(preparing.start(""), "preparing a tool…");
    }
}
