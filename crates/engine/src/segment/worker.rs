//! The SAM 3 worker thread: it owns the model and the last encoded photo, and runs every
//! request (load, render the model input, encode, clicks, descriptions, detail passes) in
//! turn. The session only sends jobs and receives outcomes over channels, so no thread ever
//! waits on a lock the model holds, and the UI thread never runs the model.
//!
//! Every job runs under `catch_unwind`: a panic inside candle becomes an error outcome, drops
//! the (possibly inconsistent) model and leaves the worker ready for the next job. The busy
//! counters are decremented by drop guards, so they can't stay stuck. The model is unloaded
//! after [`IDLE_UNLOAD`] without requests (it holds several GB).

use std::collections::VecDeque;
use std::panic::AssertUnwindSafe;
use std::path::PathBuf;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::mpsc::{self, RecvTimeoutError};
use std::time::Duration;

use lightcraft_develop::SegMask;
use lightcraft_segment::parsers::{self, ParserSpec};
use lightcraft_segment::segformer::{Parsed, SegFormer};
use lightcraft_segment::{Encoded, Sam3};

use super::auto::{self, Auto, AutoOut};
use super::{Input, Prompt, Tag};
use crate::media::RenderJob;

/// Unload the model after this long without a request.
pub const IDLE_UNLOAD: Duration = Duration::from_secs(10 * 60);

/// What to compute.
pub enum Kind {
    /// Load the model and encode the photo (so the first click is fast).
    Prepare,
    Clicks(Vec<lightcraft_segment::Click>),
    Text(String),
    /// A zoomed-in pass over `region` (normalized) of the photo, rendered by the job's `render`.
    Detail {
        prompt: Prompt,
        region: [f64; 4],
    },
    /// Lightroom's automatic masks (subject, sky, landscape, people), from whichever models are
    /// installed.
    Auto(Auto),
    /// Panics (tests of the worker's recovery).
    #[cfg(test)]
    Panic,
}

/// One request.
pub struct Job {
    /// The SAM 3 model folder (see `sam3`).
    pub dir: PathBuf,
    /// Which photo and look the request is about (the encode cache's key).
    pub key: u64,
    /// Renders the model input when the cache doesn't hold `key` (always for a detail pass).
    pub render: Option<RenderJob>,
    /// The photo's file, when it is missing on disk (for a clearer error if rendering fails).
    pub missing: Option<String>,
    pub kind: Kind,
    /// Whether SAM 3 is installed in `dir` (automatic masks also run without it).
    pub sam3: bool,
    /// The folder the parsers are installed in (`<models>/<parser id>/`), if any.
    pub models: Option<PathBuf>,
    /// A second render: the part of the photo a People parts job zooms into.
    pub crop: Option<RenderJob>,
    pub tag: Tag,
    pub reply: mpsc::Sender<Outcome>,
}

/// A finished request: the segmentation (`None`: nothing selected / nothing to report).
pub struct Outcome {
    pub tag: Tag,
    pub result: Result<Option<SegMask>, String>,
    /// What an automatic mask job found (with `result` `Ok(None)`).
    pub auto: Option<AutoOut>,
    /// A newer request for the same selection came in; this one was skipped.
    pub superseded: bool,
}

/// Counters the session reads without locking.
#[derive(Default)]
pub struct Shared {
    /// Requests other than detail passes, queued or running.
    pub pending: AtomicUsize,
    /// Detail passes queued or running.
    pub detail: AtomicUsize,
    /// The model is in memory.
    pub loaded: AtomicBool,
    /// Loading the model or encoding a photo right now (the slow parts).
    pub encoding: AtomicBool,
}

/// Decrements a counter when dropped (also when unwinding).
struct Done<'a>(&'a AtomicUsize);

impl Drop for Done<'_> {
    fn drop(&mut self) {
        let _ = self.0.try_update(Ordering::SeqCst, Ordering::SeqCst, |n| Some(n.saturating_sub(1)));
    }
}

/// Sets a flag while alive.
struct Flag<'a>(&'a AtomicBool);

impl<'a> Flag<'a> {
    fn raise(f: &'a AtomicBool) -> Self {
        f.store(true, Ordering::SeqCst);
        Flag(f)
    }
}

impl Drop for Flag<'_> {
    fn drop(&mut self) {
        self.0.store(false, Ordering::SeqCst);
    }
}

