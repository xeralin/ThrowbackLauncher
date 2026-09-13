"use client";

import {
  useCallback,
  useDeferredValue,
  useEffect,
  useMemo,
  useRef,
  useState,
} from "react";
import { BackHeading } from "@/components/BackHeading";
import { Button, iconButton } from "@/components/Button";
import { ConfirmModal } from "@/components/ConfirmModal";
import { Modal } from "@/components/Modal";
import { ExternalLink } from "@/components/ExternalLink";
import {
  DefaultLibraryIcon,
  FolderIcon,
  RemoveIcon,
  TerminalIcon,
} from "@/components/icons";
import { type LogLine } from "@/components/LogBox";
import { Note } from "@/components/Note";
import { SeasonInfo } from "@/components/SeasonInfo";
import { PickerRow, iconBox } from "@/components/ui";
import { SaveCheck, TextSetting } from "@/components/SettingsControls";
import { CardKeyArt } from "@/components/SeasonKeyArt";
import { ShearsModal } from "@/components/ShearsModal";
import { OptionGroup, Tabs, type TabItem } from "@/components/Tabs";
import {
  TransferBar,
  TransferPanel,
  TransferPercent,
} from "@/components/TransferPanel";
import { UninstallModal } from "@/components/UninstallModal";
import {
  editionActive,
  editionLaunching,
  editionQueued,
  editionRunning,
  onBridgeReady,
  seasonLaunching,
  seasonRunning,
  seasonTitle,
  shearsActions,
  useDownloader,
  useHomeSeasons,
  useLaunch,
  useLibraries,
  usePlatform,
  useSettings,
  useShears,
  useUpdateBusy,
  type SeasonInstalls,
  type LibraryEntry,
  type Season,
  type ShearsKind,
  type ShearsScan,
} from "@/lib/bridge";
import { HM_INFO, SEASON_INFO } from "@/config/season-info";
import { site } from "@/config/site";
import { operatorsLocked, soloOperators } from "@/lib/seasons";
import { showToast } from "@/lib/toast";

const LOG_CAP = 1000;

function LibraryPicker({
  libraries,
  selected,
  onSelect,
}: {
  libraries: LibraryEntry[];
  selected: string;
  onSelect: (path: string) => void;
}) {
  return (
    <div className="flex flex-col gap-2">
      {libraries.map((library) => (
        <PickerRow
          key={library.path}
          label={library.display}
          title={library.path}
          selected={library.path === selected}
          disabled={!library.exists}
          strike={!library.exists}
          onSelect={() => onSelect(library.path)}
        >
          {library.default && <DefaultLibraryIcon />}
        </PickerRow>
      ))}
    </div>
  );
}

function BetaHint() {
  return (
    <p className="text-body text-text-muted">
      This build is only available on the Heated Metal Discord. Download the{" "}
      <code>.7z</code> archive from{" "}
      <ExternalLink
        href={site.indevReleasesUrl}
        className="text-link hover:underline [&>code]:text-inherit"
      >
        <code>#indev-releases</code>
      </ExternalLink>{" "}
      first, then choose it here.
    </p>
  );
}

const NO_INSTALLS: SeasonInstalls = {
  tb: { installed: false, partial: false },
  hm: { installed: false, partial: false },
};

type TabId = "manage" | "info";

type SeasonModal =
  | { kind: "shears" }
  | { kind: "uninstall"; hm: boolean }
  | { kind: "switch"; toHm: boolean }
  | { kind: "download" }
  | { kind: "lockedOperators" }
  | { kind: "removeDownload"; hm: boolean }
  | { kind: "launchArgs" };

const EDITION_TABS: TabItem<"tb" | "hm">[] = [
  { id: "tb", label: "Throwback" },
  { id: "hm", label: "Heated Metal" },
];

