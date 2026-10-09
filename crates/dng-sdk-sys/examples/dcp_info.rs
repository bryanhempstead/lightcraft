//! `dcp_info <profile.dcp> [neutral r g b | temp K tint]`: a camera profile's facts, matrices and (for a camera
//! neutral) its camera → XYZ D50 matrix, as JSON (diagnostics for tools/lr-compare).
fn main() -> Result<(), Box<dyn std::error::Error>> {
    let a: Vec<String> = std::env::args().collect();
    let p = lightcraft_dng_sdk_sys::Profile::parse(&std::fs::read(a.get(1).ok_or("usage: dcp_info <dcp> [r g b]")?)?).map_err(|e| e.to_string())?;
    let i = p.info().map_err(|e| e.to_string())?;
    let m = p.matrices().map_err(|e| e.to_string())?;
    if a.get(2).map(String::as_str) == Some("curve") {
        let xs: Vec<f64> = (0..=4096).map(|i| i as f64 / 4096.0).collect();
        let (ys, own) = p.tone_curve(&xs).map_err(|e| e.to_string())?;
        println!("{{\"own\":{own},\"ys\":{ys:?}}}");
        return Ok(());
    }
    let mut spec = None;
    if a.get(2).map(String::as_str) == Some("temp") && a.len() >= 5 {
        let xy = lightcraft_dng_sdk_sys::temp_tint_to_xy(a[3].parse()?, a[4].parse()?).map_err(|e| e.to_string())?;
        let s = p.color_spec(lightcraft_dng_sdk_sys::White::Xy(xy[0], xy[1]), None).map_err(|e| e.to_string())?;
        spec = Some(format!("{{\"white_xy\":{:?},\"camera_white\":{:?},\"camera_to_pcs\":{:?}}}", s.white_xy, s.camera_white, s.camera_to_pcs));
    } else if a.len() >= 5 {
        let n = [a[2].parse()?, a[3].parse()?, a[4].parse()?];
        let s = p.color_spec(lightcraft_dng_sdk_sys::White::Neutral(n), None).map_err(|e| e.to_string())?;
        spec = Some(format!("{{\"white_xy\":{:?},\"camera_white\":{:?},\"camera_to_pcs\":{:?}}}", s.white_xy, s.camera_white, s.camera_to_pcs));
    }
    println!(
        "{{\"model\":{:?},\"name\":{:?},\"illuminants\":{:?},\"cm1\":{:?},\"cm2\":{:?},\"fm1\":{:?},\"fm2\":{:?},\"spec\":{}}}",
        i.unique_model,
        i.name,
        i.illuminants,
        m[0],
        m[1],
        m[2],
        m[3],
        spec.unwrap_or_else(|| "null".into())
    );
    Ok(())
}
