"use client";

import { Modal } from "@/components/Modal";
import {
  formatPlaytime,
  usePlaytime,
  useSeasons,
  type PlaytimeEntry,
  type Season,
} from "@/lib/bridge";

function Stat({ label, value }: { label: string; value: string | number }) {
  return (
    <div className="flex h-[4.125rem] flex-col gap-2 rounded-md border border-border bg-surface-2 px-2 pt-[7px]">
      <span className="whitespace-nowrap font-display text-[min(1.6rem,9vw-10px)] font-bold leading-[1.625rem] text-text text-box-cap">
        {value}
      </span>
      <span className="font-mono text-label leading-none text-text-muted text-box-cap">
        {label}
      </span>
    </div>
  );
}

function Timeline({
  entries,
  seasons,
}: {
  entries: PlaytimeEntry[];
  seasons: Season[];
}) {
  const years = [
    ...new Set(seasons.map((season) => season.code.split("S")[0])),
  ];
  const minutes = new Map<string, number>();
  for (const entry of entries)
    minutes.set(entry.code, (minutes.get(entry.code) ?? 0) + entry.minutes);
  const peak = Math.max(...minutes.values());

  return (
    <div className="flex flex-col gap-1">
      {years.map((year) => (
        <div
          key={year}
          className="grid grid-cols-5 gap-0.5 max-content:grid-cols-4"
        >
          {seasons
            .filter((season) => season.code.startsWith(`${year}S`))
            .map((season) => {
              const played = minutes.get(season.code) ?? 0;
              return (
                <div
                  key={season.key}
                  style={
                    played > 0
                      ? {
                          backgroundColor: `color-mix(in srgb, var(--color-action) ${15 + (played / peak) * 35}%, var(--color-surface-2))`,
                        }
                      : undefined
                  }
                  className={`flex h-5 items-center justify-between gap-1 rounded-sm bg-surface-2 px-1.5 font-mono text-label leading-none ${
                    played > 0 ? "text-text" : "text-text-muted"
                  }`}
                >
                  <span>{season.code}</span>
                  {played > 0 && (
                    <span className="whitespace-nowrap max-content:hidden">
                      {formatPlaytime(played)}
                    </span>
                  )}
                </div>
              );
            })}
        </div>
      ))}
    </div>
  );
}

export function PlaytimeModal({ onClose }: { onClose: () => void }) {
  const entries = usePlaytime() ?? [];
  const seasons = useSeasons();
  if (!seasons) return null;

  const total = entries.reduce((sum, entry) => sum + entry.minutes, 0);
  const sessions = entries.reduce((sum, entry) => sum + entry.sessions, 0);
  const lastPlayed = [...entries].sort(
    (a, b) => b.lastPlayed - a.lastPlayed,
  )[0];
  const firstPlayed = Math.min(...entries.map((entry) => entry.firstPlayed));

  return (
    <Modal wide title="Play time" onClose={onClose}>
      <div className="flex flex-col gap-4">
        <div className="grid grid-cols-4 gap-2 max-content:grid-cols-2">
          <Stat label="All time" value={formatPlaytime(total)} />
          <Stat label="Sessions" value={sessions} />
          <Stat
            label="Last played"
            value={lastPlayed.hm ? `${lastPlayed.code} HM` : lastPlayed.code}
          />
          <Stat
            label="Playing since"
            value={new Date(firstPlayed * 1000).toLocaleDateString("en-US", {
              month: "short",
              day: "numeric",
              year: "numeric",
            })}
          />
        </div>
        <Timeline entries={entries} seasons={seasons} />
      </div>
    </Modal>
  );
}
