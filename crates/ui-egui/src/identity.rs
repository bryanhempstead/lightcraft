//! The identity plate (Lightroom Classic's Edit ▸ Identity Plate Setup, made better): a logo —
//! a transparent PNG / SVG / WebP / JPEG — or a styled line of text, drawn as decoration in the
//! top bar (where Classic puts it), in a corner of the window or in a corner of the photo area.
//!
//! - Off by default. Settings ▸ identity. sets it up; `app.identityPlate` gets / sets it.
//! - Purely decorative: painted, never interactive (clicks pass straight through), laid out where
//!   no control is (it slides along its edge away from buttons) and faded while the pointer is
//!   over it.
//! - A tint color (picked on a color wheel) multiplies the image's RGB, so a white mark takes the
//!   color exactly; alpha is kept. `none.` keeps the original colors.
//! - The picked file is copied into `<config>/identity/` (so the original may move or go); a file
//!   already in that folder is used where it is.
//! - A missing, huge or corrupt file gives an error (a notice in the app, an error from the
//!   command), never a panic: the file size and the decoded pixel count are capped, and the
//!   decoded image is fitted within [`MAX_SIDE`].

use std::path::{Path, PathBuf};

use egui::{Color32, ColorImage, Rect, Vec2, pos2, vec2};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};

/// Largest plate file read (bytes).
pub const MAX_FILE_BYTES: u64 = 32 << 20;
/// Largest source image decoded (pixels); bigger is refused before decoding.
pub const MAX_PIXELS: u64 = 1 << 26;
/// The decoded plate is fitted within this many pixels on its long side.
pub const MAX_SIDE: u32 = 1024;
/// Plate height range (points).
pub const SIZE_RANGE: std::ops::RangeInclusive<f32> = 12.0..=200.0;
/// Margin range (points).
pub const MARGIN_RANGE: std::ops::RangeInclusive<f32> = 0.0..=80.0;

/// Image file or styled text.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum PlateKind {
    #[default]
    Image,
    Text,
}

/// Where the plate goes.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub enum Place {
    /// The top bar's free space, left of the search field (Lightroom Classic's spot).
    #[default]
    #[serde(rename = "bar")]
    Bar,
    #[serde(rename = "tl")]
    Tl,
    #[serde(rename = "tr")]
    Tr,
    #[serde(rename = "bl")]
    Bl,
    #[serde(rename = "br")]
    Br,
    #[serde(rename = "canvas.tl")]
    CanvasTl,
    #[serde(rename = "canvas.tr")]
    CanvasTr,
    #[serde(rename = "canvas.bl")]
    CanvasBl,
    #[serde(rename = "canvas.br")]
    CanvasBr,
}

impl Place {
    pub const ALL: [Place; 9] =
        [Place::Bar, Place::Tl, Place::Tr, Place::Bl, Place::Br, Place::CanvasTl, Place::CanvasTr, Place::CanvasBl, Place::CanvasBr];

    /// (right side?, bottom?) of a corner; `None` for the bar.
    fn corner(self) -> Option<(bool, bool)> {
        match self {
            Place::Bar => None,
            Place::Tl | Place::CanvasTl => Some((false, false)),
            Place::Tr | Place::CanvasTr => Some((true, false)),
            Place::Bl | Place::CanvasBl => Some((false, true)),
            Place::Br | Place::CanvasBr => Some((true, true)),
        }
    }

    fn on_canvas(self) -> bool {
        matches!(self, Place::CanvasTl | Place::CanvasTr | Place::CanvasBl | Place::CanvasBr)
    }

    /// The JSON name (`bar`, `tl`, …, `canvas.br`).
    pub fn name(self) -> String {
        serde_json::to_value(self).ok().and_then(|v| v.as_str().map(str::to_string)).unwrap_or_default()
    }
}

/// The identity plate's settings (saved with the app settings in `ui.json`, key
/// `settings.identityPlate`).
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default, rename_all = "camelCase")]
pub struct IdentityPlate {
    pub on: bool,
    pub kind: PlateKind,
    /// The image (normally a copy in `<config>/identity/`).
    pub image: String,
    pub text: String,
    /// Text plate in the semibold face.
    pub bold: bool,
    pub place: Place,
    /// Height in points (a text plate's line height).
    pub size: f32,
    /// 0..1.
    pub opacity: f32,
    /// Distance from the window / photo-area edges (points).
    pub margin: f32,
    /// Shown in Library, Develop, Map.
    pub library: bool,
    pub develop: bool,
    pub map: bool,
    /// `#rrggbb` multiplied into the plate's color; `None` = original colors (a text plate then
    /// uses the theme's text color).
    pub tint: Option<String>,
}

