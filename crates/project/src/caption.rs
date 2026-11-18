//! Caption tracks.
//!
//! Premiere keeps captions in their own track area above the video tracks. A [`CaptionTrack`] has a
//! format (Subtitle, CEA-608, CEA-708, Teletext), a track style (font, size, colour, background box,
//! position) and a list of [`Caption`] blocks with exact in/out times in sequence ticks. Captions on
//! one track never overlap (like track items); [`CaptionTrack::check`] verifies it.
//!
//! Caption text is kept as written (lines separated by `\n`, inline `<i>`/`<b>`/`<u>` tags and
//! WebVTT cue settings preserved) so files round-trip; renderers strip markup for display.

use filmcraft_time::{Tick, TimeRange};
use serde::{Deserialize, Serialize};

use crate::{ClipId, TrackId};

/// Caption stream format of a track (Premiere's "Caption Format" choices).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum CaptionFormat {
    /// Open/sidecar subtitles (SRT, WebVTT).
    #[default]
    Subtitle,
    /// CEA-608 (line 21) closed captions.
    Cea608,
    /// CEA-708 (DTVCC) closed captions.
    Cea708,
    /// EBU Teletext subtitles.
    Teletext,
}

impl CaptionFormat {
    pub const ALL: [CaptionFormat; 4] = [CaptionFormat::Subtitle, CaptionFormat::Cea608, CaptionFormat::Cea708, CaptionFormat::Teletext];
    pub fn label(self) -> &'static str {
        match self {
            CaptionFormat::Subtitle => "Subtitle",
            CaptionFormat::Cea608 => "CEA-608",
            CaptionFormat::Cea708 => "CEA-708",
            CaptionFormat::Teletext => "Teletext",
        }
    }
    pub fn from_name(s: &str) -> Option<CaptionFormat> {
        let n = s.to_ascii_lowercase().replace(['-', ' ', '_'], "");
        Self::ALL.iter().copied().find(|f| f.label().to_ascii_lowercase().replace(['-', ' '], "") == n)
    }
    /// Characters per line limit used when wrapping (608 is fixed at 32 columns).
    pub fn max_columns(self) -> Option<usize> {
        match self {
            CaptionFormat::Cea608 => Some(32),
            CaptionFormat::Teletext => Some(40),
            CaptionFormat::Cea708 => Some(42),
            CaptionFormat::Subtitle => None,
        }
    }
}

/// Horizontal text alignment.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum CaptionAlign {
    Left,
    #[default]
    Center,
    Right,
}

/// Vertical placement of the caption block in the frame.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum CaptionAnchor {
    Top,
    Middle,
    #[default]
    Bottom,
}

/// How the words of a caption appear while it shows (FilmCraft Plus animated captions for short
/// vertical video). The timing comes from [`Caption::words`]; a caption without word times
/// spreads its words evenly over its duration.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum CaptionAnimation {
    /// Static captions (the whole block shows at once).
    #[default]
    None,
    /// Words pop in as they are spoken; the spoken word is highlighted and slightly larger.
    Pop,
    /// All words show; a rounded box in the highlight colour glides to the spoken word.
    Box,
    /// One big word at a time, popping in.
    OneWord,
    /// Words start dim and fill with the highlight colour from left to right as they are spoken.
    Karaoke,
    /// Words rise into place and fade in; no highlight.
    Rise,
    /// Every word on its own tile (text in the highlight colour's contrast), stacked as spoken.
    Tiles,
}

impl CaptionAnimation {
    pub const ALL: [CaptionAnimation; 7] = [
        CaptionAnimation::None,
        CaptionAnimation::Pop,
        CaptionAnimation::Box,
        CaptionAnimation::OneWord,
        CaptionAnimation::Karaoke,
        CaptionAnimation::Rise,
        CaptionAnimation::Tiles,
    ];
    pub fn label(self) -> &'static str {
        match self {
            CaptionAnimation::None => "None",
            CaptionAnimation::Pop => "Pop",
            CaptionAnimation::Box => "Box",
            CaptionAnimation::OneWord => "One Word",
            CaptionAnimation::Karaoke => "Karaoke",
            CaptionAnimation::Rise => "Rise",
            CaptionAnimation::Tiles => "Tiles",
        }
    }
    pub fn from_name(s: &str) -> Option<CaptionAnimation> {
        let n = s.to_ascii_lowercase().replace(['-', ' ', '_'], "");
        Self::ALL.iter().copied().find(|a| a.label().to_ascii_lowercase().replace(' ', "") == n)
    }
    fn is_none(&self) -> bool {
        *self == CaptionAnimation::None
    }
}

