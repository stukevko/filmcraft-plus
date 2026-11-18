//! FilmCraft Plus animation effects (definitions in `filmcraft_project::effect::plus`):
//!
//! - **Draw On** (`draw_on`): a sketch, a logo or any line art draws itself. The lines of the
//!   picture are found (by transparency, or as dark lines on light paper / light lines on dark),
//!   every line pixel gets the moment it is "drawn" — how far it lies along its line from where
//!   the line starts — and the picture is revealed in that order. The paper can be removed, so a
//!   photographed pencil sketch draws itself over the video.
//! - **Marker** (`marker`): an arrow, circle, underline, box, cross or tick that is drawn by
//!   hand over the picture, to point at something.
//!
//! Both run on the clip's own time (they start with the clip) and can take themselves away
//! again before the clip ends.
//!
//! Original FilmCraft algorithms. Images are premultiplied linear f32; colour parameters are
//! display-encoded.

use std::collections::VecDeque;

use filmcraft_geom::Vec2;
use filmcraft_project::EffectInstance;
use rayon::prelude::*;

use super::plus::hd;
use super::{FxCtx, Image, bv, chv, cv, dec, enc, fv, hash01, luma, pv, smoothstep};

fn finite_or(v: f32, d: f32) -> f32 {
    if v.is_finite() { v } else { d }
}

/// How much is drawn (0..1) and how much is taken away again (0..1) at the clip's current time.
fn progress(e: &EffectInstance, cx: &FxCtx) -> (f32, f32) {
    let now = if cx.seconds.is_finite() { cx.seconds.max(0.0) } else { 0.0 };
    let delay = f64::from(finite_or(fv(e, "delay", cx), 0.0).clamp(0.0, 3600.0));
    let dur = f64::from(finite_or(fv(e, "duration", cx), 1.0).clamp(0.0, 3600.0));
    let drawn = if dur <= 1e-6 { if now >= delay { 1.0 } else { 0.0 } } else { ((now - delay) / dur).clamp(0.0, 1.0) };
    let mut erased = 0.0;
    if bv(e, "erase")
        && let Some(env) = cx.env
    {
        let len = env.clip_seconds();
        let out = dur.min(len / 2.0).max(1e-3);
        if len.is_finite() && len > 0.0 {
            erased = (1.0 - (len - now) / out).clamp(0.0, 1.0);
        }
    }
    (drawn as f32, erased as f32)
}

// ------------------------------------------------------------------ Draw On

/// Longest side of the picture the drawing order is worked out on.
const WORK: usize = 640;

#[derive(Clone, Copy, PartialEq)]
enum Ink {
    Alpha,
    Dark,
    Light,
}

/// The order a picture's lines are drawn in.
struct Order {
    w: usize,
    h: usize,
    /// Per working pixel: when it is drawn, 0..1 (1 for paper).
    time: Vec<f32>,
}

impl Order {
    /// The drawing time at a position given as fractions of the picture (bilinear).
    fn at(&self, u: f32, v: f32) -> f32 {
        if self.w == 0 || self.h == 0 {
            return 0.0;
        }
        let (x, y) = ((u * self.w as f32 - 0.5).clamp(0.0, (self.w - 1) as f32), (v * self.h as f32 - 0.5).clamp(0.0, (self.h - 1) as f32));
        let (x0, y0) = (x as usize, y as usize);
        let (x1, y1) = ((x0 + 1).min(self.w - 1), (y0 + 1).min(self.h - 1));
        let (fx, fy) = (x - x0 as f32, y - y0 as f32);
        let g = |x: usize, y: usize| self.time.get(y * self.w + x).copied().unwrap_or(1.0);
        // lines are thin: the earlier of the neighbours, so a line is never held back by paper
        let top = g(x0, y0).min(g(x1, y0)) * (1.0 - fy) + g(x0, y1).min(g(x1, y1)) * fy;
        let left = g(x0, y0).min(g(x0, y1)) * (1.0 - fx) + g(x1, y0).min(g(x1, y1)) * fx;
        top.min(left)
    }
}

