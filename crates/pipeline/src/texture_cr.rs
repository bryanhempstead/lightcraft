//! Presence ▸ Texture as Camera Raw renders it (R21, Bryan's fork; `docs/lr-match.md` → Round 10).
//!
//! Ported from RAWmakase (`crates/rawmakase-engine/src/develop/texture.rs` and `laplacian.rs`,
//! <https://github.com/pch/rawmakase>, MIT, Copyright (c) 2026 RAWmakase contributors; see
//! NOTICE), whose constants were fitted to Camera Raw 18.7 renders of sine gratings, flats and
//! edges. Camera Raw's Texture is a local contrast of log luminance over a broad band of scales
//! (a few to about thirty pixels of the full-resolution photo): faint detail is boosted most
//! (×1.78 at ±0.1 EV and +100) and strong contrast hardly at all, large flats stay as they are.
//! Here: a Laplacian pyramid (5-tap binomial reduce / expand) of the log luminance, each level
//! compressed where it is strong (`band / (1 + (band / t)²)`) and weighted, the sum scaled by a
//! strength that the slider sets. RAWmakase takes the detail of each colour channel; LightCraft
//! applies it to log luminance like its other local-contrast operators (the masks' Texture reads
//! the same plane). Checked against our own Camera Raw oracle renders (`oracle/make_fx.py fx2`).

use lightcraft_raster::{Plane, par_rows};

/// Render Texture (global and in masks) with this operator; `false` brings back LightCraft's
/// older single-band texture (one line, as for `adobe::HUE_SAT_IN_FINISH`).
pub const MEASURED: bool = true;

/// Take each channel's detail in the camera's own (white-balanced) RGB when the source has a
/// camera colour model (round 11), as RAWmakase does: the camera matrix then spreads a colour
/// edge's channel detail into chroma the way Camera Raw's Texture does at colour edges.
pub const CAMERA_SPACE: bool = true;

/// The working → camera matrices Texture's detail is taken in for `info` (and back), if any.
pub fn space(info: &crate::SourceInfo) -> Option<([[f32; 3]; 3], [[f32; 3]; 3])> {
    let cw = info.camera_wb.as_ref().filter(|_| CAMERA_SPACE && !crate::scene_cr::r11_off())?;
    let ok = |m: &[[f32; 3]; 3]| m.iter().flatten().all(|v| v.is_finite());
    (ok(&cw.from_working) && ok(&cw.to_working)).then_some((cw.from_working, cw.to_working))
}

/// `c` (linear working RGB) with each channel's Texture gain `g` applied in `space` ([`space`]).
#[inline]
pub fn apply(c: [f32; 3], g: [f32; 3], space: Option<&([[f32; 3]; 3], [[f32; 3]; 3])>) -> [f32; 3] {
    let mul = |m: &[[f32; 3]; 3], v: [f32; 3]| -> [f32; 3] { std::array::from_fn(|r| m[r][0] * v[0] + m[r][1] * v[1] + m[r][2] * v[2]) };
    match space {
        Some((from, to)) => {
            let k = mul(from, c);
            mul(to, [k[0] * g[0], k[1] * g[1], k[2] * g[2]])
        }
        None => [c[0] * g[0], c[1] * g[1], c[2] * g[2]],
    }
}

/// Pyramid levels, in full-resolution pixels: level `l` holds detail about `2^l` pixels across.
pub const LEVELS: usize = 6;
/// Each level's weight and the log2 contrast above which it is compressed.
pub const WEIGHTS: [f32; LEVELS] = [1.402, 0.665, 1.571, 0.583, 0., 0.072];
pub const THRESHOLDS: [f32; LEVELS] = [0.506, 0.098, 0.167, 0.192, 0.135, 0.176];
/// Strength by Texture (−1..1).
const AMOUNTS: [f32; 6] = [-1., -0.5, 0., 0.25, 0.5, 1.];
const STRENGTH: [f32; 6] = [-0.752, -0.499, 0., 0.334, 0.546, 0.852];

/// The detail's scale for Texture `amount` (−1..1; masks add to the global slider, the sum is
/// clamped).
#[inline]
pub fn strength(amount: f32) -> f32 {
    if !amount.is_finite() {
        return 0.0;
    }
    let a = amount.clamp(-1.0, 1.0);
    let i = AMOUNTS.partition_point(|v| *v <= a).clamp(1, AMOUNTS.len() - 1);
    let t = (a - AMOUNTS[i - 1]) / (AMOUNTS[i] - AMOUNTS[i - 1]);
    STRENGTH[i - 1] + (STRENGTH[i] - STRENGTH[i - 1]) * t
}

