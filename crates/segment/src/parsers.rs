//! The semantic "parsers" behind Select Sky, Landscape classes and People parts: three public
//! SegFormer checkpoints ([`crate::segformer`]), each optional and downloaded only when the user
//! asks for it (like SAM 3), and what LightCraft makes of their classes.
//!
//! - **Scene** — `nvidia/segformer-b5-finetuned-ade-640-640` (ADE20K, 150 classes; NVIDIA's
//!   SegFormer licence: non-commercial use): sky, water, vegetation, mountains, architecture,
//!   natural / artificial ground, and people as a fallback when SAM 3 isn't installed.
//! - **Body** — `sayeed99/segformer_b3_clothes` (ATR human parsing, MIT): hair, face, arms, legs,
//!   clothes, shoes, hats.
//! - **Face** — `jonathandinu/face-parsing` (CelebAMask-HQ, SegFormer-B5; non-commercial
//!   research use): skin, nose, ears, eyebrows, eyes, lips, mouth, hair, neck, clothing.
//!
//! Lightroom's own people parts that no public model labels are split from the face model's
//! classes by the photo's pixels: Eye Sclera vs Iris and Pupil (the eye's dark disc), Teeth (the
//! bright, unsaturated part of the open mouth).

use crate::fetch::FileSpec;

/// One downloadable parser.
#[derive(Clone, Copy, Debug)]
pub struct ParserSpec {
    /// Folder name under the models folder (`models/<id>/`).
    pub id: &'static str,
    pub name: &'static str,
    /// What it is for (shown before downloading).
    pub purpose: &'static str,
    pub files: &'static [FileSpec<'static>],
    /// Base URLs (`<base>/<file>`), in order.
    pub mirrors: &'static [&'static str],
    pub license: &'static str,
    pub license_url: &'static str,
    /// Longest edge of the image it is run on.
    pub input: usize,
}

impl ParserSpec {
    pub fn bytes(&self) -> u64 {
        self.files.iter().filter_map(|f| f.size).sum()
    }
}

pub const SCENE: ParserSpec = ParserSpec {
    id: "scene-segformer-b5-ade",
    name: "Scene (SegFormer-B5, ADE20K)",
    purpose: "Sky and Landscape masks (sky, water, vegetation, mountains, architecture, ground)",
    files: &[
        FileSpec {
            name: "config.json",
            size: Some(6886),
            sha256: Some("083fbb6bc7ce4a2fc54bdb08686943cc5a72d14b32e81187b6ae12a58092bc13"),
            max: 6886,
        },
        FileSpec {
            name: crate::WEIGHTS_FILE,
            size: Some(338_983_700),
            sha256: Some("3f451be733bc5f69227886ef4a472a373f3440ad11b59b3b82cf50f500855b62"),
            max: 338_983_700,
        },
    ],
    // (the safetensors conversion lives on a pull-request revision of NVIDIA's repository)
    mirrors: &["https://huggingface.co/nvidia/segformer-b5-finetuned-ade-640-640/resolve/f4c8e8f5b581b6bc8ed5208e4bd139d95f65610f"],
    license: "NVIDIA Source Code License for SegFormer (non-commercial)",
    license_url: "https://github.com/NVlabs/SegFormer/blob/master/LICENSE",
    input: 1024,
};

pub const BODY: ParserSpec = ParserSpec {
    id: "body-segformer-b3-clothes",
    name: "People (SegFormer-B3, human parsing)",
    purpose: "People masks: hair, body skin and clothes",
    files: &[
        FileSpec {
            name: "config.json",
            size: Some(1764),
            sha256: Some("6590fb6f9f61e834ac89311bd27aa69a46393447d4ee2bc56b71bd2986170ef6"),
            max: 1764,
        },
        FileSpec {
            name: crate::WEIGHTS_FILE,
            size: Some(189_029_000),
            sha256: Some("f70ae566c5773fb335796ebaa8acc924ac25eb97222c2b2967d44d2fc11568e6"),
            max: 189_029_000,
        },
    ],
    mirrors: &["https://huggingface.co/sayeed99/segformer_b3_clothes/resolve/6c12f0e4edd353fb65d4e3f9d90fdabaefea6d9e"],
    license: "MIT",
    license_url: "https://huggingface.co/sayeed99/segformer_b3_clothes",
    input: 768,
};

