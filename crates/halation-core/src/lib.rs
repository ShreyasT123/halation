//! halation-core — shared data model for the Halation darkroom.
//!
//! Develop parameters + slider metadata, GPU uniform layouts, image / histogram buffers and the
//! branching edit-history graph. No I/O, no GPU handles, no UI types.
//!
//! Cargo: `bytemuck = { version = "1", features = ["derive"] }`, `serde = { version = "1", features = ["derive"] }`.

use bytemuck::{Pod, Zeroable};
use serde::{Deserialize, Serialize};
use std::fmt;
use std::sync::atomic::{AtomicU64, Ordering};

pub mod curve;
pub mod gpu_extras;
pub mod grading;

pub use curve::{CurveChannel, SplineSegment, ToneCurve, MAX_CURVE_POINTS, MAX_SEGMENTS};
pub use gpu_extras::{GpuExtraUniforms, GpuFrameInputs, LutBuffer};
pub use grading::{AiMaskKind, ColorGrading, LocalAdjustment, MaskBuffer, MaskSource, Wheel};

// ------------------------------------------------------------------------------------------------
// Parameter identifiers & slider metadata
// ------------------------------------------------------------------------------------------------

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum ParamId {
    Exposure,
    Contrast,
    Highlights,
    Shadows,
    Whites,
    Blacks,
    Temperature,
    Tint,
    Vibrance,
    Saturation,
}

impl ParamId {
    pub const ALL: [ParamId; 10] = [
        ParamId::Exposure,
        ParamId::Contrast,
        ParamId::Highlights,
        ParamId::Shadows,
        ParamId::Whites,
        ParamId::Blacks,
        ParamId::Temperature,
        ParamId::Tint,
        ParamId::Vibrance,
        ParamId::Saturation,
    ];
    pub const WHITE_BALANCE: [ParamId; 2] = [ParamId::Temperature, ParamId::Tint];
    pub const TONE: [ParamId; 6] = [
        ParamId::Exposure,
        ParamId::Contrast,
        ParamId::Highlights,
        ParamId::Shadows,
        ParamId::Whites,
        ParamId::Blacks,
    ];
    pub const PRESENCE: [ParamId; 2] = [ParamId::Vibrance, ParamId::Saturation];

    pub const fn label(self) -> &'static str {
        match self {
            ParamId::Exposure => "Exposure",
            ParamId::Contrast => "Contrast",
            ParamId::Highlights => "Highlights",
            ParamId::Shadows => "Shadows",
            ParamId::Whites => "Whites",
            ParamId::Blacks => "Blacks",
            ParamId::Temperature => "Temp",
            ParamId::Tint => "Tint",
            ParamId::Vibrance => "Vibrance",
            ParamId::Saturation => "Saturation",
        }
    }

    pub const fn min(self) -> f32 {
        match self {
            ParamId::Exposure => -5.0,
            ParamId::Temperature => 2000.0,
            ParamId::Tint => -150.0,
            _ => -100.0,
        }
    }

    pub const fn max(self) -> f32 {
        match self {
            ParamId::Exposure => 5.0,
            ParamId::Temperature => 50000.0,
            ParamId::Tint => 150.0,
            _ => 100.0,
        }
    }

    pub const fn default_value(self) -> f32 {
        match self {
            ParamId::Temperature => 5500.0,
            _ => 0.0,
        }
    }

    pub const fn unit(self) -> &'static str {
        match self {
            ParamId::Exposure => "EV",
            ParamId::Temperature => "K",
            _ => "",
        }
    }

    /// Temperature is edited on a logarithmic axis so equal slider travel is an equal perceptual shift.
    pub const fn is_log(self) -> bool {
        matches!(self, ParamId::Temperature)
    }

    pub fn slider_range(self) -> (f32, f32) {
        (self.to_slider(self.min()), self.to_slider(self.max()))
    }

    pub const fn slider_step(self) -> f32 {
        match self {
            ParamId::Exposure => 0.01,
            ParamId::Temperature => 0.002,
            _ => 1.0,
        }
    }

    pub fn to_slider(self, value: f32) -> f32 {
        if self.is_log() {
            value.clamp(self.min(), self.max()).ln()
        } else {
            value
        }
    }

    pub fn from_slider(self, slider: f32) -> f32 {
        let raw = if self.is_log() { slider.exp() } else { slider };
        self.snap(raw.clamp(self.min(), self.max()))
    }

    fn snap(self, v: f32) -> f32 {
        match self {
            ParamId::Exposure => (v * 100.0).round() / 100.0,
            ParamId::Temperature => (v / 10.0).round() * 10.0,
            _ => v.round(),
        }
    }

    pub fn get(self, p: &DevelopParameters) -> f32 {
        match self {
            ParamId::Exposure => p.exposure,
            ParamId::Contrast => p.contrast,
            ParamId::Highlights => p.highlights,
            ParamId::Shadows => p.shadows,
            ParamId::Whites => p.whites,
            ParamId::Blacks => p.blacks,
            ParamId::Temperature => p.temperature,
            ParamId::Tint => p.tint,
            ParamId::Vibrance => p.vibrance,
            ParamId::Saturation => p.saturation,
        }
    }

    pub fn set(self, p: &mut DevelopParameters, v: f32) {
        let v = if v.is_finite() { v.clamp(self.min(), self.max()) } else { self.default_value() };
        match self {
            ParamId::Exposure => p.exposure = v,
            ParamId::Contrast => p.contrast = v,
            ParamId::Highlights => p.highlights = v,
            ParamId::Shadows => p.shadows = v,
            ParamId::Whites => p.whites = v,
            ParamId::Blacks => p.blacks = v,
            ParamId::Temperature => p.temperature = v,
            ParamId::Tint => p.tint = v,
            ParamId::Vibrance => p.vibrance = v,
            ParamId::Saturation => p.saturation = v,
        }
    }

    /// Numeric readout without unit: `+0.50`, `-15`, `5500`.
    pub fn format_value(self, v: f32) -> String {
        match self {
            ParamId::Exposure => signed(v, 2),
            ParamId::Temperature => format!("{:.0}", v),
            _ => signed(v, 0),
        }
    }

    pub fn format_with_unit(self, v: f32) -> String {
        let n = self.format_value(v);
        if self.unit().is_empty() {
            n
        } else {
            format!("{n} {}", self.unit())
        }
    }
}

