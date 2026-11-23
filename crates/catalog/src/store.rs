//! Byte storage for the catalog files (`catalog.snap`, `catalog.log`).
//!
//! [`FsStore`] is the native implementation (a library directory); [`MemStore`] keeps files in
//! memory (tests, ephemeral sessions) and lets tests simulate crashes by editing the bytes. A web
//! host can implement [`Store`] over OPFS.

use std::collections::BTreeMap;
use std::io;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};

/// A flat namespace of named files with the few operations the journal needs.
pub trait Store: Send {
    /// The whole file, or `None` if it doesn't exist.
    fn read(&mut self, name: &str) -> io::Result<Option<Vec<u8>>>;
    /// Replace a file atomically: after a crash either the old or the new content is visible,
    /// never a mix (native: write temp + fsync + rename + fsync directory).
    fn write_atomic(&mut self, name: &str, data: &[u8]) -> io::Result<()>;
    /// [`Store::write_atomic`] with streamed content: `fill` writes the new content into the
    /// given writer (an error aborts and leaves the old file). Same durability: the file is
    /// replaced only once all of it is written (native: and fsynced). Returns its length.
    ///
    /// The default buffers the content in memory; [`FsStore`] streams it into the temp file, so
    /// a large snapshot is never held in memory as a whole.
    fn write_atomic_with(&mut self, name: &str, fill: &mut dyn FnMut(&mut dyn io::Write) -> io::Result<()>) -> io::Result<u64> {
        let mut buf = Vec::new();
        fill(&mut buf)?;
        self.write_atomic(name, &buf)?;
        Ok(buf.len() as u64)
    }
    /// Append, and make durable. [`FsStore`] writes before returning (an app crash keeps the
    /// bytes) and fsyncs on a worker thread right after: a flush to the drive took 5–400 ms of the
    /// UI thread per star or flag while culling (QA-PERF). A failed fsync fails the next append.
    /// On an error, part of `data` may have been written (a full disk, a dropped network share):
    /// the journal cuts the file back to its previous length with [`Store::truncate`] before
    /// appending again.
    fn append(&mut self, name: &str, data: &[u8]) -> io::Result<()>;
    /// Cut a file to `len` bytes (drop a torn tail before appending again). A missing file is
    /// left missing.
    fn truncate(&mut self, name: &str, len: u64) -> io::Result<()>;
    /// Human-readable location (diagnostics).
    fn describe(&self) -> String;
    /// Another handle on the same files that a worker thread can write `catalog.snap` through
    /// while this one keeps appending to the log (background compaction). `None` (the default):
    /// snapshots are written synchronously.
    fn background_writer(&self) -> Option<Box<dyn Store>> {
        None
    }
}

/// Files in a directory.
pub struct FsStore {
    dir: PathBuf,
    /// Open append handle (kept between appends; dropped on rewrite/truncate).
    appender: Option<(String, std::fs::File)>,
    /// fsyncs appended bytes off the caller's thread (started by the first append).
    syncer: Option<Syncer>,
}

/// A worker that fsyncs the files [`FsStore::append`] wrote: appends queued while one fsync runs
/// are covered by the next (they share the file). Dropping it waits for the queued fsyncs.
struct Syncer {
    tx: Option<std::sync::mpsc::Sender<std::fs::File>>,
    state: Arc<SyncState>,
    thread: Option<std::thread::JoinHandle<()>>,
}

#[derive(Default)]
struct SyncState {
    /// fsyncs queued or running.
    pending: Mutex<usize>,
    idle: std::sync::Condvar,
    /// The first failed fsync, reported by the next append (or [`FsStore::flush`]).
    error: Mutex<Option<io::Error>>,
}

impl Syncer {
    fn start() -> Option<Syncer> {
        let (tx, rx) = std::sync::mpsc::channel::<std::fs::File>();
        let state = Arc::new(SyncState::default());
        let st = state.clone();
        let thread = std::thread::Builder::new()
            .name("catalog-fsync".into())
            .spawn(move || {
                while let Ok(f) = rx.recv() {
                    // everything queued meanwhile: one fsync per file covers all of its appends
                    let mut files = vec![f];
                    files.extend(rx.try_iter());
                    let n = files.len();
                    for f in files {
                        if let Err(e) = f.sync_data() {
                            st.error.lock().unwrap_or_else(std::sync::PoisonError::into_inner).get_or_insert(e);
                        }
                    }
                    let mut p = st.pending.lock().unwrap_or_else(std::sync::PoisonError::into_inner);
                    *p = p.saturating_sub(n);
                    st.idle.notify_all();
                }
            })
            .ok()?;
        Some(Syncer { tx: Some(tx), state, thread: Some(thread) })
    }

