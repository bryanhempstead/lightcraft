# Progress — Bryan's LightCraft (his copy of storytold/lightcraft, ~/crafts/lightcraft)

Bryan wants LightCraft to replace Lightroom Classic for him: same shortcuts, presets, catalog data, devices.
Rules are in AGENTS.md (never crash, everything is a command, *Fork rules*). Since 2026-10-09 this is his own fork
(origin = bryanhempstead/lightcraft): Adobe's DNG SDK + his installed Adobe profiles are allowed at runtime, no Adobe
files committed. Commit on `main` and push to origin. Upstream (storytold) changes are optional cherry-picks.

## Open

- [ ] (Claude fix session, 2026-11-24) **Everything from 11-22 / 11-23 is merged** (every agent branch is 0 ahead of
  main; their entries moved to ## Done with their "Left:" lines). Real leftovers worth picking up: Library filter-bar
  heart toggle (only `fav:yes` search today); merges. thresholds not tried on Bryan's real brackets/panoramas; edit-stall
  (XMP queue) not tried on an external drive; HDR loupe renders on the CPU; Monogram profile not tried on the console.

- [x] (Claude fix session, 2026-11-23 13:00) **Sharp views + exports, console as a drag, fav., picked sliders** —
  `geometry::Frame::supersample` / `sample_oriented` + gpu `render::sample`: a minified output is sampled at 2× and
  box-averaged (was Mitchell prefilter → bilinear tap = double blur; Laplacian sharpness of Bryan's loupe 0.13 → 0.23,
  a Lanczos downscale 0.28; exports at reduced size 0.18 → 0.28); `panels::detail::pixel_exact` draws a render made for
  the view pixel for pixel. Monogram/MIDI moves are one interaction (`keymap::console_begin` / `console_tick`, 350 ms
  settle: drafts, no history/journal/XMP per tick). fav.: `Photo.favorite`, `Op::SetFavorite`, catalog VERSION 11,
  `cmd:photo.favorite` / `photo.toggleFavorite`, `fav:yes` search, `Filter.favorite`, heart on grid / filmstrip /
  toolbar / context menu; tap H = fav., hold H 400 ms = H's binding (`shortcuts::tap_or_hold_h`). Click a slider's
  name = picked (`widgets::picked_slider`): ↑/↓ + wheel change it, ←/→ next slider, click elsewhere / Esc lets go.
  `ui.key {state: down|up}`. Console driver side (~/.hammerspoon): slider rev. honoured, Shadows/Highlights reversed.
  Sync Settings copies from the FIRST photo picked (`Session::sync_from` / `sync_source`, set in `library.select`;
  the dialog says "from: <file>"). Every dialog: Return presses its OK (`dialogs::show`), and no shortcut fires behind
  an open dialog (`shortcuts::handle`). Settings ▸ Interface: automation clicks wait for a still widget and scroll a
  hidden one into view (`Headless::request`, `widgets::reveal_in_scroll` / `ask_reveal`, `control::Outcome::Retry`)
  — the 4 Settings tests broken by this morning's scrolling Interface tab pass again.
  View ▸ Scroll Wheel ▸ "Mouse Wheel Steps Photos (⌘ Zooms)" (`WheelMode::Photos`, `detail::navigate_gesture`).
  Photo ▸ Edit In: "Edit in <editor> ⌘E" + the editors in /Applications (`state::installed_editors`,
  `cmd:app.externalEditor {app}`; Settings ▸ General quick picks photocraft. / photoshop. / default.). PhotoCraft
  (~/crafts/photocraft, Bryan's fork) packaged with `packaging/macos/package.sh --arch aarch64` (his Apple Development
  identity) and installed to /Applications/PhotoCraft.app; opening a TIFF with `open -a PhotoCraft` verified.
  Edit In round trip: the copy is a 16-bit PSD by default (`format: psd|tiff`; PhotoCraft's ⌘S writes PSD in place,
  verified with photocraft-cli layer.duplicate → LightCraft reads it, corr 0.997), rendered WITHOUT grain
  (`ExportOptions.without_grain`) and the original's grain is set on the copy (`holdGrain`, default on; Settings ▸
  External Editor + Photo ▸ Edit In toggles). Crop dials (CropPositionX/Y, scale) now run as one interaction too.
  Left: a filter-bar heart toggle (only `fav:yes` search for now); merges. area (agent branch `merges-area`,
  worktree ~/crafts/lightcraft-merges).
- [x] (Claude fix session, 2026-11-23 09:02) **Lightroom-import repair APPLIED to Bryan's real library** (he OK'd it in the
  coordinator session: "oka fix it now"; it never ran there because the Mac was out of processes — Hammerspoon appboost
  spawn storm, fixed in ~/.hammerspoon). Backup first: `Backups/2026-11-23 pre-lr-repair/` (full catalog.snap, cmp-checked).
  Dry run = real run: crops fixed 2,395, radials fixed 82 on 80 photos, 2,452 photos, 0 skipped. Re-check dry run after:
  all `alreadyRight`. LightCraft.app relaunched (`open -a … --no-demo --control 7980`), the live app reports the same.
- NOTE: **every agent branch below marked "not merged" IS merged into main** (checked 11-23: only
  worktree-agent-a1ec001e5eb225c2e has a commit not on main, and it went in as 447dd5e39). Their "Left:" lines still stand.
- [x] (Claude, 2026-11-21 evening) **Committed**: 7096c20 LR-COLOR round 6 (Lightroom's named lens profiles incl. Sigma 15 mm
  fisheye + Voigtländer, ICC v2 sRGB / Adobe RGB, Exif ColorSpace, Make/Model; held-out fisheye 6.23 → 2.28 ΔE, R6 defaults
  4.01 → 2.52, M262 + Voigtländer 6.73 → 4.37 — docs/lr-match.md Round 6), 07d0f3c LC-SYNC (Sync ▸ Match exposure on the
  finished render, Match Brightness ⌘⌥⇧M, Auto Align ⌘⌥⇧U), 79dbc70 LC-MERGE (Focus Stack ⌃⌥F, Best Take ⌃⌥T with per-person
  face picks — Apple Vision helper `crates/engine/src/faces.swift` built once into ~/Library/Application Support/LightCraft/bin —,
  Merge… ⌃⌥M kind picker, `lc_cmd = <command>` Monogram lines). Logitech Options+ has a LightCraft profile copied from
  Lightroom's (backup in ~/Library/Application Support/LogiOptionsPlus/backup-2026-11-21/; needs.you #136).
  Left: TIFF exports have no Exif block; lens CA models unused; auto lens profile for fresh imports whose lens tag doesn't
  match; display colour management (P3); `ui-egui tests_library_problem` flakes under heavy load (passes alone).
- [x] (Claude, 2026-11-21 later) **Monogram console = a LightCraft integration** (d11982a, 8f724a2): LightCraft listens on
  127.0.0.1:50111 (`apps/lightcraft/src/monogram_server.rs`) for Monogram's app protocols (`keys.lrinput`: Lightroom lines
  and the JSON "adobe" lines); registered with Monogram from `~/.hammerspoon/devices/dev.py monoIntegrate` (ADD_APP, unsigned
  list); profile "LightCraftMonogram1" = Lightroom 1 + far-right slider `lc_cycle_cropAspect`. Cycle lists in keymap.json
  `cycles` (`keys.cycle`, `keys.cycles`). LrSuperKeys hold H/J/K/B + drag = targeted HSL; K = Luminance restored.
