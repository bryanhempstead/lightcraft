//! Catalog backups (Lightroom Classic's Catalog Settings ▸ General ▸ Back up catalog).
//!
//! A backup is a dated folder in the backup folder (default `<library>/Backups/`):
//!
//! ```text
//! Backups/2026-11-21 153012/
//!   catalog.snap.gz   the whole catalog as it was in memory (gzip of a normal catalog.snap)
//!   presets.json  prefs.json  view.json   (copied as they are)
//!   backup.json       manifest: when, how many photos, each file's size and CRC-32
//! ```
//!
//! It is written into `<name>.partial` and renamed once everything (manifest last) is on disk, so
//! a half-written backup never looks like one. With "test the backup" on, it is read back (sizes,
//! CRCs, gunzip, the catalog loaded like a library) before it counts. Only the newest `keep`
//! backups are kept; pruning deletes just the files a manifest lists, never anything else.
//! Restoring ([`crate::Session::restore_backup`]) backs up the current state first.

use std::io::Write;
use std::path::{Path, PathBuf};

use lightcraft_catalog::{Catalog, Journal, MemStore};
use serde::{Deserialize, Serialize};

/// Default backup folder name inside the library.
pub const DIR_NAME: &str = "Backups";
pub const MANIFEST: &str = "backup.json";
pub const SNAPSHOT_GZ: &str = "catalog.snap.gz";
/// The library's settings files a backup keeps.
pub const SETTINGS_FILES: [&str; 3] = ["presets.json", "prefs.json", "view.json"];
const FORMAT: &str = "lightcraft-backup";
const VERSION: u32 = 1;
/// The largest catalog a backup may unpack to (corrupt or hostile size fields can't allocate more).
const MAX_SNAPSHOT: usize = 4 << 30;

/// When LightCraft offers to back up the catalog (asked when quitting, like Lightroom).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum Schedule {
    Never,
    /// Every time LightCraft quits.
    EveryExit,
    /// Once a day, when quitting.
    Daily,
    /// Once a week, when quitting (Lightroom's default).
    #[default]
    Weekly,
    /// Once a month, when quitting.
    Monthly,
    /// The next time LightCraft quits (then back to weekly).
    NextExit,
}

impl Schedule {
    pub fn parse(s: &str) -> Option<Schedule> {
        Some(match s {
            "never" => Schedule::Never,
            "everyExit" | "exit" | "always" => Schedule::EveryExit,
            "daily" | "day" => Schedule::Daily,
            "weekly" | "week" => Schedule::Weekly,
            "monthly" | "month" => Schedule::Monthly,
            "nextExit" | "next" => Schedule::NextExit,
            _ => return None,
        })
    }
    pub fn id(self) -> &'static str {
        match self {
            Schedule::Never => "never",
            Schedule::EveryExit => "everyExit",
            Schedule::Daily => "daily",
            Schedule::Weekly => "weekly",
            Schedule::Monthly => "monthly",
            Schedule::NextExit => "nextExit",
        }
    }
    /// Seconds between backups (`None`: at every quit / never).
    fn interval(self) -> Option<i64> {
        match self {
            Schedule::Daily => Some(86_400),
            Schedule::Weekly => Some(7 * 86_400),
            Schedule::Monthly => Some(30 * 86_400),
            _ => None,
        }
    }
}

/// Backup settings, saved in the library's `prefs.json`.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default, rename_all = "camelCase")]
pub struct BackupPrefs {
    pub schedule: Schedule,
    /// The backup folder (`None`: `Backups` in the library).
    pub dir: Option<String>,
    /// How many backups to keep (older ones are deleted after a new one is verified; 0 = all).
    pub keep: u32,
    /// Read every new backup back (sizes, checksums, the catalog loads) before it counts.
    pub verify: bool,
    /// Compact the catalog (fresh snapshot, empty log) after backing up.
    pub optimize: bool,
    /// When the last backup was made (ISO time, UTC).
    pub last: Option<String>,
}

impl Default for BackupPrefs {
    fn default() -> Self {
        BackupPrefs { schedule: Schedule::Weekly, dir: None, keep: 10, verify: true, optimize: true, last: None }
    }
}

