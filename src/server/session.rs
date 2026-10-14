//! the session, the shared state, and saving conversations.

use anyhow::{Context, Result};
use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::{Arc, Mutex};
use tokio::sync::mpsc;
use crate::api::CodexClient;
use crate::catalog;
use crate::proto::{
    WireAnswerRequest,
    WireEvent, WireMsg, WireMsgKind, WirePhase, WireSnapshot,
    WireUsage,
};
use crate::protocol::{InputItem, Usage};
use crate::tools;
use crate::tools::AskSlot;
use crate::store;


/// Everything a turn needs, shared by every session.
pub struct Agent {
    /// The Codex client, built the first time a Codex-protocol request needs it.
    /// Requiring its credentials at startup would stop the whole server for
    /// anyone signed into a different provider.
    pub(crate) codex: Mutex<Option<Arc<CodexClient>>>,
    /// Which registry entry is active; decides the wire protocol and the key.
    pub provider: Mutex<String>,
    pub model: Mutex<String>,
    pub effort: Mutex<String>,
    pub registry: tools::Shared,
    pub cwd: PathBuf,
}

impl Agent {
    /// The Codex client, loaded on demand and cached. A failure is not
    /// remembered: signing in afterwards makes the next call succeed.
    pub(crate) fn codex(&self) -> Result<Arc<CodexClient>> {
        let mut slot = self
            .codex
            .lock()
            .map_err(|_| anyhow::anyhow!("the Codex client lock was poisoned"))?;
        if let Some(client) = slot.as_ref() {
            return Ok(client.clone());
        }
        let credentials = crate::auth::Auth::load().context("load Codex credentials")?;
        let client = Arc::new(CodexClient::new(credentials)?);
        *slot = Some(client.clone());
        Ok(client)
    }

    /// Drops the cached client, so freshly stored credentials are picked up.
    pub(crate) fn forget_codex(&self) {
        if let Ok(mut slot) = self.codex.lock() {
            *slot = None;
        }
    }

    pub(crate) fn model(&self) -> String {
        self.model.lock().map(|g| g.clone()).unwrap_or_default()
    }
    pub(crate) fn effort(&self) -> String {
        self.effort.lock().map(|g| g.clone()).unwrap_or_default()
    }
    pub(crate) fn provider(&self) -> String {
        self.provider
            .lock()
            .map(|g| g.clone())
            .unwrap_or_else(|_| crate::providers::DEFAULT_PROVIDER.to_string())
    }
}

#[derive(Default)]
pub(crate) struct SessionState {
    pub(crate) title: String,
    /// `build` runs everything; `plan` refuses the tools that change files, so
    /// the model investigates and proposes instead of editing.
    pub(crate) mode: String,
    /// Which tools ask before running, switched with ctrl+shift+a; see
    /// `tools::PermissionMode`.
    pub(crate) permissions: crate::tools::PermissionMode,
    pub(crate) name: String,
    pub(crate) phase: WirePhase,
    pub(crate) transcript: Vec<WireMsg>,
    pub(crate) live: String,
    pub(crate) usage: Option<WireUsage>,
    pub(crate) error: Option<String>,
    pub(crate) question: Option<WireAnswerRequest>,
}

impl Default for WirePhase {
    fn default() -> Self {
        WirePhase::Idle
    }
}

pub struct Session {
    pub(crate) id: String,
    /// Working directory for this session's tools. A single server serves
    /// several projects, so this arrives with the request rather than coming
    /// from the server's own cwd; `/cd` changes it while the session lives.
    pub(crate) cwd: std::sync::Mutex<PathBuf>,
    pub(crate) state: Mutex<SessionState>,
    pub(crate) history: Arc<tokio::sync::Mutex<Vec<InputItem>>>,
    /// One channel per attached client, so nobody misses events.
    clients: Mutex<Vec<mpsc::UnboundedSender<WireEvent>>>,
    /// The session this one follows, for a side chat: its questions see the
    /// whole conversation they were opened from, without writing into it.
    pub(crate) parent: Option<String>,
    /// What this session's turns changed, for `/undo` and `/diff`.
    pub(crate) undo: Arc<std::sync::Mutex<tools::UndoLog>>,
    /// How many undo turns existed when each user message arrived, so a rewind
    /// knows how far back the files have to go.
    pub(crate) rewind_points: Mutex<Vec<usize>>,
    /// This session's window into `ask_question`, so two sessions never mix up
    /// their questions.
    pub(crate) ask: Arc<AskSlot>,
    pub(crate) agent: Mutex<Option<tokio::task::JoinHandle<()>>>,
}

