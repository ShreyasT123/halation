//! 3-way colour grading (Sprint 4) and local adjustments / masks (Sprint 8) data model.

use serde::{Deserialize, Serialize};
use std::sync::atomic::{AtomicU64, Ordering};

const PROPHOTO_LUMA: [f32; 3] = [0.28804, 0.71187, 0.00009];

// ------------------------------------------------------------------------------------------------
// Colour wheels
// ------------------------------------------------------------------------------------------------

/// One colour wheel: puck angle = hue, puck radius = saturation, side slider = luminance.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct Wheel {
    pub hue: f32, // 0 ..= 360 degrees
    pub sat: f32, // 0 ..= 1
    pub lum: f32, // -1 ..= 1
}

impl Default for Wheel {
    fn default() -> Self {
        Self { hue: 0.0, sat: 0.0, lum: 0.0 }
    }
}

impl Wheel {
    pub fn is_neutral(&self) -> bool {
        self.sat.abs() < 1.0e-4 && self.lum.abs() < 1.0e-4
    }

    /// Puck position on the unit disc (x right, y up), `None` when neutral.
    pub fn puck(&self) -> [f32; 2] {
        let a = self.hue.to_radians();
        [a.cos() * self.sat, a.sin() * self.sat]
    }

    pub fn set_from_puck(&mut self, x: f32, y: f32) {
        let r = x.hypot(y);
        self.sat = r.min(1.0);
        if r > 1.0e-4 {
            self.hue = y.atan2(x).to_degrees().rem_euclid(360.0);
        }
    }

    /// `(Î”r, Î”g, Î”b, Î”lum)` gain terms: zero-luma chroma offset of the hue scaled by saturation,
    /// plus the luminance lift. The shader applies `c *= 1 + weight * (rgb + w)`.
    pub fn gain_terms(&self) -> [f32; 4] {
        let [r, g, b] = hue_to_rgb(self.hue);
        let l = r * PROPHOTO_LUMA[0] + g * PROPHOTO_LUMA[1] + b * PROPHOTO_LUMA[2];
        let k = self.sat.clamp(0.0, 1.0) * 0.5;
        [(r - l) * k, (g - l) * k, (b - l) * k, self.lum.clamp(-1.0, 1.0) * 0.5]
    }
}

/// Fully saturated RGB (max = 1, min = 0) for a hue in degrees.
pub fn hue_to_rgb(hue: f32) -> [f32; 3] {
    let h = hue.rem_euclid(360.0) / 60.0;
    let f = |n: f32| {
        let k = (n + h).rem_euclid(6.0);
        (k.min(4.0 - k)).clamp(0.0, 1.0)
    };
    [f(5.0), f(3.0), f(1.0)].map(|v| 1.0 - v)
}

#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct ColorGrading {
    pub shadows: Wheel,
    pub midtones: Wheel,
    pub highlights: Wheel,
    pub global: Wheel,
    /// How far the zones overlap, 0 ..= 100 (default 50).
    pub blending: f32,
    /// Shifts the shadow / highlight boundary, -100 ..= 100 (positive favours highlights).
    pub balance: f32,
}

impl Default for ColorGrading {
    fn default() -> Self {
        Self {
            shadows: Wheel::default(),
            midtones: Wheel::default(),
            highlights: Wheel::default(),
            global: Wheel::default(),
            blending: 50.0,
            balance: 0.0,
        }
    }
}

impl ColorGrading {
    pub fn is_neutral(&self) -> bool {
        self.shadows.is_neutral() && self.midtones.is_neutral() && self.highlights.is_neutral() && self.global.is_neutral()
    }

    pub fn sanitized(mut self) -> Self {
        for w in [&mut self.shadows, &mut self.midtones, &mut self.highlights, &mut self.global] {
            w.hue = if w.hue.is_finite() { w.hue.rem_euclid(360.0) } else { 0.0 };
            w.sat = if w.sat.is_finite() { w.sat.clamp(0.0, 1.0) } else { 0.0 };
            w.lum = if w.lum.is_finite() { w.lum.clamp(-1.0, 1.0) } else { 0.0 };
        }
        self.blending = if self.blending.is_finite() { self.blending.clamp(0.0, 100.0) } else { 50.0 };
        self.balance = if self.balance.is_finite() { self.balance.clamp(-100.0, 100.0) } else { 0.0 };
        self
    }

    /// Labels of wheels that differ from `previous`, for history entries.
    pub fn change_labels(&self, previous: &Self) -> Vec<String> {
        [
            ("Shadows wheel", self.shadows != previous.shadows),
            ("Midtones wheel", self.midtones != previous.midtones),
            ("Highlights wheel", self.highlights != previous.highlights),
            ("Global wheel", self.global != previous.global),
        ]
        .into_iter()
        .filter_map(|(label, changed)| changed.then(|| label.to_owned()))
        .collect()
    }
}

