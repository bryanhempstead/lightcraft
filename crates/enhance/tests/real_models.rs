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

/// `LC_RAW=<a Bayer raw> LC_MODELS=… LC_OUT=<dir>`: Enhance timings on a real photo, and 512² crops (plain AHD,
/// Raw Details, built-in denoise, RawNIND) as PNGs through a plain gamma, for looking at.
#[test]
#[ignore]
fn enhance_a_real_raw() {
    use lightcraft_enhance::Options;
    use lightcraft_enhance::job::{self, Models};
    let (Some(dir), Some(raw)) = (models(), std::env::var_os("LC_RAW")) else { return };
    let out = std::env::var_os("LC_OUT").map(PathBuf::from);
    let bytes = std::fs::read(&raw).unwrap();
    let t = Instant::now();
    let img = lightcraft_raw::decode(&bytes).unwrap();
    eprintln!("decode {:.2} s, {}×{}", t.elapsed().as_secs_f64(), img.width, img.height);
    let onnx = std::env::temp_dir().join(format!("lc-rawnind-job-{}.onnx", std::process::id()));
    lightcraft_denoise_core::archive::extract(&dir.join("rawdenoise-nind.dtmodel"), "model_bayer.onnx", &onnx, 512 << 20).unwrap();
    let model = job::DenoiseModel::load(&onnx, lightcraft_enhance::best_device()).unwrap();
    let _ = std::fs::remove_file(&onnx);
    let with_model = Models { denoise: Some(std::sync::Arc::new(model)), super_resolution: None };
    let runs: [(&str, Options, &Models); 4] = [
        ("ahd", Options { denoise: 0.0, raw_details: false, super_resolution: false }, &Models::default()),
        ("rcd", Options { denoise: 0.0, raw_details: true, super_resolution: false }, &Models::default()),
        ("builtin", Options { denoise: 100.0, ..Default::default() }, &Models::default()),
        ("rawnind", Options { denoise: 100.0, ..Default::default() }, &with_model),
    ];
    for (name, o, m) in runs {
        if name == "builtin" && std::env::var_os("LC_SKIP_BUILTIN").is_some() {
            continue;
        }
        let t = Instant::now();
        let r = match name {
            "ahd" => job::Output { image: img.develop(lightcraft_raw::Method::Ahd).unwrap(), denoiser: None, timings: Vec::new() },
            _ => job::run(&img, &o, m, &|_, _| true).unwrap(),
        };
        eprintln!("{name}: {:.2} s ({:?}) {:?}", t.elapsed().as_secs_f64(), r.denoiser, r.timings);
        if let Some(out) = &out {
            let (w, h) = (r.image.width, r.image.height);
            let (x0, y0, s) = (w / 2 - 256, h / 2 - 256, 512);
            // grey-world white balance over the crop, then a plain gamma
            let mut sum = [0f64; 3];
            for y in y0..y0 + s {
                for x in x0..x0 + s {
                    for c in 0..3 {
                        sum[c] += r.image.data[y * w + x][c] as f64;
                    }
                }
            }
            let g = sum[1] / (s * s) as f64;
            let wb: [f32; 3] = std::array::from_fn(|c| (0.18 / g.max(1e-6) * sum[1] / sum[c].max(1e-9)) as f32);
            let mut px = Vec::with_capacity(s * s * 3);
            for y in y0..y0 + s {
                for x in x0..x0 + s {
                    let p = r.image.data[y * w + x];
                    for c in 0..3 {
                        px.push(((p[c] * wb[c]).clamp(0.0, 1.0).powf(1.0 / 2.2) * 255.0) as u8);
                    }
                }
            }
            std::fs::create_dir_all(out).unwrap();
            let f = std::fs::File::create(out.join(format!("{name}.png"))).unwrap();
            let mut e = png::Encoder::new(std::io::BufWriter::new(f), s as u32, s as u32);
            e.set_color(png::ColorType::Rgb);
            e.write_header().unwrap().write_image_data(&px).unwrap();
        }
    }
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
            eprintln!(
                "{dt:?}: conv2d {tc:?}, im2col conv {ti:?}, same-size matmul {tm:?}, cat {:?} ({} {} {} {})",
                t.elapsed(),
                y.dims()[1],
                z.dims()[1],
                m.dims()[0],
                c.dims()[1]
            );
        }
    }
}

#[test]
#[ignore]
fn compact_layer_speed() {
    use candle_core::{DType, Tensor};
    let d = lightcraft_enhance::best_device();
    for dt in [DType::F32, DType::F16] {
        let x = Tensor::randn(0f32, 1f32, (1, 64, 384, 384), &d).unwrap().to_dtype(dt).unwrap();
        let w = Tensor::randn(0f32, 0.05f32, (64, 64, 3, 3), &d).unwrap().to_dtype(dt).unwrap();
        let a = Tensor::randn(0f32, 0.1f32, (1, 64, 1, 1), &d).unwrap().to_dtype(dt).unwrap();
        for _ in 0..2 {
            let t = Instant::now();
            let y = lightcraft_enhance::netrun::conv2d(&x, &w, 1).unwrap();
            d.synchronize().unwrap();
            let tc = t.elapsed();
            let t = Instant::now();
            let pos = y.relu().unwrap();
            let z = (&pos + (&y - &pos).unwrap().broadcast_mul(&a).unwrap()).unwrap();
            d.synchronize().unwrap();
            let tp = t.elapsed();
            let t = Instant::now();
            let big = Tensor::randn(0f32, 1f32, (1, 48, 384, 384), &d).unwrap().to_dtype(dt).unwrap();
            let s = big
                .reshape((1, 3, 4, 4, 384, 384))
                .unwrap()
                .permute((0, 1, 4, 2, 5, 3))
                .unwrap()
                .contiguous()
                .unwrap()
                .reshape((1, 3, 1536, 1536))
                .unwrap();
            let p = s.avg_pool2d(2).unwrap();
            d.synchronize().unwrap();
            eprintln!("{dt:?}: conv {tc:?}, prelu {tp:?}, shuffle+pool {:?} {}", t.elapsed(), z.dims()[1] + p.dims()[1]);
        }
    }
}

#[test]
#[ignore]
fn esrgan_speed() {
    let Some(dir) = models() else { return };
    let file = std::env::var("LC_SR_FILE").unwrap_or_else(|_| "realesr-general-x4v3.pth".into());
    let m = lightcraft_enhance::esrgan::SuperResolution::load(&dir.join(&file), lightcraft_enhance::best_device()).unwrap();
    let mut img = lightcraft_raster::Rgb32f::new(768, 768);
    for (i, p) in img.data.iter_mut().enumerate() {
        *p = [(i % 768) as f32 / 768.0, 0.3, 0.6];
    }
    let small = lightcraft_raster::Rgb32f::new(64, 64);
    let _ = m.upscale(&small, &AtomicBool::new(false), &|_, _| {}).unwrap();
    let t = Instant::now();
    let out = m.upscale(&img, &AtomicBool::new(false), &|_, _| {}).unwrap();
    let s = t.elapsed().as_secs_f64();
    eprintln!("{file}: 768² → {}² in {s:.2} s ({:.1} s per input megapixel)", out.width, s / 0.59);
    let p = out.data[768 * 1536 + 768];
    assert!((p[1] - 0.3).abs() < 0.05 && (p[2] - 0.6).abs() < 0.05, "{p:?}");
}
