//! Interchange: spot removal (`crs:RetouchAreas`, and the older `crs:RetouchInfo`) → our
//! [`Spot`](lightcraft_develop::Spot)s.
//!
//! `RetouchAreas` holds one entry per spot: `SpotType` (`heal`, `clone`, or `heal_patchmatch` for
//! a content-aware remove), `Opacity` and `Feather` (0..1), the brushed path as a paint mask
//! (`Masks[0].Dabs`: `d x y` points and `r radius` changes, normalized) and, for heal / clone,
//! where the source is (`SourceX` / `OffsetY` hold the source position of the path's first point).
//! A circle spot's mask is a `Mask/Ellipse` (`X`, `Y`, `SizeX`, `SizeY`). `RetouchInfo` is the older
//! circle spot (kept as a copy of the same spots by newer versions): `centerX/Y`, `radius`, `sourceX/Y`, `opacity`,
//! `spotType`; in XMP each one is a `name = value, …` string.
//!
//! Implemented from black-box observation of the stored fields; best effort (the healing
//! algorithms differ).

use lightcraft_meta::XmpValue;
use serde_json::{Value, json};

use crate::crs_masks::Values;

/// The containers this module reads.
pub const CONTAINERS: [&str; 4] = ["crs:RetouchAreas", "crs:RetouchInfo", "crs:PointColors", "crs:ColorVariance"];
/// Spots read per photo at most (hostile input stays bounded).
const MAX_SPOTS: usize = 2000;
/// Path points kept per spot at most.
const MAX_POINTS: usize = 4000;

fn text<'a>(v: &'a XmpValue, k: &str) -> Option<&'a str> {
    v.field(k).and_then(XmpValue::text).map(str::trim).filter(|s| !s.is_empty())
}

fn num(v: &XmpValue, k: &str) -> Option<f64> {
    text(v, k)?.trim_start_matches('+').parse::<f64>().ok().filter(|x| x.is_finite())
}

fn items(v: &XmpValue) -> &[XmpValue] {
    match v {
        XmpValue::Array(a) => a,
        _ => &[],
    }
}

fn unit(v: f64) -> f64 {
    v.clamp(0.0, 1.0)
}

fn mode(kind: &str) -> &'static str {
    match kind.to_ascii_lowercase().as_str() {
        "clone" => "clone",
        "heal" => "heal",
        // content-aware remove: rebuilt from patches of the surroundings
        "heal_patchmatch" => "fill",
        // anything newer
        _ => "remove",
    }
}

/// One `RetouchAreas` entry.
fn area(a: &XmpValue) -> Option<Value> {
    let m = items(a.field("crs:Masks")?).first()?;
    let size = match (num(m, "crs:SizeX"), num(m, "crs:SizeY")) {
        (Some(a), Some(b)) => Some(a.max(b)),
        (a, b) => a.or(b),
    };
    let mut radius = num(m, "crs:Radius").or(size).unwrap_or(0.02);
    let mut points = Vec::new();
    // a circle spot (`Mask/Ellipse`): its centre
    if let (Some(x), Some(y)) = (num(m, "crs:X"), num(m, "crs:Y")) {
        points.push((unit(x), unit(y)));
    }
    for d in m.field("crs:Dabs").map(items).unwrap_or_default() {
        let Some(t) = d.text() else { continue };
        let mut it = t.split_whitespace();
        match it.next() {
            Some("d") => {
                let (Some(x), Some(y)) = (it.next().and_then(|v| v.parse::<f64>().ok()), it.next().and_then(|v| v.parse::<f64>().ok())) else {
                    continue;
                };
                if x.is_finite() && y.is_finite() && points.len() < MAX_POINTS {
                    points.push((unit(x), unit(y)));
                }
            }
            Some("r") => {
                if let Some(r) = it.next().and_then(|v| v.parse::<f64>().ok()).filter(|r| r.is_finite() && *r > 0.0) {
                    radius = radius.max(r);
                }
            }
            _ => {}
        }
    }
    let first = *points.first()?;
    let kind = mode(text(a, "crs:SpotType").unwrap_or("heal"));
    let source = match (kind, num(a, "crs:SourceX"), num(a, "crs:OffsetY")) {
        ("remove", ..) => Value::Null,
        (_, Some(sx), Some(sy)) => json!({"x": unit(sx) - first.0, "y": unit(sy) - first.1}),
        _ => Value::Null,
    };
    Some(json!({
        "mode": kind,
        "points": points.iter().map(|(x, y)| json!({"x": x, "y": y})).collect::<Vec<_>>(),
        "size": radius.clamp(0.0005, 0.5),
        "feather": (num(a, "crs:Feather").unwrap_or(0.5) * 100.0).clamp(0.0, 100.0),
        "opacity": (num(a, "crs:Opacity").unwrap_or(1.0) * 100.0).clamp(0.0, 100.0),
        "source_offset": source,
    }))
}

