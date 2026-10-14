[English](README.md) · [Русский](README.ru.md) · [Українська](README.uk.md) · [Čeština](README.cs.md) · [Español](README.es.md) · **Deutsch** · [中文](README.zh.md) · [日本語](README.ja.md)

# exp-1 Agent

Ein minimaler Coding-Agent in Rust mit Terminal-UI, der mit **27 Anbietern** spricht und jedem die
Modellliste übergibt, die er tatsächlich bedient: Der Katalog kommt live von models.dev, mit einer
Tabelle in der Binärdatei als Offline-Rückfalllösung. Fünf Übertragungsprotokolle decken sie alle
ab — die **OpenAI-Chat-Completions**-API, die sich ein Dutzend Anbieter teilen, **OpenAI
Responses**, **Anthropic Messages**, Vertex AIs natives **Gemini `generateContent`** und das
Codex-Backend — und ein Anbieter meldet sich mit einem API-Schlüssel, einer Gerätecode-Anmeldung,
der eigenen Anmeldung des Herstellers oder Zugangsdaten an, die die Maschine bereits hat. Der
Codex-Weg kam zuerst und ist noch einer davon: ChatGPT-OAuth gegen `auth.openai.com`, mit Anfragen
an `https://chatgpt.com/backend-api/codex/responses`.

## Was er tut

- Streamt Antworten vom Codex-Responses-Endpunkt (SSE, von Hand geparst).
- Führt die Agentenschleife aus — Anfrage → Stream → Werkzeug ausführen → Ausgabe zurückfüttern →
  wiederholen —, bis das Modell keine Werkzeuge mehr anfordert.
- Führt zwei Threads: die Hauptunterhaltung und einen Neben-Thread für Fragen, die diese nicht
  unterbrechen dürfen.
- Zeigt Denkschritte, gestreamten Assistententext, Werkzeugaufrufe und deren Ausgabe in einem
  Transkript, mit Dauer und Uhrzeit je Runde.

## Werkzeuge

| Werkzeug | Argumente | Hinweise |
| --- | --- | --- |
| `read` | `path`, `limit?`, `pos?` | Standardmäßig höchstens 400 Zeilen; `limit` ändert das, `pos` ist die erste Zeile, 1-basiert. Die Ausgabe ist nummeriert, und ein abgeschnittener Lesevorgang sagt, wie es weitergeht. Eine Datei mit 3000 Zeilen wird absichtlich in Stücken durchgegangen: sie auf einmal zu verschlingen kostet Zehntausende Token. |
| `write` | `path`, `content` | Legt übergeordnete Verzeichnisse an; überschreibt eine vorhandene Datei. |
| `edit` | `path`, `old`, `new`, `edits?` | Exakter Ersatz, alles oder nichts. Ein Fehlschlag benennt die nächstgelegenen Zeilen oder die Positionen der Treffer, ein Abweichen bei Leerzeichen kostet also kein erneutes Lesen. `edits` nimmt mehrere `{old, new}`-Paare für eine Datei in einem Aufruf. Das Ergebnis zeigt, was sich geändert hat. |
| `fetch` | `url`, `limit?`, `pos?` | Eine Seite als reiner Text, seitenweise wie eine Datei: `limit` Zeilen (Standard 400) ab `pos` (1-basiert). Skripte und Stylesheets werden entfernt, Entities dekodiert. |
| `count` | `path`, `refresh?` | Zeilen in einer Datei oder über alle Dateien unter einem Verzeichnis, ohne den Inhalt zu lesen. Pro Datei in `.cache/count` zwischengespeichert, geschlüsselt nach Größe und mtime. |
| `grep` | `pattern`, `path?`, `include?`, `max?` | Suche mit regulärem Ausdruck über eine Datei oder einen Baum, ohne die Dateien in die Unterhaltung zu laden. `include` filtert nach Dateinamen (`*.rs`). Überspringt `.git`, `.cache`, `target`, `node_modules`; folgt keinen Symlinks; ignoriert Nicht-Textdateien. Begrenzt auf `max` Treffer (Standard 100). |
| `delete` | `path` | Entfernt eine Datei. Verzeichnisse werden abgelehnt — einen Baum zu leeren gehört in `shell`, wo der ganze Befehl sichtbar ist. |
| `shell` | `command`, `background?`, `limit?`, `output_id?` | Ein Vordergrundbefehl blockiert den Agenten, bis er fertig ist; nach 60 s wird ein Befehl in den Hintergrund hochgestuft und dem Agenten wird mitgeteilt, wann er endet. `background: true` gibt sofort eine ID zurück. Hintergrundbefehle werden nie per Zeitgeber beendet. Die Ausgabe ist auf 30 000 Bytes begrenzt: eine längere behält Anfang und Ende, die Lücke wird markiert. `output_id` liest jeden früheren Lauf mit einem anderen `limit` erneut. |
| `ast` | `path?`, `refresh?` | Projektbaum im Format `tree`. Zwischengespeichert in `.cache/ast`; `refresh: true` baut neu auf. `.git` wird übersprungen. |
| `websearch` | `query`, `numResults?`, `type?`, `livecrawl?`, `contextMaxCharacters?` | Websuche mit Seiteninhalten. Nutzt Exas gehostetes MCP unter `mcp.exa.ai`, das **keinen API-Schlüssel** braucht; `EXA_API_KEY` hebt die Ratenlimits auf. `type` ist `auto`/`fast`/`deep`. |
| `ask_question` | `question`, `options?`, `label?`, `submit?` | Blockiert die Runde und zeigt ein Fragen-Panel. `↑↓` wählen eine Option, `Tab` wechselt zwischen Fragen, `Enter` bestätigt, `Esc` schließt. Was getippt wird, gilt als freie Antwort und gewinnt gegen die hervorgehobene Option. |

Shell-Befehle laufen ohne Sandbox als der Benutzer. Ein Vordergrundbefehl wird nach 60 Sekunden in
den Hintergrund hochgestuft, nichts beendet einen Hintergrundbefehl per Zeitgeber, und die Ausgabe
ist auf 30 000 Bytes begrenzt: ein längeres Ergebnis behält Anfang und Ende, die Lücke wird als
ausgelassen markiert.

Das Werkzeug `web` durchsucht das Web: zuerst Exas gehostetes MCP, weil es Seiteninhalt liefert,
und DuckDuckGos HTML-Seite, sobald Exas gemeinsame kostenlose Stufe die Rate begrenzt.
`EXA_API_KEY` stellt Exa auf deinen eigenen Tarif um und ist die einzige Einstellung, die einer der
beiden Wege braucht.

## Plattformen

Der Agent ist eine Codebasis für Linux, macOS und Windows: TUI, Agentenschleife, Anbieter, MCP,
Skills, Sitzungen und der mitgelieferte Titler sind reines Rust und laufen überall. Die Teile, die
das Betriebssystem berühren, haben eine Verzweigung je Plattform:

| | Linux | macOS | Windows |
| --- | --- | --- | --- |
| TUI, Agent, Anbieter, MCP, Skills, Sitzungen | ja | ja | ja |
| Öffner der Anmeldeseite | `xdg-open` | `open` | `cmd /C start` |
| Daemon-Lebenszeichen und -Stopp | `kill` 0 / SIGTERM→SIGKILL | wie Linux | `tasklist` / `taskkill /T` dann `/F` |
| Server von seiner Konsole abgekoppelt | `process_group(0)` | wie Linux | `DETACHED_PROCESS \| CREATE_NEW_PROCESS_GROUP` |
| Konfigurations- und Statusverzeichnisse | `$XDG_*`, `~/.config` | `$XDG_*`, `~/.config` | `%APPDATA%`, `%LOCALAPPDATA%` |
| Zwischenablagebild (`ctrl + v`) | `wl-paste`, `xclip` | `pngpaste` | PowerShell + `System.Windows.Forms` |
| Benachrichtigung am Rundenende | `notify-send` | `osascript` | PowerShell-Sprechblase |
| Archivinstallationen (`exp-1 install *.zip`) | `unzip` / `tar` | wie Linux | `tar` (bsdtar liest zip) |
| Lokale MCP-Server namens `npx` | wie geschrieben | wie geschrieben | erneut versucht als `npx.cmd` |

