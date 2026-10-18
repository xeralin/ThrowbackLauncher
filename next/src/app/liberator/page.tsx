"use client";

import {
  useEffect,
  useMemo,
  useState,
  useSyncExternalStore,
  useRef,
} from "react";
import { Button } from "@/components/Button";
import { LiberatorMissingModal } from "@/components/LiberatorMissingModal";
import { LiberatorStatus } from "@/components/LiberatorStatus";
import { CHEVRON_RIGHT, INFO, StrokeIcon } from "@/components/icons";
import {
  columnItem,
  columnItemActive,
  columnItemIdle,
  panel,
} from "@/components/ui";
import { SupportedSeasons } from "@/components/SupportedSeasons";
import { Switch } from "@/components/Switch";
import { Tabs, type TabItem } from "@/components/Tabs";
import {
  onBridgeEvent,
  type GametypeNode,
  type LiberatorCapabilities,
  useLiberator,
  useSettings,
} from "@/lib/bridge";

type SessionMods = Partial<Record<keyof LiberatorCapabilities, boolean>>;

type Session = { mods: SessionMods; path: number[]; lastPicked: string };

const SESSION_DEFAULT: Session = { mods: {}, path: [], lastPicked: "" };

let session = SESSION_DEFAULT;

let sessionStarted = false;
const sessionListeners = new Set<() => void>();

function setSession(next: Partial<Session>) {
  session = { ...session, ...next };
  for (const listener of sessionListeners) listener();
}

function subscribeSession(listener: () => void): () => void {
  if (!sessionStarted) {
    sessionStarted = true;
    onBridgeEvent("liberator", (event, args) => {
      if (event === "state" && !(args[0] as { attached: boolean }).attached)
        setSession(SESSION_DEFAULT);
    });
  }
  sessionListeners.add(listener);
  return () => sessionListeners.delete(listener);
}

type Mod = { key: keyof LiberatorCapabilities; label: string };

const PLAYERS_GROUP: Mod[] = [
  { key: "deathless", label: "Deathless Players and Hostage" },
  { key: "unlimitedEquip", label: "Unlimited Equipment" },
  { key: "unlimitedAmmo", label: "Unlimited Ammo" },
];

const MATCH_GROUP: Mod[] = [
  { key: "infiniteTime", label: "Infinite Match Time" },
  { key: "disableAI", label: "Brain-Dead AI" },
  { key: "displayBuild", label: "Display Build Number" },
];

const LOADOUT_GROUP: Mod[] = [
  { key: "disablePrimary", label: "Disable Primary Weapon" },
  { key: "disableSecondary", label: "Disable Secondary Weapon" },
  { key: "disablePrimaryGadget", label: "Disable Primary Gadget" },
  { key: "disableSecondaryGadget", label: "Disable Secondary Gadget" },
];

type TabId = "playlist" | "modifications" | "support";

function ModToggle({
  label,
  checked,
  disabled,
  onToggle,
}: {
  label: string;
  checked: boolean;
  disabled: boolean;
  onToggle: (value: boolean) => void;
}) {
  return (
    <div className="flex items-center gap-2 px-2 py-1.5">
      <span className="flex-1 text-ui text-text">{label}</span>
      <Switch
        label={label}
        checked={checked}
        onChange={onToggle}
        disabled={disabled}
      />
    </div>
  );
}

