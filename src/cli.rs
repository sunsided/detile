use clap::Parser;
use std::path::PathBuf;

#[derive(Debug, Parser)]
#[command(name = "tile-detect", about = "Detect regular tile grids in images")]
pub struct Cli {
    /// Input image path
    pub image: PathBuf,

    /// Output results as JSON
    #[arg(long)]
    pub json: bool,

    /// Save debug overlay image to this path
    #[arg(long, value_name = "PATH")]
    pub debug_overlay: Option<PathBuf>,

    /// Minimum tile stride in pixels
    #[arg(long, default_value_t = 4)]
    pub min_stride: u32,

    /// Maximum tile stride in pixels
    #[arg(long)]
    pub max_stride: Option<u32>,

    /// Maximum margin width in pixels
    #[arg(long)]
    pub max_margin: Option<u32>,

    /// Number of top candidates to retain
    #[arg(long, default_value_t = 10)]
    pub top_candidates: usize,

    /// Report all distinct grid scales (texture / tile / layout), up to N
    #[arg(long, value_name = "N")]
    pub levels: Option<usize>,

    /// Boost square tiles in scoring
    #[arg(long)]
    pub prefer_square: bool,

    /// Disable margin detection (stride equals tile size)
    #[arg(long = "no-margin")]
    pub no_margin: bool,

    /// Minimum confidence threshold (0..1)
    #[arg(long, default_value_t = 0.65)]
    pub min_confidence: f32,
}
