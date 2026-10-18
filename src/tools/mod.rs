//! Tool implementations: read, write, edit, shell, ast.
//!
//! All paths are resolved against the session working directory. Nothing is
//! sandboxed: commands run as the user.

use serde_json::Value;
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex as StdMutex};
use std::time::{Duration, Instant};
use tokio::sync::{Mutex, oneshot};

/// A foreground command still running after this long is promoted to a
/// background task, so the agent is never blocked on something slow. A
/// background task has no clock on it: builds and test suites run for minutes,
/// and the old 120s axe was cutting them off mid-flight.
const FOREGROUND_LIMIT: Duration = Duration::from_secs(60);
/// Hard cap on anything handed back to the model.
const MAX_OUTPUT: usize = 30_000;
/// Lines a `read` returns when the caller gives no `limit`: enough to see the
/// shape of a file, small enough that a few reads do not eat the conversation.
/// The answer says how to continue, so a large file is walked, not swallowed.
const READ_DEFAULT_LINES: usize = 400;
/// Default line cap when the caller passes no `limit`.
const DEFAULT_LINES: usize = 200;
/// Directory-tree entries before we stop descending. The tree is an
/// orientation view, not an inventory: a listing that runs to thousands of
/// lines costs more context than the project is worth.
const TREE_MAX_ENTRIES: usize = 600;
/// Build output and tool caches. Walking them says nothing about the project:
/// in a Rust repo they are 99% of the files (a `target` runs to tens of
/// thousands), and listing them buries the real tree in noise.
const ARTIFACT_DIRS: &[&str] = &[
    "target",
    "node_modules",
    "dist",
    "build",
    "out",
    "__pycache__",
    ".venv",
    "venv",
    ".tox",
    ".next",
    ".mypy_cache",
    ".pytest_cache",
    ".ruff_cache",
    "coverage",
];

/// Not the project at all, so never listed, counted or searched.
fn hidden_dir(name: &str) -> bool {
    name == ".git" || name == ".cache"
}

/// Directories a search never descends into: no source lives there, and their
/// matches are noise that costs the reader a second search.
fn search_skip(name: &str) -> bool {
    hidden_dir(name) || ARTIFACT_DIRS.contains(&name)
}

const AST_CACHE: &str = ".cache/ast-2";
/// Per-file line counts, beside the ast tree.
const COUNT_CACHE: &str = ".cache/count";

pub struct Outcome {
    /// Text handed to the model as the function_call_output.
    pub output: String,
    /// Set when a command was moved to the background, so the caller can watch
    /// for it finishing and tell the agent.
    pub background_id: Option<String>,
}

struct BackgroundTask {
    command: String,
    started: Instant,
    handle: Option<tokio::task::JoinHandle<String>>,
}

/// Output of a finished command, kept so the model can re-read it with a
/// different `limit` after getting it wrong.
#[derive(Clone)]
struct StoredOutput {
    label: String,
    text: String,
}

/// How long the agent waits for an answer before giving up on itself.
const ASK_TIMEOUT: Duration = Duration::from_secs(600);
/// How long a permission prompt waits before the call is refused: long enough
/// to notice it, short enough not to stall a turn for ten minutes.
pub const PERMISSION_TIMEOUT: Duration = Duration::from_secs(120);

/// Exa's hosted MCP search, used when `EXA_API_KEY` is set. Its keyless tier
/// rate-limits after one query, so DuckDuckGo is the default instead.
const EXA_MCP: &str = "https://mcp.exa.ai/mcp?tools=web_search_exa";
/// DuckDuckGo's HTML endpoint: no key, no meaningful rate limit.
const DDG_HTML: &str = "https://html.duckduckgo.com/html/";
/// DuckDuckGo answers a plain client user agent with an empty page.
const BROWSER_UA: &str =
    "Mozilla/5.0 (X11; Linux x86_64) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/124.0 Safari/537.36";
const SEARCH_TIMEOUT: Duration = Duration::from_secs(25);
/// Lines of a fetched page handed over unless the caller says otherwise.
const FETCH_DEFAULT_LIMIT: usize = 400;

/// One question the agent is waiting on.
#[derive(Clone)]
pub struct AskItem {
    /// Short name shown as a tab when several questions are asked together.
    pub label: String,
    pub text: String,
    pub options: Vec<String>,
}

struct AskRequest {
    questions: Vec<AskItem>,
    submit_label: String,
    reply: oneshot::Sender<Vec<String>>,
}

