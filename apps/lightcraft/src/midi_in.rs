//! MIDI input for Settings ▸ ctrl. (Monogram Creator, any controller): while "MIDI in" is on,
//! every MIDI source is connected and its raw messages go to the UI ([`LightcraftApp::midi_rx`]),
//! which maps them (`lightcraft_ui_egui::keymap`). macOS only (CoreMIDI through `midir`); other
//! platforms say so on the ctrl. page.

use lightcraft_ui_egui::LightcraftApp;

#[derive(Default)]
pub struct MidiIn {
    #[cfg(target_os = "macos")]
    conns: Vec<midir::MidiInputConnection<()>>,
    /// Sources seen at the last scan.
    names: Vec<String>,
    checked: f64,
    on: bool,
}

impl MidiIn {
    /// Per frame: connect / disconnect with the setting; rescan every few seconds for devices
    /// plugged in later (Monogram Creator's virtual port appears when it starts).
    pub fn tick(&mut self, app: &mut LightcraftApp, ctx: &egui::Context) {
        let want = app.keymap.file.midi_enabled;
        if !want {
            if self.on {
                self.disconnect();
                app.midi_rx = None;
                app.midi_status = "MIDI in is off".into();
            }
            return;
        }
        let now = ctx.input(|i| i.time);
        if self.on && now - self.checked < 5.0 {
            return;
        }
        self.checked = now;
        self.on = true;
        self.scan(app, ctx);
    }

    fn disconnect(&mut self) {
        #[cfg(target_os = "macos")]
        self.conns.clear();
        self.names.clear();
        self.on = false;
    }

    #[cfg(not(target_os = "macos"))]
    fn scan(&mut self, app: &mut LightcraftApp, _ctx: &egui::Context) {
        app.midi_status = "MIDI in works on macOS only in this build".into();
    }

    #[cfg(target_os = "macos")]
    fn scan(&mut self, app: &mut LightcraftApp, ctx: &egui::Context) {
        let probe = match midir::MidiInput::new("LightCraft") {
            Ok(p) => p,
            Err(e) => {
                app.midi_status = format!("MIDI isn't available: {e}");
                return;
            }
        };
        let ports = probe.ports();
        let names: Vec<String> = ports.iter().map(|p| probe.port_name(p).unwrap_or_else(|_| "MIDI source".into())).collect();
        if names == self.names && !self.conns.is_empty() {
            return;
        }
        self.conns.clear();
        let (tx, rx) = std::sync::mpsc::channel::<Vec<u8>>();
        let mut failed = Vec::new();
        for (port, name) in ports.iter().zip(&names) {
            let input = match midir::MidiInput::new("LightCraft") {
                Ok(i) => i,
                Err(e) => {
                    failed.push(format!("{name}: {e}"));
                    continue;
                }
            };
            let (tx, repaint) = (tx.clone(), ctx.clone());
            match input.connect(
                port,
                "lightcraft-in",
                move |_, msg, _| {
                    let _ = tx.send(msg.to_vec());
                    repaint.request_repaint();
                },
                (),
            ) {
                Ok(c) => self.conns.push(c),
                Err(e) => failed.push(format!("{name}: {e}")),
            }
        }
        app.midi_rx = Some(rx);
        self.names = names;
        app.midi_status = if self.names.is_empty() {
            "MIDI in is on: no MIDI sources yet (start Monogram Creator, or plug in the controller)".into()
        } else {
            format!("MIDI in from: {}", self.names.join(", "))
        };
        if !failed.is_empty() {
            app.midi_status.push_str(&format!(" · couldn't open {}", failed.join(", ")));
        }
    }
}