Die Unix- und Windows-Zweige des Daemons und der Zwischenablage wurden auf Linux durch
vorübergehendes Aktivieren auf Kompilierbarkeit geprüft; `creation_flags` ist der einzige
Windows-spezifische Aufruf, der sich ohne Windows-Ziel nicht prüfen lässt, und nichts davon wurde
auf einer echten Windows- oder macOS-Maschine ausgeführt.

### Cross-Kompilierung von Linux aus

Ein Windows-Binary lässt sich auf Arch mit der mingw-Toolchain bauen (das erfordert einmalig root):

```sh
sudo pacman -S --needed mingw-w64-binutils mingw-w64-gcc
rustup target add x86_64-pc-windows-gnu
cargo build --release --target x86_64-pc-windows-gnu
```

`dlltool` aus `mingw-w64-binutils` brauchen die Importbibliotheken; ohne es bleibt der Build bei
`windows-sys` stehen.

Ohne root entsteht dasselbe Binary aus einem unter `~/.local` entpackten llvm-mingw-tarball. Die
Rust-std für dieses Ziel bittet den Linker um `libgcc.a`, das llvm-mingw nicht mitliefert, deshalb
stellt ein Shim aus zwei Symlinks die Entsprechungen bereit, und `--no-default-features` lässt den
mitgelieferten Titler weg, so wie die CI es tut:

```sh
mkdir -p ~/.local/mingw-shim
ln -sf ~/.local/llvm-mingw/lib/clang/*/lib/windows/libclang_rt.builtins-x86_64.a ~/.local/mingw-shim/libgcc.a
ln -sf ~/.local/llvm-mingw/x86_64-w64-mingw32/lib/libunwind.a ~/.local/mingw-shim/libgcc_eh.a
PATH="$HOME/.local/mingw-shim:$HOME/.local/llvm-mingw/bin:$PATH" \
CARGO_TARGET_X86_64_PC_WINDOWS_GNU_LINKER="$HOME/.local/llvm-mingw/bin/x86_64-w64-mingw32-clang" \
RUSTFLAGS="-L $HOME/.local/mingw-shim" \
cargo build --release --target x86_64-pc-windows-gnu --no-default-features
```

`cargo` und `rustup` aus `~/.cargo/bin` kennen die Cross-Ziele; die Systempakete nicht, und auf
dieser Maschine liegt `~/.cargo/bin` nicht auf dem `PATH`, also mit vollem Pfad aufrufen.

Ein macOS-Binary braucht ebenfalls kein root, und mit `cargo zigbuild` lässt sich der ganze Baum
für `aarch64-apple-darwin` bauen: `zig cc` kompiliert die C-Abhängigkeiten cross (`aws-lc-sys`
eingeschlossen, was ein einfaches `cargo check` nicht kann — das `cc` des Hosts weist
`-arch arm64` zurück), und jede Crate kompiliert. Was es stoppt, ist der letzte Schritt: Das
`rustls`-Feature von `reqwest` 0.13 zieht `rustls-platform-verifier` herein, und der Linker meldet
dann `unable to find framework 'Security'` und `'CoreFoundation'`. Die gibt es nur im Apple SDK;
Zig linkt Mach-O und libSystem problemlos, es liefert die Frameworks nicht mit. Die
macOS-Binaries kommen also von einem Mac oder aus der CI (`.github/workflows/release.yml` baut
beide Architekturen). Das folgende Rezept kommt bis zum Linken:

```sh
mkdir -p ~/.local/zig && curl -L https://ziglang.org/download/0.14.1/zig-x86_64-linux-0.14.1.tar.xz \
  | tar -xJ -C ~/.local/zig --strip-components=1
cargo install cargo-zigbuild
rustup target add aarch64-apple-darwin
PATH="$HOME/.local/zig:$PATH" cargo zigbuild --release --target aarch64-apple-darwin
```

Das Ergebnis ist unsigniert. Auf Apple Silicon wird ein unsigniertes Binary beim Start beendet,
deshalb muss es einmal auf dem Mac selbst signiert werden:

```sh
codesign -s - --force ./exp1-agent        # ad-hoc signature
xattr -cr ./exp1-agent                    # if it arrived by download
```

## Voraussetzungen

- Rust 1.85+ (Edition 2024)
- Entweder eine vorhandene Anmeldung in der Codex CLI (deren `auth.json` wiederverwendet wird)
  oder `/login`, das denselben Gerätecode-Ablauf selbst ausführt.
- Die benachbarte Crate `titler` unter `../mygame/titler`. Sie zieht Candle herein, weshalb der
  erste Build eine Weile dauert und das Binary ~16 MB statt ~7 MB groß ist. Sobald die Gewichte im
  Hugging-Face-Cache liegen, läuft die Titelvergabe offline.

## Verwendung

```sh
cargo run --release                      # uses the current directory
exp1 --model gpt-6-luna --effort high    # pick a model and reasoning effort
exp1 --cwd ~/projects/my-app             # run the shell tool somewhere else
exp1 --help
```

| Taste | Aktion |
| --- | --- |
| `Enter` | Den Prompt senden oder den Befehl ausführen, wenn er bereits vollständig ist |
| `↑` `↓` | Die Auswahl der Vervollständigung bewegen oder frühere Prompts zurückholen |
| `Tab` | Die hervorgehobene Vervollständigung übernehmen |
| `Ctrl + /` | Zwischen Haupt-Thread und Neben-Thread wechseln |
| `Ctrl + Shift + A` | Den Berechtigungsmodus wechseln: always ask → never ask → ask only commands |
| `Esc` | Für sich allein nichts |
| `Esc` `Esc` | Die laufende Runde abbrechen oder die Vervollständigungsliste bzw. die Auswahl schließen |
| `Ctrl-C` | Beenden |
| `Ctrl-U` | Die Eingabezeile leeren |
| `Ctrl-Backspace` | Das vorherige Wort löschen |
| `Ctrl-Delete` | Das nächste Wort löschen |
| `Ctrl-W`, `Alt-Backspace` | Wie `Ctrl-Backspace` |
| Mausrad | Das Transkript scrollen |
| `PgUp` / `PgDn` | Das Transkript scrollen |
| `←` `→` `Home` `End` | Den Cursor bewegen |
| `Backspace` / `Delete` | Bearbeiten |

Die Pfeiltasten wandern durch die Prompt-Historie: Nach dem Senden von `hi` holt `↑` es zurück ins
Feld, weitere Drücke gehen weiter zurück, und `↓` geht vorwärts bis zum leeren Feld. Gescrollt wird
mit dem Mausrad, damit die Pfeiltasten für die Historie frei bleiben.

Tippen von `/` öffnet eine nach Präfix gefilterte Vervollständigungsliste. Nach einem Leerzeichen
vervollständigt sie auch die Argumente des Befehls — `/themes <tab>` listet die Farbschemata,
`/model <tab>` die Modelle und `/model gpt-6-luna <tab>` die Aufwandsstufen. `Enter` auf ein
nacktes `/model` oder `/themes` öffnet stattdessen eine nummerierte Auswahlliste; `/model` ist
zweistufig, erst Modell, dann Aufwand. Bei leerem Feld sitzt der Cursor ganz am Anfang, vor dem
Platzhalter.

Die Hinweiszeile ändert sich je nach Kontext. Die Eingabe ist gesperrt, während eine Runde läuft.

`Esc` ist absichtlich wirkungslos; erst der Doppeldruck unterbricht. Unterbrechen bricht die
laufende Anfrage ab und verwirft jeglichen Teiltext des Assistenten, die Unterhaltungshistorie
bleibt unangetastet.

## Slash-Befehle

Werden lokal behandelt; sie erreichen das Modell nie.

