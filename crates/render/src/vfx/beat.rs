//! FilmCraft Plus: what the music is doing right now, for the audio-reactive Plus effects.
//!
//! Works on a short window of mono sequence audio that *ends* at the current frame
//! ([`super::FxEnv::audio`]), so an effect stays a pure function of time (no state between frames):
//!
//! - **Kick** (sub + kick, below ~150 Hz) and **snare** (crack 1.8–7 kHz plus body 150–350 Hz) hits:
//!   the energy of each band per 5 ms hop, an onset where it rises sharply above the 15–60 ms before
//!   it and is among the loudest of the window (so hi-hats and ghost notes do not count). Hops are
//!   aligned to absolute sample positions, so a hit gets the same index on every frame it is seen
//!   from: the effects use it to pick a stable random direction per hit.
//! - **Spectrum**: Hann-windowed FFT bands on a log scale, weighted towards the low end (sub and
//!   kick full, snare body and crack medium, melody by a "Melody Response" factor), with a release
//!   (towers fall smoothly) and gravity-falling peaks, both from spectra of the recent past.
//!
//! Original FilmCraft code (no third-party DSP).

use filmcraft_audio_dsp::fft::Fft;

/// A drum hit seen in the window.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Hit {
    /// Stable identity of the hit (its absolute hop number).
    pub index: i64,
    /// Seconds since the hit.
    pub age: f64,
    /// 0..1.
    pub strength: f32,
}

impl Hit {
    /// `strength`, decayed exponentially with `decay` seconds.
    pub fn env(&self, decay: f64) -> f32 {
        if !self.age.is_finite() || self.age < 0.0 {
            return 0.0;
        }
        (f64::from(self.strength) * (-self.age / decay.max(0.01)).exp()) as f32
    }
}

/// Audio window the effects ask for (seconds).
pub const WINDOW: f64 = 1.3;

/// A second-order filter section (RBJ cookbook form).
#[derive(Clone, Copy)]
struct Biquad {
    b: [f32; 3],
    a: [f32; 2],
}

impl Biquad {
    fn lowpass(f: f32, sr: f32) -> Biquad {
        Self::make(f, sr, 0.707, false)
    }
    fn highpass(f: f32, sr: f32) -> Biquad {
        Self::make(f, sr, 0.707, true)
    }
    fn make(f: f32, sr: f32, q: f32, high: bool) -> Biquad {
        let w = std::f32::consts::TAU * (f / sr).clamp(1e-4, 0.49);
        let (s, c) = w.sin_cos();
        let alpha = s / (2.0 * q);
        let a0 = 1.0 + alpha;
        let (b0, b1) = if high { ((1.0 + c) / 2.0, -(1.0 + c)) } else { ((1.0 - c) / 2.0, 1.0 - c) };
        Biquad { b: [b0 / a0, b1 / a0, b0 / a0], a: [-2.0 * c / a0, (1.0 - alpha) / a0] }
    }
    fn run(&self, x: &[f32]) -> Vec<f32> {
        let (mut x1, mut x2, mut y1, mut y2) = (0.0f32, 0.0f32, 0.0f32, 0.0f32);
        x.iter()
            .map(|&v| {
                let v = if v.is_finite() { v } else { 0.0 };
                let y = self.b[0] * v + self.b[1] * x1 + self.b[2] * x2 - self.a[0] * y1 - self.a[1] * y2;
                (x2, x1, y2, y1) = (x1, v, y1, y);
                y
            })
            .collect()
    }
}

fn chain(x: &[f32], filters: &[Biquad]) -> Vec<f32> {
    filters.iter().fold(x.to_vec(), |acc, f| f.run(&acc))
}

/// Energy (dB) of `x` per hop; hop `k` covers absolute samples `[first + k·hop, … + hop)`.
fn hop_energy(x: &[f32], start: i64, hop: usize) -> (i64, Vec<f32>) {
    let hop_i = hop as i64;
    let first = start.div_euclid(hop_i) + 1; // first hop fully inside
    let off = (first * hop_i - start).max(0) as usize;
    let e = x.get(off..).unwrap_or(&[]).chunks_exact(hop).map(|c| 10.0 * (c.iter().map(|v| v * v).sum::<f32>() / hop as f32 + 1e-10).log10()).collect();
    (first, e)
}

