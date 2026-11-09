//! halation-color — working-space colour science and the reference (CPU) develop pipeline.
//!
//! * Working space: radiometrically linear ProPhoto RGB (ROMM). Luminance is always computed with the
//!   ProPhoto row `[0.28804, 0.71187, 0.00009]` — Rec.709 weights are only used inside the display
//!   gamut mapper, where the data really is linear sRGB.
//! * Every operator works on linear light. No 8-bit formulas, no clamping before the display stage.
//! * `PreparedDevelop::develop` is the single source of truth for the maths;
//!   `assets/shaders/develop_pipeline.wgsl` mirrors it line for line. Change one, change the other.
//!
//! Cargo: `glam = "0.29"`.

use glam::{DMat3, DVec3, Mat3, Vec3};

mod display;
mod extras;
mod icc;

pub use display::{DisplayLut, DisplayProfile, DISPLAY_LUT_SIZE, SHAPER_GAMMA};
pub use extras::sample_mask;
pub use icc::build_icc_profile;

// ------------------------------------------------------------------------------------------------
// Constants
// ------------------------------------------------------------------------------------------------

/// Linear ProPhoto RGB luminance row (sums to 1.0).
pub const PROPHOTO_LUMA: [f32; 3] = [0.28804, 0.71187, 0.00009];
/// Rec.709 / linear sRGB luminance row (display gamut mapping only).
pub const REC709_LUMA: [f32; 3] = [0.2126, 0.7152, 0.0722];
/// Scene-linear 18 % reflectance — pivot of the contrast operator.
pub const MID_GRAY: f32 = 0.18;
/// Temperature (K) at which white balance is the identity.
pub const REFERENCE_TEMPERATURE: f32 = 5500.0;
/// Stops of gain at ±100 for highlights / shadows.
pub const HIGHLIGHT_SHADOW_RANGE_EV: f32 = 1.5;

const TINT_DUV_PER_UNIT: f64 = 0.0002;
const MIN_LUMA: f32 = 1.0e-6;

const D50: DVec3 = DVec3::new(0.96422, 1.0, 0.82521);
const D65: DVec3 = DVec3::new(0.95047, 1.0, 1.08883);

#[inline]
pub fn luminance(rgb: [f32; 3]) -> f32 {
    rgb[0] * PROPHOTO_LUMA[0] + rgb[1] * PROPHOTO_LUMA[1] + rgb[2] * PROPHOTO_LUMA[2]
}

#[inline]
fn smoothstep(e0: f32, e1: f32, x: f32) -> f32 {
    let t = ((x - e0) / (e1 - e0)).clamp(0.0, 1.0);
    t * t * (3.0 - 2.0 * t)
}

// ------------------------------------------------------------------------------------------------
// Transfer functions
// ------------------------------------------------------------------------------------------------

/// sRGB-encoded [0,1] → linear light.
#[inline]
pub fn srgb_eotf(v: f32) -> f32 {
    if v <= 0.04045 {
        v / 12.92
    } else {
        ((v + 0.055) / 1.055).powf(2.4)
    }
}

/// Linear light → sRGB-encoded [0,1].
#[inline]
pub fn srgb_oetf(l: f32) -> f32 {
    if l <= 0.003_130_8 {
        l * 12.92
    } else {
        1.055 * l.powf(1.0 / 2.4) - 0.055
    }
}

// ------------------------------------------------------------------------------------------------
// Primaries & chromatic adaptation
// ------------------------------------------------------------------------------------------------

fn rows(r0: [f64; 3], r1: [f64; 3], r2: [f64; 3]) -> DMat3 {
    DMat3::from_cols(
        DVec3::from_array(r0),
        DVec3::from_array(r1),
        DVec3::from_array(r2),
    )
    .transpose()
}

fn srgb_to_xyz_d65() -> DMat3 {
    rows(
        [0.4124564, 0.3575761, 0.1804375],
        [0.2126729, 0.7151522, 0.0721750],
        [0.0193339, 0.1191920, 0.9503041],
    )
}

fn display_p3_to_xyz_d65() -> DMat3 {
    rows(
        [0.4865709, 0.2656677, 0.1982173],
        [0.2289746, 0.6917385, 0.0792869],
        [0.0000000, 0.0451134, 1.0439444],
    )
}

