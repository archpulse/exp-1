//! TUI-side connection to the background server.
//!
//! A normal `exp1` launch calls [`Connection::open`], which reuses a warm server
//! when one is registered and spawns one otherwise — the same flow as
//! opencode's default command taking `daemon.transport()` and handing it to the UI.

use anyhow::{Context, Result, bail};
use futures_util::StreamExt;
use reqwest::Client;
use std::path::Path;
use tokio::sync::mpsc;

use crate::proto::{
    WireImage,
    AnswerBody, LoginBody, PromptBody, ScopedBody, WireEvent, WireProvider, WireSnapshot,
};
use crate::{daemon, server};

/// Talks to one server; sessions are created per conversation.
pub struct Connection {
    http: Client,
    url: String,
    password: String,
}

impl Connection {
    /// Ensures a server is running and returns a connection to it.
    pub async fn open() -> Result<Self> {
        let url = daemon::ensure_running().await?;
        Ok(Self {
            http: Client::builder()
                .build()
                .context("build the HTTP client")?,
            url,
            password: daemon::password(),
        })
    }

    fn auth(&self, request: reqwest::RequestBuilder) -> reqwest::RequestBuilder {
        request.header("authorization", format!("Bearer {}", self.password))
    }

    /// Creates a session rooted at `cwd` and starts streaming its events.
    pub async fn create_session(
        &self,
        cwd: &Path,
        parent: Option<&str>,
    ) -> Result<(String, mpsc::UnboundedReceiver<WireEvent>)> {
        let mut request = self
            .auth(self.http.post(format!("{}/sessions", self.url)))
            .header("x-exp1-directory", cwd.to_string_lossy().to_string());
        if let Some(parent) = parent {
            request = request.header("x-exp1-parent", parent);
        }
        let response = request.send()
            .await
            .context("create a session")?;
        if !response.status().is_success() {
            bail!("session creation failed: HTTP {}", response.status());
        }
        let snapshot: WireSnapshot = response.json().await.context("read the new session")?;
        let events = self.stream(&snapshot.id).await?;
        Ok((snapshot.id, events))
    }

    /// Opens the SSE stream for a session, as a channel of events.
    /// Everything a session already holds: its transcript, title and usage.
    /// The event stream only carries what happens next, so attaching to a
    /// saved conversation has to ask for the past separately.
    /// Goes back to one user message: the transcript, the model's history and
    /// the file changes after it are dropped. Returns the server's note.
    pub async fn rewind(&self, session: &str, index: usize, text: &str) -> Result<String> {
        let response = self
            .auth(self.http.post(format!("{}/sessions/{session}/rewind", self.url)))
            .json(&serde_json::json!({ "index": index, "text": text }))
            .send()
            .await
            .context("rewind the session")?;
        if !response.status().is_success() {
            let status = response.status();
            let body = response.text().await.unwrap_or_default();
            bail!("rewind failed: HTTP {status}: {body}");
        }
        let answer: serde_json::Value = response.json().await.context("read the rewind answer")?;
        Ok(answer
            .get("note")
            .and_then(|note| note.as_str())
            .unwrap_or_default()
            .to_string())
    }

    pub async fn snapshot(&self, session: &str) -> Result<WireSnapshot> {
        let response = self
            .auth(self.http.get(format!("{}/sessions/{session}", self.url)))
            .send()
            .await
            .context("read the session")?;
        if !response.status().is_success() {
            bail!("session snapshot failed: HTTP {}", response.status());
        }
        response.json().await.context("parse the session")
    }

    pub async fn stream(&self, session: &str) -> Result<mpsc::UnboundedReceiver<WireEvent>> {
        let response = self
            .auth(self.http.get(format!("{}/sessions/{session}/events", self.url)))
            .header("accept", "text/event-stream")
            .send()
            .await
            .context("open the event stream")?;
        if !response.status().is_success() {
            bail!("event stream failed: HTTP {}", response.status());
        }

        let (tx, rx) = mpsc::unbounded_channel();
        let mut body = response.bytes_stream();
        tokio::spawn(async move {
            let mut buffer = String::new();
            while let Some(chunk) = body.next().await {
                let Ok(chunk) = chunk else { return };
                buffer.push_str(&String::from_utf8_lossy(&chunk));
                while let Some(newline) = buffer.find('\n') {
                    let line: String = buffer.drain(..=newline).collect();
                    let Some(payload) = line.trim().strip_prefix("data:") else {
                        continue;
                    };
                    let payload = payload.trim();
                    if payload.is_empty() {
                        continue;
                    }
                    if let Ok(event) = serde_json::from_str::<WireEvent>(payload)
                        && tx.send(event).is_err()
                    {
                        return;
                    }
                }
            }
        });
        Ok(rx)
    }