impl Default for IdentityPlate {
    fn default() -> Self {
        IdentityPlate {
            on: false,
            kind: PlateKind::Image,
            image: String::new(),
            text: String::new(),
            bold: true,
            place: Place::Bar,
            size: 28.0,
            opacity: 0.85,
            margin: 12.0,
            library: true,
            develop: true,
            map: true,
            tint: None,
        }
    }
}

impl IdentityPlate {
    /// Clamp the numbers into range (a hand-edited `ui.json`, an agent's parameters).
    pub fn sanitize(&mut self) {
        let fin = |v: f32, d: f32| if v.is_finite() { v } else { d };
        self.size = fin(self.size, 28.0).clamp(*SIZE_RANGE.start(), *SIZE_RANGE.end());
        self.opacity = fin(self.opacity, 0.85).clamp(0.0, 1.0);
        self.margin = fin(self.margin, 12.0).clamp(*MARGIN_RANGE.start(), *MARGIN_RANGE.end());
        if self.tint.as_deref().is_some_and(|t| parse_hex(t).is_none()) {
            self.tint = None;
        }
        self.text = self.text.chars().take(120).collect();
    }

    pub fn tint_rgb(&self) -> Option<[u8; 3]> {
        self.tint.as_deref().and_then(parse_hex)
    }
}

/// `#rrggbb` / `rrggbb` / `#rgb` → RGB.
pub fn parse_hex(s: &str) -> Option<[u8; 3]> {
    let h = s.trim().trim_start_matches('#');
    let digits: Vec<u8> = h.chars().map(|c| c.to_digit(16).and_then(|d| u8::try_from(d).ok())).collect::<Option<_>>()?;
    match digits.as_slice() {
        [r, g, b] => Some([r * 17, g * 17, b * 17]),
        [r1, r0, g1, g0, b1, b0] => Some([r1 * 16 + r0, g1 * 16 + g0, b1 * 16 + b0]),
        _ => None,
    }
}

pub fn to_hex(c: [u8; 3]) -> String {
    format!("#{:02x}{:02x}{:02x}", c[0], c[1], c[2])
}

/// Swatches offered next to the color wheel: white, the warm tan, light grey, black, the accent.
pub const SWATCHES: [(&str, &str); 5] = [("white", "#ffffff"), ("tan", "#d8c8a8"), ("grey", "#9a9a9a"), ("black", "#000000"), ("blue", "#0165dd")];

// ------------------------------------------------------------------------------ decoding

fn looks_like_svg(path: &Path, bytes: &[u8]) -> bool {
    if path.extension().and_then(|e| e.to_str()).is_some_and(|e| e.eq_ignore_ascii_case("svg") || e.eq_ignore_ascii_case("svgz")) {
        return true;
    }
    let head = String::from_utf8_lossy(bytes.get(..bytes.len().min(512)).unwrap_or_default()).to_ascii_lowercase();
    let head = head.trim_start_matches('\u{feff}').trim_start();
    head.starts_with("<svg") || (head.starts_with("<?xml") && head.contains("<svg"))
}

/// Decode plate bytes (raster or SVG) into an sRGB image with alpha, fitted within
/// [`MAX_SIDE`]. Errors for anything it can't read; never panics.
pub fn decode(path: &Path, bytes: &[u8]) -> Result<ColorImage, String> {
    if bytes.len() as u64 > MAX_FILE_BYTES {
        return Err(format!("the file is {} MB; an identity plate can be at most {} MB", bytes.len() >> 20, MAX_FILE_BYTES >> 20));
    }
    if bytes.is_empty() {
        return Err("the file is empty".into());
    }
    if looks_like_svg(path, bytes) {
        return decode_svg(bytes);
    }
    let opts = lightcraft_codecs::DecodeOptions { max_size: Some((MAX_SIDE, MAX_SIDE)), max_pixels: MAX_PIXELS };
    let d = lightcraft_codecs::decode(bytes, opts).map_err(|e| format!("can't read the image: {e}"))?;
    let rgb = d.image.to_srgb8();
    let (w, h) = (rgb.width, rgb.height);
    if w == 0 || h == 0 {
        return Err("the image has no pixels".into());
    }
    let alpha = d.alpha.as_ref().filter(|a| a.width == w && a.height == h);
    let mut px = Vec::with_capacity(w.saturating_mul(h).saturating_mul(4));
    for (i, p) in rgb.data.iter().enumerate() {
        let a = alpha.and_then(|a| a.data.get(i)).map_or(255, |a| (a.clamp(0.0, 1.0) * 255.0).round() as u8);
        px.extend_from_slice(&[p[0], p[1], p[2], a]);
    }
    Ok(ColorImage::from_rgba_unmultiplied([w, h], &px))
}