/// What the UI needs to draw the question panel.
#[derive(Clone)]
pub struct AskSnapshot {
    pub questions: Vec<AskItem>,
    pub submit_label: String,
}

/// Hand-off between a blocking `ask_question` call and the event loop.
pub struct AskSlot {
    pending: StdMutex<Option<AskRequest>>,
}

impl AskSlot {
    pub fn new() -> Arc<Self> {
        Arc::new(Self {
            pending: StdMutex::new(None),
        })
    }

    /// Tool side: publish the question, then wait for the UI to answer it.
    pub async fn ask(
        &self,
        questions: Vec<AskItem>,
        submit_label: String,
    ) -> Option<Vec<String>> {
        self.ask_within(questions, submit_label, ASK_TIMEOUT).await
    }

    /// The same with an explicit patience: a permission prompt should not hold
    /// a turn for the ten minutes a real question may take.
    pub async fn ask_within(
        &self,
        questions: Vec<AskItem>,
        submit_label: String,
        patience: Duration,
    ) -> Option<Vec<String>> {
        let (tx, rx) = oneshot::channel();
        let request = AskRequest {
            questions,
            submit_label,
            reply: tx,
        };
        *self.pending.lock().expect("ask slot") = Some(request);
        match tokio::time::timeout(patience, rx).await {
            Ok(Ok(answers)) => Some(answers),
            // Timed out or the slot was replaced; never leave the model hanging.
            _ => None,
        }
    }

    /// UI side: look at the pending question without consuming it.
    pub fn peek(&self) -> Option<AskSnapshot> {
        let guard = self.pending.lock().expect("ask slot");
        guard.as_ref().map(|request| AskSnapshot {
            questions: request.questions.clone(),
            submit_label: request.submit_label.clone(),
        })
    }

    /// UI side: deliver the answers and release the waiting call.
    pub fn answer(&self, answers: Vec<String>) {
        let taken = self.pending.lock().expect("ask slot").take();
        if let Some(request) = taken {
            let _ = request.reply.send(answers);
        }
    }

}

pub struct Registry {
    /// Shared HTTP client, so repeated searches reuse connections.
    http: reqwest::Client,
    background: HashMap<String, BackgroundTask>,
    outputs: HashMap<String, StoredOutput>,
    counter: usize,
}

impl Registry {
    /// The shared HTTP client, for the few callers outside this module.
    pub fn http(&self) -> reqwest::Client {
        self.http.clone()
    }

    pub fn new() -> Self {
        Self {
            http: reqwest::Client::builder()
                .connect_timeout(Duration::from_secs(10))
                .build()
                .unwrap_or_default(),
            background: HashMap::new(),
            outputs: HashMap::new(),
            counter: 0,
        }
    }

    /// True while a background task is still running.
    pub async fn background_running(&self, id: &str) -> bool {
        self.background
            .get(id)
            .and_then(|task| task.handle.as_ref())
            .is_some_and(|handle| !handle.is_finished())
    }

    fn next_id(&mut self, prefix: &str) -> String {
        self.counter += 1;
        format!("{prefix}-{}", self.counter)
    }
}

pub type Shared = Arc<Mutex<Registry>>;

/// One rule from `permissions.json`: what it applies to and what to do.
#[derive(Debug, Clone, serde::Deserialize)]
pub struct Rule {
    /// Tool name, or `*` for any.
    pub action: String,
    /// What the rule matches — a path, a command line, a URL. `*` and `?`
    /// work as wildcards.
    pub resource: String,
    /// `allow`, `ask` or `deny`.
    pub effect: String,
}

#[derive(Debug, Clone, serde::Deserialize, Default)]
pub struct Rules {
    #[serde(default)]
    pub rules: Vec<Rule>,
}

impl Rules {
    /// The first rule that matches, if any. No match means the tool runs, so
    /// the file is a list of exceptions rather than a whitelist.
    pub fn decide(&self, action: &str, resource: &str) -> Option<&Rule> {
        self.rules
            .iter()
            .find(|rule| glob(&rule.action, action) && glob(&rule.resource, resource))
    }
}

