[English](README.md) · [Русский](README.ru.md) · [Українська](README.uk.md) · [Čeština](README.cs.md) · **Español** · [Deutsch](README.de.md) · [中文](README.zh.md) · [日本語](README.ja.md)

# exp-1 agent

<img src="assets/exp-1-logo.png" alt="exp-1 logo" width="96" />

Un agente de programación mínimo en Rust con una interfaz de terminal que habla con **27
proveedores**, entregando a cada uno la lista de modelos que realmente sirve: el catálogo llega en
vivo desde models.dev, con una tabla en el binario como alternativa sin conexión. Cinco protocolos
de cable los cubren todos — la API **OpenAI chat-completions**, compartida por una docena de
proveedores, **OpenAI Responses**, **Anthropic messages**, el **`generateContent` de Gemini**
nativo de Vertex AI y el backend de Codex — y un proveedor inicia sesión con una clave de API, un
inicio de sesión por código de dispositivo, el propio inicio de sesión del proveedor o credenciales
que la máquina ya tiene. La ruta de Codex llegó primero y sigue siendo una de ellas: OAuth de
ChatGPT contra `auth.openai.com`, con las peticiones a
`https://chatgpt.com/backend-api/codex/responses`.

## Qué hace

- Transmite las respuestas del endpoint Responses de Codex (SSE, analizado a mano).
- Ejecuta el bucle del agente — petición → flujo → ejecutar herramienta → devolver la salida →
  repetir — hasta que el modelo deja de pedir herramientas.
- Mantiene dos hilos: la conversación principal, y un hilo lateral para preguntas que no deben
  interrumpirla.
- Muestra el razonamiento, el texto transmitido del asistente, las llamadas a herramientas y su
  salida en una sola transcripción, con la duración por turno y la hora del reloj.

## Herramientas

| Herramienta | Argumentos | Notas |
| --- | --- | --- |
| `read` | `path`, `limit?`, `pos?` | Como máximo 400 líneas por defecto; `limit` lo cambia, `pos` es la primera línea, con base 1. La salida va numerada y una lectura cortada dice cómo continuar. Un archivo de 3000 líneas se recorre por partes a propósito: tragárselo entero cuesta decenas de miles de tokens. |
| `write` | `path`, `content` | Crea los directorios superiores; sobrescribe un archivo existente. |
| `edit` | `path`, `old`, `new`, `edits?` | Reemplazo exacto, todo o nada. Un fallo nombra las líneas más cercanas o las posiciones de las coincidencias, así que un desajuste de espacios no obliga a releer. `edits` admite varios pares `{old, new}` para un solo archivo en una sola llamada. El resultado muestra qué cambió. |
| `fetch` | `url`, `limit?`, `pos?` | Una página como texto plano, paginada como un archivo: `limit` líneas (400 por defecto) desde `pos` (con base 1). Se descartan los scripts y los estilos, se decodifican las entidades. |
| `count` | `path`, `refresh?` | Líneas de un archivo, o de todos los archivos bajo un directorio, sin leer el contenido. Se cachea por archivo en `.cache/count`, indexado por tamaño y mtime. |
| `grep` | `pattern`, `path?`, `include?`, `max?` | Búsqueda con expresiones regulares en un archivo o un árbol sin leer los archivos en la conversación. `include` filtra por nombre de archivo (`*.rs`). Omite `.git`, `.cache`, `target`, `node_modules`; no sigue enlaces simbólicos; ignora los archivos no de texto. Limitada a `max` coincidencias (100 por defecto). |
| `delete` | `path` | Elimina un archivo. Los directorios se rechazan: vaciar un árbol es cosa de `shell`, donde el comando entero es visible. |
| `shell` | `command`, `background?`, `limit?`, `output_id?` | En primer plano bloquea al agente hasta que termina; tras 60 s un comando se pasa a segundo plano y se avisa al agente cuando termina. `background: true` devuelve un id de inmediato. Los comandos en segundo plano nunca se matan por un temporizador. La salida se limita a 30 000 bytes: una más larga conserva el principio y el final, con el hueco marcado. `output_id` vuelve a leer cualquier ejecución anterior con otro `limit`. |
| `ast` | `path?`, `refresh?` | Árbol del proyecto en formato `tree`. Se cachea en `.cache/ast`; `refresh: true` lo reconstruye. Se omite `.git`. |
| `websearch` | `query`, `numResults?`, `type?`, `livecrawl?`, `contextMaxCharacters?` | Búsqueda web con el contenido de las páginas. Usa el MCP alojado de Exa en `mcp.exa.ai`, que **no necesita clave de API**; define `EXA_API_KEY` para ampliar los límites de peticiones. `type` es `auto`/`fast`/`deep`. |
| `ask_question` | `question`, `options?`, `label?`, `submit?` | Bloquea el turno y muestra un panel de pregunta. `↑↓` eligen una opción, `Tab` se mueve entre preguntas, `Enter` confirma, `Esc` descarta. Cualquier cosa que se escriba se toma como respuesta libre y prevalece sobre la opción resaltada. |

Los comandos de shell se ejecutan sin sandbox como el usuario. Un comando en primer plano se pasa a
segundo plano tras 60 segundos, nada mata un comando en segundo plano por un temporizador, y la
salida se limita a 30 000 bytes: un resultado más largo conserva el principio y el final, con el
hueco marcado como omitido.

La herramienta `web` busca en la web: primero el MCP alojado de Exa, porque devuelve el contenido de
la página, y la página HTML de DuckDuckGo cuando el nivel gratuito compartido de Exa limita las
peticiones. `EXA_API_KEY` cambia Exa a tu propio plan y es el único ajuste que necesita cualquiera de
las dos vías.


## Plataformas

El agente es un solo código base para Linux, macOS y Windows: la TUI, el bucle del agente, los
proveedores, MCP, las habilidades, las sesiones y el titler integrado son Rust puro y funcionan en
todas partes. Las partes que tocan el sistema operativo tienen una rama por plataforma:

| | Linux | macOS | Windows |
| --- | --- | --- | --- |
| TUI, agente, proveedores, MCP, habilidades, sesiones | sí | sí | sí |
| Apertura de la página de inicio de sesión | `xdg-open` | `open` | `cmd /C start` |
| Vitalidad y parada del daemon | `kill` 0 / SIGTERM→SIGKILL | igual | `tasklist` / `taskkill /T` y luego `/F` |
| Servidor desprendido de su consola | `process_group(0)` | igual | `DETACHED_PROCESS \| CREATE_NEW_PROCESS_GROUP` |
| Directorios de configuración y estado | `$XDG_*`, `~/.config` | `$XDG_*`, `~/.config` | `%APPDATA%`, `%LOCALAPPDATA%` |
| Imagen del portapapeles (`ctrl + v`) | `wl-paste`, `xclip` | `pngpaste` | PowerShell + `System.Windows.Forms` |
| Aviso de fin de turno | `notify-send` | `osascript` | notificación globo de PowerShell |
| Instalaciones desde archivo (`exp-1 install *.zip`) | `unzip` / `tar` | igual | `tar` (bsdtar lee zip) |
| Servidores MCP locales llamados `npx` | tal cual | tal cual | se reintenta como `npx.cmd` |

