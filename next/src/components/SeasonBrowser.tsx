"use client";

import {
  memo,
  useCallback,
  useEffect,
  useLayoutEffect,
  useMemo,
  useRef,
  useState,
  type ReactNode,
} from "react";
import { createPortal, flushSync } from "react-dom";
import { usePathname } from "next/navigation";
import { Button, iconButton } from "@/components/Button";
import { Note } from "@/components/Note";
import { SeasonDetail } from "@/components/SeasonDetail";
import { CardKeyArt, SeasonKeyArt } from "@/components/SeasonKeyArt";
import { StrokeIcon } from "@/components/icons";
import { iconBox, inputClasses } from "@/components/ui";
import {
  editionActive,
  editionLaunching,
  editionQueued,
  editionRunning,
  seasonTitle,
  useDownloader,
  useLaunch,
  useSettings,
  useUpdateBusy,
  type Season,
} from "@/lib/bridge";
import {
  GRID_GAP,
  PAGE_PAD_CLAMP,
  ROW_SPAN_MAX,
  SIDEBAR_W_CLAMP,
  TOPBAR_H_PX,
} from "@/config/layout";
import { useDetail } from "@/lib/detail";
import { seasonRank } from "@/lib/seasons";
import { useTopbarSlot } from "@/lib/topbar-slot";
import { withViewTransition } from "@/lib/view-transition";
import { hasOpenModal } from "@/components/Modal";

const BannerCard = memo(function BannerCard({
  season,
  onOpen,
  onRegister,
}: {
  season: Season;
  onOpen: (season: Season) => void;
  onRegister: (id: string, el: HTMLElement | null) => void;
}) {
  return (
    <button
      type="button"
      ref={(el) => onRegister(season.id, el)}
      onClick={() => onOpen(season)}
      className="group relative block h-[210px] w-full overflow-hidden rounded-none text-left focus-visible:-outline-offset-2"
    >
      <SeasonKeyArt
        keyArt={season.keyArt}
        sizes="100vw"
        imgClassName="keyart-zoom"
      />

      <div className="absolute inset-0 bg-black/32 transition-colors duration-200 group-hover:bg-black/12 group-focus-visible:bg-black/12" />

      <div className="absolute left-10 top-1/2 -translate-y-1/2 font-display text-[1.9rem] font-bold leading-none text-text">
        {season.code}
      </div>

      <div className="pointer-events-none absolute inset-0 flex items-center justify-center px-28">
        <span className="font-display text-[1.9rem] font-bold leading-none text-text">
          {season.name}
        </span>
      </div>
    </button>
  );
});

const DEFAULT_SIZE = "1x2";
const ROW_SPANS = [
  "",
  "",
  "row-span-2",
  "row-span-3",
  "row-span-4",
  "row-span-5",
  "row-span-6",
  "row-span-7",
];
const COL_SPANS = [
  "",
  "",
  "col-span-2 max-cards:col-span-1",
  "col-span-2 max-cards:col-span-1 cards-wide:col-span-3",
  "col-span-2 max-cards:col-span-1 cards-wide:col-span-3 wide:col-span-4",
];
const KEY_ART_MAX_ASPECT = 5.5;

function maxSpanW(grid: Element): number {
  const columns = getComputedStyle(grid).gridTemplateColumns.split(" ").length;
  return Math.min(COL_SPANS.length - 1, columns);
}

function cardSizes(spanW: number, spanH: number): string {
  const size = (cols: number) => {
    const rowUnit =
      (TOPBAR_H_PX + (ROW_SPAN_MAX - 1) * GRID_GAP) / ROW_SPAN_MAX - GRID_GAP;
    const height = `${((KEY_ART_MAX_ASPECT * 100 * spanH) / ROW_SPAN_MAX).toFixed(2)}dvh - ${(
      KEY_ART_MAX_ASPECT *
      (rowUnit * spanH + GRID_GAP)
    ).toFixed(
      2,
    )}px - ${((KEY_ART_MAX_ASPECT * 2 * spanH) / ROW_SPAN_MAX).toFixed(2)} * ${PAGE_PAD_CLAMP}`;
    const span = Math.min(spanW, cols);
    const track = `(100vw - ${SIDEBAR_W_CLAMP} - 2 * ${PAGE_PAD_CLAMP} - ${GRID_GAP * cols}px)`;
    const width =
      span === cols
        ? `${track} + ${GRID_GAP * (span - 1)}px`
        : span === 1
          ? `${track}/${cols}`
          : `${track}*${span}/${cols} + ${GRID_GAP * (span - 1)}px`;
    return `max(${width}, ${height})`;
  };
  return `(min-width: 100em) ${size(4)}, (min-width: 80em) ${size(3)}, ${size(2)}`;
}

