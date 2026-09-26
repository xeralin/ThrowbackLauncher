"use client";

import { LogBox, type LogLine } from "@/components/LogBox";
import { NyanCat } from "@/components/NyanCat";
import { determinatePercent, useDownloadProgress } from "@/lib/bridge";

export function TransferBar({
  active,
  state,
}: {
  active: boolean;
  state: string;
}) {
  const { progress, step, steps } = useDownloadProgress();
  if (!active) return null;

  const paused = state === "paused";
  const percent =
    state === "downloading" || paused
      ? determinatePercent(progress, step, steps)
      : null;

  return (
    <>
      <div className="h-3.5 min-w-0 flex-1 rounded-full border border-border bg-well">
        <div
          key={percent === null ? "indeterminate" : "determinate"}
          data-paused={paused || undefined}
          className={`transfer-fill h-full min-w-3 rounded-full ${
            percent === null
              ? "w-full"
              : "transition-[width] duration-200 ease-out"
          }`}
          style={percent === null ? undefined : { width: `${progress}%` }}
        >
          <NyanCat />
        </div>
      </div>
      {percent !== null && (
        <span className="block translate-y-[1px] font-display text-[1.2rem] font-bold leading-none tabular-nums text-text">
          {percent}%
        </span>
      )}
    </>
  );
}

export function TransferPanel({
  lines,
  active,
  state,
}: {
  lines: LogLine[];
  active: boolean;
  state: string;
}) {
  if (!lines.length || (!active && state !== "failed")) return null;

  return (
    <div className="mt-3 flex min-h-0 flex-1 flex-col">
      <LogBox lines={lines} className="min-h-24 flex-1" />
    </div>
  );
}
