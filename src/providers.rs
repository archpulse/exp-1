//! Provider registry.
//!
//! The wire protocol is a property of the **model**, not the provider: a gateway
//! like opencode-go fronts three of them at once, routing grok-4.7 to
//! `/v1/responses`, claude-haiku-5-5 to `/v1/messages` and glm-5.3 to
//! `/v1/chat/completions`. So each provider lists its models with the protocol
//! each one speaks.
//!
//! Base URLs always include the version prefix; the protocol contributes only
//! the final path segment.
//!
//! Providers whose authentication is a first-party product flow rather than a
//! documented API are listed but marked unavailable, naming what is missing.

use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Wire {
    /// `{base}/chat/completions`
    OpenAiChat,
    /// `{base}/messages`, Anthropic-shaped.
    Anthropic,
    /// `{base}/responses`, the shape Codex uses.
    OpenAiResponses,
    /// Vertex AI's `generateContent`, which builds its own whole path.
    Gemini,
    /// The Codex backend, which also carries OAuth.
    CodexResponses,
}

impl Wire {
    /// The path appended to a base URL that already carries its version.
    pub fn suffix(self) -> &'static str {
        match self {
            Wire::OpenAiChat => "/chat/completions",
            Wire::Anthropic => "/messages",
            Wire::OpenAiResponses | Wire::CodexResponses => "/responses",
            // Vertex's path is built from project and location, so the base
            // contributes nothing here.
            Wire::Gemini => "",
        }
    }

    /// True for the protocols our own backends implement.
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Auth {
    /// Sent as `Authorization: Bearer`.
    Bearer,
    /// Sent as `x-api-key`.
    ApiKeyHeader,
    /// An OAuth access token, sent as `Authorization: Bearer` with the
    /// first-party client's identity headers.
    OAuthToken,
    /// Signed in through the vendor's device-code page.
    DeviceLogin,
    /// Credentials the machine already holds — gcloud's application default
    /// credentials, for instance. Nothing to store, nothing to paste.
    Ambient,
}

