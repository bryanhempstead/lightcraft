//! Faces for Best Take: where each person's face is in each photo of a burst, and which face in
//! one photo is the same person in another.
//!
//! Boxes come from the photo's own face regions when it has them (named faces Lightroom & co.
//! wrote to XMP), else from Apple Vision on macOS through a small helper (`faces.swift`, our own
//! source, built once with `swiftc` into LightCraft's support folder). Elsewhere, or without the
//! Swift compiler, only tagged faces are found. Boxes are normalized `[x, y, w, h]`, top-left
//! origin, in the photo's upright, uncropped frame (the frame [`lightcraft_merge::load_frame`]
//! with orientation applied gives the merge).

use lightcraft_catalog::PhotoId;
use serde::Serialize;

use crate::Session;

/// A face box: normalized `[x, y, w, h]`, top-left origin.
pub type FaceBox = [f64; 4];

/// One person across the burst.
#[derive(Clone, Debug, PartialEq, Serialize)]
pub struct TakePerson {
    /// The name on a tagged face, else "Person n".
    pub name: String,
    /// Their face in each photo (`None` where it wasn't found), in the order of the photos.
    pub boxes: Vec<Option<FaceBox>>,
}

#[cfg(target_os = "macos")]
const SWIFT: &str = include_str!("faces.swift");

/// The helper binary, built from [`SWIFT`] on first use (rebuilt when the source changes).
#[cfg(target_os = "macos")]
fn helper() -> Result<std::path::PathBuf, String> {
    use std::hash::{Hash, Hasher};
    let home = std::env::var_os("HOME").ok_or("no HOME")?;
    let dir = std::path::PathBuf::from(home).join("Library/Application Support/LightCraft/bin");
    let mut h = std::collections::hash_map::DefaultHasher::new();
    SWIFT.hash(&mut h);
    let bin = dir.join(format!("lc-faces-{:016x}", h.finish()));
    if bin.exists() {
        return Ok(bin);
    }
    std::fs::create_dir_all(&dir).map_err(|e| format!("{}: {e}", dir.display()))?;
    let src = dir.join("lc-faces.swift");
    std::fs::write(&src, SWIFT).map_err(|e| format!("{}: {e}", src.display()))?;
    let out = std::process::Command::new("/usr/bin/swiftc")
        .args(["-O", "-o"])
        .arg(&bin)
        .arg(&src)
        .output()
        .map_err(|e| format!("the Swift compiler isn't available ({e}); install Xcode's command line tools to find faces"))?;
    if !out.status.success() || !bin.exists() {
        return Err(format!("building the face finder failed: {}", String::from_utf8_lossy(&out.stderr).chars().take(300).collect::<String>()));
    }
    Ok(bin)
}

/// Vision's face boxes for each image file (faces below 0.5 confidence dropped).
#[cfg(target_os = "macos")]
fn vision(paths: &[std::path::PathBuf]) -> Result<Vec<Vec<FaceBox>>, String> {
    let bin = helper()?;
    let out = std::process::Command::new(&bin).args(paths).output().map_err(|e| format!("face finder: {e}"))?;
    let v: serde_json::Value = serde_json::from_slice(&out.stdout).map_err(|e| format!("face finder output: {e}"))?;
    let list = v.as_array().ok_or("face finder output: not a list")?;
    Ok(paths
        .iter()
        .enumerate()
        .map(|(i, _)| {
            list.get(i)
                .and_then(|a| a.as_array())
                .map(|faces| {
                    faces
                        .iter()
                        .filter(|f| f.get("c").and_then(|c| c.as_f64()).unwrap_or(1.0) >= 0.5)
                        .filter_map(|f| {
                            let g = |k: &str| f.get(k).and_then(|x| x.as_f64()).filter(|x| x.is_finite());
                            let b = [g("x")?, g("y")?, g("w")?, g("h")?];
                            (b[2] > 0.0 && b[3] > 0.0).then_some(b)
                        })
                        .collect()
                })
                .unwrap_or_default()
        })
        .collect())
}

#[cfg(not(target_os = "macos"))]
fn vision(_: &[std::path::PathBuf]) -> Result<Vec<Vec<FaceBox>>, String> {
    Err("finding untagged faces needs macOS; tag the faces (People) first".into())
}

