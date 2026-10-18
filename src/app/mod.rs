//! Terminal UI state and rendering for exp-1 agent.
//!
//! Layout follows the OpenAI Codex TUI: a `>_` masthead, submitted prompts as
//! highlighted bands in the transcript, a completion popup or selection picker
//! above the input, a padded input band with a blinking caret, then a model line
//! and a hint line. Two threads exist — the main conversation and a side thread
//! opened with ctrl+/.

use crate::protocol::InputItem;
use ratatui::text::Line;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};


const SPINNER: [&str; 8] = ["⠋", "⠙", "⠹", "⠸", "⠼", "⠴", "⠦", "⠧"];

const MAIN_PLACEHOLDER: &str = "Ask exp-1 to do anything";
const SIDE_PLACEHOLDER: &str = "Ask a follow-up question";
const INPUT_HEIGHT: u16 = 3;
const POPUP_MAX: usize = 9;
const NAME_COLUMN: usize = 14;
const CARET: &str = "│";
const BLINK: Duration = Duration::from_millis(500);
pub const COMMANDS: [(&str, &str); 19] = [
    (
        "/login",
        "choose a provider; one that is signed in just switches",
    ),
    ("/scoped-models", "pick which models /model offers"),
    ("/init", "create an AGENTS.md file with instructions for exp-1"),
    ("/status", "show current session configuration"),
    ("/model", "choose what model and reasoning effort to use"),
    ("/themes", "switch the colour scheme"),
    ("/btw", "ask a side question without stopping the main task"),
    ("/sessions", "reopen an earlier conversation"),
    ("/schedule", "let a request run later, on its own"),
    ("/retitle", "name this session again from the conversation"),
    ("/effort", "choose the reasoning effort for the current model"),
    ("/mcp", "show the MCP servers and the tools they offer"),
    ("/cd", "move the session to another directory"),
    ("/compact", "summarise the conversation now instead of at the limit"),
    ("/undo", "put back the files the last turn changed"),
    ("/diff", "show what the last turn changed"),
    ("/fork", "continue this conversation as a new session"),
    ("/retry", "configure request attempts"),
    ("/help", "show keyboard shortcuts"),
];

pub fn pretty_model(name: &str) -> String {
    name.split('-')
        .map(|part| {
            if part.eq_ignore_ascii_case("gpt") {
                return "GPT".to_string();
            }
            let mut chars = part.chars();
            match chars.next() {
                Some(first) => first.to_uppercase().collect::<String>() + chars.as_str(),
                None => String::new(),
            }
        })
        .collect::<Vec<_>>()
        .join("-")
}