impl Session {
    pub(crate) fn new(id: String, cwd: PathBuf, parent: Option<String>) -> Arc<Self> {
        Arc::new(Self {
            id,
            cwd: std::sync::Mutex::new(cwd),
            parent,
            state: Mutex::new(SessionState::default()),
            history: Arc::new(tokio::sync::Mutex::new(Vec::new())),
            clients: Mutex::new(Vec::new()),
            undo: Arc::new(std::sync::Mutex::new(tools::UndoLog::default())),
            rewind_points: Mutex::new(Vec::new()),
            ask: AskSlot::new(),
            agent: Mutex::new(None),
        })
    }

    /// Subagent picks up provider/model/effort changes from the parent each time.
    pub(crate) fn mode(&self) -> String {
        self.state
            .lock()
            .map(|state| state.mode.clone())
            .unwrap_or_else(|_| "build".to_string())
    }

    /// How much this session's tools ask before they run.
    pub(crate) fn permissions(&self) -> crate::tools::PermissionMode {
        self.state
            .lock()
            .map(|state| state.permissions)
            .unwrap_or_default()
    }

    /// The directory the tools of this session work in.
    pub(crate) fn cwd(&self) -> PathBuf {
        self.cwd
            .lock()
            .map(|cwd| cwd.clone())
            .unwrap_or_else(|_| PathBuf::from("."))
    }

    pub(crate) fn broadcast(&self, event: WireEvent) {
        if let Ok(mut clients) = self.clients.lock() {
            clients.retain(|sender| sender.send(event.clone()).is_ok());
        }
    }

    pub(crate) fn subscribe(&self) -> mpsc::UnboundedReceiver<WireEvent> {
        let (tx, rx) = mpsc::unbounded_channel();
        if let Ok(mut clients) = self.clients.lock() {
            clients.push(tx);
        }
        rx
    }

    pub(crate) fn snapshot(&self) -> WireSnapshot {
        let state = self.state.lock().expect("session state");
        WireSnapshot {
            id: self.id.clone(),
            title: state.title.clone(),
            name: state.name.clone(),
            phase: state.phase,
            transcript: state.transcript.clone(),
            live: state.live.clone(),
            usage: state.usage.clone(),
            mode: if state.mode.is_empty() {
                "build".to_string()
            } else {
                state.mode.clone()
            },
            permissions: state.permissions,
            error: state.error.clone(),
            question: state.question.clone(),
        }
    }

    pub(crate) fn push(&self, role: &str, text: impl Into<String>) {
        self.push_kind(role, text, WireMsgKind::Plain);
    }

    /// A tool's output: kept in the transcript, hidden by default in the UI.
    pub(crate) fn push_output(&self, text: impl Into<String>) {
        self.push_kind("tool", text, WireMsgKind::ToolOutput);
    }

    /// A line the next one replaces: a subagent's current step, so a long
    /// investigation does not grow the transcript one step at a time.
    pub(crate) fn progress(&self, text: impl Into<String>) {
        let text = text.into();
        if let Ok(mut state) = self.state.lock() {
            match state.transcript.last_mut() {
                Some(last) if last.kind == WireMsgKind::Progress => last.text = text.clone(),
                _ => state.transcript.push(WireMsg {
                    role: "tool".to_string(),
                    text: text.clone(),
                    images: 0,
                    kind: WireMsgKind::Progress,
                }),
            }
        }
        self.broadcast(WireEvent::Tool {
            text,
            line: WireMsgKind::Progress,
        });
    }

