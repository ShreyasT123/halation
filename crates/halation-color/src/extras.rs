//! Tone curve, colour grading and local-adjustment stages of the reference develop pipeline
//! (Sprints 4 and 8). Mirrors the matching blocks of `assets/shaders/develop_pipeline.wgsl`.
//!
//! Needs `halation-core` as a path dependency of this crate.

use super::{
    contrast_gamma, srgb_eotf, srgb_oetf, tone_highlights_shadows, vibrance_saturation, DevelopInputs, PreparedDevelop,
    MID_GRAY, MIN_LUMA, PROPHOTO_LUMA,
};
use glam::Vec3;
use halation_core::curve::evaluate_segments;
use halation_core::{DevelopParameters, MaskBuffer, SplineSegment, MAX_SEGMENTS};

#[derive(Debug, Clone, Copy)]
struct LocalPrepared {
    gain: f32,
    contrast_gamma: f32,
    highlights_ev: f32,
    shadows_ev: f32,
    saturation: f32,
    temperature: f32,
    invert: bool,
}

#[derive(Debug, Clone)]
pub(crate) struct Extras {
    segments: Vec<SplineSegment>,
    counts: [u32; 4],
    grading: Option<[[f32; 4]; 4]>,
    local: Option<LocalPrepared>,
}

impl Extras {
    fn eval(&self, channel: usize, x: f32) -> f32 {
        let base = channel * MAX_SEGMENTS;
        evaluate_segments(&self.segments[base..base + self.counts[channel] as usize], x)
    }

    /// Curves act on the sRGB-encoded value of each working-space channel; values above 1.0 pass through.
    fn curve_channel(&self, v: f32, channel: usize) -> f32 {
        if v > 1.0 {
            return v;
        }
        let mut e = srgb_oetf(v.max(0.0));
        if self.counts[0] > 0 {
            e = self.eval(0, e);
        }
        if self.counts[channel] > 0 {
            e = self.eval(channel, e);
        }
        srgb_eotf(e)
    }

    pub(crate) fn curves_and_grading(&self, mut c: Vec3) -> Vec3 {
        if self.counts.iter().any(|&n| n > 0) {
            c = Vec3::new(self.curve_channel(c.x, 1), self.curve_channel(c.y, 2), self.curve_channel(c.z, 3));
        }
        if let Some(g) = &self.grading {
            c = grade(c, g);
        }
        c
    }

    pub(crate) fn local(&self, c: Vec3, mask: f32) -> Vec3 {
        let Some(l) = &self.local else { return c };
        let m = if l.invert { 1.0 - mask } else { mask };
        if m <= 0.0 {
            return c;
        }
        let mut a = c * l.gain;
        a = tone_highlights_shadows(a, l.highlights_ev, l.shadows_ev);
        if l.contrast_gamma != 1.0 {
            a = contrast_gamma(a, l.contrast_gamma);
        }
        if l.saturation != 0.0 {
            a = vibrance_saturation(a, 0.0, l.saturation);
        }
        if l.temperature != 0.0 {
            a *= Vec3::new(1.0 + 0.25 * l.temperature, 1.0, 1.0 - 0.25 * l.temperature);
        }
        c.lerp(a, m.min(1.0))
    }
}

/// Shadows / midtones / highlights weights from stops-from-gray, then a multiplicative gain per zone.
fn grade(c: Vec3, g: &[[f32; 4]; 4]) -> Vec3 {
    let l = c.dot(Vec3::from_array(PROPHOTO_LUMA));
    let (ws, wm, wh) = if l > MIN_LUMA {
        let s = (l / MID_GRAY).log2();
        let ws = 1.0 - smoothstep(-4.0, 0.0, s);
        let wh = smoothstep(0.0, 3.0, s);
        (ws, (1.0 - ws - wh).max(0.0), wh)
    } else {
        (1.0, 0.0, 0.0)
    };
    let term = |t: &[f32; 4], w: f32| (Vec3::new(t[0], t[1], t[2]) + Vec3::splat(t[3])) * w;
    let gain = Vec3::ONE + term(&g[0], ws) + term(&g[1], wm) + term(&g[2], wh) + term(&g[3], 1.0);
    (c * gain).max(Vec3::ZERO)
}

#[inline]
fn smoothstep(e0: f32, e1: f32, x: f32) -> f32 {
    let t = ((x - e0) / (e1 - e0)).clamp(0.0, 1.0);
    t * t * (3.0 - 2.0 * t)
}

