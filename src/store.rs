//! Persistence under `~/.config/exp-1/sessions/`.
//!
//! Settings live in `settings.json`; every conversation is its own file named
//! after the moment it started, so `/sessions` can list and reopen them.

use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};
use std::path::PathBuf;

use crate::protocol::InputItem;

const SETTINGS: &str = "settings.json";

/// The user's home directory. Windows spells it `USERPROFILE`; `HOME` is the
/// Unix name and wins when both are set.
pub fn home() -> PathBuf {
    std::env::var_os("HOME")
        .or_else(|| std::env::var_os("USERPROFILE"))
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from("/"))
}

/// Configuration: settings, credentials, prompts, permissions, MCP servers and
/// skills. `$XDG_CONFIG_HOME/exp-1`, or `~/.config/exp-1`.
pub fn config_dir() -> PathBuf {
    if let Some(base) = std::env::var_os("XDG_CONFIG_HOME").filter(|base| !base.is_empty()) {
        return PathBuf::from(base).join("exp-1");
    }
    // Windows keeps application data under APPDATA rather than a dot-directory.
    #[cfg(windows)]
    if let Some(base) = std::env::var_os("APPDATA").filter(|base| !base.is_empty()) {
        return PathBuf::from(base).join("exp-1");
    }
    home().join(".config").join("exp-1")
}

/// Conversations, which are a cache: losing them costs context, not setup.
/// `$XDG_CACHE_HOME/exp-1/sessions`, or `~/.cache/exp-1/sessions`.
pub fn dir() -> PathBuf {
    if let Some(base) = std::env::var_os("XDG_CACHE_HOME").filter(|base| !base.is_empty()) {
        return PathBuf::from(base).join("exp-1").join("sessions");
    }
    #[cfg(windows)]
    if let Some(base) = std::env::var_os("LOCALAPPDATA").filter(|base| !base.is_empty()) {
        return PathBuf::from(base).join("exp-1").join("sessions");
    }
    home().join(".cache").join("exp-1").join("sessions")
}

/// How much of the cache the conversations may take. Past this, the oldest
/// session files are dropped: they are a cache, and the alternative is a
/// directory that grows without ever being looked at again.
pub const CACHE_LIMIT_BYTES: u64 = 250 * 1024 * 1024;

/// Drops the oldest conversations until the directory fits `limit` bytes.
pub fn trim_cache(limit: u64) {
    let Ok(entries) = std::fs::read_dir(dir()) else {
        return;
    };
    let mut files: Vec<(std::time::SystemTime, u64, PathBuf)> = entries
        .flatten()
        .filter_map(|entry| {
            let path = entry.path();
            let name = entry.file_name().to_string_lossy().to_string();
            if !name.ends_with(".json") || name == SETTINGS || name == CREDENTIALS {
                return None;
            }
            let meta = entry.metadata().ok()?;
            Some((meta.modified().ok()?, meta.len(), path))
        })
        .collect();
    let mut total: u64 = files.iter().map(|(_, size, _)| *size).sum();
    if total <= limit {
        return;
    }
    // Oldest first, so the newest conversations survive.
    files.sort_by_key(|(modified, _, _)| *modified);
    for (_, size, path) in files {
        if total <= limit {
            break;
        }
        if std::fs::remove_file(&path).is_ok() {
            total = total.saturating_sub(size);
        }
    }
}

/// Moves what older builds kept beside the configuration into the cache, and
/// leaves the configuration where it belongs. Runs before anything reads.
pub fn migrate() {
    let old = config_dir().join("sessions");
    if !old.is_dir() {
        return;
    }
    // Settings and credentials stay in the configuration directory.
    for name in [SETTINGS, CREDENTIALS] {
        let from = old.join(name);
        let to = config_dir().join(name);
        if from.is_file() && !to.exists() {
            let _ = std::fs::rename(&from, &to);
        }
    }
    // Everything else there is a conversation.
    if let Ok(entries) = std::fs::read_dir(&old) {
        let _ = std::fs::create_dir_all(dir());
        for entry in entries.flatten() {
            let path = entry.path();
            let name = entry.file_name().to_string_lossy().to_string();
            if !name.ends_with(".json") || name == SETTINGS || name == CREDENTIALS {
                continue;
            }
            let target = dir().join(&name);
            if !target.exists() {
                let _ = std::fs::rename(&path, &target);
            }
        }
    }
    // The directory is empty now, or holds only leftovers.
    let _ = std::fs::remove_dir(&old);
}

