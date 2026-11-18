//! Animated captions (FilmCraft Plus): word-by-word captions for short vertical video.
//!
//! A caption track whose style has a [`CaptionAnimation`] draws its captions through
//! [`render_animated`] instead of the static [`render_caption`](crate::burn::render_caption). The
//! picture is a pure function of the time since the caption started, so scrubbing, playback and
//! export agree.
//!
//! - **Timing** comes from [`Caption::words`] (made from the transcript's word times). When the
//!   caption text was edited so the word count no longer matches, or there are no word times, the
//!   words share the caption's duration in proportion to their length.
//! - **Size** is relative to a 1920×1080 picture by *area*, so the same style reads the same in
//!   landscape and in 9:16 portrait. A word wider than the frame shrinks the whole block.
//! - **Layers**, bottom to top: soft shadow, line background, word boxes / tiles (highlight
//!   colour), outline, text, highlighted text.
//!
//! All sizes derive from clamped style values and the frame size; overlays are capped like the
//! static ones.

use filmcraft_project::{Caption, CaptionAlign, CaptionAnchor, CaptionAnimation, CaptionStyle};
use filmcraft_text::{ParagraphStyle, TextStyle, layout, render};
use filmcraft_time::Tick;

use crate::burn::{MAX_CAPTION_SIZE_1080, MAX_OUTLINE_RADIUS, MAX_OVERLAY_PIXELS, Overlay, caption_style, lin, vtt_overrides};

/// A word laid out in the frame.
#[derive(Clone, Debug)]
struct Word {
    text: String,
    /// Seconds from the caption start.
    start: f32,
    end: f32,
    line: usize,
    /// Left edge and width at scale 1.
    x: f32,
    w: f32,
}

/// How a word is drawn at one moment.
#[derive(Clone, Copy, Debug, PartialEq)]
struct Draw {
    alpha: f32,
    scale: f32,
    /// Downward offset in pixels.
    dy: f32,
    /// 0 = text colour, 1 = highlight colour.
    mix: f32,
    /// Fraction of the word (from the left) drawn in the highlight colour (Karaoke).
    fill: f32,
}

const HIDDEN: Draw = Draw { alpha: 0.0, scale: 1.0, dy: 0.0, mix: 0.0, fill: 0.0 };
const PLAIN: Draw = Draw { alpha: 1.0, scale: 1.0, dy: 0.0, mix: 0.0, fill: 0.0 };

fn unit(x: f32) -> f32 {
    if x.is_finite() { x.clamp(0.0, 1.0) } else { 0.0 }
}

fn smooth(x: f32) -> f32 {
    let x = unit(x);
    x * x * (3.0 - 2.0 * x)
}

/// Ease out with a small overshoot (0 → 1, peaking near 1.1).
fn back_out(x: f32) -> f32 {
    let x = unit(x) - 1.0;
    const C: f32 = 1.70158;
    1.0 + (C + 1.0) * x * x * x + C * x * x
}

/// The caption's lines as words, upper-cased when the style says so.
fn tokens(c: &Caption, uppercase: bool) -> Vec<Vec<String>> {
    c.plain_lines()
        .iter()
        .map(|l| l.split_whitespace().map(|w| if uppercase { w.to_uppercase() } else { w.to_string() }).collect::<Vec<_>>())
        .filter(|l: &Vec<String>| !l.is_empty())
        .collect()
}

/// Start and end (seconds from the caption start) of each of `n` words with the given lengths.
fn timings(c: &Caption, lens: &[usize]) -> Vec<(f32, f32)> {
    let dur = (c.duration.seconds() as f32).max(0.01);
    if c.words.len() == lens.len() {
        return c
            .words
            .iter()
            .map(|w| {
                let s = (w.start.seconds() as f32).clamp(0.0, dur);
                (s, (w.end.seconds() as f32).clamp(s, dur.max(s)))
            })
            .collect();
    }
    // no usable word times: share the duration by length (a little is kept for the last word to rest)
    let total: usize = lens.iter().map(|l| l + 2).sum::<usize>().max(1);
    let span = dur * 0.92;
    let mut at = 0.0;
    lens.iter()
        .map(|l| {
            let d = span * (l + 2) as f32 / total as f32;
            let r = (at, at + d);
            at += d;
            r
        })
        .collect()
}

/// The index of the word being spoken at `t` (the last one that has started).
fn active(words: &[Word], t: f32) -> Option<usize> {
    words.iter().rposition(|w| w.start <= t)
}