impl BackupPrefs {
    /// A backup is due when quitting at `now` (ISO time).
    pub fn due(&self, now: &str) -> bool {
        match self.schedule {
            Schedule::Never => false,
            Schedule::EveryExit | Schedule::NextExit => true,
            s => match (&self.last, s.interval()) {
                (Some(last), Some(secs)) => lightcraft_catalog::dates::shift_iso(last, secs).is_none_or(|next| next.as_str() <= now),
                _ => true,
            },
        }
    }
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct FileEntry {
    pub name: String,
    pub bytes: u64,
    pub crc32: u32,
}

/// `backup.json`.
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Manifest {
    pub format: String,
    pub version: u32,
    /// ISO time (UTC).
    pub created: String,
    /// The library it was made from.
    pub library: String,
    pub photos: usize,
    pub albums: usize,
    /// The catalog's op number at the time.
    pub seq: u64,
    pub files: Vec<FileEntry>,
    /// Why it was made ("before restoring …"), if not a regular backup.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub note: Option<String>,
}

/// A gzip file (RFC 1952) of `data`.
pub fn gzip(data: &[u8], level: u8) -> Vec<u8> {
    let deflated = miniz_oxide::deflate::compress_to_vec(data, level);
    let mut out = Vec::with_capacity(deflated.len() + 18);
    out.extend_from_slice(&[0x1f, 0x8b, 8, 0, 0, 0, 0, 0, 0, 0xff]);
    out.extend_from_slice(&deflated);
    out.extend_from_slice(&crc32fast::hash(data).to_le_bytes());
    out.extend_from_slice(&(data.len() as u32).to_le_bytes());
    out
}

/// Unpack a gzip file written by [`gzip`] (no optional header fields), checking its CRC and size.
pub fn gunzip(data: &[u8]) -> Result<Vec<u8>, String> {
    let header = data.get(..10).ok_or("not a gzip file (too short)")?;
    if header.first() != Some(&0x1f) || header.get(1) != Some(&0x8b) || header.get(2) != Some(&8) {
        return Err("not a gzip file".into());
    }
    if header.get(3).copied().unwrap_or(0) != 0 {
        return Err("gzip header fields this reader doesn't handle".into());
    }
    let n = data.len();
    let body = data.get(10..n.saturating_sub(8)).ok_or("gzip file cut short")?;
    let tail = data.get(n.saturating_sub(8)..).ok_or("gzip file cut short")?;
    let word = |i: usize| tail.get(i..i + 4).and_then(|b| b.try_into().ok()).map(u32::from_le_bytes).unwrap_or(0);
    let out =
        miniz_oxide::inflate::decompress_to_vec_with_limit(body, MAX_SNAPSHOT).map_err(|e| format!("damaged compressed data ({:?})", e.status))?;
    if crc32fast::hash(&out) != word(0) || out.len() as u32 != word(4) {
        return Err("checksum mismatch: the compressed catalog is damaged".into());
    }
    Ok(out)
}

/// `2026-11-21T15:30:12` → `2026-11-21 153012` (a folder name every file system takes).
pub fn folder_name(created: &str) -> String {
    let (date, time) = created.split_once('T').unwrap_or((created, ""));
    let time: String = time.chars().filter(char::is_ascii_digit).take(6).collect();
    let date: String = date.chars().filter(|c| c.is_ascii_digit() || *c == '-').collect();
    if time.is_empty() { date } else { format!("{date} {time}") }
}

fn write_synced(path: &Path, data: &[u8]) -> std::io::Result<()> {
    let mut f = std::fs::File::create(path)?;
    f.write_all(data)?;
    f.sync_all()
}

/// What goes into a backup.
pub struct Contents<'a> {
    pub catalog: &'a Catalog,
    pub seq: u64,
    /// (name, bytes) of the settings files that exist.
    pub settings: Vec<(&'static str, Vec<u8>)>,
    pub created: &'a str,
    pub library: &'a str,
    pub note: Option<String>,
}

/// Write a backup into a new dated folder in `root`; returns its folder. Nothing that looks like
/// a backup is left behind on failure (the `.partial` folder is removed).
pub fn write(root: &Path, c: &Contents) -> Result<PathBuf, String> {
    std::fs::create_dir_all(root).map_err(|e| format!("can't create the backup folder {}: {e}", root.display()))?;
    let base = folder_name(c.created);
    let mut name = base.clone();
    let mut i = 2;
    while root.join(&name).exists() || root.join(format!("{name}.partial")).exists() {
        if i > 999 {
            return Err(format!("no free backup folder name in {}", root.display()));
        }
        name = format!("{base} ({i})");
        i += 1;
    }
    let partial = root.join(format!("{name}.partial"));
    let done = root.join(&name);
    let result = (|| -> Result<(), String> {
        std::fs::create_dir(&partial).map_err(|e| format!("{}: {e}", partial.display()))?;
        let snap = lightcraft_catalog::journal::snapshot_bytes(c.seq, c.catalog).map_err(|e| format!("catalog: {e}"))?;
        let gz = gzip(&snap, 3);
        drop(snap);
        let mut files = Vec::new();
        let mut put = |name: &str, data: &[u8]| -> Result<(), String> {
            write_synced(&partial.join(name), data).map_err(|e| format!("{name}: {e}"))?;
            files.push(FileEntry { name: name.to_string(), bytes: data.len() as u64, crc32: crc32fast::hash(data) });
            Ok(())
        };
        put(SNAPSHOT_GZ, &gz)?;
        for (n, data) in &c.settings {
            put(n, data)?;
        }
        let manifest = Manifest {
            format: FORMAT.into(),
            version: VERSION,
            created: c.created.to_string(),
            library: c.library.to_string(),
            photos: c.catalog.len(),
            albums: c.catalog.albums().count(),
            seq: c.seq,
            files,
            note: c.note.clone(),
        };
        let json = serde_json::to_vec_pretty(&manifest).map_err(|e| e.to_string())?;
        write_synced(&partial.join(MANIFEST), &json).map_err(|e| format!("{MANIFEST}: {e}"))?;
        std::fs::rename(&partial, &done).map_err(|e| format!("{}: {e}", done.display()))?;
        if let Ok(d) = std::fs::File::open(root) {
            let _ = d.sync_all();
        }
        Ok(())
    })();
    if let Err(e) = result {
        let _ = remove_listed(&partial);
        let _ = std::fs::remove_dir(&partial);
        return Err(e);
    }
    Ok(done)
}

/// Read a backup's manifest (no checks beyond its format).
pub fn manifest(dir: &Path) -> Result<Manifest, String> {
    let bytes = std::fs::read(dir.join(MANIFEST)).map_err(|e| format!("{}: not a LightCraft backup ({e})", dir.display()))?;
    let m: Manifest = serde_json::from_slice(&bytes).map_err(|e| format!("{MANIFEST}: {e}"))?;
    if m.format != FORMAT {
        return Err(format!("{}: not a LightCraft backup", dir.display()));
    }
    if m.version > VERSION {
        return Err(format!("{}: made by a newer LightCraft (backup format v{})", dir.display(), m.version));
    }
    Ok(m)
}

/// What a verified backup holds, ready to restore.
pub struct Verified {
    pub manifest: Manifest,
    /// The plain `catalog.snap`.
    pub snapshot: Vec<u8>,
    pub settings: Vec<(String, Vec<u8>)>,
}

/// Check a backup completely: every listed file is there with its size and CRC-32, the catalog
/// unpacks and loads like a library, and holds the photos the manifest says.
pub fn verify(dir: &Path) -> Result<Verified, String> {
    let manifest = manifest(dir)?;
    let mut snapshot = None;
    let mut settings = Vec::new();
    for f in &manifest.files {
        if f.name.contains(['/', '\\']) || f.name.starts_with('.') {
            return Err(format!("{MANIFEST} lists a file outside the backup: {}", f.name));
        }
        let data = std::fs::read(dir.join(&f.name)).map_err(|e| format!("{}: {e}", f.name))?;
        if data.len() as u64 != f.bytes || crc32fast::hash(&data) != f.crc32 {
            return Err(format!("{} is damaged (size or checksum differs from when it was backed up)", f.name));
        }
        if f.name == SNAPSHOT_GZ {
            snapshot = Some(gunzip(&data).map_err(|e| format!("{SNAPSHOT_GZ}: {e}"))?);
        } else if SETTINGS_FILES.contains(&f.name.as_str()) {
            settings.push((f.name.clone(), data));
        }
    }
    let snapshot = snapshot.ok_or_else(|| format!("{}: the backup has no catalog", dir.display()))?;
    let store = MemStore::new();
    store.set(lightcraft_catalog::journal::SNAPSHOT, snapshot);
    let (_, catalog, _) = Journal::open(Box::new(store.clone())).map_err(|e| format!("the backed-up catalog doesn't load: {e}"))?;
    if catalog.len() != manifest.photos {
        return Err(format!("the backed-up catalog has {} photos, the manifest says {}", catalog.len(), manifest.photos));
    }
    let snapshot = store.get(lightcraft_catalog::journal::SNAPSHOT).unwrap_or_default();
    Ok(Verified { manifest, snapshot, settings })
}

/// The backups in `root`, newest first (folders without a readable manifest are skipped).
pub fn list(root: &Path) -> Vec<(PathBuf, Manifest)> {
    let Ok(rd) = std::fs::read_dir(root) else { return vec![] };
    let mut v: Vec<(PathBuf, Manifest)> = rd
        .flatten()
        .map(|e| e.path())
        .filter(|p| p.is_dir() && p.extension().is_none_or(|x| x != "partial"))
        .filter_map(|p| manifest(&p).ok().map(|m| (p, m)))
        .collect();
    v.sort_by(|a, b| b.1.created.cmp(&a.1.created).then_with(|| b.0.cmp(&a.0)));
    v
}

/// Delete exactly the files a backup's manifest lists (and the manifest), then the folder if it
/// is empty: anything else someone put there stays.
fn remove_listed(dir: &Path) -> Result<(), String> {
    if let Ok(m) = manifest(dir) {
        for f in m.files.iter().filter(|f| !f.name.contains(['/', '\\'])) {
            let _ = std::fs::remove_file(dir.join(&f.name));
        }
    } else {
        // a .partial folder of ours: only the names a backup writes
        for n in SETTINGS_FILES.iter().chain(&[SNAPSHOT_GZ]) {
            let _ = std::fs::remove_file(dir.join(n));
        }
    }
    let _ = std::fs::remove_file(dir.join(MANIFEST));
    std::fs::remove_dir(dir).map_err(|e| format!("{}: {e}", dir.display()))
}

/// Keep the newest `keep` backups in `root` (0 = all); returns the folders deleted.
pub fn prune(root: &Path, keep: u32) -> Vec<PathBuf> {
    if keep == 0 {
        return vec![];
    }
    list(root).into_iter().skip(keep as usize).filter_map(|(p, _)| remove_listed(&p).ok().map(|_| p)).collect()
}
