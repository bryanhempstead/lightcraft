//! LR-VID-PLAY / TRIM / COVER / EDIT: the Loupe's video controls, as Lightroom Classic's — a
//! bar under the clip with play./pause., the time, a scrubber (drag to seek), trim. (in / out
//! handles on the scrubber), poster. (this frame becomes the thumbnail), cap. (Capture Frame:
//! a JPEG next to the clip, stacked with it) and snd. (sound on / off). Space plays and pauses.
//!
//! Frames come from [`lightcraft_engine::video`]: a [`Player`] while playing, a single frame
//! fetched in the background while scrubbing paused; both with the clip's edits when it has any.
//! When nothing has been played or scrubbed the Loupe shows its normal render: the poster frame
//! through the develop pipeline.

use std::sync::Arc;
use std::sync::mpsc::{Receiver, channel};

use egui::{Color32, CornerRadius, Pos2, Rect, Sense, Stroke, StrokeKind, pos2, vec2};
use lightcraft_catalog::{MediaKind, Photo, PhotoId, Source};
use lightcraft_engine::video::{self, Player, VideoInfo};
use serde_json::{Value, json};

use crate::LightcraftApp;
use crate::theme::Tokens;
use crate::widgets::register;

/// Longest edge of playback frames (decoding and developing more is wasted on a loupe).
const PLAY_EDGE: f64 = 1280.0;
/// Longest edge of playback frames with draft quality off.
const FULL_PLAY_EDGE: f64 = 3840.0;
/// Longest edge of a scrubbed still.
const STILL_EDGE: f64 = 1600.0;
/// The bar's height and its distance from the canvas edges.
const BAR_H: f32 = 36.0;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Grab {
    Playhead,
    In,
    Out,
}

/// The Loupe's video state (one clip at a time).
#[derive(Default)]
pub struct VideoView {
    id: Option<PhotoId>,
    info: Option<Result<VideoInfo, String>>,
    probing: Option<Receiver<Result<VideoInfo, String>>>,
    /// Playhead (seconds into the clip).
    pub pos: f64,
    pub playing: bool,
    /// Wall clock (egui time) and playhead when playing (re)started.
    origin: (f64, f64),
    player: Option<Player>,
    /// What the canvas shows instead of the render: a played or scrubbed frame (its time).
    tex: Option<(egui::TextureHandle, f64)>,
    still: Option<(u64, Receiver<Result<lightcraft_raster::Rgba8, String>>)>,
    still_gen: u64,
    /// The playhead moved away from the poster (play or scrub): show `tex`, not the render.
    moved: bool,
    grab: Option<Grab>,
    /// Trim handles shown on the scrubber.
    pub trim_mode: bool,
    /// Sound off (snd.).
    pub muted: bool,
    pub error: Option<String>,
}

impl VideoView {
    /// Drop the shown frame's texture (the screen's colour space changed: it is made again).
    pub fn forget_textures(&mut self) {
        self.tex = None;
    }

    fn reset(&mut self, id: Option<PhotoId>) {
        *self = VideoView { trim_mode: self.trim_mode, muted: self.muted, ..Default::default() };
        self.id = id;
    }
    /// The clip's length: the catalog's, else the probe's.
    fn duration(&self, p: &Photo) -> f64 {
        p.duration
            .or_else(|| self.info.as_ref().and_then(|i| i.as_ref().ok()).map(|i| i.duration))
            .filter(|d| d.is_finite() && *d > 0.0)
            .unwrap_or(0.0)
    }
}

fn path_of(p: &Photo) -> Option<String> {
    match &p.source {
        Source::File { path } => Some(path.clone()),
        Source::Demo { .. } => None,
    }
}

