//! `log` records on stderr for the native desktop app and CLI (#168). The web build installs
//! eframe's `WebLogger` instead.
//!
//! Warnings and errors are shown by default. `LIGHTCRAFT_LOG` or `RUST_LOG` picks another level
//! (`off`, `error`, `warn`, `info`, `debug`, `trace`); `LIGHTCRAFT_LOG` wins when both are set.
//! `RUST_LOG` also accepts per-target directives (`lightcraft=debug,wgpu=warn`): a bare level
//! applies to everything, and the most verbose `lightcraft…=level` directive sets LightCraft's.
//! Below warnings only LightCraft's own records are shown, so `info` and `debug` aren't drowned
//! out by the GPU and windowing libraries.

use log::{Level, LevelFilter, Metadata, Record};

/// Records shown at a level, written to stderr as `<prefix>: LEVEL target: message`.
pub struct StderrLog {
    prefix: &'static str,
    level: LevelFilter,
}

impl StderrLog {
    pub fn new(prefix: &'static str, level: LevelFilter) -> Self {
        Self { prefix, level }
    }
}

impl log::Log for StderrLog {
    fn enabled(&self, m: &Metadata) -> bool {
        m.level() <= self.level && (m.level() <= Level::Warn || m.target().starts_with("lightcraft"))
    }
    fn log(&self, r: &Record) {
        if r.level() <= Level::Warn {
            remember(&format!("{} {}: {}", r.level(), r.target(), r.args()));
        }
        if self.enabled(r.metadata()) {
            eprintln!("{}: {} {}: {}", self.prefix, r.level(), r.target(), r.args());
        }
    }
    fn flush(&self) {}
}

/// How many recent warnings and errors [`recent`] keeps.
const RECENT: usize = 40;

type Recent = std::collections::VecDeque<(i64, String)>;
static RECENT_LOG: std::sync::Mutex<Recent> = std::sync::Mutex::new(std::collections::VecDeque::new());

/// Keep `line` among the recent warnings and errors (the installed logger does this for every
/// warning and error, whatever level it shows; the fix notes record the last few).
pub fn remember(line: &str) {
    let at = web_time::SystemTime::now().duration_since(web_time::UNIX_EPOCH).map(|d| d.as_secs() as i64).unwrap_or(0);
    // (capped: one runaway message can't hold megabytes)
    let line: String = line.chars().take(600).collect();
    let mut q = RECENT_LOG.lock().unwrap_or_else(std::sync::PoisonError::into_inner);
    q.push_back((at, line));
    while q.len() > RECENT {
        q.pop_front();
    }
}

/// The last `n` warnings and errors logged, oldest first: (Unix seconds, `LEVEL target: message`).
pub fn recent(n: usize) -> Vec<(i64, String)> {
    let q = RECENT_LOG.lock().unwrap_or_else(std::sync::PoisonError::into_inner);
    q.iter().skip(q.len().saturating_sub(n)).cloned().collect()
}

fn parse_level(s: &str) -> Option<LevelFilter> {
    match s.trim().to_ascii_lowercase().as_str() {
        "off" | "none" => Some(LevelFilter::Off),
        "error" => Some(LevelFilter::Error),
        "warn" | "warning" => Some(LevelFilter::Warn),
        "info" => Some(LevelFilter::Info),
        "debug" => Some(LevelFilter::Debug),
        "trace" => Some(LevelFilter::Trace),
        _ => None,
    }
}

/// The level that `LIGHTCRAFT_LOG` / `RUST_LOG` ask for; warnings when neither names one.
pub fn level_from(lightcraft_log: Option<&str>, rust_log: Option<&str>) -> LevelFilter {
    if let Some(l) = lightcraft_log.and_then(parse_level) {
        return l;
    }
    let Some(rust_log) = rust_log else { return LevelFilter::Warn };
    let mut bare = None;
    let mut ours: Option<LevelFilter> = None;
    for directive in rust_log.split(',') {
        match directive.split_once('=') {
            None => bare = parse_level(directive).or(bare),
            Some((target, level)) if target.trim().starts_with("lightcraft") => {
                if let Some(l) = parse_level(level) {
                    ours = Some(ours.map_or(l, |o| o.max(l)));
                }
            }
            Some(_) => {}
        }
    }
    ours.or(bare).unwrap_or(LevelFilter::Warn)
}

