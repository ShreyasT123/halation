//! halation-raw — image ingestion.
//!
//! Everything that leaves this crate is **linear ProPhoto RGB** (the working space): files decoded by
//! the `image` crate are sRGB-encoded, so they are linearised and re-primaried here instead of being
//! fed to the develop pipeline as if they were already linear light.
//!
//! Camera RAW (`.arw .cr2 .cr3 .nef .orf .rw2`) is catalogued and thumbnailed through its embedded
//! EXIF preview; full sensor decode arrives with the LibRaw FFI backend and plugs into [`decode_file`].
//!
//! Cargo: `image`, `kamadak-exif` (as `exif`), `thiserror`, `halation-core`.

use halation_core::ImageBufferF32;
use std::{fs::File, io::BufReader, path::Path, sync::OnceLock};
use thiserror::Error;

/// Lower-case extensions Halation offers in import dialogs and folder scans.
pub const SUPPORTED_EXTENSIONS: &[&str] = &[
    "arw", "cr2", "cr3", "nef", "orf", "rw2", "dng", "tif", "tiff", "jpg", "jpeg", "png", "webp",
];

/// Linear sRGB → linear ProPhoto (Bradford-adapted D65 → D50). Row-major; verified against
/// `halation_color::srgb_to_working` by that crate's test-suite.
const SRGB_TO_PROPHOTO: [[f32; 3]; 3] = [
    [0.5293468, 0.3300729, 0.1405813],
    [0.0983738, 0.8734610, 0.0281646],
    [0.0168829, 0.1176725, 0.8654443],
];

const LUT_SIZE: usize = 4096;

#[derive(Debug, Clone, Default)]
pub struct RawMetadata {
    pub camera: String,
    pub lens: String,
    pub focal_length: String,
    pub iso: Option<u32>,
    pub shutter: String,
    pub aperture: String,
    pub captured_at: Option<String>,
}

#[derive(Debug, Clone)]
pub struct RawAsset {
    pub id: u64,
    pub filename: String,
    pub metadata: RawMetadata,
    /// Linear ProPhoto RGB, unclamped.
    pub buffer: ImageBufferF32,
}

/// 8-bit RGBA preview (sRGB encoded) for catalog thumbnails.
#[derive(Debug, Clone)]
pub struct Thumbnail {
    pub width: u32,
    pub height: u32,
    pub rgba: Vec<u8>,
}

