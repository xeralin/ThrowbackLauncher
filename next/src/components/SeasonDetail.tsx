"use client";

import { Fragment, useDeferredValue, useEffect, useRef, useState } from "react";
import { BackHeading } from "@/components/BackHeading";
import { Button, iconButton } from "@/components/Button";
import { CopyButton } from "@/components/CopyButton";
import {
  FolderIcon,
  MarkIcon,
  PROTON,
  RemoveIcon,
  StrokeIcon,
} from "@/components/icons";
import { LogBox, type LogLine } from "@/components/LogBox";
import { SeasonInfo } from "@/components/SeasonInfo";
import { iconBox } from "@/components/ui";
import { CardKeyArt } from "@/components/SeasonKeyArt";
import { RendererMenu } from "@/components/RendererMenu";
import { useSeasonManage } from "@/components/SeasonManage";
import { OptionGroup, Tabs, type TabItem } from "@/components/Tabs";
import { TransferBar } from "@/components/TransferBar";
import {
  seasonTitle,
  useHomeSeasons,
  type GameArgs,
  type Season,
} from "@/lib/bridge";
import { hmInfo, SEASON_INFO } from "@/config/season-info";
import { site } from "@/config/site";
import { showToast } from "@/lib/toast";

const LOG_CAP = 1000;

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

const TABS: TabItem<TabId>[] = [
  { id: "manage", label: "Manage" },
  { id: "info", label: "Info" },
];

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
  const [gameArgs, setGameArgs] = useState<GameArgs>({
    args: "",
    renderer: "",
  });
  const [tab, setTab] = useState<TabId>("manage");
  const [homeSeasons] = useHomeSeasons();
  const deferredLog = useDeferredValue(log);
  const seededRef = useRef(false);
  const {
    dl,
    lc,
    platform,
    updateBusy,
    installs,
    hmActive,
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
    lockedOps,
    shearsCuts,
    protons,
    setModal,
    ensureHm,
    verify,
    openDownloadPrompt,
    openProton,
    copyCrashLog,
    openFolder,
    modals,
  } = useSeasonManage(season, hm, {
    scanShears: true,
    onLog: (line) =>
      setLog((prev) =>
        [
          ...prev,
          ...line.split("\n").map((text) => ({ id: logId.current++, text })),
        ].slice(-LOG_CAP),
      ),
    onLogHistory: (history) =>
      setLog(
        history
          ? history
              .split("\n")
              .slice(-LOG_CAP)
              .map((text) => ({ id: logId.current++, text }))
          : [],
      ),
    onClearLog: () => setLog([]),
    onUninstalled: (otherInstalled) => {
      const otherSeasons = (homeSeasons ?? []).some(
        (s) => s.key !== season.key,
      );
      if (!otherInstalled && otherSeasons) onBack();
    },
  });

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

  const { gameArgs: fetchGameArgs } = lc;
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
    return queuedEdition ? (
      <Button
        variant="secondary"
        onClick={() => dl.dequeue(season.key, hmActive)}
      >
        Remove from queue
      </Button>
    ) : (
      <Button variant="secondary" disabled={playingSeason} onClick={verify}>
        {downloading ? "Queue verify" : "Verify"}
      </Button>
    );
  }

  const transferActions = (
    <Fragment key="transfer">
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
    </Fragment>
  );

  const info = SEASON_INFO[season.key];

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
            preload
          />
        </div>

        <Tabs
          tabs={TABS}
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
                  <Fragment key="installed">
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
                  </Fragment>
                ) : queuedEdition ? (
                  <Button
                    variant="secondary"
                    onClick={() => dl.dequeue(season.key, hmActive)}
                  >
                    Remove from queue
                  </Button>
                ) : (
                  <Fragment key="partial">
                    <Button
                      variant="primary"
                      disabled={editionInstall.partial && playingSeason}
                      onClick={() => {
                        if (lockedOps) setModal({ kind: "lockedOperators" });
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
                            else ensureHm(() => dl.switchToHm(season.key));
                          }}
                        >
                          {hmActive ? "Switch to HM" : "Switch to TB"}
                        </Button>
                      )}
                  </Fragment>
                )}
              </span>
              <span className="flex min-w-0 flex-1 items-center gap-3">
                <TransferBar state={editionState} />
              </span>
              {!transferring && editionInstall.installed && (
                <span className={`${iconBox} bg-surface`}>
                  {hmActive && (
                    <CopyButton
                      label="Copy crash log"
                      copiedLabel="Crash log copied"
                      disabled={!installs.hm.crashLog}
                      onCopy={copyCrashLog}
                    />
                  )}
                  {platform === "linux" && (
                    <button
                      type="button"
                      aria-label="Proton"
                      disabled={protons?.length === 0}
                      onClick={openProton}
                      className={iconButton}
                    >
                      <StrokeIcon d={PROTON} />
                    </button>
                  )}
                  <button
                    type="button"
                    aria-label="Open folder"
                    onClick={openFolder}
                    className={iconButton}
                  >
                    <FolderIcon />
                  </button>
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
              <SeasonInfo
                entry={
                  hmActive
                    ? {
                        ...hmInfo(season.hmBeta),
                        release:
                          season.hmBeta && installs.hm.built
                            ? new Date(
                                installs.hm.built * 1000,
                              ).toLocaleDateString("en-US", {
                                month: "short",
                                day: "numeric",
                                year: "numeric",
                                timeZone: "UTC",
                              })
                            : info.release,
                        requirements: [
                          platform === "windows" &&
                            `Latest [Visual C++ Redistributable](${site.vcRedistUrl})`,
                          season.key === "Y5S3_ShadowLegacy" &&
                            "Medium or above in-game textures",
                          platform === "windows"
                            ? "External overlays like **Overwolf** disabled"
                            : "External overlays disabled",
                          season.hmBeta &&
                            `Latest \`Unstable.7z\` from [\`#indev-releases\`](${site.heatedMetalDiscordUrl})`,
                        ],
                      }
                    : info
                }
                build={season.build}
                sizeGb={season.sizeGb}
              />
            </div>
          ) : (
            deferredLog.length > 0 &&
            editionState !== "idle" && (
              <div className="mt-3 flex min-h-0 flex-1 flex-col">
                <LogBox lines={deferredLog} />
              </div>
            )
          )}
        </div>
      </div>

      {modals}
    </>
  );
}
