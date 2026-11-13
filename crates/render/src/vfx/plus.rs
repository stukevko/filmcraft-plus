//! FilmCraft Plus effects (definitions in `filmcraft_project::effect::plus`): Organic Shake,
//! Flicker Stack and Look Builder.
//!
//! Conventions match the rest of [`crate::vfx`]: images are premultiplied linear f32. Distances
//! the user sets in pixels are "pixels at HD width" (1920), scaled by the layer width, so a preset
//! looks the same on a 720p proxy, a 1080p clip and a 4K clip. Colour work that artists judge by
//! eye (contrast, split toning, fades, grain) runs on sRGB-encoded straight colour.
//!
//! All three are original FilmCraft algorithms and preset values.

use filmcraft_geom::{Affine, Vec2};
use filmcraft_project::EffectInstance;
use rayon::prelude::*;

use super::{FxCtx, Image, add_light, blend_simple, bv, chv, cv, dec, enc, fv, hash01, highlights, luma, noise1, noise3, place, smoothstep};
use crate::effects::gaussian;

/// Layer pixels per "HD pixel" (the unit of every pixel-sized Plus parameter).
pub(super) fn hd(img: &Image) -> f64 {
    (img.w as f64 / 1920.0).max(1e-3)
}

pub(super) fn seed_of(e: &EffectInstance, cx: &FxCtx) -> u64 {
    let s = fv(e, "seed", cx);
    if s.is_finite() && s > 0.0 { s as u64 } else { 0 }
}

/// A gaussian blur that runs on a reduced copy for big radii (diffusion, glows): the result is
/// visually identical at those sizes and many times faster on 4K frames.
pub(super) fn soft_blur(img: &Image, sigma: f32) -> Image {
    if !(sigma.is_finite()) || sigma <= 0.3 {
        return img.clone();
    }
    let mut small = img.clone();
    let mut f = 1.0_f32;
    while sigma / f > 12.0 && small.w >= 64 && small.h >= 64 {
        small = small.downsample2();
        f *= 2.0;
    }
    gaussian(&mut small, sigma / f, sigma / f, true);
    if f <= 1.0 {
        return small;
    }
    let m = Affine::scale(img.w as f64 / small.w.max(1) as f64, img.h as f64 / small.h.max(1) as f64);
    small.transformed(img.w, img.h, &m)
}

/// Hue (0..1), saturation, value → RGB, all 0..1.
fn hsv(h: f32, s: f32, v: f32) -> [f32; 3] {
    let h = (h.rem_euclid(1.0)) * 6.0;
    let i = h.floor();
    let f = h - i;
    let (p, q, t) = (v * (1.0 - s), v * (1.0 - s * f), v * (1.0 - s * (1.0 - f)));
    match i as i32 {
        0 => [v, t, p],
        1 => [q, v, p],
        2 => [p, v, t],
        3 => [p, q, v],
        4 => [t, p, v],
        _ => [v, p, q],
    }
}

/// Horizontal red / blue shift by `d` layer pixels (green stays).
pub(super) fn rgb_shift(img: &mut Image, d: f32) {
    if !d.is_finite() || d.abs() <= 0.2 {
        return;
    }
    let src = img.clone();
    let w = img.w;
    img.px.par_chunks_mut(w * 4).enumerate().for_each(|(y, row)| {
        let py = y as f32 + 0.5;
        for x in 0..w {
            let px = x as f32 + 0.5;
            let rr = src.sample_bilinear(px - d, py);
            let g = src.get(x, y);
            let bb = src.sample_bilinear(px + d, py);
            let p = &mut row[x * 4..x * 4 + 4];
            p[0] = rr[0];
            p[1] = g[1];
            p[2] = bb[2];
            p[3] = rr[3].max(g[3]).max(bb[3]);
        }
    });
}

/// Where the clip stands in the music (Music Sync group): the index of the current beat (or
/// beat group), seconds since it started, and the interval between them. `None` when Sync to
/// Music is Off.
struct BeatClock {
    index: i64,
    since: f64,
    interval: f64,
    /// Beats per sync interval (so `index` can be turned back into a beat number).
    beats_per: f64,
    /// Kick (Audio) / Snare (Audio): driven by drum hits in the mix, not by a tempo.
    audio: bool,
    /// Hit strength 0..1 (1 for tempo sync).
    strength: f64,
}

/// The kick and snare hits at this frame in `track` (`None` = master mix).
pub(super) fn drum_hits(cx: &FxCtx, track: Option<usize>, sens: f32) -> (Option<super::beat::Hit>, Option<super::beat::Hit>) {
    let Some(w) = cx.env.and_then(|env| env.audio(0.0, super::beat::WINDOW, track)) else { return (None, None) };
    super::beat::hits(&w.samples, w.sample_rate, w.end, sens)
}

fn beat_clock(e: &EffectInstance, cx: &FxCtx) -> Option<BeatClock> {
    let mode = chv(e, "sync");
    if mode == 0 {
        return None;
    }
    if mode >= filmcraft_project::effect::SYNC_KICK {
        let (kick, snare) = drum_hits(cx, None, 1.0);
        let hit = if mode == filmcraft_project::effect::SYNC_KICK { kick } else { snare };
        // no hit in the last second: far past the last one, so everything is at rest
        let (index, since, strength) = hit.map_or((i64::MIN / 2, 10.0, 0.0), |h| (h.index, h.age, f64::from(h.strength)));
        return Some(BeatClock { index, since, interval: 0.5, beats_per: 2.0, audio: true, strength });
    }
    let bpm = fv(e, "bpm", cx) as f64;
    let bpm = if bpm.is_finite() { bpm.clamp(20.0, 400.0) } else { 120.0 };
    let beat = 60.0 / bpm;
    let beats_per = match mode {
        2 => 2.0,
        3 => 4.0,
        4 => 0.5,
        _ => 1.0,
    };
    let interval = beat * beats_per;
    let off = fv(e, "beat_offset", cx) as f64;
    let off = if off.is_finite() { off } else { 0.0 };
    let t = if cx.seconds.is_finite() { cx.seconds } else { 0.0 };
    let pos = (t - off) / interval;
    if !pos.is_finite() {
        return None;
    }
    let idx = pos.floor();
    Some(BeatClock { index: idx.clamp(-1e15, 1e15) as i64, since: (pos - idx) * interval, interval, beats_per, audio: false, strength: 1.0 })
}

// ================================================================== Organic Shake

#[derive(Clone, Copy)]
struct Shake {
    /// Horizontal / vertical amplitude, HD pixels.
    ax: f64,
    ay: f64,
    /// Zoom amplitude as a fraction (0.01 = 1 %).
    az: f64,
    /// Rotation amplitude, degrees.
    rot: f64,
    freq: f64,
    rough: f64,
    /// Seconds for the shake to fall to 1/e from the clip start (0 = constant).
    decay: f64,
    /// Heartbeat zoom pulse (fraction).
    pulse: f64,
}