fn decode_svg(bytes: &[u8]) -> Result<ColorImage, String> {
    use resvg::{tiny_skia, usvg};
    let guarded = std::panic::catch_unwind(|| -> Result<ColorImage, String> {
        let tree = usvg::Tree::from_data(bytes, &usvg::Options::default()).map_err(|e| format!("can't read the SVG: {e}"))?;
        let size = tree.size();
        let (sw, sh) = (size.width(), size.height());
        if !(sw.is_finite() && sh.is_finite() && sw > 0.0 && sh > 0.0) {
            return Err("the SVG has no size".into());
        }
        let k = MAX_SIDE as f32 / sw.max(sh);
        let (w, h) = (((sw * k).round() as u32).clamp(1, MAX_SIDE), ((sh * k).round() as u32).clamp(1, MAX_SIDE));
        let mut pixmap = tiny_skia::Pixmap::new(w, h).ok_or("the SVG is too large")?;
        resvg::render(&tree, tiny_skia::Transform::from_scale(w as f32 / sw, h as f32 / sh), &mut pixmap.as_mut());
        // tiny-skia's pixels are premultiplied RGBA, like egui's
        Ok(ColorImage::from_rgba_premultiplied([w as usize, h as usize], pixmap.data()))
    });
    guarded.unwrap_or_else(|_| Err("the SVG renderer failed on this file".into()))
}

/// Read and decode a plate file (size-capped before reading).
pub fn load(path: &Path) -> Result<ColorImage, String> {
    let meta = std::fs::metadata(path).map_err(|e| format!("{}: {e}", path.display()))?;
    if !meta.is_file() {
        return Err(format!("{} is not a file", path.display()));
    }
    if meta.len() > MAX_FILE_BYTES {
        return Err(format!("{} is {} MB; an identity plate can be at most {} MB", path.display(), meta.len() >> 20, MAX_FILE_BYTES >> 20));
    }
    let bytes = std::fs::read(path).map_err(|e| format!("{}: {e}", path.display()))?;
    decode(path, &bytes).map_err(|e| format!("{}: {e}", path.display()))
}

/// Multiply the image's RGB by `tint` (premultiplied pixels: same as multiplying the straight
/// color, alpha untouched).
pub fn tinted(mut img: ColorImage, tint: Option<[u8; 3]>) -> ColorImage {
    if let Some([r, g, b]) = tint {
        let m = |c: u8, k: u8| ((u16::from(c) * u16::from(k) + 127) / 255) as u8;
        for p in &mut img.pixels {
            let [pr, pg, pb, pa] = p.to_array();
            *p = Color32::from_rgba_premultiplied(m(pr, r), m(pg, g), m(pb, b), pa);
        }
    }
    img
}

/// The identity folder (`<config>/identity`).
pub fn default_dir() -> Option<PathBuf> {
    lightcraft_engine::camera_profiles::config_dir().map(|d| d.join("identity"))
}

fn inside(path: &Path, dir: &Path) -> bool {
    match (path.canonicalize(), dir.canonicalize()) {
        (Ok(p), Ok(d)) => p.parent() == Some(d.as_path()),
        _ => false,
    }
}

/// Check that `path` is a readable plate and keep a copy in `dir` (unless it's already there):
/// returns the path the plate should use. Without a folder (`--memory`, tests) the file is used
/// where it is.
pub fn adopt(path: &Path, dir: Option<&Path>) -> Result<PathBuf, String> {
    load(path)?;
    let Some(dir) = dir else { return Ok(path.to_path_buf()) };
    if inside(path, dir) {
        return Ok(path.to_path_buf());
    }
    std::fs::create_dir_all(dir).map_err(|e| format!("can't create {}: {e}", dir.display()))?;
    let name = path.file_name().map(|n| n.to_string_lossy().to_string()).filter(|n| !n.is_empty()).unwrap_or_else(|| "plate".into());
    let bytes = std::fs::read(path).map_err(|e| format!("{}: {e}", path.display()))?;
    let (stem, ext) = match name.rsplit_once('.') {
        Some((s, e)) if !s.is_empty() => (s.to_string(), format!(".{e}")),
        _ => (name.clone(), String::new()),
    };
    for n in 1..1000u32 {
        let candidate = if n == 1 { dir.join(&name) } else { dir.join(format!("{stem}-{n}{ext}")) };
        match std::fs::read(&candidate) {
            // the same file was picked before: reuse its copy
            Ok(existing) if existing == bytes => return Ok(candidate),
            Ok(_) => continue,
            Err(_) => {
                let tmp = candidate.with_extension("tmp-copy");
                std::fs::write(&tmp, &bytes).map_err(|e| format!("can't copy to {}: {e}", dir.display()))?;
                std::fs::rename(&tmp, &candidate).map_err(|e| format!("can't copy to {}: {e}", dir.display()))?;
                return Ok(candidate);
            }
        }
    }
    Err(format!("too many plates named {name} in {}", dir.display()))
}

// ------------------------------------------------------------------------------ the command