/// How word `i` is drawn at `t` seconds into the caption.
fn draw_of(anim: CaptionAnimation, words: &[Word], i: usize, t: f32) -> Draw {
    let Some(w) = words.get(i) else { return HIDDEN };
    let age = t - w.start;
    let next = words.get(i + 1).map_or(f32::MAX, |n| n.start);
    // highlight: quickly on when the word starts, gently off when the next one does
    let lit = unit(age / 0.06) * (1.0 - unit((t - next) / 0.15));
    match anim {
        CaptionAnimation::None => PLAIN,
        CaptionAnimation::Pop => {
            if age < 0.0 {
                return HIDDEN;
            }
            Draw { alpha: unit(age / 0.04), scale: 0.6 + 0.4 * back_out(age / 0.18) + 0.08 * lit, dy: 0.0, mix: lit, fill: 0.0 }
        }
        CaptionAnimation::Box => Draw { alpha: unit(t / 0.1), scale: 1.0 + 0.04 * lit, ..PLAIN },
        CaptionAnimation::OneWord => {
            if active(words, t).unwrap_or(0) != i {
                return HIDDEN;
            }
            let age = age.max(t.min(0.2));
            Draw { alpha: unit(age / 0.04), scale: 0.7 + 0.3 * back_out(age / 0.16), ..PLAIN }
        }
        CaptionAnimation::Karaoke => {
            let len = (w.end - w.start).max(0.04);
            Draw { alpha: unit(t / 0.1), fill: unit(age / len), ..PLAIN }
        }
        CaptionAnimation::Rise => {
            if age < 0.0 {
                return HIDDEN;
            }
            let e = 1.0 - (1.0 - unit(age / 0.24)).powi(3);
            Draw { alpha: e, dy: 1.0 - e, ..PLAIN }
        }
        CaptionAnimation::Tiles => {
            if age < 0.0 {
                return HIDDEN;
            }
            Draw { alpha: unit(age / 0.05), scale: 0.65 + 0.35 * back_out(age / 0.18), ..PLAIN }
        }
    }
}

/// Add an anti-aliased rounded rectangle of coverage `a` to `mask` (`ow`×`oh`, origin `(ox, oy)`).
#[allow(clippy::too_many_arguments)]
fn round_rect(mask: &mut [f32], ow: usize, oh: usize, origin: (i32, i32), x0: f32, y0: f32, x1: f32, y1: f32, r: f32, a: f32) {
    if !(x1 > x0 && y1 > y0) || a <= 0.0 {
        return;
    }
    let (ox, oy) = origin;
    let (cx, cy) = ((x0 + x1) / 2.0, (y0 + y1) / 2.0);
    let (hx, hy) = ((x1 - x0) / 2.0, (y1 - y0) / 2.0);
    let r = r.clamp(0.0, hx.min(hy));
    let ys = ((y0.floor() as i32 - 1).max(oy), (y1.ceil() as i32 + 1).min(oy + oh as i32));
    let xs = ((x0.floor() as i32 - 1).max(ox), (x1.ceil() as i32 + 1).min(ox + ow as i32));
    for y in ys.0..ys.1 {
        for x in xs.0..xs.1 {
            // signed distance to the rounded rectangle
            let (qx, qy) = ((x as f32 + 0.5 - cx).abs() - (hx - r), (y as f32 + 0.5 - cy).abs() - (hy - r));
            let d = (qx.max(0.0).powi(2) + qy.max(0.0).powi(2)).sqrt() + qx.max(qy).min(0.0) - r;
            let cov = (0.5 - d).clamp(0.0, 1.0) * a;
            if cov > 0.0
                && let Some(p) = mask.get_mut((y - oy) as usize * ow + (x - ox) as usize)
            {
                *p = p.max(cov);
            }
        }
    }
}

