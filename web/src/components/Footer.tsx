import { useLang } from "../i18n";
import { Reveal } from "./shared";

const READMES: [string, string][] = [
  ["en", "../README.md"],
  ["ru", "../README.ru.md"],
  ["uk", "../README.uk.md"],
  ["de", "../README.de.md"],
  ["es", "../README.es.md"],
  ["cs", "../README.cs.md"],
  ["zh", "../README.zh.md"],
  ["ja", "../README.ja.md"],
];

export default function Footer() {
  const { lang, s } = useLang();
  const ru = lang === "ru";
  const credits: [string, string][] = ru
    ? [
        ["режиссура", "тишина"],
        ["оператор", "мигающий курсор"],
        ["в ролях", "ты и двадцать семь моделей"],
        ["музыка", "стук клавиш в 2 часа ночи"],
        ["свет", "экран и настольная лампа"],
      ]
    : [
        ["direction", "silence"],
        ["camera", "the blinking cursor"],
        ["starring", "you and twenty-seven models"],
        ["music", "keystrokes at 2am"],
        ["light", "the screen and a desk lamp"],
      ];

  return (
    <footer className="relative border-t hairline px-6 pb-14 pt-32 md:px-10">
      <div className="mx-auto max-w-3xl text-center">
        <Reveal>
          <div className="serif-display text-7xl italic text-milk md:text-9xl">exp-1</div>
        </Reveal>

        <Reveal delay={150}>
          <dl className="mx-auto mt-20 space-y-4 font-mono text-[11px] uppercase tracking-[0.28em]">
            {credits.map(([k, v]) => (
              <div key={k} className="grid grid-cols-2 gap-6">
                <dt className="text-right text-faint">{k}</dt>
                <dd className="text-left text-milk/75">{v}</dd>
              </div>
            ))}
          </dl>
        </Reveal>

        <Reveal delay={250}>
          <div className="mt-20 flex items-center justify-center gap-5 font-mono text-[11px] uppercase tracking-[0.3em] text-dim">
            <span>Linux</span>
            <span className="h-1 w-1 rounded-full bg-accent" />
            <span>macOS</span>
            <span className="h-1 w-1 rounded-full bg-accent" />
            <span>Windows</span>
          </div>
          <p className="mt-4 font-mono text-[10px] uppercase tracking-[0.25em] text-faint">
            {s("foot.note")}
          </p>
        </Reveal>

        <Reveal delay={300}>
          <nav aria-label={s("foot.docs")} className="mx-auto mt-14 flex max-w-xl flex-wrap items-center justify-center gap-x-6 gap-y-3 font-mono text-[11px] uppercase tracking-[0.22em]">
            <span className="w-full text-center text-faint">{s("foot.readme")}</span>
            {READMES.map(([l, href]) => (
              <a key={l} href={href} className="rounded text-dim transition-colors hover:text-milk focus-visible:outline-none focus-visible:ring-1 focus-visible:ring-accent/60">
                {l}
              </a>
            ))}
          </nav>
          <div className="mx-auto mt-8 flex max-w-xl flex-wrap items-center justify-center gap-x-6 gap-y-3 font-mono text-[11px] uppercase tracking-[0.22em]">
            <a href="#start" className="rounded border hairline px-5 py-2.5 text-milk transition-colors hover:border-accent/50 focus-visible:outline-none focus-visible:ring-1 focus-visible:ring-accent/60">
              {s("foot.install")}
            </a>
          </div>
        </Reveal>

        <div className="mt-24 flex flex-col items-center justify-between gap-4 border-t hairline pt-8 font-mono text-[10px] uppercase tracking-[0.25em] text-faint sm:flex-row">
          <span>© {new Date().getFullYear()} exp-1</span>
          <span>{s("foot.bottom")}</span>
          <a href="#top" className="transition-colors hover:text-milk">{s("foot.top")}</a>
        </div>
      </div>
    </footer>
  );
}
