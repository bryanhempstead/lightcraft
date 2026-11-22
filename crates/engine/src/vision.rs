//! Background indexing for People, Assisted Culling and AI search (`lightcraft_vision`).
//!
//! Two indexes, each a side file in the library folder (`vision/faces.lcidx`, `vision/search.lcidx`; see
//! `lightcraft_vision::index`), keyed by the photo's content key:
//!
//! - **faces**: every face YuNet finds in the photo (box, landmarks, confidence), with its SFace embedding, how
//!   closed each eye is, and how sharp the face is. Found faces also become unnamed face regions on the photo
//!   ([`Session::vision_poll`] merges them in, as LightCraft's bookkeeping, not an undo step); a region that was
//!   already there (named in Lightroom, drawn by hand) gets its embedding from the face found at the same place, or
//!   from its own box when the finder missed it.
//! - **search**: the photo's CLIP embedding, for natural-language search.
//!
//! A run is planned on the session ([`Session::vision_start`]: the photos not indexed yet, each with what to read
//! its pixels from), runs on worker threads (the app) or inline (CLI, MCP, tests), and can be cancelled; because
//! every photo is written to the index as soon as it is done, a quit, a crash or a cancel loses at most the photos
//! in flight, and the next run starts with the rest. Photos can join a running run ([`Session::vision_enqueue`]:
//! an import's new photos go first).
//!
//! **Finding faces on its own** (Lightroom Classic's *Find faces in entire catalog* / *Only find faces as
//! needed*): [`FaceDetect`] in the library's preferences. [`Session::faces_auto_tick`], called by the app every frame,
//! plans a run when there is something to do: every photo of the library (newest first), or the photos the People view
//! shows while it is open. Automatic runs are throttled (half the workers, and they pause while the user is busy:
//! [`touch`]), stop with `stop.` until the user resumes them, and skip photos that failed once this session. Pixels come from a raw's embedded preview when it has one
//! (fast: no raw decode), else from the photo's source decoded at the analysis size; decodes run as background work
//! ([`crate::memory::in_background`]), so the grid and loupe go first.

use std::collections::VecDeque;
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc;
use std::sync::{Arc, Mutex, PoisonError};

use lightcraft_catalog::{PhotoId, Source};
use lightcraft_vision::image::Rgb8;
use lightcraft_vision::index::{Embedding, FaceEntry, FaceRecord, Store};
pub use lightcraft_vision::models::{self, Package};
use serde::Serialize;
use serde_json::{Value, json};

use crate::media::{PreviewLoader, SourceRef};
use crate::{Result, Session};

/// Long edge of the picture faces are found in.
pub const FACE_EDGE: usize = 1280;
/// Long edge read for CLIP (it looks at 224 × 224).
pub const SEARCH_EDGE: usize = 448;
/// The index files' tags: a change of models or of what is stored starts the index over.
/// Automatic runs take a breather after each photo while the user worked within this long.
pub const BUSY_WINDOW_MS: u64 = 1500;

/// When faces are found without being asked (Lightroom Classic: Catalog Settings ▸ Metadata ▸ Face Detection, and
/// the People view's first-run choice).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum FaceDetect {
    /// Only when asked (`find.`, Assisted Culling).
    Off,
    /// The photos the People view shows, while it is open (Lightroom's *Only find faces as needed*).
    #[default]
    AsNeeded,
    /// Every photo of the library, in the background (Lightroom's *Find faces in entire catalog*).
    All,
}

impl FaceDetect {
    pub fn parse(s: &str) -> Option<FaceDetect> {
        match s {
            "off" | "never" => Some(FaceDetect::Off),
            "asNeeded" | "needed" | "people" => Some(FaceDetect::AsNeeded),
            "all" | "catalog" | "library" => Some(FaceDetect::All),
            _ => None,
        }
    }
    pub fn key(self) -> &'static str {
        match self {
            FaceDetect::Off => "off",
            FaceDetect::AsNeeded => "asNeeded",
            FaceDetect::All => "all",
        }
    }
}

/// The library's face preferences (`prefs.json` → `faces`).
#[derive(Clone, Copy, Debug, Default, PartialEq, Serialize, serde::Deserialize)]
#[serde(default, rename_all = "camelCase")]
pub struct FacePrefs {
    pub detect: FaceDetect,
}

/// Milliseconds since the first call, for [`touch`].
fn now_ms() -> u64 {
    static START: std::sync::OnceLock<web_time::Instant> = std::sync::OnceLock::new();
    START.get_or_init(web_time::Instant::now).elapsed().as_millis() as u64
}

static LAST_TOUCH: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);

/// The user is doing something (the app calls this on input): automatic runs hold back for a moment.
pub fn touch() {
    LAST_TOUCH.store(now_ms().max(1), Ordering::Relaxed);
}

/// The user worked within the last `window` milliseconds.
pub fn user_busy(window: u64) -> bool {
    let t = LAST_TOUCH.load(Ordering::Relaxed);
    t != 0 && now_ms().saturating_sub(t) < window
}

pub const FACES_TAG: &str = "faces yunet-2023mar sface-2021dec open-closed-eye-0001 v1";
pub const SEARCH_TAG: &str = "search clip-vit-b32-laion2b v1";

/// Where the models live: `LIGHTCRAFT_VISION_MODELS`, else `~/Library/Application Support/LightCraft/Models/Vision`.
pub fn models_root() -> PathBuf {
    if let Some(d) = std::env::var_os("LIGHTCRAFT_VISION_MODELS") {
        return PathBuf::from(d);
    }
    match std::env::var_os("HOME") {
        Some(h) => PathBuf::from(h).join("Library/Application Support/LightCraft/Models/Vision"),
        None => std::env::temp_dir().join("LightCraft Vision Models"),
    }
}

/// Which index.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize)]
#[serde(rename_all = "camelCase")]
pub enum Kind {
    Faces,
    Search,
}

impl Kind {
    pub fn package(self) -> Package {
        match self {
            Kind::Faces => Package::Faces,
            Kind::Search => Package::Search,
        }
    }
    pub fn parse(s: &str) -> Option<Kind> {
        match s {
            "faces" | "people" => Some(Kind::Faces),
            "search" | "clip" => Some(Kind::Search),
            _ => None,
        }
    }
}

/// How far a run has got.
#[derive(Clone, Debug, Default, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Progress {
    pub running: bool,
    pub done: usize,
    pub total: usize,
    pub failed: usize,
    /// Faces found so far in this run.
    pub faces: usize,
    /// Photos a second (since the run started).
    pub rate: f64,
    pub cancelled: bool,
    /// Started by [`Session::faces_auto_tick`] (throttled), not by the user.
    pub auto: bool,
    /// Why the run stopped early (e.g. the model is missing).
    pub error: Option<String>,
    #[serde(skip)]
    started: Option<web_time::Instant>,
}

