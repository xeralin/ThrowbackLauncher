import { readdir, mkdir, rm, stat } from "node:fs/promises";
import { fileURLToPath } from "node:url";
import { join, parse } from "node:path";
import sharp from "sharp";
import WIDTHS from "../src/config/keyart-widths.json" with { type: "json" };

const SRC = "assets/keyart";
const OUT = "public/keyart";
const FULL = WIDTHS[WIDTHS.length - 1];

const outDir = (width) => (width === FULL ? OUT : join(OUT, String(width)));

const configMtime = Math.max(
  ...(await Promise.all(
    [fileURLToPath(import.meta.url), "src/config/keyart-widths.json"].map(
      async (path) => (await stat(path)).mtimeMs,
    ),
  )),
);

await Promise.all(
  WIDTHS.map((width) => mkdir(outDir(width), { recursive: true })),
);

const files = (await readdir(SRC)).filter((f) =>
  /\.(jpe?g|png|webp)$/i.test(f),
);

const names = new Set(files.map((file) => parse(file).name));
await Promise.all(
  WIDTHS.map(async (width) => {
    for (const entry of await readdir(outDir(width), { withFileTypes: true })) {
      if (!entry.isFile() || !entry.name.endsWith(".webp")) continue;
      if (names.has(parse(entry.name).name)) continue;
      await rm(join(outDir(width), entry.name));
    }
  }),
);

await Promise.all(
  files.map(async (file) => {
    const src = join(SRC, file);
    const name = parse(file).name;
    const srcMtime = (await stat(src)).mtimeMs;
    const stale = (
      await Promise.all(
        WIDTHS.map(async (width) => {
          const out = join(outDir(width), `${name}.webp`);
          try {
            const o = await stat(out);
            if (o.mtimeMs >= srcMtime && o.mtimeMs >= configMtime) return null;
          } catch {}
          return { width, out };
        }),
      )
    ).filter(Boolean);
    if (stale.length === 0) return;
    const image = sharp(src);
    await Promise.all(
      stale.map(({ width, out }) =>
        image
          .clone()
          .resize({ width, withoutEnlargement: true })
          .webp({ quality: 90, effort: 6 })
          .toFile(out),
      ),
    );
  }),
);
