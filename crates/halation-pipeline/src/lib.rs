use halation_core::{DevelopParameters, HistogramData};
use halation_raw::RawAsset;
use rayon::prelude::*;

pub struct PipelineOutput {
    pub display_rgba: Vec<u8>,
    pub histogram: HistogramData,
    pub histogram_rgba: Vec<u8>,
}

pub fn process_frame(raw: &RawAsset, params: &DevelopParameters) -> PipelineOutput {
    let exp_mult = 2.0_f32.powf(params.exposure);
    let pixels = &raw.buffer.pixels;

    // 1. Multithreaded parallel pixel processing using Rayon
    let processed_pixels: Vec<[u8; 3]> = pixels
        .par_iter()
        .map(|rgb| {
            // Exposure
            let mut c = [rgb[0] * exp_mult, rgb[1] * exp_mult, rgb[2] * exp_mult];
            // White Balance
            c = halation_color::white_balance(c, params.temperature, params.tint);
            // Highlights & Shadows
            c = halation_color::apply_highlights_shadows(c, params.highlights, params.shadows);
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

    // 3. Render real-time RGB histogram texture (256x64 px)
    let histogram_rgba = render_histogram_bitmap(&histogram, 256, 64);

    PipelineOutput {
        display_rgba,
        histogram,
        histogram_rgba,
    }
}

fn render_histogram_bitmap(hist: &HistogramData, width: u32, height: u32) -> Vec<u8> {
    let mut bmp = vec![25; (width * height * 4) as usize]; // dark grey background

    let max_count = hist.r.iter().chain(&hist.g).chain(&hist.b).cloned().max().unwrap_or(1).max(1);

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
