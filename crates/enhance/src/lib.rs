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
//! - [`lama`]: Generative Remove's inpainting (LaMa, big-lama).
//! - [`dsrnet`]: Remove reflections (DSRNet with VGG-19 features).
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
pub mod dsrnet;
#[cfg(feature = "models")]
pub mod esrgan;
#[cfg(feature = "models")]
pub mod genfill;
#[cfg(feature = "models")]
pub mod job;
#[cfg(feature = "models")]
pub mod lama;
#[cfg(feature = "models")]
pub mod netrun;
#[cfg(feature = "models")]
#[allow(clippy::all, dead_code)]
pub mod sd;

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
/// the CPU. One device for the process: candle compiles its Metal kernels per device on first use (~30 s on an
/// M3 Pro), so every model shares them.
pub fn best_device() -> candle_core::Device {
    static DEVICE: std::sync::OnceLock<candle_core::Device> = std::sync::OnceLock::new();
    DEVICE
        .get_or_init(|| {
            #[cfg(target_os = "macos")]
            if std::env::var_os("LIGHTCRAFT_ENHANCE_CPU").is_none()
                && let Ok(d) = candle_core::Device::new_metal(0)
            {
                return d;
            }
            candle_core::Device::Cpu
        })
        .clone()
}

#[cfg(feature = "models")]
/// Hold this while using the GPU device: candle 0.9.2's Metal device gives wrong results when several threads queue
/// work on it at once (measured: the runner tests fail when they share it unguarded), so every model's GPU work runs
/// one at a time. `None` on the CPU (no lock needed).
pub fn gpu_lock(device: &candle_core::Device) -> Option<std::sync::MutexGuard<'static, ()>> {
    static GPU: std::sync::Mutex<()> = std::sync::Mutex::new(());
    (!device.is_cpu()).then(|| GPU.lock().unwrap_or_else(std::sync::PoisonError::into_inner))
}

#[cfg(feature = "models")]
/// Compile the device's kernels now (a background thread at start-up), so the first Enhance doesn't wait for them.
pub fn warm_up() {
    let d = best_device();
    if d.is_cpu() {
        return;
    }
    let _gpu = gpu_lock(&d);
    let run = || -> candle_core::Result<()> {
        let x = candle_core::Tensor::zeros((1, 8, 16, 16), candle_core::DType::F32, &d)?;
        let w = candle_core::Tensor::zeros((8, 8, 3, 3), candle_core::DType::F32, &d)?;
        let y = netrun::conv2d(&x, &w, 1)?.relu()?.max_pool2d(2)?;
        let _ = y.to_dtype(candle_core::DType::F16)?.avg_pool2d(2)?.to_dtype(candle_core::DType::F32)?.flatten_all()?.to_vec1::<f32>()?;
        d.synchronize()
    };
    if let Err(e) = run() {
        log::warn!("enhance warm-up: {e}");
    }
}