/// The onsets in an energy series, oldest first (quieter than `gate_db` below the window's loudest
/// hop do not count).
fn onsets(e: &[f32], first: i64, hop: usize, sr: u32, rise_db: f32, gate_db: f32, skip: usize) -> Vec<(i64, f32)> {
    let max = e.iter().copied().fold(f32::MIN, f32::max);
    let mut out = Vec::new();
    if max < -60.0 {
        return out;
    }
    let flux = |k: usize| -> f32 {
        let lo = k.saturating_sub(12);
        let hi = k.saturating_sub(3);
        if hi <= lo {
            return 0.0;
        }
        let prev = e.get(lo..hi).map_or(0.0, |w| w.iter().sum::<f32>() / w.len() as f32);
        e.get(k).copied().unwrap_or(f32::MIN) - prev
    };
    let min_gap = (sr as usize / 10 / hop).max(1); // 100 ms
    let mut k = skip.max(12);
    while k < e.len() {
        let f = flux(k);
        // the rising edge: the first hop over the threshold (the loudest hop of the hit is a few
        // milliseconds later, and the strength comes from there)
        let loud = (k..(k + 4).min(e.len())).any(|j| e.get(j).is_some_and(|v| *v > max - gate_db && *v > -55.0));
        if f > rise_db && loud {
            let top = (k..(k + 4).min(e.len())).map(flux).fold(f, f32::max);
            out.push((first + k as i64, (0.45 + (top - rise_db) / 14.0).clamp(0.45, 1.0)));
            k += min_gap;
        } else {
            k += 1;
        }
    }
    out
}

/// Kick and snare hits in `x` (mono, ending at absolute sample `end`), `sens` 0.25..4 (1 = normal).
pub fn hits(x: &[f32], sr: u32, end: i64, sens: f32) -> (Option<Hit>, Option<Hit>) {
    if x.len() < 2048 || sr < 8000 {
        return (None, None);
    }
    let srf = sr as f32;
    let hop = ((f64::from(sr) * 0.005).round() as usize).max(16);
    let start = end - x.len() as i64;
    let rise = (6.0 / sens.clamp(0.25, 4.0)).clamp(1.5, 20.0);
    let skip = (0.06 * f64::from(sr)) as usize / hop; // filter warm-up
    let kick_band = chain(x, &[Biquad::lowpass(150.0, srf), Biquad::lowpass(150.0, srf)]);
    let crack = chain(x, &[Biquad::highpass(1800.0, srf), Biquad::lowpass(7000.0, srf)]);
    let body = chain(x, &[Biquad::highpass(150.0, srf), Biquad::lowpass(350.0, srf)]);
    let (first, ek) = hop_energy(&kick_band, start, hop);
    let (_, ec) = hop_energy(&crack, start, hop);
    let (_, eb) = hop_energy(&body, start, hop);
    // the snare: the crack, helped a little by the body (claps have no body; hi-hats are quieter)
    let es: Vec<f32> = ec.iter().zip(&eb).map(|(c, b)| c + 0.25 * (b - c).clamp(0.0, 6.0)).collect();
    let age = |index: i64| (end - index * hop as i64) as f64 / f64::from(sr);
    let kicks = onsets(&ek, first, hop, sr, rise, 15.0, skip);
    let kick = kicks.last().map(|&(i, s)| Hit { index: i, age: age(i), strength: s });
    let snare = onsets(&es, first, hop, sr, rise + 1.0, 10.0, skip)
        .into_iter()
        // the beater click of a kick is not a snare
        .rfind(|(i, _)| kicks.iter().all(|(k, _)| (k - i).abs() > 3))
        .map(|(i, s)| Hit { index: i, age: age(i), strength: s });
    (kick, snare)
}

/// Spectrum settings.
#[derive(Clone, Copy, Debug)]
pub struct SpectrumCfg {
    pub bands: usize,
    pub fmin: f32,
    pub fmax: f32,
    /// Level gain (1 = 0 dBFS full scale, 2 = +6 dB).
    pub gain: f32,
    /// Weight of the melody range (above ~400 Hz), 0..1.
    pub melody: f32,
    /// Seconds for a tower to fall to 1/e.
    pub release: f32,
    /// Seconds a peak holds before it falls.
    pub hold: f32,
}

/// 2048 points: 43 ms at 48 kHz, short enough that a tower rises with its kick.
const FFT_N: usize = 2048;
const RANGE_DB: f32 = 54.0;

/// Weight of a band centred at `f` Hz: sub and kick full, low mids a little less, the rest
/// `melody` (snare body and crack a bit more).
pub fn weight(f: f32, melody: f32) -> f32 {
    let m = melody.clamp(0.0, 1.0);
    let lerp = |a: f32, b: f32, t: f32| a + (b - a) * t.clamp(0.0, 1.0);
    let l = |lo: f32, hi: f32| (f / lo).ln() / (hi / lo).ln();
    if f <= 150.0 {
        1.0
    } else if f <= 400.0 {
        lerp(1.0, 0.8, l(150.0, 400.0))
    } else if f <= 1500.0 {
        lerp(0.8, m, l(400.0, 1500.0))
    } else if f <= 6000.0 {
        m.max(0.5 * m + 0.25)
    } else {
        m
    }
}

