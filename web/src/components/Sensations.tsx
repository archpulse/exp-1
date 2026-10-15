import { useCallback, useEffect, useRef, useState } from "react";
import { useLang } from "../i18n";
import { Eyebrow, Reveal, useInView } from "./shared";

const HEAVY_TOTAL = 7400;
const EXP_TOTAL = 84;

function fmt(ms: number, msWord: string, sWord: string) {
  if (ms < 1000) return `${Math.round(ms)} ${msWord}`;
  return `${(ms / 1000).toFixed(2)} ${sWord}`;
}

export default function Sensations() {
  const { lang, s } = useLang();
  const ru = lang === "ru";
  const [t, setT] = useState(0);
  const [running, setRunning] = useState(false);
  const [done, setDone] = useState(false);
  const raf = useRef(0);
  const start = useRef(0);
  const { ref, inView } = useInView<HTMLDivElement>(0.4);

  const steps: string[] = ru
    ? ["Запуск оболочки…", "Восстановление 11 вкладок…", "Загрузка 38 расширений…", "Индексация 14 382 файлов…", "Синхронизация настроек…", "Проверка обновлений…", "Готово. Наконец-то."]
    : ["Starting the shell…", "Restoring 11 tabs…", "Loading 38 extensions…", "Indexing 14,382 files…", "Syncing settings…", "Checking for updates…", "Done. Finally."];
  const at = [0, 900, 1900, 3300, 4900, 6100, HEAVY_TOTAL];
  const msWord = ru ? "мс" : "ms";
  const sWord = ru ? "с" : "s";

  const run = useCallback(() => {
    cancelAnimationFrame(raf.current);
    setDone(false);
    setRunning(true);
    setT(0);
    start.current = performance.now();
    const loop = (now: number) => {
      const el = now - start.current;
      if (el >= HEAVY_TOTAL) {
        setT(HEAVY_TOTAL);
        setRunning(false);
        setDone(true);
        return;
      }
      setT(el);
      raf.current = requestAnimationFrame(loop);
    };
    raf.current = requestAnimationFrame(loop);
  }, []);

  useEffect(() => {
    if (inView) run();
  }, [inView, run]);

  useEffect(() => () => cancelAnimationFrame(raf.current), []);

  const heavyProgress = Math.min(1, t / HEAVY_TOTAL);
  const expReady = t >= EXP_TOTAL || done;
  const heavySteps = steps.filter((_, i) => at[i] <= t);
  const idle = !running && !done;
  const cursorLines = ru
    ? ["Курсор уже ждёт тебя.", "Ты успел сформулировать вопрос.", "Получил ответ. Поправил функцию.", "Закоммитил. А там всё ещё грузится."]
    : ["The cursor is already waiting for you.", "You managed to phrase the question.", "Got the answer. Fixed the function.", "Committed. And that thing is still loading."];

  return (
    <section id="feel" className="relative px-6 py-32 md:px-10 md:py-48">
      <div className="mx-auto max-w-7xl">
        <Reveal>
          <Eyebrow index="01">{s("nav.feel")}</Eyebrow>
        </Reveal>
        <div className="mt-10 grid gap-10 md:grid-cols-12">
          <Reveal className="md:col-span-7" delay={100}>
            <h2 className="serif-display text-5xl text-milk md:text-7xl lg:text-8xl">
              {ru ? (<>Пока одни <span className="italic text-dim">грузятся</span>,<br />ты уже <span className="italic text-accent">пишешь.</span></>) : (<>While some <span className="italic text-dim">are loading,</span><br />you are already <span className="italic text-accent">writing.</span></>)}
            </h2>
          </Reveal>
          <Reveal className="self-end md:col-span-5" delay={220}>
            <p className="mt-10 max-w-md font-mono text-[13px] leading-[1.9] text-dim">
              {ru
                ? "Художественная иллюстрация: тяжёлый старт слева — собирательный образ. Это демо страницы, а не замер. Реальный факт: ~1% ядра в простое."
                : "An artistic illustration of the feeling: the heavy start on the left is a composite image. This is page demo, not a benchmark. A real fact from the README: ~1% of one core idle."}
            </p>
          </Reveal>
        </div>

        <div ref={ref} className="mt-20 grid gap-4 md:grid-cols-2">
          <Reveal delay={100}>
            <div className="relative h-full overflow-hidden rounded-2xl border hairline bg-ink-2/60 p-7 md:p-9">
              <div className="flex items-baseline justify-between font-mono text-[11px] uppercase tracking-[0.22em] text-faint">
                <span>{ru ? "обычная тяжёлая IDE" : "a typical heavy IDE"}</span>
                <span className={done ? "text-milk/70" : ""}>{done ? (ru ? "открылась" : "opened") : running ? (ru ? "ждём…" : "waiting…") : "—"}</span>
              </div>
              <div className="mt-8 font-serif text-7xl font-light tabular-nums text-milk/40 md:text-8xl">
                {fmt(Math.min(t, HEAVY_TOTAL), msWord, sWord)}
              </div>
              <div className="mt-8 h-px w-full bg-milk/10">
                <div className="h-px bg-milk/40 transition-all duration-100" style={{ width: `${heavyProgress * 100}%` }} />
              </div>
              <div className="mt-6 h-[188px] space-y-2 font-mono text-[13px] text-dim">
                {heavySteps.map((st, i) => (
                  <p key={i} className={i === heavySteps.length - 1 ? "text-milk/80" : ""}>{st}</p>
                ))}
              </div>
            </div>
          </Reveal>
          <Reveal delay={220}>
            <div className="relative h-full overflow-hidden rounded-2xl border border-accent/20 bg-ink-2/60 p-7 md:p-9">
              <div className="flex items-baseline justify-between font-mono text-[11px] uppercase tracking-[0.22em] text-faint">
                <span>exp-1</span>
                <span className="text-accent">{expReady ? (ru ? "готов" : "ready") : idle ? "—" : "…"}</span>
              </div>
              <div className="relative mt-8 font-serif text-7xl font-light tabular-nums text-accent md:text-8xl">
                {fmt(expReady ? EXP_TOTAL : t, msWord, sWord)}
              </div>
              <div className="relative mt-8 h-px w-full bg-milk/10">
                <div
                  className="h-px bg-accent shadow-[0_0_10px_var(--accent)] transition-all duration-100"
                  style={{ width: expReady ? "100%" : `${(t / EXP_TOTAL) * 100}%` }}
                />
              </div>
              <div className="relative mt-6 h-[188px] font-mono text-[13px]">
                {expReady && (
                  <div className="fade-up">
                    <div className="flex items-center text-milk">
                      <span className="mr-3 text-accent">›</span>
                      <span className="caret" />
                    </div>
                    <div className="mt-8 space-y-2 text-[12px] text-dim">
                      {t > 600 && <p className="fade-up">{cursorLines[0]}</p>}
                      {t > 2200 && <p className="fade-up">{cursorLines[1]}</p>}
                      {t > 4200 && <p className="fade-up">{cursorLines[2]}</p>}
                      {t > 6400 && <p className="fade-up text-milk/80">{cursorLines[3]}</p>}
                    </div>
                  </div>
                )}
              </div>
            </div>
          </Reveal>
        </div>

        <Reveal delay={150}>
          <div className="mt-14">
            <div className="relative h-14">
              <div className="absolute inset-x-0 top-1/2 h-px bg-milk/10" />
              <Marker pos={EXP_TOTAL / HEAVY_TOTAL} label="exp-1 · 84 ms" accent up />
              <Marker pos={150 / HEAVY_TOTAL} label={ru ? "blink · ~150 ms" : "eye blink · ~150 ms"} offset />
              <Marker pos={1} label={ru ? "IDE · 7.4 с" : "IDE · 7.4 s"} />
              <div
                className="absolute top-1/2 h-px bg-milk/40"
                style={{ width: `${heavyProgress * 100}%` }}
              />
            </div>
            <div className="mt-8 flex items-center justify-between font-mono text-[11px] uppercase tracking-[0.22em] text-faint">
              <span>0 ms</span>
              <button
                onClick={run}
                className="rounded-full border hairline px-5 py-2 text-dim transition-colors hover:border-accent/40 hover:text-milk"
              >
                {ru ? "↻ ещё раз" : "↻ replay"}
              </button>
              <span>7.4 s</span>
            </div>
          </div>
        </Reveal>
      </div>
    </section>
  );
}

function Marker({ pos, label, accent, up, offset }: { pos: number; label: string; accent?: boolean; up?: boolean; offset?: boolean }) {
  const right = pos > 0.9;
  return (
    <div className="absolute top-0 h-full" style={{ left: `${pos * 100}%` }}>
      <div className={`absolute left-0 top-1/2 h-3 w-px -translate-y-1/2 ${accent ? "bg-accent" : "bg-milk/30"}`} />
      <div
        className={`absolute whitespace-nowrap font-mono text-[10px] uppercase tracking-[0.18em] ${
          accent ? "text-accent" : "text-faint"
        } ${up ? "top-0" : "bottom-0"} ${right ? "right-0" : offset ? "left-6" : "left-0"}`}
      >
        {label}
      </div>
    </div>
  );
}