function PlaylistColumns({
  roots,
  path,
  lastPicked,
  onPick,
  onPath,
}: {
  roots: GametypeNode[];
  path: number[];
  lastPicked: string;
  onPick: (id: string) => void;
  onPath: (path: number[]) => void;
}) {
  const activePath =
    path.length === 0 && (roots[0]?.children.length ?? 0) > 0 ? [0] : path;

  const columns: GametypeNode[][] = [roots];
  let current: GametypeNode[] = roots;
  for (const index of activePath) {
    const next = current[index];
    if (!next || next.children.length === 0) break;
    columns.push(next.children);
    current = next.children;
  }

  function leaf(col: number, id: string) {
    onPath(activePath.slice(0, col));
    onPick(id);
  }

  return (
    <div className="flex h-full overflow-x-auto">
      {columns.map((nodes, col) => (
        <ul
          key={col}
          className="min-w-[150px] flex-1 overflow-y-auto border-r border-border p-2 last:border-r-0"
        >
          {nodes.map((node, index) => {
            const branch = node.children.length > 0;
            const active = branch && activePath[col] === index;
            const chosen = !branch && node.id === lastPicked;
            return (
              <li key={`${col}:${index}`}>
                <button
                  type="button"
                  onClick={() =>
                    branch
                      ? onPath([...activePath.slice(0, col), index])
                      : leaf(col, node.id)
                  }
                  className={`${columnItem} ${
                    chosen
                      ? "bg-action-dim text-text"
                      : active
                        ? columnItemActive
                        : columnItemIdle
                  }`}
                >
                  <span className="min-w-0 flex-1 truncate-fade">
                    {node.text}
                  </span>
                  {branch && (
                    <StrokeIcon
                      d={CHEVRON_RIGHT}
                      className="size-3.5 shrink-0"
                    />
                  )}
                </button>
              </li>
            );
          })}
        </ul>
      ))}
    </div>
  );
}

