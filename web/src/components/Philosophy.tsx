import { useEffect, useRef, useState, type ReactNode } from "react";
import { useLang } from "../i18n";
import { Eyebrow, Reveal, useInView } from "./shared";

/* ---------- card shell ---------- */
function Card({
  index,
  title,
  text,
  demo,
  delay,
}: {
  index: string;
  title: ReactNode;
  text: ReactNode;
  demo: ReactNode;
  delay: number;
}) {
  return (
    <Reveal delay={delay} className="h-full">
      <article className="group flex h-full flex-col rounded-2xl border hairline bg-ink-2/50 p-3 transition-colors duration-700 hover:border-milk/15">
        <div className="relative h-60 overflow-hidden rounded-xl border hairline bg-ink">{demo}</div>
        <div className="flex flex-1 flex-col px-4 pb-5 pt-8 md:px-6">
          <div className="font-mono text-[11px] tracking-[0.25em] text-accent">{index}</div>
          <h3 className="mt-4 font-serif text-4xl font-light leading-none tracking-tight text-milk md:text-5xl">{title}</h3>
          <p className="mt-5 max-w-md font-mono text-[13px] leading-[1.9] text-dim">{text}</p>
        </div>
      </article>
    </Reveal>
  );
}

/* ---------- 1. /btw ---------- */
const SPIN = ["⠋", "⠙", "⠹", "⠸", "⠼", "⠴", "⠦", "⠧", "⠇", "⠏"];
const BTW_Q_EN = "/btw how to reverse a string in bash?";
const BTW_Q_RU = "/btw как развернуть строку в bash?";
const BTW_A = 'rev <<< "$s"';

function BtwDemo({ active }: { active: boolean }) {
  const { lang } = useLang();
  const ru = lang === "ru";
  const BTW_Q = ru ? BTW_Q_RU : BTW_Q_EN;
  const [frame, setFrame] = useState(0);
  const [typed, setTyped] = useState(0);
  const [showA, setShowA] = useState(false);
  const [files, setFiles] = useState(3);

  useEffect(() => {
    if (!active) return;
    const s = setInterval(() => setFrame((f) => f + 1), 90);
    const f = setInterval(() => setFiles((n) => (n >= 12 ? 3 : n + 1)), 900);
    return () => {
      clearInterval(s);
      clearInterval(f);
    };
  }, [active]);

  useEffect(() => {
    if (!active) return;
    let cancelled = false;
    const timers: number[] = [];
    const cycle = () => {
      if (cancelled) return;
      setTyped(0);
      setShowA(false);
      for (let i = 1; i <= BTW_Q.length; i++) {
        timers.push(window.setTimeout(() => setTyped(i), 600 + i * 45));
      }
      const end = 600 + BTW_Q.length * 45;
      timers.push(window.setTimeout(() => setShowA(true), end + 450));
      timers.push(window.setTimeout(cycle, end + 4200));
    };
    cycle();
    return () => {
      cancelled = true;
      timers.forEach(clearTimeout);
    };
  }, [active]);

  return (
    <div className="flex h-full flex-col p-5 font-mono text-[11.5px]">
      <div className="flex items-center gap-4 text-[10px] uppercase tracking-[0.2em]">
        <span className="text-milk/70">{ru ? "основная" : "main"}</span>
        <span className="text-faint">/btw</span>
      </div>
      <div className="mt-3 rounded-lg border hairline bg-ink-2 p-3">
        <div className="flex items-center gap-2 text-dim">
          <span className="text-accent">{SPIN[frame % SPIN.length]}</span>
          {ru ? "рефакторинг auth-модуля" : "refactoring the auth module"}
        </div>
        <div className="mt-2 flex items-center gap-3 text-faint">
          <div className="h-px flex-1 bg-milk/10">
            <div className="h-px bg-accent/70 transition-all duration-700" style={{ width: `${(files / 12) * 100}%` }} />
          </div>
          <span className="tabular-nums">{String(files).padStart(2, "0")}/12</span>
        </div>
      </div>
      <div className="mt-3 flex-1 rounded-lg border border-accent/20 bg-ink-2 p-3">
        <div className="text-milk/90">
          <span className="mr-2 text-accent">›</span>
          {BTW_Q.slice(0, typed)}
          {!showA && <span className="caret ml-0.5" style={{ height: "1em" }} />}
        </div>
        {showA && (
          <div className="fade-up mt-2 text-dim">
            <span className="mr-2 text-faint">◦</span>
            <span className="text-milk">{BTW_A}</span>
            <span className="ml-3 text-faint">{ru ? "— главная задача не прерывалась" : "— the main job never paused"}</span>
          </div>
        )}
      </div>
    </div>
  );
}