/// Grow a coverage mask by a disc of radius `r` (the text outline). Row maxima of growing width
/// are built once and reused for every row offset, so the cost is linear in the radius: an animated
/// caption is redrawn on every frame.
fn dilate(m: &[f32], w: usize, h: usize, r: f32) -> Vec<f32> {
    let mut out = vec![0.0f32; w * h];
    if m.len() != w * h || w == 0 || h == 0 || !r.is_finite() || r <= 0.0 {
        return out;
    }
    let reach = r.min(MAX_OUTLINE_RADIUS) + 0.5;
    let ri = reach.floor() as usize;
    // half-width of the disc at each row offset
    let half: Vec<usize> = (0..=ri).map(|dy| (reach * reach - (dy * dy) as f32).max(0.0).sqrt().floor() as usize).collect();
    let mut cur = m.to_vec();
    let mut next = vec![0.0f32; w * h];
    for k in 0..=ri {
        for (dy, _) in half.iter().enumerate().filter(|(_, hw)| **hw == k) {
            for y in 0..h {
                for sy in [y.checked_sub(dy), (dy > 0).then_some(y + dy).filter(|v| *v < h)].into_iter().flatten() {
                    let (dst, src) = (&mut out[y * w..(y + 1) * w], &cur[sy * w..(sy + 1) * w]);
                    for (d, s) in dst.iter_mut().zip(src) {
                        if *s > *d {
                            *d = *s;
                        }
                    }
                }
            }
        }
        if k == ri {
            break;
        }
        // one pixel wider
        for y in 0..h {
            let row = &cur[y * w..(y + 1) * w];
            let dst = &mut next[y * w..(y + 1) * w];
            for x in 0..w {
                let l = if x > 0 { row[x - 1] } else { 0.0 };
                let rr = if x + 1 < w { row[x + 1] } else { 0.0 };
                dst[x] = row[x].max(l).max(rr);
            }
        }
        std::mem::swap(&mut cur, &mut next);
    }
    out
}

/// Box-blur `m` twice with radius `r` (a soft shadow).
fn blur(m: &mut [f32], w: usize, h: usize, r: usize) {
    if r == 0 || w == 0 || h == 0 || m.len() != w * h {
        return;
    }
    let mut tmp = vec![0.0f32; m.len()];
    let norm = 1.0 / (2 * r + 1) as f32;
    for _ in 0..2 {
        for y in 0..h {
            let row = &m[y * w..(y + 1) * w];
            let mut acc: f32 = row.iter().take(r + 1).sum();
            for x in 0..w {
                tmp[y * w + x] = acc * norm;
                if x + r + 1 < w {
                    acc += row[x + r + 1];
                }
                if x >= r {
                    acc -= row[x - r];
                }
            }
        }
        for x in 0..w {
            let mut acc: f32 = (0..=r.min(h - 1)).map(|y| tmp[y * w + x]).sum();
            for y in 0..h {
                m[y * w + x] = acc * norm;
                if y + r + 1 < h {
                    acc += tmp[(y + r + 1) * w + x];
                }
                if y >= r {
                    acc -= tmp[(y - r) * w + x];
                }
            }
        }
    }
}

/// Words placed on lines no wider than `max_w` (source lines are kept and re-wrapped); with
/// `single`, every word is alone on line 0. Returns the words and the number of lines.
fn wrap(lines: &[Vec<String>], times: &[(f32, f32)], ts: &TextStyle, single: bool, max_w: f32, gap: f32) -> (Vec<Word>, usize) {
    let mut words: Vec<Word> = Vec::new();
    let mut line = 0usize;
    let mut i = 0usize;
    for src in lines {
        let mut x = 0.0f32;
        for (k, text) in src.iter().enumerate() {
            let ww = filmcraft_text::measure(text, ts);
            let (start, end) = times.get(i).copied().unwrap_or((0.0, 0.0));
            i += 1;
            if single {
                words.push(Word { text: text.clone(), start, end, line: 0, x: 0.0, w: ww });
                continue;
            }
            if k > 0 && x + ww > max_w {
                line += 1;
                x = 0.0;
            }
            words.push(Word { text: text.clone(), start, end, line, x, w: ww });
            x += ww + gap;
        }
        line += 1;
    }
    (words, line)
}

/// Add a word's coverage (`mask`, columns `x0..x1`) to the text layer and the highlight layer:
/// split at column `fill` (Karaoke) or mixed by `mix`.
#[allow(clippy::too_many_arguments)]
fn splat(mask: &[f32], text: &mut [f32], hi: &mut [f32], (ow, oh): (usize, usize), (x0, x1): (usize, usize), alpha: f32, fill: Option<i64>, mix: f32) {
    for y in 0..oh {
        for x in x0..x1 {
            let j = y * ow + x;
            let Some(&a) = mask.get(j) else { continue };
            if a <= 0.0 {
                continue;
            }
            let a = a.min(1.0) * alpha;
            let m = match fill {
                Some(f) if (x as i64) < f => 1.0,
                Some(_) => 0.0,
                None => mix,
            };
            if let (Some(tp), Some(hp)) = (text.get_mut(j), hi.get_mut(j)) {
                *tp = (*tp + a * (1.0 - m)).min(1.0);
                *hp = (*hp + a * m).min(1.0);
            }
        }
    }
}

