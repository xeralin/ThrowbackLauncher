"use client";

import { SeasonBrowser } from "@/components/SeasonBrowser";
import { useSeasons } from "@/lib/bridge";

export default function DownloadPage() {
  return (
    <SeasonBrowser
      seasons={useSeasons()}
      emptyMessage="No seasons available."
      searchable
    />
  );
}
