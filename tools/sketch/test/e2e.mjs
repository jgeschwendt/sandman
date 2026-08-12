// Browser test of the pair-drawing loop against the REAL design/sandman-v0.excalidraw.
// The file is backed up first and restored byte-identically at the end (checksum asserted).
//   mise exec -- node test/e2e.mjs        (server must already be running on 7873)
import assert from "node:assert/strict";
import { createHash } from "node:crypto";
import fs from "node:fs/promises";
import os from "node:os";
import path from "node:path";
import { chromium } from "playwright";

const BASE = process.env.SKETCH_URL || "http://127.0.0.1:7873";
const SCENE = process.env.SKETCH_TEST_SCENE || "sandman-v0";
const DESIGN = process.env.SKETCH_DESIGN_DIR
  ? path.resolve(process.env.SKETCH_DESIGN_DIR)
  : path.resolve(import.meta.dirname, "..", "..", "..", "design");
const FILE = path.join(DESIGN, `${SCENE}.excalidraw`);
const BACKUP = path.join(os.tmpdir(), `sketch-e2e-${SCENE}-${process.pid}.bak`);
const ALT = "sketch-e2e-alt"; // a second scene, created and removed by this test
const ALT_FILE = path.join(DESIGN, `${ALT}.excalidraw`);

const sha = (buf) => createHash("sha256").update(buf).digest("hex");
const sleep = (ms) => new Promise((r) => setTimeout(r, ms));

let failures = 0;
const check = async (label, fn) => {
  try {
    await fn();
    console.log(`  ok   ${label}`);
  } catch (err) {
    failures++;
    console.log(`  FAIL ${label}\n       ${(err.stack || err.message).split("\n").slice(0, 3).join("\n       ")}`);
  }
};

/** Poll `fn` until it returns truthy or the deadline passes. */
async function until(fn, { timeout = 8000, step = 100, what = "condition" } = {}) {
  const deadline = Date.now() + timeout;
  let last;
  while (Date.now() < deadline) {
    last = await fn();
    if (last) return last;
    await sleep(step);
  }
  throw new Error(`timed out after ${timeout}ms waiting for ${what} (last: ${JSON.stringify(last)})`);
}

const readDisk = async () => JSON.parse(await fs.readFile(FILE, "utf8"));

/** Write the file the way Claude would: straight to disk, behind the app's back. */
async function writeDisk(scene) {
  await fs.writeFile(FILE, JSON.stringify(scene, null, 2) + "\n");
  await sleep(20);
}

const original = await fs.readFile(FILE);
await fs.writeFile(BACKUP, original);
console.log(`sketch e2e → ${BASE}  scene=${SCENE}`);
console.log(`  backup ${BACKUP}  sha256=${sha(original).slice(0, 16)}…`);

const browser = await chromium.launch();
const context = await browser.newContext({ viewport: { width: 1400, height: 900 } });
const page = await context.newPage();
const consoleErrors = [];
page.on("console", (msg) => {
  if (msg.type() === "error") consoleErrors.push(msg.text());
});
page.on("pageerror", (err) => consoleErrors.push(`pageerror: ${err.message}`));

const state = () =>
  page.evaluate(() => ({
    count: window.__sketch?.elementCount ?? -1,
    status: window.__sketch?.status,
    dirty: window.__sketch?.dirty,
    mtime: window.__sketch?.mtime,
    badge: document.querySelector(".badge")?.textContent,
    scrollX: window.excalidrawAPI?.getAppState().scrollX,
    scrollY: window.excalidrawAPI?.getAppState().scrollY,
    zoom: window.excalidrawAPI?.getAppState().zoom.value,
  }));

