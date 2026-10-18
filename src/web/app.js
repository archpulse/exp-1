/* The page's behaviour: a shell in the shape of a chat app, with its settings.
   No framework, no build step, no network — this file ships inside the binary,
   the same as the styles. */
(() => {
  "use strict";

  const boot = JSON.parse(document.getElementById("boot")?.textContent || "{}");
  let web = Object.assign({}, boot.web || {});
  /* The page has its own conversations; the agent's sessions stay in the TUI. */
  let sessions = [];
  let saving = false;
  let advancedOpen = false;

  /* ------------------------------------------------------------------ words */

  const STRINGS = {
    ru: {
      newChat: "Новый чат", scheduled: "Запланировано", plugins: "Плагины",
      pinned: "Закреплённые", projects: "Проекты", recent: "Недавние",
      emptyPinned: "Закреплённых чатов пока нет", emptyProjects: "Проектов пока нет",
      emptyRecent: "Пока пусто", hello: "Что у вас сегодня на повестке?",
      helloNamed: "Чем могу помочь,", reasoningOff: "Быстрые ответы включены",
      ask: "Спросить exp-1", reasoning: "Быстрые ответы", reasoningLabel: "Размышление",
      reasoningHint: "Выключить размышление — это и есть быстрые ответы",
      reasoningOn: "Быстрые ответы включены", reasoningFast: "Размышление выключено",
      voice: "Голос", mic: "Микрофон", nextStep: "Чат появится в следующем шаге.",
      suggestSchedule: "Запланировать задачу", suggestSettings: "Настроить внешний вид",
      suggestModels: "Выбрать модели страницы",
      disclaimer: "exp-1 может ошибаться. Проверяйте важную информацию.",
      settings: "Настройки", back: "Вернуться в приложение", search: "Поиск",
      personalGroup: "Персональные настройки", general: "Общие",
      appearance: "Внешний вид", personalization: "Персонализация",
      notifications: "Уведомления", profile: "Профиль", security: "Безопасность и вход",
      account: "Учётная запись", archived: "Архивные чаты", parental: "Родительский контроль",
      trusted: "Доверенный контакт", voiceSection: "Голос", storage: "Хранилище",
      pets: "Питомцы", keyboard: "Клавиатура", usage: "Использование", payment: "Оплата",
      data: "Управление данными", integrations: "Интеграции",
      langName: "Язык", langDesc: "Язык пользовательского интерфейса приложения",
      langAuto: "Автоматически определять",
      memorySection: "Память", memoryName: "Включить память",
      memoryIntro: "Настройте, как exp-1 обращается с памятью.",
      memoryDesc: "Разрешить exp-1 учитывать ваши чаты и файлы, чтобы отвечать точнее",
      memorySummary: "Сводка памяти",
      memorySummaryDesc: "Что exp-1 о вас знает, лежит в пользовательских инструкциях",
      memoryFoot: "exp-1 может использовать память, чтобы отвечать точнее по прошлым разговорам.",
      manage: "Управление", styleSection: "Базовый стиль и тон", styleName: "Стиль и тон",
      styleDesc: "Как модель разговаривает во всех чатах", styleDefault: "По умолчанию",
      stylePro: "Профессиональный", styleFriendly: "Дружелюбный", styleCandid: "Прямой",
      friendliness: "Дружелюбие", enthusiasm: "Энтузиазм", emoji: "Эмодзи",
      visualReplies: "Визуальные ответы", headings: "Заголовки и списки",
      less: "Менее", def: "По умолчанию", more: "Более",
      customSection: "Пользовательские инструкции", customDesc: "Добавьте дополнительные инструкции и контекст для всех чатов",
      customHint: "Сохраняется автоматически и попадает в системный промпт каждого чата.",
      modelsSection: "Модели", modelsDesc: "Три модели выбираются командой /web-models в TUI",
      tierFast: "Быстрая", tierMedium: "Средняя", tierPowerful: "Мощная", tierUnset: "не задана",
      extraSection: "Дополнительные настройки", fast: "Быстрые ответы",
      fastDesc: "Отвечать с минимальным ризонингом или без него, если провайдер это поддерживает",
      suggested: "Предлагаемые промпты", suggestedDesc: "Показывать примеры запросов на пустом экране",
      history: "Использовать историю записей", historyDesc: "Учитывать прошлые разговоры при ответе",
      modeSection: "Визуальный стиль", modeName: "Режим", modeLight: "Светлый",
      modeDark: "Тёмный", modeAuto: "Как в системе", themeName: "Тема",
      themeDesc: "Тема TUI задаёт акцент, пока свой не выбран",
      accentName: "Акцентный цвет", accentDefault: "По умолчанию", accentBlue: "Синий",
      accentGreen: "Зелёный", accentYellow: "Жёлтый", accentPink: "Розовый",
      accentOrange: "Оранжевый", accentPurple: "Фиолетовый", accentWhite: "Белый",
      accentCustom: "Свой цвет", bgName: "Фон", fgName: "Цвет переднего плана",
      advanced: "Расширенные", uiFont: "Размер шрифта интерфейса",
      uiFontDesc: "Базовый размер текста страницы", codeFont: "Размер шрифта кода",
      codeFontDesc: "Шрифт кода в чатах и диффах", motionName: "Уменьшение движения",
      motionDesc: "Меньше анимаций: как в системе, всегда или никогда",
      motionSystem: "Системный", motionOn: "Вкл.", motionOff: "Выкл.",
      separateThemes: "Раздельные режимы светлой и тёмной тем",
      separateThemesDesc: "Светлая тема появится позже; выбор сохранится",
      contrastName: "Контраст", diffName: "Маркеры различий",
      diffDesc: "Показывать изменения цветом или знаками +/−",
      diffColor: "Цвет", diffSigns: "+/−",
      jobsEmpty: "Расписаний нет — заведите их командой /schedule в TUI",
      jobDone: "выполнено", jobPaused: "на паузе", jobEvery: "каждые", jobDue: "срок",
      jobsHint: "Задачи создаются, запускаются и хранят историю в TUI; здесь их видно и можно поставить на паузу",
      pause: "Пауза", resume: "Продолжить", runNow: "Запустить", remove: "Удалить",
      pluginsEmpty: "Плагинов нет — добавьте MCP-серверы в файл",
      pluginTools: "инструментов", pluginDown: "недоступен",
      loading: "Загружаю…", loadFailed: "Не удалось загрузить — попробуйте ещё раз",
      saved: "Сохранено", saveFailed: "Не сохранилось", saveRefused: "Сервер отказал",
      off: "выкл", on: "вкл",
    },
    en: {
      newChat: "New chat", scheduled: "Scheduled", plugins: "Plugins",
      pinned: "Pinned", projects: "Projects", recent: "Recent",
      emptyPinned: "No pinned chats yet", emptyProjects: "No projects yet",
      emptyRecent: "Nothing yet", hello: "What is on your mind today?",
      helloNamed: "How can I help,", reasoningOff: "Fast answers are on",
      ask: "Ask exp-1", reasoning: "Fast answers", reasoningLabel: "Reasoning",
      reasoningHint: "Switching reasoning off is what fast answers means",
      reasoningOn: "Fast answers are on", reasoningFast: "Reasoning is off",
      voice: "Voice", mic: "Microphone", nextStep: "The chat lands in the next step.",
      suggestSchedule: "Schedule a task", suggestSettings: "Tune how this page looks",
      suggestModels: "Pick the page's models",
      disclaimer: "exp-1 can make mistakes. Check important information.",
      settings: "Settings", back: "Back to the app", search: "Search",
      personalGroup: "Personal settings", general: "General",
      appearance: "Appearance", personalization: "Personalization",
      notifications: "Notifications", profile: "Profile", security: "Security and login",
      account: "Account", archived: "Archived chats", parental: "Parental controls",
      trusted: "Trusted contact", voiceSection: "Voice", storage: "Storage",
      pets: "Pets", keyboard: "Keyboard", usage: "Usage", payment: "Payment",
      data: "Data controls", integrations: "Integrations",
      langName: "Language", langDesc: "Language of the application interface",
      langAuto: "Detect automatically",
      memorySection: "Memory", memoryName: "Enable memory",
      memoryIntro: "Set how exp-1 handles memory.",
      memoryDesc: "Let exp-1 use your chats and files to answer more accurately",
      memorySummary: "Memory summary",
      memorySummaryDesc: "What exp-1 knows about you sits in the custom instructions",
      memoryFoot: "exp-1 can use memory to answer more accurately from earlier conversations.",
      manage: "Manage", styleSection: "Base style and tone", styleName: "Style and tone",
      styleDesc: "How the model talks in every chat", styleDefault: "Default",
      stylePro: "Professional", styleFriendly: "Friendly", styleCandid: "Candid",
      friendliness: "Friendliness", enthusiasm: "Enthusiasm", emoji: "Emoji",
      visualReplies: "Visual replies", headings: "Headings and lists",
      less: "Less", def: "Default", more: "More",
      customSection: "Custom instructions", customDesc: "Add instructions and context for every chat",
      customHint: "Saved automatically; it joins the system prompt of every chat.",
      modelsSection: "Models", modelsDesc: "Pick the three models with /web-models in the TUI",
      tierFast: "Fast", tierMedium: "Medium", tierPowerful: "Powerful", tierUnset: "not set",
      extraSection: "Additional settings", fast: "Fast answers",
      fastDesc: "Answer with the lowest reasoning, or none at all, when the provider allows it",
      suggested: "Suggested prompts", suggestedDesc: "Show example requests on the empty screen",
      history: "Use record history", historyDesc: "Use earlier conversations when answering",
      modeSection: "Visual style", modeName: "Mode", modeLight: "Light",
      modeDark: "Dark", modeAuto: "Match system", themeName: "Theme",
      themeDesc: "The TUI's theme sets the accent until one is picked here",
      accentName: "Accent colour", accentDefault: "Default", accentBlue: "Blue",
      accentGreen: "Green", accentYellow: "Yellow", accentPink: "Pink",
      accentOrange: "Orange", accentPurple: "Purple", accentWhite: "White",
      accentCustom: "Custom colour", bgName: "Background", fgName: "Foreground colour",
      advanced: "Advanced", uiFont: "Interface font size",
      uiFontDesc: "Base text size of the page", codeFont: "Code font size",
      codeFontDesc: "Code in chats and diffs", motionName: "Reduce motion",
      motionDesc: "Fewer animations: the system's choice, always or never",
      motionSystem: "System", motionOn: "On", motionOff: "Off",
      separateThemes: "Separate light and dark modes",
      separateThemesDesc: "A light theme comes later; the choice is kept",
      contrastName: "Contrast", diffName: "Diff markers",
      diffDesc: "Mark changes with colour or with +/− signs",
      diffColor: "Colour", diffSigns: "+/−",
      jobsEmpty: "No schedules yet — make them with /schedule in the TUI",
      jobDone: "done", jobPaused: "paused", jobEvery: "every", jobDue: "due",
      jobsHint: "Jobs are made and run from the TUI; here you can see them and pause one",
      pause: "Pause", resume: "Resume", runNow: "Run now", remove: "Remove",
      pluginsEmpty: "No plugins yet — add MCP servers to the file",
      pluginTools: "tools", pluginDown: "unavailable",
      loading: "Loading…", loadFailed: "Could not load — try again",
      saved: "Saved", saveFailed: "Not saved", saveRefused: "The server refused",
      off: "off", on: "on",
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
    grid: icon('<path d="M4 9h6V4H4zM14 9h6V4h-6zM4 20h6v-5H4zM14 20h6v-5h-6z"/>'),
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
    download: icon('<path d="M12 4v10M8 11l4 4 4-4M5 20h14"/>'),
    copy: icon('<rect x="9" y="9" width="11" height="11" rx="2"/><path d="M6 15H5a1 1 0 0 1-1-1V5a1 1 0 0 1 1-1h9a1 1 0 0 1 1 1v1"/>'),
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

  const note = (text) => el("div", { class: "hint", text });

  async function save() {
    if (saving) return;
    saving = true;
    try {
      const response = await fetch("/api/config", {
        method: "POST",
        headers: { "content-type": "application/json" },
        body: JSON.stringify(web),
      });
      if (response.ok) {
        web = Object.assign(web, (await response.json()).web || {});
        toast(t("saved"));
      } else {
        toast(`${t("saveFailed")} — HTTP ${response.status}`);
      }
    } catch (error) {
      toast(`${t("saveFailed")} — ${error.message}`);
    } finally {
      saving = false;
    }
  }

  /* ------------------------------------------------------------------ theme */

  const validHex = (value) => /^#[0-9a-f]{6}$/i.test((value || "").trim());

  function accentValue() {
    const chosen = (web.accent || "").trim();
    if (!chosen) return boot.accent || "#3b82f6";
    const named = (boot.palette || []).find((colour) => colour.name === chosen);
    if (named) return named.hex;
    return validHex(chosen) ? chosen.toLowerCase() : boot.accent || "#3b82f6";
  }

  function setColour(name, value, fallback) {
    const style = document.documentElement.style;
    if (validHex(value)) style.setProperty(name, value.trim().toLowerCase());
    else if (value) style.setProperty(name, fallback);
    else style.removeProperty(name);
  }

  function applyTheme() {
    const style = document.documentElement.style;
    const accent = accentValue();
    style.setProperty("--accent", accent);
    style.setProperty("--accent-soft", accent + "26");
    setColour("--bg", web.background, "#212121");
    setColour("--fg", web.foreground, "#ececec");
    style.setProperty("--ui-font", `${Math.min(28, Math.max(10, web.ui_font || 15))}px`);
    style.setProperty("--code-font", `${Math.min(28, Math.max(8, web.code_font || 12))}px`);
    style.setProperty("--contrast", (0.6 + Math.min(200, web.contrast ?? 60) / 100).toFixed(2));
    document.documentElement.dataset.motion =
      web.motion === "on" || web.motion === "off" ? web.motion : "system";
    document.documentElement.dataset.mode =
      web.mode === "light" || web.mode === "dark" ? web.mode : "system";
  }

  /* -------------------------------------------------------------- the parts */

  const hexToHsv = (hex) => {
    const value = (hex || "#000000").replace("#", "");
    const r = parseInt(value.slice(0, 2), 16) / 255;
    const g = parseInt(value.slice(2, 4), 16) / 255;
    const b = parseInt(value.slice(4, 6), 16) / 255;
    const max = Math.max(r, g, b);
    const min = Math.min(r, g, b);
    let hue = 0;
    if (max !== min) {
      const d = max - min;
      if (max === r) hue = ((g - b) / d + (g < b ? 6 : 0)) / 6;
      else if (max === g) hue = ((b - r) / d + 2) / 6;
      else hue = ((r - g) / d + 4) / 6;
    }
    return { h: hue, s: max === 0 ? 0 : (max - min) / max, v: max };
  };

  const hsvToHex = ({ h, s, v }) => {
    const f = (n) => {
      const k = (n + h * 6) % 6;
      const value = v - v * s * Math.max(0, Math.min(k, 4 - k, 1));
      return Math.round(value * 255)
        .toString(16)
        .padStart(2, "0");
    };
    return `#${f(5)}${f(3)}${f(1)}`;
  };

  function closeOverlays() {
    for (const node of document.querySelectorAll(".menu, .picker")) node.remove();
  }
  document.addEventListener("click", (event) => {
    if (!event.target.closest(".menu") && !event.target.closest(".picker") && !event.target.closest(".select")) {
      closeOverlays();
    }
  });

  const place = (node, anchor) => {
    document.body.append(node);
    const box = anchor.getBoundingClientRect();
    node.style.left = `${Math.max(12, Math.min(box.left, window.innerWidth - node.offsetWidth - 12))}px`;
    node.style.top = `${Math.min(box.bottom + 6, window.innerHeight - node.offsetHeight - 12)}px`;
    return node;
  };

  const FALLBACK = { background: "#212121", foreground: "#ececec" };

  function openColourPicker(anchor, key) {
    closeOverlays();
    const start = validHex(web[key])
      ? web[key]
      : key === "accent"
        ? accentValue()
        : FALLBACK[key] || "#ececec";
    let hsv = hexToHsv(start);
    const field = el("div", { class: "field" });
    const knob = el("span", { class: "knob" });
    field.append(knob);
    const hue = el("input", { type: "range", class: "hue", min: "0", max: "360", value: String(Math.round(hsv.h * 360)) });
    const picker = el("div", { class: "picker" }, [field, hue]);

    const paint = (commit) => {
      field.style.backgroundImage =
        `linear-gradient(to top, #000, transparent), linear-gradient(to right, #fff, hsl(${Math.round(hsv.h * 360)}, 100%, 50%))`;
      knob.style.left = `${hsv.s * 100}%`;
      knob.style.top = `${(1 - hsv.v) * 100}%`;
      web[key] = hsvToHex(hsv);
      applyTheme();
      const shown = web[key].toUpperCase();
      const field_ = anchor.matches("input.hex-value") ? anchor : anchor.querySelector(".hex-value");
      if (field_) field_.value = shown;
      if (commit) save();
    };

    field.addEventListener("pointerdown", (event) => {
      field.setPointerCapture(event.pointerId);
      const move = (moveEvent) => {
        const box = field.getBoundingClientRect();
        hsv.s = Math.min(1, Math.max(0, (moveEvent.clientX - box.left) / box.width));
        hsv.v = Math.min(1, Math.max(0, 1 - (moveEvent.clientY - box.top) / box.height));
        paint(false);
      };
      move(event);
      const up = () => {
        field.removeEventListener("pointermove", move);
        field.removeEventListener("pointerup", up);
        paint(true);
      };
      field.addEventListener("pointermove", move);
      field.addEventListener("pointerup", up);
    });
    hue.addEventListener("input", () => {
      hsv.h = Number(hue.value) / 360;
      paint(false);
    });
    hue.addEventListener("change", () => paint(true));

    place(picker, anchor);
    paint(false);
  }

  const MENUS = () => ({
    language: [
      { value: "", label: t("langAuto") },
      { value: "ru", label: "Русский" },
      { value: "en", label: "English" },
    ],
    style: [
      { value: "default", label: t("styleDefault") },
      { value: "professional", label: t("stylePro") },
      { value: "friendly", label: t("styleFriendly") },
      { value: "candid", label: t("styleCandid") },
    ],
    visual_replies: [
      { value: "default", label: t("def") },
      { value: "less", label: t("less") },
    ],
  });

  const trait = (key) => [
    { value: "less", label: t("less") },
    { value: "default", label: t("def") },
    { value: "more", label: t("more") },
  ];

  const select = (key, options, after, swatchFor) => {
    const node = el("button", { class: "select" });
    const current = () => (options().find((option) => option.value === (web[key] ?? options()[0].value)) || options()[0]);
    const label = () => {
      const choice = current();
      const children = [];
      if (swatchFor && swatchFor(choice)) children.push(el("span", { class: "colour-dot", style: `background:${swatchFor(choice)}` }));
      children.push(el("span", { text: choice.label }));
      children.push(el("span", { html: icons.chevron }));
      node.textContent = "";
      node.append(...children);
    };
    label();
    node.addEventListener("click", (event) => {
      event.stopPropagation();
      closeOverlays();
      const menu = el("div", { class: "menu" });
      for (const option of options()) {
        menu.append(
          el("button", {
            class: option.value === (web[key] ?? options()[0].value) ? "on" : "",
            onclick: () => {
              if (option.action) {
                closeOverlays();
                option.action(node);
                return;
              }
              web[key] = option.value;
              closeOverlays();
              label();
              applyTheme();
              save();
              if (after) after();
            },
          }, [
            swatchFor && swatchFor(option) ? el("span", { class: "colour-dot", style: `background:${swatchFor(option)}` }) : null,
            el("span", { text: option.label }),
          ]),
        );
      }
      place(menu, node);
    });
    return node;
  };

  const toggle = (key, after) => {
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
      if (after) after();
    });
    return node;
  };

  const rows = (list) => el("div", { class: "card" }, list);
  const row = (name, desc, control) =>
    el("div", { class: "row" }, [
      el("div", { class: "text" }, [
        el("div", { class: "name", text: name }),
        desc ? el("div", { class: "desc", text: desc }) : null,
      ]),
      control ? el("div", { class: "control" }, [control]) : el("span", { class: "spacer", text: "›" }),
    ]);
  const section = (title) => el("div", { class: "set-section", text: title });

  const segmented = (key, options, after) => {
    const node = el("div", { class: "segment small" });
    for (const option of options) {
      node.append(
        el("button", {
          class: (web[key] ?? options[0].value) === option.value ? "on" : "",
          text: option.label,
          onclick: (event) => {
            web[key] = option.value;
            for (const sibling of event.currentTarget.parentElement.children) sibling.classList.remove("on");
            event.currentTarget.classList.add("on");
            save();
            if (after) after();
          },
        }),
      );
    }
    return node;
  };

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
      list.append(el("button", { class: "side-item", text: session.title, onclick: nextStep }));
    }
    if (!pinned.length) list.append(el("div", { class: "side-label", text: t("pinned") }), el("div", { class: "side-empty", text: t("emptyPinned") }));
    list.append(el("div", { class: "side-label", text: t("projects") }));
    list.append(el("div", { class: "side-empty", text: t("emptyProjects") }));
    list.append(el("div", { class: "side-label", text: t("recent") }));
    if (!sessions.length) list.append(el("div", { class: "side-empty", text: t("emptyRecent") }));

    return [
      el("div", { class: "side-top" }, [
        el("div", { class: "brand" }, [el("span", { class: "brand-name", text: "exp-1" })]),
        el("div", { class: "side-icons" }, [
          el("button", { class: "icon-btn", html: icons.search, title: t("search"), onclick: nextStep }),
          el("button", { class: "icon-btn", html: icons.panel, title: t("settings"), onclick: () => go("/settings/general") }),
        ]),
      ]),
      el("button", { class: "side-row primary" + (route() === "/" ? " active" : ""), onclick: () => go("/") }, [
        el("span", { html: icons.pencil }),
        el("span", { text: t("newChat") }),
      ]),
      sideRow(icons.clock, t("scheduled"), "/scheduled"),
      sideRow(icons.puzzle, t("plugins"), "/plugins"),
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

  const nextStep = () => toast(t("nextStep"));

  /* The reference greets by name; the name is the one you are logged in as. */
  const greeting = () => (boot.user ? `${t("helloNamed")} ${boot.user}?` : t("hello"));

  function chatView() {
    const input = el("input", {
      placeholder: t("ask"),
      onkeydown: (event) => {
        if (event.key === "Enter") nextStep();
      },
    });
    /* The pill is the reasoning switch, the way the reference has it: plain while
       reasoning is on, filled when fast answers (no reasoning) is what is set. */
    const fast = el("button", { class: "pill" + (web.fast_answers ? " on" : ""), title: t("reasoningHint") }, [
      el("span", { html: icons.sparkle }),
      el("span", { text: t("reasoningLabel") }),
    ]);
    fast.addEventListener("click", () => {
      web.fast_answers = !web.fast_answers;
      fast.classList.toggle("on", !!web.fast_answers);
      toast(web.fast_answers ? t("reasoningFast") : t("reasoningOn"));
      save();
    });

    const view = el("div", { class: "chat-view" }, [
      el("div", { class: "chat-hello", text: greeting() }),
      el("div", { class: "composer" }, [
        el("button", { class: "plus", html: icons.plus, onclick: nextStep }),
        input,
        el("div", { class: "tools" }, [
          fast,
          el("button", { class: "mic", html: icons.mic, title: t("mic"), onclick: nextStep }),
          el("button", { class: "voice", html: icons.wave, title: t("voice"), onclick: nextStep }),
        ]),
      ]),
    ]);
    if (web.suggested_prompts) {
      const box = el("div", { class: "chat-suggestions" });
      const suggestions = [
        { glyph: icons.clock, text: t("suggestSchedule"), at: "/scheduled" },
        { glyph: icons.sliders, text: t("suggestSettings"), at: "/settings/appearance" },
        { glyph: icons.sparkle, text: t("suggestModels"), at: "/settings/personalization" },
      ];
      for (const item of suggestions) {
        box.append(
          el("button", { class: "suggestion", onclick: () => go(item.at) }, [
            el("span", { html: item.glyph }),
            el("span", { text: item.text }),
          ]),
        );
      }
      view.append(box);
    }
    return view;
  }

  /* --------------------------------------------------------------- settings */

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
          el("button", {
            class: "set-link" + (item.path === active ? " active" : ""),
            onclick: item.path ? () => go(item.path) : nextStep,
          }, [el("span", { html: item.icon }), el("span", { text: label })]),
        );
      }
    };
    filter.addEventListener("input", paint);
    paint();
    return el("div", { class: "set-nav" }, [
      el("div", { class: "set-top" }, [
        el("button", { class: "icon-btn", html: icons.panel, onclick: () => go("/") }),
      ]),
      el("button", { class: "set-back", onclick: () => go("/") }, [
        el("span", { html: icons.back }),
        el("span", { text: t("back") }),
      ]),
      el("div", { class: "set-search" }, [el("span", { html: icons.search }), filter]),
      el("div", { class: "set-group", text: t("personalGroup") }),
      list,
    ]);
  }

  function generalScreen(body) {
    body.append(
      el("div", { class: "set-page" }, [
        el("h1", { class: "set-title", text: t("general") }),
        rows([
          row(t("langName"), t("langDesc"), select("language", () => MENUS().language, render)),
        ]),
      ]),
    );
  }

  function appearanceScreen(body) {
    const paletteName = (value) => `accent${value.charAt(0).toUpperCase()}${value.slice(1)}`;
    const custom = validHex(web.accent) ? web.accent.toLowerCase() : "";
    const accentOptions = () => {
      const items = [{ value: "", label: t("accentDefault") }];
      for (const colour of boot.palette || []) {
        items.push({ value: colour.name, label: t(paletteName(colour.name)) });
      }
      items.push({
        value: custom || "__custom",
        label: custom ? custom.toUpperCase() : t("accentCustom"),
        action: (anchor) => openColourPicker(anchor, "accent"),
      });
      return items;
    };
    const accentSwatch = (option) => {
      if (option.value === "") return "#ffffff";
      if (option.value === "__custom") return undefined;
      if (/^#[0-9a-f]{6}$/i.test(option.value)) return option.value;
      const named = (boot.palette || []).find((colour) => colour.name === option.value);
      return named ? named.hex : accentValue();
    };

    const modes = [
      { value: "system", label: t("modeAuto"), cls: "mode auto" },
      { value: "light", label: t("modeLight"), cls: "mode light" },
      { value: "dark", label: t("modeDark"), cls: "mode dark" },
    ].map((mode) => {
      const tile = el("button", {
        class: mode.cls + ((web.mode || "system") === mode.value ? " on" : ""),
        title: mode.label,
        onclick: () => {
          web.mode = mode.value;
          applyTheme();
          for (const sibling of tile.parentElement.children) sibling.classList.remove("on");
          tile.classList.add("on");
          save();
        },
      }, [
        el("span", { class: "mode-bar" }),
        el("span", { class: "mode-line" }),
        el("span", { class: "mode-line short" }),
        el("span", { class: "mode-dot" }),
      ]);
      return tile;
    });

    const colourRow = (key, name) => {
      const fallback = key === "background" ? "#212121" : "#ececec";
      const dot = el("span", { class: "colour-dot" });
      const input = el("input", { class: "hex-value", spellcheck: "false" });
      const pill = el("span", { class: "select static editable" }, [dot, input]);
      const sync = () => {
        const colour = validHex(web[key]) ? web[key].trim().toLowerCase() : fallback;
        dot.style.background = colour;
        input.value = colour.toUpperCase();
      };
      sync();
      input.addEventListener("focus", () => openColourPicker(pill, key));
      input.addEventListener("keydown", (event) => {
        if (event.key !== "Enter") return;
        if (!validHex(input.value)) {
          sync();
          return;
        }
        web[key] = input.value.trim().toLowerCase();
        sync();
        applyTheme();
        save();
      });
      pill.addEventListener("pointerdown", (event) => {
        if (event.target === input) return;
        event.preventDefault();
        openColourPicker(pill, key);
      });
      return row(name, null, pill);
    };

    const advanced = el("div");
    const advancedHead = el("div", { class: "advanced-head" + (advancedOpen ? " open" : "") }, [
      el("span", { text: t("advanced") }),
      el("span", { class: "chevron", html: icons.chevron }),
    ]);
    const paintAdvanced = () => {
      advanced.textContent = "";
      advancedHead.classList.toggle("open", advancedOpen);
      if (!advancedOpen) return;
      const number = (key) => {
        const field = el("input", { type: "number", class: "number", min: "8", max: "28", value: String(web[key] || "") });
        field.addEventListener("change", () => {
          web[key] = Number(field.value);
          applyTheme();
          save();
        });
        return el("span", {}, [field, el("span", { class: "unit", text: "px" })]);
      };
      const slider = el("input", { type: "range", class: "slider", min: "0", max: "140", value: String(web.contrast ?? 60) });
      const contrastValue = el("span", { class: "unit", text: String(web.contrast ?? 60) });
      slider.addEventListener("input", () => {
        web.contrast = Number(slider.value);
        contrastValue.textContent = slider.value;
        applyTheme();
      });
      slider.addEventListener("change", () => save());
      advanced.append(
        rows([
          row(t("uiFont"), t("uiFontDesc"), number("ui_font")),
          row(t("codeFont"), t("codeFontDesc"), number("code_font")),
        ]),
        rows([
          row(t("motionName"), t("motionDesc"), segmented("motion", [
            { value: "system", label: t("motionSystem") },
            { value: "on", label: t("motionOn") },
            { value: "off", label: t("motionOff") },
          ], applyTheme)),
          row(t("separateThemes"), t("separateThemesDesc"), toggle("separate_themes")),
        ]),
        rows([
          row(t("contrastName"), null, el("span", {}, [slider, contrastValue])),
          row(t("diffName"), t("diffDesc"), segmented("diff_markers", [
            { value: "color", label: t("diffColor") },
            { value: "signs", label: t("diffSigns") },
          ])),
        ]),
      );
    };
    advancedHead.addEventListener("click", () => {
      advancedOpen = !advancedOpen;
      paintAdvanced();
    });
    paintAdvanced();

    body.append(
      el("div", { class: "set-page" }, [
        el("h1", { class: "set-title", text: t("appearance") }),
        section(t("modeSection")),
        rows([
          el("div", { class: "row" }, [
            el("div", { class: "text" }, [el("div", { class: "name", text: t("modeName") })]),
            el("div", { class: "control modes" }, modes),
          ]),
        ]),
        el("div", { class: "set-section spacer-line" }),
        rows([
          el("div", { class: "row" }, [
            el("div", { class: "text" }, [el("div", { class: "name", text: t("themeName") })]),
            el("div", { class: "control theme-control" }, [
              el("button", { class: "icon-btn", html: icons.download, title: t("themeName"), onclick: nextStep }),
              el("button", { class: "icon-btn", html: icons.copy, title: t("themeName"), onclick: nextStep }),
              el("span", { class: "select static" }, [
                el("span", { class: "aa", text: "Aa" }),
                el("span", { text: boot.theme || "default" }),
              ]),
            ]),
          ]),
          row(t("accentName"), null, select("accent", accentOptions, null, accentSwatch)),
          colourRow("background", t("bgName")),
          colourRow("foreground", t("fgName")),
        ]),
        advancedHead,
        advanced,
      ]),
    );
  }

  function personalizationScreen(body) {
    const textarea = el("textarea", { placeholder: t("customDesc") });
    textarea.value = web.custom_instructions || "";
    textarea.addEventListener("blur", () => {
      web.custom_instructions = textarea.value;
      save();
    });
    const focus = () => {
      textarea.focus();
      textarea.scrollIntoView({ block: "center", behavior: "smooth" });
    };

    const tiers = web.tiers || {};
    const tierRows = [
      ["fast", t("tierFast")],
      ["medium", t("tierMedium")],
      ["powerful", t("tierPowerful")],
    ].map(([slot, label]) => row(label, tiers[slot] || t("tierUnset"), null));

    body.append(
      el("div", { class: "set-page" }, [
        el("h1", { class: "set-title", text: t("personalization") }),
        section(t("memorySection")),
        note(t("memoryIntro")),
        rows([
          row(t("memoryName"), t("memoryDesc"), toggle("memory")),
          el("div", { class: "row link", onclick: focus }, [
            el("div", { class: "text" }, [
              el("div", { class: "name", text: t("memorySummary") }),
              el("div", { class: "desc", text: t("memorySummaryDesc") }),
            ]),
            el("div", { class: "control" }, [el("span", { class: "button", text: t("manage") })]),
          ]),
          el("div", { class: "row footnote" }, [
            el("div", { class: "text" }, [el("div", { class: "desc", text: t("memoryFoot") })]),
          ]),
        ]),
        section(t("styleSection")),
        rows([
          row(t("styleName"), t("styleDesc"), select("style", () => MENUS().style)),
          row(t("friendliness"), null, select("friendliness", () => trait())),
          row(t("enthusiasm"), null, select("enthusiasm", () => trait())),
          row(t("emoji"), null, select("emoji", () => trait())),
          row(t("visualReplies"), null, select("visual_replies", () => MENUS().visual_replies)),
          row(t("headings"), null, select("headings", () => trait())),
        ]),
        section(t("customSection")),
        rows([el("div", { class: "instructions" }, [textarea, note(t("customHint"))])]),
        section(t("modelsSection")),
        rows(tierRows),
        note(t("modelsDesc")),
        section(t("extraSection")),
        rows([
          row(t("fast"), t("fastDesc"), toggle("fast_answers")),
          row(t("suggested"), t("suggestedDesc"), toggle("suggested_prompts", render)),
          row(t("history"), t("historyDesc"), toggle("record_history")),
        ]),
      ]),
    );
  }

  /* ------------------------------------------------------ scheduled, plugins */

  async function json(url, options) {
    const response = await fetch(url, options);
    if (!response.ok) throw new Error(`HTTP ${response.status}`);
    return response.json();
  }

  function scheduledScreen(body) {
    const page = el("div", { class: "set-page" }, [
      el("h1", { class: "set-title", text: t("scheduled") }),
      note(t("loading")),
    ]);
    body.append(page);
    const paint = async () => {
      page.textContent = "";
      page.append(el("h1", { class: "set-title", text: t("scheduled") }));
      let jobs = [];
      try {
        jobs = await json("/api/schedules");
      } catch {
        page.append(note(t("loadFailed")));
        return;
      }
      if (!jobs.length) {
        page.append(note(`${t("jobsEmpty")}`));
        return;
      }
      const list = el("div", { class: "card" });
      for (const job of jobs) {
        const state = job.done
          ? t("jobDone")
          : job.paused
            ? t("jobPaused")
            : job.every
              ? `${t("jobEvery")} ${job.every}`
              : `${t("jobDue")} ${job.when}`;
        const actions = el("div", { class: "acts" });
        const act = async (action, method) => {
          try {
            await json(`/api/schedules/${encodeURIComponent(job.id)}${action ? "/" + action : ""}`, { method: method || "POST" });
            paint();
          } catch (error) {
            toast(`${t("loadFailed")} — ${error.message}`);
          }
        };
        if (!job.done) {
          actions.append(el("button", { text: job.paused ? t("resume") : t("pause"), onclick: () => act(job.paused ? "resume" : "pause") }));
          actions.append(el("button", { text: t("runNow"), onclick: () => act("run") }));
        }
        actions.append(el("button", { class: "danger", text: t("remove"), onclick: () => act("", "DELETE") }));
        list.append(
          el("div", { class: "job" }, [
            el("div", { class: "text" }, [
              el("div", { class: "name", text: job.name }),
              el("div", { class: "desc", text: `${state}${job.last ? " · " + job.last : ""}` }),
            ]),
            actions,
          ]),
        );
      }
      page.append(list, note(t("jobsHint")));
    };
    paint();
  }

  function pluginsScreen(body) {
    const page = el("div", { class: "set-page" }, [
      el("h1", { class: "set-title", text: t("plugins") }),
      note(t("loading")),
    ]);
    body.append(page);
    json("/api/plugins")
      .then((servers) => {
        page.textContent = "";
        page.append(el("h1", { class: "set-title", text: t("plugins") }));
        if (!servers.length) {
          page.append(note(`${t("pluginsEmpty")} ${boot.mcp_path || ""}`));
          return;
        }
        const list = el("div", { class: "card" });
        for (const server of servers) {
          const tools = server.tools ?? 0;
          const desc = server.error
            ? `${t("pluginDown")}: ${server.error}`
            : `${tools} ${t("pluginTools")}`;
          list.append(
            el("div", { class: "job" }, [
              el("div", { class: "text" }, [
                el("div", { class: "name", text: server.name || "?" }),
                el("div", { class: "desc", text: `${server.kind || "?"} · ${desc}` }),
              ]),
            ]),
          );
        }
        page.append(list, note(`${boot.mcp_path || ""}`));
      })
      .catch(() => {
        page.textContent = "";
        page.append(el("h1", { class: "set-title", text: t("plugins") }), note(t("loadFailed")));
      });
  }

  /* ------------------------------------------------------------------ paint */

  const SCREENS = {
    "/settings/general": generalScreen,
    "/settings/appearance": appearanceScreen,
    "/settings/personalization": personalizationScreen,
    "/scheduled": scheduledScreen,
    "/plugins": pluginsScreen,
  };

  const route = () => location.hash.replace(/^#/, "") || "/";
  const go = (path) => {
    if (route() === path) render();
    else location.hash = "#" + path;
  };
  window.addEventListener("hashchange", render);

  function render() {
    const current = SCREENS[route()] ? route() : "/";
    document.documentElement.lang = language();
    applyTheme();
    const sidebar = document.getElementById("sidebar");
    const main = document.getElementById("main");
    const withNav = current.startsWith("/settings/");
    sidebar.textContent = "";
    if (withNav) sidebar.hidden = true;
    else {
      sidebar.hidden = false;
      sidebar.append(...sidebarNodes());
    }
    main.textContent = "";
    if (current === "/") {
      main.append(chatView());
      return;
    }
    const body = el("div", { class: "set-body" });
    main.append(el("div", { class: "settings" }, [withNav ? settingsNav(current) : el("div", { class: "set-nav" }, [el("button", { class: "set-back", onclick: () => go("/") }, [el("span", { html: icons.back }), el("span", { text: t("back") })])]), body]));
    SCREENS[current](body);
  }

  /* A page left open picks up a theme or colour changed elsewhere. */
  window.addEventListener("focus", async () => {
    try {
      const config = await json("/api/config");
      web = Object.assign(web, config.web || {});
      boot.accent = config.accent || boot.accent;
      applyTheme();
    } catch {
      /* a closed server is not worth a message here */
    }
  });

  render();
})();
