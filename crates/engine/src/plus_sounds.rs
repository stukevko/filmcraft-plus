//! FilmCraft Plus **Sound Effects**: a small pack of transition sounds (whooshes, an impact, a
//! riser, a glitch, a pop) and commands that lay them under the picture.
//!
//! The sounds are not recordings: every one is synthesised here from noise and sine waves
//! (original work, nothing to license), written as a WAV file next to the project the first time
//! it is used and imported into the bin "FilmCraft Plus Sounds".
//!
//! | command | menu (Sequence ▸ Sound Effects) | what it does |
//! |---|---|---|
//! | `sounds.addToTransitions` | Add Sounds to All Transitions | a fitting sound under every video transition, its loudest moment on the middle of the transition |
//! | `sounds.addToCuts` | Add Swish to Every Cut | a short swish on every plain cut of V1 |
//! | `sounds.add.<id>` | Whoosh, Long Whoosh, Swish, Impact, Riser, Glitch, Pop | that sound at the playhead |
//! | `sounds.importAll` | Import All Sounds into the Project | the whole pack into the bin, to drag by hand |
//! | `sounds.add` | — | `{"sound":id,"seconds":f64?}` |
//!
//! Sounds go on an audio track named "SFX" (added when there is none). Where that track already
//! has a clip nothing is added, so running a command twice does not double the sounds.

use std::f32::consts::PI;

use filmcraft_project::{BinEntry, BinId, ItemId, ItemKind, TrackId, TrackKind};
use filmcraft_time::{Tick, TimeRange};
use serde_json::{Value, json};

use crate::commands::{CommandSpec, bad, f64_p, has_seq, str_p};
use crate::{EngineError, Result, Session};

const SR: u32 = 48_000;
const TRACK: &str = "SFX";
const BIN: &str = "FilmCraft Plus Sounds";
/// Most sounds one command places.
const MAX_SOUNDS: usize = 2000;

/// One sound of the pack.
#[derive(Clone, Copy, Debug)]
pub struct Sound {
    pub id: &'static str,
    /// Menu label (English; the UI translates it).
    pub name: &'static str,
    /// File and project item name.
    file: &'static str,
    /// Seconds from its start to the moment that belongs on the cut.
    peak: f64,
    /// Peak level, 0..1.
    level: f32,
}

pub const SOUNDS: &[Sound] = &[
    Sound { id: "whoosh", name: "Whoosh", file: "Plus Whoosh.wav", peak: 0.33, level: 0.45 },
    Sound { id: "longWhoosh", name: "Long Whoosh", file: "Plus Long Whoosh.wav", peak: 0.6, level: 0.42 },
    Sound { id: "swish", name: "Swish", file: "Plus Swish.wav", peak: 0.13, level: 0.32 },
    Sound { id: "impact", name: "Impact", file: "Plus Impact.wav", peak: 0.02, level: 0.7 },
    Sound { id: "riser", name: "Riser", file: "Plus Riser.wav", peak: 1.58, level: 0.42 },
    Sound { id: "glitch", name: "Glitch", file: "Plus Glitch.wav", peak: 0.12, level: 0.38 },
    Sound { id: "pop", name: "Pop", file: "Plus Pop.wav", peak: 0.012, level: 0.5 },
];

fn sound(id: &str) -> Option<&'static Sound> {
    SOUNDS.iter().find(|s| s.id == id)
}

// ------------------------------------------------------------------ synthesis

/// Repeatable noise in −1..1.
struct Noise(u32);

impl Noise {
    fn next(&mut self) -> f32 {
        let mut x = self.0.max(1);
        x ^= x << 13;
        x ^= x >> 17;
        x ^= x << 5;
        self.0 = x;
        (x as f32 / u32::MAX as f32) * 2.0 - 1.0
    }
    /// 0..1.
    fn unit(&mut self) -> f32 {
        self.next() * 0.5 + 0.5
    }
}

/// A state-variable filter whose cut-off can move every sample; returns (low-pass, band-pass).
#[derive(Default)]
struct Filter {
    low: f32,
    band: f32,
}

