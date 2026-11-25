//! The crop model.
//!
//! The image (w × h pixels after orientation) is rotated by `angle` about its centre. The crop is an
//! axis-aligned rectangle in that rotated frame, stored *normalized* to the image size
//! (`x / w`, `y / h`), so it is independent of preview resolution. The crop must stay inside the
//! rotated image (Lightroom's "Constrain to image").

use serde::{Deserialize, Serialize};

use crate::{Affine, Point, Rect, Vec2};

#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
pub struct CropGeometry {
    /// Normalized crop rectangle in the rotated frame (0..1 when angle = 0).
    pub rect: Rect,
    /// Straighten angle in degrees, clockwise positive, −45..45.
    pub angle: f64,
}

impl Default for CropGeometry {
    fn default() -> Self {
        Self { rect: Rect::UNIT, angle: 0.0 }
    }
}

impl CropGeometry {
    pub fn is_identity(&self) -> bool {
        self.angle == 0.0 && self.rect == Rect::UNIT
    }

    /// Crop rectangle in pixels of the rotated frame for an image of `w × h`.
    pub fn rect_px(&self, w: f64, h: f64) -> Rect {
        self.rect.scale(w, h)
    }

    /// Transform from *output* (cropped) pixel coordinates to *source* (oriented image) pixel
    /// coordinates, for an output of `out_w × out_h` pixels rendered from a `w × h` source.
    pub fn output_to_source(&self, w: f64, h: f64, out_w: f64, out_h: f64) -> Affine {
        let r = self.rect_px(w, h);
        // output -> crop rect in rotated frame
        let to_rot = Affine::translate(Vec2::new(r.x0, r.y0)) * Affine::scale(r.width() / out_w, r.height() / out_h);
        // rotated frame -> source: inverse rotation about the image centre
        let c = Point::new(w / 2.0, h / 2.0);
        Affine::rotate_about(-self.angle.to_radians(), c) * to_rot
    }

    /// Whether every crop corner lies within the rotated image.
    pub fn is_within_image(&self, w: f64, h: f64) -> bool {
        let r = self.rect_px(w, h);
        let back = Affine::rotate_about(-self.angle.to_radians(), Point::new(w / 2.0, h / 2.0));
        r.corners().iter().all(|p| {
            let q = back.apply(*p);
            q.x >= -1e-6 * w && q.y >= -1e-6 * h && q.x <= w * (1.0 + 1e-6) && q.y <= h * (1.0 + 1e-6)
        })
    }

    /// Shrink the crop (about its centre, keeping aspect) and then shift it until it fits inside the
    /// rotated image.
    pub fn constrained(&self, w: f64, h: f64) -> CropGeometry {
        if self.is_within_image(w, h) {
            return *self;
        }
        let r = self.rect_px(w, h);
        // Try moving towards the image centre first.
        let c = Point::new(w / 2.0, h / 2.0);
        let mut best = *self;
        for i in 1..=32 {
            let t = i as f64 / 32.0;
            let nc = r.center().lerp(c, t);
            let cand = CropGeometry { rect: Rect::from_center(nc, r.width(), r.height()).scale(1.0 / w, 1.0 / h), angle: self.angle };
            if cand.is_within_image(w, h) {
                return cand;
            }
            best = cand;
        }
        // Then shrink around the centre.
        let s = max_inscribed_scale(w, h, self.angle, r.width(), r.height());
        let nr = Rect::from_center(c, r.width() * s, r.height() * s);
        let _ = best;
        CropGeometry { rect: nr.scale(1.0 / w, 1.0 / h), angle: self.angle }
    }

    /// The crop moved by `(dx, dy)` (normalized) along that line only, as far as it can go while it
    /// stays inside the rotated image (a console's CropPositionX / Y: the other axis never moves).
    /// A crop that is already outside the image moves the whole way.
    pub fn shifted(&self, dx: f64, dy: f64, w: f64, h: f64) -> CropGeometry {
        if !(dx.is_finite() && dy.is_finite()) {
            return *self;
        }
        let at = |t: f64| CropGeometry { rect: self.rect.translate(Vec2::new(dx * t, dy * t)), angle: self.angle };
        let full = at(1.0);
        if full.is_within_image(w, h) || !self.is_within_image(w, h) {
            return full;
        }
        // the translations that keep a rectangle inside a convex shape are convex: bisect the last one
        let (mut lo, mut hi) = (0.0, 1.0);
        for _ in 0..40 {
            let mid = (lo + hi) / 2.0;
            if at(mid).is_within_image(w, h) {
                lo = mid;
            } else {
                hi = mid;
            }
        }
        at(lo)
    }

