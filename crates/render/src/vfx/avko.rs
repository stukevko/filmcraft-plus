//! AVKO Spectrum (definition in `filmcraft_project::effect::plus`): an audio spectrum drawn in the
//! style of the AVKO logo. Every frequency band is a "Plattenbau" tower of square blocks with
//! windows and balconies cut out of it; the antennas on the roofs are the peaks (they hold and then
//! fall); a street with a dashed lane runs underneath; the white has a grunge speckle.
//!
//! The towers follow the music weighted to the low end ([`super::beat`]): sub and kick fully, snare
//! body and crack medium, melodies only a little. On every kick the towers jump and the whole
//! picture (the clip under the spectrum too, with Composite on Original) shakes; on every snare it
//! twitches (colour split plus one of slide, zoom jump or flash, chosen per hit).
//!
//! Distances are in "HD pixels" (1920 wide) like the other Plus effects; the blocks themselves are
//! sized from the Width and Towers settings and drawn on whole pixels, so they stay crisp.

use filmcraft_geom::Vec2;
use filmcraft_project::EffectInstance;
use rayon::prelude::*;

use super::beat::{self, Hit, SpectrumCfg};
use super::plus::{hd, rgb_shift, seed_of, soft_blur};
use super::{FxCtx, Image, add_light, bv, chv, cv, dec, fv, hash01, place};

fn f(e: &EffectInstance, id: &str, cx: &FxCtx, def: f32) -> f32 {
    let v = fv(e, id, cx);
    if v.is_finite() { v } else { def }
}

/// Fill the whole-pixel rectangle `[x0, x1) × [y0, y1)` with `c` (premultiplied).
fn fill(img: &mut Image, x0: f64, y0: f64, x1: f64, y1: f64, c: [f32; 4]) {
    let (w, h) = (img.w as f64, img.h as f64);
    let cl = |v: f64, m: f64| if v.is_finite() { v.round().clamp(0.0, m) as usize } else { 0 };
    let (xa, xb, ya, yb) = (cl(x0, w), cl(x1, w), cl(y0, h), cl(y1, h));
    for y in ya..yb {
        let row = y * img.w;
        for x in xa..xb {
            if let Some(p) = img.px.get_mut((row + x) * 4..(row + x) * 4 + 4) {
                p.copy_from_slice(&c);
            }
        }
    }
}

/// The windows and balcony of one block at `(x, y)` of size `b`, cut out (`light` = `None`) or
/// lit. Four kinds, like the logo: two square windows, one wide framed window, each with or
/// without a balcony bar.
fn windows(img: &mut Image, x: f64, y: f64, b: f64, kind: u32, light: Option<[f32; 4]>) {
    let hole = light.unwrap_or([0.0; 4]);
    let line = (b * 0.07).max(1.0);
    let r = |img: &mut Image, u0: f64, v0: f64, u1: f64, v1: f64, c: [f32; 4]| fill(img, x + u0 * b, y + v0 * b, x + u1 * b, y + v1 * b, c);
    match kind % 2 {
        // two square windows side by side
        0 => {
            r(img, 0.22, 0.2, 0.47, 0.47, hole);
            r(img, 0.53, 0.2, 0.78, 0.47, hole);
        }
        // one wide framed window (only the frame is cut, the pane stays)
        _ => {
            let (x0, y0, x1, y1) = (x + 0.22 * b, y + 0.22 * b, x + 0.78 * b, y + 0.5 * b);
            fill(img, x0, y0, x1, y0 + line, hole);
            fill(img, x0, y1 - line, x1, y1, hole);
            fill(img, x0, y0, x0 + line, y1, hole);
            fill(img, x1 - line, y0, x1, y1, hole);
        }
    }
    if kind / 2 == 0 {
        // balcony bar (always a cut-out)
        fill(img, x + 0.2 * b, y + 0.62 * b, x + 0.8 * b, y + 0.62 * b + line.max(b * 0.08), [0.0; 4]);
    }
}