fn write_atomic(path: &PathBuf, bytes: &[u8]) -> Result<()> {
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent).with_context(|| format!("create {}", parent.display()))?;
    }
    let tmp = path.with_extension("tmp");
    std::fs::write(&tmp, bytes).with_context(|| format!("write {}", tmp.display()))?;
    std::fs::rename(&tmp, path).with_context(|| format!("replace {}", path.display()))?;
    Ok(())
}

pub fn now_iso() -> String {
    chrono::Local::now().to_rfc3339()
}

// ------------------------------------------------------------------ settings

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct Settings {
    pub provider: String,
    pub model: String,
    pub effort: String,
    pub theme: String,
    /// When set, only these models appear in `/model`. Empty means everything.
    pub scoped_models: Vec<String>,
    /// Who names a session: "auto" (the Codex model first, the bundled titler
    /// as a fallback), "local" (the bundled titler only) or "remote".
    pub titler: String,
    /// A model of the same provider to try when the current one keeps failing
    /// with server-side or transport errors. Empty means no fallback.
    pub fallback_model: String,
}

impl Default for Settings {
    fn default() -> Self {
        Self {
            provider: String::new(),
            model: String::new(),
            effort: String::new(),
            theme: "default".to_string(),
            scoped_models: Vec::new(),
            titler: String::new(),
            fallback_model: String::new(),
        }
    }
}

pub fn load_settings() -> Settings {
    std::fs::read_to_string(config_dir().join(SETTINGS))
        .ok()
        .and_then(|raw| serde_json::from_str(&raw).ok())
        .unwrap_or_default()
}

pub fn save_settings(settings: &Settings) {
    if let Ok(json) = serde_json::to_vec_pretty(settings) {
        let _ = write_atomic(&config_dir().join(SETTINGS), &json);
    }
}

// ------------------------------------------------------------------ credentials

const CREDENTIALS: &str = "credentials.json";

/// Per-provider API keys, plus which provider is active. Kept apart from
/// `settings.json` so the file can be locked down to the owner.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(default)]
pub struct Credentials {
    /// Provider id to API key.
    pub keys: std::collections::BTreeMap<String, String>,
}

pub fn load_credentials() -> Credentials {
    std::fs::read_to_string(config_dir().join(CREDENTIALS))
        .ok()
        .and_then(|raw| serde_json::from_str(&raw).ok())
        .unwrap_or_default()
}

pub fn save_credentials(credentials: &Credentials) -> Result<()> {
    let path = config_dir().join(CREDENTIALS);
    let json = serde_json::to_vec_pretty(credentials)?;
    write_atomic(&path, &json)?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let _ = std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o600));
    }
    Ok(())
}

/// Looks for an Anthropic OAuth token another tool already holds, so `/login`
/// does not make the user sign in twice. PI keeps one at
/// `~/.pi/agent/auth.json`, Claude Code at `~/.claude/.credentials.json`.
pub fn adopt_anthropic_token() -> Option<String> {
    let home = home();
    let candidates = [
        home.join(".pi/agent/auth.json"),
        home.join(".claude/.credentials.json"),
    ];
    for path in candidates {
        let Ok(raw) = std::fs::read_to_string(&path) else {
            continue;
        };
        let Ok(value) = serde_json::from_str::<serde_json::Value>(&raw) else {
            continue;
        };
        for key in ["anthropic", "claude", "claudeAiOauth"] {
            let Some(entry) = value.get(key) else { continue };
            // Both tools nest the access token, under one of these names.
            for field in ["access", "accessToken", "access_token"] {
                if let Some(token) = entry.get(field).and_then(serde_json::Value::as_str)
                    && token.contains("sk-ant-oat")
                {
                    return Some(token.to_string());
                }
            }
        }
    }
    None
}

/// The key for one provider, if the user has stored one.
pub fn api_key(provider: &str) -> Option<String> {
    load_credentials()
        .keys
        .get(provider)
        .map(|key| key.trim().to_string())
        .filter(|key| !key.is_empty())
}

