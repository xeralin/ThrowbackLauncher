"use client";

import { useLayoutEffect, useRef, useState, type ReactNode } from "react";

export type TabItem<T extends string> = {
  id: T;
  label: string;
  disabled?: boolean;
};

function Group<T extends string>({
  tabs,
  active,
  onSelect,
  radio = false,
  label,
}: {
  tabs: TabItem<T>[];
  active: T;
  onSelect: (id: T) => void;
  radio?: boolean;
  label?: string;
}) {
  const refs = useRef<Record<string, HTMLButtonElement | null>>({});
  const [indicator, setIndicator] = useState({ left: 0, width: 0 });

  useLayoutEffect(() => {
    function remeasure() {
      const el = refs.current[active];
      if (el) setIndicator({ left: el.offsetLeft, width: el.offsetWidth });
    }
    remeasure();
    window.addEventListener("resize", remeasure);
    document.fonts?.ready.then(remeasure);
    return () => window.removeEventListener("resize", remeasure);
  }, [active, tabs]);

  const enabled = tabs.filter((tab) => !tab.disabled);

  function onKeyDown(event: React.KeyboardEvent) {
    const step = rovingStep(event, radio);
    if (!step) return;
    const index = enabled.findIndex((tab) => tab.id === active);
    if (index === -1) return;
    event.preventDefault();
    const next = enabled[(index + step + enabled.length) % enabled.length];
    onSelect(next.id);
    refs.current[next.id]?.focus();
  }

  return (
    <div
      role={radio ? "radiogroup" : "tablist"}
      aria-label={label}
      onKeyDown={onKeyDown}
      className="relative flex gap-1"
    >
      {tabs.map((tab) => (
        <button
          key={tab.id}
          ref={(el) => {
            refs.current[tab.id] = el;
          }}
          type="button"
          role={radio ? "radio" : "tab"}
          id={radio ? undefined : `tab-${tab.id}`}
          aria-selected={radio ? undefined : active === tab.id}
          aria-checked={radio ? active === tab.id : undefined}
          aria-controls={
            radio || active !== tab.id ? undefined : `tabpanel-${tab.id}`
          }
          tabIndex={tab.id === active ? 0 : -1}
          disabled={tab.disabled}
          onClick={() => onSelect(tab.id)}
          className={`px-4 py-2 font-mono text-label uppercase tracking-[0.12em] transition-colors ${
            active === tab.id
              ? "text-text"
              : tab.disabled
                ? "cursor-not-allowed text-text-muted/40"
                : "text-text-muted hover:text-text"
          }`}
        >
          {tab.label}
        </button>
      ))}
      <span
        aria-hidden
        className="pointer-events-none absolute -bottom-px h-0.5 bg-action transition-[left,width] duration-300 ease-out"
        style={{ left: indicator.left, width: indicator.width }}
      />
    </div>
  );
}

export function OptionGroup<T extends string>(props: {
  tabs: TabItem<T>[];
  active: T;
  onSelect: (id: T) => void;
  label: string;
}) {
  return <Group {...props} radio />;
}

export function rovingStep(
  event: React.KeyboardEvent,
  vertical: boolean,
): number {
  const forward =
    event.key === "ArrowRight" || (vertical && event.key === "ArrowDown");
  const backward =
    event.key === "ArrowLeft" || (vertical && event.key === "ArrowUp");
  return forward ? 1 : backward ? -1 : 0;
}

export function Tabs<T extends string>({
  tabs,
  active,
  onSelect,
  trailing,
}: {
  tabs: TabItem<T>[];
  active: T;
  onSelect: (id: T) => void;
  trailing?: ReactNode;
}) {
  return (
    <div className="flex flex-wrap items-end justify-between gap-x-4 border-b border-border">
      <Group tabs={tabs} active={active} onSelect={onSelect} />
      {trailing}
    </div>
  );
}
