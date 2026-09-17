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
    q: "How do I fix the MSVCRXXX.dll error?",
    display: (
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
            Visit this{" "}
            <ExternalLink href="https://github.com/abbodi1406/vcredist/releases/latest">
              repository
            </ExternalLink>
          </li>
          <li>
            Download <code>VisualCppRedist_AIO_x86_x64.exe</code> and run it as
            administrator
          </li>
          <li>Restart your computer and try launching the game again</li>
        </ol>
        <Note className="my-3">
          If the error persists, make sure Windows is fully up to date, then
          repeat the steps above.
        </Note>
      </>
    ),
  },
  {
    id: "missing-exe",
    q: "How do I fix a missing .exe or the uplay_rx_loader64.dll error?",
    display: (
      <>
        How do I fix a missing <code>.exe</code> or the{" "}
        <code>uplay_rx_loader64.dll</code> error?
      </>
    ),
    platform: "windows",
    a: (
      <>
        <p>
          This is usually caused by your antivirus blocking or removing a
          required file.
        </p>
        <ol>
          <li>
            Follow the instructions on the{" "}
            <Link href="/faq/general#antivirus-exclusion">General page</Link> to
            add an exclusion for your library folder
          </li>
          <li>
            Use <strong>Verify</strong> in the <strong>Manage</strong> tab of
            the season to restore the removed files
          </li>
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
    q: 'My old R6S install opens the current season or gets stuck on "Preparing Content"',
    display: (
      <>
        My old R6S install opens the current season or gets stuck on{" "}
        <em>Preparing Content</em>
      </>
    ),
    a: (
      <>
        <ol>
          <li>
            <strong>Restart the game</strong> — Close it completely with{" "}
            <strong>Stop</strong> in the Launcher
            <OnWindows> or via Task Manager</OnWindows> and try again
          </li>
          <li>
            <strong>Verify your files</strong> — Use <strong>Verify</strong> in
            the <strong>Manage</strong> tab of the season to check for missing
            or corrupted files
          </li>
          <OnWindows>
            <li>
              <strong>Check your antivirus</strong> — If files were removed,
              exclude your library folder (see the{" "}
              <Link href="/faq/general#antivirus-exclusion">General page</Link>)
              and run <strong>Verify</strong> again
            </li>
          </OnWindows>
        </ol>
        <Note className="my-3">
          This issue usually resolves itself after a restart.
        </Note>
      </>
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
          <li>
            Move it into the folder of the affected season inside your library,
            replacing the existing file
          </li>
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
    q: 'Why do I get a "User profile loading failed" error?',
    display: (
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