fn adobe_rgb_to_xyz_d65() -> DMat3 {
    rows(
        [0.5767309, 0.1855540, 0.1881852],
        [0.2973769, 0.6273491, 0.0752741],
        [0.0270343, 0.0706872, 0.9911085],
    )
}

fn prophoto_to_xyz_d50() -> DMat3 {
    rows(
        [0.7976749, 0.1351917, 0.0313534],
        [0.2880402, 0.7118741, 0.0000857],
        [0.0000000, 0.0000000, 0.8252100],
    )
}

fn bradford() -> DMat3 {
    rows(
        [0.8951, 0.2664, -0.1614],
        [-0.7502, 1.7135, 0.0367],
        [0.0389, -0.0685, 1.0296],
    )
}

/// Bradford chromatic adaptation matrix (XYZ → XYZ) from `src_white` to `dst_white`.
fn bradford_cat(src_white: DVec3, dst_white: DVec3) -> DMat3 {
    let m = bradford();
    let (s, d) = (m * src_white, m * dst_white);
    m.inverse() * DMat3::from_diagonal(DVec3::new(d.x / s.x, d.y / s.y, d.z / s.z)) * m
}

fn rgb_to_rgb(src_to_xyz: DMat3, src_white: DVec3, dst_to_xyz: DMat3, dst_white: DVec3) -> DMat3 {
    dst_to_xyz.inverse() * bradford_cat(src_white, dst_white) * src_to_xyz
}

/// Linear sRGB → linear ProPhoto (working space).
pub fn srgb_to_working() -> Mat3 {
    rgb_to_rgb(srgb_to_xyz_d65(), D65, prophoto_to_xyz_d50(), D50).as_mat3()
}

/// Linear ProPhoto (working space) → linear sRGB.
pub fn working_to_srgb() -> Mat3 {
    rgb_to_rgb(prophoto_to_xyz_d50(), D50, srgb_to_xyz_d65(), D65).as_mat3()
}

/// Output colour spaces for export / ICC embedding.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum OutputSpace {
    Srgb,
    DisplayP3,
    AdobeRgb,
    ProPhoto,
}

impl OutputSpace {
    /// Linear working space → linear output primaries.
    pub fn matrix_from_working(self) -> Mat3 {
        match self {
            OutputSpace::Srgb => working_to_srgb(),
            OutputSpace::DisplayP3 => {
                rgb_to_rgb(prophoto_to_xyz_d50(), D50, display_p3_to_xyz_d65(), D65).as_mat3()
            }
            OutputSpace::AdobeRgb => {
                rgb_to_rgb(prophoto_to_xyz_d50(), D50, adobe_rgb_to_xyz_d65(), D65).as_mat3()
            }
            OutputSpace::ProPhoto => Mat3::IDENTITY,
        }
    }

    /// Linear → encoded transfer function of the space.
    pub fn encode(self, linear: f32) -> f32 {
        let l = linear.clamp(0.0, 1.0);
        match self {
            OutputSpace::Srgb | OutputSpace::DisplayP3 => srgb_oetf(l),
            OutputSpace::AdobeRgb => l.powf(256.0 / 563.0),
            OutputSpace::ProPhoto => l.powf(1.0 / 1.8),
        }
    }

    pub fn name(self) -> &'static str {
        match self {
            OutputSpace::Srgb => "sRGB",
            OutputSpace::DisplayP3 => "Display P3",
            OutputSpace::AdobeRgb => "Adobe RGB (1998)",
            OutputSpace::ProPhoto => "ProPhoto RGB",
        }
    }
}

/// An output space with its working-space matrix resolved once (cheap to use per pixel).
#[derive(Debug, Clone, Copy)]
pub struct OutputTransform {
    pub space: OutputSpace,
    matrix: Mat3,
}

impl OutputTransform {
    pub fn new(space: OutputSpace) -> Self {
        Self {
            space,
            matrix: space.matrix_from_working(),
        }
    }
}

// ------------------------------------------------------------------------------------------------
// White balance (temperature / tint → Bradford matrix in the working space)
// ------------------------------------------------------------------------------------------------

