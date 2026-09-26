"use client";

import { useEffect, useState } from "react";
import { ConfirmModal } from "@/components/ConfirmModal";
import { ExclusionStepList } from "@/components/SetupSteps";
import { onBridgeEvent } from "@/lib/bridge";

export function LiberatorMissingModal({
  folder,
  onRestore,
  onClose,
}: {
  folder: string;
  onRestore: () => void;
  onClose: () => void;
}) {
  const [busy, setBusy] = useState(false);

  useEffect(
    () =>
      onBridgeEvent("liberator", (event) => {
        if (event === "restored") setBusy(false);
      }),
    [],
  );

  function confirm() {
    setBusy(true);
    onRestore();
  }

  return (
    <ConfirmModal
      title="Liberator is missing"
      confirmLabel="Download"
      busyLabel="Downloading"
      busy={busy}
      onConfirm={confirm}
      onCancel={onClose}
    >
      <div className="flex flex-col gap-3">
        <p className="text-body text-text-muted">
          Windows Security may have removed <code>Liberator.exe</code>.
        </p>
        <ExclusionStepList paths={[folder]} />
      </div>
    </ConfirmModal>
  );
}
