//! Enhance commands: `photo.enhance` (Photo ▸ Enhance…, run synchronously here; the desktop dialog runs the same
//! [`crate::enhance::EnhanceJob`] on a worker thread), `enhance.preview`, `enhance.status`, `enhance.remove`,
//! `enhance.depth` and the model downloads (`enhance.models`, `enhance.model.download`, `enhance.model.cancel`).

use lightcraft_catalog::PhotoId;
use serde_json::{Value, json};

use super::{CommandSpec, bad, bool_or, cmd, str_param};
use crate::{Result, Session};

fn can_enhance(s: &Session) -> std::result::Result<(), String> {
    if !crate::enhance::AVAILABLE {
        return Err("Enhance is not available in this build".into());
    }
    if s.selection.ids.is_empty() { Err("select a raw photo".into()) } else { Ok(()) }
}

fn always(_: &Session) -> std::result::Result<(), String> {
    Ok(())
}

fn preview(s: &mut Session, p: &Value) -> Result<Value> {
    let c = "enhance.preview";
    let opts = crate::enhance::parse_options(c, p)?;
    let id = p.get("id").and_then(Value::as_u64).map(PhotoId).or_else(|| s.active()).ok_or_else(|| bad(c, "no photo"))?;
    let rect = match p.get("rect").and_then(Value::as_array) {
        Some(a) if a.len() == 4 => {
            let v: Vec<f64> = a.iter().filter_map(Value::as_f64).collect();
            [v.first().copied().unwrap_or(0.4), v.get(1).copied().unwrap_or(0.4), v.get(2).copied().unwrap_or(0.6), v.get(3).copied().unwrap_or(0.6)]
        }
        _ => [0.4, 0.4, 0.6, 0.6],
    };
    let side = p.get("side").and_then(Value::as_u64).unwrap_or(512) as usize;
    let job = s.plan_enhance(opts, &[id], false, false, Some((rect, side)))?;
    let out = job.run(&|_, _| true).map_err(|e| bad(c, e))?;
    let mut info = match out.photos.into_iter().next() {
        Some(Ok(p)) => p.info,
        Some(Err((_, e))) => return Err(bad(c, e)),
        None => json!({}),
    };
    if let Some((before, after)) = &out.preview {
        info["previewSize"] = json!([after.width, after.height]);
        for (key, img) in [("beforePath", before), ("afterPath", after)] {
            if let Some(path) = str_param(p, key) {
                let png = lightcraft_codecs::encode_png(&lightcraft_codecs::EncodeImage::rgba8(img), &lightcraft_codecs::EncodeMeta::default())
                    .map_err(|e| bad(c, e.to_string()))?;
                s.check_write_target(path).map_err(|e| bad(c, e))?;
                crate::export::write_file(path, &png).map_err(|e| bad(c, e))?;
                info[key] = json!(path);
            }
        }
    }
    Ok(info)
}

/// Edit the active photo's lens blur.
fn lens_blur_edit(s: &mut Session, c: &str, label: &str, f: impl FnOnce(&mut lightcraft_develop::DevelopSettings) -> Result<()>) -> Result<Value> {
    let id = s.active().ok_or_else(|| bad(c, "no active photo"))?;
    let mut d = (*s.develop_of(id).ok_or_else(|| bad(c, "no active photo"))?).clone();
    f(&mut d)?;
    let out = json!({"lensBlur": d.lens_blur, "hasDepth": d.depth.is_some()});
    s.set_develop(id, d, label)?;
    Ok(out)
}

/// The depth (0 nearest … 1 farthest) at the nearest tenth of the photo: where Lens Blur focuses first (the
/// subject is usually the nearest thing).
fn near_focus(d: &lightcraft_develop::DevelopSettings) -> Option<f64> {
    let g = d.depth.as_ref()?.grid()?;
    let mut v = g.data.clone();
    v.sort_by(f32::total_cmp);
    v.get(v.len() / 10).map(|x| f64::from(*x))
}