/// `app.identityPlate`: with no parameters, the plate's settings and state; otherwise the given
/// fields change (`on`, `kind`, `image`, `text`, `bold`, `place`, `size`, `opacity`, `margin`,
/// `library`, `develop`, `map`, `tint` — `"#rrggbb"` or `null` for none, `show`: `"all"` or a list
/// of modules). An `image` is checked and copied into the identity folder first; a bad one
/// changes nothing.
pub fn command(app: &mut crate::LightcraftApp, p: &Value) -> Result<Value, String> {
    let mut plate = app.ui.settings.identity_plate.clone();
    let Some(obj) = p.as_object().filter(|o| !o.is_empty()) else { return Ok(state(app)) };
    let mut v = serde_json::to_value(&plate).map_err(|e| e.to_string())?;
    let Some(cur) = v.as_object_mut() else { return Err("app.identityPlate: internal state".into()) };
    for (k, val) in obj {
        match k.as_str() {
            "image" => {
                let path = val.as_str().ok_or("app.identityPlate: `image` must be a path")?.trim();
                let path = if path.is_empty() {
                    String::new()
                } else {
                    let dir = app.identity_dir.clone();
                    adopt(Path::new(path), dir.as_deref()).map_err(|e| format!("app.identityPlate: {e}"))?.to_string_lossy().to_string()
                };
                cur.insert("image".into(), json!(path));
                if !path.is_empty() && obj.get("kind").is_none() {
                    cur.insert("kind".into(), json!("image"));
                }
            }
            "tint" => {
                let t = match val {
                    Value::Null => Value::Null,
                    Value::String(s) if s.trim().is_empty() || s.trim() == "none" => Value::Null,
                    Value::String(s) => json!(to_hex(parse_hex(s).ok_or_else(|| format!("app.identityPlate: `tint` {s:?} is not a #rrggbb color"))?)),
                    _ => return Err("app.identityPlate: `tint` must be \"#rrggbb\" or null".into()),
                };
                cur.insert("tint".into(), t);
            }
            "show" => {
                let mods: Vec<String> = match val {
                    Value::String(s) if s == "all" => vec!["library".into(), "develop".into(), "map".into()],
                    Value::String(s) => vec![s.clone()],
                    Value::Array(a) => a.iter().filter_map(|m| m.as_str().map(str::to_string)).collect(),
                    _ => return Err("app.identityPlate: `show` must be \"all\" or a list of library / develop / map".into()),
                };
                for m in ["library", "develop", "map"] {
                    cur.insert(m.into(), json!(mods.iter().any(|x| x == m)));
                }
            }
            "on" | "kind" | "text" | "bold" | "place" | "size" | "opacity" | "margin" | "library" | "develop" | "map" => {
                cur.insert(k.clone(), val.clone());
            }
            other => return Err(format!("app.identityPlate: unknown parameter `{other}`")),
        }
    }
    plate = serde_json::from_value(v).map_err(|e| format!("app.identityPlate: {e}"))?;
    plate.sanitize();
    app.ui.settings.identity_plate = plate;
    Ok(state(app))
}

/// The settings plus what's drawn: `loaded` (the decoded size) or `error`, and `rect` (points,
/// last frame) when it's on screen.
pub fn state(app: &crate::LightcraftApp) -> Value {
    let mut v = serde_json::to_value(&app.ui.settings.identity_plate).unwrap_or(Value::Null);
    if let Some(o) = v.as_object_mut() {
        o.insert("loaded".into(), app.identity.texture.as_ref().map_or(Value::Null, |t| json!(t.size())));
        o.insert("error".into(), app.identity.error.clone().map_or(Value::Null, Value::from));
        o.insert("rect".into(), app.identity.rect.map_or(Value::Null, |r| json!([r.min.x, r.min.y, r.max.x, r.max.y])));
        o.insert("folder".into(), app.identity_dir.as_ref().map_or(Value::Null, |d| json!(d.display().to_string())));
    }
    v
}

// ------------------------------------------------------------------------------ drawing

/// The decoded plate (a texture), kept until the file, its date or the tint change.
#[derive(Default)]
pub struct Cache {
    key: Option<(String, Option<std::time::SystemTime>, Option<[u8; 3]>)>,
    pub texture: Option<egui::TextureHandle>,
    /// The texture's pixels, for screenshots drawn off-screen (`ui.screenshot` headless).
    pub pixels: Option<std::sync::Arc<ColorImage>>,
    pub error: Option<String>,
    /// Where the plate was drawn last frame (points), if it was.
    pub rect: Option<Rect>,
}

