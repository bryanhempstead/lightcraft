//! LightCraft is English only (Bryan, 2026-11-23): the UI text is the English source text.
//! `tr` and `tr_format!` stay as the markers on display text, so the call sites are unchanged.

/// A built-in display label; returns it unchanged.
pub fn tr(source: &str) -> &str {
    source
}

/// `format!` for display text.
macro_rules! tr_format {
    ($($args:tt)*) => {
        format!($($args)*)
    };
}

// `macro_rules!` is textual: the module re-export is what lets call sites say
// `crate::i18n::tr_format!`.
pub(crate) use tr_format;

/// A stock preset's or profile's name for display (stock and user names alike are shown as written).
pub fn builtin_label(source: &str, _builtin: bool) -> &str {
    source
}

/// A profile's name for display.
pub fn profile_label<'a>(_id: &str, name: &'a str) -> &'a str {
    name
}

/// What the grid is titled for the session's source: its label, or a library folder's last
/// two names (`photos/travel`; the folder's name is the user's and stays verbatim).
pub fn source_title(session: &lightcraft_engine::Session) -> String {
    match (session.source, session.library_folder.as_deref()) {
        // a shoot narrowed to some of its folders or cameras says which (`Erika › raw/M262`)
        (lightcraft_engine::LibrarySource::LibraryFolder, Some(path)) => match session.active_album_part().filter(|p| !p.title.is_empty()) {
            Some(p) => format!("{} › {}", lightcraft_catalog::folders::folder_label(path), p.title),
            None => lightcraft_catalog::folders::folder_label(path),
        },
        // a collection narrowed to some of its folders or cameras says which (`Erika › raw/M262`)
        (source @ lightcraft_engine::LibrarySource::Album(_), _) => match session.active_album_part().filter(|p| !p.title.is_empty()) {
            Some(p) => format!("{} › {}", source.label(&session.catalog), p.title),
            None => source.label(&session.catalog),
        },
        // merges.: whose (`Erika › merges.`)
        (lightcraft_engine::LibrarySource::Merges, _) => session.merges_scope.clone().unwrap_or_default().title(&session.catalog),
        (source, _) => source.label(&session.catalog),
    }
}

/// A date group heading (`2026-09-20`, `2026-09`, `2026`): the grid's full form, or the date
/// sidebar's `short` one ("Sunday, 20" / "September" / "2026"). The grouping keys never change.
pub fn date_group_label(key: &str, short: bool) -> String {
    let label = lightcraft_catalog::dates::group_label(key);
    if !short {
        return label;
    }
    match key.len() {
        // "September 2026" → "September"
        7 => label.split(' ').next().unwrap_or(&label).to_string(),
        // "Sunday, 20 September 2026" → "Sunday, 20"
        10 => label.rsplitn(3, ' ').nth(2).unwrap_or(&label).to_string(),
        _ => label,
    }
}

