//! LR-MAP-MODULE: Lightroom Classic's Map module — OpenStreetMap tiles (with their attribution),
//! a pin for the photos in view that have a GPS position (nearby ones grouped, with a count),
//! photos dragged from the filmstrip or grid dropped on the map to geotag them, and the Saved
//! Locations panel on the left. Pins are picked like grid cells: click = just these, ⌘-click =
//! add / remove, ⇧-click = the run from the last click; all. / none. above the map.
//!
//! Tiles come from [`lightcraft_engine::map::TileStore`] (background threads, disk cache); while
//! the network is out the map is an empty grid with a notice, and pins, geotagging and saved
//! locations keep working.

use std::collections::HashMap;
use std::sync::Arc;

use egui::{Align2, Color32, Rect, Sense, Stroke, StrokeKind, pos2, vec2};
use lightcraft_engine::map::{self, Tile, TileId, TileStore};
use serde_json::{Value, json};

use crate::LightcraftApp;
use crate::theme::Tokens;
use crate::widgets::{register, text_button};

/// Height of the row of buttons above the map.
const BAR: f32 = 40.0;
/// Pins closer than this (screen px) are drawn as one, with a count.
const CLUSTER_PX: f32 = 30.0;
/// Textures kept for tiles not drawn lately.
const TEXTURES: usize = 320;

/// The map's tiles, their textures and what the Saved Locations panel shows.
#[derive(Default)]
pub struct MapRuntime {
    store: Option<Arc<TileStore>>,
    /// The folder `store` caches into (a new library gets a new store).
    store_dir: Option<std::path::PathBuf>,
    /// Never fetch tiles (tests, or the user's choice); cached ones are still drawn.
    pub offline: bool,
    textures: HashMap<TileId, (egui::TextureHandle, u64)>,
    frame: u64,
    /// The map area's last width (px), for a new saved location's radius.
    width: f32,
    /// (catalog revision, `map.locations`).
    locations: Option<(u64, Vec<Value>)>,
}

impl MapRuntime {
    /// `{online, inMemory, fetched, pending, error}` of the tile store (null before the map was shown).
    pub fn status(&self) -> Value {
        self.store.as_ref().map(|s| s.status()).unwrap_or(Value::Null)
    }
}

fn store(app: &mut LightcraftApp) -> Arc<TileStore> {
    let lib = app.session.library.as_ref().filter(|l| l.on_disk).map(|l| l.dir.clone());
    let dir = TileStore::default_dir(lib.as_deref());
    match &app.map.store {
        Some(s) if app.map.store_dir == dir => s.clone(),
        _ => {
            let s = TileStore::new(dir.clone(), !app.map.offline);
            app.map.store = Some(s.clone());
            app.map.store_dir = dir;
            app.map.textures.clear();
            s
        }
    }
}

/// Radius (metres) for a location saved at the map's centre: a sixth of the view's width.
pub fn default_radius(app: &LightcraftApp) -> f64 {
    let w = if app.map.width > 10.0 { app.map.width as f64 } else { 800.0 };
    (w / 6.0 * map::metres_per_px(app.ui.map.lat, app.ui.map.zoom)).clamp(10.0, 2_000_000.0).round()
}

fn locations(app: &mut LightcraftApp) -> Vec<Value> {
    let rev = app.session.catalog.revision;
    if let Some((r, v)) = &app.map.locations
        && *r == rev
    {
        return v.clone();
    }
    let v = app.session.execute("map.locations", &json!({})).ok().and_then(|v| v.as_array().cloned()).unwrap_or_default();
    app.map.locations = Some((rev, v.clone()));
    v
}

/// A group of pins drawn as one.
struct Cluster {
    pos: egui::Pos2,
    ids: Vec<u64>,
}

/// The Map module's canvas: the button row, then the map.
pub fn show(app: &mut LightcraftApp, ui: &mut egui::Ui) {
    let t = Tokens::get(ui.ctx());
    let full = ui.max_rect();
    let bar = Rect::from_min_size(full.min, vec2(full.width(), BAR));
    let area = Rect::from_min_max(pos2(full.left(), bar.bottom()), full.max);
    app.map.width = area.width();
    app.map.frame += 1;
    let pins: Vec<(u64, f64, f64)> = app
        .session
        .execute("map.pins", &json!({}))
        .ok()
        .and_then(|v| v["pins"].as_array().cloned())
        .unwrap_or_default()
        .iter()
        .filter_map(|p| Some((p["id"].as_u64()?, p["lat"].as_f64()?, p["lon"].as_f64()?)))
        .collect();
    if app.ui.map.fit_pending {
        app.ui.map.fit_pending = false;
        let pts: Vec<(f64, f64)> = pins.iter().map(|p| (p.1, p.2)).collect();
        if let Some(((la, lo), z)) = map::fit(&pts, area.width() as f64, area.height() as f64) {
            (app.ui.map.lat, app.ui.map.lon, app.ui.map.zoom) = (la, lo, z);
        }
    }
    toolbar(app, ui, bar, &pins);
    canvas(app, ui, area, &pins, &t);
}

