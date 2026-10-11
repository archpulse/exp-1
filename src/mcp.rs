//! MCP servers: the tools they publish and the JSON-RPC that reaches them.
//!
//! Configuration lives in `~/.config/exp-1/mcp.json`, shaped exactly like
//! opencode's `mcp` block so an entry can be copied between the two:
//!
//! ```json
//! {
//!   "mcp": {
//!     "context7": { "type": "remote", "url": "https://mcp.context7.com/mcp" },
//!     "blender":  { "type": "local", "command": ["blender-mcp"],
//!                   "environment": { "BLENDER_HOST": "localhost" } }
//!   }
//! }
//! ```
//!
//! A server is reached over stdio when it is `local` and over HTTP when it is
//! `remote`; `"disabled": true` skips one without deleting it. Tools are shown
//! to the model as `<server>_<tool>`, both halves sanitised the way opencode
//! sanitises them, so two servers can never collide on a name.

use anyhow::{Context, Result, bail};
use serde::Deserialize;
use serde_json::{Value, json};
use std::collections::HashMap;
use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::Duration;
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};
use tokio::sync::{Mutex, RwLock, oneshot};

/// How long one request may take, handshake included.
const CALL_TIMEOUT: Duration = Duration::from_secs(30);

/// One entry of the `mcp` object.
#[derive(Debug, Clone, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum ServerConfig {
    Local {
        command: Vec<String>,
        #[serde(default)]
        environment: HashMap<String, String>,
        #[serde(default)]
        disabled: bool,
    },
    Remote {
        url: String,
        #[serde(default)]
        headers: HashMap<String, String>,
        #[serde(default)]
        disabled: bool,
    },
}

impl ServerConfig {
    fn disabled(&self) -> bool {
        match self {
            ServerConfig::Local { disabled, .. } | ServerConfig::Remote { disabled, .. } => *disabled,
        }
    }

    fn kind(&self) -> &'static str {
        match self {
            ServerConfig::Local { .. } => "local",
            ServerConfig::Remote { .. } => "remote",
        }
    }
}

#[derive(Debug, Clone, Deserialize)]
struct Config {
    #[serde(default)]
    mcp: HashMap<String, ServerConfig>,
}

/// The name the model sees for one tool of one server.
fn exposed_name(server: &str, tool: &str) -> String {
    let clean = |text: &str| {
        text.chars()
            .map(|ch| {
                if ch.is_ascii_alphanumeric() || ch == '_' || ch == '-' {
                    ch
                } else {
                    '_'
                }
            })
            .collect::<String>()
    };
    format!("{}_{}", clean(server), clean(tool))
}

enum Transport {
    Local {
        stdin: Mutex<tokio::process::ChildStdin>,
        pending: Arc<Mutex<HashMap<u64, oneshot::Sender<Value>>>>,
        /// Held for as long as the connection: `kill_on_drop` ends the process
        /// the moment this handle goes away.
        #[allow(dead_code)]
        child: tokio::process::Child,
    },
    Remote {
        http: reqwest::Client,
        url: String,
        headers: Vec<(String, String)>,
    },
}

/// The wire itself, before any tools are known.
struct Connection {
    transport: Transport,
    next_id: AtomicU64,
}