    /// The crop scaled by `k` about its centre, keeping its shape (a console's CropScale): never
    /// bigger than the largest crop of that shape the rotated image holds; one that no longer fits
    /// around its centre slides towards the image centre, only as far as it has to.
    pub fn scaled(&self, k: f64, w: f64, h: f64) -> CropGeometry {
        let r = self.rect_px(w, h);
        if !(k.is_finite() && k > 0.0) || r.width() <= 0.0 || r.height() <= 0.0 {
            return *self;
        }
        let k = k.min(max_inscribed_scale(w, h, self.angle, r.width(), r.height()));
        let (cw, ch) = (r.width() * k, r.height() * k);
        let at = |c: Point| CropGeometry { rect: Rect::from_center(c, cw, ch).scale(1.0 / w, 1.0 / h), angle: self.angle };
        let here = at(r.center());
        if here.is_within_image(w, h) {
            return here;
        }
        let mid = Point::new(w / 2.0, h / 2.0);
        let (mut lo, mut hi) = (0.0, 1.0);
        for _ in 0..40 {
            let t = (lo + hi) / 2.0;
            if at(r.center().lerp(mid, t)).is_within_image(w, h) {
                hi = t;
            } else {
                lo = t;
            }
        }
        at(r.center().lerp(mid, hi))
    }
}

/// How a crop was framed, remembered so that changing its aspect ratio and coming back gives the same
/// framing: `punch` is how punched in it was (1 = the largest crop of its shape the image holds,
/// 0.5 = half that size) and `centre` the framed subject (normalized, in the rotated frame). `result`
/// is the crop this framing produced; the memory only counts while the crop still equals it, so a
/// hand edit, a straighten or an undo starts a fresh framing from what is on screen.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Framing {
    pub result: CropGeometry,
    pub punch: f64,
    pub centre: Point,
    /// The crop's width (normalized) before the image forced it smaller, so turning the angle back
    /// grows it back instead of leaving it shrunk.
    pub width: f64,
}

impl CropGeometry {
    /// How punched in this crop is on a `w × h` image: its size relative to the largest crop of the
    /// same shape (and angle) that fits, `0 < punch ≤ 1`. A full-frame crop is 1.
    pub fn punch(&self, w: f64, h: f64) -> f64 {
        let r = self.rect_px(w, h);
        let full = max_inscribed_scale(w, h, self.angle, r.width(), r.height());
        let p = 1.0 / full;
        if p.is_finite() && p > 0.0 { p.min(1.0) } else { 1.0 }
    }

    /// The framing of this crop to carry into a new shape: the remembered one while it still matches
    /// (`memory`), else what the crop is right now.
    pub fn framing(&self, memory: Option<Framing>, w: f64, h: f64) -> (f64, Point) {
        match memory {
            Some(f) if f.result == *self && f.punch.is_finite() && f.centre.x.is_finite() && f.centre.y.is_finite() => (f.punch, f.centre),
            _ => (self.punch(w, h), self.rect.center()),
        }
    }

    /// Turned to `angle`: the same shape, centred on the same subject, as big as it was before the
    /// image forced it smaller (`framing` remembers that), shrunk only as much as the rotated image
    /// needs and slid only as far as it needs. Never non-finite; bad input leaves the crop alone.
    pub fn turned(&self, angle: f64, framing: Option<Framing>, w: f64, h: f64) -> (CropGeometry, Framing) {
        let angle = if angle.is_finite() { angle.clamp(-45.0, 45.0) } else { self.angle };
        let r = self.rect_px(w, h);
        let keep = (*self, Framing { result: *self, punch: self.punch(w, h), centre: self.rect.center(), width: self.rect.width() });
        if !(r.width() > 0.0 && r.height() > 0.0 && r.width().is_finite() && r.height().is_finite()) {
            return keep;
        }
        let aspect = r.width() / r.height();
        let (centre, ideal) = match framing {
            Some(f) if f.result == *self && f.width.is_finite() && f.width > 0.0 && f.centre.x.is_finite() && f.centre.y.is_finite() => (f.centre, f.width),
            _ => (self.rect.center(), self.rect.width()),
        };
        let biggest = crop_fit_angle(w, h, angle, Some(aspect)).rect.width();
        let punch = if biggest > 0.0 { (ideal / biggest).min(1.0) } else { 1.0 };
        let next = CropGeometry::with_punch(angle, aspect, punch, centre, w, h);
        (next, Framing { result: next, punch: next.punch(w, h), centre, width: ideal })
    }