/// The session's handle on the worker thread (started on the first request, restarted if it
/// ever ends).
#[derive(Default)]
pub struct Worker {
    tx: Option<mpsc::Sender<Job>>,
    thread: Option<std::thread::JoinHandle<()>>,
    pub shared: Arc<Shared>,
}

impl Worker {
    fn counter(&self, job: &Job) -> &AtomicUsize {
        if matches!(job.kind, Kind::Detail { .. }) { &self.shared.detail } else { &self.shared.pending }
    }

    /// Whether the thread has ended (it never should); its counters are then meaningless.
    fn dead(&self) -> bool {
        self.thread.as_ref().is_some_and(|t| t.is_finished())
    }

    pub fn pending(&self) -> usize {
        if self.dead() { 0 } else { self.shared.pending.load(Ordering::SeqCst) }
    }

    pub fn detail(&self) -> usize {
        if self.dead() { 0 } else { self.shared.detail.load(Ordering::SeqCst) }
    }

    pub fn encoding(&self) -> bool {
        !self.dead() && self.shared.encoding.load(Ordering::SeqCst)
    }

    pub fn loaded(&self) -> bool {
        !self.dead() && self.shared.loaded.load(Ordering::SeqCst)
    }

    /// Queue `job`; returns at once.
    pub fn submit(&mut self, job: Job) -> Result<(), String> {
        if self.dead() {
            // a thread that ended took its counters' meaning with it
            self.tx = None;
            self.thread = None;
            self.shared = Arc::new(Shared::default());
        }
        self.counter(&job).fetch_add(1, Ordering::SeqCst);
        let job = match &self.tx {
            Some(tx) => match tx.send(job) {
                Ok(()) => return Ok(()),
                Err(mpsc::SendError(job)) => job,
            },
            None => job,
        };
        // (re)start the thread
        let (tx, rx) = mpsc::channel();
        let shared = self.shared.clone();
        match std::thread::Builder::new().name("sam3".into()).spawn(move || run(&rx, &shared, IDLE_UNLOAD)) {
            Ok(t) => {
                self.thread = Some(t);
                self.tx = Some(tx.clone());
                tx.send(job).map_err(|_| "the AI mask worker stopped".to_string())
            }
            Err(e) => {
                let _ = self.counter(&job).try_update(Ordering::SeqCst, Ordering::SeqCst, |n| Some(n.saturating_sub(1)));
                Err(format!("could not start the AI mask worker: {e}"))
            }
        }
    }
}