/// One model and the protocol it answers on.
pub type Model = (&'static str, Wire);

pub struct Provider {
    pub id: &'static str,
    pub label: &'static str,
    /// Always ends with the version prefix, e.g. `https://api.openai.com/v1`.
    pub base_url: &'static str,
    pub auth: Auth,
    pub headers: &'static [(&'static str, &'static str)],
    pub models: &'static [Model],
}

const NO_HEADERS: &[(&str, &str)] = &[];

/// Shorthand so the tables below stay readable.
const fn m(id: &'static str, wire: Wire) -> Model {
    (id, wire)
}

use Wire::{Anthropic, CodexResponses, Gemini, OpenAiChat, OpenAiResponses};

pub const PROVIDERS: &[Provider] = &[
    Provider {
        id: "openai-auth",
        label: "openai (auth)",
        base_url: "https://chatgpt.com/backend-api/codex",
        // Runs the ChatGPT device flow itself rather than relying on the Codex
        // CLI having signed in already.
        auth: Auth::DeviceLogin,
        headers: NO_HEADERS,
        models: &[
            m("gpt-5.6-terra", CodexResponses),
            m("gpt-5.6-luna", CodexResponses),
            m("gpt-6-luna", CodexResponses),
        ],
    },
    Provider {
        id: "openai-api",
        label: "openai (api)",
        base_url: "https://api.openai.com/v1",
        auth: Auth::Bearer,
        headers: NO_HEADERS,
        models: &[
            m("gpt-5.1", OpenAiResponses),
            m("gpt-5-mini", OpenAiResponses),
            m("gpt-4o", OpenAiChat),
            m("gpt-4o-mini", OpenAiChat),
        ],
    },
    Provider {
        id: "anthropic",
        label: "anthropic (api)",
        base_url: "https://api.anthropic.com/v1",
        auth: Auth::ApiKeyHeader,
        headers: &[("anthropic-version", "2023-06-01")],
        models: &[
            m("claude-sonnet-4-5", Anthropic),
            m("claude-opus-4-1", Anthropic),
            m("claude-haiku-4-5", Anthropic),
        ],
    },
    Provider {
        id: "gemini",
        label: "google gemini",
        base_url: "https://generativelanguage.googleapis.com/v1beta/openai",
        auth: Auth::Bearer,
        headers: NO_HEADERS,
        models: &[
            m("gemini-2.5-pro", OpenAiChat),
            m("gemini-2.5-flash", OpenAiChat),
        ],
    },
    Provider {
        id: "deepseek",
        label: "deepseek",
        base_url: "https://api.deepseek.com/v1",
        auth: Auth::Bearer,
        headers: NO_HEADERS,
        models: &[
            m("deepseek-chat", OpenAiChat),
            m("deepseek-reasoner", OpenAiChat),
        ],
    },
    Provider {
        id: "kimi",
        label: "kimi",
        base_url: "https://api.moonshot.ai/v1",
        auth: Auth::Bearer,
        headers: NO_HEADERS,
        models: &[
            m("kimi-k2-0905-preview", OpenAiChat),
            m("moonshot-v1-128k", OpenAiChat),
        ],
    },
    Provider {
        id: "qwen",
        label: "qwen",
        base_url: "https://dashscope.aliyuncs.com/compatible-mode/v1",
        auth: Auth::Bearer,
        headers: NO_HEADERS,
        models: &[
            m("qwen3-max", OpenAiChat),
            m("qwen3-coder-plus", OpenAiChat),
            m("qwen-plus", OpenAiChat),
        ],
    },
    Provider {
        id: "minimax",
        label: "minimax",
        base_url: "https://api.minimax.io/anthropic/v1",
        auth: Auth::Bearer,
        headers: NO_HEADERS,
        models: &[m("MiniMax-M2", Anthropic), m("MiniMax-Text-01", Anthropic)],
    },
    Provider {
        id: "stepfun",
        label: "stepfun",
        base_url: "https://api.stepfun.com/v1",
        auth: Auth::Bearer,
        headers: NO_HEADERS,
        models: &[m("step-2-16k", OpenAiChat), m("step-1v-8k", OpenAiChat)],
    },
    Provider {
        id: "fireworks",
        label: "fireworks-ai",
        base_url: "https://api.fireworks.ai/inference/v1",
        auth: Auth::Bearer,
        headers: NO_HEADERS,
        models: &[m(
            "accounts/fireworks/models/llama-v3p3-70b-instruct",
            OpenAiChat,
        )],
    },
    Provider {
        id: "openrouter",
        label: "openrouter",
        base_url: "https://openrouter.ai/api/v1",
        auth: Auth::Bearer,
        headers: &[
            ("HTTP-Referer", "https://github.com/exp-1-agent"),
            ("X-Title", "exp-1 agent"),
        ],
        models: &[
            m("anthropic/claude-sonnet-4.5", OpenAiChat),
            m("openai/gpt-5.1", OpenAiChat),
            m("z-ai/glm-4.6", OpenAiChat),
        ],
    },
    Provider {
        id: "groq",
        label: "groq",
        base_url: "https://api.groq.com/openai/v1",
        auth: Auth::Bearer,
        headers: NO_HEADERS,
        models: &[
            m("llama-3.3-70b-versatile", OpenAiChat),
            m("moonshotai/kimi-k2-instruct", OpenAiChat),
        ],
    },
    Provider {
        id: "grok",
        label: "grok",
        base_url: "https://api.x.ai/v1",
        auth: Auth::Bearer,
        headers: NO_HEADERS,
        models: &[
            m("grok-4", OpenAiChat),
            m("grok-4-fast", OpenAiChat),
            m("grok-code-fast-1", OpenAiChat),
        ],
    },
    Provider {
        id: "github-copilot",
        label: "github copilot",
        // Replaced at request time with the host named by the Copilot token.
        base_url: "https://api.individual.githubcopilot.com",
        auth: Auth::DeviceLogin,
        headers: &[
            ("editor-version", "vscode/1.107.0"),
            ("editor-plugin-version", "copilot-chat/0.35.0"),
            ("copilot-integration-id", "vscode-chat"),
            ("x-github-api-version", "2026-06-01"),
        ],
        models: &[
            m("gpt-5.4", OpenAiChat),
            m("claude-sonnet-4.5", OpenAiChat),
            m("gpt-4o", OpenAiChat),
        ],
    },
    Provider {
        id: "meta-auth",
        label: "meta (auth)",
        base_url: "https://api.meta.ai/v1",
        auth: Auth::DeviceLogin,
        headers: NO_HEADERS,
        models: &[
            m("muse-spark-1.3", OpenAiChat),
            m("muse-spark-1.2", OpenAiChat),
        ],
    },
    // GLM is Zhipu AI's family. Z.AI serves it worldwide, Zhipu serves it in
    // China, and each runs a separate coding-plan endpoint for subscribers.
    Provider {
        id: "glm",
        label: "glm (z.ai)",
        base_url: "https://api.z.ai/api/paas/v4",
        auth: Auth::Bearer,
        headers: NO_HEADERS,
        models: &[
            m("glm-5.3", OpenAiChat),
            m("glm-5.3-flash", OpenAiChat),
            m("glm-5.3-flashx", OpenAiChat),
            m("glm-5.2", OpenAiChat),
            m("glm-5", OpenAiChat),
            m("glm-4.6", OpenAiChat),
        ],
    },
    Provider {
        id: "glm-coding-plan",
        label: "glm coding plan (z.ai)",
        base_url: "https://api.z.ai/api/coding/paas/v4",
        auth: Auth::Bearer,
        headers: NO_HEADERS,
        models: &[
            m("glm-5.3", OpenAiChat),
            m("glm-5.3-highspeed", OpenAiChat),
            m("glm-5.3-flash", OpenAiChat),
            m("glm-5.2", OpenAiChat),
        ],
    },
    Provider {
        id: "glm-cn",
        label: "glm (zhipu, cn)",
        base_url: "https://open.bigmodel.cn/api/paas/v4",
        auth: Auth::Bearer,
        headers: NO_HEADERS,
        models: &[
            m("glm-5.3", OpenAiChat),
            m("glm-5.3-flash", OpenAiChat),
            m("glm-5.3-flashx", OpenAiChat),
            m("glm-5.2", OpenAiChat),
            m("glm-5.1", OpenAiChat),
            m("glm-5", OpenAiChat),
        ],
    },
    Provider {
        id: "glm-cn-coding-plan",
        label: "glm coding plan (zhipu, cn)",
        base_url: "https://open.bigmodel.cn/api/coding/paas/v4",
        auth: Auth::Bearer,
        headers: NO_HEADERS,
        models: &[
            m("glm-5.3", OpenAiChat),
            m("glm-5.3-highspeed", OpenAiChat),
            m("glm-5.3-flash", OpenAiChat),
        ],
    },
    Provider {
        id: "muse-spark",
        label: "muse spark",
        base_url: "https://api.meta.ai/v1",
        auth: Auth::Bearer,
        headers: NO_HEADERS,
        models: &[
            m("muse-spark-1.3", OpenAiChat),
            m("muse-spark-1.2", OpenAiChat),
            m("muse-spark-1.1", OpenAiChat),
        ],
    },
    Provider {
        id: "cline-pass",
        label: "clinepass",
        base_url: "https://api.cline.bot/api/v1",
        auth: Auth::Bearer,
        headers: NO_HEADERS,
        models: &[
            m("cline-pass/glm-5.3-flash", OpenAiChat),
            m("cline-pass/qwen3.7-max", OpenAiChat),
        ],
    },
    Provider {
        id: "agentrouter",
        label: "agentrouter",
        base_url: "https://agentrouter.org/v1",
        auth: Auth::Bearer,
        headers: NO_HEADERS,
        models: &[m("claude-sonnet-4-5", OpenAiChat), m("gpt-5", OpenAiChat)],
    },
    Provider {
        id: "opencode-zen",
        label: "opencode-zen",
        base_url: "https://opencode.ai/zen/v1",
        auth: Auth::Bearer,
        headers: NO_HEADERS,
        models: &[
            m("claude-sonnet-4-5", OpenAiChat),
            m("gpt-5-codex", OpenAiChat),
            m("kimi-k2", OpenAiChat),
        ],
    },
    // The hybrid gateway: three protocols behind one base URL.
    Provider {
        id: "opencode-go",
        label: "opencode-go",
        base_url: "https://opencode.ai/zen/go/v1",
        auth: Auth::Bearer,
        headers: NO_HEADERS,
        models: &[
            m("grok-4.7", OpenAiResponses),
            m("grok-4.6", OpenAiResponses),
            m("gpt-6-luna", OpenAiResponses),
            m("gpt-5.6-luna", OpenAiResponses),
            m("muse-spark-1.3-contributor", OpenAiResponses),
            m("muse-spark-1.2-contributor", OpenAiResponses),
            m("claude-haiku-5-5", Anthropic),
            m("minimax-m3", Anthropic),
            m("minimax-m2.7", Anthropic),
            m("qwen3.8-max", Anthropic),
            m("qwen3.8-flash", Anthropic),
            m("qwen3.7-plus", Anthropic),
            m("glm-5.3", OpenAiChat),
            m("glm-5.3-flash", OpenAiChat),
            m("glm-5.2", OpenAiChat),
            m("kimi-k3", OpenAiChat),
            m("kimi-k2.7-code", OpenAiChat),
            m("deepseek-v4-pro", OpenAiChat),
            m("deepseek-v4-flash", OpenAiChat),
            m("deepseek-v4-flash-vision-exp", OpenAiChat),
            m("deepseek-v4.1-flash", OpenAiChat),
            m("mimo-v2.6-pro", OpenAiChat),
            m("mimo-v2.6-flash", OpenAiChat),
            m("mimo-v2.5", OpenAiChat),
            m("mimo-v2.5-pro", OpenAiChat),
            m("longcat-2.0", OpenAiChat),
            m("longcat-2.5-preview-free", OpenAiChat),
            m("step-5-preview-free", OpenAiChat),
            m("hy3", OpenAiChat),
            m("hy4-preview", OpenAiChat),
            m("space-bunny", OpenAiChat),
        ],
    },
    // --- listed so the picker is complete, but honestly not implemented ---
    Provider {
        id: "anthropic-auth",
        label: "anthropic (auth)",
        base_url: "https://api.anthropic.com/v1",
        auth: Auth::OAuthToken,
        headers: &[("anthropic-version", "2023-06-01")],
        models: &[
            m("claude-sonnet-4-5", Anthropic),
            m("claude-opus-4-1", Anthropic),
            m("claude-haiku-4-5", Anthropic),
        ],
    },
    Provider {
        id: "vertex",
        label: "google vertex",
        // Only the host varies with the location; `request_target` builds the
        // rest of the path from the project and location in the environment.
        base_url: "https://aiplatform.googleapis.com/v1",
        auth: Auth::Ambient,
        headers: NO_HEADERS,
        models: &[
            m("gemini-2.5-pro", Gemini),
            m("gemini-2.5-flash", Gemini),
            m("claude-sonnet-4-5", Gemini),
        ],
    },
    Provider {
        id: "gitlab-duo",
        label: "gitlab-duo",
        // GitLab proxies Claude through its own gateway, so the Anthropic body
        // shape applies with a different host and token.
        base_url: "https://cloud.gitlab.com/ai/v1/proxy/anthropic/v1",
        auth: Auth::Bearer,
        headers: NO_HEADERS,
        models: &[
            m("claude-sonnet-4-5", Anthropic),
            m("claude-haiku-4-5", Anthropic),
        ],
    },
];

/// The names the first-party Claude Code client uses, which the OAuth backend
/// expects in canonical casing.
const CLAUDE_CODE_TOOLS: [&str; 17] = [
    "Read",
    "Write",
    "Edit",
    "Bash",
    "Grep",
    "Glob",
    "AskUserQuestion",
    "EnterPlanMode",
    "ExitPlanMode",
    "KillShell",
    "NotebookEdit",
    "Skill",
    "Task",
    "TaskOutput",
    "TodoWrite",
    "WebFetch",
    "WebSearch",
];

/// Case-insensitive match against the first-party tool names.
pub fn claude_code_name(name: &str) -> String {
    CLAUDE_CODE_TOOLS
        .iter()
        .find(|known| known.eq_ignore_ascii_case(name))
        .map(|known| (*known).to_string())
        .unwrap_or_else(|| name.to_string())
}

/// True when a token is an OAuth access token rather than an API key.
pub fn is_oauth_token(token: &str) -> bool {
    token.contains("sk-ant-oat")
}

pub fn by_id(id: &str) -> Option<&'static Provider> {
    PROVIDERS.iter().find(|provider| provider.id == id)
}

