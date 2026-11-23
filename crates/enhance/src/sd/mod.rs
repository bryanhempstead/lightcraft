//! Stable Diffusion's building blocks (UNet, VAE, CLIP text encoder, DDIM), from Hugging Face candle-transformers
//! 0.9.2 `models::stable_diffusion` (MIT OR Apache-2.0, see NOTICE). Changes: `candle` is `candle_core`, the tracing
//! spans are no-ops (no `tracing` dependency), flash attention is left out, the `unwrap`s are errors, the VAE gives
//! its mean ([`vae::DiagonalGaussianDistribution::mode`]) so a fill is the same every time, stride-1 convolutions
//! are LightCraft's ([`with_tracing::Conv2d`]), the UNet's attention stays in the model's precision (upstream casts
//! to single: twice the time at 512 px), and only the parts the inpainting pipeline ([`crate::genfill`]) uses are
//! here (no DDPM / Euler / UniPC schedulers, no SD 2 / XL configs).

pub mod attention;
pub mod clip;
pub mod ddim;
pub mod embeddings;
pub mod resnet;
pub mod schedulers;
pub mod unet_2d;
pub mod unet_2d_blocks;
pub mod utils;
pub mod vae;

/// No-op stand-ins for the `tracing` spans the upstream code opens.
pub(crate) mod tracing {
    #[derive(Clone, Debug)]
    pub struct Span;

    impl Span {
        pub fn enter(&self) {}
    }

    #[allow(dead_code)]
    pub enum Level {
        TRACE,
    }

    macro_rules! span {
        ($($t:tt)*) => {
            $crate::sd::tracing::Span
        };
    }
    pub(crate) use span;
}

/// candle-transformers' traced convolution, here LightCraft's faster one ([`crate::netrun::conv2d_taps`], ~3× candle
/// 0.9.2's Metal convolution) for stride-1 kernels, whose weights are kept laid out as taps; strided ones (the
/// downsamplers) stay candle's.
pub(crate) mod with_tracing {
    use candle_core::{Module, Result, Tensor};

    #[derive(Debug, Clone)]
    pub struct Conv2d {
        /// `[kh, kw, cin, cout]` (the fast path), else candle's convolution.
        taps: Option<Tensor>,
        inner: Option<candle_nn::Conv2d>,
        pad: usize,
        bias: Option<Tensor>,
    }

    pub fn conv2d(cin: usize, cout: usize, k: usize, cfg: candle_nn::Conv2dConfig, vb: candle_nn::VarBuilder) -> Result<Conv2d> {
        let groups = cfg.groups.max(1);
        let w = vb.get((cout, cin / groups, k, k), "weight")?;
        let b = vb.get(cout, "bias")?;
        if cfg.stride == 1 && cfg.dilation == 1 && groups == 1 {
            Ok(Conv2d { taps: Some(w.permute((2, 3, 1, 0))?.contiguous()?), inner: None, pad: cfg.padding, bias: Some(b.reshape((1, cout, 1, 1))?) })
        } else {
            Ok(Conv2d { taps: None, inner: Some(candle_nn::Conv2d::new(w, Some(b), cfg)), pad: cfg.padding, bias: None })
        }
    }

    impl Module for Conv2d {
        fn forward(&self, x: &Tensor) -> Result<Tensor> {
            match (&self.taps, &self.inner) {
                (Some(t), _) => {
                    let y = crate::netrun::conv2d_taps(x, t, self.pad)?;
                    match &self.bias {
                        Some(b) => y.broadcast_add(b),
                        None => Ok(y),
                    }
                }
                (None, Some(c)) => c.forward(x),
                (None, None) => candle_core::bail!("a convolution without weights"),
            }
        }
    }
}

/// SD 1.5's UNet (in_channels 9 for the inpainting checkpoint), from runwayml's `unet/config.json`.
pub fn unet_config(sliced_attention_size: Option<usize>) -> unet_2d::UNet2DConditionModelConfig {
    let bc = |out_channels, use_cross_attn, attention_head_dim| unet_2d::BlockConfig { out_channels, use_cross_attn, attention_head_dim };
    unet_2d::UNet2DConditionModelConfig {
        blocks: vec![bc(320, Some(1), 8), bc(640, Some(1), 8), bc(1280, Some(1), 8), bc(1280, None, 8)],
        center_input_sample: false,
        cross_attention_dim: 768,
        downsample_padding: 1,
        flip_sin_to_cos: true,
        freq_shift: 0.,
        layers_per_block: 2,
        mid_block_scale_factor: 1.,
        norm_eps: 1e-5,
        norm_num_groups: 32,
        sliced_attention_size,
        use_linear_projection: false,
    }
}

/// SD 1.5's VAE (`vae/config.json`).
pub fn vae_config() -> vae::AutoEncoderKLConfig {
    vae::AutoEncoderKLConfig {
        block_out_channels: vec![128, 256, 512, 512],
        layers_per_block: 2,
        latent_channels: 4,
        norm_num_groups: 32,
        use_quant_conv: true,
        use_post_quant_conv: true,
    }
}

/// SD 1.5's DDIM (`scheduler/scheduler_config.json`: scaled-linear betas 0.00085…0.012, epsilon prediction,
/// eta 0 — no noise is added after the start, so the same seed gives the same fill).
pub fn scheduler(steps: usize) -> candle_core::Result<Box<dyn schedulers::Scheduler>> {
    use schedulers::SchedulerConfig;
    let cfg = ddim::DDIMSchedulerConfig { prediction_type: schedulers::PredictionType::Epsilon, eta: 0.0, ..Default::default() };
    cfg.build(steps.clamp(1, 1000))
}