fn signed(v: f32, decimals: usize) -> String {
    let scale = 10f32.powi(decimals as i32);
    let rounded = (v * scale).round() / scale;
    if rounded == 0.0 {
        format!("{:.*}", decimals, 0.0)
    } else {
        format!("{:+.*}", decimals, rounded)
    }
}

// ------------------------------------------------------------------------------------------------
// Develop parameters
// ------------------------------------------------------------------------------------------------

/// Non-destructive edit state. `#[serde(default)]` keeps catalogs written by older builds loadable.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct DevelopParameters {
    pub exposure: f32,    // -5.0 ..= +5.0 EV
    pub contrast: f32,    // -100 ..= +100
    pub highlights: f32,  // -100 ..= +100
    pub shadows: f32,     // -100 ..= +100
    pub whites: f32,      // -100 ..= +100
    pub blacks: f32,      // -100 ..= +100
    pub temperature: f32, // 2000 K ..= 50000 K (5500 K = neutral)
    pub tint: f32,        // -150 ..= +150
    pub vibrance: f32,    // -100 ..= +100
    pub saturation: f32,  // -100 ..= +100
    pub curve: ToneCurve,
    pub grading: ColorGrading,
    pub local: Option<LocalAdjustment>,
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
            vibrance: 0.0,
            saturation: 0.0,
            curve: ToneCurve::default(),
            grading: ColorGrading::default(),
            local: None,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct ParamChange {
    pub id: ParamId,
    pub from: f32,
    pub to: f32,
}

impl ParamChange {
    /// `Exposure +0.50 EV`
    pub fn label(&self) -> String {
        format!("{} {}", self.id.label(), self.id.format_with_unit(self.to))
    }
    /// `0.00 → +0.50 EV`
    pub fn delta_label(&self) -> String {
        format!("{} → {}", self.id.format_value(self.from), self.id.format_with_unit(self.to))
    }
}

/// Returns `(label, detail)` for a set of changes, e.g. `("Exposure +0.50 EV | Contrast -15", "2 settings")`.
pub fn summarize_changes(changes: &[ParamChange]) -> (String, String) {
    match changes {
        [] => ("No change".into(), String::new()),
        [one] => (one.label(), one.delta_label()),
        many => {
            let mut label = many.iter().take(2).map(ParamChange::label).collect::<Vec<_>>().join(" | ");
            if many.len() > 2 {
                label.push_str(&format!(" | +{}", many.len() - 2));
            }
            (label, format!("{} settings", many.len()))
        }
    }
}

