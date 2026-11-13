"use client";

import { useCallback, useEffect, useMemo, useRef, useState } from "react";
import { CheatEngineSetting } from "@/components/CheatEngineSetting";
import { Button, iconButton } from "@/components/Button";
import { ConfirmModal } from "@/components/ConfirmModal";
import { Modal, modalTitle } from "@/components/Modal";
import { PlaytimeModal } from "@/components/PlaytimeModal";
import { SelectHmModal } from "@/components/SelectHmModal";
import { DefaultLibraryIcon, RemoveIcon } from "@/components/icons";
import { ListRow, PickerRow } from "@/components/ui";
import { SaveCheck, TextSetting } from "@/components/SettingsControls";
import { ShearsModal } from "@/components/ShearsModal";
import { UninstallModal } from "@/components/UninstallModal";
import {
  editionActive,
  editionLaunching,
  editionQueued,
  editionRunning,
  onBridgeReady,
  shearsActions,
  useDownloader,
  useLaunch,
  useLibraries,
  usePlatform,
  usePlaytime,
  useProtons,
  useSettings,
  useShears,
  useUpdateBusy,
  type CheatEngineStatus,
  type SeasonInstalls,
  type Season,
  type ShearsKind,
  type ShearsScan,
} from "@/lib/bridge";
import { showToast } from "@/lib/toast";

type SeasonModal =
  | { kind: "shears" }
  | { kind: "uninstall" }
  | { kind: "selectHm"; then: () => void }
  | { kind: "download" }
  | { kind: "removeDownload" }
  | { kind: "proton" }
  | { kind: "playtime" };

const NO_INSTALL: SeasonInstalls["tb"] = {
  installed: false,
  partial: false,
  built: 0,
  crashLog: false,
};