/// Load (or reload) the image plate when its file or tint changed; an error becomes a notice once.
fn ensure(app: &mut crate::LightcraftApp, ctx: &egui::Context) {
    let plate = &app.ui.settings.identity_plate;
    let path = plate.image.trim().to_string();
    if path.is_empty() {
        app.identity.key = None;
        app.identity.texture = None;
        app.identity.pixels = None;
        app.identity.error = None;
        return;
    }
    let modified = std::fs::metadata(&path).and_then(|m| m.modified()).ok();
    let key = (path.clone(), modified, plate.tint_rgb());
    if app.identity.key.as_ref() == Some(&key) {
        return;
    }
    app.identity.key = Some(key);
    match load(Path::new(&path)) {
        Ok(img) => {
            let img = std::sync::Arc::new(tinted(img, plate.tint_rgb()));
            app.identity.texture = Some(ctx.load_texture("identity-plate", egui::ImageData::Color(img.clone()), egui::TextureOptions::LINEAR));
            app.identity.pixels = Some(img);
            app.identity.error = None;
        }
        Err(e) => {
            app.identity.texture = None;
            app.identity.pixels = None;
            app.notices.push(format!("Identity plate: {e}. Pick another image in Settings ▸ identity."));
            app.identity.error = Some(e);
        }
    }
}

/// Is the plate shown in the current module?
pub fn shown_here(app: &crate::LightcraftApp) -> bool {
    let p = &app.ui.settings.identity_plate;
    if !p.on || app.ui.fullscreen {
        return false;
    }
    if app.ui.view == crate::state::ViewMode::Map {
        p.map
    } else if app.ui.module == crate::state::Module::Develop {
        p.develop
    } else {
        p.library
    }
}

/// Memory key: the top bar's free space this frame (set by the top bar).
pub const BAR_SLOT: &str = "identity-bar-slot";

/// Registered widgets that count as controls the plate must not cover (not the photo cells,
/// thumbnails or big areas).
pub fn controls(widgets: &[(String, Rect)], window: Rect) -> Vec<Rect> {
    let big = window.area() * 0.08;
    widgets
        .iter()
        .filter(|(id, r)| {
            let skip = ["cell:", "film:", "thumb", "photo:", "canvas", "panel:", "map:", "grid:", "loupe", "image:", "pane:"];
            r.is_positive() && r.area() < big && !skip.iter().any(|s| id.starts_with(s))
        })
        .map(|(_, r)| *r)
        .collect()
}

/// Where a plate of `size` goes: `place` in `window` / `canvas` / the top bar's `slot`, `margin`
/// in from the edges, slid along its edge until it covers none of `controls`. Returns the rect
/// and whether it found free space (otherwise it sits in the corner, faded).
pub fn layout(place: Place, size: Vec2, margin: f32, window: Rect, canvas: Rect, slot: Option<Rect>, controls: &[Rect]) -> (Rect, bool) {
    let free = |r: &Rect| !controls.iter().any(|c| c.intersects(*r));
    if place == Place::Bar
        && let Some(slot) = slot.filter(|s| s.width() > 8.0 && s.height() > 4.0)
    {
        // fitted into the bar's free space, left-aligned, vertically centred
        let k = (slot.height() / size.y.max(1.0)).min(slot.width() / size.x.max(1.0)).min(1.0);
        let s = size * k;
        let r = Rect::from_min_size(pos2(slot.left(), slot.center().y - s.y / 2.0), s);
        return (r, free(&r));
    }
    let (right, bottom) = place.corner().unwrap_or((false, false));
    let container = if place.on_canvas() && canvas.is_positive() { canvas } else { window };
    let inner = container.shrink(margin.min(container.width() / 4.0).min(container.height() / 4.0));
    let k = (inner.width() * 0.5 / size.x.max(1.0)).min(inner.height() * 0.5 / size.y.max(1.0)).min(1.0);
    let s = size * k.max(0.0);
    let at = |dx: f32, dy: f32| {
        let x = if right { inner.right() - s.x - dx } else { inner.left() + dx };
        let y = if bottom { inner.bottom() - s.y - dy } else { inner.top() + dy };
        Rect::from_min_size(pos2(x, y), s)
    };
    let corner = at(0.0, 0.0);
    if free(&corner) {
        return (corner, true);
    }
    // slide away from the corner along either edge, nearest free spot first
    let (max_x, max_y) = ((inner.width() - s.x).max(0.0), (inner.height() - s.y).max(0.0));
    let mut d = 8.0;
    while d <= max_x.max(max_y) {
        for r in [(d <= max_x).then(|| at(d, 0.0)), (d <= max_y).then(|| at(0.0, d))].into_iter().flatten() {
            if free(&r) {
                return (r, true);
            }
        }
        d += 8.0;
    }
    (corner, false)
}

/// The plate's natural size (points) and what to draw.
enum Content {
    Image(egui::TextureId, Vec2),
    Text(std::sync::Arc<egui::Galley>, egui::FontId),
}