export function SeasonDetail({
  season,
  hm,
  onHmChange,
  onBack,
}: {
  season: Season;
  hm: boolean;
  onHmChange: (hm: boolean) => void;
  onBack: () => void;
}) {
  const [log, setLog] = useState<LogLine[]>([]);
  const logId = useRef(0);
  const [modal, setModal] = useState<SeasonModal | null>(null);
  const [dlLibrary, setDlLibrary] = useState("");
  const [argsSaved, setArgsSaved] = useState(0);
  const [tab, setTab] = useState<TabId>("manage");
  const [installs, setInstalls] = useState<SeasonInstalls>(NO_INSTALLS);
  const [shearsScan, setShearsScan] = useState<ShearsScan | null>(null);
  const [cutting, setCutting] = useState(false);
  const [homeSeasons] = useHomeSeasons();
  const deferredLog = useDeferredValue(log);
  const seededRef = useRef(false);
  const lc = useLaunch();
  const platform = usePlatform();
  const settings = useSettings();
  const shears = useShears();
  const updateBusy = useUpdateBusy();
  const [libraryEntries, refreshLibraries] = useLibraries();
  const libs = useMemo(() => libraryEntries ?? [], [libraryEntries]);
  const multiLib = libs.length > 1;
  const hmActive = hm && season.hmAvailable;
  const shearsCuts = useMemo(() => shearsActions(shearsScan), [shearsScan]);

  const { installs: fetchInstalls } = lc;
  const refresh = useCallback(() => {
    fetchInstalls(season.key, setInstalls);
    if (shears.ready)
      shears.scan(season.key, (result) => {
        if (result.ok) setShearsScan(result.scan);
        else showToast(result.message);
      });
  }, [fetchInstalls, season.key, shears]);

  const dl = useDownloader({
    onLog: (line) => {
      if (dl.activeKey !== season.key) return;
      setLog((prev) =>
        [
          ...prev,
          ...line.split("\n").map((text) => ({ id: logId.current++, text })),
        ].slice(-LOG_CAP),
      );
    },
    onLogHistory: (key, history) => {
      if (key !== season.key) return;
      setLog(
        history
          ? history
              .split("\n")
              .slice(-LOG_CAP)
              .map((text) => ({ id: logId.current++, text }))
          : [],
      );
    },
    onDone: () => refresh(),
    onPartialDeleted: (key) => {
      if (key === season.key) refresh();
    },
  });

  const activeSeason = dl.activeKey === season.key;
  const downloadingSeason = activeSeason && dl.running;
  const downloading = dl.running;
  const playingEdition = editionRunning(lc, season.key, hmActive);
  const launchingEdition = editionLaunching(lc, season.key, hmActive);
  const playingSeason =
    seasonRunning(lc, season.key) || seasonLaunching(lc, season.key);
  const queuedEdition = editionQueued(dl, season.key, hmActive);
  const verifyQueuedEdition = editionQueued(dl, season.key, hmActive, {
    verify: true,
  });
  const editionInstall = hmActive ? installs.hm : installs.tb;
  const activeEdition = editionActive(dl, season.key, hmActive);
  const downloadingEdition = activeEdition && dl.running;
  const verifyingEdition = activeEdition && dl.verifying;
  const editionState = activeEdition ? dl.state : "idle";
  const transferring = downloadingEdition || editionState === "paused";
  if (
    (downloadingSeason || playingSeason) &&
    modal !== null &&
    modal.kind !== "download" &&
    modal.kind !== "removeDownload" &&
    modal.kind !== "lockedOperators" &&
    modal.kind !== "launchArgs"
  )
    setModal(null);

  useEffect(() => {
    if (!dl.ready) return;
    if (dl.activeKey !== season.key) {
      seededRef.current = false;
      return;
    }
    if (!seededRef.current) {
      seededRef.current = true;
      dl.requestLog();
    }
  }, [dl, season.key]);

  useEffect(() => {
    if (lc.ready) refresh();
  }, [lc.ready, refresh]);

  useEffect(() => {
    if (!settings) return;
    const onLibraries = () => refresh();
    settings.libraries_changed.connect(onLibraries);
    return () => settings.libraries_changed.disconnect(onLibraries);
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

  useEffect(() => {
    if (!settings) return;
    const onSaved = () => setArgsSaved((tick) => tick + 1);
    settings.launch_args_changed.connect(onSaved);
    return () => settings.launch_args_changed.disconnect(onSaved);
  }, [settings]);

  function startDownload(library = "") {
    setModal(null);
    const lib = editionInstall.partial ? "" : library;
    if (downloading) {
      dl.enqueue(season.key, hmActive, lib);
      return;
    }
    setLog([]);
    dl.start(season.key, hmActive, lib);
  }

  function cut(kind: ShearsKind, level: number) {
    setCutting(true);
    shears.cut(season.key, kind, level, (result) => {
      setCutting(false);
      if (!result.ok) {
        showToast(result.message);
        return;
      }
      setShearsScan(result.scan);
      if (shearsActions(result.scan).length === 0) setModal(null);
    });
  }

  function preferredLibrary(): string {
    const preferred =
      libs.find((library) => library.default && library.exists) ??
      libs.find((library) => library.exists);
    return preferred?.path ?? "";
  }

  function openDownloadPrompt() {
    refreshLibraries();
    const library = preferredLibrary();
    setDlLibrary(library);
    if (multiLib) setModal({ kind: "download" });
    else startDownload(library);
  }

  function playButtons() {
    return playingEdition || launchingEdition ? (
      <Button variant="primary" onClick={() => lc.stop(season.key)}>
        Stop
      </Button>
    ) : (
      <Button
        variant="primary"
        disabled={downloadingSeason || updateBusy}
        onClick={() => lc.launch(season.key, hmActive)}
      >
        Play
      </Button>
    );
  }

  function verifyButton() {
    return verifyQueuedEdition ? (
      <Button
        variant="secondary"
        onClick={() => dl.dequeue(season.key, hmActive)}
      >
        Remove from queue
      </Button>
    ) : (
      <Button
        variant="secondary"
        disabled={playingSeason}
        onClick={() => {
          if (!downloading) setLog([]);
          dl.verify(season.key, hmActive);
        }}
      >
        {downloading ? "Queue verify" : "Verify"}
      </Button>
    );
  }

  const cancelRun = verifyingEdition || editionInstall.installed;
  const transferActions = (
    <>
      {editionState === "paused" ? (
        <Button variant="primary" onClick={() => dl.setPaused(false)}>
          Continue
        </Button>
      ) : (
        <Button
          variant="secondary"
          pulse={editionState !== "downloading"}
          onClick={() => (cancelRun ? dl.cancel() : dl.setPaused(true))}
        >
          {cancelRun ? "Cancel" : "Pause"}
        </Button>
      )}
      {editionState === "paused" && (
        <Button
          variant="secondary"
          onClick={() => setModal({ kind: "removeDownload", hm: hmActive })}
        >
          Remove
        </Button>
      )}
    </>
  );

  const lockedOps = !hmActive && operatorsLocked(season.key);

  const info = SEASON_INFO[season.key];
  const tabs: TabItem<TabId>[] = useMemo(
    () => [
      { id: "manage", label: "Manage" },
      ...(hmActive || info ? [{ id: "info" as const, label: "Info" }] : []),
    ],
    [hmActive, info],
  );

  return (
    <>
      <div
        className={
          tab === "info"
            ? "flex flex-col"
            : "flex h-[max(calc(100dvh_-_var(--topbar-h)_-_2*var(--page-pad)),30rem)] flex-col"
        }
      >
        <BackHeading title={seasonTitle(season, hmActive)} onBack={onBack} />

        <div className="relative mb-3 h-[214px] min-h-[160px] overflow-hidden rounded-lg border border-border">
          <CardKeyArt
            season={{ ...season, hm: hmActive }}
            sizes="100vw"
            priority
          />
        </div>

        <Tabs
          tabs={tabs}
          active={tab}
          onSelect={setTab}
          trailing={
            season.hmAvailable ? (
              <OptionGroup
                tabs={EDITION_TABS}
                active={hmActive ? "hm" : "tb"}
                onSelect={(id) => onHmChange(id === "hm")}
                label="Edition"
              />
            ) : undefined
          }
        />

        <div
          role="tabpanel"
          id={`tabpanel-${tab}`}
          aria-labelledby={`tab-${tab}`}
          className={
            tab === "info" ? undefined : "flex min-h-0 flex-1 flex-col"
          }
        >
          {tab !== "info" && (
            <div className="mt-4 flex items-center gap-3">
              <span className="flex flex-wrap items-center gap-2">
                {transferring ? (
                  transferActions
                ) : editionInstall.installed ? (
                  <>
                    {playButtons()}
                    {verifyButton()}
                    {hmActive && season.hmBeta && (
                      <Button
                        variant="secondary"
                        disabled={playingSeason || downloading}
                        onClick={() => dl.importHm(season.key)}
                      >
                        Replace HM files
                      </Button>
                    )}
                    {!hmActive && (
                      <Button
                        variant="secondary"
                        disabled={
                          downloadingSeason ||
                          playingSeason ||
                          shearsCuts.length === 0
                        }
                        onClick={() => setModal({ kind: "shears" })}
                      >
                        Shears
                      </Button>
                    )}
                  </>
                ) : queuedEdition ? (
                  <Button
                    variant="secondary"
                    onClick={() => dl.dequeue(season.key, hmActive)}
                  >
                    Remove from queue
                  </Button>
                ) : (
                  <>
                    <Button
                      variant="primary"
                      disabled={editionInstall.partial && playingSeason}
                      onClick={() => {
                        if (lockedOps) setModal({ kind: "lockedOperators" });
                        else if (editionInstall.partial) startDownload();
                        else openDownloadPrompt();
                      }}
                    >
                      {editionInstall.partial
                        ? downloading
                          ? "Queue verify"
                          : "Verify"
                        : downloading
                          ? "Queue download"
                          : "Download"}
                    </Button>
                    {editionInstall.partial && (
                      <Button
                        variant="secondary"
                        disabled={playingSeason}
                        onClick={() =>
                          setModal({ kind: "removeDownload", hm: hmActive })
                        }
                      >
                        Remove
                      </Button>
                    )}
                    {!editionInstall.partial &&
                      (hmActive
                        ? installs.tb.installed
                        : installs.hm.installed) && (
                        <Button
                          variant="secondary"
                          disabled={playingSeason || downloading}
                          onClick={() =>
                            setModal({ kind: "switch", toHm: hmActive })
                          }
                        >
                          {hmActive ? "Switch to HM" : "Switch to TB"}
                        </Button>
                      )}
                  </>
                )}
              </span>
              <span className="flex min-w-0 flex-1 items-center gap-3">
                <TransferBar active={transferring} state={editionState} />
                <TransferPercent state={editionState} />
              </span>
              {!transferring && editionInstall.installed && (
                <span className={`${iconBox} bg-surface`}>
                  <button
                    type="button"
                    aria-label="Open folder"
                    onClick={() =>
                      onBridgeReady((bridge) =>
                        bridge.info.open_season(season.key, hmActive),
                      )
                    }
                    className={iconButton}
                  >
                    <FolderIcon />
                  </button>
                  {platform === "linux" && (
                    <button
                      type="button"
                      aria-label="Launch options"
                      onClick={() => setModal({ kind: "launchArgs" })}
                      className={iconButton}
                    >
                      <TerminalIcon />
                    </button>
                  )}
                  <button
                    type="button"
                    aria-label="Uninstall"
                    disabled={downloadingSeason || playingSeason}
                    onClick={() =>
                      setModal({ kind: "uninstall", hm: hmActive })
                    }
                    className={iconButton}
                  >
                    <RemoveIcon />
                  </button>
                </span>
              )}
            </div>
          )}

          {tab === "info" ? (
            <div className="mt-4">
              {hmActive ? (
                <SeasonInfo
                  entry={{
                    ...HM_INFO,
                    release: info?.release ?? "",
                    note: season.hmBeta ? (
                      <>
                        The Heated Metal build comes from the{" "}
                        <ExternalLink href={site.indevReleasesUrl}>
                          <code>#indev-releases</code>
                        </ExternalLink>{" "}
                        channel on the Heated Metal Discord.
                      </>
                    ) : undefined,
                  }}
                  build={season.build}
                  sizeGb={season.sizeGb}
                />
              ) : (
                info && (
                  <SeasonInfo
                    entry={info}
                    build={season.build}
                    sizeGb={season.sizeGb}
                  />
                )
              )}
            </div>
          ) : (
            <TransferPanel
              lines={deferredLog}
              active={transferring}
              state={editionState}
            />
          )}
        </div>
      </div>

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
          <LibraryPicker
            libraries={libs}
            selected={dlLibrary}
            onSelect={setDlLibrary}
          />
        </Modal>
      )}

      {modal?.kind === "lockedOperators" && (
        <ConfirmModal
          title="Operators are locked"
          confirmLabel={editionInstall.partial ? "Verify" : "Download"}
          note="There is currently no fix."
          onConfirm={() => {
            setModal(null);
            if (editionInstall.partial) startDownload();
            else openDownloadPrompt();
          }}
          onCancel={() => setModal(null)}
        >
          <p className="text-body text-text-muted">
            Pick {soloOperators(season.key)} once you complete the tutorial.
          </p>
        </ConfirmModal>
      )}

      {modal?.kind === "removeDownload" && (
        <ConfirmModal
          title="Remove download"
          confirmLabel="Remove"
          confirmOnEnter={false}
          onConfirm={() => {
            const hm = modal.hm;
            setModal(null);
            dl.deletePartial(season.key, hm);
          }}
          onCancel={() => setModal(null)}
        >
          <p className="text-body text-text-muted">
            This permanently removes the files downloaded so far.
          </p>
        </ConfirmModal>
      )}

      {modal?.kind === "switch" && (
        <ConfirmModal
          title={modal.toHm ? "Switch to Heated Metal" : "Switch to Throwback"}
          confirmLabel={
            modal.toHm && season.hmBeta ? "Choose archive" : "Switch"
          }
          onConfirm={() => {
            const toHm = modal.toHm;
            setModal(null);
            if (toHm) dl.switchToHm(season.key);
            else dl.removeHm(season.key);
          }}
          onCancel={() => setModal(null)}
        >
          {modal.toHm && season.hmBeta ? (
            <BetaHint />
          ) : (
            <p className="text-body text-text-muted">
              {modal.toHm
                ? "This switches your Throwback install to Heated Metal."
                : "This switches your Heated Metal install to Throwback."}
            </p>
          )}
        </ConfirmModal>
      )}

      {modal?.kind === "launchArgs" && (
        <Modal
          title="Launch options"
          onClose={() => setModal(null)}
          footer={
            <Note className="mr-auto">
              Set arguments like <code>MANGOHUD=1 %command%</code>.
            </Note>
          }
        >
          <div className="relative">
            <TextSetting
              value={storedArgs}
              placeholder="%command%"
              autoFocus
              className="w-full pr-8"
              onCommit={(draft) => {
                if (draft.trim() !== storedArgs)
                  settings?.set_launch_args(season.key, draft.trim());
              }}
            />
            <SaveCheck confirm={argsSaved} />
          </div>
        </Modal>
      )}

      {modal?.kind === "shears" && (
        <ShearsModal
          actions={shearsCuts}
          busy={cutting}
          onCut={cut}
          onClose={() => setModal(null)}
        />
      )}

      {modal?.kind === "uninstall" && (
        <UninstallModal
          season={season}
          hm={modal.hm}
          onClose={() => setModal(null)}
          onDone={() => {
            const otherInstalled = modal.hm
              ? installs.tb.installed
              : installs.hm.installed;
            const otherSeasons = (homeSeasons ?? []).some(
              (s) => s.key !== season.key,
            );
            refresh();
            if (!otherInstalled && otherSeasons) onBack();
          }}
        />
      )}
    </>
  );
}
