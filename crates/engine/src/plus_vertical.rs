//! FilmCraft Plus **Vertical Version**: one command turns the open sequence into a copy in
//! another shape (9:16 for Reels, TikTok and Shorts; 1:1; 4:5), ready to export.
//!
//! | command | menu | what it does |
//! |---|---|---|
//! | `sequence.reframe.9x16` | Sequence ▸ Vertical Version ▸ 9:16 (Reels / TikTok / Shorts) | a 9:16 copy |
//! | `sequence.reframe.1x1` | … ▸ 1:1 (Square) | a square copy |
//! | `sequence.reframe.4x5` | … ▸ 4:5 (Portrait) | a 4:5 copy |
//! | `sequence.reframe` | — | `{"aspect":"9:16"\|"1:1"\|"4:5"}` |
//!
//! The copy keeps the edit (clips, cuts, transitions, sound, captions, markers) and the original
//! stays as it is. In the copy every clip that filled the old frame gets **Face Follow**
//! (`face_follow`): it fills the new frame and pans with the face. Smaller clips (picture in
//! picture, titles) keep their size and their place relative to the middle; adjustment layers
//! are scaled to cover the new frame.

use std::sync::Arc;

use filmcraft_geom::{Rect, Vec2};
use filmcraft_project::{ItemId, ItemKind, ParamValue, Sequence, TrackItem, find_effect};
use serde_json::{Value, json};

use crate::commands::{CommandSpec, bad, has_seq, str_p};
use crate::{EngineError, Result, Session};

const FOLLOW: &str = "face_follow";

/// The frame size of the `aspect` (`w:h`) version of a `(w, h)` sequence: its short side stays.
fn new_size((w, h): (u32, u32), aspect: (u32, u32)) -> (u32, u32) {
    let short = w.min(h).max(2);
    let even = |v: u64| (v.min(u64::from(u32::MAX)) as u32) & !1;
    let (a, b) = (u64::from(aspect.0.max(1)), u64::from(aspect.1.max(1)));
    if a <= b { (even(u64::from(short)), even(u64::from(short) * b / a)) } else { (even(u64::from(short) * a / b), even(u64::from(short))) }
}

/// Whether `it` covers (nearly) the whole frame of `seq` in the middle of the clip.
fn fills_frame(project: &filmcraft_project::Project, seq: &Sequence, it: &TrackItem) -> bool {
    let Some(size) = project.source_size(it.item) else { return false };
    let mt = it.effect_time_at(it.start + filmcraft_time::Tick(it.duration.0 / 2));
    let m = filmcraft_render::motion_matrix(seq, it, size, filmcraft_render::source_par(project, it.item), mt);
    let b = m.bounds(&Rect { x: 0.0, y: 0.0, w: f64::from(size.0), h: f64::from(size.1) });
    let (w, h) = (f64::from(seq.settings.width), f64::from(seq.settings.height));
    b.x <= 0.02 * w && b.y <= 0.02 * h && b.x + b.w >= 0.98 * w && b.y + b.h >= 0.98 * h
}

