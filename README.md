# exp-1 agent

A minimal coding agent in Rust with a terminal UI. It authenticates **exactly the way the
Codex CLI does** — ChatGPT OAuth against `auth.openai.com`, with model requests going to
`https://chatgpt.com/backend-api/codex/responses`. That is the only auth provider it supports.

## What it does

- Streams responses from the Codex Responses endpoint (SSE, parsed by hand).
- Runs the agent loop — request → stream → run tool → feed output back → repeat — until the
  model stops asking for tools.
- Keeps two threads: the main conversation, and a side thread for questions that must not
  interrupt it.
- Shows reasoning, streamed assistant text, tool calls and their output in one transcript, with
  per-turn duration and clock time.

## Tools

| Tool | Arguments | Notes |
| --- | --- | --- |
| `read` | `path`, `limit?`, `pos?` | Whole file by default; `limit` caps lines, `pos` is the first line, 1-based. Output is numbered. |
| `write` | `path`, `content` | Creates parent directories; overwrites an existing file. |
| `edit` | `path`, `old`, `new` | Exact replacement. Fails when `old` is absent or matches more than once. |
| `fetch` | `url`, `limit?`, `pos?` | One page as plain text, paged like a file: `limit` lines (default 400) from `pos` (1-based). Scripts and styles are dropped, entities decoded. |
| `count` | `path`, `refresh?` | Lines in a file, or across every file under a directory, without reading the contents. Cached per file in `.cache/count`, keyed by size and mtime. |
| `delete` | `path` | Removes a file. Directories are refused — emptying a tree belongs in `shell`, where the whole command is visible. |
| `shell` | `command`, `background?`, `limit?`, `output_id?` | Foreground blocks the agent until it finishes. `background: true` returns an id and lets the agent keep working; poll with `output_id`. Output is line-limited, and `output_id` re-reads any earlier run with a different `limit`. |
| `ast` | `path?`, `refresh?` | Project tree in `tree` format. Cached in `.cache/ast`; `refresh: true` rebuilds. `.git` is skipped. |
| `websearch` | `query`, `numResults?`, `type?`, `livecrawl?`, `contextMaxCharacters?` | Web search with page contents. Uses Exa's hosted MCP at `mcp.exa.ai`, which needs **no API key**; set `EXA_API_KEY` to lift rate limits. `type` is `auto`/`fast`/`deep`. |
| `ask_question` | `question`, `options?`, `label?`, `submit?` | Blocks the turn and shows a question panel. `↑↓` pick an option, `Tab` moves between questions, `Enter` confirms, `Esc` dismisses. Anything typed is taken as a free-form answer and wins over the highlighted option. |

Shell commands run unsandboxed as the user, with a 120 s foreground timeout and output capped at
30 000 bytes.

The `web` tool searches the web: Exa's hosted MCP first, because it returns page content,
and DuckDuckGo's HTML page when Exa's shared free tier rate-limits. `EXA_API_KEY` switches Exa to
your own plan and is the only setting either path needs.


## Platforms

The agent is one codebase for Linux, macOS and Windows: the TUI, the agent loop, providers, MCP,
skills, sessions and the bundled titler are plain Rust and run everywhere. The parts that touch the
operating system have a branch per platform:

| | Linux | macOS | Windows |
| --- | --- | --- | --- |
| TUI, agent, providers, MCP, skills, sessions | yes | yes | yes |
| Sign-in page opener | `xdg-open` | `open` | `cmd /C start` |
| Daemon liveness and stop | `kill` 0 / SIGTERM→SIGKILL | same | `tasklist` / `taskkill /T` then `/F` |
| Server detached from its console | `process_group(0)` | same | `DETACHED_PROCESS \| CREATE_NEW_PROCESS_GROUP` |
| Config and state directories | `$XDG_*`, `~/.config` | `$XDG_*`, `~/.config` | `%APPDATA%`, `%LOCALAPPDATA%` |
| Clipboard image (`ctrl + v`) | `wl-paste`, `xclip` | `pngpaste` | PowerShell + `System.Windows.Forms` |
| Turn-finished notification | `notify-send` | `osascript` | PowerShell balloon tip |
| Archive installs (`exp-1 install *.zip`) | `unzip` / `tar` | same | `tar` (bsdtar reads zip) |
| Local MCP servers named `npx` | as written | as written | retried as `npx.cmd` |

The Unix and Windows branches of the daemon and the clipboard were compile-checked by temporarily
activating them on Linux; `creation_flags` is the one Windows-only call that cannot be checked
without a Windows target, and nothing here has been run on a real Windows or macOS machine.

### Cross-building from Linux