| Befehl | Wirkung |
| --- | --- |
| `/login` | Einen Anbieter wählen: ein angemeldeter wechselt einfach; wählt man den gerade genutzten, meldet man sich erneut an. Gerätecode-Anmeldungen öffnen die Seite des Herstellers im Browser; die übrigen nehmen einen API-Schlüssel |
| `/scoped-models` | Anhaken, welche Modelle `/model` anbieten darf; leer bedeutet alle |
| `/sessions` | Eine gespeicherte Unterhaltung zum Wiederöffnen wählen |
| `/init` | Ein `AGENTS.md`-Gerüst im Arbeitsverzeichnis schreiben |
| `/model` | Die zweistufige Auswahl öffnen (Modell, dann Aufwand); `/model <name> [low\|medium\|high]` setzt sie direkt |
| `/themes` | Die Farbschemaauswahl öffnen; `/themes <name>` setzt sie direkt |
| `/status` | Modell, Aufwand, Arbeitsverzeichnis, Werkzeuge, Farbschema, Plan, Anmelde-Anbieter anzeigen |
| `/btw` | Eine Nebenfrage im Neben-Thread beantworten, während die Haupt-Runde weiterläuft |
| `/clear` | Das Transkript und die Unterhaltungshistorie leeren |
| `/help` | Tastenkürzel anzeigen |
| `/compact` | Die Unterhaltung jetzt zusammenfassen, statt zu warten, bis das Fenster voll ist |
| `/undo` | Die Dateien wiederherstellen, die die letzte Runde geändert hat (im Speicher; ein Neustart vergisst sie) |
| `/diff` | Unified-Diff dessen, was die letzte Runde geändert hat |
| `/fork` | Diese Unterhaltung als neue Sitzung fortsetzen und die ursprüngliche unberührt lassen |
| `/sessions <text>` | Die gespeicherten Unterhaltungen durchsuchen, statt sie nur aufzulisten |
| `/retitle` | Die Sitzung erneut benennen, aus der ursprünglichen Anfrage und der jüngsten Unterhaltung |
| `/effort` | Den Denkaufwand für das aktuelle Modell wählen, ohne das Modell zu wechseln |
| `/mcp` | Die MCP-Server und die von ihnen angebotenen Werkzeuge auflisten |

Verfügbare Farbschemata: `default` (blau), `amber`, `jade`, `mono`. Die Farben sind Truecolor, ein
Terminal-Farbschema kann sie also nicht umdeuten.

## Oberfläche

Das Layout folgt der TUI der OpenAI Codex: keine umrandeten Panels, ein `>_`-Kopfbereich mit dem
Arbeitsverzeichnis darunter, ein Vervollständigungsfenster oder eine nummerierte Auswahlliste über
der Eingabe, eine gepolsterte Eingabeleiste mit `›`-Prompt und blinkendem `│`-Cursor, dann eine
Modellzeile und eine Hinweiszeile.

- Ein abgeschickter Prompt wird, wie bei Codex, als Band über die volle Breite im Transkript
  dargestellt.
- **Die Werkzeugausgabe ist standardmäßig verborgen**: Das Transkript trägt den Aufruf
  (`read src/main.rs`), nicht das, was er zurückgegeben hat. `ctrl-o` zeigt die Ausgabe unter ihrem
  Aufruf und verbirgt sie wieder — das ist der Unterschied zwischen der Arbeit zu folgen und sie zu
  lesen.
- **Die Maus ist nicht nur zum Scrollen da.** Das Rad scrollt, Ziehen markiert und kopiert, und ein
  Klick wirkt auf das, worauf er gefallen ist: eine Zeile einer Auswahlliste oder des
  Vervollständigungsfensters verhält sich wie Enter auf dieser Zeile, der Hinweis
  `ctrl + / for side` in der Fußzeile wechselt den Thread, und ein Link in einer Nachricht öffnet
  sich im Browser. Jeder Frame zeichnet auf, welche Bereiche er gemalt hat, ein Klick ist also ein
  Treffertest gegen den letzten Frame statt einer Vermutung über das Layout. Hover-Hervorhebung ist
  absichtlich nicht implementiert: Sie kostet ein Neuzeichnen pro Mausbewegung, und der Cursor des
  Terminals sagt bereits, wo der Zeiger ist.
- Ein Subagent meldet seinen aktuellen Schritt in **einer Zeile, die sich aktualisiert**
  (`subagent → grep tokenizer`) statt einer Zeile pro Schritt, und der Aufruf, der ihn gestartet
  hat, lautet `task: <description>`.
- **Ein Klick auf eine eigene Nachricht fragt, was damit geschehen soll**, statt es einfach zu tun:
  ein kleines Menü mit `copy` (der Text geht in die Zwischenablage) und `revert`. Zurücksetzen
  verwirft alles nach dieser Nachricht — die Unterhaltung, das, was das Modell unterhalb dieses
  Punktes gesagt bekam, und jede Dateiänderung der nachfolgenden Runden, die das Undo-Protokoll
  zurücknimmt. Die Nachricht selbst verlässt das Transkript, und ihr Text kehrt in den Prompt
  zurück, sodass er bearbeitet und erneut gesendet werden kann. Der Text wird geprüft, bevor etwas
  verworfen wird, ein Klick auf die falsche Nachricht tut also nichts.
- Das Transkript hält sich an die neueste Nachricht; das Mausrad oder `PageUp`/`PageDown` scrollt
  durch die Historie nach oben, `ctrl-g` springt zurück zur neuesten Nachricht und `ctrl-t` zur
  ältesten, und eine hochgescrollte Ansicht bleibt stehen, während neue Zeilen eintreffen, statt
  wieder nach unten gezogen zu werden.
- Ziehen mit der Maus markiert Text überall auf dem Bildschirm — Kopfbereich inklusive — und beim
  Loslassen landet er in der Zwischenablage: zuerst OSC 52, damit es auch über ssh funktioniert,
  dann `wl-copy`, `xclip` oder `xsel`, wenn sie vorhanden sind. Ein Hinweis `copied N chars`
  bestätigt es in der Hinweiszeile. Die Auswahl ist ein ruhiger Farbton hinter dem Text und endet am
  letzten Zeichen jeder Zeile, so wie sich die Auswahl des Terminals selbst verhält. Das Terminal
  sieht das Ziehen nie, weil die Anwendung die Maus zum Scrollen belegt, dies ist also der einzige
  Weg zum Kopieren.
- Ein angehängtes Bild erscheint als ausgefüllter `Image 1`-Chip über dem Prompt, daneben
  `ctrl-v replaces it`, sodass sichtbar ist, was gleich gesendet wird — direkt in der Eingabeleiste.
- Während eine Runde läuft, sitzt eine Aktivitätszeile **direkt unter dem Prompt, zu dem sie
  gehört** — mit eigenem Label für jede Tätigkeit: `⠋ thinking 4s`, solange eine Anfrage unterwegs
  ist, `⠋ preparing write… (12 KB) 8s`, solange das Modell noch die Argumente eines Werkzeugaufrufs
  schreibt, `⠋ running 4s`, solange das Werkzeug ausgeführt wird. Jedes Label trägt seine eigene
  Uhr, die Sekunden beziehen sich also auf das, was gerade geschieht, und zählen nie die ganze
  Runde; die Gesamtdauer der Runde meldet am Ende `Worked for`. Die untere Leiste bleibt ein
  schlichtes `GPT-6-Luna high · /path · <thread> · Main [default]`-Label;
  `esc esc interrupts` steht in der Hinweiszeile.
- Nach dem Ende einer Runde wird `Worked for 5s · 12:08 PM` an das Transkript angehängt.
- Die Modellzeile lautet `GPT-6-Luna high · /path · <thread name> · Main [default]` oder
  `… · Agent` im Neben-Thread. Der Thread-Name stammt aus dem ersten Prompt.
- Die Hinweiszeile trägt den Thread-Wechsel rechts, in Rot, während der Neben-Thread aktiv ist.
- Ein leerer Bildschirm bleibt ruhig: Die vollständige Befehlsliste erscheint nur auf einer
  Maschine, die noch nie eine Unterhaltung gespeichert hat, danach ist es eine Zeile mit Verweis
  auf `/` und `/help`.
- Tippen bleibt während einer Runde aktiv, eine `/btw`-Frage lässt sich also stellen, ohne zu
  warten.
- `/btw` oder `ctrl + /` antwortet in einem Neben-Chat, der die ganze Hauptunterhaltung als Kontext
  mitführt, ohne ihr etwas hinzuzufügen.
- `shift + tab` wechselt zwischen Build- und Plan-Modus; die Fußzeile zeigt `plan mode`, solange er
  an ist.
- Eine Runde, die länger als fünfzehn Sekunden läuft, läutet bei ihrem Ende die Terminalglocke (und
  `notify-send`, wo der Desktop es hat), ein langer Auftrag muss also nicht beobachtet werden.