/// What a photo's pixels are read from (all `Send`).
#[derive(Clone)]
pub struct PixelSource {
    /// A raw's embedded preview (path, loader).
    pub embedded: Option<(String, PreviewLoader)>,
    /// A standard image file read straight to 8-bit sRGB at the size needed (JPEG's scaled decode; no float
    /// pipeline), before `source`.
    pub file: Option<String>,
    pub source: SourceRef,
}

/// A standard image file (JPEG, PNG, TIFF, WebP…) decoded at about `edge` as sRGB, upright.
#[cfg(not(target_arch = "wasm32"))]
fn decode_file(path: &str, edge: usize) -> Option<Rgb8> {
    let len = std::fs::metadata(path).ok()?.len();
    if len > 512 << 20 {
        return None;
    }
    let _permit = crate::memory::work_gate().acquire(len as usize * 3);
    let bytes = std::fs::read(path).ok()?;
    let d = lightcraft_codecs::decode(&bytes, lightcraft_codecs::DecodeOptions::fit(edge as u32, edge as u32)).ok()?;
    let img = d.to_srgb8().oriented(lightcraft_geom::Orientation::from_exif(d.orientation));
    let data: Vec<u8> = img.data.iter().flat_map(|p| [p[0], p[1], p[2]]).collect();
    Rgb8::new(img.width, img.height, data).map(|r| r.fit(edge).0)
}

#[cfg(target_arch = "wasm32")]
fn decode_file(_: &str, _: usize) -> Option<Rgb8> {
    None
}

impl PixelSource {
    /// The picture, sRGB, no larger than `edge` along its long side.
    pub fn load(&self, edge: usize) -> std::result::Result<Rgb8, String> {
        if let Some((path, l)) = &self.embedded
            && let Some(img) = l(path, edge)
        {
            let bytes: Vec<u8> = img.data.iter().flat_map(|p| [p[0], p[1], p[2]]).collect();
            if let Some(rgb) = Rgb8::new(img.width, img.height, bytes) {
                return Ok(rgb.fit(edge).0);
            }
        }
        if let Some(img) = self.file.as_deref().and_then(|p| decode_file(p, edge)) {
            return Ok(img);
        }
        let src = self.source.load()?;
        let m = lightcraft_color::REC2020.to_space(&lightcraft_color::SRGB).to_f32();
        use lightcraft_color::transfer::encode_srgb8;
        let mut bytes = Vec::with_capacity(src.width * src.height * 3);
        for p in &src.data {
            let c = [0, 1, 2].map(|i| m[i][0] * p[0] + m[i][1] * p[1] + m[i][2] * p[2]);
            bytes.extend(c.map(|v| encode_srgb8(if v.is_finite() { v.clamp(0.0, 1.0) } else { 0.0 })));
        }
        let rgb = Rgb8::new(src.width, src.height, bytes).ok_or("the decoded picture has the wrong size")?;
        Ok(rgb.fit(edge).0)
    }
}

/// One photo to index.
#[derive(Clone)]
pub struct Item {
    pub id: PhotoId,
    pub key: String,
    pub pixels: PixelSource,
    /// The photo's named and hand-drawn face regions (`[x, y, w, h]`, normalized, upright): they get embeddings even
    /// where the finder misses them. (Unnamed regions from other apps that the finder doesn't confirm are left alone:
    /// phones write boxes that don't always sit on a face.)
    pub regions: Vec<[f32; 4]>,
}

/// What a worker sends back.
pub enum Event {
    Faces {
        id: PhotoId,
        generation: u64,
        record: FaceRecord,
    },
    Embedded {
        id: PhotoId,
    },
    /// The photo couldn't be analysed (unreadable, …): automatic runs don't try it again this session.
    Failed {
        key: String,
    },
}

/// The photos waiting for a run's workers (more can join while it runs).
type Queue = Arc<Mutex<VecDeque<Item>>>;

struct Running {
    kind: Kind,
    cancel: Arc<AtomicBool>,
    progress: Arc<Mutex<Progress>>,
    thread: Option<std::thread::JoinHandle<()>>,
    queue: Queue,
}

/// A model download in the background.
struct Download {
    package: Package,
    cancel: Arc<AtomicBool>,
    state: Arc<Mutex<DownloadState>>,
}

#[derive(Clone, Debug, Default, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct DownloadState {
    pub running: bool,
    pub done: u64,
    pub total: u64,
    pub error: Option<String>,
    pub finished: bool,
}

/// The session's recognition state: models folder, the open indexes, the running jobs.
pub struct Vision {
    /// The models folder (`<root>/faces/`, `<root>/search/`).
    pub root: PathBuf,
    /// Run indexing on worker threads and apply results in [`Session::vision_poll`] (the desktop app); otherwise
    /// [`Session::vision_start`] runs to the end before it returns (CLI, MCP, tests).
    pub background: bool,
    /// Where the index files go; `None`: `<library>/vision/`, or a temporary folder for a library not on disk.
    pub dir: Option<PathBuf>,
    faces: Option<(PathBuf, Arc<Mutex<Store<FaceRecord>>>)>,
    search: Option<(PathBuf, Arc<Mutex<Store<Embedding>>>)>,
    running: Vec<Running>,
    results: (mpsc::Sender<Event>, mpsc::Receiver<Event>),
    downloads: Vec<Download>,
    /// Bumped whenever an index changes (caches of clusters and searches key on it).
    pub generation: u64,
    /// The CLIP model for text queries, kept loaded between searches.
    #[cfg(not(target_arch = "wasm32"))]
    clip: Option<Arc<lightcraft_vision::clip::Clip>>,
    /// The last search: query, index generation, ranked photos.
    search_cache: Option<(String, u64, Arc<Vec<(PhotoId, f32)>>)>,
    messages: Vec<String>,
    /// Automatic face finding was stopped by the user (until [`Vision::resume_auto`] or the next library).
    pub auto_paused: bool,
    /// What the last automatic planning looked at (catalog revision, index generation, scope) and when, so an
    /// unchanged library isn't planned again every frame.
    auto_planned: Option<((u64, u64, bool), web_time::Instant)>,
    /// Content keys that failed this session (automatic runs skip them).
    failed: std::collections::HashSet<String>,
    /// The photos the last automatic run was planned with, in order (tests, `faces.auto`).
    pub auto_last_plan: Vec<PhotoId>,
    /// The newest photo the automatic run knows of: newer ones (an import) join its front.
    auto_newest: PhotoId,
}

impl Default for Vision {
    fn default() -> Self {
        Vision {
            root: models_root(),
            background: false,
            dir: None,
            faces: None,
            search: None,
            running: Vec::new(),
            results: mpsc::channel(),
            downloads: Vec::new(),
            generation: 0,
            #[cfg(not(target_arch = "wasm32"))]
            clip: None,
            search_cache: None,
            messages: Vec::new(),
            auto_paused: false,
            auto_planned: None,
            failed: Default::default(),
            auto_last_plan: Vec::new(),
            auto_newest: PhotoId(0),
        }
    }
}

