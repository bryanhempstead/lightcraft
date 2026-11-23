//! Export: encode a rendered image to JPEG / PNG / TIFF / WebP / AVIF (or copy the original / write a
//! DNG, each with the edits in XMP) in sRGB, Display P3, Adobe RGB
//! (1998) compatible, ProPhoto RGB or Rec. 2020 with an embedded ICC profile we generate from the
//! published primaries and curves, optional output sharpening and a JPEG file-size limit. Pure (bytes in, bytes out) so the desktop
//! app, CLI, MCP and the web build share it; writing the file is the caller's job.

pub use lightcraft_codecs::TiffCompression;
use lightcraft_codecs::{ChromaSubsampling, EncodeImage, EncodeMeta, NamedSpace, Samples, encode, icc};
use lightcraft_meta::{DateTime, Gps, Metadata};
pub use lightcraft_pipeline::{DeepImage, DeepSamples, OutputDepth, OutputSpace};
use lightcraft_raster::Rgba8;
use serde::{Deserialize, Serialize};

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum ExportFormat {
    #[default]
    Jpeg,
    Png,
    Tiff,
    Webp,
    Avif,
    /// The original file, unchanged, with an XMP sidecar holding the edits.
    Original,
    /// A DNG of the raw data (raw photos only), the edits embedded as XMP.
    Dng,
    /// JPEG XL (E07): lossy at `quality`, or lossless at quality 100.
    Jxl,
    /// Photoshop document (E07): one flat layer, 8 or 16 bit (PSB past 30,000 pixels).
    Psd,
}

impl ExportFormat {
    pub fn parse(s: &str) -> Option<Self> {
        Some(match s.to_ascii_lowercase().as_str() {
            "jpeg" | "jpg" => Self::Jpeg,
            "png" => Self::Png,
            "tiff" | "tif" => Self::Tiff,
            "webp" => Self::Webp,
            "avif" => Self::Avif,
            "original" => Self::Original,
            "dng" => Self::Dng,
            "jxl" | "jpegxl" => Self::Jxl,
            "psd" | "psb" => Self::Psd,
            _ => return None,
        })
    }
    pub fn extension(self) -> &'static str {
        match self {
            Self::Jpeg => "jpg",
            Self::Png => "png",
            Self::Tiff => "tif",
            Self::Webp => "webp",
            Self::Avif => "avif",
            // the original keeps its own extension (see `ExportOptions::file_name_for`)
            Self::Original => "",
            Self::Dng => "dng",
            Self::Jxl => "jxl",
            Self::Psd => "psd",
        }
    }

    /// Formats that render pixels (everything but `Original` and `Dng`).
    pub fn is_rendered(self) -> bool {
        !matches!(self, Self::Original | Self::Dng)
    }
}

/// How the output size is chosen (see [`Resize`]).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum ResizeMode {
    /// `value` = long edge.
    #[default]
    LongEdge,
    /// `value` = short edge.
    ShortEdge,
    /// `value` = width.
    Width,
    /// `value` = height.
    Height,
    /// Lightroom's Width & Height: fit inside `value` wide × `height` tall as the photo stands
    /// (400 × 600 makes a 400 × 600 portrait or a 400 × 267 landscape); 0 leaves that side free.
    WidthHeight,
    /// Lightroom's Dimensions: fit inside `value × height` whichever way round the photo is (the
    /// larger value on the long edge: a 400 × 600 portrait or a 600 × 400 landscape).
    Dimensions,
    /// `value` = megapixels.
    Megapixels,
    /// `value` = percent of the full size.
    Percent,
}

impl ResizeMode {
    /// The Resize to Fit menu: Lightroom's six in its order, then LightCraft's Width / Height only.
    pub const ALL: [(ResizeMode, &'static str); 8] = [
        (ResizeMode::WidthHeight, "Width & Height"),
        (ResizeMode::Dimensions, "Dimensions"),
        (ResizeMode::LongEdge, "Long Edge"),
        (ResizeMode::ShortEdge, "Short Edge"),
        (ResizeMode::Megapixels, "Megapixels"),
        (ResizeMode::Percent, "Percentage"),
        (ResizeMode::Width, "Width only"),
        (ResizeMode::Height, "Height only"),
    ];

    pub fn label(self) -> &'static str {
        Self::ALL.iter().find(|(m, _)| *m == self).map_or("Long Edge", |(_, l)| l)
    }

    /// Whether the mode's values are lengths (pixels, inches or cm), not megapixels / percent.
    pub fn is_length(self) -> bool {
        !matches!(self, Self::Megapixels | Self::Percent)
    }

    /// Whether the mode takes two values (`value` and `height`).
    pub fn is_pair(self) -> bool {
        matches!(self, Self::WidthHeight | Self::Dimensions)
    }
}

/// The unit of a [`Resize`]'s lengths (Lightroom's pixels / in / cm), converted at the export's
/// resolution.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum SizeUnit {
    #[default]
    Pixels,
    Inches,
    Cm,
}

impl SizeUnit {
    pub const ALL: [(SizeUnit, &'static str); 3] = [(SizeUnit::Pixels, "pixels"), (SizeUnit::Inches, "in"), (SizeUnit::Cm, "cm")];

    pub fn label(self) -> &'static str {
        Self::ALL.iter().find(|(u, _)| *u == self).map_or("pixels", |(_, l)| l)
    }

    /// Pixels in one of this unit at `ppi`.
    pub fn pixels_per(self, ppi: f64) -> f64 {
        match self {
            Self::Pixels => 1.0,
            Self::Inches => ppi,
            Self::Cm => ppi / 2.54,
        }
    }
}

/// The unit the Resolution field shows (files always store pixels per inch).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum ResolutionUnit {
    /// pixels per inch
    #[default]
    Inch,
    /// pixels per cm
    Cm,
}

/// Output size: a mode, its value(s) and unit, and whether smaller photos keep their own size.
#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default, rename_all = "camelCase")]
pub struct Resize {
    pub mode: ResizeMode,
    pub value: f32,
    /// Second value for [`ResizeMode::WidthHeight`] (height) and [`ResizeMode::Dimensions`].
    pub height: f32,
    /// Never upscale past the photo's own (cropped) size.
    pub dont_enlarge: bool,
    /// The unit of `value` / `height` in the length modes.
    pub unit: SizeUnit,
}

impl Default for Resize {
    fn default() -> Self {
        Self { mode: ResizeMode::LongEdge, value: 2048.0, height: 2048.0, dont_enlarge: true, unit: SizeUnit::Pixels }
    }
}

impl Resize {
    pub fn long_edge(px: u32) -> Self {
        Self { value: px as f32, ..Default::default() }
    }

    /// This size with its lengths in pixels at `ppi` (inches / cm converted; pixels, megapixels
    /// and percent as they are).
    pub fn in_pixels(&self, ppi: u16) -> Self {
        if self.unit == SizeUnit::Pixels || !self.mode.is_length() {
            return Self { unit: SizeUnit::Pixels, ..*self };
        }
        let k = self.unit.pixels_per(f64::from(ppi.max(1)));
        let px = |v: f32| (f64::from(v) * k).round().min(1.0e6) as f32;
        Self { value: px(self.value), height: px(self.height), unit: SizeUnit::Pixels, ..*self }
    }

    /// Scale factor from a `w × h` (cropped, full-resolution) photo to the output (lengths in
    /// pixels: see [`Resize::in_pixels`]).
    pub fn scale(&self, w: f64, h: f64) -> f64 {
        let (w, h) = (w.max(1.0), h.max(1.0));
        let (long, short) = (w.max(h), w.min(h));
        let v = f64::from(self.value).max(0.0);
        let v2 = f64::from(self.height).max(0.0);
        let k = match self.mode {
            ResizeMode::LongEdge => v / long,
            ResizeMode::ShortEdge => v / short,
            ResizeMode::Width => v / w,
            ResizeMode::Height => v / h,
            ResizeMode::WidthHeight => match (v > 0.0, v2 > 0.0) {
                (true, true) => (v / w).min(v2 / h),
                (true, false) => v / w,
                (false, true) => v2 / h,
                (false, false) => 1.0,
            },
            ResizeMode::Dimensions => {
                let (a, b) = (v.max(v2), v.min(v2));
                if b > 0.0 { (a / long).min(b / short) } else { a / long }
            }
            ResizeMode::Megapixels => (v * 1e6 / (w * h)).sqrt(),
            ResizeMode::Percent => v / 100.0,
        };
        let k = if self.dont_enlarge { k.min(1.0) } else { k };
        // at least one pixel, at most the encoders' 65535 limit
        k.clamp(1.0 / short, 65_535.0 / long)
    }

    /// Output size for a `w × h` (cropped, full-resolution) photo.
    pub fn apply(&self, w: f64, h: f64) -> (usize, usize) {
        let k = self.scale(w, h);
        (((w * k).round() as usize).max(1), ((h * k).round() as usize).max(1))
    }
}

/// What happens when an output file already exists.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum Conflict {
    /// Add `-2`, `-3`, … to the name.
    #[default]
    Unique,
    Overwrite,
    /// Leave the existing file and skip the photo.
    Skip,
    /// Lightroom's "Ask what to do": the desktop app lists the files that exist before it starts
    /// ([`existing_targets`]) and asks per file (new name, overwrite, skip), passing the answers as
    /// `resolve`; with nobody to ask (CLI, MCP) a taken name gets a new one, as with `Unique`.
    Ask,
}

impl Conflict {
    /// Lightroom's Existing Files menu, in its order.
    pub const ALL: [(Conflict, &'static str); 4] = [
        (Conflict::Ask, "Ask what to do"),
        (Conflict::Unique, "Choose a new name for the exported file"),
        (Conflict::Overwrite, "Overwrite WITHOUT WARNING"),
        (Conflict::Skip, "Skip"),
    ];

    pub fn label(self) -> &'static str {
        Self::ALL.iter().find(|(c, _)| *c == self).map_or("Ask what to do", |(_, l)| l)
    }
}

/// A named set of `app.export` params, in a folder of the Export dialog's preset list.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct ExportPreset {
    pub name: String,
    pub params: serde_json::Value,
    /// The preset list's folder (empty = User Presets).
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub folder: String,
}

/// The folder of the user's presets that aren't in a folder of their own.
pub const USER_PRESETS: &str = "User Presets";
/// The folder of Lightroom's built-in presets.
pub const LIGHTROOM_PRESETS: &str = "Lightroom Presets";
/// The folder of LightCraft's own built-in presets.
pub const LIGHTCRAFT_PRESETS: &str = "LightCraft Presets";

impl ExportPreset {
    /// The folder the preset list shows it in.
    pub fn folder_name(&self) -> &str {
        if self.folder.trim().is_empty() { USER_PRESETS } else { self.folder.trim() }
    }
}

/// The built-in export presets (File → Export with Preset): Lightroom's four (Burn Full-Sized
/// JPEGs writes to the hard drive here: no disc burning), then LightCraft's.
pub fn builtin_presets() -> Vec<ExportPreset> {
    let p = |folder: &str, name: &str, params: serde_json::Value| ExportPreset { name: name.into(), params, folder: folder.into() };
    let (lr, lc) = (LIGHTROOM_PRESETS, LIGHTCRAFT_PRESETS);
    vec![
        p(
            lr,
            "Burn Full-Sized JPEGs",
            serde_json::json!({"format": "jpeg", "quality": 100, "longEdge": 0, "colorSpace": "srgb", "ppi": 240,
                "exportTo": "chooseLater", "subfolder": "LightCraft Burned Exports", "service": "disc"}),
        ),
        p(lr, "Export to DNG", serde_json::json!({"format": "dng", "exportTo": "chooseLater", "after": "nothing"})),
        p(
            lr,
            "For Email",
            serde_json::json!({"format": "jpeg", "quality": 60, "longEdge": 640, "ppi": 72, "colorSpace": "srgb", "service": "email",
                "metadata": "copyrightContact"}),
        ),
        p(
            lr,
            "For Email (Hard Drive)",
            serde_json::json!({"format": "jpeg", "quality": 60, "resize": {"mode": "widthHeight", "value": 640, "height": 640}, "ppi": 72,
                "colorSpace": "srgb", "exportTo": "chooseLater", "after": "showInFinder"}),
        ),
        p(lc, "JPEG (Small)", serde_json::json!({"format": "jpeg", "quality": 85, "longEdge": 2048, "colorSpace": "srgb"})),
        p(lc, "JPEG (Large)", serde_json::json!({"format": "jpeg", "quality": 92, "longEdge": 0, "colorSpace": "srgb"})),
        p(lc, "Original + Settings", serde_json::json!({"format": "original"})),
        p(lc, "DNG", serde_json::json!({"format": "dng"})),
    ]
}

/// The Export dialog's Export To menu at the top: where the files go once written.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum ExportService {
    /// Hard Drive: the files are the result.
    #[default]
    HardDrive,
    /// Email: written to a temporary folder and attached to a new mail message (desktop).
    Email,
    /// CD/DVD: LightCraft burns no discs; the files go to the folder chosen, ready to burn.
    Disc,
    /// Apple Photos (macOS): written, then imported into Photos.
    Photos,
}

impl ExportService {
    pub const ALL: [(ExportService, &'static str); 4] = [
        (ExportService::HardDrive, "Hard Drive"),
        (ExportService::Email, "Email"),
        (ExportService::Disc, "CD/DVD"),
        (ExportService::Photos, "Apple Photos"),
    ];

    pub fn label(self) -> &'static str {
        Self::ALL.iter().find(|(s, _)| *s == self).map_or("Hard Drive", |(_, l)| l)
    }
}

/// Adobe Photoshop as installed on this Mac (`/Applications/Adobe Photoshop …/Adobe Photoshop ….app`,
/// the newest by name), for Post-Processing ▸ Open in Adobe Photoshop.
pub fn photoshop_app() -> Option<String> {
    photoshop_in(std::path::Path::new("/Applications"))
}

/// [`photoshop_app`] under `apps`.
pub fn photoshop_in(apps: &std::path::Path) -> Option<String> {
    let mut found: Vec<std::path::PathBuf> = std::fs::read_dir(apps)
        .ok()?
        .flatten()
        .map(|e| e.path())
        .filter(|p| p.file_name().is_some_and(|n| n.to_string_lossy().starts_with("Adobe Photoshop")))
        .filter_map(|p| {
            if p.extension().is_some_and(|x| x == "app") {
                return Some(p);
            }
            let rd = std::fs::read_dir(&p).ok()?;
            rd.flatten().map(|e| e.path()).find(|a| {
                a.extension().is_some_and(|x| x == "app") && a.file_name().is_some_and(|n| n.to_string_lossy().starts_with("Adobe Photoshop"))
            })
        })
        .collect();
    found.sort();
    found.pop().map(|p| p.to_string_lossy().to_string())
}

/// Output sharpening target (applied after resizing, in display-encoded values).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum SharpenFor {
    #[default]
    None,
    Screen,
    Matte,
    Glossy,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum SharpenAmount {
    Low,
    #[default]
    Standard,
    High,
}

/// Where a watermark sits.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum Anchor {
    TopLeft,
    Top,
    TopRight,
    Left,
    Center,
    Right,
    BottomLeft,
    Bottom,
    #[default]
    BottomRight,
}

/// A text watermark, sized relative to the image so every export size looks the same.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default, rename_all = "camelCase")]
pub struct Watermark {
    pub text: String,
    /// Basic Japanese vertical lettering: upright glyphs in columns from right to left.
    pub vertical: bool,
    /// Text height as a fraction of the image's short edge.
    pub size: f32,
    /// 0..1.
    pub opacity: f32,
    pub anchor: Anchor,
    /// Margin as a fraction of the short edge.
    pub inset: f32,
    /// sRGB colour.
    pub color: [u8; 3],
    /// Soft dark drop shadow for legibility on bright areas.
    pub shadow: bool,
    /// A graphic (PNG with transparency, JPEG, …) drawn instead of the text (empty = text).
    pub image: String,
    /// The graphic's width as a fraction of the photo's width.
    pub image_width: f32,
    /// The colour space the photo is in (set by the encoder; the graphic is converted into it).
    #[serde(skip)]
    pub target: Option<OutputSpace>,
}

impl Default for Watermark {
    fn default() -> Self {
        Self {
            text: String::new(),
            vertical: false,
            size: 0.035,
            opacity: 0.7,
            anchor: Anchor::BottomRight,
            inset: 0.025,
            color: [255; 3],
            shadow: true,
            image: String::new(),
            image_width: 0.2,
            target: None,
        }
    }
}

/// Inter SemiBold (OFL, see assets/ATTRIBUTION.md).
static WATERMARK_FONT: &[u8] = include_bytes!("../../../assets/fonts/Inter-SemiBold.ttf");

