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
#[cfg(feature = "titler")]
pub(crate) mod titler;

use anyhow::{ Result };
use crossterm::event::{
    self, DisableBracketedPaste, DisableMouseCapture, EnableBracketedPaste, EnableMouseCapture,
    Event, KeyEventKind, KeyboardEnhancementFlags, MouseButton, MouseEventKind,
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
    // The permission mode survives a restart; the server seeds new sessions
    // from the same setting, so both sides start out agreeing.
    app.permissions = settings.permissions;
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
    // Bracketed paste is what keeps a pasted block one event: without it the
    // terminal sends it key by key, its newlines arrive as Enter, and the first
    // line is sent before the rest has been typed in.
    execute!(out, EnableMouseCapture, EnableBracketedPaste)?;
    let _ = execute!(
        out,
        PushKeyboardEnhancementFlags(KeyboardEnhancementFlags::DISAMBIGUATE_ESCAPE_CODES)
    );
    let mut terminal = Terminal::new(CrosstermBackend::new(out))?;

    let outcome = event_loop(&mut terminal, &mut app, &ctx, &mut event_rx);

    let _ = execute!(
        terminal.backend_mut(),
        DisableMouseCapture,
        DisableBracketedPaste
    );
    let _ = execute!(terminal.backend_mut(), PopKeyboardEnhancementFlags);
    disable_raw_mode()?;
    execute!(terminal.backend_mut(), LeaveAlternateScreen)?;
    terminal.show_cursor()?;

    outcome
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

mod cli;
mod commands;
mod desktop;
mod keys;
mod messages;

use self::{cli::*, commands::*, desktop::*, keys::*, messages::*};
