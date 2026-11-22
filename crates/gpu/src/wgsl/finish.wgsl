// The per-pixel stage: a straight port of `lightcraft_pipeline::finish` (keep in step with it).
// Bindings: img (rgb, pre-exposure), log_l, base, clar, tex, dark, masks (NMASK planes, then the
// blurred chromaticity when HAS_CHROMA), aux
// (tone LUT | chroma curve | sRGB LUT | curve LUTs | mask terms), out (packed RGBA8).

fn tone_apply(y: f32) -> f32 {
    if (y <= 0.0) {
        return 0.0;
    }
    let ev = log2(y / GREY);
    let f = clamp((ev - TONE_MIN_EV) / (TONE_MAX_EV - TONE_MIN_EV), 0.0, 1.0) * f32(TONE_N - 1u);
    let i = min(u32(f), TONE_N - 2u);
    let t = f - f32(i);
    let v = aux[i] + (aux[i + 1u] - aux[i]) * t;
    if (ev < TONE_MIN_EV) {
        return v * (y / (GREY * TONE_MIN_GAIN));
    }
    return v;
}

// The camera chroma curve follows the tone LUT in `aux` (`ToneMap::chroma_scale`).
fn chroma_scale(o: f32) -> f32 {
    let f = clamp(o, 0.0, 1.0) * f32(CHROMA_N - 1u);
    let i = min(u32(f), CHROMA_N - 2u);
    let t = f - f32(i);
    let a = aux[TONE_N + i];
    let b = aux[TONE_N + i + 1u];
    if (a == b) {
        return a;
    }
    return a + (b - a) * t;
}

// ---- Adobe camera base (`lightcraft_pipeline::adobe`, Bryan's fork)

fn ad_lin_to_srgb(v: f32) -> f32 {
    if (v <= 0.0031308) {
        return v * 12.92;
    }
    return 1.055 * pow(v, 1.0 / 2.4) - 0.055;
}

fn ad_srgb_to_lin(v: f32) -> f32 {
    if (v <= 0.04045) {
        return v / 12.92;
    }
    return pow((v + 0.055) / 1.055, 2.4);
}

fn ad_mat(f: u32) -> array<vec3<f32>, 3> {
    return array<vec3<f32>, 3>(
        vec3<f32>(pf(f), pf(f + 1u), pf(f + 2u)),
        vec3<f32>(pf(f + 3u), pf(f + 4u), pf(f + 5u)),
        vec3<f32>(pf(f + 6u), pf(f + 7u), pf(f + 8u)),
    );
}

// `adobe::ramp`
fn ad_ramp(x: f32, black: f32) -> f32 {
    if (black <= 0.0) {
        return x;
    }
    let slope = 1.0 / (1.0 - black);
    let radius = min(0.5 * black, 1.0 / 16.0 / slope);
    if (x <= black - radius) {
        return 0.0;
    }
    if (x >= black + radius) {
        return (x - black) * slope;
    }
    let y = x - (black - radius);
    return slope / (4.0 * radius) * y * y;
}

fn ad_entry(t: u32, v: u32, h: u32, s: u32) -> vec3<f32> {
    let b = F_AD_T + t * 6u;
    let i = pu(b) + ((v * pu(b + 1u) + h) * pu(b + 2u) + s) * 3u;
    return vec3<f32>(aux[i], aux[i + 1u], aux[i + 2u]);
}

// `HsvTable::lookup` axis: (i0, i1, t)
fn ad_axis(x: f32, n: u32) -> vec3<f32> {
    let f = clamp(x, 0.0, 1.0) * f32(n - 1u);
    let lo = select(0u, n - 2u, n >= 2u);
    let i0 = min(u32(f), lo);
    let i1 = min(i0 + 1u, n - 1u);
    return vec3<f32>(f32(i0), f32(i1), clamp(f - f32(i0), 0.0, 1.0));
}

fn ad_lookup(t: u32, h: f32, s: f32, v: f32) -> vec3<f32> {
    let b = F_AD_T + t * 6u;
    let hd = pu(b + 1u);
    let sd = pu(b + 2u);
    let vd = pu(b + 3u);
    let hf = clamp(rem_euclid(h, 360.0) / 360.0 * f32(hd), 0.0, f32(hd));
    let h0 = min(u32(hf), hd - 1u);
    let h1 = (h0 + 1u) % hd;
    let th = clamp(hf - f32(h0), 0.0, 1.0);
    let sa = ad_axis(s, sd);
    let s0 = u32(sa.x);
    let s1 = u32(sa.y);
    var v0 = 0u;
    var v1 = 0u;
    var tv = 0.0;
    if (vd > 1u) {
        let va = ad_axis(v, vd);
        v0 = u32(va.x);
        v1 = u32(va.y);
        tv = va.z;
    }
    let lo0 = mix(ad_entry(t, v0, h0, s0), ad_entry(t, v0, h0, s1), sa.z);
    let hi0 = mix(ad_entry(t, v0, h1, s0), ad_entry(t, v0, h1, s1), sa.z);
    let p0 = mix(lo0, hi0, th);
    if (v0 == v1) {
        return p0;
    }
    let lo1 = mix(ad_entry(t, v1, h0, s0), ad_entry(t, v1, h0, s1), sa.z);
    let hi1 = mix(ad_entry(t, v1, h1, s0), ad_entry(t, v1, h1, s1), sa.z);
    return mix(p0, mix(lo1, hi1, th), tv);
}

fn ad_limited(x: f32, k: f32) -> f32 {
    if (x <= 1.0) {
        return min(x * k, 1.0);
    }
    return x * min(k, 1.0);
}

fn ad_hsv_to_rgb(h: f32, s: f32, v: f32) -> vec3<f32> {
    let hp = rem_euclid(h, 360.0) / 60.0;
    let sector = min(u32(hp), 5u);
    let f = hp - f32(sector);
    let p = v * (1.0 - s);
    let q = v * (1.0 - s * f);
    let t = v * (1.0 - s * (1.0 - f));
    switch sector {
        case 0u: { return vec3<f32>(v, t, p); }
        case 1u: { return vec3<f32>(q, v, p); }
        case 2u: { return vec3<f32>(p, v, t); }
        case 3u: { return vec3<f32>(p, q, v); }
        case 4u: { return vec3<f32>(t, p, v); }
        default: { return vec3<f32>(v, p, q); }
    }
}

// `HsvTable::apply` (linear ProPhoto)
fn ad_table(t: u32, rgb: vec3<f32>) -> vec3<f32> {
    let mx = max(rgb.x, max(rgb.y, rgb.z));
    if (!(mx > 0.0) || mx > 3.0e38) {
        return rgb;
    }
    let mn = min(rgb.x, min(rgb.y, rgb.z));
    let d = mx - mn;
    var hue = 0.0;
    if (d > 0.0) {
        var hh = 0.0;
        if (mx == rgb.x) {
            hh = rem_euclid((rgb.y - rgb.z) / d, 6.0);
        } else if (mx == rgb.y) {
            hh = (rgb.z - rgb.x) / d + 2.0;
        } else {
            hh = (rgb.x - rgb.y) / d + 4.0;
        }
        hue = hh * 60.0;
    }
    let sat = d / mx;
    let srgbv = pu(F_AD_T + t * 6u + 4u) != 0u;
    var vi = mx;
    if (srgbv) {
        vi = ad_lin_to_srgb(min(mx, 1.0));
    }
    let k = ad_lookup(t, hue, sat, vi);
    let s2 = ad_limited(sat, k.y);
    var v2 = 0.0;
    if (srgbv && mx <= 1.0) {
        v2 = ad_srgb_to_lin(min(ad_lin_to_srgb(mx) * k.z, 1.0));
    } else if (srgbv) {
        v2 = mx * min(ad_srgb_to_lin(min(k.z, 1.0)), 1.0);
    } else {
        v2 = ad_limited(mx, k.z);
    }
    return ad_hsv_to_rgb(hue + k.x, s2, v2);
}