/// Work out the drawing order of a `w`×`h` ink mask (`true` = line). `direction`: 0 left to
/// right, 1 top to bottom, 2 from the middle outwards.
fn order(mask: &[bool], w: usize, h: usize, direction: u32) -> Order {
    let n = w * h;
    let mut time = vec![1.0f32; n];
    if n == 0 || mask.len() < n {
        return Order { w, h, time };
    }
    // where a line "starts": smaller is earlier
    let key = |i: usize| -> f32 {
        let (x, y) = ((i % w) as f32 / w as f32, (i / w) as f32 / h as f32);
        match direction {
            1 => y + x * 0.05,
            2 => (x - 0.5).hypot(y - 0.5),
            _ => x + y * 0.05,
        }
    };
    // pass 1: the connected lines, each with its starting pixel
    let mut label = vec![u32::MAX; n];
    let mut lines: Vec<(usize, f32)> = Vec::new();
    let mut queue: VecDeque<usize> = VecDeque::new();
    let neighbours = |i: usize, out: &mut [usize; 8]| -> usize {
        let (x, y) = ((i % w) as isize, (i / w) as isize);
        let mut k = 0;
        for (dx, dy) in [(-1, 0), (1, 0), (0, -1), (0, 1), (-1, -1), (1, -1), (-1, 1), (1, 1)] {
            let (nx, ny) = (x + dx, y + dy);
            if nx >= 0 && ny >= 0 && (nx as usize) < w && (ny as usize) < h {
                out[k] = ny as usize * w + nx as usize;
                k += 1;
            }
        }
        k
    };
    let mut near = [0usize; 8];
    for start in 0..n {
        if !mask[start] || label[start] != u32::MAX {
            continue;
        }
        let id = lines.len() as u32;
        let (mut seed, mut best) = (start, key(start));
        label[start] = id;
        queue.push_back(start);
        while let Some(i) = queue.pop_front() {
            if key(i) < best {
                best = key(i);
                seed = i;
            }
            let k = neighbours(i, &mut near);
            for &j in &near[..k] {
                if mask[j] && label[j] == u32::MAX {
                    label[j] = id;
                    queue.push_back(j);
                }
            }
        }
        lines.push((seed, best));
    }
    if lines.is_empty() {
        return Order { w, h, time };
    }
    // pass 2: along every line from its start, in the order the lines start
    let mut by_start: Vec<usize> = (0..lines.len()).collect();
    by_start.sort_by(|a, b| lines[*a].1.total_cmp(&lines[*b].1));
    let mut dist = vec![u32::MAX; n];
    let mut spans: Vec<(u32, f32, f32)> = vec![(0, 0.0, 0.0); lines.len()];
    let mut total = 0.0f32;
    for &l in &by_start {
        let seed = lines[l].0;
        dist[seed] = 0;
        queue.push_back(seed);
        let mut far = 0u32;
        while let Some(i) = queue.pop_front() {
            let d = dist[i];
            far = far.max(d);
            let k = neighbours(i, &mut near);
            for &j in &near[..k] {
                if mask[j] && dist[j] == u32::MAX {
                    dist[j] = d + 1;
                    queue.push_back(j);
                }
            }
        }
        // a dot still takes a moment; a small pause between two lines
        let len = far as f32 + 4.0;
        spans[l] = (far, total, len);
        total += len + 2.0;
    }
    for i in 0..n {
        if mask[i] && dist[i] != u32::MAX {
            let (_, from, _) = spans[label[i] as usize];
            time[i] = ((from + dist[i] as f32) / total.max(1.0)).clamp(0.0, 1.0);
        }
    }
    // the soft edge around a line is drawn with the line
    for _ in 0..3 {
        let prev = time.clone();
        let assigned: Vec<bool> = prev.iter().zip(mask).map(|(t, m)| *m || *t < 1.0).collect();
        for i in 0..n {
            if assigned[i] {
                continue;
            }
            let k = neighbours(i, &mut near);
            let t = near[..k].iter().filter(|j| assigned[**j]).map(|j| prev[*j]).fold(1.0f32, f32::min);
            time[i] = t;
        }
    }
    Order { w, h, time }
}

