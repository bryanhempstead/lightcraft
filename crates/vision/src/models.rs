//! The open models People, Assisted Culling and AI search use: what each one is, its licence, and where it is
//! downloaded from. None is part of LightCraft: each is downloaded only when the user asks (having seen its size and
//! licence) into `<models dir>/<package>/`, size- and SHA-256-checked before it is used.
//!
//! Two packages: **faces** (YuNet + SFace + the eye-state network, 39 MB, for People and culling) and **search**
//! (CLIP ViT-B/32 trained by LAION, 607 MB, for natural-language search).

use std::path::{Path, PathBuf};

/// A download package.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum Package {
    /// Face detection, recognition and eye state.
    Faces,
    /// CLIP image and text embeddings.
    Search,
}

/// One file of a package.
pub struct ModelFile {
    pub name: &'static str,
    pub size: u64,
    pub sha256: &'static str,
    /// Where it can be downloaded, tried in order.
    pub urls: &'static [&'static str],
}

pub struct PackageInfo {
    pub id: Package,
    /// Folder and command name.
    pub key: &'static str,
    pub name: &'static str,
    /// What it holds, for the user (models and their licences).
    pub models: &'static str,
    pub licence: &'static str,
    pub source: &'static str,
    pub files: &'static [ModelFile],
}

pub const YUNET: &str = "face_detection_yunet_2023mar.onnx";
pub const SFACE: &str = "face_recognition_sface_2021dec.onnx";
pub const EYES: &str = "open_closed_eye.onnx";
pub const CLIP_WEIGHTS: &str = "model.safetensors";
pub const CLIP_VOCAB: &str = "vocab.json";
pub const CLIP_MERGES: &str = "merges.txt";

pub const PACKAGES: [PackageInfo; 2] = [
    PackageInfo {
        id: Package::Faces,
        key: "faces",
        name: "Face recognition (YuNet, SFace, open-closed-eye)",
        models: "YuNet face detector (OpenCV Zoo, MIT), SFace face recognizer (OpenCV Zoo, Apache-2.0), open-closed-eye-0001 (Intel Open Model Zoo, Apache-2.0)",
        licence: "MIT / Apache-2.0",
        source: "https://github.com/opencv/opencv_zoo",
        files: &[
            ModelFile {
                name: YUNET,
                size: 232_589,
                sha256: "8f2383e4dd3cfbb4553ea8718107fc0423210dc964f9f4280604804ed2552fa4",
                urls: &[
                    "https://huggingface.co/opencv/face_detection_yunet/resolve/3cc26e7f1014a5ee5d74a42acee58bafc9d0a310/face_detection_yunet_2023mar.onnx",
                    "https://media.githubusercontent.com/media/opencv/opencv_zoo/main/models/face_detection_yunet/face_detection_yunet_2023mar.onnx",
                ],
            },
            ModelFile {
                name: SFACE,
                size: 38_696_353,
                sha256: "0ba9fbfa01b5270c96627c4ef784da859931e02f04419c829e83484087c34e79",
                urls: &[
                    "https://huggingface.co/opencv/face_recognition_sface/resolve/3d7082438a6e4551e840c9b2bb60b71e8da4b524/face_recognition_sface_2021dec.onnx",
                    "https://media.githubusercontent.com/media/opencv/opencv_zoo/main/models/face_recognition_sface/face_recognition_sface_2021dec.onnx",
                ],
            },
            ModelFile {
                name: EYES,
                size: 46_164,
                sha256: "4daa100034482525a26c9afb9297c16580a531189e66e3d2b2ac7d32becfd593",
                urls: &[
                    "https://storage.openvinotoolkit.org/repositories/open_model_zoo/public/2022.1/open-closed-eye-0001/open_closed_eye.onnx",
                    "https://download.01.org/opencv/openvino_training_extensions/models/open_closed_eye/open_closed_eye.onnx",
                ],
            },
        ],
    },
    PackageInfo {
        id: Package::Search,
        key: "search",
        name: "AI search (CLIP ViT-B/32, LAION-2B)",
        models: "CLIP ViT-B/32 trained on LAION-2B (LAION, MIT)",
        licence: "MIT",
        source: "https://huggingface.co/laion/CLIP-ViT-B-32-laion2B-s34B-b79K",
        files: &[
            ModelFile {
                name: CLIP_WEIGHTS,
                size: 605_157_884,
                sha256: "74813fbcdc750f235c9784c367ca1394d2a5c25eb0aac92761752ac239db7cff",
                urls: &[concat!(
                    "https://huggingface.co/laion/CLIP-ViT-B-32-laion2B-s34B-b79K/resolve/1a25a446712ba5ee05982a381eed697ef9b435cf/",
                    "model.safetensors"
                )],
            },
            ModelFile {
                name: "config.json",
                size: 4_355,
                sha256: "1284cbff35169abb23a1c5525a8b0f543c7bd191d4b9aed63880c1571bc4191c",
                urls: &[concat!(
                    "https://huggingface.co/laion/CLIP-ViT-B-32-laion2B-s34B-b79K/resolve/1a25a446712ba5ee05982a381eed697ef9b435cf/",
                    "config.json"
                )],
            },
            ModelFile {
                name: CLIP_VOCAB,
                size: 862_328,
                sha256: "5047b556ce86ccaf6aa22b3ffccfc52d391ea4accdab9c2f2407da5b742d4363",
                urls: &[concat!(
                    "https://huggingface.co/laion/CLIP-ViT-B-32-laion2B-s34B-b79K/resolve/1a25a446712ba5ee05982a381eed697ef9b435cf/",
                    "vocab.json"
                )],
            },
            ModelFile {
                name: CLIP_MERGES,
                size: 524_618,
                sha256: "f1cd414c2d113a28b296afc0fd66ae854c0e0c34d5de373a2f5b7f62e0c02344",
                urls: &[concat!(
                    "https://huggingface.co/laion/CLIP-ViT-B-32-laion2B-s34B-b79K/resolve/1a25a446712ba5ee05982a381eed697ef9b435cf/",
                    "merges.txt"
                )],
            },
        ],
    },
];

