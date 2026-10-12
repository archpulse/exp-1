//! Tool implementations: read, write, edit, shell, ast.
//!
//! All paths are resolved against the session working directory. Nothing is
//! sandboxed: commands run as the user.

use serde_json::{Value, json};
use std::collections::HashMap;
use std::io::Read;
use std::path::{Path, PathBuf};
use std::process::Stdio;
use std::sync::{Arc, Mutex as StdMutex};
use std::time::{Duration, Instant};
use tokio::process::Command;
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
/// Directory-tree entries before we stop descending.
const TREE_MAX_ENTRIES: usize = 2000;
/// Directories never worth walking.
const TREE_SKIP: &[&str] = &[".git"];
const AST_CACHE: &str = ".cache/ast";
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

    /// Takes the last turn, for restoring.
    pub fn take_last(&mut self) -> Vec<(PathBuf, Option<Vec<u8>>)> {
        self.turns.pop().unwrap_or_default()
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

// ---------------------------------------------------------------- read

pub async fn read(cwd: &Path, args: &Value) -> Outcome {
    let Some(raw) = args.get("path").and_then(Value::as_str) else {
        return err("read: 'path' is required");
    };
    let limit = args.get("limit").and_then(Value::as_u64).map(|v| v as usize);
    let pos = args.get("pos").and_then(Value::as_u64).map(|v| v.max(1) as usize);
    let path = resolve(cwd, raw);

    let mut file = match std::fs::File::open(&path) {
        Ok(file) => file,
        Err(e) => return err(format!("read: {}: {e}", path.display())),
    };
    let mut text = String::new();
    if let Err(e) = file.read_to_string(&mut text) {
        return err(format!("read: {}: not valid UTF-8 text ({e})", path.display()));
    }

    let total = text.lines().count();
    let start = pos.unwrap_or(1);
    if start > total.max(1) {
        return err(format!(
            "read: {raw}: line {start} is past the end of the file ({total} lines)"
        ));
    }

    // No `limit` no longer means "the whole file": a 3000-line source file is
    // tens of thousands of tokens, and the model rarely needs all of it at once.
    let wanted = limit.unwrap_or(READ_DEFAULT_LINES);
    let taken: Vec<String> = text
        .lines()
        .skip(start - 1)
        .take(wanted)
        .enumerate()
        .map(|(offset, line)| format!("{:>5} | {}", start + offset, line))
        .collect();

    let header = format!("{} ({} lines)", raw, total);
    let mut body = taken.join("\n");
    if body.len() > MAX_OUTPUT {
        let end = cut(&body, MAX_OUTPUT).len();
        body.truncate(end);
        body.push_str("\n... [output truncated by exp-1 agent; read again with pos]");
    }
    let mut output = format!("{header}\n{body}");
    if start - 1 + taken.len() < total {
        output.push_str(&format!(
            "\n... showing {l} of {total} lines from {start}; call read again with a larger limit or pos to continue",
            l = taken.len()
        ));
    }
    Outcome {
        output,
        background_id: None,
    }
}

// ---------------------------------------------------------------- write

/// Removes one file. A directory is refused rather than emptied: taking a tree
/// apart is a shell job, and the user should see the whole command.
pub async fn delete(cwd: &Path, undo: &Arc<StdMutex<UndoLog>>, args: &Value) -> Outcome {
    let Some(raw) = args.get("path").and_then(Value::as_str) else {
        return err("delete: 'path' is required");
    };
    let path = resolve(cwd, raw);
    // `symlink_metadata` so a link to a directory is removed as the link it is.
    let metadata = match std::fs::symlink_metadata(&path) {
        Ok(metadata) => metadata,
        Err(e) => return err(format!("delete: {}: {e}", path.display())),
    };
    if metadata.is_dir() {
        return err(format!(
            "delete: {} is a directory — remove it with shell, so the whole command is visible",
            path.display()
        ));
    }
    if let Ok(mut log) = undo.lock() {
        log.record(&path);
    }
    if let Err(e) = std::fs::remove_file(&path) {
        return err(format!("delete: {}: {e}", path.display()));
    }
    Outcome {
        output: format!("deleted {}", path.display()),
        background_id: None,
    }
}

pub async fn write(cwd: &Path, undo: &Arc<StdMutex<UndoLog>>, args: &Value) -> Outcome {
    let Some(raw) = args.get("path").and_then(Value::as_str) else {
        return err("write: 'path' is required");
    };
    let Some(content) = args.get("content").and_then(Value::as_str) else {
        return err("write: 'content' is required");
    };
    let path = resolve(cwd, raw);
    if let Some(parent) = path.parent() {
        if let Err(e) = std::fs::create_dir_all(parent) {
            return err(format!("write: {}: {e}", parent.display()));
        }
    }
    let existed = path.exists();
    // Recorded even when the file is new: undoing then means deleting it, and
    // `record` remembers that it did not exist.
    if let Ok(mut log) = undo.lock() {
        log.record(&path);
    }
    if let Err(e) = std::fs::write(&path, content) {
        return err(format!("write: {}: {e}", path.display()));
    }
    Outcome {
        output: format!(
            "{} {raw} ({} lines, {} bytes)",
            if existed { "overwrote" } else { "created" },
            content.lines().count(),
            content.len()
        ),
        background_id: None,
    }
}

// ---------------------------------------------------------------- edit

/// The replacements an `edit` call asks for: one `old`/`new` pair, or an
/// `edits` array applied in order.
fn edit_pairs(args: &Value) -> Result<Vec<(String, String)>, String> {
    if let Some(list) = args.get("edits") {
        let Some(list) = list.as_array() else {
            return Err("edit: 'edits' must be an array of {\"old\", \"new\"} objects".to_string());
        };
        if list.is_empty() {
            return Err("edit: 'edits' is empty".to_string());
        }
        let mut pairs = Vec::with_capacity(list.len());
        for (index, item) in list.iter().enumerate() {
            let old = item.get("old").and_then(Value::as_str).unwrap_or_default();
            let new = item.get("new").and_then(Value::as_str).unwrap_or_default();
            if old.is_empty() {
                return Err(format!("edit: edits[{index}].old is required"));
            }
            if old == new {
                return Err(format!("edit: edits[{index}] has identical 'old' and 'new'"));
            }
            pairs.push((old.to_string(), new.to_string()));
        }
        return Ok(pairs);
    }
    let Some(old) = args.get("old").and_then(Value::as_str) else {
        return Err("edit: 'old' is required (or an 'edits' array)".to_string());
    };
    let Some(new) = args.get("new").and_then(Value::as_str) else {
        return Err("edit: 'new' is required".to_string());
    };
    if old == new {
        return Err("edit: 'old' and 'new' are identical".to_string());
    }
    Ok(vec![(old.to_string(), new.to_string())])
}

/// When `old` is not in the file, the lines carrying its first non-empty line
/// are what the model meant: the mismatch is nearly always whitespace, and
/// showing the real text saves a full re-read.
fn missing_hint(raw: &str, text: &str, old: &str) -> String {
    let needle = old
        .lines()
        .find(|line| !line.trim().is_empty())
        .unwrap_or("")
        .trim()
        .to_string();
    let mut hint = format!("edit: 'old' not found in {raw}");
    if needle.chars().count() < 4 {
        hint.push_str("; read the file again and copy the exact text");
        return hint;
    }
    let near: Vec<String> = text
        .lines()
        .enumerate()
        .filter(|(_, line)| line.contains(needle.as_str()))
        .take(3)
        .map(|(index, line)| format!("{:>5} | {}", index + 1, clip_line_to(line, 120)))
        .collect();
    if near.is_empty() {
        hint.push_str("; read the file again and copy the exact text");
    } else {
        hint.push_str(&format!(
            "; the closest lines are below — the whitespace has to match exactly:\n{}",
            near.join("\n")
        ));
    }
    hint
}

/// When `old` matches more than once, say where, so the model can widen the
/// snippet without reading the file back.
fn ambiguous_hint(raw: &str, text: &str, old: &str, hits: usize) -> String {
    let places: Vec<String> = text
        .match_indices(old)
        .take(5)
        .map(|(at, _)| (text[..at].lines().count() + 1).to_string())
        .collect();
    format!(
        "edit: 'old' matches {hits} times in {raw} (lines {}); include more surrounding context to make it unique",
        places.join(", ")
    )
}

pub async fn edit(cwd: &Path, undo: &Arc<StdMutex<UndoLog>>, args: &Value) -> Outcome {
    let Some(raw) = args.get("path").and_then(Value::as_str) else {
        return err("edit: 'path' is required");
    };
    let pairs = match edit_pairs(args) {
        Ok(pairs) => pairs,
        Err(message) => return err(message),
    };
    let path = resolve(cwd, raw);
    let text = match std::fs::read_to_string(&path) {
        Ok(text) => text,
        Err(e) => return err(format!("edit: {}: {e}", path.display())),
    };

    // Every replacement is checked against the text the previous one produced,
    // and nothing is written until all of them have passed: a call that fails
    // halfway leaves the file exactly as it was.
    let mut updated = text.clone();
    for (old, new) in &pairs {
        let hits = updated.matches(old.as_str()).count();
        if hits == 0 {
            return err(missing_hint(raw, &updated, old));
        }
        if hits > 1 {
            return err(ambiguous_hint(raw, &updated, old, hits));
        }
        updated = updated.replacen(old.as_str(), new, 1);
    }
    if let Ok(mut log) = undo.lock() {
        log.record(&path);
    }
    if let Err(e) = std::fs::write(&path, &updated) {
        return err(format!("edit: {}: {e}", path.display()));
    }

    let removed: usize = pairs.iter().map(|(old, _)| old.lines().count()).sum();
    let added: usize = pairs.iter().map(|(_, new)| new.lines().count()).sum();
    let lines = updated.lines().count();
    if pairs.len() == 1 {
        let (old, new) = &pairs[0];
        // A short window around the change, so the model can see what it did
        // instead of reading the file back — those re-reads cost far more than
        // the edit itself, and they were most of what filled the conversation.
        let at = text
            .find(old.as_str())
            .map(|offset| text[..offset].lines().count() + 1)
            .unwrap_or(1);
        let from = at.saturating_sub(2).max(1);
        let window: Vec<String> = updated
            .lines()
            .skip(from - 1)
            .take(new.lines().count() + 4)
            .enumerate()
            .map(|(index, body)| format!("{:>5} | {body}", from + index))
            .collect();
        return Outcome {
            output: format!(
                "edited {raw}: replaced {removed} line(s) with {added}; file is now {lines} lines\n{}",
                window.join("\n")
            ),
            background_id: None,
        };
    }
    let changes: Vec<String> = pairs
        .iter()
        .map(|(_, new)| format!("  - {}", clip_line_to(new.lines().next().unwrap_or(""), 100)))
        .collect();
    Outcome {
        output: format!(
            "edited {raw}: {} changes, {removed} line(s) replaced with {added}; file is now {lines} lines\n{}",
            pairs.len(),
            changes.join("\n")
        ),
        background_id: None,
    }
}

// ---------------------------------------------------------------- shell

fn argv_from(args: &Value) -> Result<Vec<String>, String> {
    if let Some(array) = args.get("command").and_then(Value::as_array) {
        let argv: Vec<String> = array
            .iter()
            .filter_map(Value::as_str)
            .map(str::to_string)
            .collect();
        if !argv.is_empty() {
            return Ok(argv);
        }
    }
    if let Some(cmd) = args.get("cmd").and_then(Value::as_str)
        && !cmd.trim().is_empty()
    {
        return Ok(vec!["/bin/sh".into(), "-c".into(), cmd.to_string()]);
    }
    Err("shell: 'command' is required".to_string())
}

/// Re-reads a previous command's output, or polls a background task.
pub async fn read_output(registry: &Shared, args: &Value) -> Outcome {
    let Some(id) = args.get("output_id").and_then(Value::as_str) else {
        return err("shell: 'output_id' is required");
    };
    let limit = args.get("limit").and_then(Value::as_u64).map(|v| v as usize);

    let stored = { registry.lock().await.outputs.get(id).cloned() };
    if let Some(stored) = stored {
        return Outcome {
            output: format!(
                "re-read of '{}' with limit {}:\n{}",
                stored.label,
                limit.unwrap_or(DEFAULT_LINES),
                clamp_output(&stored.text, limit)
            ),

            background_id: None,

        };
    }

    // Otherwise it must be a background task.
    let mut guard = registry.lock().await;
    let Some(task) = guard.background.get_mut(id) else {
        let known: Vec<String> = guard
            .outputs
            .keys()
            .chain(guard.background.keys())
            .cloned()
            .collect();
        return err(format!(
            "shell: unknown output_id '{id}'{}\nstart a command first, then poll with the id the \
             shell tool returned",
            if known.is_empty() {
                "; no stored output yet".to_string()
            } else {
                format!("; known ids: {}", known.join(", "))
            }
        ));
    };
    let done = task.handle.as_ref().is_some_and(|h| h.is_finished());
    if !done {
        return Outcome {
            output: format!(
                "'{}' is still running ({}s elapsed); call shell again with output_id '{id}' to check again",
                task.command,
                task.started.elapsed().as_secs()
            ),

            background_id: None,

        };
    }
    let command = task.command.clone();
    let handle = task.handle.take().expect("finished");
    drop(guard);

    let output = match handle.await {
        Ok(output) => output,
        Err(e) if e.is_cancelled() => "exit_code: killed\ncommand was stopped".to_string(),
        Err(_) => "exit_code: failed\ntask panicked".to_string(),
    };

    let mut guard = registry.lock().await;
    guard.background.remove(id);
    guard.outputs.insert(
        id.to_string(),
        StoredOutput {
            label: command,
            text: output.clone(),
        },
    );
    Outcome {
        output: clamp_output(&output, limit),
        background_id: None,
    }
}

/// Removes ANSI escape sequences. `TERM=dumb` stops most tools from colouring
/// their output, but not all of them, and escape bytes are pure noise in a
/// model's context.
fn strip_ansi(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    let mut chars = text.chars().peekable();
    while let Some(ch) = chars.next() {
        if ch != '\u{1b}' {
            out.push(ch);
            continue;
        }
        match chars.peek().copied() {
            // CSI: ESC [ … a byte in 0x40..=0x7e ends it.
            Some('[') => {
                chars.next();
                for next in chars.by_ref() {
                    if ('\u{40}'..='\u{7e}').contains(&next) {
                        break;
                    }
                }
            }
            // OSC: ESC ] … ends at BEL or ESC \.
            Some(']') => {
                chars.next();
                while let Some(next) = chars.next() {
                    if next == '\u{7}' {
                        break;
                    }
                    if next == '\u{1b}' && chars.peek() == Some(&'\\') {
                        chars.next();
                        break;
                    }
                }
            }
            // Any other escape is two characters.
            Some(_) => {
                chars.next();
            }
            None => {}
        }
    }
    out
}

fn render_output(output: &std::process::Output) -> String {
    let mut body = String::new();
    let stdout = strip_ansi(&String::from_utf8_lossy(&output.stdout));
    let stderr = strip_ansi(&String::from_utf8_lossy(&output.stderr));
    if !stdout.is_empty() {
        body.push_str(stdout.trim_end());
    }
    if !stderr.is_empty() {
        if !body.is_empty() {
            body.push('\n');
        }
        body.push_str("[stderr]\n");
        body.push_str(stderr.trim_end());
    }
    if body.is_empty() {
        body.push_str("(no output)");
    }
    format!("exit_code: {}\n{body}", output.status.code().unwrap_or(-1))
}

pub async fn shell(cwd: &Path, registry: &Shared, args: &Value) -> Outcome {
    // Only an explicit, non-empty id means "read previous output"; a model that
    // sends an empty one alongside a command means to run that command.
    if args
        .get("output_id")
        .and_then(Value::as_str)
        .is_some_and(|id| !id.trim().is_empty())
    {
        return read_output(registry, args).await;
    }
    let argv = match argv_from(args) {
        Ok(argv) => argv,
        Err(message) => return err(message),
    };
    let limit = args.get("limit").and_then(Value::as_u64).map(|v| v as usize);
    let background = args
        .get("background")
        .and_then(Value::as_bool)
        .unwrap_or(false);
    let label = argv.join(" ");

    let mut command = Command::new(&argv[0]);
    command
        .args(&argv[1..])
        .current_dir(cwd)
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    command.env("TERM", "dumb");
    // Dropping the task — the turn being torn down, the server shutting down —
    // stops the process instead of leaving it running with nobody listening.
    command.kill_on_drop(true);

    if background {
        // The agent keeps working; the model polls this id when it needs the result.
        let id = {
            let mut guard = registry.lock().await;
            guard.next_id("bg")
        };
        let handle = tokio::spawn(async move {
            match command.output().await {
                Err(e) => format!("exit_code: spawn_failed\n{e}"),
                Ok(output) => render_output(&output),
            }
        });
        let mut guard = registry.lock().await;
        guard.background.insert(
            id.clone(),
            BackgroundTask {
                command: label.clone(),
                started: Instant::now(),
                handle: Some(handle),
            },
        );
        return Outcome {
            output: format!(
                "started in background as '{id}'\ncall shell again with {{\"output_id\": \"{id}\"}} to poll it"
            ),
            // The watcher needs this to report the finish back to the agent.
            background_id: Some(id),
        };
    }

    // Foreground: the agent waits, but only for so long. A command still
    // running after a minute is handed to the background so nothing blocks.
    let child = match command.spawn() {
        Ok(child) => child,
        Err(e) => return err(format!("shell: could not start '{label}': {e}")),
    };
    let mut waiter = Box::pin(async move { child.wait_with_output().await });

    match tokio::time::timeout(FOREGROUND_LIMIT, &mut waiter).await {
        Ok(Ok(output)) => {
            let text = render_output(&output);
            let id = {
                let mut guard = registry.lock().await;
                let id = guard.next_id("out");
                guard.outputs.insert(
                    id.clone(),
                    StoredOutput {
                        label: label.clone(),
                        text: text.clone(),
                    },
                );
                id
            };
            Outcome {
                output: format!(
                    "{}\noutput_id: {id} — re-read with a different limit if this was cut short",
                    clamp_output(&text, limit)
                ),
                background_id: None,
            }
        }
        Ok(Err(e)) => err(format!("shell: '{label}' failed: {e}")),
        Err(_) => {
            // Promote the still-running process instead of killing it.
            let id = {
                let mut guard = registry.lock().await;
                let id = guard.next_id("bg");
                let handle = tokio::spawn(async move {
                    match waiter.await {
                        Err(e) => format!("exit_code: spawn_failed\n{e}"),
                        Ok(output) => render_output(&output),
                    }
                });
                guard.background.insert(
                    id.clone(),
                    BackgroundTask {
                        command: label.clone(),
                        started: Instant::now(),
                        handle: Some(handle),
                    },
                );
                id
            };
            Outcome {
                output: format!(
                    "'{label}' is still running after {}s, so it was moved to the background as \
                     '{id}'. Carry on; you will be told when it finishes, or poll it with \
                     {{\"output_id\": \"{id}\"}}.",
                    FOREGROUND_LIMIT.as_secs()
                ),
                background_id: Some(id),
            }
        }
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

// ---------------------------------------------------------------- websearch

/// Percent-encodes just enough for an API key in a query string.
fn encode_query_value(value: &str) -> String {
    let mut out = String::with_capacity(value.len());
    for byte in value.bytes() {
        match byte {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'_' | b'.' | b'~' => {
                out.push(byte as char)
            }
            other => out.push_str(&format!("%{other:02X}")),
        }
    }
    out
}

fn exa_endpoint() -> String {
    match std::env::var("EXA_API_KEY") {
        Ok(key) if !key.trim().is_empty() => {
            format!("{EXA_MCP}?exaApiKey={}", encode_query_value(key.trim()))
        }
        _ => EXA_MCP.to_string(),
    }
}

/// Pulls the text out of one JSON-RPC payload.
fn from_payload(payload: &str) -> Option<Result<String, String>> {
    if !payload.starts_with('{') {
        return None;
    }
    let value: Value = serde_json::from_str(payload).ok()?;
    if let Some(error) = value.get("error") {
        let message = error
            .get("message")
            .and_then(Value::as_str)
            .unwrap_or("search failed");
        return Some(Err(message.to_string()));
    }
    let text = value
        .get("result")?
        .get("content")?
        .as_array()?
        .iter()
        .find_map(|item| item.get("text").and_then(Value::as_str))
        .map(str::to_string)?;
    Some(Ok(text))
}

/// The MCP server answers either as plain JSON or as an SSE stream.
fn extract_search_text(body: &str) -> Option<Result<String, String>> {
    if let Some(found) = from_payload(body.trim()) {
        return Some(found);
    }
    for line in body.lines() {
        if let Some(rest) = line.trim().strip_prefix("data: ")
            && let Some(found) = from_payload(rest.trim())
        {
            return Some(found);
        }
    }
    None
}

/// HTML to readable text: scripts and styles dropped, tags removed, entities
/// unescaped, whitespace collapsed, so the result reads like an article.
fn html_to_text(html: &str) -> String {
    let mut out = String::with_capacity(html.len() / 2);
    let mut index = 0usize;
    while index < html.len() {
        if html.as_bytes()[index] == b'<' {
            let rest = &html[index..];
            let head = rest[..rest.len().min(64)].to_ascii_lowercase();
            let lower = rest.to_ascii_lowercase();
            let skip = if head.starts_with("<script") {
                lower.find("</script>").map(|at| at + 9)
            } else if head.starts_with("<style") {
                lower.find("</style>").map(|at| at + 8)
            } else {
                None
            };
            if let Some(skip) = skip {
                index += skip;
                continue;
            }
            match rest.find('>') {
                Some(at) => index += at + 1,
                None => break,
            }
            out.push(' ');
            continue;
        }
        let ch = html[index..].chars().next().unwrap_or(' ');
        out.push(ch);
        index += ch.len_utf8();
    }
    let unescaped = unescape_html(&out);
    let mut lines: Vec<String> = Vec::new();
    for line in unescaped.lines() {
        let trimmed = line.split_whitespace().collect::<Vec<_>>().join(" ");
        if trimmed.is_empty() {
            if lines.last().is_some_and(|last| !last.is_empty()) {
                lines.push(String::new());
            }
        } else {
            lines.push(trimmed);
        }
    }
    lines.join("\n").trim().to_string()
}

/// Reads one page as text, paged the way `read` pages a file: `limit` is a
/// maximum number of lines and `pos` is the 1-based line to start from, so a
/// long document is walked through instead of flooding the conversation.
pub async fn fetch(http: &reqwest::Client, args: &Value) -> Outcome {
    let Some(url) = args.get("url").and_then(Value::as_str) else {
        return err("fetch: 'url' is required");
    };
    if !url.starts_with("http://") && !url.starts_with("https://") {
        return err(format!("fetch: '{url}' is not an http(s) URL"));
    }
    let limit = args
        .get("limit")
        .and_then(Value::as_u64)
        .unwrap_or(FETCH_DEFAULT_LIMIT as u64)
        .clamp(1, 5_000) as usize;
    let pos = args.get("pos").and_then(Value::as_u64).unwrap_or(1).max(1) as usize;

    let request = http
        .get(url)
        .header("User-Agent", BROWSER_UA)
        .header("Accept", "text/html,application/xhtml+xml,text/plain;q=0.9,*/*;q=0.8");
    let response = match tokio::time::timeout(SEARCH_TIMEOUT, request.send()).await {
        Err(_) => return err(format!("fetch: timed out after {}s", SEARCH_TIMEOUT.as_secs())),
        Ok(Err(e)) => return err(format!("fetch: request failed: {e}")),
        Ok(Ok(response)) => response,
    };
    let status = response.status();
    let body = response.text().await.unwrap_or_default();
    if !status.is_success() {
        return err(format!(
            "fetch: HTTP {status}: {}",
            body.chars().take(200).collect::<String>()
        ));
    }

    let text = if body.trim_start().starts_with('<') {
        html_to_text(&body)
    } else {
        body.trim().to_string()
    };
    let lines: Vec<&str> = text.lines().collect();
    let total = lines.len();
    if total == 0 {
        return err(format!("fetch: {url} returned nothing readable"));
    }
    if pos > total {
        return err(format!(
            "fetch: {url}: line {pos} is past the end of the page ({total} lines)"
        ));
    }
    let taken: Vec<&str> = lines.iter().skip(pos - 1).take(limit).copied().collect();
    let mut body = taken.join("\n");
    if body.len() > MAX_OUTPUT {
        let end = cut(&body, MAX_OUTPUT).len();
        body.truncate(end);
        body.push_str("\n... [output truncated by exp-1 agent; fetch again with a further pos]");
    }
    let mut output = format!("{url} ({total} lines of text)\n\n{body}");
    if pos - 1 + taken.len() < total {
        output.push_str(&format!(
            "\n\n… [showing {} of {total} lines from {pos}; call fetch again with a larger limit or a further pos to continue]",
            taken.len()
        ));
    }
    Outcome {
        output,
        background_id: None,
    }
}

pub async fn websearch(http: &reqwest::Client, args: &Value) -> Outcome {
    let Some(query) = args.get("query").and_then(Value::as_str) else {
        return err("websearch: 'query' is required");
    };
    if query.trim().is_empty() {
        return err("websearch: 'query' is empty");
    }
    let limit = args
        .get("numResults")
        .and_then(Value::as_u64)
        .unwrap_or(8)
        .clamp(1, 20) as usize;
    let query = query.trim();

    // Exa first, because it returns page content rather than snippets. Its
    // keyless tier is shared per machine and does rate-limit, so DuckDuckGo
    // answers when it does; when DuckDuckGo blocks the request instead, the
    // two failures are reported together rather than one hiding the other.
    let mut failures: Vec<String> = Vec::new();
    match exa_search(http, query, args).await {
        Ok(text) => {
            return Outcome {
                output: text,
                background_id: None,
            };
        }
        Err(why) => failures.push(why),
    }
    match ddg_search(http, query, limit).await {
        Ok(text) => {
            return Outcome {
                output: text,
                background_id: None,
            };
        }
        Err(why) => failures.push(why),
    }
    err(format!(
        "websearch: no backend answered — {}. Set EXA_API_KEY to search under your own plan.",
        failures.join("; ")
    ))
}

/// Keyless search over DuckDuckGo's HTML page, parsed by hand: the markup is
/// a flat list of `result__a` links with a `result__snippet` beside each.
async fn ddg_search(http: &reqwest::Client, query: &str, limit: usize) -> Result<String, String> {
    let request = http
        .post(DDG_HTML)
        .header("User-Agent", BROWSER_UA)
        .header("Accept", "text/html,application/xhtml+xml")
        .form(&[("q", query), ("kl", "wt-wt")]);
    let response = match tokio::time::timeout(SEARCH_TIMEOUT, request.send()).await {
        Err(_) => return Err(format!("duckduckgo: timed out after {}s", SEARCH_TIMEOUT.as_secs())),
        Ok(Err(e)) => return Err(format!("duckduckgo: request failed: {e}")),
        Ok(Ok(response)) => response,
    };
    let status = response.status();
    let body = response.text().await.unwrap_or_default();
    if !status.is_success() {
        return Err(format!("duckduckgo: HTTP {status}"));
    }
    let results = parse_ddg(&body, limit);
    if results.is_empty() {
        return Err("duckduckgo: no results (blocked or rate limited)".to_string());
    }
    Ok(results.join("\n\n"))
}

/// Pulls titles, links and snippets out of the DuckDuckGo result page.
fn parse_ddg(html: &str, limit: usize) -> Vec<String> {
    let mut out: Vec<String> = Vec::new();
    let mut rest = html;
    while out.len() < limit {
        let Some(at) = rest.find("result__a") else {
            break;
        };
        rest = &rest[at..];
        let Some(href) = rest.find("href=\"") else {
            break;
        };
        let after = &rest[href + 6..];
        let Some(end) = after.find('"') else {
            break;
        };
        let url = decode_ddg_url(&after[..end]);
        let Some(gt) = after[end..].find('>') else {
            break;
        };
        let title_part = &after[end + gt + 1..];
        let Some(lt) = title_part.find("</a>") else {
            break;
        };
        let title = unescape_html(&strip_tags(&title_part[..lt]));
        rest = &title_part[lt..];
        let snippet = rest
            .find("result__snippet")
            .and_then(|at| {
                let after = &rest[at..];
                let gt = after.find('>')?;
                let text = &after[gt + 1..];
                let lt = text.find("</a>")?;
                Some(unescape_html(&strip_tags(&text[..lt])))
            })
            .unwrap_or_default();
        let number = out.len() + 1;
        if title.is_empty() {
            continue;
        }
        out.push(format!("{number}. {title}\n   {url}\n   {snippet}"));
    }
    out
}

/// Drops the inline tags DuckDuckGo bolds query words with.
fn strip_tags(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    let mut inside = false;
    for ch in text.chars() {
        match ch {
            '<' => inside = true,
            '>' => inside = false,
            _ if !inside => out.push(ch),
            _ => {}
        }
    }
    out.trim().to_string()
}

/// DuckDuckGo wraps outgoing links as `/l/?uddg=<percent-encoded>`.
fn decode_ddg_url(href: &str) -> String {
    let url = match href.split_once("uddg=") {
        Some((_, rest)) => percent_decode(rest.split('&').next().unwrap_or(rest)),
        None => href.to_string(),
    };
    if let Some(rest) = url.strip_prefix("//") {
        format!("https://{rest}")
    } else {
        url
    }
}

fn percent_decode(text: &str) -> String {
    let bytes = text.as_bytes();
    let mut out: Vec<u8> = Vec::with_capacity(bytes.len());
    let mut index = 0;
    while index < bytes.len() {
        match bytes[index] {
            b'%' if index + 2 < bytes.len() => {
                let hex = std::str::from_utf8(&bytes[index + 1..index + 3]).unwrap_or("");
                match u8::from_str_radix(hex, 16) {
                    Ok(byte) => {
                        out.push(byte);
                        index += 3;
                    }
                    Err(_) => {
                        out.push(bytes[index]);
                        index += 1;
                    }
                }
            }
            b'+' => {
                out.push(b' ');
                index += 1;
            }
            byte => {
                out.push(byte);
                index += 1;
            }
        }
    }
    String::from_utf8_lossy(&out).to_string()
}

fn unescape_html(text: &str) -> String {
    text.replace("&amp;", "&")
        .replace("&lt;", "<")
        .replace("&gt;", ">")
        .replace("&quot;", "\"")
        .replace("&#x27;", "'")
        .replace("&#39;", "'")
        .replace("&nbsp;", " ")
        .replace("&hellip;", "…")
        .replace("&mdash;", "—")
        .replace("&ndash;", "–")
}

/// Exa's own endpoint, used when a key is set: the MCP one shares a free tier
/// that rate-limits almost immediately.
async fn exa_search(http: &reqwest::Client, query: &str, args: &Value) -> Result<String, String> {
    let payload = json!({
        "jsonrpc": "2.0",
        "id": 1,
        "method": "tools/call",
        "params": {
            "name": "web_search_exa",
            "arguments": {
                "query": query,
                "type": args.get("type").and_then(Value::as_str).unwrap_or("auto"),
                "numResults": args.get("numResults").and_then(Value::as_u64).unwrap_or(8),
                "livecrawl": args
                    .get("livecrawl")
                    .and_then(Value::as_str)
                    .unwrap_or("fallback"),
                "contextMaxCharacters": args
                    .get("contextMaxCharacters")
                    .and_then(Value::as_u64)
                    .unwrap_or(10_000),
            }
        }
    });

    let request = http
        .post(exa_endpoint())
        .header("Accept", "application/json, text/event-stream")
        .json(&payload);

    let response = match tokio::time::timeout(SEARCH_TIMEOUT, request.send()).await {
        Err(_) => return Err(format!("exa: timed out after {}s", SEARCH_TIMEOUT.as_secs())),
        Ok(Err(e)) => return Err(format!("exa: request failed: {e}")),
        Ok(Ok(response)) => response,
    };

    let status = response.status();
    let body = response.text().await.unwrap_or_default();
    if status.as_u16() == 429 {
        return Err("exa: the shared free tier rate limit was reached".to_string());
    }
    if !status.is_success() {
        return Err(format!(
            "exa: HTTP {status}: {}",
            body.chars().take(200).collect::<String>()
        ));
    }

    match extract_search_text(&body) {
        Some(Ok(text)) => Ok(text),
        Some(Err(message)) => Err(format!("exa: {message}")),
        // Nothing parseable came back; hand the raw body over rather than swallow it.
        None => Ok(body.chars().take(MAX_OUTPUT).collect()),
    }
}

// ---------------------------------------------------------------- ast

/// Builds a `tree`-style listing. `.git` is skipped; everything else is walked.
fn build_tree(root: &Path, budget: &mut usize) -> String {
    let mut out = format!("{}\n", root.display());
    walk(root, "", budget, &mut out);
    out
}

fn walk(dir: &Path, prefix: &str, budget: &mut usize, out: &mut String) {
    if *budget == 0 {
        return;
    }
    let Ok(entries) = std::fs::read_dir(dir) else {
        return;
    };
    let mut names: Vec<(String, bool)> = entries
        .filter_map(|entry| {
            let entry = entry.ok()?;
            let name = entry.file_name().to_string_lossy().to_string();
            let is_dir = entry.file_type().map(|t| t.is_dir()).unwrap_or(false);
            Some((name, is_dir))
        })
        .filter(|(name, _)| !TREE_SKIP.contains(&name.as_str()))
        .collect();
    names.sort_by(|a, b| b.1.cmp(&a.1).then_with(|| a.0.cmp(&b.0)));

    for (index, (name, is_dir)) in names.iter().enumerate() {
        if *budget == 0 {
            out.push_str(&format!("{prefix}└── … (listing truncated)\n"));
            return;
        }
        *budget -= 1;
        let last = index + 1 == names.len();
        let branch = if last { "└── " } else { "├── " };
        out.push_str(&format!("{prefix}{branch}{name}\n"));
        if *is_dir {
            let next_prefix = format!("{prefix}{}", if last { "    " } else { "│   " });
            walk(&dir.join(name), &next_prefix, budget, out);
        }
    }
}

/// One cached answer: the count, and what the file looked like when it was
/// taken, so a changed file is never answered from a stale cache.
#[derive(serde::Serialize, serde::Deserialize, Clone)]
struct CachedCount {
    lines: u64,
    size: u64,
    mtime_ns: u64,
}

fn load_count_cache(path: &Path) -> HashMap<String, CachedCount> {
    std::fs::read_to_string(path)
        .ok()
        .and_then(|raw| serde_json::from_str(&raw).ok())
        .unwrap_or_default()
}

fn save_count_cache(path: &Path, cache: &HashMap<String, CachedCount>) {
    if let Some(parent) = path.parent() {
        let _ = std::fs::create_dir_all(parent);
    }
    if let Ok(raw) = serde_json::to_string(cache) {
        let _ = std::fs::write(path, raw);
    }
}

/// Counts newlines without holding the file in memory.
fn count_lines(path: &Path) -> Option<u64> {
    let mut file = std::fs::File::open(path).ok()?;
    let mut buffer = [0u8; 64 * 1024];
    let mut newlines = 0u64;
    let mut last = 0u8;
    let mut read_anything = false;
    loop {
        let read = file.read(&mut buffer).ok()?;
        if read == 0 {
            break;
        }
        read_anything = true;
        last = buffer[read - 1];
        newlines += buffer[..read].iter().filter(|byte| **byte == b'\n').count() as u64;
    }
    if !read_anything {
        return Some(0);
    }
    // A last line without its newline still counts as a line.
    Some(if last == b'\n' { newlines } else { newlines + 1 })
}

/// The count for one file, from `.cache/count` when size and mtime still match.
fn counted(
    path: &Path,
    key: &str,
    cache: &mut HashMap<String, CachedCount>,
    refresh: bool,
) -> Option<(u64, bool)> {
    let meta = std::fs::metadata(path).ok()?;
    let size = meta.len();
    let mtime_ns = meta
        .modified()
        .ok()
        .and_then(|time| time.duration_since(std::time::UNIX_EPOCH).ok())
        .map(|age| age.as_nanos() as u64)
        .unwrap_or(0);
    if !refresh
        && let Some(hit) = cache.get(key)
        && hit.size == size
        && hit.mtime_ns == mtime_ns
    {
        return Some((hit.lines, true));
    }
    let lines = count_lines(path)?;
    cache.insert(
        key.to_string(),
        CachedCount {
            lines,
            size,
            mtime_ns,
        },
    );
    Some((lines, false))
}

/// A path as the caller wrote it: relative when it sits inside the working
/// directory, absolute otherwise.
fn shown_path(cwd: &Path, path: &Path) -> String {
    path.strip_prefix(cwd)
        .map(|rest| rest.display().to_string())
        .unwrap_or_else(|_| path.display().to_string())
}

/// Counts the lines of a file, or of every file under a directory, without
/// pulling the contents into the conversation. Answers are kept in
/// `.cache/count` and recounted whenever size or modification time changes.
pub async fn count(cwd: &Path, args: &Value) -> Outcome {
    let Some(raw) = args.get("path").and_then(Value::as_str) else {
        return err("count: 'path' is required");
    };
    let path = resolve(cwd, raw);
    if !path.exists() {
        return err(format!("count: {}: no such file or directory", path.display()));
    }
    let refresh = args
        .get("refresh")
        .and_then(Value::as_bool)
        .unwrap_or(false);
    let cache_path = cwd.join(COUNT_CACHE);
    let mut cache = if refresh {
        HashMap::new()
    } else {
        load_count_cache(&cache_path)
    };

    if path.is_file() {
        let key = path.to_string_lossy().to_string();
        let Some((lines, cached)) = counted(&path, &key, &mut cache, refresh) else {
            return err(format!("count: {}: unreadable", path.display()));
        };
        save_count_cache(&cache_path, &cache);
        let note = if cached { " (from .cache/count)" } else { "" };
        return Outcome {
            output: format!("{}: {lines} lines{note}", shown_path(cwd, &path)),
            background_id: None,
        };
    }

    // A directory: every file under it, minus `.git` and the cache itself.
    let mut files: Vec<(String, u64)> = Vec::new();
    let mut total = 0u64;
    let mut stack = vec![path.clone()];
    while let Some(dir) = stack.pop() {
        let Ok(entries) = std::fs::read_dir(&dir) else {
            continue;
        };
        for entry in entries.flatten() {
            let name = entry.file_name().to_string_lossy().to_string();
            if name == ".git" || name == ".cache" {
                continue;
            }
            let child = entry.path();
            if child.is_dir() {
                stack.push(child);
                continue;
            }
            let key = child.to_string_lossy().to_string();
            if let Some((lines, _)) = counted(&child, &key, &mut cache, refresh) {
                total += lines;
                files.push((shown_path(cwd, &child), lines));
            }
        }
    }
    save_count_cache(&cache_path, &cache);
    files.sort_by(|a, b| b.1.cmp(&a.1));
    let mut out = format!(
        "{}: {total} lines across {} file(s)",
        shown_path(cwd, &path),
        files.len()
    );
    for (name, lines) in files.iter().take(20) {
        out.push_str(&format!("\n  {lines:>8}  {name}"));
    }
    if files.len() > 20 {
        out.push_str(&format!("\n  … and {} more", files.len() - 20));
    }
    Outcome {
        output: out,
        background_id: None,
    }
}

pub async fn ast(cwd: &Path, args: &Value) -> Outcome {
    let root = args
        .get("path")
        .and_then(Value::as_str)
        .map(|raw| resolve(cwd, raw))
        .unwrap_or_else(|| cwd.to_path_buf());
    if !root.is_dir() {
        return err(format!("ast: {} is not a directory", root.display()));
    }
    let cache = cwd.join(AST_CACHE);
    let refresh = args
        .get("refresh")
        .and_then(Value::as_bool)
        .unwrap_or(false);

    let cached = if refresh {
        None
    } else {
        std::fs::read_to_string(&cache).ok().filter(|text| !text.trim().is_empty())
    };
    if let Some(text) = cached {
        return Outcome {
            output: format!(
                "{text}\n(from cache {}; pass \"refresh\": true to rebuild)",
                cache.display()
            ),

            background_id: None,

        };
    }

    let mut budget = TREE_MAX_ENTRIES;
    let tree = build_tree(&root, &mut budget);
    if let Some(parent) = cache.parent()
        && let Err(e) = std::fs::create_dir_all(parent)
    {
        return err(format!("ast: {}: {e}", parent.display()));
    }
    let _ = std::fs::write(&cache, &tree);
    Outcome {
        output: format!(
            "{}\n(cached to {}; {budget} entries still unlisted)",
            tree.trim_end(),
            cache.display()
        ),
        background_id: None,
    }
}

// ---------------------------------------------------------------- grep

/// Matches a `grep` prints before it stops; the answer says how to ask for more.
const GREP_DEFAULT_MAX: usize = 100;
/// One match line is shown up to this many characters: a minified file is a
/// single line of megabytes and would eat the whole answer.
const GREP_LINE_CAP: usize = 400;
/// Directories a search never enters: version control, caches, build output and
/// dependencies. Everything else, hidden directories included, is searched.
const GREP_SKIP: &[&str] = &[".git", ".cache", "target", "node_modules"];

/// Finds `pattern` in the files under `path` without pulling them into the
/// conversation. The answer is capped, so a broad pattern returns a bounded
/// list and says how to narrow it.
pub async fn grep(cwd: &Path, args: &Value) -> Outcome {
    let Some(pattern) = args.get("pattern").and_then(Value::as_str) else {
        return err("grep: 'pattern' is required");
    };
    let re = match regex::Regex::new(pattern) {
        Ok(re) => re,
        Err(e) => return err(format!("grep: invalid pattern: {e}")),
    };
    let max = args
        .get("max")
        .and_then(Value::as_u64)
        .map(|v| v.max(1) as usize)
        .unwrap_or(GREP_DEFAULT_MAX);
    let raw = args.get("path").and_then(Value::as_str).unwrap_or(".");
    let include = args.get("include").and_then(Value::as_str).map(str::to_string);
    let path = resolve(cwd, raw);
    if !path.exists() {
        return err(format!("grep: {raw}: no such file or directory"));
    }

    let mut files: Vec<PathBuf> = Vec::new();
    if path.is_dir() {
        let mut stack = vec![path.clone()];
        while let Some(dir) = stack.pop() {
            let Ok(entries) = std::fs::read_dir(&dir) else {
                continue;
            };
            for entry in entries.flatten() {
                let Ok(kind) = entry.file_type() else {
                    continue;
                };
                if kind.is_symlink() {
                    continue; // a link cycle would loop forever
                }
                if kind.is_dir() {
                    let name = entry.file_name().to_string_lossy().to_string();
                    if !GREP_SKIP.contains(&name.as_str()) {
                        stack.push(entry.path());
                    }
                } else {
                    if let Some(include) = &include {
                        let name = entry.file_name().to_string_lossy().to_string();
                        if !glob(include, &name) {
                            continue;
                        }
                    }
                    files.push(entry.path());
                }
            }
        }
        files.sort();
    } else {
        files.push(path.clone());
    }

    let mut matches: Vec<String> = Vec::new();
    let mut files_hit = 0usize;
    let mut files_read = 0usize;
    let mut capped = false;
    'files: for file in &files {
        // Anything that is not text — a binary, an unreadable file — has no
        // line matches to report.
        let Ok(text) = std::fs::read_to_string(file) else {
            continue;
        };
        files_read += 1;
        let mut hit = false;
        for (index, line) in text.lines().enumerate() {
            if !re.is_match(line) {
                continue;
            }
            if !hit {
                hit = true;
                files_hit += 1;
            }
            matches.push(format!(
                "{}:{}: {}",
                shown_path(cwd, file),
                index + 1,
                clip_line(line)
            ));
            if matches.len() >= max {
                capped = true;
                break 'files;
            }
        }
    }

    if matches.is_empty() {
        return Outcome {
            output: format!(
                "no matches for '{pattern}' under {raw} ({files_read} text file(s) searched)"
            ),
            background_id: None,
        };
    }

    let mut body = matches.join("\n");
    if body.len() > MAX_OUTPUT {
        let end = cut(&body, MAX_OUTPUT).len();
        body.truncate(end);
        body.push_str("\n... [output truncated by exp-1 agent; narrow the pattern or the path]");
    }
    let mut output = format!(
        "{pattern}: {} match(es) in {files_hit} file(s)\n{body}",
        matches.len()
    );
    if capped {
        output.push_str(&format!(
            "\n... stopped at {max} matches; call grep again with a larger max or a narrower path"
        ));
    }
    Outcome {
        output,
        background_id: None,
    }
}

/// A match line, trimmed and clipped so one minified line cannot eat the answer.
fn clip_line(line: &str) -> String {
    clip_line_to(line, GREP_LINE_CAP)
}

/// One line, trimmed and clipped to a readable width.
fn clip_line_to(line: &str, limit: usize) -> String {
    let trimmed = line.trim_end();
    if trimmed.chars().count() <= limit {
        return trimmed.to_string();
    }
    let mut clipped: String = trimmed.chars().take(limit).collect();
    clipped.push('…');
    clipped
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

pub fn specs() -> Vec<Value> {
    vec![
        json!({
            "type": "function",
            "name": "read",
            "description": "Read a text file. Returns at most 400 lines by default; pass \
                            limit for a different number of lines and pos for a 1-based \
                            first line. A cut read says how to continue.",
            "parameters": {
                "type": "object",
                "properties": {
                    "path": { "type": "string", "description": "File to read, relative to the working directory." },
                    "limit": { "type": "integer", "description": "Maximum number of lines to return." },
                    "pos": { "type": "integer", "description": "First line to return, 1-based." }
                },
                "required": ["path"],
                "additionalProperties": false
            }
        }),
        json!({
            "type": "function",
            "name": "write",
            "description": "Create a file or overwrite it entirely.",
            "parameters": {
                "type": "object",
                "properties": {
                    "path": { "type": "string" },
                    "content": { "type": "string" }
                },
                "required": ["path", "content"],
                "additionalProperties": false
            }
        }),
        json!({
            "type": "function",
            "name": "delete",
            "description": "Delete a file. Directories are refused; use shell for those. \
                            There is no undo, so delete only what you are sure about.",
            "parameters": {
                "type": "object",
                "properties": {
                    "path": { "type": "string", "description": "File to delete, relative to the working directory." }
                },
                "required": ["path"],
                "additionalProperties": false
            }
        }),
        json!({
            "type": "function",
            "name": "edit",
            "description": "Replace an exact snippet in a file. Fails when 'old' is absent or \
                            matches more than once, and the failure names the closest lines or \
                            the positions of the matches. Pass several 'edits' to change one \
                            file in one call: they apply in order, all or nothing. The result \
                            shows what changed, so there is no need to read the file back.",
            "parameters": {
                "type": "object",
                "properties": {
                    "path": { "type": "string" },
                    "old": { "type": "string", "description": "Exact text to replace." },
                    "new": { "type": "string", "description": "Replacement text." },
                    "edits": {
                        "type": "array",
                        "description": "Several replacements in one call, applied in order, all or nothing.",
                        "items": {
                            "type": "object",
                            "properties": {
                                "old": { "type": "string" },
                                "new": { "type": "string" }
                            },
                            "required": ["old", "new"],
                            "additionalProperties": false
                        }
                    }
                },
                "required": ["path"],
                "additionalProperties": false
            }
        }),
        json!({
            "type": "function",
            "name": "shell",
            "description": "Run a command. Foreground by default: you wait until it finishes, \
                            and one still running after 60s is promoted to the background \
                            where you are told when it finishes. Pass background true to \
                            keep working. A background command is never killed on a timer. \
                            Output is capped: a very long one keeps the beginning and the \
                            end, with the gap marked; re-read a previous run with output_id \
                            and a new limit.",
            "parameters": {
                "type": "object",
                "properties": {
                    "command": { "type": "array", "items": { "type": "string" } },
                    "background": { "type": "boolean", "description": "Run without waiting." },
                    "limit": { "type": "integer", "description": "Maximum number of output lines." },
                    "output_id": { "type": "string", "description": "Re-read previous output, or poll a background task." }
                },
                "required": [],
                "additionalProperties": false
            }
        }),
        json!({
            "type": "function",
            "name": "task",
            "description": "Spawn a subagent to do one focused piece of work in its own context \
                            and report back. Use it for work that would otherwise flood your \
                            context — a broad search across many files, a long investigation — \
                            or for something you can run in parallel with your own work.",
            "parameters": {
                "type": "object",
                "properties": {
                    "description": { "type": "string", "description": "Three or four words naming the task." },
                    "prompt": { "type": "string", "description": "The full task for the subagent, with everything it needs." }
                },
                "required": ["prompt"],
                "additionalProperties": false
            }
        }),
        json!({
            "type": "function",
            "name": "fetch",
            "description": "Read one web page as plain text. Use it when you have a URL — \
                            documentation, a changelog, an issue — instead of searching for it. \
                            The page is paged like a file: `limit` caps the lines returned \
                            (default 400) and `pos` is the 1-based line to start at.",
            "parameters": {
                "type": "object",
                "properties": {
                    "url": { "type": "string", "description": "Page to read, http or https." },
                    "limit": { "type": "integer", "description": "Maximum lines to return (default 400)." },
                    "pos": { "type": "integer", "description": "First line to return, 1-based." }
                },
                "required": ["url"],
                "additionalProperties": false
            }
        }),
        json!({
            "type": "function",
            "name": "count",
            "description": "Count the lines of a file, or of every file under a directory, \
                            without reading the contents into the conversation. Answers are \
                            cached in .cache/count and recounted when the file changes; pass \
                            \"refresh\": true to ignore the cache.",
            "parameters": {
                "type": "object",
                "properties": {
                    "path": { "type": "string", "description": "File or directory, relative to the working directory." },
                    "refresh": { "type": "boolean", "description": "Recount instead of using .cache/count." }
                },
                "required": ["path"],
                "additionalProperties": false
            }
        }),
        json!({
            "type": "function",
            "name": "grep",
            "description": "Find a pattern in the files under a directory. Use it to locate \
                            code instead of reading files to look for something. `pattern` \
                            is a regular expression. Skips .git, .cache, target and \
                            node_modules, follows no symlinks, and ignores files that are \
                            not text. The answer is capped, and a broad pattern says how to \
                            narrow it.",
            "parameters": {
                "type": "object",
                "properties": {
                    "pattern": { "type": "string", "description": "Regular expression to find." },
                    "path": { "type": "string", "description": "File or directory, relative to the working directory (default: the working directory)." },
                    "include": { "type": "string", "description": "Only search files whose name matches this pattern, e.g. *.rs." },
                    "max": { "type": "integer", "description": "Matches to return (default 100)." }
                },
                "required": ["pattern"],
                "additionalProperties": false
            }
        }),
        json!({
            "type": "function",
            "name": "websearch",
            "description": "Search the web and get page contents back. Use it for anything \
                            outside the machine: current versions, documentation, error \
                            messages, or facts you cannot read from the filesystem.",
            "parameters": {
                "type": "object",
                "properties": {
                    "query": { "type": "string", "description": "What to search for." },
                    "numResults": { "type": "integer", "description": "How many results (default 8)." },
                    "type": { "type": "string", "enum": ["auto", "fast", "deep"], "description": "auto is balanced, fast is quick, deep is thorough." },
                    "livecrawl": { "type": "string", "enum": ["fallback", "preferred"], "description": "When to crawl live instead of cached pages." },
                    "contextMaxCharacters": { "type": "integer", "description": "Cap on returned context (default 10000)." }
                },
                "required": ["query"],
                "additionalProperties": false
            }
        }),
        json!({
            "type": "function",
            "name": "ask_question",
            "description": "Ask the user a question and wait for the answer. Use it when a \
                            decision is the user's to make and guessing would waste work — a \
                            choice of approach, a missing credential, an ambiguous requirement. \
                            Give options when there are sensible choices; whatever the user types \
                            is taken as their own answer.",
            "parameters": {
                "type": "object",
                "properties": {
                    "question": { "type": "string", "description": "What to ask." },
                    "options": { "type": "array", "items": { "type": "string" }, "description": "Suggested answers the user can pick instead of typing." },
                    "label": { "type": "string", "description": "Short name for the question." },
                    "submit": { "type": "string", "description": "Label for the submit tab." }
                },
                "required": ["question"],
                "additionalProperties": false
            }
        }),
        json!({
            "type": "function",
            "name": "ast",
            "description": "Show the directory tree of the project. Cached in .cache/ast; pass \
                            refresh true to rebuild it.",
            "parameters": {
                "type": "object",
                "properties": {
                    "path": { "type": "string", "description": "Directory to list. Defaults to the working directory." },
                    "refresh": { "type": "boolean", "description": "Rebuild instead of using the cache." }
                },
                "required": [],
                "additionalProperties": false
            }
        }),
    ]
}
#[cfg(test)]
mod tests {
    use super::*;

    /// Escape sequences are stripped: colour codes are noise in a model's
    /// context, and some tools emit them even with TERM=dumb.
    #[test]
    fn escape_sequences_are_stripped() {
        assert_eq!(strip_ansi("\u{1b}[31mred\u{1b}[0m plain"), "red plain");
        assert_eq!(strip_ansi("\u{1b}]0;title\u{7}after"), "after");
        assert_eq!(strip_ansi("no escapes"), "no escapes");
    }

    /// A capped output keeps both ends: the head says what ran, the tail says
    /// how it ended, and the middle is what goes.
    #[test]
    fn a_capped_output_keeps_both_ends() {
        let text = format!("first line\n{}\nlast line", "x".repeat(MAX_OUTPUT * 2));
        let clamped = clamp(&text, None);
        assert!(clamped.starts_with("first line"), "{}", &clamped[..40.min(clamped.len())]);
        assert!(
            clamped.ends_with("last line"),
            "{}",
            &clamped[clamped.len().saturating_sub(40)..]
        );
        assert!(clamped.contains("omitted"), "the gap is marked: {}", clamped.len());
        assert!(clamped.len() <= MAX_OUTPUT + 64, "{}", clamped.len());
    }

    /// `include` narrows a search to matching file names.
    #[test]
    fn grep_can_limit_itself_to_matching_files() {
        let dir = std::env::temp_dir().join(format!("exp1-grep-include-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(dir.join("src")).expect("temp dir");
        std::fs::write(dir.join("src/lib.rs"), "needle\n").expect("write");
        std::fs::write(dir.join("src/notes.md"), "needle\n").expect("write");

        let runtime = tokio::runtime::Runtime::new().expect("runtime");
        let outcome = runtime.block_on(grep(
            &dir,
            &json!({ "pattern": "needle", "include": "*.rs" }),
        ));
        assert!(outcome.output.contains("lib.rs:1: needle"), "{}", outcome.output);
        assert!(!outcome.output.contains("notes.md"), "{}", outcome.output);
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// Several replacements in one call, and nothing written when one of them
    /// cannot be made.
    #[test]
    fn an_edit_can_change_several_places_at_once() {
        let dir = std::env::temp_dir().join(format!("exp1-multi-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).expect("temp dir");
        let file = dir.join("code.rs");
        std::fs::write(&file, "alpha\nbeta\ngamma\n").expect("write");

        let undo = Arc::new(StdMutex::new(UndoLog::default()));
        let runtime = tokio::runtime::Runtime::new().expect("runtime");
        let outcome = runtime.block_on(edit(
            &dir,
            &undo,
            &json!({
                "path": "code.rs",
                "edits": [
                    { "old": "alpha", "new": "ALPHA" },
                    { "old": "gamma", "new": "GAMMA" }
                ]
            }),
        ));
        assert!(outcome.output.contains("2 changes"), "{}", outcome.output);
        assert_eq!(std::fs::read_to_string(&file).expect("read"), "ALPHA\nbeta\nGAMMA\n");

        // One bad pair leaves the file exactly as it was.
        let outcome = runtime.block_on(edit(
            &dir,
            &undo,
            &json!({
                "path": "code.rs",
                "edits": [
                    { "old": "beta", "new": "BETA" },
                    { "old": "delta", "new": "DELTA" }
                ]
            }),
        ));
        assert!(outcome.output.contains("not found"), "{}", outcome.output);
        assert_eq!(
            std::fs::read_to_string(&file).expect("read"),
            "ALPHA\nbeta\nGAMMA\n",
            "nothing was written"
        );
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// A failed edit says enough to fix it without reading the file again.
    #[test]
    fn an_edit_failure_points_at_the_real_text() {
        let dir = std::env::temp_dir().join(format!("exp1-hint-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).expect("temp dir");
        let file = dir.join("code.rs");
        std::fs::write(&file, "fn main() {\n    let total = 1;\n    let other = 2;\n}\n")
            .expect("write");

        let undo = Arc::new(StdMutex::new(UndoLog::default()));
        let runtime = tokio::runtime::Runtime::new().expect("runtime");
        // The model got the indentation of the second line wrong, so the
        // two-line snippet is not in the file.
        let outcome = runtime.block_on(edit(
            &dir,
            &undo,
            &json!({
                "path": "code.rs",
                "old": "    let total = 1;\n   let other = 2;",
                "new": "    let total = 3;"
            }),
        ));
        assert!(outcome.output.contains("closest lines"), "{}", outcome.output);
        assert!(outcome.output.contains("2 |     let total = 1;"), "{}", outcome.output);

        // Two candidates: the answer says where they are.
        std::fs::write(&file, "let x = 1;\nlet x = 1;\n").expect("write");
        let outcome = runtime.block_on(edit(
            &dir,
            &undo,
            &json!({ "path": "code.rs", "old": "let x = 1;", "new": "let x = 2;" }),
        ));
        assert!(outcome.output.contains("lines 1, 2"), "{}", outcome.output);
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// A grep answers with `path:line: text`, skips build output, and stops at
    /// `max` with a note on how to ask for more.
    #[test]
    fn grep_finds_matches_and_skips_build_dirs() {
        let dir = std::env::temp_dir().join(format!("exp1-grep-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(dir.join("src")).expect("temp dir");
        std::fs::create_dir_all(dir.join("target")).expect("temp dir");
        std::fs::write(dir.join("src/lib.rs"), "fn alpha() {}\nfn beta() {}\n").expect("write");
        std::fs::write(dir.join("target/junk.rs"), "fn alpha() {}\n").expect("write");

        let runtime = tokio::runtime::Runtime::new().expect("runtime");
        let outcome = runtime.block_on(grep(&dir, &json!({ "pattern": "fn \\w+" })));
        assert!(outcome.output.contains("src/lib.rs:1: fn alpha() {}"), "{}", outcome.output);
        assert!(outcome.output.contains("src/lib.rs:2: fn beta() {}"), "{}", outcome.output);
        assert!(!outcome.output.contains("junk.rs"), "target must be skipped: {}", outcome.output);
        assert!(outcome.output.contains("2 match(es) in 1 file(s)"), "{}", outcome.output);

        // A cap stops the list and says how to continue.
        let outcome = runtime.block_on(grep(&dir, &json!({ "pattern": "fn", "max": 1 })));
        assert!(outcome.output.contains("stopped at 1 matches"), "{}", outcome.output);

        // A bad pattern is refused rather than treated as literal text.
        let outcome = runtime.block_on(grep(&dir, &json!({ "pattern": "fn (" })));
        assert!(outcome.output.contains("invalid pattern"), "{}", outcome.output);

        // Nothing found is still an answer, with the number of files searched.
        let outcome = runtime.block_on(grep(&dir, &json!({ "pattern": "nothing-here" })));
        assert!(outcome.output.contains("no matches"), "{}", outcome.output);
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// Truncation lands on a character boundary: cutting Cyrillic at a byte
    /// offset used to panic in the middle of a letter.
    #[test]
    fn truncation_never_splits_a_character() {
        let text = "привет, мир! ".repeat(4000);
        assert!(text.len() > MAX_OUTPUT, "the sample must be over the cap");
        let clamped = clamp(&text, None);
        assert!(clamped.len() <= MAX_OUTPUT + 64, "the byte cap held: {}", clamped.len());
        assert!(clamped.starts_with("привет"), "and nothing was mangled");

        // The helper itself never returns a slice inside a character.
        for at in 0..text.len().min(200) {
            let piece = cut(&text, at);
            assert!(text.starts_with(piece));
        }
    }

    /// A read with no limit must not swallow a large file: it returns a window
    /// and says how to continue.
    #[test]
    fn a_read_without_a_limit_is_bounded() {
        let dir = std::env::temp_dir().join(format!("exp1-read-{}", std::process::id()));
        std::fs::create_dir_all(&dir).expect("temp dir");
        let file = dir.join("big.txt");
        let body: String = (1..=1000).map(|n| format!("line {n}\n")).collect();
        std::fs::write(&file, body).expect("write");

        let runtime = tokio::runtime::Runtime::new().expect("runtime");
        let outcome = runtime.block_on(read(&dir, &json!({ "path": "big.txt" })));
        assert!(outcome.output.contains("big.txt (1000 lines)"), "{}", &outcome.output[..80.min(outcome.output.len())]);
        assert!(outcome.output.contains("showing 400 of 1000"), "{}", outcome.output);
        assert!(outcome.output.contains("line 400"), "the window should end at line 400");
        assert!(!outcome.output.contains("line 401"), "and not go past it");
        assert!(outcome.output.contains("pos to continue"), "{}", outcome.output);

        // An explicit limit still wins.
        let outcome = runtime.block_on(read(&dir, &json!({ "path": "big.txt", "limit": 10 })));
        assert!(outcome.output.contains("showing 10 of 1000"), "{}", outcome.output);
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// The edit result carries the changed lines, so the model does not read
    /// the file back just to see what it did.
    #[test]
    fn an_edit_shows_the_changed_lines() {
        let dir = std::env::temp_dir().join(format!("exp1-edit-{}", std::process::id()));
        std::fs::create_dir_all(&dir).expect("temp dir");
        let file = dir.join("code.rs");
        std::fs::write(&file, "one\ntwo\nthree\nfour\n").expect("write");

        let undo = Arc::new(StdMutex::new(UndoLog::default()));
        let runtime = tokio::runtime::Runtime::new().expect("runtime");
        let outcome = runtime.block_on(edit(
            &dir,
            &undo,
            &json!({ "path": "code.rs", "old": "three", "new": "THREE" }),
        ));
        assert!(outcome.output.contains("THREE"), "{}", outcome.output);
        assert!(outcome.output.contains("3 | THREE"), "the window is numbered: {}", outcome.output);
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn counts_lines_and_recounts_when_the_file_changes() {
        let dir = std::env::temp_dir().join(format!("exp1-count-{}", std::process::id()));
        std::fs::create_dir_all(&dir).expect("temp dir");
        let file = dir.join("lines.txt");

        // No trailing newline: the last line still counts.
        std::fs::write(&file, "one\ntwo\nthree").expect("write");
        assert_eq!(count_lines(&file), Some(3));
        // A trailing newline does not add an empty line.
        std::fs::write(&file, "one\ntwo\n").expect("write");
        assert_eq!(count_lines(&file), Some(2));
        std::fs::write(&file, "").expect("write");
        assert_eq!(count_lines(&file), Some(0));

        // The cache answers the second call, and a changed file is recounted.
        let runtime = tokio::runtime::Runtime::new().expect("runtime");
        std::fs::write(&file, "a\nb\n").expect("write");
        let first = runtime.block_on(count(&dir, &json!({ "path": "lines.txt" })));
        assert!(first.output.contains("2 lines"), "{}", first.output);
        assert!(!first.output.contains("from .cache"), "{}", first.output);
        let second = runtime.block_on(count(&dir, &json!({ "path": "lines.txt" })));
        assert!(second.output.contains("from .cache"), "{}", second.output);
        // A different mtime must invalidate it, not report the old number.
        std::thread::sleep(std::time::Duration::from_millis(20));
        std::fs::write(&file, "a\nb\nc\nd\n").expect("write");
        let third = runtime.block_on(count(&dir, &json!({ "path": "lines.txt" })));
        assert!(third.output.contains("4 lines"), "{}", third.output);

        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn delete_removes_a_file_and_refuses_a_directory() {
        let dir = std::env::temp_dir().join(format!("exp1-delete-{}", std::process::id()));
        std::fs::create_dir_all(&dir).expect("temp dir");
        let file = dir.join("gone.txt");
        std::fs::write(&file, "bye").expect("write");

        let runtime = tokio::runtime::Runtime::new().expect("runtime");
        let undo = Arc::new(StdMutex::new(UndoLog::default()));
        let outcome = runtime.block_on(delete(&dir, &undo, &json!({ "path": "gone.txt" })));
        assert!(!file.exists(), "the file should be gone");
        assert!(outcome.output.contains("deleted"), "{}", outcome.output);

        // A directory is refused, and a missing file is reported by name.
        let outcome = runtime.block_on(delete(&dir, &undo, &json!({ "path": "." })));
        assert!(outcome.output.contains("directory"), "{}", outcome.output);
        let outcome = runtime.block_on(delete(&dir, &undo, &json!({ "path": "nope.txt" })));
        assert!(outcome.output.contains("nope.txt"), "{}", outcome.output);

        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn reads_a_plain_json_reply() {
        let body = r#"{"result":{"content":[{"type":"text","text":"hello"}]}}"#;
        assert_eq!(extract_search_text(body), Some(Ok("hello".to_string())));
    }

    #[test]
    fn reads_an_sse_reply() {
        let body = "event: message\ndata: {\"result\":{\"content\":[{\"type\":\"text\",\"text\":\"found it\"}]}}\n\n";
        assert_eq!(extract_search_text(body), Some(Ok("found it".to_string())));
    }

    #[test]
    fn surfaces_a_jsonrpc_error() {
        let body = r#"{"error":{"code":-32602,"message":"invalid arguments"}}"#;
        assert_eq!(
            extract_search_text(body),
            Some(Err("invalid arguments".to_string()))
        );
    }

    #[test]
    fn reads_duckduckgo_results() {
        let html = r##"<a rel="nofollow" class="result__a" href="//duckduckgo.com/l/?uddg=https%3A%2F%2Fratatui.rs%2F&amp;rut=x">Ratatui <b>Rust</b></a>
                      <a class="result__snippet" href="#">Cook up <b>delicious</b> TUIs in Rust &amp; more</a>"##;
        let results = parse_ddg(html, 5);
        assert_eq!(results.len(), 1);
        assert!(results[0].contains("https://ratatui.rs/"), "{}", results[0]);
        assert!(results[0].contains("Cook up delicious TUIs in Rust & more"), "{}", results[0]);
    }

    #[test]
    fn skips_content_without_text() {
        let body = r#"{"result":{"content":[{"type":"image","data":"zzz"},{"type":"text","text":"second"}]}}"#;
        assert_eq!(extract_search_text(body), Some(Ok("second".to_string())));
    }

    #[test]
    fn unreadable_bodies_yield_nothing() {
        assert_eq!(extract_search_text("<html>nope</html>"), None);
        assert_eq!(extract_search_text(""), None);
    }

    #[test]
    fn encodes_query_values() {
        assert_eq!(encode_query_value("abc-123_X.y~z"), "abc-123_X.y~z");
        assert_eq!(encode_query_value("a b&c=d"), "a%20b%26c%3Dd");
    }
}