/// Presets in `SHAKE_PRESETS` order after Custom.
const SHAKES: [Shake; 8] = [
    // Handheld
    Shake { ax: 7.0, ay: 5.0, az: 0.006, rot: 0.5, freq: 0.9, rough: 3.0, decay: 0.0, pulse: 0.0 },
    // Walking
    Shake { ax: 9.0, ay: 16.0, az: 0.010, rot: 0.9, freq: 1.8, rough: 2.0, decay: 0.0, pulse: 0.0 },
    // Running
    Shake { ax: 20.0, ay: 32.0, az: 0.020, rot: 2.2, freq: 3.0, rough: 3.0, decay: 0.0, pulse: 0.0 },
    // Car Ride
    Shake { ax: 5.0, ay: 12.0, az: 0.004, rot: 0.35, freq: 7.0, rough: 4.0, decay: 0.0, pulse: 0.0 },
    // Earthquake
    Shake { ax: 40.0, ay: 34.0, az: 0.030, rot: 1.8, freq: 9.0, rough: 5.0, decay: 0.0, pulse: 0.0 },
    // Impact Hit
    Shake { ax: 70.0, ay: 60.0, az: 0.060, rot: 4.0, freq: 13.0, rough: 4.0, decay: 0.7, pulse: 0.0 },
    // Drone Drift
    Shake { ax: 14.0, ay: 10.0, az: 0.012, rot: 0.8, freq: 0.25, rough: 2.0, decay: 0.0, pulse: 0.0 },
    // Heartbeat
    Shake { ax: 2.0, ay: 2.0, az: 0.002, rot: 0.1, freq: 1.0, rough: 2.0, decay: 0.0, pulse: 0.035 },
];

fn shake_params(e: &EffectInstance, cx: &FxCtx) -> Shake {
    let p = chv(e, "preset") as usize;
    if p >= 1
        && let Some(s) = SHAKES.get(p - 1)
    {
        return *s;
    }
    let g = |id: &str| {
        let v = fv(e, id, cx) as f64;
        if v.is_finite() { v.max(0.0) } else { 0.0 }
    };
    Shake {
        ax: g("amp_x"),
        ay: g("amp_y"),
        az: g("amp_z") / 100.0,
        rot: g("amp_rot"),
        freq: g("frequency").max(0.01),
        rough: g("roughness").clamp(1.0, 6.0),
        decay: g("decay"),
        pulse: 0.0,
    }
}

/// Fractal 1-D noise, about −1..1 (fractional octaves fade in, like `fbm`).
fn shake_noise(t: f64, ch: u64, seed: u64, rough: f64) -> f64 {
    let n = rough.clamp(1.0, 6.0);
    let (mut s, mut a, mut f, mut norm) = (0.0, 1.0, 1.0, 0.0);
    let mut o = 0u64;
    while (o as f64) < n {
        let k = (n - o as f64).min(1.0);
        s += noise1(t * f, ch * 10 + o, seed) as f64 * a * k;
        norm += a * k;
        a *= 0.5;
        f *= 2.07;
        o += 1;
    }
    (s / norm.max(1e-9) * 1.5).clamp(-1.5, 1.5)
}

/// A double "lub-dub" pulse at about 66 bpm, 0..1.
fn heartbeat(t: f64) -> f64 {
    let ph = (t * 1.1).rem_euclid(1.0);
    let g = |c: f64, w: f64| (-((ph - c) / w).powi(2)).exp();
    g(0.0, 0.05) + g(1.0, 0.05) + 0.6 * g(0.2, 0.05)
}

/// Organic Shake: natural, multi-octave camera shake with motion blur and edge cover.
pub fn organic_shake(img: &mut Image, e: &EffectInstance, cx: &FxCtx) {
    let s = shake_params(e, cx);
    let inten = (fv(e, "intensity", cx) as f64 / 100.0).max(0.0);
    let speed = (fv(e, "speed", cx) as f64 / 100.0).clamp(0.01, 10.0);
    if !inten.is_finite() || inten <= 1e-4 || img.w == 0 || img.h == 0 {
        return;
    }
    let k = hd(img);
    let seed = seed_of(e, cx);
    let t0 = if cx.seconds.is_finite() { cx.seconds } else { 0.0 };
    let env = if s.decay > 1e-3 { (-t0.max(0.0) / s.decay).exp() } else { 1.0 };
    // Music Sync: a fresh hit on every beat that dies away before the next one
    // (drum hits: nothing between hits, and a stronger hit shakes harder)
    let beat_env = beat_clock(e, cx).map_or(1.0, |b| {
        let hit = b.strength * (-b.since / (b.interval * 0.35)).exp();
        if b.audio { hit } else { 0.1 + 0.9 * hit }
    });
    let amp = inten * env * beat_env;
    if amp <= 1e-4 {
        return;
    }
    let (w, h) = (img.w as f64, img.h as f64);
    // Edge cover from the *peak* amplitudes, so the scale does not pulse with the shake.
    let cover = if bv(e, "auto_scale") {
        let peak = 1.5 * inten;
        let th = (s.rot * peak).to_radians().min(0.6);
        let rot_cover = th.cos() + (w / h).max(h / w) * th.sin();
        let pos_cover = 1.0 + 2.0 * (s.ax * k * peak / w).max(s.ay * k * peak / h);
        let zoom_out = (1.0 - s.az * peak).max(0.5);
        (rot_cover * pos_cover / zoom_out).clamp(1.0, 3.0)
    } else {
        1.0
    };
    let c = Vec2::new(w / 2.0, h / 2.0);
    let xform = |t: f64| -> Affine {
        let tt = t * speed * s.freq;
        let n = |ch: u64| shake_noise(tt, ch, seed, s.rough);
        let off = Vec2::new(n(0) * s.ax * k * amp, n(1) * s.ay * k * amp);
        let zoom = (1.0 + (n(3) * s.az + s.pulse * heartbeat(t * speed)) * amp).max(0.05) * cover;
        Affine::translate(c.x + off.x, c.y + off.y)
            .then_apply(&Affine::rotate_deg(n(2) * s.rot * amp))
            .then_apply(&Affine::scale(zoom, zoom))
            .then_apply(&Affine::translate(-c.x, -c.y))
    };
    let fps = if cx.frame_rate.is_finite() && cx.frame_rate > 1.0 { cx.frame_rate } else { 30.0 };
    let shutter = (fv(e, "motion_blur", cx) as f64 / 100.0).clamp(0.0, 1.0) / fps;
    let n = match chv(e, "blur_quality") {
        0 => 3,
        2 => 13,
        _ => 7,
    };
    let mid = xform(t0);
    // how far the picture travels during the shutter (corners), in layer pixels
    let travel = if shutter > 0.0 {
        let (a, b) = (xform(t0 - shutter / 2.0), xform(t0 + shutter / 2.0));
        [Vec2::new(0.0, 0.0), Vec2::new(w, 0.0), Vec2::new(0.0, h), Vec2::new(w, h)].iter().map(|p| (a.apply(*p) - b.apply(*p)).length()).fold(0.0, f64::max)
    } else {
        0.0
    };
    if travel < 0.4 {
        *img = img.transformed(img.w, img.h, &mid);
        return;
    }
    // sub-frame samples, more when the motion is long
    let n = n.max(((travel / 3.0).ceil() as usize).min(24)).max(2);
    let invs: Vec<Affine> = (0..n).filter_map(|i| xform(t0 + (i as f64 / (n - 1) as f64 - 0.5) * shutter).inverse()).collect();
    if invs.is_empty() {
        return;
    }
    let src = img.clone();
    let inv_n = 1.0 / invs.len() as f32;
    let wi = img.w;
    img.px.par_chunks_mut(wi * 4).enumerate().for_each(|(y, row)| {
        let py = y as f64 + 0.5;
        for x in 0..wi {
            let px = x as f64 + 0.5;
            let mut acc = [0.0f32; 4];
            for m in &invs {
                let (u, v) = (m.a * px + m.c * py + m.e, m.b * px + m.d * py + m.f);
                let p = src.sample_bilinear(u as f32, v as f32);
                for k in 0..4 {
                    acc[k] += p[k];
                }
            }
            for k in 0..4 {
                row[x * 4 + k] = acc[k] * inv_n;
            }
        }
    });
}

