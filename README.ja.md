[English](README.md) · [Русский](README.ru.md) · [Українська](README.uk.md) · [Čeština](README.cs.md) · [Español](README.es.md) · [Deutsch](README.de.md) · [中文](README.zh.md) · **日本語**

# exp-1 エージェント

<img src="assets/exp-1-logo.png" alt="exp-1 logo" width="96" />

ターミナル UI を備えた、Rust 製の最小限のコーディングエージェントです。**27 のプロバイダー**と対話し、各プロバイダーが実際に提供するモデル一覧を渡します。カタログは models.dev からライブで取得し、バイナリ内のテーブルをオフラインのフォールバックとします。5 つのワイヤプロトコルがそれらすべてを覆います — 十数のベンダーが共有する **OpenAI chat-completions** API、**OpenAI Responses**、**Anthropic messages**、Vertex AI ネイティブの **Gemini `generateContent`**、そして Codex バックエンド — プロバイダーのサインインは、API キー、デバイスコードのサインイン、ベンダー自身のログイン、またはマシンがすでに持っている認証情報によって行います。Codex の経路は最初に登場し、今もその 1 つです: `auth.openai.com` に対する ChatGPT OAuth で、リクエストは `https://chatgpt.com/backend-api/codex/responses` に送られます。

## できること

- Codex Responses エンドポイントからレスポンスをストリーミングします（SSE、手書きのパーサー）。
- エージェントループを回します — リクエスト → ストリーム → ツール実行 → 出力のフィードバック → 繰り返し — モデルがツールを要求しなくなるまで。
- 2 つのスレッドを保ちます。メインの会話と、それを中断してはならない質問のためのサイドスレッドです。
- 推論、ストリーミングされるアシスタントのテキスト、ツール呼び出しとその出力を 1 つのトランスクリプトに表示し、ターンごとの所要時間と時刻を添えます。

## ツール

| ツール | 引数 | 備考 |
| --- | --- | --- |
| `read` | `path`, `limit?`, `pos?` | 既定では最大 400 行。`limit` で変更でき、`pos` は 1 始まりの先頭行です。出力には行番号が付き、途中で切れた読み込みは続き方を示します。3000 行のファイルをわざと分割して読み進めます。一気に飲み込むと数万トークンかかるためです。 |
| `write` | `path`, `content` | 親ディレクトリを作成し、既存ファイルは上書きします。 |
| `edit` | `path`, `old`, `new`, `edits?` | 完全一致の置換で、全部か無しかです。失敗時は最も近い行、または一致箇所の位置を示すので、空白の不一致で読み直す必要はありません。`edits` は 1 ファイル分の複数の `{old, new}` ペアを 1 回の呼び出しで受け取ります。結果は変更内容を示します。 |
| `fetch` | `url`, `limit?`, `pos?` | 1 つのページをプレーンテキストとして取得し、ファイルと同じようにページングします。`pos`（1 始まり）から `limit` 行（既定 400）。スクリプトとスタイルは除去し、エンティティはデコードします。 |
| `count` | `path`, `refresh?` | ファイルの行数、またはディレクトリ配下のすべてのファイル全体の行数を、内容を読まずに数えます。ファイル単位で `.cache/count` にキャッシュし、サイズと mtime をキーにします。 |
| `grep` | `pattern`, `path?`, `include?`, `max?` | ファイルまたはツリーを対象に、ファイルを会話へ読み込まずに正規表現検索します。`include` はファイル名で絞り込みます（`*.rs`）。`.git`、`.cache`、`target`、`node_modules` をスキップし、シンボリックリンクは追わず、非テキストファイルは無視します。一致は `max` 件までです（既定 100）。 |
| `delete` | `path` | ファイルを削除します。ディレクトリは拒否します — ツリーを空にするのは `shell` の仕事で、コマンド全体が見えるからです。 |
| `shell` | `command`, `background?`, `limit?`, `output_id?` | フォアグラウンドはコマンドが終わるまでエージェントをブロックします。60 秒を過ぎるとコマンドはバックグラウンドへ昇格し、終了時にエージェントへ通知されます。`background: true` はすぐに id を返します。バックグラウンドのコマンドがタイマーで殺されることはありません。出力は 30 000 バイトで上限となり、それより長い場合は先頭と末尾を残して間を明示します。`output_id` は以前の実行を別の `limit` で読み直します。 |
| `ast` | `path?`, `refresh?` | プロジェクトツリーを `tree` 形式で返します。`.cache/ast` にキャッシュし、`refresh: true` で再構築します。`.git` はスキップします。 |
| `websearch` | `query`, `numResults?`, `type?`, `livecrawl?`, `contextMaxCharacters?` | ページ内容付きの Web 検索です。`mcp.exa.ai` の Exa ホスト型 MCP を使い、**API キーは不要**です。レート制限を外すには `EXA_API_KEY` を設定します。`type` は `auto`/`fast`/`deep` です。 |
| `ask_question` | `question`, `options?`, `label?`, `submit?` | ターンをブロックし、質問パネルを表示します。`↑↓` で選択肢を選び、`Tab` で質問間を移動し、`Enter` で確定し、`Esc` で閉じます。入力した文字は自由回答として扱われ、ハイライト中の選択肢より優先されます。 |

シェルコマンドはユーザーとしてサンドボックスなしで実行されます。フォアグラウンドのコマンドは 60 秒後にバックグラウンドへ昇格し、バックグラウンドのコマンドをタイマーで殺すものはなく、出力は 30 000 バイトで上限となります。それより長い結果は先頭と末尾を残し、間は省略として明示されます。

`web` ツールは Web を検索します。ページ内容を返す Exa のホスト型 MCP を先に使い、Exa の共有無料枠がレート制限を返すときは DuckDuckGo の HTML ページを使います。`EXA_API_KEY` は Exa を自分のプランに切り替えるもので、どちらの経路でも必要な設定はこれだけです。


## プラットフォーム

このエージェントは Linux、macOS、Windows 共通の 1 つのコードベースです。TUI、エージェントループ、プロバイダー、MCP、スキル、セッション、同梱の titler はすべて素の Rust で、どこでも動きます。オペレーティングシステムに触れる部分はプラットフォームごとに分岐します。

| | Linux | macOS | Windows |
| --- | --- | --- | --- |
| TUI、エージェント、プロバイダー、MCP、スキル、セッション | はい | はい | はい |
| サインインページを開くもの | `xdg-open` | `open` | `cmd /C start` |
| デーモンの生存確認と停止 | `kill` 0 / SIGTERM→SIGKILL | 同じ | `tasklist` / `taskkill /T` のち `/F` |
| コンソールから切り離されたサーバー | `process_group(0)` | 同じ | `DETACHED_PROCESS \| CREATE_NEW_PROCESS_GROUP` |
| 設定ディレクトリと状態ディレクトリ | `$XDG_*`, `~/.config` | `$XDG_*`, `~/.config` | `%APPDATA%`, `%LOCALAPPDATA%` |
| クリップボードの画像（`ctrl + v`） | `wl-paste`, `xclip` | `pngpaste` | PowerShell + `System.Windows.Forms` |
| ターン完了の通知 | `notify-send` | `osascript` | PowerShell のバルーン通知 |
| アーカイブのインストール（`exp-1 install *.zip`） | `unzip` / `tar` | 同じ | `tar`（bsdtar は zip を読みます） |
| `npx` という名前のローカル MCP サーバー | 書かれたとおり | 書かれたとおり | `npx.cmd` として再試行します |

