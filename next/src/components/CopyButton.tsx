"use client";

import { useState } from "react";
import { iconButton } from "@/components/Button";
import { CHECK, StrokeIcon } from "@/components/icons";
import { saveCheck } from "@/components/ui";

export function CopyButton({
  label,
  copiedLabel,
  disabled,
  onCopy,
}: {
  label: string;
  copiedLabel: string;
  disabled?: boolean;
  onCopy: (copied: () => void) => void;
}) {
  const [copied, setCopied] = useState(0);

  return (
    <button
      type="button"
      aria-label={copied > 0 ? copiedLabel : label}
      disabled={disabled}
      onClick={() => onCopy(() => setCopied((tick) => tick + 1))}
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
          className={`${saveCheck} absolute left-1 top-1`}
        />
      )}
    </button>
  );
}
