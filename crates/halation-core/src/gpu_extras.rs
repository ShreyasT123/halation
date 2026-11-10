//! Second uniform block uploaded next to `GpuDevelopUniforms` (binding 5 in the develop shader):
//! tone-curve segment counts, colour-grading gain terms, the local-adjustment parameters and the
//! display-stage switches. 128 bytes, std140-safe (everything is `vec4`).

use crate::{DevelopParameters, GpuDevelopMatrices, GpuDevelopUniforms, LocalAdjustment, MaskBuffer, SplineSegment};
use bytemuck::{Pod, Zeroable};
use std::sync::atomic::{AtomicU64, Ordering};

#[repr(C, align(16))]
#[derive(Debug, Clone, Copy, PartialEq, Pod, Zeroable)]
pub struct GpuExtraUniforms {
    /// Valid cubic segments per channel: Rgb, Red, Green, Blue (0 ⇒ channel skipped).
    pub curve_counts: [u32; 4],
    /// `(Δr, Δg, Δb, Δlum)` gain terms per zone, see `Wheel::gain_terms`.
    pub shadows: [f32; 4],
    pub midtones: [f32; 4],
    pub highlights: [f32; 4],
    pub global: [f32; 4],
    /// Local adjustment: exposure EV, contrast, highlights, shadows (slider units).
    pub local_a: [f32; 4],
    /// Local adjustment: saturation, temperature shift, mask enabled (0/1), mask inverted (0/1).
    pub local_b: [f32; 4],
    /// `[display LUT enabled, dither enabled, grading enabled, 0]`.
    pub flags: [u32; 4],
    /// `[blending / 100, balance / 100, 0, 0]` — 3-way grading zone overlap.
    pub grading: [f32; 4],
}

const _: () = assert!(std::mem::size_of::<GpuExtraUniforms>() == 144);

impl GpuExtraUniforms {
    pub fn build(
        params: &DevelopParameters,
        curve_counts: [u32; 4],
        lut_enabled: bool,
        dither: bool,
        mask_present: bool,
        clip_overlay: bool,
    ) -> Self {
        let local: Option<&LocalAdjustment> = params.local.as_ref().filter(|l| l.has_effect() && mask_present);
        let (local_a, local_b) = match local {
            Some(l) => (
                [l.exposure, l.contrast, l.highlights, l.shadows],
                [l.saturation, l.temperature, 1.0, if l.invert { 1.0 } else { 0.0 }],
            ),
            None => ([0.0; 4], [0.0; 4]),
        };
        let g = &params.grading;
        Self {
            curve_counts,
            shadows: g.shadows.gain_terms(),
            midtones: g.midtones.gain_terms(),
            highlights: g.highlights.gain_terms(),
            global: g.global.gain_terms(),
            local_a,
            local_b,
            flags: [lut_enabled as u32, dither as u32, (!g.is_neutral()) as u32, clip_overlay as u32],
            grading: [g.blending / 100.0, g.balance / 100.0, 0.0, 0.0],
        }
    }
}

static NEXT_LUT_ID: AtomicU64 = AtomicU64::new(1);

/// A 3D display LUT as RGBA32F texels (`size³`, index `r + size·(g + size·b)`).
/// `id` identifies the content so the GPU backend can cache the uploaded texture.
#[derive(Debug, Clone)]
pub struct LutBuffer {
    pub id: u64,
    pub size: u32,
    pub rgba32f: Vec<f32>,
}

impl LutBuffer {
    pub fn new(size: u32, rgba32f: Vec<f32>) -> Self {
        Self { id: NEXT_LUT_ID.fetch_add(1, Ordering::Relaxed), size, rgba32f }
    }

    pub fn is_valid(&self) -> bool {
        self.size >= 2 && self.rgba32f.len() == (self.size as usize).pow(3) * 4
    }
}

static NEXT_DETAIL_ID: AtomicU64 = AtomicU64::new(1);

/// Per-pixel neighbourhood maps consumed by the texture / clarity / sharpening / dehaze stages:
/// `(fine, coarse, sharp, dark)` — log₂ luminance ratios to blurs at three radii plus the normalised
/// dark channel. `id` identifies the content so the GPU backend can cache the uploaded texture.
#[derive(Debug, Clone)]
pub struct DetailMap {
    pub id: u64,
    pub width: u32,
    pub height: u32,
    pub data: Vec<[f32; 4]>,
}

impl DetailMap {
    pub fn new(width: u32, height: u32, data: Vec<[f32; 4]>) -> Self {
        Self { id: NEXT_DETAIL_ID.fetch_add(1, Ordering::Relaxed), width, height, data }
    }

    pub fn is_valid(&self) -> bool {
        self.width > 0 && self.height > 0 && self.data.len() == self.width as usize * self.height as usize
    }

    #[inline]
    pub fn sample(&self, x: usize, y: usize) -> [f32; 4] {
        if !self.is_valid() {
            return [0.0; 4];
        }
        self.data[y.min(self.height as usize - 1) * self.width as usize + x.min(self.width as usize - 1)]
    }
}

/// Everything the develop shader needs besides the image itself.
pub struct GpuFrameInputs<'a> {
    pub uniforms: GpuDevelopUniforms,
    pub matrices: GpuDevelopMatrices,
    pub extra: GpuExtraUniforms,
    pub adjust: crate::GpuAdjustUniforms,
    /// `4 × MAX_SEGMENTS` spline segments (see `ToneCurve::gpu_segments`).
    pub segments: &'a [SplineSegment],
    pub lut: Option<&'a LutBuffer>,
    pub mask: Option<&'a MaskBuffer>,
    pub detail: Option<&'a DetailMap>,
}
