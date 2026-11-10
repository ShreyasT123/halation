//! Global-edit stages added on top of the base develop pipeline: Effects (texture, clarity, dehaze,
//! vignette, grain), Detail (sharpening), HSL colour mixer, B&W mixer, parametric curve â€” plus Auto
//! Tone and the white-balance eyedropper solver. Every function here has a line-for-line twin in
//! `assets/shaders/develop_pipeline.wgsl.txt`; keep them in sync.
//!
//! Neighbourhood stages (texture, clarity, sharpening, dehaze) read per-pixel *detail maps*
//! computed once per image by `halation-pipeline::detail`:
//! `(fine, coarse, sharp, dark)` = logâ‚‚-ratios of luminance to its blur at three radii and the
//! normalised dark channel.

use super::{srgb_eotf, srgb_oetf, white_balance_matrix, MID_GRAY, MIN_LUMA, PROPHOTO_LUMA};
use glam::{Mat3, Vec3};
use halation_core::{AdjustId, BAND_COUNT, BAND_HUES, DevelopParameters};

/// Per-pixel inputs beyond the colour itself.
#[derive(Debug, Clone, Copy)]
pub struct PixelContext {
    /// Local-adjustment mask value 0..1.
    pub mask: f32,
    /// `(fine, coarse, sharp, dark)` detail-map sample (zeros when no maps).
    pub detail: [f32; 4],
    /// Pixel coordinates (x, y) and image size (w, h) â€” for vignette and grain.
    pub px: [f32; 2],
    pub size: [f32; 2],
}

impl PixelContext {
    pub fn masked(mask: f32) -> Self {
        Self { mask, detail: [0.0; 4], px: [0.0; 2], size: [1.0; 2] }
    }
}

// ------------------------------------------------------------------------------------------------
// Helpers
// ------------------------------------------------------------------------------------------------

#[inline]
fn smoothstep(e0: f32, e1: f32, x: f32) -> f32 {
    let t = ((x - e0) / (e1 - e0)).clamp(0.0, 1.0);
    t * t * (3.0 - 2.0 * t)
}

#[inline]
fn luma(c: Vec3) -> f32 {
    c.dot(Vec3::from_array(PROPHOTO_LUMA))
}

pub fn rgb_to_hsl(c: [f32; 3]) -> (f32, f32, f32) {
    let (r, g, b) = (c[0], c[1], c[2]);
    let (mx, mn) = (r.max(g.max(b)), r.min(g.min(b)));
    let l = 0.5 * (mx + mn);
    let d = mx - mn;
    if d < 1.0e-6 {
        return (0.0, 0.0, l);
    }
    let s = d / (1.0 - (2.0 * l - 1.0).abs()).max(1.0e-6);
    let h = if mx == r {
        ((g - b) / d).rem_euclid(6.0)
    } else if mx == g {
        (b - r) / d + 2.0
    } else {
        (r - g) / d + 4.0
    };
    (h * 60.0, s.clamp(0.0, 1.0), l)
}

pub fn hsl_to_rgb(h: f32, s: f32, l: f32) -> [f32; 3] {
    let c = (1.0 - (2.0 * l - 1.0).abs()) * s;
    let hp = h.rem_euclid(360.0) / 60.0;
    let x = c * (1.0 - (hp.rem_euclid(2.0) - 1.0).abs());
    let (r, g, b) = match hp as u32 {
        0 => (c, x, 0.0),
        1 => (x, c, 0.0),
        2 => (0.0, c, x),
        3 => (0.0, x, c),
        4 => (x, 0.0, c),
        _ => (c, 0.0, x),
    };
    let m = l - 0.5 * c;
    [r + m, g + m, b + m]
}

/// Piecewise-linear band weights that partition the hue circle. Each band owns the arc to the next
/// centre (so at a centre only that band is active, and neighbouring bands cross-fade evenly).
pub fn band_weights(hue: f32) -> [f32; BAND_COUNT] {
    let mut w = [0.0f32; BAND_COUNT];
    let h = hue.rem_euclid(360.0);
    // As the left endpoint of arc `i`.
    for i in 0..BAND_COUNT {
        let lo = BAND_HUES[i];
        let hi = if i + 1 < BAND_COUNT { BAND_HUES[i + 1] } else { BAND_HUES[0] + 360.0 };
        let hh = if h < lo { h + 360.0 } else { h };
        if hh >= lo && hh < hi {
            w[i] += 1.0 - (hh - lo) / (hi - lo);
        }
    }
    // As the right endpoint of the previous arc.
    for i in 0..BAND_COUNT {
        let prev = (i + BAND_COUNT - 1) % BAND_COUNT;
        let lo = BAND_HUES[prev];
        let hi = if i == 0 { BAND_HUES[0] + 360.0 } else { BAND_HUES[i] };
        let hh = if h < lo { h + 360.0 } else { h };
        if hh >= lo && hh < hi {
            w[i] += (hh - lo) / (hi - lo);
        }
    }
    w
}