fn toolbar(app: &mut LightcraftApp, ui: &mut egui::Ui, bar: Rect, pins: &[(u64, f64, f64)]) {
    let t = Tokens::get(ui.ctx());
    ui.painter().rect_filled(bar, 0.0, t.chrome);
    ui.painter().hline(bar.x_range(), bar.bottom() - 0.5, Stroke::new(1.0, t.divider));
    let mut child =
        ui.new_child(egui::UiBuilder::new().max_rect(bar.shrink2(vec2(12.0, 8.0))).layout(egui::Layout::left_to_right(egui::Align::Center)));
    child.spacing_mut().item_spacing.x = 8.0;
    let run = |app: &mut LightcraftApp, ui: &mut egui::Ui, id: &str, label: &str, cmd: &str, params: Value, tip: &str| {
        if text_button(ui, id, label, false).on_hover_text(crate::i18n::tr(tip)).clicked()
            && let Err(e) = app.run(cmd, params)
        {
            app.toast(ui.ctx(), e);
        }
    };
    run(app, &mut child, "mapFit", "fit.", "map.fit", json!({}), "Show every pin");
    run(app, &mut child, "mapZoomOut", "out.", "map.zoomOut", json!({}), "Zoom out");
    run(app, &mut child, "mapZoomIn", "in.", "map.zoomIn", json!({}), "Zoom in");
    run(app, &mut child, "mapSave", "save.", "dialog.mapSaveLocation", json!({}), "Save the place at the centre of the map as a location");
    let ids: Vec<u64> = pins.iter().map(|p| p.0).collect();
    run(app, &mut child, "mapAll", "all.", "library.select", json!({"ids": ids, "mode": "replace"}), "Select every photo on the map");
    run(app, &mut child, "mapNone", "none.", "library.selectNone", json!({}), "Select none");
    let total = app.session.visible_cloned().len();
    let text = format!("{} of {total} photos placed", pins.len());
    child.label(egui::RichText::new(text).font(t.font(12.0)).color(t.text_dim));
}

/// The world pixel (at the current zoom) of the map's centre, and the world's width.
fn centre(app: &LightcraftApp) -> ((f64, f64), f64) {
    let z = app.ui.map.zoom;
    (map::world_px(app.ui.map.lat, app.ui.map.lon, z), map::TILE * 2f64.powf(z.clamp(0.0, map::MAX_ZOOM)))
}

/// Screen position of a world pixel: the copy of the world nearest the centre.
fn to_screen(area: Rect, c: (f64, f64), world: f64, w: (f64, f64)) -> egui::Pos2 {
    let mut dx = w.0 - c.0;
    if world > 0.0 {
        dx -= (dx / world).round() * world;
    }
    area.center() + vec2(dx as f32, (w.1 - c.1) as f32)
}

/// The latitude / longitude under a screen position.
fn to_lat_lon(app: &LightcraftApp, area: Rect, p: egui::Pos2) -> (f64, f64) {
    let (c, _) = centre(app);
    let d = p - area.center();
    map::lat_lon(c.0 + d.x as f64, c.1 + d.y as f64, app.ui.map.zoom)
}

/// Zoom to `z`, keeping the place under `anchor` (screen) where it is.
fn zoom_at(app: &mut LightcraftApp, area: Rect, anchor: egui::Pos2, z: f64) {
    let z = z.clamp(1.0, map::MAX_ZOOM);
    let (la, lo) = to_lat_lon(app, area, anchor);
    app.ui.map.zoom = z;
    let w = map::world_px(la, lo, z);
    let d = anchor - area.center();
    let (lat, lon) = map::lat_lon(w.0 - d.x as f64, w.1 - d.y as f64, z);
    (app.ui.map.lat, app.ui.map.lon) = (lat, lon);
}