/// The bar's `position / length`: `m:ss.cc`, or `m:ss:ff` with View Options ▸ Show frame number
/// when displaying video time (frames at the clip's rate).
pub(crate) fn time_label(app: &LightcraftApp, pos: f64, dur: f64) -> String {
    use lightcraft_engine::cmd::{video_frame_timecode, video_timecode};
    if app.ui.settings.develop_view.video_frames {
        let fps = app.video.info.as_ref().and_then(|i| i.as_ref().ok()).map_or(25.0, |i| i.fps);
        format!("{} / {}", video_frame_timecode(pos, fps), video_frame_timecode(dur, fps))
    } else {
        format!("{} / {}", video_timecode(pos), video_timecode(dur))
    }
}

/// Size of a frame that fits `edge` for the clip (even, for decoders).
fn frame_size(p: &Photo, edge: f64) -> (u32, u32) {
    video::fit_even(p.width.max(2), p.height.max(2), edge as u32)
}

/// The active photo is a video shown in the Loupe (Space plays it).
pub fn wants_space(app: &LightcraftApp) -> bool {
    app.ui.view == crate::state::ViewMode::Detail && active_video(app).is_some()
}

fn active_video(app: &LightcraftApp) -> Option<Arc<Photo>> {
    let id = app.session.active()?;
    app.session.catalog.photo(id).filter(|p| p.kind == MediaKind::Video).cloned()
}

/// Follow the active photo: a new clip starts paused at its in point, probed in the background.
fn sync(app: &mut LightcraftApp, p: &Photo) {
    if app.video.id == Some(p.id) {
        return;
    }
    app.video.reset(Some(p.id));
    app.video.pos = p.video.unwrap_or_default().poster.unwrap_or(0.0);
    if let Some(path) = path_of(p) {
        let (tx, rx) = channel();
        let spawned = std::thread::Builder::new().name("lc-video-probe".into()).spawn(move || {
            let _ = tx.send(video::probe(&path));
        });
        match spawned {
            Ok(_) => app.video.probing = Some(rx),
            Err(e) => app.video.info = Some(Err(e.to_string())),
        }
    }
}

fn poll(app: &mut LightcraftApp, ctx: &egui::Context, p: &Photo) {
    let p3 = app.renderer.display_p3;
    let v = &mut app.video;
    if let Some(rx) = &v.probing {
        match rx.try_recv() {
            Ok(r) => {
                v.info = Some(r);
                v.probing = None;
            }
            Err(std::sync::mpsc::TryRecvError::Empty) => ctx.request_repaint_after(std::time::Duration::from_millis(50)),
            Err(_) => v.probing = None,
        }
    }
    let dur = v.duration(p);
    let (a, b) = p.video.unwrap_or_default().range(Some(dur).filter(|d| *d > 0.0));
    let end = b.unwrap_or(dur);
    if v.playing {
        let now = ctx.input(|i| i.time);
        v.pos = (v.origin.1 + (now - v.origin.0)).max(a);
        if let Some(pl) = v.player.as_mut() {
            if let Some(f) = pl.frame_at(v.pos) {
                let img = crate::display_p3::screen_image(p3, [f.image.width, f.image.height], &f.image.as_bytes());
                match v.tex.as_mut() {
                    Some((t, at)) => {
                        t.set(img, egui::TextureOptions::LINEAR);
                        *at = f.t;
                    }
                    None => v.tex = Some((ctx.load_texture("video-frame", img, egui::TextureOptions::LINEAR), f.t)),
                }
            }
            if let Some(e) = pl.error() {
                v.error = Some(e);
            }
            if pl.finished() && v.pos >= end - 0.05 {
                v.playing = false;
                v.player = None;
                v.pos = end;
            }
        }
        if v.pos >= end + 0.5 {
            // (a stream that ended early) stop at the out point
            v.playing = false;
            v.player = None;
            v.pos = end;
        }
        ctx.request_repaint();
    }
    if let Some((generation, rx)) = &v.still {
        match rx.try_recv() {
            Ok(Ok(img)) => {
                let at = *generation;
                let ci = crate::display_p3::screen_image(p3, [img.width, img.height], &img.as_bytes());
                if at == v.still_gen {
                    match v.tex.as_mut() {
                        Some((t, _)) => t.set(ci, egui::TextureOptions::LINEAR),
                        None => v.tex = Some((ctx.load_texture("video-frame", ci, egui::TextureOptions::LINEAR), v.pos)),
                    }
                    if let Some(t) = v.tex.as_mut() {
                        t.1 = v.pos;
                    }
                }
                v.still = None;
            }
            Ok(Err(e)) => {
                v.error = Some(e);
                v.still = None;
            }
            Err(std::sync::mpsc::TryRecvError::Empty) => ctx.request_repaint_after(std::time::Duration::from_millis(30)),
            Err(_) => v.still = None,
        }
    }
}

