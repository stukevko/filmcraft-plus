//! FilmCraft Plus portrait effects (definitions in `filmcraft_project::effect::plus`):
//!
//! - **Auto Focus** (`auto_focus`): keeps the person sharp and blurs everything else, with the
//!   focus following the face on its own. The focus area is a head-and-shoulders shape placed on
//!   the tracked face ([`super::face`]); the blur grows with the distance from it, like a lens
//!   with a wide aperture, and the background blur does not smear the person into the background.
//! - **Studio Light** (`studio_light`): a relighting grade. It exposes for the face, adds a soft
//!   key light that follows it (from the left, the right or the front), lifts the shadows like a
//!   fill light, dims the background, smooths skin and adds a little glow.
//!
//! Both fall back to a point the user can place when no face is in view, and ease between the
//! two, so nothing jumps when the face is lost for a moment.
//!
//! Original FilmCraft algorithms and preset values. Images are premultiplied linear f32; the
//! colour steps people judge by eye run on sRGB-encoded straight colour.

use filmcraft_project::EffectInstance;
use rayon::prelude::*;

use super::face;
use super::fast::{dec, enc};
use super::plus::{Roll, skin_mask, soft_blur};
use super::{FxCtx, Image, bv, chv, fv, luma, pv, smoothstep};

/// The person in the layer: face centre and face side in layer pixels.
#[derive(Clone, Copy, Debug)]
struct Subject {
    x: f32,
    y: f32,
    s: f32,
    /// 0..1: how much of this comes from a found face (the rest is the manual point).
    found: f32,
    /// Brightness of the face (display-encoded), when one was found.
    luma: f32,
}

fn finite_or(v: f32, d: f32) -> f32 {
    if v.is_finite() { v } else { d }
}

/// A 0..100 slider as 0..1.
fn unit(e: &EffectInstance, id: &str, cx: &FxCtx) -> f32 {
    (finite_or(fv(e, id, cx), 0.0) / 100.0).clamp(0.0, 1.0)
}

/// Where the person is: the tracked face, eased towards the manual Focus Point and Size when the
/// face is not found (or tracking is off).
fn subject(e: &EffectInstance, cx: &FxCtx, img: &Image) -> Subject {
    let (w, h) = (img.w as f32, img.h as f32);
    let p = pv(e, "point", cx, img);
    let (px, py) = (finite_or(p.x as f32, w * 0.5), finite_or(p.y as f32, h * 0.38));
    let size = (finite_or(fv(e, "size", cx), 28.0) / 100.0).clamp(0.02, 2.0) * h;
    let manual = Subject { x: px, y: py, s: size.max(1.0), found: 0.0, luma: 0.5 };
    if !bv(e, "track") {
        return manual;
    }
    let half = 0.08 + 0.9 * f64::from(unit(e, "smooth", cx));
    let Some(f) = face::tracked(cx, img, half) else { return manual };
    if !(f.x.is_finite() && f.y.is_finite() && f.size.is_finite() && f.conf.is_finite()) {
        return manual;
    }
    let k = smoothstep(0.0, 0.35, f.conf);
    let mix = |a: f32, b: f32| a + (b - a) * k;
    Subject {
        x: mix(manual.x, f.x.clamp(0.0, 1.0) * w),
        y: mix(manual.y, f.y.clamp(0.0, 1.0) * h),
        s: mix(manual.s, (f.size.clamp(0.01, 2.0) * h).max(1.0)),
        found: k,
        luma: finite_or(f.luma, 0.5).clamp(0.0, 1.0),
    }
}