/// Kim et al. cubic-spline approximation of the Planckian locus (1667 K – 25000 K, clamped outside).
fn planckian_xy(kelvin: f64) -> (f64, f64) {
    let t = kelvin.clamp(1667.0, 25000.0);
    let (t2, t3) = (t * t, t * t * t);
    let x = if t <= 4000.0 {
        -0.2661239e9 / t3 - 0.2343589e6 / t2 + 0.8776956e3 / t + 0.179910
    } else {
        -3.0258469e9 / t3 + 2.1070379e6 / t2 + 0.2226347e3 / t + 0.240390
    };
    let y = if t <= 2222.0 {
        ((-1.1063814 * x - 1.34811020) * x + 2.18555832) * x - 0.20219683
    } else if t <= 4000.0 {
        ((-0.9549476 * x - 1.37418593) * x + 2.09137015) * x - 0.16748867
    } else {
        ((3.0817580 * x - 5.87338670) * x + 3.75112997) * x - 0.37001483
    };
    (x, y)
}

/// XYZ (Y = 1) of the *assumed scene illuminant*. Tint shifts along CIE 1960 `v` (+ = greener light).
fn illuminant_xyz(temperature: f64, tint: f64) -> DVec3 {
    let (x, y) = planckian_xy(temperature);
    let d = 12.0 * y - 2.0 * x + 3.0;
    let (u, v) = (4.0 * x / d, 6.0 * y / d);
    let v = v + tint * TINT_DUV_PER_UNIT;
    let d2 = 2.0 * u - 8.0 * v + 4.0;
    let (x, y) = (3.0 * u / d2, 2.0 * v / d2);
    DVec3::new(x / y, 1.0, (1.0 - x - y) / y)
}

/// Lightroom-style white balance in linear ProPhoto: the sliders state which illuminant the scene was
/// lit by, so a higher temperature (bluer assumed light) renders the image warmer and a positive
/// tint (greener assumed light) renders it more magenta. `5500 K / 0` is the exact identity and white
/// keeps unit luminance, so white balance never changes exposure.
pub fn white_balance_matrix(temperature: f32, tint: f32) -> Mat3 {
    if (temperature - REFERENCE_TEMPERATURE).abs() < 0.5 && tint.abs() < 1.0e-4 {
        return Mat3::IDENTITY;
    }
    let src = illuminant_xyz(temperature as f64, tint as f64);
    let dst = illuminant_xyz(REFERENCE_TEMPERATURE as f64, 0.0);
    let pp = prophoto_to_xyz_d50();
    let m = pp.inverse() * bradford_cat(src, dst) * pp;
    let white = m * DVec3::ONE;
    let y = white.x * PROPHOTO_LUMA[0] as f64
        + white.y * PROPHOTO_LUMA[1] as f64
        + white.z * PROPHOTO_LUMA[2] as f64;
    (m * (1.0 / y)).as_mat3()
}

// ------------------------------------------------------------------------------------------------
// Tonal operators (slider units in, linear light in / out)
// ------------------------------------------------------------------------------------------------

pub fn white_balance(rgb: [f32; 3], temperature: f32, tint: f32) -> [f32; 3] {
    (white_balance_matrix(temperature, tint) * Vec3::from_array(rgb))
        .max(Vec3::ZERO)
        .to_array()
}

pub fn apply_exposure(rgb: [f32; 3], ev: f32) -> [f32; 3] {
    (Vec3::from_array(rgb) * ev.exp2()).to_array()
}

/// Highlights / shadows: smooth masks over *stops from middle gray*, applied as a luminance-driven gain
/// so hue and chroma ratios are preserved. Shadows act below 0 EV (full below −6 EV); highlights act
/// above 0 EV (full at +3 EV). ±100 ⇒ ±`HIGHLIGHT_SHADOW_RANGE_EV` stops.
pub fn apply_highlights_shadows(rgb: [f32; 3], highlights: f32, shadows: f32) -> [f32; 3] {
    let k = HIGHLIGHT_SHADOW_RANGE_EV / 100.0;
    tone_highlights_shadows(Vec3::from_array(rgb), highlights * k, shadows * k).to_array()
}

fn tone_highlights_shadows(c: Vec3, highlights_ev: f32, shadows_ev: f32) -> Vec3 {
    let l = c.dot(Vec3::from_array(PROPHOTO_LUMA));
    if l <= MIN_LUMA {
        return c;
    }
    let stops = (l / MID_GRAY).log2();
    let w_shadow = 1.0 - smoothstep(-6.0, 0.0, stops);
    let w_highlight = smoothstep(0.0, 3.0, stops);
    c * (shadows_ev * w_shadow + highlights_ev * w_highlight).exp2()
}

