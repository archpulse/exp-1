[English](README.md) · [Русский](README.ru.md) · [Українська](README.uk.md) · **Čeština** · [Español](README.es.md) · [Deutsch](README.de.md) · [中文](README.zh.md) · [日本語](README.ja.md)

# exp-1 agent

<img src="assets/exp-1-logo.png" alt="exp-1 logo" width="96" />

Minimální kódovací agent v Rustu s terminálovým UI, který mluví s **27 poskytovateli** a každému předává seznam modelů, které skutečně obsluhuje: katalog přichází živě z models.dev, zatímco tabulka v binárním souboru slouží jako offline záloha. Pět přenosových protokolů pokrývá je všechny — API **OpenAI chat-completions**, které sdílí tucet dodavatelů, **OpenAI Responses**, **Anthropic messages**, nativní **Gemini `generateContent`** od Vertex AI a backend Codex — a poskytovatel se přihlašuje API klíčem, stránkou s kódem zařízení, vlastním přihlášením dodavatele nebo přihlašovacími údaji, které stroj už má. Cesta Codexu přišla první a pořád je jednou z nich: ChatGPT OAuth proti `auth.openai.com`, s požadavky na `https://chatgpt.com/backend-api/codex/responses`.

## Co umí

- Streamuje odpovědi z koncového bodu Codex Responses (SSE, parsované ručně).
- Běží smyčku agenta — požadavek → stream → spuštění nástroje → vrácení výstupu zpět → opakování — dokud model nepřestane žádat o nástroje.
- Drží dvě vlákna: hlavní konverzaci a vedlejší vlákno pro otázky, které ji nesmí přerušit.
- Zobrazuje uvažování, streamovaný text asistenta, volání nástrojů a jejich výstup v jednom přepisu, s délkou každého tahu a časem.

## Nástroje

| Nástroj | Argumenty | Poznámky |
| --- | --- | --- |
| `read` | `path`, `limit?`, `pos?` | Ve výchozím stavu nejvýše 400 řádků; `limit` to změní, `pos` je první řádek, počítáno od 1. Výstup je očíslovaný a uříznuté čtení řekne, jak pokračovat. Soubor o 3000 řádcích se schválně prochází po částech: spolknout ho celý stojí desítky tisíc tokenů. |
| `write` | `path`, `content` | Vytváří nadřazené adresáře; přepíše existující soubor. |
| `edit` | `path`, `old`, `new`, `edits?` | Přesná náhrada, vše nebo nic. Neúspěch pojmenuje nejbližší řádky nebo pozice shod, takže neshoda v bílých znacích nestojí další čtení. `edits` vezme několik dvojic `{old, new}` pro jeden soubor v jednom volání. Výsledek ukáže, co se změnilo. |
| `fetch` | `url`, `limit?`, `pos?` | Jedna stránka jako prostý text, stránkovaná jako soubor: `limit` řádků (výchozích 400) od `pos` (počítáno od 1). Skripty a styly se zahazují, entity dekódují. |
| `count` | `path`, `refresh?` | Řádky v souboru, nebo ve všech souborech pod adresářem, bez čtení obsahu. Kešuje se po souborech v `.cache/count`, podle velikosti a času změny. |
| `grep` | `pattern`, `path?`, `include?`, `max?` | Hledání regulárním výrazem v souboru nebo stromu, aniž by se soubory načetly do konverzace. `include` filtruje podle názvu souboru (`*.rs`). Přeskakuje `.git`, `.cache`, `target`, `node_modules`; nenásleduje symbolické odkazy; ignoruje netextové soubory. Omezeno na `max` shod (výchozích 100). |
| `delete` | `path` | Odstraní soubor. Adresáře odmítá — vyprázdnit strom patří do `shell`, kde je vidět celý příkaz. |
| `shell` | `command`, `background?`, `limit?`, `output_id?` | Běh v popředí blokuje agenta, dokud příkaz neskončí; po 60 s se příkaz přesune na pozadí a agent dostane zprávu, až skončí. `background: true` vrátí id okamžitě. Příkazy na pozadí se nikdy nezabíjejí podle časovače. Výstup je omezen na 30 000 bajtů: delší si ponechá začátek a konec, mezera je označena. `output_id` znovu přečte kterýkoli dřívější běh s jiným `limit`. |
| `ast` | `path?`, `refresh?` | Strom projektu ve formátu `tree`. Kešuje se v `.cache/ast`; `refresh: true` ho přestaví. `.git` se přeskakuje. |
| `websearch` | `query`, `numResults?`, `type?`, `livecrawl?`, `contextMaxCharacters?` | Hledání na webu s obsahem stránek. Používá hostovaný MCP od Exa na `mcp.exa.ai`, který **nepotřebuje žádný API klíč**; nastavením `EXA_API_KEY` se zvednou limity. `type` je `auto`/`fast`/`deep`. |
| `ask_question` | `question`, `options?`, `label?`, `submit?` | Zablokuje tah a zobrazí panel s otázkou. `↑↓` vybírají možnost, `Tab` přechází mezi otázkami, `Enter` potvrzuje, `Esc` zavírá. Cokoli napsaného se bere jako volná odpověď a má přednost před zvýrazněnou možností. |

Příkazy shellu běží bez sandboxu jako uživatel. Příkaz v popředí se po 60 sekundách přesune na pozadí, příkaz na pozadí nikdy nezabije časovač a výstup je omezen na 30 000 bajtů: delší výsledek si ponechá začátek a konec a mezera je označena jako vynechaná.

Nástroj `web` hledá na webu: nejprve hostovaný MCP od Exa, protože vrací obsah stránek, a když sdílená bezplatná úroveň Exa narazí na limit, HTML stránku DuckDuckGo. `EXA_API_KEY` přepne Exa na váš vlastní plán a je to jediné nastavení, které obě cesty potřebují.


## Platformy

Agent je jedna kódová základna pro Linux, macOS a Windows: TUI, smyčka agenta, poskytovatelé, MCP, dovednosti, relace a přibalený titler jsou obyčejný Rust a běží všude. Části, které se dotýkají operačního systému, mají větev pro každou platformu:

| | Linux | macOS | Windows |
| --- | --- | --- | --- |
| TUI, agent, poskytovatelé, MCP, dovednosti, relace | ano | ano | ano |
| Otevírání přihlašovací stránky | `xdg-open` | `open` | `cmd /C start` |
| Živost démona a zastavení | `kill` 0 / SIGTERM→SIGKILL | totéž | `tasklist` / `taskkill /T` pak `/F` |
| Server odpoutaný od své konzoly | `process_group(0)` | totéž | `DETACHED_PROCESS \| CREATE_NEW_PROCESS_GROUP` |
| Adresáře konfigurace a stavu | `$XDG_*`, `~/.config` | `$XDG_*`, `~/.config` | `%APPDATA%`, `%LOCALAPPDATA%` |
| Obrázek ze schránky (`ctrl + v`) | `wl-paste`, `xclip` | `pngpaste` | PowerShell + `System.Windows.Forms` |
| Oznámení o dokončení tahu | `notify-send` | `osascript` | bublinkové oznámení PowerShellu |
| Instalace archivů (`exp-1 install *.zip`) | `unzip` / `tar` | totéž | `tar` (bsdtar čte zip) |
| Místní MCP servery s názvem `npx` | jak jsou napsané | jak jsou napsané | znovu zkoušeny jako `npx.cmd` |