impl Filter {
    fn run(&mut self, x: f32, hz: f32, damping: f32) -> (f32, f32) {
        let f = 2.0 * (PI * hz.clamp(20.0, 7000.0) / SR as f32).sin();
        self.low += f * self.band;
        let high = x - self.low - damping.clamp(0.15, 2.0) * self.band;
        self.band += f * high;
        if !(self.low.is_finite() && self.band.is_finite()) {
            *self = Self::default();
        }
        (self.low, self.band)
    }
}

fn frames(seconds: f32) -> usize {
    (seconds * SR as f32) as usize
}

/// Air rushing past: noise through a narrow band that sweeps up and back down while the sound
/// swells and fades, moving from left to right.
fn whoosh(seconds: f32, (lo, hi): (f32, f32), peak: f32, width: f32, seed: u32) -> Vec<[f32; 2]> {
    let n = frames(seconds);
    let (mut a, mut b) = (Noise(seed), Noise(seed ^ 0x9e37_79b9));
    let (mut f1, mut f2, mut f3, mut f4) = (Filter::default(), Filter::default(), Filter::default(), Filter::default());
    (0..n)
        .map(|i| {
            let t = i as f32 / n.max(1) as f32;
            // the pitch peaks more sharply than the loudness, so the sweep is heard
            let sweep = (-((t - peak) / width).powi(2)).exp();
            let swell = (-((t - peak) / (width * 1.5)).powi(2)).exp();
            let hz = lo * (hi / lo.max(1.0)).powf(sweep);
            // two bands in a row: a narrow, moving colour instead of plain hiss
            let (_, first) = f1.run(a.next(), hz, 0.22);
            let (_, hiss) = f2.run(first, hz, 0.35);
            let (_, body) = f3.run(b.next(), hz * 0.5, 0.5);
            let (rumble, _) = f4.run(b.next(), 120.0, 1.2);
            let s = (hiss + 0.8 * body + 0.35 * rumble * sweep) * swell;
            let pan = (0.3 + 0.4 * t) * PI / 2.0;
            [s * pan.cos(), s * pan.sin()]
        })
        .collect()
}

/// A low hit: a sine that drops in pitch, a click on top, and a short dull tail.
fn impact() -> Vec<[f32; 2]> {
    let n = frames(1.3);
    let (mut a, mut l, mut r) = (Noise(0x1234_5678), Noise(0x0bad_cafe), Noise(0x0c0f_fee1));
    let (mut click, mut body, mut tl, mut tr) = (Filter::default(), Filter::default(), Filter::default(), Filter::default());
    let mut phase = 0.0f32;
    (0..n)
        .map(|i| {
            let t = i as f32 / SR as f32;
            phase += 2.0 * PI * (36.0 + 85.0 * (-t * 22.0).exp()) / SR as f32;
            let sub = phase.sin() * (-t * 4.2).exp();
            let (c, _) = click.run(a.next(), 3200.0, 1.0);
            let (bd, _) = body.run(a.next(), 320.0, 1.0);
            let s = (1.7 * (sub + 0.8 * c * (-t * 70.0).exp() + 1.2 * bd * (-t * 7.0).exp())).tanh();
            let tail = (-t * 5.0).exp() * 0.25;
            [s + tl.run(l.next(), 900.0, 1.0).0 * tail, s + tr.run(r.next(), 900.0, 1.0).0 * tail]
        })
        .collect()
}

/// Tension that builds to a cut: noise and a tone that rise and stop dead.
fn riser() -> Vec<[f32; 2]> {
    let n = frames(1.6);
    let (mut a, mut b) = (Noise(0x5eed_0001), Noise(0x5eed_0002));
    let (mut fa, mut fb) = (Filter::default(), Filter::default());
    let mut phase = 0.0f32;
    (0..n)
        .map(|i| {
            let t = i as f32 / n.max(1) as f32;
            let secs = i as f32 / SR as f32;
            let hz = 300.0 * 20.0f32.powf(t);
            phase += 2.0 * PI * (200.0 * 6.0f32.powf(t)) / SR as f32;
            let tone = phase.sin() * t.powf(2.5) * 0.3 * (1.0 + 0.3 * (2.0 * PI * (6.0 + 10.0 * t) * secs).sin());
            let grow = t * t;
            // stops dead: the last 12 ms fall to nothing
            let stop = ((1.0 - t) * 1.6 / 0.012).min(1.0);
            [(fa.run(a.next(), hz, 0.5).1 * grow + tone) * stop, (fb.run(b.next(), hz, 0.5).1 * grow + tone) * stop]
        })
        .collect()
}