/// The watermark faces: Inter first, then the craft-fonts CJK faces (Mincho first, the watermark's
/// serif look; then any other CJK face — a watermark has no UI language, so every CJK script is
/// offered). Without craft-fonts that is Inter alone, and CJK characters draw as Inter's
/// missing-glyph box.
fn watermark_fonts(craft: &'static [crate::fonts::CraftFont]) -> Vec<ab_glyph::FontRef<'static>> {
    let mut cjk: Vec<_> = crate::fonts::cjk(craft).collect();
    cjk.sort_by_key(|f| !f.is_mincho());
    std::iter::once(WATERMARK_FONT)
        .chain(cjk.into_iter().map(|f| f.bytes))
        .filter_map(|bytes| ab_glyph::FontRef::try_from_slice(bytes).ok())
        .collect()
}

/// The font's vertical form, positioned from its vertical origin within the watermark cell.
/// Unlike Unicode presentation-form characters, GSUB works with BIZ UD fonts too.
fn vertical_watermark_glyph(
    font: &ab_glyph::FontRef<'_>,
    data: &harfrust::ShaperData,
    ch: char,
    px: f32,
    left: f32,
    top: f32,
) -> Option<ab_glyph::Glyph> {
    use ab_glyph::{Font, ScaleFont};
    let face = harfrust::FontRef::new(font.font_data()).ok()?;
    let mut buf = harfrust::UnicodeBuffer::new();
    buf.add(ch, 0);
    buf.set_direction(harfrust::Direction::TopToBottom);
    buf.guess_segment_properties();
    let shaped = data.shaper(&face).build().shape(buf, harfrust::ShapeOptions::new());
    if shaped.glyph_infos().len() != 1 {
        return None;
    }
    let gid = u16::try_from(shaped.glyph_infos().first()?.glyph_id).ok().filter(|g| *g != 0)?;
    let pos = shaped.glyph_positions().first()?;
    let sf = font.as_scaled(px);
    let (sx, sy) = (sf.h_scale_factor(), sf.v_scale_factor());
    let height = -(pos.y_advance as f32) * sy;
    let pen = ab_glyph::point(left + px / 2.0 + pos.x_offset as f32 * sx, top + (px - height) / 2.0 - pos.y_offset as f32 * sy);
    Some(ab_glyph::GlyphId(gid).with_scale_and_position(px, pen))
}

/// Draw `wm` onto `img` (straight alpha blending of the encoded values).
pub fn draw_watermark(img: &mut Rgba8, wm: &Watermark) {
    let width = img.width;
    watermark_coverage(img.width, img.height, wm, crate::fonts::CRAFT_FONTS, |x, y, k, col| {
        let p = &mut img.data[y * width + x];
        for c in 0..3 {
            p[c] = (p[c] as f32 + (col[c] as f32 - p[c] as f32) * k).round() as u8;
        }
    });
}

/// Draw `wm` onto a high-bit-depth image (`wm.color` is in the image's space, 8-bit encoded; float
/// images are linear and blend in linear light).
pub fn draw_watermark_deep(img: &mut DeepImage, wm: &Watermark) {
    let (width, trc) = (img.width, img.space.trc());
    match &mut img.samples {
        DeepSamples::U16(v) => watermark_coverage(img.width, img.height, wm, crate::fonts::CRAFT_FONTS, |x, y, k, col| {
            let i = (y * width + x) * 3;
            for c in 0..3 {
                let p = v[i + c] as f32;
                v[i + c] = (p + (col[c] as f32 * 257.0 - p) * k).round().clamp(0.0, 65535.0) as u16;
            }
        }),
        DeepSamples::F32(v) => watermark_coverage(img.width, img.height, wm, crate::fonts::CRAFT_FONTS, |x, y, k, col| {
            let i = (y * width + x) * 3;
            for c in 0..3 {
                let target = trc.decode(col[c] as f32 / 255.0);
                v[i + c] += (target - v[i + c]) * k;
            }
        }),
    }
}

/// Lay out `wm` on a `width × height` image and call `blend(x, y, coverage × opacity, colour)` for
/// every covered pixel (shadow pass first). `craft` is [`crate::fonts::CRAFT_FONTS`] (a parameter
/// so tests can render without it).
fn watermark_coverage(
    width: usize,
    height: usize,
    wm: &Watermark,
    craft: &'static [crate::fonts::CraftFont],
    mut blend_px: impl FnMut(usize, usize, f32, [u8; 3]),
) {
    use ab_glyph::{Font, PxScale, ScaleFont, point};
    if !wm.image.trim().is_empty() {
        logo_coverage(width, height, wm, blend_px);
        return;
    }
    let text = wm.text.trim();
    if text.is_empty() || width == 0 || height == 0 {
        return;
    }
    let fonts = watermark_fonts(craft);
    let Some(latin) = fonts.first() else { return };
    let vertical_data: Vec<_> = if wm.vertical {
        fonts.iter().map(|font| harfrust::FontRef::new(font.font_data()).ok().map(|face| harfrust::ShaperData::new(&face))).collect()
    } else {
        Vec::new()
    };
    let short = width.min(height) as f32;
    let px = (wm.size.clamp(0.005, 0.5) * short).max(6.0);
    let mut glyphs = Vec::new();
    let mut x = 0.0f32;
    let mut y = 0.0f32;
    let mut tw = 0.0f32;
    let mut prev = None;
    let columns = text.split('\n').count();
    let mut column = 0usize;
    for original in text.chars() {
        if original == '\n' {
            if wm.vertical {
                column = column.saturating_add(1);
                tw = tw.max(y);
                y = 0.0;
            } else {
                tw = tw.max(x);
                x = 0.0;
                y += px;
            }
            prev = None;
            continue;
        }
        if wm.vertical
            && matches!(original as u32, 0x3000..=0x30FF | 0x3400..=0x9FFF | 0xFF01..=0xFF60)
            && let Some(face) = fonts.iter().position(|font| font.glyph_id(original).0 != 0)
            && let (Some(font), Some(Some(data))) = (fonts.get(face), vertical_data.get(face))
        {
            let left = columns.saturating_sub(column.saturating_add(1)) as f32 * px;
            if let Some(glyph) = vertical_watermark_glyph(font, data, original, px, left, y)
                && (glyph.id != font.glyph_id(original)
                    || !matches!(original, '、' | '。' | 'ー' | '（' | '）' | '「' | '」' | '『' | '』' | '【' | '】'))
            {
                glyphs.push((face, glyph, None));
                y += px;
                continue;
            }
        }
        let vertical_form = match original {
            '、' => '︑',
            '。' => '︒',
            '（' => '︵',
            '）' => '︶',
            '「' => '﹁',
            '」' => '﹂',
            '『' => '﹃',
            '』' => '﹄',
            '【' => '︻',
            '】' => '︼',
            '…' => '︙',
            other => other,
        };
        let has = |font: &ab_glyph::FontRef<'_>, c: char| font.glyph_id(c).0 != 0;
        let ch = if wm.vertical && fonts.iter().skip(1).any(|f| has(f, vertical_form)) { vertical_form } else { original };
        // The first face with the glyph; Inter (its missing-glyph box) when none has it.
        let face = fonts.iter().position(|f| has(f, ch)).unwrap_or(0);
        let sf = fonts.get(face).unwrap_or(latin).as_scaled(PxScale::from(px));
        let id = sf.glyph_id(ch);
        if wm.vertical {
            let left = columns.saturating_sub(column.saturating_add(1)) as f32 * px;
            let pen = point(left + (px - sf.h_advance(id)) / 2.0, y + sf.ascent());
            let rotation = matches!(original, 'ー' | '—' | '–').then_some((left + px / 2.0, y + px / 2.0));
            glyphs.push((face, id.with_scale_and_position(px, pen), rotation));
            y += px;
        } else {
            if let Some((previous_face, previous)) = prev
                && previous_face == face
            {
                x += sf.kern(previous, id);
            }
            glyphs.push((face, id.with_scale_and_position(px, point(x, y + sf.ascent())), None));
            x += sf.h_advance(id);
            prev = Some((face, id));
        }
    }
    let (tw, th) = if wm.vertical { (columns as f32 * px, tw.max(y)) } else { (tw.max(x), y + px) };
    let inset = wm.inset.clamp(0.0, 0.4) * short;
    let (w, h) = (width as f32, height as f32);
    use Anchor::*;
    let ox = match wm.anchor {
        TopLeft | Left | BottomLeft => inset,
        Top | Center | Bottom => (w - tw) / 2.0,
        TopRight | Right | BottomRight => w - inset - tw,
    };
    let oy = match wm.anchor {
        TopLeft | Top | TopRight => inset,
        Left | Center | Right => (h - th) / 2.0,
        BottomLeft | Bottom | BottomRight => h - inset - th,
    };
    let alpha = wm.opacity.clamp(0.0, 1.0);
    let mut blend = |gx: i32, gy: i32, cov: f32, col: [u8; 3], a: f32| {
        if gx < 0 || gy < 0 || gx >= width as i32 || gy >= height as i32 {
            return;
        }
        blend_px(gx as usize, gy as usize, (cov * a).clamp(0.0, 1.0), col);
    };
    let passes: &[(f32, [u8; 3], f32)] =
        if wm.shadow { &[((px * 0.05).max(1.0), [0, 0, 0], 0.45), (0.0, wm.color, 1.0)] } else { &[(0.0, wm.color, 1.0)] };
    for &(off, col, a) in passes {
        for (face, g, rotation) in &glyphs {
            if let Some(o) = fonts.get(*face).and_then(|font| font.outline_glyph(g.clone())) {
                let b = o.px_bounds();
                o.draw(|gx, gy, cov| {
                    let (sx, sy) = (b.min.x + gx as f32, b.min.y + gy as f32);
                    let (sx, sy) = match rotation {
                        Some((cx, cy)) => (cx - (sy - cy), cy + (sx - cx)),
                        None => (sx, sy),
                    };
                    blend((ox + off + sx) as i32, (oy + off + sy) as i32, cov, col, a * alpha);
                });
            }
        }
    }
}

/// A watermark graphic, decoded once per file (and again when the file changes).
fn watermark_logo(path: &str) -> Option<std::sync::Arc<Rgba8>> {
    use std::collections::HashMap;
    use std::sync::{Arc, Mutex};
    static CACHE: Mutex<Option<HashMap<String, (std::time::SystemTime, Arc<Rgba8>)>>> = Mutex::new(None);
    let modified = std::fs::metadata(path).and_then(|m| m.modified()).ok()?;
    let mut c = CACHE.lock().unwrap_or_else(|e| e.into_inner());
    let map = c.get_or_insert_with(HashMap::new);
    if let Some((m, img)) = map.get(path)
        && *m == modified
    {
        return Some(img.clone());
    }
    let bytes = std::fs::read(path).ok()?;
    let img = Arc::new(lightcraft_codecs::decode(&bytes, Default::default()).ok()?.to_srgb8());
    map.insert(path.to_string(), (modified, img.clone()));
    Some(img)
}

/// The graphic watermark: scaled (premultiplied, so transparent edges stay clean) to
/// `image_width` of the photo, placed like the text, blended with its alpha × opacity.
fn logo_coverage(width: usize, height: usize, wm: &Watermark, mut blend_px: impl FnMut(usize, usize, f32, [u8; 3])) {
    let Some(logo) = watermark_logo(wm.image.trim()) else {
        log::warn!("watermark: can't read {}", wm.image);
        return;
    };
    if width == 0 || height == 0 || logo.width == 0 || logo.height == 0 {
        return;
    }
    let lw = ((wm.image_width.clamp(0.01, 1.0) * width as f32).round() as usize).clamp(1, width);
    let lh = ((lw as f32 * logo.height as f32 / logo.width as f32).round() as usize).clamp(1, height);
    let pre: lightcraft_raster::Image<[f32; 4]> = lightcraft_raster::Image::from_fn(logo.width, logo.height, |x, y| {
        let p = logo.get(x, y);
        let a = p[3] as f32 / 255.0;
        [p[0] as f32 * a, p[1] as f32 * a, p[2] as f32 * a, a]
    });
    let scaled = lightcraft_raster::resample::resize(&pre, lw, lh, lightcraft_raster::resample::Filter::Mitchell);
    let inset = wm.inset.clamp(0.0, 0.4) * width.min(height) as f32;
    let (w, h, lwf, lhf) = (width as f32, height as f32, lw as f32, lh as f32);
    use Anchor::*;
    let ox = match wm.anchor {
        TopLeft | Left | BottomLeft => inset,
        Top | Center | Bottom => (w - lwf) / 2.0,
        TopRight | Right | BottomRight => w - inset - lwf,
    }
    .max(0.0) as usize;
    let oy = match wm.anchor {
        TopLeft | Top | TopRight => inset,
        Left | Center | Right => (h - lhf) / 2.0,
        BottomLeft | Bottom | BottomRight => h - inset - lhf,
    }
    .max(0.0) as usize;
    let space = wm.target.unwrap_or(OutputSpace::Srgb);
    let op = wm.opacity.clamp(0.0, 1.0);
    for y in 0..lh {
        for x in 0..lw {
            let [r, g, b, a] = scaled.get(x, y);
            let a = a.clamp(0.0, 1.0);
            let (px, py) = (ox + x, oy + y);
            if a <= 1e-4 || px >= width || py >= height {
                continue;
            }
            let c = [r, g, b].map(|v| (v / a).round().clamp(0.0, 255.0) as u8);
            blend_px(px, py, a * op, srgb8_in(space, c));
        }
    }
}

/// Which metadata is embedded in exported files.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum MetadataPolicy {
    /// All Metadata: camera, capture settings, lens, location, title, caption, keywords, copyright.
    #[default]
    All,
    /// All Except Camera Raw Info. LightCraft never embeds its develop settings in a rendered
    /// file (a re-imported export would be edited twice), so this writes what `All` does.
    AllExceptCameraRaw,
    /// All Except Camera & Camera Raw Info: no camera / lens make, model or capture settings.
    AllExceptCamera,
    /// Copyright & Contact Info Only: the copyright fields and the creator.
    CopyrightContact,
    /// Copyright Only: notice, status, usage terms and info URL.
    Copyright,
    /// Nothing (the colour profile is still embedded). Not in Lightroom's menu.
    None,
}

impl MetadataPolicy {
    /// The Metadata section's Include menu: Lightroom's five in its order, then LightCraft's None.
    pub const ALL: [(MetadataPolicy, &'static str); 6] = [
        (MetadataPolicy::Copyright, "Copyright Only"),
        (MetadataPolicy::CopyrightContact, "Copyright & Contact Info Only"),
        (MetadataPolicy::AllExceptCameraRaw, "All Except Camera Raw Info"),
        (MetadataPolicy::AllExceptCamera, "All Except Camera & Camera Raw Info"),
        (MetadataPolicy::All, "All Metadata"),
        (MetadataPolicy::None, "None (LightCraft)"),
    ];

    pub fn label(self) -> &'static str {
        Self::ALL.iter().find(|(m, _)| *m == self).map_or("All Metadata", |(_, l)| l)
    }

    /// Whether the policy writes more than copyright / contact info (the person, location and
    /// keyword options apply).
    pub fn is_full(self) -> bool {
        matches!(self, Self::All | Self::AllExceptCameraRaw | Self::AllExceptCamera)
    }
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default, rename_all = "camelCase")]
pub struct ExportOptions {
    /// Export To (the dialog's top menu): Hard Drive, Email, CD/DVD or Apple Photos. The engine
    /// writes files either way; the desktop app hands them to Mail / Photos.
    pub service: ExportService,
    pub format: ExportFormat,
    /// 1–100 (JPEG, AVIF).
    pub quality: u8,
    /// Output size (`None` = full size).
    pub resize: Option<Resize>,
    /// Print resolution written into the file (pixels per inch).
    pub ppi: u16,
    /// The unit the dialog shows the resolution in (Lightroom's pixels per inch / per cm).
    pub resolution_unit: ResolutionUnit,
    /// JPEG only: largest quality whose file fits in this many KB.
    pub limit_kb: Option<u32>,
    pub sharpen: SharpenFor,
    pub sharpen_amount: SharpenAmount,
    /// File name template with the batch-rename tokens ([`crate::rename::expand_tokens`]): `{name}`
    /// (original stem), `{seq}` (zero-padded to 3, counting from `start_number`; `{seq:N}` for N
    /// digits), `{date}` (capture date, YYYYMMDD; `{date:%Y-%m-%d}`…), `{num}`, `{folder}`,
    /// `{camera}`, `{lens}`, `{iso}`, `{rating}`, `{title}`, `{creator}`, `{ext}`; the output
    /// extension is appended.
    pub naming: String,
    /// First `{seq}` value.
    pub start_number: u32,
    /// `{text}`: Lightroom's Custom Text.
    pub custom_text: String,
    /// The appended extension's case (Lightroom's Extensions popup).
    pub ext_case: crate::rename::ExtCase,
    /// Post-Processing: what happens once the files are written.
    pub after: AfterExport,
    /// [`AfterExport::OpenInApp`]: the application (name or path; empty = each file's default
    /// app) or an Export Action (an app or script in the Export Actions folder).
    pub after_app: String,
    /// Export Location ▸ Export To: a specific folder (`dir`), Choose folder later (asked when the
    /// export starts), Same folder as original photo (`dir` unused), or a standard folder (Home,
    /// Documents, Pictures, Desktop). `sameFolder: true` (older presets) reads as `sameFolder`.
    pub export_to: ExportTo,
    /// Add to This Catalog: the exported files are imported (in place) once written.
    pub add_to_catalog: bool,
    /// With `add_to_catalog`: stack each one with its original, above or below it.
    pub add_to_stack: Option<StackPlace>,
    /// Created inside the destination folder (empty = none).
    pub subfolder: String,
    /// When a file of the same name exists in the destination folder.
    pub conflict: Conflict,
    /// TIFF compression.
    pub tiff_compression: TiffCompression,
    /// DNG export: how the raw data is stored.
    pub dng_compression: DngCompression,
    /// DNG export: the embedded JPEG preview (Lightroom's JPEG Preview: None / Medium / Full Size).
    pub dng_preview: DngPreview,
    /// DNG export: Lightroom's Use Lossy Compression (demosaiced, 8-bit lossy JPEG tiles; the
    /// Image Sizing size applies). Both need Adobe's DNG SDK; without it the DNG is written as before.
    pub dng_lossy: bool,
    pub metadata: MetadataPolicy,
    /// Strip GPS / location even when the policy would include it.
    pub remove_location: bool,
    /// Lightroom's Remove Person Info: the names of the people tagged in the photo are not
    /// written as keywords.
    pub remove_people: bool,
    /// Write Keywords as Lightroom Hierarchy (`lr:hierarchicalSubject`, full `a|b|c` paths).
    pub keyword_hierarchy: bool,
    /// Text watermark (none when absent or the text is empty).
    pub watermark: Option<Watermark>,
    /// Output colour space (AVIF is always sRGB: its muxer cannot embed a profile).
    pub color_space: OutputSpace,
    /// E08: an ICC profile the file is converted to and tagged with (`colorMatch`, or the path of an
    /// `.icc` / `.icm` RGB profile): the photo is rendered in ProPhoto RGB, then converted.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub icc_profile: Option<String>,
    /// Bits per channel: 8, 16 (PNG, TIFF), 32 (TIFF: float, linear) or 10 (AVIF); `None` = the
    /// format's default (TIFF 16, everything else 8). See [`ExportOptions::effective_depth`].
    pub bit_depth: Option<u8>,
    /// HDR output for photos edited in HDR: JPEG as an ISO 21496-1 gain map JPEG (the SDR
    /// rendition plus a gain map, so the file looks right everywhere; quality as set, `limit_kb`
    /// not applied), AVIF as 10-bit Rec. 2020 PQ, 32-bit float TIFF with the highlights above SDR
    /// white kept. Other formats, and photos without an HDR edit, are SDR.
    pub hdr: bool,
    /// The curve of an HDR AVIF, PNG or JPEG XL: PQ (default) or HLG.
    pub hdr_transfer: HdrCurve,
    /// Lightroom's "Maximize Compatibility" for HDR output: an SDR base image plus a gain map
    /// (JPEG always; JPEG XL with this on), so the file looks right in every viewer and lights up
    /// on HDR displays. Off: JPEG XL is written PQ / HLG.
    pub hdr_compat: bool,
    /// LR-VID-EXPORT: how a selection's videos export (Include Video Files, format, quality).
    pub video: crate::video::VideoExport,
}

/// Transfer curve of an HDR AVIF / PNG / JPEG XL export (`hdrTransfer`): PQ (absolute, SDR white
/// at 203 cd/m²) or HLG (relative to the display; BT.2100 with SDR white at signal 0.75).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum HdrCurve {
    #[default]
    Pq,
    Hlg,
}

impl HdrCurve {
    pub fn parse(s: &str) -> Option<Self> {
        match s.to_ascii_lowercase().as_str() {
            "pq" => Some(Self::Pq),
            "hlg" => Some(Self::Hlg),
            _ => None,
        }
    }

    pub fn codec(self) -> lightcraft_codecs::HdrTransfer {
        match self {
            Self::Pq => lightcraft_codecs::HdrTransfer::Pq,
            Self::Hlg => lightcraft_codecs::HdrTransfer::Hlg,
        }
    }
}

/// Export Location ▸ Export To (Lightroom's list, in its order).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum ExportTo {
    /// The folder the export names (`dir`).
    #[default]
    SpecificFolder,
    /// Choose folder later (useful for presets): the folder is asked for when the export starts.
    ChooseLater,
    /// Same folder as original photo.
    SameFolder,
    Home,
    Documents,
    Pictures,
    Desktop,
}

impl ExportTo {
    /// Every choice with Lightroom's label, in its menu order.
    pub const ALL: [(ExportTo, &'static str); 7] = [
        (ExportTo::SpecificFolder, "Specific folder"),
        (ExportTo::ChooseLater, "Choose folder later (useful for presets)"),
        (ExportTo::SameFolder, "Same folder as original photo"),
        (ExportTo::Home, "Home folder"),
        (ExportTo::Documents, "Documents folder"),
        (ExportTo::Pictures, "Pictures folder"),
        (ExportTo::Desktop, "Desktop"),
    ];

    pub fn label(self) -> &'static str {
        Self::ALL.iter().find(|(t, _)| *t == self).map_or("Specific folder", |(_, l)| l)
    }

    /// The standard folder this choice names (Home, Documents, Pictures, Desktop) under `home`.
    pub fn standard_dir_in(self, home: &std::path::Path) -> Option<std::path::PathBuf> {
        match self {
            Self::Home => Some(home.to_path_buf()),
            Self::Documents => Some(home.join("Documents")),
            Self::Pictures => Some(home.join("Pictures")),
            Self::Desktop => Some(home.join("Desktop")),
            Self::SpecificFolder | Self::ChooseLater | Self::SameFolder => None,
        }
    }

    /// [`ExportTo::standard_dir_in`] the user's home folder (`HOME`, `USERPROFILE` on Windows).
    pub fn standard_dir(self) -> Option<String> {
        let home = std::env::var_os("HOME").or_else(|| std::env::var_os("USERPROFILE")).map(std::path::PathBuf::from)?;
        self.standard_dir_in(&home).map(|d| d.to_string_lossy().to_string())
    }

    /// The folder an export writes to: the standard folder this names, else `dir` (a specific or
    /// chosen folder; unused for the same folder as the original).
    pub fn resolve(self, dir: &str) -> String {
        self.standard_dir().unwrap_or_else(|| dir.trim().to_string())
    }
}

/// Where Add to Stack puts an exported photo: above or below its original.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum StackPlace {
    Above,
    Below,
}

/// Add to This Catalog (E03): import the written files in place, each stacked with its original
/// when asked (`add_to_stack`). Called on the session's thread once an export finished; one undo
/// step for the stacks. Returns `{added, stacked}`, or `None` when the export doesn't ask.
pub fn add_to_catalog(s: &mut crate::Session, o: &ExportOptions, files: &[serde_json::Value]) -> Option<serde_json::Value> {
    use lightcraft_catalog::{Op, PhotoId, Source, Stack};
    if !o.add_to_catalog {
        return None;
    }
    let written: Vec<(String, Option<u64>)> =
        files.iter().filter_map(|f| Some((f.get("path")?.as_str()?.to_string(), f.get("photo").and_then(serde_json::Value::as_u64)))).collect();
    if written.is_empty() {
        return Some(serde_json::json!({"added": 0}));
    }
    let paths: Vec<String> = written.iter().map(|w| w.0.clone()).collect();
    if let Err(e) = crate::import::import(s, &paths, crate::import::ImportMode::Add) {
        return Some(serde_json::json!({"added": 0, "error": e.to_string()}));
    }
    let by_path: std::collections::HashMap<String, PhotoId> = s
        .catalog
        .photos()
        .filter_map(|p| match &p.source {
            Source::File { path } => Some((path.clone(), p.id)),
            Source::Demo { .. } => None,
        })
        .collect();
    let mut added = 0;
    let mut stacked = 0;
    for (path, orig) in &written {
        let Some(new) = by_path.get(path).copied() else { continue };
        added += 1;
        let (Some(place), Some(orig)) = (o.add_to_stack, orig.map(PhotoId)) else { continue };
        if orig == new || s.catalog.photo(orig).is_none() || s.catalog.stack_of(new).is_some() {
            continue;
        }
        let op = match s.catalog.stack_of(orig).cloned() {
            Some(st) => {
                let mut photos = st.photos.clone();
                let at = st.position(orig).map_or(photos.len(), |i| if place == StackPlace::Above { i } else { i + 1 });
                photos.insert(at.min(photos.len()), new);
                Op::SetStack { id: st.id, photos, collapsed: st.collapsed }
            }
            None => {
                let id = s.catalog.alloc_stack_id();
                let photos = if place == StackPlace::Above { vec![new, orig] } else { vec![orig, new] };
                Op::AddStack { stack: Stack { id, photos, collapsed: false } }
            }
        };
        // one at a time: the next file may join the same stack
        match s.commit("Add to Stack", op) {
            Ok(()) => stacked += 1,
            Err(e) => log::warn!("export: add to stack: {e}"),
        }
    }
    s.merge_undo(stacked, "Add to Stack");
    Some(serde_json::json!({"added": added, "stacked": stacked}))
}

/// Lightroom's Export ▸ Post-Processing ▸ After Export.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum AfterExport {
    /// Do nothing.
    #[default]
    Nothing,
    /// Show the exported files in the Finder (Explorer, the file manager).
    ShowInFinder,
    /// Open them in an application (Lightroom's Open in Photoshop / Other Application…) or hand
    /// them to an Export Action (an app or script from the Export Actions folder).
    OpenInApp,
}

/// The Export Actions folder (`~/Library/Application Support/LightCraft/Export Actions` on macOS,
/// `$XDG_CONFIG_HOME/lightcraft/Export Actions` elsewhere): apps, aliases and scripts put there
/// are offered under After Export, like Lightroom's.
pub fn export_actions_dir() -> Option<std::path::PathBuf> {
    let home = std::env::var_os(if cfg!(windows) { "APPDATA" } else { "HOME" }).map(std::path::PathBuf::from)?;
    Some(if cfg!(target_os = "macos") {
        home.join("Library/Application Support/LightCraft/Export Actions")
    } else if cfg!(windows) {
        home.join("LightCraft").join("Export Actions")
    } else {
        std::env::var_os("XDG_CONFIG_HOME").map(std::path::PathBuf::from).unwrap_or_else(|| home.join(".config")).join("lightcraft/Export Actions")
    })
}

/// The Export Actions (name, path), sorted by name; hidden files skipped.
pub fn export_actions() -> Vec<(String, String)> {
    let Some(dir) = export_actions_dir() else { return vec![] };
    let Ok(rd) = std::fs::read_dir(&dir) else { return vec![] };
    let mut v: Vec<(String, String)> = rd
        .flatten()
        .filter(|e| !e.file_name().to_string_lossy().starts_with('.'))
        .map(|e| {
            let p = e.path();
            let name = p.file_stem().map(|n| n.to_string_lossy().to_string()).unwrap_or_default();
            (name, p.to_string_lossy().to_string())
        })
        .collect();
    v.sort();
    v
}