Unixové i Windowsové větve démona a schránky byly ověřeny kompilací díky tomu, že se na Linuxu dočasně aktivovaly; `creation_flags` je jediné volání jen pro Windows, které bez Windows cíle zkontrolovat nelze, a nic zde neběželo na skutečném Windows ani macOS.

### Křížové sestavování z Linuxu

Windowsový binární soubor se na Archu sestaví s nástrojovým řetězcem mingw (to jednou potřebuje roota):

```sh
sudo pacman -S --needed mingw-w64-binutils mingw-w64-gcc
rustup target add x86_64-pc-windows-gnu
cargo build --release --target x86_64-pc-windows-gnu
```

`dlltool` z `mingw-w64-binutils` je to, co importní knihovny potřebují; bez něj se sestavování zastaví na `windows-sys`.

Bez roota vznikne stejný binární soubor z tarballu llvm-mingw rozbaleného pod `~/.local`. Rust std pro tento cíl žádá linker o `libgcc.a`, který llvm-mingw nedodává, takže dvousymlinková vrstva dodá ekvivalenty a `--no-default-features` vynechá přibalený titler stejně jako CI:

```sh
mkdir -p ~/.local/mingw-shim
ln -sf ~/.local/llvm-mingw/lib/clang/*/lib/windows/libclang_rt.builtins-x86_64.a ~/.local/mingw-shim/libgcc.a
ln -sf ~/.local/llvm-mingw/x86_64-w64-mingw32/lib/libunwind.a ~/.local/mingw-shim/libgcc_eh.a
PATH="$HOME/.local/mingw-shim:$HOME/.local/llvm-mingw/bin:$PATH" \
CARGO_TARGET_X86_64_PC_WINDOWS_GNU_LINKER="$HOME/.local/llvm-mingw/bin/x86_64-w64-mingw32-clang" \
RUSTFLAGS="-L $HOME/.local/mingw-shim" \
cargo build --release --target x86_64-pc-windows-gnu --no-default-features
```

`cargo` a `rustup` z `~/.cargo/bin` znají křížové cíle; systémové balíčky ne, a na tomto stroji není `~/.cargo/bin` v `PATH`, takže je volejte úplnou cestou.

Binární soubor pro macOS také nepotřebuje roota a s `cargo zigbuild` se celý strom sestaví pro `aarch64-apple-darwin`: `zig cc` křížově přeloží C závislosti (včetně `aws-lc-sys`, což obyčejný `cargo check` nezvládne — hostitelský `cc` odmítne `-arch arm64`) a přeloží se každý crate. Zastaví to až poslední krok: funkce `rustls` v `reqwest` 0.13 táhne `rustls-platform-verifier` a linker pak hlásí `unable to find framework 'Security'` a `'CoreFoundation'`. Ty žijí jen v Apple SDK; Zig bez problémů linkuje Mach-O a libSystem, frameworky však nedodává. Binární soubory pro macOS tedy přicházejí z Macu nebo z CI (`.github/workflows/release.yml` sestavuje obě architektury). Postup níže je ten, který se dostane až k linkování:

```sh
mkdir -p ~/.local/zig && curl -L https://ziglang.org/download/0.14.1/zig-x86_64-linux-0.14.1.tar.xz \
  | tar -xJ -C ~/.local/zig --strip-components=1
cargo install cargo-zigbuild
rustup target add aarch64-apple-darwin
PATH="$HOME/.local/zig:$PATH" cargo zigbuild --release --target aarch64-apple-darwin
```

Výsledek je nepodepsaný. Na Apple Silicon se nepodepsaný binární soubor při spuštění zabije, takže se musí jednou podepsat přímo na Macu:

```sh
codesign -s - --force ./exp1-agent        # ad-hoc signature
xattr -cr ./exp1-agent                    # if it arrived by download
```

## Požadavky

- Rust 1.85+ (edice 2024)
- Buď existující přihlášení v Codex CLI (jeho `auth.json` se znovu použije), nebo `/login`, které samo spustí stejný tok s kódem zařízení.
- Přibalený titler, zkompilovaný do agenta ze `src/titler.rs` pod funkcí `titler`. Táhne Candle, a proto první sestavení chvíli trvá a binární soubor má ~16 MB místo ~7 MB. Jakmile jsou váhy v keši Hugging Face, pojmenovávání probíhá offline.

## Instalace

K instalaci nejnovější verze z GitHubu potřebujete Rust 1.85+ a Cargo:

```sh
cargo install --git https://github.com/archpulse/exp-1.git --locked exp1-agent
exp1-agent --help
```

Cargo nainstaluje binární soubor `exp1-agent` do `~/.cargo/bin`; v případě potřeby přidejte tento adresář do `PATH`. Výchozí sestavení obsahuje lokální titler, takže první instalace trvá déle kvůli kompilaci Candle.

## Použití

```sh
cargo run --release                      # uses the current directory
exp1-agent --model gpt-6-luna --effort high # pick a model and reasoning effort
exp1-agent --cwd ~/projects/my-app          # run the shell tool somewhere else
exp1-agent --help
```

| Klávesa | Akce |
| --- | --- |
| `Enter` | Odešle prompt, nebo spustí příkaz, když už je hotový |
| `↑` `↓` | Posouvá výběr v dokončení, nebo si vybavuje předchozí prompty |
| `Tab` | Přijme zvýrazněné dokončení |
| `Ctrl + /` | Přepíná mezi hlavním a vedlejším vláknem |
| `Ctrl + Shift + A` | Přepne režim oprávnění: always ask → never ask → ask only commands |
| `Esc` | Sám o sobě nic nedělá |
| `Esc` `Esc` | Zruší běžící tah, nebo zavře seznam dokončení či výběr |
| `Ctrl-C` | Ukončí aplikaci |
| `Ctrl-U` | Vymaže vstupní řádek |
| `Ctrl-Backspace` | Smaže předchozí slovo |
| `Ctrl-Delete` | Smaže následující slovo |
| `Ctrl-W`, `Alt-Backspace` | Totéž jako `Ctrl-Backspace` |
| Kolečko myši | Roluje přepis |
| `PgUp` / `PgDn` | Rolují přepis |
| `←` `→` `Home` `End` | Pohybují kurzorem |
| `Backspace` / `Delete` | Upravují text |

Šipky procházejí historii promptů: po odeslání `hi` vrátí `↑` tento text zpět do pole, další stisky jdou dál dozadu a `↓` jde vpřed až k prázdnému poli. Roluje se kolečkem myši, takže šipky zůstávají volné pro historii.

Napsání `/` otevře seznam dokončení filtrovaný podle předpony. Po mezeře dokončuje i argumenty příkazu — `/themes <tab>` vypíše schémata, `/model <tab>` modely a `/model gpt-6-luna <tab>` úrovně úsilí uvažování. `Enter` na samotném `/model` nebo `/themes` místo toho otevře očíslovaný seznam pro výběr; `/model` má dva kroky, nejprve model a pak úsilí. U prázdného pole sedí kurzor úplně na začátku, před zástupným textem.

Řádek nápovědy se mění podle kontextu. Během tahu je vstup uzamčen.

