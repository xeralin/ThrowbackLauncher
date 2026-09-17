"use client";

import { useState } from "react";
import { Button, iconButton } from "@/components/Button";
import { Modal } from "@/components/Modal";
import { ExternalLink } from "@/components/ExternalLink";
import { Note } from "@/components/Note";
import { card, ListRow } from "@/components/ui";
import { Row } from "@/components/SettingsControls";
import { Switch } from "@/components/Switch";
import { RemoveIcon } from "@/components/icons";
import { VersionChip } from "@/components/VersionChip";
import { site } from "@/config/site";
import { useRvpn, useSettings } from "@/lib/bridge";

export function RvpnCard() {
  const [manageOpen, setManageOpen] = useState(false);
  const rvpn = useRvpn();
  const settings = useSettings();

  const running = rvpn.state === "running";
  const building = rvpn.state === "building";
  const uninstalling = rvpn.state === "uninstalling";
  const active = running || building;
  const jobRunning = rvpn.state !== "idle";

  const content = !rvpn.ready ? null : rvpn.installed ||
    active ||
    rvpn.hasInstaller ? (
    <div className="flex min-w-0 items-center gap-3">
      {building && rvpn.step && (
        <code className="chip animate-pulse">{rvpn.step}</code>
      )}
      {rvpn.installed && (
        <Button
          variant="secondary"
          className="shrink-0"
          disabled={uninstalling}
          onClick={() => setManageOpen(true)}
        >
          Manage
        </Button>
      )}
      {running && (
        <Button
          variant="secondary"
          className="shrink-0"
          onClick={() => rvpn.stop()}
        >
          Stop
        </Button>
      )}
      {!active && (
        <Button
          variant="primary"
          className="shrink-0"
          disabled={jobRunning}
          onClick={() => rvpn.run()}
        >
          Run
        </Button>
      )}
    </div>
  ) : (
    <Note>
      <ExternalLink href={site.radminVpnUrl}>Download</ExternalLink> the
      installer and{" "}
      <button
        type="button"
        onClick={() => rvpn.selectInstaller()}
        className="cursor-pointer"
      >
        select it
      </button>
      {"."}
    </Note>
  );

  return (
    <div className={card}>
      <Row label="Radmin VPN">{content}</Row>
      {manageOpen && (
        <Modal
          title={
            <span className="flex items-center justify-between gap-3">
              Radmin VPN
              {rvpn.version && <VersionChip version={rvpn.version} />}
            </span>
          }
          onClose={() => setManageOpen(false)}
        >
          <div className="flex flex-col gap-2">
            <ListRow
              label="Autorun"
              hint="Run Radmin VPN automatically when you open the Launcher."
            >
              <span className="flex h-6 items-center">
                <Switch
                  label="Autorun"
                  checked={settings?.rvpn_autorun ?? false}
                  onChange={(value) => settings?.set_rvpn_autorun(value)}
                />
              </span>
            </ListRow>
            <ListRow label="Uninstall">
              <button
                type="button"
                aria-label="Uninstall Radmin VPN"
                disabled={jobRunning}
                onClick={() => {
                  setManageOpen(false);
                  rvpn.uninstall();
                }}
                className={`-mr-1 ${iconButton}`}
              >
                <RemoveIcon />
              </button>
            </ListRow>
          </div>
        </Modal>
      )}
    </div>
  );
}