// `adobe::rgb_tone` with the tone table: hue-preserving, on the largest and smallest channel
fn ad_tone3(hi: f32, mid: f32, lo: f32) -> vec3<f32> {
    let th = tone_apply(hi);
    let tl = tone_apply(lo);
    return vec3<f32>(th, tl + (th - tl) * (mid - lo) / (hi - lo), tl);
}

fn ad_rgb_tone(c0: vec3<f32>) -> vec3<f32> {
    // an HDR render keeps scene values up to SCENE_TOP (`adobe::rgb_tone_to`)
    let c = clamp(c0, vec3<f32>(0.0), vec3<f32>(pf(F_SCENE_TOP)));
    let r = c.x;
    let g = c.y;
    let b = c.z;
    if (r >= g) {
        if (g > b) {
            let o = ad_tone3(r, g, b);
            return vec3<f32>(o.x, o.y, o.z);
        } else if (b > r) {
            let o = ad_tone3(b, r, g);
            return vec3<f32>(o.y, o.z, o.x);
        } else if (b > g) {
            let o = ad_tone3(r, b, g);
            return vec3<f32>(o.x, o.z, o.y);
        }
        let gg = tone_apply(g);
        return vec3<f32>(tone_apply(r), gg, gg);
    }
    if (r >= b) {
        let o = ad_tone3(g, r, b);
        return vec3<f32>(o.y, o.x, o.z);
    } else if (b > g) {
        let o = ad_tone3(b, g, r);
        return vec3<f32>(o.z, o.y, o.x);
    }
    let o = ad_tone3(g, b, r);
    return vec3<f32>(o.z, o.x, o.y);
}

// `finish::Curves::apply`'s master curve (curve LUT 3), hue-preserving like `adobe::rgb_tone`
fn cv_tone3(hi: f32, mid: f32, lo: f32) -> vec3<f32> {
    let th = curve(3u, hi);
    let tl = curve(3u, lo);
    return vec3<f32>(th, tl + (th - tl) * (mid - lo) / (hi - lo), tl);
}

fn curve_rgb_tone(c0: vec3<f32>) -> vec3<f32> {
    let c = clamp(c0, vec3<f32>(0.0), vec3<f32>(1.0));
    let r = c.x;
    let g = c.y;
    let b = c.z;
    if (r >= g) {
        if (g > b) {
            let o = cv_tone3(r, g, b);
            return vec3<f32>(o.x, o.y, o.z);
        } else if (b > r) {
            let o = cv_tone3(b, r, g);
            return vec3<f32>(o.y, o.z, o.x);
        } else if (b > g) {
            let o = cv_tone3(r, b, g);
            return vec3<f32>(o.x, o.z, o.y);
        }
        let gg = curve(3u, g);
        return vec3<f32>(curve(3u, r), gg, gg);
    }
    if (r >= b) {
        let o = cv_tone3(g, r, b);
        return vec3<f32>(o.y, o.x, o.z);
    } else if (b > g) {
        let o = cv_tone3(b, g, r);
        return vec3<f32>(o.z, o.y, o.x);
    }
    let o = cv_tone3(g, b, r);
    return vec3<f32>(o.z, o.x, o.y);
}

fn adobe_tone(c: vec3<f32>) -> vec3<f32> {
    var p = mul3(ad_mat(F_AD_TO_PP), c);
    let black = pf(F_AD_BLACK);
    if (black > 0.0) {
        p = vec3<f32>(ad_ramp(p.x, black), ad_ramp(p.y, black), ad_ramp(p.z, black));
    }
    let nt = pu(F_AD_NT);
    let hk = pf(F_AD_HSM_K);
    for (var t = 0u; t < nt; t += 1u) {
        if (t == 0u && hk != 1.0) {
            p = ad_table(t, p * hk) / hk;
        } else {
            p = ad_table(t, p);
        }
    }
    return mul3(ad_mat(F_AD_FROM_PP), ad_rgb_tone(p));
}

fn encode_srgb(v: f32) -> f32 {
    let o = pu(F_SRGB_OFF);
    let f = clamp(v, 0.0, 1.0) * f32(SRGB_N);
    let i = min(u32(f), SRGB_N - 1u);
    let t = f - f32(i);
    return aux[o + i] + (aux[o + i + 1u] - aux[o + i]) * t;
}

// `finish::STYLE_LUMA`: luminance weights of the style space (ProPhoto) the curves and grain act in.
const STYLE_LUMA: vec3<f32> = vec3<f32>(0.288075, 0.711835, 0.00009);

// sRGB-curve-encoded → linear (clamped to 0..1), `lightcraft_color::transfer::srgb_to_linear`.
fn decode_srgb(v: f32) -> f32 {
    let e = clamp(v, 0.0, 1.0);
    if (e > 0.04045) {
        return pow((e + 0.055) / 1.055, 2.4);
    }
    return e / 12.92;
}

fn curve(ch: u32, x: f32) -> f32 {
    let o = pu(F_CURVE_OFF) + ch * CURVE_N;
    let f = clamp(x, 0.0, 1.0) * f32(CURVE_N - 1u);
    let i = min(u32(f), CURVE_N - 2u);
    let t = f - f32(i);
    return aux[o + i] + (aux[o + i + 1u] - aux[o + i]) * t;
}

fn band_weights(h: f32) -> array<f32, 8> {
    var w: array<f32, 8>;
    for (var i = 0u; i < 8u; i++) {
        let a = pf(F_BANDH + i);
        let b = pf(F_BANDH + (i + 1u) % 8u);
        let span = rem_euclid(wrap_angle(b - a), TAU);
        let d = rem_euclid(wrap_angle(h - a), TAU);
        if (d <= span) {
            let t = d / span;
            let s = 0.5 - 0.5 * cos(t * PI);
            w[i] += 1.0 - s;
            w[(i + 1u) % 8u] += s;
            break;
        }
    }
    return w;
}

// `PointK::apply` for Point Color sample `k` (OkLCh in, OkLCh out).
fn point_color(k: u32, lch: vec3<f32>) -> vec3<f32> {
    let o = F_PC + k * POINT_WORDS;
    let sl = pf(o);
    let sc = pf(o + 1u);
    let sh = pf(o + 2u);
    let var_k = pf(o + 6u);
    let wh_ = pf(o + 7u);
    let wc_ = pf(o + 8u);
    let wl_ = pf(o + 9u);
    var l = lch.x;
    var c = lch.y;
    var h = lch.z;
    var wh = 1.0;
    if (sc >= 0.02) {
        wh = (1.0 - sstep(0.5 * wh_, wh_, abs(wrap_angle(h - sh)))) * sstep(0.005, 0.025, c);
    }
    if (wh <= 0.0) {
        return lch;
    }
    let wc = 1.0 - sstep(0.5 * wc_, wc_, abs(c - sc));
    let wl = 1.0 - sstep(0.5 * wl_, wl_, abs(l - sl));
    let w = wh * wc * wl;
    if (w <= 0.0) {
        return lch;
    }
    h += w * (var_k * wrap_angle(h - sh) + pf(o + 3u));
    c += w * var_k * (c - sc);
    c = max(c * (1.0 + w * pf(o + 4u)), 0.0);
    l += w * (var_k * (l - sl) + pf(o + 5u));
    return vec3<f32>(l, c, h);
}

// `colortab::axis`: lower index and fraction of x on a clamped axis.
fn ctab_axis(a_in: array<f32, 5>, n: u32, x: f32) -> vec2<f32> {
    var a = a_in;
    let xc = clamp(x, a[0], a[n - 1u]);
    var i = 0u;
    while (i + 2u < n && xc > a[i + 1u]) {
        i++;
    }
    return vec2<f32>(f32(i), (xc - a[i]) / (a[i + 1u] - a[i]));
}

