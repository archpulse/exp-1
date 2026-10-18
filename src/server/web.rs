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

/// Where the boot state goes in `index.html`, and the style block with the
/// page's colours.
const BOOT: &str = "__EXP1_BOOT__";
const THEME: &str = "__EXP1_THEME__";

/// The colours the page offers by name, in the order its menu lists them. The
/// names are the same ones `app/theme.rs` uses for the terminal's themes.
pub(crate) const ACCENTS: [(&str, &str); 7] = [
    ("blue", "#3b82f6"),
    ("green", "#10b981"),
    ("yellow", "#eab308"),
    ("pink", "#ec4899"),
    ("orange", "#f97316"),
    ("purple", "#8b5cf6"),
    ("white", "#ffffff"),
];

/// The colour the page paints with: what the user picked, a palette name or a
/// `#rrggbb`, otherwise the accent of the theme the TUI is on.
pub(crate) fn accent_hex(settings: &store::Settings) -> String {
    let chosen = settings.web.accent.trim();
    if chosen.is_empty() {
        return crate::app::by_name(&settings.theme).accent_hex();
    }
    if let Some((_, hex)) = ACCENTS
        .iter()
        .find(|(name, _)| name.eq_ignore_ascii_case(chosen))
    {
        return (*hex).to_string();
    }
    if valid_hex(chosen) {
        return chosen.to_lowercase();
    }
    crate::app::by_name(&settings.theme).accent_hex()
}

/// The only shape a colour may have before it reaches a stylesheet.
fn valid_hex(value: &str) -> bool {
    value.len() == 7
        && value.starts_with('#')
        && value[1..].chars().all(|ch| ch.is_ascii_hexdigit())
}

/// A colour from the settings, or the built-in one when it is missing or wrong.
fn colour_or(value: &str, fallback: &str) -> String {
    let value = value.trim();
    if value.is_empty() {
        fallback.to_string()
    } else if valid_hex(value) {
        value.to_lowercase()
    } else {
        fallback.to_string()
    }
}

/// The variables the page paints from: the accent, the two page colours, the
/// two text sizes, the motion choice and the contrast. Everything is checked
/// here, because this string goes straight into a `<style>` tag.
pub(crate) fn theme_style(settings: &store::Settings) -> String {
    let web = &settings.web;
    let accent = accent_hex(settings);
    let background = colour_or(&web.background, "#212121");
    let foreground = colour_or(&web.foreground, "#ececec");
    let contrast = 0.6 + f32::from(web.contrast.min(200)) / 100.0;
    format!(
        ":root{{--accent:{accent};--accent-soft:{accent}26;--bg:{background};--fg:{foreground};\
         --ui-font:{}px;--code-font:{}px;--contrast:{contrast:.2};}}",
        web.ui_font.clamp(10, 28),
        web.code_font.clamp(8, 28),
    )
}

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
        "no access — the page answers to 127.0.0.1, and to the server password",
    )
        .into_response()
}

/// Whether the request asked for a loopback name. The server listens only on
/// 127.0.0.1, so the page does not need the password; this check is what keeps a
/// website that points its own name at 127.0.0.1 (DNS rebinding) from reading
/// answers meant for the machine's owner.
fn host_is_loopback(headers: &HeaderMap) -> bool {
    loopback_name(
        headers
            .get(header::HOST)
            .and_then(|value| value.to_str().ok())
            .unwrap_or_default(),
    )
}

fn loopback_name(host: &str) -> bool {
    let name = if let Some(rest) = host.strip_prefix('[') {
        rest.split(']').next().unwrap_or(rest)
    } else {
        host.rsplit_once(':').map_or(host, |(name, _)| name)
    };
    matches!(name, "127.0.0.1" | "localhost" | "::1")
}

