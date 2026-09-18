"use client";

import { Fragment, useEffect, useState } from "react";
import { ExternalLink } from "@/components/ExternalLink";
import { stepBox, stepList } from "@/components/ui";
import { site } from "@/config/site";
import { onBridgeReady, useLibraries } from "@/lib/bridge";

export function ExclusionSteps() {
  const [launcher, setLauncher] = useState("");
  const [libraries] = useLibraries();

  useEffect(() => {
    onBridgeReady((bridge) => bridge.settings.launcher_folder(setLauncher));
  }, []);

  if (!launcher || !libraries) return null;

  const folders = [
    launcher,
    ...libraries
      .filter((library) => !library.fixed)
      .map((library) => library.path),
  ];

  return (
    <div className={`prose max-w-[720px] ${stepBox}`}>
      <ol className={stepList}>
        <li>
          Search for <strong>Virus & Threat Protection</strong> in the Windows
          start menu
        </li>
        <li>
          Click <strong>Manage settings</strong> under{" "}
          <em>Virus & Threat Protection Settings</em>
        </li>
        <li>
          Scroll down to <em>Exclusions</em> and click{" "}
          <strong>Add or remove exclusions</strong>
        </li>
        <li>
          Click <strong>Add an exclusion</strong> &gt; <strong>Folder</strong>{" "}
          and add
          {folders.map((folder) => (
            <Fragment key={folder}>
              {" "}
              <code className="code-chip">{folder}</code>
            </Fragment>
          ))}
        </li>
      </ol>
    </div>
  );
}

export function ProtonSteps() {
  return (
    <div className={`prose max-w-[720px] ${stepBox}`}>
      <p className="mb-[0.3rem] text-[0.78rem] leading-[1.45]">
        <strong>Y9S2 New Blood</strong> only runs on a specific Proton build.
      </p>
      <ol className={stepList}>
        <li>
          Download the Proton build from{" "}
          <ExternalLink href={site.indevReleasesUrl}>
            <code>#indev-releases</code>
          </ExternalLink>
        </li>
        <li>
          Extract it into{" "}
          <code>~/.local/share/ThrowbackLauncher/bin/proton</code>
        </li>
        <li>
          Restart the Launcher, then pick it under <strong>Proton</strong> on
          this page
        </li>
      </ol>
    </div>
  );
}
