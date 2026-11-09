//! halation-ml — on-device vision (Sprint 8). No sockets, no cloud: ONNX Runtime with DirectML
//! (Windows), CoreML (macOS) or CPU (Linux / fallback).
//!
//! Two decoupled stages:
//! 1. [`VisionEngine::embed`] — run once per opened photo on a background task: MobileSAM image encoder
//!    (INT8) → `[1, 256, 64, 64]` embedding, cached in an [`Embedding`].
//! 2. [`VisionEngine::decode`] / [`VisionEngine::select`] — the lightweight prompt decoder on the cached
//!    embedding; this is the interactive path (target < 15 ms on a GPU provider).
//!
//! Models (not bundled; drop them into `assets/models/`, `assets/models/mobilesam/`, or point
//! `HALATION_MODELS` at the directory): `mobilesam_encoder_quant.onnx` plus either
//! `mobilesam_decoder_quant.onnx` (compact 3-input MobileSAM export) or `mobilesam_decoder.onnx`
//! (official 6-input SAM export with `mask_input, has_mask_input, orig_im_size`). The compact form
//! is detected automatically and gets 0..1 encoder preprocessing; the official form gets SAM's
//! mean/std normalization.
//!
//! Cargo: `ort = { version = "=2.0.0-rc.10", features = ["directml", "coreml"] }` (enable per target),
//! `thiserror`, `halation-core`.

pub mod mask;

use halation_core::{AiMaskKind, MaskBuffer};
use mask::{automatic_prompts, logits_to_mask, preprocess_for_encoder, EncoderNorm, Prompt};
use ort::{
    execution_providers::{CPUExecutionProvider, ExecutionProviderDispatch},
    session::{builder::GraphOptimizationLevel, Session},
    value::{Tensor, ValueType},
};
use std::{
    path::{Path, PathBuf},
    sync::{Arc, Mutex},
};
use thiserror::Error;

pub const ENCODER_FILE: &str = "mobilesam_encoder_quant.onnx";
pub const DECODER_FILE: &str = "mobilesam_decoder.onnx";
const EMBEDDING_LEN: usize = 256 * 64 * 64;

/// Filenames seen in the wild for the MobileSAM encoder. The first entry is the documented name.
const ENCODER_FILES: &[&str] = &[
    "mobilesam_encoder_quant.onnx",
    "mobilesam_encoder.onnx",
    "encoder_quant.onnx",
    "encoder.onnx",
];
/// Filenames seen in the wild for the MobileSAM decoder. The official export omits `_quant` on the
/// decoder even when the encoder is quantised, so both spellings are accepted.
const DECODER_FILES: &[&str] = &[
    "mobilesam_decoder_quant.onnx",
    "mobilesam_decoder.onnx",
    "decoder_quant.onnx",
    "decoder.onnx",
];

#[derive(Debug, Error)]
pub enum MlError {
    #[error("model file missing: {0}")]
    MissingModel(PathBuf),
    #[error("model files not found in {0} (expected mobilesam_encoder_quant.onnx and mobilesam_decoder*.onnx)")]
    MissingModels(PathBuf),
    #[error("onnx runtime: {0}")]
    Ort(String),
    #[error("unexpected model output: {0}")]
    Shape(String),
    #[error("engine lock poisoned")]
    Poisoned,
}

fn ort_err(e: impl std::fmt::Display) -> MlError {
    MlError::Ort(e.to_string())
}

fn find_in_dir(dir: &Path, names: &[&str]) -> Option<PathBuf> {
    names.iter().map(|n| dir.join(n)).find(|p| p.is_file())
}

