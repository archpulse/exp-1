[English](README.md) · [Русский](README.ru.md) · [Українська](README.uk.md) · [Čeština](README.cs.md) · [Español](README.es.md) · [Deutsch](README.de.md) · **中文** · [日本語](README.ja.md)

# exp-1 代理

<img src="assets/exp-1-logo.png" alt="exp-1 logo" width="96" />

一个用 Rust 编写的极简编码代理，带终端 UI，与 **27 个提供方**对话，并向每个提供方传递它实际提供的模型列表：目录实时来自 models.dev，二进制中的一张表则是离线后备。五种线协议覆盖它们全部——由十几家厂商共享的 **OpenAI chat-completions** API、**OpenAI Responses**、**Anthropic messages**、Vertex AI 原生的 **Gemini `generateContent`** 以及 Codex 后端——而提供方通过 API key、设备码登录、厂商自有登录或机器已经持有的凭据来登录。Codex 路径最先出现，至今仍是其中之一：对接 `auth.openai.com` 的 ChatGPT OAuth，请求发往 `https://chatgpt.com/backend-api/codex/responses`。

## 功能

- 以流式从 Codex Responses 端点接收响应（SSE，手工解析）。
- 运行代理循环——请求 → 流式接收 → 运行工具 → 回填输出 → 重复——直到模型不再请求工具。
- 维护两个线程：主对话，以及一个用于提不该打断主对话的问题的侧线程。
- 在同一条记录中显示推理、流式助手文本、工具调用及其输出，并带有每个回合的耗时和时钟时间。

## 工具

| 工具 | 参数 | 说明 |
| --- | --- | --- |
| `read` | `path`, `limit?`, `pos?` | 默认最多 400 行；`limit` 可以改变这一数量，`pos` 是起始行，从 1 计数。输出带行号，被截断的读取会说明如何继续。3000 行的文件是刻意分段读取的：一次吞下会花费数万个 token。 |
| `write` | `path`, `content` | 创建父目录；覆盖已存在的文件。 |
| `edit` | `path`, `old`, `new`, `edits?` | 精确替换，全有或全无。失败时会指出最接近的行或匹配的位置，因此空白字符不一致不需要重新读取。`edits` 可为同一个文件在一次调用中传入多组 `{old, new}`。结果显示改动内容。 |
| `fetch` | `url`, `limit?`, `pos?` | 把一个页面作为纯文本获取，像文件一样分页：从 `pos`（从 1 计数）开始的 `limit` 行（默认 400）。脚本和样式会被丢弃，实体会被解码。 |
| `count` | `path`, `refresh?` | 文件中的行数，或目录下每个文件的行数，无需读取内容。按文件缓存在 `.cache/count`，以大小和 mtime 为键。 |
| `grep` | `pattern`, `path?`, `include?`, `max?` | 在文件或目录树中做正则表达式搜索，不把文件读进对话。`include` 按文件名过滤（`*.rs`）。跳过 `.git`、`.cache`、`target`、`node_modules`；不跟随符号链接；忽略非文本文件。最多 `max` 个匹配（默认 100）。 |
| `delete` | `path` | 删除文件。目录会被拒绝——清空一棵目录树属于 `shell`，在那里整条命令都可见。 |
| `shell` | `command`, `background?`, `limit?`, `output_id?` | 前台命令会阻塞代理直到其结束；60 秒后命令会被提升到后台，结束时通知代理。`background: true` 立即返回一个 id。后台命令绝不会被定时杀掉。输出上限为 30 000 字节：更长的输出保留开头和结尾，中间的空缺会被标记。`output_id` 可以用不同的 `limit` 重新读取任何更早的运行。 |
| `ast` | `path?`, `refresh?` | 以 `tree` 格式展示项目树。缓存在 `.cache/ast`；`refresh: true` 会重建。跳过 `.git`。 |
| `websearch` | `query`, `numResults?`, `type?`, `livecrawl?`, `contextMaxCharacters?` | 带页面内容的网络搜索。使用 Exa 托管在 `mcp.exa.ai` 的 MCP，**无需 API key**；设置 `EXA_API_KEY` 可以解除速率限制。`type` 取 `auto`/`fast`/`deep`。 |
| `ask_question` | `question`, `options?`, `label?`, `submit?` | 阻塞该回合并显示一个问题面板。`↑↓` 选择选项，`Tab` 在多个问题之间移动，`Enter` 确认，`Esc` 关闭。输入的任何文字都会作为自由回答，并优先于高亮的选项。 |

Shell 命令以用户身份在无沙箱下运行。前台命令在 60 秒后被提升到后台，没有任何东西会按定时杀掉后台命令，输出上限为 30 000 字节：更长的结果保留开头和结尾，中间的空缺标记为已省略。

`web` 工具用于网络搜索：优先用 Exa 的托管 MCP，因为它会返回页面内容；当 Exa 的共享免费额度触发速率限制时，改用 DuckDuckGo 的 HTML 页面。`EXA_API_KEY` 会把 Exa 切换到你自己套餐，是两条路径都唯一需要的设置。


## 平台

这个代理对 Linux、macOS 和 Windows 是同一份代码库：TUI、代理循环、提供方、MCP、技能、会话和内置的 titler 都是纯 Rust，到处都能运行。接触操作系统的部分按平台分支：

| | Linux | macOS | Windows |
| --- | --- | --- | --- |
| TUI、代理、提供方、MCP、技能、会话 | 是 | 是 | 是 |
| 登录页面打开器 | `xdg-open` | `open` | `cmd /C start` |
| 守护进程存活检测与停止 | `kill` 0 / SIGTERM→SIGKILL | 同上 | `tasklist` / `taskkill /T` 然后 `/F` |
| 从控制台分离的服务器 | `process_group(0)` | 同上 | `DETACHED_PROCESS \| CREATE_NEW_PROCESS_GROUP` |
| 配置和状态目录 | `$XDG_*`, `~/.config` | `$XDG_*`, `~/.config` | `%APPDATA%`, `%LOCALAPPDATA%` |
| 剪贴板图片（`ctrl + v`） | `wl-paste`, `xclip` | `pngpaste` | PowerShell + `System.Windows.Forms` |
| 回合结束通知 | `notify-send` | `osascript` | PowerShell 气泡提示 |
| 压缩包安装（`exp-1 install *.zip`） | `unzip` / `tar` | 同上 | `tar`（bsdtar 能读 zip） |
| 名为 `npx` 的本地 MCP 服务器 | 按原样 | 按原样 | 以 `npx.cmd` 重试 |

