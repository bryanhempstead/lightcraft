//! What the Folders panel says about a disk: its name and how full it is. Both touch the disk
//! (a sleeping NAS can take seconds), so callers run them off the UI thread.

/// `(total, available)` bytes of the disk `path` is on; `None` when it can't be read (the disk is
/// gone, or this platform has no `df`).
pub fn space(path: &str) -> Option<(u64, u64)> {
    if cfg!(any(target_arch = "wasm32", windows)) || !std::path::Path::new(path).exists() {
        return None;
    }
    let out = std::process::Command::new("df").arg("-kP").arg(path).output().ok()?;
    if !out.status.success() {
        return None;
    }
    parse_df(&String::from_utf8_lossy(&out.stdout))
}

/// `df -kP`'s second line: `Filesystem 1024-blocks Used Available Capacity Mounted on`.
fn parse_df(text: &str) -> Option<(u64, u64)> {
    let line = text.lines().nth(1)?;
    // the file-system name may hold spaces: count the numbers from the end
    let words: Vec<&str> = line.split_whitespace().collect();
    let pct = words.iter().rposition(|w| w.ends_with('%'))?;
    let avail: u64 = words.get(pct.checked_sub(1)?)?.parse().ok()?;
    let total: u64 = words.get(pct.checked_sub(3)?)?.parse().ok()?;
    Some((total.saturating_mul(1024), avail.saturating_mul(1024)))
}

/// The startup disk's name (`Macintosh HD`): on macOS, the entry in `/Volumes` that links to `/`.
pub fn startup_name() -> Option<String> {
    if !cfg!(target_os = "macos") {
        return None;
    }
    let rd = std::fs::read_dir("/Volumes").ok()?;
    rd.flatten().find_map(|e| {
        let target = std::fs::read_link(e.path()).ok()?;
        (target == std::path::Path::new("/")).then(|| e.file_name().to_string_lossy().to_string())
    })
}

/// `1.2 / 2 TB`: used and total, in the unit of the total.
pub fn space_label(total: u64, avail: u64) -> String {
    let used = total.saturating_sub(avail) as f64;
    let total = total as f64;
    let (unit, div) = [("TB", 1e12), ("GB", 1e9), ("MB", 1e6)].into_iter().find(|(_, d)| total >= *d).unwrap_or(("KB", 1e3));
    let n = |v: f64| {
        let x = v / div;
        if x >= 100.0 || (x - x.round()).abs() < 0.05 { format!("{:.0}", x) } else { format!("{:.1}", x) }
    };
    format!("{} / {} {unit}", n(used), n(total))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn df_lines_and_labels() {
        let t = "Filesystem 1024-blocks Used Available Capacity Mounted on\n//admin@LOT48/My Media 1953125000 1171875000 781250000 61% /Volumes/My Media\n";
        assert_eq!(parse_df(t), Some((2_000_000_000_000, 800_000_000_000)));
        assert_eq!(space_label(2_000_000_000_000, 800_000_000_000), "1.2 / 2 TB");
        assert_eq!(space_label(500_000_000_000, 250_000_000_000), "250 / 500 GB");
        assert_eq!(parse_df("garbage"), None);
        assert_eq!(parse_df("h\nfs x y z 5% /"), None);
        assert_eq!(space("/definitely/not/here"), None);
    }
}