/// Draw On: the picture's lines draw themselves.
pub fn draw_on(img: &mut Image, e: &EffectInstance, cx: &FxCtx) {
    if img.w < 2 || img.h < 2 {
        return;
    }
    let (drawn, erased) = progress(e, cx);
    let remove_paper = bv(e, "remove_paper");
    let (w, h) = (img.w, img.h);
    // ---- what is line, what is paper
    let straight = |p: &[f32]| -> ([f32; 3], f32) {
        let a = p[3];
        if a > 1e-5 { ([p[0] / a, p[1] / a, p[2] / a], a) } else { ([0.0; 3], 0.0) }
    };
    let clear = img.px.chunks(4).step_by(7).filter(|p| p[3] < 0.5).count() * 7;
    let mut border: Vec<f32> = Vec::new();
    for x in (0..w).step_by((w / 64).max(1)) {
        for y in [0, h - 1] {
            border.push(luma(enc(straight(&img.px[(y * w + x) * 4..(y * w + x) * 4 + 4]).0)));
        }
    }
    for y in (0..h).step_by((h / 64).max(1)) {
        for x in [0, w - 1] {
            border.push(luma(enc(straight(&img.px[(y * w + x) * 4..(y * w + x) * 4 + 4]).0)));
        }
    }
    border.sort_by(f32::total_cmp);
    let paper = border.get(border.len() / 2).copied().filter(|v| v.is_finite()).unwrap_or(1.0);
    let kind = match chv(e, "ink") {
        1 => Ink::Alpha,
        2 => Ink::Dark,
        3 => Ink::Light,
        _ if clear * 50 > w * h => Ink::Alpha,
        _ if paper >= 0.5 => Ink::Dark,
        _ => Ink::Light,
    };
    // 0 = only the strongest lines … 1 = the faintest pencil
    let sens = (finite_or(fv(e, "sensitivity", cx), 50.0) / 100.0).clamp(0.0, 1.0);
    let (lo, hi) = (0.30 - 0.26 * sens, 0.55 - 0.35 * sens);
    let ink_of = move |p: &[f32]| -> f32 {
        let (c, a) = straight(p);
        match kind {
            Ink::Alpha => a,
            Ink::Dark => a * smoothstep(lo, hi, paper - luma(enc(c))),
            Ink::Light => a * smoothstep(lo, hi, luma(enc(c)) - paper),
        }
    };
    // ---- the drawing order, on a small copy
    let k = (WORK as f32 / w.max(h) as f32).min(1.0);
    let (mw, mh) = (((w as f32 * k) as usize).max(1), ((h as f32 * k) as usize).max(1));
    let mask: Vec<bool> = (0..mw * mh)
        .into_par_iter()
        .map(|i| {
            let (x, y) = (((i % mw) * w / mw).min(w - 1), ((i / mw) * h / mh).min(h - 1));
            // the strongest of a few samples, so thin lines survive the shrinking
            let step = ((w / mw).max(1) / 2).max(1);
            let mut best = 0.0f32;
            for (dx, dy) in [(0, 0), (step, 0), (0, step), (step, step)] {
                let (sx, sy) = ((x + dx).min(w - 1), (y + dy).min(h - 1));
                best = best.max(ink_of(&img.px[(sy * w + sx) * 4..(sy * w + sx) * 4 + 4]));
            }
            best > 0.35
        })
        .collect();
    let ord = order(&mask, mw, mh, chv(e, "direction"));
    // ---- reveal
    let soft = 0.004 + 0.05 * (finite_or(fv(e, "softness", cx), 30.0) / 100.0).clamp(0.0, 1.0);
    let head = drawn * (1.0 + soft);
    let tail = erased * (1.0 + soft);
    let tint = bv(e, "tint").then(|| {
        let c = cv(e, "color", cx);
        dec([c[0], c[1], c[2]])
    });
    let paper_lin = dec([paper, paper, paper]);
    img.px.par_chunks_mut(w * 4).enumerate().for_each(|(y, row)| {
        for x in 0..w {
            let p = &mut row[x * 4..x * 4 + 4];
            let ink = ink_of(p);
            let t = ord.at((x as f32 + 0.5) / w as f32, (y as f32 + 0.5) / h as f32);
            // drawn once the pen has passed, gone once the eraser has
            let show = (1.0 - smoothstep(head - soft, head, t)) * smoothstep(tail - soft, tail, t + 1e-6).max(if erased <= 0.0 { 1.0 } else { 0.0 });
            let (c, a) = straight(p);
            let c = tint.unwrap_or(c);
            let out = if kind == Ink::Alpha || remove_paper {
                // only the line, over whatever is underneath
                let a = if kind == Ink::Alpha { a } else { ink } * show;
                [c[0] * a, c[1] * a, c[2] * a, a]
            } else {
                // the paper stays; the line appears on it
                let m = ink * show;
                let line = if tint.is_some() { c } else { straight(p).0 };
                let c = [paper_lin[0] + (line[0] - paper_lin[0]) * m, paper_lin[1] + (line[1] - paper_lin[1]) * m, paper_lin[2] + (line[2] - paper_lin[2]) * m];
                [c[0] * a, c[1] * a, c[2] * a, a]
            };
            for (d, s) in p.iter_mut().zip(out) {
                *d = if s.is_finite() { s.max(0.0) } else { 0.0 };
            }
        }
    });
}

// ------------------------------------------------------------------ Marker

/// A polyline with the pen's thickness at each point (a fraction of the full thickness).
type Stroke = Vec<(Vec2, f32)>;

/// Smooth wobble in −1..1 along a stroke.
fn wobble(seed: u64, at: f32) -> f32 {
    let (i, f) = (at.floor(), at - at.floor());
    let v = |k: f32| hash01(k as i64, 0, 0, seed) * 2.0 - 1.0;
    let s = f * f * (3.0 - 2.0 * f);
    v(i) + (v(i + 1.0) - v(i)) * s
}

/// `pts` resampled into a hand-drawn stroke: slightly unsteady, thinner at both ends.
fn hand(pts: &[Vec2], shake: f32, seed: u64, closed: bool) -> Stroke {
    let mut out = Stroke::new();
    let total: f64 = pts.windows(2).map(|w| (w[1].x - w[0].x).hypot(w[1].y - w[0].y)).sum();
    if pts.len() < 2 || !total.is_finite() || total <= 1e-6 {
        return out;
    }
    let steps = ((total / 6.0) as usize).clamp(8, 400);
    let mut run = 0.0f64;
    for w in pts.windows(2) {
        let (a, b) = (w[0], w[1]);
        let len = (b.x - a.x).hypot(b.y - a.y);
        if len <= 1e-9 {
            continue;
        }
        let (nx, ny) = (-(b.y - a.y) / len, (b.x - a.x) / len);
        let n = ((steps as f64 * len / total).ceil() as usize).max(1);
        for i in 0..=n {
            let f = i as f64 / n as f64;
            let s = ((run + len * f) / total) as f32;
            let off = f64::from(wobble(seed, s * 7.0) * shake);
            // a pen lands and lifts: thinner at the very ends (not where a closed shape meets)
            let taper = if closed { 1.0 } else { 0.55 + 0.45 * smoothstep(0.0, 0.08, s).min(smoothstep(0.0, 0.12, 1.0 - s)) };
            out.push((Vec2::new(a.x + (b.x - a.x) * f + nx * off, a.y + (b.y - a.y) * f + ny * off), taper));
        }
        run += len;
    }
    out
}

