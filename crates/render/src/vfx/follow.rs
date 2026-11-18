//! FilmCraft Plus **Face Follow** (`face_follow`, definition in
//! `filmcraft_project::effect::plus`): the clip fills the frame and pans so the face stays in
//! it, like a camera operator would. It is what turns a landscape clip into a vertical one that
//! keeps the person in the picture (Sequence ▸ Vertical Version puts it on every clip).
//!
//! Like Animate In / Out it moves the whole placed layer: [`placement`] gives the extra transform
//! in output pixels, the compositor applies it, and the effect's pixel pass is empty. The pan
//! never shows an empty edge: it stops where the picture ends.
//!
//! Original FilmCraft algorithm; the face comes from [`super::face`].

use filmcraft_geom::{Affine, Vec2};
use filmcraft_project::TrackItem;

use super::face;
use super::{FxCtx, Image, bv, fv, smoothstep};

pub(crate) const ID: &str = "face_follow";

/// The effect's settings at one moment.
#[derive(Clone, Copy, Debug)]
struct Settings {
    fill: bool,
    /// ≥ 1.
    zoom: f64,
    /// 0..1.
    follow: f64,
    /// Where the face should sit, as fractions of the frame.
    target: Vec2,
}

fn finite_or(v: f64, d: f64) -> f64 {
    if v.is_finite() { v } else { d }
}

/// The transform (output pixels → output pixels) for a layer whose placed bounds are `min..max`
/// in a `frame`: fill and zoom about the layer's middle, then pan towards `face` (its place in
/// output pixels before this transform, and how sure it is, 0..1) as far as the picture reaches.
fn solve(min: Vec2, max: Vec2, frame: (f64, f64), face: Option<(Vec2, f64)>, s: &Settings) -> Option<Affine> {
    let (w, h) = frame;
    let (bw, bh) = (max.x - min.x, max.y - min.y);
    if !(w > 0.0
        && h > 0.0
        && w.is_finite()
        && h.is_finite()
        && bw > 1e-6
        && bh > 1e-6
        && bw.is_finite()
        && bh.is_finite()
        && min.x.is_finite()
        && min.y.is_finite())
    {
        return None;
    }
    let c = Vec2::new((min.x + max.x) / 2.0, (min.y + max.y) / 2.0);
    // cover the frame exactly (as much of the picture as possible), then the user's zoom
    let k = (if s.fill { (w / bw).max(h / bh) } else { 1.0 } * s.zoom).clamp(1e-3, 64.0);
    let to = if s.fill { Vec2::new(w / 2.0, h / 2.0) } else { c };
    // p ↦ to + (p − c)·k
    let place = |p: Vec2| Vec2::new(to.x + (p.x - c.x) * k, to.y + (p.y - c.y) * k);
    let (lo, hi) = (place(min), place(max));
    let mut d = Vec2::new(0.0, 0.0);
    if let Some((f, conf)) = face
        && f.x.is_finite()
        && f.y.is_finite()
    {
        let f = place(f);
        let strength = s.follow * smoothstep(0.0, 0.35, finite_or(conf, 0.0) as f32) as f64;
        let want = Vec2::new((w * s.target.x - f.x) * strength, (h * s.target.y - f.y) * strength);
        // only as far as the picture still covers the frame
        let reach = |want: f64, lo: f64, hi: f64, size: f64| if lo <= 0.0 && hi >= size { want.clamp(size - hi, -lo) } else { 0.0 };
        d = Vec2::new(reach(want.x, lo.x, hi.x, w), reach(want.y, lo.y, hi.y, h));
    }
    let xf = Affine::translate(to.x + d.x, to.y + d.y).then_apply(&Affine::scale(k, k)).then_apply(&Affine::translate(-c.x, -c.y));
    let at_rest = (k - 1.0).abs() < 1e-9 && (to.x + d.x - c.x).abs() < 1e-6 && (to.y + d.y - c.y).abs() < 1e-6;
    (!at_rest).then_some(xf)
}

