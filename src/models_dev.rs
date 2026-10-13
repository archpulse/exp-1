//! The live model list, from models.dev.
//!
//! The protocol is a property of the model, not the provider (see
//! `providers.rs`), and models.dev records it per model as the AI SDK package
//! the provider is configured with: `@ai-sdk/anthropic` for a Claude model
//! behind a gateway, `@ai-sdk/openai` for a Grok one, `@ai-sdk/google` for a
//! Gemini one. Reading the catalogue opencode itself reads keeps the list
//! current without a table in this repository that goes stale.
//!
//! The file is fetched at most once every few hours, cached beside the
//! sessions, and held in memory. A provider with no entry there keeps its table
//! in `providers.rs`, so the app is fully usable offline.

use anyhow::Context;
use crate::providers::Wire;
use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::{OnceLock, RwLock};
use std::time::{Duration, SystemTime};

const SOURCE: &str = "https://models.dev/api.json";
/// How long a downloaded catalogue is trusted before a refresh is due.
const TTL: Duration = Duration::from_secs(12 * 60 * 60);
const FILE: &str = "models-dev.json";

/// One model as the catalogue lists it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Entry {
    pub slug: String,
    /// The protocol it speaks, when the catalogue names a package we know.
    pub wire: Option<Wire>,
    /// The context window in tokens, when the catalogue states one.
    pub context: Option<u64>,
    /// The reasoning levels it takes, in the order the catalogue lists them.
    pub efforts: Vec<String>,
}

#[derive(Default)]
struct Catalogue {
    /// models.dev provider id → its models, sorted by id.
    providers: HashMap<String, Vec<Entry>>,
    fetched: Option<SystemTime>,
}

static CATALOGUE: OnceLock<RwLock<Catalogue>> = OnceLock::new();

fn slot() -> &'static RwLock<Catalogue> {
    CATALOGUE.get_or_init(|| RwLock::new(load_cache()))
}

pub fn cache_path() -> PathBuf {
    crate::store::cache_root().join(FILE)
}

/// The wire an AI SDK package speaks.
fn wire_from_npm(npm: &str) -> Option<Wire> {
    Some(match npm {
        // In AI SDK v5 the OpenAI package speaks the Responses API, which is
        // what entries carrying it are configured for.
        "@ai-sdk/openai" => Wire::OpenAiResponses,
        "@ai-sdk/anthropic" => Wire::Anthropic,
        "@ai-sdk/google" | "@ai-sdk/google-vertex" => Wire::Gemini,
        "@ai-sdk/openai-compatible" | "@ai-sdk/xai" | "@ai-sdk/groq" | "@ai-sdk/deepseek"
        | "@ai-sdk/mistral" | "@ai-sdk/cerebras" | "@ai-sdk/togetherai" | "@ai-sdk/perplexity"
        | "@ai-sdk/azure" | "@ai-sdk/alibaba" | "@ai-sdk/moonshotai" | "@ai-sdk/fireworks"
        | "@openrouter/ai-sdk-provider" => Wire::OpenAiChat,
        _ => return None,
    })
}

#[derive(serde::Deserialize)]
struct RawProvider {
    #[serde(default)]
    models: HashMap<String, RawModel>,
}

#[derive(serde::Deserialize)]
struct RawModel {
    #[serde(default)]
    provider: Option<RawPackage>,
    #[serde(default)]
    limit: Option<RawLimit>,
    #[serde(default)]
    reasoning_options: Vec<RawReasoning>,
}

/// How a model can be asked to think. `effort` carries the levels; a plain
/// `toggle` says the model reasons or does not, which is not a level.
#[derive(serde::Deserialize)]
struct RawReasoning {
    #[serde(default, rename = "type")]
    kind: String,
    /// A level can be `null` in the catalogue, which is not a level.
    #[serde(default)]
    values: Vec<Option<String>>,
}

#[derive(serde::Deserialize)]
struct RawPackage {
    #[serde(default)]
    npm: Option<String>,
}

#[derive(serde::Deserialize)]
struct RawLimit {
    #[serde(default)]
    context: Option<u64>,
}

