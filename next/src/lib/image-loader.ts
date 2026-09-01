import type { ImageLoaderProps } from "next/image";
import keyArtWidths from "@/config/keyart-widths.json";

const variantWidths = keyArtWidths.slice(0, -1);

export default function imageLoader({ src, width }: ImageLoaderProps): string {
  if (!src.startsWith("/keyart/")) return src;
  const variant = variantWidths.find((value) => value >= width);
  return variant ? src.replace("/keyart/", `/keyart/${variant}/`) : src;
}
