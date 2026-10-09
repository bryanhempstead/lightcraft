//! Builds Adobe's DNG SDK (C++, from `vendor/dng_sdk_1_7_1`, see `tools/fetch-dng-sdk.sh`) plus its
//! bundled libjpeg and LightCraft's C ABI shim into one static library. When the SDK isn't there,
//! or the target isn't a native desktop build, nothing is compiled and the crate's API reports
//! "unavailable" (`cfg(lc_dng_sdk)` unset), so the rest of LightCraft builds and runs as before.

use std::path::PathBuf;

fn main() {
    println!("cargo::rustc-check-cfg=cfg(lc_dng_sdk)");
    println!("cargo:rerun-if-env-changed=LIGHTCRAFT_DNG_SDK");
    println!("cargo:rerun-if-changed=shim");
    let manifest = PathBuf::from(std::env::var("CARGO_MANIFEST_DIR").unwrap_or_default());
    let sdk = std::env::var_os("LIGHTCRAFT_DNG_SDK").map(PathBuf::from).unwrap_or_else(|| manifest.join("../../vendor/dng_sdk_1_7_1"));
    let target = std::env::var("TARGET").unwrap_or_default();
    let os = std::env::var("CARGO_CFG_TARGET_OS").unwrap_or_default();
    let source = sdk.join("dng_sdk/source");
    println!("cargo:rerun-if-changed={}", source.display());
    if target.contains("wasm") || !(os == "macos" || os == "linux") {
        return;
    }
    if !source.join("dng_color_spec.cpp").is_file() {
        println!("cargo:warning=Adobe DNG SDK not found at {} (run tools/fetch-dng-sdk.sh): building without it", sdk.display());
        return;
    }
    let out = PathBuf::from(std::env::var("OUT_DIR").unwrap_or_default());
    // libjxl's generated export header (JPEG XL itself is stubbed out: shim/jxl_stubs.cpp)
    let gen_dir = out.join("gen");
    let _ = std::fs::create_dir_all(gen_dir.join("jxl"));
    let export = "#pragma once\n#define JXL_EXPORT\n#define JXL_DEPRECATED\n#define JXL_THREADS_EXPORT\n";
    for name in ["jxl_export.h", "jxl_threads_export.h"] {
        let _ = std::fs::write(gen_dir.join("jxl").join(name), export);
    }

    let jpeg = sdk.join("libjpeg");
    let jpeg_files = [
        "jaricom", "jcapimin", "jcapistd", "jcarith", "jccoefct", "jccolor", "jcdctmgr", "jchuff", "jcinit", "jcmainct", "jcmarker", "jcmaster",
        "jcomapi", "jcparam", "jcprepct", "jcsample", "jctrans", "jdapimin", "jdapistd", "jdarith", "jdatadst", "jdatasrc", "jdcoefct", "jdcolor",
        "jddctmgr", "jdhuff", "jdinput", "jdmainct", "jdmarker", "jdmaster", "jdmerge", "jdpostct", "jdsample", "jdtrans", "jerror", "jfdctflt",
        "jfdctfst", "jfdctint", "jidctflt", "jidctfst", "jidctint", "jquant1", "jquant2", "jutils", "jmemmgr", "jmemnobs",
    ];
    let mut c = cc::Build::new();
    c.warnings(false).flag_if_supported("-w").opt_level(2).include(&jpeg);
    for f in jpeg_files {
        c.file(jpeg.join(format!("{f}.c")));
    }
    c.compile("lcdngjpeg");

    let mut cpp = cc::Build::new();
    cpp.cpp(true)
        .std("c++17")
        .warnings(false)
        .flag_if_supported("-w")
        .opt_level(2)
        .define(if os == "macos" { "qMacOS" } else { "qLinux" }, "1")
        .define("qDNGUseXMP", "0")
        .define("qDNGXMPFiles", "0")
        .define("qDNGXMPDocOps", "0")
        .define("qDNGUseLibJPEG", "1")
        .define("qDNGValidateTarget", "0")
        .include(&gen_dir)
        .include(sdk.join("libjxl/libjxl/lib/include"))
        .include(&jpeg)
        .include(&source)
        .include(manifest.join("shim"));
    let skip = ["dng_validate.cpp", "dng_jxl.cpp", "dng_xmp_sdk.cpp", "dng_update_meta.cpp"];
    let mut files: Vec<PathBuf> = std::fs::read_dir(&source).map(|rd| rd.filter_map(|e| e.ok().map(|e| e.path())).collect()).unwrap_or_default();
    files.retain(|p| p.extension().is_some_and(|e| e == "cpp") && !skip.iter().any(|s| p.file_name().is_some_and(|n| n == *s)));
    files.sort();
    for f in &files {
        cpp.file(f);
    }
    cpp.file(manifest.join("shim/lc_dng.cpp"));
    cpp.file(manifest.join("shim/jxl_stubs.cpp"));
    cpp.compile("lcdngsdk");
    println!("cargo:rustc-link-lib=z");
    if os == "macos" {
        println!("cargo:rustc-link-lib=framework=CoreFoundation");
        println!("cargo:rustc-link-lib=framework=CoreServices");
    }
    println!("cargo:rustc-cfg=lc_dng_sdk");
}