/// The model and the last encoded photo.
#[derive(Default)]
struct State {
    model: Option<(PathBuf, Sam3)>,
    cache: Option<(u64, Encoded)>,
    /// The last photo rendered for the models.
    input: Option<(u64, Input)>,
    /// Parsers loaded so far (by id).
    parsers: Vec<(&'static str, SegFormer)>,
    /// The scene parse of photo `input` holds.
    scene: Option<(u64, Parsed)>,
}

/// Whether `later` makes `job` pointless (it asks for the same selection again).
fn supersedes(later: &Job, job: &Job) -> bool {
    match (&later.tag, &job.tag) {
        (Tag::Prepare, Tag::Prepare) => later.key == job.key,
        (Tag::Clicks { photo: a, mask: am, comp: ac, .. }, Tag::Clicks { photo: b, mask: bm, comp: bc, .. }) => (a, am, ac) == (b, bm, bc),
        (Tag::Detail { photo: a, mask: am, comp: ac, .. }, Tag::Detail { photo: b, mask: bm, comp: bc, .. }) => (a, am, ac) == (b, bm, bc),
        _ => false,
    }
}

fn run(rx: &mpsc::Receiver<Job>, shared: &Shared, idle: Duration) {
    let mut state = State::default();
    let mut queue: VecDeque<Job> = VecDeque::new();
    loop {
        if queue.is_empty() {
            match rx.recv_timeout(idle) {
                Ok(j) => queue.push_back(j),
                Err(RecvTimeoutError::Timeout) => {
                    if state.model.is_some() || !state.parsers.is_empty() {
                        state = State::default();
                        shared.loaded.store(false, Ordering::SeqCst);
                        log::info!("SAM 3 unloaded after {} minutes without use", idle.as_secs() / 60);
                    }
                    continue;
                }
                Err(RecvTimeoutError::Disconnected) => return,
            }
        }
        while let Ok(j) = rx.try_recv() {
            queue.push_back(j);
        }
        let Some(mut job) = queue.pop_front() else { continue };
        let is_detail = matches!(job.kind, Kind::Detail { .. });
        // counts the job as pending until just before its reply goes out: a caller woken by the
        // reply must not still see the worker busy with it
        let done = Done(if is_detail { &shared.detail } else { &shared.pending });
        if queue.iter().any(|later| supersedes(later, &job)) {
            drop(done);
            let _ = job.reply.send(Outcome { tag: job.tag, result: Ok(None), auto: None, superseded: true });
            continue;
        }
        // one model on the GPU at a time: candle's Metal device gives wrong results when Enhance and the masks
        // queue work on it from two threads at once (measured in crates/enhance)
        #[cfg(feature = "enhance")]
        let gpu = lightcraft_enhance::gpu_lock(&lightcraft_enhance::best_device());
        let result = match std::panic::catch_unwind(AssertUnwindSafe(|| run_job(&mut state, shared, &mut job))) {
            Ok(r) => r,
            Err(p) => {
                let why = p.downcast_ref::<&str>().map(|s| s.to_string()).or_else(|| p.downcast_ref::<String>().cloned()).unwrap_or_default();
                log::error!("SAM 3 panicked: {why}");
                // don't trust what the model kept: load it again next time
                state = State::default();
                Err("the AI model failed unexpectedly (it will be reloaded on the next try)".to_string())
            }
        };
        #[cfg(feature = "enhance")]
        drop(gpu);
        if result.is_err() && !is_detail {
            state.cache = None;
            state.input = None;
            state.scene = None;
        }
        shared.loaded.store(state.model.is_some() || !state.parsers.is_empty(), Ordering::SeqCst);
        drop(done);
        let (result, auto) = match result {
            Ok(Out::Seg(s)) => (Ok(s), None),
            Ok(Out::Auto(a)) => (Ok(None), Some(a)),
            Err(e) => (Err(e), None),
        };
        let _ = job.reply.send(Outcome { tag: job.tag, result, auto, superseded: false });
    }
}

/// The model for `dir`, loading it when needed.
fn model<'a>(state: &'a mut State, shared: &Shared, dir: &std::path::Path) -> Result<&'a mut Sam3, String> {
    if state.model.as_ref().is_none_or(|(d, _)| d != dir) {
        state.model = None;
        state.cache = None;
        let _busy = Flag::raise(&shared.encoding);
        let t = web_time::Instant::now();
        let m = Sam3::load(dir).map_err(|e| e.to_string())?;
        log::info!("SAM 3 loaded in {:?}", t.elapsed());
        state.model = Some((dir.to_path_buf(), m));
        shared.loaded.store(true, Ordering::SeqCst);
    }
    state.model.as_mut().map(|(_, m)| m).ok_or_else(|| "the model did not load".to_string())
}

/// Render the model input.
fn render(job: &mut Job) -> Result<Input, String> {
    let r = job.render.take().ok_or("the photo is not available")?.run();
    let img = match r.rendered {
        Ok(r) => r.image,
        Err(e) => {
            return Err(match &job.missing {
                Some(path) => format!(
                    "This photo's file is missing (moved or deleted), so it can't be analyzed for AI masks: {path}. Library ▸ Find Missing Photos can relink it."
                ),
                None => format!("couldn't render the photo for AI masks: {e}"),
            });
        }
    };
    let rgb = img.data.iter().flat_map(|p| [p[0], p[1], p[2]]).collect();
    Ok(Input { rgb, w: img.width, h: img.height })
}

/// The rendered photo `job.key` (rendering it when the cache holds another).
fn input<'a>(state: &'a mut State, job: &mut Job) -> Result<&'a Input, String> {
    if state.input.as_ref().is_none_or(|(k, _)| *k != job.key) {
        state.input = None;
        let i = render(job)?;
        state.input = Some((job.key, i));
    }
    state.input.as_ref().map(|(_, i)| i).ok_or_else(|| "the photo is not available".to_string())
}

