"use client";

import { useEffect, useMemo, useRef, useState } from "react";
import { createPortal } from "react-dom";
import { buttonBase, buttonVariants, iconButton } from "@/components/Button";
import { card, heading, iconBox, ListRow } from "@/components/ui";
import { Note } from "@/components/Note";
import { ExternalLink } from "@/components/ExternalLink";
import { VersionChip } from "@/components/VersionChip";
import { Modal } from "@/components/Modal";
import { StrokeIcon } from "@/components/icons";
import {
  useDownloader,
  useUpdate,
  type InstalledComponent,
  type UpdateComponent,
} from "@/lib/bridge";
import { Markdown } from "@/components/Markdown";
import { parseReleaseNotes } from "@/lib/release-notes";
import { dismissToast, RATE_LIMIT_TOAST, showToast } from "@/lib/toast";
import { useTopbarSlot } from "@/lib/topbar-slot";
import { site } from "@/config/site";

const UPDATE_TOAST = "update";

function UpdateCard({
  component,
  applying,
  progress,
  disabled,
  onApply,
}: {
  component: UpdateComponent;
  applying: boolean;
  progress: number;
  disabled: boolean;
  onApply: () => void;
}) {
  const notes = useMemo(
    () => parseReleaseNotes(component.body, component.repository),
    [component.body, component.repository],
  );
  return (
    <div className={card}>
      <div className="flex flex-wrap items-center gap-x-4 gap-y-2">
        <span className="flex min-w-0 items-center gap-2.5">
          <span className={`truncate ${heading}`}>{component.name}</span>
          <VersionChip version={component.target} className="shrink-0" />
        </span>
        <button
          type="button"
          aria-disabled={applying || undefined}
          disabled={!applying && disabled}
          onClick={applying ? undefined : onApply}
          className={`${buttonBase} ml-auto shrink-0 justify-center ${
            applying
              ? "relative overflow-hidden bg-[color-mix(in_srgb,var(--color-action)_45%,black)] text-action-text"
              : `disabled:cursor-not-allowed disabled:opacity-40 ${buttonVariants.primary}`
          }`}
        >
          {applying ? (
            <>
              <span
                aria-hidden
                className="absolute inset-0 origin-left bg-action transition-transform duration-200"
                style={{ transform: `scaleX(${progress / 100})` }}
              />
              <span className="relative">Update</span>
            </>
          ) : (
            "Update"
          )}
        </button>
      </div>
      {notes.truncated ? (
        <>
          <div className="cut-fade -mb-2 pb-4">
            <Markdown nodes={notes.nodes} />
          </div>
          <Note>
            Read the{" "}
            <ExternalLink href={component.url}>full release notes</ExternalLink>
          </Note>
        </>
      ) : (
        notes.nodes.length > 0 && <Markdown nodes={notes.nodes} />
      )}
    </div>
  );
}

export default function UpdatesPage() {
  const downloading = useDownloader().running;
  const update = useUpdate();
  const refreshRef = useRef<HTMLButtonElement>(null);
  const [spins, setSpins] = useState(0);
  const [installed, setInstalled] = useState<InstalledComponent[] | null>(null);
  const prevComponents = useRef(0);

  useEffect(() => {
    const shrank = update.components.length < prevComponents.current;
    prevComponents.current = update.components.length;
    if (shrank && document.activeElement === document.body)
      refreshRef.current?.focus();
  }, [update.components]);
  const slot = useTopbarSlot();

  const wasChecking = useRef(false);
  const manualCheck = useRef(false);
  useEffect(() => {
    if (wasChecking.current && !update.checking) {
      const wasManual = manualCheck.current;
      manualCheck.current = false;
      if (update.checkError === "rate_limit") {
        showToast(update.checkErrorDetail, {
          key: RATE_LIMIT_TOAST,
        });
      } else if (update.checkError === "error") {
        showToast("Update check failed", {
          key: UPDATE_TOAST,
        });
      } else if (update.components.length > 0) {
        dismissToast(UPDATE_TOAST);
      } else if (wasManual) {
        showToast("Everything is up to date", {
          key: UPDATE_TOAST,
        });
      }
    }
    wasChecking.current = update.checking;
  }, [
    update.checking,
    update.components.length,
    update.checkError,
    update.checkErrorDetail,
  ]);

  return (
    <>
      {slot &&
        createPortal(
          <span className={`${iconBox} bg-surface-2`}>
            <button
              type="button"
              aria-label="Installed versions"
              onClick={() => {
                update.installed(setInstalled);
              }}
              className={iconButton}
            >
              <StrokeIcon d="M11 21.73a2 2 0 0 0 2 0l7-4A2 2 0 0 0 21 16V8a2 2 0 0 0-1-1.73l-7-4a2 2 0 0 0-2 0l-7 4A2 2 0 0 0 3 8v8a2 2 0 0 0 1 1.73zM12 22V12M3.3 7l7.703 4.734a2 2 0 0 0 1.994 0L20.7 7" />
            </button>
            <button
              type="button"
              ref={refreshRef}
              aria-label="Check for updates"
              disabled={update.busy}
              onClick={() => {
                manualCheck.current = true;
                setSpins((n) => n + 1);
                update.check(true);
              }}
              className={iconButton}
            >
              <StrokeIcon
                key={spins}
                d="M3 12a9 9 0 0 1 9-9 9.75 9.75 0 0 1 6.74 2.74L21 8M21 3v5h-5M21 12a9 9 0 0 1-9 9 9.75 9.75 0 0 1-6.74-2.74L3 16M8 16H3v5"
                className={`size-4${spins ? " animate-spin-once" : ""}`}
              />
            </button>
          </span>,
          slot,
        )}
      {update.selfUpdatable ? (
        <Note className="mb-6 max-w-[600px]">
          Keeps the Launcher,{" "}
          <ExternalLink href={site.depotDownloaderRepoUrl}>
            DepotDownloader
          </ExternalLink>
          , <ExternalLink href="https://7-zip.org/">7z</ExternalLink>,{" "}
          <ExternalLink href={site.throwbackLoaderRepoUrl}>
            ThrowbackLoader
          </ExternalLink>
          , and{" "}
          <ExternalLink href={site.heatedMetalRepoUrl}>
            Heated Metal
          </ExternalLink>{" "}
          up to date.
        </Note>
      ) : (
        <Note variant="error" className="mb-6 max-w-[600px]">
          Download <code>Installer.exe</code> from the{" "}
          <ExternalLink href={site.latestReleaseUrl}>
            latest release
          </ExternalLink>{" "}
          to receive Launcher updates.
        </Note>
      )}

      <div className="flex max-w-[600px] flex-col gap-4">
        {update.components.map((component) => (
          <UpdateCard
            key={component.name}
            component={component}
            applying={update.applying === component.name && update.busy}
            progress={update.progress}
            disabled={update.busy || update.checking || downloading}
            onApply={() => update.apply(component.name)}
          />
        ))}
      </div>

      {installed && (
        <Modal title="Installed versions" onClose={() => setInstalled(null)}>
          <div className="flex flex-col gap-2">
            {installed.map((component) => (
              <ListRow key={component.name} label={component.name}>
                <VersionChip version={component.version} />
              </ListRow>
            ))}
          </div>
        </Modal>
      )}
    </>
  );
}