/// The id a provider has in the models.dev catalogue, when it is listed there.
/// Its entry supplies the live model list and the protocol of each model; a
/// provider without one keeps the table above.
pub fn catalog_id(id: &str) -> Option<&'static str> {
    Some(match id {
        "openai-api" => "openai",
        "anthropic" | "anthropic-auth" => "anthropic",
        "gemini" => "google",
        "vertex" => "google-vertex",
        "deepseek" => "deepseek",
        "kimi" => "moonshotai",
        "qwen" => "alibaba-cn",
        "minimax" => "minimax-coding-plan",
        "stepfun" => "stepfun",
        "fireworks" => "fireworks-ai",
        "openrouter" => "openrouter",
        "groq" => "groq",
        "grok" => "xai",
        "github-copilot" => "github-copilot",
        "meta-auth" | "muse-spark" => "meta",
        "glm" => "zai",
        "glm-coding-plan" => "zai-coding-plan",
        "glm-cn" => "zhipuai",
        "glm-cn-coding-plan" => "zhipuai-coding-plan",
        "cline-pass" => "cline-pass",
        "agentrouter" => "agentrouter",
        "opencode-zen" => "opencode",
        "opencode-go" => "opencode-go",
        _ => return None,
    })
}

/// Providers that accept the same key: signing into one signs into all of
/// them. The opencode gateways are one account behind two base URLs.
pub fn key_group(id: &str) -> Option<&'static str> {
    match id {
        "opencode-go" | "opencode-zen" => Some("opencode"),
        _ => None,
    }
}