`Esc` je schválně nečinný; přerušuje až dvojí stisk. Přerušení zruší probíhající požadavek a zahodí částečný text asistenta, historii konverzace přitom nechá nedotčenou.

## Příkazy se lomítkem

Zpracovávají se lokálně; nikdy se nedostanou k modelu.

| Příkaz | Účinek |
| --- | --- |
| `/login` | Vybere poskytovatele: u již přihlášeného se jen přepne; výběr právě používaného se přihlásí znovu. Přihlášení kódem zařízení otevře v prohlížeči stránku dodavatele; ostatní si vezmou API klíč |
| `/scoped-models` | Zaškrtne, které modely smí `/model` nabízet; prázdné znamená všechny |
| `/sessions` | Vybere uloženou konverzaci k otevření |
| `/init` | Zapíše do pracovního adresáře kostru `AGENTS.md` |
| `/model` | Otevře dvoukrokový výběr (model, pak úsilí); `/model <name> [low\|medium\|high]` ho nastaví přímo |
| `/themes` | Otevře výběr motivů; `/themes <name>` ho nastaví přímo |
| `/status` | Ukáže model, úsilí, pracovní adresář, nástroje, motiv, plán, poskytovatele přihlášení |
| `/btw` | Odpoví na doplňující otázku ve vedlejším vlákně, zatímco hlavní tah běží dál |
| `/clear` | Vymaže přepis a historii konverzace |
| `/help` | Zobrazí klávesové zkratky |
| `/compact` | Shrne konverzaci hned, místo aby se čekalo, až se okno naplní |
| `/undo` | Vrátí soubory, které změnil poslední tah (v paměti; restart je zapomene) |
| `/diff` | Sjednocený diff změn posledního tahu |
| `/fork` | Pokračuje touto konverzací jako novou relací, původní nechá být |
| `/sessions <text>` | Místo pouhého výpisu prohledá uložené konverzace |
| `/retitle` | Pojmenuje relaci znovu podle původního zadání a nedávné konverzace |
| `/effort` | Vybere úsilí uvažování pro aktuální model, aniž by změnil model |
| `/mcp` | Vypíše MCP servery a nástroje, které nabízejí |

Dostupné motivy: `default` (modrý), `amber`, `jade`, `mono`. Barvy jsou truecolor, takže je motiv terminálu nemůže přemapovat.

## Rozhraní

Rozvržení následuje TUI OpenAI Codex: žádné orámované panely, záhlaví `>_` s pracovním adresářem pod ním, seznam dokončení nebo očíslovaný výběr nad vstupem, vstupní pruh s odsazením, značkou `›` a blikajícím kurzorem `│`, pak řádek s modelem a řádek nápovědy.

- Odeslaný prompt se do přepisu vykreslí zpět jako pruh přes celou šířku, stejně jako to dělá Codex.
- **Výstup nástrojů je ve výchozím stavu skrytý**: přepis nese volání (`read src/main.rs`), ne to, co vrátilo. `ctrl-o` zobrazí výstup pod jeho voláním a zase ho skryje — to je rozdíl mezi sledováním práce a jejím čtením.
- **Myš není jen na rolování.** Kolečko roluje, tažením se vybírá a kopíruje a klik funguje na to, na co padl: řádek výběru nebo seznamu dokončení se chová jako Enter na tom řádku, nápověda `ctrl + / for side` v zápatí přepíná vlákna a odkaz ve zprávě se otevře v prohlížeči. Každé vykreslení si zaznamenává oblasti, které nakreslilo, takže klik je test zásahu proti poslednímu snímku, ne odhad rozvržení. Zvýrazňování při najetí je schválně neimplementované: stálo by překreslení při každém pohybu myši a kurzor terminálu už sám říká, kde ukazatel je.
- Subagent hlásí svůj aktuální krok na **jednom řádku, který se aktualizuje** (`subagent → grep tokenizer`), a ne řádkem na krok; volání, které ho spustilo, zní `task: <description>`.
- **Klik na vaši vlastní zprávu se zeptá, co s ní**, místo aby to udělal: malá nabídka s `copy` (text jde do schránky) a `revert`. Vrácení zahodí všechno za touto zprávou — konverzaci, co se modeli pod tímto bodem řeklo, i každou změnu souborů, kterou následující tahy provedly a kterou log pro vrácení změn obnoví. Zpráva sama z přepisu zmizí a její text se vrátí do promptu, takže se dá upravit a poslat znovu. Text se zkontroluje, než se cokoli zahodí, takže klik, který padne na špatnou zprávu, nic neudělá.
- Přepis se drží u nejnovější zprávy; kolečko myši nebo `PageUp`/`PageDown` roluje historií nahoru, `ctrl-g` skočí zpět k nejnovější zprávě a `ctrl-t` k nejstarší, a zobrazení posunuté nahoru zůstane tam, kde je, i když přicházejí nové řádky, místo aby se táhlo zpět dolů.
- Tažení myší vybírá text kdekoli na obrazovce — včetně záhlaví — a puštění ho dá do schránky: nejprve OSC 52, takže to funguje i přes ssh, pak `wl-copy`, `xclip` nebo `xsel`, když existují. Poznámka `copied N chars` to potvrdí v řádku nápovědy. Výběr je tichý nádech barvy za textem a končí u posledního znaku každého řádku, stejně jako se chová vlastní výběr terminálu. Terminál tažení nikdy nevidí, protože aplikace si myš drží pro rolování, takže je to jediný způsob, jak kopírovat.
- Připojený obrázek se nad promptem zobrazí jako vyplněný štítek `Image 1` s textem `ctrl-v replaces it` vedle, takže je ve vstupním pruhu vidět, co se chystá odeslat.
- Dokud tah běží, řádek s aktivitou sedí **přímo pod promptem, ke kterému patří** — každá činnost má vlastní popisek: `⠋ thinking 4s`, když je požadavek na cestě, `⠋ preparing write… (12 KB) 8s`, když model ještě píše argumenty volání nástroje, `⠋ running 4s`, když nástroj běží. Každý popisek má vlastní hodiny, takže sekundy se týkají toho, co se děje teď, a nikdy nepočítají celý tah; součet tahu hlásí na konci `Worked for`. Spodní lišta zůstává prostým popiskem `GPT-6-Luna high · /path · <thread> · Main [default]`; `esc esc interrupts` bydlí v řádku nápovědy.
- Po dokončení tahu se do přepisu přidá `Worked for 5s · 12:08 PM`.
- Řádek s modelem zní `GPT-6-Luna high · /path · <thread name> · Main [default]`, nebo `… · Agent` ve vedlejším vlákně. Název vlákna se bere z prvního promptu.
- Řádek nápovědy nese vpravo přepínač vláken, červený, když je aktivní vedlejší vlákno.
- Prázdná obrazovka zůstává tichá: úplný seznam příkazů se objeví jen na stroji, který ještě nikdy neuložil konverzaci, a potom už je to jen jeden řádek odkazující na `/` a `/help`.
- Psaní zůstává živé i během tahu, takže otázku přes `/btw` lze položit bez čekání.
- `/btw`, nebo `ctrl + /`, odpovídá ve vedlejším chatu, který si celou hlavní konverzaci nese jako kontext, aniž by do ní něco přidával.
- `shift + tab` přepíná mezi režimem build a plan; dokud je zapnutý, zápatí ukazuje `plan mode`.
- Tah delší než patnáct sekund po dokončení zazvoní na terminálový zvonek (a na `notify-send`, kde ho plocha má), takže dlouhá práce nemusí být hlídaná.
- `shift + enter` zapíše zlom řádku místo odeslání, takže prompt může mít několik řádků; vstupní pruh roste, aby je pojal.
- `ctrl + v` vezme obrázek ze schránky (Wayland `wl-paste`, X11 `xclip`) a připojí ho k dalšímu promptu; v řádku nápovědy se do odeslání ukazuje `▣ image attached`. Obrázek cestuje jako data URL, takže ho předají všechny backendy Codex, OpenAI, Anthropic i Gemini.
- `ask_question` vyvolá panel nad vstupem, zrcadlící nástroj Question: zvýrazňovací pruh po levém okraji, titulek `Questions`, záložky, když se ptá několik otázek společně, očíslované možnosti s vybranou zvýrazněnou, položku `Type your own answer` a dole `⇥tab ↑↓ select enter confirm esc dismiss`.