/// One word of a caption with its time, for animated captions.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct CaptionWord {
    pub text: String,
    /// When the word starts being spoken, from the caption's start.
    pub start: Tick,
    /// When it ends, from the caption's start.
    pub end: Tick,
}

const DEFAULT_HIGHLIGHT: [u8; 4] = [255, 225, 77, 255];
fn is_default_highlight(c: &[u8; 4]) -> bool {
    *c == DEFAULT_HIGHLIGHT
}
fn is_false(b: &bool) -> bool {
    !*b
}
fn is_zero(v: &f32) -> bool {
    *v == 0.0
}

/// Track-level caption style (Premiere's caption track style / Essential Graphics text settings).
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct CaptionStyle {
    /// Font family name (bundled, craft-fonts or system); unknown or empty names fall back to Inter.
    pub font: String,
    /// Font size in pixels for a 1080-line frame (scaled with the frame height).
    pub size: f32,
    /// Text colour, sRGB + alpha.
    pub color: [u8; 4],
    /// Draw a background box behind each line.
    pub background: bool,
    /// Background box colour, sRGB + alpha.
    pub background_color: [u8; 4],
    pub align: CaptionAlign,
    pub anchor: CaptionAnchor,
    /// Distance from the anchored frame edge, as a fraction of the frame height.
    pub margin: f32,
    /// Line height as a multiple of the font size.
    pub line_spacing: f32,
    /// Text outline width in pixels at 1080 lines (0 = none).
    pub outline: f32,
    pub outline_color: [u8; 4],
    /// Font style within the family (`Regular`, `Medium`, `SemiBold`, `Bold Italic`…), resolved
    /// like graphic text: the nearest weight when the family has no face of that name.
    pub font_style: String,
    /// Word animation (FilmCraft Plus).
    #[serde(skip_serializing_if = "CaptionAnimation::is_none")]
    pub animation: CaptionAnimation,
    /// Colour of the spoken word (or its box / tile), sRGB + alpha.
    #[serde(skip_serializing_if = "is_default_highlight")]
    pub highlight_color: [u8; 4],
    /// Draw the text in capitals.
    #[serde(skip_serializing_if = "is_false")]
    pub uppercase: bool,
    /// Soft drop shadow strength 0..1 (0 = none).
    #[serde(skip_serializing_if = "is_zero")]
    pub shadow: f32,
}

impl Default for CaptionStyle {
    fn default() -> Self {
        Self {
            font: "Inter".into(),
            size: 54.0,
            color: [255, 255, 255, 255],
            background: true,
            background_color: [0, 0, 0, 191],
            align: CaptionAlign::Center,
            anchor: CaptionAnchor::Bottom,
            margin: 0.08,
            line_spacing: 1.25,
            outline: 0.0,
            outline_color: [0, 0, 0, 255],
            font_style: "SemiBold".into(),
            animation: CaptionAnimation::None,
            highlight_color: DEFAULT_HIGHLIGHT,
            uppercase: false,
            shadow: 0.0,
        }
    }
}

/// The animated caption looks (FilmCraft Plus), for [`CaptionStyle::look`].
pub const CAPTION_LOOKS: &[&str] = &["Punch", "Spotlight", "One Word", "Karaoke", "Rise", "Tiles"];

impl CaptionStyle {
    /// A ready-made animated caption look by name (see [`CAPTION_LOOKS`]; case, spaces and dashes
    /// are ignored). Sizes are for a 1080-line picture and follow the frame's area.
    pub fn look(name: &str) -> Option<CaptionStyle> {
        let n = name.to_ascii_lowercase().replace([' ', '-', '_'], "");
        let base = CaptionStyle {
            font_style: "Bold".into(),
            background: false,
            anchor: CaptionAnchor::Bottom,
            margin: 0.24,
            line_spacing: 1.15,
            uppercase: true,
            ..Default::default()
        };
        Some(match n.as_str() {
            // white capitals, the spoken word pops in yellow
            "punch" => CaptionStyle { animation: CaptionAnimation::Pop, size: 100.0, outline: 7.0, shadow: 0.6, ..base },
            // a violet box glides from word to word
            "spotlight" => {
                CaptionStyle { animation: CaptionAnimation::Box, size: 90.0, outline: 0.0, shadow: 0.8, highlight_color: [124, 58, 237, 255], ..base }
            }
            // one big word at a time
            "oneword" => CaptionStyle { animation: CaptionAnimation::OneWord, size: 165.0, outline: 9.0, shadow: 0.7, margin: 0.3, ..base },
            // words fill with mint green as they are spoken
            "karaoke" => {
                CaptionStyle { animation: CaptionAnimation::Karaoke, size: 92.0, outline: 6.0, shadow: 0.5, highlight_color: [61, 255, 180, 255], ..base }
            }
            // quiet: mixed case, words rise in, soft shadow only
            "rise" => CaptionStyle {
                animation: CaptionAnimation::Rise,
                size: 80.0,
                outline: 0.0,
                shadow: 1.0,
                uppercase: false,
                font_style: "SemiBold".into(),
                ..base
            },
            // black capitals on white tiles
            "tiles" => CaptionStyle {
                animation: CaptionAnimation::Tiles,
                size: 82.0,
                outline: 0.0,
                shadow: 0.6,
                color: [12, 12, 12, 255],
                highlight_color: [245, 245, 245, 255],
                ..base
            },
            _ => return None,
        })
    }
}