/// The program and arguments [`post_process`] runs for the written `paths` (`None`: nothing to
/// run). Pure, so it is tested without launching anything.
pub fn post_process_command(o: &ExportOptions, paths: &[String]) -> Option<(String, Vec<String>)> {
    let first = paths.first()?;
    match o.after {
        AfterExport::Nothing => None,
        AfterExport::ShowInFinder => Some(if cfg!(target_os = "macos") {
            ("/usr/bin/open".into(), std::iter::once("-R".to_string()).chain(paths.iter().cloned()).collect())
        } else if cfg!(windows) {
            ("explorer".into(), vec![format!("/select,{first}")])
        } else {
            let dir = std::path::Path::new(first).parent().map(|d| d.to_string_lossy().to_string()).unwrap_or_default();
            ("xdg-open".into(), vec![dir])
        }),
        AfterExport::OpenInApp => {
            let app = o.after_app.trim();
            let path = std::path::Path::new(app);
            let is_bundle = path.extension().is_some_and(|e| e.eq_ignore_ascii_case("app"));
            if !app.is_empty() && path.is_file() && !is_bundle {
                // an Export Action script / program: run it with the files
                return Some((app.to_string(), paths.to_vec()));
            }
            Some(if cfg!(target_os = "macos") {
                let mut args = Vec::new();
                if !app.is_empty() {
                    args.push("-a".to_string());
                    args.push(app.to_string());
                }
                args.extend(paths.iter().cloned());
                ("/usr/bin/open".into(), args)
            } else if app.is_empty() {
                (if cfg!(windows) { "explorer".into() } else { "xdg-open".into() }, vec![first.clone()])
            } else {
                (app.to_string(), paths.to_vec())
            })
        }
    }
}

/// Post-Processing after an export: run [`post_process_command`] for the files it wrote (per-photo
/// results with a `path`). Never fails the export; returns what was done to report, or `None`.
pub fn post_process(o: &ExportOptions, files: &[serde_json::Value]) -> Option<serde_json::Value> {
    if o.after == AfterExport::Nothing {
        return None;
    }
    let paths: Vec<String> = files.iter().filter_map(|f| f.get("path").and_then(serde_json::Value::as_str)).map(str::to_string).collect();
    let Some((program, args)) = post_process_command(o, &paths) else {
        return Some(serde_json::json!({"after": o.after, "skipped": "no files written"}));
    };
    #[cfg(not(target_arch = "wasm32"))]
    {
        match std::process::Command::new(&program).args(&args).spawn() {
            // reaped on a thread: no zombie, and the export never waits for the app
            Ok(mut child) => {
                let _ = std::thread::Builder::new().name("export-after".into()).spawn(move || child.wait());
                Some(serde_json::json!({"after": o.after, "program": program, "files": paths.len()}))
            }
            Err(e) => Some(serde_json::json!({"after": o.after, "program": program, "error": e.to_string()})),
        }
    }
    #[cfg(target_arch = "wasm32")]
    {
        let _ = args;
        Some(serde_json::json!({"after": o.after, "program": program, "error": "not available in the browser"}))
    }
}

impl Default for ExportOptions {
    fn default() -> Self {
        Self {
            service: ExportService::HardDrive,
            format: ExportFormat::Jpeg,
            quality: 90,
            resize: None,
            ppi: 240,
            resolution_unit: ResolutionUnit::Inch,
            limit_kb: None,
            sharpen: SharpenFor::None,
            sharpen_amount: SharpenAmount::Standard,
            naming: "{name}".into(),
            start_number: 1,
            custom_text: String::new(),
            ext_case: crate::rename::ExtCase::Keep,
            after: AfterExport::Nothing,
            after_app: String::new(),
            export_to: ExportTo::SpecificFolder,
            add_to_catalog: false,
            add_to_stack: None,
            subfolder: String::new(),
            conflict: Conflict::Unique,
            tiff_compression: TiffCompression::Deflate,
            dng_compression: DngCompression::Lossless,
            dng_preview: DngPreview::Medium,
            dng_lossy: false,
            metadata: MetadataPolicy::All,
            remove_location: false,
            remove_people: false,
            keyword_hierarchy: false,
            watermark: None,
            color_space: OutputSpace::Srgb,
            icc_profile: None,
            bit_depth: None,
            hdr: false,
            hdr_transfer: HdrCurve::Pq,
            hdr_compat: true,
            video: crate::video::VideoExport::default(),
        }
    }
}

/// The `app.export` params that are not export options: which photos, where to, a preset to start
/// from, and the desktop app's background flag.
pub const TARGET_PARAMS: &[&str] = &["id", "ids", "path", "dir", "preset", "background", "resolve"];

/// The export option params [`ExportOptions::from_json`] reads (and [`ExportOptions::to_json`] writes).
pub const OPTION_PARAMS: &[&str] = &[
    "service",
    "format",
    "iccProfile",
    "quality",
    "resize",
    "longEdge",
    "shortEdge",
    "width",
    "height",
    "megapixels",
    "percent",
    "dontEnlarge",
    "ppi",
    "resolutionUnit",
    "limitKb",
    "sharpen",
    "sharpenAmount",
    "naming",
    "startNumber",
    "customText",
    "extCase",
    "after",
    "afterApp",
    "sameFolder",
    "exportTo",
    "addToCatalog",
    "addToStack",
    "subfolder",
    "conflict",
    "tiffCompression",
    "dngCompression",
    "dngPreview",
    "dngLossy",
    "metadata",
    "removeLocation",
    "removePeople",
    "keywordHierarchy",
    "watermark",
    "colorSpace",
    "bitDepth",
    "hdr",
    "hdrTransfer",
    "hdrCompat",
    "video",
];

/// Lightroom's Simple Copyright Watermark: each photo's Copyright field as a text watermark (none
/// on a photo whose field is empty).
pub fn simple_copyright_watermark() -> Watermark {
    Watermark { text: SIMPLE_COPYRIGHT.into(), ..Default::default() }
}

/// The text of [`simple_copyright_watermark`].
pub const SIMPLE_COPYRIGHT: &str = "{copyright}";

/// The keys of a `watermark` object ([`Watermark`], camelCase).
pub const WATERMARK_PARAMS: &[&str] = &["text", "vertical", "size", "opacity", "anchor", "inset", "color", "shadow", "image", "imageWidth"];

/// Text height as a fraction of the short edge: the range the renderer draws without clamping.
pub const WATERMARK_SIZE_RANGE: (f32, f32) = (0.005, 0.5);
/// Margin as a fraction of the short edge.
pub const WATERMARK_INSET_RANGE: (f32, f32) = (0.0, 0.4);
/// A graphic's width as a fraction of the photo's width.
pub const WATERMARK_IMAGE_WIDTH_RANGE: (f32, f32) = (0.01, 1.0);

/// Levenshtein distance, for "did you mean" on a misspelled key (both inputs are short).
fn edit_distance(a: &str, b: &str) -> usize {
    let (a, b): (Vec<char>, Vec<char>) = (a.chars().collect(), b.chars().collect());
    let mut prev: Vec<usize> = (0..=b.len()).collect();
    for (i, ca) in a.iter().enumerate() {
        let mut cur = vec![i + 1; b.len() + 1];
        for (j, cb) in b.iter().enumerate() {
            let sub = prev.get(j).copied().unwrap_or(0) + usize::from(ca != cb);
            let del = prev.get(j + 1).copied().unwrap_or(0) + 1;
            let ins = cur.get(j).copied().unwrap_or(0) + 1;
            if let Some(c) = cur.get_mut(j + 1) {
                *c = sub.min(del).min(ins);
            }
        }
        prev = cur;
    }
    prev.last().copied().unwrap_or(0)
}

/// `unknown parameter \`k\``, naming the closest known key when one is near.
fn unknown_key(what: &str, k: &str, known: &[&str]) -> String {
    let near = known.iter().map(|n| (edit_distance(&k.to_ascii_lowercase(), &n.to_ascii_lowercase()), *n)).filter(|(d, _)| *d <= 2).min();
    match near {
        Some((_, n)) => format!("unknown {what} `{k}` (did you mean `{n}`?)"),
        None => format!("unknown {what} `{k}` (one of {})", known.join(", ")),
    }
}

/// A JSON integer (`92`; never `92.5` or `"92"`), as [`ExportOptions::from_json`] reads them.
fn whole(v: &serde_json::Value) -> Option<i64> {
    v.as_i64()
}

impl ExportOptions {
    /// Stored export settings (a saved preset, `prefs.json ▸ lastExport`) with the keys an older or
    /// newer version may have written and this one doesn't know dropped, also inside `watermark`.
    /// [`ExportOptions::validate`] is strict about what a caller types; settings saved on disk are
    /// not the caller's typo, and refusing them would make a preset or Export with Previous fail
    /// for good after an update.
    pub fn known_keys_only(p: &serde_json::Value) -> serde_json::Value {
        let mut p = p.clone();
        if let Some(o) = p.as_object_mut() {
            o.retain(|k, _| OPTION_PARAMS.contains(&k.as_str()) || TARGET_PARAMS.contains(&k.as_str()));
            if let Some(w) = o.get_mut("watermark").and_then(serde_json::Value::as_object_mut) {
                w.retain(|k, _| WATERMARK_PARAMS.contains(&k.as_str()));
            }
        }
        p
    }

    /// Strict [`ExportOptions::from_json`] for a command that is about to export (issue #181): an
    /// unknown parameter or a value that cannot be read as what it is for is an error, never a
    /// silent default. `app.export` targets (`id`, `ids`, `path`, `dir`, `preset`, `background`)
    /// are accepted and left to the caller.
    pub fn from_params(p: &serde_json::Value) -> crate::Result<Self> {
        Self::validate("app.export", p)?;
        Ok(Self::from_json(p))
    }

    /// Check `app.export` params for `cmd` (the command named in the error): every key must be an
    /// export option ([`OPTION_PARAMS`]) or a target ([`TARGET_PARAMS`]), every value must be the
    /// kind of thing its key takes, and a `watermark` object must hold only [`WATERMARK_PARAMS`]
    /// with its sizes in range (issue #183). `null` means "not given", as a dropped key does.
    pub fn validate(cmd: &str, p: &serde_json::Value) -> crate::Result<()> {
        use serde_json::Value;
        let bad = |msg: String| crate::cmd::bad(cmd, msg);
        let Some(obj) = p.as_object() else { return Err(bad("params must be an object".into())) };
        let known: Vec<&str> = OPTION_PARAMS.iter().chain(TARGET_PARAMS).copied().collect();
        let int = |k: &str, v: &Value, lo: i64, hi: i64| -> crate::Result<i64> {
            whole(v).filter(|i| (lo..=hi).contains(i)).ok_or_else(|| bad(format!("`{k}` must be an integer {lo}..{hi}")))
        };
        let num = |k: &str, v: &Value, lo: f64, hi: f64| -> crate::Result<f64> {
            v.as_f64().filter(|f| f.is_finite() && (lo..=hi).contains(f)).ok_or_else(|| bad(format!("`{k}` must be a number {lo}..{hi}")))
        };
        fn string<'a>(cmd: &str, k: &str, v: &'a Value) -> crate::Result<&'a str> {
            v.as_str().ok_or_else(|| crate::cmd::bad(cmd, format!("`{k}` must be a string")))
        }
        let boolean = |k: &str, v: &Value| -> crate::Result<bool> { v.as_bool().ok_or_else(|| bad(format!("`{k}` must be true or false"))) };
        let one_of = |k: &str, v: &Value, options: &[&str]| -> crate::Result<()> {
            let s = string(cmd, k, v)?;
            if options.iter().any(|o| o.eq_ignore_ascii_case(s)) { Ok(()) } else { Err(bad(format!("`{k}` must be one of {}", options.join("|")))) }
        };
        for (k, v) in obj {
            if v.is_null() {
                continue;
            }
            match k.as_str() {
                "format" => {
                    if ExportFormat::parse(string(cmd, k, v)?).is_none() {
                        return Err(bad(format!("`{k}` must be one of jpeg|png|tiff|webp|avif|dng|original")));
                    }
                }
                "quality" => {
                    int(k, v, 1, 100)?;
                }
                "resolutionUnit" => one_of(k, v, &["inch", "cm"])?,
                "service" => one_of(k, v, &["hardDrive", "email", "disc", "photos"])?,
                "ppi" => {
                    int(k, v, 1, u16::MAX as i64)?;
                }
                "limitKb" => {
                    int(k, v, 0, u32::MAX as i64)?;
                }
                "startNumber" => {
                    int(k, v, 0, 999_999)?;
                }
                "video" => {
                    let ok = v.as_object().is_some_and(|o| o.keys().all(|x| matches!(x.as_str(), "include" | "format" | "quality")))
                        && serde_json::from_value::<crate::video::VideoExport>(v.clone()).is_ok();
                    if !ok {
                        return Err(bad(format!("`{k}` must be an object {{include: bool, format: h264|original, quality: max|high|medium|low}}")));
                    }
                }
                "bitDepth" => {
                    if !matches!(whole(v), Some(8 | 10 | 16 | 32)) {
                        return Err(bad(format!("`{k}` must be 8, 10, 16 or 32")));
                    }
                }
                "longEdge" | "shortEdge" | "width" | "height" | "megapixels" | "percent" => {
                    num(k, v, 0.0, 1.0e9)?;
                }
                "resize" => {
                    let Some(r) = v.as_object() else {
                        return Err(bad(format!("`{k}` must be an object {{mode, value, height?, dontEnlarge?, unit?}}")));
                    };
                    if let Some(x) = r.keys().find(|x| !matches!(x.as_str(), "mode" | "value" | "height" | "dontEnlarge" | "unit")) {
                        return Err(bad(unknown_key("resize key", x, &["mode", "value", "height", "dontEnlarge", "unit"])));
                    }
                    serde_json::from_value::<Resize>(v.clone()).map_err(|e| bad(format!("`{k}`: {e}")))?;
                }
                "dontEnlarge" | "dngLossy" | "removeLocation" | "removePeople" | "keywordHierarchy" | "background" | "hdr" | "hdrCompat" => {
                    boolean(k, v)?;
                }
                "extCase" => one_of(k, v, &["keep", "lower", "upper"])?,
                "after" => one_of(k, v, &["nothing", "showInFinder", "openInApp"])?,
                "exportTo" => one_of(k, v, &["specificFolder", "chooseLater", "sameFolder", "home", "documents", "pictures", "desktop"])?,
                "sameFolder" | "addToCatalog" => {
                    boolean(k, v)?;
                }
                "addToStack" if v.is_null() => {}
                "addToStack" => one_of(k, v, &["above", "below"])?,
                "afterApp" => {
                    string(cmd, k, v)?;
                }
                "naming" | "subfolder" | "path" | "dir" | "preset" | "customText" | "iccProfile" => {
                    string(cmd, k, v)?;
                }
                "sharpen" => one_of(k, v, &["none", "screen", "matte", "glossy"])?,
                "hdrTransfer" => one_of(k, v, &["pq", "hlg"])?,
                "sharpenAmount" => one_of(k, v, &["low", "standard", "high"])?,
                "conflict" => one_of(k, v, &["unique", "overwrite", "skip", "ask"])?,
                "resolve" => {
                    let ok = v.as_object().is_some_and(|o| {
                        o.iter().all(|(id, c)| id.parse::<u64>().is_ok() && matches!(c.as_str(), Some("unique" | "overwrite" | "skip")))
                    });
                    if !ok {
                        return Err(bad(format!("`{k}` must be an object {{\"<photo id>\": unique|overwrite|skip}}")));
                    }
                }
                "tiffCompression" => one_of(k, v, &["none", "lzw", "zip", "deflate"])?,
                "dngCompression" => {
                    serde_json::from_value::<DngCompression>(v.clone())
                        .map_err(|_| bad(format!("`{k}` must be one of lossless|deflate|uncompressed")))?;
                }
                "metadata" => one_of(k, v, &["all", "allExceptCameraRaw", "allExceptCamera", "copyrightContact", "copyright", "none"])?,
                "dngPreview" => one_of(k, v, &["none", "medium", "full"])?,
                "colorSpace" => {
                    if OutputSpace::parse(string(cmd, k, v)?).is_none() {
                        return Err(bad(format!("`{k}` must be one of srgb|displayP3|adobeRgb|proPhoto|rec2020")));
                    }
                }
                "watermark" => Self::validate_watermark(cmd, v)?,
                "id" => {
                    int(k, v, 0, i64::MAX)?;
                }
                "ids" => {
                    let ok = v.as_array().is_some_and(|a| a.iter().all(|x| x.is_u64()));
                    if !ok {
                        return Err(bad(format!("`{k}` must be an array of photo ids")));
                    }
                }
                _ => return Err(bad(unknown_key("parameter", k, &known))),
            }
        }
        Ok(())
    }

    /// A `watermark` value: text, or an object of [`WATERMARK_PARAMS`] (issue #183).
    fn validate_watermark(cmd: &str, v: &serde_json::Value) -> crate::Result<()> {
        use serde_json::Value;
        let bad = |msg: String| crate::cmd::bad(cmd, msg);
        let obj = match v {
            Value::String(_) => return Ok(()),
            Value::Object(o) => o,
            _ => {
                return Err(bad(
                    "`watermark` must be text or an object {text, vertical, size, opacity, anchor, inset, color, shadow, image, imageWidth}".into(),
                ));
            }
        };
        let frac = |k: &str, v: &Value, (lo, hi): (f32, f32), unit: &str| -> crate::Result<()> {
            match v.as_f64() {
                Some(f) if f.is_finite() && (f64::from(lo)..=f64::from(hi)).contains(&f) => Ok(()),
                _ => Err(bad(format!("`watermark.{k}` must be a number {lo}..{hi} ({unit})"))),
            }
        };
        for (k, v) in obj {
            if v.is_null() {
                continue;
            }
            match k.as_str() {
                "text" | "image" => {
                    v.as_str().ok_or_else(|| bad(format!("`watermark.{k}` must be a string")))?;
                }
                "vertical" | "shadow" => {
                    v.as_bool().ok_or_else(|| bad(format!("`watermark.{k}` must be true or false")))?;
                }
                "size" => frac(k, v, WATERMARK_SIZE_RANGE, "fraction of the short edge")?,
                "opacity" => frac(k, v, (0.0, 1.0), "0 = invisible, 1 = solid")?,
                "inset" => frac(k, v, WATERMARK_INSET_RANGE, "fraction of the short edge")?,
                "imageWidth" => frac(k, v, WATERMARK_IMAGE_WIDTH_RANGE, "fraction of the photo's width")?,
                "anchor" => {
                    serde_json::from_value::<Anchor>(v.clone()).map_err(|_| {
                        bad("`watermark.anchor` must be one of topLeft|top|topRight|left|center|right|bottomLeft|bottom|bottomRight".into())
                    })?;
                }
                "color" => {
                    let ok = v.as_array().is_some_and(|a| a.len() == 3 && a.iter().all(|c| matches!(whole(c), Some(0..=255))));
                    if !ok {
                        return Err(bad("`watermark.color` must be [r, g, b] with 0..255 each (sRGB)".into()));
                    }
                }
                _ => return Err(bad(unknown_key("watermark key", k, WATERMARK_PARAMS))),
            }
        }
        serde_json::from_value::<Watermark>(v.clone()).map(|_| ()).map_err(|e| bad(format!("`watermark`: {e}")))
    }

    /// Read options from command params (`format`, `quality`, `limitKb`, `sharpen`, `sharpenAmount`,
    /// `naming`, `metadata`, `removeLocation`, `watermark`, `colorSpace`, `bitDepth`, `ppi`) and the
    /// size: one of `longEdge`, `shortEdge`, `width`, `height` (both = fit inside W × H),
    /// `megapixels`, `percent` (absent or 0 = full size), or a [`Resize`] object as `resize`; plus
    /// `dontEnlarge` (default true).
    ///
    /// Lenient: anything unknown or unreadable falls back to the default, which suits stored
    /// prefs and dialog prefill. A command that is about to export uses
    /// [`ExportOptions::from_params`], which validates first.
    pub fn from_json(p: &serde_json::Value) -> Self {
        use serde_json::Value;
        let d = Self::default();
        let s = |k: &str| p.get(k).and_then(Value::as_str);
        let u = |k: &str| p.get(k).and_then(Value::as_u64);
        fn enm<T: serde::de::DeserializeOwned>(p: &Value, k: &str) -> Option<T> {
            p.get(k).cloned().and_then(|v| serde_json::from_value(v).ok())
        }
        Self {
            service: enm(p, "service").unwrap_or(d.service),
            format: s("format").and_then(ExportFormat::parse).unwrap_or(d.format),
            quality: u("quality").map_or(d.quality, |q| q.clamp(1, 100) as u8),
            resize: Self::resize_from_json(p),
            ppi: u("ppi").filter(|v| *v > 0).map_or(d.ppi, |v| v.min(u16::MAX as u64) as u16),
            resolution_unit: enm(p, "resolutionUnit").unwrap_or(d.resolution_unit),
            limit_kb: u("limitKb").filter(|v| *v > 0).map(|v| v.min(u32::MAX as u64) as u32),
            sharpen: enm(p, "sharpen").unwrap_or(d.sharpen),
            sharpen_amount: enm(p, "sharpenAmount").unwrap_or(d.sharpen_amount),
            naming: s("naming").map_or(d.naming, str::to_string),
            start_number: u("startNumber").map_or(d.start_number, |v| v.min(999_999) as u32),
            custom_text: s("customText").map_or(d.custom_text, crate::rename::Batch::clean),
            ext_case: s("extCase").and_then(crate::rename::ExtCase::parse).unwrap_or(d.ext_case),
            after: enm(p, "after").unwrap_or(d.after),
            after_app: s("afterApp").map_or(d.after_app, |v| v.trim().to_string()),
            export_to: match (enm::<ExportTo>(p, "exportTo"), p.get("sameFolder").and_then(Value::as_bool)) {
                (Some(t), _) => t,
                (None, Some(true)) => ExportTo::SameFolder,
                (None, _) => d.export_to,
            },
            add_to_catalog: p.get("addToCatalog").and_then(Value::as_bool).unwrap_or(d.add_to_catalog),
            add_to_stack: enm(p, "addToStack").or(d.add_to_stack),
            subfolder: s("subfolder").map_or(d.subfolder, |v| v.trim().replace(['\\', ':', '\0'], "_")),
            conflict: enm(p, "conflict").unwrap_or(d.conflict),
            tiff_compression: match s("tiffCompression").map(str::to_ascii_lowercase).as_deref() {
                Some("none") => TiffCompression::None,
                Some("lzw") => TiffCompression::Lzw,
                Some("zip" | "deflate") => TiffCompression::Deflate,
                _ => d.tiff_compression,
            },
            dng_compression: enm(p, "dngCompression").unwrap_or(d.dng_compression),
            dng_preview: enm(p, "dngPreview").unwrap_or(d.dng_preview),
            dng_lossy: p.get("dngLossy").and_then(Value::as_bool).unwrap_or(d.dng_lossy),
            metadata: enm(p, "metadata").unwrap_or(d.metadata),
            remove_location: p.get("removeLocation").and_then(Value::as_bool).unwrap_or(d.remove_location),
            remove_people: p.get("removePeople").and_then(Value::as_bool).unwrap_or(d.remove_people),
            keyword_hierarchy: p.get("keywordHierarchy").and_then(Value::as_bool).unwrap_or(d.keyword_hierarchy),
            watermark: match p.get("watermark") {
                Some(Value::String(t)) => Some(Watermark { text: t.clone(), ..Default::default() }),
                Some(v @ Value::Object(_)) => serde_json::from_value(v.clone()).ok(),
                _ => None,
            }
            .filter(|w: &Watermark| !w.text.trim().is_empty() || !w.image.trim().is_empty()),
            color_space: s("colorSpace").and_then(OutputSpace::parse).unwrap_or(d.color_space),
            icc_profile: match p.get("iccProfile") {
                Some(serde_json::Value::String(x)) if !x.trim().is_empty() => Some(x.trim().to_string()),
                Some(_) => None,
                None => d.icc_profile.clone(),
            },
            bit_depth: u("bitDepth").filter(|b| matches!(b, 8 | 10 | 16 | 32)).map(|b| b as u8),
            hdr: p.get("hdr").and_then(Value::as_bool).unwrap_or(d.hdr),
            hdr_transfer: s("hdrTransfer").and_then(HdrCurve::parse).unwrap_or(d.hdr_transfer),
            hdr_compat: p.get("hdrCompat").and_then(Value::as_bool).unwrap_or(d.hdr_compat),
            video: p.get("video").cloned().and_then(|v| serde_json::from_value(v).ok()).unwrap_or(d.video),
        }
    }

    fn resize_from_json(p: &serde_json::Value) -> Option<Resize> {
        use serde_json::Value;
        let f = |k: &str| p.get(k).and_then(Value::as_f64).filter(|v| *v > 0.0);
        let dont_enlarge = p.get("dontEnlarge").and_then(Value::as_bool).unwrap_or(true);
        if let Some(r) = p.get("resize").filter(|v| v.is_object()) {
            let r: Resize = serde_json::from_value(r.clone()).ok()?;
            return (r.value > 0.0).then_some(Resize { dont_enlarge: p.get("dontEnlarge").and_then(Value::as_bool).unwrap_or(r.dont_enlarge), ..r });
        }
        let r = |mode, value: f64| Resize { mode, value: value.min(65_535.0) as f32, height: 0.0, dont_enlarge, unit: SizeUnit::Pixels };
        Some(match (f("width"), f("height")) {
            (Some(w), Some(h)) => Resize { height: h.min(65_535.0) as f32, ..r(ResizeMode::Dimensions, w) },
            (Some(w), None) => r(ResizeMode::Width, w),
            (None, Some(h)) => r(ResizeMode::Height, h),
            (None, None) => {
                if let Some(v) = f("longEdge") {
                    r(ResizeMode::LongEdge, v)
                } else if let Some(v) = f("shortEdge") {
                    r(ResizeMode::ShortEdge, v)
                } else if let Some(v) = f("megapixels") {
                    r(ResizeMode::Megapixels, v)
                } else {
                    let v = f("percent")?;
                    r(ResizeMode::Percent, v)
                }
            }
        })
    }

    /// These options as `app.export` params ([`ExportOptions::from_json`] reads them back
    /// unchanged; full size is written as `longEdge: 0`).
    pub fn to_json(&self) -> serde_json::Value {
        let mut v = serde_json::to_value(self).unwrap_or_default();
        if let Some(o) = v.as_object_mut() {
            o.remove("resize");
            if let Some(sz) = Self::resize_json(self.resize.as_ref()).as_object() {
                o.extend(sz.clone());
            }
            if self.resize.is_none() {
                o.insert("longEdge".into(), 0.into());
            }
            o.retain(|_, v| !v.is_null());
        }
        v
    }

    /// Whether `p` names an output size (any of the size params, even 0 = full size).
    pub fn has_size_param(p: &serde_json::Value) -> bool {
        ["resize", "longEdge", "shortEdge", "width", "height", "megapixels", "percent"].iter().any(|k| p.get(k).is_some())
    }

    /// The size params of [`ExportOptions::from_json`] for `resize` (`{}` = full size).
    pub fn resize_json(resize: Option<&Resize>) -> serde_json::Value {
        match resize {
            None => serde_json::json!({}),
            Some(r) => serde_json::json!({"resize": r, "dontEnlarge": r.dont_enlarge}),
        }
    }

    /// The sample format the file is written with (unsupported requests fall back to the closest).
    pub fn effective_depth(&self) -> OutputDepth {
        match (self.format, self.bit_depth) {
            (ExportFormat::Jpeg | ExportFormat::Webp | ExportFormat::Original | ExportFormat::Dng, _) => OutputDepth::U8,
            // an AVIF gain map: the SDR base (the HDR rendition is rendered beside it)
            (ExportFormat::Avif, _) if self.avif_gain_map() => OutputDepth::U8,
            (ExportFormat::Png | ExportFormat::Avif, _) if self.hdr => OutputDepth::F32Hdr,
            (ExportFormat::Jxl, _) if self.hdr && !self.hdr_compat => OutputDepth::F32Hdr,
            (ExportFormat::Png, Some(16 | 32)) => OutputDepth::U16,
            (ExportFormat::Png, _) => OutputDepth::U8,
            (ExportFormat::Tiff, Some(8)) => OutputDepth::U8,
            (ExportFormat::Tiff, Some(32)) if self.hdr => OutputDepth::F32Hdr,
            (ExportFormat::Tiff, Some(32)) => OutputDepth::F32Linear,
            (ExportFormat::Tiff, _) => OutputDepth::U16,
            (ExportFormat::Avif, Some(10 | 16 | 32)) => OutputDepth::U16,
            (ExportFormat::Avif, _) => OutputDepth::U8,
            (ExportFormat::Jxl | ExportFormat::Psd, Some(16 | 32)) => OutputDepth::U16,
            (ExportFormat::Jxl | ExportFormat::Psd, _) => OutputDepth::U8,
        }
    }

    /// The bit depths `format` offers (value, label); the first is its default.
    pub fn bit_depths(format: ExportFormat) -> &'static [(u8, &'static str)] {
        match format {
            ExportFormat::Jpeg | ExportFormat::Webp | ExportFormat::Original | ExportFormat::Dng => &[(8, "8-bit")],
            ExportFormat::Png => &[(8, "8-bit"), (16, "16-bit")],
            ExportFormat::Tiff => &[(16, "16-bit"), (8, "8-bit"), (32, "32-bit float")],
            ExportFormat::Avif => &[(8, "8-bit"), (10, "10-bit")],
            ExportFormat::Jxl => &[(8, "8-bit"), (16, "16-bit")],
            ExportFormat::Psd => &[(16, "16-bit"), (8, "8-bit")],
        }
    }

    /// Whether these options write HDR files (for photos edited in HDR): JPEG (gain map), AVIF
    /// (PQ) or 32-bit float TIFF with [`ExportOptions::hdr`].
    pub fn hdr_output(&self) -> bool {
        self.hdr
            && matches!(
                (self.format, self.bit_depth),
                (ExportFormat::Jpeg | ExportFormat::Avif | ExportFormat::Png | ExportFormat::Jxl, _) | (ExportFormat::Tiff, Some(32))
            )
    }

    /// Whether these options write an SDR base plus a gain map (HDR JPEG, and HDR JPEG XL or
    /// AVIF with Maximize Compatibility).
    pub fn gain_map_output(&self) -> bool {
        self.hdr_output() && (self.format == ExportFormat::Jpeg || (self.format == ExportFormat::Jxl && self.hdr_compat) || self.avif_gain_map())
    }

    /// An HDR AVIF with Maximize Compatibility: SDR base + ISO 21496-1 gain map, written by macOS's
    /// ImageIO ([`crate::imageio`]); without it (another platform) HDR AVIFs are PQ / HLG.
    pub fn avif_gain_map(&self) -> bool {
        self.format == ExportFormat::Avif && self.hdr && self.hdr_compat && crate::imageio::available()
    }

    /// The colour space the file is actually written in (AVIF: sRGB, or Rec. 2020 for HDR).
    pub fn effective_space(&self) -> OutputSpace {
        match (self.format, self.color_space) {
            (ExportFormat::Avif, _) if self.avif_gain_map() => OutputSpace::Srgb,
            (ExportFormat::Avif | ExportFormat::Png, _) if self.hdr => OutputSpace::Rec2020,
            (ExportFormat::Jxl, _) if self.hdr && !self.hdr_compat => OutputSpace::Rec2020,
            (ExportFormat::Avif, _) => OutputSpace::Srgb,
            // E08: rendered wide, then converted to the chosen profile
            _ if self.icc_profile.is_some() && !matches!(self.format, ExportFormat::Jxl | ExportFormat::Webp) => OutputSpace::ProPhoto,
            // a lossy JPEG XL declares its colours by enum: no Adobe RGB / ProPhoto one
            (ExportFormat::Jxl, OutputSpace::AdobeRgb | OutputSpace::ProPhoto) if self.quality < 100 => OutputSpace::Srgb,
            (_, s) => s,
        }
    }

    /// Output file name for photo `p` at 1-based position `seq` in a batch (the original's
    /// extension is kept for [`ExportFormat::Original`]).
    pub fn file_name_for(&self, p: &lightcraft_catalog::Photo, seq: usize) -> String {
        self.file_name_in(p, seq, 0)
    }

    /// [`ExportOptions::file_name_for`] in a batch of `total` photos (`{total}`).
    pub fn file_name_in(&self, p: &lightcraft_catalog::Photo, seq: usize, total: usize) -> String {
        let base = if self.naming.trim().is_empty() { "{name}" } else { self.naming.as_str() };
        let n = seq + self.start_number.max(1) as usize - 1;
        let batch = crate::rename::Batch { total, text: self.custom_text.clone(), ext_case: self.ext_case, ..Default::default() };
        let name = crate::rename::expand_tokens_in(base, p, n, 3, &batch);
        let mut name: String = name.chars().map(|c| if matches!(c, '/' | '\\' | ':' | '\0') { '_' } else { c }).collect();
        let (stem, ext) = p.file_name.rsplit_once('.').unwrap_or((&p.file_name, ""));
        if name.trim().is_empty() {
            // every token came out empty (no title, no capture time…): keep the original name
            name = stem.to_string();
        }
        if self.format == ExportFormat::Original {
            let base = name.trim_end_matches('.');
            if ext.is_empty() { base.to_string() } else { format!("{base}.{}", self.ext_case.apply(ext)) }
        } else {
            format!("{name}.{}", self.ext_case.apply(self.format.extension()))
        }
    }
}

