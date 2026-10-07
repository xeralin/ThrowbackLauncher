"use client";

import { useEffect, useRef } from "react";

export type LogLine = { id: number; text: string };

export function LogBox({ lines }: { lines: LogLine[] }) {
  const ref = useRef<HTMLDivElement>(null);
  const follow = useRef(true);
  const pinnedTop = useRef(-1);

  useEffect(() => {
    const el = ref.current;
    if (!el) return;
    if (el.scrollHeight <= el.clientHeight) follow.current = true;
    if (follow.current) {
      el.scrollTop = el.scrollHeight;
      pinnedTop.current = el.scrollTop;
    }
  }, [lines]);

  return (
    <div
      ref={ref}
      onScroll={(event) => {
        const el = event.currentTarget;
        if (el.scrollTop === pinnedTop.current) {
          follow.current = true;
          return;
        }
        follow.current = el.scrollHeight - el.scrollTop - el.clientHeight < 4;
      }}
      className="min-h-24 flex-1 select-text overflow-auto [overflow-anchor:none] rounded-lg border border-border bg-well p-3 font-mono text-label leading-[1.5] text-text-muted"
    >
      {lines.map((line) => (
        <div key={line.id} className="whitespace-pre-wrap break-words">
          {line.text}
        </div>
      ))}
    </div>
  );
}
