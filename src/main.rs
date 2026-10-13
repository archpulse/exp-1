//! exp-1 agent — a minimal TUI coding agent that authenticates exactly like
//! the Codex CLI does (ChatGPT OAuth against `auth.openai.com`, requests to
//! `https://chatgpt.com/backend-api/codex/responses`).

mod api;
mod app;
mod auth;
mod backends;
mod catalog;
mod client;
mod daemon;
mod dictate;
mod latex;
mod mcp;
mod md;
mod models_dev;
mod proto;
mod providers;
mod oauth;
mod protocol;
mod server;
mod skills;
mod store;
mod tools;

use anyhow::{Context, Result};
use crossterm::event::{
    self, DisableMouseCapture, EnableMouseCapture, Event, KeyCode, KeyEvent, KeyEventKind,
    KeyModifiers, KeyboardEnhancementFlags, MouseButton, MouseEventKind,
    PopKeyboardEnhancementFlags, PushKeyboardEnhancementFlags,
};
use crossterm::execute;
use crossterm::terminal::{
    EnterAlternateScreen, LeaveAlternateScreen, disable_raw_mode, enable_raw_mode,
};
use ratatui::Terminal;
use ratatui::backend::CrosstermBackend;
use std::io::stdout;
use std::path::PathBuf;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};
use tokio::sync::mpsc;


use app::{App, AskView, Phase, Role, THEMES};
use proto::{WireEvent, WireModel, WireMsgKind, WirePhase, WireProvider};
use protocol::Usage;

/// Two Esc presses inside this window abort the running turn.
const DOUBLE_ESC: Duration = Duration::from_millis(600);

/// Everything the UI needs: a live connection to the background server, and the
/// server-side session id behind each UI thread.
struct Ctx {
    conn: Arc<client::Connection>,
    /// The directory this UI was started in, used by local commands such as
    /// /init and by the `@file` completion. `/cd` moves it, and the async
    /// task that does so holds a clone.
    cwd: Arc<Mutex<PathBuf>>,
    /// `[main, side]` — which server session each thread drives. Mutable,
    /// because /sessions can re-point the main thread at another session, and
    /// an `Arc` so the task that does so can hold a handle.
    sessions: Arc<Mutex<[String; 2]>>,
    events: mpsc::UnboundedSender<AgentMsg>,
}

impl Ctx {
    fn cwd(&self) -> PathBuf {
        self.cwd
            .lock()
            .map(|cwd| cwd.clone())
            .unwrap_or_else(|_| PathBuf::from("."))
    }

    fn session(&self, thread: usize) -> String {
        self.sessions
            .lock()
            .map(|slots| slots[thread].clone())
            .unwrap_or_default()
    }
}

#[derive(Clone)]
enum AgentMsg {
    Phase(usize, Phase),
    Delta(usize, String),
    Reasoning(usize, String),
    AssistantDone(usize, String),
    /// A tool line exactly as the server produced it, and what kind it is.
    ToolLine(usize, String, WireMsgKind),
    Usage(usize, Usage),
    Error(usize, String),
    Done(usize, String),
    Title(usize, String),
    /// The UI is now driving a different server session.
    Attached(usize, String),
    /// The provider registry arrived, open `/login`.
    Providers(Vec<WireProvider>),
    /// The model list arrived, open `/scoped-models`.
    Models(Vec<WireModel>),
    /// The working directory moved: the new path.
    Cwd(String),
    /// The installed skills arrived: `/name` and its description.
    Skills(Vec<(String, String)>),
    /// The model catalogue arrived at startup, for the context meter.
    Catalog(Vec<proto::WireModel>),
    /// A saved conversation arrived, to fill a thread that just attached.
    Loaded(usize, proto::WireSnapshot),
    /// A user message was clicked: the conversation goes back to it.
    Rewound(String),
    /// A piece of dictated text arrived.
    Dictation(String),
    /// The recognizer ended: the button goes back to its idle look.
    DictationStopped,
    /// A login completed: label, model, effort, and the line to show.
    ProviderChanged(String, String, String, String),
    Ask(usize, proto::WireAnswerRequest),
    AskClosed(usize),
    Idle(usize),
}

/// Translates one server event into updates for a UI thread.
fn translate(thread: usize, event: WireEvent) -> Vec<AgentMsg> {
    match event {
        WireEvent::Snapshot(_) => Vec::new(),
        WireEvent::Phase { phase } => vec![AgentMsg::Phase(
            thread,
            match phase {
                WirePhase::Idle => Phase::Idle,
                WirePhase::Streaming => Phase::Streaming,
                WirePhase::Running => Phase::Running,
            },
        )],
        WireEvent::Delta { text } => vec![AgentMsg::Delta(thread, text)],
        WireEvent::Reasoning { text } => vec![AgentMsg::Reasoning(thread, text)],
        WireEvent::Assistant { text } => vec![AgentMsg::AssistantDone(thread, text)],
        WireEvent::Tool { text, line } => vec![AgentMsg::ToolLine(thread, text, line)],
        WireEvent::Usage { usage } => vec![AgentMsg::Usage(
            thread,
            Usage {
                input_tokens: usage.input_tokens,
                output_tokens: usage.output_tokens,
                cached_tokens: usage.cached_tokens,
            },
        )],
        WireEvent::Error { message } => vec![AgentMsg::Error(thread, message)],
        WireEvent::Done { label } => vec![AgentMsg::Done(thread, label)],
        WireEvent::Title { title } => vec![AgentMsg::Title(thread, title)],
        WireEvent::Ask(request) => vec![AgentMsg::Ask(thread, request)],
        WireEvent::AskClosed => vec![AgentMsg::AskClosed(thread)],
        WireEvent::Idle => vec![AgentMsg::Idle(thread)],
    }
}

/// Drains a session stream into the UI channel for the lifetime of the app.
fn pump_events(
    thread: usize,
    mut events: mpsc::UnboundedReceiver<WireEvent>,
    tx: mpsc::UnboundedSender<AgentMsg>,
) {
    tokio::spawn(async move {
        while let Some(event) = events.recv().await {
            for message in translate(thread, event) {
                if tx.send(message).is_err() {
                    return;
                }
            }
        }
    });
}

const AGENTS_TEMPLATE: &str = "\
# AGENTS.md

Instructions for exp-1 agent in this repository.

## Commands
- Build:
- Test:
- Lint / format:
- Run:

## Conventions
- Describe the code layout a newcomer should know first.
- Note any tooling that must run before a change is testable.
- Record conventions that are not obvious from the code itself.
";

#[derive(Clone)]
struct Args {
    model: Option<String>,
    effort: Option<String>,
    cwd: PathBuf,
}

fn parse_args() -> Result<Args> {
    let mut model = std::env::var("EXP1_MODEL").ok();
    let mut effort: Option<String> = None;
    let mut cwd = std::env::current_dir().context("resolve current directory")?;

    let mut argv = std::env::args().skip(1);
    while let Some(arg) = argv.next() {
        match arg.as_str() {
            "--model" | "-m" => {
                model = Some(argv.next().context("--model needs a value")?);
            }
            "--effort" => {
                effort = Some(argv.next().context("--effort needs a value")?);
            }
            "--cwd" => {
                cwd = PathBuf::from(argv.next().context("--cwd needs a path")?);
            }
            "--help" | "-h" => {
                print_help();
                std::process::exit(0);
            }
            other => anyhow::bail!("unknown argument '{other}' (try --help)"),
        }
    }

    Ok(Args { model, effort, cwd })
}

fn print_help() {
    println!(
        "exp-1 agent {}\n\n\
         USAGE:\n    exp1 [OPTIONS]\n    exp1 install <skill>  install a skill, run it with /<name>\n\
         \x20   exp1 service <start|stop|restart|status|password>\n\n\
         OPTIONS:\n\
         \x20   -m, --model <NAME>   Model to use (default: the catalogue pick, env: EXP1_MODEL)\n\
         \x20       --effort <LEVEL>  Reasoning effort: low | medium | high (default: medium)\n\
         \x20       --cwd <PATH>      Working directory for tools\n\
         \x20   -h, --help           Show this help\n\n\
         TOOLS:\n\
         \x20 read   read a file; limit limits lines, pos sets the first line\n\
         \x20 write  create or overwrite a file\n\
         \x20 edit   replace an exact snippet, failing when it is not unique\n\
         \x20 delete remove a file (directories are refused; no undo)\n\
         \x20 shell  run a command; background true keeps the agent working\n\
         \x20 ast    show the project tree, cached in .cache/ast\n\
         \x20 count  count lines in a file or a tree, cached in .cache/count\n\
         \x20 web    search the web (Exa first, DuckDuckGo as the fallback)\n\
         \x20 fetch  read one web page as text; limit lines, pos starts at a line\n\
         \x20 task   spawn a subagent to do focused work in its own context\n\
         \x20 ask    ask the user a question and wait for the answer\n\n\
         COMMANDS:\n\
         \x20 /login          pick a provider; one that is signed in just switches\n\
         \x20 /login <id>     sign in again (device page, or a fresh key)\n\
         \x20 /scoped-models  choose which models /model offers\n\
         \x20 /sessions       reopen an earlier conversation\n\
         \x20 /retitle       name the session again from the conversation\n\
         \x20 /effort        pick the reasoning effort for the current model\n\
         \x20 /mcp           list the MCP servers and their tools\n\
         \x20 /compact       summarise the conversation now\n\
         \x20 /undo /diff     restore or inspect what the last turn changed\n\
         \x20 /fork          continue as a new session\n\
         \x20 /retry <1-10>   configure request attempts\n\
         \x20 /model /themes /status /init /clear /btw /help\n\n\
         KEYS:\n\
         \x20 ctrl + /   switch between the main thread and the side thread\n\
         \x20 shift+tab  switch between build and plan mode\n\
         \x20 esc esc    interrupt the running turn\n\n\
         AUTH:\n\
         \x20 Run /login in the app. openai (auth), github copilot and meta (auth) sign in\n\
         \x20 through the vendor's device-code page, which opens in your browser; the rest\n\
         \x20 take an API key. Picking a provider that is signed in switches to it, and\n\
         \x20 picking the one already in use signs in again. google vertex uses gcloud's\n\
         \x20 application-default credentials, and the Codex CLI's own auth.json is read\n\
         \x20 as a fallback and never rewritten.",
        env!("CARGO_PKG_VERSION")
    );
}


