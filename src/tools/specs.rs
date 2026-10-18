//! the JSON schemas handed to the model.

//! Tool implementations: read, write, edit, shell, ast.
//!
//! All paths are resolved against the session working directory. Nothing is
//! sandboxed: commands run as the user.

use serde_json::{Value, json};

/// A foreground command still running after this long is promoted to a
/// background task, so the agent is never blocked on something slow. A
/// background task has no clock on it: builds and test suites run for minutes,
/// and the old 120s axe was cutting them off mid-flight.

pub fn specs() -> Vec<Value> {
    vec![
        json!({
            "type": "function",
            "name": "read",
            "description": "Read a text file. Returns at most 400 lines by default; pass \
                            limit for a different number of lines and pos for a 1-based \
                            first line. A cut read says how to continue.",
            "parameters": {
                "type": "object",
                "properties": {
                    "path": { "type": "string", "description": "File to read, relative to the working directory." },
                    "limit": { "type": "integer", "description": "Maximum number of lines to return." },
                    "pos": { "type": "integer", "description": "First line to return, 1-based." }
                },
                "required": ["path"],
                "additionalProperties": false
            }
        }),
        json!({
            "type": "function",
            "name": "write",
            "description": "Create a file or overwrite it entirely.",
            "parameters": {
                "type": "object",
                "properties": {
                    "path": { "type": "string" },
                    "content": { "type": "string" }
                },
                "required": ["path", "content"],
                "additionalProperties": false
            }
        }),
        json!({
            "type": "function",
            "name": "delete",
            "description": "Delete a file. Directories are refused; use shell for those. \
                            There is no undo, so delete only what you are sure about.",
            "parameters": {
                "type": "object",
                "properties": {
                    "path": { "type": "string", "description": "File to delete, relative to the working directory." }
                },
                "required": ["path"],
                "additionalProperties": false
            }
        }),
        json!({
            "type": "function",
            "name": "edit",
            "description": "Replace an exact snippet in a file. Fails when 'old' is absent or \
                            matches more than once, and the failure names the closest lines or \
                            the positions of the matches. Pass several 'edits' to change one \
                            file in one call: they apply in order, all or nothing. The result \
                            shows what changed, so there is no need to read the file back.",
            "parameters": {
                "type": "object",
                "properties": {
                    "path": { "type": "string" },
                    "old": { "type": "string", "description": "Exact text to replace." },
                    "new": { "type": "string", "description": "Replacement text." },
                    "edits": {
                        "type": "array",
                        "description": "Several replacements in one call, applied in order, all or nothing.",
                        "items": {
                            "type": "object",
                            "properties": {
                                "old": { "type": "string" },
                                "new": { "type": "string" }
                            },
                            "required": ["old", "new"],
                            "additionalProperties": false
                        }
                    }
                },
                "required": ["path"],
                "additionalProperties": false
            }
        }),
        json!({
            "type": "function",
            "name": "shell",
            "description": "Run a command. Foreground by default: you wait until it finishes, \
                            and one still running after 60s is promoted to the background \
                            where you are told when it finishes. Pass background true to \
                            keep working. A background command is never killed on a timer. \
                            Output is capped: a very long one keeps the beginning and the \
                            end, with the gap marked; re-read a previous run with output_id \
                            and a new limit.",
            "parameters": {
                "type": "object",
                "properties": {
                    "command": { "type": "array", "items": { "type": "string" } },
                    "background": { "type": "boolean", "description": "Run without waiting." },
                    "limit": { "type": "integer", "description": "Maximum number of output lines." },
                    "output_id": { "type": "string", "description": "Re-read previous output, or poll a background task." }
                },
                "required": [],
                "additionalProperties": false
            }
        }),
        json!({
            "type": "function",
            "name": "task",
            "description": "Spawn a subagent to do one focused piece of work in its own context \
                            and report back. Use it for work that would otherwise flood your \
                            context — a broad search across many files, a long investigation — \
                            or for something you can run in parallel with your own work.",
            "parameters": {
                "type": "object",
                "properties": {
                    "description": { "type": "string", "description": "Three or four words naming the task." },
                    "prompt": { "type": "string", "description": "The full task for the subagent, with everything it needs." }
                },
                "required": ["prompt"],
                "additionalProperties": false
            }
        }),
        json!({
            "type": "function",
            "name": "fetch",
            "description": "Read one web page as plain text. Use it when you have a URL — \
                            documentation, a changelog, an issue — instead of searching for it. \
                            The page is paged like a file: `limit` caps the lines returned \
                            (default 400) and `pos` is the 1-based line to start at.",
            "parameters": {
                "type": "object",
                "properties": {
                    "url": { "type": "string", "description": "Page to read, http or https." },
                    "limit": { "type": "integer", "description": "Maximum lines to return (default 400)." },
                    "pos": { "type": "integer", "description": "First line to return, 1-based." }
                },
                "required": ["url"],
                "additionalProperties": false
            }
        }),
        json!({
            "type": "function",
            "name": "count",
            "description": "Count the lines of a file, or of every file under a directory, \
                            without reading the contents into the conversation. Answers are \
                            cached in .cache/count and recounted when the file changes; pass \
                            \"refresh\": true to ignore the cache.",
            "parameters": {
                "type": "object",
                "properties": {
                    "path": { "type": "string", "description": "File or directory, relative to the working directory." },
                    "refresh": { "type": "boolean", "description": "Recount instead of using .cache/count." }
                },
                "required": ["path"],
                "additionalProperties": false
            }
        }),
        json!({
            "type": "function",
            "name": "grep",
            "description": "Find a pattern in the files under a directory. Use it to locate \
                            code instead of reading files to look for something. `pattern` \
                            is a regular expression. Skips .git, .cache and build output \
                            (target, node_modules, dist, build, out, virtualenvs and other \
                            caches), follows no symlinks, and ignores files that are \
                            not text. The answer is capped, and a broad pattern says how to \
                            narrow it.",
            "parameters": {
                "type": "object",
                "properties": {
                    "pattern": { "type": "string", "description": "Regular expression to find." },
                    "path": { "type": "string", "description": "File or directory, relative to the working directory (default: the working directory)." },
                    "include": { "type": "string", "description": "Only search files whose name matches this pattern, e.g. *.rs." },
                    "max": { "type": "integer", "description": "Matches to return (default 100)." }
                },
                "required": ["pattern"],
                "additionalProperties": false
            }
        }),
        json!({
            "type": "function",
            "name": "websearch",
            "description": "Search the web and get page contents back. Use it for anything \
                            outside the machine: current versions, documentation, error \
                            messages, or facts you cannot read from the filesystem.",
            "parameters": {
                "type": "object",
                "properties": {
                    "query": { "type": "string", "description": "What to search for." },
                    "numResults": { "type": "integer", "description": "How many results (default 8)." },
                    "type": { "type": "string", "enum": ["auto", "fast", "deep"], "description": "auto is balanced, fast is quick, deep is thorough." },
                    "livecrawl": { "type": "string", "enum": ["fallback", "preferred"], "description": "When to crawl live instead of cached pages." },
                    "contextMaxCharacters": { "type": "integer", "description": "Cap on returned context (default 10000)." }
                },
                "required": ["query"],
                "additionalProperties": false
            }
        }),
        json!({
            "type": "function",
            "name": "ask_question",
            "description": "Ask the user a question and wait for the answer. Use it when a \
                            decision is the user's to make and guessing would waste work — a \
                            choice of approach, a missing credential, an ambiguous requirement. \
                            Give options when there are sensible choices; whatever the user types \
                            is taken as their own answer.",
            "parameters": {
                "type": "object",
                "properties": {
                    "question": { "type": "string", "description": "What to ask." },
                    "options": { "type": "array", "items": { "type": "string" }, "description": "Suggested answers the user can pick instead of typing." },
                    "label": { "type": "string", "description": "Short name for the question." },
                    "submit": { "type": "string", "description": "Label for the submit tab." }
                },
                "required": ["question"],
                "additionalProperties": false
            }
        }),
        json!({
            "type": "function",
            "name": "schedule",
            "description": "Schedule a request to run later without the user: one job is one \
                            request plus a time. Use it when the user asks for something to \
                            happen on a schedule — a `/schedule …` message, \"every day\", \
                            \"every Monday at nine\", \"in 5 minutes\", \"remind me tomorrow\". \
                            Compute `at` from the current local time in your instructions and \
                            pass a local timestamp like 2026-10-18T09:00; `every` repeats the \
                            job. A run happens in its own session, unattended, so write \
                            `request` as a complete instruction to yourself.",
            "parameters": {
                "type": "object",
                "properties": {
                    "action": { "type": "string", "enum": ["create", "list", "cancel", "pause", "resume"], "description": "What to do; create is the default." },
                    "request": { "type": "string", "description": "What to do when it fires, as a complete instruction to yourself (create)." },
                    "at": { "type": "string", "description": "The first or only run: a local timestamp like 2026-10-18T09:00, or a wait like `in 5m` (create)." },
                    "every": { "type": "string", "description": "Repeat interval: 30m, 2h, 1d, 1w. Omit for a one-shot." },
                    "name": { "type": "string", "description": "Short name for the list and for notifications." },
                    "id": { "type": "string", "description": "Which job: its id or its exact name (cancel, pause, resume, run)." },
                    "directory": { "type": "string", "description": "Where the run works. Defaults to this session's directory." }
                },
                "required": ["action"],
                "additionalProperties": false
            }
        }),
        json!({
            "type": "function",
            "name": "ast",
            "description": "Show the directory tree of the project. Cached in .cache/ast; pass \
                            refresh true to rebuild it.",
            "parameters": {
                "type": "object",
                "properties": {
                    "path": { "type": "string", "description": "Directory to list. Defaults to the working directory." },
                    "refresh": { "type": "boolean", "description": "Rebuild instead of using the cache." }
                },
                "required": [],
                "additionalProperties": false
            }
        }),
    ]
}
