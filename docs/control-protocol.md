# Control protocol

`lightcraft --control 7980` (or `LIGHTCRAFT_CONTROL_PORT=7980`) starts a JSON-lines server on
`127.0.0.1:7980` (loopback only). One request per line, one reply per line, in order:

```text
→ {"id": 1, "method": "engine.execute", "params": {"command": "photo.rate", "params": {"rating": 4}}}
← {"id": 1, "ok": true, "result": null}
← {"id": 2, "ok": false, "error": "unknown command `nope`"}
```

Requests are answered on the UI thread between frames (timeout 60 s).

**Only requests are read.** Every line must be a JSON object with a string `method` (`id` and `params` are
optional; blank lines are skipped). Anything else — text that isn't JSON, a JSON array or number, an object without
`method`, invalid UTF-8, or a line longer than 4 MiB — gets one error reply
(`{"ok": false, "error": "… closing the connection"}`) and the server **closes the connection**; nothing sent after
it on that connection runs. This keeps an HTTP request (for example a web page's cross-origin `fetch` to
`127.0.0.1:<port>`) from smuggling a command in its body: its request line is rejected first. Junk never reaches the
UI thread, and at most 16 connections are served at once (further ones get an error line and are closed). Clients
that hit an error reply should reconnect. The port has no authentication, so only enable it when you need it. The MCP server's connect
mode ([mcp.md](mcp.md)) is a thin layer over this channel. Implementation:
`crates/ui-egui/src/control.rs` (methods) and `apps/lightcraft/src/control_server.rs` (transport).

## Methods

