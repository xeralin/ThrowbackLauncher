"use client";

import { useEffect, useMemo, useRef, useState } from "react";
import { createPortal } from "react-dom";
import { buttonBase, buttonVariants, iconButton } from "@/components/Button";
import { heading, iconBox, link, ListRow, textBoxText } from "@/components/ui";
import { Note } from "@/components/Note";
import { ExternalLink } from "@/components/ExternalLink";
import { VersionChip } from "@/components/VersionChip";
import { Modal } from "@/components/Modal";
import { CHEVRON_DOWN, GITHUB, REFRESH, StrokeIcon } from "@/components/icons";
import {
  useDownloader,
  useReleaseHistory,
  useUpdate,
  type InstalledComponent,
  type Release,
  type UpdateComponent,
} from "@/lib/bridge";
import { Markdown } from "@/components/Markdown";
import { parseReleaseNotes } from "@/lib/release-notes";
import { dismissToast, RATE_LIMIT_TOAST, showToast } from "@/lib/toast";
import { useTopbarSlot } from "@/lib/topbar-slot";
import { site } from "@/config/site";

const UPDATE_TOAST = "update";

const segment =
  "content-center whitespace-nowrap rounded-none px-[0.6rem] font-display text-ui font-bold text-box-cap transition-colors";

const changelogBox = "rounded-lg border border-border bg-well";

const boxRow = "border-t border-border px-[0.6rem] py-[0.45rem]";

function releaseDate(date: string) {
  return new Date(date).toLocaleDateString("en-US", {
    month: "short",
    day: "numeric",
    year: "numeric",
  });
}

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
    <div className={changelogBox}>
      <div className="flex flex-wrap items-center gap-x-4 gap-y-2 py-[0.45rem] pl-[0.6rem] pr-[0.45rem]">
        <span className="flex min-w-0 items-center gap-2.5">
          <span className={`truncate ${heading}`}>{component.name}</span>
          <VersionChip version={component.target} className="shrink-0" />
        </span>
        <button
          type="button"
          disabled={!applying && disabled}
          onClick={applying ? undefined : onApply}
          className={`${buttonBase} ml-auto shrink-0 justify-center ${
            applying
              ? `relative cursor-not-allowed overflow-hidden bg-[color-mix(in_srgb,var(--color-action)_45%,black)] text-action-text ${
                  progress > 0 && progress < 100 ? "" : "animate-pulse"
                }`
              : `disabled:cursor-not-allowed disabled:opacity-40 ${buttonVariants.primary}`
          }`}
        >
          {applying ? (
            <>
              <span
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
      {notes.length > 0 && <Markdown nodes={notes} className={boxRow} />}
      {component.url && (
        <div
          className={`flex items-baseline justify-between gap-4 text-ui text-text-muted ${boxRow}`}
        >
          <span>{releaseDate(component.date)}</span>
          <ExternalLink href={component.url} className={link}>
            View on GitHub
          </ExternalLink>
        </div>
      )}
    </div>
  );
}

function HistoryEntry({ release }: { release: Release }) {
  const [open, setOpen] = useState(false);
  const [seen, setSeen] = useState(false);
  const notes = useMemo(
    () => (seen ? parseReleaseNotes(release.body, release.repository) : []),
    [seen, release.body, release.repository],
  );
  return (
    <div
      data-reveal
      className={`${changelogBox} transition-colors has-[>button:hover]:border-action-edge`}
    >
      <button
        type="button"
        onClick={() => {
          setOpen((value) => !value);
          setSeen(true);
        }}
        className="flex h-7.5 w-full items-center gap-2.5 px-[0.6rem] text-left"
      >
        <span className={textBoxText}>{releaseDate(release.date)}</span>
        <VersionChip version={release.version} className="ml-auto shrink-0" />
        <StrokeIcon
          d={CHEVRON_DOWN}
          className={`size-3.5 shrink-0 text-text-muted transition-transform duration-200${
            open ? " rotate-180" : ""
          }`}
        />
      </button>
      <div
        inert={!open}
        className={`grid transition-[grid-template-rows] duration-200 ease-out-cubic ${
          open ? "grid-rows-[1fr]" : "grid-rows-[0fr]"
        }`}
      >
        <div className="min-h-0 overflow-hidden">
          {notes.length > 0 && <Markdown nodes={notes} className={boxRow} />}
        </div>
      </div>
    </div>
  );
}

export default function UpdatesPage() {
  const downloading = useDownloader().running;
  const update = useUpdate();
  const history = useReleaseHistory();
  const [historyName, setHistoryName] = useState("");
  const shown =
    history.find(
      (entry) => entry.name === historyName && entry.releases.length > 0,
    ) ?? history.find((entry) => entry.releases.length > 0);
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
                d={REFRESH}
                className={`size-4${spins ? " animate-spin-once" : ""}`}
              />
            </button>
          </span>,
          slot,
        )}
      {!update.selfUpdatable && (
        <Note variant="error" className="mb-6 max-w-[600px]">
          Download <code>ThrowbackLauncher.exe</code> from the{" "}
          <ExternalLink href={site.latestReleaseUrl}>
            latest release
          </ExternalLink>{" "}
          to receive Launcher updates.
        </Note>
      )}

      {update.components.length > 0 && (
        <div className="mb-6 flex max-w-[600px] flex-col gap-4">
          {update.components.map((component) => (
            <UpdateCard
              key={component.name}
              component={component}
              applying={update.applying === component.name}
              progress={update.progress}
              disabled={update.busy || update.checking || downloading}
              onApply={() => update.apply(component.name)}
            />
          ))}
        </div>
      )}

      {shown && (
        <div className="flex max-w-[600px] flex-col gap-4">
          <div className="flex flex-wrap gap-px self-start overflow-hidden rounded-lg border border-border bg-border">
            {history.map(({ name, releases }) => (
              <button
                key={name}
                type="button"
                disabled={releases.length === 0}
                onClick={() => setHistoryName(name)}
                className={`${segment} h-7.5 grow ${
                  name === shown.name
                    ? "bg-surface-2 text-text"
                    : "bg-surface text-text-muted hover:text-text disabled:cursor-not-allowed disabled:text-text-muted/40"
                }`}
              >
                {name}
              </button>
            ))}
          </div>
          <div className="flex flex-col gap-2">
            {shown.releases.map((release) => (
              <HistoryEntry
                key={`${shown.name} ${release.version}`}
                release={release}
              />
            ))}
          </div>
        </div>
      )}

      {installed && (
        <Modal title="Installed versions" onClose={() => setInstalled(null)}>
          <div className="flex flex-col gap-2">
            {installed.map((component) => (
              <ListRow key={component.name} label={component.name}>
                <VersionChip version={component.version} />
                <ExternalLink
                  href={`https://github.com/${component.repository}`}
                  className="flex text-text-muted transition-colors hover:text-text"
                >
                  <StrokeIcon
                    d={GITHUB}
                    fill="currentColor"
                    className="size-4 stroke-0"
                  />
                </ExternalLink>
              </ListRow>
            ))}
          </div>
        </Modal>
      )}
    </>
  );
}