/// An antenna on the roof at `roof`, its mast telescoped up to the peak roof `peak` (the same as
/// `roof` when the tower is at its peak): a mast with one T bar or three short bars on top.
fn antenna(img: &mut Image, x: f64, roof: f64, peak: f64, b: f64, kind: u32, c: [f32; 4]) {
    let t = (b * 0.05).max(1.0);
    let top = peak.min(roof) - b * 0.75;
    fill(img, x - t / 2.0, top, x + t / 2.0, roof, c);
    if kind.is_multiple_of(2) {
        fill(img, x - b * 0.2, top, x + b * 0.2, top + t, c);
    } else {
        for k in 0..3 {
            let y = top + k as f64 * b * 0.11;
            let half = b * (0.16 - 0.03 * k as f64);
            fill(img, x - half, y, x + half, y + t, c);
        }
    }
}

/// AVKO Spectrum.
pub fn avko_spectrum(img: &mut Image, e: &EffectInstance, cx: &FxCtx) {
    if img.w == 0 || img.h == 0 {
        return;
    }
    let (w, h) = (img.w as f64, img.h as f64);
    let k = hd(img);
    let seed = seed_of(e, cx);
    let n = f(e, "towers", cx, 20.0).clamp(4.0, 96.0).round() as usize;
    let floors = f(e, "floors", cx, 7.0).clamp(2.0, 40.0).round() as usize;
    let track = match chv(e, "source") as usize {
        0 => None,
        i => Some(i - 1),
    };

    // ---- listen
    let cfg = SpectrumCfg {
        bands: n,
        fmin: f(e, "fmin", cx, 30.0).clamp(15.0, 2000.0),
        fmax: f(e, "fmax", cx, 12_000.0).clamp(500.0, 22_000.0),
        gain: (f(e, "sensitivity", cx, 100.0) / 100.0).clamp(0.1, 10.0),
        melody: (f(e, "melody", cx, 35.0) / 100.0).clamp(0.0, 1.0),
        release: f(e, "release", cx, 0.18).clamp(0.02, 2.0),
        hold: f(e, "hold", cx, 0.25).clamp(0.0, 3.0),
    };
    let hit_sens = (f(e, "hit_sens", cx, 100.0) / 100.0).clamp(0.25, 4.0);
    let audio = cx.env.and_then(|env| env.audio(0.0, beat::WINDOW, track));
    let (mut level, mut peak, kick, snare, centers) = match &audio {
        Some(a) => {
            let (l, p) = beat::spectrum(&a.samples, a.sample_rate, &cfg);
            let (kk, ss) = beat::hits(&a.samples, a.sample_rate, a.end, hit_sens);
            (l, p, kk, ss, beat::band_centers(&cfg, a.sample_rate))
        }
        None => (vec![0.0; n], vec![0.0; n], None, None, beat::band_centers(&cfg, 48_000)),
    };
    let decay = f64::from(f(e, "hit_decay", cx, 0.2).clamp(0.03, 1.5));
    let ke = kick.map_or(0.0, |h: Hit| h.env(decay));
    let se = snare.map_or(0.0, |h: Hit| h.env(decay * 0.8));
    let kick_jump = (f(e, "kick_boost", cx, 60.0) / 100.0).clamp(0.0, 3.0) * ke * 0.5;
    let snare_jump = (f(e, "snare_boost", cx, 45.0) / 100.0).clamp(0.0, 3.0) * se * 0.4;
    for ((l, p), fc) in level.iter_mut().zip(peak.iter_mut()).zip(&centers) {
        if *fc <= 150.0 {
            *l += kick_jump;
        }
        if (150.0..400.0).contains(fc) || (1500.0..7000.0).contains(fc) {
            *l += snare_jump;
        }
        *l = l.clamp(0.0, 1.0);
        *p = p.max(*l);
    }

    // ---- draw the town on a layer of its own
    let c = cv(e, "color", cx);
    let lin = dec([c[0], c[1], c[2]]);
    let a = c[3].clamp(0.0, 1.0);
    let color = [lin[0] * a, lin[1] * a, lin[2] * a, a];
    let lights = (f(e, "lights", cx, 0.0) / 100.0).clamp(0.0, 1.0) * ke;
    let light = (lights > 0.02).then(|| {
        let lc = cv(e, "light_color", cx);
        let l = dec([lc[0], lc[1], lc[2]]);
        [l[0] * lights, l[1] * lights, l[2] * lights, lights]
    });
    let width = (f(e, "width", cx, 76.0) / 100.0).clamp(0.05, 2.0) as f64 * w;
    let pitch = width / n as f64;
    let gap = (f(e, "gap", cx, 10.0) / 100.0).clamp(0.0, 0.6) as f64;
    let b = (pitch * (1.0 - gap)).floor().max(2.0);
    let g = (pitch - b).max(0.0);
    let x0 = (f(e, "center_x", cx, 50.0) / 100.0) as f64 * w - width / 2.0;
    let base = (f(e, "baseline", cx, 80.0) / 100.0).clamp(0.0, 1.0) as f64 * h;
    let mut town = Image::new(img.w, img.h);
    let antennas = bv(e, "antennas");
    for i in 0..n {
        let lv = level.get(i).copied().unwrap_or(0.0);
        let pk = peak.get(i).copied().unwrap_or(0.0);
        let fl = ((f64::from(lv) * floors as f64).round() as usize).clamp(1, floors);
        let pf = ((f64::from(pk) * floors as f64).round() as usize).clamp(fl, floors);
        let bx = x0 + i as f64 * pitch + g / 2.0;
        for j in 0..fl {
            let by = base - (j + 1) as f64 * b - j as f64 * g;
            fill(&mut town, bx, by, bx + b, by + b, color);
            let kind = (hash01(i as i64, j as i64, 1, seed) * 4.0) as u32;
            windows(&mut town, bx, by, b, kind, light);
        }
        if antennas {
            let roof_at = |fl: usize| base - fl as f64 * b - (fl.saturating_sub(1)) as f64 * g;
            let ax = bx + b * (0.25 + 0.5 * f64::from(hash01(i as i64, 0, 2, seed)));
            antenna(&mut town, ax, roof_at(fl), roof_at(pf), b, (hash01(i as i64, 0, 3, seed) * 2.0) as u32, color);
        }
    }
    if bv(e, "street") {
        let t = (b * 0.1).max(1.0);
        let (sx0, sx1) = (x0 - b * 0.2, x0 + width + b * 0.2);
        fill(&mut town, sx0, base + b * 0.3, sx1, base + b * 0.3 + t, color);
        let (dash, period) = (b * 0.9, b * 1.6);
        let speed = f64::from(f(e, "street_speed", cx, 1.0).clamp(-20.0, 20.0));
        let secs = if cx.seconds.is_finite() { cx.seconds } else { 0.0 };
        let shift = (secs * speed * b * 2.0).rem_euclid(period);
        let y = base + b * 0.75;
        let mut x = sx0 - period + shift;
        let mut guard = 0;
        while x < sx1 && guard < 100_000 {
            fill(&mut town, x.max(sx0), y, (x + dash).min(sx1), y + t * 0.8, color);
            x += period;
            guard += 1;
        }
    }
    // grunge: dark specks in the white, fixed to the picture so they do not crawl
    let grunge = (f(e, "grunge", cx, 50.0) / 100.0).clamp(0.0, 1.0) * 0.07;
    if grunge > 0.0 {
        let tw = town.w;
        town.px.par_chunks_mut(tw * 4).enumerate().for_each(|(y, row)| {
            for x in 0..tw {
                if let Some(p) = row.get_mut(x * 4..x * 4 + 4)
                    && p[3] > 0.0
                    && hash01(x as i64, y as i64, 7, seed) < grunge
                {
                    p.fill(0.0);
                }
            }
        });
    }

    // ---- composite
    let on_original = bv(e, "on_original");
    if !on_original {
        img.px.fill(0.0);
    }
    img.px.par_chunks_mut(4).zip(town.px.par_chunks(4)).for_each(|(p, t)| {
        let inv = 1.0 - t[3];
        for c in 0..4 {
            p[c] = t[c] + p[c] * inv;
        }
    });
    let glow = (f(e, "glow", cx, 15.0) / 100.0).clamp(0.0, 1.0) * (1.0 + 0.8 * ke);
    if glow > 0.005 {
        let soft = soft_blur(&town, (10.0 * k) as f32);
        let g = glow * 1.4;
        img.px.par_chunks_mut(4).zip(soft.px.par_chunks(4)).for_each(|(p, s)| add_light(p, [s[0] * g, s[1] * g, s[2] * g]));
    }

    // ---- the whole picture moves with the drums
    let mid = Vec2::new(w / 2.0, h / 2.0);
    let small = w.min(h).max(1.0);
    if let Some(kh) = kick {
        let amt = f64::from((f(e, "kick_shake", cx, 60.0) / 100.0).clamp(0.0, 5.0)) * f64::from(ke);
        if amt > 0.003 {
            let wob = (kh.age * std::f64::consts::TAU * 7.0).cos();
            let ang = f64::from(hash01(kh.index, 3, 9, seed)) * std::f64::consts::TAU;
            let d = 28.0 * k * amt * wob;
            let rot = (f64::from(hash01(kh.index, 4, 9, seed)) - 0.5) * 2.4 * amt * wob;
            // zoom in just enough that the moved picture still covers the frame
            let cover = if on_original { 1.0 + 2.0 * d.abs() / small + rot.abs().to_radians() * (w.max(h) / small) } else { 1.0 };
            place(img, mid, Vec2::new(ang.cos() * d, ang.sin() * d), (1.0 + 0.035 * amt) * cover, rot);
        }
    }
    if let Some(sh) = snare {
        let s = f64::from((f(e, "snare_twitch", cx, 50.0) / 100.0).clamp(0.0, 5.0)) * f64::from(se);
        if s > 0.003 {
            let r = |a: i64| f64::from(hash01(sh.index, a, 11, seed));
            match (r(1) * 3.0) as u32 {
                0 => {
                    let d = 45.0 * k * s * if r(2) < 0.5 { -1.0 } else { 1.0 };
                    let cover = if on_original { 1.0 + 2.0 * d.abs() / w.max(1.0) } else { 1.0 };
                    place(img, mid, Vec2::new(d, 0.0), cover, 0.0);
                }
                1 => place(img, mid, Vec2::new(0.0, 0.0), 1.0 + 0.09 * s, 0.0),
                _ => {
                    let gain = (1.0 + 0.9 * s) as f32;
                    img.map_rgb(|c, _, _| [c[0] * gain, c[1] * gain, c[2] * gain]);
                }
            }
            rgb_shift(img, (16.0 * k * s) as f32 * if r(3) < 0.5 { -1.0 } else { 1.0 });
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::vfx::{AudioWindow, FxEnv};
    use filmcraft_geom::Affine;
    use filmcraft_project::{EffectInstance, ParamValue, find_effect};
    use filmcraft_time::Tick;

    /// An environment whose audio is the test beat of [`beat::tests::beat`].
    struct Music {
        x: Vec<f32>,
        t: f64,
    }

    impl FxEnv for Music {
        fn frame(&self, _: f64, _: &EffectInstance) -> Option<Image> {
            None
        }
        fn source_frame(&self, _: f64, _: f32) -> Option<Image> {
            None
        }
        fn track(&self, _: usize) -> Option<Image> {
            None
        }
        fn layer_to_output(&self) -> Affine {
            Affine::IDENTITY
        }
        fn clip_seconds(&self) -> f64 {
            2.0
        }
        fn clip_offset(&self) -> f64 {
            self.t
        }
        fn frame_rate(&self) -> f64 {
            30.0
        }
        fn media_timecode(&self) -> String {
            String::new()
        }
        fn file_name(&self) -> String {
            String::new()
        }
        fn sequence_name(&self) -> String {
            String::new()
        }
        fn sequence_size(&self) -> (u32, u32) {
            (320, 180)
        }
        fn source_size(&self) -> (u32, u32) {
            (320, 180)
        }
        fn clip_key(&self) -> u64 {
            1
        }
        fn audio(&self, _: f64, _: f64, _: Option<usize>) -> Option<AudioWindow> {
            let (w, end) = beat::tests::window(&self.x, self.t);
            Some(AudioWindow { samples: w.to_vec(), sample_rate: beat::tests::SR, end })
        }
    }

    fn cx(env: Option<&dyn FxEnv>, t: f64) -> FxCtx<'_> {
        FxCtx {
            t: Tick::from_seconds_f64(t),
            px_scale: 1.0,
            seconds: t,
            timecode: "",
            clip_name: "",
            project: None,
            env,
            frame_rate: 30.0,
            working: Default::default(),
        }
    }

    fn set(e: &mut EffectInstance, id: &str, v: ParamValue) {
        if let Some(p) = e.param_mut(id) {
            p.value = v;
        }
    }

    fn background() -> Image {
        let mut img = Image::new(320, 180);
        for (i, p) in img.px.chunks_mut(4).enumerate() {
            p.copy_from_slice(&[0.02 + (i % 320) as f32 / 4000.0, 0.03, 0.05, 1.0]);
        }
        img
    }

    /// Opaque pixels of the left `cols` of the town (spectrum alone, no hits).
    fn coverage(t: f64, x: &[f32]) -> usize {
        let env = Music { x: x.to_vec(), t };
        let mut e = find_effect("avko_spectrum").unwrap().instance();
        set(&mut e, "on_original", ParamValue::Bool(false));
        for id in ["kick_shake", "snare_twitch", "glow", "grunge"] {
            set(&mut e, id, ParamValue::Float(0.0));
        }
        set(&mut e, "antennas", ParamValue::Bool(false));
        set(&mut e, "street", ParamValue::Bool(false));
        let mut img = background();
        avko_spectrum(&mut img, &e, &cx(Some(&env), t));
        // the lowest four of 20 towers: the left fifth of the 76 % wide town
        img.px.chunks(4).enumerate().filter(|(i, p)| (i % 320) < 38 + 49 && p[3] > 0.5).count()
    }

    #[test]
    fn kick_towers_rise_on_the_kick_and_fall_after() {
        let x = beat::tests::beat();
        let hit = coverage(1.04, &x);
        let later = coverage(1.22, &x);
        assert!(hit > later, "kick towers {hit} at the kick vs {later} after");
    }

    #[test]
    fn the_whole_picture_shakes_on_a_kick_and_rests_without_music() {
        let x = beat::tests::beat();
        let e = find_effect("avko_spectrum").unwrap().instance();
        let render = |env: Option<&dyn FxEnv>, t: f64, shake: f64| {
            let mut e = e.clone();
            set(&mut e, "kick_shake", ParamValue::Float(shake));
            set(&mut e, "snare_twitch", ParamValue::Float(0.0));
            let mut img = background();
            avko_spectrum(&mut img, &e, &cx(env, t));
            img
        };
        let env = Music { x: x.clone(), t: 1.02 };
        let (still, shaken) = (render(Some(&env), 1.02, 0.0), render(Some(&env), 1.02, 100.0));
        // the background itself moved, not only the towers: compare the sky (top rows)
        let sky = |img: &Image| img.px[..320 * 40 * 4].to_vec();
        assert_ne!(sky(&still), sky(&shaken), "background did not shake");
        // no audio: a quiet town on the ground floor, no movement, finite pixels
        let quiet = render(None, 0.5, 100.0);
        assert!(quiet.px.iter().all(|v| v.is_finite()));
        assert_eq!(sky(&quiet), sky(&background()));
        assert!(quiet.px != background().px, "the ground floor and street are drawn");
    }

    #[test]
    fn hostile_parameters_do_not_crash() {
        let x = beat::tests::beat();
        let env = Music { x, t: 1.02 };
        let def = find_effect("avko_spectrum").unwrap();
        for bad in [f64::NAN, f64::INFINITY, -1e12, 1e12, 0.0] {
            let mut e = def.instance();
            for p in &def.params {
                if matches!(p.default, ParamValue::Float(_)) {
                    set(&mut e, p.id, ParamValue::Float(bad));
                }
            }
            set(&mut e, "source", ParamValue::Choice(99));
            for img in [background(), Image::new(0, 0), Image::new(1, 1), Image::new(3, 2000)] {
                let mut img = img;
                avko_spectrum(&mut img, &e, &cx(Some(&env), 1.02));
                assert!(img.px.iter().all(|v| v.is_finite()));
            }
        }
    }

    #[test]
    fn shake_and_flicker_follow_kick_and_snare() {
        let x = beat::tests::beat();
        let diff = |a: &Image, b: &Image| a.px.iter().zip(&b.px).map(|(x, y)| (x - y).abs()).sum::<f32>();
        // Organic Shake on Kick (Audio): moves right after a kick, rests before the next one
        let mut e = find_effect("organic_shake").unwrap().instance();
        set(&mut e, "sync", ParamValue::Choice(filmcraft_project::effect::SYNC_KICK));
        set(&mut e, "motion_blur", ParamValue::Float(0.0));
        // (the edge cover zooms by a constant amount; leave it out to see the movement alone)
        set(&mut e, "auto_scale", ParamValue::Bool(false));
        set(&mut e, "intensity", ParamValue::Float(400.0));
        let at = |t: f64| {
            let env = Music { x: x.clone(), t };
            let mut img = background();
            super::super::plus::organic_shake(&mut img, &e, &cx(Some(&env), t));
            diff(&img, &background())
        };
        let (hit, rest) = (at(1.03), at(1.45));
        assert!(hit > rest * 4.0, "shake on the kick {hit} vs before the next {rest}");
        // Flicker Stack on Snare (Audio): fires right after a snare, quiet half a second later
        let mut e = find_effect("flicker_stack").unwrap().instance();
        set(&mut e, "sync", ParamValue::Choice(filmcraft_project::effect::SYNC_KICK + 1));
        let fl = |t: f64| {
            let env = Music { x: x.clone(), t };
            let mut img = background();
            super::super::plus::flicker_stack(&mut img, &e, &cx(Some(&env), t));
            diff(&img, &background())
        };
        assert!(fl(1.27) > 0.0, "no twitch on the snare");
    }
}

/// Writes preview frames (raw sRGB8 RGBA, 960×540) of the effect over a night-sky gradient to the
/// folder in `FILMCRAFT_AVKO_PREVIEW`: `cargo test -p filmcraft-render avko_preview -- --ignored`.
#[cfg(test)]
#[test]
#[ignore = "writes preview images"]
fn avko_preview() {
    use crate::vfx::{AudioWindow, FxEnv};
    use filmcraft_project::{ParamValue, find_effect};
    let Some(dir) = std::env::var_os("FILMCRAFT_AVKO_PREVIEW") else { return };
    struct Env(Vec<f32>, f64);
    impl FxEnv for Env {
        fn frame(&self, _: f64, _: &EffectInstance) -> Option<Image> {
            None
        }
        fn source_frame(&self, _: f64, _: f32) -> Option<Image> {
            None
        }
        fn track(&self, _: usize) -> Option<Image> {
            None
        }
        fn layer_to_output(&self) -> filmcraft_geom::Affine {
            filmcraft_geom::Affine::IDENTITY
        }
        fn clip_seconds(&self) -> f64 {
            2.0
        }
        fn clip_offset(&self) -> f64 {
            self.1
        }
        fn frame_rate(&self) -> f64 {
            30.0
        }
        fn media_timecode(&self) -> String {
            String::new()
        }
        fn file_name(&self) -> String {
            String::new()
        }
        fn sequence_name(&self) -> String {
            String::new()
        }
        fn sequence_size(&self) -> (u32, u32) {
            (960, 540)
        }
        fn source_size(&self) -> (u32, u32) {
            (960, 540)
        }
        fn clip_key(&self) -> u64 {
            1
        }
        fn audio(&self, _: f64, _: f64, _: Option<usize>) -> Option<AudioWindow> {
            let (w, end) = beat::tests::window(&self.0, self.1);
            Some(AudioWindow { samples: w.to_vec(), sample_rate: beat::tests::SR, end })
        }
    }
    let x = beat::tests::beat();
    for (name, t) in [("between", 0.62), ("kick", 1.035), ("snare", 1.265)] {
        let mut img = Image::new(960, 540);
        for (i, p) in img.px.chunks_mut(4).enumerate() {
            let (px, py) = ((i % 960) as f32 / 960.0, (i / 960) as f32 / 540.0);
            let c = super::dec([0.05 + 0.08 * py, 0.05 + 0.04 * px, 0.12 + 0.1 * (1.0 - py)]);
            p.copy_from_slice(&[c[0], c[1], c[2], 1.0]);
        }
        let mut e = find_effect("avko_spectrum").expect("def").instance();
        if let Some(p) = e.param_mut("lights") {
            p.value = ParamValue::Float(if name == "kick" { 60.0 } else { 0.0 });
        }
        let env = Env(x.clone(), t);
        let cx = FxCtx {
            t: filmcraft_time::Tick::from_seconds_f64(t),
            px_scale: 1.0,
            seconds: t,
            timecode: "",
            clip_name: "",
            project: None,
            env: Some(&env),
            frame_rate: 30.0,
            working: Default::default(),
        };
        avko_spectrum(&mut img, &e, &cx);
        let bytes: Vec<u8> = img
            .px
            .chunks(4)
            .flat_map(|p| {
                let a = p[3].max(1e-6);
                let s = super::enc([p[0] / a, p[1] / a, p[2] / a]);
                [s[0], s[1], s[2], p[3]].map(|v| (v.clamp(0.0, 1.0) * 255.0).round() as u8)
            })
            .collect();
        std::fs::write(std::path::Path::new(&dir).join(format!("{name}.rgba")), bytes).expect("write");
    }
}