pub const FACE: ParserSpec = ParserSpec {
    id: "face-segformer-b5-celebamask",
    name: "Faces (SegFormer-B5, face parsing)",
    purpose: "People masks: face skin, eyebrows, eyes, lips, teeth",
    files: &[
        FileSpec {
            name: "config.json",
            size: Some(1689),
            sha256: Some("01d82e818569beda6aec804642e34ce187bed43326337d52ce370ff015964d68"),
            max: 1689,
        },
        FileSpec {
            name: crate::WEIGHTS_FILE,
            size: Some(338_580_732),
            sha256: Some("c2bec795a8c243db71bd95be538fd62559003566466c71237e45c99b920f4b62"),
            max: 338_580_732,
        },
    ],
    mirrors: &["https://huggingface.co/jonathandinu/face-parsing/resolve/a99b26503a09c8cc0de4aad7057d39ac9224979e"],
    license: "CelebAMask-HQ terms (non-commercial research)",
    license_url: "https://github.com/switchablenorms/CelebAMask-HQ",
    input: 512,
};

/// Every parser, in download order.
pub const ALL: [ParserSpec; 3] = [SCENE, BODY, FACE];

/// Whether `dir` holds a parser's files.
pub fn is_parser_dir(dir: &std::path::Path) -> bool {
    dir.join(crate::WEIGHTS_FILE).is_file() && dir.join("config.json").is_file()
}

/// Lightroom's Landscape classes and the ADE20K classes each takes in.
pub const LANDSCAPE: &[(&str, &str, &[&str])] = &[
    ("sky", "Sky", &["sky"]),
    ("water", "Water", &["water", "sea", "river", "lake", "waterfall", "swimming pool", "fountain"]),
    ("vegetation", "Vegetation", &["tree", "grass", "plant", "palm", "flower"]),
    ("mountains", "Mountains", &["mountain", "hill"]),
    (
        "architecture",
        "Architecture",
        &[
            "building",
            "house",
            "skyscraper",
            "tower",
            "bridge",
            "wall",
            "fence",
            "column",
            "hovel",
            "windowpane",
            "door",
            "awning",
            "canopy",
            "railing",
            "bannister",
            "booth",
            "grandstand",
            "pier",
            "tent",
            "ceiling",
        ],
    ),
    ("naturalGround", "Natural Ground", &["earth", "sand", "rock", "field", "dirt track", "land"]),
    ("artificialGround", "Artificial Ground", &["road", "sidewalk", "floor", "path", "runway", "stairs", "stairway", "step", "rug"]),
];

/// The label of Landscape class `id` (`None`: not one).
pub fn landscape_label(id: &str) -> Option<&'static str> {
    LANDSCAPE.iter().find(|c| c.0 == id).map(|c| c.1)
}

/// ADE20K classes that are a photo's subject when SAM 3 isn't there to find it.
pub const SCENE_SUBJECT: &[&str] = &["person", "animal"];
pub const SCENE_PEOPLE: &[&str] = &["person"];

/// Lightroom's people parts, in its order: id, label.
pub const PEOPLE_PARTS: &[(&str, &str)] = &[
    ("person", "Entire Person"),
    ("faceSkin", "Face Skin"),
    ("bodySkin", "Body Skin"),
    ("eyebrows", "Eyebrows"),
    ("eyeSclera", "Eye Sclera"),
    ("iris", "Iris and Pupil"),
    ("lips", "Lips"),
    ("teeth", "Teeth"),
    ("hair", "Hair"),
    ("clothes", "Clothes"),
];