/// Fetch the frame at the playhead (paused scrubbing), latest request wins.
fn fetch_still(app: &mut LightcraftApp, p: &Photo) {
    let Some(path) = path_of(p) else { return };
    let (t, size) = (app.video.pos, frame_size(p, STILL_EDGE));
    let develop = video::has_edits(&p.develop).then(|| p.develop.clone());
    app.video.still_gen += 1;
    let generation = app.video.still_gen;
    let (tx, rx) = channel();
    let spawned = std::thread::Builder::new().name("lc-video-still".into()).spawn(move || {
        let r = video::frame_rgba(&path, t, size.0.max(size.1) as usize, develop.as_deref(), video::playback_quality());
        let _ = tx.send(r);
    });
    if spawned.is_ok() {
        app.video.still = Some((generation, rx));
    }
}

/// Start (or restart, after a seek) playback from the playhead to the out point.
fn start(app: &mut LightcraftApp, ctx: &egui::Context, p: &Photo) -> Result<(), String> {
    let path = path_of(p).ok_or("this video has no file")?;
    let info = match &app.video.info {
        Some(Ok(i)) => i.clone(),
        Some(Err(e)) => return Err(e.clone()),
        None => return Err("still reading the video…".into()),
    };
    let dur = app.video.duration(p);
    let (a, b) = p.video.unwrap_or_default().range(Some(dur).filter(|d| *d > 0.0));
    let end = b.unwrap_or(dur);
    // from the in point when at (or past) the end, or before the in point
    if app.video.pos >= end - 0.05 || app.video.pos < a {
        app.video.pos = a;
    }
    let develop = video::has_edits(&p.develop).then(|| p.develop.clone());
    // View Options ▸ Play HD video at draft quality (off: frames up to 4K)
    let edge = if app.ui.settings.develop_view.video_draft { PLAY_EDGE } else { FULL_PLAY_EDGE };
    let player = Player::start(&path, &info, app.video.pos, end, frame_size(p, edge), develop, !app.video.muted)?;
    app.video.player = Some(player);
    app.video.playing = true;
    app.video.moved = true;
    app.video.origin = (ctx.input(|i| i.time), app.video.pos);
    app.video.error = None;
    ctx.request_repaint();
    Ok(())
}

fn pause(app: &mut LightcraftApp) {
    app.video.playing = false;
    app.video.player = None;
}

/// Move the playhead (a paused clip fetches that frame; a playing one restarts there).
fn seek(app: &mut LightcraftApp, ctx: &egui::Context, p: &Photo, t: f64) {
    let dur = app.video.duration(p);
    app.video.pos = if dur > 0.0 { t.clamp(0.0, dur) } else { t.max(0.0) };
    app.video.moved = true;
    if app.video.playing {
        let _ = start(app, ctx, p);
    } else {
        fetch_still(app, p);
    }
}

