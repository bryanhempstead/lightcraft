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
pub const CONTAINERS: [&str; 3] = ["crs:RetouchAreas", "crs:RetouchInfo", "crs:PointColors"];
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
        // content-aware remove (and anything newer)
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

/// Point Color samples (`crs:PointColors`): the sampled colour (`SrcHue` in radians, `SrcSat`,
/// `SrcLum` 0..1), its shifts (`HueShift`, `SatScale`, `LumScale`, −1..1), `RangeAmount` and the
/// hue / saturation / luminance ranges (`LowerNone`…`UpperNone`). Approximate: the sample is read
/// as an sRGB HSL colour and placed in our OkLCh terms.
pub fn point_colors(values: &Values) -> Vec<Value> {
    let Some(list) = values.get("crs:PointColors") else { return Vec::new() };
    let mut out = Vec::new();
    for p in items(list).iter().take(64) {
        let get = |k: &str| num(p, &format!("crs:{k}"));
        let (Some(h), Some(sat), Some(lum)) = (get("SrcHue"), get("SrcSat"), get("SrcLum")) else { continue };
        let deg = h.to_degrees().rem_euclid(360.0);
        let rgb = lightcraft_color::perceptual::hsl_to_rgb(deg as f32, sat.clamp(0.0, 1.0) as f32, lum.clamp(0.0, 1.0) as f32);
        let lin = rgb.map(lightcraft_color::transfer::srgb_to_linear);
        let lab = lightcraft_color::perceptual::oklab_from_2020(lin);
        let chroma = (lab[1] * lab[1] + lab[2] * lab[2]).sqrt() as f64;
        let hue = (lab[2] as f64).atan2(lab[1] as f64).to_degrees().rem_euclid(360.0);
        let width = |k: &str| -> f64 {
            let r = p.field(&format!("crs:{k}"));
            match r.map(|r| (num(r, "crs:LowerNone"), num(r, "crs:UpperNone"))) {
                Some((Some(lo), Some(hi))) => ((hi - lo) * 50.0).clamp(0.0, 100.0),
                _ => 50.0,
            }
        };
        let pct = |k: &str| (get(k).unwrap_or(0.0) * 100.0).clamp(-100.0, 100.0);
        out.push(json!({
            "lum": (lab[0] as f64).clamp(0.0, 1.0),
            "chroma": chroma.clamp(0.0, 0.4),
            "hue": hue,
            "hue_shift": pct("HueShift"),
            "sat_shift": pct("SatScale"),
            "lum_shift": pct("LumScale"),
            "range": (get("RangeAmount").unwrap_or(0.5) * 100.0).clamp(0.0, 100.0),
            "hue_range": width("HueRange"),
            "sat_range": width("SatRange"),
            "lum_range": width("LumRange"),
        }));
    }
    out
}