fn candidate_dirs(base: &Path) -> Vec<PathBuf> {
    let mut dirs = Vec::new();
    let mut push = |p: PathBuf| {
        if p.is_dir() && !dirs.contains(&p) {
            dirs.push(p);
        }
    };
    push(base.to_path_buf());
    for sub in ["mobilesam", "mobilesam-onnx-float", "mobilesam-onnx"] {
        push(base.join(sub));
        push(base.join(sub).join("mobilesam-onnx-float"));
    }
    // A shallow scan covers layouts such as `models/mobilesam/mobilesam-onnx-float`.
    if let Ok(entries) = std::fs::read_dir(base) {
        for entry in entries.flatten().take(64) {
            let p = entry.path();
            if !p.is_dir() {
                continue;
            }
            let name = p.file_name().and_then(|n| n.to_str()).unwrap_or_default();
            if name.starts_with('.') || name == "target" || name == "node_modules" {
                continue;
            }
            push(p.clone());
            push(p.join("mobilesam-onnx-float"));
        }
    }
    dirs
}

/// Find a matching encoder/decoder pair below `base`. Accepts the common filenames and the
/// `mobilesam/` subdirectory layout.
pub fn find_models(base: &Path) -> Option<(PathBuf, PathBuf)> {
    for dir in candidate_dirs(base) {
        if let (Some(enc), Some(dec)) = (
            find_in_dir(&dir, ENCODER_FILES),
            find_in_dir(&dir, DECODER_FILES),
        ) {
            return Some((enc, dec));
        }
    }
    None
}

/// Whether a complete encoder/decoder pair can be found below `base`.
pub fn has_models(base: &Path) -> bool {
    find_models(base).is_some()
}

/// Cached encoder output for one image.
#[derive(Clone)]
pub struct Embedding {
    pub image_id: u64,
    scale: f32,
    orig_w: u32,
    orig_h: u32,
    data: Arc<Vec<f32>>,
}

pub struct VisionEngine {
    encoder: Mutex<Session>,
    decoder: Mutex<Session>,
    provider: &'static str,
    /// Whether the decoder is the official 6-input SAM export.
    full_decoder: bool,
    /// Maximum number of prompt points the decoder accepts (`usize::MAX` when dynamic).
    point_budget: usize,
    /// Encoder input normalization matching the decoder variant.
    norm: EncoderNorm,
}

fn providers() -> (Vec<ExecutionProviderDispatch>, &'static str) {
    #[cfg(target_os = "windows")]
    {
        use ort::execution_providers::DirectMLExecutionProvider;
        (
            vec![
                DirectMLExecutionProvider::default().build(),
                CPUExecutionProvider::default().build(),
            ],
            "DirectML",
        )
    }
    #[cfg(target_os = "macos")]
    {
        use ort::execution_providers::CoreMLExecutionProvider;
        (
            vec![
                CoreMLExecutionProvider::default().build(),
                CPUExecutionProvider::default().build(),
            ],
            "CoreML",
        )
    }
    #[cfg(not(any(target_os = "windows", target_os = "macos")))]
    {
        (vec![CPUExecutionProvider::default().build()], "CPU")
    }
}

fn build_session(path: &Path, eps: Vec<ExecutionProviderDispatch>) -> Result<Session, MlError> {
    if !path.is_file() {
        return Err(MlError::MissingModel(path.to_path_buf()));
    }
    Session::builder()
        .map_err(ort_err)?
        .with_execution_providers(eps)
        .map_err(ort_err)?
        .with_optimization_level(GraphOptimizationLevel::Level3)
        .map_err(ort_err)?
        .commit_from_file(path)
        .map_err(ort_err)
}

fn load_pair(
    encoder: &Path,
    decoder: &Path,
    eps: Vec<ExecutionProviderDispatch>,
) -> Result<(Session, Session), MlError> {
    Ok((
        build_session(encoder, eps.clone())?,
        build_session(decoder, eps)?,
    ))
}

fn tensor_hw(shape: &[i64]) -> Result<(usize, usize), MlError> {
    let dims: Vec<usize> = shape
        .iter()
        .filter(|&&d| d > 0)
        .map(|&d| d as usize)
        .collect();
    if dims.len() < 2 {
        return Err(MlError::Shape(format!(
            "mask {shape:?} has no spatial dimensions"
        )));
    }
    Ok((dims[dims.len() - 1], dims[dims.len() - 2]))
}