fn canvas(app: &mut LightcraftApp, ui: &mut egui::Ui, area: Rect, pins: &[(u64, f64, f64)], t: &Tokens) {
    let resp = ui.interact(area, egui::Id::new("map-canvas"), Sense::click_and_drag());
    register(ui.ctx(), "map:canvas", area);
    let dragging_photos = app.ui.dragging_photos.clone();
    // ---- navigation: drag pans, the wheel / pinch zooms about the pointer, double-click zooms in
    if dragging_photos.is_none() && resp.dragged() {
        let d = resp.drag_delta();
        let (c, _) = centre(app);
        let (lat, lon) = map::lat_lon(c.0 - d.x as f64, c.1 - d.y as f64, app.ui.map.zoom);
        (app.ui.map.lat, app.ui.map.lon) = (lat, lon);
    }
    if let Some(ptr) = resp.hover_pos() {
        let (scroll, pinch) = ui.input(|i| (i.smooth_scroll_delta.y, i.zoom_delta()));
        let dz = scroll as f64 / 240.0 + (pinch as f64).log2();
        if dz.abs() > 1e-4 && dz.is_finite() {
            zoom_at(app, area, ptr, app.ui.map.zoom + dz);
        }
        if resp.double_clicked() {
            zoom_at(app, area, ptr, app.ui.map.zoom.round() + 1.0);
        }
    }
    let painter = ui.painter_at(area);
    painter.rect_filled(area, 0.0, Color32::from_gray(58));
    // ---- tiles
    let store = store(app);
    let zoom = app.ui.map.zoom;
    let z = zoom.round().clamp(0.0, map::MAX_ZOOM);
    let scale = 2f64.powf(zoom - z);
    let ts = map::TILE * scale;
    let (c, world) = centre(app);
    let (x0, y0) = (c.0 - area.width() as f64 / 2.0, c.1 - area.height() as f64 / 2.0);
    let (tx0, ty0) = ((x0 / ts).floor() as i64, (y0 / ts).floor() as i64);
    let (tx1, ty1) = (((x0 + area.width() as f64) / ts).floor() as i64, ((y0 + area.height() as f64) / ts).floor() as i64);
    let mut missing = 0usize;
    let mut loading = 0usize;
    let frame = app.map.frame;
    // (a view at most ~40 tiles across: anything more is a broken zoom, drawn as the grid)
    if (tx1 - tx0) < 40 && (ty1 - ty0) < 40 {
        for ty in ty0..=ty1 {
            for tx in tx0..=tx1 {
                let r = Rect::from_min_size(
                    area.min + vec2((tx as f64 * ts - x0) as f32, (ty as f64 * ts - y0) as f32),
                    vec2(ts as f32 + 0.5, ts as f32 + 0.5),
                );
                let Some(id) = TileId::wrapped(z as u8, tx, ty) else { continue };
                let tex = match app.map.textures.get_mut(&id) {
                    Some((tex, used)) => {
                        *used = frame;
                        Some(tex.id())
                    }
                    None => match store.get(id) {
                        Tile::Ready(img) => {
                            let color = Arc::new(egui::ColorImage::from_rgba_unmultiplied([img.width, img.height], &img.as_bytes()));
                            let tex = ui.ctx().load_texture(format!("map-{}-{}-{}", id.z, id.x, id.y), color, egui::TextureOptions::LINEAR);
                            let tid = tex.id();
                            app.map.textures.insert(id, (tex, frame));
                            Some(tid)
                        }
                        Tile::Loading => {
                            loading += 1;
                            None
                        }
                        Tile::Missing => {
                            missing += 1;
                            None
                        }
                    },
                };
                match tex {
                    Some(tex) => {
                        painter.image(tex, r, Rect::from_min_max(pos2(0.0, 0.0), pos2(1.0, 1.0)), Color32::WHITE);
                    }
                    None => {
                        painter.rect_stroke(r, 0.0, Stroke::new(1.0, Color32::from_gray(72)), StrokeKind::Inside);
                    }
                }
            }
        }
    }
    if app.map.textures.len() > TEXTURES {
        let mut by_age: Vec<(u64, TileId)> = app.map.textures.iter().map(|(k, v)| (v.1, *k)).collect();
        by_age.sort_by_key(|a| a.0);
        for (_, k) in by_age.iter().take(app.map.textures.len() - TEXTURES) {
            app.map.textures.remove(k);
        }
    }
    if loading > 0 {
        ui.ctx().request_repaint_after(std::time::Duration::from_millis(120));
    }
    // ---- saved locations: their circles and names
    for l in locations(app) {
        let (Some(la), Some(lo), Some(rad)) = (l["lat"].as_f64(), l["lon"].as_f64(), l["radius"].as_f64()) else { continue };
        let p = to_screen(area, c, world, map::world_px(la, lo, zoom));
        let rpx = (rad / map::metres_per_px(la, zoom).max(1e-9)).clamp(3.0, 4000.0) as f32;
        // (a strong edge: it must read on light streets and dark water alike)
        painter.circle(p, rpx, Color32::from_rgba_unmultiplied(30, 110, 220, 40), Stroke::new(2.0, Color32::from_rgb(30, 110, 220)));
        let name = l["name"].as_str().unwrap_or("");
        let g = painter.layout_no_wrap(name.to_string(), t.semibold(11.5), Color32::WHITE);
        let br = Rect::from_center_size(p - vec2(0.0, rpx + 12.0), g.size() + vec2(12.0, 6.0));
        painter.rect_filled(br, 4.0, Color32::from_black_alpha(160));
        painter.galley(br.center() - g.size() / 2.0, g, Color32::WHITE);
    }
    // ---- pins, grouped when they would overlap
    let mut clusters: Vec<Cluster> = Vec::new();
    for (id, la, lo) in pins {
        let p = to_screen(area, c, world, map::world_px(*la, *lo, zoom));
        if !area.expand(20.0).contains(p) {
            continue;
        }
        match clusters.iter_mut().find(|k| k.pos.distance(p) < CLUSTER_PX) {
            Some(k) => {
                let n = k.ids.len() as f32;
                k.pos = pos2((k.pos.x * n + p.x) / (n + 1.0), (k.pos.y * n + p.y) / (n + 1.0));
                k.ids.push(*id);
            }
            None => clusters.push(Cluster { pos: p, ids: vec![*id] }),
        }
    }
    let ptr = ui.input(|i| i.pointer.latest_pos());
    let mut hit: Option<usize> = None;
    for (i, k) in clusters.iter().enumerate() {
        let selected = k.ids.iter().any(|id| app.session.selection.contains(lightcraft_catalog::PhotoId(*id)));
        let r = if k.ids.len() > 1 { 13.0 } else { 9.0 };
        let hovered = ptr.is_some_and(|p| p.distance(k.pos) <= r + 3.0) && area.contains(k.pos);
        if hovered {
            hit = Some(i);
        }
        let fill = if selected { t.accent } else { Color32::from_rgb(222, 92, 60) };
        painter.circle(k.pos + vec2(0.0, 1.5), r + 1.0, Color32::from_black_alpha(90), Stroke::NONE);
        painter.circle(k.pos, r, fill, Stroke::new(if hovered { 2.5 } else { 1.5 }, Color32::WHITE));
        if k.ids.len() > 1 {
            painter.text(k.pos, Align2::CENTER_CENTER, k.ids.len().to_string(), t.semibold(10.5), Color32::WHITE);
        }
        // (named after its lowest photo id: stable whatever the sort order)
        let first = k.ids.iter().min().copied().unwrap_or(0);
        register(ui.ctx(), format!("map:pin-{first}"), Rect::from_center_size(k.pos, vec2(r * 2.0, r * 2.0)));
    }
    if let Some(i) = hit
        && let Some(k) = clusters.get(i)
    {
        let n = k.ids.len();
        resp.clone().on_hover_text(crate::i18n::tr_format!("{n} photo{}", if n == 1 { "" } else { "s" }, n = n));
    }
    // ---- clicks on pins pick their photos (click / ⌘-click / ⇧-click)
    if resp.clicked()
        && let Some(k) = hit.and_then(|i| clusters.get(i))
    {
        let m = ui.input(|i| i.modifiers);
        let ids = k.ids.clone();
        let r = if m.command {
            app.run("library.select", json!({"ids": ids, "mode": "toggle"}))
        } else if m.shift {
            app.run("library.select", json!({"ids": [ids.first().copied().unwrap_or(0)], "mode": "range"}))
                .and_then(|_| app.run("library.select", json!({"ids": ids, "mode": "add"})))
        } else {
            app.run("library.select", json!({"ids": ids, "mode": "replace"}))
        };
        if let Err(e) = r {
            app.toast(ui.ctx(), e);
        }
    }
    // ---- photos dropped on the map are placed there
    if let Some(ids) = dragging_photos
        && let Some(p) = ptr.filter(|p| area.contains(*p))
    {
        painter.rect_stroke(area.shrink(1.0), 0.0, Stroke::new(2.0, t.accent), StrokeKind::Inside);
        painter.circle(p, 6.0, Color32::TRANSPARENT, Stroke::new(2.0, t.accent));
        if ui.input(|i| i.pointer.any_released()) {
            let (lat, lon) = to_lat_lon(app, area, p);
            let n = ids.len();
            match app.run("map.geotag", json!({"ids": ids, "lat": lat, "lon": lon})) {
                Ok(_) => app.toast(ui.ctx(), format!("Placed {n} photo{} at {lat:.5}, {lon:.5}", if n == 1 { "" } else { "s" })),
                Err(e) => app.toast(ui.ctx(), e),
            }
            app.ui.dragging_photos = None;
        }
    }
    // ---- the attribution (required) and, when tiles can't load, why
    let attr = map::ATTRIBUTION;
    let g = painter.layout_no_wrap(attr.to_string(), t.font(11.0), Color32::from_gray(30));
    let ar = Rect::from_min_size(area.max - g.size() - vec2(14.0, 10.0), g.size() + vec2(10.0, 6.0));
    painter.rect_filled(ar, 3.0, Color32::from_white_alpha(210));
    painter.galley(ar.min + vec2(5.0, 3.0), g, Color32::from_gray(30));
    register(ui.ctx(), "map:attribution", ar);
    let ar_resp = ui.interact(ar, egui::Id::new("map-attribution"), Sense::click()).on_hover_text(map::COPYRIGHT_URL);
    if ar_resp.clicked()
        && let Err(e) = crate::links::open(app, map::COPYRIGHT_URL)
    {
        app.toast(ui.ctx(), e);
    }
    if missing > 0 {
        let why = store.status()["error"].as_str().map(str::to_string).unwrap_or_else(|| "map tiles can't be loaded".into());
        let text = format!("{why} — pins, geotagging and saved locations still work");
        let g = painter.layout(text, t.font(12.0), t.text, (area.width() - 48.0).clamp(160.0, 520.0));
        let nr = Rect::from_min_size(area.min + vec2(12.0, 12.0), g.size() + vec2(20.0, 14.0));
        painter.rect(nr, 6.0, t.chrome, Stroke::new(1.0, t.divider), StrokeKind::Inside);
        painter.galley(nr.min + vec2(10.0, 7.0), g, t.text);
        register(ui.ctx(), "map:notice", nr);
    }
    if pins.is_empty() {
        let text = crate::i18n::tr("No photos in view have a GPS position: drag photos from the filmstrip onto the map to place them.");
        let g = painter.layout(text.to_string(), t.font(12.5), t.text, (area.width() - 80.0).clamp(160.0, 460.0));
        let nr = Rect::from_center_size(pos2(area.center().x, area.bottom() - 56.0), g.size() + vec2(24.0, 16.0));
        painter.rect(nr, 6.0, t.chrome, Stroke::new(1.0, t.divider), StrokeKind::Inside);
        painter.galley(nr.min + vec2(12.0, 8.0), g, t.text);
    }
}