/// Unsharp mask on an 8-bit image (separable box-blur approximation of a small Gaussian).
pub fn output_sharpen(img: &mut Rgba8, target: SharpenFor, amount: SharpenAmount) {
    let src: Vec<[f32; 3]> = img.data.iter().map(|p| [p[0] as f32, p[1] as f32, p[2] as f32]).collect();
    if let Some(out) = unsharp(img.width, img.height, &src, target, amount) {
        for (p, v) in img.data.iter_mut().zip(out) {
            for c in 0..3 {
                p[c] = v[c].round().clamp(0.0, 255.0) as u8;
            }
        }
    }
}

/// [`output_sharpen`] on a high-bit-depth image (float images are sharpened in linear light).
pub fn output_sharpen_deep(img: &mut DeepImage, target: SharpenFor, amount: SharpenAmount) {
    let (w, h) = (img.width, img.height);
    match &mut img.samples {
        DeepSamples::U16(v) => {
            let src: Vec<[f32; 3]> = v.as_chunks::<3>().0.iter().map(|c| [c[0] as f32, c[1] as f32, c[2] as f32]).collect();
            if let Some(out) = unsharp(w, h, &src, target, amount) {
                for (d, s) in v.iter_mut().zip(out.as_flattened()) {
                    *d = s.round().clamp(0.0, 65535.0) as u16;
                }
            }
        }
        DeepSamples::F32(v) => {
            let src: Vec<[f32; 3]> = v.as_chunks::<3>().0.iter().map(|c| [c[0], c[1], c[2]]).collect();
            if let Some(out) = unsharp(w, h, &src, target, amount) {
                for (d, s) in v.iter_mut().zip(out.as_flattened()) {
                    *d = s.max(0.0);
                }
            }
        }
    }
}

/// The sharpened values of `src` (`w × h`), or `None` when nothing is to be done.
fn unsharp(w: usize, h: usize, src: &[[f32; 3]], target: SharpenFor, amount: SharpenAmount) -> Option<Vec<[f32; 3]>> {
    let (radius, base) = match target {
        SharpenFor::None => return None,
        SharpenFor::Screen => (1usize, 0.35f32),
        SharpenFor::Matte => (2, 0.6),
        SharpenFor::Glossy => (1, 0.5),
    };
    let k = base
        * match amount {
            SharpenAmount::Low => 0.6,
            SharpenAmount::Standard => 1.0,
            SharpenAmount::High => 1.5,
        };
    if w < 3 || h < 3 {
        return None;
    }
    let r = radius as isize;
    let box_pass = |inp: &[[f32; 3]], horizontal: bool| -> Vec<[f32; 3]> {
        let mut out = vec![[0.0; 3]; inp.len()];
        for y in 0..h {
            for x in 0..w {
                let mut acc = [0.0f32; 3];
                for d in -r..=r {
                    let (sx, sy) = if horizontal {
                        ((x as isize + d).clamp(0, w as isize - 1) as usize, y)
                    } else {
                        (x, (y as isize + d).clamp(0, h as isize - 1) as usize)
                    };
                    let p = inp[sy * w + sx];
                    acc = [acc[0] + p[0], acc[1] + p[1], acc[2] + p[2]];
                }
                let n = (2 * r + 1) as f32;
                out[y * w + x] = acc.map(|v| v / n);
            }
        }
        out
    };
    let blur = box_pass(&box_pass(src, true), false);
    Some(src.iter().zip(&blur).map(|(s, b)| std::array::from_fn(|c| s[c] + (s[c] - b[c]) * k)).collect())
}

/// Encode a rendered display-referred image according to `o` (its pixels must already be in
/// `o.effective_space()`). Resizing to `long_edge` is the
/// caller's job (render at that size); sharpening is applied here.
pub fn encode_image(img: &Rgba8, o: &ExportOptions) -> Result<Vec<u8>, String> {
    encode_with_metadata(img, o, None)
}

/// Like [`encode_image`], embedding `meta` (already filtered by the policy) as EXIF + XMP.
pub fn encode_with_metadata(img: &Rgba8, o: &ExportOptions, meta: Option<&Metadata>) -> Result<Vec<u8>, String> {
    let mut img = finish_8bit(img, o);
    let space = o.effective_space();
    let mut profile = icc::write_named(named_space(space));
    let mut exif_space = exif_color_space(space);
    if let Some(target) = export_profile(o)? {
        // E08: ProPhoto → the chosen profile (8-bit)
        let mut f: Vec<f32> = img.data.iter().flat_map(|p| p.iter().map(|v| f32::from(*v) / 255.0)).collect();
        icc::convert_samples(&profile, &target, &mut f, 4)?;
        for (px, c) in img.data.iter_mut().zip(f.as_chunks::<4>().0) {
            *px = [0, 1, 2, 3].map(|k| (c[k].clamp(0.0, 1.0) * 255.0).round() as u8);
        }
        profile = target;
        exif_space = 0xFFFF;
    }
    let exif = meta.map(|m| lightcraft_meta::write_exif_with(m, Some(exif_space)));
    let xmp = meta.map(|m| lightcraft_meta::write_xmp(m, None));
    let meta = EncodeMeta { icc: Some(&profile), exif: exif.as_deref(), xmp: xmp.as_deref(), ppi: Some(o.ppi) };
    let e = EncodeImage::rgba8(&img);
    let r = match o.format {
        ExportFormat::Jpeg => {
            // 4:4:4 normally; 4:2:0 when targeting a file size (much smaller at equal visual quality).
            let sub = if o.limit_kb.is_some() { ChromaSubsampling::S420 } else { ChromaSubsampling::S444 };
            let jpeg = |q: u8| encode::encode_jpeg(&e, q, sub, &meta);
            match o.limit_kb {
                Some(kb) => {
                    let limit = kb as usize * 1024;
                    // Binary search for the highest quality that fits.
                    let (mut lo, mut hi) = (1u8, o.quality);
                    let mut best = None;
                    while lo <= hi {
                        let mid = lo + (hi - lo) / 2;
                        let b = jpeg(mid).map_err(|e| e.to_string())?;
                        if b.len() <= limit {
                            best = Some(b);
                            lo = mid + 1;
                        } else if mid == 1 {
                            break;
                        } else {
                            hi = mid - 1;
                        }
                    }
                    return best.ok_or_else(|| format!("cannot fit within {kb} KB"));
                }
                None => jpeg(o.quality),
            }
        }
        ExportFormat::Png => encode::encode_png(&e, &meta),
        ExportFormat::Tiff => encode::encode_tiff(&e, o.tiff_compression, &meta),
        ExportFormat::Webp => encode::encode_webp_lossless(&e, &meta),
        ExportFormat::Avif => encode::encode_avif(&e, o.quality, 8, &meta),
        ExportFormat::Jxl => lightcraft_codecs::encode_jxl(&e, &meta, &jxl_options(o)),
        ExportFormat::Psd => lightcraft_codecs::encode_psd(&e, &meta),
        f @ (ExportFormat::Original | ExportFormat::Dng) => return Err(format!("{f:?} export does not encode pixels")),
    };
    r.map_err(|e| e.to_string())
}

/// E08: the ICC profile an export converts to (ColorMatch RGB, or a profile file), checked.
pub fn export_profile(o: &ExportOptions) -> Result<Option<Vec<u8>>, String> {
    let Some(p) = o.icc_profile.as_deref() else { return Ok(None) };
    if matches!(o.format, ExportFormat::Jxl | ExportFormat::Webp | ExportFormat::Avif | ExportFormat::Original | ExportFormat::Dng) {
        return Ok(None);
    }
    let bytes = if p.eq_ignore_ascii_case("colorMatch") {
        icc::colormatch_profile()
    } else {
        let len = std::fs::metadata(p).map_err(|e| format!("{p}: {e}"))?.len();
        if len > icc::MAX_EXPORT_ICC as u64 {
            return Err(format!("{p}: larger than 16 MB, not an ICC profile"));
        }
        std::fs::read(p).map_err(|e| format!("{p}: {e}"))?
    };
    icc::check_export_profile(&bytes).map_err(|e| format!("{p}: {e}"))?;
    Ok(Some(bytes))
}

/// RGB ICC profiles installed on this Mac (ColorSync folders), by file path, for the Export
/// dialog's Profile popup.
pub fn installed_profiles() -> Vec<String> {
    let mut dirs = vec![std::path::PathBuf::from("/Library/ColorSync/Profiles"), std::path::PathBuf::from("/System/Library/ColorSync/Profiles")];
    if let Some(h) = std::env::var_os("HOME") {
        dirs.push(std::path::PathBuf::from(h).join("Library/ColorSync/Profiles"));
    }
    let mut out = Vec::new();
    for d in dirs {
        let Ok(rd) = std::fs::read_dir(&d) else { continue };
        for e in rd.flatten().take(500) {
            let p = e.path();
            if p.extension().is_some_and(|x| x.eq_ignore_ascii_case("icc") || x.eq_ignore_ascii_case("icm")) {
                out.push(p.display().to_string());
            }
        }
    }
    out.sort();
    out
}

/// Output sharpening and the watermark on an 8-bit render in `o.effective_space()`.
fn finish_8bit(img: &Rgba8, o: &ExportOptions) -> Rgba8 {
    let mut img = img.clone();
    output_sharpen(&mut img, o.sharpen, o.sharpen_amount);
    let space = o.effective_space();
    if let Some(wm) = &o.watermark {
        let wm = Watermark { color: srgb8_in(space, wm.color), target: Some(space), ..wm.clone() };
        draw_watermark(&mut img, &wm);
    }
    img
}

/// Output sharpening and the watermark on a high-bit-depth render.
fn finish_deep(img: &DeepImage, o: &ExportOptions) -> DeepImage {
    let mut img = img.clone();
    output_sharpen_deep(&mut img, o.sharpen, o.sharpen_amount);
    if let Some(wm) = &o.watermark {
        let wm = Watermark { color: srgb8_in(img.space, wm.color), target: Some(img.space), ..wm.clone() };
        draw_watermark_deep(&mut img, &wm);
    }
    img
}

/// Encode an ISO 21496-1 gain map JPEG from the SDR rendition `sdr` (8-bit, in the output space)
/// and the HDR rendition `hdr` (float linear, same space and size). The gain map is measured
/// against the base as written, so its 8-bit rounding is handed back by the map.
pub fn encode_gain_map_jpeg(sdr: &Rgba8, hdr: &DeepImage, o: &ExportOptions, meta: Option<&Metadata>) -> Result<Vec<u8>, String> {
    use lightcraft_codecs::gainmap;
    let base = finish_8bit(sdr, o);
    let hdr = finish_deep(hdr, o);
    let DeepSamples::F32(hv) = &hdr.samples else {
        return Err("HDR export needs a float HDR render".into());
    };
    let (w, h) = (base.width, base.height);
    if (hdr.width, hdr.height) != (w, h) || hv.len() < w * h * 3 {
        return Err("HDR and SDR renders differ in size".into());
    }
    let space = o.effective_space();
    let trc = space.trc();
    let sdr_lin: Vec<[f32; 3]> = base.data.iter().map(|p| [0, 1, 2].map(|c| trc.decode(p[c] as f32 / 255.0))).collect();
    let hdr_lin: &[[f32; 3]] = hv.as_chunks::<3>().0;
    let (map, gm) = gainmap::compute(&sdr_lin, hdr_lin, w, h, space.luma(), &gain_map_options(o.format)).map_err(|e| e.to_string())?;
    let profile = icc::write_named(named_space(space));
    let exif = meta.map(|m| lightcraft_meta::write_exif_with(m, Some(exif_color_space(space))));
    let xmp = meta.map(|m| lightcraft_meta::write_xmp(m, None));
    let em = EncodeMeta { icc: Some(&profile), exif: exif.as_deref(), xmp: xmp.as_deref(), ppi: Some(o.ppi) };
    if o.format == ExportFormat::Jxl {
        return lightcraft_codecs::encode_jxl_gain_map(&EncodeImage::rgba8(&base), &em, &jxl_options(o), &map, &gm).map_err(|e| e.to_string());
    }
    if o.format == ExportFormat::Avif {
        // a near-lossless gain-map JPEG, re-encoded by ImageIO as an AVIF with the same base,
        // metadata and gain map
        let jpeg = gainmap::encode_jpeg(&EncodeImage::rgba8(&base), &map, &gm, 100, ChromaSubsampling::S444, &em).map_err(|e| e.to_string())?;
        return crate::imageio::avif_gain_map(&jpeg, o.quality);
    }
    let sub = if o.limit_kb.is_some() { ChromaSubsampling::S420 } else { ChromaSubsampling::S444 };
    gainmap::encode_jpeg(&EncodeImage::rgba8(&base), &map, &gm, o.quality, sub, &em).map_err(|e| e.to_string())
}