守护进程和剪贴板的 Unix 与 Windows 分支都通过在 Linux 上临时启用它们做过编译检查；`creation_flags` 是唯一一个没有 Windows 目标就无法检查的 Windows 专属调用，而且这里没有任何东西在真正的 Windows 或 macOS 机器上运行过。

### 从 Linux 交叉构建

在 Arch 上，用 mingw 工具链可以构建 Windows 二进制（这需要一次 root）：

```sh
sudo pacman -S --needed mingw-w64-binutils mingw-w64-gcc
rustup target add x86_64-pc-windows-gnu
cargo build --release --target x86_64-pc-windows-gnu
```

导入库需要的是来自 `mingw-w64-binutils` 的 `dlltool`；没有它，构建会停在 `windows-sys`。

没有 root 时，同一个二进制可以从解包到 `~/.local` 下的 llvm-mingw tarball 中得到。该目标的 Rust std 会向链接器索要 `libgcc.a`，而 llvm-mingw 并不提供它，所以用两个符号链接组成的垫片提供等价物，并用 `--no-default-features` 像 CI 那样去掉内置的 titler：

```sh
mkdir -p ~/.local/mingw-shim
ln -sf ~/.local/llvm-mingw/lib/clang/*/lib/windows/libclang_rt.builtins-x86_64.a ~/.local/mingw-shim/libgcc.a
ln -sf ~/.local/llvm-mingw/x86_64-w64-mingw32/lib/libunwind.a ~/.local/mingw-shim/libgcc_eh.a
PATH="$HOME/.local/mingw-shim:$HOME/.local/llvm-mingw/bin:$PATH" \
CARGO_TARGET_X86_64_PC_WINDOWS_GNU_LINKER="$HOME/.local/llvm-mingw/bin/x86_64-w64-mingw32-clang" \
RUSTFLAGS="-L $HOME/.local/mingw-shim" \
cargo build --release --target x86_64-pc-windows-gnu --no-default-features
```

来自 `~/.cargo/bin` 的 `cargo` 和 `rustup` 认识这些交叉目标；系统包里的不认识，而这台机器上 `~/.cargo/bin` 不在 `PATH` 上，所以要用完整路径调用它们。

macOS 二进制同样不需要 root，用 `cargo zigbuild` 就能为 `aarch64-apple-darwin` 构建整棵树：`zig cc` 交叉编译 C 依赖（包括 `aws-lc-sys`，这是普通的 `cargo check` 做不到的——宿主机的 `cc` 会拒绝 `-arch arm64`），每个 crate 都能编译。挡住它的是最后一步：`reqwest` 0.13 的 `rustls` 特性会拉入 `rustls-platform-verifier`，链接器随后报 `unable to find framework 'Security'` 和 `'CoreFoundation'`。这些只存在于 Apple SDK 中；Zig 能顺利链接 Mach-O 和 libSystem，但并不附带这些 framework。所以 macOS 二进制来自 Mac 或 CI（`.github/workflows/release.yml` 会构建两种架构）。下面的步骤能做到链接那一步：

```sh
mkdir -p ~/.local/zig && curl -L https://ziglang.org/download/0.14.1/zig-x86_64-linux-0.14.1.tar.xz \
  | tar -xJ -C ~/.local/zig --strip-components=1
cargo install cargo-zigbuild
rustup target add aarch64-apple-darwin
PATH="$HOME/.local/zig:$PATH" cargo zigbuild --release --target aarch64-apple-darwin
```

结果是未签名的。在 Apple Silicon 上，未签名的二进制一启动就会被杀掉，所以必须在 Mac 本机上签名一次：

```sh
codesign -s - --force ./exp1-agent        # ad-hoc signature
xattr -cr ./exp1-agent                    # if it arrived by download
```

## 要求

- Rust 1.85+（edition 2024）
- 一个已有的 Codex CLI 登录（会复用它的 `auth.json`），或者 `/login`，它会自己运行同样的设备码流程。
- 内置的 titler 在 `titler` feature 下从 `src/titler.rs` 编译进代理。它会引入 Candle，因此首次构建要花些时间，二进制也约 21 MB 而不是约 14 MB。权重进入 Hugging Face 缓存后，起标题就是离线的。

## 安装

安装 crates.io 上的最新版本需要 Rust 1.85+ 和 Cargo：

```sh
cargo install exp1-agent
exp1-agent --help
```

Cargo 会将 `exp1-agent` 安装到 `~/.cargo/bin`；如有需要，请将该目录加入 `PATH`。默认构建包含本地 titler，因此首次安装会因编译 Candle 而耗时更久。

## 用法

```sh
cargo run --release                      # uses the current directory
exp1-agent --model gpt-6-luna --effort high # pick a model and reasoning effort
exp1-agent --cwd ~/projects/my-app          # run the shell tool somewhere else
exp1-agent --help
```

| 按键 | 操作 |
| --- | --- |
| `Enter` | 发送提示词；当命令已经完整时则运行该命令 |
| `↑` `↓` | 移动补全选项，或调取之前的提示词 |
| `Tab` | 接受高亮的补全项 |
| `Ctrl + /` | 在主线程和侧线程之间切换 |
| `Ctrl + Shift + A` | 切换权限模式：always ask → never ask → ask only commands |
| `Esc` | 单独按下没有作用 |
| `Esc` `Esc` | 中止正在运行的回合，或关闭补全列表或选择器 |
| `Ctrl-C` | 退出 |
| `Ctrl-U` | 清空输入行 |
| `Ctrl-Backspace` | 删除前一个词 |
| `Ctrl-Delete` | 删除后一个词 |
| `Ctrl-W`, `Alt-Backspace` | 同 `Ctrl-Backspace` |
| 鼠标滚轮 | 滚动记录 |
| `PgUp` / `PgDn` | 滚动记录 |
| `←` `→` `Home` `End` | 移动光标 |
| `Backspace` / `Delete` | 编辑 |

方向键用于浏览提示词历史：发送 `hi` 之后，`↑` 会把它带回输入框，继续按会继续向前回溯，`↓` 则向前走到空输入框。滚动由鼠标滚轮负责，方向键因此留给历史记录。