type Rect = (f32, f32, f32, f32);

/// Spotlight: the box behind the spoken word, gliding in from the previous word.
fn glide_box(boxes: &mut [f32], (ow, oh): (usize, usize), origin: (i32, i32), words: &[Word], t: f32, radius: f32, rect_of: &dyn Fn(&Word) -> Rect) {
    let cur = active(words, t).unwrap_or(0);
    let Some(wd) = words.get(cur) else { return };
    let mut r = rect_of(wd);
    if cur > 0
        && let Some(prev) = words.get(cur - 1)
    {
        let k = smooth((t - wd.start) / 0.1);
        let p = rect_of(prev);
        let mix = |a: f32, b: f32| a + (b - a) * k;
        // across a line break the box fades over instead of flying diagonally
        if prev.line == wd.line {
            r = (mix(p.0, r.0), mix(p.1, r.1), mix(p.2, r.2), mix(p.3, r.3));
        } else {
            round_rect(boxes, ow, oh, origin, p.0, p.1, p.2, p.3, radius, 1.0 - k);
            round_rect(boxes, ow, oh, origin, r.0, r.1, r.2, r.3, radius, k);
            r = (0.0, 0.0, 0.0, 0.0);
        }
    }
    round_rect(boxes, ow, oh, origin, r.0, r.1, r.2, r.3, radius, unit(t / 0.1));
}