デーモンとクリップボードの Unix 版と Windows 版の分岐は、Linux 上で一時的に有効化してコンパイルチェック済みです。`creation_flags` は Windows ターゲットなしでは確認できない唯一の Windows 専用呼び出しで、ここにあるものは実際の Windows や macOS のマシンでは動かしていません。

### Linux からのクロスビルド

Windows バイナリは Arch 上で mingw ツールチェーンを使ってビルドできます（これには一度だけ root が必要です）。

```sh
sudo pacman -S --needed mingw-w64-binutils mingw-w64-gcc
rustup target add x86_64-pc-windows-gnu
cargo build --release --target x86_64-pc-windows-gnu
```

インポートライブラリが必要とするのは `mingw-w64-binutils` の `dlltool` です。これがないとビルドは `windows-sys` で止まります。

root がなくても、`~/.local` の下に展開した llvm-mingw の tarball から同じバイナリが得られます。このターゲット向けの Rust std はリンカーに `libgcc.a` を要求しますが、llvm-mingw はそれを同梱していないため、2 つのシンボリックリンクからなるシムが相当品を用意します。`--no-default-features` は CI と同じように同梱の titler を外します。

```sh
mkdir -p ~/.local/mingw-shim
ln -sf ~/.local/llvm-mingw/lib/clang/*/lib/windows/libclang_rt.builtins-x86_64.a ~/.local/mingw-shim/libgcc.a
ln -sf ~/.local/llvm-mingw/x86_64-w64-mingw32/lib/libunwind.a ~/.local/mingw-shim/libgcc_eh.a
PATH="$HOME/.local/mingw-shim:$HOME/.local/llvm-mingw/bin:$PATH" \
CARGO_TARGET_X86_64_PC_WINDOWS_GNU_LINKER="$HOME/.local/llvm-mingw/bin/x86_64-w64-mingw32-clang" \
RUSTFLAGS="-L $HOME/.local/mingw-shim" \
cargo build --release --target x86_64-pc-windows-gnu --no-default-features
```

`~/.cargo/bin` の `cargo` と `rustup` はクロスターゲットを認識しますが、システムパッケージのものは認識しません。またこのマシンでは `~/.cargo/bin` が `PATH` にないので、フルパスで呼び出してください。

macOS バイナリにも root は不要で、`cargo zigbuild` を使えばツリー全体が `aarch64-apple-darwin` 向けにビルドされます。`zig cc` が C の依存（`aws-lc-sys` を含みます。素の `cargo check` ではできません — ホストの `cc` が `-arch arm64` を拒否します）をクロスコンパイルし、すべてのクレートがコンパイルされます。止まるのは最後のステップです。`reqwest` 0.13 の `rustls` フィーチャーが `rustls-platform-verifier` を引き込み、リンカーが `unable to find framework 'Security'` と `'CoreFoundation'` を報告します。これらは Apple SDK にしかなく、Zig は Mach-O と libSystem は問題なくリンクしますが、フレームワークは同梱していません。そのため macOS バイナリは Mac か CI から得ます（`.github/workflows/release.yml` が両方のアーキテクチャをビルドします）。以下はリンクまで到達する手順です。

```sh
mkdir -p ~/.local/zig && curl -L https://ziglang.org/download/0.14.1/zig-x86_64-linux-0.14.1.tar.xz \
  | tar -xJ -C ~/.local/zig --strip-components=1
cargo install cargo-zigbuild
rustup target add aarch64-apple-darwin
PATH="$HOME/.local/zig:$PATH" cargo zigbuild --release --target aarch64-apple-darwin
```

結果は署名なしです。Apple Silicon では署名なしのバイナリは起動時に kill されるため、Mac 自体で一度署名する必要があります。

```sh
codesign -s - --force ./exp1-agent        # ad-hoc signature
xattr -cr ./exp1-agent                    # if it arrived by download
```

## 要件

- Rust 1.85 以上（edition 2024）
- 既存の Codex CLI のログイン（その `auth.json` を再利用します）か、同じデバイスフローを自身で実行する `/login` のいずれか。
- 同梱の titler は `titler` フィーチャーの下で `src/titler.rs` からエージェントにコンパイルされます。Candle を引き込むため、初回ビルドに時間がかかり、バイナリは約 7 MB ではなく約 16 MB になります。重みが Hugging Face のキャッシュにあれば、タイトル生成はオフラインになります。

## インストール

crates.io から最新版をインストールするには Rust 1.85+ と Cargo が必要です。

```sh
cargo install exp1-agent
exp1-agent --help
```

Cargo は `exp1-agent` を `~/.cargo/bin` にインストールします。必要に応じて、このディレクトリを `PATH` に追加してください。標準ビルドにはローカル titler が含まれるため、初回インストールは Candle のコンパイルに時間がかかります。

## 使い方

```sh
cargo run --release                      # uses the current directory
exp1-agent --model gpt-6-luna --effort high # pick a model and reasoning effort
exp1-agent --cwd ~/projects/my-app          # run the shell tool somewhere else
exp1-agent --help
```

| キー | 動作 |
| --- | --- |
| `Enter` | プロンプトを送信します。すでにコマンドとして完成していれば、そのコマンドを実行します |
| `↑` `↓` | 補完の選択を移動します。または以前のプロンプトを呼び出します |
| `Tab` | ハイライト中の補完を確定します |
| `Ctrl + /` | メインスレッドとサイドスレッドを切り替えます |
| `Ctrl + Shift + A` | 権限モードを切り替えます: always ask → never ask → ask only commands |
| `Esc` | 単独では何もしません |
| `Esc` `Esc` | 実行中のターンを打ち切ります。または補完リストやピッカーを閉じます |
| `Ctrl-C` | 終了します |
| `Ctrl-U` | 入力行を消します |
| `Ctrl-Backspace` | 前の単語を削除します |
| `Ctrl-Delete` | 次の単語を削除します |
| `Ctrl-W`, `Alt-Backspace` | `Ctrl-Backspace` と同じです |
| マウスホイール | トランスクリプトをスクロールします |
| `PgUp` / `PgDn` | トランスクリプトをスクロールします |
| `←` `→` `Home` `End` | カーソルを移動します |
| `Backspace` / `Delete` | 編集します |

矢印キーはプロンプト履歴をたどります。`hi` を送信したあと `↑` でそれが入力欄に戻り、さらに押すと前へ、`↓` で空の入力欄まで進みます。スクロールはマウスホイールが担うので、矢印キーは履歴のために空いています。

`/` を入力すると、前方一致で絞り込まれる補完リストが開きます。空白のあとはコマンドの引数も補完します — `/themes <tab>` はスキームを、`/model <tab>` はモデルを、`/model gpt-6-luna <tab>` は推論エフォートを列挙します。素の `/model` や `/themes` で `Enter` を押すと、代わりに番号付きの選択リストが開きます。`/model` は 2 段階で、まずモデル、次にエフォートです。入力欄が空のとき、カーソルはプレースホルダーの前、いちばん先頭にあります。

