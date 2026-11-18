//! FilmCraft Plus **Chart** (`chart`, definition in `filmcraft_project::effect::plus`): an
//! animated bar, line, pie or donut chart from numbers typed as text.
//!
//! The values come from one line of text, `Jan 12; Feb 18; Mar 9` (see [`parse`]). The chart
//! grows in on the clip's own time: bars rise one after another while their numbers count up, a
//! line is drawn from left to right, a pie fills clockwise. It is drawn over the clip's picture
//! (put it on a Transparent Video to have it as its own layer).
//!
//! Original FilmCraft drawing code. Images are premultiplied linear f32; colour parameters are
//! display-encoded.

use filmcraft_geom::Vec2;
use filmcraft_project::EffectInstance;

use super::draw::{progress, stamp};
use super::text::{TextLook, draw_text};
use super::{FxCtx, Image, bv, chv, cv, dec, fv, pv, smoothstep, tv};

/// Most entries a chart shows.
const MAX_ENTRIES: usize = 24;
/// Longest label kept (characters).
const MAX_LABEL: usize = 28;

fn finite_or(v: f32, d: f32) -> f32 {
    if v.is_finite() { v } else { d }
}

/// The entries of a chart from its text: `label value` pairs separated by `;` or line breaks
/// (then a decimal comma is understood), or else by commas. The value is the last number of an
/// entry; `:` and `=` may stand between label and value, and a unit may follow the number
/// (`Miete: 450 €`). Entries without a number are left out.
pub(crate) fn parse(text: &str) -> Vec<(String, f64)> {
    let strong = text.contains(';') || text.contains('\n');
    let parts: Vec<&str> = if strong { text.split([';', '\n']).collect() } else { text.split(',').collect() };
    let mut out = Vec::new();
    for part in parts {
        let tokens: Vec<&str> = part.split(|c: char| c.is_whitespace() || c == ':' || c == '=').filter(|t| !t.is_empty()).collect();
        let number = |t: &str| -> Option<f64> {
            let t = t.trim_matches(|c: char| !(c.is_ascii_digit() || c == '-' || c == '.' || c == ','));
            let t = if strong { t.replace(',', ".") } else { t.to_string() };
            t.parse::<f64>().ok().filter(|v| v.is_finite()).map(|v| v.clamp(-1.0e12, 1.0e12))
        };
        let Some((at, value)) = tokens.iter().enumerate().rev().find_map(|(i, t)| number(t).map(|v| (i, v))) else { continue };
        let label: String = tokens[..at].join(" ").chars().filter(|c| !c.is_control()).take(MAX_LABEL).collect();
        out.push((label, value));
        if out.len() >= MAX_ENTRIES {
            break;
        }
    }
    out
}

/// A number as the chart writes it: whole when every value is whole, else one decimal.
fn number(v: f64, whole: bool, unit: &str) -> String {
    let s = if whole { format!("{:.0}", v) } else { format!("{:.1}", v) };
    // "-0" reads oddly while counting up
    let s = if s.trim_start_matches('-').chars().all(|c| c == '0' || c == '.') { s.trim_start_matches('-').to_string() } else { s };
    if unit.is_empty() { s } else { format!("{s}{}{unit}", if unit.chars().next().is_some_and(char::is_alphanumeric) { " " } else { "" }) }
}

fn ease_out(p: f32) -> f32 {
    1.0 - (1.0 - p.clamp(0.0, 1.0)).powi(3)
}

/// Ease out that swings a little past 1 and settles.
fn spring(p: f32) -> f32 {
    let q = p.clamp(0.0, 1.0) - 1.0;
    1.0 + 2.2 * q * q * q + 1.2 * q * q
}