/// Lay out and rasterise an animated caption `since` its start for a `w`×`h` frame.
pub fn render_animated(c: &Caption, style: &CaptionStyle, since: Tick, w: usize, h: usize) -> Option<Overlay> {
    if w == 0 || h == 0 {
        return None;
    }
    let anim = style.animation;
    let lines = tokens(c, style.uppercase);
    let flat: Vec<&String> = lines.iter().flatten().collect();
    if flat.is_empty() {
        return None;
    }
    let times = timings(c, &flat.iter().map(|s| s.chars().count()).collect::<Vec<_>>());
    let t = since.seconds() as f32;
    let t = if t.is_finite() { t.max(0.0) } else { 0.0 };

    // ---- size: by picture area, shrunk when a word (or One Word's word) does not fit
    let scale = ((w as f32 * h as f32) / (1920.0 * 1080.0)).sqrt();
    let max_w = w as f32 * 0.86;
    let tiles = anim == CaptionAnimation::Tiles;
    let fin = |v: f32, d: f32| if v.is_finite() { v } else { d };
    let mut px = (fin(style.size, 54.0).clamp(4.0, MAX_CAPTION_SIZE_1080) * scale).clamp(4.0, (h as f32).max(4.0));
    let face = |px: f32| caption_style(&style.font, &style.font_style, px);
    let widest = flat.iter().map(|s| filmcraft_text::measure(s, &face(px))).fold(0.0f32, f32::max);
    let room = max_w - if tiles { px * 0.7 } else { 0.0 };
    if widest > room && widest > 0.0 {
        px = (px * room / widest).max(4.0);
    }
    let ts: TextStyle = face(px);
    let gap = (filmcraft_text::measure("x x", &ts) - 2.0 * filmcraft_text::measure("x", &ts)).max(px * 0.2) + if tiles { px * 0.55 } else { 0.0 };

    // ---- words into lines (source lines re-wrapped to the frame; One Word shows one at a time)
    let (words, line) = wrap(&lines, &times, &ts, anim == CaptionAnimation::OneWord, max_w, gap);
    let n_lines = if anim == CaptionAnimation::OneWord { 1 } else { line.max(1) };
    let line_w = |l: usize| {
        if anim == CaptionAnimation::OneWord {
            return words.get(active(&words, t).unwrap_or(0)).map_or(0.0, |w| w.w);
        }
        words.iter().filter(|w| w.line == l).map(|w| w.x + w.w).fold(0.0f32, f32::max)
    };

    // ---- block placement
    let vm = filmcraft_text::fonts::face(filmcraft_text::resolve(&ts.family, &ts.style).face).metrics(px);
    let (asc, desc, cap) = (vm.ascent, vm.descent, vm.cap_height.max(px * 0.5));
    let lh = px * fin(style.line_spacing, 1.25).clamp(0.9, 3.0) * if tiles { 1.25 } else { 1.0 };
    let block_h = lh * n_lines as f32;
    let (line_override, align_override) = vtt_overrides(&c.settings);
    let align = align_override.unwrap_or(style.align);
    let margin = fin(style.margin, 0.08).clamp(0.0, 0.45) * h as f32;
    let top = match (line_override, style.anchor) {
        (Some(f), _) => (f * h as f32).min(h as f32 - block_h),
        (None, CaptionAnchor::Top) => margin,
        (None, CaptionAnchor::Middle) => (h as f32 - block_h) / 2.0,
        (None, CaptionAnchor::Bottom) => h as f32 - margin - block_h,
    }
    .max(0.0)
    .round();
    let side = w as f32 * 0.07;
    let line_x = |l: usize| match align {
        CaptionAlign::Left => side,
        CaptionAlign::Right => w as f32 - side - line_w(l),
        CaptionAlign::Center => (w as f32 - line_w(l)) / 2.0,
    };

    // ---- the overlay rectangle: the block plus room for overshoot, outline and shadow
    let outline = (fin(style.outline, 0.0) * scale).clamp(0.0, MAX_OUTLINE_RADIUS);
    let shadow = fin(style.shadow, 0.0).clamp(0.0, 1.0);
    let shadow_r = if shadow > 0.0 { (px * 0.1).round().clamp(1.0, 40.0) } else { 0.0 };
    let pad = px * 0.5 + outline.ceil() + shadow_r * 3.0 + 2.0;
    let widest_line = (0..n_lines).map(line_w).fold(0.0f32, f32::max).max(widest.min(room));
    let bx0 = match align {
        CaptionAlign::Left => side,
        CaptionAlign::Right => w as f32 - side - widest_line,
        CaptionAlign::Center => (w as f32 - widest_line) / 2.0,
    };
    let (left, right) = ((bx0 - pad).floor().clamp(0.0, w as f32), (bx0 + widest_line + pad).ceil().clamp(0.0, w as f32));
    let (otop, bottom) = ((top - pad).floor().clamp(0.0, h as f32), (top + block_h + pad).ceil().clamp(0.0, h as f32));
    if right <= left || bottom <= otop {
        return None;
    }
    let origin = (left as i32, otop as i32);
    let (ow, oh) = ((right - left) as usize, (bottom - otop) as usize);
    if ow.checked_mul(oh).is_none_or(|pixels| pixels > MAX_OVERLAY_PIXELS) {
        return None;
    }

    // ---- draw
    let mut text = vec![0.0f32; ow * oh];
    let mut hi = vec![0.0f32; ow * oh];
    let mut boxes = vec![0.0f32; ow * oh];
    let mut bg = vec![0.0f32; ow * oh];
    let mut scratch = filmcraft_text::Mask { w: ow, h: oh, a: vec![0.0f32; ow * oh] };
    let baseline_of = |l: usize| (top + lh * l as f32 + (lh - (asc + desc)) / 2.0 + asc).round();
    let pad_x = px * 0.28;
    let pad_y = px * 0.16;
    let radius = px * 0.22;
    // the rectangle around word `i` (for boxes and tiles), at scale 1
    let rect_of = |wd: &Word| {
        let (x, b) = (line_x(wd.line) + wd.x, baseline_of(wd.line));
        (x - pad_x, b - cap - pad_y, x + wd.w + pad_x, b + pad_y)
    };
    if style.background && style.background_color[3] > 0 && anim != CaptionAnimation::OneWord {
        for l in 0..n_lines {
            let (x, b) = (line_x(l), baseline_of(l));
            round_rect(&mut bg, ow, oh, origin, x - pad_x * 1.4, b - cap - pad_y * 1.6, x + line_w(l) + pad_x * 1.4, b + pad_y * 1.6, radius, unit(t / 0.1));
        }
    }
    if anim == CaptionAnimation::Box {
        glide_box(&mut boxes, (ow, oh), origin, &words, t, radius, &rect_of);
    }
    for (i, wd) in words.iter().enumerate() {
        let d = draw_of(anim, &words, i, t);
        if d.alpha <= 0.0 {
            continue;
        }
        // whole pixel sizes keep the glyph cache small while a word scales
        let spx = (px * d.scale).round().max(1.0);
        let s = spx / px;
        let (x, b) = (line_x(wd.line) + wd.x, baseline_of(wd.line) + d.dy * px * 0.4);
        let (cx, cy) = (x + wd.w / 2.0, b - cap / 2.0);
        if tiles {
            let r = rect_of(wd);
            let grow = |v: f32, c: f32| c + (v - c) * s;
            round_rect(&mut boxes, ow, oh, origin, grow(r.0, cx), grow(r.1, cy), grow(r.2, cx), grow(r.3, cy), radius * s, d.alpha);
        }
        scratch.a.fill(0.0);
        let l = layout(&wd.text, &ts, &ParagraphStyle::default());
        render::draw(&l, &[s, 0.0, 0.0, s, cx - s * wd.w / 2.0, (cy + s * (b - cy)).round()], &mut scratch, origin);
        // the column where the Karaoke fill ends
        let fill_x = if anim == CaptionAnimation::Karaoke { (x + wd.w * d.fill - left).round() as i64 } else { i64::MIN };
        let (x_lo, x_hi) = (((x - pad - left).floor().max(0.0)) as usize, ((x + wd.w + pad - left).ceil().clamp(0.0, ow as f32)) as usize);
        let fill = (anim == CaptionAnimation::Karaoke).then_some(fill_x);
        splat(&scratch.a, &mut text, &mut hi, (ow, oh), (x_lo, x_hi.min(ow)), d.alpha, fill, d.mix);
    }

    // ---- compose
    let glyphs: Vec<f32> = text.iter().zip(&hi).map(|(a, b)| (a + b).min(1.0)).collect();
    let stroke = if outline > 0.0 { dilate(&glyphs, ow, oh, outline) } else { Vec::new() };
    let shade = if shadow > 0.0 {
        // everything solid, moved down a little and blurred
        let off = ((px * 0.07).round() as usize).min(oh);
        let mut m = vec![0.0f32; ow * oh];
        for y in off..oh {
            for x in 0..ow {
                let j = (y - off) * ow + x;
                m[y * ow + x] = glyphs[j].max(boxes[j]).max(stroke.get(j).copied().unwrap_or(0.0));
            }
        }
        blur(&mut m, ow, oh, shadow_r as usize);
        m
    } else {
        Vec::new()
    };
    let (tc, hc, oc, bc) = (lin(style.color), lin(style.highlight_color), lin(style.outline_color), lin(style.background_color));
    // Karaoke: the words still to come are dimmed
    let dim = if anim == CaptionAnimation::Karaoke { 0.6 } else { 1.0 };
    let mut out = vec![0.0f32; ow * oh * 4];
    for j in 0..ow * oh {
        let mut p = [0.0f32; 4];
        let over = |p: &mut [f32; 4], c: [f32; 4], a: f32| {
            if a > 0.0 {
                for k in 0..4 {
                    p[k] = c[k] * a + p[k] * (1.0 - c[3] * a);
                }
            }
        };
        if let Some(s) = shade.get(j) {
            over(&mut p, [0.0, 0.0, 0.0, 1.0], s * shadow * 0.75);
        }
        over(&mut p, bc, bg[j]);
        over(&mut p, hc, boxes[j]);
        if let Some(s) = stroke.get(j) {
            over(&mut p, oc, *s);
        }
        over(&mut p, tc, text[j] * dim);
        over(&mut p, hc, hi[j]);
        out[j * 4..j * 4 + 4].copy_from_slice(&p);
    }
    Some(Overlay { x: origin.0, y: origin.1, w: ow, h: oh, px: out })
}

