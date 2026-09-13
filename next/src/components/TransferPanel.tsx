"use client";

import { LogBox, type LogLine } from "@/components/LogBox";
import { determinatePercent, useDownloadProgress } from "@/lib/bridge";
import { heading } from "@/components/ui";

function percentValue(
  progress: number,
  step: number,
  steps: number,
  state: string,
): number | null {
  if (state !== "downloading" && state !== "paused") return null;
  return determinatePercent(progress, step, steps);
}

export function TransferPercent({ state }: { state: string }) {
  const { progress, step, steps } = useDownloadProgress();
  const percent = percentValue(progress, step, steps, state);
  if (percent === null) return null;
  return (
    <span
      className={`block translate-y-[0.5px] ${heading} leading-none tabular-nums`}
    >
      {percent}%
    </span>
  );
}

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
  const percent = percentValue(progress, step, steps, state);

  return (
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
      />
    </div>
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
