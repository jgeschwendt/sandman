import { useCallback, useEffect, useRef, useState } from "react";
import {
  CaptureUpdateAction,
  Excalidraw,
  MainMenu,
  restore,
  sceneCoordsToViewportCoords,
  viewportCoordsToSceneCoords,
} from "@excalidraw/excalidraw";
import "@excalidraw/excalidraw/index.css";
import {
  POLL_MS,
  SAVE_DEBOUNCE_MS,
  fetchScene,
  fetchVersion,
  listScenes,
  putScene,
  signature,
  toDiskShape,
} from "./sync.js";

const DEFAULT_SCENE = "sandman-v0";

const initialName = () => new URLSearchParams(location.search).get("f") || DEFAULT_SCENE;

/**
 * Run a file straight off disk through Excalidraw's own importer. Claude writes this JSON by hand,
 * so it may carry invalid fractional `index` keys, missing defaults or stale bindings — restore()
 * repairs all of that. Without it, one bad index throws inside the renderer and the canvas stops
 * accepting new shapes.
 */
function importScene(scene) {
  // `index` is a fractional-index key with a syntax no human writes correctly — a malformed one
  // ("b000") throws "invalid order key" deep in the renderer and silently kills shape creation.
  // Drop it and let restore() regenerate from array order, which is the z-order a hand-written
  // file actually means.
  const elements = (scene?.elements ?? []).map(({ index, ...el }) => el);
  const restored = restore({ elements, appState: null, files: scene?.files ?? {} }, null, null, {
    refreshDimensions: false,
    repairBindings: true,
  });
  return {
    elements: restored.elements,
    files: restored.files ?? {},
    viewBackgroundColor: scene?.appState?.viewBackgroundColor ?? "#ffffff",
  };
}

// ─── visor canvas anchor ──────────────────────────────────────────────────────

const LABEL_MAX = 60; // enough for the thread snippet in visor's panel

/** What visor shows as the thread's snippet: the shape's own text, its bound label, else its type. */
function labelFor(element, elements) {
  const bound = element.boundElements?.find((b) => b.type === "text");
  const text = element.text ?? (bound && elements.find((el) => el.id === bound.id)?.text);
  return text?.trim().slice(0, LABEL_MAX) || element.type;
}

/**
 * Visor pins comment threads to the drawn scene rather than to the canvas box: it hands us a click
 * and takes back an opaque anchor, then re-asks that anchor for viewport coords on every repaint —
 * so a pin rides pan, zoom and the shape itself. Both halves read `window.excalidrawAPI` at call
 * time, never at definition time: visor probes long before Excalidraw has mounted.
 */
window.__visorCanvas = {
  anchor(canvas, clientX, clientY) {
    const a = window.excalidrawAPI;
    if (!a) return null;
    const { x, y } = viewportCoordsToSceneCoords({ clientX, clientY }, a.getAppState());
    const elements = a.getSceneElements(); // already non-deleted; array order *is* z-order
    // Topmost first, so walk front-to-back. Bounds are axis-aligned — rotation is ignored because
    // these are wireframes, where being a few pixels off a tilted box still lands on the box.
    const hit = [...elements]
      .reverse()
      .find((el) => x >= el.x && x <= el.x + el.width && y >= el.y && y <= el.y + el.height);
    // x/y are the fallback: they still place the pin once the anchored element is deleted.
    return {
      v: 1,
      elementId: hit?.id ?? null,
      dx: hit ? x - hit.x : 0,
      dy: hit ? y - hit.y : 0,
      x,
      y,
      label: hit ? labelFor(hit, elements) : null,
    };
  },

  locate(canvas, payload) {
    const a = window.excalidrawAPI;
    if (!a || typeof payload?.x !== "number" || typeof payload.y !== "number") return null;
    const el = payload.elementId && a.getSceneElements().find((e) => e.id === payload.elementId);
    const sceneX = el ? el.x + payload.dx : payload.x;
    const sceneY = el ? el.y + payload.dy : payload.y;
    return sceneCoordsToViewportCoords({ sceneX, sceneY }, a.getAppState());
  },
};

