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
import { Modal, modalTitle } from "@/components/Modal";
import { ExternalLink } from "@/components/ExternalLink";
import {
  DefaultLibraryIcon,
  FolderIcon,
  MarkIcon,
  RemoveIcon,
  StrokeIcon,
} from "@/components/icons";
import { type LogLine } from "@/components/LogBox";
import { SeasonInfo } from "@/components/SeasonInfo";
import { PickerRow, iconBox, link } from "@/components/ui";
import { SaveCheck, TextSetting } from "@/components/SettingsControls";
import { CardKeyArt } from "@/components/SeasonKeyArt";
import { ShearsModal } from "@/components/ShearsModal";
import { RendererMenu } from "@/components/RendererMenu";
import { ExclusionSteps, ProtonSteps } from "@/components/SetupSteps";
import { OptionGroup, Tabs, type TabItem } from "@/components/Tabs";
import { TransferBar, TransferPanel } from "@/components/TransferPanel";
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
  type GameArgs,
  type ProtonOption,
  type Season,
  type ShearsKind,
  type ShearsScan,
} from "@/lib/bridge";
import { HM_INFO, SEASON_INFO } from "@/config/season-info";
import { site } from "@/config/site";
import { operatorsLocked } from "@/lib/seasons";
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

const RENDERERS = {
  vulkan: [
    { arg: "", label: "DirectX" },
    { arg: "/vulkan", label: "Vulkan" },
  ],
  dx11: [
    { arg: "", label: "DirectX 12" },
    { arg: "/dx11", label: "DirectX 11" },
  ],
};

const RENDERER_ARGS = Object.values(RENDERERS)
  .flat()
  .map((option) => option.arg)
  .filter(Boolean);

type TabId = "manage" | "info";

type SeasonModal =
  | { kind: "shears" }
  | { kind: "uninstall"; hm: boolean }
  | { kind: "hmArchive" }
  | { kind: "download" }
  | { kind: "lockedOperators" }
  | { kind: "removeDownload"; hm: boolean }
  | { kind: "proton" };

