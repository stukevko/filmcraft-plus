//! Tests of [`crate::plus`]: beat markers and Sync Plus Effects to Music on generated 120 BPM music.

use std::sync::Arc;

use filmcraft_audio_dsp::remix as dsp;
use filmcraft_project::{ClipId, ParamValue};
use serde_json::json;

use super::*;

const SR: u32 = 48_000;

/// A session with 120 BPM music on A1 at `at` seconds and a colour matte on V1 at 0 s; returns
/// (session, music clip, video clip) with the music clip selected.
fn session(at: f64) -> (Session, ClipId, ClipId) {
    let (m, _) = dsp::test_music(SR, 120.0, 8, 5);
    let mut s = Session::default();
    s.execute("file.newSequence", json!({"name": "plus", "audio": 2, "video": 1})).unwrap();
    let inter: Vec<f32> = (0..m[0].len()).flat_map(|i| [m[0][i], m[1][i]]).collect();
    let bytes: Arc<[u8]> = crate::previews::write_wav_f32(&inter, SR).into();
    let item = crate::commands::import_bytes(&mut s, "/music.wav", bytes, None).unwrap();
    let r = s.execute("timeline.place", json!({"item": item.0, "audioTrack": "A1", "seconds": at})).unwrap();
    let music = ClipId(r["clips"][0].as_u64().unwrap());
    let matte = s.execute("file.newColorMatte", json!({"width": 64, "height": 36, "color": "#ff0000"})).unwrap()["item"].as_u64().unwrap();
    let r = s.execute("timeline.place", json!({"item": matte, "videoTrack": "V1", "seconds": 0.0})).unwrap();
    let video = ClipId(r["clips"][0].as_u64().unwrap());
    s.execute("timeline.select", json!({"clips": [music.0]})).unwrap();
    (s, music, video)
}

#[test]
fn beat_markers_land_on_the_beats() {
    let (mut s, _, _) = session(1.0);
    let r = s.execute("markers.addBeatMarkers", json!({})).unwrap();
    let bpm = r["bpm"].as_f64().unwrap();
    assert!((bpm - 120.0).abs() < 2.0, "tempo {bpm}");
    let seq = s.active_sequence().unwrap();
    let ts: Vec<f64> = seq.markers.iter().map(|m| m.start.seconds()).collect();
    assert!(ts.len() >= 28, "8 bars of 4 beats, got {}", ts.len());
    // half a second apart, after the clip start
    assert!(ts[0] >= 1.0 - 1e-6);
    for w in ts.windows(2) {
        assert!((w[1] - w[0] - 0.5).abs() < 0.03, "beats {} → {}", w[0], w[1]);
    }
    // bars are red, the other beats yellow
    assert_eq!(seq.markers[0].color, filmcraft_project::Label::Rose);
    assert_eq!(seq.markers[1].color, filmcraft_project::Label::Yellow);
    // again with every 4th beat replaces them (one undo step each)
    s.execute("markers.addBeatMarkers", json!({"every": 4})).unwrap();
    let n4 = s.active_sequence().unwrap().markers.len();
    assert!((n4 as f64 - ts.len() as f64 / 4.0).abs() <= 1.0, "{n4}");
    s.execute("edit.undo", json!({})).unwrap();
    assert_eq!(s.active_sequence().unwrap().markers.len(), ts.len());
    // hostile parameter
    assert!(s.execute("markers.addBeatMarkers", json!({"every": 3})).is_err());
}

#[test]
fn sync_sets_tempo_and_phase_on_plus_effects() {
    let (mut s, _, video) = session(0.25);
    // nothing to sync yet
    assert!(s.execute("effects.syncToMusic", json!({})).is_err());
    s.execute("effects.apply", json!({"clips": [video.0], "effect": "beat_pulse"})).unwrap();
    s.execute("effects.apply", json!({"clips": [video.0], "effect": "organic_shake"})).unwrap();
    s.execute("timeline.select", json!({"clips": []})).unwrap();
    // with nothing selected the command needs the clip named
    assert!(s.execute("effects.syncToMusic", json!({})).is_err());
    let music = s.active_sequence().unwrap().audio_tracks[0].items[0].id;
    let r = s.execute("effects.syncToMusic", json!({"clip": music.0})).unwrap();
    assert_eq!(r["effects"], 2);
    let seq = s.active_sequence().unwrap();
    let it = seq.find_item(video).unwrap().1;
    for e in it.effects.iter().filter(|e| e.effect == "beat_pulse" || e.effect == "organic_shake") {
        let f = |k: &str| e.param(k).and_then(|p| p.value.as_f64()).unwrap();
        assert!((f("bpm") - 120.0).abs() < 2.0, "{}", e.effect);
        // the music starts at 0.25 s, so the first beat in the clip's time is near 0.25 s
        let off = f("beat_offset");
        assert!((off - 0.25).abs() < 0.05, "{} offset {off}", e.effect);
        assert!(matches!(e.param("sync").map(|p| &p.value), Some(ParamValue::Choice(c)) if *c > 0), "{}", e.effect);
    }
}