/// One turn: request -> stream -> run tools -> repeat, until the model stops
/// asking for tools. Runs as its own task so a double Esc can abort it.
fn run_slash(app: &mut App, ctx: &Ctx, line: &str) -> bool {
    let (name, rest) = match line.split_once(' ') {
        Some((name, rest)) => (name, rest.trim()),
        None => (line, ""),
    };

    match name {
        "/help" => {
            app.thread_mut().push(
                Role::System,
                "enter send · ctrl-c quit · ctrl-o tool output · ctrl-v paste image · ctrl-backspace deletes a word · esc esc interrupts\nctrl-/ switches thread · ctrl-g jumps to the newest message · ctrl-t to the oldest · pgup/pgdn or the wheel scroll",
            );
        }
        "/cd" => {
            // With no argument it reports where the session is, which is the
            // question people actually have when they type it.
            let conn = ctx.conn.clone();
            let session = ctx.session(0);
            let tx = ctx.events.clone();
            let wanted = rest.to_string();
            let cwd = ctx.cwd.clone();
            tokio::spawn(async move {
                match conn.cd(&session, &wanted).await {
                    Ok(landed) => {
                        if !landed.is_empty()
                            && let Ok(mut slot) = cwd.lock()
                        {
                            *slot = PathBuf::from(&landed);
                        }
                        let _ = tx.send(AgentMsg::Cwd(landed));
                    }
                    Err(err) => {
                        let _ = tx.send(AgentMsg::Error(0, format!("{err:#}")));
                    }
                }
            });
            app.thread_mut().clear_input();
            return true;
        }
        "/effort" => {
            // The list belongs to the current model, so it is read fresh.
            if app.efforts_for(&app.model).is_empty() {
                app.thread_mut()
                    .push(Role::System, "this model takes no reasoning effort");
            } else {
                app.open_effort_picker("Select Effort");
            }
            app.thread_mut().clear_input();
            return true;
        }
        "/mcp" => {
            let conn = ctx.conn.clone();
            let tx = ctx.events.clone();
            tokio::spawn(async move {
                match conn.mcp().await {
                    Ok(servers) => {
                        let _ = tx.send(AgentMsg::Done(0, render_mcp(&servers)));
                    }
                    Err(err) => {
                        let _ = tx.send(AgentMsg::Error(0, format!("{err:#}")));
                    }
                }
            });
            app.thread_mut().clear_input();
            return true;
        }
        "/fork" => {
            let conn = ctx.conn.clone();
            let tx = ctx.events.clone();
            let session = ctx.session(0);
            let sessions = ctx.sessions.clone();
            tokio::spawn(async move {
                match conn.fork(&session).await {
                    Ok(fresh) => {
                        if let Ok(mut slots) = sessions.lock() {
                            slots[0] = fresh.clone();
                        }
                        let _ = tx.send(AgentMsg::Attached(0, "fork".to_string()));
                        match conn.stream(&fresh).await {
                            Ok(events) => {
                                if let Ok(snapshot) = conn.snapshot(&fresh).await {
                                    let _ = tx.send(AgentMsg::Loaded(0, snapshot));
                                }
                                pump_events(0, events, tx.clone());
                                let _ = tx.send(AgentMsg::Done(
                                    0,
                                    "forked: this conversation continues as a new session".to_string(),
                                ));
                            }
                            Err(err) => {
                                let _ = tx.send(AgentMsg::Error(0, format!("{err:#}")));
                            }
                        }
                    }
                    Err(err) => {
                        let _ = tx.send(AgentMsg::Error(0, format!("{err:#}")));
                    }
                }
            });
            app.thread_mut().clear_input();
            return true;
        }
        "/undo" => {
            let conn = ctx.conn.clone();
            let session = ctx.session(0);
            let tx = ctx.events.clone();
            tokio::spawn(async move {
                match conn.undo(&session).await {
                    Ok(note) => {
                        let _ = tx.send(AgentMsg::Done(0, note));
                    }
                    Err(err) => {
                        let _ = tx.send(AgentMsg::Error(0, format!("{err:#}")));
                    }
                }
            });
            app.thread_mut().clear_input();
            return true;
        }
        "/diff" => {
            let conn = ctx.conn.clone();
            let session = ctx.session(0);
            let tx = ctx.events.clone();
            tokio::spawn(async move {
                match conn.diff(&session).await {
                    Ok(diff) => {
                        let _ = tx.send(AgentMsg::Done(0, diff));
                    }
                    Err(err) => {
                        let _ = tx.send(AgentMsg::Error(0, format!("{err:#}")));
                    }
                }
            });
            app.thread_mut().clear_input();
            return true;
        }
        "/compact" => {
            let conn = ctx.conn.clone();
            let session = ctx.session(0);
            let tx = ctx.events.clone();
            tokio::spawn(async move {
                match conn.compact(&session).await {
                    Ok(()) => {
                        let _ = tx.send(AgentMsg::Done(
                            0,
                            "compacted; the summary is in the conversation".to_string(),
                        ));
                    }
                    Err(err) => {
                        let _ = tx.send(AgentMsg::Error(0, format!("{err:#}")));
                    }
                }
            });
            app.thread_mut().clear_input();
            return true;
        }
        "/retitle" => {
            // The new name arrives as a Title event, so nothing is shown here
            // beyond the fact that the model was asked.
            let conn = ctx.conn.clone();
            let session = ctx.session(0);
            let tx = ctx.events.clone();
            tokio::spawn(async move {
                if let Err(err) = conn.retitle(&session).await {
                    let _ = tx.send(AgentMsg::Error(0, format!("{err:#}")));
                }
            });
            app.thread_mut()
                .push(Role::System, "naming this session again from the conversation…");
            app.thread_mut().clear_input();
            return true;
        }
        "/clear" => {
            // The screen is the visible half of a conversation; the other half
            // is the history the server sends the model, so both are emptied.
            let conn = ctx.conn.clone();
            let tx = ctx.events.clone();
            let sessions = [ctx.session(0), ctx.session(1)];
            tokio::spawn(async move {
                for session in sessions {
                    if let Err(err) = conn.clear(&session).await {
                        let _ = tx.send(AgentMsg::Error(0, format!("{err:#}")));
                    }
                }
            });
            let thread = app.thread_mut();
            thread.clear_transcript();
            thread.reasoning.clear();
            thread.usage = None;
            thread.error = None;
            thread.history.lock().unwrap().clear();
            // Clearing starts a brand new conversation rather than reusing the file.
            app.session_id = None;
            app.session_title.clear();
            app.title_started = false;
            // The footer shows the thread's name, so both threads lose theirs:
            // a cleared conversation is not "Анализ проекта" any more.
            app.threads[0].name.clear();
            app.threads[1].clear_transcript();
            app.threads[1].history.lock().unwrap().clear();
            app.threads[1].name.clear();
            app.thread_mut()
                .push(Role::System, "cleared; this conversation starts over");
        }
        "/status" => {
            let plan = app
                .plan
                .as_ref()
                .map(|p| format!("\nplan: {p}"))
                .unwrap_or_default();
            let theme_name = app.theme().name.to_string();
            let thread_label = app.thread().kind.label().to_string();
            let body = format!(
                "model: {}\neffort: {}\nworking directory: {}\ntools: read, write, edit, shell, ast\ntheme: {}\nthread: {}\nauth: Codex (ChatGPT OAuth){plan}",
                app.model,
                app.effort,
                app.cwd,
                theme_name,
                thread_label,
            );
            app.thread_mut().push(Role::System, body);
        }
        "/themes" => {
            if rest.is_empty() {
                app.open_theme_picker();
                app.thread_mut().clear_input();
                return true;
            }
            match THEMES
                .iter()
                .position(|theme| theme.name.eq_ignore_ascii_case(rest))
            {
                Some(index) => {
                    app.theme = index;
                    let name = app.theme().name.to_string();
                    app.thread_mut().push(Role::System, format!("Theme set to {name}"));
                }
                None => app
                    .thread_mut()
                    .push(Role::System, format!("unknown theme '{rest}' — try /themes")),
            }
        }
        "/model" => {
            if rest.is_empty() {
                app.open_model_picker();
                app.thread_mut().clear_input();
                return true;
            }
            let mut parts = rest.split_whitespace();
            let chosen = parts.next().unwrap_or_default().to_string();
            // `provider/slug` names one exactly; a bare name takes the provider
            // already in use when several offer it.
            let offered = app.offered_models();
            let row = match chosen.rsplit_once('/') {
                Some((provider, slug)) => offered
                    .iter()
                    .find(|model| model.provider == provider && model.slug == slug)
                    .cloned(),
                None => {
                    let active = app.active_provider_id().map(str::to_string);
                    offered
                        .iter()
                        .filter(|model| model.slug == chosen)
                        .min_by_key(|model| Some(model.provider.clone()) != active)
                        .cloned()
                }
            };
            let Some(row) = row else {
                app.thread_mut()
                    .push(Role::System, format!("unknown model '{chosen}' — try /model"));
                return true;
            };
            let levels: Vec<String> = row.efforts.iter().map(|e| e.effort.clone()).collect();
            if let Some(level) = parts.next() {
                // Providers that do not publish their levels take anything.
                if !levels.is_empty() && !levels.iter().any(|name| name == level) {
                    app.thread_mut().push(
                        Role::System,
                        format!(
                            "'{level}' is not a {} level; this model takes {}",
                            row.display_name,
                            levels.join(", ")
                        ),
                    );
                    return true;
                }
                app.effort = level.to_string();
            }
            app.model = row.slug.clone();
            // Switching models without naming an effort still has to land on an
            // effort the new model accepts.
            let settled = if levels.is_empty() || levels.iter().any(|name| name == &app.effort) {
                app.effort.clone()
            } else {
                row.default_effort
                    .clone()
                    .unwrap_or_else(|| levels[0].clone())
            };
            app.effort = settled.clone();
            // A model from another provider means switching to it first.
            let switch = (!row.provider.is_empty()
                && app
                    .active_provider_id()
                    .is_none_or(|active| active != row.provider))
            .then(|| row.provider.clone());
            // The server owns the real values; keep the UI's copy in step.
            let conn = ctx.conn.clone();
            let model = row.slug.clone();
            let effort = settled.clone();
            tokio::spawn(async move {
                if let Some(provider) = switch {
                    let _ = conn.login(&provider, None, None, false).await;
                }
                let _ = conn.configure(Some(model), Some(effort)).await;
            });
            let (model, effort) = (app.model_label(), app.effort.clone());
            app.thread_mut()
                .push(Role::System, format!("Model changed to {model} {effort}"));
        }
        "/btw" => {
            if rest.is_empty() {
                app.thread_mut()
                    .push(Role::System, "usage: /btw <question>");
                return true;
            }
            start_side_question(app, ctx, rest.to_string());
        }
        "/login" => {
            // `/login <id>` signs in again; bare `/login` opens the picker.
            if !rest.is_empty() {
                let wanted = rest.to_string();
                // A key provider has nothing to force: it needs the key typed.
                // A device-login one re-runs the vendor's page instead.
                let typed = app
                    .providers
                    .iter()
                    .find(|provider| provider.id == wanted)
                    .is_some_and(|provider| provider.needs_key);
                if typed {
                    let label = app
                        .providers
                        .iter()
                        .find(|provider| provider.id == wanted)
                        .map(|provider| provider.label.clone())
                        .unwrap_or_else(|| wanted.clone());
                    app.entering_key = Some((wanted, label.clone()));
                    app.thread_mut()
                        .push(Role::System, format!("Paste the new API key for {label}"));
                    app.thread_mut().clear_input();
                    return true;
                }
                let conn = ctx.conn.clone();
                let tx = ctx.events.clone();
                let session = ctx.session(0);
                tokio::spawn(async move {
                    match conn.login(&wanted, None, Some(session), true).await {
                        Ok(config) => {
                            let _ = tx.send(AgentMsg::ProviderChanged(
                                config.label,
                                config.model,
                                config.effort,
                                config.message,
                            ));
                        }
                        Err(err) => {
                            let _ = tx.send(AgentMsg::Error(0, format!("login failed: {err:#}")));
                        }
                    }
                });
                app.thread_mut().clear_input();
                return true;
            }
            let conn = ctx.conn.clone();
            let tx = ctx.events.clone();
            tokio::spawn(async move {
                match conn.providers().await {
                    Ok(list) => {
                        let _ = tx.send(AgentMsg::Providers(list));
                    }
                    Err(err) => {
                        let _ = tx.send(AgentMsg::Error(0, format!("{err:#}")));
                    }
                }
            });
            app.thread_mut().clear_input();
            return true;
        }
        "/scoped-models" => {
            let conn = ctx.conn.clone();
            let tx = ctx.events.clone();
            tokio::spawn(async move {
                match conn.models_all().await {
                    Ok(list) => {
                        let _ = tx.send(AgentMsg::Models(list));
                    }
                    Err(err) => {
                        let _ = tx.send(AgentMsg::Error(0, format!("{err:#}")));
                    }
                }
            });
            app.thread_mut().clear_input();
            return true;
        }
        "/sessions" => {
            // Only what belongs to this directory: its own sessions, anything
            // started below it, and the global ones. An argument searches the
            // conversations themselves.
            let sessions = if rest.is_empty() {
                store::sessions_in(&ctx.cwd(), &store::home())
            } else {
                store::search_sessions(&ctx.cwd(), &store::home(), rest)
            };
            if sessions.is_empty() {
                app.thread_mut()
                    .push(Role::System, "no saved sessions yet");
            } else {
                app.open_session_picker(sessions);
            }
            app.thread_mut().clear_input();
            return true;
        }
        "/init" => {
            let path = ctx.cwd().join("AGENTS.md");
            if path.exists() {
                app.thread_mut()
                    .push(Role::System, format!("{} already exists", path.display()));
            } else {
                match std::fs::write(&path, AGENTS_TEMPLATE) {
                    Ok(()) => app.thread_mut().push(
                        Role::System,
                        format!("Created {}", app::shorten_cwd(&path)),
                    ),
                    Err(err) => app.thread_mut().push(
                        Role::System,
                        format!("could not write {}: {err}", path.display()),
                    ),
                }
            }
        }
        _ => {
            // Not a built-in command: it may be an installed skill, which runs
            // on the server so its instructions land in the conversation.
            let wanted = name.trim_start_matches('/').to_string();
            if !app
                .skills
                .iter()
                .any(|(skill, _)| skill.trim_start_matches('/') == wanted)
            {
                return false;
            }
            let conn = ctx.conn.clone();
            let session = ctx.session(0);
            let tx = ctx.events.clone();
            tokio::spawn(async move {
                if let Err(err) = conn.skill(&session, &wanted).await {
                    let _ = tx.send(AgentMsg::Error(0, format!("{err:#}")));
                }
            });
            // The server writes the `/skill` line into the transcript itself.
            app.thread_mut().clear_input();
            return true;
        }
    }

    true
}

