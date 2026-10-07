"use client";

import { CopyPathRow } from "@/components/CopyPathRow";
import { ExternalLink } from "@/components/ExternalLink";
import { PROTON, StrokeIcon } from "@/components/icons";
import { stepBox, stepList } from "@/components/ui";
import { site } from "@/config/site";
import { useLibraries, useSettings } from "@/lib/bridge";

export function ExclusionSteps({ library }: { library?: string }) {
  const settings = useSettings();
  const [libraries] = useLibraries();

  if (!settings || !libraries) return null;

  const custom = libraries.filter((entry) => !entry.fixed && entry.exists);
  const shown =
    library === undefined
      ? custom
      : custom.filter((entry) => entry.path === library);

  return (
    <div className="max-w-[720px]">
      <ExclusionStepList
        paths={[settings.launcher_folder, ...shown.map((entry) => entry.path)]}
      />
    </div>
  );
}

export function ExclusionStepList({ paths }: { paths: string[] }) {
  return (
    <div className="flex w-fit max-w-full flex-col gap-3">
      <div className={`prose ${stepBox}`}>
        <ol className={stepList}>
          <li>
            Search for <strong>Virus & threat protection</strong> in the Windows
            Start menu
          </li>
          <li>
            Click <strong>Manage settings</strong> under{" "}
            <em>Virus & threat protection settings</em>
          </li>
          <li>
            Scroll down to <em>Exclusions</em> and click{" "}
            <strong>Add or remove exclusions</strong>
          </li>
          <li>
            Click <strong>Add an exclusion</strong> &gt; <strong>Folder</strong>{" "}
            and paste the {paths.length > 1 ? "paths" : "path"} below
          </li>
        </ol>
      </div>
      <div className="flex flex-col gap-2">
        {paths.map((path) => (
          <CopyPathRow key={path} path={path} />
        ))}
      </div>
    </div>
  );
}

export function ProtonSteps() {
  return (
    <div className={`prose w-fit max-w-[720px] ${stepBox}`}>
      <p className="mb-[0.3rem] text-[0.78rem] leading-[1.45]">
        <strong>Y9S2 Heated Metal</strong> only runs on a specific Proton build.
      </p>
      <ol className={stepList}>
        <li>
          Download the Proton build from{" "}
          <ExternalLink
            href={site.heatedMetalDiscordUrl}
            className="whitespace-nowrap"
          >
            <code>#indev-releases</code>
          </ExternalLink>
        </li>
        <li>
          Extract it into{" "}
          <code>~/.local/share/ThrowbackLauncher/bin/proton</code>
        </li>
        <li>
          Pick it under{" "}
          <strong>
            <StrokeIcon
              d={PROTON}
              className="inline size-[1.15em] align-[-0.2em]"
            />{" "}
            Proton
          </strong>{" "}
          in the <strong>Manage</strong> tab
        </li>
      </ol>
    </div>
  );
}