Barvy jsou truecolor, takže je motiv terminálu nemůže přemapovat. Model, úsilí a motiv se za běhu přepínají přes `/model` a `/themes`.

Vědomé rozdíly proti Codexu: Codex má `/permissions` a `/review`, které potřebují politiku sandboxu a recenzenta diffů; toto sestavení má místo nich `/clear` a `/help`. Zápatí Codexu inzeruje překryvy agenta a zkratek, které toto MVP nemá.

## Autentizace

`/login` vypíše poskytovatele a každý se přihlašuje tak, jak jeho dodavatel očekává:

- **API klíč** pro většinu z nich, odesílaný jako `Authorization: Bearer` (nebo `x-api-key` pro API Anthropic) a uložený po poskytovatelích v `~/.config/exp-1/credentials.json` s právy `0600`. Jeden klíč může pokrýt několik poskytovatelů — `opencode-go` a `opencode-zen` jsou jeden účet za dvěma základními URL a `providers::key_group` pojmenovává skupiny.
- **Přihlášení kódem zařízení** pro `openai (auth)`, `github copilot` a `meta (auth)`: v prohlížeči se otevře přihlašovací stránka dodavatele a URL a kód se vypíšou jako záložní varianta.
- **Token z vlastního přihlášení dodavatele** pro `anthropic (auth)`, odesílaný s identitními hlavičkami oficiálního klienta.
- **Přihlašovací údaje, které stroj už má**, pro `google vertex`: to, co nastavil `gcloud auth application-default login`, přičemž `GOOGLE_CLOUD_PROJECT` a `GOOGLE_CLOUD_LOCATION` pojmenovávají projekt a region. Nic není třeba vkládat.

Cesta Codexu si navíc drží vlastní soubor: při prvním spuštění agent zkopíruje `$CODEX_HOME/auth.json` (výchozí `~/.codex/auth.json`) do `~/.exp1-agent/auth.json` a pracuje s touto kopií. **Vaše skutečné přihlašovací údaje k Codexu se nikdy nepřepisují.** Když přístupovému tokenu zbývá do vypršení 5 minut, agent ho obnoví:

```
POST https://auth.openai.com/oauth/token
grant_type=refresh_token
client_id=app_EMoamEEZ73f0CkXaXp7hrann
```

Odpověď rotuje obnovovací token, takže se nový zapíše zpět do vlastní kopie agenta. Uložený soubor se zapisuje atomicky s právy `0600`.

## Modely a úsilí

Ani seznam modelů, ani úrovně úsilí uvažování nejsou zadrátované. Obě se čtou z vlastního katalogu Codex CLI na `$CODEX_HOME/models_cache.json` (výchozí `~/.codex/models_cache.json`) a u každé položky používají `slug`, `display_name`, `description`, `visibility`, `default_reasoning_level` a `supported_reasoning_levels`. Výchozí hodnota při startu je první uvedený model v tomto katalogu.

**Každý další poskytovatel dostane svůj seznam živě, z models.dev.** Katalog se stahuje nejvýše jednou za dvanáct hodin do `~/.cache/exp-1/models-dev.json`, drží se v paměti a čte se pro tři věci: id modelů, které poskytovatel nabízí, protokol, kterým každý model mluví (models.dev ho zaznamenává jako balíček AI SDK, s nímž je poskytovatel nakonfigurován — `@ai-sdk/anthropic` pro model Claude za bránou, `@ai-sdk/openai` pro model Grok) a kontextové okno, z něhož se počítá práh automatické kompaktace. Poskytovatel, který v katalogu chybí, si nechá tabulku v `src/providers.rs`, a stejně tak stroj bez připojení, takže aplikace na stahování nikdy není závislá.

Model, který tabulkám úplně uniká, protokol stejně dostane: rozhodne předpona výrobce (`claude-*`, `minimax-*` a `qwen-*` mluví Anthropic, `grok-*`, `gpt-*` a `muse-*` mluví Responses, `gemini-*` mluví Gemini, všechno ostatní je chat) a za cokoli dalšího odpovídá poskytovatel, který mluví jediným protokolem pro všechno.

**Výběr ukazuje každého přihlášeného poskytovatele**, jednu polici místo jedné na přihlášení: řádky nesou `via <provider>`, takže se dvě čepice na stejném názvu modelu rozeznají, poskytovatel v použití je první a výběr modelu od jiného poskytovatele se na něj nejdřív přepne a teprve pak model nastaví. Tak dlouhá police se hledá psaním: každý výběr filtruje za psaní, backspace shodu rozšiřuje a Escape rozšíří, než se zavře. Rozsah v `/scoped-models` filtruje tutéž polici a pojmenovává model jednoho poskytovatele (`opencode-go/grok-4.7`), takže stejný název od dvou poskytovatelů jsou dvě položky; starší rozsah s holými názvy stále znamená „tento název, od kteréhokoli poskytovatele“ a `ctrl-a` v tomto výběru rozsah vyčistí, což znovu nabídne všechno. `/model grok-4.7` vezme poskytovatele v použití, když ten název nabízí víc z nich, a `/model opencode-go/grok-4.7` pojmenuje jednoho přesně.

Úrovně úsilí se model od modelu skutečně liší, a proto jejich hádání nefunguje:

| Model | Úsilí |
| --- | --- |
| `gpt-5.6-terra` | low, medium, high, xhigh, max, **ultra** |
| `gpt-6-luna` | low, medium, high, xhigh, max |
| `gpt-5.5` | low, medium, high, xhigh |

Výběr úsilí, dokončování `/model <name> <tab>` i validace se řídí tím, který model je aktivní. Přepnutí modelu úsilí přenese, když ho nový model přijímá, a jinak spadne na `default_reasoning_level` toho modelu, takže nikdy nemůžete skončit s požadavkem na úsilí, které model nemá:

```
/model gpt-5.6-terra ultra   →  GPT-5.6-Terra ultra
/model gpt-5.5               →  GPT-5.5 medium          (ultra is not a gpt-5.5 level)
/model gpt-5.5 max           →  'max' is not a GPT-5.5 level;
                                this model takes low, medium, high, xhigh
```

