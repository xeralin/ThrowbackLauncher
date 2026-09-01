import type { Metadata } from "next";
import { FAQ_PAGES } from "@/config/faq";
import Link from "next/link";
import { FaqHero } from "@/components/FaqHero";
import { SectionTitle } from "@/components/SectionTitle";
import { Prose } from "@/components/Prose";
import { SupportedSeasons } from "@/components/SupportedSeasons";

export const metadata: Metadata = FAQ_PAGES.liberator;

export default function Liberator() {
  return (
    <>
      <FaqHero page="liberator" />

      <SectionTitle>How to Use It</SectionTitle>
      <Prose>
        <h3>Enabling it</h3>
        <ol>
          <li>
            Open the <Link href="/liberator">Liberator</Link> page in the
            Launcher
          </li>
          <li>Make sure Liberator is enabled</li>
          <li>Launch the game from the Launcher</li>
        </ol>

        <h3>Custom game</h3>
        <ol>
          <li>Create a local custom game</li>
          <li>
            Select the game mode in the <strong>Playlist</strong> tab on the
            Liberator page
          </li>
          <li>
            If you want to play Terrorist Hunt or the Outbreak event, make sure
            you are on the <strong>blue team</strong>, then start the match
          </li>
        </ol>
      </Prose>

      <SectionTitle>Support</SectionTitle>
      <SupportedSeasons />
    </>
  );
}