/// One `RetouchInfo` entry: a struct (Lua) or a `name = value, …` string (XMP).
fn info(v: &XmpValue) -> Option<Value> {
    let get = |k: &str| -> Option<f64> {
        match v {
            XmpValue::Text(t) => {
                t.split(',').filter_map(|kv| kv.split_once('=')).find(|(n, _)| n.trim() == k).and_then(|(_, x)| x.trim().parse().ok())
            }
            _ => num(v, &format!("crs:{k}")),
        }
        .filter(|x: &f64| x.is_finite())
    };
    let kind = match v {
        XmpValue::Text(t) => {
            t.split(',').filter_map(|kv| kv.split_once('=')).find(|(n, _)| n.trim() == "spotType").map(|(_, x)| x.trim().to_string())
        }
        _ => text(v, "crs:spotType").map(str::to_string),
    };
    let (cx, cy) = (unit(get("centerX")?), unit(get("centerY")?));
    let kind = mode(kind.as_deref().unwrap_or("heal"));
    let source = match (get("sourceX"), get("sourceY")) {
        (Some(sx), Some(sy)) if kind != "remove" => json!({"x": unit(sx) - cx, "y": unit(sy) - cy}),
        _ => Value::Null,
    };
    Some(json!({
        "mode": kind,
        "points": [{"x": cx, "y": cy}],
        "size": get("radius").unwrap_or(0.02).clamp(0.0005, 0.5),
        "feather": 50.0,
        "opacity": (get("opacity").unwrap_or(1.0) * 100.0).clamp(0.0, 100.0),
        "source_offset": source,
    }))
}

/// The spots in a packet's structured values, and how many entries could not be read.
pub fn spots(values: &Values) -> (Vec<Value>, usize) {
    let mut out = Vec::new();
    let mut skipped = 0;
    let areas = values.get("crs:RetouchAreas").map(items).unwrap_or_default();
    // newer versions keep the old list too, as a copy of the same spots
    let legacy = if areas.is_empty() { values.get("crs:RetouchInfo").map(items).unwrap_or_default() } else { &[] };
    for (list, read) in [(areas, area as fn(&XmpValue) -> Option<Value>), (legacy, info)] {
        for a in list {
            if out.len() >= MAX_SPOTS {
                skipped += 1;
                continue;
            }
            match read(a) {
                Some(s) => out.push(s),
                None => skipped += 1,
            }
        }
    }
    (out, skipped)
}

/// Point Color samples (`crs:PointColors`). Two forms carry the same numbers (RAWmakase's reading
/// of Camera Raw 18.7, confirmed on our oracle): XMP sidecars and presets write each swatch as 19
/// comma-separated values — the sample as HSV of linear ProPhoto (hue in sixths of a turn 0..6,
/// saturation, value), the Hue / Saturation / Luminance shifts (−1..1), Range (0..1), then the hue,
/// saturation and luminance ranges (outer, inner, inner, outer points) — with one Variance per
/// swatch in `crs:ColorVariance`; Lightroom's catalog stores the same fields by name (`SrcHue`,
/// `HueRange = { LowerNone, LowerFull, UpperFull, UpperNone }`, `Variance`). Lightroom's empty
/// selection (all −1) and samples outside what Camera Raw accepts are left out. The sample keeps
/// Lightroom's own values (`lr`, rendered with Camera Raw's model) and is also placed in our
/// OkLCh terms for the panel.
pub fn point_colors(values: &Values) -> Vec<Value> {
    let Some(list) = values.get("crs:PointColors") else { return Vec::new() };
    let variances: Vec<f64> =
        values.get("crs:ColorVariance").map(|v| items(v).iter().map(|i| i.text().and_then(parse_num).unwrap_or(0.0)).collect()).unwrap_or_default();
    let mut out = Vec::new();
    for (i, p) in items(list).iter().take(64).enumerate() {
        let swatch = match p.text() {
            Some(t) => from_numbers(t, variances.get(i).copied().unwrap_or(0.0)),
            None => from_fields(p),
        };
        if let Some(v) = swatch {
            out.push(v);
        }
    }
    out
}