/// Digital stutter: short bursts of crushed tones and stepped noise with gaps between them.
fn glitch() -> Vec<[f32; 2]> {
    let n = frames(0.38);
    let mut rng = Noise(0x61_17c4);
    let mut out = vec![[0.0f32; 2]; n];
    let mut at = 0usize;
    while at < n {
        let len = frames(0.012 + 0.033 * rng.unit()).max(1);
        let kind = (rng.unit() * 5.0) as u32;
        let hz = 150.0 + 2350.0 * rng.unit().powi(2);
        let hold = (SR as f32 / (2000.0 + 7000.0 * rng.unit())).max(1.0) as usize;
        let level = 0.5 + 0.5 * rng.unit();
        let pan = (0.2 + 0.6 * rng.unit()) * PI / 2.0;
        let fade = 1.0 - (at as f32 / n as f32).powi(3);
        let mut held = 0.0f32;
        for k in 0..len.min(n - at) {
            let s = match kind {
                // a gap
                0 => 0.0,
                // a square tone, crushed to a few levels
                1 | 2 => {
                    let sq = if ((k as f32 * hz / SR as f32).fract()) < 0.5 { 1.0 } else { -1.0 };
                    (sq * 3.0f32).round() / 3.0
                }
                // noise that changes only every few samples
                _ => {
                    if k % hold == 0 {
                        held = rng.next();
                    }
                    held
                }
            };
            // 1 ms edges, so a burst is hard but does not click
            let edge = (k.min(len - 1 - k.min(len - 1)) as f32 / 48.0).min(1.0);
            let v = s * level * fade * edge;
            if let Some(o) = out.get_mut(at + k) {
                *o = [v * pan.cos(), v * pan.sin()];
            }
        }
        at += len;
    }
    out
}

/// A short, round blip for things that appear.
fn pop() -> Vec<[f32; 2]> {
    let n = frames(0.16);
    let mut a = Noise(0x0000_b0b0);
    let mut phase = 0.0f32;
    (0..n)
        .map(|i| {
            let t = i as f32 / SR as f32;
            phase += 2.0 * PI * (180.0 + 520.0 * (-t * 45.0).exp()) / SR as f32;
            let s = phase.sin() * (-t * 32.0).exp() + 0.3 * a.next() * (-t * 400.0).exp();
            [s, s]
        })
        .collect()
}

/// The finished sound: stereo, interleaved, faded at both ends and brought to its level.
pub fn render(s: &Sound) -> Vec<f32> {
    let mut v = match s.id {
        "whoosh" => whoosh(0.7, (220.0, 3800.0), 0.47, 0.17, 0x00a1_b2c3),
        "longWhoosh" => whoosh(1.25, (160.0, 3000.0), 0.48, 0.19, 0x0077_1234),
        "swish" => whoosh(0.3, (700.0, 5500.0), 0.43, 0.18, 0x0042_4242),
        "impact" => impact(),
        "riser" => riser(),
        "glitch" => glitch(),
        _ => pop(),
    };
    let n = v.len();
    // 4 ms in, 8 ms out (the impact and the pop start at once: 0.5 ms)
    let fade_in = if matches!(s.id, "impact" | "pop") { 24 } else { 192 };
    let mut peak = 0.0f32;
    for (i, f) in v.iter_mut().enumerate() {
        let g = ((i + 1) as f32 / fade_in as f32).min(1.0) * ((n - i) as f32 / 384.0).min(1.0);
        for c in f.iter_mut() {
            *c = if c.is_finite() { *c * g } else { 0.0 };
            peak = peak.max(c.abs());
        }
    }
    let gain = if peak > 1e-6 { s.level / peak } else { 0.0 };
    v.iter().flat_map(|f| [f[0] * gain, f[1] * gain]).collect()
}

