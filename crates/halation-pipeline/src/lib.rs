//! halation-pipeline — frame scheduler.
//!
//! `process_buffer` resolves the develop parameters into one `PreparedDevelop`, then renders the frame
//! on the GPU (the same maths, in WGSL) and falls back to a Rayon-parallel CPU render when no usable
//! adapter exists. The CPU path and the shader are driven by the same prepared constants, so both
//! produce the same picture.
//!
//! Cargo: `rayon`, `halation-core`, `halation-color`, `halation-gpu`, `halation-raw`.

use halation_color::{sample_mask, DevelopInputs, DisplayLut, DisplayProfile, PixelContext, PreparedDevelop};
use halation_core::{
    DetailMap, DevelopParameters, GpuAdjustUniforms, GpuDevelopMatrices, GpuDevelopUniforms, GpuExtraUniforms, GpuFrameInputs, GpuMat3x3,
    HistogramData, ImageBufferF32, LutBuffer, MaskBuffer,
};
use halation_gpu::GpuDevelopEngine;
use halation_raw::RawAsset;
use rayon::prelude::*;
use std::sync::Arc;

pub mod culling;
pub mod detail;
pub mod export;
pub mod external;
pub mod thumbs;

pub const HISTOGRAM_WIDTH: u32 = 512;
pub const HISTOGRAM_HEIGHT: u32 = 128;

pub struct PipelineOutput {
    pub width: u32,
    pub height: u32,
    /// sRGB-encoded 8-bit RGBA, row-major, `width * height * 4` bytes.
    pub display_rgba: Vec<u8>,
    pub histogram: HistogramData,
    /// `HISTOGRAM_WIDTH × HISTOGRAM_HEIGHT` RGBA bitmap of `histogram`.
    pub histogram_rgba: Vec<u8>,
    pub used_gpu: bool,
}

pub fn develop_inputs(p: &DevelopParameters) -> DevelopInputs {
    DevelopInputs {
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
    }
}

/// Monitor transform shared by both back ends: the LUT object drives the CPU path, the texel buffer
/// is uploaded once by the GPU engine (cached by `buffer.id`).
#[derive(Debug)]
pub struct DisplayTransform {
    pub lut: Arc<DisplayLut>,
    pub buffer: LutBuffer,
}

impl DisplayTransform {
    pub fn from_profile(profile: &DisplayProfile) -> Self {
        let lut = Arc::new(DisplayLut::build(profile));
        let buffer = LutBuffer::new(lut.size as u32, lut.rgba32f());
        Self { lut, buffer }
    }

    pub fn srgb() -> Self {
        Self::from_profile(&DisplayProfile::srgb())
    }

    pub fn description(&self) -> &str {
        &self.lut.description
    }
}

/// Everything besides image and parameters that influences a render.
#[derive(Clone)]
pub struct RenderContext {
    pub display: Option<Arc<DisplayTransform>>,
    pub mask: Option<Arc<MaskBuffer>>,
    pub dither: bool,
    /// Paint clipped shadows blue and clipped highlights red (histogram corner triangles / `J`).
    pub clip_overlay: bool,
}

impl Default for RenderContext {
    fn default() -> Self {
        Self { display: None, mask: None, dither: true, clip_overlay: false }
    }
}

fn gpu_matrices(prepared: &PreparedDevelop) -> GpuDevelopMatrices {
    GpuDevelopMatrices {
        white_balance: GpuMat3x3::from_columns(prepared.wb_columns()),
        to_display: GpuMat3x3::from_columns(prepared.display_columns()),
    }
}

pub fn process_frame(raw: &RawAsset, params: &DevelopParameters) -> PipelineOutput {
    process_buffer(&raw.buffer, params, None)
}

pub fn process_frame_with_gpu(raw: &RawAsset, params: &DevelopParameters, gpu: Option<&GpuDevelopEngine>) -> PipelineOutput {
    process_buffer(&raw.buffer, params, gpu)
}

pub fn process_buffer(buffer: &ImageBufferF32, params: &DevelopParameters, gpu: Option<&GpuDevelopEngine>) -> PipelineOutput {
    process_with_context(buffer, params, gpu, &RenderContext::default())
}

