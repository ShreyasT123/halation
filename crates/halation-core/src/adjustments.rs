use crate::DevelopParameters;
use bytemuck::{Pod, Zeroable};
use serde::{Deserialize, Serialize};

pub const BAND_COUNT: usize = 8;
pub const BAND_HUES: [f32; BAND_COUNT] = [0.0, 30.0, 60.0, 120.0, 180.0, 240.0, 280.0, 320.0];
pub const BAND_NAMES: [&str; BAND_COUNT] = ["Red", "Orange", "Yellow", "Green", "Aqua", "Blue", "Purple", "Magenta"];

#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct BandAdjust {
    pub hue: f32,
    pub sat: f32,
    pub lum: f32,
}

impl Default for BandAdjust {
    fn default() -> Self {
        Self { hue: 0.0, sat: 0.0, lum: 0.0 }
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct ColorMixer {
    pub bands: [BandAdjust; BAND_COUNT],
}

impl Default for ColorMixer {
    fn default() -> Self {
        Self { bands: [BandAdjust::default(); BAND_COUNT] }
    }
}

impl ColorMixer {
    pub fn is_neutral(&self) -> bool {
        self.bands.iter().all(|b| b.hue == 0.0 && b.sat == 0.0 && b.lum == 0.0)
    }

    pub fn sanitized(mut self) -> Self {
        for band in &mut self.bands {
            band.hue = finite_clamp(band.hue, -100.0, 100.0, 0.0);
            band.sat = finite_clamp(band.sat, -100.0, 100.0, 0.0);
            band.lum = finite_clamp(band.lum, -100.0, 100.0, 0.0);
        }
        self
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct BlackWhite {
    pub enabled: bool,
    pub mix: [f32; BAND_COUNT],
}

impl Default for BlackWhite {
    fn default() -> Self {
        Self { enabled: false, mix: [0.0; BAND_COUNT] }
    }
}

impl BlackWhite {
    pub fn sanitized(mut self) -> Self {
        for value in &mut self.mix {
            *value = finite_clamp(*value, -100.0, 100.0, 0.0);
        }
        self
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct Effects {
    pub texture: f32,
    pub clarity: f32,
    pub dehaze: f32,
    pub vignette: f32,
    pub vignette_midpoint: f32,
    pub vignette_roundness: f32,
    pub vignette_feather: f32,
    pub grain: f32,
    pub grain_size: f32,
    pub grain_roughness: f32,
}

impl Default for Effects {
    fn default() -> Self {
        Self {
            texture: 0.0,
            clarity: 0.0,
            dehaze: 0.0,
            vignette: 0.0,
            vignette_midpoint: 50.0,
            vignette_roundness: 0.0,
            vignette_feather: 50.0,
            grain: 0.0,
            grain_size: 50.0,
            grain_roughness: 50.0,
        }
    }
}

impl Effects {
    pub fn sanitized(mut self) -> Self {
        self.texture = finite_clamp(self.texture, -100.0, 100.0, 0.0);
        self.clarity = finite_clamp(self.clarity, -100.0, 100.0, 0.0);
        self.dehaze = finite_clamp(self.dehaze, -100.0, 100.0, 0.0);
        self.vignette = finite_clamp(self.vignette, -100.0, 100.0, 0.0);
        self.vignette_midpoint = finite_clamp(self.vignette_midpoint, 0.0, 100.0, 50.0);
        self.vignette_roundness = finite_clamp(self.vignette_roundness, -100.0, 100.0, 0.0);
        self.vignette_feather = finite_clamp(self.vignette_feather, 0.0, 100.0, 50.0);
        self.grain = finite_clamp(self.grain, 0.0, 100.0, 0.0);
        self.grain_size = finite_clamp(self.grain_size, 0.0, 100.0, 50.0);
        self.grain_roughness = finite_clamp(self.grain_roughness, 0.0, 100.0, 50.0);
        self
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct Detail {
    pub sharpen: f32,
    pub detail: f32,
    pub masking: f32,
}

impl Default for Detail {
    fn default() -> Self {
        Self { sharpen: 0.0, detail: 50.0, masking: 0.0 }
    }
}

impl Detail {
    pub fn sanitized(mut self) -> Self {
        self.sharpen = finite_clamp(self.sharpen, 0.0, 150.0, 0.0);
        self.detail = finite_clamp(self.detail, 0.0, 100.0, 50.0);
        self.masking = finite_clamp(self.masking, 0.0, 100.0, 0.0);
        self
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct Parametric {
    pub highlights: f32,
    pub lights: f32,
    pub darks: f32,
    pub shadows: f32,
    pub split: [f32; 3],
}

impl Default for Parametric {
    fn default() -> Self {
        Self { highlights: 0.0, lights: 0.0, darks: 0.0, shadows: 0.0, split: [25.0, 50.0, 75.0] }
    }
}

impl Parametric {
    pub fn is_neutral(&self) -> bool {
        self.highlights == 0.0 && self.lights == 0.0 && self.darks == 0.0 && self.shadows == 0.0
    }

    pub fn sanitized(mut self) -> Self {
        self.highlights = finite_clamp(self.highlights, -100.0, 100.0, 0.0);
        self.lights = finite_clamp(self.lights, -100.0, 100.0, 0.0);
        self.darks = finite_clamp(self.darks, -100.0, 100.0, 0.0);
        self.shadows = finite_clamp(self.shadows, -100.0, 100.0, 0.0);
        for value in &mut self.split {
            *value = finite_clamp(*value, 0.0, 100.0, 50.0);
        }
        self.split[0] = self.split[0].min(self.split[1]);
        self.split[2] = self.split[2].max(self.split[1]);
        self
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum AdjustId {
    MixHue(usize),
    MixSat(usize),
    MixLum(usize),
    BwMix(usize),
    ParametricHighlights,
    ParametricLights,
    ParametricDarks,
    ParametricShadows,
    ParametricSplit0,
    ParametricSplit1,
    ParametricSplit2,
    Texture,
    Clarity,
    Dehaze,
    Vignette,
    VignetteMidpoint,
    VignetteRoundness,
    VignetteFeather,
    Grain,
    GrainSize,
    GrainRoughness,
    Sharpen,
    SharpenDetail,
    SharpenMasking,
}

impl AdjustId {
    pub const PARAMETRIC: [Self; 4] = [Self::ParametricHighlights, Self::ParametricLights, Self::ParametricDarks, Self::ParametricShadows];
    pub const PARAMETRIC_SPLIT: [Self; 3] = [Self::ParametricSplit0, Self::ParametricSplit1, Self::ParametricSplit2];
    pub const DETAIL: [Self; 3] = [Self::Sharpen, Self::SharpenDetail, Self::SharpenMasking];
    pub const ALL: [Self; 52] = [
        Self::MixHue(0), Self::MixHue(1), Self::MixHue(2), Self::MixHue(3), Self::MixHue(4), Self::MixHue(5), Self::MixHue(6), Self::MixHue(7),
        Self::MixSat(0), Self::MixSat(1), Self::MixSat(2), Self::MixSat(3), Self::MixSat(4), Self::MixSat(5), Self::MixSat(6), Self::MixSat(7),
        Self::MixLum(0), Self::MixLum(1), Self::MixLum(2), Self::MixLum(3), Self::MixLum(4), Self::MixLum(5), Self::MixLum(6), Self::MixLum(7),
        Self::BwMix(0), Self::BwMix(1), Self::BwMix(2), Self::BwMix(3), Self::BwMix(4), Self::BwMix(5), Self::BwMix(6), Self::BwMix(7),
        Self::ParametricHighlights, Self::ParametricLights, Self::ParametricDarks, Self::ParametricShadows,
        Self::ParametricSplit0, Self::ParametricSplit1, Self::ParametricSplit2,
        Self::Texture, Self::Clarity, Self::Dehaze,
        Self::Vignette, Self::VignetteMidpoint, Self::VignetteRoundness, Self::VignetteFeather,
        Self::Grain, Self::GrainSize, Self::GrainRoughness,
        Self::Sharpen, Self::SharpenDetail, Self::SharpenMasking,
    ];

    pub const fn label(self) -> &'static str {
        match self {
            Self::MixHue(_) => "Hue",
            Self::MixSat(_) => "Saturation",
            Self::MixLum(_) => "Luminance",
            Self::BwMix(_) => "B&W Mix",
            Self::ParametricHighlights => "Highlights",
            Self::ParametricLights => "Lights",
            Self::ParametricDarks => "Darks",
            Self::ParametricShadows => "Shadows",
            Self::ParametricSplit0 => "Shadows",
            Self::ParametricSplit1 => "Midtones",
            Self::ParametricSplit2 => "Highlights",
            Self::Texture => "Texture",
            Self::Clarity => "Clarity",
            Self::Dehaze => "Dehaze",
            Self::Vignette | Self::Grain => "Amount",
            Self::VignetteMidpoint => "Midpoint",
            Self::VignetteRoundness => "Roundness",
            Self::VignetteFeather => "Feather",
            Self::GrainSize => "Size",
            Self::GrainRoughness => "Roughness",
            Self::Sharpen => "Sharpening",
            Self::SharpenDetail => "Detail",
            Self::SharpenMasking => "Masking",
        }
    }

    pub const fn history_label(self) -> &'static str {
        self.label()
    }

    pub const fn min(self) -> f32 {
        match self {
            Self::Grain | Self::GrainSize | Self::GrainRoughness | Self::Sharpen | Self::SharpenDetail | Self::SharpenMasking
            | Self::VignetteMidpoint | Self::VignetteFeather | Self::ParametricSplit0 | Self::ParametricSplit1 | Self::ParametricSplit2 => 0.0,
            _ => -100.0,
        }
    }

    pub const fn max(self) -> f32 {
        match self {
            Self::Sharpen => 150.0,
            _ => 100.0,
        }
    }

    pub const fn default_value(self) -> f32 {
        match self {
            Self::VignetteMidpoint | Self::GrainSize | Self::GrainRoughness | Self::SharpenDetail | Self::VignetteFeather => 50.0,
            Self::ParametricSplit0 => 25.0,
            Self::ParametricSplit1 => 50.0,
            Self::ParametricSplit2 => 75.0,
            _ => 0.0,
        }
    }

    pub fn get(self, p: &DevelopParameters) -> f32 {
        match self {
            Self::MixHue(i) => p.mixer.bands[band_index(i)].hue,
            Self::MixSat(i) => p.mixer.bands[band_index(i)].sat,
            Self::MixLum(i) => p.mixer.bands[band_index(i)].lum,
            Self::BwMix(i) => p.bw.mix[band_index(i)],
            Self::ParametricHighlights => p.parametric.highlights,
            Self::ParametricLights => p.parametric.lights,
            Self::ParametricDarks => p.parametric.darks,
            Self::ParametricShadows => p.parametric.shadows,
            Self::ParametricSplit0 => p.parametric.split[0],
            Self::ParametricSplit1 => p.parametric.split[1],
            Self::ParametricSplit2 => p.parametric.split[2],
            Self::Texture => p.effects.texture,
            Self::Clarity => p.effects.clarity,
            Self::Dehaze => p.effects.dehaze,
            Self::Vignette => p.effects.vignette,
            Self::VignetteMidpoint => p.effects.vignette_midpoint,
            Self::VignetteRoundness => p.effects.vignette_roundness,
            Self::VignetteFeather => p.effects.vignette_feather,
            Self::Grain => p.effects.grain,
            Self::GrainSize => p.effects.grain_size,
            Self::GrainRoughness => p.effects.grain_roughness,
            Self::Sharpen => p.detail.sharpen,
            Self::SharpenDetail => p.detail.detail,
            Self::SharpenMasking => p.detail.masking,
        }
    }

    pub fn set(self, p: &mut DevelopParameters, value: f32) {
        let value = finite_clamp(value, self.min(), self.max(), self.default_value());
        match self {
            Self::MixHue(i) => p.mixer.bands[band_index(i)].hue = value,
            Self::MixSat(i) => p.mixer.bands[band_index(i)].sat = value,
            Self::MixLum(i) => p.mixer.bands[band_index(i)].lum = value,
            Self::BwMix(i) => p.bw.mix[band_index(i)] = value,
            Self::ParametricHighlights => p.parametric.highlights = value,
            Self::ParametricLights => p.parametric.lights = value,
            Self::ParametricDarks => p.parametric.darks = value,
            Self::ParametricShadows => p.parametric.shadows = value,
            Self::ParametricSplit0 => p.parametric.split[0] = value.min(p.parametric.split[1]),
            Self::ParametricSplit1 => {
                p.parametric.split[1] = value;
                p.parametric.split[0] = p.parametric.split[0].min(value);
                p.parametric.split[2] = p.parametric.split[2].max(value);
            }
            Self::ParametricSplit2 => p.parametric.split[2] = value.max(p.parametric.split[1]),
            Self::Texture => p.effects.texture = value,
            Self::Clarity => p.effects.clarity = value,
            Self::Dehaze => p.effects.dehaze = value,
            Self::Vignette => p.effects.vignette = value,
            Self::VignetteMidpoint => p.effects.vignette_midpoint = value,
            Self::VignetteRoundness => p.effects.vignette_roundness = value,
            Self::VignetteFeather => p.effects.vignette_feather = value,
            Self::Grain => p.effects.grain = value,
            Self::GrainSize => p.effects.grain_size = value,
            Self::GrainRoughness => p.effects.grain_roughness = value,
            Self::Sharpen => p.detail.sharpen = value,
            Self::SharpenDetail => p.detail.detail = value,
            Self::SharpenMasking => p.detail.masking = value,
        }
    }

    pub fn is_default(self, p: &DevelopParameters) -> bool {
        (self.get(p) - self.default_value()).abs() < 1.0e-4
    }

    pub fn format_value(self, value: f32) -> String {
        if value == 0.0 { "0".to_owned() } else { format!("{value:+.0}") }
    }
}

#[repr(C, align(16))]
#[derive(Debug, Clone, Copy, PartialEq, Pod, Zeroable)]
pub struct GpuAdjustUniforms {
    pub effects: [f32; 4],
    pub vignette: [f32; 4],
    pub grain: [f32; 4],
    pub detail: [f32; 4],
    pub air: [f32; 4],
    pub mixer_hue: [[f32; 4]; 2],
    pub mixer_sat: [[f32; 4]; 2],
    pub mixer_lum: [[f32; 4]; 2],
    pub bw_mix: [[f32; 4]; 2],
    pub parametric_amounts: [f32; 4],
    pub parametric_split: [f32; 4],
    pub flags: [u32; 4],
}

const _: () = assert!(std::mem::size_of::<GpuAdjustUniforms>() == 256);

impl GpuAdjustUniforms {
    pub fn build(p: &DevelopParameters, aspect: f32, air: [f32; 3]) -> Self {
        let mut out = Self::zeroed();
        out.effects = [p.effects.texture / 100.0 * 1.5, p.effects.clarity / 100.0, p.effects.dehaze / 100.0, aspect];
        out.vignette = [p.effects.vignette / 100.0, p.effects.vignette_midpoint / 100.0, p.effects.vignette_roundness / 100.0, p.effects.vignette_feather / 100.0];
        out.grain = [p.effects.grain / 100.0, p.effects.grain_size / 100.0, p.effects.grain_roughness / 100.0, 0.0];
        out.detail = [p.detail.sharpen / 150.0 * 3.0, p.detail.detail / 100.0, p.detail.masking / 100.0, 0.0];
        out.air = [air[0], air[1], air[2], 0.0];
        for i in 0..BAND_COUNT {
            let lane = i % 4;
            let row = i / 4;
            out.mixer_hue[row][lane] = p.mixer.bands[i].hue / 100.0;
            out.mixer_sat[row][lane] = p.mixer.bands[i].sat / 100.0;
            out.mixer_lum[row][lane] = p.mixer.bands[i].lum / 100.0;
            out.bw_mix[row][lane] = p.bw.mix[i] / 100.0;
        }
        out.parametric_amounts = [p.parametric.highlights / 100.0, p.parametric.lights / 100.0, p.parametric.darks / 100.0, p.parametric.shadows / 100.0];
        out.parametric_split = [p.parametric.split[0] / 100.0, p.parametric.split[1] / 100.0, p.parametric.split[2] / 100.0, 0.0];
        let enabled = p.effects.texture != 0.0 || p.effects.clarity != 0.0 || p.effects.dehaze != 0.0 || p.detail.sharpen > 0.0
            || p.effects.vignette != 0.0 || p.effects.grain > 0.0 || p.bw.enabled || !p.mixer.is_neutral() || !p.parametric.is_neutral();
        out.flags = [u32::from(enabled), u32::from(p.bw.enabled), u32::from(!p.mixer.is_neutral()), u32::from(!p.parametric.is_neutral())];
        out
    }
}

fn band_index(index: usize) -> usize {
    index.min(BAND_COUNT - 1)
}

fn finite_clamp(value: f32, min: f32, max: f32, default: f32) -> f32 {
    if value.is_finite() { value.clamp(min, max) } else { default }
}