/// Group faces into people by position: bursts barely move, so the face nearest the same spot is
/// the same person. People come from the photo with the most faces, left to right.
pub fn match_people(frames: &[Vec<(FaceBox, Option<String>)>]) -> Vec<TakePerson> {
    let Some(reference) = frames.iter().max_by_key(|f| f.len()) else { return Vec::new() };
    let mut seeds: Vec<&(FaceBox, Option<String>)> = reference.iter().collect();
    seeds.sort_by(|a, b| a.0[0].total_cmp(&b.0[0]));
    let centre = |b: &FaceBox| (b[0] + b[2] / 2.0, b[1] + b[3] / 2.0);
    let mut people: Vec<TakePerson> = seeds
        .iter()
        .enumerate()
        .map(|(i, (_, name))| TakePerson { name: name.clone().unwrap_or_else(|| format!("Person {}", i + 1)), boxes: vec![None; frames.len()] })
        .collect();
    for (fi, faces) in frames.iter().enumerate() {
        // each face goes to the nearest seed, each seed takes at most one face per photo
        let mut pairs: Vec<(f64, usize, usize)> = Vec::new();
        for (k, (b, _)) in faces.iter().enumerate() {
            let (cx, cy) = centre(b);
            for (pi, (s, _)) in seeds.iter().enumerate() {
                let (sx, sy) = centre(s);
                let d = (cx - sx).hypot(cy - sy);
                if d < (s[2].max(s[3]) * 1.5).max(0.1) {
                    pairs.push((d, k, pi));
                }
            }
        }
        pairs.sort_by(|a, b| a.0.total_cmp(&b.0));
        let (mut face_used, mut person_used) = (vec![false; faces.len()], vec![false; people.len()]);
        for (_, k, pi) in pairs {
            if face_used.get(k).copied().unwrap_or(true) || person_used.get(pi).copied().unwrap_or(true) {
                continue;
            }
            face_used[k] = true;
            person_used[pi] = true;
            if let (Some(p), Some((b, name))) = (people.get_mut(pi), faces.get(k)) {
                if let Some(slot) = p.boxes.get_mut(fi) {
                    *slot = Some(*b);
                }
                if p.name.starts_with("Person ")
                    && let Some(n) = name
                {
                    p.name = n.clone();
                }
            }
        }
    }
    people
}

impl Session {
    /// The faces in each of `ids` (tagged faces where a photo has them, else Vision's), grouped
    /// into people.
    pub fn take_faces(&mut self, ids: &[PhotoId]) -> Result<Vec<TakePerson>, String> {
        let mut frames: Vec<Vec<(FaceBox, Option<String>)>> = Vec::new();
        let mut need: Vec<(usize, PhotoId)> = Vec::new();
        for (i, id) in ids.iter().enumerate() {
            let p = self.catalog.photo(*id).ok_or("no such photo")?;
            let tagged: Vec<(FaceBox, Option<String>)> = p
                .meta
                .regions
                .iter()
                .filter(|r| r.kind == lightcraft_meta::RegionKind::Face)
                .map(|r| ([r.rect.x0, r.rect.y0, r.rect.x1 - r.rect.x0, r.rect.y1 - r.rect.y0], r.name.clone()))
                .filter(|(b, _)| b[2] > 0.0 && b[3] > 0.0)
                .collect();
            if tagged.is_empty() {
                need.push((i, *id));
            }
            frames.push(tagged);
        }
        if !need.is_empty() {
            // upright, uncropped renders for Vision (the frame the merge works in)
            let dir = std::env::temp_dir().join(format!("lc-faces-{}", std::process::id()));
            std::fs::create_dir_all(&dir).map_err(|e| e.to_string())?;
            let mut paths = Vec::new();
            for (i, id) in &need {
                let job = self.render_job(*id, 1600, 1600, false, false).ok_or("no such photo")?;
                let r = job.run();
                self.accept(&r);
                let img = r.rendered?.image;
                let jpg = lightcraft_codecs::encode_jpeg(
                    &lightcraft_codecs::EncodeImage::rgba8(&img),
                    88,
                    lightcraft_codecs::ChromaSubsampling::S420,
                    &lightcraft_codecs::EncodeMeta::default(),
                )
                .map_err(|e| e.to_string())?;
                let path = dir.join(format!("{i}.jpg"));
                std::fs::write(&path, jpg).map_err(|e| e.to_string())?;
                paths.push(path);
            }
            let found = vision(&paths);
            let _ = std::fs::remove_dir_all(&dir);
            let found = found?;
            for ((i, _), faces) in need.iter().zip(found) {
                if let Some(f) = frames.get_mut(*i) {
                    *f = faces.into_iter().map(|b| (b, None)).collect();
                }
            }
        }
        Ok(match_people(&frames))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn people_follow_position_across_frames() {
        let a = vec![([0.1, 0.2, 0.1, 0.1], None), ([0.6, 0.2, 0.1, 0.1], Some("Kate".to_string()))];
        let b = vec![([0.62, 0.21, 0.1, 0.1], None)]; // only the right face found
        let c = vec![([0.61, 0.2, 0.1, 0.1], None), ([0.11, 0.22, 0.1, 0.1], None)];
        let p = match_people(&[a, b, c]);
        assert_eq!(p.len(), 2);
        assert_eq!(p[0].name, "Person 1");
        assert_eq!(p[1].name, "Kate");
        assert!(p[0].boxes[1].is_none() && p[0].boxes[2].is_some_and(|b| (b[0] - 0.11).abs() < 1e-9));
        assert!(p[1].boxes.iter().all(Option::is_some));
        assert!(match_people(&[]).is_empty());
    }
}
