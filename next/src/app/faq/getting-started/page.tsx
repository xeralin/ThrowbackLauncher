import type { Metadata } from "next";
import { FAQ_PAGES } from "@/config/faq";
import { FaqHero } from "@/components/FaqHero";
import { Note } from "@/components/Note";
import { ExclusionSteps } from "@/components/SetupSteps";
import { FaqAccordion, type FaqItem } from "@/components/FaqAccordion";
import { ExternalLink } from "@/components/ExternalLink";
import { site } from "@/config/site";

export const metadata: Metadata = FAQ_PAGES["getting-started"];

const faqs: FaqItem[] = [
  {
    id: "antivirus-exclusion",
    q: "How do I add an antivirus exclusion?",
    platform: "windows",
    a: (
      <>
        <p>
          Windows Security may flag Heated Metal and the Liberator as false
          positives.
        </p>
        <ExclusionSteps />
      </>
    ),
  },
  {
    id: "steam-login",
    q: "Why does the Launcher need my Steam login?",
    a: (
      <>
        <p>
          Your login is required to access the Steam depot servers, where the
          old game files are stored. The Launcher uses{" "}
          <ExternalLink href={site.depotDownloaderRepoUrl}>
            DepotDownloader
          </ExternalLink>
          , an open-source tool.
        </p>
        <Note className="my-3">
          Your password is never stored. The Launcher keeps only an access
          token, just like the Steam client.
        </Note>
      </>
    ),
  },
  {
    id: "download-broken",
    q: "Why is my download stuck at a certain percentage?",
    a: (
      <p>
        Some game files are very large, and the percentage only updates when a
        file is done. If there is network activity, it is still downloading.
      </p>
    ),
  },
  {
    id: "current-season",
    q: "Do I need the current season of R6S installed?",
    a: <p>No. Each downloaded season runs on its own, like a separate game.</p>,
  },
];

export default function GettingStarted() {
  return (
    <>
      <FaqHero page="getting-started" />
      <FaqAccordion items={faqs} />
    </>
  );
}
