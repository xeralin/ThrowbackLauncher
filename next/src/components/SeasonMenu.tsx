"use client";

import {
  useEffect,
  useLayoutEffect,
  useRef,
  useState,
  type ReactNode,
} from "react";
import { createPortal } from "react-dom";
import { iconButton } from "@/components/Button";
import {
  COPY,
  FolderIcon,
  PROTON,
  REFRESH,
  RemoveIcon,
  StrokeIcon,
} from "@/components/icons";
import { useSeasonManage } from "@/components/SeasonManage";
import { iconBox } from "@/components/ui";
import type { Season } from "@/lib/bridge";
import { usePopoverDismiss } from "@/lib/popover";
import { showToast } from "@/lib/toast";

const EDGE = 8;

type MenuTool = {
  label: string;
  icon: ReactNode;
  hidden?: boolean;
  disabled?: boolean;
  onSelect: () => void;
};

export function SeasonMenu({
  season,
  x,
  y,
  onClose,
}: {
  season: Season;
  x: number;
  y: number;
  onClose: () => void;
}) {
  const [open, setOpen] = useState(true);
  const [opener] = useState(() => document.activeElement);
  const ref = useRef<HTMLDivElement>(null);
  const {
    dl,
    platform,
    installs,
    loaded,
    checkingHm,
    hmActive,
    editionInstall,
    playingSeason,
    downloadingSeason,
    queuedEdition,
    editionState,
    transferring,
    protons,
    modal,
    setModal,
    verify,
    openProton,
    copyCrashLog,
    openFolder,
    modals,
  } = useSeasonManage(season, season.hm);

  const tools: MenuTool[] = [];
  if (transferring) {
    if (editionState === "paused")
      tools.push({
        label: "Remove",
        icon: <RemoveIcon />,
        onSelect: () => setModal({ kind: "removeDownload" }),
      });
  } else if (editionInstall.installed) {
    tools.push(
      queuedEdition
        ? {
            label: "Remove from queue",
            icon: <StrokeIcon d="M18 6 6 18M6 6l12 12" />,
            onSelect: () => dl.dequeue(season.key, hmActive),
          }
        : {
            label: "Verify",
            icon: <StrokeIcon d={REFRESH} />,
            disabled: playingSeason,
            onSelect: verify,
          },
      {
        label: "Copy crash log",
        icon: <StrokeIcon d={COPY} />,
        hidden: !hmActive,
        disabled: !installs.hm.crashLog,
        onSelect: () => copyCrashLog(() => showToast("Crash log copied")),
      },
      {
        label: "Proton",
        icon: <StrokeIcon d={PROTON} />,
        hidden: platform !== "linux",
        disabled: protons?.length === 0,
        onSelect: openProton,
      },
      { label: "Open folder", icon: <FolderIcon />, onSelect: openFolder },
      {
        label: "Uninstall",
        icon: <RemoveIcon />,
        disabled: downloadingSeason || playingSeason,
        onSelect: () => setModal({ kind: "uninstall" }),
      },
    );
  } else if (editionInstall.partial && !queuedEdition) {
    tools.push({
      label: "Remove",
      icon: <RemoveIcon />,
      disabled: playingSeason,
      onSelect: () => setModal({ kind: "removeDownload" }),
    });
  }
  const shownTools = tools.filter((tool) => !tool.hidden);

  const ready = loaded && (platform !== "linux" || protons !== null);
  if (open && ready && shownTools.every((tool) => tool.disabled))
    setOpen(false);
  const shown = open && ready;

  usePopoverDismiss(open, setOpen, ref);

  useEffect(() => {
    if (!open && !modal && !checkingHm) onClose();
  }, [open, modal, checkingHm, onClose]);

  useEffect(() => {
    if (open || modal || checkingHm || !(opener instanceof HTMLElement)) return;
    if (document.activeElement === document.body)
      opener.focus({ preventScroll: true });
  }, [open, modal, checkingHm, opener]);

  useLayoutEffect(() => {
    const menu = ref.current;
    if (!menu) return;
    const box = menu.getBoundingClientRect();
    const left = Math.max(EDGE, Math.min(x, innerWidth - box.width - EDGE));
    const top =
      y + box.height > innerHeight - EDGE ? Math.max(EDGE, y - box.height) : y;
    menu.style.left = `${left}px`;
    menu.style.top = `${top}px`;
  });

  useEffect(() => {
    if (shown)
      ref.current
        ?.querySelector<HTMLButtonElement>("button:enabled")
        ?.focus({ preventScroll: true });
  }, [shown]);

  function select(tool: MenuTool) {
    setOpen(false);
    tool.onSelect();
  }

  function onKeyDown(event: React.KeyboardEvent) {
    const keys = [
      "ArrowDown",
      "ArrowUp",
      "ArrowRight",
      "ArrowLeft",
      "Home",
      "End",
    ];
    if (!keys.includes(event.key)) return;
    event.preventDefault();
    const buttons = [
      ...(ref.current?.querySelectorAll<HTMLButtonElement>("button:enabled") ??
        []),
    ];
    if (!buttons.length) return;
    const index = buttons.indexOf(document.activeElement as HTMLButtonElement);
    const next =
      event.key === "Home"
        ? 0
        : event.key === "End"
          ? buttons.length - 1
          : (index +
              (event.key === "ArrowDown" || event.key === "ArrowRight"
                ? 1
                : -1) +
              buttons.length) %
            buttons.length;
    buttons[next].focus();
  }

  return (
    <>
      {shown &&
        createPortal(
          <div
            ref={ref}
            onKeyDown={onKeyDown}
            onContextMenu={(event) => event.preventDefault()}
            className={`fixed z-(--z-menu) ${iconBox} bg-surface shadow-lg`}
          >
            {shownTools.map((tool) => (
              <button
                key={tool.label}
                type="button"
                disabled={tool.disabled}
                onClick={() => select(tool)}
                className={iconButton}
              >
                {tool.icon}
              </button>
            ))}
          </div>,
          document.body,
        )}
      {modals}
    </>
  );
}
