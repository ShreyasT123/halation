use halation_core::{DevelopParameters, HistogramData};
use halation_gpu::GpuDevelopEngine;
use halation_raw::RawAsset;
use rayon::prelude::*;
use std::{
    collections::{HashMap, VecDeque},
    io::Cursor,
    path::{Path, PathBuf},
    sync::{Arc, Mutex},
};

/// Small WebP preview cache. The memory tier is bounded; disk files are keyed
/// by the source file's BLAKE3 digest and can be safely discarded at any time.
pub struct ThumbnailCache {
    root: PathBuf,
    capacity: usize,
    memory: Mutex<(HashMap<[u8; 32], Arc<Vec<u8>>>, VecDeque<[u8; 32]>)>,
}

impl ThumbnailCache {
    pub fn new(root: impl Into<PathBuf>) -> Self {
        Self {
            root: root.into(),
            capacity: 500,
            memory: Mutex::new((HashMap::new(), VecDeque::new())),
        }
    }

    pub fn get_or_create(&self, source: &Path) -> Result<Arc<Vec<u8>>, String> {
        let mut source_file = std::fs::File::open(source).map_err(|e| e.to_string())?;
        let mut hasher = blake3::Hasher::new();
        std::io::copy(&mut source_file, &mut hasher).map_err(|e| e.to_string())?;
        let key = *hasher.finalize().as_bytes();
        {
            let mut cache = self.memory.lock().map_err(|e| e.to_string())?;
            if let Some(value) = cache.0.get(&key).cloned() {
                cache.1.retain(|entry| entry != &key);
                cache.1.push_back(key);
                return Ok(value);
            }
        }
        std::fs::create_dir_all(&self.root).map_err(|e| e.to_string())?;
        let disk_path = self
            .root
            .join(format!("{}.webp", blake3::Hash::from_bytes(key).to_hex()));
        let bytes = if disk_path.is_file() {
            std::fs::read(&disk_path).map_err(|e| e.to_string())?
        } else {
            let decoded = image::open(source).map_err(|e| e.to_string())?;
            let thumb = decoded.thumbnail(256, 256).to_rgb8();
            let mut encoded = Cursor::new(Vec::new());
            image::DynamicImage::ImageRgb8(thumb)
                .write_to(&mut encoded, image::ImageFormat::WebP)
                .map_err(|e| e.to_string())?;
            let bytes = encoded.into_inner();
            std::fs::write(&disk_path, &bytes).map_err(|e| e.to_string())?;
            bytes
        };
        let value = Arc::new(bytes);
        let mut cache = self.memory.lock().map_err(|e| e.to_string())?;
        cache.0.insert(key, value.clone());
        cache.1.retain(|entry| entry != &key);
        cache.1.push_back(key);
        while cache.1.len() > self.capacity {
            if let Some(oldest) = cache.1.pop_front() {
                cache.0.remove(&oldest);
            }
        }
        Ok(value)
    }
}

#[derive(Debug, Clone, Copy, Default)]
pub struct CullingMetrics {
    /// Variance of the grayscale Laplacian; larger values usually indicate focus.
    pub sharpness: f32,
    /// Fraction of pixels with any channel at or above 0.995 linear white.
    pub clipped_highlights: f32,
}

/// Fast local technical-quality metrics. Eye-state analysis requires a face
/// landmark model and is intentionally not inferred from these image metrics.
pub fn analyze_for_culling(raw: &RawAsset) -> CullingMetrics {
    let width = raw.buffer.width as usize;
    let height = raw.buffer.height as usize;
    if width < 3 || height < 3 || raw.buffer.pixels.len() < width * height {
        return CullingMetrics::default();
    }
    let gray: Vec<f32> = raw
        .buffer
        .pixels
        .iter()
        .map(|p| 0.2126 * p[0] + 0.7152 * p[1] + 0.0722 * p[2])
        .collect();
    let interior = (width - 2) * (height - 2);
    let (lap_sum, lap_sq, clipped) = (1..height - 1)
        .into_par_iter()
        .map(|y| {
            let (mut sum, mut square, mut clipped) = (0.0_f64, 0.0_f64, 0_u64);
            for x in 1..width - 1 {
                let i = y * width + x;
                let lap =
                    gray[i - 1] + gray[i + 1] + gray[i - width] + gray[i + width] - 4.0 * gray[i];
                sum += lap as f64;
                square += (lap as f64) * (lap as f64);
                let p = raw.buffer.pixels[i];
                clipped += u64::from(p[0] >= 0.995 || p[1] >= 0.995 || p[2] >= 0.995);
            }
            (sum, square, clipped)
        })
        .reduce(|| (0.0, 0.0, 0), |a, b| (a.0 + b.0, a.1 + b.1, a.2 + b.2));
    let n = interior as f64;
    CullingMetrics {
        sharpness: ((lap_sq / n) - (lap_sum / n).powi(2)).max(0.0) as f32,
        clipped_highlights: clipped as f32 / interior as f32,
    }
}

#[derive(Debug, Clone, Copy)]
pub enum ExportFormat {
    Jpeg,
    Png,
    WebP,
    Tiff,
}

#[derive(Debug, Clone)]
pub struct ExportJob {
    pub source: PathBuf,
    pub destination: PathBuf,
    pub params: DevelopParameters,
}