输入 `/` 会打开按前缀过滤的补全列表。输入空格之后，它也会补全命令的参数——`/themes <tab>` 列出配色方案，`/model <tab>` 列出模型，`/model gpt-6-luna <tab>` 列出推理强度。在裸的 `/model` 或 `/themes` 上按 `Enter` 则会打开一个编号选择列表；`/model` 分两步，先模型后强度。输入框为空时，光标停在最开头，位于占位符之前。

提示行会随上下文变化。回合运行期间输入被锁定。

`Esc` 是刻意无作用的；真正打断的是连按两次。打断会中止进行中的请求，并丢弃任何不完整的助手文本，对话历史保持不变。

## 斜杠命令

在本地处理；它们永远不会送到模型。

| 命令 | 效果 |
| --- | --- |
| `/login` | 选择一个提供方：已登录的会直接切换；选中正在使用的那个则会重新登录。设备码登录会在浏览器中打开厂商页面；其余的需要 API key |
| `/scoped-models` | 勾选 `/model` 允许提供的模型；为空表示全部 |
| `/sessions` | 选择一段已保存的对话重新打开 |
| `/init` | 在工作目录中写入一个 `AGENTS.md` 骨架 |
| `/model` | 打开两步选择器（先模型，后强度）；`/model <name> [low\|medium\|high]` 可直接设置 |
| `/themes` | 打开主题选择器；`/themes <name>` 可直接设置 |
| `/status` | 显示模型、强度、工作目录、工具、主题、计划、认证提供方 |
| `/btw` | 在侧线程中回答一个侧边问题，同时主回合继续运行 |
| `/clear` | 清空记录和对话历史 |
| `/help` | 显示键盘快捷键 |
| `/compact` | 立即总结对话，而不是等窗口填满 |
| `/undo` | 恢复上一个回合改动的文件（仅在内存中；重启后忘记） |
| `/diff` | 上一个回合改动的统一 diff |
| `/fork` | 把这段对话作为新会话继续，原会话保持不变 |
| `/schedule` | 让请求之后自行运行：`/schedule <做什么以及何时>` 创建任务，直接输入 `/schedule` 列出任务，`rm` 和 `pause` 管理任务 |
| `/sessions <text>` | 不只是列出，而是搜索已保存的对话 |
| `/retitle` | 根据原始请求和最近的对话重新给会话命名 |
| `/effort` | 为当前模型选择推理强度，不改变模型 |
| `/mcp` | 列出 MCP 服务器及其提供的工具 |

可用主题：`default`（蓝色）、`amber`、`jade`、`mono`。颜色为 truecolor，因此终端主题无法重新映射它们。

## 界面

布局沿用 OpenAI Codex TUI：没有带边框的面板，一个 `>_` 页眉，其下是工作目录，输入框上方是补全弹窗或编号选择器，一块带内边距的输入区，配 `›` 提示符和闪烁的 `│` 光标，然后是模型行和提示行。

- 已提交的提示词会像 Codex 那样，在记录中以整宽条带的形式回显。
- **工具输出默认隐藏**：记录中只有调用（`read src/main.rs`），没有它返回的内容。`ctrl-o` 会在调用下方显示输出，再按一次又隐藏，这就是跟进工作和逐字阅读之间的区别。
- **鼠标不只是用来滚动的。** 滚轮滚动，拖动选择和复制，点击则作用于落在其上的任何东西：选择器或补全弹窗中的某一行，行为等同于在该行按 Enter；页脚中的 `ctrl + / for side` 提示会切换线程；消息中的链接会在浏览器中打开。每一帧都会记录它绘制的区域，因此点击是对最后一帧做命中检测，而不是对布局的猜测。悬停高亮被刻意不实现：它会让每次鼠标移动都产生一次重绘，而终端自己的光标已经说明了指针在哪里。
- 子代理在**一行会更新的文字**里报告当前步骤（`subagent → grep tokenizer`），而不是每一步一行；启动它的那个调用显示为 `task: <description>`。
- **点击你自己的消息会先问要做什么**，而不是直接做：一个小菜单，包含 `copy`（文本进入剪贴板）和 `revert`。回退会丢弃该消息之后的一切——对话、该点之后的模型所被告知的内容，以及其后各回合做出的每一处文件改动，后者由撤销日志恢复。该消息本身会离开记录，其文本回到输入框，因此可以编辑并再次发送。丢弃任何东西之前都会先检查文本，所以点错了消息也不会有任何效果。
- 记录会停留在最新消息；鼠标滚轮或 `PageUp`/`PageDown` 向上滚动历史，`ctrl-g` 跳回最新消息，`ctrl-t` 跳到最旧消息，而向上滚动后的视图在新行到来时会停在原处，而不是被拖回底部。
- 用鼠标拖动可以选中屏幕任何位置的文本——包括页眉——松开后放入剪贴板：先用 OSC 52，所以在 ssh 上也能用；然后是 `wl-copy`、`xclip` 或 `xsel`（如果存在）。提示行中的 `copied N chars` 提示会确认。选中内容是文字后面的一层淡色底纹，并在每一行的最后一个字符处停止，行为与终端自身的选择一致。终端从不会看到拖动，因为应用为滚动占用了鼠标，所以这是唯一的复制方式。
- 附加的图片会显示为输入框上方一个实心的 `Image 1` 标签，旁边是 `ctrl-v replaces it`，因此即将发送的内容在输入区本身就可见。
- 回合运行时，活动行位于**它所属的提示词正下方**——每件事都有自己的标签：请求进行中显示 `⠋ thinking 4s`，模型仍在写工具调用参数时显示 `⠋ preparing write… (12 KB) 8s`，工具执行时显示 `⠋ running 4s`。每个标签都带自己的时钟，所以秒数反映的是当前正在发生的事，从不统计整个回合；回合的总耗时由结尾的 `Worked for` 报告。底部栏始终是朴素的 `GPT-6-Luna high · /path · <thread> · Main [default]` 标签；`esc esc interrupts` 位于提示行。
- 回合结束后，`Worked for 5s · 12:08 PM` 会追加到记录中。
- 模型行显示 `GPT-6-Luna high · /path · <thread name> · Main [default]`，在侧线程中则是 `… · Agent`。线程名取自第一条提示词。
- 提示行右侧带有线程切换，侧线程激活时显示为红色。
- 空屏保持安静：完整命令列表只在从未保存过对话的机器上出现，之后就只有一行，指向 `/` 和 `/help`。
- 回合期间输入仍然可用，因此可以不等候就提出 `/btw` 问题。
- `/btw` 或 `ctrl + /` 会在侧边对话中作答，它把整个主对话作为上下文，但不向其中添加任何东西。
- `shift + tab` 在 build 和 plan 模式之间切换；开启时页脚显示 `plan mode`。
- 运行超过十五秒的回合在结束时会让终端响铃（桌面支持时还会调用 `notify-send`），这样长时间的任务不必一直盯着。
- `shift + enter` 写入换行而不是发送，所以提示词可以有多行；输入区会变高以容纳。
- `ctrl + v` 从剪贴板取一张图片（Wayland 用 `wl-paste`，X11 用 `xclip`）并附加到下一条提示词，在发送前提示行会显示 `▣ image attached`。图片以 data URL 形式传输，所以 Codex、OpenAI、Anthropic 和 Gemini 后端都能传递它。
- `ask_question` 会在输入框上方升起一个面板，与 Question 工具相呼应：左边缘有一条强调色竖条，标题为 `Questions`，同时问多个问题时显示标签页，编号选项并突出显示当前高亮项，一个 `Type your own answer` 槽位，底部是 `⇥tab ↑↓ select enter confirm esc dismiss`。

