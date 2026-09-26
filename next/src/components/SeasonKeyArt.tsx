import Image from "next/image";
import type { Season } from "@/lib/bridge";

export const keyArtFade =
  "absolute inset-x-0 bottom-0 h-[70%] bg-gradient-to-t from-black/90 via-black/55 to-transparent";

export function SeasonKeyArt({
  keyArt,
  sizes,
  priority = false,
  imgClassName = "",
}: {
  keyArt: string | null;
  sizes: string;
  priority?: boolean;
  imgClassName?: string;
}) {
  return (
    <div className="absolute inset-0 bg-gradient-to-br from-keyart-fallback to-bg">
      {keyArt && (
        <Image
          src={keyArt}
          alt=""
          fill
          priority={priority}
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
  priority = false,
}: {
  season: Season;
  sizes: string;
  priority?: boolean;
}) {
  return season.hm ? (
    <SeasonKeyArt keyArt="/keyart/hm.webp" sizes={sizes} priority={priority} />
  ) : (
    <>
      <SeasonKeyArt keyArt={season.keyArt} sizes={sizes} priority={priority} />
      <div className={keyArtFade} />
    </>
  );
}
