//! fetch and websearch, with their parsers.

//! Tool implementations: read, write, edit, shell, ast.
//!
//! All paths are resolved against the session working directory. Nothing is
//! sandboxed: commands run as the user.

use super::*;
use serde_json::{Value, json};
use std::io::Read;

/// A foreground command still running after this long is promoted to a
/// background task, so the agent is never blocked on something slow. A
/// background task has no clock on it: builds and test suites run for minutes,
/// and the old 120s axe was cutting them off mid-flight.

// ---------------------------------------------------------------- websearch

/// Percent-encodes just enough for an API key in a query string.
pub(crate) fn encode_query_value(value: &str) -> String {
    let mut out = String::with_capacity(value.len());
    for byte in value.bytes() {
        match byte {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'_' | b'.' | b'~' => {
                out.push(byte as char)
            }
            other => out.push_str(&format!("%{other:02X}")),
        }
    }
    out
}

fn exa_endpoint() -> String {
    match std::env::var("EXA_API_KEY") {
        Ok(key) if !key.trim().is_empty() => {
            format!("{EXA_MCP}?exaApiKey={}", encode_query_value(key.trim()))
        }
        _ => EXA_MCP.to_string(),
    }
}

/// Pulls the text out of one JSON-RPC payload.
fn from_payload(payload: &str) -> Option<Result<String, String>> {
    if !payload.starts_with('{') {
        return None;
    }
    let value: Value = serde_json::from_str(payload).ok()?;
    if let Some(error) = value.get("error") {
        let message = error
            .get("message")
            .and_then(Value::as_str)
            .unwrap_or("search failed");
        return Some(Err(message.to_string()));
    }
    let text = value
        .get("result")?
        .get("content")?
        .as_array()?
        .iter()
        .find_map(|item| item.get("text").and_then(Value::as_str))
        .map(str::to_string)?;
    Some(Ok(text))
}

/// The MCP server answers either as plain JSON or as an SSE stream.
pub(crate) fn extract_search_text(body: &str) -> Option<Result<String, String>> {
    if let Some(found) = from_payload(body.trim()) {
        return Some(found);
    }
    for line in body.lines() {
        if let Some(rest) = line.trim().strip_prefix("data: ")
            && let Some(found) = from_payload(rest.trim())
        {
            return Some(found);
        }
    }
    None
}

/// HTML to readable text: scripts and styles dropped, tags removed, entities
/// unescaped, whitespace collapsed, so the result reads like an article.
fn html_to_text(html: &str) -> String {
    let mut out = String::with_capacity(html.len() / 2);
    let mut index = 0usize;
    while index < html.len() {
        if html.as_bytes()[index] == b'<' {
            let rest = &html[index..];
            let head = rest[..rest.len().min(64)].to_ascii_lowercase();
            let lower = rest.to_ascii_lowercase();
            let skip = if head.starts_with("<script") {
                lower.find("</script>").map(|at| at + 9)
            } else if head.starts_with("<style") {
                lower.find("</style>").map(|at| at + 8)
            } else {
                None
            };
            if let Some(skip) = skip {
                index += skip;
                continue;
            }
            match rest.find('>') {
                Some(at) => index += at + 1,
                None => break,
            }
            out.push(' ');
            continue;
        }
        let ch = html[index..].chars().next().unwrap_or(' ');
        out.push(ch);
        index += ch.len_utf8();
    }
    let unescaped = unescape_html(&out);
    let mut lines: Vec<String> = Vec::new();
    for line in unescaped.lines() {
        let trimmed = line.split_whitespace().collect::<Vec<_>>().join(" ");
        if trimmed.is_empty() {
            if lines.last().is_some_and(|last| !last.is_empty()) {
                lines.push(String::new());
            }
        } else {
            lines.push(trimmed);
        }
    }
    lines.join("\n").trim().to_string()
}