#[derive(Clone)]
pub struct Suggestion {
    pub label: String,
    pub insert: String,
    pub description: String,
}
/// Shortens a path to `~/…` the way a shell prompt does.
pub fn shorten_cwd(path: &std::path::Path) -> String {
    let full = path.to_string_lossy().to_string();
    match std::env::var_os("HOME") {
        Some(home) => {
            let home = home.to_string_lossy().to_string();
            if !home.is_empty() && full.starts_with(&home) {
                let rest = &full[home.len()..];
                if rest.is_empty() {
                    return "~".to_string();
                }
                return format!("~{rest}");
            }
        }
        None => {}
    }
    full
}
pub struct App {
    pub started: Instant,
    /// Models and their reasoning efforts, from the Codex cache.
    pub catalog: Vec<crate::catalog::ModelInfo>,
    /// Identifier of the saved session, assigned on the first turn.
    pub session_id: Option<String>,
    /// Name the model gave this session.
    pub session_title: String,
    /// Rows behind the `/sessions` picker, so a choice maps back to an id.
    pub session_list: Vec<crate::store::SessionMeta>,
    /// The provider registry, loaded when `/login` runs.
    pub providers: Vec<crate::proto::WireProvider>,
    /// Models the active provider offers, for `/scoped-models`.
    pub all_models: Vec<crate::proto::WireModel>,
    /// Models ticked in `/scoped-models`; empty means no restriction.
    pub scoped: std::collections::BTreeSet<String>,
    /// Set while an API key is being typed: (provider id, label).
    pub entering_key: Option<(String, String)>,
    /// Images queued from the clipboard for the next prompt: (media type, base64).
    pub pending_images: Vec<(String, String)>,
    /// Set once the namer has been started for this session.
    pub title_started: bool,
    /// Installed skills, as `("/name", description)`, so they sit beside the
    /// built-in commands in the popup and in the empty-state help.
    pub skills: Vec<(String, String)>,
    /// Set when a long turn finished while the user was probably away; the
    /// event loop rings the terminal bell and clears it.
    pub attention: bool,
    /// Ids of schedule notices already shown: the scheduler broadcasts each one
    /// to every attached session, and only the first delivery counts.
    pub seen_notices: std::collections::HashSet<String>,
    /// `build` or `plan`, switched with shift+tab.
    pub mode: String,
    /// How much the agent asks before a tool runs, switched with
    /// ctrl+shift+a; the server holds the authoritative copy per session.
    pub permissions: crate::tools::PermissionMode,
    /// True when there is nothing saved yet, so the empty screen can teach the
    /// commands once instead of on every launch.
    pub first_run: bool,
    /// Whether tool output is drawn under its call line. Off by default: the
    /// call is what a reader follows, the output is what they go back for.
    pub show_tool_output: bool,
    /// What the last frame drew that a click can act on. Interior mutability
    /// because the render passes only borrow the app.
    pub clicks: std::cell::RefCell<Vec<ClickTarget>>,
    /// What is typed while a picker is open, narrowing the rows shown. Long
    /// lists (every provider's models, say) are unusable without it.
    pub picker_filter: String,
    /// The message a click asked about, while its little menu is open.
    pub message_target: Option<(usize, String)>,
    /// The running recognizer, when the microphone is on.
    pub dictation: Option<crate::dictate::Dictation>,
    /// When the current dictation started, for the button's timer.
    pub dictating_since: Option<Instant>,
    /// A mouse drag over the screen: anchor and head, both (column, row).
    pub selection: Option<((u16, u16), (u16, u16))>,
    /// Plain text of every drawn screen row, refreshed each render, so a
    /// selection can be turned back into text. The terminal draws it; this is
    /// the only copy of what is on the screen.
    screen: Vec<String>,
    /// Short-lived footer note, with the moment it was set.
    pub copied: Option<(String, Instant)>,
    pub threads: Vec<Thread>,
    pub active: usize,
    pub last_esc: Option<Instant>,
    pub picker: Option<Picker>,
    /// Non-zero while the agent is blocked on an answer from the user.
    pub asking: Option<AskView>,
    pub theme: usize,
    /// Active provider id, shown in the footer and used by `/login`.
    pub provider: String,
    pub model: String,
    pub effort: String,
    pub plan: Option<String>,
    pub cwd: String,
    pub tick: usize,
    pub quit: bool,
}

impl App {
    /// Reasoning efforts the given model accepts, with their blurbs.
    pub fn efforts_for(&self, slug: &str) -> Vec<(String, String)> {
        self.catalog
            .iter()
            .find(|model| model.slug == slug)
            .map(|model| model.efforts.clone())
            .unwrap_or_default()
    }

    pub fn model_info(&self, slug: &str) -> Option<&crate::catalog::ModelInfo> {
        self.catalog.iter().find(|model| model.slug == slug)
    }

    /// Display name for the active model, from the catalogue when known.
    pub fn model_label(&self) -> String {
        self.model_info(&self.model)
            .map(|model| model.display_name.clone())
            .unwrap_or_else(|| pretty_model(&self.model))
    }

