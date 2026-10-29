"use client";

import { useState } from "react";
import { CHEVRON_RIGHT, StrokeIcon } from "@/components/icons";
import { Prose } from "@/components/Prose";
import { SeasonTable } from "@/components/SeasonTable";
import {
  columnItem,
  columnItemActive,
  columnItemIdle,
  columnLabel,
  panel,
  stepBox,
  stepList,
} from "@/components/ui";
import { SUPPORT, seasonYear, yearGroups } from "@/config/liberator-builds";

export function SupportedSeasons() {
  const groups = yearGroups(SUPPORT);
  const [year, setYear] = useState(0);
  return (
    <div className="flex flex-wrap items-start justify-between gap-x-4">
      <div className="flex flex-wrap items-start gap-x-4">
        <ul className={`mb-5 w-[150px] p-2 ${panel}`}>
          {groups.map((rows, index) => (
            <li key={rows[0].build}>
              <button
                type="button"
                onClick={() => setYear(index)}
                className={`${columnItem} ${index === year ? columnItemActive : columnItemIdle}`}
              >
                <span className={columnLabel}>Year {seasonYear(rows[0])}</span>
                <StrokeIcon d={CHEVRON_RIGHT} className="size-3.5 shrink-0" />
              </button>
            </li>
          ))}
        </ul>
        <Prose>
          <div className="grid">
            {groups.map((rows, index) => (
              <div
                key={rows[0].build}
                className={`[grid-area:1/1] ${index === year ? "" : "invisible"}`}
              >
                <SeasonTable
                  rows={rows}
                  showEvent={rows.some((row) => row.event)}
                />
              </div>
            ))}
          </div>
        </Prose>
      </div>
      <div className={`prose mb-5 max-w-[280px] ${stepBox}`}>
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
    </div>
  );
}