/// The colours of a palette (display-encoded), cycled.
fn palette(kind: u32, base: [f32; 3], i: usize) -> [f32; 3] {
    const AVKO: [[f32; 3]; 6] = [[1.0, 0.84, 0.2], [0.49, 0.23, 0.93], [0.24, 1.0, 0.71], [0.93, 0.93, 0.93], [1.0, 0.33, 0.33], [0.3, 0.64, 1.0]];
    const NEON: [[f32; 3]; 6] = [[0.0, 1.0, 0.9], [1.0, 0.0, 0.8], [0.6, 1.0, 0.0], [1.0, 0.6, 0.0], [0.4, 0.4, 1.0], [1.0, 1.0, 1.0]];
    const PASTEL: [[f32; 3]; 6] = [[0.98, 0.7, 0.7], [0.7, 0.85, 0.98], [0.75, 0.95, 0.75], [0.98, 0.9, 0.6], [0.85, 0.75, 0.98], [0.95, 0.95, 0.95]];
    const SHADES: [f32; 6] = [1.0, 0.8, 0.62, 0.48, 0.36, 0.28];
    match kind {
        0 => AVKO[i % 6],
        1 => base,
        2 => NEON[i % 6],
        3 => PASTEL[i % 6],
        _ => base.map(|c| c * SHADES[i % 6]),
    }
}

/// Paint `lin` (linear, straight) over a premultiplied pixel with coverage `a`.
fn over(p: &mut [f32], lin: [f32; 3], a: f32) {
    let a = if a.is_finite() { a.clamp(0.0, 1.0) } else { 0.0 };
    if a <= 0.0 || p.len() < 4 {
        return;
    }
    for i in 0..3 {
        p[i] = p[i] * (1.0 - a) + lin[i] * a;
    }
    p[3] += (1.0 - p[3]) * a;
}

/// A filled rectangle `x0..x1` × `y0..y1` with corners rounded by `r`.
fn rect(img: &mut Image, (x0, y0, x1, y1): (f32, f32, f32, f32), r: f32, lin: [f32; 3], alpha: f32) {
    if !(x0.is_finite() && y0.is_finite() && x1.is_finite() && y1.is_finite()) || x1 <= x0 || y1 <= y0 || alpha <= 0.0 {
        return;
    }
    let (w, h) = (img.w, img.h);
    let r = r.clamp(0.0, (x1 - x0).min(y1 - y0) / 2.0);
    let (cx, cy, hx, hy) = ((x0 + x1) / 2.0, (y0 + y1) / 2.0, (x1 - x0) / 2.0, (y1 - y0) / 2.0);
    let (ax, bx) = ((x0 - 1.0).floor().max(0.0) as usize, ((x1 + 1.0).ceil().max(0.0) as usize).min(w));
    let (ay, by) = ((y0 - 1.0).floor().max(0.0) as usize, ((y1 + 1.0).ceil().max(0.0) as usize).min(h));
    for y in ay..by {
        for x in ax..bx {
            // distance to the rounded box
            let (qx, qy) = (((x as f32 + 0.5) - cx).abs() - (hx - r), ((y as f32 + 0.5) - cy).abs() - (hy - r));
            let d = qx.max(0.0).hypot(qy.max(0.0)) + qx.max(qy).min(0.0) - r;
            let c = (0.5 - d).clamp(0.0, 1.0);
            if c > 0.0 {
                over(&mut img.px[(y * w + x) * 4..(y * w + x) * 4 + 4], lin, c * alpha);
            }
        }
    }
}

/// A ring segment round `c` from angle `a0` to `a1` (turns, clockwise from the top), between
/// the radii `r_in` and `r_out`.
fn wedge(img: &mut Image, c: Vec2, (r_in, r_out): (f32, f32), (a0, a1): (f32, f32), lin: [f32; 3], alpha: f32) {
    if !(c.x.is_finite() && c.y.is_finite() && r_out.is_finite()) || r_out <= 0.0 || a1 <= a0 || alpha <= 0.0 {
        return;
    }
    let (w, h) = (img.w, img.h);
    let (cx, cy) = (c.x as f32, c.y as f32);
    let (ax, bx) = ((cx - r_out - 1.0).floor().max(0.0) as usize, ((cx + r_out + 1.0).ceil().max(0.0) as usize).min(w));
    let (ay, by) = ((cy - r_out - 1.0).floor().max(0.0) as usize, ((cy + r_out + 1.0).ceil().max(0.0) as usize).min(h));
    for y in ay..by {
        for x in ax..bx {
            let mut cover = 0.0f32;
            for (sx, sy) in [(0.25f32, 0.25f32), (0.75, 0.25), (0.25, 0.75), (0.75, 0.75)] {
                let (dx, dy) = (x as f32 + sx - cx, y as f32 + sy - cy);
                let d = dx.hypot(dy);
                if d > r_out || d < r_in {
                    continue;
                }
                // clockwise from the top, in turns
                let mut turn = dx.atan2(-dy) / std::f32::consts::TAU;
                if turn < 0.0 {
                    turn += 1.0;
                }
                if (turn >= a0 && turn < a1) || (a1 > 1.0 && turn < a1 - 1.0) {
                    cover += 0.25;
                }
            }
            if cover > 0.0 {
                over(&mut img.px[(y * w + x) * 4..(y * w + x) * 4 + 4], lin, cover * alpha);
            }
        }
    }
}