#[inline]
fn hash(mut x: u32) -> u32 {
    x ^= x >> 16;
    x = x.wrapping_mul(0x7feb_352d);
    x ^= x >> 15;
    x = x.wrapping_mul(0x846c_a68b);
    x ^= x >> 16;
    x
}

#[inline]
fn hash2(x: u32, y: u32) -> f32 {
    hash(x ^ hash(y.wrapping_add(0x9e37_79b9))) as f32 / u32::MAX as f32
}

// ------------------------------------------------------------------------------------------------
// Prepared stage state
// ------------------------------------------------------------------------------------------------

#[derive(Debug, Clone)]
pub(crate) struct AdjustPrepared {
    parametric: Option<([f32; 4], [f32; 3])>, // amounts /100, splits 0..1
    mixer: Option<([f32; 8], [f32; 8], [f32; 8])>,
    bw: Option<[f32; 8]>,
    texture: f32,
    clarity: f32,
    sharpen: f32,
    sharpen_detail: f32,
    sharpen_masking: f32,
    dehaze: f32,
    vignette: Option<[f32; 4]>, // amount, midpoint, roundness, feather (normalised)
    grain: Option<[f32; 3]>,    // amount, size, roughness
    pub(crate) air: Vec3,
    pub(crate) aspect: f32,
}

impl AdjustPrepared {
    pub(crate) fn new(p: &DevelopParameters) -> Option<Self> {
        let e = &p.effects;
        let sharpen = p.detail.sharpen.max(0.0);
        let any = !p.parametric.is_neutral()
            || !p.mixer.is_neutral()
            || p.bw.enabled
            || e.texture != 0.0
            || e.clarity != 0.0
            || e.dehaze != 0.0
            || sharpen > 0.0
            || e.vignette != 0.0
            || e.grain > 0.0;
        if !any {
            return None;
        }
        let band = |f: fn(&halation_core::BandAdjust) -> f32| -> [f32; 8] { std::array::from_fn(|i| f(&p.mixer.bands[i]) / 100.0) };
        Some(Self {
            parametric: (!p.parametric.is_neutral()).then(|| {
                (
                    [p.parametric.highlights / 100.0, p.parametric.lights / 100.0, p.parametric.darks / 100.0, p.parametric.shadows / 100.0],
                    [p.parametric.split[0] / 100.0, p.parametric.split[1] / 100.0, p.parametric.split[2] / 100.0],
                )
            }),
            mixer: (!p.mixer.is_neutral()).then(|| (band(|b| b.hue), band(|b| b.sat), band(|b| b.lum))),
            bw: p.bw.enabled.then(|| std::array::from_fn(|i| p.bw.mix[i] / 100.0)),
            texture: e.texture / 100.0 * 1.5,
            clarity: e.clarity / 100.0,
            sharpen: sharpen / 150.0 * 3.0,
            sharpen_detail: p.detail.detail / 100.0,
            sharpen_masking: p.detail.masking / 100.0,
            dehaze: e.dehaze / 100.0,
            vignette: (e.vignette != 0.0).then(|| [e.vignette / 100.0, e.vignette_midpoint / 100.0, e.vignette_roundness / 100.0, e.vignette_feather / 100.0]),
            grain: (e.grain > 0.0).then(|| [e.grain / 100.0, e.grain_size / 100.0, e.grain_roughness / 100.0]),
            air: Vec3::ZERO,
            aspect: 1.0,
        })
    }

    /// Whether any stage reads the per-pixel detail maps.
    pub(crate) fn needs_detail_map(&self) -> bool {
        self.texture != 0.0 || self.clarity != 0.0 || self.sharpen > 0.0 || self.dehaze != 0.0
    }

