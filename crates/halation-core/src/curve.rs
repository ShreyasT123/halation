//! Tone curve model (Sprint 4): per-channel control points, monotone cubic (Fritsch–Carlson)
//! interpolation and the piecewise-polynomial form uploaded to the GPU.
//!
//! The curve is evaluated analytically — `a·x³ + b·x² + c·x + d` per segment — so there is no
//! 256-entry LUT and therefore no shadow quantisation. Curves live on a *perceptual* axis (sRGB
//! encoded), so mid-tones sit at 0.5 like every darkroom curve editor; the develop pipeline encodes,
//! evaluates and decodes around it.

use bytemuck::{Pod, Zeroable};
use serde::{Deserialize, Serialize};

/// Control points per channel (endpoints included).
pub const MAX_CURVE_POINTS: usize = 17;
/// Cubic segments per channel on the GPU.
pub const MAX_SEGMENTS: usize = MAX_CURVE_POINTS - 1;
/// Minimum horizontal distance between neighbouring control points.
pub const MIN_POINT_GAP: f32 = 0.01;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum CurveChannel {
    Rgb,
    Red,
    Green,
    Blue,
}

impl CurveChannel {
    pub const ALL: [CurveChannel; 4] = [CurveChannel::Rgb, CurveChannel::Red, CurveChannel::Green, CurveChannel::Blue];

    pub const fn label(self) -> &'static str {
        match self {
            CurveChannel::Rgb => "RGB",
            CurveChannel::Red => "Red",
            CurveChannel::Green => "Green",
            CurveChannel::Blue => "Blue",
        }
    }

    pub const fn index(self) -> usize {
        self as usize
    }
}

/// One cubic segment, valid for `x ∈ [domain.0, domain.1]`, polynomial in `x − domain.0`.
#[repr(C, align(16))]
#[derive(Debug, Clone, Copy, PartialEq, Pod, Zeroable, Serialize, Deserialize)]
pub struct SplineSegment {
    pub coeffs: [f32; 4], // a, b, c, d
    pub domain: [f32; 2], // x_min, x_max
    pub _pad: [f32; 2],
}

const _: () = assert!(std::mem::size_of::<SplineSegment>() == 32);

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct ToneCurve {
    pub rgb: Vec<[f32; 2]>,
    pub red: Vec<[f32; 2]>,
    pub green: Vec<[f32; 2]>,
    pub blue: Vec<[f32; 2]>,
}

fn identity() -> Vec<[f32; 2]> {
    vec![[0.0, 0.0], [1.0, 1.0]]
}

impl Default for ToneCurve {
    fn default() -> Self {
        Self { rgb: identity(), red: identity(), green: identity(), blue: identity() }
    }
}

fn is_identity(points: &[[f32; 2]]) -> bool {
    points.iter().all(|p| (p[0] - p[1]).abs() < 1.0e-5)
}

impl ToneCurve {
    pub fn is_identity(&self) -> bool {
        CurveChannel::ALL.iter().all(|&c| is_identity(self.points(c)))
    }

    pub fn points(&self, channel: CurveChannel) -> &[[f32; 2]] {
        match channel {
            CurveChannel::Rgb => &self.rgb,
            CurveChannel::Red => &self.red,
            CurveChannel::Green => &self.green,
            CurveChannel::Blue => &self.blue,
        }
    }

    fn points_mut(&mut self, channel: CurveChannel) -> &mut Vec<[f32; 2]> {
        match channel {
            CurveChannel::Rgb => &mut self.rgb,
            CurveChannel::Red => &mut self.red,
            CurveChannel::Green => &mut self.green,
            CurveChannel::Blue => &mut self.blue,
        }
    }

    pub fn reset_channel(&mut self, channel: CurveChannel) {
        *self.points_mut(channel) = identity();
    }

    /// Insert a point at `(x, y)`; returns its index, or `None` if the curve is full or too crowded.
    pub fn insert_point(&mut self, channel: CurveChannel, x: f32, y: f32) -> Option<usize> {
        let pts = self.points_mut(channel);
        if pts.len() >= MAX_CURVE_POINTS {
            return None;
        }
        let x = x.clamp(0.0, 1.0);
        let at = pts.iter().position(|p| p[0] > x).unwrap_or(pts.len());
        if at == 0 || at == pts.len() {
            return None;
        }
        if x - pts[at - 1][0] < MIN_POINT_GAP || pts[at][0] - x < MIN_POINT_GAP {
            return None;
        }
        pts.insert(at, [x, y.clamp(0.0, 1.0)]);
        Some(at)
    }