fn content(app: &crate::LightcraftApp, ctx: &egui::Context, plate: &IdentityPlate, size: f32) -> Option<Content> {
    match plate.kind {
        PlateKind::Image => {
            let tex = app.identity.texture.as_ref()?;
            let [w, h] = tex.size();
            if w == 0 || h == 0 {
                return None;
            }
            let k = size / h as f32;
            Some(Content::Image(tex.id(), vec2(w as f32 * k, size)))
        }
        PlateKind::Text => {
            let text = plate.text.trim();
            if text.is_empty() {
                return None;
            }
            let t = crate::theme::Tokens::get(ctx);
            let font = if plate.bold { t.semibold(size * 0.72) } else { t.font(size * 0.72) };
            let galley = ctx.fonts_mut(|f| f.layout_no_wrap(text.to_string(), font.clone(), Color32::WHITE));
            Some(Content::Text(galley, font))
        }
    }
}

/// The color a plate is painted with (texture tint is baked in; text takes the tint or the
/// theme's text color), at `alpha`.
fn paint_content(painter: &egui::Painter, c: &Content, r: Rect, plate: &IdentityPlate, alpha: f32) {
    let a = alpha.clamp(0.0, 1.0);
    match c {
        Content::Image(id, _) => {
            painter.image(*id, r, Rect::from_min_max(pos2(0.0, 0.0), pos2(1.0, 1.0)), Color32::WHITE.gamma_multiply(a));
        }
        Content::Text(g, font) => {
            let t = crate::theme::Tokens::get(painter.ctx());
            let col = plate.tint_rgb().map_or(t.text, |[r, g, b]| Color32::from_rgb(r, g, b));
            // fitted (the bar): the same text at the fitted size
            let k = r.height() / g.size().y.max(1.0);
            let font = egui::FontId::new(font.size * k, font.family.clone());
            painter.text(r.left_center(), egui::Align2::LEFT_CENTER, g.text(), font, col.gamma_multiply(a));
        }
    }
}

impl Content {
    fn size(&self) -> Vec2 {
        match self {
            Content::Image(_, s) => *s,
            Content::Text(g, _) => g.size(),
        }
    }
}

/// Draw the plate for this frame (after the panels and the photo area, under dialogs and menus).
/// `canvas` is the photo area.
pub fn paint(app: &mut crate::LightcraftApp, ui: &egui::Ui, canvas: Rect) {
    app.identity.rect = None;
    if !shown_here(app) {
        return;
    }
    let ctx = ui.ctx().clone();
    if app.ui.settings.identity_plate.kind == PlateKind::Image {
        ensure(app, &ctx);
    }
    let plate = app.ui.settings.identity_plate.clone();
    let Some(c) = content(app, &ctx, &plate, plate.size) else { return };
    let window = ctx.content_rect();
    let slot = ctx.data(|d| d.get_temp::<Rect>(egui::Id::new(BAR_SLOT))).filter(|_| app.ui.top_panel);
    let widgets = ctx.data(|d| d.get_temp::<crate::widgets::Registry>(egui::Id::new("lc-registry"))).unwrap_or_default();
    let ctl = controls(&widgets.0, window);
    let (r, clear) = layout(plate.place, c.size(), plate.margin, window, canvas, slot, &ctl);
    // under the pointer it fades away, so whatever is beneath stays visible and clickable
    let hover = ctx.pointer_hover_pos().is_some_and(|p| r.expand(6.0).contains(p));
    let fade = ctx.animate_value_with_time(egui::Id::new("identity-fade"), if hover { 0.12 } else { 1.0 }, 0.15);
    let alpha = plate.opacity * fade * if clear { 1.0 } else { 0.35 };
    paint_content(ui.painter(), &c, r, &plate, alpha);
    app.identity.rect = Some(r);
}