/// Whites / blacks: black point moves by ∓0.02 and white point by ∓0.25 (linear) at ±100.
pub fn apply_whites_blacks(rgb: [f32; 3], whites: f32, blacks: f32) -> [f32; 3] {
    let (black, inv) = black_white_points(whites, blacks);
    ((Vec3::from_array(rgb) - Vec3::splat(black)) * inv)
        .max(Vec3::ZERO)
        .to_array()
}

fn black_white_points(whites: f32, blacks: f32) -> (f32, f32) {
    let black = -blacks.clamp(-100.0, 100.0) * 0.0002;
    let white = 1.0 - whites.clamp(-100.0, 100.0) * 0.0025;
    (black, 1.0 / (white - black).max(0.05))
}

/// Log-domain contrast about 18 % gray: stops-from-gray are scaled by γ = 1 + 0.005·contrast, i.e.
/// `0.18·(c / 0.18)^γ`. Pivot, zero and monotonicity are preserved — deep shadows never invert and
/// nothing goes negative.
pub fn apply_contrast(rgb: [f32; 3], contrast: f32) -> [f32; 3] {
    contrast_gamma(
        Vec3::from_array(rgb),
        1.0 + contrast.clamp(-100.0, 100.0) * 0.005,
    )
    .to_array()
}

fn contrast_gamma(c: Vec3, gamma: f32) -> Vec3 {
    let f = |v: f32| MID_GRAY * (v.max(1.0e-10) / MID_GRAY).powf(gamma);
    Vec3::new(f(c.x), f(c.y), f(c.z))
}

/// Vibrance (boosts low-saturation colours more) and saturation, both scaling chroma around ProPhoto luma.
pub fn apply_vibrance_saturation(rgb: [f32; 3], vibrance: f32, saturation: f32) -> [f32; 3] {
    vibrance_saturation(Vec3::from_array(rgb), vibrance * 0.01, saturation * 0.01).to_array()
}

fn vibrance_saturation(c: Vec3, vibrance: f32, saturation: f32) -> Vec3 {
    let luma = c.dot(Vec3::from_array(PROPHOTO_LUMA));
    let (max_c, min_c) = (c.max_element(), c.min_element());
    let current = (max_c - min_c) / (max_c + 0.001);
    let gain = (1.0 + vibrance * (1.0 - current)) * (1.0 + saturation);
    (Vec3::splat(luma) + (c - Vec3::splat(luma)) * gain).max(Vec3::ZERO)
}

/// Luminance-preserving desaturation of negative (out-of-gamut) linear sRGB, then clamp to [0, 1].
fn gamut_map(c: Vec3) -> Vec3 {
    let l = c.dot(Vec3::from_array(REC709_LUMA));
    let min = c.min_element();
    let c = if min < 0.0 && l > 0.0 {
        Vec3::splat(l) + (c - Vec3::splat(l)) * (l / (l - min))
    } else {
        c
    };
    c.clamp(Vec3::ZERO, Vec3::ONE)
}

// ------------------------------------------------------------------------------------------------
// Prepared develop pipeline
// ------------------------------------------------------------------------------------------------

/// Slider values (same units as `halation_core::DevelopParameters`; this crate stays dependency-free).
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct DevelopInputs {
    pub exposure: f32,
    pub contrast: f32,
    pub highlights: f32,
    pub shadows: f32,
    pub whites: f32,
    pub blacks: f32,
    pub temperature: f32,
    pub tint: f32,
    pub vibrance: f32,
    pub saturation: f32,
}

impl Default for DevelopInputs {
    fn default() -> Self {
        Self {
            exposure: 0.0,
            contrast: 0.0,
            highlights: 0.0,
            shadows: 0.0,
            whites: 0.0,
            blacks: 0.0,
            temperature: REFERENCE_TEMPERATURE,
            tint: 0.0,
            vibrance: 0.0,
            saturation: 0.0,
        }
    }
}