// ================================================================== Flicker Stack

#[derive(Clone, Copy)]
struct Flicker {
    rate: f64,
    density: f64,
    /// blur, color, light, scale, slide, split, time
    ops: [bool; 7],
    blur: f64,
    color: f64,
    light: f64,
    scale: f64,
    slide: f64,
    split: f64,
    time: f64,
}

/// Presets in `FLICKER_PRESETS` order after Custom.
const FLICKERS: [Flicker; 6] = [
    // Subtle
    Flicker {
        rate: 4.0,
        density: 0.25,
        ops: [true, false, true, true, false, true, false],
        blur: 8.0,
        color: 0.2,
        light: 0.3,
        scale: 4.0,
        slide: 10.0,
        split: 5.0,
        time: 2.0,
    },
    // Music Video
    Flicker {
        rate: 8.0,
        density: 0.45,
        ops: [true, true, true, true, true, true, false],
        blur: 25.0,
        color: 0.45,
        light: 0.8,
        scale: 12.0,
        slide: 40.0,
        split: 14.0,
        time: 4.0,
    },
    // Glitch Hit
    Flicker {
        rate: 16.0,
        density: 0.6,
        ops: [false, true, false, true, true, true, true],
        blur: 10.0,
        color: 0.6,
        light: 0.4,
        scale: 8.0,
        slide: 90.0,
        split: 35.0,
        time: 3.0,
    },
    // Horror
    Flicker {
        rate: 6.0,
        density: 0.35,
        ops: [true, false, true, true, true, false, true],
        blur: 18.0,
        color: 0.0,
        light: 1.2,
        scale: 6.0,
        slide: 25.0,
        split: 0.0,
        time: 6.0,
    },
    // Strobe
    Flicker {
        rate: 12.0,
        density: 0.7,
        ops: [false, false, true, false, false, false, false],
        blur: 0.0,
        color: 0.0,
        light: 2.0,
        scale: 0.0,
        slide: 0.0,
        split: 0.0,
        time: 1.0,
    },
    // Dreamy
    Flicker {
        rate: 3.0,
        density: 0.3,
        ops: [true, true, true, true, false, false, false],
        blur: 40.0,
        color: 0.3,
        light: 0.6,
        scale: 5.0,
        slide: 0.0,
        split: 0.0,
        time: 2.0,
    },
];

fn flicker_params(e: &EffectInstance, cx: &FxCtx) -> Flicker {
    let p = chv(e, "preset") as usize;
    if p >= 1
        && let Some(f) = FLICKERS.get(p - 1)
    {
        return *f;
    }
    let g = |id: &str| {
        let v = fv(e, id, cx) as f64;
        if v.is_finite() { v.max(0.0) } else { 0.0 }
    };
    Flicker {
        rate: g("rate").clamp(0.25, 60.0),
        density: (g("density") / 100.0).clamp(0.0, 1.0),
        ops: [bv(e, "do_blur"), bv(e, "do_color"), bv(e, "do_light"), bv(e, "do_scale"), bv(e, "do_slide"), bv(e, "do_split"), bv(e, "do_time")],
        blur: g("blur_amt"),
        color: g("color_amt") / 100.0,
        light: g("light_amt") / 100.0,
        scale: g("scale_amt"),
        slide: g("slide_amt"),
        split: g("split_amt"),
        time: g("time_amt").clamp(1.0, 60.0),
    }
}