Las ramas Unix y Windows del daemon y del portapapeles se comprobaron en compilación activándolas
temporalmente en Linux; `creation_flags` es la única llamada exclusiva de Windows que no puede
comprobarse sin un objetivo Windows, y nada de esto se ha ejecutado en una máquina Windows o macOS
real.

### Compilación cruzada desde Linux

Un binario de Windows se compila en Arch con la cadena de herramientas mingw (esto necesita root una
vez):

```sh
sudo pacman -S --needed mingw-w64-binutils mingw-w64-gcc
rustup target add x86_64-pc-windows-gnu
cargo build --release --target x86_64-pc-windows-gnu
```

`dlltool`, de `mingw-w64-binutils`, es lo que necesitan las bibliotecas de importación; sin él la
compilación se detiene en `windows-sys`.

Sin root, el mismo binario sale de un tarball de llvm-mingw desempaquetado bajo `~/.local`. El std de
Rust para este objetivo pide al enlazador `libgcc.a`, que llvm-mingw no incluye, así que un shim de
dos enlaces simbólicos aporta los equivalentes, y `--no-default-features` deja fuera el titler
integrado igual que hace la CI:

```sh
mkdir -p ~/.local/mingw-shim
ln -sf ~/.local/llvm-mingw/lib/clang/*/lib/windows/libclang_rt.builtins-x86_64.a ~/.local/mingw-shim/libgcc.a
ln -sf ~/.local/llvm-mingw/x86_64-w64-mingw32/lib/libunwind.a ~/.local/mingw-shim/libgcc_eh.a
PATH="$HOME/.local/mingw-shim:$HOME/.local/llvm-mingw/bin:$PATH" \
CARGO_TARGET_X86_64_PC_WINDOWS_GNU_LINKER="$HOME/.local/llvm-mingw/bin/x86_64-w64-mingw32-clang" \
RUSTFLAGS="-L $HOME/.local/mingw-shim" \
cargo build --release --target x86_64-pc-windows-gnu --no-default-features
```

Los `cargo` y `rustup` de `~/.cargo/bin` conocen los objetivos de compilación cruzada; los paquetes
del sistema no, y en esta máquina `~/.cargo/bin` no está en el `PATH`, así que llámalos por su ruta
completa.

Un binario de macOS tampoco necesita root, y con `cargo zigbuild` todo el árbol se compila para
`aarch64-apple-darwin`: `zig cc` compila de forma cruzada las dependencias en C (`aws-lc-sys`
incluida, cosa que un simple `cargo check` no puede hacer — el `cc` anfitrión rechaza `-arch arm64`),
y todos los crates compilan. Lo que lo detiene es el último paso: la característica `rustls` de
`reqwest` 0.13 arrastra `rustls-platform-verifier`, y entonces el enlazador informa de `unable to
find framework 'Security'` y `'CoreFoundation'`. Esos solo existen en el SDK de Apple; Zig enlaza
Mach-O y libSystem sin problema, pero no incluye los frameworks. Así que los binarios de macOS vienen
de un Mac o de la CI (`.github/workflows/release.yml` compila las dos arquitecturas). La receta de
abajo es la que llega hasta el enlazado:

```sh
mkdir -p ~/.local/zig && curl -L https://ziglang.org/download/0.14.1/zig-x86_64-linux-0.14.1.tar.xz \
  | tar -xJ -C ~/.local/zig --strip-components=1
cargo install cargo-zigbuild
rustup target add aarch64-apple-darwin
PATH="$HOME/.local/zig:$PATH" cargo zigbuild --release --target aarch64-apple-darwin
```

El resultado no está firmado. En Apple Silicon un binario sin firmar se mata al arrancar, así que hay
que firmarlo una vez en el propio Mac:

```sh
codesign -s - --force ./exp1-agent        # ad-hoc signature
xattr -cr ./exp1-agent                    # if it arrived by download
```

## Requisitos

- Rust 1.85+ (edición 2024)
- O bien un inicio de sesión existente de la CLI de Codex (se reutiliza su `auth.json`), o bien
  `/login`, que ejecuta él mismo el mismo flujo de dispositivo.
- El crate `titler` integrado en `crates/titler`. Arrastra Candle, y por eso la primera compilación
  tarda un rato y el binario ocupa ~16 MB en vez de ~7 MB. Una vez que los pesos están en la caché
  de Hugging Face, titular es offline, y un clon recién hecho no necesita un checkout hermano.

## Instalación

Para instalar la versión más reciente desde GitHub necesitas Rust 1.85+ y Cargo:

```sh
cargo install --git https://github.com/archpulse/exp-1.git --locked exp1-agent
exp1-agent --help
```

Cargo instala el binario `exp1-agent` en `~/.cargo/bin`; añade ese directorio a `PATH` si hace falta. La compilación predeterminada incluye el titulador local, así que la primera instalación tarda más mientras se compila Candle.

## Uso

```sh
cargo run --release                      # uses the current directory
exp1-agent --model gpt-6-luna --effort high # pick a model and reasoning effort
exp1-agent --cwd ~/projects/my-app          # run the shell tool somewhere else
exp1-agent --help
```

| Tecla | Acción |
| --- | --- |
| `Enter` | Envía el prompt, o ejecuta el comando cuando ya está completo |
| `↑` `↓` | Mueve la selección de autocompletado, o recupera prompts anteriores |
| `Tab` | Acepta el autocompletado resaltado |
| `Ctrl + /` | Cambia entre el hilo principal y el hilo lateral |
| `Ctrl + Shift + A` | Cambia el modo de permisos: always ask → never ask → ask only commands |
| `Esc` | Nada por sí solo |
| `Esc` `Esc` | Aborta el turno en curso, o descarta la lista de autocompletado o el selector |
| `Ctrl-C` | Salir |
| `Ctrl-U` | Borra la línea de entrada |
| `Ctrl-Backspace` | Borra la palabra anterior |
| `Ctrl-Delete` | Borra la palabra siguiente |
| `Ctrl-W`, `Alt-Backspace` | Igual que `Ctrl-Backspace` |
| Rueda del ratón | Desplaza la transcripción |
| `PgUp` / `PgDn` | Desplaza la transcripción |
| `←` `→` `Home` `End` | Mueve el cursor |
| `Backspace` / `Delete` | Editar |

Las teclas de flecha recorren el historial de prompts: tras enviar `hi`, `↑` lo trae de vuelta al
campo, las pulsaciones siguientes retroceden más, y `↓` avanza hasta el campo vacío. El
desplazamiento está en la rueda del ratón, así que las flechas quedan libres para el historial.

Escribir `/` abre una lista de autocompletado filtrada por prefijo. Tras un espacio, también
completa los argumentos del comando: `/themes <tab>` lista los esquemas, `/model <tab>` los modelos,
y `/model gpt-6-luna <tab>` los esfuerzos de razonamiento. `Enter` sobre un `/model` o `/themes` a
secas abre en su lugar una lista de selección numerada; `/model` son dos pasos, primero el modelo y
luego el esfuerzo. Con el campo vacío el cursor está justo al principio, antes del marcador de
posición.

La línea de pistas cambia según el contexto. La entrada se bloquea mientras hay un turno en curso.

`Esc` es inerte a propósito; lo que interrumpe es la doble pulsación. Interrumpir aborta la petición
en curso y descarta cualquier texto parcial del asistente, dejando intacto el historial de la
conversación.