impl Connection {
    /// Starts the process or remembers the URL.
    fn open(config: &ServerConfig) -> Result<Self> {
        let transport = match config {
            ServerConfig::Local {
                command,
                environment,
                ..
            } => {
                let (program, args) = command
                    .split_first()
                    .context("a local server needs a command")?;
                let mut child = tokio::process::Command::new(program)
                    .args(args)
                    .envs(environment)
                    .stdin(std::process::Stdio::piped())
                    .stdout(std::process::Stdio::piped())
                    .stderr(std::process::Stdio::null())
                    .kill_on_drop(true)
                    .spawn()
                    .or_else(|err| {
                        // Windows resolves `npx` as `npx.cmd`, and the bare name
                        // does not spawn; one retry covers that.
                        #[cfg(windows)]
                        if !program.contains('.') {
                            return tokio::process::Command::new(format!("{program}.cmd"))
                                .args(args)
                                .envs(environment)
                                .stdin(std::process::Stdio::piped())
                                .stdout(std::process::Stdio::piped())
                                .stderr(std::process::Stdio::null())
                                .kill_on_drop(true)
                                .spawn();
                        }
                        Err(err)
                    })
                    .with_context(|| format!("start {program}"))?;
                let stdin = child.stdin.take().context("open the server's stdin")?;
                let stdout = child.stdout.take().context("open the server's stdout")?;
                let pending: Arc<Mutex<HashMap<u64, oneshot::Sender<Value>>>> =
                    Arc::new(Mutex::new(HashMap::new()));
                // Answers arrive on their own schedule, so one task reads the
                // stream and hands each reply to whoever waits for its id.
                let reader_pending = pending.clone();
                tokio::spawn(async move {
                    let mut lines = BufReader::new(stdout).lines();
                    while let Ok(Some(line)) = lines.next_line().await {
                        let Ok(message) = serde_json::from_str::<Value>(&line) else {
                            continue;
                        };
                        let Some(id) = message.get("id").and_then(Value::as_u64) else {
                            continue;
                        };
                        if let Some(sender) = reader_pending.lock().await.remove(&id) {
                            let _ = sender.send(message);
                        }
                    }
                    // The stream ended: nobody is going to get an answer.
                    reader_pending.lock().await.clear();
                });
                Transport::Local {
                    stdin: Mutex::new(stdin),
                    pending,
                    child,
                }
            }
            ServerConfig::Remote { url, headers, .. } => Transport::Remote {
                http: reqwest::Client::builder()
                    .timeout(CALL_TIMEOUT)
                    .build()
                    .context("build the MCP client")?,
                url: url.clone(),
                headers: headers.iter().map(|(k, v)| (k.clone(), v.clone())).collect(),
            },
        };
        Ok(Self {
            transport,
            next_id: AtomicU64::new(1),
        })
    }

    /// Sends one request and waits for its result.
    async fn request(&self, method: &str, params: Value) -> Result<Value> {
        let id = self.next_id.fetch_add(1, Ordering::Relaxed);
        let body = json!({ "jsonrpc": "2.0", "id": id, "method": method, "params": params });
        let answer = match &self.transport {
            Transport::Local { stdin, pending, .. } => {
                let (sender, receiver) = oneshot::channel();
                pending.lock().await.insert(id, sender);
                {
                    let mut stdin = stdin.lock().await;
                    stdin
                        .write_all(body.to_string().as_bytes())
                        .await
                        .context("write to the server")?;
                    stdin.write_all(b"\n").await.context("write to the server")?;
                    stdin.flush().await.context("write to the server")?;
                }
                match tokio::time::timeout(CALL_TIMEOUT, receiver).await {
                    Ok(Ok(message)) => message,
                    Ok(Err(_)) => bail!("the server closed the connection"),
                    Err(_) => {
                        pending.lock().await.remove(&id);
                        bail!("no answer within {}s", CALL_TIMEOUT.as_secs());
                    }
                }
            }
            Transport::Remote { http, url, headers } => {
                let mut request = http
                    .post(url)
                    .header("content-type", "application/json")
                    .header("accept", "application/json, text/event-stream")
                    .json(&body);
                for (name, value) in headers {
                    request = request.header(name.as_str(), value.as_str());
                }
                let response = request.send().await.context("reach the server")?;
                let status = response.status();
                let text = response.text().await.unwrap_or_default();
                if !status.is_success() {
                    bail!("HTTP {status}: {}", text.chars().take(200).collect::<String>());
                }
                parse_message(&text)?
            }
        };
        if let Some(error) = answer.get("error") {
            let message = error
                .get("message")
                .and_then(Value::as_str)
                .unwrap_or("the server reported an error");
            bail!("{message}");
        }
        Ok(answer.get("result").cloned().unwrap_or(Value::Null))
    }

    /// Sends a notification: no id, no answer expected.
    async fn notify(&self, method: &str) {
        let body = json!({ "jsonrpc": "2.0", "method": method });
        match &self.transport {
            Transport::Local { stdin, .. } => {
                let mut stdin = stdin.lock().await;
                let _ = stdin.write_all(body.to_string().as_bytes()).await;
                let _ = stdin.write_all(b"\n").await;
                let _ = stdin.flush().await;
            }
            Transport::Remote { http, url, headers } => {
                let mut request = http
                    .post(url)
                    .header("content-type", "application/json")
                    .header("accept", "application/json, text/event-stream")
                    .json(&body);
                for (name, value) in headers {
                    request = request.header(name.as_str(), value.as_str());
                }
                let _ = request.send().await;
            }
        }
    }
}

