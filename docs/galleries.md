# Client galleries (the `galleries.` tab)

LC-GALLERIES. The client galleries Bryan runs from Second Brain's Client tab (Pic-Time-style, self-hosted:
token links with an optional password, downloads, slideshows, print orders, client hearts) are also a module tab in
LightCraft, beside `library.`, `develop.` and `map.` (⌥⌘4, Window ▸ Galleries). Second Brain stays the one place
they live; LightCraft is a second door into them, plus what only a photo editor can add.

## One source of truth

- **Second Brain owns the galleries**: `<userData>/client/galleries.json` (userData =
  `~/Library/Application Support/second-brain`), the gallery server, LOT48 publishing, the client book.
- It serves them to LightCraft through a **local JSON API** (`src/galleryapi.js` in second-brain), started with the
  app. Every route calls the same `src/client.js` / `src/clientbook.js` functions the Client tab's IPC handlers call
  — no second copy of the logic.
- **LightCraft never writes `galleries.json`.** When Second Brain isn't running, the tab reads it to show the
  galleries read-only, with a notice and `start.` (opens `/Applications/Second Brain.app`); every change is refused
  with "Second Brain isn't running". A missing, unreadable or broken file is an empty list with a notice.

## UX

- **Cards**: cover, name, client, photo count, `live.` (published to LOT48) or `local.`, views, ♥ client picks,
  orders, `pw` when it has a password, `lc.` when it was made in LightCraft. A card opens the gallery.
- **A gallery**: its photos with badges — ♥ the client's hearts, ★ his highlights, `hidden.` photos taken out of the
  client's view, `lc.` exported from this library. Pick photos with click / ⌘-click / ⇧-click, `all.` / `none.`,
  ⌘A and Esc (Esc again closes the gallery). With photos picked: `hide.` / `show.` (out of / back into the client's
  gallery; files untouched), `dev.` (double-click also) opens the source photo in Develop.
- **Right panel**: settings (title, client, email, password, expiry `YYYY-MM-DD`, downloads, print store) with
  `save.` / `undo.`; the link (`copy.`, `open.`, `publish.` / `unpublish.`); **client picks** (`flag.` = Pick flag, `star.` =
  4 stars, `show.` = the "client picks" collection in the Library); `re.export.` / `all.` for galleries made here;
  `slide.` makes a slideshow film of the picked photos (or all) in Second Brain.
- **Left panel**: **new gallery from** the selection (`selected.`) or what the grid shows (`view.`: a collection, a shoot,
  one camera-folder part of a collection, a filter), with a name, client and watermark (text or a watermark
  preset's name); the **client book** (a click fills the new gallery's client).

## What LightCraft adds

- **Galleries with the edits.** `gallery.create` exports the photos through `lightcraft_engine::export` (the same
  encoder as File ▸ Export): JPEG quality 85, long edge 2048 px (never enlarged), sRGB, screen sharpening, location
  removed, copyright + contact metadata, optional watermark; `export` overrides any `app.export` key. The folder is
  `<shoot>/gallery/<name>/`, the shoot being the folder the photos share (`…/raw/<camera>` and `…/raw` count as
  their shoot); photos with no common shoot go to `~/Pictures/LightCraft Galleries/<name>`. Then the gallery is
  registered with Second Brain (`POST /v1/galleries`) and opens.
- **Re-export in place.** The catalog keeps which photo each file is (`gallery.link.<id>` library record: file →
  photo id, the export settings, each photo's fingerprint). `gallery.reexport` writes the photos edited since (or
  all, `all: true`, or more photos, `ids`) over their own files, and tells Second Brain the folder changed.
- **Client picks back in the library.** `gallery.picks` pulls the hearts from the copy LOT48 serves and maps them to
  library photos; `gallery.applyPicks {flag?, rating?, label?}` marks them and keeps a "<name> · client picks"
  collection (in a "Client Picks" collection set) in step with them, one undo step each.
- **Gallery photo → Develop.** `gallery.sourcePhoto` selects the source photo (showing the whole library if it is
  filtered out); `galleries.develop` then opens Develop.

## Commands

Engine (`crates/engine/src/cmd/galleries.rs`; MCP / control channel / CLI): `gallery.status`, `gallery.list`,
`gallery.get`, `gallery.create`, `gallery.reexport`, `gallery.update`, `gallery.delete`, `gallery.setFolder`,
`gallery.publish`, `gallery.unpublish`, `gallery.link`, `gallery.hide`, `gallery.picks`, `gallery.applyPicks`,
`gallery.sourcePhoto`, `gallery.stats`, `gallery.orders`, `gallery.orderStatus`, `gallery.slideshow`,
`gallery.slideshowStatus`, `gallery.proofs`, `gallery.clientBook`, `gallery.saveClient`, `gallery.saveShoot`,
`gallery.mailDraft`. `command.list` has each one's parameters.

UI (`crates/ui-egui/src/panels/galleries.rs`): `view.galleries {id?}`, `galleries.refresh`, `galleries.open {id?}`,
`galleries.newFrom` (background export), `galleries.reexport`, `galleries.publish {on}`, `galleries.pick {names |
all | none}`, `galleries.save`, `galleries.develop`, `galleries.showPicks`, `galleries.applyPicks`,
`galleries.copyLink`, `galleries.openLink`, `galleries.hide`, `galleries.slideshow`, `galleries.start`.

