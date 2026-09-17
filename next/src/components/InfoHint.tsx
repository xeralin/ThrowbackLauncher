"use client";

import { useEffect, useRef, useState } from "react";
import { StrokeIcon } from "@/components/icons";

const OPEN_DELAY = 300;
const CLOSE_DELAY = 100;

let openHints = 0;

export function hasOpenInfoHint(): boolean {
  return openHints > 0;
}

export function InfoHint({ text }: { text: string }) {
  const [open, setOpen] = useState(false);
  const ref = useRef<HTMLSpanElement>(null);
  const timer = useRef<ReturnType<typeof setTimeout> | null>(null);

  useEffect(() => {
    if (!open) return;
    openHints += 1;
    function onDown(event: MouseEvent) {
      if (ref.current && !ref.current.contains(event.target as Node)) {
        setOpen(false);
      }
    }
    function onKey(event: KeyboardEvent) {
      if (event.key === "Escape") setOpen(false);
    }
    window.addEventListener("mousedown", onDown);
    window.addEventListener("keydown", onKey);

    return () => {
      openHints -= 1;
      window.removeEventListener("mousedown", onDown);
      window.removeEventListener("keydown", onKey);
    };
  }, [open]);

  useEffect(
    () => () => {
      if (timer.current) clearTimeout(timer.current);
    },
    [],
  );

  function schedule(next: boolean, delay: number) {
    if (timer.current) clearTimeout(timer.current);
    timer.current = setTimeout(() => setOpen(next), delay);
  }

  function show() {
    if (timer.current) clearTimeout(timer.current);
    setOpen(true);
  }

  return (
    <span
      ref={ref}
      className="relative shrink-0"
      onPointerEnter={(event) => {
        if (event.pointerType === "mouse") schedule(true, OPEN_DELAY);
      }}
      onPointerLeave={(event) => {
        if (event.pointerType === "mouse") schedule(false, CLOSE_DELAY);
      }}
    >
      <button
        type="button"
        aria-label="Info"
        aria-expanded={open}
        onClick={show}
        className="flex shrink-0 text-text-muted transition-colors hover:text-text"
      >
        <StrokeIcon className="size-3.5">
          <circle cx="12" cy="12" r="10" />
          <line x1="12" x2="12" y1="16" y2="12" />
          <line x1="12" x2="12.01" y1="8" y2="8" />
        </StrokeIcon>
      </button>
      <span
        aria-hidden={!open}
        className={`absolute left-0 top-full z-20 mt-1.5 w-60 rounded-md border border-border bg-surface-2 p-2 text-left text-ui leading-snug text-text shadow-lg transition-[opacity,display] transition-discrete duration-100 ${
          open
            ? "opacity-100 starting:opacity-0"
            : "pointer-events-none hidden opacity-0"
        }`}
      >
        {text}
      </span>
    </span>
  );
}