/// How far `(x, y)` is outside the person, in face sizes (0 inside): a head ellipse around the
/// face (a little higher, for the hair) and, with `body`, neck, shoulders and chest below it,
/// open to the bottom of the frame.
fn outside(sub: &Subject, area: f32, body: bool, x: f32, y: f32) -> f32 {
    let s = (sub.s * area).max(1.0);
    let (dx, dy) = ((x - sub.x) / (0.78 * s), (y - (sub.y - 0.12 * s)) / s);
    let mut d = ((dx * dx + dy * dy).sqrt() - 1.0) * 0.85;
    if body {
        let v = (y - (sub.y + 0.75 * s)) / s;
        // neck → shoulders
        let half = 0.55 + 1.25 * smoothstep(0.0, 0.9, v);
        let bx = ((x - sub.x).abs() / s - half).max(0.0);
        let by = (-v).max(0.0);
        d = d.min((bx * bx + by * by).sqrt());
    }
    d.max(0.0)
}

/// Blur of `img` that takes little from where `keep` is low (normalised convolution), so the
/// sharp person does not bleed into the blurred background around them.
fn weighted_blur(img: &Image, keep_at: &(dyn Fn(f32, f32) -> f32 + Sync), sigma: f32) -> Image {
    // a blur this wide has no fine detail: work on a smaller copy and scale the result up
    let mut pre: Option<Image> = None;
    let mut f = 1.0f32;
    loop {
        let cur = pre.as_ref().unwrap_or(img);
        if !(sigma / f > 3.0 && f < 8.0 && cur.w >= 128 && cur.h >= 128) {
            break;
        }
        pre = Some(cur.downsample2());
        f *= 2.0;
    }
    let mut pre = pre.unwrap_or_else(|| img.clone());
    let (pw, ph) = (pre.w.max(1), pre.h.max(1));
    let mut wgt = Image::new(pre.w, pre.h);
    let (sx, sy) = (img.w as f32 / pw as f32, img.h as f32 / ph as f32);
    pre.px.par_chunks_mut(pw * 4).zip(wgt.px.par_chunks_mut(pw * 4)).enumerate().for_each(|(y, (prow, qrow))| {
        for (x, (p, q)) in prow.chunks_exact_mut(4).zip(qrow.chunks_exact_mut(4)).enumerate() {
            let k = keep_at((x as f32 + 0.5) * sx, (y as f32 + 0.5) * sy);
            let k = if k.is_finite() { k.clamp(0.0, 1.0) } else { 0.0 };
            for c in p.iter_mut() {
                *c *= k;
            }
            q.fill(k);
        }
    });
    let s = sigma / f;
    crate::effects::gaussian(&mut pre, s, s, true);
    crate::effects::gaussian(&mut wgt, s, s, true);
    pre.px.par_chunks_mut(4).zip(wgt.px.par_chunks(4)).for_each(|(p, q)| {
        let k = q[0].max(1e-3);
        for c in p.iter_mut() {
            *c /= k;
        }
    });
    if f <= 1.0 {
        return pre;
    }
    super::fast::enlarge(&pre, img.w, img.h)
}

