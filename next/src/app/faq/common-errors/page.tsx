import type { Metadata } from "next";
import { FAQ_PAGES } from "@/config/faq";
import Image from "next/image";
import Link from "next/link";
import { FaqHero } from "@/components/FaqHero";
import { Note } from "@/components/Note";
import { FaqAccordion, type FaqItem } from "@/components/FaqAccordion";
import { ExternalLink } from "@/components/ExternalLink";
import { OnWindows } from "@/components/OnPlatform";

export const metadata: Metadata = FAQ_PAGES["common-errors"];

const faqs: FaqItem[] = [
  {
    id: "game-crash",
    q: "Why does my game keep crashing?",
    platform: "windows",
    a: (
      <p>
        External overlays like Overwolf may be incompatible with the old R6S
        build, so disable them before launching the game.
      </p>
    ),
  },
  {
    id: "download-errors",
    q: "I am getting errors while downloading. What should I do?",
    a: (
      <>
        <p>
          Most errors during download do not affect the final result and can be
          ignored.
        </p>
        <ul>
          <li>
            <strong>Encountered error downloading chunk</strong> — Safe to
            ignore, since the Steam servers were briefly unreachable and the
            Launcher retries automatically
          </li>
          <li>
            <strong>Depot is not available</strong> — You do not own R6S on
            Steam
          </li>
          <li>
            <strong>Failed to allocate file</strong> — Free up storage space on
            the install drive
          </li>
          <li>
            <strong>The process cannot access the file</strong> — Close any
            program that might be interfering, such as
            <OnWindows> your antivirus or</OnWindows> another game instance
          </li>
        </ul>
      </>
    ),
  },
  {
    id: "controller-input",
    q: "Why is my game stuck on controller input?",
    a: (
      <ol>
        <li>
          <strong>Restart the game</strong> — This mostly happens on the first
          launch after a download and is gone for good afterwards
        </li>
        <li>
          <strong>Unplug your controller</strong> — If that did not help,
          restart without it and plug it back in afterwards
        </li>
      </ol>
    ),
  },
  {
    id: "msvcr-dll",
    q: (
      <>
        How do I fix the <code>MSVCRXXX.dll</code> error?
      </>
    ),
    platform: "windows",
    a: (
      <>
        <p>
          This error means your system is missing a required Microsoft Visual
          C++ Redistributable package.
        </p>
        <ol>
          <li>
            Download{" "}
            <ExternalLink href="https://github.com/abbodi1406/vcredist/releases/latest/download/VisualCppRedist_AIO_x86_x64.exe">
              <code>VisualCppRedist_AIO_x86_x64.exe</code>
            </ExternalLink>{" "}
            and run it as administrator
          </li>
          <li>Restart your computer and try launching the game again</li>
        </ol>
        <Note className="my-3">
          If the error persists, make sure Windows is fully up to date.
        </Note>
      </>
    ),
  },
  {
    id: "d3dcompiler-dll",
    q: (
      <>
        How do I fix the <code>D3DCOMPILER_43.dll</code> error?
      </>
    ),
    platform: "windows",
    a: (
      <>
        <p>
          This error means your system is missing a legacy DirectX library that
          is not included in Windows.
        </p>
        <ol>
          <li>
            Download the{" "}
            <ExternalLink href="https://www.microsoft.com/en-US/download/details.aspx?id=35">
              DirectX End-User Runtime Web Installer
            </ExternalLink>{" "}
            from Microsoft
          </li>
          <li>
            Run <code>dxwebsetup.exe</code>
          </li>
          <li>Restart your computer and try launching the game again</li>
        </ol>
      </>
    ),
  },
  {
    id: "corrupt-dll",
    q: "How do I fix missing or corrupt DLL files?",
    a: (
      <>
        <p>
          If you get an error mentioning any of the following files, the fix is
          the same.
        </p>
        <ul>
          <li>
            <code>amd_ags_x64.dll</code>
          </li>
          <li>
            <code>gfsdk_ssao_d3d11.win64.dll</code>
          </li>
          <li>
            <code>vivoxsdk_x64.dll</code>
          </li>
          <li>
            <code>bink2w64.dll</code>
          </li>
        </ul>
        <ol>
          <li>
            Delete the specified <code>.dll</code> file from the season folder
          </li>
          <li>
            Use <strong>Verify</strong> in the <strong>Manage</strong> tab of
            the season to restore missing files
          </li>
        </ol>
      </>
    ),
  },
  {
    id: "wrong-version",
    q: (
      <>
        My old R6S install opens the current season or gets stuck on{" "}
        <em>Preparing Content</em>. What should I do?
      </>
    ),
    a: (
      <ol>
        <li>
          <strong>Restart the game</strong> — Close it completely with{" "}
          <strong>Stop</strong> in the Launcher
          <OnWindows> or via Task Manager</OnWindows> and try again
        </li>
        <OnWindows>
          <li>
            <strong>Check your antivirus</strong> — If files were removed,
            exclude the Launcher and library folders (see{" "}
            <Link href="/faq/getting-started#antivirus-exclusion">
              Getting Started
            </Link>
            )
          </li>
        </OnWindows>
        <li>
          <strong>Clear the app cache</strong> — Use <strong>Clear</strong> next
          to <strong>App cache</strong> in <strong>Settings</strong>
        </li>
        <li>
          <strong>Verify your files</strong> — Use <strong>Verify</strong> in
          the <strong>Manage</strong> tab of the season to check for missing or
          corrupted files
        </li>
      </ol>
    ),
  },
  {
    id: "sku-rus",
    q: "My game is in Russian. How do I switch to English?",
    a: (
      <>
        <p>
          Some Steam accounts own a regional version of the game called SKU RUS,
          which is in Russian by default. If yours is affected, switch the
          language with these steps.
        </p>
        <ol>
          <li>
            Download the{" "}
            <a href="/downloads/localization.lang" download="localization.lang">
              <code>localization.lang</code>
            </a>{" "}
            file
          </li>
          <li>Move it into the season folder and replace the existing file</li>
          <li>Launch the game</li>
        </ol>
        <Image
          src="/media/others/sku-rus.webp"
          alt="SKU RUS regions map"
          width={768}
          height={112}
          unoptimized
        />
      </>
    ),
  },
  {
    id: "user-profile",
    q: (
      <>
        Why do I get a <em>User profile loading failed</em> error?
      </>
    ),
    a: (
      <p>
        This error is expected and does not affect gameplay. Click{" "}
        <strong>OK</strong>.
      </p>
    ),
  },
];

export default function CommonErrors() {
  return (
    <>
      <FaqHero page="common-errors" />
      <FaqAccordion items={faqs} />
    </>
  );
}