/// Seeds the side thread with everything the main thread knows, once.
/// `/btw` answers a question in the side thread, with the main context but no
/// tools, while the main turn keeps running.
fn start_side_question(app: &mut App, ctx: &Ctx, question: String) {
    {
        let thread = &mut app.threads[1];
        thread.name = if thread.name.is_empty() {
            thread_name(&question)
        } else {
            thread.name.clone()
        };
        thread.started_at = Some(Instant::now());
        thread.phase = Phase::Streaming;
        thread.error = None;
        thread.live.clear();
    }
    let conn = ctx.conn.clone();
    let session = ctx.session(1);
    tokio::spawn(async move {
        if let Err(err) = conn.prompt(&session, &question, Vec::new(), Vec::new()).await {
            eprintln!("exp-1: side question failed: {err:#}");
        }
    });
}

/// Ticks or unticks one model in the `/scoped-models` list.
fn toggle_scoped(app: &mut App, index: usize) {
    let Some(model) = app.all_models.get(index).cloned() else {
        return;
    };
    let key = app::scope_key(&model);
    // An empty set means "everything", so the first tick has to fill it in.
    if app.scoped.is_empty() {
        app.scoped = app.all_models.iter().map(app::scope_key).collect();
    }
    if !app.scoped.remove(&key) {
        app.scoped.insert(key);
    }
    // Ticking every row is the same as no scope at all.
    let every: std::collections::BTreeSet<String> =
        app.all_models.iter().map(app::scope_key).collect();
    if app.scoped == every {
        app.scoped.clear();
    }
    app.open_scoped_picker();
    if let Some(picker) = app.picker.as_mut() {
        picker.selected = index;
    }
}

fn select_from_picker(app: &mut App, ctx: &Ctx) {
    let Some(picker) = app.picker.take() else {
        return;
    };
    let index = picker.selected;
    let thread = app.active;
    match picker.kind {
        app::PickerKind::Model => {
            // The list is what the picker drew: the rows the scope allows,
            // each knowing which provider it belongs to.
            let Some(row) = app.offered_models().get(index).cloned() else {
                return;
            };
            let slug = row.slug.clone();
            // Choosing a model from another provider means switching to it.
            let switch = (!row.provider.is_empty()
                && app
                    .active_provider_id()
                    .is_none_or(|active| active != row.provider))
            .then(|| row.provider.clone());
            // The new model may not accept the effort we were using.
            let settled = app
                .model_info(&slug)
                .map(|model| model.settle_effort(&app.effort))
                .unwrap_or_else(|| app.effort.clone());
            app.model = slug.clone();
            app.effort = settled.clone();
            let conn = ctx.conn.clone();
            let (model, effort) = (slug, settled);
            tokio::spawn(async move {
                if let Some(provider) = switch {
                    let _ = conn.login(&provider, None, None, false).await;
                }
                let _ = conn.configure(Some(model), Some(effort)).await;
            });
            // Choosing a model is only half of it; follow with the effort.
            app.open_effort_picker("Select Model and Effort");
        }
        app::PickerKind::Effort => {
            let efforts = app.efforts_for(&app.model);
            let Some((level, _)) = efforts.get(index).cloned() else {
                return;
            };
            app.effort = level.clone();
            let conn = ctx.conn.clone();
            let effort = level;
            tokio::spawn(async move {
                let _ = conn.configure(None, Some(effort)).await;
            });
            let (model, effort) = (app.model_label(), app.effort.clone());
            app.threads[thread]
                .push(Role::System, format!("Model changed to {model} {effort}"));
        }
        app::PickerKind::Provider => {
            // Cloned up front so the borrowed list is released before mutation.
            let Some((id, label, logged_in, active, needs_key, device_login)) =
                app.providers.get(index).map(|provider| {
                    (
                        provider.id.clone(),
                        provider.label.clone(),
                        provider.logged_in,
                        provider.active,
                        provider.needs_key,
                        provider.device_login,
                    )
                })
            else {
                return;
            };
            // A signed-in provider that is not the one in use only needs
            // switching, and the server narrates that. Picking the one already
            // in use is how a sign-in is repeated, so that runs the flow — or
            // asks for a key, for the providers that take one.
            let switching = logged_in && !active;
            if !switching && needs_key && !device_login {
                app.entering_key = Some((id, label.clone()));
                app.thread_mut().push(
                    Role::System,
                    if logged_in {
                        format!("Paste the new API key for {label}, then press Enter")
                    } else {
                        format!("Paste the API key for {label}, then press Enter")
                    },
                );
                return;
            }
            // Only force the flow where there is something to sign in with;
            // an ambient provider has nothing to repeat.
            let force = !switching && (needs_key || device_login);
            let conn = ctx.conn.clone();
            let tx = ctx.events.clone();
            let session = ctx.session(0);
            tokio::spawn(async move {
                match conn.login(&id, None, Some(session), force).await {
                    Ok(config) => {
                        let _ = tx.send(AgentMsg::ProviderChanged(
                            config.label,
                            config.model,
                            config.effort,
                            config.message,
                        ));
                    }
                    Err(err) => {
                        let _ = tx.send(AgentMsg::Error(0, format!("login failed: {err:#}")));
                    }
                }
            });
        }
        app::PickerKind::Scoped => {
            toggle_scoped(app, index);
        }
        app::PickerKind::Message => {
            // The picked row is the choice: 0 copies, 1 goes back.
            let choice = index;
            let Some((message_index, text)) = app.message_target.take() else {
                return;
            };
            if choice == 0 {
                let chars = text.chars().count();
                if copy_to_clipboard(&text) {
                    app.copied = Some((format!("copied {chars} chars"), Instant::now()));
                }
            } else {
                revert_to(app, ctx, message_index, text);
            }
        }
        app::PickerKind::Session => {
            let Some(meta) = app.session_list.get(index).cloned() else {
                return;
            };
            {
                let Ok(mut slots) = ctx.sessions.lock() else {
                    return;
                };
                if slots[0] == meta.id {
                    return;
                }
                slots[0] = meta.id.clone();
            }
            let conn = ctx.conn.clone();
            let tx = ctx.events.clone();
            tokio::spawn(async move {
                match conn.stream(&meta.id).await {
                    Ok(events) => {
                        let _ = tx.send(AgentMsg::Attached(0, meta.label()));
                        // The stream only carries what happens next, so the
                        // conversation so far is fetched as a snapshot.
                        match conn.snapshot(&meta.id).await {
                            Ok(snapshot) => {
                                let _ = tx.send(AgentMsg::Loaded(0, snapshot));
                            }
                            Err(err) => {
                                let _ = tx.send(AgentMsg::Error(
                                    0,
                                    format!("could not read the session: {err:#}"),
                                ));
                            }
                        }
                        pump_events(0, events, tx);
                    }
                    Err(err) => {
                        let _ = tx.send(AgentMsg::Error(
                            0,
                            format!("could not attach to the session: {err:#}"),
                        ));
                    }
                }
            });
        }
        app::PickerKind::Theme => {
            if index >= THEMES.len() {
                return;
            }
            app.theme = index;
            let name = app.theme().name.to_string();
            app.threads[thread].push(Role::System, format!("Theme set to {name}"));
        }
    }
}