/// The wire a model id implies, for one no table lists. Gateways route by
/// upstream vendor, so the prefix is what gives it away.
fn infer_wire(model: &str) -> Option<Wire> {
    Some(match model.split(['-', '.']).next().unwrap_or(model) {
        "claude" | "minimax" | "qwen" => Wire::Anthropic,
        "grok" | "gpt" | "muse" => Wire::OpenAiResponses,
        "gemini" => Wire::Gemini,
        _ => return None,
    })
}

/// The provider used before anyone logs in.
pub const DEFAULT_PROVIDER: &str = "openai-auth";

impl Provider {
    pub fn needs_key(&self) -> bool {
        matches!(self.auth, Auth::Bearer | Auth::ApiKeyHeader | Auth::OAuthToken)
    }

    /// True when `/login` has to run a device flow rather than ask for a key.
    pub fn needs_device_login(&self) -> bool {
        matches!(self.auth, Auth::DeviceLogin)
    }

    /// The protocol a given model answers on, falling back to the first entry.
    pub fn wire_for(&self, model: &str) -> Wire {
        if let Some((_, wire)) = self.models.iter().find(|(id, _)| *id == model) {
            return *wire;
        }
        // A model the table does not know: the live catalogue says which
        // protocol it speaks, then the family the id belongs to, then, for a
        // provider that speaks one protocol throughout, that one.
        if let Some(catalog) = catalog_id(self.id)
            && let Some(wire) = crate::models_dev::wire_for(catalog, model)
        {
            return wire;
        }
        if let Some(wire) = infer_wire(model) {
            return wire;
        }
        let mut wires = self.models.iter().map(|(_, wire)| *wire);
        match (wires.next(), wires.next()) {
            (Some(only), None) => only,
            _ => OpenAiChat,
        }
    }

