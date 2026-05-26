use serde::{Deserialize, Serialize};

#[derive(Debug, Clone)]
pub struct DetectOptions {
    pub min_stride: u32,
    pub max_stride: Option<u32>,
    pub max_margin: Option<u32>,
    pub top_candidates: usize,
    pub prefer_square: bool,
    pub allow_margin: bool,
    pub min_confidence: f32,
}

impl Default for DetectOptions {
    fn default() -> Self {
        Self {
            min_stride: 4,
            max_stride: None,
            max_margin: None,
            top_candidates: 10,
            prefer_square: false,
            allow_margin: true,
            min_confidence: 0.65,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct GridDetection {
    pub detected: bool,
    pub tile_width: u32,
    pub tile_height: u32,
    pub stride_x: u32,
    pub stride_y: u32,
    pub offset_x: u32,
    pub offset_y: u32,
    pub margin_x: u32,
    pub margin_y: u32,
    pub columns: u32,
    pub rows: u32,
    pub confidence: f32,
    pub x_axis: AxisDetection,
    pub y_axis: AxisDetection,
    pub candidates: Vec<GridCandidate>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AxisDetection {
    pub stride: u32,
    pub offset: u32,
    pub tile_size: u32,
    pub margin: u32,
    pub count: u32,
    pub confidence: f32,
    pub periodicity_score: f32,
    pub offset_score: f32,
    pub margin_score: f32,
    pub coverage_score: f32,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct GridCandidate {
    pub tile_width: u32,
    pub tile_height: u32,
    pub stride_x: u32,
    pub stride_y: u32,
    pub offset_x: u32,
    pub offset_y: u32,
    pub margin_x: u32,
    pub margin_y: u32,
    pub columns: u32,
    pub rows: u32,
    pub confidence: f32,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "status")]
pub enum DetectionResult {
    Found(GridDetection),
    NotFound {
        reason: String,
        best_confidence: f32,
        candidates: Vec<GridCandidate>,
    },
}

#[derive(Debug, Clone)]
pub struct StrideCandidate {
    pub stride: u32,
    pub score: f32,
}

#[derive(Debug, Clone)]
pub struct MarginDetection {
    pub tile_size: u32,
    pub margin: u32,
    pub score: f32,
}