    /// Dehaze: remove (`> 0`) or add (`< 0`) atmospheric light using the normalised dark channel.
    pub(crate) fn dehaze(&self, c: Vec3, dark: f32) -> Vec3 {
        if self.dehaze == 0.0 {
            return c;
        }
        let a = self.air;
        if self.dehaze > 0.0 {
            let t = (1.0 - 0.95 * self.dehaze * dark.clamp(0.0, 1.0)).clamp(0.15, 1.0);
            ((c - a) / t + a).max(Vec3::ZERO)
        } else {
            let k = -self.dehaze * 0.6;
            c * (1.0 - k) + a * k
        }
    }

    /// Texture (fine), clarity (coarse, mid-tone weighted) and sharpening (radius map, edge-masked).
    pub(crate) fn detail(&self, c: Vec3, d: [f32; 4]) -> Vec3 {
        if self.texture == 0.0 && self.clarity == 0.0 && self.sharpen == 0.0 {
            return c;
        }
        let y = luma(c).max(MIN_LUMA);
        let mid = 4.0 * srgb_oetf(y.min(1.0)) * (1.0 - srgb_oetf(y.min(1.0)));
        let mut stops = self.texture * d[0] + self.clarity * d[1] * (0.35 + 0.65 * mid);
        if self.sharpen > 0.0 {
            let edge = smoothstep(0.02, 0.15, d[0].abs());
            let mask = 1.0 + (edge - 1.0) * self.sharpen_masking;
            let signal = d[2] + (d[0] - d[2]) * self.sharpen_detail * 0.6;
            stops += self.sharpen * signal * mask;
        }
        c * stops.clamp(-3.0, 3.0).exp2()
    }

    /// Parametric curve on the perceptual axis: smooth, monotone-safe region bumps.
    pub(crate) fn parametric(&self, c: Vec3) -> Vec3 {
        let Some((amt, s)) = &self.parametric else { return c };
        let f = |v: f32| -> f32 {
            if v > 1.0 {
                return v;
            }
            let e = srgb_oetf(v.max(0.0));
            let edges = [0.0, s[0], s[1], s[2], 1.0];
            // regions: shadows [0,s0], darks [s0,s1], lights [s1,s2], highlights [s2,1]
            let region_amounts = [amt[3], amt[2], amt[1], amt[0]];
            let mut shift = 0.0;
            for k in 0..4 {
                let (a, b) = (edges[k], edges[k + 1]);
                if e > a && e < b {
                    let t = (e - a) / (b - a);
                    let bump = 0.5 * (1.0 - (std::f32::consts::TAU * t).cos());
                    shift += region_amounts[k] * 0.3 * (b - a) * bump;
                }
            }
            srgb_eotf((e + shift).clamp(0.0, 1.0))
        };
        Vec3::new(f(c.x), f(c.y), f(c.z))
    }

    /// B&W conversion (band-weighted luminance) or HSL mixer, both on the sRGB-encoded value.
    pub(crate) fn mixer_bw(&self, c: Vec3) -> Vec3 {
        if self.mixer.is_none() && self.bw.is_none() {
            return c;
        }
        if c.max_element() > 1.0 {
            return c;
        }
        let e = Vec3::new(srgb_oetf(c.x.max(0.0)), srgb_oetf(c.y.max(0.0)), srgb_oetf(c.z.max(0.0)));
        let (h, s, l) = rgb_to_hsl(e.to_array());
        let w = band_weights(h);
        if let Some(mix) = &self.bw {
            let y = luma(c);
            let gain: f32 = (0..BAND_COUNT).map(|i| w[i] * mix[i]).sum();
            let g = (y * (1.0 + s * gain * 0.8).max(0.0)).max(0.0);
            return Vec3::splat(g);
        }
        let Some((hue, sat, lum)) = &self.mixer else { return c };
        let dh: f32 = (0..BAND_COUNT).map(|i| w[i] * hue[i]).sum::<f32>() * 30.0;
        let ds: f32 = (0..BAND_COUNT).map(|i| w[i] * sat[i]).sum();
        let dl: f32 = (0..BAND_COUNT).map(|i| w[i] * lum[i]).sum();
        let s2 = (s * (1.0 + ds)).clamp(0.0, 1.0);
        let l2 = (l + dl * 0.25 * s * (1.0 - (2.0 * l - 1.0).abs())).clamp(0.0, 1.0);
        let out = hsl_to_rgb(h + dh, s2, l2);
        Vec3::new(srgb_eotf(out[0].clamp(0.0, 1.0)), srgb_eotf(out[1].clamp(0.0, 1.0)), srgb_eotf(out[2].clamp(0.0, 1.0)))
    }