    /// Move point `index`; endpoints keep x = 0 / 1, interior points stay between their neighbours.
    pub fn move_point(&mut self, channel: CurveChannel, index: usize, x: f32, y: f32) {
        let pts = self.points_mut(channel);
        let last = pts.len() - 1;
        if index > last {
            return;
        }
        let y = y.clamp(0.0, 1.0);
        let x = if index == 0 {
            0.0
        } else if index == last {
            1.0
        } else {
            x.clamp(pts[index - 1][0] + MIN_POINT_GAP, pts[index + 1][0] - MIN_POINT_GAP)
        };
        pts[index] = [x, y];
    }

    /// Remove an interior point.
    pub fn remove_point(&mut self, channel: CurveChannel, index: usize) -> bool {
        let pts = self.points_mut(channel);
        if index == 0 || index + 1 >= pts.len() {
            return false;
        }
        pts.remove(index);
        true
    }

    /// Index of the point within `radius` (normalised units) of `(x, y)`.
    pub fn hit_test(&self, channel: CurveChannel, x: f32, y: f32, radius: f32) -> Option<usize> {
        self.points(channel)
            .iter()
            .enumerate()
            .map(|(i, p)| (i, (p[0] - x).hypot(p[1] - y)))
            .filter(|(_, d)| *d <= radius)
            .min_by(|a, b| a.1.total_cmp(&b.1))
            .map(|(i, _)| i)
    }

    pub fn evaluate(&self, channel: CurveChannel, x: f32) -> f32 {
        evaluate_points(self.points(channel), x)
    }

    pub fn segments(&self, channel: CurveChannel) -> Vec<SplineSegment> {
        build_segments(self.points(channel))
    }

    /// Flat GPU payload: `4 × MAX_SEGMENTS` segments (Rgb, Red, Green, Blue; unused slots zeroed)
    /// and the number of valid segments per channel.
    pub fn gpu_segments(&self) -> (Vec<SplineSegment>, [u32; 4]) {
        let mut flat = vec![SplineSegment::zeroed(); 4 * MAX_SEGMENTS];
        let mut counts = [0u32; 4];
        for channel in CurveChannel::ALL {
            let i = channel.index();
            if is_identity(self.points(channel)) {
                continue; // count 0 ⇒ shader skips the channel
            }
            let segs = self.segments(channel);
            counts[i] = segs.len().min(MAX_SEGMENTS) as u32;
            for (k, seg) in segs.into_iter().take(MAX_SEGMENTS).enumerate() {
                flat[i * MAX_SEGMENTS + k] = seg;
            }
        }
        (flat, counts)
    }

    pub fn sanitized(mut self) -> Self {
        for channel in CurveChannel::ALL {
            let pts = self.points_mut(channel);
            pts.retain(|p| p[0].is_finite() && p[1].is_finite());
            pts.iter_mut().for_each(|p| *p = [p[0].clamp(0.0, 1.0), p[1].clamp(0.0, 1.0)]);
            pts.sort_by(|a, b| a[0].total_cmp(&b[0]));
            pts.dedup_by(|b, a| (b[0] - a[0]).abs() < 1.0e-4);
            pts.truncate(MAX_CURVE_POINTS);
            if pts.len() < 2 || pts[0][0] > 0.0 || pts[pts.len() - 1][0] < 1.0 {
                *pts = identity();
            }
        }
        self
    }
}