- `shift + enter` schreibt einen Zeilenumbruch statt zu senden, ein Prompt kann also mehrzeilig
  sein; die Eingabeleiste wächst mit.
- `ctrl + v` holt ein Bild aus der Zwischenablage (Wayland `wl-paste`, X11 `xclip`) und hängt es an
  den nächsten Prompt, mit `▣ image attached` in der Hinweiszeile, bis es gesendet wird. Das Bild
  reist als Data-URL, die Backends Codex, OpenAI, Anthropic und Gemini reichen es also alle weiter.
- `ask_question` hebt ein Panel über die Eingabe, das dem Question-Werkzeug nachempfunden ist: ein
  Akzentbalken an der linken Kante, der Titel `Questions`, Reiter, wenn mehrere Fragen zusammen
  gestellt werden, nummerierte Optionen mit hervorgehobener Auswahl, ein Feld
  `Type your own answer` und `⇥tab ↑↓ select enter confirm esc dismiss` am unteren Rand.

Die Farben sind Truecolor, ein Terminal-Farbschema kann sie also nicht umdeuten. Modell, Aufwand und
Farbschema wechseln zur Laufzeit über `/model` und `/themes`.

Unterschiede zu Codex, die absichtlich sind: Codex hat `/permissions` und `/review`, die
Sandbox-Richtlinien und einen Diff-Reviewer brauchen; dieser Build hat stattdessen `/clear` und
`/help`. Die Fußzeile von Codex bewirbt Agenten- und Tastenkürzel-Overlays, die dieser MVP nicht
hat.

## Authentifizierung

`/login` listet die Anbieter auf, und jeder meldet sich so an, wie es sein Hersteller erwartet:

- **Ein API-Schlüssel** für die meisten von ihnen, gesendet als `Authorization: Bearer` (oder
  `x-api-key` für die Anthropic-API) und pro Anbieter in `~/.config/exp-1/credentials.json`
  gespeichert, Modus `0600`. Ein Schlüssel kann mehrere Anbieter abdecken — `opencode-go` und
  `opencode-zen` sind ein Konto hinter zwei Basis-URLs, und `providers::key_group` benennt die
  Gruppen.
- **Eine Gerätecode-Anmeldung** für `openai (auth)`, `github copilot` und `meta (auth)`: Die
  Anmeldeseite des Herstellers öffnet sich im Browser, und URL und Code werden als Rückfalllösung
  ausgegeben.
- **Ein Token aus der eigenen Anmeldung des Herstellers** für `anthropic (auth)`, gesendet mit den
  Identitäts-Headern des herstellereigenen Clients.
- **Zugangsdaten, die die Maschine bereits hat**, für `google vertex`: was
  `gcloud auth application-default login` bereits angemeldet hat, wobei `GOOGLE_CLOUD_PROJECT` und
  `GOOGLE_CLOUD_LOCATION` Projekt und Region benennen. Nichts einzufügen.

Der Codex-Weg hält außerdem eine eigene Datei vor: Beim ersten Lauf kopiert der Agent
`$CODEX_HOME/auth.json` (Standard `~/.codex/auth.json`) nach `~/.exp1-agent/auth.json` und arbeitet
von dieser Kopie aus. **Deine echten Codex-Zugangsdaten werden nie überschrieben.** Wenn der
Access-Token innerhalb von 5 Minuten abläuft, frischt der Agent ihn auf:

```
POST https://auth.openai.com/oauth/token
grant_type=refresh_token
client_id=app_EMoamEEZ73f0CkXaXp7hrann
```

Die Antwort rotiert den Refresh-Token, der neue wird also in die eigene Kopie des Agenten
zurückgeschrieben. Die gespeicherte Datei wird atomar mit Modus `0600` geschrieben.

## Modelle und Aufwandsstufen

Weder die Modellliste noch die Aufwandsstufen sind fest einprogrammiert. Beide werden aus dem
eigenen Katalog der Codex CLI unter `$CODEX_HOME/models_cache.json` (Standard
`~/.codex/models_cache.json`) gelesen, und zwar aus `slug`, `display_name`, `description`,
`visibility`, `default_reasoning_level` und `supported_reasoning_levels` jedes Eintrags. Der
Startstandard ist das erste gelistete Modell in diesem Katalog.

**Jeder andere Anbieter bekommt seine Liste live, von models.dev.** Der Katalog wird höchstens
alle zwölf Stunden nach `~/.cache/exp-1/models-dev.json` geholt, im Speicher gehalten und für drei
Dinge gelesen: die Modell-IDs, die ein Anbieter anbietet, das Protokoll, das jedes Modell spricht
(models.dev hält es als das AI-SDK-Paket fest, mit dem der Anbieter konfiguriert ist —
`@ai-sdk/anthropic` für ein Claude-Modell hinter einem Gateway, `@ai-sdk/openai` für ein
Grok-Modell), und das Kontextfenster, aus dem die Schwelle für die automatische Komprimierung
berechnet wird. Ein Anbieter, der im Katalog fehlt, behält die Tabelle in `src/providers.rs`, und
ebenso eine Maschine, die offline ist, die App hängt also nie an diesem Abruf.

Ein Modell, das die Tabellen völlig verfehlen, bekommt trotzdem ein Protokoll: Sein
Herstellerpräfix entscheidet (`claude-*`, `minimax-*` und `qwen-*` sprechen Anthropic, `grok-*`,
`gpt-*` und `muse-*` sprechen Responses, `gemini-*` spricht Gemini, alles andere ist Chat), und ein
Anbieter, der durchweg ein Protokoll spricht, antwortet für alles Übrige.

**Die Auswahl zeigt jeden Anbieter, der angemeldet ist**, eine einzige Liste statt einer pro
Anmeldung: Die Zeilen tragen `via <provider>`, sodass zwei Anbieter unter demselben Modellnamen
auseinanderzuhalten sind, der genutzte Anbieter steht zuerst, und die Wahl eines Modells von einem
anderen Anbieter wechselt erst zu diesem, bevor das Modell gesetzt wird. Eine so lange Liste wird
durch Tippen durchsucht: Jede Auswahl filtert beim Tippen, Backspace erweitert die Treffermenge,
und Escape erweitert, bevor es schließt. Der Umfang in `/scoped-models` filtert dieselbe Liste und
nennt ein Modell eines Anbieters (`opencode-go/grok-4.7`), derselbe Name von zwei Anbietern ist
also zwei Einträge; ein älterer Umfang mit bloßen Namen bedeutet weiterhin „dieser Name, von jedem
Anbieter", und `ctrl-a` löscht in dieser Auswahl den Umfang, was wieder alles anbietet.
`/model grok-4.7` nimmt den genutzten Anbieter, wenn mehrere diesen Namen anbieten, und
`/model opencode-go/grok-4.7` benennt einen genau.

Die Aufwandsstufen unterscheiden sich tatsächlich je Modell, deshalb funktioniert Raten nicht:

| Modell | Aufwandsstufen |
| --- | --- |
| `gpt-5.6-terra` | low, medium, high, xhigh, max, **ultra** |
| `gpt-6-luna` | low, medium, high, xhigh, max |
| `gpt-5.5` | low, medium, high, xhigh |

Die Aufwandsauswahl, die Vervollständigungen von `/model <name> <tab>` und die Prüfung folgen alle
dem jeweils aktiven Modell. Ein Modellwechsel übernimmt den Aufwand, wenn das neue Modell ihn
akzeptiert, und fällt sonst auf `default_reasoning_level` dieses Modells zurück, man kann also nie
eine Aufwandsstufe verlangen, die das Modell nicht hat:

```
/model gpt-5.6-terra ultra   →  GPT-5.6-Terra ultra
/model gpt-5.5               →  GPT-5.5 medium          (ultra is not a gpt-5.5 level)
/model gpt-5.5 max           →  'max' is not a GPT-5.5 level;
                                this model takes low, medium, high, xhigh
```

Nur Modelle mit `visibility: "list"` erscheinen in der Auswahl; verborgene bleiben über ihren Namen
erreichbar. Fehlt die Katalogdatei, fällt die App auf eine konservative eingebaute Liste zurück,
damit sie brauchbar bleibt, und sagt es in der Auswahl.

