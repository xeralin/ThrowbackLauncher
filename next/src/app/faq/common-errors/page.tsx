import type { Metadata } from "next";
import { FAQ_PAGES } from "@/config/faq";
import Image from "next/image";
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
      <>
        <p>
          If your game crashes or you get an <code>MSVCRXXX.dll</code> or{" "}
          <code>D3DCOMPILER_43.dll</code> error, follow these steps.
        </p>
        <ol>
          <li>
            Download{" "}
            <ExternalLink href="https://github.com/abbodi1406/vcredist/releases/latest/download/VisualCppRedist_AIO_x86_x64.exe">
              <code>VisualCppRedist_AIO_x86_x64.exe</code>
            </ExternalLink>{" "}
            and run it as administrator
          </li>
          <li>
            Download the{" "}
            <ExternalLink href="https://www.microsoft.com/en-US/download/details.aspx?id=35">
              DirectX End-User Runtime Web Installer
            </ExternalLink>{" "}
            from Microsoft and run <code>dxwebsetup.exe</code>
          </li>
          <li>
            Disable external overlays like Overwolf, since they may be
            incompatible with the old R6S build
          </li>
          <li>Restart your computer and try launching the game again</li>
        </ol>
        <Note className="my-3">
          If the problem persists, make sure Windows is fully up to date.
        </Note>
      </>
    ),
  },
  {
    id: "download-errors",
    q: "I am getting errors while downloading. What should I do?",
    a: (
      <ul>
        <li>
          <strong>Could not connect to Steam</strong> — Check your internet
          connection and try again, since the Steam servers may be briefly
          unreachable
        </li>
        <li>
          <strong>Too many login attempts</strong> — Steam is blocking further
          logins, so wait a while before trying again
        </li>
        <li>
          <strong>Steam login failed</strong> — Check your password and Steam
          Guard code, then log in again
        </li>
        <li>
          <strong>You do not own R6S on Steam</strong> — You need a Steam
          account that owns R6S
        </li>
        <li>
          <strong>A file is already in use</strong> — Close any program that
          might be interfering, such as
          <OnWindows> your antivirus or</OnWindows> another game instance
        </li>
        <li>
          <strong>Not enough disk space</strong> — Free up storage space with
          Shears or pick a different library
        </li>
        <li>
          <strong>Download failed</strong> — Press <strong>Verify</strong> to
          continue the download
        </li>
      </ul>
    ),
  },
  {
    id: "no-proton",
    q: (
      <>
        Why do I get <em>No Proton found</em>?
      </>
    ),
    platform: "linux",
    a: (
      <>
        <p>The Launcher runs each season with Proton from Steam.</p>
        <ol>
          <li>
            Install Steam from your package manager, not as Flatpak or Snap
          </li>
          <li>
            Install <strong>Proton Experimental</strong> under{" "}
            <strong>Library</strong> &gt; <strong>Tools</strong> in Steam
          </li>
        </ol>
        <Note className="my-3">
          You can also place any Proton build in{" "}
          <code>~/.local/share/ThrowbackLauncher/bin/proton</code>.
        </Note>
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
            Delete the specified <code>.dll</code> file from the game folder
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
    id: "sku-rus",
    q: (
      <>
        Why do I get <em>Your R6S license is not supported</em>?
      </>
    ),
    a: (
      <>
        <p>
          This error means your Steam account owns a regional version of the
          game called SKU RUS, which is only sold in the countries below. The
          Launcher does not support it, so you need a Steam account that owns
          the worldwide version.
        </p>
        <Image
          src="/media/others/sku-rus.webp"
          alt=""
          width={744}
          height={90}
          unoptimized
        />
      </>
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