/// The strokes of `shape` between the points `a` and `b` (layer pixels), in drawing order.
fn strokes(shape: u32, a: Vec2, b: Vec2, bend: f32, shake: f32, seed: u64) -> Vec<Stroke> {
    let (dx, dy) = (b.x - a.x, b.y - a.y);
    let len = dx.hypot(dy);
    if !len.is_finite() || len < 1.0 {
        return Vec::new();
    }
    let (ux, uy) = (dx / len, dy / len);
    let (nx, ny) = (-uy, ux);
    // a curved line from a to b (the bend pushes its middle sideways)
    let curve = |from: Vec2, to: Vec2, bend: f64| -> Vec<Vec2> {
        (0..=24)
            .map(|i| {
                let t = f64::from(i) / 24.0;
                let side = 4.0 * t * (1.0 - t) * bend;
                Vec2::new(from.x + (to.x - from.x) * t + nx * side, from.y + (to.y - from.y) * t + ny * side)
            })
            .collect()
    };
    let bend = f64::from(bend) * len * 0.25;
    let (x0, x1, y0, y1) = (a.x.min(b.x), a.x.max(b.x), a.y.min(b.y), a.y.max(b.y));
    let (cx, cy, rx, ry) = ((x0 + x1) / 2.0, (y0 + y1) / 2.0, ((x1 - x0) / 2.0).max(4.0), ((y1 - y0) / 2.0).max(4.0));
    match shape {
        // Arrow: the shaft, then the two strokes of the head
        0 => {
            let shaft = curve(a, b, bend);
            // the head follows the end of the (bent) shaft
            let before = shaft.get(shaft.len().saturating_sub(4)).copied().unwrap_or(a);
            let (hx, hy) = (b.x - before.x, b.y - before.y);
            let hl = hx.hypot(hy).max(1e-6);
            let (hx, hy) = (hx / hl, hy / hl);
            let size = (len * 0.22).clamp(12.0, 220.0);
            let wing = |sign: f64| {
                let (c, s) = (0.5f64.cos(), 0.5f64.sin() * sign);
                let (wx, wy) = (-(hx * c - hy * s), -(hx * s + hy * c));
                vec![Vec2::new(b.x + wx * size, b.y + wy * size), b]
            };
            vec![hand(&shaft, shake, seed, false), hand(&wing(1.0), shake * 0.4, seed ^ 1, false), hand(&wing(-1.0), shake * 0.4, seed ^ 2, false)]
        }
        // Circle: a little more than once round, not quite closing on itself
        1 => {
            let pts: Vec<Vec2> = (0..=64)
                .map(|i| {
                    let t = f64::from(i) / 64.0;
                    let ang = -2.2 + t * std::f64::consts::TAU * 1.07;
                    let r = 1.0 + 0.05 * t;
                    Vec2::new(cx + rx * 1.15 * r * ang.cos(), cy + ry * 1.25 * r * ang.sin())
                })
                .collect();
            vec![hand(&pts, shake, seed, false)]
        }
        // Underline: one sweep, a little bowed
        2 => vec![hand(&curve(a, b, bend), shake, seed, false)],
        // Box: four sides in one go, overshooting the first corner a little
        3 => {
            let o = ((x1 - x0).min(y1 - y0) * 0.04).min(12.0);
            let pts = [Vec2::new(x0 - o, y0), Vec2::new(x1, y0), Vec2::new(x1, y1), Vec2::new(x0, y1), Vec2::new(x0, y0 - o)];
            vec![hand(&pts, shake, seed, false)]
        }
        // Cross: two strokes
        4 => vec![hand(&[Vec2::new(x0, y0), Vec2::new(x1, y1)], shake, seed, false), hand(&[Vec2::new(x1, y0), Vec2::new(x0, y1)], shake, seed ^ 3, false)],
        // Tick: down to the low point, then the long stroke up
        _ => {
            let pts = [Vec2::new(x0, cy + ry * 0.1), Vec2::new(x0 + (x1 - x0) * 0.36, y1), Vec2::new(x1, y0)];
            vec![hand(&pts, shake, seed, false)]
        }
    }
}

