"use client";

import { useEffect, useState } from "react";
import { ConfirmModal } from "@/components/ConfirmModal";
import { SelectHmModal } from "@/components/SelectHmModal";
import { ExclusionSteps } from "@/components/SetupSteps";
import {
  onBridgeEvent,
  useDownloader,
  useLaunch,
  usePlatform,
  useSeasons,
} from "@/lib/bridge";

type MissingFiles = { key: string; library: string };

function FilesMissing({
  missing,
  onClose,
}: {
  missing: MissingFiles;
  onClose: () => void;
}) {
  const [selectHm, setSelectHm] = useState(false);
  const seasons = useSeasons();
  const dl = useDownloader();
  const lc = useLaunch();
  const platform = usePlatform();

  const verify = () => {
    onClose();
    lc.installs(missing.key, (installs) => {
      if (installs.hm.installed) dl.restoreHm(missing.key);
      else dl.enqueue(missing.key, true, "");
    });
  };

  if (selectHm)
    return (
      <SelectHmModal
        onSelect={dl.pickHmArchive}
        onSelected={verify}
        onClose={onClose}
      />
    );

  return (
    <ConfirmModal
      title="Files are missing"
      confirmLabel="Verify"
      onConfirm={() => {
        if (!seasons?.find((season) => season.key === missing.key)?.hmBeta)
          verify();
        else
          dl.hmArchiveCached((cached) =>
            cached ? verify() : setSelectHm(true),
          );
      }}
      onCancel={onClose}
    >
      {platform === "windows" ? (
        <div className="flex flex-col gap-3">
          <p className="text-body text-text-muted">
            Windows Security may have removed Heated Metal files.
          </p>
          <ExclusionSteps library={missing.library} />
        </div>
      ) : (
        <p className="text-body text-text-muted">
          Heated Metal files were not found.
        </p>
      )}
    </ConfirmModal>
  );
}

export function HmFilesMissingModal() {
  const [missing, setMissing] = useState<MissingFiles | null>(null);

  useEffect(() => {
    const show = (event: string, args: unknown[]) => {
      if (event === "hm_files_missing")
        setMissing({ key: args[0] as string, library: args[1] as string });
    };
    const offs = [
      onBridgeEvent("launch", show),
      onBridgeEvent("downloader", show),
    ];
    return () => offs.forEach((off) => off());
  }, []);

  return (
    missing && (
      <FilesMissing
        key={missing.key}
        missing={missing}
        onClose={() => setMissing(null)}
      />
    )
  );
}