fn has_active(s: &Session) -> std::result::Result<(), String> {
    if s.active().is_some() { Ok(()) } else { Err("no active photo".into()) }
}

fn stroke_of(p: &Value) -> lightcraft_develop::BrushStroke {
    let pts = p
        .get("points")
        .and_then(Value::as_array)
        .map(|a| a.iter().filter_map(|q| Some(lightcraft_geom::Point::new(q.get(0)?.as_f64()?, q.get(1)?.as_f64()?))).collect())
        .unwrap_or_default();
    let num = |k: &str, d: f64| p.get(k).and_then(Value::as_f64).filter(|v| v.is_finite()).unwrap_or(d);
    lightcraft_develop::BrushStroke {
        points: pts,
        size: num("size", 0.03).clamp(0.001, 0.5),
        feather: num("feather", 50.0).clamp(0.0, 100.0),
        flow: num("flow", 100.0).clamp(0.0, 100.0),
        density: num("density", 100.0).clamp(0.0, 100.0),
        erase: p.get("erase").and_then(Value::as_bool).unwrap_or(false),
        auto_mask: false,
    }
}

pub fn specs() -> Vec<CommandSpec> {
    vec![
        cmd!(
            "enhance.depth",
            "Estimate Depth",
            [],
            None,
            "{ids?} — estimate each photo's depth map (Depth Anything V2 Small; needs its model: enhance.model.download model=depth) for Lens Blur and Depth Range masks → {photos: [{id, width, height, seconds}]}",
            super::has_selection,
            |s, p| {
                let ids: Vec<PhotoId> = s.targets(p);
                let mut out = Vec::new();
                for id in ids {
                    out.push(s.estimate_depth(id)?);
                }
                Ok(json!({"photos": out}))
            }
        ),
        cmd!(
            query "enhance.depthHistogram",
            "Depth Histogram",
            [],
            None,
            "{id?, bins=64} — how much of the photo lies at each depth (0 nearest … 1 farthest) → {bins: [fraction…]} (null without a depth map)",
            always,
            |s, p| {
                let id = p.get("id").and_then(Value::as_u64).map(PhotoId).or_else(|| s.active());
                let bins = p.get("bins").and_then(Value::as_u64).unwrap_or(64) as usize;
                let h = id.and_then(|id| s.develop_of(id)).and_then(|d| d.depth.as_ref().and_then(|m| m.grid())).map(|g| g.histogram(bins));
                Ok(json!({"bins": h}))
            }
        ),
        cmd!(
            "lensBlur.apply",
            "Apply Lens Blur",
            [],
            None,
            "{on=true} — switch Lens Blur on (estimating the depth map when the photo has none; first time at Lightroom's start: amount 50, focus on the nearest subject) or off",
            has_active,
            |s, p| {
                let on = bool_or(p, "on", true);
                let id = s.active().ok_or_else(|| bad("lensBlur.apply", "no active photo"))?;
                if on {
                    s.ensure_depth(id)?;
                }
                lens_blur_edit(s, "lensBlur.apply", if on { "Apply Lens Blur" } else { "Lens Blur Off" }, |d| {
                    d.lens_blur.enabled = on;
                    if on && d.lens_blur.amount == 0.0 {
                        d.lens_blur.amount = 50.0;
                    }
                    if on && d.lens_blur.focal_range == 0.0 && d.lens_blur.focal_distance == 0.0 {
                        let near = near_focus(d).unwrap_or(0.1);
                        d.lens_blur.focal_distance = (near * 100.0).clamp(0.0, 100.0);
                        d.lens_blur.focal_range = 15.0;
                    }
                    Ok(())
                })
            }
        ),
        cmd!(
            "lensBlur.focusAt",
            "Lens Blur Focus",
            [],
            None,
            "{x, y} normalised photo point — focus on what is there (Lightroom's focus picker: the focal range centres on its depth)",
            has_active,
            |s, p| {
                let c = "lensBlur.focusAt";
                let (x, y) =
                    (p.get("x").and_then(Value::as_f64).ok_or_else(|| bad(c, "x"))?, p.get("y").and_then(Value::as_f64).ok_or_else(|| bad(c, "y"))?);
                let id = s.active().ok_or_else(|| bad(c, "no active photo"))?;
                s.ensure_depth(id)?;
                lens_blur_edit(s, c, "Lens Blur Focus", |d| {
                    let g = d.depth.as_ref().and_then(|m| m.grid()).ok_or_else(|| bad(c, "the photo has no depth map"))?;
                    d.lens_blur.focal_distance = (f64::from(g.sample(x, y)) * 100.0).clamp(0.0, 100.0);
                    if d.lens_blur.focal_range == 0.0 {
                        d.lens_blur.focal_range = 15.0;
                    }
                    Ok(())
                })
            }
        ),
        cmd!("lensBlur.bokeh", "Bokeh Shape", [], None, "{shape: circle|bubble|fiveBlade|ring}", has_active, |s, p| {
            let c = "lensBlur.bokeh";
            let shape: lightcraft_develop::Bokeh =
                serde_json::from_value(p.get("shape").cloned().unwrap_or(json!("circle"))).map_err(|e| bad(c, e.to_string()))?;
            lens_blur_edit(s, c, "Bokeh", |d| {
                d.lens_blur.bokeh = shape;
                Ok(())
            })
        }),
        cmd!(
            "lensBlur.brush",
            "Lens Blur Refine Brush",
            [],
            None,
            "{kind: focus|blur, points: [[x, y]…] normalised, size=0.03 (of the long edge), feather=50, flow=100, density=100, erase=false} — Refine: paint what stays sharp (focus) or what is fully blurred (blur)",
            has_active,
            |s, p| {
                let c = "lensBlur.brush";
                let blur = match str_param(p, "kind").unwrap_or("focus") {
                    "focus" => false,
                    "blur" => true,
                    k => return Err(bad(c, format!("kind: focus or blur, not `{k}`"))),
                };
                let st = stroke_of(p);
                if st.points.is_empty() {
                    return Err(bad(c, "points: at least one [x, y]"));
                }
                lens_blur_edit(s, c, "Lens Blur Brush", |d| {
                    let v = if blur { &mut d.lens_blur.blur_strokes } else { &mut d.lens_blur.focus_strokes };
                    if v.len() >= 512 {
                        return Err(bad(c, "too many strokes"));
                    }
                    v.push(st);
                    Ok(())
                })
            }
        ),
        cmd!(
            "lensBlur.clearBrushes",
            "Clear Lens Blur Brushes",
            [],
            None,
            "{kind?: focus|blur} — remove the Refine strokes (both kinds without `kind`)",
            has_active,
            |s, p| {
                let kind = str_param(p, "kind").map(str::to_string);
                lens_blur_edit(s, "lensBlur.clearBrushes", "Clear Lens Blur Brushes", |d| {
                    if kind.as_deref() != Some("blur") {
                        d.lens_blur.focus_strokes.clear();
                    }
                    if kind.as_deref() != Some("focus") {
                        d.lens_blur.blur_strokes.clear();
                    }
                    Ok(())
                })
            }
        ),
        cmd!(
            "photo.enhance",
            "Enhance",
            [],
            None,
            "{ids?, denoise: 0..100 (or true = 50), rawDetails=false, superResolution=false, inPlace=false, stack=true} — Lightroom's Photo ▸ Enhance: AI Denoise (RawNIND when downloaded, else the built-in denoiser), Raw Details (RCD demosaic), Super Resolution (2×; needs its model). Writes `<name>-Enhanced-NR.dng` (‑RD, ‑SR, ‑NR‑SR) next to each raw, imported with its settings and stacked on top; inPlace keeps the result in the Enhance cache and sets the photo's `enhance` settings instead → {photos: [{id, newId?, path?, seconds, denoiser}]}",
            can_enhance,
            |s, p| {
                let opts = crate::enhance::parse_options("photo.enhance", p)?;
                let ids: Vec<PhotoId> = s.targets(p);
                s.enhance_now(opts, &ids, bool_or(p, "inPlace", false), bool_or(p, "stack", true))
            }
        ),
        cmd!(
            query "enhance.preview",
            "Enhance Preview",
            [],
            None,
            "{id?, denoise, rawDetails, superResolution, rect=[x0, y0, x1, y1] (normalised, the unrotated photo; default the middle fifth), side=512, beforePath?, afterPath?} — that part of the photo before and after Enhance, rendered with its settings (PNG files) → {previewSize, seconds}",
            can_enhance,
            preview
        ),
        cmd!(
            query "enhance.status",
            "Enhance Status",
            [],
            None,
            "{ids?} — each photo's in-place Enhance (options, whether its result is in the cache) and the models → {photos, models}",
            always,
            |s, p| {
                let ids: Vec<PhotoId> = s.targets(p);
                Ok(s.enhance_status(&ids))
            }
        ),
        cmd!(
            "enhance.remove",
            "Remove Enhance",
            [],
            None,
            "{ids?} — stop rendering the photos from their in-place Enhance result (undoable; the cache file stays until cleared)",
            super::has_selection,
            |s, p| {
                let ids: Vec<PhotoId> = s.targets(p);
                let mut n = 0;
                for id in ids {
                    if let Some(d) = s.develop_of(id)
                        && crate::enhance::options_of(&d).any()
                    {
                        let mut d = (*d).clone();
                        d.enhance = Default::default();
                        s.media.forget(id);
                        s.set_develop(id, d, "Remove Enhance")?;
                        n += 1;
                    }
                }
                Ok(json!({"removed": n}))
            }
        ),
        cmd!(
            query "enhance.models",
            "Enhance Models",
            [],
            None,
            "{} — the Enhance and Generative Remove models (raw-denoise, super-resolution, depth, inpaint): licence, size, installed, download progress",
            always,
            |s, _| Ok(s.enhancer.models_status())
        ),
        cmd!(
            query "enhance.model.download",
            "Download Enhance Model",
            [],
            None,
            "{model: raw-denoise|super-resolution|depth|inpaint, acknowledged: true} — download that model in the background (only after the user agreed to its licence and size; see enhance.models); watch enhance.models → {started}",
            always,
            |s, p| {
                let c = "enhance.model.download";
                let key = str_param(p, "model").ok_or_else(|| bad(c, "model: raw-denoise, super-resolution, depth or inpaint"))?;
                let id = lightcraft_enhance::models::by_key(key).ok_or_else(|| bad(c, format!("unknown model `{key}`")))?;
                let m = lightcraft_enhance::models::info(id);
                if p.get("acknowledged").and_then(Value::as_bool) != Some(true) {
                    return Err(bad(
                        c,
                        format!(
                            "{} is a {:.0} MB download under {} ({}), not LightCraft's licence: pass `acknowledged: true` once the user has agreed",
                            m.name,
                            m.size as f64 / 1e6,
                            m.licence,
                            m.licence_url
                        ),
                    ));
                }
                let started = s.enhancer.start_download(key).map_err(|e| bad(c, e))?;
                Ok(json!({"started": started}))
            }
        ),
        cmd!(
            query "enhance.model.cancel",
            "Cancel Enhance Model Download",
            [],
            None,
            "{model} — stop that download (what has arrived is kept; a new download resumes) → {cancelled}",
            always,
            |s, p| Ok(json!({"cancelled": s.enhancer.cancel_download(str_param(p, "model").unwrap_or(""))}))
        ),
    ]
}