A Windows binary builds on Arch with the mingw toolchain (this needs root once):

```sh
sudo pacman -S --needed mingw-w64-binutils mingw-w64-gcc
rustup target add x86_64-pc-windows-gnu
cargo build --release --target x86_64-pc-windows-gnu
```

`dlltool` from `mingw-w64-binutils` is what the import libraries need; without it the build stops at
`windows-sys`.

A macOS binary needs no root either, but it cannot be finished on Linux as things stand: `reqwest`
0.13's `rustls` feature pulls `rustls-platform-verifier`, which needs `Security.framework`,
`SystemConfiguration` and `CoreFoundation` at link time, and those live only in the Apple SDK. Zig
links Mach-O and libSystem happily; it does not ship the frameworks. Build the macOS binaries on a
Mac or in CI (`.github/workflows/release.yml` does all four targets). The recipe below is the one
that gets as far as the link:

```sh
mkdir -p ~/.local/zig && curl -L https://ziglang.org/download/0.14.1/zig-x86_64-linux-0.14.1.tar.xz \
  | tar -xJ -C ~/.local/zig --strip-components=1
cargo install cargo-zigbuild
rustup target add aarch64-apple-darwin
PATH="$HOME/.local/zig:$PATH" cargo zigbuild --release --target aarch64-apple-darwin
```

The result is unsigned. On Apple Silicon an unsigned binary is killed on launch, so it has to be
signed once on the Mac itself:

```sh
codesign -s - --force ./exp1-agent        # ad-hoc signature
xattr -cr ./exp1-agent                    # if it arrived by download
```

## Requirements

- Rust 1.85+ (edition 2024)
- Either an existing Codex CLI login (its `auth.json` is reused) or `/login`, which runs the same
  device flow itself.
- The sibling `titler` crate at `../mygame/titler`. It pulls in Candle, which is why the first
  build takes a while and the binary is ~16 MB rather than ~7 MB. Once the weights are in the
  Hugging Face cache, titling is offline.

## Usage

```sh
cargo run --release                      # uses the current directory
exp1 --model gpt-6-luna --effort high    # pick a model and reasoning effort
exp1 --cwd ~/projects/my-app             # run the shell tool somewhere else
exp1 --help
```

| Key | Action |
| --- | --- |
| `Enter` | Send the prompt, or run the command when it is already complete |
| `↑` `↓` | Move the completion selection, or recall previous prompts |
| `Tab` | Accept the highlighted completion |
| `Ctrl + /` | Switch between the main thread and the side thread |
| `Esc` | Nothing on its own |
| `Esc` `Esc` | Abort the running turn, or dismiss the completion list or picker |
| `Ctrl-C` | Quit |
| `Ctrl-U` | Clear the input line |
| `Ctrl-Backspace` | Delete the previous word |
| `Ctrl-Delete` | Delete the next word |
| `Ctrl-W`, `Alt-Backspace` | Same as `Ctrl-Backspace` |
| Mouse wheel | Scroll the transcript |
| `PgUp` / `PgDn` | Scroll the transcript |
| `←` `→` `Home` `End` | Move the caret |
| `Backspace` / `Delete` | Edit |

The arrow keys walk the prompt history: after sending `hi`, `↑` brings it back into the field,
further presses walk further back, and `↓` walks forward to the empty field. Scrolling is on the
mouse wheel, so the arrows stay free for history.

Typing `/` opens a completion list filtered by prefix. After a space, it completes the command's
arguments too — `/themes <tab>` lists the schemes, `/model <tab>` the models, and
`/model gpt-6-luna <tab>` the reasoning efforts. `Enter` on a bare `/model` or `/themes` opens a
numbered selection list instead; `/model` is two steps, model first and then effort. With an empty
field the caret sits at the very start, before the placeholder.

The hint line changes contextually. Input is locked while a turn is running.

`Esc` is deliberately inert; the double press is what interrupts. Interrupting aborts the
in-flight request and drops any partial assistant text, leaving the conversation history intact.

## Slash commands

Handled locally; they never reach the model.