// `colortab::lookup`: (Δhue, ln chroma ratio, Δlightness) from the summed table in aux.
fn ctab_lookup(l: f32, c: f32, h: f32) -> vec3<f32> {
    let la = array<f32, 5>(0.2, 0.4, 0.6, 0.8, 1.0);
    let ca = array<f32, 5>(0.0, 0.06, 0.14, 0.26, 0.26);
    let u = rem_euclid(h / TAU, 1.0) * 36.0;
    let h0 = u32(floor(u)) % 36u;
    let fh = u - floor(u);
    let lx = ctab_axis(la, 5u, l);
    let cx = ctab_axis(ca, 4u, c);
    let l0 = u32(lx.x);
    let c0 = u32(cx.x);
    let base = pu(F_CTAB_OFF);
    var out = vec3<f32>(0.0);
    for (var dh = 0u; dh < 2u; dh++) {
        let wh = select(1.0 - fh, fh, dh == 1u);
        for (var dl = 0u; dl < 2u; dl++) {
            let wl = select(1.0 - lx.y, lx.y, dl == 1u);
            for (var dc = 0u; dc < 2u; dc++) {
                let wc = select(1.0 - cx.y, cx.y, dc == 1u);
                let k = base + ((((h0 + dh) % 36u) * 5u + l0 + dl) * 4u + c0 + dc) * 3u;
                out += wh * wl * wc * vec3<f32>(aux[k], aux[k + 1u], aux[k + 2u]);
            }
        }
    }
    return out;
}

// `point_color_cr::smoothed`: a smoothed trapezoid (outer o0, o3; inner o1, o2) raised to ramps.
fn pc_trap(x: f32, o: u32, r0: f32, r1: f32) -> f32 {
    let p0 = aux[o];
    let p1 = aux[o + 1u];
    let p2 = aux[o + 2u];
    let p3 = aux[o + 3u];
    var rise = 1.0;
    if (x < p1) {
        if (x <= p0) {
            rise = 0.0;
        } else {
            let t = (x - p0) / (p1 - p0);
            rise = pow(t * t * (3.0 - 2.0 * t), r0);
        }
    }
    var fall = 1.0;
    if (x > p2) {
        if (x >= p3) {
            fall = 0.0;
        } else {
            let t = (p3 - x) / (p3 - p2);
            fall = pow(t * t * (3.0 - 2.0 * t), r1);
        }
    }
    return min(rise, fall);
}

fn pc_hsv(p: vec3<f32>) -> vec3<f32> {
    let mx = max(p.x, max(p.y, p.z));
    let mn = min(p.x, min(p.y, p.z));
    let d = mx - mn;
    var h = 0.0;
    if (d >= 1e-9) {
        if (mx == p.x) {
            h = rem_euclid((p.y - p.z) / d, 6.0);
        } else if (mx == p.y) {
            h = (p.z - p.x) / d + 2.0;
        } else {
            h = (p.x - p.y) / d + 4.0;
        }
    }
    var s = 0.0;
    if (mx > 0.0) {
        s = d / mx;
    }
    return vec3<f32>(h / 6.0 * TAU, s, mx);
}

fn pc_rgb(h: f32, s: f32, v: f32) -> vec3<f32> {
    let h6 = rem_euclid(h / TAU, 1.0) * 6.0;
    let c = v * s;
    let x = c * (1.0 - abs(h6 % 2.0 - 1.0));
    var q = vec3<f32>(c, 0.0, x);
    let k = u32(h6);
    if (k == 0u) {
        q = vec3<f32>(c, x, 0.0);
    } else if (k == 1u) {
        q = vec3<f32>(x, c, 0.0);
    } else if (k == 2u) {
        q = vec3<f32>(0.0, c, x);
    } else if (k == 3u) {
        q = vec3<f32>(0.0, x, c);
    } else if (k == 4u) {
        q = vec3<f32>(x, 0.0, c);
    }
    return q + vec3<f32>(v - c);
}

// `point_color_cr::Swatch::apply` (swatch at aux[o..o+22]).
fn point_color_cr(o: u32, p: vec3<f32>) -> vec3<f32> {
    let q = max(p, vec3<f32>(0.0));
    if (max(q.x, max(q.y, q.z)) <= 1e-6) {
        return p;
    }
    let hsv = pc_hsv(q);
    let ev = ad_lin_to_srgb(min(hsv.z, 1.0));
    let d = wrap_angle(hsv.x - aux[o]);
    let sw = aux[o + 1u];
    let lw = aux[o + 2u];
    let weight = pc_trap(d * aux[o + 3u], o + 6u, 1.0282, 1.0282)
        * pc_trap(sw + (hsv.y - sw) * aux[o + 4u], o + 10u, 1.8348, 0.9897)
        * pc_trap(lw + (ev - lw) * aux[o + 5u], o + 14u, 2.7765, 1.1354)
        * min(hsv.y / 0.0578, 1.0);
    if (!(weight > 0.0)) {
        return p;
    }
    let variance = aux[o + 21u];
    let turn = weight * (aux[o + 18u] + variance * d);
    var log_s = weight * aux[o + 19u];
    var log_v = weight * aux[o + 20u];
    if (variance != 0.0) {
        let s2 = clamp(sw + (hsv.y - sw) * exp2(0.7904 * variance), 1e-4, 1.0);
        let v2 = ad_srgb_to_lin(clamp(lw + (ev - lw) * exp2(0.2123 * variance), 1e-4, 1.0));
        log_s += weight * log2(s2 / max(hsv.y, 1e-4));
        log_v += weight * log2(v2 / max(ad_srgb_to_lin(ev), 1e-6));
    }
    let out = pc_rgb(hsv.x + turn, clamp(hsv.y * exp2(log_s), 0.0, 1.0), hsv.z * exp2(log_v));
    return min(out, max(q, vec3<f32>(1.0))) + (p - q);
}

// `texture_cr::strength`: the detail's scale for Texture −1..1 (piecewise linear).
fn texture_strength(amount: f32) -> f32 {
    let a = clamp(amount, -1.0, 1.0);
    let xs = array<f32, 6>(-1.0, -0.5, 0.0, 0.25, 0.5, 1.0);
    let ys = array<f32, 6>(-0.752, -0.499, 0.0, 0.334, 0.546, 0.852);
    var i = 1u;
    while (i < 5u && a > xs[i]) {
        i++;
    }
    let t = (a - xs[i - 1u]) / (xs[i] - xs[i - 1u]);
    return ys[i - 1u] + (ys[i] - ys[i - 1u]) * t;
}

