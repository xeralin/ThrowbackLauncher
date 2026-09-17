"use client";

import { Note } from "@/components/Note";
import { SeasonBrowser } from "@/components/SeasonBrowser";
import { useSeasons } from "@/lib/bridge";

export default function DownloadPage() {
  return (
    <SeasonBrowser
      seasons={useSeasons()}
      emptyMessage={
        <Note className="max-w-[640px]">No seasons available.</Note>
      }
      searchable
    />
  );
}