/// Bilinear mask lookup at normalised image coordinates; returns 0..1.
pub fn sample_mask(mask: &MaskBuffer, u: f32, v: f32) -> f32 {
    if !mask.is_valid() {
        return 0.0;
    }
    let (w, h) = (mask.width as usize, mask.height as usize);
    let x = (u * w as f32 - 0.5).clamp(0.0, (w - 1) as f32);
    let y = (v * h as f32 - 0.5).clamp(0.0, (h - 1) as f32);
    let (x0, y0) = (x as usize, y as usize);
    let (x1, y1) = ((x0 + 1).min(w - 1), (y0 + 1).min(h - 1));
    let (fx, fy) = (x - x0 as f32, y - y0 as f32);
    let px = |xx: usize, yy: usize| mask.data[yy * w + xx] as f32 / 255.0;
    let top = px(x0, y0) + (px(x1, y0) - px(x0, y0)) * fx;
    let bottom = px(x0, y1) + (px(x1, y1) - px(x0, y1)) * fx;
    top + (bottom - top) * fy
}

impl PreparedDevelop {
    /// Everything the develop pipeline can do: sliders, tone curves, grading wheels and (when a mask
    /// raster is available) the local adjustment.
    pub fn from_params(p: &DevelopParameters, mask_present: bool) -> Self {
        let mut prepared = Self::new(&DevelopInputs {
            exposure: p.exposure,
            contrast: p.contrast,
            highlights: p.highlights,
            shadows: p.shadows,
            whites: p.whites,
            blacks: p.blacks,
            temperature: p.temperature,
            tint: p.tint,
            vibrance: p.vibrance,
            saturation: p.saturation,
        });
        let (segments, counts) = p.curve.gpu_segments();
        let grading = (!p.grading.is_neutral()).then(|| {
            [p.grading.shadows.gain_terms(), p.grading.midtones.gain_terms(), p.grading.highlights.gain_terms(), p.grading.global.gain_terms()]
        });
        let k = super::HIGHLIGHT_SHADOW_RANGE_EV / 100.0;
        let local = p.local.as_ref().filter(|l| mask_present && l.has_effect()).map(|l| LocalPrepared {
            gain: l.exposure.clamp(-5.0, 5.0).exp2(),
            contrast_gamma: 1.0 + l.contrast.clamp(-100.0, 100.0) * 0.005,
            highlights_ev: l.highlights.clamp(-100.0, 100.0) * k,
            shadows_ev: l.shadows.clamp(-100.0, 100.0) * k,
            saturation: l.saturation.clamp(-100.0, 100.0) * 0.01,
            temperature: l.temperature.clamp(-100.0, 100.0) * 0.01,
            invert: l.invert,
        });
        if counts.iter().any(|&n| n > 0) || grading.is_some() || local.is_some() {
            prepared.extras = Some(Box::new(Extras { segments, counts, grading, local }));
        }
        prepared
    }

    /// Whether a local adjustment will be applied (the CPU renderer then needs per-pixel mask values).
    pub fn has_local(&self) -> bool {
        self.extras.as_ref().is_some_and(|x| x.local.is_some())
    }

    /// Valid curve segments per channel (Rgb, Red, Green, Blue).
    pub fn curve_counts(&self) -> [u32; 4] {
        self.extras.as_ref().map_or([0; 4], |x| x.counts)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use halation_core::{CurveChannel, Wheel};

    #[test]
    fn identity_extras_are_skipped() {
        assert!(PreparedDevelop::from_params(&DevelopParameters::default(), true).extras.is_none());
    }

    #[test]
    fn curve_lifts_midtones() {
        let mut p = DevelopParameters::default();
        p.curve.insert_point(CurveChannel::Rgb, 0.5, 0.7);
        let lit = PreparedDevelop::from_params(&p, false).develop_srgb8([MID_GRAY; 3]);
        let base = PreparedDevelop::from_params(&DevelopParameters::default(), false).develop_srgb8([MID_GRAY; 3]);
        assert!(lit[0] > base[0] + 20);
    }

    #[test]
    fn warm_highlight_wheel_tints_bright_pixels_only() {
        let mut p = DevelopParameters::default();
        p.grading.highlights = Wheel { hue: 40.0, sat: 1.0, lum: 0.0 };
        let d = PreparedDevelop::from_params(&p, false);
        let bright = d.develop([1.0; 3]);
        let dark = d.develop([0.002; 3]);
        assert!(bright[0] > bright[2]);
        assert!((dark[0] - dark[2]).abs() < 0.002);
    }

    #[test]
    fn local_mask_blends() {
        let mut p = DevelopParameters::default();
        p.local = Some(halation_core::LocalAdjustment {
            source: Some(halation_core::MaskSource::Linear { start: [0.0, 0.0], end: [1.0, 1.0] }),
            exposure: 1.0,
            ..Default::default()
        });
        let d = PreparedDevelop::from_params(&p, true);
        let (off, on) = (d.develop_masked([0.18; 3], 0.0)[0], d.develop_masked([0.18; 3], 1.0)[0]);
        assert!(on > off * 1.5);
    }
}