/// Decode, develop, and encode a batch on Rayon workers. Each file is isolated:
/// one corrupt source does not cancel the rest of the queue.
pub fn export_batch(jobs: &[ExportJob], format: ExportFormat) -> Vec<Result<PathBuf, String>> {
    jobs.par_iter()
        .map(|job| {
            let raw = halation_raw::decode_file(&job.source).map_err(|e| e.to_string())?;
            let output = process_frame(&raw, &job.params);
            let rgba = image::RgbaImage::from_raw(
                raw.buffer.width,
                raw.buffer.height,
                output.display_rgba,
            )
            .ok_or_else(|| "develop pipeline returned an invalid pixel buffer".to_owned())?;
            let image = image::DynamicImage::ImageRgba8(rgba);
            let image_format = match format {
                ExportFormat::Jpeg => image::ImageFormat::Jpeg,
                ExportFormat::Png => image::ImageFormat::Png,
                ExportFormat::WebP => image::ImageFormat::WebP,
                ExportFormat::Tiff => image::ImageFormat::Tiff,
            };
            if let Some(parent) = job.destination.parent() {
                std::fs::create_dir_all(parent).map_err(|e| e.to_string())?;
            }
            image
                .save_with_format(&job.destination, image_format)
                .map_err(|e| e.to_string())?;
            Ok(job.destination.clone())
        })
        .collect()
}

pub struct PipelineOutput {
    pub display_rgba: Vec<u8>,
    pub histogram: HistogramData,
    pub histogram_rgba: Vec<u8>,
    pub gpu_used: bool,
}

pub fn process_frame(raw: &RawAsset, params: &DevelopParameters) -> PipelineOutput {
    process_frame_with_gpu(raw, params, None)
}

pub fn process_frame_with_gpu(
    raw: &RawAsset,
    params: &DevelopParameters,
    gpu: Option<&GpuDevelopEngine>,
) -> PipelineOutput {
    if let Some(display_rgba) = gpu.and_then(|engine| engine.process(&raw.buffer, params).ok()) {
        return finish_frame(display_rgba, true);
    }
    let exp_mult = 2.0_f32.powf(params.exposure);
    let white_balance = halation_color::white_balance_matrix(params.temperature, params.tint);
    let pixels = &raw.buffer.pixels;

    // 1. Multithreaded parallel pixel processing using Rayon
    let processed_pixels: Vec<[u8; 3]> = pixels
        .par_iter()
        .map(|rgb| {
            // Exposure
            let mut c = [rgb[0] * exp_mult, rgb[1] * exp_mult, rgb[2] * exp_mult];
            // White Balance
            c = halation_color::apply_matrix(white_balance, c);
            // Highlights & Shadows
            c = halation_color::apply_highlights_shadows(c, params.highlights, params.shadows);
            c = halation_color::apply_whites_blacks(c, params.whites, params.blacks);
            // Contrast
            c = halation_color::apply_contrast(c, params.contrast);

            [
                (c[0] * 255.0) as u8,
                (c[1] * 255.0) as u8,
                (c[2] * 255.0) as u8,
            ]
        })
        .collect();

    // 2. Build 8-bit display buffer & compute 256-bin Histogram
    let mut display_rgba = Vec::with_capacity(processed_pixels.len() * 4);
    let mut histogram = HistogramData::default();

    for p in &processed_pixels {
        display_rgba.push(p[0]);
        display_rgba.push(p[1]);
        display_rgba.push(p[2]);
        display_rgba.push(255);

        histogram.r[p[0] as usize] += 1;
        histogram.g[p[1] as usize] += 1;
        histogram.b[p[2] as usize] += 1;
    }

    finish_frame_with_histogram(display_rgba, histogram, false)
}

fn finish_frame(display_rgba: Vec<u8>, gpu_used: bool) -> PipelineOutput {
    let mut histogram = HistogramData::default();
    for pixel in display_rgba.chunks_exact(4) {
        histogram.r[pixel[0] as usize] += 1;
        histogram.g[pixel[1] as usize] += 1;
        histogram.b[pixel[2] as usize] += 1;
    }
    finish_frame_with_histogram(display_rgba, histogram, gpu_used)
}

fn finish_frame_with_histogram(
    display_rgba: Vec<u8>,
    histogram: HistogramData,
    gpu_used: bool,
) -> PipelineOutput {
    let histogram_rgba = render_histogram_bitmap(&histogram, 256, 64);
    PipelineOutput {
        display_rgba,
        histogram,
        histogram_rgba,
        gpu_used,
    }
}

fn render_histogram_bitmap(hist: &HistogramData, width: u32, height: u32) -> Vec<u8> {
    let mut bmp = vec![25; (width * height * 4) as usize]; // dark grey background

    let max_count = hist
        .r
        .iter()
        .chain(&hist.g)
        .chain(&hist.b)
        .cloned()
        .max()
        .unwrap_or(1)
        .max(1);

    for x in 0..width {
        let r_h = ((hist.r[x as usize] as f32 / max_count as f32) * (height - 1) as f32) as u32;
        let g_h = ((hist.g[x as usize] as f32 / max_count as f32) * (height - 1) as f32) as u32;
        let b_h = ((hist.b[x as usize] as f32 / max_count as f32) * (height - 1) as f32) as u32;

        for y in 0..height {
            let inv_y = height - 1 - y;
            let idx = ((y * width + x) * 4) as usize;

            let in_r = inv_y <= r_h;
            let in_g = inv_y <= g_h;
            let in_b = inv_y <= b_h;

            if in_r || in_g || in_b {
                bmp[idx] = if in_r { 220 } else { 0 };
                bmp[idx + 1] = if in_g { 220 } else { 0 };
                bmp[idx + 2] = if in_b { 220 } else { 0 };
                bmp[idx + 3] = 255;
            }
        }
    }
    bmp
}
