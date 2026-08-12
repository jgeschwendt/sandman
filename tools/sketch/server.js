#!/usr/bin/env node
// sketch — serves the built Excalidraw app plus a tiny file API over design/*.excalidraw.
// Zero dependencies on purpose: dist/ is committed, so running this needs no npm install.

import { createServer } from "node:http";
import { createReadStream } from "node:fs";
import fs from "node:fs/promises";
import path from "node:path";
import { fileURLToPath } from "node:url";
import { randomBytes } from "node:crypto";

const HERE = path.dirname(fileURLToPath(import.meta.url));
const DIST = path.join(HERE, "dist");
const DESIGN = process.env.SKETCH_DESIGN_DIR
  ? path.resolve(process.env.SKETCH_DESIGN_DIR)
  : path.resolve(HERE, "..", "..", "design");
const PORT = Number(process.env.SKETCH_PORT || 7873);
const HOST = process.env.SKETCH_HOST || "127.0.0.1";
const DEFAULT_SCENE = "sandman-v0";
const EXT = ".excalidraw";
const MAX_BODY = 32 * 1024 * 1024;

// appState keys that describe *where you are looking*, never *what is drawn*. Never persisted —
// they would churn every git diff.
const VOLATILE_APPSTATE = new Set([
  "collaborators",
  "cursorButton",
  "editingElement",
  "offsetLeft",
  "offsetTop",
  "openDialog",
  "openMenu",
  "openPopup",
  "openSidebar",
  "scrollX",
  "scrollY",
  "scrolledOutside",
  "selectedElementIds",
  "selectedGroupIds",
  "selectionElement",
  "width",
  "height",
  "zoom",
]);

const MIME = {
  ".css": "text/css; charset=utf-8",
  ".html": "text/html; charset=utf-8",
  ".js": "text/javascript; charset=utf-8",
  ".json": "application/json; charset=utf-8",
  ".map": "application/json; charset=utf-8",
  ".png": "image/png",
  ".svg": "image/svg+xml",
  ".ttf": "font/ttf",
  ".woff": "font/woff",
  ".woff2": "font/woff2",
};

class HttpError extends Error {
  constructor(status, message) {
    super(message);
    this.status = status;
  }
}

/**
 * Resolve `?f=` to an absolute path inside DESIGN. Traversal-safe: the name is reduced to its
 * basename, must be a plain filename, and the resolved path is re-checked against DESIGN.
 */
function scenePath(rawName) {
  const requested = (rawName ?? DEFAULT_SCENE).trim() || DEFAULT_SCENE;
  const base = path.basename(requested);
  if (base !== requested) throw new HttpError(400, "scene name must be a bare filename");
  if (base.startsWith(".")) throw new HttpError(400, "scene name must not start with a dot");
  const file = base.endsWith(EXT) ? base : base + EXT;
  if (!/^[A-Za-z0-9][A-Za-z0-9._-]*\.excalidraw$/.test(file)) {
    throw new HttpError(400, "scene name must be [A-Za-z0-9._-] and end in .excalidraw");
  }
  const full = path.join(DESIGN, file);
  if (path.dirname(full) !== DESIGN) throw new HttpError(400, "scene must live directly in design/");
  return { file, name: file.slice(0, -EXT.length), full };
}

function sendJson(res, status, body, headers = {}) {
  const payload = JSON.stringify(body);
  res.writeHead(status, {
    "content-type": "application/json; charset=utf-8",
    "cache-control": "no-store",
    ...headers,
  });
  res.end(payload);
  return true;
}

async function readBody(req) {
  const chunks = [];
  let size = 0;
  for await (const chunk of req) {
    size += chunk.length;
    if (size > MAX_BODY) throw new HttpError(413, "scene too large");
    chunks.push(chunk);
  }
  return Buffer.concat(chunks).toString("utf8");
}

/**
 * Canonical on-disk shape. Elements and the background come from the client; every other appState
 * key is carried forward from the file already on disk (so hand-authored keys like gridSize are
 * never silently dropped), minus the volatile viewport keys.
 */
function normalizeScene(incoming, existing) {
  if (!incoming || typeof incoming !== "object" || Array.isArray(incoming)) {
    throw new HttpError(400, "body must be a scene object");
  }
  if (incoming.type !== "excalidraw") throw new HttpError(400, 'body.type must be "excalidraw"');
  if (!Array.isArray(incoming.elements)) throw new HttpError(400, "body.elements must be an array");

  const elements = incoming.elements.filter((el) => el && typeof el === "object" && !el.isDeleted);

  const appState = {};
  for (const [key, value] of Object.entries(existing?.appState ?? {})) {
    if (!VOLATILE_APPSTATE.has(key)) appState[key] = value;
  }
  const background = incoming.appState?.viewBackgroundColor ?? appState.viewBackgroundColor ?? "#ffffff";
  appState.viewBackgroundColor = background;

  // Union the posted files with what is on disk, then keep only what elements still reference.
  const allFiles = { ...(existing?.files ?? {}), ...(incoming.files ?? {}) };
  const referenced = new Set();
  for (const el of incoming.elements) if (el?.fileId) referenced.add(el.fileId);
  const files = {};
  for (const id of Object.keys(allFiles).sort()) if (referenced.has(id)) files[id] = allFiles[id];

  return {
    type: "excalidraw",
    version: 2,
    source: incoming.source || existing?.source || "sandman-sketch",
    elements,
    appState,
    files,
  };
}

async function readScene(full) {
  const raw = await fs.readFile(full, "utf8");
  try {
    return JSON.parse(raw);
  } catch (err) {
    throw new HttpError(500, `${path.basename(full)} is not valid JSON: ${err.message}`);
  }
}

