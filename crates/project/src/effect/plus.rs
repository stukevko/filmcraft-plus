//! FilmCraft Plus: an extra effect folder with three easy-to-use "one effect, many presets" tools.
//!
//! - **Organic Shake** (`organic_shake`): natural, multi-octave camera shake with X / Y / Z
//!   (zoom) / rotation channels, shutter-accurate motion blur, automatic edge cover and an
//!   impact decay, with ready-made presets (Handheld, Walking, Earthquake, Impact Hit…).
//! - **Flicker Stack** (`flicker_stack`): random, rhythmic "twitches" of blur, colour, light,
//!   scale, slide, colour split and time, all driven by one Amount slider and a preset.
//! - **Look Builder** (`look_builder`): a stack of finishing tools in the order a colourist
//!   would use them (Subject ▸ Color ▸ Matte ▸ Lens ▸ Film), with one-click looks and a
//!   Strength slider.
//!
//! All three are original FilmCraft work: names, parameters, preset values and algorithms were
//! written for this project. They are inspired by the *kind* of tool common in editing suites,
//! but contain no third-party code, presets, LUTs or names. Pixel implementations live in
//! `filmcraft_render::vfx::plus`.

use super::{EffectDef, ParamDef, b, ch, col, fs, grp, video};

/// Top-level Effects panel folder of the FilmCraft Plus effects.
pub const PLUS_TOP: &str = "FilmCraft Plus";
const SHAKE: &[&str] = &[PLUS_TOP, "Shake & Motion"];
const FLICKER: &[&str] = &[PLUS_TOP, "Flicker & Glitch"];
const LOOKS: &[&str] = &[PLUS_TOP, "Looks"];

/// Organic Shake presets (index 0 = Custom: the sliders below are used as they are).
pub const SHAKE_PRESETS: &[&str] = &["Custom", "Handheld", "Walking", "Running", "Car Ride", "Earthquake", "Impact Hit", "Drone Drift", "Heartbeat"];
/// Flicker Stack presets (index 0 = Custom).
pub const FLICKER_PRESETS: &[&str] = &["Custom", "Subtle", "Music Video", "Glitch Hit", "Horror", "Strobe", "Dreamy"];
/// Look Builder looks (index 0 = Custom).
pub const LOOK_PRESETS: &[&str] = &[
    "Custom",
    "Teal & Orange",
    "Warm Film",
    "Cold Thriller",
    "Bleach Bypass",
    "Vintage 70s",
    "Pastel Dream",
    "Noir",
    "Neon Night",
    "Golden Hour",
    "Green Code",
    "Faded Matte",
    "Cinema Contrast",
];
/// Motion-blur quality: sub-frame samples per frame.
pub const BLUR_QUALITY: &[&str] = &["Draft", "Normal", "High"];

fn pct(id: &'static str, label: &'static str, def: f64) -> ParamDef {
    fs(id, label, def, (0.0, 100.0), (0.0, 100.0), "%", 1)
}
fn seed() -> ParamDef {
    fs("seed", "Random Seed", 0.0, (0.0, 10000.0), (0.0, 1000.0), "", 0)
}

fn organic_shake() -> EffectDef {
    const G: &str = "Custom Shake";
    video(
        "organic_shake",
        "Organic Shake",
        SHAKE,
        vec![
            ch("preset", "Preset", SHAKE_PRESETS, 1),
            fs("intensity", "Intensity", 100.0, (0.0, 1000.0), (0.0, 300.0), "%", 1),
            fs("speed", "Speed", 100.0, (1.0, 1000.0), (10.0, 300.0), "%", 1),
            grp(fs("amp_x", "Horizontal", 8.0, (0.0, 2000.0), (0.0, 200.0), "", 1), G),
            grp(fs("amp_y", "Vertical", 6.0, (0.0, 2000.0), (0.0, 200.0), "", 1), G),
            grp(fs("amp_z", "Zoom (Z Shake)", 1.0, (0.0, 100.0), (0.0, 20.0), "%", 2), G),
            grp(fs("amp_rot", "Rotation", 0.6, (0.0, 90.0), (0.0, 10.0), "°", 2), G),
            grp(fs("frequency", "Frequency", 1.2, (0.05, 60.0), (0.05, 20.0), "Hz", 2), G),
            grp(fs("roughness", "Roughness", 3.0, (1.0, 6.0), (1.0, 6.0), "", 1), G),
            grp(fs("decay", "Decay Time", 0.0, (0.0, 30.0), (0.0, 5.0), "s", 2), G),
            pct("motion_blur", "Motion Blur", 50.0),
            ch("blur_quality", "Motion Blur Quality", BLUR_QUALITY, 1),
            b("auto_scale", "Hide Edges (Auto Scale)", true),
            seed(),
        ],
    )
}

