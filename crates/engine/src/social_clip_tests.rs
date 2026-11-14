//! Tests of [`crate::social_clip`]: `plus.socialClip` on the demo project with a fixed transcript.

use std::sync::Arc;

use filmcraft_project::{ParamValue, Transcript, Word};
use filmcraft_speech::FixedTranscriber;
use filmcraft_time::Tick;
use serde_json::{Value, json};

use super::*;

/// The demo project with a transcriber that hears "Hello um world." then, after a 1.2 s pause,
/// "Second speaker here." in the first A1 clip.
fn session() -> Session {
    let mut s = Session::default();
    s.execute("file.openDemoProject", json!({})).unwrap();
    let q = s.active_sequence().unwrap();
    let sin = q.audio_tracks[0].items[0].source_in;
    let sec = |x: f64| sin + Tick::from_seconds_f64(x);
    let mut t = Transcript { language: "en".into(), ..Default::default() };
    for (text, a, b) in [("Hello", 0.2, 0.5), ("um", 0.6, 0.9), ("world.", 1.0, 1.4), ("Second", 2.6, 3.0), ("speaker", 3.0, 3.5), ("here.", 3.5, 4.0)] {
        t.words.push(Word::new(text, sec(a), sec(b)));
    }
    t.normalize();
    s.transcriber = Some(Arc::new(FixedTranscriber { transcript: t, id: "fixed".into() }));
    s
}

fn social_tracks(s: &Session) -> usize {
    s.active_sequence().unwrap().caption_tracks.iter().filter(|t| t.name == "Social").count()
}

fn clips_named(s: &Session, name: &str) -> usize {
    s.active_sequence().unwrap().video_tracks.iter().flat_map(|t| t.items.iter()).filter(|i| i.name == name).count()
}

#[test]
fn one_command_makes_the_clip_and_one_undo_takes_it_back() {
    let mut s = session();
    let before = s.project.clone();
    let len = s.active_sequence().unwrap().duration();
    let undo = s.history.undo.len();

    let r = s.execute("plus.socialClip", json!({"hook": "Your vocal sounds muddy?", "look": "warm film", "lookStrength": 60})).unwrap();
    assert_eq!(r["skipped"], json!({}), "{r}");

    let q = s.active_sequence().unwrap();
    assert_eq!((q.settings.width, q.settings.height), (1080, 1920));
    // the 1.2 s pause and the "um" are gone
    assert!(r["done"]["jumpCuts"]["cuts"].as_u64().unwrap() >= 2, "{r}");
    assert!(q.duration() < len, "{:?} < {len:?}", q.duration());
    assert!((r["seconds"].as_f64().unwrap() - q.duration().seconds()).abs() < 1e-9);
    // short captions on their own track, without the filler word
    let t = q.caption_tracks.iter().find(|t| t.name == "Social").unwrap();
    assert!(!t.captions.is_empty());
    assert!(t.captions.iter().all(|c| c.text.chars().count() <= 18 && !c.text.contains("um")), "{:?}", t.captions.iter().map(|c| &c.text).collect::<Vec<_>>());
    assert!(!t.style.background && t.style.outline > 0.0 && t.style.font_style == "Bold");
    // the hook sits at the start
    let hook = q.video_tracks.iter().flat_map(|t| t.items.iter()).find(|i| i.name == "Your vocal sounds muddy?").unwrap();
    assert_eq!(hook.start, Tick::ZERO);
    assert!((hook.duration.seconds() - 2.0).abs() < 0.05);
    // every footage clip has the look at 60 %, the hook has none
    assert!(hook.effect("look_builder").is_none());
    let looks: Vec<_> = q.video_tracks.iter().flat_map(|t| t.items.iter()).filter_map(|i| i.effect("look_builder")).collect();
    assert_eq!(looks.len(), r["done"]["look"]["clips"].as_u64().unwrap() as usize);
    assert!(!looks.is_empty());
    for e in looks {
        assert_eq!(e.param("look").map(|p| &p.value), Some(&ParamValue::Choice(2)));
        assert_eq!(e.param("strength").map(|p| &p.value), Some(&ParamValue::Float(60.0)));
    }

    assert_eq!(s.history.undo.len(), undo + 1, "one undo step");
    s.execute("edit.undo", json!({})).unwrap();
    assert!(Arc::ptr_eq(&before, &s.project) || *before == *s.project);
}

#[test]
fn running_it_again_does_not_double_captions_or_the_hook() {
    let mut s = session();
    let p = json!({"hook": "One knob.", "look": 1});
    s.execute("plus.socialClip", p.clone()).unwrap();
    let looks = |s: &Session| {
        s.active_sequence()
            .unwrap()
            .video_tracks
            .iter()
            .flat_map(|t| t.items.iter())
            .map(|i| i.effects.iter().filter(|e| e.effect == "look_builder").count())
            .sum::<usize>()
    };
    let n = looks(&s);
    let r = s.execute("plus.socialClip", p).unwrap();
    assert_eq!(social_tracks(&s), 1, "{r}");
    assert_eq!(clips_named(&s, "One knob."), 1, "{r}");
    assert_eq!(looks(&s), n, "no second Look Builder");
    assert!(r["skipped"]["hook"].is_string(), "{r}");
}

