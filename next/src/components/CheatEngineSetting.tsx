"use client";

import { Switch } from "@/components/Switch";
import { ListRow } from "@/components/ui";
import { useCheatEngine, type CheatEngineStatus } from "@/lib/bridge";
import { showToast } from "@/lib/toast";

export function CheatEngineSetting({
  seasonKey,
  status,
  onStatus,
}: {
  seasonKey: string;
  status: CheatEngineStatus;
  onStatus: (status: CheatEngineStatus) => void;
}) {
  const ce = useCheatEngine({
    onDone: () => {
      ce.status(seasonKey, onStatus);
    },
  });

  function settle(error: string) {
    if (error) showToast(error);
    ce.status(seasonKey, onStatus);
  }

  function toggle(on: boolean) {
    if (!on) ce.remove(seasonKey, settle);
    else if (status.present) ce.add(seasonKey, settle);
    else
      ce.pickInstaller((path) => {
        if (!path) return;
        ce.install(seasonKey);
        ce.status(seasonKey, onStatus);
      });
  }

  return (
    <ListRow label="Cheat Engine">
      <span className={`flex${status.busy ? " animate-pulse" : ""}`}>
        <Switch
          label="Open Cheat Engine alongside the game"
          checked={status.enabled && status.present}
          disabled={status.busy}
          onChange={toggle}
        />
      </span>
    </ListRow>
  );
}
