//! Opt-in checks with the real downloaded models (never in CI):
//! `LC_MODELS=<folder with rawdenoise-nind.dtmodel, RealESRGAN_x2plus.pth> cargo test -p lightcraft-enhance --release --test real_models -- --ignored --nocapture`

use std::path::PathBuf;
use std::sync::atomic::AtomicBool;
use std::time::Instant;

use lightcraft_denoise_core::run::TileRunner;

fn models() -> Option<PathBuf> {
    std::env::var_os("LC_MODELS").map(PathBuf::from)
}

#[test]
#[ignore]
fn rawnind_runs_a_tile_on_candle() {
    let Some(dir) = models() else { return };
    let onnx = std::env::temp_dir().join(format!("lc-rawnind-{}.onnx", std::process::id()));
    lightcraft_denoise_core::archive::extract(&dir.join("rawdenoise-nind.dtmodel"), "model_bayer.onnx", &onnx, 512 << 20).unwrap();
    let net = lightcraft_enhance::onnx::read(&onnx).unwrap();
    let _ = std::fs::remove_file(&onnx);
    let tile = 256;
    let input: Vec<f32> = (0..4 * tile * tile).map(|i| 0.2 + 0.05 * (((i * 7919) % 101) as f32 / 101.0 - 0.5)).collect();
    let want = lightcraft_enhance::reference::run(&net, tile, &input).ok();
    let r = lightcraft_enhance::netrun::NetRunner::new(&net, tile, lightcraft_enhance::best_device()).unwrap();
    let _ = r.run(&input).unwrap();
    let t = Instant::now();
    let got = r.run(&input).unwrap();
    eprintln!("RawNIND {tile}² tile on {:?}: {:?} ({:.1} GMAC)", r.device(), t.elapsed(), net.macs(tile) as f64 / 1e9);
    if let Some(want) = want {
        let worst = got.iter().zip(&want).map(|(a, b)| (a - b).abs()).fold(0f32, f32::max);
        let range = want.iter().fold(0f32, |m, v| m.max(v.abs()));
        let mean_diff = got.iter().zip(&want).map(|(a, b)| (a - b).abs()).sum::<f32>() / want.len() as f32;
        eprintln!("difference to the reference interpreter: worst {worst}, mean {mean_diff}, output range {range}");
        assert!(worst <= 1e-4 * range.max(1.0), "worst {worst} of {range}");
    }
    let mean = got.iter().sum::<f32>() / got.len() as f32;
    assert!(mean.is_finite());
}

#[test]
#[ignore]
fn metal_op_speed() {
    use candle_core::{DType, Tensor};
    let d = lightcraft_enhance::best_device();
    for dt in [DType::F32, DType::F16] {
        let x = Tensor::randn(0f32, 1f32, (1, 192, 192, 192), &d).unwrap().to_dtype(dt).unwrap();
        let w = Tensor::randn(0f32, 1f32, (64, 192, 3, 3), &d).unwrap().to_dtype(dt).unwrap();
        let a = Tensor::randn(0f32, 1f32, (64, 192 * 9), &d).unwrap().to_dtype(dt).unwrap();
        let b = Tensor::randn(0f32, 1f32, (192 * 9, 192 * 192), &d).unwrap().to_dtype(dt).unwrap();
        for _ in 0..2 {
            let t = Instant::now();
            let y = x.conv2d(&w, 1, 1, 1, 1).unwrap();
            d.synchronize().unwrap();
            let tc = t.elapsed();
            let t = Instant::now();
            let z = lightcraft_enhance::netrun::conv2d(&x, &w, 1).unwrap();
            d.synchronize().unwrap();
            let ti = t.elapsed();
            let t = Instant::now();
            let m = a.matmul(&b).unwrap();
            d.synchronize().unwrap();
            let tm = t.elapsed();
            let t = Instant::now();
            let c = Tensor::cat(&[&x, &x], 1).unwrap();
            d.synchronize().unwrap();
            eprintln!("{dt:?}: conv2d {tc:?}, im2col conv {ti:?}, same-size matmul {tm:?}, cat {:?} ({} {} {} {})", t.elapsed(), y.dims()[1], z.dims()[1], m.dims()[0], c.dims()[1]);
        }
    }
}

#[test]
#[ignore]
fn esrgan_speed() {
    let Some(dir) = models() else { return };
    let m = lightcraft_enhance::esrgan::SuperResolution::load(&dir.join("RealESRGAN_x2plus.pth"), lightcraft_enhance::best_device()).unwrap();
    let mut img = lightcraft_raster::Rgb32f::new(768, 768);
    for (i, p) in img.data.iter_mut().enumerate() {
        *p = [(i % 768) as f32 / 768.0, 0.3, 0.6];
    }
    let small = lightcraft_raster::Rgb32f::new(64, 64);
    let _ = m.upscale(&small, &AtomicBool::new(false), &|_, _| {}).unwrap();
    let t = Instant::now();
    let out = m.upscale(&img, &AtomicBool::new(false), &|_, _| {}).unwrap();
    let s = t.elapsed().as_secs_f64();
    eprintln!("ESRGAN 768² → {}² in {s:.2} s ({:.1} s per input megapixel)", out.width, s / 0.59);
}
