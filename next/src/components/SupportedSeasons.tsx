import { Prose } from "@/components/Prose";
import { SeasonTable } from "@/components/SeasonTable";
import {
  FULL_SUPPORT,
  FULL_SUPPORT_EVENTS,
  UNLOCK_ALL_SEASONS,
} from "@/config/liberator-builds";

export function SupportedSeasons() {
  return (
    <Prose>
      <div className="flex flex-wrap items-start gap-x-6">
        <SeasonTable rows={FULL_SUPPORT} />
        <SeasonTable rows={FULL_SUPPORT_EVENTS} showEvent />
        <SeasonTable rows={UNLOCK_ALL_SEASONS} />
      </div>
    </Prose>
  );
}
