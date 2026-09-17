import type { Metadata } from "next";
import { Hero } from "@/components/Hero";
import { CardGrid, NavCard } from "@/components/NavCard";
import { FAQ_PAGES } from "@/config/faq";

export const metadata: Metadata = FAQ_PAGES.index;

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
        description={FAQ_PAGES.index.description}
      />

      <CardGrid>
        <NavCard href="/faq/general" {...FAQ_PAGES.general} />
        <NavCard href="/faq/multiplayer" {...FAQ_PAGES.multiplayer} />
        <NavCard href="/faq/common-errors" {...FAQ_PAGES["common-errors"]} />
        <NavCard
          href="/faq/how-to-get-help"
          {...FAQ_PAGES["how-to-get-help"]}
        />
        <NavCard href="/faq/heated-metal" {...FAQ_PAGES["heated-metal"]} />
        <NavCard href="/faq/cheat-engine" {...FAQ_PAGES["cheat-engine"]} />
      </CardGrid>
    </>
  );
}