| Command | Effect |
| --- | --- |
| `/login` | Pick a provider: a signed-in one just switches; picking the one in use signs in again. Device-code sign-ins open the vendor's page in the browser; the rest take an API key |
| `/scoped-models` | Tick which models `/model` is allowed to offer; empty means all |
| `/sessions` | Pick a saved conversation to reopen |
| `/init` | Write an `AGENTS.md` scaffold in the working directory |
| `/model` | Open the two-step picker (model, then effort); `/model <name> [low\|medium\|high]` sets it directly |
| `/themes` | Open the theme picker; `/themes <name>` sets it directly |
| `/status` | Show model, effort, working directory, tools, theme, plan, auth provider |
| `/btw` | Answer a side question in the side thread while the main turn keeps running |
| `/clear` | Clear the transcript and the conversation history |
| `/help` | Show keyboard shortcuts |
| `/compact` | Summarise the conversation now instead of waiting for the window to fill |
| `/undo` | Put back the files the last turn changed (in memory; a restart forgets them) |
| `/diff` | Unified diff of what the last turn changed |
| `/fork` | Continue this conversation as a new session, leaving the original alone |
| `/sessions <text>` | Search the saved conversations instead of just listing them |
| `/retitle` | Name the session again from the original request and the recent conversation |
| `/effort` | Pick the reasoning effort for the current model, without changing the model |
| `/mcp` | List the MCP servers and the tools they offer |

Available themes: `default` (blue), `amber`, `jade`, `mono`. Colours are truecolor, so a
terminal theme cannot remap them.

## Interface

The layout follows the OpenAI Codex TUI: no bordered panels, a `>_` masthead with the working
directory beneath it, a completion popup or a numbered selection picker above the input, a padded
input band with a `›` prompt and a blinking `│` caret, then a model line and a hint line.

- A submitted prompt is drawn back in the transcript as a full-width band, the way Codex does.
- The transcript sticks to the newest message; the mouse wheel scrolls upwards through history.
- Dragging with the mouse selects text anywhere on the screen — masthead included — and releasing
  puts it on the clipboard: OSC 52 first, so it also works over ssh, then `wl-copy`, `xclip` or
  `xsel` when they exist. A `copied N chars` note confirms it in the hint line. The selection is a
  quiet tint behind the text and stops at the last character of each row, the way a terminal's own
  selection behaves. The terminal never sees the drag, because the app holds the mouse for
  scrolling, so this is the only way to copy.
- An attached picture shows as a filled `Image 1` chip above the prompt, with `ctrl-v replaces it`
  beside it, so what is about to be sent is visible in the input band itself.
- While a turn runs, an activity line sits **directly under the prompt it belongs to** —
  `⠋ thinking 4s`, then `⠋ running 4s` while a tool executes. The bottom bar stays a plain
  `GPT-6-Luna high · /path · <thread> · Main [default]` label; `esc esc interrupts` lives in the
  hint line.
- After a turn finishes, `Worked for 5s · 12:08 PM` is appended to the transcript.
- The model line reads `GPT-6-Luna high · /path · <thread name> · Main [default]` or `… · Agent`
  in the side thread. The thread name is taken from the first prompt.
- The hint line carries the thread switch on the right, in red while the side thread is active.
- An empty screen stays quiet: the full command list appears only on a machine that has never
  saved a conversation, and afterwards it is one line pointing at `/` and `/help`.
- Typing stays live during a turn, so a `/btw` question can be asked without waiting.
- `/btw`, or `ctrl + /`, answers in a side chat that carries the whole main conversation as
  context without adding anything to it.
- `shift + tab` switches between build and plan mode; the footer shows `plan mode` while it is on.
- A turn that runs longer than fifteen seconds rings the terminal bell (and `notify-send`, where the
  desktop has it) when it finishes, so a long job does not have to be watched.
- `shift + enter` writes a line break instead of sending, so a prompt can be several lines;
  the input band grows to hold them.
- `ctrl + v` takes an image off the clipboard (Wayland `wl-paste`, X11 `xclip`) and attaches it
  to the next prompt, showing `▣ image attached` in the hint line until it is sent. The picture
  travels as a data URL, so the Codex, OpenAI, Anthropic and Gemini backends all pass it on.
- `ask_question` raises a panel above the input, mirroring the Question tool: an accent bar down
  the left edge, the `Questions` title, tabs when several questions are asked together, numbered
  options with the highlighted one picked out, a `Type your own answer` slot, and
  `⇥tab ↑↓ select enter confirm esc dismiss` at the bottom.

Colours are truecolor, so a terminal theme cannot remap them. Model, effort and theme switch at
runtime via `/model` and `/themes`.

Differences from Codex that are deliberate: Codex has `/permissions` and `/review`, which need
sandbox policy and a diff reviewer; this build has `/clear` and `/help` instead. Codex's footer
advertises agent and shortcut overlays, which this MVP does not have.

## Authentication

On first run the agent copies `$CODEX_HOME/auth.json` (default `~/.codex/auth.json`) into
`~/.exp1-agent/auth.json` and works from that copy. **Your real Codex credentials are never
rewritten.**

When the access token is within 5 minutes of expiry the agent refreshes it:

```
POST https://auth.openai.com/oauth/token
grant_type=refresh_token
client_id=app_EMoamEEZ73f0CkXaXp7hrann
```

The response rotates the refresh token, so the new one is written back to the agent's own copy.
The stored file is written atomically with mode `0600`.

## Models and efforts

Neither the model list nor the reasoning efforts are hardcoded. Both are read from the Codex
CLI's own catalogue at `$CODEX_HOME/models_cache.json` (default `~/.codex/models_cache.json`),
using each entry's `slug`, `display_name`, `description`, `visibility`,
`default_reasoning_level` and `supported_reasoning_levels`. The startup default is the first
listed model in that catalogue.

Efforts genuinely differ per model, which is why guessing them does not work:

| Model | Efforts |
| --- | --- |
| `gpt-5.6-terra` | low, medium, high, xhigh, max, **ultra** |
| `gpt-6-luna` | low, medium, high, xhigh, max |
| `gpt-5.5` | low, medium, high, xhigh |

The effort picker, the `/model <name> <tab>` completions and the validation all follow whichever
model is active. Switching models carries the effort over when the new model accepts it and
otherwise falls back to that model's `default_reasoning_level`, so you can never end up asking for
an effort the model does not have:

```
/model gpt-5.6-terra ultra   →  GPT-5.6-Terra ultra
/model gpt-5.5               →  GPT-5.5 medium          (ultra is not a gpt-5.5 level)
/model gpt-5.5 max           →  'max' is not a GPT-5.5 level;
                                this model takes low, medium, high, xhigh
```

Only models with `visibility: "list"` appear in the picker; hidden ones remain reachable by name.
If the catalogue file is missing the app falls back to a conservative built-in list so it stays
usable, and says so in the picker.

Only models the signed-in ChatGPT account is entitled to will actually answer — the backend
rejects the rest with an explanatory error.

## Request shape

```jsonc
POST https://chatgpt.com/backend-api/codex/responses
Authorization: Bearer <access_token>
chatgpt-account-id: <tokens.account_id>
originator: codex_cli_rs
User-Agent: codex_cli_rs/<version> (<os>; <arch>) terminal
Accept: text/event-stream
Content-Type: application/json
session-id: <uuid per request>

{
  "model": "gpt-5.6-terra",
  "stream": true,
  "instructions": "…",
  "input": [
    { "type": "message", "role": "user",
      "content": [{ "type": "input_text", "text": "…" }] },
    { "type": "function_call", "call_id": "…", "name": "shell", "arguments": "…" },
    { "type": "function_call_output", "call_id": "…", "output": "exit_code: 0\n…" }
  ],
  "tools": [{ "type": "function", "name": "read", "parameters": { … } }, …],
  "tool_choice": "auto",
  "parallel_tool_calls": false,
  "reasoning": { "effort": "medium", "summary": "auto" },
  "store": false,
  "include": []
}
```

Events consumed: `response.output_text.delta`, `response.reasoning_summary_text.delta`,
`response.output_item.done`, `response.completed`, `response.failed`, `response.incomplete`,
`error`. Anything else is ignored.

## Rendering

Assistant answers are rendered as rich text in-process. There is no external renderer, no
`pandoc`, no TeX binary, no image rasterisation — everything happens inside the TUI, so it costs
almost nothing.

- **Markdown**: headings, bold, italic, inline code, fenced code blocks with a gutter and a
  language tag, bullet and ordered lists, blockquotes, horizontal rules, links, GitHub-style pipe
  tables laid out with box-drawing borders and per-column alignment, and inline `$…$` math.
- **LaTeX math**: symbols and relations, Greek, `\frac`, `\sqrt`, sub/superscripts, `\mathbb`
  and `\mathcal`, combining accents (`\bar`, `\hat`, `\vec`, …), upright operator names
  (`\log`, `\sin`, `\lim`, …), and display `$$…$$` blocks.

Anything the converter does not recognise is emitted verbatim, braces included, so unfamiliar
input stays readable instead of turning to mush. It is a converter, not a TeX engine: macros,
environments such as `tikz`, and multi-line layouts are not interpreted.

Markdown is parsed **once**, when a message arrives, into theme-independent runs; drawing only
maps those runs to colours. Switching themes therefore reparses nothing.

Measured on the built binary: **~1 % of one core while idle** with the caret blinking, ~3 % during
rapid wheel scrolling.

## Subagents and long commands

**Subagents.** The `task` tool runs a second agent with its own context and the same tools, then
returns only its report — useful for work that would otherwise flood the caller's context. Nested
subagents are offered while depth remains, capped at two levels so recursion cannot run away. The
transcript shows each step the subagent takes.