/// The pyramid level of the full-resolution photo that a render's finest level shows: a render
/// at `px_per_long` of a photo `native_long` px long starts `round(log2(native / render))`
/// levels down (0 when the native size is unknown or the render is larger).
pub fn level_offset(px_per_long: f64, native_long: f32) -> usize {
    if px_per_long.is_nan() || px_per_long <= 0.0 || native_long.is_nan() || native_long <= 0.0 {
        return 0;
    }
    let r = native_long as f64 / px_per_long;
    if !r.is_finite() || r <= 1.0 {
        return 0;
    }
    (r.log2().round().max(0.0) as usize).min(LEVELS)
}

/// The 5-tap binomial filter, and its expanding weights for even and odd samples.
const TAPS: [f32; 5] = [1. / 16., 4. / 16., 6. / 16., 4. / 16., 1. / 16.];
const EVEN: [f32; 3] = [TAPS[0] * 2., TAPS[2] * 2., TAPS[4] * 2.];
const ODD: f32 = TAPS[1] * 2.;

/// `v` reflected into `0..n` about its ends, without repeating them.
#[inline]
fn mirror(v: isize, n: usize) -> usize {
    let n = n.max(1) as isize;
    let v = v.abs();
    (if v >= n { 2 * n - 2 - v } else { v }).clamp(0, n - 1) as usize
}

#[inline]
fn at(p: &[f32], i: usize) -> f32 {
    p.get(i).copied().unwrap_or(0.0)
}

/// Blurred and halved.
fn down(p: &Plane) -> Plane {
    let (w, h) = (p.width.div_ceil(2), p.height.div_ceil(2));
    let mut wide = vec![0.0f32; w * p.height];
    par_rows(&mut wide, w, |y, out| {
        let row = y * p.width;
        for (x, o) in out.iter_mut().enumerate() {
            *o = (0..5).map(|k| at(&p.data, row + mirror(2 * x as isize + k as isize - 2, p.width)) * TAPS[k]).sum();
        }
    });
    let mut data = vec![0.0f32; w * h];
    par_rows(&mut data, w, |y, out| {
        for (x, o) in out.iter_mut().enumerate() {
            *o = (0..5).map(|k| at(&wide, mirror(2 * y as isize + k as isize - 2, p.height) * w + x) * TAPS[k]).sum();
        }
    });
    Plane { width: w, height: h, data }
}

/// Doubled to `w` × `h` and blurred, as a Laplacian pyramid expands a level.
fn up(p: &Plane, w: usize, h: usize) -> Plane {
    let expand = |i: usize, f: &dyn Fn(isize) -> f32| -> f32 {
        let c = (i / 2) as isize;
        if i.is_multiple_of(2) { f(c - 1) * EVEN[0] + f(c) * EVEN[1] + f(c + 1) * EVEN[2] } else { (f(c) + f(c + 1)) * ODD }
    };
    let mut wide = vec![0.0f32; w * p.height];
    par_rows(&mut wide, w, |y, out| {
        let row = y * p.width;
        for (x, o) in out.iter_mut().enumerate() {
            *o = expand(x, &|c| at(&p.data, row + mirror(c, p.width)));
        }
    });
    let mut data = vec![0.0f32; w * h];
    par_rows(&mut data, w, |y, out| {
        for (x, o) in out.iter_mut().enumerate() {
            *o = expand(y, &|c| at(&wide, mirror(c, p.height) * w + x));
        }
    });
    Plane { width: w, height: h, data }
}

/// The compressed, weighted log2 detail of `log_l` (log2 luminance, any offset), whose finest
/// level is the photo's pyramid level `offset` ([`level_offset`]); Texture `amount` changes a
/// pixel's log2 luminance by `strength(amount) · detail`.
pub fn detail(log_l: &Plane, offset: usize) -> Plane {
    detail_from(log_l, offset)
}

/// [`detail`] of each channel's log2 (linear scene RGB `img`), interleaved: Camera Raw's Texture
/// also raises colour contrast at colour edges, as if each channel had its own.
pub fn detail_rgb(img: &lightcraft_raster::Rgb32f, offset: usize) -> lightcraft_raster::Rgb32f {
    detail_rgb_in(img, offset, None)
}