/// The model and the encoded photo `job.key` (rendering and encoding it when needed).
fn encoded<'a>(state: &'a mut State, shared: &Shared, job: &mut Job) -> Result<(&'a mut Sam3, &'a mut Encoded), String> {
    model(state, shared, &job.dir)?;
    if state.cache.as_ref().is_none_or(|(k, _)| *k != job.key) {
        state.cache = None;
        input(state, job)?;
        let _busy = Flag::raise(&shared.encoding);
        let (Some((_, m)), Some((_, input))) = (state.model.as_mut(), state.input.as_ref()) else {
            return Err("the model did not load".into());
        };
        let t = web_time::Instant::now();
        let mut enc = m.encode(&input.rgb, input.w, input.h).map_err(|e| e.to_string())?;
        m.prepare_clicks(&mut enc).map_err(|e| e.to_string())?;
        log::info!("SAM 3 encoded {}×{} in {:?}", input.w, input.h, t.elapsed());
        state.cache = Some((job.key, enc));
    }
    match (state.model.as_mut(), state.cache.as_mut()) {
        (Some((_, m)), Some((_, e))) => Ok((m, e)),
        _ => Err("the model is not ready".into()),
    }
}

/// Parser `spec` from `models`, loading it when needed; `None` when it isn't installed.
fn parser<'a>(state: &'a mut State, shared: &Shared, models: Option<&std::path::Path>, spec: &ParserSpec) -> Result<Option<&'a SegFormer>, String> {
    let Some(dir) = models.map(|m| m.join(spec.id)).filter(|d| lightcraft_segment::parsers::is_parser_dir(d)) else { return Ok(None) };
    if !state.parsers.iter().any(|(id, _)| *id == spec.id) {
        let _busy = Flag::raise(&shared.encoding);
        let t = web_time::Instant::now();
        let m = SegFormer::load(&dir, lightcraft_segment::best_device()).map_err(|e| format!("{}: {e}", spec.name))?;
        log::info!("{} loaded in {:?}", spec.name, t.elapsed());
        state.parsers.push((spec.id, m));
        shared.loaded.store(true, Ordering::SeqCst);
    }
    Ok(state.parsers.iter().find(|(id, _)| *id == spec.id).map(|(_, m)| m))
}

/// The scene parse of photo `job.key` (computing it when needed); `None` without the scene model.
fn scene(state: &mut State, shared: &Shared, job: &mut Job) -> Result<bool, String> {
    if state.scene.as_ref().is_some_and(|(k, _)| *k == job.key) {
        return Ok(true);
    }
    let models = job.models.clone();
    if parser(state, shared, models.as_deref(), &parsers::SCENE)?.is_none() {
        return Ok(false);
    }
    input(state, job)?;
    let _busy = Flag::raise(&shared.encoding);
    let (Some((_, i)), Some((_, m))) = (state.input.as_ref(), state.parsers.iter().find(|(id, _)| *id == parsers::SCENE.id)) else {
        return Ok(false);
    };
    let t = web_time::Instant::now();
    let p = m.parse(&i.rgb, i.w, i.h, parsers::SCENE.input).map_err(|e| e.to_string())?;
    log::info!("scene parsed {}×{} in {:?}", i.w, i.h, t.elapsed());
    state.scene = Some((job.key, p));
    Ok(true)
}

/// The error when no model for `auto` is installed (starts with [`super::NO_AUTO_MODEL`]: the UI
/// offers the download on it).
fn no_model(auto: &Auto) -> String {
    let what = match auto {
        Auto::Sky | Auto::Landscape(_) | Auto::Coverage => "the Scene model",
        Auto::Subject | Auto::People => "SAM 3 or the Scene model",
        Auto::Parts { .. } => "the People and Faces models",
    };
    format!(
        "{} ({what}). Download them when LightCraft offers it, or with `segment.parsers.download {{\"acknowledged\": true}}`.",
        super::NO_AUTO_MODEL
    )
}

