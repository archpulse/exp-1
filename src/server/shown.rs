//! how a tool call and its output are shown to the user.



pub(crate) fn command_preview(name: &str, arguments: &str) -> String {
    let value: serde_json::Value = match serde_json::from_str(arguments) {
        Ok(value) => value,
        Err(_) => return name.to_string(),
    };
    let path = |key: &str| {
        value
            .get(key)
            .and_then(|v| v.as_str())
            .map(str::to_string)
    };
    match name {
        "read" => path("path").map(|p| format!("read {p}")).unwrap_or_default(),
        "write" => path("path").map(|p| format!("write {p}")).unwrap_or_default(),
        "edit" => path("path").map(|p| format!("edit {p}")).unwrap_or_default(),
        "delete" => path("path")
            .map(|p| format!("delete {p}"))
            .unwrap_or_default(),
        "ast" => "ast".to_string(),
        "count" => path("path")
            .map(|p| format!("count {p}"))
            .unwrap_or_default(),
        "grep" => path("pattern")
            .map(|p| format!("grep {p}"))
            .unwrap_or_default(),
        "fetch" => path("url").map(|u| format!("fetch {u}")).unwrap_or_default(),
        "websearch" => path("query")
            .map(|q| format!("search: {q}"))
            .unwrap_or_else(|| "websearch".to_string()),
        "ask_question" => path("question")
            .map(|q| format!("ask: {q}"))
            .unwrap_or_else(|| "ask".to_string()),
        // A subagent call is named by its description, not by the whole task
        // text it carries.
        "task" => path("description")
            .map(|d| format!("task: {d}"))
            .unwrap_or_else(|| "task".to_string()),
        _ => {
            let argv = value
                .get("command")
                .and_then(|c| c.as_array())
                .map(|items| {
                    items
                        .iter()
                        .filter_map(|i| i.as_str())
                        .collect::<Vec<_>>()
                        .join(" ")
                })
                .or_else(|| path("cmd"))
                .unwrap_or_else(|| arguments.to_string());
            format!("$ {argv}")
        }
    }
}

pub(crate) fn preview(text: &str) -> String {
    let collapsed: Vec<&str> = text.lines().take(12).collect();
    let mut out = collapsed.join("\n");
    if text.lines().count() > 12 || out.chars().count() > 400 {
        // Truncate at the 400th character, not the 400th byte: slicing a
        // multi-byte character in half panics, and Cyrillic output is routine.
        let end = out
            .char_indices()
            .nth(400)
            .map(|(at, _)| at)
            .unwrap_or(out.len());
        out.truncate(end);
        out.push_str("\n…");
    }
    out
}
/// The media type of a picture, by its extension.
pub(crate) fn media_type(path: &std::path::Path) -> &'static str {
    match path
        .extension()
        .and_then(|ext| ext.to_str())
        .map(str::to_ascii_lowercase)
        .as_deref()
    {
        Some("jpg") | Some("jpeg") => "image/jpeg",
        Some("gif") => "image/gif",
        Some("webp") => "image/webp",
        _ => "image/png",
    }
}

pub(crate) fn is_image(path: &std::path::Path) -> bool {
    matches!(
        path.extension()
            .and_then(|ext| ext.to_str())
            .map(str::to_ascii_lowercase)
            .as_deref(),
        Some("png") | Some("jpg") | Some("jpeg") | Some("gif") | Some("webp")
    )
}