ヒント行は状況に応じて変わります。ターンの実行中は入力がロックされます。

`Esc` は意図的に何もしません。中断するのは 2 回押しです。中断は実行中のリクエストを打ち切り、部分的なアシスタントのテキストを破棄しますが、会話履歴はそのまま残ります。

## スラッシュコマンド

ローカルで処理され、モデルには届きません。

| コマンド | 効果 |
| --- | --- |
| `/login` | プロバイダーを選びます。サインイン済みのものは切り替えるだけ、使用中のものを選ぶと再サインインします。デバイスコードのサインインはブラウザーでベンダーのページを開き、それ以外は API キーを受け取ります |
| `/scoped-models` | `/model` が提示してよいモデルにチェックを入れます。空ならすべてです |
| `/sessions` | 保存された会話を選んで開き直します |
| `/init` | 作業ディレクトリに `AGENTS.md` のスキャフォールドを書きます |
| `/model` | 2 段階のピッカー（モデル、次にエフォート）を開きます。`/model <name> [low\|medium\|high]` は直接設定します |
| `/themes` | テーマピッカーを開きます。`/themes <name>` は直接設定します |
| `/status` | モデル、エフォート、作業ディレクトリ、ツール、テーマ、プラン、認証プロバイダーを表示します |
| `/btw` | メインのターンを動かしたまま、サイドスレッドでサイドの質問に答えます |
| `/clear` | トランスクリプトと会話履歴を消します |
| `/help` | キーボードショートカットを表示します |
| `/compact` | ウィンドウが埋まるのを待たず、いま会話を要約します |
| `/undo` | 直前のターンが変更したファイルを戻します（メモリ上のみ。再起動で忘れます） |
| `/diff` | 直前のターンが変更した内容の unified diff です |
| `/fork` | この会話を新しいセッションとして続け、元はそのままにします |
| `/sessions <text>` | 保存された会話を一覧するだけでなく検索します |
| `/retitle` | 最初のリクエストと直近の会話から、セッションに再度名前を付けます |
| `/effort` | モデルを変えずに、現在のモデルの推論エフォートを選びます |
| `/mcp` | MCP サーバーと、それらが提供するツールを一覧します |

利用可能なテーマ: `default`（青）、`amber`、`jade`、`mono`。色はトゥルーカラーなので、ターミナルのテーマで置き換えることはできません。

## インターフェース

レイアウトは OpenAI Codex TUI に倣っています。枠線付きのパネルはなく、作業ディレクトリを下に添えた `>_` のマストヘッド、入力欄の上には補完ポップアップまたは番号付きの選択ピッカー、`›` プロンプトと点滅する `│` カーソルを備えた余白付きの入力帯、そのあとにモデル行とヒント行が続きます。

- 送信したプロンプトは、Codex と同じように、全幅の帯としてトランスクリプトに描き戻されます。
- **ツールの出力は既定で非表示です**。トランスクリプトに載るのは呼び出し（`read src/main.rs`）だけで、返された内容は載りません。`ctrl-o` はその呼び出しの下に出力を表示し、もう一度押すと隠します。これが、作業を追うことと読むことの違いです。
- **マウスはスクロールだけのものではありません。** ホイールはスクロールし、ドラッグは選択してコピーし、クリックは着地した先で働きます。ピッカーや補完ポップアップの行ならその行への Enter と同じように動き、フッターの `ctrl + / for side` ヒントはスレッドを切り替え、メッセージ内のリンクはブラウザーで開きます。各フレームは自身が描いた領域を記録するため、クリックはレイアウトの推測ではなく直前のフレームに対するヒットテストです。ホバーのハイライトは意図的に実装していません。マウスの移動ごとに再描画を要しますし、ポインターの位置はターミナル自身のカーソルがすでに示しているからです。
- サブエージェントは現在のステップを、ステップごとの行ではなく**更新される 1 行**（`subagent → grep tokenizer`）で報告し、それを起動した呼び出しは `task: <description>` として表示されます。
- **自分のメッセージをクリックすると、それをどうするかを尋ねます**。すぐに実行はしません。`copy`（テキストはクリップボードへ）と `revert` のある小さなメニューです。revert はそのメッセージより後のすべてを捨てます — 会話、その地点より下でモデルが伝えられた内容、そしてそれ以降のターンが行ったすべてのファイル変更です（後者は undo ログが戻します）。メッセージ自体はトランスクリプトから消え、そのテキストはプロンプトに戻るので、編集して再送できます。何かを捨てる前にテキストを確認するため、誤ったメッセージへのクリックは何もしません。
- トランスクリプトは最新のメッセージに張り付きます。マウスホイールまたは `PageUp`/`PageDown` で履歴を上へスクロールし、`ctrl-g` で最新のメッセージへ、`ctrl-t` で最古へ飛びます。上へスクロールした表示は、新しい行が届いても引き戻されることなくその場に留まります。
- マウスでドラッグすると画面上のどこでも — マストヘッドも含めて — テキストを選択でき、離すとクリップボードに入ります。まず OSC 52（ssh 越しでも機能します）、次に存在すれば `wl-copy`、`xclip`、`xsel` です。`copied N chars` のノートがヒント行で確認を示します。選択はテキスト背後の静かな色合いで、各行の最後の文字で止まります。ターミナル自身の選択と同じ挙動です。アプリがスクロールのためにマウスを握っているのでターミナルはドラッグを認識せず、これが唯一のコピー手段です。
- 添付した画像は、プロンプトの上に塗りつぶしの `Image 1` チップとして表示され、その横に `ctrl-v replaces it` が付きます。送信されようとしているものが入力帯自体で見えるわけです。
- ターンの実行中、アクティビティ行は**それが属するプロンプトのすぐ下**に留まり、行うことごとに自前のラベルを持ちます。リクエストが往復している間は `⠋ thinking 4s`、モデルがまだツール呼び出しの引数を書いている間は `⠋ preparing write… (12 KB) 8s`、ツールが実行されている間は `⠋ running 4s` です。各ラベルは自前の時計を持つので、秒数は今起きていることについてであり、ターン全体を数えることはありません。ターンの合計は最後に `Worked for` が報告します。ボトムバーは素の `GPT-6-Luna high · /path · <thread> · Main [default]` ラベルのままで、`esc esc interrupts` はヒント行にあります。
- ターンが終わると、`Worked for 5s · 12:08 PM` がトランスクリプトに追記されます。
- モデル行は `GPT-6-Luna high · /path · <thread name> · Main [default]`、サイドスレッドでは `… · Agent` と表示されます。スレッド名は最初のプロンプトから取られます。
- ヒント行は右側にスレッド切り替えを載せ、サイドスレッドがアクティブな間は赤で表示します。
- 空の画面は静かなままです。完全なコマンド一覧が出るのは会話を一度も保存していないマシンだけで、その後は `/` と `/help` を指す 1 行になります。
- ターン中も入力は生きているので、`/btw` の質問を待たずにできます。
- `/btw` または `ctrl + /` は、メインの会話全体をコンテキストとして運びながら、それに何も加えないサイドチャットで答えます。
- `shift + tab` はビルドモードとプランモードを切り替えます。フッターはオンの間 `plan mode` を表示します。
- 15 秒を超えて走ったターンは、終了時にターミナルのベル（デスクトップに備わっていれば `notify-send`）を鳴らすので、長いジョブを見張り続ける必要はありません。
- `shift + enter` は送信せずに改行を書き込むので、プロンプトは複数行にできます。入力帯はそれらを収めようと大きくなります。
- `ctrl + v` はクリップボードから画像を取り（Wayland は `wl-paste`、X11 は `xclip`）、次のプロンプトに添付します。送信されるまでヒント行に `▣ image attached` を表示します。画像はデータ URL として運ばれるため、Codex、OpenAI、Anthropic、Gemini の各バックエンドがすべて引き継ぎます。
- `ask_question` は入力欄の上にパネルを出します。Question ツールを写したもので、左端に沿ってアクセントバー、`Questions` のタイトル、複数の質問をまとめて尋ねるときのタブ、ハイライトされたものが際立つ番号付き選択肢、`Type your own answer` の枠、そして下部に `⇥tab ↑↓ select enter confirm esc dismiss` があります。

