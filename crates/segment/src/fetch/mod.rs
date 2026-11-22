//! Downloading the SAM 3 model, only when the user asked for it. The downloader itself (mirrors,
//! resuming, checks, the pure-Rust HTTPS client) is the `lightcraft-fetch` crate; this module
//! says what SAM 3 needs and where it may come from.
//!
//! The weights are never part of LightCraft (SAM License, see docs/ai-masks.md); where they are
//! downloaded from is configured by [`DEFAULT_MIRRORS`] and the user's own list.

pub use lightcraft_fetch::*;

use std::path::Path;

/// The `facebook/sam3` checkpoint files [`crate::Sam3::load`] needs.
///
/// Every file is pinned (size + SHA-256) to Meta's official `facebook/sam3` checkpoint
/// (`model.safetensors`: its Hugging Face LFS SHA-256; the tokenizer files: hashed from copies
/// whose git blob ids equal the official ones), so a mirror can only ever serve Meta's files.
pub const SAM3_FILES: &[FileSpec] = &[
    FileSpec {
        name: "vocab.json",
        size: Some(862_328),
        sha256: Some("5047b556ce86ccaf6aa22b3ffccfc52d391ea4accdab9c2f2407da5b742d4363"),
        max: 862_328,
    },
    FileSpec {
        name: "merges.txt",
        size: Some(524_619),
        sha256: Some("9fd691f7c8039210e0fced15865466c65820d09b63988b0174bfe25de299051a"),
        max: 524_619,
    },
    FileSpec {
        name: crate::WEIGHTS_FILE,
        size: Some(SAM3_WEIGHTS_SIZE),
        sha256: Some("6d06f0a5f84e435071fe6603e61d0b4cc7b40e0d39d487cfd4d67d8cc11cc14a"),
        max: SAM3_WEIGHTS_SIZE,
    },
];

/// Size of the official `model.safetensors` (3.44 GB).
pub const SAM3_WEIGHTS_SIZE: u64 = 3_439_938_512;

/// Where LightCraft downloads the model from, in order: each is a base URL, and file `f` is at
/// `<base>/<f>`. Hugging Face's own `facebook/sam3` is gated (each user must accept the SAM
/// License there and download with their own token), so the defaults are public, ungated copies
/// of it on Hugging Face (the SAM License lets anyone redistribute the materials along with the
/// licence), each pinned to a commit. Their files are checked against Meta's hashes in
/// [`SAM3_FILES`]: a copy that changed is never used, the next one is tried. Checked reachable
/// 2026-11-22 (`cargo test -p lightcraft-segment -- --ignored mirrors`).
pub const DEFAULT_MIRRORS: &[&str] = &[
    "https://huggingface.co/jetjodh/sam3/resolve/1aa50ce07302cb375f85d8084b68a0fb378b8d85",
    "https://huggingface.co/bodhicitta/sam3/resolve/cba430d22f6fdc3f06ad3841274ec7bb55885f2f",
    "https://huggingface.co/MIUProject/sam3/resolve/0766c7228f92f0a4514b3d8d3c32ee40a856d71e",
    "https://huggingface.co/JobHarvest/sam3/resolve/90ab711c3b98c6d012c9ebf31f2b575b47b614bc",
    "https://huggingface.co/DiffusionWave/sam3/resolve/480147c4f3cf808f763e6d44f762e71616ea1cec",
];

/// Environment variable with extra mirrors (base URLs separated by commas, spaces or newlines),
/// tried before the defaults.
pub const MIRRORS_ENV: &str = "LIGHTCRAFT_SAM3_MIRRORS";

/// The mirrors to try for SAM 3, in order: the environment variable's, then the mirrors file's
/// (one base URL per line, `#` comments), then [`DEFAULT_MIRRORS`].
pub fn mirrors(env: Option<&str>, file: Option<&Path>) -> Vec<String> {
    lightcraft_fetch::mirrors(env, file, DEFAULT_MIRRORS)
}

/// [`mirrors`] with other defaults (none: only the user's).
pub fn mirrors_with(env: Option<&str>, file: Option<&Path>, defaults: &[&str]) -> Vec<String> {
    lightcraft_fetch::mirrors(env, file, defaults)
}

/// Download settings for SAM 3: the defaults, plus [`TOKEN_ENV`] for a gated mirror.
pub fn options() -> Options {
    Options { token_env: Some(TOKEN_ENV), ..Options::default() }
}

/// Environment variable with a bearer token for the SAM 3 mirror's own host (a gated Hugging Face repository).
pub const TOKEN_ENV: &str = "LIGHTCRAFT_SAM3_TOKEN";

/// What to tell the user when there is no mirror for the SAM 3 model.
pub fn no_mirrors_message() -> String {
    format!(
        "no download location is configured for the SAM 3 model in this build (set {MIRRORS_ENV}, or put the files in the model folder yourself; see docs/ai-masks.md)"
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sam3_mirrors_come_from_the_environment_the_file_and_the_defaults() {
        let m = mirrors(Some("https://a.example/x/"), Some(Path::new("/nonexistent/mirrors.txt")));
        assert_eq!(m.len(), 1 + DEFAULT_MIRRORS.len());
        assert!(no_mirrors_message().contains(MIRRORS_ENV));
        assert!(!DEFAULT_MIRRORS.is_empty() && DEFAULT_MIRRORS.iter().all(|m| m.starts_with("https://")));
        assert!(SAM3_FILES.iter().all(|f| f.size.is_some() && f.sha256.is_some()), "every file pinned");
    }

    /// Network check (run by hand): each default mirror serves the tokenizer files with Meta's
    /// hashes.
    #[test]
    #[ignore = "network"]
    fn mirrors_are_reachable() {
        let tok: Vec<FileSpec> = SAM3_FILES.iter().filter(|f| f.name != crate::WEIGHTS_FILE).cloned().collect();
        for (i, m) in DEFAULT_MIRRORS.iter().enumerate() {
            let dir = std::env::temp_dir().join(format!("lc-sam3-mirror-{}-{i}", std::process::id()));
            let _ = std::fs::remove_dir_all(&dir);
            std::fs::create_dir_all(&dir).unwrap();
            let cancel = std::sync::atomic::AtomicBool::new(false);
            download(&tok, &[m.to_string()], &dir, &options(), &cancel, &mut |_| {}).unwrap_or_else(|e| panic!("{m}: {e}"));
            assert!(dir.join("vocab.json").is_file() && dir.join("merges.txt").is_file(), "{m}");
            let _ = std::fs::remove_dir_all(&dir);
        }
    }
}