type HistoryEntry = {
  tbSeason?: string;
  tbHm?: boolean;
  tbDepth?: number;
  tbPath?: string;
};

type CardAction = {
  kind: "pause" | "dequeue" | "verify" | "cancel" | "play" | "stop";
  label: string;
  primary: boolean;
};

const DashCard = memo(function DashCard({
  season,
  actionLabel,
  actionPrimary,
  tone,
  editing,
  dragging,
  wigglePhase,
  spanW,
  spanH,
  onRegister,
  onOpen,
  onAction,
  onDragStart,
  onDragOver,
  onDragEnd,
  onMove,
  onResizePreview,
  onResizeCommit,
}: {
  season: Season;
  actionLabel: string | null;
  actionPrimary: boolean;
  tone?: "purple" | "muted";
  editing: boolean;
  dragging: boolean;
  wigglePhase: number;
  spanW: number;
  spanH: number;
  onRegister: (id: string, el: HTMLElement | null) => void;
  onOpen: (season: Season) => void;
  onAction: (season: Season) => void;
  onDragStart: (id: string) => void;
  onDragOver: (id: string) => void;
  onDragEnd: (cancelled: boolean) => void;
  onMove: (id: string, delta: number) => void;
  onResizePreview: (id: string, width: number, height: number) => void;
  onResizeCommit: (id: string, width: number, height: number) => void;
}) {
  const rootRef = useRef<HTMLDivElement | null>(null);

  function startResize(event: React.PointerEvent) {
    const root = rootRef.current;
    const grid = root?.parentElement;
    if (!root || !grid || event.button !== 0 || !event.isPrimary) return;
    event.preventDefault();
    event.stopPropagation();
    event.currentTarget.setPointerCapture(event.pointerId);
    const pointerId = event.pointerId;
    const gridStyle = getComputedStyle(grid);
    const columns = gridStyle.gridTemplateColumns.split(" ").length;
    const gap = parseFloat(gridStyle.gap) || GRID_GAP;
    const unitW = (grid.clientWidth - (columns - 1) * gap) / columns + gap;
    const maxW = maxSpanW(grid);
    const startRect = root.getBoundingClientRect();
    const startX = event.clientX;
    const startY = event.clientY;
    const startW = spanW;
    const startH = spanH;
    const renderedW = Math.round((startRect.width + gap) / unitW);
    const rowUnit = (startRect.height + gap) / startH;
    let lastW = spanW;
    let lastH = spanH;

    function detach() {
      window.removeEventListener("pointermove", onMove);
      window.removeEventListener("pointerup", onUp);
      window.removeEventListener("pointercancel", onCancel);
    }

    function onMove(move: PointerEvent) {
      if (move.pointerId !== pointerId) return;
      const column = Math.max(
        1,
        Math.min(
          maxW,
          Math.round((startRect.width + move.clientX - startX + gap) / unitW),
        ),
      );
      const width = column === renderedW ? startW : column;
      const height = Math.max(
        1,
        Math.min(
          ROW_SPAN_MAX,
          Math.round(
            (startRect.height + move.clientY - startY + gap) / rowUnit,
          ),
        ),
      );
      if (width !== lastW || height !== lastH) {
        lastW = width;
        lastH = height;
        onResizePreview(season.id, width, height);
      }
    }

    function onUp(up: PointerEvent) {
      if (up.pointerId !== pointerId) return;
      detach();
      onResizeCommit(season.id, lastW, lastH);
    }

    function onCancel(cancel: PointerEvent) {
      if (cancel.pointerId !== pointerId) return;
      detach();
      onResizePreview(season.id, startW, startH);
    }

    window.addEventListener("pointermove", onMove);
    window.addEventListener("pointerup", onUp);
    window.addEventListener("pointercancel", onCancel);
  }

  return (
    <div
      ref={(el) => {
        rootRef.current = el;
        onRegister(season.id, el);
      }}
      data-tone={tone}
      style={{
        viewTransitionName: `card-${season.id}`,
        ...(editing && !dragging
          ? { animationDelay: `-${wigglePhase * 90}ms` }
          : {}),
      }}
      draggable={editing}
      tabIndex={editing ? 0 : undefined}
      role={editing ? "button" : undefined}
      aria-label={editing ? seasonTitle(season) : undefined}
      aria-keyshortcuts={editing ? "Alt+ArrowLeft Alt+ArrowRight" : undefined}
      onKeyDown={
        editing
          ? (event) => {
              if (
                !event.altKey ||
                (event.key !== "ArrowLeft" && event.key !== "ArrowRight")
              )
                return;
              event.preventDefault();
              onMove(season.id, event.key === "ArrowRight" ? 1 : -1);
            }
          : undefined
      }
      onDragStart={(event) => {
        event.dataTransfer.effectAllowed = "move";
        event.dataTransfer.setData("application/x-throwback-season", season.id);
        onDragStart(season.id);
      }}
      onDragOver={(event) => {
        event.preventDefault();
        onDragOver(season.id);
      }}
      onDrop={(event) => event.preventDefault()}
      onDragEnd={(event) => onDragEnd(event.dataTransfer.dropEffect === "none")}
      className={`group relative h-full rounded-lg border border-border bg-surface transition-[border-color,box-shadow,opacity] duration-200 ${
        editing
          ? "cursor-grab active:cursor-grabbing"
          : "cursor-pointer overflow-hidden card-glow-hover card-line-hover"
      } ${editing && !dragging ? "animate-wiggle" : ""} ${
        dragging ? "opacity-40" : ""
      } ${COL_SPANS[spanW] ?? ""} ${ROW_SPANS[spanH] ?? ""}`}
    >
      <div className="absolute inset-0 overflow-hidden rounded-[7px] will-change-transform">
        <CardKeyArt season={season} sizes={cardSizes(spanW, spanH)} />
      </div>
      {!editing && (
        <button
          type="button"
          aria-label={seasonTitle(season)}
          onClick={() => onOpen(season)}
          className="absolute inset-0 cursor-pointer rounded-lg"
        />
      )}
      {editing && (
        <span
          role="button"
          tabIndex={0}
          aria-label={`Resize ${seasonTitle(season)}`}
          aria-keyshortcuts="ArrowRight ArrowLeft ArrowUp ArrowDown"
          draggable={false}
          onDragStart={(event) => {
            event.preventDefault();
            event.stopPropagation();
          }}
          onPointerDown={startResize}
          onKeyDown={(event) => {
            if (event.altKey) return;
            if (event.key === " " || event.key === "Enter") {
              event.preventDefault();
              return;
            }
            const steps: Record<string, [number, number]> = {
              ArrowRight: [1, 0],
              ArrowLeft: [-1, 0],
              ArrowDown: [0, 1],
              ArrowUp: [0, -1],
            };
            const step = steps[event.key];
            if (!step) return;
            event.preventDefault();
            const grid = rootRef.current?.parentElement;
            const maxW = grid ? maxSpanW(grid) : 1;
            const width = Math.max(1, Math.min(maxW, spanW + step[0]));
            const height = Math.max(1, Math.min(ROW_SPAN_MAX, spanH + step[1]));
            if (width === spanW && height === spanH) return;
            onResizePreview(season.id, width, height);
            onResizeCommit(season.id, width, height);
          }}
          className="absolute -bottom-[3px] -right-[3px] z-10 size-4 touch-none cursor-nwse-resize text-text/90"
        >
          <svg viewBox="0 0 16 16" fill="none" className="size-full">
            <path
              d="M 2.5 13.5 L 6 13.5 A 7.5 7.5 0 0 0 13.5 6 L 13.5 2.5"
              stroke="currentColor"
              strokeWidth="5"
              strokeLinecap="round"
            />
          </svg>
        </span>
      )}
      <div className="pointer-events-none absolute inset-x-0 bottom-0 flex items-end justify-between gap-2 py-2 pl-3 pr-2">
        <div className="min-w-0 grow truncate-fade font-display text-[1.05rem] font-bold leading-tight text-text">
          {seasonTitle(season)}
        </div>
        {actionLabel && (
          <Button
            variant={actionPrimary ? "primary" : "secondary"}
            className={editing ? "" : "pointer-events-auto"}
            tabIndex={editing ? -1 : undefined}
            onClick={() => onAction(season)}
          >
            {actionLabel}
          </Button>
        )}
      </div>
    </div>
  );
});