#[test]
fn commands_are_listed_and_refuse_without_music() {
    let mut s = Session::default();
    s.execute("file.newSequence", json!({"name": "empty"})).unwrap();
    assert!(s.execute("markers.addBeatMarkers", json!({})).is_err());
    assert!(s.execute("effects.syncToMusic", json!({"clip": 999_999})).is_err());
}

/// Animate In / Out on a clip: the picture is (nearly) absent on the clip's first and last frame,
/// untouched in the middle, smaller on the way in, and the animation follows a trim.
#[test]
fn animate_in_out_brings_a_clip_in_and_takes_it_out() {
    let (mut s, _, video) = session(0.0);
    let secs = filmcraft_time::Tick::from_seconds_f64;
    let (start, len) = {
        let (_, it) = s.active_sequence().unwrap().find_item(video).unwrap();
        (it.start, it.duration.seconds())
    };
    let fps = s.active_sequence().unwrap().settings.frame_rate.as_f64();
    let at = |s: &Session, t: f64| s.render_program_at(1.0, start + secs(t)).expect("frame");
    // red in the middle of the frame, and how many pixels show the clip at all
    let centre = |img: &filmcraft_render::Image| img.get(img.w / 2, img.h / 2);
    let covered = |img: &filmcraft_render::Image| img.px.chunks(4).filter(|p| p[0] > 0.2).count();
    let plain = at(&s, len / 2.0);
    let full = covered(&plain);
    assert!(full > 0 && centre(&plain)[0] > 0.5, "the small matte sits in the middle, like a picture laid over a video");
    s.execute("effects.apply", json!({"clips": [video.0], "effect": "Animate In / Out"})).unwrap();
    assert_eq!(at(&s, len / 2.0).px, plain.px, "at rest the clip is untouched");
    let first = at(&s, 0.0);
    assert!(centre(&first)[0] < 0.25 * centre(&plain)[0], "nearly invisible on the first frame: {:?}", centre(&first));
    let coming = at(&s, 0.12);
    assert!(covered(&coming) < full && covered(&coming) > 0, "Pop: smaller on the way in ({} of {full})", covered(&coming));
    let last = at(&s, len - 1.0 / fps);
    assert!(centre(&last)[0] < 0.3 * centre(&plain)[0], "nearly gone on the last frame: {:?}", centre(&last));
    // Slide Up: the picture comes from below, so the top rows are empty on the way in
    s.execute("effects.setParam", json!({"clip": video.0, "effect": "animate_in_out", "param": "style", "value": 1})).unwrap();
    let sliding = at(&s, 0.1);
    let row = |img: &filmcraft_render::Image| {
        let (mut sum, mut n) = (0.0f64, 0.0f64);
        for (i, p) in img.px.chunks(4).enumerate() {
            if p[0] > 0.02 {
                sum += (i / img.w) as f64;
                n += 1.0;
            }
        }
        if n > 0.0 { sum / n } else { f64::NAN }
    };
    assert!(row(&sliding) > row(&plain) + 20.0, "Slide Up comes from below: row {} vs {}", row(&sliding), row(&plain));
    assert_eq!(covered(&at(&s, len / 2.0)), full, "and rests where Motion put it");
    // without effects (the Program Monitor's fx switch) nothing moves
    let off = filmcraft_render::RenderOptions { scale: 1.0, effects: false, ..Default::default() };
    let provider = s.media.provider(s.project.clone(), s.services.clone());
    let seq = s.state.active_sequence.expect("sequence");
    let raw = filmcraft_render::render_sequence(&s.project, seq, start, off, &provider);
    assert!(centre(&raw)[0] > 0.5 * centre(&plain)[0]);
    // undo takes the effect off again
    s.execute("edit.undo", json!({})).unwrap();
    s.execute("edit.undo", json!({})).unwrap();
    assert_eq!(at(&s, 0.0).px, plain.px);
}
