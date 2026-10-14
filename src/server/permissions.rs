//! permission rules and plan mode.

use std::sync::Arc;
use crate::tools;

use super::*;

/// In plan mode the tools that change files are refused before they run.
pub(crate) fn plan_refusal(session: &Arc<Session>, name: &str) -> Option<String> {
    if session.mode() != "plan" {
        return None;
    }
    matches!(name, "write" | "edit" | "delete").then(|| {
        format!("{name}: refused — the session is in plan mode (shift+tab to leave it)")
    })
}

/// What a permission rule matches for this call: the path for file tools, the
/// command line for shell, the URL or query for the net tools.
pub(crate) fn permission_resource(name: &str, arguments: &str, cwd: &std::path::Path) -> String {
    let Ok(value) = serde_json::from_str::<serde_json::Value>(arguments) else {
        return arguments.to_string();
    };
    let text = |key: &str| {
        value
            .get(key)
            .and_then(|field| field.as_str())
            .map(str::to_string)
    };
    match name {
        "read" | "write" | "edit" | "delete" | "count" | "grep" => text("path")
            .map(|raw| {
                let path = std::path::Path::new(&raw);
                if path.is_absolute() {
                    raw
                } else {
                    cwd.join(path).display().to_string()
                }
            })
            .unwrap_or_else(|| arguments.to_string()),
        "shell" => value
            .get("command")
            .and_then(|command| command.as_array())
            .map(|items| {
                items
                    .iter()
                    .filter_map(|item| item.as_str())
                    .collect::<Vec<_>>()
                    .join(" ")
            })
            .unwrap_or_else(|| arguments.to_string()),
        "fetch" => text("url").unwrap_or_else(|| arguments.to_string()),
        "websearch" => text("query").unwrap_or_else(|| arguments.to_string()),
        _ => arguments.to_string(),
    }
}

/// Asks the rules file, and when it says so the user, whether a call may run.
/// `None` means it may; `Some(reason)` is what to hand back instead.
pub(crate) async fn permission_check(
    session: &Arc<Session>,
    name: &str,
    arguments: &str,
) -> Option<String> {
    let resource = permission_resource(name, arguments, &session.cwd());
    let rules = tools::load_rules();
    let decision = rules.decide(name, &resource).map(|rule| rule.effect.as_str());
    match session.permissions().verdict(decision, name) {
        tools::Verdict::Deny => Some(format!(
            "{name}: not permitted — permissions.json denies `{resource}`"
        )),
        tools::Verdict::Run => None,
        tools::Verdict::Ask => {
            let question = tools::AskItem {
                label: "Permission".to_string(),
                text: format!("Allow `{name}` on `{resource}`?"),
                options: vec!["Allow".to_string(), "Deny".to_string()],
            };
            let allowed = session
                .ask
                .ask_within(vec![question], "Submit".to_string(), tools::PERMISSION_TIMEOUT)
                .await
                .and_then(|answers| answers.first().cloned())
                .map(|answer| {
                    let answer = answer.to_ascii_lowercase();
                    answer.contains("allow") || matches!(answer.trim(), "y" | "yes")
                })
                .unwrap_or(false);
            if allowed {
                None
            } else {
                Some(format!("{name}: refused by the user (`{resource}`)"))
            }
        }
    }
}