    pub async fn prompt(
        &self,
        session: &str,
        text: &str,
        images: Vec<WireImage>,
        files: Vec<String>,
    ) -> Result<()> {
        let response = self
            .auth(self.http.post(format!("{}/sessions/{session}/prompt", self.url)))
            .json(&PromptBody {
                text: text.to_string(),
                images,
                files,
            })
            .send()
            .await
            .context("send the prompt")?;
        if !response.status().is_success() {
            let status = response.status();
            let body = response.text().await.unwrap_or_default();
            bail!("prompt rejected: HTTP {status}: {body}");
        }
        Ok(())
    }

    /// Moves the session's tools to another directory, and reports where they
    /// landed (the server resolves `~` and relative paths).
    pub async fn cd(&self, session: &str, path: &str) -> Result<String> {
        let response = self
            .auth(self.http.post(format!("{}/sessions/{session}/cd", self.url)))
            .json(&crate::proto::CdBody {
                path: path.to_string(),
            })
            .send()
            .await
            .context("change directory")?;
        let status = response.status();
        let body = response.text().await.unwrap_or_default();
        if !status.is_success() {
            bail!("{body}");
        }
        let value: serde_json::Value = serde_json::from_str(&body).unwrap_or_default();
        Ok(value
            .get("cwd")
            .and_then(|cwd| cwd.as_str())
            .unwrap_or_default()
            .to_string())
    }

    /// Empties the session: transcript, model history and undo log.
    pub async fn clear(&self, session: &str) -> Result<()> {
        let response = self
            .auth(self.http.post(format!("{}/sessions/{session}/clear", self.url)))
            .send()
            .await
            .context("clear the session")?;
        if !response.status().is_success() {
            bail!("clear failed: HTTP {}", response.status());
        }
        Ok(())
    }

    /// Switches the session between building and planning.
    pub async fn mode(&self, session: &str, mode: &str) -> Result<String> {
        let response = self
            .auth(self.http.post(format!("{}/sessions/{session}/mode", self.url)))
            .json(&crate::proto::ModeBody {
                mode: mode.to_string(),
            })
            .send()
            .await
            .context("switch the mode")?;
        let status = response.status();
        let body = response.text().await.unwrap_or_default();
        if !status.is_success() {
            bail!("{body}");
        }
        let value: serde_json::Value = serde_json::from_str(&body).unwrap_or_default();
        Ok(value
            .get("note")
            .and_then(|note| note.as_str())
            .unwrap_or("mode switched")
            .to_string())
    }

    /// Switches how much the session asks before running a tool. Returns the
    /// server's note.
    pub async fn permissions(
        &self,
        session: &str,
        permissions: crate::tools::PermissionMode,
    ) -> Result<String> {
        let response = self
            .auth(self.http.post(format!("{}/sessions/{session}/permissions", self.url)))
            .json(&crate::proto::PermissionsBody { permissions })
            .send()
            .await
            .context("switch the permission mode")?;
        let status = response.status();
        let body = response.text().await.unwrap_or_default();
        if !status.is_success() {
            bail!("{body}");
        }
        let value: serde_json::Value = serde_json::from_str(&body).unwrap_or_default();
        Ok(value
            .get("note")
            .and_then(|note| note.as_str())
            .unwrap_or("permission mode switched")
            .to_string())
    }

    /// Continues this conversation as a new session, leaving the original as
    /// it is. Returns the new session's id.
    pub async fn fork(&self, session: &str) -> Result<String> {
        let response = self
            .auth(self.http.post(format!("{}/sessions/{session}/fork", self.url)))
            .send()
            .await
            .context("fork the session")?;
        let status = response.status();
        let body = response.text().await.unwrap_or_default();
        if !status.is_success() {
            bail!("{body}");
        }
        let snapshot: WireSnapshot = serde_json::from_str(&body).context("read the fork")?;
        Ok(snapshot.id)
    }

    /// Puts the files back the way they were before the last turn.
    pub async fn undo(&self, session: &str) -> Result<String> {
        let response = self
            .auth(self.http.post(format!("{}/sessions/{session}/undo", self.url)))
            .send()
            .await
            .context("ask for an undo")?;
        let status = response.status();
        let body = response.text().await.unwrap_or_default();
        if !status.is_success() {
            bail!("{body}");
        }
        let value: serde_json::Value = serde_json::from_str(&body).unwrap_or_default();
        Ok(value
            .get("note")
            .and_then(|note| note.as_str())
            .unwrap_or("undone")
            .to_string())
    }

