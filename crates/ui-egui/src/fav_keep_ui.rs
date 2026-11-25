//! Favorites kept safe in the app ([`lightcraft_engine::fav_keep`]): Settings ▸ General ▸
//! Favorites goes to the engine, the `kept.` mark on a heart, the favorites. row's counts and a
//! toast when the keep folder's disk (LOT48) is offline with copies waiting.

use egui::{Color32, Painter, Rect, Stroke, pos2, vec2};

use crate::LightcraftApp;
use crate::theme::Tokens;

/// The keep folder the settings ask for, when this app may write there ([`LightcraftApp::fav_keep_live`]).
pub fn wanted_root(app: &LightcraftApp) -> Option<std::path::PathBuf> {
    let s = &app.ui.settings;
    let dir = s.fav_keep_dir.trim();
    (app.fav_keep_live && s.fav_keep && !dir.is_empty()).then(|| std::path::PathBuf::from(dir))
}

/// Once a frame: the setting into the engine (a change starts a fresh look at every favorite),
/// a repaint when the copy worker has something due, the offline toast once per stretch offline.
pub fn tick(app: &mut LightcraftApp, ctx: &egui::Context) {
    let root = wanted_root(app);
    app.session.set_fav_keep_root(root);
    if let Some(d) = app.session.fav_keep.wake_in() {
        ctx.request_repaint_after(d.max(std::time::Duration::from_millis(100)));
    }
    match app.session.fav_keep.online() {
        Some(false) if !app.ui.fav_offline_told => {
            let st = app.session.fav_keep_status();
            app.ui.fav_offline_told = true;
            if st.waiting > 0 {
                let n = st.waiting;
                app.toast_soon(
                    format!(
                        "{disk} is offline: {n} favorite{s} wait{v} to be kept · they copy when it is back",
                        disk = st.disk,
                        s = if n == 1 { "" } else { "s" },
                        v = if n == 1 { "s" } else { "" }
                    ),
                    8.0,
                );
            }
        }
        Some(true) => app.ui.fav_offline_told = false,
        _ => {}
    }
}

/// "312 kept on LOT48 · 4 waiting" (and what is wrong, when something is), for the favorites.
/// row's tooltip and Settings.
pub fn summary(app: &LightcraftApp) -> String {
    let st = app.session.fav_keep_status();
    if !st.on {
        return if app.ui.settings.fav_keep {
            "kept safe: not in this window (only the app with its saved settings copies)".into()
        } else {
            "kept safe: off (Settings ▸ General ▸ Favorites)".into()
        };
    }
    let mut parts = vec![format!("{} kept on {}", st.kept, st.disk)];
    if st.waiting > 0 {
        parts.push(format!("{} waiting", st.waiting));
    }
    if st.using_kept > 0 {
        parts.push(format!("{} shown from the kept copy (original gone)", st.using_kept));
    }
    if st.failed > 0 {
        parts.push(format!("{} couldn't be copied (original not found), tried again later", st.failed));
    }
    if st.online == Some(false) {
        parts.push(format!("{} offline: copies wait until it is back", st.disk));
    } else if st.running {
        parts.push("copying…".into());
    }
    parts.join(" · ")
}

/// The `kept.` mark: a small check on the heart's lower right corner (`heart` = the heart's rect).
pub fn kept_mark(p: &Painter, t: &Tokens, heart: Rect) {
    // off the heart's corner, so the heart still reads as a heart (inside the badge bar)
    let r = (heart.width() * 0.3).max(3.2);
    let c = heart.right_bottom() + vec2(r * 0.45, -r * 0.55);
    p.circle(c, r, Color32::from_black_alpha(215), Stroke::new(1.0, t.fav));
    let s = r * 0.62;
    let pts = [pos2(c.x - s * 0.75, c.y), pos2(c.x - s * 0.15, c.y + s * 0.6), pos2(c.x + s * 0.8, c.y - s * 0.6)];
    p.line_segment([pts[0], pts[1]], Stroke::new(1.3, Color32::WHITE));
    p.line_segment([pts[1], pts[2]], Stroke::new(1.3, Color32::WHITE));
}
