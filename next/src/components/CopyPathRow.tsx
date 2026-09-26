"use client";

import { useState } from "react";
import { iconButton } from "@/components/Button";
import { CHECK, StrokeIcon } from "@/components/icons";
import { ListRow } from "@/components/ui";

export function CopyPathRow({ path }: { path: string }) {
  const [copied, setCopied] = useState(0);

  function copy() {
    navigator.clipboard.writeText(path).then(
      () => setCopied((tick) => tick + 1),
      () => {},
    );
  }

  return (
    <ListRow label={path} title={path}>
      <span className="-mr-1 flex shrink-0 items-center">
        <button
          type="button"
          aria-label={copied > 0 ? "Path copied" : "Copy path"}
          onClick={copy}
          onAnimationEnd={() => setCopied(0)}
          className={`relative ${iconButton}`}
        >
          <StrokeIcon
            d="M11 9h9a2 2 0 0 1 2 2v9a2 2 0 0 1-2 2h-9a2 2 0 0 1-2-2v-9a2 2 0 0 1 2-2ZM5 15H4a2 2 0 0 1-2-2V4a2 2 0 0 1 2-2h9a2 2 0 0 1 2 2v1"
            className={`size-4 transition-opacity duration-200 ${copied > 0 ? "opacity-0" : ""}`}
          />
          {copied > 0 && (
            <StrokeIcon
              key={copied}
              d={CHECK}
              className="animate-save-check absolute left-1 top-1 size-4 text-success opacity-0 [&]:stroke-[2.5]"
            />
          )}
        </button>
      </span>
    </ListRow>
  );
}
