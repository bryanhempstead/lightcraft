//! The open models Enhance (and Generative Remove) use: what each one is, its licence, and where it is downloaded from.
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
    /// Real-ESRGAN general x4v3 (SRVGGNetCompact), box-averaged to 2×: Super Resolution.
    SuperResolution,
    /// Depth Anything V2 Small: relative depth for Lens Blur and Depth Range masks.
    Depth,
    /// LaMa big-lama: Generative Remove's inpainting.
    Inpaint,
    /// DSRNet: Remove Reflections (with [`ModelId::Vgg19`]).
    Reflection,
    /// VGG-19's ImageNet weights: the features DSRNet is built on.
    Vgg19,
    /// Stable Diffusion 1.5 Inpainting: Generative Remove's diffusion fill (`gen.ai.`; LaMa stays the quick one).
    GenFill,
}

/// Another file a model needs beside its main one (downloaded with it, into the same folder).
pub struct Part {
    pub url: &'static str,
    pub file: &'static str,
    pub size: u64,
    pub sha256: &'static str,
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
    /// More files the model needs (each pinned like the main one).
    pub parts: &'static [Part],
}

impl ModelInfo {
    /// Bytes to download, all files.
    pub fn bytes(&self) -> u64 {
        self.parts.iter().fold(self.size, |a, p| a.saturating_add(p.size))
    }
}

pub const MODELS: [ModelInfo; 7] = [
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
        parts: &[],
    },
    ModelInfo {
        id: ModelId::SuperResolution,
        key: "super-resolution",
        name: "Real-ESRGAN general x4v3 (SRVGGNetCompact)",
        licence: "BSD-3-Clause",
        licence_url: "https://github.com/xinntao/Real-ESRGAN/blob/master/LICENSE",
        source: "https://github.com/xinntao/Real-ESRGAN",
        url: "https://github.com/xinntao/Real-ESRGAN/releases/download/v0.2.5.0/realesr-general-x4v3.pth",
        file: "realesr-general-x4v3.pth",
        size: 4_885_111,
        sha256: "8dc7edb9ac80ccdc30c3a5dca6616509367f05fbc184ad95b731f05bece96292",
        weights: "realesr-general-x4v3.pth",
        entry: None,
        parts: &[],
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
        parts: &[],
    },
    ModelInfo {
        id: ModelId::Inpaint,
        key: "inpaint",
        name: "LaMa big-lama (Generative Remove)",
        licence: "Apache-2.0",
        licence_url: "https://github.com/advimman/lama/blob/main/LICENSE",
        source: "https://github.com/advimman/lama",
        // the generator of the published big-lama checkpoint as safetensors (Apache-2.0 mirror, pinned commit)
        url: "https://huggingface.co/AEmotionStudio/lama-inpainting/resolve/d194c0932b1b022f0e34b9e574dc25e24db27c27/big-lama.safetensors",
        file: "big-lama.safetensors",
        size: 204_348_820,
        sha256: "12cb5eb5d515400f262b48d4e7c1994183e170aedb4b86699d9a55b770038e16",
        weights: "big-lama.safetensors",
        entry: None,
        parts: &[],
    },
    ModelInfo {
        id: ModelId::Reflection,
        key: "reflection",
        name: "DSRNet (Remove Reflections)",
        licence: "Apache-2.0",
        licence_url: "https://github.com/mingcv/DSRNet/blob/main/LICENSE",
        source: "https://github.com/mingcv/DSRNet",
        // the authors' released checkpoint (setting II, with the Nature data), from their Google Drive folder
        url: "https://drive.usercontent.google.com/download?id=19His-4uV0C_fteSOI_mBL8vyQn_Fdh_8&export=download&confirm=t",
        file: "dsrnet_l_4000_epoch33.pt",
        size: 495_086_231,
        sha256: "09a7acdb3efef6ad7065164e205bf5173a016ce8b24fc56d9751c8791cf65cff",
        weights: "dsrnet_l_4000_epoch33.pt",
        entry: None,
        parts: &[],
    },
    ModelInfo {
        id: ModelId::Vgg19,
        key: "vgg19",
        name: "VGG-19 ImageNet features (for Remove Reflections)",
        licence: "BSD-3-Clause",
        licence_url: "https://github.com/pytorch/vision/blob/main/LICENSE",
        source: "https://huggingface.co/timm/vgg19.tv_in1k",
        url: "https://huggingface.co/timm/vgg19.tv_in1k/resolve/6ee3e2c532177d3b28c99c0eb12b069f160879c7/model.safetensors",
        file: "model.safetensors",
        size: 574_672_302,
        sha256: "11aea017b543ff59233749d8f1cf544cda749378f5790d2fedab841fb86b7669",
        weights: "model.safetensors",
        entry: None,
        parts: &[],
    },
    ModelInfo {
        id: ModelId::GenFill,
        key: "gen-fill",
        name: "Stable Diffusion 1.5 Inpainting (Generative Remove, gen.ai.)",
        licence: "CreativeML OpenRAIL-M",
        licence_url: "https://huggingface.co/stable-diffusion-v1-5/stable-diffusion-inpainting/blob/main/LICENSE.md",
        source: "https://huggingface.co/stable-diffusion-v1-5/stable-diffusion-inpainting",
        // RunwayML's weights as re-uploaded by the stable-diffusion-v1-5 organisation, half precision, pinned
        // commit: the UNet (the main file), the VAE and the CLIP text encoder
        url: "https://huggingface.co/stable-diffusion-v1-5/stable-diffusion-inpainting/resolve/8a4288a76071f7280aedbdb3253bdb9e9d5d84bb/unet/diffusion_pytorch_model.fp16.safetensors",
        file: "unet.safetensors",
        size: 1_719_154_104,
        sha256: "24b788b4a777748377cc20364eea4ae113c8c42f4468c16bc8c02fdae5492af9",
        weights: "unet.safetensors",
        entry: None,
        parts: &[
            Part {
                url: "https://huggingface.co/stable-diffusion-v1-5/stable-diffusion-inpainting/resolve/8a4288a76071f7280aedbdb3253bdb9e9d5d84bb/vae/diffusion_pytorch_model.fp16.safetensors",
                file: "vae.safetensors",
                size: 167_335_342,
                sha256: "4fbcf0ebe55a0984f5a5e00d8c4521d52359af7229bb4d81890039d2aa16dd7c",
            },
            Part {
                url: "https://huggingface.co/stable-diffusion-v1-5/stable-diffusion-inpainting/resolve/8a4288a76071f7280aedbdb3253bdb9e9d5d84bb/text_encoder/model.fp16.safetensors",
                file: "text_encoder.safetensors",
                size: 246_144_864,
                sha256: "77795e2023adcf39bc29a884661950380bd093cf0750a966d473d1718dc9ef4e",
            },
        ],
    },
];