/// Full-featured render: sliders, tone curves, grading wheels, local mask and display LUT.
pub fn process_with_context(
    buffer: &ImageBufferF32,
    params: &DevelopParameters,
    gpu: Option<&GpuDevelopEngine>,
    ctx: &RenderContext,
) -> PipelineOutput {
    if !buffer.is_valid() {
        let display_rgba = vec![0, 0, 0, 255];
        let histogram = HistogramData::from_rgba8(&display_rgba);
        let histogram_rgba = render_histogram_bitmap(&histogram, HISTOGRAM_WIDTH, HISTOGRAM_HEIGHT);
        return PipelineOutput { width: 1, height: 1, display_rgba, histogram, histogram_rgba, used_gpu: false };
    }
    let mask = ctx.mask.as_deref().filter(|m| m.is_valid());
    let aspect = buffer.width as f32 / buffer.height.max(1) as f32;
    let mut prepared = PreparedDevelop::from_params(params, mask.is_some())
        .with_display_lut(ctx.display.as_ref().map(|d| d.lut.clone()));
    let detail = prepared.needs_detail_map().then(|| detail::build(buffer));
    let air = detail.as_ref().map_or([0.0; 3], |(_, air)| *air);
    prepared = prepared.with_detail_context(aspect, air);
    let detail_map = detail.as_ref().map(|(map, _)| map);
    let gpu_frame = gpu.and_then(|engine| {
        let (segments, counts) = params.curve.gpu_segments();
        let frame = GpuFrameInputs {
            uniforms: GpuDevelopUniforms::from_params(params, counts.iter().sum()),
            matrices: gpu_matrices(&prepared),
            extra: GpuExtraUniforms::build(params, counts, ctx.display.is_some(), ctx.dither, mask.is_some(), ctx.clip_overlay),
            adjust: GpuAdjustUniforms::build(params, aspect, air),
            segments: &segments,
            lut: ctx.display.as_deref().map(|d| &d.buffer),
            mask,
            detail: detail_map,
        };
        engine.process(buffer, &frame).ok()
    });
    let (display_rgba, used_gpu) = match gpu_frame {
        Some(rgba) => (rgba, true),
        None => (render_cpu(buffer, &prepared, mask, detail_map, ctx.clip_overlay), false),
    };
    let histogram = histogram_parallel(&display_rgba);
    let histogram_rgba = render_histogram_bitmap(&histogram, HISTOGRAM_WIDTH, HISTOGRAM_HEIGHT);
    PipelineOutput { width: buffer.width, height: buffer.height, display_rgba, histogram, histogram_rgba, used_gpu }
}

fn render_cpu(
    buffer: &ImageBufferF32,
    prepared: &PreparedDevelop,
    mask: Option<&MaskBuffer>,
    detail: Option<&DetailMap>,
    clip_overlay: bool,
) -> Vec<u8> {
    let (w, h) = (buffer.width as usize, buffer.height as usize);
    let mut rgba = vec![255u8; buffer.pixels.len() * 4];
    rgba.par_chunks_exact_mut(w * 4).zip(buffer.pixels.par_chunks_exact(w)).enumerate().for_each(|(y, (out_row, in_row))| {
        let v = (y as f32 + 0.5) / h as f32;
        for (x, (out, px)) in out_row.chunks_exact_mut(4).zip(in_row).enumerate() {
            let m = match mask {
                Some(mask) if prepared.has_local() => sample_mask(mask, (x as f32 + 0.5) / w as f32, v),
                _ => 0.0,
            };
            let ctx = PixelContext {
                mask: m,
                detail: detail.map_or([0.0; 4], |d| d.sample(x, y)),
                px: [x as f32, y as f32],
                size: [w as f32, h as f32],
            };
            let mut c = prepared.develop_srgb8_ctx(*px, &ctx);
            if clip_overlay {
                if c.iter().any(|&v| v >= 254) {
                    c = [255, 34, 34];
                } else if c.iter().any(|&v| v <= 1) {
                    c = [34, 85, 255];
                }
            }
            out[..3].copy_from_slice(&c);
        }
    });
    rgba
}

/// Copy the normalised rectangle `[x, y, w, h]` out of a full-resolution buffer (longest side capped
/// at `max_dim`, keeping the rectangle's centre). Returns the cropped buffer and the normalised
/// rectangle it actually covers, which the loupe uses to place it exactly over the preview.
pub fn crop_region(buffer: &ImageBufferF32, rect: [f32; 4], max_dim: u32) -> (ImageBufferF32, [f32; 4]) {
    let (w, h) = (buffer.width as f32, buffer.height as f32);
    let mut px = [rect[0] * w, rect[1] * h, rect[2] * w, rect[3] * h];
    let cap = max_dim.max(1) as f32;
    for k in 0..2 {
        if px[2 + k] > cap {
            px[k] += (px[2 + k] - cap) * 0.5;
            px[2 + k] = cap;
        }
    }
    let x0 = (px[0].floor().max(0.0) as u32).min(buffer.width.saturating_sub(1));
    let y0 = (px[1].floor().max(0.0) as u32).min(buffer.height.saturating_sub(1));
    let cw = ((px[2].ceil() as u32).max(1)).min(buffer.width - x0);
    let ch = ((px[3].ceil() as u32).max(1)).min(buffer.height - y0);
    let mut out = Vec::with_capacity((cw * ch) as usize);
    for y in y0..y0 + ch {
        let row = (y * buffer.width + x0) as usize;
        out.extend_from_slice(&buffer.pixels[row..row + cw as usize]);
    }
    let covered = [x0 as f32 / w, y0 as f32 / h, cw as f32 / w, ch as f32 / h];
    (ImageBufferF32::new(cw, ch, out), covered)
}

