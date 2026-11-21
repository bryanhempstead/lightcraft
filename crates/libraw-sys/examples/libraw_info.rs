//! `cargo run --release -p lightcraft-libraw-sys --example libraw_info -- FILE...`: what LibRaw
//! reads from each file (geometry, levels, CFA, white balance, matrix, names) and the decode time.
fn main() {
    println!("LibRaw {:?}, {} cameras", lightcraft_libraw_sys::version(), lightcraft_libraw_sys::camera_count());
    for path in std::env::args().skip(1) {
        let Ok(bytes) = std::fs::read(&path) else {
            println!("{path}: unreadable");
            continue;
        };
        let t = std::time::Instant::now();
        match lightcraft_libraw_sys::decode(&bytes) {
            Ok(d) => {
                let i = &d.info;
                println!("== {path} ({:.0} ms, {})", t.elapsed().as_secs_f64() * 1000.0, i.decoder);
                println!("  {} {} · lens {:?} · ISO {} · {} s · f/{} · {} mm", i.make, i.model, i.lens, i.iso, i.shutter, i.aperture, i.focal_length);
                println!(
                    "  frame {}x{} visible {:?} crop {:?} orientation {} channels {}",
                    i.raw_width, i.raw_height, i.visible, i.default_crop, i.orientation, i.channels
                );
                println!(
                    "  cfa {:?} bits {} white {} black {}x{} {:?}",
                    i.cfa,
                    i.bits,
                    i.white,
                    i.black.rows,
                    i.black.cols,
                    &i.black.values[..i.black.values.len().min(8)]
                );
                println!("  wb {:?} xyz→cam {:?}", i.as_shot_multipliers, i.xyz_to_camera);
                let max = d.samples.iter().max().copied().unwrap_or(0);
                let mean = d.samples.iter().map(|&v| v as f64).sum::<f64>() / d.samples.len().max(1) as f64;
                println!("  samples {} max {max} mean {mean:.1}", d.samples.len());
            }
            Err(e) => println!("== {path}: {e}"),
        }
    }
}
