//! Per-image neighbourhood maps for the creative detail stages.
//!
//! The CPU (`halation-color::adjust`) and GPU (`develop_pipeline.wgsl`) stages both consume the same
//! RGBA texture: `(fine, coarse, sharp, dark)`, where the first three are log₂ luminance ratios to
//! blurs at three radii and `dark` is the normalised dark channel used by dehaze.

use halation_core::{DetailMap, ImageBufferF32};
use rayon::prelude::*;

#[inline]
fn luma(px: &[f32; 3]) -> f32 {
    px[0] * 0.28804 + px[1] * 0.71187 + px[2] * 0.00009
}

/// Separable box blur with a sliding prefix sum (`O(w·h)`), edge-clamped windows.
fn box_blur(src: &[f32], w: usize, h: usize, radius: usize) -> Vec<f32> {
    if radius == 0 || w == 0 || h == 0 {
        return src.to_vec();
    }
    let mut tmp = vec![0.0f32; w * h];
    tmp.par_chunks_mut(w).enumerate().for_each(|(y, row)| {
        let mut prefix = vec![0.0f64; w + 1];
        for x in 0..w {
            prefix[x + 1] = prefix[x] + src[y * w + x] as f64;
        }
        for (x, out) in row.iter_mut().enumerate() {
            let x0 = x.saturating_sub(radius);
            let x1 = (x + radius + 1).min(w);
            *out = ((prefix[x1] - prefix[x0]) / (x1 - x0) as f64) as f32;
        }
    });

    let mut out = vec![0.0f32; w * h];
    out.par_chunks_mut(w).enumerate().for_each(|(y, row)| {
        let y0 = y.saturating_sub(radius);
        let y1 = (y + radius + 1).min(h);
        for (x, o) in row.iter_mut().enumerate() {
            let mut acc = 0.0f64;
            for sy in y0..y1 {
                acc += tmp[sy * w + x] as f64;
            }
            *o = (acc / (y1 - y0) as f64) as f32;
        }
    });
    out
}

/// Estimate the atmospheric light: mean RGB of the brightest 0.1 % of pixels by dark channel.
fn atmospheric_light(pixels: &[[f32; 3]], dark: &[f32]) -> [f32; 3] {
    if pixels.is_empty() {
        return [0.0; 3];
    }
    let mut bins = [0u32; 256];
    for &d in dark {
        bins[(d.clamp(0.0, 1.0) * 255.0) as usize] += 1;
    }
    let target = ((pixels.len() as f32 * 0.001).ceil() as u32).max(1);
    let mut seen = 0u32;
    let mut threshold = 1.0f32;
    for i in (0..256).rev() {
        seen += bins[i];
        if seen >= target {
            threshold = i as f32 / 255.0;
            break;
        }
    }
    let (mut sum, mut count) = ([0.0f64; 3], 0u32);
    for (px, &d) in pixels.iter().zip(dark) {
        if d >= threshold {
            sum[0] += px[0] as f64;
            sum[1] += px[1] as f64;
            sum[2] += px[2] as f64;
            count += 1;
        }
    }
    if count == 0 {
        return [0.0; 3];
    }
    let inv = 1.0 / count as f64;
    [(sum[0] * inv) as f32, (sum[1] * inv) as f32, (sum[2] * inv) as f32]
}

/// Build the detail maps (and atmospheric-light estimate) for `buffer`.
pub fn build(buffer: &ImageBufferF32) -> (DetailMap, [f32; 3]) {
    let (w, h) = (buffer.width as usize, buffer.height as usize);
    if !buffer.is_valid() || w == 0 || h == 0 {
        return (DetailMap::new(1, 1, vec![[0.0; 4]]), [0.0; 3]);
    }
    let mut lum = vec![0.0f32; w * h];
    let mut dark = vec![0.0f32; w * h];
    lum.par_iter_mut().zip(dark.par_iter_mut()).zip(buffer.pixels.par_iter()).for_each(|((l, d), px)| {
        *l = luma(px).max(1.0e-6);
        *d = px[0].min(px[1]).min(px[2]).clamp(0.0, 1.0);
    });

    let min_dim = w.min(h);
    let fine_r = 1;
    let coarse_r = (min_dim / 100).clamp(4, 64);
    let sharp_r = (min_dim / 400).clamp(2, 8);
    let (fine, coarse, sharp) = (
        box_blur(&lum, w, h, fine_r),
        box_blur(&lum, w, h, coarse_r),
        box_blur(&lum, w, h, sharp_r),
    );

    let mut data = vec![[0.0f32; 4]; w * h];
    data.par_iter_mut().enumerate().for_each(|(i, out)| {
        let y = lum[i];
        out[0] = (y / fine[i].max(1.0e-6)).log2().clamp(-4.0, 4.0);
        out[1] = (y / coarse[i].max(1.0e-6)).log2().clamp(-4.0, 4.0);
        out[2] = (y / sharp[i].max(1.0e-6)).log2().clamp(-4.0, 4.0);
        out[3] = dark[i];
    });

    let air = atmospheric_light(&buffer.pixels, &dark);
    (DetailMap::new(buffer.width, buffer.height, data), air)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn flat_image_has_zero_detail_and_dark_channel() {
        let src = ImageBufferF32::new(8, 8, vec![[0.2, 0.2, 0.2]; 64]);
        let (map, air) = build(&src);
        assert!(map.is_valid());
        assert!(map.data.iter().all(|d| d[0].abs() < 1.0e-5 && d[1].abs() < 1.0e-5 && d[2].abs() < 1.0e-5));
        assert!(air.iter().all(|v| (*v - 0.2).abs() < 1.0e-4));
    }

    #[test]
    fn an_edge_produces_fine_contrast() {
        let mut px = vec![[0.05, 0.05, 0.05]; 16 * 16];
        for y in 0..16 {
            for x in 8..16 {
                px[y * 16 + x] = [0.8, 0.8, 0.8];
            }
        }
        let src = ImageBufferF32::new(16, 16, px);
        let (map, _) = build(&src);
        assert!(map.data.iter().any(|d| d[0] > 0.5));
        assert!(map.data.iter().any(|d| d[0] < -0.5));
    }
}
