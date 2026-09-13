"use client";

import { useEffect, useState } from "react";
import { StrokeIcon, CHEVRON_DOWN } from "@/components/icons";
import type { ReactNode } from "react";
import { usePlatformView } from "@/lib/platform-view";

export type FaqItem = {
  id: string;
  q: string;
  display?: ReactNode;
  a: ReactNode;
  platform?: "windows" | "linux";
};

function Item({ item }: { item: FaqItem }) {
  const [open, setOpen] = useState(false);
  const [pulse, setPulse] = useState(false);
  const answerId = `faq-${item.id}-answer`;
  const anchor = item.id;

  useEffect(() => {
    function openFromHash() {
      if (window.location.hash.slice(1) !== anchor) return;
      window.history.replaceState(
        window.history.state,
        "",
        window.location.pathname + window.location.search,
      );
      setOpen(true);
      setPulse(true);
      requestAnimationFrame(() =>
        document.getElementById(anchor)?.scrollIntoView({ block: "start" }),
      );
    }
    openFromHash();
    window.addEventListener("hashchange", openFromHash);
    return () => window.removeEventListener("hashchange", openFromHash);
  }, [anchor]);

  return (
    <div
      id={anchor}
      data-reveal
      onAnimationEnd={(event) => {
        if (event.animationName === "hashPulse") setPulse(false);
      }}
      className={`question${open ? " open row-selected" : ""}${pulse ? " hash-pulse" : ""}`}
    >
      <button
        type="button"
        className="question-header"
        aria-expanded={open}
        aria-controls={answerId}
        onClick={() => setOpen((value) => !value)}
      >
        <span className="question-title">{item.display ?? item.q}</span>
        <StrokeIcon d={CHEVRON_DOWN} className="question-chevron" />
      </button>
      <div id={answerId} className="answer" inert={!open}>
        <div className="answer-clip">
          <div className="answer-inner prose">{item.a}</div>
        </div>
      </div>
    </div>
  );
}

export function FaqAccordion({ items }: { items: FaqItem[] }) {
  const os = usePlatformView();
  const visible = items.filter(
    (item) => !item.platform || item.platform === os,
  );
  return (
    <div className="faq-list">
      {visible.map((item) => (
        <Item key={item.id} item={item} />
      ))}
    </div>
  );
}
