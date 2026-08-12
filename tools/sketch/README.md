# sketch — pair-drawing on `design/*.excalidraw`

A local Excalidraw wired straight to the repo's `design/` directory, so the owner and Claude can
draw on the same file at the same time.

- **Owner** draws in the browser. Edits autosave to `design/<name>.excalidraw` 800 ms after the
  pointer goes quiet. No Open/Save dialogs — the file *is* the document.
- **Claude** edits the same `.excalidraw` JSON with ordinary file tools. The page polls the file's
  mtime every 2 s and pulls the change onto the canvas without touching the viewport.

Port **7873**. Scene defaults to `sandman-v0`; `?f=<name>` or the header dropdown switches files.

## Run

```sh
tools/sketch/run.sh            # serves the pre-built dist/ — no npm needed
open http://127.0.0.1:7873/
```

Through visor (a local reverse proxy that adds an on-page comment rail), from the
repo root — 9001, because 9000 is taken on my machine:

```sh
visor -p 9001 7873             # writes .visor/ into cwd (gitignored)
open http://127.0.0.1:9001/
```

`dist/` is committed, so a fresh clone runs with nothing but node. Rebuild only after changing
`src/`:

```sh
cd tools/sketch
mise exec -- npm install       # not `npm ci` — the lockfile carries cross-platform optional deps
mise exec -- npm run build     # vite build + copies Excalidraw's fonts into dist/fonts
```

### Hacking on the tool itself

```sh
mise exec -- node server.js    # terminal 1 — the file API on 7873
mise exec -- npm run dev       # terminal 2 — Vite HMR on 7874, proxying /scene* to 7873
```

### Tests

```sh
mise exec -- node test/api.mjs   # file API: traversal, round-trip, atomic-write proof (scratch scene)
mise exec -- node test/e2e.mjs   # Playwright: the whole loop against the real sandman-v0, restored after
```

## @claude mentions

A text element containing `@claude` (any case) is a request addressed to Claude. A session
watching the scene polls the file's mtime (~2 s, the app's own cadence) and keys each request
on `(element id, ±60 chars of whitespace-collapsed context around the mention)` — so editing
the question itself re-raises it, while keystroke autosaves and edits elsewhere in the same
element stay quiet, and deleting the element raises nothing. Claude answers on the canvas,
near the mention; removing the `@claude` phrase retires the request.

## The conflict rule

Last-writer-wins is fine; **silent loss is not**.

| local state | file changed on disk | what happens                                    |
| ----------- | -------------------- | ----------------------------------------------- |
| clean       | yes                  | pull it — `updateScene`, viewport untouched     |
| dirty       | yes                  | **stop.** Banner, two buttons, nothing is written |
| dirty       | no                   | debounced PUT 800 ms after the last edit        |

While the conflict banner is up the app writes nothing and pulls nothing:

- **keep mine — overwrite** → PUT the canvas over the file.
- **take theirs — discard mine** → refetch and replace the canvas.

A continuous edit stream produces *no* writes until you pause — that is deliberate. An earlier
version re-PUT immediately after each save while edits kept arriving, which meant a browser that
was being drawn on constantly overwrote Claude's file before the poll could ever notice it moved.

The badge in the header reads `saved` / `dirty` / `saving` / `conflict` / `error`.

## On-disk format

```jsonc
{ "type": "excalidraw", "version": 2, "source": "...", "elements": [...],
  "appState": { "viewBackgroundColor": "#ffffff" }, "files": {} }
```

- **Written**: `elements` and `viewBackgroundColor`. Nothing viewport-shaped (`scrollX`, `scrollY`,
  `zoom`, selection, open dialogs) ever reaches the file — panning must not show up in a git diff.
- **Carried forward**: any *other* `appState` key already in the file (e.g. `gridSize`) survives a
  write, so hand-authored settings are never silently dropped.
- **`files`** (embedded images) round-trips; entries no element references are pruned.
- Elements with `isDeleted: true` are dropped on write.
- Indented with 2 spaces (Excalidraw's own export format). The first save of a file written by
  another tool will therefore reformat it once.

## Writing scenes as Claude

Edit `design/<name>.excalidraw` directly; the browser picks it up within 2 s.

**Omit the `index` field on elements you add.** It is a fractional-index key with a syntax you will
get wrong — a malformed one (e.g. `"b000"`) throws `invalid order key` inside Excalidraw's renderer
and silently stops the canvas from accepting new shapes. The app strips `index` from everything it
loads and lets Excalidraw regenerate the keys from **array order**, so: *position in the `elements`
array is the z-order.* (Consequence: the first save after a hand-edit rewrites every `index` to
Excalidraw's canonical form — `a0`, `a1`, … — once.)

Otherwise the usual element shape applies: `id`, `type`, `x`, `y`, `width`, `height`, `version`,
`seed`, `strokeColor`, … Missing defaults are filled in by Excalidraw's `restore()` on load.

## API

| method | route                    | behavior                                                        |
| ------ | ------------------------ | --------------------------------------------------------------- |
| GET    | `/scenes`                | `{dir, default, scenes:[{name,file,mtimeMs,size}]}`              |
| GET    | `/scene?f=<name>`        | the file's JSON; `x-scene-mtime` header carries the mtime        |
| PUT    | `/scene?f=<name>`        | normalize + atomic write (tmp→fsync→rename); returns new mtime   |
| GET    | `/scene/version?f=<name>`| `{exists, mtimeMs, size}` — the cheap poll                       |

`f` is reduced to a basename, must match `[A-Za-z0-9][A-Za-z0-9._-]*`, may not start with a dot, and
resolves inside `design/` or the request is refused. `.excalidraw` is appended if absent.

## Known edges

- The pull/edit race is narrowed, not eliminated: a remote change fetched in the same instant the
  first stroke lands is caught by a re-check after the fetch, but the poll is still a poll.
- CJK text renders in a fallback font — Excalidraw's Xiaolai font is 12 MB and is left out of
  `dist/` on purpose. `SKETCH_FONTS=all mise exec -- npm run build` includes it.
- `dist/` is ~8.6 MB, most of it lazily-loaded chunks (mermaid, katex, cytoscape) that the
  text-to-diagram dialog would need and this tool never fetches at runtime.

## Layout

```
server.js            zero-dependency node http server: static dist/ + the file API
run.sh               the run story
src/App.jsx          the sync loop (load, debounce-save, poll, conflict)
src/sync.js          fetch/PUT helpers, the dirty-signature, the on-disk shape
scripts/copy-fonts.mjs   copies Excalidraw's runtime fonts into dist/
test/api.mjs         file API + atomic write tests
test/e2e.mjs         Playwright test of the full loop
```
