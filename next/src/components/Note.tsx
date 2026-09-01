import type { ReactNode } from "react";

const linkClasses =
  "[&_:is(a,button)]:text-notice-link [&_:is(a,button)]:underline [&_:is(a,button):hover]:text-notice-link-hover";

const noticeTone =
  "bg-notice-bg text-notice-text [&_code]:border-notice-border [&_code]:bg-notice-bg-strong [&_code]:text-notice-code";

export function Note({
  children,
  className = "",
}: {
  children: ReactNode;
  className?: string;
}) {
  return (
    <div
      className={`w-fit select-text rounded-r-sm border-l-[3px] border-l-notice-edge px-[0.4rem] py-[0.35rem] text-ui leading-[1.5] ${noticeTone} ${linkClasses} [&_strong]:font-semibold [&_strong]:text-inherit ${className}`}
    >
      {children}
    </div>
  );
}
