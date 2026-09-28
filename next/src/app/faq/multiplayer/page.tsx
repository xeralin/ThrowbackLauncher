import type { Metadata } from "next";
import { FAQ_PAGES } from "@/config/faq";
import { site } from "@/config/site";
import Link from "next/link";
import { FaqHero } from "@/components/FaqHero";
import { SectionTitle } from "@/components/SectionTitle";
import { Prose } from "@/components/Prose";
import { FaqAccordion, type FaqItem } from "@/components/FaqAccordion";
import { ExternalLink } from "@/components/ExternalLink";
import { OnLinux, OnWindows } from "@/components/OnPlatform";

export const metadata: Metadata = FAQ_PAGES.multiplayer;

const faqs: FaqItem[] = [
  {
    id: "host-matters",
    q: "Does it matter who hosts the game?",
    a: (
      <p>
        Yes. The player with the most stable internet connection should host. If
        you are experiencing lag, try switching hosts.
      </p>
    ),
  },
  {
    id: "find-game",
    q: "I cannot find the hosted game. What should I check?",
    a: (
      <ol>
        <li>
          <strong>Check your game version</strong> — All players must be on the
          same build, which installing the same season through the Launcher
          guarantees, and which <strong>Show Metrics</strong> in the game
          settings confirms
        </li>
        <li>
          <strong>Check your Radmin VPN network</strong> — Make sure all players
          are connected to the same network and no other VPN is running
        </li>
        <li>
          <strong>Check your firewall</strong> — Make sure the old R6S build is
          allowed through your firewall
          <OnWindows> for both private and public networks</OnWindows>
        </li>
        <li>
          <strong>Restart</strong> — Try restarting both the game and Radmin VPN
        </li>
      </ol>
    ),
  },
];

export default function Multiplayer() {
  return (
    <>
      <FaqHero page="multiplayer" />

      <SectionTitle>Radmin VPN Setup</SectionTitle>
      <Prose>
        <ol>
          <OnWindows>
            <li>
              Download and install{" "}
              <ExternalLink href={site.radminVpnUrl}>Radmin VPN</ExternalLink>
            </li>
          </OnWindows>
          <OnLinux>
            <li>
              Download the{" "}
              <ExternalLink href={site.radminVpnUrl}>
                Radmin VPN installer
              </ExternalLink>{" "}
              for Windows
            </li>
            <li>
              Open the <Link href="/settings">Settings</Link> and select the
              downloaded <code>.exe</code> under <strong>Radmin VPN</strong>
            </li>
          </OnLinux>
          <li>
            Open <strong>Network</strong> at the top of Radmin VPN and create a
            network, or join one that your friends are already in
          </li>
        </ol>
      </Prose>

      <SectionTitle>How to Play</SectionTitle>
      <Prose>
        <ol>
          <li>Make sure all players are connected to the same network</li>
          <li>Launch the game and create a local custom game</li>
          <li>
            Other players can join by selecting <strong>Join Local</strong> from
            the main menu
          </li>
        </ol>
      </Prose>

      <SectionTitle>Frequently Asked Questions</SectionTitle>
      <FaqAccordion items={faqs} />
    </>
  );
}
