"use client";

import { useRef, useState, type ReactNode } from "react";
import {
  ACCENT_HUE_MAX,
  ACCENT_HUE_TRACK,
  ACCENT_STEP,
  accentHex,
  accentParts,
  applyAccent,
} from "@/config/accents";
import { stepperButton } from "@/components/Button";
import { InfoHint } from "@/components/InfoHint";
import {
  StrokeIcon,
  CHECK,
  CHEVRON_LEFT,
  CHEVRON_RIGHT,
} from "@/components/icons";
import { rovingStep } from "@/components/Tabs";
import { inputClasses, heading } from "@/components/ui";

function useEscapeRevert(reset: () => void) {
  const skip = useRef(false);
  const onKeyDown = (event: React.KeyboardEvent<HTMLInputElement>) => {
    if (event.key === "Enter") event.currentTarget.blur();
    else if (event.key === "Escape") {
      event.stopPropagation();
      skip.current = true;
      reset();
      event.currentTarget.blur();
    }
  };
  const guard = (commit: () => void) => () => {
    if (skip.current) skip.current = false;
    else commit();
  };
  return { onKeyDown, guard };
}

function useDraft(value: string) {
  const [draft, setDraft] = useState(value);
  const [prev, setPrev] = useState(value);
  if (prev !== value) {
    setPrev(value);
    setDraft(value);
  }
  return [draft, setDraft] as const;
}

export function Row({
  label,
  hint,
  children,
}: {
  label: string;
  hint?: string;
  children: ReactNode;
}) {
  return (
    <div className="flex min-h-8 items-center justify-between gap-4">
      <span className="flex items-center gap-1.5">
        <span className={heading}>{label}</span>
        {hint && <InfoHint text={hint} />}
      </span>
      {children}
    </div>
  );
}

export function ColorBar({
  label,
  colors,
  value,
  onSelect,
}: {
  label: string;
  colors: string[];
  value: string;
  onSelect: (color: string) => void;
}) {
  const refs = useRef(new Map<string, HTMLButtonElement | null>());
  const selected = colors.includes(value) ? value : colors[0];

  function onKeyDown(event: React.KeyboardEvent) {
    const step = rovingStep(event, true);
    if (!step) return;
    const index = colors.indexOf(selected);
    if (index === -1) return;
    event.preventDefault();
    const next = colors[(index + step + colors.length) % colors.length];
    onSelect(next);
    refs.current.get(next)?.focus();
  }

  return (
    <span
      role="radiogroup"
      aria-label={label}
      onKeyDown={onKeyDown}
      className="flex h-8 overflow-hidden rounded-md border border-border"
    >
      {colors.map((color) => (
        <button
          key={color}
          type="button"
          role="radio"
          aria-label={`${label} ${color}`}
          aria-checked={color === value}
          tabIndex={color === selected ? 0 : -1}
          ref={(el) => {
            refs.current.set(color, el);
          }}
          onClick={() => onSelect(color)}
          style={{ backgroundColor: color }}
          className={`h-full w-6 shrink-0 rounded-none transition focus-visible:-outline-offset-4 ${
            color === value ? "ring-2 ring-inset ring-text" : ""
          }`}
        />
      ))}
    </span>
  );
}

export function SaveCheck({ confirm }: { confirm: number }) {
  const [initial] = useState(confirm);
  if (confirm <= initial) return null;
  return (
    <span
      key={confirm}
      className="pointer-events-none absolute right-2.5 top-1/2 -translate-y-1/2"
    >
      <StrokeIcon
        d={CHECK}
        className="animate-save-check size-4 text-success opacity-0 [&]:stroke-[2.5]"
      />
    </span>
  );
}

export function AccentPicker({
  value,
  onCommit,
}: {
  value: string;
  onCommit: (hex: string) => void;
}) {
  const [draft, setDraft] = useState(() => accentParts(value));
  const [seen, setSeen] = useState(value);
  const [mine, setMine] = useState(value);
  if (seen !== value) {
    setSeen(value);
    if (value !== mine) setDraft(accentParts(value));
  }

  const preview = (next: { hue: number; level: number }) => {
    setDraft(next);
    applyAccent(accentHex(next.hue, next.level));
  };
  const commit = () => {
    const current = accentParts(value);
    if (draft.hue === current.hue && draft.level === current.level) return;
    const hex = accentHex(draft.hue, draft.level);
    setMine(hex);
    onCommit(hex);
  };
  const slider = "color-slider w-full min-w-0";

  return (
    <span className="flex h-full w-full flex-col justify-center gap-6">
      <span className="flex h-6 w-full items-center">
        <input
          type="range"
          min={0}
          max={ACCENT_HUE_MAX}
          step={1}
          value={ACCENT_HUE_MAX - draft.hue}
          aria-label="Accent hue"
          onChange={(event) =>
            preview({
              ...draft,
              hue: ACCENT_HUE_MAX - Number(event.target.value),
            })
          }
          onPointerUp={commit}
          onKeyUp={commit}
          style={{ background: ACCENT_HUE_TRACK }}
          className={slider}
        />
      </span>
      <span className="flex h-6 w-full items-center">
        <input
          type="range"
          min={0}
          max={1}
          step={ACCENT_STEP}
          value={draft.level}
          aria-label="Accent intensity"
          onChange={(event) =>
            preview({ ...draft, level: Number(event.target.value) })
          }
          onPointerUp={commit}
          onKeyUp={commit}
          style={{
            background: `linear-gradient(to right, ${Array.from(
              { length: 5 },
              (_, i) => accentHex(draft.hue, i / 4),
            ).join(", ")})`,
          }}
          className={slider}
        />
      </span>
    </span>
  );
}