/// The UI's video commands (`video.playPause`, `video.seek`, `video.trimMode`, `video.posterHere`,
/// `video.captureHere`, `video.sound`), on the active clip.
pub fn command(app: &mut LightcraftApp, id: &str, p: &Value) -> Result<Value, String> {
    let photo = active_video(app).ok_or("the active photo isn't a video")?;
    sync(app, &photo);
    let ctx = app.tasks.repaint.clone().unwrap_or_default();
    match id {
        "video.playPause" => {
            if app.video.playing {
                pause(app);
            } else {
                // the probe may still be running: wait for it a moment (control channel, tests)
                if app.video.info.is_none()
                    && let Some(rx) = app.video.probing.take()
                {
                    app.video.info = rx.recv_timeout(std::time::Duration::from_secs(20)).ok();
                }
                start(app, &ctx, &photo)?;
            }
            Ok(json!({"playing": app.video.playing, "pos": app.video.pos}))
        }
        "video.seek" => {
            let t = p.get("time").and_then(lightcraft_engine::cmd::video_seconds).ok_or("video.seek: `time` (seconds or m:ss)")?;
            seek(app, &ctx, &photo, t);
            Ok(json!({"pos": app.video.pos}))
        }
        "video.trimMode" => {
            app.video.trim_mode = p.get("on").and_then(Value::as_bool).unwrap_or(!app.video.trim_mode);
            Ok(json!({"trimMode": app.video.trim_mode}))
        }
        "video.sound" => {
            app.video.muted = !p.get("on").and_then(Value::as_bool).unwrap_or(app.video.muted);
            Ok(json!({"sound": !app.video.muted}))
        }
        "video.posterHere" => {
            let r = app.run("video.setPoster", json!({"id": photo.id.0, "time": app.video.pos}))?;
            app.video.moved = false;
            app.toast_soon(format!("Poster frame set at {}", lightcraft_engine::cmd::video_timecode(app.video.pos)), 3.0);
            Ok(r)
        }
        "video.captureHere" => {
            pause(app);
            let r = app.run("video.captureFrame", json!({"id": photo.id.0, "time": app.video.pos}))?;
            let name = r["path"].as_str().and_then(|s| s.rsplit('/').next()).unwrap_or("").to_string();
            app.toast_soon(format!("Captured frame {} as {name}", lightcraft_engine::cmd::video_timecode(app.video.pos)), 4.0);
            Ok(r)
        }
        _ => Err(format!("unknown video command {id}")),
    }
}