fn reframe(s: &mut Session, aspect: (u32, u32)) -> Result<Value> {
    const CMD: &str = "sequence.reframe";
    let seq_id = s.state.active_sequence.ok_or(EngineError::NoSequence)?;
    let old = s.active_sequence().ok_or(EngineError::NoSequence)?.clone();
    let (ow, oh) = (old.settings.width, old.settings.height);
    let (nw, nh) = new_size((ow, oh), aspect);
    if (nw, nh) == (ow, oh) {
        return Err(bad(CMD, format!("the sequence already is {}:{}", aspect.0, aspect.1)));
    }
    let mut settings = old.settings.clone();
    settings.width = nw;
    settings.height = nh;
    settings.preset = format!("{nw}x{nh} {}", settings.frame_rate.label());
    settings.validate().map_err(|e| bad(CMD, e))?;
    let base = s.project.item(seq_id).map(|i| i.name.clone()).unwrap_or_default();
    let tag = format!("{}x{}", aspect.0, aspect.1);
    let name = (1..1000u32)
        .map(|k| if k == 1 { format!("{base} {tag}") } else { format!("{base} {tag} ({k})") })
        .find(|n| !s.project.items.values().any(|i| &i.name == n))
        .unwrap_or_else(|| format!("{base} {tag}"));
    let shift = Vec2::new((f64::from(nw) - f64::from(ow)) / 2.0, (f64::from(nh) - f64::from(oh)) / 2.0);
    let cover = (f64::from(nw) / f64::from(ow.max(1))).max(f64::from(nh) / f64::from(oh.max(1))).max(1.0);
    let def = find_effect(FOLLOW).ok_or_else(|| EngineError::Other("Face Follow is not available".into()))?;
    let mut seq = old.clone();
    seq.settings = settings;
    let mut followed = 0usize;
    for track in &mut seq.video_tracks {
        for it in &mut track.items {
            let kind = s.project.item(it.item).map(|p| &p.kind);
            let full = fills_frame(&s.project, &old, it);
            // everything keeps its place relative to the middle of the frame
            if let Some(pos) = it.effect_mut("motion").and_then(|m| m.params.get_mut("position")) {
                pos.map_values(|v| match v {
                    ParamValue::Vec2(p) => ParamValue::Vec2(Vec2::new(p.x + shift.x, p.y + shift.y)),
                    other => other,
                });
            }
            match kind {
                Some(ItemKind::AdjustmentLayer { .. }) if full => {
                    if let Some(scale) = it.effect_mut("motion").and_then(|m| m.params.get_mut("scale")) {
                        scale.map_values(|v| match v {
                            ParamValue::Float(x) => ParamValue::Float(x * cover),
                            other => other,
                        });
                    }
                }
                Some(ItemKind::Media(_) | ItemKind::Subclip { .. } | ItemKind::Sequence(_)) if full && it.effect(FOLLOW).is_none() => {
                    it.effects.push(def.instance());
                    followed += 1;
                }
                _ => {}
            }
        }
    }
    let id: ItemId = s.edit("Make Vertical Version", |pr, st| {
        let label = pr.item(seq_id).map(|i| i.label).ok_or(EngineError::NoSequence)?;
        let bin = pr.root.parent_of(seq_id);
        let id = pr.add_item(&name, label, ItemKind::Sequence(Arc::new(seq)), bin);
        st.active_sequence = Some(id);
        if !st.open_sequences.contains(&id) {
            st.open_sequences.push(id);
        }
        st.selection.clear();
        Ok(id)
    })?;
    s.events.push(crate::Event::OpenSequence(id));
    Ok(json!({"sequence": id.0, "name": name, "width": nw, "height": nh, "followed": followed}))
}

fn aspect_of(text: &str) -> Option<(u32, u32)> {
    match text.trim().replace(['x', 'X', '/'], ":").as_str() {
        "9:16" => Some((9, 16)),
        "1:1" => Some((1, 1)),
        "4:5" => Some((4, 5)),
        "16:9" => Some((16, 9)),
        _ => None,
    }
}

const MENU: &[&str] = &["Sequence", "Vertical Version"];

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
        spec("sequence.reframe.9x16", "9:16 (Reels / TikTok / Shorts)", |s, _| reframe(s, (9, 16))),
        spec("sequence.reframe.1x1", "1:1 (Square)", |s, _| reframe(s, (1, 1))),
        spec("sequence.reframe.4x5", "4:5 (Portrait)", |s, _| reframe(s, (4, 5))),
        CommandSpec {
            id: "sequence.reframe",
            label: "Make Version in Another Shape",
            menu: &[],
            shortcut: None,
            params: r#"{"aspect":"9:16|1:1|4:5|16:9"="9:16"}"#,
            enabled: has_seq,
            run: |s, p| {
                let text = str_p(p, "aspect").unwrap_or("9:16");
                let aspect = aspect_of(text).ok_or_else(|| bad("sequence.reframe", format!("unknown aspect `{text}` (9:16, 1:1, 4:5, 16:9)")))?;
                reframe(s, aspect)
            },
            journal: true,
        },
    ]
}