fn lock<T>(m: &Mutex<T>) -> std::sync::MutexGuard<'_, T> {
    m.lock().unwrap_or_else(PoisonError::into_inner)
}

impl Vision {
    pub fn installed(&self, p: Package) -> bool {
        models::installed(&self.root, p)
    }

    /// The running (or last) run of `kind`.
    pub fn progress(&self, kind: Kind) -> Progress {
        self.running.iter().find(|r| r.kind == kind).map(|r| lock(&r.progress).clone()).unwrap_or_default()
    }

    pub fn busy(&self) -> bool {
        self.running.iter().any(|r| lock(&r.progress).running)
    }

    /// A run of `kind` is going.
    pub fn running(&self, kind: Kind) -> bool {
        self.running.iter().any(|r| r.kind == kind && lock(&r.progress).running)
    }

    /// Stop the running run(s) (`None`: all). Stopping automatic face finding pauses it until it is resumed.
    pub fn cancel(&mut self, kind: Option<Kind>) {
        for r in &self.running {
            if kind.is_none_or(|k| k == r.kind) {
                r.cancel.store(true, Ordering::Relaxed);
                let p = lock(&r.progress);
                if r.kind == Kind::Faces && p.auto && p.running {
                    self.auto_paused = true;
                }
            }
        }
    }

    /// A run of `kind` that looks busy and has no workers (tests of queueing).
    #[cfg(test)]
    pub(crate) fn fake_running(&mut self, kind: Kind, auto: bool) {
        let progress = Arc::new(Mutex::new(Progress { running: true, auto, ..Default::default() }));
        self.running.retain(|r| r.kind != kind);
        self.running.push(Running { kind, cancel: Arc::new(AtomicBool::new(false)), progress, thread: None, queue: Default::default() });
    }

    /// Mark the fake run of `kind` finished (its workers "ended") and hand back its thread slot for joining.
    #[cfg(test)]
    pub(crate) fn fake_finish(&mut self, kind: Kind) {
        for r in &mut self.running {
            if r.kind == kind {
                lock(&r.progress).running = false;
                r.thread = std::thread::Builder::new().spawn(|| {}).ok();
            }
        }
    }

    /// The photos a run of `kind` still has queued, in order.
    #[cfg(test)]
    pub(crate) fn queued(&self, kind: Kind) -> Vec<PhotoId> {
        self.running.iter().filter(|r| r.kind == kind).flat_map(|r| lock(&r.queue).iter().map(|i| i.id).collect::<Vec<_>>()).collect()
    }

    /// Let automatic face finding go on again.
    pub fn resume_auto(&mut self) {
        self.auto_paused = false;
        self.auto_planned = None;
    }

    /// Messages for the user (failed runs), once.
    pub fn take_messages(&mut self) -> Vec<String> {
        std::mem::take(&mut self.messages)
    }

    pub fn download_state(&self, p: Package) -> DownloadState {
        self.downloads.iter().rev().find(|d| d.package == p).map(|d| lock(&d.state).clone()).unwrap_or_default()
    }

    /// Start downloading `p` (in the background with `background`, else to the end). The caller has the user's
    /// consent (size and licence shown).
    pub fn download(&mut self, p: Package) -> std::result::Result<(), String> {
        #[cfg(target_arch = "wasm32")]
        {
            let _ = p;
            Err("model downloads aren't available here".into())
        }
        #[cfg(not(target_arch = "wasm32"))]
        {
            if self.downloads.iter().any(|d| d.package == p && lock(&d.state).running) {
                return Ok(());
            }
            let cancel = Arc::new(AtomicBool::new(false));
            let state = Arc::new(Mutex::new(DownloadState { running: true, total: models::info(p).size(), ..Default::default() }));
            let (root, c2, s2) = (self.root.clone(), cancel.clone(), state.clone());
            let work = move || {
                let r = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
                    models::download(&root, p, &c2, &mut |done, total| {
                        let mut s = lock(&s2);
                        s.done = done;
                        s.total = total;
                    })
                }))
                .unwrap_or_else(|_| Err("the download stopped unexpectedly".into()));
                let mut s = lock(&s2);
                s.running = false;
                s.finished = r.is_ok();
                s.error = r.err();
            };
            self.downloads.retain(|d| d.package != p);
            self.downloads.push(Download { package: p, cancel, state: state.clone() });
            if self.background {
                std::thread::Builder::new().name("vision-download".into()).spawn(work).map_err(|e| e.to_string())?;
                Ok(())
            } else {
                work();
                match lock(&state).error.clone() {
                    Some(e) => Err(e),
                    None => Ok(()),
                }
            }
        }
    }

    pub fn cancel_download(&mut self, p: Package) {
        for d in &self.downloads {
            if d.package == p {
                d.cancel.store(true, Ordering::Relaxed);
            }
        }
    }

    /// Forget the open indexes (another library was opened).
    pub fn reset(&mut self) {
        self.cancel(None);
        self.faces = None;
        self.search = None;
        self.search_cache = None;
        self.generation += 1;
        self.auto_paused = false;
        self.auto_planned = None;
        self.auto_newest = PhotoId(0);
        self.failed.clear();
    }
}

impl Session {
    /// The folder the index files of this library go in.
    pub fn vision_dir(&self) -> PathBuf {
        if let Some(d) = &self.vision.dir {
            return d.clone();
        }
        match self.library.as_ref().filter(|l| l.on_disk) {
            Some(l) => l.dir.join("vision"),
            None => std::env::temp_dir().join(format!("lightcraft-vision-{}-{}", std::process::id(), self.library_generation())),
        }
    }

    /// The faces index of this library (opened on first use).
    pub fn faces_index(&mut self) -> Result<Arc<Mutex<Store<FaceRecord>>>> {
        let dir = self.vision_dir();
        if let Some((d, s)) = &self.vision.faces
            && *d == dir
        {
            return Ok(s.clone());
        }
        let s = Arc::new(Mutex::new(Store::open(&dir.join("faces.lcidx"), FACES_TAG).map_err(|e| crate::EngineError::Other(e.to_string()))?));
        self.vision.faces = Some((dir, s.clone()));
        self.vision.generation += 1;
        Ok(s)
    }

    /// The search index of this library (opened on first use).
    pub fn search_index(&mut self) -> Result<Arc<Mutex<Store<Embedding>>>> {
        let dir = self.vision_dir();
        if let Some((d, s)) = &self.vision.search
            && *d == dir
        {
            return Ok(s.clone());
        }
        let s = Arc::new(Mutex::new(Store::open(&dir.join("search.lcidx"), SEARCH_TAG).map_err(|e| crate::EngineError::Other(e.to_string()))?));
        self.vision.search = Some((dir, s.clone()));
        self.vision.generation += 1;
        Ok(s)
    }