#[cfg(test)]
mod tests {
    use super::*;
    use filmcraft_project::{CaptionWord, ClipId};
    use filmcraft_time::TICKS_PER_SECOND;

    fn secs(s: f64) -> Tick {
        Tick::from_seconds_f64(s)
    }

    /// "make it loud now": four words, half a second each.
    fn cap() -> Caption {
        let words = ["make", "it", "loud", "now"]
            .iter()
            .enumerate()
            .map(|(i, w)| CaptionWord { text: w.to_string(), start: secs(i as f64 * 0.5), end: secs(i as f64 * 0.5 + 0.4) })
            .collect();
        Caption {
            id: ClipId(1),
            start: Tick(0),
            duration: Tick(2 * TICKS_PER_SECOND),
            text: "make it\nloud now".into(),
            speaker: None,
            cue_id: None,
            settings: String::new(),
            words,
        }
    }

    fn style(animation: CaptionAnimation) -> CaptionStyle {
        CaptionStyle { animation, size: 84.0, background: false, outline: 5.0, font_style: "Bold".into(), uppercase: true, margin: 0.2, ..Default::default() }
    }

    fn ink(o: &Overlay) -> f32 {
        o.px.chunks(4).map(|p| p[3]).sum()
    }

    /// Pixels close to the highlight colour (yellow: red and green high, blue low).
    fn yellow(o: &Overlay) -> usize {
        o.px.chunks(4).filter(|p| p[3] > 0.9 && p[0] > 0.7 && p[1] > 0.5 && p[2] < 0.3).count()
    }

