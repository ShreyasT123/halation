use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct DevelopParameters {
    pub exposure: f32,       // -5.0 .. +5.0 EV
    pub contrast: f32,       // -100 .. +100
    pub highlights: f32,     // -100 .. +100
    pub shadows: f32,        // -100 .. +100
    pub temperature: f32,    // 2000K .. 10000K
    pub tint: f32,           // -100 .. +100
}

impl Default for DevelopParameters {
    fn default() -> Self {
        Self {
            exposure: 0.0,
            contrast: 0.0,
            highlights: 0.0,
            shadows: 0.0,
            temperature: 5500.0,
            tint: 0.0,
        }
    }
}

#[derive(Debug, Clone)]
pub struct HistogramData {
    pub r: [u32; 256],
    pub g: [u32; 256],
    pub b: [u32; 256],
}

impl Default for HistogramData {
    fn default() -> Self {
        Self {
            r: [0; 256],
            g: [0; 256],
            b: [0; 256],
        }
    }
}

#[derive(Debug, Clone)]
pub struct ImageBufferF32 {
    pub width: u32,
    pub height: u32,
    pub pixels: Vec<[f32; 3]>, // Linear floating-point RGB
}