// ------------------------------------------------------------------ project

fn bin(s: &mut Session) -> Result<BinId> {
    let found = s.project.root.children.iter().find_map(|c| match c {
        BinEntry::Bin(b) if b.name == BIN => Some(b.id),
        _ => None,
    });
    match found {
        Some(id) => Ok(id),
        None => s.edit("New Bin", |pr, _| Ok(pr.add_bin(BIN, None))),
    }
}

/// The project item of `sound` (written and imported on first use) and its length.
fn item(s: &mut Session, sound: &Sound, p: &Value) -> Result<(ItemId, Tick)> {
    let have =
        s.project.items.values().find(|i| i.name == sound.file && matches!(&i.kind, ItemKind::Media(m) if !m.offline && m.info.has_audio())).map(|i| i.id);
    let id = match have {
        Some(id) => id,
        None => {
            let samples = render(sound);
            let dir = crate::voiceover::media_dir(s, p, "Sounds");
            let path = std::path::Path::new(&dir).join(sound.file).to_string_lossy().into_owned();
            if !cfg!(target_arch = "wasm32") {
                std::fs::create_dir_all(&dir).map_err(|e| EngineError::Other(format!("{dir}: {e}")))?;
            }
            let bytes = crate::previews::write_wav_f32(&samples, SR);
            s.services.write_file(&path, &bytes).map_err(|e| EngineError::Other(format!("{path}: {e}")))?;
            let bin = bin(s)?;
            crate::commands::import_bytes(s, &path, bytes.into(), Some(bin))?
        }
    };
    let dur = s.project.item(id).map(|i| i.duration()).filter(|d| *d > Tick::ZERO).ok_or_else(|| EngineError::Other(format!("{} has no sound", sound.file)))?;
    Ok((id, dur))
}

/// The "SFX" audio track of the open sequence, added when there is none (or it is locked).
fn sfx_track(s: &mut Session) -> Result<TrackId> {
    let seq = s.active_sequence().ok_or(EngineError::NoSequence)?;
    if let Some(t) = seq.audio_tracks.iter().find(|t| t.name == TRACK && !t.locked) {
        return Ok(t.id);
    }
    let before: Vec<TrackId> = seq.audio_tracks.iter().map(|t| t.id).collect();
    s.execute("sequence.addTracks", json!({"video": 0, "audio": 1, "audioAfter": "last"}))?;
    let id = s
        .active_sequence()
        .and_then(|q| q.audio_tracks.iter().map(|t| t.id).find(|id| !before.contains(id)))
        .ok_or_else(|| EngineError::Other("could not add an audio track".into()))?;
    s.edit_sequence("Name Track", |q, _, _| {
        if let Some(t) = q.audio_tracks.iter_mut().find(|t| t.id == id) {
            t.name = TRACK.to_string();
        }
        Ok(())
    })?;
    Ok(id)
}

