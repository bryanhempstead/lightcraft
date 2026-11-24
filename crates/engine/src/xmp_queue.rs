//! Deferred XMP auto-write (the app's way; [`Session::set_xmp_deferred`]).
//!
//! With auto-write on, every committed change used to write the photo's sidecar before the command
//! returned — on the UI thread, on the photo's drive (often a slow external volume), after a walk
//! over the whole catalog for shared stems. Deferred, a change only queues the photo: once it has
//! been still for [`XMP_DEBOUNCE`] (at once when the photo is left), its sidecar is written on a
//! worker thread, several changes to it as one write. The journal stays the source of truth: the
//! queue itself is a library record (`xmp.pending`, written with the change that queued it), so
//! after a crash the next start writes what was still waiting. Quit, closing the library and
//! turning deferral off write everything queued first ([`Session::flush_sidecars`]).
//!
//! Frontends without a frame loop (CLI, MCP, tests) leave deferral off and keep writing at once.

use std::collections::BTreeMap;
use std::time::Duration;

use lightcraft_catalog::{Op, PhotoId, XmpSync};
use web_time::Instant;

use crate::Session;
use crate::sidecar::{SidecarWrite, StemOwners};

/// How long a photo has to be left alone before its sidecar is written.
pub const XMP_DEBOUNCE: Duration = Duration::from_millis(1500);

/// The library record listing photos whose sidecar is still to be written.
const PENDING_RECORD: &str = "xmp.pending";

type Written = Vec<(PhotoId, Result<XmpSync, String>)>;

#[derive(Default)]
pub(crate) struct XmpQueue {
    /// Queue instead of writing at once.
    pub(crate) on: bool,
    /// Photo → when its sidecar is due.
    due: BTreeMap<PhotoId, Instant>,
    /// The writes in flight (one batch at a time, so one photo is never written twice at once).
    job: Option<std::thread::JoinHandle<Written>>,
    /// The ids the `xmp.pending` record holds now.
    recorded: Vec<PhotoId>,
}

impl Session {
    /// Defer XMP auto-write to a worker thread ([`crate::xmp_queue`]); the app turns this on. Off
    /// writes whatever is queued first.
    pub fn set_xmp_deferred(&mut self, on: bool) {
        if !on {
            self.flush_sidecars();
        }
        self.xmp_queue.on = on;
    }

    /// Photos whose sidecar is queued or being written.
    pub fn sidecars_pending(&self) -> usize {
        self.xmp_queue.due.len() + usize::from(self.xmp_queue.job.is_some())
    }

    /// Queue photos changed by a command (each due [`XMP_DEBOUNCE`] after its latest change).
    pub(crate) fn queue_sidecars(&mut self, ids: Vec<PhotoId>) {
        let at = Instant::now() + XMP_DEBOUNCE;
        for id in ids {
            self.xmp_queue.due.insert(id, at);
        }
        self.record_pending();
    }

    /// Photo `id` was left: write its queued sidecar now rather than after the debounce.
    pub(crate) fn sidecar_due_now(&mut self, id: PhotoId) {
        if let Some(at) = self.xmp_queue.due.get_mut(&id) {
            *at = Instant::now();
        }
    }

    /// Once a frame: take in finished writes, start the due ones. Cheap when nothing is queued.
    pub fn poll_sidecars(&mut self) {
        if self.xmp_queue.job.as_ref().is_some_and(std::thread::JoinHandle::is_finished) {
            self.finish_sidecar_job();
        }
        if self.xmp_queue.job.is_some() || self.xmp_queue.due.is_empty() {
            return;
        }
        let now = Instant::now();
        let ready: Vec<PhotoId> = self.xmp_queue.due.iter().filter(|(_, at)| **at <= now).map(|(id, _)| *id).collect();
        if ready.is_empty() {
            return;
        }
        let writes = self.prepare_sidecars(&ready);
        let spawned = std::thread::Builder::new().name("xmp-writer".into()).spawn(move || run_writes(writes));
        match spawned {
            Ok(h) => self.xmp_queue.job = Some(h),
            Err(e) => {
                // no thread (wasm, or out of them): write them here; they are taken off the queue
                // already, so prepare again
                log::warn!("auto-write XMP: no writer thread ({e}); writing now");
                let written = run_writes(self.prepare_ids(&ready));
                self.take_written(written);
            }
        }
    }