/// Who is logged in, for the greeting: the reference greets by name.
fn user_name() -> String {
    std::env::var("USER")
        .or_else(|_| std::env::var("USERNAME"))
        .or_else(|_| std::env::var("LOGNAME"))
        .unwrap_or_default()
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
    if !handed_over && !web_authorized(&shared, &headers) && !host_is_loopback(&headers) {
        return locked();
    }
    let boot = boot_state(&shared, &accept_language(&headers)).await;
    // The page's colours and sizes come from the settings, and they go in
    // before the script runs so the first paint is already right.
    let settings = store::load_settings();
    let page = INDEX
        .replace(BOOT, &boot)
        .replace(THEME, &theme_style(&settings));
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
    let plugins = shared.mcp.specs().await.len();
    let scheduled = shared.scheduler.jobs().len();
    let state = serde_json::json!({
        "version": env!("CARGO_PKG_VERSION"),
        "accept_language": language,
        "web": settings.web,
        "provider": settings.provider,
        "model": settings.model,
        "theme": settings.theme,
        "user": user_name(),
        "accent": accent_hex(&settings),
        "palette": ACCENTS.iter().map(|(name, hex)| serde_json::json!({ "name": name, "hex": hex })).collect::<Vec<_>>(),
        "plugins": plugins,
        "scheduled": scheduled,
        "mcp_path": crate::store::mcp_path().display().to_string(),
    });
    // A JSON blob inside a <script> tag must not be able to close that tag.
    serde_json::to_string(&state)
        .unwrap_or_else(|_| "{}".to_string())
        .replace('<', "\\u003c")
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
    if !web_authorized(&shared, &headers) && !host_is_loopback(&headers) {
        return locked();
    }
    let settings = store::load_settings();
    Json(serde_json::json!({
        "web": settings.web,
        "provider": settings.provider,
        "model": settings.model,
        "theme": settings.theme,
        "accent": accent_hex(&settings),
        "palette": ACCENTS.iter().map(|(name, hex)| serde_json::json!({ "name": name, "hex": hex })).collect::<Vec<_>>(),
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
    if !web_authorized(&shared, &headers) && !host_is_loopback(&headers) {
        return locked();
    }
    let mut settings = store::load_settings();
    settings.web = web;
    store::save_settings(&settings);
    Json(serde_json::json!({ "saved": true, "web": settings.web })).into_response()
}

/// `GET /api/schedules`: the jobs, as the page's scheduled section lists them.
pub(crate) async fn schedules(State(shared): State<Arc<Shared>>, headers: HeaderMap) -> Response {
    if !web_authorized(&shared, &headers) && !host_is_loopback(&headers) {
        return locked();
    }
    Json(jobs_json(&shared)).into_response()
}

/// `POST /api/schedules/{id}/{action}`: `run`, `pause` or `resume` one job.
pub(crate) async fn schedule_action(
    State(shared): State<Arc<Shared>>,
    Path((id, action)): Path<(String, String)>,
    headers: HeaderMap,
) -> Response {
    if !web_authorized(&shared, &headers) && !host_is_loopback(&headers) {
        return locked();
    }
    match act(&shared, &id, &action) {
        Ok(note) => Json(serde_json::json!({ "note": note })).into_response(),
        Err(message) => (StatusCode::NOT_FOUND, message).into_response(),
    }
}

/// `DELETE /api/schedules/{id}`: drop one job.
pub(crate) async fn schedule_remove(
    State(shared): State<Arc<Shared>>,
    Path(id): Path<String>,
    headers: HeaderMap,
) -> Response {
    if !web_authorized(&shared, &headers) && !host_is_loopback(&headers) {
        return locked();
    }
    match remove(&shared, &id) {
        Ok(note) => Json(serde_json::json!({ "note": note })).into_response(),
        Err(message) => (StatusCode::NOT_FOUND, message).into_response(),
    }
}

/// `GET /api/plugins`: the MCP servers and the tools they offer.
pub(crate) async fn plugins(State(shared): State<Arc<Shared>>, headers: HeaderMap) -> Response {
    if !web_authorized(&shared, &headers) && !host_is_loopback(&headers) {
        return locked();
    }
    Json(shared.mcp.status().await).into_response()
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The page needs no key on loopback, and the Host check is what makes that
    /// safe: a name that is not loopback is refused even though it resolves here.
    #[test]
    fn only_loopback_names_skip_the_key() {
        for host in ["127.0.0.1", "127.0.0.1:7878", "localhost:7878", "[::1]:7878", "[::1]"] {
            assert!(loopback_name(host), "{host} is loopback");
        }
        for host in ["", "example.com", "example.com:7878", "evil.test", "127.0.0.1.evil.test", "0.0.0.0:7878"] {
            assert!(!loopback_name(host), "{host} is not loopback");
        }
    }

    /// A colour reaches a stylesheet, so only a palette name or a plain
    /// `#rrggbb` gets through; anything else falls back to the theme.
    #[test]
    fn only_real_colours_reach_the_stylesheet() {
        let mut settings = store::Settings::default();
        settings.theme = "amber".to_string();
        assert_eq!(accent_hex(&settings), "#d99120", "the theme decides by default");
        settings.web.accent = "pink".to_string();
        assert_eq!(accent_hex(&settings), "#ec4899", "a palette name");
        settings.web.accent = "#123abc".to_string();
        assert_eq!(accent_hex(&settings), "#123abc", "a colour of one's own");
        settings.web.accent = "red;} body { background: url(evil) }".to_string();
        assert_eq!(accent_hex(&settings), "#d99120", "an injection falls back");
        assert_eq!(colour_or("#AABBCC", "#000000"), "#aabbcc");
        assert_eq!(colour_or("javascript:alert(1)", "#212121"), "#212121");
        assert_eq!(colour_or("", "#212121"), "#212121");
    }

    /// The style block carries the sizes and the contrast, and it clamps what it
    /// cannot use. The motion choice is an attribute the script sets, because a
    /// stylesheet cannot put one on the element itself.
    #[test]
    fn the_style_block_carries_the_appearance_settings() {
        let mut settings = store::Settings::default();
        settings.web.ui_font = 200;
        settings.web.code_font = 2;
        settings.web.contrast = 60;
        let style = theme_style(&settings);
        assert!(style.contains("--ui-font:28px"), "{style}");
        assert!(style.contains("--code-font:8px"), "{style}");
        assert!(style.contains("--contrast:1.20"), "{style}");
        settings.web.background = "#101010".to_string();
        settings.web.foreground = "rgb(1,2,3)".to_string();
        let style = theme_style(&settings);
        assert!(style.contains("--bg:#101010"), "{style}");
        assert!(style.contains("--fg:#ececec"), "not a colour, so the default: {style}");
    }
}