impl DevelopParameters {
    pub fn get(&self, id: ParamId) -> f32 {
        id.get(self)
    }
    pub fn set(&mut self, id: ParamId, v: f32) {
        id.set(self, v)
    }
    pub fn with(mut self, id: ParamId, v: f32) -> Self {
        id.set(&mut self, v);
        self
    }
    pub fn is_default(&self) -> bool {
        *self == Self::default()
    }
    pub fn is_default_param(&self, id: ParamId) -> bool {
        (id.get(self) - id.default_value()).abs() < 1e-4
    }
    pub fn reset_param(&mut self, id: ParamId) {
        id.set(self, id.default_value());
    }
    /// Clamp every field into range; NaN / inf fall back to the default.
    pub fn sanitized(mut self) -> Self {
        for id in ParamId::ALL {
            let v = id.get(&self);
            id.set(&mut self, v);
        }
        self.curve = self.curve.sanitized();
        self.grading = self.grading.sanitized();
        self.local = self.local.map(LocalAdjustment::sanitized);
        self
    }

    /// Auto label + detail for a history entry, covering sliders, curves, wheels and local masks.
    pub fn describe_changes(&self, previous: &Self) -> (String, String) {
        let sliders = self.changes_from(previous);
        let mut labels: Vec<String> = sliders.iter().map(ParamChange::label).collect();
        for channel in CurveChannel::ALL {
            if self.curve.points(channel) != previous.curve.points(channel) {
                labels.push(match channel {
                    CurveChannel::Rgb => "Tone Curve".to_owned(),
                    other => format!("Tone Curve {}", other.label()),
                });
            }
        }
        labels.extend(self.grading.change_labels(&previous.grading));
        if self.local != previous.local {
            labels.push("Local adjustment".to_owned());
        }
        match labels.len() {
            0 => ("No change".to_owned(), String::new()),
            1 => {
                let detail = if sliders.len() == 1 { sliders[0].delta_label() } else { String::new() };
                (labels.remove(0), detail)
            }
            n => {
                let mut label = labels.iter().take(2).cloned().collect::<Vec<_>>().join(" | ");
                if n > 2 {
                    label.push_str(&format!(" | +{}", n - 2));
                }
                (label, format!("{n} settings"))
            }
        }
    }
    pub fn changes_from(&self, previous: &Self) -> Vec<ParamChange> {
        ParamId::ALL
            .into_iter()
            .filter_map(|id| {
                let (from, to) = (id.get(previous), id.get(self));
                ((from - to).abs() > 1e-4).then_some(ParamChange { id, from, to })
            })
            .collect()
    }
}

// ------------------------------------------------------------------------------------------------
// GPU memory layouts (std140-safe, 16-byte aligned)
// ------------------------------------------------------------------------------------------------

#[repr(C, align(16))]
#[derive(Debug, Clone, Copy, PartialEq, Pod, Zeroable)]
pub struct GpuDevelopUniforms {
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
    pub num_spline_segments: u32,
    pub _padding: f32,
}

const _: () = assert!(std::mem::size_of::<GpuDevelopUniforms>() == 48);

impl GpuDevelopUniforms {
    pub fn from_params(p: &DevelopParameters, num_spline_segments: u32) -> Self {
        Self {
            exposure: p.exposure,
            contrast: p.contrast,
            highlights: p.highlights,
            shadows: p.shadows,
            whites: p.whites,
            blacks: p.blacks,
            temperature: p.temperature,
            tint: p.tint,
            vibrance: p.vibrance,
            saturation: p.saturation,
            num_spline_segments,
            _padding: 0.0,
        }
    }
}

/// 3×3 matrix for uniform buffers: each column padded to 16 bytes.
#[repr(C, align(16))]
#[derive(Debug, Clone, Copy, PartialEq, Pod, Zeroable)]
pub struct GpuMat3x3 {
    pub col0: [f32; 4],
    pub col1: [f32; 4],
    pub col2: [f32; 4],
}

impl GpuMat3x3 {
    pub const IDENTITY: Self = Self {
        col0: [1.0, 0.0, 0.0, 0.0],
        col1: [0.0, 1.0, 0.0, 0.0],
        col2: [0.0, 0.0, 1.0, 0.0],
    };
    /// `cols[i]` is column *i* (matches `glam::Mat3::to_cols_array_2d`).
    pub fn from_columns(cols: [[f32; 3]; 3]) -> Self {
        Self {
            col0: [cols[0][0], cols[0][1], cols[0][2], 0.0],
            col1: [cols[1][0], cols[1][1], cols[1][2], 0.0],
            col2: [cols[2][0], cols[2][1], cols[2][2], 0.0],
        }
    }
}