## Comandos con barra

Se gestionan localmente; nunca llegan al modelo.

| Comando | Efecto |
| --- | --- |
| `/login` | Elige un proveedor: uno con sesión iniciada solo cambia a él; elegir el que está en uso vuelve a iniciar sesión. Los inicios de sesión por código de dispositivo abren la página del proveedor en el navegador; el resto piden una clave de API |
| `/scoped-models` | Marca qué modelos tiene permitido ofrecer `/model`; vacío significa todos |
| `/sessions` | Elige una conversación guardada para reabrirla |
| `/init` | Escribe un esqueleto de `AGENTS.md` en el directorio de trabajo |
| `/model` | Abre el selector de dos pasos (modelo y luego esfuerzo); `/model <name> [low\|medium\|high]` lo fija directamente |
| `/themes` | Abre el selector de temas; `/themes <name>` lo fija directamente |
| `/status` | Muestra modelo, esfuerzo, directorio de trabajo, herramientas, tema, plan, proveedor de autenticación |
| `/btw` | Responde una pregunta aparte en el hilo lateral mientras el turno principal sigue en curso |
| `/clear` | Borra la transcripción y el historial de la conversación |
| `/help` | Muestra los atajos de teclado |
| `/compact` | Resume la conversación ahora en vez de esperar a que se llene la ventana |
| `/undo` | Restaura los archivos que cambió el último turno (en memoria; un reinicio los olvida) |
| `/diff` | Diff unificado de lo que cambió el último turno |
| `/fork` | Continúa esta conversación como una sesión nueva, dejando intacta la original |
| `/sessions <text>` | Busca en las conversaciones guardadas en vez de solo listarlas |
| `/retitle` | Vuelve a nombrar la sesión a partir de la petición original y la conversación reciente |
| `/effort` | Elige el esfuerzo de razonamiento del modelo actual, sin cambiar de modelo |
| `/mcp` | Lista los servidores MCP y las herramientas que ofrecen |

Temas disponibles: `default` (azul), `amber`, `jade`, `mono`. Los colores son truecolor, así que un
tema del terminal no puede reasignarlos.

## Interfaz

El diseño sigue la TUI de OpenAI Codex: sin paneles con borde, una cabecera `>_` con el directorio de
trabajo debajo, un emergente de autocompletado o un selector numerado por encima de la entrada, una
banda de entrada con margen interior y un indicador `›` y un cursor `│` parpadeante, y luego una
línea de modelo y una línea de pistas.

- Un prompt enviado se redibuja en la transcripción como una banda de ancho completo, igual que hace
  Codex.
- **La salida de las herramientas está oculta por defecto**: la transcripción lleva la llamada
  (`read src/main.rs`), no lo que devolvió. `ctrl-o` muestra la salida bajo su llamada y la vuelve a
  ocultar, que es la diferencia entre seguir el trabajo y leerlo.
- **El ratón no sirve solo para desplazarse.** La rueda desplaza, arrastrar selecciona y copia, y un
  clic actúa sobre lo que haya tocado: una fila de un selector o del emergente de autocompletado se
  comporta como Enter sobre esa fila, la pista `ctrl + / for side` del pie cambia de hilo, y un
  enlace de un mensaje se abre en el navegador. Cada fotograma registra las regiones que dibujó, así
  que un clic es una prueba de impacto contra el último fotograma en vez de una conjetura sobre el
  diseño. El resaltado al pasar el ratón no está implementado a propósito: cuesta un redibujado por
  cada movimiento del ratón, y el propio cursor del terminal ya dice dónde está el puntero.
- Un subagente informa de su paso actual en **una sola línea que se actualiza**
  (`subagent → grep tokenizer`) en vez de una línea por paso, y la llamada que lo inició dice
  `task: <description>`.
- **Un clic en tu propio mensaje pregunta qué hacer con él**, en vez de hacerlo: un menú pequeño con
  `copy` (el texto va al portapapeles) y `revert`. Revertir descarta todo lo posterior a ese mensaje
  — la conversación, lo que se le dijo al modelo por debajo de ese punto y cada cambio de archivo
  que hicieron los turnos siguientes, que el registro de deshacer restaura. El mensaje sale de la
  transcripción y su texto vuelve al prompt, para poder editarlo y enviarlo de nuevo. El texto se
  comprueba antes de descartar nada, así que un clic que cae en el mensaje equivocado no hace nada.
- La transcripción se pega al mensaje más reciente; la rueda del ratón o `PageUp`/`PageDown` se
  desplazan hacia arriba por el historial, `ctrl-g` salta de vuelta al mensaje más reciente y
  `ctrl-t` al más antiguo, y una vista desplazada hacia arriba se queda donde está mientras llegan
  líneas nuevas en vez de ser arrastrada de vuelta al final.
- Arrastrar con el ratón selecciona texto en cualquier parte de la pantalla — la cabecera incluida —
  y al soltar lo pone en el portapapeles: primero OSC 52, para que también funcione por ssh, y luego
  `wl-copy`, `xclip` o `xsel` cuando existen. Una nota `copied N chars` lo confirma en la línea de
  pistas. La selección es un tinte discreto detrás del texto y se detiene en el último carácter de
  cada fila, como se comporta la selección del propio terminal. El terminal nunca ve el arrastre,
  porque la aplicación retiene el ratón para el desplazamiento, así que esta es la única forma de
  copiar.
- Una imagen adjunta aparece como un chip relleno `Image 1` encima del prompt, con
  `ctrl-v replaces it` al lado, de modo que lo que está a punto de enviarse se ve en la propia banda
  de entrada.
- Mientras hay un turno en curso, una línea de actividad se sitúa **justo debajo del prompt al que
  pertenece** — con su propia etiqueta para cada cosa que hace: `⠋ thinking 4s` mientras hay una
  petición en vuelo, `⠋ preparing write… (12 KB) 8s` mientras el modelo sigue escribiendo los
  argumentos de una llamada a herramienta, `⠋ running 4s` mientras la herramienta se ejecuta. Cada
  etiqueta lleva su propio reloj, así que los segundos son sobre lo que ocurre ahora y nunca cuentan
  el turno entero; el total del turno es lo que informa `Worked for` al final. La barra inferior
  sigue siendo una etiqueta simple `GPT-6-Luna high · /path · <thread> · Main [default]`;
  `esc esc interrupts` vive en la línea de pistas.
- Cuando un turno termina, `Worked for 5s · 12:08 PM` se añade a la transcripción.
- La línea del modelo dice `GPT-6-Luna high · /path · <thread name> · Main [default]` o `… · Agent`
  en el hilo lateral. El nombre del hilo se toma del primer prompt.
- La línea de pistas lleva el cambio de hilo a la derecha, en rojo mientras el hilo lateral está
  activo.
- Una pantalla vacía se queda discreta: la lista completa de comandos aparece solo en una máquina
  que nunca ha guardado una conversación, y después es una sola línea que apunta a `/` y `/help`.
- La escritura sigue activa durante un turno, así que se puede hacer una pregunta con `/btw` sin
  esperar.
- `/btw`, o `ctrl + /`, responde en un chat lateral que lleva toda la conversación principal como
  contexto sin añadirle nada.
