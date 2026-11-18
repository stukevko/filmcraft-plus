//! FilmCraft Plus **Animate In / Out** (`animate_in_out`, definition in
//! `filmcraft_project::effect::plus`): a picture or video laid over the main video animates in at
//! the start of its clip and out at its end, on its own, however long the clip is and wherever
//! Motion has put it. Trim the clip and the animation follows.
//!
//! Unlike the pixel effects, this one moves the whole placed layer (so nothing is cut off at the
//! clip's own edges): [`placement`] gives the extra transform in output pixels and the extra
//! opacity, and the compositor applies them when it places the layer. The effect's pixel pass is
//! empty.
//!
//! Original FilmCraft motion curves.

use filmcraft_geom::{Affine, Vec2};
use filmcraft_project::{ParamValue, TrackItem};
use filmcraft_time::Tick;

pub(crate) const ID: &str = "animate_in_out";

/// How the layer is placed on top of Motion and Opacity.
#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) struct Placement {
    /// Output pixels → output pixels, applied after Motion.
    pub xf: Affine,
    /// Multiplies the clip's opacity, 0..1.
    pub opacity: f32,
}

/// The effect's settings at one moment.
#[derive(Clone, Copy, Debug)]
struct Settings {
    style: u32,
    /// Seconds.
    d_in: f64,
    d_out: f64,
    /// 0..3.
    amount: f64,
    /// 0..1.
    bounce: f64,
}

fn finite_or(v: f64, d: f64) -> f64 {
    if v.is_finite() { v } else { d }
}

fn ease_out(p: f64) -> f64 {
    1.0 - (1.0 - p).powi(3)
}

/// Ease out that swings past 1 and settles (`bounce` 0..1 sets how far).
fn back_out(p: f64, bounce: f64) -> f64 {
    let c = 2.6 * bounce;
    let q = p - 1.0;
    1.0 + (c + 1.0) * q * q * q + c * q * q
}

fn smooth(p: f64) -> f64 {
    let p = p.clamp(0.0, 1.0);
    p * p * (3.0 - 2.0 * p)
}

/// Scale, rotation (degrees), offset (fractions of the frame width and height) and opacity for
/// being `gone` (1 = not there yet / gone, 0 = in place). `arrive`: the way in (with the
/// overshoot); the way out is plainer.
fn pose(s: &Settings, p: f64, arrive: bool) -> (f64, f64, Vec2, f64) {
    let p = p.clamp(0.0, 1.0);
    // how far from "in place": 1 → 0
    let gone = if arrive { 1.0 - ease_out(p) } else { (1.0 - p).powi(2) };
    let swing = if arrive { 1.0 - back_out(p, s.bounce) } else { gone };
    let k = s.amount;
    let fade = smooth(p / 0.6);
    let slide = 0.16 * k * gone;
    match s.style {
        // Pop
        0 => (((1.0 - 0.45 * k.min(2.0) * swing).max(0.0)), 0.0, Vec2::new(0.0, 0.0), fade),
        // Slide Up (from below), Slide Down, from Left, from Right
        1 => (1.0, 0.0, Vec2::new(0.0, slide), fade),
        2 => (1.0, 0.0, Vec2::new(0.0, -slide), fade),
        3 => (1.0, 0.0, Vec2::new(-slide, 0.0), fade),
        4 => (1.0, 0.0, Vec2::new(slide, 0.0), fade),
        // Zoom: settles from larger
        5 => (1.0 + 0.3 * k * gone, 0.0, Vec2::new(0.0, 0.0), smooth(p)),
        // Spin
        6 => (((1.0 - 0.4 * k.min(2.0) * swing).max(0.0)), -16.0 * k * swing, Vec2::new(0.0, 0.0), fade),
        // Fade
        _ => (1.0, 0.0, Vec2::new(0.0, 0.0), smooth(p)),
    }
}

