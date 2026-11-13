import Image from "next/image";
import { keyArtFade } from "@/components/SeasonKeyArt";
import { renderInline } from "@/components/Markdown";
import { panel, textBox, textBoxText } from "@/components/ui";
import type {
  SeasonInfoEntry,
  InfoOperator,
  InfoMap,
} from "@/config/season-info";

const boxCell = `content-center px-[0.6rem] ${textBoxText}`;

function InfoList({ rows }: { rows: string[] }) {
  if (rows.length === 0) return null;
  return (
    <div className={`prose divide-y divide-border ${panel}`}>
      {rows.map((row) => (
        <div
          key={row}
          className="px-[0.6rem] text-[0.78rem] leading-[normal] text-text-muted [--cap-pad:0.5rem] text-box-cap [&_code]:text-[0.68rem] [&_code]:whitespace-nowrap"
        >
          {renderInline(row)}
        </div>
      ))}
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
  const highlights =
    events.length > 0
      ? [
          ...entry.highlights,
          `${events.map((name) => `**${name}**`).join(" and ")} event${events.length > 1 ? "s" : ""}`,
        ]
      : entry.highlights;
  const requirements =
    entry.requirements?.filter((row): row is string => row !== false) ?? [];

  return (
    <div className="grid items-start gap-4 content:grid-cols-[minmax(0,1fr)_260px]">
      <div className="grid min-w-0 grid-cols-[repeat(auto-fill,minmax(min(280px,100%),1fr))] gap-3">
        {entry.operators.map((op) => (
          <OperatorCard key={op.name} op={op} />
        ))}
        {entry.maps.map((map) => (
          <MapCard key={map.name} map={map} />
        ))}
      </div>
      <div className="flex flex-col gap-3">
        <InfoList rows={highlights} />
        <InfoList rows={requirements} />
        <div className={`${textBox} self-end`}>
          <span className={boxCell}>{entry.release}</span>
          <span className={boxCell}>{sizeGb} GB</span>
          <span className="flex items-center px-[0.6rem]">
            <code className="code-chip">{build}</code>
          </span>
        </div>
      </div>
    </div>
  );
}
