/* The page's behaviour: a shell in the shape of a chat app, with the settings
   screens it supports so far. No framework, no build step, no network — this
   file ships inside the binary, the same as the styles. */
(() => {
  "use strict";

  const boot = JSON.parse(document.getElementById("boot")?.textContent || "{}");
  let web = Object.assign({}, boot.web || {});
  let sessions = boot.sessions || [];
  let accent = boot.accent || "#3b82f6";
  let saving = false;

  /* ------------------------------------------------------------------ words */

  const STRINGS = {
    ru: {
      newChat: "Новый чат", scheduled: "Запланировано", library: "Библиотека",
      plugins: "Плагины", explore: "Обзор", pinned: "Закреплённые", projects: "Проекты",
      recent: "Недавние", emptyPinned: "Закреплённых чатов пока нет",
      emptyProjects: "Проектов пока нет", emptyRecent: "Пока пусто",
      hello: "Что у вас сегодня на повестке?", ask: "Спросить exp-1",
      reasoning: "Быстрые ответы", reasoningOn: "Быстрые ответы включены",
      voice: "Голос", mic: "Микрофон",
      soonToast: "Эта часть появится в следующем шаге — пока работают настройки.",
      disclaimer: "exp-1 может ошибаться. Проверяйте важную информацию.",
      settings: "Настройки", back: "Вернуться в приложение", search: "Поиск",
      personalGroup: "Персональные настройки", general: "Общие",
      personalization: "Персонализация", appearance: "Внешний вид",
      notifications: "Уведомления", profile: "Профиль", security: "Безопасность и вход",
      account: "Учётная запись", archived: "Архивные чаты", parental: "Родительский контроль",
      trusted: "Доверенный контакт", voiceSection: "Голос", storage: "Хранилище",
      pets: "Питомцы", keyboard: "Клавиатура", usage: "Использование", payment: "Оплата",
      data: "Управление данными", integrations: "Интеграции", soon: "скоро",
      langName: "Язык", langDesc: "Язык пользовательского интерфейса приложения",
      langAuto: "Автоматически определять",
      memorySection: "Память", memoryName: "Включить память",
      memoryDesc: "Разрешить exp-1 учитывать ваши чаты и файлы, чтобы отвечать точнее",
      memorySummary: "Сводка памяти",
      memorySummaryDesc: "Посмотрите, что exp-1 о вас знает, в пользовательских инструкциях",
      manage: "Управление",
      styleSection: "Базовый стиль и тон", styleName: "Стиль и тон",
      styleDesc: "Как модель разговаривает во всех чатах",
      styleDefault: "По умолчанию", stylePro: "Профессиональный",
      styleFriendly: "Дружелюбный", styleCandid: "Прямой",
      friendliness: "Дружелюбие", enthusiasm: "Энтузиазм", emoji: "Эмодзи",
      visualReplies: "Визуальные ответы", headings: "Заголовки и списки",
      less: "Менее", def: "По умолчанию", more: "Более",
      customSection: "Пользовательские инструкции", customName: "exp-1",
      customDesc: "Добавьте дополнительные инструкции и контекст для всех чатов",
      customHint: "Сохраняется автоматически и попадает в системный промпт каждого чата.",
      extraSection: "Дополнительные настройки", fast: "Быстрые ответы",
      fastDesc: "Отвечать с минимальным ризонингом или без него, если провайдер это поддерживает",
      suggested: "Предлагаемые промпты", suggestedDesc: "Показывать примеры запросов на пустом экране",
      history: "Использовать историю записей", historyDesc: "Учитывать прошлые разговоры при ответе",
      modelsSection: "Модели", modelsDesc: "Три модели выбираются командой /web-models в TUI",
      tierFast: "Быстрая", tierMedium: "Средняя", tierPowerful: "Мощная",
      tierUnset: "не задана", saved: "Сохранено", saveFailed: "Не сохранилось — попробуйте ещё раз",
      noKey: "Нет доступа: откройте страницу заново через /web в TUI",
      soonTitle: "Скоро", soonBody: "Этот раздел ещё не подключён. Сейчас работают «Общие», «Внешний вид» и «Персонализация».",
      modeSection: "Визуальный стиль", modeName: "Режим",
      modeLight: "Светлый", modeDark: "Тёмный", modeAuto: "Как в системе",
      themeName: "Тема", themeDesc: "Тема TUI задаёт акцентный цвет этой страницы",
      accentName: "Акцентный цвет", bgName: "Фон", fgName: "Цвет переднего плана",
      fromTui: "меняется командой /themes в TUI", advanced: "Расширенные",
    },
    en: {
      newChat: "New chat", scheduled: "Scheduled", library: "Library",
      plugins: "Plugins", explore: "Explore", pinned: "Pinned", projects: "Projects",
      recent: "Recent", emptyPinned: "No pinned chats yet",
      emptyProjects: "No projects yet", emptyRecent: "Nothing yet",
      hello: "What is on your mind today?", ask: "Ask exp-1",
      reasoning: "Fast answers", reasoningOn: "Fast answers are on",
      voice: "Voice", mic: "Microphone",
      soonToast: "This part lands in the next step — settings work now.",
      disclaimer: "exp-1 can make mistakes. Check important information.",
      settings: "Settings", back: "Back to the app", search: "Search",
      personalGroup: "Personal settings", general: "General",
      personalization: "Personalization", appearance: "Appearance",
      notifications: "Notifications", profile: "Profile", security: "Security and login",
      account: "Account", archived: "Archived chats", parental: "Parental controls",
      trusted: "Trusted contact", voiceSection: "Voice", storage: "Storage",
      pets: "Pets", keyboard: "Keyboard", usage: "Usage", payment: "Payment",
      data: "Data controls", integrations: "Integrations", soon: "soon",
      langName: "Language", langDesc: "Language of the application interface",
      langAuto: "Detect automatically",
      memorySection: "Memory", memoryName: "Enable memory",
      memoryDesc: "Let exp-1 use your chats and files to answer more accurately",
      memorySummary: "Memory summary",
      memorySummaryDesc: "See what exp-1 knows about you in the custom instructions",
      manage: "Manage",
      styleSection: "Base style and tone", styleName: "Style and tone",
      styleDesc: "How the model talks in every chat",
      styleDefault: "Default", stylePro: "Professional",
      styleFriendly: "Friendly", styleCandid: "Candid",
      friendliness: "Friendliness", enthusiasm: "Enthusiasm", emoji: "Emoji",
      visualReplies: "Visual replies", headings: "Headings and lists",
      less: "Less", def: "Default", more: "More",
      customSection: "Custom instructions", customName: "exp-1",
      customDesc: "Add instructions and context for every chat",
      customHint: "Saved automatically; it joins the system prompt of every chat.",
      extraSection: "Additional settings", fast: "Fast answers",
      fastDesc: "Answer with the lowest reasoning, or none at all, when the provider allows it",
      suggested: "Suggested prompts", suggestedDesc: "Show example requests on the empty screen",
      history: "Use record history", historyDesc: "Use earlier conversations when answering",
      modelsSection: "Models", modelsDesc: "Pick the three models with /web-models in the TUI",
      tierFast: "Fast", tierMedium: "Medium", tierPowerful: "Powerful",
      tierUnset: "not set", saved: "Saved", saveFailed: "Not saved — try again",
      noKey: "No access: open the page again through /web in the TUI",
      soonTitle: "Soon", soonBody: "This section is not wired up yet. General, Appearance and Personalization work now.",
      modeSection: "Visual style", modeName: "Mode",
      modeLight: "Light", modeDark: "Dark", modeAuto: "Match system",
      themeName: "Theme", themeDesc: "The TUI's theme sets this page's accent colour",
      accentName: "Accent colour", bgName: "Background", fgName: "Foreground colour",
      fromTui: "change it with /themes in the TUI", advanced: "Advanced",
    },
  };

  const language = () => {
    if (web.language && STRINGS[web.language]) return web.language;
    const tag = (navigator.language || boot.accept_language || "en").toLowerCase();
    return /^(ru|uk|be|kk|ky|tg|hy|az)/.test(tag) ? "ru" : "en";
  };
  const t = (key) => (STRINGS[language()] || STRINGS.en)[key] ?? STRINGS.en[key] ?? key;

  /* ------------------------------------------------------------------ icons */

  const icon = (body) =>
    `<svg width="18" height="18" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="1.7" stroke-linecap="round" stroke-linejoin="round">${body}</svg>`;

  const icons = {
    plus: icon('<path d="M12 5v14M5 12h14"/>'),
    pencil: icon('<path d="M4 20h4l10-10-4-4L4 16v4z"/><path d="M14 6l4 4"/>'),
    clock: icon('<circle cx="12" cy="12" r="9"/><path d="M12 7v5l3 2"/>'),
    book: icon('<path d="M5 4h9a2 2 0 0 1 2 2v14H7a2 2 0 0 1-2-2z"/><path d="M16 6h3v14H9"/>'),
    grid: icon('<path d="M4 9h6V4H4zM14 9h6V4h-6zM4 20h6v-5H4zM14 20h6v-5h-6z"/>'),
    dots: icon('<circle cx="5" cy="12" r="1.4"/><circle cx="12" cy="12" r="1.4"/><circle cx="19" cy="12" r="1.4"/>'),
    search: icon('<circle cx="11" cy="11" r="7"/><path d="M20 20l-3.5-3.5"/>'),
    panel: icon('<rect x="3" y="4" width="18" height="16" rx="2"/><path d="M9 4v16"/>'),
    back: icon('<path d="M15 6l-6 6 6 6"/>'),
    chevron: icon('<path d="M6 9l6 6 6-6"/>'),
    sliders: icon('<path d="M4 7h10M18 7h2M4 17h4M12 17h8"/><circle cx="16" cy="7" r="2"/><circle cx="10" cy="17" r="2"/>'),
    person: icon('<circle cx="12" cy="8" r="3.4"/><path d="M5 20a7 7 0 0 1 14 0"/>'),
    palette: icon('<circle cx="12" cy="12" r="9"/><circle cx="9" cy="10" r="1.2"/><circle cx="15" cy="10" r="1.2"/><path d="M12 21a3 3 0 0 1 0-6"/>'),
    bell: icon('<path d="M6 9a6 6 0 1 1 12 0c0 5 2 6 2 6H4s2-1 2-6z"/><path d="M10 20a2 2 0 0 0 4 0"/>'),
    shield: icon('<path d="M12 3l7 3v6c0 5-3 8-7 9-4-1-7-4-7-9V6z"/>'),
    card: icon('<rect x="3" y="6" width="18" height="12" rx="2"/><path d="M3 10h18"/>'),
    box: icon('<path d="M4 8l8-4 8 4v8l-8 4-8-4z"/><path d="M4 8l8 4 8-4M12 12v8"/>'),
    puzzle: icon('<path d="M9 4h6v3a2 2 0 1 0 0 4h3v6h-3a2 2 0 1 1-4 0H8v-6h3a2 2 0 1 0 0-4z"/>'),
    key: icon('<circle cx="8" cy="12" r="4"/><path d="M12 12h9M18 12v4M15 12v3"/>'),
    mic: icon('<rect x="9" y="3" width="6" height="11" rx="3"/><path d="M5 12a7 7 0 0 0 14 0M12 19v2"/>'),
    wave: icon('<path d="M4 12h2M8 8v8M12 5v14M16 9v6M20 12h0.5"/>'),
    sparkle: icon('<path d="M12 4l1.6 4.4L18 10l-4.4 1.6L12 16l-1.6-4.4L6 10l4.4-1.6z"/>'),
  };

  /* ------------------------------------------------------------------ atoms */

  const el = (tag, props = {}, children = []) => {
    const node = document.createElement(tag);
    for (const [key, value] of Object.entries(props)) {
      if (value === undefined || value === null || value === false) continue;
      if (key === "class") node.className = value;
      else if (key === "text") node.textContent = value;
      else if (key === "html") node.innerHTML = value;
      else if (key.startsWith("on")) node.addEventListener(key.slice(2), value);
      else node.setAttribute(key, value === true ? "" : value);
    }
    for (const child of [].concat(children)) if (child) node.append(child);
    return node;
  };

  const toast = (text) => {
    const node = document.getElementById("toast");
    node.textContent = text;
    node.hidden = false;
    clearTimeout(node._timer);
    node._timer = setTimeout(() => (node.hidden = true), 2600);
  };

  const soon = () => toast(t("soonToast"));

  async function save() {
    if (saving) return;
    saving = true;
    try {
      const response = await fetch("/api/config", {
        method: "POST",
        headers: { "content-type": "application/json" },
        body: JSON.stringify(web),
      });
      if (response.status === 401) return toast(t("noKey"));
      if (!response.ok) return toast(t("saveFailed"));
      const body = await response.json();
      web = Object.assign(web, body.web || {});
      toast(t("saved"));
    } catch {
      toast(t("saveFailed"));
    } finally {
      saving = false;
    }
  }

  /* ------------------------------------------------------------- navigation */

  const route = () => location.hash.replace(/^#/, "") || "/";
  const go = (path) => {
    location.hash = "#" + path;
  };
  window.addEventListener("hashchange", render);

  /* ---------------------------------------------------------------- sidebar */

  const sideRow = (glyph, label, path, extra) =>
    el("button", { class: "side-row", onclick: () => go(path) }, [
      el("span", { html: glyph }),
      el("span", { text: label }),
      extra ? el("span", { class: "count", text: extra }) : null,
    ]);

  function sidebarNodes() {
    const pinned = (web.pinned || [])
      .map((id) => sessions.find((session) => session.id === id))
      .filter(Boolean);
    const list = el("div", { class: "side-scroll" });
    for (const session of pinned) {
      list.append(el("button", { class: "side-item", text: session.title, onclick: soon }));
    }
    if (!pinned.length) list.append(el("div", { class: "side-empty", text: t("emptyPinned") }));
    list.append(el("div", { class: "side-label", text: t("projects") }));
    list.append(el("div", { class: "side-empty", text: t("emptyProjects") }));
    list.append(el("div", { class: "side-label", text: t("recent") }));
    if (!sessions.length) list.append(el("div", { class: "side-empty", text: t("emptyRecent") }));
    for (const session of sessions) {
      list.append(el("button", { class: "side-item", text: session.title, title: session.when, onclick: soon }));
    }

    return [
      el("div", { class: "side-top" }, [
        el("div", { class: "brand" }, [
          el("span", { class: "brand-mark", text: ">_" }),
          el("span", { text: "exp-1" }),
        ]),
        el("div", { class: "side-icons" }, [
          el("button", { class: "icon-btn", html: icons.search, title: t("search"), onclick: soon }),
          el("button", {
            class: "icon-btn",
            html: icons.panel,
            title: t("settings"),
            onclick: () => go("/settings/general"),
          }),
        ]),
      ]),
      el("button", { class: "side-row primary", onclick: () => go("/") }, [
        el("span", { html: icons.pencil }),
        el("span", { text: t("newChat") }),
      ]),
      sideRow(icons.clock, t("scheduled"), "/scheduled", String(boot.scheduled ?? 0)),
      sideRow(icons.book, t("library"), "/library"),
      sideRow(icons.grid, t("plugins"), "/plugins", String(boot.plugins ?? 0)),
      sideRow(icons.dots, t("explore"), "/explore"),
      list,
      el("div", { class: "side-foot" }, [
        el("div", { class: "profile", onclick: () => go("/settings/personalization") }, [
          el("div", { class: "avatar", text: "exp" }),
          el("div", {}, [
            el("div", { class: "profile-name", text: boot.model || "exp-1" }),
            el("div", { class: "profile-sub", text: boot.provider || "-" }),
          ]),
        ]),
      ]),
    ];
  }

  /* ------------------------------------------------------------------- chat */

  function chatView() {
    const input = el("input", {
      placeholder: t("ask"),
      onkeydown: (event) => {
        if (event.key === "Enter") soon();
      },
    });
    const fast = el("button", { class: "pill" + (web.fast_answers ? " on" : "") }, [
      el("span", { html: icons.sparkle }),
      el("span", { text: t("reasoning") }),
    ]);
    fast.addEventListener("click", () => {
      web.fast_answers = !web.fast_answers;
      fast.classList.toggle("on", !!web.fast_answers);
      toast(web.fast_answers ? t("reasoningOn") : t("reasoning"));
      save();
    });

    const view = el("div", { class: "chat-view" }, [
      el("div", { class: "chat-hello", text: t("hello") }),
      el("div", { class: "composer" }, [
        el("button", { class: "plus", html: icons.plus, onclick: soon }),
        input,
        el("div", { class: "tools" }, [
          fast,
          el("button", { class: "mic", html: icons.mic, title: t("mic"), onclick: soon }),
          el("button", { class: "voice", html: icons.wave, title: t("voice"), onclick: soon }),
        ]),
      ]),
      el("div", { class: "chat-under", text: t("disclaimer") }),
    ]);
    if (web.suggested_prompts) {
      const chips = el("div", { class: "chat-chips" });
      chips.append(el("button", { class: "chip", text: "/schedule", onclick: soon }));
      chips.append(el("button", { class: "chip", text: "/web-models", onclick: soon }));
      chips.append(
        el("button", { class: "chip", text: t("settings"), onclick: () => go("/settings/general") }),
      );
      view.append(chips);
    }
    return view;
  }

  /* --------------------------------------------------------- settings: bits */

  const SETTINGS_ROWS = [
    { key: "general", icon: icons.sliders, path: "/settings/general" },
    { key: "appearance", icon: icons.palette, path: "/settings/appearance" },
    { key: "personalization", icon: icons.person, path: "/settings/personalization" },
    { key: "notifications", icon: icons.bell },
    { key: "profile", icon: icons.person },
    { key: "security", icon: icons.shield },
    { key: "account", icon: icons.person },
    { key: "archived", icon: icons.box },
    { key: "parental", icon: icons.shield },
    { key: "trusted", icon: icons.person },
    { key: "voiceSection", icon: icons.mic },
    { key: "storage", icon: icons.box },
    { key: "pets", icon: icons.person },
    { key: "keyboard", icon: icons.key },
    { key: "usage", icon: icons.sliders },
    { key: "payment", icon: icons.card },
    { key: "data", icon: icons.grid },
    { key: "integrations", icon: icons.puzzle },
  ];

  const card = (children) => el("div", { class: "card" }, children);

  const row = (name, desc, control) =>
    el("div", { class: "row" }, [
      el("div", { class: "text" }, [
        el("div", { class: "name", text: name }),
        desc ? el("div", { class: "desc", text: desc }) : null,
      ]),
      control ? el("div", { class: "control" }, [control]) : el("span", { class: "spacer", text: "›" }),
    ]);

  const section = (title) => el("div", { class: "set-section", text: title });

  const toggle = (key) => {
    const node = el("button", {
      class: "toggle" + (web[key] ? " on" : ""),
      role: "switch",
      "aria-checked": String(!!web[key]),
    });
    node.addEventListener("click", () => {
      web[key] = !web[key];
      node.classList.toggle("on", !!web[key]);
      node.setAttribute("aria-checked", String(!!web[key]));
      save();
    });
    return node;
  };

  function closeMenus() {
    for (const menu of document.querySelectorAll(".menu")) menu.remove();
  }
  document.addEventListener("click", (event) => {
    if (!event.target.closest(".menu") && !event.target.closest(".select")) closeMenus();
  });

  const select = (key, options, after) => {
    const node = el("button", { class: "select" });
    const label = () =>
      (options.find((option) => option.value === (web[key] || options[0].value)) || options[0]).label;
    node.append(el("span", { text: label() }), el("span", { html: icons.chevron }));
    node.addEventListener("click", (event) => {
      event.stopPropagation();
      closeMenus();
      const menu = el("div", { class: "menu" });
      for (const option of options) {
        menu.append(
          el("button", {
            class: option.value === (web[key] || options[0].value) ? "on" : "",
            text: option.label,
            onclick: () => {
              web[key] = option.value;
              closeMenus();
              node.firstChild.textContent = option.label;
              save();
              if (after) after();
            },
          }),
        );
      }
      document.body.append(menu);
      const box = node.getBoundingClientRect();
      menu.style.left =
        Math.max(12, Math.min(box.left, window.innerWidth - menu.offsetWidth - 12)) + "px";
      menu.style.top = Math.min(box.bottom + 6, window.innerHeight - menu.offsetHeight - 12) + "px";
    });
    return node;
  };

  const three = (key) =>
    select(key, [
      { value: "less", label: t("less") },
      { value: "default", label: t("def") },
      { value: "more", label: t("more") },
    ]);

  const swatch = (colour) =>
    el("span", { class: "swatch", style: `background:${colour}`, html: "" });

  const valuePill = (text) => el("span", { class: "select static", text });

  /* ------------------------------------------------------ settings: screens */

  function settingsNav(active) {
    const filter = el("input", { placeholder: t("search") });
    const list = el("div");
    const paint = () => {
      list.textContent = "";
      const needle = filter.value.trim().toLowerCase();
      for (const item of SETTINGS_ROWS) {
        const label = t(item.key);
        if (needle && !label.toLowerCase().includes(needle)) continue;
        list.append(
          el(
            "button",
            {
              class:
                "set-link" + (item.path === active ? " active" : "") + (item.path ? "" : " soon"),
              onclick: item.path ? () => go(item.path) : soon,
            },
            [
              el("span", { html: item.icon }),
              el("span", { text: label }),
              item.path ? null : el("span", { class: "tag", text: t("soon") }),
            ],
          ),
        );
      }
    };
    filter.addEventListener("input", paint);
    paint();
    return el("div", { class: "set-nav" }, [
      el("button", { class: "set-back", onclick: () => go("/") }, [
        el("span", { html: icons.back }),
        el("span", { text: t("back") }),
      ]),
      el("div", { class: "set-search" }, [el("span", { html: icons.search }), filter]),
      el("div", { class: "set-group", text: t("personalGroup") }),
      list,
    ]);
  }

  function generalScreen() {
    return el("div", { class: "set-page" }, [
      el("h1", { class: "set-title", text: t("general") }),
      card([
        row(
          t("langName"),
          t("langDesc"),
          select("language", [
            { value: "", label: t("langAuto") },
            { value: "ru", label: "Русский" },
            { value: "en", label: "English" },
          ], render),
        ),
      ]),
    ]);
  }

  function appearanceScreen() {
    const modes = [
      { label: t("modeLight"), cls: "mode light" },
      { label: t("modeDark"), cls: "mode dark on" },
      { label: t("modeAuto"), cls: "mode auto" },
    ].map((mode) =>
      el("button", { class: mode.cls, title: mode.label, onclick: soon }, [
        el("span", { class: "mode-bar" }),
        el("span", { class: "mode-line" }),
        el("span", { class: "mode-line short" }),
        el("span", { class: "mode-dot" }),
      ]),
    );

    return el("div", { class: "set-page" }, [
      el("h1", { class: "set-title", text: t("appearance") }),
      section(t("modeSection")),
      card([
        el("div", { class: "row" }, [
          el("div", { class: "text" }, [el("div", { class: "name", text: t("modeName") })]),
          el("div", { class: "control modes" }, modes),
        ]),
      ]),
      el("div", { class: "set-section spacer-line" }),
      card([
        row(
          t("themeName"),
          t("themeDesc"),
          el("span", { class: "select static" }, [
            el("span", { class: "aa", text: "Aa" }),
            el("span", { text: boot.theme || "default" }),
          ]),
        ),
        row(
          t("accentName"),
          null,
          el("span", { class: "select static" }, [swatch(accent), el("span", { text: accent })]),
        ),
        row(t("bgName"), null, el("span", { class: "select static" }, [swatch("#212121"), el("span", { text: "#212121" })])),
        row(
          t("fgName"),
          null,
          el("span", { class: "select static" }, [swatch("#ececec"), el("span", { text: "#ececec" })]),
        ),
      ]),
      el("div", { class: "hint", text: t("fromTui") }),
      el("div", { class: "set-section soon-row", onclick: soon, text: t("advanced") + " ›" }),
    ]);
  }

  function personalizationScreen() {
    const textarea = el("textarea", { placeholder: t("customDesc") });
    textarea.value = web.custom_instructions || "";
    textarea.addEventListener("blur", () => {
      web.custom_instructions = textarea.value;
      save();
    });
    const focusInstructions = () => {
      textarea.focus();
      textarea.scrollIntoView({ block: "center", behavior: "smooth" });
    };

    const tiers = web.tiers || {};
    const tierRows = [
      ["fast", t("tierFast")],
      ["medium", t("tierMedium")],
      ["powerful", t("tierPowerful")],
    ].map(([slot, label]) => row(label, tiers[slot] || t("tierUnset"), null));

    return el("div", { class: "set-page" }, [
      el("h1", { class: "set-title", text: t("personalization") }),
      section(t("memorySection")),
      card([
        row(t("memoryName"), t("memoryDesc"), toggle("memory")),
        el("div", { class: "row link", onclick: focusInstructions }, [
          el("div", { class: "text" }, [
            el("div", { class: "name", text: t("memorySummary") }),
            el("div", { class: "desc", text: t("memorySummaryDesc") }),
          ]),
          el("div", { class: "control" }, [el("span", { class: "button", text: t("manage") })]),
        ]),
      ]),
      section(t("styleSection")),
      card([
        row(
          t("styleName"),
          t("styleDesc"),
          select("style", [
            { value: "default", label: t("styleDefault") },
            { value: "professional", label: t("stylePro") },
            { value: "friendly", label: t("styleFriendly") },
            { value: "candid", label: t("styleCandid") },
          ]),
        ),
        row(t("friendliness"), null, three("friendliness")),
        row(t("enthusiasm"), null, three("enthusiasm")),
        row(t("emoji"), null, three("emoji")),
        row(
          t("visualReplies"),
          null,
          select("visual_replies", [
            { value: "default", label: t("def") },
            { value: "less", label: t("less") },
          ]),
        ),
        row(t("headings"), null, three("headings")),
      ]),
      section(t("customSection")),
      card([
        el("div", { class: "instructions" }, [
          textarea,
          el("div", { class: "hint", text: t("customHint") }),
        ]),
      ]),
      section(t("modelsSection")),
      card(tierRows),
      el("div", { class: "hint", text: t("modelsDesc") }),
      section(t("extraSection")),
      card([
        row(t("fast"), t("fastDesc"), toggle("fast_answers")),
        row(t("suggested"), t("suggestedDesc"), toggle("suggested_prompts")),
        row(t("history"), t("historyDesc"), toggle("record_history")),
      ]),
    ]);
  }

  const SCREENS = {
    "/settings/general": generalScreen,
    "/settings/appearance": appearanceScreen,
    "/settings/personalization": personalizationScreen,
  };

  const CHAT_LINKS = ["scheduled", "library", "plugins", "explore"];

  function soonPage(key) {
    const title = CHAT_LINKS.includes(key) ? t(key) : t("soonTitle");
    return el("div", { class: "soon-page" }, [
      el("div", {}, [el("h2", { text: title }), el("p", { text: t("soonBody") })]),
    ]);
  }

  /* ------------------------------------------------------------------ paint */

  function render() {
    const current = route();
    document.documentElement.lang = language();
    const sidebar = document.getElementById("sidebar");
    const main = document.getElementById("main");
    sidebar.textContent = "";
    sidebar.append(...sidebarNodes());
    main.textContent = "";
    if (current === "/") main.append(chatView());
    else if (SCREENS[current]) main.append(el("div", { class: "settings" }, [settingsNav(current), el("div", { class: "set-body" }, [SCREENS[current]()])]));
    else main.append(soonPage(current.replace("/", "")));
  }

  /* The accent is the TUI's theme; a page left open picks up a change when the
     window is looked at again. */
  window.addEventListener("focus", async () => {
    try {
      const response = await fetch("/api/config");
      if (!response.ok) return;
      const body = await response.json();
      const fresh = body.web || {};
      const accentNow = body.accent || accent;
      if (accentNow !== accent) {
        accent = accentNow;
        document.documentElement.style.setProperty("--accent", accent);
        document.documentElement.style.setProperty("--accent-soft", accent + "26");
        render();
      }
      web = Object.assign(web, fresh);
    } catch {
      /* a closed server is not worth a message here */
    }
  });

  render();
})();