/// The Map module's left panel: Saved Locations.
pub fn left(app: &mut LightcraftApp, ui: &mut egui::Ui) {
    let t = Tokens::get(ui.ctx());
    let frame = egui::Frame::NONE.fill(t.chrome).stroke(Stroke::new(1.0, t.divider));
    let width = app.ui.left_width;
    let resized = super::resizable_side(ui, true, "left_panel", frame, width, crate::state::LEFT_WIDTH, 0.0, |ui| {
        ui.spacing_mut().item_spacing.y = 0.0;
        let (hr, _) = ui.allocate_exact_size(vec2(ui.available_width(), 40.0), Sense::hover());
        ui.painter().text(
            pos2(hr.left() + 18.0, hr.center().y),
            Align2::LEFT_CENTER,
            crate::i18n::tr("Saved Locations"),
            t.semibold(13.5),
            t.text_label,
        );
        let mut row =
            ui.new_child(egui::UiBuilder::new().max_rect(hr.shrink2(vec2(12.0, 8.0))).layout(egui::Layout::right_to_left(egui::Align::Center)));
        row.spacing_mut().item_spacing.x = 8.0;
        if text_button(&mut row, "mapLocAdd", "add.", false).on_hover_text(crate::i18n::tr("Save the place at the centre of the map")).clicked()
            && let Err(e) = app.run("dialog.mapSaveLocation", json!({}))
        {
            app.toast(ui.ctx(), e);
        }
        if text_button(&mut row, "mapLocClear", "clr.", false)
            .on_hover_text(crate::i18n::tr("Show all photos again (clear the location filter)"))
            .clicked()
            && let Err(e) = app.run("library.clearFilter", json!({}))
        {
            app.toast(ui.ctx(), e);
        }
        ui.painter().hline(hr.x_range(), hr.bottom(), Stroke::new(1.0, t.divider));
        let list = locations(app);
        egui::ScrollArea::vertical().id_salt("map-locations").auto_shrink([false, false]).show(ui, |ui| {
            ui.add_space(6.0);
            if list.is_empty() {
                let (r, _) = ui.allocate_exact_size(vec2(ui.available_width(), 64.0), Sense::hover());
                let g = ui.painter().layout(
                    crate::i18n::tr("No saved locations yet: move the map to a place and press add.").to_string(),
                    t.font(12.0),
                    t.text_dim,
                    r.width() - 36.0,
                );
                ui.painter().galley(r.min + vec2(18.0, 8.0), g, t.text_dim);
            }
            for l in &list {
                location_row(app, ui, l, &t);
            }
        });
    });
    if let Some(w) = resized {
        app.ui.left_width = w;
    }
}

