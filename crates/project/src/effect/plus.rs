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
const MUSIC: &[&str] = &[PLUS_TOP, "Music"];

/// Music Sync: what the effect locks to. `Markers ▸ Sync Plus Effects to Music` fills in the
/// tempo and the first beat from the selected music clip.
/// "Kick (Audio)" and "Snare (Audio)" listen to the sequence mix instead of a tempo.
pub const SYNC_MODES: &[&str] = &["Off", "Every Beat", "Every 2 Beats", "Every Bar (4 Beats)", "Half Beats", "Kick (Audio)", "Snare (Audio)"];
/// Index of "Kick (Audio)" in [`SYNC_MODES`] ("Snare (Audio)" is the next one).
pub const SYNC_KICK: u32 = 5;
/// AVKO Spectrum: which audio it listens to.
pub const AUDIO_SOURCES: &[&str] = &["Master Mix", "A1", "A2", "A3", "A4", "A5", "A6", "A7", "A8"];
/// Beat Pulse presets (index 0 = Custom).
pub const PULSE_PRESETS: &[&str] = &["Custom", "Bass Punch", "Flash", "Glow Hit", "RGB Kick", "Zoom Bounce", "Shake Hit"];

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
/// The Music Sync group shared by the Plus effects (`sync_default` = index into [`SYNC_MODES`]).
fn music_sync(sync_default: u32) -> Vec<ParamDef> {
    const G: &str = "Music Sync";
    vec![
        grp(ch("sync", "Sync to Music", SYNC_MODES, sync_default), G),
        grp(fs("bpm", "Tempo (BPM)", 120.0, (20.0, 400.0), (60.0, 200.0), "", 2), G),
        grp(fs("beat_offset", "First Beat", 0.0, (-600.0, 600.0), (-2.0, 2.0), "s", 3), G),
    ]
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
        ]
        .into_iter()
        .chain(music_sync(0))
        .collect(),
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
        ]
        .into_iter()
        .chain(music_sync(0))
        .collect(),
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

fn beat_pulse() -> EffectDef {
    const G: &str = "Custom Pulse";
    let mut ps = vec![
        ch("preset", "Preset", PULSE_PRESETS, 1),
        fs("amount", "Amount", 100.0, (0.0, 1000.0), (0.0, 200.0), "%", 1),
        grp(fs("decay", "Pulse Length", 35.0, (2.0, 100.0), (5.0, 100.0), "%", 1), G),
        grp(fs("zoom", "Zoom", 6.0, (0.0, 100.0), (0.0, 30.0), "%", 1), G),
        grp(fs("light", "Brightness", 30.0, (0.0, 500.0), (0.0, 200.0), "%", 1), G),
        grp(pct("glow", "Glow", 0.0), G),
        grp(fs("split", "RGB Split", 0.0, (0.0, 500.0), (0.0, 80.0), "", 1), G),
        grp(fs("shake", "Shake", 0.0, (0.0, 500.0), (0.0, 80.0), "", 1), G),
        grp(fs("blur", "Blur", 0.0, (0.0, 200.0), (0.0, 60.0), "", 1), G),
        b("accent_bar", "Accent First Beat of Bar", true),
        seed(),
    ];
    ps.extend(music_sync(1));
    video("beat_pulse", "Beat Pulse", MUSIC, ps)
}

/// The Plus effects that follow the music (they carry the Music Sync group).
pub const MUSIC_SYNCED: &[&str] = &["organic_shake", "flicker_stack", "beat_pulse"];

/// AVKO Spectrum: an audio spectrum of "Plattenbau" towers (each band a tower of window blocks,
/// antennas as falling peaks, a street underneath), weighted to sub, kick and snare, which also
/// shakes the whole picture on every kick and twitches it on every snare. Made for AVKO's logo.
fn avko_spectrum() -> EffectDef {
    const LAY: &str = "Layout";
    const RESP: &str = "Response";
    const LOOK: &str = "Look";
    const HITS: &str = "Kick & Snare";
    video(
        "avko_spectrum",
        "AVKO Spectrum",
        MUSIC,
        vec![
            ch("source", "Audio Source", AUDIO_SOURCES, 0),
            b("on_original", "Composite on Original", true),
            grp(fs("towers", "Towers", 20.0, (4.0, 96.0), (6.0, 48.0), "", 0), LAY),
            grp(fs("floors", "Max Floors", 7.0, (2.0, 40.0), (3.0, 20.0), "", 0), LAY),
            grp(fs("center_x", "Center X", 50.0, (-50.0, 150.0), (0.0, 100.0), "%", 1), LAY),
            grp(fs("baseline", "Street Height", 80.0, (0.0, 100.0), (20.0, 100.0), "%", 1), LAY),
            grp(fs("width", "Width", 76.0, (5.0, 200.0), (20.0, 100.0), "%", 1), LAY),
            grp(fs("gap", "Block Gap", 10.0, (0.0, 60.0), (0.0, 40.0), "%", 1), LAY),
            grp(fs("fmin", "Lowest Frequency", 30.0, (15.0, 2000.0), (20.0, 200.0), "Hz", 0), RESP),
            grp(fs("fmax", "Highest Frequency", 12000.0, (500.0, 22000.0), (2000.0, 20000.0), "Hz", 0), RESP),
            grp(fs("sensitivity", "Sensitivity", 100.0, (10.0, 1000.0), (25.0, 400.0), "%", 1), RESP),
            grp(pct("melody", "Melody Response", 35.0), RESP),
            grp(fs("release", "Fall Time", 0.18, (0.02, 2.0), (0.05, 0.8), "s", 2), RESP),
            grp(b("antennas", "Antennas (Peaks)", true), RESP),
            grp(fs("hold", "Antenna Hold", 0.25, (0.0, 3.0), (0.0, 1.0), "s", 2), RESP),
            grp(col("color", "Building Color", [0.88, 0.88, 0.88, 1.0]), LOOK),
            grp(pct("grunge", "Grunge", 50.0), LOOK),
            grp(pct("glow", "Glow", 15.0), LOOK),
            grp(b("street", "Street", true), LOOK),
            grp(fs("street_speed", "Street Speed", 1.0, (-20.0, 20.0), (-4.0, 4.0), "", 2), LOOK),
            grp(pct("lights", "Window Lights on Kick", 0.0), LOOK),
            grp(col("light_color", "Window Light Color", [1.0, 0.78, 0.35, 1.0]), LOOK),
            grp(fs("kick_shake", "Kick Shake", 60.0, (0.0, 500.0), (0.0, 200.0), "%", 1), HITS),
            grp(fs("kick_boost", "Kick Jump", 60.0, (0.0, 300.0), (0.0, 150.0), "%", 1), HITS),
            grp(fs("snare_twitch", "Snare Twitch", 50.0, (0.0, 500.0), (0.0, 200.0), "%", 1), HITS),
            grp(fs("snare_boost", "Snare Jump", 45.0, (0.0, 300.0), (0.0, 150.0), "%", 1), HITS),
            grp(fs("hit_decay", "Hit Length", 0.2, (0.03, 1.5), (0.05, 0.6), "s", 2), HITS),
            grp(fs("hit_sens", "Hit Sensitivity", 100.0, (25.0, 400.0), (25.0, 300.0), "%", 1), HITS),
            seed(),
        ],
    )
}

pub(super) fn defs() -> Vec<EffectDef> {
    let mut v = vec![organic_shake(), flicker_stack(), look_builder(), beat_pulse(), avko_spectrum()];
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
        for id in ["organic_shake", "flicker_stack", "look_builder", "beat_pulse", "avko_spectrum"] {
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