/// A remote server answers with either plain JSON or an SSE stream; both carry
/// the same JSON-RPC message.
fn parse_message(text: &str) -> Result<Value> {
    let trimmed = text.trim_start();
    if trimmed.starts_with('{') {
        return serde_json::from_str(trimmed).context("parse the server's answer");
    }
    for line in trimmed.lines() {
        let Some(payload) = line.strip_prefix("data:") else {
            continue;
        };
        if let Ok(value) = serde_json::from_str::<Value>(payload.trim()) {
            return Ok(value);
        }
    }
    bail!("the server sent nothing this client understands");
}

/// One server, connected or failed, with the tools it offered.
pub struct Server {
    pub name: String,
    pub kind: &'static str,
    pub tools: Vec<(String, Value)>,
    pub error: Option<String>,
    connection: Option<Connection>,
}

impl Server {
    /// Connects, shakes hands and asks for the tool list.
    async fn connect(name: &str, config: &ServerConfig) -> Result<Arc<Self>> {
        let connection = Connection::open(config)?;
        connection
            .request(
                "initialize",
                json!({
                    "protocolVersion": "2025-06-18",
                    "capabilities": {},
                    "clientInfo": { "name": "exp-1-agent", "version": env!("CARGO_PKG_VERSION") },
                }),
            )
            .await
            .context("initialize")?;
        connection.notify("notifications/initialized").await;
        let listed = connection
            .request("tools/list", json!({}))
            .await
            .context("list tools")?;
        let tools = listed
            .get("tools")
            .and_then(Value::as_array)
            .map(|tools| {
                tools
                    .iter()
                    .filter_map(|tool| {
                        let name = tool.get("name").and_then(Value::as_str)?.to_string();
                        Some((name, tool.clone()))
                    })
                    .collect::<Vec<_>>()
            })
            .unwrap_or_default();
        Ok(Arc::new(Self {
            name: name.to_string(),
            kind: config.kind(),
            tools,
            error: None,
            connection: Some(connection),
        }))
    }

    /// A server that never came up, kept so `/mcp` can say why.
    fn failed(name: &str, config: &ServerConfig, error: String) -> Self {
        Self {
            name: name.to_string(),
            kind: config.kind(),
            tools: Vec::new(),
            error: Some(error),
            connection: None,
        }
    }

    /// Calls one of its tools and flattens the answer into text.
    async fn call(&self, tool: &str, arguments: Value) -> Result<String> {
        if let Some(error) = &self.error {
            bail!("server unavailable: {error}");
        }
        let Some(connection) = &self.connection else {
            bail!("not connected");
        };
        let result = connection
            .request(
                "tools/call",
                json!({ "name": tool, "arguments": arguments }),
            )
            .await?;
        let text = result
            .get("content")
            .and_then(Value::as_array)
            .map(|parts| {
                parts
                    .iter()
                    .filter_map(|part| part.get("text").and_then(Value::as_str))
                    .collect::<Vec<_>>()
                    .join("\n")
            })
            .filter(|text| !text.is_empty())
            .unwrap_or_else(|| result.to_string());
        if result
            .get("isError")
            .and_then(Value::as_bool)
            .unwrap_or(false)
        {
            bail!("{text}");
        }
        Ok(text)
    }
}

#[derive(Default)]
struct Inner {
    servers: Vec<Arc<Server>>,
    /// Exposed name to (server index, tool name).
    routes: HashMap<String, (usize, String)>,
    specs: Vec<Value>,
}

/// Every configured server, and the tools they offer.
#[derive(Default)]
pub struct Registry {
    inner: RwLock<Inner>,
}