export function useSeasonManage(
  season: Season,
  hm: boolean,
  {
    scanShears = false,
    onLog,
    onLogHistory,
    onClearLog,
    onUninstalled,
  }: {
    scanShears?: boolean;
    onLog?: (line: string) => void;
    onLogHistory?: (history: string) => void;
    onClearLog?: () => void;
    onUninstalled?: (otherInstalled: boolean) => void;
  } = {},
) {
  const [modal, setModal] = useState<SeasonModal | null>(null);
  const [dlLibrary, setDlLibrary] = useState("");
  const [argsSaved, setArgsSaved] = useState(0);
  const [protons, reloadProtons] = useProtons();
  const [ceStatus, setCeStatus] = useState<CheatEngineStatus | null>(null);
  const [installs, setInstalls] = useState<SeasonInstalls>({
    tb: NO_INSTALL,
    hm: NO_INSTALL,
    prefix: false,
  });
  const [loaded, setLoaded] = useState(false);
  const [checkingHm, setCheckingHm] = useState(false);
  const [shearsScan, setShearsScan] = useState<ShearsScan | null>(null);
  const lc = useLaunch();
  const platform = usePlatform();
  const settings = useSettings();
  const shears = useShears();
  const updateBusy = useUpdateBusy();
  const [libraryEntries, refreshLibraries] = useLibraries();
  const libs = useMemo(() => libraryEntries ?? [], [libraryEntries]);
  const multiLib = libs.length > 1;
  const hmActive = hm && season.hmAvailable;
  const playtimeEntries = usePlaytime();
  const playtime =
    playtimeEntries?.find(
      (entry) => entry.key === season.key && entry.hm === hmActive,
    )?.minutes ?? 0;
  const anyPlaytime = (playtimeEntries?.length ?? 0) > 0;
  const cheatEngine = !hmActive && installs.tb.installed;
  const shearsCuts = useMemo(() => shearsActions(shearsScan), [shearsScan]);

  const { installs: fetchInstalls } = lc;
  const refresh = useCallback(() => {
    fetchInstalls(season.key, (result) => {
      setInstalls(result);
      setLoaded(true);
    });
    if (scanShears && shears.ready)
      shears.scan(season.key, (result) => {
        if (result.ok) setShearsScan(result.scan);
        else showToast(result.message);
      });
  }, [fetchInstalls, season.key, scanShears, shears]);

  const dl = useDownloader({
    onLog: (line) => {
      if (dl.activeKey === season.key) onLog?.(line);
    },
    onLogHistory: (key, history) => {
      if (key === season.key) onLogHistory?.(history);
    },
    onDone: (key) => {
      if (key === season.key) refresh();
    },
    onPartialDeleted: (key) => {
      if (key === season.key) refresh();
    },
  });

  const activeSeason = dl.activeKey === season.key;
  const downloadingSeason = activeSeason && dl.running;
  const downloading = dl.running;
  const playingEdition = editionRunning(lc, season.key, hmActive);
  const launchingEdition = editionLaunching(lc, season.key, hmActive);
  const runningSeason = lc.running.some((ref) => ref.key === season.key);
  const playingSeason = runningSeason || lc.launching?.key === season.key;
  const queuedEdition = editionQueued(dl, season.key, hmActive);
  const editionInstall = hmActive ? installs.hm : installs.tb;
  const activeEdition = editionActive(dl, season.key, hmActive);
  const downloadingEdition = activeEdition && dl.running;
  const verifyingEdition = activeEdition && dl.verifying;
  const editionState = activeEdition ? dl.state : "idle";
  const transferring = downloadingEdition || editionState === "paused";
  const cancelRun = verifyingEdition || editionInstall.installed;
  if (
    (downloadingSeason || playingSeason) &&
    (modal?.kind === "shears" || modal?.kind === "uninstall")
  )
    setModal(null);

  useEffect(() => {
    if (!lc.ready) return;
    refresh();
    window.addEventListener("focus", refresh);
    return () => window.removeEventListener("focus", refresh);
  }, [lc.ready, refresh, playingSeason, runningSeason, downloadingEdition]);

  useEffect(() => {
    if (!settings) return;
    settings.libraries_changed.connect(refresh);
    return () => settings.libraries_changed.disconnect(refresh);
  }, [settings, refresh]);

  const prevLibPaths = useRef<string[] | null>(null);
  useEffect(() => {
    const paths = libs.map((library) => library.path);
    const prev = prevLibPaths.current;
    prevLibPaths.current = paths;
    if (modal?.kind !== "download" || !prev) return;
    const added = paths.find((path) => !prev.includes(path));
    if (added) setDlLibrary(added);
  }, [libs, modal]);

  const storedArgs = settings?.launch_args[season.key] ?? "";
  const seasonProton = settings?.season_proton[season.key] ?? "";
  const activeProton = protons?.some((p) => p.internal === seasonProton)
    ? seasonProton
    : settings?.proton;

  useEffect(() => {
    if (!settings) return;
    const onSaved = () => setArgsSaved((tick) => tick + 1);
    settings.launch_args_changed.connect(onSaved);
    return () => settings.launch_args_changed.disconnect(onSaved);
  }, [settings]);

  useEffect(() => {
    if (!settings) return;
    const onRemoved = () => {
      fetchInstalls(season.key, setInstalls);
      if (cheatEngine)
        onBridgeReady((bridge) =>
          bridge.cheatengine?.status(season.key, setCeStatus),
        );
    };
    settings.prefix_removed.connect(onRemoved);
    return () => settings.prefix_removed.disconnect(onRemoved);
  }, [settings, fetchInstalls, cheatEngine, season.key]);

  function ensureHm(then: () => void) {
    if (!hmActive || !season.hmBeta) {
      then();
      return;
    }
    setCheckingHm(true);
    dl.hmArchiveCached((cached) => {
      setCheckingHm(false);
      if (cached) then();
      else setModal({ kind: "selectHm", then });
    });
  }

  function startDownload(library = "") {
    setModal(null);
    ensureHm(() => {
      if (!downloading) onClearLog?.();
      dl.enqueue(season.key, hmActive, editionInstall.partial ? "" : library);
    });
  }

  function verify() {
    ensureHm(() => {
      if (!downloading) onClearLog?.();
      dl.verify(season.key, hmActive);
    });
  }

  function cut(kind: ShearsKind, level: number) {
    shears.cut(season.key, kind, level, (result) => {
      if (!result.ok) {
        showToast(result.message);
        refresh();
        return;
      }
      setShearsScan(result.scan);
      if (shearsActions(result.scan).length === 0) setModal(null);
    });
  }

  function openDownloadPrompt() {
    if (editionInstall.partial) {
      startDownload();
      return;
    }
    refreshLibraries();
    const library =
      (
        libs.find((entry) => entry.default && entry.exists) ??
        libs.find((entry) => entry.exists)
      )?.path ?? "";
    setDlLibrary(library);
    const chosen = libs.find((entry) => entry.default && !entry.fixed);
    if (multiLib && !chosen?.exists) setModal({ kind: "download" });
    else startDownload(library);
  }

  function openProton() {
    setCeStatus(null);
    onBridgeReady((bridge) =>
      bridge.cheatengine?.status(season.key, setCeStatus),
    );
    reloadProtons();
    setModal({ kind: "proton" });
  }

  function copyCrashLog(copied: () => void) {
    onBridgeReady((bridge) =>
      bridge.info.copy_crash_log(season.key, (error) => {
        if (!error) copied();
        else {
          showToast(error);
          refresh();
        }
      }),
    );
  }

  function openFolder() {
    onBridgeReady((bridge) => bridge.info.open_season(season.key, hmActive));
  }

  const modals = (
    <>
      {modal?.kind === "download" && (
        <Modal
          title={
            <span className="flex items-center justify-between gap-3">
              Download
              <code className="chip">{season.sizeGb} GB</code>
            </span>
          }
          onClose={() => setModal(null)}
          onConfirm={() => startDownload(dlLibrary)}
          footer={
            <>
              <Button variant="secondary" onClick={() => setModal(null)}>
                Cancel
              </Button>
              <Button
                variant="secondary"
                disabled={downloading}
                onClick={() => settings?.add_library()}
              >
                Add library
              </Button>
              <Button
                variant="primary"
                onClick={() => startDownload(dlLibrary)}
              >
                {downloading ? "Queue download" : "Download"}
              </Button>
            </>
          }
        >
          <div className="flex flex-col gap-2">
            {libs.map((library) => (
              <PickerRow
                key={library.path}
                label={library.display}
                title={library.path}
                selected={library.path === dlLibrary}
                disabled={!library.exists}
                strike={!library.exists}
                onSelect={() => setDlLibrary(library.path)}
              >
                {library.default && <DefaultLibraryIcon />}
              </PickerRow>
            ))}
          </div>
        </Modal>
      )}

      {modal?.kind === "removeDownload" && (
        <ConfirmModal
          title="Remove download"
          confirmLabel="Remove"
          confirmOnEnter={false}
          onConfirm={() => {
            setModal(null);
            dl.deletePartial(season.key, hmActive);
          }}
          onCancel={() => setModal(null)}
        >
          <p className="text-body text-text-muted">
            This permanently removes the files downloaded so far.
          </p>
        </ConfirmModal>
      )}

      {modal?.kind === "selectHm" && (
        <SelectHmModal
          onSelected={() => {
            setModal(null);
            modal.then();
          }}
          onClose={() => setModal(null)}
        />
      )}

      {modal?.kind === "proton" &&
        settings &&
        protons &&
        (!cheatEngine || ceStatus) && (
          <Modal
            title="Proton"
            onClose={() => setModal(null)}
            aside={
              <div className="flex flex-col gap-3">
                <h2 className={modalTitle}>Launch options</h2>
                <div className="flex flex-col gap-2">
                  <div className="relative">
                    <TextSetting
                      value={storedArgs}
                      placeholder="%command%"
                      onCommit={(draft) => {
                        if (draft.trim() !== storedArgs)
                          settings.set_launch_args(season.key, draft.trim());
                      }}
                    />
                    <SaveCheck confirm={argsSaved} />
                  </div>
                  {cheatEngine && ceStatus && (
                    <CheatEngineSetting
                      seasonKey={season.key}
                      status={ceStatus}
                      onStatus={setCeStatus}
                    />
                  )}
                  {installs.prefix && (
                    <ListRow label="Prefix">
                      <span className="-mr-1 flex shrink-0 items-center">
                        <button
                          type="button"
                          disabled={playingSeason || ceStatus?.busy}
                          onClick={() => settings.remove_prefix(season.key)}
                          className={iconButton}
                        >
                          <RemoveIcon />
                        </button>
                      </span>
                    </ListRow>
                  )}
                </div>
              </div>
            }
          >
            <div className="flex flex-col gap-2">
              {protons.map((proton) => {
                const isDefault = proton.internal === settings.proton;
                return (
                  <PickerRow
                    key={proton.internal}
                    label={proton.display}
                    selected={proton.internal === activeProton}
                    onSelect={() => {
                      settings.set_season_proton(
                        season.key,
                        isDefault ? "" : proton.internal,
                      );
                      setModal(null);
                    }}
                  >
                    {isDefault && <code className="chip">Default</code>}
                  </PickerRow>
                );
              })}
            </div>
          </Modal>
        )}

      {modal?.kind === "playtime" && (
        <PlaytimeModal onClose={() => setModal(null)} />
      )}

      {modal?.kind === "shears" && (
        <ShearsModal
          actions={shearsCuts}
          onCut={cut}
          onClose={() => setModal(null)}
        />
      )}

      {modal?.kind === "uninstall" && (
        <UninstallModal
          season={season}
          hm={hmActive}
          onClose={() => setModal(null)}
          onDone={(ok) => {
            const otherInstalled = hmActive
              ? installs.tb.installed
              : installs.hm.installed;
            refresh();
            if (ok) onUninstalled?.(otherInstalled);
          }}
        />
      )}
    </>
  );

  return {
    dl,
    lc,
    platform,
    updateBusy,
    installs,
    loaded,
    checkingHm,
    hmActive,
    playtime,
    anyPlaytime,
    editionInstall,
    playingEdition,
    launchingEdition,
    playingSeason,
    downloadingSeason,
    downloading,
    queuedEdition,
    editionState,
    transferring,
    cancelRun,
    shearsCuts,
    protons,
    modal,
    setModal,
    ensureHm,
    verify,
    openDownloadPrompt,
    openProton,
    copyCrashLog,
    openFolder,
    modals,
  };
}
