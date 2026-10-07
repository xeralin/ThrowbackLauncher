import type { Metadata } from "next";
import { Hero } from "@/components/Hero";
import { CardGrid, NavCard } from "@/components/NavCard";
import { FAQ_PAGES } from "@/config/faq";

const description =
  "Your guide to downloading, setting up, and playing older Rainbow Six Siege seasons.";

export const metadata: Metadata = { title: "FAQ", description };

export default function Faq() {
  return (
    <>
      <Hero
        tag="Operation Throwback"
        corner="R6S"
        title={
          <>
            Welcome to the <em>Throwback FAQ</em>
          </>
        }
        description={description}
      />

      <CardGrid>
        <NavCard
          href="/faq/getting-started"
          {...FAQ_PAGES["getting-started"]}
        />
        <NavCard href="/faq/common-errors" {...FAQ_PAGES["common-errors"]} />
        <NavCard href="/faq/multiplayer" {...FAQ_PAGES.multiplayer} />
        <NavCard
          href="/faq/how-to-get-help"
          {...FAQ_PAGES["how-to-get-help"]}
        />
        <NavCard href="/faq/heated-metal" {...FAQ_PAGES["heated-metal"]} />
      </CardGrid>
    </>
  );
}
