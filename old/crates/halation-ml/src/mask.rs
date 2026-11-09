//! Mask utilities that need no neural network: SAM image preprocessing, automatic prompt points for
//! "Select Subject" / "Select Sky", logit → 8-bit conversion and the analytic gradient masks.

use halation_core::{AiMaskKind, MaskBuffer, MaskSource};

pub const SAM_INPUT: usize = 1024;
const MEAN: [f32; 3] = [123.675, 116.28, 103.53];
const STD: [f32; 3] = [58.395, 57.12, 57.375];

/// How the encoder expects its RGB input.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum EncoderNorm {
    /// Pixels mapped to 0..1. Used by the 3-input MobileSAM exports.
    ZeroOne,
    /// SAM's ImageNet mean/std normalization. Used by the official 6-input exports.
    MeanStd,
}

/// A SAM prompt point in normalised image coordinates; `label` 1 = foreground, 0 = background.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Prompt {
    pub x: f32,
    pub y: f32,
    pub label: f32,
}

impl From<[f32; 3]> for Prompt {
    fn from(p: [f32; 3]) -> Self {
        Self {
            x: p[0],
            y: p[1],
            label: p[2],
        }
    }
}

/// Resize so the longest side is 1024 (bilinear), normalise with the SAM mean / std, zero-pad to
/// 1024² and return NCHW floats plus the resize scale.
pub fn preprocess_for_encoder(
    rgb: &[u8],
    width: usize,
    height: usize,
    norm: EncoderNorm,
) -> (Vec<f32>, f32) {
    let scale = SAM_INPUT as f32 / width.max(height) as f32;
    let (rw, rh) = (
        ((width as f32 * scale).round() as usize).clamp(1, SAM_INPUT),
        ((height as f32 * scale).round() as usize).clamp(1, SAM_INPUT),
    );
    let plane = SAM_INPUT * SAM_INPUT;
    let mut out = vec![0.0f32; 3 * plane];
    for y in 0..rh {
        let sy = ((y as f32 + 0.5) / scale - 0.5).clamp(0.0, (height - 1) as f32);
        let (y0, fy) = (sy as usize, sy - sy.floor());
        let y1 = (y0 + 1).min(height - 1);
        for x in 0..rw {
            let sx = ((x as f32 + 0.5) / scale - 0.5).clamp(0.0, (width - 1) as f32);
            let (x0, fx) = (sx as usize, sx - sx.floor());
            let x1 = (x0 + 1).min(width - 1);
            for c in 0..3 {
                let at = |xx: usize, yy: usize| rgb[(yy * width + xx) * 3 + c] as f32;
                let top = at(x0, y0) * (1.0 - fx) + at(x1, y0) * fx;
                let bottom = at(x0, y1) * (1.0 - fx) + at(x1, y1) * fx;
                let v = top * (1.0 - fy) + bottom * fy;
                out[c * plane + y * SAM_INPUT + x] = match norm {
                    EncoderNorm::ZeroOne => v / 255.0,
                    EncoderNorm::MeanStd => (v - MEAN[c]) / STD[c],
                };
            }
        }
    }
    (out, scale)
}

/// Prompt points used by the one-click selections (users refine with extra clicks afterwards).
pub fn automatic_prompts(kind: AiMaskKind) -> Vec<Prompt> {
    match kind {
        AiMaskKind::Subject => vec![
            Prompt {
                x: 0.5,
                y: 0.52,
                label: 1.0,
            },
            Prompt {
                x: 0.5,
                y: 0.40,
                label: 1.0,
            },
            Prompt {
                x: 0.04,
                y: 0.04,
                label: 0.0,
            },
            Prompt {
                x: 0.96,
                y: 0.04,
                label: 0.0,
            },
            Prompt {
                x: 0.04,
                y: 0.96,
                label: 0.0,
            },
            Prompt {
                x: 0.96,
                y: 0.96,
                label: 0.0,
            },
        ],
        AiMaskKind::Sky => vec![
            Prompt {
                x: 0.5,
                y: 0.08,
                label: 1.0,
            },
            Prompt {
                x: 0.2,
                y: 0.10,
                label: 1.0,
            },
            Prompt {
                x: 0.8,
                y: 0.10,
                label: 1.0,
            },
            Prompt {
                x: 0.5,
                y: 0.92,
                label: 0.0,
            },
        ],
    }
}

/// Decoder logits (`> 0` ⇒ inside) → 8-bit soft mask. The sigmoid keeps edges anti-aliased once the
/// GPU samples the mask bilinearly.
pub fn logits_to_mask(logits: &[f32], width: u32, height: u32) -> MaskBuffer {
    let data = logits
        .iter()
        .map(|&l| (255.0 / (1.0 + (-1.5 * l).exp()) + 0.5) as u8)
        .collect();
    MaskBuffer::new(width, height, data)
}

