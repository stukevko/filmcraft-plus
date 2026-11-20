//! Local end-to-end timing of the FilmCraft Plus effects on a real video file (not a correctness
//! test): `FILMCRAFT_PERF_CLIP=<video> cargo test --release -p filmcraft-engine --lib
//! plus_bench -- --ignored --nocapture`. `FILMCRAFT_PERF_OUT=<dir>` also writes one frame per
//! effect there as raw RGB.

use serde_json::json;

use crate::Session;

#[test]
#[ignore = "timing only; needs a video file"]
fn plus_bench_on_a_real_clip() {
    let Some(path) = std::env::var_os("FILMCRAFT_PERF_CLIP") else { return };
    let out = std::env::var_os("FILMCRAFT_PERF_OUT");
    let effects = ["", "Organic Shake", "Flicker Stack", "Look Builder", "Auto Focus", "Studio Light", "Face Follow", "Animate In / Out", "Marker", "Chart"];
    for scale in [1.0f32, 0.5] {
        for effect in effects {
            let mut s = Session::default();
            s.execute("file.newSequence", json!({"name": "perf", "width": 1920, "height": 1080, "fps": 30, "video": 1, "audio": 1})).unwrap();
            let r = s.execute("file.import", json!({"paths": [path.to_string_lossy()]})).unwrap();
            let item = r["items"][0].as_u64().expect("imported");
            let placed = s.execute("timeline.place", json!({"item": item, "track": "V1", "seconds": 0.0})).unwrap();
            let clip = placed["clips"][0].as_u64().or_else(|| placed["clip"].as_u64()).expect("clip id");
            if effect == "Face Follow" {
                s.execute("sequence.reframe.9x16", json!({})).unwrap();
            } else if !effect.is_empty() {
                s.execute("effects.apply", json!({"clips": [clip], "effect": effect})).unwrap();
            }
            let fps = 30.0;
            let frames = 30;
            let started = std::time::Instant::now();
            let (mut worst, mut last) = (0.0f64, None);
            for f in 0..frames {
                let t = std::time::Instant::now();
                last = s.render_program_at(scale, filmcraft_time::Tick::from_seconds_f64(2.0 + f64::from(f) / fps));
                worst = worst.max(t.elapsed().as_secs_f64() * 1000.0);
            }
            let each = started.elapsed().as_secs_f64() * 1000.0 / f64::from(frames);
            eprintln!("PERF scale {scale:3} {:18} {each:7.1} ms/frame (worst {worst:7.1})", if effect.is_empty() { "(no effect)" } else { effect });
            if let (Some(dir), Some(img)) = (&out, last)
                && scale == 1.0
            {
                let raw: Vec<u8> =
                    img.px.chunks(4).flat_map(|p| [p[0], p[1], p[2]].map(|v| (v.clamp(0.0, 1.0).powf(1.0 / 2.2) * 255.0).round() as u8)).collect();
                let name = format!("{}_{}x{}.rgb", effect.replace([' ', '/'], ""), img.w, img.h);
                std::fs::write(std::path::Path::new(dir).join(name), raw).expect("write");
            }
        }
    }
}
