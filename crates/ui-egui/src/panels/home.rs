//! Home (FilmCraft Plus): the start screen. It offers a new project from a template (vertical
//! Reel, YouTube, music video, empty), Open Project…, and the recent projects with when each was
//! last changed. The app starts here (Settings ▸ General ▸ At Startup ▸ Show Home), the header's
//! Home button and File ▸ Close Project come back to it, and it gives way to Edit as soon as a
//! project is open.
//!
//! Automation ids: `home.name`, `home.new.<template>`, `home.open`, `home.recent.<i>`,
//! `home.recent.<i>.remove`, `home.demo`.

use egui::{Align2, Color32, Rect, Sense, Stroke, StrokeKind, pos2, vec2};
use filmcraft_engine::plus_project::{TEMPLATES, Template};
use serde::{Deserialize, Serialize};
use serde_json::json;

use crate::FilmcraftApp;
use crate::icons::{self, Icon};
use crate::state::Mode;
use crate::theme::Tokens;

/// Home's state.
#[derive(Clone, Debug, Default, Serialize, Deserialize)]
#[serde(default, rename_all = "camelCase")]
pub struct HomeUi {
    /// Name typed for the next new project (empty = "Untitled").
    pub name: String,
    /// The project Home was last drawn over and in which frame, to notice a project opening.
    #[serde(skip)]
    seen: Option<(u64, u64)>,
    #[serde(skip)]
    files: Files,
}

/// What is known about the recent project files (refreshed every few seconds, not every frame).
#[derive(Clone, Debug, Default)]
struct Files {
    /// `ctx` time of the last look.
    at: f64,
    paths: Vec<String>,
    /// Per path: `None` = the file is gone; `Some(age)` = seconds since it was last changed
    /// (`None` inside when the system doesn't say).
    info: Vec<Option<Option<u64>>>,
}

/// Recent projects listed.
const ROWS: usize = 10;
const ROW_H: f32 = 34.0;
const CARD: egui::Vec2 = vec2(196.0, 136.0);
const GAP: f32 = 14.0;
const PAD: f32 = 32.0;
/// Seconds between looks at the recent files.
const REFRESH_S: f64 = 4.0;
/// Longest project name typed (characters).
const MAX_NAME: usize = 120;

/// "5 min ago" for an age in seconds.
fn ago(secs: u64) -> String {
    const DAY: u64 = 86_400;
    match secs {
        0..60 => tl!("just now").to_string(),
        60..3600 => tlf!("{n} min ago", n = secs / 60),
        3600..DAY => tlf!("{n} h ago", n = secs / 3600),
        _ => match secs / DAY {
            1 => tl!("1 day ago").to_string(),
            d @ 2..60 => tlf!("{n} days ago", n = d),
            d @ 60..730 => tlf!("{n} months ago", n = d / 30),
            d => tlf!("{n} years ago", n = d / 365),
        },
    }
}

/// A project file's name without the `.fcproj` ending.
fn title(path: &str) -> String {
    let name = crate::menus::recent_name(path);
    match name.rsplit_once('.') {
        Some((stem, ext)) if !stem.is_empty() && ext.eq_ignore_ascii_case("fcproj") => stem.to_string(),
        _ => name,
    }
}

/// Whether a project is open (saved somewhere, or with a sequence in it) rather than the blank
/// one the app starts with and Close Project leaves.
fn project_open(app: &FilmcraftApp) -> bool {
    app.session.path.is_some() || app.session.project.sequences().next().is_some()
}

fn refresh(files: &mut Files, paths: &[String], now: f64) {
    if files.paths == paths && (now - files.at).abs() < REFRESH_S && files.info.len() == paths.len() {
        return;
    }
    files.at = now;
    files.paths = paths.to_vec();
    files.info = paths.iter().map(|p| std::fs::metadata(p).ok().map(|m| m.modified().ok().and_then(|t| t.elapsed().ok()).map(|d| d.as_secs()))).collect();
}

