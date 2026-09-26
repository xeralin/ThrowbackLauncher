"use client";

import { useState } from "react";
import { ConfirmModal } from "@/components/ConfirmModal";
import { useUninstall, type Season } from "@/lib/bridge";
import { showToast } from "@/lib/toast";

export function UninstallModal({
  season,
  hm,
  onClose,
  onDone,
}: {
  season: Season;
  hm: boolean;
  onClose: () => void;
  onDone: () => void;
}) {
  const [busy, setBusy] = useState(false);

  const un = useUninstall({
    onDone: (ok, message) => {
      setBusy(false);
      if (ok) {
        onDone();
        onClose();
      } else {
        showToast(message);
      }
    },
  });

  function confirm() {
    setBusy(true);
    un.run(season.key, hm);
  }

  return (
    <ConfirmModal
      title={`Uninstall ${hm ? "Heated Metal" : season.name}`}
      confirmLabel="Uninstall"
      busyLabel="Removing"
      confirmOnEnter={false}
      busy={busy}
      onConfirm={confirm}
      onCancel={onClose}
    >
      <p className="text-body text-text-muted">
        This permanently removes the installed files.
      </p>
    </ConfirmModal>
  );
}