// ------------------------------------------------------------------------------------------------
// Local adjustments & masks
// ------------------------------------------------------------------------------------------------

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum AiMaskKind {
    Subject,
    Sky,
}

/// How a mask is produced. AI masks store their prompt points so the raster can be regenerated.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub enum MaskSource {
    Ai { kind: AiMaskKind, points: Vec<[f32; 3]> }, // (x, y, label) in normalised image space; label 1 = foreground
    Linear { start: [f32; 2], end: [f32; 2] },
    Radial { center: [f32; 2], radius: [f32; 2], feather: f32 },
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct LocalAdjustment {
    pub name: String,
    pub enabled: bool,
    pub source: Option<MaskSource>,
    pub invert: bool,
    pub exposure: f32,    // -5 ..= 5 EV
    pub contrast: f32,    // -100 ..= 100
    pub highlights: f32,  // -100 ..= 100
    pub shadows: f32,     // -100 ..= 100
    pub saturation: f32,  // -100 ..= 100
    pub temperature: f32, // -100 (cooler) ..= 100 (warmer)
}

impl Default for LocalAdjustment {
    fn default() -> Self {
        Self {
            name: "Mask 1".into(),
            enabled: true,
            source: None,
            invert: false,
            exposure: 0.0,
            contrast: 0.0,
            highlights: 0.0,
            shadows: 0.0,
            saturation: 0.0,
            temperature: 0.0,
        }
    }
}

impl LocalAdjustment {
    pub fn has_effect(&self) -> bool {
        self.enabled
            && self.source.is_some()
            && [self.exposure, self.contrast, self.highlights, self.shadows, self.saturation, self.temperature]
                .iter()
                .any(|v| v.abs() > 1.0e-4)
    }

    pub fn sanitized(mut self) -> Self {
        let clamp = |v: f32, lo: f32, hi: f32| if v.is_finite() { v.clamp(lo, hi) } else { 0.0 };
        self.exposure = clamp(self.exposure, -5.0, 5.0);
        for v in [&mut self.contrast, &mut self.highlights, &mut self.shadows, &mut self.saturation, &mut self.temperature] {
            *v = clamp(*v, -100.0, 100.0);
        }
        self
    }
}

static NEXT_MASK_ID: AtomicU64 = AtomicU64::new(1);

/// 8-bit single-channel mask raster in normalised image space (0 = untouched, 255 = fully affected).
/// `id` identifies the content so GPU backends can cache the uploaded texture.
#[derive(Debug, Clone)]
pub struct MaskBuffer {
    pub id: u64,
    pub width: u32,
    pub height: u32,
    pub data: Vec<u8>,
}

impl MaskBuffer {
    pub fn new(width: u32, height: u32, data: Vec<u8>) -> Self {
        Self { id: NEXT_MASK_ID.fetch_add(1, Ordering::Relaxed), width, height, data }
    }

    pub fn is_valid(&self) -> bool {
        self.width > 0 && self.height > 0 && self.data.len() == self.width as usize * self.height as usize
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn hue_wheel_primaries() {
        let close = |a: [f32; 3], b: [f32; 3]| a.iter().zip(b).all(|(x, y)| (x - y).abs() < 1.0e-4);
        assert!(close(hue_to_rgb(0.0), [1.0, 0.0, 0.0]));
        assert!(close(hue_to_rgb(120.0), [0.0, 1.0, 0.0]));
        assert!(close(hue_to_rgb(240.0), [0.0, 0.0, 1.0]));
        assert!(close(hue_to_rgb(60.0), [1.0, 1.0, 0.0]));
    }

    #[test]
    fn puck_roundtrip() {
        let mut w = Wheel::default();
        w.set_from_puck(0.0, 0.5);
        assert!((w.hue - 90.0).abs() < 1.0e-3 && (w.sat - 0.5).abs() < 1.0e-6);
        let p = w.puck();
        assert!(p[0].abs() < 1.0e-5 && (p[1] - 0.5).abs() < 1.0e-5);
    }

    #[test]
    fn grading_terms_have_zero_luma_chroma() {
        let t = Wheel { hue: 30.0, sat: 1.0, lum: 0.0 }.gain_terms();
        let l = t[0] * PROPHOTO_LUMA[0] + t[1] * PROPHOTO_LUMA[1] + t[2] * PROPHOTO_LUMA[2];
        assert!(l.abs() < 1.0e-5);
    }
}
