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
      aria-hidden="true"
      focusable="false"
    >
      {d ? <path d={d} /> : children}
    </svg>
  );
}

export const THROWBACK_MARK =
  "M18.17 20.21c-2.26 0.06 -7.41 -0.13 -12.66 -3.21c-0.78 -0.46 -1.88 -1.16 -3.11 -2.19l-0.42 -0.35l0.09 -0.53c0.15 -0.95 0.44 -1.86 0.87 -2.71c-0.48 0.14 -0.96 0.28 -1.44 0.42c0.41 -0.8 1.03 -1.78 1.96 -2.74c0.39 -0.4 0.77 -0.74 1.14 -1.03c-0.43 -0.07 -0.86 -0.15 -1.3 -0.22c0.94 -0.87 1.81 -1.4 2.43 -1.72c1.06 -0.55 2.12 -0.87 2.64 -1.02c0.14 -0.04 0.26 -0.08 0.34 -0.1c-0.33 -0.34 -0.66 -0.67 -0.99 -1.01c0.91 0.07 1.86 0.17 2.83 0.29c1.1 0.14 2.16 0.29 3.16 0.47c0.81 0.4 1.62 0.79 2.43 1.19l0.14 0.24s0.08 0.13 0.08 0.14c0.91 0.18 1.81 0.44 2.65 0.76c1.68 0.65 2.26 1.21 2.6 1.67c0.27 0.37 0.48 0.78 0.63 1.23l0.16 0.66c0.03 0.22 0.06 0.43 0.1 0.65c-0.09 0.01 -2.3 0.17 -2.39 0.18c-0.03 0 -1.3 0.1 -1.33 0.1c-0.37 -0.02 -0.75 -0.03 -1.13 -0.04c-0.3 -0.01 -0.61 -0.01 -0.92 -0.01h-0.01l-0.87 0.55c-0.33 0.45 -0.47 0.87 -0.53 1.18c-0.28 1.46 0.68 3.1 2.38 4.23c0.59 0.37 1.18 0.74 1.77 1.11c-0.2 0.06 -0.42 0.15 -0.66 0.26c-0.15 0.07 -0.28 0.14 -0.4 0.21c0.66 0.4 1.31 0.8 1.97 1.2c-0.49 0.06 -1.26 0.14 -2.21 0.16Z";

export const HEATED_METAL_MARK =
  "M12.93 2.09 16.24 2.09 19.31 7.69 17.38 10.56ZM8.0 13.28 12.97 4.85 15.01 8.43 11.98 13.56ZM13.48 12.69 15.45 9.3 20.33 17.97 16.16 17.97ZM1.5 16.47 4.69 11.03 8.36 11.15 3.47 19.55ZM7.41 14.38 13.28 14.42 15.09 18.01 5.32 17.93ZM13.16 18.84 22.5 18.84 21.04 21.91 14.62 21.91Z";

export function MarkIcon({
  d,
  className = "size-4",
}: {
  d: string;
  className?: string;
}) {
  return (
    <svg
      viewBox="0 0 24 24"
      fill="currentColor"
      className={className}
      aria-hidden="true"
      focusable="false"
    >
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

export function TerminalIcon() {
  return <StrokeIcon d="m4 17 6-6-6-6M12 19h8" />;
}

const BOOKMARK = "M19 20l-7-4.6L5 20V5a2 2 0 0 1 2-2h10a2 2 0 0 1 2 2z";

export function BookmarkIcon({ filled = false }: { filled?: boolean }) {
  return <StrokeIcon d={BOOKMARK} fill={filled ? "currentColor" : "none"} />;
}

export function DefaultLibraryIcon() {
  return (
    <span
      role="img"
      aria-label="Default library"
      className="shrink-0 p-1 text-text-muted"
    >
      <BookmarkIcon filled />
    </span>
  );
}
