//! Wire types shared by the background server and its clients.

use serde::{Deserialize, Serialize};

/// One transcript entry, as the UI displays it.
/// What kind of line a transcript message is.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum WireMsgKind {
    /// Ordinary text, shown as it arrives.
    #[default]
    Plain,
    /// The output of a tool call: hidden unless the reader asks for it.
    ToolOutput,
    /// A line the next one replaces, for work that reports its steps.
    Progress,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct WireMsg {
    /// `user`, `assistant`, `tool` or `system`.
    pub role: String,
    pub text: String,
    #[serde(default)]
    pub images: usize,
    #[serde(default)]
    pub kind: WireMsgKind,
}

/// How far along a turn is.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum WirePhase {
    Idle,
    Streaming,
    Running,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct WireUsage {
    pub input_tokens: u64,
    pub output_tokens: u64,
    pub cached_tokens: u64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct WireQuestion {
    pub label: String,
    pub text: String,
    pub options: Vec<String>,
}

/// A full picture of a session, sent when a client attaches.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct WireSnapshot {
    pub id: String,
    pub title: String,
    pub name: String,
    pub phase: WirePhase,
    pub transcript: Vec<WireMsg>,
    pub live: String,
    pub usage: Option<WireUsage>,
    /// `build` or `plan`; see the mode handler.
    #[serde(default)]
    pub mode: String,
    pub error: Option<String>,
    pub question: Option<WireAnswerRequest>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct WireAnswerRequest {
    pub questions: Vec<WireQuestion>,
    pub submit_label: String,
}

/// Streamed over SSE while a session is attached.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum WireEvent {
    Snapshot(WireSnapshot),
    Phase { phase: WirePhase },
    Delta { text: String },
    Reasoning { text: String },
    Assistant { text: String },
    Tool { text: String, #[serde(default)] line: WireMsgKind },
    Usage { usage: WireUsage },
    Error { message: String },
    Done { label: String },
    /// The agent is blocked on an answer.
    Ask(WireAnswerRequest),
    AskClosed,
    Title { title: String },
    Idle,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct PromptBody {
    pub text: String,
    /// Pictures pasted into the prompt, if any.
    #[serde(default)]
    pub images: Vec<WireImage>,
    /// Files named with `@` in the prompt, resolved against the session's
    /// directory: pictures are attached, the rest are handed to the model to
    /// read before it answers.
    #[serde(default)]
    pub files: Vec<String>,
}

/// `shift+tab`: which mode the session is in.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ModeBody {
    pub mode: String,
}

/// `/<skill>`: which installed skill to run.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SkillBody {
    pub name: String,
}

/// `/cd`: where the session's tools should work next.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CdBody {
    pub path: String,
}

/// One pasted image, base64 without the data-URL prefix.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct WireImage {
    pub media_type: String,
    pub data: String,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct AnswerBody {
    /// One answer per question, in order.
    pub answers: Vec<String>,
}

/// One entry of the `/login` provider picker.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct WireProvider {
    pub id: String,
    pub label: String,
    pub logged_in: bool,
    /// True when the credential is a key the user pastes, as opposed to a
    /// device-code sign-in or credentials the machine already has.
    pub needs_key: bool,
    /// True when signing in happens on the vendor's device page rather than by
    /// pasting a key, so the picker knows not to ask for one.
    pub device_login: bool,
    pub active: bool,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct LoginBody {
    pub provider: String,
    pub key: Option<String>,
    /// The session that asked, so the server can narrate the login there.
    pub session: Option<String>,
    /// Sign in again even though a credential is already stored.
    #[serde(default)]
    pub force: bool,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct ScopedBody {
    pub models: Vec<String>,
}

/// A model the active provider offers.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct WireModel {
    pub slug: String,
    pub display_name: String,
    pub description: String,
    pub efforts: Vec<WireEffort>,
    pub default_effort: Option<String>,
    #[serde(default)]
    pub context_window: u64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct WireEffort {
    pub effort: String,
    pub description: String,
}

#[derive(Debug, Serialize)]
pub struct SessionSummary {
    pub id: String,
    pub title: String,
    pub name: String,
    pub phase: WirePhase,
}

#[derive(Debug, Serialize)]
pub struct Health {
    pub healthy: bool,
    pub version: String,
}