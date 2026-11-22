//! Lightroom Classic export presets (`.lrtemplate`, a Lua table under `Export Presets/`) read as
//! LightCraft export presets: the export settings Lightroom keeps in `value` (`LR_format`,
//! `LR_jpeg_quality`, `LR_size_*`, `LR_export_destinationType`, …) mapped onto `app.export`
//! params. Settings LightCraft has no counterpart for (plug-in sections, video presets) are
//! reported, never guessed. The file is only read.

use serde_json::{Map, Value, json};

use crate::export::ExportPreset;
use crate::preset_import::{Lua, parse_lua};

/// A value of the preset's `value` table.
fn field<'a>(v: &'a Lua, key: &str) -> Option<&'a Lua> {
    v.get(key)
}

fn num(v: &Lua, key: &str) -> Option<f64> {
    match field(v, key)? {
        Lua::Num(n) if n.is_finite() => Some(*n),
        Lua::Str(s) => s.trim().parse::<f64>().ok().filter(|n| n.is_finite()),
        _ => None,
    }
}

fn flag(v: &Lua, key: &str) -> Option<bool> {
    match field(v, key)? {
        Lua::Bool(b) => Some(*b),
        _ => None,
    }
}

fn text<'a>(v: &'a Lua, key: &str) -> Option<&'a str> {
    field(v, key)?.str()
}

/// Lightroom's filename tokens (`{{image_name}}`, `{{naming_sequenceNumber_3Digits}}`, …) as
/// LightCraft's; the tokens it has no counterpart for are listed in `unknown`.
pub fn convert_tokens(lr: &str, unknown: &mut Vec<String>) -> String {
    let mut out = String::new();
    let mut rest = lr;
    while let Some(at) = rest.find("{{") {
        out.push_str(rest.get(..at).unwrap_or(""));
        let after = rest.get(at + 2..).unwrap_or("");
        let Some(end) = after.find("}}") else {
            out.push_str(rest.get(at..).unwrap_or(""));
            rest = "";
            break;
        };
        let token = after.get(..end).unwrap_or("");
        rest = after.get(end + 2..).unwrap_or("");
        let digits = |prefix: &str| {
            token.strip_prefix(prefix).and_then(|d| d.strip_suffix("Digits")).and_then(|d| d.parse::<u8>().ok()).filter(|d| (1..=9).contains(d))
        };
        let ours = match token {
            "image_name" | "image_originalName" => "{name}".to_string(),
            "image_originalNumberSuffix" | "image_numberSuffix" => "{originalnum}".to_string(),
            "image_folderName" => "{folder}".to_string(),
            "custom_token" => "{text}".to_string(),
            "date_YYYY" => "{yyyy}".to_string(),
            "date_YY" => "{yy}".to_string(),
            "date_MM" => "{mm}".to_string(),
            "date_DD" => "{dd}".to_string(),
            "date_Month" => "{month}".to_string(),
            "date_Mon" => "{mon}".to_string(),
            "date_YYYYMMDD" => "{yyyymmdd}".to_string(),
            "date_YYMMDD" => "{yymmdd}".to_string(),
            "date_hh" | "date_HH" => "{hour}".to_string(),
            "date_mm" => "{minute}".to_string(),
            "date_ss" => "{second}".to_string(),
            "date_DDD" => "{julian}".to_string(),
            "com.adobe.title" => "{title}".to_string(),
            "com.adobe.caption" => "{caption}".to_string(),
            "com.adobe.creator" => "{creator}".to_string(),
            "com.adobe.copyright" => "{copyright}".to_string(),
            "com.adobe.location" => "{location}".to_string(),
            "com.adobe.city" => "{city}".to_string(),
            "com.adobe.state" => "{state}".to_string(),
            "com.adobe.country" => "{country}".to_string(),
            "com.adobe.rating" => "{rating}".to_string(),
            "com.adobe.colorLabel" => "{label}".to_string(),
            "com.adobe.isoSpeedRating" => "{iso}".to_string(),
            "com.adobe.focalLength" => "{focal}".to_string(),
            "com.adobe.aperture" => "{aperture}".to_string(),
            "com.adobe.shutterSpeed" => "{shutter}".to_string(),
            "com.adobe.model" | "com.adobe.cameraModel" => "{model}".to_string(),
            "com.adobe.make" | "com.adobe.cameraMake" => "{make}".to_string(),
            "com.adobe.lens" => "{lens}".to_string(),
            "naming_totalCount" | "naming_sequenceTotal" => "{total}".to_string(),
            _ => match (digits("naming_sequenceNumber_"), digits("naming_imageNumber_"), digits("naming_importNumber_")) {
                (Some(d), _, _) => format!("{{seq:{d}}}"),
                (None, Some(_), _) => "{image}".to_string(),
                (None, None, Some(_)) => "{import}".to_string(),
                _ => {
                    unknown.push(format!("{{{{{token}}}}}"));
                    String::new()
                }
            },
        };
        out.push_str(&ours);
    }
    out.push_str(rest);
    out
}

