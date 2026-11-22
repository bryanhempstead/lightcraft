//! Enhance: Lightroom Classic's Photo ▸ Enhance (Denoise, Raw Details, Super Resolution) and the depth map Lens Blur
//! and Depth Range masks read, with open models run on candle (Metal on macOS, else the CPU).
//!
//! - [`models`]: the three models, their licences and pinned downloads (never bundled; fetched on request).
//! - AI Denoise: `lightcraft-denoise-core` (ported from storytold/lightcraft) cuts the Bayer mosaic into tiles and
//!   blends them; each tile runs either its built-in block-matching denoiser or the RawNIND UtNet2 model
//!   ([`onnx`] reads it into a [`net::Net`], [`netrun::NetRunner`] runs it on candle). Both denoise and demosaic at
//!   once, like Lightroom's Denoise.
//! - [`esrgan`]: 2× Super Resolution (Real-ESRGAN x2plus).
//! - [`depth`]: relative depth (Depth Anything V2 Small).
//!
//! Model files are hostile input: sizes are capped and checked, a malformed file is an error, never a panic.
//!
//! `net`, `onnx`, `onnx_proto`, `reference` and `synthetic*` are from storytold/lightcraft's `lightcraft-denoise`
//! (MIT OR Apache-2.0); `dinov2` and `depth_anything_v2` from Hugging Face candle-transformers 0.9.2 (MIT OR
//! Apache-2.0). See NOTICE.
#![forbid(unsafe_code)]
#![deny(clippy::unwrap_used, clippy::expect_used, clippy::panic, clippy::unimplemented, clippy::todo, clippy::unreachable)]

pub mod models;
pub mod net;
pub mod onnx;
mod onnx_proto;
pub mod options;
pub mod reference;
#[doc(hidden)]
pub mod synthetic;
#[doc(hidden)]
pub mod synthetic_proto;

#[cfg(feature = "models")]
#[allow(clippy::all, dead_code)]
mod depth_anything_v2;
#[cfg(feature = "models")]
#[allow(clippy::all, dead_code)]
mod dinov2;

#[cfg(feature = "models")]
pub mod depth;
#[cfg(feature = "models")]
pub mod esrgan;
#[cfg(feature = "models")]
pub mod job;
#[cfg(feature = "models")]
pub mod netrun;

pub use lightcraft_denoise_core as denoise;
pub use options::Options;

#[cfg(feature = "models")]
pub use lightcraft_fetch as fetch;

#[derive(Debug, thiserror::Error)]
pub enum Error {
    #[error("{0}")]
    Input(String),
    #[error("model: {0}")]
    Model(String),
    #[error("cancelled")]
    Cancelled,
    #[cfg(feature = "models")]
    #[error(transparent)]
    Candle(#[from] candle_core::Error),
}

pub type Result<T> = std::result::Result<T, Error>;

#[cfg(feature = "models")]
/// The best device here: Metal on macOS (when a GPU is available), else the CPU. `LIGHTCRAFT_ENHANCE_CPU=1` forces
/// the CPU.
pub fn best_device() -> candle_core::Device {
    #[cfg(target_os = "macos")]
    if std::env::var_os("LIGHTCRAFT_ENHANCE_CPU").is_none()
        && let Ok(d) = candle_core::Device::new_metal(0)
    {
        return d;
    }
    candle_core::Device::Cpu
}
