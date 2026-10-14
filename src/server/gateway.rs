//! the gateway headers, the model list, login and config routes.

use axum::extract::{Query, State};
use axum::http::{HeaderMap, StatusCode};
use axum::response::{IntoResponse, Json, Response};
use std::sync::Arc;
use crate::catalog;
use crate::proto::{
    LoginBody, ScopedBody, WireEffort, WireModel, WireProvider,
};
use crate::store;

use super::*;

/// `POST /config` — the model and effort are server-wide, so every attached UI
/// changes the same values rather than keeping private copies.
#[derive(Debug, serde::Serialize, serde::Deserialize)]
pub struct ConfigBody {
    pub model: Option<String>,
    pub effort: Option<String>,
}

#[derive(Debug, serde::Serialize, serde::Deserialize)]
pub struct WireConfig {
    pub model: String,
    pub effort: String,
    pub plan: Option<String>,
    /// The scope `/model` is held to, so a client knows it from the start.
    #[serde(default)]
    pub scoped: Vec<String>,
}

/// What a `/login` actually did, so the client can say the truth: a provider
/// that was already signed in is switched to, not signed into again.
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum LoginOutcome {
    /// A credential was already on disk; the provider was just activated.
    Switched,
    /// A pasted key was stored and the provider activated.
    SignedIn,
    /// A device flow is running; the page and code are in the response.
    Device,
}

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct WireLogin {
    pub outcome: LoginOutcome,
    pub label: String,
    pub model: String,
    pub effort: String,
    #[serde(default)]
    pub verification_uri: Option<String>,
    #[serde(default)]
    pub user_code: Option<String>,
    /// The line the client shows, in the same words the session received.
    pub message: String,
}

/// Whether a credential for this provider is already on the machine. The
/// Codex-style provider keeps its tokens in its own file, and Vertex takes
/// whatever gcloud is signed in with, so neither is in `credentials.json`.
fn has_credential(provider: &crate::providers::Provider) -> bool {
    match provider.id {
        "openai-auth" => crate::auth::Auth::load().is_ok(),
        "vertex" => true,
        _ => store::api_key(provider.id).is_some(),
    }
}
/// Builds the request target for a provider. GitHub Copilot is the odd one:
/// its stored credential is a long-lived GitHub token, and the host and bearer
/// token are both derived from a short-lived exchange done per request.
/// Headers the opencode gateways want on every call. They route by session and
/// refuse a request without it outright: "Request is missing x-opencode-session
/// and cannot be routed efficiently".
fn gateway_headers(
    provider: &'static crate::providers::Provider,
    session: &Arc<Session>,
) -> Vec<(String, String)> {
    if crate::providers::key_group(provider.id) != Some("opencode") {
        return Vec::new();
    }
    vec![
        (
            "x-opencode-session".to_string(),
            gateway_session_id(&session.id),
        ),
        (
            "x-opencode-project".to_string(),
            gateway_project_id(&session.cwd()),
        ),
        ("x-opencode-client".to_string(), "exp-1-agent".to_string()),
    ]
}

/// The gateways route by project as well, and their own client sends an
/// internal id there — not a path. A path is not even valid header material
/// when it holds anything but ASCII, and a request routed by a mangled project
/// lands on an upstream that does not serve the model: "Upstream response was
/// not valid JSON", five attempts in a row.
pub(crate) fn gateway_project_id(cwd: &std::path::Path) -> String {
    use std::hash::{Hash, Hasher};
    let mut hasher = std::collections::hash_map::DefaultHasher::new();
    cwd.hash(&mut hasher);
    format!("{:016x}", hasher.finish())
}

/// The gateways route by session, and their own client sends `ses_` followed by
/// 64 hex digits. Ours is a UUID, which the routing is left to guess at — and
/// guessed badly, it answers "Upstream response was not valid JSON" from an
/// upstream that was never meant for this model. The same id, in their shape.
pub(crate) fn gateway_session_id(session: &str) -> String {
    let hex: String = session.chars().filter(|ch| ch.is_ascii_hexdigit()).collect();
    let mut id = format!("ses_{hex}");
    while id.len() < 68 {
        id.push_str(&hex);
    }
    id.truncate(68);
    id
}

