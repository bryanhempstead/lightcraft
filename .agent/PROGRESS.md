# Progress — Bryan's LightCraft (his copy of storytold/lightcraft, ~/crafts/lightcraft)

Bryan wants LightCraft to replace Lightroom Classic for him: same shortcuts, presets, catalog data, devices.
Upstream rules are in AGENTS.md (never crash, pure Rust, everything is a command). This board is for his changes.
Commit locally on `main` (no push until he OKs a fork). Pull upstream with `git pull` and merge.

## Open
- [ ] (colour agent round 2, 2026-10-09) **Bryan runs** — replaces step 1 of the round-1 item below (steps 2–3 there stay done). App closed, after `cargo build --release -p lightcraft-cli -p lightcraft`. Reads copies of the catalog/previews, writes only `~/Library/Application Support/LightCraft/camera-profiles/` and scratch folders in /tmp; never the library. Order matters (calibrate rewrites the profiles; wbmap / lensfit add to them):
  1. `python3 tools/lr-compare/lr_compare.py calibrate --work /tmp/lc-cal2 --same-sensor X100F=X-T2` → Leica + R6 from real Lightroom renders, GR III / Q2 / iPhone ProRAW with the Leica look on their own matrices, X100F = X-T2 (~3 min)
  2. white balance: `printf '[{"model":"LEICA M (Typ 262)","ext":"dng","look":"Summer Fields","n":90},{"model":"Canon EOS R6","ext":"cr3","look":"Summer Fields","n":70},{"model":"RICOH GR III","ext":"dng","look":"Summer Fields","n":60},{"model":"X-T2","ext":"raf","look":"Summer Fields","n":25},{"model":"X100F","ext":"raf","look":"Summer Fields","n":45},{"model":"iPhone 13 Pro","ext":"dng","look":"Summer Fields","n":12}]' > /tmp/lc-wb.json && python3 tools/lr-compare/lr_compare.py prepare --work /tmp/lc-wb --spec /tmp/lc-wb.json && python3 tools/lr-compare/lr_compare.py migrate --work /tmp/lc-wb --tag final --profiles "$HOME/Library/Application Support/Adobe/CameraRaw/ImportedSettings" && python3 tools/lr-compare/lr_compare.py wbmap --work /tmp/lc-wb` (~10 min)
  3. lens vignetting: `printf '[{"model":"Canon EOS R6","ext":"cr3","look":"Adobe Color","n":160}]' > /tmp/lc-lens.json && python3 tools/lr-compare/lr_compare.py prepare --work /tmp/lc-lens --spec /tmp/lc-lens.json && python3 tools/lr-compare/lr_compare.py migrate --work /tmp/lc-lens --tag final && python3 tools/lr-compare/lr_compare.py lensfit --work /tmp/lc-lens` (~5 min)
  4. restart LightCraft (renders pick up the new code + profiles; no library repair needed this round). Then `rm -rf /tmp/lc-cal2 /tmp/lc-wb /tmp/lc-lens`.
