//! Headless UI test of Home (FilmCraft Plus), the start screen: a new project from a template,
//! the recent projects (open, remove from the list), the unsaved-changes question, and Close
//! Project coming back to it.
//!
//! Set `FILMCRAFT_UI_SNAPSHOT_DIR=<dir>` to also render the window offscreen with wgpu and write
//! `home-*.png` there; without it no GPU is needed.

use std::sync::mpsc::{Sender, channel};

use egui_kittest::Harness;
use filmcraft_engine::Session;
use filmcraft_ui_egui::FilmcraftApp;
use filmcraft_ui_egui::control::ControlRequest;
use filmcraft_ui_egui::state::Mode;
use serde_json::{Value, json};

struct Driver {
    harness: Harness<'static, FilmcraftApp>,
    tx: Sender<ControlRequest>,
    snapshots: Option<std::path::PathBuf>,
}

impl Driver {
    /// The app as it starts: an empty project, Home showing.
    fn home(size: egui::Vec2) -> Self {
        let (tx, rx) = channel();
        let mut app = FilmcraftApp::new(Session::default()).with_control(rx);
        app.ui.mode = Mode::Home;
        let snapshots = std::env::var_os("FILMCRAFT_UI_SNAPSHOT_DIR").map(std::path::PathBuf::from);
        let mut b = Harness::builder().with_size(size).with_max_steps(10_000);
        if snapshots.is_some() {
            b = b.wgpu();
        }
        let harness = b.build_eframe(move |_cc| app);
        let mut d = Driver { harness, tx, snapshots };
        d.frames(4);
        d
    }

    fn app(&self) -> &FilmcraftApp {
        self.harness.state()
    }

    fn frames(&mut self, n: usize) {
        for _ in 0..n {
            let ctx = self.harness.ctx.clone();
            let mut raw = std::mem::take(self.harness.input_mut());
            eframe::App::raw_input_hook(self.harness.state_mut(), &ctx, &mut raw);
            *self.harness.input_mut() = raw;
            self.harness.step();
        }
    }

    fn ok(&mut self, method: &str, params: Value) -> Value {
        let (req, reply) = ControlRequest::new(method, params.clone());
        self.tx.send(req).unwrap();
        for _ in 0..600 {
            self.frames(1);
            if let Ok(v) = reply.try_recv() {
                assert_eq!(v["ok"], json!(true), "{method} {params} failed: {v}");
                return v["result"].clone();
            }
        }
        panic!("no reply to {method} {params}");
    }

    fn click(&mut self, id: &str) {
        self.ok("ui.click", json!({"id": id}));
        self.frames(4);
    }

    fn ids(&mut self, prefix: &str) -> Vec<String> {
        let v = self.ok("ui.elements", json!({"prefix": prefix}));
        v.as_array().unwrap().iter().filter_map(|e| e["id"].as_str().map(str::to_string)).collect()
    }

    fn snapshot(&mut self, name: &str) {
        let Some(dir) = self.snapshots.clone() else { return };
        self.frames(2);
        match self.harness.render() {
            Ok(img) => {
                std::fs::create_dir_all(&dir).unwrap();
                img.save(dir.join(format!("home-{name}.png"))).unwrap();
            }
            Err(e) => eprintln!("snapshot {name} skipped: {e}"),
        }
    }
}