Antworten werden nur Modelle, auf die das angemeldete ChatGPT-Konto Anspruch hat — der Rest wird
vom Backend mit einer erklärenden Fehlermeldung abgelehnt.

## Aufbau der Anfrage

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

Ausgewertete Ereignisse: `response.output_text.delta`, `response.reasoning_summary_text.delta`,
`response.output_item.done`, `response.completed`, `response.failed`, `response.incomplete`,
`error`. Alles andere wird ignoriert.

## Darstellung

Antworten des Assistenten werden prozessintern als Rich Text gerendert. Es gibt keinen externen
Renderer, kein `pandoc`, kein TeX-Binary, keine Bildrasterung — alles geschieht innerhalb der TUI
und kostet daher fast nichts.

- **Markdown**: Überschriften, Fett, Kursiv, Inline-Code, eingezäunte Codeblöcke mit Randspalte und
  Sprachkennung, Aufzählungs- und nummerierte Listen, Blockzitate, horizontale Linien, Links,
  GitHub-artige Pipe-Tabellen mit Rahmen aus Linienzeichen und Ausrichtung je Spalte sowie
  Inline-Mathematik `$…$`.
- **LaTeX-Mathematik**: Symbole und Relationen, Griechisch, `\frac`, `\sqrt`, Tief- und
  Hochstellungen, `\mathbb` und `\mathcal`, kombinierende Akzente (`\bar`, `\hat`, `\vec`, …),
  aufrechte Operatornamen (`\log`, `\sin`, `\lim`, …) und Anzeigeblöcke `$$…$$`.

Alles, was der Konverter nicht kennt, wird wörtlich ausgegeben, geschweifte Klammern inklusive,
unbekannte Eingaben bleiben also lesbar statt zu Brei zu werden. Er ist ein Konverter, keine
TeX-Engine: Makros, Umgebungen wie `tikz` und mehrzeilige Layouts werden nicht interpretiert.

Markdown wird **einmal** geparst, wenn eine Nachricht eintrifft, in farbschemaunabhängige
Textläufe; das Zeichnen ordnet diese Läufe nur Farben zu. Ein Farbschemawechsel parst daher nichts
neu.

Am gebauten Binary gemessen: **~1 % eines Kerns im Leerlauf**, mit blinkendem Cursor, ~3 % beim
schnellen Scrollen mit dem Rad.

## Subagenten und lange Befehle

**Subagenten.** Das Werkzeug `task` führt einen zweiten Agenten mit eigenem Kontext und denselben
Werkzeugen aus und gibt nur dessen Bericht zurück — nützlich für Arbeit, die sonst den Kontext des
Aufrufers fluten würde. Der Bericht wird bei 6 000 Zeichen abgeschnitten, weil er im Kontext des
Aufrufers landet, und das Transkript zeigt seinen Fortschritt in einer einzigen Zeile, die bei
jedem Schritt ersetzt wird, statt einer Zeile pro Schritt. Verschachtelte Subagenten werden
angeboten, solange Tiefe übrig ist, begrenzt auf zwei Ebenen, damit die Rekursion nicht ausufert.
Das Transkript zeigt jeden Schritt, den der Subagent macht.

**Langsame Befehle werden hochgestuft, nicht getötet.** Ein Vordergrundbefehl, der nach
**60 Sekunden** noch läuft, wird in den Hintergrund verschoben und dem Agenten wird das mitgeteilt,
statt die Runde zu blockieren:

```
'sleep 75' is still running after 60s, so it was moved to the background as 'bg-1'.
Carry on; you will be told when it finishes, or poll it with {"output_id": "bg-1"}.
```

**Das Ende unterbricht.** Wenn ein Hintergrundbefehl endet, wird die laufende Runde abgebrochen und
dem Agenten die Ausgabe übergeben, ein Ergebnis geht also nie stillschweigend hinter langer Arbeit
verloren:

```
• interrupted: a background command finished
```

## Diktat

Der Mikrofonknopf in der unteren Ecke (`ctrl-r` tut dasselbe) diktiert in den Prompt: drücken,
sprechen, erneut drücken. Text trifft ein, während du sprichst, weil der Erkenner streamt, und der
Knopf trägt einen Zeitgeber, solange er zuhört.

Der Erkenner ist **whistle** — die lokal laufende Sprach-Engine in `../mygame/whistle_rs` —,
gestartet als `whistle --mic`. Dieser Modus lässt ihn streamen: Er nimmt rohes 16-kHz-Mono von
pipewires `pw-record` sekundenweise auf, schickt jede Sekunde durch den Streaming-Decoder und gibt
pro Aktualisierung eine JSON-Zeile aus (`text` ist das Bestätigte, `pending` das noch Laufende).
Ein Zeilenumbruch auf seinem stdin beendet den Strom und spült den Rest aus. Befehl, Sprache,
Bias-Wörter und Audioquelle stammen aus der Umgebung:

Das Aufnahmeprogramm ist das der Plattform selbst: `pw-record` (pipewire) unter Linux, `ffmpeg` mit
avfoundation unter macOS, `ffmpeg` mit dshow unter Windows (`WHISTLE_SOURCE` wählt das Eingabegerät,
`WHISTLE_RECORDER` ersetzt den Befehl ganz). Die App selbst startet nur `EXP1_DICTATE --mic` und
liest dessen JSON-Zeilen, jeder Erkenner, der auf dieselbe Weise streamt, funktioniert also; ohne
einen sagt der Druck auf den Mikrofonknopf das in der Fußzeile und sonst passiert nichts.

```sh
EXP1_DICTATE=/path/to/whistle     # default: whistle on PATH
EXP1_DICTATE_LANG=en              # default: whatever it detects
EXP1_DICTATE_KEYWORDS=country,Americans
WHISTLE_SOURCE=…                  # default: the system's own input
```

Das Diktat gibt es nur unter Linux, weil auch das Aufnahmeprogramm nur dort existiert; der Rest
des Agenten nicht.

Dieser Modus ist lokal in dieser Kopie von whistle: Die Logik liegt in ihrer Bibliothek als Typ
`Dictation` (`start`, `next`, `finish`), und ihre `main.rs` ist eine dünne Schleife darum. whistle
hat hier kein eigenes Repository, eine frische Kopie von seinem Autor muss also diese zwei Dateien
erneut eingespielt bekommen, oder `EXP1_DICTATE` muss auf etwas zeigen, das dasselbe Protokoll
spricht. Ein Erkenner, der `--mic` nicht kennt, wird nicht ignoriert: Er sagt es, mit eigenen
Worten, in der Fußzeile.

## Anbieter

`/login` öffnet eine Anbieterauswahl; `/scoped-models` entscheidet, welche von deren Modellen
`/model` anbietet (leer bedeutet alle, und der Umfang wird in `settings.json` gespeichert). Die
Auswahl ist eine schlichte alphabetische Liste, der Cursor steht beim Öffnen auf dem genutzten
Anbieter, nichts muss also über seine Position gefunden werden. Die Wahl eines bereits angemeldeten
Anbieters wechselt zu ihm und sagt es; die Wahl des gerade genutzten — oder `/login <id>` — meldet
absichtlich erneut an. Eine Gerätecode-Anmeldung öffnet die Seite des Herstellers im Browser und
gibt URL und Code als Rückfalllösung aus.

