// Excalidraw loads its fonts at runtime from `${window.EXCALIDRAW_ASSET_PATH}fonts/...`, which the
// bundler cannot see. Copy them into dist/ so the tool works offline with no CDN.
// Xiaolai (12 MB of CJK glyphs) is skipped — see README. Set SKETCH_FONTS=all to include it.
import fs from "node:fs/promises";
import path from "node:path";
import { fileURLToPath } from "node:url";

const HERE = path.dirname(fileURLToPath(import.meta.url));
const SRC = path.resolve(HERE, "..", "node_modules", "@excalidraw", "excalidraw", "dist", "prod", "fonts");
const OUT = path.resolve(HERE, "..", "dist", "fonts");
const SKIP = process.env.SKETCH_FONTS === "all" ? new Set() : new Set(["Xiaolai"]);

await fs.rm(OUT, { recursive: true, force: true });
await fs.mkdir(OUT, { recursive: true });

let bytes = 0;
for (const entry of await fs.readdir(SRC, { withFileTypes: true })) {
  if (SKIP.has(entry.name)) continue;
  await fs.cp(path.join(SRC, entry.name), path.join(OUT, entry.name), { recursive: true });
}
const walk = async (dir) => {
  for (const entry of await fs.readdir(dir, { withFileTypes: true })) {
    const full = path.join(dir, entry.name);
    if (entry.isDirectory()) await walk(full);
    else bytes += (await fs.stat(full)).size;
  }
};
await walk(OUT);
console.log(`fonts → dist/fonts (${(bytes / 1024).toFixed(0)} KB, skipped: ${[...SKIP].join(", ") || "none"})`);
