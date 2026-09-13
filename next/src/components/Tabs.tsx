"use client";

import {
  createContext,
  useContext,
  useLayoutEffect,
  useRef,
  useState,
  type ReactNode,
} from "react";

export type TabItem<T extends string> = {
  id: T;
  label: string;
  icon?: ReactNode;
  disabled?: boolean;
};

const CompactContext = createContext(false);

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
  const compact = useContext(CompactContext);
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
  }, [active, tabs, compact]);

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
          aria-label={compact && tab.icon ? tab.label : undefined}
          className={`py-2 font-mono text-label uppercase tracking-[0.12em] transition-colors ${
            compact && tab.icon ? "px-3" : "px-4"
          } ${
            active === tab.id
              ? "text-text"
              : tab.disabled
                ? "cursor-not-allowed text-text-muted/40"
                : "text-text-muted hover:text-text"
          }`}
        >
          {compact && tab.icon ? tab.icon : tab.label}
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
  const wrapRef = useRef<HTMLDivElement>(null);
  const groupRef = useRef<HTMLDivElement>(null);
  const trailingRef = useRef<HTMLDivElement>(null);
  const expanded = useRef({ group: 0, trailing: 0 });
  const compactTrailing = useRef(0);
  const [compact, setCompact] = useState(false);

  useLayoutEffect(() => {
    const wrap = wrapRef.current;
    const group = groupRef.current;
    if (!wrap || !group) return;
    if (compact)
      compactTrailing.current = trailingRef.current?.offsetWidth ?? 0;
    function measure() {
      if (!wrap || !group) return;
      const live = trailingRef.current?.offsetWidth ?? 0;
      if (!compact)
        expanded.current = { group: group.offsetWidth, trailing: live };
      const trailing = compact
        ? expanded.current.trailing + live - compactTrailing.current
        : live;
      const rest = trailingRef.current
        ? parseFloat(getComputedStyle(wrap).columnGap) + trailing
        : 0;
      setCompact(expanded.current.group + rest > wrap.clientWidth);
    }
    measure();
    const observer = new ResizeObserver(measure);
    observer.observe(wrap);
    if (trailingRef.current) observer.observe(trailingRef.current);
    return () => observer.disconnect();
  }, [compact]);

  return (
    <CompactContext value={compact}>
      <div
        ref={wrapRef}
        className="flex flex-wrap-reverse items-start gap-x-4 border-b border-border"
      >
        <div ref={groupRef}>
          <Group tabs={tabs} active={active} onSelect={onSelect} />
        </div>
        {trailing && (
          <div
            ref={trailingRef}
            className="ml-auto flex items-center self-stretch"
          >
            {trailing}
          </div>
        )}
      </div>
    </CompactContext>
  );
}
