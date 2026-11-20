//! Face finding and tracking for the FilmCraft Plus portrait effects (Auto Focus, Studio Light).
//!
//! **Finding.** A boosted cascade of local-binary-pattern features, evaluated here in FilmCraft's
//! own code: a 24×24 window slides over a pyramid of the picture; every stage sums the votes of a
//! few features (each compares the brightness of the eight cells around a centre cell and looks
//! the resulting pattern up in a 256-entry table) and rejects the window when the sum is below the
//! stage's threshold. Windows that pass all stages and agree with their neighbours are a face.
//! The trained numbers (thresholds, feature rectangles, tables) are the open LBP frontal face data
//! of the OpenCV project (Apache-2.0, `assets/models/`, see `ATTRIBUTION.md`); it finds faces
//! looking roughly at the camera.
//!
//! **Tracking.** Effects are pure functions of time, so there is no tracker state: the face is
//! found in the clip's frames around the current one (about twelve analyses a second, cached per
//! clip) and the positions are combined with a median-guarded, centre-weighted average. A few
//! missed frames are bridged by their neighbours; [`Face::conf`] says how much of the window had a
//! face, so effects can ease towards their fallback instead of jumping.

use std::collections::HashMap;
use std::sync::{Mutex, OnceLock, PoisonError};

use super::{FxCtx, Image, luma};

static CASCADE_XML: &str = include_str!("../../../../assets/models/lbpcascade_frontalface.xml");

/// One feature's vote: a 256-bit table indexed by the pattern, and the two votes.
struct Weak {
    feature: usize,
    subset: [u32; 8],
    leaves: [f32; 2],
}

struct Stage {
    threshold: f32,
    weak: Vec<Weak>,
}

/// A parsed cascade.
pub struct Cascade {
    win: (usize, usize),
    stages: Vec<Stage>,
    /// Top-left cell of each feature's 3×3 grid: x, y, cell width, cell height.
    features: Vec<[usize; 4]>,
}

/// The text inside every `<tag>…</tag>` of `text`, in order.
fn blocks<'a>(text: &'a str, tag: &str) -> Vec<&'a str> {
    let (open, close) = (format!("<{tag}>"), format!("</{tag}>"));
    let mut out = Vec::new();
    let mut rest = text;
    while let Some(a) = rest.find(&open) {
        let after = &rest[a + open.len()..];
        let Some(b) = after.find(&close) else { break };
        out.push(&after[..b]);
        rest = &after[b + close.len()..];
    }
    out
}

fn numbers<T: std::str::FromStr>(s: &str) -> Option<Vec<T>> {
    s.split_whitespace().map(|t| t.parse().ok()).collect()
}