pub(crate) async fn request_target(
    provider: &'static crate::providers::Provider,
    model: &str,
    session: &Arc<Session>,
    effort: &str,
) -> Result<crate::backends::Target, String> {
    let stored = store::api_key(provider.id);
    let wire = provider.wire_for(model);
    let headers = gateway_headers(provider, session);

    // Vertex mints an access token from gcloud's application default
    // credentials, and builds its path from the project and location.
    if provider.id == "vertex" {
        let config = crate::oauth::vertex_config()
            .await
            .map_err(|err| format!("{err:#}"))?;
        let model_path = format!(
            "/projects/{}/locations/{}/publishers/google/models/{}:streamGenerateContent?alt=sse",
            config.project, config.location, model
        );
        return Ok(crate::backends::Target {
            provider,
            wire,
            endpoint: format!(
                "https://{}-aiplatform.googleapis.com/v1{model_path}",
                config.location
            ),
            key: Some(config.access_token),
            model: model.to_string(),
            headers,
            effort: effort.to_string(),
        });
    }

    // GitLab Duo rejects a personal access token directly; it has to be
    // exchanged for the OIDC token the gateway accepts.
    if provider.id == "gitlab-duo" {
        let pat = stored.ok_or_else(|| "store a GitLab token with /login first".to_string())?;
        let instance = std::env::var("GITLAB_INSTANCE_URL")
            .unwrap_or_else(|_| "https://gitlab.com".to_string());
        let oidc = crate::oauth::gitlab_oidc(&pat, &instance)
            .await
            .map_err(|err| format!("{err:#}"))?;
        return Ok(crate::backends::Target {
            provider,
            wire,
            endpoint: provider.endpoint_for(model),
            key: Some(oidc),
            model: model.to_string(),
            headers,
            effort: effort.to_string(),
        });
    }

    if provider.id == "github-copilot" {
        let github = stored.ok_or_else(|| "sign in to github copilot with /login first".to_string())?;
        let (host, token) = crate::oauth::copilot_bearer(&github)
            .await
            .map_err(|err| format!("{err:#}"))?;
        return Ok(crate::backends::Target {
            provider,
            wire,
            endpoint: format!("{host}{}", wire.suffix()),
            key: Some(token),
            model: model.to_string(),
            headers,
            effort: effort.to_string(),
        });
    }

    Ok(crate::backends::Target {
        provider,
        wire,
        endpoint: provider.endpoint_for(model),
        key: stored,
        model: model.to_string(),
        headers,
        effort: effort.to_string(),
    })
}

/// Writes a freshly obtained Codex-style auth file into our own state dir.
fn store_auth_file(raw: &str) -> Result<(), String> {
    let path = crate::auth::Auth::state_path();
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent).map_err(|err| err.to_string())?;
    }
    std::fs::write(&path, raw).map_err(|err| err.to_string())
}

/// The level to start from when a model offers several: `medium` when it is
/// among them, the middle of the list otherwise.
fn default_level(levels: &[String]) -> Option<String> {
    if levels.iter().any(|level| level == "medium") {
        return Some("medium".to_string());
    }
    levels.get(levels.len() / 2).cloned()
}
/// What a model list request asks for.
#[derive(serde::Deserialize)]
pub(crate) struct ModelsQuery {
    /// `all=1` ignores the scope: the `/scoped-models` picker has to offer the
    /// models that are not ticked yet.
    #[serde(default)]
    all: Option<String>,
}

