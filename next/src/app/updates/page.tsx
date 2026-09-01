"use client";

import { useEffect, useRef, useState } from "react";
import { createPortal } from "react-dom";
import { buttonBase, buttonVariants, iconButton } from "@/components/Button";
import { card, iconBox, ListRow } from "@/components/ui";
import { Note } from "@/components/Note";
import { ExternalLink } from "@/components/ExternalLink";
import { VersionChip } from "@/components/VersionChip";
import { Dialog } from "@/components/Dialog";
import { StrokeIcon } from "@/components/icons";
import {
  useDownloader,
  useUpdate,
  type InstalledComponent,
  type UpdateComponent,
} from "@/lib/bridge";
import { renderInline } from "@/lib/inline-markdown";
import { dismissToast, RATE_LIMIT_TOAST, showToast } from "@/lib/toast";
import { useTopbarSlot } from "@/lib/topbar-slot";
import { site } from "@/config/site";

const UPDATE_TOAST = "update";

type ReleaseNoteEntry = UpdateComponent["notes"][number];
type ReleaseNoteGroup = { text: string; children: string[] };
type Block =
  | { kind: "heading"; text: string }
  | { kind: "list"; ordered: boolean; items: ReleaseNoteGroup[] };

function toBlocks(notes: ReleaseNoteEntry[]): Block[] {
  const blocks: Block[] = [];
  for (const note of notes) {
    if (note.kind === "heading") {
      blocks.push({ kind: "heading", text: note.text });
      continue;
    }
    const ordered = note.kind === "number";
    const last = blocks[blocks.length - 1];
    if (last?.kind === "list" && last.ordered === ordered) {
      if (note.level > 0 && last.items.length > 0)
        last.items[last.items.length - 1].children.push(note.text);
      else last.items.push({ text: note.text, children: [] });
    } else {
      blocks.push({
        kind: "list",
        ordered,
        items: [{ text: note.text, children: [] }],
      });
    }
  }
  return blocks;
}

function ReleaseNoteList({
  ordered,
  items,
}: {
  ordered: boolean;
  items: ReleaseNoteGroup[];
}) {
  const List = ordered ? "ol" : "ul";
  return (
    <List
      className={`space-y-0.5 pl-4 text-ui text-text-muted ${
        ordered ? "list-decimal" : "list-disc"
      }`}
    >
      {items.map((note, index) => (
        <li key={index}>
          {renderInline(note.text)}
          {note.children.length > 0 && (
            <ul className="list-[circle] space-y-0.5 pl-4">
              {note.children.map((child, childIndex) => (
                <li key={childIndex}>{renderInline(child)}</li>
              ))}
            </ul>
          )}
        </li>
      ))}
    </List>
  );
}

function ReleaseNotes({ notes }: { notes: UpdateComponent["notes"] }) {
  return (
    <div className="flex flex-col gap-2">
      {toBlocks(notes).map((block, index) => {
        if (block.kind === "heading") {
          return (
            <div
              key={index}
              className="mt-1.5 font-display text-[0.9rem] font-bold text-text first:mt-0"
            >
              {renderInline(block.text)}
            </div>
          );
        }
        return (
          <ReleaseNoteList
            key={index}
            ordered={block.ordered}
            items={block.items}
          />
        );
      })}
    </div>
  );
}

export default function UpdatesPage() {
  const downloading = useDownloader().running;
  const update = useUpdate();
  const refreshRef = useRef<HTMLButtonElement>(null);
  const [spinning, setSpinning] = useState(false);
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
              disabled={update.checking || update.busy}
              onClick={() => {
                manualCheck.current = true;
                setSpinning(true);
                update.check(true);
              }}
              onAnimationIteration={() => {
                if (!update.checking) setSpinning(false);
              }}
              className={iconButton}
            >
              <StrokeIcon
                d="M3 12a9 9 0 0 1 9-9 9.75 9.75 0 0 1 6.74 2.74L21 8M21 3v5h-5M21 12a9 9 0 0 1-9 9 9.75 9.75 0 0 1-6.74-2.74L3 16M8 16H3v5"
                className={`size-4${spinning ? " animate-spin" : ""}`}
              />
            </button>
          </span>,
          slot,
        )}
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
        <ExternalLink href={site.heatedMetalRepoUrl}>HeatedMetal</ExternalLink>{" "}
        up to date.
      </Note>

      <div className="flex max-w-[600px] flex-col gap-4">
        {update.components.map((component) => {
          const applying = update.applying === component.name && update.busy;
          return (
            <div key={component.name} className={card}>
              <div className="flex items-center justify-between gap-4">
                <span className="flex min-w-0 items-center gap-2.5">
                  <span className="truncate font-display text-[1.05rem] font-bold text-text">
                    {component.name}
                  </span>
                  <VersionChip
                    version={component.target}
                    className="shrink-0"
                  />
                </span>
                <button
                  type="button"
                  aria-disabled={applying || undefined}
                  disabled={
                    !applying && (update.busy || update.checking || downloading)
                  }
                  onClick={
                    applying ? undefined : () => update.apply(component.name)
                  }
                  className={`${buttonBase} shrink-0 justify-center ${
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
                        style={{
                          transform: `scaleX(${update.progress / 100})`,
                        }}
                      />
                      <span className="relative">Update</span>
                    </>
                  ) : (
                    "Update"
                  )}
                </button>
              </div>
              {component.notes.length > 0 && (
                <ReleaseNotes notes={component.notes} />
              )}
            </div>
          );
        })}
      </div>

      {installed && (
        <Dialog title="Installed versions" onClose={() => setInstalled(null)}>
          <div className="flex flex-col gap-2">
            {installed.map((component) => (
              <ListRow key={component.name} label={component.name}>
                <VersionChip version={component.version} />
              </ListRow>
            ))}
          </div>
        </Dialog>
      )}
    </>
  );
}