/// Parses the whole catalogue into `provider id → models`.
fn parse(raw: &str) -> HashMap<String, Vec<Entry>> {
    // Parsed provider by provider: the catalogue is a live external file, and
    // one entry worded differently must not cost the rest of the list.
    let Ok(providers) = serde_json::from_str::<HashMap<String, serde_json::Value>>(raw) else {
        return HashMap::new();
    };
    providers
        .into_iter()
        .filter_map(|(id, value)| {
            let provider: RawProvider = serde_json::from_value(value).ok()?;
            let mut models: Vec<Entry> = provider
                .models
                .into_iter()
                .map(|(slug, model)| Entry {
                    wire: model
                        .provider
                        .and_then(|package| package.npm)
                        .as_deref()
                        .and_then(wire_from_npm),
                    context: model.limit.and_then(|limit| limit.context),
                    efforts: model
                        .reasoning_options
                        .into_iter()
                        .filter(|option| option.kind == "effort")
                        .flat_map(|option| option.values)
                        .flatten()
                        .collect(),
                    slug,
                })
                .collect();
            models.sort_by(|a, b| a.slug.cmp(&b.slug));
            Some((id, models))
        })
        .collect()
}

fn load_cache() -> Catalogue {
    let path = cache_path();
    let Ok(raw) = std::fs::read_to_string(&path) else {
        return Catalogue::default();
    };
    let fetched = std::fs::metadata(&path)
        .and_then(|meta| meta.modified())
        .ok();
    Catalogue {
        providers: parse(&raw),
        fetched,
    }
}

/// The models the catalogue lists for a provider; empty when it has no entry.
pub fn models_for(provider: &str) -> Vec<Entry> {
    let guard = slot().read().expect("models.dev catalogue");
    guard.providers.get(provider).cloned().unwrap_or_default()
}

/// The protocol the catalogue gives a model, when it names one we know.
pub fn wire_for(provider: &str, slug: &str) -> Option<Wire> {
    let guard = slot().read().expect("models.dev catalogue");
    guard
        .providers
        .get(provider)?
        .iter()
        .find(|entry| entry.slug == slug)?
        .wire
}

/// The reasoning levels the catalogue gives a model, in its own order.
pub fn levels_for(provider: &str, slug: &str) -> Vec<String> {
    let guard = slot().read().expect("models.dev catalogue");
    guard
        .providers
        .get(provider)
        .and_then(|models| models.iter().find(|entry| entry.slug == slug))
        .map(|entry| entry.efforts.clone())
        .unwrap_or_default()
}

/// The context window the catalogue states for a model. Without one the
/// conversation would never be compacted, so a gateway model needs this.
pub fn context_limit(provider: &str, slug: &str) -> Option<u64> {
    let guard = slot().read().expect("models.dev catalogue");
    guard
        .providers
        .get(provider)?
        .iter()
        .find(|entry| entry.slug == slug)?
        .context
}

/// True when nothing is cached, or what is cached is old enough that asking
/// for the current list is worth a request.
pub fn stale() -> bool {
    let guard = slot().read().expect("models.dev catalogue");
    if guard.providers.is_empty() {
        return true;
    }
    match guard.fetched {
        Some(when) => SystemTime::now()
            .duration_since(when)
            .map(|age| age > TTL)
            .unwrap_or(true),
        None => true,
    }
}

/// Downloads the catalogue and swaps it in. A failure leaves the old one in
/// place: a stale list beats no list.
pub async fn refresh(http: &reqwest::Client) -> anyhow::Result<()> {
    let response = http
        .get(SOURCE)
        .timeout(Duration::from_secs(30))
        .send()
        .await
        .context("fetch models.dev")?;
    if !response.status().is_success() {
        anyhow::bail!("models.dev answered {}", response.status());
    }
    let body = response.text().await.context("read models.dev")?;
    let providers = parse(&body);
    if providers.is_empty() {
        anyhow::bail!("models.dev returned nothing usable");
    }
    let path = cache_path();
    if let Some(parent) = path.parent() {
        let _ = std::fs::create_dir_all(parent);
    }
    let _ = std::fs::write(&path, &body);
    let mut guard = slot().write().expect("models.dev catalogue");
    guard.providers = providers;
    guard.fetched = Some(SystemTime::now());
    Ok(())
}