/// Matrices uploaded next to the scalar uniforms (96 bytes).
#[repr(C, align(16))]
#[derive(Debug, Clone, Copy, PartialEq, Pod, Zeroable)]
pub struct GpuDevelopMatrices {
    pub white_balance: GpuMat3x3,
    pub to_display: GpuMat3x3,
}

const _: () = assert!(std::mem::size_of::<GpuDevelopMatrices>() == 96);

// ------------------------------------------------------------------------------------------------
// Image + histogram buffers
// ------------------------------------------------------------------------------------------------

static NEXT_IMAGE_ID: AtomicU64 = AtomicU64::new(1);

/// Linear floating-point RGB in the **working space** (linear ProPhoto RGB). Unclamped.
/// `id` identifies the pixel content so GPU backends can cache uploaded textures.
#[derive(Clone)]
pub struct ImageBufferF32 {
    pub id: u64,
    pub width: u32,
    pub height: u32,
    pub pixels: Vec<[f32; 3]>,
}

impl ImageBufferF32 {
    pub fn new(width: u32, height: u32, pixels: Vec<[f32; 3]>) -> Self {
        Self { id: NEXT_IMAGE_ID.fetch_add(1, Ordering::Relaxed), width, height, pixels }
    }
    /// Call after mutating `pixels` in place so cached GPU copies are invalidated.
    pub fn touch(&mut self) {
        self.id = NEXT_IMAGE_ID.fetch_add(1, Ordering::Relaxed);
    }
    pub fn is_valid(&self) -> bool {
        self.width > 0 && self.height > 0 && self.pixels.len() == self.width as usize * self.height as usize
    }
}

impl fmt::Debug for ImageBufferF32 {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "ImageBufferF32 {{ id: {}, {}x{} }}", self.id, self.width, self.height)
    }
}

#[derive(Debug, Clone)]
pub struct HistogramData {
    pub r: [u32; 256],
    pub g: [u32; 256],
    pub b: [u32; 256],
    pub luma: [u32; 256],
}

impl Default for HistogramData {
    fn default() -> Self {
        Self { r: [0; 256], g: [0; 256], b: [0; 256], luma: [0; 256] }
    }
}

impl HistogramData {
    /// Histogram of an 8-bit display-referred RGBA buffer.
    pub fn from_rgba8(rgba: &[u8]) -> Self {
        let mut h = Self::default();
        for px in rgba.chunks_exact(4) {
            let (r, g, b) = (px[0] as usize, px[1] as usize, px[2] as usize);
            h.r[r] += 1;
            h.g[g] += 1;
            h.b[b] += 1;
            h.luma[(54 * r + 183 * g + 19 * b) >> 8] += 1;
        }
        h
    }

    pub fn total(&self) -> u64 {
        self.luma.iter().map(|&c| c as u64).sum()
    }

    /// Tallest interior bin across channels (bins 0 and 255 are ignored so clipping spikes do not flatten the graph).
    pub fn peak(&self) -> u32 {
        [&self.r, &self.g, &self.b, &self.luma]
            .iter()
            .flat_map(|c| c[1..255].iter().copied())
            .max()
            .unwrap_or(1)
            .max(1)
    }

    /// Fraction of pixels with any channel at 0.
    pub fn clipped_shadows(&self) -> f32 {
        let t = self.total().max(1) as f32;
        self.r[0].max(self.g[0]).max(self.b[0]) as f32 / t
    }

    /// Fraction of pixels with any channel at 255.
    pub fn clipped_highlights(&self) -> f32 {
        let t = self.total().max(1) as f32;
        self.r[255].max(self.g[255]).max(self.b[255]) as f32 / t
    }
}

// ------------------------------------------------------------------------------------------------
// Branching edit history
// ------------------------------------------------------------------------------------------------

#[derive(Debug, Clone)]
pub struct HistoryNode {
    pub id: usize,
    pub parent: Option<usize>,
    pub label: String,
    pub detail: String,
    pub params: DevelopParameters,
}

/// In-memory history DAG. Undoing and editing spawns a new branch; nothing is ever truncated.
#[derive(Debug, Clone)]
pub struct HistoryGraph {
    nodes: Vec<HistoryNode>,
    active: usize,
}

impl HistoryGraph {
    pub fn new(baseline: DevelopParameters, label: impl Into<String>) -> Self {
        Self {
            nodes: vec![HistoryNode { id: 0, parent: None, label: label.into(), detail: String::new(), params: baseline }],
            active: 0,
        }
    }
    /// Rebuild a graph from persisted nodes. Node `i` must carry `id == i`, parents must precede
    /// children, and `active` must be a valid index; otherwise `None`.
    pub fn from_parts(nodes: Vec<HistoryNode>, active: usize) -> Option<Self> {
        let valid = !nodes.is_empty()
            && active < nodes.len()
            && nodes.iter().enumerate().all(|(i, n)| n.id == i && n.parent.map_or(true, |p| p < i));
        valid.then_some(Self { nodes, active })
    }