#[test]
fn the_outro_is_found_by_name_and_appended_once() {
    let mut s = session();
    let matte = s.execute("file.newColorMatte", json!({"width": 64, "height": 36, "color": "#101010"})).unwrap()["item"].as_u64().unwrap();
    s.edit("Rename", |pr, _| {
        pr.items.get_mut(&filmcraft_project::ItemId(matte)).unwrap().name = "Opener Fatboard".into();
        Ok(())
    })
    .unwrap();
    let r = s.execute("plus.socialClip", json!({"captions": false})).unwrap();
    assert_eq!(r["done"]["outro"]["item"], matte, "{r}");
    let q = s.active_sequence().unwrap();
    let last = q.video_tracks.iter().flat_map(|t| t.items.iter()).max_by_key(|i| i.start + i.duration).unwrap();
    assert_eq!(last.item.0, matte);
    assert_eq!(last.start + last.duration, q.duration());
    let count = |s: &Session| s.active_sequence().unwrap().video_tracks.iter().flat_map(|t| t.items.iter()).filter(|i| i.item.0 == matte).count();
    assert_eq!(count(&s), 1);
    // again: still one outro; `autoOutro: false` on a fresh project adds none
    let r = s.execute("plus.socialClip", json!({"captions": false})).unwrap();
    assert!(r["skipped"]["outro"].is_string(), "{r}");
    assert_eq!(count(&s), 1);
    // no look on the outro
    let r = s.execute("plus.socialClip", json!({"captions": false, "look": "Noir"})).unwrap();
    assert!(r["done"]["look"]["clips"].as_u64().unwrap() >= 1, "{r}");
    let q = s.active_sequence().unwrap();
    assert!(q.video_tracks.iter().flat_map(|t| t.items.iter()).filter(|i| i.item.0 == matte).all(|i| i.effect("look_builder").is_none()));
}

#[test]
fn without_speech_the_frame_still_turns_vertical() {
    let mut s = Session::default();
    s.execute("file.newSequence", json!({"name": "silent", "audio": 1, "video": 1})).unwrap();
    let matte = s.execute("file.newColorMatte", json!({"width": 64, "height": 36, "color": "#ff0000"})).unwrap()["item"].as_u64().unwrap();
    s.execute("timeline.place", json!({"item": matte, "track": "V1", "seconds": 0.0})).unwrap();
    let r = s.execute("plus.socialClip", json!({})).unwrap();
    assert_eq!(r["done"]["vertical"]["height"], 1920, "{r}");
    assert_eq!(r["done"]["vertical"]["filled"], 1, "{r}");
    assert!(r["skipped"]["jumpCuts"].is_string() && r["skipped"]["captions"].is_string(), "{r}");
    assert_eq!(social_tracks(&s), 0);
    // already portrait: the size stays
    s.execute("sequence.settings", json!({"width": 720, "height": 1280})).unwrap();
    let r = s.execute("plus.socialClip", json!({"jumpCuts": false, "captions": false})).unwrap();
    assert_eq!(r["done"]["vertical"]["height"], 1280, "{r}");
    assert_eq!(r["skipped"], json!({}), "{r}");
}

#[test]
fn hostile_parameters_fail_before_anything_changes() {
    let mut s = session();
    let before = s.project.clone();
    let undo = s.history.undo.len();
    let long = "x".repeat(201);
    let bad: Vec<Value> = vec![
        json!({"pauseSeconds": 0}),
        json!({"pauseSeconds": -1}),
        json!({"pauseSeconds": 1e300}),
        json!({"pauseSeconds": "fast"}),
        json!({"hookSeconds": 0}),
        json!({"hookSeconds": 31}),
        json!({"maxChars": 0}),
        json!({"maxChars": 100000}),
        json!({"maxChars": -5}),
        json!({"look": "Custom"}),
        json!({"look": 0}),
        json!({"look": 999}),
        json!({"look": "no such look"}),
        json!({"look": -1}),
        json!({"lookStrength": 101}),
        json!({"outro": 999_999_999u64}),
        json!({"outro": "opener"}),
        json!({"hook": long}),
        json!([1, 2, 3]),
        json!("clip"),
    ];
    for p in bad {
        assert!(s.execute("plus.socialClip", p.clone()).is_err(), "{p}");
        assert!(Arc::ptr_eq(&before, &s.project), "{p} changed the project");
        assert_eq!(s.history.undo.len(), undo, "{p}");
    }
    // no sequence open
    let mut empty = Session::default();
    assert!(empty.execute("plus.socialClip", json!({})).is_err());
    // an empty hook is no hook
    let r = s.execute("plus.socialClip", json!({"hook": "   ", "look": ""})).unwrap();
    assert!(r["done"]["hook"].is_null() && r["done"]["look"].is_null(), "{r}");
}

#[test]
fn a_marker_named_hook_carries_the_hook_line() {
    let mut s = session();
    s.execute("markers.add", json!({"seconds": 1.0, "name": "hook", "comment": "  No curves. One knob.  "})).unwrap();
    let r = s.execute("plus.socialClip", json!({"jumpCuts": false, "captions": false})).unwrap();
    assert!(r["done"]["hook"]["clip"].is_u64(), "{r}");
    assert_eq!(clips_named(&s, "No curves. One knob."), 1);
    // an explicit hook wins, an explicit empty one means none
    let r = s.execute("plus.socialClip", json!({"jumpCuts": false, "captions": false, "hook": ""})).unwrap();
    assert!(r["done"]["hook"].is_null() && r["skipped"]["hook"].is_null(), "{r}");
    assert_eq!(clips_named(&s, "No curves. One knob."), 1);
}
