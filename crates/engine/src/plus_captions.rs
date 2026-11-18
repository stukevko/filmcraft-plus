//! FilmCraft Plus: animated captions for Reels / TikTok / Shorts in one click.
//!
//! | command | menu | what it does |
//! |---|---|---|
//! | `captions.animated.punch` … `.tiles` | Sequence ▸ Animated Captions ▸ *look* | transcribes the sequence if it has no transcript yet (downloading the speech model on first use), cuts the words into short blocks and puts them on an "Animated Captions" track in that look; when the track exists it only changes the look, so corrected text is kept |
//! | `captions.animated.rebuild` | Sequence ▸ Animated Captions ▸ Rebuild from Transcript | makes the blocks again from the transcript (after cuts or transcript edits), keeping the look |
//!
//! The words keep their spoken times ([`filmcraft_project::CaptionWord`]), which is what the
//! animation ([`filmcraft_captions::animate`]) runs on. Blocks are sized for the picture: about
//! 14 characters a line in portrait, 22 in landscape, two lines, a new block at every pause.
//! Commas, full stops, colons and semicolons at word ends are dropped, as short-form captions do;
//! question and exclamation marks stay.

use filmcraft_edit::transcript::{self as tx, CaptionRules, SeqWord};
use filmcraft_project::{CAPTION_LOOKS, Caption, CaptionAnimation, CaptionFormat, CaptionStyle, CaptionTrack, CaptionWord, ClipId, TrackId};
use filmcraft_time::Tick;
use serde_json::{Value, json};

use crate::commands::{CommandSpec, bad, has_seq, str_p};
use crate::{EngineError, Result, Session};

/// Name of the caption track the commands make and look for.
pub const TRACK_NAME: &str = "Animated Captions";

/// A word without the punctuation short-form captions leave out.
fn clean(word: &str) -> String {
    let t = word.trim_end_matches([',', '.', ';', ':']).trim_matches(['"', '„', '“', '”']);
    if t.is_empty() { word.to_string() } else { t.to_string() }
}

/// Block rules for a `w`×`h` picture and a look.
fn rules(w: u32, h: u32, anim: CaptionAnimation) -> CaptionRules {
    let portrait = h > w;
    let mut max_chars = if portrait { 14 } else { 22 };
    if anim == CaptionAnimation::Tiles {
        max_chars -= 3;
    }
    CaptionRules {
        max_chars,
        lines: 2,
        min_duration: Tick::from_seconds_f64(0.35),
        max_duration: Tick::from_seconds_f64(3.0),
        gap_frames: 0,
        break_pause: Tick::from_seconds_f64(0.45),
    }
}

/// The captions for `words`, with word times relative to each caption.
fn build(words: &[SeqWord], rules: &CaptionRules, rate: filmcraft_time::FrameRate, mut id: impl FnMut() -> ClipId) -> Vec<Caption> {
    tx::caption_blocks(words, rules, rate)
        .into_iter()
        .map(|b| {
            let text = b.text.lines().map(|l| l.split_whitespace().map(clean).collect::<Vec<_>>().join(" ")).collect::<Vec<_>>().join("\n");
            let said = words.get(b.words.clone()).unwrap_or(&[]);
            let rel = |t: Tick| Tick((t - b.start).0.clamp(0, (b.end - b.start).0.max(0)));
            Caption {
                id: id(),
                start: b.start,
                duration: b.end - b.start,
                text,
                speaker: b.speaker.clone(),
                cue_id: None,
                settings: String::new(),
                words: said.iter().map(|w| CaptionWord { text: clean(&w.text), start: rel(w.start), end: rel(w.end) }).collect(),
            }
        })
        .collect()
}

/// The sequence's words, transcribing first when there is no transcript yet.
fn words_or_transcribe(s: &mut Session, cmd: &str) -> Result<Vec<SeqWord>> {
    let words = crate::transcript::sequence_words(s);
    if !words.is_empty() {
        return Ok(words);
    }
    // first use: fetch the speech model (a test or host transcriber needs none)
    if s.transcriber.is_none() && crate::transcript::speech_available() {
        let model = s.prefs.media_analysis.whisper_model.clone();
        let installed =
            filmcraft_speech::models::find(&model).zip(crate::transcript::models_dir()).is_some_and(|(m, dir)| filmcraft_speech::models::installed(&dir, m));
        if !installed {
            s.execute("transcript.downloadModel", json!({"model": model}))
                .map_err(|e| bad(cmd, format!("the speech model could not be downloaded ({e}); check the internet connection and try again")))?;
        }
    }
    s.execute("sequence.transcribe", json!({}))?;
    let words = crate::transcript::sequence_words(s);
    if words.is_empty() {
        return Err(bad(cmd, "no speech was found in this sequence"));
    }
    Ok(words)
}

fn track_id(s: &Session) -> Option<TrackId> {
    s.active_sequence()?.caption_tracks.iter().find(|t| t.name == TRACK_NAME).map(|t| t.id)
}

