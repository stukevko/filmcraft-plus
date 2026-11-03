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