/// Deletes the run of whitespace before the cursor, then the word before it.
fn delete_word_back(input: &mut Vec<char>, cursor: &mut usize) {
    while *cursor > 0 && input[*cursor - 1].is_whitespace() {
        input.remove(*cursor - 1);
        *cursor -= 1;
    }
    while *cursor > 0 && !input[*cursor - 1].is_whitespace() {
        input.remove(*cursor - 1);
        *cursor -= 1;
    }
}

/// Deletes the whitespace after the cursor, then the word after it.
fn delete_word_forward(input: &mut Vec<char>, cursor: usize) {
    while cursor < input.len() && input[cursor].is_whitespace() {
        input.remove(cursor);
    }
    while cursor < input.len() && !input[cursor].is_whitespace() {
        input.remove(cursor);
    }
}

fn handle_ask_key(app: &mut App, key: KeyEvent, ctx: &Ctx) {
    match key.code {
        KeyCode::Up => {
            if let Some(ask) = app.asking.as_mut() {
                ask.move_selection(-1);
                ask.typed = false;
            }
        }
        KeyCode::Down => {
            if let Some(ask) = app.asking.as_mut() {
                ask.move_selection(1);
                ask.typed = false;
            }
        }
        KeyCode::Tab => {
            if let Some(ask) = app.asking.as_mut()
                && ask.questions.len() > 1
            {
                ask.index = (ask.index + 1) % ask.questions.len();
                ask.selected = 0;
                ask.typed = false;
                app.thread_mut().clear_input();
            }
        }
        KeyCode::Enter => {
            let typed = app.thread().input_text();
            let answers = {
                let Some(ask) = app.asking.as_ref() else {
                    return;
                };
                ask.questions
                    .iter()
                    .enumerate()
                    .map(|(position, question)| {
                        if position == ask.index {
                            ask.answer_for_current(&typed)
                        } else {
                            question.options.first().cloned().unwrap_or_default()
                        }
                    })
                    .collect::<Vec<String>>()
            };
            let conn = ctx.conn.clone();
            let session = ctx.session(app.active);
            tokio::spawn(async move {
                let _ = conn.answer(&session, answers).await;
            });
            app.asking = None;
            app.thread_mut().clear_input();
        }
        KeyCode::Esc => {
            app.asking = None;
            app.thread_mut().clear_input();
        }
        KeyCode::Char('c') if key.modifiers.contains(KeyModifiers::CONTROL) => {
            app.quit = true;
        }
        KeyCode::Char('u') if key.modifiers.contains(KeyModifiers::CONTROL) => {
            let thread = app.thread_mut();
            thread.input.clear();
            thread.cursor = 0;
            if let Some(ask) = app.asking.as_mut() {
                ask.typed = false;
            }
        }
        KeyCode::Char(ch) => {
            let thread = app.thread_mut();
            thread.input.insert(thread.cursor, ch);
            thread.cursor += 1;
            if let Some(ask) = app.asking.as_mut() {
                ask.typed = true;
            }
        }
        KeyCode::Backspace => {
            let thread = app.thread_mut();
            if thread.cursor > 0 {
                thread.input.remove(thread.cursor - 1);
                thread.cursor -= 1;
            }
            let empty = thread.input.is_empty();
            if let Some(ask) = app.asking.as_mut() {
                ask.typed = !empty;
            }
        }
        _ => {}
    }
}

/// Word-wise deletion, shared by the prompt and the question panel.
///
/// Terminals report what the user calls Ctrl+Backspace as the single byte
/// `0x08`, which is also Ctrl+H, so that spelling has to be accepted too.
/// Alt+Backspace is the classic readline binding for the same operation.
fn delete_word(app: &mut App, key: KeyEvent) -> bool {
    let control = key.modifiers.contains(KeyModifiers::CONTROL);
    let backward = match key.code {
        KeyCode::Char('h') | KeyCode::Char('w') => control,
        KeyCode::Backspace => {
            key.modifiers
                .intersects(KeyModifiers::CONTROL | KeyModifiers::ALT)
        }
        _ => false,
    };
    let forward = matches!(key.code, KeyCode::Delete) && control;
    if !backward && !forward {
        return false;
    }

    let thread = app.thread_mut();
    if backward {
        let cursor = &mut thread.cursor;
        delete_word_back(&mut thread.input, cursor);
    } else {
        delete_word_forward(&mut thread.input, thread.cursor);
    }
    let empty = thread.input.is_empty();
    if let Some(ask) = app.asking.as_mut() {
        ask.typed = !empty;
    }
    true
}