/// Reads one page as text, paged the way `read` pages a file: `limit` is a
/// maximum number of lines and `pos` is the 1-based line to start from, so a
/// long document is walked through instead of flooding the conversation.
pub async fn fetch(http: &reqwest::Client, args: &Value) -> Outcome {
    let Some(url) = args.get("url").and_then(Value::as_str) else {
        return err("fetch: 'url' is required");
    };
    if !url.starts_with("http://") && !url.starts_with("https://") {
        return err(format!("fetch: '{url}' is not an http(s) URL"));
    }
    let limit = args
        .get("limit")
        .and_then(Value::as_u64)
        .unwrap_or(FETCH_DEFAULT_LIMIT as u64)
        .clamp(1, 5_000) as usize;
    let pos = args.get("pos").and_then(Value::as_u64).unwrap_or(1).max(1) as usize;

    let request = http
        .get(url)
        .header("User-Agent", BROWSER_UA)
        .header("Accept", "text/html,application/xhtml+xml,text/plain;q=0.9,*/*;q=0.8");
    let response = match tokio::time::timeout(SEARCH_TIMEOUT, request.send()).await {
        Err(_) => return err(format!("fetch: timed out after {}s", SEARCH_TIMEOUT.as_secs())),
        Ok(Err(e)) => return err(format!("fetch: request failed: {e}")),
        Ok(Ok(response)) => response,
    };
    let status = response.status();
    let body = response.text().await.unwrap_or_default();
    if !status.is_success() {
        return err(format!(
            "fetch: HTTP {status}: {}",
            body.chars().take(200).collect::<String>()
        ));
    }

    let text = if body.trim_start().starts_with('<') {
        html_to_text(&body)
    } else {
        body.trim().to_string()
    };
    let lines: Vec<&str> = text.lines().collect();
    let total = lines.len();
    if total == 0 {
        return err(format!("fetch: {url} returned nothing readable"));
    }
    if pos > total {
        return err(format!(
            "fetch: {url}: line {pos} is past the end of the page ({total} lines)"
        ));
    }
    let taken: Vec<&str> = lines.iter().skip(pos - 1).take(limit).copied().collect();
    let mut body = taken.join("\n");
    if body.len() > MAX_OUTPUT {
        let end = cut(&body, MAX_OUTPUT).len();
        body.truncate(end);
        body.push_str("\n... [output truncated by exp-1 agent; fetch again with a further pos]");
    }
    let mut output = format!("{url} ({total} lines of text)\n\n{body}");
    if pos - 1 + taken.len() < total {
        output.push_str(&format!(
            "\n\n… [showing {} of {total} lines from {pos}; call fetch again with a larger limit or a further pos to continue]",
            taken.len()
        ));
    }
    Outcome {
        output,
        background_id: None,
    }
}

pub async fn websearch(http: &reqwest::Client, args: &Value) -> Outcome {
    let Some(query) = args.get("query").and_then(Value::as_str) else {
        return err("websearch: 'query' is required");
    };
    if query.trim().is_empty() {
        return err("websearch: 'query' is empty");
    }
    let limit = args
        .get("numResults")
        .and_then(Value::as_u64)
        .unwrap_or(8)
        .clamp(1, 20) as usize;
    let query = query.trim();

    // Exa first, because it returns page content rather than snippets. Its
    // keyless tier is shared per machine and does rate-limit, so DuckDuckGo
    // answers when it does; when DuckDuckGo blocks the request instead, the
    // two failures are reported together rather than one hiding the other.
    let mut failures: Vec<String> = Vec::new();
    match exa_search(http, query, args).await {
        Ok(text) => {
            return Outcome {
                output: text,
                background_id: None,
            };
        }
        Err(why) => failures.push(why),
    }
    match ddg_search(http, query, limit).await {
        Ok(text) => {
            return Outcome {
                output: text,
                background_id: None,
            };
        }
        Err(why) => failures.push(why),
    }
    err(format!(
        "websearch: no backend answered — {}. Set EXA_API_KEY to search under your own plan.",
        failures.join("; ")
    ))
}

/// Keyless search over DuckDuckGo's HTML page, parsed by hand: the markup is
/// a flat list of `result__a` links with a `result__snippet` beside each.
async fn ddg_search(http: &reqwest::Client, query: &str, limit: usize) -> Result<String, String> {
    let request = http
        .post(DDG_HTML)
        .header("User-Agent", BROWSER_UA)
        .header("Accept", "text/html,application/xhtml+xml")
        .form(&[("q", query), ("kl", "wt-wt")]);
    let response = match tokio::time::timeout(SEARCH_TIMEOUT, request.send()).await {
        Err(_) => return Err(format!("duckduckgo: timed out after {}s", SEARCH_TIMEOUT.as_secs())),
        Ok(Err(e)) => return Err(format!("duckduckgo: request failed: {e}")),
        Ok(Ok(response)) => response,
    };
    let status = response.status();
    let body = response.text().await.unwrap_or_default();
    if !status.is_success() {
        return Err(format!("duckduckgo: HTTP {status}"));
    }
    let results = parse_ddg(&body, limit);
    if results.is_empty() {
        return Err("duckduckgo: no results (blocked or rate limited)".to_string());
    }
    Ok(results.join("\n\n"))
}