- [ ] **Window managers (Magnet ⌘← ⌘→ ⌘↑) need LightCraft launched through LaunchServices**: started as the brain's child
  process, macOS reports AXWindows = [] for it (AXFocusedWindow still works) and Magnet does nothing. Fixed in
  `~/second-brain/src/crafts.js` (signs builds with his Apple Development cert → stable identity, drive access kept across
  rebuilds → launches with `open -a`); uncommitted there, takes effect after a brain restart (needs.you #131). The
  `target/release/LightCraft.app` on disk is signed now; a plain `cargo build` + copy needs re-signing (crafts.js does it).
- [x] (Claude, 2026-11-21) Fixed: Temp slider was linear 2,000–50,000 K (daylight at 7 % of the track, ~340 K per pixel) → Lightroom-like non-linear track (`widgets::track_frac`); Leica's "not selected" lens tag hidden in the info overlay; unrated grid cells show five quiet dots instead of an empty footer; native menu leaves Tab / ⇧Tab to the app (the `shortcuts_map_to_accelerators` failure); Monogram ↓/↑ pass-through.
- [x] (Hermes, 2026-10-11) Round-4 apply steps DONE: rebuilt (cargo 1.99, 40s), `library.rematchProfiles readCatalog=true` real run — 6,983 adobe:Adobe Color / 117 Standard / 56 Monochrome, lensCorrectionsOn 116, verified live in catalog.snap (seq 3014). App restarted via brain-app launcher so sources re-decode on the Adobe base (cache key changed).
- [ ] Open colour items: Highlights / Shadows and Clarity / Texture / Dehaze vs spatial targets; why Lightroom's custom WB moves 2× (Leica 1×); X-T2 at 34 600 K; Nautica.
- [x] (HEIC, 2026-10-11) `migrate-lightroom --only-new` is a no-op now (imported 0, 4,047 duplicates) — 50 HEIC incl. all 35 stragglers already in catalog (verified in catalog.snap). `library.importFailures` empty. Only remaining bit: app restart so the running app decodes HEIC too.
- [ ] (colour agent round 3, 2026-10-09) **Bryan runs** — Adobe's own profiles (replaces the round-1/2 "Bryan runs" calibrate / wbmap / lensfit steps for cameras with an Adobe Standard DCP; those fitted profiles stay only as the fallback, e.g. iPhone ProRAW). LightCraft closed:
  1. `cd ~/crafts/lightcraft && sh tools/fetch-dng-sdk.sh` (already done on this Mac: vendor/ is there; re-run after a fresh clone)
  2. `cargo build --release -p lightcraft-cli -p lightcraft` (then let the brain app rebuild/launch LightCraft.app as usual)
  3. `target/release/lightcraft-cli run --library "$HOME/Pictures/LightCraft Library" library.rematchProfiles readCatalog=true dryRun=true` → check `adobe` (photos going to adobe:Adobe Color / Monochrome…) and `lensCorrectionsOn`, then the same without `dryRun=true` (one undo step)
  4. start LightCraft: sources re-decode on the Adobe base by themselves (the cache key changed).
- [ ] (colour agent round 2, 2026-10-09) **Superseded by round 3 above** for every camera with an Adobe Standard DCP (R6, M262, GR III, X-T2, X100F, Q2): only worth running for the fallback cameras (iPhone ProRAW). **Bryan runs** — replaces step 1 of the round-1 item below (steps 2–3 there stay done). App closed, after `cargo build --release -p lightcraft-cli -p lightcraft`. Reads copies of the catalog/previews, writes only `~/Library/Application Support/LightCraft/camera-profiles/` and scratch folders in /tmp; never the library. Order matters (calibrate rewrites the profiles; wbmap / lensfit add to them):
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
- [x] (folds agent, 2026-11-23, branch `agent-folds` in ~/crafts/lc-folds, merged to main by 11-24) **Every list / section folds,
  remembered** (Bryan's rule, SOUL 11-23). One map `UiState.folds` (section id → open, ui.json, `#[serde(default)]`),
  mirrored into the egui context each frame (`crate::folds::sync` at the top of `ui()`); helpers `folds::header` /
  `header_sized` / `label` / `triangle` / `toggle_on_click(_or)`; commands `ui.fold {id, open?}` (no open = toggle) and
  `ui.folds` (folded, saved, onScreen); every header registers `fold:<id>`. Default open (export's closed sections
  keep their default). Made remembered: Collections album sets, profile browser groups, Import window sections and
  folder tree, Export dialog sections + preset folders, Local folder tree, Publish services/sets, keyword tree rows.
  Made foldable: every Settings group (all tabs, incl. accents., Places and keywords, Cameras), Shortcuts groups +
  unmatched Classic keys, tool drawers (Crop, Remove, Red Eye, Masking, Info, Keywords, Versions, History when not in
  a Classic panel), Masks list, People/Landscape picker, Develop sub-groups (Sharpening, Noise Reduction, Post-Crop
  Vignetting, Grain, Calibration primaries, lens Amount/Distortion/Defringe/CA/Vignetting, mixer Hue/Sat/Lum in all.,
  Point Color Range, SDR Settings, Lens Blur Focal Range/Refine), Keyword Set + Suggestions, Metadata EXIF/IPTC
  Extension/DNG/Video, History "In Lightroom Classic", People view (Named, Unnamed, Faces, Similar, Confirmed),
  Galleries side sections, Places tags:, Map Saved Locations, grid date/publish/merges groups (▾ at the header's left;
  the rest of the header still picks the group), Library Filter column browser (folds to one line), Copy/Sync/Paste/
  New Preset checklist headings, All Metadata groups, What's New releases. Checked, nothing to fold: compare,
  rules editor, capture-time calendar, enhance, mask effect combo. Tests: `tests_folds` (command + ui.json round trip,
  folded sections hidden, layout_faults clean), `tests_grid::grid_groups_fold_by_their_triangle`. Verified: ui-egui
  --lib 417 green, clippy -D warnings green, headless screenshots looked at (Settings, Develop). Left: merge; not tried
  in the running app; grid keyboard navigation still walks through a folded group's photos.
- [x] (labels agent, 2026-11-23, branch `agent-labels` in ~/crafts/lc-labels, merged to main by 11-24) **Whole-word labels +
  color.** Bryan's rule (SOUL 11-23): labels spell every word out, lowercase, periods for spaces, "color" never
  "colour". ~90 labels renamed (clr. → clear./color., del. → delete., rem./rm./rmv. → remove., bkup. → backup., shrt. →
  shortcuts., dev.view. → develop.view., ctrl. → controllers., cams. → cameras., tl./tr./bl./br. → top.left. …, subj./bg./
  obj./ppl./desc./lin./rad./lum. → subject./background./… on the mask tiles, veg./mtn./arch./nat.gnd./art.gnd. →
  vegetation./mountains./architecture./natural.ground./artificial.ground., b&w. → black.and.white., pano. → panorama.,
  hdr.pano. → hdr.panorama. (catalog `MergeType::tag`), lc. → lightcraft., lr.* → lightroom.*, fav. → favorites., …);
  widget/command ids untouched; colour → color in every string literal (comments/identifiers untouched).
  Widths: mask tiles go 3 a row when a label needs it (`tile_layout`), `widgets::segmented` sizes segments by label
  when equal widths don't fit, profile browser filter. / view. rows stacked, fix-note edit. button 36 pt. Layout
  audit (`layout_faults`) now ignores text painted over by a later opaque rect (`HeadlessView::visible_texts`: the
  loupe bar under the Settings window). New test `tests_masking::whole_word_labels_fit_mask_tiles_and_profile_segments`.
  Verified: ui-egui --lib 413 green, clippy -D warnings green; screenshots looked at (mask tiles, profiles, settings).
  Left: merge; Settings `choices()` rows keep their old 4 pt gaps (rule says 8); not tried in the running app.
- [x] (hitch agent, 2026-11-23, branch `agent-hitch`, worktree ~/crafts/lc-hitch, merged to main by 11-24) **Preview lag: commits off
  the UI thread, crop dials cut one render.** Measured first (probes below, Bryan's M262 DNG, 16k-photo on-disk library
  with his presets.json, XMP auto-write on, load 20–45). (1) Every committed edit wrote the XMP sidecar on the UI
  thread: `StemOwners::of` over the whole catalog (6–15 ms) + read/merge/fsync-write on the photo's drive (~10 ms on
  the internal SSD; an external HDD's fsync is the likely 479 ms frame) — now `xmp_queue`: the app
  (`Session::set_xmp_deferred(true)`) queues the photo, writes it on an `xmp-writer` thread once still for 1.5 s (at
  once when the photo is left), several edits = one write; the queue is library record `xmp.pending` journaled with
  the edit, so a crash re-writes it on the next open; quit (`notices::may_close`), `close_library` and a library
  switch flush it. CLI/MCP/tests keep writing at once. `StemOwners::for_photos` = one prefix pass for the written
  photos. (2) `persist()` rebuilt the 0.5 MB presets.json on EVERY command (2–8 ms, also each console tick): now
  only when presets / profile lists changed (`PresetsSeen`, field-by-field compare), never mid-drag. Slider-click
  commit on the UI thread: 22–34 ms → 0.3–1.7 ms. (3) Console moves ended with a bare `end_interaction`: never
  auto-wrote the sidecar — now `develop.endInteraction` (`keymap::end_console_move`). (4) The "700 ms start-of-turn
  stall" on Shadows was the probe: LC_BENCH_EDIT sets Shadows +30, so detents 71–90 were past +100 and counted as
  unseen until the sharp render (p90). Probe fixed (unchanged detents count as shown); going the other way: median
  16–29 ms, p90 30 ms, no stall (a turn started within ~250 ms of a new edit's sharp render still waits for the
  draft-size warm-up, by design). (5) Crop X/Y dial: every detent was a whole-pipeline render (~80–120 ms at 849 px,
  ~8/s); the "1.1 s median" was the same probe artifact (the crop hit the photo's edge). Now one uncropped draft per
  turn (`Session::reframe_draft_job`, settings minus crop/flips/constrain, the view's decoded source, ≤ 2560 px) and
  the loupe cuts each detent's crop out of it (`detail::crop_mesh`, straighten + flips; `app.reframe_key`): tick →
  screen median 117–133 → 1–2 ms, p90 149–169 → 38 ms (first detent ~150–200 ms), sharper drafts (1415 vs 849 px).
  Tests: engine `deferred_auto_write_is_written_after_the_debounce_and_on_close`,
  `deferred_auto_write_survives_a_crash_and_leaving_the_photo`; ui `crop_dial_draws_its_detents_out_of_one_render`
  (cut vs sharp mean |Δ| 2.5; a 5% wrong cut fails at 8.6). Probes (ignored): `commit_hitch` (new: LC_BENCH_RAW,
  LC_BENCH_PHOTOS, LC_BENCH_PRESETS), `console_dial_lag` + LC_BENCH_GAP / LC_BENCH_TRACE; LIGHTCRAFT_PROFILE=1 prints
  slow commands' stages (`engine: <cmd> … (run, xmp, persist)`). Left: not tried in Bryan's running app or on an
  external drive (worktree builds have no DNG SDK / LibRaw: vendor/ isn't in worktrees); crop.scale turns zoom into
  the uncropped draft (softer while turning); the sharp render after a crop turn is still a whole-pipeline render
  (~250–300 ms after the 350 ms settle); a sharp texture upload is ~15 ms of one frame.
- [x] (pipeline agent, 2026-11-23, branch `agent-pipeline` in ~/crafts/lc-pipeline, merged to main by 11-24) **LC-SB-PIPELINE: Second
  Brain shoot pipeline marks + send.to.pipeline.** (Bryan: "add an area that marks that it came from the 2nd brain
  pipeline … if it's not then I can retroactively put it there … mark or ask if I want it backed up and then put it in
  the currently editing area"). Engine `crates/engine/src/sb_pipeline.rs`: reads `<SB userData>/shootpipe.json`
  READ-ONLY (Session.second_brain_dir / LIGHTCRAFT_SECOND_BRAIN_DIR / ~/Library/Application Support/second-brain),
  re-read only when its mtime/size or the inbox changes; bad/missing file = no marks. `cmd:pipeline.status {folder?}`,
  `cmd:pipeline.add {folder, backup: run|done|skip, name?}` → a request file in `<SB userData>/lightcraft-inbox/<id>.json`
  (temp + rename; LightCraft NEVER writes shootpipe.json) + a best-effort nudge `POST /v1/pipeline/inbox` on SB's
  gallery API. UI `crates/ui-egui/src/sb_pipeline.rs`: `pipeline.<stage>` row under the shoot's topmost Folders row
  (left.rs `pipeline_row`, stage words: imported, backed.up, culling, editing, exporting, delivering, posting,
  finished; `pipeline.pending` until SB takes a request), Info line "from the second brain pipeline · editing" /
  `send.to.pipeline.`, folder right-click `send.to.pipeline.` → `Dialog::SendToPipeline` (back.up. /
  already.backed.up. / skip., cancel. + accent ok., Return = ok.). Tests: engine `tests_sb_pipeline.rs`, ui-egui
  `tests_sb_pipeline.rs` (rows, info, stage change, dialog layout audit, queue + pending).
  Second Brain side (in ~/second-brain, see its board): src/shootpipe.js `fromLightCraft` + `drainInbox` + launch /
  fs.watch / 60 s pickup, src/galleryapi.js route, test/shootpipe-lightcraft.test.js — uncommitted there, but the running SB (restarted 18:05) already has it: the route answers.
  Verified: `cargo test --release -p lightcraft-engine -p lightcraft-ui-egui --lib` 635 + 403 green; clippy -D warnings
  green for ui-egui and for engine with `--features sam,enhance` (engine alone without features fails on 2 dead-code
  items already on main: genremove.rs Shared::set/current, remove_detect.rs remove_found_add). Not tried in the
  running app (not restarted). Second Brain part UNCOMMITTED (hold blocked by a stale ask; needs.you #224).
  Left: merge this branch; restart both apps (ask first); Bryan tries it on a real folder (needs.you #225).
- [x] (crop-lens agent, 2026-11-23, branch `agent-crop-lens`, worktree ~/crafts/lc-crop-lens, merged to main by 11-24) **Crop dials one
  axis, lens profiles by hand, loupe drafts.** (1) LR-CONSOLE-CROP: `crop.nudge` (now an engine command) moved a
  straightened crop back inside with `CropGeometry::constrained`, which pulls towards the image centre on both axes —
  every CropPositionX detent near a rotated edge also moved Y; and in the Crop tool (photo unstraightened, crop a tilted
  frame) the crop's own X looked diagonal. Now `CropGeometry::shifted` / `scaled` (one axis, as far as the rotated photo
  allows; scale about the centre keeping the shape) and `axes: image` while the Crop tool is open. (2)
  LR-EDIT-OPTICS-PROFILE: Lens Corrections ▸ Profile shows the profile in use + make. / model. / prof. pickers over the
  installed .lcp files, auto. / none.; `cmd:lens.profiles` / `lens.setProfile` / `lens.profileCorrections`; Enable
  Profile Corrections takes the Set Lens lens's profile (`lcp::match_lens`), Set Lens re-matches while it's on; fixed
  the .lcp parser cutting entries at nested lists (912 of 3602 installed profiles, every Leica M one). On this Mac:
  Heliar 1:4.5/15 → VM 15mm f/4.5 Super Wide Heliar II Aspherical (or III — Bryan to check), Summicron-M 1:2/35 →
  SUMMICRON-M 35 f/2 ASPH. (if his is the pre-ASPH one, pick it by hand), Summicron-M 1:2/50 → SUMMICRON-M 50 f/2,
  Elmar 1:4/135 → no profile installed. (3) Loupe drafts: the loupe's slot runs one render at a time
  (`JobPool::set_exclusive`; 3–4 drafts used to share the GPU), drafts share the sharp render's decoded source
  (`Session::view_draft_job`), the draft size's stages are warmed in the background once the sharp render is shown
  (`Renderer::warm`), crop dials draft at 0.35 (every draft is a whole-pipeline render), the edited photo's thumbnail
  waits for the move to end. Probe: `console_dial_lag` (ignored; LC_BENCH_RAW=… LC_BENCH_LEGACY=1 for the old way).
  Measured on his M262 DNG, edited, 2427 px loupe, 30 detents/s, old vs new interleaved on a loaded machine (load 40):
  Clarity — drafts shown 35–43 → 78–80 of 90, tick→screen median 62–76 → 23 ms, p90 278–330 → 77–151 ms; Shadows —
  drafts 35–38 → 65–66, median 70 → 22 ms (p90 ~730 ms both: a stall at the start of the turn, not found); steady-
  state Shadows draft 5–14 ms (gpu finish only). Crop X dial: draft 279 → 89 ms (849 px), but tick→screen still ~1.1 s
  median — not explained yet (per-draft GPU ~80 ms, yet only ~5 drafts/s land).
  Left: commit-time costs on the UI thread not measured here (XMP auto-write walks `StemOwners::of` the whole catalog
  and writes to the photo's drive per committed edit; `persist()` per command) — candidates for his 479 ms hitches.
- [x] (map-tags agent, 2026-11-23, merged 11-23 19:10) **LR-MAP-PLACES: no photos in the ocean,
  place search, album places, suggested keywords.** Map: no pin without a position or at 0, 0 ("N without a
  location" bottom left → untagged.); EXIF reader drops void GPS (GPSStatus V / no refs / 0,0 — the M262's), XMP 0,0;
  `cmd:library.fixVoidGps {dryRun?}` clears stored 0,0 (one undo step). Lenses: `catalog::clean_lens` ("not
  selected" / "----" / "Unknown" = no lens; GR III/IIIx, X100, Q, RX1 name theirs) on import +
  `cmd:library.fixLensNames {dryRun?}`. Engine `places.rs` + `cmd/places.rs`: `location.search` (coordinates local;
  Nominatim, only the typed text, ≤1 req/s, `geocode-cache.json`), `location.apply` (selection / album / folder,
  one undo step, XMP), `location.setDefault` / `location.default`, imports into an album / folder with a place get
  it (`import.autoPlace` / `placeKeywords` on, `autoKeywords` off), `keywords.suggest` (place, album name, CLIP
  content from the AI search index) / `applySuggested` / `dropSuggested`. UI: Library right panel ▸ Place & Tags
  (`ui-egui/src/panels/places.rs`, `view.places`), Settings ▸ Import rows. Left: run `library.fixVoidGps` and
  `library.fixLensNames` on Bryan's library after merging; no EXIF LensID fallback (the meta crate reads none);
  content tags need the AI search index built; not tried against the live Nominatim server.
- [x] (Claude fix session, 2026-11-23 16:20) **Bryan's 11-23 list — HANDOFF STATE (read this first)**
  - DONE + in the running app: sharp loupe/exports, console = one drag, fav. (tap/hold H), picked sliders, sync from
    first pick + Return = OK, Edit In ⌘E → PhotoCraft (PSD, grain held), wheel steps photos, merges. (merged),
    Leica M apertures (6,154 repaired), presets' Look → profile (29 linked), Set Lens + lens list (Heliar 1:4.5/15,
    Summicron-M 1:2/35, 1:2/50, Leitz Elmar 1:4/135), quick tools after `|` (center./dust.), WB pick., file-type
    tags, delete → next photo, crop grid while adjusting, no stale 1:1 tiles, every action bindable + searchable.
  - 22:45 UPDATE: ALSO merged + pushed + in the running apps: `agent-hitch` (deferred XMP sidecars, crop dials from
    one render), `agent-labels` (full-word labels, both apps), `agent-folds` (every list/section folds, remembered —
    `ui.fold` / `ui.folds`; via the CLI call them as {"method":"engine.execute","params":{"command":"ui.folds"}}, the
    shorthand treats ui.* as protocol methods), grid ←/→ skip folded groups (91eccc187). PhotoCraft with labels +
    folds installed in /Applications. Nothing running. Left: Settings `choices()` rows keep 4 pt gaps (<8 px rule);
    docs still name some old labels (parity.md, merge.md "pano."); hitch fix not tried on an external drive.
  - 20:15 UPDATE: ALSO merged + pushed + in the running app: `agent-map-tags` (fixVoidGps 6,474 + fixLensNames 7,567
    APPLIED on Bryan's library), `agent-crop-lens` (Bryan: Heliar II = auto pick OK; his Summicron 35 is the OLD non-ASPH —
    no Adobe profile exists, ASPH profile is the closest auto pick), `agent-pipeline` (Second Brain side committed
    d71a66e). PhotoCraft English-only + green icon installed in /Applications. RUNNING: `agent-hitch` (~/crafts/lc-hitch:
    UI-thread stall per committed edit — XMP/StemOwners/persist; Shadows start stall; crop-dial throughput),
    `agent-labels` (~/crafts/lc-labels + ~/crafts/pc-labels: full-word labels, list of non-collapsible sections).
  - 18:55 UPDATE: committed + pushed + in the running app: wheel/edge tabs/vignette/Ctrl→⌘/Esc/WB dropper (4190614fb),
    importRated keeps LightCraft stars/flags/labels on photos already in the library, merges of `english-only`
    (+ Settings ▸ Language kept with english. only — Bryan: "I only meant English only as the language options"),
    `agent-panels-dialogs`, `agent-accents` (aae157105). Worktree target/ dirs of those deleted. Second Brain ingest
    fixes committed (2d53198) + app restarted; photo_app = lc (Bryan). Stale HOLD-REQUEST cleared (Bryan OK).
    PhotoCraft: english-only merged (eb74db9, PhotoCraft ▸ Language ▸ English kept), green Ps icon now in
    assets/app-icon (d64757d) — package + install to /Applications pending.
    Agent builds were SIGSTOPped so merges went first (scratchpad agent-builds.sh stop|cont) — resume them.
  - AGENTS STILL RUNNING (worktrees; + `agent-pipeline` ~/crafts/lc-pipeline: 2nd-brain pipeline tag + send.to.pipeline.): `agent-crop-lens` (~/crafts/lc-crop-lens: crop X dial moving Y, lens profile
    corrections per lens, preview/console lag — Bryan: "still preview lag"), `agent-accents` (~/crafts/lc-accents:
    up to 4 accent colours, wheel + presets, roles), `agent-map-tags` (~/crafts/lc-map-tags: no-GPS photos off the
    map, void GPS fix (M262 GPSStatus=V → 6,474 photos at 0,0) + repair, lens placeholders "not selected" → none +
    repair, place search/apply, keyword suggestions).
  - TODO by me: `library.importRated` (engine/src/cmd/shoot.rs) must not overwrite stars changed in LightCraft
    (only set when rating is 0 / keepExisting); after merges: restart LightCraft (log in ~/second-brain/.restarts.log,
    `lightcraft-cli run --connect app.quit`, cp binary into target/release/LightCraft.app, codesign --identifier
    ai.storyteller.lightcraft with F142493EF338CE19AF1072995A7AAA4961F53DE9, open -a … --args --no-demo --control 7980),
    run repairs from map-tags (void GPS, lens placeholders) dry run first; delete agent worktrees' target/ dirs.
  - Second Brain (agent done, UNCOMMITTED in ~/second-brain: cull previews keep raw orientation; LightCraft auto
    send gets the landed folders not the drive root). Blocked on stale ~/second-brain/.agent/HOLD-REQUEST (Bryan:
    needs.you #218/#219, question #220 photo app setting). ~/.hammerspoon dev. sync is committed (ddaa414).
  - Hammerspoon/dev. macros (Ctrl→⌘) are the "Steam Deck modifier for keyboard macro" session's
    (local_6e1fc28f…); LightCraft keymap's 22 Ctrl+Alt+Shift+letter Monogram codes stay (⌘⌥⇧ collides with 7 built-ins).
  - A watcher keeps cargo/rustc at background QoS until ~19:00 (task in the session).
  - Waiting on Bryan: a "flat" photo example (#217).
  - NEW RULE (Bryan 11-23, ~/.hermes/SOUL.md): every UI label spelled out in full, lowercase, periods for spaces, no
    abbreviations, "color" never "clr." (e.g. dl.→download., enh.→enhance., ovl.→overlay., wb.→white.balance.,
    sav.→save., dflt→default.). TODO after the agent branches merge: relabel LightCraft (and PhotoCraft) labels in
    one pass (agent), update tests that click by label ids only if ids change (prefer keeping widget ids).
  - Done elsewhere: NEBULA68B layer 1 ⌘-position key now LGUI (deck session); dev. has no Ctrl; Lightroom-import
    repair re-checked 16:45 (0 changes; 2,390 crops + 82 radials right, 5 skipped = edited here).
- [x] (merges agent, 2026-11-23, branch `merges-area`, merged to main by 11-24) **LC-MERGE-FIND: merges. — found photo merges in
  the sidebar** (Bryan: "recognize possible photo merges and put those in a separate area … on the side bar").
  Engine `crates/engine/src/merge_find.rs`: per folder, capture-time runs (body / lens / focal, a few s apart) →
  brackets from exposure steps (`hdr.`), pano / focus stack / hdr pano from 64 px thumbnails (shift search, 4×4
  sharpness tiles) or recorded focus distance; groups are `merge.group.<id>` records (open / merged / dismissed, never
  re-suggested). `cmd:merge.find` / `candidates` / `mergeGroups` / `dismiss` / `setKind` / `unmerge` / `showIn`,
  `library.source {kind: merges, path|id}` (`LibrarySource::Merges` + `Session.merges_scope`). Catalog **v12**:
  `Photo.merge` (`MergeMark {into, kind, onlyMerges}`, `Op::SetMerge`) — merged originals leave every view and
  count (`in_library`, `Filter`, album counts), never deleted; `Meta.exposure_bias` / `focus_distance` (meta: Exif
  SubjectDistance). App `crates/ui-egui/src/merges.rs`: Settings ▸ General ▸ find merges: auto. (default, background
  per-folder fingerprints) / find.; `merges.` rows under folders (left.rs `folder_rows`) and collections
  (`albums_tree_view`); the view's bar (find. all. none. merge. skip. both. merges.only. unmerge.) and group headers
  (kind menu, merge., skip.); merges run one at a time on the merge worker (`merge::start_group` / `next_group`);
  `hdr.` / `pano.` / `stack.` / `hdr.pano.` tags on grid + filmstrip. Tests: engine `tests_merges.rs` +
  `merge_find` unit tests, ui-egui `tests_merges.rs` (row, view, layout audit, merge → originals gone, tag).
  Left: not tried on Bryan's real brackets / panoramas (thresholds from synthetic tests — needs.you test.); a skipped
  group comes back only by ⌘Z; on a 15k library the first auto pass loads thumbnails for every same-exposure run
  (background, low priority) — watch it once; no hdr.pano. join of separate groups by hand.
- [x] (cropfix agent, 2026-11-22, branch worktree-agent-a146fa0dc5606b7bd, merged to main by 11-24) **LR-EDIT-CROP: straightened
  crops from Lightroom** — `crs::crop_from_lr` / `crop_to_lr` / `crop_props` (Lightroom's crop edges = two corners of
  the rotated crop in the unrotated image; needs the stored aspect: `to_partial_report`'s, sidecars re-fitted by
  `crs::refit_crop`); sidecars and exported originals carry `crs:HasCrop` + crop fields in the stored frame
  (`sidecar::sidecar_packet_for_file`, `lightroom_crop_props`; the sync hash packet stays without them);
  `cmd:library.repairLightroomImport` (`lr_repair.rs`: crops + grouped radials still as migrated, skips ones changed
  here, before/after dry run, one undo step; `crs_masks::masks_legacy_flipped` = the old reading). Acceptance 206:
  4.65 → 3.94 ΔE, passing 34 → 42. NOT run on Bryan's library: dry run first
  (`lightcraft-cli run --library <lib> library.repairLightroomImport dryRun=true`). Left: Lightroom snapshots / history
  versions keep the old crop; Lightroom sidecars on EXIF-turned files are read in the shown frame.
- [x] (polish agent, 2026-11-22, branch worktree-agent-a07ce8bef78df8a0c, merged to main by 11-24) **LC-POLISH: loose ends of
  2026-11-22** — Before half of side-by-side / split Before/After at 1:1 gets its own native tiles (`detail.rs`
  `detail_layer`, view 3); Develop's Collections = the Library rows with parts triangles (`left::develop_collections`);
  Esc with a popup open only closes it, Esc closes notes. when it has the keyboard (`fixnotes::has_focus`); Settings
  gets its own window id + hints wrap to the visible dialog; dialog buttons 8 px apart; layout audit test
  (`tests_panels::settings_tabs_and_new_panels_have_nothing_clipped`, `HeadlessView::texts`); ~440 plain + 44 format
  messages in all six catalogs. Left: HDR rendition at 1:1 (region renders are SDR), pets (docs/people.md → Pets).
- [x] (genfill agent, 2026-11-22, branch worktree-agent-aed52566890eddbd3, merged to main by 11-24) **LR-REM-GEN: gen.ai. is a
  diffusion model** — Stable Diffusion 1.5 Inpainting (CreativeML OpenRAIL-M, Enhance model `gen-fill`, 3 pinned
  files 2.13 GB; `ModelInfo.parts`) on candle: `crates/enhance/src/sd/` (candle-transformers 0.9.2 SD code, vendored,
  NOTICE), `crates/enhance/src/genfill.rs` (LaMa start → 12 DDIM steps at 512 px, refining tiles to 1536 px, grain,
  pull–push seamless join, CPU-seeded noise), `netrun::conv2d_taps` (SD's convs), engine `genremove.rs` (engine per
  spot `Generative.quick`, seeds, LaMa fallback with `.lama` markers, auto-remake when the model arrives, progress +
  `remove.cancel`, `remove.generate {remake}`), Remove panel `quick.` / `stop.` / `remake.`. ~10 s a 512 px
  variation, ~18 s 1024 px on an idle M3 Pro; much slower under load. needs.you #204 (download OK), #205 (real-photo
  test). Left: VAE ~40 % of the time (TAESD?), haze/softness vs Lightroom, first fill pays kernel compile.
- [x] (qa agent, 2026-11-22, branch worktree-agent-afcfd18c32ee6aa74, merged to main by 11-24) **QA-PERF: measured on a copy of
  Bryan's 15,865-photo library** — `docs/perf.md` (numbers before/after, how to measure, what's still slow).
  Fixed: smart-album recount per keystroke (`crates/catalog/src/album_counts.rs`, `AlbumCountMemo`), fsync per op
  on the UI thread (`FsStore` fsyncs on a worker), render results converted to egui images on the UI thread
  (`render::Prepared` / `Pixels`, made on the render worker; 1:1 pan stalls up to 1.5 s), 60 fps polling while
  renders run (`JobPool::set_on_done` wakes the UI), GPU readback waiting for all GPU work in the process
  (`finish_and_read` waits for its own submission), every demo session repainting every demo photo on the shared rayon pool (the "0 thumbnails after 120 s" UI-test flake; `media::demo_render` shares them),
  `source_total` sorting the catalog to count it (`Catalog::count`), `LibraryLock::acquire` refusing a reopen while
  a forked child still held the lock (retries 2 s when no owner note); `ui.inspect` → `perf.window` + `resetPerf`;
  `Renderer::diagnose` in test failure messages; the grid / Alt-preview / identity-plate tests wait for what they
  check. Left (in docs/perf.md): first-view raw decodes for thumbnails (19 % cached), Shoots / folder tree /
  visible sort rebuilt per change, export + 1:1 render time, `keep_pixels` CPU copies with `--control`.
- [x] (fixnotes agent, 2026-11-22, branch worktree-agent-abcb4f6b3b76a1a30, merged to main by 11-24) **LC-UI-FIXNOTES: notes. →
  fix. → Claude** — `crates/ui-egui/src/fixnotes.rs` (new: top-bar `notes.` button with open-count badge, floating
  panel ⌘⌥⇧N, context capture, screenshots via `LightcraftApp::queue_screenshot` (the `ui.screenshot` path), request
  writer), `cmd:notes.toggle/add/list/send/markFixed/reopen/edit/remove/select`, `docs/fix-notes.md` (request format
  for the Claude-side watcher). Files: `<config>/fix-notes.json`, `fix-notes-shots/`, `fix-inbox/<YYYYMMDD-HHMMSS>-fix.json`
  (UTC; `LIGHTCRAFT_FIX_INBOX` overrides). `lightcraft_engine::logging::{remember, recent}` keep the last 40
  warnings/errors (desktop AppLogger + StderrLog feed it); ui-egui build.rs sets `LIGHTCRAFT_GIT_COMMIT`. ⌘⇧N was
  taken (New Folder), hence ⌘⌥⇧N. Left: the Claude-side folder watcher (not app code); Bryan to try it.
- [x] (plate agent, 2026-11-22, branch worktree-agent-aa8ef530a41bd0901, merged to main by 11-24) **LRC-UI-IDENTITYPLATE + clean
  launch** — `crates/ui-egui/src/identity.rs` (new: settings, decode with caps, SVG via resvg, tint, copy into
  `<config>/identity/`, layout that slides off controls, paint, preview), Settings ▸ identity. (`identity_tab`,
  `tint_picker`; `panels::edit::paint_hue_wheel` shared with Color Grading), `cmd:app.identityPlate` /
  `cmd:app.identityPlateSetup`, top bar publishes its free space (`identity::BAR_SLOT`), `AppSettings.identity_plate`,
  `Services.pick_image`, `LightcraftApp::prepare` (fonts before frame 1), main.rs opaque `clear_color` + transparent
  window until the first frame (the grey box at launch). needs.you #195. Left: plates in exported slideshows/prints
  (no such modules), multi-font text.
- [x] (zoomux agent, 2026-11-22, branch worktree-agent-aa922a86531cd4ef5, merged to main by 11-24) **LR-VIEW-ZOOM click / wheel
  zoom, LRC-VIEW-LOUPEOVERLAY, LRC-LIB-MISSING loupe banner** — `UiState.wheel` / `wheel_step` / `loupe_overlay`
  (new, saved in ui.json), `click_zoom` 0 = Fill; `view.wheelZoom`, `view.navigate {remember}` (toolbar zoom menu +
  Navigator set the click level), `view.loupeOverlay` (⌘⌥O) / `overlayShow` / `loupeGrid` / `loupeGuides` /
  `layoutImage` / `chooseLayoutImage` / `compositionOverlay` (O in the loupe) / `compositionOverlayOrientation` (⇧O) /
  `frameGuides`; drawing + picker in `panels/crop_overlay.rs`; `detail.rs` `missing_banner` (loc. / fldr.);
  `ui.scroll {unit: line}`; ui-egui now depends on lightcraft-codecs (layout image decode). Gesture zoom max 800 → 1600.
  Left: Bryan to try the wheel on his mouse + trackpad (needs.you).
- [x] (captime agent, 2026-11-22, branch worktree-agent-a045d8d9ff069e865, merged to main by 11-24) **LR-LIB-CAPTURETIME: right-click
  ▸ Adjust Time…** in grid / filmstrip / loupe / Develop / Compare (`dialog.captureTime {anchor}`), anchored on the photo
  right-clicked; new dialog `crates/ui-egui/src/capture_time_dialog.rs` (thumbnail, big current time, typed time
  `3:34 pm` / `15:34` / `+1h` / `-1d`, calendar, step buttons, offset + count + before → after, Enter / Esc, apply.);
  modes adj. shift. zone. same.time. match.cam. orig. file.date.; engine plan `crates/engine/src/capture_time.rs`
  (`anchor`, `match`, `dryRun`; virtual copies + collapsed-stack members follow; undated photos skipped) and
  `dates::parse_time_input`. Fix: XMP auto-write now writes capture-time changes. Left: writing the time into
  JPEG/DNG files, telling two bodies of one model apart.
- [x] (subfolders agent, 2026-11-22, branch worktree-agent-a0f3c68e00ff61adc, merged to main by 11-24) **LRC-LIB-COLL-PARTS:
  a shoot's / collection's camera folders under its row** — every shoots. row (Bryan's weddings) and every Collections row (smart, in sets, and in the Map module's
  left panel, which now lists Collections under Saved Locations) has a disclosure triangle listing the folders its
  photos come from (`raw/M262 · LEICA M (Typ 262)`, `raw/R6`, `jpg`; `fldr.` / `cam.` in the header for cameras);
  click / ⌘ / ⇧ / all. / none. narrow the collection (`shoot.show {path, folders | cameras}`, `shoot.parts`, `library.source {kind: album, folders | cameras}`,
  `album.parts`; `Session::album_part`, kept in view.json); drop onto a row adds to the collection; ← / → over a
  collection close / open it. Camera folders beside them on disk with photos not in the library (`GR3 · not in library 401 add.`; photos counted, pairs once, sidecars never) import in place into the collection (`import::start_paths_into`). `crates/engine/src/album_parts.rs`, `left.rs` (`album_part_rows`), headless tests
  `tests_collection_parts.rs`. Left: cameras by model only (no serial), Develop's Collections panel has no triangles.
- [x] (export agent, 2026-11-22, branch worktree-agent-a3771648dd17295ab, merged to main by 11-24; main merged in twice, DNG
  preview / lossy options kept) **LR-EXP-DIALOG: Export dialog laid out like Lightroom Classic's** —
  `crates/ui-egui/src/export_dialog.rs` (new), `export_task.rs` (queue, pause), `crates/engine/src/export.rs`
  (`ExportTo`, `Conflict::Ask` + `existing_targets` / `resolve`, `MetadataPolicy` ×6, `ResizeMode::WidthHeight`,
  `SizeUnit`, `ResolutionUnit`, `ExportService`, `simple_copyright_watermark`, `photoshop_app`, `preview_job`),
  `cmd/export.rs` (preset folders, rename, move, write / import incl. `.lrtemplate` via `export_lrtemplate.rs`,
  `export.existingFiles`, `export.actionsFolder`). `Dialog::Export.opts` is boxed now; Dialog::ExportExisting;
  `UiState.dialog_next`. New parity rows LR-EXP-PRESETFOLDERS / SERVICE / PLUGINS / PREVIEW / QUEUE. needs.you #189.
  Left: watermark rotation / Fit / Fill, DPX video, a plug-in API, Lightroom custom watermarks in `.lrtemplate`s.
- [x] (faces agent round 2, 2026-11-22, branch worktree-agent-a60c94c49b5526429, merged to main by 11-24) **People / culling
  finished** (LR-PREF-PEOPLE, LR-LIB-PEOPLE, LRC-LIB-PEOPLE, LR-LIB-CULL, LR-FILT-CULL, LR-IMP-CULL-AT-IMPORT,
  LRC-IMP-PREVIEWCULL, LR-MASK-SUBJECT; `docs/people.md`). **Catalog v10** (`Region.drawn`, `Analysis.smiles` /
  `expression`; merge note: another branch bumping to v10 must renumber). Faces found by themselves
  (`faces.auto`, Settings ▸ General ▸ Find faces off./view./all.; half the workers, waits while the user works,
  top-bar activity strip with stop, `resume.`, imports join the front, failed photos skipped); Draw Face Region in
  the loupe (draw / move / resize / rename); OpenCV Zoo FER expression model (4.8 MB, Apache-2.0, own package
  `expressions`; identical to ONNX Runtime on 38 crops; face index record v2) → smiles / expression in culling,
  rules and the dialog; culling on worker threads (`CullJob`, `cull_poll`) and at import as bookkeeping
  (`library.import {cull}`); person keywords ↔ face names in step; People bulk row (name / yes. / no. / rm.) and
  opened stacks; Select Subject uses the camera's focus point (`crates/meta/src/focus.rs`: upstream's Sony reader
  + Canon AFInfo2/3 checked on Bryan's R6 CR3s, Nikon AFInfo2, Fujifilm FocusPixel from ExifTool docs only).
  PRelu/per-channel broadcast fast path in the ONNX interpreter (PRelu 300 → 2 ms a face under load).
  Left: pets, expression network on a labelled set of Bryan's photos, Nikon/Fuji focus on real files, 13k-library
  throughput without the build load.
- [x] (cameras agent, 2026-11-22, branch worktree-agent-acff297ced8c6b589, merged to main by 11-24) **LR-IMP-CAMERA-COVERAGE +
  LR-IMP-FORMATS from Bryan's real library** — `docs/camera-coverage.md`: all 15 raw bodies (catalog + drives) decode
  natively, crop = Lightroom's (smart previews' OriginalDefaultFinalSize), levels = LibRaw, ΔE vs his stored previews.
  69d692264 JPEG XL DNG native (jxl-oxide; his 3,745 JXL smart previews, ΔE 0.64 vs original), deba98211 LibRaw
  fallback keeps the file's DNG/RAF tags (BaselineExposure, opcodes, gain map, DR bias: was up to 3 EV dark),
  ee46520e8 + 9f58b2133 Canon black from the real masked columns (T3 2282→2048, CC0 6D 2591→2047…), 4378f2085 CR2
  white = ColorData SpecularWhiteLevel, 32d96d428 Rebel/Kiss names → Camera Raw profile (T3 rendered washed out),
  7e1f99a05 CinemaDNG make. lr-compare.json was briefly overwritten by my compare run (tag nat); a later r12g4 run by
  another agent has replaced it since, so nothing to restore. needs.you #183 (rebuild Previews.lrdata), #184 (where the
  //lot48/…/MY MEDIA originals are). Left: Lightroom ΔE for Q2 / 5D III / iPhone 17 / Sony / Nikon (no previews),
  specular vs normal white for Canon, DNG SDK built without JXL.
- [x] (zoom agent, 2026-11-22, branch worktree-agent-a0b644f80b07fbd8d, merged to main by 11-24) **LR-VIEW-ZOOM ✅ + LR-VIEW-COLORMANAGED ✅**
  — region renders (`crates/pipeline/src/region.rs`: `RenderRequest::region`, `Frame::window`, `Globals`,
  `render_region`, `lightcraft_gpu::render_region`, `Session::detail_job`); the loupe / its before view / Compare
  draw native-resolution 512 px pieces over the capped preview at 1:1 and closer (`crates/ui-egui/src/tiles.rs`,
  `Slot::Detail(view)`, `ui.inspect` → `perf.detail`). Region = same pixels as the full-size render (CPU max 2 LSB,
  GPU max 1). Colour: the 8-bit window surface is tagged Display P3 on a Mac (`patches/egui-wgpu`
  `request_sdr_color_space`; `display_p3::WindowSpace`, `app.viewSpace` → `colorManaged`), video / previews / map
  tiles converted like photos. Merge note: `Rendered` gained `region`, `RenderRequest` gained `region`,
  `RenderJob` gained `context`; the finish kernel has 4 new params at the end (`WIN_*`). Left: Before half of
  Before/After and HDR renditions at 1:1 stay at preview size.
- [x] (hdr2 agent, 2026-11-22, branch worktree-agent-ac64e49f32754ea1f, merged to main by 11-24) **HDR leftovers + LR-VID-EDIT** —
  e0f0ea988 HDR loupe on the GPU (finish kernel's float HDR output, `PEAK`/`SCENE_TOP` params appended; CPU/GPU
  equivalence crates/gpu/tests/hdr.rs; 1936x1290 draft drag 12–14 ms GPU vs 340–620 ms CPU under load ~100; UI
  half-float conversion threaded 49 → ~10 ms), 5f71af34b the whole Edit panel on video (video_settings keeps all but
  geometry / per-photo tools / HDR; develop_frame = media::develop, GPU, full quality for playback and export),
  0c2d7825a crs:SDRBlend rendered (highlight saturation, Hdr::rendition_blend, CPU+GPU), 34adf5884 HDR histogram
  (pipeline::hdr_histogram + panels/hdr_histogram.rs, Slot::HdrMaster 512 px render, RenderJob::hdr_master),
  Visualize HDR beyond-the-display red (Overlay::HdrRange(headroom tenths), Lightroom's cyan/blue/purple/magenta),
  HDR in Compare / Survey / Second Window, 655697c9a gain-map AVIF via macOS ImageIO (engine/src/imageio.rs + .swift;
  Core Image reads it as HDR, peak 4.0x). needs.you #161 (Studio Display look), #162 (AVIF in Preview/Photos, edited
  video playback). Left: J clipping in HDR mode (Lightroom's yellow/red), HDR tone vs Lightroom, SDR rendition is the
  HDR-off render (Lightroom: local tone map), Second Window uses the main display's headroom, Windows/Linux HDR.
  Note for merging: crates/gpu/src/wgsl/finish.wgsl's look block became `fn look(d, x, y)` (same code, moved) —
  the Clarity/Dehaze builder edits the operators above it.
- [x] (genremove agent, 2026-11-22, branch worktree-agent-a3ee2e30c0c3eb570, merged to main by 11-24) **Remove tool, AI side (parity I:
  LR-REM-GEN 🟡, LR-REM-DETECT ✅, LR-REM-PEOPLE ✅, LR-REM-REFLECT 🟡, KEY-GENAI ✅; `docs/remove.md`)** — 327bc1b17 LaMa
  big-lama on candle (FFT as cos/sin matrix products, matched to PyTorch to 3e-6), a22cb17ad Generative Remove spots
  (gen.ai., 3 variations via context/mirror/scale recipes, < / > and ⌥← / ⌥→, more.; fills cached in
  ~/Library/Caches/LightCraft/Remove before white balance, remade from the photo when missing; PatchMatch stand-in +
  notice without the model) and Detect candidates (det. → chips + highlighted outlines, click / ⌘ / ⇧, all. / none.,
  rem.N; one generative spot per object), fbf698d66 Remove Reflections (DSRNet + VGG-19, matched to PyTorch to 1.4e-6;
  refl. / rmv. / keep. / Amount, layer at ≤ 512 px subtracted after white balance). Models in the Enhance catalog:
  inpaint, reflection (authors' Google Drive), vgg19 (timm on HF), all pinned. Verified headless on public-domain / CC0
  photos (scratch only). needs.you #163–#165. Left: LaMa ~5 s per 1024² variation (f16 / faster 7×7 convs), fills
  capped at 1024 px, wires (SAM 3) not run, Find Dust Spots over-reports on textured photos, Lightroom's generative /
  reflection XMP not read.
- [x] (profiles agent, 2026-11-22, branch worktree-agent-a5465a93af5a64b95, merged to main by 11-24) **Profiles + adaptive presets +
  SR speed** — 05f28a03c SR channels-last on Metal (24 MP 217 → 43 s, PSNR 73–79 dB vs the old path); LR-PROF-CAMERA /
  LEGACY / ADAPTIVE: Camera Matching + imported DCPs (Camera Raw `CameraProfiles/Camera|Imported`, `.dcp` import into
  the library) decode on their own DCP through the SDK (`cam:<name>`, `files::with_base_profile`, sources cached per
  base), Legacy `legacy:ACR 4.4` (matrices only + ACR3 curve), Adaptive Color / B&W look tables (per-photo part needs
  Camera Raw's encrypted `ai_look` model); GR III vs the SDK's reference render ΔE76 1.0–1.8; U09: browser filter
  all./clr./b&w., views grid./lrg./lst., mng. (hide groups), imp., profile groups per photo in menu + browser;
  LR-PRE-ADAPTIVE / LR-MASK-ADAPTIVE: Camera Raw's Premium + Adobe presets offered read only (`adobe.<uuid>`,
  `presets.adobe`, app loads at start), Landscape / People `MaskSubCategoryID` mapped, preset.apply computes new AI
  masks (a4ea8443f; 20dcf2a89: People presets detect people first, Blur Background → Lens Blur; real parsers on
  teton / crew: Blue Drama sky, Whiten Teeth, Autumn01 5/7 classes, Pop subject). Bryan's catalog: 16,823 photos all
  resolve (no Camera Matching used). Left: Fujifilm film sims (no DCPs installed here), Snow class, Facial Hair = hair,
  People masks = first person, Adaptive per-photo tables, Cat Eye / oval bokeh. needs.you #166, #167.
- [x] (faces agent, 2026-11-22, branch worktree-agent-a8e5dc1bc1eab7b7e, merged to main by 11-24) **Faces / People view, AI culling
  criteria, natural-language search** (LR-PREF-PEOPLE, LR-LIB-PEOPLE, LRC-LIB-PEOPLE, LR-FILT-PEOPLE, LR-LIB-CULL,
  LR-FILT-SEARCH-AI; `docs/people.md`). New crate `crates/vision` (Apache-2.0): our own ONNX interpreter running OpenCV
  Zoo's YuNet + SFace and Intel's open-closed-eye (checked against OpenCV 4.12), CLIP ViT-B/32 (LAION) ported from
  candle-transformers, face clustering, append-only checksummed side indexes (`<library>/vision/*.lcidx`). **Catalog
  v9** (`Op::SetRegions`, `Region.rejected`, `Analysis.faces/eyes_closed/face_focus`; older formats kept as
  `catalog.snap.v8.bak` on upgrade). Engine: `faces.find` / `search.index` background runs (resumable, cancellable,
  progress), People (named / unnamed stacks / suggestions / ✓ ✗ / person keywords / rename / remove / addFace),
  `search.ai` + `Filter.semantic`, culling blinks + face focus, MWG regions written to XMP, Lightroom `AgLibraryFace`
  migrated. UI: People view rewritten, loupe `?` name field, search field `ai.` mode, cull dialog face options.
  Left: finding faces automatically at import, drawing face boxes with the mouse, eye model false positives on
  dogs / squints (needs.you), throughput measured only under a load average of ~140 (other builders).
- [x] (fidelity agent round 11, 2026-11-22, branch worktree-agent-afb4702f45ae2dfcf, merged to main by 11-24) **Render round 11**
  (docs/lr-match.md → Round 11): `crates/pipeline/src/scene_cr.rs` ports RAWmakase's scene map (512 px measurement
  copy, guided base) with its Clarity, haze model and Highlights / Shadows global rule (networks copied verbatim,
  NOTICE); CPU + GPU (`crates/gpu/tests/scene_cr.rs`). Clarity evaluated at each pixel's level and refit on the oracle
  (`oracle/clafit.py`, `LIGHTCRAFT_CLARITY_DUMP`): raws +100 3.75 → 2.51 ΔE; negative Clarity keeps the old band.
  Positive Dehaze: raws +100 10.1 → 4.7 (grid +15/+30 slightly worse). H/S rule **off** (`HS_MEASURED`;
  `LIGHTCRAFT_HS_CR=1` measures it): oracle much better, acceptance 3.99 → 3.96 / passes 25 → 31 but the R6 cells
  fail the gate (needs.you #171). Texture in camera RGB measured worse (off). New oracle set `make_r11.py`
  (synthetic + 9 raws), `lcrun.sh` renders raws, `r11cmp.py`. Full 183-photo gate ran (LOT48 up): pass.
  Next: R6 shadows lift of the H/S rule, Clarity's regional moves, Texture's chroma at colour edges.
- [x] (AI masks agent, 2026-11-22, branch worktree-agent-af352bb019bacca9d, merged to main by 11-24) **AI masks — parity K + REM rows**
  Done: 08552d289 SAM 3 downloads work (5 ungated HF copies, every file hash-pinned; DEFAULT_MIRRORS was empty);
  365717b7c SegFormer in pure Rust (crates/segment/src/segformer.rs, ported from candle-transformers) + three
  public parsers (scene ADE20K B5, body human-parsing B3, face CelebAMask B5) + upstream's SAM 3 subject/semantic;
  a031421cf JPEG downscale overflow crash; 8ac063f51 Sky / Landscape / People parts / Subject / Background with
  stored segmentations, Update AI Settings (Photo menu, `⚠ upd.` chips: basis hash), Remove ▸ Detect (⌥⇧O),
  ppl. / land. pickers, get. row for the parsers, `lightcraft-cli --features sam`. Verified with real models on
  public-domain photos (scratchpad only; env-gated test `tests_ai_masks`). **Left:** SAM 3 paths unrun (needs.you
  #153: Bryan's yes to the SAM License), camera focus point for Subject, Remove ▸ Detect picks every object (not one
  at a time) and fills with PatchMatch, no person thumbnails in the picker, facial hair not split from hair.
- [x] (hdr agent, 2026-11-22, branch worktree-agent-a9350ba4bbcf77e05, merged to main by 11-24) **HDR (parity section Q)** —
  a1cb8f667 HDR edit (upstream's settings / SDR rendition / F32Hdr render, fitted to the fork's tone paths via
  `ToneMap::with_headroom`), c4816b7a8 gain map JPEG (craft-gainmap), 7336ed84e HDR export + `develop.hdr`, 3e92256db Basic
  panel hdr. / HDR Limit / vis.hdr. / sdr.prev. + SDR Settings, 7aaa6247c `crs:HDREditMode` / `HDRMaxValue` / `SDR*`
  read (sidecars, DNG, presets, LR catalogs) and written back, 15d68acdb HDR AVIF PQ, b82320374 PQ/HLG AVIF / PNG / JPEG XL +
  JXL gain map (Maximize Compatibility), 1aedae0df EDR loupe on macOS (`patches/egui-wgpu` via [patch.crates-io],
  half-float extended-sRGB surface; extended P3 crashes wgpu-hal 30.0.1), 6bb0fdd5a parity. Exports checked with
  macOS Core Image (all open as HDR except gain-map JXL = SDR base). needs.you #155 (loupe on the Studio Display),
  #156 (exports in Preview/Photos). Left: HDR loupe renders on the CPU (slow drags), HDR histogram region,
  Visualize HDR's beyond-display red, HDR tone not measured vs Lightroom, AVIF gain maps, Windows/Linux HDR.
- [x] (fidelity agent, 2026-11-22, branch worktree-agent-aeda33008d910a26f, merged to main by 11-24) **Render-fidelity rows R16 R19–R23
  R04** against the Camera Raw oracle (docs/lr-match.md → Round 10; `oracle/make_fx.py`, `make_fuji.py`, `fx2_ana.py`,
  `grade_ana.py`, `bw_ana.py`, `local_ana.py` + `gen_local.py`, `lcrun.sh`). Found RAWmakase (github.com/pch/rawmakase,
  MIT), whose operators were measured against Camera Raw 18.7, and ported + checked on our oracle: Texture (pyramid, per
  channel), Color Grading (channel curves; legacy split toning = Blending 100), B&W grey chart, Point Color (XMP 19-number
  form + catalog fields keep Lightroom's sample), negative Dehaze; masks' Contrast / Whites / Blacks are our own measured
  curves. All CPU + GPU, each with a `MEASURED` switch (on). oracle_render now reads masks / spots / Point Color.
  Left: Clarity (image-relative, RAWmakase's 4-scale model is the next port), positive Dehaze (RAWmakase `haze.rs`),
  local Highlights / Shadows, Fuji base +0.16 EV (left until R02), Texture's colour-edge effect is ¼ of Camera Raw's.
  The LOT48 SMB share hung mid-session (needs.you #158): the gate ran on the 41 photos not on it.
- [x] (enhance agent, 2026-11-22, branch worktree-agent-aca49497fc3d37ff4, merged to main by 11-24) **Enhance + Lens Blur + Depth
  Range** (rows DENOISE ✅, RAWDETAILS ✅, SUPERRES 🟡, ENH-DIALOG ✅, ENH-INPLACE ✅, MENU-PHOTO-ENHANCE ✅, LENSBLUR ✅,
  MASK-DEPTHRANGE ✅; `docs/enhance.md`). New crates: `enhance` (candle runners: RawNIND raw denoise via upstream's ONNX
  reader, Real-ESRGAN general x4v3 SR, Depth Anything V2 Small depth; pinned consented downloads), `denoise-core`
  (upstream, built-in BM3D + Bayer tiling), `rcd` (RCD demosaic from RawTherapee, GPL). Photo ▸ Enhance… (⌃⌥I) →
  `<name>-Enhanced-NR.dng` stacked, or in place (cache + `enhance` settings, decoded by `files::load_raw`). Lens Blur
  panel + `lensBlur.*`, Depth Range masks. Measured on a GR III ISO 4000 DNG (M3 Pro): RawNIND denoise 11.2 s,
  built-in 37.6 s, RCD 0.34 s. **Found:** candle 0.9.2's CPU conv2d is wrong when channels == width (affects
  `crates/segment` on non-Metal machines), and candle compiles Metal kernels ~30 s on first use per device (one shared
  device + warm-up thread here). Left: SR speed, Cat Eye bokeh, on-photo depth overlay, X-Trans denoise (RawNIND's
  linear model), compare against Lightroom's Denoise (needs.you #157).
- [x] (video-publish agent, 2026-11-22, branch worktree-agent-a6dca08b63db85737, merged to main by 11-24) **Video, publish, Photos,
  Map, sidecar split + 2 bugs** — catalog v8 (c617f3b19: `Op::SetVideo` trim/poster, `Op::SetRecord` library records);
  video (376135e69 engine, 58ebbe267 UI): import, Loupe bar play./pause. (Space) / scrubber / trim. / poster. / cap. /
  snd., Classic's video edits through the pipeline, trimmed edited H.264 export (Export dialog ▸ Video), ffmpeg else
  macOS AVFoundation helper (`video.swift`) else a notice; Map module (merged 1e7ba3b63, OSM tiles, pins, drag to
  geotag, saved locations); Publish Services Hard Drive + Apple Photos and File ▸ Export to Photos (merged 8f71d1b6b /
  43f30031e; needs.you #152 to try it on his real Photos library); LR-IMP-SIDECAR-SPLIT (2d4a436f7); fixes: Export
  dialog scrolls in a short window (e82d320ab, f4e7ea2d0), tethered-capture toast (toast_soon). Left: the rest of the
  Edit panel on video, pure-Rust video decode, online publish services, LR-PREF-PEOPLE (skipped: face recognition isn't
  trivial on the current face code). Headless shots: scratchpad `video-publish/shots/`.
- [x] (detail agent, 2026-11-21 night, branch worktree-agent-af436a1c05b9b1fde, merged to main by 11-24) **Detail / Effects / Looks rows
  R03 R06 R07 R13 R15 R24 R25** measured with the Camera Raw oracle (`oracle/make_detail.py`, `grain_ana.py`,
  `sharp_ana.py`, `sharpfit.py`, `make_ca.py`, `make_nr_real.py`; docs/lr-match.md → Round 9): Look LUT profiles on the
  GPU (Summer Fields drag 104 → 2.5 ms), grain = Camera Raw's (|log| 0.47 → 0.05), sharpening/masking at 1:1
  (1.76 → 0.42), ProRAW "Embedded" on its own profile + gain map (9.87 → 3.00 ΔE), NR strength (1.10 → 0.40), fringe
  eyedropper; R13 measured: Camera Raw doesn't apply the profile CA model — no change. Gate green after every row
  (4.00 ΔE, 25 pass). Left: R27 AI Denoise (upstream BM3D port), R28 sky mask, Alt-drag sharpening mask preview, the
  15 Adobe-Standard ProRAW photos need Apple's per-lens DCP. Build note: worktrees need `LIGHTCRAFT_DNG_SDK` /
  `LIGHTCRAFT_LIBRAW` pointing at the main checkout's `vendor/` or the oracle/gate numbers are off by up to 8 L*.
- [x] (tools agent, 2026-11-21 night, branch worktree-agent-a791cd84ab42f5251, merged to main by 11-24) **Develop tools rows U01–U05,
  R17, R29, U10, U11** — U01 per-setting Copy / Sync / Paste / New Preset checklist (43 settings, 14 headings, choose
  masks, all. / none. / def. / mod., saved subsets), U02 relative paste + Merge / Replace masks, U03 Alt-drag previews
  (ported), U04 mask Effect presets (ported; his 6 Lightroom local presets import in place via Masking ▸ Effect ▸
  import.), U05 crop extras (As Shot, Aspect Ratios / Center / Fifths, overlays to cycle, B&A button cycles layouts),
  R17 content-aware Fill spots (`crates/inpaint` from upstream; `heal_patchmatch` → Fill), R29 photos converted to
  Display P3 on a Mac (Settings ▸ Interface ▸ Photo color; needs.you #150: Bryan compares loupe vs export on the
  Studio Display), U10 point-curve extras, U11 ⌥-drag marquee deletes spots. Left: U14, U24, U12, U06, U09, U13, U07,
  U08, U17, U22, U16, U18–U21, U23 and the section-3 develop behaviours. Known load flake (passes alone):
  `ui-egui headless::tests::demo_grid_snapshot_has_ui_pixels`, `browse_a_local_folder`.
- [x] (lrsync agent, 2026-11-21 night, branch worktree-agent-a56ca074558d7129e) **Lightroom migration rows L01 / R14 /
  U15 / R18 / R26** — c456db97f R14 (Upright: PerspectiveUpright 3 = Level / 4 = Vertical / 2 = Full, stored
  `UprightTransform_<mode>` used instead of re-detection, Constrain Crop; `repairLightroomMigration upright`),
  657ebd80d U15 + R18 (history side files `<library>/Lightroom History/`, History panel "In Lightroom Classic",
  `history.restoreLightroom`; snapshots → versions; `library.migrateLightroomHistory`), e077d6fcb L01
  (`library.syncLightroom`, File ▸ Sync from Lightroom Classic…, `<library>/lightroom-sync.json`), then R26
  (`library.lightroomDust`). **Bryan runs, on the live library after merging** (app closed or via the menu):
  `library.migrateLightroomHistory` (15 s: 13,773 photos / 73,373 steps, 19 snapshots → versions), File ▸ Sync from
  Lightroom Classic… (today: 1 photo re-edited after the migration snapshot, 14 duplicates, 2,947 offline), then
  `library.lightroomDust dryRun=true` → without (12 of 39 photos in the library, 38 spots). Left: the 192 Upright
  photos (Medora, BONUSBOY) aren't in the library (folder missing); Find Dust Spots misses some faint specks
  Lightroom removed (detector sensitivity); `migrated` time in lightroom-migration.json is the CLI's fixed clock
  (2026-09-30), so first-sync baselines use the newest Lightroom time the migration recorded instead.
- [x] (Claude, 2026-11-21 night) **"No gaps" program — Lightroom Classic 15.4.1 reverse-engineered (Library + Develop only).**
  Research (local only, Adobe text never in the repo): /private/tmp/claude-501/lr-re/{library,develop}/inventory.md,
  usage table from his catalog, ranked list gaps.md (103 rows: Tier 1 = 23, Tier 2 = 22, Tier 3 = 58). Running on branches
  (merge one at a time into main, full gate after each): upstream storytold/lightcraft merge (4,818 commits: CR3, Denoise,
  Lens Blur, People …), LibRaw fallback decoder (crates/libraw-sys), Sync ▸ Match camera & lens color (chart match per
  camera after each camera renders like Lightroom; per-lens / per-phone cast tables learned from his catalog; ProRAW gain
  table map). Tier 1 build order after the merge: per-look acceptance gate (Summer Fields, Nautica × camera) → point
  curve → calibration → PV5 vs PV6 split → ProRAW gain map → Basic tone on edited photos (local Highlights/Shadows) →
  Look RGBTable on GPU (7,457 photos render on CPU) → Copy/Sync per-slider groups (+ "Modified", saved subsets) →
  custom WB → grain → parametric curve → sharpening 126/94 at 1:1 → re-sync edits from the Lightroom catalog (mobile) →
  desktop catalog backups → lens CA model → HSL on JPEG/HEIC → Nautica → radial-mask local tone → content-aware spots →
  mobile snapshots as versions → stored Upright transforms → Point Color → Develop view color management (Studio
  Display). Then Tier 2 (weekly use), Tier 3 (everything else: AI masks, generative remove, HDR, culling …).
- [x] (Claude CR3 agent, 2026-11-21, branch `worktree-agent-ab68a8dba69d04286`, merged to main by 11-24) **CR3 "not decoded yet"**: the CRX
  decoder has worked since 10-08 (R6 RAW: 92 files decode, ~90 ms / 20 MP, sensor identical to upstream's decoder); the notice
  was a stale catalog flag on 4,312 photos imported before it. Fix: stamped preview-only flags + background recheck on library
  open (`library.recheckPreviewOnly`, ported from upstream #662). R6 vs Lightroom previews: raw 4.61 ΔE vs embedded JPEG 7.23.
  Upstream merge assessed, not done (238 conflict hunks / 34.8k lines in 109 files; raw/tiff/meta moved to craft-libs).
- [x] (Claude, 2026-11-21 audit) **Rundown — Bryan's asks and where they stand** (from his messages 10-08…11-21):
  - ✅ Lightroom catalog moved over and in sync: LightCraft 13,865 photos; LR-Cat2 has nothing captured after 10-09 (checked a copy of the .lrcat). Folders, collections, 187 presets, looks, keymap, LrSuperKeys, MIDI.
  - ✅ Pipeline wiring (brain app LC. buttons, `library.folderStatus` / `importRated` / `app.export`, photo app. lr/lc/both): commands answer live.
  - ✅ Library | Develop split, Import window, filmstrip, Classic right panel; shoots. area; any format (CR3, HEIC via macOS; Sony A7S III ARW checked 11-21: 0.7 s full render, Adobe Standard DCP installed).
  - 🟡 Colour = Lightroom ("if this doesn't work I can't use this application"): held-out mean ΔE 3.95, 24/103 photos pass. Still visible side by side on edited photos. Next round: Highlights/Shadows + Clarity/Texture/Dehaze spatial match, custom-WB 2× (non-Leica), X-T2 extreme Temp.
  - 🟡 Monogram: bridge built 10-08 but **LightCraft.monogram was never imported into Monogram Creator** (its state.json has only "Lightroom 1") — and it couldn't have worked: Monogram's importer (`Su`, service.js) reads `.monogram` assignments as LISTS per module type in the console's local-id order, so the uuid-keyed file imported empty. Fixed 11-21: `keymap::monogram_file` + `monogram_layout` (order from Monogram's own "Layout Updated" log line, read-only); `tools/monogram/LightCraft.monogram` regenerated. Hardware = Palette Gear Core V1 with its own USB-MIDI port; LightCraft's MIDI in hears it ("Palette Multi-function Device"). All 30 controls map (C{> ↓/↑ "adjust any slider" passes through to LightCraft's hover-nudge). Still untested end to end: needs Bryan to import the file in Monogram Creator and turn a dial. Protocol notes: `~/.hammerspoon/devices/re/MONOGRAM.md`.
  - Bryan doesn't want Slideshow / Print / Book / Web (10-08: "I don't need … slideshow or layout") — keep them off the priority list.
- [x] (Claude, 2026-11-21) **Improvements vs Lightroom Classic, ranked for his wedding work:**
  1. Colour round 5 (above) — the one blocker.
  2. True 1:1 zoom: previews cap at 2,560 px, so 100 % isn't real pixels on a 24 MP M262/R6 file → focus checks while culling are unreliable (LR-VIEW-ZOOM, P0).
  3. AI Denoise (Detail panel "Denoise…") — he shoots receptions at high ISO; LR's most-used Enhance.
  4. Subject / Sky / People masks (SAM 3 is in `crates/segment`; weights need a mirror or manual install).
  5. Detail panel 1:1 loupe box (LR shows a small zoomed preview above Sharpening).
  6. Upstream storytold merges (60+ commits incl. wider preview fallback) — optional now.
- [x] (Claude colour agent, 2026-11-21, uncommitted on main) **Colour round 5** — docs/lr-match.md → Round 5. Fresh held-out `w9` (103): 4.99 → 4.97 ΔE, 8 pass. Fixed: curves / creative table / grain now act in one space (ProPhoto + sRGB curve) for every export space, CPU + GPU (`finish::STYLE_SPACE`); new `tools/lr-compare/export_check.py`. Next, in order: (1) use Lightroom's *named* lens profile (`LensProfileFilename`; 469 Leica M + Voigtlander, Exif "not selected") and de-fish the Sigma 15 mm fisheye profile (213 photos, R6 defaults 11 ΔE vs 2.7); (2) colour-manage the window: the Metal layer is untagged (sRGB shown as display/P3 values) — P3-tagged surface + Develop view rendered in Display P3; (3) ICC v2 profiles + Exif ColorSpace like Lightroom's files.
- 2026-10-09 (colour agent round 3) **Adobe's own profiles through the DNG SDK** — policy 41d81b5, SDK crate 92571ab, base ef77b2c + this round. `crates/dng-sdk-sys` (DNG SDK 1.7.1 from adobe.com, vendor/ gitignored, C ABI shim), `engine/src/adobe.rs` (Adobe Standard DCP colour spec + hue/sat map at decode, Adobe Raw looks as `adobe:<name>`, exact Temp/Tint, per-camera exposure offsets), `engine/src/lcp.rs` (Adobe lens profiles), `pipeline/src/adobe.rs` (look tables + ACR3/profile curve as RGBTone in ProPhoto, CPU + GPU, equivalence test), Basic tone refitted on the exact base (`tone::lr::ADOBE_*`), `LIGHTCRAFT_LR_DUMP` per-pixel dumps. Held-out B 4.46 → 4.46 ΔE (p95 10.7, passing 21 → 23), all 5.13 → 4.94, X100F 6.39 → 4.94, B&W t2 4.57 → 4.32. Details, tables, what misses: docs/lr-match.md → Round 3. Scratch: colour agent scratchpad `lc3/` (fit.py, dump.py, run.sh, breport.py, grid-round3*.png).
  Next: Lightroom's PV2012 sliders on heavily edited photos are the remaining gap (per-pixel tone error ~3.7 L*, not a per-photo offset); a Camera Raw (Photoshop) oracle on synthetic DNGs would measure them exactly — needs Bryan's OK to script Photoshop. Then calibration sliders (greens/teals), grain (finer, fainter), 1:1 edge contrast (~0.8), X-T2 raw exposure bias, ProRAW gain table map.
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