fn apply_look(s: &mut Session, look: &str) -> Result<Value> {
    let cmd = "captions.animated";
    let style = CaptionStyle::look(look).ok_or_else(|| bad(cmd, format!("unknown look `{look}` (one of {CAPTION_LOOKS:?})")))?;
    // the track is there: restyle it and keep its (possibly corrected) captions
    if let Some(tid) = track_id(s) {
        let n = s.edit_sequence("Animated Captions Look", move |q, _, _| {
            let t = q.caption_track_mut(tid).ok_or(EngineError::NoSequence)?;
            // keep where the user put them
            let (anchor, margin, font) = (t.style.anchor, t.style.margin, t.style.font.clone());
            t.style = CaptionStyle { anchor, margin, font, ..style };
            t.enabled = true;
            Ok(t.captions.len())
        })?;
        return Ok(json!({"track": tid.0, "captions": n, "look": look, "restyled": true}));
    }
    let words = words_or_transcribe(s, cmd)?;
    let (w, h) = s.active_sequence().map(|q| (q.settings.width, q.settings.height)).ok_or(EngineError::NoSequence)?;
    let (r, rate) = (rules(w, h, style.animation), s.sequence_rate());
    let (tid, n) = s.edit_sequence("Animated Captions", move |q, ctx, st| {
        let tid = TrackId(ctx.alloc());
        let mut t = CaptionTrack::new(tid, TRACK_NAME.into(), CaptionFormat::Subtitle);
        t.style = style;
        t.captions = build(&words, &r, rate, || ClipId(ctx.alloc()));
        let n = t.captions.len();
        q.caption_tracks.insert(0, t);
        st.caption_selection.clear();
        Ok((tid, n))
    })?;
    Ok(json!({"track": tid.0, "captions": n, "look": look, "restyled": false}))
}

fn rebuild(s: &mut Session, _: &Value) -> Result<Value> {
    let cmd = "captions.animated.rebuild";
    let tid = track_id(s).ok_or_else(|| bad(cmd, "there is no Animated Captions track yet: pick a look first"))?;
    let words = crate::transcript::sequence_words(s);
    if words.is_empty() {
        return Err(bad(cmd, "the sequence has no transcript"));
    }
    let q = s.active_sequence().ok_or(EngineError::NoSequence)?;
    let anim = q.caption_tracks.iter().find(|t| t.id == tid).map_or(CaptionAnimation::Pop, |t| t.style.animation);
    let (r, rate) = (rules(q.settings.width, q.settings.height, anim), s.sequence_rate());
    let n = s.edit_sequence("Rebuild Animated Captions", move |q, ctx, st| {
        let caps = build(&words, &r, rate, || ClipId(ctx.alloc()));
        let t = q.caption_track_mut(tid).ok_or(EngineError::NoSequence)?;
        t.captions = caps;
        st.caption_selection.clear();
        Ok(t.captions.len())
    })?;
    Ok(json!({"track": tid.0, "captions": n}))
}

fn has_track(s: &Session) -> std::result::Result<(), String> {
    has_seq(s)?;
    track_id(s).map(|_| ()).ok_or_else(|| "pick an Animated Captions look first".to_string())
}

/// A look can be applied: there is a track to restyle, a transcript, or something to transcribe.
fn can_caption(s: &Session) -> std::result::Result<(), String> {
    has_seq(s)?;
    if track_id(s).is_some() || !crate::transcript::sequence_words(s).is_empty() {
        return Ok(());
    }
    if s.transcriber.is_none() && !crate::transcript::speech_available() {
        return Err("this build has no speech recognition".into());
    }
    if s.active_sequence().is_some_and(|q| q.audio_tracks.iter().any(|t| t.items.iter().any(|i| i.enabled))) {
        Ok(())
    } else {
        Err("the sequence has no audio to transcribe".into())
    }
}

const MENU: &[&str] = &["Sequence", "Animated Captions"];

fn look_spec(id: &'static str, label: &'static str, run: fn(&mut Session, &Value) -> Result<Value>) -> CommandSpec {
    CommandSpec { id, label, menu: MENU, shortcut: None, params: "{}", enabled: can_caption, run, journal: true }
}

pub fn commands() -> Vec<CommandSpec> {
    vec![
        look_spec("captions.animated.punch", "Punch (Yellow Pop)", |s, _| apply_look(s, "Punch")),
        look_spec("captions.animated.spotlight", "Spotlight (Gliding Box)", |s, _| apply_look(s, "Spotlight")),
        look_spec("captions.animated.oneWord", "One Word", |s, _| apply_look(s, "One Word")),
        look_spec("captions.animated.karaoke", "Karaoke (Colour Fill)", |s, _| apply_look(s, "Karaoke")),
        look_spec("captions.animated.rise", "Rise (Minimal)", |s, _| apply_look(s, "Rise")),
        look_spec("captions.animated.tiles", "Tiles", |s, _| apply_look(s, "Tiles")),
        CommandSpec {
            id: "captions.animated.rebuild",
            label: "Rebuild from Transcript",
            menu: MENU,
            shortcut: None,
            params: "{}",
            enabled: has_track,
            run: rebuild,
            journal: true,
        },
        CommandSpec {
            id: "captions.animated.look",
            label: "Animated Captions Look",
            menu: &[],
            shortcut: None,
            params: r#"{"look":"Punch|Spotlight|One Word|Karaoke|Rise|Tiles"}"#,
            enabled: can_caption,
            run: |s, p| {
                let look = str_p(p, "look").ok_or_else(|| bad("captions.animated.look", "pass `look`"))?.to_string();
                apply_look(s, &look)
            },
            journal: true,
        },
    ]
}