impl Cascade {
    /// Parse cascade XML (the `opencv_storage` layout of LBP cascades with depth-1 trees).
    /// `None` when anything is missing or out of range, so evaluation never reads out of bounds.
    pub fn parse(xml: &str) -> Option<Cascade> {
        let one = |tag: &str| blocks(xml, tag).first().and_then(|s| s.trim().parse::<usize>().ok());
        let win = (one("width")?, one("height")?);
        if !(8..=256).contains(&win.0) || !(8..=256).contains(&win.1) {
            return None;
        }
        let features: Vec<[usize; 4]> = blocks(blocks(xml, "features").first()?, "rect")
            .iter()
            .map(|r| {
                let v = numbers::<usize>(r)?;
                let f: [usize; 4] = v.try_into().ok()?;
                // the 3×3 grid lies inside the window
                (f[2] >= 1 && f[3] >= 1 && f[0] + 3 * f[2] <= win.0 && f[1] + 3 * f[3] <= win.1).then_some(f)
            })
            .collect::<Option<_>>()?;
        let body = blocks(xml, "stages");
        let body = body.first()?;
        let counts: Vec<usize> = blocks(body, "maxWeakCount").iter().map(|s| s.trim().parse().ok()).collect::<Option<_>>()?;
        let thresholds: Vec<f32> = blocks(body, "stageThreshold").iter().map(|s| s.trim().parse().ok()).collect::<Option<_>>()?;
        let nodes = blocks(body, "internalNodes");
        let leaves = blocks(body, "leafValues");
        if counts.len() != thresholds.len()
            || counts.is_empty()
            || counts.iter().sum::<usize>() != nodes.len()
            || nodes.len() != leaves.len()
            || nodes.len() > 100_000
        {
            return None;
        }
        let mut weak = nodes.iter().zip(&leaves).map(|(n, l)| {
            // left child, right child, feature, then the 256-bit table as eight words
            let n = numbers::<i64>(n)?;
            let l = numbers::<f32>(l)?;
            let feature = usize::try_from(*n.get(2)?).ok().filter(|f| *f < features.len())?;
            let mut subset = [0u32; 8];
            for (s, v) in subset.iter_mut().zip(n.get(3..11)?) {
                *s = i32::try_from(*v).ok()? as u32;
            }
            let leaves: [f32; 2] = l.try_into().ok()?;
            leaves.iter().all(|v| v.is_finite()).then_some(Weak { feature, subset, leaves })
        });
        let mut stages = Vec::with_capacity(counts.len());
        for (n, threshold) in counts.into_iter().zip(thresholds) {
            let weak = weak.by_ref().take(n).collect::<Option<Vec<_>>>()?;
            if weak.len() != n || !threshold.is_finite() {
                return None;
            }
            stages.push(Stage { threshold, weak });
        }
        Some(Cascade { win, stages, features })
    }

    /// Whether the window at `(x, y)` of the picture behind `integral` (`iw` wide, with a zero
    /// row and column first) passes every stage.
    fn passes(&self, integral: &[u32], iw: usize, x: usize, y: usize) -> bool {
        let at = |px: usize, py: usize| integral.get(py * iw + px).copied().unwrap_or(0);
        for st in &self.stages {
            let mut sum = 0.0f32;
            for wk in &st.weak {
                let Some(&[fx, fy, cw, ch]) = self.features.get(wk.feature) else { return false };
                let (x0, y0) = (x + fx, y + fy);
                // sum of cell (i, j) of the 3×3 grid
                let cell = |i: usize, j: usize| {
                    let (a, b) = (x0 + i * cw, y0 + j * ch);
                    at(a + cw, b + ch).wrapping_sub(at(a, b + ch)).wrapping_sub(at(a + cw, b)).wrapping_add(at(a, b))
                };
                let c = cell(1, 1);
                let bit = |i: usize, j: usize, v: u32| if cell(i, j) >= c { v } else { 0 };
                let code = bit(0, 0, 128) | bit(1, 0, 64) | bit(2, 0, 32) | bit(2, 1, 16) | bit(2, 2, 8) | bit(1, 2, 4) | bit(0, 2, 2) | bit(0, 1, 1);
                let set = wk.subset[(code >> 5) as usize] & (1 << (code & 31)) != 0;
                sum += wk.leaves[usize::from(!set)];
            }
            if sum < st.threshold {
                return false;
            }
        }
        true
    }
}

/// The built-in frontal face cascade.
fn cascade() -> Option<&'static Cascade> {
    static C: OnceLock<Option<Cascade>> = OnceLock::new();
    C.get_or_init(|| Cascade::parse(CASCADE_XML)).as_ref()
}

/// A face in a picture: centre and side of its square, in pixels of that picture.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Det {
    pub x: f32,
    pub y: f32,
    pub size: f32,
    /// How many windows agreed.
    pub hits: u32,
}