/* ---------- 2. providers ---------- */
const PROVIDERS = [
  ["openai-auth", "codex"],
  ["anthropic", "messages"],
  ["gemini", "generateContent"],
  ["deepseek", "chat"],
  ["openrouter", "any"],
  ["groq", "llama"],
  ["grok", "xai"],
  ["opencode-go", "grok-4.7"],
  ["ollama*", "local"],
];

function ProvidersDemo({ active }: { active: boolean }) {
  const { lang } = useLang();
  const ru = lang === "ru";
  const [i, setI] = useState(0);
  useEffect(() => {
    if (!active) return;
    const t = setInterval(() => setI((n) => n + 1), 1700);
    return () => clearInterval(t);
  }, [active]);
  const [p, m] = PROVIDERS[i % PROVIDERS.length];
  const num = ((i * 4) % 27) + 1;

  return (
    <button
      onClick={() => setI((n) => n + 1)}
      className="flex h-full w-full flex-col justify-between p-5 text-left font-mono"
      aria-label={ru ? "Сменить модель" : "Next model"}
    >
      <div className="flex items-center justify-between text-[10px] uppercase tracking-[0.2em] text-faint">
        <span>{ru ? "модель" : "model"}</span>
        <span className="tabular-nums">
          <span className="text-milk/80">{String(num).padStart(2, "0")}</span> / 27
        </span>
      </div>
      <div className="overflow-hidden">
        <div key={i} className="tick-in">
          <div className="text-[11px] uppercase tracking-[0.25em] text-dim">{p}</div>
          <div className="mt-1 font-serif text-5xl font-light italic text-accent">{m}</div>
        </div>
      </div>
      <div className="flex items-center gap-2 text-[11px] text-faint">
        <span key={`a${i}`} className="kbd" style={{ animation: "keypress 1.7s ease-in-out" }}>/model</span>
        <span className="ml-auto">{ru ? "разговор продолжается" : "conversation goes on"}</span>
      </div>
      <div className="mt-1 text-[10px] leading-relaxed text-faint">{ru ? "/login — вход · * локальные бэкенды зависят от окружения" : "/login signs in · * local backends depend on the environment"}</div>
    </button>
  );
}

/* ---------- 3. revert ---------- */
const MESSAGES_EN = [
  { who: "you", text: "Add caching for API requests" },
  { who: "exp", text: "Done. TTL 60 seconds." },
  { who: "you", text: "Now rewrite everything in GraphQL" },
  { who: "exp", text: "Rewrote 34 files. Tests are failing." },
  { who: "you", text: "…why did I ask for that" },
];
const MESSAGES_RU = [
  { who: "ты", text: "Добавь кэш для запросов к API" },
  { who: "exp", text: "Готово. TTL 60 секунд." },
  { who: "ты", text: "А теперь перепиши всё на GraphQL" },
  { who: "exp", text: "Переписал 34 файла. Тесты падают." },
  { who: "ты", text: "…зачем я это попросил" },
];

function RevertDemo() {
  const { lang } = useLang();
  const ru = lang === "ru";
  const MESSAGES = ru ? MESSAGES_RU : MESSAGES_EN;
  const [cut, setCut] = useState<number | null>(null);
  const [hover, setHover] = useState<number | null>(null);

  return (
    <div className="flex h-full flex-col p-4 font-mono text-[11.5px]">
      <div className="mb-2 flex items-center justify-between px-1 text-[10px] uppercase tracking-[0.2em] text-faint">
        <span>{cut === null ? (ru ? "кликни на своё сообщение" : "click your message") : (ru ? "время повёрнуто назад" : "time turned back")}</span>
        {cut !== null && (
          <button onClick={() => setCut(null)} className="text-accent hover:text-milk">
            {ru ? "↺ сброс" : "↺ reset"}
          </button>
        )}
      </div>
      <div className="space-y-1">
        {MESSAGES.map((m, i) => {
          const gone = cut !== null && i > cut;
          const willGo = cut === null && hover !== null && i > hover;
          const mine = m.who === "ты" || m.who === "you";
          return (
            <div
              key={i}
              onMouseEnter={() => mine && setHover(i)}
              onMouseLeave={() => setHover(null)}
              onClick={() => mine && cut === null && setCut(i)}
              className={`flex gap-3 rounded-md px-2 py-1.5 transition-all duration-500 ${
                mine && cut === null ? "cursor-pointer hover:bg-milk/[0.04]" : ""
              } ${gone ? "pointer-events-none max-h-0 -translate-x-2 py-0 opacity-0 blur-sm" : "max-h-10"} ${
                willGo ? "opacity-30 line-through decoration-milk/30" : ""
              }`}
            >
              <span className={`w-6 shrink-0 ${mine ? "text-accent" : "text-faint"}`}>{m.who}</span>
              <span className={mine ? "text-milk/90" : "text-dim"}>{m.text}</span>
              {mine && hover === i && cut === null && <span className="ml-auto text-accent">↺</span>}
            </div>
          );
        })}
      </div>
      {cut !== null && (
        <div className="fade-up mt-auto px-2 text-dim">
          <span className="text-accent">›</span> {ru ? "как будто этого и не было" : "as if it never happened"}<span className="caret ml-1" style={{ height: "1em" }} />
        </div>
      )}
    </div>
  );
}

