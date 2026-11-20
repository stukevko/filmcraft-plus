//! Fast building blocks for the FilmCraft Plus effects, which run them on every pixel of every
//! frame: table-driven sRGB transfer functions (the exact ones call `powf` per channel) and a
//! plain bilinear enlargement (the general resampler handles any affine and costs several times
//! more). Both agree with the exact versions to well under one 10-bit code value.

use std::sync::OnceLock;

use rayon::prelude::*;

use super::Image;
use filmcraft_color::{linear_to_srgb, srgb_to_linear};

const N: usize = 4096;

fn tables() -> &'static (Vec<f32>, Vec<f32>) {
    static T: OnceLock<(Vec<f32>, Vec<f32>)> = OnceLock::new();
    T.get_or_init(|| {
        // encode is indexed by the square root of the linear value: evenly fine in the shadows,
        // where the curve is steepest
        let enc = (0..=N).map(|i| linear_to_srgb((i as f32 / N as f32).powi(2))).collect();
        let dec = (0..=N).map(|i| srgb_to_linear(i as f32 / N as f32)).collect();
        (enc, dec)
    })
}

#[inline]
fn lookup(t: &[f32], at: f32) -> f32 {
    let x = at * N as f32;
    let i = (x as usize).min(N - 1);
    let f = x - i as f32;
    match (t.get(i), t.get(i + 1)) {
        (Some(a), Some(b)) => a + (b - a) * f,
        _ => 0.0,
    }
}

#[inline]
fn enc1(v: f32) -> f32 {
    if v > 0.0 && v <= 1.0 {
        lookup(&tables().0, v.sqrt())
    } else if v > 1.0 && v.is_finite() {
        // brighter than white (rare): the exact curve
        linear_to_srgb(v)
    } else {
        0.0
    }
}

/// Linear → display-encoded (sRGB); negative and non-finite values give 0.
#[inline]
pub(super) fn enc(c: [f32; 3]) -> [f32; 3] {
    [enc1(c[0]), enc1(c[1]), enc1(c[2])]
}

/// Display-encoded (sRGB, clamped to 0..1) → linear.
#[inline]
pub(super) fn dec(c: [f32; 3]) -> [f32; 3] {
    let t = &tables().1;
    let one = |v: f32| if v > 0.0 { lookup(t, v.min(1.0)) } else { 0.0 };
    [one(c[0]), one(c[1]), one(c[2])]
}

/// `src` enlarged to `w`×`h` with bilinear filtering (pixel centres aligned, edges repeated).
pub(super) fn enlarge(src: &Image, w: usize, h: usize) -> Image {
    let mut out = Image::new(w, h);
    if src.w == 0 || src.h == 0 || w == 0 || h == 0 || src.px.len() < src.w * src.h * 4 {
        return out;
    }
    let (sw, sh) = (src.w, src.h);
    // per output column: the two source columns and the weight of the second
    let cols: Vec<(usize, usize, f32)> = (0..w)
        .map(|x| {
            let fx = ((x as f32 + 0.5) * sw as f32 / w as f32 - 0.5).clamp(0.0, (sw - 1) as f32);
            let x0 = fx as usize;
            (x0 * 4, (x0 + 1).min(sw - 1) * 4, fx - x0 as f32)
        })
        .collect();
    out.px.par_chunks_mut(w * 4).enumerate().for_each(|(y, row)| {
        let fy = ((y as f32 + 0.5) * sh as f32 / h as f32 - 0.5).clamp(0.0, (sh - 1) as f32);
        let y0 = fy as usize;
        let wy = fy - y0 as f32;
        let a = &src.px[y0 * sw * 4..(y0 + 1) * sw * 4];
        let b = &src.px[(y0 + 1).min(sh - 1) * sw * 4..((y0 + 1).min(sh - 1) + 1) * sw * 4];
        for (p, (x0, x1, wx)) in row.chunks_exact_mut(4).zip(&cols) {
            for c in 0..4 {
                let top = a[x0 + c] + (a[x1 + c] - a[x0 + c]) * wx;
                let bottom = b[x0 + c] + (b[x1 + c] - b[x0 + c]) * wx;
                p[c] = top + (bottom - top) * wy;
            }
        }
    });
    out
}

/// Side of a [`grain`] tile.
const TILE: usize = 256;

/// A tile of film grain for one frame: `TILE`×`TILE` values in −1..1, different for every
/// `seed` (which should change with the frame). Sampling it per pixel costs a few table reads
/// where smooth 3-D noise costs dozens of hashes; at grain sizes the repeat cannot be seen.
pub(super) fn grain(seed: u64) -> Vec<f32> {
    let mut state = seed ^ 0x9E37_79B9_7F4A_7C15;
    (0..TILE * TILE)
        .map(|_| {
            // splitmix64
            state = state.wrapping_add(0x9E37_79B9_7F4A_7C15);
            let mut z = state;
            z = (z ^ (z >> 30)).wrapping_mul(0xBF58_476D_1CE4_E5B9);
            z = (z ^ (z >> 27)).wrapping_mul(0x94D0_49BB_1331_11EB);
            z ^= z >> 31;
            // the sum of two uniform values: a soft bell around 0
            let (a, b) = ((z & 0xFFFF) as f32 / 65535.0, ((z >> 16) & 0xFFFF) as f32 / 65535.0);
            a + b - 1.0
        })
        .collect()
}