色はトゥルーカラーなので、ターミナルのテーマで置き換えることはできません。モデル、エフォート、テーマは実行中に `/model` と `/themes` で切り替わります。

Codex との違いは意図的です。Codex には `/permissions` と `/review` があり、サンドボックスポリシーと差分レビュアーを必要としますが、このビルドには代わりに `/clear` と `/help` があります。Codex のフッターはエージェントとショートカットのオーバーレイを宣伝していますが、この MVP にはありません。

## 認証

`/login` はプロバイダーを一覧表示し、それぞれはベンダーが期待する方法でサインインします:

- **API キー**: 大半のプロバイダー向け。`Authorization: Bearer`（Anthropic API では `x-api-key`）として送信され、プロバイダーごとに `~/.config/exp-1/credentials.json` へモード `0600` で保存されます。1 つのキーが複数のプロバイダーを覆えます — `opencode-go` と `opencode-zen` は 2 つのベース URL の背後の 1 アカウントで、`providers::key_group` がグループを名付けます。
- **デバイスコードのサインイン**: `openai (auth)`、`github copilot`、`meta (auth)` 向け。ブラウザーでベンダーのサインインページが開き、URL とコードがフォールバックとして表示されます。
- **ベンダー自身のログインによるトークン**: `anthropic (auth)` 用。公式クライアントのアイデンティティヘッダー付きで送信されます。
- **マシンがすでに持っている認証情報**: `google vertex` 用。`gcloud auth application-default login` が設定したものを使い、`GOOGLE_CLOUD_PROJECT` と `GOOGLE_CLOUD_LOCATION` がプロジェクトとリージョンを名指しします。貼り付けるものはありません。

Codex の経路はさらに自身のファイルを保ちます。初回実行時、エージェントは `$CODEX_HOME/auth.json`（既定は `~/.codex/auth.json`）を `~/.exp1-agent/auth.json` にコピーし、そのコピーで動作します。**実際の Codex の認証情報が書き換えられることはありません。** アクセストークンの有効期限まで 5 分を切ると、エージェントはそれをリフレッシュします:

```
POST https://auth.openai.com/oauth/token
grant_type=refresh_token
client_id=app_EMoamEEZ73f0CkXaXp7hrann
```

レスポンスはリフレッシュトークンをローテートするので、新しいものがエージェント自身のコピーへ書き戻されます。保存されるファイルはモード `0600` でアトミックに書かれます。

## モデルとエフォート

モデル一覧も推論エフォートもハードコードされていません。どちらも Codex CLI 自身のカタログ `$CODEX_HOME/models_cache.json`（既定は `~/.codex/models_cache.json`）から読み、各エントリの `slug`、`display_name`、`description`、`visibility`、`default_reasoning_level`、`supported_reasoning_levels` を使います。起動時の既定は、そのカタログで最初に載っているモデルです。

**ほかのすべてのプロバイダーは一覧をライブで、models.dev から取得します。** カタログは最大 12 時間に 1 度 `~/.cache/exp-1/models-dev.json` へ取得され、メモリに保持され、3 つのために読まれます。プロバイダーが提供するモデル id、各モデルが話すプロトコル（models.dev はプロバイダーが設定されている AI SDK パッケージとして記録します — ゲートウェイ背後の Claude モデルなら `@ai-sdk/anthropic`、Grok なら `@ai-sdk/openai`）、そして自動コンパクションのしきい値の計算元になるコンテキストウィンドウです。カタログにないプロバイダーは `src/providers.rs` のテーブルを保ち、オフラインのマシンも同様なので、アプリが取得に依存することはありません。

テーブルがまったく見逃したモデルにもプロトコルは与えられます。決めるのはベンダー接頭辞です（`claude-*`、`minimax-*`、`qwen-*` は Anthropic、`grok-*`、`gpt-*`、`muse-*` は Responses、`gemini-*` は Gemini、それ以外はすべて chat）。あるプロトコルしか話さないプロバイダーは、その他すべてに答えます。

**ピッカーはサインイン済みのプロバイダーをすべて表示します**。サインインごとに 1 棚ではなく 1 つの棚で、行には `via <provider>` が付くので同じモデル名の 2 つの帽子を見分けられ、使用中のプロバイダーが先頭に来ます。別のプロバイダーのモデルを選ぶと、モデルを設定する前にそちらへ切り替わります。これほど長い棚は入力で検索します。どのピッカーも入力に応じて絞り込まれ、バックスペースで一致が広がり、Escape は閉じる前に広げます。`/scoped-models` のスコープは同じ棚を絞り込み、1 つのプロバイダーのモデルを名指しします（`opencode-go/grok-4.7`）。同じ名前でもプロバイダーが違えば 2 つのエントリです。裸の名前を持つ古いスコープは依然として「この名前、どのプロバイダーからでも」を意味し、そのピッカーでの `ctrl-a` はスコープを消し、再びすべてを提示します。`/model grok-4.7` は複数のプロバイダーがその名前を提供するとき使用中のプロバイダーを取り、`/model opencode-go/grok-4.7` は 1 つを正確に指定します。

エフォートはモデルごとに実際に異なるため、推測してもうまくいきません。

| モデル | エフォート |
| --- | --- |
| `gpt-5.6-terra` | low, medium, high, xhigh, max, **ultra** |
| `gpt-6-luna` | low, medium, high, xhigh, max |
| `gpt-5.5` | low, medium, high, xhigh |

エフォートピッカー、`/model <name> <tab>` の補完、検証はすべて、アクティブなモデルに従います。モデルを切り替えると、新しいモデルが受け入れる場合はエフォートが引き継がれ、そうでなければそのモデルの `default_reasoning_level` に戻ります。モデルが持たないエフォートを要求する事態には決してなりません。

```
/model gpt-5.6-terra ultra   →  GPT-5.6-Terra ultra
/model gpt-5.5               →  GPT-5.5 medium          (ultra is not a gpt-5.5 level)
/model gpt-5.5 max           →  'max' is not a GPT-5.5 level;
                                this model takes low, medium, high, xhigh
```