/// Pulls titles, links and snippets out of the DuckDuckGo result page.
pub(crate) fn parse_ddg(html: &str, limit: usize) -> Vec<String> {
    let mut out: Vec<String> = Vec::new();
    let mut rest = html;
    while out.len() < limit {
        let Some(at) = rest.find("result__a") else {
            break;
        };
        rest = &rest[at..];
        let Some(href) = rest.find("href=\"") else {
            break;
        };
        let after = &rest[href + 6..];
        let Some(end) = after.find('"') else {
            break;
        };
        let url = decode_ddg_url(&after[..end]);
        let Some(gt) = after[end..].find('>') else {
            break;
        };
        let title_part = &after[end + gt + 1..];
        let Some(lt) = title_part.find("</a>") else {
            break;
        };
        let title = unescape_html(&strip_tags(&title_part[..lt]));
        rest = &title_part[lt..];
        let snippet = rest
            .find("result__snippet")
            .and_then(|at| {
                let after = &rest[at..];
                let gt = after.find('>')?;
                let text = &after[gt + 1..];
                let lt = text.find("</a>")?;
                Some(unescape_html(&strip_tags(&text[..lt])))
            })
            .unwrap_or_default();
        let number = out.len() + 1;
        if title.is_empty() {
            continue;
        }
        out.push(format!("{number}. {title}\n   {url}\n   {snippet}"));
    }
    out
}

/// Drops the inline tags DuckDuckGo bolds query words with.
fn strip_tags(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    let mut inside = false;
    for ch in text.chars() {
        match ch {
            '<' => inside = true,
            '>' => inside = false,
            _ if !inside => out.push(ch),
            _ => {}
        }
    }
    out.trim().to_string()
}

/// DuckDuckGo wraps outgoing links as `/l/?uddg=<percent-encoded>`.
fn decode_ddg_url(href: &str) -> String {
    let url = match href.split_once("uddg=") {
        Some((_, rest)) => percent_decode(rest.split('&').next().unwrap_or(rest)),
        None => href.to_string(),
    };
    if let Some(rest) = url.strip_prefix("//") {
        format!("https://{rest}")
    } else {
        url
    }
}

fn percent_decode(text: &str) -> String {
    let bytes = text.as_bytes();
    let mut out: Vec<u8> = Vec::with_capacity(bytes.len());
    let mut index = 0;
    while index < bytes.len() {
        match bytes[index] {
            b'%' if index + 2 < bytes.len() => {
                let hex = std::str::from_utf8(&bytes[index + 1..index + 3]).unwrap_or("");
                match u8::from_str_radix(hex, 16) {
                    Ok(byte) => {
                        out.push(byte);
                        index += 3;
                    }
                    Err(_) => {
                        out.push(bytes[index]);
                        index += 1;
                    }
                }
            }
            b'+' => {
                out.push(b' ');
                index += 1;
            }
            byte => {
                out.push(byte);
                index += 1;
            }
        }
    }
    String::from_utf8_lossy(&out).to_string()
}

fn unescape_html(text: &str) -> String {
    text.replace("&amp;", "&")
        .replace("&lt;", "<")
        .replace("&gt;", ">")
        .replace("&quot;", "\"")
        .replace("&#x27;", "'")
        .replace("&#39;", "'")
        .replace("&nbsp;", " ")
        .replace("&hellip;", "…")
        .replace("&mdash;", "—")
        .replace("&ndash;", "–")
}

/// Exa's own endpoint, used when a key is set: the MCP one shares a free tier
/// that rate-limits almost immediately.
async fn exa_search(http: &reqwest::Client, query: &str, args: &Value) -> Result<String, String> {
    let payload = json!({
        "jsonrpc": "2.0",
        "id": 1,
        "method": "tools/call",
        "params": {
            "name": "web_search_exa",
            "arguments": {
                "query": query,
                "type": args.get("type").and_then(Value::as_str).unwrap_or("auto"),
                "numResults": args.get("numResults").and_then(Value::as_u64).unwrap_or(8),
                "livecrawl": args
                    .get("livecrawl")
                    .and_then(Value::as_str)
                    .unwrap_or("fallback"),
                "contextMaxCharacters": args
                    .get("contextMaxCharacters")
                    .and_then(Value::as_u64)
                    .unwrap_or(10_000),
            }
        }
    });

    let request = http
        .post(exa_endpoint())
        .header("Accept", "application/json, text/event-stream")
        .json(&payload);

    let response = match tokio::time::timeout(SEARCH_TIMEOUT, request.send()).await {
        Err(_) => return Err(format!("exa: timed out after {}s", SEARCH_TIMEOUT.as_secs())),
        Ok(Err(e)) => return Err(format!("exa: request failed: {e}")),
        Ok(Ok(response)) => response,
    };

    let status = response.status();
    let body = response.text().await.unwrap_or_default();
    if status.as_u16() == 429 {
        return Err("exa: the shared free tier rate limit was reached".to_string());
    }
    if !status.is_success() {
        return Err(format!(
            "exa: HTTP {status}: {}",
            body.chars().take(200).collect::<String>()
        ));
    }

    match extract_search_text(&body) {
        Some(Ok(text)) => Ok(text),
        Some(Err(message)) => Err(format!("exa: {message}")),
        // Nothing parseable came back; hand the raw body over rather than swallow it.
        None => Ok(body.chars().take(MAX_OUTPUT).collect()),
    }
}

