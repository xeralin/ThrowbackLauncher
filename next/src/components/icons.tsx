import type { ReactNode } from "react";

export function StrokeIcon({
  d,
  className = "size-4",
  fill = "none",
  children,
}: {
  d?: string;
  className?: string;
  fill?: string;
  children?: ReactNode;
}) {
  return (
    <svg
      viewBox="0 0 24 24"
      fill={fill}
      stroke="currentColor"
      strokeWidth={2}
      strokeLinecap="round"
      strokeLinejoin="round"
      className={className}
    >
      {d ? <path d={d} /> : children}
    </svg>
  );
}

export function MarkIcon({ d, className }: { d: string; className: string }) {
  return (
    <svg viewBox="0 0 24 24" fill="currentColor" className={className}>
      <path d={d} />
    </svg>
  );
}

export function FolderIcon() {
  return (
    <StrokeIcon d="M20 20a2 2 0 0 0 2-2V8a2 2 0 0 0-2-2h-7.9a2 2 0 0 1-1.69-.9L9.6 3.9A2 2 0 0 0 7.93 3H4a2 2 0 0 0-2 2v13a2 2 0 0 0 2 2Z" />
  );
}

export function RemoveIcon() {
  return (
    <StrokeIcon d="M5 3h14l-1.4 15.2a2 2 0 0 1-2 1.8H8.4a2 2 0 0 1-2-1.8Z" />
  );
}

export const CHEVRON_RIGHT = "m9 6 6 6-6 6";
export const CHEVRON_LEFT = "m15 6-6 6 6 6";
export const CHEVRON_DOWN = "m6 9 6 6 6-6";
export const CHECK = "M5 13l4 4L19 7";
export const CLOCK = "M22 12a10 10 0 1 1-20 0 10 10 0 0 1 20 0zM12 6v6l4 2";
export const COPY =
  "M11 9h9a2 2 0 0 1 2 2v9a2 2 0 0 1-2 2h-9a2 2 0 0 1-2-2v-9a2 2 0 0 1 2-2ZM5 15H4a2 2 0 0 1-2-2V4a2 2 0 0 1 2-2h9a2 2 0 0 1 2 2v1";
export const PROTON = "m4 17 6-6-6-6M12 19h8";
export const REFRESH =
  "M3 12a9 9 0 0 1 9-9 9.75 9.75 0 0 1 6.74 2.74L21 8M21 3v5h-5M21 12a9 9 0 0 1-9 9 9.75 9.75 0 0 1-6.74-2.74L3 16M8 16H3v5";
export const INFO =
  "M22 12a10 10 0 1 1-20 0 10 10 0 0 1 20 0zM12 16v-4M12 8h.01";
export const GITHUB =
  "M12 .297c-6.63 0-12 5.373-12 12 0 5.303 3.438 9.8 8.205 11.385.6.113.82-.258.82-.577 0-.285-.01-1.04-.015-2.04-3.338.724-4.042-1.61-4.042-1.61C4.422 18.07 3.633 17.7 3.633 17.7c-1.087-.744.084-.729.084-.729 1.205.084 1.838 1.236 1.838 1.236 1.07 1.835 2.809 1.305 3.495.998.108-.776.417-1.305.76-1.605-2.665-.3-5.466-1.332-5.466-5.93 0-1.31.465-2.38 1.235-3.22-.135-.303-.54-1.523.105-3.176 0 0 1.005-.322 3.3 1.23.96-.267 1.98-.399 3-.405 1.02.006 2.04.138 3 .405 2.28-1.552 3.285-1.23 3.285-1.23.645 1.653.24 2.873.12 3.176.765.84 1.23 1.91 1.23 3.22 0 4.61-2.805 5.625-5.475 5.92.42.36.81 1.096.81 2.22 0 1.606-.015 2.896-.015 3.286 0 .315.21.69.825.57C20.565 22.092 24 17.592 24 12.297c0-6.627-5.373-12-12-12";

const BOOKMARK = "M19 20l-7-4.6L5 20V5a2 2 0 0 1 2-2h10a2 2 0 0 1 2 2z";

export function BookmarkIcon({ filled = false }: { filled?: boolean }) {
  return <StrokeIcon d={BOOKMARK} fill={filled ? "currentColor" : "none"} />;
}

export function UnlockIcon({ className }: { className: string }) {
  return (
    <StrokeIcon className={className}>
      <rect x="3" y="11" width="13" height="10" rx="1.5" fill="currentColor" />
      <path d="M12.5 10V6a3.75 3.75 0 0 1 7.5 0v2" strokeWidth={2.5} />
    </StrokeIcon>
  );
}

const TRIANGLE =
  "M21.73 18l-8-14a2 2 0 0 0-3.48 0l-8 14A2 2 0 0 0 4 21h16a2 2 0 0 0 1.73-3z";

export function WarningIcon({ className }: { className: string }) {
  return (
    <StrokeIcon className={className}>
      <path d={TRIANGLE} />
      <path
        d={`${TRIANGLE}M11 9a1 1 0 0 1 2 0v4a1 1 0 0 1-2 0zM13 17a1 1 0 0 1-2 0 1 1 0 0 1 2 0z`}
        fill="currentColor"
        fillRule="evenodd"
        stroke="none"
      />
    </StrokeIcon>
  );
}

export function DefaultLibraryIcon() {
  return (
    <span className="shrink-0 p-1 text-text-muted">
      <BookmarkIcon filled />
    </span>
  );
}
