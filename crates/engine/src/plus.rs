//! FilmCraft Plus music tools: beat markers and beat-synced effects, from the beat tracker that
//! Clip ▸ Remix already runs ([`filmcraft_audio_dsp::remix::analyze`]).
//!
//! | command | menu | what it does |
//! |---|---|---|
//! | `markers.addBeatMarkers` | Markers ▸ Add Beat Markers from Music | a sequence marker on every beat (or every 2nd / 4th beat) of the selected music clip; bars get their own colour |
//! | `effects.syncToMusic` | Markers ▸ Sync Plus Effects to Music | sets Tempo and First Beat of every Organic Shake, Flicker Stack and Beat Pulse in the sequence from the music, and switches Sync to Music on where it was Off |
//!
//! Both read the clip's audio through the same cached analysis as Remix, so running both is one
//! analysis. Beat times follow the clip's speed; reversed and remixed clips are refused.

use filmcraft_audio_dsp::remix as dsp;
use filmcraft_project::{ClipId, Label, Marker, MarkerId, MarkerKind, ParamValue, keyframe::Param};
use filmcraft_time::Tick;
use serde_json::{Value, json};

use crate::commands::{CommandSpec, bad, bool_p, has_seq, u64_p};
use crate::{EngineError, Result, Session};

/// The music clip's beats on the sequence timeline (attack times), and its tempo.
struct Beats {
    times: Vec<Tick>,
    bpm: f64,
}

fn beats_of(s: &Session, c: ClipId, cmd: &str) -> Result<Beats> {
    let seq = s.active_sequence().ok_or(EngineError::NoSequence)?;
    let sr = seq.settings.sample_rate.max(1);
    let (_, it) = seq.find_item(c).ok_or_else(|| bad(cmd, "no such clip"))?;
    if filmcraft_render::remix::is_remixed(it) {
        return Err(bad(cmd, "the clip is remixed: revert the remix first, or use a copy of the music"));
    }
    if it.reverse {
        return Err(bad(cmd, "the clip plays in reverse"));
    }
    let speed = if it.speed.is_finite() && it.speed > 1e-6 { it.speed } else { 1.0 };
    let (start, source_in, dur) = (it.start, it.source_in, it.duration);
    // the source range the clip plays
    let src_dur = Tick::from_seconds_f64(dur.seconds() * speed);
    let a = crate::remix::analysis(s, it.item, source_in, src_dur)?;
    if a.beats.is_empty() || !a.bpm.is_finite() || a.bpm <= 0.0 {
        return Err(bad(cmd, "no steady beat found in this clip"));
    }
    let lead = dsp::beat_lead(sr);
    let times = a
        .beats
        .iter()
        .map(|&b| b.saturating_add(lead))
        .filter(|&b| b >= 0 && b < a.len)
        .map(|b| start + Tick::from_seconds_f64(b as f64 / sr as f64 / speed))
        .filter(|&t| t < start + dur)
        .collect();
    Ok(Beats { times, bpm: a.bpm * speed })
}

fn add_beat_markers(s: &mut Session, p: &Value) -> Result<Value> {
    const CMD: &str = "markers.addBeatMarkers";
    let c = crate::remix::target_clip(s, p, CMD)?;
    let every = match u64_p(p, "every").unwrap_or(1) {
        n @ (1 | 2 | 4) => n as usize,
        n => return Err(bad(CMD, format!("`every` must be 1, 2 or 4 (got {n})"))),
    };
    let replace = bool_p(p, "replace").unwrap_or(true);
    let beats = beats_of(s, c, CMD)?;
    let times: Vec<(usize, Tick)> = beats.times.iter().copied().enumerate().filter(|(i, _)| i % every == 0).collect();
    let n = times.len();
    let bpm = beats.bpm;
    s.edit_sequence("Add Beat Markers", move |q, ctx, _| {
        if replace {
            q.markers.retain(|m| !m.name.starts_with("Beat ") || !m.comment.starts_with("FilmCraft Plus"));
        }
        for (i, t) in times {
            let bar = i / 4 + 1;
            let beat = i % 4 + 1;
            q.markers.push(Marker {
                url: String::new(),
                author: String::new(),
                custom_color: None,
                id: MarkerId(ctx.alloc()),
                start: t,
                duration: Tick::ZERO,
                name: format!("Beat {bar}.{beat}"),
                comment: format!("FilmCraft Plus · {bpm:.1} BPM"),
                kind: MarkerKind::Comment,
                color: if beat == 1 { Label::Rose } else { Label::Yellow },
            });
        }
        q.markers.sort_by_key(|m| m.start);
        Ok(())
    })?;
    Ok(json!({"markers": n, "bpm": (bpm * 10.0).round() / 10.0}))
}