/// All per-frame constants resolved once, so per-pixel work is a handful of multiplies.
/// Stage order: white balance → exposure → highlights/shadows → whites/blacks → contrast →
/// vibrance/saturation → display transform (matrix, gamut map, sRGB OETF).
#[derive(Debug, Clone)]
pub struct PreparedDevelop {
    extras: Option<Box<extras::Extras>>,
    lut: Option<std::sync::Arc<DisplayLut>>,
    wb: Mat3,
    to_display: Mat3,
    exposure_gain: f32,
    highlights_ev: f32,
    shadows_ev: f32,
    black_point: f32,
    inv_range: f32,
    contrast_gamma: f32,
    vibrance: f32,
    saturation: f32,
}

impl PreparedDevelop {
    pub fn new(i: &DevelopInputs) -> Self {
        let k = HIGHLIGHT_SHADOW_RANGE_EV / 100.0;
        let (black_point, inv_range) = black_white_points(i.whites, i.blacks);
        Self {
            extras: None,
            lut: None,
            wb: white_balance_matrix(i.temperature, i.tint),
            to_display: working_to_srgb(),
            exposure_gain: i.exposure.clamp(-10.0, 10.0).exp2(),
            highlights_ev: i.highlights.clamp(-100.0, 100.0) * k,
            shadows_ev: i.shadows.clamp(-100.0, 100.0) * k,
            black_point,
            inv_range,
            contrast_gamma: 1.0 + i.contrast.clamp(-100.0, 100.0) * 0.005,
            vibrance: i.vibrance.clamp(-100.0, 100.0) * 0.01,
            saturation: i.saturation.clamp(-100.0, 100.0) * 0.01,
        }
    }

    /// White-balance matrix as three columns (feed to the GPU uniform).
    pub fn wb_columns(&self) -> [[f32; 3]; 3] {
        self.wb.to_cols_array_2d()
    }

    /// Working space → linear sRGB matrix as three columns (feed to the GPU uniform).
    pub fn display_columns(&self) -> [[f32; 3]; 3] {
        self.to_display.to_cols_array_2d()
    }

    /// Working-space linear RGB → display-referred *linear* sRGB in [0, 1].
    pub fn develop(&self, rgb: [f32; 3]) -> [f32; 3] {
        self.develop_masked(rgb, 0.0)
    }

    /// Like [`develop`](Self::develop) with the local-adjustment mask value (0..1) at this pixel.
    /// With a display LUT installed the result is already display-encoded.
    pub fn develop_masked(&self, rgb: [f32; 3], mask: f32) -> [f32; 3] {
        let c = self.stages(rgb, mask);
        match &self.lut {
            Some(lut) => lut.apply(c.to_array()),
            None => gamut_map(self.to_display * c).to_array(),
        }
    }

    /// Result of every creative stage in the linear working space (before any output transform).
    pub fn develop_working(&self, rgb: [f32; 3], mask: f32) -> [f32; 3] {
        self.stages(rgb, mask).to_array()
    }

    /// Output-referred, *encoded* result in `space` in [0, 1]. Used by export and external-editor
    /// hand-off: it never applies the monitor LUT.
    pub fn develop_to_output(&self, rgb: [f32; 3], mask: f32, out: &OutputTransform) -> [f32; 3] {
        let d = gamut_map(out.matrix * self.stages(rgb, mask));
        [
            out.space.encode(d.x),
            out.space.encode(d.y),
            out.space.encode(d.z),
        ]
    }

    fn stages(&self, rgb: [f32; 3], mask: f32) -> Vec3 {
        let mut c = (self.wb * Vec3::from_array(rgb)).max(Vec3::ZERO);
        c *= self.exposure_gain;
        if self.highlights_ev != 0.0 || self.shadows_ev != 0.0 {
            c = tone_highlights_shadows(c, self.highlights_ev, self.shadows_ev);
        }
        c = ((c - Vec3::splat(self.black_point)) * self.inv_range).max(Vec3::ZERO);
        if self.contrast_gamma != 1.0 {
            c = contrast_gamma(c, self.contrast_gamma);
        }
        if let Some(x) = &self.extras {
            c = x.curves_and_grading(c);
        }
        if self.vibrance != 0.0 || self.saturation != 0.0 {
            c = vibrance_saturation(c, self.vibrance, self.saturation);
        }
        if let Some(x) = &self.extras {
            c = x.local(c, mask);
        }
        c
    }

    /// Same as [`develop`](Self::develop) followed by the sRGB OETF, rounded to 8 bits.
    pub fn develop_srgb8(&self, rgb: [f32; 3]) -> [u8; 3] {
        self.develop_srgb8_masked(rgb, 0.0)
    }