// `ColorOps::apply`.
fn color_ops(rgb: vec3<f32>, local_sat: f32, local_hue: f32) -> vec3<f32> {
    if (pu(F_OPS_IDENTITY) != 0u && local_sat == 0.0 && local_hue == 0.0) {
        return rgb;
    }
    let lab0 = oklab(rgb);
    var l = lab0.x;
    var c = sqrt(lab0.y * lab0.y + lab0.z * lab0.z);
    var h = atan2(lab0.z, lab0.y);
    if (pu(F_BW_CR) != 0u) {
        // Camera Raw's B&W (`bw_cr::Mix::gray`): the colour's luminance times the grid's factor
        let y = min(bw_cr_gray(mul3(ad_mat(F_PP_TO), rgb), lum2020(rgb)), 1.0);
        return grade_lab(oklab(vec3<f32>(y)));
    }
    if (pu(F_BW) != 0u) {
        // B&W: the mix reads the colour as it is (Saturation / Vibrance / HSL don't apply)
        let w = band_weights(h);
        var mix = 0.0;
        for (var i = 0u; i < 8u; i++) {
            mix += w[i] * pf(F_BW_MIX + i);
        }
        l = max(l + mix * min(c / 0.2, 1.0) * BW_GAIN, 0.0);
        return grade_lab(vec3<f32>(l, 0.0, 0.0));
    }
    if (pu(F_CTAB) != 0u) {
        let d = ctab_lookup(l, c, h);
        h += d.x;
        c *= exp(d.y);
        l += d.z;
    }
    if (pu(F_MIXER) != 0u) {
        let w = band_weights(h);
        var dh = 0.0;
        var ds = 0.0;
        var dl = 0.0;
        for (var i = 0u; i < 8u; i++) {
            dh += w[i] * pf(F_MIX_HUE + i);
            ds += w[i] * pf(F_MIX_SAT + i);
            dl += w[i] * pf(F_MIX_LUM + i);
        }
        let chroma_w = min(c / 0.12, 1.0);
        h += dh * chroma_w;
        c *= max(1.0 + ds, 0.0);
        l += dl * chroma_w * sqrt(max(l, 0.05));
    }
    if (pu(F_PC_CR_N) > 0u) {
        // Lightroom's swatches with Camera Raw's model (`point_color_cr`), linear ProPhoto HSV
        var p = mul3(ad_mat(F_PP_TO), oklab_inv(vec3<f32>(l, c * cos(h), c * sin(h))));
        for (var k = 0u; k < pu(F_PC_CR_N); k++) {
            p = point_color_cr(pu(F_PC_CR_OFF) + k * 22u, p);
        }
        let lab = oklab(mul3(ad_mat(F_PP_FROM), p));
        l = lab.x;
        c = sqrt(lab.y * lab.y + lab.z * lab.z);
        h = atan2(lab.z, lab.y);
    }
    for (var k = 0u; k < pu(F_NPC); k++) {
        let r = point_color(k, vec3<f32>(l, c, h));
        l = r.x;
        c = r.y;
        h = r.z;
    }
    let vib = pf(F_VIBRANCE);
    if (vib != 0.0) {
        let low = 1.0 - clamp(c / 0.22, 0.0, 1.0);
        var skin = 1.0;
        if (vib > 0.0) {
            let q = wrap_angle(h - pf(F_SKIN)) / 0.35;
            skin = 1.0 - 0.6 * exp(-(q * q));
        }
        c *= max(1.0 + vib * low * low * skin * 1.2, 0.0);
    }
    let sat = pf(F_SATURATION);
    if (sat != 0.0 || local_sat != 0.0) {
        c *= max(1.0 + sat + local_sat, 0.0);
    }
    h += local_hue;
    return grade_lab(vec3<f32>(l, c * cos(h), c * sin(h)));
}

// `ColorOps::grade`: colour grading on OkLab, then back to linear Rec.2020.
fn grade_lab(lab_in: vec3<f32>) -> vec3<f32> {
    var lab = lab_in;
    if (pu(F_GRADING) != 0u) {
        let m = 0.5 - pf(F_BALANCE) * 0.25;
        let width = 0.15 + pf(F_BLENDING) * 0.5;
        let ws = 1.0 - sstep(m - width, m + width * 0.25, lab.x);
        let wh = sstep(m - width * 0.25, m + width, lab.x);
        let wm = max(1.0 - ws - wh, 0.0);
        let wts = array<f32, 4>(ws, wm, wh, 1.0);
        for (var k = 0u; k < 4u; k++) {
            let wt = wts[k];
            lab.y += pf(F_WHEELS + 3u * k) * wt;
            lab.z += pf(F_WHEELS + 3u * k + 1u) * wt;
            lab.x += pf(F_WHEELS + 3u * k + 2u) * wt;
        }
    }
    let rgb = oklab_inv(lab);
    if (pu(F_GRADE_CR) == 0u) {
        return rgb;
    }
    // Camera Raw's Color Grading (`grading_cr::Curves::apply`): a curve per ProPhoto channel
    let p = mul3(ad_mat(F_PP_TO), rgb);
    let q = vec3<f32>(grade_cr_channel(0u, p.x), grade_cr_channel(1u, p.y), grade_cr_channel(2u, p.z));
    return mul3(ad_mat(F_PP_FROM), q);
}

// `grading_cr::Curves::channel`: 64 gains over x^(1/2.2), three channels per sample.
fn grade_cr_channel(c: u32, v: f32) -> f32 {
    let at = pow(clamp(v, 0.0, 1.0), 1.0 / 2.2) * 63.0;
    let i = min(u32(at), 62u);
    let t = at - f32(i);
    let o = pu(F_GRADE_CR_OFF);
    return v * (aux[o + i * 3u + c] * (1.0 - t) + aux[o + (i + 1u) * 3u + c] * t);
}

// `bw_cr::Mix::gray`: trilinear lookup of the 36 × 6 × 6 grid of linear-ProPhoto HSV.
fn bw_cr_gray(p0: vec3<f32>, y: f32) -> f32 {
    let p = max(p0, vec3<f32>(0.0));
    let mx = max(p.x, max(p.y, p.z));
    let mn = min(p.x, min(p.y, p.z));
    if (mx <= 1e-6) {
        return max(y, 0.0);
    }
    let d = mx - mn;
    var h = 0.0;
    if (d >= 1e-9) {
        if (mx == p.x) {
            h = rem_euclid((p.y - p.z) / d, 6.0) / 6.0;
        } else if (mx == p.y) {
            h = ((p.z - p.x) / d + 2.0) / 6.0;
        } else {
            h = ((p.x - p.y) / d + 4.0) / 6.0;
        }
    }
    let s = d / mx;
    let fh = rem_euclid(h, 1.0) * 36.0 - 0.5;
    let fs = clamp(sqrt(clamp(s, 0.0, 1.0)) * 6.0 - 0.5, 0.0, 5.0);
    let fv = clamp(pow(clamp(mx, 0.0, 1.0), 0.45) * 6.0 - 0.5, 0.0, 5.0);
    let h0 = floor(fh);
    let th = fh - h0;
    let ts = fract(fs);
    let tv = fract(fv);
    let s0 = u32(fs);
    let v0 = u32(fv);
    let s1 = min(s0 + 1u, 5u);
    let v1 = min(v0 + 1u, 5u);
    let o = pu(F_BW_CR_OFF);
    var acc = 0.0;
    for (var dh = 0; dh < 2; dh++) {
        let hi = u32(rem_euclid(h0 + f32(dh), 36.0));
        let wh = select(1.0 - th, th, dh == 1);
        for (var k = 0u; k < 2u; k++) {
            let si = select(s0, s1, k == 1u);
            let ws = select(1.0 - ts, ts, k == 1u);
            for (var m = 0u; m < 2u; m++) {
                let vi = select(v0, v1, m == 1u);
                let wv = select(1.0 - tv, tv, m == 1u);
                acc += aux[o + (hi * 6u + si) * 6u + vi] * wh * ws * wv;
            }
        }
    }
    return max(y, 0.0) * exp2(acc);
}

// `colorops::calibrate`: primaries matrix, then the shadows tint (luminance kept).
fn calibrate(c0: vec3<f32>) -> vec3<f32> {
    var c = c0;
    if (pu(F_CALIB) != 0u) {
        let m = array<vec3<f32>, 3>(
            vec3<f32>(pf(F_CALIB_M), pf(F_CALIB_M + 1u), pf(F_CALIB_M + 2u)),
            vec3<f32>(pf(F_CALIB_M + 3u), pf(F_CALIB_M + 4u), pf(F_CALIB_M + 5u)),
            vec3<f32>(pf(F_CALIB_M + 6u), pf(F_CALIB_M + 7u), pf(F_CALIB_M + 8u)),
        );
        c = max(mul3(m, c), vec3<f32>(0.0));
    }
    let st = pf(F_SHADOW_TINT);
    if (st != 0.0) {
        let y0 = lum2020(c);
        let w = 1.0 - sstep(-5.0, -0.5, log2(max(y0, 1e-7) / 0.18));
        c.y *= max(1.0 - SHADOW_TINT_K * st * w, 0.0);
        let y1 = max(lum2020(c), 1e-9);
        c = c * y0 / y1;
    }
    return c;
}