/* ---------- 4. voice ---------- */
const SPEECH_EN = "find where we lose the database connection and suggest a fix".split(" ");
const SPEECH_RU = "найди где мы теряем соединение с базой и предложи как это починить".split(" ");

function VoiceDemo({ active }: { active: boolean }) {
  const { lang } = useLang();
  const ru = lang === "ru";
  const SPEECH = ru ? SPEECH_RU : SPEECH_EN;
  const [holding, setHolding] = useState(false);
  const [words, setWords] = useState(0);
  const [auto, setAuto] = useState(true);
  const timer = useRef<number | undefined>(undefined);
  const rootRef = useRef<HTMLDivElement>(null);

  // keyboard: ctrl + r — как в агенте. Слушаем только когда фокус внутри демо,
  // чтобы не перехватывать глобальные хоткеи страницы и браузера.
  useEffect(() => {
    if (!active) return;
    const root = rootRef.current;
    if (!root) return;
    const down = (e: KeyboardEvent) => {
      if (e.ctrlKey && (e.key === "r" || e.key === "R" || e.key === "к" || e.key === "К")) {
        e.preventDefault();
        setAuto(false);
        setHolding(true);
      }
    };
    const up = (e: KeyboardEvent) => {
      if (e.key === "Control" || e.key === "r" || e.key === "R" || e.key === "к" || e.key === "К") setHolding(false);
    };
    root.addEventListener("keydown", down);
    root.addEventListener("keyup", up);
    return () => {
      root.removeEventListener("keydown", down);
      root.removeEventListener("keyup", up);
    };
  }, [active]);

  // auto demo loop until user interacts
  useEffect(() => {
    if (!active || !auto) return;
    let alive = true;
    const loop = () => {
      if (!alive) return;
      setWords(0);
      setHolding(true);
      window.setTimeout(() => alive && setHolding(false), 4200);
      window.setTimeout(loop, 7000);
    };
    const s = window.setTimeout(loop, 600);
    return () => {
      alive = false;
      clearTimeout(s);
    };
  }, [active, auto]);

  useEffect(() => {
    clearInterval(timer.current);
    if (holding) {
      timer.current = window.setInterval(() => {
        setWords((w) => (w >= SPEECH.length ? w : w + 1));
      }, 300);
    }
    return () => clearInterval(timer.current);
  }, [holding]);

  const start = () => {
    setAuto(false);
    if (words >= SPEECH.length) setWords(0);
    setHolding(true);
  };

  return (
    <div
      ref={rootRef}
      tabIndex={0}
      aria-label={ru ? "Демо диктовки: фокус здесь, ctrl+r — говорить" : "Dictation demo: focus here, ctrl+r to talk"}
      className="flex h-full flex-col justify-between rounded-[inherit] p-5 font-mono text-[11.5px] outline-none focus-visible:ring-1 focus-visible:ring-accent/60"
    >
      <div className="flex items-center justify-between text-[10px] uppercase tracking-[0.2em] text-faint">
        <span className={holding ? "text-accent" : ""}>{holding ? (ru ? "● слушаю" : "● listening") : (ru ? "○ тишина" : "○ silence")}</span>
        <span className="flex items-center gap-1.5">
          <span className="kbd">ctrl</span>+<span className="kbd">r</span>
        </span>
      </div>

      <div className="flex h-14 items-center justify-center gap-[3px]">
        {Array.from({ length: 40 }).map((_, i) => (
          <span
            key={i}
            className="w-[2px] origin-center rounded-full bg-accent transition-opacity duration-500"
            style={{
              height: `${20 + Math.abs(Math.sin(i * 1.7)) * 80}%`,
              opacity: holding ? 0.85 : 0.15,
              animation: holding ? `wave ${0.6 + (i % 5) * 0.13}s ease-in-out ${i * 0.03}s infinite` : "none",
              transform: holding ? undefined : "scaleY(0.15)",
            }}
          />
        ))}
      </div>

      <div className="min-h-[3.2em] text-milk/90">
        <span className="mr-2 text-accent">›</span>
        {SPEECH.slice(0, words).map((w, i) => (
          <span key={i} className="fade-up inline-block">
            {w}&nbsp;
          </span>
        ))}
        <span className="caret" style={{ height: "1em" }} />
      </div>

      <button
        onPointerDown={start}
        onPointerUp={() => setHolding(false)}
        onPointerLeave={() => !auto && setHolding(false)}
        onKeyDown={(e) => {
          if (e.key === " " || e.key === "Enter") {
            e.preventDefault();
            start();
          }
        }}
        onKeyUp={() => setHolding(false)}
        aria-pressed={holding}
        className={`select-none rounded-full border py-2 text-[10px] uppercase tracking-[0.25em] transition-all duration-300 focus-visible:outline-none focus-visible:ring-1 focus-visible:ring-accent/60 ${
          holding && !auto ? "border-accent bg-accent/10 text-accent" : "hairline text-dim hover:text-milk"
        }`}
      >
        {ru ? "зажми и говори" : "hold and talk"}
      </button>
      <div className="mt-2 text-center text-[10px] leading-relaxed text-faint">
        {ru ? "демо: в агенте — whistle --mic, Linux, текст стримится по мере речи" : "demo: in the agent — whistle --mic, Linux, text streams as you speak"}
      </div>
    </div>
  );
}

