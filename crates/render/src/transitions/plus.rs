//! FilmCraft Plus transitions (definitions in `filmcraft_project::effect::plus`), original work for
//! this fork:
//!
//! - **Block Build** (`plus_block_build`): the incoming picture is built from square blocks that
//!   pop in tower by tower (the AVKO look), each with a short flash.
//! - **Whip Smear** (`plus_whip`): a whip pan: both pictures fly through the frame with a long
//!   directional smear, a slight camera tilt and colour fringes at the fastest moment.
//! - **Punch Through** (`plus_punch`): the outgoing picture is punched through its centre (zoom,
//!   spin, radial colour split), a flash covers the cut and the incoming picture lands.
//! - **Glitch Slice** (`plus_glitch_slice`): horizontal slices tear sideways with colour fringes and
//!   switch to the incoming picture one after another.
//!
//! Like every transition here: `p = 0` is exactly A, `p = 1` exactly B (the caller returns those
//! without calling in), continuous near both ends, deterministic.

use super::{Tx, bell, ease_in, ease_in_out, ease_out, hash, mix, paint, scale4, smoothstep, taps};
use crate::image::Image;

/// 0 → 1 with a small overshoot.
fn back_out(x: f32) -> f32 {
    let x = x.clamp(0.0, 1.0) - 1.0;
    const C: f32 = 1.70158;
    1.0 + (C + 1.0) * x * x * x + C * x * x
}