/// Refreshes when due; the outcome is not interesting, the list is.
pub async fn refresh_if_stale(http: &reqwest::Client) {
    if !stale() {
        return;
    }
    let _ = refresh(http).await;
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The packages behind the wires we support.
    #[test]
    fn packages_map_to_wires() {
        assert_eq!(wire_from_npm("@ai-sdk/anthropic"), Some(Wire::Anthropic));
        assert_eq!(wire_from_npm("@ai-sdk/openai"), Some(Wire::OpenAiResponses));
        assert_eq!(wire_from_npm("@ai-sdk/google"), Some(Wire::Gemini));
        assert_eq!(wire_from_npm("@ai-sdk/openai-compatible"), Some(Wire::OpenAiChat));
        assert_eq!(wire_from_npm("@ai-sdk/amazon-bedrock"), None);
    }

    /// A catalogue entry becomes a model with the protocol it speaks.
    #[test]
    fn the_catalogue_shape_is_read() {
        let raw = r#"{
            "opencode-go": { "models": {
                "grok-4.7": { "id": "grok-4.7", "provider": { "npm": "@ai-sdk/openai" }, "limit": { "context": 131072, "output": 8192 } },
                "claude-haiku-5-5": { "provider": { "npm": "@ai-sdk/anthropic" } },
                "space-bunny": { "provider": { "npm": "@ai-sdk/openai-compatible" } },
                "mystery": {}
            } },
            "empty": {}
        }"#;
        let providers = parse(raw);
        let go = providers.get("opencode-go").expect("provider");
        let slugs: Vec<&str> = go.iter().map(|entry| entry.slug.as_str()).collect();
        assert_eq!(slugs, vec!["claude-haiku-5-5", "grok-4.7", "mystery", "space-bunny"]);
        let entry = |slug: &str| go.iter().find(|entry| entry.slug == slug);
        assert_eq!(entry("grok-4.7").and_then(|e| e.wire), Some(Wire::OpenAiResponses));
        assert_eq!(entry("grok-4.7").and_then(|e| e.context), Some(131_072));
        assert_eq!(entry("claude-haiku-5-5").and_then(|e| e.wire), Some(Wire::Anthropic));
        assert_eq!(entry("claude-haiku-5-5").and_then(|e| e.context), None);
        assert_eq!(entry("space-bunny").and_then(|e| e.wire), Some(Wire::OpenAiChat));
        assert_eq!(entry("mystery").and_then(|e| e.wire), None);
        assert!(providers.get("empty").is_some_and(|models| models.is_empty()));
    }

    /// The levels a model takes come from the catalogue, in its order; a
    /// toggle is not a level and is left out.
    #[test]
    fn reasoning_levels_come_from_the_catalogue() {
        let raw = r#"{
            "opencode-go": { "models": {
                "grok": { "reasoning_options": [
                    { "type": "effort", "values": ["low", "medium", "xhigh"] }
                ] },
                "plain": { "reasoning_options": [ { "type": "toggle" } ] },
                "none": {}
            } }
        }"#;
        let providers = parse(raw);
        let go = providers.get("opencode-go").expect("provider");
        let levels = |slug: &str| {
            go.iter()
                .find(|entry| entry.slug == slug)
                .map(|entry| entry.efforts.clone())
                .unwrap_or_default()
        };
        assert_eq!(levels("grok"), vec!["low", "medium", "xhigh"]);
        assert!(levels("plain").is_empty());
        assert!(levels("none").is_empty());
    }

    /// A level written as null is not a level, and one provider the catalogue
    /// words differently must not take the others down with it.
    #[test]
    fn a_loose_entry_costs_only_itself() {
        let raw = r#"{
            "loose": { "models": {
                "odd": { "reasoning_options": [
                    { "type": "effort", "values": [null, "low", "high"] }
                ] }
            } },
            "broken": { "models": "not an object" },
            "fine": { "models": { "ok": {} } }
        }"#;
        let providers = parse(raw);
        let loose = providers.get("loose").expect("the loose provider stays");
        assert_eq!(loose[0].efforts, vec!["low", "high"]);
        assert!(providers.get("fine").is_some(), "its neighbour survives");
        assert!(providers.get("broken").is_none(), "and only the broken one goes");
    }

    /// Nonsense in the cache must not take the list down with it.
    #[test]
    fn a_broken_catalogue_parses_to_nothing() {
        assert!(parse("not json").is_empty());
        assert!(parse("[]").is_empty());
    }
}