/// A painted button; returns its response.
fn button(app: &mut FilmcraftApp, ui: &mut egui::Ui, r: Rect, id: &str, label: &str, icon: Option<Icon>, primary: bool) -> egui::Response {
    let t = app.tokens;
    let resp = ui.interact(r, egui::Id::unique(("home-btn", id)), Sense::click());
    app.auto.add(id, r, label);
    let (bg, fg) = if primary {
        (if resp.hovered() { t.accent_hover } else { t.accent }, Color32::WHITE)
    } else {
        (if resp.hovered() { t.hover } else { t.field_bg }, t.text)
    };
    ui.painter().rect_filled(r, 8.0, bg);
    if !primary {
        ui.painter().rect_stroke(r, 8.0, Stroke::new(1.0, t.field_border), StrokeKind::Inside);
    }
    let mut x = r.min.x + 14.0;
    if let Some(icon) = icon {
        icons::paint(ui.painter(), Rect::from_center_size(pos2(x + 8.0, r.center().y), vec2(16.0, 16.0)), icon, fg);
        x += 24.0;
    }
    ui.painter().with_clip_rect(r).text(pos2(x, r.center().y), Align2::LEFT_CENTER, label, Tokens::ui(13.0), fg);
    resp
}

/// Width a [`button`] with this label needs.
fn button_width(ui: &egui::Ui, label: &str, icon: bool) -> f32 {
    ui.painter().layout_no_wrap(label.to_string(), Tokens::ui(13.0), Color32::WHITE).size().x + if icon { 52.0 } else { 28.0 }
}