    #[test]
    fn pop_reveals_words_as_they_are_spoken_and_highlights_the_current_one() {
        let (c, st) = (cap(), style(CaptionAnimation::Pop));
        let at = |s: f64| render_animated(&c, &st, secs(s), 1080, 1920).expect("overlay");
        let (one, two, all) = (ink(&at(0.3)), ink(&at(0.8)), ink(&at(1.9)));
        assert!(one > 0.0 && two > one && all > two * 1.5, "{one} {two} {all}");
        assert!(yellow(&at(0.3)) > 200, "the spoken word is highlighted");
        // the highlight moves on: at 1.9 s only "now" is yellow, far less than the whole block
        let o = at(1.9);
        assert!(yellow(&o) > 200 && (yellow(&o) as f32) < 0.5 * o.px.chunks(4).filter(|p| p[3] > 0.9).count() as f32);
        // overlays stay inside the frame
        for s in [0.0, 0.31, 1.0, 1.99] {
            let o = at(s);
            assert!(o.x >= 0 && o.y >= 0 && o.x as usize + o.w <= 1080 && o.y as usize + o.h <= 1920);
            assert!(o.px.iter().all(|v| v.is_finite() && *v >= 0.0 && *v <= 1.0001));
        }
    }

    #[test]
    fn every_animation_draws_in_portrait_and_landscape() {
        for anim in CaptionAnimation::ALL.into_iter().filter(|a| *a != CaptionAnimation::None) {
            for (w, h) in [(1080, 1920), (1920, 1080), (64, 36)] {
                let o = render_animated(&cap(), &style(anim), secs(1.2), w, h).unwrap_or_else(|| panic!("{anim:?} {w}x{h}"));
                assert!(ink(&o) > 0.0, "{anim:?} {w}x{h}");
            }
        }
    }

    #[test]
    fn one_word_shows_one_word_and_karaoke_fills() {
        let c = cap();
        let one = |s: f64| render_animated(&c, &style(CaptionAnimation::OneWord), secs(s), 1080, 1920).expect("overlay");
        // "it" is narrower than "loud"
        let width = |o: &Overlay| {
            let cols: Vec<usize> = (0..o.w).filter(|x| (0..o.h).any(|y| o.px[(y * o.w + x) * 4 + 3] > 0.5)).collect();
            cols.last().copied().unwrap_or(0) - cols.first().copied().unwrap_or(0)
        };
        assert!(width(&one(0.9)) < width(&one(1.4)));
        let k = |s: f64| yellow(&render_animated(&c, &style(CaptionAnimation::Karaoke), secs(s), 1080, 1920).expect("overlay"));
        assert!(k(0.05) < k(0.9) && k(0.9) < k(1.95), "{} {} {}", k(0.05), k(0.9), k(1.95));
    }

    #[test]
    fn outline_grows_by_a_disc() {
        // one pixel grows into a disc of the radius; nothing leaks across the edges
        let (w, h) = (21, 21);
        let mut m = vec![0.0f32; w * h];
        m[10 * w + 10] = 1.0;
        let o = dilate(&m, w, h, 4.0);
        let at = |x: usize, y: usize| o[y * w + x];
        assert_eq!((at(10, 10), at(14, 10), at(10, 6), at(13, 13)), (1.0, 1.0, 1.0, 1.0));
        assert_eq!((at(15, 10), at(10, 15), at(14, 14), at(0, 0)), (0.0, 0.0, 0.0, 0.0));
        m[0] = 1.0;
        let o = dilate(&m, w, h, 2.0);
        assert_eq!((o[2], o[2 * w], o[3]), (1.0, 1.0, 0.0));
        assert!(dilate(&m, w, h, f32::NAN).iter().all(|v| *v == 0.0));
        assert_eq!(dilate(&m, w, h, 1e9).len(), w * h);
    }

