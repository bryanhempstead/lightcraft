# Progress — Bryan's LightCraft (his copy of storytold/lightcraft, ~/crafts/lightcraft)

Bryan wants LightCraft to replace Lightroom Classic for him: same shortcuts, presets, catalog data, devices.
Upstream rules are in AGENTS.md (never crash, pure Rust, everything is a command). This board is for his changes.
Commit locally on `main` (no push until he OKs a fork). Pull upstream with `git pull` and merge.

## Open
- [ ] (decoding agent, 2026-10-08) **Bryan runs** (app closed, after `cargo build --release -p lightcraft-cli`): `target/release/lightcraft-cli migrate-lightroom --only-new --no-presets --library "$HOME/Pictures/LightCraft Library" --catalog "$HOME/Pictures/Lightroom/LR-Cat2.lrcat-v13-3.lrcat"` — brings in the 35 HEIC the migration failed on (tested on copies: 34 imported + 1 byte-identical duplicate). Then rebuild/restart LightCraft so the app decodes HEIC too.
- [ ] (classic-ui agent, 2026-10-08) Classic layout: Library | Develop modules (G/E/D + picker), filmstrip both modules, Develop right/left panels in Classic order, Library right panel (Quick Develop, Keywording, Metadata), Classic import window. OWNS: lib.rs layout, topbar/strip/right/edit/presets/bottombar/import/dialogs(import). NOT left.rs (left-panel agent) — the "Import…" button at the bottom of the Library left panel: left-panel agent please leave room / I'll add a `panels::left::import_button` hook call only after asking here.
- [x] (left-panel agent, 2026-10-08) Library LEFT panel = Classic: Navigator, Catalog, Folders (disks, root folders, subfolders toggle, sync/find missing/create/add parent), Collections (sets/smart), then **Import… / Export… buttons at the bottom — I add them in left.rs**: Import… runs the menu command `file.addPhotos` (classic-ui agent: point `file.addPhotos` at your Classic import window, or tell me your new id here and I switch), Export… runs `dialog.export`. Local / By Date / Keywords leave the left panel (Local only while a Local folder is browsed; Keywords → Library right panel's Keyword List — `panels::left::keywords_section` stays callable). OWNS: panels/left.rs, cmd/folders.rs, lr_migrate collections. Engine done: 773c63d, feb1102, 8a790b1. Menu ask (classic-ui agent, you own menus): Classic has a top-level **Library** menu; `library.showSubfolders` (checked state wired in menubar::checked) sits under View until you add Library to `menubar::MENUS` — then change its menu path in cmd/folders.rs (or tell me).
- [x] (lr agent, 2026-10-08) Lightroom Classic migration: catalog + every preset folder — f6b0429, 2756349, c9f5328 (see Done)
- [x] (keys agent, 2026-10-08) Shortcuts: user keymap + "Lightroom Classic" key profile + shrt. settings page (LrKeys / LrSuperKeys built in) — 2fab9d6
- [x] (keys agent) Devices: MIDI in + mouse buttons; Monogram profile generated from his LR one — 2fab9d6, b307350
- [x] (brain app) macOS .app bundle: ~/second-brain/src/crafts.js ensureApp builds target/release/LightCraft.app (bundle id ai.storyteller.lightcraft)
- [x] (keys agent) "left off." resume point: button + view.resumeLastLeftOff + per-folder/album landing + grid/filmstrip marker — 2fab9d6
- [ ] Try the Monogram profile on the real console (import tools/monogram/LightCraft.monogram in Monogram Creator, tick ctrl. ▸ MIDI in): check the relative-dial direction/speed and that Creator accepts MIDI on pressAndTurn / doubleTap / pressAndHold

## Notes for the next agent
- (decoding agent, 2026-10-08) **Formats**: CR3 now decodes natively (cherry-picked upstream #279; it rode into 8a790b1 because a parallel commit swept the shared index — commit with `git commit -- <paths>`). On macOS everything else goes through `crates/engine/src/files/sysdecode.rs` (sips → cached TIFF in `<library>/System Decodes/`, photos marked preview_only "decoded by macOS …", UI notice in widgets.rs). Import failures persist in `<library>/import-failures.json`: `library.importFailures` / `library.retryFailedImports`. Docs: docs/macos-decode.md. UI agents: the Import window's file-type list could use `lightcraft_engine::import::extensions()`; a "Retry failed imports" menu item could call `library.retryFailedImports`. Upstream main still has 60+ unmerged commits (incl. #391/#393 wider preview fallback, CRW/MRW/X3F import) — a full merge conflicts in ui-egui (menus/shortcuts/lib.rs), do it when the UI agents are done.
- (classic-ui agent, 2026-10-08 18:xx) **ui-egui compiles again** (`cargo build -p lightcraft-ui-egui` clean, no warnings). New files: panels/filmstrip.rs, develop_left.rs, library_right.rs. Layout in lib.rs ui(): topbar(module picker) → filmstrip (full width) → right panel (Develop: right::show / Library: library_right) → left panel (Develop: develop_left / Library: left::show — yours) → toolbar. `file.addPhotos` (no paths) will open the Classic Import window (in progress; until then the file picker). Running ui-egui tests now; fixing what the layout change breaks.
- Inventory of his Lightroom data + device bindings: second-brain session 2026-10-08 (see the brief in each task).
- Brain app side: ~/second-brain/src/crafts.js (launch, build, LC. → control port 7980), renderer/crafts.js.
- **Resume point API (lr agent → keys agent, 2026-10-08, in f6b0429):** engine query command
  `library.resumePoint {folder?: "/abs/path", subfolders?: bool = true, album?: albumId}` (neither = whole library)
  → `{photoId, at: "YYYY-MM-DDTHH:MM:SS" (UTC), source: "edit" | "lightroomEdit" | "lightroomTouch"}` or `null`.
  It picks the photo with the latest of: `Photo::edited` (LightCraft edits), Lightroom's last develop-history time and
  Lightroom's touchTime (both kept by the migration in `<library>/lightroom-migration.json`, `{touched: {photoId: iso},
  edited: {photoId: iso}}`). If the UI records its own "last viewed" per folder/album, prefer that and call this as the
  fallback. Rust: `lightcraft_engine::lr_migrate::resume_point(&session, folder, album, subfolders)`.

## Done
- 2026-10-08 (left-panel agent) **Library left panel = Classic + migration collections** — 773c63d, feb1102, 8a790b1,
  85167f1, 3c4d23d. Navigator / Catalog / Folders (disk rows with space + online, root folders, Show Photos in
  Subfolders, Synchronize, Find Missing Folder, Create Folder Inside, Add Parent / Promote) / Collections (sets,
  smart) / Import… Export…. Engine: cmd/folders.rs, engine::disks, catalog::folders::root_folders. Migration copies
  the catalog's collection tree exactly (no wrapper), Quick Collection → ours, sync-duplicate members → masters;
  `library.flattenLightroomCollections` + `migrate-lightroom --collections-only` fix a library made earlier.
  Screenshots: scratchpad leftpanel/left1.png, left2-subfolders-on.png, left3-subfolders-off.png.
  Next: collection-set rows have no disclosure triangle yet (click folds); drag photos between folders not done.
- 2026-10-08 (decoding agent) **Any photo format** — CR3 native (upstream #279; 50/50 of his R6 files decode), macOS ImageIO fallback for HEIC/HEIF/AVIF/JP2/EXR/TGA/… + raws we can't decode (0322ab3), "Decoded by macOS" notice + README/parity (dc4af71), retry failed imports + `migrate-lightroom --only-new` (0322ab3).
- 2026-10-08 (keys agent) **Keys, MIDI, mouse, left off** — `crates/ui-egui/src/keymap.rs` (keymap.json at
  `~/Library/Application Support/LightCraft/keymap.json`, env LIGHTCRAFT_KEYMAP; profile lightroom|classic + user
  bindings, live reload, damaged file = defaults + notice, never overwritten), Settings ▸ shrt. / ctrl.,
  super-key commands (`keys.*`, `preset.applyByName`, `crop.nudge`, `view.develop`, `view.colorMixer`), MIDI via
  `apps/lightcraft/src/midi_in.rs` (midir/CoreMIDI, only while ctrl. ▸ MIDI in is on), `leftoff.rs`
  (`ui.json` → leftOff; lands on open of a folder/album via library.source/browse; falls back to library.resumePoint).
  His keymap.json was generated: profile classic + LrSuperKeys (H/J/K/B colour-mixer keys, 107 slider steps) +
  LrKeys (⌥\ ⌥4 ⌥Q ⌥O ⌥X ⌥W ⌥R ⌥U) + 28 Monogram MIDI mappings, MIDI in on. Monogram profile:
  tools/monogram/LightCraft.monogram + README. Tests: keymap unit tests, crates/ui-egui/src/tests_keys.rs (headless).
  Merged the fork's 158 upstream commits (a242327). Heads-up: LrSuperKeys H/J/K/B override Classic's H (pins),
  J (clipping), K (brush), B (quick collection) as they did in his Lightroom; shrt. ▸ del. brings the built-ins back.
- 2026-10-08 (lr agent) **Migrate from Lightroom Classic** — `library.migrateLightroom` (File menu, background:
  worker reads catalog + maps develop, background import, then apply), `lightcraft-cli migrate-lightroom`,
  `library.resumePoint`. Code: crates/engine/src/lr_migrate.rs, crs_spots.rs, cmd/lightroom.rs,
  crates/ui-egui/src/lr_migrate.rs (+ ImportTask then-hook in import.rs). Docs: docs/xmp-interop.md → "Lightroom
  Classic catalogs". Tested on a scratch library with a 217-photo subset of LR-Cat2 (+125 virtual copies): ratings,
  flags, keywords, exposure/contrast, crops (stored→shown frame), masks, spots, point colour all round-trip.
  NOT run on his real library yet. Full run (Bryan, app closed or from the File menu):
  `lightcraft-cli migrate-lightroom --library "$HOME/Pictures/LightCraft Library" --catalog "$HOME/Pictures/Lightroom/LR-Cat2.lrcat-v13-3.lrcat"`
  Open gap: creative profiles "Summer Fields" (4,220 photos) / "Nautica" (870) are Archipelago RGBTable looks; they
  match automatically once a `.cube` of the same name is imported (`profile.import`), or `lookMap`. DCPs: unsupported.
- 2026-10-08 (lr agent) **Shoot commands for the brain pipeline** (engine, control channel + MCP):
  `library.folderStatus {folder, subfolders?, scanDisk?, ids?}`, `library.importRated {files: [{path, rating, flag,
  label, keywords}], mode?, albumName?…}`, `library.showFolder {folder, subfolders?, select?}`,
  `app.export {folder, subfolders?, preset, dir, background: true}` (poll `ui.inspect` → `export`),
  `library.exportCatalog {folder | ids, dest}`.
