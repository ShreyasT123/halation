use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct DevelopParameters {
    pub exposure: f32,    // -5.0 .. +5.0 EV
    pub contrast: f32,    // -100 .. +100
    pub highlights: f32,  // -100 .. +100
    pub shadows: f32,     // -100 .. +100
    pub whites: f32,      // -100 .. +100
    pub blacks: f32,      // -100 .. +100
    pub temperature: f32, // 2000K .. 10000K
    pub tint: f32,        // -100 .. +100
    #[serde(default = "default_tone_curve")]
    pub tone_curve: Vec<[f32; 2]>, // Four channels, five monotone spline points per channel
    #[serde(default = "default_color_grading")]
    pub color_grading: [[f32; 3]; 3], // Shadows, midtones, highlights: hue, saturation, luminance
}

fn default_tone_curve() -> Vec<[f32; 2]> {
    let identity = [
        [0.0, 0.0],
        [0.25, 0.25],
        [0.5, 0.5],
        [0.75, 0.75],
        [1.0, 1.0],
    ];
    identity.into_iter().cycle().take(20).collect()
}

fn default_color_grading() -> [[f32; 3]; 3] {
    [[0.58, 0.0, 0.0]; 3]
}

impl Default for DevelopParameters {
    fn default() -> Self {
        Self {
            exposure: 0.0,
            contrast: 0.0,
            highlights: 0.0,
            shadows: 0.0,
            whites: 0.0,
            blacks: 0.0,
            temperature: 5500.0,
            tint: 0.0,
            tone_curve: default_tone_curve(),
            color_grading: default_color_grading(),
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
