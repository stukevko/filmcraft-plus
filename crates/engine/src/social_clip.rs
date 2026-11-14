//! FilmCraft Plus: one command that turns the open sequence into a short vertical clip for
//! TikTok, Reels and Shorts.
//!
//! | command | menu | what it does |
//! |---|---|---|
//! | `plus.socialClip` | Sequence ▸ Make Social Clip | jump cuts from the transcript, an outro, a 1080×1920 frame with every clip filled, a look, big captions and a hook line, as one undo step |
//!
//! From the menu it runs with its defaults: the outro is the project item named Outro…, Opener…
//! or Endcard…, and the hook is the notes of the sequence marker named "Hook".
//!
//! Every step is an existing command (`transcript.removePauses`, `timeline.place`,
//! `sequence.settings`, `clip.fillFrame`, `effects.apply`, `transcript.createCaptions`,
//! `graphics.newText`), run inside one [`Session::edit_group`]. A step that cannot run (no
//! speech, no speech model, no outro in the project) is reported under `skipped` with the reason
//! and the others still run. A bad parameter fails the whole command before anything changes.

use filmcraft_project::{ClipId, ItemId, effect::LOOK_PRESETS};
use filmcraft_time::Tick;
use serde_json::{Map, Value, json};

use crate::commands::{CommandSpec, bad, bool_p, has_seq, item_p, str_p, u64_p};
use crate::{EngineError, Result, Session};

const CMD: &str = "plus.socialClip";
/// Name of the caption track this command makes (and replaces when it runs again).
const CAPTION_TRACK: &str = "Social";
/// A project item whose name starts with one of these is used as the outro when none is passed.
const OUTRO_NAMES: &[&str] = &["outro", "opener", "endcard"];
/// A sequence marker with this name carries the hook line in its notes.
const HOOK_MARKER: &str = "Hook";
/// Portrait frame of TikTok, Reels and Shorts.
const FRAME: (u32, u32) = (1080, 1920);

/// The checked parameters.
struct Opts {
    vertical: bool,
    jump_cuts: bool,
    transcribe: bool,
    pause: f64,
    fillers: bool,
    captions: bool,
    max_chars: u64,
    hook: Option<String>,
    marker_hook: bool,
    hook_seconds: f64,
    outro: Option<ItemId>,
    auto_outro: bool,
    look: Option<usize>,
    look_strength: f64,
}

/// A finite number in `lo..=hi`, or the default when the parameter is absent.
fn ranged(p: &Value, k: &str, default: f64, lo: f64, hi: f64) -> Result<f64> {
    match p.get(k) {
        None | Some(Value::Null) => Ok(default),
        Some(v) => match v.as_f64() {
            Some(x) if x.is_finite() && x >= lo && x <= hi => Ok(x),
            _ => Err(bad(CMD, format!("`{k}` must be a number from {lo} to {hi}"))),
        },
    }
}

/// `look`: a Look Builder preset by name (any case) or index; "Custom" (0) is not a look.
fn look_p(p: &Value) -> Result<Option<usize>> {
    let unknown = || bad(CMD, format!("unknown `look`; one of: {}", LOOK_PRESETS.iter().skip(1).copied().collect::<Vec<_>>().join(", ")));
    match p.get("look") {
        None | Some(Value::Null) => Ok(None),
        Some(Value::String(name)) if name.is_empty() => Ok(None),
        Some(Value::String(name)) => match LOOK_PRESETS.iter().position(|l| l.eq_ignore_ascii_case(name)) {
            Some(i) if i > 0 => Ok(Some(i)),
            _ => Err(unknown()),
        },
        Some(v) => match v.as_u64().and_then(|i| usize::try_from(i).ok()) {
            Some(i) if i > 0 && i < LOOK_PRESETS.len() => Ok(Some(i)),
            _ => Err(unknown()),
        },
    }
}

