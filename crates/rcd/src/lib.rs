//  Ratio Corrected Demosaicing (RCD), ported to Rust for LightCraft.
//
//  Copyright (c) 2017-2020 Luis Sanz Rodriguez (luis.sanz.rodriguez(at)gmail(dot)com) and Ingo Weyrich
//  (heckflosse67@gmx.de). Original code: https://github.com/LuisSR/RCD-Demosaicing (release 2.3), as tiled and
//  optimised in RawTherapee (rtengine/rcd_demosaic.cc), in cooperation with Hanno Schwalm.
//
//  This program is free software: you can redistribute it and/or modify it under the terms of the GNU General
//  Public License as published by the Free Software Foundation, either version 3 of the License, or (at your
//  option) any later version.
//
//  This program is distributed in the hope that it will be useful, but WITHOUT ANY WARRANTY; without even the
//  implied warranty of MERCHANTABILITY or FITNESS FOR A PARTICULAR PURPOSE. See the GNU General Public License for
//  more details. You should have received a copy of the GNU General Public License along with this program. If not,
//  see <https://www.gnu.org/licenses/>.
//
//  Modified by the LightCraft contributors (2026): translated from C++ to Rust; the picture is processed in
//  full-width horizontal bands (in parallel) over a mirror-padded copy, so no separate border interpolation is
//  needed; values are not clipped to 1 (scene-linear input with highlights above white).

//! RCD: green is interpolated along the vertical or horizontal direction the local colour-difference high-pass
//! statistics favour, with ratio-corrected (low-pass weighted) estimates; red and blue then follow along the
//! diagonals and cardinals the same way. Fewer zipper and maze artefacts than AHD on fine detail, a fraction of the
//! cost. LightCraft's Raw Details demosaic.
#![forbid(unsafe_code)]
#![deny(clippy::unwrap_used, clippy::expect_used, clippy::panic, clippy::unimplemented, clippy::todo, clippy::unreachable)]

use rayon::prelude::*;

/// Rows each band produces (even, so a band starts on the pattern's phase).
const BAND: usize = 128;
/// Mirrored margin around the picture (even, at least RCD's border of 9).
const B: usize = 10;
const EPS: f32 = 1e-5;
const EPSSQ: f32 = 1e-10;

/// Largest picture accepted (samples).
pub const MAX_PIXELS: usize = 400_000_000;

#[inline]
fn sqr(x: f32) -> f32 {
    x * x
}

#[inline]
fn intp(a: f32, b: f32, c: f32) -> f32 {
    a * (b - c) + c
}

/// Mirror `i` into `0..n` keeping its parity (the colour filter's phase).
#[inline]
fn reflect(i: isize, n: usize) -> usize {
    let n = n as isize;
    if n <= 1 {
        return 0;
    }
    let period = 2 * (n - 1);
    let mut i = i.rem_euclid(period);
    if i >= n {
        i = period - i;
    }
    i as usize
}

/// Demosaic a Bayer mosaic (`width × height` samples, row-major; `cfa[y & 1][x & 1]` is the colour at `(x, y)`:
/// 0 red, 1 green, 2 blue). `None` when the pattern isn't a Bayer one (two greens on a diagonal, one red, one blue)
/// or the sizes don't fit.
pub fn rcd(mosaic: &[f32], width: usize, height: usize, cfa: [[u8; 2]; 2]) -> Option<Vec<[f32; 3]>> {
    let n = width.checked_mul(height).filter(|&n| n > 0 && n <= MAX_PIXELS)?;
    if mosaic.len() != n || width < 2 || height < 2 {
        return None;
    }
    let mut counts = [0u8; 3];
    for c in cfa.iter().flatten() {
        *counts.get_mut(*c as usize)? += 1;
    }
    if counts != [1, 2, 1] || cfa[0][0] == 1 && cfa[1][1] != 1 || cfa[0][1] == 1 && cfa[1][0] != 1 {
        return None;
    }
    let mut out = vec![[0f32; 3]; n];
    out.par_chunks_mut(BAND * width).enumerate().for_each(|(i, rows)| band(mosaic, width, height, cfa, i * BAND, rows));
    Some(out)
}