fn handle_key(app: &mut App, key: KeyEvent, ctx: &Ctx) {
    // ctrl+/ switches between the main conversation and the side thread.
    // shift+tab switches between building and planning: plan mode refuses the
    // tools that change files, so the agent proposes instead of editing.
    if key.code == KeyCode::BackTab {
        let next = if app.mode == "plan" { "build" } else { "plan" };
        app.mode = next.to_string();
        let conn = ctx.conn.clone();
        let tx = ctx.events.clone();
        let session = ctx.session(0);
        let wanted = next.to_string();
        tokio::spawn(async move {
            match conn.mode(&session, &wanted).await {
                Ok(note) => {
                    let _ = tx.send(AgentMsg::Done(0, note));
                }
                Err(err) => {
                    let _ = tx.send(AgentMsg::Error(0, format!("{err:#}")));
                }
            }
        });
        return;
    }

    // A terminal sends raw 0x1F, which crossterm reports as Ctrl+7; a kitty
    // that negotiated the CSI-u protocol reports Ctrl+/ instead.
    let ctrl_slash = key.modifiers.contains(KeyModifiers::CONTROL)
        && matches!(key.code, KeyCode::Char('/') | KeyCode::Char('7'));
    if ctrl_slash {
        app.active = 1 - app.active;
        app.picker = None;
        return;
    }

    if key.modifiers.contains(KeyModifiers::CONTROL) {
        match key.code {
            KeyCode::Char('c') | KeyCode::Char('C') => {
                app.quit = true;
                return;
            }
            KeyCode::Char('u') | KeyCode::Char('U') => {
                let thread = app.thread_mut();
                thread.input.clear();
                thread.cursor = 0;
                return;
            }
            _ => {}
        }
    }

    if delete_word(app, key) {
        return;
    }

    // While a question is open the keyboard answers it, nothing else.
    if app.asking.is_some() {
        handle_ask_key(app, key, ctx);
        return;
    }

    if app.picker.is_some() {
        match key.code {
            KeyCode::Up => {
                if let Some(picker) = app.picker.as_mut() {
                    picker.selected = picker.selected.saturating_sub(1);
                }
            }
            KeyCode::Down => {
                if let Some(picker) = app.picker.as_mut() {
                    let last = picker.items.len().saturating_sub(1);
                    picker.selected = (picker.selected + 1).min(last);
                }
            }
            KeyCode::Enter => {
                let scoped = app
                    .picker
                    .as_ref()
                    .is_some_and(|picker| picker.kind == app::PickerKind::Scoped);
                if scoped {
                    let index = app.picker.as_ref().map(|p| p.selected).unwrap_or(0);
                    toggle_scoped(app, index);
                } else {
                    select_from_picker(app, ctx);
                }
            }
            KeyCode::Char('a') => {
                // In the scope picker, `a` puts every model back on the shelf:
                // an empty scope means no restriction at all.
                let scoped = app
                    .picker
                    .as_ref()
                    .is_some_and(|picker| picker.kind == app::PickerKind::Scoped);
                if scoped {
                    app.scoped.clear();
                    app.open_scoped_picker();
                }
            }
            KeyCode::Esc => {
                // A message menu is answered, not saved: closing it forgets
                // which message it was about.
                if app
                    .picker
                    .as_ref()
                    .is_some_and(|picker| picker.kind == app::PickerKind::Message)
                {
                    app.message_target = None;
                    app.picker = None;
                    return;
                }
                let scoped = app
                    .picker
                    .as_ref()
                    .is_some_and(|picker| picker.kind == app::PickerKind::Scoped);
                if scoped {
                    let chosen: Vec<String> = app.scoped.iter().cloned().collect();
                    let conn = ctx.conn.clone();
                    tokio::spawn(async move {
                        let _ = conn.scope_models(chosen).await;
                    });
                    let count = app.scoped.len();
                    app.picker = None;
                    app.thread_mut().push(
                        Role::System,
                        if count == 0 {
                            "All models are offered again".to_string()
                        } else {
                            format!("{count} model(s) will be offered in /model")
                        },
                    );
                } else {
                    app.picker = None;
                }
            }
            _ => {}
        }
        return;
    }

    let popup_len = app.suggestions().len();
    let busy = app.thread().busy();

    match key.code {
        KeyCode::Esc => {
            // A single Esc deliberately does nothing. A second one, inside the
            // window, aborts the running turn or closes the completion list.
            let now = Instant::now();
            let double = app
                .last_esc
                .is_some_and(|previous| now.duration_since(previous) < DOUBLE_ESC);
            app.last_esc = if double { None } else { Some(now) };
            if !double {
                return;
            }
            if !app.suggestions().is_empty() {
                app.thread_mut().popup_open = false;
            } else if app.thread().busy() {
                let conn = ctx.conn.clone();
                let index = app.active;
                let session = ctx.session(index);
                tokio::spawn(async move {
                    let _ = conn.interrupt(&session).await;
                });
            }
        }
        KeyCode::Up => {
            if popup_len > 0 {
                app.thread_mut().selected = app.thread().selected.saturating_sub(1);
            } else {
                app.thread_mut().history_prev();
            }
        }
        KeyCode::Down => {
            if popup_len > 0 {
                let last = popup_len - 1;
                app.thread_mut().selected = (app.thread().selected + 1).min(last);
            } else {
                app.thread_mut().history_next();
            }
        }
        KeyCode::Tab => {
            app.accept_suggestion();
        }
        KeyCode::Enter => {
            // Shift+Enter writes a line break instead of sending: a prompt can
            // be several lines, the way Codex takes it.
            if key.modifiers.contains(KeyModifiers::SHIFT) {
                let thread = app.thread_mut();
                thread.input.insert(thread.cursor, '\n');
                thread.cursor += 1;
                return;
            }
            if popup_len > 0 {
                let typed = app.thread().input_text();
                let exact = app.suggestions().iter().any(|s| s.label == typed.trim());
                if !exact && app.accept_suggestion() {
                    return;
                }
                app.thread_mut().popup_open = false;
            }
            let text = app.thread().input_text();
            let text = text.trim().to_string();
            if text.is_empty() {
                return;
            }

            // `/btw` is the one thing allowed while the main turn is busy.
            if let Some(question) = text.strip_prefix("/btw ") {
                let question = question.trim().to_string();
                if question.is_empty() {
                    app.thread_mut().push(Role::System, "usage: /btw <question>");
                } else {
                    start_side_question(app, ctx, question);
                }
                app.thread_mut().clear_input();
                return;
            }

            if busy {
                return;
            }
            if app.is_bare_command("/model") {
                app.open_model_picker();
                app.thread_mut().clear_input();
                return;
            }
            if app.is_bare_command("/themes") {
                app.open_theme_picker();
                app.thread_mut().clear_input();
                return;
            }
            if text.starts_with('/') && !text.contains(' ') {
                if !run_slash(app, ctx, &text) {
                    app.thread_mut()
                        .push(Role::System, format!("unknown command '{text}' — try /help"));
                }
                app.thread_mut().clear_input();
                return;
            }
            if text.starts_with('/') && run_slash(app, ctx, &text) {
                app.thread_mut().clear_input();
                return;
            }

            // A key being typed is sent to /login, not to the model. The text
            // is never empty here: an empty Enter returns above, which keeps
            // the prompt open until something is typed.
            if let Some((id, _)) = app.entering_key.clone() {
                let key = text.clone();
                app.thread_mut().clear_input();
                app.entering_key = None;
                let conn = ctx.conn.clone();
                let tx = ctx.events.clone();
                let session = ctx.session(0);
                tokio::spawn(async move {
                    match conn.login(&id, Some(key), Some(session), true).await {
                        Ok(config) => {
                            let _ = tx.send(AgentMsg::ProviderChanged(
                                config.label,
                                config.model,
                                config.effort,
                                config.message,
                            ));
                        }
                        Err(err) => {
                            let _ = tx.send(AgentMsg::Error(0, format!("login failed: {err:#}")));
                        }
                    }
                });
                return;
            }

            // Sending is now a request to the server, not a local turn.
            let images = std::mem::take(&mut app.pending_images)
                .into_iter()
                .map(|(media_type, data)| proto::WireImage { media_type, data })
                .collect::<Vec<_>>();
            let image_count = images.len();
            let index = app.active;
            let thread = app.thread_mut();
            thread.remember(&text);
            if thread.name.is_empty() {
                thread.name = thread_name(&text);
            }
            thread.push_user(&text, image_count);
            thread.clear_input();
            thread.error = None;
            thread.scroll = 0;
            thread.started_at = Some(Instant::now());
            thread.phase = Phase::Streaming;
            let conn = ctx.conn.clone();
            let session = ctx.session(index);
            // `@file` mentions travel as paths; the server decides what to
            // attach and what to hand over to be read.
            let files = mentions(&text, &ctx.cwd());
            tokio::spawn(async move {
                if let Err(err) = conn.prompt(&session, &text, images, files).await {
                    eprintln!("exp-1: prompt failed: {err:#}");
                }
            });
        }
        KeyCode::Backspace => {
            let thread = app.thread_mut();
            if thread.cursor > 0 {
                thread.input.remove(thread.cursor - 1);
                thread.cursor -= 1;
                thread.selected = 0;
                thread.popup_open = true;
            }
        }
        KeyCode::Delete => {
            let thread = app.thread_mut();
            if thread.cursor < thread.input.len() {
                thread.input.remove(thread.cursor);
                thread.selected = 0;
                thread.popup_open = true;
            }
        }
        KeyCode::Left => {
            let thread = app.thread_mut();
            thread.cursor = thread.cursor.saturating_sub(1);
        }
        KeyCode::Right => {
            let thread = app.thread_mut();
            thread.cursor = (thread.cursor + 1).min(thread.input.len());
        }
        KeyCode::Home => app.thread_mut().cursor = 0,
        KeyCode::End => {
            let thread = app.thread_mut();
            thread.cursor = thread.input.len();
        }
        KeyCode::PageUp => {
            let thread = app.thread_mut();
            thread.scroll = thread.scroll.saturating_add(10);
        }
        KeyCode::PageDown => {
            let thread = app.thread_mut();
            thread.scroll = thread.scroll.saturating_sub(10);
        }
        KeyCode::Char(ch) => {
            // ctrl+v takes a picture off the clipboard instead of typing a "v".
            if ch == 'v' && key.modifiers.contains(KeyModifiers::CONTROL) {
                attach_clipboard_image(app);
                return;
            }
            // ctrl+o shows the output of tool calls, which is hidden by default:
            // the call lines are the story, the output is the footnote.
            if ch == 'o' && key.modifiers.contains(KeyModifiers::CONTROL) {
                app.show_tool_output = !app.show_tool_output;
                return;
            }
            // ctrl+r is the microphone: the first press starts dictation, the
            // second stops it.
            if ch == 'r' && key.modifiers.contains(KeyModifiers::CONTROL) {
                toggle_dictation(app, ctx);
                return;
            }
            // ctrl+g goes back to the newest message, ctrl+t to the oldest:
            // paging through a long conversation should not be the only way.
            if ch == 'g' && key.modifiers.contains(KeyModifiers::CONTROL) {
                app.thread_mut().scroll = 0;
                return;
            }
            if ch == 't' && key.modifiers.contains(KeyModifiers::CONTROL) {
                // Clamped to the transcript's own length when it is drawn.
                app.thread_mut().scroll = u16::MAX;
                return;
            }
            // Typing stays available while a turn runs, so a /btw side question
            // can be asked mid-task.
            let thread = app.thread_mut();
            thread.input.insert(thread.cursor, ch);
            thread.cursor += 1;
            thread.selected = 0;
            thread.popup_open = true;
            thread.history_index = None;
        }
        _ => {}
    }
}

/// A click that did not drag: do what the thing under it does. A picker row is
/// Enter on that row, a suggestion is Tab, the footer hint is `ctrl + /`, and a
/// link opens in the browser.
fn click_at(app: &mut App, ctx: &Ctx, column: u16, row: u16) {
    let Some(action) = app.click_at(column, row) else {
        return;
    };
    match action {
        app::ClickAction::PickerRow(index) => {
            if let Some(picker) = app.picker.as_mut() {
                picker.selected = index;
            }
            let scoped = app
                .picker
                .as_ref()
                .is_some_and(|picker| picker.kind == app::PickerKind::Scoped);
            if scoped {
                toggle_scoped(app, index);
            } else {
                select_from_picker(app, ctx);
            }
        }
        app::ClickAction::Suggestion(index) => {
            app.thread_mut().selected = index;
            app.accept_suggestion();
        }
        app::ClickAction::SideThread => {
            app.active = 1 - app.active;
            app.picker = None;
        }
        app::ClickAction::Link(url) => crate::oauth::open_in_browser(&url),
        app::ClickAction::Mic => toggle_dictation(app, ctx),
        app::ClickAction::Rewind { index, text } => {
            // Not straight to reverting: a click asks what to do with it.
            app.open_message_picker(index, text);
        }
    }
}