/// How much the agent asks before it runs a tool: every tool that changes
/// something (`always ask`, the fresh-install default), nothing (`never ask`),
/// or only `shell` (`ask only commands`). Cycled with `ctrl + shift + a` in the
/// TUI, kept per session, saved in `settings.json` and enforced by the server.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum PermissionMode {
    /// The default: `write`, `edit`, `delete` and `shell` raise the question.
    #[default]
    AlwaysAsk,
    /// Nothing asks; the agent simply does the work.
    NeverAsk,
    /// Only `shell` asks; file writes and edits go through.
    AskOnlyCommands,
}

/// What one call settles on, once `permissions.json` and the permission mode
/// have both been read.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Verdict {
    /// Run it without asking.
    Run,
    /// Raise the permission question.
    Ask,
    /// Refuse it. No mode makes this go away.
    Deny,
}

impl PermissionMode {
    /// The exact wording shown in the footer and in the documentation.
    pub fn label(self) -> &'static str {
        match self {
            Self::AlwaysAsk => "always ask",
            Self::NeverAsk => "never ask",
            Self::AskOnlyCommands => "ask only commands",
        }
    }

    /// The next mode in the order `ctrl + shift + a` walks them.
    pub fn cycle(self) -> Self {
        match self {
            Self::AlwaysAsk => Self::NeverAsk,
            Self::NeverAsk => Self::AskOnlyCommands,
            Self::AskOnlyCommands => Self::AlwaysAsk,
        }
    }

    /// Whether the mode asks for this tool on its own. Read-only tools
    /// (`read`, `grep`, `count`, `ast`, `fetch`, `websearch`) never ask, so
    /// only the tools that change files, the shell, and the scheduler are ever
    /// named here.
    pub fn asks(self, tool: &str) -> bool {
        match self {
            Self::AlwaysAsk => matches!(tool, "write" | "edit" | "delete" | "shell" | "schedule"),
            Self::NeverAsk => false,
            Self::AskOnlyCommands => tool == "shell",
        }
    }

    /// What the mode and a matching rule effect settle on together. A `deny`
    /// refuses in every mode — the mode never loosens it. An explicit `ask`
    /// rule keeps its own question; otherwise the mode decides whether the
    /// tool has to ask by itself.
    pub fn verdict(self, rule_effect: Option<&str>, tool: &str) -> Verdict {
        match rule_effect {
            Some("deny") => Verdict::Deny,
            Some("ask") => Verdict::Ask,
            _ if self.asks(tool) => Verdict::Ask,
            _ => Verdict::Run,
        }
    }
}

/// A small glob: `*` for any run, `?` for one character, everything else
/// literal. Enough for paths and command prefixes without a crate.
fn glob(pattern: &str, text: &str) -> bool {
    let pattern: Vec<char> = pattern.chars().collect();
    let text: Vec<char> = text.chars().collect();
    let (mut p, mut t) = (0usize, 0usize);
    let (mut star, mut mark) = (None, 0usize);
    while t < text.len() {
        if p < pattern.len() && (pattern[p] == '?' || pattern[p] == text[t]) {
            p += 1;
            t += 1;
        } else if p < pattern.len() && pattern[p] == '*' {
            star = Some(p);
            mark = t;
            p += 1;
        } else if let Some(at) = star {
            p = at + 1;
            mark += 1;
            t = mark;
        } else {
            return false;
        }
    }
    while p < pattern.len() && pattern[p] == '*' {
        p += 1;
    }
    p == pattern.len()
}

/// Reads `~/.config/exp-1/permissions.json`, writing an empty list the first
/// time so there is a file to edit.
pub fn load_rules() -> Rules {
    let path = crate::store::permissions_path();
    let Ok(raw) = std::fs::read_to_string(&path) else {
        if let Some(parent) = path.parent() {
            let _ = std::fs::create_dir_all(parent);
        }
        let _ = std::fs::write(&path, "{\n  \"rules\": []\n}\n");
        return Rules::default();
    };
    serde_json::from_str(&raw).unwrap_or_default()
}

/// What a turn changed, so `/undo` can put it back and `/diff` can show it.
///
/// Entries live in memory for the life of the session: a restart forgets them,
/// which is why the diff is a convenience and not a backup.
#[derive(Default)]
pub struct UndoLog {
    /// One entry per turn, oldest first.
    turns: Vec<Vec<(PathBuf, Option<Vec<u8>>)>>,
}

impl UndoLog {
    /// Opens a turn. Anything the tools touch from here belongs to it.
    pub fn begin(&mut self) {
        self.turns.push(Vec::new());
    }