/// A round-ended polyline.
fn line(img: &mut Image, pts: &[Vec2], radius: f32, lin: [f32; 3], alpha: f32) {
    if pts.len() < 2 || alpha <= 0.0 {
        return;
    }
    let (w, h) = (img.w, img.h);
    let mut cover = vec![0.0f32; w * h];
    for s in pts.windows(2) {
        if s[0].x.is_finite() && s[0].y.is_finite() && s[1].x.is_finite() && s[1].y.is_finite() {
            stamp(&mut cover, w, h, s[0], s[1], radius);
        }
    }
    for (p, c) in img.px.chunks_mut(4).zip(&cover) {
        if *c > 0.0 {
            over(p, lin, c * alpha);
        }
    }
}

/// Text in the chart's font; `align` 0 left, 1 centre, 2 right.
fn label(img: &mut Image, text: &str, pos: (f32, f32), px: f32, align: u32, color: [f32; 3], alpha: f32) {
    if alpha <= 0.003 || px < 3.0 || text.is_empty() {
        return;
    }
    let look = TextLook {
        family: "Inter",
        pos: Vec2::new(f64::from(pos.0), f64::from(pos.1)),
        px,
        align,
        color: [color[0], color[1], color[2], alpha.clamp(0.0, 1.0)],
        box_alpha: 0.0,
    };
    draw_text(img, text, look);
}

/// Everything the chart types share.
struct Sheet<'a> {
    entries: &'a [(String, f64)],
    unit: &'a str,
    whole: bool,
    /// Plot area: left, top, right, bottom (layer pixels).
    plot: (f32, f32, f32, f32),
    /// Text size.
    px: f32,
    /// 0..1 through the draw time.
    drawn: f32,
    /// Opacity of everything (falls while the chart is taken away).
    alpha: f32,
    palette: u32,
    base: [f32; 3],
    text: [f32; 3],
    values: bool,
}

impl Sheet<'_> {
    fn colour(&self, i: usize) -> [f32; 3] {
        dec(palette(self.palette, self.base, i))
    }
    /// How far entry `i` is (0..1): the entries start one after another and all end with the
    /// draw time.
    fn step(&self, i: usize) -> f32 {
        let n = self.entries.len().max(1) as f32;
        let each = 0.5;
        let start = (1.0 - each) * i as f32 / n.max(1.0);
        ((self.drawn - start) / each).clamp(0.0, 1.0)
    }
}

