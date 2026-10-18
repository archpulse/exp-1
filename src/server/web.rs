//! the web page: a chat-like shell served on localhost, beside the TUI.
//!
//! It lives on the same port and in the same process as everything else, so
//! opening the agent opens the page. The pages, styles and script travel inside
//! the binary: no build step, no network, nothing to install.

use axum::extract::{Path, Query, State};
use axum::http::{header, HeaderMap, HeaderValue, StatusCode};
use axum::response::{Html, IntoResponse, Json, Response};
use std::collections::HashMap;
use std::sync::Arc;

use crate::store;

use super::*;

/// The page, its styles and its script, straight from the binary.
const INDEX: &str = include_str!("../web/index.html");
const APP_CSS: &str = include_str!("../web/app.css");
const APP_JS: &str = include_str!("../web/app.js");

/// Where the boot state goes in `index.html`.
const BOOT: &str = "__EXP1_BOOT__";

/// A browser cannot send our bearer header, so the page takes the same password
/// once in the URL and keeps it as a cookie from then on.
pub(crate) fn web_authorized(shared: &Shared, headers: &HeaderMap) -> bool {
    if routes::authorized(shared, headers) {
        return true;
    }
    headers
        .get(header::COOKIE)
        .and_then(|value| value.to_str().ok())
        .is_some_and(|cookies| {
            cookies.split(';').any(|cookie| {
                cookie
                    .trim()
                    .strip_prefix("exp1_key=")
                    .is_some_and(|key| key == shared.password)
            })
        })
}

fn locked() -> Response {
    (
        StatusCode::UNAUTHORIZED,
        "missing key — open the URL the TUI prints with /web",
    )
        .into_response()
}

/// The first tag of `Accept-Language`, for the page to fall back on when the
/// browser tells it nothing better.
fn accept_language(headers: &HeaderMap) -> String {
    headers
        .get(header::ACCEPT_LANGUAGE)
        .and_then(|value| value.to_str().ok())
        .and_then(|value| value.split(',').next())
        .map(|value| value.split(';').next().unwrap_or(value).trim())
        .filter(|value| !value.is_empty())
        .unwrap_or("en")
        .to_string()
}

/// `GET /`: the shell, with its first state baked in so the first paint needs
/// no second request.
pub(crate) async fn page(
    State(shared): State<Arc<Shared>>,
    Query(params): Query<HashMap<String, String>>,
    headers: HeaderMap,
) -> Response {
    let handed_over = params.get("key").is_some_and(|key| key == &shared.password);
    if !handed_over && !web_authorized(&shared, &headers) {
        return locked();
    }
    let boot = boot_state(&shared, &accept_language(&headers)).await;
    // The accent comes from the TUI's theme, and it goes in before the script
    // runs so the first paint is already the right colour.
    let theme = crate::app::by_name(&store::load_settings().theme);
    let page = INDEX
        .replace(BOOT, &boot)
        .replace("__EXP1_ACCENT__", &theme.accent_hex())
        .replace("__EXP1_ACCENT_SOFT__", &theme.accent_soft());
    let mut response = Html(page).into_response();
    if handed_over {
        let cookie = format!(
            "exp1_key={}; Path=/; HttpOnly; SameSite=Strict",
            shared.password
        );
        if let Ok(value) = HeaderValue::from_str(&cookie) {
            response.headers_mut().insert(header::SET_COOKIE, value);
        }
    }
    response
}

/// Everything the first paint needs: the settings, the conversations, and the
/// counts the sidebar shows.
async fn boot_state(shared: &Arc<Shared>, language: &str) -> String {
    let settings = store::load_settings();
    let sessions = session_rows(50);
    let plugins = shared.mcp.specs().await.len();
    let scheduled = shared.scheduler.jobs().len();
    let state = serde_json::json!({
        "version": env!("CARGO_PKG_VERSION"),
        "accept_language": language,
        "web": settings.web,
        "provider": settings.provider,
        "model": settings.model,
        "theme": settings.theme,
        "accent": crate::app::by_name(&settings.theme).accent_hex(),
        "sessions": sessions,
        "plugins": plugins,
        "scheduled": scheduled,
    });
    // A JSON blob inside a <script> tag must not be able to close that tag.
    serde_json::to_string(&state)
        .unwrap_or_else(|_| "{}".to_string())
        .replace('<', "\\u003c")
}

/// What the sidebar lists: the conversations, newest first. All of them, not
/// only this directory's — the page is global.
fn session_rows(limit: usize) -> Vec<serde_json::Value> {
    store::list_sessions()
        .iter()
        .take(limit)
        .map(|meta| {
            serde_json::json!({
                "id": meta.id,
                "title": meta.label(),
                "when": meta.when(),
            })
        })
        .collect()
}

/// `GET /assets/{file}`: the styles and the script.
pub(crate) async fn asset(Path(file): Path<String>) -> Response {
    let (body, mime) = match file.as_str() {
        "app.css" => (APP_CSS, "text/css; charset=utf-8"),
        "app.js" => (APP_JS, "text/javascript; charset=utf-8"),
        _ => return (StatusCode::NOT_FOUND, "no such asset").into_response(),
    };
    (
        [
            (header::CONTENT_TYPE, mime),
            (header::CACHE_CONTROL, "no-cache"),
        ],
        body,
    )
        .into_response()
}

/// `GET /api/config`: what the settings screens read.
pub(crate) async fn config_get(State(shared): State<Arc<Shared>>, headers: HeaderMap) -> Response {
    if !web_authorized(&shared, &headers) {
        return locked();
    }
    let settings = store::load_settings();
    Json(serde_json::json!({
        "web": settings.web,
        "provider": settings.provider,
        "model": settings.model,
        "theme": settings.theme,
        "accent": crate::app::by_name(&settings.theme).accent_hex(),
        "version": env!("CARGO_PKG_VERSION"),
    }))
    .into_response()
}

/// `POST /api/config`: save what a settings screen changed. The body is the
/// whole `web` object, so a page that is a version behind cannot half-write it.
pub(crate) async fn config_post(
    State(shared): State<Arc<Shared>>,
    headers: HeaderMap,
    Json(web): Json<store::WebSettings>,
) -> Response {
    if !web_authorized(&shared, &headers) {
        return locked();
    }
    let mut settings = store::load_settings();
    settings.web = web;
    store::save_settings(&settings);
    Json(serde_json::json!({ "saved": true, "web": settings.web })).into_response()
}

/// `GET /api/sessions`: every conversation, newest first.
pub(crate) async fn sessions(State(shared): State<Arc<Shared>>, headers: HeaderMap) -> Response {
    if !web_authorized(&shared, &headers) {
        return locked();
    }
    Json(session_rows(200)).into_response()
}