fn sample_bilinear(data: &[f32], w: usize, h: usize, x: f32, y: f32) -> f32 {
    if w == 0 || h == 0 {
        return 0.0;
    }
    let x = x.clamp(0.0, (w - 1) as f32);
    let y = y.clamp(0.0, (h - 1) as f32);
    let x0 = x.floor() as usize;
    let y0 = y.floor() as usize;
    let x1 = (x0 + 1).min(w - 1);
    let y1 = (y0 + 1).min(h - 1);
    let fx = x - x0 as f32;
    let fy = y - y0 as f32;
    let top = data[y0 * w + x0] * (1.0 - fx) + data[y0 * w + x1] * fx;
    let bottom = data[y1 * w + x0] * (1.0 - fx) + data[y1 * w + x1] * fx;
    top * (1.0 - fy) + bottom * fy
}

impl VisionEngine {
    /// Load both models from `model_dir` (or one of its subdirectories). Fails fast with
    /// [`MlError::MissingModels`] so the UI can simply grey out the AI mask buttons.
    pub fn new(model_dir: &Path) -> Result<Self, MlError> {
        let (enc_path, dec_path) = find_models(model_dir)
            .ok_or_else(|| MlError::MissingModels(model_dir.to_path_buf()))?;
        let (eps, preferred) = providers();
        // DirectML does not support every quantised MobileSAM graph. Fall back to CPU when the
        // preferred provider cannot initialise the sessions.
        let (encoder, decoder, provider) = match load_pair(&enc_path, &dec_path, eps) {
            Ok((encoder, decoder)) => (encoder, decoder, preferred),
            Err(first) => {
                let cpu = vec![CPUExecutionProvider::default().build()];
                match load_pair(&enc_path, &dec_path, cpu) {
                    Ok((encoder, decoder)) => (encoder, decoder, "CPU"),
                    Err(_) => return Err(first),
                }
            }
        };

        // The official SAM ONNX export has six decoder inputs; the compact MobileSAM export used by
        // this project has only the three core ones. Detect which one we loaded and adapt both the
        // encoder preprocessing and the prompt count.
        let full_decoder = decoder.inputs.iter().any(|i| i.name == "mask_input");
        let point_budget = decoder
            .inputs
            .iter()
            .find(|i| i.name == "point_coords")
            .and_then(|i| match &i.input_type {
                ValueType::Tensor { dimensions, .. } => dimensions.get(1).copied(),
                _ => None,
            })
            .filter(|&d| d > 0)
            .map(|d| d as usize)
            .unwrap_or(usize::MAX);
        let norm = if full_decoder {
            EncoderNorm::MeanStd
        } else {
            EncoderNorm::ZeroOne
        };

        Ok(Self {
            encoder: Mutex::new(encoder),
            decoder: Mutex::new(decoder),
            provider,
            full_decoder,
            point_budget,
            norm,
        })
    }

