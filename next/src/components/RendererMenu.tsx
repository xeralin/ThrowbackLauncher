"use client";

import { useEffect, useRef, useState, type ReactNode } from "react";
import { buttonVariants } from "@/components/Button";
import { CHEVRON_DOWN, StrokeIcon } from "@/components/icons";

let openMenus = 0;

export function hasOpenRendererMenu(): boolean {
  return openMenus > 0;
}

export function RendererMenu({
  options,
  active,
  disabled = false,
  onSelect,
  children,
}: {
  options: { arg: string; label: string }[];
  active: string;
  disabled?: boolean;
  onSelect: (arg: string) => void;
  children: ReactNode;
}) {
  const [open, setOpen] = useState(false);
  const ref = useRef<HTMLSpanElement>(null);

  useEffect(() => {
    if (!open) return;
    openMenus += 1;
    function onDown(event: MouseEvent) {
      if (ref.current && !ref.current.contains(event.target as Node))
        setOpen(false);
    }
    function onKey(event: KeyboardEvent) {
      if (event.key === "Escape") setOpen(false);
    }
    window.addEventListener("mousedown", onDown);
    window.addEventListener("keydown", onKey);
    return () => {
      openMenus -= 1;
      window.removeEventListener("mousedown", onDown);
      window.removeEventListener("keydown", onKey);
    };
  }, [open]);

  return (
    <span ref={ref} className="relative inline-flex items-center gap-1">
      {children}
      <button
        type="button"
        aria-label="Renderer"
        aria-haspopup="menu"
        aria-expanded={open}
        disabled={disabled}
        onClick={() => setOpen((value) => !value)}
        className={`inline-flex h-8 w-6 shrink-0 items-center justify-center rounded-md rounded-l-none transition duration-200 ${buttonVariants.primary} disabled:cursor-not-allowed disabled:opacity-40`}
      >
        <StrokeIcon
          d={CHEVRON_DOWN}
          className={`size-3.5 transition-transform duration-200${open ? " rotate-180" : ""}`}
        />
      </button>
      <span
        role="menu"
        aria-label="Renderer"
        className={`absolute left-0 top-full z-20 mt-1 grid w-full transition-[grid-template-rows,visibility] duration-200 ease-out-cubic ${
          open
            ? "[grid-template-rows:1fr]"
            : "invisible [grid-template-rows:0fr]"
        }`}
      >
        <span className="flex min-h-0 flex-col overflow-hidden rounded-md bg-surface shadow-lg ring-1 ring-border ring-inset">
          <span className="flex flex-col p-2">
            {options.map((option) => (
              <button
                key={option.label}
                type="button"
                role="menuitemradio"
                aria-checked={option.arg === active}
                onClick={() => {
                  setOpen(false);
                  if (option.arg !== active) onSelect(option.arg);
                }}
                className={`flex w-full items-center rounded-md border-l-2 border-transparent px-2 py-1 text-left text-ui font-semibold transition-colors ${
                  option.arg === active
                    ? "bg-action-dim text-text"
                    : "text-text-muted hover:bg-surface-2 hover:text-text"
                }`}
              >
                {option.label}
              </button>
            ))}
          </span>
        </span>
      </span>
    </span>
  );
}
