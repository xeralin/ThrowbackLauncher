import { Prose } from "@/components/Prose";
import { SeasonTable } from "@/components/SeasonTable";
import { stepBox, stepList } from "@/components/ui";
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
      <div className={`${stepBox} mb-5 max-w-[280px]`}>
        <ol className={stepList}>
          <li>Create a local custom game</li>
          <li>
            Select the game mode in the <strong>Playlist</strong> tab
          </li>
          <li>
            For any PvE mode, make sure all players are on the{" "}
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
