//! Console dial turns (Monogram, MIDI): latency telemetry.
//!
//! `LIGHTCRAFT_DIAL_LOG=1` prints one `[dial]` line per event on stderr (wall-clock ms): a dial
//! line applied (its age since the socket thread queued it, and what applying it cost), a loupe
//! draft requested and shown — each with the number of dial lines applied so far. `docs/perf.md`
//! → "Console dial latency" is measured from it by a script that sends dial lines to the
//! Monogram port.

use std::sync::atomic::{AtomicU64, Ordering};

/// Dial lines applied since the app started (counted only while the log is on).
static UNITS: AtomicU64 = AtomicU64::new(0);

/// Is `LIGHTCRAFT_DIAL_LOG` set?
pub fn log_enabled() -> bool {
    static ON: std::sync::OnceLock<bool> = std::sync::OnceLock::new();
    *ON.get_or_init(|| std::env::var_os("LIGHTCRAFT_DIAL_LOG").is_some())
}

/// `LIGHTCRAFT_DIAL_LEGACY=1`: render dial turns the way they were before 2026-11-24 (drafts queued
/// behind the sharp render), to measure what the change buys.
pub fn legacy() -> bool {
    static ON: std::sync::OnceLock<bool> = std::sync::OnceLock::new();
    *ON.get_or_init(|| std::env::var_os("LIGHTCRAFT_DIAL_LEGACY").is_some())
}

/// Print `event` with the wall-clock time and the dial lines applied so far.
pub fn log(event: &str) {
    if !log_enabled() {
        return;
    }
    let ms = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_secs_f64() * 1000.0).unwrap_or(0.0);
    use std::io::Write;
    let _ = writeln!(std::io::stderr(), "[dial] t={ms:.2} units={} {event}", UNITS.load(Ordering::Relaxed));
}

/// `n` dial lines were applied.
pub fn applied(n: u64) {
    UNITS.fetch_add(n, Ordering::Relaxed);
}

/// The Monogram line a `keys.lrinput` control request carries.
pub fn line_of(req: &crate::control::ControlRequest) -> Option<&str> {
    let p = &req.params;
    if req.method != "engine.execute" || p.get("command").and_then(|c| c.as_str()) != Some("keys.lrinput") {
        return None;
    }
    p.get("params")?.get("line")?.as_str()
}