function ArrangeIcon({ active }: { active: boolean }) {
  return (
    <StrokeIcon className="size-4">
      <path d="M13 21h8" />
      <path
        fill={active ? "currentColor" : "none"}
        d="M16.5 3.5a2.12 2.12 0 0 1 3 3L7 19l-4 1 1-4Z"
      />
    </StrokeIcon>
  );
}

const SEARCH_MAX_LENGTH = 25;

export function SeasonBrowser({
  seasons,
  emptyMessage,
  layout = "banner",
  onReturn,
  searchable = false,
}: {
  seasons: Season[] | null;
  emptyMessage: ReactNode;
  layout?: "banner" | "dashboard";
  onReturn?: () => void;
  searchable?: boolean;
}) {
  const [selected, setSelected] = useState<{
    season: Season;
    hm: boolean;
    depth: number;
  } | null>(null);
  const [query, setQuery] = useState("");
  const searchRef = useRef<HTMLInputElement>(null);

  useEffect(() => {
    if (!searchable || selected) return;
    function onKey(event: KeyboardEvent) {
      if (event.ctrlKey || event.metaKey || event.altKey) return;
      const typing = event.key.length === 1 && !(event.key === " " && !query);
      const erasing = event.key === "Backspace" && !!query;
      if (!typing && !erasing) return;
      const target = event.target;
      if (
        target instanceof HTMLInputElement ||
        target instanceof HTMLTextAreaElement
      )
        return;
      if (hasOpenModal()) return;
      event.preventDefault();
      setQuery((prev) =>
        erasing
          ? prev.slice(0, -1)
          : (prev + event.key).slice(0, SEARCH_MAX_LENGTH),
      );
      const input = searchRef.current;
      if (input) {
        input.focus();
        requestAnimationFrame(() =>
          input.setSelectionRange(input.value.length, input.value.length),
        );
      }
    }
    window.addEventListener("keydown", onKey);
    return () => window.removeEventListener("keydown", onKey);
  }, [searchable, selected, query]);
  const pathname = usePathname();
  const topbarSlot = useTopbarSlot();
  const [direction, setDirection] = useState<"none" | "forward" | "back">(
    "none",
  );
  const [returnScroll, setReturnScroll] = useState(0);
  const [editing, setEditing] = useState(false);
  const [resetErase, setResetErase] = useState(0);
  const [dragId, setDragId] = useState<string | null>(null);
  const [draftOrder, setDraftOrder] = useState<string[] | null>(null);
  const [draftSizes, setDraftSizes] = useState<Record<string, string>>({});
  const cardRefs = useRef(new Map<string, HTMLElement>());
  const lastRects = useRef(new Map<string, { left: number; top: number }>());
  const flipping = useRef(false);
  const dl = useDownloader();
  const lc = useLaunch();
  const settings = useSettings();
  const updateBusy = useUpdateBusy();

  const [restoredFrom, setRestoredFrom] = useState<Season[] | null>(null);
  if (seasons?.length && seasons !== restoredFrom) {
    setRestoredFrom(seasons);
    const entry = window.history.state as HistoryEntry | null;
    if (!selected && entry?.tbSeason && entry.tbPath === pathname) {
      const restored =
        seasons.find(
          (s) => s.key === entry.tbSeason && s.hm === !!entry.tbHm,
        ) ?? seasons.find((s) => s.key === entry.tbSeason);
      if (restored)
        setSelected({
          season: restored,
          hm: !!entry.tbHm,
          depth: entry.tbDepth ?? 0,
        });
    }
  }

  const open = useCallback(
    (season: Season, hm: boolean = season.hm) => {
      const state = window.history.state as HistoryEntry | null;
      if (!state?.tbSeason) setReturnScroll(window.scrollY);
      const depth = (state?.tbDepth ?? 0) + 1;
      setDirection("forward");
      window.history.pushState(
        {
          tbSeason: season.key,
          tbHm: hm,
          tbDepth: depth,
          tbPath: pathname,
        },
        "",
      );
      setSelected({ season, hm, depth });
    },
    [pathname],
  );

  const setEdition = useCallback((hm: boolean) => {
    const state = window.history.state as HistoryEntry | null;
    if (state?.tbSeason)
      window.history.replaceState({ ...state, tbHm: hm }, "");
    setSelected((prev) => (prev ? { ...prev, hm } : prev));
  }, []);

  const resetToList = useCallback(() => {
    const depth = (window.history.state as HistoryEntry | null)?.tbDepth ?? 0;
    if (depth > 0) window.history.go(-depth);
  }, []);

  const closeDetail = useCallback(() => {
    const closing = selected?.season.id;
    flushSync(() => {
      setDirection("back");
      setSelected(null);
    });
    window.scrollTo(0, returnScroll);
    const el = closing ? cardRefs.current.get(closing) : undefined;
    const target =
      el instanceof HTMLButtonElement
        ? el
        : el?.querySelector<HTMLElement>("button");
    target?.focus({ preventScroll: true });
    onReturn?.();
  }, [selected, returnScroll, onReturn]);

  const back = useCallback(() => {
    window.history.back();
  }, []);

  const { setDetail } = useDetail();
  useEffect(() => {
    setDetail(
      selected
        ? {
            label: seasonTitle(selected.season, selected.hm),
            seasonKey: selected.season.key,
            reset: resetToList,
          }
        : null,
    );
    return () => setDetail(null);
  }, [selected, setDetail, resetToList]);

  useLayoutEffect(() => {
    if (!editing) {
      if (lastRects.current.size) lastRects.current = new Map();
      return;
    }
    const previous = lastRects.current;
    const next = new Map<string, { left: number; top: number }>();
    const flips: Animation[] = [];
    cardRefs.current.forEach((el, id) => {
      const rect = el.getBoundingClientRect();
      const pos = {
        left: rect.left + window.scrollX,
        top: rect.top + window.scrollY,
      };
      next.set(id, pos);
      const before = previous.get(id);
      if (!before) return;
      const dx = before.left - pos.left;
      const dy = before.top - pos.top;
      if (
        (Math.abs(dx) > 2 || Math.abs(dy) > 2) &&
        !document.documentElement.hasAttribute("data-reduce-motion")
      ) {
        flips.push(
          el.animate(
            [
              { transform: `translate(${dx}px, ${dy}px)` },
              { transform: "none" },
            ],
            { duration: 260, easing: "cubic-bezier(0.2, 0, 0, 1)" },
          ),
        );
      }
    });
    lastRects.current = next;
    if (flips.length) {
      flipping.current = true;
      Promise.allSettled(flips.map((flip) => flip.finished)).then(() => {
        flipping.current = false;
      });
    }
  });

  useEffect(() => {
    const onPop = (event: PopStateEvent) => {
      const state = event.state as HistoryEntry | null;
      const poppedDepth = state?.tbDepth ?? 0;
      const list = seasons ?? [];
      const target = state?.tbSeason
        ? (list.find(
            (s) => s.key === state.tbSeason && s.hm === !!state.tbHm,
          ) ?? list.find((s) => s.key === state.tbSeason))
        : null;
      if (target) {
        setDirection(poppedDepth < (selected?.depth ?? 0) ? "back" : "forward");
        setSelected({ season: target, hm: !!state?.tbHm, depth: poppedDepth });
      } else if (selected) {
        closeDetail();
      }
    };
    window.addEventListener("popstate", onPop);
    return () => window.removeEventListener("popstate", onPop);
  }, [seasons, selected, closeDetail]);

  useEffect(() => {
    if (!selected && !query) return;
    function onBack(event: Event) {
      event.preventDefault();
      if (!selected && query) {
        setQuery("");
        return;
      }
      back();
    }
    window.addEventListener("throwback:back", onBack);
    return () => window.removeEventListener("throwback:back", onBack);
  }, [selected, back, query]);

  useEffect(() => {
    if (!seasons) return;
    function openByRef(ref: { key: string; hm: boolean }) {
      const target = seasons?.find((season) => season.key === ref.key);
      if (!target) return;
      const hm = ref.hm && target.hmAvailable;
      if (selected?.season.key === target.key) {
        if (selected.hm !== hm) setEdition(hm);
        return;
      }
      open(target, hm);
    }
    const pending = window.sessionStorage.getItem("tb-open-season");
    if (pending) {
      const ref = JSON.parse(pending) as { key: string; hm: boolean };
      if (seasons.some((season) => season.key === ref.key)) {
        window.sessionStorage.removeItem("tb-open-season");
        openByRef(ref);
      }
    }
    function onOpen(event: Event) {
      openByRef((event as CustomEvent).detail as { key: string; hm: boolean });
    }
    window.addEventListener("throwback:open-season", onOpen);
    return () => window.removeEventListener("throwback:open-season", onOpen);
  }, [seasons, selected, open, setEdition]);

  const cardAction = useCallback(
    (season: Season): CardAction | null => {
      const activeEdition = editionActive(dl, season.key, season.hm);
      if (season.partial) {
        if (dl.running && activeEdition) {
          return dl.state === "downloading"
            ? { kind: "pause", label: "Pause", primary: false }
            : null;
        }
        if (activeEdition && dl.state === "failed") {
          return { kind: "verify", label: "Verify", primary: false };
        }
        if (editionQueued(dl, season.key, season.hm)) {
          return {
            kind: "dequeue",
            label: "Remove from queue",
            primary: false,
          };
        }
        return { kind: "verify", label: "Verify", primary: false };
      }
      if (dl.running && activeEdition) {
        return dl.state === "downloading"
          ? { kind: "cancel", label: "Cancel", primary: false }
          : null;
      }
      if (
        editionRunning(lc, season.key, season.hm) ||
        editionLaunching(lc, season.key, season.hm)
      ) {
        return { kind: "stop", label: "Stop", primary: true };
      }
      if (updateBusy) return null;
      return { kind: "play", label: "Play", primary: true };
    },
    [dl, lc, updateBusy],
  );

  const cardTone = useCallback(
    (season: Season): "purple" | "muted" | undefined => {
      const activeEdition = editionActive(dl, season.key, season.hm);
      if (dl.verifying && activeEdition) return "purple";
      if (!season.partial) return undefined;
      if (dl.running && activeEdition) return "purple";
      return "muted";
    },
    [dl],
  );

  const registerCard = useCallback((id: string, el: HTMLElement | null) => {
    if (el) cardRefs.current.set(id, el);
    else cardRefs.current.delete(id);
  }, []);

  const startDrag = useCallback((id: string) => setDragId(id), []);

  const previewResize = useCallback(
    (id: string, width: number, height: number) =>
      setDraftSizes((prev) => ({ ...prev, [id]: `${width}x${height}` })),
    [],
  );

  const commitResize = useCallback(
    (id: string, width: number, height: number) =>
      settings?.set_home_size(id, width, height),
    [settings],
  );

  const animation =
    direction === "forward"
      ? "animate-slide-from-right"
      : direction === "back"
        ? "animate-slide-from-left"
        : "";

  const trimmed = query.trim().toLowerCase();
  const visible = useMemo(
    () =>
      seasons && trimmed
        ? seasons.filter((season) =>
            [season.label, season.hmAvailable ? "heated metal hm" : ""]
              .join(" ")
              .toLowerCase()
              .includes(trimmed),
          )
        : seasons,
    [seasons, trimmed],
  );

  const bySeason = useMemo(
    () => new Map((visible ?? []).map((season) => [season.id, season])),
    [visible],
  );

  const savedOrder = settings?.home_order;
  const defaultOrder = useMemo(
    () =>
      [...bySeason.values()]
        .sort((a, b) => seasonRank(a.key) - seasonRank(b.key))
        .map((season) => season.id),
    [bySeason],
  );
  const baseKeys = useMemo(() => {
    const saved = savedOrder ?? [];
    return [
      ...saved.filter((id) => bySeason.has(id)),
      ...defaultOrder.filter((id) => !saved.includes(id)),
    ];
  }, [bySeason, savedOrder, defaultOrder]);

  const effectiveOrder = useMemo(() => {
    if (!draftOrder || !draftOrder.every((id) => bySeason.has(id)))
      return baseKeys;
    const missing = baseKeys.filter((id) => !draftOrder.includes(id));
    return missing.length ? [...draftOrder, ...missing] : draftOrder;
  }, [draftOrder, bySeason, baseKeys]);

  const latest = useRef({
    dragId,
    draftOrder,
    effectiveOrder,
    savedOrder,
    settings,
    cardAction,
    dl,
    lc,
  });
  useLayoutEffect(() => {
    latest.current = {
      dragId,
      draftOrder,
      effectiveOrder,
      savedOrder,
      settings,
      cardAction,
      dl,
      lc,
    };
  });

  const runAction = useCallback((season: Season) => {
    const { cardAction, dl, lc } = latest.current;
    const action = cardAction(season);
    if (!action) return;
    switch (action.kind) {
      case "pause":
        dl.setPaused(true);
        break;
      case "dequeue":
        dl.dequeue(season.key, season.hm);
        break;
      case "verify":
        dl.enqueue(season.key, season.hm, "");
        break;
      case "cancel":
        dl.cancel();
        break;
      case "play":
        lc.launch(season.key, season.hm);
        break;
      case "stop":
        lc.stop(season.key);
        break;
    }
  }, []);

  const moveDragged = useCallback((overId: string) => {
    const { dragId, effectiveOrder } = latest.current;
    if (!dragId || dragId === overId || flipping.current) return;
    const from = effectiveOrder.indexOf(dragId);
    const to = effectiveOrder.indexOf(overId);
    if (from === -1 || to === -1 || from === to) return;
    const next = [...effectiveOrder];
    next.splice(from, 1);
    next.splice(to, 0, dragId);
    setDraftOrder(next);
  }, []);

  const persistOrder = useCallback((order: string[]) => {
    const { savedOrder, settings } = latest.current;
    const saved = savedOrder ?? [];
    const displayed = new Set(order);
    const queue = [...order];
    const merged = saved.map((id) =>
      displayed.has(id) ? (queue.shift() as string) : id,
    );
    settings?.set_home_order([...merged, ...queue]);
  }, []);

  const moveCard = useCallback(
    (id: string, delta: number) => {
      const { effectiveOrder } = latest.current;
      const from = effectiveOrder.indexOf(id);
      const to = from + delta;
      if (from === -1 || to < 0 || to >= effectiveOrder.length) return;
      const next = [...effectiveOrder];
      next.splice(from, 1);
      next.splice(to, 0, id);
      persistOrder(next);
      flushSync(() => setDraftOrder(next));
      cardRefs.current.get(id)?.focus();
    },
    [persistOrder],
  );

  const endDrag = useCallback(
    (cancelled: boolean) => {
      const { draftOrder } = latest.current;
      if (cancelled) setDraftOrder(null);
      else if (draftOrder) persistOrder(draftOrder);
      setDragId(null);
    },
    [persistOrder],
  );

  const applyLayout = useCallback(
    (apply: () => void) => {
      if (editing) apply();
      else withViewTransition(apply, "cards");
    },
    [editing],
  );

  const reverseOrder = useCallback(() => {
    const order = [...latest.current.effectiveOrder].reverse();
    persistOrder(order);
    applyLayout(() => setDraftOrder(order));
  }, [persistOrder, applyLayout]);

  const resetLayout = useCallback(() => {
    latest.current.settings?.reset_home_layout();
    setResetErase((value) => value + 1);
    applyLayout(() => {
      setDraftOrder(defaultOrder);
      setDraftSizes(
        Object.fromEntries(defaultOrder.map((id) => [id, DEFAULT_SIZE])),
      );
    });
  }, [applyLayout, defaultOrder]);

  let listContent: ReactNode;
  if (seasons === null || visible === null) {
    listContent = (
      <p className="text-ui">
        <code className="chip animate-pulse">Loading seasons</code>
      </p>
    );
  } else if (seasons.length === 0) {
    listContent = emptyMessage;
  } else if (visible.length === 0) {
    listContent = (
      <Note className="max-w-[640px]">
        No seasons match <span className="font-semibold">{query.trim()}</span>.
      </Note>
    );
  } else if (layout === "dashboard") {
    listContent = (
      <div
        className="home-grid grid grid-cols-2 max-cards:grid-cols-1 cards-wide:grid-cols-3 wide:grid-cols-4"
        style={
          {
            "--grid-gap": `${GRID_GAP}px`,
            "--home-rows": `${ROW_SPAN_MAX}`,
          } as React.CSSProperties
        }
        onDragOver={editing ? (event) => event.preventDefault() : undefined}
        onDrop={editing ? (event) => event.preventDefault() : undefined}
      >
        {effectiveOrder.flatMap((id, index) => {
          const season = bySeason.get(id);
          if (!season) return [];
          const action = cardAction(season);
          const [spanW, spanH] = (
            draftSizes[season.id] ??
            settings?.home_sizes[season.id] ??
            DEFAULT_SIZE
          )
            .split("x")
            .map(Number);
          return (
            <DashCard
              key={season.id}
              season={season}
              actionLabel={action ? action.label : null}
              actionPrimary={action !== null && action.primary}
              tone={cardTone(season)}
              editing={editing}
              dragging={dragId === season.id}
              wigglePhase={index % 4}
              spanW={spanW}
              spanH={spanH}
              onResizePreview={previewResize}
              onResizeCommit={commitResize}
              onRegister={registerCard}
              onOpen={open}
              onAction={runAction}
              onDragStart={startDrag}
              onDragOver={moveDragged}
              onDragEnd={endDrag}
              onMove={moveCard}
            />
          );
        })}
      </div>
    );
  } else {
    listContent = (
      <div className="-m-(--page-pad) flex flex-col">
        {visible.map((season) => (
          <BannerCard
            key={season.id}
            season={season}
            onOpen={open}
            onRegister={registerCard}
          />
        ))}
      </div>
    );
  }

  return (
    <div key={selected ? selected.season.key : "list"} className={animation}>
      {selected ? (
        <SeasonDetail
          season={selected.season}
          hm={selected.hm}
          onHmChange={setEdition}
          onBack={back}
        />
      ) : (
        <>
          {layout === "dashboard" &&
            topbarSlot &&
            (seasons?.length ?? 0) > 0 &&
            createPortal(
              <div className={`${iconBox} bg-surface-2`}>
                <button
                  type="button"
                  aria-label="Reset layout"
                  onClick={resetLayout}
                  className={iconButton}
                >
                  <StrokeIcon
                    key={resetErase}
                    d="m7 21-4.3-4.3c-1-1-1-2.5 0-3.4l9.6-9.6c1-1 2.5-1 3.4 0l5.6 5.6c1 1 1 2.5 0 3.4L13 21M22 21H7M5 11l9 9"
                    className={`size-4${resetErase ? " animate-erase-once" : ""}`}
                  />
                </button>
                {(seasons?.length ?? 0) > 1 && (
                  <button
                    type="button"
                    aria-label="Reverse order"
                    onClick={reverseOrder}
                    className={iconButton}
                  >
                    <StrokeIcon d="M7 20V4m0 0L4 7m3-3 3 3M17 4v16m0 0 3-3m-3 3-3-3" />
                  </button>
                )}
                <button
                  type="button"
                  aria-label={editing ? "Done arranging" : "Arrange seasons"}
                  onClick={() => {
                    setEditing((value) => !value);
                    setResetErase(0);
                    setDraftOrder(null);
                    setDraftSizes({});
                    setDragId(null);
                  }}
                  className={`p-1 transition-colors ${
                    editing ? "text-action" : "text-text-muted hover:text-text"
                  }`}
                >
                  <ArrangeIcon active={editing} />
                </button>
              </div>,
              topbarSlot,
            )}
          {searchable &&
            topbarSlot &&
            createPortal(
              <input
                ref={searchRef}
                value={query}
                maxLength={SEARCH_MAX_LENGTH}
                aria-label="Search seasons"
                placeholder="Search"
                spellCheck={false}
                onChange={(event) => setQuery(event.target.value)}
                onKeyDown={(event) => {
                  if (event.key === "Escape") setQuery("");
                }}
                className={`${inputClasses} w-[200px]`}
              />,
              topbarSlot,
            )}
          {listContent}
        </>
      )}
    </div>
  );
}
