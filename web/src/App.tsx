import { useEffect, useState } from "react";
import Header, { type Accent } from "./components/Header";
import Hero from "./components/Hero";
import Sensations from "./components/Sensations";
import Philosophy from "./components/Philosophy";
import Manifesto from "./components/Manifesto";
import Details from "./components/Details";
import Start from "./components/Start";
import Footer from "./components/Footer";
import { LangProvider } from "./i18n";

function readAccent(): Accent {
  try {
    const v = localStorage.getItem("exp1-accent");
    if (v === "amber" || v === "jade" || v === "mono" || v === "default") return v;
  } catch {
    /* ignore */
  }
  return "default";
}

export default function App() {
  return (
    <LangProvider>
      <Shell />
    </LangProvider>
  );
}

function Shell() {
  const [accent, setAccent] = useState<Accent>(() => {
    const saved = readAccent();
    if (saved === "amber" || saved === "jade" || saved === "mono" || saved === "default") return saved;
    return "default";
  });

  useEffect(() => {
    const root = document.documentElement;
    if (accent === "default") root.removeAttribute("data-accent");
    else root.setAttribute("data-accent", accent);
    try {
      localStorage.setItem("exp1-accent", accent);
    } catch {
      /* ignore */
    }
  }, [accent]);

  return (
    <div className="grain relative min-h-screen">
      <Header accent={accent} setAccent={setAccent} />
      <main>
        <Hero />
        <Divider />
        <Start />
        <Divider />
        <Sensations />
        <Divider />
        <Philosophy />
        <Manifesto />
        <Divider />
        <Details />
      </main>
      <Footer />
    </div>
  );
}

function Divider() {
  return (
    <div className="mx-auto max-w-7xl px-6 md:px-10">
      <div className="h-px bg-gradient-to-r from-transparent via-milk/10 to-transparent" />
    </div>
  );
}