pub fn part_label(id: &str) -> Option<&'static str> {
    PEOPLE_PARTS.iter().find(|p| p.0 == id).map(|p| p.1)
}

/// The body model's classes for a part (by name).
pub fn body_classes(part: &str) -> &'static [&'static str] {
    match part {
        "person" => &[
            "Hat",
            "Hair",
            "Sunglasses",
            "Upper-clothes",
            "Skirt",
            "Pants",
            "Dress",
            "Belt",
            "Left-shoe",
            "Right-shoe",
            "Face",
            "Left-leg",
            "Right-leg",
            "Left-arm",
            "Right-arm",
            "Bag",
            "Scarf",
        ],
        "bodySkin" => &["Left-leg", "Right-leg", "Left-arm", "Right-arm"],
        "hair" => &["Hair"],
        "clothes" => &["Hat", "Sunglasses", "Upper-clothes", "Skirt", "Pants", "Dress", "Belt", "Left-shoe", "Right-shoe", "Scarf"],
        _ => &[],
    }
}

/// The face model's classes for a part (by name). Eye sclera / iris and teeth are further split by
/// the pixels ([`split_eyes`], [`teeth`]).
pub fn face_classes(part: &str) -> &'static [&'static str] {
    match part {
        "person" => &[
            "skin", "nose", "eye_g", "l_eye", "r_eye", "l_brow", "r_brow", "l_ear", "r_ear", "mouth", "u_lip", "l_lip", "hair", "hat", "ear_r",
            "neck",
        ],
        "faceSkin" => &["skin", "nose", "l_ear", "r_ear"],
        "bodySkin" => &["neck"],
        "eyebrows" => &["l_brow", "r_brow"],
        "eyeSclera" | "iris" => &["l_eye", "r_eye"],
        "lips" => &["u_lip", "l_lip"],
        "teeth" => &["mouth"],
        "hair" => &["hair"],
        "clothes" => &["cloth", "hat", "ear_r", "eye_g"],
        _ => &[],
    }
}

/// Class indices of `names` among `labels` (case-insensitive).
pub fn indices(labels: &[String], names: &[&str]) -> Vec<usize> {
    labels.iter().enumerate().filter(|(_, l)| names.iter().any(|n| n.eq_ignore_ascii_case(l))).map(|(i, _)| i).collect()
}

/// Rec. 709 luma of an RGB triple (0..255).
fn luma(p: [f32; 3]) -> f32 {
    0.2126 * p[0] + 0.7152 * p[1] + 0.0722 * p[2]
}

fn saturation(p: [f32; 3]) -> f32 {
    let (mx, mn) = (p[0].max(p[1]).max(p[2]), p[0].min(p[1]).min(p[2]));
    if mx <= 0.0 { 0.0 } else { (mx - mn) / mx }
}

/// The value at quantile `q` (0..1) of `v` (empty: 0).
fn quantile(v: &mut [f32], q: f32) -> f32 {
    if v.is_empty() {
        return 0.0;
    }
    v.sort_by(f32::total_cmp);
    let i = ((v.len() - 1) as f32 * q.clamp(0.0, 1.0)).round() as usize;
    v.get(i).copied().unwrap_or(0.0)
}