| Method | Params | Result |
|---|---|---|
| `engine.execute` (alias `ui.menu.invoke`) | `{command, params?}` | Run any engine or UI command (see `engine.commands`) |
| `engine.commands` | — | Engine + UI commands: id, label, menu, shortcut, params doc, enabled |
| `ui.menu.list` | — | Menu entries (flat: id, label, menu path, shortcut, enabled) |
| `ui.menu.tree` | — | The menu bar as shown (File … Help): items `{id, params?, label, shortcut?, enabled, checked?}`, separators, submenus — the model behind the native macOS menu bar and the in-window menus |
| `ui.inspect` | `{resetPerf?}` | UI state, window, canvas/image rects, `scroll: {grid, filmstrip}` (scroll offsets in points, `null` until drawn), active photo, selection, perf (`frameMs` = layout, `logicMs` = per-frame logic before it, `updateMs` = both, `maxUpdateMs`, `window` = frames and frames over 16.7 / 50 / 100 ms with the slowest (`maxMs`, `maxLogicMs`) since the last `ui.inspect {"resetPerf": true}` (see `docs/perf.md`), `fps`, render queue …, `gpu` = adapter in use, `gpuReason` = why renders don't use the GPU, `gpuFallback` = latest render redone on the CPU and why — see `docs/gpu-pipeline.md`), status, memory (bytes per cache, see `library.memory`; plus stage caches and textures), `export: {running: {total, done, current} \| null, last}`, `notices` (warnings waiting to be shown, e.g. a damaged settings file; OK = `button:noticeOk`), `quitPrompt` (why quitting was stopped: unsaved changes; `button:quitRetry` / `button:quitAnyway` / `button:quitCancel`), `import: {done, total, imported, cancelled} \| null` (an import runs on a worker thread), `tasks` (other background work: `Find Missing Photos`, `Auto Import`) |
| `ui.widgets` | `{filter?}` | On-screen widgets `{id, rect: [x, y, w, h]}` (screen points) |
| `ui.clickWidget` / `ui.dragWidget` / `ui.hoverWidget` | `{id, count?, fx?, fy?}` / `{id, toX?, toY?, dx?, dy?, steps?}` / `{id, fx?, fy?}` | Real egui input on a widget (hover: the pointer rests on it, e.g. for preset/profile previews) |
| `ui.move` / `ui.click` / `ui.drag` | `{x, y, count?, button?: left\|right\|middle\|back\|forward}` / `{x, y, toX, toY, steps?}` | Raw pointer input, screen points (`back` / `forward` = the mouse's thumb buttons) |
| `ui.pointer` | `{events: [{kind: down\|drag\|up, x, y}], alt?, shift?, cmd?}` | Gesture in normalized image coordinates (Detail view) |
| `ui.key` | `{key, cmd?, shift?, alt?, ctrl?}` | Key press |
| `ui.text` | `{text}` | Text input |
| `ui.scroll` | `{dx, dy, unit?, cmd?, ctrl?, shift?, alt?}` | Wheel / two-finger scroll at the current pointer. `unit`: `point` (a trackpad, default) or `line` (mouse-wheel notches, 1 = one notch). Over the image it zooms or pans as `view.wheelZoom` says (default: a notched wheel zooms, a trackpad pans); ⌘-scroll zooms |
| `ui.zoom` | `{factor}` | Pinch zoom at the current pointer (positive scale multiplier; 1 = unchanged). Position it first with `ui.move` or `ui.hoverWidget` |
| `ui.set` | partial UI state, e.g. `{"view": "detail"}` | Resulting UI state |
| `ui.dialog.confirm` / `ui.dialog.cancel` | — | Close the open dialog |
| `ui.resize` | `{width, height}` | Resize the window |
| `ui.screenshot` | `{path?, headless?}` | `{path, width, height}` once the frame (with finished renders) is captured. `headless: true` draws the UI on the CPU (no compositor needed); a windowed capture that gets no frame within 2 s falls back to headless automatically |
| `engine.execute {command: "app.export", params}` | export params (see `docs/mcp.md`), plus `preset`, `dir` / `path`, `ids`, `background` | Writes the files and returns `{files}`; with `background: true` (what the Export dialog and menus use) it returns `{background: true, total}` at once and the batch runs on a worker thread — poll `ui.inspect` → `export` |
| `ui.render` | `{id?, size?, path?}` | Render a photo (PNG to `path`), `{width, height}` |
| `app.quit` | — | Close the app |
| `engine.execute {command: "keys.list"}` | — | The keymap (Settings ▸ shortcuts. / controllers.): `{path, error, file, rows, recording, learn, lastMidi, midiStatus}` — `rows` = every binding with its layer (`builtin` / `profile` / `user`), `overridden`, `conflict` |
| `keys.set` / `keys.add` / `keys.record` | partial `keymap.json` (`profile: lightroom\|classic`, `bindings`, `midi`, `midiEnabled`, `steps`) / `{keys, command, params?, label?}` / `{command?, params?, label?, index?}` | Change the keymap (saved to `<config>/keymap.json`); `keys.record` makes the next key press (or mouse back/forward/middle click) the binding's keys |
| `keys.import` | `{source: lrkeys\|superkeys\|monogram, path?, profile?, write?}` | Read LrKeys `bindings.json`, LrSuperKeys `Shortcuts.xml`+`SpeedKeys.xml`, or a Monogram `state.json` profile (read-only) into the keymap → `{mapped, skipped, …}`; `write` saves the generated Monogram profile |
| `keys.midi` / `keys.learn` | `{midi: cc\|note, number, value?, channel?}` or `{bytes}` / `{on}` | A MIDI message as if from the MIDI input → what it did / MIDI learn |
| super keys | `keys.nudge {control, dir, size: small\|large}` · `keys.macro {steps: [{command, params}]}` · `keys.send {keys}` · `preset.applyByName {name}` · `crop.nudge {x?, y?, scale?}` (engine: one axis only, about the centre) · `view.develop` · `view.colorMixer {mode}` · `view.resumeLastLeftOff` | Commands for keys, MIDI and the control channel (see `crates/ui-egui/src/keymap.rs`, `leftoff.rs`) |
| hardware buttons | `view.section {id, control?}` · `tool.targetedAdjustment {target, on?}` (via `engine.execute`) | For Stream Deck / Monogram buttons (Hammerspoon → this port); both are idempotent, so pressing twice leaves the same state. `view.section`: switches to Develop with the Edit panel and **opens** the Develop panel `id` (`basic`, `toneCurve`, `hsl`, `colorGrading`, `detail`, `lensCorrections`, `transform`, `effects`, `lensBlur`, `calibration`, or the older `light`, `color`, `mixer`, `curve`, `grading`, `optics`, `geometry`), never closes it, and scrolls it into view. With `control` (a develop control id, e.g. `light.shadows`, `mixer.blue.sat`) it also picks that slider as a click on its name does (highlighted; ↑ ↓ and the wheel change it, ← → step to its neighbours), opens its sub-section, shows the mixer tab that holds it, and scrolls to it; `id` may be left out then (the control's own panel). A control that isn't in `id`'s panel, an unknown `id` or `control` → error. A control the panel wouldn't draw (a B&W photo's `mixer.*`, a colour photo's `bw.*`, `optics.profile*` with Enable Profile Corrections off, `hdr.*` with HDR off, `lensBlur.*` before Apply, `pointColor.<n>.*` for a point that doesn't exist, Color Grading's wheels, `crop.angle`…) is an error that says why (`` `bw.red` isn't shown: the photo is in colour… ``) and changes nothing; a slider that is drawn but greyed out opens and scrolls to its section and returns a `note` with `picked: null`. The result's `picked` is the slider id as drawn. While a slider is picked, **Tab** / **Shift+Tab** step down / up the sliders of its panel (greyed-out ones skipped), round the end, scrolling only if the next one is out of view; ← → still walk every slider on screen; with nothing picked Tab hides the side panels as before. `tool.targetedAdjustment`: `target` = `curve` \| `hue` \| `saturation` \| `luminance`; arms that targeted adjustment tool exactly as the panel's button does (hue / saturation / luminance first set the HSL mixer mode and open HSL / Color; `curve` opens Tone Curve); `on` defaults to `true` and is not a toggle, `on: false` puts any targeted adjustment tool away. Code: `crates/ui-egui/src/device_cmds.rs` |
| Monogram inputs | `keys.lrinput {line}` · `develop.listControls {section?}` | One line of Monogram's Lightroom protocol (`keys.lrinput`) and the catalog of every develop control; see *Monogram console inputs* below |
| merges. | `merge.find {path, deep?, background?}` · `merge.candidates {path, deep?, state?: open\|merged\|dismissed\|all}` · `merge.thumbs {ids, size?: 320, render?}` · `merge.mergeGroups {ids, kind?, frames?, options?}` (`options`: the Merge box's choices, camelCase — `projection: auto\|spherical\|cylindrical\|perspective`, `boundaryWarp: 0–100`, `autoCrop`, `crop: [x0, y0, x1, y1]` (normalized; the merged photo's crop, in place of the auto-crop one — the Merge box's draggable crop frame; also on `merge.hdr` / `merge.panorama` / …), `fillEdges`, `deghost: none\|low\|medium\|high`, `align`, `stack`, `autoSettings`, `bracket`; a missing key = Settings ▸ merge.'s default for the group's kind; in the app → `{queued}`, the groups merged one at a time on the worker) · `merge.dismiss {ids}` (send.back.) · `merge.restore {ids}` · `merge.setFrames {id, photos}` · `merge.setKind {id, kind: hdr\|pano\|stack\|hdrPano}` · `merge.showIn {ids, where: both\|merges}` · `merge.unmerge {ids}` · UI: `merge.findMode {mode: weddings (auto)\|every\|off (find)}`, `merge.settings {<partial Settings ▸ merge.: panorama\|hdr\|hdrPanorama\|focusStack: {mergesOnly, stack, autoSettings, autoCrop, projection, deghost, hideOriginals}, showFrom}}` → all of them, `merge.layout {layout?: waves\|grid\|strip\|focus, aspect?: original\|square\|3:2\|4:5\|16:9, perRow?: 2–8}` (`perRow`: groups per row, each group a tile, 4 by default; per screen size: small < 1300, medium, large, huge > 2600 points; aspect shapes only the cards), `merge.cardSize {perRow?\|step?: ±1}` (smaller. / bigger.), `merge.filter {kinds?, confidences?, from?, camera?, clear?}`, merge. page: `merge.step {by: ±1}` (← / →), `merge.mergePicked` (↩ / M: the Merge box for the picked groups), `merge.openGroups {ids}`, `merge.mergeNow` (no box), `merge.sendBackPicked` (S), `merge.pickAll` (⌘A) (all via `engine.execute`) | Found photo merges (`docs/merge.md`). A shoot folder with camera subfolders is one pool: pass `deep: true`. `merge.candidates` → `{groups: [Group], running}`, Group = `{id, kind, tag, photos (the frames merge. merges), state, result, folder, found, confidence: sure\|likely\|maybe, reasons: [string], frames: [{id, file, folder, subfolder (relative to the pool's folder, "" if none), camera, captured, exposure ("1/250 f/8 ISO 100"), inGroup}], cameras: [string], subfolders: [string]}`, oldest first. `merge.thumbs` → `{thumbs: {"<id>": "/abs/path.jpg"}}`: JPEGs from the thumbnail cache in `<library>/merges-thumbs/`, ids without a cached thumbnail left out (`render: true` renders them). `merge.setFrames` → `{id, photos, out}`; `merge.restore` → `{restored}`. Widgets: `mergeTile:<id>` (a group's tile), `group:merges:group:<id>` (header: picks the group), `mergeFilter:kind|confidence|camera` (the bar's drop-downs; rows `menu:mergesFilterKind-<kind>|all|none`, `…Confidence-<c>`, `…Camera-<n>`), `mergeWave:<id>`, `mergeCard:<id>:<photo>`, `button:mergesKind-<id>` / `mergesGroupMerge-<id>` / `mergesGroupSendBack-<id>` / `mergesRestore-<id>`, `chip:mergesConfidence-<id>`, `fold:merges.group:<id>` / `merges.spread:<id>` / `merges.sentBack`, bar `button:mergesFind` … `mergesBigger` |
| fix notes | `notes.list {status?}` · `notes.add {text, shot?}` · `notes.send {ids?}` · `notes.markFixed {ids, note?}` · `notes.reopen` · `notes.edit` · `notes.remove` · `notes.select` · `notes.toggle {open?}` (all via `engine.execute`) | The notes. panel's notes about what isn't working and the fix requests it writes for the Claude session: `docs/fix-notes.md` |

Image navigation is also available directly as the UI command `view.navigate`, with
`{zoom?: "fit" | "fill" | {"percent": number}, pan?: [x, y]}`. Percentage zoom accepts fractional
values greater than 0 and at most 800; pan is the normalized image centre, with coordinates from 0 to 1.
Pinching keeps the image point under the pointer steady and zooms between Fit and 800%; two-finger
scrolling pans in both axes and respects the operating system's scrolling direction and momentum.
These gestures work in Detail (including editing tools and full-screen preview), Compare and Reference
views, and only apply over their image areas. Panning stops at the image edges.

### When the library can't be saved

With a persistent library, every command that changes something is written to the catalog journal (fsynced) before
it replies. If that write fails (disk full, volume gone, permissions), the command replies `ok: false` with
`"saved in memory but not written to disk: <reason>; LightCraft will retry"`. The change itself **is** applied (and
undoable) and stays queued: the next command, and the app's frame loop every couple of seconds, retry the write, so
nothing is lost once the disk is writable again — unless the app quits first. Meanwhile `ui.inspect` → `unsaved`
is `{ops, error}` (else `null`), `library.info` reports `unsavedOps` / `unsavedError`, and the top bar's cloud icon
shows a warning (widget `indicator:unsaved`). Queries and commands that change nothing still succeed. A failed
compaction (snapshot) is not a failed command — the log is kept whole — and only shows in `library.info` →
`lastError`.

### When the library can't be opened

If the desktop app can't open its library at launch (another program has it open, an unreadable or newer-format
catalog, a missing drive), the session starts empty and in memory — never with demo photos — and a window asks what
to do: `ui.inspect` → `libraryProblem` is `{path, error, temporarySession, pendingImport}` (else `null`); its buttons
are `button:libraryRetry`, `button:libraryChoose`, `button:libraryTemporary` (Continue Without Saving) and
`button:libraryQuit`. A temporary session shows a banner (`indicator:temporarySession`, `button:libraryReopen`) and
writes nothing. `app.openLibrary` opening a library ends it.

## Monogram console inputs

Monogram's Lightroom integration sends one text line per move; LightCraft reads them with `keys.lrinput {line}`
(`crates/ui-egui/src/keymap.rs`, `lr_input`). The number after `=` says how it moved: a slider `128,min,max`
(hardware 0–255 across the range), a dial `^0.05,min,max` (up) / `v0.05,min,max` (down, a digit follows the `v`:
`lc_cycle = vertical` is a list name, not a dial), or nothing / `reset` (a plain press: back to the default).

- **The numbers do what Monogram's Lightroom plugin does** (decoded from its bytecode), each line one change on the
  value read just then: a **dial** `^d,min,max` adds `d` (Temperature: `d` × 25 K up to 8000 K, ×50 to 10000, ×100
  to 14000, ×400 beyond), rounds to the control's decimals, and holds the result to the narrower of the line's range
  and the control's; an absolute **pot** `pos,min,max` (or a bare `pos`: the control's range) is linear across that
  range with dead zones (`pos` ≤ 3 is the minimum, ≥ 252 the maximum), never picks up, and Temperature rides
  Monogram's warped track (128 of 255 on 2000–50000 K is ≈ 6943 K); `name = +` / `-` steps a thousandth of the range
  (25 at most), `++` / `--` one native nudge; a colour wheel's dial turns the wheel: the point (sat·cos hue,
  sat·sin hue) moves `d` along X or Y and comes back as hue and saturation together (a wheel's pot is still X =
  saturation, Y = hue).
- **Lightroom names** (`Exposure`, `Tint`, …; the table is `LR_PARAMS`): the colour-grading wheels
  `ShadowsWheelX/Y`, `MidtonesWheelX/Y`, `HighlightsWheelX/Y`, `GlobalWheelX/Y` (X = the wheel's saturation 0–100,
  Y = its hue 0–360) and `ParametricShadowSplit` / `ParametricMidtoneSplit` / `ParametricHighlightSplit` are in it.
- **`lc_set_<control id> = …`: every develop control.** The id is any `id` of `develop.listControls`, indexed ones
  too: `lc_set_hdr.maxEv = 128,0,5` (slider), `lc_set_curve.splitMid = ^2,20,80` (dial), `lc_set_light.exposure = 0.5`
  (a bare number: the value itself, in the control's units, clamped to its range), `lc_set_pointColor.1.hueShift =`
  (reset). An unknown id (`crop.x` / `crop.y` / `crop.scale` only nudge, so they aren't one), a list element that
  doesn't exist on the active photo (`pointColor.7.…`), or a number that is not finite (`nan`, `inf`, a word) is an
  error, never a reset. It is the same code path as the Lightroom names (a console drag: one history step once the
  hand is still; a reset or a one-shot ends the drag first, so it is a step of its own).
- **`local_*`: the mask sliders.** `local_Temperature`, `Tint`, `Exposure`, `Contrast`, `Highlights`, `Shadows`,
  `Whites`, `Blacks`, `Texture`, `Clarity`, `Hue`, `Saturation`, `Sharpness`, `LuminanceNoise`, `Moire`,
  `Defringe`, `Dehaze` set the same-named slider of the *selected* mask (`mask.select`; an error if none).
  A slider spans LightCraft's own range, so Monogram's ±180 `local_Hue` covers ±100. (`local_ToningLuminance`
  of the Lightroom plugin has no LightCraft slider.)
- **`reset = <tool>`**: `spotremoval`, `redeye`, `gradient`, `circularGradient`, `brushing` run
  `develop.resetLocalTool {tool: spots|redEye|linear|radial|brush}` (deletes every one of that kind, one undo
  step; a mask is of the kind of its first component, empty masks go with any mask tool → `{removed, kinds}`);
  `crop` runs `crop.reset`; `alldevelop` is ⌘⇧R.
- **`lc_cmd = <command id> [JSON params]`** runs any command, the way its key does (`shortcuts::dispatch`: key
  aliases, culling and advance, panel toggles), and reports the error of a command that fails or doesn't exist.
  Several commands separated by ` ; ` (a space, a semicolon, a space; one inside a JSON string doesn't count) run in
  order: `lc_cmd = view.colorMixer {"mode": "hue"} ; tool.targetedAdjustment {"target": "hue"}`. Every step is
  parsed before any runs (an empty step is an error); a slider move in progress is ended before each step; the
  first step that fails stops the chain and is reported (`lc_cmd: step 2 \`…\` failed (1 ran before it): …`). The
  reply is `{run}` for one command, `{ran, commands}` for a chain.
