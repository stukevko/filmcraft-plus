//! Local timing of the FilmCraft Plus effects (not a correctness test):
//! `cargo test --release -p filmcraft-render --lib vfx::bench_tests -- --ignored --nocapture`.

use filmcraft_project::{ParamValue, find_effect};
use filmcraft_time::Tick;

use super::{FxCtx, Image};

fn picture(w: usize, h: usize) -> Image {
    let mut img = Image::new(w, h);
    for (i, p) in img.px.chunks_mut(4).enumerate() {
        let (x, y) = ((i % w) as f32 / w as f32, (i / w) as f32 / h as f32);
        let v = 0.5 + 0.5 * ((x * 40.0).sin() * (y * 30.0).cos());
        p.copy_from_slice(&[0.2 + 0.5 * v * x, 0.2 + 0.4 * v, 0.3 + 0.4 * y, 1.0]);
    }
    img
}

#[test]
#[ignore = "timing only"]
fn plus_effects_per_frame() {
    let sizes = [(1920usize, 1080usize), (3840, 2160)];
    let ids = ["organic_shake", "flicker_stack", "look_builder", "beat_pulse", "avko_spectrum", "auto_focus", "studio_light", "draw_on", "marker", "chart"];
    for (w, h) in sizes {
        let src = picture(w, h);
        for id in ids {
            let mut e = find_effect(id).expect("effect").instance();
            // the pixel work alone; finding the face is timed on its own
            if matches!(id, "auto_focus" | "studio_light")
                && let Some(p) = e.param_mut("track")
            {
                p.value = ParamValue::Bool(false);
            }
            if id == "look_builder"
                && let Some(p) = e.param_mut("preset")
            {
                p.value = ParamValue::Choice(2);
            }
            let mut best = f64::MAX;
            for k in 0..3 {
                let mut img = src.clone();
                let cx = FxCtx {
                    t: Tick::ZERO,
                    px_scale: 1.0,
                    seconds: 0.4 + 0.04 * f64::from(k),
                    timecode: "",
                    clip_name: "",
                    project: None,
                    env: None,
                    frame_rate: 25.0,
                    working: Default::default(),
                };
                let started = std::time::Instant::now();
                let _ = super::apply(&mut img, &e, &cx);
                best = best.min(started.elapsed().as_secs_f64() * 1000.0);
            }
            eprintln!("BENCH {w}x{h} {id:14} {best:8.1} ms");
        }
    }
}

#[test]
#[ignore = "timing only"]
fn building_blocks() {
    use rayon::prelude::*;
    let src = picture(1920, 1080);
    let time = |name: &str, f: &mut dyn FnMut()| {
        let mut best = f64::MAX;
        for _ in 0..3 {
            let s = std::time::Instant::now();
            f();
            best = best.min(s.elapsed().as_secs_f64() * 1000.0);
        }
        eprintln!("BLOCK {name:28} {best:8.1} ms");
    };
    for sigma in [2.0f32, 5.0, 9.0, 12.0, 22.0, 40.0] {
        time(&format!("soft_blur {sigma}"), &mut || {
            let _ = super::plus::soft_blur(&src, sigma);
        });
    }
    time("clone", &mut || {
        let _ = src.clone();
    });
    time("enc+dec pass", &mut || {
        let mut img = src.clone();
        img.px.par_chunks_mut(4).for_each(|p| {
            let d = super::enc([p[0], p[1], p[2]]);
            let o = super::dec(d);
            p[..3].copy_from_slice(&o);
        });
    });
    time("fast enc+dec pass", &mut || {
        let mut img = src.clone();
        img.px.par_chunks_mut(4).for_each(|p| {
            let d = super::fast::enc([p[0], p[1], p[2]]);
            let o = super::fast::dec(d);
            p[..3].copy_from_slice(&o);
        });
    });
    time("clone only", &mut || {
        let c = src.clone();
        std::hint::black_box(&c);
    });
    let small = src.downsample2().downsample2();
    time("enlarge x4", &mut || {
        std::hint::black_box(super::fast::enlarge(&small, 1920, 1080));
    });
    time("gaussian 3 at 1/4", &mut || {
        let mut c = small.clone();
        crate::effects::gaussian(&mut c, 3.0, 3.0, true);
        std::hint::black_box(&c);
    });
    time("gaussian 3 at full", &mut || {
        let mut c = src.clone();
        crate::effects::gaussian(&mut c, 3.0, 3.0, true);
        std::hint::black_box(&c);
    });
    time("downsample2", &mut || {
        let _ = src.downsample2();
    });
    let small = src.downsample2().downsample2();
    time("upsample x4", &mut || {
        let m = filmcraft_geom::Affine::scale(4.0, 4.0);
        let _ = small.transformed(1920, 1080, &m);
    });
}

#[test]
#[ignore = "timing only"]
fn face_finding() {
    for (w, h) in [(1920usize, 1080usize), (480, 270)] {
        let src = picture(w, h);
        let mut best = f64::MAX;
        for _ in 0..3 {
            let s = std::time::Instant::now();
            std::hint::black_box(super::face::face_in(&src));
            best = best.min(s.elapsed().as_secs_f64() * 1000.0);
        }
        eprintln!("FACE {w}x{h} (no face in it: all three passes) {best:8.1} ms");
    }
}