/// A [`grain`] tile read at (`u`, `v`) cells, bilinear, wrapping.
#[inline]
pub(super) fn grain_at(tile: &[f32], u: f32, v: f32) -> f32 {
    if tile.len() < TILE * TILE || !u.is_finite() || !v.is_finite() {
        return 0.0;
    }
    let (uf, vf) = (u.floor(), v.floor());
    let (fx, fy) = (u - uf, v - vf);
    let (x0, y0) = ((uf as i64).rem_euclid(TILE as i64) as usize, (vf as i64).rem_euclid(TILE as i64) as usize);
    let (x1, y1) = ((x0 + 1) % TILE, (y0 + 1) % TILE);
    let top = tile[y0 * TILE + x0] + (tile[y0 * TILE + x1] - tile[y0 * TILE + x0]) * fx;
    let bottom = tile[y1 * TILE + x0] + (tile[y1 * TILE + x1] - tile[y1 * TILE + x0]) * fx;
    top + (bottom - top) * fy
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_tables_agree_with_the_exact_curves() {
        let mut worst = (0.0f32, 0.0f32);
        for i in 0..=200_000 {
            let v = i as f32 / 200_000.0;
            worst.0 = worst.0.max((enc([v, v, v])[0] - linear_to_srgb(v)).abs());
            worst.1 = worst.1.max((dec([v, v, v])[0] - srgb_to_linear(v)).abs());
        }
        assert!(worst.0 < 2.5e-4 && worst.1 < 2.5e-4, "encode {} decode {}", worst.0, worst.1);
        // black and white are exact, the round trip is stable
        for (got, want) in enc([0.0, 1.0, 0.0]).into_iter().chain(dec([0.0, 1.0, 0.0])).zip([0.0, 1.0, 0.0, 0.0, 1.0, 0.0]) {
            assert!((got - want).abs() < 1e-6, "{got} {want}");
        }
        for v in [0.001f32, 0.02, 0.18, 0.5, 0.9] {
            assert!((dec(enc([v, v, v]))[0] - v).abs() < 3e-4, "{v}");
        }
        // brighter than white, negative and broken values
        assert!((enc([4.0, 4.0, 4.0])[0] - linear_to_srgb(4.0)).abs() < 1e-6);
        assert_eq!(enc([-1.0, f32::NAN, f32::NEG_INFINITY]), [0.0, 0.0, 0.0]);
        assert!(enc([f32::INFINITY, 0.0, 0.0])[0] == 0.0);
        assert!(dec([-1.0, f32::NAN, 7.0]).into_iter().zip([0.0, 0.0, 1.0]).all(|(a, b)| (a - b).abs() < 1e-6));
    }

    #[test]
    fn enlarging_keeps_flat_areas_and_blends_edges() {
        let mut src = Image::new(4, 2);
        for (i, p) in src.px.chunks_mut(4).enumerate() {
            let v = if i % 4 < 2 { 0.0 } else { 1.0 };
            p.copy_from_slice(&[v, v, v, 1.0]);
        }
        let big = enlarge(&src, 16, 8);
        assert_eq!((big.w, big.h), (16, 8));
        assert_eq!(big.get(0, 0)[0], 0.0);
        assert_eq!(big.get(15, 7)[0], 1.0);
        let mid = big.get(8, 4)[0];
        assert!(mid > 0.3 && mid < 0.9, "{mid}");
        assert!(big.px.chunks(4).all(|p| (p[3] - 1.0).abs() < 1e-6));
        // same size is the same picture; hostile sizes give an empty or black picture
        assert_eq!(enlarge(&src, 4, 2).px, src.px);
        assert_eq!(enlarge(&src, 0, 5).px.len(), 0);
        assert!(enlarge(&Image::new(0, 0), 3, 3).px.iter().all(|v| *v == 0.0));
        assert_eq!(enlarge(&src, 1, 1).px.len(), 4);
    }

    #[test]
    fn grain_is_centred_differs_per_frame_and_wraps() {
        let (a, b) = (grain(1), grain(2));
        assert_eq!(a.len(), TILE * TILE);
        assert_ne!(a, b);
        assert_eq!(a, grain(1));
        let mean = a.iter().sum::<f32>() / a.len() as f32;
        let spread = (a.iter().map(|v| v * v).sum::<f32>() / a.len() as f32).sqrt();
        assert!(mean.abs() < 0.02 && spread > 0.3 && spread < 0.5, "{mean} {spread}");
        assert!(a.iter().all(|v| (-1.0..=1.0).contains(v)));
        assert_eq!(grain_at(&a, 3.0, 5.0), a[5 * TILE + 3]);
        assert_eq!(grain_at(&a, 3.0 + TILE as f32, 5.0 - TILE as f32), a[5 * TILE + 3]);
        for (u, v) in [(f32::NAN, 0.0), (1e30, -1e30), (-0.5, 255.7), (f32::INFINITY, 1.0)] {
            assert!(grain_at(&a, u, v).is_finite());
        }
        assert_eq!(grain_at(&[], 1.0, 1.0), 0.0);
    }
}
