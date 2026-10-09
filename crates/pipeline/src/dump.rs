//! Per-pixel measurement dumps for `tools/lr-compare` (Bryan's fork): `LIGHTCRAFT_LR_DUMP=<file>`
//! makes the next CPU render write, per output pixel, [`N`] floats — scene EV after exposure, the
//! Highlights/Shadows EV, all local EV moves, edge-aware base EV, local level EV, halo weight, tone
//! input EV, mask weight, encoded result (3), normalised position (2), saturation, local
//! exposure, 0 — after a header `[w, h, N, out_w, out_h, exposure EV, key offset, Highlights,
//! Shadows]`, little-endian `f32`. Unset (always, in normal use) it
//! costs one environment read per render. Write failures are ignored (a diagnostic).

use std::sync::Mutex;

pub const N: usize = 16;

pub struct Dump {
    path: std::ffi::OsString,
    w: usize,
    h: usize,
    buf: Mutex<Vec<f32>>,
}

impl Dump {
    pub fn from_env(w: usize, h: usize) -> Option<Dump> {
        let path = std::env::var_os("LIGHTCRAFT_LR_DUMP")?;
        let n = w.checked_mul(h)?.checked_mul(N).filter(|n| *n <= 1 << 28)?;
        Some(Dump { path, w, h, buf: Mutex::new(vec![0.0; n]) })
    }

    pub fn put(&self, i: usize, v: [f32; N]) {
        let mut g = self.buf.lock().unwrap_or_else(|e| e.into_inner());
        if let Some(slot) = g.get_mut(i * N..i * N + N) {
            slot.copy_from_slice(&v);
        }
    }

    pub fn write(self, extra: [f32; 6]) {
        let g = self.buf.into_inner().unwrap_or_else(|e| e.into_inner());
        let mut bytes = Vec::with_capacity(g.len() * 4 + 24);
        for v in [self.w as f32, self.h as f32, N as f32].into_iter().chain(extra).chain(g) {
            bytes.extend_from_slice(&v.to_le_bytes());
        }
        let _ = std::fs::write(&self.path, bytes);
    }
}