/// Half-size copy of an 8-bit grey picture scaled by `1 / f` (bilinear), for the pyramid.
fn shrink(src: &[u8], w: usize, h: usize, nw: usize, nh: usize) -> Vec<u8> {
    let mut out = vec![0u8; nw * nh];
    if w == 0 || h == 0 || nw == 0 || nh == 0 || src.len() < w * h {
        return out;
    }
    let (sx, sy) = (w as f32 / nw as f32, h as f32 / nh as f32);
    for y in 0..nh {
        let fy = ((y as f32 + 0.5) * sy - 0.5).clamp(0.0, (h - 1) as f32);
        let (y0, ty) = (fy.floor() as usize, fy.fract());
        let y1 = (y0 + 1).min(h - 1);
        for x in 0..nw {
            let fx = ((x as f32 + 0.5) * sx - 0.5).clamp(0.0, (w - 1) as f32);
            let (x0, tx) = (fx.floor() as usize, fx.fract());
            let x1 = (x0 + 1).min(w - 1);
            let p = |a: usize, b: usize| f32::from(src[b * w + a]);
            let top = p(x0, y0) + (p(x1, y0) - p(x0, y0)) * tx;
            let bot = p(x0, y1) + (p(x1, y1) - p(x0, y1)) * tx;
            out[y * nw + x] = (top + (bot - top) * ty).round() as u8;
        }
    }
    out
}

/// `src` turned around its centre (`sin`, `cos` of the angle): `out(p) = src(c + R·(p − c))`,
/// nearest pixel, edges repeated.
fn rotated(src: &[u8], w: usize, h: usize, sin: f32, cos: f32) -> Vec<u8> {
    let mut out = vec![0u8; w * h];
    if w == 0 || h == 0 || src.len() < w * h {
        return out;
    }
    let (cx, cy) = (w as f32 / 2.0, h as f32 / 2.0);
    for y in 0..h {
        for x in 0..w {
            let (dx, dy) = (x as f32 + 0.5 - cx, y as f32 + 0.5 - cy);
            let sx = ((cx + dx * cos - dy * sin) as isize).clamp(0, w as isize - 1) as usize;
            let sy = ((cy + dx * sin + dy * cos) as isize).clamp(0, h as isize - 1) as usize;
            out[y * w + x] = src[sy * w + sx];
        }
    }
    out
}

/// Faces in an 8-bit grey picture, strongest first.
pub fn detect(gray: &[u8], w: usize, h: usize) -> Vec<Det> {
    detect_with(gray, w, h, 1.1, 2)
}

fn detect_with(gray: &[u8], w: usize, h: usize, factor: f32, min_hits: u32) -> Vec<Det> {
    let Some(c) = cascade() else { return Vec::new() };
    if gray.len() < w * h || w < c.win.0 || h < c.win.1 || w > 4096 || h > 4096 {
        return Vec::new();
    }
    let mut raw: Vec<(f32, f32, f32)> = Vec::new();
    let mut level = gray[..w * h].to_vec();
    let (mut lw, mut lh) = (w, h);
    let mut scale = 1.0f32;
    loop {
        // integral image with a zero first row and column
        let iw = lw + 1;
        let mut integral = vec![0u32; iw * (lh + 1)];
        for y in 0..lh {
            let mut row = 0u32;
            for x in 0..lw {
                row += u32::from(level[y * lw + x]);
                integral[(y + 1) * iw + x + 1] = integral[y * iw + x + 1].wrapping_add(row);
            }
        }
        let mut y = 0;
        while y + c.win.1 <= lh {
            let mut x = 0;
            while x + c.win.0 <= lw {
                if c.passes(&integral, iw, x, y) && raw.len() < 4000 {
                    let side = c.win.0 as f32 * scale;
                    raw.push(((x as f32 + c.win.0 as f32 / 2.0) * scale, (y as f32 + c.win.1 as f32 / 2.0) * scale, side));
                }
                x += 2;
            }
            y += 2;
        }
        scale *= factor;
        let (nw, nh) = ((w as f32 / scale) as usize, (h as f32 / scale) as usize);
        if nw < c.win.0 || nh < c.win.1 {
            break;
        }
        level = shrink(gray, w, h, nw, nh);
        (lw, lh) = (nw, nh);
    }
    // group windows that agree (similar place and size); a lone window is noise
    let mut groups: Vec<(f32, f32, f32, u32)> = Vec::new();
    for (x, y, s) in raw {
        let hit = groups.iter_mut().find(|g| {
            let (gx, gy, gs) = (g.0 / g.3 as f32, g.1 / g.3 as f32, g.2 / g.3 as f32);
            let near = 0.3 * gs.min(s);
            (gx - x).abs() <= near && (gy - y).abs() <= near && s <= gs * 1.5 && gs <= s * 1.5
        });
        match hit {
            Some(g) => *g = (g.0 + x, g.1 + y, g.2 + s, g.3 + 1),
            None => groups.push((x, y, s, 1)),
        }
    }
    let mut out: Vec<Det> =
        groups.into_iter().filter(|g| g.3 >= min_hits).map(|g| Det { x: g.0 / g.3 as f32, y: g.1 / g.3 as f32, size: g.2 / g.3 as f32, hits: g.3 }).collect();
    out.sort_by(|a, b| (b.size * (b.hits as f32).sqrt()).total_cmp(&(a.size * (a.hits as f32).sqrt())));
    out
}