pub fn set_api_key(provider: &str, key: &str) -> Result<()> {
    let mut credentials = load_credentials();
    credentials
        .keys
        .insert(provider.to_string(), key.trim().to_string());
    save_credentials(&credentials)
}


// ------------------------------------------------------------------ sessions

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct StoredMsg {
    /// `user`, `assistant`, `tool` or `system`.
    pub role: String,
    pub text: String,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(default)]
pub struct StoredThread {
    pub name: String,
    pub transcript: Vec<StoredMsg>,
    /// Model-side conversation, so a resumed session keeps its context.
    pub history: Vec<InputItem>,
    pub input_history: Vec<String>,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(default)]
pub struct StoredSession {
    pub id: String,
    /// Generated by the model from the first exchange.
    pub title: String,
    pub updated: String,
    /// The directory the session was started in. Empty for sessions saved
    /// before this was recorded, which are treated as global.
    pub directory: String,
    pub main: StoredThread,
    pub side: StoredThread,
}

/// What the `/sessions` list needs, without loading every transcript.
#[derive(Debug, Clone)]
pub struct SessionMeta {
    pub id: String,
    pub title: String,
    pub updated: String,
    pub directory: String,
    /// First prompt, used when the model has not named the session yet.
    pub preview: String,
}

impl SessionMeta {
    /// The line shown in the picker.
    pub fn label(&self) -> String {
        if !self.title.is_empty() {
            self.title.clone()
        } else if !self.preview.is_empty() {
            self.preview.clone()
        } else {
            self.id.clone()
        }
    }

    /// Short, local rendering of the last update.
    pub fn when(&self) -> String {
        chrono::DateTime::parse_from_rfc3339(&self.updated)
            .map(|time| time.format("%d %b %H:%M").to_string())
            .unwrap_or_else(|_| self.updated.clone())
    }
}

fn session_path(id: &str) -> PathBuf {
    dir().join(format!("{id}.json"))
}

/// Every saved session, most recently used first.
pub fn list_sessions() -> Vec<SessionMeta> {
    let Ok(entries) = std::fs::read_dir(dir()) else {
        return Vec::new();
    };
    let mut sessions: Vec<SessionMeta> = entries
        .filter_map(|entry| {
            let entry = entry.ok()?;
            let name = entry.file_name().to_string_lossy().to_string();
            if !name.ends_with(".json") || name == SETTINGS {
                return None;
            }
            let raw = std::fs::read_to_string(entry.path()).ok()?;
            let session: StoredSession = serde_json::from_str(&raw).ok()?;
            // A file without an id is not a session: pointer files and half
            // written ones used to show up in the picker as blank rows.
            if session.id.is_empty() {
                return None;
            }
            let preview = session
                .main
                .transcript
                .iter()
                .find(|msg| msg.role == "user")
                .map(|msg| {
                    let first = msg.text.lines().next().unwrap_or("").trim();
                    let mut text: String = first.chars().take(48).collect();
                    if first.chars().count() > 48 {
                        text.push('…');
                    }
                    text
                })
                .unwrap_or_default();
            Some(SessionMeta {
                id: session.id,
                title: session.title,
                updated: session.updated,
                directory: session.directory,
                preview,
            })
        })
        .collect();
    sessions.sort_by(|a, b| b.updated.cmp(&a.updated).then_with(|| b.id.cmp(&a.id)));
    sessions
}

/// Sessions a directory should see: the ones started there, anything started
/// below it, and the global ones — started in `$HOME`, or saved before the
/// directory was recorded. `$HOME` itself shows only the global ones, so a
/// home directory does not turn into a list of every project on the machine.
pub fn sessions_in(cwd: &std::path::Path, home: &std::path::Path) -> Vec<SessionMeta> {
    filter_sessions(list_sessions(), cwd, home)
}

/// The visibility rule itself, split out so it can be tested without a disk.
fn filter_sessions(
    all: Vec<SessionMeta>,
    cwd: &std::path::Path,
    home: &std::path::Path,
) -> Vec<SessionMeta> {
    all.into_iter()
        .filter(|meta| {
            if meta.directory.is_empty() {
                return true;
            }
            let directory = std::path::Path::new(&meta.directory);
            if directory == home || directory == cwd {
                return true;
            }
            cwd != home && directory.starts_with(cwd)
        })
        .collect()
}