/// How an HDR export's gain map is made. A JPEG gets a colour (R, G, B) gain map (ISO 21496-1): the
/// SDR rendition clips and desaturates highlights the HDR one keeps, which a single luminance gain
/// can't put back (macOS ImageIO decoding a LightCraft JPEG to HDR: headroom ΔEITP 31 with one
/// channel, 3.5 with three; `docs/lr-match.md` → HDR tone). An AVIF keeps the single channel:
/// ImageIO's AVIF writer drops a colour gain map (the file opens as its SDR base).
pub fn gain_map_options(format: ExportFormat) -> lightcraft_codecs::gainmap::GainMapOptions {
    lightcraft_codecs::gainmap::GainMapOptions { multichannel: format == ExportFormat::Jpeg, ..Default::default() }
}

/// The codecs' name of an output space (for its ICC profile).
pub fn named_space(s: OutputSpace) -> NamedSpace {
    match s {
        OutputSpace::Srgb => NamedSpace::Srgb,
        OutputSpace::DisplayP3 => NamedSpace::DisplayP3,
        OutputSpace::AdobeRgb => NamedSpace::AdobeRgb,
        OutputSpace::ProPhoto => NamedSpace::ProPhoto,
        OutputSpace::Rec2020 => NamedSpace::Rec2020,
    }
}

/// An 8-bit sRGB colour expressed in `space` (encoded with its curve).
pub fn srgb8_in(space: OutputSpace, c: [u8; 3]) -> [u8; 3] {
    if space == OutputSpace::Srgb {
        return c;
    }
    let lin = c.map(lightcraft_color::transfer::decode_srgb8);
    let m = lightcraft_color::SRGB.to_space(&space.rgb_space()).to_f32();
    let t = space.trc();
    std::array::from_fn(|i| {
        let v = m[i][0] * lin[0] + m[i][1] * lin[1] + m[i][2] * lin[2];
        (t.encode(v) * 255.0 + 0.5) as u8
    })
}

/// Encode a render according to `o`: its high-bit-depth samples when it has them (16-bit PNG/TIFF,
/// 10-bit AVIF, 32-bit float linear TIFF), else its 8-bit image.
pub fn encode_rendered(r: &lightcraft_pipeline::Rendered, o: &ExportOptions, meta: Option<&Metadata>) -> Result<Vec<u8>, String> {
    match &r.deep {
        Some(d) if o.effective_depth() != OutputDepth::U8 => encode_deep(d, o, meta),
        _ => encode_with_metadata(&r.image, o, meta),
    }
}

/// Encode a high-bit-depth image (see [`encode_rendered`]).
pub fn encode_deep(img: &DeepImage, o: &ExportOptions, meta: Option<&Metadata>) -> Result<Vec<u8>, String> {
    let mut img = finish_deep(img, o);
    let mut profile = match img.samples {
        DeepSamples::F32(_) => icc::write_matrix_trc(&img.space.rgb_space(), &lightcraft_codecs::Trc::Linear),
        DeepSamples::U16(_) => icc::write_named(named_space(img.space)),
    };
    let mut exif_space = exif_color_space(img.space);
    if let (Some(target), DeepSamples::U16(v)) = (export_profile(o)?, &mut img.samples) {
        // E08: ProPhoto → the chosen profile (16-bit)
        let mut f: Vec<f32> = v.iter().map(|x| f32::from(*x) / 65535.0).collect();
        icc::convert_samples(&profile, &target, &mut f, 3)?;
        for (x, c) in v.iter_mut().zip(&f) {
            *x = (c.clamp(0.0, 1.0) * 65535.0).round() as u16;
        }
        profile = target;
        exif_space = 0xFFFF;
    }
    let exif = meta.map(|m| lightcraft_meta::write_exif_with(m, Some(exif_space)));
    let xmp = meta.map(|m| lightcraft_meta::write_xmp(m, None));
    let meta = EncodeMeta { icc: Some(&profile), exif: exif.as_deref(), xmp: xmp.as_deref(), ppi: Some(o.ppi) };
    let (w, h) = (img.width as u32, img.height as u32);
    let e = match &img.samples {
        DeepSamples::U16(v) => EncodeImage::new(w, h, 3, Samples::U16(v)),
        DeepSamples::F32(v) => EncodeImage::new(w, h, 3, Samples::F32(v)),
    };
    let r = match (o.format, &img.samples) {
        (ExportFormat::Tiff, _) => encode::encode_tiff(&e, o.tiff_compression, &meta),
        (ExportFormat::Avif, DeepSamples::F32(v)) if img.space == OutputSpace::Rec2020 => {
            lightcraft_codecs::encode_avif_hdr(w, h, v, o.quality, 8, &meta, o.hdr_transfer.codec())
        }
        (ExportFormat::Png, DeepSamples::F32(v)) if img.space == OutputSpace::Rec2020 && o.hdr => {
            lightcraft_codecs::encode_png_hdr(w, h, v, &meta, o.hdr_transfer.codec())
        }
        (ExportFormat::Jxl, DeepSamples::F32(v)) if img.space == OutputSpace::Rec2020 && o.hdr => {
            lightcraft_codecs::encode_jxl_hdr(w, h, v, &meta, &jxl_options(o), o.hdr_transfer.codec())
        }
        (ExportFormat::Avif, _) => encode::encode_avif(&e, o.quality, 8, &meta),
        (ExportFormat::Png, _) => encode::encode_png(&e, &meta),
        (ExportFormat::Jxl, _) => lightcraft_codecs::encode_jxl(&e, &meta, &jxl_options(o)),
        (ExportFormat::Psd, _) => lightcraft_codecs::encode_psd(&e, &meta),
        (f, _) => return Err(format!("{f:?} export is 8-bit only")),
    };
    r.map_err(|e| e.to_string())
}

/// JPEG XL settings from the export options: quality 100 = lossless; the colour encoding of the
/// output space (ProPhoto / Adobe RGB have no JPEG XL enum: sRGB primaries are declared and the
/// ICC profile rides along only for lossless files, so those keep to sRGB / P3 / Rec.2020).
fn jxl_options(o: &ExportOptions) -> lightcraft_codecs::JxlOptions {
    let colour = match o.effective_space() {
        OutputSpace::DisplayP3 => lightcraft_codecs::JxlColour::DisplayP3,
        OutputSpace::Rec2020 => lightcraft_codecs::JxlColour::Rec2020,
        _ => lightcraft_codecs::JxlColour::Srgb,
    };
    lightcraft_codecs::JxlOptions { quality: o.quality, lossless: o.quality >= 100, colour }
}

/// Parse a shutter speed such as `1/250`, `0.5` or `2"` into seconds.
fn parse_shutter(s: &str) -> Option<f64> {
    let s = s.trim().trim_end_matches(['s', '"']).trim();
    match s.split_once('/') {
        Some((n, d)) => Some(n.trim().parse::<f64>().ok()? / d.trim().parse::<f64>().ok()?),
        None => s.parse().ok(),
    }
    .filter(|v: &f64| v.is_finite() && *v > 0.0)
}

/// The Exif `ColorSpace` for pixels in `space`, as Lightroom writes it: 1 = sRGB, else 0xFFFF
/// (uncalibrated: the embedded ICC profile names the space).
fn exif_color_space(space: lightcraft_pipeline::OutputSpace) -> u16 {
    if space == lightcraft_pipeline::OutputSpace::Srgb { 1 } else { 0xFFFF }
}

/// The camera's Exif `Make` and `Model` back from the library's "make model" string (as import
/// joined them): a known maker's name, else up to where the model repeats the maker's first word
/// ("Canon Canon EOS R6", "LEICA CAMERA AG LEICA Q2"), else the first word.
pub fn split_camera(camera: &str) -> (Option<String>, Option<String>) {
    const MAKES: &[&str] = &[
        "RICOH IMAGING COMPANY, LTD.",
        "OLYMPUS IMAGING CORP.",
        "OLYMPUS CORPORATION",
        "OM Digital Solutions",
        "EASTMAN KODAK COMPANY",
        "Minolta Co., Ltd.",
        "KONICA MINOLTA",
        "NIKON CORPORATION",
        "LEICA CAMERA AG",
        "Blackmagic Design",
        "Phase One",
        "Hasselblad",
        "PENTAX Corporation",
        "SAMSUNG TECHWIN",
    ];
    let c = camera.trim();
    if c.is_empty() {
        return (None, None);
    }
    let split = |at: usize| {
        let (make, model) = (c[..at].trim(), c[at..].trim());
        (Some(make.to_string()).filter(|m| !m.is_empty()), Some(model.to_string()).filter(|m| !m.is_empty()))
    };
    if let Some(m) = MAKES
        .iter()
        .find(|m| c.len() > m.len() && c.is_char_boundary(m.len()) && c[..m.len()].eq_ignore_ascii_case(m) && c[m.len()..].starts_with(' '))
    {
        return split(m.len());
    }
    let words: Vec<(usize, &str)> = c.split_whitespace().map(|w| ((w.as_ptr() as usize).saturating_sub(c.as_ptr() as usize), w)).collect();
    let Some(&(_, first)) = words.first() else { return (None, None) };
    if words.len() == 1 {
        return (None, Some(c.to_string()));
    }
    if let Some(&(i, _)) = words.iter().skip(1).find(|(_, w)| w.eq_ignore_ascii_case(first)) {
        return split(i);
    }
    words.get(1).map_or((None, Some(c.to_string())), |&(i, _)| split(i))
}

/// The metadata to embed for `photo` under `o.metadata` / `o.remove_location`. `None` = embed nothing.
pub fn export_metadata(photo: &lightcraft_catalog::Photo, o: &ExportOptions) -> Option<Metadata> {
    export_metadata_with(None, photo, o)
}

/// [`export_metadata`] with the library's keyword list (synonyms, export options, person keywords).
pub fn export_metadata_with(cat: Option<&lightcraft_catalog::Catalog>, photo: &lightcraft_catalog::Photo, o: &ExportOptions) -> Option<Metadata> {
    let m = &photo.meta;
    let text = |s: &str| (!s.trim().is_empty()).then(|| s.to_string());
    // copyright info (also under "copyright only"): notice, creator, status, usage terms, info URL
    let mut out = Metadata {
        copyright: text(&m.copyright),
        copyright_marked: m.copyright_status.marked(),
        usage_terms: text(&m.usage_terms),
        copyright_url: text(&m.copyright_url),
        artist: text(&m.creator),
        software: Some("LightCraft".into()),
        ..Default::default()
    };
    match o.metadata {
        MetadataPolicy::None => return None,
        MetadataPolicy::Copyright => return Some(Metadata { artist: None, ..out }),
        MetadataPolicy::CopyrightContact => return Some(out),
        MetadataPolicy::All | MetadataPolicy::AllExceptCameraRaw | MetadataPolicy::AllExceptCamera => {}
    }
    out.title = text(&m.title);
    out.caption = text(&m.caption);
    out.alt_text = text(&m.alt_text);
    out.extended_description = text(&m.extended_description);
    if !o.remove_location {
        out.sublocation = text(&m.location);
        out.city = text(&m.city);
        out.state = text(&m.state);
        out.country = text(&m.country);
    }
    (out.keywords, out.hierarchical_keywords) = export_keywords_with(cat, photo, o);
    // IPTC Extension (L21): Remove Person Info leaves out what describes the people
    out.iptc_ext = m
        .iptc_ext
        .iter()
        .filter(|(k, _)| !(o.remove_people && lightcraft_meta::iptc_ext::field(k).is_some_and(|f| f.person)))
        .map(|(k, v)| (k.clone(), v.clone()))
        .collect();
    out.capture_time = photo.captured.as_deref().and_then(DateTime::parse_iso);
    out.rating = (photo.rating > 0).then_some(photo.rating as i8);
    // Pixels are exported upright: orientation is baked in.
    out.orientation = Some(lightcraft_meta::Orientation::Normal);
    // (a photo inside a saved location marked Private keeps its position to the library)
    if !o.remove_location && !cat.is_some_and(|c| crate::map::in_private_location(c, m.gps)) {
        out.gps = m.gps.map(|(latitude, longitude)| Gps { latitude, longitude, altitude: None });
    }
    if matches!(o.metadata, MetadataPolicy::All | MetadataPolicy::AllExceptCameraRaw) {
        (out.make, out.model) = split_camera(&m.camera);
        out.lens_model = text(&m.lens);
        out.focal_length = m.focal_mm.map(f64::from);
        out.f_number = m.aperture.map(f64::from);
        out.exposure_time = parse_shutter(&m.shutter);
        out.iso = m.iso;
    }
    Some(out)
}

/// The keywords an export writes (E06), with default keyword options: see [`export_keywords_with`].
pub fn export_keywords(photo: &lightcraft_catalog::Photo, o: &ExportOptions) -> (Vec<String>, Vec<String>) {
    export_keywords_with(None, photo, o)
}

/// The keywords an export writes (E06, L11): each `a|b|c` keyword as its own words `a`, `b`, `c`
/// (Lightroom includes the containing keywords unless the keyword says not to), its synonyms, none
/// of a keyword left out of export (the keyword list's options), the named people unless Remove
/// Person Info (which also leaves out keywords marked as a person), and the full paths as
/// `lr:hierarchicalSubject` when Write Keywords as Lightroom Hierarchy is on.
pub fn export_keywords_with(
    cat: Option<&lightcraft_catalog::Catalog>,
    photo: &lightcraft_catalog::Photo,
    o: &ExportOptions,
) -> (Vec<String>, Vec<String>) {
    let empty;
    let cat = match cat {
        Some(c) => c,
        None => {
            empty = lightcraft_catalog::Catalog::new();
            &empty
        }
    };
    let kws: Vec<String> =
        photo.meta.keywords.iter().filter(|k| !(o.remove_people && cat.keyword_info(k).is_some_and(|i| i.person))).cloned().collect();
    let ek = cat.export_keywords(&kws);
    let mut flat = ek.flat;
    if !o.remove_people {
        for r in photo.meta.regions.iter().filter(|r| r.kind == lightcraft_meta::RegionKind::Face) {
            // a person keyword left out of export (its own Include on Export option) keeps its name out too
            let hidden = |n: &str| {
                cat.listed_keywords().any(|k| {
                    k.info.person && !k.info.include_on_export && k.path.rsplit('|').next().is_some_and(|l| l.trim().eq_ignore_ascii_case(n))
                })
            };
            if let Some(n) = r.name.as_deref().map(str::trim).filter(|n| !n.is_empty())
                && !flat.iter().any(|x| x.eq_ignore_ascii_case(n))
                && !hidden(n)
            {
                flat.push(n.to_string());
            }
        }
    }
    let hier = if o.keyword_hierarchy { ek.hierarchical } else { Vec::new() };
    (flat, hier)
}

/// One exported file.
pub struct Exported {
    pub file_name: String,
    pub bytes: Vec<u8>,
    pub width: usize,
    pub height: usize,
    /// Files written next to it, named like it with another extension: `(extension, bytes)` (the
    /// XMP sidecar of an `Original` export).
    pub sidecars: Vec<(&'static str, Vec<u8>)>,
}

/// Output size of photo `p` under `o` (its cropped full size when `o.resize` is `None`).
pub fn output_size(p: &lightcraft_catalog::Photo, o: &ExportOptions) -> (usize, usize) {
    let (w, h) = lightcraft_pipeline::native_output_size(p.width.max(1) as usize, p.height.max(1) as usize, &p.develop);
    match &o.resize {
        Some(r) => r.in_pixels(o.ppi).apply(w, h),
        None => ((w.round() as usize).max(1), (h.round() as usize).max(1)),
    }
}

/// The Export dialog's before-export preview (LightCraft): photo `id` exported small (long edge
/// at most `max_edge`) with everything `o` does to it — size, sharpening, watermark — once as
/// `o`'s format (to estimate the file size) and once as an 8-bit sRGB PNG (to show). Prepared
/// here; [`PreviewJob::run`] renders on any thread.
pub struct PreviewJob {
    /// The full-size output (`o`'s), in pixels.
    pub size: (usize, usize),
    /// Output pixels per preview pixel (for the size estimate).
    pub area_ratio: f64,
    /// The size samples (tiny, a quarter of the preview's pixels, the preview's), in the format.
    sized: Option<[PreparedExport; 3]>,
    shown: PreparedExport,
    limit_kb: Option<u32>,
    /// An Original / DNG export: the file's own size (bytes) is the estimate.
    file_bytes: Option<u64>,
}

/// What [`PreviewJob::run`] made: the preview PNG and the estimated file size.
pub struct Preview {
    /// The preview, 8-bit sRGB.
    pub image: Rgba8,
    pub width: usize,
    pub height: usize,
    /// Estimated bytes of the exported file (`None`: no estimate for this format).
    pub estimated_bytes: Option<u64>,
}

/// Set up a [`PreviewJob`] for photo `id` (`None` for a video).
pub fn preview_job(
    session: &mut crate::Session,
    id: lightcraft_catalog::PhotoId,
    o: &ExportOptions,
    max_edge: u32,
) -> Result<Option<PreviewJob>, String> {
    let p = session.catalog.photo(id).ok_or("no such photo")?;
    if p.kind == lightcraft_catalog::MediaKind::Video {
        return Ok(None);
    }
    let file_bytes = (!o.format.is_rendered()).then_some(p.file_size);
    let size = if o.format.is_rendered() { output_size(p, o) } else { (p.width.max(1) as usize, p.height.max(1) as usize) };
    let long = size.0.max(size.1).max(1) as f64;
    let small_long = long.min(f64::from(max_edge.max(16))).round();
    let k = small_long / long;
    let (pw, ph) = (((size.0 as f64 * k).round()).max(1.0), ((size.1 as f64 * k).round()).max(1.0));
    let area_ratio = (size.0 as f64 * size.1 as f64) / (pw * ph);
    let small = Resize { mode: ResizeMode::LongEdge, value: small_long as f32, height: 0.0, dont_enlarge: false, unit: SizeUnit::Pixels };
    let base = if o.format.is_rendered() { o.clone() } else { ExportOptions { format: ExportFormat::Jpeg, ..o.clone() } };
    let sized_opts = ExportOptions { resize: Some(small), limit_kb: None, after: AfterExport::Nothing, add_to_catalog: false, ..base.clone() };
    let shown_opts = ExportOptions {
        format: ExportFormat::Png,
        bit_depth: Some(8),
        color_space: OutputSpace::Srgb,
        icc_profile: None,
        hdr: false,
        metadata: MetadataPolicy::None,
        ..sized_opts.clone()
    };
    // the size samples: the preview's size, half of it, and a tiny one (the fixed overhead:
    // headers, profile, metadata)
    let at = |edge: f64| ExportOptions {
        resize: Some(Resize { value: edge.max(8.0).round() as f32, ..small }),
        limit_kb: None,
        after: AfterExport::Nothing,
        add_to_catalog: false,
        ..base.clone()
    };
    let sized = if o.format.is_rendered() && area_ratio > 1.0 {
        Some([
            prepare_export(session, id, &at(16.0), 1)?,
            prepare_export(session, id, &at(small_long / 2.0), 1)?,
            prepare_export(session, id, &sized_opts, 1)?,
        ])
    } else if o.format.is_rendered() {
        // the preview is the full size: its own file is the answer
        Some([prepare_export(session, id, &at(16.0), 1)?, prepare_export(session, id, &sized_opts, 1)?, prepare_export(session, id, &sized_opts, 1)?])
    } else {
        None
    };
    let shown = prepare_export(session, id, &shown_opts, 1)?;
    Ok(Some(PreviewJob { size, area_ratio, sized, shown, limit_kb: o.limit_kb.filter(|_| o.format == ExportFormat::Jpeg), file_bytes }))
}

/// Estimated bytes at `area_ratio` × the pixels of the `big` sample, from samples at a tiny size
/// (`overhead` bytes), a quarter of `big`'s pixels (`half`) and `big`: the image data grows as a
/// power of the pixel count (detail per pixel falls as the picture gets larger), fitted from the
/// two samples.
pub fn extrapolate_size(overhead: u64, half: u64, big: u64, area_ratio: f64) -> u64 {
    let h = overhead as f64 * 0.9;
    let (a, b) = ((half as f64 - h).max(1.0), (big as f64 - h).max(1.0));
    // the samples are 4× apart in pixels
    let alpha = ((b / a).ln() / 4f64.ln()).clamp(0.45, 1.0);
    let alpha = if alpha.is_finite() { alpha } else { 1.0 };
    (h + b * area_ratio.max(1.0).powf(alpha)).round().max(0.0) as u64
}

impl PreviewJob {
    pub fn run(self) -> Result<Preview, String> {
        let shown = self.shown.run()?;
        let estimated_bytes = match (self.file_bytes, self.sized) {
            (Some(b), _) => Some(b),
            (None, Some([tiny, half, big])) => {
                let (t, b) = (tiny.run()?.bytes.len() as u64, big.run()?.bytes.len() as u64);
                let est = if self.area_ratio > 1.0 { extrapolate_size(t, half.run()?.bytes.len() as u64, b, self.area_ratio) } else { b };
                Some(match self.limit_kb {
                    Some(kb) => est.min(u64::from(kb) * 1024),
                    None => est,
                })
            }
            (None, None) => None,
        };
        let image = lightcraft_codecs::decode_thumbnail(&shown.bytes, shown.width.max(shown.height).max(1) as u32).map_err(|e| e.to_string())?.image;
        Ok(Preview { width: shown.width, height: shown.height, image, estimated_bytes })
    }
}

/// `1.2 MB`, `640 KB`, `12 bytes`.
pub fn human_bytes(b: u64) -> String {
    match b {
        0..1024 => format!("{b} bytes"),
        1024..1_048_576 => format!("{:.0} KB", b as f64 / 1024.0),
        1_048_576..1_073_741_824 => format!("{:.1} MB", b as f64 / 1_048_576.0),
        _ => format!("{:.2} GB", b as f64 / 1_073_741_824.0),
    }
}

/// Render photo `id` at the requested size and encode it (or copy / convert its original for
/// [`ExportFormat::Original`] / [`ExportFormat::Dng`]).
pub fn export_photo(session: &mut crate::Session, id: lightcraft_catalog::PhotoId, o: &ExportOptions, seq: usize) -> Result<Exported, String> {
    prepare_export(session, id, o, seq)?.run()
}

/// One photo's export, set up from the session ([`prepare_export`]). [`PreparedExport::run`] does
/// the heavy part (read, decode, render, encode) without the session, so it can run on another
/// thread.
pub struct PreparedExport {
    pub photo: lightcraft_catalog::PhotoId,
    pub file_name: String,
    /// The original's folder (Export To ▸ Same folder as original photo).
    pub source_dir: Option<String>,
    /// What to do if this photo's file exists, answered for it (Existing Files ▸ Ask what to do);
    /// `None` = the batch's policy.
    pub conflict: Option<Conflict>,
    work: Work,
    /// The library's originals, which [`run_batch`] never writes over (shared by a batch).
    guard: std::sync::Arc<crate::originals::OriginalGuard>,
}

impl PreparedExport {
    /// The file names this export writes besides its main file's (the XMP sidecar of an
    /// `Original` export), as extensions.
    fn sidecar_exts(&self) -> &'static [&'static str] {
        match self.work {
            Work::File { dng: None, .. } => &["xmp"],
            _ => &[],
        }
    }
}