- `shift + tab` alterna entre el modo build y el modo plan; el pie muestra `plan mode` mientras está
  activo.
- Un turno que dura más de quince segundos hace sonar la campana del terminal (y `notify-send`,
  donde el escritorio lo tenga) al terminar, para no tener que vigilar un trabajo largo.
- `shift + enter` escribe un salto de línea en vez de enviar, así que un prompt puede tener varias
  líneas; la banda de entrada crece para contenerlas.
- `ctrl + v` toma una imagen del portapapeles (Wayland `wl-paste`, X11 `xclip`) y la adjunta al
  siguiente prompt, mostrando `▣ image attached` en la línea de pistas hasta que se envía. La imagen
  viaja como data URL, así que los backends de Codex, OpenAI, Anthropic y Gemini la transmiten
  todos.
- `ask_question` levanta un panel sobre la entrada, a imagen de la herramienta Question: una barra
  de acento por el borde izquierdo, el título `Questions`, pestañas cuando se hacen varias preguntas
  juntas, opciones numeradas con la resaltada destacada, un hueco `Type your own answer`, y
  `⇥tab ↑↓ select enter confirm esc dismiss` en la parte inferior.

Los colores son truecolor, así que un tema del terminal no puede reasignarlos. El modelo, el
esfuerzo y el tema cambian en tiempo de ejecución mediante `/model` y `/themes`.

Diferencias deliberadas respecto a Codex: Codex tiene `/permissions` y `/review`, que necesitan
política de sandbox y un revisor de diffs; esta compilación tiene `/clear` y `/help` en su lugar. El
pie de Codex anuncia superposiciones de agentes y atajos, que este MVP no tiene.

## Autenticación

`/login` lista los proveedores, y cada uno inicia sesión de la forma que espera su proveedor:

- **Una clave de API** para la mayoría de ellos, enviada como `Authorization: Bearer` (o `x-api-key`
  para la API de Anthropic) y guardada por proveedor en `~/.config/exp-1/credentials.json`, con modo
  `0600`. Una clave puede cubrir varios proveedores — `opencode-go` y `opencode-zen` son una sola
  cuenta detrás de dos URL base, y `providers::key_group` nombra los grupos.
- **Un inicio de sesión por código de dispositivo** para `openai (auth)`, `github copilot` y
  `meta (auth)`: la página de inicio de sesión del proveedor se abre en el navegador, y la URL y el
  código se imprimen como alternativa.
- **Un token del propio inicio de sesión del proveedor** para `anthropic (auth)`, enviado con las
  cabeceras de identidad del cliente oficial.
- **Credenciales que la máquina ya tiene** para `google vertex`: lo que ya haya iniciado sesión
  `gcloud auth application-default login`, con `GOOGLE_CLOUD_PROJECT` y `GOOGLE_CLOUD_LOCATION`
  nombrando el proyecto y la región. Nada que pegar.

La ruta de Codex además mantiene un archivo propio: en la primera ejecución el agente copia
`$CODEX_HOME/auth.json` (por defecto `~/.codex/auth.json`) a `~/.exp1-agent/auth.json` y trabaja desde
esa copia. **Tus credenciales reales de Codex nunca se reescriben.** Cuando al token de acceso le
quedan 5 minutos o menos para caducar, el agente lo renueva:

```
POST https://auth.openai.com/oauth/token
grant_type=refresh_token
client_id=app_EMoamEEZ73f0CkXaXp7hrann
```

La respuesta rota el token de refresco, así que el nuevo se escribe de vuelta en la copia propia del
agente. El archivo guardado se escribe de forma atómica con modo `0600`.

## Modelos y esfuerzos

Ni la lista de modelos ni los esfuerzos de razonamiento están escritos a fuego. Ambos se leen del
propio catálogo de la CLI de Codex en `$CODEX_HOME/models_cache.json` (por defecto
`~/.codex/models_cache.json`), usando el `slug`, `display_name`, `description`, `visibility`,
`default_reasoning_level` y `supported_reasoning_levels` de cada entrada. El valor por defecto al
arrancar es el primer modelo listado en ese catálogo.

**Todos los demás proveedores obtienen su lista en vivo, desde models.dev.** El catálogo se descarga
como máximo una vez cada doce horas en `~/.cache/exp-1/models-dev.json`, se mantiene en memoria y se
lee para tres cosas: los ids de modelo que ofrece un proveedor, el protocolo que habla cada modelo
(models.dev lo registra como el paquete del AI SDK con el que está configurado el proveedor —
`@ai-sdk/anthropic` para un modelo Claude detrás de una pasarela, `@ai-sdk/openai` para uno de Grok)
y la ventana de contexto, que es a partir de la cual se calcula el umbral de compactación
automática. Un proveedor que falte en el catálogo conserva la tabla de `src/providers.rs`, y una
máquina sin conexión también, así que la aplicación nunca depende de la descarga.

Un modelo que las tablas no recogen en absoluto también obtiene un protocolo: lo decide su prefijo de
proveedor (`claude-*`, `minimax-*` y `qwen-*` hablan Anthropic; `grok-*`, `gpt-*` y `muse-*` hablan
Responses; `gemini-*` habla Gemini; todo lo demás es chat), y un proveedor que habla un solo
protocolo responde por cualquier otra cosa.

**El selector muestra todos los proveedores con sesión iniciada**, un estante en vez de uno por
inicio de sesión: las filas llevan `via <provider>` para distinguir dos sombreros sobre el mismo
nombre de modelo, el proveedor en uso va primero, y elegir un modelo de otro proveedor cambia a él
antes de fijar el modelo. Un estante así de largo se busca escribiendo: todos los selectores filtran
mientras escribes, el retroceso amplía la coincidencia, y Escape amplía antes de cerrarse. El ámbito
de `/scoped-models` filtra el mismo estante y nombra el modelo de un proveedor
(`opencode-go/grok-4.7`), así que el mismo nombre de dos proveedores son dos entradas; un ámbito más
antiguo con nombres a secas sigue significando «este nombre, de cualquier proveedor», y `ctrl-a` en
ese selector borra el ámbito, que vuelve a ofrecerlo todo. `/model grok-4.7` toma el proveedor en
uso cuando varios ofrecen ese nombre, y `/model opencode-go/grok-4.7` nombra uno exactamente.

Los esfuerzos difieren de verdad según el modelo, y por eso adivinarlos no funciona:

| Modelo | Esfuerzos |
| --- | --- |
| `gpt-5.6-terra` | low, medium, high, xhigh, max, **ultra** |
| `gpt-6-luna` | low, medium, high, xhigh, max |
| `gpt-5.5` | low, medium, high, xhigh |

El selector de esfuerzo, los autocompletados de `/model <name> <tab>` y la validación siguen todos
al modelo que esté activo. Cambiar de modelo arrastra el esfuerzo cuando el nuevo modelo lo acepta
y, si no, recurre al `default_reasoning_level` de ese modelo, así que nunca puedes acabar pidiendo un
esfuerzo que el modelo no tiene:

```
/model gpt-5.6-terra ultra   →  GPT-5.6-Terra ultra
/model gpt-5.5               →  GPT-5.5 medium          (ultra is not a gpt-5.5 level)
/model gpt-5.5 max           →  'max' is not a GPT-5.5 level;
                                this model takes low, medium, high, xhigh
```