fn run_auto(state: &mut State, shared: &Shared, job: &mut Job, auto: Auto) -> Result<AutoOut, String> {
    // the scene parse, only where it is used (SAM 3 answers Subject and People without it)
    let needs_scene = match auto {
        Auto::Parts { .. } => false,
        Auto::Subject | Auto::People => !job.sam3,
        _ => true,
    };
    let have_scene = needs_scene && scene(state, shared, job)?;
    let scene_labels: Option<Vec<String>> = state.parsers.iter().find(|(id, _)| *id == parsers::SCENE.id).map(|(_, m)| m.labels().to_vec());
    match auto {
        Auto::Sky | Auto::Landscape(_) | Auto::Coverage => {
            let class = match &auto {
                Auto::Landscape(c) => c.clone(),
                _ => "sky".to_string(),
            };
            if let (true, Some((_, p)), Some(labels)) = (have_scene, state.scene.as_ref(), scene_labels.as_deref()) {
                if matches!(auto, Auto::Coverage) {
                    return Ok(AutoOut::Coverage(auto::coverage(p, labels)));
                }
                return auto::landscape(p, labels, &class)?.map(AutoOut::Seg).ok_or_else(|| nothing(&class));
            }
            // without the scene model, SAM 3 describes it
            if job.sam3 && !matches!(auto, Auto::Coverage) {
                let prompt: Vec<&str> = class
                    .split(',')
                    .map(|c| match c.trim() {
                        "naturalGround" => "ground, sand, rocks",
                        "artificialGround" => "road, pavement, floor",
                        "vegetation" => "trees, grass, plants",
                        "architecture" => "buildings",
                        "mountains" => "mountains",
                        c => c,
                    })
                    .collect();
                let (m, enc) = encoded(state, shared, job)?;
                return text_logits(m, enc, &prompt.join(", "))?
                    .map(|l| AutoOut::Seg(SegMask::from_logits(lightcraft_segment::MASK_SIDE, &l)))
                    .ok_or_else(|| nothing(&class));
            }
            Err(no_model(&auto))
        }
        Auto::Subject => {
            if job.sam3 {
                let (m, enc) = encoded(state, shared, job)?;
                let mut models = auto::Models { sam: Some((m, enc)), scene: None };
                if let Some(s) = auto::subject(&mut models)? {
                    return Ok(AutoOut::Seg(s));
                }
            }
            if let (true, Some((_, p)), Some(labels)) = (have_scene, state.scene.as_ref(), scene_labels.as_deref()) {
                return auto::scene_subject(p, labels).map(AutoOut::Seg).ok_or_else(|| "No subject was found in this photo.".to_string());
            }
            if job.sam3 { Err("No subject was found in this photo.".into()) } else { Err(no_model(&auto)) }
        }
        Auto::People => {
            if job.sam3 {
                let (m, enc) = encoded(state, shared, job)?;
                return Ok(AutoOut::People(auto::people(&mut auto::Models { sam: Some((m, enc)), scene: None }, None, &[])?));
            }
            if !have_scene {
                return Err(no_model(&auto));
            }
            // the faces (body parser) split people standing together
            let models = job.models.clone();
            let mut faces: Vec<(f64, f64)> = Vec::new();
            if parser(state, shared, models.as_deref(), &parsers::BODY)?.is_some()
                && let (Some((_, i)), Some((_, body))) = (state.input.as_ref(), state.parsers.iter().find(|(id, _)| *id == parsers::BODY.id))
            {
                let _busy = Flag::raise(&shared.encoding);
                let pb = body.parse(&i.rgb, i.w, i.h, parsers::BODY.input).map_err(|e| e.to_string())?;
                let fp = pb.prob(&parsers::indices(body.labels(), &["Face"]));
                for c in parsers::components(&fp, pb.vw, pb.vh, 0.5) {
                    if c.len() >= 4
                        && let Some(b) = parsers::bbox(&c, pb.vw)
                    {
                        faces.push(((b[0] + b[2]) as f64 / 2.0 / pb.vw as f64, (b[1] + b[3]) as f64 / 2.0 / pb.vh as f64));
                    }
                }
            }
            match state.scene.as_ref() {
                Some((_, p)) => Ok(AutoOut::People(auto::people(&mut auto::Models { sam: None, scene: Some(p) }, scene_labels.as_deref(), &faces)?)),
                None => Err(no_model(&auto)),
            }
        }
        Auto::Parts { rect, region, parts } => {
            // the person: found again on the whole photo, the one that overlaps `rect` most
            let found = run_auto(state, shared, job, Auto::People)?;
            let AutoOut::People(people) = found else { return Err("no people".into()) };
            let person = people
                .into_iter()
                .map(|p| (auto::iou(p.rect, rect), p))
                .filter(|(o, _)| *o > 0.3)
                .max_by(|a, b| a.0.total_cmp(&b.0))
                .map(|(_, p)| p)
                .ok_or("That person is no longer found in this photo.")?;
            let models = job.models.clone();
            if parser(state, shared, models.as_deref(), &parsers::BODY)?.is_none() {
                return Err(no_model(&Auto::Parts { rect, region, parts }));
            }
            parser(state, shared, models.as_deref(), &parsers::FACE)?;
            let crop_job = job.crop.take().ok_or("the photo is not available")?;
            let r = crop_job.run();
            let img = r.rendered.map_err(|e| format!("couldn't render the photo for AI masks: {e}"))?.image;
            let crop = Input { rgb: img.data.iter().flat_map(|p| [p[0], p[1], p[2]]).collect(), w: img.width, h: img.height };
            let body = state.parsers.iter().find(|(id, _)| *id == parsers::BODY.id).map(|(_, m)| m).ok_or("the People model did not load")?;
            let face = state.parsers.iter().find(|(id, _)| *id == parsers::FACE.id).map(|(_, m)| m);
            let _busy = Flag::raise(&shared.encoding);
            let t = web_time::Instant::now();
            let (seg, detail) = auto::parts(body, face, &crop, region, &person.seg, &parts)?;
            log::info!("people parts ({}×{}) in {:?}", crop.w, crop.h, t.elapsed());
            Ok(AutoOut::Parts { seg, detail })
        }
    }
}