    /// 8-bit display output: LUT-encoded when a display LUT is installed, sRGB otherwise.
    pub fn develop_srgb8_masked(&self, rgb: [f32; 3], mask: f32) -> [u8; 3] {
        let c = self.develop_masked(rgb, mask);
        if self.lut.is_some() {
            return [to8(c[0]), to8(c[1]), to8(c[2])];
        }
        [encode8(c[0]), encode8(c[1]), encode8(c[2])]
    }

    /// Install a monitor-profile LUT in place of the built-in sRGB display transform.
    pub fn with_display_lut(mut self, lut: Option<std::sync::Arc<DisplayLut>>) -> Self {
        self.lut = lut;
        self
    }
}

#[inline]
fn encode8(linear: f32) -> u8 {
    (srgb_oetf(linear) * 255.0 + 0.5) as u8
}

#[inline]
fn to8(encoded: f32) -> u8 {
    (encoded.clamp(0.0, 1.0) * 255.0 + 0.5) as u8
}

// ------------------------------------------------------------------------------------------------
// Tests
// ------------------------------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn prophoto_luma_sums_to_one() {
        assert!((PROPHOTO_LUMA.iter().sum::<f32>() - 1.0).abs() < 1.0e-5);
    }

    #[test]
    fn matrices_match_published_values() {
        // columns of linear-sRGB → linear-ProPhoto (also embedded in halation-raw)
        let m = srgb_to_working().to_cols_array_2d();
        let expect = [
            [0.5293468, 0.0983738, 0.0168829],
            [0.3300729, 0.8734610, 0.1176725],
            [0.1405813, 0.0281646, 0.8654443],
        ];
        for c in 0..3 {
            for r in 0..3 {
                assert!(
                    (m[c][r] - expect[c][r]).abs() < 3.0e-4,
                    "col {c} row {r}: {}",
                    m[c][r]
                );
            }
        }
        assert!((white_balance_matrix(5500.0, 0.0) * Vec3::ONE - Vec3::ONE).length() < 1.0e-6);
    }

    #[test]
    fn defaults_are_a_pure_display_transform() {
        let p = PreparedDevelop::new(&DevelopInputs::default());
        assert_eq!(p.develop_srgb8([0.0; 3]), [0, 0, 0]);
        assert_eq!(p.develop_srgb8([1.0; 3]), [255, 255, 255]);
        let g = p.develop_srgb8([MID_GRAY; 3]);
        assert!(g.iter().all(|&v| (117..=119).contains(&v)), "{g:?}");
    }

    #[test]
    fn warmer_with_higher_temperature_and_magenta_with_positive_tint() {
        let warm = white_balance([0.18; 3], 8000.0, 0.0);
        assert!(warm[0] > warm[2]);
        let cool = white_balance([0.18; 3], 3500.0, 0.0);
        assert!(cool[2] > cool[0]);
        let magenta = white_balance([0.18; 3], 5500.0, 80.0);
        assert!(magenta[1] < magenta[0] && magenta[1] < magenta[2]);
        assert!((luminance(white_balance([1.0; 3], 8000.0, 0.0)) - 1.0).abs() < 1.0e-3);
    }

    #[test]
    fn contrast_pivots_on_mid_gray_and_never_inverts() {
        let g = apply_contrast([MID_GRAY; 3], 100.0);
        assert!((g[0] - MID_GRAY).abs() < 1.0e-5);
        let (lo, hi) = (
            apply_contrast([0.01; 3], 100.0)[0],
            apply_contrast([0.5; 3], 100.0)[0],
        );
        assert!(lo < 0.01 && lo >= 0.0 && hi > 0.5);
        assert!(apply_contrast([0.0; 3], 100.0)[0] < 1.0e-8);
    }

    #[test]
    fn highlights_and_shadows_are_signed_and_localised() {
        let lifted = apply_highlights_shadows([0.01; 3], 0.0, 100.0)[0];
        assert!(lifted > 0.01 * 2.0);
        let mid = apply_highlights_shadows([MID_GRAY; 3], 100.0, 100.0)[0];
        assert!((mid - MID_GRAY).abs() < 0.01);
        let recovered = apply_highlights_shadows([1.0; 3], -100.0, 0.0)[0];
        assert!(recovered < 0.5);
    }
}