    /// Queue an fsync of `f`; gives it back when the worker is gone.
    fn send(&self, f: std::fs::File) -> Result<(), std::fs::File> {
        let Some(tx) = &self.tx else { return Err(f) };
        *self.state.pending.lock().unwrap_or_else(std::sync::PoisonError::into_inner) += 1;
        tx.send(f).map_err(|e| {
            let mut p = self.state.pending.lock().unwrap_or_else(std::sync::PoisonError::into_inner);
            *p = p.saturating_sub(1);
            e.0
        })
    }

    fn take_error(&self) -> Option<io::Error> {
        self.state.error.lock().unwrap_or_else(std::sync::PoisonError::into_inner).take()
    }

    /// Wait until every queued fsync has run.
    fn wait(&self) {
        let mut p = self.state.pending.lock().unwrap_or_else(std::sync::PoisonError::into_inner);
        while *p > 0 {
            p = self.state.idle.wait(p).unwrap_or_else(std::sync::PoisonError::into_inner);
        }
    }
}

impl Drop for Syncer {
    fn drop(&mut self) {
        self.tx = None;
        if let Some(t) = self.thread.take() {
            let _ = t.join();
        }
    }
}

impl FsStore {
    /// Use `dir` (created if missing).
    pub fn open(dir: impl AsRef<Path>) -> io::Result<FsStore> {
        std::fs::create_dir_all(dir.as_ref())?;
        Ok(FsStore { dir: dir.as_ref().to_path_buf(), appender: None, syncer: None })
    }

    /// Wait until everything appended so far is fsynced; the error of a failed fsync since the
    /// last append, if any.
    pub fn flush(&mut self) -> io::Result<()> {
        let Some(s) = &self.syncer else { return Ok(()) };
        s.wait();
        s.take_error().map_or(Ok(()), Err)
    }
    pub fn dir(&self) -> &Path {
        &self.dir
    }
    fn path(&self, name: &str) -> PathBuf {
        self.dir.join(name)
    }
    fn sync_dir(&self) {
        // Makes the rename durable on POSIX; opening a directory fails on Windows (where
        // `rename` is already journaled by NTFS), so errors are ignored.
        if let Ok(d) = std::fs::File::open(&self.dir) {
            let _ = d.sync_all();
        }
    }
}

impl Store for FsStore {
    fn read(&mut self, name: &str) -> io::Result<Option<Vec<u8>>> {
        match std::fs::read(self.path(name)) {
            Ok(b) => Ok(Some(b)),
            Err(e) if e.kind() == io::ErrorKind::NotFound => Ok(None),
            Err(e) => Err(e),
        }
    }

    fn write_atomic(&mut self, name: &str, data: &[u8]) -> io::Result<()> {
        use std::io::Write;
        if self.appender.as_ref().is_some_and(|(n, _)| n == name) {
            self.appender = None;
        }
        let tmp = self.path(&format!("{name}.tmp"));
        {
            let mut f = std::fs::File::create(&tmp)?;
            f.write_all(data)?;
            f.sync_all()?;
        }
        std::fs::rename(&tmp, self.path(name))?;
        self.sync_dir();
        Ok(())
    }

    fn write_atomic_with(&mut self, name: &str, fill: &mut dyn FnMut(&mut dyn io::Write) -> io::Result<()>) -> io::Result<u64> {
        use std::io::Write;
        if self.appender.as_ref().is_some_and(|(n, _)| n == name) {
            self.appender = None;
        }
        let tmp = self.path(&format!("{name}.tmp"));
        let written = (|| {
            let mut w = Counter { inner: io::BufWriter::with_capacity(1 << 20, std::fs::File::create(&tmp)?), n: 0 };
            fill(&mut w)?;
            w.flush()?;
            let f = w.inner.into_inner().map_err(|e| e.into_error())?;
            f.sync_all()?;
            Ok(w.n)
        })();
        let n = match written {
            Ok(n) => n,
            Err(e) => {
                let _ = std::fs::remove_file(&tmp);
                return Err(e);
            }
        };
        std::fs::rename(&tmp, self.path(name))?;
        self.sync_dir();
        Ok(n)
    }

    fn append(&mut self, name: &str, data: &[u8]) -> io::Result<()> {
        use std::io::Write;
        if self.appender.as_ref().is_none_or(|(n, _)| n != name) {
            let existed = self.path(name).exists();
            let f = std::fs::OpenOptions::new().create(true).append(true).open(self.path(name))?;
            if !existed {
                self.sync_dir();
            }
            self.appender = Some((name.to_string(), f));
        }
        // an earlier append whose fsync failed: this one fails (the handle may be dead)
        if let Some(e) = self.syncer.as_ref().and_then(Syncer::take_error) {
            self.appender = None;
            return Err(e);
        }
        if self.syncer.is_none() && !cfg!(target_arch = "wasm32") {
            self.syncer = Syncer::start();
        }
        let Some((_, f)) = self.appender.as_mut() else { return Err(io::Error::other("appender not open")) };
        let written = f.write_all(data).and_then(|()| {
            // fsync on the worker; on this thread when there is none (or no second handle)
            let queued = match (&self.syncer, f.try_clone()) {
                (Some(s), Ok(c)) => s.send(c).is_ok(),
                _ => false,
            };
            if queued { Ok(()) } else { f.sync_data() }
        });
        if written.is_err() {
            // The handle may be dead (a network share that reconnected, a removed volume): the next
            // attempt reopens the file. Part of `data` may be in the file; the caller cuts it back
            // ([`Store::truncate`]) before appending again.
            self.appender = None;
        }
        written
    }