pub fn info(id: ModelId) -> &'static ModelInfo {
    match id {
        ModelId::RawDenoise => &MODELS[0],
        ModelId::SuperResolution => &MODELS[1],
        ModelId::Depth => &MODELS[2],
        ModelId::Inpaint => &MODELS[3],
        ModelId::Reflection => &MODELS[4],
        ModelId::Vgg19 => &MODELS[5],
        ModelId::GenFill => &MODELS[6],
    }
}

pub fn by_key(key: &str) -> Option<ModelId> {
    MODELS.iter().find(|m| m.key == key).map(|m| m.id)
}

/// The folder of `id` under `root`.
pub fn dir(root: &Path, id: ModelId) -> PathBuf {
    root.join(info(id).key)
}

/// The installed weights of `id` under `root`, when present (with every other file it needs).
pub fn installed(root: &Path, id: ModelId) -> Option<PathBuf> {
    let d = dir(root, id);
    let p = d.join(info(id).weights);
    (p.is_file() && info(id).parts.iter().all(|part| d.join(part.file).is_file())).then_some(p)
}

/// What to tell the user when `id` is missing.
pub fn missing_message(root: &Path, id: ModelId) -> String {
    let m = info(id);
    format!(
        "the {} model is not installed. Download it ({:.0} MB, {}) when LightCraft offers it, or run `enhance.model.download {{\"model\": \"{}\", \"acknowledged\": true}}`, or put {} in {}",
        m.name,
        m.bytes() as f64 / 1e6,
        m.licence,
        m.key,
        m.weights,
        dir(root, id).display()
    )
}

#[cfg(feature = "models")]
/// Download `id` into `root` (blocking; run on a background thread), then extract the archive entry.
pub fn download(
    root: &Path,
    id: ModelId,
    cancel: &std::sync::atomic::AtomicBool,
    progress: &mut dyn FnMut(&lightcraft_fetch::Progress),
) -> Result<PathBuf, String> {
    let m = info(id);
    let d = dir(root, id);
    // the other files first (small), the main one last: `installed` then means all are there
    let mut before = 0u64;
    let total = m.bytes();
    for (url, file, size, sha256) in m.parts.iter().map(|p| (p.url, p.file, p.size, p.sha256)).chain([(m.url, m.file, m.size, m.sha256)]) {
        let spec = lightcraft_fetch::FileSpec { name: file, size: Some(size), sha256: Some(sha256), max: size };
        let mut each = |p: &lightcraft_fetch::Progress| {
            let q = lightcraft_fetch::Progress { file: p.file.clone(), done: before.saturating_add(p.done), total, mirror: p.mirror.clone() };
            progress(&q);
        };
        lightcraft_fetch::download_url(&spec, url, &d, &lightcraft_fetch::Options::default(), cancel, &mut each).map_err(|e| e.to_string())?;
        before = before.saturating_add(size);
    }
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
            for p in m.parts {
                assert!(p.sha256.len() == 64 && p.url.starts_with("https://") && p.file != m.file);
            }
        }
        assert!(info(ModelId::GenFill).bytes() > info(ModelId::GenFill).size);
        let root = std::env::temp_dir().join(format!("lc-enhance-models-{}", std::process::id()));
        assert!(installed(&root, ModelId::Depth).is_none());
        assert!(missing_message(&root, ModelId::Depth).contains("enhance.model.download"));
    }
}
