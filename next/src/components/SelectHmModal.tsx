"use client";

import { useEffect, useState } from "react";
import { ConfirmModal } from "@/components/ConfirmModal";
import { ExternalLink } from "@/components/ExternalLink";
import { ProtonSteps } from "@/components/SetupSteps";
import { link } from "@/components/ui";
import { site } from "@/config/site";
import { onBridgeEvent, usePlatform } from "@/lib/bridge";

export function SelectHmModal({
  onSelect,
  onSelected,
  onClose,
}: {
  onSelect: () => void;
  onSelected: () => void;
  onClose: () => void;
}) {
  const platform = usePlatform();
  const [busy, setBusy] = useState(false);

  useEffect(
    () =>
      onBridgeEvent("downloader", (event, args) => {
        if (event !== "hm_archive_picked" || !busy) return;
        if (args[0]) onSelected();
        else setBusy(false);
      }),
    [busy, onSelected],
  );

  function select() {
    setBusy(true);
    onSelect();
  }

  return (
    <ConfirmModal
      title="Heated Metal"
      confirmLabel="Select HM"
      busy={busy}
      onConfirm={select}
      onCancel={onClose}
    >
      <div className="flex flex-col gap-3">
        <p className="text-body text-text-muted">
          Download the latest <code>Unstable.7z</code> from{" "}
          <ExternalLink
            href={site.heatedMetalDiscordUrl}
            className={`${link} whitespace-nowrap [&>code]:text-inherit`}
          >
            <code>#indev-releases</code>
          </ExternalLink>
          .
        </p>
        {platform !== "windows" && <ProtonSteps />}
      </div>
    </ConfirmModal>
  );
}
