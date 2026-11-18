//! Tests of [`crate::plus_captions`]: one-click animated captions on the demo project with a
//! fixed transcriber.

use std::sync::Arc;

use filmcraft_project::{CaptionAnimation, Transcript, Word};
use filmcraft_speech::FixedTranscriber;
use filmcraft_time::Tick;
use serde_json::json;

use crate::Session;

/// The demo project; its first A1 clip "says" eight words, with a pause after the fourth.
fn session() -> Session {
    let mut s = Session::default();
    s.execute("file.openDemoProject", json!({})).unwrap();
    let sin = s.active_sequence().unwrap().audio_tracks[0].items[0].source_in;
    let sec = |x: f64| sin + Tick::from_seconds_f64(x);
    let mut t = Transcript { language: "de".into(), ..Default::default() };
    let said = [("Das", 0.2), ("ist,", 0.45), ("mein", 0.7), ("Beat.", 0.95), ("Hört", 2.4), ("ihr", 2.65), ("das", 2.9), ("auch?", 3.15)];
    for (text, a) in said {
        t.words.push(Word::new(text, sec(a), sec(a + 0.2)));
    }
    t.normalize();
    s.transcriber = Some(Arc::new(FixedTranscriber { transcript: t, id: "fixed".into() }));
    s
}

#[test]
fn one_click_transcribes_and_makes_animated_captions() {
    let mut s = session();
    assert!(s.project.transcripts.is_empty());
    assert!(s.execute("captions.animated.rebuild", json!({})).is_err(), "no track yet");
    let r = s.execute("captions.animated.punch", json!({})).unwrap();
    assert_eq!(r["restyled"], false, "{r}");
    assert!(!s.project.transcripts.is_empty(), "transcribed on the way");
    let q = s.active_sequence().unwrap();
    let tr = &q.caption_tracks[0];
    assert_eq!((tr.name.as_str(), tr.style.animation, tr.style.uppercase), ("Animated Captions", CaptionAnimation::Pop, true));
    assert!(tr.captions.len() >= 2, "the pause starts a new block: {:?}", tr.captions.iter().map(|c| &c.text).collect::<Vec<_>>());
    tr.check().unwrap();
    let all: Vec<&str> = tr.captions.iter().flat_map(|c| c.words.iter().map(|w| w.text.as_str())).collect();
    // (the demo sequence uses the media in several clips, so the words come more than once)
    assert_eq!(all[..4], ["Das", "ist", "mein", "Beat"], "{all:?}");
    assert!(all.contains(&"auch?") && all.iter().all(|w| !w.ends_with([',', '.'])), "commas and full stops go, the question mark stays: {all:?}");
    for c in &tr.captions {
        // text and words agree, and the word times are inside the caption, in order
        assert_eq!(c.text.split_whitespace().count(), c.words.len(), "{:?}", c.text);
        assert!(c.words.windows(2).all(|w| w[0].start <= w[1].start));
        assert!(c.words.iter().all(|w| w.start >= Tick::ZERO && w.end <= c.duration && w.start <= w.end), "{c:?}");
    }
    // the first word starts when it is spoken: 0.2 s into the clip
    let clip_start = q.audio_tracks[0].items[0].start;
    let first = &tr.captions[0];
    let spoken = (first.start + first.words[0].start - clip_start).seconds();
    assert!((spoken - 0.2).abs() < 0.05, "{spoken}");
    // it draws
    let (w, h) = (q.settings.width as usize, q.settings.height as usize);
    let at = first.start + first.words[0].start + Tick::from_seconds_f64(0.1);
    assert_eq!(filmcraft_captions::burn::sequence_overlays(q, at, w, h).len(), 1);
}

#[test]
fn another_look_restyles_and_keeps_corrections() {
    let mut s = session();
    s.execute("captions.animated.tiles", json!({})).unwrap();
    let (tid, cid) = {
        let t = &s.active_sequence().unwrap().caption_tracks[0];
        (t.id, t.captions[0].id)
    };
    // the user fixes a word and moves the captions up
    s.edit_sequence("fix", move |q, _, _| {
        let t = q.caption_track_mut(tid).unwrap();
        t.captions.iter_mut().find(|c| c.id == cid).unwrap().text = "Das ist MEIN Beat".into();
        t.style.margin = 0.4;
        Ok(())
    })
    .unwrap();
    let r = s.execute("captions.animated.look", json!({"look": "karaoke"})).unwrap();
    assert_eq!(r["restyled"], true);
    let t = &s.active_sequence().unwrap().caption_tracks[0];
    assert_eq!((t.style.animation, t.style.margin), (CaptionAnimation::Karaoke, 0.4));
    assert_eq!(t.captions[0].text, "Das ist MEIN Beat");
    assert_eq!(s.active_sequence().unwrap().caption_tracks.len(), 1, "no second track");
    // one undo step back to Tiles
    s.execute("edit.undo", json!({})).unwrap();
    assert_eq!(s.active_sequence().unwrap().caption_tracks[0].style.animation, CaptionAnimation::Tiles);
    // rebuild takes the transcript again
    s.execute("captions.animated.rebuild", json!({})).unwrap();
    assert_ne!(s.active_sequence().unwrap().caption_tracks[0].captions[0].text, "Das ist MEIN Beat");
    assert!(s.execute("captions.animated.look", json!({"look": "nope"})).is_err());
    assert!(s.execute("captions.animated.look", json!({})).is_err());
    // the style command knows the new settings
    s.execute("captions.setStyle", json!({"track": "C1", "animation": "one word", "highlightColor": "#ff0000", "uppercase": false, "shadow": 5.0})).unwrap();
    let st = &s.active_sequence().unwrap().caption_tracks[0].style;
    assert_eq!((st.animation, st.highlight_color, st.uppercase, st.shadow), (CaptionAnimation::OneWord, [255, 0, 0, 255], false, 1.0));
    assert!(s.execute("captions.setStyle", json!({"track": "C1", "animation": "wobble"})).is_err());
}

#[test]
fn nothing_to_caption_is_refused() {
    let mut s = Session::default();
    assert!(s.execute("captions.animated.punch", json!({})).is_err(), "no sequence");
    s.execute("file.newSequence", json!({"name": "empty"})).unwrap();
    assert!(s.execute("captions.animated.punch", json!({})).is_err(), "no audio");
}
