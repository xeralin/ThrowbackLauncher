import { site } from "@/config/site";

export const BAR_PRESETS = [
  { fill: "#c388e3", stripe: "#dcbaef" },
  { fill: "#6aa5fc", stripe: "#a8cbfe" },
  { fill: "#00bdcf", stripe: "#87d9e2" },
  { fill: "#3cbf85", stripe: "#9adab7" },
  { fill: "#e57db1", stripe: "#f2b4d0" },
];

export const DEFAULT_FILL = BAR_PRESETS[0].fill;
export const DEFAULT_STRIPE = BAR_PRESETS[0].stripe;

export const DEFAULT_ACCENT = site.themeColor;

const ACCENT_L_BASE = 0.4;
const ACCENT_CHROMA_FLOOR = 0.25;
const ACCENT_L_PEAK_MIN = 0.517;

export const ACCENT_STEP = 0.01;

const ACCENT_LEVELS = Math.round(1 / ACCENT_STEP);

function gamma(v: number): number {
  return v <= 0.0031308 ? 12.92 * v : 1.055 * Math.pow(v, 1 / 2.4) - 0.055;
}

function linear(v: number): number {
  const c = v / 255;
  return c <= 0.04045 ? c / 12.92 : ((c + 0.055) / 1.055) ** 2.4;
}

function toRgb(lightness: number, hue: number, chroma: number): number[] {
  const a = chroma * Math.cos((hue * Math.PI) / 180);
  const b = chroma * Math.sin((hue * Math.PI) / 180);
  const l = (lightness + 0.3963377774 * a + 0.2158037573 * b) ** 3;
  const m = (lightness - 0.1055613458 * a - 0.0638541728 * b) ** 3;
  const s = (lightness - 0.0894841775 * a - 1.291485548 * b) ** 3;
  return [
    4.0767416621 * l - 3.3077115913 * m + 0.2309699292 * s,
    -1.2684380046 * l + 2.6097574011 * m - 0.3413193965 * s,
    -0.0041960863 * l - 0.7034186147 * m + 1.707614701 * s,
  ].map(gamma);
}

function maxChroma(lightness: number, hue: number): number {
  let low = 0;
  let high = 0.4;
  for (let step = 0; step < 24; step += 1) {
    const mid = (low + high) / 2;
    if (toRgb(lightness, hue, mid).every((v) => v >= -0.0005 && v <= 1.0005))
      low = mid;
    else high = mid;
  }
  return low;
}

const vividCache = new Map<number, number>();

function vividLightness(hue: number): number {
  const key = Math.round(hue);
  const known = vividCache.get(key);
  if (known !== undefined) return known;
  let low = ACCENT_L_PEAK_MIN;
  let high = 0.99;
  for (let step = 0; step < 20; step += 1) {
    const third = (high - low) / 3;
    if (maxChroma(low + third, key) < maxChroma(high - third, key))
      low += third;
    else high -= third;
  }
  const peak = (low + high) / 2;
  vividCache.set(key, peak);
  return peak;
}

function lightnessFor(hue: number, level: number): number {
  return ACCENT_L_BASE + (vividLightness(hue) - ACCENT_L_BASE) * level;
}

function chromaFor(lightness: number, hue: number, level: number): number {
  return (
    (ACCENT_CHROMA_FLOOR + (1 - ACCENT_CHROMA_FLOOR) * level) *
    maxChroma(lightness, hue)
  );
}

export function accentHex(hue: number, level: number): string {
  const lightness = lightnessFor(hue, level);
  return (
    "#" +
    toRgb(lightness, hue, chromaFor(lightness, hue, level))
      .map((v) =>
        Math.round(Math.min(1, Math.max(0, v)) * 255)
          .toString(16)
          .padStart(2, "0"),
      )
      .join("")
  );
}

export function accentParts(hex: string): { hue: number; level: number } {
  const value = parseInt(hex.slice(1), 16);
  const r = linear((value >> 16) & 255);
  const g = linear((value >> 8) & 255);
  const b = linear(value & 255);
  const l = Math.cbrt(0.4122214708 * r + 0.5363325363 * g + 0.0514459929 * b);
  const m = Math.cbrt(0.2119034982 * r + 0.6806995451 * g + 0.1073969566 * b);
  const s = Math.cbrt(0.0883024619 * r + 0.2817188376 * g + 0.6299787005 * b);
  const ax = 1.9779984951 * l - 2.428592205 * m + 0.4505937099 * s;
  const bx = 0.0259040371 * l + 0.7827717662 * m - 0.808675766 * s;
  const angle = (Math.atan2(bx, ax) * 180) / Math.PI;
  const hue =
    ax * ax + bx * bx < 1e-9
      ? DEFAULT_HUE
      : Math.round(angle < 0 ? angle + 360 : angle) % 360;
  const lightness = 0.2104542553 * l + 0.793617785 * m - 0.0040720468 * s;
  const radians = (hue * Math.PI) / 180;
  let best = 0;
  let closest = Infinity;
  for (let step = 0; step <= ACCENT_LEVELS; step += 1) {
    const level = step * ACCENT_STEP;
    const light = lightnessFor(hue, level);
    const chroma = chromaFor(light, hue, level);
    const distance =
      (light - lightness) ** 2 +
      (chroma * Math.cos(radians) - ax) ** 2 +
      (chroma * Math.sin(radians) - bx) ** 2;
    if (distance < closest) {
      closest = distance;
      best = level;
    }
  }
  return { hue, level: best };
}

const DEFAULT_HUE = accentParts(DEFAULT_ACCENT).hue;

function contrastText(hex: string): string {
  const value = parseInt(hex.slice(1), 16);
  const luminance =
    0.2126 * linear((value >> 16) & 255) +
    0.7152 * linear((value >> 8) & 255) +
    0.0722 * linear(value & 255);
  return 1.05 / (luminance + 0.05) >= (luminance + 0.05) / 0.05
    ? "#ffffff"
    : "var(--color-bg)";
}

export function applyAccent(hex: string): void {
  const root = document.documentElement;
  const value = hex || DEFAULT_ACCENT;
  root.style.setProperty("--color-action", value);
  root.style.setProperty("--color-action-text", contrastText(value));
  root.dataset.accent = hex ? "custom" : "default";
}

export const ACCENT_HUE_MAX = 359;

export const ACCENT_HUE_TRACK = `linear-gradient(to right, ${Array.from(
  { length: 13 },
  (_, i) => accentHex(360 - i * 30, 1),
).join(", ")})`;