    /// Remembers a file as it was before a change; `None` means it did not
    /// exist, so undoing deletes it again.
    pub fn record(&mut self, path: &Path) {
        let before = std::fs::read(path).ok();
        if self.turns.is_empty() {
            self.turns.push(Vec::new());
        }
        if let Some(turn) = self.turns.last_mut() {
            // One entry per file per turn: the first state is the one to keep.
            if !turn.iter().any(|(known, _)| known == path) {
                turn.push((path.to_path_buf(), before));
            }
        }
    }

    /// How many turns have been opened; a rewind keeps this many.
    pub fn turns(&self) -> usize {
        self.turns.len()
    }

    /// Drops every turn from `keep` on and hands it back for restoring, oldest
    /// first. A rewind to a point in the conversation uses this.
    pub fn rewind_to(&mut self, keep: usize) -> Vec<Vec<(PathBuf, Option<Vec<u8>>)>> {
        if keep >= self.turns.len() {
            return Vec::new();
        }
        self.turns.split_off(keep)
    }

    /// The last turn as it stands, for a diff.
    pub fn last(&self) -> &[(PathBuf, Option<Vec<u8>>)] {
        self.turns.last().map(Vec::as_slice).unwrap_or(&[])
    }
}

/// A unified diff of two texts, or `None` when they read the same. Long files
/// fall back to a plain summary rather than building a table in memory.
pub fn unified_diff(path: &str, before: &str, after: &str) -> Option<String> {
    let a: Vec<&str> = before.lines().collect();
    let b: Vec<&str> = after.lines().collect();
    if a == b {
        return None;
    }
    if a.len().saturating_mul(b.len()) > 4_000_000 {
        return Some(format!(
            "--- {path}\n+++ {path}\n  ({} lines before, {} after; too large to diff line by line)",
            a.len(),
            b.len()
        ));
    }
    let mut lcs = vec![vec![0usize; b.len() + 1]; a.len() + 1];
    for i in (0..a.len()).rev() {
        for j in (0..b.len()).rev() {
            lcs[i][j] = if a[i] == b[j] {
                lcs[i + 1][j + 1] + 1
            } else {
                lcs[i + 1][j].max(lcs[i][j + 1])
            };
        }
    }
    let mut body = String::new();
    let (mut i, mut j) = (0usize, 0usize);
    while i < a.len() && j < b.len() {
        if a[i] == b[j] {
            body.push_str(&format!("  {}\n", a[i]));
            i += 1;
            j += 1;
        } else if lcs[i + 1][j] >= lcs[i][j + 1] {
            body.push_str(&format!("- {}\n", a[i]));
            i += 1;
        } else {
            body.push_str(&format!("+ {}\n", b[j]));
            j += 1;
        }
    }
    while i < a.len() {
        body.push_str(&format!("- {}\n", a[i]));
        i += 1;
    }
    while j < b.len() {
        body.push_str(&format!("+ {}\n", b[j]));
        j += 1;
    }
    Some(format!("--- {path}\n+++ {path}\n{body}"))
}

fn err(message: impl Into<String>) -> Outcome {
    let message = message.into();
    Outcome {
        output: message,
        background_id: None,
    }
}

/// A byte offset moved back to a character boundary, so truncating text never
/// panics in the middle of a multi-byte character.
fn cut(text: &str, at: usize) -> &str {
    let mut end = at.min(text.len());
    while end > 0 && !text.is_char_boundary(end) {
        end -= 1;
    }
    &text[..end]
}

/// Applies a line cap and a byte cap, in that order.
fn clamp(text: &str, limit: Option<usize>) -> String {
    let limit = limit.unwrap_or(DEFAULT_LINES);
    let mut lines: Vec<&str> = text.lines().take(limit).collect();
    let truncated_lines = text.lines().count() > lines.len();
    let mut out = lines.drain(..).collect::<Vec<_>>().join("\n");
    if truncated_lines {
        out.push_str(&format!("\n... truncated at {limit} lines"));
    }
    if out.len() > MAX_OUTPUT {
        // Keep both ends and drop the middle: a build log opens with what ran
        // and ends with the error, and the middle is the part nobody reads. A
        // cut of the tail alone would hide exactly the lines that matter.
        let marker_room = 160;
        let head = cut(&out, MAX_OUTPUT / 3).len();
        let tail_len = MAX_OUTPUT.saturating_sub(head + marker_room);
        let tail_start = cut(&out, out.len() - tail_len).len();
        let omitted = tail_start - head;
        let mut kept = String::with_capacity(MAX_OUTPUT);
        kept.push_str(&out[..head]);
        kept.push_str(&format!(
            "\n... [exp-1 agent omitted {omitted} characters here; re-read with output_id and a larger limit] ...\n"
        ));
        kept.push_str(&out[tail_start..]);
        out = kept;
    }
    out
}