pub fn show(app: &mut FilmcraftApp, ui: &mut egui::Ui, rect: Rect) {
    let t = app.tokens;
    ui.painter().rect_filled(rect, t.radius, t.panel_bg);
    let ctx = ui.ctx().clone();
    // A project opened while Home was showing (a recent one, the Open dialog, recovery, a file
    // dropped on the window): go and edit it. Closing a project keeps Home.
    let (frame, epoch) = (ctx.cumulative_frame_nr(), app.session.document_epoch);
    if let Some((was, at)) = app.ui.home.seen
        && frame.saturating_sub(at) <= 1
        && was != epoch
        && project_open(app)
    {
        app.ui.home.seen = None;
        app.ui.mode = Mode::Edit;
        ctx.request_repaint();
        return;
    }
    app.ui.home.seen = Some((epoch, frame));

    let paths: Vec<String> = app.session.prefs.general.recent_projects.iter().take(ROWS).cloned().collect();
    refresh(&mut app.ui.home.files, &paths, ctx.input(|i| i.time));
    let info = app.ui.home.files.info.clone();

    let inner = rect.shrink2(vec2(1.0, 4.0));
    let mut child = ui.new_child(egui::UiBuilder::new().max_rect(inner).id_salt("home"));
    child.set_clip_rect(inner);
    egui::ScrollArea::vertical().auto_shrink([false, false]).show(&mut child, |ui| {
        let w = ui.available_width().max(CARD.x + 2.0 * PAD);
        let per_row = (((w - 2.0 * PAD + GAP) / (CARD.x + GAP)).floor() as usize).clamp(1, TEMPLATES.len());
        let card_rows = TEMPLATES.len().div_ceil(per_row);
        let cards_h = card_rows as f32 * (CARD.y + GAP);
        let list_h = if paths.is_empty() { 44.0 } else { 26.0 + paths.len() as f32 * ROW_H };
        let height = 96.0 + 76.0 + cards_h + 30.0 + 44.0 + list_h + 76.0;
        let (area, _) = ui.allocate_exact_size(vec2(w, height), Sense::hover());
        let x0 = area.min.x + PAD;
        let mut y = area.min.y + 40.0;

        ui.painter().text(pos2(x0, y), Align2::LEFT_CENTER, tl!("Home"), Tokens::semibold(22.0), t.text);
        ui.painter().text(pos2(x0, y + 28.0), Align2::LEFT_CENTER, tl!("Start a new project, or carry on with a recent one."), Tokens::ui(13.0), t.text_dim);
        y += 72.0;

        // ---- New Project: a name and the templates
        ui.painter().text(pos2(x0, y), Align2::LEFT_CENTER, tl!("New Project"), Tokens::semibold(15.0), t.text);
        y += 22.0;
        let label = ui.painter().text(pos2(x0, y + 15.0), Align2::LEFT_CENTER, tl!("Name"), Tokens::ui(13.0), t.text_dim);
        let field = Rect::from_min_size(pos2(label.max.x + 12.0, y), vec2(320.0_f32.min((area.max.x - PAD - label.max.x - 12.0).max(80.0)), 30.0));
        let resp = ui.put(
            field,
            egui::TextEdit::singleline(&mut app.ui.home.name)
                .id(egui::Id::unique("home-name"))
                .hint_text(tl!("Untitled"))
                .char_limit(MAX_NAME)
                .font(Tokens::ui(13.0))
                .align(Align2::LEFT_CENTER)
                .desired_width(field.width()),
        );
        app.auto.add_response("home.name", &resp, "Project name");
        y += 46.0;
        let mut start: Option<&Template> = None;
        for (i, tpl) in TEMPLATES.iter().enumerate() {
            let r = Rect::from_min_size(pos2(x0 + (i % per_row) as f32 * (CARD.x + GAP), y + (i / per_row) as f32 * (CARD.y + GAP)), CARD);
            if card(app, ui, r, tpl).clicked() {
                start = Some(tpl);
            }
        }
        y += cards_h + 30.0;

        // ---- Recent, with Open Project… beside the heading
        let list_w = 760.0_f32.min(area.max.x - PAD - x0).max(CARD.x);
        ui.painter().text(pos2(x0, y), Align2::LEFT_CENTER, tl!("Recent"), Tokens::semibold(15.0), t.text);
        let open_label = tl!("Open Project…");
        let bw = button_width(ui, open_label, true);
        let open_r = Rect::from_min_size(pos2((x0 + list_w - bw).max(x0 + 90.0), y - 16.0), vec2(bw, 32.0));
        let open_clicked = button(app, ui, open_r, "home.open", open_label, Some(Icon::Folder), false).clicked();
        y += 28.0;
        let (mut open_path, mut remove) = (None, None);
        if paths.is_empty() {
            ui.painter().text(
                pos2(x0, y + 14.0),
                Align2::LEFT_CENTER,
                tl!("No projects yet. The projects you save appear here."),
                Tokens::ui(13.0),
                t.text_dim,
            );
        } else {
            ui.painter().text(pos2(x0 + 32.0, y + 8.0), Align2::LEFT_CENTER, tl!("Name"), Tokens::ui(11.5), t.text_faint);
            ui.painter().text(pos2(x0 + list_w - 40.0, y + 8.0), Align2::RIGHT_CENTER, tl!("Changed"), Tokens::ui(11.5), t.text_faint);
            ui.painter().line_segment([pos2(x0, y + 20.0), pos2(x0 + list_w, y + 20.0)], Stroke::new(1.0, t.separator));
            y += 26.0;
            for (i, path) in paths.iter().enumerate() {
                let row = Rect::from_min_size(pos2(x0, y + i as f32 * ROW_H), vec2(list_w, ROW_H));
                match recent_row(app, ui, row, i, path, info.get(i).copied().flatten()) {
                    Some(RowClick::Open) => open_path = Some(path.clone()),
                    Some(RowClick::Remove) => remove = Some(path.clone()),
                    None => {}
                }
            }
        }
        y += list_h - if paths.is_empty() { 0.0 } else { 26.0 } + 24.0;

        // ---- the demo project stays one click away (not over unsaved work)
        let mut demo = false;
        if !app.session.is_dirty() {
            let label = tl!("Open the Demo Project");
            let r = Rect::from_min_size(pos2(x0, y), vec2(button_width(ui, label, true), 32.0));
            demo = button(app, ui, r, "home.demo", label, Some(Icon::Sequence), false).clicked();
        }

        if let Some(tpl) = start {
            new_project(app, &ctx, tpl);
        } else if open_clicked {
            let _ = crate::menus::invoke(app, &ctx, "file.open", json!({}));
        } else if let Some(path) = open_path {
            // with unsaved changes this asks first; the project opens after the answer
            if crate::menus::invoke(app, &ctx, "file.open", json!({"path": path})).is_ok() && app.ui.clip_dialog.is_none() && project_open(app) {
                app.ui.mode = Mode::Edit;
            }
        } else if let Some(path) = remove {
            let mut next = app.session.prefs.clone();
            next.general.recent_projects.retain(|p| p != &path);
            if let Err(e) = app.session.set_prefs(next) {
                app.ui.status = e.to_string();
            }
        } else if demo && crate::menus::invoke(app, &ctx, "file.openDemoProject", json!({})).is_ok() {
            app.ui.mode = Mode::Edit;
        }
    });
}