/// The folder photo `item` is written to: the batch's (with the subfolder), or with Export To ▸
/// Same folder as original photo the original's.
fn target_dir(item_source: Option<&str>, o: &ExportOptions, to: &Destination) -> String {
    let join = |a: &str, b: &str| if a.is_empty() { b.to_string() } else { format!("{}/{b}", a.trim_end_matches('/')) };
    let base = match item_source.filter(|_| o.export_to == ExportTo::SameFolder) {
        Some(src) => src.to_string(),
        None => to.dir.clone(),
    };
    if o.subfolder.is_empty() { base } else { join(&base, &o.subfolder) }
}

/// Existing Files ▸ Ask what to do: the photos whose file (or sidecar) is already there, as
/// `(photo, path)`, before anything is rendered — what the desktop app asks about. Names two
/// photos of the batch would both take count too (the second one's).
pub fn existing_targets(
    items: &[PreparedExport],
    o: &ExportOptions,
    to: &Destination,
    exists: &dyn Fn(&str) -> bool,
) -> Vec<(lightcraft_catalog::PhotoId, String)> {
    if to.exact.is_some() && items.len() == 1 {
        return Vec::new();
    }
    let mut seen = std::collections::HashSet::new();
    let mut out = Vec::new();
    for item in items {
        let dir = target_dir(item.source_dir.as_deref(), o, to);
        let path = if dir.is_empty() { item.file_name.clone() } else { format!("{}/{}", dir.trim_end_matches('/'), item.file_name) };
        let group: Vec<String> = std::iter::once(path.clone()).chain(item.sidecar_exts().iter().map(|x| sidecar_path(&path, x))).collect();
        if group.iter().any(|p| seen.contains(p) || exists(p)) {
            out.push((item.photo, path));
        }
        seen.extend(group);
    }
    out
}

/// Apply `resolve` (`{"<photo id>": unique|overwrite|skip}`, the answers to Ask what to do) to
/// prepared exports.
pub fn apply_resolutions(items: &mut [PreparedExport], resolve: Option<&serde_json::Value>) {
    let Some(map) = resolve.and_then(serde_json::Value::as_object) else { return };
    for item in items {
        if let Some(c) = map.get(&item.photo.0.to_string()).cloned().and_then(|v| serde_json::from_value::<Conflict>(v).ok()) {
            item.conflict = Some(c);
        }
    }
}

struct VideoWork {
    path: String,
    range: (f64, Option<f64>),
    opts: crate::video::VideoExport,
    develop: std::sync::Arc<lightcraft_develop::DevelopSettings>,
}

struct RenderWork {
    job: crate::media::RenderJob,
    /// The HDR rendition of a gain map JPEG (`job` renders its SDR base).
    hdr_job: Option<crate::media::RenderJob>,
    meta: Option<Metadata>,
    opts: ExportOptions,
}

enum Work {
    Render(Box<RenderWork>),
    /// LR-VID-EXPORT: a video, trimmed and with its edits ([`crate::video::export`]).
    Video(Box<VideoWork>),
    /// [`ExportFormat::Original`] (the file's bytes + an XMP sidecar) or [`ExportFormat::Dng`] (the
    /// raw data re-encoded as a lossless DNG with the edits in its XMP).
    File {
        path: String,
        read: Option<crate::merge::ByteReader>,
        packet: String,
        dng: Option<Box<DngWork>>,
        label: String,
        size: (usize, usize),
    },
}

/// Set up the export of photo `id` at 1-based position `seq` of a batch. (For a whole batch use
/// [`prepare_batch`]: it looks at the library's originals once.)
pub fn prepare_export(
    session: &mut crate::Session,
    id: lightcraft_catalog::PhotoId,
    o: &ExportOptions,
    seq: usize,
) -> Result<PreparedExport, String> {
    let guard = std::sync::Arc::new(session.original_guard());
    prepare_guarded(session, id, o, seq, 0, guard)
}

/// [`prepare_export`] for each of `ids` in order (`{seq}` = position, from 1).
pub fn prepare_batch(session: &mut crate::Session, ids: &[lightcraft_catalog::PhotoId], o: &ExportOptions) -> Result<Vec<PreparedExport>, String> {
    let guard = std::sync::Arc::new(session.original_guard());
    // Include Video Files off: a selection's videos are left out (Lightroom's Video section)
    let is_video = |id: &lightcraft_catalog::PhotoId| session.catalog.photo(*id).is_some_and(|p| p.kind == lightcraft_catalog::MediaKind::Video);
    let ids: Vec<lightcraft_catalog::PhotoId> = ids.iter().copied().filter(|id| o.video.include || !is_video(id)).collect();
    ids.iter().enumerate().map(|(i, id)| prepare_guarded(session, *id, o, i + 1, ids.len(), guard.clone())).collect()
}

fn prepare_guarded(
    session: &mut crate::Session,
    id: lightcraft_catalog::PhotoId,
    o: &ExportOptions,
    seq: usize,
    total: usize,
    guard: std::sync::Arc<crate::originals::OriginalGuard>,
) -> Result<PreparedExport, String> {
    let p = session.catalog.photo(id).ok_or("no such photo")?;
    let file_name = o.file_name_in(p, seq, total);
    // HDR output only for photos edited in HDR; the rest of the batch exports as usual
    let sdr_opts;
    let o = if o.hdr && !p.develop.hdr.enabled {
        sdr_opts = ExportOptions { hdr: false, ..o.clone() };
        &sdr_opts
    } else {
        o
    };
    let source_dir = match &p.source {
        lightcraft_catalog::Source::File { path } => std::path::Path::new(path).parent().map(|d| d.to_string_lossy().to_string()),
        lightcraft_catalog::Source::Demo { .. } => None,
    };
    let work = if p.kind == lightcraft_catalog::MediaKind::Video {
        if !o.video.include {
            return Err(format!("{}: videos are left out (turn on Include Video Files)", p.file_name));
        }
        let lightcraft_catalog::Source::File { path } = &p.source else { return Err(format!("{}: no video file", p.file_name)) };
        let range = p.video.unwrap_or_default().range(p.duration);
        let name = crate::video::export_name(&file_name, o.video.format);
        let file_name = match o.video.format {
            crate::video::VideoFormat::H264 => name,
            // Original keeps the file's own extension
            crate::video::VideoFormat::Original => {
                let ext = std::path::Path::new(path).extension().map(|e| e.to_string_lossy().to_string()).unwrap_or_else(|| "mov".into());
                format!(
                    "{}.{ext}",
                    std::path::Path::new(&file_name).file_stem().map(|s| s.to_string_lossy().to_string()).unwrap_or_else(|| "video".into())
                )
            }
        };
        let work = Work::Video(Box::new(VideoWork { path: path.clone(), range, opts: o.video, develop: p.develop.clone() }));
        return Ok(PreparedExport { photo: id, file_name, work, guard, source_dir, conflict: None });
    } else if o.format.is_rendered() {
        let (w, h) = output_size(p, o);
        let meta = export_metadata_with(Some(&session.catalog), p, o);
        let mut opts = o.clone();
        // a text watermark is a text template (E10): its tokens are this photo's ({title},
        // {date:%Y}, {creator}, {seq} …, the file-name tokens)
        if let Some(wm) = opts.watermark.as_mut().filter(|w| w.text.contains('{')) {
            let batch = crate::rename::Batch { total, text: o.custom_text.clone(), ..Default::default() };
            wm.text = crate::rename::expand_tokens_in(&wm.text, p, seq, 1, &batch);
        }
        // Simple Copyright Watermark (`{copyright}`) on a photo with no copyright: no watermark,
        // as in Lightroom
        if opts.watermark.as_ref().is_some_and(|w| w.text.trim().is_empty() && w.image.trim().is_empty()) {
            opts.watermark = None;
        }
        let gain_map = o.gain_map_output();
        let job = session.export_job(id, w, h, o.effective_space(), o.effective_depth())?;
        let hdr_job = if gain_map { Some(session.export_job(id, w, h, o.effective_space(), OutputDepth::F32Hdr)?) } else { None };
        Work::Render(Box::new(RenderWork { job, hdr_job, meta, opts }))
    } else {
        let dng = if o.format == ExportFormat::Dng { Some(Box::new(DngWork::prepare(session, id, o)?)) } else { None };
        let p = session.catalog.photo(id).ok_or("no such photo")?;
        let lightcraft_catalog::Source::File { path } = &p.source else {
            return Err(format!("{} is a generated demo photo: it has no original file to export", p.file_name));
        };
        Work::File {
            path: path.clone(),
            read: session.media.file_bytes.clone(),
            packet: crate::sidecar::sidecar_packet(p, &session.catalog),
            dng,
            label: p.file_name.clone(),
            size: (p.width as usize, p.height as usize),
        }
    };
    Ok(PreparedExport { photo: id, file_name, work, guard, source_dir, conflict: None })
}

impl PreparedExport {
    /// The text watermark this photo gets, its tokens expanded (rendered exports only).
    pub fn watermark_text(&self) -> Option<String> {
        match &self.work {
            Work::Render(r) => r.opts.watermark.as_ref().map(|w| w.text.clone()),
            _ => None,
        }
    }

    /// A video export (encoded by ffmpeg / AVFoundation rather than rendered).
    pub fn is_video(&self) -> bool {
        matches!(self.work, Work::Video(_))
    }

    pub fn run(self) -> Result<Exported, String> {
        let file_name = self.file_name;
        match self.work {
            Work::Video(v) => {
                // encoded to a temporary file, then handed to the batch's writer like any export
                let info = crate::video::probe(&v.path)?;
                let tmp = std::env::temp_dir().join(format!(
                    "lc-video-export-{}-{:x}-{}",
                    std::process::id(),
                    lightcraft_preview::hash_bytes(format!("{}{file_name}", v.path).as_bytes()).0,
                    file_name.replace(['/', '\\'], "_")
                ));
                let r = crate::video::export(&v.path, &tmp, &info, v.range, v.opts, Some(&v.develop), &|_| true);
                let bytes = r.and_then(|notice| {
                    if let Some(n) = notice {
                        log::warn!("{file_name}: {n}");
                    }
                    std::fs::read(&tmp).map_err(|e| format!("{}: {e}", tmp.display()))
                });
                let _ = std::fs::remove_file(&tmp);
                let (w, h) = match v.opts.format {
                    crate::video::VideoFormat::H264 => crate::video::export_size(&info, v.opts.quality),
                    crate::video::VideoFormat::Original => (info.width, info.height),
                };
                Ok(Exported { file_name, bytes: bytes?, width: w as usize, height: h as usize, sidecars: Vec::new() })
            }
            Work::Render(w) => {
                let RenderWork { job, hdr_job, meta, opts } = *w;
                let r = job.run().rendered?;
                let bytes = match hdr_job {
                    Some(hj) => {
                        let hr = hj.run().rendered?;
                        let hdr = hr.deep.ok_or("the HDR render has no float samples")?;
                        encode_gain_map_jpeg(&r.image, &hdr, &opts, meta.as_ref())?
                    }
                    None => encode_rendered(&r, &opts, meta.as_ref())?,
                };
                Ok(Exported { file_name, bytes, width: r.image.width, height: r.image.height, sidecars: Vec::new() })
            }
            Work::File { path, read, packet, dng, label, size } => {
                let bytes = match &read {
                    Some(r) => r(&path)?,
                    None => std::fs::read(&path).map_err(|e| format!("{path}: {e}"))?,
                };
                let Some(dng) = dng else {
                    return Ok(Exported { file_name, bytes, width: size.0, height: size.1, sidecars: vec![("xmp", packet.into_bytes())] });
                };
                if lightcraft_raw::probe(&bytes).is_none() {
                    return Err(format!("{label}: DNG export needs a raw photo"));
                }
                let raw = lightcraft_raw::decode(&bytes).map_err(|e| format!("{label}: {e}"))?;
                drop(bytes);
                let (bytes, notice) = dng.write(&raw, packet, &label)?;
                if let Some(n) = notice {
                    log::warn!("{n}");
                }
                Ok(Exported { file_name, bytes, width: raw.width, height: raw.height, sidecars: Vec::new() })
            }
        }
    }
}

/// LR-EXP-DNGOPT: the JPEG preview a DNG export embeds (Lightroom's File Handling ▸ JPEG Preview).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum DngPreview {
    /// Only the small IFD0 thumbnail.
    None,
    /// 1024 px on the long edge.
    #[default]
    Medium,
    /// The photo's full size.
    Full,
}

/// Long edge of a DNG's Medium Size preview and of its thumbnail.
const DNG_MEDIUM: usize = 1024;
const DNG_THUMB: usize = 256;

/// A DNG export's extras: the previews (rendered with the photo's edits, as Lightroom embeds
/// them) and lossy output, both written by Adobe's DNG SDK.
struct DngWork {
    compression: DngCompression,
    preview: DngPreview,
    lossy: bool,
    /// Long edge of a lossy DNG (0 = full size): the Image Sizing size.
    lossy_size: u32,
    thumb_job: Option<crate::media::RenderJob>,
    preview_job: Option<crate::media::RenderJob>,
}

impl DngWork {
    fn prepare(session: &mut crate::Session, id: lightcraft_catalog::PhotoId, o: &ExportOptions) -> Result<DngWork, String> {
        let p = session.catalog.photo(id).ok_or("no such photo")?;
        let lossy_size = if o.dng_lossy && o.resize.is_some() {
            let (w, h) = output_size(p, o);
            w.max(h).min(65_000) as u32
        } else {
            0
        };
        let full = lightcraft_pipeline::native_output_size(p.width.max(1) as usize, p.height.max(1) as usize, &p.develop);
        let preview_edge = match o.dng_preview {
            DngPreview::None => 0,
            DngPreview::Medium => DNG_MEDIUM,
            DngPreview::Full => full.0.max(full.1) as usize,
        };
        // (renders are only worth making when the SDK is there to embed them)
        let sdk = lightcraft_dng_sdk_sys::available();
        let mut job = |edge: usize| -> Option<crate::media::RenderJob> {
            (sdk && edge > 0).then(|| session.export_job(id, edge, edge, OutputSpace::Srgb, OutputDepth::U8).ok()).flatten()
        };
        let thumb_job = if o.dng_preview != DngPreview::None || o.dng_lossy { job(DNG_THUMB) } else { None };
        let preview_job = job(preview_edge);
        Ok(DngWork { compression: o.dng_compression, preview: o.dng_preview, lossy: o.dng_lossy, lossy_size, thumb_job, preview_job })
    }

    /// The rendered image as 8-bit RGB (`None` when the render failed: the SDK renders instead).
    fn rgb(job: Option<crate::media::RenderJob>) -> Option<(u32, u32, Vec<u8>)> {
        let r = job?.run().rendered.ok()?;
        let img = &r.image;
        let rgb: Vec<u8> = img.data.iter().flat_map(|p| [p[0], p[1], p[2]]).collect();
        Some((img.width as u32, img.height as u32, rgb))
    }

    /// `raw` written as a DNG with the previews / lossy data asked for and `packet` as its XMP; the
    /// plain LightCraft DNG (no preview, not lossy) when the SDK isn't there or fails, with why.
    fn write(self, raw: &lightcraft_raw::RawImage, packet: String, label: &str) -> Result<(Vec<u8>, Option<String>), String> {
        let compression = match self.compression {
            DngCompression::Lossless => lightcraft_raw::DngCompression::Lj92 { tile: 256 },
            DngCompression::Deflate => lightcraft_raw::DngCompression::Deflate { tile: 256, half: false },
            DngCompression::Uncompressed => lightcraft_raw::DngCompression::Uncompressed,
        };
        let plain = lightcraft_raw::write_dng(raw, &lightcraft_raw::DngWriteOptions { xmp: Some(packet.clone()), compression, ..Default::default() })
            .map_err(|e| e.to_string())?;
        if self.preview == DngPreview::None && !self.lossy {
            return Ok((plain, None));
        }
        if !lightcraft_dng_sdk_sys::available() {
            return Ok((plain, Some(format!("{label}: embedded preview / lossy DNG need Adobe's DNG SDK (not in this build): written without"))));
        }
        let medium = if self.preview == DngPreview::Medium { DNG_MEDIUM as u32 } else { 0 };
        let sdk_preview = match self.preview {
            DngPreview::None => 0,
            DngPreview::Medium => medium,
            DngPreview::Full => raw.width.max(raw.height) as u32,
        };
        let thumb = Self::rgb(self.thumb_job);
        let preview = Self::rgb(self.preview_job);
        fn img(x: &Option<(u32, u32, Vec<u8>)>) -> Option<lightcraft_dng_sdk_sys::Rgb8<'_>> {
            x.as_ref().map(|(w, h, rgb)| lightcraft_dng_sdk_sys::Rgb8 { width: *w, height: *h, rgb })
        }
        let finish = lightcraft_dng_sdk_sys::DngFinish {
            thumbnail: img(&thumb),
            preview: img(&preview),
            sdk_preview,
            lossy: self.lossy,
            lossy_size: self.lossy_size,
            uncompressed: self.compression == DngCompression::Uncompressed,
        };
        let done = lightcraft_dng_sdk_sys::finish_dng(&plain, &finish)
            .map_err(|e| e.to_string())
            .and_then(|d| lightcraft_tiff::set_ifd0_tag(&d, lightcraft_tiff::tags::XMP, 1, packet.as_bytes()).map_err(|e| e.to_string()));
        match done {
            Ok(d) => Ok((d, None)),
            Err(e) => Ok((plain, Some(format!("{label}: the DNG SDK couldn't add the preview / lossy data ({e}): written without")))),
        }
    }
}

/// How an exported DNG stores its raw data.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum DngCompression {
    /// Lossless JPEG (the usual DNG compression).
    #[default]
    Lossless,
    /// Deflate (zip) tiles with prediction.
    Deflate,
    Uncompressed,
}

/// The destination of a batch: a folder (with `ExportOptions::subfolder` and the conflict policy
/// applied to each file together with its sidecars), or one exact file path (single photo; an
/// ordinary file already there is replaced). Either way a catalogued original (or its sidecar)
/// is never written over: see [`crate::originals`].
#[derive(Clone, Debug, Default)]
pub struct Destination {
    pub dir: String,
    pub exact: Option<String>,
}

/// Export `ids` in order ([`prepare_batch`] + [`run_batch`]), stopping at the first error.
pub fn export_batch(
    session: &mut crate::Session,
    ids: &[lightcraft_catalog::PhotoId],
    o: &ExportOptions,
    to: &Destination,
    write: &mut dyn FnMut(&str, &[u8]) -> Result<(), String>,
    exists: &dyn Fn(&str) -> bool,
) -> Result<Vec<serde_json::Value>, String> {
    let items = prepare_batch(session, ids, o)?;
    run_batch(items, o, to, write, exists, true, &mut |_, _| true)
}

/// Write an exported or rendered file on disk: its folder is created if needed, and the file is
/// replaced atomically ([`lightcraft_catalog::safe_file::write_atomic_nosync`]: a temp file
/// renamed into place), so a failure part-way leaves any previous file intact and no truncated
/// one. Not synced to disk (issue #134): an export can always be made again from the original.
/// The native writer behind exports, renders and screenshots (app, CLI, MCP).
pub fn write_file(path: &str, bytes: &[u8]) -> Result<(), String> {
    write_with(path, bytes, false)
}

/// [`write_file`], synced to disk: for a written file that becomes a library photo (Edit a Copy)
/// and so can't simply be exported again once the catalog points at it.
pub fn write_file_durable(path: &str, bytes: &[u8]) -> Result<(), String> {
    write_with(path, bytes, true)
}

fn write_with(path: &str, bytes: &[u8], durable: bool) -> Result<(), String> {
    use lightcraft_catalog::safe_file::{write_atomic, write_atomic_nosync};
    let p = std::path::Path::new(path);
    if let Some(dir) = p.parent().filter(|d| !d.as_os_str().is_empty()) {
        std::fs::create_dir_all(dir).map_err(|e| format!("{}: {e}", dir.display()))?;
    }
    if durable { write_atomic(p, bytes) } else { write_atomic_nosync(p, bytes) }.map_err(|e| format!("{path}: {e}"))
}

/// The path of the sidecar with extension `ext` of the exported file `main`.
fn sidecar_path(main: &str, ext: &str) -> String {
    std::path::Path::new(main).with_extension(ext).to_string_lossy().to_string()
}

