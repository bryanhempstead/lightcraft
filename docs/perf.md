# Performance on a real library

Measured on Bryan's library (a copy: catalog, thumbnail cache and settings in a scratch folder, XMP
auto-write off, originals read-only on `/Volumes/BONUSBOY` and `/Volumes/hempstead.studio`): 15,865
photos — 6,514 Leica M262 DNG, 4,312 Canon R6 CR3, 1,576 iPhone JPEG, 1,263 Sony A7S III ARW, 1,209
Ricoh GR III DNG, 685 Fuji X-T2 / X100F RAF and a few others. MacBook Pro M3 Pro (12 cores, 18 GB),
release build, windowed app on its own control port, `LIGHTCRAFT_PROFILE=1`.

**Read every number with its load average.** Other agents were building and testing on the same
machine the whole time (load average 50–660 on 12 cores), so absolute times are pessimistic and
noisy; the UI-thread profiles (`sample <pid>`) say where the time goes regardless of load.

## How to measure

- `ui.inspect` → `perf.window`: frames, frames over 16.7 / 50 / 100 ms and the slowest update (with
  its logic part) since the last `ui.inspect {"resetPerf": true}` — the stalls a sampled `updateMs`
  misses. Reset, do one thing, read it.
- `library.info` → `persistence` (`lastAppendMs`, `maxAppendMs`); `ui.inspect` → `memory`; macOS
  `footprint -p <pid>` (RSS under-reports: GPU buffers and compressed pages).
- `sample <pid> 6` while a script repeats the action: the main thread's call tree shows what a frame
  spends its time on.
- Scripts used (JSON-lines control channel): open a source, scroll the grid, `library.next` through a
  shoot waiting for `loupe.source == "render"`, `ui.dragWidget` on `slider:light.exposure`,
  `ui.key` P / U / 3 / 0 with auto-advance, `view.navigate` zoom 100 % + 60 pans, `app.export` 20
  JPEGs in the background, idle with `ps -o cputime`.

## Results (2026-11-22)

| Workflow | Before | After | Notes |
|---|---|---|---|
| Open the library | 633 ms (15,865 photos, 0 log records) | — | first frame 543 ms (450 ms logic) |
| Scroll All Photographs, 150 × 400 pt | 151 frames, worst 53 ms, 71 fps (load 600) | — | fill after the scroll 2.5 s (~130 thumbnails) |
| Grid, first screen of a source never browsed | 86 s at load 600; 4.9 s for a new screen at load 160 (2 s waiting on raw decodes) | — | the thumbnail cache held 3,010 of 15,865 photos (19 %): most shoots decode raws on first view |
| Flag P / U with auto-advance, All Photographs (worst frame per key) | p50 88, p90 159, max 363 ms (load 58); 69 / 73 / 87 (load 314) | **39 / 46 / 90 ms (load 523)** | profile: smart-album counts 314 → 33 samples, fsync 215 → 0 |
| Same in a 2,735-photo shoot | 65 / 75 / 81 ms (load 315) | **28 / 36 / 44 ms (load 532)** | |
| Stars 3 / 0 | 78 / 86 / 87 ms (load 55) | see flags | same path |
| Catalog append (one op) | last 4.8 ms, **max 421 ms** (F_FULLFSYNC on the UI thread) | written on the UI thread, fsynced on a worker | `library.info` → `persistence` |
| Loupe next through a shoot (30 photos, load 230) | first pixels p50 17 ms / p90 27 ms; sharp p50 0.8 s / p90 1.3 s / max 2.2 s; UI worst 17 ms | — | ~10 raw decodes per step (filmstrip thumbnails, prefetch) |
| Loupe next, 200 photos of the 2,735-photo shoot (load 650) | first pixels p50 18 / p90 47 ms; sharp p50 3.3 s / p90 8.2 s / max 21 s; UI: 43 frames > 100 ms (worst 923 ms), 58 frames/s for 873 s | — | before the render-result and polling fixes; sharp time is render-bound at that load |
| Develop drag, exposure, 40 steps (load 160) | drafts 16–57 ms each, sharp 0.6 s after release; UI worst 28 ms | — | M262 / R6 / A7S III full renders 0.8 / 1.4 / 2.0 s at load 600 |
| 1:1 zoom | sharp in 9–60 s at load 350–620 (full-resolution raw source 4–17 s under that load) | — | render side, not the UI |
| 1:1 pan, 60 steps on a photo not seen before (worst frame per step) | p90 32–70 ms, **max 87–1,524 ms** (load 56–475) | **p90 2–5 ms, max 12–44 ms (load 600)** | profile: `Renderer::poll` pixel conversion 510 → 0 samples |
| Export 20 full-size JPEGs (background) | 13.9 s/photo (load 100); UI: 32 frames > 100 ms, worst 555 ms | 8 photos: 6.9 s/photo (load 580); UI: 2 frames > 100 ms, worst 211 ms | export speed is the render path under load; budget is 1 s/24 MP |
| Filter / sort / select all (15,865) | worst frame 6–64 ms | — | |
| Idle (grid, loupe, People view, settled) | 0.2–0.6 frames/s, 1 % CPU | — | |
| Idle while thumbnails render (grid) | 60 frames/s polling for results (68 % CPU incl. workers) | woken by each result | |
| Memory | footprint 1.6 GB after open, 5.5 GB peak after develop + 1:1, back to 2.7–4 GB | — | GPU buffers are most of the peak |
| Log | no panics; warnings only for a busy Monogram port and wrong command names in the scripts | | |