export default function LiberatorPage() {
  const settings = useSettings();
  const lib = useLiberator();

  const {
    mods,
    path: columnPath,
    lastPicked,
  } = useSyncExternalStore(
    subscribeSession,
    () => session,
    () => session,
  );
  const [tab, setTab] = useState<TabId>("support");
  const [missingClosed, setMissingClosed] = useState(false);
  const [pageShown, setPageShown] = useState(false);

  const enabled = settings?.liberator_enabled ?? true;
  const caps = lib.capabilities;
  const controlsEnabled = lib.unlocked && !!caps.fullFeature;
  const playlistReady = controlsEnabled && !!lib.tree;
  const modsReady =
    controlsEnabled &&
    ([...PLAYERS_GROUP, ...MATCH_GROUP, ...LOADOUT_GROUP].some(
      (mod) => caps[mod.key],
    ) ||
      !!caps.endRound ||
      !!caps.endMatch);
  const [prevPlaylistReady, setPrevPlaylistReady] = useState(playlistReady);
  const focusInTabs = useRef(false);

  if (
    (tab === "playlist" && !controlsEnabled) ||
    (tab === "modifications" && !modsReady)
  )
    setTab("support");

  if (playlistReady !== prevPlaylistReady) {
    setPrevPlaylistReady(playlistReady);
    if (playlistReady && tab !== "modifications") setTab("playlist");
  }

  useEffect(() => {
    function onFocusIn(event: FocusEvent) {
      const target = event.target;
      focusInTabs.current =
        target instanceof HTMLElement &&
        !!target.closest('[role="tablist"], [role="tabpanel"]');
    }
    document.addEventListener("focusin", onFocusIn);
    return () => document.removeEventListener("focusin", onFocusIn);
  }, []);

  useEffect(() => {
    if (focusInTabs.current && document.activeElement === document.body)
      document.getElementById(`tab-${tab}`)?.focus();
  }, [tab]);

  useEffect(() => {
    let alive = true;
    const animations = document.querySelector("main")?.getAnimations() ?? [];
    Promise.allSettled(animations.map((animation) => animation.finished)).then(
      () => {
        if (alive) setPageShown(true);
      },
    );
    return () => {
      alive = false;
    };
  }, []);

  const tabs: TabItem<TabId>[] = useMemo(
    () => [
      {
        id: "playlist",
        label: "Playlist",
        icon: (
          <StrokeIcon
            d="M4 3h5a1 1 0 0 1 1 1v5a1 1 0 0 1-1 1H4a1 1 0 0 1-1-1V4a1 1 0 0 1 1-1zM4 14h5a1 1 0 0 1 1 1v5a1 1 0 0 1-1 1H4a1 1 0 0 1-1-1v-5a1 1 0 0 1 1-1zM14 4h7M14 9h7M14 15h7M14 20h7"
            className="size-3.5"
          />
        ),
        disabled: !playlistReady,
      },
      {
        id: "modifications",
        label: "Modifications",
        icon: (
          <StrokeIcon
            d="M20 7h-9M14 17H5M20 17a3 3 0 1 1-6 0 3 3 0 0 1 6 0zM10 7a3 3 0 1 1-6 0 3 3 0 0 1 6 0z"
            className="size-3.5"
          />
        ),
        disabled: !modsReady,
      },
      {
        id: "support",
        label: "Support",
        icon: <StrokeIcon d={INFO} className="size-3.5" />,
      },
    ],
    [modsReady, playlistReady],
  );

  function toggleMod(key: keyof LiberatorCapabilities, checked: boolean) {
    setSession({ mods: { ...session.mods, [key]: checked } });
    lib.setMod(key, checked);
  }

  function pickGametype(id: string) {
    setSession({ lastPicked: id });
    lib.setPlaylist(id);
  }

  const renderGroup = (group: Mod[]) => (
    <div className={panel}>
      <div className="p-2">
        {group.map((mod) => (
          <ModToggle
            key={mod.key}
            label={mod.label}
            checked={!!mods[mod.key]}
            disabled={!caps[mod.key]}
            onToggle={(value) => toggleMod(mod.key, value)}
          />
        ))}
      </div>
    </div>
  );

  return (
    <div className="flex h-[calc(100dvh_-_var(--topbar-h)_-_2*var(--page-pad))] flex-col">
      <Tabs
        tabs={tabs}
        active={tab}
        onSelect={setTab}
        trailing={
          <span className="flex items-center gap-2.5">
            <LiberatorStatus lib={lib} enabled={enabled} />
            <Switch
              label="Liberator"
              checked={enabled}
              onChange={(value) => settings?.set_liberator_enabled(value)}
            />
          </span>
        }
      />

      <div
        role="tabpanel"
        id={`tabpanel-${tab}`}
        aria-labelledby={`tab-${tab}`}
        className="mt-4 min-h-0 flex-1 overflow-y-auto"
      >
        {tab === "modifications" && (
          <div className="max-w-[720px]">
            <div className="grid grid-cols-2 gap-4 max-cards:grid-cols-1">
              <div className="flex flex-col gap-4">
                {renderGroup(PLAYERS_GROUP)}
                {renderGroup(MATCH_GROUP)}
              </div>

              <div className="flex flex-col gap-4">
                {renderGroup(LOADOUT_GROUP)}
                <div className={panel}>
                  <div className="flex gap-2 p-4">
                    <Button
                      variant="secondary"
                      className="flex-1"
                      disabled={!caps.endRound}
                      onClick={lib.endRound}
                    >
                      End round
                    </Button>
                    <Button
                      variant="secondary"
                      className="flex-1"
                      disabled={!caps.endMatch}
                      onClick={lib.endMatch}
                    >
                      End match
                    </Button>
                  </div>
                </div>
              </div>
            </div>
          </div>
        )}

        {tab === "playlist" && lib.tree && (
          <div className={`h-full overflow-hidden ${panel}`}>
            <PlaylistColumns
              roots={lib.tree}
              path={columnPath}
              lastPicked={lastPicked}
              onPick={pickGametype}
              onPath={(path) => setSession({ path })}
            />
          </div>
        )}

        {tab === "support" && <SupportedSeasons />}
      </div>

      {pageShown &&
        settings?.liberator_enabled &&
        !lib.available &&
        !missingClosed && (
          <LiberatorMissingModal
            folder={settings.launcher_folder}
            onRestore={lib.restore}
            onClose={() => setMissingClosed(true)}
          />
        )}
    </div>
  );
}
