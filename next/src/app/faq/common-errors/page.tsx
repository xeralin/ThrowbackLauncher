import type { Metadata } from "next";
import { FAQ_PAGES } from "@/config/faq";
import Image from "next/image";
import { FaqHero } from "@/components/FaqHero";
import { Note } from "@/components/Note";
import { FaqAccordion, type FaqItem } from "@/components/FaqAccordion";
import { ExternalLink } from "@/components/ExternalLink";

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