// `finish::refine_saturation`.
fn refine_saturation(before: vec3<f32>, after: vec3<f32>, refine: f32) -> vec3<f32> {
    let lw = STYLE_LUMA;
    let y0 = dot(before, lw);
    let y1 = dot(after, lw);
    let s0 = (max(before.x, max(before.y, before.z)) - min(before.x, min(before.y, before.z))) / max(y0, 1e-4);
    let s1 = (max(after.x, max(after.y, after.z)) - min(after.x, min(after.y, after.z))) / max(y1, 1e-4);
    if (s1 <= 1e-6 || s0 <= 1e-6) {
        return after;
    }
    let k = clamp(pow(s0 / s1, 1.0 - refine), 0.0, 4.0);
    if (abs(k - 1.0) < 1e-6) {
        return after;
    }
    // chroma scaled in linear light around the luminance (`finish::refine_saturation`)
    let lin = vec3<f32>(decode_srgb(after.x), decode_srgb(after.y), decode_srgb(after.z));
    let yl = dot(lin, lw);
    let o = max(yl + (lin - yl) * k, vec3<f32>(0.0));
    return vec3<f32>(ad_lin_to_srgb(o.x), ad_lin_to_srgb(o.y), ad_lin_to_srgb(o.z));
}

fn ghash(i: i32, j: i32, seed: u32) -> f32 {
    var v = (bitcast<u32>(i) * GRAIN_H0) ^ (bitcast<u32>(j) * GRAIN_H1) ^ (seed * GRAIN_H2);
    v ^= v >> 13u;
    v = v * GRAIN_H3;
    v ^= v >> 15u;
    return f32(v & 0xffffu) / 32768.0 - 1.0;
}

fn grain_noise(x: f32, y: f32, seed: u32) -> f32 {
    let x0 = floor(x);
    let y0 = floor(y);
    let fx = x - x0;
    let fy = y - y0;
    let i = i32(x0);
    let j = i32(y0);
    let u = fx;
    let v = fy;
    let a = ghash(i, j, seed) + (ghash(i + 1, j, seed) - ghash(i, j, seed)) * u;
    let b = ghash(i, j + 1, seed) + (ghash(i + 1, j + 1, seed) - ghash(i, j + 1, seed)) * u;
    return a + (b - a) * v;
}

// Lightroom-matched Highlights / Shadows: EV by base EV, quarter stops from -10
fn hs_lut_at(ev: f32) -> f32 {
    let n = 57u;
    let f = clamp((ev + 10.0) * 4.0, 0.0, f32(n - 1u));
    let k = min(u32(f), n - 2u);
    let t = f - f32(k);
    let o = pu(F_HS_OFF) + k;
    return aux[o] + (aux[o + 1u] - aux[o]) * t;
}

// `dehaze_cr::gain` applied to a channel `v` at Dehaze `s` (−1..0), tables at DZ_OFF.
fn dz_cr(s: f32, v: f32) -> f32 {
    if (v <= 0.0 || s >= 0.0) {
        return v;
    }
    let level = log2(max(v, 6.1035156e-5));
    let x = clamp((level - pf(F_DZ_KEY) + 12.0) / 13.0 * 48.0 - 0.5, 0.0, 47.0);
    let i = min(u32(x), 46u);
    let f = x - f32(i);
    let o = pu(F_DZ_OFF);
    let b0 = aux[o + i] + (aux[o + i + 1u] - aux[o + i]) * f;
    let b1 = aux[o + 48u + i] + (aux[o + 48u + i + 1u] - aux[o + 48u + i]) * f;
    let b2 = aux[o + 96u + i] + (aux[o + 96u + i + 1u] - aux[o + 96u + i]) * f;
    var g = 0.0;
    if (s <= -0.4) {
        g = b0 + (b1 - b0) * (s + 1.0) / 0.6;
    } else if (s <= -0.2) {
        g = b1 + (b2 - b1) * (s + 0.4) / 0.2;
    } else {
        g = b2 * (s / -0.2);
    }
    return v * exp2(g);
}

// `local_tone_cr::at`: table `k` (0 Contrast, 1 Whites, 2 Blacks) at amount `a`, level `l1`.
fn lt_cr(k: u32, a_in: f32, l1: f32) -> f32 {
    if (a_in == 0.0) {
        return 0.0;
    }
    let f = clamp((l1 + 5.5) / 0.5, 0.0, 15.0);
    let i = min(u32(f), 14u);
    let t = f - f32(i);
    let o = pu(F_LT_CR_OFF) + k * 64u;
    let r0 = aux[o + i] + (aux[o + i + 1u] - aux[o + i]) * t;
    let r1 = aux[o + 16u + i] + (aux[o + 16u + i + 1u] - aux[o + 16u + i]) * t;
    let r2 = aux[o + 32u + i] + (aux[o + 32u + i + 1u] - aux[o + 32u + i]) * t;
    let r3 = aux[o + 48u + i] + (aux[o + 48u + i + 1u] - aux[o + 48u + i]) * t;
    let a = clamp(a_in, -1.0, 1.0);
    if (a <= -0.5) {
        return r1 + (r0 - r1) * (-a - 0.5) / 0.5;
    } else if (a < 0.0) {
        return r1 * (-a / 0.5);
    } else if (a <= 0.5) {
        return r2 * (a / 0.5);
    }
    return r2 + (r3 - r2) * (a - 0.5) / 0.5;
}

fn legacy_hs(b: f32, hh: f32, ss: f32) -> f32 {
    let ws = 1.0 - sstep(-4.8, 0.3, b);
    let wh = sstep(-1.0, 2.8, b);
    return ss * 1.7 * ws * sqrt(ws) + hh * 1.7 * wh;
}

// ---- LUT profiles (`lightcraft_pipeline::lut`, R03)

// `Lut3d::apply` (trilinear) on the table in `aux` at LUT_OFF (red fastest).
fn lut_at(r: u32, g: u32, b: u32) -> vec3<f32> {
    let n = pu(F_LUT_N);
    let o = pu(F_LUT_OFF) + 3u * (r + n * (g + n * b));
    return vec3<f32>(aux[o], aux[o + 1u], aux[o + 2u]);
}

fn lut_apply(c: vec3<f32>) -> vec3<f32> {
    let n = pu(F_LUT_N);
    let s = f32(n - 1u);
    var idx = vec3<u32>(0u);
    var fr = vec3<f32>(0.0);
    for (var k = 0u; k < 3u; k++) {
        let lo = pf(F_LUT_MIN + k);
        let span = max(pf(F_LUT_MAX + k) - lo, 1e-6);
        let x = clamp((c[k] - lo) / span, 0.0, 1.0) * s;
        let i = min(u32(floor(x)), n - 2u);
        idx[k] = i;
        fr[k] = x - f32(i);
    }
    let r = idx.x;
    let g = idx.y;
    let b = idx.z;
    let c00 = mix(lut_at(r, g, b), lut_at(r + 1u, g, b), fr.x);
    let c10 = mix(lut_at(r, g + 1u, b), lut_at(r + 1u, g + 1u, b), fr.x);
    let c01 = mix(lut_at(r, g, b + 1u), lut_at(r + 1u, g, b + 1u), fr.x);
    let c11 = mix(lut_at(r, g + 1u, b + 1u), lut_at(r + 1u, g + 1u, b + 1u), fr.x);
    return mix(mix(c00, c10, fr.y), mix(c01, c11, fr.y), fr.z);
}

// `LutTransfer::encode` / `decode` (clamped to 0..1).
fn lut_encode(v: f32) -> f32 {
    let x = clamp(v, 0.0, 1.0);
    let t = pu(F_LUT_TRC);
    if (t == 1u) {
        return ad_lin_to_srgb(x);
    }
    if (t == 2u) {
        return pow(x, 1.0 / pf(F_LUT_GAMMA));
    }
    return x;
}

