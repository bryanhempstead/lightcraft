//! A prioritized job pool with per-slot de-duplication.
//!
//! Each job targets a *slot* (a grid cell, the loupe…). Submitting for a slot replaces a job for
//! that slot that hasn't started yet, so scrolling or dragging never builds a backlog. Workers
//! take the highest priority first (newest first among equals). Frontends re-prioritize or drop
//! queued jobs when the view changes (e.g. thumbnails scrolled out of view).
//!
//! Native: a fixed set of worker threads, started on first use. wasm32 (no threads): the host
//! calls [`JobPool::run_inline`] once per frame.

use std::hash::Hash;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::{Receiver, Sender, channel};
use std::sync::{Arc, Condvar, Mutex};

pub type Job<R> = Box<dyn FnOnce() -> R + Send>;

struct Entry<S, R> {
    slot: S,
    key: u64,
    priority: u32,
    seq: u64,
    job: Job<R>,
}

/// Called by a worker once a finished job's result can be received.
pub type Waker = Arc<dyn Fn() + Send + Sync>;

/// The group a slot's jobs run one at a time in, if any ([`JobPool::set_exclusive`]).
pub type Exclusive<S> = Arc<dyn Fn(&S) -> Option<u32> + Send + Sync>;

struct Shared<S, R> {
    queue: Mutex<Vec<Entry<S, R>>>,
    cv: Condvar,
    shutdown: AtomicBool,
    on_done: Mutex<Option<Waker>>,
    /// The groups whose jobs never run side by side, and the jobs running now with their group
    /// (both read under `queue`'s lock).
    exclusive: Mutex<Option<Exclusive<S>>>,
    running: Mutex<Vec<(S, Option<u32>)>>,
}

/// A finished job.
pub struct Done<S, R> {
    pub slot: S,
    /// The key the job was submitted with (to drop stale results).
    pub key: u64,
    pub result: R,
    /// Run time in milliseconds (0 on wasm).
    pub ms: f64,
}

pub struct JobPool<S, R> {
    shared: Arc<Shared<S, R>>,
    tx: Sender<Done<S, R>>,
    rx: Receiver<Done<S, R>>,
    #[cfg_attr(target_arch = "wasm32", allow(dead_code))]
    threads: usize,
    started: bool,
    seq: u64,
}

impl<S: Copy + Eq + Hash + Send + 'static, R: Send + 'static> JobPool<S, R> {
    /// A pool with `threads` workers (none on wasm; `0` = jobs only run via [`JobPool::run_inline`]).
    pub fn new(threads: usize) -> Self {
        let (tx, rx) = channel();
        JobPool {
            shared: Arc::new(Shared {
                queue: Mutex::new(Vec::new()),
                cv: Condvar::new(),
                shutdown: AtomicBool::new(false),
                on_done: Mutex::new(None),
                exclusive: Mutex::new(None),
                running: Mutex::new(Vec::new()),
            }),
            tx,
            rx,
            threads,
            started: false,
            seq: 0,
        }
    }

    /// Workers for this machine: cores − 1, between 2 and 8.
    pub fn default_threads() -> usize {
        #[cfg(not(target_arch = "wasm32"))]
        {
            std::thread::available_parallelism().map(|n| n.get().saturating_sub(1)).unwrap_or(4).clamp(2, 8)
        }
        #[cfg(target_arch = "wasm32")]
        {
            1
        }
    }

    fn start(&mut self) {
        if self.started {
            return;
        }
        self.started = true;
        #[cfg(not(target_arch = "wasm32"))]
        for i in 0..self.threads {
            let shared = self.shared.clone();
            let tx = self.tx.clone();
            let _ = std::thread::Builder::new().name(format!("lc-job-{i}")).spawn(move || {
                while let Some(e) = take(&shared, true) {
                    let t0 = std::time::Instant::now();
                    let result = (e.job)();
                    let done = Done { slot: e.slot, key: e.key, result, ms: t0.elapsed().as_secs_f64() * 1000.0 };
                    finished(&shared, e.slot);
                    if tx.send(done).is_err() {
                        break;
                    }
                    let wake = shared.on_done.lock().unwrap_or_else(|e| e.into_inner()).clone();
                    if let Some(w) = wake {
                        w();
                    }
                }
            });
        }
    }

    /// Queue `job` for `slot`, replacing a queued (not yet running) job for the same slot.
    pub fn submit(&mut self, slot: S, key: u64, priority: u32, job: Job<R>) {
        self.start();
        self.seq += 1;
        let (lock, cv) = (&self.shared.queue, &self.shared.cv);
        let mut q = lock.lock().unwrap_or_else(|e| e.into_inner());
        q.retain(|e| e.slot != slot);
        q.push(Entry { slot, key, priority, seq: self.seq, job });
        cv.notify_one();
    }

