//! Local assisted culling (Sprint 6) — everything runs on-device, no network.
//!
//! Analysis produces raw [`CullMetrics`] once per photo; [`CullThresholds`] turn them into verdicts, so the
//! UI's threshold sliders re-filter the grid instantly without re-running any analysis.
//!
//! * Focus: variance of the Laplacian (global) and mean Sobel gradient of the strongest 10 % edge
//!   pixels (the in-focus subject boundaries), both on luma.
//! * Motion blur: anisotropy of the gradient structure tensor, weighted by how soft the image is.
//! * Technical rejects: clipped highlight / shadow fractions.
//! * Blinks: eye aspect ratio from six landmarks per eye supplied by an [`EyeDetector`]
//!   (`halation-ml` provides one backed by a face-landmark ONNX model; without a detector eye state
//!   is simply unknown and never causes a reject).

use halation_raw::decode_thumbnail;
use rayon::prelude::*;
use std::path::PathBuf;

/// Analysis resolution (longest edge).
pub const ANALYSIS_SIZE: u32 = 768;

/// Six landmarks per eye in the usual EAR order: outer corner, upper-outer, upper-inner, inner
/// corner, lower-inner, lower-outer (pixel coordinates).
pub type EyeLandmarks = [[f32; 2]; 6];

pub trait EyeDetector: Send + Sync {
    fn detect_eyes(&self, rgba: &[u8], width: u32, height: u32) -> Vec<EyeLandmarks>;
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct CullMetrics {
    /// Global focus score in 0..1 (log-scaled Laplacian variance).
    pub sharpness: f32,
    /// Edge-weighted focus score in 0..1 (strongest 10 % of gradients).
    pub subject_sharpness: f32,
    /// Directional-blur likelihood in 0..1.
    pub motion_blur: f32,
    pub clipped_highlights: f32,
    pub clipped_shadows: f32,
    /// Smallest eye aspect ratio found, `None` if no eyes were detected / no detector.
    pub eye_aspect_ratio: Option<f32>,
}

impl CullMetrics {
    pub fn focus(&self) -> f32 {
        0.5 * (self.sharpness + self.subject_sharpness)
    }