/// Flicker Stack: random rhythmic twitches of blur, colour, light, scale, slide, colour split and
/// time. Each "slot" (1 / Flickers per Second long) either fires or not (Density); a firing slot
/// picks a random subset of the enabled operations and a random strength, and eases out.
pub fn flicker_stack(img: &mut Image, e: &EffectInstance, cx: &FxCtx) {
    let f = flicker_params(e, cx);
    let amount = (fv(e, "amount", cx) as f64 / 100.0).max(0.0);
    if !amount.is_finite() || amount <= 1e-4 || img.w == 0 || img.h == 0 || !f.ops.iter().any(|&o| o) {
        return;
    }
    let seed = seed_of(e, cx);
    let t = if cx.seconds.is_finite() { cx.seconds.max(0.0) } else { 0.0 };
    // free-running slots, or one slot per beat (most beats fire) with Music Sync
    let (slot, frac, density) = match beat_clock(e, cx) {
        // a drum hit always fires, for half a second at most
        Some(b) if b.audio => {
            if b.since > b.interval {
                return;
            }
            (b.index, (b.since / b.interval) as f32, 1.0)
        }
        Some(b) => (b.index, (b.since / b.interval) as f32, f.density.max(0.75)),
        None => {
            let pos = t * f.rate;
            (pos.floor().clamp(-1e15, 1e15) as i64, (pos - pos.floor()) as f32, f.density)
        }
    };
    let r = |a: i64, b: i64| hash01(slot, a, b, seed);
    if r(0, 1) as f64 >= density {
        return;
    }
    let strength = ((0.35 + 0.65 * r(1, 1)) * (1.0 - 0.45 * frac)) as f64 * amount;
    // which operations fire in this slot (at least one of the enabled ones)
    let enabled: Vec<usize> = (0..7).filter(|&i| f.ops[i]).collect();
    let mut on = [false; 7];
    for &i in &enabled {
        on[i] = r(10 + i as i64, 2) < 0.55;
    }
    if !on.iter().any(|&o| o) {
        let pick = ((r(9, 2) * enabled.len() as f32) as usize).min(enabled.len().saturating_sub(1));
        if let Some(&i) = enabled.get(pick) {
            on[i] = true;
        }
    }
    let dir = |j: i64| r(20 + j, 3) * 2.0 - 1.0;
    let k = hd(img);

    // time: a frame from a little earlier (or later)
    if on[6]
        && let Some(env) = cx.env
    {
        let fps = env.frame_rate().max(1.0);
        let frames = (1.0 + (r(30, 4) as f64) * (f.time - 1.0)).round().max(1.0);
        let sign = if r(31, 4) < 0.75 { -1.0 } else { 1.0 };
        if let Some(fr) = env.frame(sign * frames / fps, e)
            && fr.w == img.w
            && fr.h == img.h
        {
            *img = fr;
        }
    }
    // scale + slide in one resample
    let sc = if on[3] { 1.0 + f.scale / 100.0 * strength * (0.5 + 0.5 * r(32, 5) as f64) } else { 1.0 };
    let sl = if on[4] {
        let horizontal = r(33, 5) < 0.7;
        let d = f.slide * k * strength * dir(1) as f64;
        if horizontal { Vec2::new(d, 0.0) } else { Vec2::new(0.0, d) }
    } else {
        Vec2::new(0.0, 0.0)
    };
    if (sc - 1.0).abs() > 1e-4 || sl.length() > 0.05 {
        let c = Vec2::new(img.w as f64 / 2.0, img.h as f64 / 2.0);
        place(img, c, sl, sc, 0.0);
    }
    if on[0] {
        let sigma = (f.blur * k * strength * 0.5) as f32;
        if sigma > 0.3 {
            *img = soft_blur(img, sigma);
        }
    }
    if on[5] {
        let d = (f.split * k * strength) as f32 * if dir(2) < 0.0 { -1.0 } else { 1.0 };
        rgb_shift(img, d);
    }
    let color_k = if on[1] { (f.color * strength).clamp(0.0, 1.0) as f32 } else { 0.0 };
    let light_k = if on[2] { (f.light * strength) as f32 } else { 0.0 };
    if color_k > 1e-3 || light_k > 1e-3 {
        let tint = if bv(e, "random_color") || chv(e, "preset") != 0 {
            hsv(r(40, 6), 0.75, 1.0)
        } else {
            let c = cv(e, "color", cx);
            [c[0], c[1], c[2]]
        };
        img.map_rgb(|c, _, _| {
            let mut c = c;
            if color_k > 0.0 {
                let d = enc(c);
                let t = blend_simple(4, d, tint);
                c = dec([d[0] + (t[0] - d[0]) * color_k, d[1] + (t[1] - d[1]) * color_k, d[2] + (t[2] - d[2]) * color_k]);
            }
            if light_k > 0.0 {
                let g = 1.0 + light_k;
                c = [c[0] * g, c[1] * g, c[2] * g];
            }
            c
        });
    }
}

// ================================================================== Beat Pulse

#[derive(Clone, Copy)]
struct Pulse {
    /// Pulse length as a fraction of the interval between pulses.
    decay: f64,
    zoom: f64,
    light: f64,
    glow: f64,
    split: f64,
    shake: f64,
    blur: f64,
}

/// Presets in `PULSE_PRESETS` order after Custom (zoom %, brightness %, glow 0..1, HD pixels).
const PULSES: [Pulse; 6] = [
    // Bass Punch
    Pulse { decay: 0.30, zoom: 6.0, light: 0.25, glow: 0.0, split: 0.0, shake: 6.0, blur: 0.0 },
    // Flash
    Pulse { decay: 0.25, zoom: 0.0, light: 1.8, glow: 0.0, split: 0.0, shake: 0.0, blur: 0.0 },
    // Glow Hit
    Pulse { decay: 0.40, zoom: 2.0, light: 0.3, glow: 0.6, split: 0.0, shake: 0.0, blur: 0.0 },
    // RGB Kick
    Pulse { decay: 0.25, zoom: 3.0, light: 0.1, glow: 0.0, split: 18.0, shake: 0.0, blur: 0.0 },
    // Zoom Bounce
    Pulse { decay: 0.45, zoom: 12.0, light: 0.0, glow: 0.0, split: 0.0, shake: 0.0, blur: 4.0 },
    // Shake Hit
    Pulse { decay: 0.30, zoom: 3.0, light: 0.1, glow: 0.0, split: 6.0, shake: 25.0, blur: 0.0 },
];

fn pulse_params(e: &EffectInstance, cx: &FxCtx) -> Pulse {
    let p = chv(e, "preset") as usize;
    if p >= 1
        && let Some(v) = PULSES.get(p - 1)
    {
        return *v;
    }
    let g = |id: &str| {
        let v = fv(e, id, cx) as f64;
        if v.is_finite() { v.max(0.0) } else { 0.0 }
    };
    Pulse {
        decay: (g("decay") / 100.0).clamp(0.02, 1.0),
        zoom: g("zoom").min(100.0),
        light: g("light") / 100.0,
        glow: (g("glow") / 100.0).min(1.0),
        split: g("split"),
        shake: g("shake"),
        blur: g("blur"),
    }
}

