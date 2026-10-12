//! Terminal UI state and rendering for exp-1 agent.
//!
//! Layout follows the OpenAI Codex TUI: a `>_` masthead, submitted prompts as
//! highlighted bands in the transcript, a completion popup or selection picker
//! above the input, a padded input band with a blinking caret, then a model line
//! and a hint line. Two threads exist — the main conversation and a side thread
//! opened with ctrl+/.

use ratatui::Frame;
use ratatui::layout::{Constraint, Direction, Layout, Rect};
use ratatui::style::{Color, Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, Paragraph};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use crate::md;
use crate::protocol::{InputItem, Usage};

const SPINNER: [&str; 8] = ["⠋", "⠙", "⠹", "⠸", "⠼", "⠴", "⠦", "⠧"];

const MAIN_PLACEHOLDER: &str = "Ask exp-1 to do anything";
const SIDE_PLACEHOLDER: &str = "Ask a follow-up question";
const INPUT_HEIGHT: u16 = 3;
const POPUP_MAX: usize = 9;
const NAME_COLUMN: usize = 14;
const CARET: &str = "│";
const BLINK: Duration = Duration::from_millis(500);

/// A colour scheme. Colours are truecolor so a terminal theme cannot remap them.
pub struct Theme {
    pub name: &'static str,
    pub accent: Color,
    pub dim: Color,
    pub selection: Color,
    pub input_bg: Color,
    pub warn: Color,
    pub code: Color,
    pub math: Color,
}

const fn rgb(r: u8, g: u8, b: u8) -> Color {
    Color::Rgb(r, g, b)
}

pub const THEMES: [Theme; 4] = [
    Theme {
        name: "default",
        accent: rgb(59, 130, 246),
        dim: rgb(120, 120, 120),
        selection: rgb(59, 130, 246),
        input_bg: rgb(58, 58, 58),
        warn: rgb(239, 111, 111),
        code: rgb(126, 200, 140),
        math: rgb(214, 178, 96),
    },
    Theme {
        name: "amber",
        accent: rgb(217, 145, 32),
        dim: rgb(133, 122, 100),
        selection: rgb(217, 145, 32),
        input_bg: rgb(56, 48, 34),
        warn: rgb(220, 120, 90),
        code: rgb(150, 205, 160),
        math: rgb(240, 200, 110),
    },
    Theme {
        name: "jade",
        accent: rgb(52, 190, 140),
        dim: rgb(110, 130, 118),
        selection: rgb(52, 190, 140),
        input_bg: rgb(36, 52, 45),
        warn: rgb(235, 150, 120),
        code: rgb(120, 205, 160),
        math: rgb(200, 225, 140),
    },
    Theme {
        name: "mono",
        accent: rgb(214, 214, 214),
        dim: rgb(122, 122, 122),
        selection: rgb(84, 88, 96),
        input_bg: rgb(50, 50, 50),
        warn: rgb(220, 130, 130),
        code: rgb(180, 180, 180),
        math: rgb(210, 210, 210),
    },
];

