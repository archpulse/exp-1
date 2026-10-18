//! The background server: one process owns every session, every client attaches
//! to it over HTTP. Modelled on opencode's `serve`.

use anyhow::{Context, Result};
use axum::Router;
use axum::routing::{delete, get, post};
use std::path::PathBuf;
use std::sync::{Arc, Mutex};
use tokio::net::TcpListener;

use crate::catalog;
use crate::daemon;
use crate::tools;
use crate::store;

/// Tool rounds in one turn. Real work on a large project runs into the dozens;
/// the limit is a guard against a loop that never converges, not a budget.
const MAX_ITERATIONS: usize = 200;
/// Characters of a subagent's report the parent keeps; anything beyond is cut,
/// because the report lands in the parent's context.
const SUBAGENT_REPORT_CHARS: usize = 6_000;
/// How deep `task` may recurse before subagents stop being offered.
const MAX_SUBAGENT_DEPTH: usize = 2;
/// Binds the listening socket so the caller can register the real port.
pub async fn bind(port: Option<u16>) -> Result<TcpListener> {
    let address = format!("127.0.0.1:{}", port.unwrap_or(0));
    TcpListener::bind(&address)
        .await
        .with_context(|| format!("bind {address}"))
}

pub async fn serve(listener: TcpListener, port: u16) -> Result<()> {
    // Older builds kept conversations beside the configuration; move them into
    // the cache before anything reads, and keep that cache bounded.
    store::migrate();
    store::trim_cache(store::CACHE_LIMIT_BYTES);
    // Every prompt gets its file up front, so there is something to edit
    // before the role is first used.
    for (name, builtin) in [
        ("system", INSTRUCTIONS),
        ("subagent", SUBAGENT_INSTRUCTIONS),
        ("compaction", COMPACTION_PROMPT),
        ("title", TITLE_PROMPT),
        ("plan", PLAN_NOTE),
    ] {
        let _ = store::prompt(name, builtin);
    }
    let agent = Agent {
        codex: Mutex::new(None),
        provider: Mutex::new(
            store::load_settings()
                .provider
                .is_empty()
                .then(|| crate::providers::DEFAULT_PROVIDER.to_string())
                .unwrap_or_else(|| store::load_settings().provider),
        ),
        model: Mutex::new(crate::catalog::load().iter().find(|m| m.listed).map(|m| m.slug.clone()).unwrap_or_default()),
        effort: Mutex::new("medium".to_string()),
        registry: Arc::new(tokio::sync::Mutex::new(tools::Registry::new())),
        cwd: std::env::current_dir().unwrap_or_else(|_| PathBuf::from(".")),
    };

    // Sensible starting values from the saved settings, without ever resuming.
    let mut settings = store::load_settings();
    if !settings.model.is_empty() {
        if let Ok(mut model) = agent.model.lock() {
            *model = settings.model.clone();
        }
    }
    if let Ok(mut effort) = agent.effort.lock() {
        *effort = if settings.effort.is_empty() {
            "medium".to_string()
        } else {
            settings.effort.clone()
        };
    }

    // Only listed models, with their own efforts, so the UI can offer the right list.
    let catalog = catalog::load();
    if let Some(first) = catalog.iter().find(|m| m.listed)
        && let Ok(mut model) = agent.model.lock()
        && model.is_empty()
    {
        *model = first.slug.clone();
    }

    // The saved pair can be out of step — a model from one provider saved while
    // another was in use, which makes the first request go out on a provider
    // that has never heard of it. The model decides, here as in `/config`.
    {
        let stored_model = agent.model.lock().map(|model| model.clone()).unwrap_or_default();
        let stored_provider = agent.provider.lock().map(|provider| provider.clone()).unwrap_or_default();
        if !stored_model.is_empty()
            && crate::providers::by_id(&stored_provider)
                .is_some_and(|provider| !offers(provider, &stored_model))
        {
            match owner_of(&stored_model) {
                Some(owner) => {
                    if let Ok(mut provider) = agent.provider.lock() {
                        *provider = owner.id.to_string();
                    }
                    settings.provider = owner.id.to_string();
                    let _ = store::save_settings(&settings);
                }
                None => {
                    // Nobody signed in offers it: keep the provider, drop the
                    // model for its first, so a turn can run.
                    if let Some(first) = crate::providers::by_id(&stored_provider)
                        .and_then(|provider| provider.models.first())
                        && let Ok(mut model) = agent.model.lock()
                    {
                        *model = first.0.to_string();
                        settings.model = agent.model.lock().map(|model| model.clone()).unwrap_or_default();
                        let _ = store::save_settings(&settings);
                    }
                }
            }
        }
    }

    let plan = agent.codex().ok().and_then(|client| client.plan());
    // MCP servers are reached in the background: a slow or dead one must not
    // hold up the agent, and its tools appear once it answers.
    let mcp = Arc::new(crate::mcp::Registry::default());
    let mcp_loading = mcp.clone();
    tokio::spawn(async move {
        mcp_loading.fill().await;
    });
    let restored = restore_sessions();
    let catalog = catalog::load();
    let shared = Arc::new(Shared {
        password: daemon::password(),
        agent: Arc::new(agent),
        catalog,
        mcp,
        plan: Mutex::new(plan),
        sessions: Mutex::new(restored),
        scheduler: Scheduler::load(),
    });

    // The clock behind `/schedule`: it reports and catches up on what was
    // missed while nothing was running, then keeps watching the list.
    spawn_scheduler(shared.clone());

    // The model list is fetched, not baked in: bring it up to date in the
    // background so the first picker of the day is current.
    {
        let shared = shared.clone();
        tokio::spawn(async move {
            let http = shared.agent.registry.lock().await.http();
            crate::models_dev::refresh_if_stale(&http).await;
        });
    }

    let app = Router::new()
        .route("/health", get(health))
        .route("/", get(web_page))
        .route("/assets/{file}", get(web_asset))
        .route("/api/config", get(web_config).post(web_config_save))
        .route("/api/sessions", get(web_sessions))
        .route("/api/schedules", get(web_schedules))
        .route("/api/schedules/{id}/{action}", post(web_schedule_action))
        .route("/api/schedules/{id}", delete(web_schedule_remove))
        .route("/api/plugins", get(web_plugins))
        .route("/sessions", get(list_sessions).post(create_session))
        .route("/sessions/{id}", get(session_snapshot))
        .route("/sessions/{id}/prompt", post(prompt))
        .route("/sessions/{id}/answer", post(answer))
        .route("/sessions/{id}/interrupt", post(interrupt))
        .route("/sessions/{id}/events", get(events))
        .route("/sessions/{id}/retitle", post(retitle))
        .route("/sessions/{id}/compact", post(compact_now))
        .route("/sessions/{id}/undo", post(undo))
        .route("/sessions/{id}/rewind", post(rewind))
        .route("/sessions/{id}/fork", post(fork_session))
        .route("/sessions/{id}/mode", post(set_mode))
        .route("/sessions/{id}/permissions", post(set_permissions))
        .route("/sessions/{id}/clear", post(clear_session))
        .route("/sessions/{id}/diff", post(show_diff))
        .route("/mcp", get(list_mcp))
        .route("/sessions/{id}/cd", post(cd))
        .route("/sessions/{id}/skill", post(run_skill))
        .route("/skills", get(list_skills))
        .route("/models", get(models))
        .route("/config", get(get_config).post(config))
        .route("/providers", get(list_providers))
        .route("/login", post(login))
        .route("/scoped-models", post(scoped_models))
        .route("/schedules", get(list_schedules))
        .route("/schedules/{id}", delete(delete_schedule))
        .route("/schedules/{id}/{action}", post(schedule_action))
        .with_state(shared.clone());

    eprintln!("exp-1 server listening on http://127.0.0.1:{port}");
    axum::serve(listener, app).await.context("serve")?;
    Ok(())
}

mod prompts;
mod subagent;
mod turn;
mod history;
mod session;
mod shown;
mod permissions;
mod titles;
mod gateway;
mod routes;
mod scheduler;
mod web;

pub(crate) use self::{prompts::*, subagent::*, turn::*, history::*, session::*, shown::*, permissions::*, titles::*, gateway::*, routes::*, scheduler::{act, jobs_json, remove, schedule_tool, spawn_scheduler, Scheduler}, web::{asset as web_asset, config_get as web_config, config_post as web_config_save, page as web_page, plugins as web_plugins, schedule_action as web_schedule_action, schedule_remove as web_schedule_remove, schedules as web_schedules, sessions as web_sessions}};

#[cfg(test)]
mod tests;