    pub fn provider(&self) -> &'static str {
        self.provider
    }

    /// Stage 1 (background): encode `rgb` (8-bit, `width·height·3`) into an [`Embedding`].
    pub fn embed(
        &self,
        image_id: u64,
        rgb: &[u8],
        width: u32,
        height: u32,
    ) -> Result<Embedding, MlError> {
        if width == 0 || height == 0 || rgb.len() != width as usize * height as usize * 3 {
            return Err(MlError::Shape("image buffer size".into()));
        }
        let (input, scale) =
            preprocess_for_encoder(rgb, width as usize, height as usize, self.norm);
        let tensor = Tensor::from_array(([1usize, 3, mask::SAM_INPUT, mask::SAM_INPUT], input))
            .map_err(ort_err)?;
        let session = self.encoder.lock().map_err(|_| MlError::Poisoned)?;
        let (in_name, out_name) = (
            session.inputs[0].name.clone(),
            session.outputs[0].name.clone(),
        );
        let inputs = ort::inputs![in_name.as_str() => tensor].map_err(ort_err)?;
        let outputs = session.run(inputs).map_err(ort_err)?;
        let (shape, data) = outputs[out_name.as_str()]
            .try_extract_raw_tensor::<f32>()
            .map_err(ort_err)?;
        if data.len() != EMBEDDING_LEN {
            return Err(MlError::Shape(format!(
                "embedding {shape:?} has {} values, expected {EMBEDDING_LEN}",
                data.len()
            )));
        }
        Ok(Embedding {
            image_id,
            scale,
            orig_w: width,
            orig_h: height,
            data: Arc::new(data.to_vec()),
        })
    }

    /// Stage 2 (interactive): decode a soft mask from prompt points. `out_long_edge` bounds the mask
    /// resolution (the GPU resamples it bilinearly, 512 is plenty and keeps this under ~15 ms).
    pub fn decode(
        &self,
        emb: &Embedding,
        prompts: &[Prompt],
        out_long_edge: u32,
    ) -> Result<MaskBuffer, MlError> {
        let (ow, oh) = (emb.orig_w as f32, emb.orig_h as f32);
        let shrink = (out_long_edge as f32 / ow.max(oh)).min(1.0);
        let (mw, mh) = (
            ((ow * shrink).round() as u32).max(1),
            ((oh * shrink).round() as u32).max(1),
        );

        // Some MobileSAM exports only accept a single prompt point. Keep the most recent points if
        // the caller supplied more than the decoder's static input dimension allows.
        let prompts = if prompts.len() > self.point_budget {
            &prompts[prompts.len() - self.point_budget..]
        } else {
            prompts
        };

        let mut coords = Vec::with_capacity((prompts.len() + 1) * 2);
        let mut labels = Vec::with_capacity(prompts.len() + 1);
        for p in prompts {
            coords.extend([
                p.x.clamp(0.0, 1.0) * ow * emb.scale,
                p.y.clamp(0.0, 1.0) * oh * emb.scale,
            ]);
            labels.push(p.label);
        }
        // SAM's padding point only fits when the decoder has a free prompt slot.
        if prompts.len() < self.point_budget {
            coords.extend([0.0, 0.0]);
            labels.push(-1.0);
        }
        if labels.is_empty() {
            coords.extend([ow * emb.scale * 0.5, oh * emb.scale * 0.5]);
            labels.push(1.0);
        }
        let n = labels.len();

        let embeddings = Tensor::from_array(([1usize, 256, 64, 64], emb.data.as_ref().clone()))
            .map_err(ort_err)?;
        let point_coords = Tensor::from_array(([1usize, n, 2], coords)).map_err(ort_err)?;
        let point_labels = Tensor::from_array(([1usize, n], labels)).map_err(ort_err)?;

        let session = self.decoder.lock().map_err(|_| MlError::Poisoned)?;
        let output_name = session
            .outputs
            .iter()
            .find(|o| o.name == "masks")
            .or_else(|| session.outputs.first())
            .map(|o| o.name.clone())
            .unwrap_or_else(|| "masks".to_owned());

        // The official SAM export takes the extra mask/size inputs; the compact MobileSAM export
        // does not. Only pass the inputs the loaded graph declares.
        let (shape, logits) = if self.full_decoder {
            let mask_input = Tensor::from_array(([1usize, 1, 256, 256], vec![0.0f32; 256 * 256]))
                .map_err(ort_err)?;
            let has_mask_input = Tensor::from_array(([1usize], vec![0.0f32])).map_err(ort_err)?;
            let orig_im_size =
                Tensor::from_array(([2usize], vec![mh as f32, mw as f32])).map_err(ort_err)?;
            let inputs = ort::inputs![
                "image_embeddings" => embeddings,
                "point_coords" => point_coords,
                "point_labels" => point_labels,
                "mask_input" => mask_input,
                "has_mask_input" => has_mask_input,
                "orig_im_size" => orig_im_size,
            ]
            .map_err(ort_err)?;
            let outputs = session.run(inputs).map_err(ort_err)?;
            let (shape, data) = outputs[output_name.as_str()]
                .try_extract_raw_tensor::<f32>()
                .map_err(ort_err)?;
            (shape.to_vec(), data.to_vec())
        } else {
            let inputs = ort::inputs![
                "image_embeddings" => embeddings,
                "point_coords" => point_coords,
                "point_labels" => point_labels,
            ]
            .map_err(ort_err)?;
            let outputs = session.run(inputs).map_err(ort_err)?;
            let (shape, data) = outputs[output_name.as_str()]
                .try_extract_raw_tensor::<f32>()
                .map_err(ort_err)?;
            (shape.to_vec(), data.to_vec())
        };

        let (lw, lh) = tensor_hw(&shape)?;
        let (mw_us, mh_us) = (mw as usize, mh as usize);
        let data = if (lw, lh) == (mw_us, mh_us) {
            if logits.len() < mw_us * mh_us {
                return Err(MlError::Shape(format!(
                    "mask {shape:?} smaller than {mw}×{mh}"
                )));
            }
            logits[..mw_us * mh_us].to_vec()
        } else {
            // The compact export returns a square low-resolution mask covering the padded 1024²
            // encoder input. The official export returns the requested image rectangle. Resample
            // either layout to the requested mask size.
            let content_w = emb.orig_w as f32 * emb.scale;
            let content_h = emb.orig_h as f32 * emb.scale;
            let padded = !self.full_decoder;
            let mut out = Vec::with_capacity(mw_us * mh_us);
            for y in 0..mh_us {
                for x in 0..mw_us {
                    let (sx, sy) = if padded {
                        let cx = (x as f32 + 0.5) / mw_us as f32 * content_w;
                        let cy = (y as f32 + 0.5) / mh_us as f32 * content_h;
                        (
                            cx / mask::SAM_INPUT as f32 * lw as f32 - 0.5,
                            cy / mask::SAM_INPUT as f32 * lh as f32 - 0.5,
                        )
                    } else {
                        (
                            (x as f32 + 0.5) / mw_us as f32 * lw as f32 - 0.5,
                            (y as f32 + 0.5) / mh_us as f32 * lh as f32 - 0.5,
                        )
                    };
                    out.push(sample_bilinear(&logits, lw, lh, sx, sy));
                }
            }
            out
        };
        Ok(logits_to_mask(&data, mw, mh))
    }

    /// One-click "Select Subject" / "Select Sky" with optional refinement clicks.
    pub fn select(
        &self,
        emb: &Embedding,
        kind: AiMaskKind,
        extra: &[Prompt],
    ) -> Result<MaskBuffer, MlError> {
        let automatic = automatic_prompts(kind);
        let mut prompts = Vec::with_capacity(automatic.len() + extra.len());
        if extra.is_empty() {
            prompts.extend(automatic);
        } else {
            // Keep the primary automatic point alongside the user's refinement clicks.
            if let Some(first) = automatic.first() {
                prompts.push(*first);
            }
            prompts.extend_from_slice(extra);
        }
        if prompts.len() > self.point_budget {
            if extra.is_empty() {
                prompts.truncate(self.point_budget);
            } else {
                let start = prompts.len().saturating_sub(self.point_budget);
                prompts = prompts[start..].to_vec();
            }
        }
        self.decode(emb, &prompts, 512)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn missing_models_fail_before_the_runtime_is_touched() {
        let dir = std::env::temp_dir().join("halation-no-models");
        match VisionEngine::new(&dir) {
            Err(MlError::MissingModels(p)) => assert_eq!(p, dir),
            other => panic!("expected MissingModels, got {:?}", other.err()),
        }
    }

    #[test]
    fn finds_models_in_mobilesam_subdirectory() {
        let root = std::env::temp_dir().join(format!(
            "halation-models-{}-{:?}",
            std::process::id(),
            std::thread::current().id()
        ));
        let dir = root.join("mobilesam");
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(dir.join(ENCODER_FILE), b"x").unwrap();
        std::fs::write(dir.join("mobilesam_decoder_quant.onnx"), b"x").unwrap();
        assert!(has_models(&root));
        let (encoder, decoder) = find_models(&root).unwrap();
        assert_eq!(encoder, dir.join(ENCODER_FILE));
        assert_eq!(decoder, dir.join("mobilesam_decoder_quant.onnx"));
        std::fs::remove_dir_all(&root).ok();
    }
}
