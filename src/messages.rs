//! server events, and how they change the screen.

use std::time::{Duration, Instant};
use tokio::sync::mpsc;

use super::*;

#[derive(Clone)]
pub(crate) enum AgentMsg {
    Phase(usize, Phase),
    /// What the model is preparing right now, for the activity line.
    Preparing(usize, String),
    Delta(usize, String),
    Reasoning(usize, String),
    AssistantDone(usize, String),
    /// A tool line exactly as the server produced it, and what kind it is.
    ToolLine(usize, String, WireMsgKind),
    Usage(usize, Usage),
    Error(usize, String),
    Done(usize, String),
    Title(usize, String),
    /// The UI is now driving a different server session.
    Attached(usize, String),
    /// The provider registry arrived, open `/login`.
    Providers(Vec<WireProvider>),
    /// The model list arrived, open `/scoped-models`.
    Models(Vec<WireModel>),
    /// The working directory moved: the new path.
    Cwd(String),
    /// The installed skills arrived: `/name` and its description.
    Skills(Vec<(String, String)>),
    /// The model catalogue arrived at startup, for the context meter.
    Catalog(Vec<proto::WireModel>),
    /// A saved conversation arrived, to fill a thread that just attached.
    Loaded(usize, proto::WireSnapshot),
    /// A user message was clicked: the conversation goes back to it.
    Rewound(String),
    /// A piece of dictated text arrived.
    Dictation(String),
    /// Words the recognizer has heard but not committed yet: shown, not inserted.
    Dictating(String),
    /// The recognizer ended: the button goes back to its idle look.
    DictationStopped,
    /// The recognizer complained on its way out.
    DictationFailed(String),
    /// A login completed: label, model, effort, and the line to show.
    ProviderChanged(String, String, String, String),
    Ask(usize, proto::WireAnswerRequest),
    AskClosed(usize),
    Idle(usize),
}

/// Translates one server event into updates for a UI thread.
fn translate(thread: usize, event: WireEvent) -> Vec<AgentMsg> {
    match event {
        WireEvent::Snapshot(_) => Vec::new(),
        WireEvent::Phase { phase } => vec![AgentMsg::Phase(
            thread,
            match phase {
                WirePhase::Idle => Phase::Idle,
                WirePhase::Streaming => Phase::Streaming,
                WirePhase::Running => Phase::Running,
            },
        )],
        WireEvent::Delta { text } => vec![AgentMsg::Delta(thread, text)],
        WireEvent::Preparing { text } => vec![AgentMsg::Preparing(thread, text)],
        WireEvent::Reasoning { text } => vec![AgentMsg::Reasoning(thread, text)],
        WireEvent::Assistant { text } => vec![AgentMsg::AssistantDone(thread, text)],
        WireEvent::Tool { text, line } => vec![AgentMsg::ToolLine(thread, text, line)],
        WireEvent::Usage { usage } => vec![AgentMsg::Usage(
            thread,
            Usage {
                input_tokens: usage.input_tokens,
                output_tokens: usage.output_tokens,
                cached_tokens: usage.cached_tokens,
            },
        )],
        WireEvent::Error { message } => vec![AgentMsg::Error(thread, message)],
        WireEvent::Done { label } => vec![AgentMsg::Done(thread, label)],
        WireEvent::Title { title } => vec![AgentMsg::Title(thread, title)],
        WireEvent::Ask(request) => vec![AgentMsg::Ask(thread, request)],
        WireEvent::AskClosed => vec![AgentMsg::AskClosed(thread)],
        WireEvent::Idle => vec![AgentMsg::Idle(thread)],
    }
}

/// Drains a session stream into the UI channel for the lifetime of the app.
pub(crate) fn pump_events(
    thread: usize,
    mut events: mpsc::UnboundedReceiver<WireEvent>,
    tx: mpsc::UnboundedSender<AgentMsg>,
) {
    tokio::spawn(async move {
        while let Some(event) = events.recv().await {
            for message in translate(thread, event) {
                if tx.send(message).is_err() {
                    return;
                }
            }
        }
    });
}
/// `/mcp` output: one line per server, with what went wrong when it did.
pub(crate) fn render_mcp(servers: &[serde_json::Value]) -> String {
    if servers.is_empty() {
        return format!(
            "no MCP servers configured — add them to {}",
            store::mcp_path().display()
        );
    }
    servers
        .iter()
        .map(|server| {
            let name = server.get("name").and_then(|v| v.as_str()).unwrap_or("?");
            let kind = server.get("kind").and_then(|v| v.as_str()).unwrap_or("?");
            match server.get("error").and_then(|v| v.as_str()) {
                Some(error) => format!("{name} ({kind}): unavailable — {error}"),
                None => {
                    let tools = server.get("tools").and_then(|v| v.as_u64()).unwrap_or(0);
                    format!("{name} ({kind}): {tools} tool(s)")
                }
            }
        })
        .collect::<Vec<_>>()
        .join("\n")
}