fn opts(s: &Session, p: &Value) -> Result<Opts> {
    let outro = match p.get("outro") {
        None | Some(Value::Null) => None,
        Some(_) => {
            let id = item_p(p, "outro").ok_or_else(|| bad(CMD, "`outro` must be a project item id"))?;
            if !s.project.items.contains_key(&id) {
                return Err(bad(CMD, format!("no project item {} for `outro`", id.0)));
            }
            Some(id)
        }
    };
    let hook = str_p(p, "hook").map(str::trim).filter(|h| !h.is_empty());
    if hook.is_some_and(|h| h.chars().count() > 200) {
        return Err(bad(CMD, "`hook` is longer than 200 characters"));
    }
    let max_chars = match p.get("maxChars") {
        None | Some(Value::Null) => 18,
        Some(_) => match u64_p(p, "maxChars") {
            Some(n) if (6..=60).contains(&n) => n,
            _ => return Err(bad(CMD, "`maxChars` must be from 6 to 60")),
        },
    };
    Ok(Opts {
        vertical: bool_p(p, "vertical").unwrap_or(true),
        jump_cuts: bool_p(p, "jumpCuts").unwrap_or(true),
        transcribe: bool_p(p, "transcribe").unwrap_or(true),
        pause: ranged(p, "pauseSeconds", 0.4, 0.1, 10.0)?,
        fillers: bool_p(p, "fillers").unwrap_or(true),
        captions: bool_p(p, "captions").unwrap_or(true),
        max_chars,
        hook: hook.map(str::to_string),
        // an explicit `hook` (even an empty one) wins over the marker
        marker_hook: p.get("hook").is_none_or(Value::is_null),
        hook_seconds: ranged(p, "hookSeconds", 2.0, 0.2, 30.0)?,
        outro,
        auto_outro: bool_p(p, "autoOutro").unwrap_or(true),
        look: look_p(p)?,
        look_strength: ranged(p, "lookStrength", 100.0, 0.0, 100.0)?,
    })
}

/// The hook written on the timeline: the notes of the first marker named "Hook" (any case), so
/// the menu command needs no dialog. Cut to 200 characters (project data, not a parameter).
fn marker_hook(q: &filmcraft_project::Sequence) -> Option<String> {
    let m = q.markers.iter().find(|m| m.name.trim().eq_ignore_ascii_case(HOOK_MARKER))?;
    let text: String = m.comment.trim().chars().take(200).collect();
    (!text.is_empty()).then_some(text)
}

/// The first project item (by id) whose name starts with one of [`OUTRO_NAMES`], any case.
fn find_outro(s: &Session) -> Option<ItemId> {
    s.project
        .items
        .values()
        .find(|i| {
            let name = i.name.to_lowercase();
            OUTRO_NAMES.iter().any(|n| name.starts_with(n))
        })
        .map(|i| i.id)
}

/// The sequence's footage clips on video tracks: no titles or shapes (their layers are laid out
/// in frame pixels, so filling the frame or grading them would be wrong), and none made from `not`.
fn footage(s: &Session, not: Option<ItemId>) -> Vec<ClipId> {
    let Some(q) = s.active_sequence() else { return Vec::new() };
    q.video_tracks
        .iter()
        .flat_map(|t| t.items.iter())
        .filter(|it| Some(it.item) != not)
        .filter(|it| !it.effects.iter().any(|e| filmcraft_project::graphic::is_layer_id(&e.effect)))
        .map(|it| it.id)
        .collect()
}

fn ids(clips: &[ClipId]) -> Vec<u64> {
    clips.iter().map(|c| c.0).collect()
}

/// Run a step that may be impossible in this project or build: its error becomes the reason
/// under `skipped` and the other steps go on.
fn optional(s: &mut Session, skipped: &mut Map<String, Value>, step: &str, id: &str, params: Value) -> Option<Value> {
    match s.execute(id, params) {
        Ok(v) => Some(v),
        Err(e) => {
            skipped.insert(step.to_string(), json!(e.to_string()));
            None
        }
    }
}

fn has_words(s: &Session) -> bool {
    !crate::transcript::sequence_words(s).is_empty()
}

fn jump_cuts(s: &mut Session, o: &Opts, done: &mut Map<String, Value>, skipped: &mut Map<String, Value>) {
    if !has_words(s) && o.transcribe {
        match crate::transcript::can_transcribe(s) {
            Ok(()) => {
                if let Some(r) = optional(s, skipped, "transcribe", "transcript.generate", json!({})) {
                    done.insert("transcribed".into(), r["items"].as_array().map_or(json!(0), |a| json!(a.len())));
                }
            }
            Err(why) => {
                skipped.insert("transcribe".into(), json!(why));
            }
        }
    }
    if !has_words(s) {
        skipped.insert("jumpCuts".into(), json!("no speech found in the sequence"));
        return;
    }
    let mut cut = 0.0;
    let mut cuts = 0u64;
    let mut steps = vec![("transcript.removePauses", json!({"minSeconds": o.pause, "keepSeconds": 0.1}))];
    if o.fillers {
        steps.push(("transcript.removeFillers", json!({})));
    }
    for (id, params) in steps {
        if let Some(r) = optional(s, skipped, "jumpCuts", id, params) {
            cuts = cuts.saturating_add(r["removed"].as_u64().unwrap_or(0));
            cut += r["seconds"].as_f64().unwrap_or(0.0);
        }
    }
    done.insert("jumpCuts".into(), json!({"cuts": cuts, "seconds": (cut * 100.0).round() / 100.0}));
}