Solo aparecen en el selector los modelos con `visibility: "list"`; los ocultos siguen siendo
alcanzables por nombre. Si falta el archivo del catálogo, la aplicación recurre a una lista integrada
conservadora para seguir siendo usable, y lo dice en el selector.

Solo responderán de verdad los modelos a los que tiene derecho la cuenta de ChatGPT con sesión
iniciada; el backend rechaza el resto con un error explicativo.

## Forma de la petición

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

Eventos consumidos: `response.output_text.delta`, `response.reasoning_summary_text.delta`,
`response.output_item.done`, `response.completed`, `response.failed`, `response.incomplete`,
`error`. Cualquier otro se ignora.

## Renderizado

Las respuestas del asistente se renderizan como texto enriquecido dentro del proceso. No hay
renderizador externo, ni `pandoc`, ni binario de TeX, ni rasterización de imágenes: todo ocurre
dentro de la TUI, así que no cuesta casi nada.

- **Markdown**: encabezados, negrita, cursiva, código en línea, bloques de código delimitados con un
  margen lateral y una etiqueta de lenguaje, listas con viñetas y numeradas, citas en bloque, reglas
  horizontales, enlaces, tablas de barras al estilo de GitHub maquetadas con bordes de dibujo de
  cajas y alineación por columna, y matemáticas `$…$` en línea.
- **Matemáticas LaTeX**: símbolos y relaciones, griego, `\frac`, `\sqrt`, subíndices y superíndices,
  `\mathbb` y `\mathcal`, acentos combinados (`\bar`, `\hat`, `\vec`, …), nombres de operadores en
  redonda (`\log`, `\sin`, `\lim`, …) y bloques en display `$$…$$`.

Todo lo que el conversor no reconoce se emite tal cual, llaves incluidas, así que la entrada
desconocida sigue siendo legible en vez de volverse papilla. Es un conversor, no un motor de TeX:
las macros, los entornos como `tikz` y las maquetaciones de varias líneas no se interpretan.

Markdown se analiza **una vez**, cuando llega un mensaje, en fragmentos independientes del tema; el
dibujado solo asigna colores a esos fragmentos. Por eso cambiar de tema no reanaliza nada.

Medido en el binario compilado: **~1 % de un núcleo en reposo** con el cursor parpadeando, ~3 %
durante un desplazamiento rápido con la rueda.

## Subagentes y comandos largos

**Subagentes.** La herramienta `task` ejecuta un segundo agente con su propio contexto y las mismas
herramientas, y luego devuelve solo su informe, útil para trabajo que de otro modo inundaría el
contexto de quien llama. El informe se corta a 6 000 caracteres, porque aterriza en el contexto de
quien llama, y la transcripción muestra su progreso en una sola línea que se reemplaza en cada paso
en vez de una línea por paso. Se ofrecen subagentes anidados mientras quede profundidad, con un tope
de dos niveles para que la recursión no se desboque. La transcripción muestra cada paso que da el
subagente.

**Los comandos lentos se pasan a segundo plano, no se matan.** Un comando en primer plano que sigue
en marcha tras **60 segundos** se mueve a segundo plano y se avisa al agente, en vez de bloquear el
turno:

```
'sleep 75' is still running after 60s, so it was moved to the background as 'bg-1'.
Carry on; you will be told when it finishes, or poll it with {"output_id": "bg-1"}.
```

**Terminar interrumpe.** Cuando un comando en segundo plano acaba, el turno en curso se aborta y se
le da la salida al agente, así que un resultado nunca se pierde en silencio detrás de un trabajo
largo:

```
• interrupted: a background command finished
```

## Dictado

El botón de micrófono de la esquina inferior (`ctrl-r` hace lo mismo) dicta en el prompt: púlsalo,
habla, púlsalo otra vez. El texto llega mientras hablas, porque el reconocedor transmite en flujo, y
el botón lleva un temporizador mientras escucha.

El reconocedor es **whistle** — el motor de voz en el dispositivo de Cactus — iniciado como
`whistle --mic`. Ese modo es lo que le hace transmitir en flujo: graba 16 kHz mono
en bruto desde el `pw-record` de pipewire un segundo cada vez, pasa cada segundo por el decodificador
en flujo e imprime una línea JSON por actualización (`text` es lo confirmado, `pending` lo que aún
está en vuelo). Un salto de línea en su stdin termina el flujo y vacía el final. El comando, el
idioma, las palabras de sesgo y la fuente de audio vienen del entorno:

El grabador es el de la propia plataforma: `pw-record` (pipewire) en Linux, `ffmpeg` con avfoundation
en macOS, `ffmpeg` con dshow en Windows (`WHISTLE_SOURCE` elige el dispositivo de entrada,
`WHISTLE_RECORDER` reemplaza el comando sin más). La aplicación solo lanza `EXP1_DICTATE --mic` y lee
sus líneas JSON, así que cualquier reconocedor que transmita igual sirve; sin uno, pulsar el botón
del micrófono lo dice en el pie y no ocurre nada más.

```sh
EXP1_DICTATE=/path/to/whistle     # default: whistle on PATH
EXP1_DICTATE_LANG=en              # default: whatever it detects
EXP1_DICTATE_KEYWORDS=country,Americans
WHISTLE_SOURCE=…                  # default: the system's own input
```

El dictado es solo para Linux, porque el grabador lo es; el resto del agente no.

Ese modo es local a esta copia de whistle: la lógica está en su biblioteca como un tipo `Dictation`
(`start`, `next`, `finish`) y su `main.rs` es un bucle fino sobre él. whistle no tiene repositorio
propio aquí, así que una copia nueva de su autor tiene que volver a aplicar esos dos archivos, o bien
apuntar `EXP1_DICTATE` a algo que hable el mismo protocolo. Un reconocedor que no conoce `--mic` no
se ignora: lo dice, con sus propias palabras, en el pie.

## Proveedores

`/login` abre un selector de proveedores; `/scoped-models` decide qué modelos de ellos ofrece
`/model` (vacío significa todos, y el ámbito se guarda en `settings.json`). El selector es una lista
alfabética simple, con el cursor abriéndose en el proveedor en uso, así que nada hay que buscarlo por
posición. Elegir un proveedor que ya tiene sesión iniciada cambia a él y lo dice; elegir el que ya
está en uso — o ejecutar `/login <id>` — vuelve a iniciar sesión a propósito. Un inicio de sesión por
código de dispositivo abre la página del proveedor en el navegador e imprime la URL y el código como
alternativa.

**Una clave puede cubrir varios proveedores.** `opencode-go` y `opencode-zen` son una sola cuenta
detrás de dos URL base, así que iniciar sesión en cualquiera de ellos guarda la clave para ambos
(`providers::key_group` nombra los grupos). Las pasarelas de opencode además enrutan por sesión: cada
petición lleva `x-opencode-session`, `x-opencode-project` y `x-opencode-client`, sin los cuales la
pasarela la rechaza de plano («Request is missing x-opencode-session»).

Cinco protocolos de cable cubren todo lo implementado aquí: la API **OpenAI chat-completions**,
compartida por una docena de proveedores, **OpenAI Responses**, **Anthropic messages**, el
**`generateContent` de Gemini** nativo de Vertex AI y el **backend de Codex**, con el que habla
`openai (auth)`. Todos transmiten SSE, y todos se decodifican en
el mismo flujo de eventos que produce el cliente de Codex, así que el bucle del agente no sabe con
cuál está hablando.

