"use client";

import { useEffect, useRef, useState } from "react";
import { INFO, StrokeIcon } from "@/components/icons";
import { usePopoverDismiss } from "@/lib/popover";

const OPEN_DELAY = 300;
const CLOSE_DELAY = 100;

export function InfoHint({ text }: { text: string }) {
  const [open, setOpen] = useState(false);
  const ref = useRef<HTMLSpanElement>(null);
  const timer = useRef<ReturnType<typeof setTimeout> | null>(null);

  usePopoverDismiss(open, setOpen, ref);

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
        <StrokeIcon d={INFO} className="size-3.5" />
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
