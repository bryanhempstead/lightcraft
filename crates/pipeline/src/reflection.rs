//! Remove Reflections, rendered: the photo's separated layers (made by the engine with a reflection-separation
//! model and handed over like Generative Remove's fills, [`crate::spots::set_gen_loader`]) are resampled from
//! normalized coordinates at the render's size and white-balanced like the photo, then
//!
//! - **Remove**: the reflection layer (`<key>-r`: the photo minus the scene behind the glass, scene-linear) is
//!   subtracted, by Amount;
//! - **Keep**: the photo moves towards the reflection alone (`<key>-k`), by Amount.
//!
//! Applied right after white balance, before spots, at any resolution (previews and exports match). Without the
//! layers (no model, not made yet) the photo renders as it is.

use std::sync::Arc;

use lightcraft_develop::DevelopSettings;
use lightcraft_geom::Point;
use lightcraft_raster::Rgb32f;

use crate::geometry::Frame;
use crate::spots::GenFill;

/// The layer the settings use, when it is there.
pub fn layer(s: &DevelopSettings) -> Option<Arc<GenFill>> {
    let r = &s.reflection;
    if !r.active() {
        return None;
    }
    let name = format!("{}-{}", r.key, if r.keep { 'k' } else { 'r' });
    crate::spots::load_named(&name).filter(|f| f.image.width > 0 && f.image.height > 0 && f.image.data.len() == f.image.width * f.image.height)
}

/// Part of the scene-linear stage's cache key: the settings and whether the layer is there.
pub fn key(s: &DevelopSettings) -> String {
    let r = &s.reflection;
    if !r.active() {
        return String::new();
    }
    format!("{} {} {} {}", r.keep, r.amount.to_bits(), r.key, layer(s).is_some())
}

/// Apply the reflection settings to `img` (the output frame, white-balanced with `wb`).
pub fn apply(img: &mut Rgb32f, s: &DevelopSettings, frame: &Frame, wb: Option<[[f32; 3]; 3]>) {
    let Some(l) = layer(s) else { return };
    let a = (s.reflection.amount / 100.0).clamp(0.0, 1.0) as f32;
    let keep = s.reflection.keep;
    let (w, h) = (img.width, img.height);
    let to_norm = frame.out_to_norm(w, h);
    let (lw, lh) = (l.image.width as f64, l.image.height as f64);
    let (rw, rh) = ((l.rect[2] - l.rect[0]).max(1e-9), (l.rect[3] - l.rect[1]).max(1e-9));
    crate::for_rows(&mut img.data, w, |y, row| {
        for (x, px) in row.iter_mut().enumerate() {
            let n = to_norm.apply(Point::new(x as f64 + 0.5, y as f64 + 0.5));
            let (u, v) = ((n.x - l.rect[0]) / rw * lw, (n.y - l.rect[1]) / rh * lh);
            let c = l.image.sample_bilinear(u as f32, v as f32);
            let c = match &wb {
                Some(m) => std::array::from_fn(|r| m[r][0] * c[0] + m[r][1] * c[1] + m[r][2] * c[2]),
                None => c,
            };
            for k in 0..3 {
                let v = if keep { px[k] + (c[k] - px[k]) * a } else { px[k] - c[k] * a };
                px[k] = if v.is_finite() { v.max(0.0) } else { px[k] };
            }
        }
    });
}

#[cfg(test)]
mod tests {
    use super::*;
    use lightcraft_develop::Reflection;

    #[test]
    fn the_layer_is_subtracted_or_kept_at_any_size() {
        let layer = Rgb32f::from_fn(10, 10, |x, _| if x < 5 { [0.2, 0.1, 0.0] } else { [0.0; 3] });
        crate::spots::test_fill("000000000000ref0-r", GenFill { geometry: 0, rect: [0.0, 0.0, 1.0, 1.0], image: layer.clone() });
        crate::spots::test_fill("000000000000ref0-k", GenFill { geometry: 0, rect: [0.0, 0.0, 1.0, 1.0], image: layer });
        let mut s = DevelopSettings {
            reflection: Reflection { enabled: true, keep: false, amount: 50.0, key: "000000000000ref0".into() },
            ..Default::default()
        };
        for w in [40usize, 200] {
            let mut img = Rgb32f::from_fn(w, w, |_, _| [0.5; 3]);
            let frame = Frame::new(w, w, &s, true);
            apply(&mut img, &s, &frame, None);
            let left = img.get(w / 10, w / 2);
            let right = img.get(w - w / 10, w / 2);
            assert!((left[0] - 0.4).abs() < 1e-3 && (left[1] - 0.45).abs() < 1e-3 && (left[2] - 0.5).abs() < 1e-3, "{w}: {left:?}");
            assert_eq!(right, [0.5; 3]);
        }
        s.reflection.keep = true;
        s.reflection.amount = 100.0;
        let mut img = Rgb32f::from_fn(40, 40, |_, _| [0.5; 3]);
        let frame = Frame::new(40, 40, &s, true);
        apply(&mut img, &s, &frame, None);
        assert!((img.get(4, 20)[0] - 0.2).abs() < 1e-3 && img.get(36, 20) == [0.0; 3]);
        // off, or not made: nothing changes
        for r in [Reflection { enabled: false, ..s.reflection.clone() }, Reflection { key: "00000000000000ff".into(), ..s.reflection.clone() }] {
            let t = DevelopSettings { reflection: r, ..Default::default() };
            let mut img = Rgb32f::from_fn(40, 40, |_, _| [0.5; 3]);
            apply(&mut img, &t, &frame, None);
            assert_eq!(img.get(4, 20), [0.5; 3]);
            assert!(key(&t).is_empty() || key(&t).ends_with("false"));
        }
    }
}