/// The Settings preview: a small window mock-up with the plate where it will go.
pub fn preview(app: &mut crate::LightcraftApp, ui: &mut egui::Ui, size: Vec2) {
    let t = crate::theme::Tokens::get(ui.ctx());
    let (outer, _) = ui.allocate_exact_size(size, egui::Sense::hover());
    crate::widgets::register(ui.ctx(), "preview:identityPlate", outer);
    let p = ui.painter_at(outer);
    let window = ctx_window(ui.ctx());
    // the mock-up keeps the window's proportions
    let k = (outer.width() / window.width().max(1.0)).min(outer.height() / window.height().max(1.0));
    let mock = Rect::from_center_size(outer.center(), window.size() * k);
    let bar_h = t.top_bar_h * k;
    let side = t.panel_w * k;
    let film = t.film_h * k;
    p.rect_filled(mock, 3.0, t.grid_bg);
    let bar = Rect::from_min_max(mock.min, pos2(mock.right(), mock.top() + bar_h));
    p.rect_filled(bar, 0.0, t.header);
    let left = Rect::from_min_max(pos2(mock.left(), bar.bottom()), pos2(mock.left() + side, mock.bottom() - film));
    let right = Rect::from_min_max(pos2(mock.right() - side, bar.bottom()), pos2(mock.right(), mock.bottom() - film));
    let strip = Rect::from_min_max(pos2(mock.left(), mock.bottom() - film), mock.max);
    for r in [left, right, strip] {
        p.rect_filled(r, 0.0, t.header);
    }
    let canvas = Rect::from_min_max(pos2(left.right(), bar.bottom()), pos2(right.left(), strip.top()));
    p.rect_filled(canvas, 0.0, t.grid_bg);
    p.rect_stroke(mock, 3.0, egui::Stroke::new(1.0, t.field_border), egui::StrokeKind::Inside);
    if app.ui.settings.identity_plate.kind == PlateKind::Image {
        ensure(app, ui.ctx());
    }
    let plate = app.ui.settings.identity_plate.clone();
    let Some(c) = content(app, ui.ctx(), &plate, plate.size * k) else {
        p.text(mock.center(), egui::Align2::CENTER_CENTER, crate::i18n::tr("no plate yet"), t.font(11.0), t.text_dim);
        return;
    };
    let slot = Rect::from_min_max(pos2(mock.left() + 220.0 * k, bar.top() + 4.0 * k), pos2(mock.center().x - 320.0 * k, bar.bottom() - 4.0 * k));
    let (r, _) = layout(plate.place, c.size(), plate.margin * k, mock, canvas, Some(slot), &[]);
    let alpha = if plate.on { plate.opacity } else { plate.opacity * 0.5 };
    paint_content(&p, &c, r, &plate, alpha);
}

fn ctx_window(ctx: &egui::Context) -> Rect {
    let r = ctx.content_rect();
    if r.is_positive() { r } else { Rect::from_min_size(pos2(0.0, 0.0), vec2(1600.0, 1000.0)) }
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used)]
mod tests {
    use super::*;

    /// A procedural PNG: a white ring on transparency (encoded with the codecs crate's PNG writer).
    pub(crate) fn ring_png(w: u32, h: u32) -> Vec<u8> {
        let mut px = Vec::with_capacity((w * h * 4) as usize);
        let (cx, cy, r) = (w as f32 / 2.0, h as f32 / 2.0, w.min(h) as f32 * 0.4);
        for y in 0..h {
            for x in 0..w {
                let d = ((x as f32 + 0.5 - cx).powi(2) + (y as f32 + 0.5 - cy).powi(2)).sqrt();
                let on = (d - r).abs() < r * 0.25;
                px.extend_from_slice(&[255, 255, 255, if on { 255 } else { 0 }]);
            }
        }
        let mut out = Vec::new();
        {
            let mut enc = png::Encoder::new(&mut out, w, h);
            enc.set_color(png::ColorType::Rgba);
            enc.set_depth(png::BitDepth::Eight);
            let mut wr = enc.write_header().unwrap();
            wr.write_image_data(&px).unwrap();
        }
        out
    }