ピッカーに現れるのは `visibility: "list"` のモデルだけです。隠れたモデルも名前で到達できます。カタログファイルがない場合、アプリは保守的な組み込みリストに戻って使える状態を保ち、ピッカーでその旨を述べます。

実際に応答するのは、サインイン中の ChatGPT アカウントが権利を持つモデルだけです — 残りはバックエンドが説明付きのエラーで拒否します。

## リクエストの形

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

消費するイベント: `response.output_text.delta`、`response.reasoning_summary_text.delta`、`response.output_item.done`、`response.completed`、`response.failed`、`response.incomplete`、`error` です。それ以外は無視します。

## レンダリング

アシスタントの回答はプロセス内でリッチテキストとしてレンダリングされます。外部レンダラーはなく、`pandoc` も TeX バイナリも画像のラスタライズもありません — すべてが TUI の中で起きるため、ほとんどコストがかかりません。

- **Markdown**: 見出し、太字、斜体、インラインコード、ガターと言語タグ付きのフェンス付きコードブロック、箇条書きと番号付きリスト、引用、水平線、リンク、罫線でレイアウトし列ごとに整列する GitHub 風のパイプテーブル、そしてインラインの `$…$` 数式。
- **LaTeX 数式**: 記号と関係、ギリシャ文字、`\frac`、`\sqrt`、上付き・下付き文字、`\mathbb` と `\mathcal`、合成アクセント（`\bar`、`\hat`、`\vec` など）、立体の演算子名（`\log`、`\sin`、`\lim` など）、そしてディスプレイの `$$…$$` ブロック。

コンバーターが認識しないものは、波括弧も含めてそのまま出力されるので、なじみのない入力もどろどろにならず読めるままです。これはコンバーターであり TeX エンジンではありません。マクロ、`tikz` のような環境、複数行のレイアウトは解釈しません。

Markdown はメッセージが届いたときに**一度だけ**、テーマ非依存のランへパースされます。描画はそれらのランを色へ写すだけです。したがってテーマの切り替えで再パースは何も起きません。

ビルド済みバイナリでの実測: カーソルが点滅しているアイドル時で**1 コアの約 1 %**、ホイールを速くスクロールしている間で約 3 % です。

## サブエージェントと長いコマンド

**サブエージェント。** `task` ツールは、独自のコンテキストと同じツールを持つ 2 つ目のエージェントを実行し、その報告だけを返します — 呼び出し側のコンテキストを溢れさせるような作業に有用です。報告は呼び出し側のコンテキストに載るため 6 000 文字で切られ、トランスクリプトはその進捗を、ステップごとに 1 行ではなく、各ステップで置き換えられる 1 行で示します。ネストしたサブエージェントは深さが残っている間だけ提供され、再帰が暴走しないよう 2 レベルまでに制限されます。トランスクリプトはサブエージェントの各ステップを表示します。

**遅いコマンドは殺さず、昇格させます。** フォアグラウンドのコマンドが **60 秒**を過ぎても走っていれば、ターンをブロックする代わりにバックグラウンドへ移され、エージェントはそのように伝えられます。

```
'sleep 75' is still running after 60s, so it was moved to the background as 'bg-1'.
Carry on; you will be told when it finishes, or poll it with {"output_id": "bg-1"}.
```

**完了は中断を起こします。** バックグラウンドのコマンドが終わると、実行中のターンは打ち切られ、エージェントに出力が渡されます。長い作業の陰で結果が黙って失われることはありません。

```
• interrupted: a background command finished
```

## ディクテーション

右下隅のマイクボタン（`ctrl-r` も同じです）はプロンプトへ音声入力します。押して、話して、もう一度押します。認識器がストリーミングするため、話している間にもテキストが届きます。ボタンは聞いている間タイマーを表示します。

認識器は **whistle** です — Cactus のオンデバイスの音声エンジンで、`whistle --mic` として起動します。このモードがストリーミングを可能にします。pipewire の `pw-record` から生の 16 kHz モノラルを 1 秒ずつ録音し、毎秒をストリーミングデコーダーに通し、更新ごとに 1 行の JSON を出力します（`text` が確定したもの、`pending` がまだ処理中のものです）。stdin への改行でストリームが終わり、残りがフラッシュされます。コマンド、言語、バイアス語、音声の入力元は環境から来ます。

レコーダーは各プラットフォーム自身のものです。Linux では `pw-record`（pipewire）、macOS では avfoundation 付きの `ffmpeg`、Windows では dshow 付きの `ffmpeg` です（`WHISTLE_SOURCE` が入力デバイスを選び、`WHISTLE_RECORDER` はコマンドごと置き換えます）。アプリ自身は `EXP1_DICTATE --mic` を起動してその JSON 行を読むだけなので、同じようにストリーミングする認識器ならどれでも動きます。認識器がなければ、マイクボタンを押すとフッターでその旨が告げられ、ほかには何も起きません。

```sh
EXP1_DICTATE=/path/to/whistle     # default: whistle on PATH
EXP1_DICTATE_LANG=en              # default: whatever it detects
EXP1_DICTATE_KEYWORDS=country,Americans
WHISTLE_SOURCE=…                  # default: the system's own input
```

音声入力はレコーダー同様 Linux 専用です。エージェントの残りの部分はそうではありません。

このモードはこのコピーの whistle にローカルなものです。ロジックはそのライブラリに `Dictation` 型（`start`、`next`、`finish`）としてあり、その `main.rs` はそれを回す薄いループです。whistle はここに独自のリポジトリを持たないため、作者から新しいコピーを取ってきた場合はこの 2 つのファイルを当て直すか、`EXP1_DICTATE` を同じプロトコルを話す何かに向ける必要があります。`--mic` を知らない認識器も無視されません。フッターに、それ自身の言葉でその旨を述べます。

## プロバイダー

`/login` はプロバイダーのピッカーを開き、`/scoped-models` はそのうちどのモデルを `/model` が提示するかを決めます（空ならすべてで、スコープは `settings.json` に保存されます）。ピッカーは素のアルファベット順のリストで、カーソルは使用中のプロバイダーで開くので、位置で探す必要はありません。すでにサインイン済みのプロバイダーを選ぶとそこへ切り替えてそう告げ、使用中のものを選ぶ（または `/login <id>` を実行する）と意図的に再サインインします。デバイスコードのサインインはブラウザーでベンダーのページを開き、フォールバックとして URL とコードを表示します。

**1 つのキーが複数のプロバイダーを覆えます。** `opencode-go` と `opencode-zen` は 2 つのベース URL の背後の 1 アカウントなので、どちらかにサインインすると両方のキーが保存されます（`providers::key_group` がグループを名付けます）。opencode のゲートウェイはセッションでもルーティングします。すべてのリクエストは `x-opencode-session`、`x-opencode-project`、`x-opencode-client` を運び、これらがないとゲートウェイはあっさり拒否します（"Request is missing x-opencode-session"）。

ここに実装されたすべてを 5 つのワイヤプロトコルが覆います。十数のベンダーが共有する **OpenAI chat-completions** API、**OpenAI Responses**、**Anthropic messages**、Vertex AI ネイティブの **Gemini `generateContent`**、そして `openai (auth)` が対話する **Codex バックエンド**です。すべて SSE でストリーミングし、すべて Codex クライアントが生成するのと同じイベントストリームへデコードされるため、エージェントループはどこと話しているかを知りません。