fn bars(img: &mut Image, s: &Sheet, horizontal: bool) {
    let (x0, y0, x1, y1) = s.plot;
    let n = s.entries.len();
    let top = s.entries.iter().map(|e| e.1).fold(0.0f64, f64::max).max(1e-9);
    let axis = dec([s.text[0], s.text[1], s.text[2]]);
    let room = s.px * 1.6;
    if horizontal {
        // labels left of the bars (a third of the width at most), values at the bar ends
        let widest = s.entries.iter().map(|e| e.0.chars().count()).max().unwrap_or(0) as f32;
        let left = x0 + (widest * s.px * 0.58 + s.px * 0.6).min((x1 - x0) * 0.34);
        let slot = (y1 - y0) / n as f32;
        let span = (x1 - left - s.px * 4.5).max(4.0);
        rect(img, (left - 1.5, y0, left + 1.5, y1), 1.0, axis, 0.5 * s.alpha * smoothstep(0.0, 0.15, s.drawn));
        for (i, (name, v)) in s.entries.iter().enumerate() {
            let p = s.step(i);
            let len = span * (v.max(0.0) / top) as f32 * spring(p).max(0.0);
            let (cy, half) = (y0 + slot * (i as f32 + 0.5), slot * 0.31);
            let fade = s.alpha * smoothstep(0.0, 0.3, p);
            rect(img, (left, cy - half, left + len, cy + half), half * 0.35, s.colour(i), s.alpha * smoothstep(0.0, 0.1, p));
            label(img, name, (left - s.px * 0.5, cy), s.px, 2, s.text, fade);
            if s.values {
                label(img, &number(v * f64::from(ease_out(p)), s.whole, s.unit), (left + len + s.px * 0.4, cy), s.px, 0, s.text, fade);
            }
        }
        return;
    }
    let (base, slot) = (y1 - room, (x1 - x0) / n as f32);
    let span = (base - y0 - if s.values { room } else { 0.0 }).max(4.0);
    rect(img, (x0, base - 1.5, x1, base + 1.5), 1.0, axis, 0.5 * s.alpha * smoothstep(0.0, 0.15, s.drawn));
    for (i, (name, v)) in s.entries.iter().enumerate() {
        let p = s.step(i);
        let height = span * (v.max(0.0) / top) as f32 * spring(p).max(0.0);
        let (cx, half) = (x0 + slot * (i as f32 + 0.5), slot * 0.31);
        let fade = s.alpha * smoothstep(0.0, 0.3, p);
        rect(img, (cx - half, base - height, cx + half, base), half * 0.3, s.colour(i), s.alpha * smoothstep(0.0, 0.1, p));
        label(img, name, (cx, base + room * 0.55), s.px.min(slot * 0.34), 1, s.text, fade);
        if s.values {
            label(img, &number(v * f64::from(ease_out(p)), s.whole, s.unit), (cx, base - height - room * 0.45), s.px.min(slot * 0.4), 1, s.text, fade);
        }
    }
}

fn graph(img: &mut Image, s: &Sheet) {
    let (x0, y0, x1, y1) = s.plot;
    let n = s.entries.len();
    let room = s.px * 1.6;
    let (base, top_y) = (y1 - room, y0 + if s.values { room } else { s.px * 0.5 });
    let (lo, hi) = s.entries.iter().fold((0.0f64, f64::MIN), |(lo, hi), e| (lo.min(e.1), hi.max(e.1)));
    let range = (hi - lo).max(1e-9);
    let slot = (x1 - x0) / n as f32;
    let at = |i: usize| -> Vec2 {
        let v = s.entries.get(i).map_or(0.0, |e| e.1);
        Vec2::new(f64::from(x0 + slot * (i as f32 + 0.5)), f64::from(base) - f64::from(base - top_y) * (v - lo) / range)
    };
    let axis = dec(s.text);
    rect(img, (x0, base - 1.5, x1, base + 1.5), 1.0, axis, 0.5 * s.alpha * smoothstep(0.0, 0.15, s.drawn));
    let colour = s.colour(0);
    // the pen: from the first point to the last over the draw time
    let reach = ease_out(s.drawn) * (n.saturating_sub(1)) as f32;
    let mut pts: Vec<Vec2> = Vec::new();
    for i in 0..n {
        if (i as f32) <= reach {
            pts.push(at(i));
        } else {
            let (a, b, f) = (at(i - 1), at(i), f64::from(reach - (i - 1) as f32));
            if f > 0.0 {
                pts.push(Vec2::new(a.x + (b.x - a.x) * f, a.y + (b.y - a.y) * f));
            }
            break;
        }
    }
    // a soft fill under the line, as far as the pen is
    for w in pts.windows(2) {
        let (a, b) = (w[0], w[1]);
        let (xa, xb) = (a.x.floor().max(0.0) as usize, (b.x.ceil().max(0.0) as usize).min(img.w));
        for x in xa..xb {
            let f = if (b.x - a.x).abs() > 1e-9 { ((x as f64 + 0.5 - a.x) / (b.x - a.x)).clamp(0.0, 1.0) } else { 0.0 };
            let top = (a.y + (b.y - a.y) * f) as f32;
            rect(img, (x as f32, top, x as f32 + 1.0, base), 0.0, colour, 0.16 * s.alpha);
        }
    }
    let thick = (s.px * 0.11).max(1.2);
    line(img, &pts, thick, colour, s.alpha);
    for i in 0..n {
        let p = ((reach - i as f32 + 0.25) / 0.6).clamp(0.0, 1.0);
        let (name, v) = &s.entries[i];
        let c = at(i);
        if p > 0.0 {
            let r = thick * 2.3 * spring(p).max(0.0);
            wedge(img, c, (0.0, r), (0.0, 1.0), colour, s.alpha);
            wedge(img, c, (0.0, r * 0.45), (0.0, 1.0), dec([0.05, 0.05, 0.07]), s.alpha);
            if s.values {
                label(img, &number(*v, s.whole, s.unit), (c.x as f32, c.y as f32 - room * 0.55), s.px.min(slot * 0.4), 1, s.text, s.alpha * p);
            }
        }
        label(img, name, (c.x as f32, base + room * 0.55), s.px.min(slot * 0.34), 1, s.text, s.alpha * smoothstep(0.0, 0.2, s.drawn));
    }
}

