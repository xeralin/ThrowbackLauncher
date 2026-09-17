"use client";

import Link from "next/link";
import { useEffect, useRef, useState, type ReactNode } from "react";
import { usePathname, useRouter } from "next/navigation";
import { CardKeyArt } from "@/components/SeasonKeyArt";
import { StrokeIcon } from "@/components/icons";
import { BlinkCursor, microLabel } from "@/components/ui";
import { isActivePath, navSections, normalizePath } from "@/config/nav";
import { site } from "@/config/site";
import { fetchMemberCount } from "@/lib/discord";
import {
  determinatePercent,
  seasonTitle,
  useDownloader,
  useDownloadProgress,
  useLaunch,
  useSeasons,
  useUpdate,
  type Season,
} from "@/lib/bridge";
import { useDetail } from "@/lib/detail";

function PauseIcon() {
  return <StrokeIcon d="M9.5 6v12M14.5 6v12" className="size-6" />;
}

function PlayIcon() {
  return <StrokeIcon d="M8.5 5.7 18.5 12l-10 6.3Z" className="size-6" />;
}

function CancelIcon() {
  return (
    <StrokeIcon d="M7.5 7.5 16.5 16.5M16.5 7.5 7.5 16.5" className="size-6" />
  );
}

function ActivityCard({
  season,
  tone,
  action,
  progress,
  onOpen,
}: {
  season: Season;
  tone?: "transfer" | "muted";
  action?: { label: string; icon: ReactNode; onClick: () => void };
  progress?: number;
  onOpen: () => void;
}) {
  return (
    <div
      data-tone={tone}
      className="card-glow-hover relative m-2 h-12 w-[calc(100%-1rem)] overflow-hidden rounded-md border border-border transition-[border-color,box-shadow] duration-200 [--card-glow-blur:16px]"
    >
      <CardKeyArt season={season} sizes="280px" />
      <button
        type="button"
        aria-label={seasonTitle(season)}
        onClick={onOpen}
        className="absolute inset-0 cursor-pointer"
      />
      {action && (
        <button
          type="button"
          aria-label={action.label}
          onClick={action.onClick}
          className="absolute inset-y-0 right-0 flex w-9 cursor-pointer items-center justify-center text-text"
        >
          {action.icon}
        </button>
      )}
      <div
        className={`pointer-events-none absolute inset-x-0 bottom-0 flex items-end justify-between gap-2 p-2 ${
          action ? "pr-11" : "pr-2"
        }`}
      >
        <span className="min-w-0 grow truncate-fade font-display text-[0.8rem] font-bold leading-none text-text">
          {seasonTitle(season)}
        </span>
      </div>
      {progress !== undefined && (
        <div
          className="card-line pointer-events-none absolute inset-x-0 bottom-0 h-px origin-left rounded-[1px] transition-transform duration-200"
          style={{ transform: `scaleX(${progress / 100})` }}
        />
      )}
    </div>
  );
}

function DownloadCard({
  season,
  onOpen,
  onToggle,
  paused,
  verifying,
}: {
  season: Season;
  onOpen: () => void;
  onToggle?: () => void;
  paused?: boolean;
  verifying?: boolean;
}) {
  const { progress, step, steps } = useDownloadProgress();
  const determinate = determinatePercent(progress, step, steps) !== null;
  return (
    <ActivityCard
      season={season}
      tone={paused ? "muted" : "transfer"}
      progress={determinate ? progress : undefined}
      onOpen={onOpen}
      action={
        onToggle && {
          label: paused ? "Continue" : verifying ? "Cancel" : "Pause",
          icon: paused ? (
            <PlayIcon />
          ) : verifying ? (
            <CancelIcon />
          ) : (
            <PauseIcon />
          ),
          onClick: onToggle,
        }
      }
    />
  );
}

function RunningCard({
  season,
  onOpen,
  onStop,
}: {
  season: Season;
  onOpen: () => void;
  onStop: () => void;
}) {
  return (
    <ActivityCard
      season={season}
      onOpen={onOpen}
      action={{ label: "Stop", icon: <CancelIcon />, onClick: onStop }}
    />
  );
}