/// Auto Focus: the person stays sharp, the rest is blurred, following the face.
pub fn auto_focus(img: &mut Image, e: &EffectInstance, cx: &FxCtx) {
    if img.w < 2 || img.h < 2 {
        return;
    }
    let blur = unit(e, "blur", cx);
    let dim = unit(e, "dim", cx);
    let show = bv(e, "show");
    if blur <= 1e-4 && dim <= 1e-4 && !show {
        return;
    }
    let sub = subject(e, cx, img);
    let area = (finite_or(fv(e, "area", cx), 100.0) / 100.0).clamp(0.2, 5.0);
    let body = bv(e, "body");
    let feather = 0.15 + 1.6 * unit(e, "feather", cx);
    let (w, h) = (img.w, img.h);
    // 0 in focus → 1 fully out of focus
    let mut mask = vec![0.0f32; w * h];
    mask.par_chunks_mut(w).enumerate().for_each(|(y, row)| {
        for (x, m) in row.iter_mut().enumerate() {
            *m = smoothstep(0.0, feather, outside(&sub, area, body, x as f32 + 0.5, y as f32 + 0.5));
        }
    });
    if show {
        // setup view: the out-of-focus part in red, no blur
        img.px.par_chunks_mut(4).zip(mask.par_iter()).for_each(|(p, m)| {
            let a = p[3];
            let k = 0.55 * m;
            p[0] += (a - p[0]) * k;
            p[1] += (0.08 * a - p[1]) * k;
            p[2] += (0.08 * a - p[2]) * k;
        });
        return;
    }
    if blur > 1e-4 {
        let k = w.min(h) as f32 / 1080.0;
        let sigma = (blur * 40.0 * k).max(0.35);
        // the background keeps a small weight inside the person so the division stays stable
        let keep = |x: f32, y: f32| 0.04 + 0.96 * smoothstep(0.0, feather, outside(&sub, area, body, x, y));
        let far = weighted_blur(img, &keep, sigma);
        let mid = weighted_blur(img, &keep, sigma * 0.4);
        img.px.par_chunks_mut(4).zip(mid.px.par_chunks(4)).zip(far.px.par_chunks(4)).zip(mask.par_iter()).for_each(|(((p, a), b), m)| {
            // sharp → half blur → full blur, like a lens going out of focus
            let (t0, t1) = ((m * 2.0).min(1.0), (m * 2.0 - 1.0).max(0.0));
            let g = 1.0 - 0.6 * dim * m;
            for c in 0..4 {
                let v = p[c] + (a[c] - p[c]) * t0;
                let v = (v + (b[c] - v) * t1) * if c < 3 { g } else { 1.0 };
                p[c] = if v.is_finite() { v } else { p[c] };
            }
        });
    } else if dim > 1e-4 {
        img.px.par_chunks_mut(4).zip(mask.par_iter()).for_each(|(p, m)| {
            let g = 1.0 - 0.6 * dim * m;
            p[0] *= g;
            p[1] *= g;
            p[2] *= g;
        });
    }
}

#[derive(Clone, Copy, Debug)]
struct Light {
    key: f32,
    side: f32,
    spread: f32,
    fill: f32,
    background: f32,
    skin: f32,
    warmth: f32,
    contrast: f32,
    glow: f32,
}

/// The presets after "Custom", in the order of `STUDIO_PRESETS`.
const LIGHTS: [Light; 5] = [
    // Soft Key: a big soft light from the upper left, gentle fill
    Light { key: 0.45, side: -0.35, spread: 1.0, fill: 0.35, background: 0.25, skin: 0.30, warmth: 0.10, contrast: 0.10, glow: 0.15 },
    // Beauty: frontal, very soft, lots of fill and smoothing
    Light { key: 0.55, side: 0.0, spread: 1.2, fill: 0.60, background: 0.15, skin: 0.60, warmth: 0.15, contrast: 0.0, glow: 0.30 },
    // Dramatic: a hard light from the side, dark background
    Light { key: 0.60, side: -0.70, spread: 0.7, fill: 0.0, background: 0.60, skin: 0.15, warmth: 0.0, contrast: 0.35, glow: 0.05 },
    // Clean Bright: even, bright, neutral (tutorials, product talk)
    Light { key: 0.35, side: 0.0, spread: 1.6, fill: 0.50, background: 0.0, skin: 0.25, warmth: 0.0, contrast: 0.05, glow: 0.10 },
    // Warm Evening: a warm lamp from the right, cosy
    Light { key: 0.50, side: 0.45, spread: 0.9, fill: 0.25, background: 0.40, skin: 0.30, warmth: 0.45, contrast: 0.15, glow: 0.25 },
];