/// Splits the eyes' probability `eye` (cells of a `w × h` grid over the interleaved RGB `rgb`
/// sampled at the same cells) into (sclera, iris and pupil): within each eye, the iris and pupil
/// are its darker cells (below the midpoint between its dark and bright ends), the sclera the rest.
pub fn split_eyes(eye: &[f32], rgb: &[f32], w: usize, h: usize) -> (Vec<f32>, Vec<f32>) {
    let n = w * h;
    let mut sclera = vec![0f32; n];
    let mut iris = vec![0f32; n];
    let px = |i: usize| -> [f32; 3] {
        [rgb.get(i * 3).copied().unwrap_or(0.0), rgb.get(i * 3 + 1).copied().unwrap_or(0.0), rgb.get(i * 3 + 2).copied().unwrap_or(0.0)]
    };
    for comp in components(eye, w, h, 0.5) {
        let mut l: Vec<f32> = comp.iter().map(|&i| luma(px(i))).collect();
        let (dark, bright) = (quantile(&mut l, 0.15), quantile(&mut l, 0.9));
        let cut = 0.5 * (dark + bright);
        let soft = ((bright - dark) * 0.15).max(2.0);
        // the eye's cells and their soft edge around them
        for &i in &comp {
            let p = eye.get(i).copied().unwrap_or(0.0);
            let t = ((luma(px(i)) - cut) / soft).clamp(-1.0, 1.0) * 0.5 + 0.5; // 1 = bright
            if let (Some(s), Some(r)) = (sclera.get_mut(i), iris.get_mut(i)) {
                *s = p * t;
                *r = p * (1.0 - t);
            }
        }
    }
    // the eye's soft fringe (below 0.5, outside every component) goes with the sclera
    for (s, p) in sclera.iter_mut().zip(eye) {
        if *p <= 0.5 {
            *s = *p;
        }
    }
    (sclera, iris)
}

/// Teeth within the open mouth's probability `mouth`: its bright, unsaturated cells.
pub fn teeth(mouth: &[f32], rgb: &[f32], w: usize, h: usize) -> Vec<f32> {
    let n = w * h;
    let px = |i: usize| -> [f32; 3] {
        [rgb.get(i * 3).copied().unwrap_or(0.0), rgb.get(i * 3 + 1).copied().unwrap_or(0.0), rgb.get(i * 3 + 2).copied().unwrap_or(0.0)]
    };
    let mut out = vec![0f32; n];
    let inside: Vec<usize> = (0..n).filter(|i| mouth.get(*i).is_some_and(|p| *p > 0.5)).collect();
    if inside.is_empty() {
        return out;
    }
    let mut l: Vec<f32> = inside.iter().map(|&i| luma(px(i))).collect();
    let (dark, bright) = (quantile(&mut l, 0.1), quantile(&mut l, 0.95));
    if bright - dark < 20.0 {
        return out; // a closed or dark mouth: no teeth showing
    }
    let cut = dark + 0.55 * (bright - dark);
    let soft = ((bright - dark) * 0.12).max(2.0);
    for i in 0..n {
        let p = mouth.get(i).copied().unwrap_or(0.0);
        if p <= 0.0 {
            continue;
        }
        let c = px(i);
        let b = ((luma(c) - cut) / soft).clamp(-1.0, 1.0) * 0.5 + 0.5;
        let s = 1.0 - ((saturation(c) - 0.35) / 0.1).clamp(0.0, 1.0);
        if let Some(o) = out.get_mut(i) {
            *o = p * b * s;
        }
    }
    out
}

/// 4-connected components of the cells above `t` (cell indices, largest first; capped at 64).
pub fn components(p: &[f32], w: usize, h: usize, t: f32) -> Vec<Vec<usize>> {
    let n = w * h;
    let mut seen = vec![false; n];
    let mut out: Vec<Vec<usize>> = Vec::new();
    for start in 0..n {
        if seen.get(start).copied().unwrap_or(true) || p.get(start).is_none_or(|v| *v <= t) {
            continue;
        }
        let mut comp = Vec::new();
        let mut stack = vec![start];
        if let Some(s) = seen.get_mut(start) {
            *s = true;
        }
        while let Some(i) = stack.pop() {
            comp.push(i);
            let (x, y) = (i % w, i / w);
            let mut push = |j: usize| {
                if p.get(j).is_some_and(|v| *v > t)
                    && let Some(s) = seen.get_mut(j)
                    && !*s
                {
                    *s = true;
                    stack.push(j);
                }
            };
            if x > 0 {
                push(i - 1);
            }
            if x + 1 < w {
                push(i + 1);
            }
            if y > 0 {
                push(i - w);
            }
            if y + 1 < h {
                push(i + w);
            }
        }
        out.push(comp);
    }
    out.sort_by_key(|c| std::cmp::Reverse(c.len()));
    out.truncate(64);
    out
}

