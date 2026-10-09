//! `sdk_render <in.dng> <profile.dcp | -> <max-size> <out.ppm>`: the DNG SDK's reference render
//! (`dng_render`, sRGB, 16-bit PPM), optionally with an external camera profile. A diagnostic for
//! comparing LightCraft and Lightroom against Adobe's reference pipeline.
use std::io::Write;

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let a: Vec<String> = std::env::args().collect();
    if a.len() < 5 {
        return Err("usage: sdk_render <in.dng> <profile.dcp|-> <max-size> <out.ppm>".into());
    }
    let dng = std::fs::read(&a[1])?;
    let dcp = if a[2] == "-" { None } else { Some(std::fs::read(&a[2])?) };
    let max: u32 = a[3].parse()?;
    let (w, h, px) = lightcraft_dng_sdk_sys::render_dng(&dng, dcp.as_deref(), max, false).map_err(|e| format!("{}: {e}", a[1]))?;
    let mut f = std::io::BufWriter::new(std::fs::File::create(&a[4])?);
    write!(f, "P6\n{w} {h}\n65535\n")?;
    for v in px {
        f.write_all(&((v.clamp(0.0, 1.0) * 65535.0 + 0.5) as u16).to_be_bytes())?;
    }
    Ok(())
}