    /// What `id`'s pixels are read from for analysis at `edge`.
    pub fn pixel_source(&mut self, id: PhotoId, edge: usize) -> Option<PixelSource> {
        let p = self.catalog.photo(id)?.clone();
        let embedded = match (&p.source, &self.media.preview_loader) {
            (Source::File { path }, Some(l)) if p.kind == lightcraft_catalog::MediaKind::Raw => Some((path.clone(), l.clone())),
            _ => None,
        };
        // (only with the file system hooks: the web and tests without files read through the loader)
        let file = match (&p.source, &self.media.file_loader) {
            (Source::File { path }, Some(_)) if p.kind == lightcraft_catalog::MediaKind::Image && self.media.preview_loader.is_some() => {
                Some(path.clone())
            }
            _ => None,
        };
        let mut source = self.media.source_ref(&p, crate::media::SourceLevel::for_size(edge));
        if let SourceRef::File { max_edge, .. } = &mut source {
            *max_edge = edge;
        }
        Some(PixelSource { embedded, file, source })
    }

    /// The photos of `ids` an index of `kind` doesn't hold yet (all of them with `force`), as work items.
    pub fn vision_items(&mut self, kind: Kind, ids: &[PhotoId], force: bool) -> Result<Vec<Item>> {
        let edge = match kind {
            Kind::Faces => FACE_EDGE,
            Kind::Search => SEARCH_EDGE,
        };
        // with the expression model installed, photos analysed before it count as not done
        let expressions = self.vision.installed(Package::Expressions);
        let have: std::collections::HashSet<String> = match kind {
            Kind::Faces => {
                let idx = self.faces_index()?;
                lock(&idx).iter().filter(|(_, r)| r.expressions || !expressions).map(|(k, _)| k.clone()).collect()
            }
            Kind::Search => {
                let idx = self.search_index()?;
                lock(&idx).iter().map(|(k, _)| k.clone()).collect()
            }
        };
        let mut out = Vec::new();
        let mut seen = std::collections::HashSet::new();
        for id in ids {
            let Some(p) = self.catalog.photo(*id) else { continue };
            if !p.in_library() || p.kind == lightcraft_catalog::MediaKind::Video || !seen.insert(*id) {
                continue;
            }
            let key = crate::media::content_key(p);
            if !force && have.contains(&key) {
                continue;
            }
            let regions = p
                .meta
                .regions
                .iter()
                .filter(|r| r.kind == lightcraft_meta::RegionKind::Face && (r.name.is_some() || r.drawn))
                .map(|r| [r.rect.x0 as f32, r.rect.y0 as f32, (r.rect.x1 - r.rect.x0) as f32, (r.rect.y1 - r.rect.y0) as f32])
                .collect();
            let Some(pixels) = self.pixel_source(*id, edge) else { continue };
            out.push(Item { id: *id, key, pixels, regions });
        }
        Ok(out)
    }

    /// Index `ids` for `kind` (those not done yet, or all with `force`): in the background with
    /// `vision.background`, else to the end. Returns how many photos were queued.
    pub fn vision_start(&mut self, kind: Kind, ids: &[PhotoId], force: bool) -> Result<usize> {
        if !self.vision.installed(kind.package()) {
            return Err(crate::EngineError::Other(models::missing_message(&self.vision.root, kind.package())));
        }
        let items = self.vision_items(kind, ids, force)?;
        self.vision_run(kind, items, false)
    }

    /// Queue `ids` (those not done yet) for `kind`: ahead of what a running run still has to do, else in a new run.
    /// Returns how many photos were queued.
    pub fn vision_enqueue(&mut self, kind: Kind, ids: &[PhotoId]) -> Result<usize> {
        if !self.vision.installed(kind.package()) {
            return Err(crate::EngineError::Other(models::missing_message(&self.vision.root, kind.package())));
        }
        let items = self.vision_items(kind, ids, false)?;
        if items.is_empty() {
            return Ok(0);
        }
        if let Some(r) = self.vision.running.iter().find(|r| r.kind == kind) {
            // under the progress lock: the workers can't be finishing while the photos go in
            let mut p = lock(&r.progress);
            if p.running && !r.cancel.load(Ordering::Relaxed) {
                let mut q = lock(&r.queue);
                let queued: std::collections::HashSet<PhotoId> = q.iter().map(|i| i.id).collect();
                let new: Vec<Item> = items.into_iter().filter(|i| !queued.contains(&i.id)).collect();
                let n = new.len();
                for item in new.into_iter().rev() {
                    q.push_front(item);
                }
                p.total += n;
                return Ok(n);
            }
        }
        self.vision_run(kind, items, false)
    }

    /// Start a run over `items` (`auto`: planned by [`Session::faces_auto_tick`], throttled).
    fn vision_run(&mut self, kind: Kind, items: Vec<Item>, auto: bool) -> Result<usize> {
        if !self.vision.installed(kind.package()) {
            return Err(crate::EngineError::Other(models::missing_message(&self.vision.root, kind.package())));
        }
        if self.vision.running.iter().any(|r| r.kind == kind && lock(&r.progress).running) {
            return Err(crate::EngineError::Other(format!("{} indexing is already running", if kind == Kind::Faces { "face" } else { "search" })));
        }
        let n = items.len();
        let cancel = Arc::new(AtomicBool::new(false));
        let progress =
            Arc::new(Mutex::new(Progress { running: true, total: n, auto, started: Some(web_time::Instant::now()), ..Default::default() }));
        let queue: Queue = Arc::new(Mutex::new(VecDeque::from(items)));
        let dir = models::dir(&self.vision.root, kind.package());
        let expr_dir = self.vision.installed(Package::Expressions).then(|| models::dir(&self.vision.root, Package::Expressions));
        let tx = self.vision.results.0.clone();
        let generation = self.library_generation();
        #[cfg(not(target_arch = "wasm32"))]
        let work: Box<dyn FnOnce() + Send> = match kind {
            Kind::Faces => {
                let store = self.faces_index()?;
                let (c, p, q) = (cancel.clone(), progress.clone(), queue.clone());
                Box::new(move || worker::faces(q, dir, expr_dir, store, tx, c, p, generation))
            }
            Kind::Search => {
                let store = self.search_index()?;
                let (c, p, q) = (cancel.clone(), progress.clone(), queue.clone());
                Box::new(move || worker::search(q, dir, store, tx, c, p))
            }
        };
        #[cfg(target_arch = "wasm32")]
        let work: Box<dyn FnOnce() + Send> = {
            let _ = (dir, expr_dir, tx, generation);
            let p = progress.clone();
            Box::new(move || {
                let mut g = lock(&p);
                g.running = false;
                g.error = Some("recognition isn't available here".into());
            })
        };
        self.vision.running.retain(|r| r.kind != kind);
        if self.vision.background {
            let thread = std::thread::Builder::new()
                .name(format!("vision-{kind:?}").to_lowercase())
                .spawn(work)
                .map_err(|e| crate::EngineError::Other(format!("can't start indexing: {e}")))?;
            self.vision.running.push(Running { kind, cancel, progress, thread: Some(thread), queue });
        } else {
            work();
            self.vision.running.push(Running { kind, cancel, progress, thread: None, queue });
            self.vision_poll();
            if let Some(e) = self.vision.progress(kind).error {
                return Err(crate::EngineError::Other(e));
            }
        }
        Ok(n)
    }