/// Lay `sounds` (which sound, and the sequence time its peak belongs at) on the SFX track as one
/// undo step. Returns how many were placed and how many found the track taken.
fn place(s: &mut Session, sounds: &[(&'static Sound, Tick)], p: &Value, label: &str) -> Result<Value> {
    let seq_id = s.state.active_sequence.ok_or(EngineError::NoSequence)?;
    let n0 = s.history.undo.len();
    let result = (|| -> Result<Value> {
        let track = sfx_track(s)?;
        let rate = s.project.sequence(seq_id).map(|q| q.settings.frame_rate).unwrap_or_default();
        let mut plan = Vec::new();
        for (sound, at) in sounds.iter().take(MAX_SOUNDS) {
            let (id, dur) = item(s, sound, p)?;
            // the peak on `at`; a sound that would start before the sequence loses its head
            let start = *at - Tick::from_seconds_f64(sound.peak);
            let skip = if start < Tick::ZERO { Tick::ZERO - start } else { Tick::ZERO };
            if skip < dur {
                plan.push((id, start.max(Tick::ZERO), TimeRange::new(skip, dur - skip)));
            }
        }
        let (placed, taken) = s.edit(label, |pr, st| {
            let (mut placed, mut taken) = (Vec::new(), 0usize);
            for (id, start, source) in &plan {
                let free = pr
                    .sequence(seq_id)
                    .and_then(|q| q.audio_tracks.iter().find(|t| t.id == track))
                    .is_some_and(|t| !t.items.iter().any(|it| it.start < *start + source.duration && *start < it.end()));
                if !free {
                    taken += 1;
                    continue;
                }
                let mut it =
                    pr.make_track_item(*id, TrackKind::Audio, *start, *source, rate).ok_or(EngineError::Other("the sound is not importable".into()))?;
                it.duration = source.duration;
                let clip = it.id;
                let seq = pr.sequence_mut(seq_id).ok_or(EngineError::NoSequence)?;
                let t = seq.audio_tracks.iter_mut().find(|t| t.id == track).ok_or_else(|| EngineError::Other("the SFX track was deleted".into()))?;
                let pos = t.items.partition_point(|x| x.start <= *start);
                t.items.insert(pos, it);
                placed.push(clip);
            }
            if let Some(seq) = pr.sequence(seq_id) {
                seq.check().map_err(EngineError::Other)?;
            }
            if !placed.is_empty() {
                st.selection = placed.clone();
            }
            Ok((placed, taken))
        })?;
        Ok(json!({"placed": placed.iter().map(|c| c.0).collect::<Vec<_>>(), "skipped": taken, "track": TRACK}))
    })();
    crate::clip_ops::collapse_history(s, n0, label);
    if result.is_err() && s.history.undo.len() > n0 {
        let _ = s.undo();
        s.history.redo.clear();
    }
    result
}

/// The sound that suits a transition (by its effect id) of `seconds`.
fn sound_for(effect: &str, seconds: f64) -> &'static str {
    let has = |words: &[&str]| words.iter().any(|w| effect.contains(w));
    if has(&["glitch", "vhs", "digital", "static", "block"]) {
        "glitch"
    } else if has(&["punch", "zoom", "flash", "white", "burn", "impact"]) {
        "impact"
    } else if has(&["dissolve", "dip", "fade", "morph"]) {
        "swish"
    } else if seconds > 0.9 {
        "longWhoosh"
    } else {
        "whoosh"
    }
}

fn add_to_transitions(s: &mut Session, p: &Value) -> Result<Value> {
    let seq = s.active_sequence().ok_or(EngineError::NoSequence)?;
    let mut found: Vec<(&'static Sound, Tick)> = Vec::new();
    for t in &seq.video_tracks {
        for x in t.transitions.iter().filter(|x| x.from.is_some() && x.to.is_some()) {
            if let Some(snd) = sound(sound_for(&x.effect.effect, x.duration.seconds())) {
                found.push((snd, x.start + Tick(x.duration.0 / 2)));
            }
        }
    }
    if found.is_empty() {
        return Err(bad("sounds.addToTransitions", "there are no transitions between clips in this sequence"));
    }
    found.sort_by_key(|f| f.1);
    place(s, &found, p, "Add Transition Sounds")
}

fn add_to_cuts(s: &mut Session, p: &Value) -> Result<Value> {
    let seq = s.active_sequence().ok_or(EngineError::NoSequence)?;
    let swish = sound("swish").ok_or_else(|| EngineError::Other("the sound pack is incomplete".into()))?;
    let mut found = Vec::new();
    if let Some(v1) = seq.video_tracks.first() {
        for pair in v1.items.windows(2) {
            let cut = pair[1].start;
            // a plain cut: the clips touch and no transition sits on it
            if pair[0].end() == cut && !v1.transitions.iter().any(|x| x.start <= cut && cut <= x.end()) {
                found.push((swish, cut));
            }
        }
    }
    if found.is_empty() {
        return Err(bad("sounds.addToCuts", "there are no plain cuts on V1"));
    }
    place(s, &found, p, "Add Cut Sounds")
}

fn add_one(s: &mut Session, id: &str, p: &Value) -> Result<Value> {
    let snd = sound(id).ok_or_else(|| bad("sounds.add", format!("unknown sound `{id}` ({})", SOUNDS.iter().map(|s| s.id).collect::<Vec<_>>().join(", "))))?;
    let at = match f64_p(p, "seconds") {
        Some(v) if v.is_finite() && (0.0..=86_400.0).contains(&v) => Tick::from_seconds_f64(v),
        Some(_) => return Err(bad("sounds.add", "`seconds` must be between 0 and 86400")),
        None => s.playhead(),
    };
    let r = place(s, &[(snd, at)], p, "Add Sound Effect")?;
    if r["placed"].as_array().is_none_or(|a| a.is_empty()) {
        return Err(bad("sounds.add", "the SFX track already has a clip there"));
    }
    Ok(r)
}

fn import_all(s: &mut Session, p: &Value) -> Result<Value> {
    let n0 = s.history.undo.len();
    let result: Result<Vec<u64>> = SOUNDS.iter().map(|snd| item(s, snd, p).map(|(id, _)| id.0)).collect();
    crate::clip_ops::collapse_history(s, n0, "Import Sound Effects");
    Ok(json!({"items": result?}))
}

const MENU: &[&str] = &["Sequence", "Sound Effects"];

pub fn commands() -> Vec<CommandSpec> {
    let spec = |id, label, run: fn(&mut Session, &Value) -> Result<Value>| CommandSpec {
        id,
        label,
        menu: MENU,
        shortcut: None,
        params: "{}",
        enabled: has_seq,
        run,
        journal: true,
    };
    vec![
        spec("sounds.addToTransitions", "Add Sounds to All Transitions", add_to_transitions),
        spec("sounds.addToCuts", "Add Swish to Every Cut", add_to_cuts),
        spec("sounds.add.whoosh", "Whoosh", |s, p| add_one(s, "whoosh", p)),
        spec("sounds.add.longWhoosh", "Long Whoosh", |s, p| add_one(s, "longWhoosh", p)),
        spec("sounds.add.swish", "Swish", |s, p| add_one(s, "swish", p)),
        spec("sounds.add.impact", "Impact", |s, p| add_one(s, "impact", p)),
        spec("sounds.add.riser", "Riser", |s, p| add_one(s, "riser", p)),
        spec("sounds.add.glitch", "Glitch", |s, p| add_one(s, "glitch", p)),
        spec("sounds.add.pop", "Pop", |s, p| add_one(s, "pop", p)),
        CommandSpec {
            id: "sounds.importAll",
            label: "Import All Sounds into the Project",
            menu: MENU,
            shortcut: None,
            params: "{}",
            enabled: |_| Ok(()),
            run: import_all,
            journal: true,
        },
        CommandSpec {
            id: "sounds.add",
            label: "Add Sound Effect",
            menu: &[],
            shortcut: None,
            params: r#"{"sound":"whoosh|longWhoosh|swish|impact|riser|glitch|pop","seconds":f64?}"#,
            enabled: has_seq,
            run: |s, p| {
                let id = str_p(p, "sound").ok_or_else(|| bad("sounds.add", "pass `sound`"))?.to_string();
                add_one(s, &id, p)
            },
            journal: true,
        },
    ]
}

#[cfg(test)]
mod tests {
    use super::*;

    fn demo() -> Session {
        let mut s = Session::default();
        s.execute("file.openDemoProject", json!({})).unwrap();
        s
    }

    fn sfx(s: &Session) -> Vec<(f64, f64, String)> {
        let q = s.active_sequence().unwrap();
        q.audio_tracks.iter().filter(|t| t.name == TRACK).flat_map(|t| &t.items).map(|i| (i.start.seconds(), i.duration.seconds(), i.name.clone())).collect()
    }

    #[test]
    fn every_sound_is_clean_repeatable_and_peaks_where_it_says() {
        for snd in SOUNDS {
            let v = render(snd);
            assert_eq!(v.len() % 2, 0);
            let secs = v.len() as f64 / 2.0 / f64::from(SR);
            assert!((0.1..=2.0).contains(&secs), "{}: {secs} s", snd.id);
            assert!(v.iter().all(|x| x.is_finite() && x.abs() <= 1.0), "{}", snd.id);
            let peak = v.iter().fold(0.0f32, |m, x| m.max(x.abs()));
            assert!((peak - snd.level).abs() < 1e-3, "{}: peak {peak}", snd.id);
            // silent at the very end, and (but for the hits) at the start: no click on the cut
            assert!(v[v.len() - 2].abs() < 0.01 && v[v.len() - 1].abs() < 0.01, "{}", snd.id);
            assert!(v[0].abs() < 0.05 && v[1].abs() < 0.05, "{}", snd.id);
            // the loudest 50 ms lie around the stated peak
            let win = (0.05 * f64::from(SR)) as usize;
            let energy = |at: usize| v[at * 2..((at + win) * 2).min(v.len())].iter().map(|x| f64::from(x * x)).sum::<f64>();
            let n = v.len() / 2;
            let loudest = (0..n.saturating_sub(win)).step_by(240).max_by(|a, b| energy(*a).total_cmp(&energy(*b))).unwrap_or(0);
            let centre = (loudest + win / 2) as f64 / f64::from(SR);
            assert!((centre - snd.peak).abs() < 0.2, "{}: loudest at {centre:.2} s, peak says {}", snd.id, snd.peak);
            assert_eq!(render(snd), v, "{}: the same every time", snd.id);
            // something to hear on both sides
            let (l, r): (f32, f32) = v.chunks(2).fold((0.0, 0.0), |m, f| (m.0 + f[0].abs(), m.1 + f[1].abs()));
            assert!(l > 1.0 && r > 1.0, "{}: {l} {r}", snd.id);
        }
        assert_eq!(sound_for("plus_glitch_slice", 0.5), "glitch");
        assert_eq!(sound_for("plus_punch", 0.5), "impact");
        assert_eq!(sound_for("cross_dissolve", 1.0), "swish");
        assert_eq!(sound_for("push", 0.5), "whoosh");
        assert_eq!(sound_for("push", 1.5), "longWhoosh");
    }

    #[test]
    fn transitions_get_their_sounds_once() {
        let mut s = demo();
        let q = s.active_sequence().unwrap().clone();
        let between: Vec<f64> = q
            .video_tracks
            .iter()
            .flat_map(|t| &t.transitions)
            .filter(|x| x.from.is_some() && x.to.is_some())
            .map(|x| (x.start + Tick(x.duration.0 / 2)).seconds())
            .collect();
        assert!(!between.is_empty(), "the demo has transitions between clips");
        let (tracks, undo) = (q.audio_tracks.len(), s.history.undo.len());
        let r = s.execute("sounds.addToTransitions", json!({})).unwrap();
        assert_eq!(r["placed"].as_array().unwrap().len(), between.len());
        assert_eq!(s.active_sequence().unwrap().audio_tracks.len(), tracks + 1, "an SFX track was added");
        assert_eq!(s.history.undo.len(), undo + 1, "one undo step");
        let placed = sfx(&s);
        assert_eq!(placed.len(), between.len());
        // each sound's peak sits on the middle of its transition
        for mid in &between {
            assert!(
                placed.iter().any(|(start, _, name)| {
                    let snd = SOUNDS.iter().find(|x| x.file == name).expect("a pack sound");
                    (start + snd.peak - mid).abs() < 0.002
                }),
                "no sound peaks at {mid}: {placed:?}"
            );
        }
        // the sounds are in their bin, and the picture's own sound is untouched
        let bin = s.project.root.children.iter().find_map(|c| if let BinEntry::Bin(b) = c { (b.name == BIN).then_some(b) } else { None }).expect("bin");
        assert!(!bin.children.is_empty());
        assert_eq!(s.active_sequence().unwrap().audio_tracks[0], q.audio_tracks[0]);
        // again: nothing doubles
        let again = s.execute("sounds.addToTransitions", json!({})).unwrap();
        assert!(again["placed"].as_array().unwrap().is_empty());
        assert_eq!(again["skipped"].as_u64(), Some(between.len() as u64));
        assert_eq!(sfx(&s).len(), between.len());
        // undo takes the sounds and the track away again
        s.execute("edit.undo", json!({})).unwrap();
        s.execute("edit.undo", json!({})).unwrap();
        assert_eq!(s.active_sequence().unwrap().audio_tracks.len(), tracks);
        // and the mix still renders
        s.execute("edit.redo", json!({})).unwrap();
        assert!(s.render_program(0.125).is_some());
    }

    #[test]
    fn single_sounds_cuts_and_the_pack() {
        let mut s = demo();
        // at a time; near the start the sound loses its head instead of moving
        let r = s.execute("sounds.add", json!({"sound": "impact", "seconds": 3.0})).unwrap();
        assert_eq!(r["placed"].as_array().unwrap().len(), 1);
        let one = sfx(&s);
        assert!((one[0].0 - (3.0 - 0.02)).abs() < 1e-3, "{one:?}");
        s.execute("sounds.add", json!({"sound": "riser", "seconds": 0.5})).unwrap();
        let two = sfx(&s);
        let riser = two.iter().find(|x| x.2.contains("Riser")).expect("riser");
        assert!(riser.0.abs() < 1e-9 && (riser.1 - (1.6 - 1.08)).abs() < 0.01, "{riser:?}");
        // taken, unknown and hostile requests are refused
        assert!(s.execute("sounds.add", json!({"sound": "pop", "seconds": 3.0})).is_err(), "there is a clip there");
        assert!(s.execute("sounds.add", json!({"sound": "kazoo"})).is_err());
        for bad in [-1.0, 1e12] {
            assert!(s.execute("sounds.add", json!({"sound": "pop", "seconds": bad})).is_err());
        }
        // at the playhead from the menu
        s.execute("playhead.set", json!({"time": Tick::from_seconds_f64(8.0).0})).unwrap();
        s.execute("sounds.add.whoosh", json!({})).unwrap();
        assert!(sfx(&s).iter().any(|x| x.2.contains("Whoosh") && (x.0 + 0.33 - 8.0).abs() < 0.05));
        // every plain cut of V1 gets a swish (those under a transition do not)
        let q = s.active_sequence().unwrap().clone();
        let v1 = &q.video_tracks[0];
        let cuts =
            v1.items.windows(2).filter(|w| w[0].end() == w[1].start && !v1.transitions.iter().any(|x| x.start <= w[1].start && w[1].start <= x.end())).count();
        match s.execute("sounds.addToCuts", json!({})) {
            Ok(r) => assert_eq!(r["placed"].as_array().unwrap().len() + r["skipped"].as_u64().unwrap() as usize, cuts),
            Err(_) => assert_eq!(cuts, 0),
        }
        // the whole pack lands in the bin once
        let before = s.project.items.len();
        let r = s.execute("sounds.importAll", json!({})).unwrap();
        assert_eq!(r["items"].as_array().unwrap().len(), SOUNDS.len());
        let after = s.project.items.len();
        s.execute("sounds.importAll", json!({})).unwrap();
        assert_eq!(s.project.items.len(), after, "already there");
        assert!(after > before);
    }
}

/// Local preview: `FILMCRAFT_SOUND_PREVIEW=<dir>` writes every sound of the pack there as a WAV.
#[cfg(test)]
#[test]
#[ignore = "writes preview files"]
fn sound_preview() {
    let Some(dir) = std::env::var_os("FILMCRAFT_SOUND_PREVIEW") else { return };
    for snd in SOUNDS {
        std::fs::write(std::path::Path::new(&dir).join(snd.file), crate::previews::write_wav_f32(&render(snd), SR)).expect("write");
    }
}