    /// Post-crop style vignette followed by luminance grain (perceptual-domain noise as a linear gain).
    pub(crate) fn vignette_grain(&self, mut c: Vec3, ctx: &PixelContext) -> Vec3 {
        if let Some([amount, midpoint, roundness, feather]) = self.vignette {
            let (u, v) = ((ctx.px[0] + 0.5) / ctx.size[0].max(1.0), (ctx.px[1] + 0.5) / ctx.size[1].max(1.0));
            let (mut x, mut y) = ((u - 0.5) * 2.0, (v - 0.5) * 2.0);
            let t = (roundness + 1.0) * 0.5;
            if self.aspect >= 1.0 {
                x *= 1.0 + (self.aspect - 1.0) * t;
            } else {
                y *= 1.0 + (1.0 / self.aspect - 1.0) * t;
            }
            let norm = (1.0 + (self.aspect.max(1.0 / self.aspect) - 1.0) * t).max(1.0);
            let d = (x * x + y * y).sqrt() / (std::f32::consts::SQRT_2 * norm);
            let inner = midpoint * 0.9;
            let mask = smoothstep(inner, inner + 0.1 + 0.9 * feather, d);
            c *= (2.0 * amount * mask).exp2();
        }
        if let Some([amount, size, roughness]) = self.grain {
            let size_px = 1.0 + size * 3.0;
            let white = hash2((ctx.px[0] / size_px) as u32, (ctx.px[1] / size_px) as u32) * 2.0 - 1.0;
            let (gx, gy) = (ctx.px[0] / (size_px * 2.0), ctx.px[1] / (size_px * 2.0));
            let (ix, iy) = (gx.floor(), gy.floor());
            let (fx, fy) = (smoothstep(0.0, 1.0, gx - ix), smoothstep(0.0, 1.0, gy - iy));
            let (x0, y0) = (ix as u32, iy as u32);
            let (a, b, cc, d) = (hash2(x0, y0), hash2(x0 + 1, y0), hash2(x0, y0 + 1), hash2(x0 + 1, y0 + 1));
            let smooth = ((a + (b - a) * fx) + ((cc + (d - cc) * fx) - (a + (b - a) * fx)) * fy) * 2.0 - 1.0;
            let n = smooth + (white - smooth) * roughness;
            let y = luma(c).max(MIN_LUMA);
            let e = srgb_oetf(y.min(1.0));
            let delta = n * amount * 0.08 * 4.0 * e * (1.0 - e);
            let gain = srgb_eotf((e + delta).clamp(0.0, 1.0)) / srgb_eotf(e).max(1.0e-6);
            c *= gain;
        }
        c
    }
}

// ------------------------------------------------------------------------------------------------
// Auto Tone
// ------------------------------------------------------------------------------------------------

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct AutoTone {
    pub exposure: f32,
    pub contrast: f32,
    pub highlights: f32,
    pub shadows: f32,
    pub whites: f32,
    pub blacks: f32,
}

/// Derive tone sliders from the linear luminance distribution (`luma` is consumed / reordered).
pub fn auto_tone(luma: &mut [f32]) -> AutoTone {
    let neutral = AutoTone { exposure: 0.0, contrast: 0.0, highlights: 0.0, shadows: 0.0, whites: 0.0, blacks: 0.0 };
    let n = luma.len();
    if n < 16 {
        return neutral;
    }
    let mut pick = |q: f32| -> f32 {
        let k = ((n - 1) as f32 * q) as usize;
        let (_, v, _) = luma.select_nth_unstable_by(k, |a, b| a.total_cmp(b));
        *v
    };
    let (p01, p50, p99) = (pick(0.01).max(1.0e-5), pick(0.5).max(1.0e-4), pick(0.99).max(1.0e-4));
    let exposure = ((MID_GRAY / p50).log2() * 0.7).clamp(-2.5, 2.5);
    let gain = exposure.exp2();
    let (hi, lo) = (p99 * gain, p01 * gain);
    // Highlight recovery looks at the raw clipped highlights: exposure compensation alone cannot
    // bring back detail that was already above white in the source.
    let highlights = if p99 > 0.9 { -(((p99 / 0.9).log2()) * 35.0).clamp(0.0, 50.0) } else { 0.0 };
    let whites = if hi < 0.8 { ((0.95 / hi).log2() * 30.0).clamp(0.0, 60.0) } else { 0.0 };
    let shadows = if lo < 0.02 { ((0.02 / lo).log2() * 14.0).clamp(0.0, 80.0) } else { 0.0 };
    let blacks = if lo > 0.004 { -(lo / 0.02 * 100.0).clamp(0.0, 60.0) } else { 0.0 };
    let range = (p99 / p01).log2();
    let contrast = if range < 6.0 { ((6.0 - range) * 8.0).clamp(0.0, 30.0) } else { 0.0 };
    AutoTone { exposure: (exposure * 100.0).round() / 100.0, contrast: contrast.round(), highlights: highlights.round(), shadows: shadows.round(), whites: whites.round(), blacks: blacks.round() }
}