キーはプロバイダーごとに `~/.config/exp-1/credentials.json` に、モード `0600` で保存されます。`openai (auth)` は代わりに Codex 形式の `auth.json` を保持し、`google vertex` は `gcloud auth application-default login` がすでにサインインしたものを使い（`GOOGLE_CLOUD_PROJECT` と `GOOGLE_CLOUD_LOCATION` がプロジェクトとリージョンを名指しします）、デバイスコードのプロバイダーは受け取ったトークンを同じ認証情報ファイルに置きます。

| プロバイダー | ベース URL | プロトコル |
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

すべてのエントリは実装済みです。各モデルは応答するプロトコルを名指しするので、`opencode-go` のようなゲートウェイは 3 つを同時に提供できます。一覧とベース URL は当て推量ではなく [models.dev](https://models.dev)（opencode 自身が読む公開レジストリ）から来ています。

上のすべてのベース URL は、わざと無効なキーでプローブしました。拒否されることが URL と認証ヘッダーが正しい証拠です。27 すべてが `400`、`401`、`403` のいずれかで答えました — `fireworks-ai` を除いて。その `404` は `chatcmpl-` のリクエスト id を伴う独自の `"Model not found"` なので、そのリクエストも推論ルートに到達しています。`google vertex` はプロジェクトスコープのパスで `401` を返します。

## モード

`shift + tab` はセッションを 2 つのモード間で切り替え、フッターがどちらがアクティブかを示します。

- **build** — ここに書かれているとおりのすべてです。
- **plan** — ファイルを変更するツール（`write`、`edit`、`delete`）は拒否され、*かつ*ツール一覧からも外されるので、モデルは編集せずに調査して計画を提案します。`shell` は読み取り専用に留まることが期待されます。文言は `prompts/plan.md` にあり、ほかと同様に編集できます。

## パーミッション

`~/.config/exp-1/permissions.json` は例外のリストです。どのルールにも一致しない呼び出しは実行されます。最初の一致が決めます。

```json
{
  "rules": [
    { "action": "shell", "resource": "rm -rf *", "effect": "deny" },
    { "action": "write", "resource": "/etc/*", "effect": "ask" },
    { "action": "fetch", "resource": "*", "effect": "allow" }
  ]
}
```

`action` はツール名か `*`、`resource` は呼び出しが触れるものです — ファイルツールならパス、`shell` ならコマンドライン、ネット系ツールなら URL かクエリ — そして `*` と `?` がワイルドカードとして働きます。`effect` は `allow`、`ask`、`deny` です。`ask` は質問パネルを出して、答えるまで呼び出しを保留します（2 分で、その後は呼び出しが拒否されます）。ルールはサブエージェントにも適用されます。

`ctrl + shift + a` は権限モードを切り替えます。**always ask**（新規インストールの既定）は `write`、`edit`、`delete`、`shell` に確認を出し、**never ask** は確認なしで実行し、**ask only commands** は `shell` のときだけ確認します。読み取り専用のツールは決して確認せず、`effect: deny` のルールはどのモードでも拒否します — モードが deny を緩めることはありません。

## プロジェクトの指示

`AGENTS.md` は作業ディレクトリとその上のすべての親から、最も外側を先に読み、システムプロンプトに追記されます — これによりリポジトリ自身のルールが会話に貼り付けられることなくモデルに届きます。`/init` が開始用のファイルを書きます。

## プロバイダーの失敗

サーバー側またはトランスポートの失敗は 2 秒後に一度だけ再試行します。拒否されたリクエスト（4xx）は再試行しません。プライマリモデルが失敗し続けると、`settings.json` の `fallback_model` が同じプロバイダーのモデルを名指しし、ターンを諦める前に一度試します。切り替えはトランスクリプトに記されます。

## スキル

スキルとは `SKILL.md` を中に持つディレクトリで、Claude Code と opencode が使う形です。インストールするとコマンドになります。

```sh
exp-1 install ~/skills/pdf-tools          # a directory with SKILL.md
exp-1 install notes.md                    # a single markdown file
exp-1 install tools.zip                   # an archive (unzip or tar)
exp-1 install https://example.com/skill.md
```

ファイルは `~/.config/exp-1/skills/<name>/` に置かれ、名前はフロントマターに `name:` フィールドがあればそれ、なければファイル名かディレクトリ名です。インストールされたスキルは補完ポップアップと空状態のヘルプで組み込みコマンドの隣に現れ、`/<name>` がそれを実行します。サーバーはスキルの指示を読み、それをタスクとして会話に書き込み、モデルにそれに従わせます。ピッカーが表示するのはフロントマターの `description:` です。

## MCP サーバー

[MCP](https://modelcontextprotocol.io) サーバーのツールは、組み込みのものと並んでモデルに提供されます。一覧は `~/.config/exp-1/mcp.json` にあり、opencode が使う形とまったく同じなので、エントリを両者の間でコピーできます。

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

- `local` サーバーは起動され、stdio 越しに会話します。`remote` は HTTP 越しに到達し、JSON または SSE の応答を取ります。`"disabled": true` はサーバーを削除せずスキップし、`"headers"` はリモートサーバーが必要とするものを追加します。
- ツールは `<server>_<tool>` としてモデルに届き、両方の半分は opencode がサニタイズするのと同じ方法でサニタイズされます（`context7_resolve-library-id`）。2 つのサーバーが衝突することは決してありません。
- サーバーは起動時に**バックグラウンドで**接続されます。遅いものや死んだものがエージェントを引き留めることはなく、捨てられるのではなく報告されます — `/mcp` はすべてのサーバーを、そのツール数か失敗したエラーとともに一覧します。

## バックグラウンドサーバー

opencode のデーモンに倣っています。**通常の `exp1-agent` の起動はサーバーを立ち上げてそこに接続します**。opencode の既定コマンドが `daemon.transport()` を取って UI に渡すのとまったく同じです。

その帰結こそが要点です。UI を閉じてもサーバーは動いたままなので、次の起動は温かいプロセスを再利用します。サーバーが再利用されるのは、**バージョンとビルドの両方**が同じときだけです。`server.json` は実行ファイルのサイズと更新時刻を記録するので、サーバー起動後に再ビルドされたクライアントは、サーバーがもはや話さないプロトコルで会話するのではなく、それを置き換えます。このマシンでの実測:

| | コールド起動 | 次の起動 |
| --- | --- | --- |
| UI の起動 → 最初のフレーム | 189 ms | **84 ms** |
| UI RSS | 14.4 MB | 16.0 MB |
| サーバー RSS | 15.2 MB | 20.5 MB（ターン後） |

5 つのセッションを同時に開いた状態でもう一度実測:

| | サーバー 1 つ + UI 5 つ | スタンドアロンプロセス 5 つ |
| --- | --- | --- |
| アイドル | 97 MB (15 + 5 × 16) | 183 MB (5 × 37) |
| 同梱の titler あり | 143 MB | 423 MB (5 × 85) |

サーバーは読み込むものを遅延します。Codex クライアントは最初のターンで現れ（+5 MB）、同梱の titler はタイトルが実際にローカルで生成されたときにだけ現れます（+48 MB。`auto` がモデルを優先するのはこのためです）。その遅延が要点の大半です — 48 MB の重みは、プロセスごとに一度ではなく、マシン上のセッション全体で一度だけ払われます。

高価でマシンごとに一度の状態 — ツールレジストリと約 49 MB の titler モデル — は 1 つのサーバーに住み、Codex クライアントは Codex プロトコルのリクエストが初めてそれを必要としたときに構築されるので、ほかのプロバイダーにサインインしたマシンは Codex の認証情報なしでサーバーを起動します。各 UI は薄いクライアントなので、N 個の並行セッションのコストは `N x 61 MB` ではなく `1 x 63 MB + N x 15 MB` で、コールドスタートを払うのは最初の起動だけです。

```sh
exp1-agent serve                 # run the server in the foreground
exp1-agent service start         # spawn it detached (survives the launcher)
exp1-agent service status        # pid, url, version, health
exp1-agent service stop          # SIGTERM, then SIGKILL after a grace period
exp1-agent service restart
exp1-agent service password      # print, or set with an argument
```

- **登録**: `~/.local/state/exp-1/server.json` が `{id, version, url, pid}` を保持します。クライアントはそれを読み、2 秒のタイムアウトで `/health` を確認し、バージョンも一致するときだけサーバーを再利用します — 古いものやよそのサーバーは先に停止されます。
- **パスワード**: `~/.local/state/exp-1/password`、32 の乱数バイト、モード `0600`。再起動をまたいで保たれるので、新しく起動したクライアントは何も教えられずに再接続します。
- **切り離した起動**: 標準ストリームを閉じた自前のプロセスグループで `exp1-agent serve --register` を実行するので、どのクライアントが起動したよりも長く生き残ります。

### HTTP API

すべてのルートは `Authorization: Bearer <password>` を要求します。

| メソッド | パス | 目的 |
| --- | --- | --- |
| `GET` | `/health` | 生存確認とバージョン |
| `GET` | `/sessions` | 稼働中のセッションを一覧します |
| `POST` | `/sessions` | 作成します。`x-exp1-directory` が作業ディレクトリを設定します |
| `GET` | `/sessions/{id}` | 完全なスナップショット |
| `POST` | `/sessions/{id}/prompt` | ターンを開始します |
| `GET` | `/sessions/{id}/events` | SSE: スナップショット、デルタ、ツール、使用量、タイトル、done、idle |
| `POST` | `/sessions/{id}/answer` | 保留中の `ask_question` に答えます |
| `POST` | `/sessions/{id}/interrupt` | 実行中のターンを打ち切ります |
| `GET` | `/models` | カタログ。各モデルの実際のエフォート付き。`?all=1` はスコープを無視します。スコープを設定するピッカーはスコープ外のモデルを必要とするためです |

セッションは自身の作業ディレクトリを運ぶので、1 つのサーバーが複数のプロジェクトにサービスします。ディレクトリはサーバーの起動時に固定されるのではなく、`POST /sessions` とともに届きます。

## 自動コンパクション

長い会話はコンテキストウィンドウを溢れる前に要約されます。毎ターンのあと、サーバーはリクエストの `input_tokens` を Codex カタログのモデルの `context_window` と比較し、会話がその **95 %** に達したらコンパクションを行います。

```
Context at 7378 of 258400 tokens (95% threshold) — compacting
Conversation compacted; continuing from the summary
```

コンパクションは会話を、要約プロンプトと opencode が使う構造化テンプレート — Objective、Important Details、Work State（Completed / Active / Blocked）、Next Move、Relevant Files — とともにモデルへ送ります。ツール出力は要約者が溺れないよう 2000 文字に切り詰められます。

書き換えられた会話は**要約のあとに、そのまま保たれた直近 5 つのユーザーターンが続く**もので、それぞれにエージェントのツール呼び出しとその結果が付きます。手元のスレッドを失わないためです。

```
[user]      Summary of everything so far: ## Objective …
[user]      Turn two: run shell ["seq","1","900"] and reply OK.
[call]      shell
[result]    exit_code: 0 1 2 3 …
[assistant] OK
… four more turns …
```

コンパクションの前に述べられた事実は、そのあとも知られています。それらのターンの 1 つがそれ自体巨大なら、結果が再びウィンドウに収まるまで古い末尾のエントリが捨てられます。1 つの巨大なツール出力が会話を永遠に再コンパクションさせることはありません。

別の割合が欲しければ `EXP1_COMPACT_PERCENT` です。272 000 トークンのウィンドウを埋めずにこの経路を試す方法でもあります。

会話はディスク上に最大 5 000 のトランスクリプトメッセージを保ちます。モデル自身のコンテキストは間引きとコンパクションで別に制限されるので、長いセッションもトークンを費やさずに読めるままです。

## 永続化

設定は `~/.config/exp-1/`（または `$XDG_CONFIG_HOME/exp-1`）にあります。会話自体は `~/.cache/exp-1/sessions/`（または `$XDG_CACHE_HOME/exp-1/sessions`）にあります。トランスクリプトはキャッシュであり、失っても失うのはコンテキストで、セットアップではないからです。デーモン自身のファイル — `server.json` と `password` — は `~/.local/state/exp-1/` にあります。

| ファイル | 内容 |
| --- | --- |
| `config/settings.json` | モデル、推論エフォート、テーマ、titler モード、フォールバックモデル |
| `config/credentials.json` | プロバイダーごとに 1 つの API キー、モード `0600` |
| `config/prompts/*.md` | システムプロンプト。編集可能: `system`、`subagent`、`compaction`、`title`、`plan` |
| `config/permissions.json` | allow/ask/deny のルール |
| `config/mcp.json` | MCP サーバー |
| `config/skills/` | インストールされたスキル（1 つにつき 1 ディレクトリ） |
| `cache/sessions/<id>.json` | 1 つの会話: タイトル、両方のトランスクリプト、モデル側の履歴、プロンプト履歴 |

会話は最新の **250 メッセージ**をディスクに保ち、キャッシュ全体はサーバー起動時に **250 MB** へ — 古いファイルから順に — 切り詰められます。すべてが `config/exp-1/sessions/` にあった古いレイアウトは、変更後の最初の起動で移行されます。

設定はアプリの終了時に書かれ、セッションは完了したターンごとに書かれるので、クラッシュで失われるのはせいぜい実行中のターンだけです。

同梱の titler はオプションのフィーチャーです。`cargo build --release --no-default-features` はそれを外し、Candle 依存全体も一緒に外します。その場合セッションのタイトルはモデルからのみ来ます。titler が Candle を引き込むため、CI はそのようにビルドします。crate はツリー内にありますが、リリースビルドがその分を負担する必要はありません。

プロンプトは定数ではなくファイルです。`~/.config/exp-1/prompts/system.md`（エージェント自身の指示）、`subagent.md`、`compaction.md`、`title.md` は初回起動時に書き出され、リクエストごとに読まれるので、編集は再ビルドなしで次のターンに効きます。起動時のモデルとエフォートの解決順は **コマンドライン → 保存された設定 → カタログで最初に載っているモデル** です。

**どの実行も空のセッションで始まります。** 自動で再開されるものはありません — `/sessions` が保存された会話のピッカーを、新しい順に、それぞれ生成されたタイトル、id、最終使用時刻とともに開きます。一覧はディレクトリでスコープされます。セッションが見えるのは、それが開始された場所とその下のどこからでも、`$HOME` で開始されたもの（「グローバル」なもの）はどこからでもです。`$HOME` 自体はグローバルなものだけを一覧するので、すべてのプロジェクトのカタログにはなりません。1 つ選ぶと、離れる会話を保存して、選んだものを読み込みます。`/clear` は現在の会話を捨てて新しいものを始め、設定は保ちます。

### セッションのタイトル

タイトルは同梱の **titler** から来ます — ローカルの T5（7.6M パラメーター、エンコーダー 3 層とデコーダー 3 層）で、このプロセス内で Candle を通して動き、`src/titler.rs` からエージェントにコンパイルされます。重み（`Cyronius/titler`）は Hugging Face のキャッシュから読まれ、プロセスごとに最大 1 回読み込まれるので、ディスクにあればタイトル生成はローカルで、即時で、無料です。

生成は最初のターンが終わった瞬間に、UI スレッドではなくブロッキングスレッドで走ります。親を持つセッション — サイドチャット — は、それが従う会話に属するため、まったく名前を付けられません。ライブラリの多言語経路（生の経路ではなく）を使うので、言語が検出され、英語以外のプロンプトはまず英語へ翻訳されます。セッションは一度だけ名付けられ、タイトルが届くまではピッカーがプロンプトから取ったプレビューを表示します。

**誰がセッションに名前を付けるかは設定です**。`settings.json` の `titler` です。

| 値 | 挙動 |
| --- | --- |
| `auto`（既定） | まず Codex モデル、フォールバックとして同梱の titler |
| `local` | 同梱の titler のみ — API 消費なし、タイトルは弱め |
| `remote` | Codex モデルのみ |

タイトルはセッション自身のモデルではなく、カタログの安価な `luna` 層に求めます — opencode がプロバイダーにその小さなモデルを求めるのと同じ考えです — そしてタイトルにかかったトークンは、ほかの呼び出しと同様にセッションの使用量に加算されます。

`/retitle` はセッションに再度名前を付けます。今度は最初のリクエストと直近の会話（8 000 文字まで）からで、これは opencode がセッションを何もない名前に改名したときにタイトルを再生成する方法です。プロンプト自体は `prompts/title.md` にあり、完全な opencode 風の文書です。タスク、ルール（ユーザーと同じ言語、50 文字以下、正確な用語と数字を保つ、決して質問に答えない）、そして 10 の実例です。

`auto` が Codex モデルを先頭に置くのは意図的です。7.6M の titler はそのサイズに正直です。同じロシア語のプロンプトで、«сколько будет два плюс два» には `Understanding Two Plus Terms` を、表を求めるリクエストには `Request for Sign` を生成しました。Google の無料翻訳器が «табличку» を "sign" と訳し、小さなモデルがその誤訳にタイトルを付けたためです。Codex モデルは同じプロンプトに `Сколько будет два плюс два?` と `Формула корней квадратного уравнения в LaTeX` と名付けます。その LLM 経路が `prompts/title.md` の役割です — 同梱のモデルは最初のメッセージをそのまま取り、このファイルをまったく無視します。

## レイアウト

| ファイル | 役割 |
| --- | --- |
| `src/auth.rs` | 認証情報の読み込み、JWT 有効期限の確認、リフレッシュ、アトミックな保存 |
| `src/store.rs` | `~/.config/exp-1/sessions/` 下の設定とセッションファイル |
| `src/catalog.rs` | Codex キャッシュからのモデルと推論エフォートのカタログ |
| `src/titler.rs` | セッションに名前を付けるローカル T5（`titler` フィーチャーでコンパイル） |
| `src/protocol.rs` | リクエスト/レスポンスのワイヤ型 |
| `src/md.rs` | Markdown とパイプテーブルからスタイル付きランへ |
| `src/latex.rs` | LaTeX 数式から Unicode へ |
| `src/api.rs` | HTTP トランスポート、SSE デコード、ヘッダー構築 |
| `src/tools.rs` | `read`、`write`、`edit`、`shell`、`ast`、`ask_question`、およびバックグラウンド/出力レジストリ |
| `src/app.rs` | スレッド、TUI 状態、レンダリング、セッションのスナップショット/復元 |
| `src/main.rs` | CLI、キー処理、エージェントループ |

## 制限

1 ターンは最大 200 のツールラウンドを回ります。そこに達した場合は、黙って止まるのではなくトランスクリプトがそう告げます。

**トークンの節約。** リクエストは意図的に小さく保たれます。`read` はページングし、`grep` は一致数を制限し、長いコマンド出力は両端を残し、サブエージェントの報告は 6 000 文字で切られ、各リクエストの前に — 直近 2 ターンを除いて — 最も古いツール出力が、何を捨てたかを述べる 1 行のノートに置き換えられます。モデルはまだ必要なもののためにツールを再実行できますし、ユーザーが見るトランスクリプトは全文を保ちます。

**並列呼び出し。** モデルのメッセージは一度に複数のツールを求めることがあります。読み取り専用の呼び出しの連続（`read`、`grep`、`count`、`ast`、`fetch`、`websearch`）は同時に実行されます。書き込むものはすべて順序を保つので、同じファイルへの書き込みと読み取りが入れ替わることはありません。

これは MVP です。既知のギャップ:

- Vertex は gcloud のアプリケーション既定の認証情報しか取りません。署名付き JWT を必要とするサービスアカウントキーは生成されません。
- サイドチャットはどの質問でもメインの会話全体を見ます — メインの履歴は読まれるだけで、書かれることはありません — そのため `/btw` の回答は完全なコンテキストを持ち、メインスレッドは手つかずのままです。
- タスクの打ち切り以外にストリーミングのキャンセルはありません。
- コマンドはサンドボックスなしで実行されます。

## 検証済みの対象

プロトコルの詳細は `openai/codex` のソースに対して確認し、実際のリクエストで裏付けました。エンドポイントとヘッダー、リフレッシュのローテーション、テキストのストリーミング、`function_call` の往復、そしてファイルを書く複数回反復のツールループです。

## クレジット

- **音声入力**は Cactus の音声モデル **whistle** とエンジン **Needle**（`cactus-needle`、Apache-2.0）で動いています。
  製品ページは [cactuscompute.com/whistle](https://cactuscompute.com/whistle)、重みと各プラットフォーム向けエンジンは
  [Hugging Face](https://huggingface.co/Cactus-Compute/whistle) にあります。エンジンはそれらを
  `~/.cache/cactus-needle` にダウンロードします。本リポジトリには含まれていません。
- **任意のローカルタイトラー**は Hugging Face の [Candle](https://github.com/huggingface/candle) を使っています
  （Apache-2.0 または MIT）。
- それ以外は `Cargo.toml` にある crate で、それぞれ自らのライセンスに従います。

## ライセンス

本プロジェクトは GNU General Public License バージョン 3 のみでライセンスされています。[LICENSE](LICENSE) を参照してください。
サードパーティの依存関係やアセットには独自のライセンスが適用される場合があります。各ライセンス表示と上記のクレジットを参照してください。