    /// The unified diff of what the last turn changed.
    pub async fn diff(&self, session: &str) -> Result<String> {
        let response = self
            .auth(self.http.post(format!("{}/sessions/{session}/diff", self.url)))
            .send()
            .await
            .context("ask for a diff")?;
        let status = response.status();
        let body = response.text().await.unwrap_or_default();
        if !status.is_success() {
            bail!("{body}");
        }
        let value: serde_json::Value = serde_json::from_str(&body).unwrap_or_default();
        Ok(value
            .get("diff")
            .and_then(|diff| diff.as_str())
            .unwrap_or_default()
            .to_string())
    }

    /// Asks the server to summarise the conversation now.
    pub async fn compact(&self, session: &str) -> Result<()> {
        let response = self
            .auth(self.http.post(format!("{}/sessions/{session}/compact", self.url)))
            .send()
            .await
            .context("ask for compaction")?;
        let status = response.status();
        if !status.is_success() {
            let body = response.text().await.unwrap_or_default();
            bail!("{body}");
        }
        Ok(())
    }

    /// The installed skills, as name and description.
    pub async fn skills(&self) -> Result<Vec<(String, String)>> {
        let response = self
            .auth(self.http.get(format!("{}/skills", self.url)))
            .send()
            .await
            .context("ask for the skills")?;
        if !response.status().is_success() {
            bail!("skills failed: HTTP {}", response.status());
        }
        let list: Vec<serde_json::Value> = response.json().await.context("read the skills")?;
        Ok(list
            .into_iter()
            .filter_map(|skill| {
                let name = skill.get("name").and_then(|v| v.as_str())?.to_string();
                let description = skill
                    .get("description")
                    .and_then(|v| v.as_str())
                    .unwrap_or_default()
                    .to_string();
                Some((name, description))
            })
            .collect())
    }

    /// Runs an installed skill in this session.
    pub async fn skill(&self, session: &str, name: &str) -> Result<()> {
        let response = self
            .auth(self.http.post(format!("{}/sessions/{session}/skill", self.url)))
            .json(&crate::proto::SkillBody {
                name: name.to_string(),
            })
            .send()
            .await
            .context("run the skill")?;
        let status = response.status();
        if !status.is_success() {
            let body = response.text().await.unwrap_or_default();
            bail!("{body}");
        }
        Ok(())
    }

    /// The MCP servers and what they offer.
    pub async fn mcp(&self) -> Result<Vec<serde_json::Value>> {
        let response = self
            .auth(self.http.get(format!("{}/mcp", self.url)))
            .send()
            .await
            .context("ask for the MCP servers")?;
        if !response.status().is_success() {
            bail!("mcp failed: HTTP {}", response.status());
        }
        response.json().await.context("read the MCP list")
    }

    /// Asks the server to name the session again from its own history.
    pub async fn retitle(&self, session: &str) -> Result<()> {
        let response = self
            .auth(
                self.http
                    .post(format!("{}/sessions/{session}/retitle", self.url)),
            )
            .send()
            .await
            .context("retitle the session")?;
        if !response.status().is_success() {
            bail!("retitle failed: HTTP {}", response.status());
        }
        Ok(())
    }

    pub async fn answer(&self, session: &str, answers: Vec<String>) -> Result<()> {
        self.auth(self.http.post(format!("{}/sessions/{session}/answer", self.url)))
            .json(&AnswerBody { answers })
            .send()
            .await
            .context("send the answer")?;
        Ok(())
    }

    pub async fn interrupt(&self, session: &str) -> Result<()> {
        self.auth(
            self.http
                .post(format!("{}/sessions/{session}/interrupt", self.url)),
        )
        .send()
        .await
        .context("interrupt the turn")?;
        Ok(())
    }

    /// Updates the shared model and effort on the server.
    pub async fn configure(&self, model: Option<String>, effort: Option<String>) -> Result<()> {
        self.auth(self.http.post(format!("{}/config", self.url)))
            .json(&server::ConfigBody { model, effort })
            .send()
            .await
            .context("update server configuration")?;
        Ok(())
    }

    /// The provider registry, with availability and login state.
    pub async fn providers(&self) -> Result<Vec<WireProvider>> {
        let response = self
            .auth(self.http.get(format!("{}/providers", self.url)))
            .send()
            .await
            .context("fetch the providers")?;
        response.json().await.context("read the providers")
    }