/// Beat Pulse: on every beat (Music Sync) the picture punches in, flashes, glows, splits, shakes
/// or blurs, and eases back before the next beat. With Sync Off it holds the full pulse, so the
/// Amount can be keyframed by hand.
pub fn beat_pulse(img: &mut Image, e: &EffectInstance, cx: &FxCtx) {
    let p = pulse_params(e, cx);
    let amount = (fv(e, "amount", cx) as f64 / 100.0).max(0.0);
    if !amount.is_finite() || amount <= 1e-4 || img.w == 0 || img.h == 0 {
        return;
    }
    let clock = beat_clock(e, cx);
    let (env, index, since) = match &clock {
        Some(b) => {
            // a 10 ms attack so the punch does not click, then an exponential release
            let attack = (b.since / 0.01).min(1.0);
            let release = (-b.since / (b.interval * p.decay)).exp();
            // the first beat of each bar hits a little harder
            let beat_no = (b.index as f64 * b.beats_per).floor() as i64;
            let accent = if bv(e, "accent_bar") && b.beats_per <= 1.0 && beat_no.rem_euclid(4) == 0 { 1.3 } else { 1.0 };
            (attack * release * accent * b.strength, b.index, b.since)
        }
        None => (1.0, 0, 0.0),
    };
    let kk = env * amount;
    if kk <= 1e-4 {
        return;
    }
    let k = hd(img);
    let seed = seed_of(e, cx);
    // zoom + shake in one resample
    let sc = 1.0 + p.zoom / 100.0 * kk;
    let sh = if p.shake > 0.0 {
        let ang = hash01(index, 1, 7, seed) as f64 * std::f64::consts::TAU;
        let wob = (since * 45.0).cos();
        let d = p.shake * k * kk * wob;
        Vec2::new(ang.cos() * d, ang.sin() * d)
    } else {
        Vec2::new(0.0, 0.0)
    };
    if (sc - 1.0).abs() > 1e-4 || sh.length() > 0.05 {
        let c = Vec2::new(img.w as f64 / 2.0, img.h as f64 / 2.0);
        place(img, c, sh, sc, 0.0);
    }
    if p.blur > 0.0 {
        let sigma = (p.blur * k * kk * 0.5) as f32;
        if sigma > 0.3 {
            *img = soft_blur(img, sigma);
        }
    }
    if p.split > 0.0 {
        let sign = if hash01(index, 2, 7, seed) < 0.5 { -1.0 } else { 1.0 };
        rgb_shift(img, (p.split * k * kk) as f32 * sign);
    }
    if p.glow > 1e-3 {
        let hi = soft_blur(&highlights(img, 0.55, 0.25), (22.0 * k) as f32);
        let amt = (p.glow * kk * 1.6) as f32;
        img.px.par_chunks_mut(4).zip(hi.px.par_chunks(4)).for_each(|(px, h)| add_light(px, [h[0] * amt, h[1] * amt, h[2] * amt]));
    }
    if p.light > 1e-3 {
        let g = (1.0 + p.light * kk) as f32;
        img.map_rgb(|c, _, _| [c[0] * g, c[1] * g, c[2] * g]);
    }
}

// ================================================================== Look Builder

#[derive(Clone, Copy)]
struct Look {
    skin: f32,
    exposure: f32,
    contrast: f32,
    saturation: f32,
    temp: f32,
    tint: f32,
    sh_col: [f32; 3],
    sh_amt: f32,
    hi_col: [f32; 3],
    hi_amt: f32,
    fade: f32,
    rolloff: f32,
    diffusion: f32,
    diff_size: f32,
    halation: f32,
    vignette: f32,
    vig_size: f32,
    grain: f32,
    grain_size: f32,
}

const NEUTRAL: Look = Look {
    skin: 0.0,
    exposure: 0.0,
    contrast: 0.0,
    saturation: 1.0,
    temp: 0.0,
    tint: 0.0,
    sh_col: [0.5, 0.5, 0.5],
    sh_amt: 0.0,
    hi_col: [0.5, 0.5, 0.5],
    hi_amt: 0.0,
    fade: 0.0,
    rolloff: 0.0,
    diffusion: 0.0,
    diff_size: 30.0,
    halation: 0.0,
    vignette: 0.0,
    vig_size: 0.6,
    grain: 0.0,
    grain_size: 1.2,
};

#[allow(clippy::too_many_arguments)]
const fn look(
    exposure: f32,
    contrast: f32,
    saturation: f32,
    temp: f32,
    tint: f32,
    sh: ([f32; 3], f32),
    hi: ([f32; 3], f32),
    fade: f32,
    rolloff: f32,
    lens: (f32, f32, f32),
    vig: (f32, f32),
    grain: (f32, f32),
) -> Look {
    Look {
        skin: 0.0,
        exposure,
        contrast,
        saturation,
        temp,
        tint,
        sh_col: sh.0,
        sh_amt: sh.1,
        hi_col: hi.0,
        hi_amt: hi.1,
        fade,
        rolloff,
        diffusion: lens.0,
        diff_size: lens.1,
        halation: lens.2,
        vignette: vig.0,
        vig_size: vig.1,
        grain: grain.0,
        grain_size: grain.1,
    }
}

const GREY: [f32; 3] = [0.5, 0.5, 0.5];

/// Looks in `LOOK_PRESETS` order after Custom. Fractions (0.15 = 15 %), colours sRGB 0..1.
const LOOKS: [Look; 12] = [
    // Teal & Orange
    look(0.0, 0.15, 1.10, 0.05, 0.0, ([0.0, 0.45, 0.55], 0.45), ([1.0, 0.62, 0.32], 0.35), 0.0, 0.08, (0.10, 30.0, 0.0), (0.25, 0.6), (0.08, 1.2)),
    // Warm Film
    look(0.05, 0.10, 0.95, 0.25, 0.03, ([0.35, 0.25, 0.20], 0.15), ([1.0, 0.80, 0.55], 0.20), 0.06, 0.15, (0.15, 35.0, 0.35), (0.30, 0.6), (0.25, 1.4)),
    // Cold Thriller
    look(-0.15, 0.25, 0.75, -0.30, -0.05, ([0.05, 0.25, 0.35], 0.35), ([0.75, 0.90, 1.0], 0.15), 0.0, 0.05, (0.0, 30.0, 0.0), (0.40, 0.55), (0.12, 1.2)),
    // Bleach Bypass
    look(0.0, 0.45, 0.45, 0.0, 0.0, (GREY, 0.0), (GREY, 0.0), 0.0, 0.10, (0.05, 30.0, 0.0), (0.30, 0.6), (0.18, 1.3)),
    // Vintage 70s
    look(0.0, -0.05, 0.85, 0.20, 0.08, ([0.25, 0.30, 0.15], 0.30), ([1.0, 0.85, 0.60], 0.30), 0.15, 0.20, (0.20, 40.0, 0.25), (0.35, 0.55), (0.35, 1.8)),
    // Pastel Dream
    look(0.25, -0.25, 0.80, 0.05, 0.10, ([0.55, 0.50, 0.70], 0.25), ([1.0, 0.85, 0.90], 0.25), 0.18, 0.25, (0.35, 60.0, 0.0), (0.10, 0.7), (0.05, 1.0)),
    // Noir
    look(0.0, 0.40, 0.0, 0.0, 0.0, (GREY, 0.0), (GREY, 0.0), 0.04, 0.10, (0.08, 30.0, 0.0), (0.50, 0.5), (0.30, 1.6)),
    // Neon Night
    look(0.05, 0.20, 1.40, -0.15, 0.20, ([0.15, 0.05, 0.40], 0.40), ([1.0, 0.30, 0.75], 0.25), 0.0, 0.05, (0.25, 40.0, 0.30), (0.30, 0.6), (0.10, 1.2)),
    // Golden Hour
    look(0.10, 0.08, 1.15, 0.40, 0.05, ([0.40, 0.25, 0.15], 0.20), ([1.0, 0.75, 0.35], 0.40), 0.0, 0.15, (0.25, 50.0, 0.20), (0.20, 0.65), (0.08, 1.2)),
    // Green Code
    look(0.0, 0.20, 0.70, -0.10, -0.35, ([0.0, 0.30, 0.10], 0.40), ([0.75, 1.0, 0.70], 0.20), 0.04, 0.05, (0.0, 30.0, 0.0), (0.35, 0.55), (0.15, 1.2)),
    // Faded Matte
    look(0.0, -0.15, 0.80, 0.0, 0.0, ([0.20, 0.25, 0.30], 0.15), (GREY, 0.0), 0.22, 0.18, (0.0, 30.0, 0.0), (0.15, 0.65), (0.15, 1.3)),
    // Cinema Contrast
    look(0.0, 0.30, 1.05, 0.0, 0.0, ([0.05, 0.15, 0.20], 0.15), ([1.0, 0.90, 0.80], 0.10), 0.0, 0.12, (0.08, 30.0, 0.0), (0.25, 0.6), (0.10, 1.2)),
];