/// Append the outro after the last clip, unless the sequence already ends with it.
fn outro(s: &mut Session, item: ItemId, done: &mut Map<String, Value>, skipped: &mut Map<String, Value>) {
    let Some(q) = s.active_sequence() else { return };
    let end = q.duration();
    let already = q.video_tracks.iter().chain(q.audio_tracks.iter()).flat_map(|t| t.items.iter()).any(|it| it.item == item && it.start + it.duration >= end);
    if already && end > Tick::ZERO {
        skipped.insert("outro".into(), json!("the sequence already ends with the outro"));
        return;
    }
    if let Some(r) = optional(s, skipped, "outro", "timeline.place", json!({"item": item.0, "time": end.0})) {
        done.insert("outro".into(), json!({"item": item.0, "clips": r["clips"]}));
    }
}

fn vertical(s: &mut Session, done: &mut Map<String, Value>, skipped: &mut Map<String, Value>) -> Result<()> {
    let Some(q) = s.active_sequence() else { return Ok(()) };
    let (w, h) = (q.settings.width, q.settings.height);
    if h <= w {
        // a frame size the sequence cannot take is a real error, not a skipped step
        s.execute("sequence.settings", json!({"width": FRAME.0, "height": FRAME.1}))?;
    }
    let clips = footage(s, None);
    let filled = if clips.is_empty() { None } else { optional(s, skipped, "fillFrame", "clip.fillFrame", json!({"clips": ids(&clips)})) };
    let (w, h) = s.active_sequence().map_or(FRAME, |q| (q.settings.width, q.settings.height));
    done.insert("vertical".into(), json!({"width": w, "height": h, "filled": if filled.is_some() { clips.len() } else { 0 }}));
    Ok(())
}

/// Look Builder with the chosen look on every footage clip but the outro; a clip that already
/// has a Look Builder keeps it and only changes look and strength.
fn look(s: &mut Session, look: usize, strength: f64, outro: Option<ItemId>, done: &mut Map<String, Value>, skipped: &mut Map<String, Value>) {
    let Some(name) = LOOK_PRESETS.get(look) else { return };
    let clips = footage(s, outro);
    let without: Vec<ClipId> = match s.active_sequence() {
        Some(q) => clips.iter().copied().filter(|c| q.find_item(*c).is_some_and(|(_, it)| it.effect("look_builder").is_none())).collect(),
        None => Vec::new(),
    };
    if !without.is_empty() && optional(s, skipped, "look", "effects.apply", json!({"clips": ids(&without), "effect": "look_builder"})).is_none() {
        return;
    }
    let mut n = 0usize;
    for c in &clips {
        let set = |s: &mut Session, param: &str, value: Value| {
            s.execute("effects.setParam", json!({"clip": c.0, "effect": "look_builder", "param": param, "value": value}))
        };
        match set(s, "look", json!(look)).and_then(|_| set(s, "strength", json!(strength))) {
            Ok(_) => n += 1,
            Err(e) => {
                skipped.insert("look".into(), json!(e.to_string()));
            }
        }
    }
    done.insert("look".into(), json!({"look": name, "clips": n}));
}

/// Short one-line captions in the lower third, bold with an outline instead of a box. The
/// track made by an earlier run is replaced.
fn captions(s: &mut Session, o: &Opts, done: &mut Map<String, Value>, skipped: &mut Map<String, Value>) {
    if !has_words(s) {
        skipped.insert("captions".into(), json!("no speech found in the sequence"));
        return;
    }
    let old: Vec<u64> = s.active_sequence().map(|q| q.caption_tracks.iter().filter(|t| t.name == CAPTION_TRACK).map(|t| t.id.0).collect()).unwrap_or_default();
    for t in old {
        // a track that cannot be removed only means two caption tracks: say so and carry on
        optional(s, skipped, "captions", "captions.deleteTrack", json!({"track": t}));
    }
    let params = json!({"maxChars": o.max_chars, "lines": 1, "minSeconds": 0.3, "maxSeconds": 2.5, "name": CAPTION_TRACK});
    let Some(r) = optional(s, skipped, "captions", "transcript.createCaptions", params) else { return };
    let h = s.active_sequence().map_or(FRAME.1, |q| q.settings.height);
    // 84 px on a 1920 px high frame, in proportion on any other
    let size = (f64::from(h) * 84.0 / 1920.0).clamp(12.0, 400.0).round();
    let style = json!({"track": r["track"], "fontStyle": "Bold", "size": size, "background": false, "outline": (size / 14.0).round(), "align": "center", "anchor": "bottom", "margin": 0.26});
    optional(s, skipped, "captionStyle", "captions.setStyle", style);
    done.insert("captions".into(), json!({"track": r["track"], "captions": r["captions"]}));
}