    pub fn verdict(&self, t: &CullThresholds) -> CullVerdict {
        if self.eye_aspect_ratio.is_some_and(|ear| ear < t.blink_ear) {
            return CullVerdict::Reject(CullReason::Blink);
        }
        if self.focus() < t.min_focus {
            return CullVerdict::Reject(CullReason::Soft);
        }
        if self.motion_blur > t.max_motion {
            return CullVerdict::Reject(CullReason::MotionBlur);
        }
        if self.clipped_highlights > t.max_clipped {
            return CullVerdict::Reject(CullReason::BlownHighlights);
        }
        if self.focus() >= t.select_focus && self.clipped_highlights <= t.max_clipped * 0.5 {
            return CullVerdict::Select;
        }
        CullVerdict::Neutral
    }
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct CullThresholds {
    pub min_focus: f32,
    pub select_focus: f32,
    pub max_motion: f32,
    pub max_clipped: f32,
    pub blink_ear: f32,
}

impl Default for CullThresholds {
    fn default() -> Self {
        Self { min_focus: 0.40, select_focus: 0.62, max_motion: 0.30, max_clipped: 0.08, blink_ear: 0.18 }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CullReason {
    Soft,
    MotionBlur,
    BlownHighlights,
    Blink,
}

impl CullReason {
    pub fn label(self) -> &'static str {
        match self {
            CullReason::Soft => "Out of focus",
            CullReason::MotionBlur => "Motion blur",
            CullReason::BlownHighlights => "Blown highlights",
            CullReason::Blink => "Eyes closed",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CullVerdict {
    Select,
    Neutral,
    Reject(CullReason),
}

/// `(|p2−p6| + |p3−p5|) / (2·|p1−p4|)`; open eyes ≈ 0.25–0.35, closed < 0.15.
pub fn eye_aspect_ratio(p: &EyeLandmarks) -> f32 {
    let d = |a: [f32; 2], b: [f32; 2]| (a[0] - b[0]).hypot(a[1] - b[1]);
    let width = d(p[0], p[3]);
    if width < 1.0e-6 {
        return 0.0;
    }
    (d(p[1], p[5]) + d(p[2], p[4])) / (2.0 * width)
}

fn luma_plane(rgba: &[u8]) -> Vec<f32> {
    rgba.chunks_exact(4).map(|p| 0.2126 * p[0] as f32 + 0.7152 * p[1] as f32 + 0.0722 * p[2] as f32).collect()
}

/// Analyse an 8-bit RGBA image (any size; [`ANALYSIS_SIZE`] is the intended scale).
pub fn analyze_rgba(rgba: &[u8], width: u32, height: u32, eyes: &[EyeLandmarks]) -> CullMetrics {
    let (w, h) = (width as usize, height as usize);
    let luma = luma_plane(rgba);
    let n = (w * h).max(1) as f32;
    let (mut hi, mut lo) = (0usize, 0usize);
    for &l in &luma {
        hi += (l >= 250.0) as usize;
        lo += (l <= 3.0) as usize;
    }

    let (mut lap_sum, mut lap_sq, mut count) = (0.0f64, 0.0f64, 0usize);
    let mut magnitudes = Vec::with_capacity(w * h);
    let (mut sxx, mut syy, mut sxy) = (0.0f64, 0.0f64, 0.0f64);
    if w >= 3 && h >= 3 {
        for y in 1..h - 1 {
            for x in 1..w - 1 {
                let at = |dx: isize, dy: isize| luma[(y as isize + dy) as usize * w + (x as isize + dx) as usize];
                let lap = at(-1, 0) + at(1, 0) + at(0, -1) + at(0, 1) - 4.0 * at(0, 0);
                let gx = (at(1, -1) + 2.0 * at(1, 0) + at(1, 1)) - (at(-1, -1) + 2.0 * at(-1, 0) + at(-1, 1));
                let gy = (at(-1, 1) + 2.0 * at(0, 1) + at(1, 1)) - (at(-1, -1) + 2.0 * at(0, -1) + at(1, -1));
                lap_sum += lap as f64;
                lap_sq += (lap as f64) * (lap as f64);
                count += 1;
                magnitudes.push((gx * gx + gy * gy).sqrt());
                sxx += (gx * gx) as f64;
                syy += (gy * gy) as f64;
                sxy += (gx * gy) as f64;
            }
        }
    }
    let count_f = count.max(1) as f64;
    let lap_var = (lap_sq / count_f - (lap_sum / count_f).powi(2)).max(0.0);
    let sharpness = (((1.0 + lap_var).ln() / 2001.0f64.ln()) as f32).clamp(0.0, 1.0);

    let subject_sharpness = if magnitudes.is_empty() {
        0.0
    } else {
        let top = (magnitudes.len() / 10).max(1);
        let split = magnitudes.len() - top;
        magnitudes.select_nth_unstable_by(split, |a, b| a.total_cmp(b));
        let mean_top = magnitudes[split..].iter().sum::<f32>() / top as f32;
        // Sobel response of a perfectly sharp 0→255 step is ~1020.
        (mean_top / 400.0).clamp(0.0, 1.0)
    };

    let trace = sxx + syy;
    let anisotropy = if trace > 1.0e-9 { (((sxx - syy).powi(2) + 4.0 * sxy * sxy).sqrt() / trace) as f32 } else { 0.0 };
    let softness = 1.0 - 0.5 * (sharpness + subject_sharpness);

    CullMetrics {
        sharpness,
        subject_sharpness,
        motion_blur: (anisotropy * softness * 1.6).clamp(0.0, 1.0),
        clipped_highlights: hi as f32 / n,
        clipped_shadows: lo as f32 / n,
        eye_aspect_ratio: eyes.iter().map(eye_aspect_ratio).min_by(|a, b| a.total_cmp(b)),
    }
}

/// Decode + analyse many files in parallel; order matches `paths`.
pub fn analyze_files(paths: &[PathBuf], detector: Option<&dyn EyeDetector>) -> Vec<(PathBuf, Result<CullMetrics, String>)> {
    paths
        .par_iter()
        .map(|path| {
            let result = decode_thumbnail(path, ANALYSIS_SIZE).map_err(|e| e.to_string()).map(|t| {
                let eyes = detector.map(|d| d.detect_eyes(&t.rgba, t.width, t.height)).unwrap_or_default();
                analyze_rgba(&t.rgba, t.width, t.height, &eyes)
            });
            (path.clone(), result)
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn checker(w: u32, h: u32, cell: u32) -> Vec<u8> {
        (0..w * h)
            .flat_map(|i| {
                let (x, y) = (i % w, i / w);
                let v = if ((x / cell) + (y / cell)) % 2 == 0 { 230u8 } else { 25 };
                [v, v, v, 255]
            })
            .collect()
    }

    fn box_blur(rgba: &[u8], w: usize, h: usize, r: usize) -> Vec<u8> {
        let mut out = rgba.to_vec();
        for y in 0..h {
            for x in 0..w {
                let (mut acc, mut n) = (0u32, 0u32);
                for yy in y.saturating_sub(r)..=(y + r).min(h - 1) {
                    for xx in x.saturating_sub(r)..=(x + r).min(w - 1) {
                        acc += rgba[(yy * w + xx) * 4] as u32;
                        n += 1;
                    }
                }
                let v = (acc / n) as u8;
                out[(y * w + x) * 4..(y * w + x) * 4 + 3].fill(v);
            }
        }
        out
    }

    #[test]
    fn blurred_image_scores_lower_and_is_rejected() {
        let sharp = checker(96, 96, 6);
        let soft = box_blur(&sharp, 96, 96, 8);
        let (a, b) = (analyze_rgba(&sharp, 96, 96, &[]), analyze_rgba(&soft, 96, 96, &[]));
        assert!(a.focus() > b.focus() + 0.2, "{a:?} {b:?}");
        let t = CullThresholds::default();
        assert_eq!(b.verdict(&t), CullVerdict::Reject(CullReason::Soft));
        assert_ne!(a.verdict(&t), CullVerdict::Reject(CullReason::Soft));
    }

    #[test]
    fn blink_detected_from_landmarks() {
        let open: EyeLandmarks = [[0.0, 0.0], [3.0, -3.0], [7.0, -3.0], [10.0, 0.0], [7.0, 3.0], [3.0, 3.0]];
        let closed: EyeLandmarks = [[0.0, 0.0], [3.0, -0.4], [7.0, -0.4], [10.0, 0.0], [7.0, 0.4], [3.0, 0.4]];
        assert!(eye_aspect_ratio(&open) > 0.25 && eye_aspect_ratio(&closed) < 0.1);
        let base = analyze_rgba(&checker(64, 64, 4), 64, 64, &[closed]);
        assert_eq!(base.verdict(&CullThresholds::default()), CullVerdict::Reject(CullReason::Blink));
    }

    #[test]
    fn blown_out_frames_are_flagged() {
        let white = vec![255u8; 64 * 64 * 4];
        assert!(analyze_rgba(&white, 64, 64, &[]).clipped_highlights > 0.99);
    }
}