fn merge(mut a: HistogramData, b: HistogramData) -> HistogramData {
    for i in 0..256 {
        a.r[i] += b.r[i];
        a.g[i] += b.g[i];
        a.b[i] += b.b[i];
        a.luma[i] += b.luma[i];
    }
    a
}

fn histogram_parallel(rgba: &[u8]) -> HistogramData {
    rgba.par_chunks(1 << 18).map(HistogramData::from_rgba8).reduce(HistogramData::default, merge)
}

/// RGB histogram: additive per-channel fills (overlaps read as white), luminance outline, quarter
/// grid lines, and blue / red edge markers when shadows / highlights clip.
pub fn render_histogram_bitmap(hist: &HistogramData, width: u32, height: u32) -> Vec<u8> {
    let (w, h) = (width as usize, height as usize);
    let mut bmp = vec![0u8; w * h * 4];
    let peak = hist.peak() as f32;
    let column = |counts: &[u32; 256], x: usize| -> usize {
        let v = (counts[x * 256 / w] as f32 / peak).min(1.0).powf(0.7);
        (v * (h - 1) as f32) as usize
    };
    let (clip_lo, clip_hi) = (hist.clipped_shadows() > 0.001, hist.clipped_highlights() > 0.001);
    for x in 0..w {
        let (rh, gh, bh, lh) = (column(&hist.r, x), column(&hist.g, x), column(&hist.b, x), column(&hist.luma, x));
        let grid = x > 0 && x % (w / 4) == 0;
        for y in 0..h {
            let up = h - 1 - y;
            let mut px = if grid { [44i32; 3] } else { [28i32; 3] };
            if up <= rh {
                px[0] += 110;
            }
            if up <= gh {
                px[1] += 110;
            }
            if up <= bh {
                px[2] += 110;
            }
            if up == lh || up + 1 == lh {
                px = [215; 3];
            }
            if clip_lo && x < 3 {
                px = [34, 85, 255];
            }
            if clip_hi && x + 3 >= w {
                px = [255, 34, 34];
            }
            let i = (y * w + x) * 4;
            bmp[i] = px[0].clamp(0, 255) as u8;
            bmp[i + 1] = px[1].clamp(0, 255) as u8;
            bmp[i + 2] = px[2].clamp(0, 255) as u8;
            bmp[i + 3] = 255;
        }
    }
    bmp
}

/// Area-averaged (box filter) downscale in linear light so the interactive preview stays photometrically
/// correct. Buffers already within `max_dim` are returned unchanged.
pub fn make_preview(buffer: &ImageBufferF32, max_dim: u32) -> ImageBufferF32 {
    let (w, h) = (buffer.width as u64, buffer.height as u64);
    let longest = w.max(h);
    if longest <= max_dim as u64 || !buffer.is_valid() {
        return buffer.clone();
    }
    let scale = longest as f64 / max_dim as f64;
    let tw = ((w as f64 / scale).round() as u64).max(1);
    let th = ((h as f64 / scale).round() as u64).max(1);
    let mut out = vec![[0.0f32; 3]; (tw * th) as usize];
    out.par_chunks_mut(tw as usize).enumerate().for_each(|(oy, row)| {
        let y0 = (oy as u64 * h / th) as usize;
        let y1 = (((oy as u64 + 1) * h).div_ceil(th) as usize).clamp(y0 + 1, h as usize);
        for (ox, px) in row.iter_mut().enumerate() {
            let x0 = (ox as u64 * w / tw) as usize;
            let x1 = (((ox as u64 + 1) * w).div_ceil(tw) as usize).clamp(x0 + 1, w as usize);
            let mut acc = [0.0f32; 3];
            for sy in y0..y1 {
                let base = sy * w as usize;
                for p in &buffer.pixels[base + x0..base + x1] {
                    acc[0] += p[0];
                    acc[1] += p[1];
                    acc[2] += p[2];
                }
            }
            let n = ((y1 - y0) * (x1 - x0)) as f32;
            *px = [acc[0] / n, acc[1] / n, acc[2] / n];
        }
    });
    ImageBufferF32::new(tw as u32, th as u32, out)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn preview_preserves_mean_and_bounds() {
        let src = ImageBufferF32::new(400, 200, vec![[0.25, 0.5, 0.75]; 400 * 200]);
        let p = make_preview(&src, 100);
        assert_eq!((p.width, p.height), (100, 50));
        assert!(p.pixels.iter().all(|px| (px[1] - 0.5).abs() < 1.0e-6));
    }

    #[test]
    fn cpu_default_render_is_display_transform_only() {
        let src = ImageBufferF32::new(2, 1, vec![[0.18; 3], [1.0; 3]]);
        let out = process_buffer(&src, &DevelopParameters::default(), None);
        assert_eq!(&out.display_rgba[4..7], &[255, 255, 255]);
        assert!((117..=119).contains(&out.display_rgba[0]));
        assert!(!out.used_gpu);
    }
}