- [ ] (colour agent, 2026-10-09) **Bryan runs** (after `cargo build --release -p lightcraft-cli -p lightcraft` and with LightCraft closed — it reads the Lightroom catalog/previews from copies, writes only LightCraft's own config folder and library):
  1. `python3 tools/lr-compare/lr_compare.py calibrate --work /tmp/lc-calibrate` → Lightroom-matched camera profiles in `~/Library/Application Support/LightCraft/camera-profiles/` (R6, M262, GR III, X-T2; ~1 min)
  2. `target/release/lightcraft-cli run --library "$HOME/Pictures/LightCraft Library" library.rematchProfiles readCatalog=true dryRun=true` then without `dryRun=true` → Summer Fields / Nautica / other XMP looks imported and set on the photos that use them
  3. `target/release/lightcraft-cli run --library "$HOME/Pictures/LightCraft Library" library.repairLightroomMigration dryRun=true` then without → straighten angles (sign bug) + photos that took a stale XMP sidecar
- [ ] (decoding agent, 2026-10-08) **Bryan runs** (app closed, after `cargo build --release -p lightcraft-cli`): `target/release/lightcraft-cli migrate-lightroom --only-new --no-presets --library "$HOME/Pictures/LightCraft Library" --catalog "$HOME/Pictures/Lightroom/LR-Cat2.lrcat-v13-3.lrcat"` — brings in the 35 HEIC the migration failed on (tested on copies: 34 imported + 1 byte-identical duplicate). Then rebuild/restart LightCraft so the app decodes HEIC too.
- [x] (classic-ui agent, 2026-10-08) Classic layout: Library | Develop modules, filmstrip, Develop / Library panels, Import window — see Done
- [x] (left-panel agent, 2026-10-08) Library LEFT panel = Classic: Navigator, Catalog, Folders (disks, root folders, subfolders toggle, sync/find missing/create/add parent), Collections (sets/smart), then **Import… / Export… buttons at the bottom — I add them in left.rs**: Import… runs the menu command `file.addPhotos` (classic-ui agent: point `file.addPhotos` at your Classic import window, or tell me your new id here and I switch), Export… runs `dialog.export`. Local / By Date / Keywords leave the left panel (Local only while a Local folder is browsed; Keywords → Library right panel's Keyword List — `panels::left::keywords_section` stays callable). OWNS: panels/left.rs, cmd/folders.rs, lr_migrate collections. Engine done: 773c63d, feb1102, 8a790b1. Menu ask (classic-ui agent, you own menus): Classic has a top-level **Library** menu; `library.showSubfolders` (checked state wired in menubar::checked) sits under View until you add Library to `menubar::MENUS` — then change its menu path in cmd/folders.rs (or tell me).
- [x] (lr agent, 2026-10-08) Lightroom Classic migration: catalog + every preset folder — f6b0429, 2756349, c9f5328 (see Done)
- [x] (keys agent, 2026-10-08) Shortcuts: user keymap + "Lightroom Classic" key profile + shrt. settings page (LrKeys / LrSuperKeys built in) — 2fab9d6
- [x] (keys agent) Devices: MIDI in + mouse buttons; Monogram profile generated from his LR one — 2fab9d6, b307350
- [x] (brain app) macOS .app bundle: ~/second-brain/src/crafts.js ensureApp builds target/release/LightCraft.app (bundle id ai.storyteller.lightcraft)
- [x] (keys agent) "left off." resume point: button + view.resumeLastLeftOff + per-folder/album landing + grid/filmstrip marker — 2fab9d6
- [ ] Try the Monogram profile on the real console (import tools/monogram/LightCraft.monogram in Monogram Creator, tick ctrl. ▸ MIDI in): check the relative-dial direction/speed and that Creator accepts MIDI on pressAndTurn / doubleTap / pressAndHold

## Notes for the next agent
- (classic-ui agent, 2026-10-08 later) For the left-panel agent / coordinator: (1) **Library menu** added (menubar.rs MENUS = File, Edit, Library, View, Photo, Window, Help; `menus::LIBRARY_MENU` moves New Collection/Set/Smart, Filter Bar, Clear Filters, Previous Import (`view.previousImport`), **Show Photos in Subfolders**, Find Missing Photos, Sync Metadata there without changing the engine paths). (2) `file.addPhotos` with no `paths` now opens the Classic **Import window** (crates/ui-egui/src/import_window.rs) — your Import… button works as is. `file.addFolder` / `file.addFromDevice` / drops keep the review dialog. (3) Fixed a render loop: `render::thumb_current` now lets a larger cached thumbnail serve a smaller ask — before, the filmstrip (256) and a navigator/grid asking another size re-rendered the same photo every frame forever (settle never finished; CPU burn). Your Library Navigator's `thumb_px` ask is fine now. (4) `UiState.left_panel` defaults to **true** now (Classic), and `view.leftPanel {show}` / `view.filmstrip {show}` take an explicit value. Tests still failing that look like yours: i18n `every_language…` / `german_panels…` (expect "My Photos"), headless `local_location_can_be_hidden_and_restored`, `keyword_list_filters_renames_and_suggests` (source:keyword:*), tests_masking `local_folder_tree_expands_and_browses`, tests_panels `a_deep_chain…`, `date_and_keyword_rows…`, `right_click_opens_a_folders_menu…`, `the_selection_bar_stays_inside_the_panel`.
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
- 2026-10-09 (colour agent round 2) **Lightroom match round 2 + detail** — 8598707, 1363918, 7f99374, 3c65e23, 5adf51e, 890224c, cb85294, 17c1866, ed2411f, 8234c5e, b32229f, 173a417. Round 1's ground truth was partly the camera's embedded JPEG (now filtered); Leica/R6 recalibrated on real renders, look transfer for GR III/ProRAW, X100F = X-T2; image-adaptive Highlights/Shadows + refitted tone tables; per-camera WB map; LR-like sharpening / grain / colour NR, halo-free local tone (regression test); crop-aware preview level; R6 lens vignetting; B&W mix; JPEGs through a reference curve. Held-out split B mean dE 5.88 → 4.46, p95 13.5 → 10.7, passing 2 → 21 of 103 (defaults 19/40, edited 2/63). Details, tables and what still misses: docs/lr-match.md → Round 2. New tool subcommands: `lr_compare.py detail | wbmap | lensfit`, `calibrate --same-sensor`. Scratch (work sets, dumps, crops): the colour agent's scratchpad `lc/` + `lc/r2/`.
  Next: edited photos still ~6 dE (per-photo exposure adaptivity beyond the image key, masks, mid-scale local contrast ~20 % low, CR3/RAF distortion), Nautica on iPhone JPEGs, X-T2 profile (no renders, no matrices), Q2 / 5D Mark II unmeasured. Test failing on main not from this work: `apps/lightcraft native_menu::shortcuts_map_to_accelerators`.
- 2026-10-09 (shoots agent) **Library ▸ shoots. + tidier Folders** — 522f550, 8924e48, 5f77510, d96f0e0, a575f8c + docs.
  Bryan: "keep my photo work centrally located … only the areas I import in the drives … not so many drop-downs".
  shoots. (under Catalog): one row per shoot on any disk (camera/format folders raw, M262, X-T2 raw, GR III,
  Canon R6, 100MSDCF… climb to the shoot; containers like Pictures / "- High Priority Edit" split) — on his real
  folder list it gives 15 rows (Erika and Connor wedding, *C&J WEDDING, Iceland - Our Wedding…, chicago, SYNC…).
  Engine: `library.shoots`, `shoot.show/pin/hide/rename/sort` (prefs.json → shoots), `library.source
  {kind: libraryFolder, subfolders}`. Folders: only imported roots, camera roots named "Shoot › M262", one-way
  chains as one row, open state kept in ui.json (foldersOpen), import. (Import window on that folder, Add mode)
  on shoots/folders/disks; Browse Folder Without Importing… moved into the Import window. Tests:
  crates/catalog/src/tests_shoots.rs, engine cmd/shoots.rs, crates/ui-egui/src/tests_shoots.rs
  (LIGHTCRAFT_TEST_SHOTS=dir writes screenshots). Heads-up: the colour agent's 5adf51e swept my half-done
  catalog lib.rs/folders.rs edits (commit with `git commit -- <paths>`); fixed by 522f550.
  Next: on his real library check the startup-disk rows ("bryanhempstead" = 10 loose photos in ~) and whether any
  shoot splits/merges wrong (rename./remove. cover it; no regrouping yet); drag photos between shoots not done.
- 2026-10-09 (colour agent) **Lightroom colour fidelity** — 07da802, 17cc043, 2445b4c + this round. Creative XMP
  profiles (`crs:RGBTable` decoded: base-85 Z85 variant + zlib + n³ u16 deltas; applied after the tone curves),
  imported by `profile.import` / migration, `library.rematchProfiles`; `calibrate --lightroom` (Lightroom-matched
  camera profiles + white-balance model from the user's catalog); Lightroom-matched Basic tone (`tone::lr`, CPU+GPU);
  RAF exposure bias; crop-angle sign; sidecar settings; `library.repairLightroomMigration`; `tools/lr-compare`.
  Measured on 167 of his photos vs Lightroom's previews (mean CIEDE2000): Summer Fields 14.3 → 8.0, Nautica 9.7 →
  9.0, Adobe Color 8.1 → 5.0, default settings 7.4 → 2.6; R6 9.5 → 6.0, M262 12.2 → 7.7, GR III 11.6 → 6.7, X-T2
  10.1 → 5.4, X100F 19.7 → 9.5. L1007499.DNG 10.6 → 3.1. Details + what still differs: docs/lr-match.md.
  Next: image-adaptive PV2012 tone + local Highlights/Shadows, Lightroom's HSL/Saturation space, ProRAW gain table
  map, iPhone HDR JPEG handling, X100F calibration, per-camera slider bias (GR III +0.35 EV, X-T2 −0.2 EV).
- 2026-10-08 (classic-ui agent) **Lightroom Classic layout** — modules Library | Develop (picker top right, G/E/D,
  ⌥⌘1/⌥⌘2, `view.library` / `view.develop`, state `UiState.module`; selection carried across; Esc in Develop →
  Library grid); filmstrip across the bottom in both (panels/filmstrip.rs: source breadcrumb, flag/rating quick
  filter, F6 hide → triangle bar, drag top edge to resize, `view.filmHeight`); Develop right panel in Classic order
  (right.rs + edit.rs `CLASSIC_PANELS`: Histogram + Original Photo, tool strip with tool drawer, Basic, Tone Curve,
  HSL / Color (HSL tabs Hue/Sat/Lum/All + Color tab + TAT), Color Grading, Detail, Lens Corrections Profile/Manual,
  Transform, Effects, Calibration; switches on all but Basic — new develop section ids `curve`, `hsl`, `grading`;
  ⌥-click solo; Previous/Reset); Develop left panel (develop_left.rs: Navigator FIT/FILL/1:1/2:1 + pan box,
  Presets (groups closed, remembered), Snapshots, History, Collections; Copy…/Paste); Library right panel
  (library_right.rs: Histogram, Quick Develop, Keywording, Keyword List, Metadata; Sync Metadata
  (`library.syncMetadata`) / Sync Settings (`dialog.syncSettings`)); Classic Import window (import_window.rs, ⇧⌘I
  and Library ▸ Import…; `import.source`; prefs remembered in ui.json `importPrefs`; Previous Import after);
  panel keys F5–F8, Tab, ⇧Tab, T, L (Lights Out); Library menu; one-row Classic sliders. Fixed a thumbnail render
  loop (render.rs `thumb_current`) and a context deadlock risk in slider typing. Tests: tests_classic.rs (+ unit
  tests in import_window / develop_left / library_right / develop). Headless shots: scratchpad `classic/*.png`.
  Next: try it on his real library (Develop on his raws, the 187 presets, BH WEDDING keyword set); wasm build not
  checked here (target not installed).
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