## What was fixed (QA-PERF)

- **Smart-album counts after every change** (`crates/catalog/src/album_counts.rs`): each flag or star
  re-tested every photo against every smart album (two "keyword contains Duplicate" albums: 31,730
  rule evaluations, 10–30 ms a keystroke). `AlbumCountMemo` keeps which smart albums each photo
  matched and re-tests only photos whose `Arc` changed; albums, stacks, label names and the clock are
  compared and any change starts over. Test: `memoized_counts_match_a_full_count_and_retest_only_changed_photos`.
- **fsync on the UI thread** (`crates/catalog/src/store.rs`): `FsStore::append` wrote and
  `F_FULLFSYNC`ed every op before returning (5 ms typical, 421 ms worst). It now writes before
  returning (an app crash keeps the bytes) and fsyncs on a worker that coalesces queued appends; a
  failed fsync fails the next append, and dropping the store waits. Test:
  `appends_are_written_at_once_and_fsynced_on_the_worker`.
- **Render results converted on the UI thread** (`crates/ui-egui/src/render.rs`, `tiles.rs`): every
  finished render was copied to bytes, converted to an egui image and (Display P3) converted again on
  the UI thread — 20–60 ms for a 2560 px loupe render and up to 1.5 s for a burst of 1:1 pieces.
  The render worker now makes `Pixels` (sRGB + shown) and the UI thread only uploads them.
- **60 fps polling while renders run**: the job pool now wakes the UI when a result is ready
  (`JobPool::set_on_done`); the frame loop polls every 250 ms as a safety net only. Test:
  `pool_wakes_after_each_result_is_ready`.
- **Every GPU readback waited for all GPU work in the process** (`crates/gpu/src/ctx.rs`
  `finish_and_read`): `device.poll(Wait)` without a submission index waited for whatever any other
  thread had queued too, so a thumbnail or the loupe sat behind an export's (or another view's)
  GPU work, and in the 12-thread UI test run the grid's thumbnails got no result for 120 s
  (`0 done, 10 in flight`). It now waits for its own submission.
- **The grid's source total sorted the whole catalog** after every change just to count it
  (`Catalog::count`).
- `ui.inspect` → `perf.window` / `resetPerf` (above), so the next measurement is one call.
- **Reopening a library right after closing it** could say "already open in another LightCraft"
  under load (seen once per full engine test run): `LibraryLock::acquire` now retries for up to
  2 s while the holder left no owner note or is this process (a forked child's copy of the lock);
  another program still gets refused at once. Test: `a_holder_without_an_owner_note_is_waited_for`.

## UI tests under load

`headless::tests::demo_grid_snapshot_has_ui_pixels`, `tests_alt_preview::alt_during_a_blacks_drag…` and
`tests_identity::text_plate_and_modules` failed in full `cargo test` runs on a loaded machine. With a
renderer diagnosis in the message (`Renderer::diagnose`), the grid test showed `0 done, 10 in
flight` after 120 s: no render finished at all, because every GPU readback waited for all GPU work
of the 12 tests running in the process (fixed above). The tests now also wait for what they check:
the grid until every on-screen cell has its thumbnail, the Alt preview until it (and then the photo)
is on screen, the identity plate until it is drawn — instead of a `settle()` that needs every render
in the app to finish. A 91-test subset at load ~600: before 1 failure (the grid test, `0 done`);
after, the grid and Alt tests pass. At load 700 some other render-heavy tests
(`snapshots_are_deterministic…`, `tests_hdr::compare_survey…`, `browse_a_local_folder`) still time out
after 120–300 s: that is CPU starvation, not a hang.

## Still slow (ordered by how much it hurts culling and editing)

1. **First view of a shoot decodes raws for thumbnails**: only 19 % of the library has a cached
   thumbnail; a new screen of the grid waits ~1–5 s (more under load). Build the grid thumbnails
   at import / in the background (`library.buildPreviews {size: thumbs}`) for whole shoots.
2. **Per change, still on the UI thread** (flag / star, ~5–10 ms on an idle machine): the visible
   list is re-filtered and re-sorted (`Session::refresh_visible_base`, 15,865 photos), the Shoots
   list and folder tree are rebuilt (`cmd::shoots::list`, `folder_tree_with`), the native menu is
   rebuilt each frame (`NativeMenu::update`). Cache them by what they depend on, not the revision.
3. **1:1 and export render time**: the full-resolution raw source takes 1–4 s at modest load (4–17 s
   at load 600) before the GPU stages; export misses its 1 s / 24 MP budget. Render-side work.
4. **CPU copies of every texture when the control channel is on** (`keep_pixels`, for headless
   screenshots): the app Bryan runs with `--control 7980` keeps ~180 MB of duplicate pixels for
   216 textures. Keep them only for the loupe slot, or render headless screenshots from the cache.
5. **Memory peaks at 5.5 GB footprint** after develop drags and 1:1 on three raws (GPU stage
   caches); it trims back when idle.
