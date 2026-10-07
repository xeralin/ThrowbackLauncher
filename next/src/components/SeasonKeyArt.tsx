import Image from "next/image";
import type { Season } from "@/lib/bridge";

export const keyArtFade =
  "absolute inset-x-0 bottom-0 h-[70%] bg-gradient-to-t from-black/90 via-black/55 to-transparent";

export function SeasonKeyArt({
  keyArt,
  sizes,
  preload,
  imgClassName = "",
}: {
  keyArt: string | null;
  sizes: string;
  preload?: boolean;
  imgClassName?: string;
}) {
  return (
    <div className="absolute inset-0 bg-gradient-to-br from-[#1a0d12] to-bg">
      {keyArt && (
        <Image
          src={keyArt}
          alt=""
          fill
          preload={preload}
          sizes={sizes}
          className={`object-cover object-center ${imgClassName}`}
        />
      )}
    </div>
  );
}

export function CardKeyArt({
  season,
  sizes,
  preload,
}: {
  season: Season;
  sizes: string;
  preload?: boolean;
}) {
  return season.hm ? (
    <SeasonKeyArt keyArt="/keyart/hm.webp" sizes={sizes} preload={preload} />
  ) : (
    <>
      <SeasonKeyArt keyArt={season.keyArt} sizes={sizes} preload={preload} />
      <div className={keyArtFade} />
    </>
  );
}