/// One caption block on a caption track.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Caption {
    pub id: ClipId,
    /// Timeline start (sequence ticks).
    pub start: Tick,
    pub duration: Tick,
    /// Caption text; lines separated by `\n`.
    pub text: String,
    /// Speaker name (WebVTT `<v Name>` voice span).
    #[serde(default)]
    pub speaker: Option<String>,
    /// WebVTT cue identifier, kept for round trips.
    #[serde(default)]
    pub cue_id: Option<String>,
    /// WebVTT cue settings (`line:90% align:start`…), kept verbatim.
    #[serde(default)]
    pub settings: String,
    /// The words with their times, for animated captions (empty for ordinary captions).
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub words: Vec<CaptionWord>,
}

impl Caption {
    pub fn end(&self) -> Tick {
        self.start + self.duration
    }
    pub fn range(&self) -> TimeRange {
        TimeRange::new(self.start, self.duration)
    }
    /// The caption's text with markup removed and entities decoded (what is drawn).
    pub fn plain_lines(&self) -> Vec<String> {
        plain_text(&self.text).lines().map(str::to_string).collect()
    }
}

/// A caption track (shown in the caption area above the video tracks).
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct CaptionTrack {
    pub id: TrackId,
    pub name: String,
    pub format: CaptionFormat,
    /// BCP-47 language tag (`en`, `fr-CA`…).
    #[serde(default)]
    pub language: String,
    #[serde(default)]
    pub style: CaptionStyle,
    /// Track output (eye): shown in the Program monitor and burned in on export.
    pub enabled: bool,
    pub locked: bool,
    /// Follows inserts / extracts on other tracks.
    #[serde(default = "yes")]
    pub sync_lock: bool,
    /// Captions sorted by start, non-overlapping.
    pub captions: Vec<Caption>,
    /// WebVTT header blocks (STYLE / REGION / NOTE) kept for round trips.
    #[serde(default)]
    pub vtt_blocks: Vec<String>,
}

fn yes() -> bool {
    true
}

impl CaptionTrack {
    pub fn new(id: TrackId, name: String, format: CaptionFormat) -> Self {
        Self {
            id,
            name,
            format,
            language: "en".into(),
            style: CaptionStyle::default(),
            enabled: true,
            locked: false,
            sync_lock: true,
            captions: Vec::new(),
            vtt_blocks: Vec::new(),
        }
    }
    pub fn end(&self) -> Tick {
        self.captions.iter().map(Caption::end).max().unwrap_or(Tick::ZERO)
    }
    /// Caption covering `t`.
    pub fn caption_at(&self, t: Tick) -> Option<&Caption> {
        let idx = self.captions.partition_point(|c| c.start <= t);
        idx.checked_sub(1).map(|i| &self.captions[i]).filter(|c| t < c.end())
    }
    pub fn caption(&self, id: ClipId) -> Option<&Caption> {
        self.captions.iter().find(|c| c.id == id)
    }
    pub fn caption_mut(&mut self, id: ClipId) -> Option<&mut Caption> {
        self.captions.iter_mut().find(|c| c.id == id)
    }
    pub fn sort(&mut self) {
        self.captions.sort_by_key(|c| c.start);
    }
    /// Invariant check: captions sorted, positive duration, no overlaps.
    pub fn check(&self) -> Result<(), String> {
        self.check_bounds()?;
        for c in &self.captions {
            if c.duration.0 <= 0 {
                return Err(format!("{}: caption {:?} has non-positive duration", self.name, c.id));
            }
        }
        for w in self.captions.windows(2) {
            if w[0].end() > w[1].start {
                return Err(format!("{}: captions {:?} and {:?} overlap", self.name, w[0].id, w[1].id));
            }
        }
        Ok(())
    }
    /// Only the time bounds that keep caption arithmetic from overflowing (see `Track::check_bounds`).
    pub fn check_bounds(&self) -> Result<(), String> {
        for c in &self.captions {
            if !crate::bounded_time_range(c.start, c.duration) {
                return Err(format!("{}: caption {:?} is outside supported time bounds", self.name, c.id));
            }
        }
        Ok(())
    }
}