    /// The full URL for a model.
    pub fn endpoint_for(&self, model: &str) -> String {
        format!(
            "{}{}",
            self.base_url.trim_end_matches('/'),
            self.wire_for(model).suffix()
        )
    }

    /// Every model just as ids, for pickers and scoping.
    pub fn model_ids(&self) -> Vec<String> {
        self.models.iter().map(|(id, _)| (*id).to_string()).collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ids_are_unique() {
        let mut seen = std::collections::HashSet::new();
        for provider in PROVIDERS {
            assert!(seen.insert(provider.id), "duplicate id {}", provider.id);
        }
    }

    /// The opencode gateways are one account, so one key signs into both.
    #[test]
    fn the_opencode_gateways_share_one_key() {
        assert_eq!(key_group("opencode-go"), Some("opencode"));
        assert_eq!(key_group("opencode-zen"), key_group("opencode-go"));
        assert_eq!(key_group("anthropic"), None);
    }

    /// A model no table lists still gets a protocol: the vendor decides.
    #[test]
    fn a_model_the_table_misses_falls_back_by_family() {
        let go = by_id("opencode-go").expect("opencode-go");
        // Known models keep the table's answer, whatever a heuristic says.
        assert_eq!(go.wire_for("claude-haiku-5-5"), Wire::Anthropic);
        assert_eq!(go.wire_for("glm-5.3"), Wire::OpenAiChat);
        assert_eq!(go.wire_for("grok-4.7"), Wire::OpenAiResponses);
        // Unknown ones follow their vendor, and anything else is chat, since a
        // gateway that speaks several protocols has no single default.
        assert_eq!(go.wire_for("claude-opus-9"), Wire::Anthropic);
        assert_eq!(go.wire_for("grok-9"), Wire::OpenAiResponses);
        assert_eq!(go.wire_for("brand-new-thing"), Wire::OpenAiChat);
    }

    /// Providers listed in models.dev carry the id they are filed under there.
    #[test]
    fn catalog_ids_are_mapped_where_the_names_differ() {
        assert_eq!(catalog_id("opencode-zen"), Some("opencode"));
        assert_eq!(catalog_id("opencode-go"), Some("opencode-go"));
        assert_eq!(catalog_id("gemini"), Some("google"));
        assert_eq!(catalog_id("openai-auth"), None);
    }

    #[test]
    fn every_provider_is_usable() {
        for provider in PROVIDERS {
            assert!(
                provider.base_url.starts_with("https://"),
                "{} has no https base url",
                provider.id
            );
            assert!(!provider.models.is_empty(), "{} lists no models", provider.id);
            assert!(
                provider.auth != Auth::Ambient || !provider.needs_key(),
                "{} asks for a key it cannot store",
                provider.id
            );
        }
    }

    #[test]
    fn the_path_comes_from_the_model() {
        let anthropic = by_id("anthropic").expect("anthropic");
        assert_eq!(
            anthropic.endpoint_for("claude-sonnet-4-5"),
            "https://api.anthropic.com/v1/messages"
        );
        let deepseek = by_id("deepseek").expect("deepseek");
        assert_eq!(
            deepseek.endpoint_for("deepseek-chat"),
            "https://api.deepseek.com/v1/chat/completions"
        );
        let codex = by_id("openai-auth").expect("codex");
        assert_eq!(
            codex.endpoint_for("gpt-5.6-terra"),
            "https://chatgpt.com/backend-api/codex/responses"
        );
    }

    #[test]
    fn a_gateway_can_serve_several_protocols_at_once() {
        let go = by_id("opencode-go").expect("opencode-go");
        assert_eq!(
            go.endpoint_for("grok-4.7"),
            "https://opencode.ai/zen/go/v1/responses"
        );
        assert_eq!(
            go.endpoint_for("claude-haiku-5-5"),
            "https://opencode.ai/zen/go/v1/messages"
        );
        assert_eq!(
            go.endpoint_for("glm-5.3"),
            "https://opencode.ai/zen/go/v1/chat/completions"
        );
        // Three different protocols behind one provider.
        let wires: std::collections::HashSet<Wire> =
            go.models.iter().map(|(_, wire)| *wire).collect();
        assert!(wires.len() >= 3, "expected a hybrid gateway");
    }

    #[test]
    fn the_default_provider_exists() {
        assert!(by_id(DEFAULT_PROVIDER).is_some());
    }

    /// Every wire a provider advertises must be one the server can actually
    /// send: the shared backends, or the Codex client for `CodexResponses`.
    #[test]
    fn every_listed_wire_has_a_backend() {
        for provider in PROVIDERS {
            for (model, wire) in provider.models {
                let backed = wire.is_remote() || *wire == Wire::CodexResponses;
                assert!(
                    backed,
                    "{}/{} uses {wire:?}, which has no backend",
                    provider.id, model
                );
            }
        }
    }


    /// Claude Code names tools in canonical casing; the OAuth backend rejects
    /// anything else.
    #[test]
    fn claude_code_tool_casing() {
        assert_eq!(claude_code_name("read"), "Read");
        assert_eq!(claude_code_name("write"), "Write");
        assert_eq!(claude_code_name("edit"), "Edit");
        assert_eq!(claude_code_name("task"), "Task");
        assert_eq!(claude_code_name("websearch"), "WebSearch");
        // Anything the first-party client does not have is left alone.
        assert_eq!(claude_code_name("shell"), "shell");
        assert_eq!(claude_code_name("ast"), "ast");
        assert_eq!(claude_code_name("ask_question"), "ask_question");
    }
}