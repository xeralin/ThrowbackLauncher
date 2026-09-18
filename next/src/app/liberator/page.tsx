"use client";

import {
  useEffect,
  useMemo,
  useState,
  useSyncExternalStore,
  useRef,
} from "react";
import { Button } from "@/components/Button";
import { InfoHint } from "@/components/InfoHint";
import { CHEVRON_RIGHT, StrokeIcon } from "@/components/icons";
import { heading, panel } from "@/components/ui";
import {
  SupportedSeasons,
  type SupportView,
} from "@/components/SupportedSeasons";
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

const session = {
  mods: {} as SessionMods,
  path: [] as number[],
};

let sessionStarted = false;
const sessionListeners = new Set<() => void>();

function setSession(mods: SessionMods, path: number[]) {
  session.mods = mods;
  session.path = path;
  for (const listener of sessionListeners) listener();
}

function subscribeSession(listener: () => void): () => void {
  if (!sessionStarted) {
    sessionStarted = true;
    onBridgeEvent("liberator", (event, args) => {
      if (event === "state" && !(args[0] as { attached: boolean }).attached)
        setSession({}, []);
    });
  }
  sessionListeners.add(listener);
  return () => sessionListeners.delete(listener);
}

type Mod = { key: keyof LiberatorCapabilities; label: string; hint?: string };

const PLAYERS_GROUP: Mod[] = [
  {
    key: "deathless",
    label: "Deathless Players and Hostage",
    hint: "Players and the hostage survive most damage.",
  },
  {
    key: "unlimitedEquip",
    label: "Unlimited Equipment",
    hint: "Gives every player unlimited equipment and reinforcements.",
  },
  {
    key: "unlimitedAmmo",
    label: "Unlimited Ammo",
    hint: "Gives every player unlimited ammo with no reloading. Enable it before spawning. It can be buggy.",
  },
];

const MATCH_GROUP: Mod[] = [
  { key: "infiniteTime", label: "Infinite Match Time" },
  {
    key: "disableAI",
    label: "Brain-Dead AI",
    hint: "Enable this before the match starts.",
  },
  {
    key: "displayBuild",
    label: "Display Build Number",
    hint: "Displays the build number in R6S. Toggle the Display Mode once in the R6S Options menu to make it appear.",
  },
];

const LOADOUT_GROUP: Mod[] = [
  { key: "disablePrimary", label: "Disable Primary Weapon" },
  { key: "disableSecondary", label: "Disable Secondary Weapon" },
  { key: "disablePrimaryGadget", label: "Disable Primary Gadget" },
  { key: "disableSecondaryGadget", label: "Disable Secondary Gadget" },
];

type TabId = "playlist" | "modifications" | SupportView;

function ModToggle({
  label,
  checked,
  disabled,
  onToggle,
  hint,
}: {
  label: string;
  checked: boolean;
  disabled: boolean;
  onToggle: (value: boolean) => void;
  hint?: string;
}) {
  return (
    <div className="flex items-center gap-2 px-2 py-1.5">
      <span className="flex flex-1 items-center gap-1.5 text-ui text-text">
        {label}
        {hint && <InfoHint text={hint} />}
      </span>
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
                  className={`flex w-full items-center gap-1.5 rounded-md border-l-2 border-transparent px-2 py-1 text-left text-ui font-semibold transition-colors ${
                    chosen
                      ? "bg-action-dim text-text"
                      : active
                        ? "bg-surface-2 text-text"
                        : "text-text-muted hover:bg-surface-2 hover:text-text"
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

  const mods = useSyncExternalStore(
    subscribeSession,
    () => session.mods,
    () => session.mods,
  );
  const columnPath = useSyncExternalStore(
    subscribeSession,
    () => session.path,
    () => session.path,
  );
  const [lastPicked, setLastPicked] = useState("");
  const [tab, setTab] = useState<TabId>("full");

  const enabled = settings?.liberator_enabled ?? true;
  const caps = lib.capabilities;
  const controlsEnabled = lib.applied && !!caps.fullFeature;
  const [prevControlsEnabled, setPrevControlsEnabled] =
    useState(controlsEnabled);
  const focusInTabs = useRef(false);

  if (controlsEnabled !== prevControlsEnabled) {
    setPrevControlsEnabled(controlsEnabled);
    if (controlsEnabled) setTab("playlist");
    else if (tab !== "unlock") setTab("full");
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
    if (controlsEnabled) return;
    if (focusInTabs.current && document.activeElement === document.body)
      document.getElementById("tab-full")?.focus();
  }, [controlsEnabled]);

  useEffect(() => {
    if (!lastPicked) return;
    const timer = setTimeout(() => setLastPicked(""), 5000);
    return () => clearTimeout(timer);
  }, [lastPicked]);

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
        disabled: !controlsEnabled,
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
        disabled: !controlsEnabled,
      },
      {
        id: "full",
        label: "Support",
        icon: (
          <StrokeIcon
            d="M22 12a10 10 0 1 1-20 0 10 10 0 0 1 20 0zM12 16v-4M12 8h.01"
            className="size-3.5"
          />
        ),
      },
      {
        id: "unlock",
        label: "Unlock All",
        icon: (
          <StrokeIcon
            d="M5 10h14a2 2 0 0 1 2 2v8a2 2 0 0 1-2 2H5a2 2 0 0 1-2-2v-8a2 2 0 0 1 2-2zM7 10V7a5 5 0 0 1 9.33-2.5"
            className="size-3.5"
          />
        ),
      },
    ],
    [controlsEnabled],
  );

  function toggleMod(key: keyof LiberatorCapabilities, checked: boolean) {
    setSession({ ...mods, [key]: checked }, columnPath);
    lib.setMod(key, checked);
  }

  function pickGametype(id: string) {
    setLastPicked(id);
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
            disabled={!controlsEnabled || !caps[mod.key]}
            onToggle={(value) => toggleMod(mod.key, value)}
            hint={mod.hint}
          />
        ))}
      </div>
    </div>
  );

  return (
    <div className="flex h-full flex-col">
      <Tabs
        tabs={tabs}
        active={tab}
        onSelect={setTab}
        trailing={
          <span className={`flex items-center gap-2.5 ${heading}`}>
            <span>
              {!enabled ? (
                <span className="text-text-muted">Disabled</span>
              ) : !lib.available ? (
                <span className="text-text-muted">Liberator.exe missing</span>
              ) : lib.attached ? (
                lib.status || "Attached"
              ) : (
                "Waiting for R6S to launch"
              )}
            </span>
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
        className="mt-4 min-h-0 flex-1"
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
                      disabled={!controlsEnabled || !caps.endRound}
                      onClick={lib.endRound}
                    >
                      End round
                    </Button>
                    <Button
                      variant="secondary"
                      className="flex-1"
                      disabled={!controlsEnabled || !caps.endMatch}
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

        {tab === "playlist" && controlsEnabled && lib.tree && (
          <div className={`h-full overflow-hidden ${panel}`}>
            <PlaylistColumns
              roots={lib.tree}
              path={columnPath}
              lastPicked={lastPicked}
              onPick={pickGametype}
              onPath={(path) => setSession(mods, path)}
            />
          </div>
        )}

        {(tab === "full" || tab === "unlock") && (
          <div className="h-full overflow-y-auto">
            <SupportedSeasons view={tab} />
          </div>
        )}
      </div>
    </div>
  );
}
