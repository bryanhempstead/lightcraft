//! One raw file through Enhance: Denoise (and its demosaic), or Raw Details (the RCD demosaic), then Super
//! Resolution; the result is camera RGB exactly where [`lightcraft_raw::RawImage::develop`] gives it (normalised,
//! demosaiced, `OpcodeList3` and the default crop applied), so it can be written as a linear DNG or used as the
//! photo's source in place.

use std::path::Path;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};

use lightcraft_denoise_core::bayer::Layout;
use lightcraft_denoise_core::builtin::{self, Builtin};
use lightcraft_denoise_core::run::{Control, Params, TileRunner, denoise_bayer};
use lightcraft_denoise_core::{DenoiserManifest, known};
use lightcraft_raster::Rgb32f;
use lightcraft_raw::{Method, Normalized, RawImage};
use serde::{Deserialize, Serialize};

use crate::esrgan::SuperResolution;
use crate::netrun::NetRunner;
use crate::{Error, Result};

/// What to do (Lightroom's Enhance dialog).
#[derive(Clone, Copy, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(default, rename_all = "camelCase")]
pub struct Options {
    /// AI Denoise with this amount, 1..100 (0: off).
    pub denoise: f32,
    /// Raw Details: the finer demosaic (implied by Denoise, which demosaics itself).
    pub raw_details: bool,
    /// Super Resolution: twice the width and height.
    pub super_resolution: bool,
}

impl Options {
    pub fn any(&self) -> bool {
        self.denoise > 0.0 || self.raw_details || self.super_resolution
    }

    /// Lightroom's file name suffix: `Enhanced-NR`, `-RD`, `-SR`, `-NR-SR` …
    pub fn suffix(&self) -> String {
        let mut s = String::from("Enhanced");
        if self.denoise > 0.0 {
            s.push_str("-NR");
        } else if self.raw_details {
            s.push_str("-RD");
        }
        if self.super_resolution {
            s.push_str("-SR");
        }
        s
    }

    /// A stable text naming these options (cache keys).
    pub fn key(&self) -> String {
        format!("nr{:.0}-rd{}-sr{}", self.denoise.clamp(0.0, 100.0), u8::from(self.raw_details), u8::from(self.super_resolution))
    }
}

/// The RawNIND model, loaded.
pub struct DenoiseModel {
    pub runner: NetRunner,
    pub manifest: DenoiserManifest,
}

impl DenoiseModel {
    /// Load `model_bayer.onnx` (see [`crate::models`]) on `device`.
    pub fn load(onnx: &Path, device: candle_core::Device) -> Result<Self> {
        let manifest = known::find("rawnind-bayer").map(|k| k.manifest).ok_or_else(|| Error::Model("RawNIND is not in this build's catalog".into()))?;
        let net = crate::onnx::read(onnx).map_err(|e| Error::Model(format!("{}: {e}", onnx.display())))?;
        let runner = NetRunner::new(&net, manifest.tile as usize, device).map_err(|e| Error::Model(e.to_string()))?;
        Ok(DenoiseModel { runner, manifest })
    }
}

/// The models a job may use (each `None` when not installed: Denoise then uses the built-in denoiser, Super
/// Resolution is refused).
#[derive(Clone, Default)]
pub struct Models {
    pub denoise: Option<Arc<DenoiseModel>>,
    pub super_resolution: Option<Arc<SuperResolution>>,
}

/// `progress(fraction, stage)`; returns false to cancel.
pub type Progress<'a> = dyn Fn(f32, &str) -> bool + Sync + 'a;

/// Which denoiser a job used.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub enum Denoiser {
    /// RawNIND UtNet2 on candle.
    RawNind,
    /// The built-in block-matching denoiser (no download).
    Builtin,
}