/// Where the editable prompts live: one markdown file per role, beside the
/// session directory.
pub fn prompts_dir() -> PathBuf {
    config_dir().join("prompts")
}

/// The prompt for one role, read from its file. The built-in text is written
/// out the first time, so there is always a file to edit; an empty or missing
/// file falls back to the built-in rather than sending the model nothing.
pub fn prompt(name: &str, builtin: &str) -> String {
    let path = prompts_dir().join(format!("{name}.md"));
    if let Ok(text) = std::fs::read_to_string(&path)
        && !text.trim().is_empty()
    {
        return text;
    }
    if let Some(parent) = path.parent() {
        let _ = std::fs::create_dir_all(parent);
    }
    let _ = std::fs::write(&path, builtin);
    builtin.to_string()
}

/// The permission rules: one file beside the prompts.
pub fn permissions_path() -> PathBuf {
    config_dir().join("permissions.json")
}

/// Where installed skills live: one directory per skill, each with SKILL.md.
pub fn skills_dir() -> PathBuf {
    config_dir().join("skills")
}

/// The MCP server list: one file next to the prompts, in opencode's shape.
pub fn mcp_path() -> PathBuf {
    config_dir().join("mcp.json")
}

/// Sessions visible from a directory whose text mentions `query`: the title,
/// the preview, or anything said in either thread.
pub fn search_sessions(
    cwd: &std::path::Path,
    home: &std::path::Path,
    query: &str,
) -> Vec<SessionMeta> {
    let needle = query.to_lowercase();
    if needle.trim().is_empty() {
        return sessions_in(cwd, home);
    }
    sessions_in(cwd, home)
        .into_iter()
        .filter(|meta| {
            if meta.title.to_lowercase().contains(&needle)
                || meta.preview.to_lowercase().contains(&needle)
            {
                return true;
            }
            let Some(saved) = load_session(&meta.id) else {
                return false;
            };
            saved
                .main
                .transcript
                .iter()
                .chain(saved.side.transcript.iter())
                .any(|msg| msg.text.to_lowercase().contains(&needle))
        })
        .collect()
}

pub fn load_session(id: &str) -> Option<StoredSession> {    let raw = std::fs::read_to_string(session_path(id)).ok()?;
    serde_json::from_str(&raw).ok()
}

pub fn save_session(session: &StoredSession) -> Result<()> {
    let json = serde_json::to_vec(session)?;
    write_atomic(&session_path(&session.id), &json)
}
#[cfg(test)]
mod tests {
    use super::*;
    use std::path::Path;

    fn meta(id: &str, directory: &str) -> SessionMeta {
        SessionMeta {
            id: id.to_string(),
            title: String::new(),
            updated: String::new(),
            directory: directory.to_string(),
            preview: String::new(),
        }
    }

    #[test]
    fn sessions_are_scoped_by_directory() {
        let home = Path::new("/home/u");
        let project = Path::new("/home/u/projects/app");
        let sub = Path::new("/home/u/projects/app/crate");
        let elsewhere = Path::new("/home/u/other");
        let all = vec![
            meta("global", "/home/u"),
            meta("legacy", ""),
            meta("project", "/home/u/projects/app"),
            meta("sub", "/home/u/projects/app/crate"),
            meta("elsewhere", "/home/u/other"),
        ];
        let ids = |cwd: &Path| -> Vec<String> {
            filter_sessions(all.clone(), cwd, home)
                .into_iter()
                .map(|m| m.id)
                .collect()
        };
        // Home shows the global ones and nothing from below it.
        assert_eq!(ids(home), vec!["global", "legacy"]);
        // A project sees itself, what is below it, and the globals.
        assert_eq!(ids(project), vec!["global", "legacy", "project", "sub"]);
        // A directory below sees only its own subtree plus the globals.
        assert_eq!(ids(sub), vec!["global", "legacy", "sub"]);
        // Unrelated directories stay out.
        assert_eq!(ids(elsewhere), vec!["global", "legacy", "elsewhere"]);
    }
}