Ve výběru se objeví jen modely s `visibility: "list"`; skryté zůstávají dostupné podle názvu. Když katalogový soubor chybí, aplikace spadne na konzervativní vestavěný seznam, aby zůstala použitelná, a ve výběru to řekne.

Odpovídat budou jen modely, na které má přihlášený účet ChatGPT nárok — zbytek backend odmítne s vysvětlující chybou.

## Tvar požadavku

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

Zpracovávané události: `response.output_text.delta`, `response.reasoning_summary_text.delta`, `response.output_item.done`, `response.completed`, `response.failed`, `response.incomplete`, `error`. Cokoli jiného se ignoruje.

## Vykreslování

Odpovědi asistenta se vykreslují jako bohatý text přímo v procesu. Není tu žádný externí vykreslovač, žádný `pandoc`, žádná binárka TeX, žádná rasterizace obrázků — všechno se děje uvnitř TUI, takže to nestojí téměř nic.

- **Markdown**: nadpisy, tučné a kurzívové písmo, řádkový kód, ohraničené bloky kódu s linkou a značkou jazyka, odrážkové a číslované seznamy, blokové citace, vodorovná pravítka, odkazy, trubkové tabulky ve stylu GitHubu s rámečky z kreslicích znaků a zarovnáním po sloupcích a řádková matematika `$…$`.
- **Matematika LaTeX**: symboly a relace, řečtina, `\frac`, `\sqrt`, dolní a horní indexy, `\mathbb` a `\mathcal`, kombinované akcenty (`\bar`, `\hat`, `\vec`, …), jména operátorů vzpřímeně (`\log`, `\sin`, `\lim`, …) a zobrazované bloky `$$…$$`.

Co převodník nerozpozná, vypíše doslova, včetně složených závorek, takže neznámý vstup zůstane čitelný, místo aby se rozsypal. Je to převodník, ne TeX engine: makra, prostředí jako `tikz` a víceřádkové rozvržení se nevykládají.

Markdown se parsuje **jednou**, když zpráva dorazí, na běhy nezávislé na motivu; kreslení je jen mapuje na barvy. Přepnutí motivu tedy nic neparsuje znovu.

Změřeno na sestaveném binárním souboru: **~1 % jednoho jádra v klidu** s blikajícím kurzorem, ~3 % při rychlém rolování kolečkem.

## Subagenti a dlouhé příkazy

**Subagenti.** Nástroj `task` spustí druhého agenta s vlastním kontextem a stejnými nástroji a pak vrátí jen jeho zprávu — hodí se pro práci, která by jinak zaplavila kontext volajícího. Zpráva se uřezává na 6 000 znaků, protože přistává v kontextu volajícího, a přepis ukazuje její průběh na jediném řádku, který se při každém kroku nahradí, místo aby každý krok měl vlastní řádek. Vnoření subagenti se nabízejí, dokud zbývá hloubka, nejvýše dvě úrovně, aby se rekurze nemohla rozjet. V přepisu je vidět každý krok, který subagent udělá.

**Pomalé příkazy se přesouvají, nezabíjejí.** Příkaz v popředí, který po **60 sekundách** stále běží, se přesune na pozadí a agent se to dozví, místo aby to blokovalo tah:

```
'sleep 75' is still running after 60s, so it was moved to the background as 'bg-1'.
Carry on; you will be told when it finishes, or poll it with {"output_id": "bg-1"}.
```

**Dokončení přerušuje.** Když příkaz na pozadí skončí, běžící tah se přeruší a agent dostane výstup, takže se výsledek nikdy tiše neztratí za dlouhou prací:

```
• interrupted: a background command finished
```

## Diktování

Tlačítko mikrofonu v dolním rohu (`ctrl-r` dělá totéž) diktuje do promptu: stiskněte, mluvte, stiskněte znovu. Text přichází, když mluvíte, protože rozpoznávač streamuje, a tlačítko během poslechu nese stopky.

Rozpoznávač je **whistle** — hlasový engine běžící na zařízení od Cactus — spouštěný jako `whistle --mic`. Právě tento režim ho nutí streamovat: nahrává syrové mono 16 kHz z pipewire `pw-record` po jedné sekundě, každou sekundu protlačí streamovacím dekodérem a po každé aktualizaci vypíše jeden JSON řádek (`text` je to potvrzené, `pending` to, co je ještě na cestě). Nový řádek na jeho standardním vstupu ukončí proud a vypláchne zbytek. Příkaz, jazyk, zvýhodněná slova a zdroj zvuku přicházejí z prostředí:

Nahrávač je vlastní platformě: `pw-record` (pipewire) na Linuxu, `ffmpeg` s avfoundation na macOS, `ffmpeg` s dshow na Windows (`WHISTLE_SOURCE` vybírá vstupní zařízení, `WHISTLE_RECORDER` příkaz úplně nahradí). Aplikace sama spouští jen `EXP1_DICTATE --mic` a čte jeho JSON řádky, takže funguje jakýkoli rozpoznávač, který streamuje stejně; bez něj stisk tlačítka mikrofonu v zápatí řekne, že žádný není, a nic dalšího se nestane.

```sh
EXP1_DICTATE=/path/to/whistle     # default: whistle on PATH
EXP1_DICTATE_LANG=en              # default: whatever it detects
EXP1_DICTATE_KEYWORDS=country,Americans
WHISTLE_SOURCE=…                  # default: the system's own input
```

Diktování je jen pro Linux, protože nahrávač je; zbytek agenta ne.

Tento režim je místní pro tuto kopii whistle: logika sedí v její knihovně jako typ `Dictation` (`start`, `next`, `finish`) a její `main.rs` je tenká smyčka nad ním. whistle zde nemá vlastní repozitář, takže v čerstvé kopii od jejího autora se ty dva soubory musí znovu aplikovat, nebo se `EXP1_DICTATE` musí ukázat na něco, co mluví stejným protokolem. Rozpoznávač, který `--mic` nezná, se neignoruje: řekne to svými slovy v zápatí.

## Poskytovatelé

`/login` otevře výběr poskytovatelů; `/scoped-models` rozhoduje, které z jejich modelů `/model` nabízí (prázdné znamená všechny a rozsah se ukládá do `settings.json`). Výběr je prostý abecední seznam a kurzor se otevírá na poskytovateli v použití, takže se nic nemusí hledat podle pozice. Výběr už přihlášeného poskytovatele se na něj přepne a řekne to; výběr toho, který se právě používá — nebo spuštění `/login <id>` — se schválně přihlásí znovu. Přihlášení kódem zařízení otevře v prohlížeči stránku dodavatele a jako záložní variantu vypíše URL a kód.

**Jeden klíč může pokrýt několik poskytovatelů.** `opencode-go` a `opencode-zen` jsou jeden účet za dvěma základními URL, takže přihlášení do jednoho uloží klíč pro oba (`providers::key_group` pojmenovává skupiny). Brány opencode také směrují podle relace: každý požadavek nese `x-opencode-session`, `x-opencode-project` a `x-opencode-client`, bez nichž ho brána rovnou odmítne („Request is missing x-opencode-session“).

