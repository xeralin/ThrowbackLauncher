"use client";

import { useState } from "react";
import { iconButton } from "@/components/Button";
import { Modal } from "@/components/Modal";
import { Note } from "@/components/Note";
import { ListRow, link } from "@/components/ui";
import { RemoveIcon, StrokeIcon } from "@/components/icons";
import { useCheatEngine, type CheatEngineSeason } from "@/lib/bridge";
import { showToast } from "@/lib/toast";

export function CheatEngineInstaller() {
  const [seasons, setSeasons] = useState<CheatEngineSeason[] | null>(null);
  const [busy, setBusy] = useState(false);
  const ce = useCheatEngine({
    onDone: (ok, message) => {
      setBusy(false);
      if (!ok) showToast(message);
    },
  });

  function open() {
    ce.seasons((list) => {
      if (list.length === 0) showToast("Download a season first");
      else setSeasons(list);
    });
  }

  function add(season: CheatEngineSeason) {
    if (season.present) {
      ce.add(season.key, (result) => {
        if (!result.ok) showToast(result.message);
        ce.seasons(setSeasons);
      });
      return;
    }
    ce.pickInstaller((path) => {
      if (!path) return;
      setSeasons(null);
      setBusy(true);
      ce.install(season.key);
    });
  }

  function remove(key: string) {
    ce.remove(key, (result) => {
      if (!result.ok) showToast(result.message);
      ce.seasons(setSeasons);
    });
  }

  return (
    <>
      <button
        type="button"
        disabled={!ce.ready}
        aria-disabled={busy || undefined}
        onClick={busy ? undefined : open}
        className={`${link} disabled:cursor-not-allowed disabled:no-underline ${busy ? "pointer-events-none animate-pulse" : "disabled:opacity-40"}`}
      >
        Set up Cheat Engine
      </button>

      {seasons && (
        <Modal
          title="Cheat Engine"
          onClose={() => setSeasons(null)}
          footer={
            <Note className="mr-auto">
              Deny any bundled offers to avoid adware.
            </Note>
          }
        >
          <div className="flex flex-col gap-2">
            {seasons.map((season) => (
              <ListRow key={season.key} label={season.label}>
                {season.hasCe ? (
                  <button
                    type="button"
                    aria-label={`Remove Cheat Engine from ${season.label}`}
                    onClick={() => remove(season.key)}
                    className={`-mr-1 shrink-0 ${iconButton}`}
                  >
                    <RemoveIcon />
                  </button>
                ) : (
                  <button
                    type="button"
                    aria-label={`Add Cheat Engine to ${season.label}`}
                    onClick={() => add(season)}
                    className={`-mr-1 shrink-0 ${iconButton}`}
                  >
                    <StrokeIcon d="M12 5v14M5 12h14" />
                  </button>
                )}
              </ListRow>
            ))}
          </div>
        </Modal>
      )}
    </>
  );
}
