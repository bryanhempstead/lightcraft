//! Builds LibRaw (C++, from `vendor/LibRaw-0.22.3`, see `tools/fetch-libraw.sh`) plus LightCraft's
//! C ABI shim into one static library. When LibRaw isn't there, or the target isn't a native
//! desktop build, nothing is compiled and the crate's API reports "unavailable" (`cfg(lc_libraw)`
//! unset), so the rest of LightCraft builds and runs as before. `LIGHTCRAFT_NO_LIBRAW=1` builds
//! without it on purpose; `LIGHTCRAFT_LIBRAW=DIR` points at another LibRaw source tree.
//!
//! Options: zlib (deflate DNGs; ships with macOS and every Linux) on; libjpeg (lossy DNGs, which
//! LightCraft decodes itself), RawSpeed, the DNG SDK glue, X3F tools and OpenMP off.

use std::path::PathBuf;

/// LibRaw's own source list (`Makefile.am` → `lib_libraw_a_SOURCES`).
const FILES: &[&str] = &[
    "libraw_c_api",
    "libraw_datastream",
    "decoders/canon_600",
    "decoders/crx",
    "decoders/pana8",
    "decoders/decoders_dcraw",
    "decoders/sonycc",
    "decompressors/losslessjpeg",
    "decoders/decoders_libraw_dcrdefs",
    "decoders/olympus14",
    "decoders/decoders_libraw",
    "decoders/dng",
    "decoders/fp_dng",
    "decoders/fuji_compressed",
    "decoders/generic",
    "decoders/kodak_decoders",
    "decoders/load_mfbacks",
    "decoders/smal",
    "decoders/unpack_thumb",
    "decoders/unpack",
    "demosaic/aahd_demosaic",
    "demosaic/ahd_demosaic",
    "demosaic/dcb_demosaic",
    "demosaic/dht_demosaic",
    "demosaic/misc_demosaic",
    "demosaic/xtrans_demosaic",
    "integration/dngsdk_glue",
    "integration/rawspeed_glue",
    "metadata/adobepano",
    "metadata/canon",
    "metadata/ciff",
    "metadata/cr3_parser",
    "metadata/epson",
    "metadata/exif_gps",
    "metadata/fuji",
    "metadata/identify_tools",
    "metadata/identify",
    "metadata/kodak",
    "metadata/leica",
    "metadata/makernotes",
    "metadata/mediumformat",
    "metadata/minolta",
    "metadata/misc_parsers",
    "metadata/nikon",
    "metadata/normalize_model",
    "metadata/olympus",
    "metadata/hasselblad_model",
    "metadata/p1",
    "metadata/pentax",
    "metadata/samsung",
    "metadata/sony",
    "metadata/tiff",
    "postprocessing/aspect_ratio",
    "postprocessing/dcraw_process",
    "postprocessing/mem_image",
    "postprocessing/postprocessing_aux",
    "postprocessing/postprocessing_utils_dcrdefs",
    "postprocessing/postprocessing_utils",
    "preprocessing/ext_preprocess",
    "preprocessing/raw2image",
    "preprocessing/subtract_black",
    "tables/cameralist",
    "tables/colorconst",
    "tables/colordata",
    "tables/wblists",
    "utils/curves",
    "utils/decoder_info",
    "utils/init_close_utils",
    "utils/open",
    "utils/phaseone_processing",
    "utils/read_utils",
    "utils/thumb_utils",
    "utils/utils_dcraw",
    "utils/utils_libraw",
    "write/apply_profile",
    "write/file_write",
    "write/tiff_writer",
    "x3f/x3f_parse_process",
    "x3f/x3f_utils_patched",
];

fn main() {
    println!("cargo::rustc-check-cfg=cfg(lc_libraw)");
    println!("cargo:rerun-if-env-changed=LIGHTCRAFT_LIBRAW");
    println!("cargo:rerun-if-env-changed=LIGHTCRAFT_NO_LIBRAW");
    println!("cargo:rerun-if-changed=shim");
    let manifest = PathBuf::from(std::env::var("CARGO_MANIFEST_DIR").unwrap_or_default());
    let root = std::env::var_os("LIGHTCRAFT_LIBRAW").map(PathBuf::from).unwrap_or_else(|| manifest.join("../../vendor/LibRaw-0.22.3"));
    let target = std::env::var("TARGET").unwrap_or_default();
    let os = std::env::var("CARGO_CFG_TARGET_OS").unwrap_or_default();
    let src = root.join("src");
    println!("cargo:rerun-if-changed={}", src.display());
    if target.contains("wasm") || !(os == "macos" || os == "linux") || std::env::var_os("LIGHTCRAFT_NO_LIBRAW").is_some() {
        return;
    }
    if !src.join("libraw_c_api.cpp").is_file() {
        println!("cargo:warning=LibRaw not found at {} (run tools/fetch-libraw.sh): building without it", root.display());
        return;
    }
    let mut cpp = cc::Build::new();
    cpp.cpp(true)
        .std("c++11")
        .warnings(false)
        .flag_if_supported("-w")
        .flag_if_supported("-pthread")
        .opt_level(2)
        .define("USE_ZLIB", "1")
        .define("LIBRAW_NODLL", "1")
        .include(&root)
        .include(manifest.join("shim"));
    for f in FILES {
        cpp.file(src.join(format!("{f}.cpp")));
    }
    cpp.file(manifest.join("shim/lc_libraw.cpp"));
    cpp.compile("lclibraw");
    println!("cargo:rustc-link-lib=z");
    println!("cargo:rustc-cfg=lc_libraw");
}