/// Goes back to a message: the conversation, what the model was told after it
/// and the file changes those turns made.
fn revert_to(app: &mut App, ctx: &Ctx, index: usize, text: String) {
    let conn = ctx.conn.clone();
    let session = ctx.session(0);
    let tx = ctx.events.clone();
    app.threads[0].error = None;
    tokio::spawn(async move {
        match conn.rewind(&session, index, &text).await {
            Ok(_) => {
                // The transcript is the server's now, note and all.
                if let Ok(snapshot) = conn.snapshot(&session).await {
                    let _ = tx.send(AgentMsg::Loaded(0, snapshot));
                }
                let _ = tx.send(AgentMsg::Rewound(text));
            }
            Err(err) => {
                let _ = tx.send(AgentMsg::Error(0, format!("{err:#}")));
            }
        }
    });
}

/// The microphone button: the first press starts the recognizer, the second
/// stops it and lets it flush the words still in flight.
fn toggle_dictation(app: &mut App, ctx: &Ctx) {
    if let Some(dictation) = app.dictation.take() {
        app.dictating_since = None;
        // Stopping waits for the recognizer to finish, so it does not block the
        // interface.
        tokio::spawn(async move { dictation.stop() });
        return;
    }
    let tx = ctx.events.clone();
    let ready = tx.clone();
    match crate::dictate::Dictation::start(
        move |text| {
            let _ = tx.send(AgentMsg::Dictation(text));
        },
        move || {
            let _ = ready.send(AgentMsg::DictationStopped);
        },
    ) {
        Ok(dictation) => {
            app.dictation = Some(dictation);
            app.dictating_since = Some(Instant::now());
        }
        Err(err) => {
            // The transcript is the server's; a local note would put the two
            // out of step, so failures go in the footer's error slot instead.
            app.threads[0].error = Some(format!("dictation did not start: {err:#}"));
        }
    }
}

/// The files named with `@` in a prompt, resolved against the working
/// directory. A name may be quoted (`@"my file.png"`) when it has spaces.
fn mentions(text: &str, cwd: &std::path::Path) -> Vec<String> {
    let mut found = Vec::new();
    let chars: Vec<char> = text.chars().collect();
    let mut index = 0;
    while index < chars.len() {
        if chars[index] != '@' {
            index += 1;
            continue;
        }
        // Only a word starting with @ counts, so mail@example.com is not one.
        let at_word_start = index == 0 || chars[index - 1].is_whitespace();
        if !at_word_start {
            index += 1;
            continue;
        }
        index += 1;
        let quoted = chars.get(index) == Some(&'"');
        if quoted {
            index += 1;
        }
        let mut name = String::new();
        while index < chars.len() {
            let ch = chars[index];
            if quoted {
                if ch == '"' {
                    index += 1;
                    break;
                }
            } else if ch.is_whitespace() {
                break;
            }
            name.push(ch);
            index += 1;
        }
        let name = name.trim();
        if name.is_empty() {
            continue;
        }
        let path = std::path::Path::new(name);
        let full = if path.is_absolute() {
            path.to_path_buf()
        } else {
            cwd.join(path)
        };
        found.push(full.to_string_lossy().to_string());
    }
    found
}

/// Rings the terminal bell and, when the desktop has one, shows a notification.
/// Both are best effort: a headless machine simply gets the bell.
fn ring_attention() {
    use std::io::Write;
    let mut out = std::io::stdout();
    let _ = out.write_all(b"\x07");
    let _ = out.flush();

    // Every branch compiles everywhere; `cfg!` only picks which one runs.
    let mut command = if cfg!(target_os = "macos") {
        let mut command = std::process::Command::new("osascript");
        command.args([
            "-e",
            "display notification \"the turn has finished\" with title \"exp-1 agent\"",
        ]);
        command
    } else if cfg!(target_os = "windows") {
        let mut command = std::process::Command::new("powershell");
        command.args([
            "-NoProfile",
            "-Command",
            "Add-Type -AssemblyName System.Windows.Forms; \
             $n = New-Object System.Windows.Forms.NotifyIcon; \
             $n.Icon = [System.Drawing.SystemIcons]::Information; $n.Visible = $true; \
             $n.ShowBalloonTip(5000, 'exp-1 agent', 'the turn has finished', 'Info')",
        ]);
        command
    } else {
        let mut command = std::process::Command::new("notify-send");
        command.args(["exp-1 agent", "the turn has finished"]);
        command
    };
    let _ = command
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .spawn();
}

/// `/mcp` output: one line per server, with what went wrong when it did.
fn render_mcp(servers: &[serde_json::Value]) -> String {
    if servers.is_empty() {
        return format!(
            "no MCP servers configured — add them to {}",
            store::mcp_path().display()
        );
    }
    servers
        .iter()
        .map(|server| {
            let name = server.get("name").and_then(|v| v.as_str()).unwrap_or("?");
            let kind = server.get("kind").and_then(|v| v.as_str()).unwrap_or("?");
            match server.get("error").and_then(|v| v.as_str()) {
                Some(error) => format!("{name} ({kind}): unavailable — {error}"),
                None => {
                    let tools = server.get("tools").and_then(|v| v.as_u64()).unwrap_or(0);
                    format!("{name} ({kind}): {tools} tool(s)")
                }
            }
        })
        .collect::<Vec<_>>()
        .join("\n")
}

fn thread_name(prompt: &str) -> String {
    let first = prompt.lines().next().unwrap_or(prompt).trim();
    let mut name: String = first.chars().take(32).collect();
    if first.chars().count() > 32 {
        name.push('…');
    }
    name
}

fn apply(app: &mut App, message: AgentMsg) {
    let (index, update): (usize, Box<dyn FnOnce(&mut App)>) = match message {
        AgentMsg::Phase(index, phase) => (index, Box::new(move |app: &mut App| {
            // A turn that ran long enough for the user to walk away gets a bell
            // and, where the desktop has one, a notification.
            if phase == Phase::Idle
                && app.threads[index]
                    .started_at
                    .is_some_and(|start| start.elapsed() >= Duration::from_secs(15))
            {
                app.attention = true;
                app.threads[index].started_at = None;
            }
            let thread = &mut app.threads[index];
            if phase == Phase::Streaming {
                thread.live.clear();
            }
            thread.phase = phase;
        })),
        AgentMsg::Delta(index, delta) => (index, Box::new(move |app: &mut App| {
            app.threads[index].live.push_str(&delta);
        })),
        AgentMsg::Reasoning(index, text) => (index, Box::new(move |app: &mut App| {
            app.threads[index].reasoning.push_str(&text);
        })),
        AgentMsg::AssistantDone(index, text) => (index, Box::new(move |app: &mut App| {
            let thread = &mut app.threads[index];
            thread.push(Role::Assistant, text);
            thread.live.clear();
            thread.reasoning.clear();
        })),
        AgentMsg::ToolLine(index, text, kind) => (index, Box::new(move |app: &mut App| {
            let thread = &mut app.threads[index];
            thread.live.clear();
            match kind {
                // A subagent's step replaces the previous one.
                WireMsgKind::Progress => thread.push_progress(text),
                _ => thread.push_kind(Role::Tool, text, kind),
            }
        })),
        AgentMsg::Usage(index, usage) => (index, Box::new(move |app: &mut App| {
            app.threads[index].usage = Some(usage);
        })),
        AgentMsg::Error(index, message) => (index, Box::new(move |app: &mut App| {
            let thread = &mut app.threads[index];
            thread.error = Some(message);
            thread.live.clear();
        })),
        AgentMsg::Done(index, label) => (index, Box::new(move |app: &mut App| {
            app.threads[index].push(Role::System, label);
        })),
        AgentMsg::Loaded(index, snapshot) => (index, Box::new(move |app: &mut App| {
            let thread = &mut app.threads[index];
            thread.clear_transcript();
            thread.error = snapshot.error.clone();
            thread.usage = snapshot.usage.as_ref().map(|usage| Usage {
                input_tokens: usage.input_tokens,
                output_tokens: usage.output_tokens,
                cached_tokens: usage.cached_tokens,
            });
            thread.phase = match snapshot.phase {
                proto::WirePhase::Idle => Phase::Idle,
                _ => Phase::Streaming,
            };
            for message in &snapshot.transcript {
                let role = match message.role.as_str() {
                    "user" => Role::User,
                    "assistant" => Role::Assistant,
                    "tool" => Role::Tool,
                    _ => Role::System,
                };
                if role == Role::User {
                    thread.push_user(message.text.clone(), message.images);
                } else {
                    thread.push_kind(role, message.text.clone(), message.kind);
                }
            }
            if !snapshot.name.is_empty() {
                thread.name = snapshot.name.clone();
            }
            if !snapshot.mode.is_empty() {
                app.mode = snapshot.mode.clone();
            }
            if !snapshot.title.is_empty() {
                app.session_title = snapshot.title.clone();
            }
        })),
        AgentMsg::Rewound(text) => (0, Box::new(move |app: &mut App| {
            // The server has already cut the conversation and put the files
            // back; the prompt returns so it can be edited and sent again.
            let thread = &mut app.threads[0];
            thread.input = text.chars().collect();
            thread.cursor = thread.input.len();
            thread.popup_open = false;
            thread.error = None;
            thread.scroll = 0;
        })),
        AgentMsg::Dictation(text) => (0, Box::new(move |app: &mut App| {
            let thread = &mut app.threads[0];
            // Words arrive one committed piece at a time; they belong in the
            // prompt with a space between pieces.
            let needs_space = thread
                .input
                .last()
                .is_some_and(|ch| !ch.is_whitespace());
            if needs_space {
                thread.input.push(' ');
            }
            thread.input.extend(text.chars());
            thread.cursor = thread.input.len();
        })),
        AgentMsg::DictationStopped => (0, Box::new(move |app: &mut App| {
            app.dictation = None;
            app.dictating_since = None;
        })),
        AgentMsg::Catalog(list) => (0, Box::new(move |app: &mut App| {
            app.all_models = list.clone();
        })),
        AgentMsg::Skills(list) => (0, Box::new(move |app: &mut App| {
            app.skills = list.clone();
        })),
        AgentMsg::Cwd(path) => (0, Box::new(move |app: &mut App| {
            app.cwd = path.clone();
            app.threads[0].push(Role::System, format!("cwd: {path}"));
        })),
        AgentMsg::Title(index, title) => (index, Box::new(move |app: &mut App| {
            // The generated title replaces the placeholder taken from the first
            // prompt, in the footer and in the picker alike.
            app.session_title = title.clone();
            if let Some(thread) = app.threads.get_mut(index) {
                thread.name = title;
            }
        })),
        AgentMsg::Attached(index, name) => (index, Box::new(move |app: &mut App| {
            let thread = &mut app.threads[index];
            thread.clear_transcript();
            thread.usage = None;
            thread.error = None;
            thread.phase = Phase::Idle;
            thread.name = name;
        })),
        AgentMsg::Providers(list) => (0, Box::new(move |app: &mut App| {
            app.providers = list;
            app.open_provider_picker();
        })),
        AgentMsg::Models(list) => (0, Box::new(move |app: &mut App| {
            // The scope is only changed by ticking; opening the picker must not
            // rewrite it, or a provider's entry would stamp out another's.
            app.all_models = list;
            app.open_scoped_picker();
        })),
        AgentMsg::ProviderChanged(label, model, effort, message) => (0, Box::new(move |app: &mut App| {
            app.provider = label.clone();
            app.model = model.clone();
            app.effort = effort.clone();
            app.entering_key = None;
            app.threads[0].push(Role::System, message.clone());
        })),
        AgentMsg::Ask(index, request) => (index, Box::new(move |app: &mut App| {
            app.active = index;
            app.asking = Some(AskView {
                questions: request
                    .questions
                    .into_iter()
                    .map(|question| tools::AskItem {
                        label: question.label,
                        text: question.text,
                        options: question.options,
                    })
                    .collect(),
                submit_label: request.submit_label,
                index: 0,
                selected: 0,
                typed: false,
            });
            app.thread_mut().clear_input();
        })),
        AgentMsg::AskClosed(index) => (index, Box::new(move |app: &mut App| {
            let _ = index;
            app.asking = None;
        })),
        AgentMsg::Idle(index) => (index, Box::new(move |app: &mut App| {
            let thread = &mut app.threads[index];
            thread.phase = Phase::Idle;
            thread.live.clear();
            thread.started_at = None;
        })),
    };
    if index < app.threads.len() {
        update(app);
    }
}