/// [`detail_rgb`] with each pixel first taken through `from` (linear working → camera RGB,
/// [`space`]).
pub fn detail_rgb_in(img: &lightcraft_raster::Rgb32f, offset: usize, from: Option<&[[f32; 3]; 3]>) -> lightcraft_raster::Rgb32f {
    let ch = |k: usize| {
        let v = |p: &[f32; 3]| match from {
            Some(m) => m[k][0] * p[0] + m[k][1] * p[1] + m[k][2] * p[2],
            None => p[k],
        };
        let data = img
            .data
            .iter()
            .map(|p| {
                let x = v(p);
                if x.is_finite() { x.max(1e-6).log2() } else { 0.0 }
            })
            .collect();
        detail(&Plane { width: img.width, height: img.height, data }, offset)
    };
    let (r, (g, b)) = lightcraft_raster::par_join(|| ch(0), || lightcraft_raster::par_join(|| ch(1), || ch(2)));
    let data = (0..img.width * img.height).map(|i| [at(&r.data, i), at(&g.data, i), at(&b.data, i)]).collect();
    lightcraft_raster::Rgb32f { width: img.width, height: img.height, data }
}

fn detail_from(plane: &Plane, level: usize) -> Plane {
    if level >= LEVELS || plane.width < 4 || plane.height < 4 {
        return Plane { width: plane.width, height: plane.height, data: vec![0.0; plane.width * plane.height] };
    }
    let coarse = down(plane);
    let mut out = up(&detail_from(&coarse, level + 1), plane.width, plane.height);
    let blurred = up(&coarse, plane.width, plane.height);
    drop(coarse);
    let (wt, t) = (WEIGHTS[level], THRESHOLDS[level]);
    let w = plane.width;
    par_rows(&mut out.data, w, |y, row| {
        for (x, o) in row.iter_mut().enumerate() {
            let i = y * w + x;
            let band = at(&plane.data, i) - at(&blurred.data, i);
            let q = band / t;
            let v = wt * band / (1.0 + q * q);
            if v.is_finite() {
                *o += v;
            }
        }
    });
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The gain Texture `amount` gives a sine grating of `f` cycles per pixel and `amplitude`
    /// EV, at full resolution.
    fn gain_at(f: f32, amplitude: f32, amount: f32) -> f32 {
        let (w, h) = (1024usize, 64usize);
        let data: Vec<f32> = (0..w * h).map(|i| amplitude * (std::f32::consts::TAU * f * (i % w) as f32).sin()).collect();
        let p = Plane { width: w, height: h, data };
        let d = detail(&p, 0);
        let s = strength(amount);
        let amp = |v: &dyn Fn(usize) -> f32| {
            let (mut a, mut b) = (0.0f32, 0.0f32);
            for x in 128..896 {
                let t = std::f32::consts::TAU * f * x as f32;
                a += v(32 * w + x) * t.sin();
                b += v(32 * w + x) * t.cos();
            }
            a.hypot(b)
        };
        amp(&|i| p.data[i] + s * d.data[i]) / amp(&|i| p.data[i])
    }

    /// Camera Raw at Texture +100 (our oracle, `oracle/make_fx.py fx2`; RAWmakase's 18.7
    /// measurements agree): faint and mid detail lifted most, strong contrast and coarse scales
    /// hardly; negative Texture smooths the same band.
    #[test]
    fn texture_follows_camera_raw_over_scale_and_contrast() {
        for (f, amplitude, camera_raw) in
            [(1.0 / 89.0, 0.4, 1.32), (1.0 / 34.0, 0.4, 1.54), (1.0 / 8.0, 0.4, 1.63), (0.03, 0.1, 1.78), (0.03, 2.0, 1.05)]
        {
            let ours = gain_at(f, amplitude, 1.0);
            assert!((ours - camera_raw).abs() < 0.15, "{f} {amplitude}: {ours} against {camera_raw}");
        }
        assert!(gain_at(0.03, 0.4, -1.0) < 0.75);
        assert!((gain_at(0.03, 0.4, 0.0) - 1.0).abs() < 1e-5);
    }

    #[test]
    fn offsets_and_strength_are_safe() {
        assert_eq!(level_offset(1024.0, 0.0), 0);
        assert_eq!(level_offset(1024.0, 1024.0), 0);
        assert_eq!(level_offset(2560.0, 6000.0), 1);
        assert_eq!(level_offset(0.0, 6000.0), 0);
        assert_eq!(level_offset(1.0, f32::INFINITY), 0);
        assert_eq!(strength(f32::NAN), 0.0);
        assert!((strength(2.0) - 0.852).abs() < 1e-6);
        // tiny and empty planes give an all-zero detail, never a panic
        for (w, h) in [(0, 0), (1, 1), (3, 9), (5, 4)] {
            let p = Plane { width: w, height: h, data: vec![0.5; w * h] };
            assert_eq!(detail(&p, 0).data.len(), w * h);
        }
    }
}
