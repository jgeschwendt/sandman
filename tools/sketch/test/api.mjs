// File-API checks: path-traversal refusals, GET/PUT round-trip, and an atomicity proof —
// a reader loop hammering the file while rapid PUTs land must never observe a partial write.
// Runs against a scratch scene by default; the real design/ files are never touched.
import assert from "node:assert/strict";
import fs from "node:fs/promises";
import path from "node:path";

const BASE = process.env.SKETCH_URL || "http://127.0.0.1:7873";
const SCENE = process.env.SKETCH_TEST_SCENE || "sketch-selftest";
const DESIGN = process.env.SKETCH_DESIGN_DIR
  ? path.resolve(process.env.SKETCH_DESIGN_DIR)
  : path.resolve(import.meta.dirname, "..", "..", "..", "design");
const FILE = path.join(DESIGN, `${SCENE}.excalidraw`);

let failures = 0;
const check = async (label, fn) => {
  try {
    await fn();
    console.log(`  ok   ${label}`);
  } catch (err) {
    failures++;
    console.log(`  FAIL ${label}\n       ${err.message}`);
  }
};

const get = (p) => fetch(`${BASE}${p}`);
const put = (name, body) =>
  fetch(`${BASE}/scene?f=${encodeURIComponent(name)}`, {
    method: "PUT",
    headers: { "content-type": "application/json" },
    body: JSON.stringify(body),
  });

const rect = (i) => ({
  id: `selftest-${i}`,
  type: "rectangle",
  x: i * 10,
  y: 0,
  width: 100,
  height: 50,
  angle: 0,
  strokeColor: "#1e1e1e",
  backgroundColor: "transparent",
  fillStyle: "solid",
  strokeWidth: 1,
  strokeStyle: "solid",
  roughness: 1,
  opacity: 100,
  groupIds: [],
  frameId: null,
  roundness: null,
  seed: 1,
  version: i + 1,
  versionNonce: 1,
  isDeleted: false,
  boundElements: null,
  updated: 1,
  link: null,
  locked: false,
  index: `a${String(i).padStart(3, "0")}`,
});

const scene = (n) => ({
  type: "excalidraw",
  version: 2,
  source: "sketch-selftest",
  elements: Array.from({ length: n }, (_, i) => rect(i)),
  appState: { viewBackgroundColor: "#ffffff" },
  files: {},
});

console.log(`sketch api tests → ${BASE}  scene=${SCENE}`);

// ─── traversal ───────────────────────────────────────────────────────────────
for (const bad of ["../../etc/passwd", "..%2f..%2fetc%2fpasswd", "a/b.excalidraw", ".hidden", "..", "%2e%2e%2fx"]) {
  await check(`rejects f=${bad}`, async () => {
    const res = await get(`/scene/version?f=${bad}`);
    assert.equal(res.status, 400, `expected 400, got ${res.status}`);
  });
}

await check("PUT rejects a non-excalidraw body", async () => {
  const res = await put(SCENE, { type: "notexcalidraw", elements: [] });
  assert.equal(res.status, 400);
});

// ─── round-trip ──────────────────────────────────────────────────────────────
await check("PUT creates the file, GET returns it", async () => {
  const res = await put(SCENE, scene(3));
  assert.equal(res.status, 200);
  const body = await res.json();
  assert.equal(body.elements, 3);
  const got = await get(`/scene?f=${SCENE}`);
  const back = await got.json();
  assert.equal(back.elements.length, 3);
  assert.equal(back.appState.viewBackgroundColor, "#ffffff");
  assert.equal(Number(got.headers.get("x-scene-mtime")), body.mtimeMs);
});

await check("deleted elements are stripped on write", async () => {
  const s = scene(3);
  s.elements[1] = { ...s.elements[1], isDeleted: true };
  await put(SCENE, s);
  const back = await (await get(`/scene?f=${SCENE}`)).json();
  assert.equal(back.elements.length, 2);
});

