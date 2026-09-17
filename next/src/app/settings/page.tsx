"use client";

import { useEffect, useState } from "react";
import Link from "next/link";
import { Button, iconButton } from "@/components/Button";
import { ConfirmModal } from "@/components/ConfirmModal";
import { Modal } from "@/components/Modal";
import { Note } from "@/components/Note";
import { ListRow, PickerRow, card, heading } from "@/components/ui";
import {
  AccentPicker,
  ColorBar,
  HexSetting,
  Row,
  SaveCheck,
  LevelStepper,
  Stepper,
  TextSetting,
} from "@/components/SettingsControls";
import { Switch } from "@/components/Switch";
import { RvpnCard } from "@/components/RvpnCard";
import { Tabs, type TabItem } from "@/components/Tabs";
import {
  BookmarkIcon,
  DefaultLibraryIcon,
  FolderIcon,
  RemoveIcon,
} from "@/components/icons";
import {
  BAR_PRESETS,
  DEFAULT_ACCENT,
  DEFAULT_FILL,
  DEFAULT_STRIPE,
} from "@/config/accents";
import { usePlatformView } from "@/lib/platform-view";
import {
  onBridgeReady,
  useDiskUsage,
  useDownloader,
  useLibraries,
  useSettings,
  type LibraryEntry,
  type ProtonOption,
} from "@/lib/bridge";

type TabId = "downloads" | "extra";

const TABS: TabItem<TabId>[] = [
  { id: "downloads", label: "Downloads" },
  { id: "extra", label: "Extra" },
];

