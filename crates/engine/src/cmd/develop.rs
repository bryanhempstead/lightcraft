//! Develop commands. All operate on the active photo unless `ids` are given (sync).

use lightcraft_catalog::{Op, PhotoId, Version};
use lightcraft_develop::{DevelopSettings, Preset, Section, SettingsGroup, Treatment, Upright, WbMode, controls};
use lightcraft_geom::{CropGeometry, CropHandle, Point, Rect, crop_fit_angle, drag_crop};
use serde_json::{Value, json};

use super::copy_subsets::{groups_param, masks_param, merge_masks_param, paste_base};
use super::{CommandSpec, always, bad, bool_or, cmd, f64_or, f64_req, has_active, has_clipboard, has_selection, ok, str_param};
use crate::{Result, Session, media::SourceLevel};

fn active(s: &Session, c: &str) -> Result<PhotoId> {
    s.active().ok_or_else(|| bad(c, "no active photo"))
}

/// Modify the active photo's settings with `f` and commit (or preview during an interaction).
fn edit(s: &mut Session, c: &str, label: &str, f: impl FnOnce(&mut DevelopSettings) -> Result<()>) -> Result<Value> {
    let id = active(s, c)?;
    let mut d = (*s.develop_of(id).unwrap_or_default()).clone();
    f(&mut d)?;
    s.set_develop(id, d, label)?;
    ok()
}

fn section_param(p: &Value, c: &str) -> Result<Section> {
    let v = p.get("section").cloned().ok_or_else(|| bad(c, "missing `section`"))?;
    serde_json::from_value(v).map_err(|e| bad(c, e.to_string()))
}

/// Turn the crop to `angle`, keeping its size if it still fits, else shrinking it to fit.
fn straighten(d: &mut DevelopSettings, w: f64, h: f64, angle: f64) {
    let r = d.crop.geometry.rect_px(w, h);
    let fit = crop_fit_angle(w, h, angle, Some(r.width() / r.height()));
    let cand = CropGeometry { rect: d.crop.geometry.rect, angle };
    d.crop.geometry = if cand.is_within_image(w, h) { cand } else { fit };
}

/// The level angle for photo `id` with settings `d` (its horizon or plumb lines), if it has one.
fn level_angle(s: &mut Session, id: PhotoId, d: &DevelopSettings) -> Option<f64> {
    let src = s.source_now(id, SourceLevel::Preview).ok()?;
    let info = s.source_info(id);
    let level = lightcraft_pipeline::upright::level_degrees(&src, &info, d)?;
    Some((level.clamp(-45.0, 45.0) * 100.0).round() / 100.0)
}

/// How bright photo `id` comes out with settings `d`: the log-average linear luminance of a small
/// render (the finished look, after tone curve and every other setting), or `None` if it can't render.
pub(crate) fn rendered_brightness(s: &mut Session, id: PhotoId, d: &DevelopSettings) -> Option<f64> {
    let mut job = s.render_job(id, 256, 256, false, true)?;
    job.key ^= d.hash64();
    job.settings = std::sync::Arc::new(d.clone());
    (job.cache, job.stages, job.view_cache) = (None, None, None);
    let r = job.run();
    s.accept(&r);
    let img = r.rendered.ok()?.image;
    let lin = |v: u8| {
        let c = f64::from(v) / 255.0;
        if c <= 0.04045 { c / 12.92 } else { ((c + 0.055) / 1.055).powf(2.4) }
    };
    let n = img.data.len();
    let sum: f64 = img.data.iter().map(|px| (0.2126 * lin(px[0]) + 0.7152 * lin(px[1]) + 0.0722 * lin(px[2]) + 1e-4).ln()).sum();
    (n > 0).then(|| (sum / n as f64).exp()).filter(|b| b.is_finite() && *b > 0.0)
}

/// Move `d`'s Exposure until photo `id` renders as bright as `target` (render, correct, repeat:
/// the tone curve bends the response, so one step is not enough). `false`: it couldn't render.
fn match_brightness(s: &mut Session, id: PhotoId, d: &mut DevelopSettings, target: f64) -> bool {
    for _ in 0..6 {
        let Some(b) = rendered_brightness(s, id, d) else { return false };
        let step = (target / b).log2();
        if !step.is_finite() || step.abs() < 0.02 {
            break;
        }
        let next = (d.light.exposure + step).clamp(-5.0, 5.0);
        if (next - d.light.exposure).abs() < 1e-3 {
            break; // at the slider's end
        }
        d.light.exposure = (next * 100.0).round() / 100.0;
    }
    true
}

/// log2 of the light photo `id` gathered (shutter s × ISO ÷ f-number²), from its metadata.
fn gathered_ev(s: &Session, id: PhotoId) -> Option<f64> {
    let m = &s.catalog.photo(id)?.meta;
    let t = match m.shutter.split_once('/') {
        Some((a, b)) => a.trim().parse::<f64>().ok()? / b.trim().parse::<f64>().ok()?,
        None => m.shutter.trim().trim_end_matches('s').trim().parse::<f64>().ok()?,
    };
    let (n, iso) = (m.aperture? as f64, m.iso? as f64);
    (t > 0.0 && n > 0.0 && iso > 0.0).then(|| (t * iso / (n * n)).log2())
}

fn crop_dims(s: &Session, id: PhotoId) -> (f64, f64) {
    let p = s.catalog.photo(id);
    let (w, h) = p.map(|p| (p.width.max(1) as f64, p.height.max(1) as f64)).unwrap_or((3.0, 2.0));
    let o = p.map(|p| p.develop.orientation).unwrap_or_default();
    if o.swaps_axes() { (h, w) } else { (w, h) }
}

/// `v` as a list of numbers; `None` unless it is an array of nothing but numbers.
fn numbers(v: Option<&Value>) -> Option<Vec<f64>> {
    v?.as_array()?.iter().map(Value::as_f64).collect()
}

/// A width × height lock stored as hundredths. Rejects non-finite, non-positive and
/// extreme (beyond 20:1) shapes, which would give an empty or unusable crop.
fn aspect_tuple(c: &str, x: f64, y: f64) -> Result<(u32, u32)> {
    let ratio = x / y;
    if !(x.is_finite()
        && y.is_finite()
        && x > 0.0
        && y > 0.0
        && ratio.is_finite()
        && (1.0 / lightcraft_geom::MAX_RATIO..=lightcraft_geom::MAX_RATIO).contains(&ratio))
    {
        return Err(bad(c, "aspect needs two positive numbers with a ratio between 1:20 and 20:1"));
    }
    let to = |v: f64| (v * 100.0).round();
    let (a, b) = (to(x), to(y));
    if a < 1.0 || b < 1.0 || a > f64::from(u32::MAX) || b > f64::from(u32::MAX) {
        return Err(bad(c, "aspect numbers are out of range"));
    }
    Ok((a as u32, b as u32))
}

fn set_aspect(d: &mut DevelopSettings, w: f64, h: f64, aspect: Option<(u32, u32)>) {
    d.crop.aspect = aspect;
    let a = aspect.map(|(aw, ah)| {
        // orientation of the crop follows the image (landscape image → landscape crop)
        let (aw, ah) = (aw as f64, ah as f64);
        if (w >= h) == (aw >= ah) { aw / ah } else { ah / aw }
    });
    let angle = d.crop.geometry.angle;
    d.crop.geometry = crop_fit_angle(w, h, angle, a);
}

/// The user preset named by `p.id` (built-ins can't be changed).
fn user_preset<'a>(s: &'a mut Session, p: &Value, c: &str) -> Result<&'a mut Preset> {
    let pid = str_param(p, "id").ok_or_else(|| bad(c, "missing id"))?;
    let pr = s.presets.iter_mut().find(|x| x.id == pid).ok_or_else(|| bad(c, format!("unknown preset `{pid}`")))?;
    if pr.builtin {
        return Err(bad(c, "built-in presets can't be changed"));
    }
    Ok(pr)
}