/// A face relative to the frame: centre as fractions of the width and height, size as a fraction
/// of the frame height.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Face {
    pub x: f32,
    pub y: f32,
    pub size: f32,
    /// 0..1: how much of the tracking window had a face.
    pub conf: f32,
    /// Mean display-encoded brightness of the face, 0..1.
    pub luma: f32,
}

/// Longest side of the grey picture the detector works on (a face must be about a twelfth of it).
const WORK: usize = 320;

/// The main face of a picture (the largest, most agreed-on one).
pub fn face_in(img: &Image) -> Option<Face> {
    if img.w < 16 || img.h < 16 {
        return None;
    }
    // box-average down to the working size, then to display-encoded grey
    let k = (img.w.max(img.h) as f32 / WORK as f32).max(1.0);
    let (w, h) = (((img.w as f32 / k) as usize).max(1), ((img.h as f32 / k) as usize).max(1));
    let mut gray = vec![0u8; w * h];
    for y in 0..h {
        let (y0, y1) = ((y as f32 * k) as usize, (((y + 1) as f32 * k) as usize).clamp(1, img.h));
        for x in 0..w {
            let (x0, x1) = ((x as f32 * k) as usize, (((x + 1) as f32 * k) as usize).clamp(1, img.w));
            let (mut sum, mut n) = (0.0f32, 0u32);
            for sy in y0..y1.max(y0 + 1).min(img.h) {
                for sx in x0..x1.max(x0 + 1).min(img.w) {
                    if let Some(p) = img.px.get((sy * img.w + sx) * 4..(sy * img.w + sx) * 4 + 3) {
                        sum += luma([p[0], p[1], p[2]]);
                        n += 1;
                    }
                }
            }
            let v = if n > 0 && sum.is_finite() { (sum / n as f32).clamp(0.0, 1.0) } else { 0.0 };
            gray[y * w + x] = (filmcraft_color::linear_to_srgb(v) * 255.0).round() as u8;
        }
    }
    // the main face: big, agreed on, and rather in the middle than at the edge
    let main = |found: Vec<Det>| {
        let score = |d: &Det| d.size * (d.hits as f32).sqrt() * (1.2 - (d.x / w as f32 - 0.5).abs());
        found.into_iter().max_by(|a, b| score(a).total_cmp(&score(b)))
    };
    let mut d = main(detect(&gray, w, h));
    if d.is_none() {
        // a tilted head: look again with the picture turned either way
        for angle in [-20.0f32, 20.0] {
            let (sin, cos) = angle.to_radians().sin_cos();
            let turned = rotated(&gray, w, h, sin, cos);
            if let Some(t) = main(detect(&turned, w, h)) {
                let (cx, cy) = (w as f32 / 2.0, h as f32 / 2.0);
                let (dx, dy) = (t.x - cx, t.y - cy);
                d = Some(Det { x: (cx + dx * cos - dy * sin).clamp(0.0, w as f32), y: (cy + dx * sin + dy * cos).clamp(0.0, h as f32), ..t });
                break;
            }
        }
    }
    let d = d?;
    // brightness of the inner face (cheeks, nose, forehead), away from hair and background
    let r = d.size * 0.3;
    let (x0, x1) = ((d.x - r).max(0.0) as usize, ((d.x + r) as usize).min(w.saturating_sub(1)));
    let (y0, y1) = ((d.y - r).max(0.0) as usize, ((d.y + r) as usize).min(h.saturating_sub(1)));
    let (mut sum, mut n) = (0u64, 0u64);
    for y in y0..=y1 {
        for x in x0..=x1 {
            sum += u64::from(gray.get(y * w + x).copied().unwrap_or(0));
            n += 1;
        }
    }
    let luma = if n > 0 { sum as f32 / n as f32 / 255.0 } else { 0.5 };
    Some(Face { x: d.x / w as f32, y: d.y / h as f32, size: d.size / h as f32, conf: 1.0, luma })
}