/// Fritsch–Carlson monotone tangents for `points` (sorted by x, at least 2 points).
fn tangents(points: &[[f32; 2]]) -> Vec<f32> {
    let n = points.len();
    let h: Vec<f32> = points.windows(2).map(|w| (w[1][0] - w[0][0]).max(1.0e-6)).collect();
    let delta: Vec<f32> = points.windows(2).zip(&h).map(|(w, h)| (w[1][1] - w[0][1]) / h).collect();
    let mut m = vec![0.0f32; n];
    m[0] = delta[0];
    m[n - 1] = delta[n - 2];
    for k in 1..n - 1 {
        m[k] = if delta[k - 1] * delta[k] <= 0.0 { 0.0 } else { 0.5 * (delta[k - 1] + delta[k]) };
    }
    for k in 0..n - 1 {
        if delta[k] == 0.0 {
            m[k] = 0.0;
            m[k + 1] = 0.0;
            continue;
        }
        let (a, b) = (m[k] / delta[k], m[k + 1] / delta[k]);
        let s = a * a + b * b;
        if s > 9.0 {
            let tau = 3.0 / s.sqrt();
            m[k] = tau * a * delta[k];
            m[k + 1] = tau * b * delta[k];
        }
    }
    m
}

pub fn build_segments(points: &[[f32; 2]]) -> Vec<SplineSegment> {
    if points.len() < 2 {
        return Vec::new();
    }
    let m = tangents(points);
    points
        .windows(2)
        .enumerate()
        .map(|(k, w)| {
            let h = (w[1][0] - w[0][0]).max(1.0e-6);
            let delta = (w[1][1] - w[0][1]) / h;
            let a = (m[k] + m[k + 1] - 2.0 * delta) / (h * h);
            let b = (3.0 * delta - 2.0 * m[k] - m[k + 1]) / h;
            SplineSegment { coeffs: [a, b, m[k], w[0][1]], domain: [w[0][0], w[1][0]], _pad: [0.0; 2] }
        })
        .collect()
}

pub fn evaluate_points(points: &[[f32; 2]], x: f32) -> f32 {
    let segments = build_segments(points);
    evaluate_segments(&segments, x)
}

/// Same arithmetic as the WGSL `evaluate_curve`; out-of-range input passes through unchanged.
pub fn evaluate_segments(segments: &[SplineSegment], x: f32) -> f32 {
    if segments.is_empty() {
        return x;
    }
    let x = x.clamp(0.0, 1.0);
    for seg in segments {
        if x >= seg.domain[0] && x <= seg.domain[1] {
            let u = x - seg.domain[0];
            let [a, b, c, d] = seg.coeffs;
            return ((a * u + b) * u + c) * u + d;
        }
    }
    x
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn identity_is_exact() {
        let c = ToneCurve::default();
        assert!(c.is_identity());
        for i in 0..=20 {
            let x = i as f32 / 20.0;
            assert!((c.evaluate(CurveChannel::Rgb, x) - x).abs() < 1.0e-5);
        }
    }

    #[test]
    fn monotone_points_give_monotone_curve_without_overshoot() {
        let mut c = ToneCurve::default();
        c.insert_point(CurveChannel::Rgb, 0.25, 0.10).unwrap();
        c.insert_point(CurveChannel::Rgb, 0.75, 0.92).unwrap();
        let mut prev = -1.0;
        for i in 0..=1000 {
            let y = c.evaluate(CurveChannel::Rgb, i as f32 / 1000.0);
            assert!(y >= prev - 1.0e-5 && (-1.0e-4..=1.0001).contains(&y), "x={i} y={y}");
            prev = y;
        }
    }

    #[test]
    fn passes_through_control_points() {
        let mut c = ToneCurve::default();
        c.insert_point(CurveChannel::Green, 0.4, 0.55).unwrap();
        assert!((c.evaluate(CurveChannel::Green, 0.4) - 0.55).abs() < 1.0e-4);
        assert!(c.evaluate(CurveChannel::Green, 0.0).abs() < 1.0e-6 && (c.evaluate(CurveChannel::Green, 1.0) - 1.0).abs() < 1.0e-6);
    }

    #[test]
    fn editing_rules() {
        let mut c = ToneCurve::default();
        assert!(c.insert_point(CurveChannel::Rgb, 0.5, 0.5).is_some());
        assert!(c.insert_point(CurveChannel::Rgb, 0.505, 0.5).is_none());
        c.move_point(CurveChannel::Rgb, 1, 2.0, 0.4);
        assert!(c.rgb[1][0] < 1.0);
        assert!(!c.remove_point(CurveChannel::Rgb, 0));
        assert!(c.remove_point(CurveChannel::Rgb, 1));
        assert!(c.is_identity());
    }
}