fn nothing(class: &str) -> String {
    let labels: Vec<String> = class.split(',').map(|c| lightcraft_develop::ai::landscape_label(c.trim()).unwrap_or(c).to_lowercase()).collect();
    format!("No {} was found in this photo.", labels.join(" or "))
}

/// Every phrase of `text` ("car, road"), merged (the per-pixel maximum); `None` when nothing
/// matches.
fn text_logits(model: &mut Sam3, enc: &mut Encoded, text: &str) -> Result<Option<Vec<f32>>, String> {
    let mut merged: Option<Vec<f32>> = None;
    for phrase in text.split([',', ';']).map(str::trim).filter(|p| !p.is_empty()) {
        if let Some(p) = model.segment_text(enc, phrase, 0.5).map_err(|e| e.to_string())? {
            merged = Some(match merged {
                None => p.logits,
                Some(m) => m.iter().zip(&p.logits).map(|(a, b)| a.max(*b)).collect(),
            });
        }
    }
    Ok(merged)
}

/// What a job produced.
enum Out {
    Seg(Option<SegMask>),
    Auto(AutoOut),
}

fn run_job(state: &mut State, shared: &Shared, job: &mut Job) -> Result<Out, String> {
    match std::mem::replace(&mut job.kind, Kind::Prepare) {
        Kind::Auto(a) => run_auto(state, shared, job, a).map(Out::Auto),
        other => {
            // (put it back: run_sam takes it)
            job.kind = other;
            run_sam(state, shared, job).map(Out::Seg)
        }
    }
}