    /// Apply what the workers finished: found faces become unnamed face regions (one bookkeeping batch); report
    /// runs that ended with an error. Returns whether anything changed.
    pub fn vision_poll(&mut self) -> bool {
        let mut ops = Vec::new();
        let mut changed = false;
        let gen_now = self.library_generation();
        while let Ok(ev) = self.vision.results.1.try_recv() {
            changed = true;
            match ev {
                Event::Faces { id, generation, record } if generation == gen_now => {
                    if let Some(op) = self.merge_found_faces(id, &record) {
                        ops.push(op);
                    }
                    // a photo culled before its faces were known gets its face scores now
                    if let Some(a) = self.catalog.photo(id).and_then(|p| p.analysis) {
                        let f = crate::cmd::cull_scores(&record);
                        let b = lightcraft_catalog::Analysis {
                            faces: f.faces,
                            eyes_closed: f.eyes_closed,
                            face_focus: f.focus,
                            smiles: f.smiles,
                            expression: f.expression,
                            ..a
                        };
                        if b != a {
                            ops.push(lightcraft_catalog::Op::SetAnalysis { id, analysis: Some(b) });
                        }
                    }
                }
                Event::Failed { key } => {
                    self.vision.failed.insert(key);
                }
                Event::Faces { .. } | Event::Embedded { .. } => {}
            }
        }
        if !ops.is_empty()
            && let Err(e) = self.apply_bookkeeping(lightcraft_catalog::Op::Batch { ops })
        {
            log::warn!("faces: {e}");
        }
        // finished runs: join their threads, report errors once; photos queued as a run was ending start a new one
        let mut leftovers: Vec<(Kind, Vec<PhotoId>)> = Vec::new();
        for r in &mut self.vision.running {
            let p = lock(&r.progress).clone();
            if !p.running
                && let Some(t) = r.thread.take()
            {
                if t.join().is_err() {
                    self.vision.messages.push("indexing stopped unexpectedly".into());
                }
                if !p.cancelled && p.error.is_none() {
                    let left: Vec<PhotoId> = lock(&r.queue).drain(..).map(|i| i.id).collect();
                    if !left.is_empty() {
                        leftovers.push((r.kind, left));
                    }
                }
                if let Some(e) = p.error.filter(|_| !p.cancelled) {
                    self.vision.messages.push(e);
                }
                changed = true;
            }
        }
        for (kind, ids) in leftovers {
            if let Err(e) = self.vision_start(kind, &ids, false) {
                log::warn!("vision: {e}");
            }
        }
        if changed {
            self.vision.generation += 1;
        }
        changed
    }

    /// The face regions `record` adds to photo `id` (faces found where it has none), as one op.
    pub(crate) fn merge_found_faces(&self, id: PhotoId, record: &FaceRecord) -> Option<lightcraft_catalog::Op> {
        let p = self.catalog.photo(id)?;
        let mut regions = p.meta.regions.clone();
        let mut added = false;
        for f in record.faces.iter().filter(|f| f.score > 0.0) {
            let rect = to_rect(f.rect);
            if rect.x1 - rect.x0 < 0.002 || rect.y1 - rect.y0 < 0.002 {
                continue;
            }
            if regions.iter().any(|r| iou(&r.rect, &rect) >= 0.3) {
                continue;
            }
            regions.push(lightcraft_meta::Region::face(rect));
            added = true;
        }
        added.then_some(lightcraft_catalog::Op::SetRegions { id, regions })
    }
}

impl Session {
    /// Find faces without being asked, as [`FacePrefs::detect`] says ([`FaceDetect::All`]: the whole library, newest
    /// first; [`FaceDetect::AsNeeded`]: the photos in view while the People view is open). Called by the app every
    /// frame; it plans a run only when the library or the index changed since it last looked (at most every few
    /// seconds) and nothing is running. Returns how many photos it queued.
    pub fn faces_auto_tick(&mut self, people_open: bool) -> usize {
        let active = match self.face_prefs.detect {
            FaceDetect::Off => false,
            FaceDetect::AsNeeded => people_open,
            FaceDetect::All => true,
        };
        if !active || self.vision.auto_paused {
            return 0;
        }
        // cheap checks first: this runs every frame
        let key = (self.catalog.revision, self.vision.generation, people_open);
        if let Some((k, at)) = &self.vision.auto_planned
            && (*k == key || at.elapsed() < std::time::Duration::from_secs(3))
        {
            return 0;
        }
        self.vision.auto_planned = Some((key, web_time::Instant::now()));
        let newest = self.catalog.photos().map(|p| p.id).max().unwrap_or(PhotoId(0));
        if self.vision.running(Kind::Faces) {
            // photos imported while the whole library is being analysed go to the front of the run
            let p = self.vision.progress(Kind::Faces);
            if p.auto && self.face_prefs.detect == FaceDetect::All && newest > self.vision.auto_newest {
                let since = self.vision.auto_newest;
                let mut new: Vec<PhotoId> = self.catalog.photos().filter(|p| p.in_library() && p.id > since).map(|p| p.id).collect();
                new.sort_unstable_by(|a, b| b.cmp(a));
                self.vision.auto_newest = newest;
                return self.vision_enqueue(Kind::Faces, &new).unwrap_or(0);
            }
            return 0;
        }
        if !self.vision.installed(Package::Faces) {
            return 0;
        }
        self.vision.auto_newest = newest;
        let mut ids: Vec<PhotoId> = match self.face_prefs.detect {
            FaceDetect::All => self.catalog.photos().filter(|p| p.in_library()).map(|p| p.id).collect(),
            _ => self.visible_cloned(),
        };
        // newest first: an import's photos before the backlog
        ids.sort_unstable_by(|a, b| b.cmp(a));
        let items = match self.vision_items(Kind::Faces, &ids, false) {
            Ok(v) => v,
            Err(e) => {
                log::warn!("faces: {e}");
                return 0;
            }
        };
        let items: Vec<Item> = items.into_iter().filter(|i| !self.vision.failed.contains(&i.key)).collect();
        if items.is_empty() {
            return 0;
        }
        self.vision.auto_last_plan = items.iter().map(|i| i.id).collect();
        match self.vision_run(Kind::Faces, items, true) {
            Ok(n) => n,
            Err(e) => {
                log::warn!("faces: {e}");
                0
            }
        }
    }
}

/// AI search results: scores at least this (CLIP cosine) …
pub const SEARCH_MIN: f32 = 0.2;
/// … and no further than this below the best one …
pub const SEARCH_SPREAD: f32 = 0.07;
/// … at most this many.
pub const SEARCH_MAX: usize = 1000;

