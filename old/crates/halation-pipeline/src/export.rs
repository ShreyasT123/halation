//! Batch export (Sprint 8): develop at full resolution, resample, watermark, encode.
//!
//! * Resampling is Lanczos3 on the *linear* working-space buffer before development, so shrinking never
//!   darkens fine detail.
//! * Output never uses the monitor LUT: pixels go to the chosen output space (sRGB, Display P3,
//!   Adobe RGB, ProPhoto) and a generated ICC profile is embedded wherever the encoder allows it.
//! * Linear DNG stores 16-bit linear ProPhoto (`LinearRaw`) with the matching `ColorMatrix1`.
//! * Batches run two images at a time (each develop is internally parallel) so full-resolution
//!   buffers do not exhaust memory, and report count / percentage / ETA through a callback.
//!
//! Cargo: `image` (png, jpeg, tiff, webp), `ab_glyph = "0.2"`, `rayon`, `halation-color`.

use halation_color::{
    build_icc_profile, sample_mask, OutputSpace, OutputTransform, PreparedDevelop,
};
use halation_core::{DevelopParameters, ImageBufferF32, MaskBuffer};
use halation_raw::decode_file;
use image::{
    codecs::{jpeg::JpegEncoder, png::PngEncoder, tiff::TiffEncoder, webp::WebPEncoder},
    ExtendedColorType, ImageEncoder,
};
use rayon::prelude::*;
use std::{
    f32::consts::PI,
    fs,
    io::Cursor,
    path::{Path, PathBuf},
    sync::{
        atomic::{AtomicUsize, Ordering},
        Arc,
    },
    time::Instant,
};

#[derive(Debug, Clone, Copy, PartialEq)]
pub enum ExportFormat {
    Jpeg {
        quality: u8,
    },
    Png {
        sixteen_bit: bool,
    },
    Tiff {
        sixteen_bit: bool,
    },
    /// Lossless WebP (the `image` crate has no lossy encoder).
    WebP,
    LinearDng,
}

