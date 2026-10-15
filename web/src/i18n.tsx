import { createContext, useContext, useEffect, useState, type ReactNode } from "react";

export type Lang = "en" | "ru";
export type OS = "linux" | "mac" | "windows";

function readLang(): Lang {
  try {
    const v = localStorage.getItem("exp1-lang");
    if (v === "ru" || v === "en") return v;
  } catch { /* ignore */ }
  return "en";
}

export const INSTALL_CMDS: Record<OS, string[]> = {
  linux: [
    "git clone https://github.com/archpulse/exp-1.git exp-1",
    "cd exp-1",
    "cargo build --release",
    "cargo install --path .",
  ],
  mac: [
    "git clone -b mac https://github.com/archpulse/exp-1.git exp-1",
    "cd exp-1",
    "cargo build --release",
    "cargo install --path .",
  ],
  windows: [
    "git clone -b windows https://github.com/archpulse/exp-1.git exp-1",
    "cd exp-1",
    "cargo build --release",
    "cargo install --path .",
  ],
};

export const STR: Record<Lang, Record<string, string>> = {
  en: {
    "nav.start": "setup", "nav.feel": "feel", "nav.phil": "philosophy", "nav.details": "details",
    "accent.label": "accent", "accent.group": "Accent color", "lang.name": "language",
    "accent.def": "frost (/themes default)", "accent.amber": "amber (/themes amber)",
    "accent.jade": "jade (/themes jade)", "accent.mono": "mono (/themes mono)",
    "hero.eyebrow": "a companion living in the terminal",
    "hero.h1a": "Silence on the air.", "hero.h1b": "Speed in your fingers.",
    "hero.sub": "A minimal Rust TUI coding agent: 27 providers, two conversation threads. Build from source — pick your OS below. Just start typing.",
    "hero.term": "~/night", "hero.aria": "Try typing",
    "hero.idle": "click the window and start typing", "hero.active": "type · enter to send",
    "hero.r0": "Hear you. But this is only the showcase — the real me lives in your terminal: clone the repo and build.",
    "hero.r1": "Good thought. We will write it together — after /login and picking a model.",
    "hero.r2": "It starts from source. The commands are just below.",
    "hero.r3": "Quiet, is not it? That is the point: tool output hides behind ctrl+o.",
    "hero.disclaimer": "No published releases yet: the binary builds from source, providers sign in via /login. Demo numbers on this page illustrate — they do not benchmark.",
    "hero.copy": "copy", "hero.copied": "copied ✓", "hero.more": "↓ more",
    "hero.install": "install — pick your OS",
    "hero.note": "main is Linux. macOS lives on the mac branch, Windows on the windows branch.",
    "hero.statProv": "providers", "hero.statBtw": "second thread", "hero.statSil": "silence",
    "start.eyebrow": "setup", "start.h2": "Four commands to work.",
    "start.intro": "No installers: clone, switch to your OS branch, build, sign in. You need Rust 1.85+ — the agent does the rest.",
    "start.s0t": "clone", "start.s0x": "One repo, three branches: main for Linux, mac for macOS, windows for Windows.",
    "start.s1t": "switch + build", "start.s1x": "Checkout your OS branch, then cargo build --release and cargo install --path .",
    "start.s2t": "sign in", "start.s2x": "A provider picker: Codex OAuth via auth.openai.com, an API key, a device flow, or whatever the machine already holds.",
    "start.s3t": "pick", "start.s3x": "A two-step picker: model first, then effort (low/medium/high). Live catalogue from models.dev, offline table in the binary.",
    "start.copy": "click to copy", "start.copied": "copied ✓",
    "start.keys": "keys — you will learn them in an evening",
    "foot.note": "dictation — Linux · v0.1.0 · Rust 1.85+",
    "foot.readme": "readme", "foot.install": "install",
    "foot.bottom": "shot at night · not a single extra window", "foot.top": "↑ back to top",
    "foot.docs": "Documentation",
  },
  ru: {
    "nav.start": "запуск", "nav.feel": "ощущения", "nav.phil": "философия", "nav.details": "детали",
    "accent.label": "акцент", "accent.group": "Акцентный цвет", "lang.name": "язык",
    "accent.def": "холодный (/themes default)", "accent.amber": "янтарь (/themes amber)",
    "accent.jade": "нефрит (/themes jade)", "accent.mono": "моно (/themes mono)",
    "hero.eyebrow": "напарник, живущий в терминале",
    "hero.h1a": "Тишина в эфире.", "hero.h1b": "Скорость в пальцах.",
    "hero.sub": "Минималистичный агент на Rust: 27 провайдеров, два потока. Сборка из исходников — выбери свою ОС ниже.",
    "hero.term": "~/ночь", "hero.aria": "Попробуй напечатать",
    "hero.idle": "нажми на окно и начни печатать", "hero.active": "печатай · enter — отправить",
    "hero.r0": "Слышу. Но здесь только витрина — настоящий я в твоём терминале.",
    "hero.r1": "Хорошая мысль. Допишем вместе — после /login.",
    "hero.r2": "Запускается из исходников. Команды чуть ниже.",
    "hero.r3": "Тихо, правда? Так и должно быть: вывод за ctrl+o.",
    "hero.disclaimer": "Релизов пока нет: бинарник собирается из исходников, вход через /login.",
    "hero.copy": "копировать", "hero.copied": "в буфере ✓", "hero.more": "↓ дальше",
    "hero.install": "установка — выбери свою ОС",
    "hero.note": "main — это Linux. macOS живёт на ветке mac, Windows — на ветке windows.",
    "start.eyebrow": "запуск", "start.h2": "Четыре команды до работы.",
    "start.intro": "Никаких установщиков: клонируешь, ветка своей ОС, сборка, вход. Нужен Rust 1.85+.",
    "start.s0t": "склонируй", "start.s0x": "Один репозиторий, три ветки: main — Linux, mac — macOS, windows — Windows.",
    "start.s1t": "ветка + сборка", "start.s1x": "Переключи ветку своей ОС, далее cargo build --release и cargo install --path .",
    "start.s2t": "войди", "start.s2x": "Пикер провайдеров: Codex OAuth, API-ключ, device flow.",
    "start.s3t": "выбери", "start.s3x": "Двухшаговый пикер: модель, затем effort. Каталог — живой из models.dev.",
    "start.copy": "клик — копировать", "start.copied": "в буфере ✓",
    "start.keys": "клавиши — выучишь за вечер",
    "foot.note": "диктовка — Linux · v0.1.0 · Rust 1.85+",
    "foot.readme": "readme", "foot.install": "установка",
    "foot.bottom": "снято ночью · без единого лишнего окна", "foot.top": "↑ к началу",
    "hero.statProv": "провайдеров", "hero.statBtw": "второй поток", "hero.statSil": "тишина",
    "foot.docs": "Документация",
  },
};

const Ctx = createContext<{ lang: Lang; setLang: (l: Lang) => void; s: (k: string) => string }>({
  lang: "en", setLang: () => {}, s: (k) => k,
});

export function LangProvider({ children }: { children: ReactNode }) {
  const [lang, setLangState] = useState<Lang>(readLang);
  const setLang = (l: Lang) => {
    setLangState(l);
    try { localStorage.setItem("exp1-lang", l); } catch { /* ignore */ }
  };
  useEffect(() => { document.documentElement.lang = lang; }, [lang]);
  const s = (k: string) => STR[lang][k] ?? STR.en[k] ?? k;
  return <Ctx.Provider value={{ lang, setLang, s }}>{children}</Ctx.Provider>;
}

export function useLang() { return useContext(Ctx); }