/// The centre frequency (Hz) of each band of `cfg` at sample rate `sr`.
pub fn band_centers(cfg: &SpectrumCfg, sr: u32) -> Vec<f32> {
    let (lo, hi) = band_range(cfg, sr);
    let n = cfg.bands.max(1);
    (0..n).map(|i| lo * (hi / lo).powf((i as f32 + 0.5) / n as f32)).collect()
}

fn band_range(cfg: &SpectrumCfg, sr: u32) -> (f32, f32) {
    let lo = if cfg.fmin.is_finite() { cfg.fmin.max(10.0) } else { 30.0 };
    let hi = if cfg.fmax.is_finite() { cfg.fmax.min(sr as f32 * 0.49) } else { 12_000.0 };
    (lo, hi.max(lo * 1.5))
}

/// Levels (0..1) of the bands for the FFT window ending `back` samples before the end of `x`.
fn levels(x: &[f32], back: usize, fft: &Fft, win: &[f32], cfg: &SpectrumCfg, sr: u32) -> Vec<f32> {
    let n = FFT_N;
    let end = x.len().saturating_sub(back);
    let seg = x.get(end.saturating_sub(n)..end).unwrap_or(&[]);
    let mut re = vec![0.0f32; n];
    let mut im = vec![0.0f32; n];
    let pad = n - seg.len();
    for (i, v) in seg.iter().enumerate() {
        if let (Some(r), Some(w)) = (re.get_mut(pad + i), win.get(pad + i)) {
            *r = if v.is_finite() { v * w } else { 0.0 };
        }
    }
    fft.forward(&mut re, &mut im);
    let norm = 4.0 / n as f32; // Hann coherent gain 0.5, one-sided ×2
    let mag = |b: usize| re.get(b).zip(im.get(b)).map_or(0.0, |(r, i)| (r * r + i * i).sqrt() * norm);
    let bin_hz = sr as f32 / n as f32;
    let (lo, hi) = band_range(cfg, sr);
    let bands = cfg.bands.max(1);
    (0..bands)
        .map(|i| {
            let f0 = lo * (hi / lo).powf(i as f32 / bands as f32);
            let f1 = lo * (hi / lo).powf((i + 1) as f32 / bands as f32);
            let fc = (f0 * f1).sqrt();
            let (b0, b1) = ((f0 / bin_hz).floor() as usize, ((f1 / bin_hz).ceil() as usize).max((f0 / bin_hz).floor() as usize + 1));
            let m = (b0..b1.min(n / 2)).map(mag).fold(0.0, f32::max);
            // lift the highs a little (+2 dB / octave above 1 kHz) so melodies stay visible
            let tilt = if fc > 1000.0 { 2.0 * (fc / 1000.0).log2() } else { 0.0 };
            let db = 20.0 * (m * cfg.gain + 1e-9).log10() + tilt;
            ((db + RANGE_DB) / RANGE_DB).clamp(0.0, 1.0) * weight(fc, cfg.melody)
        })
        .collect()
}

