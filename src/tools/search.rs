//! ast, count and grep: finding things without reading files.

//! Tool implementations: read, write, edit, shell, ast.
//!
//! All paths are resolved against the session working directory. Nothing is
//! sandboxed: commands run as the user.

use super::*;
use serde_json::Value;
use std::collections::HashMap;
use std::io::Read;
use std::path::{Path, PathBuf};

/// A foreground command still running after this long is promoted to a
/// background task, so the agent is never blocked on something slow. A
/// background task has no clock on it: builds and test suites run for minutes,
/// and the old 120s axe was cutting them off mid-flight.

// ---------------------------------------------------------------- ast

/// Builds a `tree`-style listing. `.git` is skipped; everything else is walked.
pub(crate) fn build_tree(root: &Path, budget: &mut usize) -> String {
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
        .filter(|(name, _)| !hidden_dir(name))
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
        // Build output is named, not walked: the reader learns it is there
        // without paying thousands of lines to see what is inside.
        if *is_dir && ARTIFACT_DIRS.contains(&name.as_str()) {
            out.push_str(&format!("{prefix}{branch}{name}/ (not walked)\n"));
            continue;
        }
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
pub(crate) fn count_lines(path: &Path) -> Option<u64> {
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
            if hidden_dir(&name) || ARTIFACT_DIRS.contains(&name.as_str()) {
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
                    if !search_skip(&name) {
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
pub(crate) fn clip_line(line: &str) -> String {
    clip_line_to(line, GREP_LINE_CAP)
}

/// One line, trimmed and clipped to a readable width.
pub(crate) fn clip_line_to(line: &str, limit: usize) -> String {
    let trimmed = line.trim_end();
    if trimmed.chars().count() <= limit {
        return trimmed.to_string();
    }
    let mut clipped: String = trimmed.chars().take(limit).collect();
    clipped.push('…');
    clipped
}

