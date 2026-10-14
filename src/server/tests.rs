use anyhow::{Context, Result};
use axum::Router;
use axum::extract::{Path, Query, State};
use axum::http::{HeaderMap, StatusCode};
use axum::response::sse::{Event, Sse};
use axum::response::{IntoResponse, Json, Response};
use axum::routing::{get, post};
use futures_util::StreamExt;
use std::collections::HashMap;
use std::convert::Infallible;
use std::path::PathBuf;
use std::sync::{Arc, Mutex};
use std::time::Instant;
use tokio::net::TcpListener;
use tokio::sync::mpsc;
use crate::api::{ApiEvent, CodexClient};
use crate::catalog;
use crate::daemon;
use crate::proto::{
    AnswerBody, Health, LoginBody, PromptBody, RewindBody, ScopedBody, SessionSummary, WireAnswerRequest,
    WireEvent, WireMsg, WireMsgKind, WireEffort, WireModel, WirePhase, WireProvider, WireSnapshot,
    WireUsage,
};
use crate::protocol::{ContentPart, InputItem, Reasoning, ResponsesRequest, Usage};
use crate::tools;
use crate::tools::AskSlot;
use crate::store;

use super::*;

#[cfg(test)]
mod tests {
    use super::*;

    /// Old tool output is dropped before a request goes out; the last turns
    /// stay whole, and the note names the call it replaced.
    #[test]
    fn old_tool_output_is_dropped_from_requests() {
        let big = "x".repeat(60_000);
        let mut history = Vec::new();
        for turn in 0..5 {
            history.push(InputItem::user_text(format!("question {turn}")));
            history.push(InputItem::FunctionCall {
                call_id: format!("call-{turn}"),
                name: "read".to_string(),
                arguments: format!("{{\"path\": \"file-{turn}.rs\"}}"),
            });
            history.push(InputItem::FunctionCallOutput {
                call_id: format!("call-{turn}"),
                output: big.clone(),
            });
        }
        let before: usize = history.iter().map(item_chars).sum();
        assert!(before > PRUNE_AT_CHARS, "the fixture must be over the limit");

        let dropped = prune_history(&mut history);
        let after: usize = history.iter().map(item_chars).sum();
        assert!(dropped >= 3, "the oldest outputs go first: {dropped}");
        assert!(before - after > 150_000, "most of the bulk is gone: {}", before - after);
        assert!(after < PRUNE_AT_CHARS, "{after}");

        let outputs: Vec<&String> = history
            .iter()
            .filter_map(|item| match item {
                InputItem::FunctionCallOutput { output, .. } => Some(output),
                _ => None,
            })
            .collect();
        assert_eq!(outputs.len(), 5);
        assert!(outputs[3] == &big && outputs[4] == &big, "the working set stays whole");
        assert!(outputs[0].contains("dropped"), "{}", outputs[0]);
        assert!(outputs[0].contains("read file-0.rs"), "the note names the call: {}", outputs[0]);
    }

    /// One long turn: the oldest results go, the last few stay whole, so a
    /// twenty-call turn cannot overflow the window before it ends.
    #[test]
    fn a_single_long_turn_is_pruned_too() {
        let big = "y".repeat(40_000);
        let mut history = vec![InputItem::user_text("do the thing")];
        for call in 0..10 {
            history.push(InputItem::FunctionCall {
                call_id: format!("call-{call}"),
                name: "shell".to_string(),
                arguments: "{\"command\": [\"ls\"]}".to_string(),
            });
            history.push(InputItem::FunctionCallOutput {
                call_id: format!("call-{call}"),
                output: big.clone(),
            });
        }
        let before: usize = history.iter().map(item_chars).sum();
        assert!(before > PRUNE_AT_CHARS, "{before}");

        let dropped = prune_history(&mut history);
        let after: usize = history.iter().map(item_chars).sum();
        assert!(dropped >= 2, "{dropped}");
        assert!(before - after > 80_000, "{}", before - after);
        assert!(after < before);
    }

    /// The project header is an id, never a path: a path with Cyrillic in it
    /// cannot be routed, which is what broke every turn in that directory.
    #[test]
    fn the_gateway_project_id_is_ascii_whatever_the_path() {
        let id = gateway_project_id(std::path::Path::new("/home/user/projects/test/аквариум"));
        assert!(id.chars().all(|ch| ch.is_ascii_alphanumeric()), "{id}");
        assert_eq!(id.len(), 16, "{id}");
        assert_ne!(
            id,
            gateway_project_id(std::path::Path::new("/home/user/projects/test/other"))
        );
    }

    /// The session header carries the shape the gateways route by: `ses_` and
    /// 64 hex digits, whatever the session id looks like.
    #[test]
    fn the_gateway_session_id_has_the_shape_they_route_by() {
        let id = gateway_session_id("3a0a41f9-e2b9-4c9f-ba3a-3015b3a923a7");
        assert!(id.starts_with("ses_"), "{id}");
        assert_eq!(id.len(), 68, "{id}");
        let hex = &id[4..];
        assert_eq!(hex.len(), 64);
        assert!(hex.chars().all(|ch| ch.is_ascii_hexdigit()), "{id}");
        // Two sessions must not collide.
        assert_ne!(id, gateway_session_id("11111111-2222-3333-4444-555555555555"));
    }

    /// A tool preview is cut at a character boundary. The old code truncated at
    /// byte 400, which panics inside a multi-byte character; this sample is
    /// built so byte 400 lands in the middle of a Cyrillic letter.
    #[test]
    fn a_preview_never_splits_a_character() {
        // Each unit is 27 bytes and 400 = 27*14 + 22, which is inside `р`.
        let sample = "привет — мир!   ".repeat(200);
        assert!(!sample.is_char_boundary(400), "the sample must reproduce the trap");

        let shown = preview(&sample);
        assert!(shown.ends_with('…'), "{}", shown);
        let kept = shown.trim_end_matches('…').trim_end_matches('\n');
        assert!(sample.starts_with(kept), "nothing was mangled");
        assert!(shown.chars().count() <= 402, "cut at the 400th character");
    }

    /// A failure that is worth another attempt, and one that is not.
    #[test]
    fn only_transient_failures_are_retried() {
        assert!(transient("HTTP 500 Internal Server Error: upstream"));
        assert!(transient("error sending request for url (https://api.openai.com)"));
        assert!(transient("operation timed out after 25s"));
        assert!(transient("connection reset by peer"));
        assert!(transient("dns error: failed to lookup address information"));
        assert!(transient("Temporary failure: DNS lookup failed"));
        assert!(transient("temporary failure in name resolution"));
        assert!(!transient("HTTP 404: model not found"));
        // The gateway's own failure, wearing a 400.
        assert!(transient(
            "opencode-go: HTTP 400 Bad Request: {\"error\":{\"type\":\"server_error\",\"message\":\"Upstream response was not valid JSON\"}}"
        ));
        assert!(transient("HTTP 400: upstream response was not valid json"));
        assert!(!transient("HTTP 400: {'error':'unknown model'}"));
        assert!(transient("HTTP 429 Too Many Requests: rate limit reached"));
        assert!(!transient("HTTP 401: invalid api key"));
        assert!(!transient("unknown provider 'nope' — run /login"));
    }
}
