use clap::{Args, Parser, Subcommand};
use std::path::PathBuf;

#[derive(Debug, Parser)]
#[command(name = "tile-detect", about = "Detect regular tile grids in images")]
pub struct Cli {
    #[command(subcommand)]
    pub command: Option<Command>,

    #[command(flatten)]
    pub detect: DetectArgs,
}

#[derive(Debug, Subcommand)]
pub enum Command {
    /// Launch the interactive viewer (requires building with --features ui)
    Ui(UiArgs),
}

#[derive(Debug, Args)]
pub struct DetectArgs {
    /// Input image path
    pub image: Option<PathBuf>,

    /// Output results as JSON
    #[arg(long)]
    pub json: bool,

    /// Save debug overlay image to this path
    #[arg(long, value_name = "PATH")]
    pub debug_overlay: Option<PathBuf>,

    /// Save the deduplicated tile atlas (recovered tileset) to this path
    #[arg(long, value_name = "PATH")]
    pub atlas: Option<PathBuf>,

    /// Atlas dedup tolerance: max mean per-channel diff (0 = exact; ~8 absorbs JPEG noise)
    #[arg(long, default_value_t = 8.0)]
    pub atlas_tolerance: f32,

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

#[derive(Debug, Args)]
pub struct UiArgs {
    /// Input image path
    pub image: PathBuf,

    /// Minimum tile stride in pixels
    #[arg(long, default_value_t = 4)]
    pub min_stride: u32,

    /// Maximum tile stride in pixels
    #[arg(long)]
    pub max_stride: Option<u32>,

    /// Maximum margin width in pixels
    #[arg(long)]
    pub max_margin: Option<u32>,

    /// Boost square tiles in scoring
    #[arg(long)]
    pub prefer_square: bool,

    /// Disable margin detection (stride equals tile size)
    #[arg(long = "no-margin")]
    pub no_margin: bool,

    /// Maximum number of grid configurations to list
    #[arg(long, default_value_t = 12)]
    pub max_levels: usize,
}