const EDITION_TABS: TabItem<"tb" | "hm">[] = [
  {
    id: "tb",
    label: "Throwback",
    icon: (
      <MarkIcon
        d="M18.17 20.21c-2.26 0.06 -7.41 -0.13 -12.66 -3.21c-0.78 -0.46 -1.88 -1.16 -3.11 -2.19l-0.42 -0.35l0.09 -0.53c0.15 -0.95 0.44 -1.86 0.87 -2.71c-0.48 0.14 -0.96 0.28 -1.44 0.42c0.41 -0.8 1.03 -1.78 1.96 -2.74c0.39 -0.4 0.77 -0.74 1.14 -1.03c-0.43 -0.07 -0.86 -0.15 -1.3 -0.22c0.94 -0.87 1.81 -1.4 2.43 -1.72c1.06 -0.55 2.12 -0.87 2.64 -1.02c0.14 -0.04 0.26 -0.08 0.34 -0.1c-0.33 -0.34 -0.66 -0.67 -0.99 -1.01c0.91 0.07 1.86 0.17 2.83 0.29c1.1 0.14 2.16 0.29 3.16 0.47c0.81 0.4 1.62 0.79 2.43 1.19l0.14 0.24s0.08 0.13 0.08 0.14c0.91 0.18 1.81 0.44 2.65 0.76c1.68 0.65 2.26 1.21 2.6 1.67c0.27 0.37 0.48 0.78 0.63 1.23l0.16 0.66c0.03 0.22 0.06 0.43 0.1 0.65c-0.09 0.01 -2.3 0.17 -2.39 0.18c-0.03 0 -1.3 0.1 -1.33 0.1c-0.37 -0.02 -0.75 -0.03 -1.13 -0.04c-0.3 -0.01 -0.61 -0.01 -0.92 -0.01h-0.01l-0.87 0.55c-0.33 0.45 -0.47 0.87 -0.53 1.18c-0.28 1.46 0.68 3.1 2.38 4.23c0.59 0.37 1.18 0.74 1.77 1.11c-0.2 0.06 -0.42 0.15 -0.66 0.26c-0.15 0.07 -0.28 0.14 -0.4 0.21c0.66 0.4 1.31 0.8 1.97 1.2c-0.49 0.06 -1.26 0.14 -2.21 0.16Z"
        className="size-3.5"
      />
    ),
  },
  {
    id: "hm",
    label: "Heated Metal",
    icon: (
      <MarkIcon
        d="M12.93 2.09 16.24 2.09 19.31 7.69 17.38 10.56ZM8.0 13.28 12.97 4.85 15.01 8.43 11.98 13.56ZM13.48 12.69 15.45 9.3 20.33 17.97 16.16 17.97ZM1.5 16.47 4.69 11.03 8.36 11.15 3.47 19.55ZM7.41 14.38 13.28 14.42 15.09 18.01 5.32 17.93ZM13.16 18.84 22.5 18.84 21.04 21.91 14.62 21.91Z"
        className="size-3.5"
      />
    ),
  },
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
  const [protons, setProtons] = useState<ProtonOption[] | null>(null);
  const [gameArgs, setGameArgs] = useState<GameArgs>({
    args: "",
    renderer: "",
  });
  const [tab, setTab] = useState<TabId>("manage");
  const [installs, setInstalls] = useState<SeasonInstalls>({
    tb: { installed: false, partial: false, library: "", built: 0 },
    hm: { installed: false, partial: false, library: "", built: 0 },
  });
  const [shearsScan, setShearsScan] = useState<ShearsScan | null>(null);
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

  const { installs: fetchInstalls, gameArgs: fetchGameArgs } = lc;
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
    modal.kind !== "proton"
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
  const gameTokens = gameArgs.args.split(/\s+/).filter(Boolean);
  const renderers = gameArgs.renderer ? RENDERERS[gameArgs.renderer] : null;
  const activeRenderer =
    gameTokens.find((arg) => RENDERER_ARGS.includes(arg)) ?? "";

  useEffect(() => {
    if (lc.ready) fetchGameArgs(season.key, hmActive, setGameArgs);
  }, [lc.ready, fetchGameArgs, season.key, hmActive, installs]);

  function setRenderer(renderer: string) {
    const rest = gameTokens.filter((arg) => !RENDERER_ARGS.includes(arg));
    const args = (renderer ? [...rest, renderer] : rest).join(" ");
    lc.setGameArgs(season.key, hmActive, args, (error) => {
      if (error) {
        showToast(error);
        return;
      }
      setGameArgs((current) => ({ ...current, args }));
    });
  }

  useEffect(() => {
    if (modal?.kind === "proton" && settings)
      settings.proton_options(setProtons);
  }, [modal, settings]);

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

  function playButtons() {
    if (playingEdition || launchingEdition)
      return (
        <Button variant="primary" onClick={() => lc.stop(season.key)}>
          Stop
        </Button>
      );
    const play = (
      <Button
        variant="primary"
        disabled={downloadingSeason || updateBusy}
        onClick={() => lc.launch(season.key, hmActive)}
        className={renderers ? "rounded-r-none" : ""}
      >
        Play
      </Button>
    );
    return renderers ? (
      <RendererMenu
        options={renderers}
        active={activeRenderer}
        disabled={downloadingSeason || updateBusy}
        onSelect={setRenderer}
      >
        {play}
      </RendererMenu>
    ) : (
      play
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
                        Update HM
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
                          onClick={() => {
                            if (!hmActive) dl.removeHm(season.key);
                            else if (!season.hmBeta) dl.switchToHm(season.key);
                            else
                              dl.hmArchiveCached((cached) => {
                                if (cached) dl.switchToHm(season.key);
                                else setModal({ kind: "hmArchive" });
                              });
                          }}
                        >
                          {hmActive ? "Switch to HM" : "Switch to TB"}
                        </Button>
                      )}
                  </>
                )}
              </span>
              <span className="flex min-w-0 flex-1 items-center gap-3">
                <TransferBar active={transferring} state={editionState} />
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
                      aria-label="Proton"
                      onClick={() => setModal({ kind: "proton" })}
                      className={iconButton}
                    >
                      <StrokeIcon d="m4 17 6-6-6-6M12 19h8" />
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
                    release:
                      season.hmBeta && installs.hm.built
                        ? new Date(installs.hm.built * 1000).toLocaleDateString(
                            "en-US",
                            {
                              month: "short",
                              day: "numeric",
                              year: "numeric",
                              timeZone: "UTC",
                            },
                          )
                        : (info?.release ?? ""),
                    setup:
                      platform === "windows" ? (
                        <ExclusionSteps library={installs.hm.library} />
                      ) : season.key === "Y9S2_NewBlood" ? (
                        <ProtonSteps />
                      ) : undefined,
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
            Pick Striker and Sentry once you complete the tutorial.
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

      {modal?.kind === "hmArchive" && (
        <ConfirmModal
          title="Switch to Heated Metal"
          confirmLabel="Choose archive"
          onConfirm={() => {
            setModal(null);
            dl.switchToHm(season.key);
          }}
          onCancel={() => setModal(null)}
        >
          <p className="text-body text-text-muted">
            Download the <code>.7z</code> archive from{" "}
            <ExternalLink
              href={site.indevReleasesUrl}
              className={`${link} [&>code]:text-inherit`}
            >
              <code>#indev-releases</code>
            </ExternalLink>{" "}
            and choose it here.
          </p>
        </ConfirmModal>
      )}

      {modal?.kind === "proton" && settings && protons && (
        <Modal
          title="Proton"
          onClose={() => setModal(null)}
          footer={
            <div className="flex w-full flex-col gap-3">
              <h2 className={modalTitle}>Launch options</h2>
              <div className="relative">
                <TextSetting
                  value={storedArgs}
                  placeholder="%command%"
                  className="w-full pr-8"
                  onCommit={(draft) => {
                    if (draft.trim() !== storedArgs)
                      settings.set_launch_args(season.key, draft.trim());
                  }}
                />
                <SaveCheck confirm={argsSaved} />
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
