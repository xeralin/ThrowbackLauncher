export function Switch({
  checked,
  onChange,
  label,
  disabled = false,
}: {
  checked: boolean;
  onChange: (value: boolean) => void;
  label: string;
  disabled?: boolean;
}) {
  const on = `border-action bg-action${disabled ? "" : " group-hover:bg-action-deep"}`;
  const off = `border-border bg-surface-2${disabled ? "" : " group-hover:bg-border"}`;
  const press = disabled ? "" : " group-active:scale-x-[1.25]";
  return (
    <button
      type="button"
      role="switch"
      aria-checked={checked}
      aria-label={label}
      disabled={disabled}
      onClick={() => onChange(!checked)}
      className="group inline-flex items-center disabled:cursor-not-allowed disabled:opacity-40"
    >
      <span
        className={`relative h-[18px] w-[34px] shrink-0 rounded-md border transition-colors duration-200 ${
          checked ? on : off
        }`}
      >
        <span
          className={`absolute left-[3px] top-1/2 size-[10px] -translate-y-1/2 rounded-[3px] shadow-[0_1px_2px_rgba(0,0,0,0.45)] transition-[translate,scale,background-color] duration-200 ease-out${press} ${
            checked
              ? "origin-right translate-x-[16px] bg-action-text"
              : "origin-left translate-x-0 bg-text"
          }`}
        />
      </span>
    </button>
  );
}