    pub fn theme(&self) -> &'static Theme {
        &THEMES[self.theme % THEMES.len()]
    }

    fn caret_visible(&self) -> bool {
        (self.started.elapsed().as_millis() / BLINK.as_millis()) % 2 == 0
    }

    pub fn thread(&self) -> &Thread {
        &self.threads[self.active]
    }

    pub fn thread_mut(&mut self) -> &mut Thread {
        let index = self.active;
        &mut self.threads[index]
    }

    pub fn new(
        cwd: String,
        main_history: Arc<Mutex<Vec<InputItem>>>,
        side_history: Arc<Mutex<Vec<InputItem>>>,
    ) -> Self {
        let main = Thread::new(ThreadKind::Main, main_history);
        let side = Thread::new(ThreadKind::Side, side_history);
        Self {
            started: Instant::now(),
            catalog: Vec::new(),
            session_id: None,
            session_title: String::new(),
            session_list: Vec::new(),
            providers: Vec::new(),
            all_models: Vec::new(),
            scoped: std::collections::BTreeSet::new(),
            entering_key: None,
            pending_images: Vec::new(),
            title_started: false,
            skills: Vec::new(),
            attention: false,
            seen_notices: std::collections::HashSet::new(),
            mode: "build".to_string(),
            permissions: crate::tools::PermissionMode::AlwaysAsk,
            first_run: false,
            show_tool_output: false,
            clicks: std::cell::RefCell::new(Vec::new()),
            picker_filter: String::new(),
            message_target: None,
            dictation: None,
            dictating_since: None,
            selection: None,
            screen: Vec::new(),
            copied: None,
            threads: vec![main, side],
            active: 0,
            last_esc: None,
            picker: None,
            asking: None,
            theme: 0,
            provider: String::new(),
            model: String::new(),
            effort: String::new(),
            plan: None,
            cwd,
            tick: 0,
            quit: false,
        }
    }

    /// Command names while typing `/…`, then arguments after `/cmd `.
    pub fn suggestions(&self) -> Vec<Suggestion> {
        let thread = self.thread();
        if thread.busy() || !thread.popup_open || self.picker.is_some() {
            return Vec::new();
        }
        let text = thread.input_text();

        // `@file` completion: the word being typed picks the directory and the
        // needle, so `@src/ma` lists what starts with `ma` inside `src`.
        if let Some(partial) = at_token(&text) {
            return self.file_suggestions(&partial);
        }

        let Some(query) = text.strip_prefix('/') else {
            return Vec::new();
        };

        let Some((command, argument)) = query.split_once(' ') else {
            let needle = query.to_ascii_lowercase();
            // Built-in commands first, then whatever skills are installed.
            let mut all: Vec<(String, String)> = COMMANDS
                .iter()
                .map(|(name, description)| (name.to_string(), description.to_string()))
                .collect();
            all.extend(self.skills.iter().cloned());
            return all
                .into_iter()
                .filter(|(name, _)| {
                    name.trim_start_matches('/')
                        .to_ascii_lowercase()
                        .starts_with(&needle)
                })
                .map(|(name, description)| Suggestion {
                    label: name.clone(),
                    insert: format!("{name} "),
                    description,
                })
                .collect();
        };

        let parts: Vec<&str> = argument.split_whitespace().collect();
        let needle = parts.first().copied().unwrap_or_default().to_lowercase();
        match command {
            "model" => match parts.len() {
                // Model names, from the catalogue the provider publishes.
                0 => self
                    .catalog
                    .iter()
                    .filter(|model| {
                        model.listed && model.slug.to_lowercase().starts_with(&needle)
                    })
                    .map(|model| Suggestion {
                        label: model.slug.clone(),
                        insert: model.slug.clone(),
                        description: model.description.clone(),
                    })
                    .collect(),
                // A model is already chosen, so the second argument is the effort.
                1 if self.catalog.iter().any(|model| model.slug == parts[0]) => self
                    .efforts_for(parts[0])
                    .into_iter()
                    .filter(|(level, _)| level.to_lowercase().starts_with(&needle))
                    .map(|(level, blurb)| Suggestion {
                        label: level.clone(),
                        insert: level,
                        description: blurb,
                    })
                    .collect(),
                _ => Vec::new(),
            },
            "themes" if parts.is_empty() => THEMES
                .iter()
                .filter(|theme| theme.name.starts_with(&needle))
                .map(|theme| Suggestion {
                    label: theme.name.to_string(),
                    insert: theme.name.to_string(),
                    description: "colour scheme".to_string(),
                })
                .collect(),
            "retry" => {
                let needle = needle.trim_start_matches('<').to_ascii_lowercase();
                (1..=10)
                    .map(|attempts| attempts.to_string())
                    .filter(|attempts| attempts.starts_with(&needle))
                    .map(|attempts| Suggestion {
                        label: attempts.clone(),
                        insert: attempts,
                        description: "number of request attempts".to_string(),
                    })
                    .collect()
            },
            _ => Vec::new(),
        }
    }

    /// Files under the working directory that match an `@` mention.
    fn file_suggestions(&self, partial: &str) -> Vec<Suggestion> {
        let (dir_part, needle) = match partial.rsplit_once('/') {
            Some((dir, needle)) => (format!("{dir}/"), needle.to_ascii_lowercase()),
            None => (String::new(), partial.to_ascii_lowercase()),
        };
        let base = std::path::Path::new(&self.cwd).join(&dir_part);
        let Ok(entries) = std::fs::read_dir(&base) else {
            return Vec::new();
        };
        let mut matches: Vec<(String, bool)> = entries
            .filter_map(|entry| {
                let entry = entry.ok()?;
                let name = entry.file_name().to_string_lossy().to_string();
                // Hidden files stay out of the way unless asked for by name.
                if name.starts_with('.') && !needle.starts_with('.') {
                    return None;
                }
                if !name.to_ascii_lowercase().starts_with(&needle) {
                    return None;
                }
                let is_dir = entry.file_type().map(|kind| kind.is_dir()).unwrap_or(false);
                Some((name, is_dir))
            })
            .collect();
        matches.sort();
        matches
            .into_iter()
            .take(12)
            .map(|(name, is_dir)| {
                let tail = if is_dir { "/" } else { "" };
                Suggestion {
                    label: format!("@{dir_part}{name}{tail}"),
                    // A directory keeps the mention open; a file closes it.
                    insert: if is_dir {
                        format!("@{dir_part}{name}/")
                    } else {
                        format!("@{dir_part}{name} ")
                    },
                    description: if is_dir { "directory" } else { "file" }.to_string(),
                }
            })
            .collect()
    }

    pub fn accept_suggestion(&mut self) -> bool {
        let suggestions = self.suggestions();
        let Some(suggestion) = suggestions.get(self.thread().selected).cloned() else {
            return false;
        };
        let thread = self.thread_mut();
        while matches!(thread.input.last(), Some(c) if !c.is_whitespace()) {
            thread.input.pop();
        }
        thread.input.extend(suggestion.insert.chars());
        thread.cursor = thread.input.len();
        thread.selected = 0;
        thread.popup_open = false;
        true
    }

    pub fn is_bare_command(&self, name: &str) -> bool {
        self.thread().input_text().trim() == name
    }

    /// Whether the scope lets this row through. An empty scope means every
    /// row does, and a bare name in an older scope means "this name, from any
    /// provider that offers it".
    pub fn model_allowed(&self, model: &crate::proto::WireModel) -> bool {
        self.scoped.is_empty()
            || self.scoped.contains(&scope_key(model))
            || self.scoped.contains(&model.slug)
    }

    /// The models `/model` offers: everything, or what the scope allows.
    pub fn offered_models(&self) -> Vec<crate::proto::WireModel> {
        self.all_models
            .iter()
            .filter(|model| self.model_allowed(model))
            .cloned()
            .collect()
    }

    /// Opens a picker with nothing typed in it yet, so the rows are the whole
    /// list.
    fn set_picker(&mut self, picker: Picker) {
        self.picker_filter.clear();
        self.picker = Some(picker);
    }

    /// Puts the selection on the first row the filter leaves.
    pub fn select_first_shown_row(&mut self) {
        let Some(picker) = self.picker.as_ref() else {
            return;
        };
        let first = self
            .shown_picker_items(picker)
            .first()
            .map(|(index, _)| *index);
        if let (Some(first), Some(picker)) = (first, self.picker.as_mut()) {
            picker.selected = first;
        }
    }

    /// The rows a picker shows, with their place in the full list: the filter
    /// narrows them, and everything remembers where it came from.
    pub fn shown_picker_items<'a>(&self, picker: &'a Picker) -> Vec<(usize, &'a PickerItem)> {
        let needle = self.picker_filter.trim().to_lowercase();
        picker
            .items
            .iter()
            .enumerate()
            .filter(|(_, item)| {
                needle.is_empty()
                    || item.label.to_lowercase().contains(&needle)
                    || item.description.to_lowercase().contains(&needle)
            })
            .collect()
    }

    /// Moves the picker's selection by `delta` places through the rows it is
    /// showing, keeping it inside them.
    pub fn move_picker_selection(&mut self, delta: isize) {
        let Some(picker) = self.picker.as_ref() else {
            return;
        };
        let shown: Vec<usize> = self
            .shown_picker_items(picker)
            .into_iter()
            .map(|(index, _)| index)
            .collect();
        if shown.is_empty() {
            return;
        }
        let at = shown.iter().position(|index| *index == picker.selected);
        let next = match (at, delta) {
            (Some(at), _) => (at as isize + delta).clamp(0, shown.len() as isize - 1) as usize,
            (None, 1) => 0,
            (None, _) => shown.len() - 1,
        };
        let selected = shown[next];
        if let Some(picker) = self.picker.as_mut() {
            picker.selected = selected;
        }
    }

    /// The id of the provider in use, when the list has arrived.
    pub fn active_provider_id(&self) -> Option<&str> {
        self.providers
            .iter()
            .find(|provider| provider.active)
            .map(|provider| provider.id.as_str())
    }

    pub fn open_model_picker(&mut self) {
        let current = self.model.clone();
        let active = self.active_provider_id().map(str::to_string);
        let mine = |model: &crate::proto::WireModel| {
            model.slug == current && active.as_deref().is_none_or(|id| id == model.provider)
        };
        // What the scope allows, which is everything when it is empty.
        let offered = self.offered_models();
        let items = offered
            .iter()
            .map(|model| PickerItem {
                label: format!(
                    "{}{}",
                    model.display_name,
                    if mine(model) { " (current)" } else { "" }
                ),
                description: model.description.clone(),
            })
            .collect();
        let selected = offered.iter().position(mine).unwrap_or(0);
        self.set_picker(Picker {
            kind: PickerKind::Model,
            title: "Select Model and Effort".to_string(),
            items,
            selected,
        });
    }

    /// Opens the effort picker. `/effort` opens it on its own; choosing a model
    /// opens it as the second half of that choice, hence the title.
    pub fn open_effort_picker(&mut self, title: &str) {
        // The effort list belongs to the model, so it is read fresh each time.
        let current = self.effort.clone();
        let efforts = self.efforts_for(&self.model);
        let items = efforts
            .iter()
            .map(|(level, blurb)| PickerItem {
                label: format!(
                    "{level}{}",
                    if *level == current { " (current)" } else { "" }
                ),
                description: blurb.clone(),
            })
            .collect();
        let selected = efforts
            .iter()
            .position(|(level, _)| *level == current)
            .unwrap_or(0);
        self.set_picker(Picker {
            kind: PickerKind::Effort,
            title: title.to_string(),
            items,
            selected,
        });
    }

    /// `/login` — one row per provider, with a marker for the active one.
    pub fn open_provider_picker(&mut self) {
        let items = self
            .providers
            .iter()
            .map(|provider| {
                let status = if provider.active {
                    "active".to_string()
                } else if provider.logged_in {
                    "signed in".to_string()
                } else {
                    "not configured".to_string()
                };
                crate::app::PickerItem {
                    label: format!(
                        "{}{}",
                        provider.label,
                        if provider.active { " (current)" } else { "" }
                    ),
                    description: status,
                }
            })
            .collect();
        self.set_picker(Picker {
            kind: PickerKind::Provider,
            title: "Login".to_string(),
            items,
            selected: self
                .providers
                .iter()
                .position(|provider| provider.active)
                .unwrap_or(0),
        });
    }

    /// A click on your own message asks what to do with it. Two things can
    /// happen, and neither is a surprise.
    pub fn open_message_picker(&mut self, index: usize, text: String) {
        let preview: String = text.chars().take(60).collect();
        self.message_target = Some((index, text));
        self.set_picker(Picker {
            kind: PickerKind::Message,
            title: format!("Message · {preview}"),
            items: vec![
                PickerItem {
                    label: "copy".to_string(),
                    description: "put the text on the clipboard".to_string(),
                },
                PickerItem {
                    label: "revert".to_string(),
                    description: "go back to it: the conversation and the file changes after it go"
                        .to_string(),
                },
            ],
            selected: 0,
        });
    }

    /// `/scoped-models` — tick the models `/model` is allowed to offer.
    pub fn open_scoped_picker(&mut self) {
        let items = self
            .all_models
            .iter()
            .map(|model| crate::app::PickerItem {
                label: format!(
                    "{} {}",
                    if self.model_allowed(model) { "[x]" } else { "[ ]" },
                    scope_key(model)
                ),
                description: model.description.clone(),
            })
            .collect();
        self.set_picker(Picker {
            kind: PickerKind::Scoped,
            title: "Models shown in /model".to_string(),
            items,
            selected: 0,
        });
    }

    pub fn open_session_picker(&mut self, sessions: Vec<crate::store::SessionMeta>) {
        let items = sessions
            .iter()
            .map(|session| PickerItem {
                label: session.label(),
                description: format!("{} · {}", session.id, session.when()),
            })
            .collect();
        self.session_list = sessions;
        self.set_picker(Picker {
            kind: PickerKind::Session,
            title: "Sessions".to_string(),
            items,
            selected: 0,
        });
    }

    pub fn open_theme_picker(&mut self) {
        let items = THEMES
            .iter()
            .enumerate()
            .map(|(index, theme)| PickerItem {
                label: format!(
                    "{}{}",
                    theme.name,
                    if index == self.theme { " (current)" } else { "" }
                ),
                description: "colour scheme".to_string(),
            })
            .collect();
        self.set_picker(Picker {
            kind: PickerKind::Theme,
            title: "Select Theme".to_string(),
            items,
            selected: self.theme,
        });
    }

    /// How full the current model's window is, when both halves are known.
    pub fn context_share(&self) -> Option<(u64, u64)> {
        let used = self.thread().usage.as_ref()?.input_tokens;
        let window = self
            .all_models
            .iter()
            .find(|model| model.slug == self.model)?
            .context_window;
        (window > 0 && used > 0).then_some((used, window))
    }

    fn model_line(&self) -> String {
        // The spinner lives in the transcript now, so this stays a plain label.
        format!("{} {}", self.model_label(), self.effort)
    }
}

mod picker;
mod render;
mod state;
mod text;
mod theme;

pub(crate) use self::{picker::*, render::*, state::*, text::*, theme::*};

#[cfg(test)]
mod tests;