/// The hook: a centred bold line in the upper third for the first seconds, on its own clip
/// above everything. A clip with this exact text at the start is kept, not doubled.
fn hook(s: &mut Session, text: &str, seconds: f64, done: &mut Map<String, Value>, skipped: &mut Map<String, Value>) {
    let Some(q) = s.active_sequence() else { return };
    let (w, h) = (f64::from(q.settings.width), f64::from(q.settings.height));
    if q.video_tracks.iter().flat_map(|t| t.items.iter()).any(|it| it.start == Tick::ZERO && it.name == text) {
        skipped.insert("hook".into(), json!("the sequence already starts with this hook"));
        return;
    }
    let size = (h * 0.045).clamp(12.0, 400.0).round();
    let params = json!({"text": text, "newClip": true, "time": 0, "seconds": seconds, "size": size, "fontStyle": "Bold", "position": [w * 0.08, h * 0.14], "box": [w * 0.84, h * 0.22]});
    let Some(r) = optional(s, skipped, "hook", "graphics.newText", params) else { return };
    let props = json!({"align": "center", "stroke": true, "stroke_width": (size / 10.0).round()});
    optional(s, skipped, "hookStyle", "graphics.set", json!({"clip": r["clip"], "layer": r["layer"], "props": props, "time": 0}));
    done.insert("hook".into(), json!({"clip": r["clip"], "seconds": seconds}));
}

fn social_clip(s: &mut Session, p: &Value) -> Result<Value> {
    if !(p.is_null() || p.is_object()) {
        return Err(bad(CMD, "parameters must be an object"));
    }
    let mut o = opts(s, p)?;
    let Some(q) = s.active_sequence() else { return Err(EngineError::NoSequence) };
    if o.hook.is_none() && o.marker_hook {
        o.hook = marker_hook(q);
    }
    let playhead = s.playhead();
    let (done, skipped) = s.edit_group("Make Social Clip", |s| {
        let (mut done, mut skipped) = (Map::new(), Map::new());
        if o.jump_cuts {
            jump_cuts(s, &o, &mut done, &mut skipped);
        }
        let outro_item = o.outro.or_else(|| if o.auto_outro { find_outro(s) } else { None });
        if let Some(item) = outro_item {
            outro(s, item, &mut done, &mut skipped);
        }
        if o.vertical {
            vertical(s, &mut done, &mut skipped)?;
        }
        if let Some(l) = o.look {
            look(s, l, o.look_strength, outro_item, &mut done, &mut skipped);
        }
        if o.captions {
            captions(s, &o, &mut done, &mut skipped);
        }
        if let Some(text) = &o.hook {
            hook(s, text, o.hook_seconds, &mut done, &mut skipped);
        }
        Ok((done, skipped))
    })?;
    // the steps move the playhead (new clips select themselves): put it back inside the result
    let end = s.active_sequence().map_or(Tick::ZERO, |q| q.duration());
    s.set_playhead(playhead.min(end));
    Ok(json!({"done": done, "skipped": skipped, "seconds": end.seconds()}))
}

pub fn commands() -> Vec<CommandSpec> {
    vec![CommandSpec {
        id: CMD,
        label: "Make Social Clip (Vertical, Jump Cuts, Captions)",
        menu: &["Sequence"],
        shortcut: None,
        params: r#"{"vertical":bool=true (1080x1920, every clip fills the frame),"jumpCuts":bool=true (remove pauses and filler words from the transcript),"transcribe":bool=true (transcribe first when there is no transcript),"pauseSeconds":0.1..10=0.4,"fillers":bool=true,"captions":bool=true,"maxChars":6..60=18,"hook":str? (line shown at the start; default: the notes of the sequence marker named Hook, "" for none),"hookSeconds":0.2..30=2,"outro":id? (project item appended at the end; default: the item named Outro…, Opener… or Endcard…),"autoOutro":bool=true,"look":"Teal & Orange"|index? (Look Builder preset),"lookStrength":0..100=100}"#,
        enabled: has_seq,
        run: social_clip,
        journal: true,
    }]
}