pub(crate) fn thread_name(prompt: &str) -> String {
    let first = prompt.lines().next().unwrap_or(prompt).trim();
    let mut name: String = first.chars().take(32).collect();
    if first.chars().count() > 32 {
        name.push('…');
    }
    name
}

pub(crate) fn apply(app: &mut App, message: AgentMsg) {
    let (index, update): (usize, Box<dyn FnOnce(&mut App)>) = match message {
        AgentMsg::Phase(index, phase) => (index, Box::new(move |app: &mut App| {
            // A turn that ran long enough for the user to walk away gets a bell
            // and, where the desktop has one, a notification.
            if phase == Phase::Idle
                && app.threads[index]
                    .started_at
                    .is_some_and(|start| start.elapsed() >= Duration::from_secs(15))
            {
                app.attention = true;
                app.threads[index].started_at = None;
            }
            let thread = &mut app.threads[index];
            if phase == Phase::Streaming {
                thread.live.clear();
            }
            // The preparing note belongs to one request; a new phase means a new
            // thing is happening.
            thread.preparing = None;
            // Each request and each tool gets its own clock: the line under the
            // prompt is about what is happening now.
            thread.activity_since = match phase {
                Phase::Idle => None,
                _ => Some(Instant::now()),
            };
            thread.phase = phase;
        })),
        AgentMsg::Preparing(index, text) => (index, Box::new(move |app: &mut App| {
            app.threads[index].preparing = Some(text);
        })),
        AgentMsg::Delta(index, delta) => (index, Box::new(move |app: &mut App| {
            let thread = &mut app.threads[index];
            // Visible text means the model is writing words, not a tool call.
            thread.preparing = None;
            thread.live.push_str(&delta);
        })),
        AgentMsg::Reasoning(index, text) => (index, Box::new(move |app: &mut App| {
            app.threads[index].reasoning.push_str(&text);
        })),
        AgentMsg::AssistantDone(index, text) => (index, Box::new(move |app: &mut App| {
            let thread = &mut app.threads[index];
            thread.push(Role::Assistant, text);
            thread.live.clear();
            thread.reasoning.clear();
        })),
        AgentMsg::ToolLine(index, text, kind) => (index, Box::new(move |app: &mut App| {
            let thread = &mut app.threads[index];
            thread.live.clear();
            match kind {
                // A subagent's step replaces the previous one.
                WireMsgKind::Progress => thread.push_progress(text),
                _ => thread.push_kind(Role::Tool, text, kind),
            }
        })),
        AgentMsg::Usage(index, usage) => (index, Box::new(move |app: &mut App| {
            app.threads[index].usage = Some(usage);
        })),
        AgentMsg::Error(index, message) => (index, Box::new(move |app: &mut App| {
            let thread = &mut app.threads[index];
            thread.error = Some(message);
            thread.live.clear();
        })),
        AgentMsg::Done(index, label) => (index, Box::new(move |app: &mut App| {
            app.threads[index].push(Role::System, label);
        })),
        AgentMsg::Loaded(index, snapshot) => (index, Box::new(move |app: &mut App| {
            let thread = &mut app.threads[index];
            thread.clear_transcript();
            thread.error = snapshot.error.clone();
            thread.usage = snapshot.usage.as_ref().map(|usage| Usage {
                input_tokens: usage.input_tokens,
                output_tokens: usage.output_tokens,
                cached_tokens: usage.cached_tokens,
            });
            thread.phase = match snapshot.phase {
                proto::WirePhase::Idle => Phase::Idle,
                _ => Phase::Streaming,
            };
            for message in &snapshot.transcript {
                let role = match message.role.as_str() {
                    "user" => Role::User,
                    "assistant" => Role::Assistant,
                    "tool" => Role::Tool,
                    _ => Role::System,
                };
                if role == Role::User {
                    thread.push_user(message.text.clone(), message.images);
                } else {
                    thread.push_kind(role, message.text.clone(), message.kind);
                }
            }
            if !snapshot.name.is_empty() {
                thread.name = snapshot.name.clone();
            }
            if !snapshot.mode.is_empty() {
                app.mode = snapshot.mode.clone();
            }
            if !snapshot.title.is_empty() {
                app.session_title = snapshot.title.clone();
            }
        })),
        AgentMsg::Rewound(text) => (0, Box::new(move |app: &mut App| {
            // The server has already cut the conversation and put the files
            // back; the prompt returns so it can be edited and sent again.
            let thread = &mut app.threads[0];
            thread.input = text.chars().collect();
            thread.cursor = thread.input.len();
            thread.popup_open = false;
            thread.error = None;
            thread.scroll = 0;
        })),
        AgentMsg::Dictation(text) => (0, Box::new(move |app: &mut App| {
            let thread = &mut app.threads[0];
            // Words arrive one committed piece at a time; they belong in the
            // prompt with a space between pieces.
            let needs_space = thread
                .input
                .last()
                .is_some_and(|ch| !ch.is_whitespace());
            if needs_space {
                thread.input.push(' ');
            }
            thread.input.extend(text.chars());
            thread.cursor = thread.input.len();
            // What was in flight is now in the prompt.
            thread.dictating = None;
        })),
        AgentMsg::Dictating(text) => (0, Box::new(move |app: &mut App| {
            let thread = &mut app.threads[0];
            thread.dictating = (!text.trim().is_empty()).then_some(text);
        })),
        AgentMsg::DictationStopped => (0, Box::new(move |app: &mut App| {
            app.dictation = None;
            app.dictating_since = None;
            if let Some(thread) = app.threads.first_mut() {
                thread.dictating = None;
            }
        })),
        AgentMsg::DictationFailed(complaint) => (0, Box::new(move |app: &mut App| {
            // Its own words: a build without the recording mode says so here.
            let first = complaint.lines().next().unwrap_or_default().trim();
            app.threads[0].dictating = None;
            app.threads[0].error = Some(format!("dictation: {first}"));
        })),
        AgentMsg::Catalog(list) => (0, Box::new(move |app: &mut App| {
            app.all_models = list.clone();
        })),
        AgentMsg::Skills(list) => (0, Box::new(move |app: &mut App| {
            app.skills = list.clone();
        })),
        AgentMsg::Cwd(path) => (0, Box::new(move |app: &mut App| {
            app.cwd = path.clone();
            app.threads[0].push(Role::System, format!("cwd: {path}"));
        })),
        AgentMsg::Title(index, title) => (index, Box::new(move |app: &mut App| {
            // The generated title replaces the placeholder taken from the first
            // prompt, in the footer and in the picker alike.
            app.session_title = title.clone();
            if let Some(thread) = app.threads.get_mut(index) {
                thread.name = title;
            }
        })),
        AgentMsg::Attached(index, name) => (index, Box::new(move |app: &mut App| {
            let thread = &mut app.threads[index];
            thread.clear_transcript();
            thread.usage = None;
            thread.error = None;
            thread.phase = Phase::Idle;
            thread.name = name;
        })),
        AgentMsg::Providers(list) => (0, Box::new(move |app: &mut App| {
            app.providers = list;
            app.open_provider_picker();
        })),
        AgentMsg::Models(list) => (0, Box::new(move |app: &mut App| {
            // The scope is only changed by ticking; opening the picker must not
            // rewrite it, or a provider's entry would stamp out another's.
            app.all_models = list;
            app.open_scoped_picker();
        })),
        AgentMsg::ProviderChanged(label, model, effort, message) => (0, Box::new(move |app: &mut App| {
            app.provider = label.clone();
            app.model = model.clone();
            app.effort = effort.clone();
            app.entering_key = None;
            app.threads[0].push(Role::System, message.clone());
        })),
        AgentMsg::Ask(index, request) => (index, Box::new(move |app: &mut App| {
            app.active = index;
            app.asking = Some(AskView {
                questions: request
                    .questions
                    .into_iter()
                    .map(|question| tools::AskItem {
                        label: question.label,
                        text: question.text,
                        options: question.options,
                    })
                    .collect(),
                submit_label: request.submit_label,
                index: 0,
                selected: 0,
                typed: false,
            });
            app.thread_mut().clear_input();
        })),
        AgentMsg::AskClosed(index) => (index, Box::new(move |app: &mut App| {
            let _ = index;
            app.asking = None;
        })),
        AgentMsg::Idle(index) => (index, Box::new(move |app: &mut App| {
            let thread = &mut app.threads[index];
            thread.phase = Phase::Idle;
            thread.live.clear();
            thread.started_at = None;
        })),
    };
    if index < app.threads.len() {
        update(app);
    }
}