/// Read a Lightroom export preset: its title (or `fallback_name`), the `app.export` params it
/// maps to, and the settings that couldn't be carried over.
pub fn read_export_lrtemplate(text_in: &str, fallback_name: &str) -> Result<(ExportPreset, Vec<String>), String> {
    let root = parse_lua(text_in.trim_start_matches('\u{feff}'))?;
    if let Some(kind) = text(&root, "type")
        && !kind.eq_ignore_ascii_case("Export")
    {
        return Err(format!("not an export preset (a {kind} preset)"));
    }
    let v = field(&root, "value").ok_or("no `value` table: not a Lightroom export preset")?;
    let title =
        text(&root, "title").map(crate::preset_import::delocalize).filter(|t| !t.trim().is_empty()).unwrap_or_else(|| fallback_name.to_string());
    let mut p = Map::new();
    let mut notes: Vec<String> = Vec::new();
    let mut put = |k: &str, x: Value| {
        p.insert(k.to_string(), x);
    };
    // Export To (top)
    if let Some(svc) = text(v, "exportServiceProvider") {
        match svc {
            "com.adobe.ag.export.file" => {}
            s if s.contains("email") => put("service", json!("email")),
            s if s.contains("burn") || s.contains("cd") => put("service", json!("disc")),
            s => notes.push(format!("export service {s} (a plug-in): exported to the hard drive")),
        }
    }
    // File Settings
    if let Some(f) = text(v, "LR_format") {
        let ours = match f.to_ascii_uppercase().as_str() {
            "JPEG" => Some("jpeg"),
            "TIFF" => Some("tiff"),
            "PSD" => Some("psd"),
            "DNG" => Some("dng"),
            "PNG" => Some("png"),
            "AVIF" => Some("avif"),
            "JXL" | "JPEGXL" | "JPEG XL" => Some("jxl"),
            "ORIGINAL" => Some("original"),
            _ => None,
        };
        match ours {
            Some(o) => put("format", json!(o)),
            None => notes.push(format!("format {f}")),
        }
    }
    if let Some(q) = num(v, "LR_jpeg_quality") {
        // 0..1 in the file (Lightroom shows 0..100)
        let q = if q <= 1.0 { q * 100.0 } else { q };
        put("quality", json!(q.round().clamp(1.0, 100.0) as u64));
    }
    if flag(v, "LR_jpeg_useLimitSize") == Some(true)
        && let Some(k) = num(v, "LR_jpeg_limitSize").filter(|k| *k >= 1.0)
    {
        put("limitKb", json!(k.round().min(f64::from(u32::MAX)) as u64));
    }
    if let Some(cs) = text(v, "LR_export_colorSpace") {
        let ours = match cs.to_ascii_lowercase().as_str() {
            "srgb" => Some("srgb"),
            "adobergb" => Some("adobeRgb"),
            "prophotorgb" => Some("proPhoto"),
            "displayp3" | "p3" => Some("displayP3"),
            "rec2020" | "hdr_rec2020" => Some("rec2020"),
            _ => None,
        };
        match ours {
            Some(c) => put("colorSpace", json!(c)),
            None if std::path::Path::new(cs).is_absolute() => put("iccProfile", json!(cs)),
            None => notes.push(format!("color space {cs}")),
        }
    }
    if let Some(b) = num(v, "LR_export_bitDepth").map(|b| b.round() as u64).filter(|b| matches!(b, 8 | 16 | 32)) {
        put("bitDepth", json!(b));
    }
    if let Some(c) = text(v, "LR_tiff_compressionMethod") {
        let lc = c.to_ascii_lowercase();
        let ours = if lc.contains("lzw") {
            "lzw"
        } else if lc.contains("zip") {
            "zip"
        } else {
            "none"
        };
        put("tiffCompression", json!(ours));
    }
    if flag(v, "LR_export_useHDR").or(flag(v, "LR_export_HDR")) == Some(true) {
        put("hdr", json!(true));
    }
    // Image Sizing
    let ppi = num(v, "LR_size_resolution")
        .filter(|r| *r > 0.0)
        .map(|r| if text(v, "LR_size_resolutionUnits").is_some_and(|u| u.eq_ignore_ascii_case("cm")) { r * 2.54 } else { r });
    if let Some(r) = ppi {
        put("ppi", json!(r.round().clamp(1.0, f64::from(u16::MAX)) as u64));
    }
    if text(v, "LR_size_resolutionUnits").is_some_and(|u| u.eq_ignore_ascii_case("cm")) {
        put("resolutionUnit", json!("cm"));
    }
    if flag(v, "LR_size_doConstrain") == Some(true) {
        let (w, h) = (num(v, "LR_size_maxWidth").unwrap_or(0.0).max(0.0), num(v, "LR_size_maxHeight").unwrap_or(0.0).max(0.0));
        let unit = match text(v, "LR_size_units").map(str::to_ascii_lowercase).as_deref() {
            Some("inch" | "inches" | "in") => "inches",
            Some("cm") => "cm",
            _ => "pixels",
        };
        let resize = match text(v, "LR_size_resizeType").unwrap_or("wh") {
            "dimensions" => json!({"mode": "dimensions", "value": w, "height": h, "unit": unit}),
            "longEdge" => json!({"mode": "longEdge", "value": w.max(h), "unit": unit}),
            "shortEdge" => json!({"mode": "shortEdge", "value": w.max(h), "unit": unit}),
            "megapixels" => json!({"mode": "megapixels", "value": num(v, "LR_size_megapixels").unwrap_or(12.0).max(0.01)}),
            "percentage" => json!({"mode": "percent", "value": num(v, "LR_size_percentage").map_or(100.0, |x| if x <= 1.0 { x * 100.0 } else { x })}),
            _ => json!({"mode": "widthHeight", "value": w, "height": h, "unit": unit}),
        };
        let mut r = resize;
        r["dontEnlarge"] = json!(flag(v, "LR_size_doNotEnlarge").unwrap_or(true));
        put("resize", r);
    } else if field(v, "LR_size_doConstrain").is_some() {
        put("longEdge", json!(0));
    }
    // Output Sharpening
    if flag(v, "LR_outputSharpeningOn") == Some(true) {
        let media = match text(v, "LR_outputSharpeningMedia").map(str::to_ascii_lowercase).as_deref() {
            Some("matte") => "matte",
            Some("glossy") => "glossy",
            _ => "screen",
        };
        put("sharpen", json!(media));
        let level = match num(v, "LR_outputSharpeningLevel").map(|l| l.round() as i64) {
            Some(1) => "low",
            Some(3) => "high",
            _ => "standard",
        };
        put("sharpenAmount", json!(level));
    } else if flag(v, "LR_outputSharpeningOn") == Some(false) {
        put("sharpen", json!("none"));
    }
    // Metadata
    if let Some(m) = text(v, "LR_embeddedMetadataOption") {
        let lc = m.to_ascii_lowercase();
        let ours = if lc.contains("contact") {
            "copyrightContact"
        } else if lc.contains("copyright") {
            "copyright"
        } else if lc.contains("exceptcamerainfo") || lc.contains("exceptcameraandcameraraw") || lc.contains("exceptcamera&") {
            "allExceptCamera"
        } else if lc.contains("exceptcameraraw") {
            "allExceptCameraRaw"
        } else {
            "all"
        };
        put("metadata", json!(ours));
    }
    if let Some(b) = flag(v, "LR_removeLocationMetadata") {
        put("removeLocation", json!(b));
    }
    if let Some(b) = flag(v, "LR_removeFaceMetadata") {
        put("removePeople", json!(b));
    }
    if text(v, "LR_metadata_keywordOptions").is_some_and(|k| k.to_ascii_lowercase().contains("hierarch")) {
        put("keywordHierarchy", json!(true));
    }
    // Watermarking
    if flag(v, "LR_useWatermark") == Some(true) {
        match text(v, "LR_watermarking_id") {
            Some(id) if id.contains("simpleCopyright") => put("watermark", json!({"text": crate::export::SIMPLE_COPYRIGHT})),
            Some(id) => notes.push(format!("watermark {id} (Lightroom's own watermark: save it again in LightCraft)")),
            None => {}
        }
    }
    // File Naming
    if flag(v, "LR_renamingTokensOn") == Some(true)
        && let Some(tokens) = text(v, "LR_tokens")
    {
        let mut unknown = Vec::new();
        let naming = convert_tokens(tokens, &mut unknown);
        if !naming.trim().is_empty() {
            put("naming", json!(naming));
        }
        notes.extend(unknown.into_iter().map(|t| format!("file name token {t}")));
    }
    if let Some(t) = text(v, "LR_tokenCustomString").filter(|t| !t.is_empty()) {
        put("customText", json!(t));
    }
    if let Some(n) = num(v, "LR_initialSequenceNumber").filter(|n| *n >= 0.0) {
        put("startNumber", json!(n.round().min(999_999.0) as u64));
    }
    match text(v, "LR_extensionCase").map(str::to_ascii_lowercase).as_deref() {
        Some("lowercase") => put("extCase", json!("lower")),
        Some("uppercase") => put("extCase", json!("upper")),
        _ => {}
    }
    // Export Location
    match text(v, "LR_collisionHandling").map(str::to_ascii_lowercase).as_deref() {
        Some("ask") => put("conflict", json!("ask")),
        Some("rename") => put("conflict", json!("unique")),
        Some("overwrite") => put("conflict", json!("overwrite")),
        Some("skip") => put("conflict", json!("skip")),
        _ => {}
    }
    let dest = text(v, "LR_export_destinationType").or_else(|| text(v, "export_destinationType"));
    let export_to = match dest.map(str::to_ascii_lowercase).as_deref() {
        Some("sourcefolder") => Some("sameFolder"),
        Some("desktop") => Some("desktop"),
        Some("documents") => Some("documents"),
        Some("home") => Some("home"),
        Some("pictures") => Some("pictures"),
        Some("chooselater" | "choose later" | "tempfolder") => Some("chooseLater"),
        Some("specificfolder") => Some("specificFolder"),
        _ => None,
    };
    if let Some(t) = export_to {
        put("exportTo", json!(t));
    }
    if export_to == Some("specificFolder")
        && let Some(d) =
            text(v, "LR_export_destinationPathPrefix").or_else(|| text(v, "export_destinationPathPrefix")).filter(|d| !d.trim().is_empty())
    {
        put("dir", json!(d));
    }
    if flag(v, "LR_export_useSubfolder").or_else(|| flag(v, "export_useSubfolder")) == Some(true)
        && let Some(s) =
            text(v, "LR_export_destinationPathSuffix").or_else(|| text(v, "export_destinationPathSuffix")).filter(|s| !s.trim().is_empty())
    {
        put("subfolder", json!(s));
    }
    if let Some(b) = flag(v, "LR_reimportExportedPhoto") {
        put("addToCatalog", json!(b));
        if b && flag(v, "LR_reimport_stackWithOriginal") == Some(true) {
            let pos = text(v, "LR_reimport_stackWithOriginal_position").filter(|p| p.eq_ignore_ascii_case("above")).map_or("below", |_| "above");
            put("addToStack", json!(pos));
        }
    }
    // Post-Processing
    match text(v, "LR_export_postProcessing").or_else(|| text(v, "export_postProcessing")).map(str::to_ascii_lowercase).as_deref() {
        Some("donothing") | None => {}
        Some("showinfinder" | "revealinfinder" | "showinexplorer") => put("after", json!("showInFinder")),
        Some("openinphotoshop") => {
            put("after", json!("openInApp"));
            if let Some(ps) = crate::export::photoshop_app() {
                put("afterApp", json!(ps));
            }
        }
        Some(other) => {
            put("after", json!("openInApp"));
            match text(v, "LR_export_postProcessingApplication").or_else(|| text(v, "export_postProcessingApplication")) {
                Some(app) => put("afterApp", json!(app)),
                None => notes.push(format!("after export: {other} (opens in the default app)")),
            }
        }
    }
    // Video
    match text(v, "LR_export_videoFileHandling").map(str::to_ascii_lowercase).as_deref() {
        Some("include") => put("video", json!({"include": true})),
        Some("exclude") => put("video", json!({"include": false})),
        _ => {}
    }
    let params = Value::Object(p);
    // what app.export would refuse is a mapping mistake: drop it rather than import a broken preset
    let params = match crate::export::ExportOptions::validate("export.importPresets", &params) {
        Ok(()) => params,
        Err(e) => return Err(format!("{title}: {e}")),
    };
    Ok((ExportPreset { name: title, params, folder: String::new() }, notes))
}

