import { Prose } from "@/components/Prose";
import { SeasonTable } from "@/components/SeasonTable";
import {
  FULL_SUPPORT,
  FULL_SUPPORT_EVENTS,
  UNLOCK_ALL_SEASONS,
  yearPairs,
} from "@/config/liberator-builds";

export type SupportView = "full" | "unlock";

function FullSupport() {
  return (
    <>
      <SeasonTable rows={FULL_SUPPORT} />
      <SeasonTable rows={FULL_SUPPORT_EVENTS} showEvent />
      <div className="mb-5 max-w-[280px] rounded-lg border border-border px-[0.6rem] py-[0.45rem]">
        <ol className="mb-0 pl-[1.15rem] text-[0.78rem] leading-[1.45] [&>li:last-child]:mb-0">
          <li>Create a local custom game</li>
          <li>
            Select the game mode in the <strong>Playlist</strong> tab
          </li>
          <li>
            For Terrorist Hunt or the Outbreak event, join the{" "}
            <strong>blue team</strong>, then start the match
          </li>
        </ol>
      </div>
    </>
  );
}

function UnlockAll() {
  return yearPairs(UNLOCK_ALL_SEASONS).map((rows) => (
    <SeasonTable key={rows[0].build} rows={rows} />
  ));
}

export function SupportedSeasons({ view }: { view: SupportView }) {
  return (
    <Prose>
      <div className="flex flex-wrap items-start gap-x-4">
        {view === "full" ? <FullSupport /> : <UnlockAll />}
      </div>
    </Prose>
  );
}