function StepIcon({ right }: { right: boolean }) {
  return <StrokeIcon d={right ? CHEVRON_RIGHT : CHEVRON_LEFT} />;
}

export function Stepper({
  label,
  value,
  min,
  max,
  onCommit,
}: {
  label: string;
  value: number;
  min: number;
  max: number;
  onCommit: (value: number) => void;
}) {
  const [draft, setDraft] = useState(String(value));
  const [prev, setPrev] = useState(value);
  const [sent, setSent] = useState<number | null>(null);
  if (prev !== value) {
    setPrev(value);
    if (sent === null) setDraft(String(value));
    else if (sent === value) setSent(null);
  }

  const inputRef = useRef<HTMLInputElement>(null);
  const { onKeyDown, guard } = useEscapeRevert(() =>
    setDraft(String(sent ?? value)),
  );
  const digits = String(max).length;
  const clamp = (next: number) => Math.min(max, Math.max(min, next));
  const step = (delta: number, refocus: boolean) => {
    const next = clamp(current + delta);
    commit(next);
    if (refocus && (next <= min || next >= max)) inputRef.current?.focus();
  };
  const parsed = Number.parseInt(draft, 10);
  const current = Number.isNaN(parsed) ? (sent ?? value) : clamp(parsed);
  const commit = (next: number) => {
    setDraft(String(next));
    if (next !== (sent ?? value)) {
      setSent(next);
      onCommit(next);
    }
  };

  return (
    <div className="flex h-8 items-center rounded-md border border-border bg-surface-2 has-[input:focus]:border-action">
      <button
        type="button"
        aria-label={`Decrease ${label}`}
        disabled={current <= min}
        onClick={(event) => step(-1, event.detail === 0)}
        className={stepperButton}
      >
        <StepIcon right={false} />
      </button>
      <input
        ref={inputRef}
        value={draft}
        inputMode="numeric"
        aria-label={label}
        onChange={(event) =>
          setDraft(event.target.value.replace(/\D/g, "").slice(0, digits))
        }
        onBlur={guard(() => commit(current))}
        onKeyDown={(event) => {
          if (event.key === "ArrowUp" || event.key === "ArrowDown") {
            event.preventDefault();
            step(event.key === "ArrowUp" ? 1 : -1, false);
            return;
          }
          onKeyDown(event);
        }}
        className="w-[3.5ch] bg-transparent text-center font-mono text-ui text-text outline-none"
      />
      <button
        type="button"
        aria-label={`Increase ${label}`}
        disabled={current >= max}
        onClick={(event) => step(1, event.detail === 0)}
        className={stepperButton}
      >
        <StepIcon right={true} />
      </button>
    </div>
  );
}

export function TextSetting({
  value,
  onCommit,
  className,
  maxLength,
  placeholder,
  autoFocus,
  sanitize,
}: {
  value: string;
  onCommit: (value: string) => void;
  className: string;
  maxLength?: number;
  placeholder?: string;
  autoFocus?: boolean;
  sanitize?: (value: string) => string;
}) {
  const [draft, setDraft] = useDraft(value);
  const { onKeyDown, guard } = useEscapeRevert(() => setDraft(value));

  return (
    <input
      value={draft}
      maxLength={maxLength}
      placeholder={placeholder}
      autoFocus={autoFocus}
      onChange={(event) =>
        setDraft(sanitize ? sanitize(event.target.value) : event.target.value)
      }
      onBlur={guard(() => onCommit(draft))}
      onKeyDown={onKeyDown}
      className={`${className} ${inputClasses}`}
    />
  );
}

export function HexSetting({
  label,
  value,
  onCommit,
}: {
  label: string;
  value: string;
  onCommit: (hex: string) => void;
}) {
  const [draft, setDraft] = useDraft(value);
  const { onKeyDown, guard } = useEscapeRevert(() => setDraft(value));

  const commit = () => {
    if (/^#[0-9a-f]{6}$/.test(draft)) {
      if (draft !== value) onCommit(draft);
    } else {
      setDraft(value);
    }
  };

  return (
    <input
      value={draft}
      aria-label={label}
      spellCheck={false}
      onChange={(event) =>
        setDraft(
          `#${event.target.value
            .replace(/[^0-9a-fA-F]/g, "")
            .toLowerCase()
            .slice(0, 6)}`,
        )
      }
      onBlur={guard(commit)}
      onKeyDown={onKeyDown}
      className={`w-21 shrink-0 ${inputClasses}`}
    />
  );
}