/// Marker: a hand-drawn arrow, circle, underline, box, cross or tick.
pub fn marker(img: &mut Image, e: &EffectInstance, cx: &FxCtx) {
    if img.w < 2 || img.h < 2 {
        return;
    }
    let (drawn, erased) = progress(e, cx);
    let opacity = (finite_or(fv(e, "opacity", cx), 100.0) / 100.0).clamp(0.0, 1.0);
    if drawn <= 0.0 || erased >= 1.0 || opacity <= 0.0 {
        return;
    }
    let k = hd(img) as f32;
    let (w, h) = (img.w, img.h);
    let limit = |p: Vec2| {
        Vec2::new(
            if p.x.is_finite() { p.x.clamp(-4.0 * w as f64, 5.0 * w as f64) } else { w as f64 / 2.0 },
            if p.y.is_finite() { p.y.clamp(-4.0 * h as f64, 5.0 * h as f64) } else { h as f64 / 2.0 },
        )
    };
    let (a, b) = (limit(pv(e, "from", cx, img)), limit(pv(e, "to", cx, img)));
    let radius = (finite_or(fv(e, "thickness", cx), 10.0).clamp(0.5, 200.0) * k / 2.0).max(0.4);
    let shake = (finite_or(fv(e, "wobble", cx), 40.0) / 100.0).clamp(0.0, 1.0) * radius * 1.6;
    let bend = (finite_or(fv(e, "curve", cx), 0.0) / 100.0).clamp(-1.0, 1.0);
    let seed = (finite_or(fv(e, "seed", cx), 1.0).clamp(0.0, 1.0e6) as u64).wrapping_mul(0x9e37_79b9);
    let all = strokes(chv(e, "shape"), a, b, bend, shake, seed);
    let seg_len = |s: &Stroke| -> f64 { s.windows(2).map(|w| (w[1].0.x - w[0].0.x).hypot(w[1].0.y - w[0].0.y)).sum() };
    let total: f64 = all.iter().map(seg_len).sum();
    if !(total.is_finite() && total > 0.0) {
        return;
    }
    // the part of the whole path that shows: from where the eraser is to where the pen is
    let (from, to) = (f64::from(erased) * total, f64::from(drawn) * total);
    let mut cover = vec![0.0f32; w * h];
    let mut run = 0.0f64;
    for s in &all {
        for seg in s.windows(2) {
            let ((p, tp), (q, tq)) = (seg[0], seg[1]);
            let len = (q.x - p.x).hypot(q.y - p.y);
            let (s0, s1) = (run, run + len);
            run = s1;
            if len <= 1e-9 || s1 <= from || s0 >= to {
                continue;
            }
            // cut the segment at the pen and at the eraser
            let (f0, f1) = (((from - s0) / len).clamp(0.0, 1.0), ((to - s0) / len).clamp(0.0, 1.0));
            let (p0, p1) = (Vec2::new(p.x + (q.x - p.x) * f0, p.y + (q.y - p.y) * f0), Vec2::new(p.x + (q.x - p.x) * f1, p.y + (q.y - p.y) * f1));
            let r = radius * (tp + (tq - tp) * 0.5);
            stamp(&mut cover, w, h, p0, p1, r);
        }
    }
    let c = cv(e, "color", cx);
    let lin = dec([c[0], c[1], c[2]]);
    img.px.par_chunks_mut(4).zip(cover.par_iter()).for_each(|(p, m)| {
        let m = (m * opacity).clamp(0.0, 1.0);
        if m > 0.0 {
            for i in 0..3 {
                p[i] = p[i] * (1.0 - m) + lin[i] * m;
            }
            p[3] = p[3] + (1.0 - p[3]) * m;
        }
    });
}