/// Change version `p.index` of the active photo (`f` gets it and the current settings).
fn edit_version(s: &mut Session, p: &Value, c: &str, label: &str, f: impl FnOnce(&mut Version, std::sync::Arc<DevelopSettings>)) -> Result<Value> {
    let id = active(s, c)?;
    let i = f64_req(p, "index", c)? as usize;
    let ph = s.catalog.photo(id).ok_or_else(|| bad(c, "no photo"))?;
    let mut versions = ph.versions.clone();
    let v = versions.get_mut(i).ok_or_else(|| bad(c, "no such version"))?;
    f(v, ph.develop.clone());
    s.commit(label, Op::SetVersions { id, versions })?;
    ok()
}

pub fn specs() -> Vec<CommandSpec> {
    vec![
        cmd!(
            "develop.set",
            "Set Develop Values",
            [],
            None,
            "{control?: controlId, value?: number, values?: {controlId: number}, ids?: [photo]} — see develop.controls",
            has_active,
            |s, p| {
                let mut vals: Vec<(String, f64)> = Vec::new();
                if let (Some(c), Some(v)) = (str_param(p, "control"), p.get("value").and_then(Value::as_f64)) {
                    vals.push((c.to_string(), v));
                }
                if let Some(o) = p.get("values").and_then(Value::as_object) {
                    for (k, v) in o {
                        vals.push((k.clone(), v.as_f64().ok_or_else(|| bad("develop.set", format!("`{k}` must be a number")))?));
                    }
                }
                if vals.is_empty() {
                    return Err(bad("develop.set", "give `control`+`value` or `values`"));
                }
                for (k, _) in &vals {
                    if controls::find(k).is_none() {
                        return Err(bad("develop.set", format!("unknown control `{k}`")));
                    }
                }
                let label = if vals.len() == 1 { controls::find(&vals[0].0).map(|c| c.label).unwrap_or("Edit").to_string() } else { "Edit".into() };
                let apply = |d: &mut DevelopSettings, info: &lightcraft_pipeline::SourceInfo| {
                    if d.wb.mode == WbMode::AsShot && vals.iter().any(|(k, _)| k == "wb.temp" || k == "wb.tint") {
                        d.wb.temp = info.as_shot_temp;
                        d.wb.tint = info.as_shot_tint;
                    }
                    for (k, v) in &vals {
                        controls::set(d, k, *v);
                        if k == "wb.temp" || k == "wb.tint" {
                            d.wb.mode = WbMode::Custom;
                        }
                    }
                };
                if let Some(ids) = p.get("ids").and_then(Value::as_array) {
                    let ids: Vec<PhotoId> = ids.iter().filter_map(Value::as_u64).map(PhotoId).collect();
                    let ops = ids
                        .iter()
                        .filter_map(|id| s.develop_of(*id).map(|d| (*id, d)))
                        .filter_map(|(id, d)| {
                            let mut d = (*d).clone();
                            let info = s.source_info(id);
                            apply(&mut d, &info);
                            s.develop_op(id, d, &label)
                        })
                        .collect();
                    s.commit(&label, Op::Batch { ops })?;
                    return ok();
                }
                let id = active(s, "develop.set")?;
                let info = s.source_info(id);
                edit(s, "develop.set", &label, |d| {
                    apply(d, &info);
                    Ok(())
                })
            }
        ),
        cmd!("develop.adjust", "Nudge Develop Value", [], None, "{control, delta}", has_active, |s, p| {
            let c = str_param(p, "control").ok_or_else(|| bad("develop.adjust", "missing control"))?.to_string();
            let spec = controls::find(&c).ok_or_else(|| bad("develop.adjust", "unknown control"))?;
            let delta = f64_req(p, "delta", "develop.adjust")?;
            let info = s.source_info(active(s, "develop.adjust")?);
            edit(s, "develop.adjust", spec.label, |d| {
                if c == "wb.temp" || c == "wb.tint" {
                    if d.wb.mode == WbMode::AsShot {
                        d.wb.temp = info.as_shot_temp;
                        d.wb.tint = info.as_shot_tint;
                    }
                    d.wb.mode = WbMode::Custom;
                }
                let v = controls::get(d, &c).unwrap_or(spec.default);
                controls::set(d, &c, v + delta);
                Ok(())
            })
        }),
        cmd!("develop.merge", "Apply Settings JSON", [], None, "{settings: partial DevelopSettings JSON, label?}", has_active, |s, p| {
            let partial = p.get("settings").cloned().ok_or_else(|| bad("develop.merge", "missing settings"))?;
            let label = str_param(p, "label").unwrap_or("Edit").to_string();
            edit(s, "develop.merge", &label, |d| {
                *d = d.merged(&partial).map_err(|e| bad("develop.merge", e.to_string()))?;
                Ok(())
            })
        }),
        cmd!("develop.reset", "Reset Edits", ["Photo"], Some("Cmd+Shift+R"), "{ids?}", has_selection, |s, p| {
            let ids = s.targets(p);
            let ops = ids
                .iter()
                .filter_map(|id| s.develop_of(*id).map(|d| (*id, d)))
                .filter_map(|(id, _d)| {
                    // back to the photo's import defaults when a default preset gave it its look
                    let look = s.catalog.photo(id).and_then(|p| p.import_look.clone());
                    let fresh = match look {
                        Some(l) => (*l).clone(),
                        None => {
                            let info = s.source_info(id);
                            DevelopSettings {
                                wb: lightcraft_develop::WhiteBalance { mode: WbMode::AsShot, temp: info.as_shot_temp, tint: info.as_shot_tint },
                                ..Default::default()
                            }
                        }
                    };
                    s.develop_op(id, fresh, "Reset")
                })
                .collect();
            s.commit("Reset", Op::Batch { ops })?;
            ok()
        }),
        cmd!(
            "develop.resetSection",
            "Reset Section",
            [],
            None,
            "{section: light|curve|color|mixer|grading|effects|vignette|grain|detail|optics|geometry}",
            has_active,
            |s, p| {
                let sec = section_param(p, "develop.resetSection")?;
                edit(s, "develop.resetSection", &format!("Reset {}", sec.label()), |d| {
                    d.reset_section(sec);
                    Ok(())
                })
            }
        ),
        cmd!("develop.resetControl", "Reset Control", [], None, "{control}", has_active, |s, p| {
            let c = str_param(p, "control").ok_or_else(|| bad("develop.resetControl", "missing control"))?.to_string();
            let spec = controls::find(&c).ok_or_else(|| bad("develop.resetControl", "unknown control"))?;
            edit(s, "develop.resetControl", spec.label, |d| {
                controls::set(d, &c, spec.default);
                Ok(())
            })
        }),
        cmd!("develop.sectionEnabled", "Toggle Section", [], None, "{section: string, enabled: bool}", has_active, |s, p| {
            let sec = str_param(p, "section").ok_or_else(|| bad("develop.sectionEnabled", "missing section"))?.to_string();
            let on = bool_or(p, "enabled", true);
            edit(s, "develop.sectionEnabled", "Toggle Section", |d| {
                d.set_section_enabled(&sec, on);
                Ok(())
            })
        }),
        cmd!(
            "develop.autoBwMix",
            "Auto B&W Mix",
            [],
            None,
            "{} — sets the black & white mix from the photo's colours (and switches to B&W)",
            has_active,
            |s, _| {
                let id = active(s, "develop.autoBwMix")?;
                let src = s.source_now(id, SourceLevel::Thumb).map_err(|e| bad("develop.autoBwMix", e))?;
                let info = s.source_info(id);
                let d = s.develop_of(id).unwrap_or_default();
                let m = lightcraft_pipeline::auto::auto_bw_mix(&src, &info, &d);
                edit(s, "develop.autoBwMix", "Auto B&W Mix", |d| {
                    d.treatment = Treatment::Bw;
                    let b = &mut d.bw_mix;
                    [b.red, b.orange, b.yellow, b.green, b.aqua, b.blue, b.purple, b.magenta] = m;
                    Ok(())
                })?;
                Ok(json!({"mix": m}))
            }
        ),
        cmd!("develop.auto", "Auto Settings", ["Photo"], Some("Shift+A"), "{}", has_active, |s, _| {
            let id = active(s, "develop.auto")?;
            let src = s.source_now(id, SourceLevel::Thumb).map_err(|e| bad("develop.auto", e))?;
            let info = s.source_info(id);
            let d = s.develop_of(id).unwrap_or_default();
            let a = lightcraft_pipeline::auto::auto_tone(&src, &info, &d);
            edit(s, "develop.auto", "Auto", |d| {
                d.light.exposure = a.exposure;
                d.light.contrast = a.contrast;
                d.light.highlights = a.highlights;
                d.light.shadows = a.shadows;
                d.light.whites = a.whites;
                d.light.blacks = a.blacks;
                d.color.vibrance = a.vibrance;
                d.color.saturation = a.saturation;
                Ok(())
            })?;
            Ok(serde_json::to_value(a).unwrap_or_default())
        }),
        cmd!(
            "develop.wb",
            "White Balance",
            [],
            None,
            "{mode: asShot|auto|daylight|cloudy|shade|tungsten|fluorescent|flash|custom, temp?, tint?}",
            has_active,
            |s, p| {
                let mode: WbMode =
                    serde_json::from_value(p.get("mode").cloned().unwrap_or(json!("custom"))).map_err(|e| bad("develop.wb", e.to_string()))?;
                let id = active(s, "develop.wb")?;
                let info = s.source_info(id);
                let (mut t, mut tint) = match mode {
                    WbMode::AsShot => (info.as_shot_temp, info.as_shot_tint),
                    WbMode::Auto => {
                        let src = s.source_now(id, SourceLevel::Thumb).map_err(|e| bad("develop.wb", e))?;
                        lightcraft_pipeline::auto::auto_wb(&src, &info)
                    }
                    m => m
                        .preset()
                        .map(|(t, ti)| {
                            // presets are relative to daylight for rendered files
                            if info.raw && !info.relative_wb { (t, ti) } else { (6500.0 * t / 5500.0, ti) }
                        })
                        .unwrap_or((info.as_shot_temp, info.as_shot_tint)),
                };
                t = f64_or(p, "temp", t);
                tint = f64_or(p, "tint", tint);
                // The resolved temperature/tint is stored, so rendering never depends on the mode; the
                // mode is kept for the UI's WB dropdown.
                edit(s, "develop.wb", "White Balance", |d| {
                    d.wb.mode = mode;
                    d.wb.temp = t.clamp(2000.0, 50000.0);
                    d.wb.tint = tint.clamp(-150.0, 150.0);
                    Ok(())
                })?;
                Ok(json!({"temp": t, "tint": tint}))
            }
        ),
        cmd!("develop.wbPick", "White Balance Selector", [], Some("W"), "{x, y} normalized image coords of a neutral point", has_active, |s, p| {
            let (x, y) = (f64_req(p, "x", "develop.wbPick")?, f64_req(p, "y", "develop.wbPick")?);
            let id = active(s, "develop.wbPick")?;
            let src = s.source_now(id, SourceLevel::Thumb).map_err(|e| bad("develop.wbPick", e))?;
            let info = s.source_info(id);
            let d = s.develop_of(id).unwrap_or_default();
            // Sample a small neighbourhood in the oriented source.
            let o = src.oriented(d.orientation);
            let (cx, cy) = ((x.clamp(0.0, 1.0) * o.width as f64) as isize, (y.clamp(0.0, 1.0) * o.height as f64) as isize);
            let mut acc = [0.0f32; 3];
            for dy in -2..=2 {
                for dx in -2..=2 {
                    let c = o.get_clamped(cx + dx, cy + dy);
                    for i in 0..3 {
                        acc[i] += c[i] / 25.0;
                    }
                }
            }
            let patch = lightcraft_raster::Rgb32f::filled(4, 4, acc);
            let (t, tint) = lightcraft_pipeline::auto::auto_wb(&patch, &info);
            edit(s, "develop.wbPick", "White Balance", |d| {
                d.wb.mode = WbMode::Custom;
                d.wb.temp = t;
                d.wb.tint = tint;
                Ok(())
            })?;
            Ok(json!({"temp": t, "tint": tint}))
        }),
        cmd!(
            "develop.defringePick",
            "Fringe Color Selector",
            [],
            None,
            "{x, y} normalized image coords of a purple or green fringe: sets that Defringe hue range around the colour (and Amount 5 when it is 0) → {fringe: purple|green, amount, hueLo, hueHi}",
            has_active,
            |s, p| {
                let (x, y) = (f64_req(p, "x", "develop.defringePick")?, f64_req(p, "y", "develop.defringePick")?);
                let id = active(s, "develop.defringePick")?;
                // fringes are a few source pixels wide: the preview level, not a thumbnail
                let src = s.source_now(id, SourceLevel::Preview).map_err(|e| bad("develop.defringePick", e))?;
                let d = s.develop_of(id).unwrap_or_default();
                let o = src.oriented(d.orientation);
                let (cx, cy) = ((x.clamp(0.0, 1.0) * o.width as f64) as isize, (y.clamp(0.0, 1.0) * o.height as f64) as isize);
                let (kind, pos) = lightcraft_pipeline::optics::fringe_pick(&o, cx, cy)
                    .ok_or_else(|| bad("develop.defringePick", "the sampled colour is not a purple or green fringe"))?;
                // a 24-wide range centred on the colour (Lightroom keeps the two ends ≥ 10 apart)
                let lo = (pos - 12.0).clamp(0.0, 76.0);
                let hi = lo + 24.0;
                let mut out = json!({});
                edit(s, "develop.defringePick", "Defringe", |d| {
                    let o = &mut d.optics;
                    let (amount, name) = match kind {
                        lightcraft_pipeline::optics::Fringe::Purple => {
                            (o.defringe_purple_hue_lo, o.defringe_purple_hue_hi) = (lo, hi);
                            (&mut o.defringe_purple_amount, "purple")
                        }
                        lightcraft_pipeline::optics::Fringe::Green => {
                            (o.defringe_green_hue_lo, o.defringe_green_hue_hi) = (lo, hi);
                            (&mut o.defringe_green_amount, "green")
                        }
                    };
                    if *amount <= 0.0 {
                        *amount = 5.0;
                    }
                    out = json!({"fringe": name, "amount": *amount, "hueLo": lo, "hueHi": hi});
                    Ok(())
                })?;
                Ok(out)
            }
        ),
        cmd!("develop.treatment", "Black & White", ["Photo"], Some("V"), "{bw?: bool} (toggles when omitted)", has_active, |s, p| {
            edit(s, "develop.treatment", "Treatment", |d| {
                let bw = p.get("bw").and_then(Value::as_bool).unwrap_or(d.treatment != Treatment::Bw);
                d.treatment = if bw { Treatment::Bw } else { Treatment::Color };
                Ok(())
            })
        }),
        cmd!("develop.profile", "Set Profile", [], None, "{id: profile id (see profiles.list), amount?: 0..200}", has_active, |s, p| {
            let id = str_param(p, "id").ok_or_else(|| bad("develop.profile", "missing id"))?.to_string();
            if !crate::presets::PROFILES.iter().any(|x| x.id == id) && !s.lut_profiles.iter().any(|x| x.id == id) {
                return Err(bad("develop.profile", format!("unknown profile `{id}`")));
            }
            edit(s, "develop.profile", "Profile", |d| {
                d.profile.id = id.clone();
                d.profile.amount = f64_or(p, "amount", d.profile.amount).clamp(0.0, 200.0);
                Ok(())
            })?;
            s.note_profile_used(&id);
            ok()
        }),
        cmd!("profile.favorite", "Favorite Profile", [], None, "{id, favorite?: bool} (toggles when omitted)", always, |s, p| {
            let id = str_param(p, "id").ok_or_else(|| bad("profile.favorite", "missing id"))?.to_string();
            if crate::presets::profile(&id).is_none() && !s.lut_profiles.iter().any(|x| x.id == id) {
                return Err(bad("profile.favorite", format!("unknown profile `{id}`")));
            }
            let on = s.profile_favorites.contains(&id);
            let want = bool_or(p, "favorite", !on);
            if want && !on {
                s.profile_favorites.push(id);
            } else if !want {
                s.profile_favorites.retain(|f| *f != id);
            }
            Ok(json!({"favorite": want}))
        }),
        cmd!("develop.curve", "Set Point Curve", [], None, "{channel: master|red|green|blue, points: [[x,y],...] in 0..1}", has_active, |s, p| {
            let ch = str_param(p, "channel").unwrap_or("master").to_string();
            let pts: Vec<Point> = p
                .get("points")
                .and_then(Value::as_array)
                .ok_or_else(|| bad("develop.curve", "missing points"))?
                .iter()
                .filter_map(|q| Some(Point::new(q.get(0)?.as_f64()?.clamp(0.0, 1.0), q.get(1)?.as_f64()?.clamp(0.0, 1.0))))
                .collect();
            edit(s, "develop.curve", "Tone Curve", |d| {
                match ch.as_str() {
                    "master" | "rgb" => d.curve.master = pts,
                    "red" => d.curve.red = pts,
                    "green" => d.curve.green = pts,
                    "blue" => d.curve.blue = pts,
                    other => return Err(bad("develop.curve", format!("unknown channel `{other}`"))),
                }
                Ok(())
            })
        }),
        // ---- crop & rotate
        cmd!("crop.set", "Set Crop", [], None, "{rect?: [x0,y0,x1,y1] normalized in the straightened frame, angle?: degrees}", has_active, |s, p| {
            let id = active(s, "crop.set")?;
            let (w, h) = crop_dims(s, id);
            edit(s, "crop.set", "Crop", |d| {
                if let Some(a) = p.get("rect").and_then(Value::as_array) {
                    let v: Vec<f64> = a.iter().filter_map(Value::as_f64).collect();
                    if v.len() != 4 {
                        return Err(bad("crop.set", "rect needs 4 numbers"));
                    }
                    d.crop.geometry.rect = Rect::new(v[0].min(v[2]), v[1].min(v[3]), v[0].max(v[2]), v[1].max(v[3]));
                }
                if let Some(a) = p.get("angle").and_then(Value::as_f64) {
                    d.crop.geometry.angle = a.clamp(-45.0, 45.0);
                }
                d.crop.geometry = d.crop.geometry.constrained(w, h);
                Ok(())
            })
        }),
        cmd!(
            "crop.drag",
            "Drag Crop Handle",
            [],
            None,
            "{handle: topLeft|topRight|bottomRight|bottomLeft|top|right|bottom|left|move | 0..8, from: [x,y], to: [x,y] (normalized, straightened frame), start?: [x0,y0,x1,y1] (default: the current crop)} — drags a crop handle; stops at the image edge, keeps the opposite edges, honours the aspect lock",
            has_active,
            |s, p| {
                let id = active(s, "crop.drag")?;
                let (w, h) = crop_dims(s, id);
                let handle = match p.get("handle") {
                    Some(Value::String(n)) => CropHandle::from_name(n),
                    Some(v) => v.as_u64().and_then(|i| u8::try_from(i).ok()).and_then(CropHandle::from_index),
                    None => None,
                }
                .ok_or_else(|| bad("crop.drag", "unknown handle"))?;
                let point = |k: &str| -> Result<Point> {
                    match numbers(p.get(k)).as_deref() {
                        Some([x, y]) if x.is_finite() && y.is_finite() => Ok(Point::new(*x, *y)),
                        _ => Err(bad("crop.drag", format!("`{k}` needs [x, y]"))),
                    }
                };
                let (from, to) = (point("from")?, point("to")?);
                let given = match (p.get("start"), numbers(p.get("start")).as_deref()) {
                    (None | Some(Value::Null), _) => None,
                    (_, Some(&[x0, y0, x1, y1])) if [x0, y0, x1, y1].iter().all(|v| v.is_finite()) => {
                        Some(Rect::new(x0.min(x1), y0.min(y1), x0.max(x1), y0.max(y1)))
                    }
                    _ => return Err(bad("crop.drag", "`start` needs 4 numbers")),
                };
                edit(s, "crop.drag", "Crop", |d| {
                    let start = CropGeometry { rect: given.unwrap_or(d.crop.geometry.rect), angle: d.crop.geometry.angle };
                    // A locked crop keeps the shape it has: the lock stores a width:height tuple whose
                    // orientation is reinterpreted per image, so the frame itself is the truth.
                    let ratio = d.crop.aspect.and(Some(start.rect_px(w, h).aspect())).filter(|a| a.is_finite() && *a > 0.0);
                    d.crop.geometry = drag_crop(start, handle, from, to, w, h, ratio);
                    Ok(())
                })
            }
        ),
        cmd!(
            "crop.straighten",
            "Straighten",
            [],
            None,
            "{angle: degrees -45..45} — keeps the largest crop of the current aspect",
            has_active,
            |s, p| {
                let id = active(s, "crop.straighten")?;
                let (w, h) = crop_dims(s, id);
                let angle = f64_req(p, "angle", "crop.straighten")?.clamp(-45.0, 45.0);
                edit(s, "crop.straighten", "Straighten", |d| {
                    straighten(d, w, h, angle);
                    Ok(())
                })
            }
        ),
        cmd!(
            "crop.autoStraighten",
            "Auto Straighten",
            [],
            None,
            "{} — level the horizon (or plumb lines) with the crop angle",
            has_active,
            |s, _| {
                let id = active(s, "crop.autoStraighten")?;
                let src = s.source_now(id, SourceLevel::Preview).map_err(|e| bad("crop.autoStraighten", e))?;
                let info = s.source_info(id);
                let d = s.develop_of(id).unwrap_or_default();
                let Some(level) = lightcraft_pipeline::upright::level_degrees(&src, &info, &d) else {
                    return Ok(json!({"changed": false, "reason": "no dominant horizontal or vertical lines"}));
                };
                // Same convention as the crop angle (verified: a horizon descending 6° to the right
                // comes out level with angle = level ≈ −6°).
                let angle = level.clamp(-45.0, 45.0);
                s.execute("crop.straighten", &json!({"angle": (angle * 100.0).round() / 100.0}))?;
                Ok(json!({"changed": true, "angle": angle}))
            }
        ),
        cmd!(
            "crop.aspect",
            "Crop Aspect",
            [],
            None,
            "{aspect: \"original\"|\"asShot\" (the camera's own framing: raws are decoded inside the in-camera crop, so the same as original)|\"free\"|\"current\" (lock the crop's present shape)|\"toggle\" (lock ↔ free)|\"1x1\"|\"4x5\"|\"8.5x11\"|\"5x7\"|\"2x3\"|\"4x3\"|\"16x9\"|\"16x10\"|[w,h]}",
            has_active,
            |s, p| {
                let id = active(s, "crop.aspect")?;
                let (w, h) = crop_dims(s, id);
                let locked = s.develop_of(id).is_some_and(|d| d.crop.aspect.is_some());
                let lock_current = match p.get("aspect").and_then(Value::as_str) {
                    Some("current") => true,
                    Some("toggle") => !locked,
                    _ => false,
                };
                if lock_current {
                    // lock to the rectangle as it is: no refit
                    return edit(s, "crop.aspect", "Lock Crop Aspect", |d| {
                        let r = d.crop.geometry.rect_px(w, h);
                        d.crop.aspect = Some(((r.width() * 100.0).round().max(1.0) as u32, (r.height() * 100.0).round().max(1.0) as u32));
                        Ok(())
                    });
                }
                let aspect = match p.get("aspect") {
                    Some(Value::String(a)) if a == "toggle" => None,
                    Some(Value::String(a)) if a == "free" => None,
                    Some(Value::String(a)) if a == "original" || a == "asShot" => Some(((w * 100.0) as u32, (h * 100.0) as u32)),
                    Some(Value::String(a)) => {
                        let (x, y) = a.split_once('x').ok_or_else(|| bad("crop.aspect", "use WxH"))?;
                        let (x, y): (f64, f64) =
                            (x.parse().map_err(|_| bad("crop.aspect", "bad number"))?, y.parse().map_err(|_| bad("crop.aspect", "bad number"))?);
                        Some(aspect_tuple("crop.aspect", x, y)?)
                    }
                    Some(v @ Value::Array(_)) => match numbers(Some(v)).as_deref() {
                        Some(&[x, y]) => Some(aspect_tuple("crop.aspect", x, y)?),
                        _ => return Err(bad("crop.aspect", "use [w, h] with two numbers")),
                    },
                    _ => return Err(bad("crop.aspect", "missing aspect")),
                };
                edit(s, "crop.aspect", "Crop Aspect", |d| {
                    if aspect.is_none() {
                        d.crop.aspect = None;
                    } else {
                        set_aspect(d, w, h, aspect);
                    }
                    Ok(())
                })
            }
        ),
        cmd!("crop.rotateAspect", "Rotate Crop Aspect", [], Some("X"), "{}", has_active, |s, _| {
            let id = active(s, "crop.rotateAspect")?;
            let (w, h) = crop_dims(s, id);
            edit(s, "crop.rotateAspect", "Rotate Crop", |d| {
                let r = d.crop.geometry.rect_px(w, h);
                let a = r.height() / r.width();
                d.crop.aspect = d.crop.aspect.map(|(x, y)| (y, x));
                let angle = d.crop.geometry.angle;
                d.crop.geometry = crop_fit_angle(w, h, angle, Some(a));
                Ok(())
            })
        }),
        cmd!("crop.reset", "Reset Crop", [], Some("Cmd+Alt+R"), "{}", has_active, |s, _| {
            edit(s, "crop.reset", "Reset Crop", |d| {
                d.crop = Default::default();
                Ok(())
            })
        }),
        // ---- HDR editing
        cmd!(
            "develop.hdr",
            "HDR",
            [],
            None,
            "{enabled?: bool (default: toggle), maxEv?: number 0..5 (headroom limit, stops above SDR white)} — HDR editing: highlights above SDR white for HDR exports and displays; every SDR render uses the SDR rendition (the hdr.sdr* controls)",
            has_active,
            |s, p| {
                let id = active(s, "develop.hdr")?;
                let cur = s.develop_of(id).unwrap_or_default().hdr;
                let enabled = bool_or(p, "enabled", !cur.enabled);
                let max_ev = match p.get("maxEv") {
                    None => None,
                    Some(v) => Some(v.as_f64().filter(|x| x.is_finite()).ok_or_else(|| bad("develop.hdr", "`maxEv` must be a number"))?),
                };
                let label = if enabled { "HDR On" } else { "HDR Off" };
                edit(s, "develop.hdr", label, |d| {
                    d.hdr.enabled = enabled;
                    if let Some(m) = max_ev {
                        d.hdr.max_ev = m.clamp(0.0, lightcraft_develop::Hdr::MAX_EV_LIMIT);
                    }
                    Ok(())
                })?;
                Ok(json!({"enabled": enabled, "maxEv": s.develop_of(id).map(|d| d.hdr.max_ev)}))
            }
        ),
        // ---- geometry: Upright
        cmd!(
            "geometry.upright",
            "Upright",
            [],
            None,
            "{mode: off|auto|guided|level|vertical|full} — analyses the photo (line segments → vanishing points) and stores the correction; run again to update after lens changes",
            has_active,
            |s, p| {
                let mode: Upright =
                    serde_json::from_value(p.get("mode").cloned().unwrap_or(json!("auto"))).map_err(|e| bad("geometry.upright", e.to_string()))?;
                let id = active(s, "geometry.upright")?;
                let d = s.develop_of(id).unwrap_or_default();
                let transform = match mode {
                    Upright::Off | Upright::Guided => None,
                    m => {
                        let src = s.source_now(id, SourceLevel::Preview).map_err(|e| bad("geometry.upright", e))?;
                        let info = s.source_info(id);
                        Some(lightcraft_pipeline::upright::auto_transform(&src, &info, &d, m))
                    }
                };
                edit(s, "geometry.upright", "Upright", |d| {
                    d.geometry.upright = mode;
                    d.geometry.upright_transform = transform;
                    Ok(())
                })?;
                Ok(json!({"mode": mode, "transform": transform}))
            }
        ),
        cmd!(
            "geometry.guides",
            "Guided Upright Guides",
            [],
            None,
            "{guides: [[x0,y0,x1,y1], …] (≤ 4, normalized coords of the lens-corrected image), add?: bool (append one guide, dropping the oldest beyond 4)} — switches Upright to Guided",
            has_active,
            |s, p| {
                let parse = |v: &Value| -> Option<(Point, Point)> {
                    let a: Vec<f64> = v.as_array()?.iter().filter_map(Value::as_f64).collect();
                    (a.len() == 4 && a.iter().all(|x| x.is_finite())).then(|| (Point::new(a[0], a[1]), Point::new(a[2], a[3])))
                };
                let guides: Vec<(Point, Point)> =
                    p.get("guides").and_then(Value::as_array).map(|a| a.iter().filter_map(parse).collect()).unwrap_or_default();
                let add = bool_or(p, "add", false);
                edit(s, "geometry.guides", "Guided Upright", |d| {
                    if add {
                        d.geometry.guides.extend(guides);
                    } else {
                        d.geometry.guides = guides;
                    }
                    let n = d.geometry.guides.len();
                    if n > 4 {
                        d.geometry.guides.drain(..n - 4);
                    }
                    d.geometry.upright = Upright::Guided;
                    d.geometry.upright_transform = None;
                    Ok(())
                })
            }
        ),
        // ---- copy / paste / sync
        cmd!(
            "develop.copy",
            "Copy Edit Settings",
            ["Edit"],
            Some("Cmd+C"),
            "{groups?: [settingsGroup] (per-setting ids from develop.settingsGroups, or a heading id such as light / detail / crop), subset?: all | none | default | modified | a saved subset (develop.saveSubset), masks?: [mask id] (only these masks)} (default: the groups last copied; at first all but crop, masking, remove and red eye), mode?: exact (default) | sinceFirst | sinceStep — paste the settings, or the changes made since the first history state / history `step` (an index) moved onto each photo's own (Lightroom's relative paste) → {groups, mode}",
            has_active,
            |s, p| {
                let c = "develop.copy";
                let id = active(s, c)?;
                let base = paste_base(s, id, p, c)?;
                if let Some(g) = groups_param(s, p, c)? {
                    s.copy_groups = g;
                }
                let masks = masks_param(p, c)?;
                let d = s.develop_of(id).unwrap_or_default();
                let take = |d: &DevelopSettings| {
                    let mut v = lightcraft_develop::extract_groups(d, &s.copy_groups);
                    if let Some(m) = &masks {
                        lightcraft_develop::retain_masks(&mut v, m);
                    }
                    v
                };
                let clip = take(&d);
                s.clipboard_base = base.as_ref().map(take);
                s.clipboard = Some(clip);
                Ok(json!({"groups": s.copy_groups, "mode": str_param(p, "mode").unwrap_or("exact")}))
            }
        ),
        cmd!(
            "develop.pastePrevious",
            "Paste Settings from Previous",
            ["Edit"],
            Some("Cmd+Alt+V"),
            "{ids?, groups?: [settingsGroup], subset? (default: the copy groups)} — from the photo that was active before this one",
            has_active,
            |s, p| {
                let c = "develop.pastePrevious";
                let prev =
                    s.previous_active.filter(|id| s.catalog.photo(*id).is_some_and(|ph| !ph.deleted)).ok_or_else(|| bad(c, "no previous photo"))?;
                let groups = groups_param(s, p, c)?.unwrap_or_else(|| s.copy_groups.clone());
                let partial = lightcraft_develop::extract_groups(&s.develop_of(prev).unwrap_or_default(), &groups);
                let ids: Vec<PhotoId> = s.targets(p).into_iter().filter(|id| *id != prev).collect();
                let ops = ids
                    .iter()
                    .filter_map(|id| s.develop_of(*id).map(|d| (*id, d)))
                    .filter_map(|(id, d)| s.develop_op(id, lightcraft_develop::apply_partial(&d, &partial, 1.0), "Paste from Previous"))
                    .collect::<Vec<_>>();
                let n = ops.len();
                s.commit("Paste from Previous", Op::Batch { ops })?;
                Ok(json!({"changed": n, "from": prev.0}))
            }
        ),
        cmd!(
            "develop.paste",
            "Paste Edit Settings",
            ["Edit"],
            Some("Cmd+V"),
            "{ids?, groups?: [settingsGroup], subset? (paste only these of the copied groups), maskMode?: replace (default) | merge — copied masks replace the photo's own or are added to them} — exact settings, or the changes since a history state when copied that way (develop.copy mode) → {changed, relative}",
            has_clipboard,
            |s, p| {
                let c = "develop.paste";
                let mut clip = s.clipboard.clone().unwrap_or_default();
                let mut base = s.clipboard_base.clone();
                let merge = merge_masks_param(p, c)?;
                if let Some(only) = groups_param(s, p, c)? {
                    // the clipboard holds whole groups: rebuild it with just the chosen (copied) ones
                    let only: Vec<_> = only.into_iter().filter(|g| s.copy_groups.contains(g)).collect();
                    let pick = |v: &Value| {
                        lightcraft_develop::extract_groups(&lightcraft_develop::apply_partial(&DevelopSettings::default(), v, 1.0), &only)
                    };
                    base = base.as_ref().map(pick);
                    clip = pick(&clip);
                }
                let paste = |d: &DevelopSettings| {
                    let patch = match &base {
                        Some(b) => lightcraft_develop::relative_patch(&d.to_json(), &clip, b).unwrap_or_else(|| json!({})),
                        None => clip.clone(),
                    };
                    if merge { lightcraft_develop::apply_merging_masks(d, &patch, 1.0) } else { lightcraft_develop::apply_partial(d, &patch, 1.0) }
                };
                let ids = s.targets(p);
                let ops = ids
                    .iter()
                    .filter_map(|id| s.develop_of(*id).map(|d| (*id, d)))
                    .filter_map(|(id, d)| s.develop_op(id, paste(&d), "Paste Settings"))
                    .collect::<Vec<_>>();
                let n = ops.len();
                s.commit("Paste Settings", Op::Batch { ops })?;
                Ok(json!({"changed": n, "relative": base.is_some()}))
            }
        ),
        cmd!(
            "develop.sync",
            "Sync Settings",
            [],
            None,
            "{groups?: [settingsGroup] | subset? (as develop.copy; default: Lightroom's default subset), masks?: [mask id], matchExposure?: bool, autoAlign?: bool, matchCameraColor?: bool, scene?: bool} — copy the active photo's settings to all selected photos; matchCameraColor then matches each one's colour to the active photo across cameras / lenses / phones (develop.matchCameraColor); matchExposure sets each one's Exposure so it renders as bright as the active photo (however far over / under it was shot); autoAlign levels each one's horizon → {changed, matched, aligned, colour: [per photo]}",
            has_selection,
            |s, p| {
                let c = "develop.sync";
                let id = active(s, c)?;
                let groups = groups_param(s, p, c)?.unwrap_or_else(SettingsGroup::default_copy);
                let masks = masks_param(p, c)?;
                let (match_exp, align) = (bool_or(p, "matchExposure", false), bool_or(p, "autoAlign", false));
                let match_colour = bool_or(p, "matchCameraColor", false);
                let scene = p.get("scene").and_then(Value::as_bool);
                let d = s.develop_of(id).unwrap_or_default();
                let mut partial = lightcraft_develop::extract_groups(&d, &groups);
                if let Some(m) = masks {
                    lightcraft_develop::retain_masks(&mut partial, &m);
                }
                let target = if match_exp { rendered_brightness(s, id, &d) } else { None };
                let (r_ev, r_slider) = (gathered_ev(s, id), d.light.exposure);
                let (mut ops, mut matched, mut aligned, mut colour) = (Vec::new(), 0, 0, Vec::new());
                for x in s.selection.ids.clone().into_iter().filter(|x| *x != id) {
                    let Some(dd) = s.develop_of(x) else { continue };
                    let mut nd = lightcraft_develop::apply_partial(&dd, &partial, 1.0);
                    if match_colour {
                        colour.push(crate::camera_match::match_photo(s, id, &d, x, &mut nd, scene));
                    }
                    if let Some(t) = target {
                        // start from the camera-settings difference, then match the rendered result
                        if let (Some(r), Some(ev)) = (r_ev, gathered_ev(s, x)) {
                            nd.light.exposure = (r_slider + r - ev).clamp(-5.0, 5.0);
                        }
                        matched += usize::from(match_brightness(s, x, &mut nd, t));
                    }
                    if align && let Some(angle) = level_angle(s, x, &nd) {
                        let (w, h) = crop_dims(s, x);
                        straighten(&mut nd, w, h, angle);
                        aligned += 1;
                    }
                    ops.extend(s.develop_op(x, nd, "Sync Settings"));
                }
                let n = ops.len();
                s.commit("Sync Settings", Op::Batch { ops })?;
                Ok(json!({"changed": n, "matched": matched, "aligned": aligned, "colour": colour}))
            }
        ),
        cmd!(
            "develop.matchCameraColor",
            "Match Camera & Lens Color",
            ["Photo"],
            None,
            "{ids?, scene?: bool} — make each selected photo's colour match the active photo's across cameras, lenses and phones: a ColorChecker-style chart through both cameras' own colour paths sets Temp / Tint, HSL, Calibration and Saturation / Vibrance (raws of different cameras); the lens / device casts learned from your edits (develop.buildLensCasts) even out lens differences; with scene (default: shot within 30 min of each other) the two renders' neutrals and hue bands even out what is left. One undo step; same camera and lens without scene changes nothing → {changed, perPhoto: [{id, deltaE_before, deltaE_after, chart, lensCast, scene}]}",
            has_selection,
            |s, p| {
                let c = "develop.matchCameraColor";
                let reference = active(s, c)?;
                let rd = s.develop_of(reference).unwrap_or_default();
                let scene = p.get("scene").and_then(Value::as_bool);
                let (mut ops, mut per) = (Vec::new(), Vec::new());
                for id in s.targets(p).into_iter().filter(|x| *x != reference) {
                    let Some(dd) = s.develop_of(id) else { continue };
                    let mut nd = (*dd).clone();
                    per.push(crate::camera_match::match_photo(s, reference, &rd, id, &mut nd, scene));
                    if nd != *dd {
                        ops.extend(s.develop_op(id, nd, "Match Camera & Lens Color"));
                    }
                }
                let n = ops.len();
                if n > 0 {
                    s.commit("Match Camera & Lens Color", Op::Batch { ops })?;
                }
                Ok(json!({"changed": n, "perPhoto": per}))
            }
        ),
        cmd!(
            "develop.buildLensCasts",
            "Learn Lens & Device Colour Casts",
            [],
            None,
            "{catalog?: path to a Lightroom .lrcat (default: the library's own edits), minSessions?: 3, minPhotos?: 10, dryRun?} — learn each lens's (and phone's) colour cast from how you finished its photos next to your other lenses at the same events; saved locally (lens-casts.json in LightCraft's settings folder, never shared) for develop.matchCameraColor → {path, lenses, devices}",
            always,
            |s, p| {
                let c = "develop.buildLensCasts";
                let ev = crate::lens_casts::Evidence {
                    min_sessions: f64_or(p, "minSessions", 3.0).clamp(1.0, 1000.0) as usize,
                    min_photos: f64_or(p, "minPhotos", 10.0).clamp(1.0, 100_000.0) as usize,
                };
                let (rows, source) = match str_param(p, "catalog") {
                    Some(cat) => {
                        let scratch = str_param(p, "scratch").map(std::path::PathBuf::from).unwrap_or_else(std::env::temp_dir);
                        (
                            crate::lens_casts::rows_from_lightroom(std::path::Path::new(cat), &scratch).map_err(|e| bad(c, e))?,
                            format!("lightroom:{cat}"),
                        )
                    }
                    None => (crate::lens_casts::rows_from_library(s), "library".to_string()),
                };
                let table = crate::lens_casts::build(&rows, ev, &source);
                let mut out = table.summary();
                out["rows"] = json!(rows.len());
                if !bool_or(p, "dryRun", false) {
                    let path = crate::lens_casts::save(&table).map_err(|e| bad(c, e))?;
                    out["path"] = json!(path.display().to_string());
                }
                Ok(out)
            }
        ),
        cmd!(
            "develop.quickAdjust",
            "Quick Develop",
            [],
            None,
            "{control: develop control id, delta, ids?} — add `delta` to the control on every target photo (each from its own value; Quick Develop) in one undo step → {changed}",
            has_selection,
            |s, p| {
                let c = "develop.quickAdjust";
                let ctl = str_param(p, "control").ok_or_else(|| bad(c, "missing control"))?.to_string();
                let spec = controls::find(&ctl).ok_or_else(|| bad(c, format!("unknown control `{ctl}`")))?;
                let delta = f64_req(p, "delta", c)?;
                let label = format!("Quick Develop: {}", spec.label);
                let ops: Vec<Op> = s
                    .targets(p)
                    .into_iter()
                    .filter_map(|id| s.develop_of(id).map(|d| (id, d)))
                    .filter_map(|(id, d)| {
                        let mut nd = (*d).clone();
                        if nd.wb.mode == WbMode::AsShot && (ctl == "wb.temp" || ctl == "wb.tint") {
                            let info = s.source_info(id);
                            nd.wb.temp = info.as_shot_temp;
                            nd.wb.tint = info.as_shot_tint;
                        }
                        let v = controls::get(&nd, &ctl).unwrap_or(spec.default);
                        controls::set(&mut nd, &ctl, v + delta);
                        if ctl == "wb.temp" || ctl == "wb.tint" {
                            nd.wb.mode = WbMode::Custom;
                        }
                        s.develop_op(id, nd, &label)
                    })
                    .collect();
                let n = ops.len();
                if n > 0 {
                    s.commit(&label, Op::Batch { ops })?;
                }
                Ok(json!({"changed": n}))
            }
        ),
        cmd!(
            "develop.matchExposure",
            "Match Total Exposures",
            ["Photo"],
            None,
            "{ids?} — set each selected photo's Exposure so its total exposure (shutter × ISO ÷ aperture², plus the slider) equals the active photo's; photos without exposure data are skipped → {changed, skipped}",
            has_selection,
            |s, p| {
                let c = "develop.matchExposure";
                let reference = active(s, c)?;
                let r_ev = gathered_ev(s, reference).ok_or_else(|| bad(c, "the active photo has no shutter / aperture / ISO"))?;
                let r_slider = s.develop_of(reference).map(|d| d.light.exposure).unwrap_or(0.0);
                let ids: Vec<PhotoId> = s.targets(p).into_iter().filter(|x| *x != reference).collect();
                let (mut ops, mut skipped) = (Vec::new(), Vec::new());
                for id in ids {
                    let (Some(ev), Some(d)) = (gathered_ev(s, id), s.develop_of(id)) else {
                        skipped.push(id.0);
                        continue;
                    };
                    let mut nd = (*d).clone();
                    nd.light.exposure = (r_slider + r_ev - ev).clamp(-5.0, 5.0);
                    ops.extend(s.develop_op(id, nd, "Match Total Exposures"));
                }
                let n = ops.len();
                if n > 0 {
                    s.commit("Match Total Exposures", Op::Batch { ops })?;
                }
                Ok(json!({"changed": n, "skipped": skipped}))
            }
        ),
        cmd!(
            "develop.matchBrightness",
            "Match Brightness",
            ["Photo"],
            Some("Cmd+Alt+Shift+M"),
            "{ids?} — set each selected photo's Exposure so it renders as bright as the active photo (measured on the finished look, so a photo shot 4 stops under and one 3 over both land on it) → {changed, skipped}",
            has_selection,
            |s, p| {
                let c = "develop.matchBrightness";
                let reference = active(s, c)?;
                let d = s.develop_of(reference).unwrap_or_default();
                let target = rendered_brightness(s, reference, &d).ok_or_else(|| bad(c, "the active photo couldn't be rendered"))?;
                let (r_ev, r_slider) = (gathered_ev(s, reference), d.light.exposure);
                let (mut ops, mut skipped) = (Vec::new(), Vec::new());
                for id in s.targets(p).into_iter().filter(|x| *x != reference) {
                    let Some(dd) = s.develop_of(id) else { continue };
                    let mut nd = (*dd).clone();
                    if let (Some(r), Some(ev)) = (r_ev, gathered_ev(s, id)) {
                        nd.light.exposure = (r_slider + r - ev).clamp(-5.0, 5.0);
                    }
                    if match_brightness(s, id, &mut nd, target) {
                        ops.extend(s.develop_op(id, nd, "Match Brightness"));
                    } else {
                        skipped.push(id.0);
                    }
                }
                let n = ops.len();
                if n > 0 {
                    s.commit("Match Brightness", Op::Batch { ops })?;
                }
                Ok(json!({"changed": n, "skipped": skipped}))
            }
        ),
        cmd!(
            "develop.autoAlign",
            "Auto Align (Horizon)",
            ["Photo"],
            Some("Cmd+Alt+Shift+U"),
            "{ids?} — level the horizon (or plumb lines) of every selected photo with its crop angle, in one undo step → {changed, skipped}",
            has_selection,
            |s, p| {
                let (mut ops, mut skipped) = (Vec::new(), Vec::new());
                for id in s.targets(p) {
                    let Some(dd) = s.develop_of(id) else { continue };
                    let mut nd = (*dd).clone();
                    let Some(angle) = level_angle(s, id, &nd) else {
                        skipped.push(id.0);
                        continue;
                    };
                    let (w, h) = crop_dims(s, id);
                    straighten(&mut nd, w, h, angle);
                    ops.extend(s.develop_op(id, nd, "Auto Align"));
                }
                let n = ops.len();
                if n > 0 {
                    s.commit("Auto Align", Op::Batch { ops })?;
                }
                Ok(json!({"changed": n, "skipped": skipped}))
            }
        ),
        cmd!(
            "develop.autoSync",
            "Auto Sync",
            ["Edit"],
            Some("Cmd+Alt+Shift+A"),
            "{on?: bool} (toggles when omitted) — edits to the active photo also change the other selected photos (only the settings that changed; not spot removal / red eye) → {on}",
            always,
            |s, p| {
                s.auto_sync = p.get("on").and_then(Value::as_bool).unwrap_or(!s.auto_sync);
                Ok(json!({"on": s.auto_sync}))
            }
        ),
        // ---- presets
        cmd!("preset.apply", "Apply Preset", [], None, "{id: presetId, amount?: 0..200 (percent), ids?}", has_selection, |s, p| {
            let pid = str_param(p, "id").ok_or_else(|| bad("preset.apply", "missing id"))?;
            let preset = s.presets.iter().find(|x| x.id == pid).cloned().ok_or_else(|| bad("preset.apply", format!("unknown preset `{pid}`")))?;
            let amount = f64_or(p, "amount", 100.0).clamp(0.0, 200.0) / 100.0;
            let ids = s.targets(p);
            let ops = ids
                .iter()
                .filter_map(|id| s.develop_of(*id).map(|d| (*id, d)))
                .filter_map(|(id, d)| s.develop_op(id, preset.apply(&d, amount), &format!("Preset: {}", preset.name)))
                .collect::<Vec<_>>();
            s.commit(&format!("Preset: {}", preset.name), Op::Batch { ops })?;
            ok()
        }),
        cmd!(
            "preset.create",
            "Create Preset",
            ["Photo"],
            None,
            "{name, group?, groups?: [settingsGroup] | subset?, masks?: [mask id]}",
            has_active,
            |s, p| {
                let c = "preset.create";
                let id = active(s, c)?;
                let groups = groups_param(s, p, c)?.unwrap_or_else(SettingsGroup::default_copy);
                let masks = masks_param(p, c)?;
                let name = str_param(p, "name").ok_or_else(|| bad(c, "missing name"))?;
                let group = str_param(p, "group").unwrap_or("User Presets");
                let d = s.develop_of(id).unwrap_or_default();
                let pid = format!("user.{}.{}", s.presets.len(), name.to_lowercase().replace(' ', "-"));
                let mut preset = Preset::from_settings(&pid, name, group, &d, &groups);
                if let Some(m) = masks {
                    lightcraft_develop::retain_masks(&mut preset.settings, &m);
                }
                s.presets.push(preset);
                Ok(json!({"id": pid}))
            }
        ),
        cmd!("preset.delete", "Delete Preset", [], None, "{id}", always, |s, p| {
            let pid = str_param(p, "id").ok_or_else(|| bad("preset.delete", "missing id"))?;
            let i = s.presets.iter().position(|x| x.id == pid).ok_or_else(|| bad("preset.delete", "unknown preset"))?;
            if s.presets[i].builtin {
                return Err(bad("preset.delete", "built-in presets can't be deleted"));
            }
            s.presets.remove(i);
            ok()
        }),
        cmd!("preset.favorite", "Favorite Preset", [], None, "{id, favorite?: bool}", always, |s, p| {
            let pid = str_param(p, "id").ok_or_else(|| bad("preset.favorite", "missing id"))?;
            let pr = s.presets.iter_mut().find(|x| x.id == pid).ok_or_else(|| bad("preset.favorite", "unknown preset"))?;
            pr.favorite = bool_or(p, "favorite", !pr.favorite);
            ok()
        }),
        cmd!("preset.rename", "Rename Preset", [], None, "{id, name}", always, |s, p| {
            let c = "preset.rename";
            let name = str_param(p, "name").map(str::trim).filter(|n| !n.is_empty()).ok_or_else(|| bad(c, "missing name"))?.to_string();
            user_preset(s, p, c)?.name = name;
            ok()
        }),
        cmd!("preset.move", "Move Preset to Group", [], None, "{id, group}", always, |s, p| {
            let c = "preset.move";
            let group = str_param(p, "group").map(str::trim).filter(|n| !n.is_empty()).ok_or_else(|| bad(c, "missing group"))?.to_string();
            user_preset(s, p, c)?.group = group;
            ok()
        }),
        cmd!(
            "preset.update",
            "Update Preset with Current Settings",
            [],
            None,
            "{id, groups?: [settingsGroup] (default: the groups the preset already sets)} — from the active photo",
            has_active,
            |s, p| {
                let c = "preset.update";
                let id = active(s, c)?;
                let current = s.develop_of(id).unwrap_or_default().to_json();
                let groups = groups_param(s, p, c)?;
                let pr = user_preset(s, p, c)?;
                pr.settings = match groups {
                    Some(g) => lightcraft_develop::extract_groups(&serde_json::from_value(current).unwrap_or_default(), &g),
                    // the same keys as before, with the current values
                    None => {
                        let keys: Vec<String> = pr.settings.as_object().map(|o| o.keys().cloned().collect()).unwrap_or_default();
                        Value::Object(keys.into_iter().filter_map(|k| current.get(&k).cloned().map(|v| (k, v))).collect())
                    }
                };
                ok()
            }
        ),
        // ---- versions & history
        cmd!("version.create", "Create Version", ["Photo"], Some("Cmd+Shift+S"), "{name?}", has_active, |s, p| {
            let id = active(s, "version.create")?;
            let ph = s.catalog.photo(id).cloned().ok_or_else(|| bad("version.create", "no photo"))?;
            let mut versions = ph.versions.clone();
            let name = str_param(p, "name").map(str::to_string).unwrap_or_else(|| format!("Version {}", versions.len() + 1));
            versions.push(Version { name, created: (s.clock)(), settings: ph.develop.clone(), auto: false });
            s.commit("Create Version", Op::SetVersions { id, versions })?;
            ok()
        }),
        cmd!("version.restore", "Restore Version", [], None, "{index}", has_active, |s, p| {
            let id = active(s, "version.restore")?;
            let i = f64_req(p, "index", "version.restore")? as usize;
            let v = s.catalog.photo(id).and_then(|ph| ph.versions.get(i).cloned()).ok_or_else(|| bad("version.restore", "no such version"))?;
            s.set_develop(id, (*v.settings).clone(), &format!("Restore {}", v.name))?;
            ok()
        }),
        cmd!("version.delete", "Delete Version", [], None, "{index}", has_active, |s, p| {
            let id = active(s, "version.delete")?;
            let i = f64_req(p, "index", "version.delete")? as usize;
            let mut versions = s.catalog.photo(id).map(|ph| ph.versions.clone()).unwrap_or_default();
            if i >= versions.len() {
                return Err(bad("version.delete", "no such version"));
            }
            versions.remove(i);
            s.commit("Delete Version", Op::SetVersions { id, versions })?;
            ok()
        }),
        cmd!("version.rename", "Rename Version", [], None, "{index, name}", has_active, |s, p| {
            let c = "version.rename";
            let name = str_param(p, "name").map(str::trim).filter(|n| !n.is_empty()).ok_or_else(|| bad(c, "missing name"))?.to_string();
            edit_version(s, p, c, "Rename Version", |v, _| v.name = name)
        }),
        cmd!("version.update", "Update Version with Current Settings", [], None, "{index}", has_active, |s, p| {
            edit_version(s, p, "version.update", "Update Version", |v, current| v.settings = current)
        }),
        cmd!("history.clear", "Clear History", [], None, "{} — keeps the current settings as the only step", has_active, |s, _| {
            let id = active(s, "history.clear")?;
            let d = s.develop_of(id).unwrap_or_default();
            let history = vec![lightcraft_catalog::HistoryStep { label: "Cleared History".into(), settings: d }];
            s.commit("Clear History", Op::SetHistory { id, history })?;
            ok()
        }),
        cmd!("history.restore", "Go to History Step", [], None, "{index}", has_active, |s, p| {
            let id = active(s, "history.restore")?;
            let i = f64_req(p, "index", "history.restore")? as usize;
            let st = s.catalog.photo(id).and_then(|ph| ph.history.get(i).cloned()).ok_or_else(|| bad("history.restore", "no such step"))?;
            s.set_develop(id, (*st.settings).clone(), &format!("History: {}", st.label))?;
            ok()
        }),
        // ---- interactions (slider drags, brush strokes)
        cmd!("develop.beginInteraction", "Begin Interaction", [], None, "{label}", has_active, |s, p| {
            s.begin_interaction(str_param(p, "label").unwrap_or("Edit"))?;
            ok()
        }),
        cmd!("develop.endInteraction", "End Interaction", [], None, "{}", always, |s, _| {
            s.end_interaction()?;
            ok()
        }),
        cmd!("develop.cancelInteraction", "Cancel Interaction", [], None, "{}", always, |s, _| {
            s.cancel_interaction()?;
            ok()
        }),
    ]
}