export default function App() {
  const [name, setName] = useState(initialName);
  const [scenes, setScenes] = useState([]);
  const [boot, setBoot] = useState(null); // { initialData, source } — gates the first render
  const [status, setStatus] = useState("loading"); // loading | saved | dirty | saving | error
  const [conflict, setConflict] = useState(null); // { remoteMtimeMs }
  const [error, setError] = useState(null);

  const api = useRef(null);
  const savedSig = useRef(null); // signature of what we believe is on disk; null = not adopted yet
  const ourMtime = useRef(0);
  const applying = useRef(false); // true while we push a remote scene in — onChange must not react
  const dirty = useRef(false);
  const conflicted = useRef(false);
  const inFlight = useRef(false);
  const queued = useRef(false);
  const source = useRef("sandman-sketch");
  const saveTimer = useRef(null);
  const openFile = useRef(name); // the file the loop is currently bound to, readable from async code

  const snapshot = useCallback(() => {
    const a = api.current;
    if (!a) return null;
    const elements = a.getSceneElements();
    const appState = a.getAppState();
    const files = a.getFiles();
    return { elements, appState, files, sig: signature(elements, appState, files) };
  }, []);

  /** Adopt whatever is currently on the canvas as "clean" — used after load and after a remote pull. */
  const adoptClean = useCallback(() => {
    const snap = snapshot();
    if (!snap) return;
    savedSig.current = snap.sig;
    dirty.current = false;
    if (!conflicted.current) setStatus("saved");
  }, [snapshot]);

  // save() and scheduleSave() are mutually recursive; the ref breaks the definition cycle.
  const saveRef = useRef(null);
  const scheduleSave = useCallback(() => {
    clearTimeout(saveTimer.current);
    saveTimer.current = setTimeout(() => saveRef.current?.(), SAVE_DEBOUNCE_MS);
  }, []);

  const save = useCallback(async () => {
    const a = api.current;
    if (!a) return;
    if (inFlight.current) {
      queued.current = true;
      return;
    }
    const snap = snapshot();
    if (!snap) return;
    inFlight.current = true;
    setStatus("saving");
    try {
      const result = await putScene(name, toDiskShape(snap.elements, snap.appState, snap.files, source.current));
      // The owner may have switched files mid-flight; that write still had to land, but none of the
      // bookkeeping below belongs to the file now on screen.
      if (openFile.current !== name) {
        queued.current = false;
        return;
      }
      ourMtime.current = result.mtimeMs;
      savedSig.current = snap.sig;
      setError(null);
      // Anything drawn while the PUT was in flight leaves us dirty again.
      const now = snapshot();
      const stillDirty = now && now.sig !== savedSig.current;
      dirty.current = !!stillDirty;
      if (!conflicted.current) setStatus(stillDirty ? "dirty" : "saved");
      if (stillDirty) queued.current = true;
    } catch (err) {
      setError(String(err.message || err));
      setStatus("error");
      dirty.current = true;
    } finally {
      inFlight.current = false;
      if (queued.current && !conflicted.current) {
        queued.current = false;
        // Re-enter through the debounce, never straight back into a PUT: a continuous edit stream
        // must produce no writes at all until the owner pauses, or the poll can never see that
        // the file moved underneath us and a conflict would be silently overwritten.
        scheduleSave();
      }
    }
  }, [name, scheduleSave, snapshot]);

  useEffect(() => {
    saveRef.current = save;
  }, [save]);

  const raiseConflict = useCallback((remoteMtimeMs) => {
    conflicted.current = true;
    clearTimeout(saveTimer.current);
    queued.current = false;
    setConflict({ remoteMtimeMs });
    setStatus("conflict");
  }, []);

  /** Pull the file into the canvas. Viewport is untouched: updateScene merges appState. */
  const applyRemote = useCallback(async ({ force = false } = {}) => {
    const a = api.current;
    if (!a) return;
    const { scene, mtimeMs } = await fetchScene(name);
    if (openFile.current !== name || !api.current) return;
    // The fetch is where the time goes, so re-check: if the owner started drawing while it was in
    // flight, applying now would silently eat that stroke.
    if (!force && dirty.current) return raiseConflict(mtimeMs);
    const imported = importScene(scene);
    applying.current = true;
    try {
      if (Object.keys(imported.files).length) a.addFiles(Object.values(imported.files));
      a.updateScene({
        elements: imported.elements,
        appState: { viewBackgroundColor: imported.viewBackgroundColor },
        captureUpdate: CaptureUpdateAction.NEVER,
      });
      source.current = scene.source || source.current;
      ourMtime.current = mtimeMs;
    } finally {
      // updateScene renders asynchronously; adopt the resulting element versions, not the posted ones.
      setTimeout(() => {
        applying.current = false;
        adoptClean();
      }, 0);
    }
  }, [adoptClean, name, raiseConflict]);

  // ─── load / reload on file switch ──────────────────────────────────────────
  useEffect(() => {
    let live = true;
    openFile.current = name;
    api.current = null; // the old instance is about to unmount — stop all bookkeeping against it
    setBoot(null);
    setStatus("loading");
    setConflict(null);
    conflicted.current = false;
    dirty.current = false;
    queued.current = false;
    savedSig.current = null;
    (async () => {
      try {
        const { scene, mtimeMs } = await fetchScene(name);
        if (!live) return;
        ourMtime.current = mtimeMs;
        source.current = scene.source || "sandman-sketch";
        const imported = importScene(scene);
        setBoot({
          elements: imported.elements,
          appState: { viewBackgroundColor: imported.viewBackgroundColor },
          files: imported.files,
          scrollToContent: true,
        });
        setError(null);
      } catch (err) {
        if (!live) return;
        setError(String(err.message || err));
        setStatus("error");
      }
    })();
    return () => {
      live = false;
    };
  }, [name]);

  // Refreshed on the poll too, so a scene Claude creates shows up in the dropdown on its own.
  useEffect(() => {
    const refresh = () =>
      listScenes()
        .then((data) => setScenes(data.scenes ?? []))
        .catch(() => {});
    refresh();
    const id = setInterval(refresh, POLL_MS);
    return () => clearInterval(id);
  }, [name]);

  // ─── poll for Claude's edits ───────────────────────────────────────────────
  useEffect(() => {
    const tick = async () => {
      if (!api.current || applying.current || inFlight.current) return;
      try {
        const version = await fetchVersion(name);
        if (!version.exists || version.mtimeMs === ourMtime.current) return;
        // Both sides moved. Touch neither until the owner picks one.
        if (dirty.current) return raiseConflict(version.mtimeMs);
        await applyRemote();
      } catch (err) {
        setError(String(err.message || err));
      }
    };
    const id = setInterval(tick, POLL_MS);
    return () => clearInterval(id);
  }, [applyRemote, name, raiseConflict]);

  const onChange = useCallback(() => {
    // Ahead of every guard below: Excalidraw fires this on scroll and zoom too, and a scene we
    // pulled in from disk moves shapes just as a stroke does — visor must reproject its pins for
    // all of it. The guards are about *saving*, which none of those cases wants. Coalescing is
    // visor's job (one rAF per burst), so there is nothing to throttle here.
    window.dispatchEvent(new CustomEvent("visor:canvas-changed"));
    if (applying.current || savedSig.current === null) return;
    const snap = snapshot();
    if (!snap) return;
    if (snap.sig === savedSig.current) {
      if (dirty.current) {
        dirty.current = false;
        if (!conflicted.current) setStatus("saved");
      }
      return;
    }
    dirty.current = true;
    if (conflicted.current) return; // no writes until the conflict is resolved
    setStatus("dirty");
    scheduleSave();
  }, [scheduleSave, snapshot]);

  const keepMine = useCallback(() => {
    conflicted.current = false;
    setConflict(null);
    save();
  }, [save]);

  const takeTheirs = useCallback(() => {
    conflicted.current = false;
    setConflict(null);
    clearTimeout(saveTimer.current);
    queued.current = false;
    applyRemote({ force: true }).catch((err) => {
      setError(String(err.message || err));
      setStatus("error");
    });
  }, [applyRemote]);

  const switchScene = useCallback(
    (next) => {
      if (next === name) return;
      clearTimeout(saveTimer.current);
      if (dirty.current && !conflicted.current) save();
      const url = new URL(location.href);
      url.searchParams.set("f", next);
      history.replaceState(null, "", url);
      setName(next);
    },
    [name, save],
  );

  // Test/automation surface. The Playwright suite drives the loop through this.
  useEffect(() => {
    window.__sketch = {
      get api() {
        return api.current;
      },
      get status() {
        return status;
      },
      get dirty() {
        return dirty.current;
      },
      get mtime() {
        return ourMtime.current;
      },
      get conflict() {
        return conflict;
      },
      get elementCount() {
        return api.current ? api.current.getSceneElements().length : -1;
      },
      file: name,
      saveNow: save,
      keepMine,
      takeTheirs,
    };
  }, [conflict, keepMine, name, save, status, takeTheirs]);

  const badge = conflict ? "conflict" : status;

  return (
    <div className="sketch">
      <header className="bar">
        <span className="mark">sketch</span>
        <select value={name} onChange={(e) => switchScene(e.target.value)} aria-label="scene file">
          {scenes.length === 0 && <option value={name}>{name}</option>}
          {scenes.map((s) => (
            <option key={s.name} value={s.name}>
              {s.file}
            </option>
          ))}
        </select>
        <span className={`badge badge--${badge}`} data-status={badge}>
          {badge}
        </span>
        {error && <span className="err" title={error}>{error}</span>}
        <span className="spacer" />
        <span className="hint">design/ · autosaves {SAVE_DEBOUNCE_MS}ms after you stop · polls {POLL_MS / 1000}s</span>
      </header>

      {conflict && (
        <div className="conflict" role="alert">
          <strong>{name}.excalidraw changed on disk while you have unsaved edits.</strong>
          <button type="button" onClick={keepMine}>keep mine — overwrite</button>
          <button type="button" onClick={takeTheirs}>take theirs — discard mine</button>
        </div>
      )}

      <main className="canvas">
        {boot && (
          <Excalidraw
            key={name}
            initialData={boot}
            excalidrawAPI={(instance) => {
              api.current = instance;
              window.excalidrawAPI = instance;
              setTimeout(adoptClean, 0);
            }}
            onChange={onChange}
            UIOptions={{ canvasActions: { loadScene: false, saveToActiveFile: false, export: false } }}
          >
            <MainMenu>
              <MainMenu.DefaultItems.SearchMenu />
              <MainMenu.DefaultItems.SaveAsImage />
              <MainMenu.DefaultItems.ClearCanvas />
              <MainMenu.Separator />
              <MainMenu.DefaultItems.ToggleTheme />
              <MainMenu.DefaultItems.ChangeCanvasBackground />
            </MainMenu>
          </Excalidraw>
        )}
      </main>
    </div>
  );
}