pub const COMMANDS: [(&str, &str); 18] = [
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

/// Splits text into lines no wider than `width`, breaking at spaces where it
/// can, so a long tool output or prompt is wrapped instead of cut off at the
/// edge of the terminal.
fn wrap_text(text: &str, width: usize) -> Vec<String> {
    if width == 0 {
        return vec![text.to_string()];
    }
    let mut lines: Vec<String> = Vec::new();
    let mut current = String::new();
    let mut used = 0usize;
    for word in text.split_inclusive(' ') {
        let mut chars: Vec<char> = word.chars().collect();
        // A word longer than the line is cut rather than allowed to overflow.
        while chars.len() > width {
            let rest: Vec<char> = chars.drain(width..).collect();
            if used > 0 {
                lines.push(std::mem::take(&mut current));
                used = 0;
            }
            lines.push(chars.iter().collect());
            chars = rest;
        }
        let word_width = chars.len();
        if used + word_width > width && used > 0 {
            lines.push(std::mem::take(&mut current));
            used = 0;
        }
        current.extend(chars);
        used += word_width;
    }
    if !current.is_empty() || lines.is_empty() {
        lines.push(current);
    }
    lines
}

/// The same for styled runs: a line of markdown is split at the width so the
/// scroll offset keeps counting real screen lines.
fn wrap_runs(runs: &[md::Run], width: usize) -> Vec<Vec<md::Run>> {
    if width == 0 {
        return vec![runs.to_vec()];
    }
    let mut out: Vec<Vec<md::Run>> = Vec::new();
    let mut current: Vec<md::Run> = Vec::new();
    let mut used = 0usize;
    for run in runs {
        let mut chunk = String::new();
        for ch in run.text.chars() {
            if used == width {
                if !chunk.is_empty() {
                    current.push(md::Run::new(run.style, &chunk));
                    chunk.clear();
                }
                out.push(std::mem::take(&mut current));
                used = 0;
            }
            chunk.push(ch);
            used += 1;
        }
        if !chunk.is_empty() {
            current.push(md::Run::new(run.style, &chunk));
        }
    }
    if !current.is_empty() || out.is_empty() {
        out.push(current);
    }
    out
}

/// The word being typed when it is an `@` mention, without the `@`.
fn at_token(text: &str) -> Option<String> {
    let word = text.split_whitespace().last()?;
    word.strip_prefix('@').map(str::to_string)
}

#[derive(Clone)]
pub struct PickerItem {
    pub label: String,
    pub description: String,
}

#[derive(Clone, Copy, PartialEq)]
pub enum PickerKind {
    Model,
    Effort,
    Theme,
    Session,
    Provider,
    Scoped,
}

impl Picker {
    /// Rows the panel wants: title, spacers, one per item, hints.
    pub fn height(&self) -> u16 {
        self.items.len() as u16 + 5
    }
}

#[derive(Clone)]
pub struct Picker {
    pub kind: PickerKind,
    pub title: String,
    pub items: Vec<PickerItem>,
    pub selected: usize,
}

#[derive(Clone, Copy, PartialEq)]
pub enum Role {
    User,
    Assistant,
    Tool,
    System,
}

pub struct Msg {
    pub role: Role,
    pub text: String,
    pub images: usize,
    /// Markdown parsed once, at push time. Empty for roles that are not rich text.
    pub runs: Vec<Vec<md::Run>>,
    /// Tool output is hidden unless the reader asks for it; a progress line is
    /// replaced in place by the next one.
    pub kind: crate::proto::WireMsgKind,
}

#[derive(Clone, Copy, PartialEq)]
pub enum Phase {
    Idle,
    Streaming,
    Running,
}

/// The question panel the agent is blocked on, mirroring the Question tool.
pub struct AskView {
    pub questions: Vec<crate::tools::AskItem>,
    pub submit_label: String,
    /// Which question is on screen when several were asked together.
    pub index: usize,
    /// Highlighted option.
    pub selected: usize,
    /// Set once the user has typed their own answer, which wins over `selected`.
    pub typed: bool,
}

impl AskView {
    pub fn current(&self) -> &crate::tools::AskItem {
        &self.questions[self.index.min(self.questions.len() - 1)]
    }

    pub fn tabs_visible(&self) -> bool {
        self.questions.len() > 1
    }

    /// Rows the panel needs, including its own blank spacers.
    pub fn height(&self) -> u16 {
        let item = self.current();
        let mut total = 2u16; // title + spacer
        if self.tabs_visible() {
            total += 2; // tab row + spacer
        }
        total += 2; // question + spacer
        total += item.options.len() as u16;
        total += 3; // spacer, "your own answer", spacer
        total += 1; // hints
        total
    }

    pub fn move_selection(&mut self, delta: isize) {
        let count = self.current().options.len();
        if count == 0 {
            self.selected = 0;
            return;
        }
        let next = self.selected as isize + delta;
        self.selected = next.clamp(0, count as isize - 1) as usize;
    }

    /// The answer for the current question: typed text wins, else the option.
    pub fn answer_for_current(&self, typed: &str) -> String {
        let text = typed.trim();
        if !text.is_empty() {
            return text.to_string();
        }
        self.current()
            .options
            .get(self.selected)
            .cloned()
            .unwrap_or_default()
    }
}

#[derive(Clone, Copy, PartialEq)]
pub enum ThreadKind {
    Main,
    Side,
}

impl ThreadKind {
    pub fn label(self) -> &'static str {
        match self {
            ThreadKind::Main => "Main [default]",
            ThreadKind::Side => "Agent",
        }
    }
}

/// One conversation. The side thread shares the main history so the model
/// answers a side question with full context.
pub struct Thread {
    pub kind: ThreadKind,
    pub name: String,
    pub transcript: Vec<Msg>,
    pub history: Arc<Mutex<Vec<InputItem>>>,
    pub phase: Phase,
    pub live: String,
    pub reasoning: String,
    pub usage: Option<Usage>,
    pub error: Option<String>,
    pub started_at: Option<Instant>,
    pub input: Vec<char>,
    pub cursor: usize,
    pub input_history: Vec<String>,
    pub history_index: Option<usize>,
    pub selected: usize,
    pub popup_open: bool,
    pub scroll: u16,
    /// Built lines per transcript message, so a long conversation is not
    /// re-wrapped on every frame. Rebuilt when the width, the theme or the
    /// output setting changes; a message that changes is rebuilt on its own.
    lines: Vec<Vec<Line<'static>>>,
    lines_width: usize,
    lines_theme: usize,
    lines_output: bool,
    /// Lines the last frame drew, so a view scrolled up can be held in place
    /// while new ones arrive.
    last_total: usize,
}

impl Thread {
    fn new(kind: ThreadKind, history: Arc<Mutex<Vec<InputItem>>>) -> Self {
        Self {
            kind,
            name: String::new(),
            transcript: Vec::new(),
            history,
            phase: Phase::Idle,
            live: String::new(),
            reasoning: String::new(),
            usage: None,
            error: None,
            started_at: None,
            input: Vec::new(),
            cursor: 0,
            input_history: Vec::new(),
            history_index: None,
            selected: 0,
            popup_open: false,
            scroll: 0,
            lines: Vec::new(),
            lines_width: 0,
            lines_theme: usize::MAX,
            lines_output: false,
            last_total: 0,
        }
    }

    /// Builds the lines for every message that does not have them yet. The
    /// width, the theme and the output setting are part of the cache key.
    fn bake(&mut self, width: usize, theme_index: usize, show_output: bool, theme: &'static Theme) {
        if self.lines_width != width || self.lines_theme != theme_index || self.lines_output != show_output
        {
            self.lines.clear();
            self.lines_width = width;
            self.lines_theme = theme_index;
            self.lines_output = show_output;
        }
        if self.lines.len() > self.transcript.len() {
            self.lines.truncate(self.transcript.len());
        }
        for msg in &self.transcript[self.lines.len()..] {
            self.lines.push(message_lines(msg, width, theme, show_output));
        }
    }