#[cfg(test)]
mod tests {
    use super::*;
    use filmcraft_project::TrackKind;

    #[test]
    fn sizes_keep_the_short_side_and_stay_even() {
        assert_eq!(new_size((1920, 1080), (9, 16)), (1080, 1920));
        assert_eq!(new_size((3840, 2160), (9, 16)), (2160, 3840));
        assert_eq!(new_size((1920, 1080), (1, 1)), (1080, 1080));
        assert_eq!(new_size((1920, 1080), (4, 5)), (1080, 1350));
        assert_eq!(new_size((1080, 1920), (16, 9)), (1920, 1080));
        assert_eq!(new_size((1281, 721), (9, 16)), (720, 1280));
        for hostile in [(0, 0), (1, 1), (u32::MAX, u32::MAX), (u32::MAX, 3)] {
            let (w, h) = new_size(hostile, (9, 16));
            assert!(w % 2 == 0 && h % 2 == 0);
        }
        assert_eq!(aspect_of(" 9x16 "), Some((9, 16)));
        assert_eq!(aspect_of("2:1"), None);
    }

    #[test]
    fn the_vertical_version_keeps_the_edit_and_follows_faces() {
        let mut s = Session::default();
        s.execute("file.openDemoProject", json!({})).unwrap();
        let old_id = s.state.active_sequence.unwrap();
        let old = s.active_sequence().unwrap().clone();
        let before = s.project.sequences().count();
        let r = s.execute("sequence.reframe.9x16", json!({})).unwrap();
        assert_eq!((r["width"].as_u64(), r["height"].as_u64()), (Some(1080), Some(1920)));
        assert_eq!(s.project.sequences().count(), before + 1);
        // the copy is open, the original untouched
        let new_id = s.state.active_sequence.unwrap();
        assert_ne!(new_id, old_id);
        assert_eq!(s.project.sequence(old_id).unwrap(), &old);
        let q = s.active_sequence().unwrap().clone();
        assert_eq!((q.settings.width, q.settings.height), (1080, 1920));
        assert_eq!(q.settings.frame_rate, old.settings.frame_rate);
        assert!(s.project.item(new_id).unwrap().name.ends_with("9x16"));
        // same clips, cuts, transitions and sound
        for kind in [TrackKind::Video, TrackKind::Audio] {
            for (a, b) in old.tracks(kind).iter().zip(q.tracks(kind)) {
                assert_eq!(
                    a.items.iter().map(|i| (i.start, i.duration, i.item)).collect::<Vec<_>>(),
                    b.items.iter().map(|i| (i.start, i.duration, i.item)).collect::<Vec<_>>()
                );
                assert_eq!(a.transitions.len(), b.transitions.len());
            }
        }
        // the full-frame clips on V1 follow the face, and it renders a full vertical picture
        let v1 = &q.video_tracks[0].items;
        assert!(!v1.is_empty() && v1.iter().all(|i| i.effect(FOLLOW).is_some()), "V1 clips get Face Follow");
        assert_eq!(
            r["followed"].as_u64().map(|n| n as usize),
            Some(q.video_tracks.iter().flat_map(|t| &t.items).filter(|i| i.effect(FOLLOW).is_some()).count())
        );
        let mid = v1[1].start + filmcraft_time::Tick(v1[1].duration.0 / 2);
        let img = s.render_program_at(0.25, mid).expect("frame");
        assert!(img.w < img.h, "portrait: {}x{}", img.w, img.h);
        let covered = img.px.chunks(4).filter(|p| p[3] > 0.99).count();
        assert!(covered * 100 >= img.w * img.h * 99, "the clip fills the vertical frame ({covered} of {})", img.w * img.h);
        // one undo removes the copy and reopens nothing broken
        s.execute("edit.undo", json!({})).unwrap();
        assert_eq!(s.project.sequences().count(), before);
        // again: a second copy gets its own name; the same shape is refused
        s.execute("sequence.open", json!({"item": old_id.0})).unwrap();
        let a = s.execute("sequence.reframe", json!({"aspect": "9:16"})).unwrap()["name"].as_str().unwrap().to_string();
        assert!(s.execute("sequence.reframe.9x16", json!({})).is_err(), "already 9:16");
        s.execute("sequence.open", json!({"item": old_id.0})).unwrap();
        let b = s.execute("sequence.reframe", json!({"aspect": "9x16"})).unwrap()["name"].as_str().unwrap().to_string();
        assert_ne!(a, b);
        assert!(s.execute("sequence.reframe", json!({"aspect": "banana"})).is_err());
    }