/// Same as `clamp`, but the `exit_code:` header never eats into the line budget.
fn clamp_output(text: &str, limit: Option<usize>) -> String {
    if let Some(rest) = text.strip_prefix("exit_code: ")
        && let Some((code, body)) = rest.split_once('\n')
    {
        return format!("exit_code: {code}\n{}", clamp(body, limit));
    }
    clamp(text, limit)
}

fn resolve(cwd: &Path, raw: &str) -> PathBuf {
    let path = Path::new(raw);
    if path.is_absolute() {
        path.to_path_buf()
    } else {
        cwd.join(path)
    }
}

// ---------------------------------------------------------------- ask

fn read_ask_item(value: &Value) -> Option<AskItem> {
    let text = value.get("question").and_then(Value::as_str)?.trim().to_string();
    if text.is_empty() {
        return None;
    }
    let label = value
        .get("label")
        .and_then(Value::as_str)
        .unwrap_or("Question")
        .to_string();
    let options = value
        .get("options")
        .and_then(Value::as_array)
        .map(|items| {
            items
                .iter()
                .filter_map(Value::as_str)
                .map(str::trim)
                .filter(|s| !s.is_empty())
                .map(str::to_string)
                .collect()
        })
        .unwrap_or_default();
    Some(AskItem { label, text, options })
}

pub async fn ask_question(slot: &Arc<AskSlot>, args: &Value) -> Outcome {
    let questions: Vec<AskItem> = if let Some(list) = args.get("questions").and_then(Value::as_array)
    {
        list.iter().filter_map(read_ask_item).collect()
    } else if let Some(item) = read_ask_item(args) {
        vec![item]
    } else {
        return err("ask_question: 'question' is required");
    };
    if questions.is_empty() {
        return err("ask_question: 'question' is required");
    }
    let submit_label = args
        .get("submit")
        .and_then(Value::as_str)
        .unwrap_or("Submit")
        .to_string();

    let answers = slot.ask(questions, submit_label).await.unwrap_or_default();

    if answers.iter().all(|a| a.trim().is_empty()) {
        return Outcome {
            output: "The user dismissed the question without answering.".to_string(),
            background_id: None,
        };
    }
    let mut rendered = String::new();
    for (index, answer) in answers.iter().enumerate() {
        if answer.trim().is_empty() {
            continue;
        }
        if rendered.is_empty() {
            rendered.push_str("The user answered:\n");
        }
        if answers.len() > 1 {
            rendered.push_str(&format!("{}. {answer}\n", index + 1));
        } else {
            rendered.push_str(answer);
            rendered.push('\n');
        }
    }
    Outcome {
        output: rendered,
        background_id: None,
    }
}

// ---------------------------------------------------------------- dispatch

pub async fn call(
    cwd: &Path,
    registry: &Shared,
    undo: &Arc<StdMutex<UndoLog>>,
    ask: &Arc<AskSlot>,
    name: &str,
    arguments: &str,
) -> Outcome {
    let args: Value = match serde_json::from_str(arguments) {
        Ok(args) => args,
        Err(e) => return err(format!("{name}: invalid JSON arguments: {e}")),
    };
    match name {
        "read" => read(cwd, &args).await,
        "write" => write(cwd, undo, &args).await,
        "delete" => delete(cwd, undo, &args).await,
        "edit" => edit(cwd, undo, &args).await,
        "shell" => shell(cwd, registry, &args).await,
        "ast" => ast(cwd, &args).await,
        "count" => count(cwd, &args).await,
        "grep" => grep(cwd, &args).await,
        "fetch" => {
            let http = { registry.lock().await.http.clone() };
            fetch(&http, &args).await
        }
        "websearch" => {
            let http = { registry.lock().await.http.clone() };
            websearch(&http, &args).await
        }
        "ask_question" => ask_question(ask, &args).await,
        other => err(format!("unknown tool '{other}'")),
    }
}

mod files;
mod search;
mod shell;
mod specs;
mod web;

pub(crate) use self::{files::*, search::*, shell::*, specs::*, web::*};

#[cfg(test)]
mod tests;
