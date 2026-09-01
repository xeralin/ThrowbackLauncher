import type { Metadata } from "next";
import { FAQ_PAGES } from "@/config/faq";
import Link from "next/link";
import { FaqHero } from "@/components/FaqHero";
import { Note } from "@/components/Note";
import { FaqAccordion, type FaqItem } from "@/components/FaqAccordion";
import { ExternalLink } from "@/components/ExternalLink";
import { site } from "@/config/site";

export const metadata: Metadata = FAQ_PAGES.general;

const faqs: FaqItem[] = [
  {
    id: "ubisoft-epic-account",
    q: "I do not own R6S on Steam. Can I use my Ubisoft or Epic Games account?",
    a: (
      <>
        <p>
          No. The Launcher uses the Steam depot service to download old game
          seasons. This requires a valid Steam account with a registered license
          for R6S.
        </p>
        <p>
          <strong>R6S is free on Steam</strong> — add it to your Steam library
          on its{" "}
          <ExternalLink href="https://store.steampowered.com/app/359550/">
            store page
          </ExternalLink>{" "}
          and the Launcher will work.
        </p>
      </>
    ),
  },
  {
    id: "antivirus-exclusion",
    q: "My antivirus is blocking the game. What should I do?",
    platform: "windows",
    a: (
      <>
        <p>
          Some antivirus programs flag game files as false positives. The fix is
          to add your library folder as an exclusion.
        </p>
        <ol>
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
            Click <strong>Add an exclusion</strong>, select{" "}
            <strong>Folder</strong>, and choose your library folder
          </li>
          <li>Restart your computer and try launching the game again</li>
        </ol>
        <Note className="my-3">
          Use <strong>Verify</strong> in the <strong>Manage</strong> tab of the
          season to restore removed game files.
        </Note>
      </>
    ),
  },
  {
    id: "steam-login",
    q: "Why does the Launcher need my Steam login?",
    a: (
      <>
        <p>
          Your credentials are required to access the Steam depot servers, where
          the old game files are stored. The Launcher uses{" "}
          <ExternalLink href={site.depotDownloaderRepoUrl}>
            DepotDownloader
          </ExternalLink>
          , an open-source tool.
        </p>
        <Note className="my-3">
          Your password is never stored — the Launcher keeps only an encrypted
          access token, just like the Steam client.
        </Note>
      </>
    ),
  },
  {
    id: "username",
    q: "How do I change my username?",
    a: (
      <>
        <p>
          Open the <Link href="/settings">Settings</Link> in the Launcher and
          edit the <strong>Username</strong> field (max 16 characters).
        </p>
        <Note className="my-3">
          Set your username before launching the game so it applies in-game.
        </Note>
      </>
    ),
  },
  {
    id: "discord-presence",
    q: "How does the Discord presence work?",
    a: (
      <>
        <p>
          The Launcher can show the season you are playing as a Discord
          activity. Open the <Link href="/settings">Settings</Link> and enable{" "}
          <strong>Discord presence</strong>.
        </p>
        <Note className="my-3">
          <strong>Share my activity</strong> has to be enabled under{" "}
          <strong>Activity Privacy</strong> in your Discord settings.
        </Note>
      </>
    ),
  },
  {
    id: "verify",
    q: "What does Verify do?",
    a: (
      <p>
        On an installed season the <strong>Manage</strong> tab shows a{" "}
        <strong>Verify</strong> button. It checks for missing or corrupted files
        and re-downloads them without deleting your existing files.
      </p>
    ),
  },
  {
    id: "different-drive",
    q: "Can I install a season to a different drive?",
    a: (
      <p>
        Yes. Open the <Link href="/settings">Settings</Link>, press{" "}
        <strong>Add library</strong> to add a folder, and use the bookmark icon
        to make it the default. When more than one library exists, the Launcher
        asks which one to use before each download.
      </p>
    ),
  },
  {
    id: "current-season",
    q: "Do I need the current season of R6S installed?",
    a: (
      <p>
        No. Each season the Launcher installs runs on its own, like a separate
        game.
      </p>
    ),
  },
  {
    id: "proton-version",
    platform: "linux",
    q: "Which Proton version does the Launcher use?",
    a: (
      <p>
        The Launcher picks a Proton version that you have installed. You can
        change it under <strong>Proton</strong> in the{" "}
        <Link href="/settings">Settings</Link>.
      </p>
    ),
  },
];

export default function General() {
  return (
    <>
      <FaqHero page="general" />
      <FaqAccordion items={faqs} />
    </>
  );
}
