# Fix notes (LC-UI-FIXNOTES)

A way to tell the Claude Code session that builds LightCraft what isn't working, from inside the app.

1. **notes.** in the top bar (next to *left off.*; a blue badge counts the open notes) or **⌘⌥⇧N**
   (Window ▸ Fix Notes) opens a small floating panel. It doesn't block anything: move it anywhere
   and keep working.
2. Type what's wrong; **Enter** adds the note (**⇧Enter** starts a new line). Each note records where
   it was written (below), and with **shot.** on (the default) a screenshot of the window, taken
   with the panel hidden.
3. **fix.** sends the picked open notes, or every open note when none is picked: one request file in
   the fix inbox. The notes turn *sent.*; the panel says "sent — Claude will pick it up".
4. The Claude session fixes them and marks them *fixed.* over the control channel.

Notes: click picks one, ⌘-click adds / removes one, ⇧-click picks the run from the last click,
**all.** / **none.**; hover a note for **ed.** (edit; Enter saves, Esc cancels) and **✕** (removes
it, or every picked note when it is picked).

## Files

All under LightCraft's settings folder (`~/Library/Application Support/LightCraft/` on macOS,
`%APPDATA%\LightCraft` on Windows, `$XDG_CONFIG_HOME/lightcraft` on Linux) — never in the
repository, never in the photo library.

| Path | What |
|---|---|
| `fix-notes.json` | every note (`{version, nextId, notes: [Note]}`), kept across restarts. A damaged file is renamed `fix-notes.json.damaged-<unix time>` and the notes start empty. |
| `fix-notes-shots/<YYYYMMDD-HHMMSS>-note<id>.png` | the notes' screenshots |
| `fix-inbox/<YYYYMMDD-HHMMSS>-fix.json` | one request per **fix.** (UTC time; a second request in the same second is `…-2-fix.json`). `LIGHTCRAFT_FIX_INBOX=<folder>` puts them elsewhere. |

A request is written as `<name>.json.tmp` and renamed when complete: **watchers must ignore
`*.tmp`** and can read any `*-fix.json` they see in full. LightCraft never deletes or rewrites a
request; the Claude side may move a request it has handled (e.g. into `fix-inbox/done/`).

With `--memory` / `--demo` or `LIGHTCRAFT_NO_PREFS` notes live in memory only and there is no
inbox unless `LIGHTCRAFT_FIX_INBOX` names one. If the inbox can't be written (disk full, no
permission) **fix.** shows the error and the notes stay open.

## Request file (`version: 1`)

```json
{
  "kind": "lightcraft.fixRequest",
  "version": 1,
  "created": "2026-11-22T21:05:09Z",
  "at": 1795381509,
  "app": {
    "version": "0.4.0",
    "commit": "929e54396abc",
    "os": "macos",
    "pid": 4242,
    "logFile": "/Users/…/Library/Application Support/LightCraft/logs/lightcraft.log"
  },
  "library": "/Users/…/Pictures/LightCraft Library",
  "notesFile": "/Users/…/Library/Application Support/LightCraft/fix-notes.json",
  "notes": [ Note, … ],
  "reply": {"command": "notes.markFixed", "params": {"ids": [3, 4]}}
}
```

`app.commit` is the git commit the binary was built from (`unknown` outside a checkout); `logFile`
is `null` when the app logs to standard error only. `reply` is the call that closes the request.

### Note

```json
{
  "id": 3,
  "text": "the exposure slider jumps when I drag it fast",
  "at": 1795381400,
  "time": "2026-11-22T21:03:20Z",
  "status": "sent",
  "context": {
    "module": "develop",
    "view": "detail",
    "panel": "edit",
    "dialog": null,
    "photo": {
      "id": 1201, "fileName": "IMG_0420.CR3", "path": "/Volumes/Photos/2026/IMG_0420.CR3",
      "format": "CR3", "camera": "Canon EOS R5", "lens": "RF24-70mm F2.8 L IS USM",
      "size": [8192, 5464], "captured": "2026-11-20T16:02:11", "edited": true, "previewOnly": null
    },
    "selected": 1,
    "source": "folder:/Volumes/Photos/2026",
    "library": "/Users/…/Pictures/LightCraft Library",
    "status": null,
    "notices": [],
    "recent": [{"time": "2026-11-22T21:03:02Z", "text": "WARN lightcraft_pipeline: …"}],
    "window": [1120.0, 780.0],
    "screenshotError": "only when the screenshot failed"
  },
  "screenshot": "/Users/…/LightCraft/fix-notes-shots/20261122-210320-note3.png",
  "request": "/Users/…/LightCraft/fix-inbox/20261122-210509-fix.json",
  "sentAt": 1795381509,
  "fixedAt": null,
  "fixNote": null
}
```

- `status`: `open` → `sent` (by **fix.** / `notes.send`) → `fixed` (by `notes.markFixed`).
- `context.module`: `library` / `develop`; `view`: `photoGrid`, `squareGrid`, `detail`, `compare`,
  `survey`, `reference`, `people`, `map`; `panel`: the right-hand panel or tool (`edit`, `crop`,
  `remove`, `masking`, `redEye`, `profiles`, `info`, `keywords`, `versions`, `activity`, `none`;
  `null` in Library when a Develop tool is merely left selected); `dialog`: the open dialog's kind.
- `context.photo`: the active photo (`null` when none); `path` is `demo:<scene>` for demo photos.
- `context.recent`: the last 8 warnings and errors — the log's (`LEVEL target: message`) and the
  errors LightCraft showed or a command returned — oldest first. `status` is the last command
  error, `notices` the warnings waiting to be shown.
- `screenshot`: `null` when shot. was off, the screenshot failed (`context.screenshotError`) or it
  hadn't finished when the note was sent (it is in `fix-notes.json` / `notes.list` afterwards).

## Commands (control channel, MCP, menus)

All are UI commands: send them with `engine.execute {command, params}` to the control port
(`--control 7980`, `docs/control-protocol.md`) or through `lightcraft-cli mcp --connect`.

| Command | Params | Result |
|---|---|---|
| `notes.toggle` | `{open?: bool}` | `{open}` — shows / hides the panel (⌘⌥⇧N) |
| `notes.add` | `{text, shot?: bool}` | `{id, note, open, screenshot?}` — `shot` defaults to the panel's shot. toggle |
| `notes.list` | `{status?: "open"\|"sent"\|"fixed"\|"all"}` | `{notes, open, sent, fixed, picked, panel, file, inbox}` |
| `notes.send` | `{ids?: [id]}` | `{path, ids, count}` — given ids (open or sent), else the picked open notes, else every open one |
| `notes.markFixed` | `{ids: [id], note?: "what was done"}` | `{ids, open}` |
| `notes.reopen` | `{ids: [id]}` | `{ids, open}` — back to open |
| `notes.edit` | `{id, text}` | `{id}` |
| `notes.remove` | `{ids: [id]}` | `{removed}` |
| `notes.select` | `{ids: [id]}` or `{all: true}` | `{picked}` — `{ids: []}` picks none |

Any of them may also return `saveError` when `fix-notes.json` couldn't be written (the change is
kept in memory and the panel shows the error).

### The Claude side, in short

1. Watch `~/Library/Application Support/LightCraft/fix-inbox/` for new `*-fix.json` (ignore `*.tmp`).
2. Read the request: each note's `text`, `context` and `screenshot` (a PNG to look at).
3. Fix, test, commit (per `CLAUDE.md`).
4. `engine.execute {"command": "notes.markFixed", "params": {"ids": [...], "note": "<commit> what changed"}}`
   on port 7980 when the app is running; otherwise it can be done the next time it is.