    /// Write everything queued now (quit, closing the library, deferral off), waiting for the
    /// writes in flight first.
    pub fn flush_sidecars(&mut self) {
        if self.xmp_queue.job.is_some() {
            self.finish_sidecar_job();
        }
        if self.xmp_queue.due.is_empty() {
            return;
        }
        let ids: Vec<PhotoId> = self.xmp_queue.due.keys().copied().collect();
        let writes = self.prepare_sidecars(&ids);
        let written = run_writes(writes);
        self.take_written(written);
    }

    /// Take `ids` off the queue and gather what writing them needs (photos that can't have a
    /// sidecar any more — deleted, virtual copies — are dropped).
    fn prepare_sidecars(&mut self, ids: &[PhotoId]) -> Vec<SidecarWrite> {
        for id in ids {
            self.xmp_queue.due.remove(id);
        }
        self.prepare_ids(ids)
    }

    fn prepare_ids(&self, ids: &[PhotoId]) -> Vec<SidecarWrite> {
        let owners = StemOwners::for_photos(&self.catalog, ids);
        ids.iter().filter_map(|id| self.prepare_sidecar(*id, &owners).ok()).collect()
    }

    fn finish_sidecar_job(&mut self) {
        let Some(h) = self.xmp_queue.job.take() else { return };
        let written = h.join().unwrap_or_else(|_| {
            log::error!("auto-write XMP: the writer thread panicked");
            Vec::new()
        });
        self.take_written(written);
    }

    /// Record what the writes did (the sync status) and what is still queued.
    fn take_written(&mut self, written: Written) {
        let mut synced = Vec::new();
        for (id, r) in written {
            match r {
                Ok(sync) => synced.push((id, sync)),
                Err(e) => log::warn!("auto-write XMP: {e}"),
            }
        }
        self.record_xmp_sync(synced);
        // (outside a command: nothing else takes these)
        if self.depth == 0 {
            self.xmp_synced_now.clear();
        }
        self.record_pending();
    }

    /// Keep the `xmp.pending` record in step with the queue (+ the writes in flight), so a crash
    /// loses no sidecar write. Written with the next save, like the change that queued them.
    fn record_pending(&mut self) {
        let mut ids: Vec<PhotoId> = self.xmp_queue.due.keys().copied().collect();
        if self.xmp_queue.job.is_some() {
            // in flight: keep them listed until they are done
            ids.extend(self.xmp_queue.recorded.iter().copied().filter(|i| !self.xmp_queue.due.contains_key(i)));
        }
        ids.sort_unstable();
        ids.dedup();
        if ids == self.xmp_queue.recorded || self.library.is_none() {
            return;
        }
        let value = if ids.is_empty() { None } else { Some(serde_json::json!(ids.iter().map(|i| i.0).collect::<Vec<_>>())) };
        if let Err(e) = self.apply_bookkeeping(Op::SetRecord { key: PENDING_RECORD.into(), value }) {
            log::warn!("auto-write XMP: {e}");
        }
        self.xmp_queue.recorded = ids;
    }

    /// A library was opened: queue the sidecars a previous run left unwritten (it crashed).
    pub(crate) fn requeue_pending_sidecars(&mut self) {
        self.xmp_queue.due.clear();
        self.xmp_queue.recorded = Vec::new();
        let ids: Vec<PhotoId> = self
            .catalog
            .record(PENDING_RECORD)
            .and_then(serde_json::Value::as_array)
            .map(|a| a.iter().filter_map(serde_json::Value::as_u64).map(PhotoId).collect())
            .unwrap_or_default();
        if ids.is_empty() {
            return;
        }
        log::info!("auto-write XMP: {} sidecar(s) left unwritten last time, writing them", ids.len());
        self.xmp_queue.recorded = ids.clone();
        let now = Instant::now();
        for id in ids {
            self.xmp_queue.due.insert(id, now);
        }
        if !self.xmp_queue.on {
            self.flush_sidecars();
        }
    }
}

fn run_writes(writes: Vec<SidecarWrite>) -> Written {
    writes
        .into_iter()
        .map(|w| {
            let id = w.id;
            let r = crate::guard::catch("XMP sidecar write", || w.run().map_err(|e| e.to_string())).unwrap_or_else(Err);
            (id, r.map(|s| s.sync))
        })
        .collect()
}