/// The placement of `item`'s layer (`layer`, mapped to the output by `m`) in a `frame`, or `None`
/// when the clip has no (enabled) Face Follow or nothing changes.
pub(crate) fn placement(item: &TrackItem, cx: &FxCtx, layer: &Image, m: &Affine, frame: (f64, f64)) -> Option<Affine> {
    let e = item.effects.iter().find(|e| e.enabled && e.effect == ID)?;
    if layer.w == 0 || layer.h == 0 {
        return None;
    }
    let pct = |id: &str, d: f64| finite_or(f64::from(fv(e, id, cx)), d) / 100.0;
    let s = Settings {
        fill: bv(e, "fill"),
        zoom: pct("zoom", 100.0).clamp(1.0, 8.0),
        follow: pct("follow", 100.0).clamp(0.0, 1.0),
        target: Vec2::new((0.5 + pct("offset", 0.0)).clamp(0.0, 1.0), pct("height", 40.0).clamp(0.05, 0.95)),
    };
    let (lw, lh) = (layer.w as f64, layer.h as f64);
    let corners = [Vec2::new(0.0, 0.0), Vec2::new(lw, 0.0), Vec2::new(0.0, lh), Vec2::new(lw, lh)].map(|p| m.apply(p));
    let min = Vec2::new(corners.iter().map(|p| p.x).fold(f64::INFINITY, f64::min), corners.iter().map(|p| p.y).fold(f64::INFINITY, f64::min));
    let max = Vec2::new(corners.iter().map(|p| p.x).fold(f64::NEG_INFINITY, f64::max), corners.iter().map(|p| p.y).fold(f64::NEG_INFINITY, f64::max));
    let face = if s.follow > 1e-4 {
        // a calm camera: the face averaged over up to a second and a half either side
        let half = 0.2 + 1.3 * pct("smooth", 60.0).clamp(0.0, 1.0);
        face::tracked(cx, layer, half).map(|f| (m.apply(Vec2::new(f64::from(f.x) * lw, f64::from(f.y) * lh)), f64::from(f.conf)))
    } else {
        None
    };
    solve(min, max, frame, face, &s)
}

#[cfg(test)]
mod tests {
    use super::*;

    const PORTRAIT: (f64, f64) = (1080.0, 1920.0);

    fn settings() -> Settings {
        Settings { fill: true, zoom: 1.0, follow: 1.0, target: Vec2::new(0.5, 0.4) }
    }

    /// A 1920×1080 clip as Motion places it by default in a 1080×1920 sequence: centred, 1:1.
    fn landscape() -> (Vec2, Vec2) {
        (Vec2::new(-420.0, 420.0), Vec2::new(1500.0, 1500.0))
    }

    fn bounds(xf: &Affine, (min, max): (Vec2, Vec2)) -> (Vec2, Vec2) {
        (xf.apply(min), xf.apply(max))
    }

    #[test]
    fn a_landscape_clip_fills_a_vertical_frame() {
        let b = landscape();
        let xf = solve(b.0, b.1, PORTRAIT, None, &settings()).expect("fills");
        let (lo, hi) = bounds(&xf, b);
        assert!((lo.y - 0.0).abs() < 1e-6 && (hi.y - 1920.0).abs() < 1e-6, "full height: {lo:?} {hi:?}");
        assert!(lo.x < 0.0 && hi.x > 1080.0 && ((lo.x + hi.x) / 2.0 - 540.0).abs() < 1e-6, "centred without a face");
    }

