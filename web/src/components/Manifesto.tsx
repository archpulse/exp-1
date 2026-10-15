import { useLang } from "../i18n";
import { Reveal } from "./shared";

export default function Manifesto() {
  const { lang } = useLang();
  const ru = lang === "ru";
  return (
    <section className="relative px-6 py-32 md:px-10 md:py-44">
      <div className="mx-auto max-w-5xl text-center">
        <Reveal>
          <div className="mx-auto mb-12 h-16 w-px bg-gradient-to-b from-transparent to-accent/60" />
        </Reveal>
        <Reveal delay={100}>
          <blockquote className="serif-display text-4xl leading-[1.1] text-milk md:text-6xl lg:text-7xl">
            {ru ? (<>Хорошая программа должна <span className="italic text-dim">исчезать.</span><br />Остаёшься ты, курсор <span className="italic text-accent">и мысль.</span></>) : (<>A good program should <span className="italic text-dim">disappear.</span><br />What remains is you, the cursor <span className="italic text-accent">and the thought.</span></>)}
          </blockquote>
        </Reveal>
        <Reveal delay={250}>
          <p className="mx-auto mt-12 max-w-lg font-mono text-[12px] leading-[1.9] text-dim">
            {ru
              ? "exp-1 родился из усталости. От иконок, панелей и вечного «подождите». Мы убрали всё, что можно. Потом убрали ещё немного."
              : "exp-1 was born of fatigue. Of icons, panels, notifications, popups and the eternal “please wait, loading”. We removed everything we could. Then we removed a little more."}
          </p>
        </Reveal>
      </div>
    </section>
  );
}