/** tmp + fsync + rename: a reader never observes a partial file. */
async function atomicWrite(full, text) {
  const tmp = path.join(path.dirname(full), `.${path.basename(full)}.${process.pid}.${randomBytes(4).toString("hex")}.tmp`);
  let handle;
  try {
    handle = await fs.open(tmp, "wx", 0o644);
    await handle.writeFile(text, "utf8");
    await handle.sync();
    await handle.close();
    handle = null;
    await fs.rename(tmp, full);
  } finally {
    if (handle) await handle.close().catch(() => {});
    await fs.rm(tmp, { force: true }).catch(() => {});
  }
}

async function handleApi(req, res, url) {
  const route = url.pathname;

  if (route === "/scenes" && req.method === "GET") {
    const entries = await fs.readdir(DESIGN, { withFileTypes: true });
    const scenes = [];
    for (const entry of entries) {
      if (!entry.isFile() || !entry.name.endsWith(EXT) || entry.name.startsWith(".")) continue;
      const stat = await fs.stat(path.join(DESIGN, entry.name));
      scenes.push({ name: entry.name.slice(0, -EXT.length), file: entry.name, mtimeMs: stat.mtimeMs, size: stat.size });
    }
    scenes.sort((a, b) => a.name.localeCompare(b.name));
    return sendJson(res, 200, { dir: DESIGN, default: DEFAULT_SCENE, scenes });
  }

  if (route === "/scene/version" && req.method === "GET") {
    const { name, file, full } = scenePath(url.searchParams.get("f"));
    try {
      const stat = await fs.stat(full);
      return sendJson(res, 200, { name, file, exists: true, mtimeMs: stat.mtimeMs, size: stat.size });
    } catch (err) {
      if (err.code !== "ENOENT") throw err;
      return sendJson(res, 200, { name, file, exists: false, mtimeMs: 0, size: 0 });
    }
  }

  if (route === "/scene" && req.method === "GET") {
    const { name, file, full } = scenePath(url.searchParams.get("f"));
    let stat;
    try {
      stat = await fs.stat(full);
    } catch (err) {
      if (err.code !== "ENOENT") throw err;
      throw new HttpError(404, `${file} not found in ${DESIGN}`);
    }
    const scene = await readScene(full);
    return sendJson(res, 200, scene, {
      "x-scene-mtime": String(stat.mtimeMs),
      "x-scene-name": name,
      "x-scene-file": file,
    });
  }

  if (route === "/scene" && req.method === "PUT") {
    const { name, file, full } = scenePath(url.searchParams.get("f"));
    const raw = await readBody(req);
    let incoming;
    try {
      incoming = JSON.parse(raw);
    } catch (err) {
      throw new HttpError(400, `body is not valid JSON: ${err.message}`);
    }
    // A missing or corrupt file is not a reason to refuse the write — it only means there is no
    // prior appState/files to carry forward.
    const existing = await readScene(full).catch((err) => {
      if (err.code !== "ENOENT") console.warn(`[sketch] ignoring unreadable ${file}: ${err.message}`);
      return null;
    });
    const scene = normalizeScene(incoming, existing);
    await atomicWrite(full, JSON.stringify(scene, null, 2) + "\n");
    const stat = await fs.stat(full);
    return sendJson(res, 200, {
      ok: true,
      name,
      file,
      mtimeMs: stat.mtimeMs,
      size: stat.size,
      elements: scene.elements.length,
      files: Object.keys(scene.files).length,
    });
  }

  return null;
}

async function serveStatic(req, res, url) {
  if (req.method !== "GET" && req.method !== "HEAD") throw new HttpError(405, "method not allowed");
  const rel = decodeURIComponent(url.pathname).replace(/^\/+/, "");
  let full = path.resolve(DIST, rel || "index.html");
  if (full !== DIST && !full.startsWith(DIST + path.sep)) throw new HttpError(403, "forbidden");
  let stat = await fs.stat(full).catch(() => null);
  if (stat?.isDirectory()) {
    full = path.join(full, "index.html");
    stat = await fs.stat(full).catch(() => null);
  }
  if (!stat) {
    // SPA fallback — anything unknown gets the app shell.
    full = path.join(DIST, "index.html");
    stat = await fs.stat(full).catch(() => null);
    if (!stat) {
      throw new HttpError(404, `dist/ not built. Run: mise exec -- npm run build (in ${HERE})`);
    }
  }
  const type = MIME[path.extname(full).toLowerCase()] || "application/octet-stream";
  const immutable = full.includes(`${path.sep}assets${path.sep}`);
  res.writeHead(200, {
    "content-type": type,
    "content-length": stat.size,
    "cache-control": immutable ? "public, max-age=31536000, immutable" : "no-cache",
  });
  if (req.method === "HEAD") return res.end();
  createReadStream(full).pipe(res);
}

const server = createServer((req, res) => {
  const url = new URL(req.url, `http://${req.headers.host || HOST}`);
  res.setHeader("x-content-type-options", "nosniff");
  Promise.resolve()
    .then(async () => {
      const handled = await handleApi(req, res, url);
      if (handled === null) await serveStatic(req, res, url);
    })
    .catch((err) => {
      const status = err instanceof HttpError ? err.status : 500;
      if (status === 500) console.error(`[sketch] ${req.method} ${req.url}`, err);
      if (!res.headersSent) sendJson(res, status, { error: err.message });
      else res.end();
    });
});

server.listen(PORT, HOST, () => {
  console.log(`[sketch] http://${HOST}:${PORT}  design=${DESIGN}  dist=${DIST}`);
});
