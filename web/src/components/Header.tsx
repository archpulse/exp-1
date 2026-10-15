import { useEffect, useState } from "react";
import { useLang, type Lang } from "../i18n";

export type Accent = "default" | "amber" | "jade" | "mono";

export default function Header({ accent, setAccent }: { accent: Accent; setAccent: (a: Accent) => void }) {
  const [scrolled, setScrolled] = useState(false);
  const { lang, setLang, s } = useLang();
  useEffect(() => {
    const on = () => setScrolled(window.scrollY > 24);
    on();
    window.addEventListener("scroll", on, { passive: true });
    return () => window.removeEventListener("scroll", on);
  }, []);

  const accentLabels: Record<Accent, string> = {
    default: s("accent.def"),
    amber: s("accent.amber"),
    jade: s("accent.jade"),
    mono: s("accent.mono"),
  };
  const ACCENTS: { id: Accent; color: string }[] = [
    { id: "default", color: "#3b82f6" },
    { id: "amber", color: "#d99120" },
    { id: "jade", color: "#34be8c" },
    { id: "mono", color: "#d6d6d6" },
  ];
  const currentLabel = accentLabels[accent];

  const flip = () => setLang((lang === "en" ? "ru" : "en") as Lang);

  return (
    <header
      className={`fixed inset-x-0 top-0 z-50 transition-all duration-700 ${
        scrolled ? "border-b hairline bg-ink/75 backdrop-blur-xl" : "border-b border-transparent"
      }`}
    >
      <div className="mx-auto flex h-16 max-w-7xl items-center justify-between px-6 md:px-10">
        <a href="#top" className="group flex items-center gap-3 font-mono text-sm tracking-tight text-milk">
          <img
            src="/favicon.png"
            alt=""
            width={28}
            height={28}
            className="h-7 w-7 object-contain"
          />
          exp-1
        </a>

        <nav className="hidden items-center gap-10 font-mono text-[11px] uppercase tracking-[0.22em] text-dim md:flex">
          <a href="#start" className="transition-colors hover:text-milk">{s("nav.start")}</a>
          <a href="#feel" className="transition-colors hover:text-milk">{s("nav.feel")}</a>
          <a href="#philosophy" className="transition-colors hover:text-milk">{s("nav.phil")}</a>
          <a href="#details" className="transition-colors hover:text-milk">{s("nav.details")}</a>
        </nav>

        <div className="flex items-center gap-3">
          <button
            onClick={flip}
            aria-label={s("lang.name")}
            title={s("lang.name")}
            className="rounded-full border hairline px-3 py-1.5 font-mono text-[11px] uppercase tracking-[0.2em] text-dim transition-colors hover:border-accent/40 hover:text-milk focus-visible:outline-none focus-visible:ring-1 focus-visible:ring-accent/60"
          >
            {lang === "en" ? "ru" : "en"}
          </button>
          <span className="hidden font-mono text-[11px] uppercase tracking-[0.22em] text-dim lg:inline">
            {s("accent.label")} <span className="text-milk/80">· {currentLabel}</span>
          </span>
          <div
            role="radiogroup"
            aria-label={s("accent.group")}
            className="flex items-center gap-1 rounded-full border hairline bg-ink-2/80 p-1"
          >
            {ACCENTS.map((a) => {
              const active = a.id === accent;
              const label = accentLabels[a.id];
              return (
                <button
                  key={a.id}
                  role="radio"
                  aria-checked={active}
                  aria-label={label}
                  title={label}
                  onClick={() => setAccent(a.id)}
                  className="group relative grid h-7 w-7 place-items-center rounded-full transition-colors hover:bg-milk/5"
                >
                  <span
                    className="block rounded-full transition-all duration-500"
                    style={{
                      background: a.color,
                      width: active ? 10 : 6,
                      height: active ? 10 : 6,
                      opacity: active ? 1 : 0.45,
                      boxShadow: active ? `0 0 14px ${a.color}` : "none",
                    }}
                  />
                  {active && <span className="absolute inset-0 rounded-full border border-milk/15" />}
                </button>
              );
            })}
          </div>
        </div>
      </div>
    </header>
  );
}