fn light_params(e: &EffectInstance, cx: &FxCtx) -> Light {
    let p = chv(e, "preset") as usize;
    if p >= 1
        && let Some(l) = LIGHTS.get(p - 1)
    {
        return *l;
    }
    let signed = |id: &str| (finite_or(fv(e, id, cx), 0.0) / 100.0).clamp(-1.0, 1.0);
    Light {
        key: unit(e, "key", cx),
        side: signed("side"),
        spread: (finite_or(fv(e, "spread", cx), 100.0) / 100.0).clamp(0.3, 3.0),
        fill: unit(e, "fill", cx),
        background: unit(e, "background", cx),
        skin: unit(e, "skin", cx),
        warmth: signed("warmth"),
        contrast: unit(e, "contrast", cx),
        glow: unit(e, "glow", cx),
    }
}

/// Mean display-encoded brightness of the inner face area of `sub` in the picture (a coarse grid).
fn measured_luma(img: &Image, sub: &Subject) -> f32 {
    let r = (sub.s * 0.3).max(1.0);
    let (mut sum, mut n) = (0.0f32, 0u32);
    for j in 0..12 {
        for i in 0..12 {
            let (x, y) = (sub.x - r + 2.0 * r * (i as f32 + 0.5) / 12.0, sub.y - r + 2.0 * r * (j as f32 + 0.5) / 12.0);
            if x >= 0.0 && y >= 0.0 && (x as usize) < img.w && (y as usize) < img.h {
                let p = img.get(x as usize, y as usize);
                if p[3] > 1e-4 {
                    let v = luma(enc(Image::unpremul(p)));
                    if v.is_finite() {
                        sum += v.clamp(0.0, 1.0);
                        n += 1;
                    }
                }
            }
        }
    }
    if n > 0 { sum / n as f32 } else { 0.5 }
}