/// Strip `<…>` tags and decode the common HTML/WebVTT entities.
pub fn plain_text(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    let mut chars = s.chars().peekable();
    while let Some(c) = chars.next() {
        match c {
            '<' => {
                // A tag runs to the next '>'.  Scan forward once; preserving an unterminated
                // tag as literal text must not rescan every later opening bracket.
                let mut literal = String::new();
                let mut closed = false;
                for next in chars.by_ref() {
                    if next == '>' {
                        closed = true;
                        break;
                    }
                    literal.push(next);
                }
                if !closed {
                    out.push('<');
                    out.push_str(&literal);
                }
            }
            '{' if chars.peek() == Some(&'\\') => {
                // SSA-style override blocks some SRT files carry: {\an8}
                let mut literal = String::new();
                let mut closed = false;
                for next in chars.by_ref() {
                    if next == '}' {
                        closed = true;
                        break;
                    }
                    literal.push(next);
                }
                if !closed {
                    out.push('{');
                    out.push_str(&literal);
                }
            }
            '&' => {
                let rest: String = chars.clone().take(8).collect();
                let ents = [
                    ("amp;", '&'),
                    ("lt;", '<'),
                    ("gt;", '>'),
                    ("nbsp;", '\u{a0}'),
                    ("quot;", '"'),
                    ("apos;", '\''),
                    ("lrm;", '\u{200e}'),
                    ("rlm;", '\u{200f}'),
                ];
                if let Some((name, ch)) = ents.iter().find(|(n, _)| rest.starts_with(n)) {
                    for _ in 0..name.len() {
                        chars.next();
                    }
                    out.push(*ch);
                } else {
                    out.push('&');
                }
            }
            '\r' => {}
            _ => out.push(c),
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn styles_saved_without_a_font_style_load_semibold() {
        let old = r#"{"font":"Inter","size":54.0,"color":[255,255,255,255],"background":true,"background_color":[0,0,0,191],
            "align":"Center","anchor":"Bottom","margin":0.08,"line_spacing":1.25,"outline":0.0,"outline_color":[0,0,0,255]}"#;
        let st: CaptionStyle = serde_json::from_str(old).unwrap();
        assert_eq!(st, CaptionStyle::default());
        assert_eq!(st.font_style, "SemiBold");
    }

    #[test]
    fn plain() {
        assert_eq!(plain_text("<i>Hello</i> &amp; <b>bye</b>"), "Hello & bye");
        assert_eq!(plain_text("{\\an8}Top"), "Top");
        assert_eq!(plain_text("a < b"), "a < b");
        assert_eq!(plain_text("<v Bob>Hi</v>"), "Hi");
    }

    #[test]
    fn malformed_caption_markup_is_preserved_without_repeated_suffix_scans() {
        let angle = "<".repeat(10_000);
        let ssa = format!("{{\\{}", "{".repeat(10_000));
        assert_eq!(plain_text(&angle), angle);
        assert_eq!(plain_text(&ssa), ssa);
    }

    #[test]
    fn caption_at() {
        let mut t = CaptionTrack::new(TrackId(1), "Subtitle".into(), CaptionFormat::Subtitle);
        for (i, s) in [0i64, 100, 300].iter().enumerate() {
            t.captions.push(Caption {
                id: ClipId(i as u64),
                start: Tick(*s),
                duration: Tick(50),
                text: format!("c{i}"),
                speaker: None,
                cue_id: None,
                settings: String::new(),
                words: Vec::new(),
            });
        }
        assert_eq!(t.caption_at(Tick(120)).unwrap().text, "c1");
        assert!(t.caption_at(Tick(160)).is_none());
        assert!(t.check().is_ok());
        assert_eq!(CaptionFormat::from_name("cea608"), Some(CaptionFormat::Cea608));
        assert_eq!(CaptionFormat::from_name("CEA-708"), Some(CaptionFormat::Cea708));
    }

    #[test]
    fn corrupt_caption_times_do_not_overflow_the_overlap_check() {
        let mut track = CaptionTrack::new(TrackId(1), "Subtitles".into(), CaptionFormat::Subtitle);
        for (id, start) in [(1, i64::MAX), (2, 10)] {
            track.captions.push(Caption {
                id: ClipId(id),
                start: Tick(start),
                duration: Tick(50),
                text: String::new(),
                speaker: None,
                cue_id: None,
                settings: String::new(),
                words: Vec::new(),
            });
        }
        let result = std::panic::catch_unwind(|| track.check());
        assert!(result.is_ok());
        assert!(result.unwrap().is_err());
    }
}
