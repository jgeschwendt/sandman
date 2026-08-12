// The pair-drawing loop, minus React. Everything that decides *when* to read or write the file
// lives here so the rules are readable in one place.

export const SAVE_DEBOUNCE_MS = 800;
export const POLL_MS = 2000;
export const SOURCE = "sandman-sketch";

const qs = (name) => `?f=${encodeURIComponent(name)}`;

async function failure(res) {
  const body = await res.text();
  let message = body;
  try {
    message = JSON.parse(body).error ?? body;
  } catch {}
  return new Error(`${res.status} ${message}`);
}

export async function listScenes() {
  const res = await fetch("/scenes");
  if (!res.ok) throw await failure(res);
  return res.json();
}

export async function fetchScene(name) {
  const res = await fetch(`/scene${qs(name)}`);
  if (!res.ok) throw await failure(res);
  const mtimeMs = Number(res.headers.get("x-scene-mtime") || 0);
  return { scene: await res.json(), mtimeMs };
}

export async function fetchVersion(name) {
  const res = await fetch(`/scene/version${qs(name)}`);
  if (!res.ok) throw await failure(res);
  return res.json();
}

export async function putScene(name, scene) {
  const res = await fetch(`/scene${qs(name)}`, {
    method: "PUT",
    headers: { "content-type": "application/json" },
    body: JSON.stringify(scene),
  });
  if (!res.ok) throw await failure(res);
  return res.json();
}

/**
 * A cheap fingerprint of "what is drawn". Element `version` is bumped by Excalidraw on every
 * mutation, so summing them catches edits without deep-comparing the scene. Deliberately blind to
 * viewport, selection and tool state — those must never count as a dirty edit.
 */
export function signature(elements, appState, files) {
  let versions = 0;
  let geometry = 0;
  for (const el of elements) {
    versions += el.version ?? 0;
    // Geometry is a backstop: a programmatic updateScene can move an element without bumping
    // `version`, and that must still register as an edit.
    geometry += (el.x ?? 0) + (el.y ?? 0) + (el.width ?? 0) + (el.height ?? 0) + (el.angle ?? 0);
  }
  const fileIds = Object.keys(files ?? {}).sort().join(",");
  return `${elements.length}:${versions}:${geometry.toFixed(3)}:${appState?.viewBackgroundColor ?? ""}:${fileIds}`;
}

/** The exact JSON we write. Nothing viewport-shaped goes in. */
export function toDiskShape(elements, appState, files, source = SOURCE) {
  return {
    type: "excalidraw",
    version: 2,
    source,
    elements,
    appState: { viewBackgroundColor: appState?.viewBackgroundColor ?? "#ffffff" },
    files: files ?? {},
  };
}