/// The placement of `item` at timeline `t`, or `None` when the clip has no (enabled) Animate
/// In / Out or is at rest. `mt`: effect time (keyframes); `centre`: where Motion puts the middle
/// of the layer, output pixels; `frame`: output size; `fps`: sequence frame rate.
pub(crate) fn placement(item: &TrackItem, t: Tick, mt: Tick, centre: Vec2, frame: (f64, f64), fps: f64) -> Option<Placement> {
    let e = item.effects.iter().find(|e| e.enabled && e.effect == ID)?;
    let flag = |id: &str| !matches!(e.param(id).map(|p| &p.value), Some(ParamValue::Bool(false)));
    let s = Settings {
        style: match e.param("style").map(|p| &p.value) {
            Some(ParamValue::Choice(c)) => *c,
            _ => 0,
        },
        d_in: if flag("animate_in") { finite_or(e.f64_at("in", mt), 0.0).clamp(0.0, 10.0) } else { 0.0 },
        d_out: if flag("animate_out") { finite_or(e.f64_at("out", mt), 0.0).clamp(0.0, 10.0) } else { 0.0 },
        amount: (finite_or(e.f64_at("amount", mt), 100.0) / 100.0).clamp(0.0, 3.0),
        bounce: (finite_or(e.f64_at("bounce", mt), 50.0) / 100.0).clamp(0.0, 1.0),
    };
    let len = finite_or(item.duration.seconds(), 0.0).max(0.0);
    let at = finite_or((t - item.start).seconds(), 0.0);
    // a short clip shares its length between the two
    let both = s.d_in + s.d_out;
    let fit = if both > len && both > 0.0 { len / both } else { 1.0 };
    let (d_in, d_out) = (s.d_in * fit, s.d_out * fit);
    // measured at the middle of the frame, so the first frame already shows something
    let half = 0.5 / if fps.is_finite() && fps >= 1.0 { fps.min(1000.0) } else { 30.0 };
    let p_in = if d_in > 1e-6 { ((at + half) / d_in).clamp(0.0, 1.0) } else { 1.0 };
    let p_out = if d_out > 1e-6 { ((len - at - half) / d_out).clamp(0.0, 1.0) } else { 1.0 };
    if p_in >= 1.0 && p_out >= 1.0 {
        return None;
    }
    let (s1, r1, o1, a1) = pose(&s, p_in, true);
    let (s2, r2, o2, a2) = pose(&s, p_out, false);
    let scale = (s1 * s2).clamp(0.0, 8.0);
    let off = Vec2::new((o1.x + o2.x) * frame.0, (o1.y + o2.y) * frame.1);
    let (cx, cy) = (finite_or(centre.x, frame.0 / 2.0), finite_or(centre.y, frame.1 / 2.0));
    let xf = Affine::translate(cx + finite_or(off.x, 0.0), cy + finite_or(off.y, 0.0))
        .then_apply(&Affine::rotate_deg(r1 + r2))
        .then_apply(&Affine::scale(scale, scale))
        .then_apply(&Affine::translate(-cx, -cy));
    Some(Placement { xf, opacity: (a1 * a2).clamp(0.0, 1.0) as f32 })
}

#[cfg(test)]
mod tests {
    use super::*;
    use filmcraft_project::effect::INOUT_STYLES;
    use filmcraft_project::find_effect;

    fn secs(s: f64) -> Tick {
        Tick::from_seconds_f64(s)
    }

    fn bare(len: f64) -> TrackItem {
        serde_json::from_value(serde_json::json!({
            "id": 1, "item": 2, "name": "pic", "label": "Violet", "start": secs(10.0).0, "duration": secs(len).0, "source_in": 0,
            "speed": 1.0, "enabled": true, "effects": []
        }))
        .expect("track item")
    }

    fn clip(style: u32, len: f64) -> TrackItem {
        let mut it = bare(len);
        let mut e = find_effect(ID).expect("effect").instance();
        e.param_mut("style").expect("style").value = ParamValue::Choice(style);
        it.effects.push(e);
        it
    }

    fn at(it: &TrackItem, s: f64) -> Option<Placement> {
        let t = it.start + secs(s);
        placement(it, t, secs(s), Vec2::new(540.0, 600.0), (1080.0, 1920.0), 30.0)
    }