fn lut_decode(v: f32) -> f32 {
    let x = clamp(v, 0.0, 1.0);
    let t = pu(F_LUT_TRC);
    if (t == 1u) {
        return ad_srgb_to_lin(x);
    }
    if (t == 2u) {
        return pow(x, pf(F_LUT_GAMMA));
    }
    return x;
}

fn lut_mat(f: u32) -> array<vec3<f32>, 3> {
    return array<vec3<f32>, 3>(
        vec3<f32>(pf(f), pf(f + 1u), pf(f + 2u)),
        vec3<f32>(pf(f + 3u), pf(f + 4u), pf(f + 5u)),
        vec3<f32>(pf(f + 6u), pf(f + 7u), pf(f + 8u)),
    );
}

// `Lut3d::apply_linear`: a creative profile's table on linear working colour, blended by LUT_AMT;
// out of the table's range a colour keeps its offset unless the table clips.
fn lut_profile(c: vec3<f32>) -> vec3<f32> {
    let k = pf(F_LUT_AMT);
    let t = mul3(lut_mat(F_LUT_TO), c);
    let e = vec3<f32>(lut_encode(t.x), lut_encode(t.y), lut_encode(t.z));
    let o = lut_apply(e);
    let b = e + (o - e) * k;
    let bd = vec3<f32>(lut_decode(b.x), lut_decode(b.y), lut_decode(b.z));
    var r: vec3<f32>;
    if (pu(F_LUT_CLIP) != 0u) {
        r = mul3(lut_mat(F_LUT_FROM), bd);
    } else {
        let ed = vec3<f32>(lut_decode(e.x), lut_decode(e.y), lut_decode(e.z));
        r = mul3(lut_mat(F_LUT_FROM), t + bd - ed);
    }
    // non-finite -> 0 (NaN fails every comparison)
    return select(vec3<f32>(0.0), r, abs(r) < vec3<f32>(3.0e38));
}

fn enc8(v: f32) -> u32 {
    return u32(clamp(v, 0.0, 1.0) * 255.0 + 0.5);
}

// sRGB-curve-encoded value → the output space's own curve (`OutputTrc`: 0 sRGB, 1 gamma, 2 Rec.709).
fn out_encode(v: f32) -> f32 {
    let kind = pu(F_OUT_TRC);
    if (kind == 0u) {
        return v;
    }
    let e = clamp(v, 0.0, 1.0);
    var l = e / 12.92;
    if (e > 0.04045) {
        l = pow((e + 0.055) / 1.055, 2.4);
    }
    if (kind == 1u) {
        return pow(l, 1.0 / pf(F_OUT_GAMMA));
    }
    if (l < 0.018) {
        return l * 4.5;
    }
    return 1.099 * pow(l, 0.45) - 0.099;
}

// The look: curves, a creative profile's table and grain in the style space (ProPhoto
// primaries, sRGB curve; `finish::STYLE_SPACE`) whatever the output space, then back to linear
// working colour (`finish_with`'s `style`).
fn look(d: vec3<f32>, x: u32, y: u32) -> vec3<f32> {
    let sm = array<vec3<f32>, 3>(
        vec3<f32>(pf(F_ST_TO), pf(F_ST_TO + 1u), pf(F_ST_TO + 2u)),
        vec3<f32>(pf(F_ST_TO + 3u), pf(F_ST_TO + 4u), pf(F_ST_TO + 5u)),
        vec3<f32>(pf(F_ST_TO + 6u), pf(F_ST_TO + 7u), pf(F_ST_TO + 8u)),
    );
    let sb = array<vec3<f32>, 3>(
        vec3<f32>(pf(F_ST_FROM), pf(F_ST_FROM + 1u), pf(F_ST_FROM + 2u)),
        vec3<f32>(pf(F_ST_FROM + 3u), pf(F_ST_FROM + 4u), pf(F_ST_FROM + 5u)),
        vec3<f32>(pf(F_ST_FROM + 6u), pf(F_ST_FROM + 7u), pf(F_ST_FROM + 8u)),
    );
    let sd = mul3(sm, d);
    var e = vec3<f32>(encode_srgb(sd.x), encode_srgb(sd.y), encode_srgb(sd.z));
    if (pu(F_CURVES) != 0u) {
        let e0 = e;
        e = curve_rgb_tone(e);
        e = vec3<f32>(curve(0u, e.x), curve(1u, e.y), curve(2u, e.z));
        let rs = pf(F_REFINE_SAT);
        if (rs < 1.0) {
            e = refine_saturation(e0, e, rs);
        }
    }
    if (pu(F_LUT) == 1u) {
        // a creative profile's table after the curves (`finish_with`), in its own space
        let lin = mul3(sb, vec3<f32>(decode_srgb(e.x), decode_srgb(e.y), decode_srgb(e.z)));
        let l = mul3(sm, lut_profile(lin));
        e = vec3<f32>(encode_srgb(l.x), encode_srgb(l.y), encode_srgb(l.z));
    }
    if (pu(F_GRAIN) != 0u) {
        let px = f32(x) + 0.5;
        let py = f32(y) + 0.5;
        let gx = pf(F_GRAIN_AFF) * px + pf(F_GRAIN_AFF + 2u) * py + pf(F_GRAIN_AFF + 4u);
        let gy = pf(F_GRAIN_AFF + 1u) * px + pf(F_GRAIN_AFF + 3u) * py + pf(F_GRAIN_AFF + 5u);
        let sc = pf(F_GRAIN_SC);
        let seed = pu(F_GRAIN_SEED);
        let u = gx * sc;
        let v = gy * sc;
        let g = grain_noise(GRAIN_ROT_C * u - GRAIN_ROT_S * v, GRAIN_ROT_S * u + GRAIN_ROT_C * v, seed);
        let lum = dot(e, STYLE_LUMA);
        // `finish::grain_tone`
        let lo = pow(clamp(lum / GRAIN_KNEE, 0.0, 1.0), 0.75);
        let k = pf(F_GRAIN_AMT) * g * lo * (1.0 - GRAIN_HI * clamp(lum - GRAIN_HI0, 0.0, 1.0));
        e = e + k;
    }
    return mul3(sb, vec3<f32>(decode_srgb(e.x), decode_srgb(e.y), decode_srgb(e.z)));
}