fn location_row(app: &mut LightcraftApp, ui: &mut egui::Ui, l: &Value, t: &Tokens) {
    let id = l["id"].as_u64().unwrap_or(0);
    let (r, resp) = ui.allocate_exact_size(vec2(ui.available_width(), 34.0), Sense::click());
    register(ui.ctx(), format!("mapLocation:{id}"), r);
    if resp.hovered() {
        ui.painter().rect_filled(r, 0.0, t.hover);
    }
    let name = l["name"].as_str().unwrap_or("");
    let n = l["photos"].as_u64().unwrap_or(0);
    let private = l["private"].as_bool().unwrap_or(false);
    // the filter chip on the right, the count before it, the name in what's left
    let chip = Rect::from_min_size(pos2(r.right() - 12.0 - 46.0, r.center().y - 12.0), vec2(46.0, 24.0));
    let count = ui.painter().text(pos2(chip.left() - 10.0, r.center().y), Align2::RIGHT_CENTER, n.to_string(), t.font(12.0), t.text_dim);
    let max_name = (count.left() - 10.0 - (r.left() + 18.0)).max(20.0);
    let shown = if private { format!("{name} (private)") } else { name.to_string() };
    let g = ui.painter().layout(shown, t.font(13.0), t.text_label, max_name);
    ui.painter().galley(pos2(r.left() + 18.0, r.center().y - g.size().y / 2.0), g, t.text_label);
    let cr = ui.put(chip, |ui: &mut egui::Ui| text_button(ui, &format!("mapLocShow-{id}"), "show.", false));
    if cr.on_hover_text(crate::i18n::tr("Show only the photos in this location")).clicked()
        && let Err(e) = app.run("map.photosIn", json!({"id": id}))
    {
        app.toast(ui.ctx(), e);
    }
    if resp.clicked()
        && let (Some(la), Some(lo), Some(rad)) = (l["lat"].as_f64(), l["lon"].as_f64(), l["radius"].as_f64())
    {
        // centre on it, zoomed so its circle takes about a third of the view
        let w = if app.map.width > 10.0 { app.map.width as f64 } else { 800.0 };
        let z0 = map::metres_per_px(la, 0.0);
        let want = (rad * 2.0 * 3.0 / w).max(1e-6);
        let zoom = (z0 / want).log2().clamp(1.0, 18.0);
        let _ = app.run("view.map", json!({"lat": la, "lon": lo, "zoom": zoom}));
    }
    resp.context_menu(|ui| {
        if ui.button(crate::i18n::tr("Show Photos Here")).clicked() {
            let _ = app.run("map.photosIn", json!({"id": id}));
            ui.close();
        }
        if ui.button(crate::i18n::tr(if private { "Make Not Private" } else { "Make Private" })).clicked() {
            let _ = app.run("map.saveLocation", json!({"id": id, "private": !private}));
            ui.close();
        }
        if ui.button(crate::i18n::tr("Rename…")).clicked() {
            crate::panels::dialogs::prompt(app, "Rename Location", "Name", name, "map.saveLocation", json!({"id": id}), "name");
            ui.close();
        }
        if ui.button(crate::i18n::tr("Delete Location")).clicked() {
            let _ = app.run("map.deleteLocation", json!({"id": id}));
            ui.close();
        }
    });
}