/// The video overlay of the Loupe: the played / scrubbed frame over the render, and the control
/// bar. `img` is where the photo is drawn.
pub fn loupe(app: &mut LightcraftApp, ui: &mut egui::Ui, canvas: Rect, img: Rect, p: &Photo) {
    sync(app, p);
    let ctx = ui.ctx().clone();
    poll(app, &ctx, p);
    let t = Tokens::get(&ctx);
    let painter = ui.painter_at(canvas);
    if app.video.moved
        && let Some((tex, _)) = &app.video.tex
    {
        let r = crate::panels::detail::fit_texture_rect(img, [tex.size()[0], tex.size()[1]]);
        painter.rect_filled(img, 0.0, t.canvas);
        painter.image(tex.id(), r, Rect::from_min_max(pos2(0.0, 0.0), pos2(1.0, 1.0)), Color32::WHITE);
    }
    let dur = app.video.duration(p);
    let edit = p.video.unwrap_or_default();
    let (a, b) = edit.range(Some(dur).filter(|d| *d > 0.0));
    let end = b.unwrap_or(dur);

    // the bar: under the clip, inside the canvas, as wide as the clip (at least 520 px)
    let w = img.width().clamp(520.0f32.min(canvas.width() - 32.0), canvas.width() - 32.0);
    let y = (img.bottom() + 12.0).min(canvas.bottom() - BAR_H - 12.0);
    let bar = Rect::from_min_size(pos2(canvas.center().x - w / 2.0, y), vec2(w, BAR_H));
    painter.rect(bar, CornerRadius::same(6), t.chrome.gamma_multiply(0.92), Stroke::new(1.0, t.divider), StrokeKind::Inside);
    register(&ctx, "video:bar", bar);

    let mut clicked: Option<&str> = None;
    let mut child =
        ui.new_child(egui::UiBuilder::new().max_rect(bar.shrink2(vec2(8.0, 6.0))).layout(egui::Layout::left_to_right(egui::Align::Center)));
    child.spacing_mut().item_spacing.x = 8.0;
    let play_label = if app.video.playing { "pause." } else { "play." };
    if bar_button(&mut child, "videoPlay", play_label, app.video.playing).clicked() {
        clicked = Some("video.playPause");
    }
    let time = time_label(app, app.video.pos, dur);
    child.label(egui::RichText::new(time).font(t.font(11.5)).color(t.text_label).monospace());
    // the right-hand buttons are laid out first (right to left) so the scrubber takes what's left
    let rest = child.available_rect_before_wrap();
    let mut right = child.new_child(egui::UiBuilder::new().max_rect(rest).layout(egui::Layout::right_to_left(egui::Align::Center)));
    right.spacing_mut().item_spacing.x = 8.0;
    let snd = if app.video.muted { "mute." } else { "snd." };
    if bar_button(&mut right, "videoSound", snd, app.video.muted).on_hover_text(crate::i18n::tr("Sound on / off (needs ffplay)")).clicked() {
        clicked = Some("video.sound");
    }
    if bar_button(&mut right, "videoCapture", "cap.", false)
        .on_hover_text(crate::i18n::tr("Capture Frame: this frame as a JPEG, stacked with the video"))
        .clicked()
    {
        clicked = Some("video.captureHere");
    }
    if bar_button(&mut right, "videoPoster", "poster.", false)
        .on_hover_text(crate::i18n::tr("Set Poster Frame: this frame becomes the thumbnail"))
        .clicked()
    {
        clicked = Some("video.posterHere");
    }
    if bar_button(&mut right, "videoTrim", "trim.", app.video.trim_mode).on_hover_text(crate::i18n::tr("Trim: drag the in and out handles")).clicked()
    {
        clicked = Some("video.trimMode");
    }
    let used = right.min_rect();
    let track = Rect::from_min_max(pos2(rest.left() + 4.0, bar.center().y - 4.0), pos2(used.left() - 12.0, bar.center().y + 4.0));
    scrubber(app, ui, &ctx, p, track, dur, (a, end), edit.poster.unwrap_or(0.0));
    if let Some(e) = app.video.error.clone().or_else(|| app.video.info.as_ref().and_then(|i| i.as_ref().err().cloned())) {
        let g = painter.layout(e, t.font(11.5), t.text, (w - 24.0).max(120.0));
        let r = Rect::from_min_size(pos2(bar.left(), bar.top() - g.size().y - 18.0), vec2(w, g.size().y + 12.0));
        painter.rect_filled(r, 6.0, t.chrome.gamma_multiply(0.92));
        painter.galley(r.min + vec2(12.0, 6.0), g, t.text);
    }
    if let Some(cmd) = clicked {
        if let Err(e) = command(app, cmd, &json!({})) {
            app.video.error = Some(e);
        }
        ctx.request_repaint();
    }
}

