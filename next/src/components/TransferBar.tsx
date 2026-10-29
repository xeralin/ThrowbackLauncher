"use client";

import type { CSSProperties } from "react";
import { NyanCat, toggleNyan } from "@/components/NyanCat";
import {
  determinatePercent,
  useDownloadProgress,
  useSettings,
} from "@/lib/bridge";

export function TransferBar({ state }: { state: string }) {
  const { progress, step, steps } = useDownloadProgress();
  const settings = useSettings();
  const paused = state === "paused";
  const percent =
    state === "downloading" || paused
      ? determinatePercent(progress, step, steps)
      : null;
  if (percent === null) return null;

  return (
    <>
      <div
        className="h-3.75 min-w-0 flex-1 rounded-full border border-border bg-well"
        style={{ "--progress": `${progress}%` } as CSSProperties}
        onClick={(event) => settings && toggleNyan(event, settings)}
      >
        <div
          data-paused={paused || undefined}
          className="transfer-fill h-full w-(--progress) min-w-3 rounded-full transition-[width] duration-200 ease-out"
        >
          <NyanCat />
        </div>
      </div>
      <span className="block translate-y-[1px] font-display text-[1.2rem] font-bold leading-none tabular-nums text-text">
        {percent}%
      </span>
    </>
  );
}