export function Sidebar({
  open,
  onNavigate,
}: {
  open: boolean;
  onNavigate?: () => void;
}) {
  const pathname = normalizePath(usePathname());
  const router = useRouter();
  const [hoveredLink, setHoveredLink] = useState("");
  const linkRef = useRef<HTMLDivElement>(null);
  const [members, setMembers] = useState<string | null>(null);
  const [membersShown, setMembersShown] = useState(false);
  const membersRequested = useRef(false);

  function showMembers() {
    setMembersShown(true);
    if (membersRequested.current) return;
    membersRequested.current = true;
    fetchMemberCount(site.discordInvite).then((count) => {
      if (count != null) setMembers(count.toLocaleString("en-DK"));
    });
  }

  useEffect(() => {
    function preview(target: EventTarget | null) {
      const anchor =
        target instanceof Element ? target.closest("a[href]") : null;
      const href = anchor?.getAttribute("href") ?? "";
      setHoveredLink(
        /^https?:\/\//.test(href) && !href.startsWith(window.location.origin)
          ? href
          : "",
      );
    }
    function onOver(event: MouseEvent) {
      preview(event.target);
    }
    function onFocus(event: FocusEvent) {
      preview(event.target);
    }
    document.addEventListener("mouseover", onOver);
    document.addEventListener("focusin", onFocus);
    return () => {
      document.removeEventListener("mouseover", onOver);
      document.removeEventListener("focusin", onFocus);
    };
  }, []);

  useEffect(() => {
    const el = linkRef.current;
    if (el)
      el.classList.toggle("truncate-fade", el.scrollWidth > el.clientWidth);
  }, [hoveredLink]);
  const update = useUpdate();
  const [dragId, setDragId] = useState<string | null>(null);
  const pendingQueueFocus = useRef<string | null>(null);
  const { detail } = useDetail();
  const dl = useDownloader();
  const seasons = useSeasons();
  const findEdition = (key: string, hm: boolean) => {
    const season = seasons?.find((entry) => entry.key === key);
    return season ? { ...season, hm } : undefined;
  };
  const editionId = (season: Season) => `${season.key}:${season.hm}`;
  const queuedSeasons = dl.queue.flatMap((entry) => {
    const season = findEdition(entry.key, entry.hm);
    return season ? [season] : [];
  });
  if (
    dragId !== null &&
    !queuedSeasons.some((season) => editionId(season) === dragId)
  )
    setDragId(null);
  const activeSeason =
    dl.running || dl.state === "paused"
      ? findEdition(dl.activeKey, dl.activeHm)
      : undefined;
  const lc = useLaunch();
  const liveRefs = [...lc.running];
  if (
    lc.launching &&
    !lc.running.some(
      (ref) => ref.key === lc.launching?.key && ref.hm === lc.launching?.hm,
    )
  )
    liveRefs.push(lc.launching);
  const liveSeasons = liveRefs.flatMap((ref) => {
    const season = findEdition(ref.key, ref.hm);
    return season ? [season] : [];
  });

  useEffect(() => {
    const id = pendingQueueFocus.current;
    if (!id) return;
    pendingQueueFocus.current = null;
    const row = document.querySelector<HTMLElement>(
      `[data-queue-id="${CSS.escape(id)}"]`,
    );
    row?.focus({ preventScroll: true });
    row?.scrollIntoView({ block: "nearest" });
  }, [dl.queue]);

  function moveQueueEntry(from: number, to: number) {
    if (from < 0 || to < 0 || to >= dl.queue.length) return;
    const refs = dl.queue.map((entry) => ({
      key: entry.key,
      hm: entry.hm,
    }));
    const [moved] = refs.splice(from, 1);
    refs.splice(to, 0, moved);
    dl.reorderQueue(refs);
  }

  function openSeason(season: Season) {
    onNavigate?.();
    const ref = { key: season.key, hm: season.hm };
    if (detail?.seasonKey === season.key || pathname === "/download") {
      window.dispatchEvent(
        new CustomEvent("throwback:open-season", { detail: ref }),
      );
    } else {
      window.sessionStorage.setItem("tb-open-season", JSON.stringify(ref));
      router.push("/download");
    }
  }

  return (
    <aside
      id="sidebar"
      className={`fixed inset-y-0 left-0 z-(--z-sidebar) flex w-(--sidebar-w) flex-col overflow-y-auto border-r border-border bg-surface max-nav:transition-[translate,visibility] max-nav:duration-(--duration-view) max-nav:ease-out-cubic ${
        open
          ? "max-nav:translate-x-0"
          : "max-nav:invisible max-nav:-translate-x-full"
      }`}
    >
      <div className="border-b border-border px-5 pb-4 pt-6 max-nav:pt-14">
        <div
          onMouseEnter={showMembers}
          onMouseLeave={() => setMembersShown(false)}
          className={`mb-[0.3rem] w-fit ${microLabel} text-micro text-action`}
        >
          {membersShown && members
            ? `// ${members} MEMBERS`
            : "// R6S COMMUNITY"}
          <BlinkCursor />
        </div>
        <div className="font-display text-[1.2rem] font-bold leading-[1.2] text-text">
          <span className="text-action">Throwback</span> Launcher
        </div>
      </div>

      <nav>
        {navSections.map((section) => {
          const sectionActive = section.items.some((item) =>
            isActivePath(item.href, pathname),
          );
          return (
            <div key={section.label} className="px-3 pb-2 pt-[1.2rem]">
              <div
                className={`mb-[0.4rem] px-2 ${microLabel} text-micro ${
                  sectionActive ? "text-action" : "text-text-muted"
                }`}
              >
                {section.label}
              </div>
              {section.items.map((item) => {
                const active = isActivePath(item.href, pathname);
                return (
                  <Link
                    key={item.href}
                    href={item.href}
                    onClick={(event) => {
                      onNavigate?.();
                      if (active && detail) {
                        event.preventDefault();
                        detail.reset();
                      }
                    }}
                    aria-current={active ? "page" : undefined}
                    className={`nav-link flex items-center justify-between overflow-hidden rounded-md px-3 py-[0.55rem] text-[0.9rem] font-medium no-underline transition-[background-color,color,translate] duration-200 ${
                      active
                        ? "border-l-2 border-action bg-action-dim text-text shadow-[inset_0_0_18px_-5px_var(--color-action-glow-soft)]"
                        : "text-text-muted hover:bg-surface-2 hover:text-text"
                    }`}
                  >
                    <span>{item.label}</span>
                    {item.href === "/updates" &&
                      update.components.length > 0 && (
                        <span className="font-display text-[0.8rem] font-bold leading-none text-text">
                          {update.components.length}
                        </span>
                      )}
                  </Link>
                );
              })}
            </div>
          );
        })}
      </nav>

      <div className="mt-auto">
        {queuedSeasons.length > 0 && (
          <div className="mx-2 mb-2 flex max-h-36 flex-col-reverse overflow-y-auto rounded-md border border-border bg-surface-2">
            {queuedSeasons.map((season, index) => (
              <button
                key={editionId(season)}
                type="button"
                draggable
                onDragStart={(event) => {
                  event.dataTransfer.effectAllowed = "move";
                  setDragId(editionId(season));
                }}
                onDragEnd={() => setDragId(null)}
                onDragOver={(event) => {
                  event.preventDefault();
                  if (!dragId || dragId === editionId(season)) return;
                  const dragged = queuedSeasons.find(
                    (entry) => editionId(entry) === dragId,
                  );
                  if (!dragged) return;
                  const from = dl.queue.findIndex(
                    (entry) =>
                      entry.key === dragged.key && entry.hm === dragged.hm,
                  );
                  const to = dl.queue.findIndex(
                    (entry) =>
                      entry.key === season.key && entry.hm === season.hm,
                  );
                  moveQueueEntry(from, to);
                }}
                onKeyDown={(event) => {
                  if (
                    !event.altKey ||
                    (event.key !== "ArrowUp" && event.key !== "ArrowDown")
                  )
                    return;
                  event.preventDefault();
                  pendingQueueFocus.current = editionId(season);
                  const from = dl.queue.findIndex(
                    (entry) =>
                      entry.key === season.key && entry.hm === season.hm,
                  );
                  moveQueueEntry(
                    from,
                    from + (event.key === "ArrowUp" ? 1 : -1),
                  );
                }}
                onClick={() => openSeason(season)}
                data-queue-id={editionId(season)}
                className={`group flex w-full cursor-pointer items-center gap-2 px-2.5 py-1.5 text-left transition-colors hover:bg-border ${
                  dragId === editionId(season) ? "opacity-40" : ""
                }`}
              >
                <span className="font-mono text-micro text-text-muted">
                  {index + 1}
                </span>
                <span className="min-w-0 grow truncate-fade font-display text-label font-bold leading-none text-text-muted transition-colors group-hover:text-text">
                  {seasonTitle(season)}
                </span>
              </button>
            ))}
          </div>
        )}
        {activeSeason && (
          <DownloadCard
            season={activeSeason}
            onOpen={() => openSeason(activeSeason)}
            paused={dl.state === "paused"}
            verifying={dl.verifying}
            onToggle={
              dl.verifying && dl.state === "downloading"
                ? () => dl.cancel()
                : dl.state === "downloading" || dl.state === "paused"
                  ? () => dl.setPaused(dl.state !== "paused")
                  : undefined
            }
          />
        )}
        {liveSeasons.map((season) => (
          <RunningCard
            key={editionId(season)}
            season={season}
            onOpen={() => openSeason(season)}
            onStop={() => lc.stop(season.key)}
          />
        ))}
      </div>

      {hoveredLink && (
        <div
          ref={linkRef}
          className="pointer-events-none fixed bottom-2 left-2 z-(--z-link-preview) max-w-[calc(100vw_-_1rem)] overflow-hidden whitespace-nowrap rounded-md border border-border bg-surface-2 px-[3px] pb-[2px] pt-[3px] font-mono text-[11px] leading-none text-text max-nav:hidden"
        >
          {hoveredLink}
        </div>
      )}
    </aside>
  );
}