fn temp_dir(tag: &str) -> std::path::PathBuf {
    let dir = std::env::temp_dir().join(format!("filmcraft-home-ui-{tag}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    dir
}

#[test]
fn home_starts_projects_from_templates() {
    let mut d = Driver::home(egui::vec2(1600.0, 980.0));
    let cards = d.ids("home.new.");
    for id in ["home.new.reel", "home.new.youtube", "home.new.youtube4k", "home.new.musicVideo", "home.new.empty"] {
        assert!(cards.iter().any(|c| c == id), "{id} in {cards:?}");
    }
    assert_eq!(d.ids("home.open"), ["home.open"]);
    assert_eq!(d.ids("home.demo"), ["home.demo"]);
    assert!(d.ids("home.recent.").is_empty(), "no recent projects yet");
    d.snapshot("empty");
    // a name, then the vertical template: a clean project with a 1080×1920 sequence, in Edit
    d.click("home.name");
    d.ok("ui.type", json!({"text": "Mein Reel"}));
    d.frames(3);
    assert_eq!(d.app().ui.home.name, "Mein Reel");
    d.click("home.new.reel");
    assert_eq!(d.app().ui.mode, Mode::Edit);
    let s = &d.app().session;
    assert_eq!(s.project.name, "Mein Reel");
    let q = s.active_sequence().expect("sequence");
    assert_eq!((q.settings.width, q.settings.height), (1080, 1920));
    assert!(!s.is_dirty());
    assert!(d.app().ui.home.name.is_empty(), "the name field is ready for the next project");
    // Home again (header button), an edit, then another template: the question comes first
    d.click("header.home");
    assert_eq!(d.app().ui.mode, Mode::Home);
    d.ok("engine.execute", json!({"command": "file.newSequence", "params": {"name": "Extra"}}));
    d.frames(3);
    // …a command that gives something to edit leaves Home
    assert_eq!(d.app().ui.mode, Mode::Edit);
    assert!(d.app().session.is_dirty());
    d.click("header.home");
    assert!(d.ids("home.demo").is_empty(), "the demo project is not offered over unsaved work");
    d.click("home.new.youtube");
    assert_eq!(d.app().ui.clip_dialog.as_ref().map(|c| c.command.as_str()), Some("file.newProjectPreset"));
    assert_eq!(d.app().session.project.name, "Mein Reel", "nothing replaced yet");
    d.snapshot("ask");
    d.click("newProject.cancel");
    assert!(d.app().ui.clip_dialog.is_none());
    assert_eq!(d.app().session.project.sequences().count(), 2, "Cancel keeps the project");
    assert_eq!(d.app().ui.mode, Mode::Home);
    d.click("home.new.youtube");
    d.click("newProject.dontSave");
    assert_eq!(d.app().ui.mode, Mode::Edit);
    let s = &d.app().session;
    assert_eq!(s.project.name, "Untitled");
    let q = s.active_sequence().expect("sequence");
    assert_eq!((q.settings.width, q.settings.height), (1920, 1080));
    assert_eq!(s.project.sequences().count(), 1);
}

#[test]
fn home_lists_recent_projects_and_close_project_comes_back() {
    let dir = temp_dir("recent");
    let mut d = Driver::home(egui::vec2(1280.0, 800.0));
    // two saved projects
    let (a, b) = (dir.join("Erstes Video.fcproj"), dir.join("Zweites Video.fcproj"));
    for (path, template) in [(&a, "youtube"), (&b, "reel")] {
        d.ok("engine.execute", json!({"command": "file.newProjectPreset", "params": {"preset": template}}));
        d.ok("engine.execute", json!({"command": "file.saveAs", "params": {"path": path.to_string_lossy()}}));
    }
    // Close Project from the menu shows Home with both, newest first
    d.ok("ui.menu.invoke", json!({"id": "file.closeProject"}));
    d.frames(4);
    assert_eq!(d.app().ui.mode, Mode::Home);
    assert!(d.app().session.path.is_none());
    let rows = d.ids("home.recent.");
    assert!(rows.contains(&"home.recent.0".to_string()) && rows.contains(&"home.recent.1".to_string()), "{rows:?}");
    let labels = d.ok("ui.elements", json!({"prefix": "home.recent."}));
    assert_eq!(labels[0]["label"], json!("Zweites Video"), "{labels}");
    d.snapshot("recent");
    // a click opens the older one and goes to Edit
    d.click("home.recent.1");
    assert_eq!(d.app().ui.mode, Mode::Edit);
    assert_eq!(d.app().session.path.as_deref(), Some(a.to_string_lossy().as_ref()));
    assert_eq!(d.app().session.active_sequence().map(|q| q.settings.height), Some(1080));
    // a file that is gone is marked, opening it fails without leaving Home, and × takes it off
    d.ok("ui.menu.invoke", json!({"id": "file.closeProject"}));
    d.frames(4);
    std::fs::remove_file(&b).unwrap();
    let at = d.app().session.prefs.general.recent_projects.iter().position(|p| p == &b.to_string_lossy()).expect("listed");
    d.click(&format!("home.recent.{at}"));
    assert_eq!(d.app().ui.mode, Mode::Home);
    assert!(d.app().session.path.is_none());
    d.ok("ui.move", json!({"id": format!("home.recent.{at}")}));
    d.frames(3);
    d.click(&format!("home.recent.{at}.remove"));
    let recent = d.app().session.prefs.general.recent_projects.clone();
    assert!(!recent.iter().any(|p| p == &b.to_string_lossy()), "{recent:?}");
    assert!(recent.iter().any(|p| p == &a.to_string_lossy()));
    // a narrow window still lays everything out (cards wrap, the page scrolls)
    let mut small = Driver::home(egui::vec2(420.0, 360.0));
    assert!(!small.ids("home.new.reel").is_empty());
    small.snapshot("small");
    let _ = std::fs::remove_dir_all(&dir);
}
