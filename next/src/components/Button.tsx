import type { ComponentPropsWithRef, ReactNode } from "react";

const buttonSkin =
  "whitespace-nowrap rounded-md font-mono text-label indent-[0.08em] tracking-[0.08em] shadow-[0_2px_14px_transparent] transition duration-200";

export const buttonBase = `inline-flex h-8 items-center px-[1.1rem] ${buttonSkin}`;

export const buttonVariants = {
  primary:
    "bg-action text-action-text not-disabled:hover:bg-action-deep not-disabled:hover:shadow-[0_2px_14px_var(--color-action-glow)]",
  secondary:
    "border border-border bg-surface-2 text-text-muted not-disabled:hover:bg-border not-disabled:hover:text-text not-disabled:hover:shadow-[0_2px_14px_var(--color-action-glow)]",
};

const iconButtonSkin =
  "text-text-muted transition-colors enabled:hover:text-text disabled:cursor-not-allowed disabled:opacity-40";

export const iconButton = `p-1 ${iconButtonSkin}`;

export const stepperButton = `flex items-center self-stretch px-[2px] ${iconButtonSkin}`;

const base = `${buttonBase} justify-center disabled:cursor-not-allowed disabled:opacity-40`;

export function Button({
  variant,
  pulse = false,
  className = "",
  children,
  ...props
}: {
  variant: "primary" | "secondary";
  pulse?: boolean;
  children: ReactNode;
} & ComponentPropsWithRef<"button">) {
  return (
    <button
      type="button"
      className={`${base} ${buttonVariants[variant]} ${
        pulse ? "animate-pulse pointer-events-none" : ""
      } ${className}`}
      {...props}
      aria-disabled={pulse || undefined}
      onClick={pulse ? undefined : props.onClick}
    >
      {children}
    </button>
  );
}