#[allow(clippy::too_many_arguments)]
fn scrubber(app: &mut LightcraftApp, ui: &mut egui::Ui, ctx: &egui::Context, p: &Photo, track: Rect, dur: f64, (a, end): (f64, f64), poster: f64) {
    if track.width() < 24.0 {
        return;
    }
    let t = Tokens::get(ctx);
    let x_of = |s: f64| track.left() + (if dur > 0.0 { (s / dur).clamp(0.0, 1.0) } else { 0.0 }) as f32 * track.width();
    let s_of = |x: f32| ((x - track.left()) / track.width()).clamp(0.0, 1.0) as f64 * dur;
    let hit = track.expand2(vec2(8.0, 10.0));
    let resp = ui.interact(hit, egui::Id::new("video-scrubber"), Sense::click_and_drag());
    register(ctx, "video:scrubber", track);
    let painter = ui.painter();
    painter.rect_filled(track, 4.0, t.track);
    // the trimmed range
    let kept = Rect::from_min_max(pos2(x_of(a), track.top()), pos2(x_of(end), track.bottom()));
    painter.rect_filled(kept, 4.0, t.accent.gamma_multiply(0.55));
    // the poster frame: a small mark above the track
    let px = x_of(poster);
    painter.line_segment([pos2(px, track.top() - 5.0), pos2(px, track.top() - 1.0)], Stroke::new(2.0, t.text_dim));
    // in / out handles (trim.)
    let handle = |x: f32| Rect::from_center_size(pos2(x, track.center().y), vec2(8.0, 20.0));
    if app.video.trim_mode {
        for (which, x) in [("in", x_of(a)), ("out", x_of(end))] {
            let r = handle(x);
            painter.rect(r, CornerRadius::same(2), t.text, Stroke::new(1.0, t.chrome), StrokeKind::Inside);
            register(ctx, format!("video:{which}"), r);
        }
    }
    // the playhead
    let hx = x_of(app.video.pos);
    painter.line_segment([pos2(hx, track.top() - 7.0), pos2(hx, track.bottom() + 7.0)], Stroke::new(2.0, t.text));
    register(ctx, "video:playhead", Rect::from_center_size(pos2(hx, track.center().y), vec2(4.0, 22.0)));

    let at: Option<Pos2> = resp.interact_pointer_pos();
    if resp.drag_started() || (resp.clicked() && app.video.grab.is_none()) {
        app.video.grab = at.map(|pt| {
            if app.video.trim_mode && handle(x_of(a)).expand(4.0).contains(pt) {
                Grab::In
            } else if app.video.trim_mode && handle(x_of(end)).expand(4.0).contains(pt) {
                Grab::Out
            } else {
                Grab::Playhead
            }
        });
    }
    if let (Some(g), Some(pt)) = (app.video.grab, at)
        && (resp.dragged() || resp.clicked() || resp.drag_started())
    {
        let s = s_of(pt.x);
        match g {
            Grab::Playhead => seek(app, ctx, p, s),
            // the handles move live (shown), and commit once when let go
            Grab::In | Grab::Out => {
                app.video.pos = s;
                app.video.moved = true;
            }
        }
    }
    if (resp.drag_stopped() || resp.clicked())
        && let Some(g) = app.video.grab.take()
    {
        let s = app.video.pos;
        let r = match g {
            Grab::In => app.run("video.setTrim", json!({"id": p.id.0, "in": s})),
            Grab::Out => app.run("video.setTrim", json!({"id": p.id.0, "out": s})),
            Grab::Playhead => Ok(Value::Null),
        };
        match r {
            Ok(_) if g != Grab::Playhead => fetch_still(app, p),
            Ok(_) => {}
            Err(e) => app.video.error = Some(e),
        }
    }
    if app.video.grab.is_some() && app.video.grab != Some(Grab::Playhead) {
        // the live handle while dragging: the playhead stands in for it
        ctx.request_repaint();
    }
}

/// A bar button that never keeps keyboard focus: Space must reach play / pause, not "click" the
/// button last clicked (which played and paused at once).
fn bar_button(ui: &mut egui::Ui, id: &str, label: &str, active: bool) -> egui::Response {
    let r = crate::widgets::text_button(ui, id, label, active);
    if r.has_focus() || r.gained_focus() {
        r.surrender_focus();
    }
    r
}

/// The duration badge a grid / filmstrip cell shows on a video (`m:ss`).
pub fn badge(p: &Photo) -> Option<String> {
    if p.kind != MediaKind::Video {
        return None;
    }
    let d = p.duration.filter(|d| d.is_finite() && *d >= 0.0).unwrap_or(0.0);
    let s = d.round() as u64;
    Some(if s >= 3600 { format!("{}:{:02}:{:02}", s / 3600, (s / 60) % 60, s % 60) } else { format!("{}:{:02}", s / 60, s % 60) })
}