    /// A crop of shape `aspect` (width / height in pixels) at `angle`, as punched in as `punch` says
    /// (a fraction of the largest crop of that shape), centred on `centre` (normalized) or, where
    /// that would leave the rotated image, as near to it as the image allows. Never bigger than the
    /// image holds, never non-finite: bad input gives the largest crop of the shape.
    pub fn with_punch(angle: f64, aspect: f64, punch: f64, centre: Point, w: f64, h: f64) -> CropGeometry {
        let biggest = crop_fit_angle(w, h, angle, Some(aspect));
        if !(punch.is_finite() && punch > 0.0 && centre.x.is_finite() && centre.y.is_finite()) || punch >= 1.0 {
            return biggest;
        }
        let big = biggest.rect_px(w, h);
        let (cw, ch) = (big.width() * punch, big.height() * punch);
        let want = Point::new(centre.x * w, centre.y * h);
        let at = |c: Point| CropGeometry { rect: Rect::from_center(c, cw, ch).scale(1.0 / w, 1.0 / h), angle };
        let here = at(want);
        if here.is_within_image(w, h) {
            return here;
        }
        // slide towards the image centre only as far as it takes (the centred crop always fits)
        let mid = Point::new(w / 2.0, h / 2.0);
        let (mut lo, mut hi) = (0.0, 1.0);
        for _ in 0..40 {
            let t = (lo + hi) / 2.0;
            if at(want.lerp(mid, t)).is_within_image(w, h) {
                hi = t;
            } else {
                lo = t;
            }
        }
        at(want.lerp(mid, hi))
    }
}

/// Largest scale `s` such that a centred `cw·s × ch·s` rectangle fits inside a `w × h` image rotated
/// by `angle_deg` about its centre.
pub fn max_inscribed_scale(w: f64, h: f64, angle_deg: f64, cw: f64, ch: f64) -> f64 {
    let a = angle_deg.to_radians().abs();
    let (s, c) = a.sin_cos();
    // Half extents of the crop must satisfy, in the image frame (rotated back):
    // |x| c + |y| s <= w/2 and |x| s + |y| c <= h/2 for the crop corner (x, y) = (cw/2, ch/2)·scale.
    let (hx, hy) = (cw / 2.0, ch / 2.0);
    let s1 = (w / 2.0) / (hx * c + hy * s);
    let s2 = (h / 2.0) / (hx * s + hy * c);
    s1.min(s2)
}