// ------------------------------------------------------------------------------------------------
// White-balance eyedropper
// ------------------------------------------------------------------------------------------------

fn neutrality_error(m: &Mat3, rgb: Vec3) -> f32 {
    let c = (*m * rgb).max(Vec3::splat(1.0e-6));
    ((c.x / c.y) - 1.0).powi(2) + ((c.z / c.y) - 1.0).powi(2)
}

/// Temperature (K) and tint that render the sampled working-space colour neutral.
pub fn solve_white_balance(rgb: [f32; 3]) -> (f32, f32) {
    let v = Vec3::from_array(rgb).max(Vec3::splat(1.0e-6));
    let (mut best, mut best_t, mut best_tint) = (f32::MAX, 5500.0f32, 0.0f32);
    for ti in 0..56 {
        let t = 2000.0 * (25000.0f32 / 2000.0).powf(ti as f32 / 55.0);
        for tint_i in 0..31 {
            let tint = -150.0 + tint_i as f32 * 10.0;
            let e = neutrality_error(&white_balance_matrix(t, tint), v);
            if e < best {
                (best, best_t, best_tint) = (e, t, tint);
            }
        }
    }
    let (mut dt, mut dtint) = (best_t * 0.04, 5.0);
    for _ in 0..24 {
        for (t, tint) in [(best_t + dt, best_tint), (best_t - dt, best_tint), (best_t, best_tint + dtint), (best_t, best_tint - dtint)] {
            let (t, tint) = (t.clamp(2000.0, 25000.0), tint.clamp(-150.0, 150.0));
            let e = neutrality_error(&white_balance_matrix(t, tint), v);
            if e < best {
                (best, best_t, best_tint) = (e, t, tint);
            }
        }
        dt *= 0.7;
        dtint *= 0.7;
    }
    ((best_t / 10.0).round() * 10.0, best_tint.round())
}

/// Convenience for the UI: which scalars Auto Tone may touch.
pub const AUTO_TONE_IDS: [AdjustId; 0] = [];

#[cfg(test)]
mod tests {
    use super::*;

    fn prepared(p: &DevelopParameters) -> AdjustPrepared {
        AdjustPrepared::new(p).expect("active stage")
    }

    #[test]
    fn hsl_roundtrips() {
        for c in [[0.8, 0.2, 0.1], [0.1, 0.6, 0.9], [0.5, 0.5, 0.5], [0.0, 0.0, 0.0], [1.0, 1.0, 0.0]] {
            let (h, s, l) = rgb_to_hsl(c);
            let back = hsl_to_rgb(h, s, l);
            assert!((0..3).all(|k| (back[k] - c[k]).abs() < 1.0e-4), "{c:?} -> {back:?}");
        }
    }

    #[test]
    fn band_weights_cover_the_circle() {
        for h in (0..360).step_by(5) {
            let w = band_weights(h as f32);
            assert!((w.iter().sum::<f32>() - 1.0).abs() < 1.0e-4, "hue {h}");
        }
    }

    #[test]
    fn mixer_saturation_only_touches_its_band() {
        let mut p = DevelopParameters::default();
        AdjustId::MixSat(0).set(&mut p, -100.0); // red
        let a = prepared(&p);
        let red = a.mixer_bw(Vec3::new(0.6, 0.05, 0.05));
        let blue = a.mixer_bw(Vec3::new(0.05, 0.05, 0.6));
        assert!((red.x - red.y).abs() < 0.05, "red should be desaturated: {red:?}");
        assert!((blue.z - 0.6).abs() < 0.02 && (blue.x - 0.05).abs() < 0.02, "blue untouched: {blue:?}");
    }