#[derive(Debug, Error)]
pub enum DecodeError {
    #[error("image decode failed: {0}")]
    Image(#[from] image::ImageError),
    #[error("could not open file: {0}")]
    Io(#[from] std::io::Error),
}

fn srgb_lut() -> &'static [f32] {
    static LUT: OnceLock<Vec<f32>> = OnceLock::new();
    LUT.get_or_init(|| {
        (0..=LUT_SIZE)
            .map(|i| {
                let v = i as f32 / LUT_SIZE as f32;
                if v <= 0.04045 {
                    v / 12.92
                } else {
                    ((v + 0.055) / 1.055).powf(2.4)
                }
            })
            .collect()
    })
}

#[inline]
fn linearize(lut: &[f32], v: f32) -> f32 {
    let x = v.clamp(0.0, 1.0) * LUT_SIZE as f32;
    let i = x as usize;
    let f = x - i as f32;
    let a = lut[i];
    let b = lut[(i + 1).min(LUT_SIZE)];
    a + (b - a) * f
}

#[inline]
fn to_working(lut: &[f32], p: &[f32]) -> [f32; 3] {
    let (r, g, b) = (linearize(lut, p[0]), linearize(lut, p[1]), linearize(lut, p[2]));
    let m = &SRGB_TO_PROPHOTO;
    [
        m[0][0] * r + m[0][1] * g + m[0][2] * b,
        m[1][0] * r + m[1][1] * g + m[1][2] * b,
        m[2][0] * r + m[2][1] * g + m[2][2] * b,
    ]
}

pub fn decode_file(path: &Path) -> Result<RawAsset, DecodeError> {
    let rgb = image::open(path)?.to_rgb32f();
    let (width, height) = rgb.dimensions();
    let lut = srgb_lut();
    let pixels: Vec<[f32; 3]> = rgb.as_raw().chunks_exact(3).map(|p| to_working(lut, p)).collect();
    Ok(RawAsset {
        id: 0,
        filename: path.file_name().and_then(|n| n.to_str()).unwrap_or("Untitled").to_owned(),
        metadata: read_metadata(path),
        buffer: ImageBufferF32::new(width, height, pixels),
    })
}

/// Fast catalog thumbnail (longest edge ≤ `max_dim`). Falls back to the JPEG preview embedded in the
/// EXIF block when the container is not decodable by `image` (typical for camera RAW).
pub fn decode_thumbnail(path: &Path, max_dim: u32) -> Result<Thumbnail, DecodeError> {
    let img = match image::open(path) {
        Ok(img) => img,
        Err(primary) => embedded_preview(path).ok_or(primary)?,
    };
    let thumb = img.thumbnail(max_dim, max_dim).to_rgba8();
    let (width, height) = thumb.dimensions();
    Ok(Thumbnail { width, height, rgba: thumb.into_raw() })
}

fn embedded_preview(path: &Path) -> Option<image::DynamicImage> {
    let file = File::open(path).ok()?;
    let exif = exif::Reader::new().read_from_container(&mut BufReader::new(file)).ok()?;
    let offset = exif.get_field(exif::Tag::JPEGInterchangeFormat, exif::In::THUMBNAIL)?.value.get_uint(0)? as usize;
    let length = exif.get_field(exif::Tag::JPEGInterchangeFormatLength, exif::In::THUMBNAIL)?.value.get_uint(0)? as usize;
    let bytes = exif.buf().get(offset..offset.checked_add(length)?)?;
    image::load_from_memory(bytes).ok()
}

/// EXIF metadata without decoding pixels (cheap enough to run during import).
pub fn read_metadata(path: &Path) -> RawMetadata {
    let mut metadata = RawMetadata::default();
    let Ok(file) = File::open(path) else {
        return metadata;
    };
    let Ok(exif) = exif::Reader::new().read_from_container(&mut BufReader::new(file)) else {
        return metadata;
    };
    let field = |tag| exif.get_field(tag, exif::In::PRIMARY).map(|f| f.display_value().with_unit(&exif).to_string());
    metadata.camera = [field(exif::Tag::Make), field(exif::Tag::Model)]
        .into_iter()
        .flatten()
        .map(|s| s.trim_matches('"').trim().to_owned())
        .filter(|s| !s.is_empty())
        .collect::<Vec<_>>()
        .join(" ");
    metadata.lens = field(exif::Tag::LensModel).map(|s| s.trim_matches('"').to_owned()).unwrap_or_default();
    metadata.focal_length = field(exif::Tag::FocalLength).unwrap_or_default();
    metadata.iso = exif
        .get_field(exif::Tag::PhotographicSensitivity, exif::In::PRIMARY)
        .and_then(|f| f.value.get_uint(0));
    metadata.shutter = field(exif::Tag::ExposureTime).unwrap_or_default();
    metadata.aperture = field(exif::Tag::FNumber).unwrap_or_default();
    metadata.captured_at = field(exif::Tag::DateTimeOriginal);
    metadata
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn neutral_gray_stays_neutral() {
        let lut = srgb_lut();
        let p = to_working(lut, &[0.5, 0.5, 0.5]);
        assert!((p[0] - p[1]).abs() < 1.0e-4 && (p[1] - p[2]).abs() < 1.0e-4);
        assert!((p[0] - 0.2140).abs() < 1.0e-3);
    }

    #[test]
    fn matrix_rows_sum_to_one() {
        for row in SRGB_TO_PROPHOTO {
            assert!((row.iter().sum::<f32>() - 1.0).abs() < 1.0e-4);
        }
    }
}