    #[test]
    fn the_pan_brings_the_face_to_the_middle_and_stops_at_the_edge() {
        let b = landscape();
        let s = settings();
        // a face a little right of the middle of the source: it lands in the middle of the frame
        let face = Vec2::new(-420.0 + 1920.0 * 0.6, 420.0 + 1080.0 * 0.4);
        let xf = solve(b.0, b.1, PORTRAIT, Some((face, 1.0)), &s).expect("pans");
        assert!((xf.apply(face).x - 540.0).abs() < 1e-6, "{:?}", xf.apply(face));
        // a face at the far right edge: the picture's edge stops at the frame's edge
        let far = Vec2::new(-420.0 + 1920.0 * 0.98, 420.0 + 540.0);
        let xf = solve(b.0, b.1, PORTRAIT, Some((far, 1.0)), &s).expect("pans");
        let (lo, hi) = bounds(&xf, b);
        assert!((hi.x - 1080.0).abs() < 1e-6 && lo.x < 0.0, "{lo:?} {hi:?}");
        assert!(xf.apply(far).x > 540.0);
        // an unsure face moves the picture less, no face not at all
        let sure = solve(b.0, b.1, PORTRAIT, Some((face, 1.0)), &s).map(|x| x.apply(face).x).unwrap_or(0.0);
        let unsure = solve(b.0, b.1, PORTRAIT, Some((face, 0.15)), &s).map(|x| x.apply(face).x).unwrap_or(0.0);
        let none = solve(b.0, b.1, PORTRAIT, Some((face, 0.0)), &s).map(|x| x.apply(face).x).unwrap_or(0.0);
        assert!(sure < unsure && unsure < none, "{sure} {unsure} {none}");
    }

    #[test]
    fn zoom_gives_room_to_follow_up_and_down_and_offsets_move_the_target() {
        let b = landscape();
        let mut s = settings();
        let face = Vec2::new(540.0, 420.0 + 1080.0 * 0.2);
        // without zoom there is no room up or down
        let y0 = solve(b.0, b.1, PORTRAIT, Some((face, 1.0)), &s).map_or(face.y, |x| x.apply(face).y);
        s.zoom = 1.5;
        let y1 = solve(b.0, b.1, PORTRAIT, Some((face, 1.0)), &s).map_or(face.y, |x| x.apply(face).y);
        assert!((y1 - 1920.0 * 0.4).abs() < (y0 - 1920.0 * 0.4).abs(), "{y0} {y1}");
        s.target.x = 0.7;
        let x = solve(b.0, b.1, PORTRAIT, Some((face, 1.0)), &s).map_or(0.0, |x| x.apply(face).x);
        assert!((x - 1080.0 * 0.7).abs() < 1e-6, "{x}");
    }

    #[test]
    fn without_fill_a_small_picture_stays_where_it_is() {
        // a picture-in-picture that does not cover the frame: nothing to pan
        let (min, max) = (Vec2::new(100.0, 100.0), Vec2::new(500.0, 325.0));
        let s = Settings { fill: false, ..settings() };
        assert!(solve(min, max, PORTRAIT, Some((Vec2::new(480.0, 120.0), 1.0)), &s).is_none());
        // and a full-frame clip with nothing to do is left alone
        assert!(solve(Vec2::new(0.0, 0.0), Vec2::new(1080.0, 1920.0), PORTRAIT, None, &settings()).is_none());
    }

    #[test]
    fn hostile_values_are_survived() {
        let s = settings();
        for bad in [f64::NAN, f64::INFINITY, -f64::INFINITY, 0.0, 1e300] {
            for b in [(Vec2::new(bad, 0.0), Vec2::new(10.0, 10.0)), (Vec2::new(0.0, 0.0), Vec2::new(bad, bad)), (Vec2::new(5.0, 5.0), Vec2::new(5.0, 5.0))] {
                for face in [None, Some((Vec2::new(bad, bad), bad))] {
                    for frame in [PORTRAIT, (0.0, 0.0), (bad, bad)] {
                        if let Some(xf) = solve(b.0, b.1, frame, face, &Settings { zoom: if bad.is_finite() { bad.max(1.0) } else { 1.0 }, ..s }) {
                            let p = xf.apply(Vec2::new(1.0, 1.0));
                            assert!(p.x.is_finite() && p.y.is_finite(), "{bad}: {xf:?}");
                        }
                    }
                }
            }
        }
    }
}
