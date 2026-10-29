"use client";

import { useEffect, useState } from "react";
import { ConfirmModal } from "@/components/ConfirmModal";
import { ExternalLink } from "@/components/ExternalLink";
import { SelectHmModal } from "@/components/SelectHmModal";
import { ExclusionSteps } from "@/components/SetupSteps";
import { link } from "@/components/ui";
import { site } from "@/config/site";
import {
  onBridgeEvent,
  useDownloader,
  useLaunch,
  usePlatform,
  useSeasons,
} from "@/lib/bridge";

type MissingFiles = { key: string; library: string; hm: boolean };

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

  const name = missing.hm ? (
    "Heated Metal"
  ) : (
    <ExternalLink href={site.throwbackLoaderRepoUrl} className={link}>
      ThrowbackLoader
    </ExternalLink>
  );

  const verify = () => {
    onClose();
    if (!missing.hm) {
      dl.verify(missing.key, false);
      return;
    }
    lc.installs(missing.key, (installs) => {
      if (installs.hm.installed) dl.restoreHm(missing.key);
      else dl.enqueue(missing.key, true, "");
    });
  };

  if (selectHm) return <SelectHmModal onSelected={verify} onClose={onClose} />;

  return (
    <ConfirmModal
      title="Files are missing"
      confirmLabel="Verify"
      onConfirm={() => {
        if (
          !missing.hm ||
          !seasons?.find((season) => season.key === missing.key)?.hmBeta
        )
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
            Windows Security may have removed {name} files.
          </p>
          <ExclusionSteps library={missing.library} />
        </div>
      ) : (
        <p className="text-body text-text-muted">
          {name} files were not found.
        </p>
      )}
    </ConfirmModal>
  );
}

export function FilesMissingModal() {
  const [missing, setMissing] = useState<MissingFiles | null>(null);

  useEffect(() => {
    const show = (event: string, args: unknown[]) => {
      if (event === "files_missing")
        setMissing({
          key: args[0] as string,
          hm: args[1] as boolean,
          library: args[2] as string,
        });
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