Vše, co je tu implementováno, pokrývá pět přenosových protokolů: API **OpenAI chat-completions**, které sdílí tucet dodavatelů, **OpenAI Responses**, **Anthropic messages**, nativní **Gemini `generateContent`** od Vertex AI a **backend Codex**, s nímž mluví `openai (auth)`. Všechny streamují SSE a všechny se dekódují do stejného proudu událostí, jaký produkuje klient Codex, takže smyčka agenta neví, se kterým mluví.

Klíče se ukládají po poskytovatelích v `~/.config/exp-1/credentials.json` s právy `0600`. `openai (auth)` si místo toho drží `auth.json` ve stylu Codexu, `google vertex` používá, co už přihlásil `gcloud auth application-default login` (s `GOOGLE_CLOUD_PROJECT` a `GOOGLE_CLOUD_LOCATION`, které pojmenovávají projekt a region), a poskytovatelé s kódem zařízení ukládají token, který dostanou, do stejného souboru s údaji.

| Poskytovatel | Základní URL | Protokol |
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

Každá položka je implementovaná: každý model uvádí protokol, na kterém odpovídá, takže brána jako `opencode-go` může obsluhovat tři najednou. Seznam a základní URL pocházejí z [models.dev](https://models.dev), veřejného registru, který čte samo opencode, a ne z hádání.

Každá základní URL výše byla testována schválně neplatným klíčem: odmítnutí dokazuje, že URL i hlavička s přihlášením jsou správné. Všech dvacet sedm odpovědělo `400`, `401` nebo `403` — kromě `fireworks-ai`, jejíž `404` je vlastní `"Model not found"` nesoucí id požadavku `chatcmpl-`, takže i tento požadavek došel až na inferenční trasu. `google vertex` odpovídá `401` na své cestě s projektem.

## Režimy

`shift + tab` přepíná relaci mezi dvěma režimy a zápatí říká, který je aktivní:

- **build** — všechno, jak je tu popsáno.
- **plan** — nástroje, které mění soubory (`write`, `edit`, `delete`), se odmítají *a* vynechávají ze seznamu nástrojů, takže model místo editování zkoumá a navrhuje plán. `shell` má zůstat jen pro čtení. Text žije v `prompts/plan.md` a dá se upravit jako ostatní.

## Oprávnění

`~/.config/exp-1/permissions.json` je seznam výjimek; volání, které neodpovídá žádnému pravidlu, se provede. Rozhoduje první shoda:

```json
{
  "rules": [
    { "action": "shell", "resource": "rm -rf *", "effect": "deny" },
    { "action": "write", "resource": "/etc/*", "effect": "ask" },
    { "action": "fetch", "resource": "*", "effect": "allow" }
  ]
}
```

`action` je název nástroje nebo `*`, `resource` je to, čeho se volání dotýká — cesta u souborových nástrojů, příkazový řádek u `shell`, URL nebo dotaz u síťových nástrojů — a `*`/`?` fungují jako zástupné znaky. `effect` je `allow`, `ask` nebo `deny`. `ask` vyvolá panel s otázkou a drží volání, dokud se neodpoví (dvě minuty, pak se volání odmítne). Pravidla platí i pro subagenty.

`ctrl + shift + a` přepíná režim oprávnění: **always ask** (výchozí pro čistou instalaci) nechá `write`, `edit`, `delete` a `shell` položit otázku, **never ask** je spustí bez otázky a **ask only commands** se ptá jen u `shell`. Nástroje jen pro čtení se neptají nikdy a pravidlo s `effect: deny` odmítne v každém režimu — režim nikdy nezmírní zákaz.

## Pokyny projektu

`AGENTS.md` se čte z pracovního adresáře a všech nadřazených adresářů nad ním, od toho nejvzdálenějšího, a připojuje se k systémovému promptu — takže vlastní pravidla repozitáře se dostanou k modeli, aniž by se vkládala do konverzace. `/init` zapíše výchozí soubor.

## Selhání poskytovatele

Selhání na straně serveru nebo přenosu se jednou zopakuje po dvou sekundách; odmítnutý požadavek (4xx) ne. Když primární model stále selhává, `fallback_model` v `settings.json` pojmenuje model stejného poskytovatele, který se jednou zkusí, než se od tahu upustí, a přepnutí se poznamená v přepisu.

## Dovednosti

Dovednost je adresář se souborem `SKILL.md`, ve tvaru, který používají Claude Code a opencode. Instalace z ní udělá příkaz:

```sh
exp-1 install ~/skills/pdf-tools          # a directory with SKILL.md
exp-1 install notes.md                    # a single markdown file
exp-1 install tools.zip                   # an archive (unzip or tar)
exp-1 install https://example.com/skill.md
```

Soubory přistanou v `~/.config/exp-1/skills/<name>/` a pojmenují se podle pole `name:` v hlavičce, když ho mají, jinak podle názvu souboru nebo adresáře. Nainstalované dovednosti se objeví vedle vestavěných příkazů v seznamu dokončení a v nápovědě prázdné obrazovky a `/<name>` jednu spustí: server přečte pokyny dovednosti, zapíše je do konverzace jako úkol a nechá model, ať je následuje. `description:` z hlavičky je to, co výběr ukazuje.

## MCP servery

Nástroje z [MCP](https://modelcontextprotocol.io) serverů se modeli nabízejí vedle vestavěných. Seznam žije v `~/.config/exp-1/mcp.json`, přesně ve tvaru, který používá opencode, takže se položka mezi nimi dá zkopírovat:

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

- Servery `local` se spouštějí a komunikuje se s nimi přes stdio; servery `remote` se dosahují přes HTTP, s odpovědí buď JSON, nebo SSE. `"disabled": true` server přeskočí, aniž by ho smazal, a `"headers"` přidá, co vzdálený server potřebuje.
- Nástroje se k modeli dostávají jako `<server>_<tool>`, přičemž obě poloviny se čistí stejně, jako je čistí opencode (`context7_resolve-library-id`), takže dva servery nikdy nemohou kolidovat.
- Servery se při startu připojují **na pozadí**: pomalý nebo mrtvý server nikdy nezdrží agenta a ohlásí se, místo aby se zahodil — `/mcp` vypíše každý server s počtem jeho nástrojů, nebo s chybou, na které selhal.

## Server na pozadí

Vzorované na démonu opencode. **Běžné spuštění `exp1-agent` server nastartuje a připojí se k němu**, přesně jako výchozí příkaz opencode vezme `daemon.transport()` a předá ho UI.

Právě o důsledek tu jde: zavření UI nechá server běžet, takže další spuštění znovu použije zahřátý proces. Server se znovu použije jen tehdy, když má **jak** stejnou verzi, **tak** stejné sestavení: `server.json` zaznamenává velikost a čas změny spustitelného souboru, takže klient znovu sestavený od spuštění serveru ho nahradí, místo aby mluvil protokolem, kterým server už nemluví. Změřeno na tomto stroji:

| | studený start | další spuštění |
| --- | --- | --- |
| spuštění UI → první snímek | 189 ms | **84 ms** |
| RSS UI | 14,4 MB | 16,0 MB |
| RSS serveru | 15,2 MB | 20,5 MB (po jednom tahu) |

Změřeno znovu s pěti otevřenými relacemi najednou:

| | jeden server + 5 UI | pět samostatných procesů |
| --- | --- | --- |
| v klidu | 97 MB (15 + 5 × 16) | 183 MB (5 × 37) |
| s přibaleným titlerem | 143 MB | 423 MB (5 × 85) |

Server je líný v tom, co načítá: klient Codex se objeví při prvním tahu (+5 MB) a přibalený titler jen tehdy, když se název skutečně generuje lokálně (+48 MB, proto `auto` dává přednost modelu). Právě ta lenost je většina pointy — 48 MB vah se platí jednou za všechny relace na stroji, místo jednou na proces.

Drahý stav placený jednou na stroj — registr nástrojů a ~49 MB modelu titleru — žije v jediném serveru a klient Codex se sestaví, když ho poprvé potřebuje požadavek protokolu Codex, takže stroj přihlášený k jakémukoli jinému poskytovateli spustí server bez přihlašovacích údajů Codexu. Každé UI je tenký klient, takže N paralelních relací stojí `1 x 63 MB + N x 15 MB` místo `N x 61 MB`, a jen první spuštění platí za studený start.

```sh
exp1-agent serve                 # run the server in the foreground
exp1-agent service start         # spawn it detached (survives the launcher)
exp1-agent service status        # pid, url, version, health
exp1-agent service stop          # SIGTERM, then SIGKILL after a grace period
exp1-agent service restart
exp1-agent service password      # print, or set with an argument
```

- **Registrace**: `~/.local/state/exp-1/server.json` drží `{id, version, url, pid}`. Klient ho přečte, zkontroluje `/health` s dvousekundovým limitem a server znovu použije jen když sedí i verze — zastaralý nebo cizí server nejdřív zastaví.
- **Heslo**: `~/.local/state/exp-1/password`, 32 náhodných bajtů, práva `0600`, přežívá restarty, takže se nově spuštěný klient připojí znovu, aniž by se mu cokoli říkalo.
- **Odpoutané spuštění**: `exp1-agent serve --register` ve vlastní procesní skupině s uzavřenými standardními proudy, takže přežije klienta, který ho spustil.

### HTTP API

Všechny trasy vyžadují `Authorization: Bearer <password>`.

| Metoda | Cesta | Účel |
| --- | --- | --- |
| `GET` | `/health` | živost a verze |
| `GET` | `/sessions` | vypíše živé relace |
| `POST` | `/sessions` | vytvoří jednu; `x-exp1-directory` nastaví její pracovní adresář |
| `GET` | `/sessions/{id}` | úplný snímek |
| `POST` | `/sessions/{id}/prompt` | spustí tah |
| `GET` | `/sessions/{id}/events` | SSE: snímek, delty, nástroje, využití, název, hotovo, nečinnost |
| `POST` | `/sessions/{id}/answer` | odpoví na čekající `ask_question` |
| `POST` | `/sessions/{id}/interrupt` | zruší běžící tah |
| `GET` | `/models` | katalog s opravdovými úrovněmi úsilí každého modelu. `?all=1` ignoruje rozsah: výběr, který rozsah nastavuje, potřebuje i modely mimo něj |

Relace si nese vlastní pracovní adresář, takže jeden server obsluhuje několik projektů; adresář přichází s `POST /sessions`, místo aby se zafixoval při startu serveru.

## Automatická kompaktace

Dlouhá konverzace se shrne dřív, než přeteče kontextové okno. Po každém tahu server porovná `input_tokens` požadavku s `context_window` modelu z katalogu Codex a zkompaktuje, jakmile konverzace dosáhne **95 %** okna:

```
Context at 7378 of 258400 tokens (95% threshold) — compacting
Conversation compacted; continuing from the summary
```

Kompaktace pošle konverzaci modeli se shrnovacím promptem a strukturovanou šablonou, kterou používá opencode — Objective, Important Details, Work State (Completed / Active / Blocked), Next Move, Relevant Files — a výstupy nástrojů zkrátí na 2000 znaků, aby se shrnovač neutopil.

Přepsaná konverzace je **shrnutí následované posledními pěti uživatelskými tahy doslovně**, každý s voláními nástrojů agenta a jejich výsledky, aby se nit v ruce neztratila:

```
[user]      Summary of everything so far: ## Objective …
[user]      Turn two: run shell ["seq","1","900"] and reply OK.
[call]      shell
[result]    exit_code: 0 1 2 3 …
[assistant] OK
… four more turns …
```

Fakt řečený před kompaktací je známý i po ní. Když je některý z těch tahů sám obrovský, nejstarší položky ocasu se zahazují, dokud se výsledek znovu nevejde do okna, takže jediný obří výstup nástroje nemůže konverzaci nutit kompaktovat pořád dokola.

Chce-li se jiný podíl, je tu `EXP1_COMPACT_PERCENT`, což je také způsob, jak cestu procvičit bez vyplnění okna o 272 000 tokenech.

Konverzace drží na disku až 5 000 zpráv přepisu; vlastní kontext modelu se ohraničuje zvlášť prořezáváním a kompaktací, takže dlouhá relace zůstává čitelná, aniž by stála tokeny.

## Perzistence

Konfigurace žije v `~/.config/exp-1/` (nebo `$XDG_CONFIG_HOME/exp-1`); samotné konverzace žijí v `~/.cache/exp-1/sessions/` (nebo `$XDG_CACHE_HOME/exp-1/sessions`), protože přepis je keš: ztráta jednoho stojí kontext, ne nastavení. Vlastní soubory démona — `server.json` a `password` — sedí v `~/.local/state/exp-1/`.

| Soubor | Drží |
| --- | --- |
| `config/settings.json` | model, úsilí uvažování, motiv, režim titleru, záložní model |
| `config/credentials.json` | jeden API klíč na poskytovatele, práva `0600` |
| `config/prompts/*.md` | systémové prompty, upravitelné: `system`, `subagent`, `compaction`, `title`, `plan` |
| `config/permissions.json` | pravidla allow/ask/deny |
| `config/mcp.json` | MCP servery |
| `config/skills/` | nainstalované dovednosti, každá jeden adresář |
| `cache/sessions/<id>.json` | jedna konverzace: název, oba přepisy, historie na straně modelu, historie promptů |

Konverzace drží na disku svých nejnovějších **250 zpráv** a celá keš se při startu serveru zkracuje na **250 MB** — nejstarší soubory první. Starší rozvržení, kde všechno sedělo v `config/exp-1/sessions/`, se migruje při prvním startu po změně.

Nastavení se zapisuje při ukončení aplikace; relace se zapisuje po každém dokončeném tahu, takže pád stojí nejvýše tah na cestě.

Přibalený titler je volitelná funkce: `cargo build --release --no-default-features` ho vynechá a vezme s sebou celou závislost Candle, a názvy relací pak přicházejí jen z modelu. CI takto sestavuje, protože titler táhne Candle: crate je ve stromu, ale release za něj nemusí platit.

Prompty jsou soubory, ne konstanty: `~/.config/exp-1/prompts/system.md` (vlastní pokyny agenta), `subagent.md`, `compaction.md` a `title.md` se při prvním spuštění vypíšou a při každém požadavku čtou, takže úprava platí pro další tah bez sestavování. Pořadí určení modelu a úsilí při startu je **příkazový řádek → uložená nastavení → první uvedený model katalogu**.

**Každé spuštění začíná na prázdné relaci.** Nic se automaticky neobnovuje — `/sessions` otevře výběr uložených konverzací, nejnovější první, každou s vygenerovaným názvem, id a časem posledního použití. Seznam je omezen podle adresáře: relace je vidět tam, kde začala, kdekoli pod tím, a všude, když začala v `$HOME` (ty „globální“). `$HOME` sám vypisuje jen ty globální, takže se nestane katalogem všech projektů. Výběr jedné uloží konverzaci, kterou opouštíte, a načte vybranou. `/clear` opustí aktuální konverzaci a začne novou a vaše nastavení zachová.

### Názvy relací

Názvy pocházejí z přibaleného **titleru** — místní T5 (7,6M parametrů, 3 vrstvy kodéru a 3 dekodéru), která běží přes Candle v tomto procesu a je zkompilována do agenta ze `src/titler.rs`. Váhy (`Cyronius/titler`) se čtou z keše Hugging Face a načítají se nejvýše jednou na proces, takže jakmile jsou na disku, generování názvů je místní, okamžité a zdarma.

Generování běží na blokujícím vlákně ve chvíli, kdy skončí první tah, nikdy na vlákně UI. Relace s rodičem — vedlejší chat — se nepojmenovávají vůbec, protože patří ke konverzaci, kterou následují. Používá se vícejazyčná cesta knihovny (ne ta surová), takže se rozpozná jazyk a neanglický prompt se nejdřív přeloží do angličtiny. Relace se pojmenuje jednou a náhled vzatý z promptu je to, co výběr ukazuje, dokud název nedorazí.

**Kdo relaci pojmenuje, je nastavení**, `titler` v `settings.json`:

| Hodnota | Chování |
| --- | --- |
| `auto` (výchozí) | nejprve model Codex, jako záložní přibalený titler |
| `local` | jen přibalený titler — žádné útraty za API, slabší názvy |
| `remote` | jen model Codex |

Názvy se žádají od levné řady `luna` v katalogu, a ne od modelu samotné relace — stejný nápad, jako když opencode žádá poskytovatele o jeho malý model — a tokeny, které název stojí, se k využití relace přičtou jako každé jiné volání.

`/retitle` pojmenuje relaci znovu, tentokrát z původního zadání plus nedávné konverzace (omezené na 8 000 znaků), což je způsob, jak opencode regeneruje název, když se relace přejmenuje na prázdno. Prompt sám žije v `prompts/title.md` a je to úplný dokument ve stylu opencode: úkol, pravidla (stejný jazyk jako uživatel, ≤50 znaků, zachovat přesné termíny a čísla, nikdy neodpovídat na otázku) a deset zpracovaných příkladů.

`auto` schválně vede modelem Codex. Titler se 7,6M parametry je ohledně své velikosti poctivý: se stejnými ruskými prompty vytvořil `Understanding Two Plus Terms` pro «сколько будет два плюс два» a `Request for Sign` pro požadavek na tabulku, protože bezplatný překladač Google vykládá «табличку» jako „sign“ a malý model pak pojmenuje chybný překlad. Model Codex tytéž prompty nazve `Сколько будет два плюс два?` a `Формула корней квадратного уравнения в LaTeX`. Právě na tuto cestu s LLM je `prompts/title.md` — přibalený model bere první zprávu tak, jak je, a soubor úplně ignoruje.

## Rozvržení

| Soubor | Role |
| --- | --- |
| `src/auth.rs` | Načtení přihlašovacích údajů, kontrola vypršení JWT, obnovení, atomické uložení |
| `src/store.rs` | Soubory nastavení a relací pod `~/.config/exp-1/sessions/` |
| `src/catalog.rs` | Katalog modelů a úsilí uvažování z keše Codexu |
| `src/titler.rs` | Místní T5, která pojmenovává relace; kompiluje se do agenta s funkcí `titler` |
| `src/protocol.rs` | Přenosové typy požadavků a odpovědí |
| `src/md.rs` | Markdown a trubkové tabulky na stylované běhy |
| `src/latex.rs` | Matematika LaTeX na Unicode |
| `src/api.rs` | Přeprava HTTP, dekódování SSE, sestavování hlaviček |
| `src/tools.rs` | `read`, `write`, `edit`, `shell`, `ast`, `ask_question` a navíc registr běhů na pozadí a výstupů |
| `src/app.rs` | Vlákna, stav TUI, vykreslování, snímek a obnovení relace |
| `src/main.rs` | CLI, zpracování kláves, smyčka agenta |

## Omezení

Tah provede nejvýše 200 kol volání nástrojů; kdyby se tam dostal, přepis to řekne, místo aby tiše přestal.

**Tokenová ekonomika.** Požadavky zůstávají schválně malé: `read` stránkuje, `grep` omezuje počet shod, dlouhý výstup příkazu si drží oba konce, zpráva subagenta se uřezává na 6 000 znaků a před každým požadavkem se nejstarší výstupy nástrojů — mimo poslední dva tahy — nahradí jednořádkovou poznámkou, co se zahodilo. Model může nástroj znovu spustit pro cokoli, co ještě potřebuje, a přepis, který vidí uživatel, si nechává plný text.

**Paralelní volání.** Zpráva modelu může požádat o několik nástrojů najednou. Série volání jen pro čtení (`read`, `grep`, `count`, `ast`, `fetch`, `websearch`) běží ve stejný čas; všechno, co zapisuje, si drží pořadí, takže zápis následovaný čtením téhož souboru nelze přehodit.

Toto je MVP. Známé mezery:

- Vertex bere jen přihlašovací údaje aplikace od gcloud; klíč účtu služby, který potřebuje podepsaný JWT, se nevytváří.
- Vedlejší chat při každé otázce vidí celou hlavní konverzaci — hlavní historie se čte, nikdy nezapisuje — takže odpověď na `/btw` má plný kontext a hlavní vlákno zůstává nedotčené.
- Žádné rušení streamování nad rámec zrušení úlohy.
- Příkazy běží bez sandboxu.

## Ověřeno proti

Podrobnosti protokolu byly zkontrolovány proti zdrojům `openai/codex` a potvrzeny živými požadavky: koncový bod a hlavičky, rotace obnovení, streamování textu, cesta `function_call` tam a zpět a vícenásobná smyčka nástrojů, která zapsala soubor.

## Poděkování

- **Diktování** běží na řečovém modelu **whistle** a enginu **Needle** od Cactus
  (`cactus-needle`, Apache-2.0): stránka produktu je [cactuscompute.com/whistle](https://cactuscompute.com/whistle),
  váhy a enginy pro jednotlivé platformy jsou na [Hugging Face](https://huggingface.co/Cactus-Compute/whistle).
  Engine si je stahuje do `~/.cache/cactus-needle`; nic z toho není součástí tohoto repozitáře.
- **Volitelný lokální titler** staví na [Candle](https://github.com/huggingface/candle) od Hugging Face,
  pod Apache-2.0 nebo MIT.
- Vše ostatní jsou crates z `Cargo.toml`, každý pod vlastní licencí.

## Licence

Tento projekt je licencován výhradně pod GNU General Public License verze 3. Viz [LICENSE](LICENSE).
Licence třetích stran a jejich oznámení se mohou lišit; viz příslušná oznámení a výše uvedené poděkování.