impl ExportFormat {
    pub fn extension(self) -> &'static str {
        match self {
            ExportFormat::Jpeg { .. } => "jpg",
            ExportFormat::Png { .. } => "png",
            ExportFormat::Tiff { .. } => "tif",
            ExportFormat::WebP => "webp",
            ExportFormat::LinearDng => "dng",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Anchor {
    TopLeft,
    TopRight,
    BottomLeft,
    BottomRight,
    Center,
}

#[derive(Debug, Clone)]
pub enum WatermarkKind {
    /// PNG / any `image`-readable logo with alpha.
    Logo(PathBuf),
    /// Text rendered with a TrueType / OpenType font file.
    Text {
        text: String,
        font: PathBuf,
        color: [u8; 3],
    },
}

#[derive(Debug, Clone)]
pub struct Watermark {
    pub kind: WatermarkKind,
    pub anchor: Anchor,
    /// Distance from the edges as a fraction of the image's long edge.
    pub margin: f32,
    /// Logo width / text height as a fraction of the image width.
    pub scale: f32,
    pub opacity: f32,
    /// Extra sub-pixel nudge in output pixels.
    pub offset: [f32; 2],
}

#[derive(Debug, Clone)]
pub struct ExportSettings {
    pub format: ExportFormat,
    pub space: OutputSpace,
    pub max_long_edge: Option<u32>,
    pub embed_profile: bool,
    pub watermark: Option<Watermark>,
}

impl Default for ExportSettings {
    fn default() -> Self {
        Self {
            format: ExportFormat::Jpeg { quality: 92 },
            space: OutputSpace::Srgb,
            max_long_edge: None,
            embed_profile: true,
            watermark: None,
        }
    }
}

#[derive(Debug, Clone)]
pub struct ExportJob {
    pub src: PathBuf,
    pub dest: PathBuf,
    pub params: DevelopParameters,
    pub mask: Option<Arc<MaskBuffer>>,
}

#[derive(Debug, Clone, Copy)]
pub struct ExportProgress {
    pub done: usize,
    pub total: usize,
    pub elapsed_secs: f32,
    pub eta_secs: f32,
}

impl ExportProgress {
    pub fn percent(&self) -> f32 {
        100.0 * self.done as f32 / self.total.max(1) as f32
    }
}

// ------------------------------------------------------------------------------------------------
// Resampling (separable Lanczos3, linear light)
// ------------------------------------------------------------------------------------------------

fn lanczos3(x: f32) -> f32 {
    let x = x.abs();
    if x >= 3.0 {
        0.0
    } else if x < 1.0e-6 {
        1.0
    } else {
        let px = PI * x;
        3.0 * px.sin() * (px / 3.0).sin() / (px * px)
    }
}

fn taps(src: usize, dst: usize) -> Vec<(usize, Vec<f32>)> {
    let scale = src as f32 / dst as f32;
    let filter_scale = scale.max(1.0);
    let support = 3.0 * filter_scale;
    (0..dst)
        .map(|i| {
            let center = (i as f32 + 0.5) * scale;
            let start = (center - support).floor().max(0.0) as usize;
            let end = ((center + support).ceil() as usize).min(src).max(start + 1);
            let mut w: Vec<f32> = (start..end)
                .map(|j| lanczos3((j as f32 + 0.5 - center) / filter_scale))
                .collect();
            let sum: f32 = w.iter().sum();
            if sum.abs() > 1.0e-8 {
                w.iter_mut().for_each(|v| *v /= sum);
            }
            (start, w)
        })
        .collect()
}

pub fn resize_lanczos3(src: &ImageBufferF32, tw: u32, th: u32) -> ImageBufferF32 {
    let (sw, sh, tw, th) = (
        src.width as usize,
        src.height as usize,
        tw.max(1) as usize,
        th.max(1) as usize,
    );
    let hx = taps(sw, tw);
    let mut mid = vec![[0.0f32; 3]; tw * sh];
    mid.par_chunks_mut(tw)
        .zip(src.pixels.par_chunks(sw))
        .for_each(|(out, row)| {
            for (o, (start, w)) in out.iter_mut().zip(&hx) {
                let mut acc = [0.0f32; 3];
                for (k, wk) in w.iter().enumerate() {
                    let p = row[start + k];
                    acc = [acc[0] + p[0] * wk, acc[1] + p[1] * wk, acc[2] + p[2] * wk];
                }
                *o = acc;
            }
        });
    let vy = taps(sh, th);
    let mut out = vec![[0.0f32; 3]; tw * th];
    out.par_chunks_mut(tw)
        .zip(vy.par_iter())
        .for_each(|(row, (start, w))| {
            for (x, o) in row.iter_mut().enumerate() {
                let mut acc = [0.0f32; 3];
                for (k, wk) in w.iter().enumerate() {
                    let p = mid[(start + k) * tw + x];
                    acc = [acc[0] + p[0] * wk, acc[1] + p[1] * wk, acc[2] + p[2] * wk];
                }
                *o = [acc[0].max(0.0), acc[1].max(0.0), acc[2].max(0.0)];
            }
        });
    ImageBufferF32::new(tw as u32, th as u32, out)
}

// ------------------------------------------------------------------------------------------------
// Development
// ------------------------------------------------------------------------------------------------

/// Develop to encoded RGB floats in `space` (`w·h·3`, row-major, 0..1).
pub fn develop_encoded(
    buffer: &ImageBufferF32,
    params: &DevelopParameters,
    mask: Option<&MaskBuffer>,
    space: OutputSpace,
) -> Vec<f32> {
    let mask = mask.filter(|m| m.is_valid());
    let prepared = PreparedDevelop::from_params(params, mask.is_some());
    let out = OutputTransform::new(space);
    develop_rows(buffer, mask, &prepared, |p, m| {
        prepared.develop_to_output(p, m, &out)
    })
}

/// Develop to linear working-space floats (for Linear DNG).
pub fn develop_linear(
    buffer: &ImageBufferF32,
    params: &DevelopParameters,
    mask: Option<&MaskBuffer>,
) -> Vec<f32> {
    let mask = mask.filter(|m| m.is_valid());
    let prepared = PreparedDevelop::from_params(params, mask.is_some());
    develop_rows(buffer, mask, &prepared, |p, m| {
        prepared.develop_working(p, m)
    })
}

fn develop_rows(
    buffer: &ImageBufferF32,
    mask: Option<&MaskBuffer>,
    prepared: &PreparedDevelop,
    f: impl Fn([f32; 3], f32) -> [f32; 3] + Sync,
) -> Vec<f32> {
    let (w, h) = (buffer.width as usize, buffer.height as usize);
    let mut out = vec![0.0f32; w * h * 3];
    out.par_chunks_exact_mut(w * 3)
        .zip(buffer.pixels.par_chunks_exact(w))
        .enumerate()
        .for_each(|(y, (row, src))| {
            let v = (y as f32 + 0.5) / h as f32;
            for (x, (o, px)) in row.chunks_exact_mut(3).zip(src).enumerate() {
                let m = match mask {
                    Some(mask) if prepared.has_local() => {
                        sample_mask(mask, (x as f32 + 0.5) / w as f32, v)
                    }
                    _ => 0.0,
                };
                o.copy_from_slice(&f(*px, m));
            }
        });
    out
}

// ------------------------------------------------------------------------------------------------
// Watermark
// ------------------------------------------------------------------------------------------------

/// RGBA coverage layer to composite (straight alpha, 0..255 per channel).
struct Layer {
    w: usize,
    h: usize,
    rgba: Vec<u8>,
}

fn render_watermark_layer(wm: &Watermark, image_w: usize) -> Result<Layer, String> {
    match &wm.kind {
        WatermarkKind::Logo(path) => {
            let logo = image::open(path)
                .map_err(|e| format!("watermark logo: {e}"))?
                .to_rgba8();
            let tw = ((wm.scale * image_w as f32).round() as u32).max(1);
            let th = ((logo.height() as f32 * tw as f32 / logo.width().max(1) as f32).round()
                as u32)
                .max(1);
            let resized =
                image::imageops::resize(&logo, tw, th, image::imageops::FilterType::Lanczos3);
            Ok(Layer {
                w: tw as usize,
                h: th as usize,
                rgba: resized.into_raw(),
            })
        }
        WatermarkKind::Text { text, font, color } => {
            use ab_glyph::{point, Font, FontVec, PxScale, ScaleFont};
            let bytes = fs::read(font).map_err(|e| format!("watermark font: {e}"))?;
            let font = FontVec::try_from_vec(bytes).map_err(|e| format!("watermark font: {e}"))?;
            let px = (wm.scale * image_w as f32).max(8.0);
            let scaled = font.as_scaled(PxScale::from(px));
            let (ascent, descent) = (scaled.ascent(), scaled.descent());
            let mut width = 0.0f32;
            let mut prev = None;
            for c in text.chars() {
                let id = scaled.glyph_id(c);
                if let Some(p) = prev {
                    width += scaled.kern(p, id);
                }
                width += scaled.h_advance(id);
                prev = Some(id);
            }
            let (w, h) = (
                (width.ceil() as usize + 4).max(1),
                ((ascent - descent).ceil() as usize + 4).max(1),
            );
            let mut rgba = vec![0u8; w * h * 4];
            let (mut x, mut prev) = (2.0f32, None);
            for c in text.chars() {
                let id = scaled.glyph_id(c);
                if let Some(p) = prev {
                    x += scaled.kern(p, id);
                }
                let glyph = id.with_scale_and_position(PxScale::from(px), point(x, 2.0 + ascent));
                if let Some(outlined) = scaled.outline_glyph(glyph) {
                    let b = outlined.px_bounds();
                    outlined.draw(|gx, gy, cov| {
                        let (ix, iy) = (b.min.x as i32 + gx as i32, b.min.y as i32 + gy as i32);
                        if ix >= 0 && iy >= 0 && (ix as usize) < w && (iy as usize) < h {
                            let i = (iy as usize * w + ix as usize) * 4;
                            let a = (cov * 255.0) as u8;
                            if a >= rgba[i + 3] {
                                rgba[i..i + 4].copy_from_slice(&[color[0], color[1], color[2], a]);
                            }
                        }
                    });
                }
                x += scaled.h_advance(id);
                prev = Some(id);
            }
            Ok(Layer { w, h, rgba })
        }
    }
}

/// Bilinear (sub-pixel) alpha composite of `layer` onto encoded RGB floats.
fn composite(enc: &mut [f32], w: usize, h: usize, layer: &Layer, wm: &Watermark) {
    let margin = wm.margin * w.max(h) as f32;
    let (lw, lh) = (layer.w as f32, layer.h as f32);
    let (ox, oy) = match wm.anchor {
        Anchor::TopLeft => (margin, margin),
        Anchor::TopRight => (w as f32 - lw - margin, margin),
        Anchor::BottomLeft => (margin, h as f32 - lh - margin),
        Anchor::BottomRight => (w as f32 - lw - margin, h as f32 - lh - margin),
        Anchor::Center => ((w as f32 - lw) * 0.5, (h as f32 - lh) * 0.5),
    };
    let (ox, oy) = (ox + wm.offset[0], oy + wm.offset[1]);
    let sample = |fx: f32, fy: f32| -> [f32; 4] {
        let (x0, y0) = (fx.floor(), fy.floor());
        let (tx, ty) = (fx - x0, fy - y0);
        let px = |x: f32, y: f32| -> [f32; 4] {
            if x < 0.0 || y < 0.0 || x >= lw || y >= lh {
                return [0.0; 4];
            }
            let i = (y as usize * layer.w + x as usize) * 4;
            let a = layer.rgba[i + 3] as f32 / 255.0;
            [
                layer.rgba[i] as f32 / 255.0 * a,
                layer.rgba[i + 1] as f32 / 255.0 * a,
                layer.rgba[i + 2] as f32 / 255.0 * a,
                a,
            ]
        };
        let (a, b, c, d) = (
            px(x0, y0),
            px(x0 + 1.0, y0),
            px(x0, y0 + 1.0),
            px(x0 + 1.0, y0 + 1.0),
        );
        let mut out = [0.0; 4];
        for k in 0..4 {
            out[k] =
                (a[k] * (1.0 - tx) + b[k] * tx) * (1.0 - ty) + (c[k] * (1.0 - tx) + d[k] * tx) * ty;
        }
        out
    };
    let (x_lo, x_hi) = (
        (ox.floor().max(0.0)) as usize,
        ((ox + lw).ceil() as usize + 1).min(w),
    );
    let (y_lo, y_hi) = (
        (oy.floor().max(0.0)) as usize,
        ((oy + lh).ceil() as usize + 1).min(h),
    );
    for y in y_lo..y_hi {
        for x in x_lo..x_hi {
            let s = sample(x as f32 - ox, y as f32 - oy);
            let a = (s[3] * wm.opacity).clamp(0.0, 1.0);
            if a <= 0.0 {
                continue;
            }
            let i = (y * w + x) * 3;
            for k in 0..3 {
                let premult = s[k] * wm.opacity;
                enc[i + k] = (enc[i + k] * (1.0 - a) + premult).clamp(0.0, 1.0);
            }
        }
    }
}

// ------------------------------------------------------------------------------------------------
// Encoders
// ------------------------------------------------------------------------------------------------

fn to_u8(enc: &[f32]) -> Vec<u8> {
    enc.iter()
        .map(|v| (v.clamp(0.0, 1.0) * 255.0 + 0.5) as u8)
        .collect()
}

fn to_u16_ne(enc: &[f32]) -> Vec<u8> {
    enc.iter()
        .flat_map(|v| ((v.clamp(0.0, 1.0) * 65535.0 + 0.5) as u16).to_ne_bytes())
        .collect()
}

fn encode(enc: &[f32], w: u32, h: u32, settings: &ExportSettings) -> Result<Vec<u8>, String> {
    let icc = settings
        .embed_profile
        .then(|| build_icc_profile(settings.space));
    let mut buf = Vec::new();
    let err = |e: image::ImageError| e.to_string();
    match settings.format {
        ExportFormat::Jpeg { quality } => {
            let mut e = JpegEncoder::new_with_quality(&mut buf, quality.clamp(1, 100));
            if let Some(icc) = icc {
                let _ = e.set_icc_profile(icc);
            }
            e.write_image(&to_u8(enc), w, h, ExtendedColorType::Rgb8)
                .map_err(err)?;
        }
        ExportFormat::Png { sixteen_bit } => {
            let mut e = PngEncoder::new(&mut buf);
            if let Some(icc) = icc {
                let _ = e.set_icc_profile(icc);
            }
            if sixteen_bit {
                e.write_image(&to_u16_ne(enc), w, h, ExtendedColorType::Rgb16)
                    .map_err(err)?;
            } else {
                e.write_image(&to_u8(enc), w, h, ExtendedColorType::Rgb8)
                    .map_err(err)?;
            }
        }
        ExportFormat::Tiff { sixteen_bit } => {
            let mut e = TiffEncoder::new(Cursor::new(&mut buf));
            if let Some(icc) = icc {
                let _ = e.set_icc_profile(icc);
            }
            if sixteen_bit {
                e.write_image(&to_u16_ne(enc), w, h, ExtendedColorType::Rgb16)
                    .map_err(err)?;
            } else {
                e.write_image(&to_u8(enc), w, h, ExtendedColorType::Rgb8)
                    .map_err(err)?;
            }
        }
        ExportFormat::WebP => {
            let mut e = WebPEncoder::new_lossless(&mut buf);
            if let Some(icc) = icc {
                let _ = e.set_icc_profile(icc);
            }
            e.write_image(&to_u8(enc), w, h, ExtendedColorType::Rgb8)
                .map_err(err)?;
        }
        ExportFormat::LinearDng => return Ok(linear_dng(enc, w, h)),
    }
    Ok(buf)
}

/// Minimal Linear DNG 1.4: 16-bit interleaved `LinearRaw`, linear ProPhoto primaries, D50.
pub fn linear_dng(linear: &[f32], w: u32, h: u32) -> Vec<u8> {
    const N: usize = 21;
    let ifd_len = 2 + N * 12 + 4;
    let data_start = 8 + ifd_len;
    let mut extra: Vec<u8> = Vec::new();
    let place = |extra: &mut Vec<u8>, bytes: &[u8]| -> u32 {
        let at = (data_start + extra.len()) as u32;
        extra.extend_from_slice(bytes);
        if extra.len() % 2 == 1 {
            extra.push(0);
        }
        at
    };
    let shorts = |v: &[u16]| v.iter().flat_map(|x| x.to_le_bytes()).collect::<Vec<u8>>();
    let longs = |v: &[u32]| v.iter().flat_map(|x| x.to_le_bytes()).collect::<Vec<u8>>();
    let srational = |v: f64| -> Vec<u8> {
        [
            ((v * 1.0e6).round() as i32).to_le_bytes(),
            1_000_000i32.to_le_bytes(),
        ]
        .concat()
    };

    let bits_off = place(&mut extra, &shorts(&[16, 16, 16]));
    let fmt_off = place(&mut extra, &shorts(&[1, 1, 1]));
    let model = b"Halation Linear ProPhoto\0";
    let model_off = place(&mut extra, model);
    let black_off = place(&mut extra, &shorts(&[0, 0, 0]));
    let white_off = place(&mut extra, &longs(&[65535, 65535, 65535]));
    // ColorMatrix1: XYZ(D50) -> camera (= linear ProPhoto) is the inverse of ProPhoto -> XYZ(D50).
    let inv = [
        [1.3459433, -0.2556075, -0.0511118],
        [-0.5445989, 1.5081673, 0.0205351],
        [0.0, 0.0, 1.2118128],
    ];
    let matrix_bytes: Vec<u8> = inv.iter().flatten().flat_map(|v| srational(*v)).collect();
    let matrix_off = place(&mut extra, &matrix_bytes);
    let neutral_off = place(
        &mut extra,
        &[1u32, 1, 1, 1, 1, 1]
            .iter()
            .flat_map(|v| v.to_le_bytes())
            .collect::<Vec<u8>>(),
    );
    let image_off = (data_start + extra.len()) as u32;
    let image_bytes = (w as usize * h as usize * 6) as u32;

    let inline_short = |v: u16| [v.to_le_bytes()[0], v.to_le_bytes()[1], 0, 0];
    // (tag, type, count, 4 value bytes)
    let entries: [(u16, u16, u32, [u8; 4]); N] = [
        (254, 4, 1, 0u32.to_le_bytes()),
        (256, 4, 1, w.to_le_bytes()),
        (257, 4, 1, h.to_le_bytes()),
        (258, 3, 3, bits_off.to_le_bytes()),
        (259, 3, 1, inline_short(1)),
        (262, 3, 1, inline_short(34892)),
        (273, 4, 1, image_off.to_le_bytes()),
        (274, 3, 1, inline_short(1)),
        (277, 3, 1, inline_short(3)),
        (278, 4, 1, h.to_le_bytes()),
        (279, 4, 1, image_bytes.to_le_bytes()),
        (284, 3, 1, inline_short(1)),
        (339, 3, 3, fmt_off.to_le_bytes()),
        (50706, 1, 4, [1, 4, 0, 0]),
        (50707, 1, 4, [1, 1, 0, 0]),
        (50708, 2, model.len() as u32, model_off.to_le_bytes()),
        (50714, 3, 3, black_off.to_le_bytes()),
        (50717, 4, 3, white_off.to_le_bytes()),
        (50721, 10, 9, matrix_off.to_le_bytes()),
        (50728, 5, 3, neutral_off.to_le_bytes()),
        (50778, 3, 1, inline_short(23)),
    ];

    let mut out = Vec::with_capacity(image_off as usize + image_bytes as usize);
    out.extend_from_slice(b"II*\0");
    out.extend_from_slice(&8u32.to_le_bytes());
    out.extend_from_slice(&(N as u16).to_le_bytes());
    for (tag, ty, count, value) in entries {
        out.extend_from_slice(&tag.to_le_bytes());
        out.extend_from_slice(&ty.to_le_bytes());
        out.extend_from_slice(&count.to_le_bytes());
        out.extend_from_slice(&value);
    }
    out.extend_from_slice(&0u32.to_le_bytes());
    out.extend_from_slice(&extra);
    for v in linear {
        out.extend_from_slice(&((v.clamp(0.0, 1.0) * 65535.0 + 0.5) as u16).to_le_bytes());
    }
    out
}

// ------------------------------------------------------------------------------------------------
// Driver
// ------------------------------------------------------------------------------------------------

pub fn export_one(job: &ExportJob, settings: &ExportSettings) -> Result<(), String> {
    let asset = decode_file(&job.src).map_err(|e| format!("{}: {e}", job.src.display()))?;
    let mut buffer = asset.buffer;
    if let Some(max) = settings.max_long_edge {
        let long = buffer.width.max(buffer.height);
        if max > 0 && long > max {
            let s = max as f64 / long as f64;
            buffer = resize_lanczos3(
                &buffer,
                (buffer.width as f64 * s).round() as u32,
                (buffer.height as f64 * s).round() as u32,
            );
        }
    }
    let (w, h) = (buffer.width, buffer.height);
    let mask = job.mask.as_deref();
    let mut enc = if settings.format == ExportFormat::LinearDng {
        develop_linear(&buffer, &job.params, mask)
    } else {
        develop_encoded(&buffer, &job.params, mask, settings.space)
    };
    drop(buffer);
    if let (Some(wm), false) = (
        &settings.watermark,
        settings.format == ExportFormat::LinearDng,
    ) {
        let layer = render_watermark_layer(wm, w as usize)?;
        composite(&mut enc, w as usize, h as usize, &layer, wm);
    }
    let bytes = encode(&enc, w, h, settings)?;
    if let Some(parent) = job.dest.parent() {
        fs::create_dir_all(parent).map_err(|e| e.to_string())?;
    }
    let part = job.dest.with_extension("part");
    fs::write(&part, bytes).map_err(|e| format!("{}: {e}", part.display()))?;
    fs::rename(&part, &job.dest).map_err(|e| e.to_string())
}

/// Export `jobs` two at a time, calling `progress` after every finished image.
pub fn export_batch(
    jobs: &[ExportJob],
    settings: &ExportSettings,
    progress: &(dyn Fn(ExportProgress) + Sync),
) -> Vec<(PathBuf, Result<(), String>)> {
    let started = Instant::now();
    let done = AtomicUsize::new(0);
    let mut results = Vec::with_capacity(jobs.len());
    for chunk in jobs.chunks(2) {
        let part: Vec<_> = chunk
            .par_iter()
            .map(|job| {
                let result = export_one(job, settings);
                let n = done.fetch_add(1, Ordering::SeqCst) + 1;
                let elapsed = started.elapsed().as_secs_f32();
                progress(ExportProgress {
                    done: n,
                    total: jobs.len(),
                    elapsed_secs: elapsed,
                    eta_secs: elapsed / n as f32 * (jobs.len() - n) as f32,
                });
                (job.dest.clone(), result)
            })
            .collect();
        results.extend(part);
    }
    results
}

pub fn default_dest(src: &Path, dir: &Path, format: ExportFormat) -> PathBuf {
    let stem = src
        .file_stem()
        .map(|s| s.to_string_lossy().into_owned())
        .unwrap_or_else(|| "export".into());
    dir.join(format!("{stem}.{}", format.extension()))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn lanczos_keeps_flat_fields_flat() {
        let src = ImageBufferF32::new(40, 30, vec![[0.25, 0.5, 0.75]; 1200]);
        let out = resize_lanczos3(&src, 17, 11);
        assert_eq!((out.width, out.height), (17, 11));
        assert!(out
            .pixels
            .iter()
            .all(|p| (p[0] - 0.25).abs() < 1.0e-4 && (p[2] - 0.75).abs() < 1.0e-4));
    }

    #[test]
    fn dng_container_is_wellformed() {
        let px = vec![0.5f32; 4 * 3 * 3];
        let dng = linear_dng(&px, 4, 3);
        assert_eq!(&dng[0..4], b"II*\0");
        assert_eq!(u16::from_le_bytes([dng[8], dng[9]]), 21);
        assert_eq!(dng.len() % 2, 0);
        let off = u32::from_le_bytes([
            dng[8 + 2 + 6 * 12 + 8],
            dng[8 + 2 + 6 * 12 + 9],
            dng[8 + 2 + 6 * 12 + 10],
            dng[8 + 2 + 6 * 12 + 11],
        ]) as usize;
        assert_eq!(dng.len(), off + 4 * 3 * 6);
    }

    #[test]
    fn exports_and_encodes_in_every_format() {
        let enc = vec![0.5f32; 8 * 8 * 3];
        for format in [
            ExportFormat::Jpeg { quality: 90 },
            ExportFormat::Png { sixteen_bit: true },
            ExportFormat::Tiff { sixteen_bit: false },
            ExportFormat::WebP,
            ExportFormat::LinearDng,
        ] {
            let bytes = encode(
                &enc,
                8,
                8,
                &ExportSettings {
                    format,
                    ..Default::default()
                },
            )
            .unwrap();
            assert!(bytes.len() > 100, "{format:?}");
        }
    }
}
