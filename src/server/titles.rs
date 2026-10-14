//! naming a session.

use std::sync::Arc;
use crate::api::ApiEvent;
use crate::catalog;
use crate::proto::WireEvent;
use crate::protocol::{ContentPart, InputItem, Reasoning, ResponsesRequest, Usage};
use crate::store;

use super::*;

/// Titles the session from its first prompt, off the turn's critical path.
pub(crate) fn maybe_name(shared: Arc<Shared>, session: Arc<Session>, first_prompt: String) {
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
pub(crate) fn regenerate_title(shared: Arc<Shared>, session: Arc<Session>) {
    tokio::spawn(async move {
        let text = conversation_for_title(&session).await;
        let (title, usage) = ask_for_title(&shared, &text).await;
        record_title_usage(&session, usage);
        store_title(&session, title);
    });
}
// ------------------------------------------------------------------ titles

/// The bundled T5, loaded at most once. Compiled in only with the `titler`
/// feature; without it, titles come from the model or not at all.
#[cfg(feature = "titler")]
static TITLER: std::sync::OnceLock<std::sync::Mutex<Option<titler::Titler>>> =
    std::sync::OnceLock::new();

#[cfg(feature = "titler")]
fn local_title(prompt: &str) -> Option<String> {
    let cell = TITLER
        .get_or_init(|| std::sync::Mutex::new(titler::Titler::from_hub("Cyronius/titler", None).ok()));
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