pub(super) fn apply(id: &str, t: &Tx) -> Option<Image> {
    let p = t.p;
    let (w, h) = (t.wf, t.hf);
    Some(match id {
        "plus_block_build" => {
            let across = t.num("blocks").clamp(2.0, 80.0).round();
            let cell = (w / across).max(1.0);
            let rows = (h / cell).ceil().max(1.0);
            let order = t.choice("order");
            let gap = t.frac("gap").clamp(0.0, 0.9);
            let seed = t.seed();
            // every block is done 0.4 after it starts, and the last one starts at 0.6
            const SPAN: f32 = 0.4;
            paint(t.w, t.h, |x, y| {
                let (i, j) = ((x / cell).floor(), (y / cell).floor());
                let (ii, ji) = (i as i32, j as i32);
                let tower = hash(ii, 0, seed);
                let jitter = hash(ii, ji, seed.wrapping_add(7));
                let along = match order {
                    // towers grow from the street, each at its own pace
                    0 => (rows - 1.0 - j) / rows * (0.6 + 0.4 * tower) * 0.85 + 0.15 * tower,
                    1 => j / rows * (0.6 + 0.4 * tower) * 0.85 + 0.15 * tower,
                    2 => i / across * 0.8 + 0.2 * jitter,
                    3 => (across - 1.0 - i) / across * 0.8 + 0.2 * jitter,
                    _ => 0.2 * tower + 0.8 * jitter,
                };
                let q = ((p - along.clamp(0.0, 1.0) * (1.0 - SPAN)) / SPAN).clamp(0.0, 1.0);
                if q <= 0.0 {
                    return t.pa(x, y);
                }
                // the block grows from its centre; the gap closes as it lands
                let half = (0.5 * back_out(q)).min(0.5) * (1.0 - gap * (1.0 - q).powi(2)) * cell;
                let (u, v) = ((x - (i + 0.5) * cell).abs(), (y - (j + 0.5) * cell).abs());
                let cover = smoothstep(0.0, 1.0, half - u.max(v) + 0.5);
                if cover <= 0.0 {
                    return t.pa(x, y);
                }
                let flash = 1.0 + 0.9 * (1.0 - q).powi(2);
                let b = t.pb(x, y);
                mix(t.pa(x, y), [b[0] * flash, b[1] * flash, b[2] * flash, b[3]], cover)
            })
        }
        "plus_whip" => {
            let (dx, dy) = t.dir();
            let travel = t.travel();
            let e = ease_in_out(p);
            let speed = bell(p);
            let len = t.frac("amount").clamp(0.0, 3.0) * travel * 0.3 * speed;
            let tilt = (t.num("tilt").clamp(-60.0, 60.0) * speed).to_radians();
            let (sn, cs) = tilt.sin_cos();
            let fringe = t.frac("rgb").clamp(0.0, 3.0) * 0.02 * travel * speed;
            let n = taps(len * 2.0).min(28);
            let (cx, cy) = (w / 2.0, h / 2.0);
            // the picture at a point of the moving strip: A, and B following right behind it
            let strip = |x: f32, y: f32| -> [f32; 4] {
                // the content moves along the direction: this pixel shows what was `travel·e` behind it
                let (ax, ay) = (x - dx * travel * e, y - dy * travel * e);
                let s = (ax - cx) * dx + (ay - cy) * dy;
                if s.abs() <= travel / 2.0 { t.sac(ax, ay) } else { t.sbc(ax + dx * travel, ay + dy * travel) }
            };
            paint(t.w, t.h, |x, y| {
                // camera tilt about the centre
                let (rx, ry) = (x - cx, y - cy);
                let (x, y) = (cx + rx * cs - ry * sn, cy + rx * sn + ry * cs);
                let mut acc = [0.0f32; 4];
                for i in 0..n {
                    let k = if n > 1 { i as f32 / (n - 1) as f32 * 2.0 - 1.0 } else { 0.0 };
                    let (sx, sy) = (x + dx * len * k, y + dy * len * k);
                    let mut c = strip(sx, sy);
                    if fringe > 0.25 {
                        c[0] = strip(sx + dx * fringe, sy + dy * fringe)[0];
                        c[2] = strip(sx - dx * fringe, sy - dy * fringe)[2];
                    }
                    for (a, v) in acc.iter_mut().zip(c) {
                        *a += v;
                    }
                }
                scale4(acc, 1.0 / n as f32)
            })
        }
        "plus_punch" => {
            let (cx, cy) = t.center();
            let st = t.frac("strength").clamp(0.0, 3.0);
            // the cut sits under the flash
            let k = smoothstep(0.42, 0.58, p);
            let za = 1.0 + 2.5 * st * ease_in((p / 0.55).min(1.0));
            let zb = 1.0 + 2.5 * st * (1.0 - ease_out(((p - 0.45) / 0.55).clamp(0.0, 1.0)));
            let peak = (-((p - 0.5) / 0.13).powi(2)).exp();
            let near = bell(p);
            let blur = st * near * 0.5;
            let n = taps(blur * w.max(h) * 0.5).min(24);
            let spin = t.num("spin").clamp(-360.0, 360.0).to_radians();
            // A spins away, B spins in from the other side and settles
            let (ra, rb) = (spin * ease_in(p), -spin * (1.0 - ease_out(p)));
            let split = t.frac("rgb").clamp(0.0, 3.0) * 0.06 * near;
            let gain = 1.0 + t.frac("flash").clamp(0.0, 4.0) * 3.0 * peak;
            let one = |img: &Image, x: f32, y: f32, z: f32, rot: f32, zoom_extra: f32| -> [f32; 4] {
                let (sn, cs) = rot.sin_cos();
                let z = z * (1.0 + zoom_extra);
                let (rx, ry) = ((x - cx) / z, (y - cy) / z);
                let (sx, sy) = (cx + rx * cs - ry * sn, cy + rx * sn + ry * cs);
                super::zoom_blur(img, sx, sy, cx, cy, blur, n)
            };
            paint(t.w, t.h, |x, y| {
                let pick = |extra: f32| -> [f32; 4] {
                    let a = if k < 1.0 { one(t.a, x, y, za, ra, extra) } else { [0.0; 4] };
                    let b = if k > 0.0 { one(t.b, x, y, zb, rb, extra) } else { [0.0; 4] };
                    mix(a, b, k)
                };
                let mut c = pick(0.0);
                if split > 0.002 {
                    c[0] = pick(split)[0];
                    c[2] = pick(-split)[2];
                }
                [c[0] * gain, c[1] * gain, c[2] * gain, c[3]]
            })
        }
        "plus_glitch_slice" => {
            let slices = t.num("slices").clamp(2.0, 200.0).round();
            let amount = t.frac("amount").clamp(0.0, 4.0);
            let rgb = t.frac("rgb").clamp(0.0, 4.0);
            let seed = t.seed();
            let env = bell(p);
            // the tear pattern changes a dozen times over the transition
            let step = (p * 12.0).floor() as i32;
            let unit = t.unit().max(0.05);
            paint(t.w, t.h, |x, y| {
                // slices of uneven height: every slice is one or two rows of a finer grid
                let fine = (y / h * slices * 2.0).floor() as i32;
                let j = fine - (hash(fine / 2, 3, seed) > 0.5 && fine % 2 == 1) as i32;
                let r = |a: i32| hash(j, a, seed);
                let torn = hash(j, step + 11, seed) < 0.55;
                let shift = if torn { (hash(j, step, seed) - 0.5) * 2.0 * amount * 0.22 * w * env } else { 0.0 };
                let fringe = rgb * 14.0 * unit * env * (0.4 + r(5));
                // slices switch one after another around the middle
                let from_b = p > 0.34 + 0.32 * r(1);
                let s = |sx: f32| if from_b { t.sbc(sx, y) } else { t.sac(sx, y) };
                let sx = x - shift;
                let mut c = s(sx);
                if fringe > 0.25 {
                    c[0] = s(sx - fringe)[0];
                    c[2] = s(sx + fringe)[2];
                }
                let flicker = 1.0 + 0.5 * env * (hash(j, step + 23, seed) - 0.5) * amount.min(1.5);
                [c[0] * flicker, c[1] * flicker, c[2] * flicker, c[3]]
            })
        }
        _ => return None,
    })
}