impl Registry {
    /// Reads the config and connects to every enabled server. A server that
    /// fails is kept with its error rather than dropped silently.
    pub async fn fill(&self) {
        let path = crate::store::mcp_path();
        let Ok(raw) = std::fs::read_to_string(&path) else {
            return;
        };
        let Ok(config) = serde_json::from_str::<Config>(&raw) else {
            eprintln!("exp-1: {} is not valid JSON", path.display());
            return;
        };
        let mut names: Vec<&String> = config.mcp.keys().collect();
        names.sort();
        let mut servers: Vec<Arc<Server>> = Vec::new();
        for name in names {
            let entry = &config.mcp[name];
            if entry.disabled() {
                continue;
            }
            match Server::connect(name, entry).await {
                Ok(server) => {
                    eprintln!(
                        "exp-1: mcp {name} ({}) offers {} tool(s)",
                        server.kind,
                        server.tools.len()
                    );
                    servers.push(server);
                }
                Err(err) => {
                    eprintln!("exp-1: mcp {name} unavailable: {err:#}");
                    servers.push(Arc::new(Server::failed(name, entry, format!("{err:#}"))));
                }
            }
        }

        let mut routes = HashMap::new();
        let mut specs = Vec::new();
        for (index, server) in servers.iter().enumerate() {
            for (tool, definition) in &server.tools {
                let exposed = exposed_name(&server.name, tool);
                let description = definition
                    .get("description")
                    .and_then(Value::as_str)
                    .unwrap_or("An MCP tool.");
                let parameters = definition
                    .get("inputSchema")
                    .cloned()
                    .unwrap_or_else(|| json!({ "type": "object", "properties": {} }));
                specs.push(json!({
                    "type": "function",
                    "name": exposed,
                    "description": format!("[{name}] {description}", name = server.name),
                    "parameters": parameters,
                }));
                routes.insert(exposed, (index, tool.clone()));
            }
        }
        let mut inner = self.inner.write().await;
        inner.servers = servers;
        inner.routes = routes;
        inner.specs = specs;
    }

    /// The tool specs MCP contributes to every request.
    pub async fn specs(&self) -> Vec<Value> {
        self.inner.read().await.specs.clone()
    }

    /// Whether a name belongs to an MCP tool.
    pub async fn owns(&self, exposed: &str) -> bool {
        self.inner.read().await.routes.contains_key(exposed)
    }

    /// Routes a call when the name belongs to an MCP tool, `None` otherwise.
    pub async fn call(&self, exposed: &str, arguments: &str) -> Option<Result<String>> {
        let (server, tool) = {
            let inner = self.inner.read().await;
            let (index, tool) = inner.routes.get(exposed)?.clone();
            let server = inner.servers.get(index)?.clone();
            (server, tool)
        };
        let arguments: Value = serde_json::from_str(arguments).unwrap_or(json!({}));
        Some(server.call(&tool, arguments).await)
    }

    /// One line per server, for `/mcp`.
    pub async fn status(&self) -> Vec<Value> {
        let inner = self.inner.read().await;
        inner
            .servers
            .iter()
            .map(|server| {
                json!({
                    "name": server.name,
                    "kind": server.kind,
                    "tools": server.tools.len(),
                    "error": server.error,
                })
            })
            .collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn tool_names_are_sanitised_the_way_opencode_sanitises_them() {
        assert_eq!(
            exposed_name("context7", "resolve-library-id"),
            "context7_resolve-library-id"
        );
        assert_eq!(exposed_name("my.server", "a/b c"), "my_server_a_b_c");
    }

    #[test]
    fn a_remote_answer_may_arrive_as_sse() {
        let plain = r#"{"jsonrpc":"2.0","id":1,"result":{"tools":[]}}"#;
        assert!(parse_message(plain).is_ok());
        let sse = "event: message\ndata: {\"jsonrpc\":\"2.0\",\"id\":1,\"result\":{\"ok\":true}}\n\n";
        let value = parse_message(sse).expect("parse");
        assert_eq!(value["result"]["ok"], true);
        assert!(parse_message("nothing here").is_err());
    }

    #[test]
    fn the_config_uses_opencodes_shape() {
        let raw = r#"{
            "mcp": {
                "context7": { "type": "remote", "url": "https://mcp.context7.com/mcp" },
                "blender": { "type": "local", "command": ["blender-mcp"],
                             "environment": { "HOST": "localhost" }, "disabled": true }
            }
        }"#;
        let config: Config = serde_json::from_str(raw).expect("parse");
        assert_eq!(config.mcp.len(), 2);
        assert!(config.mcp["blender"].disabled());
        assert_eq!(config.mcp["context7"].kind(), "remote");
    }
}