fn flicker_stack() -> EffectDef {
    const G: &str = "Custom Flicker";
    const A: &str = "Flicker Amounts";
    video(
        "flicker_stack",
        "Flicker Stack",
        FLICKER,
        vec![
            ch("preset", "Preset", FLICKER_PRESETS, 2),
            fs("amount", "Amount", 100.0, (0.0, 1000.0), (0.0, 200.0), "%", 1),
            grp(fs("rate", "Flickers per Second", 8.0, (0.25, 60.0), (0.25, 30.0), "", 2), G),
            grp(pct("density", "Density", 45.0), G),
            grp(b("do_blur", "Blur", true), G),
            grp(b("do_color", "Color", true), G),
            grp(b("do_light", "Light", true), G),
            grp(b("do_scale", "Scale", true), G),
            grp(b("do_slide", "Slide", true), G),
            grp(b("do_split", "Color Split", true), G),
            grp(b("do_time", "Time", false), G),
            grp(fs("blur_amt", "Blur Amount", 25.0, (0.0, 500.0), (0.0, 100.0), "", 1), A),
            grp(col("color", "Flicker Color", [0.15, 0.85, 1.0, 1.0]), A),
            grp(b("random_color", "Random Color", true), A),
            grp(pct("color_amt", "Color Amount", 45.0), A),
            grp(fs("light_amt", "Light Amount", 80.0, (0.0, 500.0), (0.0, 200.0), "%", 1), A),
            grp(fs("scale_amt", "Scale Amount", 12.0, (0.0, 200.0), (0.0, 50.0), "%", 1), A),
            grp(fs("slide_amt", "Slide Amount", 40.0, (0.0, 2000.0), (0.0, 300.0), "", 1), A),
            grp(fs("split_amt", "Color Split Amount", 14.0, (0.0, 500.0), (0.0, 100.0), "", 1), A),
            grp(fs("time_amt", "Time Jump (frames)", 4.0, (1.0, 60.0), (1.0, 15.0), "", 0), A),
            seed(),
        ],
    )
}

fn look_builder() -> EffectDef {
    const SUBJ: &str = "Subject";
    const COLOR: &str = "Color";
    const MATTE: &str = "Matte";
    const LENS: &str = "Lens";
    const FILM: &str = "Film";
    video(
        "look_builder",
        "Look Builder",
        LOOKS,
        vec![
            ch("look", "Look", LOOK_PRESETS, 1),
            pct("strength", "Strength", 100.0),
            grp(pct("skin_smooth", "Skin Smoothing", 0.0), SUBJ),
            grp(fs("exposure", "Exposure", 0.0, (-4.0, 4.0), (-2.0, 2.0), "", 2), COLOR),
            grp(fs("contrast", "Contrast", 0.0, (-100.0, 100.0), (-100.0, 100.0), "", 1), COLOR),
            grp(fs("saturation", "Saturation", 100.0, (0.0, 300.0), (0.0, 200.0), "%", 1), COLOR),
            grp(fs("temperature", "Temperature", 0.0, (-100.0, 100.0), (-100.0, 100.0), "", 1), COLOR),
            grp(fs("tint", "Tint", 0.0, (-100.0, 100.0), (-100.0, 100.0), "", 1), COLOR),
            grp(col("shadow_color", "Shadow Color", [0.0, 0.45, 0.55, 1.0]), COLOR),
            grp(pct("shadow_amt", "Shadow Tint", 0.0), COLOR),
            grp(col("highlight_color", "Highlight Color", [1.0, 0.65, 0.35, 1.0]), COLOR),
            grp(pct("highlight_amt", "Highlight Tint", 0.0), COLOR),
            grp(fs("fade", "Black Lift (Fade)", 0.0, (0.0, 50.0), (0.0, 30.0), "%", 1), MATTE),
            grp(fs("rolloff", "Highlight Roll-off", 0.0, (0.0, 50.0), (0.0, 30.0), "%", 1), MATTE),
            grp(pct("diffusion", "Diffusion", 0.0), LENS),
            grp(fs("diffusion_size", "Diffusion Size", 30.0, (1.0, 400.0), (1.0, 150.0), "", 1), LENS),
            grp(pct("halation", "Halation", 0.0), LENS),
            grp(pct("vignette", "Vignette", 0.0), LENS),
            grp(fs("vignette_size", "Vignette Size", 60.0, (10.0, 150.0), (10.0, 150.0), "%", 1), LENS),
            grp(pct("grain", "Film Grain", 0.0), FILM),
            grp(fs("grain_size", "Grain Size", 1.2, (0.5, 6.0), (0.5, 4.0), "", 2), FILM),
            seed(),
        ],
    )
}

pub(super) fn defs() -> Vec<EffectDef> {
    let mut v = vec![organic_shake(), flicker_stack(), look_builder()];
    for d in &mut v {
        d.accelerated = false;
        d.float32 = true;
        d.yuv = false;
    }
    v
}

#[cfg(test)]
mod tests {
    use crate::find_effect;

    #[test]
    fn plus_effects_are_registered_in_their_folder() {
        for id in ["organic_shake", "flicker_stack", "look_builder"] {
            let d = find_effect(id).unwrap_or_else(|| panic!("missing {id}"));
            assert_eq!(d.category.first(), Some(&super::PLUS_TOP), "{id}");
            // every parameter id is unique
            let mut ids: Vec<_> = d.params.iter().map(|p| p.id).collect();
            ids.sort_unstable();
            let n = ids.len();
            ids.dedup();
            assert_eq!(ids.len(), n, "{id} has duplicate parameter ids");
        }
    }
}