/// Band levels (with release) and peaks (with hold and gravity) at the end of `x`.
pub fn spectrum(x: &[f32], sr: u32, cfg: &SpectrumCfg) -> (Vec<f32>, Vec<f32>) {
    let bands = cfg.bands.clamp(1, 256);
    if x.is_empty() || sr < 8000 {
        return (vec![0.0; bands], vec![0.0; bands]);
    }
    let cfg = SpectrumCfg { bands, ..*cfg };
    let fft = Fft::new(FFT_N);
    let win: Vec<f32> = (0..FFT_N).map(|i| 0.5 - 0.5 * (std::f32::consts::TAU * i as f32 / FFT_N as f32).cos()).collect();
    let step = |s: f64| (s * f64::from(sr)) as usize;
    // release: the loudest recent level, faded by its age
    let rel = cfg.release.max(0.01);
    let mut level = vec![0.0f32; bands];
    for k in 0..6 {
        let age = k as f32 / 60.0;
        let l = levels(x, step(f64::from(age)), &fft, &win, &cfg, sr);
        let fade = (-age / rel).exp();
        for (a, b) in level.iter_mut().zip(&l) {
            *a = a.max(b * fade);
        }
    }
    // peaks: held, then falling with gravity
    let mut peak = level.clone();
    for k in 1..13 {
        let age = k as f32 * 0.05;
        let l = levels(x, step(f64::from(age)), &fft, &win, &cfg, sr);
        let fall = 3.0 * (age - cfg.hold.max(0.0)).max(0.0).powi(2);
        for (p, v) in peak.iter_mut().zip(&l) {
            *p = p.max(v - fall);
        }
    }
    (level, peak)
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;

    pub(crate) const SR: u32 = 48_000;

    /// 2 s of a beat: kicks (55 Hz thump) at 0, 0.5, 1.0, 1.5 s, snares (noise + 200 Hz body) at
    /// 0.25, 0.75, 1.25, 1.75 s, quiet hi-hats every 1/8 s and a soft 660 Hz melody.
    pub(crate) fn beat() -> Vec<f32> {
        let n = 2 * SR as usize;
        let mut seed = 12345u32;
        let mut noise = move || {
            seed ^= seed << 13;
            seed ^= seed >> 17;
            seed ^= seed << 5;
            seed as f32 / u32::MAX as f32 * 2.0 - 1.0
        };
        (0..n)
            .map(|i| {
                let t = i as f32 / SR as f32;
                let tk = t % 0.5;
                let kick = (-(tk / 0.12)).exp() * (std::f32::consts::TAU * (55.0 + 80.0 * (-(tk / 0.02)).exp()) * tk).sin() * 0.8;
                let ts = (t + 0.25) % 0.5;
                let snare = (-(ts / 0.08)).exp() * (noise() * 0.45 + 0.25 * (std::f32::consts::TAU * 200.0 * ts).sin());
                let th = t % 0.125;
                let hat = (-(th / 0.01)).exp() * noise() * 0.04;
                let mel = 0.05 * (std::f32::consts::TAU * 660.0 * t).sin();
                kick + snare + hat + mel
            })
            .collect()
    }

    /// The window of `beat()` ending at second `t`.
    pub(crate) fn window(x: &[f32], t: f64) -> (&[f32], i64) {
        let end = (t * f64::from(SR)) as usize;
        let start = end.saturating_sub((WINDOW * f64::from(SR)) as usize);
        (x.get(start..end).unwrap_or(&[]), end as i64)
    }

    #[test]
    fn finds_kicks_and_snares_where_they_are() {
        let x = beat();
        // 30 ms after the kick at 1.0 s
        let (w, end) = window(&x, 1.03);
        let (k, s) = hits(w, SR, end, 1.0);
        let k = k.expect("kick");
        assert!((k.age - 0.03).abs() < 0.012, "kick age {}", k.age);
        let s = s.expect("snare");
        assert!((s.age - 0.28).abs() < 0.012, "snare age {}", s.age);
        // 20 ms after the snare at 1.25 s
        let (w, end) = window(&x, 1.27);
        let (k2, s2) = hits(w, SR, end, 1.0);
        assert!((s2.expect("snare").age - 0.02).abs() < 0.012);
        // the same kick, seen from a later frame, keeps its index
        assert_eq!(k2.expect("kick").index, k.index);
    }

    #[test]
    fn silence_and_garbage_have_no_hits() {
        assert_eq!(hits(&vec![0.0; 60_000], SR, 60_000, 1.0), (None, None));
        assert_eq!(hits(&[f32::NAN; 60_000], SR, 60_000, 1.0), (None, None));
        assert_eq!(hits(&[0.1; 10], SR, 10, 1.0), (None, None));
        let (l, p) = spectrum(&[f32::INFINITY; 70_000], SR, &cfg());
        assert!(l.iter().chain(&p).all(|v| v.is_finite()));
    }

    fn cfg() -> SpectrumCfg {
        SpectrumCfg { bands: 20, fmin: 30.0, fmax: 12_000.0, gain: 1.0, melody: 0.35, release: 0.18, hold: 0.25 }
    }

    #[test]
    fn spectrum_leans_on_the_low_end() {
        let x = beat();
        let (w, _) = window(&x, 1.04);
        let (l, p) = spectrum(w, SR, &cfg());
        assert_eq!(l.len(), 20);
        let low = l.iter().take(4).copied().fold(0.0, f32::max);
        let high = l.iter().skip(12).copied().fold(0.0, f32::max);
        assert!(low > 0.6, "kick towers {low}");
        assert!(high < low, "melody / hats lower than the kick: {high} vs {low}");
        assert!(p.iter().zip(&l).all(|(p, l)| p >= l));
        // the kick towers fall between hits
        let (w, _) = window(&x, 1.22);
        let (l2, _) = spectrum(w, SR, &cfg());
        assert!(l2.iter().take(4).copied().fold(0.0, f32::max) < low);
        assert!(weight(60.0, 0.35) > weight(3000.0, 0.35) && weight(3000.0, 0.35) >= weight(10_000.0, 0.35));
    }
}
