import { useState } from "react";
import { INSTALL_CMDS, useLang, type OS } from "../i18n";
import { Eyebrow, Reveal } from "./shared";

const OSES: OS[] = ["linux", "mac", "windows"];
const BRANCH: Record<OS, string> = { linux: "main", mac: "mac", windows: "windows" };

const KEYS_EN: [string, string][] = [
  ["enter", "send"],
  ["ctrl+/", "main ⇄ side"],
  ["ctrl+shift+a", "permission mode"],
  ["esc esc", "abort the turn"],
  ["ctrl+o", "tool output"],
  ["ctrl+r", "dictate (whistle --mic)"],
  ["ctrl+g / ctrl+t", "newest / oldest"],
];
const KEYS_RU: [string, string][] = [
  ["enter", "отправить"],
  ["ctrl+/", "главный ⇄ боковой"],
  ["ctrl+shift+a", "режим разрешений"],
  ["esc esc", "прервать ход"],
  ["ctrl+o", "вывод инструментов"],
  ["ctrl+r", "диктовка (whistle --mic)"],
  ["ctrl+g / ctrl+t", "новейшее / старейшее"],
];

export default function Start() {
  const { lang, s } = useLang();
  const [os, setOs] = useState<OS>("linux");
  const [copied, setCopied] = useState<string | null>(null);
  const ru = lang === "ru";
  const KEYS = ru ? KEYS_RU : KEYS_EN;

  const copy = async (cmd: string) => {
    try {
      await navigator.clipboard.writeText(cmd);
    } catch {
      const ta = document.createElement("textarea");
      ta.value = cmd;
      document.body.appendChild(ta);
      ta.select();
      document.execCommand("copy");
      ta.remove();
    }
    setCopied(cmd);
    setTimeout(() => setCopied((c) => (c === cmd ? null : c)), 1600);
  };

  const steps = [
    { n: "01", cmd: INSTALL_CMDS[os][0], title: ru ? "склонируй" : "clone", text: s("start.s0x") },
    { n: "02", cmd: INSTALL_CMDS[os].slice(1).join(os === "windows" ? "\n" : " && "), title: ru ? "ветка + сборка" : "switch + build", text: s("start.s1x") },
    { n: "03", cmd: "/login", title: ru ? "войди" : "sign in", text: s("start.s2x") },
    { n: "04", cmd: "/model", title: ru ? "выбери" : "pick", text: s("start.s3x") },
  ];

  return (
    <section id="start" className="relative px-6 py-32 md:px-10 md:py-48">
      <div className="mx-auto max-w-7xl">
        <Reveal>
          <Eyebrow index="00">{s("start.eyebrow")}</Eyebrow>
        </Reveal>
        <div className="mt-10 grid gap-10 md:grid-cols-12">
          <Reveal className="md:col-span-7" delay={100}>
            <h2 className="serif-display text-5xl text-milk md:text-7xl lg:text-8xl">
              {ru ? (<>Четыре команды <span className="italic text-accent">до работы.</span></>) : (<>Four commands <span className="italic text-accent">to work.</span></>)}
            </h2>
          </Reveal>
          <Reveal className="self-end md:col-span-5" delay={220}>
            <p className="max-w-md font-mono text-[13px] leading-[1.9] text-dim">
              {s("start.intro")}
            </p>
          </Reveal>
        </div>

        <Reveal delay={100}>
          <div className="mt-14 flex flex-wrap gap-2" role="tablist" aria-label="os">
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
                {o} · {BRANCH[o]}
              </button>
            ))}
          </div>
        </Reveal>

        <div className="mt-8 grid gap-4 md:grid-cols-2">
          {steps.map((st, i) => (
            <Reveal key={`${os}-${st.n}`} delay={i * 100}>
              <button
                onClick={() => copy(st.cmd)}
                title={s("start.copy")}
                className="group flex h-full w-full flex-col rounded-2xl border hairline bg-ink-2/50 p-7 text-left transition-colors duration-500 hover:border-milk/15 focus-visible:outline-none focus-visible:ring-1 focus-visible:ring-accent/60 md:p-9"
              >
                <div className="flex items-baseline justify-between font-mono text-[11px] uppercase tracking-[0.22em] text-faint">
                  <span className="text-accent">{st.n}</span>
                  <span>{copied === st.cmd ? s("start.copied") : s("start.copy")}</span>
                </div>
                <div className="mt-6 break-all whitespace-pre-line font-mono text-base text-milk md:text-lg">
                  <span className="mr-2 text-accent">$</span>
                  {st.cmd}
                </div>
                <div className="mt-3 font-serif text-3xl font-light text-milk">{st.title}</div>
                <p className="mt-3 font-mono text-[12.5px] leading-[1.9] text-dim">{st.text}</p>
              </button>
            </Reveal>
          ))}
        </div>

        <Reveal delay={120}>
          <button
            onClick={() => copy(INSTALL_CMDS[os].join("\n"))}
            className="mt-6 w-full rounded-2xl border border-accent/30 bg-accent/[0.06] px-6 py-4 font-mono text-[12px] uppercase tracking-[0.2em] text-milk transition-colors hover:border-accent/60 focus-visible:outline-none focus-visible:ring-1 focus-visible:ring-accent/60"
          >
            {ru ? "скопировать всё для" : "copy all for"} {os} — {copied ? s("start.copied") : s("start.copy")}
          </button>
        </Reveal>

        <Reveal delay={150}>
          <div className="mt-14 overflow-hidden rounded-2xl border hairline bg-ink-2/50">
            <div className="border-b hairline px-6 py-3 font-mono text-[10px] uppercase tracking-[0.25em] text-faint">
              {s("start.keys")}
            </div>
            <dl className="grid sm:grid-cols-2 lg:grid-cols-3">
              {KEYS.map(([k, v]) => (
                <div key={k} className="flex items-center gap-4 border-b hairline px-6 py-4 font-mono text-[12px] last:border-b-0 sm:[&:nth-last-child(2)]:border-b-0 lg:[&:nth-last-child(2)]:border-b lg:[&:nth-last-child(3)]:border-b-0">
                  <dt>
                    <span className="kbd whitespace-nowrap">{k}</span>
                  </dt>
                  <dd className="text-dim">{v}</dd>
                </div>
              ))}
              <div className="flex items-center gap-4 px-6 py-4 font-mono text-[12px] lg:border-b-0">
                <dt>
                  <span className="kbd whitespace-nowrap">/btw …</span>
                </dt>
                <dd className="text-dim">{ru ? "вопрос вбок, не прерывая" : "ask aside, no interrupt"}</dd>
              </div>
            </dl>
          </div>
        </Reveal>
      </div>
    </section>
  );
}
