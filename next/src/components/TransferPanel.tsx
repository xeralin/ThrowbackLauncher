"use client";

import { LogBox, type LogLine } from "@/components/LogBox";
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
        className="h-3.5 min-w-0 flex-1 rounded-full border border-border bg-well"
        onClick={(event) => settings && toggleNyan(event, settings)}
      >
        <div
          data-paused={paused || undefined}
          className="transfer-fill h-full min-w-3 rounded-full transition-[width] duration-200 ease-out"
          style={{ width: `${progress}%` }}
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

export function TransferPanel({
  lines,
  state,
}: {
  lines: LogLine[];
  state: string;
}) {
  if (!lines.length || state === "idle") return null;

  return (
    <div className="mt-3 flex min-h-0 flex-1 flex-col">
      <LogBox lines={lines} />
    </div>
  );
}
