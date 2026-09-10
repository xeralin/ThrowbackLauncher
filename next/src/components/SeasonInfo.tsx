import type { ReactNode } from "react";
import Image from "next/image";
import { Note } from "@/components/Note";
import { keyArtFade } from "@/components/SeasonKeyArt";
import { BuildChips } from "@/components/SeasonTable";
import { renderInline } from "@/lib/inline-markdown";
import { eventsForBuild } from "@/config/liberator-builds";
import type {
  SeasonInfoEntry,
  InfoOperator,
  InfoMap,
} from "@/config/season-info";

function assetSlug(name: string): string {
  return name
    .toLowerCase()
    .normalize("NFD")
    .replace(/[̀-ͯ]/g, "")
    .replace(/ø/g, "o")
    .replace(/ /g, "-");
}

function OperatorCard({ op }: { op: InfoOperator }) {
  return (
    <div className="flex h-[72px] items-stretch overflow-hidden rounded-lg border border-border bg-surface">
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

const infoRow =
  "border-b border-border px-[0.6rem] py-[0.2rem] text-[0.78rem] leading-[1.45] text-text-muted last:border-b-0";

function InfoBox({ title, children }: { title: string; children: ReactNode }) {
  return (
    <div className="overflow-hidden rounded-lg border border-border">
      <p className="table-head">{title}</p>
      {children}
    </div>
  );
}

export function SeasonInfo({
  entry,
  build,
  sizeGb,
}: {
  entry: SeasonInfoEntry;
  build?: string;
  sizeGb?: number;
}) {
  const note = entry.note ? (
    <Note className="max-w-[720px]">{entry.note}</Note>
  ) : null;

  const hasCards = entry.operators.length > 0 || entry.maps.length > 0;

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
    <div className="prose overflow-hidden rounded-lg border border-border">
      <table className="info-table w-full">
        <thead>
          <tr>
            <th>Release</th>
            {sizeGb != null && <th className="w-px whitespace-nowrap">Size</th>}
            {build && <th className="w-px whitespace-nowrap">Build</th>}
          </tr>
        </thead>
        <tbody>
          <tr>
            <td>{entry.release}</td>
            {sizeGb != null && (
              <td className="w-px whitespace-nowrap">{sizeGb} GB</td>
            )}
            {build && (
              <td className="w-px whitespace-nowrap">
                <BuildChips builds={[build]} />
              </td>
            )}
          </tr>
        </tbody>
      </table>
    </div>
  );

  const events = build ? eventsForBuild(build) : [];

  const eventBox = events.length > 0 && (
    <InfoBox title="Events">
      {events.map((name) => (
        <p key={name} className={infoRow}>
          {name}
        </p>
      ))}
    </InfoBox>
  );

  const highlights = entry.highlights.length > 0 && (
    <InfoBox title="Highlights">
      {entry.highlights.map((highlight) => (
        <p key={highlight} className={`${infoRow} [&_code]:text-[0.68rem]`}>
          {renderInline(highlight)}
        </p>
      ))}
    </InfoBox>
  );

  return (
    <div className="grid items-start gap-4 content:grid-cols-[minmax(0,1fr)_260px]">
      <div className="flex min-w-0 flex-col gap-3 self-stretch">
        {cards}
        {note}
      </div>
      <div className="flex flex-col gap-3">
        {released}
        {eventBox}
        {highlights}
      </div>
    </div>
  );
}