    /// Activates a provider, storing a key when one is supplied.
    pub async fn login(
        &self,
        provider: &str,
        key: Option<String>,
        session: Option<String>,
        force: bool,
    ) -> Result<server::WireLogin> {
        let response = self
            .auth(self.http.post(format!("{}/login", self.url)))
            .json(&LoginBody {
                provider: provider.to_string(),
                key,
                session,
                force,
            })
            .send()
            .await
            .context("log in")?;
        if !response.status().is_success() {
            let status = response.status();
            let body = response.text().await.unwrap_or_default();
            bail!("{body} (HTTP {status})");
        }
        response.json().await.context("read the login result")
    }

    /// Restricts `/model` to a set of models; empty means all of them.
    pub async fn scope_models(&self, models: Vec<String>) -> Result<()> {
        self.auth(self.http.post(format!("{}/scoped-models", self.url)))
            .json(&ScopedBody { models })
            .send()
            .await
            .context("set the model scope")?;
        Ok(())
    }

    /// The server's current model, effort and subscription plan.
    pub async fn config(&self) -> Result<server::WireConfig> {
        let response = self
            .auth(self.http.get(format!("{}/config", self.url)))
            .send()
            .await
            .context("fetch the configuration")?;
        response.json().await.context("read the configuration")
    }

    /// Models and their real reasoning efforts, as the server sees them.
    pub async fn models(&self) -> Result<Vec<crate::proto::WireModel>> {
        let response = self
            .auth(self.http.get(format!("{}/models", self.url)))
            .send()
            .await
            .context("fetch the catalogue")?;
        response.json().await.context("read the catalogue")
    }

    /// The same list without the scope applied: the picker that *sets* the
    /// scope has to be able to offer what is not in it yet.
    pub async fn models_all(&self) -> Result<Vec<crate::proto::WireModel>> {
        let response = self
            .auth(self.http.get(format!("{}/models?all=1", self.url)))
            .send()
            .await
            .context("fetch the catalogue")?;
        response.json().await.context("read the catalogue")
    }

    /// The scheduled jobs, as the server holds them.
    pub async fn schedules(&self) -> Result<Vec<crate::schedule::Job>> {
        let response = self
            .auth(self.http.get(format!("{}/schedules", self.url)))
            .send()
            .await
            .context("fetch the schedules")?;
        if !response.status().is_success() {
            bail!("schedules failed: HTTP {}", response.status());
        }
        response.json().await.context("read the schedules")
    }

    /// `run`, `pause` or `resume` one job. Returns the server's note.
    pub async fn schedule_action(&self, id: &str, action: &str) -> Result<String> {
        let response = self
            .auth(self.http.post(format!("{}/schedules/{}/{action}", self.url, segment(id))))
            .send()
            .await
            .context("act on the schedule")?;
        schedule_note(response).await
    }

    /// Removes one job. Returns the server's note.
    pub async fn cancel_schedule(&self, id: &str) -> Result<String> {
        let response = self
            .auth(self.http.delete(format!("{}/schedules/{}", self.url, segment(id))))
            .send()
            .await
            .context("cancel the schedule")?;
        schedule_note(response).await
    }

    /// Where the page lives: the TUI hands this to a browser.
    pub fn url(&self) -> &str {
        &self.url
    }

    /// The page's settings as the server holds them.
    pub async fn web_config(&self) -> Result<serde_json::Value> {
        let response = self
            .auth(self.http.get(format!("{}/api/config", self.url)))
            .send()
            .await
            .context("fetch the web settings")?;
        if !response.status().is_success() {
            bail!("web settings failed: HTTP {}", response.status());
        }
        response.json().await.context("read the web settings")
    }

    /// Saves the page's settings whole.
    pub async fn save_web(&self, web: &serde_json::Value) -> Result<()> {
        let response = self
            .auth(self.http.post(format!("{}/api/config", self.url)))
            .json(web)
            .send()
            .await
            .context("save the web settings")?;
        if !response.status().is_success() {
            bail!("web settings were not saved: HTTP {}", response.status());
        }
        Ok(())
    }
}

/// Percent-encodes one path segment: a job can be named with spaces.
fn segment(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    for byte in text.bytes() {
        match byte {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'_' | b'.' | b'~' => {
                out.push(byte as char)
            }
            other => out.push_str(&format!("%{other:02X}")),
        }
    }
    out
}

/// The note out of a schedule response, or the server's complaint.
async fn schedule_note(response: reqwest::Response) -> Result<String> {
    let status = response.status();
    let body = response.text().await.unwrap_or_default();
    if !status.is_success() {
        bail!("{body}");
    }
    let value: serde_json::Value = serde_json::from_str(&body).unwrap_or_default();
    Ok(value
        .get("note")
        .and_then(|note| note.as_str())
        .unwrap_or_default()
        .to_string())
}