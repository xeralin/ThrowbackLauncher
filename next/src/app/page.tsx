"use client";

import { useEffect } from "react";
import { useRouter } from "next/navigation";
import { SeasonBrowser } from "@/components/SeasonBrowser";
import { useHomeSeasons } from "@/lib/bridge";

export default function HomePage() {
  const [seasons, refresh] = useHomeSeasons();
  const router = useRouter();
  const empty = seasons !== null && seasons.length === 0;

  useEffect(() => {
    if (empty) router.replace("/download");
  }, [empty, router]);

  return (
    <SeasonBrowser
      seasons={seasons}
      emptyMessage="No seasons installed yet."
      layout="dashboard"
      onReturn={refresh}
    />
  );
}