    pub fn nodes(&self) -> &[HistoryNode] {
        &self.nodes
    }
    pub fn active_id(&self) -> usize {
        self.active
    }
    pub fn active(&self) -> &HistoryNode {
        &self.nodes[self.active]
    }

    /// Commit `params` as a child of the active node (auto-labelled). `None` if nothing changed.
    pub fn commit(&mut self, params: DevelopParameters) -> Option<usize> {
        let (label, detail) = params.describe_changes(&self.nodes[self.active].params);
        self.commit_labeled(params, label, detail)
    }

    pub fn commit_labeled(&mut self, params: DevelopParameters, label: impl Into<String>, detail: impl Into<String>) -> Option<usize> {
        if self.nodes[self.active].params == params {
            return None;
        }
        let id = self.nodes.len();
        self.nodes.push(HistoryNode { id, parent: Some(self.active), label: label.into(), detail: detail.into(), params });
        self.active = id;
        Some(id)
    }

    pub fn checkout(&mut self, id: usize) -> Option<&DevelopParameters> {
        if id < self.nodes.len() {
            self.active = id;
            Some(&self.nodes[id].params)
        } else {
            None
        }
    }

    pub fn undo(&mut self) -> Option<&DevelopParameters> {
        let parent = self.nodes[self.active].parent?;
        self.active = parent;
        Some(&self.nodes[parent].params)
    }

    /// Redo follows the most recently created child branch.
    pub fn redo(&mut self) -> Option<&DevelopParameters> {
        let child = self.children(self.active).last().copied()?;
        self.active = child;
        Some(&self.nodes[child].params)
    }

    pub fn children(&self, id: usize) -> Vec<usize> {
        self.nodes.iter().filter(|n| n.parent == Some(id)).map(|n| n.id).collect()
    }

    /// Display lane: increments for every non-first child on the path to the root.
    pub fn lane(&self, id: usize) -> usize {
        let (mut lane, mut cur) = (0, id);
        while let Some(parent) = self.nodes[cur].parent {
            if self.children(parent).first() != Some(&cur) {
                lane += 1;
            }
            cur = parent;
        }
        lane
    }

    pub fn on_active_path(&self, id: usize) -> bool {
        let mut cur = Some(self.active);
        while let Some(c) = cur {
            if c == id {
                return true;
            }
            cur = self.nodes[c].parent;
        }
        false
    }
}

// ------------------------------------------------------------------------------------------------
// Tests
// ------------------------------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn legacy_json_still_loads() {
        let old = r#"{"exposure":0.5,"contrast":10.0,"highlights":0.0,"shadows":0.0,"temperature":6000.0,"tint":0.0}"#;
        let p: DevelopParameters = serde_json::from_str(old).unwrap();
        assert_eq!(p.exposure, 0.5);
        assert_eq!(p.whites, 0.0);
        assert_eq!(p.temperature, 6000.0);
    }

    #[test]
    fn slider_roundtrip_hits_default_exactly() {
        for id in ParamId::ALL {
            assert_eq!(id.from_slider(id.to_slider(id.default_value())), id.default_value());
        }
    }

    #[test]
    fn history_branches_without_truncation() {
        let mut h = HistoryGraph::new(DevelopParameters::default(), "Loaded");
        let a = h.commit(DevelopParameters::default().with(ParamId::Exposure, 0.5)).unwrap();
        let _b = h.commit(DevelopParameters::default().with(ParamId::Exposure, 0.5).with(ParamId::Contrast, 20.0)).unwrap();
        h.checkout(a);
        let c = h.commit(DevelopParameters::default().with(ParamId::Exposure, 0.5).with(ParamId::Temperature, 6500.0)).unwrap();
        assert_eq!(h.nodes().len(), 4);
        assert_eq!(h.lane(c), 1);
        assert_eq!(h.nodes()[c].label, "Temp 6500 K");
    }

    #[test]
    fn formatting() {
        assert_eq!(ParamId::Exposure.format_with_unit(0.5), "+0.50 EV");
        assert_eq!(ParamId::Contrast.format_value(-15.0), "-15");
        assert_eq!(ParamId::Contrast.format_value(0.0), "0");
    }
}