/// Run prepared exports in order: pick each one's path, and hand the bytes (and sidecars) to
/// `write`. `exists` tells whether a path is taken. The conflict policy applies to a file and its
/// sidecars as one: with Unique both get the same free name, with Skip the photo is skipped when
/// either is taken. A path that is a catalogued original (or its sidecar) is refused whatever the
/// policy. `progress(done, next file)` is called before each photo; returning false cancels the
/// rest. Returns one JSON object per photo: `{path, width, height, bytes, sidecars}`,
/// `{skipped: path}` or (unless `stop_on_error`) `{photo, file, error}`.
pub fn run_batch(
    items: Vec<PreparedExport>,
    o: &ExportOptions,
    to: &Destination,
    write: &mut dyn FnMut(&str, &[u8]) -> Result<(), String>,
    exists: &dyn Fn(&str) -> bool,
    stop_on_error: bool,
    progress: &mut dyn FnMut(usize, &str) -> bool,
) -> Result<Vec<serde_json::Value>, String> {
    use serde_json::json;
    let join = |a: &str, b: &str| if a.is_empty() { b.to_string() } else { format!("{}/{b}", a.trim_end_matches('/')) };
    let single = items.len() == 1;
    let mut taken = std::collections::HashSet::new();
    let mut out = Vec::new();
    for (i, item) in items.into_iter().enumerate() {
        if !progress(i, &item.file_name) {
            break;
        }
        let (photo, name, guard) = (item.photo, item.file_name.clone(), item.guard.clone());
        // the batch's folder, or (Same folder as original) this photo's own; + the subfolder
        let dir = target_dir(item.source_dir.as_deref(), o, to);
        // an answer to Ask what to do for this photo, else the batch's policy (Ask unanswered:
        // a new name, as nobody was there to ask)
        let conflict = match item.conflict.unwrap_or(o.conflict) {
            Conflict::Ask => Conflict::Unique,
            c => c,
        };
        let e = match item.run() {
            Ok(e) => e,
            Err(err) if stop_on_error => return Err(err),
            Err(err) => {
                out.push(json!({"photo": photo.0, "file": name, "error": err}));
                continue;
            }
        };
        // the exported file and its sidecars
        let group = |main: &str| std::iter::once(main.to_string()).chain(e.sidecars.iter().map(|(x, _)| sidecar_path(main, x))).collect::<Vec<_>>();
        let path = match to.exact.as_deref().filter(|_| single) {
            Some(p) => Ok(p.to_string()),
            None => {
                let path = join(&dir, &e.file_name);
                let busy = |main: &str| group(main).iter().any(|p| taken.contains(p) || exists(p));
                if !busy(&path) {
                    Ok(path)
                } else {
                    match conflict {
                        Conflict::Overwrite => Ok(path),
                        Conflict::Skip => {
                            out.push(json!({"skipped": path}));
                            continue;
                        }
                        Conflict::Unique | Conflict::Ask => {
                            let (stem, ext) = e.file_name.rsplit_once('.').map_or((e.file_name.as_str(), None), |(a, b)| (a, Some(b)));
                            let name = |n: usize| join(&dir, &ext.map_or(format!("{stem}-{n}"), |x| format!("{stem}-{n}.{x}")));
                            (2..1_000_000).map(name).find(|p| !busy(p)).ok_or_else(|| format!("{path}: no free file name"))
                        }
                    }
                }
            }
        };
        let file = path.clone().unwrap_or_else(|_| name.clone());
        let written = path.and_then(|path| {
            let files = group(&path);
            // never over an original, whatever the conflict policy or the exact path said
            for f in &files {
                guard.check(std::path::Path::new(f))?;
            }
            write(&path, &e.bytes)?;
            let mut sidecars = Vec::new();
            for ((_, bytes), sc) in e.sidecars.iter().zip(files.iter().skip(1)) {
                write(sc, bytes)?;
                sidecars.push(sc.clone());
            }
            Ok((path, files, sidecars))
        });
        match written {
            Ok((path, files, sidecars)) => {
                taken.extend(files);
                out.push(json!({"photo": photo.0, "path": path, "width": e.width, "height": e.height, "bytes": e.bytes.len(), "sidecars": sidecars}));
            }
            Err(err) if stop_on_error => return Err(err),
            Err(err) => out.push(json!({"photo": photo.0, "file": file, "error": err})),
        }
    }
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn test_image() -> Rgba8 {
        let mut img = Rgba8::new(96, 64);
        for (i, p) in img.data.iter_mut().enumerate() {
            let (x, y) = (i % 96, i / 96);
            *p = [(x * 2) as u8, (y * 3) as u8, ((x * y) % 255) as u8, 255];
        }
        img
    }

    #[test]
    fn formats_have_magic() {
        let img = test_image();
        for (f, magic) in
            [(ExportFormat::Jpeg, &b"\xFF\xD8"[..]), (ExportFormat::Png, b"\x89PNG"), (ExportFormat::Tiff, b"II"), (ExportFormat::Webp, b"RIFF")]
        {
            let b = encode_image(&img, &ExportOptions { format: f, ..Default::default() }).unwrap();
            assert!(b.starts_with(magic), "{f:?}");
        }
    }

    #[test]
    fn size_limit_respected() {
        let img = test_image();
        let full = encode_image(&img, &ExportOptions { quality: 100, ..Default::default() }).unwrap();
        let kb = (full.len() / 1024 / 2).max(2) as u32;
        let b = encode_image(&img, &ExportOptions { quality: 100, limit_kb: Some(kb), ..Default::default() }).unwrap();
        assert!(b.len() <= kb as usize * 1024);
    }

    #[test]
    fn sharpen_increases_edge_contrast() {
        let mut img = Rgba8::new(8, 8);
        for (i, p) in img.data.iter_mut().enumerate() {
            let v = if i % 8 < 4 { 80 } else { 170 };
            *p = [v, v, v, 255];
        }
        output_sharpen(&mut img, SharpenFor::Matte, SharpenAmount::High);
        assert!(img.data[3][0] < 80 && img.data[4][0] > 170);
    }

    #[test]
    fn metadata_policies() {
        use lightcraft_catalog::{Photo, PhotoId, Source};
        let mut p = Photo::new(PhotoId(1), Source::Demo { scene: 0 }, "a.jpg", "jpeg", 10, 10, "2026-09-30T00:00:00");
        p.meta.camera = "Synthetic X2".into();
        p.meta.copyright = "(c) Me".into();
        p.meta.copyright_status = lightcraft_catalog::CopyrightStatus::Copyrighted;
        p.meta.usage_terms = "Editorial use only".into();
        p.meta.copyright_url = "https://example.com/rights".into();
        p.meta.shutter = "1/250".into();
        p.meta.gps = Some((43.0, -110.0));
        let all = export_metadata(&p, &ExportOptions::default()).unwrap();
        assert_eq!((all.make.as_deref(), all.model.as_deref()), (Some("Synthetic"), Some("X2")), "Exif Make and Model apart");
        assert!((all.exposure_time.unwrap() - 0.004).abs() < 1e-9);
        assert!(all.gps.is_some());
        let o = ExportOptions { metadata: MetadataPolicy::AllExceptCamera, remove_location: true, ..Default::default() };
        let m = export_metadata(&p, &o).unwrap();
        assert!(m.model.is_none() && m.gps.is_none() && m.copyright.is_some());
        let c = export_metadata(&p, &ExportOptions { metadata: MetadataPolicy::Copyright, ..Default::default() }).unwrap();
        assert!(c.model.is_none() && c.gps.is_none() && c.copyright.as_deref() == Some("(c) Me"));
        // "copyright only" keeps all the copyright info: status, usage terms, info URL
        assert_eq!(
            (c.copyright_marked, c.usage_terms.as_deref(), c.copyright_url.as_deref()),
            (Some(true), Some("Editorial use only"), Some("https://example.com/rights"))
        );
        assert!(export_metadata(&p, &ExportOptions { metadata: MetadataPolicy::None, ..Default::default() }).is_none());
        // embedded and readable back from the JPEG
        let jpg = encode_with_metadata(&test_image(), &ExportOptions::default(), Some(&all)).unwrap();
        let back = lightcraft_meta::extract(&jpg);
        assert_eq!((back.make.as_deref(), back.model.as_deref()), (Some("Synthetic"), Some("X2")));
        assert_eq!(back.copyright.as_deref(), Some("(c) Me"));
        assert_eq!((back.copyright_marked, back.usage_terms.as_deref()), (Some(true), Some("Editorial use only")));
        assert_eq!(back.copyright_url.as_deref(), Some("https://example.com/rights"));
        assert!(back.gps.is_some());
    }

    /// The library keeps "make model" in one string; exports write Exif Make and Model apart, as
    /// the camera (and Lightroom's exports) did.
    #[test]
    fn camera_names_split_into_make_and_model() {
        for (camera, make, model) in [
            ("Canon Canon EOS R6", Some("Canon"), Some("Canon EOS R6")),
            ("LEICA CAMERA AG LEICA Q2", Some("LEICA CAMERA AG"), Some("LEICA Q2")),
            ("Leica Camera AG LEICA M (Typ 262)", Some("Leica Camera AG"), Some("LEICA M (Typ 262)")),
            ("RICOH IMAGING COMPANY, LTD. RICOH GR III", Some("RICOH IMAGING COMPANY, LTD."), Some("RICOH GR III")),
            ("NIKON CORPORATION NIKON D850", Some("NIKON CORPORATION"), Some("NIKON D850")),
            ("FUJIFILM X-T2", Some("FUJIFILM"), Some("X-T2")),
            ("Apple iPhone 13 Pro Max", Some("Apple"), Some("iPhone 13 Pro Max")),
            ("OLYMPUS IMAGING CORP. E-M1", Some("OLYMPUS IMAGING CORP."), Some("E-M1")),
            ("X100F", None, Some("X100F")),
            ("  ", None, None),
            ("Ünïcode  Café ✓", Some("Ünïcode"), Some("Café ✓")),
        ] {
            assert_eq!(split_camera(camera), (make.map(str::to_string), model.map(str::to_string)), "{camera}");
        }
    }

    /// Exports name their colour space in Exif like Lightroom's: 1 = sRGB, 0xFFFF otherwise.
    #[test]
    fn exif_color_space_follows_the_export_space() {
        let p = lightcraft_catalog::Photo::new(
            lightcraft_catalog::PhotoId(1),
            lightcraft_catalog::Source::Demo { scene: 0 },
            "a.jpg",
            "jpeg",
            10,
            10,
            "2026-09-30T00:00:00",
        );
        let meta = export_metadata(&p, &ExportOptions::default()).unwrap();
        for (space, want, desc) in [
            (OutputSpace::Srgb, 1u16, "sRGB IEC61966-2.1"),
            (OutputSpace::AdobeRgb, 0xFFFF, "Adobe RGB (1998)"),
            (OutputSpace::DisplayP3, 0xFFFF, "Display P3"),
            (OutputSpace::ProPhoto, 0xFFFF, "ProPhoto RGB"),
        ] {
            let o = ExportOptions { color_space: space, ..Default::default() };
            let jpg = encode_with_metadata(&test_image(), &o, Some(&meta)).unwrap();
            let entry = [0x01u8, 0xA0, 3, 0, 1, 0, 0, 0];
            let at = jpg.windows(8).position(|w| w == entry).expect("ColorSpace tag");
            assert_eq!(u16::from_le_bytes([jpg[at + 8], jpg[at + 9]]), want, "{space:?}");
            // the profile Lightroom would embed (v2 for sRGB / Adobe RGB)
            let icc = jpg.windows(12).position(|w| w == b"ICC_PROFILE\0").expect("ICC");
            let v2 = matches!(space, OutputSpace::Srgb | OutputSpace::AdobeRgb);
            assert_eq!(jpg[icc + 14 + 8], if v2 { 2 } else { 4 }, "{space:?} version");
            let profile = lightcraft_codecs::icc::write_named(named_space(space));
            assert_eq!(lightcraft_codecs::icc::parse(&profile).and_then(|i| i.description).as_deref(), Some(desc));
            assert!(!v2 || jpg.windows(desc.len()).any(|w| w == desc.as_bytes()), "{space:?}: v2 ASCII description");
        }
    }

    #[test]
    fn watermark_draws_in_the_anchored_corner_only() {
        let mut img = Rgba8::new(400, 300);
        for p in img.data.iter_mut() {
            *p = [0, 0, 0, 255];
        }
        let o = ExportOptions::from_json(&serde_json::json!({"watermark": {"text": "LightCraft", "size": 0.08, "opacity": 1.0, "shadow": false}}));
        let wm = o.watermark.clone().unwrap();
        draw_watermark(&mut img, &wm);
        let lit = |x0: usize, x1: usize, y0: usize, y1: usize| {
            (y0..y1).flat_map(|y| (x0..x1).map(move |x| (x, y))).filter(|&(x, y)| img.get(x, y)[0] > 128).count()
        };
        assert!(lit(200, 400, 225, 300) > 100, "bottom-right has text");
        assert!(lit(0, 200, 0, 150) == 0, "top-left untouched");
        // string shorthand
        assert_eq!(ExportOptions::from_json(&serde_json::json!({"watermark": "© Me"})).watermark.unwrap().text, "© Me");
        assert!(ExportOptions::from_json(&serde_json::json!({"watermark": ""})).watermark.is_none());
    }

    fn vertical_japanese() -> Watermark {
        let options =
            ExportOptions::from_json(&json!({"watermark": {"text": "日本語", "vertical": true, "size": 0.1, "shadow": false, "opacity": 1.0}}));
        options.watermark.unwrap()
    }

    #[test]
    fn japanese_watermark_options_and_legacy_defaults() {
        let old = ExportOptions::from_json(&json!({"watermark": {"text": "日本語"}}));
        assert!(!old.watermark.unwrap().vertical);
        let wm = vertical_japanese();
        assert!(wm.vertical);
        let round: Watermark = serde_json::from_value(serde_json::to_value(&wm).unwrap()).unwrap();
        assert!(round.vertical);
    }

    /// Built without craft-fonts, a Japanese watermark still lays out and draws (Inter's
    /// missing-glyph boxes), and Latin text is unaffected.
    #[test]
    fn watermarks_work_without_craft_fonts() {
        assert_eq!(watermark_fonts(&[]).len(), 1, "Inter only");
        for wm in [vertical_japanese(), Watermark { text: "LightCraft 日本語".into(), ..Watermark::default() }] {
            let mut covered = 0usize;
            watermark_coverage(400, 300, &wm, &[], |_, _, k, _| covered += usize::from(k > 0.0));
            assert!(covered > 0, "{:?} draws something", wm.text);
        }
    }

    #[test]
    fn biz_ud_vertical_watermarks_place_punctuation_at_the_top_right() {
        use ab_glyph::Font;
        let Some(entry) = crate::fonts::CRAFT_FONTS.iter().find(|f| f.family == "BIZ UDMincho") else {
            eprintln!("skipped: built without BIZ UDMincho from craft-fonts");
            return;
        };
        let font = ab_glyph::FontRef::try_from_slice(entry.bytes).unwrap();
        // BIZ UDMincho has no Unicode presentation-form characters: the old fallback drew 、。.
        assert_eq!(font.glyph_id('︑').0, 0);
        assert_eq!(font.glyph_id('︒').0, 0);
        let data = harfrust::ShaperData::new(&harfrust::FontRef::new(entry.bytes).unwrap());
        for ch in ['、', '。'] {
            let glyph = vertical_watermark_glyph(&font, &data, ch, 100.0, 0.0, 0.0).unwrap();
            assert_ne!(glyph.id, font.glyph_id(ch), "the font's vertical alternate for {ch}");
            let rect = font.outline_glyph(glyph).unwrap().px_bounds();
            assert!(rect.min.x > 50.0 && rect.max.y < 50.0, "{ch} in the upper right: {rect:?}");
        }
        let craft = Box::leak(
            vec![crate::fonts::CraftFont { family: entry.family, style: entry.style, scripts: entry.scripts, bytes: entry.bytes }].into_boxed_slice(),
        );
        let wm = Watermark {
            text: "、。".into(),
            vertical: true,
            size: 0.1,
            anchor: Anchor::TopLeft,
            inset: 0.0,
            shadow: false,
            opacity: 1.0,
            ..Default::default()
        };
        let mut pixels = Vec::new();
        watermark_coverage(400, 300, &wm, craft, |x, y, k, _| {
            if k > 0.5 {
                pixels.push((x, y));
            }
        });
        assert!(!pixels.is_empty());
        for (x, y) in pixels {
            assert!(x > 15 && y % 30 < 15, "actual export coverage at the upper right of the 30 px cell: {x}, {y}");
        }
    }

    #[test]
    fn japanese_watermarks_support_vertical_columns() {
        use ab_glyph::Font;
        let fonts = watermark_fonts(crate::fonts::CRAFT_FONTS);
        if fonts.len() < 2 {
            eprintln!("skipped: built without CRAFT_FONTS_DIR, so there is no Japanese watermark face");
            return;
        }
        for c in "日本語の文字".chars() {
            assert!(fonts[1..].iter().any(|f| f.glyph_id(c).0 != 0), "a craft-fonts face has {c}");
        }
        let wm = vertical_japanese();
        let mut img = Rgba8::new(400, 300);
        img.data.fill([0, 0, 0, 255]);
        draw_watermark(&mut img, &wm);
        let lit: Vec<_> = (0..300).flat_map(|y| (0..400).map(move |x| (x, y))).filter(|&(x, y)| img.get(x, y)[0] > 64).collect();
        assert!(lit.len() > 100);
        let w = lit.iter().map(|p| p.0).max().unwrap() - lit.iter().map(|p| p.0).min().unwrap();
        let h = lit.iter().map(|p| p.1).max().unwrap() - lit.iter().map(|p| p.1).min().unwrap();
        assert!(h > w * 2, "vertical Japanese must extend down the column");
        let two = Watermark { text: "日\n本".into(), anchor: Anchor::TopLeft, inset: 0.0, ..wm };
        img.data.fill([0, 0, 0, 255]);
        draw_watermark(&mut img, &two);
        for (text, dx) in [("本", 0), ("日", 30)] {
            let mut single = Rgba8::new(400, 300);
            single.data.fill([0, 0, 0, 255]);
            draw_watermark(&mut single, &Watermark { text: text.into(), ..two.clone() });
            for y in 0..30 {
                for x in 0..30 {
                    assert_eq!(img.get(x + dx, y), single.get(x, y), "newlines move columns left");
                }
            }
        }
    }

    #[test]
    fn graphic_watermark_is_placed_scaled_and_blended() {
        let dir = std::env::temp_dir().join(format!("lc-wm-logo-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let logo_path = dir.join("logo.png");
        // 40×20: left half opaque red, right half fully transparent
        let logo = Rgba8::from_fn(40, 20, |x, _| if x < 20 { [255, 0, 0, 255] } else { [0, 255, 0, 0] });
        let png = lightcraft_codecs::encode_png(&EncodeImage::rgba8(&logo), &EncodeMeta::default()).unwrap();
        std::fs::write(&logo_path, png).unwrap();
        let mut img = Rgba8::new(400, 300);
        img.data.iter_mut().for_each(|p| *p = [0, 0, 0, 255]);
        let wm = Watermark { image: logo_path.to_string_lossy().into(), image_width: 0.25, opacity: 1.0, inset: 0.0, ..Default::default() };
        draw_watermark(&mut img, &wm);
        // 100 px wide × 50 px high in the bottom-right corner
        assert!(img.get(310, 280)[0] > 200, "opaque half: {:?}", img.get(310, 280));
        assert_eq!(img.get(390, 280), [0, 0, 0, 255], "transparent half leaves the photo");
        assert_eq!(img.get(310, 240), [0, 0, 0, 255], "above the logo");
        assert_eq!(img.get(10, 10), [0, 0, 0, 255]);
        // an image-only watermark survives the params (no text needed)
        let o = ExportOptions::from_json(&json!({"watermark": {"image": logo_path.to_string_lossy(), "imageWidth": 0.1}}));
        assert_eq!(o.watermark.as_ref().map(|w| w.image_width), Some(0.1));
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn naming_and_params() {
        let o = ExportOptions::from_json(&serde_json::json!({"format": "jpg", "quality": 150, "longEdge": 2048, "naming": "{name}-{seq}"}));
        assert_eq!(o.format, ExportFormat::Jpeg);
        assert_eq!(o.quality, 100);
        assert_eq!(o.resize, Some(Resize { dont_enlarge: true, height: 0.0, ..Resize::long_edge(2048) }));
        assert_eq!(o.file_name_for(&named("IMG/1.png"), 7), "IMG_1-007.jpg");
        assert_eq!(o.ppi, 240);
        let orig = ExportOptions { format: ExportFormat::Original, naming: "{name}-{seq}".into(), ..Default::default() };
        assert_eq!(orig.file_name_for(&named("DSC_1.NEF"), 2), "DSC_1-002.NEF");
        assert_eq!(orig.file_name_for(&named("noext"), 1), "noext-001");
        assert_eq!(ExportOptions { format: ExportFormat::Dng, ..Default::default() }.file_name_for(&named("a.cr2"), 1), "a.dng");
    }

    fn named(file_name: &str) -> lightcraft_catalog::Photo {
        use lightcraft_catalog::{Photo, PhotoId, Source};
        Photo::new(PhotoId(1), Source::Demo { scene: 0 }, file_name, "", 1, 1, "2026-01-01T00:00:00")
    }

    /// Export naming takes the same tokens as Rename Photos and import renaming.
    #[test]
    fn naming_uses_the_rename_tokens() {
        let mut p = named("DSC_0815.NEF");
        p.captured = Some("2026-09-30T14:05:09".into());
        p.meta.camera = "Model X/2".into();
        p.meta.title = "Harbour".into();
        p.rating = 5;
        let o = |naming: &str| ExportOptions { naming: naming.into(), start_number: 9, ..Default::default() };
        assert_eq!(o("{date:%Y-%m-%d}_{title}_{seq:2}").file_name_for(&p, 1), "2026-09-30_Harbour_09.jpg");
        assert_eq!(o("{camera}-{num}-{rating}").file_name_for(&p, 1), "Model X_2-0815-5.jpg", "path separators become _");
        assert_eq!(o("{name}-{seq}").file_name_for(&p, 3), "DSC_0815-011.jpg", "a bare {{seq}} keeps its 3 digits");
        assert_eq!(o("{ext}_{name}").file_name_for(&p, 1), "NEF_DSC_0815.jpg");
        // nothing left after expanding: the original name
        p.meta.title.clear();
        assert_eq!(o("{title}").file_name_for(&p, 1), "DSC_0815.jpg");
    }

    #[test]
    fn params_round_trip() {
        let o = ExportOptions {
            format: ExportFormat::Tiff,
            quality: 70,
            resize: Some(Resize { mode: ResizeMode::Megapixels, value: 12.5, height: 0.0, dont_enlarge: false, unit: SizeUnit::Pixels }),
            ppi: 300,
            limit_kb: Some(500),
            sharpen: SharpenFor::Glossy,
            naming: "{date}-{name}-{seq}".into(),
            start_number: 42,
            subfolder: "Client".into(),
            conflict: Conflict::Skip,
            tiff_compression: TiffCompression::Lzw,
            metadata: MetadataPolicy::Copyright,
            remove_location: true,
            watermark: Some(Watermark { text: "© LC".into(), ..Default::default() }),
            color_space: OutputSpace::ProPhoto,
            bit_depth: Some(16),
            ..Default::default()
        };
        assert_eq!(ExportOptions::from_json(&o.to_json()), o);
        let full = ExportOptions::default();
        assert_eq!(ExportOptions::from_json(&full.to_json()), full);
        assert!(ExportOptions::has_size_param(&full.to_json()), "full size is explicit");
        let mut p = named("IMG.png");
        p.captured = Some("2026-09-30T10:00:00".into());
        assert_eq!(o.file_name_for(&p, 2), "20260930-IMG-043.tif");
    }

    #[test]
    fn resize_modes() {
        let size = |p: serde_json::Value, w: f64, h: f64| ExportOptions::from_json(&p).resize.map(|r| r.apply(w, h));
        // a 6000 × 4000 landscape and a 4000 × 6000 portrait
        assert_eq!(size(json!({"longEdge": 1500}), 6000.0, 4000.0), Some((1500, 1000)));
        assert_eq!(size(json!({"longEdge": 1500}), 4000.0, 6000.0), Some((1000, 1500)));
        assert_eq!(size(json!({"shortEdge": 1080}), 6000.0, 4000.0), Some((1620, 1080)));
        assert_eq!(size(json!({"width": 3000}), 4000.0, 6000.0), Some((3000, 4500)));
        assert_eq!(size(json!({"height": 1000}), 6000.0, 4000.0), Some((1500, 1000)));
        // W × H fits either orientation: the long edge gets the larger number
        assert_eq!(size(json!({"width": 1000, "height": 800}), 6000.0, 4000.0), Some((1000, 667)));
        assert_eq!(size(json!({"width": 1000, "height": 800}), 4000.0, 6000.0), Some((667, 1000)));
        assert_eq!(size(json!({"width": 1000, "height": 500}), 6000.0, 4000.0), Some((750, 500)));
        let (w, h) = size(json!({"megapixels": 6}), 6000.0, 4000.0).unwrap();
        assert!(((w * h) as f64 - 6e6).abs() < 6e6 * 0.002, "{w}×{h}");
        assert_eq!(size(json!({"percent": 25}), 6000.0, 4000.0), Some((1500, 1000)));
        // don't enlarge (default) vs enlarge
        assert_eq!(size(json!({"longEdge": 8000}), 6000.0, 4000.0), Some((6000, 4000)));
        assert_eq!(size(json!({"longEdge": 9000, "dontEnlarge": false}), 6000.0, 4000.0), Some((9000, 6000)));
        // full size / absent
        assert_eq!(size(json!({"longEdge": 0}), 6000.0, 4000.0), None);
        assert_eq!(size(json!({}), 6000.0, 4000.0), None);
        assert!(ExportOptions::has_size_param(&json!({"longEdge": 0})) && !ExportOptions::has_size_param(&json!({"quality": 3})));
        // the `resize` object form round-trips (what the dialog and prefs store)
        let r = Resize { mode: ResizeMode::ShortEdge, value: 720.0, height: 0.0, dont_enlarge: false, unit: SizeUnit::Pixels };
        let back = ExportOptions::from_json(&ExportOptions::resize_json(Some(&r))).resize;
        assert_eq!(back, Some(r));
        assert_eq!(ExportOptions::from_json(&ExportOptions::resize_json(None)).resize, None);
        assert_eq!(ExportOptions::from_json(&json!({"ppi": 300})).ppi, 300);
        // never larger than the encoders' limit, never empty
        assert_eq!(size(json!({"percent": 10000, "dontEnlarge": false}), 6000.0, 4000.0), Some((65535, 43690)));
        assert_eq!(size(json!({"width": 1}), 6000.0, 40.0).map(|s| s.1), Some(1));
    }

    /// A flat field of a saturated green inside Display P3 but outside sRGB, rendered into `space`.
    fn p3_green(space: OutputSpace) -> Rgba8 {
        use lightcraft_color::{DISPLAY_P3, REC2020};
        let c = DISPLAY_P3.to_space(&REC2020).apply_f32([0.04, 0.45, 0.04]);
        let src = lightcraft_raster::Rgb32f::filled(16, 16, c);
        let req = lightcraft_pipeline::RenderRequest { space, ..lightcraft_pipeline::RenderRequest::fit(16, 16) };
        lightcraft_pipeline::render(&src, &Default::default(), &Default::default(), &req).image
    }

    /// Decode `bytes` and return the centre pixel in linear sRGB primaries (unclamped), plus the
    /// recognised space of the embedded profile.
    fn decoded_in_srgb(bytes: &[u8]) -> ([f32; 3], Option<NamedSpace>) {
        let d = lightcraft_codecs::decode(bytes, Default::default()).expect("decodes");
        let m = d.space.to_space(&lightcraft_color::SRGB);
        (m.apply_f32(d.image.get(8, 8)), d.space.named)
    }

    #[test]
    fn p3_colour_survives_a_p3_export_and_is_clipped_in_srgb() {
        let o = |space| ExportOptions { format: ExportFormat::Png, color_space: space, ..Default::default() };
        let (p3, named) = decoded_in_srgb(&encode_image(&p3_green(OutputSpace::DisplayP3), &o(OutputSpace::DisplayP3)).unwrap());
        assert_eq!(named, Some(NamedSpace::DisplayP3));
        assert!(p3[0] < -0.03 || p3[2] < -0.03, "outside sRGB after a P3 round trip: {p3:?}");
        let (s, named) = decoded_in_srgb(&encode_image(&p3_green(OutputSpace::Srgb), &o(OutputSpace::Srgb)).unwrap());
        assert_eq!(named, Some(NamedSpace::Srgb));
        assert!(s.iter().all(|v| *v > -0.002), "{s:?}");
        // the green itself is about the same brightness either way
        assert!((p3[1] - s[1]).abs() < 0.15, "{p3:?} {s:?}");
    }

    #[test]
    fn every_space_embeds_its_own_profile_and_round_trips() {
        let grey = {
            let src = lightcraft_raster::Rgb32f::filled(16, 16, [0.18; 3]);
            let base = lightcraft_pipeline::RenderRequest::fit(16, 16);
            move |space| {
                lightcraft_pipeline::render(&src, &Default::default(), &Default::default(), &lightcraft_pipeline::RenderRequest { space, ..base })
                    .image
            }
        };
        let reference =
            decoded_in_srgb(&encode_image(&grey(OutputSpace::Srgb), &ExportOptions { format: ExportFormat::Tiff, ..Default::default() }).unwrap()).0;
        for space in OutputSpace::ALL {
            for format in [ExportFormat::Jpeg, ExportFormat::Png, ExportFormat::Tiff, ExportFormat::Webp] {
                let o = ExportOptions { format, color_space: space, ..Default::default() };
                let bytes = encode_image(&grey(space), &o).unwrap();
                let (px, named) = decoded_in_srgb(&bytes);
                assert_eq!(named, Some(named_space(space)), "{space:?} {format:?}");
                // a neutral grey decodes to the same linear value whatever the output space
                for c in 0..3 {
                    assert!((px[c] - reference[c]).abs() < 0.01, "{space:?} {format:?}: {px:?} vs {reference:?}");
                }
            }
            let icc = icc::write_named(named_space(space));
            let info = icc::parse(&icc).expect("our profile parses");
            assert_eq!(info.named, Some(named_space(space)));
            assert!(info.description.as_deref().is_some_and(|d| !d.is_empty()));
        }
        let o = ExportOptions { format: ExportFormat::Avif, color_space: OutputSpace::ProPhoto, ..Default::default() };
        assert_eq!(o.effective_space(), OutputSpace::Srgb);
        let o = ExportOptions::from_json(&serde_json::json!({"colorSpace": "displayP3"}));
        assert_eq!(o.color_space, OutputSpace::DisplayP3);
        assert_eq!(ExportOptions::from_json(&serde_json::json!({"colorSpace": "Adobe RGB (1998) compatible"})).color_space, OutputSpace::AdobeRgb);
    }

    #[test]
    fn srgb_watermark_colour_is_converted() {
        assert_eq!(srgb8_in(OutputSpace::Srgb, [10, 200, 30]), [10, 200, 30]);
        for s in OutputSpace::ALL {
            assert_eq!(srgb8_in(s, [255, 255, 255]), [255, 255, 255], "{s:?}");
            assert_eq!(srgb8_in(s, [0, 0, 0]), [0, 0, 0], "{s:?}");
        }
        let p3 = srgb8_in(OutputSpace::DisplayP3, [255, 0, 0]);
        assert!(p3[0] < 255 && p3[1] > 0, "{p3:?}");
    }

    /// A shallow horizontal grey ramp (few 8-bit levels), rendered at `depth`.
    fn ramp(depth: OutputDepth, space: OutputSpace) -> lightcraft_pipeline::Rendered {
        let src = lightcraft_raster::Rgb32f::from_fn(1024, 4, |x, _| [0.10 + 0.03 * x as f32 / 1023.0; 3]);
        let req = lightcraft_pipeline::RenderRequest { depth, space, ..lightcraft_pipeline::RenderRequest::fit(1024, 4) };
        lightcraft_pipeline::render(&src, &Default::default(), &Default::default(), &req)
    }

    fn distinct_levels(bytes: &[u8]) -> (usize, lightcraft_codecs::Decoded) {
        let d = lightcraft_codecs::decode(bytes, Default::default()).expect("decodes");
        let mut v: Vec<u32> = (0..d.image.width).map(|x| d.image.get(x, 1)[1].to_bits()).collect();
        v.dedup();
        (v.len(), d)
    }

    #[test]
    fn sixteen_bit_tiff_and_png_have_no_banding() {
        let tiff = |bit_depth| ExportOptions { format: ExportFormat::Tiff, bit_depth, ..Default::default() };
        assert_eq!(tiff(None).effective_depth(), OutputDepth::U16, "TIFF defaults to 16-bit");
        let r16 = ramp(OutputDepth::U16, OutputSpace::Srgb);
        assert!(matches!(r16.deep.as_ref().unwrap().samples, DeepSamples::U16(_)));
        let (levels16, d16) = distinct_levels(&encode_rendered(&r16, &tiff(None), None).unwrap());
        let r8 = ramp(OutputDepth::U8, OutputSpace::Srgb);
        let (levels8, _) = distinct_levels(&encode_rendered(&r8, &tiff(Some(8)), None).unwrap());
        eprintln!("gradient levels: 16-bit TIFF {levels16}, 8-bit TIFF {levels8}");
        assert!(levels8 < 40, "{levels8}");
        assert!(levels16 > 10 * levels8, "16-bit: {levels16} levels vs 8-bit: {levels8}");
        assert_eq!(d16.space.named, Some(NamedSpace::Srgb));
        // the 8-bit preview of a deep render matches the 8-bit render
        for (a, b) in r16.image.data.iter().zip(&r8.image.data) {
            assert!(a[1].abs_diff(b[1]) <= 1, "{a:?} {b:?}");
        }
        let png = ExportOptions { format: ExportFormat::Png, bit_depth: Some(16), color_space: OutputSpace::DisplayP3, ..Default::default() };
        let rp = ramp(OutputDepth::U16, OutputSpace::DisplayP3);
        let bytes = encode_rendered(&rp, &png, None).unwrap();
        assert_eq!(bytes[24], 16, "PNG IHDR bit depth");
        let (levels, d) = distinct_levels(&bytes);
        assert!(levels > 10 * levels8, "{levels}");
        assert_eq!(d.space.named, Some(NamedSpace::DisplayP3));
        // sharpening and watermarking work on deep images too
        let o = ExportOptions {
            sharpen: SharpenFor::Matte,
            watermark: Some(Watermark { text: "LC".into(), size: 0.5, ..Default::default() }),
            ..tiff(None)
        };
        assert!(encode_rendered(&r16, &o, None).is_ok());
    }

    /// An edit with tone curves, Refine Saturation and grain exported in every colour space decodes
    /// (through each file's own embedded profile) to the same colours wherever the space can hold
    /// them: the curves and grain act in one space whatever the output (`finish::STYLE_SPACE`, as
    /// in Lightroom). Before 2026-11-21 they acted on the output primaries, and an Adobe RGB or P3
    /// export of a curved photo differed from its sRGB export by up to ~2 ΔE.
    #[test]
    fn exports_in_every_space_agree_in_colour_with_curves_and_grain() {
        use lightcraft_geom::Point;
        let src = lightcraft_raster::Rgb32f::from_fn(96, 64, |x, y| {
            let (u, v) = (x as f32 / 95.0, y as f32 / 63.0);
            // moderately saturated colours all around the hue circle, dark to bright
            let h = u * std::f32::consts::TAU;
            let l = 0.03 + 0.5 * v;
            [l * (1.0 + 0.5 * h.cos()), l * (1.0 + 0.5 * (h - 2.1).cos()), l * (1.0 + 0.5 * (h + 2.1).cos())]
        });
        let mut s = lightcraft_develop::DevelopSettings::default();
        s.curve.master = vec![Point::new(0.0, 0.0), Point::new(0.3, 0.2), Point::new(0.7, 0.82), Point::new(1.0, 1.0)];
        s.curve.red = vec![Point::new(0.0, 0.04), Point::new(0.5, 0.56), Point::new(1.0, 1.0)];
        s.curve.refine_saturation = 60.0;
        s.grain.amount = 25.0;
        let info = lightcraft_pipeline::SourceInfo { raw: true, ..Default::default() };
        let export = |space| {
            let req = lightcraft_pipeline::RenderRequest { space, depth: OutputDepth::U16, ..lightcraft_pipeline::RenderRequest::fit(96, 64) };
            let r = lightcraft_pipeline::render(&src, &info, &s, &req);
            let o = ExportOptions { format: ExportFormat::Tiff, color_space: space, bit_depth: Some(16), ..Default::default() };
            let d = lightcraft_codecs::decode(&encode_rendered(&r, &o, None).unwrap(), Default::default()).expect("decodes");
            assert_eq!(d.space.named, Some(named_space(space)), "{space:?}");
            let m = d.space.to_space(&lightcraft_color::SRGB);
            (0..64).flat_map(|y| (0..96).map(move |x| (x, y))).map(|(x, y)| m.apply_f32(d.image.get(x, y))).collect::<Vec<[f32; 3]>>()
        };
        let reference = export(OutputSpace::Srgb);
        for space in [OutputSpace::DisplayP3, OutputSpace::AdobeRgb, OutputSpace::ProPhoto, OutputSpace::Rec2020] {
            let px = export(space);
            let mut worst = 0.0f32;
            for (a, b) in reference.iter().zip(&px) {
                // only where the sRGB export is not clipped
                if a.iter().all(|v| (0.003..0.997).contains(v)) {
                    worst = a.iter().zip(b).map(|(a, b)| (a - b).abs()).fold(worst, f32::max);
                }
            }
            assert!(worst < 0.004, "{space:?}: linear sRGB differs by up to {worst} from the sRGB export");
        }
    }

    #[test]
    fn float_tiff_is_linear_with_a_linear_profile() {
        let o = ExportOptions { format: ExportFormat::Tiff, bit_depth: Some(32), color_space: OutputSpace::ProPhoto, ..Default::default() };
        assert_eq!(o.effective_depth(), OutputDepth::F32Linear);
        let rf = ramp(OutputDepth::F32Linear, OutputSpace::ProPhoto);
        let (levels, df) = distinct_levels(&encode_rendered(&rf, &o, None).unwrap());
        assert!(levels > 500, "{levels}");
        assert_eq!(df.space.named, Some(NamedSpace::ProPhoto));
        assert!(df.space.trc.as_ref().is_some_and(|t| t[0].is_linear()), "{:?}", df.space.trc);
        // decodes to the same light as the 16-bit gamma-encoded export
        let o16 = ExportOptions { bit_depth: Some(16), ..o.clone() };
        let (_, d16) = distinct_levels(&encode_rendered(&ramp(OutputDepth::U16, OutputSpace::ProPhoto), &o16, None).unwrap());
        for x in [0, 300, 700, 1023] {
            let (a, b) = (df.image.get(x, 1)[1], d16.image.get(x, 1)[1]);
            assert!((a - b).abs() < 2e-4, "{x}: {a} vs {b}");
        }
    }

    #[test]
    fn bit_depth_options_per_format() {
        let d = |format, bit_depth| ExportOptions { format, bit_depth, ..Default::default() }.effective_depth();
        assert_eq!(d(ExportFormat::Jpeg, Some(16)), OutputDepth::U8);
        assert_eq!(d(ExportFormat::Webp, None), OutputDepth::U8);
        assert_eq!(d(ExportFormat::Png, None), OutputDepth::U8);
        assert_eq!(d(ExportFormat::Png, Some(16)), OutputDepth::U16);
        assert_eq!(d(ExportFormat::Avif, Some(10)), OutputDepth::U16);
        assert_eq!(ExportOptions::from_json(&serde_json::json!({"bitDepth": 16})).bit_depth, Some(16));
        assert_eq!(ExportOptions::from_json(&serde_json::json!({"bitDepth": 12})).bit_depth, None);
        for f in [ExportFormat::Jpeg, ExportFormat::Png, ExportFormat::Tiff, ExportFormat::Webp, ExportFormat::Avif] {
            let first = ExportOptions::bit_depths(f)[0].0;
            let def = d(f, None);
            assert_eq!(d(f, Some(first)), def, "{f:?}: the first choice is the default");
        }
    }

    #[cfg(not(target_arch = "wasm32"))]
    #[test]
    fn avif_ten_bit() {
        let o = ExportOptions { format: ExportFormat::Avif, bit_depth: Some(10), quality: 60, ..Default::default() };
        let r = ramp(OutputDepth::U16, OutputSpace::Srgb);
        match encode_rendered(&r, &o, None) {
            Ok(b) => assert_eq!(&b[4..8], b"ftyp"),
            Err(e) => assert!(e.contains("not available"), "{e}"),
        }
    }

    /// Issue #181: a command about to export refuses what `from_json` would silently default —
    /// unknown keys (naming the nearest known one) and values of the wrong kind or range.
    #[test]
    fn export_params_are_validated() {
        use serde_json::json;
        let err = |p: serde_json::Value| ExportOptions::from_params(&p).err().map(|e| e.to_string()).unwrap_or_default();
        let e = err(json!({"longEdgee": 400}));
        assert!(e.starts_with("invalid parameters for `app.export`: unknown parameter `longEdgee` (did you mean `longEdge`?)"), "{e}");
        assert!(err(json!({"qualty": 5})).contains("did you mean `quality`"));
        assert!(err(json!({"longEdge": "banana"})).contains("`longEdge` must be a number"));
        assert!(err(json!({"quality": 0})).contains("`quality` must be an integer 1..100"));
        assert!(err(json!({"quality": "5"})).contains("`quality` must be an integer"));
        assert!(err(json!({"quality": 92.5})).contains("`quality` must be an integer"));
        assert!(err(json!({"sharpen": "lots"})).contains("`sharpen` must be one of none|screen|matte|glossy"));
        assert!(err(json!({"format": "bmp"})).contains("`format` must be one of"));
        assert!(err(json!({"bitDepth": 12})).contains("`bitDepth` must be 8, 10, 16 or 32"));
        assert!(err(json!({"colorSpace": "cmyk"})).contains("`colorSpace` must be one of"));
        assert!(err(json!({"dontEnlarge": "no"})).contains("`dontEnlarge` must be true or false"));
        assert!(err(json!({"resize": {"mode": "longEdge", "valeu": 10}})).contains("did you mean `value`"));
        assert!(err(json!({"resize": {"mode": "diagonal", "value": 10}})).contains("`resize`:"));
        assert!(err(json!({"ids": [1, "x"]})).contains("`ids` must be an array of photo ids"));
        assert!(err(json!("jpeg")).contains("params must be an object"));
        assert!(
            ExportOptions::validate("export.savePreset", &json!({"qualty": 5}))
                .unwrap_err()
                .to_string()
                .starts_with("invalid parameters for `export.savePreset`")
        );
        // everything the presets, the dialog, Export with Previous and `render --opt` send passes
        for p in builtin_presets() {
            ExportOptions::from_params(&p.params).unwrap_or_else(|e| panic!("{}: {e}", p.name));
        }
        let full = ExportOptions {
            resize: Some(Resize::default()),
            limit_kb: Some(500),
            watermark: Some(Watermark { text: "©".into(), ..Default::default() }),
            bit_depth: Some(16),
            ..Default::default()
        };
        let mut j = full.to_json();
        for (k, v) in [
            ("dir", json!("/tmp/out")),
            ("background", json!(true)),
            ("preset", json!("x")),
            ("ids", json!([1, 2])),
            ("id", json!(1)),
            ("path", json!("a.jpg")),
        ] {
            j[k] = v;
        }
        assert_eq!(ExportOptions::from_params(&j).unwrap(), full);
        assert_eq!(ExportOptions::from_params(&ExportOptions::default().to_json()).unwrap(), ExportOptions::default());
        // null is "not given", as a dropped key is
        ExportOptions::from_params(&json!({"quality": 92, "longEdge": null, "watermark": null})).unwrap();
    }

    /// Issue #183: the `watermark` object's keys, kinds and ranges are checked (and documented in
    /// the `app.export` param string), instead of unknown keys passing and sizes clamping silently.
    #[test]
    fn watermark_params_are_validated() {
        use serde_json::json;
        let err = |w: serde_json::Value| ExportOptions::from_params(&json!({"watermark": w})).err().map(|e| e.to_string()).unwrap_or_default();
        assert!(
            err(json!(5))
                .contains("`watermark` must be text or an object {text, vertical, size, opacity, anchor, inset, color, shadow, image, imageWidth}")
        );
        assert!(
            err(json!({"text": "x", "bogus": 1}))
                .contains("unknown watermark key `bogus` (one of text, vertical, size, opacity, anchor, inset, color, shadow, image, imageWidth)")
        );
        assert!(err(json!({"text": "x", "sizee": 0.1})).contains("did you mean `size`"));
        for size in [json!(3), json!(0.5001), json!(0), json!("big")] {
            let e = err(json!({"text": "x", "size": size}));
            assert!(e.contains("`watermark.size` must be a number 0.005..0.5 (fraction of the short edge)"), "{size}: {e}");
        }
        assert!(err(json!({"text": "x", "opacity": 1.5})).contains("`watermark.opacity` must be a number 0..1"));
        assert!(err(json!({"text": "x", "inset": -0.1})).contains("`watermark.inset` must be a number 0..0.4"));
        assert!(err(json!({"image": "logo.png", "imageWidth": 2})).contains("`watermark.imageWidth` must be a number 0.01..1"));
        assert!(err(json!({"text": "x", "anchor": "middle"})).contains("`watermark.anchor` must be one of"));
        assert!(err(json!({"text": "x", "color": [255, 255]})).contains("`watermark.color` must be [r, g, b]"));
        assert!(err(json!({"text": "x", "color": [255, 255, 256]})).contains("`watermark.color`"));
        assert!(err(json!({"text": "x", "vertical": "yes"})).contains("`watermark.vertical` must be true or false"));
        assert!(err(json!({"text": 7})).contains("`watermark.text` must be a string"));
        // the documented shapes pass and read as before
        let o = ExportOptions::from_params(&json!({"watermark": "© me"})).unwrap();
        assert_eq!(o.watermark.map(|w| w.text).as_deref(), Some("© me"));
        let w = json!({"text": "日本語", "vertical": true, "size": 0.1, "opacity": 0.5, "anchor": "topLeft", "inset": 0.05, "color": [0, 0, 0], "shadow": false, "image": "", "imageWidth": 0.3});
        let wm = ExportOptions::from_params(&json!({"watermark": w})).unwrap().watermark.unwrap();
        assert!((wm.size - 0.1).abs() < 1e-6 && wm.vertical && wm.anchor == Anchor::TopLeft && wm.color == [0, 0, 0] && !wm.shadow);
        assert_eq!(ExportOptions::from_params(&json!({"watermark": {"text": "x", "size": 0.5}})).unwrap().watermark.map(|w| w.size), Some(0.5));
        // the Export dialog's sliders (1–15 % size, 5–100 % opacity, 2–100 % width) stay inside the accepted ranges
        assert!(WATERMARK_SIZE_RANGE.0 <= 0.01 && WATERMARK_SIZE_RANGE.1 >= 0.15);
        assert!(WATERMARK_IMAGE_WIDTH_RANGE.0 <= 0.02 && WATERMARK_IMAGE_WIDTH_RANGE.1 >= 1.0);
    }
}