- **`lc_cycle = <list>`**: step a cycle list (`keys.cycles`); the toast shows just the item's label.

`develop.listControls {section?}` (no photo needed; an unknown section is an error) → `{count, controls: [{id, label, section, sectionLabel, min,
max, default, step, decimals, indexed}]}`: one row per entry of `controls.rs`; an `indexed` row is a template
(`pointColor.hueShift`, `redEye.pupilSize`) that addresses list element *n* as `pointColor.<n>.hueShift`.
(`develop.controls` lists the photo's current values, with the elements that exist.)

## Headless rendering (no window, no GPU)

The egui UI can be rasterized on the CPU (`crates/ui-egui/src/softpaint.rs`, driven by
`crates/ui-egui/src/headless.rs`): same tessellation, gamma-space premultiplied blending and
scissor clipping as egui's GPU backends, so the image matches the window (minus GPU dithering).

- **In the running app:** `{"method": "ui.screenshot", "params": {"path": "a.png", "headless": true}}`.
  The UI is drawn into an offscreen context from the app's logic tick, which keeps running when the
  window is occluded or the display sleeps/locks. Windowed screenshots fall back to this after 2 s.
- **Without any app window:** `lightcraft-cli snapshot` runs a whole app session headlessly and
  answers the same control requests (same handler) from a JSON-lines script:

```text
lightcraft-cli snapshot --demo -o grid.png --size 1600x1000 [--scale 2]
lightcraft-cli snapshot --library DIR --script tour.jsonl -o shot.png
```

  `tour.jsonl` holds one request per line (`#` comments allowed), e.g.
  `{"method": "ui.set", "params": {"view": "detail", "right": "edit", "openSections": ["optics"]}}`
  then `{"method": "ui.screenshot"}`. Replies are printed to stdout. A failed request
  (`"ok": false`) does not stop the script — the remaining lines and the final screenshot still
  run — but the exit status is non-zero when any request failed, as with `run --keep-going`, so
  CI and nightly runs can judge a snapshot by its exit status. A `ui.screenshot` without
  `path` writes `-o` (then `OUT-2.png`, `OUT-3.png`, …); `ui.settle {timeoutMs?}` waits until no
  renders are in flight. Each request runs frames until it is answered and its injected input
  (clicks, keys, drags) has played out. Widget ids for `ui.clickWidget` come from `ui.widgets`
  (e.g. `button:upright-auto`). A 1600×1000 demo snapshot takes ~1–2 s (debug build).