/* ---------- section ---------- */
export default function Philosophy() {
  const { lang } = useLang();
  const ru = lang === "ru";
  const { ref, inView } = useInView<HTMLDivElement>(0.1, false);

  return (
    <section id="philosophy" className="relative px-6 py-32 md:px-10 md:py-48">
      <div className="mx-auto max-w-7xl">
        <Reveal>
          <Eyebrow index="02">философия</Eyebrow>
        </Reveal>
        <div className="mt-10 grid gap-10 md:grid-cols-12">
          <Reveal className="md:col-span-8" delay={100}>
            <h2 className="serif-display text-5xl text-milk md:text-7xl lg:text-8xl">
              {ru ? "Четыре привычки" : "Four habits"}
              <br />
              <span className="italic text-dim">{ru ? "хорошего" : "of a good"}</span> {ru ? "напарника." : "copilot."}
            </h2>
          </Reveal>
          <Reveal className="self-end md:col-span-4" delay={220}>
            <p className="font-mono text-[13px] leading-[1.9] text-dim">
              {ru ? "Без кнопок ради кнопок. Каждая функция — ответ на конкретную ночную боль." : "No buttons for buttons’ sake. Every feature answers one specific nightly pain."}
            </p>
          </Reveal>
        </div>

        <div ref={ref} className="mt-20 grid gap-4 md:grid-cols-2">
          <Card
            index="01 — /btw"
            title={ru ? <>Две мысли <span className="italic text-accent">сразу.</span></> : <>Two thoughts <span className="italic text-accent">at once.</span></>}
            text={ru ? "Основная задача выполняется. Спроси через /btw: боковой поток ответит, не прерывая главный." : "The main job is running. Ask via /btw: the side thread answers without interrupting the main one."}
            demo={<BtwDemo active={inView} />}
            delay={0}
          />
          <Card
            index={ru ? "02 — провайдеры" : "02 — providers"}
            title={ru ? <>Свобода <span className="italic text-accent">выбора.</span></> : <>Freedom <span className="italic text-accent">of choice.</span></>}
            text={ru ? "Двадцать семь провайдеров под одной крышей: Codex OAuth, API-ключи, device flow. /login — вход, /model — выбор модели." : "Twenty-seven providers under one roof: Codex OAuth, API keys, device flow. /login signs in, /model picks the model."}
            demo={<ProvidersDemo active={inView} />}
            delay={120}
          />
          <Card
            index="03 — revert"
            title={ru ? <>Право на <span className="italic text-accent">ошибку.</span></> : <>The right <span className="italic text-accent">to be wrong.</span></>}
            text={ru ? "Клик по своему сообщению: copy или revert. Revert откатывает разговор и файлы." : "Clicking your message opens a menu: copy or revert. Revert rolls the conversation back and returns files through the undo log."}
            demo={<RevertDemo />}
            delay={0}
          />
          <Card
            index="04 — голос"
            title={ru ? <>Слушает и <span className="italic text-accent">слышит.</span></> : <>It listens <span className="italic text-accent">and hears.</span></>}
            text={ru ? "Устали пальцы — нажми ctrl+r и говори. В демо зажми кнопку, в агенте нужен whistle --mic." : "Fingers tired — press ctrl+r and speak. In the demo hold the button; the agent needs whistle --mic."}
            demo={<VoiceDemo active={inView} />}
            delay={120}
          />
        </div>
      </div>
    </section>
  );
}