fn look_params(e: &EffectInstance, cx: &FxCtx) -> Look {
    let p = chv(e, "look") as usize;
    if p >= 1
        && let Some(l) = LOOKS.get(p - 1)
    {
        // a look still lets the Subject tool add skin smoothing on top
        return Look { skin: (fv(e, "skin_smooth", cx) / 100.0).clamp(0.0, 1.0), ..*l };
    }
    let g = |id: &str| {
        let v = fv(e, id, cx);
        if v.is_finite() { v } else { 0.0 }
    };
    let c3 = |id: &str| {
        let c = cv(e, id, cx);
        [c[0], c[1], c[2]]
    };
    Look {
        skin: (g("skin_smooth") / 100.0).clamp(0.0, 1.0),
        exposure: g("exposure").clamp(-4.0, 4.0),
        contrast: (g("contrast") / 100.0).clamp(-1.0, 1.0),
        saturation: (g("saturation") / 100.0).clamp(0.0, 3.0),
        temp: (g("temperature") / 100.0).clamp(-1.0, 1.0),
        tint: (g("tint") / 100.0).clamp(-1.0, 1.0),
        sh_col: c3("shadow_color"),
        sh_amt: (g("shadow_amt") / 100.0).clamp(0.0, 1.0),
        hi_col: c3("highlight_color"),
        hi_amt: (g("highlight_amt") / 100.0).clamp(0.0, 1.0),
        fade: (g("fade") / 100.0).clamp(0.0, 0.5),
        rolloff: (g("rolloff") / 100.0).clamp(0.0, 0.5),
        diffusion: (g("diffusion") / 100.0).clamp(0.0, 1.0),
        diff_size: g("diffusion_size").clamp(1.0, 400.0),
        halation: (g("halation") / 100.0).clamp(0.0, 1.0),
        vignette: (g("vignette") / 100.0).clamp(0.0, 1.0),
        vig_size: (g("vignette_size") / 100.0).clamp(0.1, 1.5),
        grain: (g("grain") / 100.0).clamp(0.0, 1.0),
        grain_size: g("grain_size").clamp(0.5, 6.0),
    }
}

/// `NEUTRAL` → `l` by `s` (Strength).
fn scaled(l: Look, s: f32) -> Look {
    let m = |a: f32, b: f32| a + (b - a) * s;
    let n = NEUTRAL;
    Look {
        skin: l.skin,
        exposure: m(n.exposure, l.exposure),
        contrast: m(n.contrast, l.contrast),
        saturation: m(n.saturation, l.saturation),
        temp: m(n.temp, l.temp),
        tint: m(n.tint, l.tint),
        sh_col: l.sh_col,
        sh_amt: m(n.sh_amt, l.sh_amt),
        hi_col: l.hi_col,
        hi_amt: m(n.hi_amt, l.hi_amt),
        fade: m(n.fade, l.fade),
        rolloff: m(n.rolloff, l.rolloff),
        diffusion: m(n.diffusion, l.diffusion),
        diff_size: l.diff_size,
        halation: m(n.halation, l.halation),
        vignette: m(n.vignette, l.vignette),
        vig_size: l.vig_size,
        grain: m(n.grain, l.grain),
        grain_size: l.grain_size,
    }
}

/// Is this (display-space) colour in the skin-tone range? 0..1.
fn skin_mask(d: [f32; 3]) -> f32 {
    let (r, g, b) = (d[0], d[1], d[2]);
    let mx = r.max(g).max(b);
    let mn = r.min(g).min(b);
    if mx <= 1e-4 || r < g || g < b * 0.9 {
        return 0.0;
    }
    let sat = (mx - mn) / mx;
    // hue in degrees for r = max
    let hue = 60.0 * (g - b) / (mx - mn).max(1e-4);
    smoothstep(0.08, 0.18, sat) * (1.0 - smoothstep(0.65, 0.8, sat)) * smoothstep(0.15, 0.3, mx) * (1.0 - smoothstep(40.0, 55.0, hue))
}

/// Soft highlight compression: above the knee, values bend towards `top`.
fn roll(d: f32, amt: f32) -> f32 {
    if amt <= 1e-4 || d <= 0.6 {
        return d;
    }
    let top = 1.0 - amt * 0.4;
    let g = 1.0 + amt * 6.0;
    let x = ((d - 0.6) / 0.4).max(0.0);
    0.6 + (top - 0.6) * (1.0 - (-x * g).exp()) / (1.0 - (-g).exp())
}