fn smoothstep(e0: f32, e1: f32, x: f32) -> f32 {
    let t = ((x - e0) / (e1 - e0)).clamp(0.0, 1.0);
    t * t * (3.0 - 2.0 * t)
}

/// Rasterise an analytic mask (`long_edge` pixels on the longest side, image `aspect` = w / h).
/// AI masks return `None`: they need the network.
pub fn rasterize(source: &MaskSource, aspect: f32, long_edge: u32) -> Option<MaskBuffer> {
    let (w, h) = if aspect >= 1.0 {
        (
            long_edge,
            ((long_edge as f32 / aspect).round() as u32).max(1),
        )
    } else {
        (
            ((long_edge as f32 * aspect).round() as u32).max(1),
            long_edge,
        )
    };
    let value: Box<dyn Fn(f32, f32) -> f32> = match source {
        MaskSource::Ai { .. } => return None,
        MaskSource::Linear { start, end } => {
            let (dx, dy) = (end[0] - start[0], end[1] - start[1]);
            let len2 = (dx * dx + dy * dy).max(1.0e-9);
            let start = *start;
            Box::new(move |x, y| {
                1.0 - smoothstep(0.0, 1.0, ((x - start[0]) * dx + (y - start[1]) * dy) / len2)
            })
        }
        MaskSource::Radial {
            center,
            radius,
            feather,
        } => {
            let (c, r, f) = (
                *center,
                [radius[0].max(1.0e-4), radius[1].max(1.0e-4)],
                feather.clamp(0.0, 1.0),
            );
            Box::new(move |x, y| {
                let d = (((x - c[0]) / r[0]).powi(2) + ((y - c[1]) / r[1]).powi(2)).sqrt();
                1.0 - smoothstep((1.0 - f).min(0.999), 1.0, d)
            })
        }
    };
    let data = (0..w * h)
        .map(|i| {
            let (x, y) = ((i % w) as f32 + 0.5, (i / w) as f32 + 0.5);
            (value(x / w as f32, y / h as f32) * 255.0 + 0.5) as u8
        })
        .collect();
    Some(MaskBuffer::new(w, h, data))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn preprocessing_pads_and_normalises() {
        let rgb = vec![128u8; 40 * 20 * 3];
        let (t, scale) = preprocess_for_encoder(&rgb, 40, 20, EncoderNorm::MeanStd);
        assert_eq!(t.len(), 3 * SAM_INPUT * SAM_INPUT);
        assert!((scale - 25.6).abs() < 1.0e-4);
        // inside the image: (128 - mean) / std; below the 512-px-tall content: zero padding
        assert!((t[10 * SAM_INPUT + 10] - (128.0 - MEAN[0]) / STD[0]).abs() < 1.0e-4);
        assert_eq!(t[(SAM_INPUT - 1) * SAM_INPUT + 5], 0.0);

        let (z, _) = preprocess_for_encoder(&rgb, 40, 20, EncoderNorm::ZeroOne);
        assert!((z[10 * SAM_INPUT + 10] - 128.0 / 255.0).abs() < 1.0e-6);
        assert_eq!(z[(SAM_INPUT - 1) * SAM_INPUT + 5], 0.0);
    }

    #[test]
    fn linear_gradient_fades_from_start_to_end() {
        let m = rasterize(
            &MaskSource::Linear {
                start: [0.5, 0.0],
                end: [0.5, 1.0],
            },
            1.5,
            60,
        )
        .unwrap();
        assert_eq!((m.width, m.height), (60, 40));
        assert!(m.data[0] > 240 && m.data[(m.height as usize - 1) * 60] < 15);
    }

    #[test]
    fn radial_mask_is_full_inside_and_empty_outside() {
        let m = rasterize(
            &MaskSource::Radial {
                center: [0.5, 0.5],
                radius: [0.2, 0.2],
                feather: 0.3,
            },
            1.0,
            50,
        )
        .unwrap();
        assert_eq!(m.data[25 * 50 + 25], 255);
        assert_eq!(m.data[0], 0);
        assert!(rasterize(
            &MaskSource::Ai {
                kind: AiMaskKind::Sky,
                points: vec![]
            },
            1.0,
            50
        )
        .is_none());
    }

    #[test]
    fn logits_map_to_soft_alpha() {
        let m = logits_to_mask(&[-9.0, 0.0, 9.0], 3, 1);
        assert_eq!((m.data[0], m.data[2]), (0, 255));
        assert!((120..=135).contains(&m.data[1]));
    }
}