**Ein Schlüssel kann mehrere Anbieter abdecken.** `opencode-go` und `opencode-zen` sind ein Konto
hinter zwei Basis-URLs, die Anmeldung bei einem von beiden speichert den Schlüssel also für beide
(`providers::key_group` benennt die Gruppen). Die opencode-Gateways leiten außerdem nach Sitzung
weiter: Jede Anfrage trägt `x-opencode-session`, `x-opencode-project` und `x-opencode-client`, ohne
die das Gateway sie rundheraus ablehnt („Request is missing x-opencode-session").

Fünf Übertragungsprotokolle decken alles ab, was hier implementiert ist: die
**OpenAI-Chat-Completions**-API, die sich ein Dutzend Anbieter teilen, **OpenAI Responses**,
**Anthropic Messages**, Vertex AIs natives **Gemini `generateContent`** und das **Codex-Backend**,
mit dem `openai (auth)` spricht. Alle streamen SSE und
alle werden in denselben Ereignisstrom dekodiert, den der Codex-Client erzeugt, die
Agentenschleife weiß also nicht, mit welchem sie spricht.

Schlüssel werden pro Anbieter in `~/.config/exp-1/credentials.json` gespeichert, Modus
`0600`. `openai (auth)` hält stattdessen eine `auth.json` im Codex-Stil, `google vertex` nutzt, was
`gcloud auth application-default login` bereits angemeldet hat (wobei `GOOGLE_CLOUD_PROJECT` und
`GOOGLE_CLOUD_LOCATION` Projekt und Region benennen), und die Gerätecode-Anbieter legen den
erhaltenen Token in dieselbe Zugangsdatendatei.

| Anbieter | Basis-URL | Protokoll |
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

Jeder Eintrag ist implementiert: Jedes Modell nennt das Protokoll, auf dem es antwortet, ein
Gateway wie `opencode-go` kann also drei gleichzeitig bedienen. Die Liste und die Basis-URLs stammen
von [models.dev](https://models.dev), dem öffentlichen Register, das opencode selbst liest, und
nicht aus Vermutungen.

Jede der obigen Basis-URLs wurde mit einem absichtlich ungültigen Schlüssel geprüft: Eine Ablehnung
beweist, dass URL und Auth-Header stimmen. Alle siebenundzwanzig antworteten mit `400`, `401` oder
`403` — außer `fireworks-ai`, dessen `404` ein eigenes `"Model not found"` mit einer
`chatcmpl-`-Request-ID ist, selbst diese Anfrage erreichte also die Inferenzroute. `google vertex`
antwortet auf seinem projektbezogenen Pfad mit `401`.

## Modi

`shift + tab` wechselt die Sitzung zwischen zwei Modi, und die Fußzeile sagt, welcher aktiv ist:

- **build** — alles wie hier beschrieben.
- **plan** — die Werkzeuge, die Dateien ändern (`write`, `edit`, `delete`), werden abgelehnt *und*
  aus der Werkzeugliste weggelassen, das Modell untersucht also und schlägt einen Plan vor, statt
  zu bearbeiten. Von `shell` wird erwartet, dass es nur liest. Der Wortlaut liegt in
  `prompts/plan.md`, editierbar wie der Rest.

## Berechtigungen

`~/.config/exp-1/permissions.json` ist eine Liste von Ausnahmen; ein Aufruf, auf den keine Regel
passt, läuft. Der erste Treffer entscheidet:

```json
{
  "rules": [
    { "action": "shell", "resource": "rm -rf *", "effect": "deny" },
    { "action": "write", "resource": "/etc/*", "effect": "ask" },
    { "action": "fetch", "resource": "*", "effect": "allow" }
  ]
}
```

`action` ist ein Werkzeugname oder `*`, `resource` ist das, was der Aufruf berührt — ein Pfad für
Dateiwerkzeuge, die Kommandozeile für `shell`, die URL oder Suchanfrage für die Netzwerkwerkzeuge —,
und `*`/`?` wirken als Platzhalter. `effect` ist `allow`, `ask` oder `deny`. Ein `ask` öffnet das
Fragen-Panel und hält den Aufruf, bis er beantwortet ist (zwei Minuten, danach wird der Aufruf
abgelehnt). Regeln gelten auch für Subagenten.

`ctrl + shift + a` wechselt den Berechtigungsmodus: **always ask** (die Voreinstellung einer
frischen Installation) lässt `write`, `edit`, `delete` und `shell` die Frage stellen, **never ask**
führt sie ohne Frage aus, und **ask only commands** fragt nur für `shell`. Werkzeuge, die nur lesen,
fragen nie, und eine Regel mit `effect: deny` lehnt in jedem Modus ab — der Modus lockert ein Verbot
nie.

## Projektanweisungen

`AGENTS.md` wird aus dem Arbeitsverzeichnis und jedem darüberliegenden Elternverzeichnis gelesen,
das äußerste zuerst, und an den System-Prompt angehängt — die eigenen Regeln eines Repositorys
erreichen das Modell also, ohne in die Unterhaltung eingefügt zu werden. `/init` schreibt eine
Startdatei.

## Anbieterfehler

Ein serverseitiger oder Transportfehler wird nach zwei Sekunden einmal wiederholt; eine abgelehnte
Anfrage (4xx) nicht. Wenn das Primärmodell weiter scheitert, benennt `fallback_model` in
`settings.json` ein Modell desselben Anbieters, das einmal versucht wird, bevor die Runde
aufgegeben wird, und der Wechsel wird im Transkript vermerkt.

## Skills

Ein Skill ist ein Verzeichnis mit einer `SKILL.md` darin, in der Form, die Claude Code und opencode
verwenden. Wer eines installiert, macht es zu einem Befehl:

```sh
exp-1 install ~/skills/pdf-tools          # a directory with SKILL.md
exp-1 install notes.md                    # a single markdown file
exp-1 install tools.zip                   # an archive (unzip or tar)
exp-1 install https://example.com/skill.md
```

Die Dateien landen in `~/.config/exp-1/skills/<name>/`, benannt nach dem `name:`-Feld des
Frontmatters, wenn es eines hat, sonst nach dem Datei- oder Verzeichnisnamen. Installierte Skills
erscheinen neben den eingebauten Befehlen im Vervollständigungsfenster und in der Hilfe des leeren
Zustands, und `/<name>` führt einen aus: Der Server liest die Anweisungen des Skills, schreibt sie
als Aufgabe in die Unterhaltung und lässt das Modell ihnen folgen. `description:` aus dem Frontmatter
zeigt die Auswahl.

## MCP-Server

Werkzeuge von [MCP](https://modelcontextprotocol.io)-Servern werden dem Modell neben den
eingebauten angeboten. Die Liste liegt in `~/.config/exp-1/mcp.json`, in genau der Form, die
opencode verwendet, ein Eintrag lässt sich also zwischen beiden kopieren:

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

- `local`-Server werden gestartet und über stdio angesprochen; `remote`-Server werden über HTTP
  erreicht, mit JSON- oder SSE-Antwort. `"disabled": true` überspringt einen Server, ohne ihn zu
  löschen, und `"headers"` ergänzt, was ein Remote-Server braucht.
- Werkzeuge erreichen das Modell als `<server>_<tool>`, beide Hälften bereinigt, so wie opencode sie
  bereinigt (`context7_resolve-library-id`), zwei Server können also nie kollidieren.
- Die Server werden beim Start **im Hintergrund** verbunden: Ein langsamer oder toter hält den
  Agenten nie auf, und er wird gemeldet statt fallen gelassen — `/mcp` listet jeden Server mit
  seiner Werkzeugzahl oder dem Fehler, an dem er gescheitert ist.

## Hintergrundserver

Nach dem Vorbild des Daemons von opencode. **Ein normaler Start von `exp1` bringt den Server hoch
und hängt sich an ihn**, genau wie der Standardbefehl von opencode `daemon.transport()` nimmt und an
die UI übergibt.

Die Folge ist der Punkt: Das Schließen der UI lässt den Server weiterlaufen, der nächste Start nutzt
also einen warmen Prozess wieder. Ein Server wird nur wiederverwendet, wenn er **sowohl** dieselbe
Version als auch derselbe Build ist: `server.json` hält Größe und Änderungszeit der ausführbaren
Datei fest, ein seit dem Serverstart neu gebauter Client ersetzt ihn also, statt ein Protokoll zu
sprechen, das der Server nicht mehr kennt. Auf dieser Maschine gemessen:

| | Kaltstart | nächster Start |
| --- | --- | --- |
| UI-Start → erster Frame | 189 ms | **84 ms** |
| UI-RSS | 14,4 MB | 16,0 MB |
| Server-RSS | 15,2 MB | 20,5 MB (nach einer Runde) |

Erneut gemessen mit fünf gleichzeitig geöffneten Sitzungen:

| | ein Server + 5 UIs | fünf eigenständige Prozesse |
| --- | --- | --- |
| Leerlauf | 97 MB (15 + 5 × 16) | 183 MB (5 × 37) |
| mit dem mitgelieferten Titler | 143 MB | 423 MB (5 × 85) |

Der Server lädt träge, was er lädt: Der Codex-Client erscheint mit der ersten Runde (+5 MB), und der
mitgelieferte Titler nur, wenn tatsächlich lokal ein Titel erzeugt wird (+48 MB, weshalb `auto` das
Modell bevorzugt). Diese Trägheit ist der eigentliche Punkt — die 48 MB Gewichte werden einmal für
alle Sitzungen der Maschine bezahlt statt einmal pro Prozess.

Der teure, einmal pro Maschine anfallende Zustand — das Werkzeugregister und das ~49 MB große
Titlermodell — lebt in dem einen Server, und der Codex-Client wird gebaut, wenn eine Anfrage im
Codex-Protokoll ihn zum ersten Mal braucht, eine Maschine, die bei irgendeinem anderen Anbieter
angemeldet ist, startet den Server also ohne Codex-Zugangsdaten. Jede UI ist ein dünner Client, N
parallele Sitzungen kosten also `1 x 63 MB + N x 15 MB` statt `N x 61 MB`, und nur der erste Start
zahlt einen Kaltstart.

```sh
exp1 serve                 # run the server in the foreground
exp1 service start         # spawn it detached (survives the launcher)
exp1 service status        # pid, url, version, health
exp1 service stop          # SIGTERM, then SIGKILL after a grace period
exp1 service restart
exp1 service password      # print, or set with an argument
```

- **Registrierung**: `~/.local/state/exp-1/server.json` hält `{id, version, url, pid}`. Ein Client
  liest sie, prüft `/health` mit 2 s Timeout und verwendet den Server nur wieder, wenn auch die
  Version passt — ein veralteter oder fremder Server wird zuerst gestoppt.
- **Passwort**: `~/.local/state/exp-1/password`, 32 zufällige Bytes, Modus `0600`, über Neustarts
  hinweg erhalten, damit ein neu gestarteter Client sich ohne weitere Angaben wieder verbindet.
- **Abgekoppelter Start**: `exp1 serve --register` in eigener Prozessgruppe mit geschlossenen
  Standardströmen, sodass er den Client überlebt, der ihn gestartet hat.

### HTTP-API

Alle Routen erfordern `Authorization: Bearer <password>`.

| Methode | Pfad | Zweck |
| --- | --- | --- |
| `GET` | `/health` | Lebenszeichen und Version |
| `GET` | `/sessions` | laufende Sitzungen auflisten |
| `POST` | `/sessions` | eine anlegen; `x-exp1-directory` setzt ihr Arbeitsverzeichnis |
| `GET` | `/sessions/{id}` | vollständige Momentaufnahme |
| `POST` | `/sessions/{id}/prompt` | eine Runde starten |
| `GET` | `/sessions/{id}/events` | SSE: snapshot, deltas, tools, usage, title, done, idle |
| `POST` | `/sessions/{id}/answer` | eine ausstehende `ask_question` beantworten |
| `POST` | `/sessions/{id}/interrupt` | die laufende Runde abbrechen |
| `GET` | `/models` | der Katalog mit den echten Aufwandsstufen jedes Modells. `?all=1` ignoriert den Umfang: Die Auswahl, die den Umfang setzt, braucht die Modelle außerhalb davon |

Eine Sitzung führt ihr eigenes Arbeitsverzeichnis mit, ein Server bedient also mehrere Projekte; das
Verzeichnis kommt mit `POST /sessions` statt beim Serverstart festgelegt zu werden.

## Automatische Komprimierung

Eine lange Unterhaltung wird zusammengefasst, bevor sie das Kontextfenster überlaufen kann. Nach
jeder Runde vergleicht der Server die `input_tokens` der Anfrage mit dem `context_window` des
Modells aus dem Codex-Katalog und komprimiert, sobald die Unterhaltung **95 %** davon erreicht:

```
Context at 7378 of 258400 tokens (95% threshold) — compacting
Conversation compacted; continuing from the summary
```

Die Komprimierung schickt die Unterhaltung mit einem Zusammenfassungs-Prompt und dem strukturierten
Template, das opencode verwendet, an das Modell — Objective, Important Details, Work State
(Completed / Active / Blocked), Next Move, Relevant Files — wobei Werkzeugausgaben auf 2000 Zeichen
gekürzt werden, damit der Zusammenfasser nicht ertrinkt.

Die umgeschriebene Unterhaltung ist **die Zusammenfassung, gefolgt von den letzten fünf
Nutzerrunden im Wortlaut**, jeweils mit den Werkzeugaufrufen des Agenten und ihren Ergebnissen,
damit der gerade verfolgte Faden nicht verloren geht:

```
[user]      Summary of everything so far: ## Objective …
[user]      Turn two: run shell ["seq","1","900"] and reply OK.
[call]      shell
[result]    exit_code: 0 1 2 3 …
[assistant] OK
… four more turns …
```

Eine Tatsache, die vor der Komprimierung genannt wurde, ist danach noch bekannt. Ist eine dieser
Runden selbst riesig, werden die ältesten Einträge am Ende verworfen, bis das Ergebnis wieder ins
Fenster passt, eine einzelne riesige Werkzeugausgabe kann die Unterhaltung also nicht endlos neu
komprimieren.

Der Anteil ist über `EXP1_COMPACT_PERCENT` einstellbar, wenn du einen anderen möchtest, womit sich
der Pfad auch prüfen lässt, ohne ein Fenster mit 272 000 Token zu füllen.

Eine Unterhaltung behält bis zu 5 000 Transkriptnachrichten auf der Platte; der eigene Kontext des
Modells wird getrennt durch Beschneiden und Komprimierung begrenzt, eine lange Sitzung bleibt also
lesbar, ohne Token zu kosten.

## Persistenz

Die Konfiguration liegt in `~/.config/exp-1/` (oder `$XDG_CONFIG_HOME/exp-1`); die Unterhaltungen
selbst liegen in `~/.cache/exp-1/sessions/` (oder `$XDG_CACHE_HOME/exp-1/sessions`), denn ein
Transkript ist ein Cache: eines zu verlieren kostet Kontext, nicht Einrichtung. Die eigenen Dateien
des Daemons — `server.json` und `password` — liegen in `~/.local/state/exp-1/`.

| Datei | Enthält |
| --- | --- |
| `config/settings.json` | Modell, Denkaufwand, Farbschema, Titlermodus, Ausweichmodell |
| `config/credentials.json` | ein API-Schlüssel je Anbieter, Modus `0600` |
| `config/prompts/*.md` | die System-Prompts, editierbar: `system`, `subagent`, `compaction`, `title`, `plan` |
| `config/permissions.json` | die allow/ask/deny-Regeln |
| `config/mcp.json` | die MCP-Server |
| `config/skills/` | installierte Skills, je ein Verzeichnis |
| `cache/sessions/<id>.json` | eine Unterhaltung: Titel, beide Transkripte, modellseitige Historie, Prompt-Historie |

Eine Unterhaltung behält ihre neuesten **250 Nachrichten** auf der Platte, und der Cache als Ganzes
wird beim Serverstart auf **250 MB** gekürzt — die ältesten Dateien zuerst. Ein älteres Layout, in
dem alles in `config/exp-1/sessions/` lag, wird beim ersten Start nach der Änderung migriert.

Einstellungen werden beim Beenden der App geschrieben; eine Sitzung wird nach jeder abgeschlossenen
Runde geschrieben, ein Absturz kostet also höchstens die laufende Runde.

Der mitgelieferte Titler ist ein optionales Feature: `cargo build --release --no-default-features`
lässt ihn weg und nimmt die ganze Candle-Abhängigkeit mit, und Sitzungstitel kommen dann allein vom
Modell. Die CI baut so, weil der Titler eine benachbarte Pfad-Abhängigkeit ist.

Die Prompts sind Dateien, keine Konstanten: `~/.config/exp-1/prompts/system.md` (die eigenen
Anweisungen des Agenten), `subagent.md`, `compaction.md` und `title.md` werden beim ersten Start
geschrieben und bei jeder Anfrage gelesen, eine Änderung wirkt also ab der nächsten Runde ohne
Neubau. Die Auflösungsreihenfolge für Modell und Aufwand beim Start ist **Kommandozeile →
gespeicherte Einstellungen → das erste gelistete Modell des Katalogs**.

**Jeder Lauf beginnt mit einer leeren Sitzung.** Nichts wird automatisch fortgesetzt — `/sessions`
öffnet eine Auswahl gespeicherter Unterhaltungen, die neueste zuerst, jede mit ihrem erzeugten
Titel, ihrer ID und der letzten Nutzungszeit. Die Liste richtet sich nach dem Verzeichnis: Eine
Sitzung ist dort sichtbar, wo sie gestartet wurde, überall darunter und überall, wenn sie in
`$HOME` gestartet wurde (die „globalen"). `$HOME` selbst listet nur die globalen, es wird also kein
Katalog jedes Projekts. Die Wahl einer davon speichert die Unterhaltung, die du verlässt, und lädt
die gewählte. `/clear` verwirft die aktuelle Unterhaltung und beginnt eine frische, wobei deine
Einstellungen erhalten bleiben.

### Sitzungstitel

Titel kommen vom mitgelieferten **Titler** — einem lokalen T5 (7,6 Mio. Parameter, 3 Encoder- und
3 Decoder-Schichten), der über Candle in diesem Prozess läuft und als Pfad-Abhängigkeit aus
`../mygame/titler` hereingezogen wird. Die Gewichte (`Cyronius/titler`) werden aus dem
Hugging-Face-Cache gelesen und höchstens einmal pro Prozess geladen, sobald sie auf der Platte
sind, ist die Titelerzeugung also lokal, sofort und kostenlos.

Die Erzeugung läuft in einem blockierenden Thread, sobald die erste Runde endet, nie im UI-Thread.
Sitzungen mit einem Elternteil — der Neben-Chat — werden überhaupt nie benannt, da sie zu der
Unterhaltung gehören, der sie folgen. Es wird der mehrsprachige Pfad der Bibliothek verwendet (nicht
der rohe), die Sprache wird also erkannt und ein nicht-englischer Prompt zuerst ins Englische
übersetzt. Sitzungen werden einmal benannt, und bis ein Titel eintrifft, zeigt die Auswahl die aus
dem Prompt genommene Vorschau.

**Wer eine Sitzung benennt, ist eine Einstellung**, `titler` in `settings.json`:

| Wert | Verhalten |
| --- | --- |
| `auto` (Standard) | zuerst das Codex-Modell, der mitgelieferte Titler als Ausweichlösung |
| `local` | nur der mitgelieferte Titler — keine API-Kosten, schwächere Titel |
| `remote` | nur das Codex-Modell |

Titel werden von der günstigen `luna`-Stufe des Katalogs erfragt statt vom eigenen Modell der
Sitzung — dieselbe Idee, wie opencode einen Anbieter nach seinem kleinen Modell fragt —, und die
Token, die ein Titel kostet, werden wie jeder andere Aufruf zum Verbrauch der Sitzung addiert.

`/retitle` benennt die Sitzung erneut, diesmal aus der ursprünglichen Anfrage plus der jüngsten
Unterhaltung (begrenzt auf 8 000 Zeichen), so wie opencode einen Titel neu erzeugt, wenn eine
Sitzung auf einen leeren Namen umbenannt wird. Der Prompt selbst liegt in `prompts/title.md` und ist das
vollständige Dokument im opencode-Stil: eine Aufgabe, die Regeln (dieselbe Sprache wie der Nutzer,
≤50 Zeichen, exakte Begriffe und Zahlen beibehalten, niemals die Frage beantworten) und zehn
ausgearbeitete Beispiele.

`auto` setzt absichtlich auf das Codex-Modell. Der Titler mit 7,6 Mio. Parametern ist ehrlich über
seine Größe: Mit denselben russischen Prompts erzeugte er `Understanding Two Plus Terms` für
«сколько будет два плюс два» und `Request for Sign` für eine Tabellenanfrage, weil Googles
kostenloser Übersetzer «табличку» als „sign" wiedergibt und das kleine Modell dann die
Fehlübersetzung betitelt. Das Codex-Modell benennt dieselben Prompts `Сколько будет два плюс два?`
und `Формула корней квадратного уравнения в LaTeX`. Für diesen LLM-Pfad ist `prompts/title.md` da
— das mitgelieferte Modell nimmt die erste Nachricht unverändert und ignoriert die Datei völlig.

## Aufbau

| Datei | Rolle |
| --- | --- |
| `src/auth.rs` | Laden der Zugangsdaten, JWT-Ablaufprüfung, Auffrischen, atomares Speichern |
| `src/store.rs` | Einstellungs- und Sitzungsdateien unter `~/.config/exp-1/sessions/` |
| `src/catalog.rs` | Katalog von Modellen und Denkaufwand aus dem Codex-Cache |
| `../mygame/titler` | Pfad-Abhängigkeit: der lokale T5, der Sitzungen benennt |
| `src/protocol.rs` | Übertragungstypen für Anfrage und Antwort |
| `src/md.rs` | Markdown und Pipe-Tabellen zu gestalteten Textläufen |
| `src/latex.rs` | LaTeX-Mathematik zu Unicode |
| `src/api.rs` | HTTP-Transport, SSE-Dekodierung, Aufbau der Header |
| `src/tools.rs` | `read`, `write`, `edit`, `shell`, `ast`, `ask_question`, dazu das Hintergrund-/Ausgaberegister |
| `src/app.rs` | Threads, TUI-Zustand, Darstellung, Momentaufnahme/Wiederherstellung der Sitzung |
| `src/main.rs` | CLI, Tastenverarbeitung, Agentenschleife |

## Grenzen

Eine Runde umfasst höchstens 200 Werkzeugrunden; erreicht sie die, sagt das Transkript es, statt
still stehenzubleiben.

**Token-Ökonomie.** Anfragen bleiben absichtlich klein: `read` arbeitet seitenweise, `grep` begrenzt
seine Treffer, eine lange Befehlsausgabe behält beide Enden, der Bericht eines Subagenten wird bei
6 000 Zeichen abgeschnitten, und vor jeder Anfrage werden die ältesten Werkzeugausgaben — außerhalb
der letzten zwei Runden — durch eine einzeilige Notiz ersetzt, die sagt, was verworfen wurde. Das
Modell kann jedes Werkzeug erneut ausführen für alles, was es noch braucht, und das Transkript, das
der Nutzer sieht, behält den vollständigen Text.

**Parallele Aufrufe.** Eine Modellnachricht kann mehrere Werkzeuge auf einmal anfordern. Folgen von
nur lesenden Aufrufen (`read`, `grep`, `count`, `ast`, `fetch`, `websearch`) laufen gleichzeitig;
alles, was schreibt, behält seine Reihenfolge, ein Schreiben gefolgt von einem Lesen derselben Datei
kann also nicht umsortiert werden.

Dies ist ein MVP. Bekannte Lücken:

- Vertex akzeptiert nur die Application-Default-Zugangsdaten von gcloud; ein Dienstkonto-Schlüssel,
  der ein signiertes JWT erfordert, wird nicht erzeugt.
- Der Neben-Chat sieht bei jeder Frage die ganze Hauptunterhaltung — die Haupt-Historie wird
  gelesen, nie geschrieben —, eine `/btw`-Antwort hat also vollen Kontext, während der Haupt-Thread
  unberührt bleibt.
- Kein Abbruch des Streamings außer dem Abbrechen der Aufgabe.
- Befehle laufen ohne Sandbox.

## Geprüft gegen

Protokolldetails wurden gegen die Quelle von `openai/codex` geprüft und mit Live-Anfragen
bestätigt: Endpunkt und Header, die Refresh-Rotation, Text-Streaming, ein `function_call`-Rundlauf
und eine Werkzeugschleife mit mehreren Iterationen, die eine Datei geschrieben hat.

## Danksagung

- **Diktat** läuft auf dem Sprachmodell **whistle** und der Engine **Needle** von Cactus
  (`cactus-needle`, Apache-2.0): die Produktseite ist [cactuscompute.com/whistle](https://cactuscompute.com/whistle),
  die Gewichte und Plattform-Engines liegen auf [Hugging Face](https://huggingface.co/Cactus-Compute/whistle).
  Die Engine lädt sie nach `~/.cache/cactus-needle`; nichts davon ist in diesem Repository enthalten.
- **Der optionale lokale Titler** baut auf [Candle](https://github.com/huggingface/candle) von Hugging Face,
  unter Apache-2.0 oder MIT.
- Alles Übrige sind die Crates aus `Cargo.toml`, jede unter ihrer eigenen Lizenz.