type FaceCache = Mutex<HashMap<(u64, i64), Option<Face>>>;
fn cache() -> &'static FaceCache {
    static C: OnceLock<FaceCache> = OnceLock::new();
    C.get_or_init(Default::default)
}

/// The time window a face is tracked over, on the grid of analysed clip frames.
struct Window {
    /// Clip time of the current frame, seconds.
    now: f64,
    /// Smoothing reach either side, seconds.
    half: f64,
    fps: f64,
    /// Frames between two analyses.
    stride: i64,
    /// Last frame of the clip.
    last: i64,
    /// Picture width / height.
    aspect: f32,
}

/// Longest clip time tracked (a day): keeps frame numbers far from overflow.
const MAX_SECONDS: f64 = 86_400.0;

/// The face at the current frame of the clip, smoothed over `half` seconds either side.
/// Without an environment (stills, tests) the current picture alone is searched.
pub fn tracked(cx: &FxCtx, img: &Image, half: f64) -> Option<Face> {
    let Some(env) = cx.env else { return face_in(img) };
    let fps = env.frame_rate();
    let fps = if fps.is_finite() { fps.clamp(1.0, 240.0) } else { 25.0 };
    let now = if cx.seconds.is_finite() { cx.seconds.clamp(0.0, MAX_SECONDS) } else { 0.0 };
    let length = env.clip_seconds();
    let length = if length.is_finite() { length.clamp(0.0, MAX_SECONDS) } else { 0.0 };
    let win = Window {
        now,
        half: if half.is_finite() { half.clamp(0.0, 2.0) } else { 0.3 },
        fps,
        // about twelve analyses a second, on a fixed grid of clip frames so neighbours share them
        stride: (fps / 12.0).round().max(1.0) as i64,
        last: (length * fps).floor() as i64,
        aspect: img.w.max(1) as f32 / img.h.max(1) as f32,
    };
    let key = env.clip_key();
    let (sw, sh) = env.source_size();
    let scale = (WORK as f32 / sw.max(sh).max(1) as f32).min(1.0);
    let analyse = |id: i64| -> Option<Face> {
        let cached = cache().lock().unwrap_or_else(PoisonError::into_inner).get(&(key, id)).copied();
        cached.unwrap_or_else(|| {
            let f = env.source_frame(id as f64 / fps - now, scale).and_then(|f| face_in(&f));
            let mut m = cache().lock().unwrap_or_else(PoisonError::into_inner);
            if m.len() > 200_000 {
                m.clear();
            }
            m.insert((key, id), f);
            f
        })
    };
    // The frames of the window that were not looked at yet (all of them after a jump of the
    // playhead, one now and then during playback) are analysed side by side on all cores, so a
    // jump costs about one analysis and not a dozen in a row.
    let (lo, hi) = span(&win);
    let missing: Vec<i64> = {
        let seen = cache().lock().unwrap_or_else(PoisonError::into_inner);
        (0..64).map(|k| lo.max(0) + k * win.stride).take_while(|id| *id <= hi.min(win.last.max(0))).filter(|id| !seen.contains_key(&(key, *id))).collect()
    };
    if missing.len() > 1 {
        use rayon::prelude::*;
        missing.par_iter().for_each(|id| {
            let _ = analyse(*id);
        });
    }
    combine(&win, &analyse)
}