@compute @workgroup_size(16, 16)
fn main(@builtin(global_invocation_id) gid: vec3<u32>) {
    let w = pu(F_W);
    let h = pu(F_H);
    let x = gid.x;
    let y = gid.y + pu(F_Y0);
    if (x >= w || y >= h) {
        return;
    }
    let i = y * w + x;
    let n = w * h;
    let gain = pf(F_GAIN);
    let ev = pf(F_EV);
    var c = vec3<f32>(img[3u * i], img[3u * i + 1u], img[3u * i + 2u]);
    if (gain != 1.0) {
        c = c * gain;
    }
    let l_pre = log_l[i];
    let l0 = l_pre + ev;

    // --- local (mask) contributions
    var lt: array<f32, MASK_SUMS>;
    var tint_on = false;
    var tint_dir = vec2<f32>(0.0, 0.0);
    var tint_amt = 0.0;
    let nm = pu(F_NMASK);
    for (var m = 0u; m < nm; m++) {
        let a = masks[m * n + i];
        if (a <= 0.0) {
            continue;
        }
        let t = pu(F_MASK_OFF) + m * MASK_TERMS;
        for (var k = 0u; k < MASK_SUMS; k++) {
            lt[k] += a * aux[t + k];
        }
        if (aux[t + MASK_SUMS] > 0.0) {
            tint_on = true;
            tint_dir = vec2<f32>(aux[t + MASK_SUMS + 1u], aux[t + MASK_SUMS + 2u]);
            tint_amt = a * aux[t + MASK_SUMS + 3u];
        }
    }
    let l_exp = lt[0];
    let l_temp = lt[1];
    let l_tint = lt[2];
    let l_noise = lt[14];

    // --- local Moiré (and the colour part of Noise): chromaticity towards its blur
    let mt = clamp(lt[15] + 0.5 * max(l_noise, 0.0), -1.0, 1.0);
    if (mt != 0.0 && pu(F_HAS_CHROMA) != 0u) {
        let raw = vec3<f32>(img[3u * i], img[3u * i + 1u], img[3u * i + 2u]);
        let y = lum2020(c);
        let ch0 = raw / max(lum2020(raw), 1e-6);
        let o = nm * n + 3u * i;
        let cb = vec3<f32>(masks[o], masks[o + 1u], masks[o + 2u]);
        c = max((ch0 + (cb - ch0) * mt) * y, vec3<f32>(0.0));
    }
    // --- local Defringe: desaturate purple / green fringes along edges
    let df = clamp(lt[16], 0.0, 1.0);
    if (df > 0.0 && pu(F_HAS_TEX) != 0u) {
        let yd = max(lum2020(c), 1e-6);
        let purple = (min(c.x, c.z) - c.y) / yd;
        let green = (c.y - max(c.x, c.z)) / yd;
        let k = df * sstep(0.04, 0.3, abs(l_pre - tex[pu(F_TEX_G_OFF) + i])) * max(sstep(0.02, 0.2, purple), sstep(0.02, 0.2, green));
        if (k > 0.0) {
            let y = lum2020(c);
            c = c + (y - c) * k;
        }
    }

    // --- dehaze (scene linear)
    let dz = pf(F_DEHAZE) + lt[10];
    if (dz != 0.0 && pu(F_HAS_DARK) != 0u) {
        let d = clamp(dark[i] / pf(F_AIR_PRE), 0.0, 1.0);
        let air = pf(F_AIR);
        if (dz > 0.0) {
            let t = max(1.0 - 0.95 * min(dz, 1.0) * d, 0.12);
            c = max((c - air * (1.0 - t)) / t, vec3<f32>(0.0));
        } else if (pu(F_DZ_CR) != 0u) {
            // Camera Raw's negative Dehaze (`dehaze_cr`): a curve per ProPhoto channel
            let q = mul3(ad_mat(F_PP_TO), c);
            let s = max(dz, -1.0);
            c = mul3(ad_mat(F_PP_FROM), vec3<f32>(dz_cr(s, q.x), dz_cr(s, q.y), dz_cr(s, q.z)));
        } else {
            let k = min(-dz, 1.0) * 0.7 * (0.35 + 0.65 * d);
            c = c + (air * 0.9 - c) * k;
        }
    }

    // --- local exposure / temp / tint
    if (l_exp != 0.0) {
        c = c * exp2(l_exp);
    }
    if (l_temp != 0.0 || l_tint != 0.0) {
        let y0 = lum2020(c);
        c = vec3<f32>(c.x * (1.0 + 0.3 * l_temp), c.y * (1.0 - 0.22 * l_tint), c.z * max(1.0 - 0.3 * l_temp, 0.0));
        let y1 = max(lum2020(c), 1e-9);
        c = c * y0 / y1;
    }

    // --- local tone in log luminance
    var l1 = l0;
    if (dz != 0.0 || l_exp != 0.0) {
        l1 = log_lum(c);
    }
    let shift = l1 - l0;
    let base = base_p[i] + ev + shift;
    var delta = 0.0;
    // Highlights / Shadows on the base, falling back to the pixel next to edges (`finish::halo_weight`)
    let hq = (l1 - base) / HALO_TAU;
    let wb = exp(-(hq * hq));
    if (pu(F_HS_LUT) != 0u) {
        delta += wb * hs_lut_at(base) + (1.0 - wb) * hs_lut_at(l1);
    }
    let hh = pf(F_HL) + lt[4];
    let ss = pf(F_SH) + lt[5];
    if (hh != 0.0 || ss != 0.0) {
        delta += wb * legacy_hs(base, hh, ss) + (1.0 - wb) * legacy_hs(l1, hh, ss);
    }
    if (pu(F_LT_CR) != 0u) {
        // Camera Raw's local Contrast / Whites / Blacks (`local_tone_cr`)
        delta += lt_cr(1u, lt[6], l1) + lt_cr(2u, lt[7], l1) + lt_cr(0u, lt[3], l1);
    } else {
        if (lt[6] != 0.0) {
            delta += lt[6] * 0.8 * sstep(0.5, 3.0, l1);
        }
        if (lt[7] != 0.0) {
            delta += lt[7] * 0.8 * (1.0 - sstep(-6.0, -1.5, l1));
        }
        if (lt[3] != 0.0) {
            delta += lt[3] * 0.14 * clamp(l1, -6.0, 4.0);
        }
    }
    let cl = pf(F_CLAR) + lt[9];
    if (cl != 0.0 && pu(F_HAS_CLAR) != 0u) {
        let det = clamp(l_pre - clar[i], -2.5, 2.5);
        let q = base / 3.2;
        let mid = exp(-(q * q));
        delta += cl * 0.85 * det * (0.35 + 0.65 * mid);
    }
    let tx = pf(F_TEX) + lt[8];
    // Camera Raw's Texture (`texture_cr`): each channel's pyramid detail (CPU-made, the first
    // 3·W·H words of `tex`), scaled
    var tex_gain = vec3<f32>(1.0);
    if (tx != 0.0 && pu(F_HAS_TEXD) != 0u) {
        let k = texture_strength(tx);
        tex_gain = exp2(k * vec3<f32>(tex[3u * i], tex[3u * i + 1u], tex[3u * i + 2u]));
    } else if (tx != 0.0 && pu(F_HAS_TEX) != 0u) {
        let det = l_pre - tex[pu(F_TEX_G_OFF) + i];
        let tame = 1.0 - 0.6 * sstep(0.4, 1.6, abs(det));
        delta += tx * 1.1 * clamp(det, -1.0, 1.0) * tame;
    }
    // capture sharpening: an unsharp mask read straight from log_l (`finish::Sharpen`)
    let sp = lt[13] * 0.6 * SHARPEN_GAIN + pf(F_SHARPEN);
    let sr = i32(pu(F_SHARP_R));
    if (sp != 0.0 && sr > 0) {
        var acc = 0.0;
        var gx = 0.0;
        var gy = 0.0;
        let sm = pf(F_SHARPEN_MASK);
        // the Edge Masking gradient's wider kernel (`finish::Sharpen::sample`)
        var rr = sr;
        if (sm > 0.0) {
            rr = max(sr, i32(pu(F_SHARP_GR)));
        }
        for (var dy = -rr; dy <= rr; dy++) {
            let yy = u32(clamp(i32(y) + dy, 0, i32(h) - 1));
            let ay = abs(dy);
            var wy = 0.0;
            var gwy = 0.0;
            if (ay <= SHARPEN_TAPS) {
                wy = pf(F_SHARP_W + u32(ay));
                gwy = pf(F_SHARP_GW + u32(ay));
            }
            for (var dx = -rr; dx <= rr; dx++) {
                let xx = u32(clamp(i32(x) + dx, 0, i32(w) - 1));
                let v = log_l[yy * w + xx];
                let ax = u32(abs(dx));
                acc += wy * pf(F_SHARP_W + ax) * v;
                let g = gwy * pf(F_SHARP_GW + ax) * v;
                gx += g * f32(dx);
                gy += g * f32(dy);
            }
        }
        var det = l_pre - acc;
        det = det / (1.0 + abs(det) / pf(F_SHARP_T));
        // Edge Masking by the smoothed gradient (`finish::Sharpen::mask`)
        var mk = 1.0;
        if (sm > 0.0) {
            let c = SHARPEN_MASK_C0 * exp(SHARPEN_MASK_K * min(sm, 1.0));
            mk = sstep(SHARPEN_MASK_LO * c, SHARPEN_MASK_HI * c, length(vec2<f32>(gx, gy)) / pf(F_SHARP_GN));
        }
        delta += sp * det * mk;
    }
    // local Noise: smooth (or, negative, boost) small-amplitude detail, keep edges
    if (l_noise != 0.0 && pu(F_HAS_TEX) != 0u) {
        let det = l_pre - tex[pu(F_TEX_G_OFF) + i];
        delta -= clamp(l_noise, -1.0, 1.0) * 0.9 * det * (1.0 - sstep(0.1, 0.5, abs(det)));
    }
    if (delta != 0.0) {
        c = c * exp2(delta);
    }
    c = c * tex_gain;

    // --- calibration (scene linear, before the tone map)
    if (pu(F_CALIB) != 0u || pf(F_SHADOW_TINT) != 0.0) {
        c = calibrate(c);
    }

    // --- tone map: an Adobe base's curve channel-wise in ProPhoto (after its look tables), else
    // on luminance with highlight desaturation
    var d = vec3<f32>(0.0);
    if (pu(F_ADOBE) != 0u) {
        d = adobe_tone(c);
    } else {
        let yl = lum2020(c);
        let o = tone_apply(yl);
        if (yl > 1e-9) {
            d = c * o / yl;
        }
        let k = chroma_scale(o);
        if (k != 1.0) {
            d = vec3<f32>(o) + (d - vec3<f32>(o)) * k;
        }
        let mx = max(d.x, max(d.y, d.z));
        let peak = pf(F_PEAK);
        if (mx > peak) {
            let t = clamp((mx - peak) / max(mx - o, 1e-6), 0.0, 1.0);
            d = d + (o - d) * t;
        }
    }

    // --- colour
    d = color_ops(d, lt[11], lt[12]);
    let hs = pf(F_HL_SAT);
    if (hs != 0.0) {
        // `finish::highlight_saturation`
        let yh = lum2020(d);
        let f = 1.0 + HL_SAT_GAIN * hs * sstep(0.5, 1.0, yh);
        d = max(vec3<f32>(yh) + (d - vec3<f32>(yh)) * f, vec3<f32>(0.0));
    }
    if (tint_on) {
        let lab = oklab(d);
        d = oklab_inv(vec3<f32>(lab.x, lab.y + tint_dir.x * 0.08 * tint_amt, lab.z + tint_dir.y * 0.08 * tint_amt));
    }

    // --- vignette (display linear, post-crop)
    if (pu(F_VIG) != 0u) {
        let fw = f32(w);
        let fh = f32(h);
        let aspect = fw / fh;
        let amount = pf(F_VIG_AMOUNT);
        let mixa = pf(F_VIG_ASPECT_MIX);
        let power = pf(F_VIG_POWER);
        let u = (f32(x) + 0.5) / fw * 2.0 - 1.0;
        let vv = (f32(y) + 0.5) / fh * 2.0 - 1.0;
        let sx = 1.0 + (aspect - 1.0) * mixa;
        let sy = 1.0 + (1.0 / aspect - 1.0) * mixa;
        let ax = abs(u * max(sx, 1.0) / max(sx, sy));
        let ay = abs(vv * max(sy, 1.0) / max(sx, sy));
        let dist = pow(pow(ax, power) + pow(ay, power), 1.0 / power);
        let start = pf(F_VIG_START);
        let t = sstep(start, start + pf(F_VIG_WIDTH), dist);
        if (t > 0.0) {
            let lum = clamp(lum2020(d), 0.0, 1.0);
            if (amount < 0.0) {
                var f = 1.0 + amount * t;
                let style = pu(F_VIG_STYLE);
                if (style == 1u) {
                    f += (1.0 - f) * pf(F_VIG_HL) * sstep(0.4, 1.0, lum);
                }
                if (style == 2u) {
                    d = d * (1.0 - (-amount) * t) + 0.0;
                } else {
                    d = d * f;
                }
            } else if (pf(F_PEAK) > 1.0) {
                // HDR: lighten towards SDR white; never pull HDR highlights down to it
                d = d + max(vec3<f32>(1.0) - d, vec3<f32>(0.0)) * amount * t * 0.85;
            } else {
                d = d + (1.0 - d) * amount * t * 0.85;
            }
        }
    }

    // --- the look: curves and grain in the style space (ProPhoto primaries, sRGB curve;
    // `finish::STYLE_SPACE`) whatever the output space, then back to linear working colour
    if (pu(F_CURVES) != 0u || pu(F_GRAIN) != 0u || pu(F_LUT) == 1u) {
        if (pf(F_PEAK) > 1.0) {
            // HDR: the look acts on the SDR part (up to white, in the style space); what lies
            // above SDR white is carried over unchanged
            let q = mul3(ad_mat(F_ST_TO), d);
            let lq = mul3(ad_mat(F_ST_TO), look(mul3(ad_mat(F_ST_FROM), min(q, vec3<f32>(1.0))), x, y));
            d = mul3(ad_mat(F_ST_FROM), lq + max(q - vec3<f32>(1.0), vec3<f32>(0.0)));
        } else {
            d = look(d, x, y);
        }
    }

    // --- gamut map to the output space (desaturate towards luminance until in range)
    let om = array<vec3<f32>, 3>(
        vec3<f32>(pf(F_OUT_M), pf(F_OUT_M + 1u), pf(F_OUT_M + 2u)),
        vec3<f32>(pf(F_OUT_M + 3u), pf(F_OUT_M + 4u), pf(F_OUT_M + 5u)),
        vec3<f32>(pf(F_OUT_M + 6u), pf(F_OUT_M + 7u), pf(F_OUT_M + 8u)),
    );
    var r = mul3(om, d);
    let peak = pf(F_PEAK);
    let yy = clamp(pf(F_OUT_Y) * r.x + pf(F_OUT_Y + 1u) * r.y + pf(F_OUT_Y + 2u) * r.z, 0.0, peak);
    var tg = 1.0;
    for (var k = 0u; k < 3u; k++) {
        let cc = r[k];
        if (cc < 0.0) {
            tg = min(tg, yy / max(yy - cc, 1e-9));
        } else if (cc > peak) {
            tg = min(tg, (peak - yy) / max(cc - yy, 1e-9));
        }
    }
    if (tg < 1.0) {
        r = yy + (r - yy) * tg;
    }

    // --- encode (sRGB curve, then the output's own curve)
    if (peak > 1.0) {
        // HDR: the (unbounded) sRGB curve, so values above SDR white encode above 1; the LUT
        // sees SDR values and the part above white is carried over. Stored linear (0..peak)
        // after the packed 8-bit (clipped) image.
        var e = vec3<f32>(ad_lin_to_srgb(max(r.x, 0.0)), ad_lin_to_srgb(max(r.y, 0.0)), ad_lin_to_srgb(max(r.z, 0.0)));
        if (pu(F_LUT) == 2u) {
            let c = clamp(e, vec3<f32>(0.0), vec3<f32>(1.0));
            e = e + (lut_apply(c) - c) * pf(F_LUT_AMT);
        }
        let o = n + 3u * i;
        for (var k = 0u; k < 3u; k++) {
            out[o + k] = bitcast<u32>(clamp(ad_srgb_to_lin(max(e[k], 0.0)), 0.0, peak));
        }
        let e8 = vec3<f32>(out_encode(e.x), out_encode(e.y), out_encode(e.z));
        out[i] = enc8(e8.x) | (enc8(e8.y) << 8u) | (enc8(e8.z) << 16u) | (255u << 24u);
        return;
    }
    var e = vec3<f32>(encode_srgb(r.x), encode_srgb(r.y), encode_srgb(r.z));
    if (pu(F_LUT) == 2u) {
        // a `.cube` profile on the encoded output, blended by the amount
        e = e + (lut_apply(clamp(e, vec3<f32>(0.0), vec3<f32>(1.0))) - e) * pf(F_LUT_AMT);
    }
    e = vec3<f32>(out_encode(e.x), out_encode(e.y), out_encode(e.z));
    out[i] = enc8(e.x) | (enc8(e.y) << 8u) | (enc8(e.z) << 16u) | (255u << 24u);
}
