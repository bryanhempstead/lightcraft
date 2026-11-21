//! `cargo run --release -p lightcraft-raw --example decodecheck -- FILE...` (or file names on stdin)
//!
//! One line per file: which decoder read it (LightCraft's own, or LibRaw as the fallback), the
//! decode time, sizes and the error when neither could; then a summary. `LIGHTCRAFT_RAW_DECODER`
//! (`libraw`, `nolibraw`) changes the order, e.g. to time LibRaw on files the native decoders read.
use std::io::BufRead;
use std::time::Instant;

fn main() {
    let mut files: Vec<String> = std::env::args().skip(1).collect();
    if files.is_empty() {
        files = std::io::stdin().lock().lines().map_while(Result::ok).filter(|l| !l.is_empty()).collect();
    }
    let mut ok = std::collections::BTreeMap::<String, (usize, f64, f64)>::new();
    let mut failed = 0usize;
    for f in &files {
        let Ok(bytes) = std::fs::read(f) else {
            println!("{f}\tunreadable");
            continue;
        };
        let t = Instant::now();
        let r = lightcraft_raw::decode_reporting(&bytes);
        let ms = t.elapsed().as_secs_f64() * 1000.0;
        match r {
            Ok((img, by)) => {
                let mp = (img.width * img.height) as f64 / 1e6;
                let (cw, ch) = img.info().developed_size();
                println!(
                    "{f}\t{by:?}\t{ms:.0} ms\t{:?} {} {}\t{}x{} → {cw}x{ch} {:?} cfa {} white {} black {:.0}",
                    img.format,
                    img.metadata.make.as_deref().unwrap_or("?"),
                    img.metadata.model.as_deref().unwrap_or("?"),
                    img.width,
                    img.height,
                    img.orientation,
                    img.cfa.as_ref().map(|c| c.name()).unwrap_or_else(|| format!("none (cpp {})", img.cpp)),
                    img.white_at(0),
                    img.black.mean()
                );
                let e = ok.entry(format!("{by:?}")).or_default();
                e.0 += 1;
                e.1 += ms;
                e.2 += mp;
            }
            Err(e) => {
                failed += 1;
                println!("{f}\tFAILED\t{ms:.0} ms\t{e}");
            }
        }
    }
    for (by, (n, ms, mp)) in &ok {
        println!("# {by}: {n} files, mean {:.0} ms ({:.0} ms per 20 MP)", ms / *n as f64, ms / mp * 20.0);
    }
    println!("# failed: {failed}; LibRaw built in: {}", lightcraft_raw::libraw_available());
}