/// The bounding box `[x0, y0, x1, y1]` (cells, exclusive end) of `cells` on a `w`-wide grid.
pub fn bbox(cells: &[usize], w: usize) -> Option<[usize; 4]> {
    if w == 0 {
        return None;
    }
    let mut b = [usize::MAX, usize::MAX, 0, 0];
    for &i in cells {
        let (x, y) = (i % w, i / w);
        b = [b[0].min(x), b[1].min(y), b[2].max(x + 1), b[3].max(y + 1)];
    }
    (b[0] != usize::MAX).then_some(b)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_parser_is_pinned_and_every_part_has_classes() {
        for p in ALL {
            assert!(p.files.iter().all(|f| f.size.is_some() && f.sha256.is_some()), "{}", p.id);
            assert!(!p.mirrors.is_empty() && p.mirrors.iter().all(|m| m.starts_with("https://")));
            assert!(p.bytes() > 100 << 20);
        }
        for (id, _) in PEOPLE_PARTS {
            assert!(!body_classes(id).is_empty() || !face_classes(id).is_empty(), "{id}");
        }
        assert_eq!(landscape_label("naturalGround"), Some("Natural Ground"));
        assert!(landscape_label("nope").is_none());
        let labels: Vec<String> = ["Background", "Hair", "Face"].iter().map(|s| s.to_string()).collect();
        assert_eq!(indices(&labels, &["hair", "face"]), [1, 2]);
    }

    #[test]
    fn eyes_split_into_sclera_and_iris_by_brightness() {
        // one eye, 6 × 1 cells: white, white, dark, dark, white, white
        let eye = [1.0f32; 6];
        let rgb: Vec<f32> = [230.0, 235.0, 40.0, 30.0, 228.0, 232.0].iter().flat_map(|v| [*v, *v, *v]).collect();
        let (s, i) = split_eyes(&eye, &rgb, 6, 1);
        assert!(s[0] > 0.9 && s[5] > 0.9 && i[0] < 0.1);
        assert!(i[2] > 0.9 && i[3] > 0.9 && s[2] < 0.1);
        // nothing there: nothing selected
        let (s, i) = split_eyes(&[0.0; 6], &rgb, 6, 1);
        assert!(s.iter().chain(&i).all(|v| *v == 0.0));
    }

    #[test]
    fn teeth_are_the_bright_unsaturated_part_of_the_mouth() {
        // mouth of 4 cells: two white teeth, a red tongue, a dark gap
        let mouth = [1.0f32; 4];
        let rgb = [235.0, 232.0, 225.0, 240.0, 238.0, 230.0, 200.0, 60.0, 70.0, 30.0, 20.0, 20.0];
        let t = teeth(&mouth, &rgb, 4, 1);
        assert!(t[0] > 0.9 && t[1] > 0.9, "{t:?}");
        assert!(t[2] < 0.1 && t[3] < 0.1, "{t:?}");
        // a closed mouth (all one dark shade): none
        assert!(teeth(&mouth, &[50.0; 12], 4, 1).iter().all(|v| *v == 0.0));
    }

    #[test]
    fn components_and_boxes() {
        let p = [1.0f32, 1.0, 0.0, 0.0, 0.0, 0.0, 0.0, 1.0, 1.0];
        let c = components(&p, 3, 3, 0.5);
        assert_eq!(c.len(), 2);
        let mut boxes: Vec<[usize; 4]> = c.iter().filter_map(|c| bbox(c, 3)).collect();
        boxes.sort();
        assert_eq!(boxes, [[0, 0, 2, 1], [1, 2, 3, 3]]);
        assert!(bbox(&[], 3).is_none());
        assert!(components(&[], 0, 0, 0.5).is_empty());
    }
}