fn pie(img: &mut Image, s: &Sheet, donut: bool) {
    let (x0, y0, x1, y1) = s.plot;
    let total: f64 = s.entries.iter().map(|e| e.1.max(0.0)).sum();
    if total <= 0.0 {
        return;
    }
    let r = ((y1 - y0) / 2.0).min((x1 - x0) * 0.3).max(2.0);
    let c = Vec2::new(f64::from(x0 + r), f64::from((y0 + y1) / 2.0));
    let inner = if donut { r * 0.58 } else { 0.0 };
    // the whole pie fills clockwise over the draw time
    let sweep = ease_out(s.drawn);
    let mut from = 0.0f32;
    let rows = s.entries.len() as f32;
    let row_h = ((y1 - y0) / rows.max(1.0)).min(s.px * 1.9);
    let legend_x = x0 + 2.0 * r + s.px * 1.4;
    let legend_y = (y0 + y1) / 2.0 - row_h * (rows - 1.0) / 2.0;
    for (i, (name, v)) in s.entries.iter().enumerate() {
        let share = (v.max(0.0) / total) as f32;
        let to = (from + share).min(sweep);
        if to > from {
            wedge(img, c, (inner, r), (from, to), s.colour(i), s.alpha);
        }
        // its legend row appears when its slice starts
        let p = ((sweep - from) / share.max(0.02)).clamp(0.0, 1.0);
        let y = legend_y + row_h * i as f32;
        let fade = s.alpha * smoothstep(0.0, 0.4, p);
        if fade > 0.0 {
            let dot = s.px * 0.36;
            rect(img, (legend_x - dot, y - dot, legend_x + dot, y + dot), dot * 0.5, s.colour(i), fade);
            let text = if s.values {
                let shown = number(f64::from(share) * 100.0 * f64::from(p), true, "%");
                if name.is_empty() { shown } else { format!("{name}  {shown}") }
            } else {
                name.clone()
            };
            label(img, &text, (legend_x + s.px, y), s.px.min(row_h * 0.62), 0, s.text, fade);
        }
        from += share;
    }
}