/// What a job made.
pub struct Output {
    /// Camera RGB (white = 1), as `RawImage::develop` gives it (twice the size with Super Resolution).
    pub image: Rgb32f,
    pub denoiser: Option<Denoiser>,
    /// Seconds per stage.
    pub timings: Vec<(&'static str, f64)>,
}

fn layout_of(n: &Normalized) -> Option<Layout> {
    let c = n.cfa.as_ref().filter(|c| c.is_bayer())?;
    Layout::from_cell([c.color_at(0, 0), c.color_at(1, 0), c.color_at(0, 1), c.color_at(1, 1)])
}

/// Whether Denoise / Raw Details can run on this raw (a Bayer mosaic, not already demosaiced).
pub fn bayer(raw: &RawImage) -> bool {
    raw.cpp == 1 && raw.cfa.as_ref().is_some_and(lightcraft_raw::Cfa::is_bayer)
}

/// Run `opts` on `raw`.
pub fn run(raw: &RawImage, opts: &Options, models: &Models, progress: &Progress) -> Result<Output> {
    if !opts.any() {
        return Err(Error::Input("choose Denoise, Raw Details or Super Resolution".into()));
    }
    if opts.super_resolution && models.super_resolution.is_none() {
        return Err(Error::Input("the Super Resolution model is not installed".into()));
    }
    if (opts.denoise > 0.0 || opts.raw_details) && !bayer(raw) {
        return Err(Error::Input(
            "Denoise and Raw Details need a Bayer raw file (this one has another sensor layout or is already demosaiced)".into(),
        ));
    }
    let cancel = AtomicBool::new(false);
    let mut timings = Vec::new();
    let mut denoiser = None;
    let t0 = std::time::Instant::now();
    let span = if opts.super_resolution { 0.35 } else { 0.95 };
    let image = if opts.denoise > 0.0 {
        let amount = opts.denoise.clamp(0.0, 100.0) / 100.0;
        raw.develop_with(|n| {
            let layout = layout_of(n).ok_or(lightcraft_raw::RawError::Unsupported("not a Bayer mosaic".into()))?;
            let report = |done: usize, total: usize| {
                let f = done as f32 / total.max(1) as f32;
                if !progress(span * f, "Denoising") {
                    cancel.store(true, Ordering::Relaxed);
                }
            };
            let ctl = Control { cancel: Some(&cancel), progress: Some(&report) };
            let (result, used) = match &models.denoise {
                Some(m) => {
                    let p = Params {
                        tile: m.manifest.tile as usize,
                        overlap: m.manifest.overlap as usize,
                        gain: m.manifest.gain,
                        clip: Some(0.99),
                        parallel: if m.runner.device().is_cpu() { 2 } else { 1 },
                    };
                    (denoise_bayer(&n.data, n.width, n.height, layout, &m.runner, &p, &ctl), Denoiser::RawNind)
                }
                None => {
                    let b = Builtin::with_noise(builtin::estimate_noise(&n.data, n.width, n.height));
                    let p = Params {
                        tile: builtin::TILE as usize,
                        overlap: builtin::OVERLAP as usize,
                        gain: builtin::manifest().gain,
                        clip: Some(0.99),
                        parallel: rayon::current_num_threads().clamp(1, 4),
                    };
                    (denoise_bayer(&n.data, n.width, n.height, layout, &b as &dyn TileRunner, &p, &ctl), Denoiser::Builtin)
                }
            };
            denoiser = Some(used);
            let mut den = match result {
                Ok(d) => d,
                Err(lightcraft_denoise_core::Error::Cancelled) => return Err(lightcraft_raw::RawError::Unsupported("cancelled".into())),
                Err(e) => return Err(lightcraft_raw::RawError::Unsupported(e.to_string())),
            };
            if amount < 0.999 {
                // Amount: towards the plain (Raw Details) demosaic
                let base = lightcraft_raw::demosaic(n, Method::Rcd);
                lightcraft_denoise_core::product::blend(&mut den, &base, 1.0 - amount);
            }
            Ok(den)
        })
    } else {
        let method = if opts.raw_details { Method::Rcd } else { Method::Ahd };
        progress(0.05, "Demosaicing");
        raw.develop(method)
    };
    if cancel.load(Ordering::Relaxed) {
        return Err(Error::Cancelled);
    }
    let mut image = image.map_err(|e| Error::Input(e.to_string()))?;
    timings.push((if opts.denoise > 0.0 { "denoise" } else { "demosaic" }, t0.elapsed().as_secs_f64()));
    if opts.super_resolution
        && let Some(sr) = &models.super_resolution
    {
        let t1 = std::time::Instant::now();
        if !progress(span, "Super Resolution") {
            return Err(Error::Cancelled);
        }
        let report = |done: usize, total: usize| {
            if !progress(span + (0.95 - span) * done as f32 / total.max(1) as f32, "Super Resolution") {
                cancel.store(true, Ordering::Relaxed);
            }
        };
        image = sr.upscale(&image, &cancel, &report)?;
        timings.push(("superResolution", t1.elapsed().as_secs_f64()));
    }
    Ok(Output { image, denoiser, timings })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn suffixes_and_keys_follow_lightroom() {
        let o = |denoise, raw_details, super_resolution| Options { denoise, raw_details, super_resolution };
        assert_eq!(o(50.0, false, false).suffix(), "Enhanced-NR");
        assert_eq!(o(0.0, true, false).suffix(), "Enhanced-RD");
        assert_eq!(o(0.0, false, true).suffix(), "Enhanced-SR");
        assert_eq!(o(50.0, true, true).suffix(), "Enhanced-NR-SR");
        assert_ne!(o(50.0, false, false).key(), o(60.0, false, false).key());
        assert!(!o(0.0, false, false).any());
    }
}