/// Start a project from `tpl`. With unsaved changes, Save / Don't Save / Cancel is asked first.
fn new_project(app: &mut FilmcraftApp, ctx: &egui::Context, tpl: &Template) {
    let params = json!({"name": app.ui.home.name.trim(), "preset": tpl.id});
    if crate::panels::clip_dialogs::ask_before_new(app, &params) {
        return;
    }
    if crate::menus::invoke(app, ctx, "file.newProjectPreset", params).is_ok() {
        app.ui.home.name.clear();
        app.ui.mode = Mode::Edit;
    }
}

/// One template card: the frame's shape, the name, size and frame rate.
fn card(app: &mut FilmcraftApp, ui: &mut egui::Ui, r: Rect, tpl: &Template) -> egui::Response {
    let t = app.tokens;
    let label = crate::i18n::t(tpl.label);
    let resp = ui.interact(r, egui::Id::unique(("home-new", tpl.id)), Sense::click());
    app.auto.add(&format!("home.new.{}", tpl.id), r, label);
    let p = ui.painter().with_clip_rect(r);
    p.rect_filled(r, 8.0, if resp.hovered() { t.hover } else { t.field_bg });
    p.rect_stroke(r, 8.0, Stroke::new(1.0, if resp.hovered() { t.accent } else { t.field_border }), StrokeKind::Inside);
    // the frame, drawn to scale in a 72×60 box
    let boxr = Rect::from_min_size(r.min + vec2(16.0, 16.0), vec2(72.0, 60.0));
    let accent = Color32::from_rgb(140, 150, 255);
    if tpl.has_sequence() {
        let (fw, fh) = (tpl.size.0 as f32, tpl.size.1 as f32);
        let k = (boxr.width() / fw).min(boxr.height() / fh);
        let frame = Rect::from_min_size(pos2(boxr.min.x, boxr.center().y - fh * k / 2.0), vec2(fw * k, fh * k));
        p.rect_filled(frame, 3.0, accent.gamma_multiply(0.22));
        p.rect_stroke(frame, 3.0, Stroke::new(1.5, accent), StrokeKind::Inside);
        // a play mark in the middle
        let (c, s) = (frame.center(), (frame.width().min(frame.height()) * 0.2).clamp(3.0, 9.0));
        p.add(egui::Shape::convex_polygon(vec![pos2(c.x - s * 0.7, c.y - s), pos2(c.x + s, c.y), pos2(c.x - s * 0.7, c.y + s)], accent, Stroke::NONE));
    } else {
        icons::paint(&p, Rect::from_min_size(boxr.min + vec2(0.0, 8.0), vec2(44.0, 44.0)), Icon::Plus, accent);
    }
    p.text(r.min + vec2(16.0, 98.0), Align2::LEFT_CENTER, label, Tokens::semibold(14.0), t.text);
    let detail = if tpl.has_sequence() { format!("{} × {} · {} fps", tpl.size.0, tpl.size.1, tpl.fps) } else { tl!("No sequence yet").to_string() };
    p.text(r.min + vec2(16.0, 118.0), Align2::LEFT_CENTER, detail, Tokens::ui(11.5), t.text_dim);
    resp
}

enum RowClick {
    Open,
    Remove,
}