fn parse_num(s: &str) -> Option<f64> {
    let s = s.trim();
    let s = s.strip_prefix('+').unwrap_or(s);
    s.parse::<f64>().ok().filter(|x| x.is_finite())
}

/// One swatch as the 19 numbers of the XMP form.
fn from_numbers(t: &str, variance: f64) -> Option<Value> {
    let n: Vec<f64> = t.split(',').filter(|v| !v.trim().is_empty()).map(parse_num).collect::<Option<Vec<_>>>()?;
    let r = |k: usize| -> Option<[f64; 4]> { Some([*n.get(k)?, *n.get(k + 1)?, *n.get(k + 2)?, *n.get(k + 3)?]) };
    let g = |k: usize| n.get(k).copied();
    swatch([g(0)?, g(1)?, g(2)?], [g(3)?, g(4)?, g(5)?], g(6)?, r(7)?, r(11)?, r(15)?, variance)
}

/// One swatch as named fields (the catalog's form).
fn from_fields(p: &XmpValue) -> Option<Value> {
    let get = |k: &str| num(p, &format!("crs:{k}"));
    let range = |k: &str, d: [f64; 4]| -> [f64; 4] {
        let f = p.field(&format!("crs:{k}"));
        let g = |n: &str, d: f64| f.and_then(|f| num(f, &format!("crs:{n}"))).unwrap_or(d);
        [g("LowerNone", d[0]), g("LowerFull", d[1]), g("UpperFull", d[2]), g("UpperNone", d[3])]
    };
    let src = [get("SrcHue")?, get("SrcSat")?, get("SrcLum")?];
    let around = |c: f64| [c - 0.73, c - 0.18, c + 0.18, c + 0.73].map(|v| v.clamp(0.0, 1.0));
    let enc = lightcraft_color::transfer::linear_to_srgb(src[2].clamp(0.0, 1.0) as f32) as f64;
    swatch(
        src,
        [get("HueShift").unwrap_or(0.0), get("SatScale").unwrap_or(0.0), get("LumScale").unwrap_or(0.0)],
        get("RangeAmount").unwrap_or(0.5),
        range("HueRange", [0.0, 1.0 / 3.0, 2.0 / 3.0, 1.0]),
        range("SatRange", around(src[1])),
        range("LumRange", around(enc)),
        get("Variance").unwrap_or(0.0),
    )
}

fn swatch(src: [f64; 3], shift: [f64; 3], range: f64, hue_r: [f64; 4], sat_r: [f64; 4], lum_r: [f64; 4], variance: f64) -> Option<Value> {
    // Lightroom's empty selection, and samples outside what Camera Raw accepts
    if !(0.0..6.0).contains(&src[0]) || !(0.0..=1.0).contains(&src[1]) || !(0.0..=1.0).contains(&src[2]) {
        return None;
    }
    // the sample in our OkLCh terms (the panel's swatch and LightCraft's own model)
    let h = (src[0] / 6.0 * 360.0) as f32;
    let pp = lightcraft_color::perceptual::hsv_to_rgb(h, src[1] as f32, src[2] as f32);
    let lin = lightcraft_color::PROPHOTO.to_space(&lightcraft_color::REC2020).apply_f32(pp);
    let lab = lightcraft_color::perceptual::oklab_from_2020(lin);
    let chroma = (lab[1] * lab[1] + lab[2] * lab[2]).sqrt() as f64;
    let hue = (lab[2] as f64).atan2(lab[1] as f64).to_degrees().rem_euclid(360.0);
    let pct = |v: f64| (v * 100.0).clamp(-100.0, 100.0);
    let width = |r: [f64; 4]| ((r[3] - r[0]) * 50.0).clamp(0.0, 100.0);
    Some(json!({
        "lum": (lab[0] as f64).clamp(0.0, 1.0),
        "chroma": chroma.clamp(0.0, 0.4),
        "hue": hue,
        "hue_shift": pct(shift[0]),
        "sat_shift": pct(shift[1]),
        "lum_shift": pct(shift[2]),
        "variance": pct(variance),
        "range": (range * 100.0).clamp(0.0, 100.0),
        "hue_range": width(hue_r),
        "sat_range": width(sat_r),
        "lum_range": width(lum_r),
        "lr": {"source": src, "hue_range": hue_r, "sat_range": sat_r, "lum_range": lum_r},
    }))
}