颜色为 truecolor，因此终端主题无法重新映射它们。模型、强度和主题可在运行时通过 `/model` 和 `/themes` 切换。

与 Codex 的差异是刻意的：Codex 有 `/permissions` 和 `/review`，它们需要沙箱策略和 diff 审查器；这个构建改用 `/clear` 和 `/help`。Codex 的页脚会宣传代理和快捷键浮层，这个 MVP 没有。

## 认证

`/login` 列出提供方，每个提供方按其厂商期望的方式登录：

- **一个 API key**，用于大多数提供方：以 `Authorization: Bearer` 发送（Anthropic API 则用 `x-api-key`），按提供方存储在 `~/.config/exp-1/credentials.json` 中，模式 `0600`。一个 key 可以覆盖多个提供方——`opencode-go` 和 `opencode-zen` 是两个 base URL 背后的同一个账户，`providers::key_group` 为这些分组命名。
- **设备码登录**，用于 `openai (auth)`、`github copilot` 和 `meta (auth)`：浏览器中打开厂商的登录页面，URL 和代码作为后备打印出来。
- **来自厂商自有登录的 token**，用于 `anthropic (auth)`，随官方客户端的身份头发送。
- **机器已经持有的凭据**，用于 `google vertex`：`gcloud auth application-default login` 配置好的那些，由 `GOOGLE_CLOUD_PROJECT` 和 `GOOGLE_CLOUD_LOCATION` 指明项目和区域。无需粘贴任何内容。

Codex 路径另外保有一个自己的文件：首次运行时，代理会把 `$CODEX_HOME/auth.json`（默认 `~/.codex/auth.json`）复制到 `~/.exp1-agent/auth.json`，并使用这份副本。**你真正的 Codex 凭据绝不会被改写。** 当访问 token 距过期不足 5 分钟时，代理会刷新它：

```
POST https://auth.openai.com/oauth/token
grant_type=refresh_token
client_id=app_EMoamEEZ73f0CkXaXp7hrann
```

响应会轮换 refresh token，因此新的会写回代理自己的副本。存储的文件以原子方式写入，权限模式为 `0600`。

## 模型与强度

模型列表和推理强度都不是硬编码的。两者都从 Codex CLI 自己的目录 `$CODEX_HOME/models_cache.json`（默认 `~/.codex/models_cache.json`）读取，使用每个条目的 `slug`、`display_name`、`description`、`visibility`、`default_reasoning_level` 和 `supported_reasoning_levels`。启动时的默认值是目录中列出的第一个模型。

**其他所有提供方都实时从 models.dev 获取自己的列表。** 该目录最多每十二小时抓取一次到 `~/.cache/exp-1/models-dev.json`，保存在内存中，并用于三件事：提供方提供的模型 id、每个模型使用的协议（models.dev 记录的是该提供方配置的 AI SDK 包——网关后面的 Claude 模型是 `@ai-sdk/anthropic`，Grok 模型是 `@ai-sdk/openai`），以及上下文窗口，自动压缩阈值正是据此计算的。目录中缺失的提供方会沿用 `src/providers.rs` 里的表，离线的机器也是如此，所以应用从不依赖这次抓取。

表格完全没收录的模型仍然会得到一个协议：由其厂商前缀决定（`claude-*`、`minimax-*` 和 `qwen-*` 说 Anthropic，`grok-*`、`gpt-*` 和 `muse-*` 说 Responses，`gemini-*` 说 Gemini，其余的都是 chat），而全程只说一种协议的提供方会替其他一切作答。

**选择器会显示所有已登录的提供方**，是一个货架而不是每次登录一个：各行带有 `via <provider>`，以便把同一模型名下的两顶帽子区分开；正在使用的提供方排在最前；从另一个提供方选择模型时，会先切换到它再设置模型。这么长的货架靠输入来搜索：每个选择器都随输入过滤，退格会放宽匹配，Escape 在关闭之前先放宽。`/scoped-models` 中的范围过滤的是同一个货架，并指明某个提供方的模型（`opencode-go/grok-4.7`），所以来自两个提供方的同名模型是两条条目；持有裸名字的旧范围仍然表示“这个名字，来自任何提供方”，在该选择器中按 `ctrl-a` 会清除范围，重新提供所有选项。当多个提供方都提供该名字时，`/model grok-4.7` 取正在使用的那个提供方，而 `/model opencode-go/grok-4.7` 精确指明一个。

强度确实因模型而异，所以猜测是行不通的：

| 模型 | 强度 |
| --- | --- |
| `gpt-5.6-terra` | low, medium, high, xhigh, max, **ultra** |
| `gpt-6-luna` | low, medium, high, xhigh, max |
| `gpt-5.5` | low, medium, high, xhigh |

强度选择器、`/model <name> <tab>` 补全和校验都跟随当前激活的模型。切换模型时，如果新模型接受当前强度就沿用，否则回退到该模型的 `default_reasoning_level`，所以你永远不会请求一个模型并不具备的强度：

```
/model gpt-5.6-terra ultra   →  GPT-5.6-Terra ultra
/model gpt-5.5               →  GPT-5.5 medium          (ultra is not a gpt-5.5 level)
/model gpt-5.5 max           →  'max' is not a GPT-5.5 level;
                                this model takes low, medium, high, xhigh
```

只有 `visibility: "list"` 的模型会出现在选择器中；隐藏的模型仍可按名字访问。如果目录文件缺失，应用会回退到一份保守的内置列表以保持可用，并在选择器中说明这一点。