Las claves se guardan por proveedor en `~/.config/exp-1/credentials.json`, con modo `0600`.
`openai (auth)` mantiene en su lugar un `auth.json` al estilo de Codex, `google vertex` usa lo que ya
haya iniciado sesión `gcloud auth application-default login` (con `GOOGLE_CLOUD_PROJECT` y
`GOOGLE_CLOUD_LOCATION` nombrando el proyecto y la región), y los proveedores por código de
dispositivo ponen el token que reciben en el mismo archivo de credenciales.

| Proveedor | URL base | Protocolo |
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

Todas las entradas están implementadas: cada modelo nombra el protocolo en el que responde, así que
una pasarela como `opencode-go` puede servir tres a la vez. La lista y las URL base vienen de
[models.dev](https://models.dev), el registro público que lee el propio opencode, en vez de
conjeturas.

Todas las URL base anteriores se sondearon con una clave deliberadamente inválida: un rechazo
demuestra que la URL y la cabecera de autenticación son correctas. Las veintisiete respondieron con
`400`, `401` o `403` — excepto `fireworks-ai`, cuyo `404` es su propio `"Model not found"` con un id
de petición `chatcmpl-`, así que incluso esa petición llegó a la ruta de inferencia. `google vertex`
responde `401` en su ruta con ámbito de proyecto.

## Modos

`shift + tab` cambia la sesión entre dos modos, y el pie dice cuál está activo:

- **build** — todo como se describe aquí.
- **plan** — las herramientas que cambian archivos (`write`, `edit`, `delete`) se rechazan *y* se
  dejan fuera de la lista de herramientas, así que el modelo investiga y propone un plan en vez de
  editar. Se espera que `shell` siga siendo de solo lectura. El texto vive en `prompts/plan.md`,
  editable como el resto.

## Permisos

`~/.config/exp-1/permissions.json` es una lista de excepciones; una llamada que no coincide con
ninguna regla se ejecuta. La primera coincidencia decide:

```json
{
  "rules": [
    { "action": "shell", "resource": "rm -rf *", "effect": "deny" },
    { "action": "write", "resource": "/etc/*", "effect": "ask" },
    { "action": "fetch", "resource": "*", "effect": "allow" }
  ]
}
```

`action` es un nombre de herramienta o `*`, `resource` es lo que toca la llamada — una ruta para las
herramientas de archivos, la línea de comandos para `shell`, la URL o la consulta para las
herramientas de red — y `*`/`?` funcionan como comodines. `effect` es `allow`, `ask` o `deny`. Un
`ask` levanta el panel de pregunta y retiene la llamada hasta que se responde (dos minutos, y luego
la llamada se rechaza). Las reglas se aplican también a los subagentes.

`ctrl + shift + a` cambia el modo de permisos: **always ask** (el predeterminado de una instalación
nueva) hace que `write`, `edit`, `delete` y `shell` levanten la pregunta, **never ask** los ejecuta
sin preguntar, y **ask only commands** solo pregunta por `shell`. Las herramientas de solo lectura
nunca preguntan, y una regla con `effect: deny` rechaza en todos los modos: el modo nunca suaviza un
deny.

## Instrucciones del proyecto

`AGENTS.md` se lee del directorio de trabajo y de cada directorio padre por encima, del más externo
al más interno, y se añade al prompt de sistema, para que las reglas propias de un repositorio
lleguen al modelo sin pegarlas en la conversación. `/init` escribe un archivo inicial.

## Fallos del proveedor

Un fallo del servidor o del transporte se reintenta una vez tras dos segundos; una petición
rechazada (4xx) no. Cuando el modelo principal sigue fallando, `fallback_model` en `settings.json`
nombra un modelo del mismo proveedor para probarlo una vez antes de abandonar el turno, y el cambio
se anota en la transcripción.

## Habilidades

Una habilidad es un directorio con un `SKILL.md` dentro, la forma que usan Claude Code y opencode.
Instalar una la convierte en un comando:

```sh
exp-1 install ~/skills/pdf-tools          # a directory with SKILL.md
exp-1 install notes.md                    # a single markdown file
exp-1 install tools.zip                   # an archive (unzip or tar)
exp-1 install https://example.com/skill.md
```

Los archivos van a parar a `~/.config/exp-1/skills/<name>/`, con el nombre del campo `name:` del
frontmatter cuando lo tiene, y si no, el del archivo o directorio. Las habilidades instaladas
aparecen junto a los comandos integrados en el emergente de autocompletado y en la ayuda del estado
vacío, y `/<name>` ejecuta una: el servidor lee las instrucciones de la habilidad, las escribe en la
conversación como la tarea y deja que el modelo las siga. Lo que muestra el selector es el
`description:` del frontmatter.

## Servidores MCP

Las herramientas de los servidores [MCP](https://modelcontextprotocol.io) se ofrecen al modelo junto
a las integradas. La lista vive en `~/.config/exp-1/mcp.json`, exactamente con la forma que usa
opencode, así que una entrada puede copiarse entre los dos:

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

- Los servidores `local` se lanzan y se habla con ellos por stdio; a los `remote` se llega por HTTP,
  con una respuesta JSON o SSE. `"disabled": true` omite un servidor sin borrarlo, y `"headers"`
  añade lo que necesita un servidor remoto.
- Las herramientas llegan al modelo como `<server>_<tool>`, con ambas mitades saneadas como las
  sanea opencode (`context7_resolve-library-id`), así que dos servidores nunca pueden colisionar.
- Los servidores se conectan **en segundo plano** al arrancar: uno lento o muerto nunca retrasa al
  agente, y se informa de él en vez de descartarlo — `/mcp` lista cada servidor con su número de
  herramientas o el error con el que falló.

## Servidor en segundo plano

Modelado a partir del daemon de opencode. **Un arranque normal de `exp1-agent` levanta el servidor y se
conecta a él**, exactamente como el comando por defecto de opencode toma `daemon.transport()` y se lo
entrega a la interfaz.

La consecuencia es lo importante: cerrar la interfaz deja el servidor en marcha, así que el
siguiente arranque reutiliza un proceso caliente. Un servidor se reutiliza solo cuando es **a la
vez** la misma versión y la misma compilación: `server.json` registra el tamaño y la fecha de
modificación del ejecutable, así que un cliente recompilado desde que el servidor arrancó lo
reemplaza, en vez de hablar un protocolo que el servidor ya no habla. Medido en esta máquina:

| | arranque en frío | siguiente arranque |
| --- | --- | --- |
| Lanzamiento de la interfaz → primer fotograma | 189 ms | **84 ms** |
| RSS de la interfaz | 14.4 MB | 16.0 MB |
| RSS del servidor | 15.2 MB | 20.5 MB (tras un turno) |

Medido de nuevo con cinco sesiones abiertas a la vez:

| | un servidor + 5 interfaces | cinco procesos independientes |
| --- | --- | --- |
| en reposo | 97 MB (15 + 5 × 16) | 183 MB (5 × 37) |
| con el titler integrado | 143 MB | 423 MB (5 × 85) |

El servidor es perezoso con lo que carga: el cliente de Codex aparece en el primer turno (+5 MB) y
el titler integrado solo si se genera un título de verdad en local (+48 MB, y por eso `auto` prefiere
el modelo). Esa pereza es casi todo el sentido: los 48 MB de pesos se pagan una vez para todas las
sesiones de la máquina en vez de una vez por proceso.

El estado caro, una vez por máquina — el registro de herramientas y el modelo titler de ~49 MB —
vive en el único servidor, y el cliente de Codex se construye la primera vez que lo necesita una
petición del protocolo Codex, así que una máquina con sesión iniciada en cualquier otro proveedor
arranca el servidor sin credenciales de Codex. Cada interfaz es un cliente fino, así que N sesiones
en paralelo cuestan `1 x 63 MB + N x 15 MB` en vez de `N x 61 MB`, y solo el primer arranque paga un
arranque en frío.

```sh
exp1-agent serve                 # run the server in the foreground
exp1-agent service start         # spawn it detached (survives the launcher)
exp1-agent service status        # pid, url, version, health
exp1-agent service stop          # SIGTERM, then SIGKILL after a grace period
exp1-agent service restart
exp1-agent service password      # print, or set with an argument
```

- **Registro**: `~/.local/state/exp-1/server.json` contiene `{id, version, url, pid}`. Un cliente lo
  lee, comprueba `/health` con un tiempo límite de 2 s y reutiliza el servidor solo cuando la versión
  también coincide; un servidor obsoleto o ajeno se detiene primero.
- **Contraseña**: `~/.local/state/exp-1/password`, 32 bytes aleatorios, modo `0600`, se conserva
  entre reinicios para que un cliente recién lanzado se reconecte sin que se le diga nada.
- **Lanzamiento desprendido**: `exp1-agent serve --register` en su propio grupo de procesos y con los
  flujos estándar cerrados, para que sobreviva al cliente que lo inició.

### API HTTP

Todas las rutas requieren `Authorization: Bearer <password>`.

| Método | Ruta | Propósito |
| --- | --- | --- |
| `GET` | `/health` | vitalidad y versión |
| `GET` | `/sessions` | lista las sesiones activas |
| `POST` | `/sessions` | crea una; `x-exp1-directory` fija su directorio de trabajo |
| `GET` | `/sessions/{id}` | instantánea completa |
| `POST` | `/sessions/{id}/prompt` | inicia un turno |
| `GET` | `/sessions/{id}/events` | SSE: instantánea, deltas, herramientas, uso, título, fin, inactivo |
| `POST` | `/sessions/{id}/answer` | responde a un `ask_question` pendiente |
| `POST` | `/sessions/{id}/interrupt` | aborta el turno en curso |
| `GET` | `/models` | el catálogo, con los esfuerzos reales de cada modelo. `?all=1` ignora el ámbito: el selector que fija el ámbito necesita los modelos que quedan fuera |

Una sesión lleva su propio directorio de trabajo, así que un servidor sirve a varios proyectos; el
directorio llega con `POST /sessions` en vez de fijarse cuando arranca el servidor.

## Compactación automática

Una conversación larga se resume antes de que pueda desbordar la ventana de contexto. Después de cada
turno, el servidor compara los `input_tokens` de la petición con el `context_window` del modelo del
catálogo de Codex y compacta cuando la conversación llega al **95 %** de él:

```
Context at 7378 of 258400 tokens (95% threshold) — compacting
Conversation compacted; continuing from the summary
```

La compactación envía la conversación al modelo con un prompt de resumen y la plantilla estructurada
que usa opencode — Objective, Important Details, Work State (Completed / Active / Blocked), Next Move,
Relevant Files — con las salidas de herramientas recortadas a 2000 caracteres para que el resumidor
no se ahogue.

La conversación reescrita es **el resumen seguido de los últimos cinco turnos de usuario conservados
tal cual**, cada uno con las llamadas a herramientas del agente y sus resultados, para no perder el
hilo que se tiene entre manos:

```
[user]      Summary of everything so far: ## Objective …
[user]      Turn two: run shell ["seq","1","900"] and reply OK.
[call]      shell
[result]    exit_code: 0 1 2 3 …
[assistant] OK
… four more turns …
```

Un dato dicho antes de la compactación se sigue conociendo después. Si uno de esos turnos es a su vez
enorme, se van descartando las entradas más antiguas de la cola hasta que el resultado vuelva a caber
en la ventana, así que una sola salida de herramienta gigantesca no puede hacer que la conversación
se vuelva a compactar sin fin.

El porcentaje es `EXP1_COMPACT_PERCENT` si quieres otro, que además es la forma de ejercitar ese
camino sin llenar una ventana de 272 000 tokens.

Una conversación guarda hasta 5 000 mensajes de transcripción en disco; el contexto propio del modelo
se limita aparte mediante poda y compactación, así que una sesión larga sigue siendo legible sin
costar tokens.

## Persistencia

La configuración vive en `~/.config/exp-1/` (o `$XDG_CONFIG_HOME/exp-1`); las conversaciones en sí
viven en `~/.cache/exp-1/sessions/` (o `$XDG_CACHE_HOME/exp-1/sessions`), porque una transcripción es
una caché: perder una cuesta contexto, no configuración. Los archivos propios del daemon —
`server.json` y `password` — están en `~/.local/state/exp-1/`.

| Archivo | Contiene |
| --- | --- |
| `config/settings.json` | modelo, esfuerzo de razonamiento, tema, modo del titler, modelo de reserva |
| `config/credentials.json` | una clave de API por proveedor, modo `0600` |
| `config/prompts/*.md` | los prompts de sistema, editables: `system`, `subagent`, `compaction`, `title`, `plan` |
| `config/permissions.json` | las reglas de allow/ask/deny |
| `config/mcp.json` | los servidores MCP |
| `config/skills/` | habilidades instaladas, un directorio cada una |
| `cache/sessions/<id>.json` | una conversación: título, ambas transcripciones, historial del lado del modelo, historial de prompts |

Una conversación guarda sus **250 mensajes** más recientes en disco, y la caché entera se recorta a
**250 MB** — empezando por los archivos más antiguos — cuando arranca el servidor. Un diseño
anterior, en el que todo estaba en `config/exp-1/sessions/`, se migra en el primer arranque tras el
cambio.

Los ajustes se escriben al salir de la aplicación; una sesión se escribe tras cada turno completado,
así que un fallo cuesta como mucho el turno en curso.

El titler integrado es una funcionalidad opcional: `cargo build --release --no-default-features` lo
deja fuera y se lleva consigo toda la dependencia de Candle, y los títulos de sesión vienen entonces
solo del modelo. La CI compila así porque el titler arrastra Candle: el crate está en el árbol,
pero un release no tiene que pagar por ello.

Los prompts son archivos, no constantes: `~/.config/exp-1/prompts/system.md` (las instrucciones
propias del agente), `subagent.md`, `compaction.md` y `title.md` se escriben en el primer arranque y
se leen en cada petición, así que una edición se aplica al turno siguiente sin recompilar. El orden
de resolución del modelo y el esfuerzo al arrancar es **línea de comandos → ajustes guardados → el
primer modelo listado del catálogo**.

**Cada ejecución empieza en una sesión vacía.** No se reanuda nada automáticamente: `/sessions` abre
un selector de conversaciones guardadas, de la más reciente a la más antigua, cada una con su título
generado, su id y su última vez de uso. La lista se limita por directorio: una sesión es visible
donde se inició, en cualquier punto por debajo, y en todas partes si se inició en `$HOME` (las
«globales»). El propio `$HOME` lista solo las globales, para no convertirse en un catálogo de todos
los proyectos. Elegir una guarda la conversación que dejas y carga la elegida. `/clear` abandona la
conversación actual y empieza una nueva, conservando tus ajustes.

### Títulos de sesión

Los títulos vienen del **titler** integrado, un T5 local (7,6 M de parámetros, 3 capas de codificador
y 3 de decodificador) que se ejecuta con Candle en este proceso, incorporado como dependencia de ruta
desde `crates/titler`. Los pesos (`Cyronius/titler`) se leen de la caché de Hugging Face y se
cargan como mucho una vez por proceso, así que una vez están en disco la generación de títulos es
local, instantánea y gratuita.

La generación se ejecuta en un hilo bloqueante en cuanto termina el primer turno, nunca en el hilo de
la interfaz. Las sesiones con un padre — el chat lateral — no se nombran nunca, ya que pertenecen a
la conversación a la que siguen. Se usa el camino multilingüe de la biblioteca (no el crudo), así que
se detecta el idioma y un prompt que no esté en inglés se traduce al inglés primero. Las sesiones se
nombran una vez, y la vista previa tomada del prompt es lo que muestra el selector hasta que llega un
título.

**Quién nombra una sesión es un ajuste**, `titler` en `settings.json`:

| Valor | Comportamiento |
| --- | --- |
| `auto` (por defecto) | el modelo de Codex primero, el titler integrado como reserva |
| `local` | solo el titler integrado: sin gasto de API, títulos más flojos |
| `remote` | solo el modelo de Codex |

Los títulos se piden al nivel `luna` barato del catálogo en vez de al modelo propio de la sesión —
la misma idea que opencode pidiendo a un proveedor su modelo pequeño —, y los tokens que cuesta un
título se suman al uso de la sesión como cualquier otra llamada.

`/retitle` vuelve a nombrar la sesión, esta vez a partir de la petición original más la conversación
reciente (limitada a 8000 caracteres), que es como opencode regenera un título cuando una sesión se
renombra a nada. El prompt en sí vive en `prompts/title.md` y es el documento completo al estilo de
opencode: una tarea, las reglas (el mismo idioma que el usuario, ≤50 caracteres, conservar términos y
números exactos, no responder nunca a la pregunta) y diez ejemplos resueltos.

`auto` empieza con el modelo de Codex a propósito. El titler de 7,6 M es honesto sobre su tamaño: con
los mismos prompts en ruso produjo `Understanding Two Plus Terms` para «сколько будет два плюс два»
y `Request for Sign` para una petición de tabla, porque el traductor gratuito de Google traduce
«табличку» como "sign" y el modelo pequeño titula entonces la traducción errónea. El modelo de Codex
nombra los mismos prompts `Сколько будет два плюс два?` y `Формула корней квадратного уравнения в
LaTeX`. Ese camino de LLM es para lo que sirve `prompts/title.md`: el modelo integrado toma el primer
mensaje tal cual e ignora el archivo por completo.

## Estructura

| Archivo | Rol |
| --- | --- |
| `src/auth.rs` | Carga de credenciales, comprobación de caducidad del JWT, renovación, persistencia atómica |
| `src/store.rs` | Ajustes y archivos de sesión en `~/.config/exp-1/sessions/` |
| `src/catalog.rs` | Catálogo de modelos y esfuerzos de razonamiento desde la caché de Codex |
| `crates/titler` | Dependencia de ruta: el T5 local que nombra las sesiones |
| `src/protocol.rs` | Tipos de cable de petición/respuesta |
| `src/md.rs` | Markdown y tablas de barras a fragmentos con estilo |
| `src/latex.rs` | Matemáticas LaTeX a Unicode |
| `src/api.rs` | Transporte HTTP, decodificación de SSE, construcción de cabeceras |
| `src/tools.rs` | `read`, `write`, `edit`, `shell`, `ast`, `ask_question`, más el registro de segundo plano/salida |
| `src/app.rs` | Hilos, estado de la TUI, renderizado, instantánea/restauración de sesión |
| `src/main.rs` | CLI, gestión de teclas, bucle del agente |

## Límites

Un turno ejecuta como máximo 200 rondas de herramientas; si alguna vez llega ahí, la transcripción lo
dice en vez de pararse en silencio.

**Economía de tokens.** Las peticiones se mantienen pequeñas a propósito: `read` pagina, `grep` limita
sus coincidencias, la salida de un comando largo conserva ambos extremos, el informe de un subagente
se corta a 6 000 caracteres, y antes de cada petición las salidas de herramientas más antiguas — fuera
de los dos últimos turnos — se reemplazan por una nota de una línea que dice qué se descartó. El
modelo puede volver a ejecutar una herramienta para lo que aún necesite, y la transcripción que ve el
usuario conserva el texto completo.

**Llamadas en paralelo.** Un mensaje del modelo puede pedir varias herramientas a la vez. Las rachas
de llamadas de solo lectura (`read`, `grep`, `count`, `ast`, `fetch`, `websearch`) se ejecutan al
mismo tiempo; todo lo que escribe conserva su orden, así que una escritura seguida de una lectura del
mismo archivo no puede reordenarse.

Esto es un MVP. Huecos conocidos:

- Vertex solo acepta las credenciales predeterminadas de aplicación de gcloud; una clave de cuenta de
  servicio, que necesita un JWT firmado, no se acuña.
- El chat lateral ve toda la conversación principal en cada pregunta — el historial principal se lee,
  nunca se escribe —, así que una respuesta de `/btw` tiene el contexto completo mientras el hilo
  principal permanece intacto.
- No hay cancelación del flujo más allá de abortar la tarea.
- Los comandos se ejecutan sin sandbox.

## Verificado contra

Los detalles del protocolo se comprobaron contra el código de `openai/codex` y se confirmaron con
peticiones reales: el endpoint y las cabeceras, la rotación del token de refresco, el streaming de
texto, un ida y vuelta de `function_call` y un bucle de herramientas de varias iteraciones que
escribió un archivo.

## Créditos

- **El dictado** funciona con el modelo de voz **whistle** y el motor **Needle** de Cactus
  (`cactus-needle`, Apache-2.0): la página del producto es [cactuscompute.com/whistle](https://cactuscompute.com/whistle)
  y los pesos y motores por plataforma están en [Hugging Face](https://huggingface.co/Cactus-Compute/whistle).
  El motor los descarga en `~/.cache/cactus-needle`; nada de eso se incluye en este repositorio.
- **El titulador local opcional** se apoya en [Candle](https://github.com/huggingface/candle) de Hugging Face,
  bajo Apache-2.0 o MIT.
- Todo lo demás son los crates de `Cargo.toml`, cada uno bajo su propia licencia.

## Licencia

Este proyecto se distribuye únicamente bajo la GNU General Public License versión 3. Consulta [LICENSE](LICENSE).
Las dependencias y los recursos de terceros pueden tener sus propias licencias; consulta sus avisos y la sección Créditos anterior.