    #[test]
    fn bw_conversion_is_neutral_and_band_weighted() {
        let mut p = DevelopParameters::default();
        p.bw.enabled = true;
        let a = prepared(&p);
        let g = a.mixer_bw(Vec3::new(0.7, 0.2, 0.1));
        assert!((g.x - g.y).abs() < 1.0e-6 && (g.y - g.z).abs() < 1.0e-6);
        AdjustId::BwMix(0).set(&mut p, 100.0);
        let brighter_red = prepared(&p).mixer_bw(Vec3::new(0.7, 0.2, 0.1));
        assert!(brighter_red.x > g.x);
    }

    #[test]
    fn parametric_curve_is_monotone_and_pins_the_ends() {
        let mut p = DevelopParameters::default();
        for id in AdjustId::PARAMETRIC {
            id.set(&mut p, -100.0);
        }
        let a = prepared(&p);
        assert!(a.parametric(Vec3::ZERO).x < 1.0e-6 && (a.parametric(Vec3::ONE).x - 1.0).abs() < 1.0e-6);
        let mut prev = -1.0;
        for i in 0..=400 {
            let v = srgb_eotf(i as f32 / 400.0);
            let out = a.parametric(Vec3::splat(v)).x;
            assert!(out >= prev - 1.0e-6);
            prev = out;
        }
    }

    #[test]
    fn dehaze_lifts_contrast_against_the_air_light() {
        let mut p = DevelopParameters::default();
        AdjustId::Dehaze.set(&mut p, 80.0);
        let mut a = prepared(&p);
        a.air = Vec3::splat(0.8);
        let hazy = Vec3::splat(0.5);
        assert!(a.dehaze(hazy, 0.6).x < hazy.x);
        AdjustId::Dehaze.set(&mut p, -80.0);
        let mut b = prepared(&p);
        b.air = Vec3::splat(0.8);
        assert!(b.dehaze(hazy, 0.6).x > hazy.x);
    }

    #[test]
    fn vignette_darkens_corners_not_centre_and_grain_is_deterministic() {
        let mut p = DevelopParameters::default();
        AdjustId::Vignette.set(&mut p, -80.0);
        AdjustId::Grain.set(&mut p, 60.0);
        let a = prepared(&p);
        let mk = |x: f32, y: f32| PixelContext { mask: 0.0, detail: [0.0; 4], px: [x, y], size: [100.0, 100.0] };
        let (centre, corner) = (a.vignette_grain(Vec3::splat(0.3), &mk(50.0, 50.0)), a.vignette_grain(Vec3::splat(0.3), &mk(0.0, 0.0)));
        assert!(corner.x < centre.x * 0.7);
        assert_eq!(a.vignette_grain(Vec3::splat(0.3), &mk(10.0, 20.0)), a.vignette_grain(Vec3::splat(0.3), &mk(10.0, 20.0)));
    }

    #[test]
    fn auto_tone_brightens_dark_frames_and_recovers_bright_ones() {
        let mut dark: Vec<f32> = (0..1000).map(|i| 0.005 + i as f32 * 0.00005).collect();
        let t = auto_tone(&mut dark);
        assert!(t.exposure > 0.5 && t.shadows > 0.0);
        let mut bright: Vec<f32> = (0..1000).map(|i| 0.4 + i as f32 * 0.002).collect();
        let t = auto_tone(&mut bright);
        assert!(t.exposure < 0.0 && t.highlights < 0.0);
    }

    #[test]
    fn eyedropper_neutralises_a_warm_cast() {
        let cast = [0.30, 0.22, 0.14];
        let (t, tint) = solve_white_balance(cast);
        let out = white_balance_matrix(t, tint) * Vec3::from_array(cast);
        assert!((out.x / out.y - 1.0).abs() < 0.04 && (out.z / out.y - 1.0).abs() < 0.04, "T={t} tint={tint} -> {out:?}");
        let (t0, tint0) = solve_white_balance([0.2, 0.2, 0.2]);
        assert!((t0 - 5500.0).abs() < 400.0 && tint0.abs() < 15.0, "{t0} {tint0}");
    }
}