async fn service_command(rest: &[String]) -> Result<()> {
    match rest.first().map(String::as_str) {
        Some("start") => {
            let url = daemon::ensure_running().await?;
            println!("exp-1 server running at {url}");
        }
        Some("stop") => match daemon::read_registration() {
            Some(registration) => {
                daemon::stop(&registration);
                println!("stopped server {}", registration.pid);
            }
            None => println!("no server is registered"),
        },
        Some("status") => match daemon::read_registration() {
            Some(registration) => {
                let alive = daemon::is_healthy(&registration).await;
                let current = daemon::build_id();
                let build = registration.build.as_deref().unwrap_or("unknown");
                let fits = if registration.build == current {
                    "matches this binary"
                } else {
                    "was replaced since; a launch would restart it"
                };
                println!(
                    "pid {}\nurl {}\nversion {}\nbuild {} ({fits})\nhealthy {}",
                    registration.pid,
                    registration.url,
                    registration.version,
                    build,
                    if alive { "yes" } else { "no" }
                );
            }
            None => println!("no server is registered"),
        },
        Some("restart") => {
            if let Some(registration) = daemon::read_registration() {
                daemon::stop(&registration);
            }
            let url = daemon::ensure_running().await?;
            println!("exp-1 server running at {url}");
        }
        Some("password") => match rest.get(1) {
            Some(value) => {
                daemon::set_password(value)?;
                println!("password updated");
            }
            None => println!("{}", daemon::password()),
        },
        other => {
            anyhow::bail!("unknown service command {other:?} (start|stop|status|restart|password)")
        }
    }
    Ok(())
}

#[tokio::main]
async fn main() -> Result<()> {
    install_panic_hook();
    let argv: Vec<String> = std::env::args().collect();
    match argv.get(1).map(String::as_str) {
        Some("serve") => {
            let register = argv.iter().any(|value| value == "--register");
            let port = argv
                .iter()
                .position(|value| value == "--port")
                .and_then(|index| argv.get(index + 1))
                .and_then(|value| value.parse::<u16>().ok());
            return daemon::run_foreground(register, port).await;
        }
        Some("service") => return service_command(&argv[2..]).await,
        // `exp-1 install <dir|file.md|archive|url>` puts a skill where `/<name>`
        // can find it.
        Some("install") => {
            let Some(source) = argv.get(2) else {
                anyhow::bail!("usage: exp-1 install <directory|file.md|.zip|.tar.gz|url>");
            };
            let skill = skills::install(source).await?;
            println!("installed {} → {}", skill.name, skill.path.display());
            if !skill.description.is_empty() {
                println!("  {}", skill.description);
            }
            println!("invoke it with /{}", skill.name);
            return Ok(());
        }
        // The registry is the source of truth for the README table, so the
        // binary prints it rather than a script trying to parse Rust.
        Some("--providers") => {
            for provider in providers::PROVIDERS {
                let mut wires: Vec<&str> = Vec::new();
                for (_, wire) in provider.models {
                    let name = match wire {
                        providers::Wire::OpenAiChat => "OpenAI",
                        providers::Wire::Anthropic => "Anthropic",
                        providers::Wire::OpenAiResponses => "Responses",
                        providers::Wire::CodexResponses => "Codex",
                        providers::Wire::Gemini => "Gemini",
                    };
                    if !wires.contains(&name) {
                        wires.push(name);
                    }
                }
                println!(
                    "| {} | `{}` | {} |",
                    provider.label,
                    provider.base_url.replace("https://", ""),
                    wires.join(" + ")
                );
            }
            return Ok(());
        }
        _ => {}
    }

    let args = parse_args()?;

    // A normal launch brings the shared server up and attaches to it, so every
    // later launch reuses a warm process instead of paying for a cold start.
    let conn = match client::Connection::open().await {
        Ok(conn) => Arc::new(conn),
        Err(err) => {
            eprintln!("exp-1: cannot reach the background server: {err:#}");
            std::process::exit(1);
        }
    };

    let (main_session, main_events) = conn.create_session(&args.cwd, None).await?;
    // The side chat follows the main one, so its questions see everything the
    // main conversation has said without adding to it.
    let (side_session, side_events) = conn
        .create_session(&args.cwd, Some(&main_session))
        .await?;

    let (event_tx, mut event_rx) = mpsc::unbounded_channel();
    pump_events(0, main_events, event_tx.clone());
    pump_events(1, side_events, event_tx.clone());

    // The catalogue is needed for the context meter long before `/model` is
    // ever opened, so it is fetched with the skills.
    let catalog_conn = conn.clone();
    let catalog_tx = event_tx.clone();
    tokio::spawn(async move {
        if let Ok(list) = catalog_conn.models().await {
            let _ = catalog_tx.send(AgentMsg::Catalog(list));
        }
    });

    // Installed skills become commands beside the built-in ones, in the popup
    // and in the empty-state list.
    let skills_conn = conn.clone();
    let skills_tx = event_tx.clone();
    tokio::spawn(async move {
        if let Ok(list) = skills_conn.skills().await {
            let named: Vec<(String, String)> = list
                .into_iter()
                .map(|(name, description)| (format!("/{name}"), description))
                .collect();
            let _ = skills_tx.send(AgentMsg::Skills(named));
        }
    });

    // A machine that has never saved a conversation gets the full command list
    // once; afterwards the empty screen stays out of the way.
    let ctx = Ctx {
        conn: conn.clone(),
        cwd: Arc::new(Mutex::new(args.cwd.clone())),
        sessions: Arc::new(Mutex::new([main_session, side_session])),
        events: event_tx,
    };

    // Command-line switches win, and go straight to the server.
    if args.model.is_some() || args.effort.is_some() {
        let _ = conn.configure(args.model.clone(), args.effort.clone()).await;
    }

    // The picker draws every provider's models; the scope decides what it
    // offers, and comes with the config so it is right from the first use.
    let models = conn.models_all().await.unwrap_or_default();
    let catalog: Vec<catalog::ModelInfo> = models
        .into_iter()
        .map(|model| catalog::ModelInfo {
            slug: model.slug,
            context_window: model.context_window,
            display_name: model.display_name,
            description: model.description,
            efforts: model
                .efforts
                .into_iter()
                .map(|entry| (entry.effort, entry.description))
                .collect(),
            default_effort: model.default_effort,
            listed: true,
        })
        .collect();
    let active = conn.config().await.ok();

    let settings = store::load_settings();
    let theme_index = THEMES
        .iter()
        .position(|theme| theme.name == settings.theme)
        .unwrap_or(0);

    let empty = || Arc::new(Mutex::new(Vec::new()));
    let mut app = App::new(
        args.cwd.to_string_lossy().to_string(),
        empty(),
        empty(),
    );
    app.catalog = catalog;
    app.theme = theme_index;
    // A machine that has never saved a conversation gets the full command list
    // once; afterwards the empty screen stays out of the way.
    app.first_run = store::list_sessions().is_empty();
    if let Some(active) = active {
        app.model = active.model;
        app.effort = active.effort;
        app.plan = active.plan;
        app.scoped = active.scoped.into_iter().collect();
    }

    enable_raw_mode()?;
    let mut out = stdout();
    execute!(out, EnterAlternateScreen)?;
    execute!(out, EnableMouseCapture)?;
    let _ = execute!(
        out,
        PushKeyboardEnhancementFlags(KeyboardEnhancementFlags::DISAMBIGUATE_ESCAPE_CODES)
    );
    let mut terminal = Terminal::new(CrosstermBackend::new(out))?;

    let outcome = event_loop(&mut terminal, &mut app, &ctx, &mut event_rx);

    let _ = execute!(terminal.backend_mut(), DisableMouseCapture);
    let _ = execute!(terminal.backend_mut(), PopKeyboardEnhancementFlags);
    disable_raw_mode()?;
    execute!(terminal.backend_mut(), LeaveAlternateScreen)?;
    terminal.show_cursor()?;

    outcome
}