pub fn info(id: Package) -> &'static PackageInfo {
    match id {
        Package::Faces => &PACKAGES[0],
        Package::Search => &PACKAGES[1],
    }
}

pub fn by_key(key: &str) -> Option<Package> {
    PACKAGES.iter().find(|m| m.key == key).map(|m| m.id)
}

impl PackageInfo {
    /// Bytes to download.
    pub fn size(&self) -> u64 {
        self.files.iter().map(|f| f.size).sum()
    }
}

/// The folder of `id` under `root`.
pub fn dir(root: &Path, id: Package) -> PathBuf {
    root.join(info(id).key)
}

/// Whether every file of `id` is in place under `root` (sizes checked; the hashes were checked on download).
pub fn installed(root: &Path, id: Package) -> bool {
    let d = dir(root, id);
    info(id).files.iter().all(|f| std::fs::metadata(d.join(f.name)).is_ok_and(|m| m.len() == f.size))
}

/// What to tell the user when `id` is missing.
pub fn missing_message(root: &Path, id: Package) -> String {
    let m = info(id);
    format!(
        "{} {} is not installed: download it ({:.0} MB, {}) when LightCraft offers it, or run `vision.model.download {{\"model\": \"{}\", \"acknowledged\": true}}`; it goes to {}",
        NOT_INSTALLED,
        m.name,
        m.size() as f64 / 1e6,
        m.licence,
        m.key,
        dir(root, id).display()
    )
}

/// The start of every "model not installed" message (the UI offers the download when it sees it).
pub const NOT_INSTALLED: &str = "model missing:";

#[cfg(not(target_arch = "wasm32"))]
/// Download `id` into `root` (blocking; run on a background thread). Files already there and verified are kept;
/// each file is tried from its URLs in turn.
pub fn download(root: &Path, id: Package, cancel: &std::sync::atomic::AtomicBool, progress: &mut dyn FnMut(u64, u64)) -> Result<(), String> {
    let m = info(id);
    let d = dir(root, id);
    let total = m.size();
    let mut before = 0u64;
    for f in m.files {
        let spec = lightcraft_fetch::FileSpec { name: f.name, size: Some(f.size), sha256: Some(f.sha256), max: f.size };
        let mut last = String::new();
        let mut ok = false;
        for url in f.urls {
            let r = lightcraft_fetch::download_url(
                &spec,
                url,
                &d,
                &lightcraft_fetch::Options::default(),
                cancel,
                &mut |p: &lightcraft_fetch::Progress| progress(before + p.done.min(f.size), total),
            );
            match r {
                Ok(()) => {
                    ok = true;
                    break;
                }
                Err(e) if cancel.load(std::sync::atomic::Ordering::Relaxed) => return Err(format!("cancelled ({e})")),
                Err(e) => {
                    log::warn!("vision model {}: {url}: {e}", f.name);
                    last = e.to_string();
                }
            }
        }
        if !ok {
            return Err(format!("{}: {last}", f.name));
        }
        before += f.size;
        progress(before, total);
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_file_is_pinned() {
        for p in &PACKAGES {
            assert_eq!(by_key(p.key), Some(p.id));
            for f in p.files {
                assert_eq!(f.sha256.len(), 64, "{}", f.name);
                assert!(f.sha256.chars().all(|c| c.is_ascii_hexdigit()));
                assert!(!f.urls.is_empty() && f.urls.iter().all(|u| u.starts_with("https://")));
            }
        }
        let root = std::env::temp_dir().join(format!("lc-vision-models-{}", std::process::id()));
        assert!(!installed(&root, Package::Faces));
        assert!(missing_message(&root, Package::Search).starts_with(NOT_INSTALLED));
    }
}