    fn truncate(&mut self, name: &str, len: u64) -> io::Result<()> {
        if self.appender.as_ref().is_some_and(|(n, _)| n == name) {
            self.appender = None;
        }
        let f = match std::fs::OpenOptions::new().write(true).open(self.path(name)) {
            Ok(f) => f,
            // nothing to cut (an append that failed before creating the file)
            Err(e) if e.kind() == io::ErrorKind::NotFound => return Ok(()),
            Err(e) => return Err(e),
        };
        f.set_len(len)?;
        f.sync_all()
    }

    fn describe(&self) -> String {
        self.dir.display().to_string()
    }

    #[cfg(not(target_arch = "wasm32"))]
    fn background_writer(&self) -> Option<Box<dyn Store>> {
        Some(Box::new(FsStore { dir: self.dir.clone(), appender: None, syncer: None }))
    }
}

/// Counts the bytes written through it.
struct Counter<W> {
    inner: W,
    n: u64,
}

impl<W: io::Write> io::Write for Counter<W> {
    fn write(&mut self, buf: &[u8]) -> io::Result<usize> {
        let k = self.inner.write(buf)?;
        self.n += k as u64;
        Ok(k)
    }
    fn flush(&mut self) -> io::Result<()> {
        self.inner.flush()
    }
}

/// In-memory files. Clones share the same files (so a test can "reopen" or corrupt them).
#[derive(Clone, Default)]
pub struct MemStore {
    pub files: Arc<Mutex<BTreeMap<String, Vec<u8>>>>,
    /// Offer a [`Store::background_writer`] (off by default: snapshots stay synchronous).
    pub background: bool,
}

impl MemStore {
    pub fn new() -> MemStore {
        MemStore::default()
    }
    pub fn get(&self, name: &str) -> Option<Vec<u8>> {
        self.files.lock().unwrap_or_else(|e| e.into_inner()).get(name).cloned()
    }
    pub fn set(&self, name: &str, data: Vec<u8>) {
        self.files.lock().unwrap_or_else(|e| e.into_inner()).insert(name.to_string(), data);
    }
}

impl Store for MemStore {
    fn read(&mut self, name: &str) -> io::Result<Option<Vec<u8>>> {
        Ok(self.get(name))
    }
    fn write_atomic(&mut self, name: &str, data: &[u8]) -> io::Result<()> {
        self.set(name, data.to_vec());
        Ok(())
    }
    fn append(&mut self, name: &str, data: &[u8]) -> io::Result<()> {
        self.files.lock().unwrap_or_else(|e| e.into_inner()).entry(name.to_string()).or_default().extend_from_slice(data);
        Ok(())
    }
    fn truncate(&mut self, name: &str, len: u64) -> io::Result<()> {
        if let Some(f) = self.files.lock().unwrap_or_else(|e| e.into_inner()).get_mut(name) {
            f.truncate(len as usize);
        }
        Ok(())
    }
    fn describe(&self) -> String {
        "memory".into()
    }
    fn background_writer(&self) -> Option<Box<dyn Store>> {
        self.background.then(|| Box::new(self.clone()) as Box<dyn Store>)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Appends are in the file as soon as `append` returns (an app crash keeps them); their
    /// fsyncs run on the worker, which `flush` and dropping the store wait for.
    #[test]
    fn appends_are_written_at_once_and_fsynced_on_the_worker() {
        let dir = std::env::temp_dir().join(format!("lc-store-fsync-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        let mut s = FsStore::open(&dir).unwrap();
        let mut want = Vec::new();
        for k in 0..300 {
            let line = format!("record {k}\n");
            s.append("catalog.log", line.as_bytes()).unwrap();
            want.extend_from_slice(line.as_bytes());
            assert_eq!(std::fs::read(dir.join("catalog.log")).unwrap().len(), want.len(), "written before append returned");
        }
        assert!(s.syncer.is_some(), "fsyncs run on a worker");
        s.flush().unwrap();
        assert_eq!(*s.syncer.as_ref().unwrap().state.pending.lock().unwrap(), 0);
        // a rewrite and a truncate in between keep working; drop waits for the queued fsyncs
        s.truncate("catalog.log", 9).unwrap();
        s.append("catalog.log", b"after\n").unwrap();
        drop(s);
        assert_eq!(std::fs::read(dir.join("catalog.log")).unwrap(), b"record 0\nafter\n");
        let _ = std::fs::remove_dir_all(&dir);
    }
}
