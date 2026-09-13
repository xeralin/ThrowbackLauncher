import type { ReactNode } from "react";
import { InfoHint } from "@/components/InfoHint";

export const panel = "rounded-lg border border-border bg-surface";

export const iconBox =
  "flex shrink-0 items-center rounded-lg border border-border p-[0.2rem] pr-[0.075rem]";

export const card = `${panel} flex flex-col gap-4 p-4`;

const fieldRow =
  "flex min-h-8 items-center justify-between gap-3 rounded-md border bg-surface-2 px-2";

export const inputClasses =
  "h-8 min-w-0 rounded-md border border-border bg-surface-2 px-[0.4rem] pt-[1px] font-mono text-ui text-text outline-none placeholder:text-text-muted focus:border-action";

export const microLabel = "font-mono uppercase tracking-[0.2em]";

export const heading = "font-display text-[1.05rem] font-bold text-text";

export function BlinkCursor() {
  return (
    <span aria-hidden className="ml-px inline-block animate-blink">
      _
    </span>
  );
}

function RowLabel({
  label,
  title,
  strike,
  fill = true,
}: {
  label: string;
  title?: string;
  strike: boolean;
  fill?: boolean;
}) {
  return (
    <span
      title={title}
      className={`min-w-0 font-mono text-label ${fill ? "grow truncate-fade" : ""} ${
        strike ? "text-text-muted line-through" : "text-text"
      }`}
    >
      {label}
    </span>
  );
}

export function ListRow({
  label,
  title,
  strike = false,
  hint,
  children,
}: {
  label: string;
  title?: string;
  strike?: boolean;
  hint?: string;
  children: ReactNode;
}) {
  return (
    <div className={`${fieldRow} border-border`}>
      {hint ? (
        <span className="flex min-w-0 grow items-center gap-1.5">
          <RowLabel label={label} title={title} strike={strike} fill={false} />
          <InfoHint text={hint} />
        </span>
      ) : (
        <RowLabel label={label} title={title} strike={strike} />
      )}
      {children}
    </div>
  );
}

export function PickerRow({
  label,
  title,
  selected,
  disabled,
  strike = false,
  onSelect,
  children,
}: {
  label: string;
  title?: string;
  selected: boolean;
  disabled?: boolean;
  strike?: boolean;
  onSelect: () => void;
  children?: ReactNode;
}) {
  return (
    <button
      type="button"
      disabled={disabled}
      onMouseDown={(event) => event.preventDefault()}
      onClick={onSelect}
      className={`${fieldRow} text-left transition ${
        selected
          ? "row-selected"
          : disabled
            ? "border-border cursor-not-allowed opacity-40"
            : "border-border hover:border-action-edge"
      }`}
    >
      <RowLabel label={label} title={title} strike={strike} />
      {children}
    </button>
  );
}
