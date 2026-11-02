use image::{imageops::FilterType, GenericImageView, GrayImage, ImageBuffer, Luma};
use ort::{
    execution_providers::DirectMLExecutionProvider,
    session::{builder::GraphOptimizationLevel, Session},
    value::Tensor,
};
use std::path::{Path, PathBuf};

const MODEL_SIZE: u32 = 1024;
const MASK_SIZE: u32 = 256;

pub struct MobileSam {
    encoder: Session,
    decoder: Session,
    embedding: Option<([u32; 2], Vec<f32>)>,
    cached_path: Option<PathBuf>,
}

impl MobileSam {
    pub fn load(model_dir: &Path) -> Result<Self, String> {
        let encoder_path = model_dir.join("encoder.onnx");
        let decoder_path = model_dir.join("decoder.onnx");
        if !encoder_path.is_file() || !decoder_path.is_file() {
            return Err(format!(
                "MobileSAM encoder/decoder not found in {}",
                model_dir.display()
            ));
        }
        let make_session = |path: &Path| -> Result<Session, String> {
            let builder = Session::builder()
                .map_err(|e| e.to_string())?
                .with_optimization_level(GraphOptimizationLevel::Level1)
                .map_err(|e| e.to_string())?;
            // DirectML is preferred on Windows; ONNX Runtime keeps CPU as fallback.
            let with_directml = builder
                .with_execution_providers([DirectMLExecutionProvider::default().build()])
                .map_err(|e| e.to_string())?;
            match with_directml.clone().commit_from_file(path) {
                Ok(session) => Ok(session),
                Err(_) => Session::builder()
                    .map_err(|e| e.to_string())?
                    .with_optimization_level(GraphOptimizationLevel::Level1)
                    .map_err(|e| e.to_string())?
                    .commit_from_file(path)
                    .map_err(|e| e.to_string()),
            }
        };
        Ok(Self {
            encoder: make_session(&encoder_path)?,
            decoder: make_session(&decoder_path)?,
            embedding: None,
            cached_path: None,
        })
    }

    /// Produce a full-resolution 8-bit mask from a positive point prompt in
    /// normalized source-image coordinates (0..1).
    pub fn select_point(&mut self, image_path: &Path, point: [f32; 2]) -> Result<Mask, String> {
        let canonical = image_path
            .canonicalize()
            .unwrap_or_else(|_| image_path.to_path_buf());
        if self.cached_path.as_deref() != Some(canonical.as_path()) {
            let image = image::open(image_path)
                .map_err(|e| e.to_string())?
                .to_rgb8();
            self.embedding = Some(self.encode(&image)?);
            self.cached_path = Some(canonical);
        }
        let (scaled, embedding) = self
            .embedding
            .as_ref()
            .ok_or("MobileSAM embedding was not produced")?;
        let x = point[0].clamp(0.0, 1.0) * scaled[0] as f32;
        let y = point[1].clamp(0.0, 1.0) * scaled[1] as f32;
        let embedding_tensor = Tensor::from_array(([1usize, 256, 64, 64], embedding.clone()))
            .map_err(|e| e.to_string())?;
        let coords = Tensor::from_array(([1usize, 1, 2], vec![x, y])).map_err(|e| e.to_string())?;
        let labels = Tensor::from_array(([1usize, 1], vec![1.0_f32])).map_err(|e| e.to_string())?;
        let outputs = self
            .decoder
            .run(
                ort::inputs![
                    "image_embeddings" => embedding_tensor,
                    "point_coords" => coords,
                    "point_labels" => labels,
                ]
                .map_err(|e| e.to_string())?,
            )
            .map_err(|e| e.to_string())?;
        let (_, logits) = outputs["masks"]
            .try_extract_raw_tensor::<f32>()
            .map_err(|e| e.to_string())?;
        let low = GrayImage::from_raw(
            MASK_SIZE,
            MASK_SIZE,
            logits
                .iter()
                .map(|v| if *v > 0.0 { 255 } else { 0 })
                .collect(),
        )
        .ok_or("MobileSAM returned an invalid mask tensor")?;
        let padded = image::imageops::resize(&low, MODEL_SIZE, MODEL_SIZE, FilterType::Triangle);
        let scaled_mask = image::imageops::crop_imm(&padded, 0, 0, scaled[0], scaled[1]).to_image();
        let original = image::open(image_path).map_err(|e| e.to_string())?;
        let (width, height) = original.dimensions();
        let full = image::imageops::resize(&scaled_mask, width, height, FilterType::Nearest);
        Ok(Mask {
            width,
            height,
            pixels: full.into_raw(),
        })
    }

    fn encode(&mut self, image: &image::RgbImage) -> Result<([u32; 2], Vec<f32>), String> {
        let scale = MODEL_SIZE as f32 / image.width().max(image.height()) as f32;
        let width = ((image.width() as f32 * scale).round() as u32).clamp(1, MODEL_SIZE);
        let height = ((image.height() as f32 * scale).round() as u32).clamp(1, MODEL_SIZE);
        let resized = image::imageops::resize(image, width, height, FilterType::Triangle);
        let mut chw = vec![0.0_f32; 3 * MODEL_SIZE as usize * MODEL_SIZE as usize];
        let plane = (MODEL_SIZE * MODEL_SIZE) as usize;
        for y in 0..height {
            for x in 0..width {
                let pixel = resized.get_pixel(x, y).0;
                let index = (y * MODEL_SIZE + x) as usize;
                chw[index] = pixel[0] as f32 / 255.0;
                chw[plane + index] = pixel[1] as f32 / 255.0;
                chw[2 * plane + index] = pixel[2] as f32 / 255.0;
            }
        }
        let input =
            Tensor::from_array(([1usize, 3, MODEL_SIZE as usize, MODEL_SIZE as usize], chw))
                .map_err(|e| e.to_string())?;
        let outputs = self
            .encoder
            .run(ort::inputs!["image" => input].map_err(|e| e.to_string())?)
            .map_err(|e| e.to_string())?;
        let (_, embedding) = outputs["image_embeddings"]
            .try_extract_raw_tensor::<f32>()
            .map_err(|e| e.to_string())?;
        Ok(([width, height], embedding.to_vec()))
    }
}

#[derive(Debug, Clone)]
pub struct Mask {
    pub width: u32,
    pub height: u32,
    pub pixels: Vec<u8>,
}

impl Mask {
    pub fn overlay_rgba(&self) -> Result<Vec<u8>, String> {
        let gray: GrayImage =
            ImageBuffer::<Luma<u8>, _>::from_raw(self.width, self.height, self.pixels.clone())
                .ok_or("Invalid mask size")?;
        let mut overlay = Vec::with_capacity(self.pixels.len() * 4);
        for pixel in gray.pixels() {
            overlay.extend_from_slice(&[40, 170, 255, if pixel[0] > 0 { 96 } else { 0 }]);
        }
        Ok(overlay)
    }
}
