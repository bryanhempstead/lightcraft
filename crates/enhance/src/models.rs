//! The open models Enhance uses: what each one is, its licence, and where it is downloaded from.
//!
//! None is part of LightCraft: each is downloaded only when the user asks (and has seen its licence), into
//! `<models dir>/<id>/`, size- and SHA-256-checked by `lightcraft-fetch` before it is used.

use std::path::{Path, PathBuf};

/// One Enhance model.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum ModelId {
    /// RawNIND UtNet2: joint denoise + demosaic of the Bayer mosaic (darktable-ai's `rawdenoise-nind`).
    RawDenoise,
    /// Real-ESRGAN x2plus (RRDBNet): 2× super resolution.
    SuperResolution,
    /// Depth Anything V2 Small: relative depth for Lens Blur and Depth Range masks.
    Depth,
}

pub struct ModelInfo {
    pub id: ModelId,
    /// Folder and command name.
    pub key: &'static str,
    pub name: &'static str,
    pub licence: &'static str,
    pub licence_url: &'static str,
    /// Where the model is published (for the user).
    pub source: &'static str,
    /// The exact file to download.
    pub url: &'static str,
    /// The downloaded file's name, size and SHA-256 (pinned).
    pub file: &'static str,
    pub size: u64,
    pub sha256: &'static str,
    /// The file the model is read from once installed (an archive's entry is extracted to it).
    pub weights: &'static str,
    /// The archive entry to extract into `weights` (`None`: the download is the weights).
    pub entry: Option<&'static str>,
}

pub const MODELS: [ModelInfo; 3] = [
    ModelInfo {
        id: ModelId::RawDenoise,
        key: "raw-denoise",
        name: "RawNIND UtNet2 (Bayer) — darktable AI raw denoise",
        licence: "GPL-3.0",
        licence_url: "https://www.gnu.org/licenses/gpl-3.0.html",
        source: "https://github.com/darktable-org/darktable-ai/tree/master/models/rawdenoise-nind",
        url: "https://github.com/darktable-org/darktable-ai/releases/download/release-5.6.0/rawdenoise-nind.dtmodel",
        file: "rawdenoise-nind.dtmodel",
        size: 57_700_134,
        sha256: "d71b5f1e727c85a359e6f74dca9e2016c9d8fc3e2f7ac3e9b347d80ceca969af",
        weights: "model_bayer.onnx",
        entry: Some("model_bayer.onnx"),
    },
    ModelInfo {
        id: ModelId::SuperResolution,
        key: "super-resolution",
        name: "Real-ESRGAN x2plus (RRDBNet)",
        licence: "BSD-3-Clause",
        licence_url: "https://github.com/xinntao/Real-ESRGAN/blob/master/LICENSE",
        source: "https://github.com/xinntao/Real-ESRGAN",
        url: "https://github.com/xinntao/Real-ESRGAN/releases/download/v0.2.1/RealESRGAN_x2plus.pth",
        file: "RealESRGAN_x2plus.pth",
        size: 67_061_725,
        sha256: "49fafd45f8fd7aa8d31ab2a22d14d91b536c34494a5cfe31eb5d89c2fa266abb",
        weights: "RealESRGAN_x2plus.pth",
        entry: None,
    },
    ModelInfo {
        id: ModelId::Depth,
        key: "depth",
        name: "Depth Anything V2 Small",
        licence: "Apache-2.0",
        licence_url: "https://huggingface.co/depth-anything/Depth-Anything-V2-Small/blob/main/LICENSE",
        source: "https://github.com/DepthAnything/Depth-Anything-V2",
        url: "https://huggingface.co/jeroenvlek/depth-anything-v2-safetensors/resolve/main/depth_anything_v2_vits.safetensors",
        file: "depth_anything_v2_vits.safetensors",
        size: 99_165_428,
        sha256: "07635472183d063a80e6b2d78645ab944a9210fd973b1c88e4b7e3ba23981f75",
        weights: "depth_anything_v2_vits.safetensors",
        entry: None,
    },
];

pub fn info(id: ModelId) -> &'static ModelInfo {
    match id {
        ModelId::RawDenoise => &MODELS[0],
        ModelId::SuperResolution => &MODELS[1],
        ModelId::Depth => &MODELS[2],
    }
}

pub fn by_key(key: &str) -> Option<ModelId> {
    MODELS.iter().find(|m| m.key == key).map(|m| m.id)
}

/// The folder of `id` under `root`.
pub fn dir(root: &Path, id: ModelId) -> PathBuf {
    root.join(info(id).key)
}

/// The installed weights of `id` under `root`, when present.
pub fn installed(root: &Path, id: ModelId) -> Option<PathBuf> {
    let p = dir(root, id).join(info(id).weights);
    p.is_file().then_some(p)
}

/// What to tell the user when `id` is missing.
pub fn missing_message(root: &Path, id: ModelId) -> String {
    let m = info(id);
    format!(
        "the {} model is not installed. Download it ({:.0} MB, {}) when LightCraft offers it, or run `enhance.model.download {{\"model\": \"{}\", \"acknowledged\": true}}`, or put {} in {}",
        m.name,
        m.size as f64 / 1e6,
        m.licence,
        m.key,
        m.weights,
        dir(root, id).display()
    )
}

#[cfg(not(target_arch = "wasm32"))]
/// Download `id` into `root` (blocking; run on a background thread), then extract the archive entry.
pub fn download(
    root: &Path,
    id: ModelId,
    cancel: &std::sync::atomic::AtomicBool,
    progress: &mut dyn FnMut(&lightcraft_fetch::Progress),
) -> Result<PathBuf, String> {
    let m = info(id);
    let d = dir(root, id);
    let spec = lightcraft_fetch::FileSpec { name: m.file, size: Some(m.size), sha256: Some(m.sha256), max: m.size };
    lightcraft_fetch::download_url(&spec, m.url, &d, &lightcraft_fetch::Options::default(), cancel, progress).map_err(|e| e.to_string())?;
    let weights = d.join(m.weights);
    if let Some(entry) = m.entry {
        if !weights.is_file() {
            let part = d.join(format!("{}.part", m.weights));
            lightcraft_denoise_core::archive::extract(&d.join(m.file), entry, &part, 512 << 20).map_err(|e| format!("{}: {e}", m.file))?;
            std::fs::rename(&part, &weights).map_err(|e| format!("{}: {e}", weights.display()))?;
        }
        // the archive isn't needed once its model is out
        let _ = std::fs::remove_file(d.join(m.file));
    }
    Ok(weights)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_model_is_pinned_and_found_by_key() {
        for m in &MODELS {
            assert_eq!(m.sha256.len(), 64);
            assert!(m.url.starts_with("https://"));
            assert_eq!(by_key(m.key), Some(m.id));
            assert_eq!(info(m.id).key, m.key);
        }
        let root = std::env::temp_dir().join(format!("lc-enhance-models-{}", std::process::id()));
        assert!(installed(&root, ModelId::Depth).is_none());
        assert!(missing_message(&root, ModelId::Depth).contains("enhance.model.download"));
    }
}