**Slow commands are promoted, not killed.** A foreground command still running after **60 seconds**
is moved to the background and the agent is told so, rather than blocking the turn:

```
'sleep 75' is still running after 60s, so it was moved to the background as 'bg-1'.
Carry on; you will be told when it finishes, or poll it with {"output_id": "bg-1"}.
```

**Finishing interrupts.** When a background command ends, the running turn is aborted and the
agent is given the output, so a result is never silently lost behind long work:

```
• interrupted: a background command finished
```

## Providers

`/login` opens a picker of providers; `/scoped-models` decides which of their models `/model`
offers (empty means all of them, and the scope is stored in `settings.json`). The picker is a plain
alphabetical list, with the cursor opening on the provider in use, so nothing has to be found by
position. Picking a provider
that is already signed in switches to it and says so; picking the one already in use — or running
`/login <id>` — signs in again on purpose. A device-code sign-in opens the vendor's page in the
browser and prints the URL and code as the fallback.

Four wire protocols cover everything implemented here: the **OpenAI chat-completions** API, shared
by a dozen vendors, **OpenAI Responses**, **Anthropic messages**, and Vertex AI's native
**Gemini `generateContent`**. All stream SSE, and all are decoded into the same event stream the
Codex client produces, so the agent loop does not know which one it is talking to.

Keys are stored per provider in `~/.config/exp-1/sessions/credentials.json`, mode `0600`.
`openai (auth)` keeps a Codex-style `auth.json` instead, `google vertex` uses whatever
`gcloud auth application-default login` already signed in (with `GOOGLE_CLOUD_PROJECT` and
`GOOGLE_CLOUD_LOCATION` naming the project and region), and the device-code providers put the
token they receive into the same credentials file.

| Provider | Base URL | Protocol |
| --- | --- | --- |
| openai (auth) | `chatgpt.com/backend-api/codex` | Codex |
| openai (api) | `api.openai.com/v1` | Responses + OpenAI |
| anthropic (api) | `api.anthropic.com/v1` | Anthropic |
| google gemini | `generativelanguage.googleapis.com/v1beta/openai` | OpenAI |
| deepseek | `api.deepseek.com/v1` | OpenAI |
| kimi | `api.moonshot.ai/v1` | OpenAI |
| qwen | `dashscope.aliyuncs.com/compatible-mode/v1` | OpenAI |
| minimax | `api.minimax.io/anthropic/v1` | Anthropic |
| stepfun | `api.stepfun.com/v1` | OpenAI |
| fireworks-ai | `api.fireworks.ai/inference/v1` | OpenAI |
| openrouter | `openrouter.ai/api/v1` | OpenAI |
| groq | `api.groq.com/openai/v1` | OpenAI |
| grok | `api.x.ai/v1` | OpenAI |
| github copilot | `api.individual.githubcopilot.com` | OpenAI |
| meta (auth) | `api.meta.ai/v1` | OpenAI |
| glm (z.ai) | `api.z.ai/api/paas/v4` | OpenAI |
| glm coding plan (z.ai) | `api.z.ai/api/coding/paas/v4` | OpenAI |
| glm (zhipu, cn) | `open.bigmodel.cn/api/paas/v4` | OpenAI |
| glm coding plan (zhipu, cn) | `open.bigmodel.cn/api/coding/paas/v4` | OpenAI |
| muse spark | `api.meta.ai/v1` | OpenAI |
| clinepass | `api.cline.bot/api/v1` | OpenAI |
| agentrouter | `agentrouter.org/v1` | OpenAI |
| opencode-zen | `opencode.ai/zen/v1` | OpenAI |
| opencode-go | `opencode.ai/zen/go/v1` | Responses + Anthropic + OpenAI |
| anthropic (auth) | `api.anthropic.com/v1` | Anthropic |
| google vertex | `aiplatform.googleapis.com/v1` | Gemini |
| gitlab-duo | `cloud.gitlab.com/ai/v1/proxy/anthropic/v1` | Anthropic |

