//! `oracle_render <out-dir> <file.dng>…`: render DNGs that carry Camera Raw settings in their XMP
//! (as Lightroom / Camera Raw would read them) with LightCraft at full size, linear ProPhoto
//! float, to `<out-dir>/<stem>.f32` (`[w, h]` as u32 then w·h·3 f32, little-endian). Bryan's fork:
//! the LightCraft side of the Camera Raw oracle (`docs/lr-match.md` → Round 4). `ORACLE_DEPTH=hdr`
//! renders an HDR edit's float HDR master (`docs/lr-match.md` → HDR tone) instead of its SDR rendition.
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
        let data = lightcraft_meta::parse_xmp(&xmp).map_err(|e| e.to_string())?;
        let props = data.properties;
        // the structured containers too (masks, spots, Point Color), as a sidecar import reads them
        let (img, info) = lightcraft_engine::files::load_bytes(&bytes, usize::MAX)?;
        let aspect = img.width as f64 / img.height.max(1) as f64;
        let (mut partial, _) = lightcraft_engine::crs::to_partial_report(&props, Some(&data.values), Some(true), aspect);
        // a creative look (its RGB table carried in the XMP)
        if let Ok(Some(p)) = lightcraft_engine::crs_table::profile_from_xmp(&xmp) {
            let id = format!("lut:{}", p.name);
            lightcraft_pipeline::lut::register(&id, p.lut);
            let amount = props.get("crs:Look/crs:Amount").and_then(|v| v.first()).and_then(|a| a.parse::<f64>().ok()).unwrap_or(1.0);
            partial["profile"] = serde_json::json!({"id": id, "amount": amount * 100.0});
        }
        let mut s = lightcraft_develop::apply_partial(&lightcraft_develop::DevelopSettings::default(), &partial, 1.0);
        // ORACLE_SDR=1: the same edit with HDR off (its SDR twin)
        if std::env::var_os("ORACLE_SDR").is_some() {
            s.hdr.enabled = false;
        }
        // ORACLE_SIZE: the long edge (Camera Raw's "minimum" size is 1536), else full size
        let edge = std::env::var("ORACLE_SIZE").ok().and_then(|v| v.parse::<usize>().ok());
        let (w, h) = match edge {
            Some(e) if img.width.max(img.height) > e => {
                let k = e as f64 / img.width.max(img.height) as f64;
                ((img.width as f64 * k).round() as usize, (img.height as f64 * k).round() as usize)
            }
            _ => (img.width, img.height),
        };
        // ORACLE_DEPTH=hdr: an HDR edit's float HDR master (linear, SDR white = 1), not its SDR rendition
        let depth = match std::env::var("ORACLE_DEPTH").as_deref() {
            Ok("hdr") => OutputDepth::F32Hdr,
            _ => OutputDepth::F32Linear,
        };
        let img = std::sync::Arc::new(img);
        let stem = std::path::Path::new(f).file_stem().and_then(|s| s.to_str()).unwrap_or("out");
        // ORACLE_EXPORT=jpg|avif: also the HDR export (gain map, Display P3 base) as the app writes it
        if let (Ok(fmt), true) = (std::env::var("ORACLE_EXPORT"), s.hdr.enabled) {
            use lightcraft_engine::export::{ExportFormat, ExportOptions, encode_gain_map_jpeg};
            let format = if fmt == "avif" { ExportFormat::Avif } else { ExportFormat::Jpeg };
            let o = ExportOptions { format, hdr: true, hdr_compat: true, color_space: OutputSpace::DisplayP3, quality: 95, ..Default::default() };
            let base = RenderRequest { space: o.effective_space(), ..RenderRequest::fit(w, h) };
            let sdr = lightcraft_pipeline::render(&img, &info, &s, &base);
            let hdr = lightcraft_pipeline::render(&img, &info, &s, &RenderRequest { depth: OutputDepth::F32Hdr, ..base });
            let hdr = hdr.deep.ok_or("no HDR render")?;
            std::fs::write(format!("{out}/{stem}.{fmt}"), encode_gain_map_jpeg(&sdr.image, &hdr, &o, None)?)?;
        }
        let req = RenderRequest { space: OutputSpace::ProPhoto, depth, ..RenderRequest::fit(w, h) };
        let r = lightcraft_pipeline::render(&img, &info, &s, &req);
        let Some(deep) = r.deep else { return Err("no deep render".into()) };
        let lightcraft_pipeline::output::DeepSamples::F32(v) = deep.samples else { return Err("not float".into()) };
        let mut o = std::io::BufWriter::new(std::fs::File::create(format!("{out}/{stem}.f32"))?);
        o.write_all(&(deep.width as u32).to_le_bytes())?;
        o.write_all(&(deep.height as u32).to_le_bytes())?;
        for x in v {
            o.write_all(&x.to_le_bytes())?;
        }
    }
    Ok(())
}