#[cfg(test)]
mod tests {
    use super::*;

    const PRESET: &str = r#"s = {
	id = "6C2F0F2A-0000-0000-0000-000000000000",
	internalName = "Web 2048",
	title = "Web 2048",
	type = "Export",
	value = {
		exportServiceProvider = "com.adobe.ag.export.file",
		LR_collisionHandling = "ask",
		LR_embeddedMetadataOption = "copyrightOnly",
		LR_export_bitDepth = 8,
		LR_export_colorSpace = "sRGB",
		LR_export_destinationPathPrefix = "/Users/someone/Pictures",
		LR_export_destinationPathSuffix = "Web",
		LR_export_destinationType = "specificFolder",
		LR_export_useSubfolder = true,
		LR_export_postProcessing = "showInFinder",
		LR_extensionCase = "lowercase",
		LR_format = "JPEG",
		LR_initialSequenceNumber = 5,
		LR_jpeg_limitSize = 500,
		LR_jpeg_quality = 0.8,
		LR_jpeg_useLimitSize = true,
		LR_metadata_keywordOptions = "lightroomHierarchical",
		LR_outputSharpeningLevel = 3,
		LR_outputSharpeningMedia = "screen",
		LR_outputSharpeningOn = true,
		LR_reimportExportedPhoto = true,
		LR_reimport_stackWithOriginal = true,
		LR_reimport_stackWithOriginal_position = "above",
		LR_removeFaceMetadata = true,
		LR_removeLocationMetadata = true,
		LR_renamingTokensOn = true,
		LR_size_doConstrain = true,
		LR_size_doNotEnlarge = true,
		LR_size_maxHeight = 2048,
		LR_size_maxWidth = 2048,
		LR_size_resizeType = "longEdge",
		LR_size_resolution = 72,
		LR_size_resolutionUnits = "inch",
		LR_size_units = "pixels",
		LR_tokenCustomString = "Trip",
		LR_tokens = "{{custom_token}}-{{naming_sequenceNumber_3Digits}}-{{date_YYYY}}{{unknown_thing}}",
		LR_useWatermark = true,
		LR_watermarking_id = "<simpleCopyrightWatermark>",
	},
	version = 0,
}
"#;

    #[test]
    fn maps_a_lightroom_export_preset() {
        let (p, notes) = read_export_lrtemplate(PRESET, "fallback").unwrap();
        assert_eq!(p.name, "Web 2048");
        let o = crate::export::ExportOptions::from_params(&p.params).unwrap();
        use crate::export::*;
        assert_eq!(o.format, ExportFormat::Jpeg);
        assert_eq!(o.quality, 80);
        assert_eq!(o.limit_kb, Some(500));
        assert_eq!(o.color_space, OutputSpace::Srgb);
        assert_eq!(o.conflict, Conflict::Ask);
        assert_eq!(o.metadata, MetadataPolicy::Copyright);
        assert!(o.remove_location && o.remove_people && o.keyword_hierarchy);
        assert_eq!((o.sharpen, o.sharpen_amount), (SharpenFor::Screen, SharpenAmount::High));
        assert_eq!(o.resize.map(|r| (r.mode, r.value)), Some((ResizeMode::LongEdge, 2048.0)));
        assert_eq!(o.ppi, 72);
        assert_eq!(o.export_to, ExportTo::SpecificFolder);
        assert_eq!(p.params["dir"], "/Users/someone/Pictures");
        assert_eq!(o.subfolder, "Web");
        assert_eq!(o.after, AfterExport::ShowInFinder);
        assert!(o.add_to_catalog && o.add_to_stack == Some(StackPlace::Above));
        assert_eq!(o.naming, "{text}-{seq:3}-{yyyy}");
        assert_eq!((o.custom_text.as_str(), o.start_number), ("Trip", 5));
        assert_eq!(o.ext_case, crate::rename::ExtCase::Lower);
        assert_eq!(o.watermark.map(|w| w.text), Some(SIMPLE_COPYRIGHT.to_string()));
        assert!(notes.iter().any(|n| n.contains("unknown_thing")), "{notes:?}");
    }

    #[test]
    fn refuses_other_presets_and_garbage() {
        assert!(read_export_lrtemplate("s = { type = \"Develop\", value = {} }", "x").unwrap_err().contains("not an export preset"));
        assert!(read_export_lrtemplate("s = { title = \"x\" }", "x").is_err());
        assert!(read_export_lrtemplate("{{{{", "x").is_err());
        // hostile numbers stay in range
        let (p, _) = read_export_lrtemplate(
            "s = { type = \"Export\", value = { LR_jpeg_quality = 1e300, LR_size_resolution = -4, LR_initialSequenceNumber = 1e99 } }",
            "Odd",
        )
        .unwrap();
        assert_eq!(p.name, "Odd");
        assert!(crate::export::ExportOptions::from_params(&p.params).is_ok());
    }

    #[test]
    fn tokens_convert() {
        let mut u = Vec::new();
        assert_eq!(convert_tokens("{{image_name}}_{{naming_sequenceNumber_5Digits}}", &mut u), "{name}_{seq:5}");
        assert_eq!(convert_tokens("plain {{", &mut u), "plain {{");
        assert!(u.is_empty());
    }
}