    pub fn busy(&self) -> bool {
        self.phase != Phase::Idle
    }

    pub fn input_text(&self) -> String {
        self.input.iter().collect()
    }

    pub fn placeholder(&self) -> &'static str {
        match self.kind {
            ThreadKind::Main => MAIN_PLACEHOLDER,
            ThreadKind::Side => SIDE_PLACEHOLDER,
        }
    }

    pub fn push_user(&mut self, text: impl Into<String>, images: usize) {
        let text = text.into();
        self.transcript.push(Msg {
            role: Role::User,
            text,
            images,
            runs: Vec::new(),
            kind: crate::proto::WireMsgKind::Plain,
        });
        // Sending means looking at the answer: the view goes back to the end.
        self.scroll = 0;
    }

    pub fn push(&mut self, role: Role, text: impl Into<String>) {
        self.push_kind(role, text, crate::proto::WireMsgKind::Plain);
    }

    pub fn push_kind(
        &mut self,
        role: Role,
        text: impl Into<String>,
        kind: crate::proto::WireMsgKind,
    ) {
        let text = text.into();
        let runs = match role {
            Role::Assistant => md::render(&text),
            _ => Vec::new(),
        };
        self.transcript.push(Msg {
            role,
            text,
            images: 0,
            runs,
            kind,
        });
    }

    /// A line the next one replaces: a subagent's current step, so its work
    /// does not fill the transcript one line at a time.
    pub fn push_progress(&mut self, text: impl Into<String>) {
        let text = text.into();
        match self.transcript.last_mut() {
            Some(last) if last.kind == crate::proto::WireMsgKind::Progress => {
                last.text = text;
                // The line changed, so its cached rendering is stale.
                self.lines.truncate(self.transcript.len() - 1);
            }
            _ => self.transcript.push(Msg {
                role: Role::Tool,
                text,
                images: 0,
                runs: Vec::new(),
                kind: crate::proto::WireMsgKind::Progress,
            }),
        }
    }

    pub fn clear_input(&mut self) {
        self.input.clear();
        self.cursor = 0;
        self.selected = 0;
        self.popup_open = false;
    }

    pub fn remember(&mut self, prompt: &str) {
        if self.input_history.last() == Some(&prompt.to_string()) {
            return;
        }
        self.input_history.push(prompt.to_string());
        self.history_index = None;
    }

    pub fn history_prev(&mut self) {
        if self.input_history.is_empty() {
            return;
        }
        let index = match self.history_index {
            None => self.input_history.len() - 1,
            Some(0) => 0,
            Some(index) => index - 1,
        };
        self.history_index = Some(index);
        self.input = self.input_history[index].chars().collect();
        self.cursor = self.input.len();
        self.popup_open = false;
    }

    pub fn history_next(&mut self) {
        let Some(index) = self.history_index else {
            return;
        };
        if index + 1 < self.input_history.len() {
            let next = index + 1;
            self.history_index = Some(next);
            self.input = self.input_history[next].chars().collect();
        } else {
            self.history_index = None;
            self.input.clear();
        }
        self.cursor = self.input.len();
        self.popup_open = false;
    }
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
    /// `build` or `plan`, switched with shift+tab.
    pub mode: String,
    /// True when there is nothing saved yet, so the empty screen can teach the
    /// commands once instead of on every launch.
    pub first_run: bool,
    /// Whether tool output is drawn under its call line. Off by default: the
    /// call is what a reader follows, the output is what they go back for.
    pub show_tool_output: bool,
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
            mode: "build".to_string(),
            first_run: false,
            show_tool_output: false,
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

    pub fn open_model_picker(&mut self) {
        let current = self.model.clone();
        let listed: Vec<&crate::catalog::ModelInfo> =
            self.catalog.iter().filter(|model| model.listed).collect();
        let items = listed
            .iter()
            .map(|model| PickerItem {
                label: format!(
                    "{}{}",
                    model.display_name,
                    if model.slug == current { " (current)" } else { "" }
                ),
                description: model.description.clone(),
            })
            .collect();
        let selected = listed
            .iter()
            .position(|model| model.slug == current)
            .unwrap_or(0);
        self.picker = Some(Picker {
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
        self.picker = Some(Picker {
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
        self.picker = Some(Picker {
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

    /// `/scoped-models` — tick the models `/model` is allowed to offer.
    pub fn open_scoped_picker(&mut self) {
        let items = self
            .all_models
            .iter()
            .map(|model| crate::app::PickerItem {
                label: format!(
                    "{} {}",
                    if self.scoped.is_empty() || self.scoped.contains(&model.slug) {
                        "[x]"
                    } else {
                        "[ ]"
                    },
                    model.slug
                ),
                description: model.description.clone(),
            })
            .collect();
        self.picker = Some(Picker {
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
        self.picker = Some(Picker {
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
        self.picker = Some(Picker {
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

    pub fn render(&mut self, frame: &mut Frame) {
        let area = frame.area();
        let suggestions = self.suggestions();

        // Height is budgeted top-down by hand. Letting the layout solver divide a
        // short terminal was what dropped the input band entirely once the fixed
        // rows no longer fitted.
        const FOOTER: u16 = 2;
        let footer = FOOTER.min(area.height);
        // A prompt written over several lines (shift+enter) takes the rows it
        // needs; the transcript gives them up.
        let typed_lines = self.thread().input.iter().filter(|ch| **ch == '\n').count() as u16;
        let attached = u16::from(!self.pending_images.is_empty());
        let input = (INPUT_HEIGHT + typed_lines + attached).min(area.height.saturating_sub(footer));
        let mut spare = area.height.saturating_sub(footer + input);

        // Keep at least one transcript row whenever there is room for one.
        let wants_overlay = if self.picker.is_some() {
            self.picker
                .as_ref()
                .map(Picker::height)
                .unwrap_or(0)
        } else {
            suggestions.len().min(POPUP_MAX) as u16
        };
        let overlay = wants_overlay.min(spare.saturating_sub(1));
        // A picker that renders nothing at all would leave the user stuck, so
        // give it a few rows even on a very short terminal.
        let overlay = if self.picker.is_some() && overlay < 3 {
            3.min(spare)
        } else {
            overlay
        };
        spare -= overlay;

        let wants_ask = self.asking.as_ref().map(AskView::height).unwrap_or(0);
        let ask = wants_ask.min(spare.saturating_sub(1));
        spare -= ask;

        let masthead = 2.min(spare);
        let transcript = spare - masthead;

        let rows = Layout::default()
            .direction(Direction::Vertical)
            .constraints([
                Constraint::Length(masthead),
                Constraint::Length(transcript),
                Constraint::Length(overlay),
                Constraint::Length(ask),
                Constraint::Length(input),
                Constraint::Length(footer.min(1)),
                Constraint::Length(footer.saturating_sub(1)),
            ])
            .split(area);

        self.render_masthead(frame, rows[0]);
        self.render_transcript(frame, rows[1], rows[1].width as usize);
        if let Some(picker) = &self.picker {
            self.render_picker(frame, rows[2], picker);
        } else if overlay > 0 {
            self.render_popup(frame, rows[2], &suggestions);
        }
        if let Some(ask) = &self.asking {
            self.render_ask(frame, rows[3], ask);
        }
        self.render_input(frame, rows[4]);
        self.render_footer(frame, rows[5], rows[6], !suggestions.is_empty());
        self.capture_screen(frame);
    }

    /// Starts a selection at a screen position.
    pub fn begin_selection(&mut self, column: u16, row: u16) {
        self.selection = Some(((column, row), (column, row)));
    }

    /// Moves the loose end of the selection.
    pub fn extend_selection(&mut self, column: u16, row: u16) {
        if let Some((anchor, _)) = self.selection {
            self.selection = Some((anchor, (column, row)));
        }
    }

    /// The selection as (top-left, bottom-right), whatever way it was dragged.
    pub fn ordered_selection(&self) -> Option<((u16, u16), (u16, u16))> {
        let (a, b) = self.selection?;
        if (a.1, a.0) <= (b.1, b.0) {
            Some((a, b))
        } else {
            Some((b, a))
        }
    }

    /// The selected text, taken from the screen as it was last drawn. Rows are
    /// trimmed at the end, because every row is padded to the full width.
    pub fn selected_text(&self) -> Option<String> {
        let ((x0, y0), (x1, y1)) = self.ordered_selection()?;
        let mut rows: Vec<String> = Vec::new();
        for y in y0..=y1 {
            let Some(row) = self.screen.get(y as usize) else {
                continue;
            };
            let chars: Vec<char> = row.chars().collect();
            let from = if y == y0 { x0 as usize } else { 0 }.min(chars.len());
            let to = if y == y1 {
                x1 as usize + 1
            } else {
                chars.len()
            }
            .min(chars.len())
            .max(from);
            rows.push(chars[from..to].iter().collect::<String>());
        }
        let text = rows.join("\n").trim_end().to_string();
        (!text.is_empty()).then_some(text)
    }

    /// Snapshots the drawn screen for copying, then paints the selection over
    /// it — the buffer is the one place the screen's text and its cells meet.
    fn capture_screen(&mut self, frame: &mut Frame) {
        let area = frame.area();
        let selection = self.ordered_selection();
        let theme = self.theme();
        // A selection reads as a normal terminal one: a quiet tint behind the
        // text, not the loud colour the pickers use for a highlighted row.
        let highlight = Style::default().bg(theme.input_bg);
        let buf = frame.buffer_mut();
        self.screen = (area.y..area.y + area.height)
            .map(|y| {
                let mut line = String::new();
                for x in area.x..area.x + area.width {
                    if let Some(cell) = buf.cell((x, y)) {
                        line.push_str(cell.symbol());
                    }
                }
                line.trim_end().to_string()
            })
            .collect();
        let Some(((x0, y0), (x1, y1))) = selection else {
            return;
        };
        let last = area.x + area.width - 1;
        for y in y0..=y1 {
            // A row is only as long as its text: painting the padding past the
            // last character is what turned a selection into a solid block.
            let len = self
                .screen
                .get(y as usize)
                .map(|row| row.chars().count() as u16)
                .unwrap_or(0);
            if len == 0 {
                continue;
            }
            let row_end = area.x + len - 1;
            let (from, to) = if y == y0 && y == y1 {
                (x0, x1)
            } else if y == y0 {
                (x0, last)
            } else if y == y1 {
                (area.x, x1)
            } else {
                (area.x, last)
            };
            let to = to.min(row_end);
            let from = from.min(row_end);
            if from > to {
                continue;
            }
            for x in from..=to {
                if let Some(cell) = buf.cell_mut((x, y)) {
                    cell.set_style(highlight);
                }
            }
        }
    }

    /// The question panel: accent bar on the left, tabs, numbered options.
    fn render_ask(&self, frame: &mut Frame, area: Rect, ask: &AskView) {
        let theme = self.theme();
        let item = ask.current();

        let bar = |style: Style| {
            vec![
                Span::styled("▌", style),
                Span::styled(" ", Style::default().bg(theme.input_bg)),
            ]
        };
        let push = |lines: &mut Vec<Line<'static>>, runs: Vec<(Style, String)>| {
            let mut spans = bar(runs.first().map(|(style, _)| *style).unwrap_or_default());
            for (style, text) in runs {
                spans.push(Span::styled(
                    text,
                    Style::default().bg(theme.input_bg).patch(style),
                ));
            }
            lines.push(Line::from(spans));
        };

        let mut lines: Vec<Line<'static>> = Vec::new();
        let title = Style::default().add_modifier(Modifier::BOLD);
        let dim = Style::default().fg(theme.dim);
        let chosen = Style::default().fg(theme.accent).add_modifier(Modifier::BOLD);

        push(&mut lines, vec![(title, "Questions".to_string())]);
        push(&mut lines, vec![(dim, String::new())]);

        if ask.tabs_visible() {
            let mut runs = Vec::new();
            for (position, question) in ask.questions.iter().enumerate() {
                if position > 0 {
                    runs.push((dim, "    ".to_string()));
                }
                let active = position == ask.index;
                runs.push((
                    if active { chosen } else { dim },
                    question.label.clone(),
                ));
            }
            runs.push((dim, format!("    {}", ask.submit_label)));
            push(&mut lines, runs);
            push(&mut lines, vec![(dim, String::new())]);
        }

        push(&mut lines, vec![(title, item.text.clone())]);
        push(&mut lines, vec![(dim, String::new())]);

        for (position, option) in item.options.iter().enumerate() {
            let active = position == ask.selected;
            push(
                &mut lines,
                vec![(
                    if active { chosen } else { Style::default() },
                    format!("{}. {}", position + 1, option),
                )],
            );
        }

        push(&mut lines, vec![(dim, String::new())]);
        let own = if ask.typed {
            Style::default().fg(theme.accent)
        } else {
            Style::default()
        };
        push(
            &mut lines,
            vec![(
                own,
                "Type your own answer".to_string(),
            )],
        );
        push(&mut lines, vec![(dim, String::new())]);
        push(
            &mut lines,
            vec![(
                dim,
                "⇥tab  ↑↓ select  enter confirm  esc dismiss".to_string(),
            )],
        );

        frame.render_widget(Paragraph::new(lines), area);
    }

    fn render_masthead(&self, frame: &mut Frame, area: Rect) {
        let theme = self.theme();
        let lines = vec![
            Line::from(vec![
                Span::styled(">_", Style::default().fg(theme.accent)),
                Span::raw(" "),
                Span::styled("exp-1 agent", Style::default().add_modifier(Modifier::BOLD)),
                Span::styled(
                    format!(" (v{})", env!("CARGO_PKG_VERSION")),
                    Style::default().fg(theme.dim),
                ),
            ]),
            Line::from(vec![
                Span::raw("  "),
                Span::styled(self.cwd.clone(), Style::default().fg(theme.dim)),
            ]),
        ];
        frame.render_widget(Paragraph::new(lines), area);
    }

    fn render_transcript(&mut self, frame: &mut Frame, area: Rect, width: usize) {
        let theme = self.theme();
        let (theme_index, show_output) = (self.theme, self.show_tool_output);

        // Only what is missing is built: a long conversation must not be
        // re-wrapped on every frame.
        {
            let thread = self.thread_mut();
            thread.bake(width, theme_index, show_output, theme);
        }
        let thread = self.thread();

        let mut prefix: Vec<Line> = Vec::new();
        if thread.transcript.is_empty() {
            prefix.push(Line::raw(""));
            if self.first_run {
                // First run shows command names only; descriptions are available
                // through the completion popup and /help.
                prefix.push(Line::styled(
                    "  To get started, describe a task or try one of these commands:",
                    Style::default().fg(theme.dim),
                ));
                prefix.push(Line::raw(""));
                for (name, _) in COMMANDS {
                    prefix.push(Line::from(vec![
                        Span::raw("  "),
                        Span::styled(name, Style::default().fg(theme.accent)),
                    ]));
                }
                for (name, _) in &self.skills {
                    prefix.push(Line::from(vec![
                        Span::raw("  "),
                        Span::styled(name.clone(), Style::default().fg(theme.accent)),
                    ]));
                }
            } else {
                prefix.push(Line::styled(
                    "  Describe a task to start; / opens the commands, /help the keys.",
                    Style::default().fg(theme.dim),
                ));
            }
        }

        // The streamed text and the activity line are small, so they are built
        // fresh each frame.
        let mut tail: Vec<Line> = Vec::new();
        if !thread.live.is_empty() {
            // The text being streamed is markdown too, so it is formatted while
            // it arrives instead of snapping into shape once the turn ends.
            for run_line in md::render(&thread.live) {
                for part in wrap_runs(&run_line, width) {
                    let mut spans: Vec<Span> = Vec::new();
                    for run in &part {
                        if run.style == md::Style::CodeBlock {
                            spans.push(Span::styled("│ ", Style::default().fg(theme.dim)));
                        }
                        spans.push(Span::styled(run.text.clone(), md_style(run.style, theme)));
                    }
                    if spans.is_empty() {
                        spans.push(Span::raw(""));
                    }
                    tail.push(Line::from(spans));
                }
            }
        } else if thread.busy() {
            // The activity line sits directly under the prompt it belongs to.
            let label = if thread.phase == Phase::Running {
                "running"
            } else {
                "thinking"
            };
            let elapsed = thread
                .started_at
                .map(|start| start.elapsed().as_secs())
                .unwrap_or(0);
            tail.push(Line::from(vec![
                Span::raw("  "),
                Span::styled(
                    SPINNER[self.tick % SPINNER.len()].to_string(),
                    Style::default().fg(theme.accent),
                ),
                Span::styled(
                    format!(" {label} {elapsed}s"),
                    Style::default().fg(theme.dim),
                ),
            ]));
        }

        let total =
            prefix.len() + thread.lines.iter().map(Vec::len).sum::<usize>() + tail.len();
        let height = area.height as usize;
        let max_scroll = total.saturating_sub(height);

        // A view scrolled up stays where it is while new lines arrive; one at
        // the bottom follows the conversation.
        {
            let thread = self.thread_mut();
            let grew = total.saturating_sub(thread.last_total);
            if grew > 0 && thread.scroll > 0 {
                thread.scroll = thread.scroll.saturating_add(grew as u16);
            }
            thread.last_total = total;
        }
        let thread = self.thread();

        // `scroll` counts lines scrolled up from the bottom, so the newest
        // message is what you see unless you wheel upwards.
        let scroll = (thread.scroll as usize).min(max_scroll);
        let offset = max_scroll - scroll;
        let mut visible: Vec<Line> = Vec::with_capacity(height);
        for line in prefix
            .iter()
            .chain(thread.lines.iter().flatten())
            .chain(tail.iter())
            .skip(offset)
            .take(height)
        {
            visible.push(line.clone());
        }
        frame.render_widget(Paragraph::new(visible), area);
    }

    fn render_popup(&self, frame: &mut Frame, area: Rect, suggestions: &[Suggestion]) {
        let theme = self.theme();
        let width = area.width as usize;
        let height = area.height as usize;
        let selected = self.thread().selected;
        // The label column follows the longest label, so a long command name
        // can never run into its own description.
        let name_column = suggestions
            .iter()
            .map(|suggestion| suggestion.label.chars().count())
            .max()
            .unwrap_or(NAME_COLUMN)
            .clamp(NAME_COLUMN, 30);
        // The list is often longer than the room the layout gives it, so the
        // window follows the highlight instead of always starting at the top.
        let start = if height == 0 || selected < height {
            0
        } else {
            selected + 1 - height
        };
        let mut lines: Vec<Line> = Vec::new();
        for (offset, suggestion) in suggestions.iter().skip(start).take(height.max(1)).enumerate() {
            let index = start + offset;
            let text: String = format!(
                "  {:<name_column$} {}",
                suggestion.label, suggestion.description
            )
            .chars()
            .take(width)
            .collect();
            let style = if index == selected {
                Style::default().bg(theme.selection).fg(rgb(255, 255, 255))
            } else {
                Style::default()
            };
            lines.push(Line::styled(pad_to(&text, width), style));
        }
        frame.render_widget(Paragraph::new(lines), area);
    }

    fn render_picker(&self, frame: &mut Frame, area: Rect, picker: &Picker) {
        let theme = self.theme();
        let width = area.width as usize;
        let height = area.height as usize;

        // Spacers are a nicety; items are the point. On a short panel the
        // spacers go first so the choices stay visible.
        let compact = height < 8;
        let mut lines: Vec<Line> = Vec::new();
        if !compact {
            lines.push(Line::raw(""));
        }
        lines.push(Line::styled(
            format!("  {}", picker.title),
            Style::default().add_modifier(Modifier::BOLD),
        ));
        if !compact {
            lines.push(Line::raw(""));
        }

        let total = picker.items.len();
        // One row for the hints, plus one for the range line when not compact.
        let tail = if compact { 1 } else { 2 };
        let capacity = height
            .saturating_sub(lines.len() + tail)
            .max(1);
        let visible = capacity.min(total);
        let start = if total <= visible {
            0
        } else {
            // Centre the window on the selection so it cannot scroll out of sight.
            picker
                .selected
                .saturating_sub(visible / 2)
                .min(total - visible)
        };

        let column = picker
            .items
            .iter()
            .map(|item| item.label.chars().count())
            .max()
            .unwrap_or(0)
            + 2;
        for (offset, item) in picker.items[start..start + visible].iter().enumerate() {
            let index = start + offset;
            let selected = index == picker.selected;
            let marker = if selected { ">" } else { " " };
            let body: String = format!(
                " {marker} {}. {:<column$}{}",
                index + 1,
                item.label,
                item.description
            );
            let style = if selected {
                Style::default().bg(theme.selection).fg(rgb(255, 255, 255))
            } else {
                Style::default()
            };
            lines.push(Line::styled(pad_to(&body, width), style));
        }

        if !compact && total > visible {
            lines.push(Line::styled(
                format!("  ({}-{} of {total})", start + 1, start + visible),
                Style::default().fg(theme.dim),
            ));
        }
        lines.push(Line::from(vec![
            Span::raw("  "),
            Span::styled("enter select · esc back", Style::default().fg(theme.dim)),
        ]));

        let block = Block::default().style(Style::default().bg(theme.input_bg));
        frame.render_widget(Paragraph::new(lines).block(block), area);
    }


    fn render_input(&self, frame: &mut Frame, area: Rect) {
        let theme = self.theme();
        let thread = self.thread();
        let caret = Style::default().fg(theme.accent);

        // Typing stays live during a turn, so the caret keeps blinking.
        let show_caret = self.picker.is_none() && self.caret_visible();

        // The prompt can span lines, so it is laid out line by line with the
        // caret at the cursor's own line and column.
        let mut lines: Vec<Line> = vec![Line::raw("")];
        // An attached picture shows as a filled chip above the prompt, the way
        if !self.pending_images.is_empty() {
            lines.push(Line::from(vec![
                Span::styled("  ", Style::default()),
                Span::styled(
                    format!("{} image{} queued", self.pending_images.len(), if self.pending_images.len() == 1 { "" } else { "s" }),
                    Style::default().bg(theme.accent).fg(theme.input_bg),
                ),
                Span::styled(
                    "  ctrl-v adds another · enter sends",
                    Style::default().fg(theme.dim),
                ),
            ]));
        }
        if thread.input.is_empty() {
            let mut spans = vec![Span::styled("› ", Style::default().fg(theme.dim))];
            if show_caret {
                spans.push(Span::styled(CARET, caret));
            }
            spans.push(Span::styled(
                if self.entering_key.is_some() {
                    "paste the API key"
                } else if self.asking.is_some() {
                    "your answer"
                } else {
                    thread.placeholder()
                },
                Style::default().fg(theme.dim),
            ));
            lines.push(Line::from(spans));
        } else {
            let mut spans = vec![Span::styled("› ", Style::default().fg(theme.dim))];
            for (index, ch) in thread.input.iter().enumerate() {
                if index == thread.cursor && show_caret {
                    spans.push(Span::styled(CARET, caret));
                }
                if *ch == '\n' {
                    lines.push(Line::from(std::mem::take(&mut spans)));
                    spans.push(Span::styled("  ", Style::default().fg(theme.dim)));
                    continue;
                }
                spans.push(Span::raw(ch.to_string()));
            }
            if thread.cursor >= thread.input.len() {
                if show_caret {
                    spans.push(Span::styled(CARET, caret));
                } else {
                    spans.push(Span::raw(" "));
                }
            }
            lines.push(Line::from(spans));
        }
        lines.push(Line::raw(""));

        let block = Block::default().style(Style::default().bg(theme.input_bg));
        frame.render_widget(Paragraph::new(lines).block(block), area);
    }

    fn render_footer(&self, frame: &mut Frame, model_area: Rect, hint_area: Rect, popup_open: bool) {
        let theme = self.theme();
        let thread = self.thread();

        let mut model_spans = vec![
            Span::raw("  "),
            Span::styled(self.model_line(), Style::default().fg(theme.dim)),
            Span::styled(format!("  ·  {}", self.cwd), Style::default().fg(theme.dim)),
        ];
        if !thread.name.is_empty() {
            model_spans.push(Span::raw("  ·  "));
            model_spans.push(Span::styled(
                thread.name.clone(),
                Style::default().fg(theme.accent),
            ));
        }
        model_spans.push(Span::styled(
            format!("  ·  {}", thread.kind.label()),
            Style::default().fg(theme.dim),
        ));
        // Only worth showing when it is not the default: plan mode changes what
        // the agent is allowed to do.
        if self.mode == "plan" {
            model_spans.push(Span::styled(
                "  ·  plan mode",
                Style::default().fg(theme.warn),
            ));
        }
        if let Some(usage) = &thread.usage {
            model_spans.push(Span::styled(
                format!("  ·  {}", usage.render()),
                Style::default().fg(theme.dim),
            ));
        }
        // How much of the window is gone, so a long turn is never a surprise.
        if let Some((used, window)) = self.context_share() {
            model_spans.push(Span::styled(
                format!(
                    "  ·  {}% of {}k",
                    used.saturating_mul(100) / window.max(1),
                    window / 1000
                ),
                Style::default().fg(theme.dim),
            ));
        }
        frame.render_widget(Paragraph::new(Line::from(model_spans)), model_area);

        // A fresh copy note takes the right-hand slot for a moment.
        let note = self
            .copied
            .as_ref()
            .filter(|(_, at)| at.elapsed() < std::time::Duration::from_millis(2500));
        let right = match note {
            Some((text, _)) => text.as_str(),
            None => match thread.kind {
                ThreadKind::Main => "ctrl + / for side",
                ThreadKind::Side => "ctrl + / to switch · ctrl + c to close",
            },
        };
        let right_style = match (note, thread.kind) {
            (Some(_), _) => Style::default().fg(theme.accent),
            (None, ThreadKind::Main) => Style::default().fg(theme.dim),
            (None, ThreadKind::Side) => Style::default().fg(theme.warn),
        };

        let left_text = if let Some(error) = &thread.error {
            format!("{error} · ctrl-c to quit")
        } else if popup_open {
            let total = self.suggestions().len();
            let chosen = self.thread().selected.min(total.saturating_sub(1)) + 1;
            format!("↑↓ select {chosen}/{total} · tab complete · enter accept")
        } else if thread.busy() {
            "esc esc interrupts · /btw runs alongside".to_string()
        } else {
            "enter send · ctrl-c quit · ctrl-o output · ↑↓ history · scroll to page".to_string()
        };
        let left_style = if thread.error.is_some() {
            Style::default().fg(theme.warn)
        } else {
            Style::default().fg(theme.dim)
        };

        // Lay the line out to exactly the available width: two spaces of
        // leading indent, the left hint, a gap, then the right hint. Missing the
        // indent here is what clipped the right-hand text.
        const INDENT: usize = 2;
        let width = hint_area.width as usize;

        let mut right = right.to_string();
        if right.chars().count() + INDENT + 1 > width {
            right = right.chars().take(width.saturating_sub(INDENT + 1)).collect();
        }
        let right_len = right.chars().count();

        let budget = width.saturating_sub(INDENT + 1 + right_len);
        let mut left_text: String = left_text.chars().take(budget).collect();
        // Never leave a dangling separator or half a word behind.
        if left_text.chars().count() == budget
            && let Some(cut) = left_text.rfind(" · ")
        {
            left_text.truncate(cut);
        }
        // Too short to say anything useful; a clean gap beats a fragment.
        if left_text.chars().count() < 12 {
            left_text.clear();
        }
        let gap = width.saturating_sub(INDENT + left_text.chars().count() + right_len);

        let mut line: Vec<Span> = vec![Span::raw(" ".repeat(INDENT))];
        line.push(Span::styled(left_text, left_style));
        line.push(Span::raw(" ".repeat(gap)));
        line.push(Span::styled(right, right_style));
        frame.render_widget(Paragraph::new(Line::from(line)), hint_area);
    }
}

/// Maps a parsed run to concrete colours for the active theme.
fn md_style(role: md::Style, theme: &Theme) -> Style {
    match role {
        md::Style::Text => Style::default(),
        md::Style::Heading => Style::default()
            .fg(theme.accent)
            .add_modifier(Modifier::BOLD),
        md::Style::Strong => Style::default().add_modifier(Modifier::BOLD),
        md::Style::Emphasis => Style::default().add_modifier(Modifier::ITALIC),
        md::Style::Code => Style::default().fg(theme.code),
        md::Style::CodeBlock => Style::default().fg(theme.code),
        md::Style::Quote => Style::default().fg(theme.dim),
        md::Style::Bullet => Style::default().fg(theme.accent),
        md::Style::Rule => Style::default().fg(theme.dim),
        md::Style::Link => Style::default()
            .fg(theme.accent)
            .add_modifier(Modifier::UNDERLINED),
        md::Style::Math => Style::default().fg(theme.math),
    }
}

fn pad_to(text: &str, width: usize) -> String {
    let len = text.chars().count();
    if len >= width {
        text.chars().take(width).collect()
    } else {
        format!("{text}{}", " ".repeat(width - len))
    }
}

/// The lines one transcript message renders to. Tool output is skipped unless
/// the reader asked for it.
fn message_lines(
    msg: &Msg,
    width: usize,
    theme: &'static Theme,
    show_output: bool,
) -> Vec<Line<'static>> {
    let mut lines: Vec<Line<'static>> = Vec::new();
    match msg.role {
        Role::User => {
            if msg.images > 0 {
                lines.push(pad_line(
                    &format!(
                        "  ▣ {} image{} attached",
                        msg.images,
                        if msg.images == 1 { "" } else { "s" }
                    ),
                    width,
                    theme.input_bg,
                ));
            }
            let mut first = true;
            for body in msg.text.split('\n') {
                let room = width.saturating_sub(2);
                for wrapped in wrap_text(body, room) {
                    let lead = if first { "› " } else { "  " };
                    first = false;
                    lines.push(pad_line(&format!("{lead}{wrapped}"), width, theme.input_bg));
                }
            }
        }
        Role::System => {
            for (index, body) in msg.text.split('\n').enumerate() {
                let lead = if index == 0 { "• " } else { "  " };
                let room = width.saturating_sub(lead.chars().count());
                for (part, wrapped) in wrap_text(body, room).into_iter().enumerate() {
                    let prefix = if index == 0 && part == 0 { "• " } else { "  " };
                    lines.push(Line::styled(
                        format!("{prefix}{wrapped}"),
                        Style::default().fg(theme.dim),
                    ));
                }
            }
        }
        Role::Tool => {
            // The call itself is the line that matters; its output is noise
            // unless the reader asked for it.
            if msg.kind == crate::proto::WireMsgKind::ToolOutput && !show_output {
                return lines;
            }
            for body in msg.text.split('\n') {
                let room = width.saturating_sub(4);
                for wrapped in wrap_text(body, room) {
                    lines.push(Line::styled(
                        format!("    {wrapped}"),
                        Style::default().fg(theme.dim),
                    ));
                }
            }
        }
        Role::Assistant => {
            if msg.runs.is_empty() {
                for body in msg.text.split('\n') {
                    lines.push(Line::raw(body.to_string()));
                }
            } else {
                for run_line in &msg.runs {
                    for part in wrap_runs(run_line, width) {
                        let mut spans: Vec<Span> = Vec::new();
                        for run in &part {
                            if run.style == md::Style::CodeBlock {
                                spans.push(Span::styled("│ ", Style::default().fg(theme.dim)));
                            }
                            spans.push(Span::styled(
                                run.text.clone(),
                                md_style(run.style, theme),
                            ));
                        }
                        if spans.is_empty() {
                            spans.push(Span::raw(""));
                        }
                        lines.push(Line::from(spans));
                    }
                }
            }
        }
    }
    lines.push(Line::raw(""));
    lines
}

fn pad_line(text: &str, width: usize, bg: Color) -> Line<'static> {
    Line::styled(pad_to(text, width), Style::default().bg(bg))
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

