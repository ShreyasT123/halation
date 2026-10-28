use halation_core::ImageBufferF32;

#[derive(Debug, Clone)]
pub struct RawMetadata {
    pub camera: String,
    pub lens: String,
    pub iso: u32,
    pub shutter: String,
    pub aperture: String,
}

#[derive(Debug, Clone)]
pub struct RawAsset {
    pub id: u64,
    pub filename: String,
    pub metadata: RawMetadata,
    pub buffer: ImageBufferF32,
}

pub fn load_sample_raws(width: u32, height: u32) -> Vec<RawAsset> {
    vec![
        RawAsset {
            id: 1,
            filename: "DSC_0042.RAW".into(),
            metadata: RawMetadata {
                camera: "Halation Alpha 1".into(),
                lens: "FE 50mm F1.2 GM".into(),
                iso: 100,
                shutter: "1/500s".into(),
                aperture: "f/2.8".into(),
            },
            buffer: generate_mountain_sunrise(width, height),
        },
        RawAsset {
            id: 2,
            filename: "DSC_0043.RAW".into(),
            metadata: RawMetadata {
                camera: "Halation Alpha 1".into(),
                lens: "FE 24-70mm F2.8 GM II".into(),
                iso: 400,
                shutter: "1/125s".into(),
                aperture: "f/4.0".into(),
            },
            buffer: generate_coastal_sunset(width, height),
        },
        RawAsset {
            id: 3,
            filename: "DSC_0044.RAW".into(),
            metadata: RawMetadata {
                camera: "Halation Alpha 1".into(),
                lens: "FE 85mm F1.4 GM".into(),
                iso: 800,
                shutter: "1/1000s".into(),
                aperture: "f/1.4".into(),
            },
            buffer: generate_emerald_forest(width, height),
        },
    ]
}

fn generate_mountain_sunrise(w: u32, h: u32) -> ImageBufferF32 {
    let mut pixels = Vec::with_capacity((w * h) as usize);
    for y in 0..h {
        let v = y as f32 / h as f32;
        for x in 0..w {
            let u = x as f32 / w as f32;
            let sun = (1.0 - (((u - 0.7).powi(2) + (v - 0.2).powi(2)).sqrt() / 0.4).min(1.0)).powi(2);
            let (r, g, b) = if v < 0.5 {
                (0.25 + 0.4 * (1.0 - v / 0.5) + 0.5 * sun, 0.40 + 0.3 * (1.0 - v / 0.5) + 0.3 * sun, 0.75 + 0.1 * (1.0 - v / 0.5))
            } else {
                let ridge = (u * 12.0).sin() * 0.05;
                let t = ((v - 0.5) / 0.5 + ridge).clamp(0.0, 1.0);
                (0.18 * (1.0 - t * 0.5), 0.22 * (1.0 - t * 0.5), 0.20 * (1.0 - t * 0.5))
            };
            pixels.push([r, g, b]);
        }
    }
    ImageBufferF32 { width: w, height: h, pixels }
}

fn generate_coastal_sunset(w: u32, h: u32) -> ImageBufferF32 {
    let mut pixels = Vec::with_capacity((w * h) as usize);
    for y in 0..h {
        let v = y as f32 / h as f32;
        for x in 0..w {
            let u = x as f32 / w as f32;
            let sun = (1.0 - (((u - 0.5).powi(2) + (v - 0.45).powi(2)).sqrt() / 0.35).min(1.0)).powi(2);
            let (r, g, b) = if v < 0.55 {
                (0.85 * (1.0 - v * 0.8) + sun * 0.5, 0.35 * (1.0 - v * 0.6) + sun * 0.4, 0.45 * v)
            } else {
                let ocean = ((u * 40.0).sin() * 0.02).abs();
                (0.10 + ocean, 0.15 + ocean, 0.28 + ocean)
            };
            pixels.push([r, g, b]);
        }
    }
    ImageBufferF32 { width: w, height: h, pixels }
}

fn generate_emerald_forest(w: u32, h: u32) -> ImageBufferF32 {
    let mut pixels = Vec::with_capacity((w * h) as usize);
    for y in 0..h {
        let v = y as f32 / h as f32;
        for x in 0..w {
            let u = x as f32 / w as f32;
            let dappled = ((u * 20.0).sin() * (v * 20.0).cos()).abs() * 0.2;
            let (r, g, b) = if v < 0.3 {
                (0.20 + dappled, 0.45 + dappled, 0.25)
            } else {
                let moss = (v * 0.3).clamp(0.0, 0.4);
                (0.12 + dappled * 0.5, 0.32 + moss + dappled, 0.14)
            };
            pixels.push([r, g, b]);
        }
    }
    ImageBufferF32 { width: w, height: h, pixels }
}
