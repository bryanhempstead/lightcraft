//! `lcp_info <raw>…`: the Adobe lens profile LightCraft picks for each file and the corrections
//! it derives (Bryan's fork; diagnostics for `tools/lr-compare`).
fn main() {
    for path in std::env::args().skip(1) {
        let Ok(bytes) = std::fs::read(&path) else { continue };
        let Ok(raw) = lightcraft_raw::decode(&bytes) else {
            println!("{path}: not decodable");
            continue;
        };
        let m = &raw.metadata;
        let c = raw.crop.clipped(raw.active_area.width, raw.active_area.height);
        let (w, h) = if c.width > 1 { (c.width as f64, c.height as f64) } else { (raw.active_area.width as f64, raw.active_area.height as f64) };
        let found = m.lens_model.as_deref().and_then(|l| lightcraft_engine::lcp::find(m.make.as_deref(), m.model.as_deref(), l));
        println!("{path}: {:?} {:?} lens {:?} f {:?} N {:?} {w}x{h}", m.make, m.model, m.lens_model, m.focal_length, m.f_number);
        if let Some((p, e)) = found {
            println!("  lcp {} ({} entries)", p.display(), e.len());
            println!("  {:?}", lightcraft_engine::lcp::for_raw(m, w, h));
        }
    }
}