Every entry is implemented: each model names the protocol it answers on, so a gateway like
`opencode-go` can serve three at once. The list and base URLs come from [models.dev](https://models.dev), the public registry opencode itself reads, rather than from guesswork.

Every base URL above was probed with a deliberately invalid key: a rejection proves the URL and the
auth header are right. All twenty-seven answered with `400`, `401`, or `403` — except `fireworks-ai`,
whose `404` is its own `"Model not found"` carrying a `chatcmpl-` request id, so even that request
reached the inference route. `google vertex` answers `401` on its project-scoped path.

## Modes

`shift + tab` switches the session between two modes, and the footer says which one is active:

- **build** — everything as described here.
- **plan** — the tools that change files (`write`, `edit`, `delete`) are refused *and* left out of the
  tool list, so the model investigates and proposes a plan instead of editing. `shell` is expected to
  stay read-only. The wording lives in `prompts/plan.md`, editable like the rest.

## Permissions

`~/.config/exp-1/permissions.json` is a list of exceptions; a call that matches no rule runs. The
first match decides:

```json
{
  "rules": [
    { "action": "shell", "resource": "rm -rf *", "effect": "deny" },
    { "action": "write", "resource": "/etc/*", "effect": "ask" },
    { "action": "fetch", "resource": "*", "effect": "allow" }
  ]
}
```

`action` is a tool name or `*`, `resource` is what the call touches — a path for file tools, the
command line for `shell`, the URL or query for the net tools — and `*`/`?` work as wildcards.
`effect` is `allow`, `ask` or `deny`. An `ask` raises the question panel and holds the call until it
is answered (two minutes, then the call is refused). Rules apply to subagents too.

## Project instructions

`AGENTS.md` is read from the working directory and every parent above it, outermost first, and
appended to the system prompt — so a repository's own rules reach the model without being pasted
into the conversation. `/init` writes a starting file.

## Provider failures

A server-side or transport failure is retried once after two seconds; a rejected request (4xx) is
not. When the primary model keeps failing, `fallback_model` in `settings.json` names a model of the
same provider to try once before the turn is given up, and the switch is noted in the transcript.

## Skills

A skill is a directory with a `SKILL.md` in it, the shape Claude Code and opencode use. Installing
one makes it a command:

```sh
exp-1 install ~/skills/pdf-tools          # a directory with SKILL.md
exp-1 install notes.md                    # a single markdown file
exp-1 install tools.zip                   # an archive (unzip or tar)
exp-1 install https://example.com/skill.md
```

Files land in `~/.config/exp-1/skills/<name>/`, named by the `name:` field of the frontmatter when
it has one and by the file or directory name otherwise. Installed skills appear beside the built-in
commands in the completion popup and in the empty-state help, and `/<name>` runs one: the server
reads the skill's instructions, writes them into the conversation as the task, and lets the model
follow them. `description:` from the frontmatter is what the picker shows.

## MCP servers

Tools from [MCP](https://modelcontextprotocol.io) servers are offered to the model alongside the
built-in ones. The list lives in `~/.config/exp-1/mcp.json`, in exactly the shape opencode uses, so
an entry can be copied between the two:

```json
{
  "mcp": {
    "context7": { "type": "remote", "url": "https://mcp.context7.com/mcp" },
    "blender": {
      "type": "local",
      "command": ["blender-mcp"],
      "environment": { "BLENDER_HOST": "localhost" }
    }
  }
}
```

- `local` servers are spawned and spoken to over stdio; `remote` ones are reached over HTTP, with
  either a JSON or an SSE answer. `"disabled": true` skips a server without deleting it, and
  `"headers"` adds what a remote server needs.
- Tools reach the model as `<server>_<tool>`, both halves sanitised the way opencode sanitises them
  (`context7_resolve-library-id`), so two servers can never collide.
- The servers are connected **in the background** at startup: a slow or dead one never holds up the
  agent, and it is reported rather than dropped — `/mcp` lists every server with its tool count or
  the error it failed with.

## Background server

Modelled on opencode's daemon. **A normal `exp1` launch brings the server up and attaches to it**,
exactly like opencode's default command taking `daemon.transport()` and handing it to the UI.

The consequence is the point: closing the UI leaves the server running, so the next launch reuses
a warm process. A server is reused only when it is **both** the same version and the same build:
`server.json` records the executable's size and modification time, so a client rebuilt since the
server started replaces it instead of talking a protocol the server no longer speaks. Measured on
this machine:

| | cold launch | next launch |
| --- | --- | --- |
| UI spawn → first frame | 189 ms | **84 ms** |
| UI RSS | 14.4 MB | 16.0 MB |
| server RSS | 15.2 MB | 20.5 MB (after a turn) |

Measured again with five sessions open at once:

| | one server + 5 UIs | five standalone processes |
| --- | --- | --- |
| idle | 97 MB (15 + 5 × 16) | 183 MB (5 × 37) |
| with the bundled titler | 143 MB | 423 MB (5 × 85) |

The server is lazy about what it loads: the Codex client appears on the first turn (+5 MB) and the
bundled titler only if a title is actually generated locally (+48 MB, which is why `auto` prefers
the model). That laziness is most of the point — the 48 MB of weights are paid once for every
session on the machine instead of once per process.

The expensive, once-per-machine state — the tool registry and the ~49 MB titler model — lives in
the one server, and the Codex client is built the first time a Codex-protocol request needs it, so
a machine signed into any other provider starts the server without Codex credentials. Each UI is a
thin client, so N parallel sessions cost `1 x 63 MB + N x 15 MB` instead of `N x 61 MB`, and only
the first launch pays for a cold start.

```sh
exp1 serve                 # run the server in the foreground
exp1 service start         # spawn it detached (survives the launcher)
exp1 service status        # pid, url, version, health
exp1 service stop          # SIGTERM, then SIGKILL after a grace period
exp1 service restart
exp1 service password      # print, or set with an argument
```

- **Registration**: `~/.local/state/exp-1/server.json` holds `{id, version, url, pid}`. A client
  reads it, checks `/health` with a 2 s timeout, and reuses the server only when the version also
  matches — a stale or foreign server is stopped first.
- **Password**: `~/.local/state/exp-1/password`, 32 random bytes, mode `0600`, kept across restarts
  so a newly launched client reconnects without being told anything.
- **Detached spawn**: `exp1 serve --register` in its own process group with closed standard
  streams, so it outlives whichever client started it.

### HTTP API

All routes require `Authorization: Bearer <password>`.

| Method | Path | Purpose |
| --- | --- | --- |
| `GET` | `/health` | liveness and version |
| `GET` | `/sessions` | list live sessions |
| `POST` | `/sessions` | create one; `x-exp1-directory` sets its working directory |
| `GET` | `/sessions/{id}` | full snapshot |
| `POST` | `/sessions/{id}/prompt` | start a turn |
| `GET` | `/sessions/{id}/events` | SSE: snapshot, deltas, tools, usage, title, done, idle |
| `POST` | `/sessions/{id}/answer` | answer a pending `ask_question` |
| `POST` | `/sessions/{id}/interrupt` | abort the running turn |
| `GET` | `/models` | the catalogue, with each model's real efforts |

A session carries its own working directory, so one server serves several projects; the directory
arrives with `POST /sessions` rather than being fixed when the server starts.

## Auto-compaction

A long conversation is summarised before it can overflow the context window. After every turn the
server compares the request's `input_tokens` against the model's `context_window` from the Codex
catalogue and compacts once the conversation reaches **95 %** of it:

```
Context at 7378 of 258400 tokens (95% threshold) — compacting
Conversation compacted; continuing from the summary
```

Compaction sends the conversation to the model with a summarisation prompt and the structured
template opencode uses — Objective, Important Details, Work State (Completed / Active / Blocked),
Next Move, Relevant Files — with tool outputs clipped to 2000 characters so the summariser is not
drowned.

The rewritten conversation is **the summary followed by the last five user turns kept verbatim**,
each with the agent's tool calls and their results, so the thread in hand is not lost:

```
[user]      Summary of everything so far: ## Objective …
[user]      Turn two: run shell ["seq","1","900"] and reply OK.
[call]      shell
[result]    exit_code: 0 1 2 3 …
[assistant] OK
… four more turns …
```

A fact stated before the compaction is still known after it. If one of those turns is itself
enormous, the oldest tail entries are dropped until the result fits the window again, so a single
huge tool output cannot make the conversation re-compact forever.

The share is `EXP1_COMPACT_PERCENT` if you want a different one, which is also how the path can be
exercised without filling a 272 000-token window.

## Persistence

Configuration lives in `~/.config/exp-1/` (or `$XDG_CONFIG_HOME/exp-1`); the conversations
themselves live in `~/.cache/exp-1/sessions/` (or `$XDG_CACHE_HOME/exp-1/sessions`), because a
transcript is a cache: losing one costs context, not setup. The daemon's own files — `server.json`
and `password` — sit in `~/.local/state/exp-1/`.

| File | Holds |
| --- | --- |
| `config/settings.json` | model, reasoning effort, theme, titler mode, fallback model |
| `config/credentials.json` | one API key per provider, mode `0600` |
| `config/prompts/*.md` | the system prompts, editable: `system`, `subagent`, `compaction`, `title`, `plan` |
| `config/permissions.json` | the allow/ask/deny rules |
| `config/mcp.json` | the MCP servers |
| `config/skills/` | installed skills, one directory each |
| `cache/sessions/<id>.json` | one conversation: title, both transcripts, model-side history, prompt history |

A conversation keeps its newest **250 messages** on disk, and the cache as a whole is trimmed to
**250 MB** — oldest files first — when the server starts. An older layout, where everything sat in
`config/exp-1/sessions/`, is migrated on the first start after the change.

Settings are written when the app exits; a session is written after every completed turn, so a
crash costs at most the turn in flight.

The bundled titler is an optional feature: `cargo build --release --no-default-features` leaves it
out and takes the whole Candle dependency with it, and session titles then come from the model
alone. CI builds that way, because the titler is a sibling path dependency.

The prompts are files, not constants: `~/.config/exp-1/prompts/system.md` (the agent's own
instructions), `subagent.md`, `compaction.md` and `title.md` are written out on first launch and
read on every request, so an edit applies to the next turn with no rebuild. Resolution order for model and effort on startup is
**command line → saved settings → the catalogue's first listed model**.

**Every run starts on an empty session.** Nothing is resumed automatically — `/sessions` opens a
picker of saved conversations, newest first, each shown with its generated title, id and last-used
time. The list is scoped by directory: a session is visible where it was started, anywhere below
that, and everywhere if it was started in `$HOME` (the "global" ones). `$HOME` itself lists only
the global ones, so it does not become a catalogue of every project. Choosing one saves the conversation you are leaving and loads the chosen one. `/clear`
abandons the current conversation and starts a fresh one, keeping your settings.

### Session titles

Titles come from the bundled **titler** — a local T5 (7.6M parameters, 3 encoder and 3 decoder
layers) that runs through Candle in this process, pulled in as a path dependency from
`../mygame/titler`. The weights (`Cyronius/titler`) are read from the Hugging Face cache and
loaded at most once per process, so once they are on disk title generation is local, instant and
free.

The generation runs on a blocking thread the moment the first turn finishes, never on the UI
thread. Sessions with a parent — the side chat — are never named at all, since they belong to the
conversation they follow. The library's multilingual path is used (not the raw one), so the language is detected and
a non-English prompt is translated to English first. Sessions are named once, and the preview taken
from the prompt is what the picker shows until a title arrives.

**Who names a session is a setting**, `titler` in `settings.json`:

| Value | Behaviour |
| --- | --- |
| `auto` (default) | the Codex model first, the bundled titler as a fallback |
| `local` | the bundled titler only — no API spend, weaker titles |
| `remote` | the Codex model only |

Titles are asked of the cheap `luna` tier of the catalogue rather than the session's own model —
the same idea as opencode asking a provider for its small model — and the tokens a title costs are
added to the session's usage like any other call.

`/retitle` names the session again, this time from the original request plus the recent
conversation (bounded at 8,000 characters), which is how opencode regenerates a title when a
session is renamed to nothing. The prompt itself lives in `prompts/title.md` and is the full
opencode-style document: a task, the rules (same language as the user, ≤50 characters, keep exact
terms and numbers, never answer the question) and ten worked examples.

`auto` leads with the Codex model on purpose. The 7.6M titler is honest about its size: with the
same Russian prompts it produced `Understanding Two Plus Terms` for «сколько будет два плюс два»
and `Request for Sign` for a table request, because Google's free translator renders «табличку» as
"sign" and the small model then titles the mistranslation. The Codex model names the same prompts
`Сколько будет два плюс два?` and `Формула корней квадратного уравнения в LaTeX`. That LLM path is
what `prompts/title.md` is for — the bundled model takes the first message as-is and ignores the
file entirely.

## Layout

| File | Role |
| --- | --- |
| `src/auth.rs` | Credential load, JWT expiry check, refresh, atomic persist |
| `src/store.rs` | Settings and session files under `~/.config/exp-1/sessions/` |
| `src/catalog.rs` | Model and reasoning-effort catalogue from the Codex cache |
| `../mygame/titler` | Path dependency: the local T5 that names sessions |
| `src/protocol.rs` | Request/response wire types |
| `src/md.rs` | Markdown and pipe tables to styled runs |
| `src/latex.rs` | LaTeX math to Unicode |
| `src/api.rs` | HTTP transport, SSE decoding, header construction |
| `src/tools.rs` | `read`, `write`, `edit`, `shell`, `ast`, `ask_question`, plus the background/output registry |
| `src/app.rs` | Threads, TUI state, rendering, session snapshot/restore |
| `src/main.rs` | CLI, key handling, agent loop |

## Limits

This is an MVP. Known gaps:

- Vertex takes gcloud's application-default credentials only; a service-account key, which needs a
  signed JWT, is not minted.
- The side chat sees the whole main conversation on every question — the main history is read,
  never written — so a `/btw` answer has full context while the main thread stays untouched.
- No streaming cancellation beyond aborting the task.
- Commands run unsandboxed.

## Verified against

Protocol details were checked against the `openai/codex` source and confirmed with live
requests: endpoint and headers, the refresh rotation, text streaming, a `function_call` round
trip, and a multi-iteration tool loop that wrote a file.