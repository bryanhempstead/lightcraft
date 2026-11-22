//! Recognition for the Library (L3): faces for the People view, eyes and face focus for Assisted Culling, and
//! image / text embeddings for natural-language search.
//!
//! - [`yunet`]: YuNet face detection (OpenCV Zoo, MIT) — boxes and five landmarks.
//! - [`sface`]: SFace face recognition (OpenCV Zoo, Apache-2.0) — a 128-number embedding per aligned face.
//! - [`eyes`]: open / closed eyes on a detected face (the OpenVINO Open Model Zoo's `open-closed-eye-0001`,
//!   Apache-2.0).
//! - [`clip`]: CLIP ViT-B/32 (OpenAI, MIT) image and text embeddings, ported from candle-transformers.
//! - [`cluster`]: grouping faces into people and matching them to named people.
//! - [`index`]: the append-only, checksummed side files the embeddings are kept in.
//!
//! The ONNX models run on a small interpreter of our own ([`graph`], CPU: candle's matmul for the products); CLIP
//! runs on candle (Metal on macOS). No model is part of LightCraft: each is a consented, size- and SHA-256-pinned
//! download ([`models`]). Without one, the features that need it report it and everything else works.
#![forbid(unsafe_code)]
#![deny(clippy::unwrap_used, clippy::expect_used, clippy::panic, clippy::unimplemented, clippy::todo, clippy::unreachable)]

pub mod cluster;
pub mod image;
pub mod index;
pub mod models;

#[cfg(not(target_arch = "wasm32"))]
pub mod clip;
#[cfg(not(target_arch = "wasm32"))]
pub mod eyes;
#[cfg(not(target_arch = "wasm32"))]
pub mod graph;
#[cfg(not(target_arch = "wasm32"))]
pub mod proto;
#[cfg(not(target_arch = "wasm32"))]
pub mod sface;
#[cfg(not(target_arch = "wasm32"))]
pub mod yunet;

#[cfg(not(target_arch = "wasm32"))]
pub use lightcraft_fetch as fetch;

#[derive(Debug, thiserror::Error, Clone, PartialEq)]
pub enum Error {
    /// The model file is unreadable, malformed or uses something we can't run.
    #[error("model: {0}")]
    Model(String),
    #[error("{0}")]
    Io(String),
    /// The model isn't installed (the message says how to get it).
    #[error("{0}")]
    NotInstalled(String),
    #[error("cancelled")]
    Cancelled,
}

pub type Result<T> = std::result::Result<T, Error>;

/// A face found in a picture: its box and landmarks in the picture's pixels, and the detector's confidence.
#[derive(Clone, Copy, Debug, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct Face {
    /// `[x, y, w, h]`, top-left origin.
    pub rect: [f32; 4],
    /// Right eye, left eye (of the person: the eye on the picture's left first), nose tip, right and left mouth
    /// corners, as `[x, y]`.
    pub landmarks: [[f32; 2]; 5],
    pub score: f32,
}

impl Face {
    /// The same face in a picture `s` times as large.
    pub fn scaled(&self, s: f32) -> Face {
        let mut f = *self;
        for v in &mut f.rect {
            *v *= s;
        }
        for p in &mut f.landmarks {
            p[0] *= s;
            p[1] *= s;
        }
        f
    }
}

/// The best device for the transformer models: Metal on macOS when it works, else the CPU.
#[cfg(not(target_arch = "wasm32"))]
pub fn best_device() -> candle_core::Device {
    #[cfg(target_os = "macos")]
    {
        if std::env::var_os("LIGHTCRAFT_VISION_CPU").is_none()
            && let Ok(d) = candle_core::Device::new_metal(0)
        {
            return d;
        }
    }
    candle_core::Device::Cpu
}