    #[test]
    fn every_style_comes_in_rests_and_goes_out() {
        for style in 0..INOUT_STYLES.len() as u32 {
            let it = clip(style, 3.0);
            let first = at(&it, 0.0).unwrap_or_else(|| panic!("style {style}: animated at the start"));
            assert!(first.opacity < 0.2, "style {style}: nearly invisible on the first frame, {first:?}");
            assert!(first.opacity > 0.0 || style == 0, "style {style}: the first frame shows something");
            assert!(at(&it, 1.5).is_none(), "style {style}: at rest in the middle");
            let last = at(&it, 3.0 - 1.0 / 30.0).unwrap_or_else(|| panic!("style {style}: animated at the end"));
            assert!(last.opacity < 0.2, "style {style}: {last:?}");
            // opacity rises through the way in and falls through the way out
            let mut prev = 0.0;
            for i in 0..=10 {
                let o = at(&it, 0.35 * f64::from(i) / 10.0).map_or(1.0, |p| p.opacity);
                assert!(o + 1e-6 >= prev, "style {style}: opacity falls on the way in");
                prev = o;
            }
            // everything stays finite and the centre stays put for the styles that don't travel
            for i in 0..=90 {
                if let Some(p) = at(&it, f64::from(i) / 30.0) {
                    let c = p.xf.apply(Vec2::new(540.0, 600.0));
                    assert!(c.x.is_finite() && c.y.is_finite() && p.opacity.is_finite());
                    if matches!(style, 0 | 5 | 6 | 7) {
                        assert!((c.x - 540.0).abs() < 1e-6 && (c.y - 600.0).abs() < 1e-6, "style {style}: centre moved to {c:?}");
                    }
                }
            }
        }
    }

    #[test]
    fn pop_overshoots_and_slides_come_from_their_side() {
        let it = clip(0, 3.0);
        let size = |s: f64| at(&it, s).map_or(1.0, |p| (p.xf.apply(Vec2::new(640.0, 600.0)).x - 540.0) / 100.0);
        assert!(size(0.0) < 0.75);
        assert!((0..35).any(|i| size(f64::from(i) / 100.0) > 1.01), "Pop swings past full size");
        let y = |style: u32| at(&clip(style, 3.0), 0.0).map_or(600.0, |p| p.xf.apply(Vec2::new(540.0, 600.0)).y);
        let x = |style: u32| at(&clip(style, 3.0), 0.0).map_or(540.0, |p| p.xf.apply(Vec2::new(540.0, 600.0)).x);
        assert!(y(1) > 700.0, "Slide Up starts below");
        assert!(y(2) < 500.0, "Slide Down starts above");
        assert!(x(3) < 480.0 && x(4) > 600.0, "from the left / from the right");
    }

    #[test]
    fn the_animation_follows_the_clip_and_its_switches() {
        // trimmed shorter: the way out moves with the end
        let long = clip(0, 6.0);
        let short = clip(0, 2.0);
        assert!(at(&long, 1.95).is_none());
        assert!(at(&short, 1.95).is_some());
        // a clip shorter than both animations shares its length between them
        let tiny = clip(1, 0.2);
        assert!(at(&tiny, 0.0).is_some_and(|p| p.opacity < 0.5) && at(&tiny, 0.2 - 1.0 / 30.0).is_some_and(|p| p.opacity < 0.5));
        let most = (0..6).map(|i| at(&tiny, f64::from(i) / 30.0).map_or(1.0, |p| p.opacity)).fold(0.0f32, f32::max);
        assert!(most > 0.9, "it is fully there in between: {most}");
        // switches and a disabled effect
        let mut it = clip(0, 3.0);
        it.effects[0].param_mut("animate_in").expect("switch").value = ParamValue::Bool(false);
        assert!(at(&it, 0.0).is_none() && at(&it, 2.95).is_some());
        it.effects[0].enabled = false;
        assert!(at(&it, 2.95).is_none());
        assert!(at(&bare(1.0), 0.0).is_none());
    }

    #[test]
    fn hostile_values_are_survived() {
        for bad in [f64::NAN, f64::INFINITY, -f64::INFINITY, 1e300, -1e300] {
            for style in [0u32, 3, 6, 99] {
                let mut it = clip(style, 2.0);
                for p in it.effects[0].params.values_mut() {
                    if let ParamValue::Float(v) = &mut p.value {
                        *v = bad;
                    }
                }
                for (len, fps) in [(2.0, 30.0), (0.0, 0.0), (1e-9, f64::NAN)] {
                    it.duration = secs(len);
                    for s in [-1.0, 0.0, 0.01, 1.0, 5.0] {
                        let t = it.start + secs(s);
                        if let Some(p) = placement(&it, t, secs(s), Vec2::new(f64::NAN, 1e308), (1080.0, 1920.0), fps) {
                            let c = p.xf.apply(Vec2::new(1.0, 1.0));
                            assert!(c.x.is_finite() && c.y.is_finite() && (0.0..=1.0).contains(&p.opacity), "{bad} style {style}: {p:?}");
                        }
                    }
                }
            }
        }
    }
}