    fn push_kind(&self, role: &str, text: impl Into<String>, kind: WireMsgKind) {
        if let Ok(mut state) = self.state.lock() {
            state.transcript.push(WireMsg {
                role: role.to_string(),
                text: text.into(),
                images: 0,
                kind,
            });
        }
    }

    pub(crate) fn set_phase(&self, phase: WirePhase) {
        if let Ok(mut state) = self.state.lock() {
            state.phase = phase;
            if phase != WirePhase::Streaming {
                state.live.clear();
            }
        }
        self.broadcast(WireEvent::Phase { phase });
    }
}
pub(crate) struct Shared {
    pub(crate) password: String,
    pub(crate) agent: Arc<Agent>,
    /// Loaded once; the window sizes decide when to compact.
    pub(crate) catalog: Vec<catalog::ModelInfo>,
    /// The MCP servers and the tools they offer, filled in the background.
    pub(crate) mcp: Arc<crate::mcp::Registry>,
    pub(crate) plan: Mutex<Option<String>>,
    pub(crate) sessions: Mutex<HashMap<String, Arc<Session>>>,
}

pub(crate) fn read_lock(value: &Mutex<String>) -> String {
    value.lock().map(|guard| guard.clone()).unwrap_or_default()
}

pub(crate) fn usage_to_wire(usage: Usage) -> WireUsage {
    WireUsage {
        input_tokens: usage.input_tokens,
        output_tokens: usage.output_tokens,
        cached_tokens: usage.cached_tokens,
    }
}
/// Writes a session to `~/.config/exp-1/sessions/`, so `/sessions` on any client
/// can reopen it and a restarted server still finds it.
pub(crate) async fn persist(session: &Arc<Session>) {
    let (title, name, transcript) = {
        let state = match session.state.lock() {
            Ok(state) => state,
            Err(_) => return,
        };
        (
            state.title.clone(),
            state.name.clone(),
            state
                .transcript
                .iter()
                .skip(state.transcript.len().saturating_sub(MAX_STORED_MESSAGES))
                .map(|msg| store::StoredMsg {
                    role: msg.role.clone(),
                    text: msg.text.clone(),
                    kind: msg.kind,
                })
                .collect::<Vec<_>>(),
        )
    };
    let history = session.history.lock().await.clone();
    let stored = store::StoredSession {
        id: session.id.clone(),
        title,
        updated: store::now_iso(),
        directory: session.cwd().to_string_lossy().to_string(),
        main: store::StoredThread {
            name,
            transcript,
            history,
            input_history: Vec::new(),
        },
        side: store::StoredThread::default(),
    };
    let _ = store::save_session(&stored);
}

/// How many messages of a conversation are kept on disk. A transcript line is
/// small — tool output is previewed, not stored whole — so this is generous on
/// purpose: a long conversation should still be there when it is reopened. The
/// model's own context is bounded separately by pruning and compaction.
const MAX_STORED_MESSAGES: usize = 5_000;
pub(crate) fn find(shared: &Shared, id: &str) -> Option<Arc<Session>> {
    shared.sessions.lock().expect("sessions").get(id).cloned()
}
/// Rebuilds the session map from what is on disk.
pub(crate) fn restore_sessions() -> HashMap<String, Arc<Session>> {
    let mut sessions = HashMap::new();
    let permissions = store::load_settings().permissions;
    for meta in store::list_sessions() {
        let Some(saved) = store::load_session(&meta.id) else {
            continue;
        };
        let session = Session::new(meta.id.clone(), std::env::current_dir().unwrap_or_default(), None);
        {
            let Ok(mut state) = session.state.lock() else {
                continue;
            };
            state.title = saved.title.clone();
            state.mode = "build".to_string();
            state.permissions = permissions;
            state.name = saved.main.name.clone();
            state.transcript = saved
                .main
                .transcript
                .iter()
                .map(|msg| WireMsg {
                    role: msg.role.clone(),
                    text: msg.text.clone(),
                    images: 0,
                    kind: msg.kind,
                })
                .collect();
        }
        if let Ok(mut history) = session.history.try_lock() {
            *history = saved.main.history.clone();
        }
        sessions.insert(meta.id, session);
    }
    sessions
}