/// The biggest crop of the image's own aspect (or `aspect` if given) that fits at `angle_deg`.
pub fn crop_fit_angle(w: f64, h: f64, angle_deg: f64, aspect: Option<f64>) -> CropGeometry {
    let a = aspect.unwrap_or(w / h);
    let base = Rect::new(0.0, 0.0, w, h).fit_aspect(a);
    let s = max_inscribed_scale(w, h, angle_deg, base.width(), base.height()).min(1.0);
    let r = Rect::from_center(Point::new(w / 2.0, h / 2.0), base.width() * s, base.height() * s);
    CropGeometry { rect: r.scale(1.0 / w, 1.0 / h), angle: angle_deg }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn identity_maps_pixels() {
        let c = CropGeometry::default();
        let t = c.output_to_source(400.0, 300.0, 400.0, 300.0);
        let p = t.apply(Point::new(10.0, 20.0));
        assert!((p.x - 10.0).abs() < 1e-9 && (p.y - 20.0).abs() < 1e-9);
    }

    #[test]
    fn rotated_fit_is_inside() {
        for ang in [-45.0, -10.0, 0.0, 3.5, 30.0, 45.0] {
            let c = crop_fit_angle(600.0, 400.0, ang, None);
            assert!(c.is_within_image(600.0, 400.0), "{ang}");
            // and maximal: 1% bigger does not fit (except angle 0 where it equals the image)
            let big = CropGeometry { rect: Rect::from_center(c.rect.center(), c.rect.width() * 1.01, c.rect.height() * 1.01), angle: ang };
            assert!(!big.is_within_image(600.0, 400.0), "{ang}");
        }
    }

    #[test]
    fn constrain_pulls_crop_inside() {
        let c = CropGeometry { rect: Rect::new(0.5, 0.5, 1.4, 1.2), angle: 12.0 };
        let k = c.constrained(300.0, 200.0);
        assert!(k.is_within_image(300.0, 200.0));
    }

    /// A console's zoom-out (`scaled` by 1.005 a detent) of a straightened crop sitting in a corner
    /// slides it towards the photo's centre in small steps, never away from it, up to the limit.
    #[test]
    fn scaled_out_slides_towards_the_centre_in_small_steps() {
        let (w, h) = (600.0, 400.0);
        let mut c = CropGeometry { rect: Rect::new(0.62, 0.55, 0.92, 0.8), angle: 4.0 };
        assert!(c.is_within_image(w, h));
        let far = |c: &CropGeometry| (c.rect.center().x - 0.5).hypot(c.rect.center().y - 0.5);
        for i in 0..400 {
            let next = c.scaled(1.005, w, h).constrained(w, h);
            assert!(next.is_within_image(w, h), "{i}");
            assert!(far(&next) <= far(&c) + 1e-9, "{i}: {c:?} → {next:?}");
            assert!((next.rect.center().x - c.rect.center().x).hypot(next.rect.center().y - c.rect.center().y) < 0.01, "{i}: jump");
            c = next;
        }
        assert_eq!(c.scaled(1.005, w, h), c, "at the limit");
    }

    /// The old console nudge slid a crop that left a straightened photo back with `constrained`,
    /// which pulls it towards the image centre on both axes: an x move also moved y.
    #[test]
    fn shifted_moves_along_one_axis_only() {
        let (w, h) = (600.0, 400.0);
        let c = CropGeometry { rect: Rect::new(0.4, 0.2, 0.7, 0.5), angle: 6.0 };
        assert!(c.is_within_image(w, h));
        let pushed = CropGeometry { rect: c.rect.translate(Vec2::new(0.35, 0.0)), angle: 6.0 };
        assert!(!pushed.is_within_image(w, h));
        assert!((pushed.constrained(w, h).rect.y0 - c.rect.y0).abs() > 1e-3, "the old way drifts in y");
        let s = c.shifted(0.35, 0.0, w, h);
        assert!(s.is_within_image(w, h));
        assert_eq!((s.rect.y0, s.rect.y1), (c.rect.y0, c.rect.y1));
        assert!((s.rect.width() - c.rect.width()).abs() < 1e-12);
        assert!(s.rect.x0 > c.rect.x0 + 0.01, "{s:?}");
        let v = c.shifted(0.0, -0.4, w, h);
        assert_eq!((v.rect.x0, v.rect.x1), (c.rect.x0, c.rect.x1));
        assert!(v.is_within_image(w, h) && v.rect.y0 < c.rect.y0);
        // nonsense: unchanged
        assert_eq!(c.shifted(f64::NAN, 0.0, w, h), c);
    }

    #[test]
    fn scaled_keeps_centre_and_shape() {
        let (w, h) = (600.0, 400.0);
        let c = CropGeometry { rect: Rect::new(0.4, 0.3, 0.6, 0.5), angle: 5.0 };
        let a = c.rect_px(w, h).aspect();
        let z = c.scaled(0.8, w, h);
        assert!((z.rect.center().x - c.rect.center().x).abs() < 1e-12 && (z.rect.center().y - c.rect.center().y).abs() < 1e-12);
        let mut g = c;
        for _ in 0..100 {
            g = g.scaled(1.1, w, h);
            assert!(g.is_within_image(w, h), "{g:?}");
            assert!((g.rect_px(w, h).aspect() - a).abs() < 1e-9);
        }
        assert_eq!(c.scaled(f64::NAN, w, h), c);
        assert_eq!(c.scaled(-1.0, w, h), c);
    }

    /// Punched in to half on a 3:2 photo, 4:5 keeps the centre and the half-size: it doesn't start
    /// over at the full frame.
    #[test]
    fn a_new_ratio_keeps_the_punch_in_and_the_centre() {
        let (w, h) = (3000.0, 2000.0);
        let c = CropGeometry { rect: Rect::from_center(Point::new(0.3, 0.45), 0.5, 0.5), angle: 0.0 };
        assert!((c.punch(w, h) - 0.5).abs() < 1e-9);
        let (p, centre) = c.framing(None, w, h);
        let g = CropGeometry::with_punch(0.0, 4.0 / 5.0, p, centre, w, h);
        let r = g.rect_px(w, h);
        assert!((r.aspect() - 0.8).abs() < 1e-9);
        assert!((g.rect.center().x - 0.3).abs() < 1e-9 && (g.rect.center().y - 0.45).abs() < 1e-9, "{g:?}");
        assert!((g.punch(w, h) - 0.5).abs() < 1e-9, "{}", g.punch(w, h));
        assert!(g.is_within_image(w, h));
        assert!(r.height() < h * 0.6, "not the full frame: {r:?}");
    }

    /// 4:5 → 4:3 → 4:5 comes back to the very same crop, even when the middle shape had to slide
    /// off the subject to fit.
    #[test]
    fn cycling_ratios_returns_to_the_same_crop() {
        let (w, h) = (3000.0, 2000.0);
        let c = CropGeometry { rect: Rect::from_center(Point::new(0.92, 0.5), 0.12, 0.3), angle: 0.0 };
        assert!(c.is_within_image(w, h));
        let mut mem: Option<Framing> = None;
        let mut apply = |cur: CropGeometry, ratio: f64| {
            let (p, centre) = cur.framing(mem, w, h);
            let g = CropGeometry::with_punch(cur.angle, ratio, p, centre, w, h);
            mem = Some(Framing { result: g, punch: p, centre, width: g.rect.width() });
            g
        };
        let a = apply(c, 4.0 / 5.0);
        let b = apply(a, 4.0 / 3.0);
        assert!(b.is_within_image(w, h));
        let a2 = apply(b, 4.0 / 5.0);
        assert_eq!(a, a2, "back to the same 4:5");
        // forgetting the memory (a hand edit) drifts the centre: that's what it prevents
        let drift = CropGeometry::with_punch(0.0, 4.0 / 5.0, b.punch(w, h), b.rect.center(), w, h);
        assert_ne!(a, drift);
        // a straightened crop stays inside the image at every ratio, never full frame when punched in
        let t = CropGeometry { rect: Rect::from_center(Point::new(0.5, 0.5), 0.3, 0.3), angle: 6.0 };
        let (p, centre) = t.framing(None, w, h);
        for ratio in [1.0, 4.0 / 5.0, 16.0 / 9.0, 3.0 / 2.0] {
            let g = CropGeometry::with_punch(6.0, ratio, p, centre, w, h);
            assert!(g.is_within_image(w, h), "{ratio}: {g:?}");
            assert!((g.punch(w, h) - p).abs() < 1e-6, "{ratio}");
            assert_eq!(g.angle, 6.0);
        }
        // nonsense gives the biggest crop, never a panic
        assert!(CropGeometry::with_punch(0.0, 1.0, f64::NAN, Point::new(f64::NAN, 0.0), w, h).is_within_image(w, h));
    }

    /// Bryan, 2026-11-24: "if i crop and dont punch in it wont let me move the crop around to the
    /// cut off areas": a 4:5 crop at its largest size on a 3:2 photo slides to both side edges, by a
    /// drag, a nudge or a shift, on common camera sizes (float noise in the full-height edge must not
    /// pin it).
    #[test]
    fn a_largest_crop_slides_anywhere_it_still_fits() {
        use crate::{CropHandle, Vec2, drag_crop};
        for (w, h) in [(3000.0, 2000.0), (6000.0, 4000.0), (5472.0, 3648.0), (6240.0, 4160.0), (4032.0, 3024.0), (3000.0, 1999.0)] {
            for ratio in [4.0 / 5.0, 1.0, 16.0 / 9.0, 3.0 / 2.0, 4.0 / 3.0] {
                let c = crop_fit_angle(w, h, 0.0, Some(ratio));
                let spare = 1.0 - c.rect.width();
                let tag = format!("{w}x{h} {ratio}");
                if spare < 1e-3 {
                    continue;
                }
                // drag far left / right
                let l = drag_crop(c, CropHandle::Move, Point::new(0.5, 0.5), Point::new(-2.0, 0.5), w, h, None);
                let r = drag_crop(c, CropHandle::Move, Point::new(0.5, 0.5), Point::new(3.0, 0.5), w, h, None);
                assert!(l.rect.x0.abs() < 1e-9 && (l.rect.width() - c.rect.width()).abs() < 1e-9, "{tag}: left {l:?}");
                assert!((r.rect.x1 - 1.0).abs() < 1e-9 && (r.rect.width() - c.rect.width()).abs() < 1e-9, "{tag}: right {r:?}");
                // nudge (a console's CropPositionX) by small steps reaches both edges too
                for (dir, edge) in [(-0.01, 0.0), (0.01, 1.0)] {
                    let mut g = c;
                    for _ in 0..200 {
                        g = g.shifted(dir, 0.0, w, h).constrained(w, h);
                    }
                    let at = if dir < 0.0 { g.rect.x0 } else { g.rect.x1 };
                    assert!((at - edge).abs() < 1e-6, "{tag}: nudge {dir} stopped at {at}");
                    assert_eq!((g.rect.y0, g.rect.y1), (c.rect.y0, c.rect.y1), "{tag}: one axis only");
                }
                // a straightened one slides within the rotated bounds
                let t = crop_fit_angle(w, h, 3.0, Some(ratio));
                let m = t.shifted(0.5, 0.0, w, h);
                assert!(m.is_within_image(w, h), "{tag}");
                let _ = Vec2::new(0.0, 0.0);
            }
        }
    }

    /// Turning the angle keeps the centre and the punch-in, shrinks only when the rotated photo
    /// demands it, in even steps, and grows back when the angle returns.
    #[test]
    fn turning_the_angle_keeps_the_centre_and_punch_in() {
        let (w, h) = (3000.0, 2000.0);
        // punched in with room: untouched by a few degrees
        let c = CropGeometry { rect: Rect::from_center(Point::new(0.4, 0.55), 0.2, 0.2), angle: 0.0 };
        let (t, _) = c.turned(5.0, None, w, h);
        assert_eq!(t.angle, 5.0);
        assert!([(t.rect.x0, c.rect.x0), (t.rect.y0, c.rect.y0), (t.rect.x1, c.rect.x1), (t.rect.y1, c.rect.y1)].iter().all(|(a, b)| (a - b).abs() < 1e-12), "{t:?}");
        // full frame: shrinks in even steps about the centre, and comes back
        let mut g = crop_fit_angle(w, h, 0.0, None);
        let mut f = None;
        let mut widths = vec![g.rect.width()];
        for i in 1..=20 {
            let (n, nf) = g.turned(i as f64 * 0.15, f, w, h);
            assert!(n.is_within_image(w, h));
            assert!((n.rect.center().x - 0.5).abs() < 1e-9 && (n.rect.center().y - 0.5).abs() < 1e-9);
            widths.push(n.rect.width());
            (g, f) = (n, Some(nf));
        }
        let d: Vec<f64> = widths.windows(2).map(|p| p[1] - p[0]).collect();
        assert!(d.iter().all(|x| *x < 0.0) && d.iter().cloned().fold(f64::MIN, f64::max) - d.iter().cloned().fold(f64::MAX, f64::min) < 0.005, "{d:?}");
        for i in (0..20).rev() {
            let (n, nf) = g.turned(i as f64 * 0.15, f, w, h);
            (g, f) = (n, Some(nf));
        }
        assert!((g.rect.width() - 1.0).abs() < 1e-9 && g.angle == 0.0, "{g:?}");
        // a crop pinned in a corner slides only as far as it must
        let k = CropGeometry { rect: Rect::new(0.8, 0.7, 1.0, 1.0), angle: 0.0 };
        let (t, _) = k.turned(8.0, None, w, h);
        assert!(t.is_within_image(w, h));
        assert!((t.rect.width() - k.rect.width()).abs() < 1e-9 || t.rect.width() < k.rect.width());
        assert_eq!(k.turned(f64::NAN, None, w, h).0.angle, 0.0);
    }

    #[test]
    fn half_crop_maps_to_centre_quadrant() {
        let c = CropGeometry { rect: Rect::new(0.25, 0.25, 0.75, 0.75), angle: 0.0 };
        let t = c.output_to_source(100.0, 100.0, 50.0, 50.0);
        let p = t.apply(Point::new(0.0, 0.0));
        assert!((p.x - 25.0).abs() < 1e-9 && (p.y - 25.0).abs() < 1e-9);
    }
}