Exports, publishing and every read of the list run on worker threads; the UI thread never waits for Second Brain or
LOT48.

## The API contract (second-brain `src/galleryapi.js`)

- **Where**: `127.0.0.1` only (default port 8796, `SB_GALLERY_API_PORT`; a taken port falls back to a free one).
- **Discovery**: `<userData>/gallery-api.json`, mode 0600: `{"api": 1, "port": …, "token": "<64 hex>", "pid": …,
  "started": ms}`. The token is made once (`gallery-api.token`, 0600) and kept across restarts. LightCraft reads
  the file from `LIGHTCRAFT_SECOND_BRAIN_DIR` or `~/Library/Application Support/second-brain`.
- **Auth**: `Authorization: Bearer <token>` on every request (401 otherwise). A `Host` other than
  `127.0.0.1` / `localhost` is refused (DNS rebinding), and so is any request with an `Origin` header (web pages).
- **Format**: JSON in and out (body ≤ 1 MB), `Content-Length` set. Errors: `{"ok": false, "error": "…"}` with 400
  (bad input), 404 (no such gallery / route), 405, 422 (the action failed), 500. Records never carry the link key
  (`token`); the password shows only as `hasPassword`, except in `GET /v1/galleries/:id` (so it can be edited).

| Method | Path | Body / query | Answer |
|---|---|---|---|
| GET | `/v1/ping` | | `{ok, app: "second-brain", api: 1}` |
| POST | `/v1/pipeline/inbox` | | `{ok, taken: [{file, ok, id, name, stage, backup} | {file, ok: false, error}]}` — take LightCraft's queued pipeline requests now (`pipeline.add`, `crates/engine/src/sb_pipeline.rs`) |
| GET | `/v1/galleries` | | `{galleries: [record + published, cardCover, views, viewers, downloads, orders, picks, hasPassword]}` |
| GET | `/v1/galleries/:id` | | `{gallery (+ password), photos: [{name, size, web}], videos, files}` |
| POST | `/v1/galleries` | `{folder, name?, clientName?, clientEmail?, password?, allowDownload?, expires?, allowPrints?, layout?, lightcraft?}` | `{gallery, published}` |
| PATCH | `/v1/galleries/:id` | any of `name, clientName, clientEmail, password, expires, allowDownload, allowPrints, layout, cover, highlights, lightcraft` | `{gallery}` (another key: 422) |
| DELETE | `/v1/galleries/:id` | | `{ok}` — the record only; folder and photos untouched |
| POST | `/v1/galleries/:id/folder` | `{folder}` | `{gallery}` (re-counted) |
| POST | `/v1/galleries/:id/publish` · `/unpublish` | | as the Client tab's publish. / unpublish. |
| GET | `/v1/galleries/:id/link` | | `{url, public, permanent, links: [per recipient]}` |
| POST | `/v1/galleries/:id/hidden` | `{names, hide}` | `{hidden, cover, synced}` |
| GET | `/v1/galleries/:id/picks` | | `{selected, highlights, hidden, arrange, comments: [{who, note, at, photos}]}` |
| POST | `/v1/galleries/:id/picks/pull` | | `{pulled, selected}` (hearts from the copy LOT48 serves) |
| GET | `/v1/galleries/:id/stats` · `/orders` | | `statsFor` · `{orders}` |
| POST | `/v1/orders/:orderId/status` | `{status}` | |
| GET | `/v1/galleries/:id/thumb` | `?name=&size=64..2048` | `image/jpeg` (the gallery page's cached thumbnail) |
| GET | `/v1/proofs` | | `{proofs}` |
| POST | `/v1/galleries/:id/slideshow` | `{photos?, vibe?, song?, title?: {text, subtitle}}` | `{renderId}` |
| GET | `/v1/slideshows/:renderId` · POST `…/cancel` | | `{state: running \| done \| failed \| cancelled, path, error}` |
| GET | `/v1/clientbook` | | the client book (`clients`, shoots per client, …) |
| POST | `/v1/clientbook/clients` · `/shoots` | `{client}` · `{shoot}` | `{client}` · `{shoot}` |
| POST | `/v1/galleries/:id/mail-draft` | `{recipient?, all?}` | `{to, subject, body, url}` — a draft; nothing is sent |

Tests: second-brain `test/galleryapi.test.js` (temp dirs only); LightCraft `crates/engine/src/tests_galleries.rs`
and `crates/ui-egui/src/tests_galleries.rs` against `lightcraft_engine::galleries::fake::FakeSecondBrain`, a stand-in
API on a free port (never the real app or its files).

## Not yet

Second Brain's slideshow editor (music, titles, storyboard) and proofs editor stay there (`slide.` here makes a
plain film); print orders are listed (`gallery.orders`) but not handled in the tab; email is a draft
(`gallery.mailDraft`), never sent from LightCraft; clients have no comment box (order notes come back as
`comments`). Thumbnails of galleries made elsewhere come from Second Brain's cache (or the file when it is down).