impl Session {
    /// The photos matching a natural-language `query`, best first, with their CLIP similarity (cached by query and
    /// index). Errors when the search model isn't installed or no photo is indexed yet.
    pub fn semantic_rank(&mut self, query: &str) -> Result<Arc<Vec<(PhotoId, f32)>>> {
        let q = query.split_whitespace().collect::<Vec<_>>().join(" ");
        if let Some((cq, g, r)) = &self.vision.search_cache
            && *cq == q
            && *g == self.vision.generation
        {
            return Ok(r.clone());
        }
        let index = self.search_index()?;
        if lock(&index).is_empty() {
            return Err(crate::EngineError::Other("no photo is indexed for AI search yet: run Library ▸ Index for AI Search (search.index)".into()));
        }
        let text = self.embed_query(&q)?;
        let mut scored: Vec<(PhotoId, f32)> = {
            let idx = lock(&index);
            self.catalog
                .photos()
                .filter(|p| p.in_library())
                .filter_map(|p| idx.get(&crate::media::content_key(p)).map(|e| (p.id, lightcraft_vision::sface::cosine(&e.0, &text))))
                .filter(|(_, v)| v.is_finite())
                .collect()
        };
        scored.sort_by(|a, b| b.1.total_cmp(&a.1).then(a.0.cmp(&b.0)));
        let best = scored.first().map(|s| s.1).unwrap_or(0.0);
        scored.retain(|(_, v)| *v >= SEARCH_MIN && *v >= best - SEARCH_SPREAD);
        scored.truncate(SEARCH_MAX);
        let r = Arc::new(scored.into_iter().map(|(id, v)| (id, (v * 1000.0).round() / 1000.0)).collect::<Vec<_>>());
        self.vision.search_cache = Some((q, self.vision.generation, r.clone()));
        Ok(r)
    }

    /// The CLIP embedding of a phrase (the model is loaded once and kept).
    #[cfg(not(target_arch = "wasm32"))]
    fn embed_query(&mut self, q: &str) -> Result<Vec<f32>> {
        if !self.vision.installed(Package::Search) {
            return Err(crate::EngineError::Other(models::missing_message(&self.vision.root, Package::Search)));
        }
        let clip = match &self.vision.clip {
            Some(c) => c.clone(),
            None => {
                let dir = models::dir(&self.vision.root, Package::Search);
                let device = device();
                let c = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| lightcraft_vision::clip::Clip::load(&dir, &device)))
                    .map_err(|_| crate::EngineError::Other("the search model couldn't be loaded".into()))?
                    .map_err(|e| crate::EngineError::Other(e.to_string()))?;
                let c = Arc::new(c);
                self.vision.clip = Some(c.clone());
                c
            }
        };
        std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            let _gpu = gpu_lock(clip.device());
            clip.embed_text(q)
        }))
        .map_err(|_| crate::EngineError::Other("the search model failed".into()))?
        .map_err(|e| crate::EngineError::Other(e.to_string()))
    }

    #[cfg(target_arch = "wasm32")]
    fn embed_query(&mut self, _: &str) -> Result<Vec<f32>> {
        Err(crate::EngineError::Other("AI search isn't available here".into()))
    }
}

/// A face entry's `[x, y, w, h]` as a clamped rectangle.
pub fn to_rect(r: [f32; 4]) -> lightcraft_meta::Rect {
    let c = |v: f32| if v.is_finite() { f64::from(v).clamp(0.0, 1.0) } else { 0.0 };
    lightcraft_meta::Rect { x0: c(r[0]), y0: c(r[1]), x1: c(r[0] + r[2]), y1: c(r[1] + r[3]) }
}

pub fn iou(a: &lightcraft_meta::Rect, b: &lightcraft_meta::Rect) -> f64 {
    let (x0, y0, x1, y1) = (a.x0.max(b.x0), a.y0.max(b.y0), a.x1.min(b.x1), a.y1.min(b.y1));
    let inter = (x1 - x0).max(0.0) * (y1 - y0).max(0.0);
    let union = (a.x1 - a.x0) * (a.y1 - a.y0) + (b.x1 - b.x0) * (b.y1 - b.y0) - inter;
    if union > 0.0 { inter / union } else { 0.0 }
}

/// The indexed face at (about) `rect` in `record`.
pub fn entry_for<'a>(record: &'a FaceRecord, rect: &lightcraft_meta::Rect) -> Option<&'a FaceEntry> {
    record.faces.iter().map(|f| (f, iou(&to_rect(f.rect), rect))).filter(|(_, v)| *v >= 0.3).max_by(|a, b| a.1.total_cmp(&b.1)).map(|(f, _)| f)
}

/// What the commands report about the models and indexes.
pub fn status_json(s: &mut Session) -> Value {
    let faces = s.faces_index().map(|i| lock(&i).len()).unwrap_or(0);
    let search = s.search_index().map(|i| lock(&i).len()).unwrap_or(0);
    let (detect, paused) = (s.face_prefs.detect, s.vision.auto_paused);
    let v = &s.vision;
    let pkg = |p: Package| {
        let i = models::info(p);
        json!({
            "installed": v.installed(p), "name": i.name, "models": i.models, "licence": i.licence,
            "sizeMB": (i.size() as f64 / 1e6).round(), "source": i.source, "folder": models::dir(&v.root, p),
            "download": v.download_state(p),
        })
    };
    json!({
        "faces": {
            "model": pkg(Package::Faces), "expressions": pkg(Package::Expressions), "indexed": faces, "progress": v.progress(Kind::Faces),
            "detect": detect.key(), "autoPaused": paused,
        },
        "search": {"model": pkg(Package::Search), "indexed": search, "progress": v.progress(Kind::Search)},
    })
}

#[cfg(not(target_arch = "wasm32"))]
pub(crate) mod worker {
    use super::*;
    use lightcraft_vision::Face;
    use lightcraft_vision::{eyes::EyeState, sface::SFace, yunet::YuNet};

    /// Workers decoding and analysing photos at once (each analysis is itself parallel); automatic runs take half.
    fn threads(auto: bool) -> usize {
        let n = std::thread::available_parallelism().map(|n| (n.get() / 3).clamp(1, 4)).unwrap_or(2);
        if auto { n.div_ceil(2) } else { n }
    }

    /// An automatic run waits while the user is busy (a frame or two of breathing room, re-checked), at most a few
    /// seconds a photo, and stops waiting when cancelled.
    fn pace(auto: bool, cancel: &AtomicBool) {
        if !auto {
            return;
        }
        for _ in 0..20 {
            if cancel.load(Ordering::Relaxed) || !user_busy(BUSY_WINDOW_MS) {
                return;
            }
            std::thread::sleep(std::time::Duration::from_millis(150));
        }
    }