/// Demosaic image rows `r0 .. r0 + out.len() / width` into `out`.
fn band(mosaic: &[f32], width: usize, height: usize, cfa: [[u8; 2]; 2], r0: usize, out: &mut [[f32; 3]]) {
    let rows_out = out.len() / width;
    let rows = rows_out + 2 * B;
    // an even stride keeps `index / 2` a cell index on every row
    let stride = (width + 2 * B + 1) & !1;
    let fc = |row: usize, col: usize| cfa[row & 1][col & 1] as usize;
    let (w1, w2, w3, w4) = (stride, 2 * stride, 3 * stride, 4 * stride);
    let len = rows * stride;

    // buffer (row, col) ↔ image (r0 - B + row, col - B), mirrored at the edges (B is even: same colour phase)
    let mut cfa_buf = vec![0f32; len];
    for row in 0..rows {
        let y = reflect(r0 as isize - B as isize + row as isize, height);
        for col in 0..stride {
            let x = reflect(col as isize - B as isize, width);
            cfa_buf[row * stride + col] = mosaic.get(y * width + x).copied().unwrap_or(0.0).max(0.0);
        }
    }
    let c = &cfa_buf;
    let mut rgb = [c.clone(), c.clone(), c.clone()];
    let mut vh = vec![0f32; len];
    let half = len / 2 + 1;
    let mut lpf = vec![0f32; half];

    // Step 1: vertical / horizontal discrimination
    {
        let wv = stride.saturating_sub(8);
        let mut bv = [vec![0f32; wv], vec![0f32; wv], vec![0f32; wv]];
        let mut bh = vec![0f32; stride.saturating_sub(6)];
        let vstat = |indx: usize| sqr((c[indx - w3] - c[indx - w1] - c[indx + w1] + c[indx + w3]) - 3.0 * (c[indx - w2] + c[indx + w2]) + 6.0 * c[indx]);
        for row in 3..(rows - 3).min(5) {
            for col in 4..stride - 4 {
                bv[row - 3][col - 4] = vstat(row * stride + col);
            }
        }
        let (mut v0, mut v1, mut v2) = (0usize, 1usize, 2usize);
        for row in 4..rows - 4 {
            for col in 3..stride - 3 {
                let indx = row * stride + col;
                bh[col - 3] = sqr((c[indx - 3] - c[indx - 1] - c[indx + 1] + c[indx + 3]) - 3.0 * (c[indx - 2] + c[indx + 2]) + 6.0 * c[indx]);
            }
            for col in 4..stride - 4 {
                bv[v2][col - 4] = vstat((row + 1) * stride + col);
            }
            for col in 4..stride - 4 {
                let v_stat = EPSSQ.max(bv[v0][col - 4] + bv[v1][col - 4] + bv[v2][col - 4]);
                let h_stat = EPSSQ.max(bh[col - 4] + bh[col - 3] + bh[col - 2]);
                vh[row * stride + col] = v_stat / (v_stat + h_stat);
            }
            // rotate rows: (v0, v1, v2) ← (v1, v2, v0)
            (v0, v1, v2) = (v1, v2, v0);
        }
    }

    // Step 2: low-pass filter over the red and blue positions
    for row in 2..rows - 2 {
        let mut col = 2 + (fc(row, 0) & 1);
        while col < stride - 2 {
            let indx = row * stride + col;
            lpf[indx / 2] = c[indx]
                + 0.5 * (c[indx - w1] + c[indx + w1] + c[indx - 1] + c[indx + 1])
                + 0.25 * (c[indx - w1 - 1] + c[indx - w1 + 1] + c[indx + w1 - 1] + c[indx + w1 + 1]);
            col += 2;
        }
    }

    // Step 3: green at the red and blue positions
    for row in 4..rows - 4 {
        let mut col = 4 + (fc(row, 0) & 1);
        while col < stride - 4 {
            let indx = row * stride + col;
            let lp = indx / 2;
            let ci = c[indx];
            let n_grad = EPS + ((c[indx - w1] - c[indx + w1]).abs() + (ci - c[indx - w2]).abs()) + ((c[indx - w1] - c[indx - w3]).abs() + (c[indx - w2] - c[indx - w4]).abs());
            let s_grad = EPS + ((c[indx - w1] - c[indx + w1]).abs() + (ci - c[indx + w2]).abs()) + ((c[indx + w1] - c[indx + w3]).abs() + (c[indx + w2] - c[indx + w4]).abs());
            let w_grad = EPS + ((c[indx - 1] - c[indx + 1]).abs() + (ci - c[indx - 2]).abs()) + ((c[indx - 1] - c[indx - 3]).abs() + (c[indx - 2] - c[indx - 4]).abs());
            let e_grad = EPS + ((c[indx - 1] - c[indx + 1]).abs() + (ci - c[indx + 2]).abs()) + ((c[indx + 1] - c[indx + 3]).abs() + (c[indx + 2] - c[indx + 4]).abs());
            let l = lpf[lp];
            let n_est = c[indx - w1] * (l + l) / (EPS + l + lpf[lp - w1]);
            let s_est = c[indx + w1] * (l + l) / (EPS + l + lpf[lp + w1]);
            let w_est = c[indx - 1] * (l + l) / (EPS + l + lpf[lp - 1]);
            let e_est = c[indx + 1] * (l + l) / (EPS + l + lpf[lp + 1]);
            let v_est = (s_grad * n_est + n_grad * s_est) / (n_grad + s_grad);
            let h_est = (w_grad * e_est + e_grad * w_est) / (e_grad + w_grad);
            let central = vh[indx];
            let neigh = 0.25 * ((vh[indx - w1 - 1] + vh[indx - w1 + 1]) + (vh[indx + w1 - 1] + vh[indx + w1 + 1]));
            let disc = if (0.5 - central).abs() < (0.5 - neigh).abs() { neigh } else { central };
            rgb[1][indx] = intp(disc, h_est, v_est);
            col += 2;
        }
    }

    // Step 4.0: P / Q diagonal high-pass statistics (the low-pass buffer is free again)
    let mut p_hpf = vec![0f32; half];
    let mut q_hpf = vec![0f32; half];
    for row in 3..rows - 3 {
        let mut col = 3;
        while col < stride - 3 {
            let indx = row * stride + col;
            p_hpf[indx / 2] = sqr(
                (c[indx - w3 - 3] - c[indx - w1 - 1] - c[indx + w1 + 1] + c[indx + w3 + 3]) - 3.0 * (c[indx - w2 - 2] + c[indx + w2 + 2]) + 6.0 * c[indx],
            );
            q_hpf[indx / 2] = sqr(
                (c[indx - w3 + 3] - c[indx - w1 + 1] - c[indx + w1 - 1] + c[indx + w3 - 3]) - 3.0 * (c[indx - w2 + 2] + c[indx + w2 - 2]) + 6.0 * c[indx],
            );
            col += 2;
        }
    }
    // Step 4.1: P / Q discrimination (into the low-pass buffer)
    let pq = &mut lpf;
    for row in 4..rows - 4 {
        let mut col = 4 + (fc(row, 0) & 1);
        while col < stride - 4 {
            let indx = row * stride + col;
            let (i2, i3, i4) = (indx / 2, (indx - w1 - 1) / 2, (indx + w1 - 1) / 2);
            let p_stat = EPSSQ.max(p_hpf[i3] + p_hpf[i2] + p_hpf[i4 + 1]);
            let q_stat = EPSSQ.max(q_hpf[i3 + 1] + q_hpf[i2] + q_hpf[i4]);
            pq[i2] = p_stat / (p_stat + q_stat);
            col += 2;
        }
    }
    drop(p_hpf);
    drop(q_hpf);

    // Step 4.2: red at blue and blue at red
    for row in 4..rows - 4 {
        let mut col = 4 + (fc(row, 0) & 1);
        while col < stride - 4 {
            let indx = row * stride + col;
            let ch = 2 - fc(row, col);
            let (p1, p2, p3) = (indx / 2, (indx - w1 - 1) / 2, (indx + w1 - 1) / 2);
            let central = pq[p1];
            let neigh = 0.25 * (pq[p2] + pq[p2 + 1] + pq[p3] + pq[p3 + 1]);
            let disc = if (0.5 - central).abs() < (0.5 - neigh).abs() { neigh } else { central };
            let (rc, g) = (&rgb[ch], &rgb[1]);
            let nw_grad = EPS + (rc[indx - w1 - 1] - rc[indx + w1 + 1]).abs() + (rc[indx - w1 - 1] - rc[indx - w3 - 3]).abs() + (g[indx] - g[indx - w2 - 2]).abs();
            let ne_grad = EPS + (rc[indx - w1 + 1] - rc[indx + w1 - 1]).abs() + (rc[indx - w1 + 1] - rc[indx - w3 + 3]).abs() + (g[indx] - g[indx - w2 + 2]).abs();
            let sw_grad = EPS + (rc[indx - w1 + 1] - rc[indx + w1 - 1]).abs() + (rc[indx + w1 - 1] - rc[indx + w3 - 3]).abs() + (g[indx] - g[indx + w2 - 2]).abs();
            let se_grad = EPS + (rc[indx - w1 - 1] - rc[indx + w1 + 1]).abs() + (rc[indx + w1 + 1] - rc[indx + w3 + 3]).abs() + (g[indx] - g[indx + w2 + 2]).abs();
            let nw_est = rc[indx - w1 - 1] - g[indx - w1 - 1];
            let ne_est = rc[indx - w1 + 1] - g[indx - w1 + 1];
            let sw_est = rc[indx + w1 - 1] - g[indx + w1 - 1];
            let se_est = rc[indx + w1 + 1] - g[indx + w1 + 1];
            let p_est = (nw_grad * se_est + se_grad * nw_est) / (nw_grad + se_grad);
            let q_est = (ne_grad * sw_est + sw_grad * ne_est) / (ne_grad + sw_grad);
            let v = g[indx] + intp(disc, q_est, p_est);
            rgb[ch][indx] = v;
            col += 2;
        }
    }

    // Step 4.3: red and blue at the green positions
    for row in 4..rows - 4 {
        let mut col = 4 + (fc(row, 1) & 1);
        while col < stride - 4 {
            let indx = row * stride + col;
            let central = vh[indx];
            let neigh = 0.25 * ((vh[indx - w1 - 1] + vh[indx - w1 + 1]) + (vh[indx + w1 - 1] + vh[indx + w1 + 1]));
            let disc = if (0.5 - central).abs() < (0.5 - neigh).abs() { neigh } else { central };
            let g = &rgb[1];
            let g1 = g[indx];
            let n1 = EPS + (g1 - g[indx - w2]).abs();
            let s1 = EPS + (g1 - g[indx + w2]).abs();
            let w1g = EPS + (g1 - g[indx - 2]).abs();
            let e1 = EPS + (g1 - g[indx + 2]).abs();
            let (gmw1, gpw1, gm1, gp1) = (g[indx - w1], g[indx + w1], g[indx - 1], g[indx + 1]);
            let mut vals = [0f32; 2];
            for (k, ch) in [0usize, 2].into_iter().enumerate() {
                let r = &rgb[ch];
                let sn = (r[indx - w1] - r[indx + w1]).abs();
                let ew = (r[indx - 1] - r[indx + 1]).abs();
                let n_grad = n1 + sn + (r[indx - w1] - r[indx - w3]).abs();
                let s_grad = s1 + sn + (r[indx + w1] - r[indx + w3]).abs();
                let w_grad = w1g + ew + (r[indx - 1] - r[indx - 3]).abs();
                let e_grad = e1 + ew + (r[indx + 1] - r[indx + 3]).abs();
                let n_est = r[indx - w1] - gmw1;
                let s_est = r[indx + w1] - gpw1;
                let w_est = r[indx - 1] - gm1;
                let e_est = r[indx + 1] - gp1;
                let v_est = (n_grad * s_est + s_grad * n_est) / (n_grad + s_grad);
                let h_est = (e_grad * w_est + w_grad * e_est) / (e_grad + w_grad);
                vals[k] = g1 + intp(disc, h_est, v_est);
            }
            rgb[0][indx] = vals[0];
            rgb[2][indx] = vals[1];
            col += 2;
        }
    }

    for (y, row) in out.chunks_exact_mut(width).enumerate() {
        let base = (y + B) * stride + B;
        for (x, o) in row.iter_mut().enumerate() {
            let i = base + x;
            *o = [rgb[0][i].max(0.0), rgb[1][i].max(0.0), rgb[2][i].max(0.0)];
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const RGGB: [[u8; 2]; 2] = [[0, 1], [1, 2]];

    fn mosaic(w: usize, h: usize, cfa: [[u8; 2]; 2], f: impl Fn(usize, usize) -> [f32; 3]) -> Vec<f32> {
        (0..w * h).map(|i| f(i % w, i / w)[cfa[(i / w) & 1][(i % w) & 1] as usize]).collect()
    }

    #[test]
    fn a_flat_colour_comes_back_exactly() {
        for cfa in [RGGB, [[1, 0], [2, 1]], [[2, 1], [1, 0]], [[1, 2], [0, 1]]] {
            for (w, h) in [(37, 29), (2, 2), (300, 131)] {
                let m = mosaic(w, h, cfa, |_, _| [0.3, 0.5, 0.2]);
                let out = rcd(&m, w, h, cfa).unwrap();
                for p in &out {
                    for (a, b) in p.iter().zip([0.3, 0.5, 0.2]) {
                        assert!((a - b).abs() < 1e-4, "{cfa:?} {w}×{h}: {p:?}");
                    }
                }
            }
        }
    }

    #[test]
    fn the_samples_stay_and_edges_beat_bilinear() {
        // a sharp-edged disc: RCD's error is below a bilinear demosaic's
        let (w, h) = (96, 80);
        let scene = |x: usize, y: usize| {
            let d = ((x as f32 - 48.0).powi(2) + (y as f32 - 40.0).powi(2)).sqrt();
            if d < 25.0 { [0.8, 0.6, 0.3] } else { [0.1, 0.2, 0.4] }
        };
        let m = mosaic(w, h, RGGB, scene);
        let out = rcd(&m, w, h, RGGB).unwrap();
        let mut err = 0.0;
        let mut err_bl = 0.0;
        for y in 2..h - 2 {
            for x in 2..w - 2 {
                let t = scene(x, y);
                let i = y * w + x;
                let own = RGGB[y & 1][x & 1] as usize;
                assert!((out[i][own] - m[i]).abs() < 1e-6, "a pixel's own sample is kept");
                // bilinear: mean of the same colour among the 3 × 3 neighbours
                for c in 0..3 {
                    let (mut s, mut k) = (0.0, 0.0);
                    for dy in 0..3 {
                        for dx in 0..3 {
                            let (xx, yy) = (x + dx - 1, y + dy - 1);
                            if RGGB[yy & 1][xx & 1] as usize == c {
                                s += m[yy * w + xx];
                                k += 1.0;
                            }
                        }
                    }
                    err_bl += (s / k - t[c]).powi(2);
                    err += (out[i][c] - t[c]).powi(2);
                }
            }
        }
        assert!(err < err_bl * 0.8, "rcd {err} vs bilinear {err_bl}");
    }

    #[test]
    fn hostile_sizes_are_refused() {
        assert!(rcd(&[0.0; 4], 2, 2, [[0, 0], [1, 2]]).is_none());
        assert!(rcd(&[0.0; 3], 2, 2, RGGB).is_none());
        assert!(rcd(&[], 0, 0, RGGB).is_none());
        assert!(rcd(&[0.0; 4], usize::MAX, 2, RGGB).is_none());
        assert!(rcd(&[0.0, 1.0, f32::NAN, 0.5], 2, 2, RGGB).is_some());
    }
}
