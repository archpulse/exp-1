//! read, write, edit and delete, with the undo log they write to.

//! Tool implementations: read, write, edit, shell, ast.
//!
//! All paths are resolved against the session working directory. Nothing is
//! sandboxed: commands run as the user.

use super::*;
use serde_json::Value;
use std::io::Read;
use std::path::Path;
use std::sync::{Arc, Mutex as StdMutex};

/// A foreground command still running after this long is promoted to a
/// background task, so the agent is never blocked on something slow. A
/// background task has no clock on it: builds and test suites run for minutes,
/// and the old 120s axe was cutting them off mid-flight.

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

