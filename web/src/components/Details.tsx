import { useRef, useState, type ReactNode } from "react";
import { useLang } from "../i18n";
import { Eyebrow, Reveal } from "./shared";

const ROWS = [
  ["/auth/login", "38 ms", "41 ms", "↓ 7%"],
  ["/orders", "212 ms", "96 ms", "↓ 55%"],
  ["/search", "540 ms", "188 ms", "↓ 65%"],
  ["/profile", "64 ms", "62 ms", "—"],
];

function Fraction({ top, bottom }: { top: ReactNode; bottom: ReactNode }) {
  return (
    <span className="mx-1 inline-flex flex-col items-center align-middle leading-none">
      <span className="px-1 pb-1">{top}</span>
      <span className="w-full border-t border-milk/60" />
      <span className="px-1 pt-1">{bottom}</span>
    </span>
  );
}

export default function Details() {
  const { lang } = useLang();
  const ru = lang === "ru";
  const [open, setOpen] = useState(false);
  const panelRef = useRef<HTMLDivElement>(null);

  const HIDDEN = ru
    ? [
        { tool: "read", arg: "src/db/queries.rs", meta: "400 строк макс" },
        { tool: "grep", arg: '"SELECT .* FROM orders"', meta: "без чтения в контекст" },
        { tool: "edit", arg: "src/db/queries.rs", meta: "точная замена" },
        { tool: "shell", arg: "bench --runs 50", meta: "фон после 60с" },
        { tool: "websearch", arg: "индекс для N+1", meta: "Exa, без ключа" },
      ]
    : [
        { tool: "read", arg: "src/db/queries.rs", meta: "400 lines max" },
        { tool: "grep", arg: '"SELECT .* FROM orders"', meta: "no context reads" },
        { tool: "edit", arg: "src/db/queries.rs", meta: "exact replacement" },
        { tool: "shell", arg: "bench --runs 50", meta: "background after 60s" },
        { tool: "websearch", arg: "an index for N+1", meta: "Exa, no key" },
      ];

  return (
    <section id="details" className="relative px-6 py-32 md:px-10 md:py-48">
      <div className="mx-auto max-w-7xl">
        <Reveal>
          <Eyebrow index="03">{ru ? "эстетика деталей" : "detail aesthetics"}</Eyebrow>
        </Reveal>

        <div className="mt-10 grid gap-16 lg:grid-cols-12">
          <div className="lg:col-span-5">
            <Reveal delay={100}>
              <h2 className="serif-display text-5xl text-milk md:text-7xl">
                {ru ? (<>Красиво — <br /><span className="italic text-accent">даже в терминале.</span></>) : (<>Beautiful — <br /><span className="italic text-accent">even in the terminal.</span></>)}
              </h2>
            </Reveal>
            <Reveal delay={200}>
              <p className="mt-10 max-w-md font-mono text-[13px] leading-[1.9] text-dim">
                {ru
                  ? "Markdown и LaTeX рисуются внутри TUI: таблицы с ровными колонками, формулы с дробями и индексами. Весь технический шум — read, grep, shell — сложен под капот за ctrl+o."
                  : "Markdown and LaTeX render inside the TUI: tables with even columns, formulas with fractions and indices. All the technical noise — read, grep, shell — folds away behind ctrl+o."}
              </p>
            </Reveal>

            <Reveal delay={300}>
              <dl className="mt-14 space-y-0 border-t hairline font-mono text-[12px]">
                {(ru
                  ? [
                      ["таблицы", "Pipe-таблицы с box-drawing границами и выравниванием колонок."],
                      ["формулы", "\\frac, \\sqrt, индексы, \\mathbb — без внешнего TeX."],
                      ["тишина", "В транскрипте — вызовы. Вывод — под ctrl+o."],
                    ]
                  : [
                      ["tables", "Pipe tables with box-drawing borders and column alignment."],
                      ["formulas", "\\frac, \\sqrt, indices, \\mathbb — no external TeX."],
                      ["silence", "The transcript carries calls. Output lives under ctrl+o."],
                    ]
                ).map(([k, v]) => (
                  <div key={k} className="grid grid-cols-[110px_1fr] gap-4 border-b hairline py-5">
                    <dt className="uppercase tracking-[0.22em] text-accent">{k}</dt>
                    <dd className="text-dim">{v}</dd>
                  </div>
                ))}
              </dl>
            </Reveal>
          </div>

          <Reveal delay={200} className="lg:col-span-7">
            <div className="relative">
              <div className="pointer-events-none absolute -inset-10 rounded-[40px] bg-accent/[0.04] blur-3xl" />
              <div
                ref={panelRef}
                tabIndex={0}
                role="region"
                aria-label={ru ? "Демо скрытого вывода" : "Hidden-output demo: focus here, ctrl+o to show"}
                onKeyDown={(e) => {
                  if (e.ctrlKey && (e.key === "o" || e.key === "O")) {
                    e.preventDefault();
                    setOpen((o) => !o);
                  }
                  if (e.key === "Escape" && open) setOpen(false);
                }}
                className="relative overflow-hidden rounded-2xl border hairline bg-ink-2 outline-none focus-visible:ring-1 focus-visible:ring-accent/60"
              >
                <div className="flex items-center justify-between border-b hairline px-6 py-3 font-mono text-[10px] uppercase tracking-[0.25em] text-faint">
                  <span>exp-1 · ~/shop</span>
                  <span>codex</span>
                </div>
                <div className="space-y-6 px-6 py-6 font-mono text-[12.5px] leading-[1.85]">
                  <div>
                    <button
                      onClick={() => setOpen((o) => !o)}
                      aria-expanded={open}
                      className="group flex w-full items-center gap-3 rounded text-left text-faint transition-colors hover:text-dim focus-visible:outline-none focus-visible:ring-1 focus-visible:ring-accent/60"
                    >
                      <span className={`transition-transform duration-300 ${open ? "rotate-90" : ""}`}>&gt;</span>
                      <span className="uppercase tracking-[0.2em]">{ru ? "вызовы инструментов — клик или ctrl+o" : "tool calls — click or ctrl+o"}</span>
                      <span className="ml-auto text-[10px]">{open ? (ru ? "спрятать" : "hide") : "5"}</span>
                    </button>
                    {open && (
                      <div className="fade-up">
                        <ul className="mt-3 space-y-1.5 border-l border-milk/10 pl-5 text-[11.5px]">
                          {HIDDEN.map((h) => (
                            <li key={`${h.tool}-${h.arg}`} className="flex gap-3 text-faint">
                              <span className="w-10 text-accent/70">{h.tool}</span>
                              <span className="truncate text-dim">{h.arg}</span>
                              <span className="ml-auto shrink-0">{h.meta}</span>
                            </li>
                          ))}
                        </ul>
                      </div>
                    )}
                  </div>

                  <div className="leading-relaxed text-milk/90">
                    <span className="mr-3 text-accent">◦</span>
                    {ru ? "Поиск делал N+1 запрос к базе. Собрал в один, добавил индекс." : "The search made N+1 queries to the database. Folded into one, added an index."}
                  </div>

                  <div className="overflow-x-auto">
                    <table className="w-full min-w-[420px] border-collapse text-left text-[12px]">
                      <caption className="pb-3 text-left text-[10px] uppercase tracking-[0.2em] text-faint">
                        {ru ? "иллюстрация ответа — цифры выдуманы" : "an illustration of an answer — numbers invented"}
                      </caption>
                      <thead>
                        <tr className="border-b border-milk/15 text-[10px] uppercase tracking-[0.2em] text-faint">
                          <th className="py-2.5 pr-4 font-normal">{ru ? "маршрут" : "route"}</th>
                          <th className="py-2.5 pr-4 text-right font-normal">{ru ? "было" : "before"}</th>
                          <th className="py-2.5 pr-4 text-right font-normal">{ru ? "стало" : "after"}</th>
                          <th className="py-2.5 text-right font-normal">Δ</th>
                        </tr>
                      </thead>
                      <tbody>
                        {ROWS.map((r) => {
                          const hl = r[0] === "/search";
                          return (
                            <tr key={r[0]} className="border-b hairline">
                              <td className={`py-2.5 pr-4 ${hl ? "text-milk" : "text-dim"}`}>{r[0]}</td>
                              <td className="py-2.5 pr-4 text-right tabular-nums text-faint">{r[1]}</td>
                              <td className="py-2.5 pr-4 text-right tabular-nums text-milk/90">{r[2]}</td>
                              <td className={`py-2.5 text-right tabular-nums ${hl ? "text-accent" : "text-dim"}`}>{r[3]}</td>
                            </tr>
                          );
                        })}
                      </tbody>
                    </table>
                  </div>

                  <div className="rounded-xl border hairline bg-ink px-6 py-6">
                    <div className="mb-4 text-[10px] uppercase tracking-[0.2em] text-faint">{ru ? "сложность до и после" : "complexity before and after"}</div>
                    <div className="flex flex-wrap items-center gap-x-8 gap-y-4 font-serif text-2xl text-milk md:text-[1.7rem]">
                      <span className="italic">
                        T<sub className="text-[0.55em] not-italic">{ru ? "до" : "before"}</sub> = <span className="not-italic">∑</span>
                        <sub className="text-[0.55em]">i=1</sub>
                        <sup className="-ml-2 text-[0.55em]">n</sup> (q + r<sub className="text-[0.55em]">i</sub>)
                      </span>
                      <span className="text-faint">→</span>
                      <span className="italic">
                        T<sub className="text-[0.55em] not-italic">{ru ? "после" : "after"}</sub> = q +
                        <Fraction top={<>n · r</>} bottom={<>k</>} />
                      </span>
                    </div>
                    <div className="mt-4 text-[11px] text-dim">
                      {ru ? (<>где <span className="font-serif italic text-milk/80">k</span> — размер пачки, <span className="font-serif italic text-milk/80">q</span> — цена одного похода в базу.</>) : (<>where <span className="font-serif italic text-milk/80">k</span> is the batch size, <span className="font-serif italic text-milk/80">q</span> the cost of one trip to the database.</>)}
                    </div>
                  </div>

                  <div className="flex items-center text-milk">
                    <span className="mr-3 text-accent">›</span>
                    <span className="caret" />
                  </div>
                </div>
              </div>
            </div>
          </Reveal>
        </div>
      </div>
    </section>
  );
}