fn set_float(e: &mut filmcraft_project::EffectInstance, id: &str, v: f64) {
    match e.param_mut(id) {
        Some(p) => {
            p.keyframes.clear();
            p.value = ParamValue::Float(v);
        }
        None => {
            e.params.insert(id.to_string(), Param::new(ParamValue::Float(v)));
        }
    }
}

fn sync_to_music(s: &mut Session, p: &Value) -> Result<Value> {
    const CMD: &str = "effects.syncToMusic";
    let c = crate::remix::target_clip(s, p, CMD)?;
    let beats = beats_of(s, c, CMD)?;
    let Some(&first) = beats.times.first() else {
        return Err(bad(CMD, "no beats found in this clip"));
    };
    let bpm = beats.bpm.clamp(20.0, 400.0);
    let period = 60.0 / bpm;
    let mut n = 0usize;
    s.edit_sequence("Sync Plus Effects to Music", |q, _, _| {
        for tr in q.video_tracks.iter_mut() {
            for it in tr.items.iter_mut() {
                // the first beat in the clip's own time, folded into one beat period so it is
                // valid however far into the music the clip sits
                let rel = (first - it.start).seconds();
                let off = rel.rem_euclid(period);
                for e in it.effects.iter_mut().filter(|e| filmcraft_project::effect::MUSIC_SYNCED.contains(&e.effect.as_str())) {
                    set_float(e, "bpm", bpm);
                    set_float(e, "beat_offset", off);
                    let off_now = matches!(e.param("sync").map(|p| &p.value), None | Some(ParamValue::Choice(0)));
                    if off_now {
                        match e.param_mut("sync") {
                            Some(p) => p.value = ParamValue::Choice(1),
                            None => {
                                e.params.insert("sync".into(), Param::new(ParamValue::Choice(1)));
                            }
                        }
                    }
                    n += 1;
                }
            }
        }
        Ok(())
    })?;
    if n == 0 {
        return Err(bad(CMD, "no Organic Shake, Flicker Stack or Beat Pulse in this sequence yet: add one to a video clip first"));
    }
    Ok(json!({"effects": n, "bpm": (bpm * 10.0).round() / 10.0}))
}

fn can_analyse(s: &Session) -> std::result::Result<(), String> {
    has_seq(s)?;
    crate::remix::target_clip(s, &Value::Null, "").map(|_| ()).map_err(|_| "select a music clip on an audio track".to_string())
}

pub fn commands() -> Vec<CommandSpec> {
    vec![
        CommandSpec {
            id: "markers.addBeatMarkers",
            label: "Add Beat Markers from Music",
            menu: &["Markers"],
            shortcut: None,
            params: r#"{"clip":id?,"every":1|2|4?,"replace":bool?}"#,
            enabled: can_analyse,
            run: add_beat_markers,
            journal: true,
        },
        CommandSpec {
            id: "effects.syncToMusic",
            label: "Sync Plus Effects to Music",
            menu: &["Markers"],
            shortcut: None,
            params: r#"{"clip":id?}"#,
            enabled: can_analyse,
            run: sync_to_music,
            journal: true,
        },
    ]
}