只有已登录的 ChatGPT 账户有权使用的模型才会真正作答——其余模型会被后端以带说明的错误拒绝。

## 请求形状

```jsonc
POST https://chatgpt.com/backend-api/codex/responses
Authorization: Bearer <access_token>
chatgpt-account-id: <tokens.account_id>
originator: codex_cli_rs
User-Agent: codex_cli_rs/<version> (<os>; <arch>) terminal
Accept: text/event-stream
Content-Type: application/json
session-id: <the session's id, stable for the whole conversation>

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
  "parallel_tool_calls": true,
  "prompt_cache_key": "<the session's id>",
  "reasoning": { "effort": "medium", "summary": "auto" },
  "store": false,
  "include": []
}
```

消费的事件：`response.output_text.delta`、`response.reasoning_summary_text.delta`、`response.output_item.done`、`response.completed`、`response.failed`、`response.incomplete`、`error`。其他一切都被忽略。

## 渲染

助手回答在进程内渲染为富文本。没有外部渲染器，没有 `pandoc`，没有 TeX 二进制，没有图像光栅化——一切都在 TUI 内部完成，因此几乎没有开销。

- **Markdown**：标题、粗体、斜体、行内代码、带行号栏和语言标签的围栏代码块、无序和有序列表、引用块、水平分割线、链接、用制表符边框和逐列对齐排布的 GitHub 风格管道表格，以及行内 `$…$` 数学公式。
- **LaTeX 数学**：符号和关系、希腊字母、`\frac`、`\sqrt`、上下标、`\mathbb` 和 `\mathcal`、组合重音（`\bar`、`\hat`、`\vec`、…）、正体运算符名（`\log`、`\sin`、`\lim`、…），以及行间 `$$…$$` 块。

转换器不认识的内容会原样输出，包括花括号，因此陌生的输入仍然可读，而不会变成一团乱码。它是转换器，不是 TeX 引擎：宏、`tikz` 之类的环境以及多行排版都不会被解释。

Markdown 在消息到达时**只解析一次**，解析成与主题无关的片段；绘制只是把这些片段映射到颜色。因此切换主题不会重新解析任何东西。

在构建出的二进制上实测：光标闪烁的空闲状态下**约占单核的 1%**，快速滚动滚轮时约 3%。

## 子代理与长命令

**子代理。** `task` 工具会运行第二个代理，它有自己的上下文和相同的工具，然后只返回它的报告——对于否则会淹没调用者上下文的工作很有用。报告会被截断到 6 000 个字符，因为它会进入调用者的上下文；记录用一行而不是每步一行来显示其进度，每一步都替换这一行。只要还有深度，就会提供嵌套子代理，但最多两层，以免递归失控。记录会显示子代理走的每一步。

**慢命令被提升，而不是被杀掉。** 前台命令在 **60 秒**后仍在运行就会被移到后台，并通知代理，而不是阻塞回合：

```
'sleep 75' is still running after 60s, so it was moved to the background as 'bg-1'.
Carry on; you will be told when it finishes, or poll it with {"output_id": "bg-1"}.
```

**完成会打断。** 后台命令结束时，正在运行的回合会被中止，并把输出交给代理，这样结果永远不会在长时间工作背后悄然丢失：

```
• interrupted: a background command finished
```

## 听写

底部角落的麦克风按钮（`ctrl-r` 作用相同）向输入框听写：按下它，说话，再按一次。文字会在你说话时到达，因为识别器是流式的，按钮在聆听期间带有一个计时器。

识别器是 **whistle**——Cactus 的设备端语音引擎——以 `whistle --mic` 启动。正是这个模式让它流式工作：它通过 pipewire 的 `pw-record` 每次记录一秒原始的 16 kHz 单声道音频，每秒送入流式解码器，每次更新打印一行 JSON（`text` 是已确定的内容，`pending` 是仍在处理中的内容）。其 stdin 上的换行会结束流并冲刷尾部。命令、语言、偏置词和音频来源都来自环境：

录音器是平台自己的：Linux 上是 `pw-record`（pipewire），macOS 上是带 avfoundation 的 `ffmpeg`，Windows 上是带 dshow 的 `ffmpeg`（`WHISTLE_SOURCE` 选择输入设备，`WHISTLE_RECORDER` 直接替换命令）。应用本身只启动 `EXP1_DICTATE --mic` 并读取它的 JSON 行，所以任何以同样方式流式工作的识别器都可以用；没有的话，按下麦克风按钮只会在页脚说明这一点，别的什么都不会发生。

```sh
EXP1_DICTATE=/path/to/whistle     # default: whistle on PATH
EXP1_DICTATE_LANG=en              # default: whatever it detects
EXP1_DICTATE_KEYWORDS=country,Americans
WHISTLE_SOURCE=…                  # default: the system's own input
```

听写仅限 Linux，因为录音器仅限 Linux；代理的其余部分不是。

这个模式是本副本的 whistle 特有的：逻辑以 `Dictation` 类型（`start`、`next`、`finish`）位于它的库中，其 `main.rs` 只是它上面的一层薄循环。whistle 在这里没有自己的仓库，所以从作者那里新取来的副本必须重新应用这两个文件，或者把 `EXP1_DICTATE` 指向一个说同样协议的东西。不认识 `--mic` 的识别器不会被忽视：它会用自己的话在页脚说明这一点。

## 提供方

`/login` 打开一个提供方选择器；`/scoped-models` 决定 `/model` 提供它们中的哪些模型（为空表示全部，范围存储在 `settings.json` 中）。选择器是一个朴素的字母顺序列表，光标初始停在正在使用的提供方上，因此不必靠位置去寻找。选择一个已经登录的提供方会切换到它并说明这一点；选择已经在用的那个——或运行 `/login <id>`——则会刻意重新登录。设备码登录会在浏览器中打开厂商页面，并打印 URL 和代码作为后备。

**一个 key 可以覆盖多个提供方。** `opencode-go` 和 `opencode-zen` 是两个 base URL 背后的同一个账户，所以登录任意一个都会为两者存储 key（`providers::key_group` 为这些分组命名）。opencode 网关还按会话路由：每个请求都带有 `x-opencode-session`、`x-opencode-project` 和 `x-opencode-client`，没有这些网关会直接拒绝（"Request is missing x-opencode-session"）。