try {
  // ─── 1. load ───────────────────────────────────────────────────────────────
  await page.goto(`${BASE}/?f=${SCENE}`, { waitUntil: "domcontentloaded" });
  await page.waitForFunction(() => window.excalidrawAPI, null, { timeout: 20000 });

  await check("scene loads with all 75 elements on the canvas", async () => {
    const s = await until(async () => {
      const cur = await state();
      return cur.count === 75 ? cur : null;
    }, { what: "75 elements" });
    assert.equal(s.count, 75);
  });

  await check("excalidraw canvas is mounted and sized", async () => {
    const box = await page.locator("canvas.excalidraw__canvas.static").first().boundingBox();
    assert.ok(box && box.width > 500 && box.height > 400, `canvas box: ${JSON.stringify(box)}`);
  });

  await check("header shows the filename and a saved badge", async () => {
    const file = await page.locator("select").inputValue();
    assert.equal(file, SCENE);
    const badge = await until(async () => {
      const t = await page.locator(".badge").textContent();
      return t === "saved" ? t : null;
    }, { what: "saved badge" });
    assert.equal(badge, "saved");
  });

  // ─── 2. Claude redraws on disk → hot reload ────────────────────────────────
  await check("external file change is picked up by the poll (and viewport is preserved)", async () => {
    // Park the viewport somewhere distinctive so we can prove updateScene merges appState.
    await page.evaluate(() =>
      window.excalidrawAPI.updateScene({ appState: { scrollX: -321, scrollY: -654 }, captureUpdate: "NEVER" }),
    );
    await sleep(150);
    const before = await state();
    assert.equal(Math.round(before.scrollX), -321, "precondition: viewport parked");

    const disk = await readDisk();
    disk.elements.push({
      ...disk.elements.find((el) => el.type === "rectangle"),
      id: "claude-drew-this",
      x: 1500,
      y: 1500,
      index: "b000",
    });
    await writeDisk(disk);

    const after = await until(async () => {
      const cur = await state();
      return cur.count === 76 ? cur : null;
    }, { timeout: 8000, what: "the poll to pull 76 elements" });

    assert.equal(after.count, 76);
    assert.equal(after.dirty, false);
    assert.equal(Math.round(after.scrollX), -321, "scrollX must survive updateScene");
    assert.equal(Math.round(after.scrollY), -654, "scrollY must survive updateScene");
    const ids = await page.evaluate(() => window.excalidrawAPI.getSceneElements().map((e) => e.id));
    assert.ok(ids.includes("claude-drew-this"));
  });

  // ─── 3. owner draws in the browser → debounced autosave ────────────────────
  await check("a real mouse-drawn rectangle autosaves to disk within the debounce", async () => {
    const mtimeBefore = (await fs.stat(FILE)).mtimeMs;
    const box = await page.locator("canvas.excalidraw__canvas.interactive").first().boundingBox();
    await page.mouse.click(box.x + 1000, box.y + 780); // focus the canvas so shortcuts land
    await page.keyboard.press("r");
    assert.equal(
      await page.evaluate(() => window.excalidrawAPI.getAppState().activeTool.type),
      "rectangle",
    );
    await page.mouse.move(box.x + 320, box.y + 420);
    await page.mouse.down();
    await page.mouse.move(box.x + 520, box.y + 540, { steps: 12 });
    await page.mouse.up();

    const dirty = await until(async () => {
      const cur = await state();
      return cur.badge === "dirty" || cur.badge === "saving" ? cur : null;
    }, { timeout: 3000, what: "the dirty badge" });
    assert.ok(["dirty", "saving"].includes(dirty.badge));

    await until(async () => {
      const cur = await state();
      return cur.badge === "saved" ? cur : null;
    }, { timeout: 5000, what: "the badge to return to saved" });

    const disk = await readDisk();
    assert.equal(disk.elements.length, 77, "the drawn rectangle should be on disk");
    assert.ok((await fs.stat(FILE)).mtimeMs > mtimeBefore);
    assert.equal(disk.type, "excalidraw");
    assert.equal(disk.version, 2);
    assert.ok("viewBackgroundColor" in disk.appState);
    assert.deepEqual(
      Object.keys(disk.appState).filter((k) => !["viewBackgroundColor", "gridSize"].includes(k)),
      [],
      "no appState beyond background (+ carried-forward gridSize) may be written",
    );
  });

  await check("moving an element via the API also autosaves", async () => {
    const moved = await page.evaluate(() => {
      const api = window.excalidrawAPI;
      const els = api.getSceneElements();
      const target = els.find((el) => el.id === "claude-drew-this");
      api.updateScene({
        elements: els.map((el) =>
          el.id === target.id ? { ...el, x: el.x + 137, y: el.y + 11, version: el.version + 1 } : el,
        ),
        captureUpdate: "IMMEDIATELY",
      });
      return { id: target.id, x: target.x + 137 };
    });
    await until(async () => ((await state()).badge === "saved" ? true : null), {
      timeout: 6000,
      what: "the move to save",
    });
    const disk = await readDisk();
    const el = disk.elements.find((e) => e.id === moved.id);
    assert.ok(el, "moved element still on disk");
    assert.equal(el.x, moved.x);
  });

  // ─── 4. conflict ───────────────────────────────────────────────────────────
  /** Keep the scene continuously dirty (each edit resets the 800 ms debounce) and write the file
   *  behind the app's back, which is exactly the "both of us drew at once" case. */
  async function provokeConflict(theirCount) {
    await page.evaluate(() => {
      window.__nudge = setInterval(() => {
        const api = window.excalidrawAPI;
        const els = api.getSceneElements();
        api.updateScene({
          elements: els.map((el, i) => (i === 0 ? { ...el, x: el.x + 1, version: el.version + 1 } : el)),
          captureUpdate: "IMMEDIATELY",
        });
      }, 150);
    });
    // Wait until the canvas is genuinely dirty before touching the file, otherwise the app is
    // right to just pull our write (nothing local is pending yet) and no conflict exists.
    await until(async () => (await state()).dirty === true, { timeout: 4000, what: "the canvas to go dirty" });
    const disk = await readDisk();
    disk.elements = disk.elements.slice(0, theirCount);
    await writeDisk(disk);
    const theirBytes = await fs.readFile(FILE, "utf8");
    try {
      await until(async () => (await page.locator(".conflict").count()) > 0, {
        timeout: 10000,
        what: "the conflict banner",
      });
    } finally {
      await page.evaluate(() => clearInterval(window.__nudge));
    }
    await sleep(1200); // past the debounce — a conflicted app must still not write
    return theirBytes;
  }

  await check("conflict banner appears and blocks writes while both sides have moved", async () => {
    const theirBytes = await provokeConflict(40);
    const banner = await page.locator(".conflict").textContent();
    assert.match(banner, /changed on disk/);
    assert.equal(await page.locator(".badge").textContent(), "conflict");
    assert.equal(
      await fs.readFile(FILE, "utf8"),
      theirBytes,
      "a conflicted app must not write past the debounce",
    );
    const buttons = await page.locator(".conflict button").allTextContents();
    assert.deepEqual(buttons, ["keep mine — overwrite", "take theirs — discard mine"]);
  });

  await check('"take theirs — discard mine" adopts the file and drops local edits', async () => {
    await page.locator(".conflict button", { hasText: "take theirs" }).click();
    const s = await until(async () => {
      const cur = await state();
      return cur.count === 40 && cur.badge === "saved" ? cur : null;
    }, { timeout: 8000, what: "the file's 40 elements to load" });
    assert.equal(s.count, 40);
    assert.equal(s.dirty, false);
    assert.equal(await page.locator(".conflict").count(), 0);
    assert.equal((await readDisk()).elements.length, 40, "taking theirs must not rewrite the file");
  });

  await check('"keep mine — overwrite" pushes the canvas over the file', async () => {
    await provokeConflict(12);
    assert.equal((await readDisk()).elements.length, 12, "precondition: disk has their 12");
    const mine = await page.evaluate(() => window.excalidrawAPI.getSceneElements().length);
    await page.locator(".conflict button", { hasText: "keep mine" }).click();
    await until(async () => ((await state()).badge === "saved" ? true : null), {
      timeout: 8000,
      what: "the overwrite to land",
    });
    const disk = await readDisk();
    assert.equal(disk.elements.length, mine, `disk should hold my ${mine} elements`);
    assert.equal(await page.locator(".conflict").count(), 0);
  });

  // ─── 5. hygiene ────────────────────────────────────────────────────────────
  await check("the file dropdown lists design/*.excalidraw", async () => {
    const options = await page.locator("select option").allTextContents();
    assert.ok(options.includes(`${SCENE}.excalidraw`), `options: ${options}`);
  });

  await check("a scene created on disk appears in the dropdown and can be switched to", async () => {
    await fs.writeFile(
      ALT_FILE,
      JSON.stringify(
        {
          type: "excalidraw",
          version: 2,
          source: "sketch-e2e",
          elements: [
            { id: "alt-1", type: "rectangle", x: 10, y: 10, width: 80, height: 40, version: 1, seed: 7 },
          ],
          appState: { viewBackgroundColor: "#ffffff" },
          files: {},
        },
        null,
        2,
      ) + "\n",
    );
    await until(async () => (await page.locator("select option").allTextContents()).includes(`${ALT}.excalidraw`), {
      timeout: 8000,
      what: "the new scene to appear in the dropdown",
    });
    await page.locator("select").selectOption(ALT);
    const s = await until(async () => {
      const cur = await state();
      return cur.count === 1 && cur.badge === "saved" ? cur : null;
    }, { timeout: 8000, what: "the alt scene to load" });
    assert.equal(s.count, 1);
    assert.match(page.url(), new RegExp(`f=${ALT}`));

    await page.locator("select").selectOption(SCENE);
    const back = await until(async () => {
      const cur = await state();
      return cur.count > 1 && cur.badge === "saved" ? cur : null;
    }, { timeout: 8000, what: "the original scene to load back" });
    assert.ok(back.count > 1);
    assert.equal(JSON.parse(await fs.readFile(ALT_FILE, "utf8")).elements.length, 1, "switching away must not rewrite the other file");
  });

  await check("no console errors during the whole run", async () => {
    const noisy = consoleErrors.filter((t) => !/favicon|ERR_INTERNET_DISCONNECTED/i.test(t));
    assert.deepEqual(noisy, []);
  });
} finally {
  await browser.close();
  await fs.rm(ALT_FILE, { force: true });
  await fs.writeFile(FILE, original);
  const restored = await fs.readFile(FILE);
  const same = sha(restored) === sha(original);
  console.log(`  ${same ? "ok  " : "FAIL"} design/${SCENE}.excalidraw restored byte-identical (sha256=${sha(restored).slice(0, 16)}…)`);
  if (!same) failures++;
  await fs.rm(BACKUP, { force: true });
}

console.log(failures ? `\n${failures} FAILED` : "\nall e2e checks passed");
process.exit(failures ? 1 : 0);
