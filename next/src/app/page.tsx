"use client";

import Link from "next/link";
import { SeasonBrowser } from "@/components/SeasonBrowser";
import { link } from "@/components/ui";
import { useHomeSeasons } from "@/lib/bridge";

export default function HomePage() {
  const [seasons, refresh] = useHomeSeasons();

  return (
    <SeasonBrowser
      seasons={seasons}
      emptyMessage={
        <div className="flex flex-col items-center pt-[12vh] text-center">
          <h1 className="mb-4 font-display text-[1.9rem] font-bold text-text">
            Your library is empty
          </h1>
          <p className="text-body leading-[1.6] text-text-muted">
            Pick a season from the{" "}
            <Link href="/download" className={link}>
              Download
            </Link>{" "}
            page, or add a library in{" "}
            <Link href="/settings" className={link}>
              Settings
            </Link>
            .
          </p>
        </div>
      }
      layout="dashboard"
      onReturn={refresh}
    />
  );
}