五种线协议覆盖了这里实现的一切：由十几家厂商共享的 **OpenAI chat-completions** API、**OpenAI Responses**、**Anthropic messages**、Vertex AI 原生的 **Gemini `generateContent`**，以及 `openai (auth)` 与之对话的 **Codex 后端**。它们全都以 SSE 流式传输，并全部解码为 Codex 客户端产生的同一事件流，因此代理循环并不知道自己在和哪一个对话。

key 按提供方存储在 `~/.config/exp-1/credentials.json` 中，模式 `0600`。`openai (auth)` 则保留 Codex 风格的 `auth.json`，`google vertex` 使用 `gcloud auth application-default login` 已经登录的凭据（由 `GOOGLE_CLOUD_PROJECT` 和 `GOOGLE_CLOUD_LOCATION` 指明项目和区域），设备码提供方把收到的 token 放进同一个凭据文件。

| 提供方 | Base URL | 协议 |
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

每个条目都已实现：每个模型都指明自己用哪种协议作答，所以像 `opencode-go` 这样的网关可以同时提供三种。列表和 base URL 来自 [models.dev](https://models.dev)——opencode 自己读取的公开注册表——而不是猜测。

上面每个 base URL 都用故意无效的 key 探测过：被拒绝就证明 URL 和认证头是对的。全部二十七个都以 `400`、`401` 或 `403` 作答——除了 `fireworks-ai`，它的 `404` 是它自己的 `"Model not found"`，带一个 `chatcmpl-` 请求 id，所以就连那个请求也到达了推理路由。`google vertex` 在其项目作用域路径上返回 `401`。

## 模式

`shift + tab` 在两种模式之间切换会话，页脚会说明当前是哪一种：

- **build**——一切如本文所述。
- **plan**——会改动文件的工具（`write`、`edit`、`delete`）会被拒绝，*并且*会从工具列表中剔除，因此模型会调查并提出计划，而不是编辑。`shell` 预期保持只读。措辞位于 `prompts/plan.md`，和其他文件一样可编辑。

## 权限

`~/.config/exp-1/permissions.json` 是一份例外列表；不匹配任何规则的调用会直接运行。第一个匹配决定结果：

```json
{
  "rules": [
    { "action": "shell", "resource": "rm -rf *", "effect": "deny" },
    { "action": "write", "resource": "/etc/*", "effect": "ask" },
    { "action": "fetch", "resource": "*", "effect": "allow" }
  ]
}
```

`action` 是工具名或 `*`，`resource` 是调用触及的东西——文件工具是路径，`shell` 是命令行，网络工具是 URL 或查询——`*`/`?` 作为通配符生效。`effect` 是 `allow`、`ask` 或 `deny`。`ask` 会升起问题面板并扣住调用直到被回答（两分钟，之后调用被拒绝）。规则同样适用于子代理。

`ctrl + shift + a` 在权限模式之间循环：**always ask**（全新安装的默认值）让 `write`、`edit`、`delete` 和 `shell` 都先提问，**never ask** 直接执行而不提问，**ask only commands** 只对 `shell` 提问。只读工具从不提问，而 `effect: deny` 规则在任何模式下都会拒绝——模式永远不会放宽 deny。

## 项目指令

`AGENTS.md` 会从工作目录及其每一个上级目录读取，从最外层开始，并追加到系统提示词——因此仓库自己的规则无需粘贴进对话就能到达模型。`/init` 会写入一个起始文件。

## 提供方故障

服务器端或传输故障会在两秒后重试一次；被拒绝的请求（4xx）不会。当主模型持续失败时，`settings.json` 中的 `fallback_model` 指明同一提供方的一个模型，在放弃回合之前尝试一次，这次切换会在记录中注明。

## 技能

技能是一个包含 `SKILL.md` 的目录，形态与 Claude Code 和 opencode 使用的一致。安装一个技能会让它成为一个命令：

```sh
exp-1 install ~/skills/pdf-tools          # a directory with SKILL.md
exp-1 install notes.md                    # a single markdown file
exp-1 install tools.zip                   # an archive (unzip or tar)
exp-1 install https://example.com/skill.md
```

文件落在 `~/.config/exp-1/skills/<name>/`，有 frontmatter 的 `name:` 字段时以它命名，否则以文件或目录名命名。已安装的技能会出现在补全弹窗中内置命令旁边，以及空状态帮助中，`/<name>` 可以运行一个：服务器读取技能的指令，把它们作为任务写入对话，并让模型遵循它们。选择器显示的是 frontmatter 中的 `description:`。

## MCP 服务器

来自 [MCP](https://modelcontextprotocol.io) 服务器的工具与内置工具一起提供给模型。列表位于 `~/.config/exp-1/mcp.json`，形态与 opencode 使用的完全一致，因此条目可以在两者之间复制：

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

- `local` 服务器会被启动并通过 stdio 通信；`remote` 服务器通过 HTTP 访问，答案可以是 JSON 或 SSE。`"disabled": true` 会跳过一个服务器而不删除它，`"headers"` 添加远程服务器需要的东西。
- 工具以 `<server>_<tool>` 的形式到达模型，两半都按 opencode 的净化方式处理（`context7_resolve-library-id`），因此两个服务器永远不会冲突。
- 服务器在启动时**在后台**连接：慢的或死掉的服务器永远不会拖住代理，而且它会被报告而不是被丢弃——`/mcp` 会列出每个服务器及其工具数量，或它失败时的错误。

## 后台服务器

参照 opencode 的守护进程建模。**一次普通的 `exp1-agent` 启动会拉起服务器并附着到它**，完全像 opencode 的默认命令那样取 `daemon.transport()` 并交给 UI。

结果本身就是意义：关闭 UI 后服务器仍在运行，所以下一次启动会复用一个已经热起来的进程。只有当服务器**同时**是同一版本和同一构建时才会被复用：`server.json` 记录了可执行文件的大小和修改时间，因此自服务器启动以来重新构建过的客户端会替换掉它，而不是跟它讲一种它已经不再说的协议。在这台机器上实测：

| | 冷启动 | 下一次启动 |
| --- | --- | --- |
| UI 启动 → 第一帧 | 189 ms | **84 ms** |
| UI RSS | 14.4 MB | 16.0 MB |
| 服务器 RSS | 15.2 MB | 20.5 MB（一个回合之后） |

同时打开五个会话再次实测：

| | 一个服务器 + 5 个 UI | 五个独立进程 |
| --- | --- | --- |
| 空闲 | 97 MB (15 + 5 × 16) | 183 MB (5 × 37) |
| 带内置 titler | 143 MB | 423 MB (5 × 85) |

服务器对它加载的东西很懒：Codex 客户端在第一个回合才出现（+5 MB），内置 titler 只有在本地真正生成标题时才出现（+48 MB，这就是 `auto` 偏好模型的原因）。这种懒加载正是重点所在——那 48 MB 权重在一台机器上所有会话只需付一次，而不是每个进程付一次。

昂贵、每台机器只需一份的状态——工具注册表和约 49 MB 的 titler 模型——存在于那一个服务器中，而 Codex 客户端在第一次有 Codex 协议请求需要它时才构建，因此只登录了其他提供方的机器启动服务器时不需要 Codex 凭据。每个 UI 都是瘦客户端，所以 N 个并行会话的开销是 `1 x 63 MB + N x 15 MB` 而不是 `N x 61 MB`，并且只有第一次启动需要付出冷启动的代价。

```sh
exp1-agent serve                 # run the server in the foreground
exp1-agent service start         # spawn it detached (survives the launcher)
exp1-agent service status        # pid, url, version, health
exp1-agent service stop          # SIGTERM, then SIGKILL after a grace period
exp1-agent service restart
exp1-agent service password      # print, or set with an argument
```

- **注册**：`~/.local/state/exp-1/server.json` 保存 `{id, version, url, pid}`。客户端读取它，用 2 秒超时检查 `/health`，只有在版本也匹配时才复用服务器——陈旧或外来服务器会先被停止。
- **密码**：`~/.local/state/exp-1/password`，32 个随机字节，模式 `0600`，跨重启保留，因此新启动的客户端无需被告知任何东西就能重连。
- **分离启动**：`exp1-agent serve --register` 在自己的进程组中运行，标准流关闭，因此它比启动它的任何客户端都活得久。

### HTTP API

所有路由都要求 `Authorization: Bearer <password>`。

| 方法 | 路径 | 用途 |
| --- | --- | --- |
| `GET` | `/health` | 存活检测与版本 |
| `GET` | `/sessions` | 列出活动会话 |
| `POST` | `/sessions` | 创建一个；`x-exp1-directory` 设置其工作目录 |
| `GET` | `/sessions/{id}` | 完整快照 |
| `POST` | `/sessions/{id}/prompt` | 开始一个回合 |
| `GET` | `/sessions/{id}/events` | SSE：快照、增量、工具、用量、标题、完成、空闲 |
| `POST` | `/sessions/{id}/answer` | 回答一个待处理的 `ask_question` |
| `POST` | `/sessions/{id}/interrupt` | 中止正在运行的回合 |
| `GET` | `/models` | 目录，含每个模型真实的强度。`?all=1` 忽略范围：设置范围的选择器需要范围之外的模型 |

会话带有自己的工作目录，所以一个服务器可以服务多个项目；目录随 `POST /sessions` 到达，而不是在服务器启动时固定。

## 自动压缩

长对话会在溢出上下文窗口之前被总结。每个回合之后，服务器都会把请求的 `input_tokens` 与 Codex 目录中该模型的 `context_window` 比较，一旦对话达到其 **95%** 就压缩：

```
Context at 7378 of 258400 tokens (95% threshold) — compacting
Conversation compacted; continuing from the summary
```

压缩会用一段总结提示词和 opencode 使用的结构化模板把对话发给模型——Objective、Important Details、Work State（Completed / Active / Blocked）、Next Move、Relevant Files——工具输出被裁剪到 2000 个字符，以免总结器被淹没。

重写后的对话是**摘要，后面跟着逐字保留的最后五个用户回合**，每个回合都带有代理的工具调用及其结果，因此手头的线索不会丢失：

```
[user]      Summary of everything so far: ## Objective …
[user]      Turn two: run shell ["seq","1","900"] and reply OK.
[call]      shell
[result]    exit_code: 0 1 2 3 …
[assistant] OK
… four more turns …
```

压缩之前说过的事实，压缩之后仍然知道。如果其中一个回合本身就很庞大，就会丢弃最旧的尾部条目，直到结果重新装进窗口，因此单个巨大的工具输出不会让对话永远反复压缩。

想用别的比例可以设 `EXP1_COMPACT_PERCENT`，这条路径也正是这样在不填满 272 000 token 窗口的情况下被演练的。

一段对话在磁盘上最多保留 5 000 条记录消息；模型自己的上下文由裁剪和压缩单独约束，所以长会话保持可读，且不花费 token。

## 持久化

配置位于 `~/.config/exp-1/`（或 `$XDG_CONFIG_HOME/exp-1`）；对话本身位于 `~/.cache/exp-1/sessions/`（或 `$XDG_CACHE_HOME/exp-1/sessions`），因为记录是一种缓存：丢失一份只会损失上下文，不会损失设置。守护进程自己的文件——`server.json` 和 `password`——位于 `~/.local/state/exp-1/`。

| 文件 | 保存内容 |
| --- | --- |
| `config/settings.json` | 模型、推理强度、主题、titler 模式、回退模型 |
| `config/credentials.json` | 每个提供方一个 API key，模式 `0600` |
| `config/prompts/*.md` | 系统提示词，可编辑：`system`、`subagent`、`compaction`、`title`、`plan` |
| `config/permissions.json` | allow/ask/deny 规则 |
| `config/mcp.json` | MCP 服务器 |
| `config/skills/` | 已安装的技能，每个一个目录 |
| `cache/sessions/<id>.json` | 一段对话：标题、两份记录、模型侧历史、提示词历史 |

一段对话在磁盘上保留最新的 **250 条消息**，整个缓存会在服务器启动时修剪到 **250 MB**——按最旧文件优先。更旧的布局（一切都放在 `config/exp-1/sessions/`）会在改动后的首次启动时迁移。

设置会在应用退出时写入；会话在每个完成的回合后写入，所以崩溃最多只损失进行中的那个回合。

内置的 titler 是一个可选特性：`cargo build --release --no-default-features` 会把它排除在外，并连带去掉整个 Candle 依赖，会话标题便只来自模型。CI 就是这样构建的，因为 titler 会引入 Candle：crate 就在代码树中，但 release 不必为它付出代价。

提示词是文件，不是常量：`~/.config/exp-1/prompts/system.md`（代理自己的指令）、`subagent.md`、`compaction.md` 和 `title.md` 会在首次启动时写出，并在每个请求时读取，所以一次编辑无需重新构建就会应用到下一个回合。启动时模型和强度的解析顺序是**命令行 → 已保存的设置 → 目录中列出的第一个模型**。

**每次运行都从空会话开始。** 不会自动恢复任何东西——`/sessions` 打开一个已保存对话的选择器，最新的在前，每一项显示生成的标题、id 和最后使用时间。列表按目录划定范围：会话在它启动的目录、该目录以下的任何地方可见，如果在 `$HOME` 中启动则到处可见（“全局”会话）。`$HOME` 本身只列出全局会话，因此不会变成每个项目的目录。选择一个会话会保存你正要离开的对话并加载所选的对话。`/clear` 放弃当前对话并开始新的对话，同时保留你的设置。

### 会话标题

标题来自内置的 **titler**——一个本地 T5（760 万参数，3 层编码器和 3 层解码器），在本进程中通过 Candle 运行，从 `src/titler.rs` 编译进代理。权重（`Cyronius/titler`）从 Hugging Face 缓存读取，每个进程最多加载一次，所以一旦它们落到磁盘上，标题生成就是本地的、即时的、免费的。

生成会在第一个回合结束的瞬间在阻塞线程上运行，绝不在 UI 线程上。有父会话的会话——侧边对话——根本不会被命名，因为它们属于它们所跟随的对话。使用的是库的多语言路径（不是原始路径），所以会检测语言，非英语提示词会先翻译成英语。会话只命名一次，在标题到达之前，选择器显示的是取自提示词的预览。

**由谁为会话命名是一个设置**，即 `settings.json` 中的 `titler`：

| 值 | 行为 |
| --- | --- |
| `auto`（默认） | 先由 Codex 模型命名，内置 titler 作为回退 |
| `local` | 只用内置 titler——没有 API 开销，标题较弱 |
| `remote` | 只用 Codex 模型 |

标题是向目录中便宜的 `luna` 档位索取的，而不是向会话自己的模型——这与 opencode 向提供方索取其小模型是同一个思路——标题花费的 token 会像任何其他调用一样计入会话的用量。

`/retitle` 会再次为会话命名，这次依据原始请求加上最近的对话（上限为 8,000 个字符），这也是 opencode 在会话被重命名为空时重新生成标题的方式。提示词本身位于 `prompts/title.md`，是完整的 opencode 风格文档：一个任务、规则（与用户同语言、≤50 个字符、保留精确术语和数字、绝不回答问题）以及十个示例。

`auto` 有意以 Codex 模型打头。7.6M 的 titler 对自己的体量很诚实：同样的俄语提示词，它为 «сколько будет два плюс два» 生成 `Understanding Two Plus Terms`，为一次表格请求生成 `Request for Sign`，因为 Google 的免费翻译器把 «табличку» 译成 "sign"，然后小模型就给这个错译起了标题。Codex 模型把同样的提示词命名为 `Сколько будет два плюс два?` 和 `Формула корней квадратного уравнения в LaTeX`。LLM 这条路径正是 `prompts/title.md` 的用途——内置模型原样接受第一条消息，完全忽略该文件。

## 布局

| 文件 | 角色 |
| --- | --- |
| `src/auth.rs` | 凭据加载、JWT 过期检查、刷新、原子持久化 |
| `src/store.rs` | `~/.config/exp-1/sessions/` 下的设置和会话文件 |
| `src/catalog.rs` | 来自 Codex 缓存的模型和推理强度目录 |
| `src/titler.rs` | 为会话命名的本地 T5，随 `titler` feature 编译进代理 |
| `src/protocol.rs` | 请求/响应线类型 |
| `src/md.rs` | Markdown 和管道表格到带样式的片段 |
| `src/latex.rs` | LaTeX 数学到 Unicode |
| `src/api.rs` | HTTP 传输、SSE 解码、头部构造 |
| `src/tools.rs` | `read`、`write`、`edit`、`shell`、`ast`、`ask_question`，以及后台/输出注册表 |
| `src/app.rs` | 线程、TUI 状态、渲染、会话快照/恢复 |
| `src/main.rs` | CLI、按键处理、代理循环 |

## 限制

一个回合最多运行 200 个工具轮次；如果真的到了那里，记录会说明，而不是无声地停下。

**token 经济性。** 请求刻意保持小：`read` 分页，`grep` 限制匹配数，长命令输出保留两端，子代理的报告截断到 6 000 个字符，并且在每次请求之前，最旧的工具输出——最后两个回合之外的——会被替换为一行说明，指出丢弃了什么。模型可以为仍然需要的任何东西重新运行工具，而用户看到的记录保留完整文本。

**并行调用。** 模型的一条消息可能一次请求多个工具。连续若干只读调用（`read`、`grep`、`count`、`ast`、`fetch`、`websearch`）会同时执行；一切写入都保持顺序，所以对同一文件的写入后接读取不会被重排。

这是一个 MVP。已知的缺口：

- Vertex 只接受 gcloud 的应用默认凭据；需要签名 JWT 的服务账号密钥不会被铸造。
- 侧边对话在每个问题上都会看到整个主对话——主历史只被读取，从不被写入——所以 `/btw` 的回答拥有完整上下文，而主线程保持不动。
- 除了中止任务之外，没有流式取消。
- 命令在无沙箱下运行。

## 验证依据

协议细节已对照 `openai/codex` 源码检查，并用真实请求确认：端点和请求头、refresh token 轮换、文本流式传输、一次 `function_call` 往返，以及一个写出文件的多次迭代工具循环。

## 致谢

- **听写**基于 Cactus 的 **whistle** 语音模型与 **Needle** 引擎（`cactus-needle`，Apache-2.0）：产品页面是
  [cactuscompute.com/whistle](https://cactuscompute.com/whistle)，权重与各平台引擎在
  [Hugging Face](https://huggingface.co/Cactus-Compute/whistle)。引擎会把它们下载到
  `~/.cache/cactus-needle`；本仓库不包含其中任何内容。
- **可选的本地标题生成器**基于 Hugging Face 的 [Candle](https://github.com/huggingface/candle)，
  采用 Apache-2.0 或 MIT。
- 其余都是 `Cargo.toml` 中的 crate，各自遵循自己的许可证。

## 许可证

本项目仅依据 GNU 通用公共许可证第 3 版授权。请参阅 [LICENSE](LICENSE)。
第三方依赖和资源可能有各自的许可证；请参阅其许可声明和上方的致谢部分。
