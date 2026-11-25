//! Keyboard shortcuts: parse `Cmd+Shift+X` style strings and dispatch UI and engine commands.

use egui::{Key, Modifiers};
use serde_json::json;

use crate::LightcraftApp;
use crate::keymap::{Combo, Trigger};

/// Secondary key bindings for commands that already exist: `(shortcut, command id, params JSON)`.
/// They complement the primary shortcut declared on the command (Lightroom-desktop keys that our
/// primary keymap assigns elsewhere, see docs/parity.md → Shortcuts). Shown in Help → Keyboard Shortcuts.
pub const ALIASES: &[(&str, &str, &str)] = &[
    ("Cmd+D", "library.selectNone", "{}"),
    // Lightroom Classic's panel keys (F5 top bar, F8 right panel and Tab are primary)
    ("F6", "view.filmstrip", "{}"),
    ("F7", "view.leftPanel", "{}"),
    ("Shift+E", "dialog.export", "{}"),
    // ⌘E is the external editor (Bryan 11-23); Export with Previous stays ⌘⌥⇧E
    ("Space", "view.zoomToggle", "{}"),
    ("Shift+M", "version.create", "{}"),
    ("Shift+X", "photo.flag", r#"{"flag": "reject", "advance": true}"#),
    ("Shift+Z", "photo.flag", r#"{"flag": "pick", "advance": true}"#),
    ("Shift+U", "photo.flag", r#"{"flag": "none", "advance": true}"#),
    ("Shift+6", "photo.label", r#"{"label": "red", "advance": true}"#),
    ("Shift+7", "photo.label", r#"{"label": "yellow", "advance": true}"#),
    ("Shift+8", "photo.label", r#"{"label": "green", "advance": true}"#),
    ("Shift+9", "photo.label", r#"{"label": "blue", "advance": true}"#),
    // Shift+[ / Shift+] arrive as { / } on most layouts
    ("Shift+{", "brush.featherLess", "{}"),
    ("Shift+}", "brush.featherMore", "{}"),
    // keyword set: ⌥1–⌥9 toggle the current set's keywords on the selection
    ("Alt+1", "keyword.toggleFromSet", r#"{"index": 1}"#),
    ("Alt+2", "keyword.toggleFromSet", r#"{"index": 2}"#),
    ("Alt+3", "keyword.toggleFromSet", r#"{"index": 3}"#),
    ("Alt+4", "keyword.toggleFromSet", r#"{"index": 4}"#),
    ("Alt+5", "keyword.toggleFromSet", r#"{"index": 5}"#),
    ("Alt+6", "keyword.toggleFromSet", r#"{"index": 6}"#),
    ("Alt+7", "keyword.toggleFromSet", r#"{"index": 7}"#),
    ("Alt+8", "keyword.toggleFromSet", r#"{"index": 8}"#),
    ("Alt+9", "keyword.toggleFromSet", r#"{"index": 9}"#),
];

pub fn parse(s: &str) -> Option<(Modifiers, Key)> {
    let mut m = Modifiers::NONE;
    let mut key = None;
    for part in s.split('+') {
        match part {
            "Cmd" => m.command = true,
            "Shift" => m.shift = true,
            "Alt" => m.alt = true,
            "Ctrl" => m.ctrl = true,
            // "Delete" means the key labelled ⌫ (egui's Backspace); forward-delete also matches, see `matches`.
            "Delete" => key = Some(Key::Backspace),
            k => {
                key = Key::from_name(k).or(match k {
                    "Right" => Some(Key::ArrowRight),
                    "Left" => Some(Key::ArrowLeft),
                    "Up" => Some(Key::ArrowUp),
                    "Down" => Some(Key::ArrowDown),
                    "\\" => Some(Key::Backslash),
                    "/" => Some(Key::Slash),
                    "=" => Some(Key::Equals),
                    "-" => Some(Key::Minus),
                    "[" => Some(Key::OpenBracket),
                    "]" => Some(Key::CloseBracket),
                    "'" => Some(Key::Quote),
                    "," => Some(Key::Comma),
                    _ => None,
                })
            }
        }
    }
    key.map(|k| (m, k))
}

fn matches(i: &egui::InputState, m: Modifiers, k: Key) -> bool {
    i.events.iter().any(|e| match e {
        egui::Event::Key { key, physical_key, pressed: true, modifiers, .. } => {
            // `Ctrl` is the physical Control key (on macOS distinct from Cmd; elsewhere Cmd = Ctrl);
            // "Delete" (⌫ = Backspace) also matches forward-delete
            let ctrl_ok = if m.ctrl { modifiers.ctrl } else { !modifiers.ctrl || modifiers.command };
            let cmd_ok = m.ctrl || modifiers.command == m.command;
            // winit can report shifted digits as punctuation (Shift+1 = `!`). Recognize the
            // physical number key for culling, without changing layout-aware letter shortcuts.
            let shifted_digit = m.shift
                && *physical_key == Some(k)
                && matches!(k, Key::Num0 | Key::Num1 | Key::Num2 | Key::Num3 | Key::Num4 | Key::Num5 | Key::Num6 | Key::Num7 | Key::Num8 | Key::Num9);
            (*key == k || shifted_digit || (k == Key::Backspace && *key == Key::Delete))
                && ctrl_ok
                && cmd_ok
                && modifiers.shift == m.shift
                && modifiers.alt == m.alt
        }
        _ => false,
    })
}

/// The combo `c` was pressed (or its mouse button clicked) this frame.
fn combo_pressed(i: &egui::InputState, c: &Combo) -> bool {
    match c.trigger {
        Trigger::Key(k) => matches(i, c.modifiers(), k),
        Trigger::Mouse(b) => i.events.iter().any(|e| match e {
            egui::Event::PointerButton { button, pressed: true, modifiers, .. } => *button == b && Combo::from_modifiers(*modifiers, c.trigger) == *c,
            _ => false,
        }),
    }
}

/// Settings ▸ shrt. is recording a combo: the first key press (or mouse back/forward/middle) this
/// frame, Escape = cancel. Modifier-only presses don't arrive as key events.
fn record(app: &mut LightcraftApp, ctx: &egui::Context) -> bool {
    if app.keymap.recording.is_none() {
        return false;
    }
    let got = ctx.input(|i| {
        i.events.iter().find_map(|e| match e {
            egui::Event::Key { key, pressed: true, modifiers, .. } => Some(Combo::from_modifiers(*modifiers, Trigger::Key(*key))),
            egui::Event::PointerButton {
                button: b @ (egui::PointerButton::Extra1 | egui::PointerButton::Extra2 | egui::PointerButton::Middle),
                pressed: true,
                modifiers,
                ..
            } => Some(Combo::from_modifiers(*modifiers, Trigger::Mouse(*b))),
            _ => None,
        })
    });
    let Some(c) = got else { return true };
    let target = app.keymap.recording.take();
    // the keys were for the recorder, not for a command (nor the dialog: Escape only cancels)
    ctx.input_mut(|i| i.events.retain(|e| !matches!(e, egui::Event::Key { .. })));
    if c.trigger == Trigger::Key(Key::Escape) && !c.cmd && !c.alt && !c.shift && !c.ctrl {
        return true;
    }
    let keys = crate::keymap::format_combo(&c);
    let r = crate::panels::settings::recorded(app, target.flatten(), &keys);
    app.toast(
        ctx,
        match r {
            Ok(()) => format!("{} recorded", crate::keymap::display(&keys)),
            Err(e) => e,
        },
    );
    true
}

/// A keyboard layer that puts Control where ⌘ is (Bryan's NEBULA68B layer 1, 2026-11-23: "cmd z
/// doesn't work on layer 1"): Ctrl+key runs what ⌘+key runs — when Ctrl+key has no binding of its own.
/// Returns whether it ran something (the key is used up).
fn ctrl_as_cmd(app: &mut LightcraftApp, ctx: &egui::Context) -> bool {
    let Some((key, m)) = ctx.input(|i| {
        i.events.iter().find_map(|e| match e {
            egui::Event::Key { key, pressed: true, repeat: false, modifiers, .. } if modifiers.ctrl && !modifiers.mac_cmd => Some((*key, *modifiers)),
            _ => None,
        })
    }) else {
        return false;
    };
    let mods_match = |p: Modifiers, ctrl: bool| {
        p.alt == m.alt && p.shift == m.shift && if ctrl { p.ctrl && !p.mac_cmd } else { p.mac_cmd || (p.command && !p.ctrl) }
    };
    let is = |sc: &str, ctrl: bool| parse(sc).is_some_and(|(p, k)| k == key && mods_match(p, ctrl));
    let ctrl_combo = Combo { cmd: false, ctrl: true, alt: m.alt, shift: m.shift, trigger: Trigger::Key(key) };
    let overlay = app.keymap.overlay().to_vec();
    // Ctrl+key of its own (the user's / profile's keys, a menu item, a command): leave it alone
    if crate::keymap::lookup(&overlay, &ctrl_combo).is_some()
        || crate::menus::ui_commands().any(|(_, _, sc, _)| sc.is_some_and(|s| is(s, true)))
        || lightcraft_engine::command_specs().iter().any(|c| c.shortcut.is_some_and(|s| is(s, true)))
    {
        return false;
    }
    let cmd_combo = Combo { cmd: true, ctrl: false, alt: m.alt, shift: m.shift, trigger: Trigger::Key(key) };
    let run = match crate::keymap::lookup(&overlay, &cmd_combo) {
        Some(Some(b)) => Some((b.command.clone(), b.params.clone())),
        Some(None) => None,
        None => crate::menus::ui_commands()
            .find(|(_, _, sc, _)| sc.is_some_and(|s| is(s, false)))
            .map(|(id, _, _, _)| (id.to_string(), json!({})))
            .or_else(|| {
                lightcraft_engine::command_specs().iter().find(|c| c.shortcut.is_some_and(|s| is(s, false))).map(|c| (c.id.to_string(), json!({})))
            }),
    };
    let Some((cmd, params)) = run else { return false };
    ctx.input_mut(|i| i.consume_key(m, key));
    dispatch(app, ctx, &cmd, params);
    true
}

/// How long H is held before it does what H is bound to instead of toggling fav.
pub const H_HOLD: std::time::Duration = std::time::Duration::from_millis(400);

/// H (no modifiers): a quick tap toggles fav. (the heart); held for [`H_HOLD`] it does exactly what
/// H did before — the press is replayed through the normal shortcuts (in Masking: the pins; else
/// the Remove tool; or the user's own binding, Bryan's Hue panel). Bryan, 2026-11-23: "a single
/// quick click of h with a heart, but if I click and hold it does what it did before".
fn tap_or_hold_h(app: &mut LightcraftApp, ctx: &egui::Context) {
    if app.h_replay {
        // the replayed press: let the shortcuts below have it
        app.h_replay = false;
        return;
    }
    let (down, pressed) = ctx.input(|i| {
        let fresh = i
            .events
            .iter()
            .any(|e| matches!(e, egui::Event::Key { key: Key::H, pressed: true, repeat: false, modifiers, .. } if modifiers.is_none()));
        (i.key_down(Key::H) && i.modifiers.is_none(), fresh)
    });
    // every plain H press (and its key repeats) is ours: the old action must not also fire on press
    ctx.input_mut(|i| while i.consume_key(Modifiers::NONE, Key::H) {});
    if pressed && app.h_down.is_none() {
        app.h_down = Some((std::time::Instant::now(), false));
    }
    let Some((at, fired)) = app.h_down else { return };
    if !down {
        app.h_down = None;
        if !fired {
            let _ = app.run("photo.toggleFavorite", json!({}));
        }
    } else if !fired && at.elapsed() >= H_HOLD {
        app.h_down = Some((at, true));
        app.h_replay = true;
        app.synthetic.push(egui::Event::Key { key: Key::H, physical_key: None, pressed: true, repeat: false, modifiers: Modifiers::NONE });
        ctx.request_repaint();
    } else if !fired {
        ctx.request_repaint_after(H_HOLD.saturating_sub(at.elapsed()));
    }
}

pub fn handle(app: &mut LightcraftApp, ctx: &egui::Context) {
    crate::widgets::take_wheel_for_picked_slider(ctx);
    // Tab hides the panels (Classic) rather than walking the keyboard focus through buttons: a
    // focus egui gave a widget that isn't a text field is let go, so the keys keep working
    if let Some(f) = ctx.memory(|m| m.focused())
        && egui::text_edit::TextEditState::load(ctx, f).is_none()
    {
        ctx.memory_mut(|m| m.surrender_focus(f));
    }
    let esc = ctx.input(|i| i.key_pressed(Key::Escape) && i.modifiers.is_none());
    // Esc in the notes. panel (its field or the panel last clicked) closes the panel, nothing else
    if esc && crate::fixnotes::has_focus(app, ctx) {
        ctx.input_mut(|i| i.consume_key(Modifiers::NONE, Key::Escape));
        let _ = app.run("notes.toggle", json!({"open": false}));
        return;
    }
    // Esc with a popup or menu open (the toolbar's zoom / click. / ovl. menus, a context menu)
    // only closes it: the popup sees the key when it is drawn, and the view stays
    if esc && egui::Popup::is_any_open(ctx) {
        return;
    }
    // don't steal keys from text fields
    if ctx.egui_wants_keyboard_input() || crate::widgets::slider_typing(ctx) {
        return;
    }
    // a slider picked by its name owns the arrows (the slider reads them); Esc lets go of it
    if crate::widgets::picked_slider(ctx).is_some() {
        if esc {
            ctx.input_mut(|i| i.consume_key(Modifiers::NONE, Key::Escape));
            crate::widgets::unpick_slider(ctx);
            return;
        }
        let arrows = ctx.input(|i| {
            !i.modifiers.command
                && !i.modifiers.alt
                && [Key::ArrowUp, Key::ArrowDown, Key::ArrowLeft, Key::ArrowRight].iter().any(|k| i.key_pressed(*k))
        });
        if arrows {
            return;
        }
    }
    if record(app, ctx) {
        return;
    }
    // an open dialog has the keyboard: nothing behind it reacts (Esc / Return are the dialog's)
    if app.ui.dialog.is_some() {
        return;
    }
    tap_or_hold_h(app, ctx);
    if ctrl_as_cmd(app, ctx) {
        return;
    }
    // the galleries. tab: ⌘A / Esc pick its photos, not the library's
    if app.ui.view == crate::state::ViewMode::Galleries && crate::panels::galleries::keys(app, ctx) {
        return;
    }
    // the merge. page: ← / → groups, ↩ / M merge., S send.back., ⌘A every group
    if crate::merge_page::keys(app, ctx) {
        return;
    }
    // a merges. view with frames picked: ⌫ asks out.of.group. / remove. / del. (the ⊖'s question)
    if crate::merges::delete_key(app, ctx) {
        return;
    }
    // ← / → over a Collections row close / open its folders instead of moving the photo
    if crate::panels::left::collection_keys(app, ctx) {
        return;
    }
    // Esc leaves the Draw Face Region tool before it does anything else
    if app.ui.draw_face && ctx.input_mut(|i| i.consume_key(Modifiers::NONE, Key::Escape)) {
        app.ui.draw_face = false;
        return;
    }
    let mut fire: Vec<String> = Vec::new();
    // shortcuts the native menu bar handles (it consumes those key presses itself)
    let natives: std::collections::HashSet<String> = app.native_shortcuts.iter().map(|s| crate::keymap::canonical(s)).collect();
    let native = |sc: &str| natives.contains(&crate::keymap::canonical(sc));
    let mut aliased: Vec<(String, serde_json::Value)> = Vec::new();
    let mut bound: Vec<(String, serde_json::Value)> = Vec::new();
    // the profile's and the user's bindings (Settings ▸ shrt.) win over the built-ins below
    let overlay = app.keymap.overlay().to_vec();
    let claimed = |m: Modifiers, k: Key| {
        let c = Combo { cmd: m.command, ctrl: m.ctrl, alt: m.alt, shift: m.shift, trigger: Trigger::Key(k) };
        overlay.iter().any(|(o, _, _)| *o == c)
    };
    ctx.input(|i| {
        let mut done: Vec<Combo> = Vec::new();
        for (c, _, _) in &overlay {
            if done.contains(c) || native(&crate::keymap::format_combo(c)) || !combo_pressed(i, c) {
                continue;
            }
            done.push(*c);
            if let Some(Some(b)) = crate::keymap::lookup(&overlay, c) {
                bound.push((b.command.clone(), b.params.clone()));
            }
        }
        let matches = |i: &egui::InputState, m: Modifiers, k: Key| matches(i, m, k) && !claimed(m, k);
        let mut ui_keys = Vec::new();
        for (id, _, sc, _) in crate::menus::ui_commands() {
            if let Some((m, k)) = sc.and_then(parse) {
                ui_keys.push((m, k));
                if !native(sc.unwrap_or_default()) && matches(i, m, k) {
                    fire.push(id.to_string());
                }
            }
        }
        for c in lightcraft_engine::command_specs() {
            // A UI command bound to the same key wraps the engine command (e.g. `W` opens the
            // White Balance Selector tool rather than sampling without a point): the UI one wins.
            if let Some((m, k)) = c.shortcut.filter(|s| !native(s)).and_then(parse)
                && !ui_keys.contains(&(m, k))
                && matches(i, m, k)
            {
                fire.push(c.id.to_string());
            }
        }
        for (sc, id, params) in ALIASES {
            if let Some((m, k)) = parse(sc).filter(|_| !native(sc))
                && matches(i, m, k)
            {
                aliased.push((id.to_string(), serde_json::from_str(params).unwrap_or_default()));
            }
        }
        // ⌥← / ⌥→ (Lightroom) and < / > show another variation of the selected Generative Remove spot; ⌥⇧G turns
        // Use Generative AI on and off (both only in the Remove tool, see `run_fired`)
        for (m, key, delta) in [
            (Modifiers::SHIFT, Key::Comma, -1),
            (Modifiers::SHIFT, Key::Period, 1),
            (Modifiers::ALT, Key::ArrowLeft, -1),
            (Modifiers::ALT, Key::ArrowRight, 1),
        ] {
            if matches(i, m, key) {
                fire.push(format!("genVariation:{delta}"));
            }
        }
        if matches(i, Modifiers::ALT | Modifiers::SHIFT, Key::G) {
            fire.push("genToggle".into());
        }
        // rating 0-5, colour labels 6-9 (with Shift: and advance)
        for (n, key) in [Key::Num0, Key::Num1, Key::Num2, Key::Num3, Key::Num4, Key::Num5].iter().enumerate() {
            if matches(i, Modifiers::NONE, *key) && !native(&n.to_string()) {
                fire.push(format!("rate:{n}:0"));
            }
            if matches(i, Modifiers::SHIFT, *key) {
                fire.push(format!("rate:{n}:1"));
            }
        }
        for (label, key, sc) in [("red", Key::Num6, "6"), ("yellow", Key::Num7, "7"), ("green", Key::Num8, "8"), ("blue", Key::Num9, "9")] {
            if matches(i, Modifiers::NONE, key) && !native(sc) {
                fire.push(format!("label:{label}"));
            }
        }
    });
    for (id, params) in bound {
        dispatch(app, ctx, &id, params);
    }
    for (id, params) in aliased {
        run_aliased(app, ctx, &id, params);
    }
    for f in fire {
        run_fired(app, ctx, f);
    }
}

/// Run a bound command the way its key does (culling keys in Compare/Survey, Delete on the active
/// mask, X in the crop tool…): `{}` params take the key path, others the alias path.
pub fn dispatch(app: &mut LightcraftApp, ctx: &egui::Context, id: &str, params: serde_json::Value) {
    if params.is_null() || params.as_object().is_some_and(|o| o.is_empty()) {
        run_fired(app, ctx, id.to_string());
    } else {
        run_aliased(app, ctx, id, params);
    }
}

/// [`dispatch`], reporting the error of the command the key path ended up running (an unknown id,
/// a command that refuses): the `lc_cmd` console input, one command or a chain. A key path that runs
/// no command (a toggle of the UI) is `Ok`.
pub fn dispatch_checked(app: &mut LightcraftApp, ctx: &egui::Context, id: &str, params: serde_json::Value) -> Result<(), String> {
    app.last_run_error = None;
    dispatch(app, ctx, id, params);
    app.last_run_error.take().map_or(Ok(()), Err)
}

/// Rating/flag/label keys: in Compare/Survey they act on the active photo only; Shift+key or Auto
/// Advance then moves on (next candidate in Compare, next photo elsewhere).
fn cull(app: &mut LightcraftApp, id: &str, mut params: serde_json::Value, advance: bool) {
    use crate::panels::compare;
    compare::target_active(app, &mut params);
    let ok = app.run(id, params).is_ok();
    if ok && (advance || app.ui.auto_advance) {
        compare::advance(app);
    }
}

fn run_aliased(app: &mut LightcraftApp, ctx: &egui::Context, id: &str, params: serde_json::Value) {
    // Space plays / pauses a video in the Loupe (KEY-VIDEO-PLAY)
    if id == "view.zoomToggle" && crate::video_ui::wants_space(app) {
        if let Err(e) = app.run("video.playPause", json!({})) {
            app.toast(ctx, e);
        }
        return;
    }
    // Space pauses / resumes a slideshow
    if id == "view.zoomToggle"
        && let Some((interval, _, paused)) = app.ui.slideshow
    {
        let now = ctx.input(|i| i.time);
        app.ui.slideshow = Some((interval, now + interval, !paused));
        app.toast(ctx, if paused { "Slideshow resumed" } else { "Slideshow paused" });
        return;
    }
    // flag/rate aliases (Shift+X…) go through the culling path: active photo in Compare/Survey,
    // `advance` moves to the next candidate there
    if matches!(id, "photo.flag" | "photo.rate" | "photo.label") {
        let mut params = params;
        let advance = params.get("advance").and_then(serde_json::Value::as_bool).unwrap_or(false);
        if let Some(o) = params.as_object_mut() {
            o.remove("advance");
        }
        cull(app, id, params, advance);
    } else if let Err(e) = app.run(id, params)
        && matches!(id, "app.export" | "app.exportPrevious")
    {
        // an export that can't start (e.g. no folder) says why instead of doing nothing
        app.toast(ctx, e);
    }
}

fn run_fired(app: &mut LightcraftApp, ctx: &egui::Context, f: String) {
    if f == "genToggle" {
        if app.ui.right == crate::state::RightPanel::Remove {
            app.ui.remove_generative = !app.ui.remove_generative;
            app.toast(ctx, if app.ui.remove_generative { "Use Generative AI: on" } else { "Use Generative AI: off" });
        }
        return;
    }
    if let Some(d) = f.strip_prefix("genVariation:") {
        if app.ui.right == crate::state::RightPanel::Remove && app.session.active_spot.is_some() {
            let delta: i64 = d.parse().unwrap_or(1);
            if let Ok(r) = app.run("spot.variation", json!({"delta": delta})) {
                let (v, n) = (r["variation"].as_u64().unwrap_or(0) + 1, r["count"].as_u64().unwrap_or(0));
                app.toast(ctx, format!("Variation {v} of {n}"));
            }
        }
        return;
    }
    use crate::panels::compare;
    if let Some(rest) = f.strip_prefix("rate:") {
        let (n, adv) = rest.split_once(':').unwrap_or(("0", "0"));
        cull(app, "photo.rate", json!({"rating": n.parse::<u8>().unwrap_or(0)}), adv == "1");
        let label = if n == "0" {
            crate::i18n::tr("Rating cleared").to_string()
        } else {
            crate::i18n::tr_format!("Rated {}", "★".repeat(n.parse().unwrap_or(0)))
        };
        app.toast(ctx, label);
    } else if let Some(l) = f.strip_prefix("label:") {
        cull(app, "photo.label", json!({"label": l}), false);
    } else if f == "panel.remove" && app.ui.right == crate::state::RightPanel::Masking {
        // KEY-HIDEPINS: H while masking shows / hides the mask pins (Lightroom); elsewhere it is Remove
        if let Ok(r) = app.run("view.maskPins", json!({})) {
            app.toast(ctx, if r["maskPins"] == true { crate::i18n::tr("Mask pins shown") } else { crate::i18n::tr("Mask pins hidden") });
        }
    } else if f == "panel.presets" && library_grid(app) {
        // Classic's Shift+P picks and advances in Library; keep the Presets binding elsewhere.
        cull(app, "photo.flag", json!({"flag": "pick"}), true);
    } else if f == "view.softProof" && matches!(app.ui.view, crate::state::ViewMode::PhotoGrid | crate::state::ViewMode::SquareGrid) {
        // S in a grid: expand / collapse the stack (Lightroom's Library binding)
        let _ = app.run("stack.toggle", json!({}));
    } else if compare::culling(app) && (f == "library.next" || f == "library.previous") {
        let d = if f == "library.next" { 1 } else { -1 };
        let _ = if app.ui.view == crate::state::ViewMode::Compare { compare::compare_step(app, d) } else { compare::survey_step(app, d) };
    } else if matches!(f.as_str(), "brush.smaller" | "brush.larger")
        && !matches!(app.ui.right, crate::state::RightPanel::Masking | crate::state::RightPanel::Remove)
    {
        // Lightroom: [ / ] size the brush while brushing, step the rating everywhere else
        let id = if f == "brush.larger" { "photo.increaseRating" } else { "photo.decreaseRating" };
        cull(app, id, json!({}), false);
        let stars = app.session.active().and_then(|a| app.session.catalog.photo(a)).map_or(0, |p| p.rating);
        let label =
            if stars == 0 { crate::i18n::tr("Rating cleared").to_string() } else { crate::i18n::tr_format!("Rated {}", "★".repeat(stars.into())) };
        app.toast(ctx, label);
    } else if matches!(f.as_str(), "photo.toggleFlag" | "photo.increaseFlag" | "photo.decreaseFlag") && app.ui.right != crate::state::RightPanel::Crop
    {
        cull(app, &f, json!({}), false);
        let flag = app.session.active().and_then(|a| app.session.catalog.photo(a)).map(|p| p.flag);
        app.toast(
            ctx,
            match flag {
                Some(lightcraft_engine::catalog::Flag::Pick) => crate::i18n::tr("Flagged as Pick"),
                Some(lightcraft_engine::catalog::Flag::Reject) => crate::i18n::tr("Flagged as Reject"),
                _ => crate::i18n::tr("Unflagged"),
            },
        );
    } else if matches!(f.as_str(), "photo.pick" | "photo.reject" | "photo.unflag") && app.ui.right != crate::state::RightPanel::Crop {
        cull(app, &f, json!({}), false);
        match f.as_str() {
            "photo.pick" => app.toast(ctx, crate::i18n::tr("Flagged as Pick")),
            "photo.reject" => app.toast(ctx, crate::i18n::tr("Flagged as Reject")),
            _ => app.toast(ctx, crate::i18n::tr("Unflagged")),
        }
    } else {
        // in the full-screen preview (no panels) I cycles the info overlay instead
        if f == "panel.info" && app.ui.fullscreen {
            let _ = app.run("view.infoOverlay", json!({}));
            return;
        }
        // Delete acts on what's being edited: the active mask in the Masking panel; never the
        // photo while retouching (spots are removed from their own panel).
        if f == "photo.delete" {
            use crate::state::RightPanel as R;
            match app.ui.right {
                R::Masking => {
                    if app.session.active_mask.is_some() {
                        let _ = app.run("mask.delete", json!({}));
                    }
                    return;
                }
                R::Remove => {
                    if app.session.active_spot.is_some() {
                        let _ = app.run("spot.delete", json!({}));
                    }
                    return;
                }
                R::RedEye => return,
                _ => {}
            }
            if crate::menus::delete_key(app) {
                return;
            }
        }
        // B: the brush while editing; in the grids, add to the target album (Quick Collection)
        if f == "tool.brush" && matches!(app.ui.view, crate::state::ViewMode::PhotoGrid | crate::state::ViewMode::SquareGrid) {
            if let Ok(r) = app.run("album.toggleTarget", json!({})) {
                let n = app.session.targets(&json!({})).len();
                let what = crate::i18n::tr_format!("{n} photo{}", if n == 1 { "" } else { "s" }, n = n);
                let name = r["name"].as_str().unwrap_or("Quick Collection").to_string();
                app.toast(
                    ctx,
                    if r["added"] == true {
                        crate::i18n::tr_format!("Added {what} to {name}", name = name, what = what)
                    } else {
                        crate::i18n::tr_format!("Removed {what} from {name}", name = name, what = what)
                    },
                );
            }
            return;
        }
        // X is both reject (library) and swap crop aspect (crop tool)
        if f == "photo.reject" && app.ui.right == crate::state::RightPanel::Crop {
            let _ = app.run("crop.rotateAspect", json!({}));
            return;
        }
        if f == "crop.rotateAspect" && app.ui.right != crate::state::RightPanel::Crop {
            return;
        }
        // / refreshes the selected spot's source in the Remove tool (the filmstrip elsewhere)
        if f == "view.filmstrip" && app.ui.right == crate::state::RightPanel::Remove && app.session.active_spot.is_some() {
            let _ = app.run("spot.refreshSource", json!({}));
            return;
        }
        // Shift+O cycles the mask overlay colour while masking (the crop overlay elsewhere)
        if f == "view.cropOverlay" && app.ui.right == crate::state::RightPanel::Masking {
            let _ = app.run("view.maskOverlayColor", json!({}));
            return;
        }
        // in the loupe outside the crop and masking tools: O cycles the composition overlay,
        // Shift+O turns it (LRC-VIEW-LOUPEOVERLAY)
        if app.ui.view == crate::state::ViewMode::Detail
            && !matches!(app.ui.right, crate::state::RightPanel::Crop | crate::state::RightPanel::Masking)
            && let Some(cmd) = match f.as_str() {
                "view.maskOverlay" => Some("view.compositionOverlay"),
                "view.cropOverlay" => Some("view.compositionOverlayOrientation"),
                _ => None,
            }
        {
            let _ = app.run(cmd, json!({}));
            return;
        }
        // while cropping: O cycles the guides, Shift+O their orientation, A locks the aspect
        if app.ui.right == crate::state::RightPanel::Crop {
            let crop_key = match f.as_str() {
                "view.maskOverlay" => Some(("view.cropOverlay", json!({}))),
                "view.cropOverlay" => Some(("view.cropOverlayOrientation", json!({}))),
                "view.visualizeSpots" => Some(("crop.aspect", json!({"aspect": "toggle"}))),
                _ => None,
            };
            if let Some((cmd, p)) = crop_key {
                let _ = app.run(cmd, p);
                return;
            }
        }
        // an export that can't start (e.g. no folder) says why instead of doing nothing
        let result = app.run(&f, json!({}));
        if let Err(e) = &result
            && matches!(f.as_str(), "app.export" | "app.exportPrevious" | "edit.redo")
        {
            app.toast(ctx, e.clone());
            return;
        }
        match f.as_str() {
            "photo.pick" => app.toast(ctx, crate::i18n::tr("Flagged as Pick")),
            "photo.reject" => app.toast(ctx, crate::i18n::tr("Flagged as Reject")),
            "photo.unflag" => app.toast(ctx, crate::i18n::tr("Unflagged")),
            // an undone delete says which files couldn't come back from the Trash
            "edit.undo" => {
                let msg = result.as_ref().map(crate::menus::undo_message).unwrap_or_else(|_| crate::i18n::tr("Undo").to_string());
                app.toast(ctx, msg);
            }
            "edit.redo" => app.toast(ctx, crate::i18n::tr("Redo")),
            _ => {}
        }
    }
}

pub(crate) fn library_grid(app: &LightcraftApp) -> bool {
    matches!(app.ui.view, crate::state::ViewMode::PhotoGrid | crate::state::ViewMode::SquareGrid)
}

#[cfg(test)]
mod tests {

    fn library() -> crate::headless::Headless {
        use lightcraft_catalog::{Op, Photo, PhotoId, Source};
        let mut session = lightcraft_engine::Session::new();
        for id in 1..=4 {
            let photo = Photo::new(
                PhotoId(id),
                Source::File { path: format!("/lightcraft-shortcuts/{id}.jpg") },
                &format!("{id}.jpg"),
                "JPEG",
                100,
                100,
                "2026-10-08",
            );
            session.catalog.apply(Op::AddPhoto { photo: Box::new(photo) }).unwrap();
        }
        session.execute("library.sort", &json!({"key": "fileName", "ascending": true})).unwrap();
        session.execute("library.select", &json!({"ids": [1]})).unwrap();
        let app = LightcraftApp::new(session, crate::Services { png: None, ..Default::default() });
        let mut h = crate::headless::Headless::new(app, [1200.0, 800.0], 1.0);
        h.app.ui.view = crate::state::ViewMode::PhotoGrid;
        for _ in 0..3 {
            h.step();
        }
        h
    }

    fn key(h: &mut crate::headless::Headless, key: &str, shift: bool) {
        let r = h.request("ui.key", json!({"key": key, "shift": shift}), std::time::Duration::from_secs(20));
        assert_eq!(r["ok"], true, "{r}");
    }

    #[test]
    fn library_rating_labels_flags_and_undo() {
        use lightcraft_catalog::{ColorLabel, Flag, PhotoId};
        for view in [crate::state::ViewMode::PhotoGrid, crate::state::ViewMode::SquareGrid] {
            let mut h = library();
            h.app.ui.view = view;
            h.app.session.execute("library.select", &json!({"ids": [1, 2], "active": 1})).unwrap();
            for rating in [1, 2, 3, 4, 5, 0] {
                key(&mut h, &rating.to_string(), false);
                for id in [1, 2] {
                    assert_eq!(h.app.session.catalog.photo(PhotoId(id)).unwrap().rating, rating);
                }
            }
            for (k, label) in [("6", ColorLabel::Red), ("7", ColorLabel::Yellow), ("8", ColorLabel::Green), ("9", ColorLabel::Blue)] {
                key(&mut h, k, false);
                assert_eq!(h.app.session.catalog.photo(PhotoId(1)).unwrap().label, Some(label));
                assert_eq!(h.app.session.catalog.photo(PhotoId(2)).unwrap().label, Some(label));
            }
            for (k, flag) in [("P", Flag::Pick), ("X", Flag::Reject), ("U", Flag::None)] {
                key(&mut h, k, false);
                assert_eq!(h.app.session.catalog.photo(PhotoId(1)).unwrap().flag, flag);
                assert_eq!(h.app.session.catalog.photo(PhotoId(2)).unwrap().flag, flag);
            }
            h.app.session.execute("edit.undo", &json!({})).unwrap();
            assert_eq!(h.app.session.catalog.photo(PhotoId(1)).unwrap().flag, Flag::Reject);
            assert_eq!(h.app.session.catalog.photo(PhotoId(2)).unwrap().flag, Flag::Reject);
            h.app.session.execute("edit.redo", &json!({})).unwrap();
            assert_eq!(h.app.session.catalog.photo(PhotoId(1)).unwrap().flag, Flag::None);
            assert_eq!(h.app.session.active(), Some(PhotoId(1)));
        }
    }

    #[test]
    fn shift_culling_keys_advance_exactly_once() {
        use lightcraft_catalog::{ColorLabel, Flag, PhotoId};
        for auto in [false, true] {
            for (k, rating, label, flag) in [
                ("0", 0, None, Flag::Pick),
                ("5", 5, None, Flag::Pick),
                ("6", 3, Some(ColorLabel::Red), Flag::Pick),
                ("7", 3, Some(ColorLabel::Yellow), Flag::Pick),
                ("8", 3, Some(ColorLabel::Green), Flag::Pick),
                ("9", 3, Some(ColorLabel::Blue), Flag::Pick),
                ("P", 3, None, Flag::Pick),
                ("X", 3, None, Flag::Reject),
                ("U", 3, None, Flag::None),
            ] {
                let mut h = library();
                h.app.ui.auto_advance = auto;
                h.app.session.execute("photo.rate", &json!({"rating": 3})).unwrap();
                h.app.session.execute("photo.pick", &json!({})).unwrap();
                key(&mut h, k, true);
                let p = h.app.session.catalog.photo(PhotoId(1)).unwrap();
                assert_eq!(p.rating, rating, "Shift+{k}");
                assert_eq!(p.label, label, "Shift+{k}");
                assert_eq!(p.flag, flag, "Shift+{k}");
                assert_eq!(h.app.session.active(), Some(PhotoId(2)), "Shift+{k}, auto={auto}");
            }
        }
        let mut h = library();
        h.app.session.execute("library.select", &json!({"ids": [4]})).unwrap();
        key(&mut h, "6", true);
        assert_eq!(h.app.session.active(), Some(PhotoId(4)), "last photo stays selected");
    }

    #[test]
    fn shift_pick_preserves_presets_outside_the_library_grids() {
        let mut h = library();
        h.app.ui.view = crate::state::ViewMode::SquareGrid;
        key(&mut h, "P", true);
        assert_eq!(h.app.session.active(), Some(lightcraft_catalog::PhotoId(2)));
        assert!(!h.app.ui.presets, "grid Shift+P must not open Presets");
        assert_eq!(h.app.ui.view, crate::state::ViewMode::SquareGrid);
        h.app.ui.view = crate::state::ViewMode::Detail;
        key(&mut h, "P", true);
        assert!(h.app.ui.presets);
        assert_eq!(h.app.session.active(), Some(lightcraft_catalog::PhotoId(2)));
    }

    #[test]
    fn shift_label_targets_only_candidate_in_compare() {
        use lightcraft_catalog::{ColorLabel, PhotoId};
        let mut h = library();
        h.app.session.execute("library.select", &json!({"ids": [1, 2], "active": 1})).unwrap();
        crate::panels::compare::enter_compare(&mut h.app).unwrap();
        key(&mut h, "6", true);
        assert_eq!(h.app.session.catalog.photo(PhotoId(1)).unwrap().label, None);
        assert_eq!(h.app.session.catalog.photo(PhotoId(2)).unwrap().label, Some(ColorLabel::Red));
        assert_eq!(h.app.session.active(), Some(PhotoId(3)));
    }

    #[test]
    fn plain_culling_keys_auto_advance_once() {
        use lightcraft_catalog::PhotoId;
        for k in ["0", "5", "6", "9", "P", "X", "U"] {
            let mut h = library();
            h.app.ui.auto_advance = true;
            key(&mut h, k, false);
            assert_eq!(h.app.session.active(), Some(PhotoId(2)), "{k}");
        }
    }

    #[test]
    fn color_label_actions_show_feedback_only_after_success() {
        use lightcraft_catalog::PhotoId;
        let mut h = library();
        for shift in [false, true] {
            for (k, name) in [("6", "Red"), ("7", "Yellow"), ("8", "Green"), ("9", "Blue")] {
                h.app.session.execute("library.select", &json!({"ids": [1]})).unwrap();
                h.app.ui.toast = None;
                key(&mut h, k, shift);
                assert_eq!(h.app.ui.toast.as_ref().map(|t| t.0.as_str()), Some(format!("{name} Label").as_str()));
                assert_eq!(h.app.ui.toast.as_ref().and_then(|t| t.2), h.app.session.catalog.photo(PhotoId(1)).unwrap().label);
                assert_eq!(h.app.session.active(), Some(PhotoId(if shift { 2 } else { 1 })));
            }
        }
        // Menus and the Info-panel swatches dispatch the same command, including purple/clear.
        h.app.run("photo.label", json!({"label": "purple"})).unwrap();
        assert_eq!(h.app.ui.toast.as_ref().map(|t| t.0.as_str()), Some("Purple Label"));
        h.app.run("photo.label", json!({"label": "none"})).unwrap();
        assert_eq!(h.app.ui.toast.as_ref().map(|t| t.0.as_str()), Some("Color label cleared"));
        assert!(h.app.ui.toast.as_ref().is_some_and(|t| t.2.is_none()), "clear uses neutral feedback");
        h.app.session.execute("label.setNames", &json!({"names": {"red": "Needs review"}})).unwrap();
        key(&mut h, "6", false);
        assert_eq!(h.app.ui.toast.as_ref().map(|t| t.0.as_str()), Some("Needs review Label"));
        // Native menu callbacks run before logic() catches up to the current frame's clock.
        let now = h.view.ctx.input(|i| i.time);
        h.app.last_time = now - 10.0;
        h.app.run("photo.label", json!({"label": "blue"})).unwrap();
        assert!(h.app.ui.toast.as_ref().is_some_and(|t| t.1 > now), "menu feedback must survive an idle gap");
        h.app.ui.toast = None;
        assert!(h.app.run("photo.label", json!({"label": "invalid"})).is_err());
        assert!(h.app.ui.toast.is_none(), "failed actions must not announce success");
    }

    #[test]
    fn shifted_number_punctuation_rates_and_advances() {
        use lightcraft_catalog::PhotoId;
        let mut h = library();
        // A real winit event on a layout with Shift+1 = !, rather than ui.key's logical Num1.
        for pressed in [true, false] {
            h.app.synthetic.push(egui::Event::Key {
                key: Key::Exclamationmark,
                physical_key: Some(Key::Num1),
                pressed,
                repeat: false,
                modifiers: Modifiers::SHIFT,
            });
        }
        h.step();
        assert_eq!(h.app.session.catalog.photo(PhotoId(1)).unwrap().rating, 1);
        assert_eq!(h.app.session.active(), Some(PhotoId(2)));
    }

    #[test]
    fn library_keys_yield_to_search_and_crop_context() {
        use lightcraft_catalog::{Flag, PhotoId};
        let mut h = library();
        let r = h.request("ui.clickWidget", json!({"id": "field:search"}), std::time::Duration::from_secs(20));
        assert_eq!(r["ok"], true, "{r}");
        for k in ["5", "6", "P", "X", "U"] {
            key(&mut h, k, false);
            key(&mut h, k, true);
        }
        let p = h.app.session.catalog.photo(PhotoId(1)).unwrap();
        assert_eq!(p.rating, 0);
        assert_eq!(p.label, None);
        assert_eq!(p.flag, Flag::None);
        assert_eq!(h.app.session.active(), Some(PhotoId(1)));

        let mut h = library();
        h.app.ui.view = crate::state::ViewMode::Detail;
        h.app.ui.right = crate::state::RightPanel::Crop;
        key(&mut h, "X", false);
        assert_eq!(h.app.session.catalog.photo(PhotoId(1)).unwrap().flag, Flag::None);
    }

    #[test]
    fn delete_means_the_backspace_key() {
        assert_eq!(parse("Delete"), Some((Modifiers::NONE, Key::Backspace)));
        assert_eq!(parse("Cmd+Delete"), Some((Modifiers::COMMAND, Key::Backspace)));
    }
    use super::*;

    #[test]
    fn parses_shortcuts() {
        let (m, k) = parse("Cmd+Shift+Z").unwrap();
        assert!(m.command && m.shift && !m.alt);
        assert_eq!(k, Key::Z);
        assert_eq!(parse("Right").unwrap().1, Key::ArrowRight);
        assert_eq!(parse("\\").unwrap().1, Key::Backslash);
        // every declared shortcut parses
        for (id, _, sc, _) in crate::menus::ui_commands() {
            if let Some(sc) = sc {
                assert!(parse(sc).is_some(), "{id}: {sc}");
            }
        }
        for c in lightcraft_engine::command_specs() {
            if let Some(sc) = c.shortcut {
                assert!(parse(sc).is_some(), "{}: {sc}", c.id);
            }
        }
    }

    #[test]
    fn aliases_parse_and_target_existing_commands() {
        let ui: Vec<&str> = crate::menus::ui_commands().map(|c| c.0).collect();
        for (sc, id, params) in ALIASES {
            assert!(parse(sc).is_some(), "{id}: {sc}");
            assert!(ui.contains(id) || lightcraft_engine::find_command(id).is_some(), "alias {sc} → unknown command {id}");
            assert!(serde_json::from_str::<serde_json::Value>(params).is_ok(), "alias {sc}: bad params");
        }
    }

    /// Engine commands that intentionally share a key and are disambiguated by context in [`handle`].
    const CONTEXTUAL: &[(&str, &str)] = &[("photo.reject", "crop.rotateAspect")];

    /// No key fires two different actions (a UI command may shadow the engine command it wraps).
    #[test]
    fn no_conflicting_bindings() {
        let mut ui: Vec<((Modifiers, Key), String)> = Vec::new();
        for (id, _, sc, _) in crate::menus::ui_commands() {
            if let Some(k) = sc.and_then(parse) {
                ui.push((k, id.to_string()));
            }
        }
        for (sc, id, _) in ALIASES {
            ui.push((parse(sc).unwrap(), format!("alias {id}")));
        }
        let mut engine: Vec<((Modifiers, Key), &str)> = Vec::new();
        for c in lightcraft_engine::command_specs() {
            if let Some(k) = c.shortcut.and_then(parse) {
                engine.push((k, c.id));
            }
        }
        for (sc, id, _) in ALIASES {
            let k = parse(sc).unwrap();
            assert!(!engine.iter().any(|(k2, _)| *k2 == k), "alias {sc} ({id}) shadows an engine shortcut");
        }
        for (i, (k, a)) in ui.iter().enumerate() {
            for (k2, b) in &ui[i + 1..] {
                assert!(k != k2, "{a} and {b} share a key");
            }
        }
        for (i, (k, a)) in engine.iter().enumerate() {
            for (k2, b) in &engine[i + 1..] {
                let contextual = CONTEXTUAL.iter().any(|(x, y)| (x == a && y == b) || (x == b && y == a));
                assert!(k != k2 || contextual, "{a} and {b} share a key");
            }
        }
    }
}
