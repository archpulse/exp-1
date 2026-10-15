import { useEffect, useState } from "react";
import { INSTALL_CMDS, useLang, type OS } from "../i18n";

const OSES: OS[] = ["linux", "mac", "windows"];

function Install() {
  const { s } = useLang();
  const [os, setOs] = useState<OS>("linux");
  const [copied, setCopied] = useState<string | null>(null);
  const cmds = INSTALL_CMDS[os];

  const copyAll = async () => {
    const text = cmds.join("\n");
    try {
      await navigator.clipboard.writeText(text);
    } catch {
      const ta = document.createElement("textarea");
      ta.value = text;
      document.body.appendChild(ta);
      ta.select();
      document.execCommand("copy");
      ta.remove();
    }
    setCopied(text);
    setTimeout(() => setCopied((c) => (c === text ? null : c)), 1600);
  };

  return (
    <div className="mx-auto w-full max-w-2xl">
      <div className="mb-5 text-center font-mono text-[10px] uppercase tracking-[0.3em] text-faint">
        {s("hero.install")}
      </div>
      <div className="mb-4 flex justify-center gap-2" role="tablist" aria-label={s("hero.install")}>
        {OSES.map((o) => (
          <button
            key={o}
            role="tab"
            aria-selected={os === o}
            onClick={() => setOs(o)}
            className={`rounded-full border px-5 py-1.5 font-mono text-[11px] uppercase tracking-[0.2em] transition-all duration-300 focus-visible:outline-none focus-visible:ring-1 focus-visible:ring-accent/60 ${
              os === o ? "border-accent/60 bg-accent/10 text-milk" : "hairline text-dim hover:text-milk"
            }`}
          >
            {o}
          </button>
        ))}
      </div>
      <button
        onClick={copyAll}
        className="group flex w-full flex-col gap-2 overflow-hidden rounded-2xl border hairline bg-ink-2/90 px-6 py-5 text-left font-mono text-[13px] leading-relaxed transition-colors hover:border-milk/15 focus-visible:outline-none focus-visible:ring-1 focus-visible:ring-accent/60 md:text-sm"
      >
        {cmds.map((cmd, i) => (
          <span key={`${os}-${i}`} className="fade-up flex items-baseline gap-3" style={{ animationDelay: `${i * 60}ms` }}>
            <span className="shrink-0 text-accent">$</span>
            <span className="break-all text-milk">{cmd}</span>
          </span>
        ))}
        <span
          className={`mt-2 self-start rounded-full px-4 py-1.5 text-[11px] uppercase tracking-[0.2em] transition-all duration-500 ${
            copied ? "bg-accent text-ink" : "bg-milk/[0.06] text-dim group-hover:text-milk"
          }`}
        >
          {copied ? s("hero.copied") : s("hero.copy")}
        </span>
      </button>
      <p className="mt-4 text-center font-mono text-[10px] leading-relaxed tracking-[0.15em] text-faint">
        {s("hero.note")}
      </p>
    </div>
  );
}

export default function Hero() {
  const { s } = useLang();
  const [mounted, setMounted] = useState(false);
  useEffect(() => {
    const t = requestAnimationFrame(() => setMounted(true));
    return () => cancelAnimationFrame(t);
  }, []);
  const r = (d: number) => ({
    className: `reveal ${mounted ? "is-in" : ""}`,
    style: { transitionDelay: `${d}ms` },
  });

  return (
    <section id="top" className="relative flex min-h-screen flex-col items-center justify-center px-6 pb-24 pt-32 md:px-10">
      <div {...r(0)}>
        <div className="mb-10 flex items-center justify-center gap-4 font-mono text-[11px] uppercase tracking-[0.3em] text-dim">
          <span className="h-px w-8 bg-milk/20" />
          {s("hero.eyebrow")}
          <span className="h-px w-8 bg-milk/20" />
        </div>
      </div>

      <h1 className="serif-display text-center text-[13vw] text-milk sm:text-7xl md:text-8xl lg:text-[8.5rem]">
        <span {...r(120)} className={`${r(120).className} block`}>{s("hero.h1a")}</span>
        <span {...r(260)} className={`${r(260).className} block italic text-accent`}>{s("hero.h1b")}</span>
      </h1>

      <div {...r(420)}>
        <p className="mx-auto mt-10 max-w-xl text-center font-mono text-[13px] leading-[1.9] text-dim md:text-sm">
          {s("hero.sub")}
        </p>
      </div>

      <div {...r(740)} className={`${r(740).className} mt-14 w-full`}>
        <Install />
      </div>

      <div {...r(900)}>
        <div className="mt-16 flex flex-wrap items-center justify-center gap-x-10 gap-y-3 font-mono text-[11px] uppercase tracking-[0.22em] text-faint">
          <span><span className="text-milk/80">27</span> {s("hero.statProv")}</span>
          <span className="hidden h-3 w-px bg-milk/10 sm:block" />
          <span><span className="text-milk/80">/btw</span> {s("hero.statBtw")}</span>
          <span className="hidden h-3 w-px bg-milk/10 sm:block" />
          <span><span className="text-milk/80">ctrl+o</span> {s("hero.statSil")}</span>
        </div>
        <p className="mx-auto mt-6 max-w-xl text-center font-mono text-[11px] leading-relaxed text-faint">
          {s("hero.disclaimer")}
        </p>
      </div>

      <a
        href="#start"
        className="absolute bottom-8 left-1/2 -translate-x-1/2 font-mono text-[10px] uppercase tracking-[0.35em] text-faint transition-colors hover:text-dim"
      >
        {s("hero.more")}
      </a>
    </section>
  );
}
