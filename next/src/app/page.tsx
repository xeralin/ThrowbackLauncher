"use client";

import Link from "next/link";
import { Button, buttonBase, buttonVariants } from "@/components/Button";
import { SeasonBrowser } from "@/components/SeasonBrowser";
import { pageTitle } from "@/components/ui";
import { useDownloader, useHomeSeasons, useSettings } from "@/lib/bridge";

function EmptyLibrary() {
  const settings = useSettings();
  const downloading = useDownloader().running;

  return (
    <div className="flex flex-col items-center pt-[12vh] text-center">
      <h1 className={pageTitle}>Your library is empty</h1>
      <div className="mt-6 flex gap-2">
        <Link
          href="/download"
          className={`${buttonBase} ${buttonVariants.primary}`}
        >
          Download
        </Link>
        <Button
          variant="secondary"
          disabled={downloading}
          onClick={() => settings?.add_library()}
        >
          Add library
        </Button>
      </div>
    </div>
  );
}

export default function HomePage() {
  const [seasons, refresh] = useHomeSeasons();

  return (
    <SeasonBrowser
      seasons={seasons}
      emptyMessage={<EmptyLibrary />}
      layout="dashboard"
      onReturn={refresh}
    />
  );
}
