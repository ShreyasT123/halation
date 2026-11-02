use halation_core::ImageBufferF32;
use std::{fs::File, io::BufReader, path::Path};
use thiserror::Error;

#[derive(Debug, Clone, Default)]
pub struct RawMetadata {
    pub camera: String,
    pub lens: String,
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
    pub buffer: ImageBufferF32,
}

#[derive(Debug, Error)]
pub enum DecodeError {
    #[error("image decode failed: {0}")]
    Image(#[from] image::ImageError),
    #[error("could not open file: {0}")]
    Io(#[from] std::io::Error),
}

pub fn decode_file(path: &Path) -> Result<RawAsset, DecodeError> {
    let decoded = image::open(path)?;
    let rgba = decoded.to_rgb32f();
    let (width, height) = rgba.dimensions();
    let pixels = rgba.pixels().map(|p| [p[0], p[1], p[2]]).collect();
    let metadata = read_metadata(path);
    Ok(RawAsset {
        id: 0,
        filename: path
            .file_name()
            .and_then(|n| n.to_str())
            .unwrap_or("Untitled")
            .to_owned(),
        metadata,
        buffer: ImageBufferF32 {
            width,
            height,
            pixels,
        },
    })
}

fn read_metadata(path: &Path) -> RawMetadata {
    let mut metadata = RawMetadata::default();
    let Ok(file) = File::open(path) else {
        return metadata;
    };
    let Ok(exif) = exif::Reader::new().read_from_container(&mut BufReader::new(file)) else {
        return metadata;
    };
    let field = |tag| {
        exif.get_field(tag, exif::In::PRIMARY)
            .map(|f| f.display_value().with_unit(&exif).to_string())
    };
    metadata.camera = [field(exif::Tag::Make), field(exif::Tag::Model)]
        .into_iter()
        .flatten()
        .filter(|s| !s.is_empty())
        .collect::<Vec<_>>()
        .join(" ");
    metadata.lens = field(exif::Tag::LensModel).unwrap_or_default();
    metadata.iso = exif
        .get_field(exif::Tag::PhotographicSensitivity, exif::In::PRIMARY)
        .and_then(|f| f.value.get_uint(0));
    metadata.shutter = field(exif::Tag::ExposureTime).unwrap_or_default();
    metadata.aperture = field(exif::Tag::FNumber).unwrap_or_default();
    metadata.captured_at = field(exif::Tag::DateTimeOriginal);
    metadata
}