await check("unknown appState keys on disk survive a write", async () => {
  const onDisk = JSON.parse(await fs.readFile(FILE, "utf8"));
  onDisk.appState.gridSize = 20;
  await fs.writeFile(FILE, JSON.stringify(onDisk));
  await put(SCENE, scene(3));
  const back = await (await get(`/scene?f=${SCENE}`)).json();
  assert.equal(back.appState.gridSize, 20, "gridSize should be carried forward");
  assert.equal(back.appState.viewBackgroundColor, "#ffffff");
});

await check("viewport appState is never persisted", async () => {
  const s = scene(3);
  s.appState = { viewBackgroundColor: "#f8f9fa", scrollX: 999, scrollY: -42, zoom: { value: 3 } };
  await put(SCENE, s);
  const back = await (await get(`/scene?f=${SCENE}`)).json();
  assert.equal(back.appState.viewBackgroundColor, "#f8f9fa");
  assert.equal(back.appState.scrollX, undefined);
  assert.equal(back.appState.zoom, undefined);
});

await check("embedded files round-trip and orphans are pruned", async () => {
  const s = scene(2);
  const dataURL = "data:image/png;base64,iVBORw0KGgoAAAANSUhEUg==";
  s.elements[0] = { ...s.elements[0], type: "image", fileId: "kept-file" };
  s.files = {
    "kept-file": { id: "kept-file", mimeType: "image/png", dataURL, created: 1, lastRetrieved: 1 },
    "orphan-file": { id: "orphan-file", mimeType: "image/png", dataURL, created: 1, lastRetrieved: 1 },
  };
  await put(SCENE, s);
  let back = await (await get(`/scene?f=${SCENE}`)).json();
  assert.deepEqual(Object.keys(back.files), ["kept-file"]);
  assert.equal(back.files["kept-file"].dataURL, dataURL);

  // A later PUT that omits files entirely must not lose the still-referenced image.
  const s2 = { ...s, files: {} };
  await put(SCENE, s2);
  back = await (await get(`/scene?f=${SCENE}`)).json();
  assert.deepEqual(Object.keys(back.files), ["kept-file"], "referenced file must survive a files-less PUT");
});

// ─── atomicity ───────────────────────────────────────────────────────────────
await check("rapid PUTs never expose a truncated file", async () => {
  await put(SCENE, scene(60));
  let reads = 0;
  let bad = 0;
  let stop = false;
  const reader = (async () => {
    while (!stop) {
      try {
        const raw = await fs.readFile(FILE, "utf8");
        const parsed = JSON.parse(raw);
        if (!Array.isArray(parsed.elements) || parsed.elements.length < 60) bad++;
        reads++;
      } catch (err) {
        // ENOENT would mean the file vanished mid-rename; a parse error means a partial read.
        bad++;
        reads++;
      }
    }
  })();
  const writers = [];
  for (let i = 0; i < 40; i++) writers.push(put(SCENE, scene(60 + (i % 20))));
  const results = await Promise.all(writers);
  stop = true;
  await reader;
  assert.ok(reads > 20, `reader only got ${reads} reads`);
  assert.equal(bad, 0, `${bad}/${reads} reads saw a partial or missing file`);
  assert.ok(
    results.every((r) => r.status === 200),
    "every concurrent PUT should succeed",
  );
  const final = JSON.parse(await fs.readFile(FILE, "utf8"));
  assert.ok(final.elements.length >= 60);
  console.log(`       (${reads} concurrent reads across 40 overlapping PUTs, 0 partial)`);
});

await check("no .tmp leftovers in design/", async () => {
  const leftovers = (await fs.readdir(DESIGN)).filter((f) => f.endsWith(".tmp"));
  assert.deepEqual(leftovers, []);
});

await fs.rm(FILE, { force: true });
console.log(failures ? `\n${failures} FAILED` : "\nall api tests passed");
process.exit(failures ? 1 : 0);