pub(crate) async fn models(
    State(shared): State<Arc<Shared>>,
    headers: HeaderMap,
    Query(query): Query<ModelsQuery>,
) -> Response {
    if !authorized(&shared, &headers) {
        return unauthorized();
    }
    let active = shared.agent.provider();
    let everything = query
        .all
        .as_deref()
        .is_some_and(|value| value == "1" || value == "true");
    let scoped = store::load_settings().scoped_models;
    // A scope entry names one provider's model; a bare name in an older scope
    // still means "this name, from any provider".
    let allowed = |provider: &str, slug: &str| {
        everything
            || scoped.is_empty()
            || scoped
                .iter()
                .any(|chosen| chosen == slug || chosen == &format!("{provider}/{slug}"))
    };

    // Every provider that is signed in offers its models: the picker shows the
    // whole shelf rather than only the one in use, and picking a model from
    // another provider switches to it. The active provider comes first, so the
    // model in use is near the top.
    let mut order: Vec<&crate::providers::Provider> = crate::providers::PROVIDERS
        .iter()
        .filter(|provider| has_credential(provider))
        // A provider whose models speak a protocol no backend implements has
        // nothing the picker could offer.
        .filter(|provider| {
            provider
                .models
                .iter()
                .any(|(_, wire)| wire.is_remote() || *wire == crate::providers::Wire::CodexResponses)
        })
        .collect();
    order.sort_by_key(|provider| provider.id != active);

    let mut list: Vec<WireModel> = Vec::new();
    for provider in order {
        let entries: Vec<WireModel> = if provider.id == "openai-auth" {
            // The Codex models carry their own descriptions and efforts.
            catalog::load()
                .into_iter()
                .filter(|model| model.listed)
                .map(|model| WireModel {
                    slug: model.slug,
                    provider: provider.id.to_string(),
                    display_name: model.display_name,
                    description: format!("via {} · {}", provider.label, model.description),
                    efforts: model
                        .efforts
                        .into_iter()
                        .map(|(effort, description)| WireEffort { effort, description })
                        .collect(),
                    default_effort: model.default_effort,
                    context_window: model.context_window,
                })
                .collect()
        } else {
            let live = crate::providers::catalog_id(provider.id)
                .map(crate::models_dev::models_for)
                .unwrap_or_default();
            let ids: Vec<(String, u64)> = if live.is_empty() {
                provider
                    .model_ids()
                    .into_iter()
                    .map(|slug| (slug, provider_context_window()))
                    .collect()
            } else {
                live.into_iter()
                    .map(|entry| {
                        let window = entry.context.unwrap_or_else(provider_context_window);
                        (entry.slug, window)
                    })
                    .collect()
            };
            let catalog = crate::providers::catalog_id(provider.id);
            ids.into_iter()
                .map(|(slug, context_window)| {
                    // Every model has levels of its own; the catalogue is where
                    // they are written down.
                    let levels = catalog
                        .map(|catalog| crate::models_dev::levels_for(catalog, &slug))
                        .unwrap_or_default();
                    WireModel {
                        display_name: slug.clone(),
                        description: format!("via {}", provider.label),
                        context_window,
                        provider: provider.id.to_string(),
                        default_effort: default_level(&levels),
                        efforts: levels
                            .into_iter()
                            .map(|effort| WireEffort {
                                effort,
                                description: String::new(),
                            })
                            .collect(),
                        slug,
                    }
                })
                .collect()
        };
        list.extend(
            entries
                .into_iter()
                .filter(|model| allowed(&model.provider, &model.slug)),
        );
    }
    Json(list).into_response()
}
pub(crate) async fn list_providers(State(shared): State<Arc<Shared>>, headers: HeaderMap) -> Response {
    if !authorized(&shared, &headers) {
        return unauthorized();
    }
    let active = shared.agent.provider();
    let mut list: Vec<WireProvider> = crate::providers::PROVIDERS
        .iter()
        .map(|provider| WireProvider {
            id: provider.id.to_string(),
            label: provider.label.to_string(),
            logged_in: has_credential(provider),
            needs_key: provider.needs_key(),
            device_login: provider.needs_device_login(),
            active: provider.id == active,
        })
        .collect();
    // Strictly alphabetical, the way a list of names is read. The cursor still
    // opens on the active provider, so nothing has to be found by position.
    list.sort_by_key(|entry| entry.label.to_lowercase());
    Json(list).into_response()
}
/// `/login`: pick the active provider, and store a key when one is needed.
/// Stores a key for a provider and for every provider that shares its key:
/// the opencode gateways are one account, so one paste signs into both.
fn store_key(provider: &'static crate::providers::Provider, key: &str) -> anyhow::Result<()> {
    store::set_api_key(provider.id, key)?;
    if let Some(group) = crate::providers::key_group(provider.id) {
        for sibling in crate::providers::PROVIDERS
            .iter()
            .filter(|other| crate::providers::key_group(other.id) == Some(group))
        {
            let _ = store::set_api_key(sibling.id, key);
        }
    }
    Ok(())
}
pub(crate) async fn login(
    State(shared): State<Arc<Shared>>,
    headers: HeaderMap,
    Json(body): Json<LoginBody>,
) -> Response {
    if !authorized(&shared, &headers) {
        return unauthorized();
    }
    let Some(provider) = crate::providers::by_id(&body.provider) else {
        return (StatusCode::NOT_FOUND, "unknown provider").into_response();
    };
    // Picking a provider that is already signed in should switch to it, not
    // send the user through the sign-in page a second time.
    if has_credential(provider) && !body.force {
        // A credential stored before the shared-key groups existed still has to
        // reach its siblings, so the group is completed here too.
        if let Some(key) = store::api_key(provider.id) {
            let _ = store_key(provider, &key);
        }
        if let Ok(mut slot) = shared.agent.provider.lock() {
            *slot = provider.id.to_string();
        }
        if let Some((model, _)) = provider.models.first()
            && let Ok(mut slot) = shared.agent.model.lock()
        {
            *slot = (*model).to_string();
        }
        let mut settings = store::load_settings();
        settings.provider = provider.id.to_string();
        settings.model = shared.agent.model();
        let _ = store::save_settings(&settings);
        let message = format!(
            "Already signed in to {}; switched. Pick it again in /login to sign in anew.",
            provider.label
        );
        if let Some(session) = body.session.as_deref().and_then(|id| find(&shared, id)) {
            session.push("system", message.clone());
        }
        return Json(WireLogin {
            outcome: LoginOutcome::Switched,
            label: provider.label.to_string(),
            model: shared.agent.model(),
            effort: shared.agent.effort(),
            verification_uri: None,
            user_code: None,
            message,
        })
        .into_response();
    }

    // Device-code providers sign in on the vendor's own page; there is nothing
    // to paste, so the flow is started here and watched in the background.
    if provider.needs_device_login() {
        let narrator = body
            .session
            .as_deref()
            .and_then(|id| find(&shared, id));
        let (prompt, flow) = match crate::oauth::begin(provider.id).await {
            Ok(pair) => pair,
            Err(err) => {
                return (StatusCode::BAD_GATEWAY, format!("{err:#}")).into_response();
            }
        };
        // One wording for the session and for the client, so both say the same.
        let message = format!(
            "Sign in to {}: open {}\nand enter the code {}",
            provider.label, prompt.verification_uri, prompt.user_code
        );
        // The page opens by itself, as `codex login` does; the message above
        // stays the fallback for a machine without a browser.
        crate::oauth::open_in_browser(&prompt.verification_uri);
        if let Some(session) = &narrator {
            session.push("system", message.clone());
        }

        let provider_id = provider.id.to_string();
        let label = provider.label.to_string();
        let shared_for_poll = shared.clone();
        tokio::spawn(async move {
            let deadline =
                std::time::Instant::now() + std::time::Duration::from_secs(prompt.expires_in);
            let mut interval = prompt.interval.max(2);
            while std::time::Instant::now() < deadline {
                tokio::time::sleep(std::time::Duration::from_secs(interval)).await;
                match crate::oauth::poll(&provider_id, &flow).await {
                    Ok(crate::oauth::Poll::Pending) => {}
                    Ok(crate::oauth::Poll::SlowDown) => interval += 5,
                    Ok(crate::oauth::Poll::Ready(value)) => {
                        let stored = if provider_id == "openai-auth" {
                            // The Codex reader expects a whole auth file there.
                            store_auth_file(&value)
                        } else {
                            store::set_api_key(&provider_id, &value).map_err(|err| err.to_string())
                        };
                        match stored {
                            Ok(()) => {
                                // New tokens: drop the cached Codex client so
                                // the next request loads them instead of the
                                // ones it was built with.
                                shared_for_poll.agent.forget_codex();
                                if let Ok(mut slot) = shared_for_poll.agent.provider.lock() {
                                    *slot = provider_id.clone();
                                }
                                if let Ok(mut slot) = shared_for_poll.agent.model.lock()
                                    && let Some(provider) = crate::providers::by_id(&provider_id)
                                    && let Some((model, _)) = provider.models.first()
                                {
                                    *slot = (*model).to_string();
                                }
                                let mut settings = store::load_settings();
                                settings.provider = provider_id.clone();
                                settings.model = shared_for_poll.agent.model();
                                let _ = store::save_settings(&settings);
                                if let Some(session) = &narrator {
                                    session.push(
                                        "system",
                                        format!("Signed in to {label}; it is now the active provider"),
                                    );
                                }
                            }
                            Err(message) => {
                                if let Some(session) = &narrator {
                                    session.push(
                                        "system",
                                        format!("Signed in, but the credential could not be saved: {message}"),
                                    );
                                }
                            }
                        }
                        return;
                    }
                    Err(err) => {
                        if let Some(session) = &narrator {
                            session.push("system", format!("{label} login failed: {err:#}"));
                        }
                        return;
                    }
                }
            }
            if let Some(session) = &narrator {
                session.push("system", format!("{label} login timed out; try /login again"));
            }
        });

        return Json(WireLogin {
            outcome: LoginOutcome::Device,
            label: provider.label.to_string(),
            model: shared.agent.model(),
            effort: shared.agent.effort(),
            verification_uri: Some(prompt.verification_uri),
            user_code: Some(prompt.user_code),
            message,
        })
        .into_response();
    }
    if let Some(key) = body.key.as_deref().map(str::trim).filter(|k| !k.is_empty()) {
        if let Err(err) = store_key(provider, key) {
            return (
                StatusCode::INTERNAL_SERVER_ERROR,
                format!("could not store the key: {err:#}"),
            )
                .into_response();
        }
        // A fresh sign-in is a good moment to fetch the current model list.
        let http = shared.agent.registry.lock().await.http();
        tokio::spawn(async move {
            crate::models_dev::refresh_if_stale(&http).await;
        });
    }
    // The Anthropic OAuth provider adopts a token another tool already holds
    // before asking the user to sign in.
    if provider.auth == crate::providers::Auth::OAuthToken
        && store::api_key(provider.id).is_none()
        && let Some(token) = store::adopt_anthropic_token()
    {
        let _ = store::set_api_key(provider.id, &token);
    }
    if provider.needs_key() && store::api_key(provider.id).is_none() {
        return (StatusCode::BAD_REQUEST, "an API key is required").into_response();
    }

    if let Ok(mut slot) = shared.agent.provider.lock() {
        *slot = provider.id.to_string();
    }
    // The first model of the new provider, and an effort it actually takes.
    if let Some((model, _)) = provider.models.first()
        && let Ok(mut slot) = shared.agent.model.lock()
    {
        *slot = (*model).to_string();
    }
    let mut settings = store::load_settings();
    settings.provider = provider.id.to_string();
    settings.model = shared.agent.model();
    settings.effort = shared.agent.effort();
    let _ = store::save_settings(&settings);

    let message = format!(
        "Signed in to {}; model is {} {}",
        provider.label,
        shared.agent.model(),
        shared.agent.effort()
    );
    Json(WireLogin {
        outcome: LoginOutcome::SignedIn,
        label: provider.label.to_string(),
        model: shared.agent.model(),
        effort: shared.agent.effort(),
        verification_uri: None,
        user_code: None,
        message,
    })
    .into_response()
}
pub(crate) async fn scoped_models(
    State(shared): State<Arc<Shared>>,
    headers: HeaderMap,
    Json(body): Json<ScopedBody>,
) -> Response {
    if !authorized(&shared, &headers) {
        return unauthorized();
    }
    let mut settings = store::load_settings();
    settings.scoped_models = body
        .models
        .into_iter()
        .map(|model| model.trim().to_string())
        .filter(|model| !model.is_empty())
        .collect();
    let _ = store::save_settings(&settings);
    StatusCode::NO_CONTENT.into_response()
}
pub(crate) async fn get_config(State(shared): State<Arc<Shared>>, headers: HeaderMap) -> Response {
    if !authorized(&shared, &headers) {
        return unauthorized();
    }
    Json(WireConfig {
        model: shared.agent.model(),
        effort: shared.agent.effort(),
        plan: shared.plan.lock().ok().and_then(|guard| guard.clone()),
        scoped: store::load_settings().scoped_models.into_iter().collect(),
    })
    .into_response()
}
pub(crate) async fn config(
    State(shared): State<Arc<Shared>>,
    headers: HeaderMap,
    Json(body): Json<ConfigBody>,
) -> Response {
    if !authorized(&shared, &headers) {
        return unauthorized();
    }
    if let Some(model) = body.model
        && !model.trim().is_empty()
    {
        let model = model.trim().to_string();
        // A model names its provider. Setting one the provider in use does not
        // offer used to leave the two out of step, and the next request went
        // out on the wrong provider with a model it had never heard of.
        let offered = crate::providers::by_id(&shared.agent.provider())
            .is_some_and(|provider| offers(provider, &model));
        if !offered {
            match owner_of(&model) {
                Some(owner) => {
                    if let Ok(mut slot) = shared.agent.provider.lock() {
                        *slot = owner.id.to_string();
                    }
                }
                None => {
                    return (
                        StatusCode::BAD_REQUEST,
                        format!(
                            "'{model}' is not a model of the provider in use, and no signed-in \
                             provider offers it — run /login, or pick one from /model"
                        ),
                    )
                        .into_response();
                }
            }
        }
        if let Ok(mut slot) = shared.agent.model.lock() {
            *slot = model;
        }
    }
    if let Some(effort) = body.effort
        && !effort.trim().is_empty()
        && let Ok(mut slot) = shared.agent.effort.lock()
    {
        *slot = effort;
    }
    // Load-modify-save, so fields the server does not own survive.
    let mut settings = store::load_settings();
    if let Ok(model) = shared.agent.model.lock() {
        settings.model = model.clone();
    }
    if let Ok(effort) = shared.agent.effort.lock() {
        settings.effort = effort.clone();
    }
    settings.provider = shared.agent.provider();
    let _ = store::save_settings(&settings);
    StatusCode::NO_CONTENT.into_response()
}
/// Whether a provider offers a model: its own table, the live catalogue, or —
/// for the Codex provider — the catalogue the Codex CLI keeps.
pub(crate) fn offers(provider: &crate::providers::Provider, model: &str) -> bool {
    if provider.id == "openai-auth" {
        return catalog::load().iter().any(|entry| entry.slug == model);
    }
    if provider.models.iter().any(|(id, _)| *id == model) {
        return true;
    }
    crate::providers::catalog_id(provider.id)
        .map(|catalog| {
            crate::models_dev::models_for(catalog)
                .iter()
                .any(|entry| entry.slug == model)
        })
        .unwrap_or(false)
}

/// The signed-in provider that offers a model, when one does.
pub(crate) fn owner_of(model: &str) -> Option<&'static crate::providers::Provider> {
    crate::providers::PROVIDERS
        .iter()
        .find(|provider| has_credential(provider) && offers(provider, model))
}

/// Keeps `read_lock` used from one place, so the helper is not dead code.
#[allow(dead_code)]
fn model_label(shared: &Shared) -> String {
    read_lock(&shared.agent.model)
}