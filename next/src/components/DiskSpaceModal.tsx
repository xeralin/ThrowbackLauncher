"use client";

import { useState } from "react";
import { ConfirmModal } from "@/components/ConfirmModal";
import { useDownloader } from "@/lib/bridge";

export function DiskSpaceModal() {
  const [override, setOverride] = useState<number | null | undefined>(
    undefined,
  );

  const dl = useDownloader({
    onDiskSpace: setOverride,
  });

  const shortfall =
    override === undefined ? dl.diskShortfall || null : override;

  function cancel() {
    dl.cancel();
    setOverride(null);
  }

  function proceed() {
    setOverride(null);
    dl.confirmDiskSpace();
  }

  if (shortfall === null) return null;

  return (
    <ConfirmModal
      title="Not enough disk space"
      confirmLabel="Download anyway"
      confirmOnEnter={false}
      onConfirm={proceed}
      onCancel={cancel}
    >
      <p className="text-body text-text-muted">
        Free up <code>{shortfall} GB</code> with Shears or pick a different
        library.
      </p>
    </ConfirmModal>
  );
}