/// Install the stderr logger at the level the environment asks for. Does nothing when a logger is
/// already installed.
pub fn install(prefix: &'static str) {
    let level = level_from(std::env::var("LIGHTCRAFT_LOG").ok().as_deref(), std::env::var("RUST_LOG").ok().as_deref());
    static LOGGER: std::sync::OnceLock<StderrLog> = std::sync::OnceLock::new();
    if log::set_logger(LOGGER.get_or_init(|| StderrLog::new(prefix, level))).is_ok() {
        log::set_max_level(level);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use log::Log;

    #[test]
    fn warnings_by_default_and_either_variable_picks_the_level() {
        assert_eq!(level_from(None, None), LevelFilter::Warn);
        assert_eq!(level_from(None, Some("")), LevelFilter::Warn);
        assert_eq!(level_from(None, Some("warn")), LevelFilter::Warn);
        assert_eq!(level_from(None, Some("TRACE")), LevelFilter::Trace);
        assert_eq!(level_from(None, Some("off")), LevelFilter::Off);
        assert_eq!(level_from(Some("info"), None), LevelFilter::Info);
        assert_eq!(level_from(Some("error"), Some("debug")), LevelFilter::Error, "LIGHTCRAFT_LOG wins");
        assert_eq!(level_from(Some("nonsense"), Some("debug")), LevelFilter::Debug, "an unknown LIGHTCRAFT_LOG is ignored");
    }

    #[test]
    fn rust_log_directives_set_lightcraft_s_level() {
        assert_eq!(level_from(None, Some("lightcraft=debug,wgpu=warn")), LevelFilter::Debug);
        assert_eq!(level_from(None, Some("wgpu_core=trace")), LevelFilter::Warn, "other crates' directives don't apply");
        assert_eq!(level_from(None, Some("error,lightcraft_gpu=info")), LevelFilter::Info);
        assert_eq!(level_from(None, Some("lightcraft_gpu=info,lightcraft_engine=trace")), LevelFilter::Trace);
        assert_eq!(level_from(None, Some("info,naga=off")), LevelFilter::Info);
    }

    #[test]
    fn warnings_are_remembered_even_when_not_shown() {
        let quiet = StderrLog::new("test", LevelFilter::Off);
        quiet.log(&Record::builder().level(Level::Warn).target("lightcraft_test").args(format_args!("remember me {}", 7)).build());
        quiet.log(&Record::builder().level(Level::Info).target("lightcraft_test").args(format_args!("not me")).build());
        let r = recent(RECENT);
        assert!(r.iter().any(|(_, l)| l == "WARN lightcraft_test: remember me 7"), "{r:?}");
        assert!(!r.iter().any(|(_, l)| l.contains("not me")));
    }

    #[test]
    fn only_lightcraft_records_below_warnings() {
        let log = StderrLog::new("test", LevelFilter::Debug);
        let meta = |level, target| Metadata::builder().level(level).target(target).build();
        assert!(log.enabled(&meta(Level::Warn, "wgpu_core::device")));
        assert!(log.enabled(&meta(Level::Debug, "lightcraft_gpu::ctx")));
        assert!(!log.enabled(&meta(Level::Info, "wgpu_core::device")));
        assert!(!log.enabled(&meta(Level::Trace, "lightcraft_gpu::ctx")));
        let quiet = StderrLog::new("test", LevelFilter::Off);
        assert!(!quiet.enabled(&meta(Level::Error, "lightcraft_engine")));
    }
}
