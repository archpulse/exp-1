//! the threads, messages and clicks the interface keeps.

//! Terminal UI state and rendering for exp-1 agent.
//!
//! Layout follows the OpenAI Codex TUI: a `>_` masthead, submitted prompts as
//! highlighted bands in the transcript, a completion popup or selection picker
//! above the input, a padded input band with a blinking caret, then a model line
//! and a hint line. Two threads exist — the main conversation and a side thread
//! opened with ctrl+/.
use ratatui::layout::{ Rect };
use std::sync::{Arc, Mutex};
use crate::md;
use crate::protocol::{InputItem, Usage};

use super::*;

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
/// What a click on a screen cell does.
#[derive(Clone, PartialEq, Eq)]
pub enum ClickAction {
    /// A row of the open picker, by index into its items.
    PickerRow(usize),
    /// A row of the completion popup, by index into the suggestions.
    Suggestion(usize),
    /// A link in the transcript: open it in the browser.
    Link(String),
    /// The footer hint that switches between the main and side threads.
    SideThread,
    /// A user message: go back to it, dropping everything after it.
    Rewind { index: usize, text: String },
    /// The microphone button: start or stop dictation.
    Mic,
}

/// Where a link sits inside one message's own lines.
#[derive(Clone)]
pub struct LinkSpan {
    /// The line within that message's rendering.
    pub line: usize,
    /// Columns, in characters, inside that line.
    pub start: usize,
    pub end: usize,
    pub url: String,
}

/// A region the last frame drew that a click can act on.
#[derive(Clone)]
pub struct ClickTarget {
    pub area: Rect,
    pub action: ClickAction,
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
    /// What the model is preparing right now (a tool call's arguments), shown
    /// in place of the "thinking" label.
    pub preparing: Option<String>,
    /// The words being dictated right now, if any.
    pub dictating: Option<String>,
    /// When the activity being shown now began: one model request, or one tool.
    /// The turn's own start is kept apart, so the live line counts the request
    /// rather than the whole turn, which made a working agent look stuck.
    pub activity_since: Option<Instant>,
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
    pub(crate) lines: Vec<Vec<Line<'static>>>,
    /// Links per message, in the same order as `lines`, so a click on one can
    /// open it.
    pub(crate) links: Vec<Vec<LinkSpan>>,
    lines_width: usize,
    lines_theme: usize,
    lines_output: bool,
    /// Lines the last frame drew, so a view scrolled up can be held in place
    /// while new ones arrive.
    pub(crate) last_total: usize,
}

impl Thread {
    pub(crate) fn new(kind: ThreadKind, history: Arc<Mutex<Vec<InputItem>>>) -> Self {
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
            preparing: None,
            dictating: None,
            activity_since: None,
            input: Vec::new(),
            cursor: 0,
            input_history: Vec::new(),
            history_index: None,
            selected: 0,
            popup_open: false,
            scroll: 0,
            lines: Vec::new(),
            links: Vec::new(),
            lines_width: 0,
            lines_theme: usize::MAX,
            lines_output: false,
            last_total: 0,
        }
    }

    /// Builds the lines for every message that does not have them yet. The
    /// width, the theme and the output setting are part of the cache key.
    pub(crate) fn bake(&mut self, width: usize, theme_index: usize, show_output: bool, theme: &'static Theme) {
        if self.lines_width != width || self.lines_theme != theme_index || self.lines_output != show_output
        {
            self.lines.clear();
            self.links.clear();
            self.lines_width = width;
            self.lines_theme = theme_index;
            self.lines_output = show_output;
        }
        if self.lines.len() > self.transcript.len() {
            self.lines.truncate(self.transcript.len());
            self.links.truncate(self.transcript.len());
        }
        for msg in &self.transcript[self.lines.len()..] {
            let mut links = Vec::new();
            self.lines
                .push(message_lines(msg, width, theme, show_output, &mut links));
            self.links.push(links);
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
                self.links.truncate(self.transcript.len() - 1);
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

    /// Empties the conversation and the lines built from it. Whoever rebuilds
    /// the transcript has to say so, or the old lines are drawn again from the
    /// cache.
    pub fn clear_transcript(&mut self) {
        self.transcript.clear();
        self.live.clear();
        self.lines.clear();
        self.links.clear();
        self.last_total = 0;
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
