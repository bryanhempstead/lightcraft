# Progress — Bryan's LightCraft (his copy of storytold/lightcraft, ~/crafts/lightcraft)

Bryan wants LightCraft to replace Lightroom Classic for him: same shortcuts, presets, catalog data, devices.
Rules are in AGENTS.md (never crash, everything is a command, *Fork rules*). Since 2026-10-09 this is his own fork
(origin = bryanhempstead/lightcraft): Adobe's DNG SDK + his installed Adobe profiles are allowed at runtime, no Adobe
files committed. Commit on `main` and push to origin. Upstream (storytold) changes are optional cherry-picks.

## Open
- [ ] (cameras agent, 2026-11-22, branch worktree-agent-acff297ced8c6b589, not merged) **LR-IMP-CAMERA-COVERAGE +
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
- [ ] (hdr2 agent, 2026-11-22, branch worktree-agent-ac64e49f32754ea1f, not merged) **HDR leftovers + LR-VID-EDIT** —
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
- [ ] (genremove agent, 2026-11-22, branch worktree-agent-a3ee2e30c0c3eb570, not merged) **Remove tool, AI side (parity I:
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
- [ ] (profiles agent, 2026-11-22, branch worktree-agent-a5465a93af5a64b95, not merged) **Profiles + adaptive presets +
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
- [ ] (faces agent, 2026-11-22, branch worktree-agent-a8e5dc1bc1eab7b7e, not merged) **Faces / People view, AI culling
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
- [ ] (fidelity agent round 11, 2026-11-22, branch worktree-agent-afb4702f45ae2dfcf, not merged) **Render round 11**
  (docs/lr-match.md → Round 11): `crates/pipeline/src/scene_cr.rs` ports RAWmakase's scene map (512 px measurement
  copy, guided base) with its Clarity, haze model and Highlights / Shadows global rule (networks copied verbatim,
  NOTICE); CPU + GPU (`crates/gpu/tests/scene_cr.rs`). Clarity evaluated at each pixel's level and refit on the oracle
  (`oracle/clafit.py`, `LIGHTCRAFT_CLARITY_DUMP`): raws +100 3.75 → 2.51 ΔE; negative Clarity keeps the old band.
  Positive Dehaze: raws +100 10.1 → 4.7 (grid +15/+30 slightly worse). H/S rule **off** (`HS_MEASURED`;
  `LIGHTCRAFT_HS_CR=1` measures it): oracle much better, acceptance 3.99 → 3.96 / passes 25 → 31 but the R6 cells
  fail the gate (needs.you #171). Texture in camera RGB measured worse (off). New oracle set `make_r11.py`
  (synthetic + 9 raws), `lcrun.sh` renders raws, `r11cmp.py`. Full 183-photo gate ran (LOT48 up): pass.
  Next: R6 shadows lift of the H/S rule, Clarity's regional moves, Texture's chroma at colour edges.
- [ ] (AI masks agent, 2026-11-22, branch worktree-agent-af352bb019bacca9d, not merged) **AI masks — parity K + REM rows**
  Done: 08552d289 SAM 3 downloads work (5 ungated HF copies, every file hash-pinned; DEFAULT_MIRRORS was empty);
  365717b7c SegFormer in pure Rust (crates/segment/src/segformer.rs, ported from candle-transformers) + three
  public parsers (scene ADE20K B5, body human-parsing B3, face CelebAMask B5) + upstream's SAM 3 subject/semantic;
  a031421cf JPEG downscale overflow crash; 8ac063f51 Sky / Landscape / People parts / Subject / Background with
  stored segmentations, Update AI Settings (Photo menu, `⚠ upd.` chips: basis hash), Remove ▸ Detect (⌥⇧O),
  ppl. / land. pickers, get. row for the parsers, `lightcraft-cli --features sam`. Verified with real models on
  public-domain photos (scratchpad only; env-gated test `tests_ai_masks`). **Left:** SAM 3 paths unrun (needs.you
  #153: Bryan's yes to the SAM License), camera focus point for Subject, Remove ▸ Detect picks every object (not one
  at a time) and fills with PatchMatch, no person thumbnails in the picker, facial hair not split from hair.
- [ ] (hdr agent, 2026-11-22, branch worktree-agent-a9350ba4bbcf77e05, not merged) **HDR (parity section Q)** —
  a1cb8f667 HDR edit (upstream's settings / SDR rendition / F32Hdr render, fitted to the fork's tone paths via
  `ToneMap::with_headroom`), c4816b7a8 gain map JPEG (craft-gainmap), 7336ed84e HDR export + `develop.hdr`, 3e92256db Basic
  panel hdr. / HDR Limit / vis.hdr. / sdr.prev. + SDR Settings, 7aaa6247c `crs:HDREditMode` / `HDRMaxValue` / `SDR*`
  read (sidecars, DNG, presets, LR catalogs) and written back, 15d68acdb HDR AVIF PQ, b82320374 PQ/HLG AVIF / PNG / JPEG XL +
  JXL gain map (Maximize Compatibility), 1aedae0df EDR loupe on macOS (`patches/egui-wgpu` via [patch.crates-io],
  half-float extended-sRGB surface; extended P3 crashes wgpu-hal 30.0.1), 6bb0fdd5a parity. Exports checked with
  macOS Core Image (all open as HDR except gain-map JXL = SDR base). needs.you #155 (loupe on the Studio Display),
  #156 (exports in Preview/Photos). Left: HDR loupe renders on the CPU (slow drags), HDR histogram region,
  Visualize HDR's beyond-display red, HDR tone not measured vs Lightroom, AVIF gain maps, Windows/Linux HDR.
- [ ] (fidelity agent, 2026-11-22, branch worktree-agent-aeda33008d910a26f, not merged) **Render-fidelity rows R16 R19–R23
  R04** against the Camera Raw oracle (docs/lr-match.md → Round 10; `oracle/make_fx.py`, `make_fuji.py`, `fx2_ana.py`,
  `grade_ana.py`, `bw_ana.py`, `local_ana.py` + `gen_local.py`, `lcrun.sh`). Found RAWmakase (github.com/pch/rawmakase,
  MIT), whose operators were measured against Camera Raw 18.7, and ported + checked on our oracle: Texture (pyramid, per
  channel), Color Grading (channel curves; legacy split toning = Blending 100), B&W grey chart, Point Color (XMP 19-number
  form + catalog fields keep Lightroom's sample), negative Dehaze; masks' Contrast / Whites / Blacks are our own measured
  curves. All CPU + GPU, each with a `MEASURED` switch (on). oracle_render now reads masks / spots / Point Color.
  Left: Clarity (image-relative, RAWmakase's 4-scale model is the next port), positive Dehaze (RAWmakase `haze.rs`),
  local Highlights / Shadows, Fuji base +0.16 EV (left until R02), Texture's colour-edge effect is ¼ of Camera Raw's.
  The LOT48 SMB share hung mid-session (needs.you #158): the gate ran on the 41 photos not on it.
- [ ] (enhance agent, 2026-11-22, branch worktree-agent-aca49497fc3d37ff4, not merged) **Enhance + Lens Blur + Depth
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
- [ ] (video-publish agent, 2026-11-22, branch worktree-agent-a6dca08b63db85737, not merged) **Video, publish, Photos,
  Map, sidecar split + 2 bugs** — catalog v8 (c617f3b19: `Op::SetVideo` trim/poster, `Op::SetRecord` library records);
  video (376135e69 engine, 58ebbe267 UI): import, Loupe bar play./pause. (Space) / scrubber / trim. / poster. / cap. /
  snd., Classic's video edits through the pipeline, trimmed edited H.264 export (Export dialog ▸ Video), ffmpeg else
  macOS AVFoundation helper (`video.swift`) else a notice; Map module (merged 1e7ba3b63, OSM tiles, pins, drag to
  geotag, saved locations); Publish Services Hard Drive + Apple Photos and File ▸ Export to Photos (merged 8f71d1b6b /
  43f30031e; needs.you #152 to try it on his real Photos library); LR-IMP-SIDECAR-SPLIT (2d4a436f7); fixes: Export
  dialog scrolls in a short window (e82d320ab, f4e7ea2d0), tethered-capture toast (toast_soon). Left: the rest of the
  Edit panel on video, pure-Rust video decode, online publish services, LR-PREF-PEOPLE (skipped: face recognition isn't
  trivial on the current face code). Headless shots: scratchpad `video-publish/shots/`.
- [ ] (detail agent, 2026-11-21 night, branch worktree-agent-af436a1c05b9b1fde, not merged) **Detail / Effects / Looks rows
  R03 R06 R07 R13 R15 R24 R25** measured with the Camera Raw oracle (`oracle/make_detail.py`, `grain_ana.py`,
  `sharp_ana.py`, `sharpfit.py`, `make_ca.py`, `make_nr_real.py`; docs/lr-match.md → Round 9): Look LUT profiles on the
  GPU (Summer Fields drag 104 → 2.5 ms), grain = Camera Raw's (|log| 0.47 → 0.05), sharpening/masking at 1:1
  (1.76 → 0.42), ProRAW "Embedded" on its own profile + gain map (9.87 → 3.00 ΔE), NR strength (1.10 → 0.40), fringe
  eyedropper; R13 measured: Camera Raw doesn't apply the profile CA model — no change. Gate green after every row
  (4.00 ΔE, 25 pass). Left: R27 AI Denoise (upstream BM3D port), R28 sky mask, Alt-drag sharpening mask preview, the
  15 Adobe-Standard ProRAW photos need Apple's per-lens DCP. Build note: worktrees need `LIGHTCRAFT_DNG_SDK` /
  `LIGHTCRAFT_LIBRAW` pointing at the main checkout's `vendor/` or the oracle/gate numbers are off by up to 8 L*.
- [ ] (tools agent, 2026-11-21 night, branch worktree-agent-a791cd84ab42f5251, not merged) **Develop tools rows U01–U05,
  R17, R29, U10, U11** — U01 per-setting Copy / Sync / Paste / New Preset checklist (43 settings, 14 headings, choose
  masks, all. / none. / def. / mod., saved subsets), U02 relative paste + Merge / Replace masks, U03 Alt-drag previews
  (ported), U04 mask Effect presets (ported; his 6 Lightroom local presets import in place via Masking ▸ Effect ▸
  import.), U05 crop extras (As Shot, Aspect Ratios / Center / Fifths, overlays to cycle, B&A button cycles layouts),
  R17 content-aware Fill spots (`crates/inpaint` from upstream; `heal_patchmatch` → Fill), R29 photos converted to
  Display P3 on a Mac (Settings ▸ Interface ▸ Photo color; needs.you #150: Bryan compares loupe vs export on the
  Studio Display), U10 point-curve extras, U11 ⌥-drag marquee deletes spots. Left: U14, U24, U12, U06, U09, U13, U07,
  U08, U17, U22, U16, U18–U21, U23 and the section-3 develop behaviours. Known load flake (passes alone):
  `ui-egui headless::tests::demo_grid_snapshot_has_ui_pixels`, `browse_a_local_folder`.
- [ ] (lrsync agent, 2026-11-21 night, branch worktree-agent-a56ca074558d7129e) **Lightroom migration rows L01 / R14 /
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
- [ ] (Claude, 2026-11-21 night) **"No gaps" program — Lightroom Classic 15.4.1 reverse-engineered (Library + Develop only).**
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
- [ ] (Claude CR3 agent, 2026-11-21, branch `worktree-agent-ab68a8dba69d04286`, not merged) **CR3 "not decoded yet"**: the CRX
  decoder has worked since 10-08 (R6 RAW: 92 files decode, ~90 ms / 20 MP, sensor identical to upstream's decoder); the notice
  was a stale catalog flag on 4,312 photos imported before it. Fix: stamped preview-only flags + background recheck on library
  open (`library.recheckPreviewOnly`, ported from upstream #662). R6 vs Lightroom previews: raw 4.61 ΔE vs embedded JPEG 7.23.
  Upstream merge assessed, not done (238 conflict hunks / 34.8k lines in 109 files; raw/tiff/meta moved to craft-libs).
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
- [ ] (Claude, 2026-11-21 audit) **Rundown — Bryan's asks and where they stand** (from his messages 10-08…11-21):
  - ✅ Lightroom catalog moved over and in sync: LightCraft 13,865 photos; LR-Cat2 has nothing captured after 10-09 (checked a copy of the .lrcat). Folders, collections, 187 presets, looks, keymap, LrSuperKeys, MIDI.
  - ✅ Pipeline wiring (brain app LC. buttons, `library.folderStatus` / `importRated` / `app.export`, photo app. lr/lc/both): commands answer live.
  - ✅ Library | Develop split, Import window, filmstrip, Classic right panel; shoots. area; any format (CR3, HEIC via macOS; Sony A7S III ARW checked 11-21: 0.7 s full render, Adobe Standard DCP installed).
  - 🟡 Colour = Lightroom ("if this doesn't work I can't use this application"): held-out mean ΔE 3.95, 24/103 photos pass. Still visible side by side on edited photos. Next round: Highlights/Shadows + Clarity/Texture/Dehaze spatial match, custom-WB 2× (non-Leica), X-T2 extreme Temp.
  - 🟡 Monogram: bridge built 10-08 but **LightCraft.monogram was never imported into Monogram Creator** (its state.json has only "Lightroom 1") — and it couldn't have worked: Monogram's importer (`Su`, service.js) reads `.monogram` assignments as LISTS per module type in the console's local-id order, so the uuid-keyed file imported empty. Fixed 11-21: `keymap::monogram_file` + `monogram_layout` (order from Monogram's own "Layout Updated" log line, read-only); `tools/monogram/LightCraft.monogram` regenerated. Hardware = Palette Gear Core V1 with its own USB-MIDI port; LightCraft's MIDI in hears it ("Palette Multi-function Device"). All 30 controls map (C{> ↓/↑ "adjust any slider" passes through to LightCraft's hover-nudge). Still untested end to end: needs Bryan to import the file in Monogram Creator and turn a dial. Protocol notes: `~/.hammerspoon/devices/re/MONOGRAM.md`.
  - Bryan doesn't want Slideshow / Print / Book / Web (10-08: "I don't need … slideshow or layout") — keep them off the priority list.
- [ ] (Claude, 2026-11-21) **Improvements vs Lightroom Classic, ranked for his wedding work:**
  1. Colour round 5 (above) — the one blocker.
  2. True 1:1 zoom: previews cap at 2,560 px, so 100 % isn't real pixels on a 24 MP M262/R6 file → focus checks while culling are unreliable (LR-VIEW-ZOOM, P0).
  3. AI Denoise (Detail panel "Denoise…") — he shoots receptions at high ISO; LR's most-used Enhance.
  4. Subject / Sky / People masks (SAM 3 is in `crates/segment`; weights need a mirror or manual install).
  5. Detail panel 1:1 loupe box (LR shows a small zoomed preview above Sharpening).
  6. Upstream storytold merges (60+ commits incl. wider preview fallback) — optional now.
- [x] (Claude, 2026-11-21) Fixed: Temp slider was linear 2,000–50,000 K (daylight at 7 % of the track, ~340 K per pixel) → Lightroom-like non-linear track (`widgets::track_frac`); Leica's "not selected" lens tag hidden in the info overlay; unrated grid cells show five quiet dots instead of an empty footer; native menu leaves Tab / ⇧Tab to the app (the `shortcuts_map_to_accelerators` failure); Monogram ↓/↑ pass-through.
- [x] (Hermes, 2026-10-11) Round-4 apply steps DONE: rebuilt (cargo 1.99, 40s), `library.rematchProfiles readCatalog=true` real run — 6,983 adobe:Adobe Color / 117 Standard / 56 Monochrome, lensCorrectionsOn 116, verified live in catalog.snap (seq 3014). App restarted via brain-app launcher so sources re-decode on the Adobe base (cache key changed).
- [ ] (Claude colour agent, 2026-11-21, uncommitted on main) **Colour round 5** — docs/lr-match.md → Round 5. Fresh held-out `w9` (103): 4.99 → 4.97 ΔE, 8 pass. Fixed: curves / creative table / grain now act in one space (ProPhoto + sRGB curve) for every export space, CPU + GPU (`finish::STYLE_SPACE`); new `tools/lr-compare/export_check.py`. Next, in order: (1) use Lightroom's *named* lens profile (`LensProfileFilename`; 469 Leica M + Voigtlander, Exif "not selected") and de-fish the Sigma 15 mm fisheye profile (213 photos, R6 defaults 11 ΔE vs 2.7); (2) colour-manage the window: the Metal layer is untagged (sRGB shown as display/P3 values) — P3-tagged surface + Develop view rendered in Display P3; (3) ICC v2 profiles + Exif ColorSpace like Lightroom's files.
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