/// A colour label's name: the user's custom name, or the built-in one.
pub fn color_label(catalog: &lightcraft_catalog::Catalog, label: lightcraft_catalog::ColorLabel) -> String {
    catalog.custom_label_name(label).map_or_else(|| catalog.label_name(label), str::to_string)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The interface's own symbols have a glyph in the fonts the UI installs.
    #[test]
    fn interface_symbols_are_paintable() {
        let ctx = egui::Context::default();
        ctx.set_fonts(crate::theme::font_definitions(lightcraft_engine::CRAFT_FONTS));
        let mut out = ctx.run_ui(egui::RawInput::default(), |_| {});
        out.textures_delta.clear();
        // ⌥ and ⌫ come from egui's bundled fonts; ▸ and ▾ are in neither (measured 2026-10-07).
        ctx.fonts_mut(|fonts| {
            for symbol in ['⌥', '⌫', '⇧', '⌘', '→', '…', '—', '“', '”', '·', '»', '⌄', '★'] {
                let font = egui::FontId::new(13.0, egui::FontFamily::Proportional);
                assert!(fonts.has_glyph(&font, symbol), "{symbol:?} has no glyph in any installed font");
            }
        });
    }

    /// The UI families, after one frame so the font definitions are loaded.
    fn fonts_ctx(craft: &'static [lightcraft_engine::CraftFont]) -> egui::Context {
        let ctx = egui::Context::default();
        ctx.set_fonts(crate::theme::font_definitions(craft));
        let mut out = ctx.run_ui(egui::RawInput::default(), |_| {});
        out.textures_delta.clear();
        ctx
    }

    /// Built with craft-fonts, CJK user text (file names, keywords, watermarks) renders with real
    /// glyphs (no tofu) in every UI family, Monospace included.
    #[test]
    fn craft_fonts_render_cjk_user_text() {
        for (script, sample) in [("Jpan", "日本語の文字"), ("Hans", "简体中文字"), ("Hant", "繁體中文字")] {
            if !lightcraft_engine::CRAFT_FONTS.iter().any(|font| font.serves(script)) {
                eprintln!("skipped {script}: built without a craft-fonts face for it");
                continue;
            }
            let ctx = fonts_ctx(lightcraft_engine::CRAFT_FONTS);
            ctx.fonts_mut(|fonts| {
                for family in
                    [egui::FontFamily::Proportional, egui::FontFamily::Name(crate::theme::FONT_SEMIBOLD.into()), egui::FontFamily::Monospace]
                {
                    let font = egui::FontId::new(13.0, family);
                    for ch in sample.chars() {
                        assert!(fonts.has_glyph(&font, ch), "{script}: {ch} in {font:?}");
                    }
                }
            });
        }
    }

    /// Built without craft-fonts, the UI still installs its fonts and runs; Latin text keeps Inter.
    #[test]
    fn the_ui_works_without_craft_fonts() {
        let ctx = fonts_ctx(&[]);
        ctx.fonts_mut(|fonts| {
            // (Not Monospace: egui's `has_glyph` reports false for glyphs of the family's
            // replacement-glyph face, which there is Hack, the face that draws Latin.)
            for family in [egui::FontFamily::Proportional, egui::FontFamily::Name(crate::theme::FONT_SEMIBOLD.into())] {
                let font = egui::FontId::new(13.0, family);
                assert!("LightCraft".chars().all(|ch| fonts.has_glyph(&font, ch)), "{font:?}");
            }
        });
        let mut app = crate::LightcraftApp::new(lightcraft_engine::Session::with_demo(), Default::default());
        for frame in 0..3 {
            let input = crate::headless::HeadlessView::raw_input(egui::vec2(1200.0, 800.0), 1.0, frame as f64 / 60.0, vec![]);
            let mut out = ctx.run_ui(input, |ui| {
                app.logic(ui.ctx());
                app.ui(ui);
            });
            out.textures_delta.clear();
        }
    }

    /// Settings saved when LightCraft had a language preference still load, and the old language
    /// commands and `ui.set {language}` answer with an error instead of doing nothing.
    #[test]
    fn english_only_old_settings_load_and_language_requests_fail_clearly() {
        use crate::control::{ControlRequest, Outcome};
        let legacy: crate::state::UiState = serde_json::from_str(r#"{"language":"ja","leftPanel":false}"#).unwrap();
        assert!(!legacy.left_panel);
        assert!(!serde_json::to_string(&legacy).unwrap().contains("language"));
        let mut app = crate::LightcraftApp::new(lightcraft_engine::Session::with_demo(), Default::default());
        let e = app.run("app.language.german", serde_json::json!({})).unwrap_err();
        assert!(e.contains("English only"), "{e}");
        let ctx = egui::Context::default();
        let (req, _) = ControlRequest::new("ui.set", serde_json::json!({"language": "de"}));
        let Outcome::Done(reply) = crate::control::handle(&mut app, &ctx, &req) else { panic!("expected a reply") };
        assert_eq!(reply["ok"], false, "{reply}");
        assert!(reply.to_string().contains("English only"), "{reply}");
        assert_eq!(tr("File"), "File");
        assert_eq!(tr_format!("Imported {} photo{}", 12, "s"), "Imported 12 photos");
    }

    /// Date headings keep the catalog's wording; a key that is not a date never panics.
    #[test]
    fn date_headings() {
        assert_eq!(date_group_label("2026-09-20", false), "Sunday, 20 September 2026");
        assert_eq!(date_group_label("2026-09-20", true), "Sunday, 20");
        assert_eq!(date_group_label("2026-09", true), "September");
        for key in ["", "abcd", "2026-xx", "2026-13", "2026-09-xx", "２０２６-09"] {
            let _ = (date_group_label(key, false), date_group_label(key, true));
        }
    }
}
