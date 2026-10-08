# macOS system-decoder fallback (this fork only)

> **Fork-only.** This is in Bryan's fork (`bryanhempstead/lightcraft`), not upstream
> `storytold/lightcraft`. It does not link any C library: it runs `/usr/bin/sips`, the image tool that
> ships with macOS, as a separate process. Upstream's "pure Rust, no C" rule is about what is compiled into
> LightCraft; this fallback keeps to that, but it only works on macOS.

## What it does

When the pure-Rust decoders can't read a file, LightCraft asks macOS's own image engine (ImageIO, through
`sips`) to convert it to a TIFF, then decodes that TIFF with the normal TIFF path.

| Goes through macOS | Why |
|---|---|
| HEIC / HEIF / HIF (iPhone, Canon HEIF), AVIF | no permissively licensed pure-Rust HEVC / AV1 decoder yet |
| JPEG 2000 (`jp2`, `j2k`, `jpf`, `jpx`), OpenEXR, TGA, DDS, SGI, PBM/PGM/PPM, PSB | no decoder in `lightcraft-codecs` |
| Raws the raw crate recognises but can't decode (`probe_info` refuses the variant: compressed ORF, unverified CR3 variants, Canon sRAW, …) | Apple's RAW engine renders them |
| Raws the raw crate doesn't know (`crw`, `mrw`, `3fr`, `fff`, `iiq`, `mos`, `erf`, `dcr`, `kdc`, `srw`, `srf`, `sr2`, …) | same |
| A raw whose sensor decode fails after its headers read fine (a damaged stream, CR3 C-RAW clipping issue #396) | tried as a last resort instead of an error |

Everything the native decoders handle (JPEG, PNG, TIFF, WebP, JXL, PSD, GIF, BMP, DNG, CR2, CR3, ARW, NEF,
RAF, RW2, PEF, uncompressed ORF) never goes through macOS.

Code: `crates/engine/src/files/sysdecode.rs` (`route`, `convert`, `probe`, `load`), wired into the
filesystem hooks in `crates/engine/src/files.rs` (`fs_hooks`), so import, thumbnails, the loupe, smart
previews and export all use it. Import accepts the extra extensions on macOS
(`import::is_supported` / `import::extensions`).

## The cache

Conversions are TIFFs named by a hash of the file's **path + size + modification time** (a changed file
converts again). They live in the open library's `System Decodes/` folder, or
`~/Library/Caches/LightCraft/System Decodes/` without a library (`LIGHTCRAFT_SYSDECODE_CACHE` overrides
both). The cache is kept under 4 GB by removing the least recently used conversions. Originals are only
read; nothing is ever written next to them. Deleting the folder is always safe.

`LIGHTCRAFT_NO_MACOS_DECODE=1` turns the fallback off (the files then fail to import, or raws fall back to
their embedded JPEG, as upstream).

## How such photos edit

A photo decoded by macOS is marked through `Photo::preview_only` with a reason that starts
`decoded by macOS`; the Edit and Info panels show a **Decoded by macOS** notice, and `catalog.query` reports
it as `previewOnly`. Develop treats it as a **rendered (display-referred) source**, like a TIFF or JPEG:

- White balance is relative (Temp / Tint shift the rendered colours), not an as-shot Kelvin re-balance of
  sensor data.
- Exposure and Highlights work on what macOS rendered: highlights macOS clipped are gone. A raw rendered by
  Apple keeps 16 bits, so there is more room than in a camera JPEG, but less than a real raw decode.
- Apple's default raw look (its tone curve, noise reduction, sharpening and lens corrections) is baked in.
  LightCraft's raw-only defaults (camera tone, embedded lens opcodes) don't apply.

For HEIC/AVIF there is no difference: those are rendered files to begin with.

## Failures

A file macOS can't read either (truncated, not an image) fails import with a per-file error naming sips's
message; the rest of the import continues. `sips` gets 120 s per file before it is stopped.

## Retrying files an import failed on

Every import into a library on disk keeps the files it couldn't read in `import-failures.json` (path,
error, time, mode). `library.importFailures` lists them; `library.retryFailedImports` imports them again
(each in its import's mode; files on unplugged drives stay listed; `forget: true` clears the list):

```sh
lightcraft-cli run --library "$HOME/Pictures/LightCraft Library" library.retryFailedImports
```

Files from a Lightroom Classic migration that failed before this fallback existed (they were never listed)
come in with the migration's `--only-new`, which imports what's missing and applies Lightroom's ratings,
keywords, develop settings and collections only to the photos it adds:

```sh
lightcraft-cli migrate-lightroom --only-new --no-presets \
  --library "$HOME/Pictures/LightCraft Library" \
  --catalog "$HOME/Pictures/Lightroom/LR-Cat2.lrcat-v13-3.lrcat"
```

Both need the library closed in the app (it holds the library's lock), or run the same commands from the
app's control channel.
