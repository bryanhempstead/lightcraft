//! `oracle_render <out-dir> <file.dng>…`: render DNGs that carry Camera Raw settings in their XMP
//! (as Lightroom / Camera Raw would read them) with LightCraft at full size, linear ProPhoto
//! float, to `<out-dir>/<stem>.f32` (`[w, h]` as u32 then w·h·3 f32, little-endian). Bryan's fork:
//! the LightCraft side of the Camera Raw oracle (`docs/lr-match.md` → Round 4).
use lightcraft_pipeline::{OutputDepth, OutputSpace, RenderRequest};
use std::io::Write;

fn xmp_of(bytes: &[u8]) -> Option<String> {
    let s = String::from_utf8_lossy(bytes);
    let a = s.find("<x:xmpmeta")?;
    let b = s[a..].find("</x:xmpmeta>")? + a + "</x:xmpmeta>".len();
    Some(s[a..b].to_string())
}

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let (out, files) = args.split_first().ok_or("usage: oracle_render <out-dir> <file.dng>…")?;
    std::fs::create_dir_all(out)?;
    for f in files {
        let bytes = std::fs::read(f)?;
        // a sidecar (`name.xmp`) wins over the file's own XMP, as in Camera Raw for raws
        let sidecar = std::path::Path::new(f).with_extension("xmp");
        let xmp = std::fs::read_to_string(&sidecar).ok().or_else(|| xmp_of(&bytes)).ok_or("no XMP")?;
        let props = lightcraft_meta::parse_xmp(&xmp).map_err(|e| e.to_string())?.properties;
        let mut partial = lightcraft_engine::crs::to_partial(&props, Some(true));
        // a creative look (its RGB table carried in the XMP)
        if let Ok(Some(p)) = lightcraft_engine::crs_table::profile_from_xmp(&xmp) {
            let id = format!("lut:{}", p.name);
            lightcraft_pipeline::lut::register(&id, p.lut);
            let amount = props.get("crs:Look/crs:Amount").and_then(|v| v.first()).and_then(|a| a.parse::<f64>().ok()).unwrap_or(1.0);
            partial["profile"] = serde_json::json!({"id": id, "amount": amount * 100.0});
        }
        let s = lightcraft_develop::apply_partial(&lightcraft_develop::DevelopSettings::default(), &partial, 1.0);
        // ORACLE_SIZE: the long edge (Camera Raw's "minimum" size is 1536), else full size
        let edge = std::env::var("ORACLE_SIZE").ok().and_then(|v| v.parse::<usize>().ok());
        let (img, info) = lightcraft_engine::files::load_bytes(&bytes, usize::MAX)?;
        let (w, h) = match edge {
            Some(e) if img.width.max(img.height) > e => {
                let k = e as f64 / img.width.max(img.height) as f64;
                ((img.width as f64 * k).round() as usize, (img.height as f64 * k).round() as usize)
            }
            _ => (img.width, img.height),
        };
        let req = RenderRequest { space: OutputSpace::ProPhoto, depth: OutputDepth::F32Linear, ..RenderRequest::fit(w, h) };
        let r = lightcraft_pipeline::render(&std::sync::Arc::new(img), &info, &s, &req);
        let Some(deep) = r.deep else { return Err("no deep render".into()) };
        let lightcraft_pipeline::output::DeepSamples::F32(v) = deep.samples else { return Err("not float".into()) };
        let stem = std::path::Path::new(f).file_stem().and_then(|s| s.to_str()).unwrap_or("out");
        let mut o = std::io::BufWriter::new(std::fs::File::create(format!("{out}/{stem}.f32"))?);
        o.write_all(&(deep.width as u32).to_le_bytes())?;
        o.write_all(&(deep.height as u32).to_le_bytes())?;
        for x in v {
            o.write_all(&x.to_le_bytes())?;
        }
    }
    Ok(())
}