/// One recent project: name, folder, when it last changed, and (hovered) a button that takes it
/// off the list. `age`: see [`Files::info`]; `None` = the file is gone.
fn recent_row(app: &mut FilmcraftApp, ui: &mut egui::Ui, row: Rect, i: usize, path: &str, age: Option<Option<u64>>) -> Option<RowClick> {
    let t = app.tokens;
    let name = title(path);
    let missing = age.is_none();
    let resp = ui.interact(row, egui::Id::unique(("home-recent", i)), Sense::click()).on_hover_text(path);
    app.auto.add(&format!("home.recent.{i}"), row, &name);
    let x_r = Rect::from_center_size(pos2(row.max.x - 16.0, row.center().y), vec2(22.0, 22.0));
    let hovered = resp.hovered() || ui.rect_contains_pointer(row);
    if hovered {
        ui.painter().rect_filled(row, 6.0, t.hover);
    }
    let mut click = resp.clicked().then_some(RowClick::Open);
    if hovered {
        let x = ui.interact(x_r, egui::Id::unique(("home-recent-x", i)), Sense::click()).on_hover_text(tl!("Remove from this list"));
        app.auto.add(&format!("home.recent.{i}.remove"), x_r, "Remove from this list");
        if x.hovered() {
            ui.painter().rect_filled(x_r, 4.0, t.pressed);
        }
        icons::paint(ui.painter(), x_r.shrink(5.0), Icon::Close, if x.hovered() { t.text } else { t.text_dim });
        if x.clicked() {
            click = Some(RowClick::Remove);
        }
    }
    let when = match age {
        None => tl!("File not found").to_string(),
        Some(None) => String::new(),
        Some(Some(secs)) => ago(secs),
    };
    let when_r = ui.painter().text(pos2(row.max.x - 40.0, row.center().y), Align2::RIGHT_CENTER, when, Tokens::ui(12.0), t.text_dim);
    let text = Rect::from_min_max(row.min, pos2((when_r.min.x - 12.0).max(row.min.x), row.max.y));
    let p = ui.painter().with_clip_rect(text);
    let col = if missing { t.text_dim } else { t.text };
    icons::paint(&p, Rect::from_center_size(pos2(row.min.x + 16.0, row.center().y), vec2(15.0, 15.0)), Icon::Sequence, t.text_dim);
    let name_r = p.text(pos2(row.min.x + 32.0, row.center().y), Align2::LEFT_CENTER, &name, Tokens::ui(13.5), col);
    if let Some(folder) = std::path::Path::new(path).parent() {
        p.text(pos2(name_r.max.x + 12.0, row.center().y), Align2::LEFT_CENTER, folder.to_string_lossy(), Tokens::ui(11.5), t.text_faint);
    }
    click
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ages_read_naturally_and_never_overflow() {
        assert_eq!(ago(0), "just now");
        assert_eq!(ago(59), "just now");
        assert_eq!(ago(60), "1 min ago");
        assert_eq!(ago(3599), "59 min ago");
        assert_eq!(ago(3600), "1 h ago");
        assert_eq!(ago(86_399), "23 h ago");
        assert_eq!(ago(86_400), "1 day ago");
        assert_eq!(ago(2 * 86_400), "2 days ago");
        assert_eq!(ago(59 * 86_400), "59 days ago");
        assert_eq!(ago(60 * 86_400), "2 months ago");
        assert_eq!(ago(729 * 86_400), "24 months ago");
        assert_eq!(ago(730 * 86_400), "2 years ago");
        assert!(ago(u64::MAX).ends_with("years ago"));
    }

    #[test]
    fn titles_drop_the_project_ending_only() {
        assert_eq!(title("/home/a/Mein Reel.fcproj"), "Mein Reel");
        assert_eq!(title("C:/x/Video.FCPROJ"), "Video");
        assert_eq!(title("/x/track.v2.fcproj"), "track.v2");
        assert_eq!(title("/x/notes.txt"), "notes.txt");
        assert_eq!(title("/x/.fcproj"), ".fcproj");
        assert_eq!(title(""), "");
    }

    #[test]
    fn missing_files_are_marked_and_the_look_is_cached() {
        let dir = std::env::temp_dir().join(format!("filmcraft-home-{}", std::process::id()));
        std::fs::create_dir_all(&dir).expect("dir");
        let there = dir.join("a.fcproj");
        std::fs::write(&there, b"x").expect("write");
        let paths = vec![there.to_string_lossy().to_string(), dir.join("gone.fcproj").to_string_lossy().to_string()];
        let mut f = Files::default();
        refresh(&mut f, &paths, 10.0);
        assert!(matches!(f.info[0], Some(Some(s)) if s < 3600), "{:?}", f.info);
        assert_eq!(f.info[1], None);
        // within the refresh time the file is not looked at again…
        std::fs::remove_file(&there).expect("remove");
        refresh(&mut f, &paths, 11.0);
        assert!(f.info[0].is_some());
        // …after it, or when the list changes, it is
        refresh(&mut f, &paths, 10.0 + REFRESH_S + 1.0);
        assert_eq!(f.info[0], None);
        refresh(&mut f, &paths[..1], 10.0 + REFRESH_S + 1.5);
        assert_eq!(f.info.len(), 1);
        let _ = std::fs::remove_dir_all(&dir);
    }
}
