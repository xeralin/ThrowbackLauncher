export function StrokeIcon({
  d,
  className = "size-4",
  fill = "none",
}: {
  d: string;
  className?: string;
  fill?: string;
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