/// Chart: an animated bar, line, pie or donut chart from typed numbers.
pub fn chart(img: &mut Image, e: &EffectInstance, cx: &FxCtx) {
    if img.w < 8 || img.h < 8 {
        return;
    }
    let (drawn, erased) = progress(e, cx);
    let alpha = (1.0 - erased) * (finite_or(fv(e, "opacity", cx), 100.0) / 100.0).clamp(0.0, 1.0);
    if drawn <= 0.0 || alpha <= 0.0 {
        return;
    }
    let entries = parse(&tv(e, "values"));
    if entries.is_empty() {
        return;
    }
    let kind = chv(e, "type");
    let (w, h) = (img.w as f32, img.h as f32);
    let centre = pv(e, "center", cx, img);
    let (mx, my) = (
        if centre.x.is_finite() { (centre.x as f32).clamp(-w, 2.0 * w) } else { w / 2.0 },
        if centre.y.is_finite() { (centre.y as f32).clamp(-h, 2.0 * h) } else { h / 2.0 },
    );
    let size = (finite_or(fv(e, "size", cx), 70.0) / 100.0).clamp(0.05, 2.0);
    // a 16:9 sheet, as wide as asked but never taller than the frame allows
    let bw = (w * size).min(h * size * 1.9).max(16.0);
    let bh = bw * if kind >= 3 { 0.5 } else { 0.56 };
    let (x0, y0) = (mx - bw / 2.0, my - bh / 2.0);
    let px = (bh * 0.058).clamp(4.0, 400.0);
    let text = {
        let c = cv(e, "text_color", cx);
        [c[0], c[1], c[2]]
    };
    let base = {
        let c = cv(e, "color", cx);
        [c[0], c[1], c[2]]
    };
    // the sheet arrives first
    let arrive = smoothstep(0.0, 0.12, drawn);
    let panel = (finite_or(fv(e, "panel", cx), 35.0) / 100.0).clamp(0.0, 1.0);
    let pad = px * 1.4;
    rect(img, (x0 - pad, y0 - pad, x0 + bw + pad, y0 + bh + pad), px * 1.2, dec([0.03, 0.03, 0.05]), panel * alpha * arrive);
    let title = tv(e, "title");
    let mut top = y0;
    if !title.trim().is_empty() {
        let title: String = title.chars().filter(|c| !c.is_control()).take(80).collect();
        label(img, &title, (x0, y0 + px * 0.9), px * 1.5, 0, text, alpha * arrive);
        top += px * 2.9;
    }
    let unit: String = tv(e, "unit").chars().filter(|c| !c.is_control()).take(8).collect();
    let sheet = Sheet {
        entries: &entries,
        unit: unit.trim(),
        whole: entries.iter().all(|e| e.1.fract().abs() < 1e-9),
        plot: (x0, top, x0 + bw, y0 + bh),
        px,
        drawn,
        alpha,
        palette: chv(e, "palette"),
        base,
        text,
        values: bv(e, "show_values"),
    };
    match kind {
        1 => bars(img, &sheet, true),
        2 => graph(img, &sheet),
        3 => pie(img, &sheet, false),
        4 => pie(img, &sheet, true),
        _ => bars(img, &sheet, false),
    }
    for v in &mut img.px {
        if !v.is_finite() {
            *v = 0.0;
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

    #[test]
    fn values_are_read_from_plain_text() {
        let labels = |t: &str| parse(t).into_iter().map(|(l, v)| format!("{l}={v}")).collect::<Vec<_>>();
        assert_eq!(labels("Jan 12; Feb 18; Mär 9"), ["Jan=12", "Feb=18", "Mär=9"]);
        assert_eq!(labels("A 1, B 2.5, C -3"), ["A=1", "B=2.5", "C=-3"]);
        // with ; or line breaks a decimal comma is a decimal comma
        assert_eq!(labels("Miete: 450,50 €; Strom = 62,3\nInternet 30"), ["Miete=450.5", "Strom=62.3", "Internet=30"]);
        assert_eq!(labels("10, 20, 30"), ["=10", "=20", "=30"]);
        assert_eq!(labels("Views 2024 1200; Likes 300%"), ["Views 2024=1200", "Likes=300"]);
        // nothing to show, junk and too much
        assert!(parse("").is_empty() && parse("nur Text; mehr Text").is_empty() && parse(";;;,,,").is_empty());
        assert_eq!(parse(&"x 1;".repeat(500)).len(), MAX_ENTRIES);
        assert!(parse("a 1e999; b NaN; c inf; d 5").iter().all(|e| e.1.is_finite()));
        assert_eq!(parse(&format!("{} 3", "l".repeat(500)))[0].0.chars().count(), MAX_LABEL);
        assert_eq!(number(12.0, true, ""), "12");
        assert_eq!(number(12.34, false, "%"), "12.3%");
        assert_eq!(number(450.0, true, "€"), "450€");
        assert_eq!(number(3.0, true, "kg"), "3 kg");
        assert_eq!(number(-0.0001, true, ""), "0");
    }

    fn picture() -> Image {
        let mut img = Image::new(480, 270);
        for p in img.px.chunks_mut(4) {
            p.copy_from_slice(&[0.02, 0.03, 0.05, 1.0]);
        }
        img
    }

    fn changed(a: &Image, b: &Image) -> usize {
        a.px.chunks(4).zip(b.px.chunks(4)).filter(|(p, q)| p.iter().zip(*q).any(|(x, y)| (x - y).abs() > 0.02)).count()
    }

    #[test]
    fn every_chart_grows_in_and_keeps_the_picture_opaque() {
        let src = picture();
        let n = filmcraft_project::effect::CHART_TYPES.len() as u32;
        for kind in 0..n {
            let mut e = find_effect("chart").expect("effect").instance();
            set(&mut e, "type", ParamValue::Choice(kind));
            set(&mut e, "duration", ParamValue::Float(1.0));
            set(&mut e, "panel", ParamValue::Float(0.0));
            let at = |secs: f64| {
                let mut img = src.clone();
                chart(&mut img, &e, &cx(secs));
                assert!(img.px.iter().all(|v| v.is_finite()), "type {kind}");
                assert!(img.px.chunks(4).all(|p| (p[3] - 1.0).abs() < 1e-4), "type {kind}: opaque stays opaque");
                changed(&img, &src)
            };
            let (none, some, all) = (at(0.0), at(0.4), at(2.0));
            assert_eq!(none, 0, "type {kind}: nothing before it starts");
            assert!(some > 50, "type {kind}: {some} pixels at 40 %");
            assert!(all > some, "type {kind}: {some} then {all}");
        }
        // over a transparent layer the chart is the only thing there
        let mut clear = Image::new(480, 270);
        let e = find_effect("chart").expect("effect").instance();
        chart(&mut clear, &e, &cx(10.0));
        let solid = clear.px.chunks(4).filter(|p| p[3] > 0.5).count();
        assert!(solid > 500 && solid < 480 * 270, "{solid}");
    }

    #[test]
    fn bigger_values_make_taller_bars_and_bigger_slices() {
        let src = picture();
        let mut e = find_effect("chart").expect("effect").instance();
        set(&mut e, "values", ParamValue::Text("klein 10; gross 40".into()));
        set(&mut e, "panel", ParamValue::Float(0.0));
        set(&mut e, "show_values", ParamValue::Bool(false));
        set(&mut e, "palette", ParamValue::Choice(1));
        set(&mut e, "color", ParamValue::Color([1.0, 0.0, 0.0, 1.0]));
        let mut img = src.clone();
        chart(&mut img, &e, &cx(10.0));
        // red pixels left and right of the middle: the right bar is about four times the left
        let red = |x0: usize, x1: usize| {
            (0..img.h).flat_map(|y| (x0..x1).map(move |x| (x, y))).filter(|(x, y)| img.get(*x, *y)[0] > 0.5 && img.get(*x, *y)[1] < 0.1).count() as f32
        };
        let (left, right) = (red(0, 240), red(240, 480));
        assert!(left > 50.0 && (right / left - 4.0).abs() < 0.6, "{left} {right}");
        // a pie: 10 of 50 is a fifth of the disc
        set(&mut e, "type", ParamValue::Choice(3));
        set(&mut e, "palette", ParamValue::Choice(0));
        let mut pie_img = src.clone();
        chart(&mut pie_img, &e, &cx(10.0));
        let yellow = pie_img.px.chunks(4).filter(|p| p[0] > 0.8 && p[1] > 0.5 && p[2] < 0.2).count() as f32;
        let violet = pie_img.px.chunks(4).filter(|p| p[2] > 0.5 && p[1] < 0.2).count() as f32;
        assert!(yellow > 100.0 && (violet / yellow - 4.0).abs() < 0.8, "{yellow} {violet}");
    }

    #[test]
    fn hostile_values_are_survived() {
        let def = find_effect("chart").expect("effect");
        let texts = ["", "a 1", "a -5; b -1", "a 0; b 0", "a 1e12; b 1e-12", &"z 7;".repeat(100), "\u{0}\u{7}; 3", "a 5; b 5; c 5"];
        for kind in 0..6u32 {
            for text in texts {
                for bad in [f64::NAN, f64::INFINITY, -1e30, 0.0, 50.0] {
                    let mut e = def.instance();
                    for p in e.params.values_mut() {
                        match &mut p.value {
                            ParamValue::Float(v) => *v = bad,
                            ParamValue::Vec2(v) if !bad.is_finite() || bad < 0.0 => *v = Vec2::new(bad, bad),
                            _ => {}
                        }
                    }
                    set(&mut e, "type", ParamValue::Choice(kind));
                    set(&mut e, "values", ParamValue::Text(text.to_string()));
                    set(&mut e, "title", ParamValue::Text("Titel".into()));
                    for (w, h) in [(1usize, 1usize), (9, 9), (160, 90), (90, 160)] {
                        for secs in [0.0, 0.3, 5.0, f64::NAN] {
                            let mut img = Image::new(w, h);
                            chart(&mut img, &e, &cx(secs));
                            assert!(img.px.iter().all(|v| v.is_finite()), "type {kind} `{text}` {bad} {w}x{h} {secs}");
                        }
                    }
                }
            }
        }
    }

    /// Local preview: `FILMCRAFT_CHART_PREVIEW=<dir>` writes `chart<type>_960x540.rgb` films
    /// (75 frames each) there.
    #[test]
    #[ignore = "writes preview files"]
    fn chart_preview() {
        let Some(dir) = std::env::var_os("FILMCRAFT_CHART_PREVIEW") else { return };
        let (w, h) = (960usize, 540usize);
        let setups = [
            (0u32, "Jan 12; Feb 18; Mär 9; Apr 24; Mai 31", "Streams (Tsd.)", ""),
            (1, "YouTube 54; TikTok 31; Instagram 22; Spotify 12", "Wo die Hörer herkommen", "%"),
            (2, "Mo 3; Di 5; Mi 4; Do 8; Fr 12; Sa 9; So 14", "Uploads pro Tag", ""),
            (4, "Beats 45; Mixing 25; Videos 20; Rest 10", "Zeit pro Woche", ""),
        ];
        for (kind, values, title, unit) in setups {
            let mut e = find_effect("chart").expect("effect").instance();
            set(&mut e, "type", ParamValue::Choice(kind));
            set(&mut e, "values", ParamValue::Text(values.into()));
            set(&mut e, "title", ParamValue::Text(title.into()));
            set(&mut e, "unit", ParamValue::Text(unit.into()));
            set(&mut e, "duration", ParamValue::Float(1.6));
            let mut film = Vec::new();
            for frame in 0..75 {
                let mut img = Image::new(w, h);
                for (i, p) in img.px.chunks_mut(4).enumerate() {
                    let (x, y) = ((i % w) as f32 / w as f32, (i / w) as f32 / h as f32);
                    p.copy_from_slice(&[0.05 + 0.1 * x, 0.04 + 0.05 * y, 0.12 + 0.12 * (1.0 - y), 1.0]);
                }
                chart(&mut img, &e, &cx(f64::from(frame) / 25.0));
                film.extend(img.px.chunks(4).flat_map(|p| super::super::enc([p[0], p[1], p[2]]).map(|v| (v.clamp(0.0, 1.0) * 255.0).round() as u8)));
            }
            std::fs::write(std::path::Path::new(&dir).join(format!("chart{kind}_{w}x{h}.rgb")), film).expect("write");
        }
    }
}
