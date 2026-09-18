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

export function SupportedSeasons({ view }: { view: SupportView }) {
  return (
    <Prose>
      <div className="flex flex-wrap items-start gap-x-4">
        {view === "full" ? (
          <>
            <SeasonTable rows={FULL_SUPPORT} />
            <SeasonTable rows={FULL_SUPPORT_EVENTS} showEvent />
          </>
        ) : (
          yearPairs(UNLOCK_ALL_SEASONS).map((rows) => (
            <SeasonTable key={rows[0].build} rows={rows} />
          ))
        )}
        <div className="mb-5 flex max-w-[280px] flex-col gap-3">
          {view === "full" && (
            <div className={stepBox}>
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
          )}
          <div className={stepBox}>
            <p className="mb-0 text-[0.78rem] leading-[1.45]">
              Liberator does not support <strong>Vulkan</strong>. Launch the
              season with <strong>DirectX</strong>.
            </p>
          </div>
        </div>
      </div>
    </Prose>
  );
}