    fn fail(progress: &Mutex<Progress>, e: impl Into<String>) {
        let mut p = lock(progress);
        p.running = false;
        p.error = Some(e.into());
    }

    fn tick(progress: &Mutex<Progress>, ok: bool, faces: usize) {
        let mut p = lock(progress);
        p.done += 1;
        if !ok {
            p.failed += 1;
        }
        p.faces += faces;
        if let Some(t) = p.started {
            let s = t.elapsed().as_secs_f64();
            if s > 0.0 {
                p.rate = p.done as f64 / s;
            }
        }
    }

    struct FaceModels {
        det: YuNet,
        rec: SFace,
        eyes: EyeState,
        /// The expression network, when installed.
        expr: Option<lightcraft_vision::expression::ExpressionNet>,
    }

    /// The face indexer: every queued item through detection, embedding, eyes, expression and focus.
    #[allow(clippy::too_many_arguments)]
    pub fn faces(
        queue: Queue,
        dir: PathBuf,
        expr_dir: Option<PathBuf>,
        store: Arc<Mutex<Store<FaceRecord>>>,
        tx: mpsc::Sender<Event>,
        cancel: Arc<AtomicBool>,
        progress: Arc<Mutex<Progress>>,
        generation: u64,
    ) {
        let auto = lock(&progress).auto;
        let load = || -> std::result::Result<FaceModels, String> {
            let expr = match &expr_dir {
                Some(d) => match lightcraft_vision::expression::ExpressionNet::load(&d.join(models::EXPRESSION)) {
                    Ok(e) => Some(e),
                    Err(e) => {
                        // the faces still work without it
                        log::warn!("the expression model couldn't be loaded: {e}");
                        None
                    }
                },
                None => None,
            };
            Ok(FaceModels {
                det: YuNet::load(&dir.join(models::YUNET)).map_err(|e| e.to_string())?,
                rec: SFace::load(&dir.join(models::SFACE)).map_err(|e| e.to_string())?,
                eyes: EyeState::load(&dir.join(models::EYES)).map_err(|e| e.to_string())?,
                expr,
            })
        };
        let m = match std::panic::catch_unwind(load) {
            Ok(Ok(m)) => Arc::new(m),
            Ok(Err(e)) => return fail(&progress, e),
            Err(_) => return fail(&progress, "the face models couldn't be loaded"),
        };
        let handles: Vec<_> = (0..threads(auto))
            .filter_map(|_| {
                let (m, queue, store, tx, cancel, progress) = (m.clone(), queue.clone(), store.clone(), tx.clone(), cancel.clone(), progress.clone());
                std::thread::Builder::new()
                    .name("vision-faces".into())
                    .spawn(move || {
                        loop {
                            if cancel.load(Ordering::Relaxed) {
                                break;
                            }
                            let Some(item) = lock(&queue).pop_front() else { break };
                            let r = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| analyse_photo(&m, &item)));
                            match r {
                                Ok(Ok(record)) => {
                                    let n = record.faces.len();
                                    if let Err(e) = lock(&store).put(&item.key, record.clone()) {
                                        log::warn!("faces index: {e}");
                                    }
                                    let _ = tx.send(Event::Faces { id: item.id, generation, record });
                                    tick(&progress, true, n);
                                }
                                Ok(Err(e)) => {
                                    log::info!("faces: photo {}: {e}", item.id.0);
                                    let _ = tx.send(Event::Failed { key: item.key.clone() });
                                    tick(&progress, false, 0);
                                }
                                Err(_) => {
                                    log::warn!("faces: photo {}: the analysis panicked", item.id.0);
                                    let _ = tx.send(Event::Failed { key: item.key.clone() });
                                    tick(&progress, false, 0);
                                }
                            }
                            pace(auto, &cancel);
                        }
                    })
                    .ok()
            })
            .collect();
        for h in handles {
            let _ = h.join();
        }
        let _ = lock(&store).sync();
        let mut p = lock(&progress);
        p.running = false;
        p.cancelled = cancel.load(Ordering::Relaxed);
    }

    /// Landmarks guessed from a box (the template's proportions), for a region the finder didn't find.
    fn landmarks_from_box(r: [f32; 4]) -> [[f32; 2]; 5] {
        // ArcFace template positions over its 112 × 112 square, which spans about the face box
        const T: [[f32; 2]; 5] = [[0.3419, 0.4616], [0.6565, 0.4598], [0.5002, 0.6405], [0.3710, 0.8247], [0.6315, 0.8232]];
        T.map(|p| [r[0] + p[0] * r[2], r[1] + p[1] * r[3]])
    }

    fn analyse_photo(m: &FaceModels, item: &Item) -> std::result::Result<FaceRecord, String> {
        let t0 = web_time::Instant::now();
        let img = crate::memory::in_background(|| item.pixels.load(FACE_EDGE))?;
        let t_load = t0.elapsed();
        let (w, h) = (img.width as f32, img.height as f32);
        if w < 16.0 || h < 16.0 {
            return Ok(FaceRecord { faces: Vec::new(), width: img.width as u32, height: img.height as u32, expressions: m.expr.is_some() });
        }
        let mut found: Vec<Face> = m.det.detect(&img).map_err(|e| e.to_string())?;
        // the photo's own regions the finder missed: look again around each, else take its box
        for r in &item.regions {
            let px = [r[0] * w, r[1] * h, r[2] * w, r[3] * h];
            let rect = to_rect(*r);
            if found.iter().any(|f| iou(&to_rect([f.rect[0] / w, f.rect[1] / h, f.rect[2] / w, f.rect[3] / h]), &rect) >= 0.3) {
                continue;
            }
            let (cx, cy, side) = (px[0] + px[2] / 2.0, px[1] + px[3] / 2.0, px[2].max(px[3]) * 2.0);
            let (x0, y0) = ((cx - side / 2.0).max(0.0), (cy - side / 2.0).max(0.0));
            let again = img.crop(x0 as usize, y0 as usize, side as usize, side as usize).and_then(|c| {
                let (c, k) = if c.width < 160 { (c.resize_area(c.width * 2, c.height * 2), 0.5) } else { (c, 1.0) };
                let mut faces = m.det.detect(&c).ok()?;
                faces.retain(|f| f.score >= 0.5);
                faces
                    .into_iter()
                    .map(|f| {
                        let mut f = f.scaled(k);
                        f.rect[0] += x0;
                        f.rect[1] += y0;
                        for l in &mut f.landmarks {
                            l[0] += x0;
                            l[1] += y0;
                        }
                        f
                    })
                    // only a face on the region itself
                    .find(|f| iou(&to_rect([f.rect[0] / w, f.rect[1] / h, f.rect[2] / w, f.rect[3] / h]), &rect) >= 0.2)
            });
            found.push(match again {
                Some(f) => f,
                None => Face { rect: px, landmarks: landmarks_from_box(px), score: 0.0 },
            });
        }
        let t_detect = t0.elapsed() - t_load;
        let mut faces = Vec::with_capacity(found.len());
        for f in &found {
            let embedding = m.rec.embed(&img, f).map_err(|e| e.to_string())?;
            let eyes = m.eyes.eyes(&img, f).map_err(|e| e.to_string())?.map(|e| e.closed);
            let expression = match &m.expr {
                Some(x) => x.classify(&img, f).map_err(|e| e.to_string())?.map(|e| e.probs),
                None => None,
            };
            faces.push(FaceEntry {
                rect: [f.rect[0] / w, f.rect[1] / h, f.rect[2] / w, f.rect[3] / h],
                landmarks: f.landmarks.map(|p| [p[0] / w, p[1] / h]),
                score: f.score,
                embedding,
                eyes,
                sharpness: face_sharpness(&img, f),
                expression,
            });
        }
        if std::env::var_os("LIGHTCRAFT_PROFILE").is_some() {
            eprintln!(
                "faces: photo {} {}×{}: load {:.0} ms, detect {:.0} ms, {} faces {:.0} ms",
                item.id.0,
                img.width,
                img.height,
                t_load.as_secs_f64() * 1e3,
                t_detect.as_secs_f64() * 1e3,
                faces.len(),
                (t0.elapsed() - t_load - t_detect).as_secs_f64() * 1e3
            );
        }
        Ok(FaceRecord { faces, width: img.width as u32, height: img.height as u32, expressions: m.expr.is_some() })
    }

    /// The culling focus measure on the face alone (0..100, the scale of `cull::sharpness`).
    fn face_sharpness(img: &Rgb8, f: &Face) -> f32 {
        let [x, y, fw, fh] = f.rect;
        let Some(c) = img.crop(x.max(0.0) as usize, y.max(0.0) as usize, fw.max(1.0) as usize, fh.max(1.0) as usize) else { return 0.0 };
        let data: Vec<[f32; 3]> =
            c.data.as_chunks::<3>().0.iter().map(|p| p.map(|v| lightcraft_color::transfer::srgb_to_linear(f32::from(v) / 255.0))).collect();
        let rgb = lightcraft_raster::Rgb32f { width: c.width, height: c.height, data };
        lightcraft_pipeline::cull::sharpness(&rgb)
    }

    /// The search indexer: CLIP embeddings in batches on the GPU (when there is one).
    pub fn search(
        queue: Queue,
        dir: PathBuf,
        store: Arc<Mutex<Store<Embedding>>>,
        tx: mpsc::Sender<Event>,
        cancel: Arc<AtomicBool>,
        progress: Arc<Mutex<Progress>>,
    ) {
        let device = crate::vision::device();
        let clip = match std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| lightcraft_vision::clip::Clip::load(&dir, &device))) {
            Ok(Ok(c)) => Arc::new(c),
            Ok(Err(e)) => return fail(&progress, e.to_string()),
            Err(_) => return fail(&progress, "the search model couldn't be loaded"),
        };
        const BATCH: usize = 16;
        // decoders feed a bounded channel; this thread runs the model
        let (in_tx, in_rx) = mpsc::sync_channel::<(Item, std::result::Result<Vec<f32>, String>)>(BATCH * 2);
        let decoders: Vec<_> = (0..threads(false))
            .filter_map(|_| {
                let (queue, in_tx, cancel, clip) = (queue.clone(), in_tx.clone(), cancel.clone(), clip.clone());
                std::thread::Builder::new()
                    .name("vision-decode".into())
                    .spawn(move || {
                        loop {
                            if cancel.load(Ordering::Relaxed) {
                                break;
                            }
                            let Some(item) = lock(&queue).pop_front() else { break };
                            let input = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
                                crate::memory::in_background(|| item.pixels.load(SEARCH_EDGE)).map(|img| clip.preprocess(&img))
                            }))
                            .unwrap_or_else(|_| Err("decoding panicked".into()));
                            if in_tx.send((item, input)).is_err() {
                                break;
                            }
                        }
                    })
                    .ok()
            })
            .collect();
        drop(in_tx);
        let mut batch: Vec<(Item, Vec<f32>)> = Vec::with_capacity(BATCH);
        let flush = |batch: &mut Vec<(Item, Vec<f32>)>| {
            if batch.is_empty() {
                return;
            }
            let inputs: Vec<Vec<f32>> = batch.iter().map(|(_, v)| v.clone()).collect();
            let r = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
                let _gpu = crate::vision::gpu_lock(clip.device());
                clip.embed_images(&inputs)
            }));
            match r {
                Ok(Ok(embs)) => {
                    let mut st = lock(&store);
                    for ((item, _), e) in batch.iter().zip(embs) {
                        if let Err(err) = st.put(&item.key, Embedding(e)) {
                            log::warn!("search index: {err}");
                        }
                        let _ = tx.send(Event::Embedded { id: item.id });
                    }
                    drop(st);
                    for _ in batch.iter() {
                        tick(&progress, true, 0);
                    }
                }
                Ok(Err(e)) => {
                    log::warn!("search: {e}");
                    for _ in batch.iter() {
                        tick(&progress, false, 0);
                    }
                }
                Err(_) => {
                    for _ in batch.iter() {
                        tick(&progress, false, 0);
                    }
                }
            }
            batch.clear();
        };
        for (item, input) in in_rx {
            if cancel.load(Ordering::Relaxed) {
                break;
            }
            match input {
                Ok(v) => batch.push((item, v)),
                Err(e) => {
                    log::info!("search: photo {}: {e}", item.id.0);
                    let _ = tx.send(Event::Failed { key: item.key.clone() });
                    tick(&progress, false, 0);
                }
            }
            if batch.len() >= BATCH {
                flush(&mut batch);
            }
        }
        if !cancel.load(Ordering::Relaxed) {
            flush(&mut batch);
        }
        for d in decoders {
            let _ = d.join();
        }
        let _ = lock(&store).sync();
        let mut p = lock(&progress);
        p.running = false;
        p.cancelled = cancel.load(Ordering::Relaxed);
    }
}

/// The device the transformer models run on: Enhance's shared one (Metal on macOS) when the app has the models
/// feature, else the CPU.
#[cfg(not(target_arch = "wasm32"))]
pub fn device() -> candle_core::Device {
    #[cfg(feature = "enhance")]
    {
        lightcraft_enhance::best_device()
    }
    #[cfg(not(feature = "enhance"))]
    {
        candle_core::Device::Cpu
    }
}

/// Take turns on the GPU with AI masks and Enhance (candle 0.9.2's Metal device misbehaves when threads share it).
#[cfg(not(target_arch = "wasm32"))]
pub fn gpu_lock(device: &candle_core::Device) -> Option<std::sync::MutexGuard<'static, ()>> {
    #[cfg(feature = "enhance")]
    {
        lightcraft_enhance::gpu_lock(device)
    }
    #[cfg(not(feature = "enhance"))]
    {
        let _ = device;
        None
    }
}