    #[test]
    fn small_overlays_keep_their_size_and_place() {
        let mut s = Session::default();
        s.execute("file.newSequence", json!({"name": "wide", "width": 1920, "height": 1080, "fps": 30, "video": 2, "audio": 1})).unwrap();
        let big = s.execute("file.newColorMatte", json!({"width": 1920, "height": 1080, "color": "#2040ff"})).unwrap()["item"].as_u64().unwrap();
        let small = s.execute("file.newColorMatte", json!({"width": 320, "height": 180, "color": "#ff0000"})).unwrap()["item"].as_u64().unwrap();
        s.execute("timeline.place", json!({"item": big, "track": "V1", "seconds": 0.0})).unwrap();
        s.execute("timeline.place", json!({"item": small, "track": "V2", "seconds": 0.0})).unwrap();
        s.execute("sequence.reframe.9x16", json!({})).unwrap();
        let q = s.active_sequence().unwrap();
        assert!(q.video_tracks[0].items[0].effect(FOLLOW).is_some());
        assert!(q.video_tracks[1].items[0].effect(FOLLOW).is_none(), "a small overlay is not blown up");
        let img = s.render_program_at(0.5, filmcraft_time::Tick::from_seconds_f64(0.5)).expect("frame");
        // the small red matte is still in the middle, the blue one fills the rest
        let centre = img.get(img.w / 2, img.h / 2);
        assert!(centre[0] > 0.5 && centre[2] < 0.2, "{centre:?}");
        let top = img.get(img.w / 2, 4);
        assert!(top[2] > 0.5 && top[0] < 0.2, "{top:?}");
    }
}

/// Local preview: `FILMCRAFT_FOLLOW_PREVIEW=<dir>` with wide `*.png` pictures of a person; writes
/// `<name>.9x16_<w>x<h>.rgb` (the vertical version's frame) next to each.
#[cfg(test)]
#[test]
#[ignore = "needs local test photographs"]
fn follow_preview() {
    let Some(dir) = std::env::var_os("FILMCRAFT_FOLLOW_PREVIEW") else { return };
    for entry in std::fs::read_dir(&dir).expect("dir").flatten() {
        let path = entry.path();
        if path.extension().and_then(|e| e.to_str()) != Some("png") {
            continue;
        }
        let mut s = Session::default();
        s.execute("file.newSequence", json!({"name": "wide", "width": 1920, "height": 1080, "fps": 30, "video": 1, "audio": 1})).unwrap();
        let r = s.execute("file.import", json!({"paths": [path.to_string_lossy()]})).unwrap();
        let item = r["items"][0].as_u64().expect("imported");
        s.execute("timeline.place", json!({"item": item, "track": "V1", "seconds": 0.0})).unwrap();
        s.execute("sequence.reframe.9x16", json!({})).unwrap();
        let img = s.render_program_at(0.5, filmcraft_time::Tick::from_seconds_f64(1.0)).expect("frame");
        let out: Vec<u8> = img.px.chunks(4).flat_map(|p| [p[0], p[1], p[2]].map(|v| (v.clamp(0.0, 1.0) * 255.0).round() as u8)).collect();
        let stem = path.file_stem().map(|n| n.to_string_lossy().to_string()).unwrap_or_default();
        std::fs::write(path.with_file_name(format!("{stem}.9x16_{}x{}.rgb", img.w, img.h)), out).expect("write");
    }
}
