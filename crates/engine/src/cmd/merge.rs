//! Photo Merge commands: `merge.hdr`, `merge.panorama`, `merge.hdrPanorama`, `merge.focus`, `merge.take` (+ `merge.faces`).
//!
//! Run synchronously here (CLI, MCP, control channel); the desktop UI plans the same job with
//! [`Session::plan_merge`] and runs it on a worker thread. `preview: true` merges ≤ 1024 px frames
//! and (with `previewPath`) writes the rendered preview as PNG instead of adding a photo.

use lightcraft_catalog::PhotoId;
use serde_json::{Value, json};

use super::{CommandSpec, bad, bool_or, cmd, str_param};
use crate::{Result, Session};

fn can_merge(s: &Session) -> std::result::Result<(), String> {
    if cfg!(target_arch = "wasm32") && s.media.file_bytes.is_none() {
        return Err("Photo Merge needs access to the photo files".into());
    }
    // the photo count is checked when run (an explicit `ids` parameter may name them)
    if s.catalog.photos().nth(1).is_none() { Err("the library has fewer than 2 photos".into()) } else { Ok(()) }
}

fn run(s: &mut Session, id: &str, p: &Value) -> Result<Value> {
    let (kind, finish) = crate::merge::parse(id, p)?;
    let ids: Vec<PhotoId> = s.targets(p);
    if !bool_or(p, "preview", false) {
        return s.merge_now(kind, finish, &ids);
    }
    let job = s.plan_merge(kind, finish, &ids, true)?;
    let out = job.run(&|_, _| true).map_err(|e| bad(id, e))?;
    let mut info = out.info;
    if let (Some(path), Some(img)) = (str_param(p, "previewPath"), &out.preview) {
        let png = lightcraft_codecs::encode_png(&lightcraft_codecs::EncodeImage::rgba8(img), &lightcraft_codecs::EncodeMeta::default())
            .map_err(|e| bad(id, e.to_string()))?;
        s.check_write_target(path).map_err(|e| bad(id, e))?;
        crate::export::write_file(path, &png).map_err(|e| bad(id, e))?;
        info["previewPath"] = json!(path);
    }
    if let Some(img) = &out.preview {
        info["previewSize"] = json!([img.width, img.height]);
    }
    Ok(info)
}

pub fn specs() -> Vec<CommandSpec> {
    vec![
        cmd!(
            "merge.hdr",
            "HDR Merge",
            [],
            None,
            "{ids?, align=true, deghost=none|low|medium|high, autoSettings=true, stack=false, preview=false, showOverlay=false, previewPath?}",
            can_merge,
            |s, p| run(s, "merge.hdr", p)
        ),
        cmd!(
            "merge.panorama",
            "Panorama Merge",
            [],
            None,
            "{ids?, projection=auto|spherical|cylindrical|perspective, boundaryWarp=0..100, autoCrop=false, fillEdges=false, autoSettings=true, stack=false, maxMegapixels=40, preview=false, previewPath?}",
            can_merge,
            |s, p| run(s, "merge.panorama", p)
        ),
        cmd!(
            "merge.hdrPanorama",
            "HDR Panorama Merge",
            [],
            None,
            "{ids?, bracket=0 (auto from EXIF), align, deghost, projection, boundaryWarp, autoCrop, fillEdges, autoSettings, stack, preview, previewPath?}",
            can_merge,
            |s, p| run(s, "merge.hdrPanorama", p)
        ),
        cmd!(
            "merge.focus",
            "Focus Stack Merge",
            [],
            None,
            "{ids?, align=true, autoSettings=true, stack=false, preview=false, previewPath?} — photos focused at different distances → one DNG sharp throughout (`<name>-Focus.dng`)",
            can_merge,
            |s, p| run(s, "merge.focus", p)
        ),
        cmd!(
            "merge.faces",
            "Find Faces for Best Take",
            [],
            None,
            "{ids?} — the people in the photos, matched across them by position → {people: [{name, boxes: [[x, y, w, h] | null per photo]}]} (tagged faces, else Apple Vision on macOS)",
            can_merge,
            |s, p| {
                let ids: Vec<PhotoId> = s.targets(p);
                let people = s.take_faces(&ids).map_err(|e| bad("merge.faces", e))?;
                Ok(json!({"ids": ids.iter().map(|i| i.0).collect::<Vec<_>>(), "people": people}))
            }
        ),
        cmd!(
            "merge.take",
            "Best Take Merge",
            [],
            None,
            "{ids?, base: index of the photo to start from, swaps: [{donor: photo index, baseBox: [x, y, w, h], donorBox}] (boxes from merge.faces), autoSettings=true, stack=false, preview=false, previewPath?} — each picked face onto the base photo → `<name>-Take.dng`",
            can_merge,
            |s, p| run(s, "merge.take", p)
        ),
    ]
}