/// Writes a panic where it can be found, and leaves the terminal usable: a
/// panic in raw mode otherwise looks exactly like a frozen screen, with every
/// keystroke going nowhere.
fn install_panic_hook() {
    let state = daemon::state_dir();
    std::panic::set_hook(Box::new(move |info| {
        let _ = disable_raw_mode();
        let _ = execute!(
            std::io::stdout(),
            DisableMouseCapture,
            crossterm::cursor::Show,
            LeaveAlternateScreen
        );
        let location = info
            .location()
            .map(|at| format!("{}:{}:{}", at.file(), at.line(), at.column()))
            .unwrap_or_else(|| "unknown".to_string());
        let message = info
            .payload()
            .downcast_ref::<&str>()
            .map(|text| text.to_string())
            .or_else(|| info.payload().downcast_ref::<String>().cloned())
            .unwrap_or_else(|| "panic".to_string());
        let _ = std::fs::create_dir_all(&state);
        let path = state.join("crash.log");
        if let Ok(mut file) = std::fs::OpenOptions::new()
            .create(true)
            .append(true)
            .open(&path)
        {
            use std::io::Write;
            let _ = writeln!(
                file,
                "--- {} ---\n{message}\n  at {location}\n",
                store::now_iso()
            );
        }
        // The screen is back to the shell by now, so the reason is readable
        // instead of the window simply looking frozen.
        eprintln!(
            "\nexp-1 agent stopped: {message}\n  at {location}\n  written to {}\n",
            path.display()
        );
    }));
}

/// Anything that takes noticeably long lands in `slow.log`, so a freeze that
/// cannot be reproduced still leaves a trace of what was slow.
fn note_slow(what: &str, took: std::time::Duration) {
    if took < std::time::Duration::from_millis(750) {
        return;
    }
    let state = daemon::state_dir();
    let _ = std::fs::create_dir_all(&state);
    if let Ok(mut file) = std::fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(state.join("slow.log"))
    {
        use std::io::Write;
        let _ = writeln!(file, "{} {what}: {took:?}", store::now_iso());
    }
}

/// Takes an image off the clipboard and holds it for the next prompt. Wayland
/// first, then X11; a missing tool is simply "nothing to paste".
fn attach_clipboard_image(app: &mut App) {
    use base64::Engine;
    match clipboard_image() {
        Some((media_type, bytes)) => {
            let size = if bytes.len() >= 1024 {
                format!("{} KB", bytes.len() / 1024)
            } else {
                format!("{} bytes", bytes.len())
            };
            let data = base64::engine::general_purpose::STANDARD.encode(&bytes);
            app.pending_images.push((media_type.clone(), data));
            let queued = app.pending_images.len();
            app.thread_mut().push(
                Role::System,
                format!("image attached: {media_type}, {size} — {queued} queued for the next prompt"),
            );
        }
        None => app.thread_mut().push(
            Role::System,
            "no image in the clipboard (nothing copied, or wl-paste/xclip is missing)",
        ),
    }
}

/// The raw bytes of the clipboard's first image, with its media type.
fn clipboard_image() -> Option<(String, Vec<u8>)> {
    // Each platform has its own way to the clipboard, and the body for one is
    // the whole function: only one of these blocks ever compiles.
    #[cfg(target_os = "macos")]
    {
        // macOS has no built-in CLI for images on the clipboard; `pngpaste` is
        // the tool people install for exactly this.
        match std::process::Command::new("pngpaste").arg("-").output() {
            Ok(out) if out.status.success() && !out.stdout.is_empty() => {
                Some(("image/png".to_string(), out.stdout))
            }
            _ => None,
        }
    }
    #[cfg(target_os = "windows")]
    {
        // PowerShell writes the clipboard image to a file, which is then read
        // back and deleted.
        let path = std::env::temp_dir().join(format!("exp1-clipboard-{}.png", std::process::id()));
        let script = format!(
            "Add-Type -AssemblyName System.Windows.Forms; \
             $img = [System.Windows.Forms.Clipboard]::GetImage(); \
             if ($img) {{ $img.Save('{}', [System.Drawing.Imaging.ImageFormat]::Png) }}",
            path.display()
        );
        let saved = std::process::Command::new("powershell")
            .args(["-NoProfile", "-Command", &script])
            .output()
            .map(|out| out.status.success())
            .unwrap_or(false);
        let bytes = if saved {
            std::fs::read(&path).ok().filter(|bytes| !bytes.is_empty())
        } else {
            None
        };
        let _ = std::fs::remove_file(&path);
        bytes.map(|bytes| ("image/png".to_string(), bytes))
    }
    #[cfg(all(unix, not(target_os = "macos")))]
    {
        // Wayland: ask which types are offered, then take the first image one.
        if let Ok(out) = std::process::Command::new("wl-paste")
            .arg("--list-types")
            .output()
            && out.status.success()
        {
            let types = String::from_utf8_lossy(&out.stdout);
            if let Some(media) = types
                .lines()
                .map(str::trim)
                .find(|line| line.starts_with("image/"))
                && let Ok(bytes) = std::process::Command::new("wl-paste")
                    .args(["--no-newline", "--type", media])
                    .output()
                && bytes.status.success()
                && !bytes.stdout.is_empty()
            {
                return Some((media.to_string(), bytes.stdout));
            }
        }
        // X11, for the sessions that are not Wayland.
        for media in ["image/png", "image/jpeg"] {
            if let Ok(out) = std::process::Command::new("xclip")
                .args(["-selection", "clipboard", "-t", media, "-o"])
                .output()
                && out.status.success()
                && !out.stdout.is_empty()
            {
                return Some((media.to_string(), out.stdout));
            }
        }
        None
    }
}

/// Puts text on the clipboard. OSC 52 hands it to the terminal itself, which
/// is the only route that also works over ssh; the desktop tools are tried
/// after that so a local session gets it even where OSC 52 is ignored. A
/// missing tool is not a failure: the other routes still count.
fn copy_to_clipboard(text: &str) -> bool {
    use base64::Engine;
    use std::io::Write;

    let mut delivered = false;
    let encoded = base64::engine::general_purpose::STANDARD.encode(text);
    let mut out = std::io::stdout();
    if write!(out, "\x1b]52;c;{encoded}\x07").is_ok() && out.flush().is_ok() {
        delivered = true;
    }

    let candidates: [(&str, &[&str]); 3] = [
        ("wl-copy", &[]),
        ("xclip", &["-selection", "clipboard"]),
        ("xsel", &["-ib"]),
    ];
    for (program, args) in candidates {
        let Ok(mut child) = std::process::Command::new(program)
            .args(args)
            .stdin(std::process::Stdio::piped())
            .stdout(std::process::Stdio::null())
            .stderr(std::process::Stdio::null())
            .spawn()
        else {
            continue;
        };
        if let Some(stdin) = child.stdin.as_mut() {
            let _ = stdin.write_all(text.as_bytes());
        }
        drop(child.stdin.take());
        // Reaped off the event loop, so no zombie lingers in a long session.
        std::thread::spawn(move || {
            let _ = child.wait();
        });
        delivered = true;
        break;
    }
    delivered
}

fn event_loop(
    terminal: &mut Terminal<CrosstermBackend<std::io::Stdout>>,
    app: &mut App,
    ctx: &Ctx,
    events: &mut mpsc::UnboundedReceiver<AgentMsg>,
) -> Result<()> {
    loop {
        let drawn_at = Instant::now();
        terminal.draw(|frame| app.render(frame))?;
        note_slow("draw", drawn_at.elapsed());

        // Drain every queued event per tick so fast typing is never dropped
        // behind the redraw. Scrolling is on the wheel; the arrows drive the
        // completion popup or the prompt history.
        if event::poll(Duration::from_millis(60))? {
            let keys_at = Instant::now();
            loop {
                match event::read()? {
                    Event::Key(key) if key.kind == KeyEventKind::Press => {
                        handle_key(app, key, ctx)
                    }
                    // A paste from the terminal arrives whole, not key by key.
                    Event::Paste(text) => {
                        let thread = app.thread_mut();
                        for ch in text.chars() {
                            thread.input.insert(thread.cursor, ch);
                            thread.cursor += 1;
                        }
                    }
                    Event::Mouse(mouse) => match mouse.kind {
                        MouseEventKind::ScrollUp => {
                            let thread = app.thread_mut();
                            thread.scroll = thread.scroll.saturating_add(3);
                        }
                        MouseEventKind::ScrollDown => {
                            let thread = app.thread_mut();
                            thread.scroll = thread.scroll.saturating_sub(3);
                        }
                        // Dragging anywhere selects; letting go copies. A plain
                        // click selects one cell and copies nothing.
                        MouseEventKind::Down(MouseButton::Left) => {
                            app.begin_selection(mouse.column, mouse.row);
                        }
                        MouseEventKind::Drag(MouseButton::Left) => {
                            app.extend_selection(mouse.column, mouse.row);
                        }
                        MouseEventKind::Up(MouseButton::Left) => {
                            let dragged = app
                                .ordered_selection()
                                .is_some_and(|(anchor, head)| anchor != head);
                            if dragged && let Some(text) = app.selected_text() {
                                let chars = text.chars().count();
                                if copy_to_clipboard(&text) {
                                    app.copied =
                                        Some((format!("copied {chars} chars"), Instant::now()));
                                }
                            } else if !dragged {
                                // A click that did not drag acts on whatever it
                                // landed on.
                                click_at(app, ctx, mouse.column, mouse.row);
                            }
                            app.selection = None;
                        }
                        _ => {}
                    },
                    _ => {}
                }
                if app.quit || !event::poll(Duration::ZERO)? {
                    break;
                }
            }
            note_slow("input", keys_at.elapsed());
        }

        if app.attention {
            app.attention = false;
            ring_attention();
        }
        app.tick = app.tick.wrapping_add(1);
        let events_at = Instant::now();
        while let Ok(message) = events.try_recv() {
            apply(app, message);
        }
        note_slow("events", events_at.elapsed());

        if app.quit {
            let mut settings = store::load_settings();
            settings.model = app.model.clone();
            settings.effort = app.effort.clone();
            settings.theme = app.theme().name.to_string();
            settings.provider = app.provider.clone();
            store::save_settings(&settings);
            return Ok(());
        }
    }
}