    /// Change queued jobs' priorities: `f(slot, priority)` returns the new priority, or `None` to
    /// drop the job. Returns the dropped slots.
    pub fn reprioritize(&self, mut f: impl FnMut(&S, u32) -> Option<u32>) -> Vec<S> {
        let mut dropped = Vec::new();
        let mut q = self.shared.queue.lock().unwrap_or_else(|e| e.into_inner());
        q.retain_mut(|e| match f(&e.slot, e.priority) {
            Some(p) => {
                e.priority = p;
                true
            }
            None => {
                dropped.push(e.slot);
                false
            }
        });
        dropped
    }

    /// Queued (not started) jobs.
    pub fn queued(&self) -> usize {
        self.shared.queue.lock().map(|q| q.len()).unwrap_or(0)
    }

    /// Slots `f` puts in a group run one job of that group at a time: a newer job waits (and is
    /// replaced by an even newer one for its slot) until the running one is done. An interactive view (the loupe during a
    /// slider drag or a console turn) then renders the newest settings next instead of starting
    /// a render per change side by side — three or four drafts sharing the GPU each took three or
    /// four times as long, and the picture fell further behind with every detent.
    pub fn set_exclusive(&self, f: Option<Exclusive<S>>) {
        *self.shared.exclusive.lock().unwrap_or_else(|e| e.into_inner()) = f;
    }

    /// Is a job for `slot` queued?
    pub fn is_queued(&self, slot: S) -> bool {
        self.shared.queue.lock().map(|q| q.iter().any(|e| e.slot == slot)).unwrap_or(false)
    }

    /// A finished job, if any.
    /// Call `f` on the worker each time a result is ready (a UI wakes its event loop instead of
    /// polling every frame while jobs run).
    pub fn set_on_done(&self, f: Option<Waker>) {
        *self.shared.on_done.lock().unwrap_or_else(|e| e.into_inner()) = f;
    }

    /// Is a waker set ([`JobPool::set_on_done`])?
    pub fn wakes(&self) -> bool {
        self.shared.on_done.lock().unwrap_or_else(|e| e.into_inner()).is_some()
    }

    pub fn try_recv(&self) -> Option<Done<S, R>> {
        self.rx.try_recv().ok()
    }

    /// Run up to `n` jobs on the calling thread (wasm, tests). Returns how many ran.
    pub fn run_inline(&mut self, n: usize) -> usize {
        let mut ran = 0;
        while ran < n {
            let Some(e) = take(&self.shared, false) else { break };
            let result = (e.job)();
            finished(&self.shared, e.slot);
            let _ = self.tx.send(Done { slot: e.slot, key: e.key, result, ms: 0.0 });
            ran += 1;
        }
        ran
    }
}

/// Pop the best job (highest priority, newest first). Blocks when `wait` until one is queued or
/// the pool shuts down.
fn take<S: PartialEq + Copy, R>(shared: &Shared<S, R>, wait: bool) -> Option<Entry<S, R>> {
    let mut q = shared.queue.lock().unwrap_or_else(|e| e.into_inner());
    loop {
        if shared.shutdown.load(Ordering::Relaxed) {
            return None;
        }
        let exclusive = shared.exclusive.lock().unwrap_or_else(|e| e.into_inner()).clone();
        let mut running = shared.running.lock().unwrap_or_else(|e| e.into_inner());
        let group = |s: &S| exclusive.as_ref().and_then(|f| f(s));
        let blocked = |e: &Entry<S, R>| group(&e.slot).is_some_and(|g| running.iter().any(|(_, r)| *r == Some(g)));
        if let Some(i) = q.iter().enumerate().filter(|(_, e)| !blocked(e)).max_by_key(|(_, e)| (e.priority, e.seq)).map(|(i, _)| i) {
            let e = q.swap_remove(i);
            running.push((e.slot, group(&e.slot)));
            return Some(e);
        }
        drop(running);
        if !wait {
            return None;
        }
        q = shared.cv.wait(q).unwrap_or_else(|e| e.into_inner());
    }
}

/// A job for `slot` is done: a job of an exclusive slot waiting for it can start.
fn finished<S: PartialEq, R>(shared: &Shared<S, R>, slot: S) {
    let _q = shared.queue.lock().unwrap_or_else(|e| e.into_inner());
    let mut running = shared.running.lock().unwrap_or_else(|e| e.into_inner());
    if let Some(i) = running.iter().position(|(s, _)| *s == slot) {
        running.swap_remove(i);
    }
    shared.cv.notify_all();
}

impl<S, R> Drop for JobPool<S, R> {
    fn drop(&mut self) {
        self.shared.shutdown.store(true, Ordering::Relaxed);
        self.shared.cv.notify_all();
    }
}