/// Add a round-ended line from `a` to `b` of radius `r` to a coverage mask (the strongest wins).
fn stamp(cover: &mut [f32], w: usize, h: usize, a: Vec2, b: Vec2, r: f32) {
    let pad = f64::from(r) + 1.5;
    let (x0, x1) = (((a.x.min(b.x) - pad).floor().max(0.0)) as usize, ((a.x.max(b.x) + pad).ceil().min(w as f64 - 1.0)).max(0.0) as usize);
    let (y0, y1) = (((a.y.min(b.y) - pad).floor().max(0.0)) as usize, ((a.y.max(b.y) + pad).ceil().min(h as f64 - 1.0)).max(0.0) as usize);
    if a.x.max(b.x) + pad < 0.0 || a.y.max(b.y) + pad < 0.0 || x0 >= w || y0 >= h {
        return;
    }
    let (dx, dy) = (b.x - a.x, b.y - a.y);
    let len2 = (dx * dx + dy * dy).max(1e-12);
    for y in y0..=y1.min(h - 1) {
        for x in x0..=x1.min(w - 1) {
            let (px, py) = (x as f64 + 0.5 - a.x, y as f64 + 0.5 - a.y);
            let t = ((px * dx + py * dy) / len2).clamp(0.0, 1.0);
            let d = (px - dx * t).hypot(py - dy * t) as f32;
            let c = (r + 0.5 - d).clamp(0.0, 1.0);
            if let Some(m) = cover.get_mut(y * w + x)
                && c > *m
            {
                *m = c;
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use filmcraft_project::{ParamValue, find_effect};
    use filmcraft_time::Tick;

    fn cx(seconds: f64) -> FxCtx<'static> {
        FxCtx { t: Tick::ZERO, px_scale: 1.0, seconds, timecode: "", clip_name: "", project: None, env: None, frame_rate: 25.0, working: Default::default() }
    }

    fn set(e: &mut EffectInstance, id: &str, v: ParamValue) {
        e.param_mut(id).unwrap_or_else(|| panic!("no parameter {id}")).value = v;
    }

    /// White paper with a dark horizontal line from x=20 to x=180 at y=50 and a separate dot.
    fn sketch() -> Image {
        let (w, h) = (200usize, 100usize);
        let mut img = Image::new(w, h);
        for p in img.px.chunks_mut(4) {
            p.copy_from_slice(&[0.9, 0.9, 0.9, 1.0]);
        }
        for y in 48..53 {
            for x in 20..180 {
                img.px[(y * w + x) * 4..(y * w + x) * 4 + 4].copy_from_slice(&[0.02, 0.02, 0.02, 1.0]);
            }
        }
        for y in 80..86 {
            for x in 185..191 {
                img.px[(y * w + x) * 4..(y * w + x) * 4 + 4].copy_from_slice(&[0.02, 0.02, 0.02, 1.0]);
            }
        }
        img
    }

    #[test]
    fn the_order_runs_along_a_line_and_takes_lines_one_after_another() {
        // an L: right along the top row, then down the last column; and a separate dot far right
        let (w, h) = (40usize, 20usize);
        let mut mask = vec![false; w * h];
        for x in 2..30 {
            mask[2 * w + x] = true;
        }
        for y in 2..18 {
            mask[y * w + 29] = true;
        }
        mask[10 * w + 38] = true;
        let o = order(&mask, w, h, 0);
        let t = |x: usize, y: usize| o.time[y * w + x];
        assert!(t(2, 2) < t(15, 2) && t(15, 2) < t(29, 2), "along the top");
        assert!(t(29, 2) < t(29, 10) && t(29, 10) < t(29, 17), "then down: the order follows the line, not the x position");
        assert!(t(29, 17) < t(38, 10), "the dot comes after the L is finished");
        assert!(o.time.iter().all(|v| (0.0..=1.0).contains(v)));
        // top to bottom starts the L at its top-left as well, the middle-out order somewhere else
        let mid = order(&mask, w, h, 2);
        assert!(mid.time[2 * w + 2] > mid.time[2 * w + 20]);
        // nothing to draw, and hostile sizes
        assert!(order(&[false; 12], 4, 3, 0).time.iter().all(|v| *v == 1.0));
        assert!(order(&[], 0, 0, 0).time.is_empty());
        assert_eq!(order(&[true], 4, 3, 0).time.len(), 12);
    }

    #[test]
    fn a_sketch_draws_itself_from_left_to_right() {
        let src = sketch();
        let mut e = find_effect("draw_on").expect("effect").instance();
        set(&mut e, "duration", ParamValue::Float(2.0));
        let at = |e: &EffectInstance, secs: f64| {
            let mut img = src.clone();
            draw_on(&mut img, e, &cx(secs));
            assert!(img.px.iter().all(|v| v.is_finite() && *v >= 0.0));
            img
        };
        let dark = |img: &Image, x: usize, y: usize| img.get(x, y)[0] < 0.3;
        // the paper stays, the line is not there yet
        let start = at(&e, 0.0);
        assert!(!dark(&start, 30, 50) && !dark(&start, 170, 50) && start.get(5, 5)[0] > 0.8);
        // half way: the left half of the line is drawn, the right half and the dot are not
        let half = at(&e, 1.0);
        assert!(dark(&half, 30, 50) && !dark(&half, 170, 50) && !dark(&half, 187, 82), "half way");
        // at the end the picture is the original
        let done = at(&e, 2.5);
        assert!(dark(&done, 30, 50) && dark(&done, 170, 50) && dark(&done, 187, 82));
        let worst = done.px.iter().zip(&src.px).map(|(a, b)| (a - b).abs()).fold(0.0f32, f32::max);
        assert!(worst < 0.05, "the finished drawing is the picture: {worst}");
        // without the paper only the line is left, over a transparent background
        set(&mut e, "remove_paper", ParamValue::Bool(true));
        let lines = at(&e, 2.5);
        assert!(lines.get(5, 5)[3] < 0.02 && lines.get(100, 50)[3] > 0.9);
        // …in another colour when asked
        set(&mut e, "tint", ParamValue::Bool(true));
        set(&mut e, "color", ParamValue::Color([1.0, 0.0, 0.0, 1.0]));
        let red = at(&e, 2.5).get(100, 50);
        assert!(red[0] > 0.8 && red[1] < 0.1, "{red:?}");
    }

    #[test]
    fn transparent_art_and_light_lines_are_understood() {
        // a logo with transparency: revealed by its alpha
        let (w, h) = (120usize, 60usize);
        let mut logo = Image::new(w, h);
        for y in 20..40 {
            for x in 10..110 {
                logo.px[(y * w + x) * 4..(y * w + x) * 4 + 4].copy_from_slice(&[0.2, 0.6, 0.9, 1.0]);
            }
        }
        let mut e = find_effect("draw_on").expect("effect").instance();
        set(&mut e, "duration", ParamValue::Float(1.0));
        let mut half = logo.clone();
        draw_on(&mut half, &e, &cx(0.5));
        assert!(half.get(15, 30)[3] > 0.9 && half.get(105, 30)[3] < 0.1, "left drawn, right not yet");
        let mut done = logo.clone();
        draw_on(&mut done, &e, &cx(1.5));
        assert!(done.px.iter().zip(&logo.px).all(|(a, b)| (a - b).abs() < 0.02));
        // chalk on a blackboard: light lines on dark
        let mut board = Image::new(w, h);
        for p in board.px.chunks_mut(4) {
            p.copy_from_slice(&[0.01, 0.01, 0.01, 1.0]);
        }
        for x in 10..110 {
            for y in 29..32 {
                board.px[(y * w + x) * 4..(y * w + x) * 4 + 4].copy_from_slice(&[0.9, 0.9, 0.9, 1.0]);
            }
        }
        let mut half = board.clone();
        draw_on(&mut half, &e, &cx(0.5));
        assert!(half.get(15, 30)[0] > 0.5 && half.get(105, 30)[0] < 0.2);
    }

    #[test]
    fn every_marker_is_drawn_along_its_path_and_keeps_the_picture() {
        let mut src = Image::new(320, 180);
        for p in src.px.chunks_mut(4) {
            p.copy_from_slice(&[0.1, 0.2, 0.1, 1.0]);
        }
        let n = filmcraft_project::effect::MARKER_SHAPES.len() as u32;
        for shape in 0..n {
            let mut e = find_effect("marker").expect("effect").instance();
            set(&mut e, "shape", ParamValue::Choice(shape));
            set(&mut e, "from", ParamValue::Vec2(Vec2::new(60.0, 120.0)));
            set(&mut e, "to", ParamValue::Vec2(Vec2::new(260.0, 50.0)));
            set(&mut e, "duration", ParamValue::Float(1.0));
            let changed = |secs: f64| {
                let mut img = src.clone();
                marker(&mut img, &e, &cx(secs));
                assert!(img.px.iter().all(|v| v.is_finite()), "shape {shape}");
                assert!(img.px.chunks(4).all(|p| (p[3] - 1.0).abs() < 1e-4), "shape {shape}: opaque stays opaque");
                img.px.chunks(4).zip(src.px.chunks(4)).filter(|(a, b)| (a[0] - b[0]).abs() > 0.02).count()
            };
            let (none, some, all) = (changed(0.0), changed(0.5), changed(1.5));
            assert_eq!(none, 0, "shape {shape}: nothing before the pen moves");
            assert!(some > 20 && some < all, "shape {shape}: {some} of {all} half way");
            assert!(all < 320 * 180 / 3, "shape {shape}: a line, not a fill");
        }
        // an arrow ends at its tip: the tip is painted, the far side beyond it is not
        let mut e = find_effect("marker").expect("effect").instance();
        set(&mut e, "from", ParamValue::Vec2(Vec2::new(40.0, 90.0)));
        set(&mut e, "to", ParamValue::Vec2(Vec2::new(240.0, 90.0)));
        set(&mut e, "wobble", ParamValue::Float(0.0));
        set(&mut e, "curve", ParamValue::Float(0.0));
        let mut img = src.clone();
        marker(&mut img, &e, &cx(10.0));
        assert!((img.get(236, 90)[0] - src.get(236, 90)[0]).abs() > 0.1, "the tip");
        assert!((img.get(140, 90)[0] - src.get(140, 90)[0]).abs() > 0.1, "the shaft");
        assert!((img.get(270, 90)[0] - src.get(270, 90)[0]).abs() < 1e-4, "nothing past the tip");
        assert!((img.get(210, 70)[0] - src.get(210, 70)[0]).abs() > 0.1 || (img.get(214, 74)[0] - src.get(214, 74)[0]).abs() > 0.1, "the head opens backwards");
    }

    #[test]
    fn hostile_values_are_survived() {
        for id in ["draw_on", "marker"] {
            let def = find_effect(id).expect("effect");
            for bad in [f64::NAN, f64::INFINITY, -f64::INFINITY, 1e30, -1e30] {
                let mut e = def.instance();
                for p in e.params.values_mut() {
                    match &mut p.value {
                        ParamValue::Float(v) => *v = bad,
                        ParamValue::Vec2(v) => *v = Vec2::new(bad, bad),
                        _ => {}
                    }
                }
                for (w, h) in [(1usize, 1usize), (3, 2), (64, 48)] {
                    for secs in [0.0, 0.5, f64::NAN, 1e12] {
                        let mut img = sketch().transformed(w, h, &filmcraft_geom::Affine::scale(w as f64 / 200.0, h as f64 / 100.0));
                        if id == "draw_on" {
                            draw_on(&mut img, &e, &cx(secs))
                        } else {
                            marker(&mut img, &e, &cx(secs))
                        }
                        assert!(img.px.iter().all(|v| v.is_finite()), "{id} with {bad} at {w}x{h}, {secs} s");
                    }
                }
            }
        }
    }

    /// Local preview: `FILMCRAFT_DRAW_PREVIEW=<dir>` with `<name>_<w>x<h>.rgb` pictures; writes
    /// frames `<name>.draw<k>.rgb` (and one with markers) there.
    #[test]
    #[ignore = "needs local test pictures"]
    fn draw_preview() {
        let Some(dir) = std::env::var_os("FILMCRAFT_DRAW_PREVIEW") else { return };
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
            let save = |img: &Image, tag: &str| {
                // over a mid grey, so removed paper shows
                let out: Vec<u8> = img
                    .px
                    .chunks(4)
                    .flat_map(|p| {
                        enc([p[0] + 0.05 * (1.0 - p[3]), p[1] + 0.08 * (1.0 - p[3]), p[2] + 0.2 * (1.0 - p[3])])
                            .map(|v| (v.clamp(0.0, 1.0) * 255.0).round() as u8)
                    })
                    .collect();
                std::fs::write(entry.path().with_file_name(format!("{stem}.{tag}.rgb")), out).expect("write");
            };
            let mut e = find_effect("draw_on").expect("effect").instance();
            set(&mut e, "duration", ParamValue::Float(1.0));
            set(&mut e, "remove_paper", ParamValue::Bool(true));
            for (k, secs) in [0.2, 0.45, 0.7, 1.2].into_iter().enumerate() {
                let mut img = src.clone();
                let started = std::time::Instant::now();
                draw_on(&mut img, &e, &cx(secs));
                eprintln!("{stem} draw {secs}: {:.0} ms", started.elapsed().as_secs_f64() * 1000.0);
                save(&img, &format!("draw{k}"));
            }
            let mut img = src.clone();
            for (shape, from, to, color) in [
                (0u32, (0.15, 0.8), (0.42, 0.55), [1.0, 0.85, 0.2, 1.0]),
                (1, (0.55, 0.25), (0.85, 0.6), [1.0, 0.25, 0.25, 1.0]),
                (2, (0.1, 0.93), (0.6, 0.93), [0.3, 1.0, 0.6, 1.0]),
                (5, (0.82, 0.75), (0.95, 0.9), [0.3, 1.0, 0.6, 1.0]),
                (3, (0.05, 0.08), (0.3, 0.3), [1.0, 1.0, 1.0, 1.0]),
                (4, (0.68, 0.72), (0.76, 0.86), [1.0, 0.25, 0.25, 1.0]),
            ] {
                let mut m = find_effect("marker").expect("effect").instance();
                set(&mut m, "shape", ParamValue::Choice(shape));
                set(&mut m, "from", ParamValue::Vec2(Vec2::new(from.0 * w as f64, from.1 * h as f64)));
                set(&mut m, "to", ParamValue::Vec2(Vec2::new(to.0 * w as f64, to.1 * h as f64)));
                set(&mut m, "color", ParamValue::Color(color));
                set(&mut m, "curve", ParamValue::Float(if shape == 0 { 35.0 } else { 8.0 }));
                marker(&mut img, &m, &cx(10.0));
            }
            save(&img, "markers");
            // a short film: the sketch draws itself in white over a dark picture, then markers
            if std::env::var_os("FILMCRAFT_DRAW_FILM").is_some() {
                let mut film = Vec::new();
                set(&mut e, "duration", ParamValue::Float(2.2));
                set(&mut e, "tint", ParamValue::Bool(true));
                for frame in 0..100 {
                    let secs = f64::from(frame) / 25.0;
                    let mut img = src.clone();
                    draw_on(&mut img, &e, &cx(secs));
                    for (shape, from, to, color, delay) in [
                        (1u32, (0.55, 0.5), (0.83, 0.75), [1.0, 0.3, 0.3, 1.0], 2.4),
                        (0, (0.33, 0.12), (0.2, 0.26), [1.0, 0.85, 0.2, 1.0], 2.9),
                        (2, (0.64, 0.74), (0.82, 0.72), [1.0, 0.85, 0.2, 1.0], 3.3),
                    ] {
                        let mut m = find_effect("marker").expect("effect").instance();
                        set(&mut m, "shape", ParamValue::Choice(shape));
                        set(&mut m, "from", ParamValue::Vec2(Vec2::new(from.0 * w as f64, from.1 * h as f64)));
                        set(&mut m, "to", ParamValue::Vec2(Vec2::new(to.0 * w as f64, to.1 * h as f64)));
                        set(&mut m, "color", ParamValue::Color(color));
                        set(&mut m, "delay", ParamValue::Float(delay));
                        set(&mut m, "duration", ParamValue::Float(0.35));
                        marker(&mut img, &m, &cx(secs));
                    }
                    for (i, p) in img.px.chunks(4).enumerate() {
                        let (x, y) = ((i % w) as f32 / w as f32, (i / w) as f32 / h as f32);
                        let bg = [0.02 + 0.05 * y, 0.03 + 0.03 * x, 0.08 + 0.1 * (1.0 - y)];
                        film.extend(
                            enc([p[0] + bg[0] * (1.0 - p[3]), p[1] + bg[1] * (1.0 - p[3]), p[2] + bg[2] * (1.0 - p[3])])
                                .map(|v| (v.clamp(0.0, 1.0) * 255.0).round() as u8),
                        );
                    }
                }
                std::fs::write(entry.path().with_file_name(format!("{stem}.film.rgb")), film).expect("write");
            }
        }
    }
}