    #[test]
    fn edited_text_and_hostile_values_still_draw() {
        // the text no longer matches the word times: words share the duration
        let mut c = cap();
        c.text = "make it very very loud right now".into();
        assert!(render_animated(&c, &style(CaptionAnimation::Pop), secs(1.9), 1080, 1920).is_some());
        // a word wider than the frame shrinks instead of leaving it
        c.text = "Donaudampfschifffahrtsgesellschaftskapitän".into();
        let o = render_animated(&c, &style(CaptionAnimation::Tiles), secs(1.0), 1080, 1920).expect("overlay");
        assert!(o.w <= 1080);
        c.text = "   \n ".into();
        assert!(render_animated(&c, &style(CaptionAnimation::Pop), secs(1.0), 1080, 1920).is_none());
        let mut c = cap();
        c.duration = Tick(0);
        c.words[0].start = Tick(i64::MAX / 4);
        c.words[1].end = Tick(-5);
        for (size, outline, shadow, t) in [(f32::NAN, f32::NAN, f32::NAN, 1.0), (1e9, 1e9, 1e9, -3.0), (-5.0, -5.0, -5.0, 1e12)] {
            let st = CaptionStyle { size, outline, shadow, margin: f32::NAN, line_spacing: f32::NAN, ..style(CaptionAnimation::Box) };
            for (w, h) in [(270, 480), (1, 1), (0, 10), (5000, 3)] {
                let _ = render_animated(&c, &st, secs(t), w, h);
            }
        }
    }
}

/// Writes preview frames of every look (raw sRGB8 RGB, 540×960 at 30 fps, 2.4 s each, one file per
/// look) to the folder in `FILMCRAFT_CAPTION_PREVIEW`:
/// `cargo test -p filmcraft-captions caption_preview -- --ignored`.
#[cfg(test)]
#[test]
#[ignore = "writes preview frames"]
fn caption_preview() {
    use filmcraft_project::{CAPTION_LOOKS, CaptionWord, ClipId};
    let Some(dir) = std::env::var_os("FILMCRAFT_CAPTION_PREVIEW") else { return };
    let secs = Tick::from_seconds_f64;
    let said = [("Das", 0.05, 0.25), ("ist", 0.3, 0.5), ("mein", 0.55, 0.8), ("neuer", 0.85, 1.2), ("Beat", 1.3, 1.9)];
    let c = Caption {
        id: ClipId(1),
        start: Tick(0),
        duration: secs(2.4),
        text: "Das ist mein\nneuer Beat".into(),
        speaker: None,
        cue_id: None,
        settings: String::new(),
        words: said.iter().map(|(w, a, b)| CaptionWord { text: w.to_string(), start: secs(*a), end: secs(*b) }).collect(),
    };
    let (w, h) = (540usize, 960usize);
    let started = std::time::Instant::now();
    let mut frames = 0;
    for look in CAPTION_LOOKS {
        let st = CaptionStyle::look(look).expect("look");
        let mut out = Vec::new();
        for f in 0..72 {
            // a dusk gradient as the "video"
            let mut canvas = vec![0.0f32; w * h * 4];
            for (i, p) in canvas.chunks_mut(4).enumerate() {
                let (x, y) = ((i % w) as f32 / w as f32, (i / w) as f32 / h as f32);
                p.copy_from_slice(&[0.25 - 0.2 * y, 0.12 + 0.1 * x, 0.3 - 0.1 * y, 1.0]);
            }
            if let Some(o) = render_animated(&c, &st, secs(f as f64 / 30.0), w, h) {
                o.composite_onto(&mut canvas, w, h);
            }
            frames += 1;
            out.extend(canvas.chunks(4).flat_map(|p| [p[0], p[1], p[2]].map(|v| (filmcraft_color::linear_to_srgb(v.clamp(0.0, 1.0)) * 255.0).round() as u8)));
        }
        std::fs::write(std::path::Path::new(&dir).join(format!("{}.rgb", look.replace(' ', ""))), out).expect("write");
    }
    eprintln!("{:.1} ms per frame", started.elapsed().as_secs_f64() * 1000.0 / frames as f64);
}