export default function SettingsPage() {
  const settings = useSettings();
  const diskUsageGb = useDiskUsage();
  const [libraries] = useLibraries();
  const downloading = useDownloader().running;
  const [removeTarget, setRemoveTarget] = useState<LibraryEntry | null>(null);
  const [usernameSaved, setUsernameSaved] = useState(0);
  const [usernameRevert, setUsernameRevert] = useState(0);
  const [protons, setProtons] = useState<ProtonOption[] | null>(null);
  const [protonOpen, setProtonOpen] = useState(false);
  const [tab, setTab] = useState<TabId>("downloads");
  const platform = usePlatformView();

  useEffect(() => {
    if (platform === "linux" && settings) settings.proton_options(setProtons);
  }, [platform, settings]);

  useEffect(() => {
    if (!settings) return;
    const onUsernameSaved = () => setUsernameSaved((tick) => tick + 1);
    const onInvalid = (field: string) => {
      if (field === "username") setUsernameRevert((tick) => tick + 1);
    };
    settings.username_changed.connect(onUsernameSaved);
    settings.settings_error.connect(onInvalid);
    return () => {
      settings.username_changed.disconnect(onUsernameSaved);
      settings.settings_error.disconnect(onInvalid);
    };
  }, [settings]);

  return (
    <>
      <Tabs tabs={TABS} active={tab} onSelect={setTab} />

      {settings && (
        <div
          role="tabpanel"
          id={`tabpanel-${tab}`}
          aria-labelledby={`tab-${tab}`}
          className="mt-4"
        >
          {tab === "downloads" && (
            <div className="grid max-w-[1160px] grid-cols-2 gap-4 max-settings:grid-cols-1">
              <div className="flex flex-col gap-4">
                <div className={card}>
                  <Row key={`username:${usernameRevert}`} label="Username">
                    <div className="relative w-[230px] min-w-0">
                      <TextSetting
                        value={settings.username}
                        className="w-full pr-8"
                        maxLength={16}
                        sanitize={(draft) =>
                          draft.replace(/[^A-Za-z0-9_.-]/g, "")
                        }
                        onCommit={(draft) => {
                          if (draft !== settings.username)
                            settings.set_username(draft);
                        }}
                      />
                      <SaveCheck confirm={usernameSaved} />
                    </div>
                  </Row>
                </div>
                <div className={card}>
                  <Row label="Steam session">
                    {settings.steam_account ? (
                      <div className="flex min-w-0 items-center gap-3">
                        <code className="truncate text-center">
                          {settings.steam_account}
                        </code>
                        <Button
                          variant="secondary"
                          className="shrink-0"
                          onClick={() => settings.logout()}
                        >
                          Log out
                        </Button>
                      </div>
                    ) : (
                      <Note>
                        <Link href="/download">Download</Link> a season to log
                        in.
                      </Note>
                    )}
                  </Row>
                  <Row
                    label="Discord presence"
                    hint="Display the season you are playing as a Discord activity."
                  >
                    <Switch
                      label="Discord presence"
                      checked={settings.discord_rpc}
                      onChange={(value) => settings.set_discord_rpc(value)}
                    />
                  </Row>
                </div>
                {platform === "linux" && <RvpnCard />}
              </div>
              <div className="flex flex-col gap-4">
                <div className={card}>
                  <Row
                    label="Parallel downloads"
                    hint="How many file chunks are downloaded at the same time. A higher value can be faster, but uses more bandwidth and system resources."
                  >
                    <Stepper
                      label="Parallel downloads"
                      value={settings.max_downloads}
                      min={settings.download_bounds.min}
                      max={settings.download_bounds.max}
                      onCommit={(value) => settings.set_max_downloads(value)}
                    />
                  </Row>
                  <Row label="App cache">
                    <Button
                      variant="secondary"
                      onClick={() => settings.clear_cache()}
                    >
                      Clear
                    </Button>
                  </Row>
                </div>
                {platform === "linux" && (
                  <div className={card}>
                    <Row
                      label="Proton"
                      hint="Proton is a compatibility layer that runs Windows games on Linux."
                    >
                      {protons === null ? null : protons.length === 0 ? (
                        <Note>No Proton installation was found.</Note>
                      ) : (
                        <button
                          type="button"
                          aria-label="Change Proton version"
                          onClick={() => setProtonOpen(true)}
                          className="flex h-8 w-[230px] min-w-0 items-center rounded-md border border-border bg-surface-2 px-[0.4rem] text-left transition hover:border-action-edge"
                        >
                          <span className="min-w-0 grow truncate-fade font-mono text-ui text-text">
                            {protons.find((p) => p.internal === settings.proton)
                              ?.display ?? settings.proton}
                          </span>
                        </button>
                      )}
                    </Row>
                  </div>
                )}
                <div className={card}>
                  <Row label="Libraries">
                    <div className="flex min-w-0 items-center gap-3">
                      {diskUsageGb != null && <code>{diskUsageGb} GB</code>}
                      <Button
                        variant="secondary"
                        className="shrink-0"
                        disabled={downloading}
                        onClick={() => settings.add_library()}
                      >
                        Add library
                      </Button>
                    </div>
                  </Row>
                  <div className="flex flex-col gap-2 empty:hidden">
                    {(libraries ?? []).map((library) => (
                      <ListRow
                        key={library.path}
                        label={library.display}
                        title={library.path}
                        strike={!library.exists}
                      >
                        <span className="-mr-1 flex shrink-0 items-center">
                          <button
                            type="button"
                            aria-label={`Open ${library.display}`}
                            disabled={!library.exists}
                            onClick={() =>
                              onBridgeReady((bridge) =>
                                bridge.info.open_library(library.path),
                              )
                            }
                            className={iconButton}
                          >
                            <FolderIcon />
                          </button>
                          {library.default ? (
                            <DefaultLibraryIcon />
                          ) : (
                            <button
                              type="button"
                              aria-label="Make default"
                              disabled={!library.exists}
                              onClick={() =>
                                settings.set_default_library(library.path)
                              }
                              className={iconButton}
                            >
                              <BookmarkIcon />
                            </button>
                          )}
                          {!library.default && !library.fixed && (
                            <button
                              type="button"
                              aria-label={`Remove ${library.display}`}
                              onClick={() =>
                                library.seasons > 0
                                  ? setRemoveTarget(library)
                                  : settings.remove_library(library.path)
                              }
                              className={iconButton}
                            >
                              <RemoveIcon />
                            </button>
                          )}
                        </span>
                      </ListRow>
                    ))}
                  </div>
                </div>
              </div>
            </div>
          )}

          {tab === "extra" && (
            <div className="grid max-w-[1160px] grid-cols-2 items-start gap-4 max-settings:grid-cols-1">
              <div className="flex flex-col gap-4">
                <div className={card}>
                  <div className="grid grid-cols-2 gap-4">
                    <span className={`flex h-8 items-center ${heading}`}>
                      Progress bar
                    </span>
                    <span className="flex items-center justify-end gap-2">
                      <ColorBar
                        label="Progress bar color"
                        colors={BAR_PRESETS.map((preset) => preset.fill)}
                        value={settings.bar_fill || DEFAULT_FILL}
                        onSelect={(value) => settings.set_bar_fill(value)}
                      />
                      <HexSetting
                        label="Progress bar hex code"
                        value={settings.bar_fill || DEFAULT_FILL}
                        onCommit={(hex) => settings.set_bar_fill(hex)}
                      />
                    </span>
                    <span className="flex h-8 items-center">
                      <span className="h-3.5 w-full rounded-full border border-border bg-well">
                        <span className="transfer-fill block h-full w-[70%] rounded-full" />
                      </span>
                    </span>
                    <span className="flex items-center justify-end gap-2">
                      <ColorBar
                        label="Stripe color"
                        colors={BAR_PRESETS.map((preset) => preset.stripe)}
                        value={settings.bar_stripe || DEFAULT_STRIPE}
                        onSelect={(value) => settings.set_bar_stripe(value)}
                      />
                      <HexSetting
                        label="Stripe hex code"
                        value={settings.bar_stripe || DEFAULT_STRIPE}
                        onCommit={(hex) => settings.set_bar_stripe(hex)}
                      />
                    </span>
                  </div>
                </div>
                <div className={card}>
                  {settings.tray_available && (
                    <Row
                      label="Close to tray"
                      hint="Keep the Launcher running in the tray when the window is closed."
                    >
                      <Switch
                        label="Close to tray"
                        checked={settings.close_to_tray}
                        onChange={(value) => settings.set_close_to_tray(value)}
                      />
                    </Row>
                  )}
                  <Row label="Reduce motion">
                    <Switch
                      label="Reduce motion"
                      checked={settings.reduce_motion}
                      onChange={(value) => settings.set_reduce_motion(value)}
                    />
                  </Row>
                </div>
              </div>
              <div className="flex flex-col gap-4">
                <div className={card}>
                  <div className="grid grid-cols-2 gap-4">
                    <span className={`flex h-8 items-center ${heading}`}>
                      Accent
                    </span>
                    <span className="row-span-2 min-w-0">
                      <AccentPicker
                        value={settings.accent || DEFAULT_ACCENT}
                        onCommit={(hex) => settings.set_accent(hex)}
                      />
                    </span>
                    <span className="flex h-8 items-center gap-2">
                      <HexSetting
                        label="Accent hex code"
                        value={settings.accent || DEFAULT_ACCENT}
                        onCommit={(hex) => settings.set_accent(hex)}
                      />
                      <Button
                        variant="secondary"
                        onClick={() => settings.reset_accent()}
                      >
                        Reset
                      </Button>
                    </span>
                  </div>
                </div>
                <div className={card}>
                  <Row label="Scale">
                    <LevelStepper
                      label="Scale"
                      value={settings.scale}
                      levels={settings.scale_levels}
                      suffix="%"
                      onCommit={(value) => settings.set_scale(value)}
                    />
                  </Row>
                </div>
              </div>
            </div>
          )}
        </div>
      )}

      {removeTarget && (
        <ConfirmModal
          title="Remove library"
          confirmLabel="Remove"
          note="No files will be deleted."
          onConfirm={() => {
            settings?.remove_library(removeTarget.path);
            setRemoveTarget(null);
          }}
          onCancel={() => setRemoveTarget(null)}
        >
          <p className="text-body text-text-muted">
            Seasons in this library will no longer appear.
          </p>
        </ConfirmModal>
      )}

      {protonOpen && settings && protons && (
        <Modal title="Proton" onClose={() => setProtonOpen(false)}>
          <div className="flex flex-col gap-2">
            {protons.map((proton) => {
              const selected = settings.proton === proton.internal;
              return (
                <PickerRow
                  key={proton.internal}
                  label={proton.display}
                  selected={selected}
                  onSelect={() => {
                    settings.set_proton(proton.internal);
                    setProtonOpen(false);
                  }}
                />
              );
            })}
          </div>
        </Modal>
      )}
    </>
  );
}