fn run_sam(state: &mut State, shared: &Shared, job: &mut Job) -> Result<Option<SegMask>, String> {
    let side = lightcraft_segment::MASK_SIDE;
    match std::mem::replace(&mut job.kind, Kind::Prepare) {
        Kind::Prepare => {
            encoded(state, shared, job)?;
            Ok(None)
        }
        Kind::Auto(_) => Ok(None),
        Kind::Clicks(clicks) => {
            let (m, enc) = encoded(state, shared, job)?;
            let p = m.segment_clicks(enc, &clicks).map_err(|e| e.to_string())?;
            Ok(Some(SegMask::from_logits(side, &p.logits)))
        }
        Kind::Text(text) => {
            let (m, enc) = encoded(state, shared, job)?;
            Ok(text_logits(m, enc, &text)?.map(|l| SegMask::from_logits(side, &l)))
        }
        #[cfg(test)]
        #[allow(clippy::panic)]
        Kind::Panic => panic!("a test panic"),
        Kind::Detail { prompt, region } => {
            let input = render(job)?;
            let m = model(state, shared, &job.dir)?;
            let t = web_time::Instant::now();
            let mut enc = m.encode(&input.rgb, input.w, input.h).map_err(|e| e.to_string())?;
            let (rw, rh) = (region[2] - region[0], region[3] - region[1]);
            if !(rw > 0.0 && rh > 0.0) {
                return Ok(None);
            }
            let logits = match prompt {
                Prompt::Clicks(clicks) => {
                    let inside: Vec<lightcraft_segment::Click> = clicks
                        .iter()
                        .map(|c| ((c.at.x - region[0]) / rw, (c.at.y - region[1]) / rh, c.include))
                        .filter(|(x, y, _)| (0.0..=1.0).contains(x) && (0.0..=1.0).contains(y))
                        .map(|(x, y, positive)| lightcraft_segment::Click { x: x as f32, y: y as f32, positive })
                        .collect();
                    if !inside.iter().any(|c| c.positive) {
                        return Ok(None);
                    }
                    Some(m.segment_clicks(&mut enc, &inside).map_err(|e| e.to_string())?.logits)
                }
                Prompt::Text(text) => text_logits(m, &mut enc, &text)?,
            };
            log::info!("SAM 3 detail pass ({}×{}) in {:?}", input.w, input.h, t.elapsed());
            Ok(logits.map(|l| SegMask::from_logits_in(side, &l, region)))
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn job(tag: Tag, reply: &mpsc::Sender<Outcome>) -> Job {
        // an empty folder: loading fails with an error (no model files)
        let dir = std::env::temp_dir().join(format!("lc-sam3-worker-none-{}", std::process::id()));
        Job { dir, key: 1, render: None, missing: None, kind: Kind::Prepare, sam3: true, models: None, crop: None, tag, reply: reply.clone() }
    }

    #[test]
    fn errors_come_back_and_the_counters_return_to_zero() {
        let mut w = Worker::default();
        let (tx, rx) = mpsc::channel();
        for _ in 0..3 {
            w.submit(job(Tag::Wait, &tx)).unwrap();
        }
        for _ in 0..3 {
            let o = rx.recv_timeout(Duration::from_secs(30)).unwrap();
            assert!(o.result.unwrap_err().contains("not found"));
        }
        let t = web_time::Instant::now();
        while w.pending() > 0 && t.elapsed() < Duration::from_secs(5) {
            std::thread::yield_now();
        }
        assert_eq!(w.pending(), 0);
        assert!(!w.encoding() && !w.loaded());
    }

    #[test]
    fn a_panic_in_a_job_is_an_error_and_the_worker_carries_on() {
        let shared = Shared::default();
        let (tx, rx) = mpsc::channel();
        let (jtx, jrx) = mpsc::channel();
        let mut j = job(Tag::Wait, &tx);
        j.kind = Kind::Panic;
        jtx.send(j).unwrap();
        jtx.send(job(Tag::Wait, &tx)).unwrap();
        shared.pending.store(2, Ordering::SeqCst);
        drop(jtx);
        run(&jrx, &shared, Duration::from_secs(60));
        let first = rx.recv().unwrap();
        assert!(first.result.unwrap_err().contains("failed unexpectedly"));
        let second = rx.recv().unwrap();
        assert!(second.result.is_err(), "the next job still runs");
        assert_eq!(shared.pending.load(Ordering::SeqCst), 0, "counters reset");
        assert!(!shared.encoding.load(Ordering::SeqCst));
    }

    #[test]
    fn a_newer_request_for_the_same_selection_supersedes_a_queued_one() {
        let (tx, rx) = mpsc::channel();
        let (jtx, jrx) = mpsc::channel();
        let clicks = |n: usize| Tag::Clicks {
            photo: lightcraft_catalog::PhotoId(1),
            mask: 1,
            comp: 0,
            hint: vec![lightcraft_geom::Point::new(0.5, 0.5); n],
            exclude: vec![],
        };
        for n in 1..=3 {
            jtx.send(job(clicks(n), &tx)).unwrap();
        }
        drop(jtx);
        let shared = Shared::default();
        shared.pending.store(3, Ordering::SeqCst);
        run(&jrx, &shared, Duration::from_secs(60));
        let out: Vec<Outcome> = rx.try_iter().collect();
        assert_eq!(out.len(), 3);
        assert!(out[0].superseded && out[1].superseded && !out[2].superseded);
        assert_eq!(shared.pending.load(Ordering::SeqCst), 0);
    }
}