/// Studio Light: exposure for the face, key light, fill, background dim, skin smoothing, glow.
pub fn studio_light(img: &mut Image, e: &EffectInstance, cx: &FxCtx) {
    if img.w < 2 || img.h < 2 {
        return;
    }
    let strength = unit(e, "strength", cx);
    if strength <= 1e-4 {
        return;
    }
    let l = light_params(e, cx);
    let auto = bv(e, "auto");
    let show = bv(e, "show");
    let l = Light {
        key: l.key * strength,
        fill: l.fill * strength,
        background: l.background * strength,
        skin: l.skin * strength,
        warmth: l.warmth * strength,
        contrast: l.contrast * strength,
        glow: l.glow * strength,
        ..l
    };
    let idle = l.key <= 1e-4 && l.fill <= 1e-4 && l.background <= 1e-4 && l.skin <= 1e-4 && l.warmth.abs() <= 1e-4 && l.contrast <= 1e-4 && l.glow <= 1e-4;
    if idle && !auto && !show {
        return;
    }
    let sub = subject(e, cx, img);
    let (w, h) = (img.w, img.h);
    let k = w.min(h) as f32 / 1080.0;

    // exposure for the face: bring it to a pleasant level, at most a stop and a half up. Without
    // a found face, the brightness is measured where the manual point says the face is.
    let auto_stops = if auto {
        let here = measured_luma(img, &sub);
        let have = (here + (sub.luma - here) * sub.found).clamp(0.12, 0.95);
        ((0.56 / have).powf(2.2).log2().clamp(-1.0, 1.5) * 0.8 * strength).clamp(-1.0, 1.5)
    } else {
        0.0
    };
    // the light stands a little above the face, to the side it comes from
    let (lx, ly) = (sub.x + l.side * 0.9 * sub.s, sub.y - 0.35 * sub.s);
    let radius = (l.spread * 2.6 * sub.s).max(1.0);

    let smooth = (l.skin > 1e-3).then(|| soft_blur(img, 5.0 * k));
    // Where the person, the head and the light are: smooth shapes, so they are worked out on a
    // quarter-size map (background 0..1, head 0..1, light fall-off 0..1) and enlarged.
    let (gw, gh) = if w >= 64 && h >= 64 { (w / 4, h / 4) } else { (w, h) };
    let mut geo = Image::new(gw, gh);
    let (gx, gy) = (w as f32 / gw as f32, h as f32 / gh as f32);
    geo.px.par_chunks_mut(gw * 4).enumerate().for_each(|(y, row)| {
        for (x, g) in row.chunks_exact_mut(4).enumerate() {
            let (fx, fy) = ((x as f32 + 0.5) * gx, (y as f32 + 0.5) * gy);
            let dist = ((fx - lx).powi(2) + (fy - ly).powi(2)).sqrt() / radius;
            g[0] = smoothstep(0.2, 2.5, outside(&sub, 1.0, true, fx, fy));
            g[1] = 1.0 - smoothstep(0.0, 0.8, outside(&sub, 1.0, false, fx, fy));
            g[2] = (-dist * dist).exp();
        }
    });
    let geo = if (gw, gh) == (w, h) { geo } else { super::fast::enlarge(&geo, w, h) };
    // protect the highlights the light created
    let protect = Roll::new((0.6 * (l.key + auto_stops.max(0.0) * 0.5)).clamp(0.0, 0.5));
    img.px.par_chunks_mut(w * 4).zip(geo.px.par_chunks(w * 4)).enumerate().for_each(|(y, (row, grow))| {
        let _ = y;
        for x in 0..w {
            let p = &mut row[x * 4..x * 4 + 4];
            let a = p[3];
            if a <= 1e-6 {
                continue;
            }
            let fx = x as f32 + 0.5;
            // 0 on the person → 1 in the background
            let (bg, head, fall) = (grow[x * 4], grow[x * 4 + 1], grow[x * 4 + 2]);
            if show {
                // setup view: where the key light falls, in yellow
                let t = 0.6 * fall;
                p[0] += (a - p[0]) * t;
                p[1] += (0.85 * a - p[1]) * t;
                p[2] += (0.1 * a - p[2]) * t;
                continue;
            }
            // skin smoothing on the person (edges and texture borders stay); most pixels are not
            // skin and skip it
            let d0 = enc(Image::unpremul([p[0], p[1], p[2], a]));
            if let Some(blurred) = &smooth {
                let skin = skin_mask(d0);
                if skin > 0.0 {
                    let q = blurred.get(x, y);
                    let dq = enc(Image::unpremul(q));
                    let edge = 1.0 - smoothstep(0.03, 0.12, (luma(d0) - luma(dq)).abs());
                    let m = l.skin * skin * edge * (1.0 - 0.7 * bg);
                    for c in 0..4 {
                        p[c] += (q[c] - p[c]) * m;
                    }
                }
            }
            let a = p[3].max(1e-6);
            // light: exposure, key (brighter towards the light, with modelling across the face),
            // background dim; in linear light
            let across = ((fx - sub.x) / (1.2 * sub.s)).clamp(-1.0, 1.0) * l.side.signum() * l.side.abs().sqrt();
            // what is bright already takes less of the added light, so skin and shirts keep detail
            let room = 1.0 - 0.75 * smoothstep(0.5, 0.95, luma(d0));
            let up = auto_stops.max(0.0) + l.key * (0.85 * fall + 0.45 * across.max(0.0) * head);
            let down = auto_stops.min(0.0) + l.key * 0.45 * across.min(0.0) * head - l.background * 1.2 * bg;
            let stops = up * room + down;
            let gain = stops.clamp(-4.0, 4.0).exp2();
            let wk = l.warmth * (1.0 - 0.5 * bg);
            let c = [p[0] / a * gain * (1.0 + 0.18 * wk), p[1] / a * gain, p[2] / a * gain * (1.0 - 0.18 * wk)];
            let mut d = enc(c);
            // fill: lift the shadows on the person
            if l.fill > 1e-4 {
                let yv = luma(d).clamp(0.0, 1.0);
                let lift = l.fill * 0.2 * (1.0 - yv).powi(3) * smoothstep(0.0, 0.1, yv) * (1.0 - 0.7 * bg);
                let f = ((yv + lift) / yv.max(1e-3)).min(3.0);
                for v in &mut d {
                    *v *= f;
                }
            }
            // background: a little less colour, so the person stands out
            if l.background > 1e-4 {
                let yv = luma(d);
                let s = 1.0 - 0.3 * l.background * bg;
                for v in &mut d {
                    *v = yv + (*v - yv) * s;
                }
            }
            if l.contrast > 1e-4 {
                for v in &mut d {
                    let t = v.clamp(0.0, 1.0);
                    let sc = t * t * (3.0 - 2.0 * t);
                    *v = t + (sc - t) * l.contrast * 1.2 + (*v - t);
                }
            }
            for v in &mut d {
                *v = protect.at(v.max(0.0));
            }
            let o = dec(d);
            for c in 0..3 {
                p[c] = if o[c].is_finite() { o[c].max(0.0) * a } else { p[c] };
            }
        }
    });
    // glow: bright areas bloom softly (lighten only)
    if l.glow > 1e-4 && !show {
        let soft = soft_blur(img, 22.0 * k);
        img.px.par_chunks_mut(4).zip(soft.px.par_chunks(4)).for_each(|(p, q)| {
            for c in 0..3 {
                let v = p[c] + (q[c] - p[c]).max(0.0) * l.glow * 0.45;
                p[c] = if v.is_finite() { v } else { p[c] };
            }
        });
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use filmcraft_project::{ParamValue, find_effect};
    use filmcraft_time::Tick;

    fn cx() -> FxCtx<'static> {
        FxCtx {
            t: Tick::ZERO,
            px_scale: 1.0,
            seconds: 0.0,
            timecode: "",
            clip_name: "",
            project: None,
            env: None,
            frame_rate: 25.0,
            working: Default::default(),
        }
    }

    /// A detailed picture (stripes), so blur is measurable everywhere.
    fn picture(w: usize, h: usize) -> Image {
        let mut img = Image::new(w, h);
        for y in 0..h {
            for x in 0..w {
                let v = if (x / 2 + y / 2) % 2 == 0 { 0.6 } else { 0.1 };
                let p = &mut img.px[(y * w + x) * 4..(y * w + x) * 4 + 4];
                p.copy_from_slice(&[v, v * 0.8, v * 0.6, 1.0]);
            }
        }
        img
    }

    fn set(e: &mut EffectInstance, id: &str, v: ParamValue) {
        e.param_mut(id).unwrap_or_else(|| panic!("no parameter {id}")).value = v;
    }

    fn diff_at(a: &Image, b: &Image, x: usize, y: usize) -> f32 {
        a.get(x, y).iter().zip(b.get(x, y)).map(|(p, q)| (p - q).abs()).fold(0.0, f32::max)
    }

    #[test]
    fn auto_focus_keeps_the_focus_area_sharp_and_blurs_the_rest() {
        let src = picture(160, 284);
        let mut e = find_effect("auto_focus").expect("effect").instance();
        // no face in a striped picture: the manual point is used
        set(&mut e, "point", ParamValue::Vec2(filmcraft_geom::Vec2::new(80.0, 100.0)));
        set(&mut e, "blur", ParamValue::Float(60.0));
        let mut img = src.clone();
        auto_focus(&mut img, &e, &cx());
        assert!(img.px.iter().all(|v| v.is_finite()));
        assert!(diff_at(&img, &src, 80, 100) < 1e-4, "the face stays untouched");
        assert!(diff_at(&img, &src, 80, 230) < 0.02, "the body stays sharp");
        assert!(diff_at(&img, &src, 8, 20) > 0.05, "the corner is blurred");
        assert!(img.px.chunks(4).all(|p| (p[3] - 1.0).abs() < 1e-3), "opaque stays opaque");
        // without the body, the chest is blurred too
        set(&mut e, "body", ParamValue::Bool(false));
        let mut head_only = src.clone();
        auto_focus(&mut head_only, &e, &cx());
        assert!(diff_at(&head_only, &src, 80, 250) > 0.05);
        // moving the focus point moves the sharp area
        set(&mut e, "point", ParamValue::Vec2(filmcraft_geom::Vec2::new(30.0, 40.0)));
        let mut moved = src.clone();
        auto_focus(&mut moved, &e, &cx());
        assert!(diff_at(&moved, &src, 30, 40) < 1e-4 && diff_at(&moved, &src, 130, 110) > 0.05);
    }

    #[test]
    fn the_sharp_person_does_not_bleed_into_the_blurred_background() {
        // a bright person on black: next to them the background must stay nearly black
        let (w, h) = (160usize, 284usize);
        let mut src = Image::new(w, h);
        let e = find_effect("auto_focus").expect("effect").instance();
        let sub = subject(&e, &cx(), &src);
        for y in 0..h {
            for x in 0..w {
                let inside = outside(&sub, 1.0, true, x as f32 + 0.5, y as f32 + 0.5) <= 0.0;
                let v = if inside { 1.0 } else { 0.0 };
                src.px[(y * w + x) * 4..(y * w + x) * 4 + 4].copy_from_slice(&[v, v, v, 1.0]);
            }
        }
        let mut img = src.clone();
        auto_focus(&mut img, &e, &cx());
        // far left at face height: fully out of focus, and still dark
        let p = img.get(4, sub.y as usize);
        assert!(p[0] < 0.08, "halo {p:?}");
    }

    #[test]
    fn zero_amounts_are_the_identity_and_hostile_values_are_survived() {
        let src = picture(96, 120);
        let mut e = find_effect("auto_focus").expect("effect").instance();
        set(&mut e, "blur", ParamValue::Float(0.0));
        let mut img = src.clone();
        auto_focus(&mut img, &e, &cx());
        assert_eq!(img.px, src.px);
        let mut e = find_effect("studio_light").expect("effect").instance();
        set(&mut e, "strength", ParamValue::Float(0.0));
        let mut img = src.clone();
        studio_light(&mut img, &e, &cx());
        assert_eq!(img.px, src.px);
        for id in ["auto_focus", "studio_light"] {
            let def = find_effect(id).expect("effect");
            for bad in [f64::NAN, f64::INFINITY, -f64::INFINITY, 1e30, -1e30] {
                let mut e = def.instance();
                for p in e.params.values_mut() {
                    match &mut p.value {
                        ParamValue::Float(v) => *v = bad,
                        ParamValue::Vec2(v) => *v = filmcraft_geom::Vec2::new(bad, bad),
                        _ => {}
                    }
                }
                for (w, h) in [(1usize, 1usize), (3, 2), (64, 48)] {
                    let mut img = picture(w, h);
                    if id == "auto_focus" {
                        auto_focus(&mut img, &e, &cx())
                    } else {
                        studio_light(&mut img, &e, &cx())
                    }
                    assert!(img.px.iter().all(|v| v.is_finite()), "{id} with {bad} at {w}x{h}");
                }
            }
        }
    }

    #[test]
    fn every_studio_preset_lights_the_person_more_than_the_background() {
        let (w, h) = (160usize, 284usize);
        let mut src = Image::new(w, h);
        for p in src.px.chunks_mut(4) {
            p.copy_from_slice(&[0.18, 0.15, 0.13, 1.0]);
        }
        let n = filmcraft_project::effect::STUDIO_PRESETS.len() as u32;
        assert_eq!(n as usize, LIGHTS.len() + 1);
        for preset in 1..n {
            let mut e = find_effect("studio_light").expect("effect").instance();
            set(&mut e, "preset", ParamValue::Choice(preset));
            let mut img = src.clone();
            studio_light(&mut img, &e, &cx());
            assert!(img.px.iter().all(|v| v.is_finite() && *v >= 0.0), "preset {preset}");
            let sub = subject(&e, &cx(), &src);
            let face = luma({
                let p = img.get(sub.x as usize, sub.y as usize);
                [p[0], p[1], p[2]]
            });
            let corner = luma({
                let p = img.get(2, 2);
                [p[0], p[1], p[2]]
            });
            assert!(face > corner, "preset {preset}: face {face} corner {corner}");
            assert!(img.px.iter().zip(&src.px).any(|(a, b)| (a - b).abs() > 1e-3), "preset {preset} changed nothing");
        }
    }

    #[test]
    fn the_setup_views_draw_an_overlay() {
        let src = picture(96, 160);
        for (id, param) in [("auto_focus", "show"), ("studio_light", "show")] {
            let mut e = find_effect(id).expect("effect").instance();
            set(&mut e, param, ParamValue::Bool(true));
            let mut img = src.clone();
            if id == "auto_focus" {
                auto_focus(&mut img, &e, &cx())
            } else {
                studio_light(&mut img, &e, &cx())
            }
            assert!(img.px.iter().zip(&src.px).any(|(a, b)| (a - b).abs() > 0.05), "{id}");
        }
    }

    /// Local preview: renders both effects on raw RGB photographs.
    /// `FILMCRAFT_PORTRAIT_PREVIEW=<dir>` with `<name>_<w>x<h>.rgb` files; writes `<name>.<effect>.rgb`.
    #[test]
    #[ignore = "needs local test photographs"]
    fn portrait_preview() {
        let Some(dir) = std::env::var_os("FILMCRAFT_PORTRAIT_PREVIEW") else { return };
        for entry in std::fs::read_dir(&dir).expect("dir").flatten() {
            let name = entry.file_name().to_string_lossy().to_string();
            let Some(stem) = name.strip_suffix(".rgb") else { continue };
            if stem.contains('.') {
                continue;
            }
            let Some((w, h)) = stem.rsplit_once('_').and_then(|(_, d)| d.split_once('x')) else { continue };
            let (w, h): (usize, usize) = (w.parse().expect("w"), h.parse().expect("h"));
            let raw = std::fs::read(entry.path()).expect("read");
            let mut src = Image::new(w, h);
            for (p, q) in src.px.chunks_mut(4).zip(raw.chunks(3)) {
                let c = dec([f32::from(q[0]) / 255.0, f32::from(q[1]) / 255.0, f32::from(q[2]) / 255.0]);
                p.copy_from_slice(&[c[0], c[1], c[2], 1.0]);
            }
            let mut jobs: Vec<(String, EffectInstance)> = vec![("focus".into(), find_effect("auto_focus").expect("effect").instance())];
            let mut area = find_effect("auto_focus").expect("effect").instance();
            set(&mut area, "show", ParamValue::Bool(true));
            jobs.push(("area".into(), area));
            for preset in 1..filmcraft_project::effect::STUDIO_PRESETS.len() as u32 {
                let mut e = find_effect("studio_light").expect("effect").instance();
                set(&mut e, "preset", ParamValue::Choice(preset));
                jobs.push((format!("light{preset}"), e));
            }
            for (tag, e) in jobs {
                let mut img = src.clone();
                let started = std::time::Instant::now();
                if e.effect == "auto_focus" {
                    auto_focus(&mut img, &e, &cx())
                } else {
                    studio_light(&mut img, &e, &cx())
                }
                eprintln!("{stem} {tag}: {:.0} ms, face {:?}", started.elapsed().as_secs_f64() * 1000.0, face::face_in(&src));
                let out: Vec<u8> = img.px.chunks(4).flat_map(|p| enc([p[0], p[1], p[2]]).map(|v| (v.clamp(0.0, 1.0) * 255.0).round() as u8)).collect();
                std::fs::write(entry.path().with_file_name(format!("{stem}.{tag}.rgb")), out).expect("write");
            }
        }
    }
}
