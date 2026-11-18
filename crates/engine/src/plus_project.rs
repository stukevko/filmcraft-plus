//! FilmCraft Plus project presets (not File ▸ New ▸ Project from Template…, which copies a
//! project file): one command starts a new project with a sequence that fits
//! what it is for (a vertical Reel, a YouTube video, a music video), so the Home screen can offer
//! them as one click.
//!
//! | command | what it does |
//! |---|---|
//! | `file.newProjectPreset` | replaces the project with a new one named `name` and, unless the template is `empty`, one open sequence with the template's frame size and rate |
//!
//! The new project is clean (nothing to save yet) and its history is empty, like File ▸ New ▸
//! Project.

use serde_json::{Value, json};

use crate::commands::{CommandSpec, bad, str_p};
use crate::{Result, Session};

/// A starting point for a new project.
#[derive(Clone, Copy, Debug)]
pub struct Template {
    pub id: &'static str,
    /// Card title (English; the UI translates it).
    pub label: &'static str,
    /// Name of the sequence it creates.
    pub sequence: &'static str,
    /// Frame size; `(0, 0)` creates no sequence.
    pub size: (u32, u32),
    pub fps: u32,
}

impl Template {
    /// Whether the template creates a sequence.
    pub fn has_sequence(&self) -> bool {
        self.size.0 > 0 && self.size.1 > 0
    }
}

/// The templates, in the order Home shows them.
pub const TEMPLATES: &[Template] = &[
    Template { id: "reel", label: "Reel / TikTok / Short", sequence: "Reel", size: (1080, 1920), fps: 30 },
    Template { id: "youtube", label: "YouTube 1080p", sequence: "YouTube", size: (1920, 1080), fps: 30 },
    Template { id: "youtube4k", label: "YouTube 4K", sequence: "YouTube 4K", size: (3840, 2160), fps: 30 },
    Template { id: "musicVideo", label: "Music Video", sequence: "Music Video", size: (1920, 1080), fps: 24 },
    Template { id: "empty", label: "Empty Project", sequence: "", size: (0, 0), fps: 0 },
];

/// Longest project name kept (characters).
const MAX_NAME: usize = 120;

fn new_from_template(s: &mut Session, p: &Value) -> Result<Value> {
    const CMD: &str = "file.newProjectPreset";
    let id = str_p(p, "preset").unwrap_or("empty");
    let t = TEMPLATES.iter().find(|t| t.id == id).ok_or_else(|| bad(CMD, format!("unknown preset `{id}` (reel, youtube, youtube4k, musicVideo, empty)")))?;
    let name: String = str_p(p, "name").unwrap_or("").trim().chars().filter(|c| !c.is_control()).take(MAX_NAME).collect();
    let name = if name.is_empty() { "Untitled".to_string() } else { name };
    s.execute("file.newProject", json!({ "name": name }))?;
    let mut sequence = Value::Null;
    if t.has_sequence() {
        let r = s.execute("file.newSequence", json!({"name": t.sequence, "width": t.size.0, "height": t.size.1, "fps": t.fps, "video": 3, "audio": 3}))?;
        sequence = r.get("sequence").cloned().unwrap_or(Value::Null);
        // a fresh project: nothing to undo back to, nothing to save yet
        s.history = Default::default();
        s.history.limit = 200;
        s.saved_revision = s.revision;
    }
    Ok(json!({"name": name, "preset": t.id, "sequence": sequence}))
}

pub fn commands() -> Vec<CommandSpec> {
    vec![CommandSpec {
        id: "file.newProjectPreset",
        label: "New Project from Preset",
        menu: &[],
        shortcut: None,
        params: r#"{"name":str="Untitled","preset":"reel|youtube|youtube4k|musicVideo|empty"="empty"}"#,
        enabled: |_| Ok(()),
        run: new_from_template,
        journal: true,
    }]
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_template_starts_a_clean_project_with_its_sequence() {
        for t in TEMPLATES {
            let mut s = Session::default();
            s.execute("file.openDemoProject", json!({})).unwrap();
            let r = s.execute("file.newProjectPreset", json!({"name": "  Mein Reel  ", "preset": t.id})).unwrap();
            assert_eq!(s.project.name, "Mein Reel");
            assert!(!s.is_dirty(), "{}: a new project has nothing to save", t.id);
            assert!(s.path.is_none());
            let _ = s.execute("edit.undo", json!({}));
            assert_eq!(s.project.sequences().count(), usize::from(t.has_sequence()), "{}: undo must not take the sequence away", t.id);
            if t.has_sequence() {
                let q = s.active_sequence().expect("open sequence");
                assert_eq!((q.settings.width, q.settings.height), t.size, "{}", t.id);
                assert!((q.settings.frame_rate.as_f64() - f64::from(t.fps)).abs() < 1e-6, "{}", t.id);
                assert_eq!(s.project.sequences().count(), 1);
                assert!(r["sequence"].is_u64());
            } else {
                assert_eq!(s.project.sequences().count(), 0);
                assert!(r["sequence"].is_null());
            }
        }
    }

    #[test]
    fn bad_input_is_refused_or_cleaned() {
        let mut s = Session::default();
        assert!(s.execute("file.newProjectPreset", json!({"preset": "nope"})).is_err());
        s.execute("file.newProjectPreset", json!({"name": "\u{0}\n\t", "preset": "reel"})).unwrap();
        assert_eq!(s.project.name, "Untitled");
        s.execute("file.newProjectPreset", json!({"name": "x".repeat(5000)})).unwrap();
        assert_eq!(s.project.name.chars().count(), MAX_NAME);
        assert_eq!(s.project.sequences().count(), 0);
    }
}
