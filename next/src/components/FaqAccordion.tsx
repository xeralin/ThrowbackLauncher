"use client";

import { useEffect, useState, type ReactNode } from "react";
import { StrokeIcon, CHEVRON_DOWN } from "@/components/icons";
import type { ThrowbackOS } from "@/lib/bridge";
import { usePlatformView } from "@/lib/platform-view";

export type FaqItem = {
  id: string;
  q: ReactNode;
  a: ReactNode;
  platform?: ThrowbackOS;
};

function Item({ item }: { item: FaqItem }) {
  const [open, setOpen] = useState(false);
  const [pulse, setPulse] = useState(false);
  const answerId = `faq-${item.id}-answer`;

  useEffect(() => {
    function openFromHash() {
      if (window.location.hash.slice(1) !== item.id) return;
      window.history.replaceState(
        window.history.state,
        "",
        window.location.pathname + window.location.search,
      );
      setOpen(true);
      setPulse(true);
      requestAnimationFrame(() =>
        document.getElementById(item.id)?.scrollIntoView({ block: "start" }),
      );
    }
    openFromHash();
    window.addEventListener("hashchange", openFromHash);
    return () => window.removeEventListener("hashchange", openFromHash);
  }, [item.id]);

  return (
    <div
      id={item.id}
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
        <span className="question-title">{item.q}</span>
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