    fn tmp(tag: &str) -> PathBuf {
        let d = std::env::temp_dir().join(format!("lc-identity-{tag}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&d);
        std::fs::create_dir_all(&d).unwrap();
        d
    }

    #[test]
    fn decodes_png_with_alpha_and_fits_the_cap() {
        let img = decode(Path::new("ring.png"), &ring_png(2048, 1024)).unwrap();
        assert_eq!(img.size, [1024, 512], "fitted within MAX_SIDE");
        assert_eq!(img.pixels[0].a(), 0, "corner stays transparent");
        let mid_ring = img.pixels[256 * 1024 + 512 - (512.0 * 0.4 * 0.5) as usize * 2];
        let any_opaque = img.pixels.iter().any(|p| p.a() == 255 && p.r() == 255);
        assert!(any_opaque, "ring pixels {mid_ring:?}");
    }

    #[test]
    fn corrupt_empty_and_huge_files_are_errors() {
        assert!(decode(Path::new("x.png"), b"\x89PNG\r\n\x1a\nnot really").is_err());
        assert!(decode(Path::new("x.png"), b"").is_err());
        assert!(decode(Path::new("x.svg"), b"<svg broken").is_err());
        let huge = vec![0u8; (MAX_FILE_BYTES + 1) as usize];
        let e = decode(Path::new("x.png"), &huge).unwrap_err();
        assert!(e.contains("at most"), "{e}");
        assert!(load(Path::new("/nonexistent/plate.png")).is_err());
        // a PNG header claiming more pixels than the cap is refused before decoding
        let mut big = ring_png(4, 4);
        // IHDR width / height at bytes 16..24 (CRC now wrong too: still an error, never a panic)
        big[16..20].copy_from_slice(&40000u32.to_be_bytes());
        big[20..24].copy_from_slice(&40000u32.to_be_bytes());
        assert!(decode(Path::new("x.png"), &big).is_err());
    }

    #[test]
    fn svg_plates_render() {
        let svg = br##"<svg xmlns="http://www.w3.org/2000/svg" width="200" height="100"><rect x="10" y="10" width="80" height="80" fill="#ffffff"/></svg>"##;
        let img = decode(Path::new("logo.svg"), svg).unwrap();
        assert_eq!(img.size, [1024, 512]);
        assert_eq!(img.pixels[0].a(), 0);
        assert_eq!(img.pixels[256 * 1024 + 250], Color32::WHITE);
    }

    #[test]
    fn tint_multiplies_rgb_and_keeps_alpha() {
        let img = ColorImage::from_rgba_unmultiplied([2, 1], &[255, 255, 255, 255, 255, 255, 255, 128]);
        let out = tinted(img, Some([200, 100, 50]));
        assert_eq!(out.pixels[0], Color32::from_rgb(200, 100, 50), "a white mark takes the color exactly");
        assert_eq!(out.pixels[1].a(), 128);
        assert_eq!(parse_hex("#C8a032"), Some([200, 160, 50]));
        assert_eq!(parse_hex("fff"), Some([255, 255, 255]));
        assert_eq!(parse_hex("#12345"), None);
        assert_eq!(to_hex([200, 160, 50]), "#c8a032");
    }

    #[test]
    fn adopt_copies_once_and_leaves_files_in_the_folder() {
        let d = tmp("adopt");
        let src = d.join("logo.png");
        std::fs::write(&src, ring_png(64, 64)).unwrap();
        let folder = d.join("identity");
        let a = adopt(&src, Some(&folder)).unwrap();
        assert_eq!(a, folder.join("logo.png"));
        assert!(a.exists());
        // the same file again: the same copy, no logo-2.png
        assert_eq!(adopt(&src, Some(&folder)).unwrap(), a);
        // a file already in the folder is used where it is
        assert_eq!(adopt(&a, Some(&folder)).unwrap(), a);
        assert_eq!(std::fs::read_dir(&folder).unwrap().count(), 1);
        // a different file with the same name gets its own copy
        std::fs::write(&src, ring_png(32, 32)).unwrap();
        assert_eq!(adopt(&src, Some(&folder)).unwrap(), folder.join("logo-2.png"));
        // a corrupt file is refused and not copied
        let bad = d.join("bad.png");
        std::fs::write(&bad, b"nope").unwrap();
        assert!(adopt(&bad, Some(&folder)).is_err());
        assert!(!folder.join("bad.png").exists());
        let _ = std::fs::remove_dir_all(&d);
    }

    #[test]
    fn layout_slides_off_controls() {
        let window = Rect::from_min_size(pos2(0.0, 0.0), vec2(1000.0, 600.0));
        let canvas = Rect::from_min_max(pos2(200.0, 40.0), pos2(800.0, 480.0));
        let size = vec2(80.0, 30.0);
        let (r, ok) = layout(Place::Br, size, 10.0, window, canvas, None, &[]);
        assert!(ok);
        assert_eq!(r.max, pos2(990.0, 590.0));
        // a button in the corner: the plate moves along the edge, clear of it
        let button = Rect::from_min_max(pos2(940.0, 560.0), pos2(995.0, 595.0));
        let (r, ok) = layout(Place::Br, size, 10.0, window, canvas, None, &[button]);
        assert!(ok && !r.intersects(button), "{r:?}");
        let (r, _) = layout(Place::CanvasTl, size, 10.0, window, canvas, None, &[]);
        assert_eq!(r.min, pos2(210.0, 50.0));
        // the bar: fitted to its free space
        let slot = Rect::from_min_max(pos2(300.0, 4.0), pos2(400.0, 34.0));
        let (r, _) = layout(Place::Bar, vec2(200.0, 60.0), 10.0, window, canvas, Some(slot), &[]);
        assert!(slot.contains_rect(r) && (r.width() - 100.0).abs() < 0.01, "{r:?}");
    }

    #[test]
    fn settings_sanitize_and_round_trip() {
        let mut p = IdentityPlate { size: 9999.0, opacity: f32::NAN, tint: Some("bogus".into()), ..Default::default() };
        p.sanitize();
        assert_eq!((p.size, p.opacity, p.tint), (200.0, 0.85, None));
        let p = IdentityPlate { on: true, place: Place::CanvasBr, tint: Some("#d8c8a8".into()), ..Default::default() };
        let v = serde_json::to_value(&p).unwrap();
        assert_eq!(v["place"], "canvas.br");
        assert_eq!(serde_json::from_value::<IdentityPlate>(v).unwrap(), p);
        assert!(!IdentityPlate::default().on, "off by default");
    }
}
