import Image from "next/image";
import { keyArtFade } from "@/components/SeasonKeyArt";
import { BuildChips } from "@/components/SeasonTable";
import { renderInline } from "@/components/Markdown";
import { panel, stepBox } from "@/components/ui";
import type {
  SeasonInfoEntry,
  InfoOperator,
  InfoMap,
} from "@/config/season-info";

const infoTable = "prose overflow-hidden rounded-lg border border-border";

const infoCell =
  "align-top font-body text-text-muted [&_code]:text-[0.68rem] [&_code]:whitespace-nowrap";

function InfoList({ title, rows }: { title: string; rows: string[] }) {
  if (rows.length === 0) return null;
  return (
    <div className={infoTable}>
      <table className="w-full">
        <thead>
          <tr>
            <th>{title}</th>
          </tr>
        </thead>
        <tbody>
          {rows.map((row) => (
            <tr key={row}>
              <td className={infoCell}>{renderInline(row)}</td>
            </tr>
          ))}
        </tbody>
      </table>
    </div>
  );
}

function assetSlug(name: string): string {
  return name
    .toLowerCase()
    .normalize("NFD")
    .replace(/[\u0300-\u036f]/g, "")
    .replace(/ø/g, "o")
    .replace(/ /g, "-");
}

function OperatorCard({ op }: { op: InfoOperator }) {
  return (
    <div className={`flex h-[72px] items-stretch overflow-hidden ${panel}`}>
      <div className="relative w-12 shrink-0 border-r border-border bg-surface-2">
        <Image
          src={`/info/ops/${op.img ?? assetSlug(op.name)}.webp`}
          alt=""
          fill
          unoptimized
          className="object-cover object-[50%_20%]"
        />
      </div>
      <div className="flex min-w-0 flex-1 flex-col justify-center gap-1 px-3">
        <span className="truncate font-display text-[0.95rem] font-bold leading-tight text-text">
          {op.name}
        </span>
        <p className="min-h-[2.75em] text-ui leading-snug text-text-muted">
          <span className="font-semibold text-text">{op.gadgetName}</span>
          {" — "}
          {op.gadgetDesc}
        </p>
      </div>
    </div>
  );
}

function MapCard({ map }: { map: InfoMap }) {
  return (
    <div className="relative h-[72px] overflow-hidden rounded-lg border border-border bg-surface-2">
      <Image
        src={`/info/maps/${map.img ?? assetSlug(map.name)}.webp`}
        alt=""
        fill
        unoptimized
        className="object-cover"
      />
      <div className={keyArtFade} />
      <div className="absolute bottom-2 left-3 font-display text-[0.95rem] font-bold leading-tight text-text">
        {map.name}
      </div>
    </div>
  );
}

export function SeasonInfo({
  entry,
  build,
  sizeGb,
}: {
  entry: SeasonInfoEntry;
  build: string;
  sizeGb: number;
}) {
  const events = entry.events ?? [];
  const hasCards = entry.operators.length > 0 || entry.maps.length > 0;
  const requirements =
    entry.requirements?.filter((row): row is string => row !== false) ?? [];

  const cards = hasCards && (
    <div className="grid grid-cols-[repeat(auto-fill,minmax(min(280px,100%),1fr))] gap-3">
      {entry.operators.map((op) => (
        <OperatorCard key={op.name} op={op} />
      ))}
      {entry.maps.map((map) => (
        <MapCard key={map.name} map={map} />
      ))}
    </div>
  );

  const released = (
    <div className={infoTable}>
      <table className="info-table w-full">
        <thead>
          <tr>
            <th>Release</th>
            <th className="w-px whitespace-nowrap">Size</th>
            <th className="w-px whitespace-nowrap">Build</th>
          </tr>
        </thead>
        <tbody>
          <tr>
            <td>{entry.release}</td>
            <td className="w-px whitespace-nowrap">{sizeGb} GB</td>
            <td className="w-px whitespace-nowrap">
              <BuildChips builds={[build]} />
            </td>
          </tr>
        </tbody>
      </table>
    </div>
  );

  return (
    <div className="grid items-start gap-4 content:grid-cols-[minmax(0,1fr)_260px]">
      <div className="flex min-w-0 flex-col gap-3 self-stretch">{cards}</div>
      <div className="flex flex-col gap-3">
        {released}
        <InfoList title="Highlights" rows={entry.highlights} />
        {events.length > 0 && (
          <div className={`flex self-end divide-x divide-border ${panel}`}>
            {events.map((name) => (
              <span
                key={name}
                className="whitespace-nowrap px-[0.6rem] py-[0.3rem] font-display text-[0.85rem] font-bold leading-tight text-text"
              >
                {name}
              </span>
            ))}
          </div>
        )}
        {entry.slowClose && (
          <p
            className={`${stepBox} text-[0.78rem] leading-[1.45] text-text-muted`}
          >
            Closing this season from the in-game menu can take up to 10 seconds.
          </p>
        )}
        <InfoList title="Requirements" rows={requirements} />
      </div>
    </div>
  );
}