/// Look Builder: Subject (skin smoothing) ▸ Color (exposure, white balance, contrast,
/// saturation, split toning) ▸ Matte (fade, highlight roll-off) ▸ Lens (diffusion, halation,
/// vignette) ▸ Film (grain).
pub fn look_builder(img: &mut Image, e: &EffectInstance, cx: &FxCtx) {
    if img.w == 0 || img.h == 0 {
        return;
    }
    let strength = (fv(e, "strength", cx) / 100.0).clamp(0.0, 1.0);
    let strength = if strength.is_finite() { strength } else { 1.0 };
    let l = scaled(look_params(e, cx), strength);
    let k = hd(img) as f32;

    // ---- Subject: edge-aware smoothing of skin tones
    if l.skin > 1e-3 {
        let blurred = soft_blur(img, 5.0 * k);
        let skin = l.skin;
        let w = img.w;
        img.px.par_chunks_mut(w * 4).enumerate().for_each(|(y, row)| {
            for x in 0..w {
                let p = &mut row[x * 4..x * 4 + 4];
                let a = p[3];
                if a <= 1e-6 {
                    continue;
                }
                let q = blurred.get(x, y);
                let d = enc(Image::unpremul([p[0], p[1], p[2], a]));
                let dq = enc(Image::unpremul(q));
                // keep edges and texture borders: only smooth where the blur barely changes luma
                let edge = 1.0 - smoothstep(0.03, 0.12, (luma(d) - luma(dq)).abs());
                let m = skin * skin_mask(d) * edge;
                for c in 0..4 {
                    p[c] += (q[c] - p[c]) * m;
                }
            }
        });
    }

    // ---- Color + Matte (+ Film grain, applied last in the same pass after the lens tools)
    let gain = l.exposure.exp2();
    let wb = [1.0 + 0.25 * l.temp, 1.0 - 0.2 * l.tint.max(0.0) + 0.1 * (-l.tint).max(0.0), 1.0 - 0.25 * l.temp];
    let color_on = (l.exposure.abs() > 1e-4)
        || l.contrast.abs() > 1e-4
        || (l.saturation - 1.0).abs() > 1e-4
        || l.temp.abs() > 1e-4
        || l.tint.abs() > 1e-4
        || l.sh_amt > 1e-4
        || l.hi_amt > 1e-4
        || l.fade > 1e-4
        || l.rolloff > 1e-4;
    if color_on {
        img.map_rgb(|c, _, _| {
            let c = [c[0] * gain * wb[0], c[1] * gain * wb[1], c[2] * gain * wb[2]];
            let mut d = enc(c);
            // contrast: S-curve around mid grey (positive) or a flatter line (negative)
            if l.contrast.abs() > 1e-4 {
                for v in &mut d {
                    let x = v.clamp(0.0, 1.0);
                    *v = if l.contrast > 0.0 {
                        let s = x * x * (3.0 - 2.0 * x);
                        x + (s - x) * l.contrast.min(1.0) * 1.2
                    } else {
                        0.5 + (x - 0.5) * (1.0 + l.contrast * 0.7)
                    };
                }
            }
            // saturation around luma
            let y = luma(d);
            for v in &mut d {
                *v = y + (*v - y) * l.saturation;
            }
            // split toning (overlay of the tint colours, weighted to shadows / highlights)
            let y = luma(d).clamp(0.0, 1.0);
            if l.sh_amt > 1e-4 {
                let ws = (1.0 - y).powi(2) * l.sh_amt;
                let t = blend_simple(4, d, l.sh_col);
                for i in 0..3 {
                    d[i] += (t[i] - d[i]) * ws;
                }
            }
            if l.hi_amt > 1e-4 {
                let wh = y * y * l.hi_amt;
                let t = blend_simple(4, d, l.hi_col);
                for i in 0..3 {
                    d[i] += (t[i] - d[i]) * wh;
                }
            }
            // matte: highlight roll-off, then lifted blacks
            for v in &mut d {
                *v = roll(v.clamp(0.0, 1.0), l.rolloff);
                *v = l.fade + *v * (1.0 - l.fade);
            }
            dec(d)
        });
    }

    // ---- Lens: diffusion (soft bloom), halation (red glow around highlights), vignette
    if l.diffusion > 1e-3 {
        let blur = soft_blur(img, l.diff_size * k * 0.5);
        let dif = l.diffusion;
        img.px.par_iter_mut().zip(blur.px.par_iter()).enumerate().for_each(|(i, (p, q))| {
            if i % 4 == 3 {
                return;
            }
            *p = *p + (q.max(*p) - *p) * dif * 0.6 + q * dif * 0.12;
        });
    }
    if l.halation > 1e-3 {
        let mut hi = highlights(img, 0.72, 0.18);
        hi = soft_blur(&hi, 9.0 * k);
        let tint = [1.0f32, 0.22, 0.06];
        let amt = l.halation * 0.9;
        let w = img.w;
        img.px.par_chunks_mut(w * 4).zip(hi.px.par_chunks(w * 4)).for_each(|(row, hrow)| {
            for x in 0..w {
                let hp = &hrow[x * 4..x * 4 + 4];
                let lum = luma([hp[0], hp[1], hp[2]]) * amt;
                if lum > 1e-5 {
                    add_light(&mut row[x * 4..x * 4 + 4], [tint[0] * lum, tint[1] * lum, tint[2] * lum]);
                }
            }
        });
    }
    if l.vignette > 1e-3 {
        let (w, h) = (img.w as f32, img.h as f32);
        let (cxp, cyp) = (w / 2.0, h / 2.0);
        let inner = l.vig_size * 0.7;
        let amt = l.vignette * 0.85;
        let wi = img.w;
        img.px.par_chunks_mut(wi * 4).enumerate().for_each(|(y, row)| {
            let dy = (y as f32 + 0.5 - cyp) / cyp;
            for x in 0..wi {
                let dx = (x as f32 + 0.5 - cxp) / cxp;
                let r = (dx * dx + dy * dy).sqrt() / std::f32::consts::SQRT_2 * 1.2;
                let m = 1.0 - amt * smoothstep(inner, inner + 0.7, r);
                let p = &mut row[x * 4..x * 4 + 4];
                p[0] *= m;
                p[1] *= m;
                p[2] *= m;
            }
        });
    }

    // ---- Film: animated grain, strongest in the midtones
    if l.grain > 1e-3 {
        let cell = (l.grain_size * k).max(0.35);
        let frame = (cx.t.0.rem_euclid(1 << 20)) as f32 * 0.618;
        let seed = seed_of(e, cx).wrapping_add(0x6772_6169);
        let amt = l.grain * 0.16;
        img.map_rgb(|c, x, y| {
            let (u, v) = (x as f32 / cell, y as f32 / cell);
            let g = noise3(u, v, frame, seed) * 0.7 + noise3(u * 2.1, v * 2.1, frame + 3.3, seed ^ 0x55) * 0.3;
            let mut d = enc(c);
            let y = luma(d).clamp(0.0, 1.0);
            let w = 1.0 - (2.0 * y - 1.0).abs() * 0.6;
            for v in &mut d {
                *v += g * amt * w;
            }
            dec(d)
        });
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use filmcraft_project::{ParamValue, find_effect};
    use filmcraft_time::Tick;

    fn cx_at(seconds: f64) -> FxCtx<'static> {
        FxCtx {
            t: Tick::from_seconds_f64(seconds),
            px_scale: 1.0,
            seconds,
            timecode: "",
            clip_name: "",
            project: None,
            env: None,
            frame_rate: 25.0,
            working: Default::default(),
        }
    }

    fn test_image() -> Image {
        let (w, h) = (96, 54);
        let mut img = Image::new(w, h);
        for y in 0..h {
            for x in 0..w {
                let p = &mut img.px[(y * w + x) * 4..(y * w + x) * 4 + 4];
                p[0] = x as f32 / w as f32;
                p[1] = y as f32 / h as f32;
                p[2] = 0.3 + 0.2 * ((x / 8 + y / 8) % 2) as f32;
                p[3] = 1.0;
            }
        }
        img
    }

    fn set(e: &mut EffectInstance, id: &str, v: ParamValue) {
        if let Some(p) = e.param_mut(id) {
            p.value = v;
        }
    }

    fn finite(img: &Image) -> bool {
        img.px.iter().all(|v| v.is_finite())
    }

    #[test]
    fn every_preset_renders_finite_pixels_and_changes_the_picture() {
        for (id, param, n) in [("organic_shake", "preset", 9u32), ("flicker_stack", "preset", 7), ("look_builder", "look", 13), ("beat_pulse", "preset", 7)] {
            for p in 1..n {
                let mut e = find_effect(id).unwrap().instance();
                set(&mut e, param, ParamValue::Choice(p));
                if id == "flicker_stack" {
                    // force a firing slot for the test
                    set(&mut e, "amount", ParamValue::Float(100.0));
                }
                let mut changed = false;
                for s in 0..24 {
                    let src = test_image();
                    let mut img = src.clone();
                    let cx = cx_at(0.13 + s as f64 * 0.37);
                    match id {
                        "organic_shake" => organic_shake(&mut img, &e, &cx),
                        "beat_pulse" => beat_pulse(&mut img, &e, &cx),
                        "flicker_stack" => flicker_stack(&mut img, &e, &cx),
                        _ => look_builder(&mut img, &e, &cx),
                    }
                    assert!(finite(&img), "{id} preset {p} produced NaN/inf");
                    assert_eq!((img.w, img.h), (src.w, src.h));
                    if img.px.iter().zip(&src.px).any(|(a, b)| (a - b).abs() > 1e-3) {
                        changed = true;
                    }
                }
                assert!(changed, "{id} preset {p} never changed the picture");
            }
        }
    }

    #[test]
    fn zero_intensity_and_strength_are_identity() {
        let src = test_image();
        let mut e = find_effect("organic_shake").unwrap().instance();
        set(&mut e, "intensity", ParamValue::Float(0.0));
        let mut img = src.clone();
        organic_shake(&mut img, &e, &cx_at(1.0));
        assert_eq!(img.px, src.px);

        let mut e = find_effect("flicker_stack").unwrap().instance();
        set(&mut e, "amount", ParamValue::Float(0.0));
        let mut img = src.clone();
        flicker_stack(&mut img, &e, &cx_at(1.0));
        assert_eq!(img.px, src.px);

        let mut e = find_effect("look_builder").unwrap().instance();
        set(&mut e, "strength", ParamValue::Float(0.0));
        let mut img = src.clone();
        look_builder(&mut img, &e, &cx_at(1.0));
        assert!(img.px.iter().zip(&src.px).all(|(a, b)| (a - b).abs() < 1e-4));
    }

    #[test]
    fn hostile_parameters_do_not_crash() {
        for id in ["organic_shake", "flicker_stack", "look_builder", "beat_pulse"] {
            let def = find_effect(id).unwrap();
            for bad in [f64::NAN, f64::INFINITY, -1e12, 1e12] {
                let mut e = def.instance();
                set(&mut e, "preset", ParamValue::Choice(0));
                set(&mut e, "look", ParamValue::Choice(99));
                set(&mut e, "sync", ParamValue::Choice(if bad.is_nan() { 3 } else { 1 }));
                for p in &def.params {
                    if matches!(p.default, ParamValue::Float(_)) {
                        set(&mut e, p.id, ParamValue::Float(bad));
                    }
                }
                for img in [test_image(), Image::new(0, 0), Image::new(1, 1)] {
                    let mut img = img;
                    match id {
                        "organic_shake" => organic_shake(&mut img, &e, &cx_at(f64::NAN)),
                        "flicker_stack" => flicker_stack(&mut img, &e, &cx_at(2.0)),
                        "beat_pulse" => beat_pulse(&mut img, &e, &cx_at(2.0)),
                        _ => look_builder(&mut img, &e, &cx_at(2.0)),
                    }
                }
            }
        }
    }

    #[test]
    fn bilinear_sampling_far_outside_is_clear() {
        // panicked with "attempt to add with overflow" before the saturating fix in image.rs
        let img = Image::filled(4, 4, [1.0; 4]);
        for (x, y) in [(f32::MAX, 1.0), (1.0, f32::MAX), (f32::INFINITY, f32::INFINITY), (-f32::MAX, 2.0)] {
            assert_eq!(img.sample_bilinear(x, y), [0.0; 4]);
        }
    }

    fn diff(a: &Image, b: &Image) -> f32 {
        a.px.iter().zip(&b.px).map(|(x, y)| (x - y).abs()).sum::<f32>() / a.px.len().max(1) as f32
    }

    #[test]
    fn beat_pulse_hits_on_the_beat_and_releases_before_the_next() {
        let src = test_image();
        let mut e = find_effect("beat_pulse").unwrap().instance();
        set(&mut e, "bpm", ParamValue::Float(120.0)); // a beat every 0.5 s
        set(&mut e, "beat_offset", ParamValue::Float(0.1));
        let at = |t: f64| {
            let mut img = src.clone();
            beat_pulse(&mut img, &e, &cx_at(t));
            diff(&img, &src)
        };
        let hit = at(0.1 + 0.5 * 3.0 + 0.02);
        let tail = at(0.1 + 0.5 * 3.0 + 0.48);
        assert!(hit > 0.01, "no pulse on the beat ({hit})");
        assert!(tail < hit * 0.25, "pulse did not release ({tail} vs {hit})");
    }

    #[test]
    fn synced_shake_is_strongest_right_after_the_beat() {
        let src = test_image();
        let mut e = find_effect("organic_shake").unwrap().instance();
        set(&mut e, "preset", ParamValue::Choice(5)); // Earthquake
        set(&mut e, "sync", ParamValue::Choice(1));
        set(&mut e, "bpm", ParamValue::Float(60.0));
        set(&mut e, "motion_blur", ParamValue::Float(0.0));
        set(&mut e, "auto_scale", ParamValue::Bool(false));
        let mut avg = [0.0f32; 2];
        for b in 0..8 {
            for (i, dt) in [0.05, 0.9].iter().enumerate() {
                let mut img = src.clone();
                organic_shake(&mut img, &e, &cx_at(b as f64 + dt));
                avg[i] += diff(&img, &src);
            }
        }
        assert!(avg[0] > avg[1] * 2.0, "shake should die away between beats: {avg:?}");
    }

    #[test]
    fn impact_decays_to_still() {
        let src = test_image();
        let mut e = find_effect("organic_shake").unwrap().instance();
        set(&mut e, "preset", ParamValue::Choice(6)); // Impact Hit
        set(&mut e, "auto_scale", ParamValue::Bool(false));
        set(&mut e, "motion_blur", ParamValue::Float(0.0));
        let mut img = src.clone();
        organic_shake(&mut img, &e, &cx_at(30.0));
        assert!(img.px.iter().zip(&src.px).all(|(a, b)| (a - b).abs() < 1e-3), "shake should have decayed after 30 s");
    }
}
