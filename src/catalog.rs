//! Model catalogue, read from the Codex CLI's own cache.
//!
//! Reasoning efforts differ per model — some stop at `xhigh`, some add `max`,
//! and `gpt-5.6-terra` goes to `ultra` — so the list has to come from the
//! provider rather than from a constant in this file.
//!
//! Source: `$CODEX_HOME/models_cache.json` (default `~/.codex/models_cache.json`).
//! When that file is absent the app falls back to a conservative list so it is
//! still usable, but the real answers come from the cache.

use serde::Deserialize;
use std::path::PathBuf;

/// Compact once a conversation reaches this share of the context window.
pub const COMPACT_THRESHOLD_PERCENT: u64 = 95;

#[derive(Debug, Clone)]
pub struct ModelInfo {
    pub slug: String,
    /// Human name from the catalogue, e.g. `GPT-6-Luna`.
    pub display_name: String,
    pub description: String,
    /// Reasoning efforts this model accepts, with their blurbs.
    pub efforts: Vec<(String, String)>,
    /// What the provider uses when no effort is given.
    pub default_effort: Option<String>,
    /// Whether the provider wants it listed in a picker.
    pub listed: bool,
    /// Total context window in tokens, as the provider reports it.
    pub context_window: u64,
}

impl ModelInfo {
    /// The point at which a conversation is compacted: 95% of the window by
    /// default, overridable with `EXP1_COMPACT_PERCENT`.
    pub fn context_limit(&self) -> u64 {
        if self.context_window == 0 {
            return 0;
        }
        self.context_window * compact_threshold_percent() / 100
    }
}

/// Read once per check; the override exists so the threshold can be exercised
/// without actually filling a 272k window.
pub fn compact_threshold_percent() -> u64 {
    std::env::var("EXP1_COMPACT_PERCENT")
        .ok()
        .and_then(|value| value.parse().ok())
        .filter(|value: &u64| *value > 0 && *value <= 100)
        .unwrap_or(COMPACT_THRESHOLD_PERCENT)
}

#[derive(Deserialize)]
struct Cache {
    #[serde(default)]
    models: Vec<RawModel>,
}

#[derive(Deserialize)]
struct RawModel {
    slug: String,
    #[serde(default)]
    display_name: Option<String>,
    #[serde(default)]
    description: Option<String>,
    #[serde(default)]
    default_reasoning_level: Option<String>,
    #[serde(default)]
    supported_reasoning_levels: Vec<RawLevel>,
    #[serde(default)]
    visibility: Option<String>,
    #[serde(default)]
    context_window: u64,
}

#[derive(Deserialize)]
struct RawLevel {
    effort: String,
    #[serde(default)]
    description: Option<String>,
}

fn home() -> PathBuf {
    std::env::var_os("HOME")
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from("/"))
}

pub fn path() -> PathBuf {
    std::env::var_os("CODEX_HOME")
        .map(PathBuf::from)
        .unwrap_or_else(|| home().join(".codex"))
        .join("models_cache.json")
}

impl ModelInfo {
    fn from_raw(raw: RawModel) -> Option<Self> {
        if raw.slug.trim().is_empty() {
            return None;
        }
        let efforts: Vec<(String, String)> = raw
            .supported_reasoning_levels
            .into_iter()
            .filter(|level| !level.effort.trim().is_empty())
            .map(|level| {
                (
                    level.effort,
                    level.description.unwrap_or_else(|| "reasoning effort".to_string()),
                )
            })
            .collect();
        Some(Self {
            display_name: raw.display_name.unwrap_or_else(|| raw.slug.clone()),
            description: raw.description.unwrap_or_default(),
            default_effort: raw.default_reasoning_level,
            listed: raw.visibility.as_deref() != Some("hide"),
            slug: raw.slug,
            efforts,
            context_window: raw.context_window,
        })
    }
}

/// Used only when the cache is missing, so the app remains usable.
fn fallback() -> Vec<ModelInfo> {
    const BASIC: &[(&str, &str)] = &[
        ("low", "Fast responses with lighter reasoning"),
        ("medium", "Balances speed and reasoning depth for everyday tasks"),
        ("high", "Greater reasoning depth for complex problems"),
    ];
    vec![ModelInfo {
        slug: "gpt-5.6-terra".to_string(),
        display_name: "GPT-5.6-Terra".to_string(),
        description: "Read from the built-in fallback; the Codex model cache was not found."
            .to_string(),
        efforts: BASIC
            .iter()
            .map(|(name, blurb)| ((*name).to_string(), (*blurb).to_string()))
            .collect(),
        default_effort: Some("medium".to_string()),
        listed: true,
        context_window: 272_000,
    }]
}

/// Every model the provider knows about, most capable first is not guaranteed.
pub fn load() -> Vec<ModelInfo> {
    let parsed: Vec<ModelInfo> = std::fs::read_to_string(path())
        .ok()
        .and_then(|raw| serde_json::from_str::<Cache>(&raw).ok())
        .map(|cache| {
            cache
                .models
                .into_iter()
                .filter_map(ModelInfo::from_raw)
                .collect()
        })
        .unwrap_or_default();
    if parsed.is_empty() {
        fallback()
    } else {
        parsed
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sample(slug: &str, efforts: &[&str], default: &str, visibility: &str) -> ModelInfo {
        ModelInfo {
            slug: slug.to_string(),
            display_name: slug.to_uppercase(),
            description: String::new(),
            efforts: efforts
                .iter()
                .map(|e| ((*e).to_string(), "blurb".to_string()))
                .collect(),
            default_effort: Some(default.to_string()),
            listed: visibility != "hide",
            context_window: 272_000,
        }
    }

    #[test]
    fn parses_the_cache_shape() {
        let raw = r#"{"models":[{"slug":"gpt-5.6-terra","display_name":"GPT-5.6-Terra",
            "description":"d","default_reasoning_level":"medium","visibility":"list",
            "context_window":272000,
            "supported_reasoning_levels":[{"effort":"low","description":"a"},
                                          {"effort":"ultra","description":"b"}]},
            {"slug":"secret","visibility":"hide","supported_reasoning_levels":[]}]}"#;
        let cache: Cache = serde_json::from_str(raw).expect("parse");
        let models: Vec<ModelInfo> = cache
            .models
            .into_iter()
            .filter_map(ModelInfo::from_raw)
            .collect();
        assert_eq!(models.len(), 2);
        assert_eq!(models[0].efforts.len(), 2);
        assert_eq!(models[0].efforts[1].0, "ultra");
        assert!(models[0].listed);
        assert!(!models[1].listed);
        assert_eq!(models[0].context_limit(), 272_000 * 95 / 100);
    }
}