/// The first and last frame of the tracking window, on the analysis grid.
fn span(win: &Window) -> (i64, i64) {
    let Window { now, half, fps, stride, .. } = *win;
    ((((now - half) * fps / stride as f64).floor() as i64) * stride, (((now + half) * fps / stride as f64).ceil() as i64) * stride)
}

/// One face for the window from the per-frame results of `analyse` (frame number → face).
fn combine(win: &Window, analyse: &dyn Fn(i64) -> Option<Face>) -> Option<Face> {
    let Window { now, half, fps, stride, last, aspect } = *win;
    let (lo, hi) = span(win);
    let mut found: Vec<(Face, f32)> = Vec::new();
    let mut total = 0.0f32;
    let mut id = lo.max(0);
    let mut guard = 0;
    while id <= hi.min(last) && guard < 64 {
        guard += 1;
        let t = id as f64 / fps;
        let wgt = (1.0 - ((t - now).abs() / (half + stride as f64 / fps)) as f32).max(0.05);
        total += wgt;
        if let Some(f) = analyse(id) {
            found.push((f, wgt));
        }
        id += stride;
    }
    if !found.is_empty() && total > 0.0 {
        // the median guards against a stray detection (another face, a poster); then a weighted mean
        let median = |get: fn(&Face) -> f32| {
            let mut v: Vec<f32> = found.iter().map(|(f, _)| get(f)).collect();
            v.sort_by(f32::total_cmp);
            v.get(v.len() / 2).copied().unwrap_or(0.5)
        };
        let (mx, my, ms) = (median(|f| f.x), median(|f| f.y), median(|f| f.size));
        let (mut x, mut y, mut s, mut l, mut n) = (0.0f32, 0.0f32, 0.0f32, 0.0f32, 0.0f32);
        for (f, wgt) in &found {
            if ((f.x - mx) * aspect).abs() <= ms && (f.y - my).abs() <= ms && f.size <= ms * 1.6 && ms <= f.size * 1.6 {
                x += f.x * wgt;
                y += f.y * wgt;
                s += f.size * wgt;
                l += f.luma * wgt;
                n += wgt;
            }
        }
        if n > 0.0 {
            return Some(Face { x: x / n, y: y / n, size: s / n, conf: (n / total).clamp(0.0, 1.0), luma: l / n });
        }
    }
    // No face around this frame (looking away, a hand in front): hold the nearest one found up
    // to a second and a half before or after, with a confidence that fades with the distance.
    let reach = (1.5 * fps / stride as f64).ceil() as i64;
    let mut near: Vec<(Face, f32)> = Vec::new();
    for dir in [-1i64, 1] {
        for k in 1..=reach.min(24) {
            let id = if dir < 0 { lo - k * stride } else { hi + k * stride };
            if id < 0 || id > last {
                break;
            }
            if let Some(f) = analyse(id) {
                near.push((f, (1.0 - k as f32 / (reach + 1) as f32).max(0.0)));
                break;
            }
        }
    }
    let n: f32 = near.iter().map(|(_, w)| w).sum();
    if n <= 0.0 {
        return None;
    }
    let mix = |get: fn(&Face) -> f32| near.iter().map(|(f, w)| get(f) * w).sum::<f32>() / n;
    let best = near.iter().map(|(_, w)| *w).fold(0.0f32, f32::max);
    Some(Face { x: mix(|f| f.x), y: mix(|f| f.y), size: mix(|f| f.size), conf: 0.6 * best, luma: mix(|f| f.luma) })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_built_in_cascade_parses_and_is_in_range() {
        let c = cascade().expect("cascade");
        assert_eq!(c.win, (24, 24));
        assert_eq!(c.stages.len(), 20);
        assert!(c.stages.iter().all(|s| !s.weak.is_empty()));
        assert!(c.stages.iter().flat_map(|s| &s.weak).all(|w| w.feature < c.features.len()));
    }

    #[test]
    fn broken_cascades_are_refused_not_trusted() {
        assert!(Cascade::parse("").is_none());
        assert!(Cascade::parse("<width>45</width><height>45</height>").is_none());
        // a feature that leaves the window, a feature index that does not exist, a missing table
        let ok = "<width>24</width><height>24</height><stages><maxWeakCount>1</maxWeakCount><stageThreshold>0.5</stageThreshold>\
                  <internalNodes>0 -1 0 1 2 3 4 5 6 7 8</internalNodes><leafValues>-1 1</leafValues></stages><features><rect>0 0 8 8</rect></features>";
        assert!(Cascade::parse(ok).is_some());
        assert!(Cascade::parse(&ok.replace("0 0 8 8", "0 0 9 8")).is_none());
        assert!(Cascade::parse(&ok.replace("0 -1 0 1", "0 -1 7 1")).is_none());
        assert!(Cascade::parse(&ok.replace(" 7 8</internalNodes>", "</internalNodes>")).is_none());
        assert!(Cascade::parse(&ok.replace("-1 1</leafValues>", "nan 1</leafValues>")).is_none());
        assert!(Cascade::parse(&ok.replace("<maxWeakCount>1<", "<maxWeakCount>2<")).is_none());
    }

    #[test]
    fn nothing_is_found_where_there_is_no_face() {
        // flat, noise, a gradient, pictures too small for the window, and sizes that do not match
        assert!(detect(&vec![128u8; 320 * 180], 320, 180).is_empty());
        let noise: Vec<u8> = (0..320 * 180u32).map(|i| (i.wrapping_mul(2_654_435_761) >> 24) as u8).collect();
        assert!(detect(&noise, 320, 180).is_empty());
        let ramp: Vec<u8> = (0..320 * 180).map(|i| (i % 320 * 255 / 320) as u8).collect();
        assert!(detect(&ramp, 320, 180).is_empty());
        assert!(detect(&[0u8; 100], 10, 10).is_empty());
        assert!(detect(&[0u8; 10], 320, 180).is_empty());
        assert!(face_in(&Image::new(0, 0)).is_none());
        assert!(face_in(&Image::filled(64, 64, [f32::NAN, 0.5, 0.5, 1.0])).is_none());
    }

    /// Local check against real photographs (never committed): set `FILMCRAFT_FACE_TEST` to a
    /// folder of raw 8-bit grey files named `<name>_<w>x<h>.gray`; prints what is found.
    #[test]
    #[ignore = "needs local test photographs"]
    fn finds_faces_in_photographs() {
        let Some(dir) = std::env::var_os("FILMCRAFT_FACE_TEST") else { return };
        for (factor, min_hits) in [(1.1f32, 2u32)] {
            let (mut faces, mut hit, mut wrong, mut ms) = (0, 0, 0, 0.0);
            let (mut heads, mut head_hit) = (0, 0);
            for entry in std::fs::read_dir(&dir).expect("dir").flatten() {
                let name = entry.file_name().to_string_lossy().to_string();
                let Some((w, h)) = name.strip_suffix(".gray").and_then(|n| n.rsplit_once('_')).and_then(|(_, d)| d.split_once('x')) else { continue };
                let (w, h): (usize, usize) = (w.parse().expect("w"), h.parse().expect("h"));
                let gray = std::fs::read(entry.path()).expect("read");
                let started = std::time::Instant::now();
                let found = detect_with(&gray, w, h, factor, min_hits);
                ms += started.elapsed().as_secs_f64() * 1000.0;
                // optional ground truth next to the picture: lines of `centre-x centre-y side`
                let truth: Vec<Vec<f32>> = std::fs::read_to_string(entry.path().with_extension("gt"))
                    .unwrap_or_default()
                    .lines()
                    .map(|l| l.split_whitespace().filter_map(|v| v.parse().ok()).collect())
                    .collect();
                let is =
                    |d: &Det, t: &[f32]| (d.x - t[0]).abs() < 0.35 * t[2] && (d.y - t[1]).abs() < 0.35 * t[2] && d.size > 0.4 * t[2] && d.size < 2.2 * t[2];
                faces += truth.len();
                hit += truth.iter().filter(|t| found.iter().any(|d| is(d, t))).count();
                wrong += found.iter().filter(|d| !truth.iter().any(|t| is(d, t))).count();
                let head = name.contains("head");
                heads += usize::from(head);
                head_hit += usize::from(head && found.first().is_some_and(|d| is(d, &truth[0])));
            }
            eprintln!("x{factor} >={min_hits}: faces {faces}, found {hit}, wrong {wrong}, heads {head_hit}/{heads} first, {ms:.0} ms in all");
        }
    }

    #[test]
    fn turning_a_picture_and_back_keeps_a_point_in_place() {
        // a bright dot right of the centre; turned by 90°, it must land where the mapping says
        let (w, h) = (41usize, 41usize);
        let mut g = vec![0u8; w * h];
        g[20 * w + 30] = 255;
        let t = rotated(&g, w, h, 1.0, 0.0);
        let at = t.iter().position(|v| *v == 255).expect("dot");
        let (x, y) = ((at % w) as f32 + 0.5, (at / w) as f32 + 0.5);
        // map back as face_in does
        let (cx, cy) = (20.5f32, 20.5f32);
        let (bx, by) = (cx + (x - cx) * 0.0 - (y - cy) * 1.0, cy + (x - cx) * 1.0 + (y - cy) * 0.0);
        assert!((bx - 30.5).abs() <= 1.0 && (by - 20.5).abs() <= 1.0, "{bx} {by}");
        assert_eq!(rotated(&g, 0, 0, 0.0, 1.0).len(), 0);
    }

    fn window(now: f64, half: f64) -> Window {
        Window { now, half, fps: 24.0, stride: 2, last: 240, aspect: 0.5625 }
    }
    fn face(x: f32) -> Face {
        Face { x, y: 0.35, size: 0.2, conf: 1.0, luma: 0.5 }
    }

    #[test]
    fn tracking_smooths_a_moving_face_and_ignores_a_stray_one() {
        // the face moves steadily to the right; one analysis sees a poster in the corner
        let seen = |id: i64| Some(if id == 24 { Face { x: 0.95, y: 0.9, size: 0.08, conf: 1.0, luma: 0.5 } } else { face(0.3 + id as f32 * 0.002) });
        let f = combine(&window(1.0, 0.3), &seen).expect("face");
        assert!((f.x - 0.348).abs() < 0.01 && (f.y - 0.35).abs() < 1e-3, "{f:?}");
        assert!(f.conf > 0.5 && f.conf < 1.0, "the stray analysis lowers the confidence: {}", f.conf);
        // neighbouring frames get nearly the same answer (no jitter)
        let g = combine(&window(1.0 + 1.0 / 24.0, 0.3), &seen).expect("face");
        assert!((g.x - f.x).abs() < 0.004, "{} {}", f.x, g.x);
    }

    #[test]
    fn a_lost_face_is_held_and_fades_out() {
        // a face until frame 48 (2 s), nothing after
        let seen = |id: i64| (id <= 48).then(|| face(0.4));
        let near = combine(&window(2.6, 0.2), &seen).expect("held");
        let far = combine(&window(3.4, 0.2), &seen).expect("held");
        assert!((near.x - 0.4).abs() < 1e-4 && near.conf > far.conf && far.conf > 0.0, "{near:?} {far:?}");
        assert!(near.conf <= 0.6);
        assert!(combine(&window(6.0, 0.2), &seen).is_none(), "too long ago");
        assert!(combine(&window(1.0, 0.3), &|_| None).is_none());
    }

    #[test]
    fn hostile_windows_end() {
        for now in [0.0, -5.0, 1e9, 86_400.0] {
            for half in [0.0, 2.0] {
                for (fps, stride) in [(1.0, 1), (240.0, 20)] {
                    let w = Window { now, half, fps, stride, last: 10, aspect: 1.0 };
                    let _ = combine(&w, &|id| (id % 3 == 0).then(|| face(0.5)));
                }
            }
        }
    }
}