#[cfg(test)]
mod tests {
    use super::*;

    /// R19: Camera Raw's XMP Point Color (19 numbers + `crs:ColorVariance`) keeps Lightroom's
    /// sample and ranges; the empty selection (all −1) and malformed lists are left out.
    #[test]
    fn xmp_point_colors_keep_lightrooms_sample() {
        let x = r#"<x:xmpmeta xmlns:x="adobe:ns:meta/"><rdf:RDF xmlns:rdf="http://www.w3.org/1999/02/22-rdf-syntax-ns#">
          <rdf:Description rdf:about="" xmlns:crs="http://ns.adobe.com/camera-raw-settings/1.0/">
          <crs:PointColors><rdf:Seq>
            <rdf:li>4.000000, 0.600000, 0.250000, 0.000000, -1.000000, 0.500000, 0.500000, 0.000000, 0.333333, 0.666667, 1.000000, 0.000000, 0.420000, 0.780000, 1.000000, 0.000000, 0.357000, 0.717000, 1.000000</rdf:li>
            <rdf:li>-1.000000, -1.000000, -1.000000, -1.000000, -1.000000, -1.000000, -1.000000, -1.000000, -1.000000, -1.000000, -1.000000, -1.000000, -1.000000, -1.000000, -1.000000, -1.000000, -1.000000, -1.000000, -1.000000</rdf:li>
            <rdf:li>1, 2, nan</rdf:li>
          </rdf:Seq></crs:PointColors>
          <crs:ColorVariance><rdf:Seq><rdf:li>-0.300000</rdf:li><rdf:li>0</rdf:li></rdf:Seq></crs:ColorVariance>
          </rdf:Description></rdf:RDF></x:xmpmeta>"#;
        let d = lightcraft_meta::parse_xmp(x).unwrap();
        let p = point_colors(&d.values);
        assert_eq!(p.len(), 1, "{p:?}");
        assert_eq!(p[0]["sat_shift"], json!(-100.0));
        assert_eq!(p[0]["lum_shift"], json!(50.0));
        assert_eq!(p[0]["variance"], json!(-30.0));
        assert_eq!(p[0]["lr"]["source"], json!([4.0, 0.6, 0.25]));
        let pc: lightcraft_develop::PointColor = serde_json::from_value(p[0].clone()).unwrap();
        assert!(pc.lr.is_some_and(|l| (l.sat_range[1] - 0.42).abs() < 1e-9));
        // a blue sample (HSV hue 4 sixths of a turn of ProPhoto) sits at a blue OkLCh hue
        assert!((200.0..300.0).contains(&pc.hue), "{}", pc.hue);
    }

    /// R17: Lightroom's content-aware remove (`heal_patchmatch`) becomes a content-aware Fill spot.
    #[test]
    fn content_aware_remove_maps_to_fill() {
        assert_eq!(mode("heal_patchmatch"), "fill");
        assert_eq!((mode("Heal"), mode("clone"), mode("someday")), ("heal", "clone", "remove"));
        let spot: lightcraft_develop::Spot = serde_json::from_value(serde_json::json!({"mode": mode("heal_patchmatch")})).unwrap();
        assert_eq!(spot.mode, lightcraft_develop::SpotMode::Fill);
    }
}
